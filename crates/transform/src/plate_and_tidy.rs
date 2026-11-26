//! 96/384-well microplate mapping, grid-based layout builders, analyzable-sheet plate extraction,
//! legacy `.cbtemplate` JSON compatibility aliases, and tidy long-to-wide pivot transformations
//! (`QOL-2` / `R17.7`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use curvebook_doc::{
    Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, DataSetColumn, DataSheet, DocError, FloatingNote, GroupedTable,
    MAX_DATA_SETS, MAX_PLATE_WELLS, MAX_ROWS, MAX_SUBCOLUMNS, MultipleVariablesTable, PlateCellOrigin, RectPt, SubcolumnMode, TableArchetype,
    TableKind, XColumnMode, XyTable, validate_data_sets, validate_rows, validate_subcolumns, validate_title,
};
pub use curvebook_doc::{PlateGeometry, PlateWellRole};
use curvebook_math::sum::{self, SummaryMoments};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::archetype_convert::TargetArchetypeKind;
use crate::error::TransformError;
use crate::import::validate_cell_budget;

const MAX_CBTEMPLATE_JSON_BYTES: usize = 4 * 1024 * 1024;

/// Mapping between a physical microplate well label and its destination cell coordinate (`R17.7`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateWellCellEntry {
    /// Canonical microplate well identifier (for example `"A01"` or `"G12"`).
    pub well: String,
    /// Zero-based `(row, col, subcol)` coordinate written on the mapped [`DataSheet`].
    pub coord: CellCoord,
}

/// Complete output produced by [`map_plate_to_datasheet_with_origins`] (`R17.7`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlateMapOutput {
    /// Mapped output [`DataSheet`] with populated [`DataSheet::plate_well_origins`].
    pub sheet: DataSheet,
    /// Ordered well-to-cell coordinate map suitable for command payload reporting and QC lookups.
    pub well_cell_map: Vec<PlateWellCellEntry>,
    /// Non-blocking mapping warnings (such as replicate slot collision notices).
    pub warnings: Vec<String>,
}

/// Configuration for constructing a [`PlateMapTemplate`] from 2D label and optional *X* plate grids (`R17.7`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateGridBuildOptions {
    /// Human-readable template name (must be non-empty and within title length bounds).
    pub name: String,
    /// Independent *X* / concentration column title for mapped `Xy` or `Grouped` tables.
    pub x_title: String,
    /// Case-insensitive trimmed labels assigned to [`PlateWellRole::Blank`].
    #[serde(default)]
    pub blank_labels: Vec<String>,
    /// Case-insensitive trimmed labels assigned to [`PlateWellRole::PositiveControl`].
    #[serde(default)]
    pub positive_control_labels: Vec<String>,
    /// Intended output table archetype (used to warn when `x_grid` is omitted for `Xy` or `Grouped`).
    pub target_kind: TargetArchetypeKind,
}

impl Default for PlateGridBuildOptions {
    fn default() -> Self {
        Self {
            name: String::from("Plate Layout"),
            x_title: String::from("Concentration"),
            blank_labels: Vec::new(),
            positive_control_labels: Vec::new(),
            target_kind: TargetArchetypeKind::Column,
        }
    }
}

/// Strategy for matching source rows or columns to physical microplate wells (`R17.7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PlateWellMatchMode {
    /// Match exact canonical/unpadded well IDs (`"A01"`, `"A1"`) or leading well-ID tokens before delimiters (`"A01 - Sample X1"`).
    #[default]
    ByWellId,
    /// Map source rows or columns sequentially in row-major plate order (`0..geometry.well_count()`).
    Positional,
}

/// Scalar summary reducer collapsing each per-well kinetic or column series into one plate reading (`R17.7`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PlateKineticReducer {
    /// Take the active numeric reading at zero-based row `index`.
    RowIndex(usize),
    /// Arithmetic mean of active numeric readings whose *X* coordinate lies in `[x_min, x_max]` (inclusive; requires `Xy` numeric *X*).
    XWindowMean {
        /// Inclusive lower *X* bound (finite, `<= x_max`).
        x_min: f64,
        /// Inclusive upper *X* bound (finite, `>= x_min`).
        x_max: f64,
    },
    /// Minimum active numeric reading across the well column.
    Min,
    /// Maximum active numeric reading across the well column.
    Max,
}

/// Orientation and extraction rule for reading plate values from an analyzable [`DataSheet`] (`R17.7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlateSourceExtraction {
    /// Each row represents one plate well (for example `AreaUnderCurve`'s `analyzable_data` `MultipleVariables` sheet).
    WellPerRow {
        /// Column header (trimmed, case-insensitive) supplying the numeric value per well (for example `"Net Area"`).
        value_column: String,
    },
    /// Each dataset column represents one plate well (for example a kinetic `Xy` time course), collapsed via `reducer`.
    WellPerColumn {
        /// Reduction operation applied to each well column's active (non-excluded) observations.
        reducer: PlateKineticReducer,
    },
}

/// Row-major plate readings extracted from an analyzable [`DataSheet`] alongside non-blocking warnings (`R17.7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedPlateValues {
    /// Row-major (`0..geometry.well_count()`) extracted numeric values (`None` for missing or unavailable wells).
    pub values: Vec<Option<f64>>,
    /// Non-blocking diagnostics (missing wells, unavailable/excluded rows, or skipped non-well labels).
    pub warnings: Vec<String>,
}

/// Serializable 96/384-well plate layout template (`.cbtemplate` JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlateMapTemplate {
    /// Human-readable template name.
    pub name: String,
    /// Physical microplate dimensions (`96`-well `8 × 12` or `384`-well `16 × 24`).
    pub geometry: PlateGeometry,
    /// Independent *X*-column title assigned to mapped dose-response `XyTable` outputs.
    pub x_title: String,
    /// Row-major (`A01`, `A02`, ...) well role assignments (`len() == geometry.well_count()`).
    #[serde(deserialize_with = "deserialize_bounded_wells")]
    pub wells: Vec<PlateWellRole>,
}

fn deserialize_bounded_wells<'de, D>(deserializer: D) -> Result<Vec<PlateWellRole>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct BoundedWellsVisitor;
    impl<'de> Visitor<'de> for BoundedWellsVisitor {
        type Value = Vec<PlateWellRole>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a sequence of at most {MAX_PLATE_WELLS} plate well roles")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let cap = seq.size_hint().unwrap_or(0).min(MAX_PLATE_WELLS);
            let mut out = Vec::with_capacity(cap);
            while let Some(role) = seq.next_element::<PlateWellRole>()? {
                if out.len() >= MAX_PLATE_WELLS {
                    return Err(de::Error::custom(format!("plate template `wells` exceeds maximum length {MAX_PLATE_WELLS}")));
                }
                out.push(role);
            }
            Ok(out)
        }
    }
    deserializer.deserialize_seq(BoundedWellsVisitor)
}

impl PlateMapTemplate {
    /// Create a plate map template initialized with [`PlateWellRole::Excluded`] in every well.
    pub fn new(name: impl Into<String>, geometry: PlateGeometry, x_title: impl Into<String>) -> Result<Self, TransformError> {
        let name = name.into();
        let x_title = x_title.into();
        if name.trim().is_empty() {
            return Err(TransformError::InvalidSpec { message: String::from("plate template name cannot be empty") });
        }
        validate_title(&name)?;
        validate_title(&x_title)?;
        Ok(Self { name, geometry, x_title, wells: vec![PlateWellRole::Excluded; geometry.well_count()] })
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

    /// Validate template titles, well count, and all finite treatment parameters.
    pub fn validate(&self) -> Result<(), TransformError> {
        if self.name.trim().is_empty() {
            return Err(TransformError::InvalidSpec { message: String::from("plate template name cannot be empty") });
        }
        validate_title(&self.name)?;
        validate_title(&self.x_title)?;
        let expected = self.geometry.well_count();
        if self.wells.len() != expected {
            return Err(TransformError::InvalidSpec { message: format!("plate template has {} well(s), expected {expected}", self.wells.len()) });
        }
        for (idx, role) in self.wells.iter().enumerate() {
            validate_well_role(role, self.geometry, idx)?;
        }
        Ok(())
    }

    /// Serialize this template as legacy `.cbtemplate` UTF-8 JSON.
    #[deprecated(note = "use versioned `.cbplate` persistence (`curvebook_format::PlateMapFile`) instead")]
    pub fn to_cbtemplate_json(&self) -> Result<String, TransformError> {
        for role in &self.wells {
            if let PlateWellRole::Treatment { dilution_x, .. } = role
                && !dilution_x.is_finite()
            {
                return Err(TransformError::InvalidInput {
                    message: format!("failed to serialize plate template JSON: cannot serialize non-finite float {dilution_x} to JSON"),
                });
            }
        }
        serde_json::to_string(self).map_err(|err| TransformError::InvalidInput { message: format!("failed to serialize plate template JSON: {err}") })
    }

    /// Parse a legacy `.cbtemplate` JSON string back into a validated [`PlateMapTemplate`].
    #[deprecated(note = "use versioned `.cbplate` persistence (`curvebook_format::PlateMapFile`) instead")]
    pub fn from_cbtemplate_json(json: &str) -> Result<Self, TransformError> {
        if json.len() > MAX_CBTEMPLATE_JSON_BYTES {
            return Err(TransformError::InvalidInput {
                message: format!("plate template JSON length ({} bytes) exceeds maximum {MAX_CBTEMPLATE_JSON_BYTES} bytes", json.len()),
            });
        }
        let parsed: Self = serde_json::from_str(json)
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
        validate_title(group_name)?;
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
    well_label: String,
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

/// Map row-major 96/384-well plate readings and a [`PlateMapTemplate`] into a [`DataSheet`]
/// together with populated [`DataSheet::plate_well_origins`], an ordered [`PlateWellCellEntry`]
/// map, and non-blocking mapping warnings (`R17.7`).
///
/// Supports optional blank-mean subtraction (`subtract_blank_mean = true`), excludes
/// [`PlateWellRole::Excluded`] wells from treatment and blank calculations while preserving them
/// non-destructively (as excluded cells + [`CellExclusionNote`]s in `Column`/`MultipleVariables`,
/// or QC [`FloatingNote`] annotations in `Xy`/`Grouped`), and projects onto `target_kind`.
pub fn map_plate_to_datasheet_with_origins(
    title: &str,
    geometry: PlateGeometry,
    values: &[Option<f64>],
    template: &PlateMapTemplate,
    target_kind: TargetArchetypeKind,
    subtract_blank_mean: bool,
) -> Result<PlateMapOutput, TransformError> {
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

/// Map row-major 96/384-well plate readings and a [`PlateMapTemplate`] into a [`DataSheet`].
///
/// Delegates to [`map_plate_to_datasheet_with_origins`] and returns the mapped [`DataSheet`]
/// with populated [`DataSheet::plate_well_origins`].
pub fn map_plate_to_datasheet(
    title: &str,
    geometry: PlateGeometry,
    values: &[Option<f64>],
    template: &PlateMapTemplate,
    target_kind: TargetArchetypeKind,
    subtract_blank_mean: bool,
) -> Result<DataSheet, TransformError> {
    map_plate_to_datasheet_with_origins(title, geometry, values, template, target_kind, subtract_blank_mean).map(|out| out.sheet)
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
                    well_label: label,
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
    /// `(row_idx, group_idx, sub_idx)` -> `(numeric_value, well_label)`
    cells: BTreeMap<(usize, usize, usize), (f64, String)>,
    collisions: usize,
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

    let mut cells: BTreeMap<(usize, usize, usize), (f64, String)> = BTreeMap::new();
    let mut max_sub = 0_usize;
    let mut collisions = 0_usize;
    for pt in sorted_pts {
        let Some(&g) = group_map.get(&pt.group_name) else { continue };
        let norm_x = normalize_zero(pt.dilution_x);
        let Some(&r) = x_row_by_bits.get(&norm_x.to_bits()) else { continue };
        let Some(&base_slot) = rep_slot_map.get(&pt.replicate) else { continue };
        let Some(val) = pt.value else { continue };

        let mut slot = base_slot;
        while cells.contains_key(&(r, g, slot)) {
            if slot == base_slot {
                collisions += 1;
            }
            slot += 1;
        }
        cells.insert((r, g, slot), (val, pt.well_label.clone()));
        if slot > max_sub {
            max_sub = slot;
        }
    }

    let subcol_count = (max_sub + 1).max(rep_slot_map.len()).max(1);
    validate_data_sets(groups.len())?;
    validate_rows(x_values.len())?;
    validate_subcolumns(subcol_count)?;
    validate_cell_budget(x_values.len(), groups.len(), subcol_count, 1, "plate treatment grid")?;

    Ok(TreatmentGrid { groups, x_values, subcol_count, cells, collisions })
}

fn finalize_plate_map_output(
    mut sheet: DataSheet,
    mut origins: Vec<PlateCellOrigin>,
    warnings: Vec<String>,
) -> Result<PlateMapOutput, TransformError> {
    origins.sort_by_key(|o| o.coord);
    origins.dedup_by_key(|o| o.coord);
    let well_cell_map: Vec<PlateWellCellEntry> = origins.iter().map(|o| PlateWellCellEntry { well: o.well.clone(), coord: o.coord }).collect();
    sheet.plate_well_origins = origins;
    sheet.validate()?;
    Ok(PlateMapOutput { sheet, well_cell_map, warnings })
}

fn build_plate_xy_sheet(title: &str, template: &PlateMapTemplate, collected: &CollectedPlate) -> Result<PlateMapOutput, TransformError> {
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
    let mut origins = Vec::with_capacity(grid.cells.len());
    for (&(r, c, s), (val, well)) in &grid.cells {
        xy.data_sets[c].cells[r][s] = Cell::number(*val);
        origins.push(PlateCellOrigin { coord: CellCoord::new(r, c, s), well: well.clone() });
    }

    let mut sheet = DataSheet::new(make_sheet_id("plate", title), title, TableArchetype::Xy(xy))?;
    append_plate_floating_notes(&mut sheet, collected, grid.collisions);
    let mut warnings = Vec::new();
    if grid.collisions > 0 {
        warnings.push(format!("{} replicate slot collision(s) shifted to adjacent subcolumns", grid.collisions));
    }
    finalize_plate_map_output(sheet, origins, warnings)
}

fn build_plate_grouped_sheet(title: &str, template: &PlateMapTemplate, collected: &CollectedPlate) -> Result<PlateMapOutput, TransformError> {
    let grid = build_treatment_grid(&collected.treatments)?;
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(grid.subcol_count), grid.groups.len(), grid.x_values.len())?;
    grouped.row_factor_title = non_empty_or(&template.x_title, "Dilution");
    grouped.column_factor_title = String::from("Group");
    grouped.row_titles = grid.x_values.iter().map(|x| format_compact_f64(*x)).collect();
    for (c, name) in grid.groups.iter().enumerate() {
        grouped.data_sets[c].title = name.clone();
    }
    let mut origins = Vec::with_capacity(grid.cells.len());
    for (&(r, c, s), (val, well)) in &grid.cells {
        grouped.data_sets[c].cells[r][s] = Cell::number(*val);
        origins.push(PlateCellOrigin { coord: CellCoord::new(r, c, s), well: well.clone() });
    }

    let mut sheet = DataSheet::new(make_sheet_id("plate", title), title, TableArchetype::Grouped(grouped))?;
    append_plate_floating_notes(&mut sheet, collected, grid.collisions);
    let mut warnings = Vec::new();
    if grid.collisions > 0 {
        warnings.push(format!("{} replicate slot collision(s) shifted to adjacent subcolumns", grid.collisions));
    }
    finalize_plate_map_output(sheet, origins, warnings)
}

fn build_plate_column_sheet(
    title: &str,
    _template: &PlateMapTemplate,
    collected: &CollectedPlate,
    subtract_blank_mean: bool,
) -> Result<PlateMapOutput, TransformError> {
    let mut col_names: Vec<String> = Vec::new();
    let mut col_map: BTreeMap<String, usize> = BTreeMap::new();
    let mut col_cells: Vec<Vec<Cell>> = Vec::new();
    let mut origins: Vec<PlateCellOrigin> = Vec::new();

    let mut sorted_pts: Vec<&TreatmentPoint> = collected.treatments.iter().collect();
    sorted_pts
        .sort_by(|a, b| a.dilution_x.total_cmp(&b.dilution_x).then_with(|| a.replicate.cmp(&b.replicate)).then_with(|| a.well_idx.cmp(&b.well_idx)));

    for pt in sorted_pts {
        let Some(val) = pt.value else { continue };
        validate_title(&pt.group_name)?;
        let c = ensure_column_bucket(&pt.group_name, &mut col_names, &mut col_map, &mut col_cells);
        let r = col_cells[c].len();
        col_cells[c].push(Cell::number(val));
        origins.push(PlateCellOrigin { coord: CellCoord::new(r, c, 0), well: pt.well_label.clone() });
    }

    append_control_column("Positive Control", &collected.positive_controls, &mut col_names, &mut col_map, &mut col_cells, &mut origins)?;
    if !subtract_blank_mean {
        append_control_column("Blank", &collected.blanks, &mut col_names, &mut col_map, &mut col_cells, &mut origins)?;
    }

    let mut notes = Vec::new();
    let excluded_with_values: Vec<&ExcludedWellEntry> = collected.excluded.iter().filter(|e| e.raw_value.is_some()).collect();
    if !excluded_with_values.is_empty() {
        let c = allocate_distinct_bucket("Excluded", &mut col_names, &mut col_map, &mut col_cells)?;
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
                origins.push(PlateCellOrigin { coord: CellCoord::new(r, c, 0), well: entry.label.clone() });
            }
        }
    }

    if col_names.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no plate observations available to populate Column sheet") });
    }

    let rows = col_cells.iter().map(Vec::len).max().unwrap_or(0).max(1);
    validate_data_sets(col_names.len())?;
    validate_rows(rows)?;
    validate_cell_budget(rows, col_names.len(), 1, 0, "plate Column sheet")?;
    let mut table = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, col_names.len(), rows)?;
    for (c, name) in col_names.into_iter().enumerate() {
        table.data_sets[c].title = name;
        for (r, cell) in col_cells[c].iter().enumerate() {
            if let Some(slot) = table.data_sets[c].cells.get_mut(r).and_then(|row| row.get_mut(0)) {
                *slot = cell.clone();
            }
        }
    }

    let mut sheet = DataSheet::new(make_sheet_id("plate", title), title, TableArchetype::Column(table))?;
    sheet.exclusion_notes = notes;
    sheet.exclusion_notes.sort_by_key(|n| n.coord);
    append_plate_floating_notes(&mut sheet, collected, 0);
    finalize_plate_map_output(sheet, origins, Vec::new())
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

fn allocate_distinct_bucket(
    preferred: &str,
    col_names: &mut Vec<String>,
    col_map: &mut BTreeMap<String, usize>,
    col_cells: &mut Vec<Vec<Cell>>,
) -> Result<usize, TransformError> {
    let title = if !col_map.contains_key(preferred) {
        preferred.to_owned()
    } else {
        let plate_prefixed = format!("Plate {preferred}");
        if !col_map.contains_key(&plate_prefixed) {
            plate_prefixed
        } else {
            let mut n = 2_usize;
            loop {
                let candidate = format!("{plate_prefixed} ({n})");
                if !col_map.contains_key(&candidate) {
                    break candidate;
                }
                n += 1;
            }
        }
    };
    validate_title(&title)?;
    let idx = col_names.len();
    col_names.push(title.clone());
    col_map.insert(title, idx);
    col_cells.push(Vec::new());
    Ok(idx)
}

fn append_control_column(
    name: &str,
    entries: &[ControlWellEntry],
    col_names: &mut Vec<String>,
    col_map: &mut BTreeMap<String, usize>,
    col_cells: &mut Vec<Vec<Cell>>,
    origins: &mut Vec<PlateCellOrigin>,
) -> Result<(), TransformError> {
    let populated: Vec<(&str, f64)> = entries.iter().filter_map(|e| e.value.map(|v| (e.label.as_str(), v))).collect();
    if populated.is_empty() {
        return Ok(());
    }
    let c = allocate_distinct_bucket(name, col_names, col_map, col_cells)?;
    for (well, v) in populated {
        let r = col_cells[c].len();
        col_cells[c].push(Cell::number(v));
        origins.push(PlateCellOrigin { coord: CellCoord::new(r, c, 0), well: well.to_owned() });
    }
    Ok(())
}

fn build_plate_mv_sheet(
    title: &str,
    geometry: PlateGeometry,
    values: &[Option<f64>],
    template: &PlateMapTemplate,
    collected: &CollectedPlate,
) -> Result<PlateMapOutput, TransformError> {
    let rows = geometry.well_count();
    let cols = geometry.cols();
    let blank_offset = collected.blank_mean_applied.map_or(0.0, |(m, _)| m);
    let mut mv = MultipleVariablesTable::new(8, rows)?;
    let titles = ["Well", "PlateRow", "PlateCol", "Role", "Group", &non_empty_or(&template.x_title, "DilutionX"), "Replicate", "Value"];
    for (idx, var_title) in titles.iter().enumerate() {
        mv.variables[idx].title = (*var_title).to_owned();
    }

    let mut notes = Vec::new();
    let mut origins = Vec::new();
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
                    origins.push(PlateCellOrigin { coord: CellCoord::new(r, 7, 0), well: label });
                }
            }
            PlateWellRole::Blank => {
                mv.variables[3].cells[r] = Cell::text("Blank");
                mv.variables[4].cells[r] = Cell::text("Blank");
                if let Some(val) = maybe_val {
                    mv.variables[7].cells[r] = Cell::number(*val - blank_offset);
                    origins.push(PlateCellOrigin { coord: CellCoord::new(r, 7, 0), well: label });
                }
            }
            PlateWellRole::PositiveControl => {
                mv.variables[3].cells[r] = Cell::text("PositiveControl");
                mv.variables[4].cells[r] = Cell::text("PositiveControl");
                if let Some(val) = maybe_val {
                    mv.variables[7].cells[r] = Cell::number(*val - blank_offset);
                    origins.push(PlateCellOrigin { coord: CellCoord::new(r, 7, 0), well: label });
                }
            }
            PlateWellRole::Excluded => {
                mv.variables[3].cells[r] = Cell::text("Excluded");
                mv.variables[4].cells[r] = Cell::text("Excluded");
                mv.variables[7].cells[r] = match maybe_val {
                    Some(v) => {
                        origins.push(PlateCellOrigin { coord: CellCoord::new(r, 7, 0), well: label.clone() });
                        Cell::excluded_number(*v)
                    }
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
    append_plate_floating_notes(&mut sheet, collected, 0);
    finalize_plate_map_output(sheet, origins, Vec::new())
}

/// Build a [`PlateMapTemplate`] from a strictly rectangular `8x12` or `16x24` label grid and an
/// optional matching *X* coordinate grid (`R17.7`).
///
/// Labels are kept strictly literal (zero inline concentration parsing). Blank and positive-control
/// matching is trimmed and case-insensitive; empty cells become [`PlateWellRole::Excluded`].
/// Replicate indices are assigned zero-based (`0, 1, ...`) per `(group_name, dilution_x)` in
/// row-major well order.
pub fn build_plate_template_from_grids(
    label_grid: &[Vec<String>],
    x_grid: Option<&[Vec<Option<f64>>]>,
    options: &PlateGridBuildOptions,
) -> Result<(PlateMapTemplate, Vec<String>), TransformError> {
    if options.name.trim().is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("plate template name cannot be empty") });
    }
    validate_title(&options.name)?;
    validate_title(&options.x_title)?;

    let geometry = detect_strict_plate_grid_geometry(label_grid.len(), label_grid.iter().map(Vec::len), "label_grid")?;
    if let Some(xg) = x_grid {
        let x_geom = detect_strict_plate_grid_geometry(xg.len(), xg.iter().map(Vec::len), "x_grid")?;
        if x_geom != geometry {
            return Err(TransformError::InvalidSpec {
                message: format!(
                    "x_grid geometry ({}x{}) does not match label_grid geometry ({}x{})",
                    x_geom.rows(),
                    x_geom.cols(),
                    geometry.rows(),
                    geometry.cols()
                ),
            });
        }
    }

    let blank_set: BTreeSet<String> = options.blank_labels.iter().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
    let pos_set: BTreeSet<String> = options.positive_control_labels.iter().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
    if let Some(conflict) = blank_set.intersection(&pos_set).next() {
        return Err(TransformError::InvalidSpec {
            message: format!("label `{conflict}` cannot appear in both blank_labels and positive_control_labels"),
        });
    }

    let mut wells = Vec::with_capacity(geometry.well_count());
    let mut rep_counts: BTreeMap<(String, u64), usize> = BTreeMap::new();

    for (r, row) in label_grid.iter().enumerate() {
        for (c, raw_label) in row.iter().enumerate() {
            let trimmed = raw_label.trim();
            if trimmed.is_empty() {
                wells.push(PlateWellRole::Excluded);
                continue;
            }
            let lower = trimmed.to_lowercase();
            if blank_set.contains(&lower) {
                wells.push(PlateWellRole::Blank);
                continue;
            }
            if pos_set.contains(&lower) {
                wells.push(PlateWellRole::PositiveControl);
                continue;
            }

            validate_title(trimmed)?;
            let well_id = geometry.well_label(r, c)?;
            let dilution_x = match x_grid {
                None => 0.0,
                Some(xg) => match xg.get(r).and_then(|xr| xr.get(c)).copied().flatten() {
                    None => {
                        return Err(TransformError::InvalidInput {
                            message: format!("missing x_grid value at well {well_id} for label `{trimmed}`"),
                        });
                    }
                    Some(x) if !x.is_finite() => {
                        return Err(TransformError::InvalidInput {
                            message: format!("non-finite x_grid value ({x}) at well {well_id} for label `{trimmed}`"),
                        });
                    }
                    Some(x) => normalize_zero(x),
                },
            };
            let key = (trimmed.to_owned(), dilution_x.to_bits());
            let counter = rep_counts.entry(key).or_insert(0);
            let replicate = *counter;
            *counter += 1;

            wells.push(PlateWellRole::Treatment { group_name: trimmed.to_owned(), dilution_x, replicate });
        }
    }

    let mut warnings = Vec::new();
    if x_grid.is_none() && matches!(options.target_kind, TargetArchetypeKind::Xy | TargetArchetypeKind::Grouped) {
        warnings.push(String::from("x_grid omitted: all Treatment wells default to dilution_x = 0.0 (single X row on XY/Grouped output)"));
    }

    let template = PlateMapTemplate::from_wells(options.name.clone(), geometry, options.x_title.clone(), wells)?;
    Ok((template, warnings))
}

/// Extract a strictly `8x12` or `16x24` text grid from a [`DataSheet`]'s data cells (excluding
/// column titles and row titles) (`R17.7`).
///
/// Rejects `9x13` and `17x25` tables with an explicit header-exclusion hint.
pub fn extract_plate_grid_from_datasheet(sheet: &DataSheet) -> Result<(PlateGeometry, Vec<Vec<String>>), TransformError> {
    let raw_cells = extract_raw_cell_grid_from_datasheet(sheet)?;
    let geometry = detect_strict_plate_grid_geometry(raw_cells.len(), raw_cells.iter().map(Vec::len), "plate layout sheet")?;
    let grid = raw_cells.into_iter().map(|row| row.into_iter().map(format_plate_grid_cell_text).collect()).collect();
    Ok((geometry, grid))
}

/// Two-dimensional microplate optional finite *X* coordinate grid (`8x12` or `16x24`) (`R17.7`).
pub type PlateXGrid = Vec<Vec<Option<f64>>>;

type LabeledPlateEntry = (Option<String>, Option<f64>);

/// Extract a strictly `8x12` or `16x24` optional finite *X* grid from a [`DataSheet`]'s data cells
/// (excluding column titles and row titles) (`R17.7`).
pub fn extract_plate_x_grid_from_datasheet(sheet: &DataSheet) -> Result<(PlateGeometry, PlateXGrid), TransformError> {
    let raw_cells = extract_raw_cell_grid_from_datasheet(sheet)?;
    let geometry = detect_strict_plate_grid_geometry(raw_cells.len(), raw_cells.iter().map(Vec::len), "plate X-grid sheet")?;
    let grid = raw_cells.into_iter().map(|row| row.into_iter().map(parse_plate_grid_cell_x).collect()).collect();
    Ok((geometry, grid))
}

fn extract_raw_cell_grid_from_datasheet(sheet: &DataSheet) -> Result<Vec<Vec<Cell>>, TransformError> {
    let rows = sheet.archetype.row_count();
    match &sheet.archetype {
        TableArchetype::Column(t) => {
            if t.subcolumn_mode != SubcolumnMode::Single {
                return Err(TransformError::InvalidSpec {
                    message: format!("plate grid sheet `{}` must use Single subcolumn mode, found {:?}", sheet.title, t.subcolumn_mode),
                });
            }
            Ok(extract_dataset_primary_grid(&t.data_sets, rows))
        }
        TableArchetype::Grouped(t) => {
            if t.subcolumn_mode.subcolumn_count() != 1 {
                return Err(TransformError::InvalidSpec {
                    message: format!("plate grid sheet `{}` must have 1 subcolumn per column, found {:?}", sheet.title, t.subcolumn_mode),
                });
            }
            Ok(extract_dataset_primary_grid(&t.data_sets, rows))
        }
        TableArchetype::Contingency(t) => Ok(extract_dataset_primary_grid(&t.data_sets, rows)),
        TableArchetype::PartsOfWhole(t) => Ok(extract_dataset_primary_grid(&t.data_sets, rows)),
        TableArchetype::MultipleVariables(mv) => {
            let cols = mv.variables.len();
            let mut grid = vec![vec![Cell::empty(); cols]; rows];
            for (c, var) in mv.variables.iter().enumerate() {
                for (r, cell) in var.cells.iter().enumerate().take(rows) {
                    grid[r][c] = cell.clone();
                }
            }
            Ok(grid)
        }
        TableArchetype::Xy(xy) => {
            if xy.x_mode.subcolumn_count() != 1 || xy.y_subcolumn_mode.subcolumn_count() != 1 {
                return Err(TransformError::InvalidSpec {
                    message: format!("XY plate grid sheet `{}` must have single-subcolumn X and Y columns", sheet.title),
                });
            }
            let cols = 1 + xy.data_sets.len();
            let mut grid = vec![vec![Cell::empty(); cols]; rows];
            for (r, row_vec) in grid.iter_mut().enumerate().take(rows) {
                row_vec[0] = xy.materialized_x_cell_at(r);
                for (c, ds) in xy.data_sets.iter().enumerate() {
                    if let Some(cell) = ds.cells.get(r).and_then(|rc| rc.first()) {
                        row_vec[c + 1] = cell.clone();
                    }
                }
            }
            Ok(grid)
        }
        TableArchetype::Survival(surv) => {
            let cols = 1 + surv.cohorts.len();
            let mut grid = vec![vec![Cell::empty(); cols]; rows];
            for (r, row_vec) in grid.iter_mut().enumerate().take(rows) {
                if let Some(t_cell) = surv.times.get(r) {
                    row_vec[0] = t_cell.clone();
                }
                for (c, ds) in surv.cohorts.iter().enumerate() {
                    if let Some(cell) = ds.cells.get(r).and_then(|rc| rc.first()) {
                        row_vec[c + 1] = cell.clone();
                    }
                }
            }
            Ok(grid)
        }
        TableArchetype::Nested(_) => {
            Err(TransformError::IncompatibleTable { operation: "extract_plate_grid_from_datasheet", kind: TableKind::Nested })
        }
    }
}

fn extract_dataset_primary_grid(data_sets: &[DataSetColumn], rows: usize) -> Vec<Vec<Cell>> {
    let cols = data_sets.len();
    let mut grid = vec![vec![Cell::empty(); cols]; rows];
    for (c, ds) in data_sets.iter().enumerate() {
        for (r, row_cells) in ds.cells.iter().enumerate().take(rows) {
            if let Some(cell) = row_cells.first() {
                grid[r][c] = cell.clone();
            }
        }
    }
    grid
}

fn format_plate_grid_cell_text(cell: Cell) -> String {
    if cell.excluded {
        return String::new();
    }
    match cell.value {
        CellValue::Empty => String::new(),
        CellValue::Text(s) => s,
        CellValue::Number(v) if v.is_finite() => format!("{}", normalize_zero(v)),
        CellValue::Number(_) => String::new(),
    }
}

fn parse_plate_grid_cell_x(cell: Cell) -> Option<f64> {
    if cell.excluded {
        return None;
    }
    match cell.value {
        CellValue::Empty => None,
        CellValue::Number(v) if v.is_finite() => Some(normalize_zero(v)),
        CellValue::Number(_) => None,
        CellValue::Text(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return None;
            }
            trimmed.parse::<f64>().ok().filter(|v| v.is_finite()).map(normalize_zero)
        }
    }
}

fn detect_strict_plate_grid_geometry(
    rows: usize,
    col_lens: impl IntoIterator<Item = usize>,
    grid_name: &str,
) -> Result<PlateGeometry, TransformError> {
    let widths: Vec<usize> = col_lens.into_iter().collect();
    if rows == 0 || widths.is_empty() {
        return Err(TransformError::InvalidSpec {
            message: format!("{grid_name} is empty; expected strictly 8x12 (96-well) or 16x24 (384-well) plate grid"),
        });
    }
    let first_width = widths[0];
    for (r, &w) in widths.iter().enumerate() {
        if w != first_width {
            return Err(TransformError::InvalidSpec {
                message: format!("{grid_name} row {r} has {w} column(s), expected uniform row width {first_width}"),
            });
        }
    }
    match (rows, first_width) {
        (8, 12) => Ok(PlateGeometry::Wells96),
        (16, 24) => Ok(PlateGeometry::Wells384),
        (9, 13) => Err(TransformError::InvalidSpec {
            message: format!(
                "{grid_name} has dimensions 9x13, expected 8x12 (96-well) or 16x24 (384-well); exclude plate row/column headers (A-H and 1-12) before mapping"
            ),
        }),
        (17, 25) => Err(TransformError::InvalidSpec {
            message: format!(
                "{grid_name} has dimensions 17x25, expected 8x12 (96-well) or 16x24 (384-well); exclude plate row/column headers (A-P and 1-24) before mapping"
            ),
        }),
        (7, 12) | (15, 24) => Err(TransformError::InvalidSpec {
            message: format!(
                "{grid_name} has dimensions {rows}x{first_width}, expected strictly 8x12 (96-well) or 16x24 (384-well); if the first plate row was imported as column headers, re-import with has_header=false"
            ),
        }),
        _ => Err(TransformError::InvalidSpec {
            message: format!("{grid_name} has dimensions {rows}x{first_width}, expected strictly 8x12 (96-well) or 16x24 (384-well)"),
        }),
    }
}

/// Extract row-major plate readings (`Vec<Option<f64>>` of length `geometry.well_count()`) from
/// an analyzable [`DataSheet`] (such as an `AreaUnderCurve` `analyzable_data` table or a kinetic
/// `Xy` / `Column` sheet) (`R17.7`).
pub fn extract_plate_values_from_analyzable_sheet(
    sheet: &DataSheet,
    geometry: PlateGeometry,
    extraction: &PlateSourceExtraction,
    match_mode: PlateWellMatchMode,
) -> Result<ExtractedPlateValues, TransformError> {
    match extraction {
        PlateSourceExtraction::WellPerRow { value_column } => extract_plate_values_well_per_row(sheet, geometry, value_column, match_mode),
        PlateSourceExtraction::WellPerColumn { reducer } => extract_plate_values_well_per_column(sheet, geometry, *reducer, match_mode),
    }
}

fn parse_leading_well_id(geometry: PlateGeometry, raw: &str) -> Option<usize> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(idx) = geometry.parse_well_id(trimmed) {
        return Some(idx);
    }
    let end = trimmed.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(trimmed.len());
    if end == 0 || end == trimmed.len() {
        return None;
    }
    let prefix = trimmed.get(..end)?;
    geometry.parse_well_id(prefix)
}

fn find_header_index_case_insensitive<'a>(titles: impl Iterator<Item = &'a str>, target: &str) -> Option<usize> {
    let needle = target.trim();
    let collected: Vec<&str> = titles.collect();
    collected.iter().position(|t| t.trim() == needle).or_else(|| collected.iter().position(|t| t.trim().eq_ignore_ascii_case(needle)))
}

fn dataset_row_active_value(ds: &DataSetColumn, row: usize, mode: SubcolumnMode) -> Option<f64> {
    let row_cells = ds.cells.get(row)?;
    if matches!(mode, SubcolumnMode::RawReplicates(n) if n >= 2) {
        let active: Vec<f64> = row_cells.iter().filter_map(Cell::active_number).collect();
        sum::mean(&active).ok().filter(|v| v.is_finite())
    } else {
        row_cells.first().and_then(Cell::active_number)
    }
}

fn extract_plate_values_well_per_row(
    sheet: &DataSheet,
    geometry: PlateGeometry,
    value_column: &str,
    match_mode: PlateWellMatchMode,
) -> Result<ExtractedPlateValues, TransformError> {
    let col_needle = value_column.trim();
    if col_needle.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("value_column cannot be empty for WellPerRow extraction") });
    }

    let entries: Vec<LabeledPlateEntry> = match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let Some(val_col) = find_header_index_case_insensitive(mv.variables.iter().map(|v| v.title.as_str()), col_needle) else {
                return Err(TransformError::InvalidSpec { message: format!("value_column `{col_needle}` not found in sheet `{}`", sheet.title) });
            };
            let id_col = find_header_index_case_insensitive(mv.variables.iter().map(|v| v.title.as_str()), "Series")
                .or_else(|| find_header_index_case_insensitive(mv.variables.iter().map(|v| v.title.as_str()), "Well"))
                .or_else(|| (!mv.variables.is_empty() && val_col != 0).then_some(0));

            let rows = mv.row_count();
            let mut out = Vec::with_capacity(rows);
            for r in 0..rows {
                let label = id_col
                    .and_then(|c| mv.variables.get(c))
                    .and_then(|v| v.cells.get(r))
                    .and_then(Cell::active_text)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or_else(|| mv.row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_owned));
                let val = mv.variables[val_col].cells.get(r).and_then(Cell::active_number);
                out.push((label, val));
            }
            out
        }
        TableArchetype::Column(t) => collect_dataset_well_per_row(&t.data_sets, &t.row_titles, t.subcolumn_mode, col_needle, &sheet.title)?,
        TableArchetype::Xy(t) => collect_dataset_well_per_row(&t.data_sets, &t.row_titles, t.y_subcolumn_mode, col_needle, &sheet.title)?,
        TableArchetype::Grouped(t) => collect_dataset_well_per_row(&t.data_sets, &t.row_titles, t.subcolumn_mode, col_needle, &sheet.title)?,
        _ => {
            return Err(TransformError::IncompatibleTable { operation: "extract_plate_values_from_analyzable_sheet", kind: sheet.kind() });
        }
    };

    assemble_extracted_plate_values(geometry, &entries, match_mode, "row", &sheet.title)
}

fn collect_dataset_well_per_row(
    data_sets: &[DataSetColumn],
    row_titles: &[String],
    mode: SubcolumnMode,
    col_needle: &str,
    sheet_title: &str,
) -> Result<Vec<LabeledPlateEntry>, TransformError> {
    let Some(val_col) = find_header_index_case_insensitive(data_sets.iter().map(|d| d.title.as_str()), col_needle) else {
        return Err(TransformError::InvalidSpec { message: format!("value_column `{col_needle}` not found in sheet `{sheet_title}`") });
    };
    let rows = data_sets[val_col].row_count().max(row_titles.len());
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let label = row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_owned);
        let val = dataset_row_active_value(&data_sets[val_col], r, mode);
        out.push((label, val));
    }
    Ok(out)
}

fn extract_plate_values_well_per_column(
    sheet: &DataSheet,
    geometry: PlateGeometry,
    reducer: PlateKineticReducer,
    match_mode: PlateWellMatchMode,
) -> Result<ExtractedPlateValues, TransformError> {
    let (data_sets, sub_mode, xy_opt): (&[DataSetColumn], SubcolumnMode, Option<&XyTable>) = match &sheet.archetype {
        TableArchetype::Xy(xy) => (&xy.data_sets, xy.y_subcolumn_mode, Some(xy)),
        TableArchetype::Column(col) => (&col.data_sets, col.subcolumn_mode, None),
        _ => {
            return Err(TransformError::IncompatibleTable {
                operation: "extract_plate_values_from_analyzable_sheet (WellPerColumn)",
                kind: sheet.kind(),
            });
        }
    };

    let row_count = sheet.archetype.row_count();
    match reducer {
        PlateKineticReducer::RowIndex(idx) => {
            if idx >= row_count {
                return Err(TransformError::InvalidSpec {
                    message: format!("RowIndex({idx}) out of bounds for sheet `{}` with {row_count} row(s)", sheet.title),
                });
            }
        }
        PlateKineticReducer::XWindowMean { x_min, x_max } => {
            if !x_min.is_finite() || !x_max.is_finite() || x_min > x_max {
                return Err(TransformError::InvalidSpec { message: format!("XWindowMean requires finite x_min <= x_max, got [{x_min}, {x_max}]") });
            }
            let Some(xy) = xy_opt else {
                return Err(TransformError::IncompatibleTable { operation: "XWindowMean kinetic reducer", kind: sheet.kind() });
            };
            let has_numeric_x = (0..row_count).any(|r| xy.x_value_at(r).is_some_and(f64::is_finite));
            if !has_numeric_x {
                return Err(TransformError::InvalidInput {
                    message: format!("XWindowMean requires finite numeric X coordinates in sheet `{}`", sheet.title),
                });
            }
        }
        PlateKineticReducer::Min | PlateKineticReducer::Max => {}
    }

    let mut entries = Vec::with_capacity(data_sets.len());
    for ds in data_sets {
        let label = {
            let trimmed = ds.title.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        };
        let reduced = reduce_dataset_column(ds, sub_mode, row_count, xy_opt, reducer);
        entries.push((label, reduced));
    }

    assemble_extracted_plate_values(geometry, &entries, match_mode, "column", &sheet.title)
}

fn reduce_dataset_column(
    ds: &DataSetColumn,
    mode: SubcolumnMode,
    row_count: usize,
    xy_opt: Option<&XyTable>,
    reducer: PlateKineticReducer,
) -> Option<f64> {
    match reducer {
        PlateKineticReducer::RowIndex(idx) => dataset_row_active_value(ds, idx, mode),
        PlateKineticReducer::XWindowMean { x_min, x_max } => {
            let xy = xy_opt?;
            let mut vals = Vec::new();
            for r in 0..row_count {
                if let Some(x) = xy.x_value_at(r)
                    && x.is_finite()
                    && x >= x_min
                    && x <= x_max
                    && let Some(y) = dataset_row_active_value(ds, r, mode)
                {
                    vals.push(y);
                }
            }
            sum::mean(&vals).ok().filter(|v| v.is_finite())
        }
        PlateKineticReducer::Min => (0..row_count).filter_map(|r| dataset_row_active_value(ds, r, mode)).reduce(f64::min),
        PlateKineticReducer::Max => (0..row_count).filter_map(|r| dataset_row_active_value(ds, r, mode)).reduce(f64::max),
    }
}

fn format_well_preview(wells: &[String], max_items: usize) -> String {
    if wells.len() <= max_items {
        wells.join(", ")
    } else {
        let head = wells.get(..max_items).map_or_else(String::new, |slice| slice.join(", "));
        format!("{head}, ... (+{} more)", wells.len() - max_items)
    }
}

fn assemble_extracted_plate_values(
    geometry: PlateGeometry,
    entries: &[LabeledPlateEntry],
    match_mode: PlateWellMatchMode,
    axis_word: &str,
    sheet_title: &str,
) -> Result<ExtractedPlateValues, TransformError> {
    let well_count = geometry.well_count();
    let mut values = vec![None; well_count];
    let mut warnings = Vec::new();

    match match_mode {
        PlateWellMatchMode::ByWellId => {
            let mut seen = vec![false; well_count];
            let mut matched = 0_usize;
            let mut unavailable_wells = Vec::new();
            let mut unmatched_labels = Vec::new();

            for (maybe_label, maybe_val) in entries {
                let Some(label) = maybe_label.as_deref() else {
                    continue;
                };
                let Some(well_idx) = parse_leading_well_id(geometry, label) else {
                    unmatched_labels.push(label.to_owned());
                    continue;
                };
                let well_name = geometry.label_for_index(well_idx)?;
                if seen[well_idx] {
                    return Err(TransformError::InvalidInput {
                        message: format!("duplicate well ID `{well_name}` encountered while extracting plate values from `{sheet_title}`"),
                    });
                }
                seen[well_idx] = true;
                matched += 1;
                if let Some(v) = maybe_val {
                    values[well_idx] = Some(*v);
                } else {
                    unavailable_wells.push(well_name);
                }
            }

            if matched == 0 {
                return Err(TransformError::InvalidInput {
                    message: format!("no {axis_word}s matched any valid {well_count}-well IDs in sheet `{sheet_title}`"),
                });
            }

            if !unavailable_wells.is_empty() {
                warnings.push(format!(
                    "{} matched well(s) had missing/excluded values and were left empty: {}",
                    unavailable_wells.len(),
                    format_well_preview(&unavailable_wells, 8)
                ));
            }
            let missing_wells: Vec<String> =
                seen.iter().enumerate().filter(|&(_, &was_seen)| !was_seen).filter_map(|(idx, _)| geometry.label_for_index(idx).ok()).collect();
            if !missing_wells.is_empty() {
                warnings.push(format!(
                    "{} well(s) missing from `{sheet_title}` and left empty: {}",
                    missing_wells.len(),
                    format_well_preview(&missing_wells, 8)
                ));
            }
            if !unmatched_labels.is_empty() {
                warnings.push(format!(
                    "ignored {} {axis_word}(s) without recognizable {well_count}-well IDs: {}",
                    unmatched_labels.len(),
                    format_well_preview(&unmatched_labels, 4)
                ));
            }
        }
        PlateWellMatchMode::Positional => {
            if entries.is_empty() {
                return Err(TransformError::InvalidInput {
                    message: format!("source sheet `{sheet_title}` has no {axis_word}s for positional plate extraction"),
                });
            }
            let take = entries.len().min(well_count);
            let mut unavailable_wells = Vec::new();
            for (idx, (_, maybe_val)) in entries.iter().take(take).enumerate() {
                values[idx] = *maybe_val;
                if maybe_val.is_none()
                    && let Ok(lbl) = geometry.label_for_index(idx)
                {
                    unavailable_wells.push(lbl);
                }
            }
            if entries.len() > well_count {
                warnings.push(format!(
                    "source sheet `{sheet_title}` has {} {axis_word}(s); ignored {} trailing {axis_word}(s) beyond {well_count} wells",
                    entries.len(),
                    entries.len() - well_count
                ));
            } else if entries.len() < well_count {
                warnings.push(format!(
                    "source sheet `{sheet_title}` supplied {} {axis_word}(s) for {well_count}-well plate; remaining {} well(s) left empty",
                    entries.len(),
                    well_count - entries.len()
                ));
            }
            if !unavailable_wells.is_empty() {
                warnings.push(format!(
                    "{} positional well(s) had missing/excluded values and were left empty: {}",
                    unavailable_wells.len(),
                    format_well_preview(&unavailable_wells, 8)
                ));
            }
        }
    }

    Ok(ExtractedPlateValues { values, warnings })
}

fn append_plate_floating_notes(sheet: &mut DataSheet, collected: &CollectedPlate, collisions: usize) {
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
    if collisions > 0 {
        lines.push(format!("Warning: {collisions} replicate slot collision(s) shifted to adjacent subcolumns"));
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
    /// Optional zero-based column index supplying independent *X* values (producing an `XyTable`) or row-factor labels (producing a `GroupedTable`); `None` pivots into a `ColumnTable`.
    pub x_col: Option<usize>,
    /// Zero-based column index supplying treatment/data-set column names.
    pub group_col: usize,
    /// Optional zero-based column index identifying replicate/subject IDs within `(x_col, group_col)`; `None` assigns sequential replicate slots in row-appearance order.
    pub replicate_col: Option<usize>,
    /// Zero-based column index supplying the response *Y* measurement.
    pub value_col: usize,
    /// Optional zero-based boolean/flag/reason column index marking pivoted cells as `excluded = true` (and attaching [`curvebook_doc::CellExclusionNote`] text when non-boolean); `None` relies only on inline `*` tokens.
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
        TargetArchetypeKind::Xy => pivot_tidy_to_xy(title, headers, mapping, &records, decimal_sep),
        TargetArchetypeKind::Grouped => pivot_tidy_to_grouped(title, headers, mapping, &records, decimal_sep),
        TargetArchetypeKind::MultipleVariables => pivot_tidy_to_mv(title, headers, mapping, &records, decimal_sep),
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
                let (core_x, _) = strip_tidy_asterisk(raw_x);
                if core_x.is_empty() || is_missing_token(core_x) {
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
                    (Some(core_x.to_owned()), num_x, parsed_x.excluded)
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

fn strip_tidy_asterisk(raw: &str) -> (&str, bool) {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix('*') {
        (rest.strip_suffix('*').unwrap_or(rest).trim(), true)
    } else if let Some(rest) = t.strip_suffix('*') {
        (rest.trim(), true)
    } else {
        (t, false)
    }
}

fn parse_tidy_cell(raw: &str, decimal_sep: char) -> Cell {
    let (core, excluded) = strip_tidy_asterisk(raw);
    if is_missing_token(core) {
        return Cell::empty();
    }
    let mut cell = Cell::parse_import_token(core, decimal_sep, false);
    if excluded {
        cell.excluded = true;
    }
    cell
}

fn parse_exclusion_flag(flag: &str, row_idx: usize) -> Option<String> {
    let trimmed = flag.trim();
    if is_missing_token(trimmed)
        || trimmed == "-"
        || trimmed.eq_ignore_ascii_case("false")
        || trimmed.eq_ignore_ascii_case("no")
        || trimmed.eq_ignore_ascii_case("n")
        || trimmed.eq_ignore_ascii_case("f")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("nil")
    {
        return None;
    }
    if trimmed.replace(',', ".").parse::<f64>().is_ok_and(|v| v == 0.0) {
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
        validate_title(&rec.group)?;
        let g = ensure_group_slot(&rec.group, &mut group_names, &mut group_map)?;
        while columns.len() <= g {
            columns.push(Vec::new());
        }
        if columns[g].len() >= MAX_ROWS {
            return Err(DocError::TooManyRows { count: columns[g].len() + 1, max: MAX_ROWS }.into());
        }
        columns[g].push((rec.value.clone(), rec.exclusion_note.clone()));
    }

    let rows = columns.iter().map(Vec::len).max().unwrap_or(0).max(1);
    validate_data_sets(group_names.len())?;
    validate_rows(rows)?;
    validate_cell_budget(rows, group_names.len(), 1, 0, "tidy Column pivot")?;
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, group_names.len(), rows)?;
    let mut notes = Vec::new();
    for (c, name) in group_names.into_iter().enumerate() {
        col.data_sets[c].title = name;
        for (r, (cell, maybe_note)) in columns[c].iter().enumerate() {
            if let Some(slot) = col.data_sets[c].cells.get_mut(r).and_then(|row| row.get_mut(0)) {
                *slot = cell.clone();
            }
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

fn parse_finite_with_decimal_sep(s: &str, decimal_sep: char) -> Option<f64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = if decimal_sep == ',' { trimmed.replace(',', ".").parse::<f64>().ok() } else { trimmed.parse::<f64>().ok() };
    parsed.filter(|v| v.is_finite()).map(normalize_zero)
}

fn append_tidy_collision_note(sheet: &mut DataSheet, collisions: usize) {
    if collisions > 0 {
        sheet.floating_notes.push(FloatingNote::new(
            format!("{}-tidy-qc", sheet.id.as_str()),
            RectPt::new(12.0, 12.0, 260.0, 64.0),
            format!("Warning: {collisions} replicate slot collision(s) shifted to adjacent subcolumns"),
        ));
    }
}

fn pivot_tidy_to_xy(
    title: &str,
    headers: &[String],
    mapping: &TidyColumnMapping,
    records: &[TidyRecord],
    decimal_sep: char,
) -> Result<DataSheet, TransformError> {
    let mut group_names = Vec::new();
    let mut group_map = BTreeMap::new();
    let mut x_by_bits: BTreeMap<u64, f64> = BTreeMap::new();
    let mut x_any_active_bits: BTreeSet<u64> = BTreeSet::new();
    let mut x_any_excluded_bits: BTreeSet<u64> = BTreeSet::new();

    for rec in records {
        validate_title(&rec.group)?;
        ensure_group_slot(&rec.group, &mut group_names, &mut group_map)?;
        if let Some(x) = rec.x_num {
            let nx = normalize_zero(x);
            let bits = nx.to_bits();
            if !x_by_bits.contains_key(&bits) {
                if x_by_bits.len() >= MAX_ROWS {
                    return Err(DocError::TooManyRows { count: x_by_bits.len() + 1, max: MAX_ROWS }.into());
                }
                x_by_bits.insert(bits, nx);
            }
            if rec.x_excluded {
                x_any_excluded_bits.insert(bits);
            } else {
                x_any_active_bits.insert(bits);
            }
        }
    }
    if x_by_bits.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no finite numeric X entries available for Xy pivot") });
    }
    let x_row_excluded = |bits: u64| x_any_excluded_bits.contains(&bits) && !x_any_active_bits.contains(&bits);

    let mut x_vals: Vec<f64> = x_by_bits.into_values().collect();
    x_vals.sort_by(f64::total_cmp);
    let x_row_by_bits: BTreeMap<u64, usize> = x_vals.iter().enumerate().map(|(r, &x)| (normalize_zero(x).to_bits(), r)).collect();

    let (use_named_reps, rep_slots) = build_replicate_slot_map(records, mapping.replicate_col.is_some(), decimal_sep)?;
    let mut placed: PlacedCellGrid = BTreeMap::new();
    let mut next_auto: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut max_sub = 0_usize;
    let mut collisions = 0_usize;

    for rec in records {
        let Some(&g) = group_map.get(&rec.group) else { continue };
        let Some(x) = rec.x_num else { continue };
        let bits = normalize_zero(x).to_bits();
        let Some(&r) = x_row_by_bits.get(&bits) else { continue };

        let slot = if use_named_reps {
            let base = *rep_slots.get(&rec.replicate).unwrap_or(&0);
            let mut s = base;
            while placed.contains_key(&(r, g, s)) {
                if s == base {
                    collisions += 1;
                }
                s += 1;
                if s >= MAX_SUBCOLUMNS {
                    return Err(DocError::TooManySubcolumns { count: s + 1, max: MAX_SUBCOLUMNS }.into());
                }
            }
            s
        } else {
            let cursor = next_auto.entry((r, g)).or_insert(0);
            let s = *cursor;
            if s >= MAX_SUBCOLUMNS {
                return Err(DocError::TooManySubcolumns { count: s + 1, max: MAX_SUBCOLUMNS }.into());
            }
            *cursor += 1;
            s
        };
        let mut cell = rec.value.clone();
        let mut note = rec.exclusion_note.clone();
        if rec.x_excluded && !x_row_excluded(bits) {
            cell.excluded = true;
            if note.is_none() {
                note = Some((String::from("asterisk-excluded"), String::from("X coordinate marked excluded")));
            }
        }
        placed.insert((r, g, slot), (cell, note));
        if slot > max_sub {
            max_sub = slot;
        }
    }

    let subcol_count = (max_sub + 1).max(rep_slots.len()).max(1);
    validate_data_sets(group_names.len())?;
    validate_rows(x_vals.len())?;
    validate_subcolumns(subcol_count)?;
    if let Some(xc) = mapping.x_col
        && let Some(h) = headers.get(xc)
    {
        validate_title(h.trim())?;
    }
    validate_cell_budget(x_vals.len(), group_names.len(), subcol_count, 1, "tidy XY pivot")?;

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
        xy.x_cells[r][0] = Cell { value: CellValue::Number(x), excluded: x_row_excluded(normalize_zero(x).to_bits()) };
    }
    for (c, name) in group_names.into_iter().enumerate() {
        xy.data_sets[c].title = name;
    }

    let notes = fill_y_cells_and_notes(&mut xy.data_sets, placed);
    let mut sheet = DataSheet::new(make_sheet_id("tidy", title), title, TableArchetype::Xy(xy))?;
    sheet.exclusion_notes = notes;
    append_tidy_collision_note(&mut sheet, collisions);
    Ok(sheet)
}

fn apply_tidy_x_exclusion(rec: &TidyRecord) -> PlacedCell {
    let mut cell = rec.value.clone();
    let mut note = rec.exclusion_note.clone();
    if rec.x_excluded {
        cell.excluded = true;
        if note.is_none() {
            note = Some((String::from("asterisk-excluded"), String::from("X coordinate marked excluded")));
        }
    }
    (cell, note)
}

fn pivot_tidy_to_grouped(
    title: &str,
    headers: &[String],
    mapping: &TidyColumnMapping,
    records: &[TidyRecord],
    decimal_sep: char,
) -> Result<DataSheet, TransformError> {
    let mut group_names = Vec::new();
    let mut group_map = BTreeMap::new();
    for rec in records {
        validate_title(&rec.group)?;
        ensure_group_slot(&rec.group, &mut group_names, &mut group_map)?;
    }

    let (row_titles, row_lookup) = build_grouped_row_index(mapping.x_col.is_some(), records)?;
    let (use_named_reps, rep_slots) = build_replicate_slot_map(records, mapping.replicate_col.is_some(), decimal_sep)?;

    let mut placed: PlacedCellGrid = BTreeMap::new();
    let mut next_row_or_sub: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut max_sub = 0_usize;
    let mut extra_rows = row_titles.len();
    let mut collisions = 0_usize;

    for rec in records {
        let Some(&g) = group_map.get(&rec.group) else { continue };
        let placed_cell = apply_tidy_x_exclusion(rec);
        if mapping.x_col.is_some() {
            let key = grouped_row_key(rec);
            let Some(&r) = row_lookup.get(&key) else { continue };
            let s = if use_named_reps {
                let base = *rep_slots.get(&rec.replicate).unwrap_or(&0);
                let mut slot = base;
                while placed.contains_key(&(r, g, slot)) {
                    if slot == base {
                        collisions += 1;
                    }
                    slot += 1;
                    if slot >= MAX_SUBCOLUMNS {
                        return Err(DocError::TooManySubcolumns { count: slot + 1, max: MAX_SUBCOLUMNS }.into());
                    }
                }
                slot
            } else {
                let cursor = next_row_or_sub.entry((r, g)).or_insert(0);
                let slot = *cursor;
                if slot >= MAX_SUBCOLUMNS {
                    return Err(DocError::TooManySubcolumns { count: slot + 1, max: MAX_SUBCOLUMNS }.into());
                }
                *cursor += 1;
                slot
            };
            placed.insert((r, g, s), placed_cell);
            if s > max_sub {
                max_sub = s;
            }
        } else {
            let s = if use_named_reps { *rep_slots.get(&rec.replicate).unwrap_or(&0) } else { 0 };
            let cursor = next_row_or_sub.entry((g, s)).or_insert(0);
            let r = *cursor;
            if r >= MAX_ROWS {
                return Err(DocError::TooManyRows { count: r + 1, max: MAX_ROWS }.into());
            }
            *cursor += 1;
            if r >= extra_rows {
                extra_rows = r + 1;
            }
            if s > max_sub {
                max_sub = s;
            }
            placed.insert((r, g, s), placed_cell);
        }
    }

    let row_count = extra_rows.max(1);
    validate_rows(row_count)?;
    let mut final_row_titles = row_titles;
    while final_row_titles.len() < row_count {
        final_row_titles.push(format!("Row {}", final_row_titles.len() + 1));
    }
    for rt in &final_row_titles {
        validate_title(rt)?;
    }

    let subcol_count = (max_sub + 1).max(rep_slots.len()).max(1);
    validate_data_sets(group_names.len())?;
    validate_rows(row_count)?;
    validate_subcolumns(subcol_count)?;
    if let Some(xc) = mapping.x_col
        && let Some(h) = headers.get(xc)
    {
        validate_title(h.trim())?;
    }
    if let Some(h) = headers.get(mapping.group_col) {
        validate_title(h.trim())?;
    }
    validate_cell_budget(row_count, group_names.len(), subcol_count, 0, "tidy Grouped pivot")?;

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
    append_tidy_collision_note(&mut sheet, collisions);
    Ok(sheet)
}

fn pivot_tidy_to_mv(
    title: &str,
    headers: &[String],
    mapping: &TidyColumnMapping,
    records: &[TidyRecord],
    decimal_sep: char,
) -> Result<DataSheet, TransformError> {
    let rows = records.len();
    validate_rows(rows)?;
    let has_x = mapping.x_col.is_some();
    let has_rep = mapping.replicate_col.is_some();
    let var_count = 2 + usize::from(has_x) + usize::from(has_rep);
    validate_data_sets(var_count)?;
    let x_title = header_or(headers, mapping.x_col, "X");
    let grp_title = header_or(headers, Some(mapping.group_col), "Group");
    let rep_title = header_or(headers, mapping.replicate_col, "Replicate");
    let val_title = header_or(headers, Some(mapping.value_col), "Value");
    validate_title(&x_title)?;
    validate_title(&grp_title)?;
    validate_title(&rep_title)?;
    validate_title(&val_title)?;
    validate_cell_budget(rows, var_count, 1, 0, "tidy MultipleVariables pivot")?;
    let mut mv = MultipleVariablesTable::new(var_count, rows)?;

    let mut next_idx = 0_usize;
    let x_var = has_x.then(|| {
        let i = next_idx;
        next_idx += 1;
        mv.variables[i].title = x_title;
        i
    });
    let grp_var = next_idx;
    next_idx += 1;
    mv.variables[grp_var].title = grp_title;
    let rep_var = has_rep.then(|| {
        let i = next_idx;
        next_idx += 1;
        mv.variables[i].title = rep_title;
        i
    });
    let val_var = next_idx;
    mv.variables[val_var].title = val_title;

    let mut notes = Vec::new();
    for (r, rec) in records.iter().enumerate() {
        if let Some(xi) = x_var {
            mv.variables[xi].cells[r] = match (rec.x_num, rec.x_token.as_ref()) {
                (Some(n), _) => Cell { value: CellValue::Number(n), excluded: rec.x_excluded },
                (None, Some(tok)) => Cell { value: CellValue::Text(tok.clone()), excluded: rec.x_excluded },
                (None, None) => Cell::empty(),
            };
            if mv.variables[xi].cells[r].excluded {
                notes.push(CellExclusionNote {
                    coord: CellCoord::new(r, xi, 0),
                    reason_tag: String::from("asterisk-excluded"),
                    note: String::from("X coordinate marked with '*'"),
                    timestamp_utc: String::new(),
                });
            }
        }
        mv.variables[grp_var].cells[r] = Cell::text(rec.group.clone());
        if let Some(ri) = rep_var
            && !rec.replicate.is_empty()
        {
            mv.variables[ri].cells[r] = match parse_finite_with_decimal_sep(&rec.replicate, decimal_sep) {
                Some(n) => Cell::number(n),
                None => Cell::text(rec.replicate.clone()),
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

fn fill_y_cells_and_notes(data_sets: &mut [curvebook_doc::DataSetColumn], placed: PlacedCellGrid) -> Vec<CellExclusionNote> {
    let mut notes = Vec::new();
    for ((r, c, s), (cell, maybe_note)) in placed {
        if let Some(slot) = data_sets.get_mut(c).and_then(|d| d.cells.get_mut(r)).and_then(|row| row.get_mut(s)) {
            *slot = cell.clone();
        }
        if cell.excluded {
            let (tag, note) = maybe_note.unwrap_or_else(|| (String::from("excluded"), String::from("Excluded value")));
            notes.push(CellExclusionNote { coord: CellCoord::new(r, c, s), reason_tag: tag, note, timestamp_utc: String::new() });
        }
    }
    notes.sort_by_key(|n| n.coord);
    notes
}

fn ensure_group_slot(name: &str, names: &mut Vec<String>, map: &mut BTreeMap<String, usize>) -> Result<usize, TransformError> {
    if let Some(&idx) = map.get(name) {
        Ok(idx)
    } else {
        if names.len() >= MAX_DATA_SETS {
            return Err(DocError::TooManyDataSets { count: names.len() + 1, max: MAX_DATA_SETS }.into());
        }
        let idx = names.len();
        names.push(name.to_owned());
        map.insert(name.to_owned(), idx);
        Ok(idx)
    }
}

fn build_replicate_slot_map(
    records: &[TidyRecord],
    has_replicate_col: bool,
    decimal_sep: char,
) -> Result<(bool, BTreeMap<String, usize>), TransformError> {
    if !has_replicate_col {
        return Ok((false, BTreeMap::new()));
    }
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for r in records {
        if !keys.contains(&r.replicate) {
            if keys.len() >= MAX_SUBCOLUMNS {
                return Err(DocError::TooManySubcolumns { count: keys.len() + 1, max: MAX_SUBCOLUMNS }.into());
            }
            keys.insert(r.replicate.clone());
        }
    }
    if keys.is_empty() {
        return Ok((false, BTreeMap::new()));
    }
    let mut ordered: Vec<(Option<f64>, String)> = keys.into_iter().map(|k| (parse_finite_with_decimal_sep(&k, decimal_sep), k)).collect();
    // Strict total order: finite numeric keys precede non-numeric text keys.
    ordered.sort_by(|(na, a), (nb, b)| match (na, nb) {
        (Some(x), Some(y)) => x.total_cmp(y).then_with(|| a.cmp(b)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.cmp(b),
    });
    let map = ordered.into_iter().enumerate().map(|(idx, (_, k))| (k, idx)).collect();
    Ok((true, map))
}

fn grouped_row_key(rec: &TidyRecord) -> String {
    match (&rec.x_token, rec.x_num) {
        (Some(tok), _) => tok.clone(),
        (None, Some(n)) => format_compact_f64(n),
        (None, None) => String::from("(Missing)"),
    }
}

fn build_grouped_row_index(has_x_col: bool, records: &[TidyRecord]) -> Result<(Vec<String>, BTreeMap<String, usize>), TransformError> {
    if !has_x_col {
        return Ok((Vec::new(), BTreeMap::new()));
    }
    let all_numeric = records.iter().filter(|r| r.x_token.is_some()).all(|r| r.x_num.is_some());

    if all_numeric && records.iter().any(|r| r.x_num.is_some()) {
        let mut by_bits: BTreeMap<u64, String> = BTreeMap::new();
        for rec in records {
            if let Some(n) = rec.x_num {
                let norm = normalize_zero(n);
                let bits = norm.to_bits();
                if !by_bits.contains_key(&bits) {
                    if by_bits.len() >= MAX_ROWS {
                        return Err(DocError::TooManyRows { count: by_bits.len() + 1, max: MAX_ROWS }.into());
                    }
                    let label = rec.x_token.clone().unwrap_or_else(|| format_compact_f64(norm));
                    by_bits.insert(bits, label);
                }
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
        if records.iter().any(|r| r.x_token.is_none() && r.x_num.is_none()) {
            if titles.len() >= MAX_ROWS {
                return Err(DocError::TooManyRows { count: titles.len() + 1, max: MAX_ROWS }.into());
            }
            let idx = titles.len();
            titles.push(String::from("(Missing)"));
            lookup.insert(String::from("(Missing)"), idx);
        }
        return Ok((titles, lookup));
    }

    let mut titles = Vec::new();
    let mut lookup = BTreeMap::new();
    for rec in records {
        let key = grouped_row_key(rec);
        if let std::collections::btree_map::Entry::Vacant(slot) = lookup.entry(key) {
            if titles.len() >= MAX_ROWS {
                return Err(DocError::TooManyRows { count: titles.len() + 1, max: MAX_ROWS }.into());
            }
            let idx = titles.len();
            titles.push(slot.key().clone());
            slot.insert(idx);
        }
    }
    Ok((titles, lookup))
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

#[cfg(test)]
mod audit_w1_plate_tidy_tests {
    use curvebook_doc::{DocError, MAX_SUBCOLUMNS, TableArchetype};

    use super::{TidyColumnMapping, pivot_tidy_long_to_datasheet};
    use crate::archetype_convert::TargetArchetypeKind;
    use crate::error::TransformError;

    #[test]
    fn aud_04_01_mixed_numeric_and_text_replicate_labels_never_panic() {
        let headers: Vec<String> = ["X", "Group", "Rep", "Val"].into_iter().map(String::from).collect();
        let mut rows = Vec::new();
        for i in 1..=20 {
            rows.push(vec![String::from("1"), String::from("A"), format!("{i}"), format!("{i}")]);
            rows.push(vec![String::from("1"), String::from("A"), format!("{i}a"), format!("{}", i + 100)]);
            rows.push(vec![String::from("1"), String::from("A"), format!("-{i}"), format!("{}", i + 200)]);
        }
        let mapping = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: Some(2), value_col: 3, excluded_col: None };
        let xy = pivot_tidy_long_to_datasheet("Mixed Reps XY", &headers, &rows, &mapping, TargetArchetypeKind::Xy, '.').unwrap();
        let TableArchetype::Xy(t) = &xy.archetype else { panic!("expected Xy") };
        assert_eq!(t.y_subcolumn_mode.subcolumn_count(), 60);

        let grp = pivot_tidy_long_to_datasheet("Mixed Reps Grouped", &headers, &rows, &mapping, TargetArchetypeKind::Grouped, '.').unwrap();
        let TableArchetype::Grouped(g) = &grp.archetype else { panic!("expected Grouped") };
        assert_eq!(g.subcolumn_mode.subcolumn_count(), 60);
    }

    #[test]
    fn aud_04_21_duplicate_replicate_collisions_fail_fast_at_max_subcolumns() {
        let headers: Vec<String> = ["X", "Group", "Rep", "Val"].into_iter().map(String::from).collect();
        let rows: Vec<Vec<String>> =
            (0..(MAX_SUBCOLUMNS + 2000)).map(|_| vec![String::from("1"), String::from("A"), String::from("R1"), String::from("5")]).collect();
        let mapping = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: Some(2), value_col: 3, excluded_col: None };
        let err_xy = pivot_tidy_long_to_datasheet("Dup XY", &headers, &rows, &mapping, TargetArchetypeKind::Xy, '.').unwrap_err();
        assert!(matches!(err_xy, TransformError::Doc(DocError::TooManySubcolumns { .. })));
        let err_grp = pivot_tidy_long_to_datasheet("Dup Grp", &headers, &rows, &mapping, TargetArchetypeKind::Grouped, '.').unwrap_err();
        assert!(matches!(err_grp, TransformError::Doc(DocError::TooManySubcolumns { .. })));
    }

    #[test]
    fn aud_04_44_starred_missing_x_tokens_treated_as_missing_not_literal_text() {
        let headers: Vec<String> = ["X", "Group", "Val"].into_iter().map(String::from).collect();
        let rows: Vec<Vec<String>> = [("1", "A", "10"), ("10", "A", "20"), ("2", "A", "30"), ("NA*", "A", "40"), ("*", "A", "50")]
            .into_iter()
            .map(|(x, g, v)| vec![x.to_owned(), g.to_owned(), v.to_owned()])
            .collect();
        let mapping = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: None, value_col: 2, excluded_col: None };

        // Grouped keeps numeric row sorting [1, 2, 10] and buckets starred missing X into "(Missing)".
        let grp_sheet = pivot_tidy_long_to_datasheet("Starred Missing Grp", &headers, &rows, &mapping, TargetArchetypeKind::Grouped, '.').unwrap();
        let TableArchetype::Grouped(g) = &grp_sheet.archetype else { panic!("expected Grouped") };
        assert_eq!(g.row_titles, vec!["1", "2", "10", "(Missing)"]);

        // XY skips missing X rows ("NA*" and "*") without failing as non-numeric.
        let xy_sheet = pivot_tidy_long_to_datasheet("Starred Missing XY", &headers, &rows, &mapping, TargetArchetypeKind::Xy, '.').unwrap();
        let TableArchetype::Xy(xy) = &xy_sheet.archetype else { panic!("expected Xy") };
        assert_eq!(xy.row_count(), 3);
    }
}
