//! 96/384-well microplate mapping, `.lctemplate` JSON serialization, and tidy long-to-wide
//! pivot transformations (`QOL-2`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};

use lumencraft_doc::{
    Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, DataSheet, FloatingNote, GroupedTable, MultipleVariablesTable,
    RectPt, SubcolumnMode, TableArchetype, TableKind, XColumnMode, XyTable, validate_data_sets, validate_rows, validate_subcolumns, validate_title,
};
use lumencraft_math::sum::SummaryMoments;
use serde::de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::ser::{
    self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
};
use serde::{Deserialize, Serialize};

use crate::archetype_convert::TargetArchetypeKind;
use crate::error::TransformError;

/// Standard SBS microplate grid geometries supported by `QOL-2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlateGeometry {
    /// 8 rows (A-H) x 12 columns (1-12) = 96 wells.
    Wells96,
    /// 16 rows (A-P) x 24 columns (1-24) = 384 wells.
    Wells384,
}

impl PlateGeometry {
    /// Number of plate rows (`8` for 96-well, `16` for 384-well).
    #[must_use]
    pub const fn rows(self) -> usize {
        match self {
            Self::Wells96 => 8,
            Self::Wells384 => 16,
        }
    }

    /// Number of plate columns (`12` for 96-well, `24` for 384-well).
    #[must_use]
    pub const fn cols(self) -> usize {
        match self {
            Self::Wells96 => 12,
            Self::Wells384 => 24,
        }
    }

    /// Total number of wells (`96` or `384`).
    #[must_use]
    pub const fn well_count(self) -> usize {
        self.rows() * self.cols()
    }

    /// Row-major well index for zero-based `(row, col)`.
    pub fn well_index(self, row: usize, col: usize) -> Result<usize, TransformError> {
        if row >= self.rows() || col >= self.cols() {
            return Err(TransformError::InvalidSpec {
                message: format!("well coordinate ({row}, {col}) out of bounds for {}x{} plate", self.rows(), self.cols()),
            });
        }
        Ok(row * self.cols() + col)
    }

    /// Human-readable plate well label such as `"A01"` or `"P24"`.
    pub fn well_label(self, row: usize, col: usize) -> Result<String, TransformError> {
        let _ = self.well_index(row, col)?;
        let row_byte = u8::try_from(row).map_err(|_| TransformError::InvalidSpec { message: format!("plate row index {row} out of range") })?;
        let row_char = char::from(b'A'.saturating_add(row_byte));
        Ok(format!("{row_char}{:02}", col + 1))
    }

    /// Human-readable label for a row-major flat well index `0..self.well_count()`.
    pub fn label_for_index(self, idx: usize) -> Result<String, TransformError> {
        if idx >= self.well_count() {
            return Err(TransformError::InvalidSpec { message: format!("well index {idx} out of bounds for {}-well plate", self.well_count()) });
        }
        let cols = self.cols();
        self.well_label(idx / cols, idx % cols)
    }
}

/// Experimental role assigned to each well in a [`PlateMapTemplate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlateWellRole {
    /// Experimental treatment sample with group/compound name, x concentration/dilution, and replicate index.
    Treatment { group_name: String, dilution_x: f64, replicate: usize },
    /// Background blank well used optionally for mean blank subtraction.
    Blank,
    /// Assay positive control well.
    PositiveControl,
    /// Excluded or unused well whose reading must never enter treatment groups or blank means.
    Excluded,
}

/// Serializable 96/384-well plate layout template (`.lctemplate` JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlateMapTemplate {
    pub name: String,
    pub geometry: PlateGeometry,
    pub x_title: String,
    pub wells: Vec<PlateWellRole>,
}

impl PlateMapTemplate {
    /// Create a plate map template initialized with [`PlateWellRole::Excluded`] in every well.
    pub fn new(name: impl Into<String>, geometry: PlateGeometry, x_title: impl Into<String>) -> Result<Self, TransformError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(TransformError::InvalidSpec { message: String::from("plate template name cannot be empty") });
        }
        Ok(Self { name, geometry, x_title: x_title.into(), wells: vec![PlateWellRole::Excluded; geometry.well_count()] })
    }

    /// Construct a plate template from an explicit row-major list of well roles.
    pub fn from_wells(
        name: impl Into<String>,
        geometry: PlateGeometry,
        x_title: impl Into<String>,
        wells: Vec<PlateWellRole>,
    ) -> Result<Self, TransformError> {
        let template = Self { name: name.into(), geometry, x_title: x_title.into(), wells };
        template.validate()?;
        Ok(template)
    }

    /// Assign the role at zero-based `(row, col)` well coordinates.
    pub fn set_well(&mut self, row: usize, col: usize, role: PlateWellRole) -> Result<(), TransformError> {
        let idx = self.geometry.well_index(row, col)?;
        if self.wells.len() != self.geometry.well_count() {
            return Err(TransformError::InvalidSpec {
                message: format!(
                    "template well count {} does not match {}x{} plate ({})",
                    self.wells.len(),
                    self.geometry.rows(),
                    self.geometry.cols(),
                    self.geometry.well_count()
                ),
            });
        }
        validate_well_role(&role, self.geometry, idx)?;
        self.wells[idx] = role;
        Ok(())
    }

    /// Read the role at zero-based `(row, col)` well coordinates.
    pub fn well(&self, row: usize, col: usize) -> Result<&PlateWellRole, TransformError> {
        let idx = self.geometry.well_index(row, col)?;
        self.wells.get(idx).ok_or_else(|| TransformError::InvalidSpec { message: format!("template well index {idx} missing") })
    }

    /// Validate well count and all finite treatment parameters.
    pub fn validate(&self) -> Result<(), TransformError> {
        let expected = self.geometry.well_count();
        if self.wells.len() != expected {
            return Err(TransformError::InvalidSpec { message: format!("plate template has {} well(s), expected {expected}", self.wells.len()) });
        }
        for (idx, role) in self.wells.iter().enumerate() {
            validate_well_role(role, self.geometry, idx)?;
        }
        Ok(())
    }

    /// Serialize this template as `.lctemplate` UTF-8 JSON.
    pub fn to_lctemplate_json(&self) -> Result<String, TransformError> {
        json_codec::to_json_string(self)
            .map_err(|err| TransformError::InvalidInput { message: format!("failed to serialize plate template JSON: {err}") })
    }

    /// Parse a `.lctemplate` JSON string back into a validated [`PlateMapTemplate`].
    pub fn from_lctemplate_json(json: &str) -> Result<Self, TransformError> {
        let parsed: Self = json_codec::from_json_str(json)
            .map_err(|err| TransformError::InvalidInput { message: format!("failed to parse plate template JSON: {err}") })?;
        parsed.validate()?;
        Ok(parsed)
    }
}

fn validate_well_role(role: &PlateWellRole, geometry: PlateGeometry, idx: usize) -> Result<(), TransformError> {
    if let PlateWellRole::Treatment { group_name, dilution_x, .. } = role {
        let label = geometry.label_for_index(idx).unwrap_or_else(|_| format!("#{idx}"));
        if group_name.trim().is_empty() {
            return Err(TransformError::InvalidSpec { message: format!("treatment group_name at well {label} cannot be empty") });
        }
        if !dilution_x.is_finite() {
            return Err(TransformError::InvalidInput { message: format!("treatment dilution_x at well {label} must be finite") });
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct TreatmentPoint {
    group_name: String,
    dilution_x: f64,
    replicate: usize,
    well_idx: usize,
    value: Option<f64>,
}

#[derive(Debug, Clone)]
struct ExcludedWellEntry {
    label: String,
    raw_value: Option<f64>,
}

#[derive(Debug, Clone)]
struct ControlWellEntry {
    label: String,
    value: Option<f64>,
}

struct CollectedPlate {
    treatments: Vec<TreatmentPoint>,
    excluded: Vec<ExcludedWellEntry>,
    positive_controls: Vec<ControlWellEntry>,
    blanks: Vec<ControlWellEntry>,
    blank_mean_applied: Option<(f64, usize)>,
}

/// Map row-major 96/384-well plate readings and a [`PlateMapTemplate`] into a [`DataSheet`].
///
/// Supports optional blank-mean subtraction (`subtract_blank_mean = true`), excludes
/// [`PlateWellRole::Excluded`] wells from treatment and blank calculations while preserving them
/// non-destructively (as excluded cells + [`CellExclusionNote`]s in `Column`/`MultipleVariables`,
/// or QC [`FloatingNote`] annotations in `Xy`/`Grouped`), and projects onto `target_kind`.
pub fn map_plate_to_datasheet(
    title: &str,
    geometry: PlateGeometry,
    values: &[Option<f64>],
    template: &PlateMapTemplate,
    target_kind: TargetArchetypeKind,
    subtract_blank_mean: bool,
) -> Result<DataSheet, TransformError> {
    validate_title(title)?;
    if template.geometry != geometry {
        return Err(TransformError::InvalidSpec {
            message: format!(
                "template geometry ({}x{}) does not match requested plate geometry ({}x{})",
                template.geometry.rows(),
                template.geometry.cols(),
                geometry.rows(),
                geometry.cols()
            ),
        });
    }
    template.validate()?;
    let expected = geometry.well_count();
    if values.len() != expected {
        return Err(TransformError::InvalidInput { message: format!("plate values slice has {} entries, expected {expected}", values.len()) });
    }
    for (idx, maybe_val) in values.iter().enumerate() {
        if let Some(v) = maybe_val
            && !v.is_finite()
        {
            let label = geometry.label_for_index(idx).unwrap_or_else(|_| format!("#{idx}"));
            return Err(TransformError::InvalidInput { message: format!("plate value at well {label} is non-finite ({v})") });
        }
    }

    let collected = collect_plate_records(geometry, values, template, subtract_blank_mean)?;
    match target_kind {
        TargetArchetypeKind::Xy => build_plate_xy_sheet(title, template, &collected),
        TargetArchetypeKind::Grouped => build_plate_grouped_sheet(title, template, &collected),
        TargetArchetypeKind::Column => build_plate_column_sheet(title, template, &collected, subtract_blank_mean),
        TargetArchetypeKind::MultipleVariables => build_plate_mv_sheet(title, geometry, values, template, &collected),
        TargetArchetypeKind::Contingency => {
            Err(TransformError::IncompatibleTable { operation: "map_plate_to_datasheet", kind: TableKind::Contingency })
        }
    }
}

fn collect_plate_records(
    geometry: PlateGeometry,
    values: &[Option<f64>],
    template: &PlateMapTemplate,
    subtract_blank_mean: bool,
) -> Result<CollectedPlate, TransformError> {
    let mut raw_blanks = Vec::new();
    for (role, maybe_val) in template.wells.iter().zip(values.iter()) {
        if matches!(role, PlateWellRole::Blank)
            && let Some(v) = maybe_val
        {
            raw_blanks.push(*v);
        }
    }

    let (blank_offset, blank_mean_applied) = if subtract_blank_mean {
        if raw_blanks.is_empty() {
            return Err(TransformError::InvalidInput {
                message: String::from("subtract_blank_mean=true requires at least one non-empty Blank well"),
            });
        }
        let moments = SummaryMoments::from_slice(&raw_blanks)?;
        (moments.mean, Some((moments.mean, moments.count)))
    } else {
        (0.0, None)
    };

    let mut treatments = Vec::new();
    let mut excluded = Vec::new();
    let mut positive_controls = Vec::new();
    let mut blanks = Vec::new();

    for (idx, (role, maybe_val)) in template.wells.iter().zip(values.iter()).enumerate() {
        let label = geometry.label_for_index(idx)?;
        match role {
            PlateWellRole::Treatment { group_name, dilution_x, replicate } => {
                let adj = subtract_opt(*maybe_val, blank_offset, &label)?;
                treatments.push(TreatmentPoint {
                    group_name: group_name.trim().to_owned(),
                    dilution_x: *dilution_x,
                    replicate: *replicate,
                    well_idx: idx,
                    value: adj,
                });
            }
            PlateWellRole::Blank => {
                let adj = subtract_opt(*maybe_val, blank_offset, &label)?;
                blanks.push(ControlWellEntry { label, value: adj });
            }
            PlateWellRole::PositiveControl => {
                let adj = subtract_opt(*maybe_val, blank_offset, &label)?;
                positive_controls.push(ControlWellEntry { label, value: adj });
            }
            PlateWellRole::Excluded => {
                excluded.push(ExcludedWellEntry { label, raw_value: *maybe_val });
            }
        }
    }

    Ok(CollectedPlate { treatments, excluded, positive_controls, blanks, blank_mean_applied })
}

fn subtract_opt(maybe_val: Option<f64>, offset: f64, label: &str) -> Result<Option<f64>, TransformError> {
    match maybe_val {
        None => Ok(None),
        Some(v) => {
            let adj = v - offset;
            if !adj.is_finite() {
                return Err(TransformError::InvalidInput { message: format!("blank-subtracted value at well {label} overflowed to non-finite") });
            }
            Ok(Some(adj))
        }
    }
}

struct TreatmentGrid {
    groups: Vec<String>,
    x_values: Vec<f64>,
    subcol_count: usize,
    /// `(row_idx, group_idx, sub_idx)` -> numeric value
    cells: BTreeMap<(usize, usize, usize), f64>,
}

fn build_treatment_grid(treatments: &[TreatmentPoint]) -> Result<TreatmentGrid, TransformError> {
    if treatments.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("plate template has no Treatment wells") });
    }

    let mut groups = Vec::new();
    let mut group_map: BTreeMap<String, usize> = BTreeMap::new();
    let mut x_by_bits: BTreeMap<u64, f64> = BTreeMap::new();
    let mut rep_set: BTreeSet<usize> = BTreeSet::new();

    for pt in treatments {
        if !group_map.contains_key(&pt.group_name) {
            let idx = groups.len();
            groups.push(pt.group_name.clone());
            group_map.insert(pt.group_name.clone(), idx);
        }
        let norm_x = normalize_zero(pt.dilution_x);
        x_by_bits.entry(norm_x.to_bits()).or_insert(norm_x);
        rep_set.insert(pt.replicate);
    }

    let mut x_values: Vec<f64> = x_by_bits.into_values().collect();
    x_values.sort_by(f64::total_cmp);

    let x_row_by_bits: BTreeMap<u64, usize> = x_values.iter().enumerate().map(|(r, &x)| (normalize_zero(x).to_bits(), r)).collect();
    let rep_slot_map: BTreeMap<usize, usize> = rep_set.into_iter().enumerate().map(|(s, rep)| (rep, s)).collect();

    let mut sorted_pts: Vec<&TreatmentPoint> = treatments.iter().collect();
    sorted_pts.sort_by_key(|p| (p.replicate, p.well_idx));

    let mut cells: BTreeMap<(usize, usize, usize), f64> = BTreeMap::new();
    let mut max_sub = 0_usize;
    for pt in sorted_pts {
        let Some(&g) = group_map.get(&pt.group_name) else { continue };
        let norm_x = normalize_zero(pt.dilution_x);
        let Some(&r) = x_row_by_bits.get(&norm_x.to_bits()) else { continue };
        let Some(&base_slot) = rep_slot_map.get(&pt.replicate) else { continue };
        let Some(val) = pt.value else { continue };

        let mut slot = base_slot;
        while cells.contains_key(&(r, g, slot)) {
            slot += 1;
        }
        cells.insert((r, g, slot), val);
        if slot > max_sub {
            max_sub = slot;
        }
    }

    let subcol_count = (max_sub + 1).max(rep_slot_map.len()).max(1);
    validate_data_sets(groups.len())?;
    validate_rows(x_values.len())?;
    validate_subcolumns(subcol_count)?;

    Ok(TreatmentGrid { groups, x_values, subcol_count, cells })
}

fn build_plate_xy_sheet(title: &str, template: &PlateMapTemplate, collected: &CollectedPlate) -> Result<DataSheet, TransformError> {
    let grid = build_treatment_grid(&collected.treatments)?;
    let mut xy = XyTable::new_with_x_mode(
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
        SubcolumnMode::RawReplicates(grid.subcol_count),
        grid.groups.len(),
        grid.x_values.len(),
    )?;
    xy.x_title = non_empty_or(&template.x_title, "X");
    for (r, &x) in grid.x_values.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
    }
    for (c, name) in grid.groups.iter().enumerate() {
        xy.data_sets[c].title = name.clone();
    }
    for (&(r, c, s), &val) in &grid.cells {
        xy.data_sets[c].cells[r][s] = Cell::number(val);
    }

    let mut sheet = DataSheet::new(make_sheet_id("plate", title), title, TableArchetype::Xy(xy))?;
    append_plate_floating_notes(&mut sheet, collected);
    Ok(sheet)
}

fn build_plate_grouped_sheet(title: &str, template: &PlateMapTemplate, collected: &CollectedPlate) -> Result<DataSheet, TransformError> {
    let grid = build_treatment_grid(&collected.treatments)?;
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(grid.subcol_count), grid.groups.len(), grid.x_values.len())?;
    grouped.row_factor_title = non_empty_or(&template.x_title, "Dilution");
    grouped.column_factor_title = String::from("Group");
    grouped.row_titles = grid.x_values.iter().map(|x| format_compact_f64(*x)).collect();
    for (c, name) in grid.groups.iter().enumerate() {
        grouped.data_sets[c].title = name.clone();
    }
    for (&(r, c, s), &val) in &grid.cells {
        grouped.data_sets[c].cells[r][s] = Cell::number(val);
    }

    let mut sheet = DataSheet::new(make_sheet_id("plate", title), title, TableArchetype::Grouped(grouped))?;
    append_plate_floating_notes(&mut sheet, collected);
    Ok(sheet)
}

fn build_plate_column_sheet(
    title: &str,
    _template: &PlateMapTemplate,
    collected: &CollectedPlate,
    subtract_blank_mean: bool,
) -> Result<DataSheet, TransformError> {
    let mut col_names: Vec<String> = Vec::new();
    let mut col_map: BTreeMap<String, usize> = BTreeMap::new();
    let mut col_cells: Vec<Vec<Cell>> = Vec::new();

    let mut sorted_pts: Vec<&TreatmentPoint> = collected.treatments.iter().collect();
    sorted_pts
        .sort_by(|a, b| a.dilution_x.total_cmp(&b.dilution_x).then_with(|| a.replicate.cmp(&b.replicate)).then_with(|| a.well_idx.cmp(&b.well_idx)));

    for pt in sorted_pts {
        let Some(val) = pt.value else { continue };
        let c = ensure_column_bucket(&pt.group_name, &mut col_names, &mut col_map, &mut col_cells);
        col_cells[c].push(Cell::number(val));
    }

    append_control_column("Positive Control", &collected.positive_controls, &mut col_names, &mut col_map, &mut col_cells);
    if !subtract_blank_mean {
        append_control_column("Blank", &collected.blanks, &mut col_names, &mut col_map, &mut col_cells);
    }

    let mut notes = Vec::new();
    let excluded_with_values: Vec<&ExcludedWellEntry> = collected.excluded.iter().filter(|e| e.raw_value.is_some()).collect();
    if !excluded_with_values.is_empty() {
        let c = ensure_column_bucket("Excluded", &mut col_names, &mut col_map, &mut col_cells);
        for entry in excluded_with_values {
            if let Some(val) = entry.raw_value {
                let r = col_cells[c].len();
                col_cells[c].push(Cell::excluded_number(val));
                notes.push(CellExclusionNote {
                    coord: CellCoord::new(r, c, 0),
                    reason_tag: String::from("plate-well-excluded"),
                    note: format!("Well {} excluded by plate map", entry.label),
                    timestamp_utc: String::new(),
                });
            }
        }
    }

    if col_names.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no plate observations available to populate Column sheet") });
    }

    let rows = col_cells.iter().map(Vec::len).max().unwrap_or(0).max(1);
    validate_data_sets(col_names.len())?;
    validate_rows(rows)?;
    let mut table = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, col_names.len(), rows)?;
    for (c, name) in col_names.into_iter().enumerate() {
        table.data_sets[c].title = name;
        for (r, cell) in col_cells[c].iter().enumerate() {
            table.data_sets[c].cells[r][0] = cell.clone();
        }
    }

    let mut sheet = DataSheet::new(make_sheet_id("plate", title), title, TableArchetype::Column(table))?;
    sheet.exclusion_notes = notes;
    sheet.exclusion_notes.sort_by_key(|n| n.coord);
    append_plate_floating_notes(&mut sheet, collected);
    Ok(sheet)
}

fn ensure_column_bucket(name: &str, col_names: &mut Vec<String>, col_map: &mut BTreeMap<String, usize>, col_cells: &mut Vec<Vec<Cell>>) -> usize {
    if let Some(&idx) = col_map.get(name) {
        idx
    } else {
        let idx = col_names.len();
        col_names.push(name.to_owned());
        col_map.insert(name.to_owned(), idx);
        col_cells.push(Vec::new());
        idx
    }
}

fn append_control_column(
    name: &str,
    entries: &[ControlWellEntry],
    col_names: &mut Vec<String>,
    col_map: &mut BTreeMap<String, usize>,
    col_cells: &mut Vec<Vec<Cell>>,
) {
    let vals: Vec<f64> = entries.iter().filter_map(|e| e.value).collect();
    if vals.is_empty() {
        return;
    }
    let c = ensure_column_bucket(name, col_names, col_map, col_cells);
    for v in vals {
        col_cells[c].push(Cell::number(v));
    }
}

fn build_plate_mv_sheet(
    title: &str,
    geometry: PlateGeometry,
    values: &[Option<f64>],
    template: &PlateMapTemplate,
    collected: &CollectedPlate,
) -> Result<DataSheet, TransformError> {
    let rows = geometry.well_count();
    let cols = geometry.cols();
    let blank_offset = collected.blank_mean_applied.map_or(0.0, |(m, _)| m);
    let mut mv = MultipleVariablesTable::new(8, rows)?;
    let titles = ["Well", "PlateRow", "PlateCol", "Role", "Group", &non_empty_or(&template.x_title, "DilutionX"), "Replicate", "Value"];
    for (idx, var_title) in titles.iter().enumerate() {
        mv.variables[idx].title = (*var_title).to_owned();
    }

    let mut notes = Vec::new();
    for (r, (role, maybe_val)) in template.wells.iter().zip(values.iter()).enumerate() {
        let plate_row = r / cols;
        let plate_col = r % cols;
        let label = geometry.well_label(plate_row, plate_col)?;
        let row_byte = u8::try_from(plate_row).unwrap_or(0);
        let row_letter = char::from(b'A'.saturating_add(row_byte));

        mv.variables[0].cells[r] = Cell::text(label.clone());
        mv.variables[1].cells[r] = Cell::text(row_letter.to_string());
        #[allow(clippy::cast_precision_loss)]
        {
            mv.variables[2].cells[r] = Cell::number((plate_col + 1) as f64);
        }

        match role {
            PlateWellRole::Treatment { group_name, dilution_x, replicate } => {
                mv.variables[3].cells[r] = Cell::text("Treatment");
                mv.variables[4].cells[r] = Cell::text(group_name.trim());
                mv.variables[5].cells[r] = Cell::number(*dilution_x);
                #[allow(clippy::cast_precision_loss)]
                {
                    mv.variables[6].cells[r] = Cell::number(*replicate as f64);
                }
                if let Some(val) = maybe_val {
                    mv.variables[7].cells[r] = Cell::number(*val - blank_offset);
                }
            }
            PlateWellRole::Blank => {
                mv.variables[3].cells[r] = Cell::text("Blank");
                mv.variables[4].cells[r] = Cell::text("Blank");
                if let Some(val) = maybe_val {
                    mv.variables[7].cells[r] = Cell::number(*val - blank_offset);
                }
            }
            PlateWellRole::PositiveControl => {
                mv.variables[3].cells[r] = Cell::text("PositiveControl");
                mv.variables[4].cells[r] = Cell::text("PositiveControl");
                if let Some(val) = maybe_val {
                    mv.variables[7].cells[r] = Cell::number(*val - blank_offset);
                }
            }
            PlateWellRole::Excluded => {
                mv.variables[3].cells[r] = Cell::text("Excluded");
                mv.variables[4].cells[r] = Cell::text("Excluded");
                mv.variables[7].cells[r] = match maybe_val {
                    Some(v) => Cell::excluded_number(*v),
                    None => Cell { value: CellValue::Empty, excluded: true },
                };
                notes.push(CellExclusionNote {
                    coord: CellCoord::new(r, 7, 0),
                    reason_tag: String::from("plate-well-excluded"),
                    note: format!("Well {label} excluded by plate map"),
                    timestamp_utc: String::new(),
                });
            }
        }
    }
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    let mut sheet = DataSheet::new(make_sheet_id("plate", title), title, TableArchetype::MultipleVariables(mv))?;
    sheet.exclusion_notes = notes;
    sheet.exclusion_notes.sort_by_key(|n| n.coord);
    append_plate_floating_notes(&mut sheet, collected);
    Ok(sheet)
}

fn append_plate_floating_notes(sheet: &mut DataSheet, collected: &CollectedPlate) {
    let mut lines = Vec::new();
    if let Some((mean, count)) = collected.blank_mean_applied {
        lines.push(format!("Blank mean subtracted: {} (n={count} Blank wells)", format_compact_f64(mean)));
    }
    let excluded_with_val: Vec<String> =
        collected.excluded.iter().filter_map(|e| e.raw_value.map(|v| format!("{}={}", e.label, format_compact_f64(v)))).collect();
    if !excluded_with_val.is_empty() {
        lines.push(format!("Excluded wells with readings: {}", excluded_with_val.join(", ")));
    }
    let pos_vals: Vec<String> =
        collected.positive_controls.iter().filter_map(|e| e.value.map(|v| format!("{}={}", e.label, format_compact_f64(v)))).collect();
    if !pos_vals.is_empty() {
        lines.push(format!("Positive controls: {}", pos_vals.join(", ")));
    }
    if !lines.is_empty() {
        sheet.floating_notes.push(FloatingNote::new(
            format!("{}-plate-qc", sheet.id.as_str()),
            RectPt::new(12.0, 12.0, 260.0, 96.0),
            lines.join("\n"),
        ));
    }
}

/// Column index mapping describing how a tidy long table should be pivoted into a wide [`DataSheet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TidyColumnMapping {
    pub x_col: Option<usize>,
    pub group_col: usize,
    pub replicate_col: Option<usize>,
    pub value_col: usize,
    pub excluded_col: Option<usize>,
}

#[derive(Debug, Clone)]
struct TidyRecord {
    x_token: Option<String>,
    x_num: Option<f64>,
    x_excluded: bool,
    group: String,
    replicate: String,
    value: Cell,
    exclusion_note: Option<(String, String)>,
}

/// Pivot a long/tidy tabular dataset into a wide [`DataSheet`] (`Xy`, `Column`, `Grouped`, or `MultipleVariables`).
///
/// Handles `decimal_sep` (`'.'` or `','`), trailing `'*'` exclusion markers, optional
/// `excluded_col` flags/reasons, and ragged/short/missing rows without panicking.
pub fn pivot_tidy_long_to_datasheet(
    title: &str,
    headers: &[String],
    rows: &[Vec<String>],
    mapping: &TidyColumnMapping,
    target_kind: TargetArchetypeKind,
    decimal_sep: char,
) -> Result<DataSheet, TransformError> {
    validate_title(title)?;
    if decimal_sep != '.' && decimal_sep != ',' {
        return Err(TransformError::InvalidSpec { message: format!("unsupported decimal separator `{decimal_sep}` (expected '.' or ',')") });
    }
    validate_tidy_mapping(headers, mapping, target_kind)?;
    let records = collect_tidy_records(rows, mapping, target_kind, decimal_sep)?;
    if records.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no valid non-empty tidy rows found to pivot") });
    }

    match target_kind {
        TargetArchetypeKind::Column => pivot_tidy_to_column(title, &records),
        TargetArchetypeKind::Xy => pivot_tidy_to_xy(title, headers, mapping, &records),
        TargetArchetypeKind::Grouped => pivot_tidy_to_grouped(title, headers, mapping, &records),
        TargetArchetypeKind::MultipleVariables => pivot_tidy_to_mv(title, headers, mapping, &records),
        TargetArchetypeKind::Contingency => {
            Err(TransformError::IncompatibleTable { operation: "pivot_tidy_long_to_datasheet", kind: TableKind::Contingency })
        }
    }
}

fn validate_tidy_mapping(headers: &[String], mapping: &TidyColumnMapping, target_kind: TargetArchetypeKind) -> Result<(), TransformError> {
    if target_kind == TargetArchetypeKind::Xy && mapping.x_col.is_none() {
        return Err(TransformError::InvalidSpec { message: String::from("pivoting tidy data to Xy requires TidyColumnMapping.x_col") });
    }
    if !headers.is_empty() {
        let len = headers.len();
        check_col_idx("group_col", mapping.group_col, len)?;
        check_col_idx("value_col", mapping.value_col, len)?;
        if let Some(x) = mapping.x_col {
            check_col_idx("x_col", x, len)?;
        }
        if let Some(r) = mapping.replicate_col {
            check_col_idx("replicate_col", r, len)?;
        }
        if let Some(e) = mapping.excluded_col {
            check_col_idx("excluded_col", e, len)?;
        }
    }
    Ok(())
}

fn check_col_idx(name: &str, idx: usize, header_len: usize) -> Result<(), TransformError> {
    if idx >= header_len {
        return Err(TransformError::InvalidSpec { message: format!("tidy mapping `{name}` index {idx} out of bounds for {header_len} header(s)") });
    }
    Ok(())
}

fn collect_tidy_records(
    rows: &[Vec<String>],
    mapping: &TidyColumnMapping,
    target_kind: TargetArchetypeKind,
    decimal_sep: char,
) -> Result<Vec<TidyRecord>, TransformError> {
    let mut records = Vec::new();
    for (row_idx, row) in rows.iter().enumerate() {
        let group = row_cell_trimmed(row, mapping.group_col);
        if group.is_empty() {
            continue;
        }

        let (x_token, x_num, x_excluded) = match mapping.x_col {
            None => (None, None, false),
            Some(xc) => {
                let raw_x = row_cell_trimmed(row, xc);
                if raw_x.is_empty() || is_missing_token(raw_x) {
                    if target_kind == TargetArchetypeKind::Xy {
                        continue;
                    }
                    (None, None, false)
                } else {
                    let parsed_x = parse_tidy_cell(raw_x, decimal_sep);
                    let num_x = parsed_x.active_number().or_else(|| match parsed_x.value {
                        CellValue::Number(n) if n.is_finite() => Some(normalize_zero(n)),
                        _ => None,
                    });
                    if target_kind == TargetArchetypeKind::Xy && num_x.is_none() {
                        return Err(TransformError::InvalidInput { message: format!("non-numeric X value `{raw_x}` at tidy row {}", row_idx + 1) });
                    }
                    (Some(raw_x.trim_end_matches('*').trim().to_owned()), num_x, parsed_x.excluded)
                }
            }
        };

        let raw_val = row_cell_trimmed(row, mapping.value_col);
        let mut cell = parse_tidy_cell(raw_val, decimal_sep);
        let mut exclusion_note =
            if cell.excluded { Some((String::from("asterisk-excluded"), format!("Tidy row {} marked with '*'", row_idx + 1))) } else { None };

        if let Some(ec) = mapping.excluded_col {
            let flag = row_cell_trimmed(row, ec);
            if let Some(note_text) = parse_exclusion_flag(flag, row_idx) {
                cell.excluded = true;
                exclusion_note = Some((String::from("excluded-column"), note_text));
            }
        }

        let replicate = mapping.replicate_col.map(|rc| row_cell_trimmed(row, rc).to_owned()).unwrap_or_default();

        if target_kind == TargetArchetypeKind::Grouped && mapping.x_col.is_some() && x_token.is_none() && cell.is_empty() && !cell.excluded {
            continue;
        }

        records.push(TidyRecord { x_token, x_num, x_excluded, group: group.to_owned(), replicate, value: cell, exclusion_note });
    }
    Ok(records)
}

fn row_cell_trimmed(row: &[String], col: usize) -> &str {
    row.get(col).map_or("", |s| s.trim())
}

fn is_missing_token(raw: &str) -> bool {
    raw.is_empty()
        || raw.eq_ignore_ascii_case("na")
        || raw.eq_ignore_ascii_case("n/a")
        || raw.eq_ignore_ascii_case("null")
        || raw.eq_ignore_ascii_case("nan")
}

fn parse_tidy_cell(raw: &str, decimal_sep: char) -> Cell {
    let trimmed = raw.trim();
    if is_missing_token(trimmed) {
        return Cell::empty();
    }
    Cell::parse_import_token(trimmed, decimal_sep, true)
}

fn parse_exclusion_flag(flag: &str, row_idx: usize) -> Option<String> {
    let trimmed = flag.trim();
    if trimmed.is_empty()
        || trimmed == "-"
        || trimmed == "0"
        || trimmed.eq_ignore_ascii_case("false")
        || trimmed.eq_ignore_ascii_case("no")
        || trimmed.eq_ignore_ascii_case("n")
        || trimmed.eq_ignore_ascii_case("f")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("na")
    {
        return None;
    }
    if trimmed == "1"
        || trimmed == "*"
        || trimmed.eq_ignore_ascii_case("true")
        || trimmed.eq_ignore_ascii_case("yes")
        || trimmed.eq_ignore_ascii_case("y")
        || trimmed.eq_ignore_ascii_case("t")
        || trimmed.eq_ignore_ascii_case("x")
        || trimmed.eq_ignore_ascii_case("excluded")
        || trimmed.eq_ignore_ascii_case("exclude")
    {
        return Some(format!("Tidy row {} flagged excluded ({trimmed})", row_idx + 1));
    }
    Some(trimmed.to_owned())
}

type ExclusionDetail = (String, String);
type PlacedCell = (Cell, Option<ExclusionDetail>);
type PlacedCellGrid = BTreeMap<(usize, usize, usize), PlacedCell>;

fn pivot_tidy_to_column(title: &str, records: &[TidyRecord]) -> Result<DataSheet, TransformError> {
    let mut group_names: Vec<String> = Vec::new();
    let mut group_map: BTreeMap<String, usize> = BTreeMap::new();
    let mut columns: Vec<Vec<PlacedCell>> = Vec::new();

    for rec in records {
        let g = ensure_group_slot(&rec.group, &mut group_names, &mut group_map);
        while columns.len() <= g {
            columns.push(Vec::new());
        }
        columns[g].push((rec.value.clone(), rec.exclusion_note.clone()));
    }

    let rows = columns.iter().map(Vec::len).max().unwrap_or(0).max(1);
    validate_data_sets(group_names.len())?;
    validate_rows(rows)?;
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, group_names.len(), rows)?;
    let mut notes = Vec::new();
    for (c, name) in group_names.into_iter().enumerate() {
        col.data_sets[c].title = name;
        for (r, (cell, maybe_note)) in columns[c].iter().enumerate() {
            col.data_sets[c].cells[r][0] = cell.clone();
            if cell.excluded {
                let (tag, note) = maybe_note.clone().unwrap_or_else(|| (String::from("excluded"), String::from("Excluded value")));
                notes.push(CellExclusionNote { coord: CellCoord::new(r, c, 0), reason_tag: tag, note, timestamp_utc: String::new() });
            }
        }
    }

    let mut sheet = DataSheet::new(make_sheet_id("tidy", title), title, TableArchetype::Column(col))?;
    notes.sort_by_key(|n| n.coord);
    sheet.exclusion_notes = notes;
    Ok(sheet)
}

fn pivot_tidy_to_xy(title: &str, headers: &[String], mapping: &TidyColumnMapping, records: &[TidyRecord]) -> Result<DataSheet, TransformError> {
    let mut group_names = Vec::new();
    let mut group_map = BTreeMap::new();
    let mut x_by_bits: BTreeMap<u64, f64> = BTreeMap::new();
    let mut x_excluded_bits: BTreeSet<u64> = BTreeSet::new();

    for rec in records {
        ensure_group_slot(&rec.group, &mut group_names, &mut group_map);
        if let Some(x) = rec.x_num {
            let nx = normalize_zero(x);
            let bits = nx.to_bits();
            x_by_bits.entry(bits).or_insert(nx);
            if rec.x_excluded {
                x_excluded_bits.insert(bits);
            }
        }
    }
    if x_by_bits.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no finite numeric X entries available for Xy pivot") });
    }

    let mut x_vals: Vec<f64> = x_by_bits.into_values().collect();
    x_vals.sort_by(f64::total_cmp);
    let x_row_by_bits: BTreeMap<u64, usize> = x_vals.iter().enumerate().map(|(r, &x)| (normalize_zero(x).to_bits(), r)).collect();

    let (use_named_reps, rep_slots) = build_replicate_slot_map(records, mapping.replicate_col.is_some());
    let mut placed: PlacedCellGrid = BTreeMap::new();
    let mut next_auto: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut max_sub = 0_usize;

    for rec in records {
        let Some(&g) = group_map.get(&rec.group) else { continue };
        let Some(x) = rec.x_num else { continue };
        let Some(&r) = x_row_by_bits.get(&normalize_zero(x).to_bits()) else { continue };

        let slot = if use_named_reps {
            let mut s = *rep_slots.get(&rec.replicate).unwrap_or(&0);
            while placed.contains_key(&(r, g, s)) {
                s += 1;
            }
            s
        } else {
            let cursor = next_auto.entry((r, g)).or_insert(0);
            let s = *cursor;
            *cursor += 1;
            s
        };
        placed.insert((r, g, slot), (rec.value.clone(), rec.exclusion_note.clone()));
        if slot > max_sub {
            max_sub = slot;
        }
    }

    let subcol_count = (max_sub + 1).max(rep_slots.len()).max(1);
    validate_data_sets(group_names.len())?;
    validate_rows(x_vals.len())?;
    validate_subcolumns(subcol_count)?;

    let mut xy = XyTable::new_with_x_mode(
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
        SubcolumnMode::RawReplicates(subcol_count),
        group_names.len(),
        x_vals.len(),
    )?;
    if let Some(xc) = mapping.x_col
        && let Some(h) = headers.get(xc)
        && !h.trim().is_empty()
    {
        xy.x_title = h.trim().to_owned();
    }
    for (r, &x) in x_vals.iter().enumerate() {
        xy.x_cells[r][0] = Cell { value: CellValue::Number(x), excluded: x_excluded_bits.contains(&normalize_zero(x).to_bits()) };
    }
    for (c, name) in group_names.into_iter().enumerate() {
        xy.data_sets[c].title = name;
    }

    let notes = fill_y_cells_and_notes(&mut xy.data_sets, placed);
    let mut sheet = DataSheet::new(make_sheet_id("tidy", title), title, TableArchetype::Xy(xy))?;
    sheet.exclusion_notes = notes;
    Ok(sheet)
}

fn pivot_tidy_to_grouped(title: &str, headers: &[String], mapping: &TidyColumnMapping, records: &[TidyRecord]) -> Result<DataSheet, TransformError> {
    let mut group_names = Vec::new();
    let mut group_map = BTreeMap::new();
    for rec in records {
        ensure_group_slot(&rec.group, &mut group_names, &mut group_map);
    }

    let (row_titles, row_lookup) = build_grouped_row_index(mapping.x_col.is_some(), records);
    let (use_named_reps, rep_slots) = build_replicate_slot_map(records, mapping.replicate_col.is_some());

    let mut placed: PlacedCellGrid = BTreeMap::new();
    let mut next_row_or_sub: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut max_sub = 0_usize;
    let mut extra_rows = row_titles.len();

    for rec in records {
        let Some(&g) = group_map.get(&rec.group) else { continue };
        if mapping.x_col.is_some() {
            let key = grouped_row_key(rec);
            let Some(&r) = row_lookup.get(&key) else { continue };
            let s = if use_named_reps {
                let mut slot = *rep_slots.get(&rec.replicate).unwrap_or(&0);
                while placed.contains_key(&(r, g, slot)) {
                    slot += 1;
                }
                slot
            } else {
                let cursor = next_row_or_sub.entry((r, g)).or_insert(0);
                let slot = *cursor;
                *cursor += 1;
                slot
            };
            placed.insert((r, g, s), (rec.value.clone(), rec.exclusion_note.clone()));
            if s > max_sub {
                max_sub = s;
            }
        } else {
            let s = if use_named_reps { *rep_slots.get(&rec.replicate).unwrap_or(&0) } else { 0 };
            let cursor = next_row_or_sub.entry((g, s)).or_insert(0);
            let r = *cursor;
            *cursor += 1;
            if r >= extra_rows {
                extra_rows = r + 1;
            }
            if s > max_sub {
                max_sub = s;
            }
            placed.insert((r, g, s), (rec.value.clone(), rec.exclusion_note.clone()));
        }
    }

    let row_count = extra_rows.max(1);
    let mut final_row_titles = row_titles;
    while final_row_titles.len() < row_count {
        final_row_titles.push(format!("Row {}", final_row_titles.len() + 1));
    }

    let subcol_count = (max_sub + 1).max(rep_slots.len()).max(1);
    validate_data_sets(group_names.len())?;
    validate_rows(row_count)?;
    validate_subcolumns(subcol_count)?;

    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(subcol_count), group_names.len(), row_count)?;
    grouped.row_titles = final_row_titles;
    if let Some(xc) = mapping.x_col
        && let Some(h) = headers.get(xc)
        && !h.trim().is_empty()
    {
        grouped.row_factor_title = h.trim().to_owned();
    }
    if let Some(h) = headers.get(mapping.group_col)
        && !h.trim().is_empty()
    {
        grouped.column_factor_title = h.trim().to_owned();
    }
    for (c, name) in group_names.into_iter().enumerate() {
        grouped.data_sets[c].title = name;
    }

    let notes = fill_y_cells_and_notes(&mut grouped.data_sets, placed);
    let mut sheet = DataSheet::new(make_sheet_id("tidy", title), title, TableArchetype::Grouped(grouped))?;
    sheet.exclusion_notes = notes;
    Ok(sheet)
}

fn pivot_tidy_to_mv(title: &str, headers: &[String], mapping: &TidyColumnMapping, records: &[TidyRecord]) -> Result<DataSheet, TransformError> {
    let rows = records.len();
    validate_rows(rows)?;
    let has_x = mapping.x_col.is_some();
    let has_rep = mapping.replicate_col.is_some();
    let var_count = 2 + usize::from(has_x) + usize::from(has_rep);
    let mut mv = MultipleVariablesTable::new(var_count, rows)?;

    let mut next_idx = 0_usize;
    let x_var = has_x.then(|| {
        let i = next_idx;
        next_idx += 1;
        mv.variables[i].title = header_or(headers, mapping.x_col, "X");
        i
    });
    let grp_var = next_idx;
    next_idx += 1;
    mv.variables[grp_var].title = header_or(headers, Some(mapping.group_col), "Group");
    let rep_var = has_rep.then(|| {
        let i = next_idx;
        next_idx += 1;
        mv.variables[i].title = header_or(headers, mapping.replicate_col, "Replicate");
        i
    });
    let val_var = next_idx;
    mv.variables[val_var].title = header_or(headers, Some(mapping.value_col), "Value");

    let mut notes = Vec::new();
    for (r, rec) in records.iter().enumerate() {
        if let Some(xi) = x_var {
            mv.variables[xi].cells[r] = match (rec.x_num, rec.x_token.as_ref()) {
                (Some(n), _) => Cell { value: CellValue::Number(n), excluded: rec.x_excluded },
                (None, Some(tok)) => Cell { value: CellValue::Text(tok.clone()), excluded: rec.x_excluded },
                (None, None) => Cell::empty(),
            };
        }
        mv.variables[grp_var].cells[r] = Cell::text(rec.group.clone());
        if let Some(ri) = rep_var
            && !rec.replicate.is_empty()
        {
            mv.variables[ri].cells[r] = match rec.replicate.parse::<f64>() {
                Ok(n) if n.is_finite() => Cell::number(n),
                _ => Cell::text(rec.replicate.clone()),
            };
        }
        mv.variables[val_var].cells[r] = rec.value.clone();
        if rec.value.excluded {
            let (tag, note) = rec.exclusion_note.clone().unwrap_or_else(|| (String::from("excluded"), String::from("Excluded value")));
            notes.push(CellExclusionNote { coord: CellCoord::new(r, val_var, 0), reason_tag: tag, note, timestamp_utc: String::new() });
        }
    }
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    let mut sheet = DataSheet::new(make_sheet_id("tidy", title), title, TableArchetype::MultipleVariables(mv))?;
    notes.sort_by_key(|n| n.coord);
    sheet.exclusion_notes = notes;
    Ok(sheet)
}

fn header_or(headers: &[String], col: Option<usize>, fallback: &str) -> String {
    col.and_then(|c| headers.get(c)).map(|s| s.trim()).filter(|s| !s.is_empty()).map_or_else(|| String::from(fallback), str::to_owned)
}

fn fill_y_cells_and_notes(data_sets: &mut [lumencraft_doc::DataSetColumn], placed: PlacedCellGrid) -> Vec<CellExclusionNote> {
    let mut notes = Vec::new();
    for ((r, c, s), (cell, maybe_note)) in placed {
        data_sets[c].cells[r][s] = cell.clone();
        if cell.excluded {
            let (tag, note) = maybe_note.unwrap_or_else(|| (String::from("excluded"), String::from("Excluded value")));
            notes.push(CellExclusionNote { coord: CellCoord::new(r, c, s), reason_tag: tag, note, timestamp_utc: String::new() });
        }
    }
    notes.sort_by_key(|n| n.coord);
    notes
}

fn ensure_group_slot(name: &str, names: &mut Vec<String>, map: &mut BTreeMap<String, usize>) -> usize {
    if let Some(&idx) = map.get(name) {
        idx
    } else {
        let idx = names.len();
        names.push(name.to_owned());
        map.insert(name.to_owned(), idx);
        idx
    }
}

fn build_replicate_slot_map(records: &[TidyRecord], has_replicate_col: bool) -> (bool, BTreeMap<String, usize>) {
    if !has_replicate_col {
        return (false, BTreeMap::new());
    }
    let keys: BTreeSet<String> = records.iter().map(|r| r.replicate.clone()).collect();
    if keys.is_empty() {
        return (false, BTreeMap::new());
    }
    let mut ordered: Vec<String> = keys.into_iter().collect();
    ordered.sort_by(|a, b| match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(na), Ok(nb)) if na.is_finite() && nb.is_finite() => na.total_cmp(&nb).then_with(|| a.cmp(b)),
        _ => a.cmp(b),
    });
    let map = ordered.into_iter().enumerate().map(|(idx, k)| (k, idx)).collect();
    (true, map)
}

fn grouped_row_key(rec: &TidyRecord) -> String {
    match (&rec.x_token, rec.x_num) {
        (Some(tok), _) => tok.clone(),
        (None, Some(n)) => format_compact_f64(n),
        (None, None) => String::from("(Missing)"),
    }
}

fn build_grouped_row_index(has_x_col: bool, records: &[TidyRecord]) -> (Vec<String>, BTreeMap<String, usize>) {
    if !has_x_col {
        return (Vec::new(), BTreeMap::new());
    }
    let all_numeric = records.iter().filter(|r| r.x_token.is_some()).all(|r| r.x_num.is_some());

    if all_numeric && records.iter().any(|r| r.x_num.is_some()) {
        let mut by_bits: BTreeMap<u64, String> = BTreeMap::new();
        for rec in records {
            if let Some(n) = rec.x_num {
                let norm = normalize_zero(n);
                let label = rec.x_token.clone().unwrap_or_else(|| format_compact_f64(norm));
                by_bits.entry(norm.to_bits()).or_insert(label);
            }
        }
        let mut pairs: Vec<(f64, String, Vec<String>)> = Vec::new();
        let mut num_by_key: BTreeMap<u64, Vec<String>> = BTreeMap::new();
        for rec in records {
            if let Some(n) = rec.x_num {
                let norm = normalize_zero(n);
                num_by_key.entry(norm.to_bits()).or_default().push(grouped_row_key(rec));
            }
        }
        for (bits, label) in by_bits {
            let n = f64::from_bits(bits);
            let keys = num_by_key.remove(&bits).unwrap_or_default();
            pairs.push((n, label, keys));
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut titles = Vec::with_capacity(pairs.len());
        let mut lookup = BTreeMap::new();
        for (idx, (_, label, raw_keys)) in pairs.into_iter().enumerate() {
            titles.push(label);
            for k in raw_keys {
                lookup.insert(k, idx);
            }
        }
        return (titles, lookup);
    }

    let mut titles = Vec::new();
    let mut lookup = BTreeMap::new();
    for rec in records {
        let key = grouped_row_key(rec);
        lookup.entry(key.clone()).or_insert_with(|| {
            let idx = titles.len();
            titles.push(key);
            idx
        });
    }
    (titles, lookup)
}

fn normalize_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

fn non_empty_or(candidate: &str, fallback: &str) -> String {
    let trimmed = candidate.trim();
    if trimmed.is_empty() { String::from(fallback) } else { trimmed.to_owned() }
}

fn format_compact_f64(value: f64) -> String {
    let v = normalize_zero(value);
    if v.fract() == 0.0 && v.abs() < 1e12 {
        #[allow(clippy::cast_possible_truncation)]
        return format!("{}", v as i64);
    }
    format!("{v}")
}

fn make_sheet_id(prefix: &str, title: &str) -> String {
    let mut slug = String::from(prefix);
    slug.push('-');
    let mut prev_dash = true;
    for ch in title.chars() {
        if slug.len() >= 48 {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() || slug == prefix { format!("{prefix}-sheet") } else { slug }
}

/// Minimal RFC-8259 JSON `serde` serializer and deserializer so `.lctemplate` round-trips work
/// inside `lumencraft-transform` without extra crate dependencies.
mod json_codec {
    use super::*;

    const MAX_JSON_DEPTH: usize = 64;

    #[derive(Debug, Clone)]
    pub(super) struct JsonCodecError(String);

    impl JsonCodecError {
        fn new(msg: impl Into<String>) -> Self {
            Self(msg.into())
        }
    }

    impl fmt::Display for JsonCodecError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for JsonCodecError {}

    impl ser::Error for JsonCodecError {
        fn custom<T: fmt::Display>(msg: T) -> Self {
            Self(msg.to_string())
        }
    }

    impl de::Error for JsonCodecError {
        fn custom<T: fmt::Display>(msg: T) -> Self {
            Self(msg.to_string())
        }
    }

    pub(super) fn to_json_string<T: Serialize>(value: &T) -> Result<String, JsonCodecError> {
        let mut ser = JsonSerializer { out: String::new() };
        value.serialize(&mut ser)?;
        Ok(ser.out)
    }

    pub(super) fn from_json_str<'de, T: Deserialize<'de>>(input: &'de str) -> Result<T, JsonCodecError> {
        let mut parser = JsonParser { bytes: input.as_bytes(), pos: 0, depth: 0 };
        let value = T::deserialize(&mut parser)?;
        parser.skip_ws();
        if parser.pos != parser.bytes.len() {
            return Err(JsonCodecError::new(format!("trailing JSON content at byte {}", parser.pos)));
        }
        Ok(value)
    }

    struct JsonSerializer {
        out: String,
    }

    impl JsonSerializer {
        fn push_escaped_str(&mut self, s: &str) {
            self.out.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => self.out.push_str("\\\""),
                    '\\' => self.out.push_str("\\\\"),
                    '\x08' => self.out.push_str("\\b"),
                    '\x0c' => self.out.push_str("\\f"),
                    '\n' => self.out.push_str("\\n"),
                    '\r' => self.out.push_str("\\r"),
                    '\t' => self.out.push_str("\\t"),
                    c if (c as u32) < 0x20 => {
                        let _ = write!(self.out, "\\u{:04x}", c as u32);
                    }
                    c => self.out.push(c),
                }
            }
            self.out.push('"');
        }
    }

    struct JsonSeqSerializer<'a> {
        ser: &'a mut JsonSerializer,
        first: bool,
    }

    impl SerializeSeq for JsonSeqSerializer<'_> {
        type Ok = ();
        type Error = JsonCodecError;

        fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
            if !self.first {
                self.ser.out.push(',');
            }
            self.first = false;
            value.serialize(&mut *self.ser)
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.ser.out.push(']');
            Ok(())
        }
    }

    impl SerializeTuple for JsonSeqSerializer<'_> {
        type Ok = ();
        type Error = JsonCodecError;

        fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
            SerializeSeq::serialize_element(self, value)
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            SerializeSeq::end(self)
        }
    }

    impl SerializeTupleStruct for JsonSeqSerializer<'_> {
        type Ok = ();
        type Error = JsonCodecError;

        fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
            SerializeSeq::serialize_element(self, value)
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            SerializeSeq::end(self)
        }
    }

    struct JsonTupleVariantSerializer<'a> {
        ser: &'a mut JsonSerializer,
        first: bool,
    }

    impl SerializeTupleVariant for JsonTupleVariantSerializer<'_> {
        type Ok = ();
        type Error = JsonCodecError;

        fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
            if !self.first {
                self.ser.out.push(',');
            }
            self.first = false;
            value.serialize(&mut *self.ser)
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.ser.out.push_str("]}");
            Ok(())
        }
    }

    struct JsonMapSerializer<'a> {
        ser: &'a mut JsonSerializer,
        first: bool,
        close_double: bool,
    }

    impl SerializeMap for JsonMapSerializer<'_> {
        type Ok = ();
        type Error = JsonCodecError;

        fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
            if !self.first {
                self.ser.out.push(',');
            }
            self.first = false;
            key.serialize(&mut *self.ser)
        }

        fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
            self.ser.out.push(':');
            value.serialize(&mut *self.ser)
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.ser.out.push_str(if self.close_double { "}}" } else { "}" });
            Ok(())
        }
    }

    impl SerializeStruct for JsonMapSerializer<'_> {
        type Ok = ();
        type Error = JsonCodecError;

        fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error> {
            if !self.first {
                self.ser.out.push(',');
            }
            self.first = false;
            self.ser.push_escaped_str(key);
            self.ser.out.push(':');
            value.serialize(&mut *self.ser)
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.ser.out.push_str(if self.close_double { "}}" } else { "}" });
            Ok(())
        }
    }

    impl SerializeStructVariant for JsonMapSerializer<'_> {
        type Ok = ();
        type Error = JsonCodecError;

        fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error> {
            SerializeStruct::serialize_field(self, key, value)
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            SerializeStruct::end(self)
        }
    }

    impl<'a> ser::Serializer for &'a mut JsonSerializer {
        type Ok = ();
        type Error = JsonCodecError;
        type SerializeSeq = JsonSeqSerializer<'a>;
        type SerializeTuple = JsonSeqSerializer<'a>;
        type SerializeTupleStruct = JsonSeqSerializer<'a>;
        type SerializeTupleVariant = JsonTupleVariantSerializer<'a>;
        type SerializeMap = JsonMapSerializer<'a>;
        type SerializeStruct = JsonMapSerializer<'a>;
        type SerializeStructVariant = JsonMapSerializer<'a>;

        fn serialize_bool(self, v: bool) -> Result<(), Self::Error> {
            self.out.push_str(if v { "true" } else { "false" });
            Ok(())
        }

        fn serialize_i8(self, v: i8) -> Result<(), Self::Error> {
            self.serialize_i64(i64::from(v))
        }

        fn serialize_i16(self, v: i16) -> Result<(), Self::Error> {
            self.serialize_i64(i64::from(v))
        }

        fn serialize_i32(self, v: i32) -> Result<(), Self::Error> {
            self.serialize_i64(i64::from(v))
        }

        fn serialize_i64(self, v: i64) -> Result<(), Self::Error> {
            let _ = write!(self.out, "{v}");
            Ok(())
        }

        fn serialize_u8(self, v: u8) -> Result<(), Self::Error> {
            self.serialize_u64(u64::from(v))
        }

        fn serialize_u16(self, v: u16) -> Result<(), Self::Error> {
            self.serialize_u64(u64::from(v))
        }

        fn serialize_u32(self, v: u32) -> Result<(), Self::Error> {
            self.serialize_u64(u64::from(v))
        }

        fn serialize_u64(self, v: u64) -> Result<(), Self::Error> {
            let _ = write!(self.out, "{v}");
            Ok(())
        }

        fn serialize_f32(self, v: f32) -> Result<(), Self::Error> {
            self.serialize_f64(f64::from(v))
        }

        fn serialize_f64(self, v: f64) -> Result<(), Self::Error> {
            if !v.is_finite() {
                return Err(JsonCodecError::new(format!("cannot serialize non-finite float {v} to JSON")));
            }
            let _ = write!(self.out, "{v}");
            Ok(())
        }

        fn serialize_char(self, v: char) -> Result<(), Self::Error> {
            let mut buf = [0_u8; 4];
            self.serialize_str(v.encode_utf8(&mut buf))
        }

        fn serialize_str(self, v: &str) -> Result<(), Self::Error> {
            self.push_escaped_str(v);
            Ok(())
        }

        fn serialize_bytes(self, v: &[u8]) -> Result<(), Self::Error> {
            let mut seq = self.serialize_seq(Some(v.len()))?;
            for &b in v {
                SerializeSeq::serialize_element(&mut seq, &b)?;
            }
            SerializeSeq::end(seq)
        }

        fn serialize_none(self) -> Result<(), Self::Error> {
            self.out.push_str("null");
            Ok(())
        }

        fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
            value.serialize(self)
        }

        fn serialize_unit(self) -> Result<(), Self::Error> {
            self.out.push_str("null");
            Ok(())
        }

        fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Self::Error> {
            self.serialize_unit()
        }

        fn serialize_unit_variant(self, _name: &'static str, _idx: u32, variant: &'static str) -> Result<(), Self::Error> {
            self.serialize_str(variant)
        }

        fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _name: &'static str, value: &T) -> Result<(), Self::Error> {
            value.serialize(self)
        }

        fn serialize_newtype_variant<T: ?Sized + Serialize>(
            self,
            _name: &'static str,
            _idx: u32,
            variant: &'static str,
            value: &T,
        ) -> Result<(), Self::Error> {
            self.out.push('{');
            self.push_escaped_str(variant);
            self.out.push(':');
            value.serialize(&mut *self)?;
            self.out.push('}');
            Ok(())
        }

        fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
            self.out.push('[');
            Ok(JsonSeqSerializer { ser: self, first: true })
        }

        fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
            self.serialize_seq(Some(len))
        }

        fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<Self::SerializeTupleStruct, Self::Error> {
            self.serialize_seq(Some(len))
        }

        fn serialize_tuple_variant(
            self,
            _name: &'static str,
            _idx: u32,
            variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleVariant, Self::Error> {
            self.out.push('{');
            self.push_escaped_str(variant);
            self.out.push_str(":[");
            Ok(JsonTupleVariantSerializer { ser: self, first: true })
        }

        fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
            self.out.push('{');
            Ok(JsonMapSerializer { ser: self, first: true, close_double: false })
        }

        fn serialize_struct(self, _name: &'static str, len: usize) -> Result<Self::SerializeStruct, Self::Error> {
            self.serialize_map(Some(len))
        }

        fn serialize_struct_variant(
            self,
            _name: &'static str,
            _idx: u32,
            variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStructVariant, Self::Error> {
            self.out.push('{');
            self.push_escaped_str(variant);
            self.out.push_str(":{");
            Ok(JsonMapSerializer { ser: self, first: true, close_double: true })
        }
    }

    struct JsonParser<'de> {
        bytes: &'de [u8],
        pos: usize,
        depth: usize,
    }

    impl<'de> JsonParser<'de> {
        fn skip_ws(&mut self) {
            while let Some(&b) = self.bytes.get(self.pos) {
                if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }

        fn peek(&mut self) -> Result<u8, JsonCodecError> {
            self.skip_ws();
            self.bytes.get(self.pos).copied().ok_or_else(|| JsonCodecError::new("unexpected end of JSON"))
        }

        fn expect_byte(&mut self, expected: u8) -> Result<(), JsonCodecError> {
            let b = self.peek()?;
            if b != expected {
                return Err(JsonCodecError::new(format!("expected '{}' at byte {}, found '{}'", char::from(expected), self.pos, char::from(b))));
            }
            self.pos += 1;
            Ok(())
        }

        fn enter_nesting(&mut self) -> Result<(), JsonCodecError> {
            if self.depth >= MAX_JSON_DEPTH {
                return Err(JsonCodecError::new("maximum JSON nesting depth exceeded"));
            }
            self.depth += 1;
            Ok(())
        }

        fn exit_nesting(&mut self) {
            self.depth = self.depth.saturating_sub(1);
        }

        fn parse_bool(&mut self) -> Result<bool, JsonCodecError> {
            self.skip_ws();
            if self.bytes[self.pos..].starts_with(b"true") {
                self.pos += 4;
                Ok(true)
            } else if self.bytes[self.pos..].starts_with(b"false") {
                self.pos += 5;
                Ok(false)
            } else {
                Err(JsonCodecError::new(format!("expected boolean at byte {}", self.pos)))
            }
        }

        fn consume_null(&mut self) -> Result<(), JsonCodecError> {
            self.skip_ws();
            if self.bytes[self.pos..].starts_with(b"null") {
                self.pos += 4;
                Ok(())
            } else {
                Err(JsonCodecError::new(format!("expected null at byte {}", self.pos)))
            }
        }

        fn parse_number_slice(&mut self) -> Result<&'de str, JsonCodecError> {
            self.skip_ws();
            let start = self.pos;
            if self.bytes.get(self.pos) == Some(&b'-') {
                self.pos += 1;
            }
            let digits_start = self.pos;
            while let Some(&b) = self.bytes.get(self.pos) {
                if b.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == digits_start {
                return Err(JsonCodecError::new(format!("invalid JSON number at byte {start}")));
            }
            if self.bytes.get(self.pos) == Some(&b'.') {
                self.pos += 1;
                let frac_start = self.pos;
                while let Some(&b) = self.bytes.get(self.pos) {
                    if b.is_ascii_digit() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if self.pos == frac_start {
                    return Err(JsonCodecError::new(format!("invalid JSON fractional number at byte {start}")));
                }
            }
            if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
                self.pos += 1;
                if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                    self.pos += 1;
                }
                let exp_start = self.pos;
                while let Some(&b) = self.bytes.get(self.pos) {
                    if b.is_ascii_digit() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if self.pos == exp_start {
                    return Err(JsonCodecError::new(format!("invalid JSON exponent at byte {start}")));
                }
            }
            std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| JsonCodecError::new("non-UTF8 JSON number"))
        }

        fn parse_string(&mut self) -> Result<String, JsonCodecError> {
            self.expect_byte(b'"')?;
            let mut out = String::new();
            loop {
                let Some(&b) = self.bytes.get(self.pos) else {
                    return Err(JsonCodecError::new("unterminated JSON string"));
                };
                if b == b'"' {
                    self.pos += 1;
                    return Ok(out);
                }
                if b == b'\\' {
                    self.pos += 1;
                    let esc = self.bytes.get(self.pos).copied().ok_or_else(|| JsonCodecError::new("truncated JSON escape"))?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\x08'),
                        b'f' => out.push('\x0c'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            let ch = if (0xD800..=0xDBFF).contains(&code) {
                                if !self.bytes[self.pos..].starts_with(b"\\u") {
                                    return Err(JsonCodecError::new("unpaired high UTF-16 surrogate in JSON string"));
                                }
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(JsonCodecError::new("invalid low UTF-16 surrogate in JSON string"));
                                }
                                let combined = 0x10000 + (((u32::from(code) - 0xD800) << 10) | (u32::from(low) - 0xDC00));
                                char::from_u32(combined).ok_or_else(|| JsonCodecError::new("invalid UTF-16 surrogate scalar"))?
                            } else {
                                char::from_u32(u32::from(code)).ok_or_else(|| JsonCodecError::new("invalid Unicode scalar"))?
                            };
                            out.push(ch);
                        }
                        other => {
                            return Err(JsonCodecError::new(format!("unsupported JSON escape \\{}", char::from(other))));
                        }
                    }
                } else if b < 0x20 {
                    return Err(JsonCodecError::new("unescaped control character in JSON string"));
                } else if b.is_ascii() {
                    out.push(char::from(b));
                    self.pos += 1;
                } else {
                    let rest = std::str::from_utf8(&self.bytes[self.pos..]).map_err(|_| JsonCodecError::new("invalid UTF-8 in JSON"))?;
                    let ch = rest.chars().next().ok_or_else(|| JsonCodecError::new("unexpected EOF"))?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }

        fn parse_hex4(&mut self) -> Result<u16, JsonCodecError> {
            let slice = self.bytes.get(self.pos..self.pos + 4).ok_or_else(|| JsonCodecError::new("truncated \\uXXXX escape"))?;
            let hex_str = std::str::from_utf8(slice).map_err(|_| JsonCodecError::new("invalid UTF-8 in \\uXXXX escape"))?;
            let val = u16::from_str_radix(hex_str, 16).map_err(|_| JsonCodecError::new("invalid hex digit in \\uXXXX escape"))?;
            self.pos += 4;
            Ok(val)
        }

        fn skip_any(&mut self) -> Result<(), JsonCodecError> {
            match self.peek()? {
                b'"' => {
                    let _ = self.parse_string()?;
                    Ok(())
                }
                b'{' => {
                    self.enter_nesting()?;
                    self.pos += 1;
                    if self.peek()? != b'}' {
                        loop {
                            let _ = self.parse_string()?;
                            self.expect_byte(b':')?;
                            self.skip_any()?;
                            match self.peek()? {
                                b',' => self.pos += 1,
                                b'}' => break,
                                other => return Err(JsonCodecError::new(format!("unexpected '{}' in JSON object", char::from(other)))),
                            }
                        }
                    }
                    self.expect_byte(b'}')?;
                    self.exit_nesting();
                    Ok(())
                }
                b'[' => {
                    self.enter_nesting()?;
                    self.pos += 1;
                    if self.peek()? != b']' {
                        loop {
                            self.skip_any()?;
                            match self.peek()? {
                                b',' => self.pos += 1,
                                b']' => break,
                                other => return Err(JsonCodecError::new(format!("unexpected '{}' in JSON array", char::from(other)))),
                            }
                        }
                    }
                    self.expect_byte(b']')?;
                    self.exit_nesting();
                    Ok(())
                }
                b't' | b'f' => {
                    let _ = self.parse_bool()?;
                    Ok(())
                }
                b'n' => self.consume_null(),
                _ => {
                    let _ = self.parse_number_slice()?;
                    Ok(())
                }
            }
        }
    }

    struct JsonSeqAccess<'a, 'de> {
        parser: &'a mut JsonParser<'de>,
        first: bool,
    }

    impl<'de> SeqAccess<'de> for JsonSeqAccess<'_, 'de> {
        type Error = JsonCodecError;

        fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error> {
            if self.parser.peek()? == b']' {
                return Ok(None);
            }
            if !self.first {
                self.parser.expect_byte(b',')?;
            }
            self.first = false;
            seed.deserialize(&mut *self.parser).map(Some)
        }
    }

    struct JsonMapAccess<'a, 'de> {
        parser: &'a mut JsonParser<'de>,
        first: bool,
    }

    impl<'de> MapAccess<'de> for JsonMapAccess<'_, 'de> {
        type Error = JsonCodecError;

        fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
            if self.parser.peek()? == b'}' {
                return Ok(None);
            }
            if !self.first {
                self.parser.expect_byte(b',')?;
            }
            self.first = false;
            seed.deserialize(&mut *self.parser).map(Some)
        }

        fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
            self.parser.expect_byte(b':')?;
            seed.deserialize(&mut *self.parser)
        }
    }

    struct JsonEnumAccess<'a, 'de> {
        parser: &'a mut JsonParser<'de>,
        is_unit_string: bool,
    }

    impl<'a, 'de> EnumAccess<'de> for JsonEnumAccess<'a, 'de> {
        type Error = JsonCodecError;
        type Variant = JsonVariantAccess<'a, 'de>;

        fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error> {
            let val = seed.deserialize(&mut *self.parser)?;
            if !self.is_unit_string {
                self.parser.expect_byte(b':')?;
            }
            Ok((val, JsonVariantAccess { parser: self.parser, is_unit_string: self.is_unit_string }))
        }
    }

    struct JsonVariantAccess<'a, 'de> {
        parser: &'a mut JsonParser<'de>,
        is_unit_string: bool,
    }

    impl<'de> VariantAccess<'de> for JsonVariantAccess<'_, 'de> {
        type Error = JsonCodecError;

        fn unit_variant(self) -> Result<(), Self::Error> {
            if self.is_unit_string {
                Ok(())
            } else {
                self.parser.consume_null()?;
                self.parser.expect_byte(b'}')?;
                self.parser.exit_nesting();
                Ok(())
            }
        }

        fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Self::Error> {
            if self.is_unit_string {
                return Err(JsonCodecError::new("expected object-wrapped newtype variant"));
            }
            let v = seed.deserialize(&mut *self.parser)?;
            self.parser.expect_byte(b'}')?;
            self.parser.exit_nesting();
            Ok(v)
        }

        fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error> {
            if self.is_unit_string {
                return Err(JsonCodecError::new("expected object-wrapped tuple variant"));
            }
            let v = de::Deserializer::deserialize_seq(&mut *self.parser, visitor)?;
            self.parser.expect_byte(b'}')?;
            self.parser.exit_nesting();
            Ok(v)
        }

        fn struct_variant<V: Visitor<'de>>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error> {
            if self.is_unit_string {
                return Err(JsonCodecError::new("expected object-wrapped struct variant"));
            }
            let v = de::Deserializer::deserialize_map(&mut *self.parser, visitor)?;
            self.parser.expect_byte(b'}')?;
            self.parser.exit_nesting();
            Ok(v)
        }
    }

    impl<'de> de::Deserializer<'de> for &mut JsonParser<'de> {
        type Error = JsonCodecError;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            match self.peek()? {
                b'n' => {
                    self.consume_null()?;
                    visitor.visit_unit()
                }
                b't' | b'f' => visitor.visit_bool(self.parse_bool()?),
                b'"' => visitor.visit_string(self.parse_string()?),
                b'[' => self.deserialize_seq(visitor),
                b'{' => self.deserialize_map(visitor),
                _ => {
                    let s = self.parse_number_slice()?;
                    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                        if let Ok(u) = s.parse::<u64>() {
                            return visitor.visit_u64(u);
                        }
                        if let Ok(i) = s.parse::<i64>() {
                            return visitor.visit_i64(i);
                        }
                    }
                    let f = s.parse::<f64>().map_err(|_| JsonCodecError::new("invalid f64"))?;
                    if !f.is_finite() {
                        return Err(JsonCodecError::new("non-finite f64 in JSON"));
                    }
                    visitor.visit_f64(f)
                }
            }
        }

        fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            visitor.visit_bool(self.parse_bool()?)
        }

        fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<i8>().map_err(|_| JsonCodecError::new(format!("invalid i8 `{s}`")))?;
            visitor.visit_i8(v)
        }

        fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<i16>().map_err(|_| JsonCodecError::new(format!("invalid i16 `{s}`")))?;
            visitor.visit_i16(v)
        }

        fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<i32>().map_err(|_| JsonCodecError::new(format!("invalid i32 `{s}`")))?;
            visitor.visit_i32(v)
        }

        fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<i64>().map_err(|_| JsonCodecError::new(format!("invalid i64 `{s}`")))?;
            visitor.visit_i64(v)
        }

        fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<u8>().map_err(|_| JsonCodecError::new(format!("invalid u8 `{s}`")))?;
            visitor.visit_u8(v)
        }

        fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<u16>().map_err(|_| JsonCodecError::new(format!("invalid u16 `{s}`")))?;
            visitor.visit_u16(v)
        }

        fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<u32>().map_err(|_| JsonCodecError::new(format!("invalid u32 `{s}`")))?;
            visitor.visit_u32(v)
        }

        fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<u64>().map_err(|_| JsonCodecError::new(format!("invalid u64 `{s}`")))?;
            visitor.visit_u64(v)
        }

        fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<f32>().map_err(|_| JsonCodecError::new(format!("invalid f32 `{s}`")))?;
            if !v.is_finite() {
                return Err(JsonCodecError::new("non-finite f32 in JSON"));
            }
            visitor.visit_f32(v)
        }

        fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_number_slice()?;
            let v = s.parse::<f64>().map_err(|_| JsonCodecError::new(format!("invalid f64 `{s}`")))?;
            if !v.is_finite() {
                return Err(JsonCodecError::new("non-finite f64 in JSON"));
            }
            visitor.visit_f64(v)
        }

        fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let s = self.parse_string()?;
            let mut chars = s.chars();
            let ch = chars.next().ok_or_else(|| JsonCodecError::new("empty JSON char string"))?;
            if chars.next().is_some() {
                return Err(JsonCodecError::new("multi-char JSON string for char"));
            }
            visitor.visit_char(ch)
        }

        fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            visitor.visit_string(self.parse_string()?)
        }

        fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            visitor.visit_string(self.parse_string()?)
        }

        fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.deserialize_seq(visitor)
        }

        fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.deserialize_seq(visitor)
        }

        fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            if self.peek()? == b'n' {
                self.consume_null()?;
                visitor.visit_none()
            } else {
                visitor.visit_some(self)
            }
        }

        fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.consume_null()?;
            visitor.visit_unit()
        }

        fn deserialize_unit_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, Self::Error> {
            self.deserialize_unit(visitor)
        }

        fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, Self::Error> {
            visitor.visit_newtype_struct(self)
        }

        fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.enter_nesting()?;
            self.expect_byte(b'[')?;
            let val = visitor.visit_seq(JsonSeqAccess { parser: self, first: true })?;
            self.expect_byte(b']')?;
            self.exit_nesting();
            Ok(val)
        }

        fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error> {
            self.deserialize_seq(visitor)
        }

        fn deserialize_tuple_struct<V: Visitor<'de>>(self, _name: &'static str, _len: usize, visitor: V) -> Result<V::Value, Self::Error> {
            self.deserialize_seq(visitor)
        }

        fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.enter_nesting()?;
            self.expect_byte(b'{')?;
            let val = visitor.visit_map(JsonMapAccess { parser: self, first: true })?;
            self.expect_byte(b'}')?;
            self.exit_nesting();
            Ok(val)
        }

        fn deserialize_struct<V: Visitor<'de>>(
            self,
            _name: &'static str,
            _fields: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error> {
            self.deserialize_map(visitor)
        }

        fn deserialize_enum<V: Visitor<'de>>(
            self,
            _name: &'static str,
            _variants: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error> {
            match self.peek()? {
                b'"' => visitor.visit_enum(JsonEnumAccess { parser: self, is_unit_string: true }),
                b'{' => {
                    self.enter_nesting()?;
                    self.pos += 1;
                    visitor.visit_enum(JsonEnumAccess { parser: self, is_unit_string: false })
                }
                other => Err(JsonCodecError::new(format!("expected JSON enum string or object, found '{}'", char::from(other)))),
            }
        }

        fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.deserialize_string(visitor)
        }

        fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.skip_any()?;
            visitor.visit_unit()
        }
    }
}
