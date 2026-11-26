//! Four-stage [`ImportLinkSpec`] data import, filtering, reshaping, placement, and
//! circular-link detection pipeline (`docs/implementation/data-tables-and-transforms.md` §6).

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{
    BaselineOperation, Cell, CellCoord, CellValue, CiBound, DataSetColumn, DataSheet, DelimitedScanError, DelimitedScanOptions,
    DelimitedVisitorError, ImportFilterSpec, ImportLinkSpec, InfoSheet, LinkSource, LinkedDataRegion, MAX_DATA_SETS, MAX_ROWS, MAX_SUBCOLUMNS,
    NestedGroupColumn, PlacementMode, ResultCell, ResultSheet, SheetCellRange, SheetId, TableArchetype, VariableColumn, XColumnMode,
    default_column_label, scan_delimited, transpose_cell_grid, validate_data_sets, validate_rows, validate_sheet_cells, validate_subcolumns,
    validate_title,
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
    let is_delimited = matches!(payload, ImportSourcePayload::DelimitedText { .. });
    let (raw_grid, mut header_row) = extract_stage1(&spec.source, &spec.filter, payload)?;

    // Stage 2: Row/column window, decimation, unstacking, baseline, and exclusion rules.
    let promote_first_row_header =
        is_delimited && spec.placement.propagate_title_header && !spec.placement.transpose && spec.filter.unstack_group_column.is_none();
    let mut shaped = apply_stage2_filter(raw_grid, &spec.filter, opts, &mut header_row, promote_first_row_header, info_sheet.as_deref())?;

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

    // Validate all propagated column titles before mutating `target`.
    let propagate_headers = spec.placement.propagate_title_header && !spec.placement.transpose;
    let validated_headers: Vec<Option<String>> = if propagate_headers {
        let mut titles = Vec::with_capacity(header_row.len());
        for hdr in &header_row {
            let title_opt = header_cell_title(hdr);
            if let Some(ref title) = title_opt {
                validate_title(title)?;
            }
            titles.push(title_opt);
        }
        titles
    } else {
        Vec::new()
    };

    let rows_placed = shaped.len();
    let cols_placed = shaped.iter().map(Vec::len).max().unwrap_or(0);
    let target_range = place_grid(target, &shaped, spec.placement.mode, spec.placement.anchor)?;

    if propagate_headers {
        let base_col = target_range.start.col;
        for (offset, title_opt) in validated_headers.iter().enumerate() {
            if let Some(title) = title_opt.as_deref()
                && let Some(col) = base_col.checked_add(offset)
            {
                set_column_title_if_possible(&mut target.archetype, col, title);
            }
        }
    }

    // Stage 4: Provenance tracking & InfoSheet audit entry.
    let provenance_summary = opts.provenance_summary.clone().unwrap_or_else(|| format_provenance(spec, rows_placed, cols_placed));
    if !matches!(spec.source, LinkSource::Clipboard) {
        let link_id = opts.link_id.clone().unwrap_or_else(|| next_unique_link_id(&target.linked_regions));
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
            let headers = (range.start.col..=end_c).map(|c| tab.column_headers.get(c).map_or_else(Cell::empty, |h| Cell::text(h.clone()))).collect();
            Ok((grid, headers))
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
    if decimal_sep != '.' && decimal_sep != ',' {
        return Err(TransformError::InvalidSpec { message: format!("unsupported decimal separator `{decimal_sep}` (expected '.' or ',')") });
    }
    let max_cols = MAX_DATA_SETS.saturating_mul(MAX_SUBCOLUMNS);
    let max_cells = MAX_IMPORT_CELLS;

    let mut raw_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
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
        current_row.push(field.text);
        if field.end_of_record {
            if raw_rows.len() >= MAX_ROWS {
                return Err(TransformError::InvalidSpec { message: format!("delimited row count exceeds limit {MAX_ROWS}") });
            }
            raw_rows.push(std::mem::take(&mut current_row));
        }
        Ok(())
    })
    .map_err(|err| match err {
        DelimitedVisitorError::Syntax(s) => map_delimited_scan_error(s),
        DelimitedVisitorError::Callback(e) => e,
    })?;

    let (inferred_dec, inferred_thousands) =
        curvebook_doc::infer_number_separators(raw_rows.iter().take(32).flat_map(|row| row.iter().map(String::as_str)), delimiter);
    let thousands = if inferred_dec == decimal_sep { inferred_thousands } else { None };
    let rows = raw_rows
        .into_iter()
        .map(|row| row.into_iter().map(|tok| Cell::parse_import_token_with_thousands(&tok, decimal_sep, thousands, parse_asterisk)).collect())
        .collect();
    Ok(rows)
}

fn apply_stage2_filter(
    mut grid: Vec<Vec<Cell>>,
    filter: &ImportFilterSpec,
    opts: &ImportPipelineOptions,
    header_row: &mut Vec<Cell>,
    promote_first_row_header: bool,
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
        if !header_row.is_empty() {
            if c0 >= header_row.len() {
                header_row.clear();
            } else {
                let end = c1.min(header_row.len() - 1);
                *header_row = header_row[c0..=end].to_vec();
            }
        }
    }

    if promote_first_row_header && header_row.is_empty() && !grid.is_empty() {
        *header_row = grid.remove(0);
    }

    if let Some(ref row_match) = filter.row_match {
        let max_width = grid.iter().map(Vec::len).max().unwrap_or(0);
        if !grid.is_empty() && row_match.column >= max_width {
            return Err(TransformError::InvalidSpec {
                message: format!("row_match.column {} out of bounds for grid with {max_width} column(s)", row_match.column),
            });
        }
        let needle = row_match.equals.trim();
        let mut kept = Vec::with_capacity(grid.len());
        for mut row in grid {
            let matches = row.get(row_match.column).is_some_and(|cell| cell_display_trimmed(cell) == needle);
            if !matches {
                continue;
            }
            if row_match.drop_column && row_match.column < row.len() {
                row.remove(row_match.column);
            }
            kept.push(row);
        }
        grid = kept;
        if row_match.drop_column && row_match.column < header_row.len() {
            header_row.remove(row_match.column);
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

fn cell_display_trimmed(cell: &Cell) -> String {
    match &cell.value {
        CellValue::Text(s) => s.trim().to_owned(),
        CellValue::Number(v) if v.is_finite() => {
            let norm = if *v == 0.0 { 0.0 } else { *v };
            format!("{norm}")
        }
        CellValue::Empty | CellValue::Number(_) => String::new(),
    }
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
        if g_cell.excluded {
            continue;
        }
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
    let anchor_subs = subcolumns_at_column(&target.archetype, start_col);
    if anchor.subcol >= anchor_subs {
        return Err(TransformError::InvalidSpec {
            message: format!("anchor.subcol {} out of bounds for target subcolumn count {anchor_subs}", anchor.subcol),
        });
    }
    if grid.is_empty() {
        let pos = CellCoord::new(start_row, start_col, anchor.subcol);
        return Ok(SheetCellRange::new(pos, pos));
    }

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
        target.shift_rows_down(start_row, grid.len(), existing_rows);
        target.exclusion_notes.sort_by_key(|n| n.coord);
        target.plate_well_origins.sort_by_key(|o| o.coord);
    }

    let track_origins = !target.plate_well_origins.is_empty();
    let mut overwritten_coords = BTreeSet::new();
    let mut end_col_sub = (start_col, anchor.subcol);
    for (dr, row_cells) in grid.iter().enumerate() {
        let r = start_row + dr;
        let mut col = start_col;
        let mut subcol = anchor.subcol;
        for (dc, cell) in row_cells.iter().enumerate() {
            if dc > 0 {
                let col_subs = subcolumns_at_column(&target.archetype, col);
                subcol += 1;
                if subcol >= col_subs {
                    col = col.checked_add(1).ok_or_else(|| TransformError::InvalidSpec { message: String::from("target column offset overflow") })?;
                    subcol = 0;
                }
            }
            let coord = CellCoord::new(r, col, subcol);
            if let Some(slot) = target.archetype.data_cell_mut(coord) {
                *slot = cell.clone();
                if track_origins {
                    overwritten_coords.insert(coord);
                }
                if (col, subcol) > end_col_sub {
                    end_col_sub = (col, subcol);
                }
            }
        }
    }
    if !overwritten_coords.is_empty() {
        target.plate_well_origins.retain(|o| !overwritten_coords.contains(&o.coord));
    }

    if max_row_width > 0
        && let TableArchetype::MultipleVariables(mv) = &mut target.archetype
    {
        for col in start_col..=end_col_sub.0 {
            if let Some(var) = mv.variables.get_mut(col) {
                var.refresh_inferred_role();
            }
        }
    }
    target.prune_stale_exclusion_notes();
    target.prune_stale_plate_well_origins();

    Ok(SheetCellRange::new(
        CellCoord::new(start_row, start_col, anchor.subcol),
        CellCoord::new(start_row + grid.len() - 1, end_col_sub.0, end_col_sub.1),
    ))
}

fn subcolumns_at_column(arch: &TableArchetype, col: usize) -> usize {
    match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count().max(1),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count().max(1),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count().max(1),
        TableArchetype::Nested(t) => t
            .groups
            .get(col)
            .map_or_else(|| t.groups.first().map_or(1, NestedGroupColumn::biological_unit_count), NestedGroupColumn::biological_unit_count)
            .max(1),
        _ => 1,
    }
}

fn advance_col_sub(arch: &TableArchetype, base_col: usize, base_sub: usize, offset: usize) -> Result<(usize, usize), TransformError> {
    let overflow = || TransformError::InvalidSpec { message: String::from("target column/subcolumn offset overflow") };
    if let TableArchetype::Nested(t) = arch {
        let default_units = t.groups.first().map_or(1, NestedGroupColumn::biological_unit_count).max(1);
        let mut col = base_col;
        let mut cur_sub = base_sub;
        let mut rem = offset;
        while col < t.groups.len() && rem > 0 {
            let cap = t.groups[col].biological_unit_count().max(1);
            let avail = cap.saturating_sub(cur_sub);
            if rem < avail {
                return Ok((col, cur_sub + rem));
            }
            rem -= avail;
            col = col.checked_add(1).ok_or_else(overflow)?;
            cur_sub = 0;
        }
        if rem == 0 {
            return Ok((col, cur_sub));
        }
        let flat = cur_sub.checked_add(rem).ok_or_else(overflow)?;
        let final_col = col.checked_add(flat / default_units).ok_or_else(overflow)?;
        return Ok((final_col, flat % default_units));
    }
    let subs_per_col = subcolumns_at_column(arch, base_col);
    if subs_per_col <= 1 {
        let col = base_col.checked_add(offset).ok_or_else(overflow)?;
        Ok((col, 0))
    } else {
        let flat = base_sub.checked_add(offset).ok_or_else(overflow)?;
        let col = base_col.checked_add(flat / subs_per_col).ok_or_else(overflow)?;
        Ok((col, flat % subs_per_col))
    }
}

fn validate_grid_col_bounds(arch: &TableArchetype, base_col: usize, base_sub: usize, width: usize) -> Result<usize, TransformError> {
    if width == 0 {
        return Ok(arch.column_count());
    }
    let (end_col, _) = advance_col_sub(arch, base_col, base_sub, width - 1)?;
    end_col.checked_add(1).ok_or_else(|| TransformError::InvalidSpec { message: String::from("target column/subcolumn offset overflow") })
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
            let x_subs = t.x_mode.subcolumn_count().max(t.x_cells.first().map_or(0, Vec::len));
            let x_cells = t.x_cells.len().max(effective_rows).checked_mul(x_subs);
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
            let x_subs = t.x_mode.subcolumn_count().max(t.x_cells.first().map_or(0, Vec::len));
            if t.row_titles.len() < min_rows {
                t.row_titles.resize_with(min_rows, String::new);
            }
            let y_subs = t.y_subcolumn_mode.subcolumn_count();
            for ds in &mut t.data_sets {
                ds.ensure_rows(min_rows, y_subs)?;
            }
            if t.x_cells.len() < min_rows {
                if matches!(t.x_mode, XColumnMode::GeneratedSequence { .. }) && x_subs == 1 {
                    let old_len = t.x_cells.len();
                    for r in old_len..min_rows {
                        let cell = t.materialized_x_cell_at(r);
                        t.x_cells.push(vec![cell]);
                    }
                } else {
                    t.x_cells.resize_with(min_rows, || vec![Cell::empty(); x_subs]);
                }
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

fn header_cell_title(cell: &Cell) -> Option<String> {
    if cell.excluded {
        return None;
    }
    match &cell.value {
        CellValue::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CellValue::Number(v) if v.is_finite() => {
            let norm = if *v == 0.0 { 0.0 } else { *v };
            Some(format!("{norm}"))
        }
        _ => None,
    }
}

fn next_unique_link_id(existing: &[LinkedDataRegion]) -> String {
    let mut seq = existing.len().saturating_add(1);
    loop {
        let candidate = format!("link-{seq}");
        if !existing.iter().any(|r| r.id == candidate) {
            return candidate;
        }
        seq = seq.saturating_add(1);
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
        let rows = parse_delimited_text("\u{FEFF}\"42.5\",10*\r\n\r\n\"\"\n\"a\"\"b\nc\",\n", ',', '.', true).unwrap();
        let exp = vec![
            vec![Cell::number(42.5), Cell { value: CellValue::Number(10.0), excluded: true }],
            vec![Cell::empty()],
            vec![Cell::text("a\"b\nc"), Cell::empty()],
        ];
        assert_eq!(rows, exp);
    }

    #[test]
    fn transform_dialect_handles_quote_as_delimiter_and_maps_syntax_errors() {
        let rows = parse_delimited_text("1\"2\"3\n4\"5", '"', '.', true).unwrap();
        assert_eq!(rows, vec![vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)], vec![Cell::number(4.0), Cell::number(5.0)]]);
        for bad in ["a\"b,1", "\"a\"x,1", "\"unclosed"] {
            assert!(matches!(parse_delimited_text(bad, ',', '.', true), Err(TransformError::InvalidInput { .. })));
        }
    }
}

#[cfg(test)]
mod audit_w1_import_tests {
    use curvebook_doc::{
        Cell, CellCoord as Cc, CellValue, ColumnDesignHint, ColumnTable, DataSheet, DocError, GroupedTable, ImportFilterSpec as Ifs, ImportLinkSpec,
        ImportPlacementSpec as Ips, LinkSource, LinkedDataRegion, MAX_TITLE_BYTES, NestedTable, PlacementMode, SequenceProgression, SheetCellRange,
        SubcolumnMode, TableArchetype,
    };

    use super::{ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, apply_import_link_pipeline, parse_delimited_text};
    use crate::error::TransformError;
    use crate::test_helpers as th;

    fn col_dst(id: &str, cols: usize, rows: usize) -> DataSheet {
        DataSheet::new(id, "Target", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows).unwrap())).unwrap()
    }

    fn clip(filter: Ifs, placement: Ips) -> ImportLinkSpec {
        ImportLinkSpec { source: LinkSource::Clipboard, filter, placement }
    }

    fn rng(a: (usize, usize, usize), b: (usize, usize, usize)) -> SheetCellRange {
        SheetCellRange::new(Cc::new(a.0, a.1, a.2), Cc::new(b.0, b.1, b.2))
    }

    fn csv(target: &mut DataSheet, spec: &ImportLinkSpec, text: &str) -> Result<ImportPipelineReport, TransformError> {
        let p = ImportSourcePayload::DelimitedText { text, delimiter: ',', decimal_separator: '.' };
        apply_import_link_pipeline(target, spec, p, &ImportPipelineOptions::default(), None)
    }

    #[test]
    fn aud_04_12_unstack_skips_excluded_group_keys() {
        let mut target = col_dst("dst-unstack", 2, 2);
        let spec = clip(
            Ifs { unstack_group_column: Some(0), parse_asterisk_exclusions: true, ..Default::default() },
            Ips { propagate_title_header: true, ..Default::default() },
        );
        csv(&mut target, &spec, "Ctrl,10\nCtrl*,999\nDrug,20\n").unwrap();
        let col = th::as_col(&target);
        assert_eq!((col.data_sets[0].title.as_str(), col.data_sets[1].title.as_str()), ("Ctrl", "Drug"));
        assert_eq!(
            (col.data_sets[0].cells[0][0].active_number(), col.data_sets[0].cells[1][0].is_empty(), col.data_sets[1].cells[0][0].active_number()),
            (Some(10.0), true, Some(20.0))
        );
    }

    #[test]
    fn aud_04_13_place_grid_handles_nested_biological_units() {
        let mut target = DataSheet::new("dst-nested", "Nested", TableArchetype::Nested(NestedTable::new(2, 3, 2).unwrap())).unwrap();
        let anchor = |sub| clip(Ifs::default(), Ips { anchor: Cc::new(0, 0, sub), ..Default::default() });
        assert_eq!(csv(&mut target, &anchor(0), "1,2,3,4,5,6\n7,8,9,10,11,12\n").unwrap().target_range, rng((0, 0, 0), (1, 1, 2)));
        let TableArchetype::Nested(nt) = &target.archetype else { panic!("expected Nested") };
        assert_eq!(nt.groups.len(), 2);
        assert_eq!((th::num(&nt.groups[0].cells[0][2]), th::num(&nt.groups[1].cells[0][0]), th::num(&nt.groups[1].cells[1][2])), (3.0, 4.0, 12.0));
        assert_eq!(csv(&mut target, &anchor(1), "99,100\n").unwrap().target_range, rng((0, 0, 1), (0, 0, 2)));
        assert!(matches!(csv(&mut target, &anchor(3), "1\n"), Err(TransformError::InvalidSpec { .. })));
    }

    #[test]
    fn aud_04_15_place_grid_target_range_tracks_lexicographic_end() {
        let grp = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 2).unwrap();
        let mut target = DataSheet::new("dst-grp", "Grouped", TableArchetype::Grouped(grp)).unwrap();
        let spec = clip(Ifs::default(), Ips { anchor: Cc::new(0, 0, 0), ..Default::default() });
        assert_eq!(csv(&mut target, &spec, "1,2,3,4,5\n").unwrap().target_range, rng((0, 0, 0), (0, 1, 0)));
    }

    #[test]
    fn aud_04_16_ensure_sheet_rows_keeps_generated_sequence_x_cells_rectangular() {
        let xy = th::xy_gen_seq(0.0, 1.0, SequenceProgression::Arithmetic, SubcolumnMode::Single, 1, 2);
        let mut sheet = DataSheet::new("xy-gen", "GenSeq", TableArchetype::Xy(xy)).unwrap();
        sheet.set_x_or_time_excluded(0, 0, true).unwrap();
        let spec = clip(Ifs::default(), Ips { mode: PlacementMode::AppendRows, ..Default::default() });
        csv(&mut sheet, &spec, "10\n20\n").unwrap();
        sheet.validate().unwrap();
        let xc = &th::as_xy(&sheet).x_cells;
        assert!(xc.len() == 4 && xc.iter().all(|row| row.len() == 1) && xc[0][0].excluded && !xc[3][0].excluded);
    }

    #[test]
    fn aud_04_17_insert_rows_shifts_linked_regions_and_saturates_exclusion_notes() {
        let mut target = col_dst("dst-ins", 2, 4);
        target.set_cell(Cc::new(2, 1, 0), Cell { value: CellValue::Number(77.0), excluded: true }).unwrap();
        target.exclusion_notes.extend([th::excl_note(2, 1, 0, "outlier", "Shifted"), th::excl_note(usize::MAX, 0, 0, "stale", "Saturates")]);
        target.linked_regions.push(LinkedDataRegion {
            id: String::from("link-1"),
            spec: clip(Ifs::default(), Ips { anchor: Cc::new(2, 1, 0), ..Default::default() }),
            target_range: rng((2, 1, 0), (3, 1, 0)),
            provenance_summary: String::from("existing"),
        });
        let spec = clip(Ifs::default(), Ips { mode: PlacementMode::InsertRows, anchor: Cc::new(0, 0, 0), ..Default::default() });
        csv(&mut target, &spec, "1\n2\n3\n").unwrap();
        assert_eq!((target.linked_regions[0].target_range, target.linked_regions[0].spec.placement.anchor.row), (rng((5, 1, 0), (6, 1, 0)), 5));
        assert_eq!(
            (target.exclusion_notes.len(), target.exclusion_notes[0].coord, target.exclusion_notes[0].reason_tag.as_str()),
            (1, Cc::new(5, 1, 0), "outlier")
        );
        assert!(target.cell(Cc::new(5, 1, 0)).unwrap().excluded);
    }

    #[test]
    fn aud_04_18_and_43_header_validation_propagation_decimal_sep_and_unique_link_ids() {
        let mut target = col_dst("dst-hdr", 1, 1);
        let long_csv = format!("{},42\n", "G".repeat(MAX_TITLE_BYTES + 10));
        let mut spec = clip(Ifs { unstack_group_column: Some(0), ..Default::default() }, Ips { propagate_title_header: true, ..Default::default() });
        assert!(matches!(csv(&mut target, &spec, &long_csv), Err(TransformError::Doc(DocError::TitleTooLong { .. }))));
        assert!(target.cell(Cc::new(0, 0, 0)).unwrap().is_empty());

        spec.placement.transpose = true;
        csv(&mut target, &spec, "G1,10\nG2,20\n").unwrap();
        assert_ne!(th::as_col(&target).data_sets[0].title, "G1");

        let mut plain = col_dst("dst-plain", 2, 1);
        let plain_spec = clip(Ifs::default(), Ips { propagate_title_header: true, ..Default::default() });
        assert_eq!(csv(&mut plain, &plain_spec, "Ctrl,Drug\n10,20\n30,40\n").unwrap().rows_placed, 2);
        let cp = th::as_col(&plain);
        assert_eq!((cp.data_sets[0].title.as_str(), cp.data_sets[1].title.as_str()), ("Ctrl", "Drug"));
        assert_eq!(cp.data_sets[0].cells[0][0], Cell { value: CellValue::Number(10.0), excluded: false });

        assert!(matches!(parse_delimited_text("105,2", ',', '0', false), Err(TransformError::InvalidSpec { .. })));

        let mut lnk = col_dst("dst-lnk", 1, 1);
        let file_spec = ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("a.csv"), delimiter: ',', decimal_separator: '.' },
            filter: Ifs::default(),
            placement: Ips::default(),
        };
        csv(&mut lnk, &file_spec, "1\n").unwrap();
        csv(&mut lnk, &file_spec, "2\n").unwrap();
        assert_eq!((lnk.linked_regions[0].id.as_str(), lnk.linked_regions[1].id.as_str()), ("link-1", "link-2"));
        lnk.linked_regions.remove(0);
        csv(&mut lnk, &file_spec, "3\n").unwrap();
        assert!(lnk.linked_regions.len() == 2 && lnk.linked_regions[0].id != lnk.linked_regions[1].id);
    }
}
