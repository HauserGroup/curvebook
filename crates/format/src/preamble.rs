//! Shared metadata-preamble extraction for skipped leading rows in `.xlsx` and delimited text
//! imports (`R17.7`).

use curvebook_doc::{ConstantValue, InfoSheet, SheetId, parse_import_number, validate_title};

use crate::error::FormatError;

/// Extracted `Key: Value` constants and free-text markdown notes from skipped leading rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreambleInfo {
    /// Ordered `(key, ConstantValue)` metadata constants parsed from single-cell `Key: Value` lines.
    pub constants: Vec<(String, ConstantValue)>,
    /// Free-text non-KV preamble lines joined with newlines for [`InfoSheet::notes_markdown`].
    pub notes_markdown: String,
}

impl PreambleInfo {
    /// Return `true` when neither constants nor notes were extracted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.constants.is_empty() && self.notes_markdown.trim().is_empty()
    }
}

/// Parse a raw metadata value token into a typed [`ConstantValue`] honoring `decimal_separator`.
#[must_use]
pub fn parse_preamble_constant_value(raw: &str, decimal_separator: char) -> ConstantValue {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ConstantValue::Empty;
    }
    if let Some(num) = parse_import_number(trimmed, decimal_separator, None)
        && num.is_finite()
    {
        return ConstantValue::Number(num);
    }
    ConstantValue::Text(trimmed.to_owned())
}

/// Attempt to parse a single preamble line as `(key, value)` (`Key: Value` or `Key = Value`),
/// stripping optional leading `#` / `//` comment prefixes.
///
/// Trailing empty cells are ignored by callers before calling this helper so comma-padded lines
/// such as `User: USER,,,,...` extract cleanly. Rejects keys containing path/URL separators,
/// leading digits/signs, or exceeding 64 characters.
#[must_use]
pub fn try_parse_preamble_kv(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let body = trimmed.trim_start_matches('#').trim_start_matches('/').trim();
    if body.contains("://") {
        return None;
    }
    let sep_pos = match (body.find(':'), body.find('=')) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }?;
    let key = body.get(..sep_pos)?.trim();
    let val = body.get(sep_pos + 1..)?.trim();
    if key.is_empty() || val.is_empty() || key.len() > 64 {
        return None;
    }
    let first = key.chars().next()?;
    if key.contains('/') || key.contains('\\') || first.is_ascii_digit() || first == '-' || first == '+' || !first.is_alphanumeric() {
        return None;
    }
    Some((key.to_owned(), val.to_owned()))
}

/// Extract structured `Key: Value` constants and plain-text notes from skipped leading rows.
///
/// Inspects rows where at most one non-empty cell is present (ignoring trailing empty columns
/// produced by spreadsheet/CSV exports padded to full table width), or two-cell `(Key, Value)`
/// rows where column 0 ends with `:` or `=` and column 1 holds the value. Multi-column non-metadata
/// rows are preserved verbatim as tab-joined lines in `notes_markdown`.
#[must_use]
pub fn extract_preamble_info(rows: &[Vec<String>], decimal_separator: char) -> PreambleInfo {
    let mut constants: Vec<(String, ConstantValue)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    for row in rows {
        let non_empty: Vec<&str> = row.iter().map(String::as_str).map(str::trim).filter(|s| !s.is_empty()).collect();
        if non_empty.is_empty() {
            continue;
        }
        if non_empty.len() == 1 {
            let line = non_empty.first().copied().unwrap_or("");
            if let Some((key, val)) = try_parse_preamble_kv(line) {
                let parsed_val = parse_preamble_constant_value(&val, decimal_separator);
                upsert_preamble_constant(&mut constants, key, parsed_val);
            } else {
                notes.push(line.to_owned());
            }
        } else if non_empty.len() == 2 {
            let first = non_empty.first().copied().unwrap_or("");
            let second = non_empty.get(1).copied().unwrap_or("");
            let stripped_first = first.trim_start_matches('#').trim_start_matches('/').trim();
            let cleaned_key = stripped_first.trim_end_matches([':', '=']).trim();
            let first_ch = cleaned_key.chars().next();
            if (stripped_first.ends_with(':') || stripped_first.ends_with('='))
                && !cleaned_key.is_empty()
                && cleaned_key.len() <= 64
                && !cleaned_key.contains('/')
                && !cleaned_key.contains('\\')
                && first_ch.is_some_and(|ch| !ch.is_ascii_digit() && ch.is_alphanumeric())
            {
                let parsed_val = parse_preamble_constant_value(second, decimal_separator);
                upsert_preamble_constant(&mut constants, cleaned_key.to_owned(), parsed_val);
            } else {
                notes.push(non_empty.join("\t"));
            }
        } else {
            notes.push(non_empty.join("\t"));
        }
    }

    PreambleInfo { constants, notes_markdown: notes.join("\n") }
}

fn upsert_preamble_constant(constants: &mut Vec<(String, ConstantValue)>, key: String, value: ConstantValue) {
    if let Some(existing) = constants.iter_mut().find(|(k, _)| k == &key) {
        existing.1 = value;
    } else {
        constants.push((key, value));
    }
}

/// Construct one shared [`InfoSheet`] populated with extracted preamble constants and
/// `notes_markdown`, linked to `linked_data_sheets`. Returns `Ok(None)` when `preamble` is empty.
pub fn build_preamble_info_sheet(
    id: impl Into<SheetId>,
    title: impl Into<String>,
    preamble: &PreambleInfo,
    linked_data_sheets: Vec<SheetId>,
) -> Result<Option<InfoSheet>, FormatError> {
    if preamble.is_empty() {
        return Ok(None);
    }
    let title_str = title.into();
    validate_title(&title_str)?;
    let mut info = InfoSheet::new_preseeded(id, title_str)?;
    info.linked_data_sheets = linked_data_sheets;
    for (key, val) in &preamble.constants {
        if validate_title(key).is_ok() {
            let _ = info.set_constant(key.clone(), val.clone(), None);
        }
    }
    if !preamble.notes_markdown.trim().is_empty() {
        info.notes_markdown = preamble.notes_markdown.clone();
    }
    Ok(Some(info))
}
