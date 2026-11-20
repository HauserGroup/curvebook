//! Shared RFC-4180 style delimited field scanner for CSV/TSV file import and clipboard paste.
//!
//! Strips an optional UTF-8 BOM (`\u{FEFF}`), handles `\r\n`, `\n`, and `\r` record breaks,
//! supports arbitrary Unicode delimiters and optional quote characters with doubled-quote
//! escaping, and emits each field together with its quoted bit, record/field indices, 1-based
//! physical line number, and record-end marker.
//!
//! Dialect-specific policies (such as whether quoted numeric tokens remain text, whether
//! standalone blank lines are preserved or skipped, and row/column/cell budgets) live in the
//! calling adapters in `curvebook-format` and `curvebook-transform`.

use std::fmt;
use std::iter::Peekable;
use std::str::Chars;

use crate::cell::{Cell, CellValue};

/// Configuration for [`scan_delimited`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelimitedScanOptions {
    /// Field separator character (for example `','`, `'\t'`, or `';'`).
    pub delimiter: char,
    /// Optional quoting character (typically `Some('"')`). When `None`, quoting is disabled.
    pub quote_char: Option<char>,
}

/// Single field emitted by [`scan_delimited`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelimitedField {
    /// Unescaped field text.
    pub text: String,
    /// `true` if this field was enclosed in the configured quote character.
    pub quoted: bool,
    /// 0-based index of the scanned record containing this field.
    pub record_index: usize,
    /// 0-based index of this field within its record.
    pub field_index: usize,
    /// 1-based physical line number at which this field terminated.
    pub line: usize,
    /// `true` if this field terminated the current record (line break or end of input).
    pub end_of_record: bool,
}

/// Syntax error detected while scanning delimited text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelimitedScanError {
    /// Encountered the configured quote character inside an unquoted field.
    UnexpectedQuoteInUnquotedField {
        /// 1-based physical line number where the unexpected quote occurred.
        line: usize,
    },
    /// Encountered a character other than the delimiter or line break after a closing quote.
    UnexpectedCharAfterClosingQuote {
        /// 1-based physical line number where the unexpected character occurred.
        line: usize,
        /// The unexpected trailing character.
        ch: char,
    },
    /// Reached end-of-input while still inside a quoted field.
    UnclosedQuotedField {
        /// 1-based physical line number at end-of-input.
        line: usize,
    },
}

impl fmt::Display for DelimitedScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedQuoteInUnquotedField { line } => write!(f, "line {line}: unexpected quote inside unquoted field"),
            Self::UnexpectedCharAfterClosingQuote { line, ch } => write!(f, "line {line}: unexpected character `{ch}` after closing quote"),
            Self::UnclosedQuotedField { line } => write!(f, "line {line}: unclosed quoted field at EOF"),
        }
    }
}

impl std::error::Error for DelimitedScanError {}

/// Error returned by [`scan_delimited`], distinguishing scanner syntax errors from caller
/// callback errors (such as column, row, or cell limits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelimitedVisitorError<E> {
    /// Delimited syntax error detected by the scanner.
    Syntax(DelimitedScanError),
    /// Error returned by the caller's field callback.
    Callback(E),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanState {
    FieldStart,
    Unquoted,
    Quoted,
    AfterQuote,
}

struct ScanCursor {
    field: String,
    field_quoted: bool,
    record_index: usize,
    field_index: usize,
    line_no: usize,
}

impl ScanCursor {
    fn new() -> Self {
        Self { field: String::new(), field_quoted: false, record_index: 0, field_index: 0, line_no: 1 }
    }

    fn emit<E, F>(&mut self, on_field: &mut F, end_of_record: bool) -> Result<(), DelimitedVisitorError<E>>
    where
        F: FnMut(DelimitedField) -> Result<(), E>,
    {
        let emitted = DelimitedField {
            text: std::mem::take(&mut self.field),
            quoted: std::mem::replace(&mut self.field_quoted, false),
            record_index: self.record_index,
            field_index: self.field_index,
            line: self.line_no,
            end_of_record,
        };
        on_field(emitted).map_err(DelimitedVisitorError::Callback)
    }

    fn finish_record<E, F>(&mut self, on_field: &mut F) -> Result<(), DelimitedVisitorError<E>>
    where
        F: FnMut(DelimitedField) -> Result<(), E>,
    {
        self.emit(on_field, true)?;
        self.record_index += 1;
        self.field_index = 0;
        self.line_no += 1;
        Ok(())
    }

    fn finish_field<E, F>(&mut self, on_field: &mut F) -> Result<(), DelimitedVisitorError<E>>
    where
        F: FnMut(DelimitedField) -> Result<(), E>,
    {
        self.emit(on_field, false)?;
        self.field_index += 1;
        Ok(())
    }
}

fn step_quoted(ch: char, chars: &mut Peekable<Chars<'_>>, quote_char: Option<char>, cursor: &mut ScanCursor) -> ScanState {
    if quote_char == Some(ch) {
        if chars.peek().copied() == quote_char {
            let _ = chars.next();
            cursor.field.push(ch);
            ScanState::Quoted
        } else {
            ScanState::AfterQuote
        }
    } else {
        if ch == '\n' || (ch == '\r' && chars.peek() != Some(&'\n')) {
            cursor.line_no += 1;
        }
        cursor.field.push(ch);
        ScanState::Quoted
    }
}

/// Scan `input` into delimited fields, invoking `on_field` for every completed field in order.
///
/// Unquoted `\r\n`, `\n`, and `\r` terminate the current record through a single shared path.
/// Newlines inside quoted fields are preserved verbatim while still advancing physical line
/// numbers for error reporting.
///
/// # Errors
/// Returns [`DelimitedVisitorError::Syntax`] on malformed quoting or unclosed quoted fields, or
/// [`DelimitedVisitorError::Callback`] as soon as `on_field` returns `Err`.
pub fn scan_delimited<E, F>(input: &str, opts: DelimitedScanOptions, mut on_field: F) -> Result<(), DelimitedVisitorError<E>>
where
    F: FnMut(DelimitedField) -> Result<(), E>,
{
    let text = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    if text.is_empty() {
        return Ok(());
    }

    let mut cursor = ScanCursor::new();
    let mut state = ScanState::FieldStart;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match state {
            ScanState::Quoted => {
                state = step_quoted(ch, &mut chars, opts.quote_char, &mut cursor);
            }
            ScanState::FieldStart if opts.quote_char == Some(ch) => {
                cursor.field_quoted = true;
                state = ScanState::Quoted;
            }
            ScanState::FieldStart | ScanState::Unquoted | ScanState::AfterQuote if ch == opts.delimiter => {
                cursor.finish_field(&mut on_field)?;
                state = ScanState::FieldStart;
            }
            ScanState::FieldStart | ScanState::Unquoted | ScanState::AfterQuote if ch == '\r' || ch == '\n' => {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                }
                cursor.finish_record(&mut on_field)?;
                state = ScanState::FieldStart;
            }
            ScanState::FieldStart => {
                cursor.field.push(ch);
                state = ScanState::Unquoted;
            }
            ScanState::Unquoted if opts.quote_char == Some(ch) => {
                return Err(DelimitedVisitorError::Syntax(DelimitedScanError::UnexpectedQuoteInUnquotedField { line: cursor.line_no }));
            }
            ScanState::Unquoted => {
                cursor.field.push(ch);
            }
            ScanState::AfterQuote => {
                return Err(DelimitedVisitorError::Syntax(DelimitedScanError::UnexpectedCharAfterClosingQuote { line: cursor.line_no, ch }));
            }
        }
    }

    match state {
        ScanState::Quoted => Err(DelimitedVisitorError::Syntax(DelimitedScanError::UnclosedQuotedField { line: cursor.line_no })),
        ScanState::FieldStart => {
            if cursor.field_index > 0 || !cursor.field.is_empty() {
                cursor.emit(&mut on_field, true)?;
            }
            Ok(())
        }
        ScanState::Unquoted | ScanState::AfterQuote => cursor.emit(&mut on_field, true),
    }
}

/// Format a [`Cell`] as a single tab-separated pasted-grid token for linked-paste freezing and
/// import-preview serialization.
///
/// Embedded tabs and line breaks in text cells are replaced with ASCII spaces, and non-empty
/// excluded cells receive a trailing `*` marker.
#[must_use]
pub fn format_linked_paste_tsv_cell(cell: &Cell) -> String {
    let base = match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(v) => v.to_string(),
        CellValue::Text(s) => s.replace(['\t', '\n', '\r'], " "),
    };
    if cell.excluded && !base.is_empty() { format!("{base}*") } else { base }
}

#[cfg(test)]
mod tests {
    use crate::cell::Cell;

    use super::{DelimitedField, DelimitedScanError, DelimitedScanOptions, DelimitedVisitorError, format_linked_paste_tsv_cell, scan_delimited};

    fn collect_fields(input: &str, opts: DelimitedScanOptions) -> Result<Vec<DelimitedField>, DelimitedScanError> {
        let mut out = Vec::new();
        scan_delimited::<(), _>(input, opts, |f| {
            out.push(f);
            Ok(())
        })
        .map_err(|err| match err {
            DelimitedVisitorError::Syntax(s) => s,
            DelimitedVisitorError::Callback(()) => panic!("infallible callback"),
        })?;
        Ok(out)
    }

    #[test]
    fn scans_bom_crlf_lf_cr_quoted_newlines_and_unicode_quote() {
        let opts = DelimitedScanOptions { delimiter: '§', quote_char: Some('«') };
        let input = "\u{FEFF}«123«§456\r\n«a««b\nc«§\r\n\n««";
        let fields = collect_fields(input, opts).unwrap();
        assert_eq!(
            fields,
            vec![
                DelimitedField { text: "123".into(), quoted: true, record_index: 0, field_index: 0, line: 1, end_of_record: false },
                DelimitedField { text: "456".into(), quoted: false, record_index: 0, field_index: 1, line: 1, end_of_record: true },
                DelimitedField { text: "a«b\nc".into(), quoted: true, record_index: 1, field_index: 0, line: 3, end_of_record: false },
                DelimitedField { text: String::new(), quoted: false, record_index: 1, field_index: 1, line: 3, end_of_record: true },
                DelimitedField { text: String::new(), quoted: false, record_index: 2, field_index: 0, line: 4, end_of_record: true },
                DelimitedField { text: String::new(), quoted: true, record_index: 3, field_index: 0, line: 5, end_of_record: true },
            ]
        );
    }

    #[test]
    fn reports_syntax_errors_with_physical_line_numbers() {
        let opts = DelimitedScanOptions { delimiter: ',', quote_char: Some('"') };
        assert_eq!(collect_fields("a,b\nc\"d,e", opts).unwrap_err(), DelimitedScanError::UnexpectedQuoteInUnquotedField { line: 2 });
        assert_eq!(collect_fields("\"a\nb\"x,1", opts).unwrap_err(), DelimitedScanError::UnexpectedCharAfterClosingQuote { line: 2, ch: 'x' });
        assert_eq!(collect_fields("a,b\n\"unclosed\r\nline", opts).unwrap_err(), DelimitedScanError::UnclosedQuotedField { line: 3 });
    }

    #[test]
    fn stops_immediately_on_callback_error_and_handles_disabled_quotes() {
        let opts = DelimitedScanOptions { delimiter: '"', quote_char: None };
        let fields = collect_fields("1\"2\"3\n4\"5", opts).unwrap();
        assert_eq!(fields.len(), 5);
        assert!(fields.iter().all(|f| !f.quoted));

        let mut seen = 0usize;
        let err = scan_delimited("a,b,c,d", DelimitedScanOptions { delimiter: ',', quote_char: Some('"') }, |_| {
            seen += 1;
            if seen == 2 { Err("limit") } else { Ok(()) }
        })
        .unwrap_err();
        assert_eq!(err, DelimitedVisitorError::Callback("limit"));
        assert_eq!(seen, 2);
    }

    #[test]
    fn formats_linked_paste_tsv_cells_with_whitespace_sanitization_and_exclusion_marker() {
        assert_eq!(format_linked_paste_tsv_cell(&Cell::empty()), "");
        assert_eq!(format_linked_paste_tsv_cell(&Cell { value: crate::cell::CellValue::Empty, excluded: true }), "");
        assert_eq!(format_linked_paste_tsv_cell(&Cell::number(12.5)), "12.5");
        assert_eq!(format_linked_paste_tsv_cell(&Cell { value: crate::cell::CellValue::Number(12.5), excluded: true }), "12.5*");
        assert_eq!(format_linked_paste_tsv_cell(&Cell::text("a\tb\nc\rd")), "a b c d");
        assert_eq!(format_linked_paste_tsv_cell(&Cell { value: crate::cell::CellValue::Text(String::from("ctrl\t1")), excluded: true }), "ctrl 1*");
    }
}
