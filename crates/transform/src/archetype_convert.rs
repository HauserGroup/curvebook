//! Non-destructive table archetype preview and conversion (`QOL-1`).
//!
//! Supports lossless and lossy-guarded projections among `Column`, `Grouped`, `Xy`, and
//! `MultipleVariables` tables while preserving sheet metadata, cell `excluded` masks, and
//! remapping [`CellExclusionNote`] coordinates to their new [`CellCoord`] positions.

use std::collections::{BTreeMap, BTreeSet};

use lumencraft_doc::{
    Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, DataSetColumn, DataSheet, GroupedTable, MAX_DATA_SETS,
    MultipleVariablesTable, SubcolumnMode, TableArchetype, TableKind, VariableColumn, VariableRole, VariableSource, XColumnMode, XyTable,
    default_column_label, validate_data_sets, validate_rows, validate_subcolumns,
};

use crate::error::TransformError;

/// Target table archetype supported by non-destructive table conversion (`QOL-1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TargetArchetypeKind {
    Column,
    Grouped,
    Xy,
    MultipleVariables,
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
    pub source_kind: TableKind,
    pub target_kind: TargetArchetypeKind,
    pub preserved_cells: usize,
    pub dropped_cells: usize,
    pub preserved_exclusions: usize,
    pub warnings: Vec<String>,
    pub lossy: bool,
}

/// Inspect how converting `sheet` into `target` will map cells, exclusions, and subcolumns
/// without mutating `sheet`.
pub fn preview_datasheet_conversion(sheet: &DataSheet, target: TargetArchetypeKind) -> Result<ConversionPreview, TransformError> {
    let (_, preview) = build_converted_datasheet(sheet, target)?;
    Ok(preview)
}

/// Convert `sheet` into `target`, preserving metadata, cell `excluded` masks, and remapped
/// [`CellExclusionNote`] coordinates.
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

type RepBucketMap = BTreeMap<(usize, usize), Vec<(Option<f64>, usize, Cell)>>;

struct BuiltArchetype {
    archetype: TableArchetype,
    coord_map: BTreeMap<CellCoord, CellCoord>,
    preserved_cells: usize,
    dropped_cells: usize,
    warnings: Vec<String>,
    schema_lossy: bool,
    keep_links: bool,
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
        _ => return Err(TransformError::IncompatibleTable { operation: "convert_datasheet_archetype", kind: source_kind }),
    };

    assemble_datasheet(sheet, target, built)
}

fn build_identity_result(sheet: &DataSheet, target: TargetArchetypeKind) -> (DataSheet, ConversionPreview) {
    let preserved_cells = count_occupied_cells(&sheet.archetype);
    let preserved_exclusions = count_excluded_cells(&sheet.archetype);
    let preview = ConversionPreview {
        source_kind: sheet.kind(),
        target_kind: target,
        preserved_cells,
        dropped_cells: 0,
        preserved_exclusions,
        warnings: Vec::new(),
        lossy: false,
    };
    (sheet.clone(), preview)
}

fn assemble_datasheet(
    source: &DataSheet,
    target: TargetArchetypeKind,
    mut built: BuiltArchetype,
) -> Result<(DataSheet, ConversionPreview), TransformError> {
    let mut out = DataSheet::new(source.id.clone(), source.title.clone(), built.archetype)?;
    out.frozen = source.frozen;
    out.info_sheet_id = source.info_sheet_id.clone();
    out.floating_notes = source.floating_notes.clone();
    out.color_tag = source.color_tag;
    if built.keep_links {
        out.linked_regions = source.linked_regions.clone();
    }

    let mut dropped_notes = 0_usize;
    for note in &source.exclusion_notes {
        if let Some(&new_coord) = built.coord_map.get(&note.coord) {
            out.exclusion_notes.push(CellExclusionNote {
                coord: new_coord,
                reason_tag: note.reason_tag.clone(),
                note: note.note.clone(),
                timestamp_utc: note.timestamp_utc.clone(),
            });
        } else {
            dropped_notes += 1;
            built.warnings.push(format!(
                "Exclusion note at (row={}, col={}, subcol={}) could not be remapped to target data cells",
                note.coord.row, note.coord.col, note.coord.subcol
            ));
        }
    }
    out.exclusion_notes.sort_by_key(|n| n.coord);

    let preserved_exclusions = count_excluded_cells(&out.archetype);
    let lossy = built.dropped_cells > 0 || dropped_notes > 0 || built.schema_lossy;
    let preview = ConversionPreview {
        source_kind: source.kind(),
        target_kind: target,
        preserved_cells: built.preserved_cells,
        dropped_cells: built.dropped_cells,
        preserved_exclusions,
        warnings: built.warnings,
        lossy,
    };
    Ok((out, preview))
}

fn is_occupied(cell: &Cell) -> bool {
    !cell.is_empty() || cell.excluded
}

fn count_occupied_cells(arch: &TableArchetype) -> usize {
    let data_count: usize = match arch {
        TableArchetype::Xy(t) => count_dataset_occupied(&t.data_sets) + count_grid_occupied(&t.x_cells),
        TableArchetype::Column(t) => count_dataset_occupied(&t.data_sets),
        TableArchetype::Grouped(t) => count_dataset_occupied(&t.data_sets),
        TableArchetype::Contingency(t) => count_dataset_occupied(&t.data_sets),
        TableArchetype::Survival(t) => t.times.iter().filter(|c| is_occupied(c)).count() + count_dataset_occupied(&t.cohorts),
        TableArchetype::PartsOfWhole(t) => count_dataset_occupied(&t.data_sets),
        TableArchetype::MultipleVariables(mv) => mv.variables.iter().map(|v| v.cells.iter().filter(|c| is_occupied(c)).count()).sum(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| count_grid_occupied(&g.cells)).sum(),
    };
    data_count
}

fn count_dataset_occupied(cols: &[DataSetColumn]) -> usize {
    cols.iter().map(|ds| count_grid_occupied(&ds.cells)).sum()
}

fn count_grid_occupied(grid: &[Vec<Cell>]) -> usize {
    grid.iter().map(|row| row.iter().filter(|c| is_occupied(c)).count()).sum()
}

fn count_excluded_cells(arch: &TableArchetype) -> usize {
    match arch {
        TableArchetype::Xy(t) => count_dataset_excluded(&t.data_sets) + count_grid_excluded(&t.x_cells),
        TableArchetype::Column(t) => count_dataset_excluded(&t.data_sets),
        TableArchetype::Grouped(t) => count_dataset_excluded(&t.data_sets),
        TableArchetype::Contingency(t) => count_dataset_excluded(&t.data_sets),
        TableArchetype::Survival(t) => t.times.iter().filter(|c| c.excluded).count() + count_dataset_excluded(&t.cohorts),
        TableArchetype::PartsOfWhole(t) => count_dataset_excluded(&t.data_sets),
        TableArchetype::MultipleVariables(mv) => mv.variables.iter().map(|v| v.cells.iter().filter(|c| c.excluded).count()).sum(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| count_grid_excluded(&g.cells)).sum(),
    }
}

fn count_dataset_excluded(cols: &[DataSetColumn]) -> usize {
    cols.iter().map(|ds| count_grid_excluded(&ds.cells)).sum()
}

fn count_grid_excluded(grid: &[Vec<Cell>]) -> usize {
    grid.iter().map(|row| row.iter().filter(|c| c.excluded).count()).sum()
}

fn column_row_count(cols: &[DataSetColumn], row_titles_len: usize) -> usize {
    cols.iter().map(DataSetColumn::row_count).max().unwrap_or(row_titles_len).max(row_titles_len)
}

fn convert_column_to_grouped(col: &ColumnTable) -> Result<BuiltArchetype, TransformError> {
    if col.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("Column table has no data columns") });
    }
    let cols = col.data_sets.len();
    let rows = column_row_count(&col.data_sets, col.row_titles.len());
    let target_mode = if col.subcolumn_mode == SubcolumnMode::Single { SubcolumnMode::RawReplicates(1) } else { col.subcolumn_mode };
    let subs = target_mode.subcolumn_count();
    let mut grouped = GroupedTable::new(target_mode, cols, rows)?;
    grouped.row_titles = resized_titles(&col.row_titles, rows);

    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    for (c, src_ds) in col.data_sets.iter().enumerate() {
        grouped.data_sets[c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for s in 0..subs {
                if let Some(cell) = row_cells.get(s) {
                    grouped.data_sets[c].cells[r][s] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, c, s));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }

    let mut warnings = Vec::new();
    if col.design_hint == ColumnDesignHint::UnpairedReplicates {
        warnings.push(String::from("Unpaired Column rows mapped 1:1 to Grouped row levels"));
    }
    Ok(BuiltArchetype {
        archetype: TableArchetype::Grouped(grouped),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings,
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_grouped_to_column(grp: &GroupedTable) -> Result<BuiltArchetype, TransformError> {
    if grp.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("Grouped table has no data columns") });
    }
    let cols = grp.data_sets.len();
    let rows = column_row_count(&grp.data_sets, grp.row_titles.len());

    match grp.subcolumn_mode {
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(1) => convert_grouped_single_rep_to_column(grp, cols, rows),
        SubcolumnMode::RawReplicates(n) => convert_grouped_multi_rep_to_column(grp, cols, rows, n),
        SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN => convert_grouped_parametric_to_column(grp, cols, rows),
        SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
            convert_grouped_interval_to_column(grp, cols, rows)
        }
    }
}

fn convert_grouped_single_rep_to_column(grp: &GroupedTable, cols: usize, rows: usize) -> Result<BuiltArchetype, TransformError> {
    let hint = if grp.row_titles.iter().any(|s| !s.trim().is_empty()) {
        ColumnDesignHint::PairedRepeatedMeasures
    } else {
        ColumnDesignHint::UnpairedReplicates
    };
    let mut out = ColumnTable::new_with_subcolumns(hint, SubcolumnMode::Single, cols, rows)?;
    out.row_titles = resized_titles(&grp.row_titles, rows);
    out.row_titles_expanded = out.row_titles.iter().any(|s| !s.trim().is_empty());

    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    for (c, src_ds) in grp.data_sets.iter().enumerate() {
        out.data_sets[c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            if let Some(cell) = row_cells.first() {
                out.data_sets[c].cells[r][0] = cell.clone();
                coord_map.insert(CellCoord::new(r, c, 0), CellCoord::new(r, c, 0));
                if is_occupied(cell) {
                    preserved += 1;
                }
            }
        }
    }

    let mut warnings = Vec::new();
    if grp.three_way_grouping.is_some() || grp.row_numeric_spacing.is_some() {
        warnings.push(String::from("Grouped factor metadata flattened into Column row structure"));
    }
    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(out),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings,
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_grouped_multi_rep_to_column(grp: &GroupedTable, cols: usize, rows: usize, n: usize) -> Result<BuiltArchetype, TransformError> {
    if rows <= 1 {
        let mut out = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, cols, n)?;
        let mut coord_map = BTreeMap::new();
        let mut preserved = 0_usize;
        for (c, src_ds) in grp.data_sets.iter().enumerate() {
            out.data_sets[c].title = src_ds.title.clone();
            if let Some(row_cells) = src_ds.cells.first() {
                for s in 0..n {
                    if let Some(cell) = row_cells.get(s) {
                        out.data_sets[c].cells[s][0] = cell.clone();
                        coord_map.insert(CellCoord::new(0, c, s), CellCoord::new(s, c, 0));
                        if is_occupied(cell) {
                            preserved += 1;
                        }
                    }
                }
            }
        }
        return Ok(BuiltArchetype {
            archetype: TableArchetype::Column(out),
            coord_map,
            preserved_cells: preserved,
            dropped_cells: 0,
            warnings: vec![String::from("Grouped replicate subcolumns stacked vertically down Column rows")],
            schema_lossy: false,
            keep_links: false,
        });
    }

    if let Some(total_cols) = cols.checked_mul(rows)
        && total_cols <= MAX_DATA_SETS
    {
        let mut out = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, total_cols, n)?;
        let mut coord_map = BTreeMap::new();
        let mut preserved = 0_usize;
        for (c, src_ds) in grp.data_sets.iter().enumerate() {
            for r in 0..rows {
                let dst_col = c * rows + r;
                let row_lbl = grp.row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty());
                out.data_sets[dst_col].title = match row_lbl {
                    Some(lbl) => format!("{} - {lbl}", src_ds.title),
                    None => format!("{} - Row {}", src_ds.title, r + 1),
                };
                if let Some(row_cells) = src_ds.cells.get(r) {
                    for s in 0..n {
                        if let Some(cell) = row_cells.get(s) {
                            out.data_sets[dst_col].cells[s][0] = cell.clone();
                            coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(s, dst_col, 0));
                            if is_occupied(cell) {
                                preserved += 1;
                            }
                        }
                    }
                }
            }
        }
        return Ok(BuiltArchetype {
            archetype: TableArchetype::Column(out),
            coord_map,
            preserved_cells: preserved,
            dropped_cells: 0,
            warnings: vec![String::from("Grouped (Row x Column) cells expanded into Column groups with stacked replicates")],
            schema_lossy: false,
            keep_links: false,
        });
    }

    // Exceeds MAX_DATA_SETS if expanded; fall back to keeping subcolumn 0 only (lossy).
    let mut out = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, cols, rows)?;
    out.row_titles = resized_titles(&grp.row_titles, rows);
    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    let mut dropped = 0_usize;
    for (c, src_ds) in grp.data_sets.iter().enumerate() {
        out.data_sets[c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for (s, cell) in row_cells.iter().enumerate() {
                if s == 0 {
                    out.data_sets[c].cells[r][0] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, 0), CellCoord::new(r, c, 0));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                } else if is_occupied(cell) {
                    dropped += 1;
                }
            }
        }
    }
    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(out),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings: vec![format!("Expanded column count exceeds {MAX_DATA_SETS}; truncated replicate subcolumns beyond Y1")],
        schema_lossy: true,
        keep_links: false,
    })
}

fn convert_grouped_parametric_to_column(grp: &GroupedTable, cols: usize, rows: usize) -> Result<BuiltArchetype, TransformError> {
    let mut out = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, grp.subcolumn_mode, cols, rows)?;
    out.row_titles = resized_titles(&grp.row_titles, rows);
    out.row_titles_expanded = out.row_titles.iter().any(|s| !s.trim().is_empty());
    let subs = grp.subcolumn_mode.subcolumn_count();
    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    for (c, src_ds) in grp.data_sets.iter().enumerate() {
        out.data_sets[c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for s in 0..subs {
                if let Some(cell) = row_cells.get(s) {
                    out.data_sets[c].cells[r][s] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, c, s));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }
    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(out),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings: Vec::new(),
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_grouped_interval_to_column(grp: &GroupedTable, cols: usize, rows: usize) -> Result<BuiltArchetype, TransformError> {
    let mut out = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, cols, rows)?;
    out.row_titles = resized_titles(&grp.row_titles, rows);
    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    let mut dropped = 0_usize;
    for (c, src_ds) in grp.data_sets.iter().enumerate() {
        out.data_sets[c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for (s, cell) in row_cells.iter().enumerate() {
                if s == 0 {
                    out.data_sets[c].cells[r][0] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, 0), CellCoord::new(r, c, 0));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                } else if is_occupied(cell) {
                    dropped += 1;
                }
            }
        }
    }
    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(out),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings: vec![format!("Column tables do not support {:?} subcolumns; spread/limit subcolumns 1..3 will be dropped", grp.subcolumn_mode)],
        schema_lossy: true,
        keep_links: false,
    })
}

fn convert_column_to_xy(col: &ColumnTable) -> Result<BuiltArchetype, TransformError> {
    if col.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("Column table has no data columns") });
    }
    let cols = col.data_sets.len();
    let rows = column_row_count(&col.data_sets, col.row_titles.len());
    let subs = col.subcolumn_mode.subcolumn_count();

    if cols == 1 {
        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, col.subcolumn_mode, 1, rows)?;
        xy.row_titles = resized_titles(&col.row_titles, rows);
        xy.row_titles_expanded = col.row_titles_expanded;
        xy.data_sets[0].title = col.data_sets[0].title.clone();
        populate_generated_or_titled_x(&mut xy, &col.row_titles, rows);
        let mut coord_map = BTreeMap::new();
        let mut preserved = 0_usize;
        for (r, row_cells) in col.data_sets[0].cells.iter().enumerate() {
            for s in 0..subs {
                if let Some(cell) = row_cells.get(s) {
                    xy.data_sets[0].cells[r][s] = cell.clone();
                    coord_map.insert(CellCoord::new(r, 0, s), CellCoord::new(r, 0, s));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
        return Ok(BuiltArchetype {
            archetype: TableArchetype::Xy(xy),
            coord_map,
            preserved_cells: preserved,
            dropped_cells: 0,
            warnings: vec![String::from("Single Column series mapped to Y with sequential/parsed X coordinates")],
            schema_lossy: false,
            keep_links: false,
        });
    }

    let x_mode = XColumnMode::Numbers { subcolumn_mode: col.subcolumn_mode };
    let mut xy = XyTable::new_with_x_mode(x_mode, col.subcolumn_mode, cols - 1, rows)?;
    xy.x_title = col.data_sets[0].title.clone();
    xy.row_titles = resized_titles(&col.row_titles, rows);
    xy.row_titles_expanded = col.row_titles_expanded;

    let mut preserved = 0_usize;
    let mut dropped = 0_usize;
    let mut warnings = Vec::new();
    let mut text_in_x = 0_usize;

    for (r, row_cells) in col.data_sets[0].cells.iter().enumerate() {
        for s in 0..subs {
            if let Some(cell) = row_cells.get(s) {
                match &cell.value {
                    CellValue::Text(txt) if !txt.trim().is_empty() => {
                        text_in_x += 1;
                        dropped += 1;
                        xy.x_cells[r][s] = Cell { value: CellValue::Empty, excluded: cell.excluded };
                    }
                    _ => {
                        xy.x_cells[r][s] = cell.clone();
                        if is_occupied(cell) {
                            preserved += 1;
                        }
                    }
                }
            }
        }
    }
    if text_in_x > 0 {
        warnings.push(format!("Column 0 contains {text_in_x} non-numeric text cell(s) incompatible with numeric XY X coordinates"));
    }

    let mut coord_map = BTreeMap::new();
    for (c, src_ds) in col.data_sets.iter().enumerate().skip(1) {
        let dst_c = c - 1;
        xy.data_sets[dst_c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for s in 0..subs {
                if let Some(cell) = row_cells.get(s) {
                    xy.data_sets[dst_c].cells[r][s] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, dst_c, s));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Xy(xy),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings,
        schema_lossy: text_in_x > 0,
        keep_links: false,
    })
}

fn populate_generated_or_titled_x(xy: &mut XyTable, row_titles: &[String], rows: usize) {
    let parsed: Vec<Option<f64>> =
        (0..rows).map(|r| row_titles.get(r).and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite())).collect();
    let any_title_present = row_titles.iter().any(|s| !s.trim().is_empty());
    let all_numeric = any_title_present && row_titles.iter().enumerate().all(|(r, s)| s.trim().is_empty() || parsed[r].is_some());
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
    if xy.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("XY table has no Y data columns") });
    }
    let rows = xy.row_count();
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

    // Single Y subcolumns.
    let x_subs = xy.x_mode.subcolumn_count();
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PairedRepeatedMeasures, SubcolumnMode::Single, 1 + y_cols, rows)?;
    col.row_titles = resized_titles(&xy.row_titles, rows);
    col.row_titles_expanded = xy.row_titles_expanded;
    col.data_sets[0].title = xy.x_title.clone();

    let mut preserved = 0_usize;
    let mut dropped = 0_usize;
    let mut warnings = Vec::new();
    let mut schema_lossy = false;

    copy_xy_x_into_column0(xy, &mut col, rows, &mut preserved, &mut dropped, &mut warnings);
    if x_subs > 1 {
        schema_lossy = true;
    }

    let mut coord_map = BTreeMap::new();
    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        let dst_c = c + 1;
        col.data_sets[dst_c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            if let Some(cell) = row_cells.first() {
                col.data_sets[dst_c].cells[r][0] = cell.clone();
                coord_map.insert(CellCoord::new(r, c, 0), CellCoord::new(r, dst_c, 0));
                if is_occupied(cell) {
                    preserved += 1;
                }
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(col),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn convert_xy_replicates_to_column(xy: &XyTable, rows: usize, y_cols: usize, n: usize) -> Result<BuiltArchetype, TransformError> {
    let needed = y_cols.checked_mul(n).and_then(|v| v.checked_add(1));
    if needed.is_none_or(|tot| tot > MAX_DATA_SETS) {
        return convert_xy_lossy_single_column(xy, rows, y_cols);
    }
    let total_cols = 1 + y_cols * n;
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PairedRepeatedMeasures, SubcolumnMode::Single, total_cols, rows)?;
    col.row_titles = resized_titles(&xy.row_titles, rows);
    col.row_titles_expanded = xy.row_titles_expanded;
    col.data_sets[0].title = xy.x_title.clone();

    let mut preserved = 0_usize;
    let mut dropped = 0_usize;
    let mut warnings = vec![String::from("XY replicate subcolumns expanded into individual Column columns")];
    let schema_lossy = xy.x_mode.subcolumn_count() > 1;
    copy_xy_x_into_column0(xy, &mut col, rows, &mut preserved, &mut dropped, &mut warnings);

    let mut coord_map = BTreeMap::new();
    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        for s in 0..n {
            let dst_c = 1 + c * n + s;
            col.data_sets[dst_c].title = format!("{} (Y{})", src_ds.title, s + 1);
            for (r, row_cells) in src_ds.cells.iter().enumerate() {
                if let Some(cell) = row_cells.get(s) {
                    col.data_sets[dst_c].cells[r][0] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, dst_c, 0));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(col),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn convert_xy_parametric_to_column(xy: &XyTable, rows: usize, y_cols: usize) -> Result<BuiltArchetype, TransformError> {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, xy.y_subcolumn_mode, 1 + y_cols, rows)?;
    col.row_titles = resized_titles(&xy.row_titles, rows);
    col.row_titles_expanded = xy.row_titles_expanded;
    col.data_sets[0].title = xy.x_title.clone();

    let mut preserved = 0_usize;
    let mut dropped = 0_usize;
    let mut warnings = Vec::new();
    let mut schema_lossy = false;

    let y_subs = xy.y_subcolumn_mode.subcolumn_count();
    match &xy.x_mode {
        XColumnMode::Numbers { subcolumn_mode } if *subcolumn_mode == xy.y_subcolumn_mode => {
            for (r, x_row) in xy.x_cells.iter().enumerate().take(rows) {
                for s in 0..y_subs {
                    if let Some(cell) = x_row.get(s) {
                        col.data_sets[0].cells[r][s] = cell.clone();
                        if is_occupied(cell) {
                            preserved += 1;
                        }
                    }
                }
            }
        }
        _ => {
            if xy.x_mode.subcolumn_count() > 1 {
                schema_lossy = true;
            }
            copy_xy_x_into_column0(xy, &mut col, rows, &mut preserved, &mut dropped, &mut warnings);
            if xy.x_mode.subcolumn_count() <= 1 {
                warnings.push(String::from("X values placed into Column 0 subcolumn 0 without SD/N entries"));
            }
        }
    }

    let mut coord_map = BTreeMap::new();
    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        let dst_c = c + 1;
        col.data_sets[dst_c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for s in 0..y_subs {
                if let Some(cell) = row_cells.get(s) {
                    col.data_sets[dst_c].cells[r][s] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, dst_c, s));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(col),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn convert_xy_lossy_single_column(xy: &XyTable, rows: usize, y_cols: usize) -> Result<BuiltArchetype, TransformError> {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PairedRepeatedMeasures, SubcolumnMode::Single, 1 + y_cols, rows)?;
    col.row_titles = resized_titles(&xy.row_titles, rows);
    col.row_titles_expanded = xy.row_titles_expanded;
    col.data_sets[0].title = xy.x_title.clone();

    let mut preserved = 0_usize;
    let mut dropped = 0_usize;
    let mut warnings = vec![format!(
        "Column tables do not support {:?} Y subcolumns; spread/limit subcolumns beyond subcolumn 0 will be dropped",
        xy.y_subcolumn_mode
    )];
    copy_xy_x_into_column0(xy, &mut col, rows, &mut preserved, &mut dropped, &mut warnings);

    let mut coord_map = BTreeMap::new();
    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        let dst_c = c + 1;
        col.data_sets[dst_c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for (s, cell) in row_cells.iter().enumerate() {
                if s == 0 {
                    col.data_sets[dst_c].cells[r][0] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, 0), CellCoord::new(r, dst_c, 0));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                } else if is_occupied(cell) {
                    dropped += 1;
                }
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(col),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings,
        schema_lossy: true,
        keep_links: false,
    })
}

fn copy_xy_x_into_column0(xy: &XyTable, col: &mut ColumnTable, rows: usize, preserved: &mut usize, dropped: &mut usize, warnings: &mut Vec<String>) {
    match &xy.x_mode {
        XColumnMode::GeneratedSequence { .. } => {
            for r in 0..rows {
                col.data_sets[0].cells[r][0] = xy.x_value_at(r).map_or_else(Cell::empty, Cell::number);
            }
            warnings.push(String::from("Generated X sequence materialized into static Column 0 values"));
        }
        XColumnMode::DateOrElapsedTime { .. } | XColumnMode::Numbers { .. } => {
            let mut dropped_x_subs = 0_usize;
            for (r, x_row) in xy.x_cells.iter().enumerate().take(rows) {
                for (s, cell) in x_row.iter().enumerate() {
                    if s == 0 {
                        col.data_sets[0].cells[r][0] = cell.clone();
                        if is_occupied(cell) {
                            *preserved += 1;
                        }
                    } else if is_occupied(cell) {
                        *dropped += 1;
                        dropped_x_subs += 1;
                    }
                }
            }
            if dropped_x_subs > 0 {
                warnings.push(format!("Dropped {dropped_x_subs} X error subcolumn cell(s) unsupported by Column tables"));
            }
        }
    }
}

fn convert_grouped_to_mv(grp: &GroupedTable) -> Result<BuiltArchetype, TransformError> {
    if grp.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("Grouped table has no data columns") });
    }
    let rows = column_row_count(&grp.data_sets, grp.row_titles.len());
    let row_title = non_empty_or(&grp.row_factor_title, "Row Factor");
    let col_title = non_empty_or(&grp.column_factor_title, "Column Factor");

    match grp.subcolumn_mode {
        SubcolumnMode::RawReplicates(n) => convert_grouped_reps_to_mv(grp, rows, n, row_title, col_title),
        _ => convert_grouped_summary_to_mv(grp, rows, row_title, col_title),
    }
}

fn convert_grouped_reps_to_mv(
    grp: &GroupedTable,
    rows: usize,
    n: usize,
    row_title: String,
    col_title: String,
) -> Result<BuiltArchetype, TransformError> {
    let mut records: Vec<(Cell, Cell, Option<Cell>, Cell, CellCoord)> = Vec::new();
    for r in 0..rows {
        let row_cell = grouped_row_factor_cell(grp, r);
        for (c, ds) in grp.data_sets.iter().enumerate() {
            let col_cell = Cell::text(ds.title.clone());
            if let Some(row_cells) = ds.cells.get(r) {
                for s in 0..n {
                    if let Some(cell) = row_cells.get(s)
                        && is_occupied(cell)
                    {
                        let rep = (n > 1).then(|| {
                            #[allow(clippy::cast_precision_loss)]
                            Cell::number((s + 1) as f64)
                        });
                        records.push((row_cell.clone(), col_cell.clone(), rep, cell.clone(), CellCoord::new(r, c, s)));
                    }
                }
            }
        }
    }

    let out_rows = records.len();
    validate_rows(out_rows)?;
    let has_rep = n > 1;
    let var_count = if has_rep { 4 } else { 3 };
    let val_col_idx = var_count - 1;
    let mut mv = MultipleVariablesTable::new(var_count, out_rows)?;
    mv.variables[0].title = row_title;
    mv.variables[1].title = col_title;
    if has_rep {
        mv.variables[2].title = String::from("Replicate");
    }
    mv.variables[val_col_idx].title = String::from("Value");

    let mut coord_map = BTreeMap::new();
    for (out_r, (rf, cf, rep, val, old_coord)) in records.into_iter().enumerate() {
        mv.variables[0].cells[out_r] = rf;
        mv.variables[1].cells[out_r] = cf;
        if let Some(rep_cell) = rep {
            mv.variables[2].cells[out_r] = rep_cell;
        }
        mv.variables[val_col_idx].cells[out_r] = val;
        coord_map.insert(old_coord, CellCoord::new(out_r, val_col_idx, 0));
    }
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::MultipleVariables(mv),
        coord_map,
        preserved_cells: out_rows,
        dropped_cells: 0,
        warnings: Vec::new(),
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_grouped_summary_to_mv(grp: &GroupedTable, rows: usize, row_title: String, col_title: String) -> Result<BuiltArchetype, TransformError> {
    let sub_labels = grp.subcolumn_mode.subcolumn_labels();
    let sub_count = sub_labels.len();
    let mut row_entries: Vec<(Cell, Cell, Vec<Cell>, usize, usize)> = Vec::new();

    for r in 0..rows {
        let row_cell = grouped_row_factor_cell(grp, r);
        for (c, ds) in grp.data_sets.iter().enumerate() {
            let Some(row_cells) = ds.cells.get(r) else { continue };
            if row_cells.iter().take(sub_count).any(is_occupied) {
                let mut vals = Vec::with_capacity(sub_count);
                for s in 0..sub_count {
                    vals.push(row_cells.get(s).cloned().unwrap_or_default());
                }
                row_entries.push((row_cell.clone(), Cell::text(ds.title.clone()), vals, r, c));
            }
        }
    }

    let out_rows = row_entries.len();
    validate_rows(out_rows)?;
    let mut mv = MultipleVariablesTable::new(2 + sub_count, out_rows)?;
    mv.variables[0].title = row_title;
    mv.variables[1].title = col_title;
    for (s, lbl) in sub_labels.into_iter().enumerate() {
        mv.variables[2 + s].title = lbl;
    }

    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    for (out_r, (rf, cf, vals, old_r, old_c)) in row_entries.into_iter().enumerate() {
        mv.variables[0].cells[out_r] = rf;
        mv.variables[1].cells[out_r] = cf;
        for (s, val) in vals.into_iter().enumerate() {
            if is_occupied(&val) {
                preserved += 1;
            }
            mv.variables[2 + s].cells[out_r] = val;
            coord_map.insert(CellCoord::new(old_r, old_c, s), CellCoord::new(out_r, 2 + s, 0));
        }
    }
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::MultipleVariables(mv),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings: Vec::new(),
        schema_lossy: false,
        keep_links: false,
    })
}

fn grouped_row_factor_cell(grp: &GroupedTable, r: usize) -> Cell {
    if let Some(Some(num)) = grp.row_numeric_spacing.as_ref().and_then(|v| v.get(r))
        && num.is_finite()
    {
        return Cell::number(*num);
    }
    let lbl = grp.row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty());
    match lbl {
        Some(s) => Cell::text(s.to_owned()),
        None => Cell::text(format!("Row {}", r + 1)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MvVarClass {
    Categorical,
    Label,
    Numeric,
}

fn classify_mv_variables(mv: &MultipleVariablesTable) -> Vec<MvVarClass> {
    mv.variables
        .iter()
        .map(|var| {
            if var.role == VariableRole::Categorical {
                MvVarClass::Categorical
            } else if var.role == VariableRole::Label {
                MvVarClass::Label
            } else if var.role_inferred && var.cells.iter().any(|c| !c.is_empty() && matches!(c.value, CellValue::Text(_))) {
                MvVarClass::Categorical
            } else {
                MvVarClass::Numeric
            }
        })
        .collect()
}

fn convert_mv_to_grouped(mv: &MultipleVariablesTable) -> Result<BuiltArchetype, TransformError> {
    if mv.variables.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("MultipleVariables table has no variables") });
    }
    let rows = mv_row_count(mv);
    let classes = classify_mv_variables(mv);
    let cat_cols: Vec<usize> = classes.iter().enumerate().filter_map(|(i, &c)| (c == MvVarClass::Categorical).then_some(i)).collect();
    let label_cols: Vec<usize> = classes.iter().enumerate().filter_map(|(i, &c)| (c == MvVarClass::Label).then_some(i)).collect();
    let num_cols: Vec<usize> = classes.iter().enumerate().filter_map(|(i, &c)| (c == MvVarClass::Numeric).then_some(i)).collect();

    if cat_cols.len() >= 2 && !num_cols.is_empty() {
        return pivot_mv_two_way_to_grouped(mv, &cat_cols, &label_cols, &num_cols, rows);
    }
    if cat_cols.len() == 1 && !num_cols.is_empty() {
        return pivot_mv_one_factor_to_grouped(mv, cat_cols[0], &label_cols, &num_cols, rows);
    }

    // No categorical grouping columns (or only categorical/label columns): direct column projection.
    let cols = mv.variables.len();
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(1), cols, rows)?;
    grouped.row_titles = resized_titles(&mv.row_titles, rows);
    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    let mut warnings = Vec::new();
    for (c, var) in mv.variables.iter().enumerate() {
        grouped.data_sets[c].title = var.title.clone();
        if matches!(var.source, VariableSource::Calculated { .. }) {
            warnings.push(format!("Calculated variable `{}` evaluated/frozen to static cells", var.title));
        }
        for (r, cell) in var.cells.iter().enumerate().take(rows) {
            grouped.data_sets[c].cells[r][0] = cell.clone();
            coord_map.insert(CellCoord::new(r, c, 0), CellCoord::new(r, c, 0));
            if is_occupied(cell) {
                preserved += 1;
            }
        }
    }
    Ok(BuiltArchetype {
        archetype: TableArchetype::Grouped(grouped),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings,
        schema_lossy: false,
        keep_links: false,
    })
}

fn pivot_mv_two_way_to_grouped(
    mv: &MultipleVariablesTable,
    cat_cols: &[usize],
    label_cols: &[usize],
    num_cols: &[usize],
    rows: usize,
) -> Result<BuiltArchetype, TransformError> {
    let row_fac_col = cat_cols[0];
    let col_fac_col = cat_cols[1];

    // Detect if num_cols matches a 3-column pre-summarized tuple or (Replicate, Value).
    if let Some((mode, summary_cols)) = detect_mv_summary_triple(mv, num_cols) {
        return pivot_mv_summary_to_grouped(mv, (row_fac_col, col_fac_col), &summary_cols, mode, (cat_cols, label_cols, num_cols), rows);
    }

    let (rep_col, val_col, extra_num_cols) = split_replicate_and_value(mv, num_cols);
    let mut dropped_cells = 0_usize;
    let mut warnings = Vec::new();
    let mut schema_lossy = false;

    for &ec in cat_cols.iter().skip(2).chain(label_cols.iter()).chain(extra_num_cols.iter()) {
        let cnt = count_var_occupied(&mv.variables[ec], rows);
        if cnt > 0 {
            dropped_cells += cnt;
        }
        schema_lossy = true;
    }
    if schema_lossy {
        warnings.push(String::from(
            "MultipleVariables contains more than two factor columns or extra response columns that cannot fit in a 2-way Grouped table",
        ));
    }
    if matches!(mv.variables[val_col].source, VariableSource::Calculated { .. }) {
        warnings.push(format!("Calculated variable `{}` evaluated/frozen to static cells", mv.variables[val_col].title));
    }

    let mut row_levels: Vec<String> = Vec::new();
    let mut col_levels: Vec<String> = Vec::new();
    let mut row_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut col_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut buckets: RepBucketMap = BTreeMap::new();

    for r in 0..rows {
        let r_tok = mv_cell_level_token(mv.variables[row_fac_col].cells.get(r));
        let c_tok = mv_cell_level_token(mv.variables[col_fac_col].cells.get(r));
        let val_cell = mv.variables[val_col].cells.get(r).cloned().unwrap_or_default();
        let (Some(rk), Some(ck)) = (r_tok, c_tok) else {
            if is_occupied(&val_cell) {
                dropped_cells += 1;
                schema_lossy = true;
            }
            continue;
        };
        let ri = intern_level(&mut row_levels, &mut row_index, rk);
        let ci = intern_level(&mut col_levels, &mut col_index, ck);
        let rep_order = rep_col.and_then(|rc| mv.variables[rc].cells.get(r).and_then(Cell::active_number));
        buckets.entry((ri, ci)).or_default().push((rep_order, r, val_cell));
    }

    let n_rows = row_levels.len();
    let n_cols = col_levels.len();
    if n_cols == 0 {
        return Err(TransformError::InvalidInput { message: String::from("no factor levels found to build Grouped table") });
    }
    validate_data_sets(n_cols)?;
    validate_rows(n_rows)?;
    let max_reps = buckets.values().map(Vec::len).max().unwrap_or(1).max(1);
    validate_subcolumns(max_reps)?;

    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(max_reps), n_cols, n_rows)?;
    grouped.row_factor_title = mv.variables[row_fac_col].title.clone();
    grouped.column_factor_title = mv.variables[col_fac_col].title.clone();
    grouped.row_titles = row_levels;
    for (ci, cl) in col_levels.iter().enumerate() {
        grouped.data_sets[ci].title = cl.clone();
    }

    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    for ((ri, ci), mut entries) in buckets {
        entries.sort_by(|a, b| match (a.0, b.0) {
            (Some(ra), Some(rb)) => ra.total_cmp(&rb).then_with(|| a.1.cmp(&b.1)),
            _ => a.1.cmp(&b.1),
        });
        for (s, (_, old_r, cell)) in entries.into_iter().enumerate() {
            grouped.data_sets[ci].cells[ri][s] = cell.clone();
            coord_map.insert(CellCoord::new(old_r, val_col, 0), CellCoord::new(ri, ci, s));
            if is_occupied(&cell) {
                preserved += 1;
            }
        }
    }
    // Count preserved factor tokens in row_fac_col/col_fac_col/rep_col as represented by the structure.
    let mut mapped_fac_cells = count_var_occupied(&mv.variables[row_fac_col], rows) + count_var_occupied(&mv.variables[col_fac_col], rows);
    if let Some(rc) = rep_col {
        mapped_fac_cells += count_var_occupied(&mv.variables[rc], rows);
    }
    preserved += mapped_fac_cells;

    Ok(BuiltArchetype {
        archetype: TableArchetype::Grouped(grouped),
        coord_map,
        preserved_cells: preserved,
        dropped_cells,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn pivot_mv_summary_to_grouped(
    mv: &MultipleVariablesTable,
    factors: (usize, usize),
    summary_cols: &[usize; 3],
    mode: SubcolumnMode,
    all_cols: (&[usize], &[usize], &[usize]),
    rows: usize,
) -> Result<BuiltArchetype, TransformError> {
    let (row_fac_col, col_fac_col) = factors;
    let (cat_cols, label_cols, num_cols) = all_cols;
    let mut dropped_cells = 0_usize;
    let mut schema_lossy = false;
    for &ec in cat_cols.iter().skip(2).chain(label_cols.iter()).chain(num_cols.iter().filter(|c| !summary_cols.contains(c))) {
        dropped_cells += count_var_occupied(&mv.variables[ec], rows);
        schema_lossy = true;
    }

    let mut row_levels = Vec::new();
    let mut col_levels = Vec::new();
    let mut row_index = BTreeMap::new();
    let mut col_index = BTreeMap::new();
    let mut entries: Vec<(usize, usize, usize, [Cell; 3])> = Vec::new();
    let mut seen_pairs: BTreeSet<(usize, usize)> = BTreeSet::new();

    for r in 0..rows {
        let r_tok = mv_cell_level_token(mv.variables[row_fac_col].cells.get(r));
        let c_tok = mv_cell_level_token(mv.variables[col_fac_col].cells.get(r));
        let triplet = [
            mv.variables[summary_cols[0]].cells.get(r).cloned().unwrap_or_default(),
            mv.variables[summary_cols[1]].cells.get(r).cloned().unwrap_or_default(),
            mv.variables[summary_cols[2]].cells.get(r).cloned().unwrap_or_default(),
        ];
        let (Some(rk), Some(ck)) = (r_tok, c_tok) else {
            dropped_cells += triplet.iter().filter(|c| is_occupied(c)).count();
            schema_lossy = true;
            continue;
        };
        let ri = intern_level(&mut row_levels, &mut row_index, rk);
        let ci = intern_level(&mut col_levels, &mut col_index, ck);
        if !seen_pairs.insert((ri, ci)) {
            dropped_cells += triplet.iter().filter(|c| is_occupied(c)).count();
            schema_lossy = true;
            continue;
        }
        entries.push((ri, ci, r, triplet));
    }

    validate_data_sets(col_levels.len().max(1))?;
    validate_rows(row_levels.len())?;
    let mut grouped = GroupedTable::new(mode, col_levels.len().max(1), row_levels.len())?;
    if col_levels.is_empty() {
        grouped.data_sets.clear();
    } else {
        grouped.row_factor_title = mv.variables[row_fac_col].title.clone();
        grouped.column_factor_title = mv.variables[col_fac_col].title.clone();
        grouped.row_titles = row_levels;
        for (ci, cl) in col_levels.iter().enumerate() {
            grouped.data_sets[ci].title = cl.clone();
        }
    }

    let mut coord_map = BTreeMap::new();
    let mut preserved = count_var_occupied(&mv.variables[row_fac_col], rows) + count_var_occupied(&mv.variables[col_fac_col], rows);
    for (ri, ci, old_r, triplet) in entries {
        for (s, cell) in triplet.into_iter().enumerate() {
            if is_occupied(&cell) {
                preserved += 1;
            }
            grouped.data_sets[ci].cells[ri][s] = cell;
            coord_map.insert(CellCoord::new(old_r, summary_cols[s], 0), CellCoord::new(ri, ci, s));
        }
    }

    let warnings = if schema_lossy {
        vec![String::from("Extra columns or duplicate factor pairs were dropped during Grouped summary pivot")]
    } else {
        Vec::new()
    };
    Ok(BuiltArchetype {
        archetype: TableArchetype::Grouped(grouped),
        coord_map,
        preserved_cells: preserved,
        dropped_cells,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn pivot_mv_one_factor_to_grouped(
    mv: &MultipleVariablesTable,
    row_fac_col: usize,
    label_cols: &[usize],
    num_cols: &[usize],
    rows: usize,
) -> Result<BuiltArchetype, TransformError> {
    let mut dropped_cells = 0_usize;
    let mut schema_lossy = false;
    for &lc in label_cols {
        dropped_cells += count_var_occupied(&mv.variables[lc], rows);
        schema_lossy = true;
    }

    let mut row_levels = Vec::new();
    let mut row_index = BTreeMap::new();
    let mut buckets: BTreeMap<(usize, usize), Vec<(usize, Cell)>> = BTreeMap::new();

    for r in 0..rows {
        let Some(rk) = mv_cell_level_token(mv.variables[row_fac_col].cells.get(r)) else {
            for &nc in num_cols {
                if let Some(cell) = mv.variables[nc].cells.get(r)
                    && is_occupied(cell)
                {
                    dropped_cells += 1;
                    schema_lossy = true;
                }
            }
            continue;
        };
        let ri = intern_level(&mut row_levels, &mut row_index, rk);
        for (ci, &nc) in num_cols.iter().enumerate() {
            if let Some(cell) = mv.variables[nc].cells.get(r)
                && is_occupied(cell)
            {
                buckets.entry((ri, ci)).or_default().push((r, cell.clone()));
            }
        }
    }

    let n_rows = row_levels.len();
    let n_cols = num_cols.len();
    validate_data_sets(n_cols)?;
    validate_rows(n_rows)?;
    let max_reps = buckets.values().map(Vec::len).max().unwrap_or(1).max(1);
    validate_subcolumns(max_reps)?;

    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(max_reps), n_cols, n_rows)?;
    grouped.row_factor_title = mv.variables[row_fac_col].title.clone();
    grouped.row_titles = row_levels;
    for (ci, &nc) in num_cols.iter().enumerate() {
        grouped.data_sets[ci].title = mv.variables[nc].title.clone();
    }

    let mut coord_map = BTreeMap::new();
    let mut preserved = count_var_occupied(&mv.variables[row_fac_col], rows);
    for ((ri, ci), entries) in buckets {
        let old_col = num_cols[ci];
        for (s, (old_r, cell)) in entries.into_iter().enumerate() {
            grouped.data_sets[ci].cells[ri][s] = cell;
            coord_map.insert(CellCoord::new(old_r, old_col, 0), CellCoord::new(ri, ci, s));
            preserved += 1;
        }
    }
    let warnings =
        if schema_lossy { vec![String::from("Unmatched label/missing-factor cells were dropped during Grouped pivot")] } else { Vec::new() };

    Ok(BuiltArchetype {
        archetype: TableArchetype::Grouped(grouped),
        coord_map,
        preserved_cells: preserved,
        dropped_cells,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn detect_mv_summary_triple(mv: &MultipleVariablesTable, num_cols: &[usize]) -> Option<(SubcolumnMode, [usize; 3])> {
    if num_cols.len() < 3 {
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
        if num_cols.len() == 3 {
            let matches_all = (0..3).all(|i| mv.variables[num_cols[i]].title.trim().eq_ignore_ascii_case(&labels[i]));
            if matches_all {
                return Some((mode, [num_cols[0], num_cols[1], num_cols[2]]));
            }
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
    let val_col = num_cols[0];
    let rest = num_cols[1..].to_vec();
    (None, val_col, rest)
}

fn convert_xy_to_mv(xy: &XyTable) -> Result<BuiltArchetype, TransformError> {
    if xy.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("XY table has no Y data columns") });
    }
    let rows = xy.row_count();
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
    let mut mv = MultipleVariablesTable::new(var_count, rows)?;
    mv.row_titles = resized_titles(&xy.row_titles, rows);
    mv.row_titles_expanded = xy.row_titles_expanded;
    mv.variables[0].title = non_empty_or(&xy.x_title, "X");

    let mut preserved = 0_usize;
    let mut warnings = Vec::new();
    match &xy.x_mode {
        XColumnMode::GeneratedSequence { .. } => {
            for r in 0..rows {
                mv.variables[0].cells[r] = xy.x_value_at(r).map_or_else(Cell::empty, Cell::number);
            }
            warnings.push(String::from("Generated X sequence materialized into Continuous variable column"));
        }
        XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => {
            for (r, x_row) in xy.x_cells.iter().enumerate().take(rows) {
                if let Some(cell) = x_row.first() {
                    mv.variables[0].cells[r] = cell.clone();
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }

    let mut coord_map = BTreeMap::new();
    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        let dst_c = c + 1;
        mv.variables[dst_c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate().take(rows) {
            if let Some(cell) = row_cells.first() {
                mv.variables[dst_c].cells[r] = cell.clone();
                coord_map.insert(CellCoord::new(r, c, 0), CellCoord::new(r, dst_c, 0));
                if is_occupied(cell) {
                    preserved += 1;
                }
            }
        }
    }
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::MultipleVariables(mv),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings,
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_xy_reps_to_mv(xy: &XyTable, rows: usize, n: usize) -> Result<BuiltArchetype, TransformError> {
    let mut records: Vec<(Cell, Cell, Cell, Cell, CellCoord)> = Vec::new();
    let mut preserved_x_occupied = 0_usize;
    let mut seen_x_rows = BTreeSet::new();

    for r in 0..rows {
        let x_cell = match &xy.x_mode {
            XColumnMode::GeneratedSequence { .. } => xy.x_value_at(r).map_or_else(Cell::empty, Cell::number),
            XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => {
                xy.x_cells.get(r).and_then(|s| s.first()).cloned().unwrap_or_default()
            }
        };
        for (c, ds) in xy.data_sets.iter().enumerate() {
            let grp_cell = Cell::text(ds.title.clone());
            if let Some(row_cells) = ds.cells.get(r) {
                for s in 0..n {
                    if let Some(cell) = row_cells.get(s)
                        && is_occupied(cell)
                    {
                        if is_occupied(&x_cell) && seen_x_rows.insert(r) {
                            preserved_x_occupied += 1;
                        }
                        #[allow(clippy::cast_precision_loss)]
                        let rep_cell = Cell::number((s + 1) as f64);
                        records.push((x_cell.clone(), grp_cell.clone(), rep_cell, cell.clone(), CellCoord::new(r, c, s)));
                    }
                }
            }
        }
    }

    // Also count any X-only occupied row not paired with Y? Preserve it too if X had data and no Y!
    for r in 0..rows {
        if !seen_x_rows.contains(&r)
            && !matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. })
            && let Some(x_cell) = xy.x_cells.get(r).and_then(|s| s.first())
            && is_occupied(x_cell)
        {
            preserved_x_occupied += 1;
            records.push((x_cell.clone(), Cell::empty(), Cell::empty(), Cell::empty(), CellCoord::new(r, 0, 0)));
        }
    }

    let out_rows = records.len();
    validate_rows(out_rows)?;
    let mut mv = MultipleVariablesTable::new(4, out_rows)?;
    mv.variables[0].title = non_empty_or(&xy.x_title, "X");
    mv.variables[1].title = String::from("Group");
    mv.variables[2].title = String::from("Replicate");
    mv.variables[3].title = String::from("Value");

    let mut coord_map = BTreeMap::new();
    let mut preserved_y = 0_usize;
    for (out_r, (xc, gc, rc, yc, old_coord)) in records.into_iter().enumerate() {
        mv.variables[0].cells[out_r] = xc;
        mv.variables[1].cells[out_r] = gc;
        mv.variables[2].cells[out_r] = rc;
        if is_occupied(&yc) {
            preserved_y += 1;
            coord_map.insert(old_coord, CellCoord::new(out_r, 3, 0));
        }
        mv.variables[3].cells[out_r] = yc;
    }
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::MultipleVariables(mv),
        coord_map,
        preserved_cells: preserved_x_occupied + preserved_y,
        dropped_cells: 0,
        warnings: vec![String::from("XY replicate subcolumns reshaped to tidy long (X, Group, Replicate, Value) variables")],
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_xy_wide_to_mv(xy: &XyTable, rows: usize, x_subs: usize) -> Result<BuiltArchetype, TransformError> {
    let x_var_count = if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) { 1 } else { x_subs };
    let y_subs = xy.y_subcolumn_mode.subcolumn_count();
    let total_vars = x_var_count + xy.data_sets.len() * y_subs;
    validate_data_sets(total_vars)?;

    let mut mv = MultipleVariablesTable::new(total_vars, rows)?;
    mv.row_titles = resized_titles(&xy.row_titles, rows);
    mv.row_titles_expanded = xy.row_titles_expanded;

    let mut preserved = 0_usize;
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        mv.variables[0].title = non_empty_or(&xy.x_title, "X");
        for r in 0..rows {
            mv.variables[0].cells[r] = xy.x_value_at(r).map_or_else(Cell::empty, Cell::number);
        }
    } else {
        let x_labels = match &xy.x_mode {
            XColumnMode::Numbers { subcolumn_mode } => subcolumn_mode.subcolumn_labels(),
            _ => vec![String::from("X")],
        };
        let x_base = non_empty_or(&xy.x_title, "X");
        for (s, x_lbl) in x_labels.iter().enumerate().take(x_subs) {
            mv.variables[s].title = if x_subs == 1 { x_base.clone() } else { format!("{x_base} ({x_lbl})") };
            for (r, x_row) in xy.x_cells.iter().enumerate().take(rows) {
                if let Some(cell) = x_row.get(s) {
                    mv.variables[s].cells[r] = cell.clone();
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }

    let y_labels = xy.y_subcolumn_mode.subcolumn_labels();
    let mut coord_map = BTreeMap::new();
    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        for (s, y_lbl) in y_labels.iter().enumerate().take(y_subs) {
            let dst_c = x_var_count + c * y_subs + s;
            mv.variables[dst_c].title = if y_subs == 1 { src_ds.title.clone() } else { format!("{} ({y_lbl})", src_ds.title) };
            for (r, row_cells) in src_ds.cells.iter().enumerate().take(rows) {
                if let Some(cell) = row_cells.get(s) {
                    mv.variables[dst_c].cells[r] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, dst_c, 0));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }
    for var in &mut mv.variables {
        var.refresh_inferred_role();
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::MultipleVariables(mv),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings: Vec::new(),
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_mv_to_xy(mv: &MultipleVariablesTable) -> Result<BuiltArchetype, TransformError> {
    if mv.variables.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("MultipleVariables table has no variables") });
    }
    let rows = mv_row_count(mv);
    let classes = classify_mv_variables(mv);
    let cat_cols: Vec<usize> = classes.iter().enumerate().filter_map(|(i, &c)| (c == MvVarClass::Categorical).then_some(i)).collect();
    let label_cols: Vec<usize> = classes.iter().enumerate().filter_map(|(i, &c)| (c == MvVarClass::Label).then_some(i)).collect();
    let num_cols: Vec<usize> = classes.iter().enumerate().filter_map(|(i, &c)| (c == MvVarClass::Numeric).then_some(i)).collect();

    if cat_cols.len() == 1 && num_cols.len() >= 2 {
        return pivot_mv_grouped_xy(mv, cat_cols[0], &label_cols, &num_cols, rows);
    }

    // No single grouping categorical pivot: map numeric columns directly (dropping categorical/extra labels).
    let mut dropped_cells = 0_usize;
    let mut warnings = Vec::new();
    let mut schema_lossy = false;

    for &cc in &cat_cols {
        dropped_cells += count_var_occupied(&mv.variables[cc], rows);
        schema_lossy = true;
    }

    let mut row_titles = resized_titles(&mv.row_titles, rows);
    let mut used_label_col: Option<usize> = None;
    if label_cols.len() == 1 && row_titles.iter().all(|s| s.trim().is_empty()) {
        let lc = label_cols[0];
        for (r, slot) in row_titles.iter_mut().enumerate().take(rows) {
            *slot = mv_cell_level_token(mv.variables[lc].cells.get(r)).unwrap_or_default();
        }
        used_label_col = Some(lc);
    } else {
        for &lc in &label_cols {
            dropped_cells += count_var_occupied(&mv.variables[lc], rows);
            schema_lossy = true;
        }
    }

    if num_cols.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("MultipleVariables table has no numeric variables to map to XY") });
    }

    if schema_lossy {
        warnings.push(String::from("Categorical/text variables cannot be represented directly in XY columns and will be dropped"));
    }

    let y_count = if num_cols.len() == 1 { 1 } else { num_cols.len() - 1 };
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, y_count, rows)?;
    xy.row_titles = row_titles;
    xy.row_titles_expanded = xy.row_titles.iter().any(|s| !s.trim().is_empty());

    let mut coord_map = BTreeMap::new();
    let mut preserved = used_label_col.map_or(0, |lc| count_var_occupied(&mv.variables[lc], rows));

    if num_cols.len() == 1 {
        let yc = num_cols[0];
        xy.data_sets[0].title = mv.variables[yc].title.clone();
        let row_titles_copy = xy.row_titles.clone();
        populate_generated_or_titled_x(&mut xy, &row_titles_copy, rows);
        for (r, cell) in mv.variables[yc].cells.iter().enumerate().take(rows) {
            copy_mv_cell_to_xy_numeric(&mut xy.data_sets[0].cells[r][0], cell, &mut preserved, &mut dropped_cells, &mut schema_lossy);
            coord_map.insert(CellCoord::new(r, yc, 0), CellCoord::new(r, 0, 0));
        }
        warnings.push(String::from("Single numeric variable mapped to Y with sequential X coordinates"));
    } else {
        let xc = num_cols[0];
        xy.x_title = mv.variables[xc].title.clone();
        for (r, cell) in mv.variables[xc].cells.iter().enumerate().take(rows) {
            copy_mv_cell_to_xy_numeric(&mut xy.x_cells[r][0], cell, &mut preserved, &mut dropped_cells, &mut schema_lossy);
        }
        for (yi, &yc) in num_cols.iter().skip(1).enumerate() {
            xy.data_sets[yi].title = mv.variables[yc].title.clone();
            for (r, cell) in mv.variables[yc].cells.iter().enumerate().take(rows) {
                copy_mv_cell_to_xy_numeric(&mut xy.data_sets[yi].cells[r][0], cell, &mut preserved, &mut dropped_cells, &mut schema_lossy);
                coord_map.insert(CellCoord::new(r, yc, 0), CellCoord::new(r, yi, 0));
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Xy(xy),
        coord_map,
        preserved_cells: preserved,
        dropped_cells,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn copy_mv_cell_to_xy_numeric(dst: &mut Cell, src: &Cell, preserved: &mut usize, dropped: &mut usize, schema_lossy: &mut bool) {
    match &src.value {
        CellValue::Text(s) if !s.trim().is_empty() => {
            *dropped += 1;
            *schema_lossy = true;
            *dst = Cell { value: CellValue::Empty, excluded: src.excluded };
        }
        _ => {
            *dst = src.clone();
            if is_occupied(src) {
                *preserved += 1;
            }
        }
    }
}

fn pivot_mv_grouped_xy(
    mv: &MultipleVariablesTable,
    group_col: usize,
    label_cols: &[usize],
    num_cols: &[usize],
    rows: usize,
) -> Result<BuiltArchetype, TransformError> {
    let x_col = num_cols[0];
    let num_rest: Vec<usize> = num_cols[1..].to_vec();
    let (rep_col, y_col, extra_nums) = split_replicate_and_value(mv, &num_rest);

    let mut dropped_cells = 0_usize;
    let mut schema_lossy = false;
    let mut warnings = Vec::new();
    for &ec in label_cols.iter().chain(extra_nums.iter()) {
        dropped_cells += count_var_occupied(&mv.variables[ec], rows);
        schema_lossy = true;
    }
    if schema_lossy {
        warnings.push(String::from("Extra numeric/label variables cannot fit in a single grouped XY pivot and will be dropped"));
    }

    let mut x_keys: Vec<f64> = Vec::new();
    let mut x_index: BTreeMap<u64, usize> = BTreeMap::new();
    let mut groups: Vec<String> = Vec::new();
    let mut group_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut buckets: RepBucketMap = BTreeMap::new();
    let mut x_excluded_by_row: BTreeMap<usize, bool> = BTreeMap::new();

    for r in 0..rows {
        let x_cell = mv.variables[x_col].cells.get(r);
        let g_tok = mv_cell_level_token(mv.variables[group_col].cells.get(r));
        let y_cell = mv.variables[y_col].cells.get(r).cloned().unwrap_or_default();

        let x_num = x_cell.and_then(|c| c.value.finite_f64());
        let (Some(xv), Some(gk)) = (x_num, g_tok) else {
            if let Some(xc) = x_cell
                && is_occupied(xc)
            {
                dropped_cells += 1;
                schema_lossy = true;
            }
            if is_occupied(&y_cell) {
                dropped_cells += 1;
                schema_lossy = true;
            }
            continue;
        };

        let norm_x = if xv == 0.0 { 0.0 } else { xv };
        let x_bits = norm_x.to_bits();
        let xi = if let Some(&idx) = x_index.get(&x_bits) {
            idx
        } else {
            let idx = x_keys.len();
            x_keys.push(norm_x);
            x_index.insert(x_bits, idx);
            idx
        };
        if x_cell.is_some_and(|c| c.excluded) {
            x_excluded_by_row.entry(xi).or_insert(true);
        }
        let gi = intern_level(&mut groups, &mut group_index, gk);
        let rep_ord = rep_col.and_then(|rc| mv.variables[rc].cells.get(r).and_then(Cell::active_number));
        buckets.entry((xi, gi)).or_default().push((rep_ord, r, y_cell));
    }

    if groups.is_empty() || x_keys.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no valid (X, Group, Y) records to pivot into XY") });
    }

    // Sort X rows ascending while remapping x indices cleanly.
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
    let max_reps = buckets.values().map(Vec::len).max().unwrap_or(1).max(1);
    validate_subcolumns(max_reps)?;
    let y_mode = if max_reps == 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(max_reps) };

    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, y_mode, n_cols, n_rows)?;
    xy.x_title = mv.variables[x_col].title.clone();
    for (new_r, &old_x_idx) in order.iter().enumerate() {
        let excluded = x_excluded_by_row.get(&old_x_idx).copied().unwrap_or(false);
        xy.x_cells[new_r][0] = Cell { value: CellValue::Number(x_keys[old_x_idx]), excluded };
    }
    for (gi, g_name) in groups.iter().enumerate() {
        xy.data_sets[gi].title = g_name.clone();
    }

    let mut coord_map = BTreeMap::new();
    let mut preserved = count_var_occupied(&mv.variables[x_col], rows) + count_var_occupied(&mv.variables[group_col], rows);
    if let Some(rc) = rep_col {
        preserved += count_var_occupied(&mv.variables[rc], rows);
    }
    for ((old_xi, gi), mut entries) in buckets {
        let dst_r = remap_x[old_xi];
        entries.sort_by(|a, b| match (a.0, b.0) {
            (Some(ra), Some(rb)) => ra.total_cmp(&rb).then_with(|| a.1.cmp(&b.1)),
            _ => a.1.cmp(&b.1),
        });
        for (s, (_, old_r, cell)) in entries.into_iter().enumerate() {
            if is_occupied(&cell) {
                preserved += 1;
            }
            xy.data_sets[gi].cells[dst_r][s] = cell;
            coord_map.insert(CellCoord::new(old_r, y_col, 0), CellCoord::new(dst_r, gi, s));
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Xy(xy),
        coord_map,
        preserved_cells: preserved,
        dropped_cells,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn convert_column_to_mv(col: &ColumnTable) -> Result<BuiltArchetype, TransformError> {
    if col.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("Column table has no data columns") });
    }
    let rows = column_row_count(&col.data_sets, col.row_titles.len());
    let subs = col.subcolumn_mode.subcolumn_count();
    let total_vars = col.data_sets.len() * subs;
    validate_data_sets(total_vars)?;

    let mut mv = MultipleVariablesTable::new(total_vars, rows)?;
    mv.row_titles = resized_titles(&col.row_titles, rows);
    mv.row_titles_expanded = col.row_titles_expanded;
    let labels = col.subcolumn_mode.subcolumn_labels();
    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;

    for (c, src_ds) in col.data_sets.iter().enumerate() {
        for (s, lbl) in labels.iter().enumerate().take(subs) {
            let dst_c = c * subs + s;
            mv.variables[dst_c].title = if subs == 1 { src_ds.title.clone() } else { format!("{} ({lbl})", src_ds.title) };
            for (r, row_cells) in src_ds.cells.iter().enumerate().take(rows) {
                if let Some(cell) = row_cells.get(s) {
                    mv.variables[dst_c].cells[r] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, dst_c, 0));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
            mv.variables[dst_c].refresh_inferred_role();
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::MultipleVariables(mv),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings: Vec::new(),
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_mv_to_column(mv: &MultipleVariablesTable) -> Result<BuiltArchetype, TransformError> {
    if mv.variables.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("MultipleVariables table has no variables") });
    }
    let rows = mv_row_count(mv);
    let classes = classify_mv_variables(mv);
    let cat_cols: Vec<usize> = classes.iter().enumerate().filter_map(|(i, &c)| (c == MvVarClass::Categorical).then_some(i)).collect();
    let num_cols: Vec<usize> = classes.iter().enumerate().filter_map(|(i, &c)| (c == MvVarClass::Numeric).then_some(i)).collect();

    if cat_cols.len() == 1 && num_cols.len() == 1 && mv.variables.len() == 2 {
        return unstack_mv_to_column(mv, cat_cols[0], num_cols[0], rows);
    }

    let cols = mv.variables.len();
    let mut col_tbl = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, cols, rows)?;
    col_tbl.row_titles = resized_titles(&mv.row_titles, rows);
    col_tbl.row_titles_expanded = mv.row_titles_expanded;
    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    let mut warnings = Vec::new();
    for (c, var) in mv.variables.iter().enumerate() {
        col_tbl.data_sets[c].title = var.title.clone();
        if matches!(var.source, VariableSource::Calculated { .. }) {
            warnings.push(format!("Calculated variable `{}` evaluated/frozen to static cells", var.title));
        }
        for (r, cell) in var.cells.iter().enumerate().take(rows) {
            col_tbl.data_sets[c].cells[r][0] = cell.clone();
            coord_map.insert(CellCoord::new(r, c, 0), CellCoord::new(r, c, 0));
            if is_occupied(cell) {
                preserved += 1;
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(col_tbl),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings,
        schema_lossy: false,
        keep_links: false,
    })
}

fn unstack_mv_to_column(mv: &MultipleVariablesTable, grp_col: usize, val_col: usize, rows: usize) -> Result<BuiltArchetype, TransformError> {
    let mut groups = Vec::new();
    let mut group_index = BTreeMap::new();
    let mut buckets: Vec<Vec<(usize, Cell)>> = Vec::new();
    let mut dropped = 0_usize;
    let mut schema_lossy = false;

    for r in 0..rows {
        let cell = mv.variables[val_col].cells.get(r).cloned().unwrap_or_default();
        let Some(gk) = mv_cell_level_token(mv.variables[grp_col].cells.get(r)) else {
            if is_occupied(&cell) {
                dropped += 1;
                schema_lossy = true;
            }
            continue;
        };
        let gi = intern_level(&mut groups, &mut group_index, gk);
        if buckets.len() <= gi {
            buckets.resize_with(gi + 1, Vec::new);
        }
        buckets[gi].push((r, cell));
    }

    if groups.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("no group levels found to unstack into Column table") });
    }
    let max_rows = buckets.iter().map(Vec::len).max().unwrap_or(0);
    validate_data_sets(groups.len())?;
    validate_rows(max_rows)?;

    let mut col_tbl = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, groups.len(), max_rows)?;
    let mut coord_map = BTreeMap::new();
    let mut preserved = count_var_occupied(&mv.variables[grp_col], rows);
    for (ci, g_name) in groups.into_iter().enumerate() {
        col_tbl.data_sets[ci].title = g_name;
        for (dst_r, (old_r, cell)) in buckets[ci].iter().enumerate() {
            if is_occupied(cell) {
                preserved += 1;
            }
            col_tbl.data_sets[ci].cells[dst_r][0] = cell.clone();
            coord_map.insert(CellCoord::new(*old_r, val_col, 0), CellCoord::new(dst_r, ci, 0));
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Column(col_tbl),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings: vec![String::from("Unstacked tidy (Group, Value) pairs into Column data sets")],
        schema_lossy,
        keep_links: false,
    })
}

fn convert_grouped_to_xy(grp: &GroupedTable) -> Result<BuiltArchetype, TransformError> {
    if grp.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("Grouped table has no data columns") });
    }
    let cols = grp.data_sets.len();
    let rows = column_row_count(&grp.data_sets, grp.row_titles.len());
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, grp.subcolumn_mode, cols, rows)?;
    xy.x_title = non_empty_or(&grp.row_factor_title, "X");
    xy.row_titles = resized_titles(&grp.row_titles, rows);
    xy.row_titles_expanded = xy.row_titles.iter().any(|s| !s.trim().is_empty());

    let mut warnings = Vec::new();
    if let Some(spacing) = &grp.row_numeric_spacing {
        for r in 0..rows {
            xy.x_cells[r][0] = spacing.get(r).and_then(|&v| v).filter(|v| v.is_finite()).map_or_else(Cell::empty, Cell::number);
        }
    } else {
        populate_generated_or_titled_x(&mut xy, &grp.row_titles, rows);
        warnings.push(String::from("Grouped row factor mapped to parsed or sequential X coordinates"));
    }

    let subs = grp.subcolumn_mode.subcolumn_count();
    let mut coord_map = BTreeMap::new();
    let mut preserved = 0_usize;
    for (c, src_ds) in grp.data_sets.iter().enumerate() {
        xy.data_sets[c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for s in 0..subs {
                if let Some(cell) = row_cells.get(s) {
                    xy.data_sets[c].cells[r][s] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, c, s));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Xy(xy),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: 0,
        warnings,
        schema_lossy: false,
        keep_links: false,
    })
}

fn convert_xy_to_grouped(xy: &XyTable) -> Result<BuiltArchetype, TransformError> {
    if xy.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("XY table has no Y data columns") });
    }
    let rows = xy.row_count();
    let cols = xy.data_sets.len();
    let target_mode = if xy.y_subcolumn_mode == SubcolumnMode::Single { SubcolumnMode::RawReplicates(1) } else { xy.y_subcolumn_mode };
    let mut grouped = GroupedTable::new(target_mode, cols, rows)?;
    grouped.row_factor_title = non_empty_or(&xy.x_title, "Row Factor");

    let mut spacing = Vec::with_capacity(rows);
    let mut titles = Vec::with_capacity(rows);
    let mut preserved = 0_usize;
    let mut dropped = 0_usize;
    let mut warnings = Vec::new();
    let mut schema_lossy = false;

    for r in 0..rows {
        let x_val = xy.x_value_at(r);
        spacing.push(x_val);
        let rt = xy.row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty());
        let lbl = match (rt, x_val) {
            (Some(s), _) => s.to_owned(),
            (None, Some(v)) => format!("{v}"),
            (None, None) => String::new(),
        };
        titles.push(lbl);
    }
    grouped.row_titles = titles;
    grouped.row_numeric_spacing = Some(spacing);

    if !matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        let mut dropped_x_subs = 0_usize;
        for x_row in xy.x_cells.iter().take(rows) {
            for (s, cell) in x_row.iter().enumerate() {
                if s == 0 {
                    if is_occupied(cell) && cell.active_number().is_some() {
                        preserved += 1;
                    } else if is_occupied(cell) {
                        // Excluded or non-numeric X cell cannot stay in row_numeric_spacing as active number.
                        dropped += 1;
                        schema_lossy = true;
                    }
                } else if is_occupied(cell) {
                    dropped += 1;
                    dropped_x_subs += 1;
                    schema_lossy = true;
                }
            }
        }
        if dropped_x_subs > 0 {
            warnings.push(format!("Dropped {dropped_x_subs} X error subcolumn cell(s) unsupported by Grouped row spacing"));
        }
    }

    let y_subs = target_mode.subcolumn_count();
    let mut coord_map = BTreeMap::new();
    for (c, src_ds) in xy.data_sets.iter().enumerate() {
        grouped.data_sets[c].title = src_ds.title.clone();
        for (r, row_cells) in src_ds.cells.iter().enumerate() {
            for s in 0..y_subs {
                if let Some(cell) = row_cells.get(s) {
                    grouped.data_sets[c].cells[r][s] = cell.clone();
                    coord_map.insert(CellCoord::new(r, c, s), CellCoord::new(r, c, s));
                    if is_occupied(cell) {
                        preserved += 1;
                    }
                }
            }
        }
    }

    Ok(BuiltArchetype {
        archetype: TableArchetype::Grouped(grouped),
        coord_map,
        preserved_cells: preserved,
        dropped_cells: dropped,
        warnings,
        schema_lossy,
        keep_links: false,
    })
}

fn mv_row_count(mv: &MultipleVariablesTable) -> usize {
    mv.variables.iter().map(|v| v.cells.len()).max().unwrap_or(mv.row_titles.len()).max(mv.row_titles.len())
}

fn count_var_occupied(var: &VariableColumn, rows: usize) -> usize {
    var.cells.iter().take(rows).filter(|c| is_occupied(c)).count()
}

fn mv_cell_level_token(cell: Option<&Cell>) -> Option<String> {
    let cell = cell?;
    match &cell.value {
        CellValue::Empty => None,
        CellValue::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CellValue::Number(v) if v.is_finite() => Some(format!("{v}")),
        CellValue::Number(_) => None,
    }
}

fn intern_level(levels: &mut Vec<String>, index: &mut BTreeMap<String, usize>, token: String) -> usize {
    if let Some(&idx) = index.get(&token) {
        idx
    } else {
        let idx = levels.len();
        index.insert(token.clone(), idx);
        levels.push(token);
        idx
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

// Keep unused-import-free cleanly when default_column_label isn't referenced directly.
#[allow(dead_code)]
fn _fallback_label(idx: usize) -> String {
    default_column_label(idx)
}
