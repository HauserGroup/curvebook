//! Four-stage [`ImportLinkSpec`] data import, filtering, reshaping, placement, and
//! circular-link detection pipeline (`docs/data-tables-and-transforms.md` §6).

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{
    BaselineOperation, Cell, CellCoord, CellValue, DataSetColumn, DataSheet, ImportFilterSpec, ImportLinkSpec, InfoSheet, LinkSource,
    LinkedDataRegion, MAX_DATA_SETS, MAX_ROWS, MAX_SUBCOLUMNS, NestedGroupColumn, PlacementMode, ResultCell, ResultSheet, SheetCellRange, SheetId,
    TableArchetype, VariableColumn, default_column_label, transpose_cell_grid, validate_data_sets, validate_rows, validate_sheet_cells,
    validate_subcolumns,
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
    InternalData(&'a DataSheet),
    InternalResult(&'a ResultSheet),
    DelimitedText { text: &'a str, delimiter: char, decimal_separator: char },
}

/// Summary of a completed 4-stage import pipeline run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPipelineReport {
    pub rows_placed: usize,
    pub cols_placed: usize,
    pub target_range: SheetCellRange,
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
    let mut shaped = apply_stage2_filter(raw_grid, &spec.filter, opts, &mut header_row)?;

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
        target.add_linked_region(LinkedDataRegion {
            id: link_id,
            spec: spec.clone(),
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

fn extract_stage1(
    source: &LinkSource,
    filter: &ImportFilterSpec,
    payload: ImportSourcePayload<'_>,
) -> Result<(Vec<Vec<Cell>>, Vec<Cell>), TransformError> {
    match (source, payload) {
        (LinkSource::InternalSheet { range, .. }, ImportSourcePayload::InternalData(sheet)) => Ok((sheet.extract_cell_grid(*range), Vec::new())),
        (LinkSource::InternalSheet { tab_id, range, .. }, ImportSourcePayload::InternalResult(res)) => {
            let tab = if let Some(tid) = tab_id { res.tabs.iter().find(|t| &t.id == tid) } else { res.tabs.first() };
            let Some(tab) = tab else {
                return Err(TransformError::InvalidInput { message: String::from("referenced ResultTab not found") });
            };
            let mut grid = Vec::new();
            let end_r = range.end.row.min(tab.rows.len().saturating_sub(1));
            if range.start.row <= end_r && !tab.rows.is_empty() {
                for r in range.start.row..=end_r {
                    let row = &tab.rows[r];
                    let end_c = range.end.col.min(row.len().saturating_sub(1));
                    if range.start.col <= end_c && !row.is_empty() {
                        grid.push(row[range.start.col..=end_c].iter().map(result_cell_to_cell).collect());
                    }
                }
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
            Ok((sheet.extract_cell_grid(full), Vec::new()))
        }
        _ => Err(TransformError::InvalidInput { message: String::from("mismatched LinkSource and ImportSourcePayload") }),
    }
}

fn result_cell_to_cell(rc: &ResultCell) -> Cell {
    match rc {
        ResultCell::Empty => Cell::empty(),
        ResultCell::Number { value, .. } => Cell::number(*value),
        ResultCell::PValue { raw, .. } => Cell::number(*raw),
        ResultCell::Text(s) => Cell::text(s.clone()),
        ResultCell::Boolean(b) => Cell::number(if *b { 1.0 } else { 0.0 }),
        ResultCell::ConfidenceInterval { .. } => Cell::empty(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DelimState {
    FieldStart,
    Unquoted,
    Quoted,
    AfterQuote,
}

fn push_parsed_cell(
    row: &mut Vec<Cell>,
    field: &mut String,
    decimal_sep: char,
    parse_asterisk: bool,
    total_cells: &mut usize,
    max_cols: usize,
    max_cells: usize,
) -> Result<(), TransformError> {
    if row.len() >= max_cols {
        return Err(TransformError::InvalidSpec { message: format!("delimited column count exceeds limit {max_cols}") });
    }
    let next_total = total_cells.saturating_add(1);
    if next_total > max_cells {
        return Err(TransformError::InvalidSpec { message: format!("delimited cell count exceeds limit {max_cells}") });
    }
    *total_cells = next_total;
    let tok = std::mem::take(field);
    row.push(Cell::parse_import_token(&tok, decimal_sep, parse_asterisk));
    Ok(())
}

fn push_parsed_row(rows: &mut Vec<Vec<Cell>>, row: &mut Vec<Cell>) -> Result<(), TransformError> {
    if rows.len() >= MAX_ROWS + 16 {
        return Err(TransformError::InvalidSpec { message: format!("delimited row count exceeds limit {MAX_ROWS}") });
    }
    rows.push(std::mem::take(row));
    Ok(())
}

fn parse_delimited_text(text: &str, delimiter: char, decimal_sep: char, parse_asterisk: bool) -> Result<Vec<Vec<Cell>>, TransformError> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let quote_char = '"';
    let max_cols = MAX_DATA_SETS.saturating_mul(MAX_SUBCOLUMNS).saturating_add(4);
    let max_cells = MAX_IMPORT_CELLS.saturating_add(65_536);

    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current_row: Vec<Cell> = Vec::new();
    let mut field = String::new();
    let mut state = DelimState::FieldStart;
    let mut total_cells = 0_usize;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match state {
            DelimState::FieldStart => {
                if ch == quote_char && delimiter != quote_char {
                    state = DelimState::Quoted;
                } else if ch == delimiter {
                    push_parsed_cell(&mut current_row, &mut field, decimal_sep, parse_asterisk, &mut total_cells, max_cols, max_cells)?;
                    state = DelimState::FieldStart;
                } else if ch == '\r' || ch == '\n' {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        let _ = chars.next();
                    }
                    if !current_row.is_empty() || !field.is_empty() {
                        push_parsed_cell(&mut current_row, &mut field, decimal_sep, parse_asterisk, &mut total_cells, max_cols, max_cells)?;
                        push_parsed_row(&mut rows, &mut current_row)?;
                    }
                    state = DelimState::FieldStart;
                } else {
                    field.push(ch);
                    state = DelimState::Unquoted;
                }
            }
            DelimState::Unquoted => {
                if ch == delimiter {
                    push_parsed_cell(&mut current_row, &mut field, decimal_sep, parse_asterisk, &mut total_cells, max_cols, max_cells)?;
                    state = DelimState::FieldStart;
                } else if ch == '\r' || ch == '\n' {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        let _ = chars.next();
                    }
                    push_parsed_cell(&mut current_row, &mut field, decimal_sep, parse_asterisk, &mut total_cells, max_cols, max_cells)?;
                    push_parsed_row(&mut rows, &mut current_row)?;
                    state = DelimState::FieldStart;
                } else if ch == quote_char && delimiter != quote_char {
                    return Err(TransformError::InvalidInput { message: String::from("unexpected quote inside unquoted delimited field") });
                } else {
                    field.push(ch);
                }
            }
            DelimState::Quoted => {
                if ch == quote_char {
                    if chars.peek() == Some(&quote_char) {
                        let _ = chars.next();
                        field.push(quote_char);
                    } else {
                        state = DelimState::AfterQuote;
                    }
                } else {
                    field.push(ch);
                }
            }
            DelimState::AfterQuote => {
                if ch == delimiter {
                    push_parsed_cell(&mut current_row, &mut field, decimal_sep, parse_asterisk, &mut total_cells, max_cols, max_cells)?;
                    state = DelimState::FieldStart;
                } else if ch == '\r' || ch == '\n' {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        let _ = chars.next();
                    }
                    push_parsed_cell(&mut current_row, &mut field, decimal_sep, parse_asterisk, &mut total_cells, max_cols, max_cells)?;
                    push_parsed_row(&mut rows, &mut current_row)?;
                    state = DelimState::FieldStart;
                } else {
                    return Err(TransformError::InvalidInput {
                        message: format!("unexpected character `{ch}` after closing quote in delimited text"),
                    });
                }
            }
        }
    }

    match state {
        DelimState::Quoted => {
            return Err(TransformError::InvalidInput { message: String::from("unclosed quoted field at end of delimited text") });
        }
        DelimState::FieldStart => {
            if !current_row.is_empty() || !field.is_empty() {
                push_parsed_cell(&mut current_row, &mut field, decimal_sep, parse_asterisk, &mut total_cells, max_cols, max_cells)?;
                push_parsed_row(&mut rows, &mut current_row)?;
            }
        }
        DelimState::Unquoted | DelimState::AfterQuote => {
            push_parsed_cell(&mut current_row, &mut field, decimal_sep, parse_asterisk, &mut total_cells, max_cols, max_cells)?;
            push_parsed_row(&mut rows, &mut current_row)?;
        }
    }

    Ok(rows)
}

fn apply_stage2_filter(
    mut grid: Vec<Vec<Cell>>,
    filter: &ImportFilterSpec,
    opts: &ImportPipelineOptions,
    header_row: &mut Vec<Cell>,
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

    if let Some((base_col, op)) = opts.baseline {
        apply_grid_baseline(&mut grid, base_col, op)?;
    }

    if let Some(ref pred) = opts.exclude_row_predicate {
        apply_row_exclusion_predicate(&mut grid, pred)?;
    }

    for &(r, c) in &opts.exclude_coords {
        if let Some(cell) = grid.get_mut(r).and_then(|row| row.get_mut(c)) {
            cell.excluded = true;
        }
    }

    Ok(grid)
}

fn unstack_grid_by_group(grid: &[Vec<Cell>], group_col: usize) -> Result<(Vec<Vec<Cell>>, Vec<Cell>), TransformError> {
    let mut group_order: Vec<String> = Vec::new();
    let mut buckets: BTreeMap<String, Vec<Cell>> = BTreeMap::new();

    for row in grid {
        let Some(g_cell) = row.get(group_col) else {
            return Err(TransformError::InvalidSpec { message: format!("unstack_group_column {group_col} out of bounds") });
        };
        let key = match &g_cell.value {
            CellValue::Text(s) => s.trim().to_owned(),
            CellValue::Number(v) if v.is_finite() => format!("{v}"),
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

fn apply_row_exclusion_predicate(grid: &mut [Vec<Cell>], pred: &str) -> Result<(), TransformError> {
    let expr = parse_formula(pred)?;
    for (r, row) in grid.iter_mut().enumerate() {
        let mut bindings = Vec::with_capacity(row.len() + 2);
        #[allow(clippy::cast_precision_loss)]
        {
            bindings.push(("$row", (r + 1) as f64));
            bindings.push(("$index", r as f64));
        }
        let col_names: Vec<String> = (0..row.len()).map(curvebook_doc::default_column_label).collect();
        for (c, cell) in row.iter().enumerate() {
            if let Some(v) = cell.active_number() {
                bindings.push((col_names[c].as_str(), v));
            }
        }
        let env = ScalarEnv::new(bindings, None);
        if eval_scalar_expr(&expr, &env)?.as_bool() == Some(true) {
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
        shift_archetype_rows_down(&mut target.archetype, start_row, grid.len(), existing_rows);
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

fn rotate_rows_down<T>(rows: &mut [T], start_row: usize, shift: usize, end_row: usize) {
    if let Some(slice) = rows.get_mut(start_row..end_row)
        && shift < slice.len()
    {
        slice.rotate_right(shift);
    }
}

fn shift_archetype_rows_down(arch: &mut TableArchetype, start_row: usize, insert_rows: usize, existing_rows: usize) {
    if insert_rows == 0 || start_row >= existing_rows {
        return;
    }
    let end = existing_rows + insert_rows;
    match arch {
        TableArchetype::Xy(t) => {
            rotate_rows_down(&mut t.x_cells, start_row, insert_rows, end);
            rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
            for ds in &mut t.data_sets {
                rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
            }
        }
        TableArchetype::Column(t) => {
            rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
            for ds in &mut t.data_sets {
                rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
            }
        }
        TableArchetype::Grouped(t) => {
            rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
            if let Some(spacing) = &mut t.row_numeric_spacing {
                rotate_rows_down(spacing, start_row, insert_rows, end);
            }
            for ds in &mut t.data_sets {
                rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
            }
        }
        TableArchetype::Contingency(t) => {
            rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
            for ds in &mut t.data_sets {
                rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
            }
        }
        TableArchetype::Survival(t) => {
            rotate_rows_down(&mut t.times, start_row, insert_rows, end);
            rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
            for ds in &mut t.cohorts {
                rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
            }
        }
        TableArchetype::PartsOfWhole(t) => {
            rotate_rows_down(&mut t.slice_labels, start_row, insert_rows, end);
            for ds in &mut t.data_sets {
                rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            rotate_rows_down(&mut mv.row_titles, start_row, insert_rows, end);
            for var in &mut mv.variables {
                rotate_rows_down(&mut var.cells, start_row, insert_rows, end);
            }
        }
        TableArchetype::Nested(t) => {
            rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
            for grp in &mut t.groups {
                rotate_rows_down(&mut grp.cells, start_row, insert_rows, end);
            }
        }
    }
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
