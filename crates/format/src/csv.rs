//! RFC-4180 CSV/TSV parser & writer, typed `DataSheet` import/export, and multi-stage
//! `ImportLinkSpec` pipeline (`filter -> rearrange/transpose -> baseline -> exclude/*`).

use std::collections::BTreeMap;

use lumencraft_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ContingencyTable, DataSetColumn, DataSheet, GroupedTable, ImportFilterSpec,
    ImportLinkSpec, ImportPlacementSpec, LinkedDataRegion, MAX_DATA_SETS, MAX_ROWS, MAX_SUBCOLUMNS, MultipleVariablesTable, NestedGroupColumn,
    NestedTable, PartsOfWholeTable, PlacementMode, SheetCellRange, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, VariableColumn,
    XyTable, default_column_label, transpose_cell_grid, validate_data_sets, validate_rows, validate_subcolumns, validate_title,
};

use crate::error::FormatError;

/// Configuration for RFC-4180 CSV/TSV parsing and formatting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CsvOptions {
    pub delimiter: char,
    pub quote_char: char,
    pub decimal_separator: char,
    pub thousands_separator: Option<char>,
    pub header_rows: usize,
    pub has_row_titles: bool,
    pub parse_asterisk_exclusions: bool,
    pub missing_sentinels: Vec<String>,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self::csv()
    }
}

impl CsvOptions {
    /// Standard comma-separated RFC-4180 options with `.` decimals.
    #[must_use]
    pub fn csv() -> Self {
        Self {
            delimiter: ',',
            quote_char: '"',
            decimal_separator: '.',
            thousands_separator: None,
            header_rows: 1,
            has_row_titles: false,
            parse_asterisk_exclusions: true,
            missing_sentinels: default_missing_sentinels(),
        }
    }

    /// Tab-separated options with `.` decimals.
    #[must_use]
    pub fn tsv() -> Self {
        Self { delimiter: '\t', ..Self::csv() }
    }

    /// Semicolon-delimited regional options with `,` decimal separator.
    #[must_use]
    pub fn european_csv() -> Self {
        Self { delimiter: ';', decimal_separator: ',', ..Self::csv() }
    }

    /// Auto-detect delimiter (`,`, `;`, `\t`) and decimal separator (`.` vs `,`) from sample text.
    #[must_use]
    pub fn detect(input: &str) -> Self {
        let sample = input.strip_prefix('\u{FEFF}').unwrap_or(input);
        let lines: Vec<&str> = sample.lines().filter(|l| !l.trim().is_empty()).take(10).collect();
        if lines.is_empty() {
            return Self::csv();
        }
        let candidates = ['\t', ';', ','];
        let mut best_delim = ',';
        let mut best_score = 0_usize;

        for delim in candidates {
            let counts: Vec<usize> = lines.iter().map(|l| count_unquoted_char(l, delim, '"')).collect();
            let first = counts[0];
            if first > 0 && counts.iter().all(|&c| c == first) {
                let score = first * lines.len() + 10;
                if score > best_score {
                    best_score = score;
                    best_delim = delim;
                }
            } else {
                let total: usize = counts.iter().sum();
                if total > best_score {
                    best_score = total;
                    best_delim = delim;
                }
            }
        }

        let mut opts = Self { delimiter: best_delim, ..Self::csv() };
        if best_delim == ';' && looks_like_comma_decimal(&lines, ';') {
            opts.decimal_separator = ',';
        }
        opts
    }
}

fn default_missing_sentinels() -> Vec<String> {
    vec![String::new(), String::from("NA"), String::from("NaN"), String::from("NULL"), String::from(".")]
}

fn count_unquoted_char(line: &str, target: char, quote: char) -> usize {
    let mut in_quotes = false;
    let mut count = 0;
    for ch in line.chars() {
        if ch == quote {
            in_quotes = !in_quotes;
        } else if !in_quotes && ch == target {
            count += 1;
        }
    }
    count
}

fn looks_like_comma_decimal(lines: &[&str], delim: char) -> bool {
    let mut comma_decimals = 0_usize;
    let mut dot_decimals = 0_usize;
    for line in lines.iter().skip(1) {
        for field in line.split(delim) {
            let trimmed = field.trim().trim_matches('"').trim_matches('*');
            if trimmed.contains(',') && !trimmed.contains('.') && trimmed.replace(',', ".").parse::<f64>().is_ok() {
                comma_decimals += 1;
            } else if trimmed.contains('.') && trimmed.parse::<f64>().is_ok() {
                dot_decimals += 1;
            }
        }
    }
    comma_decimals > dot_decimals
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CsvState {
    FieldStart,
    Unquoted,
    Quoted,
    AfterQuote,
}

/// Parse RFC-4180 delimited text into rectangular/jagged string records.
///
/// Strips optional UTF-8 BOM, handles `\r\n` / `\n` / `\r`, quoted delimiters/newlines, and
/// doubled quote escapes (`""`).
pub fn parse_delimited_records(input: &str, opts: &CsvOptions) -> Result<Vec<Vec<String>>, FormatError> {
    let text = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let max_cols = MAX_DATA_SETS.saturating_mul(MAX_SUBCOLUMNS).saturating_add(4);
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut state = CsvState::FieldStart;
    let mut line_no = 1_usize;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match state {
            CsvState::FieldStart => {
                if ch == opts.quote_char {
                    state = CsvState::Quoted;
                } else if ch == opts.delimiter {
                    push_field(&mut current_row, &mut field, max_cols, line_no)?;
                    state = CsvState::FieldStart;
                } else if ch == '\r' || ch == '\n' {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        let _ = chars.next();
                    }
                    push_field(&mut current_row, &mut field, max_cols, line_no)?;
                    push_record(&mut records, &mut current_row, line_no)?;
                    line_no += 1;
                    state = CsvState::FieldStart;
                } else {
                    field.push(ch);
                    state = CsvState::Unquoted;
                }
            }
            CsvState::Unquoted => {
                if ch == opts.delimiter {
                    push_field(&mut current_row, &mut field, max_cols, line_no)?;
                    state = CsvState::FieldStart;
                } else if ch == '\r' || ch == '\n' {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        let _ = chars.next();
                    }
                    push_field(&mut current_row, &mut field, max_cols, line_no)?;
                    push_record(&mut records, &mut current_row, line_no)?;
                    line_no += 1;
                    state = CsvState::FieldStart;
                } else if ch == opts.quote_char {
                    return Err(FormatError::CsvParse { line: line_no, reason: String::from("unexpected quote inside unquoted field") });
                } else {
                    field.push(ch);
                }
            }
            CsvState::Quoted => {
                if ch == opts.quote_char {
                    if chars.peek() == Some(&opts.quote_char) {
                        let _ = chars.next();
                        field.push(opts.quote_char);
                    } else {
                        state = CsvState::AfterQuote;
                    }
                } else {
                    if ch == '\n' || (ch == '\r' && chars.peek() != Some(&'\n')) {
                        line_no += 1;
                    }
                    field.push(ch);
                }
            }
            CsvState::AfterQuote => {
                if ch == opts.delimiter {
                    push_field(&mut current_row, &mut field, max_cols, line_no)?;
                    state = CsvState::FieldStart;
                } else if ch == '\r' || ch == '\n' {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        let _ = chars.next();
                    }
                    push_field(&mut current_row, &mut field, max_cols, line_no)?;
                    push_record(&mut records, &mut current_row, line_no)?;
                    line_no += 1;
                    state = CsvState::FieldStart;
                } else {
                    return Err(FormatError::CsvParse { line: line_no, reason: format!("unexpected character `{ch}` after closing quote") });
                }
            }
        }
    }

    match state {
        CsvState::Quoted => {
            return Err(FormatError::CsvParse { line: line_no, reason: String::from("unclosed quoted field at EOF") });
        }
        CsvState::FieldStart => {
            if !current_row.is_empty() || !field.is_empty() {
                push_field(&mut current_row, &mut field, max_cols, line_no)?;
                push_record(&mut records, &mut current_row, line_no)?;
            }
        }
        CsvState::Unquoted | CsvState::AfterQuote => {
            push_field(&mut current_row, &mut field, max_cols, line_no)?;
            push_record(&mut records, &mut current_row, line_no)?;
        }
    }

    Ok(records)
}

fn push_field(row: &mut Vec<String>, field: &mut String, max_cols: usize, line_no: usize) -> Result<(), FormatError> {
    if row.len() >= max_cols {
        return Err(FormatError::CsvParse { line: line_no, reason: format!("column count exceeds limit {max_cols}") });
    }
    row.push(std::mem::take(field));
    Ok(())
}

fn push_record(records: &mut Vec<Vec<String>>, row: &mut Vec<String>, line_no: usize) -> Result<(), FormatError> {
    if records.len() >= MAX_ROWS + 16 {
        return Err(FormatError::CsvParse { line: line_no, reason: format!("row count exceeds maximum {MAX_ROWS}") });
    }
    records.push(std::mem::take(row));
    Ok(())
}

/// Parse a raw cell token into a typed [`Cell`], honoring missing sentinels, regional decimal
/// separators, thousands separators, and inline `*` exclusions.
#[must_use]
pub fn parse_cell_token(raw: &str, opts: &CsvOptions) -> Cell {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Cell::empty();
    }
    if opts.parse_asterisk_exclusions && trimmed == "*" {
        return Cell { value: CellValue::Empty, excluded: true };
    }
    if is_missing_sentinel(trimmed, &opts.missing_sentinels) {
        return Cell::empty();
    }

    let (core, excluded) = if opts.parse_asterisk_exclusions {
        if let Some(rest) = trimmed.strip_suffix('*') {
            (rest.trim(), true)
        } else if let Some(rest) = trimmed.strip_prefix('*') {
            (rest.trim(), true)
        } else {
            (trimmed, false)
        }
    } else {
        (trimmed, false)
    };

    if core.is_empty() || is_missing_sentinel(core, &opts.missing_sentinels) {
        return Cell { value: CellValue::Empty, excluded };
    }

    let mut normalized = core.to_owned();
    if let Some(thousands) = opts.thousands_separator
        && thousands != opts.decimal_separator
    {
        normalized = normalized.replace(thousands, "");
    }
    if opts.decimal_separator == ',' {
        normalized = normalized.replace(',', ".");
    }

    if let Ok(num) = normalized.parse::<f64>()
        && num.is_finite()
    {
        return Cell { value: CellValue::Number(num), excluded };
    }

    Cell { value: CellValue::Text(core.to_owned()), excluded }
}

fn is_missing_sentinel(token: &str, sentinels: &[String]) -> bool {
    sentinels.iter().any(|s| {
        if s.is_empty() {
            token.is_empty()
        } else if s.len() <= 2 && !s.chars().all(|c| c.is_ascii_alphabetic()) {
            token == s
        } else {
            token.eq_ignore_ascii_case(s)
        }
    })
}

/// Format a [`Cell`] back to an unquoted delimited field string.
#[must_use]
pub fn format_cell_token(cell: &Cell, opts: &CsvOptions) -> String {
    let mut out = match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(v) if v.is_finite() => {
            let clean = if *v == 0.0 { 0.0 } else { *v };
            let mut s = format!("{clean}");
            if opts.decimal_separator == ',' {
                s = s.replace('.', ",");
            }
            s
        }
        CellValue::Number(_) => String::new(),
        CellValue::Text(s) => s.clone(),
    };
    if cell.excluded && opts.parse_asterisk_exclusions {
        out.push('*');
    }
    out
}

/// Escape and quote a single CSV/TSV field per RFC-4180.
#[must_use]
pub fn format_csv_field(field: &str, opts: &CsvOptions) -> String {
    let needs_quotes = field.contains(opts.delimiter) || field.contains(opts.quote_char) || field.contains('\n') || field.contains('\r');
    if !needs_quotes {
        return field.to_owned();
    }
    let mut out = String::with_capacity(field.len() + 2);
    out.push(opts.quote_char);
    for ch in field.chars() {
        if ch == opts.quote_char {
            out.push(opts.quote_char);
        }
        out.push(ch);
    }
    out.push(opts.quote_char);
    out
}

/// Serialize string rows into RFC-4180 delimited text (`\n` line endings).
#[must_use]
pub fn write_delimited_records(rows: &[Vec<String>], opts: &CsvOptions) -> String {
    let mut out = String::new();
    for (r_idx, row) in rows.iter().enumerate() {
        if r_idx > 0 {
            out.push('\n');
        }
        for (c_idx, field) in row.iter().enumerate() {
            if c_idx > 0 {
                out.push(opts.delimiter);
            }
            out.push_str(&format_csv_field(field, opts));
        }
    }
    out
}

/// Export a [`DataSheet`] into RFC-4180 CSV/TSV text, preserving headers, titles, and `*` exclusions.
pub fn export_datasheet_to_csv(sheet: &DataSheet, opts: &CsvOptions) -> Result<String, FormatError> {
    let rows = match &sheet.archetype {
        TableArchetype::Xy(t) => export_xy_rows(t, opts),
        TableArchetype::Column(t) => export_dataset_table_rows(
            "Row Title",
            t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty()),
            &t.row_titles,
            &t.data_sets,
            t.subcolumn_mode,
            t.row_count(),
            opts,
        ),
        TableArchetype::Grouped(t) => export_dataset_table_rows(
            &t.row_factor_title,
            t.row_titles.iter().any(|s| !s.is_empty()),
            &t.row_titles,
            &t.data_sets,
            t.subcolumn_mode,
            t.row_count(),
            opts,
        ),
        TableArchetype::Contingency(t) => export_dataset_table_rows(
            &t.row_factor_title,
            t.row_titles.iter().any(|s| !s.is_empty()),
            &t.row_titles,
            &t.data_sets,
            SubcolumnMode::Single,
            t.row_count(),
            opts,
        ),
        TableArchetype::Survival(t) => export_survival_rows(t, opts),
        TableArchetype::PartsOfWhole(t) => export_parts_of_whole_rows(t, opts),
        TableArchetype::MultipleVariables(t) => export_multivar_rows(t, opts),
        TableArchetype::Nested(t) => export_nested_rows(t, opts),
    };
    Ok(write_delimited_records(&rows, opts))
}

fn export_xy_rows(t: &XyTable, opts: &CsvOptions) -> Vec<Vec<String>> {
    let include_row_titles = t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty());
    let mut header = Vec::new();
    if include_row_titles {
        header.push(String::from("Row Title"));
    }
    let x_subs = t.x_mode.subcolumn_count();
    for s in 0..x_subs {
        header.push(if s == 0 { t.x_title.clone() } else { format!("{}:{}", t.x_title, s + 1) });
    }
    push_dataset_headers(&mut header, &t.data_sets, t.y_subcolumn_mode);
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(t.row_titles.get(r).cloned().unwrap_or_default());
        }
        for s in 0..x_subs {
            let token = t
                .x_cells
                .get(r)
                .and_then(|subs| subs.get(s))
                .map(|c| format_cell_token(c, opts))
                .unwrap_or_else(|| t.x_value_at(r).map_or_else(String::new, |v| format_cell_token(&Cell::number(v), opts)));
            row.push(token);
        }
        push_dataset_row(&mut row, &t.data_sets, r, opts);
        rows.push(row);
    }
    rows
}

fn export_dataset_table_rows(
    row_title_header: &str,
    include_row_titles: bool,
    row_titles: &[String],
    data_sets: &[DataSetColumn],
    mode: SubcolumnMode,
    row_count: usize,
    opts: &CsvOptions,
) -> Vec<Vec<String>> {
    let mut header = Vec::new();
    if include_row_titles {
        header.push(row_title_header.to_owned());
    }
    push_dataset_headers(&mut header, data_sets, mode);
    let mut rows = vec![header];
    for r in 0..row_count {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(row_titles.get(r).cloned().unwrap_or_default());
        }
        push_dataset_row(&mut row, data_sets, r, opts);
        rows.push(row);
    }
    rows
}

fn export_survival_rows(t: &SurvivalTable, opts: &CsvOptions) -> Vec<Vec<String>> {
    let include_row_titles = t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty());
    let mut header = Vec::new();
    if include_row_titles {
        header.push(String::from("Row Title"));
    }
    header.push(t.time_title.clone());
    push_dataset_headers(&mut header, &t.cohorts, SubcolumnMode::Single);
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(t.row_titles.get(r).cloned().unwrap_or_default());
        }
        let t_cell = t.times.get(r).cloned().unwrap_or_default();
        row.push(format_cell_token(&t_cell, opts));
        push_dataset_row(&mut row, &t.cohorts, r, opts);
        rows.push(row);
    }
    rows
}

fn export_parts_of_whole_rows(t: &PartsOfWholeTable, opts: &CsvOptions) -> Vec<Vec<String>> {
    let mut header = vec![String::from("Slice")];
    push_dataset_headers(&mut header, &t.data_sets, SubcolumnMode::Single);
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = vec![t.slice_labels.get(r).cloned().unwrap_or_default()];
        push_dataset_row(&mut row, &t.data_sets, r, opts);
        rows.push(row);
    }
    rows
}

fn export_multivar_rows(t: &MultipleVariablesTable, opts: &CsvOptions) -> Vec<Vec<String>> {
    let include_row_titles = t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty());
    let mut header = Vec::new();
    if include_row_titles {
        header.push(String::from("Row Title"));
    }
    for var in &t.variables {
        header.push(var.title.clone());
    }
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(t.row_titles.get(r).cloned().unwrap_or_default());
        }
        for var in &t.variables {
            let cell = var.cells.get(r).cloned().unwrap_or_default();
            row.push(format_cell_token(&cell, opts));
        }
        rows.push(row);
    }
    rows
}

fn export_nested_rows(t: &NestedTable, opts: &CsvOptions) -> Vec<Vec<String>> {
    let include_row_titles = t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty());
    let mut header = Vec::new();
    if include_row_titles {
        header.push(String::from("Row Title"));
    }
    for g in &t.groups {
        for sub_title in &g.subcolumn_titles {
            header.push(sub_title.clone());
        }
    }
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(t.row_titles.get(r).cloned().unwrap_or_default());
        }
        for g in &t.groups {
            for s in 0..g.biological_unit_count() {
                let cell = g.cells.get(r).and_then(|subs| subs.get(s)).cloned().unwrap_or_default();
                row.push(format_cell_token(&cell, opts));
            }
        }
        rows.push(row);
    }
    rows
}

fn push_dataset_headers(header: &mut Vec<String>, data_sets: &[DataSetColumn], mode: SubcolumnMode) {
    let sub_labels = mode.subcolumn_labels();
    let subs = mode.subcolumn_count();
    for ds in data_sets {
        if subs <= 1 {
            header.push(ds.title.clone());
        } else {
            for idx in 0..subs {
                let suffix = sub_labels.get(idx).map(String::as_str).unwrap_or("Y");
                header.push(format!("{}:{suffix}", ds.title));
            }
        }
    }
}

fn push_dataset_row(row: &mut Vec<String>, data_sets: &[DataSetColumn], r: usize, opts: &CsvOptions) {
    for ds in data_sets {
        let subs = ds.subcolumn_count();
        for s in 0..subs {
            let cell = ds.cell(r, s).cloned().unwrap_or_default();
            row.push(format_cell_token(&cell, opts));
        }
    }
}

/// Import delimited CSV/TSV text into a typed [`DataSheet`].
pub fn import_csv_to_datasheet(
    id: impl Into<SheetId>,
    title: impl Into<String>,
    input: &str,
    target_kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    opts: &CsvOptions,
) -> Result<DataSheet, FormatError> {
    let records = parse_delimited_records(input, opts)?;
    subcolumn_mode.validate()?;
    if !target_kind.allows_subcolumn_mode(subcolumn_mode) {
        return Err(FormatError::Doc(lumencraft_doc::DocError::IncompatibleSubcolumnMode { archetype: target_kind, mode: subcolumn_mode }));
    }

    let header_count = opts.header_rows.min(records.len());
    let headers = if header_count > 0 { records[header_count - 1].clone() } else { Vec::new() };
    let data_rows = &records[header_count..];
    let row_count = data_rows.len();
    validate_rows(row_count)?;

    let raw_col_count = headers.len().max(data_rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut col_offset = usize::from(opts.has_row_titles && raw_col_count > 0);
    let row_titles: Vec<String> = if opts.has_row_titles && raw_col_count > 0 {
        data_rows.iter().map(|r| r.first().cloned().unwrap_or_default()).collect()
    } else {
        vec![String::new(); row_count]
    };

    let arch = match target_kind {
        TableKind::Xy => {
            let x_title = headers.get(col_offset).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| String::from("X"));
            let x_col_idx = col_offset;
            if raw_col_count > col_offset {
                col_offset += 1;
            }
            let subs = subcolumn_mode.subcolumn_count();
            let data_cols = raw_col_count.saturating_sub(col_offset);
            let ds_count = data_cols.div_ceil(subs);
            validate_data_sets(ds_count)?;
            let mut xy = XyTable::new(subcolumn_mode, ds_count, row_count)?;
            xy.x_title = sanitize_title(&x_title)?;
            xy.row_titles = row_titles;
            xy.row_titles_expanded = opts.has_row_titles;
            for (r, rec) in data_rows.iter().enumerate() {
                if let Some(tok) = rec.get(x_col_idx) {
                    xy.set_x_cell(r, parse_cell_token(tok, opts))?;
                }
            }
            populate_datasets(&mut xy.data_sets, data_rows, &headers, col_offset, subs, opts)?;
            TableArchetype::Xy(xy)
        }
        TableKind::Column => {
            let subs = subcolumn_mode.subcolumn_count();
            let ds_count = raw_col_count.saturating_sub(col_offset).div_ceil(subs);
            validate_data_sets(ds_count)?;
            let hint = if subcolumn_mode.is_pre_summarized() { ColumnDesignHint::PrecomputedError } else { ColumnDesignHint::UnpairedReplicates };
            let mut col = ColumnTable::new_with_subcolumns(hint, subcolumn_mode, ds_count, row_count)?;
            col.row_titles = row_titles;
            col.row_titles_expanded = opts.has_row_titles;
            populate_datasets(&mut col.data_sets, data_rows, &headers, col_offset, subs, opts)?;
            TableArchetype::Column(col)
        }
        TableKind::Grouped => {
            let subs = subcolumn_mode.subcolumn_count();
            let ds_count = raw_col_count.saturating_sub(col_offset).div_ceil(subs);
            validate_data_sets(ds_count)?;
            let mut grp = GroupedTable::new(subcolumn_mode, ds_count, row_count)?;
            grp.row_titles = row_titles;
            populate_datasets(&mut grp.data_sets, data_rows, &headers, col_offset, subs, opts)?;
            TableArchetype::Grouped(grp)
        }
        TableKind::Contingency => {
            let ds_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(ds_count)?;
            let mut cont = ContingencyTable::new(ds_count, row_count)?;
            cont.row_titles = row_titles;
            populate_datasets(&mut cont.data_sets, data_rows, &headers, col_offset, 1, opts)?;
            TableArchetype::Contingency(cont)
        }
        TableKind::Survival => {
            let time_title = headers.get(col_offset).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| String::from("Elapsed Time"));
            let time_col = col_offset;
            if raw_col_count > col_offset {
                col_offset += 1;
            }
            let ds_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(ds_count)?;
            let mut surv = SurvivalTable::new(ds_count, row_count)?;
            surv.time_title = sanitize_title(&time_title)?;
            surv.row_titles = row_titles;
            surv.row_titles_expanded = opts.has_row_titles;
            for (r, rec) in data_rows.iter().enumerate() {
                if let Some(tok) = rec.get(time_col) {
                    surv.times[r] = parse_cell_token(tok, opts);
                }
            }
            populate_datasets(&mut surv.cohorts, data_rows, &headers, col_offset, 1, opts)?;
            TableArchetype::Survival(surv)
        }
        TableKind::PartsOfWhole => {
            let label_col = col_offset;
            if raw_col_count > col_offset && !opts.has_row_titles {
                col_offset += 1;
            }
            let ds_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(ds_count)?;
            let mut pow = PartsOfWholeTable::new(ds_count, row_count)?;
            pow.slice_labels =
                if opts.has_row_titles { row_titles } else { data_rows.iter().map(|r| r.get(label_col).cloned().unwrap_or_default()).collect() };
            populate_datasets(&mut pow.data_sets, data_rows, &headers, col_offset, 1, opts)?;
            TableArchetype::PartsOfWhole(pow)
        }
        TableKind::MultipleVariables => {
            let var_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(var_count)?;
            let mut mv = MultipleVariablesTable::new(var_count, row_count)?;
            mv.row_titles = row_titles;
            mv.row_titles_expanded = opts.has_row_titles;
            for (idx, var) in mv.variables.iter_mut().enumerate() {
                let c = col_offset + idx;
                if let Some(h) = headers.get(c).filter(|s| !s.trim().is_empty()) {
                    var.title = sanitize_title(h)?;
                }
                for (r, rec) in data_rows.iter().enumerate() {
                    if let Some(tok) = rec.get(c) {
                        var.cells[r] = parse_cell_token(tok, opts);
                    }
                }
                var.refresh_inferred_role();
            }
            TableArchetype::MultipleVariables(mv)
        }
        TableKind::Nested => {
            let bio_units = 1;
            let group_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(group_count)?;
            let mut nested = NestedTable::new(group_count, bio_units, row_count)?;
            nested.row_titles = row_titles;
            nested.row_titles_expanded = opts.has_row_titles;
            for (idx, grp) in nested.groups.iter_mut().enumerate() {
                let c = col_offset + idx;
                if let Some(h) = headers.get(c).filter(|s| !s.trim().is_empty()) {
                    grp.title = sanitize_title(h)?;
                    grp.subcolumn_titles = vec![format!("{}:1", grp.title)];
                }
                for (r, rec) in data_rows.iter().enumerate() {
                    if let Some(tok) = rec.get(c) {
                        grp.cells[r][0] = parse_cell_token(tok, opts);
                    }
                }
            }
            TableArchetype::Nested(nested)
        }
    };

    Ok(DataSheet::new(id.into(), title, arch)?)
}

fn sanitize_title(raw: &str) -> Result<String, FormatError> {
    let trimmed = raw.trim().to_owned();
    validate_title(&trimmed)?;
    Ok(trimmed)
}

fn populate_datasets(
    data_sets: &mut [DataSetColumn],
    data_rows: &[Vec<String>],
    headers: &[String],
    col_offset: usize,
    subs: usize,
    opts: &CsvOptions,
) -> Result<(), FormatError> {
    for (ds_idx, ds) in data_sets.iter_mut().enumerate() {
        let base_col = col_offset + ds_idx * subs;
        if let Some(raw_title) = headers.get(base_col).filter(|s| !s.trim().is_empty()) {
            let base = if subs > 1 { raw_title.split(':').next().unwrap_or(raw_title.as_str()) } else { raw_title.as_str() };
            ds.title = sanitize_title(base)?;
        }
        for (r, rec) in data_rows.iter().enumerate() {
            for s in 0..subs {
                if let Some(tok) = rec.get(base_col + s)
                    && let Some(slot) = ds.cell_mut(r, s)
                {
                    *slot = parse_cell_token(tok, opts);
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Multi-Stage ImportLinkSpec Pipeline
// ============================================================================

/// Optional baseline adjustment applied during multi-stage delimited import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ImportBaselineAdjustment {
    #[default]
    None,
    /// Subtract row `0` values from every subsequent row (per column).
    SubtractFirstRow,
    /// Subtract column `col` value from every other column (per row).
    SubtractColumn(usize),
    /// Divide each row value by row `0` value and multiply by 100.
    PercentOfFirstRow,
}

/// Output of parsing and filtering a delimited grid prior to placement.
#[derive(Debug, Clone, PartialEq)]
pub struct FilteredDelimitedGrid {
    pub headers: Vec<String>,
    pub cells: Vec<Vec<Cell>>,
}

/// Stage 1 & 2: Parse delimited text and apply [`ImportFilterSpec`] (`row_window`, `col_window`,
/// `decimate_every_k`, `unstack_group_column`, and `parse_asterisk_exclusions`).
pub fn filter_delimited_text(
    input: &str,
    delimiter: char,
    decimal_separator: char,
    has_header: bool,
    filter: &ImportFilterSpec,
) -> Result<FilteredDelimitedGrid, FormatError> {
    let opts = CsvOptions {
        delimiter,
        decimal_separator,
        header_rows: usize::from(has_header),
        parse_asterisk_exclusions: filter.parse_asterisk_exclusions,
        ..CsvOptions::csv()
    };
    let records = parse_delimited_records(input, &opts)?;
    if records.is_empty() {
        return Ok(FilteredDelimitedGrid { headers: Vec::new(), cells: Vec::new() });
    }
    let (raw_headers, data_rows) = if has_header { (records[0].clone(), &records[1..]) } else { (Vec::new(), records.as_slice()) };

    let (r_start, r_end) = match filter.row_window {
        Some((s, e)) if e < s => return Err(FormatError::InvalidPipeline { reason: String::from("row_window end < start") }),
        Some((s, e)) => (s.min(data_rows.len()), e.min(data_rows.len().saturating_sub(1))),
        None => (0, data_rows.len().saturating_sub(1)),
    };
    let sliced_rows = if data_rows.is_empty() || r_start >= data_rows.len() { &[][..] } else { &data_rows[r_start..=r_end] };

    let step = match filter.decimate_every_k {
        Some(0) => return Err(FormatError::InvalidPipeline { reason: String::from("decimate_every_k must be >= 1") }),
        Some(k) => k,
        None => 1,
    };

    let max_cols = raw_headers.len().max(sliced_rows.iter().map(Vec::len).max().unwrap_or(0));
    let (c_start, c_end) = match filter.col_window {
        Some((s, e)) if e < s => return Err(FormatError::InvalidPipeline { reason: String::from("col_window end < start") }),
        Some((s, e)) => (s, e.min(max_cols.saturating_sub(1))),
        None => (0, max_cols.saturating_sub(1)),
    };

    let mut headers = Vec::new();
    let mut cells = Vec::new();
    if max_cols > 0 && c_start < max_cols {
        for c in c_start..=c_end {
            headers.push(raw_headers.get(c).cloned().unwrap_or_else(|| default_column_label(c)));
        }
        for (idx, row) in sliced_rows.iter().enumerate() {
            if idx % step != 0 {
                continue;
            }
            let mut parsed_row = Vec::with_capacity(c_end - c_start + 1);
            for c in c_start..=c_end {
                let tok = row.get(c).map(String::as_str).unwrap_or("");
                parsed_row.push(parse_cell_token(tok, &opts));
            }
            cells.push(parsed_row);
        }
    }

    if let Some(group_col_raw) = filter.unstack_group_column {
        let rel_col = group_col_raw
            .checked_sub(c_start)
            .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("unstack_group_column outside col_window") })?;
        return unstack_grid(&headers, &cells, rel_col);
    }

    Ok(FilteredDelimitedGrid { headers, cells })
}

fn unstack_grid(headers: &[String], cells: &[Vec<Cell>], group_col: usize) -> Result<FilteredDelimitedGrid, FormatError> {
    if cells.is_empty() || group_col >= headers.len() {
        return Err(FormatError::InvalidPipeline { reason: String::from("unstack_group_column out of range") });
    }
    let val_col = (0..headers.len())
        .find(|&c| c != group_col)
        .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("unstack requires at least one value column alongside group column") })?;
    let mut groups: BTreeMap<String, Vec<Cell>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for row in cells {
        let Some(g_cell) = row.get(group_col) else { continue };
        let key = match &g_cell.value {
            CellValue::Empty => continue,
            CellValue::Text(s) => s.trim().to_owned(),
            CellValue::Number(v) => format!("{v}"),
        };
        if key.is_empty() {
            continue;
        }
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        let val = row.get(val_col).cloned().unwrap_or_default();
        groups.entry(key).or_default().push(val);
    }

    let max_rows = groups.values().map(Vec::len).max().unwrap_or(0);
    validate_data_sets(order.len())?;
    validate_rows(max_rows)?;
    let mut out_cells = vec![vec![Cell::empty(); order.len()]; max_rows];
    for (c_idx, key) in order.iter().enumerate() {
        if let Some(vals) = groups.get(key) {
            for (r_idx, val) in vals.iter().enumerate() {
                out_cells[r_idx][c_idx] = val.clone();
            }
        }
    }
    Ok(FilteredDelimitedGrid { headers: order, cells: out_cells })
}

/// Stage 3: Rearrange (`skip_contiguous_blanks`, `transpose`) and apply baseline normalization on
/// a filtered cell grid while preserving `excluded` flags.
#[must_use]
pub fn rearrange_and_baseline_grid(grid: &[Vec<Cell>], placement: &ImportPlacementSpec, baseline: ImportBaselineAdjustment) -> Vec<Vec<Cell>> {
    let mut working: Vec<Vec<Cell>> = if placement.skip_contiguous_blanks {
        let mut out = Vec::with_capacity(grid.len());
        let mut prev_blank = false;
        for row in grid {
            let is_blank = row.iter().all(Cell::is_empty);
            if is_blank && (out.is_empty() || prev_blank) {
                continue;
            }
            prev_blank = is_blank;
            out.push(row.clone());
        }
        out
    } else {
        grid.to_vec()
    };

    if placement.transpose {
        working = transpose_cell_grid(&working);
    }

    apply_baseline_to_grid(&mut working, baseline);
    working
}

fn apply_baseline_to_grid(grid: &mut [Vec<Cell>], baseline: ImportBaselineAdjustment) {
    match baseline {
        ImportBaselineAdjustment::None => {}
        ImportBaselineAdjustment::SubtractFirstRow => {
            let Some(base_row) = grid.first().cloned() else {
                return;
            };
            for row in grid {
                for (c, cell) in row.iter_mut().enumerate() {
                    if let (CellValue::Number(v), Some(b)) = (&cell.value, base_row.get(c).and_then(Cell::active_number)) {
                        let diff = *v - b;
                        if diff.is_finite() {
                            cell.value = CellValue::Number(diff);
                        }
                    }
                }
            }
        }
        ImportBaselineAdjustment::SubtractColumn(base_col) => {
            for row in grid {
                let Some(b) = row.get(base_col).and_then(Cell::active_number) else {
                    continue;
                };
                for cell in row {
                    if let CellValue::Number(v) = cell.value {
                        let diff = v - b;
                        if diff.is_finite() {
                            cell.value = CellValue::Number(diff);
                        }
                    }
                }
            }
        }
        ImportBaselineAdjustment::PercentOfFirstRow => {
            let Some(base_row) = grid.first().cloned() else {
                return;
            };
            for row in grid {
                for (c, cell) in row.iter_mut().enumerate() {
                    if let (CellValue::Number(v), Some(b)) = (&cell.value, base_row.get(c).and_then(Cell::active_number))
                        && b != 0.0
                    {
                        let pct = (*v / b) * 100.0;
                        if pct.is_finite() {
                            cell.value = CellValue::Number(pct);
                        }
                    }
                }
            }
        }
    }
}

/// Execute the full 4-stage CSV/TSV [`ImportLinkSpec`] pipeline into `target`, writing cells,
/// optionally promoting column titles, recording a [`LinkedDataRegion`], and returning the
/// populated [`SheetCellRange`].
pub fn apply_csv_import_link(
    target: &mut DataSheet,
    csv_text: &str,
    spec: &ImportLinkSpec,
    has_header: bool,
    baseline: ImportBaselineAdjustment,
    link_id: &str,
    provenance_summary: &str,
) -> Result<SheetCellRange, FormatError> {
    target.ensure_mutable()?;
    let (delimiter, decimal_separator) = match &spec.source {
        lumencraft_doc::LinkSource::ExternalDelimitedFile { delimiter, decimal_separator, .. } => (*delimiter, *decimal_separator),
        _ => (',', '.'),
    };
    let filtered = filter_delimited_text(csv_text, delimiter, decimal_separator, has_header, &spec.filter)?;
    let transformed = rearrange_and_baseline_grid(&filtered.cells, &spec.placement, baseline);
    let headers = if spec.placement.propagate_title_header && !spec.placement.transpose { Some(filtered.headers.as_slice()) } else { None };

    let range = place_grid_in_datasheet(target, &transformed, headers, &spec.placement)?;
    target.add_linked_region(LinkedDataRegion {
        id: link_id.to_owned(),
        spec: spec.clone(),
        target_range: range,
        provenance_summary: provenance_summary.to_owned(),
    })?;
    Ok(range)
}

fn place_grid_in_datasheet(
    target: &mut DataSheet,
    grid: &[Vec<Cell>],
    headers: Option<&[String]>,
    placement: &ImportPlacementSpec,
) -> Result<SheetCellRange, FormatError> {
    if grid.is_empty() {
        return Ok(SheetCellRange::new(placement.anchor, placement.anchor));
    }
    let grid_rows = grid.len();
    let grid_cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    if grid_cols == 0 {
        return Ok(SheetCellRange::new(placement.anchor, placement.anchor));
    }

    let start_row = match placement.mode {
        PlacementMode::Overwrite | PlacementMode::InsertRows | PlacementMode::AppendColumns => placement.anchor.row,
        PlacementMode::AppendRows => target.archetype.row_count(),
    };
    let start_col = match placement.mode {
        PlacementMode::Overwrite | PlacementMode::InsertRows | PlacementMode::AppendRows => placement.anchor.col,
        PlacementMode::AppendColumns => target.archetype.column_count(),
    };

    let needed_rows = start_row.checked_add(grid_rows).ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("row offset overflow") })?;
    validate_rows(needed_rows)?;

    ensure_sheet_capacity(target, needed_rows, start_col, placement.anchor.subcol, grid_cols)?;
    if let Some(hdrs) = headers {
        apply_headers_to_sheet(target, start_col, hdrs)?;
    }

    let mut end_coord = CellCoord::new(start_row, start_col, placement.anchor.subcol);
    for (r_off, row) in grid.iter().enumerate() {
        let r = start_row + r_off;
        for (c_off, cell) in row.iter().enumerate() {
            let (col, subcol) = resolve_target_coord(&target.archetype, start_col, placement.anchor.subcol, c_off)?;
            let coord = CellCoord::new(r, col, subcol);
            target.set_cell(coord, cell.clone())?;
            end_coord = coord;
        }
    }
    Ok(SheetCellRange::new(CellCoord::new(start_row, start_col, placement.anchor.subcol), end_coord))
}

fn ensure_sheet_capacity(
    target: &mut DataSheet,
    needed_rows: usize,
    start_col: usize,
    start_subcol: usize,
    grid_cols: usize,
) -> Result<(), FormatError> {
    match &mut target.archetype {
        TableArchetype::Xy(t) => {
            let subs = t.y_subcolumn_mode.subcolumn_count();
            let total_flat = start_col * subs + start_subcol + grid_cols;
            let needed_cols = total_flat.div_ceil(subs);
            validate_data_sets(needed_cols)?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, subs)?;
            if t.x_cells.len() < needed_rows {
                t.x_cells.resize_with(needed_rows, || vec![Cell::empty(); t.x_mode.subcolumn_count()]);
            }
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::Column(t) => {
            let subs = t.subcolumn_mode.subcolumn_count();
            let needed_cols = (start_col * subs + start_subcol + grid_cols).div_ceil(subs);
            validate_data_sets(needed_cols)?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, subs)?;
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::Grouped(t) => {
            let subs = t.subcolumn_mode.subcolumn_count();
            let needed_cols = (start_col * subs + start_subcol + grid_cols).div_ceil(subs);
            validate_data_sets(needed_cols)?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, subs)?;
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::Contingency(t) => {
            let needed_cols = start_col + grid_cols;
            validate_data_sets(needed_cols)?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, 1)?;
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::Survival(t) => {
            let needed_cols = start_col + grid_cols;
            validate_data_sets(needed_cols)?;
            grow_datasets(&mut t.cohorts, needed_cols, needed_rows, 1)?;
            if t.times.len() < needed_rows {
                t.times.resize(needed_rows, Cell::empty());
            }
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::PartsOfWhole(t) => {
            let needed_cols = start_col + grid_cols;
            validate_data_sets(needed_cols)?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, 1)?;
            if t.slice_labels.len() < needed_rows {
                t.slice_labels.resize(needed_rows, String::new());
            }
        }
        TableArchetype::MultipleVariables(t) => {
            let needed_cols = start_col + grid_cols;
            validate_data_sets(needed_cols)?;
            while t.variables.len() < needed_cols {
                let idx = t.variables.len();
                t.variables.push(VariableColumn::new(format!("Variable {}", default_column_label(idx)), needed_rows)?);
            }
            for var in &mut t.variables {
                if var.cells.len() < needed_rows {
                    var.cells.resize(needed_rows, Cell::empty());
                }
            }
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::Nested(t) => {
            let needed_cols = start_col + grid_cols;
            validate_data_sets(needed_cols)?;
            while t.groups.len() < needed_cols {
                let idx = t.groups.len();
                t.groups.push(NestedGroupColumn::new(default_column_label(idx), 1, needed_rows)?);
            }
            for g in &mut t.groups {
                let units = g.biological_unit_count();
                if g.cells.len() < needed_rows {
                    g.cells.resize_with(needed_rows, || vec![Cell::empty(); units]);
                }
            }
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
    }
    Ok(())
}

fn grow_datasets(data_sets: &mut Vec<DataSetColumn>, needed_cols: usize, needed_rows: usize, subs: usize) -> Result<(), FormatError> {
    validate_subcolumns(subs)?;
    while data_sets.len() < needed_cols {
        let idx = data_sets.len();
        data_sets.push(DataSetColumn::new(default_column_label(idx), needed_rows, subs)?);
    }
    for ds in data_sets.iter_mut() {
        ds.ensure_rows(needed_rows, ds.subcolumn_count().max(subs))?;
    }
    Ok(())
}

fn resolve_target_coord(arch: &TableArchetype, start_col: usize, start_subcol: usize, flat_offset: usize) -> Result<(usize, usize), FormatError> {
    let subs = match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
        _ => 1,
    };
    let flat = start_col * subs + start_subcol + flat_offset;
    Ok((flat / subs, flat % subs))
}

fn apply_headers_to_sheet(target: &mut DataSheet, start_col: usize, headers: &[String]) -> Result<(), FormatError> {
    for (idx, title) in headers.iter().enumerate() {
        if title.trim().is_empty() {
            continue;
        }
        validate_title(title)?;
        let col = start_col + idx;
        match &mut target.archetype {
            TableArchetype::Xy(t) => {
                if let Some(ds) = t.data_sets.get_mut(col) {
                    ds.title = title.trim().to_owned();
                }
            }
            TableArchetype::Column(t) => {
                if let Some(ds) = t.data_sets.get_mut(col) {
                    ds.title = title.trim().to_owned();
                }
            }
            TableArchetype::Grouped(t) => {
                if let Some(ds) = t.data_sets.get_mut(col) {
                    ds.title = title.trim().to_owned();
                }
            }
            TableArchetype::Contingency(t) => {
                if let Some(ds) = t.data_sets.get_mut(col) {
                    ds.title = title.trim().to_owned();
                }
            }
            TableArchetype::Survival(t) => {
                if let Some(ds) = t.cohorts.get_mut(col) {
                    ds.title = title.trim().to_owned();
                }
            }
            TableArchetype::PartsOfWhole(t) => {
                if let Some(ds) = t.data_sets.get_mut(col) {
                    ds.title = title.trim().to_owned();
                }
            }
            TableArchetype::MultipleVariables(t) => {
                if let Some(var) = t.variables.get_mut(col) {
                    var.title = title.trim().to_owned();
                }
            }
            TableArchetype::Nested(t) => {
                if let Some(g) = t.groups.get_mut(col) {
                    g.title = title.trim().to_owned();
                }
            }
        }
    }
    Ok(())
}
