//! Unified Search & Auto-Suggest Engine for CurveBook (`Cmd/Ctrl+K` Command Palette & Navigator
//! Quick Jump).
//!
//! Indexes five live sources in a single deterministic, `wasm32`-clean pass:
//! - Project sheets across all 5 folders (`Data`, `Info`, `Results`, `Graphs`, `Layouts`)
//! - Scientific analyses (`ANALYSIS_CATALOG`)
//! - Built-in nonlinear fit equations (`curvebook_fit::list_builtin_equations`, cached via `OnceLock`)
//! - DataSheet column headers (capped at top 3 in unfiltered `All` mode when query length >= 2;
//!   unbounded under `#`)
//! - Live Engine + UI commands via `CommandPaletteState` / `filter_palette_commands`

use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

use curvebook_doc::{ProjectDocument, SheetId};
use curvebook_fit::{EquationCatalogEntry, EquationCategory};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::dialogs_canvas::{ANALYSIS_CATALOG, data_column_title};

/// Maximum number of recent search selections retained in [`SearchRecents`].
pub const MAX_SEARCH_RECENTS: usize = 8;

/// Maximum number of column hits surfaced in unfiltered [`SearchScopeFilter::All`] mode.
pub const MAX_UNFILTERED_COLUMN_RESULTS: usize = 3;

/// Minimum effective query length (in Unicode scalar values) required before DataSheet columns
/// are surfaced in unfiltered [`SearchScopeFilter::All`] mode.
pub const MIN_UNFILTERED_COLUMN_QUERY_CHARS: usize = 2;

// ============================================================================
// Scope Filter & Prefix Parsing
// ============================================================================

/// Optional leading-prefix scope filter for unified search queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SearchScopeFilter {
    /// Search across sheets, analyses, equations, commands, and (when query len >= 2) columns.
    #[default]
    All,
    /// Prefix `@`: search only project sheets across all 5 folders.
    Sheets,
    /// Prefix `=`: search only scientific analyses and built-in fit equations.
    AnalysesAndEquations,
    /// Prefix `>`: search only Engine and UI commands.
    Commands,
    /// Prefix `#`: search only DataSheet column titles (uncapped).
    Columns,
}

impl SearchScopeFilter {
    /// Leading prefix character activating this scope, or `None` for [`Self::All`].
    #[must_use]
    pub const fn prefix(self) -> Option<char> {
        match self {
            Self::All => None,
            Self::Sheets => Some('@'),
            Self::AnalysesAndEquations => Some('='),
            Self::Commands => Some('>'),
            Self::Columns => Some('#'),
        }
    }

    /// Short human-readable label for scope badges and empty-state hints.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Sheets => "Sheets",
            Self::AnalysesAndEquations => "Analyses & Equations",
            Self::Commands => "Commands",
            Self::Columns => "Columns",
        }
    }
}

/// Parse an optional leading scope prefix (`@`, `=`, `>`, `#`) from `raw`, returning the resolved
/// [`SearchScopeFilter`] and the trimmed effective query string.
#[must_use]
pub fn parse_scoped_query(raw: &str) -> (SearchScopeFilter, &str) {
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix('@') {
        (SearchScopeFilter::Sheets, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix('=') {
        (SearchScopeFilter::AnalysesAndEquations, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix('>') {
        (SearchScopeFilter::Commands, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix('#') {
        (SearchScopeFilter::Columns, rest.trim())
    } else {
        (SearchScopeFilter::All, trimmed.trim_end())
    }
}

// ============================================================================
// Unicode-Safe Fuzzy Matching With Byte Spans
// ============================================================================

/// Case-insensitive fuzzy match returning `(relevance_score, highlight_byte_spans)` into
/// `haystack`.
///
/// Scoring hierarchy:
/// - Exact match: `10_000`
/// - Prefix match: `5_000`
/// - Word-boundary contiguous substring: `2_500 - pos`
/// - Non-boundary contiguous substring: `1_500 - pos`
/// - Sequential fuzzy match: base `+15` per character, `+45` for consecutive matches, `+30` at
///   word starts, `-1` per skipped character
///
/// Every returned [`Range<usize>`] is guaranteed to lie on valid UTF-8 character boundaries within
/// `haystack`.
#[must_use]
pub fn fuzzy_match_with_spans(needle: &str, haystack: &str) -> Option<(i32, Vec<Range<usize>>)> {
    if needle.is_empty() {
        return Some((0, Vec::new()));
    }
    if haystack.is_empty() {
        return None;
    }

    let folded_needle = fold_chars(needle);
    if folded_needle.is_empty() {
        return Some((0, Vec::new()));
    }
    let folded_haystack = fold_haystack_with_ranges(haystack);
    if folded_haystack.len() < folded_needle.len() {
        return None;
    }

    if let Some(exact_or_prefix) = match_exact_or_prefix(&folded_needle, &folded_haystack, haystack.len()) {
        return Some(exact_or_prefix);
    }
    if let Some(substr) = match_best_substring(&folded_needle, &folded_haystack) {
        return Some(substr);
    }
    match_sequential_fuzzy(&folded_needle, &folded_haystack)
}

fn fold_chars(input: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(input.len());
    for ch in input.chars() {
        out.extend(ch.to_lowercase());
    }
    out
}

fn fold_haystack_with_ranges(haystack: &str) -> Vec<(char, Range<usize>)> {
    let mut out = Vec::with_capacity(haystack.len());
    for (byte_idx, ch) in haystack.char_indices() {
        let span = byte_idx..(byte_idx + ch.len_utf8());
        for lower in ch.to_lowercase() {
            out.push((lower, span.clone()));
        }
    }
    out
}

fn is_word_boundary(folded: &[(char, Range<usize>)], idx: usize) -> bool {
    idx == 0 || folded.get(idx.saturating_sub(1)).is_some_and(|(prev, _)| !prev.is_alphanumeric())
}

fn spans_for_slice(folded: &[(char, Range<usize>)], start: usize, len: usize) -> Vec<Range<usize>> {
    let end = start.saturating_add(len);
    let Some(slice) = folded.get(start..end) else {
        return Vec::new();
    };
    merge_spans(slice.iter().map(|(_, r)| r.clone()).collect())
}

fn merge_spans(raw: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(raw.len());
    for span in raw {
        if span.is_empty() {
            continue;
        }
        if let Some(last) = merged.last_mut()
            && span.start <= last.end
        {
            last.end = last.end.max(span.end);
            continue;
        }
        merged.push(span);
    }
    merged
}

fn match_exact_or_prefix(needle: &[char], haystack: &[(char, Range<usize>)], haystack_byte_len: usize) -> Option<(i32, Vec<Range<usize>>)> {
    let starts_with = haystack.iter().zip(needle.iter()).all(|((h, _), n)| h == n);
    if !starts_with {
        return None;
    }
    if haystack.len() == needle.len() {
        let mut spans = Vec::new();
        if haystack_byte_len > 0 {
            spans.push(0..haystack_byte_len);
        }
        return Some((10_000, spans));
    }
    let spans = spans_for_slice(haystack, 0, needle.len());
    Some((5_000, spans))
}

fn match_best_substring(needle: &[char], haystack: &[(char, Range<usize>)]) -> Option<(i32, Vec<Range<usize>>)> {
    let n_len = needle.len();
    let max_start = haystack.len().checked_sub(n_len)?;
    let mut best: Option<(i32, usize)> = None;

    for pos in 1..=max_start {
        let Some(window) = haystack.get(pos..(pos + n_len)) else {
            continue;
        };
        if !window.iter().zip(needle.iter()).all(|((h, _), n)| h == n) {
            continue;
        }
        let base: i32 = if is_word_boundary(haystack, pos) { 2_500 } else { 1_500 };
        let pos_penalty = i32::try_from(pos).unwrap_or(i32::MAX / 4);
        let score = base.saturating_sub(pos_penalty);
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, pos));
        }
    }

    best.map(|(score, pos)| (score, spans_for_slice(haystack, pos, n_len)))
}

fn match_sequential_fuzzy(needle: &[char], haystack: &[(char, Range<usize>)]) -> Option<(i32, Vec<Range<usize>>)> {
    let mut score = 0_i32;
    let mut prev_matched = false;
    let mut needle_idx = 0_usize;
    let mut raw_spans = Vec::with_capacity(needle.len());

    for (idx, (ch, range)) in haystack.iter().enumerate() {
        let Some(&target) = needle.get(needle_idx) else {
            break;
        };
        if *ch == target {
            score = score.saturating_add(if prev_matched { 45 } else { 15 });
            if is_word_boundary(haystack, idx) {
                score = score.saturating_add(30);
            }
            raw_spans.push(range.clone());
            prev_matched = true;
            needle_idx += 1;
        } else {
            prev_matched = false;
            score = score.saturating_sub(1);
        }
    }

    (needle_idx == needle.len()).then(|| (score, merge_spans(raw_spans)))
}

// ============================================================================
// Unified Search Result Types & Recents Ring Buffer
// ============================================================================

/// Result category in the unified search surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum UnifiedSearchKind {
    Recent,
    Sheet,
    Analysis,
    Equation,
    Column,
    Command,
}

impl UnifiedSearchKind {
    /// Deterministic tie-break priority when two results have equal relevance scores.
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            Self::Recent => 0,
            Self::Sheet => 1,
            Self::Analysis => 2,
            Self::Equation => 3,
            Self::Column => 4,
            Self::Command => 5,
        }
    }

    /// Short display badge for this result kind.
    #[must_use]
    pub const fn badge(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Sheet => "Sheet",
            Self::Analysis => "Analysis",
            Self::Equation => "Equation",
            Self::Column => "Column",
            Self::Command => "Command",
        }
    }
}

/// Typed dispatch payload executed when the user activates a [`UnifiedSearchItem`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnifiedSearchAction {
    ActivateSheet(SheetId),
    JumpToColumn { sheet_id: SheetId, col: usize },
    OpenAnalysis { catalog_id: &'static str },
    OpenEquation { equation_id: String },
    RunCommand { command_id: String, is_ui_command: bool },
}

impl UnifiedSearchAction {
    /// Stable deduplication key identifying this action across recents and live query hits.
    #[must_use]
    pub fn action_key(&self) -> String {
        match self {
            Self::ActivateSheet(id) => format!("sheet:{}", id.as_str()),
            Self::JumpToColumn { sheet_id, col } => format!("col:{}:{col}", sheet_id.as_str()),
            Self::OpenAnalysis { catalog_id } => format!("analysis:{catalog_id}"),
            Self::OpenEquation { equation_id } => format!("equation:{equation_id}"),
            Self::RunCommand { command_id, is_ui_command } => format!("cmd:{is_ui_command}:{command_id}"),
        }
    }
}

/// Single ranked search result returned by [`query_unified`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedSearchItem {
    pub kind: UnifiedSearchKind,
    pub title: String,
    pub subtitle: String,
    pub category_badge: String,
    pub shortcut: String,
    pub match_spans: Vec<Range<usize>>,
    pub enabled: bool,
    pub score: i32,
    pub action: UnifiedSearchAction,
    pub completion_text: String,
}

/// Persisted-in-memory recent search selection stored inside [`SearchRecents`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentSearchEntry {
    pub title: String,
    pub subtitle: String,
    pub category_badge: String,
    pub shortcut: String,
    pub action: UnifiedSearchAction,
    pub completion_text: String,
    pub original_kind: UnifiedSearchKind,
}

/// Fixed-capacity (`MAX_SEARCH_RECENTS = 8`), deduplicated, most-recent-first ring buffer of
/// recently activated search items.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchRecents {
    entries: Vec<RecentSearchEntry>,
}

impl SearchRecents {
    /// Create an empty recents buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an activated search item at the front of the buffer, deduplicating by action key
    /// and evicting the oldest entry past [`MAX_SEARCH_RECENTS`].
    pub fn record(&mut self, item: &UnifiedSearchItem) {
        let key = item.action.action_key();
        self.entries.retain(|existing| existing.action.action_key() != key);
        self.entries.insert(
            0,
            RecentSearchEntry {
                title: item.title.clone(),
                subtitle: item.subtitle.clone(),
                category_badge: item.category_badge.clone(),
                shortcut: item.shortcut.clone(),
                action: item.action.clone(),
                completion_text: item.completion_text.clone(),
                original_kind: item.kind,
            },
        );
        self.entries.truncate(MAX_SEARCH_RECENTS);
    }

    /// Borrow all recorded recents in most-recent-first order.
    #[must_use]
    pub fn items(&self) -> &[RecentSearchEntry] {
        &self.entries
    }

    /// Number of stored recent entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no recent entries have been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remove all recorded entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

// ============================================================================
// Unified Query Engine
// ============================================================================

/// Query across sheets, scientific analyses, built-in equations, DataSheet columns, and live
/// Engine/UI commands.
///
/// - When `raw_query` has an empty effective query under [`SearchScopeFilter::All`], returns valid
///   [`SearchRecents`] first, followed by top default sheets and commands.
/// - Respects scope prefixes (`@` Sheets, `=` Analyses & Equations, `>` Commands, `#` Columns).
/// - Under [`SearchScopeFilter::All`], includes up to [`MAX_UNFILTERED_COLUMN_RESULTS`] (`3`)
///   column matches once the effective query length is at least
///   [`MIN_UNFILTERED_COLUMN_QUERY_CHARS`] (`2`). Under `#`, all matching columns are included.
/// - Results are deterministically sorted by `(score desc, kind priority, title asc)`.
#[must_use]
pub fn query_unified(app: &UiApp, raw_query: &str, max_rows: usize, recents: &SearchRecents) -> Vec<UnifiedSearchItem> {
    if max_rows == 0 {
        return Vec::new();
    }
    let (scope, query) = parse_scoped_query(raw_query);

    if query.is_empty() && scope == SearchScopeFilter::All {
        return query_empty_default(app, max_rows, recents);
    }

    let mut results = Vec::new();
    match scope {
        SearchScopeFilter::All => {
            index_sheets(app.document(), query, &mut results);
            index_analyses(app.document(), query, &mut results);
            index_equations(app.document(), query, &mut results);
            if query.chars().count() >= MIN_UNFILTERED_COLUMN_QUERY_CHARS {
                let mut col_hits = Vec::new();
                index_columns(app.document(), query, &mut col_hits);
                sort_items(&mut col_hits);
                col_hits.truncate(MAX_UNFILTERED_COLUMN_RESULTS);
                results.extend(col_hits);
            }
            index_commands(app, query, &mut results);
        }
        SearchScopeFilter::Sheets => index_sheets(app.document(), query, &mut results),
        SearchScopeFilter::AnalysesAndEquations => {
            index_analyses(app.document(), query, &mut results);
            index_equations(app.document(), query, &mut results);
        }
        SearchScopeFilter::Commands => index_commands(app, query, &mut results),
        SearchScopeFilter::Columns => index_columns(app.document(), query, &mut results),
    }

    sort_items(&mut results);
    results.truncate(max_rows);
    results
}

fn query_empty_default(app: &UiApp, max_rows: usize, recents: &SearchRecents) -> Vec<UnifiedSearchItem> {
    let doc = app.document();
    let mut out = Vec::with_capacity(max_rows);
    let mut seen_keys = HashSet::new();

    for (idx, entry) in recents.items().iter().enumerate() {
        if out.len() >= max_rows {
            break;
        }
        if !recent_action_is_valid(doc, &entry.action) {
            continue;
        }
        let key = entry.action.action_key();
        if !seen_keys.insert(key) {
            continue;
        }
        let offset = i32::try_from(idx).unwrap_or(0);
        out.push(UnifiedSearchItem {
            kind: UnifiedSearchKind::Recent,
            title: entry.title.clone(),
            subtitle: entry.subtitle.clone(),
            category_badge: String::from("Recent"),
            shortcut: entry.shortcut.clone(),
            match_spans: Vec::new(),
            enabled: recent_action_is_enabled(doc, &entry.action),
            score: 20_000_i32.saturating_sub(offset),
            action: entry.action.clone(),
            completion_text: entry.completion_text.clone(),
        });
    }

    if out.len() < max_rows {
        let mut default_commands = Vec::new();
        index_commands(app, "", &mut default_commands);
        for item in default_commands {
            if out.len() >= max_rows {
                break;
            }
            if item.enabled && seen_keys.insert(item.action.action_key()) {
                out.push(item);
            }
        }
    }

    if out.len() < max_rows {
        let mut default_sheets = Vec::new();
        index_sheets(doc, "", &mut default_sheets);
        for item in default_sheets {
            if out.len() >= max_rows {
                break;
            }
            if seen_keys.insert(item.action.action_key()) {
                out.push(item);
            }
        }
    }

    out.truncate(max_rows);
    out
}

fn recent_action_is_valid(doc: &ProjectDocument, action: &UnifiedSearchAction) -> bool {
    match action {
        UnifiedSearchAction::ActivateSheet(id) => doc.sheet_section(id).is_some(),
        UnifiedSearchAction::JumpToColumn { sheet_id, col } => doc.data_sheet(sheet_id).is_some_and(|ds| *col < ds.archetype.column_count()),
        UnifiedSearchAction::OpenAnalysis { .. } | UnifiedSearchAction::OpenEquation { .. } | UnifiedSearchAction::RunCommand { .. } => true,
    }
}

fn recent_action_is_enabled(doc: &ProjectDocument, action: &UnifiedSearchAction) -> bool {
    match action {
        UnifiedSearchAction::ActivateSheet(id) => doc.sheet_section(id).is_some(),
        UnifiedSearchAction::JumpToColumn { sheet_id, col } => doc.data_sheet(sheet_id).is_some_and(|ds| *col < ds.archetype.column_count()),
        UnifiedSearchAction::OpenAnalysis { catalog_id } => *catalog_id == "transform.simulate" || !doc.data_sheets.is_empty(),
        UnifiedSearchAction::OpenEquation { .. } => !doc.data_sheets.is_empty(),
        UnifiedSearchAction::RunCommand { .. } => true,
    }
}

fn sort_items(items: &mut [UnifiedSearchItem]) {
    items.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.kind.priority().cmp(&b.kind.priority()))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.subtitle.cmp(&b.subtitle))
    });
}

// ============================================================================
// Domain Indexers
// ============================================================================

fn index_sheets(doc: &ProjectDocument, query: &str, out: &mut Vec<UnifiedSearchItem>) {
    for sheet in &doc.data_sheets {
        let subtitle = format!("Data · {} ({} rows)", sheet.kind().label(), sheet.archetype.row_count());
        push_sheet_candidate(query, &sheet.title, subtitle, "Data", sheet.id.clone(), out);
    }
    for sheet in &doc.info_sheets {
        let subtitle = format!("Info · {} constants", sheet.constants.len());
        push_sheet_candidate(query, &sheet.title, subtitle, "Info", sheet.id.clone(), out);
    }
    for sheet in &doc.result_sheets {
        let subtitle = format!("Results · {} tabs", sheet.tabs.len());
        push_sheet_candidate(query, sheet.title(), subtitle, "Results", sheet.id.clone(), out);
    }
    for sheet in &doc.graph_sheets {
        let subtitle =
            if sheet.initialized { format!("Graph · {} series", sheet.spec.series.len()) } else { String::from("Graph · Uninitialized") };
        push_sheet_candidate(query, &sheet.title, subtitle, "Graph", sheet.id.clone(), out);
    }
    for sheet in &doc.layout_sheets {
        let subtitle = format!("Layout · {} items", sheet.spec.items.len());
        push_sheet_candidate(query, &sheet.title, subtitle, "Layout", sheet.id.clone(), out);
    }
}

fn push_sheet_candidate(query: &str, title: &str, subtitle: String, badge: &'static str, sheet_id: SheetId, out: &mut Vec<UnifiedSearchItem>) {
    let (score, match_spans) = if query.is_empty() {
        (0, Vec::new())
    } else {
        let title_hit = fuzzy_match_with_spans(query, title);
        let subtitle_hit = fuzzy_match_with_spans(query, &subtitle).map(|(s, _)| s.saturating_sub(200));
        let best_score = [title_hit.as_ref().map(|(s, _)| s.saturating_add(40)), subtitle_hit].into_iter().flatten().max();
        let Some(score) = best_score else {
            return;
        };
        let spans = title_hit.map_or_else(Vec::new, |(_, s)| s);
        (score, spans)
    };

    out.push(UnifiedSearchItem {
        kind: UnifiedSearchKind::Sheet,
        title: title.to_owned(),
        subtitle,
        category_badge: badge.to_owned(),
        shortcut: String::new(),
        match_spans,
        enabled: true,
        score,
        action: UnifiedSearchAction::ActivateSheet(sheet_id),
        completion_text: title.to_owned(),
    });
}

fn index_analyses(doc: &ProjectDocument, query: &str, out: &mut Vec<UnifiedSearchItem>) {
    let has_data = !doc.data_sheets.is_empty();
    for entry in ANALYSIS_CATALOG {
        let cat_label = entry.category.label();
        let (score, match_spans) = if query.is_empty() {
            (0, Vec::new())
        } else {
            let label_hit = fuzzy_match_with_spans(query, entry.label);
            let id_hit = fuzzy_match_with_spans(query, entry.id).map(|(s, _)| s.saturating_add(10));
            let kw_hit = fuzzy_match_with_spans(query, entry.keywords).map(|(s, _)| s);
            let cat_hit = fuzzy_match_with_spans(query, cat_label).map(|(s, _)| s.saturating_sub(50));
            let sum_hit = fuzzy_match_with_spans(query, entry.summary).map(|(s, _)| s.saturating_sub(100));
            let best_score = [label_hit.as_ref().map(|(s, _)| s.saturating_add(30)), id_hit, kw_hit, cat_hit, sum_hit].into_iter().flatten().max();
            let Some(score) = best_score else {
                continue;
            };
            let spans = label_hit.map_or_else(Vec::new, |(_, s)| s);
            (score, spans)
        };

        out.push(UnifiedSearchItem {
            kind: UnifiedSearchKind::Analysis,
            title: entry.label.to_owned(),
            subtitle: entry.summary.to_owned(),
            category_badge: cat_label.to_owned(),
            shortcut: String::new(),
            match_spans,
            enabled: has_data || entry.id == "transform.simulate",
            score,
            action: UnifiedSearchAction::OpenAnalysis { catalog_id: entry.id },
            completion_text: entry.label.to_owned(),
        });
    }
}

fn cached_builtin_equations() -> &'static [EquationCatalogEntry] {
    static EQUATIONS: OnceLock<Vec<EquationCatalogEntry>> = OnceLock::new();
    EQUATIONS.get_or_init(curvebook_fit::list_builtin_equations)
}

const fn equation_category_label(category: EquationCategory) -> &'static str {
    match category {
        EquationCategory::DoseResponseStimulation => "Dose-Response (Stimulation)",
        EquationCategory::DoseResponseInhibition => "Dose-Response (Inhibition)",
        EquationCategory::ReceptorBinding => "Receptor Binding",
        EquationCategory::EnzymeKinetics => "Enzyme Kinetics",
        EquationCategory::Exponential => "Exponential",
        EquationCategory::Polynomial => "Polynomial",
        EquationCategory::Gaussian => "Gaussian",
        EquationCategory::SineWave => "Sine Wave",
        EquationCategory::Growth => "Growth",
        EquationCategory::Peak => "Peak",
        EquationCategory::StandardCurve => "Standard Curve",
        EquationCategory::Radiobiology => "Radiobiology",
        EquationCategory::NistBenchmark => "NIST Benchmark",
        EquationCategory::UserDefined => "User Defined",
    }
}

fn index_equations(doc: &ProjectDocument, query: &str, out: &mut Vec<UnifiedSearchItem>) {
    let has_data = !doc.data_sheets.is_empty();
    for eq in cached_builtin_equations() {
        let cat_label = equation_category_label(eq.category);
        let (score, match_spans) = if query.is_empty() {
            (0, Vec::new())
        } else {
            let name_hit = fuzzy_match_with_spans(query, &eq.name);
            let id_hit = fuzzy_match_with_spans(query, &eq.id).map(|(s, _)| s.saturating_add(15));
            let cat_hit = fuzzy_match_with_spans(query, cat_label).map(|(s, _)| s.saturating_sub(50));
            let best_score = [name_hit.as_ref().map(|(s, _)| s.saturating_add(30)), id_hit, cat_hit].into_iter().flatten().max();
            let Some(score) = best_score else {
                continue;
            };
            let spans = name_hit.map_or_else(Vec::new, |(_, s)| s);
            (score, spans)
        };

        out.push(UnifiedSearchItem {
            kind: UnifiedSearchKind::Equation,
            title: eq.name.clone(),
            subtitle: format!("{cat_label} · {}", eq.id),
            category_badge: cat_label.to_owned(),
            shortcut: String::new(),
            match_spans,
            enabled: has_data,
            score,
            action: UnifiedSearchAction::OpenEquation { equation_id: eq.id.clone() },
            completion_text: eq.name.clone(),
        });
    }
}

fn index_columns(doc: &ProjectDocument, query: &str, out: &mut Vec<UnifiedSearchItem>) {
    for sheet in &doc.data_sheets {
        let col_count = sheet.archetype.column_count();
        for col in 0..col_count {
            let raw_title = data_column_title(sheet, col).unwrap_or("");
            let col_letter = curvebook_doc::default_column_label(col);
            let title = if raw_title.trim().is_empty() { format!("Column {col_letter}") } else { raw_title.to_owned() };
            let subtitle = format!("{} · Col {col_letter}", sheet.title);

            let (score, match_spans) = if query.is_empty() {
                (0, Vec::new())
            } else {
                let title_hit = fuzzy_match_with_spans(query, &title);
                let letter_hit = fuzzy_match_with_spans(query, &col_letter).map(|(s, _)| s.saturating_sub(100));
                let sheet_hit = fuzzy_match_with_spans(query, &sheet.title).map(|(s, _)| s.saturating_sub(250));
                let best_score = [title_hit.as_ref().map(|(s, _)| s.saturating_add(20)), letter_hit, sheet_hit].into_iter().flatten().max();
                let Some(score) = best_score else {
                    continue;
                };
                let spans = title_hit.map_or_else(Vec::new, |(_, s)| s);
                (score, spans)
            };

            out.push(UnifiedSearchItem {
                kind: UnifiedSearchKind::Column,
                title: title.clone(),
                subtitle,
                category_badge: String::from("Column"),
                shortcut: String::new(),
                match_spans,
                enabled: true,
                score,
                action: UnifiedSearchAction::JumpToColumn { sheet_id: sheet.id.clone(), col },
                completion_text: title,
            });
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn index_commands(app: &UiApp, query: &str, out: &mut Vec<UnifiedSearchItem>) {
    let rows = crate::egui_shell::command_palette::filter_palette_commands(app, query, 512);
    for row in rows {
        let subtitle = format_command_subtitle(&row.menu_path, &row.summary);
        let (score, match_spans) = score_command_item(query, &row.label, &row.command_id, &row.menu_path, &row.summary, &row.shortcut);
        out.push(UnifiedSearchItem {
            kind: UnifiedSearchKind::Command,
            title: row.label.clone(),
            subtitle,
            category_badge: String::from("Command"),
            shortcut: row.shortcut,
            match_spans,
            enabled: row.enabled,
            score,
            action: UnifiedSearchAction::RunCommand { command_id: row.command_id, is_ui_command: row.is_ui_command },
            completion_text: row.label,
        });
    }
}

#[cfg(target_arch = "wasm32")]
fn index_commands(app: &UiApp, query: &str, out: &mut Vec<UnifiedSearchItem>) {
    let mut palette = app.canvas_dialogs.command_palette.clone();
    palette.query = query.to_owned();
    palette.sync_shortcuts(&app.shortcuts);
    for entry in palette.matches_with_shortcuts(Some(app.session()), &app.shortcuts) {
        if matches!(entry.id, "ui.event.dispatch" | "ui.screenshot" | "ui.result.applyRemediation") {
            continue;
        }
        let label = entry.localized_label(&app.locale).to_owned();
        let subtitle = format_command_subtitle(entry.menu_path, entry.summary);
        let (score, match_spans) = score_command_item(query, &label, entry.id, entry.menu_path, entry.summary, &entry.shortcut);
        let is_ui_command = matches!(entry.origin, crate::dialogs_canvas::CommandOrigin::Ui);
        out.push(UnifiedSearchItem {
            kind: UnifiedSearchKind::Command,
            title: label.clone(),
            subtitle,
            category_badge: String::from("Command"),
            shortcut: entry.shortcut,
            match_spans,
            enabled: entry.enabled,
            score,
            action: UnifiedSearchAction::RunCommand { command_id: entry.id.to_owned(), is_ui_command },
            completion_text: label,
        });
    }
}

fn format_command_subtitle(menu_path: &str, summary: &str) -> String {
    if !menu_path.is_empty() && !summary.is_empty() {
        format!("{menu_path} — {summary}")
    } else if !summary.is_empty() {
        summary.to_owned()
    } else {
        menu_path.to_owned()
    }
}

fn score_command_item(query: &str, label: &str, command_id: &str, menu_path: &str, summary: &str, shortcut: &str) -> (i32, Vec<Range<usize>>) {
    if query.is_empty() {
        return (0, Vec::new());
    }
    let label_hit = fuzzy_match_with_spans(query, label);
    let id_hit = fuzzy_match_with_spans(query, command_id).map(|(s, _)| s.saturating_add(20));
    let menu_hit = fuzzy_match_with_spans(query, menu_path).map(|(s, _)| s);
    let sc_hit = fuzzy_match_with_spans(query, shortcut).map(|(s, _)| s);
    let sum_hit = fuzzy_match_with_spans(query, summary).map(|(s, _)| s.saturating_sub(100));
    let best_score =
        [label_hit.as_ref().map(|(s, _)| s.saturating_add(25)), id_hit, menu_hit, sc_hit, sum_hit].into_iter().flatten().max().unwrap_or(10);
    let spans = label_hit.map_or_else(Vec::new, |(_, s)| s);
    (best_score, spans)
}

// ============================================================================
// Inline Ghost-Text Completion (Tab to Complete)
// ============================================================================

/// Compute the remaining ghost-text suffix for inline `Tab` completion.
///
/// Strips any leading scope prefix (`@`, `=`, `>`, `#`) and whitespace before the query, while
/// preserving trailing whitespace so `"log "` does not falsely complete `"Log10"`. Returns
/// `Some(suffix)` only when the effective query is non-empty, is a case-insensitive prefix of
/// `top_item.completion_text`, and leaves a non-empty suffix.
#[must_use]
pub fn compute_ghost_completion(raw_query: &str, top_item: Option<&UnifiedSearchItem>) -> Option<String> {
    let item = top_item?;
    let trimmed_start = raw_query.trim_start();
    let effective = match trimmed_start.chars().next() {
        Some('@' | '=' | '>' | '#') => {
            let mut chars = trimmed_start.chars();
            let _ = chars.next();
            chars.as_str().trim_start()
        }
        _ => trimmed_start,
    };
    if effective.is_empty() {
        return None;
    }

    let folded_query = fold_chars(effective);
    if folded_query.is_empty() {
        return None;
    }
    let folded_target = fold_haystack_with_ranges(&item.completion_text);
    if folded_target.len() <= folded_query.len() {
        return None;
    }

    let mut last_end = 0_usize;
    for (idx, &q_ch) in folded_query.iter().enumerate() {
        let (t_ch, t_range) = folded_target.get(idx)?;
        if *t_ch != q_ch {
            return None;
        }
        last_end = t_range.end;
    }

    // Avoid splitting inside a single source character that expanded to multiple lowercase chars.
    if let Some((_, next_range)) = folded_target.get(folded_query.len())
        && next_range.start < last_end
    {
        return None;
    }

    if last_end >= item.completion_text.len() {
        return None;
    }
    item.completion_text.get(last_end..).map(str::to_owned)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use curvebook_doc::{
        DataSetColumn, DataSheet, GraphSheet, InfoSheet, LayoutSheet, LayoutSpec, ProjectDocument, SubcolumnMode, TableArchetype, XyTable,
    };
    use curvebook_engine::Session;

    fn make_test_app_with_rich_doc() -> UiApp {
        let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).expect("xy table");
        xy.data_sets[0] = DataSetColumn::new("Control Buffer", 4, 1).expect("col 0");
        xy.data_sets[1] = DataSetColumn::new("Drug Concentration (µM)", 4, 1).expect("col 1");
        let mut extra_cols = vec![
            DataSetColumn::new("Dose Alpha", 4, 1).expect("col 2"),
            DataSetColumn::new("Dose Beta", 4, 1).expect("col 3"),
            DataSetColumn::new("Dose Gamma", 4, 1).expect("col 4"),
            DataSetColumn::new("Dose Delta", 4, 1).expect("col 5"),
        ];
        xy.data_sets.append(&mut extra_cols);
        let data_sheet = DataSheet::new("data-1", "Enzyme Assay", TableArchetype::Xy(xy)).expect("data sheet");

        let info_sheet = InfoSheet::new_preseeded("info-1", "Lab Notes").expect("info sheet");
        let mut graph_sheet = GraphSheet::new_uninitialized_for_data("graph-1", &data_sheet).expect("graph sheet");
        graph_sheet.title = String::from("Enzyme Kinetics Curve");
        let layout_sheet = LayoutSheet::new("layout-1", "Figure 1 Panel", LayoutSpec::default()).expect("layout sheet");

        let mut doc = ProjectDocument::default();
        doc.data_sheets.push(data_sheet);
        doc.info_sheets.push(info_sheet);
        doc.graph_sheets.push(graph_sheet);
        doc.layout_sheets.push(layout_sheet);
        UiApp::from_session(Session::from_document(doc))
    }

    #[test]
    fn parse_scoped_query_handles_all_prefixes_and_whitespace() {
        assert_eq!(parse_scoped_query("  hello world  "), (SearchScopeFilter::All, "hello world"));
        assert_eq!(parse_scoped_query("@Enzyme"), (SearchScopeFilter::Sheets, "Enzyme"));
        assert_eq!(parse_scoped_query("  @  Enzyme  "), (SearchScopeFilter::Sheets, "Enzyme"));
        assert_eq!(parse_scoped_query("=anova"), (SearchScopeFilter::AnalysesAndEquations, "anova"));
        assert_eq!(parse_scoped_query(">undo"), (SearchScopeFilter::Commands, "undo"));
        assert_eq!(parse_scoped_query("#Dose"), (SearchScopeFilter::Columns, "Dose"));
        assert_eq!(parse_scoped_query("#"), (SearchScopeFilter::Columns, ""));
    }

    #[test]
    fn fuzzy_match_scoring_hierarchy_and_valid_spans() {
        let exact = fuzzy_match_with_spans("ANOVA", "ANOVA").expect("exact");
        let prefix = fuzzy_match_with_spans("ANOVA", "ANOVA One-Way").expect("prefix");
        let word_boundary = fuzzy_match_with_spans("ANOVA", "One-way ANOVA").expect("word boundary");
        let mid_substring = fuzzy_match_with_spans("nova", "Supernova").expect("substring");
        let fuzzy = fuzzy_match_with_spans("owa", "One-way ANOVA").expect("fuzzy");

        assert_eq!(exact.0, 10_000);
        assert_eq!(prefix.0, 5_000);
        assert!(exact.0 > prefix.0);
        assert!(prefix.0 > word_boundary.0);
        assert!(word_boundary.0 > mid_substring.0);
        assert!(mid_substring.0 > fuzzy.0);

        assert_eq!(exact.1, vec![0..5]);
        assert_eq!(prefix.1, vec![0..5]);
        assert_eq!(word_boundary.1, vec![8..13]);
        assert_eq!(mid_substring.1, vec![5..9]);
        assert!(!fuzzy.1.is_empty());
        for span in &fuzzy.1 {
            assert!("One-way ANOVA".is_char_boundary(span.start));
            assert!("One-way ANOVA".is_char_boundary(span.end));
        }
    }

    #[test]
    fn fuzzy_match_handles_arbitrary_unicode_without_panicking() {
        // Multi-byte UTF-8 (µ, Ω, CJK, emoji) and case-expanding characters (İ -> i + combining dot).
        let hay = "µM Concentration — İstanbul 🔬";
        let (score, spans) = fuzzy_match_with_spans("µm", hay).expect("micro prefix");
        assert_eq!(score, 5_000);
        assert_eq!(spans, vec![0..3]);
        assert_eq!(hay.get(spans[0].clone()), Some("µM"));

        let (_, istanbul_spans) = fuzzy_match_with_spans("i\u{307}stanbul", hay).expect("dotted I");
        for span in &istanbul_spans {
            assert!(hay.is_char_boundary(span.start));
            assert!(hay.is_char_boundary(span.end));
            assert!(hay.get(span.clone()).is_some());
        }

        let (_, emoji_spans) = fuzzy_match_with_spans("🔬", hay).expect("emoji");
        assert_eq!(hay.get(emoji_spans[0].clone()), Some("🔬"));

        assert!(fuzzy_match_with_spans("xyz", hay).is_none());
        assert!(fuzzy_match_with_spans("a", "").is_none());
    }

    #[test]
    fn ghost_completion_handles_prefixes_case_insensitivity_and_unicode() {
        let item = UnifiedSearchItem {
            kind: UnifiedSearchKind::Equation,
            title: String::from("Michaelis-Menten"),
            subtitle: String::new(),
            category_badge: String::from("Enzyme Kinetics"),
            shortcut: String::new(),
            match_spans: Vec::new(),
            enabled: true,
            score: 5_000,
            action: UnifiedSearchAction::OpenEquation { equation_id: String::from("michaelis_menten") },
            completion_text: String::from("Michaelis-Menten"),
        };

        assert_eq!(compute_ghost_completion("mic", Some(&item)).as_deref(), Some("haelis-Menten"));
        assert_eq!(compute_ghost_completion("=MIC", Some(&item)).as_deref(), Some("haelis-Menten"));
        assert_eq!(compute_ghost_completion("=  mic", Some(&item)).as_deref(), Some("haelis-Menten"));
        assert_eq!(compute_ghost_completion("Michaelis-Menten", Some(&item)), None);
        assert_eq!(compute_ghost_completion("mic ", Some(&item)), None);
        assert_eq!(compute_ghost_completion("mm", Some(&item)), None);
        assert_eq!(compute_ghost_completion("", Some(&item)), None);
        assert_eq!(compute_ghost_completion("=", Some(&item)), None);
        assert_eq!(compute_ghost_completion("mic", None), None);

        let unicode_item = UnifiedSearchItem { completion_text: String::from("µM Dose Response"), ..item };
        assert_eq!(compute_ghost_completion("#µm ", Some(&unicode_item)).as_deref(), Some("Dose Response"));
    }

    #[test]
    fn search_recents_deduplicates_and_caps_at_eight() {
        let mut recents = SearchRecents::new();
        for idx in 0..10 {
            let item = UnifiedSearchItem {
                kind: UnifiedSearchKind::Command,
                title: format!("Cmd {idx}"),
                subtitle: String::new(),
                category_badge: String::from("Command"),
                shortcut: String::new(),
                match_spans: Vec::new(),
                enabled: true,
                score: 100,
                action: UnifiedSearchAction::RunCommand { command_id: format!("cmd.{idx}"), is_ui_command: true },
                completion_text: format!("Cmd {idx}"),
            };
            recents.record(&item);
        }
        assert_eq!(recents.len(), MAX_SEARCH_RECENTS);
        assert_eq!(recents.items()[0].title, "Cmd 9");
        assert_eq!(recents.items()[7].title, "Cmd 2");

        // Recording an existing item moves it to the front without increasing length.
        let repeat = UnifiedSearchItem {
            kind: UnifiedSearchKind::Command,
            title: String::from("Cmd 4"),
            subtitle: String::new(),
            category_badge: String::from("Command"),
            shortcut: String::new(),
            match_spans: Vec::new(),
            enabled: true,
            score: 100,
            action: UnifiedSearchAction::RunCommand { command_id: String::from("cmd.4"), is_ui_command: true },
            completion_text: String::from("Cmd 4"),
        };
        recents.record(&repeat);
        assert_eq!(recents.len(), MAX_SEARCH_RECENTS);
        assert_eq!(recents.items()[0].title, "Cmd 4");
        assert_eq!(recents.items().iter().filter(|e| e.title == "Cmd 4").count(), 1);
    }

    #[test]
    fn query_unified_indexes_sheets_analyses_equations_columns_and_commands() {
        let app = make_test_app_with_rich_doc();
        let recents = SearchRecents::new();

        // @ Sheets scope
        let sheets = query_unified(&app, "@Enzyme", 20, &recents);
        assert!(!sheets.is_empty());
        assert!(sheets.iter().all(|i| i.kind == UnifiedSearchKind::Sheet));
        assert!(sheets.iter().any(|i| i.title == "Enzyme Assay"));
        assert!(sheets.iter().any(|i| i.title == "Enzyme Kinetics Curve"));

        // = Analyses & Equations scope
        let eqs = query_unified(&app, "=michaelis", 20, &recents);
        assert!(!eqs.is_empty());
        assert!(eqs.iter().all(|i| matches!(i.kind, UnifiedSearchKind::Analysis | UnifiedSearchKind::Equation)));
        assert!(eqs.iter().any(|i| matches!(&i.action, UnifiedSearchAction::OpenEquation { equation_id } if equation_id == "michaelis_menten")));

        let analyses = query_unified(&app, "=anova", 20, &recents);
        assert!(analyses.iter().any(|i| matches!(i.action, UnifiedSearchAction::OpenAnalysis { catalog_id: "stats.one_way_anova" })));

        // > Commands scope
        let cmds = query_unified(&app, ">undo", 20, &recents);
        assert!(!cmds.is_empty());
        assert!(cmds.iter().all(|i| i.kind == UnifiedSearchKind::Command));
        assert!(cmds.iter().any(|i| matches!(&i.action, UnifiedSearchAction::RunCommand { command_id, .. } if command_id == "edit.undo")));

        // # Columns scope (uncapped: all 4 "Dose *" columns returned)
        let cols = query_unified(&app, "#Dose", 20, &recents);
        assert_eq!(cols.len(), 4);
        assert!(cols.iter().all(|i| i.kind == UnifiedSearchKind::Column));
    }

    #[test]
    fn query_unified_caps_columns_in_unfiltered_mode_and_requires_two_chars() {
        let app = make_test_app_with_rich_doc();
        let recents = SearchRecents::new();

        // 1-char unfiltered query must not include columns.
        let one_char = query_unified(&app, "D", 100, &recents);
        assert!(one_char.iter().all(|i| i.kind != UnifiedSearchKind::Column));

        // 2+ char unfiltered query caps column hits at MAX_UNFILTERED_COLUMN_RESULTS (3), even
        // though 4 columns match "Dose".
        let unfiltered = query_unified(&app, "Dose", 100, &recents);
        let col_count = unfiltered.iter().filter(|i| i.kind == UnifiedSearchKind::Column).count();
        assert_eq!(col_count, MAX_UNFILTERED_COLUMN_RESULTS);
    }

    #[test]
    fn query_unified_empty_query_surfaces_recents_first_and_deduplicates() {
        let app = make_test_app_with_rich_doc();
        let mut recents = SearchRecents::new();

        let sheet_hits = query_unified(&app, "@Lab Notes", 5, &recents);
        let lab_notes = sheet_hits.first().expect("Lab Notes sheet");
        recents.record(lab_notes);

        let empty_hits = query_unified(&app, "", 12, &recents);
        assert!(!empty_hits.is_empty());
        assert_eq!(empty_hits[0].kind, UnifiedSearchKind::Recent);
        assert_eq!(empty_hits[0].title, "Lab Notes");

        // Ensure Lab Notes is not duplicated later in the default list.
        let lab_count = empty_hits.iter().filter(|i| i.action == lab_notes.action).count();
        assert_eq!(lab_count, 1);
    }
}
