//! Non-destructive table archetype preview and conversion (`QOL-1`).
//!
//! Supports lossless and lossy-guarded projections among `Column`, `Grouped`, `Xy`,
//! `MultipleVariables`, and `Contingency` (as a target) tables while preserving sheet metadata,
//! cell `excluded` masks, and remapping [`CellExclusionNote`] coordinates to their new [`CellCoord`]
//! positions.

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{
    Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, ContingencyTable, DataSetColumn, DataSheet, DocError, GroupedTable,
    MAX_DATA_SETS, MAX_ROWS, MAX_SUBCOLUMNS, MultipleVariablesTable, PlateCellOrigin, SequenceProgression, SubcolumnMode, TableArchetype, TableKind,
    VariableColumn, VariableRole, VariableSource, XColumnMode, XyTable, default_column_label, validate_data_sets, validate_rows, validate_subcolumns,
    validate_title,
};

use crate::error::TransformError;
use crate::import::validate_cell_budget;

/// Target table archetype supported by non-destructive table conversion (`QOL-1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TargetArchetypeKind {
    /// Convert into a one-factor [`ColumnTable`].
    Column,
    /// Convert into a two-factor [`GroupedTable`] with raw replicate subcolumns.
    Grouped,
    /// Convert into a continuous *X* vs *Y* [`XyTable`].
    Xy,
    /// Convert into a tidy observational [`MultipleVariablesTable`].
    MultipleVariables,
    /// Convert into an `R × C` non-negative integer count [`ContingencyTable`].
    Contingency,
}

impl TargetArchetypeKind {
    /// Convert into the corresponding [`TableKind`] discriminant.
    #[must_use]
    pub const fn as_table_kind(self) -> TableKind {
        match self {
            Self::Column => TableKind::Column,
            Self::Grouped => TableKind::Grouped,
            Self::Xy => TableKind::Xy,
            Self::MultipleVariables => TableKind::MultipleVariables,
            Self::Contingency => TableKind::Contingency,
        }
    }

    /// Neutral human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        self.as_table_kind().label()
    }
}

/// Non-mutating impact summary produced before or during archetype conversion (`QOL-1`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConversionPreview {
    /// Archetype of the input [`DataSheet`].
    pub source_kind: TableKind,
    /// Requested destination [`TargetArchetypeKind`].
    pub target_kind: TargetArchetypeKind,
    /// Count of non-empty cells carried over into the converted table.
    pub preserved_cells: usize,
    /// Count of non-empty cells that cannot be represented in `target_kind` (for example non-integer/negative values when targeting `Contingency` or collapsed replicate cells).
    pub dropped_cells: usize,
    /// Count of `excluded == true` cell flags preserved across the coordinate remapping.
    pub preserved_exclusions: usize,
    /// Plain-language descriptions of any structural simplifications, coerced cells, or dropped subcolumns.
    pub warnings: Vec<String>,
    /// `true` when conversion drops cells, coerces values, or discards structural metadata (refused by [`convert_datasheet_archetype`] unless `allow_lossy == true`).
    pub lossy: bool,
}

/// Inspect how converting `sheet` into `target` will map cells, exclusions, and subcolumns
/// without mutating `sheet`.
///
/// Low-level archetype conversion does not remap [`DataSheet::linked_regions`]; if the source sheet
/// has attached live links, a non-lossy informational warning is reported so higher-level callers
/// (such as `curvebook-engine`) can re-attach or detach them explicitly.
pub fn preview_datasheet_conversion(sheet: &DataSheet, target: TargetArchetypeKind) -> Result<ConversionPreview, TransformError> {
    let (_, preview) = build_converted_datasheet(sheet, target)?;
    Ok(preview)
}

/// Convert `sheet` into `target`, preserving metadata, cell `excluded` masks, and remapped
/// [`CellExclusionNote`] coordinates.
///
/// Live [`DataSheet::linked_regions`] are not automatically remapped by this low-level routine and
/// are left empty on the returned [`DataSheet`] (emitting an informational warning when present on
/// `sheet`); higher-level orchestrators (`curvebook-engine`) remap or guard incoming links
/// separately.
///
/// Refuses lossy conversions with [`TransformError::InvalidSpec`] when `allow_lossy` is `false`.
pub fn convert_datasheet_archetype(
    sheet: &DataSheet,
    target: TargetArchetypeKind,
    allow_lossy: bool,
) -> Result<(DataSheet, ConversionPreview), TransformError> {
    let (converted, preview) = build_converted_datasheet(sheet, target)?;
    if preview.lossy && !allow_lossy {
        let reason = preview.warnings.first().map_or("conversion would drop data or schema structure", String::as_str);
        return Err(TransformError::InvalidSpec {
            message: format!(
                "lossy conversion from {} to {} refused without allow_lossy=true ({} dropped cell(s)): {reason}",
                preview.source_kind.label(),
                preview.target_kind.label(),
                preview.dropped_cells
            ),
        });
    }
    Ok((converted, preview))
}

type RepBucketMap<T> = BTreeMap<(usize, usize), Vec<(Option<f64>, usize, T)>>;
type SlottedBucketMap<T> = BTreeMap<(usize, usize), Vec<(usize, usize, T)>>;

#[derive(Default)]
struct ConversionAcc {
    coord_map: BTreeMap<CellCoord, CellCoord>,
    preserved: usize,
    dropped: usize,
    warnings: Vec<String>,
    schema_lossy: bool,
}

impl ConversionAcc {
    fn record_preserved(&mut self, cell: &Cell) {
        if is_occupied(cell) {
            self.preserved += 1;
        }
    }

    fn record_dropped(&mut self, cell: &Cell) {
        if is_occupied(cell) {
            self.dropped += 1;
        }
    }

    fn drop_cells(&mut self, count: usize) {
        if count > 0 {
            self.dropped += count;
            self.schema_lossy = true;
        }
    }

    fn drop_mv_columns(&mut self, mv: &MultipleVariablesTable, cols: impl IntoIterator<Item = usize>, rows: usize) -> bool {
        let mut any = false;
        for col in cols {
            self.dropped += count_var_occupied(&mv.variables[col], rows);
            self.schema_lossy = true;
            any = true;
        }
        any
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }

    fn warn_lossy(&mut self, msg: impl Into<String>) {
        self.schema_lossy = true;
        self.warnings.push(msg.into());
    }

    fn map_coord(&mut self, src: CellCoord, dst: CellCoord) {
        self.coord_map.insert(src, dst);
    }

    fn finish(self, archetype: TableArchetype) -> BuiltArchetype {
        BuiltArchetype { archetype, acc: self }
    }
}

struct BuiltArchetype {
    archetype: TableArchetype,
    acc: ConversionAcc,
}

/// Shared dataset/subcolumn copier and unpacker keeping `CellCoord` remapping and
/// occupied/dropped cell accounting consistent across all pairwise converters.
struct DataSetSubcolumnUnpacker<'a> {
    acc: &'a mut ConversionAcc,
    rows: usize,
}

impl<'a> DataSetSubcolumnUnpacker<'a> {
    const fn new(acc: &'a mut ConversionAcc, rows: usize) -> Self {
        Self { acc, rows }
    }

    fn copy_direct(&mut self, src: &[DataSetColumn], dst: &mut [DataSetColumn], src_col_offset: usize, dst_col_offset: usize, subs: usize) {
        for (i, src_ds) in src.iter().enumerate() {
            let src_c = src_col_offset + i;
            let dst_c = dst_col_offset + i;
            dst[dst_c].title = src_ds.title.clone();
            for (r, row_cells) in src_ds.cells.iter().enumerate().take(self.rows) {
                for s in 0..subs {
                    if let Some(cell) = row_cells.get(s) {
                        set_dataset_cell(dst, dst_c, r, s, cell.clone());
                        self.acc.map_coord(CellCoord::new(r, src_c, s), CellCoord::new(r, dst_c, s));
                        self.acc.record_preserved(cell);
                    }
                }
            }
        }
    }

    fn copy_primary_only(&mut self, src: &[DataSetColumn], dst: &mut [DataSetColumn], dst_col_offset: usize) -> bool {
        let mut had_extra_subcols = false;
        for (c, src_ds) in src.iter().enumerate() {
            let dst_c = dst_col_offset + c;
            dst[dst_c].title = src_ds.title.clone();
            for (r, row_cells) in src_ds.cells.iter().enumerate().take(self.rows) {
                for (s, cell) in row_cells.iter().enumerate() {
                    if s == 0 {
                        set_dataset_cell(dst, dst_c, r, 0, cell.clone());
                        self.acc.map_coord(CellCoord::new(r, c, 0), CellCoord::new(r, dst_c, 0));
                        self.acc.record_preserved(cell);
                    } else {
                        had_extra_subcols = true;
                        self.acc.record_dropped(cell);
                    }
                }
            }
        }
        had_extra_subcols
    }

    fn unpack_to_mv(&mut self, src: &[DataSetColumn], mv: &mut MultipleVariablesTable, dst_col_offset: usize, subs: usize) {
        for (c, src_ds) in src.iter().enumerate() {
            for s in 0..subs {
                let dst_c = dst_col_offset + c * subs + s;
                for (r, row_cells) in src_ds.cells.iter().enumerate().take(self.rows) {
                    if let Some(cell) = row_cells.get(s) {
                        set_mv_cell(mv, dst_c, r, cell.clone());
                        self.acc.map_coord(CellCoord::new(r, c, s), CellCoord::new(r, dst_c, 0));
                        self.acc.record_preserved(cell);
                    }
                }
            }
        }
    }

    fn copy_mv_vars_to_datasets(&mut self, mv: &MultipleVariablesTable, src_indices: &[usize], dst: &mut [DataSetColumn]) {
        for (dst_c, &src_c) in src_indices.iter().enumerate() {
            let var = &mv.variables[src_c];
            dst[dst_c].title = var.title.clone();
            if matches!(var.source, VariableSource::Calculated { .. }) {
                self.acc.warn(format!("Calculated variable `{}` evaluated/frozen to static cells", var.title));
            }
            for (r, cell) in var.cells.iter().enumerate().take(self.rows) {
                set_dataset_cell(dst, dst_c, r, 0, cell.clone());
                self.acc.map_coord(CellCoord::new(r, src_c, 0), CellCoord::new(r, dst_c, 0));
                self.acc.record_preserved(cell);
            }
        }
    }
}

#[derive(Default)]
struct LevelInterner {
    levels: Vec<String>,
    index: BTreeMap<String, usize>,
}

impl LevelInterner {
    fn intern(&mut self, token: String) -> usize {
        if let Some(&idx) = self.index.get(&token) {
            idx
        } else {
            let idx = self.levels.len();
            self.index.insert(token.clone(), idx);
            self.levels.push(token);
            idx
        }
    }

    fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    fn len(&self) -> usize {
        self.levels.len()
    }

    fn into_levels(self) -> Vec<String> {
        self.levels
    }
}

struct MvPartition {
    rows: usize,
    cat_cols: Vec<usize>,
    label_cols: Vec<usize>,
    num_cols: Vec<usize>,
}

impl MvPartition {
    fn from_table(mv: &MultipleVariablesTable) -> Result<Self, TransformError> {
        if mv.variables.is_empty() {
            return Err(TransformError::InvalidInput { message: String::from("MultipleVariables table has no variables") });
        }
        let rows = mv_row_count(mv);
        let mut cat_cols = Vec::new();
        let mut label_cols = Vec::new();
        let mut num_cols = Vec::new();
        for (i, var) in mv.variables.iter().enumerate() {
            if var.role == VariableRole::Categorical {
                cat_cols.push(i);
            } else if var.role == VariableRole::Label {
                label_cols.push(i);
            } else if var.role_inferred && var.cells.iter().any(|c| !c.is_empty() && matches!(c.value, CellValue::Text(_))) {
                cat_cols.push(i);
            } else {
                num_cols.push(i);
            }
        }
        Ok(Self { rows, cat_cols, label_cols, num_cols })
    }
}

fn build_converted_datasheet(sheet: &DataSheet, target: TargetArchetypeKind) -> Result<(DataSheet, ConversionPreview), TransformError> {
    let source_kind = sheet.kind();
    if source_kind == target.as_table_kind() {
        return Ok(build_identity_result(sheet, target));
    }

    let built = match (&sheet.archetype, target) {
        (TableArchetype::Column(col), TargetArchetypeKind::Grouped) => convert_column_to_grouped(col)?,
        (TableArchetype::Grouped(grp), TargetArchetypeKind::Column) => convert_grouped_to_column(grp)?,
        (TableArchetype::Column(col), TargetArchetypeKind::Xy) => convert_column_to_xy(col)?,
        (TableArchetype::Xy(xy), TargetArchetypeKind::Column) => convert_xy_to_column(xy)?,
        (TableArchetype::Grouped(grp), TargetArchetypeKind::MultipleVariables) => convert_grouped_to_mv(grp)?,
        (TableArchetype::MultipleVariables(mv), TargetArchetypeKind::Grouped) => convert_mv_to_grouped(mv)?,
        (TableArchetype::Xy(xy), TargetArchetypeKind::MultipleVariables) => convert_xy_to_mv(xy)?,
        (TableArchetype::MultipleVariables(mv), TargetArchetypeKind::Xy) => convert_mv_to_xy(mv)?,
        (TableArchetype::Column(col), TargetArchetypeKind::MultipleVariables) => convert_column_to_mv(col)?,
        (TableArchetype::MultipleVariables(mv), TargetArchetypeKind::Column) => convert_mv_to_column(mv)?,
        (TableArchetype::Grouped(grp), TargetArchetypeKind::Xy) => convert_grouped_to_xy(grp)?,
        (TableArchetype::Xy(xy), TargetArchetypeKind::Grouped) => convert_xy_to_grouped(xy)?,
        (TableArchetype::Column(col), TargetArchetypeKind::Contingency) => convert_column_to_contingency(col)?,
        (TableArchetype::Grouped(grp), TargetArchetypeKind::Contingency) => convert_grouped_to_contingency(grp)?,
        (TableArchetype::Xy(xy), TargetArchetypeKind::Contingency) => convert_xy_to_contingency(xy)?,
        (TableArchetype::MultipleVariables(mv), TargetArchetypeKind::Contingency) => convert_mv_to_contingency(mv)?,
        _ => return Err(TransformError::IncompatibleTable { operation: "convert_datasheet_archetype", kind: source_kind }),
    };

    assemble_datasheet(sheet, target, built)
}

fn build_identity_result(sheet: &DataSheet, target: TargetArchetypeKind) -> (DataSheet, ConversionPreview) {
    let preview = ConversionPreview {
        source_kind: sheet.kind(),
        target_kind: target,
        preserved_cells: count_archetype_cells(&sheet.archetype, is_occupied),
        dropped_cells: 0,
        preserved_exclusions: count_archetype_cells(&sheet.archetype, |c| c.excluded),
        warnings: Vec::new(),
        lossy: false,
    };
    (sheet.clone(), preview)
}

fn assemble_datasheet(
    source: &DataSheet,
    target: TargetArchetypeKind,
    built: BuiltArchetype,
) -> Result<(DataSheet, ConversionPreview), TransformError> {
    let mut acc = built.acc;
    let mut out = DataSheet::new(source.id.clone(), source.title.clone(), built.archetype)?;
    out.frozen = source.frozen;
    out.info_sheet_id = source.info_sheet_id.clone();
    out.floating_notes = source.floating_notes.clone();
    out.color_tag = source.color_tag;

    let mut dropped_notes = 0_usize;
    for note in &source.exclusion_notes {
        if let Some(&new_coord) = acc.coord_map.get(&note.coord) {
            out.exclusion_notes.push(CellExclusionNote {
                coord: new_coord,
                reason_tag: note.reason_tag.clone(),
                note: note.note.clone(),
                timestamp_utc: note.timestamp_utc.clone(),
            });
        } else {
            dropped_notes += 1;
            acc.warn(format!(
                "Exclusion note at (row={}, col={}, subcol={}) could not be remapped to target data cells",
                note.coord.row, note.coord.col, note.coord.subcol
            ));
        }
    }
    out.exclusion_notes.sort_by_key(|n| n.coord);
    if !source.linked_regions.is_empty() {
        acc.warn(format!(
            "{} live linked data region(s) are not automatically remapped by archetype conversion and must be re-attached or detached by the caller",
            source.linked_regions.len()
        ));
    }

    for origin in &source.plate_well_origins {
        if let Some(&new_coord) = acc.coord_map.get(&origin.coord) {
            out.plate_well_origins.push(PlateCellOrigin { coord: new_coord, well: origin.well.clone() });
        }
    }
    out.plate_well_origins.sort_by_key(|o| o.coord);
    out.plate_well_origins.dedup_by_key(|o| o.coord);
    out.prune_stale_plate_well_origins();

    let preserved_exclusions = count_archetype_cells(&out.archetype, |c| c.excluded);
    let lossy = acc.dropped > 0 || dropped_notes > 0 || acc.schema_lossy;
    let preview = ConversionPreview {
        source_kind: source.kind(),
        target_kind: target,
        preserved_cells: acc.preserved,
        dropped_cells: acc.dropped,
        preserved_exclusions,
        warnings: acc.warnings,
        lossy,
    };
    Ok((out, preview))
}

fn is_occupied(cell: &Cell) -> bool {
    !cell.is_empty() || cell.excluded
}

fn count_archetype_cells(arch: &TableArchetype, pred: impl Fn(&Cell) -> bool) -> usize {
    let count_grid = |grid: &[Vec<Cell>]| -> usize { grid.iter().map(|row| row.iter().filter(|c| pred(c)).count()).sum() };
    let count_ds = |cols: &[DataSetColumn]| -> usize { cols.iter().map(|ds| count_grid(&ds.cells)).sum() };
    match arch {
        TableArchetype::Xy(t) => count_ds(&t.data_sets) + count_grid(&t.x_cells),
        TableArchetype::Column(t) => count_ds(&t.data_sets),
        TableArchetype::Grouped(t) => count_ds(&t.data_sets),
        TableArchetype::Contingency(t) => count_ds(&t.data_sets),
        TableArchetype::Survival(t) => t.times.iter().filter(|c| pred(c)).count() + count_ds(&t.cohorts),
        TableArchetype::PartsOfWhole(t) => count_ds(&t.data_sets),
        TableArchetype::MultipleVariables(mv) => mv.variables.iter().map(|v| v.cells.iter().filter(|c| pred(c)).count()).sum(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| count_grid(&g.cells)).sum(),
    }
}

fn column_row_count(cols: &[DataSetColumn], row_titles_len: usize) -> usize {
    cols.iter().map(DataSetColumn::row_count).max().unwrap_or(row_titles_len).max(row_titles_len)
}

fn has_non_empty_title(titles: &[String]) -> bool {
    titles.iter().any(|s| !s.trim().is_empty())
}

fn require_non_empty_datasets(data_sets: &[DataSetColumn], msg: &str) -> Result<(), TransformError> {
    if data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: msg.to_owned() });
    }
    Ok(())
}

// ============================================================================
// Column <-> Grouped
// ============================================================================

fn convert_column_to_grouped(col: &ColumnTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&col.data_sets, "Column table has no data columns")?;
    let rows = column_row_count(&col.data_sets, col.row_titles.len());
    let target_mode = if col.subcolumn_mode == SubcolumnMode::Single { SubcolumnMode::RawReplicates(1) } else { col.subcolumn_mode };
    let mut grouped = GroupedTable::new(target_mode, col.data_sets.len(), rows)?;
    grouped.row_titles = resized_titles(&col.row_titles, rows);

    let mut acc = ConversionAcc::default();
    DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_direct(&col.data_sets, &mut grouped.data_sets, 0, 0, target_mode.subcolumn_count());
    if col.design_hint == ColumnDesignHint::UnpairedReplicates {
        acc.warn("Unpaired Column rows mapped 1:1 to Grouped row levels");
    }
    Ok(acc.finish(TableArchetype::Grouped(grouped)))
}

fn warn_grouped_metadata_loss(grp: &GroupedTable, include_row_spacing: bool, target_label: &str, acc: &mut ConversionAcc) {
    let loses_three_way = grp.three_way_grouping.is_some();
    let loses_spacing = include_row_spacing && grp.row_numeric_spacing.is_some();
    match (loses_three_way, loses_spacing) {
        (true, true) => acc.warn_lossy(format!(
            "Grouped three-way column grouping and numeric row spacing cannot be represented in {target_label} tables and will be discarded"
        )),
        (true, false) => {
            acc.warn_lossy(format!("Grouped three-way column grouping cannot be represented in {target_label} tables and will be discarded"))
        }
        (false, true) => acc.warn_lossy(format!("Grouped numeric row spacing cannot be represented in {target_label} tables and will be discarded")),
        (false, false) => {}
    }
}

fn convert_grouped_to_column(grp: &GroupedTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&grp.data_sets, "Grouped table has no data columns")?;
    let cols = grp.data_sets.len();
    let rows = column_row_count(&grp.data_sets, grp.row_titles.len());

    match grp.subcolumn_mode {
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(1) => {
            let hint =
                if has_non_empty_title(&grp.row_titles) { ColumnDesignHint::PairedRepeatedMeasures } else { ColumnDesignHint::UnpairedReplicates };
            let mut acc = ConversionAcc::default();
            let out = copy_grouped_direct_to_column(grp, hint, SubcolumnMode::Single, rows, &mut acc)?;
            warn_grouped_metadata_loss(grp, true, "Column", &mut acc);
            Ok(acc.finish(TableArchetype::Column(out)))
        }
        SubcolumnMode::RawReplicates(n) => convert_grouped_multi_rep_to_column(grp, cols, rows, n),
        SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN => {
            let mut acc = ConversionAcc::default();
            let out = copy_grouped_direct_to_column(grp, ColumnDesignHint::PrecomputedError, grp.subcolumn_mode, rows, &mut acc)?;
            warn_grouped_metadata_loss(grp, true, "Column", &mut acc);
            Ok(acc.finish(TableArchetype::Column(out)))
        }
        SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => build_grouped_truncated_column(
            grp,
            rows,
            format!("Column tables do not support {:?} subcolumns; spread/limit subcolumns 1..3 will be dropped", grp.subcolumn_mode),
        ),
    }
}

fn copy_grouped_direct_to_column(
    grp: &GroupedTable,
    hint: ColumnDesignHint,
    mode: SubcolumnMode,
    rows: usize,
    acc: &mut ConversionAcc,
) -> Result<ColumnTable, TransformError> {
    let mut out = ColumnTable::new_with_subcolumns(hint, mode, grp.data_sets.len(), rows)?;
    out.row_titles = resized_titles(&grp.row_titles, rows);
    out.row_titles_expanded = has_non_empty_title(&out.row_titles);
    DataSetSubcolumnUnpacker::new(acc, rows).copy_direct(&grp.data_sets, &mut out.data_sets, 0, 0, mode.subcolumn_count());
    Ok(out)
}

fn build_grouped_truncated_column(grp: &GroupedTable, rows: usize, warning: String) -> Result<BuiltArchetype, TransformError> {
    let mut out = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, grp.data_sets.len(), rows)?;
    out.row_titles = resized_titles(&grp.row_titles, rows);
    let mut acc = ConversionAcc::default();
    DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_primary_only(&grp.data_sets, &mut out.data_sets, 0);
    acc.warn_lossy(warning);
    warn_grouped_metadata_loss(grp, true, "Column", &mut acc);
    Ok(acc.finish(TableArchetype::Column(out)))
}

fn convert_grouped_multi_rep_to_column(grp: &GroupedTable, cols: usize, rows: usize, n: usize) -> Result<BuiltArchetype, TransformError> {
    let mut acc = ConversionAcc::default();
    if rows <= 1 {
        let mut out = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, cols, n)?;
        for (c, src_ds) in grp.data_sets.iter().enumerate() {
            out.data_sets[c].title = src_ds.title.clone();
            if let Some(row_cells) = src_ds.cells.first() {
                stack_replicates_into_column(&mut out.data_sets, row_cells, c, (0, c), n, &mut acc);
            }
        }
        acc.warn("Grouped replicate subcolumns stacked vertically down Column rows");
        warn_grouped_metadata_loss(grp, true, "Column", &mut acc);
        return Ok(acc.finish(TableArchetype::Column(out)));
    }

    if let Some(total_cols) = cols.checked_mul(rows)
        && total_cols <= MAX_DATA_SETS
    {
        let mut expanded_titles = Vec::with_capacity(total_cols);
        for src_ds in &grp.data_sets {
            for r in 0..rows {
                let row_lbl = grp.row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty());
                let title = match row_lbl {
                    Some(lbl) => format!("{} - {lbl}", src_ds.title),
                    None => format!("{} - Row {}", src_ds.title, r + 1),
                };
                validate_title(&title)?;
                expanded_titles.push(title);
            }
        }
        let mut out = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, total_cols, n)?;
        for (c, src_ds) in grp.data_sets.iter().enumerate() {
            for r in 0..rows {
                let dst_col = c * rows + r;
                if let Some(title) = expanded_titles.get(dst_col) {
                    out.data_sets[dst_col].title = title.clone();
                }
                if let Some(row_cells) = src_ds.cells.get(r) {
                    stack_replicates_into_column(&mut out.data_sets, row_cells, dst_col, (r, c), n, &mut acc);
                }
            }
        }
        acc.warn("Grouped (Row x Column) cells expanded into Column groups with stacked replicates");
        warn_grouped_metadata_loss(grp, true, "Column", &mut acc);
        return Ok(acc.finish(TableArchetype::Column(out)));
    }

    // Exceeds MAX_DATA_SETS if expanded; fall back to keeping subcolumn 0 only (lossy).
    build_grouped_truncated_column(grp, rows, format!("Expanded column count exceeds {MAX_DATA_SETS}; truncated replicate subcolumns beyond Y1"))
}

fn stack_replicates_into_column(
    dst: &mut [DataSetColumn],
    row_cells: &[Cell],
    dst_col: usize,
    src_rc: (usize, usize),
    n: usize,
    acc: &mut ConversionAcc,
) {
    let (src_r, src_c) = src_rc;
    for s in 0..n {
        if let Some(cell) = row_cells.get(s) {
            set_dataset_cell(dst, dst_col, s, 0, cell.clone());
            acc.map_coord(CellCoord::new(src_r, src_c, s), CellCoord::new(s, dst_col, 0));
            acc.record_preserved(cell);
        }
    }
}

// ============================================================================
// Column <-> XY
// ============================================================================

fn convert_column_to_xy(col: &ColumnTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&col.data_sets, "Column table has no data columns")?;
    let cols = col.data_sets.len();
    let rows = column_row_count(&col.data_sets, col.row_titles.len());
    let subs = col.subcolumn_mode.subcolumn_count();
    let mut acc = ConversionAcc::default();

    if cols == 1 {
        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, col.subcolumn_mode, 1, rows)?;
        xy.row_titles = resized_titles(&col.row_titles, rows);
        xy.row_titles_expanded = col.row_titles_expanded;
        populate_generated_or_titled_x(&mut xy, &col.row_titles, rows);
        DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_direct(&col.data_sets[..1], &mut xy.data_sets, 0, 0, subs);
        acc.warn("Single Column series mapped to Y with sequential/parsed X coordinates");
        return Ok(acc.finish(TableArchetype::Xy(xy)));
    }

    let x_mode = XColumnMode::Numbers { subcolumn_mode: col.subcolumn_mode };
    let mut xy = XyTable::new_with_x_mode(x_mode, col.subcolumn_mode, cols - 1, rows)?;
    xy.x_title = col.data_sets[0].title.clone();
    xy.row_titles = resized_titles(&col.row_titles, rows);
    xy.row_titles_expanded = col.row_titles_expanded;

    let mut text_in_x = 0_usize;
    for (r, row_cells) in col.data_sets[0].cells.iter().enumerate().take(rows) {
        for s in 0..subs {
            if let Some(cell) = row_cells.get(s) {
                match &cell.value {
                    CellValue::Text(txt) if !txt.trim().is_empty() => {
                        text_in_x += 1;
                        acc.dropped += 1;
                        set_x_cell(&mut xy, r, s, Cell { value: CellValue::Empty, excluded: cell.excluded });
                    }
                    _ => {
                        set_x_cell(&mut xy, r, s, cell.clone());
                        acc.record_preserved(cell);
                    }
                }
            }
        }
    }
    if text_in_x > 0 {
        acc.warn_lossy(format!("Column 0 contains {text_in_x} non-numeric text cell(s) incompatible with numeric XY X coordinates"));
    }

    DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_direct(&col.data_sets[1..], &mut xy.data_sets, 1, 0, subs);
    Ok(acc.finish(TableArchetype::Xy(xy)))
}

fn populate_generated_or_titled_x(xy: &mut XyTable, row_titles: &[String], rows: usize) {
    let parsed: Vec<Option<f64>> =
        (0..rows).map(|r| row_titles.get(r).and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite())).collect();
    let all_numeric = has_non_empty_title(row_titles) && row_titles.iter().enumerate().all(|(r, s)| s.trim().is_empty() || parsed[r].is_some());
    for (r, row_x) in xy.x_cells.iter_mut().enumerate().take(rows) {
        if let Some(slot) = row_x.first_mut() {
            if all_numeric {
                *slot = parsed[r].map_or_else(Cell::empty, Cell::number);
            } else {
                #[allow(clippy::cast_precision_loss)]
                {
                    *slot = Cell::number((r + 1) as f64);
                }
            }
        }
    }
}

fn convert_xy_to_column(xy: &XyTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&xy.data_sets, "XY table has no Y data columns")?;
    let rows = xy_row_count(xy);
    let y_cols = xy.data_sets.len();

    if matches!(xy.y_subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr) {
        return convert_xy_lossy_single_column(xy, rows, y_cols);
    }
    if let SubcolumnMode::RawReplicates(n) = xy.y_subcolumn_mode
        && n >= 2
    {
        return convert_xy_replicates_to_column(xy, rows, y_cols, n);
    }
    if xy.y_subcolumn_mode.supports_parametric_moments() {
        return convert_xy_parametric_to_column(xy, rows, y_cols);
    }

    let mut acc = ConversionAcc { schema_lossy: xy.x_mode.subcolumn_count() > 1, ..ConversionAcc::default() };
    let mut col = init_xy_to_column_table(xy, 1 + y_cols, rows, &mut acc)?;
    DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_direct(&xy.data_sets, &mut col.data_sets, 0, 1, 1);
    Ok(acc.finish(TableArchetype::Column(col)))
}

fn init_xy_to_column_table(xy: &XyTable, total_cols: usize, rows: usize, acc: &mut ConversionAcc) -> Result<ColumnTable, TransformError> {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PairedRepeatedMeasures, SubcolumnMode::Single, total_cols, rows)?;
    col.row_titles = resized_titles(&xy.row_titles, rows);
    col.row_titles_expanded = xy.row_titles_expanded;
    col.data_sets[0].title = xy.x_title.clone();
    copy_xy_x_into_column0(xy, &mut col, rows, acc);
    Ok(col)
}

fn convert_xy_replicates_to_column(xy: &XyTable, rows: usize, y_cols: usize, n: usize) -> Result<BuiltArchetype, TransformError> {
    let needed = y_cols.checked_mul(n).and_then(|v| v.checked_add(1));
    if needed.is_none_or(|tot| tot > MAX_DATA_SETS) {
        return convert_xy_lossy_single_column(xy, rows, y_cols);
    }
    let mut expanded_titles = Vec::with_capacity(y_cols * n);
    for src_ds in &xy.data_sets {
        for s in 0..n {
            let title = format!("{} (Y{})", src_ds.title, s + 1);
            validate_title(&title)?;
            expanded_titles.push(title);
        }
    }

    let mut acc = ConversionAcc { schema_lossy: xy.x_mode.subcolumn_count() > 1, ..ConversionAcc::default() };
    acc.warn("XY replicate subcolumns expanded into individual Column columns");
    let mut col = init_xy_to_column_table(xy, 1 + y_cols * n, rows, &mut acc)?;

    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        for s in 0..n {
            let title_idx = c * n + s;
            let dst_c = 1 + title_idx;
            if let Some(title) = expanded_titles.get(title_idx) {
                col.data_sets[dst_c].title = title.clone();
            }
            for (r, row_cells) in src_ds.cells.iter().enumerate().take(rows) {
                if let Some(cell) = row_cells.get(s) {
                    set_dataset_cell(&mut col.data_sets, dst_c, r, 0, cell.clone());
                    acc.map_coord(CellCoord::new(r, c, s), CellCoord::new(r, dst_c, 0));
                    acc.record_preserved(cell);
                }
            }
        }
    }

    Ok(acc.finish(TableArchetype::Column(col)))
}

fn convert_xy_parametric_to_column(xy: &XyTable, rows: usize, y_cols: usize) -> Result<BuiltArchetype, TransformError> {
    let y_subs = xy.y_subcolumn_mode.subcolumn_count();
    let mut acc = ConversionAcc::default();

    if let XColumnMode::Numbers { subcolumn_mode } = &xy.x_mode
        && *subcolumn_mode == xy.y_subcolumn_mode
    {
        let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, xy.y_subcolumn_mode, 1 + y_cols, rows)?;
        col.row_titles = resized_titles(&xy.row_titles, rows);
        col.row_titles_expanded = xy.row_titles_expanded;
        col.data_sets[0].title = xy.x_title.clone();

        for (r, x_row) in xy.x_cells.iter().enumerate().take(rows) {
            for s in 0..y_subs {
                if let Some(cell) = x_row.get(s) {
                    set_dataset_cell(&mut col.data_sets, 0, r, s, cell.clone());
                    acc.record_preserved(cell);
                }
            }
        }
        DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_direct(&xy.data_sets, &mut col.data_sets, 0, 1, y_subs);
        return Ok(acc.finish(TableArchetype::Column(col)));
    }

    // X does not carry matching (Mean, Spread, N) subcolumns: place only Y arms in PrecomputedError
    // columns and promote active X coordinates into row_titles so column analyses never fail with
    // MissingSpread on a half-populated X column.
    let xy_rows = build_xy_row_titles(xy, rows)?;
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        let excluded_gen_x = count_excluded_generated_x(xy, rows);
        if excluded_gen_x > 0 {
            acc.drop_cells(excluded_gen_x);
            acc.warn_lossy(format!("Dropped {excluded_gen_x} excluded generated X value(s) unsupported by Column row titles"));
        }
    } else {
        for (r, x_val) in xy_rows.spacing.iter().enumerate() {
            let has_explicit = xy.row_titles.get(r).is_some_and(|s| !s.trim().is_empty());
            let skip_first = !has_explicit && x_val.is_some();
            if let Some(row) = xy.x_cells.get(r) {
                acc.dropped += row.iter().skip(usize::from(skip_first)).filter(|c| is_occupied(c)).count();
            }
        }
    }

    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, xy.y_subcolumn_mode, y_cols, rows)?;
    col.row_titles_expanded = has_non_empty_title(&xy_rows.titles);
    col.row_titles = xy_rows.titles;
    DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_direct(&xy.data_sets, &mut col.data_sets, 0, 0, y_subs);
    acc.warn_lossy("XY X values moved to Column row titles (numeric X spacing dropped to preserve valid PrecomputedError Y columns)");
    Ok(acc.finish(TableArchetype::Column(col)))
}

fn convert_xy_lossy_single_column(xy: &XyTable, rows: usize, y_cols: usize) -> Result<BuiltArchetype, TransformError> {
    let mut acc = ConversionAcc::default();
    acc.warn_lossy(format!(
        "Column tables do not support {:?} Y subcolumns; spread/limit subcolumns beyond subcolumn 0 will be dropped",
        xy.y_subcolumn_mode
    ));
    let mut col = init_xy_to_column_table(xy, 1 + y_cols, rows, &mut acc)?;
    DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_primary_only(&xy.data_sets, &mut col.data_sets, 1);
    Ok(acc.finish(TableArchetype::Column(col)))
}

fn copy_xy_x_into_column0(xy: &XyTable, col: &mut ColumnTable, rows: usize, acc: &mut ConversionAcc) {
    match &xy.x_mode {
        XColumnMode::GeneratedSequence { .. } => {
            for r in 0..rows {
                set_dataset_cell(&mut col.data_sets, 0, r, 0, xy_materialized_x_cell(xy, r));
            }
            acc.warn("Generated X sequence materialized into static Column 0 values");
        }
        XColumnMode::DateOrElapsedTime { .. } | XColumnMode::Numbers { .. } => {
            let mut dropped_x_subs = 0_usize;
            for (r, x_row) in xy.x_cells.iter().enumerate().take(rows) {
                for (s, cell) in x_row.iter().enumerate() {
                    if s == 0 {
                        set_dataset_cell(&mut col.data_sets, 0, r, 0, cell.clone());
                        acc.record_preserved(cell);
                    } else if is_occupied(cell) {
                        acc.dropped += 1;
                        dropped_x_subs += 1;
                    }
                }
            }
            if dropped_x_subs > 0 {
                acc.warn(format!("Dropped {dropped_x_subs} X error subcolumn cell(s) unsupported by Column tables"));
            }
        }
    }
}

// ============================================================================
// Grouped <-> MultipleVariables
// ============================================================================

fn convert_grouped_to_mv(grp: &GroupedTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&grp.data_sets, "Grouped table has no data columns")?;
    let rows = column_row_count(&grp.data_sets, grp.row_titles.len());
    let row_title = non_empty_or(&grp.row_factor_title, "Row Factor");
    let col_title = non_empty_or(&grp.column_factor_title, "Column Factor");

    match grp.subcolumn_mode {
        SubcolumnMode::RawReplicates(n) => convert_grouped_reps_to_mv(grp, rows, n, row_title, col_title),
        _ => convert_grouped_summary_to_mv(grp, rows, row_title, col_title),
    }
}

fn init_grouped_factor_mv(
    out_rows: usize,
    row_title: String,
    col_title: String,
    extra_titles: Vec<String>,
    budget_context: &'static str,
) -> Result<MultipleVariablesTable, TransformError> {
    let var_count = 2 + extra_titles.len();
    validate_rows(out_rows)?;
    validate_cell_budget(out_rows, var_count, 1, 0, budget_context)?;
    let mut mv = MultipleVariablesTable::new(var_count, out_rows)?;
    mv.variables[0].title = row_title;
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = col_title;
    mv.variables[1].lock_role(VariableRole::Categorical);
    for (idx, title) in extra_titles.into_iter().enumerate() {
        mv.variables[2 + idx].title = title;
    }
    Ok(mv)
}

fn finalize_grouped_mv(mv: &mut MultipleVariablesTable) {
    mv.row_titles_expanded = has_non_empty_title(&mv.row_titles);
    for var in mv.variables.iter_mut().skip(2) {
        var.refresh_inferred_role();
    }
}

fn convert_grouped_reps_to_mv(
    grp: &GroupedTable,
    rows: usize,
    n: usize,
    row_title: String,
    col_title: String,
) -> Result<BuiltArchetype, TransformError> {
    let mut out_rows = 0_usize;
    for r in 0..rows {
        for ds in &grp.data_sets {
            if let Some(row_cells) = ds.cells.get(r)
                && row_cells.iter().take(n).any(is_occupied)
            {
                let add = if n == 1 { usize::from(row_cells.first().is_some_and(is_occupied)) } else { n };
                out_rows = out_rows
                    .checked_add(add)
                    .ok_or_else(|| TransformError::InvalidSpec { message: String::from("Grouped-to-MultipleVariables row count overflows") })?;
                if out_rows > MAX_ROWS {
                    validate_rows(out_rows)?;
                }
            }
        }
    }

    let extra_titles = if n > 1 { vec![String::from("Replicate"), String::from("Value")] } else { vec![String::from("Value")] };
    let val_col_idx = 1 + extra_titles.len();
    let mut mv = init_grouped_factor_mv(out_rows, row_title, col_title, extra_titles, "Grouped-to-MultipleVariables replicate reshape")?;
    let mut acc = ConversionAcc::default();
    let col_labels: Vec<String> = grp.data_sets.iter().enumerate().map(|(c, ds)| non_empty_or(&ds.title, &default_column_label(c))).collect();

    let mut out_r = 0_usize;
    for r in 0..rows {
        let row_cell = grouped_row_factor_cell(grp, r);
        for (c, ds) in grp.data_sets.iter().enumerate() {
            if let Some(row_cells) = ds.cells.get(r)
                && row_cells.iter().take(n).any(is_occupied)
            {
                let col_cell = Cell::text(col_labels[c].clone());
                for s in 0..n {
                    let val = row_cells.get(s).cloned().unwrap_or_default();
                    if n == 1 && !is_occupied(&val) {
                        continue;
                    }
                    if let Some(title) = grp.row_titles.get(r) {
                        mv.row_titles[out_r].clone_from(title);
                    }
                    set_mv_cell(&mut mv, 0, out_r, row_cell.clone());
                    set_mv_cell(&mut mv, 1, out_r, col_cell.clone());
                    if n > 1 {
                        #[allow(clippy::cast_precision_loss)]
                        set_mv_cell(&mut mv, 2, out_r, Cell::number((s + 1) as f64));
                    }
                    acc.record_preserved(&val);
                    set_mv_cell(&mut mv, val_col_idx, out_r, val);
                    acc.map_coord(CellCoord::new(r, c, s), CellCoord::new(out_r, val_col_idx, 0));
                    out_r += 1;
                }
            }
        }
    }
    finalize_grouped_mv(&mut mv);
    warn_grouped_metadata_loss(grp, false, "MultipleVariables", &mut acc);
    Ok(acc.finish(TableArchetype::MultipleVariables(mv)))
}

fn convert_grouped_summary_to_mv(grp: &GroupedTable, rows: usize, row_title: String, col_title: String) -> Result<BuiltArchetype, TransformError> {
    let sub_labels = grp.subcolumn_mode.subcolumn_labels();
    let sub_count = sub_labels.len();
    let mut out_rows = 0_usize;
    for r in 0..rows {
        for ds in &grp.data_sets {
            if let Some(row_cells) = ds.cells.get(r)
                && row_cells.iter().take(sub_count).any(is_occupied)
            {
                out_rows = out_rows
                    .checked_add(1)
                    .ok_or_else(|| TransformError::InvalidSpec { message: String::from("Grouped-to-MultipleVariables row count overflows") })?;
                if out_rows > MAX_ROWS {
                    validate_rows(out_rows)?;
                }
            }
        }
    }

    let mut mv = init_grouped_factor_mv(out_rows, row_title, col_title, sub_labels, "Grouped-to-MultipleVariables summary reshape")?;
    let mut acc = ConversionAcc::default();
    let col_labels: Vec<String> = grp.data_sets.iter().enumerate().map(|(c, ds)| non_empty_or(&ds.title, &default_column_label(c))).collect();

    let mut out_r = 0_usize;
    for r in 0..rows {
        let row_cell = grouped_row_factor_cell(grp, r);
        for (c, ds) in grp.data_sets.iter().enumerate() {
            let Some(row_cells) = ds.cells.get(r) else { continue };
            if row_cells.iter().take(sub_count).any(is_occupied) {
                if let Some(title) = grp.row_titles.get(r) {
                    mv.row_titles[out_r].clone_from(title);
                }
                set_mv_cell(&mut mv, 0, out_r, row_cell.clone());
                set_mv_cell(&mut mv, 1, out_r, Cell::text(col_labels[c].clone()));
                for s in 0..sub_count {
                    let val = row_cells.get(s).cloned().unwrap_or_default();
                    acc.record_preserved(&val);
                    set_mv_cell(&mut mv, 2 + s, out_r, val);
                    acc.map_coord(CellCoord::new(r, c, s), CellCoord::new(out_r, 2 + s, 0));
                }
                out_r += 1;
            }
        }
    }
    finalize_grouped_mv(&mut mv);

    if grp.subcolumn_mode.is_pre_summarized() {
        acc.warn_lossy(format!(
            "Pre-summarized {:?} subcolumns flattened into independent Continuous variables; summary moment semantics lost",
            grp.subcolumn_mode
        ));
    }
    warn_grouped_metadata_loss(grp, false, "MultipleVariables", &mut acc);
    Ok(acc.finish(TableArchetype::MultipleVariables(mv)))
}

fn grouped_row_factor_cell(grp: &GroupedTable, r: usize) -> Cell {
    if let Some(Some(num)) = grp.row_numeric_spacing.as_ref().and_then(|v| v.get(r))
        && num.is_finite()
    {
        return Cell::number(*num);
    }
    match grp.row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(s) => Cell::text(s.to_owned()),
        None => Cell::text(format!("Row {}", r + 1)),
    }
}

fn convert_mv_to_grouped(mv: &MultipleVariablesTable) -> Result<BuiltArchetype, TransformError> {
    let part = MvPartition::from_table(mv)?;
    if part.cat_cols.len() >= 2 && !part.num_cols.is_empty() {
        return pivot_mv_two_way_to_grouped(mv, &part);
    }
    if part.cat_cols.len() == 1 && !part.num_cols.is_empty() {
        return pivot_mv_one_factor_to_grouped(mv, &part);
    }

    // No categorical grouping columns (or only categorical/label columns): direct column projection.
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(1), mv.variables.len(), part.rows)?;
    grouped.row_titles = resized_titles(&mv.row_titles, part.rows);
    let mut acc = ConversionAcc::default();
    let all_cols: Vec<usize> = (0..mv.variables.len()).collect();
    DataSetSubcolumnUnpacker::new(&mut acc, part.rows).copy_mv_vars_to_datasets(mv, &all_cols, &mut grouped.data_sets);
    Ok(acc.finish(TableArchetype::Grouped(grouped)))
}

fn build_grouped_shell(
    mode: SubcolumnMode,
    row_levels: Vec<String>,
    col_titles: &[String],
    factor_titles: (&str, Option<&str>),
    budget_context: &'static str,
) -> Result<GroupedTable, TransformError> {
    let n_rows = row_levels.len();
    let n_cols = col_titles.len();
    validate_data_sets(n_cols)?;
    validate_rows(n_rows)?;
    validate_subcolumns(mode.subcolumn_count())?;
    validate_title_slice(&row_levels)?;
    validate_title_slice(col_titles)?;
    validate_cell_budget(n_rows, n_cols, mode.subcolumn_count(), 0, budget_context)?;

    let mut grouped = GroupedTable::new(mode, n_cols, n_rows)?;
    grouped.row_factor_title = factor_titles.0.to_owned();
    if let Some(cf) = factor_titles.1 {
        grouped.column_factor_title = cf.to_owned();
    }
    grouped.row_numeric_spacing = parse_numeric_row_spacing(&row_levels);
    grouped.row_titles = row_levels;
    for (ci, title) in col_titles.iter().enumerate() {
        grouped.data_sets[ci].title = title.clone();
    }
    Ok(grouped)
}

fn pivot_mv_two_way_to_grouped(mv: &MultipleVariablesTable, part: &MvPartition) -> Result<BuiltArchetype, TransformError> {
    let row_fac_col = part.cat_cols[0];
    let col_fac_col = part.cat_cols[1];

    if let Some((mode, summary_cols)) = detect_mv_summary_triple(mv, &part.num_cols) {
        return pivot_mv_summary_to_grouped(mv, (row_fac_col, col_fac_col), &summary_cols, mode, part);
    }

    let (rep_col, val_col, extra_num_cols) = split_replicate_and_value(mv, &part.num_cols);
    let mut acc = ConversionAcc::default();
    let extra_cols = part.cat_cols.iter().skip(2).chain(part.label_cols.iter()).chain(extra_num_cols.iter()).copied();
    if acc.drop_mv_columns(mv, extra_cols, part.rows) {
        acc.warn("MultipleVariables contains more than two factor columns or extra response columns that cannot fit in a 2-way Grouped table");
    }
    if matches!(mv.variables[val_col].source, VariableSource::Calculated { .. }) {
        acc.warn(format!("Calculated variable `{}` evaluated/frozen to static cells", mv.variables[val_col].title));
    }

    let mut row_levels = LevelInterner::default();
    let mut col_levels = LevelInterner::default();
    let mut buckets: RepBucketMap<Cell> = BTreeMap::new();
    let mut warned_rep_dropped = false;
    let mut skipped_factor_cells = 0_usize;

    for r in 0..part.rows {
        let r_tok = mv_cell_level_token(mv.variables[row_fac_col].cells.get(r));
        let c_tok = mv_cell_level_token(mv.variables[col_fac_col].cells.get(r));
        let val_cell = mv.variables[val_col].cells.get(r).cloned().unwrap_or_default();
        let (Some(rk), Some(ck)) = (r_tok, c_tok) else {
            let skipped = count_mv_row_occupied(mv, [row_fac_col, col_fac_col, val_col].into_iter().chain(rep_col), r);
            acc.drop_cells(skipped);
            skipped_factor_cells += skipped;
            continue;
        };
        let ri = row_levels.intern(rk);
        let ci = col_levels.intern(ck);
        acc.preserved += 2;
        let rep_order = extract_replicate_order(mv, rep_col, r, &mut acc, &mut warned_rep_dropped);
        buckets.entry((ri, ci)).or_default().push((rep_order, r, val_cell));
    }
    if skipped_factor_cells > 0 {
        acc.warn(format!("Dropped {skipped_factor_cells} cell(s) from rows with missing or excluded factor levels"));
    }

    if col_levels.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no factor levels found to build Grouped table") });
    }
    let (assigned_buckets, max_reps, rep_collisions) = slot_buckets(buckets, rep_col.is_some())?;
    if rep_collisions > 0 {
        acc.warn(format!("{rep_collisions} duplicate replicate slot collision(s) shifted to subsequent subcolumns"));
    }
    let col_titles = col_levels.into_levels();
    let mut grouped = build_grouped_shell(
        SubcolumnMode::RawReplicates(max_reps),
        row_levels.into_levels(),
        &col_titles,
        (&mv.variables[row_fac_col].title, Some(&mv.variables[col_fac_col].title)),
        "MultipleVariables Grouped pivot",
    )?;

    for ((ri, ci), assigned) in assigned_buckets {
        for (s, old_r, cell) in assigned {
            acc.record_preserved(&cell);
            set_dataset_cell(&mut grouped.data_sets, ci, ri, s, cell);
            acc.map_coord(CellCoord::new(old_r, val_col, 0), CellCoord::new(ri, ci, s));
        }
    }
    Ok(acc.finish(TableArchetype::Grouped(grouped)))
}

struct ReplicateSlotMapper {
    one_based_int: bool,
    zero_based_int: bool,
    rank_by_bits: BTreeMap<u64, usize>,
}

impl ReplicateSlotMapper {
    fn from_values<I: IntoIterator<Item = f64>>(vals: I) -> Self {
        let mut sorted: Vec<f64> = vals.into_iter().filter(|v| v.is_finite()).map(normalize_zero).collect();
        sorted.sort_by(f64::total_cmp);
        sorted.dedup_by(|a, b| a.to_bits() == b.to_bits());
        let all_int = sorted.iter().all(|&v| v.fract() == 0.0 && v >= 0.0 && v <= (MAX_SUBCOLUMNS as f64));
        let one_based_int = !sorted.is_empty() && all_int && sorted.iter().all(|&v| v >= 1.0);
        let zero_based_int = !one_based_int && !sorted.is_empty() && all_int && sorted.iter().all(|&v| v < (MAX_SUBCOLUMNS as f64));
        let rank_by_bits = sorted.into_iter().enumerate().map(|(idx, v)| (v.to_bits(), idx)).collect();
        Self { one_based_int, zero_based_int, rank_by_bits }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn slot_for(&self, v: f64) -> Option<usize> {
        if !v.is_finite() {
            return None;
        }
        let norm = normalize_zero(v);
        if self.one_based_int {
            return Some((norm as usize).saturating_sub(1));
        }
        if self.zero_based_int {
            return Some(norm as usize);
        }
        self.rank_by_bits.get(&norm.to_bits()).copied()
    }
}

type SlottedBucketResult<T> = Result<(SlottedBucketMap<T>, usize, usize), TransformError>;
type AssignedBucketSlots<T> = Result<(Vec<(usize, usize, T)>, usize), TransformError>;

fn assign_bucket_slots<T>(mut entries: Vec<(Option<f64>, usize, T)>, mapper: Option<&ReplicateSlotMapper>) -> AssignedBucketSlots<T> {
    entries.sort_by(|a, b| match (a.0, b.0) {
        (Some(ra), Some(rb)) => ra.total_cmp(&rb).then_with(|| a.1.cmp(&b.1)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.cmp(&b.1),
    });
    let mut used = BTreeSet::new();
    let mut placed = Vec::with_capacity(entries.len());
    let mut colliding = Vec::new();
    let mut unkeyed = Vec::new();
    for (rep_opt, old_row, item) in entries {
        if let (Some(rep), Some(m)) = (rep_opt, mapper)
            && let Some(slot) = m.slot_for(rep)
        {
            if slot >= MAX_SUBCOLUMNS {
                return Err(DocError::TooManySubcolumns { count: slot + 1, max: MAX_SUBCOLUMNS }.into());
            }
            if used.insert(slot) {
                placed.push((slot, old_row, item));
            } else {
                colliding.push((slot, old_row, item));
            }
            continue;
        }
        unkeyed.push((old_row, item));
    }
    let collisions = colliding.len();
    // `entries` is sorted by `rep` and `slot_for` is monotone non-decreasing, so `colliding` is
    // non-decreasing in `desired_slot` and a monotone cursor avoids O(N^2) duplicate probing.
    let mut next_collision = 0_usize;
    for (desired_slot, old_row, item) in colliding {
        let mut s = desired_slot.saturating_add(1).max(next_collision);
        loop {
            if s >= MAX_SUBCOLUMNS {
                return Err(DocError::TooManySubcolumns { count: s + 1, max: MAX_SUBCOLUMNS }.into());
            }
            if used.insert(s) {
                break;
            }
            s += 1;
        }
        next_collision = s.saturating_add(1);
        placed.push((s, old_row, item));
    }
    let mut next_free = 0_usize;
    for (old_row, item) in unkeyed {
        loop {
            if next_free >= MAX_SUBCOLUMNS {
                return Err(DocError::TooManySubcolumns { count: next_free + 1, max: MAX_SUBCOLUMNS }.into());
            }
            if used.insert(next_free) {
                break;
            }
            next_free += 1;
        }
        placed.push((next_free, old_row, item));
        next_free += 1;
    }
    placed.sort_by_key(|(s, _, _)| *s);
    Ok((placed, collisions))
}

fn slot_buckets<T>(buckets: RepBucketMap<T>, has_rep_col: bool) -> SlottedBucketResult<T> {
    let mapper = has_rep_col
        .then(|| ReplicateSlotMapper::from_values(buckets.values().flat_map(|entries| entries.iter().filter_map(|(rep_opt, _, _)| *rep_opt))));
    let mut assigned_buckets: SlottedBucketMap<T> = BTreeMap::new();
    let mut max_reps = 1_usize;
    let mut total_collisions = 0_usize;
    for (key, entries) in buckets {
        let (assigned, collisions) = assign_bucket_slots(entries, mapper.as_ref())?;
        total_collisions += collisions;
        if let Some(bucket_max) = assigned.iter().map(|(s, _, _)| s + 1).max() {
            max_reps = max_reps.max(bucket_max);
        }
        assigned_buckets.insert(key, assigned);
    }
    Ok((assigned_buckets, max_reps, total_collisions))
}

fn extract_replicate_order(
    mv: &MultipleVariablesTable,
    rep_col: Option<usize>,
    row: usize,
    acc: &mut ConversionAcc,
    warned_rep_dropped: &mut bool,
) -> Option<f64> {
    let rc = rep_col?;
    let rep_order = mv.variables[rc].cells.get(row).and_then(Cell::active_number);
    if rep_order.is_some() {
        acc.preserved += 1;
    } else if mv.variables[rc].cells.get(row).is_some_and(is_occupied) {
        acc.drop_cells(1);
        if !*warned_rep_dropped {
            *warned_rep_dropped = true;
            acc.warn("Non-numeric or excluded replicate values cannot be mapped to explicit replicate slots and were ignored");
        }
    }
    rep_order
}

fn pivot_mv_summary_to_grouped(
    mv: &MultipleVariablesTable,
    factors: (usize, usize),
    summary_cols: &[usize; 3],
    mode: SubcolumnMode,
    part: &MvPartition,
) -> Result<BuiltArchetype, TransformError> {
    let (row_fac_col, col_fac_col) = factors;
    let mut acc = ConversionAcc::default();
    let extra_cols =
        part.cat_cols.iter().skip(2).chain(part.label_cols.iter()).chain(part.num_cols.iter().filter(|c| !summary_cols.contains(c))).copied();
    acc.drop_mv_columns(mv, extra_cols, part.rows);

    let mut row_levels = LevelInterner::default();
    let mut col_levels = LevelInterner::default();
    let mut entries: Vec<(usize, usize, usize, [Cell; 3])> = Vec::new();
    let mut seen_pairs: BTreeSet<(usize, usize)> = BTreeSet::new();

    for r in 0..part.rows {
        let r_tok = mv_cell_level_token(mv.variables[row_fac_col].cells.get(r));
        let c_tok = mv_cell_level_token(mv.variables[col_fac_col].cells.get(r));
        let triplet = [
            mv.variables[summary_cols[0]].cells.get(r).cloned().unwrap_or_default(),
            mv.variables[summary_cols[1]].cells.get(r).cloned().unwrap_or_default(),
            mv.variables[summary_cols[2]].cells.get(r).cloned().unwrap_or_default(),
        ];
        let (Some(rk), Some(ck)) = (r_tok, c_tok) else {
            let skipped = count_mv_row_occupied(mv, [row_fac_col, col_fac_col], r) + triplet.iter().filter(|c| is_occupied(c)).count();
            acc.drop_cells(skipped);
            continue;
        };
        let ri = row_levels.intern(rk);
        let ci = col_levels.intern(ck);
        if !seen_pairs.insert((ri, ci)) {
            acc.drop_cells(2 + triplet.iter().filter(|c| is_occupied(c)).count());
            continue;
        }
        entries.push((ri, ci, r, triplet));
    }

    if col_levels.is_empty() || row_levels.is_empty() {
        return Err(TransformError::InvalidInput {
            message: String::from("MultipleVariables summary Grouped pivot requires at least one row factor level and one column factor level"),
        });
    }
    let col_titles = col_levels.into_levels();
    let mut grouped = build_grouped_shell(
        mode,
        row_levels.into_levels(),
        &col_titles,
        (&mv.variables[row_fac_col].title, Some(&mv.variables[col_fac_col].title)),
        "MultipleVariables summary Grouped pivot",
    )?;

    acc.preserved += entries.len() * 2;
    for (ri, ci, old_r, triplet) in entries {
        for (s, cell) in triplet.into_iter().enumerate() {
            acc.record_preserved(&cell);
            set_dataset_cell(&mut grouped.data_sets, ci, ri, s, cell);
            acc.map_coord(CellCoord::new(old_r, summary_cols[s], 0), CellCoord::new(ri, ci, s));
        }
    }
    if acc.schema_lossy {
        acc.warn("Extra columns or duplicate factor pairs were dropped during Grouped summary pivot");
    }
    Ok(acc.finish(TableArchetype::Grouped(grouped)))
}

fn pivot_mv_one_factor_to_grouped(mv: &MultipleVariablesTable, part: &MvPartition) -> Result<BuiltArchetype, TransformError> {
    let row_fac_col = part.cat_cols[0];
    let mut acc = ConversionAcc::default();
    acc.drop_mv_columns(mv, part.label_cols.iter().copied(), part.rows);

    let mut row_levels = LevelInterner::default();
    let mut row_slot_count: BTreeMap<usize, usize> = BTreeMap::new();
    let mut buckets: SlottedBucketMap<Cell> = BTreeMap::new();

    for r in 0..part.rows {
        let Some(rk) = mv_cell_level_token(mv.variables[row_fac_col].cells.get(r)) else {
            let skipped = count_mv_row_occupied(mv, std::iter::once(row_fac_col).chain(part.num_cols.iter().copied()), r);
            acc.drop_cells(skipped);
            continue;
        };
        acc.preserved += 1;
        let ri = row_levels.intern(rk);
        if part.num_cols.iter().any(|&nc| mv.variables[nc].cells.get(r).is_some_and(is_occupied)) {
            let slot_entry = row_slot_count.entry(ri).or_insert(0);
            let slot = *slot_entry;
            *slot_entry += 1;
            for (ci, &nc) in part.num_cols.iter().enumerate() {
                if let Some(cell) = mv.variables[nc].cells.get(r)
                    && is_occupied(cell)
                {
                    buckets.entry((ri, ci)).or_default().push((slot, r, cell.clone()));
                }
            }
        }
    }

    let max_reps = row_slot_count.values().copied().max().unwrap_or(1).max(1);
    let col_titles: Vec<String> = part.num_cols.iter().map(|&nc| mv.variables[nc].title.clone()).collect();
    let mut grouped = build_grouped_shell(
        SubcolumnMode::RawReplicates(max_reps),
        row_levels.into_levels(),
        &col_titles,
        (&mv.variables[row_fac_col].title, None),
        "MultipleVariables one-factor Grouped pivot",
    )?;

    for ((ri, ci), entries) in buckets {
        let old_col = part.num_cols[ci];
        for (s, old_r, cell) in entries {
            set_dataset_cell(&mut grouped.data_sets, ci, ri, s, cell);
            acc.map_coord(CellCoord::new(old_r, old_col, 0), CellCoord::new(ri, ci, s));
            acc.preserved += 1;
        }
    }
    if acc.schema_lossy {
        acc.warn("Unmatched label/missing-factor cells were dropped during Grouped pivot");
    }
    Ok(acc.finish(TableArchetype::Grouped(grouped)))
}

fn detect_mv_summary_triple(mv: &MultipleVariablesTable, num_cols: &[usize]) -> Option<(SubcolumnMode, [usize; 3])> {
    if num_cols.len() != 3 {
        return None;
    }
    let candidates = [
        SubcolumnMode::MeanSdN,
        SubcolumnMode::MeanSemN,
        SubcolumnMode::MeanCvN,
        SubcolumnMode::MeanLowHigh,
        SubcolumnMode::UpperLowerLimits,
        SubcolumnMode::MedianIqr,
    ];
    for mode in candidates {
        let labels = mode.subcolumn_labels();
        if (0..3).all(|i| mv.variables[num_cols[i]].title.trim().eq_ignore_ascii_case(&labels[i])) {
            return Some((mode, [num_cols[0], num_cols[1], num_cols[2]]));
        }
    }
    None
}

fn split_replicate_and_value(mv: &MultipleVariablesTable, num_cols: &[usize]) -> (Option<usize>, usize, Vec<usize>) {
    if num_cols.len() >= 2
        && let Some(pos) = num_cols.iter().position(|&c| {
            let t = mv.variables[c].title.trim().to_ascii_lowercase();
            t == "replicate" || t == "rep"
        })
    {
        let rep_col = num_cols[pos];
        let others: Vec<usize> = num_cols.iter().copied().filter(|&c| c != rep_col).collect();
        if let Some((&val_col, rest)) = others.split_first() {
            return (Some(rep_col), val_col, rest.to_vec());
        }
    }
    (None, num_cols[0], num_cols[1..].to_vec())
}

// ============================================================================
// XY <-> MultipleVariables
// ============================================================================

fn convert_xy_to_mv(xy: &XyTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&xy.data_sets, "XY table has no Y data columns")?;
    let rows = xy_row_count(xy);
    let x_subs = xy.x_mode.subcolumn_count();

    if xy.y_subcolumn_mode == SubcolumnMode::Single && x_subs <= 1 {
        return convert_xy_single_to_mv(xy, rows);
    }
    if let SubcolumnMode::RawReplicates(n) = xy.y_subcolumn_mode
        && n >= 2
        && x_subs <= 1
    {
        return convert_xy_reps_to_mv(xy, rows, n);
    }
    convert_xy_wide_to_mv(xy, rows, x_subs)
}

fn convert_xy_single_to_mv(xy: &XyTable, rows: usize) -> Result<BuiltArchetype, TransformError> {
    let var_count = 1 + xy.data_sets.len();
    validate_cell_budget(rows, var_count, 1, 0, "XY-to-MultipleVariables reshape")?;
    let mut mv = MultipleVariablesTable::new(var_count, rows)?;
    mv.row_titles = resized_titles(&xy.row_titles, rows);
    mv.row_titles_expanded = xy.row_titles_expanded;
    mv.variables[0].title = non_empty_or(&xy.x_title, "X");
    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        mv.variables[c + 1].title = src_ds.title.clone();
    }

    let mut acc = ConversionAcc::default();
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        acc.warn("Generated X sequence materialized into Continuous variable column");
    }
    acc.preserved += populate_xy_mv_x_cells(xy, &mut mv, rows, 1);
    DataSetSubcolumnUnpacker::new(&mut acc, rows).unpack_to_mv(&xy.data_sets, &mut mv, 1, 1);
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }
    Ok(acc.finish(TableArchetype::MultipleVariables(mv)))
}

fn xy_row_has_unpaired_x_only(xy: &XyTable, row: usize) -> bool {
    match &xy.x_mode {
        XColumnMode::GeneratedSequence { .. } => xy.x_cells.get(row).and_then(|s| s.first()).is_some_and(|c| c.excluded),
        XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => xy.x_cells.get(row).and_then(|s| s.first()).is_some_and(is_occupied),
    }
}

fn count_xy_reps_mv_rows(xy: &XyTable, rows: usize, n: usize) -> Result<usize, TransformError> {
    let mut out_rows = 0_usize;
    for r in 0..rows {
        let mut any_y = false;
        for ds in &xy.data_sets {
            if let Some(row_cells) = ds.cells.get(r)
                && row_cells.iter().take(n).any(is_occupied)
            {
                any_y = true;
                out_rows = out_rows
                    .checked_add(n)
                    .ok_or_else(|| TransformError::InvalidSpec { message: String::from("XY-to-MultipleVariables row count overflows") })?;
                if out_rows > MAX_ROWS {
                    validate_rows(out_rows)?;
                }
            }
        }
        if !any_y && xy_row_has_unpaired_x_only(xy, r) {
            out_rows = out_rows
                .checked_add(1)
                .ok_or_else(|| TransformError::InvalidSpec { message: String::from("XY-to-MultipleVariables row count overflows") })?;
            if out_rows > MAX_ROWS {
                validate_rows(out_rows)?;
            }
        }
    }
    Ok(out_rows)
}

fn convert_xy_reps_to_mv(xy: &XyTable, rows: usize, n: usize) -> Result<BuiltArchetype, TransformError> {
    let out_rows = count_xy_reps_mv_rows(xy, rows, n)?;
    validate_rows(out_rows)?;
    validate_cell_budget(out_rows, 4, 1, 0, "XY-to-MultipleVariables replicate reshape")?;
    let mut mv = MultipleVariablesTable::new(4, out_rows)?;
    mv.variables[0].title = non_empty_or(&xy.x_title, "X");
    mv.variables[1].title = String::from("Group");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Replicate");
    mv.variables[3].title = String::from("Value");

    let mut acc = ConversionAcc::default();
    let mut paired_y_rows = BTreeSet::new();
    let mut counted_x_rows = BTreeSet::new();
    let grp_labels: Vec<String> = xy.data_sets.iter().enumerate().map(|(c, ds)| non_empty_or(&ds.title, &default_column_label(c))).collect();
    let mut out_r = 0_usize;

    for r in 0..rows {
        let x_cell = xy_materialized_x_cell(xy, r);
        for (c, ds) in xy.data_sets.iter().enumerate() {
            if let Some(row_cells) = ds.cells.get(r)
                && row_cells.iter().take(n).any(is_occupied)
            {
                paired_y_rows.insert(r);
                let grp_cell = Cell::text(grp_labels[c].clone());
                for s in 0..n {
                    let yc = row_cells.get(s).cloned().unwrap_or_default();
                    if is_occupied(&x_cell) && counted_x_rows.insert(r) {
                        acc.preserved += 1;
                    }
                    if let Some(title) = xy.row_titles.get(r)
                        && let Some(dst) = mv.row_titles.get_mut(out_r)
                    {
                        dst.clone_from(title);
                    }
                    set_mv_cell(&mut mv, 0, out_r, x_cell.clone());
                    set_mv_cell(&mut mv, 1, out_r, grp_cell.clone());
                    #[allow(clippy::cast_precision_loss)]
                    set_mv_cell(&mut mv, 2, out_r, Cell::number((s + 1) as f64));
                    if is_occupied(&yc) {
                        acc.preserved += 1;
                        acc.map_coord(CellCoord::new(r, c, s), CellCoord::new(out_r, 3, 0));
                    }
                    set_mv_cell(&mut mv, 3, out_r, yc);
                    out_r += 1;
                }
            }
        }
    }

    // Preserve any occupied X-only row (including excluded GeneratedSequence X) not paired with Y.
    for r in 0..rows {
        if !paired_y_rows.contains(&r) && xy_row_has_unpaired_x_only(xy, r) {
            let x_cell = xy_materialized_x_cell(xy, r);
            acc.preserved += 1;
            if let Some(title) = xy.row_titles.get(r)
                && let Some(dst) = mv.row_titles.get_mut(out_r)
            {
                dst.clone_from(title);
            }
            set_mv_cell(&mut mv, 0, out_r, x_cell);
            out_r += 1;
        }
    }

    mv.row_titles_expanded = xy.row_titles_expanded || has_non_empty_title(&mv.row_titles);
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }
    acc.warn("XY replicate subcolumns reshaped to tidy long (X, Group, Replicate, Value) variables");
    Ok(acc.finish(TableArchetype::MultipleVariables(mv)))
}

fn push_dataset_subcolumn_titles(data_sets: &[DataSetColumn], mode: SubcolumnMode, out: &mut Vec<String>) -> Result<(), TransformError> {
    let subs = mode.subcolumn_count();
    let labels = mode.subcolumn_labels();
    for src_ds in data_sets {
        for lbl in labels.iter().take(subs) {
            let t = if subs == 1 { src_ds.title.clone() } else { format!("{} ({lbl})", src_ds.title) };
            validate_title(&t)?;
            out.push(t);
        }
    }
    Ok(())
}

fn collect_xy_wide_mv_titles(xy: &XyTable, x_subs: usize, total_vars: usize) -> Result<Vec<String>, TransformError> {
    let mut var_titles = Vec::with_capacity(total_vars);
    let x_base = non_empty_or(&xy.x_title, "X");
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        validate_title(&x_base)?;
        var_titles.push(x_base);
    } else {
        let x_labels = match &xy.x_mode {
            XColumnMode::Numbers { subcolumn_mode } => subcolumn_mode.subcolumn_labels(),
            _ => vec![String::from("X")],
        };
        for x_lbl in x_labels.iter().take(x_subs) {
            let t = if x_subs == 1 { x_base.clone() } else { format!("{x_base} ({x_lbl})") };
            validate_title(&t)?;
            var_titles.push(t);
        }
    }
    push_dataset_subcolumn_titles(&xy.data_sets, xy.y_subcolumn_mode, &mut var_titles)?;
    Ok(var_titles)
}

fn populate_xy_mv_x_cells(xy: &XyTable, mv: &mut MultipleVariablesTable, rows: usize, x_subs: usize) -> usize {
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        for r in 0..rows {
            set_mv_cell(mv, 0, r, xy_materialized_x_cell(xy, r));
        }
        return 0;
    }
    let mut preserved = 0_usize;
    for s in 0..x_subs {
        for (r, x_row) in xy.x_cells.iter().enumerate().take(rows) {
            if let Some(cell) = x_row.get(s) {
                set_mv_cell(mv, s, r, cell.clone());
                if is_occupied(cell) {
                    preserved += 1;
                }
            }
        }
    }
    preserved
}

fn convert_xy_wide_to_mv(xy: &XyTable, rows: usize, x_subs: usize) -> Result<BuiltArchetype, TransformError> {
    let x_var_count = if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) { 1 } else { x_subs };
    let y_subs = xy.y_subcolumn_mode.subcolumn_count();
    let total_vars = xy
        .data_sets
        .len()
        .checked_mul(y_subs)
        .and_then(|y_vars| x_var_count.checked_add(y_vars))
        .ok_or_else(|| TransformError::InvalidSpec { message: String::from("XY-to-MultipleVariables column count overflows") })?;
    validate_data_sets(total_vars)?;
    validate_cell_budget(rows, total_vars, 1, 0, "XY-to-MultipleVariables wide reshape")?;

    let var_titles = collect_xy_wide_mv_titles(xy, x_subs, total_vars)?;
    let mut mv = MultipleVariablesTable::new(total_vars, rows)?;
    mv.row_titles = resized_titles(&xy.row_titles, rows);
    mv.row_titles_expanded = xy.row_titles_expanded;
    for (idx, t) in var_titles.into_iter().enumerate() {
        if let Some(var) = mv.variables.get_mut(idx) {
            var.title = t;
        }
    }

    let mut acc = ConversionAcc::default();
    acc.preserved += populate_xy_mv_x_cells(xy, &mut mv, rows, x_subs);
    DataSetSubcolumnUnpacker::new(&mut acc, rows).unpack_to_mv(&xy.data_sets, &mut mv, x_var_count, y_subs);
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    let x_sub_lossy = matches!(&xy.x_mode, XColumnMode::Numbers { subcolumn_mode } if *subcolumn_mode != SubcolumnMode::Single);
    let y_sub_lossy = xy.y_subcolumn_mode.is_pre_summarized() || matches!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(n) if n >= 2);
    if x_sub_lossy || y_sub_lossy {
        acc.warn_lossy(format!(
            "XY subcolumn structure ({:?}) flattened into independent Continuous variables; summary/replicate semantics lost",
            xy.y_subcolumn_mode
        ));
    }
    Ok(acc.finish(TableArchetype::MultipleVariables(mv)))
}

fn convert_mv_to_xy(mv: &MultipleVariablesTable) -> Result<BuiltArchetype, TransformError> {
    let part = MvPartition::from_table(mv)?;
    if part.cat_cols.len() == 1 && part.num_cols.len() >= 2 {
        return pivot_mv_grouped_xy(mv, &part);
    }

    let mut acc = ConversionAcc::default();
    acc.drop_mv_columns(mv, part.cat_cols.iter().copied(), part.rows);

    let mut row_titles = resized_titles(&mv.row_titles, part.rows);
    if part.label_cols.len() == 1 && !has_non_empty_title(&row_titles) {
        fill_mv_label_row_titles(mv, part.label_cols[0], &mut row_titles, &mut acc)?;
    } else {
        acc.drop_mv_columns(mv, part.label_cols.iter().copied(), part.rows);
    }

    if part.num_cols.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("MultipleVariables table has no numeric variables to map to XY") });
    }
    if acc.schema_lossy {
        acc.warn("Categorical/text variables cannot be represented directly in XY columns and will be dropped");
    }

    let y_count = if part.num_cols.len() == 1 { 1 } else { part.num_cols.len() - 1 };
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, y_count, part.rows)?;
    xy.row_titles = row_titles;
    xy.row_titles_expanded = has_non_empty_title(&xy.row_titles);

    if part.num_cols.len() == 1 {
        let yc = part.num_cols[0];
        xy.data_sets[0].title = mv.variables[yc].title.clone();
        let row_titles_copy = xy.row_titles.clone();
        populate_generated_or_titled_x(&mut xy, &row_titles_copy, part.rows);
        for (r, cell) in mv.variables[yc].cells.iter().enumerate().take(part.rows) {
            if let Some(slot) = xy.data_sets[0].cells.get_mut(r).and_then(|row| row.first_mut()) {
                *slot = cell.clone();
                acc.record_preserved(cell);
            }
            acc.map_coord(CellCoord::new(r, yc, 0), CellCoord::new(r, 0, 0));
        }
        acc.warn("Single numeric variable mapped to Y with sequential X coordinates");
    } else {
        let xc = part.num_cols[0];
        xy.x_title = mv.variables[xc].title.clone();
        let mut text_in_x = 0_usize;
        for (r, cell) in mv.variables[xc].cells.iter().enumerate().take(part.rows) {
            if let Some(slot) = xy.x_cells.get_mut(r).and_then(|row| row.first_mut())
                && copy_mv_cell_to_xy_numeric(slot, cell, &mut acc)
            {
                text_in_x += 1;
            }
        }
        if text_in_x > 0 {
            acc.warn_lossy(format!(
                "X variable `{}` contains {text_in_x} non-numeric text cell(s) incompatible with numeric XY X coordinates",
                mv.variables[xc].title
            ));
        }
        for (yi, &yc) in part.num_cols.iter().skip(1).enumerate() {
            xy.data_sets[yi].title = mv.variables[yc].title.clone();
            for (r, cell) in mv.variables[yc].cells.iter().enumerate().take(part.rows) {
                if let Some(slot) = xy.data_sets.get_mut(yi).and_then(|d| d.cells.get_mut(r)).and_then(|row| row.first_mut()) {
                    *slot = cell.clone();
                    acc.record_preserved(cell);
                }
                acc.map_coord(CellCoord::new(r, yc, 0), CellCoord::new(r, yi, 0));
            }
        }
    }

    Ok(acc.finish(TableArchetype::Xy(xy)))
}

fn copy_mv_cell_to_xy_numeric(dst: &mut Cell, src: &Cell, acc: &mut ConversionAcc) -> bool {
    match &src.value {
        CellValue::Text(s) if !s.trim().is_empty() => {
            acc.drop_cells(1);
            *dst = Cell { value: CellValue::Empty, excluded: src.excluded };
            true
        }
        _ => {
            *dst = src.clone();
            acc.record_preserved(src);
            false
        }
    }
}

fn pivot_mv_grouped_xy(mv: &MultipleVariablesTable, part: &MvPartition) -> Result<BuiltArchetype, TransformError> {
    let group_col = part.cat_cols[0];
    let x_col = part.num_cols[0];
    let (rep_col, y_col, extra_nums) = split_replicate_and_value(mv, &part.num_cols[1..]);

    let mut acc = ConversionAcc::default();
    if acc.drop_mv_columns(mv, part.label_cols.iter().chain(extra_nums.iter()).copied(), part.rows) {
        acc.warn("Extra numeric/label variables cannot fit in a single grouped XY pivot and will be dropped");
    }

    let mut x_keys: Vec<f64> = Vec::new();
    let mut x_index: BTreeMap<u64, usize> = BTreeMap::new();
    let mut groups = LevelInterner::default();
    let mut buckets: RepBucketMap<(Cell, bool)> = BTreeMap::new();
    let mut x_any_active: BTreeSet<usize> = BTreeSet::new();
    let mut x_any_excluded: BTreeSet<usize> = BTreeSet::new();
    let mut warned_rep_dropped = false;
    let mut skipped_xy_cells = 0_usize;

    for r in 0..part.rows {
        let x_cell = mv.variables[x_col].cells.get(r);
        let g_tok = mv_cell_level_token(mv.variables[group_col].cells.get(r));
        let y_cell = mv.variables[y_col].cells.get(r).cloned().unwrap_or_default();

        let (Some(xv), Some(gk)) = (x_cell.and_then(|c| c.value.finite_f64()), g_tok) else {
            let skipped = count_mv_row_occupied(mv, [x_col, group_col, y_col].into_iter().chain(rep_col), r);
            acc.drop_cells(skipped);
            skipped_xy_cells += skipped;
            continue;
        };

        let norm_x = normalize_zero(xv);
        let xi = *x_index.entry(norm_x.to_bits()).or_insert_with(|| {
            let idx = x_keys.len();
            x_keys.push(norm_x);
            idx
        });
        let x_excluded = x_cell.is_some_and(|c| c.excluded);
        if x_excluded {
            x_any_excluded.insert(xi);
        } else {
            x_any_active.insert(xi);
        }
        let gi = groups.intern(gk);
        acc.preserved += 2;
        let rep_ord = extract_replicate_order(mv, rep_col, r, &mut acc, &mut warned_rep_dropped);
        buckets.entry((xi, gi)).or_default().push((rep_ord, r, (y_cell, x_excluded)));
    }
    if skipped_xy_cells > 0 {
        acc.warn(format!("Dropped {skipped_xy_cells} cell(s) from rows with missing/non-numeric X or excluded group labels"));
    }

    if groups.is_empty() || x_keys.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no valid (X, Group, Y) records to pivot into XY") });
    }

    let mut order: Vec<usize> = (0..x_keys.len()).collect();
    order.sort_by(|&a, &b| x_keys[a].total_cmp(&x_keys[b]));
    let mut remap_x = vec![0_usize; x_keys.len()];
    for (new_idx, &old_idx) in order.iter().enumerate() {
        remap_x[old_idx] = new_idx;
    }

    let n_rows = order.len();
    let n_cols = groups.len();
    validate_data_sets(n_cols)?;
    validate_rows(n_rows)?;
    let (assigned_buckets, max_reps, rep_collisions) = slot_buckets(buckets, rep_col.is_some())?;
    if rep_collisions > 0 {
        acc.warn(format!("{rep_collisions} duplicate replicate slot collision(s) shifted to subsequent subcolumns"));
    }
    validate_subcolumns(max_reps)?;
    let group_titles = groups.into_levels();
    validate_title_slice(&group_titles)?;
    validate_cell_budget(n_rows, n_cols, max_reps, 1, "MultipleVariables XY pivot")?;

    let y_mode = if max_reps == 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(max_reps) };
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, y_mode, n_cols, n_rows)?;
    xy.x_title = mv.variables[x_col].title.clone();
    let row_x_excluded = |old_x_idx: usize| x_any_excluded.contains(&old_x_idx) && !x_any_active.contains(&old_x_idx);
    for (new_r, &old_x_idx) in order.iter().enumerate() {
        set_x_cell(&mut xy, new_r, 0, Cell { value: CellValue::Number(x_keys[old_x_idx]), excluded: row_x_excluded(old_x_idx) });
    }
    for (gi, g_name) in group_titles.into_iter().enumerate() {
        xy.data_sets[gi].title = g_name;
    }

    for ((old_xi, gi), assigned) in assigned_buckets {
        let dst_r = remap_x[old_xi];
        let shared_x_excluded = row_x_excluded(old_xi);
        for (s, old_r, (mut cell, x_excluded)) in assigned {
            let y_was_excluded = cell.excluded;
            if x_excluded && !shared_x_excluded {
                cell.excluded = true;
                if !y_was_excluded {
                    acc.coord_map.entry(CellCoord::new(old_r, x_col, 0)).or_insert(CellCoord::new(dst_r, gi, s));
                }
            }
            acc.record_preserved(&cell);
            set_dataset_cell(&mut xy.data_sets, gi, dst_r, s, cell);
            acc.map_coord(CellCoord::new(old_r, y_col, 0), CellCoord::new(dst_r, gi, s));
        }
    }
    Ok(acc.finish(TableArchetype::Xy(xy)))
}

// ============================================================================
// Column <-> MultipleVariables
// ============================================================================

fn convert_column_to_mv(col: &ColumnTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&col.data_sets, "Column table has no data columns")?;
    let rows = column_row_count(&col.data_sets, col.row_titles.len());
    let subs = col.subcolumn_mode.subcolumn_count();
    let total_vars = col
        .data_sets
        .len()
        .checked_mul(subs)
        .ok_or_else(|| TransformError::InvalidSpec { message: String::from("Column-to-MultipleVariables column count overflows") })?;
    validate_data_sets(total_vars)?;
    validate_cell_budget(rows, total_vars, 1, 0, "Column-to-MultipleVariables reshape")?;

    let mut var_titles = Vec::with_capacity(total_vars);
    push_dataset_subcolumn_titles(&col.data_sets, col.subcolumn_mode, &mut var_titles)?;

    let mut mv = MultipleVariablesTable::new(total_vars, rows)?;
    mv.row_titles = resized_titles(&col.row_titles, rows);
    mv.row_titles_expanded = col.row_titles_expanded;
    for (idx, t) in var_titles.into_iter().enumerate() {
        if let Some(var) = mv.variables.get_mut(idx) {
            var.title = t;
        }
    }

    let mut acc = ConversionAcc::default();
    DataSetSubcolumnUnpacker::new(&mut acc, rows).unpack_to_mv(&col.data_sets, &mut mv, 0, subs);
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    if col.subcolumn_mode.is_pre_summarized() {
        acc.warn_lossy(format!(
            "Pre-summarized {:?} subcolumns flattened into independent Continuous variables; summary moment semantics lost",
            col.subcolumn_mode
        ));
    }
    Ok(acc.finish(TableArchetype::MultipleVariables(mv)))
}

fn convert_mv_to_column(mv: &MultipleVariablesTable) -> Result<BuiltArchetype, TransformError> {
    let part = MvPartition::from_table(mv)?;
    if part.cat_cols.len() == 1 && part.num_cols.len() == 1 && mv.variables.len() == 2 {
        return unstack_mv_to_column(mv, part.cat_cols[0], part.num_cols[0], part.rows);
    }

    let mut col_tbl = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, mv.variables.len(), part.rows)?;
    col_tbl.row_titles = resized_titles(&mv.row_titles, part.rows);
    col_tbl.row_titles_expanded = mv.row_titles_expanded;
    let mut acc = ConversionAcc::default();
    let all_cols: Vec<usize> = (0..mv.variables.len()).collect();
    DataSetSubcolumnUnpacker::new(&mut acc, part.rows).copy_mv_vars_to_datasets(mv, &all_cols, &mut col_tbl.data_sets);
    Ok(acc.finish(TableArchetype::Column(col_tbl)))
}

fn unstack_mv_to_column(mv: &MultipleVariablesTable, grp_col: usize, val_col: usize, rows: usize) -> Result<BuiltArchetype, TransformError> {
    let mut groups = LevelInterner::default();
    let mut buckets: Vec<Vec<(usize, Cell)>> = Vec::new();
    let mut acc = ConversionAcc::default();

    let mut skipped_unstack_cells = 0_usize;
    for r in 0..rows {
        let cell = mv.variables[val_col].cells.get(r).cloned().unwrap_or_default();
        let Some(gk) = mv_cell_level_token(mv.variables[grp_col].cells.get(r)) else {
            let skipped = count_mv_row_occupied(mv, [grp_col, val_col], r);
            acc.drop_cells(skipped);
            skipped_unstack_cells += skipped;
            continue;
        };
        acc.preserved += 1;
        let gi = groups.intern(gk);
        if buckets.len() <= gi {
            buckets.resize_with(gi + 1, Vec::new);
        }
        buckets[gi].push((r, cell));
    }
    if skipped_unstack_cells > 0 {
        acc.warn(format!("Dropped {skipped_unstack_cells} cell(s) from rows with missing or excluded group labels"));
    }

    if groups.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no group levels found to unstack into Column table") });
    }
    let max_rows = buckets.iter().map(Vec::len).max().unwrap_or(0);
    let group_titles = groups.into_levels();
    validate_data_sets(group_titles.len())?;
    validate_rows(max_rows)?;
    validate_title_slice(&group_titles)?;
    validate_cell_budget(max_rows, group_titles.len(), 1, 0, "MultipleVariables Column unstack")?;

    let mut col_tbl = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, group_titles.len(), max_rows)?;
    for (ci, g_name) in group_titles.into_iter().enumerate() {
        col_tbl.data_sets[ci].title = g_name;
        for (dst_r, (old_r, cell)) in buckets[ci].iter().enumerate() {
            acc.record_preserved(cell);
            set_dataset_cell(&mut col_tbl.data_sets, ci, dst_r, 0, cell.clone());
            acc.map_coord(CellCoord::new(*old_r, val_col, 0), CellCoord::new(dst_r, ci, 0));
        }
    }
    acc.warn("Unstacked tidy (Group, Value) pairs into Column data sets");
    Ok(acc.finish(TableArchetype::Column(col_tbl)))
}

// ============================================================================
// Grouped <-> XY
// ============================================================================

fn convert_grouped_to_xy(grp: &GroupedTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&grp.data_sets, "Grouped table has no data columns")?;
    let rows = column_row_count(&grp.data_sets, grp.row_titles.len());
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, grp.subcolumn_mode, grp.data_sets.len(), rows)?;
    xy.x_title = non_empty_or(&grp.row_factor_title, "X");
    xy.row_titles = resized_titles(&grp.row_titles, rows);
    xy.row_titles_expanded = has_non_empty_title(&xy.row_titles);

    let mut acc = ConversionAcc::default();
    if let Some(spacing) = &grp.row_numeric_spacing {
        for r in 0..rows {
            set_x_cell(&mut xy, r, 0, spacing.get(r).and_then(|&v| v).filter(|v| v.is_finite()).map_or_else(Cell::empty, Cell::number));
        }
    } else {
        populate_generated_or_titled_x(&mut xy, &grp.row_titles, rows);
        acc.warn("Grouped row factor mapped to parsed or sequential X coordinates");
    }

    DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_direct(&grp.data_sets, &mut xy.data_sets, 0, 0, grp.subcolumn_mode.subcolumn_count());
    warn_grouped_metadata_loss(grp, false, "XY", &mut acc);
    Ok(acc.finish(TableArchetype::Xy(xy)))
}

fn convert_xy_to_grouped(xy: &XyTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&xy.data_sets, "XY table has no Y data columns")?;
    let rows = xy_row_count(xy);
    let target_mode = if xy.y_subcolumn_mode == SubcolumnMode::Single { SubcolumnMode::RawReplicates(1) } else { xy.y_subcolumn_mode };
    let xy_rows = build_xy_row_titles(xy, rows)?;

    let mut grouped = GroupedTable::new(target_mode, xy.data_sets.len(), rows)?;
    grouped.row_factor_title = non_empty_or(&xy.x_title, "Row Factor");
    grouped.row_titles = xy_rows.titles;
    grouped.row_numeric_spacing = Some(xy_rows.spacing);

    let mut acc = ConversionAcc::default();
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        let excluded_gen_x = count_excluded_generated_x(xy, rows);
        if excluded_gen_x > 0 {
            acc.drop_cells(excluded_gen_x);
            acc.warn_lossy(format!("Dropped {excluded_gen_x} excluded generated X value(s) unsupported by Grouped row spacing"));
        }
    } else {
        let mut dropped_x_primary = 0_usize;
        let mut dropped_x_subs = 0_usize;
        for x_row in xy.x_cells.iter().take(rows) {
            for (s, cell) in x_row.iter().enumerate() {
                if !is_occupied(cell) {
                    continue;
                }
                if s == 0 && cell.active_number().is_some() {
                    acc.preserved += 1;
                } else {
                    acc.drop_cells(1);
                    if s > 0 {
                        dropped_x_subs += 1;
                    } else {
                        dropped_x_primary += 1;
                    }
                }
            }
        }
        if dropped_x_primary > 0 {
            acc.warn(format!("Dropped {dropped_x_primary} excluded or non-numeric X cell(s) unsupported by Grouped row spacing"));
        }
        if dropped_x_subs > 0 {
            acc.warn(format!("Dropped {dropped_x_subs} X error subcolumn cell(s) unsupported by Grouped row spacing"));
        }
    }

    DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_direct(&xy.data_sets, &mut grouped.data_sets, 0, 0, target_mode.subcolumn_count());
    Ok(acc.finish(TableArchetype::Grouped(grouped)))
}

// ============================================================================
// * -> Contingency
// ============================================================================

fn convert_column_to_contingency(col: &ColumnTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&col.data_sets, "Column table has no data columns")?;
    build_contingency_from_datasets(&col.data_sets, &col.row_titles, None, None)
}

fn convert_grouped_to_contingency(grp: &GroupedTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&grp.data_sets, "Grouped table has no data columns")?;
    let mut built = build_contingency_from_datasets(
        &grp.data_sets,
        &grp.row_titles,
        Some(grp.row_factor_title.as_str()),
        Some(grp.column_factor_title.as_str()),
    )?;
    warn_grouped_metadata_loss(grp, true, "Contingency", &mut built.acc);
    Ok(built)
}

fn build_contingency_from_datasets(
    data_sets: &[DataSetColumn],
    row_titles: &[String],
    row_factor: Option<&str>,
    col_factor: Option<&str>,
) -> Result<BuiltArchetype, TransformError> {
    let rows = column_row_count(data_sets, row_titles.len());
    let mut cont = ContingencyTable::new(data_sets.len(), rows)?;
    cont.row_titles = resized_titles(row_titles, rows);
    if let Some(rf) = row_factor {
        cont.row_factor_title = non_empty_or(rf, "Outcome / Exposure");
    }
    if let Some(cf) = col_factor {
        cont.column_factor_title = non_empty_or(cf, "Group");
    }

    let mut acc = ConversionAcc::default();
    if DataSetSubcolumnUnpacker::new(&mut acc, rows).copy_primary_only(data_sets, &mut cont.data_sets, 0) {
        acc.warn_lossy("Extra subcolumns beyond primary value column cannot be stored in Contingency count cells and were dropped");
    }
    cont.validate_counts()?;
    Ok(acc.finish(TableArchetype::Contingency(cont)))
}

fn convert_xy_to_contingency(xy: &XyTable) -> Result<BuiltArchetype, TransformError> {
    require_non_empty_datasets(&xy.data_sets, "XY table has no Y data columns")?;
    let rows = xy_row_count(xy);
    let xy_rows = build_xy_row_titles(xy, rows)?;

    let mut extra_acc = ConversionAcc::default();
    if xy_rows.x_used_as_label {
        extra_acc.warn_lossy("XY X values converted to Contingency row labels (numeric X spacing dropped)");
    } else if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) && rows > 0 {
        extra_acc.warn_lossy("XY generated X sequence cannot be represented in Contingency tables and will be discarded");
    }
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        let excluded_gen_x = count_excluded_generated_x(xy, rows);
        if excluded_gen_x > 0 {
            extra_acc.drop_cells(excluded_gen_x);
            extra_acc.warn_lossy(format!("Dropped {excluded_gen_x} excluded generated X value(s) unsupported by Contingency row labels"));
        }
    } else {
        let mut x_primary_dropped = 0_usize;
        let mut x_extra = 0_usize;
        for (r, x_row) in xy.x_cells.iter().enumerate().take(rows) {
            for (s, cell) in x_row.iter().enumerate() {
                if !is_occupied(cell) {
                    continue;
                }
                let mapped_to_label = s == 0 && xy.row_titles.get(r).is_none_or(|t| t.trim().is_empty()) && cell.active_number().is_some();
                if !mapped_to_label {
                    extra_acc.drop_cells(1);
                    if s > 0 {
                        x_extra += 1;
                    } else {
                        x_primary_dropped += 1;
                    }
                }
            }
        }
        if x_primary_dropped > 0 {
            extra_acc.warn(format!(
                "Dropped {x_primary_dropped} X cell(s) (excluded, non-numeric, or overridden by row titles) unsupported by Contingency row labels"
            ));
        }
        if x_extra > 0 {
            extra_acc.warn(format!("Dropped {x_extra} X subcolumn cell(s) unsupported by Contingency row labels"));
        }
    }

    let mut built = build_contingency_from_datasets(&xy.data_sets, &xy_rows.titles, Some(xy.x_title.as_str()), None)?;
    built.acc.dropped += extra_acc.dropped;
    built.acc.schema_lossy |= extra_acc.schema_lossy;
    built.acc.warnings.extend(extra_acc.warnings);
    Ok(built)
}

fn convert_mv_to_contingency(mv: &MultipleVariablesTable) -> Result<BuiltArchetype, TransformError> {
    let part = MvPartition::from_table(mv)?;
    if part.cat_cols.len() == 2
        && mv.variables[part.cat_cols[0]].role == VariableRole::Categorical
        && mv.variables[part.cat_cols[1]].role == VariableRole::Categorical
    {
        if part.num_cols.is_empty() {
            return crosstab_mv_to_contingency(mv, &part, None);
        }
        if part.num_cols.len() == 1 && mv.variables.len() == 3 + part.label_cols.len() {
            return crosstab_mv_to_contingency(mv, &part, Some(part.num_cols[0]));
        }
    }

    let label_col =
        part.label_cols.first().copied().filter(|_| part.label_cols.len() == 1 && !part.num_cols.is_empty() && !has_non_empty_title(&mv.row_titles));
    let data_indices: Vec<usize> = (0..mv.variables.len()).filter(|&i| Some(i) != label_col).collect();
    if data_indices.is_empty() {
        return Err(TransformError::InvalidSpec {
            message: String::from("MultipleVariables table has no count variables to convert to Contingency"),
        });
    }
    validate_data_sets(data_indices.len())?;

    let mut cont = ContingencyTable::new(data_indices.len(), part.rows)?;
    let mut acc = ConversionAcc::default();
    if let Some(lc) = label_col {
        cont.row_factor_title = non_empty_or(&mv.variables[lc].title, "Outcome / Exposure");
        fill_mv_label_row_titles(mv, lc, &mut cont.row_titles, &mut acc)?;
    } else {
        cont.row_titles = resized_titles(&mv.row_titles, part.rows);
    }

    DataSetSubcolumnUnpacker::new(&mut acc, part.rows).copy_mv_vars_to_datasets(mv, &data_indices, &mut cont.data_sets);
    cont.validate_counts()?;
    Ok(acc.finish(TableArchetype::Contingency(cont)))
}

/// Largest exact-integer `f64` value (`2^53`) accepted and accumulated for contingency counts.
const MAX_EXACT_F64_COUNT: u64 = 1_u64 << 53;

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn validate_count_cell_value(cell: &Cell, row: usize, col: usize) -> Result<Option<u64>, TransformError> {
    if cell.excluded || cell.is_empty() {
        return Ok(None);
    }
    match &cell.value {
        CellValue::Empty => Ok(None),
        CellValue::Number(v) if v.is_finite() && *v >= 0.0 && v.fract() == 0.0 && *v <= (MAX_EXACT_F64_COUNT as f64) => Ok(Some(*v as u64)),
        _ => Err(TransformError::Doc(DocError::InvalidCellValue {
            archetype: TableKind::Contingency,
            row,
            col,
            reason: "contingency table cells must be exact non-negative integer counts <= 2^53",
        })),
    }
}

fn crosstab_mv_to_contingency(mv: &MultipleVariablesTable, part: &MvPartition, count_col: Option<usize>) -> Result<BuiltArchetype, TransformError> {
    let row_fac_col = part.cat_cols[0];
    let col_fac_col = part.cat_cols[1];
    let mut row_levels = LevelInterner::default();
    let mut col_levels = LevelInterner::default();
    let mut counts: BTreeMap<(usize, usize), u64> = BTreeMap::new();
    let mut acc = ConversionAcc::default();
    if acc.drop_mv_columns(mv, part.label_cols.iter().copied(), part.rows) {
        acc.warn("Label variable(s) cannot be represented in Contingency cross-tabulation and will be dropped");
    }

    let mut skipped_crosstab_cells = 0_usize;
    for r in 0..part.rows {
        let row_active = mv_cell_level_token(mv.variables[row_fac_col].cells.get(r));
        let col_active = mv_cell_level_token(mv.variables[col_fac_col].cells.get(r));
        let increment = match count_col {
            Some(vc) => validate_count_cell_value(&mv.variables[vc].cells.get(r).cloned().unwrap_or_default(), r, vc)?,
            None => Some(1),
        };

        let (Some(rk), Some(ck), Some(cnt)) = (row_active, col_active, increment) else {
            let skipped = count_mv_row_occupied(mv, [row_fac_col, col_fac_col].into_iter().chain(count_col), r);
            acc.drop_cells(skipped);
            skipped_crosstab_cells += skipped;
            continue;
        };
        let ri = row_levels.intern(rk);
        let ci = col_levels.intern(ck);
        let entry = counts.entry((ri, ci)).or_insert(0);
        let next = entry.checked_add(cnt).filter(|&sum| sum <= MAX_EXACT_F64_COUNT).ok_or_else(|| TransformError::InvalidInput {
            message: format!("cross-tabulated contingency count exceeds exact IEEE-754 integer limit ({MAX_EXACT_F64_COUNT})"),
        })?;
        *entry = next;
    }
    if skipped_crosstab_cells > 0 {
        acc.warn(format!("Dropped {skipped_crosstab_cells} cell(s) from rows with missing or excluded factor/count values"));
    }

    if row_levels.is_empty() || col_levels.is_empty() {
        return Err(TransformError::InvalidInput {
            message: String::from("no complete (Row Factor, Column Factor) pairs found to cross-tabulate into Contingency"),
        });
    }
    let row_titles = row_levels.into_levels();
    let col_titles = col_levels.into_levels();
    validate_rows(row_titles.len())?;
    validate_data_sets(col_titles.len())?;
    validate_title_slice(&row_titles)?;
    validate_title_slice(&col_titles)?;
    validate_cell_budget(row_titles.len(), col_titles.len(), 1, 0, "MultipleVariables Contingency crosstab")?;

    let mut cont = ContingencyTable::new(col_titles.len(), row_titles.len())?;
    cont.row_factor_title = non_empty_or(&mv.variables[row_fac_col].title, "Outcome / Exposure");
    cont.column_factor_title = non_empty_or(&mv.variables[col_fac_col].title, "Group");
    cont.row_titles = row_titles;

    for (ci, col_lbl) in col_titles.into_iter().enumerate() {
        cont.data_sets[ci].title = col_lbl;
        for ri in 0..cont.row_count() {
            let val = counts.get(&(ri, ci)).copied().unwrap_or(0);
            #[allow(clippy::cast_precision_loss)]
            {
                set_dataset_cell(&mut cont.data_sets, ci, ri, 0, Cell::number(val as f64));
            }
            acc.preserved += 1;
        }
    }
    cont.validate_counts()?;
    acc.warn("Cross-tabulated categorical MultipleVariables factors into Contingency counts");
    Ok(acc.finish(TableArchetype::Contingency(cont)))
}

// ============================================================================
// Shared cell/title/X helpers
// ============================================================================

fn xy_row_count(xy: &XyTable) -> usize {
    column_row_count(&xy.data_sets, xy.x_cells.len().max(xy.row_titles.len()))
}

fn mv_row_count(mv: &MultipleVariablesTable) -> usize {
    mv.variables.iter().map(|v| v.cells.len()).max().unwrap_or(mv.row_titles.len()).max(mv.row_titles.len())
}

fn count_var_occupied(var: &VariableColumn, rows: usize) -> usize {
    var.cells.iter().take(rows).filter(|c| is_occupied(c)).count()
}

fn count_mv_row_occupied(mv: &MultipleVariablesTable, cols: impl IntoIterator<Item = usize>, row: usize) -> usize {
    cols.into_iter().filter(|&c| mv.variables[c].cells.get(row).is_some_and(is_occupied)).count()
}

fn fill_mv_label_row_titles(
    mv: &MultipleVariablesTable,
    label_col: usize,
    row_titles: &mut [String],
    acc: &mut ConversionAcc,
) -> Result<(), TransformError> {
    let mut dropped_labels = 0_usize;
    for (r, slot) in row_titles.iter_mut().enumerate() {
        let cell = mv.variables[label_col].cells.get(r);
        if let Some(tok) = mv_cell_level_token(cell) {
            validate_title(&tok)?;
            *slot = tok;
            acc.preserved += 1;
        } else if cell.is_some_and(is_occupied) {
            acc.drop_cells(1);
            dropped_labels += 1;
        }
    }
    if dropped_labels > 0 {
        acc.warn(format!("Dropped {dropped_labels} excluded or non-representable label cell(s) when building row titles"));
    }
    Ok(())
}

struct XyRowTitles {
    titles: Vec<String>,
    spacing: Vec<Option<f64>>,
    x_used_as_label: bool,
}

fn build_xy_row_titles(xy: &XyTable, rows: usize) -> Result<XyRowTitles, TransformError> {
    let mut titles = Vec::with_capacity(rows);
    let mut spacing = Vec::with_capacity(rows);
    let mut x_used_as_label = false;
    for r in 0..rows {
        let x_val = xy_x_value_at_row(xy, r);
        spacing.push(x_val);
        let explicit = xy.row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty());
        let lbl = match (explicit, x_val) {
            (Some(s), _) => s.to_owned(),
            (None, Some(v)) => {
                x_used_as_label = true;
                format_normalized_f64(v)
            }
            (None, None) => String::new(),
        };
        validate_title(&lbl)?;
        titles.push(lbl);
    }
    Ok(XyRowTitles { titles, spacing, x_used_as_label })
}

fn normalize_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

fn format_normalized_f64(v: f64) -> String {
    let norm = normalize_zero(v);
    format!("{norm}")
}

fn mv_cell_level_token(cell: Option<&Cell>) -> Option<String> {
    let cell = cell?;
    if cell.excluded {
        return None;
    }
    match &cell.value {
        CellValue::Empty => None,
        CellValue::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CellValue::Number(v) if v.is_finite() => Some(format_normalized_f64(*v)),
        CellValue::Number(_) => None,
    }
}

fn resized_titles(src: &[String], rows: usize) -> Vec<String> {
    let mut out: Vec<String> = src.iter().take(rows).cloned().collect();
    if out.len() < rows {
        out.resize(rows, String::new());
    }
    out
}

fn non_empty_or(text: &str, fallback: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() { fallback.to_owned() } else { trimmed.to_owned() }
}

fn set_dataset_cell(data_sets: &mut [DataSetColumn], col: usize, row: usize, subcol: usize, cell: Cell) {
    if let Some(slot) = data_sets.get_mut(col).and_then(|ds| ds.cells.get_mut(row)).and_then(|r| r.get_mut(subcol)) {
        *slot = cell;
    }
}

fn set_mv_cell(mv: &mut MultipleVariablesTable, col: usize, row: usize, cell: Cell) {
    if let Some(slot) = mv.variables.get_mut(col).and_then(|v| v.cells.get_mut(row)) {
        *slot = cell;
    }
}

fn set_x_cell(xy: &mut XyTable, row: usize, subcol: usize, cell: Cell) {
    if let Some(slot) = xy.x_cells.get_mut(row).and_then(|r| r.get_mut(subcol)) {
        *slot = cell;
    }
}

fn count_excluded_generated_x(xy: &XyTable, rows: usize) -> usize {
    (0..rows).filter(|&r| xy.x_cells.get(r).and_then(|s| s.first()).is_some_and(|c| c.excluded)).count()
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn xy_materialized_x_cell(xy: &XyTable, row: usize) -> Cell {
    match &xy.x_mode {
        XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => {
            xy.x_cells.get(row).and_then(|subs| subs.first()).cloned().unwrap_or_default()
        }
        XColumnMode::GeneratedSequence { start, step, progression } => {
            let excluded = xy.x_cells.get(row).and_then(|subs| subs.first()).is_some_and(|c| c.excluded);
            let val = match progression {
                SequenceProgression::Arithmetic => *start + (row as f64) * *step,
                SequenceProgression::Geometric => *start * step.powi(row.min(i32::MAX as usize) as i32),
            };
            if val.is_finite() { Cell { value: CellValue::Number(val), excluded } } else { Cell { value: CellValue::Empty, excluded } }
        }
    }
}

fn xy_x_value_at_row(xy: &XyTable, row: usize) -> Option<f64> {
    xy_materialized_x_cell(xy, row).active_number()
}

fn parse_numeric_row_spacing(levels: &[String]) -> Option<Vec<Option<f64>>> {
    if levels.is_empty() {
        return None;
    }
    let mut parsed = Vec::with_capacity(levels.len());
    for lbl in levels {
        let num = lbl.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
        parsed.push(Some(normalize_zero(num)));
    }
    Some(parsed)
}

fn validate_title_slice(titles: &[String]) -> Result<(), TransformError> {
    for t in titles {
        validate_title(t)?;
    }
    Ok(())
}

#[cfg(test)]
mod audit_w1_convert_tests {
    use curvebook_doc::{
        Cell, CellCoord, DataSheet, DocError, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion,
        MultipleVariablesTable, SequenceProgression, SheetCellRange, SubcolumnMode, TableArchetype, ThreeWayColumnGrouping, VariableRole,
        XColumnMode, XyTable,
    };

    use super::{TargetArchetypeKind, convert_datasheet_archetype, preview_datasheet_conversion};
    use crate::error::TransformError;

    #[test]
    fn aud_04_11_generated_sequence_x_exclusions_preserved_or_flagged_lossy() {
        let mut xy = XyTable::new_with_x_mode(
            XColumnMode::GeneratedSequence { start: 0.0, step: 10.0, progression: SequenceProgression::Arithmetic },
            SubcolumnMode::Single,
            1,
            3,
        )
        .unwrap();
        xy.data_sets[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)], vec![Cell::number(3.0)]];
        let mut sheet = DataSheet::new("xy-gen-excl", "GenSeq", TableArchetype::Xy(xy)).unwrap();
        sheet.set_x_or_time_excluded(1, 0, true).unwrap();

        // Xy -> Column preserves excluded=true on row 1 X cell.
        let (col_sheet, col_prev) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::Column, false).unwrap();
        assert!(!col_prev.lossy);
        assert!(col_sheet.cell(CellCoord::new(1, 0, 0)).unwrap().excluded);

        // Xy -> MultipleVariables preserves excluded=true on row 1 X cell.
        let (mv_sheet, mv_prev) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
        assert!(!mv_prev.lossy);
        assert!(mv_sheet.cell(CellCoord::new(1, 0, 0)).unwrap().excluded);

        // Xy -> Grouped and Xy -> Contingency cannot retain excluded X flags: must flag lossy & refuse allow_lossy=false.
        for target in [TargetArchetypeKind::Grouped, TargetArchetypeKind::Contingency] {
            let prev = preview_datasheet_conversion(&sheet, target).unwrap();
            assert!(prev.lossy && prev.dropped_cells >= 1 && !prev.warnings.is_empty(), "{target:?}: {prev:?}");
            assert!(matches!(convert_datasheet_archetype(&sheet, target, false), Err(TransformError::InvalidSpec { .. })));
        }
    }

    #[test]
    fn aud_04_29_crosstab_rejects_above_2_to_53_and_sum_overflow() {
        let mut mv = MultipleVariablesTable::new(3, 2).unwrap();
        mv.variables[0].title = String::from("RowFac");
        mv.variables[0].lock_role(VariableRole::Categorical);
        mv.variables[0].cells = vec![Cell::text("A"), Cell::text("A")];
        mv.variables[1].title = String::from("ColFac");
        mv.variables[1].lock_role(VariableRole::Categorical);
        mv.variables[1].cells = vec![Cell::text("X"), Cell::text("X")];
        mv.variables[2].title = String::from("Count");
        mv.variables[2].cells = vec![Cell::number(9_007_199_254_740_992.0), Cell::number(1.0)];
        let s_sum = DataSheet::new("mv-sum-ovf", "SumOvf", TableArchetype::MultipleVariables(mv.clone())).unwrap();
        assert!(matches!(convert_datasheet_archetype(&s_sum, TargetArchetypeKind::Contingency, true), Err(TransformError::InvalidInput { .. })));

        // 2^64 must also be rejected cleanly as invalid count cell value.
        mv.variables[2].cells = vec![Cell::number(18_446_744_073_709_551_616.0), Cell::number(0.0)];
        let s_big = DataSheet::new("mv-big-cnt", "BigCnt", TableArchetype::MultipleVariables(mv)).unwrap();
        assert!(matches!(
            convert_datasheet_archetype(&s_big, TargetArchetypeKind::Contingency, true),
            Err(TransformError::Doc(DocError::InvalidCellValue { .. }))
        ));
    }

    #[test]
    fn aud_04_33_xy_reps_blank_title_uses_fallback_and_locks_group_categorical() {
        let mut xy =
            XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
        xy.data_sets[0].title = String::new();
        xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
        xy.data_sets[0].cells = vec![vec![Cell::number(10.0), Cell::number(11.0)], vec![Cell::number(20.0), Cell::number(21.0)]];
        let s = DataSheet::new("xy-blank", "BlankTitle", TableArchetype::Xy(xy)).unwrap();
        let (mv_s, prev) = convert_datasheet_archetype(&s, TargetArchetypeKind::MultipleVariables, false).unwrap();
        assert!(!prev.lossy);
        let TableArchetype::MultipleVariables(mv) = &mv_s.archetype else { panic!("expected MV") };
        assert_eq!((mv.variables[1].role, mv.variables[1].role_inferred), (VariableRole::Categorical, false));
        assert!(mv.variables[1].cells.iter().all(|c| c.active_text() == Some("A")));

        let (xy_rt, rt_prev) = convert_datasheet_archetype(&mv_s, TargetArchetypeKind::Xy, false).unwrap();
        assert!(!rt_prev.lossy);
        let TableArchetype::Xy(rt) = &xy_rt.archetype else { panic!("expected Xy") };
        assert_eq!(rt.active_xy_pairs(0), vec![(1.0, 10.0), (1.0, 11.0), (2.0, 20.0), (2.0, 21.0)]);
    }

    #[test]
    fn aud_04_34_grouped_three_way_and_spacing_loss_flagged_lossy() {
        let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 4, 2).unwrap();
        grp.row_titles = vec![String::from("Low"), String::from("High")];
        grp.row_numeric_spacing = Some(vec![Some(1.0), Some(10.0)]);
        grp.three_way_grouping = Some(ThreeWayColumnGrouping {
            factor_1_name: String::from("Drug"),
            factor_1_levels: [String::from("Vehicle"), String::from("Active")],
            factor_2_name: String::from("Sex"),
            factor_2_levels: [String::from("F"), String::from("M")],
        });
        for c in 0..4 {
            grp.data_sets[c].cells = vec![vec![Cell::number(5.0)], vec![Cell::number(6.0)]];
        }
        let s = DataSheet::new("grp-3way", "3Way", TableArchetype::Grouped(grp)).unwrap();
        for target in [TargetArchetypeKind::Column, TargetArchetypeKind::Xy, TargetArchetypeKind::MultipleVariables, TargetArchetypeKind::Contingency]
        {
            let prev = preview_datasheet_conversion(&s, target).unwrap();
            assert!(prev.lossy && !prev.warnings.is_empty(), "{target:?}: {prev:?}");
            assert!(matches!(convert_datasheet_archetype(&s, target, false), Err(TransformError::InvalidSpec { .. })));
        }
    }

    #[test]
    fn aud_04_35_and_44_mv_xy_text_y_consistency_warnings_and_duplicate_rep_ordering() {
        // AUD-04-35 part 1: direct MV -> XY preserves text in Y (matching Column/Grouped/pivoted -> XY).
        let mut mv = MultipleVariablesTable::new(2, 2).unwrap();
        mv.variables[0].title = String::from("Dose");
        mv.variables[0].cells = vec![Cell::number(1.0), Cell::number(2.0)];
        mv.variables[1].title = String::from("Response");
        mv.variables[1].lock_role(VariableRole::Continuous);
        mv.variables[1].cells = vec![Cell::number(10.0), Cell::text("BLQ")];
        let s = DataSheet::new("mv-blq", "BLQ", TableArchetype::MultipleVariables(mv)).unwrap();
        let (xy_s, prev) = convert_datasheet_archetype(&s, TargetArchetypeKind::Xy, false).unwrap();
        assert!(!prev.lossy);
        let TableArchetype::Xy(xy) = &xy_s.archetype else { panic!("expected Xy") };
        assert_eq!(xy.data_sets[0].cells[1][0].active_text(), Some("BLQ"));

        // AUD-04-35 part 3: GeneratedSequence -> Contingency marks lossy=true.
        let mut xy_gen = XyTable::new_with_x_mode(
            XColumnMode::GeneratedSequence { start: 1.0, step: 2.0, progression: SequenceProgression::Arithmetic },
            SubcolumnMode::Single,
            1,
            2,
        )
        .unwrap();
        xy_gen.data_sets[0].cells = vec![vec![Cell::number(4.0)], vec![Cell::number(9.0)]];
        let s_gen = DataSheet::new("xy-gen-cnt", "GenCnt", TableArchetype::Xy(xy_gen)).unwrap();
        let gen_prev = preview_datasheet_conversion(&s_gen, TargetArchetypeKind::Contingency).unwrap();
        assert!(gen_prev.lossy && !gen_prev.warnings.is_empty());

        // AUD-04-44 part 2: duplicate Replicate=2 rows place into Y2 then Y3 (never reordering into Y1 ahead of Y2).
        let mut mv_dup = MultipleVariablesTable::new(4, 2).unwrap();
        mv_dup.variables[0].title = String::from("Dose");
        mv_dup.variables[0].lock_role(VariableRole::Categorical);
        mv_dup.variables[0].cells = vec![Cell::text("1"), Cell::text("1")];
        mv_dup.variables[1].title = String::from("Arm");
        mv_dup.variables[1].lock_role(VariableRole::Categorical);
        mv_dup.variables[1].cells = vec![Cell::text("A"), Cell::text("A")];
        mv_dup.variables[2].title = String::from("Replicate");
        mv_dup.variables[2].cells = vec![Cell::number(2.0), Cell::number(2.0)];
        mv_dup.variables[3].title = String::from("Value");
        mv_dup.variables[3].cells = vec![Cell::number(100.0), Cell::number(200.0)];
        let s_dup = DataSheet::new("mv-dup-rep", "DupRep", TableArchetype::MultipleVariables(mv_dup)).unwrap();
        let (grp_dup, dup_prev) = convert_datasheet_archetype(&s_dup, TargetArchetypeKind::Grouped, false).unwrap();
        let TableArchetype::Grouped(g) = &grp_dup.archetype else { panic!("expected Grouped") };
        assert!(g.data_sets[0].cells[0][0].is_empty(), "Y1 must remain empty");
        assert_eq!(g.data_sets[0].cells[0][1].active_number(), Some(100.0));
        assert_eq!(g.data_sets[0].cells[0][2].active_number(), Some(200.0));
        assert!(dup_prev.warnings.iter().any(|w| w.contains("collision")));

        // AUD-04-44 part 3: linked_regions emit an informational warning without forcing lossy.
        let mut s_linked = s_dup;
        s_linked.linked_regions.push(LinkedDataRegion {
            id: String::from("l1"),
            spec: ImportLinkSpec { source: LinkSource::Clipboard, filter: ImportFilterSpec::default(), placement: ImportPlacementSpec::default() },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            provenance_summary: String::from("test"),
        });
        let lnk_prev = preview_datasheet_conversion(&s_linked, TargetArchetypeKind::Grouped).unwrap();
        assert!(!lnk_prev.lossy);
        assert!(lnk_prev.warnings.iter().any(|w| w.contains("linked data region")));
    }

    #[test]
    fn aud_04_22_grouped_and_xy_to_mv_reject_over_max_rows_before_allocating_tuples() {
        // 500 rows x 10 cols x 250 reps = 1,250,000 output rows > MAX_ROWS (1,000,000) while valid input table.
        let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(250), 10, 500).unwrap();
        for ds in &mut grp.data_sets {
            for r in 0..500 {
                ds.cells[r][0] = Cell::number(1.0);
            }
        }
        let s_grp = DataSheet::new("grp-big-mv", "BigGrp", TableArchetype::Grouped(grp)).unwrap();
        assert!(matches!(
            preview_datasheet_conversion(&s_grp, TargetArchetypeKind::MultipleVariables),
            Err(TransformError::Doc(DocError::TooManyRows { .. }))
        ));

        let mut xy =
            XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(250), 10, 500)
                .unwrap();
        for ds in &mut xy.data_sets {
            for r in 0..500 {
                ds.cells[r][0] = Cell::number(1.0);
            }
        }
        let s_xy = DataSheet::new("xy-big-mv", "BigXy", TableArchetype::Xy(xy)).unwrap();
        assert!(matches!(
            preview_datasheet_conversion(&s_xy, TargetArchetypeKind::MultipleVariables),
            Err(TransformError::Doc(DocError::TooManyRows { .. }))
        ));
    }

    #[test]
    fn aud_04_35_all_drop_paths_emit_non_empty_warnings() {
        // Unstack MV -> Column with missing group token drops cell and emits warning.
        let mut mv = MultipleVariablesTable::new(2, 2).unwrap();
        mv.variables[0].title = String::from("Group");
        mv.variables[0].lock_role(VariableRole::Categorical);
        mv.variables[0].cells = vec![Cell::text("Ctrl"), Cell::empty()];
        mv.variables[1].title = String::from("Val");
        mv.variables[1].cells = vec![Cell::number(1.0), Cell::number(2.0)];
        let s = DataSheet::new("mv-drop-warn", "DropWarn", TableArchetype::MultipleVariables(mv)).unwrap();
        let prev = preview_datasheet_conversion(&s, TargetArchetypeKind::Column).unwrap();
        assert!(prev.lossy && prev.dropped_cells >= 1 && !prev.warnings.is_empty());

        // Direct MV -> XY with non-numeric text in X drops text and emits warning.
        let mut mv_xy = MultipleVariablesTable::new(2, 2).unwrap();
        mv_xy.variables[0].title = String::from("X");
        mv_xy.variables[0].lock_role(VariableRole::Continuous);
        mv_xy.variables[0].cells = vec![Cell::number(1.0), Cell::text("bad_x")];
        mv_xy.variables[1].title = String::from("Y");
        mv_xy.variables[1].cells = vec![Cell::number(10.0), Cell::number(20.0)];
        let s_xy = DataSheet::new("mv-xy-warn", "XyWarn", TableArchetype::MultipleVariables(mv_xy)).unwrap();
        let prev_xy = preview_datasheet_conversion(&s_xy, TargetArchetypeKind::Xy).unwrap();
        assert!(prev_xy.lossy && prev_xy.dropped_cells == 1 && !prev_xy.warnings.is_empty());

        // GeneratedSequence -> Contingency with explicit row titles on every row still flags lossy=true.
        let mut xy_gen_titled = XyTable::new_with_x_mode(
            XColumnMode::GeneratedSequence { start: 1.0, step: 2.0, progression: SequenceProgression::Arithmetic },
            SubcolumnMode::Single,
            1,
            2,
        )
        .unwrap();
        xy_gen_titled.row_titles = vec![String::from("R1"), String::from("R2")];
        xy_gen_titled.data_sets[0].cells = vec![vec![Cell::number(4.0)], vec![Cell::number(9.0)]];
        let s_gen_titled = DataSheet::new("xy-gen-titled", "GenTitled", TableArchetype::Xy(xy_gen_titled)).unwrap();
        let gen_titled_prev = preview_datasheet_conversion(&s_gen_titled, TargetArchetypeKind::Contingency).unwrap();
        assert!(gen_titled_prev.lossy && !gen_titled_prev.warnings.is_empty());
    }

    #[test]
    fn aud_04_11_and_22_overflowing_excluded_generated_sequence_x_with_replicates_no_panic() {
        let mut xy = XyTable::new_with_x_mode(
            XColumnMode::GeneratedSequence { start: 1e308, step: 10.0, progression: SequenceProgression::Geometric },
            SubcolumnMode::RawReplicates(2),
            1,
            3,
        )
        .unwrap();
        xy.row_titles = vec![String::from("R0"), String::from("R1"), String::from("R2")];
        // Row 0: finite X + occupied Y; Row 1: overflowing excluded X + occupied Y; Row 2: overflowing excluded X-only.
        xy.data_sets[0].cells[0] = vec![Cell::number(1.0), Cell::number(2.0)];
        xy.data_sets[0].cells[1] = vec![Cell::number(10.0), Cell::number(20.0)];
        let mut sheet = DataSheet::new("xy-gen-ovf", "GenOvf", TableArchetype::Xy(xy)).unwrap();
        sheet.set_x_or_time_excluded(1, 0, true).unwrap();
        sheet.set_x_or_time_excluded(2, 0, true).unwrap();

        let (mv_sheet, prev) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
        assert!(!prev.lossy);
        let TableArchetype::MultipleVariables(mv) = &mv_sheet.archetype else { panic!("expected MV") };
        // 2 paired rows * 2 reps + 1 unpaired excluded X-only row = 5 rows.
        assert_eq!(mv.row_count(), 5);
        assert!(mv.variables[0].cells[2].excluded, "overflowing generated X on paired row must keep excluded=true");
        assert!(mv.variables[0].cells[3].excluded);
        assert!(mv.variables[0].cells[4].excluded, "unpaired overflowing excluded X row must keep excluded=true");
        assert_eq!(mv.row_titles[4], "R2");
    }

    #[test]
    fn aud_04_44_duplicate_replicate_collisions_fail_fast_at_max_subcolumns() {
        let n = curvebook_doc::MAX_SUBCOLUMNS + 2_000;
        let mut mv = MultipleVariablesTable::new(4, n).unwrap();
        mv.variables[0].title = String::from("Dose");
        mv.variables[0].lock_role(VariableRole::Categorical);
        mv.variables[0].cells = vec![Cell::text("1"); n];
        mv.variables[1].title = String::from("Arm");
        mv.variables[1].lock_role(VariableRole::Categorical);
        mv.variables[1].cells = vec![Cell::text("A"); n];
        mv.variables[2].title = String::from("Replicate");
        mv.variables[2].cells = vec![Cell::number(1.0); n];
        mv.variables[3].title = String::from("Value");
        mv.variables[3].cells = vec![Cell::number(5.0); n];
        let s = DataSheet::new("mv-dup-ovf", "DupOvf", TableArchetype::MultipleVariables(mv)).unwrap();
        let err = preview_datasheet_conversion(&s, TargetArchetypeKind::Grouped).unwrap_err();
        assert!(matches!(err, TransformError::Doc(DocError::TooManySubcolumns { .. })), "got {err:?}");
    }
}
