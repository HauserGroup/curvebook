//! Command Palette (`Cmd+K`) fuzzy matching over engine and UI commands.

use curvebook_engine::{Session, cmd};
use std::collections::BTreeMap;

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::menus::shortcuts::ShortcutRegistry;
use crate::menus::ui_commands::UI_COMMANDS;
use crate::menus::{ui_command_enabled_for_doc, ui_command_menu_info};

// ============================================================================
// Command Palette (Cmd+K) Fuzzy Filter Over Engine + UI Commands
// ============================================================================

/// Source layer of a Command Palette entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommandOrigin {
    Engine,
    Ui,
}

/// Scored command match displayed inside the Command Palette (`Cmd+K`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPaletteEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub menu_path: &'static str,
    pub shortcut: String,
    pub params_doc: String,
    pub summary: &'static str,
    pub beginner: &'static str,
    pub enabled: bool,
    pub origin: CommandOrigin,
    pub score: i32,
}

impl CommandPaletteEntry {
    /// Resolve the localized label for this command in `locale`, falling back to English [`Self::label`].
    #[must_use]
    pub fn localized_label(&self, locale: &str) -> &'static str {
        crate::i18n::command_label(locale, self.id, self.label)
    }

    fn format_row_header(&self, title: &str) -> String {
        let status = if self.enabled { "" } else { " [disabled]" };
        if self.shortcut.is_empty() {
            format!("{title} ({}){status} — {}", self.id, self.summary)
        } else {
            format!("{title} ({}) [{}]{status} — {}", self.id, self.shortcut, self.summary)
        }
    }

    /// Format compact command palette row copy surfacing `label`, `id`, `shortcut`, and `summary`.
    #[must_use]
    pub fn format_compact_row(&self) -> String {
        self.format_row_header(self.label)
    }

    /// Format compact command palette row copy using the localized label for `locale`.
    #[must_use]
    pub fn format_compact_row_in_locale(&self, locale: &str) -> String {
        self.format_row_header(self.localized_label(locale))
    }

    /// Format rich command palette row copy surfacing `label`, `id`, `shortcut`, `summary`, and `beginner`.
    #[must_use]
    pub fn format_palette_row(&self) -> String {
        self.format_palette_row_in_locale("en")
    }

    /// Format rich localized command palette row copy surfacing `summary` and `beginner`.
    #[must_use]
    pub fn format_palette_row_in_locale(&self, locale: &str) -> String {
        format!("{} | Tip: {}", self.format_row_header(self.localized_label(locale)), self.beginner)
    }
}

struct PaletteCandidate<'a> {
    id: &'static str,
    label: &'static str,
    menu_path: &'static str,
    default_shortcut: &'a str,
    params_doc: String,
    summary: &'static str,
    beginner: &'static str,
    enabled: bool,
    origin: CommandOrigin,
}

/// Command Palette (`Cmd+K`) fuzzy search state spanning Engine + UI commands.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommandPaletteState {
    pub query: String,
    pub selected_index: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shortcut_overrides: BTreeMap<String, String>,
}

impl CommandPaletteState {
    /// Reset search text and selection upon opening `Cmd+K`.
    pub fn open(&mut self) {
        self.query.clear();
        self.selected_index = 0;
    }

    /// Synchronize customized or unbound bindings from `shortcuts` so queries without a registry
    /// reference still surface live active shortcuts.
    pub fn sync_shortcuts(&mut self, shortcuts: &ShortcutRegistry) {
        self.shortcut_overrides.clear();
        for binding in shortcuts.bindings() {
            if binding.is_customized() {
                self.shortcut_overrides.insert(binding.command_id.clone(), binding.active_combo.clone());
            }
        }
    }

    /// Return fuzzy-filtered and relevance-sorted commands from both Engine and UI catalogs.
    #[must_use]
    pub fn matches(&self) -> Vec<CommandPaletteEntry> {
        self.collect_matches(None, None)
    }

    /// Return fuzzy-filtered and relevance-sorted commands with `enabled` evaluated for `session`.
    #[must_use]
    pub fn matches_for_session(&self, session: &Session) -> Vec<CommandPaletteEntry> {
        self.collect_matches(Some(session), None)
    }

    /// Return fuzzy-filtered and relevance-sorted commands using live shortcuts from `shortcuts`.
    #[must_use]
    pub fn matches_with_shortcuts(&self, session: Option<&Session>, shortcuts: &ShortcutRegistry) -> Vec<CommandPaletteEntry> {
        self.collect_matches(session, Some(shortcuts))
    }

    /// Format visible command palette rows surfacing `summary` and active `beginner` guidance.
    #[must_use]
    pub fn preview_rows(&self, session: Option<&Session>, max_rows: usize) -> Vec<String> {
        self.preview_rows_for_locale(session, "en", max_rows)
    }

    /// Format localized visible command palette rows surfacing `summary` and active `beginner` guidance.
    #[must_use]
    pub fn preview_rows_for_locale(&self, session: Option<&Session>, locale: &str, max_rows: usize) -> Vec<String> {
        self.format_preview_rows(&self.collect_matches(session, None), locale, max_rows)
    }

    /// Format localized visible command palette rows using live shortcuts from `shortcuts`.
    #[must_use]
    pub fn preview_rows_with_shortcuts(&self, session: Option<&Session>, shortcuts: &ShortcutRegistry, locale: &str, max_rows: usize) -> Vec<String> {
        self.format_preview_rows(&self.collect_matches(session, Some(shortcuts)), locale, max_rows)
    }

    fn format_preview_rows(&self, hits: &[CommandPaletteEntry], locale: &str, max_rows: usize) -> Vec<String> {
        let mut rows = Vec::with_capacity(hits.len().min(max_rows) + 2);
        rows.push(format!("> {}", self.query));
        for (idx, hit) in hits.iter().take(max_rows).enumerate() {
            let marker = if idx == self.selected_index { "* " } else { "  " };
            rows.push(format!("{marker}{}", hit.format_compact_row_in_locale(locale)));
        }
        if let Some(selected) = hits.get(self.selected_index).or_else(|| hits.first()) {
            rows.push(format!("Tip: {}", selected.beginner));
        }
        rows
    }

    fn resolve_shortcut(&self, shortcuts: Option<&ShortcutRegistry>, command_id: &str, default_shortcut: &str) -> String {
        if let Some(reg) = shortcuts
            && let Some(active) = reg.active_combo(command_id)
        {
            return active.to_owned();
        }
        if let Some(override_combo) = self.shortcut_overrides.get(command_id) {
            return override_combo.clone();
        }
        default_shortcut.to_owned()
    }

    fn try_build_entry(&self, shortcuts: Option<&ShortcutRegistry>, spec: PaletteCandidate<'_>, search_menu: &str) -> Option<CommandPaletteEntry> {
        let q = self.query.trim();
        let shortcut = self.resolve_shortcut(shortcuts, spec.id, spec.default_shortcut);
        let base = score_command(q, spec.id, spec.label, search_menu, &shortcut);
        let score = score_palette_entry(q, base, spec.summary, spec.beginner)?;
        Some(CommandPaletteEntry {
            id: spec.id,
            label: spec.label,
            menu_path: spec.menu_path,
            shortcut,
            params_doc: spec.params_doc,
            summary: spec.summary,
            beginner: spec.beginner,
            enabled: spec.enabled,
            origin: spec.origin,
            score,
        })
    }

    fn collect_matches(&self, session: Option<&Session>, shortcuts: Option<&ShortcutRegistry>) -> Vec<CommandPaletteEntry> {
        let mut results = Vec::new();
        for spec in cmd::registry().specs() {
            let schema = (spec.input_schema)();
            let params_doc = format_schema_params_doc(&schema, spec.summary);
            let candidate = PaletteCandidate {
                id: spec.id,
                label: spec.label,
                menu_path: spec.menu_path,
                default_shortcut: spec.shortcut,
                params_doc,
                summary: spec.summary,
                beginner: spec.beginner,
                enabled: session.is_none_or(|s| (spec.enabled)(s)),
                origin: CommandOrigin::Engine,
            };
            if let Some(entry) = self.try_build_entry(shortcuts, candidate, spec.menu_path) {
                results.push(entry);
            }
        }
        for spec in UI_COMMANDS {
            let (_menu_key, menu_path) = ui_command_menu_info(spec.id);
            let candidate = PaletteCandidate {
                id: spec.id,
                label: spec.label,
                menu_path,
                default_shortcut: spec.shortcut,
                params_doc: spec.params_doc.to_owned(),
                summary: spec.summary,
                beginner: spec.beginner,
                enabled: session.is_none_or(|s| ui_command_enabled_for_doc(s.document(), spec.id)),
                origin: CommandOrigin::Ui,
            };
            if let Some(entry) = self.try_build_entry(shortcuts, candidate, menu_path) {
                results.push(entry);
            }
        }
        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(b.id)));
        results
    }
}

fn score_palette_entry(query: &str, base_score: Option<i32>, summary: &str, beginner: &str) -> Option<i32> {
    if let Some(score) = base_score {
        return Some(score);
    }
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Some(0);
    }
    if summary.to_ascii_lowercase().contains(&needle) {
        return Some(40);
    }
    if beginner.to_ascii_lowercase().contains(&needle) {
        return Some(30);
    }
    None
}

impl CommandPaletteState {
    /// Move selection cursor down in the filtered list.
    pub fn select_next(&mut self) {
        let count = self.matches().len();
        if count > 0 {
            self.selected_index = (self.selected_index + 1) % count;
        }
    }

    /// Move selection cursor up in the filtered list.
    pub fn select_prev(&mut self) {
        let count = self.matches().len();
        if count > 0 {
            self.selected_index = if self.selected_index == 0 { count - 1 } else { self.selected_index - 1 };
        }
    }

    /// Emit [`ToolCommandIntent`] for the highlighted entry when no live [`Session`] is supplied.
    #[must_use]
    pub fn activate_selected(&self) -> Option<ToolCommandIntent> {
        self.activate_selected_for_session(None)
    }

    /// Emit [`ToolCommandIntent`] for the highlighted enabled entry, populating schema-required
    /// sheet identifiers and safe defaults from `session` when available.
    #[must_use]
    pub fn activate_selected_for_session(&self, session: Option<&Session>) -> Option<ToolCommandIntent> {
        let list = match session {
            Some(s) => self.matches_for_session(s),
            None => self.matches(),
        };
        let entry = list.get(self.selected_index).or_else(|| list.first())?;
        if session.is_some() && !entry.enabled {
            return None;
        }
        let params = build_default_params(entry.id, session);
        Some(ToolCommandIntent::new(entry.id, params))
    }
}

fn format_schema_params_doc(schema: &Value, fallback: &str) -> String {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return fallback.to_owned();
    };
    if props.is_empty() {
        return String::from("{}");
    }
    let required: Vec<&str> =
        schema.get("required").and_then(Value::as_array).map(|arr| arr.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut parts = Vec::with_capacity(props.len());
    for (name, prop_schema) in props {
        let ty = prop_schema
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| prop_schema.get("type").and_then(Value::as_array).and_then(|arr| arr.iter().filter_map(Value::as_str).find(|&t| t != "null")))
            .unwrap_or("value");
        let req = if required.contains(&name.as_str()) { "" } else { "optional " };
        parts.push(format!("\"{name}\":\"{req}{ty}\""));
    }
    format!("{{{}}}", parts.join(","))
}

fn build_default_params(command_id: &str, session: Option<&Session>) -> Value {
    let Some(doc) = session.map(Session::document) else {
        return json!({});
    };
    let active_data = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()).or_else(|| doc.data_sheets.first().map(|s| &s.id));
    let active_result = doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()).or_else(|| doc.result_sheets.first().map(|s| &s.id));
    let active_graph = doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some()).or_else(|| doc.graph_sheets.first().map(|s| &s.id));
    let active_layout = doc.active_sheet_id.as_ref().filter(|id| doc.layout_sheet(id).is_some()).or_else(|| doc.layout_sheets.first().map(|s| &s.id));
    let active_renderable =
        doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()).or(active_graph).or(active_layout);
    let active_any = doc.active_sheet_id.as_ref().or(active_data).or(active_graph).or(active_result).or(active_layout);

    let mut obj = serde_json::Map::new();
    match command_id {
        "ui.brush.point" => {
            if let Some(id) = active_data {
                obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
                obj.insert(String::from("row"), json!(0));
                obj.insert(String::from("col"), json!(0));
                obj.insert(String::from("subcol"), json!(0));
            }
        }
        "ui.canvas.applyPreset" | "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => {
            let target = if command_id == "layout.applyCanvasPreset" { active_layout } else { active_renderable };
            if let Some(id) = target {
                obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
            }
            obj.insert(String::from("preset"), Value::String(String::from("SingleColumn88mm")));
            obj.insert(String::from("resize_mode"), Value::String(String::from("KeepTypographyPt")));
        }
        "ui.analyze.open" | "ui.grid.excludeWithReason" | "ui.table.convertArchetype.open" | "ui.graph.create.open" | "ui.table.openFormulaBar" => {
            if let Some(id) = active_data {
                obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
            }
        }
        "ui.editParams.open" | "ui.result.copyMethodsReport" | "ui.result.inspectDiagnostics" | "ui.result.appendClusterColumn" => {
            if let Some(id) = active_result {
                obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
            }
        }
        "ui.graph.axes.open"
        | "ui.graph.format.open"
        | "ui.changeGraphType.open"
        | "ui.formatPoints.open"
        | "ui.applyStyleFrom.open"
        | "ui.graph.pairwise.open"
        | "ui.graph.bubblePlot.open"
        | "ui.graph.insertModelEquation" => {
            if let Some(id) = active_graph {
                obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
            }
        }
        "ui.layout.composer.open" | "ui.layout.equalizeScaling.open" => {
            if let Some(id) = active_layout {
                obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
            }
        }
        "ui.canvas.preflight" | "ui.export.figure.open" | "ui.embeddedTable.format.open" | "ui.image.format.open" => {
            if let Some(id) = active_renderable {
                obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
            }
        }
        "ui.clonePipeline.open" | "ui.print.open" => {
            if let Some(id) = active_any {
                obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
            }
        }
        _ => {
            if let Some(spec) = cmd::registry().get(command_id)
                && let Some(props) = (spec.input_schema)().get("properties").and_then(Value::as_object)
            {
                let preferred = if command_id.starts_with("graph.") {
                    active_graph
                } else if command_id.starts_with("layout.") {
                    active_layout
                } else if command_id.starts_with("analysis.") || command_id.starts_with("result.") {
                    active_result
                } else if command_id.starts_with("table.") || command_id.starts_with("transform.") {
                    active_data
                } else if command_id.starts_with("export.") {
                    active_renderable
                } else {
                    active_any
                };
                if props.contains_key("sheet_id")
                    && let Some(id) = preferred.or(active_any)
                {
                    obj.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
                }
                if props.contains_key("data_sheet_id")
                    && let Some(id) = active_data
                {
                    obj.insert(String::from("data_sheet_id"), Value::String(id.as_str().to_owned()));
                }
                if props.contains_key("source_sheet_id")
                    && let Some(id) = active_data
                {
                    obj.insert(String::from("source_sheet_id"), Value::String(id.as_str().to_owned()));
                }
                if props.contains_key("result_sheet_id")
                    && let Some(id) = active_result
                {
                    obj.insert(String::from("result_sheet_id"), Value::String(id.as_str().to_owned()));
                }
            }
        }
    }
    Value::Object(obj)
}

fn score_command(query: &str, id: &str, label: &str, menu_path: &str, shortcut: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q = query.to_ascii_lowercase();
    let best_id = fuzzy_score(&q, &id.to_ascii_lowercase());
    let best_label = fuzzy_score(&q, &label.to_ascii_lowercase());
    let best_menu = fuzzy_score(&q, &menu_path.to_ascii_lowercase());
    let best_shortcut = fuzzy_score(&q, &shortcut.to_ascii_lowercase());
    [best_label.map(|s| s + 25), best_id.map(|s| s + 20), best_menu, best_shortcut].into_iter().flatten().max()
}

fn fuzzy_score(needle: &str, haystack: &str) -> Option<i32> {
    if haystack == needle {
        return Some(10_000);
    }
    if haystack.starts_with(needle) {
        return Some(5_000);
    }
    if let Some(pos) = haystack.find(needle) {
        let bonus =
            if pos == 0 || haystack.as_bytes().get(pos.saturating_sub(1)).is_some_and(|b| !b.is_ascii_alphanumeric()) { 2_500 } else { 1_500 };
        return Some(bonus - (pos as i32));
    }
    let mut score = 0i32;
    let mut prev_matched = false;
    let mut n_iter = needle.chars().peekable();
    for (idx, ch) in haystack.chars().enumerate() {
        let Some(&target) = n_iter.peek() else {
            break;
        };
        if ch == target {
            score += if prev_matched { 45 } else { 15 };
            if idx == 0 {
                score += 30;
            }
            prev_matched = true;
            let _ = n_iter.next();
        } else {
            prev_matched = false;
            score -= 1;
        }
    }
    n_iter.peek().is_none().then_some(score)
}
