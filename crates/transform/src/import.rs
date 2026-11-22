//! Four-stage [`ImportLinkSpec`] data import, filtering, reshaping, placement, and
//! circular-link detection pipeline (`docs/implementation/data-tables-and-transforms.md` §6).

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{
    BaselineOperation, Cell, CellCoord, CellValue, CiBound, DataSetColumn, DataSheet, DelimitedScanError, DelimitedScanOptions,
    DelimitedVisitorError, ImportFilterSpec, ImportLinkSpec, InfoSheet, LinkSource, LinkedDataRegion, MAX_DATA_SETS, MAX_ROWS, MAX_SUBCOLUMNS,
    NestedGroupColumn, PlacementMode, ResultCell, ResultSheet, SheetCellRange, SheetId, TableArchetype, VariableColumn, default_column_label,
    scan_delimited, transpose_cell_grid, validate_data_sets, validate_rows, validate_sheet_cells, validate_subcolumns,
};

use crate::error::TransformError;
use crate::formula::{ScalarEnv, eval_scalar_expr, parse_formula};

/// Maximum total dense `Cell` slots allowed in a target sheet during transform/import placement.
pub const MAX_IMPORT_CELLS: usize = 10_000_000;

/// Optional post-filter transformations applied during Stage 2/3 of import.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ImportPipelineOptions {
    /// Optional baseline subtraction/ratio against `baseline_col` (in the filtered grid).
    pub baseline: Option<(usize, BaselineOperation)>,
    /// Optional cell coordinates `(row, col)` to mark `excluded = true` after shaping.
    pub exclude_coords: Vec<(usize, usize)>,
    /// Optional row-level exclusion formula (rows where predicate evaluates true have all cells marked excluded).
    pub exclude_row_predicate: Option<String>,
    /// Optional provenance identifier when attaching a [`LinkedDataRegion`].
    pub link_id: Option<String>,
    /// Optional provenance summary text.
    pub provenance_summary: Option<String>,
}

/// Source data bundle passed into [`apply_import_link_pipeline`].
#[derive(Debug, Clone)]
pub enum ImportSourcePayload<'a> {
    /// Upstream in-project [`DataSheet`] referenced by [`LinkSource::InternalSheet`].
    InternalData(&'a DataSheet),
    /// Upstream in-project [`ResultSheet`] (analyzable or terminal tab) referenced by [`LinkSource::InternalSheet`].
    InternalResult(&'a ResultSheet),
    /// Raw CSV/TSV or clipboard delimited text buffer paired with field and decimal separators.
    DelimitedText {
        /// UTF-8 delimited text payload.
        text: &'a str,
        /// Column field separator character (for example `','`, `'\t'`, or `';'`).
        delimiter: char,
        /// Decimal point character (`'.'` or `','`).
        decimal_separator: char,
    },
}

/// Summary of a completed 4-stage import pipeline run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPipelineReport {
    /// Number of rows written or inserted into the target [`DataSheet`].
    pub rows_placed: usize,
    /// Number of columns or subcolumn slots spanned per placed row.
    pub cols_placed: usize,
    /// Inclusive destination [`SheetCellRange`] populated on the target [`DataSheet`].
    pub target_range: SheetCellRange,
    /// Human-readable provenance summary string recorded for any attached [`LinkedDataRegion`].
    pub provenance_summary: String,
}

/// Detect circular dependencies across [`DataSheet::linked_regions`] using Kahn's topological sort.
///
/// Returns `Ok(order)` of `SheetId`s if acyclic, or [`TransformError::CircularDependency`] if a cycle exists.
pub fn detect_link_cycles(sheets: &[DataSheet]) -> Result<Vec<SheetId>, TransformError> {
    let mut idx_by_id: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, s) in sheets.iter().enumerate() {
        idx_by_id.insert(s.id.as_str(), i);
    }

    let n = sheets.len();
    let mut dependents = vec![Vec::new(); n];
    let mut in_degree = vec![0_usize; n];

    for (dst_idx, sheet) in sheets.iter().enumerate() {
        let mut seen = BTreeSet::new();
        for region in &sheet.linked_regions {
            if let Some(up_id) = region.spec.source.upstream_sheet_id() {
                if up_id == &sheet.id {
                    return Err(TransformError::CircularDependency { cycle: vec![sheet.id.to_string(), sheet.id.to_string()] });
                }
                if let Some(&src_idx) = idx_by_id.get(up_id.as_str())
                    && seen.insert(src_idx)
                {
                    dependents[src_idx].push(dst_idx);
                    in_degree[dst_idx] += 1;
                }
            }
        }
    }

    let mut queue: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    let mut cursor = 0;
    while cursor < queue.len() {
        let u = queue[cursor];
        cursor += 1;
        order.push(sheets[u].id.clone());
        for &v in &dependents[u] {
            in_degree[v] = in_degree[v].saturating_sub(1);
            if in_degree[v] == 0 {
                queue.push(v);
            }
        }
    }

    if order.len() < n {
        let remaining: Vec<usize> = (0..n).filter(|&i| in_degree[i] > 0).collect();
        let cycle = find_sheet_cycle(sheets, &dependents, &remaining);
        return Err(TransformError::CircularDependency { cycle });
    }
    Ok(order)
}

fn find_sheet_cycle(sheets: &[DataSheet], adj: &[Vec<usize>], remaining: &[usize]) -> Vec<String> {
    let mut state = vec![0_u8; sheets.len()];
    let mut stack = Vec::new();
    for &start in remaining {
        if state[start] == 0
            && let Some(idxs) = dfs_sheet_cycle(start, adj, &mut state, &mut stack)
        {
            return idxs.into_iter().map(|i| sheets[i].id.to_string()).collect();
        }
    }
    remaining.iter().map(|&i| sheets[i].id.to_string()).collect()
}

fn dfs_sheet_cycle(node: usize, adj: &[Vec<usize>], state: &mut [u8], stack: &mut Vec<usize>) -> Option<Vec<usize>> {
    state[node] = 1;
    stack.push(node);
    for &next in &adj[node] {
        if state[next] == 1 {
            let pos = stack.iter().position(|&x| x == next).unwrap_or(0);
            let mut cycle = stack[pos..].to_vec();
            cycle.push(next);
            return Some(cycle);
        }
        if state[next] == 0
            && let Some(found) = dfs_sheet_cycle(next, adj, state, stack)
        {
            return Some(found);
        }
    }
    let _ = stack.pop();
    state[node] = 2;
    None
}

/// Execute the 4-stage [`ImportLinkSpec`] pipeline into `target` (respecting `target.frozen`),
/// optionally updating `info_sheet` provenance notes.
pub fn apply_import_link_pipeline(
    target: &mut DataSheet,
    spec: &ImportLinkSpec,
    payload: ImportSourcePayload<'_>,
    opts: &ImportPipelineOptions,
    mut info_sheet: Option<&mut InfoSheet>,
) -> Result<ImportPipelineReport, TransformError> {
    target.ensure_mutable()?;

    if let Some(up_id) = spec.source.upstream_sheet_id()
        && up_id == &target.id
    {
        return Err(TransformError::CircularDependency { cycle: vec![target.id.to_string(), target.id.to_string()] });
    }

    // Stage 1: Extract raw grid.
    let (raw_grid, mut header_row) = extract_stage1(&spec.source, &spec.filter, payload)?;

    // Stage 2: Row/column window, decimation, unstacking, baseline, and exclusion rules.
    let mut shaped = apply_stage2_filter(raw_grid, &spec.filter, opts, &mut header_row, info_sheet.as_deref())?;

    // Stage 3: Orientation (transpose), blank-skipping, and placement at anchor.
    if spec.placement.skip_contiguous_blanks {
        shaped = collapse_contiguous_blank_rows(shaped);
    }
    if spec.placement.transpose {
        let transpose_rows = shaped.len();
        let transpose_cols = shaped.iter().map(Vec::len).max().unwrap_or(0);
        let dense_cells = transpose_rows.checked_mul(transpose_cols);
        check_transform_cell_budget(dense_cells, "transposed import")?;
        if let Some(total) = dense_cells {
            validate_sheet_cells(total)?;
        }
        shaped = transpose_cell_grid(&shaped);
    }

    let rows_placed = shaped.len();
    let cols_placed = shaped.iter().map(Vec::len).max().unwrap_or(0);
    let target_range = place_grid(target, &shaped, spec.placement.mode, spec.placement.anchor)?;

    if spec.placement.propagate_title_header {
        let base_col = target_range.start.col;
        for (offset, hdr) in header_row.iter().enumerate() {
            if let Some(title) = hdr.active_text()
                && let Some(col) = base_col.checked_add(offset)
            {
                set_column_title_if_possible(&mut target.archetype, col, title);
            }
        }
    }

    // Stage 4: Provenance tracking & InfoSheet audit entry.
    let provenance_summary = opts.provenance_summary.clone().unwrap_or_else(|| format_provenance(spec, rows_placed, cols_placed));
    if !matches!(spec.source, LinkSource::Clipboard) {
        let link_id = opts.link_id.clone().unwrap_or_else(|| format!("link-{}", target.linked_regions.len() + 1));
        let mut persisted_spec = spec.clone();
        persisted_spec.filter.baseline = opts.baseline.or(spec.filter.baseline);
        persisted_spec.filter.exclude_row_predicate = opts.exclude_row_predicate.clone().or_else(|| spec.filter.exclude_row_predicate.clone());
        target.add_linked_region(LinkedDataRegion {
            id: link_id,
            spec: persisted_spec,
            target_range,
            provenance_summary: provenance_summary.clone(),
        })?;
    }
    if let Some(ref mut info) = info_sheet {
        if !info.notes_markdown.is_empty() {
            info.notes_markdown.push('\n');
        }
        info.notes_markdown.push_str(&format!("- Import `{}`: {provenance_summary}", target.title));
    }

    Ok(ImportPipelineReport { rows_placed, cols_placed, target_range, provenance_summary })
}

fn internal_col_max_sub(arch: &TableArchetype, col: usize) -> usize {
    match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count().saturating_sub(1),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count().saturating_sub(1),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count().saturating_sub(1),
        TableArchetype::Nested(t) => t.groups.get(col).map_or(0, |g| g.biological_unit_count().saturating_sub(1)),
        _ => 0,
    }
}

fn validate_internal_data_extract_budget(sheet: &DataSheet, range: SheetCellRange) -> Result<(), TransformError> {
    let row_count = sheet.archetype.row_count();
    let col_count = sheet.archetype.column_count();
    if row_count == 0
        || col_count == 0
        || range.end.row < range.start.row
        || range.end.col < range.start.col
        || range.start.row >= row_count
        || range.start.col >= col_count
    {
        return Ok(());
    }
    let end_row = range.end.row.min(row_count - 1);
    let end_col = range.end.col.min(col_count - 1);
    let row_span = end_row - range.start.row + 1;
    let mut width = 0_usize;
    for c in range.start.col..=end_col {
        let max_sub = internal_col_max_sub(&sheet.archetype, c);
        let s_min = if c == range.start.col { range.start.subcol } else { 0 };
        let s_max = if c == range.end.col { range.end.subcol.min(max_sub) } else { max_sub };
        if s_max >= s_min {
            width = width.checked_add(s_max - s_min + 1).ok_or_else(|| TransformError::InvalidSpec {
                message: format!("internal data import cell allocation overflows budget cap {MAX_IMPORT_CELLS}"),
            })?;
        }
    }
    if width == 0 {
        return Ok(());
    }
    check_transform_cell_budget(row_span.checked_mul(width), "internal data import")
}

fn extract_stage1(
    source: &LinkSource,
    filter: &ImportFilterSpec,
    payload: ImportSourcePayload<'_>,
) -> Result<(Vec<Vec<Cell>>, Vec<Cell>), TransformError> {
    match (source, payload) {
        (LinkSource::InternalSheet { range, .. }, ImportSourcePayload::InternalData(sheet)) => {
            validate_internal_data_extract_budget(sheet, *range)?;
            Ok((sheet.extract_cell_grid(*range), Vec::new()))
        }
        (LinkSource::InternalSheet { tab_id, range, .. }, ImportSourcePayload::InternalResult(res)) => {
            let tab = if let Some(tid) = tab_id { res.tabs.iter().find(|t| &t.id == tid) } else { res.tabs.first() };
            let Some(tab) = tab else {
                return Err(TransformError::InvalidInput { message: String::from("referenced ResultTab not found") });
            };
            let col_count = tab.rows.iter().map(Vec::len).max().unwrap_or(0).max(tab.column_headers.len());
            if tab.rows.is_empty()
                || col_count == 0
                || range.end.row < range.start.row
                || range.end.col < range.start.col
                || range.start.row >= tab.rows.len()
                || range.start.col >= col_count
            {
                return Ok((Vec::new(), Vec::new()));
            }
            let end_r = range.end.row.min(tab.rows.len() - 1);
            let end_c = range.end.col.min(col_count - 1);
            let row_span = end_r - range.start.row + 1;
            let width = end_c - range.start.col + 1;
            check_transform_cell_budget(row_span.checked_mul(width), "internal result import")?;
            let mut grid = Vec::with_capacity(row_span);
            for r in range.start.row..=end_r {
                let row = &tab.rows[r];
                let mut row_cells = Vec::with_capacity(width);
                for c in range.start.col..=end_c {
                    row_cells.push(row.get(c).map_or_else(Cell::empty, result_cell_to_cell));
                }
                grid.push(row_cells);
            }
            Ok((grid, Vec::new()))
        }
        (LinkSource::ExternalDelimitedFile { delimiter, decimal_separator, .. }, ImportSourcePayload::DelimitedText { text, .. }) => {
            Ok((parse_delimited_text(text, *delimiter, *decimal_separator, filter.parse_asterisk_exclusions)?, Vec::new()))
        }
        (LinkSource::Clipboard, ImportSourcePayload::DelimitedText { text, delimiter, decimal_separator }) => {
            Ok((parse_delimited_text(text, delimiter, decimal_separator, filter.parse_asterisk_exclusions)?, Vec::new()))
        }
        (LinkSource::Clipboard, ImportSourcePayload::InternalData(sheet)) => {
            let full = SheetCellRange::new(CellCoord::default(), CellCoord::new(usize::MAX, usize::MAX, usize::MAX));
            validate_internal_data_extract_budget(sheet, full)?;
            Ok((sheet.extract_cell_grid(full), Vec::new()))
        }
        _ => Err(TransformError::InvalidInput { message: String::from("mismatched LinkSource and ImportSourcePayload") }),
    }
}

fn format_ci_bound(bound: CiBound, is_lower: bool) -> String {
    match bound {
        CiBound::Finite(v) => {
            let clean = if v == 0.0 { 0.0 } else { v };
            if clean.fract() == 0.0 && clean.abs() < 1e12 { format!("{clean:.0}") } else { clean.to_string() }
        }
        CiBound::Open => String::from(if is_lower { "-inf" } else { "+inf" }),
    }
}

fn result_cell_to_cell(rc: &ResultCell) -> Cell {
    match rc {
        ResultCell::Empty => Cell::empty(),
        ResultCell::Number { value, .. } => Cell::number(*value),
        ResultCell::PValue { raw, .. } => Cell::number(*raw),
        ResultCell::Text(s) => Cell::text(s.clone()),
        ResultCell::Boolean(b) => Cell::number(if *b { 1.0 } else { 0.0 }),
        ResultCell::ConfidenceInterval { lower, upper } => {
            Cell::text(format!("[{}, {}]", format_ci_bound(*lower, true), format_ci_bound(*upper, false)))
        }
    }
}

fn map_delimited_scan_error(err: DelimitedScanError) -> TransformError {
    match err {
        DelimitedScanError::UnexpectedQuoteInUnquotedField { .. } => {
            TransformError::InvalidInput { message: String::from("unexpected quote inside unquoted delimited field") }
        }
        DelimitedScanError::UnexpectedCharAfterClosingQuote { ch, .. } => {
            TransformError::InvalidInput { message: format!("unexpected character `{ch}` after closing quote in delimited text") }
        }
        DelimitedScanError::UnclosedQuotedField { .. } => {
            TransformError::InvalidInput { message: String::from("unclosed quoted field at end of delimited text") }
        }
    }
}

fn parse_delimited_text(text: &str, delimiter: char, decimal_sep: char, parse_asterisk: bool) -> Result<Vec<Vec<Cell>>, TransformError> {
    let max_cols = MAX_DATA_SETS.saturating_mul(MAX_SUBCOLUMNS);
    let max_cells = MAX_IMPORT_CELLS;

    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current_row: Vec<Cell> = Vec::new();
    let mut total_cells = 0_usize;
    let scan_opts = DelimitedScanOptions { delimiter, quote_char: (delimiter != '"').then_some('"') };

    scan_delimited(text, scan_opts, |field| {
        if field.end_of_record && field.field_index == 0 && !field.quoted && field.text.is_empty() {
            return Ok(());
        }
        if current_row.len() >= max_cols {
            return Err(TransformError::InvalidSpec { message: format!("delimited column count exceeds limit {max_cols}") });
        }
        let next_total = total_cells.saturating_add(1);
        if next_total > max_cells {
            return Err(TransformError::InvalidSpec { message: format!("delimited cell count exceeds limit {max_cells}") });
        }
        total_cells = next_total;
        current_row.push(Cell::parse_import_token(&field.text, decimal_sep, parse_asterisk));
        if field.end_of_record {
            if rows.len() >= MAX_ROWS {
                return Err(TransformError::InvalidSpec { message: format!("delimited row count exceeds limit {MAX_ROWS}") });
            }
            rows.push(std::mem::take(&mut current_row));
        }
        Ok(())
    })
    .map_err(|err| match err {
        DelimitedVisitorError::Syntax(s) => map_delimited_scan_error(s),
        DelimitedVisitorError::Callback(e) => e,
    })?;

    Ok(rows)
}

fn apply_stage2_filter(
    mut grid: Vec<Vec<Cell>>,
    filter: &ImportFilterSpec,
    opts: &ImportPipelineOptions,
    header_row: &mut Vec<Cell>,
    info_sheet: Option<&InfoSheet>,
) -> Result<Vec<Vec<Cell>>, TransformError> {
    if let Some((r0, r1)) = filter.row_window {
        if r0 > r1 {
            return Err(TransformError::InvalidSpec { message: format!("row_window start {r0} > end {r1}") });
        }
        if r0 >= grid.len() {
            grid.clear();
        } else {
            let end = r1.min(grid.len() - 1);
            grid = grid[r0..=end].to_vec();
        }
    }

    if let Some((c0, c1)) = filter.col_window {
        if c0 > c1 {
            return Err(TransformError::InvalidSpec { message: format!("col_window start {c0} > end {c1}") });
        }
        for row in &mut grid {
            if c0 >= row.len() {
                row.clear();
            } else {
                let end = c1.min(row.len() - 1);
                *row = row[c0..=end].to_vec();
            }
        }
    }

    if let Some(k) = filter.decimate_every_k {
        if k == 0 {
            return Err(TransformError::InvalidSpec { message: String::from("decimate_every_k must be >= 1") });
        }
        grid = grid.into_iter().step_by(k).collect();
    }

    if let Some(group_col) = filter.unstack_group_column {
        let (unstacked, hdr) = unstack_grid_by_group(&grid, group_col)?;
        grid = unstacked;
        *header_row = hdr;
    }

    if let Some((base_col, op)) = opts.baseline.or(filter.baseline) {
        apply_grid_baseline(&mut grid, base_col, op)?;
    }

    if let Some(pred) = opts.exclude_row_predicate.as_deref().or(filter.exclude_row_predicate.as_deref()) {
        apply_row_exclusion_predicate(&mut grid, header_row, pred, info_sheet)?;
    }

    for &(r, c) in &opts.exclude_coords {
        if let Some(cell) = grid.get_mut(r).and_then(|row| row.get_mut(c)) {
            cell.excluded = true;
        }
    }

    Ok(grid)
}

fn unstack_grid_by_group(grid: &[Vec<Cell>], group_col: usize) -> Result<(Vec<Vec<Cell>>, Vec<Cell>), TransformError> {
    let max_width = grid.iter().map(Vec::len).max().unwrap_or(0);
    if !grid.is_empty() && max_width > 2 {
        return Err(TransformError::InvalidSpec {
            message: format!(
                "unstack_group_column requires a 2-column (group, value) window, found {max_width} columns; narrow with col_window first"
            ),
        });
    }
    if !grid.is_empty() && max_width < 2 {
        return Err(TransformError::InvalidSpec {
            message: String::from("unstack_group_column requires at least one value column alongside the group column"),
        });
    }

    let mut group_order: Vec<String> = Vec::new();
    let mut buckets: BTreeMap<String, Vec<Cell>> = BTreeMap::new();

    for row in grid {
        let Some(g_cell) = row.get(group_col) else {
            return Err(TransformError::InvalidSpec { message: format!("unstack_group_column {group_col} out of bounds") });
        };
        let key = match &g_cell.value {
            CellValue::Text(s) => s.trim().to_owned(),
            CellValue::Number(v) if v.is_finite() => {
                let norm = if *v == 0.0 { 0.0 } else { *v };
                format!("{norm}")
            }
            _ => continue,
        };
        if key.is_empty() {
            continue;
        }
        let val_cell = row.iter().enumerate().find(|&(idx, _)| idx != group_col).map_or_else(Cell::empty, |(_, c)| c.clone());
        if !buckets.contains_key(&key) {
            group_order.push(key.clone());
        }
        buckets.entry(key).or_default().push(val_cell);
    }

    let max_rows = buckets.values().map(Vec::len).max().unwrap_or(0);
    let cols = group_order.len();
    validate_rows(max_rows)?;
    validate_data_sets(cols)?;
    check_transform_cell_budget(max_rows.checked_mul(cols), "unstacked grid")?;
    let mut out = vec![vec![Cell::empty(); cols]; max_rows];
    for (c, key) in group_order.iter().enumerate() {
        if let Some(vals) = buckets.get(key) {
            for (r, cell) in vals.iter().enumerate() {
                out[r][c] = cell.clone();
            }
        }
    }
    let header = group_order.into_iter().map(Cell::text).collect();
    Ok((out, header))
}

fn apply_grid_baseline(grid: &mut [Vec<Cell>], base_col: usize, op: BaselineOperation) -> Result<(), TransformError> {
    for row in grid.iter_mut() {
        let Some(base_val) = row.get(base_col).and_then(Cell::active_number) else {
            return Err(TransformError::InvalidInput { message: format!("baseline column {base_col} missing active numeric value") });
        };
        for cell in row.iter_mut() {
            let Some(v) = cell.value.finite_f64() else {
                continue;
            };
            let res = match op {
                BaselineOperation::Difference => v - base_val,
                BaselineOperation::Ratio => {
                    if base_val == 0.0 {
                        f64::NAN
                    } else {
                        v / base_val
                    }
                }
                BaselineOperation::PercentChange => {
                    if base_val == 0.0 {
                        f64::NAN
                    } else {
                        100.0 * (v - base_val) / base_val
                    }
                }
                BaselineOperation::FractionalChange => {
                    if base_val == 0.0 {
                        f64::NAN
                    } else {
                        (v - base_val) / base_val
                    }
                }
            };
            cell.value = CellValue::from_finite_or_empty(res);
        }
    }
    Ok(())
}

struct ImportRowEnv<'a> {
    row: &'a [Cell],
    header_row: &'a [Cell],
    col_labels: &'a [String],
    scalars: ScalarEnv<'a>,
}

impl crate::formula::FormulaContext for ImportRowEnv<'_> {
    fn row_count(&self) -> usize {
        1
    }

    fn resolve_column_index(&self, name: &str) -> Option<usize> {
        let needle = name.trim();
        if let Some(pos) = self.header_row.iter().position(|c| c.active_text().is_some_and(|t| t.trim() == needle)) {
            return Some(pos);
        }
        if let Some(pos) = self.header_row.iter().position(|c| c.active_text().is_some_and(|t| t.trim().eq_ignore_ascii_case(needle))) {
            return Some(pos);
        }
        self.col_labels.iter().position(|lbl| lbl.eq_ignore_ascii_case(needle))
    }

    fn column_count(&self) -> usize {
        self.col_labels.len()
    }

    fn column_values(&self, col_idx: usize) -> Option<Vec<crate::formula::FormulaValue>> {
        if col_idx >= self.col_labels.len() {
            return None;
        }
        let val = self.row.get(col_idx).map_or(crate::formula::FormulaValue::Missing, crate::formula::FormulaValue::from_cell);
        Some(vec![val])
    }

    fn resolve_info_constant(&self, name: &str) -> Option<crate::formula::FormulaValue> {
        crate::formula::FormulaContext::resolve_info_constant(&self.scalars, name)
    }

    fn resolve_scalar(&self, name: &str) -> Option<crate::formula::FormulaValue> {
        crate::formula::FormulaContext::resolve_scalar(&self.scalars, name)
    }
}

fn apply_row_exclusion_predicate(
    grid: &mut [Vec<Cell>],
    header_row: &[Cell],
    pred: &str,
    info_sheet: Option<&InfoSheet>,
) -> Result<(), TransformError> {
    let expr = parse_formula(pred)?;
    if grid.is_empty() {
        return Ok(());
    }
    let col_count = grid.iter().map(Vec::len).max().unwrap_or(0).max(header_row.len());
    let col_labels: Vec<String> = (0..col_count).map(default_column_label).collect();
    for (r, row) in grid.iter_mut().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let scalars = ScalarEnv::new(vec![("$row", (r + 1) as f64), ("$index", r as f64)], info_sheet);
        let excluded = {
            let env = ImportRowEnv { row, header_row, col_labels: &col_labels, scalars };
            eval_scalar_expr(&expr, &env)?.as_bool() == Some(true)
        };
        if excluded {
            for cell in row.iter_mut() {
                cell.excluded = true;
            }
        }
    }
    Ok(())
}

fn collapse_contiguous_blank_rows(grid: Vec<Vec<Cell>>) -> Vec<Vec<Cell>> {
    let mut out = Vec::with_capacity(grid.len());
    let mut prev_blank = false;
    for row in grid {
        let is_blank = row.iter().all(Cell::is_empty);
        if is_blank && prev_blank {
            continue;
        }
        prev_blank = is_blank;
        out.push(row);
    }
    out
}

fn place_grid(target: &mut DataSheet, grid: &[Vec<Cell>], mode: PlacementMode, anchor: CellCoord) -> Result<SheetCellRange, TransformError> {
    let subs_per_col = subcolumns_per_data_column(&target.archetype).max(1);
    if anchor.subcol >= subs_per_col {
        return Err(TransformError::InvalidSpec {
            message: format!("anchor.subcol {} out of bounds for target subcolumn count {subs_per_col}", anchor.subcol),
        });
    }
    if grid.is_empty() {
        return Ok(SheetCellRange::new(anchor, anchor));
    }
    let existing_rows = target.archetype.row_count();
    let start_row = match mode {
        PlacementMode::Overwrite | PlacementMode::InsertRows => anchor.row,
        PlacementMode::AppendRows => existing_rows,
        PlacementMode::AppendColumns => anchor.row,
    };
    let start_col = match mode {
        PlacementMode::AppendColumns => target.archetype.column_count(),
        _ => anchor.col,
    };

    let needed_rows =
        if mode == PlacementMode::InsertRows { existing_rows.max(start_row).checked_add(grid.len()) } else { start_row.checked_add(grid.len()) }
            .ok_or_else(|| TransformError::InvalidSpec { message: String::from("target row offset overflow") })?;
    let max_row_width = grid.iter().map(Vec::len).max().unwrap_or(0);
    let needed_cols = validate_grid_col_bounds(&target.archetype, start_col, anchor.subcol, max_row_width)?;
    let target_rows = existing_rows.max(needed_rows);
    if max_row_width > 0 {
        let min_cols = needed_cols.max(target.archetype.column_count());
        ensure_sheet_columns(&mut target.archetype, min_cols, target_rows)?;
    } else {
        ensure_sheet_rows(&mut target.archetype, target_rows)?;
    }
    if mode == PlacementMode::InsertRows && start_row < existing_rows {
        target.archetype.shift_rows_down(start_row, grid.len(), existing_rows);
        for note in &mut target.exclusion_notes {
            if note.coord.row >= start_row {
                note.coord.row += grid.len();
            }
        }
        target.exclusion_notes.sort_by_key(|n| n.coord);
    }

    let mut max_col = start_col;
    let mut max_sub = anchor.subcol;
    for (dr, row_cells) in grid.iter().enumerate() {
        let r = start_row + dr;
        for (dc, cell) in row_cells.iter().enumerate() {
            let (col, subcol) = resolve_target_coord(&target.archetype, start_col, anchor.subcol, dc)?;
            let coord = CellCoord::new(r, col, subcol);
            if let Some(slot) = target.archetype.data_cell_mut(coord) {
                *slot = cell.clone();
                max_col = max_col.max(col);
                max_sub = max_sub.max(subcol);
            }
        }
    }

    if max_row_width > 0
        && let TableArchetype::MultipleVariables(mv) = &mut target.archetype
    {
        for col in start_col..=max_col {
            if let Some(var) = mv.variables.get_mut(col) {
                var.refresh_inferred_role();
            }
        }
    }
    target.prune_stale_exclusion_notes();

    Ok(SheetCellRange::new(CellCoord::new(start_row, start_col, anchor.subcol), CellCoord::new(start_row + grid.len() - 1, max_col, max_sub)))
}

fn subcolumns_per_data_column(arch: &TableArchetype) -> usize {
    match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
        _ => 1,
    }
}

fn validate_grid_col_bounds(arch: &TableArchetype, base_col: usize, base_sub: usize, width: usize) -> Result<usize, TransformError> {
    let subs_per_col = subcolumns_per_data_column(arch).max(1);
    let span = width.max(1);
    let total_flat = base_col
        .checked_mul(subs_per_col)
        .and_then(|v| v.checked_add(base_sub))
        .and_then(|v| v.checked_add(span))
        .ok_or_else(|| TransformError::InvalidSpec { message: String::from("target column/subcolumn offset overflow") })?;
    if width == 0 { Ok(arch.column_count()) } else { Ok(total_flat.div_ceil(subs_per_col)) }
}

fn resolve_target_coord(arch: &TableArchetype, base_col: usize, base_sub: usize, offset: usize) -> Result<(usize, usize), TransformError> {
    let subs_per_col = subcolumns_per_data_column(arch);
    if subs_per_col <= 1 {
        let col =
            base_col.checked_add(offset).ok_or_else(|| TransformError::InvalidSpec { message: String::from("target column offset overflow") })?;
        Ok((col, 0))
    } else {
        let flat =
            base_sub.checked_add(offset).ok_or_else(|| TransformError::InvalidSpec { message: String::from("target subcolumn offset overflow") })?;
        let col = base_col
            .checked_add(flat / subs_per_col)
            .ok_or_else(|| TransformError::InvalidSpec { message: String::from("target column offset overflow") })?;
        Ok((col, flat % subs_per_col))
    }
}

/// Enforce [`MAX_IMPORT_CELLS`] for a rectangular `(rows * cols * subcolumns) + (rows * extra_cols_per_row)`
/// allocation using overflow-checked arithmetic prior to heap allocation.
pub(crate) fn validate_cell_budget(
    rows: usize,
    cols: usize,
    subcolumns: usize,
    extra_cols_per_row: usize,
    context: &str,
) -> Result<(), TransformError> {
    let per_col = rows.checked_mul(subcolumns);
    let data_cells = per_col.and_then(|v| v.checked_mul(cols));
    let extra_cells = rows.checked_mul(extra_cols_per_row);
    let total = data_cells.and_then(|d| extra_cells.and_then(|e| d.checked_add(e)));
    check_transform_cell_budget(total, context)
}

fn check_transform_cell_budget(total_cells: Option<usize>, context: &str) -> Result<(), TransformError> {
    let total = total_cells
        .ok_or_else(|| TransformError::InvalidSpec { message: format!("{context} cell allocation overflows budget cap {MAX_IMPORT_CELLS}") })?;
    if total > MAX_IMPORT_CELLS {
        return Err(TransformError::InvalidSpec {
            message: format!("{context} cell allocation {total} exceeds import budget limit {MAX_IMPORT_CELLS}"),
        });
    }
    Ok(())
}

fn sum_column_growth_cells(
    col_dims: impl Iterator<Item = (usize, usize)>,
    existing_cols: usize,
    effective_cols: usize,
    effective_rows: usize,
    extra_col_subs: usize,
    initial_cells: Option<usize>,
) -> Result<(), TransformError> {
    let mut total = initial_cells;
    for (col_rows, col_subs) in col_dims {
        let rows = col_rows.max(effective_rows);
        let col_cells = rows.checked_mul(col_subs);
        total = total.and_then(|acc| col_cells.and_then(|c| acc.checked_add(c)));
        check_transform_cell_budget(total, "target sheet")?;
    }
    let extra_cols = effective_cols.saturating_sub(existing_cols);
    if extra_cols > 0 {
        let added = extra_cols.checked_mul(effective_rows).and_then(|v| v.checked_mul(extra_col_subs));
        total = total.and_then(|acc| added.and_then(|c| acc.checked_add(c)));
    }
    check_transform_cell_budget(total, "target sheet")
}

fn validate_archetype_growth_budget(arch: &TableArchetype, min_cols: usize, min_rows: usize) -> Result<(), TransformError> {
    let effective_rows = arch.row_count().max(min_rows);
    validate_rows(effective_rows)?;
    let effective_cols = arch.column_count().max(min_cols);
    validate_data_sets(effective_cols)?;
    let sum_ds = |data_sets: &[DataSetColumn], default_subs: usize, initial: Option<usize>| {
        let dims = data_sets.iter().map(|ds| (ds.row_count(), ds.subcolumn_count().max(default_subs)));
        sum_column_growth_cells(dims, data_sets.len(), effective_cols, effective_rows, default_subs, initial)
    };
    match arch {
        TableArchetype::Xy(t) => {
            t.x_mode.validate()?;
            t.y_subcolumn_mode.validate()?;
            let x_cells = t.x_cells.len().max(effective_rows).checked_mul(t.x_mode.subcolumn_count());
            sum_ds(&t.data_sets, t.y_subcolumn_mode.subcolumn_count(), x_cells)
        }
        TableArchetype::Column(t) => {
            t.subcolumn_mode.validate()?;
            sum_ds(&t.data_sets, t.subcolumn_mode.subcolumn_count(), Some(0))
        }
        TableArchetype::Grouped(t) => {
            t.subcolumn_mode.validate()?;
            sum_ds(&t.data_sets, t.subcolumn_mode.subcolumn_count(), Some(0))
        }
        TableArchetype::Contingency(t) => sum_ds(&t.data_sets, 1, Some(0)),
        TableArchetype::Survival(t) => sum_ds(&t.cohorts, 1, Some(t.times.len().max(effective_rows))),
        TableArchetype::PartsOfWhole(t) => sum_ds(&t.data_sets, 1, Some(0)),
        TableArchetype::MultipleVariables(mv) => {
            let dims = mv.variables.iter().map(|v| (v.cells.len(), 1));
            sum_column_growth_cells(dims, mv.variables.len(), effective_cols, effective_rows, 1, Some(0))
        }
        TableArchetype::Nested(t) => {
            for g in &t.groups {
                validate_subcolumns(g.biological_unit_count())?;
            }
            let extra_units = t.groups.first().map_or(1, NestedGroupColumn::biological_unit_count).max(1);
            let dims = t.groups.iter().map(|g| (g.cells.len(), g.biological_unit_count()));
            sum_column_growth_cells(dims, t.groups.len(), effective_cols, effective_rows, extra_units, Some(0))
        }
    }
}

fn ensure_sheet_rows(arch: &mut TableArchetype, min_rows: usize) -> Result<(), TransformError> {
    validate_archetype_growth_budget(arch, arch.column_count(), min_rows)?;
    match arch {
        TableArchetype::Xy(t) => {
            let x_subs = t.x_mode.subcolumn_count();
            if t.x_cells.len() < min_rows {
                t.x_cells.resize_with(min_rows, || vec![Cell::empty(); x_subs]);
            }
            if t.row_titles.len() < min_rows {
                t.row_titles.resize_with(min_rows, String::new);
            }
            let y_subs = t.y_subcolumn_mode.subcolumn_count();
            for ds in &mut t.data_sets {
                ds.ensure_rows(min_rows, y_subs)?;
            }
        }
        TableArchetype::Column(t) => {
            if t.row_titles.len() < min_rows {
                t.row_titles.resize_with(min_rows, String::new);
            }
            let subs = t.subcolumn_mode.subcolumn_count();
            for ds in &mut t.data_sets {
                ds.ensure_rows(min_rows, subs)?;
            }
        }
        TableArchetype::Grouped(t) => {
            if t.row_titles.len() < min_rows {
                t.row_titles.resize_with(min_rows, String::new);
            }
            if let Some(spacing) = &mut t.row_numeric_spacing
                && spacing.len() < min_rows
            {
                spacing.resize_with(min_rows, || None);
            }
            let subs = t.subcolumn_mode.subcolumn_count();
            for ds in &mut t.data_sets {
                ds.ensure_rows(min_rows, subs)?;
            }
        }
        TableArchetype::Contingency(t) => {
            if t.row_titles.len() < min_rows {
                t.row_titles.resize_with(min_rows, String::new);
            }
            for ds in &mut t.data_sets {
                ds.ensure_rows(min_rows, 1)?;
            }
        }
        TableArchetype::Survival(t) => {
            if t.times.len() < min_rows {
                t.times.resize_with(min_rows, Cell::empty);
            }
            if t.row_titles.len() < min_rows {
                t.row_titles.resize_with(min_rows, String::new);
            }
            for ds in &mut t.cohorts {
                ds.ensure_rows(min_rows, 1)?;
            }
        }
        TableArchetype::PartsOfWhole(t) => {
            if t.slice_labels.len() < min_rows {
                t.slice_labels.resize_with(min_rows, String::new);
            }
            for ds in &mut t.data_sets {
                ds.ensure_rows(min_rows, 1)?;
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            if mv.row_titles.len() < min_rows {
                mv.row_titles.resize_with(min_rows, String::new);
            }
            for var in &mut mv.variables {
                if var.cells.len() < min_rows {
                    var.cells.resize_with(min_rows, Cell::empty);
                }
            }
        }
        TableArchetype::Nested(t) => {
            if t.row_titles.len() < min_rows {
                t.row_titles.resize_with(min_rows, String::new);
            }
            for grp in &mut t.groups {
                let units = grp.biological_unit_count();
                if grp.cells.len() < min_rows {
                    grp.cells.resize_with(min_rows, || vec![Cell::empty(); units]);
                }
            }
        }
    }
    Ok(())
}

fn push_dataset_columns(cols: &mut Vec<DataSetColumn>, min_cols: usize, row_count: usize, subs: usize) -> Result<(), TransformError> {
    while cols.len() < min_cols {
        let idx = cols.len();
        cols.push(DataSetColumn::new(default_column_label(idx), row_count, subs)?);
    }
    Ok(())
}

fn ensure_sheet_columns(arch: &mut TableArchetype, min_cols: usize, min_rows: usize) -> Result<(), TransformError> {
    let row_count = arch.row_count().max(min_rows);
    validate_archetype_growth_budget(arch, min_cols, row_count)?;
    ensure_sheet_rows(arch, row_count)?;
    match arch {
        TableArchetype::Xy(t) => push_dataset_columns(&mut t.data_sets, min_cols, row_count, t.y_subcolumn_mode.subcolumn_count())?,
        TableArchetype::Column(t) => push_dataset_columns(&mut t.data_sets, min_cols, row_count, t.subcolumn_mode.subcolumn_count())?,
        TableArchetype::Grouped(t) => push_dataset_columns(&mut t.data_sets, min_cols, row_count, t.subcolumn_mode.subcolumn_count())?,
        TableArchetype::Contingency(t) => push_dataset_columns(&mut t.data_sets, min_cols, row_count, 1)?,
        TableArchetype::Survival(t) => push_dataset_columns(&mut t.cohorts, min_cols, row_count, 1)?,
        TableArchetype::PartsOfWhole(t) => push_dataset_columns(&mut t.data_sets, min_cols, row_count, 1)?,
        TableArchetype::MultipleVariables(mv) => {
            while mv.variables.len() < min_cols {
                let idx = mv.variables.len();
                mv.variables.push(VariableColumn::new(default_column_label(idx), row_count)?);
            }
        }
        TableArchetype::Nested(t) => {
            let units = t.groups.first().map_or(1, NestedGroupColumn::biological_unit_count).max(1);
            while t.groups.len() < min_cols {
                let idx = t.groups.len();
                t.groups.push(NestedGroupColumn::new(default_column_label(idx), units, row_count)?);
            }
        }
    }
    Ok(())
}

fn set_column_title_if_possible(arch: &mut TableArchetype, col: usize, title: &str) {
    match arch {
        TableArchetype::Xy(t) => {
            if let Some(ds) = t.data_sets.get_mut(col) {
                ds.title = title.to_owned();
            }
        }
        TableArchetype::Column(t) => {
            if let Some(ds) = t.data_sets.get_mut(col) {
                ds.title = title.to_owned();
            }
        }
        TableArchetype::Grouped(t) => {
            if let Some(ds) = t.data_sets.get_mut(col) {
                ds.title = title.to_owned();
            }
        }
        TableArchetype::Contingency(t) => {
            if let Some(ds) = t.data_sets.get_mut(col) {
                ds.title = title.to_owned();
            }
        }
        TableArchetype::Survival(t) => {
            if let Some(ds) = t.cohorts.get_mut(col) {
                ds.title = title.to_owned();
            }
        }
        TableArchetype::PartsOfWhole(t) => {
            if let Some(ds) = t.data_sets.get_mut(col) {
                ds.title = title.to_owned();
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            if let Some(var) = mv.variables.get_mut(col) {
                var.title = title.to_owned();
            }
        }
        TableArchetype::Nested(t) => {
            if let Some(grp) = t.groups.get_mut(col) {
                grp.title = title.to_owned();
            }
        }
    }
}

fn format_provenance(spec: &ImportLinkSpec, rows: usize, cols: usize) -> String {
    let origin = match &spec.source {
        LinkSource::Clipboard => String::from("clipboard"),
        LinkSource::InternalSheet { sheet_id, .. } => format!("sheet:{sheet_id}"),
        LinkSource::ExternalDelimitedFile { path, .. } => format!("file:{path}"),
    };
    let tx = if spec.placement.transpose { ", transposed" } else { "" };
    format!("Linked {rows}x{cols} from {origin}{tx}")
}

#[cfg(test)]
mod dialect_tests {
    use curvebook_doc::{Cell, CellValue};

    use super::parse_delimited_text;
    use crate::error::TransformError;

    #[test]
    fn transform_dialect_parses_quoted_numbers_skips_blank_lines_and_keeps_quoted_empty_row() {
        let input = "\u{FEFF}\"42.5\",10*\r\n\r\n\"\"\n\"a\"\"b\nc\",\n";
        let rows = parse_delimited_text(input, ',', '.', true).unwrap();
        assert_eq!(rows.len(), 3);
        // Transform sends quoted fields through Cell::parse_import_token (numeric parsed as Number).
        assert_eq!(rows[0][0], Cell::number(42.5));
        assert_eq!(rows[0][1], Cell { value: CellValue::Number(10.0), excluded: true });
        // Standalone unquoted blank line skipped, while quoted empty line `""` is retained.
        assert_eq!(rows[1], vec![Cell::empty()]);
        // Quoted newline + doubled quote + trailing empty field preserved.
        assert_eq!(rows[2], vec![Cell::text("a\"b\nc"), Cell::empty()]);
    }

    #[test]
    fn transform_dialect_handles_quote_as_delimiter_and_maps_syntax_errors() {
        let rows = parse_delimited_text("1\"2\"3\n4\"5", '"', '.', true).unwrap();
        assert_eq!(rows, vec![vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)], vec![Cell::number(4.0), Cell::number(5.0)]]);

        assert!(matches!(parse_delimited_text("a\"b,1", ',', '.', true), Err(TransformError::InvalidInput { .. })));
        assert!(matches!(parse_delimited_text("\"a\"x,1", ',', '.', true), Err(TransformError::InvalidInput { .. })));
        assert!(matches!(parse_delimited_text("\"unclosed", ',', '.', true), Err(TransformError::InvalidInput { .. })));
    }
}
