//! External ID-indexed `.prism` JSON models, whole-segment archive path resolution,
//! polymorphic rich-text conversion, strict ISO-8601 dates, and color parsing.

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::MAX_ROWS;
use curvebook_palette::Rgba8;

use crate::zip::ZipIndex;

/// Maximum expanded row indices accepted from a single `cellAttributes[].rows` or
/// `replicate ranges[].range` token so adversarial ranges cannot cause unbounded allocation.
const MAX_EXPANDED_RANGE_ITEMS: usize = MAX_ROWS;

/// Routing dialect detected from `document.json` inside a `.prism` archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalDocumentDialect {
    /// CurveBook-native embedded `document.json` carrying `data_sheets`.
    NativeEmbedded,
    /// External ID-indexed `.prism` carrying `sheets.{data,analyses,...}` and/or `sheetAttributesMap`.
    ExternalIndexed,
    /// Legacy generic manifest / plain CSV container without an external index.
    GenericManifestOrCsv,
}

/// Outcome of resolving an external sheet, table, or dataset UID against archive paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdLookup {
    /// Exactly one archive path matched the UID on a whole path segment or file stem.
    Found(String),
    /// Zero archive paths matched the UID.
    Missing,
    /// Multiple distinct archive paths matched the UID; callers must skip rather than guess.
    Ambiguous(Vec<String>),
}

/// Whole-segment and whole-stem archive path index built from [`ZipIndex::entry_names`] without
/// decompressing any payloads.
#[derive(Debug, Default)]
pub struct ArchiveIdIndex {
    paths: Vec<String>,
    by_segment: BTreeMap<String, Vec<usize>>,
    by_stem: BTreeMap<String, Vec<usize>>,
}

impl ArchiveIdIndex {
    /// Index all non-`__MACOSX/` normalized entry paths from `zip_index`.
    #[must_use]
    pub fn from_zip_index(zip_index: &ZipIndex<'_>) -> Self {
        let mut paths = Vec::new();
        let mut by_segment: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut by_stem: BTreeMap<String, Vec<usize>> = BTreeMap::new();

        for raw_path in zip_index.entry_names() {
            if raw_path.starts_with("__MACOSX/") {
                continue;
            }
            let idx = paths.len();
            paths.push(raw_path.to_owned());

            let mut segs_seen = BTreeSet::new();
            for seg in raw_path.split('/') {
                if !seg.is_empty() && segs_seen.insert(seg) {
                    by_segment.entry(seg.to_owned()).or_default().push(idx);
                }
            }
            if let Some(base) = raw_path.rsplit('/').next() {
                let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
                if !stem.is_empty() {
                    by_stem.entry(stem.to_owned()).or_default().push(idx);
                }
            }
        }
        Self { paths, by_segment, by_stem }
    }

    fn resolve_from_indices(&self, indices: &[usize], predicate: impl Fn(&str) -> bool) -> IdLookup {
        let mut matches = Vec::new();
        for &idx in indices {
            if let Some(p) = self.paths.get(idx)
                && predicate(p)
            {
                matches.push(p.clone());
            }
        }
        match matches.len() {
            0 => IdLookup::Missing,
            1 => IdLookup::Found(matches.into_iter().next().unwrap_or_default()),
            _ => IdLookup::Ambiguous(matches),
        }
    }

    /// Resolve a data sheet's `sheet.json` matching `sheet_uid` as a whole path segment.
    #[must_use]
    pub fn find_data_sheet_json(&self, sheet_uid: &str) -> IdLookup {
        let Some(indices) = self.by_segment.get(sheet_uid) else {
            return IdLookup::Missing;
        };
        let preferred =
            self.resolve_from_indices(indices, |p| has_basename_ci(p, "sheet.json") && has_segment_ci(p, "data") && has_segment_ci(p, "sheets"));
        if !matches!(preferred, IdLookup::Missing) {
            return preferred;
        }
        self.resolve_from_indices(indices, |p| {
            has_basename_ci(p, "sheet.json")
                && !has_segment_ci(p, "analyses")
                && !has_segment_ci(p, "graphs")
                && !has_segment_ci(p, "info")
                && !has_segment_ci(p, "layouts")
        })
    }

    /// Resolve a table's headerless `data.csv` matching `table_uid` as a whole path segment.
    #[must_use]
    pub fn find_table_csv(&self, table_uid: &str) -> IdLookup {
        let Some(indices) = self.by_segment.get(table_uid) else {
            return IdLookup::Missing;
        };
        let preferred = self.resolve_from_indices(indices, |p| has_basename_ci(p, "data.csv") && has_segment_ci(p, "tables"));
        if !matches!(preferred, IdLookup::Missing) {
            return preferred;
        }
        self.resolve_from_indices(indices, |p| has_basename_ci(p, "data.csv"))
    }

    /// Resolve a table's `content.json` matching `table_uid` as a whole path segment.
    #[must_use]
    pub fn find_table_content_json(&self, table_uid: &str) -> IdLookup {
        let Some(indices) = self.by_segment.get(table_uid) else {
            return IdLookup::Missing;
        };
        self.resolve_from_indices(indices, |p| has_basename_ci(p, "content.json"))
    }

    /// Resolve a dataset JSON (`data/sets/<set_uid>.json`) matching `set_uid` as an exact file stem.
    #[must_use]
    pub fn find_dataset_json(&self, set_uid: &str) -> IdLookup {
        let Some(indices) = self.by_stem.get(set_uid) else {
            return IdLookup::Missing;
        };
        let preferred = self.resolve_from_indices(indices, |p| has_ext_ci(p, ".json") && has_segment_ci(p, "sets"));
        if !matches!(preferred, IdLookup::Missing) {
            return preferred;
        }
        self.resolve_from_indices(indices, |p| has_ext_ci(p, ".json"))
    }

    /// Resolve an info sheet's `sheet.json` matching `info_uid` as a whole path segment.
    #[must_use]
    pub fn find_info_sheet_json(&self, info_uid: &str) -> IdLookup {
        let Some(indices) = self.by_segment.get(info_uid) else {
            return IdLookup::Missing;
        };
        let preferred = self.resolve_from_indices(indices, |p| has_basename_ci(p, "sheet.json") && has_segment_ci(p, "info"));
        if !matches!(preferred, IdLookup::Missing) {
            return preferred;
        }
        self.resolve_from_indices(indices, |p| has_basename_ci(p, "sheet.json"))
    }

    /// Resolve a named file (`sheet.json`, `parameters.json`, `results.json`, `constants.json`)
    /// inside an analysis directory matching `analysis_uid` as a whole path segment.
    #[must_use]
    pub fn find_analysis_entry(&self, analysis_uid: &str, filename: &str) -> IdLookup {
        let Some(indices) = self.by_segment.get(analysis_uid) else {
            return IdLookup::Missing;
        };
        let preferred = self.resolve_from_indices(indices, |p| has_basename_ci(p, filename) && has_segment_ci(p, "analyses"));
        if !matches!(preferred, IdLookup::Missing) {
            return preferred;
        }
        self.resolve_from_indices(indices, |p| has_basename_ci(p, filename))
    }

    /// Resolve an analysis result-sheet JSON (`analyses/<aUid>/result_sheets/<rsUid>.json`)
    /// matching `rs_uid` as an exact file stem.
    #[must_use]
    pub fn find_analysis_result_sheet_json(&self, analysis_uid: &str, rs_uid: &str) -> IdLookup {
        let Some(indices) = self.by_stem.get(rs_uid) else {
            return IdLookup::Missing;
        };
        let scoped =
            self.resolve_from_indices(indices, |p| has_ext_ci(p, ".json") && has_segment_ci(p, analysis_uid) && has_segment_ci(p, "result_sheets"));
        if !matches!(scoped, IdLookup::Missing) {
            return scoped;
        }
        let by_analysis = self.resolve_from_indices(indices, |p| has_ext_ci(p, ".json") && has_segment_ci(p, analysis_uid));
        if !matches!(by_analysis, IdLookup::Missing) {
            return by_analysis;
        }
        self.resolve_from_indices(indices, |p| has_ext_ci(p, ".json") && has_segment_ci(p, "result_sheets"))
    }

    /// Resolve an entry (`sheet.json`, `preview.png`, `data.bin`) under `graphs/<graph_uid>/`.
    #[must_use]
    pub fn find_graph_entry(&self, graph_uid: &str, filename: &str) -> IdLookup {
        let Some(indices) = self.by_segment.get(graph_uid) else {
            return IdLookup::Missing;
        };
        let preferred = self.resolve_from_indices(indices, |p| has_basename_ci(p, filename) && has_segment_ci(p, "graphs"));
        if !matches!(preferred, IdLookup::Missing) {
            return preferred;
        }
        self.resolve_from_indices(indices, |p| has_basename_ci(p, filename))
    }

    /// Resolve a graph floating-note JSON (`graphs/<graph_uid>/floating_notes/<note_uid>.json`)
    /// matching `note_uid` as an exact file stem scoped to `graph_uid`.
    #[must_use]
    pub fn find_graph_floating_note_json(&self, graph_uid: &str, note_uid: &str) -> IdLookup {
        let Some(indices) = self.by_stem.get(note_uid) else {
            return IdLookup::Missing;
        };
        let scoped = self.resolve_from_indices(indices, |p| {
            has_ext_ci(p, ".json") && has_segment_ci(p, "graphs") && has_segment_ci(p, graph_uid) && has_segment_ci(p, "floating_notes")
        });
        if !matches!(scoped, IdLookup::Missing) {
            return scoped;
        }
        self.resolve_from_indices(indices, |p| has_ext_ci(p, ".json") && has_segment_ci(p, "graphs") && has_segment_ci(p, graph_uid))
    }

    /// Resolve an entry (`sheet.json`, `preview.png`, `data.bin`) under `layouts/<layout_uid>/`.
    #[must_use]
    pub fn find_layout_entry(&self, layout_uid: &str, filename: &str) -> IdLookup {
        let Some(indices) = self.by_segment.get(layout_uid) else {
            return IdLookup::Missing;
        };
        let preferred = self.resolve_from_indices(indices, |p| has_basename_ci(p, filename) && has_segment_ci(p, "layouts"));
        if !matches!(preferred, IdLookup::Missing) {
            return preferred;
        }
        self.resolve_from_indices(indices, |p| has_basename_ci(p, filename))
    }

    /// Resolve a media asset matching `media_uid` as an exact file stem under `media/`.
    #[must_use]
    pub fn find_media_entry(&self, media_uid: &str, preferred_ext: Option<&str>) -> IdLookup {
        let Some(indices) = self.by_stem.get(media_uid) else {
            return IdLookup::Missing;
        };
        if let Some(ext) = preferred_ext {
            let dot_ext = format!(".{}", ext.trim_start_matches('.'));
            let exact = self.resolve_from_indices(indices, |p| has_ext_ci(p, &dot_ext) && has_segment_ci(p, "media"));
            if !matches!(exact, IdLookup::Missing) {
                return exact;
            }
            let any_dir = self.resolve_from_indices(indices, |p| has_ext_ci(p, &dot_ext));
            if !matches!(any_dir, IdLookup::Missing) {
                return any_dir;
            }
        }
        self.resolve_from_indices(indices, |p| has_segment_ci(p, "media"))
    }
}

/// Return `true` when `val` matches the external ID-indexed `document.json` shape (`sheets` object
/// with folder arrays or `sheetAttributesMap`).
#[must_use]
pub fn is_external_indexed_document(val: &serde_json::Value) -> bool {
    let Some(obj) = val.as_object() else {
        return false;
    };
    if obj.contains_key("sheetAttributesMap") {
        return true;
    }
    obj.get("sheets").and_then(serde_json::Value::as_object).is_some_and(|sheets| {
        ["data", "analyses", "graphs", "info", "layouts"].iter().any(|k| sheets.get(*k).is_some_and(serde_json::Value::is_array))
    })
}

/// Return `true` when malformed `document.json` bytes or archive paths indicate an external
/// ID-indexed `.prism` archive (which must fail with a typed JSON error rather than falling back
/// to generic CSV import).
#[must_use]
pub fn looks_like_external_indexed_document_bytes(doc_bytes: &[u8], zip_index: &ZipIndex<'_>) -> bool {
    if bytes_contain_subslice(doc_bytes, b"\"sheetAttributesMap\"") {
        return true;
    }
    if bytes_contain_subslice(doc_bytes, b"\"sheets\"")
        && (bytes_contain_subslice(doc_bytes, b"\"data\"") || bytes_contain_subslice(doc_bytes, b"\"analyses\""))
    {
        return true;
    }
    zip_index.entry_names().any(|path| {
        !path.starts_with("__MACOSX/") && has_basename_ci(path, "sheet.json") && (has_segment_ci(path, "sheets") || has_segment_ci(path, "analyses"))
    })
}

fn bytes_contain_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

pub(crate) fn has_basename_ci(path: &str, expected: &str) -> bool {
    path.rsplit('/').next().is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

pub(crate) fn has_segment_ci(path: &str, segment: &str) -> bool {
    path.split('/').any(|s| s.eq_ignore_ascii_case(segment))
}

pub(crate) fn has_ext_ci(path: &str, ext_with_dot: &str) -> bool {
    path.len() >= ext_with_dot.len()
        && path.get(path.len().saturating_sub(ext_with_dot.len())..).is_some_and(|tail| tail.eq_ignore_ascii_case(ext_with_dot))
}

/// Parsed per-sheet attribute overrides from `document.json` `sheetAttributesMap`.
#[derive(Debug, Clone, Default)]
pub struct SheetAttrEntry {
    pub title: Option<String>,
    pub highlight_color: Option<Rgba8>,
    pub is_frozen: bool,
}

/// Parsed external ID-indexed `document.json`.
#[derive(Debug, Clone, Default)]
pub struct ExternalDocumentIndex {
    pub title: Option<String>,
    pub format_version: Option<String>,
    pub data_sheet_ids: Vec<String>,
    pub info_sheet_ids: Vec<String>,
    pub analysis_sheet_ids: Vec<String>,
    pub graph_sheet_ids: Vec<String>,
    pub layout_sheet_ids: Vec<String>,
    pub sheet_attrs: BTreeMap<String, SheetAttrEntry>,
    pub selected_sheet_uid: Option<String>,
}

impl ExternalDocumentIndex {
    /// Extract structured folder lists, sheet attributes, and selection state from `document.json`.
    #[must_use]
    pub fn from_json(val: &serde_json::Value) -> Self {
        let Some(obj) = val.as_object() else {
            return Self::default();
        };
        let title = obj.get("title").and_then(serde_json::Value::as_str).map(str::to_owned);
        let format_version = obj.get("formatVersion").and_then(serde_json::Value::as_str).map(str::to_owned);

        let extract_id_list = |key: &str| -> Vec<String> {
            obj.get("sheets")
                .and_then(|s| s.get(key))
                .and_then(serde_json::Value::as_array)
                .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default()
        };

        let mut sheet_attrs = BTreeMap::new();
        if let Some(map) = obj.get("sheetAttributesMap").and_then(serde_json::Value::as_object) {
            for (uid, raw_entry) in map {
                if let Some(entry_obj) = raw_entry.as_object() {
                    let entry_title = entry_obj.get("title").and_then(serde_json::Value::as_str).map(str::to_owned);
                    let highlight_color = entry_obj.get("highlightColor").and_then(serde_json::Value::as_str).and_then(parse_archive_color);
                    let is_frozen = entry_obj.get("isFrozen").and_then(serde_json::Value::as_bool).unwrap_or(false);
                    sheet_attrs.insert(uid.clone(), SheetAttrEntry { title: entry_title, highlight_color, is_frozen });
                }
            }
        }

        let selected_sheet_uid = extract_selected_sheet_uid(obj);

        Self {
            title,
            format_version,
            data_sheet_ids: extract_id_list("data"),
            info_sheet_ids: extract_id_list("info"),
            analysis_sheet_ids: extract_id_list("analyses"),
            graph_sheet_ids: extract_id_list("graphs"),
            layout_sheet_ids: extract_id_list("layouts"),
            sheet_attrs,
            selected_sheet_uid,
        }
    }
}

fn extract_selected_sheet_uid(doc_obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let ui = doc_obj.get("uiSettings")?.as_object()?;
    let current = ui.get("currentSheets")?;
    if let Some(arr) = current.as_array() {
        for item in arr {
            if let Some(uid) = item.as_str().filter(|s| !s.trim().is_empty()) {
                return Some(uid.to_owned());
            }
        }
    }
    if let Some(map) = current.as_object() {
        let current_type = ui.get("currentSheetType").and_then(serde_json::Value::as_str);
        if let Some(raw_key) = current_type {
            let mapped = match raw_key.to_ascii_lowercase().as_str() {
                "data" => Some("data"),
                "info" => Some("info"),
                "analysis" | "analyses" => Some("analysis"),
                "graph" | "graphs" => Some("graph"),
                "layout" | "layouts" => Some("layout"),
                _ => None,
            };
            for candidate in [Some(raw_key), mapped].into_iter().flatten() {
                if let Some(uid) = map.get(candidate).and_then(serde_json::Value::as_str).filter(|s| !s.trim().is_empty()) {
                    return Some(uid.to_owned());
                }
            }
        }
        for key in ["data", "analysis", "analyses", "info", "graph", "graphs", "layout", "layouts"] {
            if let Some(uid) = map.get(key).and_then(serde_json::Value::as_str).filter(|s| !s.trim().is_empty()) {
                return Some(uid.to_owned());
            }
        }
    }
    None
}

/// Parsed `data/sheets/<uid>/sheet.json` table descriptor.
#[derive(Debug, Clone, Default)]
pub struct ExternalTableJson {
    pub uid: Option<String>,
    pub class_name: Option<String>,
    pub format: Option<String>,
    pub data_format: Option<String>,
    pub x_data_set: Option<String>,
    pub x_data_set_format: Option<String>,
    pub row_titles_data_set: Option<String>,
    pub subcolumn_titles_data_set: Option<String>,
    pub replicates_count: Option<usize>,
    pub data_sets: Vec<String>,
}

/// Parsed `data/sheets/<uid>/sheet.json`.
#[derive(Debug, Clone, Default)]
pub struct ExternalDataSheetJson {
    pub uid: Option<String>,
    pub title: Option<String>,
    pub flags: Vec<String>,
    pub highlight_color: Option<Rgba8>,
    pub date_time_units: Option<String>,
    pub table: ExternalTableJson,
}

impl ExternalDataSheetJson {
    #[must_use]
    pub fn from_json(val: &serde_json::Value) -> Option<Self> {
        let obj = val.as_object()?;
        let uid = obj.get("uid").and_then(serde_json::Value::as_str).map(str::to_owned);
        let title = obj.get("title").map(|v| parse_rich_text_value(v).0).filter(|s| !s.trim().is_empty());
        let flags = obj
            .get("flags")
            .and_then(serde_json::Value::as_array)
            .map(|a| a.iter().filter_map(serde_json::Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        let highlight_color = obj.get("highlightColor").and_then(serde_json::Value::as_str).and_then(parse_archive_color);
        let date_time_units = obj.get("dateTimeConversion").and_then(|d| d.get("units")).and_then(serde_json::Value::as_str).map(str::to_owned);

        let table = if let Some(t) = obj.get("table").and_then(serde_json::Value::as_object) {
            ExternalTableJson {
                uid: t.get("uid").and_then(serde_json::Value::as_str).map(str::to_owned),
                class_name: t.get("@class").and_then(serde_json::Value::as_str).map(str::to_owned),
                format: t.get("format").and_then(serde_json::Value::as_str).map(str::to_owned),
                data_format: t.get("dataFormat").and_then(serde_json::Value::as_str).map(str::to_owned),
                x_data_set: t.get("xDataSet").and_then(serde_json::Value::as_str).map(str::to_owned),
                x_data_set_format: t.get("xDataSetFormat").and_then(serde_json::Value::as_str).map(str::to_owned),
                row_titles_data_set: t.get("rowTitlesDataSet").and_then(serde_json::Value::as_str).map(str::to_owned),
                subcolumn_titles_data_set: t.get("subcolumnTitlesDataSet").and_then(serde_json::Value::as_str).map(str::to_owned),
                replicates_count: t.get("replicatesCount").and_then(serde_json::Value::as_u64).and_then(|n| usize::try_from(n).ok()),
                data_sets: t
                    .get("dataSets")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| a.iter().filter_map(serde_json::Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default(),
            }
        } else {
            ExternalTableJson::default()
        };

        Some(Self { uid, title, flags, highlight_color, date_time_units, table })
    }

    #[must_use]
    pub fn is_analysis_view(&self) -> bool {
        self.table.format.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("view"))
            || self.flags.iter().any(|f| f.eq_ignore_ascii_case("ANALYSIS_VIEW"))
    }
}

/// Parsed per-replicate metadata from `data/sets/<uid>.json`.
#[derive(Debug, Clone, Default)]
pub struct ExternalReplicate {
    pub value_type: Option<String>,
    pub decimals_length: Option<u8>,
    pub start_value: Option<f64>,
    pub interval: Option<f64>,
    pub excluded_rows: BTreeSet<usize>,
    pub censored_rows: BTreeSet<usize>,
}

/// Parsed `data/sets/<uid>.json` (`@class="DataSet"`).
#[derive(Debug, Clone, Default)]
pub struct ExternalDataSetJson {
    pub title_markup: Option<String>,
    pub title_plain: Option<String>,
    pub format: Option<String>,
    pub date_time_units: Option<String>,
    pub replicates: Vec<ExternalReplicate>,
}

impl ExternalDataSetJson {
    #[must_use]
    pub fn from_json(val: &serde_json::Value) -> Option<Self> {
        let obj = val.as_object()?;
        let (title_markup, title_plain) = match obj.get("title") {
            Some(tv) => {
                let (m, p) = parse_rich_text_value(tv);
                let m_opt = (!m.is_empty()).then_some(m);
                let p_opt = (!p.is_empty()).then_some(p);
                (m_opt, p_opt)
            }
            None => (None, None),
        };
        let format = obj.get("format").and_then(serde_json::Value::as_str).map(str::to_owned);
        let date_time_units = obj.get("dateTimeConversion").and_then(|d| d.get("units")).and_then(serde_json::Value::as_str).map(str::to_owned);

        let mut rep_by_idx: BTreeMap<usize, ExternalReplicate> = BTreeMap::new();
        if let Some(arr) = obj.get("replicates").and_then(serde_json::Value::as_array) {
            for (idx, item) in arr.iter().enumerate() {
                if let Some(rep) = parse_external_replicate(item) {
                    rep_by_idx.insert(idx, rep);
                }
            }
        }
        if let Some(ranges) = obj.get("replicate ranges").and_then(serde_json::Value::as_array) {
            for item in ranges {
                let Some(item_obj) = item.as_object() else {
                    continue;
                };
                let Some(range_str) = item_obj.get("range").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let Some(rep_val) = item_obj.get("replicate") else {
                    continue;
                };
                let Some(rep) = parse_external_replicate(rep_val) else {
                    continue;
                };
                for idx in parse_row_index_ranges(range_str, curvebook_doc::MAX_SUBCOLUMNS) {
                    rep_by_idx.insert(idx, rep.clone());
                }
            }
        }

        let replicates = if let Some((&max_idx, _)) = rep_by_idx.last_key_value() {
            let count = max_idx.saturating_add(1).min(curvebook_doc::MAX_SUBCOLUMNS);
            (0..count).map(|i| rep_by_idx.remove(&i).unwrap_or_default()).collect()
        } else {
            Vec::new()
        };

        Some(Self { title_markup, title_plain, format, date_time_units, replicates })
    }
}

fn parse_external_replicate(val: &serde_json::Value) -> Option<ExternalReplicate> {
    let obj = val.as_object()?;
    let value_type = obj.get("valueType").and_then(serde_json::Value::as_str).map(str::to_owned);
    let decimals_length = obj.get("decimalsLength").and_then(serde_json::Value::as_u64).and_then(|d| u8::try_from(d).ok()).filter(|&d| d <= 15);
    let start_value = obj.get("startValue").and_then(serde_json::Value::as_f64).filter(|v| v.is_finite());
    let interval = obj.get("interval").and_then(serde_json::Value::as_f64).filter(|v| v.is_finite());

    let mut excluded_rows = BTreeSet::new();
    let mut censored_rows = BTreeSet::new();
    if let Some(cell_attrs) = obj.get("cellAttributes").and_then(serde_json::Value::as_array) {
        for entry in cell_attrs {
            let Some(eobj) = entry.as_object() else {
                continue;
            };
            let Some(rows_str) = eobj.get("rows").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(attrs) = eobj.get("attributes").and_then(serde_json::Value::as_array) else {
                continue;
            };
            let is_excluded = attrs.iter().any(|a| a.as_str().is_some_and(|s| s.eq_ignore_ascii_case("EXCLUDED")));
            let is_censored = attrs.iter().any(|a| a.as_str().is_some_and(|s| s.eq_ignore_ascii_case("CENSORED")));
            if is_excluded || is_censored {
                for row_idx in parse_row_index_ranges(rows_str, MAX_EXPANDED_RANGE_ITEMS) {
                    if is_excluded {
                        excluded_rows.insert(row_idx);
                    }
                    if is_censored {
                        censored_rows.insert(row_idx);
                    }
                }
            }
        }
    }

    Some(ExternalReplicate { value_type, decimals_length, start_value, interval, excluded_rows, censored_rows })
}

/// Parsed `SubColumnTitlesDataSet` JSON (`data/sets/<uid>.json`).
#[derive(Debug, Clone, Default)]
pub struct ExternalSubColumnTitlesJson {
    pub by_column: BTreeMap<usize, BTreeMap<usize, String>>,
}

impl ExternalSubColumnTitlesJson {
    #[must_use]
    pub fn from_json(val: &serde_json::Value) -> Option<Self> {
        let obj = val.as_object()?;
        let titles = obj.get("titles").and_then(serde_json::Value::as_array)?;
        let mut by_column: BTreeMap<usize, BTreeMap<usize, String>> = BTreeMap::new();
        for col_entry in titles {
            let Some(cobj) = col_entry.as_object() else {
                continue;
            };
            let Some(col_idx) = cobj.get("column").and_then(serde_json::Value::as_u64).and_then(|n| usize::try_from(n).ok()) else {
                continue;
            };
            let Some(reps) = cobj.get("replicates").and_then(serde_json::Value::as_array) else {
                continue;
            };
            let col_map = by_column.entry(col_idx).or_default();
            for rep_entry in reps {
                if let Some(robj) = rep_entry.as_object()
                    && let Some(rep_idx) = robj.get("replicate").and_then(serde_json::Value::as_u64).and_then(|n| usize::try_from(n).ok())
                    && let Some(name_val) = robj.get("name")
                {
                    let (markup, _) = parse_rich_text_value(name_val);
                    col_map.insert(rep_idx, markup);
                }
            }
        }
        Some(Self { by_column })
    }

    /// Resolve subcolumn titles for dataset `col_idx` (falling back to shared column `0` when
    /// `col_idx` has no own titles) padded to `subcol_count`.
    #[must_use]
    pub fn titles_for_column(&self, col_idx: usize, subcol_count: usize) -> Option<Vec<String>> {
        let map = self.by_column.get(&col_idx).or_else(|| self.by_column.get(&0))?;
        if map.is_empty() || subcol_count == 0 {
            return None;
        }
        let mut out = Vec::with_capacity(subcol_count);
        let mut any_non_empty = false;
        for sub in 0..subcol_count {
            let label = map.get(&sub).cloned().unwrap_or_default();
            if !label.trim().is_empty() {
                any_non_empty = true;
            }
            out.push(label);
        }
        any_non_empty.then_some(out)
    }
}

/// Parsed `info/<uid>/sheet.json`.
#[derive(Debug, Clone, Default)]
pub struct ExternalInfoSheetJson {
    pub title: Option<String>,
    pub experiment_date: Option<String>,
    pub notes_text: Option<String>,
    pub constants: Vec<(String, String)>,
    pub linked_data_sheet_id: Option<String>,
    pub highlight_color: Option<Rgba8>,
}

impl ExternalInfoSheetJson {
    #[must_use]
    pub fn from_json(val: &serde_json::Value) -> Option<Self> {
        let obj = val.as_object()?;
        let title = obj.get("title").map(|v| parse_rich_text_value(v).0).filter(|s| !s.trim().is_empty());
        let experiment_date =
            obj.get("experimentDate").and_then(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let notes_text = obj.get("notesText").map(|v| parse_rich_text_value(v).0).filter(|s| !s.is_empty());
        let linked_data_sheet_id =
            obj.get("linkedDataSheetID").and_then(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let highlight_color = obj.get("highlightColor").and_then(serde_json::Value::as_str).and_then(parse_archive_color);

        let mut constants = Vec::new();
        if let Some(arr) = obj.get("constants").and_then(serde_json::Value::as_array) {
            for item in arr {
                let Some(cobj) = item.as_object() else {
                    continue;
                };
                let name = cobj.get("name").map(|v| parse_rich_text_value(v).1).unwrap_or_default();
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let value = cobj.get("value").map(|v| parse_rich_text_value(v).1).unwrap_or_default();
                constants.push((trimmed.to_owned(), value));
            }
        }
        Some(Self { title, experiment_date, notes_text, constants, linked_data_sheet_id, highlight_color })
    }
}

/// Media representation reference inside `graphs/<uid>/sheet.json` or `layouts/<uid>/sheet.json`.
#[derive(Debug, Clone)]
pub struct MediaRepresentationRef {
    pub rep_type: String,
    pub format: String,
    pub uid: String,
}

/// Extract all `mediaCollection[].representations[]` entries from a graph or layout `sheet.json`.
#[must_use]
pub fn extract_media_representations(val: &serde_json::Value) -> Vec<MediaRepresentationRef> {
    let mut out = Vec::new();
    let Some(collection) = val.get("mediaCollection").and_then(serde_json::Value::as_array) else {
        return out;
    };
    for entry in collection {
        let Some(reps) = entry.get("representations").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for rep in reps {
            let Some(robj) = rep.as_object() else {
                continue;
            };
            let Some(uid) = robj.get("uid").and_then(serde_json::Value::as_str).filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            let rep_type = robj.get("type").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
            let format = robj.get("format").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
            out.push(MediaRepresentationRef { rep_type, format, uid: uid.to_owned() });
        }
    }
    out
}

/// Parse external `.prism` 0-based inclusive index range syntax (`"3"`, `"0~4"`, `"1,3~5"`),
/// bounded to at most `max_items` indices to prevent memory exhaustion on adversarial input.
#[must_use]
pub fn parse_row_index_ranges(spec: &str, max_items: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((start_s, end_s)) = trimmed.split_once('~') {
            let Ok(start) = start_s.trim().parse::<usize>() else {
                continue;
            };
            let Ok(end) = end_s.trim().parse::<usize>() else {
                continue;
            };
            if start > end {
                continue;
            }
            let capped_end = end.min(start.saturating_add(max_items.saturating_sub(out.len()).saturating_sub(1)));
            for idx in start..=capped_end {
                if out.len() >= max_items {
                    return out;
                }
                out.push(idx);
            }
        } else if let Ok(single) = trimmed.parse::<usize>() {
            if out.len() >= max_items {
                return out;
            }
            out.push(single);
        }
    }
    out
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RichStyle {
    bold: bool,
    italic: bool,
    superscript: bool,
    subscript: bool,
}

/// Convert a polymorphic `.prism` text value (`string` or `{ string, version, formatting: [...] }`)
/// into `(curvebook_markup, plain_text)`.
#[must_use]
pub fn parse_rich_text_value(val: &serde_json::Value) -> (String, String) {
    if let Some(s) = val.as_str() {
        return (s.to_owned(), s.to_owned());
    }
    let Some(obj) = val.as_object() else {
        return (String::new(), String::new());
    };
    let plain = obj.get("string").and_then(serde_json::Value::as_str).unwrap_or("");
    let Some(formatting) = obj.get("formatting").and_then(serde_json::Value::as_array) else {
        return (plain.to_owned(), plain.to_owned());
    };
    if plain.is_empty() || formatting.is_empty() {
        return (plain.to_owned(), plain.to_owned());
    }

    let chars: Vec<char> = plain.chars().collect();
    let mut styles = vec![RichStyle::default(); chars.len()];
    let mut any_style = false;

    for run in formatting {
        let Some(robj) = run.as_object() else {
            continue;
        };
        let Some(range_str) = robj.get("range").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(attrs) = robj.get("attributes").and_then(serde_json::Value::as_object) else {
            continue;
        };
        let bold = attrs.get("fontWeight").and_then(serde_json::Value::as_str).is_some_and(|w| w.eq_ignore_ascii_case("bold"))
            || attrs.get("bold").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let italic = attrs.get("italic").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let baseline = attrs.get("baseline").and_then(serde_json::Value::as_str).unwrap_or("");
        let superscript = baseline.eq_ignore_ascii_case("superscript") || baseline.eq_ignore_ascii_case("super");
        let subscript = baseline.eq_ignore_ascii_case("subscript") || baseline.eq_ignore_ascii_case("sub");
        if !(bold || italic || superscript || subscript) {
            continue;
        }

        for idx in parse_rich_text_range(range_str, chars.len()) {
            if let Some(slot) = styles.get_mut(idx) {
                slot.bold |= bold;
                slot.italic |= italic;
                slot.superscript |= superscript;
                slot.subscript |= subscript;
                any_style = true;
            }
        }
    }

    if !any_style {
        return (plain.to_owned(), plain.to_owned());
    }

    let escape_stars = chars.iter().filter(|&&c| c == '*').count() >= 2;
    let escape_dollars = chars.iter().filter(|&&c| c == '$').count() >= 2;

    let mut out = String::with_capacity(plain.len().saturating_add(16));
    let mut i = 0_usize;
    while i < chars.len() {
        let style = styles.get(i).copied().unwrap_or_default();
        let start = i;
        while i < chars.len() && styles.get(i).copied().unwrap_or_default() == style {
            i += 1;
        }
        let has_following = i < chars.len();
        let segment: String = chars.get(start..i).unwrap_or(&[]).iter().collect();
        let escaped = escape_inline_markup_text(&segment, escape_stars, escape_dollars, has_following);
        if style.bold {
            out.push_str("<b>");
        }
        if style.italic {
            out.push_str("<i>");
        }
        if style.superscript {
            out.push_str("<sup>");
        } else if style.subscript {
            out.push_str("<sub>");
        }
        out.push_str(&escaped);
        if style.superscript {
            out.push_str("</sup>");
        } else if style.subscript {
            out.push_str("</sub>");
        }
        if style.italic {
            out.push_str("</i>");
        }
        if style.bold {
            out.push_str("</b>");
        }
    }
    (out, plain.to_owned())
}

fn parse_rich_text_range(range_str: &str, char_len: usize) -> Vec<usize> {
    let trimmed = range_str.trim();
    if let Some((a, b)) = trimmed.split_once('~') {
        if let (Ok(start), Ok(end)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>())
            && start <= end
            && start < char_len
        {
            return (start..=end.min(char_len.saturating_sub(1))).collect();
        }
        return Vec::new();
    }
    if let Some((a, b)) = trimmed.split_once(',') {
        if let (Ok(start), Ok(len)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>())
            && start < char_len
            && len > 0
        {
            let end_excl = start.saturating_add(len).min(char_len);
            return (start..end_excl).collect();
        }
        return Vec::new();
    }
    if let Ok(single) = trimmed.parse::<usize>()
        && single < char_len
    {
        return vec![single];
    }
    Vec::new()
}

fn escape_inline_markup_text(text: &str, escape_stars: bool, escape_dollars: bool, has_following: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for (idx, ch) in text.char_indices() {
        let rest = text.get(idx..).unwrap_or_default();
        let after = text.get(idx + ch.len_utf8()..).unwrap_or_default();
        let needs_escape = match ch {
            '\\' => true,
            '<' => starts_with_curvebook_markup_tag(rest),
            '&' => starts_with_curvebook_markup_entity(rest),
            '^' | '_' => is_script_trigger_after(after, has_following),
            '*' => escape_stars && is_star_emphasis_trigger_after(after, has_following),
            '$' => escape_dollars,
            _ => false,
        };
        if needs_escape {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn starts_with_curvebook_markup_tag(rest: &str) -> bool {
    let Some(after) = rest.strip_prefix('<') else {
        return false;
    };
    let Some(close_idx) = after.as_bytes().iter().take(8).position(|&b| b == b'>') else {
        return false;
    };
    let Some(inner) = after.get(..close_idx) else {
        return false;
    };
    inner.eq_ignore_ascii_case("b")
        || inner.eq_ignore_ascii_case("/b")
        || inner.eq_ignore_ascii_case("i")
        || inner.eq_ignore_ascii_case("/i")
        || inner.eq_ignore_ascii_case("sup")
        || inner.eq_ignore_ascii_case("/sup")
        || inner.eq_ignore_ascii_case("sub")
        || inner.eq_ignore_ascii_case("/sub")
}

fn starts_with_curvebook_markup_entity(rest: &str) -> bool {
    let Some(after) = rest.strip_prefix('&') else {
        return false;
    };
    let Some(semi_idx) = after.as_bytes().iter().take(17).position(|&b| b == b';') else {
        return false;
    };
    if semi_idx == 0 || semi_idx > 16 {
        return false;
    }
    let Some(name) = after.get(..semi_idx) else {
        return false;
    };
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
    } else if let Some(dec) = name.strip_prefix('#') {
        !dec.is_empty() && dec.bytes().all(|b| b.is_ascii_digit())
    } else {
        name.bytes().all(|b| b.is_ascii_alphabetic())
    }
}

fn is_script_trigger_after(after: &str, has_following: bool) -> bool {
    let Some(first) = after.chars().next() else {
        return has_following;
    };
    if first == '{' || first == '\\' || first.is_alphanumeric() {
        return true;
    }
    if first == '-' || first == '\u{2212}' {
        return after.get(first.len_utf8()..).and_then(|s| s.chars().next()).is_some_and(char::is_alphanumeric);
    }
    false
}

fn is_star_emphasis_trigger_after(after: &str, has_following: bool) -> bool {
    let after_run = after.trim_start_matches('*');
    match after_run.chars().next() {
        Some(next) => !next.is_whitespace(),
        None => has_following,
    }
}

/// Strict ISO-8601 `YYYY-MM-DD` (or leading `YYYY-MM-DD` from an ISO-8601 timestamp) validator
/// using static per-month day bounds (`Feb <= 29`) with zero `chrono` or calendar arithmetic.
#[must_use]
pub fn parse_strict_iso8601_date(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let date_part = if trimmed.len() == 10 {
        trimmed
    } else if trimmed.len() > 10 && matches!(trimmed.as_bytes().get(10), Some(b'T' | b't' | b' ')) {
        trimmed.get(..10)?
    } else {
        return None;
    };
    let bytes = date_part.as_bytes();
    if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year: u16 = date_part.get(0..4)?.parse().ok()?;
    let month: u8 = date_part.get(5..7)?.parse().ok()?;
    let day: u8 = date_part.get(8..10)?.parse().ok()?;
    if year == 0 || !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 29,
        _ => return None,
    };
    (day <= max_day).then(|| date_part.to_owned())
}

/// Parse neutral hex, `rgb(...)`, `rgba(...)`, or W3C named color tags into [`Rgba8`].
#[must_use]
pub fn parse_archive_color(raw: &str) -> Option<Rgba8> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("transparent")
        || trimmed.eq_ignore_ascii_case("auto")
        || trimmed.eq_ignore_ascii_case("clear")
    {
        return None;
    }
    if let Some(hex) = trimmed.strip_prefix('#').or_else(|| trimmed.strip_prefix("0x")).or_else(|| trimmed.strip_prefix("0X")) {
        return parse_hex_color(hex);
    }
    let lower = trimmed.to_ascii_lowercase();
    if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<u8> = inner.split(',').filter_map(|p| p.trim().parse::<u8>().ok()).collect();
        if let (Some(&r), Some(&g), Some(&b)) = (parts.first(), parts.get(1), parts.get(2))
            && parts.len() == 3
        {
            return Some(Rgba8::rgb(r, g, b));
        }
        return None;
    }
    if let Some(inner) = lower.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() == 4
            && let (Ok(r), Ok(g), Ok(b)) = (
                parts.first().copied().unwrap_or("").parse::<u8>(),
                parts.get(1).copied().unwrap_or("").parse::<u8>(),
                parts.get(2).copied().unwrap_or("").parse::<u8>(),
            )
        {
            let a_str = parts.get(3).copied().unwrap_or("");
            let a = if let Ok(a_u8) = a_str.parse::<u8>() {
                a_u8
            } else if let Ok(a_f) = a_str.parse::<f64>()
                && (0.0..=1.0).contains(&a_f)
            {
                (a_f * 255.0).round() as u8
            } else {
                return None;
            };
            return Some(Rgba8::rgba(r, g, b, a));
        }
        return None;
    }
    match lower.as_str() {
        "red" => Some(Rgba8::rgb(220, 38, 38)),
        "orange" => Some(Rgba8::rgb(234, 88, 12)),
        "yellow" => Some(Rgba8::rgb(234, 179, 8)),
        "green" => Some(Rgba8::rgb(22, 163, 74)),
        "blue" => Some(Rgba8::rgb(37, 99, 235)),
        "purple" | "violet" => Some(Rgba8::rgb(147, 51, 234)),
        "pink" | "magenta" => Some(Rgba8::rgb(219, 39, 119)),
        "cyan" | "teal" => Some(Rgba8::rgb(8, 145, 178)),
        "gray" | "grey" => Some(Rgba8::rgb(107, 114, 128)),
        "black" => Some(Rgba8::rgb(0, 0, 0)),
        "white" => Some(Rgba8::rgb(255, 255, 255)),
        _ => None,
    }
}

fn parse_hex_color(hex: &str) -> Option<Rgba8> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(hex.get(0..1)?, 16).ok()? * 17;
            let g = u8::from_str_radix(hex.get(1..2)?, 16).ok()? * 17;
            let b = u8::from_str_radix(hex.get(2..3)?, 16).ok()? * 17;
            Some(Rgba8::rgb(r, g, b))
        }
        6 => {
            let r = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
            let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
            let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
            Some(Rgba8::rgb(r, g, b))
        }
        8 => {
            let r = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
            let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
            let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
            let a = u8::from_str_radix(hex.get(6..8)?, 16).ok()?;
            Some(Rgba8::rgba(r, g, b, a))
        }
        _ => None,
    }
}
