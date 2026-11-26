//! RFC-4180 CSV/TSV parser & writer, typed `DataSheet` import/export, and multi-stage
//! `ImportLinkSpec` pipeline (`filter -> rearrange/transpose -> baseline -> exclude/*`).

use std::collections::BTreeMap;

use curvebook_doc::{
    BaselineOperation, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ContingencyTable, DataSetColumn, DataSheet, DelimitedScanError,
    DelimitedScanOptions, DelimitedVisitorError, DocError, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkedDataRegion,
    MAX_DATA_SETS, MAX_ROWS, MAX_SUBCOLUMNS, MultipleVariablesTable, NestedGroupColumn, NestedTable, PartsOfWholeTable, PlacementMode,
    SheetCellRange, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, VariableColumn, XColumnMode, XyTable, default_column_label,
    infer_number_separators, parse_import_number, scan_delimited, transpose_cell_grid, validate_cell_grid, validate_data_sets, validate_rows,
    validate_subcolumns, validate_title,
};

use crate::error::FormatError;

/// Maximum dense `Cell` entries allocated by delimited table import, filtering, or growth.
pub const MAX_IMPORT_CELLS: usize = 10_000_000;

/// Maximum byte length permitted when reading a CSV/TSV or delimited text file from disk.
pub const MAX_CSV_FILE_BYTES: usize = 64 * 1024 * 1024;

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

    /// Auto-detect delimiter (`,`, `;`, `\t`), decimal separator (`.` vs `,`), and optional
    /// thousands grouping separator from sample text (handling quoted fields and multi-line cells).
    #[must_use]
    pub fn detect(input: &str) -> Self {
        let sample = input.strip_prefix('\u{FEFF}').unwrap_or(input);
        let lines = detect_sample_lines(sample);
        if lines.is_empty() {
            return Self::csv();
        }

        let mut best_delim = ',';
        let mut best_score = 0_usize;
        let mut explained_commas_per_line: Option<Vec<usize>> = None;

        for delim in ['\t', ';'] {
            let counts: Vec<usize> = lines.iter().map(|l| count_unquoted_char(l, delim, '"')).collect();
            let first = counts.first().copied().unwrap_or(0);
            let score = if first > 0 && counts.iter().all(|&c| c == first) {
                first.saturating_mul(lines.len()).saturating_add(10)
            } else {
                counts.iter().sum()
            };
            if score > best_score {
                best_score = score;
                best_delim = delim;
                let (per_line, num_commas) = count_numeric_commas_under_delim(&lines, &counts, delim);
                explained_commas_per_line = (num_commas > 0).then_some(per_line);
            }
        }

        let raw_comma_counts: Vec<usize> = lines.iter().map(|l| count_unquoted_char(l, ',', '"')).collect();
        let effective_comma_counts: Vec<usize> = match explained_commas_per_line {
            Some(ref explained) => raw_comma_counts.iter().zip(explained.iter()).map(|(&raw, &exp)| raw.saturating_sub(exp)).collect(),
            None => raw_comma_counts,
        };
        let first_comma = effective_comma_counts.first().copied().unwrap_or(0);
        let comma_score = if first_comma > 0 && effective_comma_counts.iter().all(|&c| c == first_comma) {
            first_comma.saturating_mul(lines.len()).saturating_add(10)
        } else {
            effective_comma_counts.iter().sum()
        };
        if comma_score > best_score {
            best_delim = ',';
        }

        let mut opts = Self { delimiter: best_delim, ..Self::csv() };
        let sample_tokens = collect_detection_tokens(sample, best_delim);
        let (decimal, thousands) = infer_number_separators(sample_tokens.iter().map(String::as_str), best_delim);
        opts.decimal_separator = decimal;
        opts.thousands_separator = thousands;
        opts
    }

    /// Detect decimal and thousands separators for tab-delimited (`\t`) text.
    #[must_use]
    pub fn detect_tsv(input: &str) -> Self {
        let sample = input.strip_prefix('\u{FEFF}').unwrap_or(input);
        let sample_tokens = collect_detection_tokens(sample, '\t');
        let (decimal, thousands) = infer_number_separators(sample_tokens.iter().map(String::as_str), '\t');
        Self { delimiter: '\t', decimal_separator: decimal, thousands_separator: thousands, ..Self::tsv() }
    }
}

/// Decode raw bytes for delimited text (CSV/TSV/clipboard), supporting UTF-8 (with or without BOM),
/// UTF-16 LE BOM (`0xFF 0xFE`), and UTF-16 BE BOM (`0xFE 0xFF`) while rejecting ZIP archives.
pub fn decode_delimited_text_bytes(bytes: &[u8], format_id: &str) -> Result<String, FormatError> {
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return Err(FormatError::UnsupportedFormat { reason: format!("binary ZIP archive payload cannot be imported as `{format_id}`") });
    }
    if bytes.len() > MAX_CSV_FILE_BYTES {
        return Err(FormatError::FileTooLarge {
            path: format_id.to_owned(),
            actual_bytes: Some(bytes.len() as u64),
            limit_bytes: MAX_CSV_FILE_BYTES,
        });
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16_bytes(rest, false, format_id);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16_bytes(rest, true, format_id);
    }
    let utf8_bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = std::str::from_utf8(utf8_bytes)
        .map_err(|err| FormatError::CsvParse { line: 1, reason: format!("invalid UTF-8 in {format_id} input: {err}") })?;
    Ok(text.strip_prefix('\u{FEFF}').unwrap_or(text).to_owned())
}

fn decode_utf16_bytes(bytes: &[u8], big_endian: bool, format_id: &str) -> Result<String, FormatError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(FormatError::CsvParse { line: 1, reason: format!("odd byte length in UTF-16 {format_id} input") });
    }
    let (chunks, _) = bytes.as_chunks::<2>();
    let units = chunks.iter().map(|&[b0, b1]| if big_endian { u16::from_be_bytes([b0, b1]) } else { u16::from_le_bytes([b0, b1]) });
    let mut out = String::with_capacity(bytes.len() / 2);
    for decoded in char::decode_utf16(units) {
        let ch = decoded.map_err(|_| FormatError::CsvParse { line: 1, reason: format!("invalid UTF-16 surrogate in {format_id} input") })?;
        out.push(ch);
    }
    Ok(out.strip_prefix('\u{FEFF}').unwrap_or(&out).to_owned())
}

fn collect_detection_tokens(sample: &str, delim: char) -> Vec<String> {
    let opts = DelimitedScanOptions { delimiter: delim, quote_char: Some('"') };
    let mut tokens = Vec::new();
    let mut fields_seen = 0_usize;
    let _ = scan_delimited(sample, opts, |field| {
        if field.record_index >= 32 || tokens.len() >= 1_024 || fields_seen >= 4_096 {
            return Err(());
        }
        fields_seen = fields_seen.saturating_add(1);
        if field.record_index > 0 || tokens.len() < 32 {
            tokens.push(field.text);
        }
        Ok(())
    });
    if tokens.is_empty() {
        for line in sample.lines().take(16) {
            for part in line.split(delim) {
                if tokens.len() >= 1_024 {
                    break;
                }
                tokens.push(part.trim_matches('"').to_owned());
            }
            if tokens.len() >= 1_024 {
                break;
            }
        }
    }
    tokens
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

fn detect_sample_lines(sample: &str) -> Vec<&str> {
    if !sample.contains('\n') && sample.contains('\r') {
        sample.split('\r').filter(|l| !l.trim().is_empty()).take(10).collect()
    } else {
        sample.lines().filter(|l| !l.trim().is_empty()).take(10).collect()
    }
}

fn token_is_numeric_with_comma(raw: &str) -> bool {
    let trimmed = raw.trim().trim_matches('*').trim();
    trimmed.contains(',') && (parse_import_number(trimmed, ',', None).is_some() || parse_import_number(trimmed, '.', Some(',')).is_some())
}

fn split_unquoted_line_fields(line: &str, delim: char, quote: char) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        if ch == quote {
            in_quotes = !in_quotes;
        } else if !in_quotes && ch == delim {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    fields.push(current);
    fields
}

fn count_numeric_commas_under_delim(lines: &[&str], delim_counts: &[usize], delim: char) -> (Vec<usize>, usize) {
    let mut per_line = Vec::with_capacity(lines.len());
    let mut total = 0_usize;
    for (&line, &dcount) in lines.iter().zip(delim_counts.iter()) {
        if dcount == 0 {
            per_line.push(0);
            continue;
        }
        let mut line_commas = 0_usize;
        for field in split_unquoted_line_fields(line, delim, '"') {
            if token_is_numeric_with_comma(&field) {
                line_commas = line_commas.saturating_add(field.chars().filter(|&c| c == ',').count());
            }
        }
        total = total.saturating_add(line_commas);
        per_line.push(line_commas);
    }
    (per_line, total)
}

/// Parse RFC-4180 delimited text into rectangular/jagged string records.
///
/// Strips optional UTF-8 BOM, handles `\r\n` / `\n` / `\r`, quoted delimiters/newlines, and
/// doubled quote escapes (`""`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCsvField {
    text: String,
    quoted: bool,
}

fn csv_default_limits(input: &str) -> (usize, usize) {
    let max_cols = MAX_DATA_SETS.saturating_mul(MAX_SUBCOLUMNS).saturating_add(4);
    let slot_bytes = std::mem::size_of::<String>().max(1);
    let max_fields_by_bytes = input.len().saturating_mul(4).max(12 * 1024 * 1024) / slot_bytes;
    let max_fields = MAX_IMPORT_CELLS.saturating_add(65_536).min(max_fields_by_bytes);
    (max_cols, max_fields)
}

pub fn parse_delimited_records(input: &str, opts: &CsvOptions) -> Result<Vec<Vec<String>>, FormatError> {
    let (max_cols, max_fields) = csv_default_limits(input);
    parse_delimited_records_with_limits(input, opts, max_cols, max_fields)
}

pub(crate) fn parse_delimited_records_with_limits(
    input: &str,
    opts: &CsvOptions,
    max_cols: usize,
    max_fields: usize,
) -> Result<Vec<Vec<String>>, FormatError> {
    let rows = parse_delimited_records_with_quotes_and_limits(input, opts, max_cols, max_fields)?;
    Ok(rows.into_iter().map(|r| r.into_iter().map(|f| f.text).collect()).collect())
}

fn map_csv_scan_error(err: DelimitedScanError) -> FormatError {
    match err {
        DelimitedScanError::UnexpectedQuoteInUnquotedField { line } => {
            FormatError::CsvParse { line, reason: String::from("unexpected quote inside unquoted field") }
        }
        DelimitedScanError::UnexpectedCharAfterClosingQuote { line, ch } => {
            FormatError::CsvParse { line, reason: format!("unexpected character `{ch}` after closing quote") }
        }
        DelimitedScanError::UnclosedQuotedField { line } => FormatError::CsvParse { line, reason: String::from("unclosed quoted field at EOF") },
    }
}

fn parse_delimited_records_with_quotes_and_limits(
    input: &str,
    opts: &CsvOptions,
    max_cols: usize,
    max_fields: usize,
) -> Result<Vec<Vec<ParsedCsvField>>, FormatError> {
    let mut records: Vec<Vec<ParsedCsvField>> = Vec::new();
    let mut current_row: Vec<ParsedCsvField> = Vec::new();
    let mut total_fields = 0_usize;
    let scan_opts = DelimitedScanOptions { delimiter: opts.delimiter, quote_char: Some(opts.quote_char) };

    scan_delimited(input, scan_opts, |field| {
        if current_row.len() >= max_cols {
            return Err(FormatError::CsvParse { line: field.line, reason: format!("column count exceeds limit {max_cols}") });
        }
        let next_total = total_fields.saturating_add(1);
        if next_total > max_fields {
            return Err(FormatError::CsvParse { line: field.line, reason: format!("delimited field count exceeds limit {max_fields}") });
        }
        total_fields = next_total;
        current_row.push(ParsedCsvField { text: field.text, quoted: field.quoted });
        if field.end_of_record {
            if records.len() >= MAX_ROWS + 16 {
                return Err(FormatError::CsvParse { line: field.line, reason: format!("row count exceeds maximum {MAX_ROWS}") });
            }
            records.push(std::mem::take(&mut current_row));
        }
        Ok(())
    })
    .map_err(|err| match err {
        DelimitedVisitorError::Syntax(s) => map_csv_scan_error(s),
        DelimitedVisitorError::Callback(e) => e,
    })?;

    Ok(records)
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

    if let Some(num) = parse_import_number(core, opts.decimal_separator, opts.thousands_separator) {
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

const QUOTED_EXCLUDED_TAG: &str = "[excluded]";

/// Match `[excluded]` followed by `k >= 0` backslashes and `:`, returning `(k, rest)`.
///
/// Using `k == 0` (`[excluded]:<payload>`) for excluded quoted text and `k + 1` backslashes to
/// escape active text that already matches the envelope forms a lossless bijection over all
/// strings while preventing collisions with external quoted strings like `"""label""*"`.
fn split_quoted_excluded_envelope(text: &str) -> Option<(usize, &str)> {
    let after_tag = text.strip_prefix(QUOTED_EXCLUDED_TAG)?;
    let backslashes = after_tag.bytes().take_while(|&b| b == b'\\').count();
    let rest = after_tag.get(backslashes..)?.strip_prefix(':')?;
    Some((backslashes, rest))
}

fn encode_quoted_excluded_text(safe: &str) -> String {
    format!("{QUOTED_EXCLUDED_TAG}:{safe}")
}

fn escape_quoted_active_text(safe: &str) -> Option<String> {
    let (backslashes, rest) = split_quoted_excluded_envelope(safe)?;
    let mut out = String::with_capacity(safe.len().saturating_add(1));
    out.push_str(QUOTED_EXCLUDED_TAG);
    for _ in 0..=backslashes {
        out.push('\\');
    }
    out.push(':');
    out.push_str(rest);
    Some(out)
}

fn decode_quoted_excluded_or_escaped(text: &str) -> Option<Cell> {
    let (backslashes, rest) = split_quoted_excluded_envelope(text)?;
    if backslashes == 0 {
        Some(Cell::excluded_text(rest.to_owned()))
    } else {
        let mut out = String::with_capacity(text.len().saturating_sub(1));
        out.push_str(QUOTED_EXCLUDED_TAG);
        for _ in 1..backslashes {
            out.push('\\');
        }
        out.push(':');
        out.push_str(rest);
        Some(Cell::text(out))
    }
}

fn parse_csv_field_cell(field: &ParsedCsvField, opts: &CsvOptions) -> Cell {
    if field.quoted {
        if opts.parse_asterisk_exclusions
            && let Some(cell) = decode_quoted_excluded_or_escaped(&field.text)
        {
            return cell;
        }
        let delimiter_is_numeric_sep = opts.delimiter == opts.decimal_separator || opts.thousands_separator == Some(opts.delimiter);
        if delimiter_is_numeric_sep && field.text.contains(opts.delimiter) {
            let parsed = parse_cell_token(&field.text, opts);
            if matches!(parsed.value, CellValue::Number(_)) {
                return parsed;
            }
        }
        Cell::text(field.text.clone())
    } else {
        parse_cell_token(&field.text, opts)
    }
}

fn can_export_excluded_text_unquoted(safe: &str, opts: &CsvOptions) -> bool {
    if safe.is_empty()
        || safe != safe.trim()
        || safe.contains(opts.delimiter)
        || safe.contains(opts.quote_char)
        || safe.contains('\n')
        || safe.contains('\r')
        || is_missing_sentinel(safe, &default_missing_sentinels())
    {
        return false;
    }
    let candidate = format!("{safe}*");
    parse_cell_token(&candidate, opts) == Cell::excluded_text(safe.to_owned())
}

/// Neutralize spreadsheet formula-injection prefixes (`=`, `+`, `-`, `@`, `\t`, `\r`) on text
/// exported into CSV/TSV cells and labels while leaving finite numbers untouched.
pub(crate) fn neutralize_csv_formula_text(raw: &str) -> String {
    if raw.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        let mut out = String::with_capacity(raw.len() + 1);
        out.push('\'');
        out.push_str(raw);
        out
    } else {
        raw.to_owned()
    }
}

fn is_ambiguous_csv_text(safe: &str, opts: &CsvOptions) -> bool {
    if safe.is_empty() || safe != safe.trim() || safe.starts_with('*') || safe.ends_with('*') {
        return true;
    }
    if is_missing_sentinel(safe, &default_missing_sentinels()) {
        return true;
    }
    parse_cell_token(safe, opts) != Cell::text(safe.to_owned())
}

fn quote_csv_field(field: &str, quote_char: char) -> String {
    let mut out = String::with_capacity(field.len() + 2);
    out.push(quote_char);
    for ch in field.chars() {
        if ch == quote_char {
            out.push(quote_char);
        }
        out.push(ch);
    }
    out.push(quote_char);
    out
}

/// Escape and quote a single CSV/TSV field per RFC-4180.
#[must_use]
pub fn format_csv_field(field: &str, opts: &CsvOptions) -> String {
    let needs_quotes = field.contains(opts.delimiter) || field.contains(opts.quote_char) || field.contains('\n') || field.contains('\r');
    if !needs_quotes {
        return field.to_owned();
    }
    quote_csv_field(field, opts.quote_char)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CsvExportField {
    text: String,
    force_quote: bool,
}

impl CsvExportField {
    fn unquoted(text: String) -> Self {
        Self { text, force_quote: false }
    }

    fn label(raw: &str) -> Self {
        let safe = neutralize_csv_formula_text(raw);
        let force_quote = safe != safe.trim();
        Self { text: safe, force_quote }
    }

    fn header(raw: &str) -> Self {
        Self::label(raw)
    }

    fn header_with_suffix(raw_base: &str, suffix: &str) -> Self {
        let safe_base = neutralize_csv_formula_text(raw_base);
        let force_quote = safe_base != safe_base.trim();
        Self { text: format!("{safe_base}:{suffix}"), force_quote }
    }

    fn cell(cell: &Cell, opts: &CsvOptions) -> Self {
        match &cell.value {
            CellValue::Empty | CellValue::Number(_) => Self::unquoted(format_cell_token(cell, opts)),
            CellValue::Text(raw) => {
                let safe = neutralize_csv_formula_text(raw);
                if cell.excluded && opts.parse_asterisk_exclusions {
                    if can_export_excluded_text_unquoted(&safe, opts) {
                        let mut text = safe;
                        text.push('*');
                        Self::unquoted(text)
                    } else {
                        Self { text: encode_quoted_excluded_text(&safe), force_quote: true }
                    }
                } else if opts.parse_asterisk_exclusions
                    && let Some(escaped) = escape_quoted_active_text(&safe)
                {
                    Self { text: escaped, force_quote: true }
                } else {
                    let force_quote = is_ambiguous_csv_text(&safe, opts);
                    Self { text: safe, force_quote }
                }
            }
        }
    }

    fn format(&self, opts: &CsvOptions) -> String {
        if self.force_quote { quote_csv_field(&self.text, opts.quote_char) } else { format_csv_field(&self.text, opts) }
    }
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

fn write_export_records(rows: &[Vec<CsvExportField>], opts: &CsvOptions) -> String {
    let mut out = String::new();
    for (r_idx, row) in rows.iter().enumerate() {
        if r_idx > 0 {
            out.push('\n');
        }
        for (c_idx, field) in row.iter().enumerate() {
            if c_idx > 0 {
                out.push(opts.delimiter);
            }
            out.push_str(&field.format(opts));
        }
    }
    out
}

/// Export a [`DataSheet`] into RFC-4180 CSV/TSV text, preserving headers, titles, and `*` exclusions.
pub fn export_datasheet_to_csv(sheet: &DataSheet, opts: &CsvOptions) -> Result<String, FormatError> {
    sheet.archetype.validate()?;
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
    Ok(write_export_records(&rows, opts))
}

fn export_xy_rows(t: &XyTable, opts: &CsvOptions) -> Vec<Vec<CsvExportField>> {
    let include_row_titles = t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty());
    let mut header = Vec::new();
    if include_row_titles {
        header.push(CsvExportField::header("Row Title"));
    }
    let is_gen_seq = matches!(t.x_mode, XColumnMode::GeneratedSequence { .. });
    if is_gen_seq {
        header.push(CsvExportField::header(&t.x_title));
    } else {
        let x_subs = t.x_mode.subcolumn_count();
        match t.x_mode {
            XColumnMode::Numbers { subcolumn_mode } if subcolumn_mode.is_pre_summarized() => {
                let sub_labels = subcolumn_mode.subcolumn_labels();
                for s in 0..x_subs {
                    let suffix = sub_labels.get(s).map(String::as_str).unwrap_or("X");
                    header.push(CsvExportField::header_with_suffix(&t.x_title, suffix));
                }
            }
            _ => {
                for s in 0..x_subs {
                    let raw = if s == 0 { t.x_title.clone() } else { format!("{}:{}", t.x_title, s + 1) };
                    header.push(CsvExportField::header(&raw));
                }
            }
        }
    }
    push_dataset_headers(&mut header, &t.data_sets, t.y_subcolumn_mode);
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(CsvExportField::label(t.row_titles.get(r).map(String::as_str).unwrap_or("")));
        }
        if is_gen_seq {
            let cell = t.materialized_x_cell_at(r);
            row.push(CsvExportField::cell(&cell, opts));
        } else {
            let x_subs = t.x_mode.subcolumn_count();
            for s in 0..x_subs {
                let field = t.x_cells.get(r).and_then(|subs| subs.get(s)).map(|c| CsvExportField::cell(c, opts)).unwrap_or_else(|| {
                    t.x_value_at(r).map_or_else(|| CsvExportField::unquoted(String::new()), |v| CsvExportField::cell(&Cell::number(v), opts))
                });
                row.push(field);
            }
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
) -> Vec<Vec<CsvExportField>> {
    let mut header = Vec::new();
    if include_row_titles {
        header.push(CsvExportField::header(row_title_header));
    }
    push_dataset_headers(&mut header, data_sets, mode);
    let mut rows = vec![header];
    for r in 0..row_count {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(CsvExportField::label(row_titles.get(r).map(String::as_str).unwrap_or("")));
        }
        push_dataset_row(&mut row, data_sets, r, opts);
        rows.push(row);
    }
    rows
}

fn export_survival_rows(t: &SurvivalTable, opts: &CsvOptions) -> Vec<Vec<CsvExportField>> {
    let include_row_titles = t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty());
    let mut header = Vec::new();
    if include_row_titles {
        header.push(CsvExportField::header("Row Title"));
    }
    header.push(CsvExportField::header(&t.time_title));
    push_dataset_headers(&mut header, &t.cohorts, SubcolumnMode::Single);
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(CsvExportField::label(t.row_titles.get(r).map(String::as_str).unwrap_or("")));
        }
        let t_cell = t.times.get(r).cloned().unwrap_or_default();
        row.push(CsvExportField::cell(&t_cell, opts));
        push_dataset_row(&mut row, &t.cohorts, r, opts);
        rows.push(row);
    }
    rows
}

fn export_parts_of_whole_rows(t: &PartsOfWholeTable, opts: &CsvOptions) -> Vec<Vec<CsvExportField>> {
    let mut header = vec![CsvExportField::header("Slice")];
    push_dataset_headers(&mut header, &t.data_sets, SubcolumnMode::Single);
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = vec![CsvExportField::label(t.slice_labels.get(r).map(String::as_str).unwrap_or(""))];
        push_dataset_row(&mut row, &t.data_sets, r, opts);
        rows.push(row);
    }
    rows
}

fn export_multivar_rows(t: &MultipleVariablesTable, opts: &CsvOptions) -> Vec<Vec<CsvExportField>> {
    let include_row_titles = t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty());
    let mut header = Vec::new();
    if include_row_titles {
        header.push(CsvExportField::header("Row Title"));
    }
    for var in &t.variables {
        header.push(CsvExportField::header(&var.title));
    }
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(CsvExportField::label(t.row_titles.get(r).map(String::as_str).unwrap_or("")));
        }
        for var in &t.variables {
            let cell = var.cells.get(r).cloned().unwrap_or_default();
            row.push(CsvExportField::cell(&cell, opts));
        }
        rows.push(row);
    }
    rows
}

fn format_nested_subcolumn_header(group_title: &str, sub_title: &str, s_idx: usize) -> CsvExportField {
    let raw_prefix = format!("{group_title}:");
    if let Some(rest) = sub_title.strip_prefix(&raw_prefix)
        && !rest.trim().is_empty()
    {
        return CsvExportField::header(sub_title);
    }
    let trimmed_group = group_title.trim();
    if trimmed_group.is_empty() {
        return CsvExportField::header(sub_title);
    }
    let trimmed_prefix = format!("{trimmed_group}:");
    let trimmed_sub = sub_title.trim();
    if let Some(rest) = trimmed_sub.strip_prefix(&trimmed_prefix) {
        return if rest.trim().is_empty() {
            CsvExportField::header(&format!("{trimmed_prefix}{}", s_idx + 1))
        } else {
            CsvExportField::header(trimmed_sub)
        };
    }
    let effective_sub = if trimmed_sub.is_empty() { format!("{}", s_idx + 1) } else { trimmed_sub.to_owned() };
    CsvExportField::header(&format!("{trimmed_prefix}{effective_sub}"))
}

fn export_nested_rows(t: &NestedTable, opts: &CsvOptions) -> Vec<Vec<CsvExportField>> {
    let include_row_titles = t.row_titles_expanded || t.row_titles.iter().any(|s| !s.is_empty());
    let mut header = Vec::new();
    if include_row_titles {
        header.push(CsvExportField::header("Row Title"));
    }
    for g in &t.groups {
        for (s_idx, sub_title) in g.subcolumn_titles.iter().enumerate() {
            header.push(format_nested_subcolumn_header(&g.title, sub_title, s_idx));
        }
    }
    let mut rows = vec![header];
    for r in 0..t.row_count() {
        let mut row = Vec::new();
        if include_row_titles {
            row.push(CsvExportField::label(t.row_titles.get(r).map(String::as_str).unwrap_or("")));
        }
        for g in &t.groups {
            for s in 0..g.biological_unit_count() {
                let cell = g.cells.get(r).and_then(|subs| subs.get(s)).cloned().unwrap_or_default();
                row.push(CsvExportField::cell(&cell, opts));
            }
        }
        rows.push(row);
    }
    rows
}

fn push_dataset_headers(header: &mut Vec<CsvExportField>, data_sets: &[DataSetColumn], mode: SubcolumnMode) {
    let sub_labels = mode.subcolumn_labels();
    let subs = mode.subcolumn_count();
    for ds in data_sets {
        if subs <= 1 {
            header.push(CsvExportField::header(&ds.title));
        } else {
            for idx in 0..subs {
                let suffix = sub_labels.get(idx).map(String::as_str).unwrap_or("Y");
                header.push(CsvExportField::header_with_suffix(&ds.title, suffix));
            }
        }
    }
}

fn push_dataset_row(row: &mut Vec<CsvExportField>, data_sets: &[DataSetColumn], r: usize, opts: &CsvOptions) {
    for ds in data_sets {
        let subs = ds.subcolumn_count();
        for s in 0..subs {
            let cell = ds.cell(r, s).cloned().unwrap_or_default();
            row.push(CsvExportField::cell(&cell, opts));
        }
    }
}

/// Detect contiguous multi-subcolumn `X` headers (`X:Mean/X:SD/X:N`, `X:Mean/X:Lower/X:Upper`,
/// `X/X:2/X:3`, `X:1/X:2/X:3`, or `X:X1/X:X2/...`) starting at `col_offset` so multi-subcolumn
/// `XyTable` CSV/XLSX round-trips preserve `X` subcolumns instead of shifting them into `Y` datasets.
pub(crate) fn detect_xy_x_subcolumn_header(headers: &[&str], col_offset: usize) -> Option<(String, SubcolumnMode)> {
    let first = headers.get(col_offset)?.trim();
    if first.is_empty() {
        return None;
    }

    // 1. Check 3-column pre-summarized X header triples (`{base}:Mean`, `{base}:SD`, `{base}:N`, ...).
    if let (Some(h0), Some(h1), Some(h2)) = (headers.get(col_offset), headers.get(col_offset + 1), headers.get(col_offset + 2))
        && let (Some((p0, s0)), Some((p1, s1)), Some((p2, s2))) = (h0.trim().rsplit_once(':'), h1.trim().rsplit_once(':'), h2.trim().rsplit_once(':'))
    {
        let base = p0.trim();
        if !base.is_empty() && p1.trim() == base && p2.trim() == base {
            let t0 = s0.trim().to_ascii_lowercase();
            let t1 = s1.trim().to_ascii_lowercase();
            let t2 = s2.trim().to_ascii_lowercase();
            let mode = match (t0.as_str(), t1.as_str(), t2.as_str()) {
                ("mean", "sd", "n") => Some(SubcolumnMode::MeanSdN),
                ("mean", "sem", "n") => Some(SubcolumnMode::MeanSemN),
                ("mean", "%cv" | "cv", "n") => Some(SubcolumnMode::MeanCvN),
                ("mean", "-error", "+error") => Some(SubcolumnMode::MeanLowHigh),
                ("mean", "lower" | "lowerlimit" | "lower limit", "upper" | "upperlimit" | "upper limit") => Some(SubcolumnMode::UpperLowerLimits),
                ("median", "q1", "q3") => Some(SubcolumnMode::MedianIqr),
                _ => None,
            };
            if let Some(m) = mode {
                return Some((base.to_owned(), m));
            }
        }
    }

    // 2. Check `{base}`, `{base}:2`, `{base}:3`, ... contiguous sequence.
    let mut numbered_count = 1_usize;
    while numbered_count < MAX_SUBCOLUMNS
        && let Some(next_h) = headers.get(col_offset + numbered_count)
    {
        let expected = format!("{first}:{}", numbered_count + 1);
        if next_h.trim() == expected {
            numbered_count += 1;
        } else {
            break;
        }
    }
    if numbered_count >= 2 {
        return Some((first.to_owned(), SubcolumnMode::RawReplicates(numbered_count)));
    }

    // 3. Check `{base}:1`, `{base}:2`, ... or `{base}:X1`, `{base}:X2`, ... contiguous sequence.
    if let Some((base_raw, suf0_raw)) = first.rsplit_once(':') {
        let base = base_raw.trim();
        let suf0 = suf0_raw.trim();
        let is_x_prefixed = suf0.eq_ignore_ascii_case("x1");
        if !base.is_empty() && (suf0 == "1" || is_x_prefixed) {
            let mut count = 1_usize;
            while count < MAX_SUBCOLUMNS
                && let Some(next_h) = headers.get(col_offset + count)
                && let Some((nb, ns)) = next_h.trim().rsplit_once(':')
            {
                let ns_t = ns.trim();
                let matches_next =
                    if is_x_prefixed { ns_t.eq_ignore_ascii_case(&format!("x{}", count + 1)) } else { ns_t == format!("{}", count + 1) };
                if nb.trim() == base && matches_next {
                    count += 1;
                } else {
                    break;
                }
            }
            if count >= 2 {
                return Some((base.to_owned(), SubcolumnMode::RawReplicates(count)));
            }
        }
    }

    None
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
    import_csv_to_datasheet_with_budget_and_x_mode(id, title, input, target_kind, subcolumn_mode, None, opts, MAX_IMPORT_CELLS)
}

/// Import delimited CSV/TSV text into a typed [`DataSheet`] with an explicit optional `X`
/// subcolumn mode for [`TableKind::Xy`] tables (useful for headerless multi-subcolumn `X` inputs).
pub fn import_csv_to_datasheet_with_x_mode(
    id: impl Into<SheetId>,
    title: impl Into<String>,
    input: &str,
    target_kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    x_subcolumn_mode: Option<SubcolumnMode>,
    opts: &CsvOptions,
) -> Result<DataSheet, FormatError> {
    import_csv_to_datasheet_with_budget_and_x_mode(id, title, input, target_kind, subcolumn_mode, x_subcolumn_mode, opts, MAX_IMPORT_CELLS)
}

/// Import delimited CSV/TSV text into a typed [`DataSheet`] bounded by `max_cells`.
pub(crate) fn import_csv_to_datasheet_with_budget(
    id: impl Into<SheetId>,
    title: impl Into<String>,
    input: &str,
    target_kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    opts: &CsvOptions,
    max_cells: usize,
) -> Result<DataSheet, FormatError> {
    import_csv_to_datasheet_with_budget_and_x_mode(id, title, input, target_kind, subcolumn_mode, None, opts, max_cells)
}

#[allow(clippy::too_many_arguments)]
fn import_csv_to_datasheet_with_budget_and_x_mode(
    id: impl Into<SheetId>,
    title: impl Into<String>,
    input: &str,
    target_kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    explicit_x_subcolumn_mode: Option<SubcolumnMode>,
    opts: &CsvOptions,
    max_cells: usize,
) -> Result<DataSheet, FormatError> {
    subcolumn_mode.validate()?;
    if !target_kind.allows_subcolumn_mode(subcolumn_mode) {
        return Err(FormatError::Doc(curvebook_doc::DocError::IncompatibleSubcolumnMode { archetype: target_kind, mode: subcolumn_mode }));
    }
    if let Some(x_sub) = explicit_x_subcolumn_mode {
        x_sub.validate()?;
        if target_kind != TableKind::Xy {
            return Err(FormatError::Doc(curvebook_doc::DocError::IncompatibleSubcolumnMode { archetype: target_kind, mode: x_sub }));
        }
    }
    let prefix_cols = usize::from(opts.has_row_titles)
        + match target_kind {
            TableKind::Xy => explicit_x_subcolumn_mode.map_or(MAX_SUBCOLUMNS, SubcolumnMode::subcolumn_count),
            TableKind::Survival => 1,
            TableKind::PartsOfWhole if !opts.has_row_titles => 1,
            _ => 0,
        };
    let data_col_limit = if matches!(target_kind, TableKind::Nested) {
        MAX_DATA_SETS.saturating_mul(MAX_SUBCOLUMNS)
    } else {
        MAX_DATA_SETS.saturating_mul(subcolumn_mode.subcolumn_count())
    };
    let max_cols = prefix_cols.saturating_add(data_col_limit);
    let header_slack = opts.header_rows.saturating_mul(max_cols).min(65_536);
    let max_fields = max_cells.saturating_add(max_cells.min(MAX_ROWS).saturating_mul(prefix_cols)).saturating_add(header_slack).max(4_096);
    let records = parse_delimited_records_with_quotes_and_limits(input, opts, max_cols, max_fields)?;

    let header_count = opts.header_rows.min(records.len());
    let headers = collapse_delimited_headers(records.get(..header_count).unwrap_or(&[]));
    let data_rows = records.get(header_count..).unwrap_or(&[]);
    let row_count = data_rows.len();
    validate_rows(row_count)?;

    let raw_col_count = headers.len().max(data_rows.iter().map(Vec::len).max().unwrap_or(0));
    let has_row_titles = opts.has_row_titles && raw_col_count > 0;
    let mut col_offset = usize::from(has_row_titles);
    let err_line = header_count.saturating_add(1);

    let arch = match target_kind {
        TableKind::Xy => {
            let header_strs: Vec<&str> = headers.iter().map(|f| f.text.as_str()).collect();
            let detected_x = detect_xy_x_subcolumn_header(&header_strs, col_offset);
            let x_sub_mode = explicit_x_subcolumn_mode.or_else(|| detected_x.as_ref().map(|(_, m)| *m)).unwrap_or(SubcolumnMode::Single);
            let x_subs = x_sub_mode.subcolumn_count();
            let x_title_field = headers.get(col_offset).filter(|f| !f.text.trim().is_empty());
            let x_col_idx = col_offset;
            let available_x_cols = raw_col_count.saturating_sub(col_offset).min(x_subs);
            col_offset = col_offset.saturating_add(available_x_cols);
            let subs = subcolumn_mode.subcolumn_count();
            let data_cols = raw_col_count.saturating_sub(col_offset);
            let ds_count = data_cols.div_ceil(subs);
            validate_data_sets(ds_count)?;
            validate_csv_cell_budget(row_count, ds_count, subs, x_subs, err_line, max_cells)?;
            let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: x_sub_mode }, subcolumn_mode, ds_count, row_count)?;
            xy.x_title = match detected_x {
                Some((base, _)) => sanitize_title(&base)?,
                None => match x_title_field {
                    Some(f) => header_field_title(f)?,
                    None => String::from("X"),
                },
            };
            assign_row_titles(&mut xy.row_titles, data_rows, has_row_titles);
            xy.row_titles_expanded = opts.has_row_titles;
            for (r, rec) in data_rows.iter().enumerate() {
                for s in 0..available_x_cols {
                    if let Some(tok) = rec.get(x_col_idx + s) {
                        xy.set_x_subcell(r, s, parse_csv_field_cell(tok, opts))?;
                    }
                }
            }
            populate_datasets(&mut xy.data_sets, data_rows, &headers, col_offset, subs, opts)?;
            TableArchetype::Xy(xy)
        }
        TableKind::Column => {
            let subs = subcolumn_mode.subcolumn_count();
            let ds_count = raw_col_count.saturating_sub(col_offset).div_ceil(subs);
            validate_data_sets(ds_count)?;
            validate_csv_cell_budget(row_count, ds_count, subs, 0, err_line, max_cells)?;
            let hint = if subcolumn_mode.is_pre_summarized() { ColumnDesignHint::PrecomputedError } else { ColumnDesignHint::UnpairedReplicates };
            let mut col = ColumnTable::new_with_subcolumns(hint, subcolumn_mode, ds_count, row_count)?;
            assign_row_titles(&mut col.row_titles, data_rows, has_row_titles);
            col.row_titles_expanded = opts.has_row_titles;
            populate_datasets(&mut col.data_sets, data_rows, &headers, col_offset, subs, opts)?;
            TableArchetype::Column(col)
        }
        TableKind::Grouped => {
            let subs = subcolumn_mode.subcolumn_count();
            let ds_count = raw_col_count.saturating_sub(col_offset).div_ceil(subs);
            validate_data_sets(ds_count)?;
            validate_csv_cell_budget(row_count, ds_count, subs, 0, err_line, max_cells)?;
            let mut grp = GroupedTable::new(subcolumn_mode, ds_count, row_count)?;
            assign_row_titles(&mut grp.row_titles, data_rows, has_row_titles);
            populate_datasets(&mut grp.data_sets, data_rows, &headers, col_offset, subs, opts)?;
            TableArchetype::Grouped(grp)
        }
        TableKind::Contingency => {
            let ds_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(ds_count)?;
            validate_csv_cell_budget(row_count, ds_count, 1, 0, err_line, max_cells)?;
            let mut cont = ContingencyTable::new(ds_count, row_count)?;
            assign_row_titles(&mut cont.row_titles, data_rows, has_row_titles);
            populate_datasets(&mut cont.data_sets, data_rows, &headers, col_offset, 1, opts)?;
            TableArchetype::Contingency(cont)
        }
        TableKind::Survival => {
            let time_title_field = headers.get(col_offset).filter(|f| !f.text.trim().is_empty());
            let time_col = col_offset;
            if raw_col_count > col_offset {
                col_offset += 1;
            }
            let ds_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(ds_count)?;
            validate_csv_cell_budget(row_count, ds_count, 1, 1, err_line, max_cells)?;
            let mut surv = SurvivalTable::new(ds_count, row_count)?;
            surv.time_title = match time_title_field {
                Some(f) => header_field_title(f)?,
                None => String::from("Elapsed Time"),
            };
            assign_row_titles(&mut surv.row_titles, data_rows, has_row_titles);
            surv.row_titles_expanded = opts.has_row_titles;
            for (r, rec) in data_rows.iter().enumerate() {
                if let Some(tok) = rec.get(time_col)
                    && let Some(slot) = surv.times.get_mut(r)
                {
                    *slot = parse_csv_field_cell(tok, opts);
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
            validate_csv_cell_budget(row_count, ds_count, 1, 0, err_line, max_cells)?;
            let mut pow = PartsOfWholeTable::new(ds_count, row_count)?;
            if has_row_titles {
                assign_row_titles(&mut pow.slice_labels, data_rows, true);
            } else if !opts.has_row_titles {
                pow.slice_labels = data_rows.iter().map(|r| r.get(label_col).map(|f| f.text.clone()).unwrap_or_default()).collect();
            }
            populate_datasets(&mut pow.data_sets, data_rows, &headers, col_offset, 1, opts)?;
            TableArchetype::PartsOfWhole(pow)
        }
        TableKind::MultipleVariables => {
            let var_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(var_count)?;
            validate_csv_cell_budget(row_count, var_count, 1, 0, err_line, max_cells)?;
            let mut mv = MultipleVariablesTable::new(var_count, row_count)?;
            assign_row_titles(&mut mv.row_titles, data_rows, has_row_titles);
            mv.row_titles_expanded = opts.has_row_titles;
            for (idx, var) in mv.variables.iter_mut().enumerate() {
                let c = col_offset + idx;
                if let Some(h) = headers.get(c).filter(|s| !s.text.trim().is_empty()) {
                    var.title = header_field_title(h)?;
                }
                for (r, rec) in data_rows.iter().enumerate() {
                    if let Some(tok) = rec.get(c)
                        && let Some(slot) = var.cells.get_mut(r)
                    {
                        *slot = parse_csv_field_cell(tok, opts);
                    }
                }
                var.refresh_inferred_role();
            }
            TableArchetype::MultipleVariables(mv)
        }
        TableKind::Nested => import_csv_nested_table(&CsvNestedContext {
            headers: &headers,
            data_rows,
            col_offset,
            row_count,
            has_row_titles,
            subcolumn_mode,
            opts,
            err_line,
            max_cells,
        })?,
    };

    Ok(DataSheet::new(id.into(), title, arch)?)
}

struct CsvNestedContext<'a> {
    headers: &'a [ParsedCsvField],
    data_rows: &'a [Vec<ParsedCsvField>],
    col_offset: usize,
    row_count: usize,
    has_row_titles: bool,
    subcolumn_mode: SubcolumnMode,
    opts: &'a CsvOptions,
    err_line: usize,
    max_cells: usize,
}

fn import_csv_nested_table(ctx: &CsvNestedContext<'_>) -> Result<TableArchetype, FormatError> {
    let value_headers: Vec<String> = ctx.headers.get(ctx.col_offset..).unwrap_or(&[]).iter().map(|f| f.text.clone()).collect();
    let groups_spec = parse_nested_column_groups(&value_headers, ctx.col_offset, ctx.subcolumn_mode);
    let group_count = groups_spec.len();
    validate_data_sets(group_count)?;
    let mut total_units: usize = 0;
    for spec in &groups_spec {
        let units = spec.col_indices.len().max(1);
        validate_subcolumns(units)?;
        total_units = total_units
            .checked_add(units)
            .ok_or_else(|| FormatError::CsvParse { line: ctx.err_line, reason: String::from("table cell count overflow") })?;
    }
    let total_cells = ctx
        .row_count
        .checked_mul(total_units)
        .ok_or_else(|| FormatError::CsvParse { line: ctx.err_line, reason: String::from("table cell count overflow") })?;
    check_csv_cell_limit(total_cells, ctx.err_line, ctx.max_cells)?;

    let mut nested = NestedTable::new(group_count, 1, ctx.row_count)?;
    assign_row_titles(&mut nested.row_titles, ctx.data_rows, ctx.has_row_titles);
    nested.row_titles_expanded = ctx.opts.has_row_titles;
    for (g_idx, spec) in groups_spec.iter().enumerate() {
        let title_str = if spec.group_title.trim().is_empty() { default_column_label(g_idx) } else { sanitize_title(&spec.group_title)? };
        let units = spec.col_indices.len().max(1);
        let mut grp = NestedGroupColumn::new(title_str, units, ctx.row_count)?;
        for (u_idx, raw_sub) in spec.sub_titles.iter().enumerate() {
            if let Some(sub_title) = raw_sub.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
                && let Some(slot) = grp.subcolumn_titles.get_mut(u_idx)
            {
                *slot = sanitize_title(sub_title)?;
            }
        }
        for (r, rec) in ctx.data_rows.iter().enumerate() {
            for (u_idx, &col_idx) in spec.col_indices.iter().enumerate() {
                if let Some(tok) = rec.get(col_idx)
                    && let Some(row_cells) = grp.cells.get_mut(r)
                    && let Some(cell) = row_cells.get_mut(u_idx)
                {
                    *cell = parse_csv_field_cell(tok, ctx.opts);
                }
            }
        }
        if let Some(slot) = nested.groups.get_mut(g_idx) {
            *slot = grp;
        }
    }
    Ok(TableArchetype::Nested(nested))
}

struct NestedColumnGroupSpec {
    group_title: String,
    col_indices: Vec<usize>,
    sub_titles: Vec<Option<String>>,
}

fn parse_nested_column_groups(value_headers: &[String], col_offset: usize, subcolumn_mode: SubcolumnMode) -> Vec<NestedColumnGroupSpec> {
    if value_headers.is_empty() {
        return Vec::new();
    }

    let split_pairs: Vec<Option<(&str, &str)>> =
        value_headers.iter().map(|h| h.rsplit_once(':').filter(|(p, s)| !p.trim().is_empty() && !s.trim().is_empty())).collect();

    let all_colon = split_pairs.iter().all(Option::is_some);
    let any_adjacent_colon_match = split_pairs.windows(2).any(|w| matches!(*w, [Some((p1, _)), Some((p2, _))] if p1.trim() == p2.trim()));

    if all_colon || any_adjacent_colon_match {
        return group_nested_by_colon(value_headers, &split_pairs, col_offset);
    }

    let chunk = match subcolumn_mode {
        SubcolumnMode::RawReplicates(k) if k > 1 => k,
        _ => 1,
    };
    if chunk > 1 && value_headers.len() >= chunk && value_headers.len().is_multiple_of(chunk) {
        return value_headers
            .chunks(chunk)
            .enumerate()
            .map(|(chunk_idx, cols)| {
                let start = col_offset + chunk_idx * chunk;
                let group_title = cols.first().map(|s| s.trim().to_owned()).unwrap_or_default();
                let col_indices = (start..start + cols.len()).collect();
                let sub_titles = cols.iter().map(|s| (!s.trim().is_empty()).then(|| s.trim().to_owned())).collect();
                NestedColumnGroupSpec { group_title, col_indices, sub_titles }
            })
            .collect();
    }

    value_headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let trimmed = h.trim().to_owned();
            NestedColumnGroupSpec { group_title: trimmed, col_indices: vec![col_offset + i], sub_titles: vec![None] }
        })
        .collect()
}

fn group_nested_by_colon(value_headers: &[String], split_pairs: &[Option<(&str, &str)>], col_offset: usize) -> Vec<NestedColumnGroupSpec> {
    let mut groups: Vec<NestedColumnGroupSpec> = Vec::new();
    for (i, h) in value_headers.iter().enumerate() {
        let col_idx = col_offset + i;
        if let Some((prefix, _)) = split_pairs.get(i).copied().flatten() {
            let prefix_trimmed = prefix.trim();
            if let Some(last) = groups.last_mut()
                && last.group_title == prefix_trimmed
                && !last.sub_titles.first().is_some_and(Option::is_none)
            {
                last.col_indices.push(col_idx);
                last.sub_titles.push(Some(h.trim().to_owned()));
                continue;
            }
            groups.push(NestedColumnGroupSpec {
                group_title: prefix_trimmed.to_owned(),
                col_indices: vec![col_idx],
                sub_titles: vec![Some(h.trim().to_owned())],
            });
        } else {
            groups.push(NestedColumnGroupSpec { group_title: h.trim().to_owned(), col_indices: vec![col_idx], sub_titles: vec![None] });
        }
    }
    groups
}

fn check_csv_cell_limit(total_cells: usize, line: usize, max_cells: usize) -> Result<(), FormatError> {
    if total_cells > max_cells {
        return Err(FormatError::CsvParse {
            line,
            reason: format!("dense cell allocation {total_cells} exceeds remaining import budget {max_cells} (cap {MAX_IMPORT_CELLS})"),
        });
    }
    Ok(())
}

fn validate_csv_cell_budget(
    rows: usize,
    cols: usize,
    subs: usize,
    extra_cols_per_row: usize,
    line: usize,
    max_cells: usize,
) -> Result<(), FormatError> {
    let per_row = cols
        .checked_mul(subs)
        .and_then(|v| v.checked_add(extra_cols_per_row))
        .ok_or_else(|| FormatError::CsvParse { line, reason: String::from("table cell count overflow") })?;
    let total_cells = rows.checked_mul(per_row).ok_or_else(|| FormatError::CsvParse { line, reason: String::from("table cell count overflow") })?;
    check_csv_cell_limit(total_cells, line, max_cells)
}

fn assign_row_titles(dst: &mut [String], data_rows: &[Vec<ParsedCsvField>], has_row_titles: bool) {
    if !has_row_titles {
        return;
    }
    for (slot, rec) in dst.iter_mut().zip(data_rows.iter()) {
        if let Some(first) = rec.first() {
            *slot = first.text.clone();
        }
    }
}

fn sanitize_title(raw: &str) -> Result<String, FormatError> {
    let trimmed = raw.trim().to_owned();
    validate_title(&trimmed)?;
    Ok(trimmed)
}

fn header_field_title(field: &ParsedCsvField) -> Result<String, FormatError> {
    if field.quoted {
        validate_title(&field.text)?;
        Ok(field.text.clone())
    } else {
        sanitize_title(&field.text)
    }
}

fn collapse_delimited_headers(header_rows: &[Vec<ParsedCsvField>]) -> Vec<ParsedCsvField> {
    match header_rows {
        [] => Vec::new(),
        [single] => single.clone(),
        many => {
            let raw_rows: Vec<Vec<String>> = many.iter().map(|row| row.iter().map(|f| f.text.clone()).collect()).collect();
            let col_count = many.iter().map(Vec::len).max().unwrap_or(0);
            let collapsed = collapse_merged_header_rows(&raw_rows);
            collapsed
                .into_iter()
                .enumerate()
                .take(col_count.max(many.first().map_or(0, Vec::len)))
                .map(|(col_idx, text)| {
                    let quoted = many.iter().any(|row| row.get(col_idx).is_some_and(|f| f.quoted));
                    ParsedCsvField { text, quoted }
                })
                .collect()
        }
    }
}

/// Collapse multi-row (possibly horizontally merged/blank-spanned) header rows into single column titles.
///
/// For `rows.len() > 1`, non-empty labels in upper header tiers are carried forward across adjacent
/// empty cells up to the last non-empty header column, then non-empty distinct tiers per column are
/// joined with `" - "`.
#[must_use]
pub fn collapse_merged_header_rows(rows: &[Vec<String>]) -> Vec<String> {
    match rows {
        [] => Vec::new(),
        [single] => single.clone(),
        many => {
            let active_cols = many.iter().map(|r| r.iter().rposition(|s| !s.trim().is_empty()).map_or(0, |idx| idx + 1)).max().unwrap_or(0);
            if active_cols == 0 {
                return Vec::new();
            }

            let mut expanded: Vec<Vec<String>> = Vec::with_capacity(many.len());
            for (tier_idx, row) in many.iter().enumerate() {
                let mut tier = Vec::with_capacity(active_cols);
                let mut carry: Option<String> = None;
                let fill_blanks = tier_idx + 1 < many.len();
                for col in 0..active_cols {
                    let cell = row.get(col).map(|s| s.trim()).unwrap_or("");
                    if !cell.is_empty() {
                        carry = Some(cell.to_owned());
                        tier.push(cell.to_owned());
                    } else if fill_blanks
                        && let Some(ref carried) = carry
                        && many.iter().skip(tier_idx + 1).any(|lower| lower.get(col).is_some_and(|s| !s.trim().is_empty()))
                    {
                        tier.push(carried.clone());
                    } else {
                        tier.push(String::new());
                    }
                }
                expanded.push(tier);
            }

            let mut out = Vec::with_capacity(active_cols);
            for col in 0..active_cols {
                let mut parts: Vec<&str> = Vec::with_capacity(expanded.len());
                for tier in &expanded {
                    if let Some(val) = tier.get(col).map(String::as_str)
                        && !val.is_empty()
                        && parts.last().copied() != Some(val)
                    {
                        parts.push(val);
                    }
                }
                out.push(parts.join(" - "));
            }
            out
        }
    }
}

fn populate_datasets(
    data_sets: &mut [DataSetColumn],
    data_rows: &[Vec<ParsedCsvField>],
    headers: &[ParsedCsvField],
    col_offset: usize,
    subs: usize,
    opts: &CsvOptions,
) -> Result<(), FormatError> {
    for (ds_idx, ds) in data_sets.iter_mut().enumerate() {
        let base_col = col_offset + ds_idx * subs;
        if let Some(raw_header) = headers.get(base_col).filter(|s| !s.text.trim().is_empty()) {
            let base =
                if subs > 1 { raw_header.text.rsplit_once(':').map_or(raw_header.text.as_str(), |(lhs, _)| lhs) } else { raw_header.text.as_str() };
            ds.title = if raw_header.quoted {
                validate_title(base)?;
                base.to_owned()
            } else {
                sanitize_title(base)?
            };
        }
        for (r, rec) in data_rows.iter().enumerate() {
            for s in 0..subs {
                if let Some(tok) = rec.get(base_col + s)
                    && let Some(slot) = ds.cell_mut(r, s)
                {
                    *slot = parse_csv_field_cell(tok, opts);
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
/// `decimate_every_k`, `unstack_group_column`, `baseline`, and `parse_asterisk_exclusions`).
///
/// Rejects `filter.exclude_row_predicate` with [`FormatError::InvalidPipeline`] because formula
/// predicate evaluation belongs to L2 `curvebook-transform` (`apply_import_link_pipeline`).
pub fn filter_delimited_text(
    input: &str,
    delimiter: char,
    decimal_separator: char,
    has_header: bool,
    filter: &ImportFilterSpec,
) -> Result<FilteredDelimitedGrid, FormatError> {
    if filter.exclude_row_predicate.is_some() {
        return Err(FormatError::InvalidPipeline {
            reason: String::from(
                "ImportFilterSpec::exclude_row_predicate requires formula evaluation; use curvebook_transform::apply_import_link_pipeline",
            ),
        });
    }
    let opts = CsvOptions {
        delimiter,
        decimal_separator,
        header_rows: usize::from(has_header),
        parse_asterisk_exclusions: filter.parse_asterisk_exclusions,
        ..CsvOptions::csv()
    };
    let (max_cols, max_fields) = csv_default_limits(input);
    let records = parse_delimited_records_with_quotes_and_limits(input, &opts, max_cols, max_fields)?;
    if records.is_empty() {
        return Ok(FilteredDelimitedGrid { headers: Vec::new(), cells: Vec::new() });
    }
    let (raw_headers, data_rows) = if has_header {
        let Some((first_row, rest_rows)) = records.split_first() else {
            return Ok(FilteredDelimitedGrid { headers: Vec::new(), cells: Vec::new() });
        };
        (first_row.iter().map(|f| f.text.clone()).collect(), rest_rows)
    } else {
        (Vec::new(), records.as_slice())
    };

    let (inferred_dec, inferred_thousands) =
        infer_number_separators(data_rows.iter().take(32).flat_map(|row| row.iter().map(|f| f.text.as_str())), delimiter);
    let parse_opts = CsvOptions { thousands_separator: if inferred_dec == decimal_separator { inferred_thousands } else { None }, ..opts };

    let (r_start, r_end) = match filter.row_window {
        Some((s, e)) if e < s => return Err(FormatError::InvalidPipeline { reason: String::from("row_window end < start") }),
        Some((s, e)) => (s.min(data_rows.len()), e.min(data_rows.len().saturating_sub(1))),
        None => (0, data_rows.len().saturating_sub(1)),
    };
    let sliced_rows = if data_rows.is_empty() { &[][..] } else { data_rows.get(r_start..=r_end).unwrap_or(&[]) };

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
        let kept_cols = c_end - c_start + 1;
        let kept_rows = sliced_rows.len().div_ceil(step);
        check_pipeline_cell_budget(kept_rows.checked_mul(kept_cols), "filtered")?;
        for c in c_start..=c_end {
            headers.push(raw_headers.get(c).cloned().unwrap_or_else(|| default_column_label(c)));
        }
        for (idx, row) in sliced_rows.iter().enumerate() {
            if idx % step != 0 {
                continue;
            }
            let mut parsed_row = Vec::with_capacity(kept_cols);
            for c in c_start..=c_end {
                let cell = row.get(c).map_or_else(Cell::empty, |tok| parse_csv_field_cell(tok, &parse_opts));
                parsed_row.push(cell);
            }
            cells.push(parsed_row);
        }
    }

    let mut out = if let Some(group_col_raw) = filter.unstack_group_column {
        let rel_col = group_col_raw
            .checked_sub(c_start)
            .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("unstack_group_column outside col_window") })?;
        unstack_grid(&headers, &cells, rel_col)?
    } else {
        FilteredDelimitedGrid { headers, cells }
    };

    if let Some((base_col, op)) = filter.baseline {
        apply_filter_baseline(&mut out.cells, base_col, op)?;
    }

    Ok(out)
}

fn apply_filter_baseline(grid: &mut [Vec<Cell>], base_col: usize, op: BaselineOperation) -> Result<(), FormatError> {
    for row in grid.iter_mut() {
        let Some(base_val) = row.get(base_col).and_then(Cell::active_number) else {
            return Err(FormatError::InvalidPipeline { reason: format!("baseline column {base_col} missing active numeric value") });
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

fn check_pipeline_cell_budget(total_cells: Option<usize>, stage: &str) -> Result<(), FormatError> {
    let total = total_cells.ok_or_else(|| FormatError::InvalidPipeline { reason: format!("{stage} cell count overflow") })?;
    if total > MAX_IMPORT_CELLS {
        return Err(FormatError::InvalidPipeline { reason: format!("{stage} dense cell allocation {total} exceeds limit {MAX_IMPORT_CELLS}") });
    }
    Ok(())
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
    check_pipeline_cell_budget(max_rows.checked_mul(order.len()), "unstacked")?;
    let mut out_cells = vec![vec![Cell::empty(); order.len()]; max_rows];
    for (c_idx, key) in order.iter().enumerate() {
        if let Some(vals) = groups.get(key) {
            for (r_idx, val) in vals.iter().enumerate() {
                if let Some(cell) = out_cells.get_mut(r_idx).and_then(|row| row.get_mut(c_idx)) {
                    *cell = val.clone();
                }
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
                    if let CellValue::Number(v) = cell.value {
                        cell.value = match base_row.get(c).and_then(Cell::active_number) {
                            Some(b) => CellValue::from_finite_or_empty(v - b),
                            None => CellValue::Empty,
                        };
                    }
                }
            }
        }
        ImportBaselineAdjustment::SubtractColumn(base_col) => {
            for row in grid {
                let base_opt = row.get(base_col).and_then(Cell::active_number);
                for cell in row {
                    if let CellValue::Number(v) = cell.value {
                        cell.value = match base_opt {
                            Some(b) => CellValue::from_finite_or_empty(v - b),
                            None => CellValue::Empty,
                        };
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
                    if let CellValue::Number(v) = cell.value {
                        cell.value = match base_row.get(c).and_then(Cell::active_number) {
                            Some(b) if b != 0.0 => CellValue::from_finite_or_empty((v / b) * 100.0),
                            _ => CellValue::Empty,
                        };
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
        curvebook_doc::LinkSource::ExternalDelimitedFile { delimiter, decimal_separator, .. } => (*delimiter, *decimal_separator),
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
    target.ensure_mutable()?;
    target.archetype.validate()?;
    if grid.is_empty() {
        return Ok(SheetCellRange::new(placement.anchor, placement.anchor));
    }
    let grid_rows = grid.len();
    let grid_cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    if grid_cols == 0 {
        return Ok(SheetCellRange::new(placement.anchor, placement.anchor));
    }

    let existing_rows = target.archetype.row_count();
    let start_row = match placement.mode {
        PlacementMode::Overwrite | PlacementMode::InsertRows | PlacementMode::AppendColumns => placement.anchor.row,
        PlacementMode::AppendRows => existing_rows,
    };
    let start_col = match placement.mode {
        PlacementMode::Overwrite | PlacementMode::InsertRows | PlacementMode::AppendRows => placement.anchor.col,
        PlacementMode::AppendColumns => target.archetype.column_count(),
    };
    let start_subcol = placement.anchor.subcol;

    let needed_rows = if placement.mode == PlacementMode::InsertRows {
        existing_rows.max(start_row).checked_add(grid_rows)
    } else {
        start_row.checked_add(grid_rows).map(|n| n.max(existing_rows))
    }
    .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("row offset overflow") })?;
    validate_rows(needed_rows)?;

    preflight_sheet_capacity_and_headers(&target.archetype, needed_rows, start_row, start_col, start_subcol, grid_cols, headers)?;

    ensure_sheet_capacity(target, needed_rows, start_row, start_col, start_subcol, grid_cols)?;
    if placement.mode == PlacementMode::InsertRows && start_row < existing_rows {
        target.shift_rows_down(start_row, grid_rows, existing_rows);
    }
    if let Some(hdrs) = headers {
        apply_headers_to_sheet(target, start_row, start_col, start_subcol, hdrs)?;
    }

    let mut end_coord = CellCoord::new(start_row, start_col, start_subcol);
    for (r_off, row) in grid.iter().enumerate() {
        let r = start_row.checked_add(r_off).ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("row offset overflow") })?;
        for (c_off, cell) in row.iter().enumerate() {
            let (col, subcol) = resolve_target_coord(&target.archetype, start_row, start_col, start_subcol, c_off)?;
            let coord = CellCoord::new(r, col, subcol);
            target.set_cell(coord, cell.clone())?;
            end_coord = coord;
        }
    }
    Ok(SheetCellRange::new(CellCoord::new(start_row, start_col, start_subcol), end_coord))
}

fn xy_x_cells_width(t: &XyTable) -> usize {
    match &t.x_mode {
        XColumnMode::GeneratedSequence { .. } => t.x_cells.first().map_or(0, Vec::len),
        _ => t.x_mode.subcolumn_count(),
    }
}

fn compute_needed_cols_with_subs(
    start_row: usize,
    start_col: usize,
    start_subcol: usize,
    grid_cols: usize,
    subs: usize,
) -> Result<usize, FormatError> {
    validate_subcolumns(subs)?;
    let total_flat = start_col
        .checked_mul(subs)
        .and_then(|v| v.checked_add(start_subcol))
        .and_then(|v| v.checked_add(grid_cols))
        .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("column offset overflow") })?;
    if start_subcol >= subs {
        return Err(FormatError::Doc(DocError::CellOutOfBounds { row: start_row, col: start_col, subcol: start_subcol }));
    }
    Ok(total_flat.div_ceil(subs))
}

fn resolve_nested_coord(
    groups: &[NestedGroupColumn],
    start_row: usize,
    start_col: usize,
    start_subcol: usize,
    flat_offset: usize,
) -> Result<(usize, usize), FormatError> {
    let Some(first_group) = groups.get(start_col) else {
        if start_subcol > 0 {
            return Err(FormatError::Doc(DocError::CellOutOfBounds { row: start_row, col: start_col, subcol: start_subcol }));
        }
        let col = start_col
            .checked_add(flat_offset)
            .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("target cell coordinate overflow") })?;
        return Ok((col, 0));
    };
    let first_units = first_group.biological_unit_count();
    validate_subcolumns(first_units)?;
    if start_subcol >= first_units {
        return Err(FormatError::Doc(DocError::CellOutOfBounds { row: start_row, col: start_col, subcol: start_subcol }));
    }
    let mut col = start_col;
    let mut subcol = start_subcol;
    let mut remaining = flat_offset;
    while let Some(group) = groups.get(col) {
        let units = group.biological_unit_count();
        validate_subcolumns(units)?;
        let avail = units.saturating_sub(subcol);
        if remaining < avail {
            return Ok((col, subcol + remaining));
        }
        remaining -= avail;
        col += 1;
        subcol = 0;
    }
    let final_col =
        col.checked_add(remaining).ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("target cell coordinate overflow") })?;
    Ok((final_col, 0))
}

fn compute_nested_needed_cols(
    groups: &[NestedGroupColumn],
    start_row: usize,
    start_col: usize,
    start_subcol: usize,
    grid_cols: usize,
) -> Result<usize, FormatError> {
    if grid_cols == 0 {
        return Ok(groups.len());
    }
    let _ = start_subcol.checked_add(grid_cols).ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("column offset overflow") })?;
    let (last_col, _) = resolve_nested_coord(groups, start_row, start_col, start_subcol, grid_cols - 1)?;
    let needed_after_last = last_col.checked_add(1).ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("column offset overflow") })?;
    Ok(groups.len().max(needed_after_last))
}

fn preflight_dataset_growth(data_sets: &[DataSetColumn], needed_cols: usize, needed_rows: usize, subs: usize) -> Result<(), FormatError> {
    validate_subcolumns(subs)?;
    if needed_cols > data_sets.len() {
        validate_cell_grid(needed_rows, 1, subs, 0)?;
    }
    for ds in data_sets {
        validate_cell_grid(needed_rows, 1, ds.subcolumn_count().max(subs), 0)?;
    }
    Ok(())
}

fn preflight_sheet_capacity_and_headers(
    arch: &TableArchetype,
    needed_rows: usize,
    start_row: usize,
    start_col: usize,
    start_subcol: usize,
    grid_cols: usize,
    headers: Option<&[String]>,
) -> Result<(), FormatError> {
    match arch {
        TableArchetype::Xy(t) => {
            t.x_mode.validate()?;
            t.y_subcolumn_mode.validate()?;
            let subs = t.y_subcolumn_mode.subcolumn_count();
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, subs)?;
            validate_data_sets(needed_cols)?;
            let x_width = xy_x_cells_width(t);
            let x_cells = t.x_cells.len().max(needed_rows).checked_mul(x_width);
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, subs, x_cells)?;
            preflight_dataset_growth(&t.data_sets, needed_cols, needed_rows, subs)?;
        }
        TableArchetype::Column(t) => {
            t.subcolumn_mode.validate()?;
            let subs = t.subcolumn_mode.subcolumn_count();
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, subs)?;
            validate_data_sets(needed_cols)?;
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, subs, Some(0))?;
            preflight_dataset_growth(&t.data_sets, needed_cols, needed_rows, subs)?;
        }
        TableArchetype::Grouped(t) => {
            t.subcolumn_mode.validate()?;
            let subs = t.subcolumn_mode.subcolumn_count();
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, subs)?;
            validate_data_sets(needed_cols)?;
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, subs, Some(0))?;
            preflight_dataset_growth(&t.data_sets, needed_cols, needed_rows, subs)?;
        }
        TableArchetype::Contingency(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, 1, Some(0))?;
            preflight_dataset_growth(&t.data_sets, needed_cols, needed_rows, 1)?;
        }
        TableArchetype::Survival(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            let time_cells = t.times.len().max(needed_rows);
            check_datasets_growth_budget(&t.cohorts, needed_cols, needed_rows, 1, Some(time_cells))?;
            preflight_dataset_growth(&t.cohorts, needed_cols, needed_rows, 1)?;
        }
        TableArchetype::PartsOfWhole(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, 1, Some(0))?;
            preflight_dataset_growth(&t.data_sets, needed_cols, needed_rows, 1)?;
        }
        TableArchetype::MultipleVariables(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            check_multivar_growth_budget(&t.variables, needed_cols, needed_rows)?;
            for idx in t.variables.len()..needed_cols {
                validate_title(&format!("Variable {}", default_column_label(idx)))?;
            }
        }
        TableArchetype::Nested(t) => {
            let needed_cols = compute_nested_needed_cols(&t.groups, start_row, start_col, start_subcol, grid_cols)?;
            validate_data_sets(needed_cols)?;
            check_nested_growth_budget(&t.groups, needed_cols, needed_rows)?;
            for g in &t.groups {
                let units = g.biological_unit_count();
                validate_subcolumns(units)?;
                validate_cell_grid(needed_rows, 1, units, 0)?;
            }
            if needed_cols > t.groups.len() {
                validate_cell_grid(needed_rows, 1, 1, 0)?;
            }
        }
    }

    if let Some(hdrs) = headers {
        for (idx, title) in hdrs.iter().enumerate() {
            let (_, subcol) = resolve_target_coord(arch, start_row, start_col, start_subcol, idx)?;
            if subcol == 0 && !title.trim().is_empty() {
                validate_title(title)?;
            }
        }
    }
    Ok(())
}

fn check_datasets_growth_budget(
    data_sets: &[DataSetColumn],
    needed_cols: usize,
    needed_rows: usize,
    subs: usize,
    extra_cells: Option<usize>,
) -> Result<(), FormatError> {
    let mut total = extra_cells.ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("sheet cell capacity overflow") })?;
    for ds in data_sets {
        let rows = ds.row_count().max(needed_rows);
        let ds_subs = ds.subcolumn_count().max(subs);
        let col_cells = rows.checked_mul(ds_subs);
        total = col_cells
            .and_then(|c| total.checked_add(c))
            .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("sheet cell capacity overflow") })?;
        check_pipeline_cell_budget(Some(total), "target sheet")?;
    }
    let extra_cols = needed_cols.saturating_sub(data_sets.len());
    if extra_cols > 0 {
        let added = extra_cols.checked_mul(needed_rows).and_then(|v| v.checked_mul(subs));
        total = added
            .and_then(|c| total.checked_add(c))
            .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("sheet cell capacity overflow") })?;
        check_pipeline_cell_budget(Some(total), "target sheet")?;
    }
    Ok(())
}

fn check_multivar_growth_budget(variables: &[VariableColumn], needed_cols: usize, needed_rows: usize) -> Result<(), FormatError> {
    let mut total = 0_usize;
    for var in variables {
        let rows = var.cells.len().max(needed_rows);
        total = total.checked_add(rows).ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("sheet cell capacity overflow") })?;
        check_pipeline_cell_budget(Some(total), "target sheet")?;
    }
    let extra_cols = needed_cols.saturating_sub(variables.len());
    let added = extra_cols.checked_mul(needed_rows);
    let combined = added.and_then(|c| total.checked_add(c));
    check_pipeline_cell_budget(combined, "target sheet")
}

fn check_nested_growth_budget(groups: &[NestedGroupColumn], needed_cols: usize, needed_rows: usize) -> Result<(), FormatError> {
    let mut total = 0_usize;
    for g in groups {
        let rows = g.cells.len().max(needed_rows);
        let grp_cells = rows.checked_mul(g.biological_unit_count());
        total = grp_cells
            .and_then(|c| total.checked_add(c))
            .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("sheet cell capacity overflow") })?;
        check_pipeline_cell_budget(Some(total), "target sheet")?;
    }
    let extra_cols = needed_cols.saturating_sub(groups.len());
    let added = extra_cols.checked_mul(needed_rows);
    let combined = added.and_then(|c| total.checked_add(c));
    check_pipeline_cell_budget(combined, "target sheet")
}

fn ensure_sheet_capacity(
    target: &mut DataSheet,
    needed_rows: usize,
    start_row: usize,
    start_col: usize,
    start_subcol: usize,
    grid_cols: usize,
) -> Result<(), FormatError> {
    match &mut target.archetype {
        TableArchetype::Xy(t) => {
            t.x_mode.validate()?;
            t.y_subcolumn_mode.validate()?;
            let subs = t.y_subcolumn_mode.subcolumn_count();
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, subs)?;
            validate_data_sets(needed_cols)?;
            let x_width = xy_x_cells_width(t);
            let x_cells = t.x_cells.len().max(needed_rows).checked_mul(x_width);
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, subs, x_cells)?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, subs)?;
            if t.x_cells.len() < needed_rows {
                t.x_cells.resize_with(needed_rows, || vec![Cell::empty(); x_width]);
            }
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::Column(t) => {
            t.subcolumn_mode.validate()?;
            let subs = t.subcolumn_mode.subcolumn_count();
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, subs)?;
            validate_data_sets(needed_cols)?;
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, subs, Some(0))?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, subs)?;
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::Grouped(t) => {
            t.subcolumn_mode.validate()?;
            let subs = t.subcolumn_mode.subcolumn_count();
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, subs)?;
            validate_data_sets(needed_cols)?;
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, subs, Some(0))?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, subs)?;
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
            if let Some(spacing) = &mut t.row_numeric_spacing
                && spacing.len() < needed_rows
            {
                spacing.resize(needed_rows, None);
            }
        }
        TableArchetype::Contingency(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, 1, Some(0))?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, 1)?;
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::Survival(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            let time_cells = t.times.len().max(needed_rows);
            check_datasets_growth_budget(&t.cohorts, needed_cols, needed_rows, 1, Some(time_cells))?;
            grow_datasets(&mut t.cohorts, needed_cols, needed_rows, 1)?;
            if t.times.len() < needed_rows {
                t.times.resize(needed_rows, Cell::empty());
            }
            if t.row_titles.len() < needed_rows {
                t.row_titles.resize(needed_rows, String::new());
            }
        }
        TableArchetype::PartsOfWhole(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, 1, Some(0))?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, 1)?;
            if t.slice_labels.len() < needed_rows {
                t.slice_labels.resize(needed_rows, String::new());
            }
        }
        TableArchetype::MultipleVariables(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_row, start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            check_multivar_growth_budget(&t.variables, needed_cols, needed_rows)?;
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
            let needed_cols = compute_nested_needed_cols(&t.groups, start_row, start_col, start_subcol, grid_cols)?;
            validate_data_sets(needed_cols)?;
            check_nested_growth_budget(&t.groups, needed_cols, needed_rows)?;
            while t.groups.len() < needed_cols {
                let idx = t.groups.len();
                t.groups.push(NestedGroupColumn::new(default_column_label(idx), 1, needed_rows)?);
            }
            for g in &mut t.groups {
                let units = g.biological_unit_count();
                validate_subcolumns(units)?;
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

fn resolve_target_coord(
    arch: &TableArchetype,
    start_row: usize,
    start_col: usize,
    start_subcol: usize,
    flat_offset: usize,
) -> Result<(usize, usize), FormatError> {
    let subs = match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) => 1,
        TableArchetype::Nested(t) => {
            return resolve_nested_coord(&t.groups, start_row, start_col, start_subcol, flat_offset);
        }
    };
    validate_subcolumns(subs)?;
    let flat = start_col
        .checked_mul(subs)
        .and_then(|v| v.checked_add(start_subcol))
        .and_then(|v| v.checked_add(flat_offset))
        .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("target cell coordinate overflow") })?;
    if start_subcol >= subs {
        return Err(FormatError::Doc(DocError::CellOutOfBounds { row: start_row, col: start_col, subcol: start_subcol }));
    }
    Ok((flat / subs, flat % subs))
}

fn apply_headers_to_sheet(
    target: &mut DataSheet,
    start_row: usize,
    start_col: usize,
    start_subcol: usize,
    headers: &[String],
) -> Result<(), FormatError> {
    for (idx, title) in headers.iter().enumerate() {
        let (col, subcol) = resolve_target_coord(&target.archetype, start_row, start_col, start_subcol, idx)?;
        if subcol != 0 || title.trim().is_empty() {
            continue;
        }
        validate_title(title)?;
        let slot = match &mut target.archetype {
            TableArchetype::Xy(t) => t.data_sets.get_mut(col).map(|ds| &mut ds.title),
            TableArchetype::Column(t) => t.data_sets.get_mut(col).map(|ds| &mut ds.title),
            TableArchetype::Grouped(t) => t.data_sets.get_mut(col).map(|ds| &mut ds.title),
            TableArchetype::Contingency(t) => t.data_sets.get_mut(col).map(|ds| &mut ds.title),
            TableArchetype::Survival(t) => t.cohorts.get_mut(col).map(|ds| &mut ds.title),
            TableArchetype::PartsOfWhole(t) => t.data_sets.get_mut(col).map(|ds| &mut ds.title),
            TableArchetype::MultipleVariables(t) => t.variables.get_mut(col).map(|var| &mut var.title),
            TableArchetype::Nested(t) => t.groups.get_mut(col).map(|g| &mut g.title),
        };
        if let Some(target_title) = slot {
            *target_title = title.trim().to_owned();
        }
    }
    Ok(())
}

#[cfg(test)]
mod dialect_tests {
    use curvebook_doc::Cell;

    use super::{
        CsvOptions, parse_csv_field_cell, parse_delimited_records, parse_delimited_records_with_limits,
        parse_delimited_records_with_quotes_and_limits,
    };
    use crate::error::FormatError;

    #[test]
    fn format_dialect_preserves_quoted_numeric_as_text_blank_lines_and_unicode_quotes() {
        let mut opts = CsvOptions::csv();
        opts.delimiter = '§';
        opts.quote_char = '«';
        let input = "\u{FEFF}«42.5«§42.5\r\n\r\n«a««b\nc«§««";
        let rows = parse_delimited_records_with_quotes_and_limits(input, &opts, 16, 64).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(parse_csv_field_cell(&rows[0][0], &opts), Cell::text("42.5"));
        assert_eq!(parse_csv_field_cell(&rows[0][1], &opts), Cell::number(42.5));
        // Standalone blank line is preserved as a single empty unquoted field in format::csv.
        assert_eq!(rows[1].len(), 1);
        assert_eq!(rows[1][0].text, "");
        assert!(!rows[1][0].quoted);
        assert_eq!(rows[2][0].text, "a«b\nc");
        assert!(rows[2][0].quoted);
        assert_eq!(parse_csv_field_cell(&rows[2][1], &opts), Cell::text(""));
    }

    #[test]
    fn format_dialect_enforces_limits_and_reports_line_numbers() {
        let opts = CsvOptions::csv();
        assert!(matches!(parse_delimited_records("a,b\nc\"d,e", &opts), Err(FormatError::CsvParse { line: 2, .. })));
        assert!(matches!(parse_delimited_records("\"a\nb\"z,1", &opts), Err(FormatError::CsvParse { line: 2, .. })));
        assert!(matches!(parse_delimited_records("1,2\n\"unclosed", &opts), Err(FormatError::CsvParse { line: 2, .. })));
        assert!(matches!(parse_delimited_records_with_limits("1,2,3", &opts, 2, 100), Err(FormatError::CsvParse { line: 1, .. })));
        assert!(matches!(parse_delimited_records_with_limits("1,2\n3,4", &opts, 10, 3), Err(FormatError::CsvParse { line: 2, .. })));
    }
}
