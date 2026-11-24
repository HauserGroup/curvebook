//! Non-blocking floating in-sheet Find bar and matching engine for `DataSheet` and `ResultSheet` (`R15`).
//!
//! Anchored top-right inside the active sheet viewport (`Order::Foreground`, `ZLayer::FloatingOverlay`)
//! with no backdrop scrim so users can continue inspecting or editing cells while navigating matches.

use std::collections::{BTreeSet, HashSet};

use curvebook_doc::{CellCoord, DataSheet, ResultCell, ResultSheet, SheetId, TableArchetype, default_column_label};
use curvebook_tools::GridCellCoord;
use egui::{CornerRadius, Id, Key, Modifiers, Order, Rect, RichText, Stroke, TextEdit, Ui, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::dialogs_canvas::ResultSheetViewState;
use crate::egui_shell::data_grid::DataGridFindHighlight;
use crate::egui_shell::results_view::ResultsFindHighlight;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
use crate::grid_view::{data_set_subcolumn_count, format_cell_display};
use crate::i18n::LocalePreset;
use crate::inspect::ZLayer;

/// Persistent UI state for the floating in-sheet Find bar (`Cmd/Ctrl+F`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SheetFindBarState {
    pub open: bool,
    pub query: String,
    pub match_case: bool,
    pub match_entire_cell: bool,
    pub all_result_tabs: bool,
    pub active_match_index: usize,
    pub request_focus: bool,
    pub last_sheet_id: Option<SheetId>,
    pub last_match_count: usize,
    pub pending_scroll_to_active: bool,
}

impl SheetFindBarState {
    /// Open the find bar for `sheet_id` and focus the query input.
    pub fn open_for_sheet(&mut self, sheet_id: Option<SheetId>) {
        let sheet_changed = self.last_sheet_id != sheet_id;
        self.open = true;
        self.request_focus = true;
        if sheet_changed {
            self.last_sheet_id = sheet_id;
            self.active_match_index = 0;
        }
        if !self.query.is_empty() {
            self.pending_scroll_to_active = true;
        }
    }

    /// Close the find bar without clearing the last query so reopening restores it.
    pub fn close(&mut self) {
        self.open = false;
        self.request_focus = false;
        self.pending_scroll_to_active = false;
    }

    /// Clamp `active_match_index` against `match_count` and update `last_match_count`.
    pub fn sync_match_count(&mut self, match_count: usize) {
        self.last_match_count = match_count;
        if match_count == 0 {
            self.active_match_index = 0;
        } else if self.active_match_index >= match_count {
            self.active_match_index %= match_count;
        }
    }

    /// Advance to the next match (wrapping around) and request scrolling to it.
    pub fn step_next(&mut self, match_count: usize) {
        self.last_match_count = match_count;
        if match_count == 0 {
            self.active_match_index = 0;
            return;
        }
        self.active_match_index = (self.active_match_index + 1) % match_count;
        self.pending_scroll_to_active = true;
    }

    /// Move to the previous match (wrapping around) and request scrolling to it.
    pub fn step_prev(&mut self, match_count: usize) {
        self.last_match_count = match_count;
        if match_count == 0 {
            self.active_match_index = 0;
            return;
        }
        self.active_match_index = if self.active_match_index == 0 { match_count - 1 } else { self.active_match_index - 1 };
        self.pending_scroll_to_active = true;
    }
}

/// Single find match inside a [`DataSheet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataSheetFindMatch {
    Cell(GridCellCoord),
    ColumnTitle { col: usize },
    RowTitle { row: usize },
}

/// Single find match inside a [`ResultSheet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResultSheetFindMatch {
    pub tab_idx: usize,
    pub row_idx: usize,
    pub col_idx: usize,
}

/// Fast O(1) lookup overlay for highlighting visible [`DataSheet`] find matches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataSheetFindOverlay {
    pub active: Option<DataSheetFindMatch>,
    pub cells: HashSet<GridCellCoord>,
    pub col_titles: HashSet<usize>,
    pub row_titles: HashSet<usize>,
}

impl DataSheetFindOverlay {
    /// Build an O(1) highlight overlay from `matches` and `active_index`.
    #[must_use]
    pub fn from_matches(matches: &[DataSheetFindMatch], active_index: usize) -> Self {
        let mut overlay = Self::default();
        if matches.is_empty() {
            return overlay;
        }
        overlay.active = matches.get(active_index % matches.len()).copied();
        for &m in matches {
            match m {
                DataSheetFindMatch::Cell(coord) => {
                    overlay.cells.insert(coord);
                }
                DataSheetFindMatch::ColumnTitle { col } => {
                    overlay.col_titles.insert(col);
                }
                DataSheetFindMatch::RowTitle { row } => {
                    overlay.row_titles.insert(row);
                }
            }
        }
        overlay
    }

    /// Returns `true` when there are no matches in this overlay.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.col_titles.is_empty() && self.row_titles.is_empty()
    }

    /// Convert into [`DataGridFindHighlight`] for `ColumnGridEguiState::active_find_matches`.
    #[must_use]
    pub fn to_grid_highlight(&self) -> DataGridFindHighlight {
        let (active_cell, active_col_title, active_row_title) = match self.active {
            Some(DataSheetFindMatch::Cell(c)) => (Some(c), None, None),
            Some(DataSheetFindMatch::ColumnTitle { col }) => (None, Some(col), None),
            Some(DataSheetFindMatch::RowTitle { row }) => (None, None, Some(row)),
            None => (None, None, None),
        };
        DataGridFindHighlight {
            active_cell,
            active_col_title,
            active_row_title,
            matching_cells: self.cells.clone(),
            matching_col_titles: self.col_titles.clone(),
            matching_row_titles: self.row_titles.clone(),
        }
    }
}

/// Fast O(1) lookup overlay for highlighting visible [`ResultSheet`] find matches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultSheetFindOverlay {
    pub active: Option<ResultSheetFindMatch>,
    pub matches: HashSet<ResultSheetFindMatch>,
}

impl ResultSheetFindOverlay {
    /// Build an O(1) highlight overlay from `matches` and `active_index`.
    #[must_use]
    pub fn from_matches(matches: &[ResultSheetFindMatch], active_index: usize) -> Self {
        let mut overlay = Self::default();
        if matches.is_empty() {
            return overlay;
        }
        overlay.active = matches.get(active_index % matches.len()).copied();
        overlay.matches.extend(matches.iter().copied());
        overlay
    }

    /// Returns `true` when there are no matches in this overlay.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Project this overlay onto `active_tab_idx` for `ResultsViewEguiState::active_find_matches`.
    #[must_use]
    pub fn to_results_highlight(&self, active_tab_idx: usize) -> ResultsFindHighlight {
        let active_match = self.active.filter(|m| m.tab_idx == active_tab_idx).map(|m| (m.row_idx, m.col_idx));
        let matching_cells: BTreeSet<(usize, usize)> =
            self.matches.iter().filter(|m| m.tab_idx == active_tab_idx).map(|m| (m.row_idx, m.col_idx)).collect();
        ResultsFindHighlight { tab_idx: active_tab_idx, active_match, matching_cells }
    }
}

fn text_matches(haystack: &str, needle: &str, needle_lower: Option<&str>, match_case: bool, match_entire_cell: bool) -> bool {
    if haystack.is_empty() || needle.is_empty() {
        return false;
    }
    if match_case {
        if match_entire_cell { haystack == needle } else { haystack.contains(needle) }
    } else {
        let h_lower = haystack.to_lowercase();
        let n_lower = needle_lower.unwrap_or(needle);
        if match_entire_cell { h_lower == n_lower } else { h_lower.contains(n_lower) }
    }
}

fn data_sheet_column_title(arch: &TableArchetype, col: usize) -> String {
    match arch {
        TableArchetype::Column(t) => {
            let raw = t.data_sets.get(col).map_or("", |d| d.title.as_str());
            if raw.is_empty() { default_column_label(col) } else { raw.to_owned() }
        }
        TableArchetype::Xy(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::Survival(t) => t.cohorts.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col).map_or_else(String::new, |v| v.title.clone()),
        TableArchetype::Nested(t) => t.groups.get(col).map_or_else(String::new, |g| g.title.clone()),
    }
}

fn data_sheet_row_titles(arch: &TableArchetype) -> &[String] {
    match arch {
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    }
}

/// Find all matches across column titles, row titles, and cell values of `sheet`.
#[must_use]
pub fn find_matches_in_data_sheet(sheet: &DataSheet, query: &str, match_case: bool, match_entire_cell: bool) -> Vec<DataSheetFindMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle_lower = (!match_case).then(|| query.to_lowercase());
    let needle_lower_ref = needle_lower.as_deref();
    let mut out = Vec::new();

    let col_count = sheet.archetype.column_count();
    for col in 0..col_count {
        let title = data_sheet_column_title(&sheet.archetype, col);
        if text_matches(&title, query, needle_lower_ref, match_case, match_entire_cell) {
            out.push(DataSheetFindMatch::ColumnTitle { col });
        }
    }

    for (row, title) in data_sheet_row_titles(&sheet.archetype).iter().enumerate() {
        if text_matches(title, query, needle_lower_ref, match_case, match_entire_cell) {
            out.push(DataSheetFindMatch::RowTitle { row });
        }
    }

    let locale_preset = LocalePreset::default();
    let row_count = sheet.archetype.row_count();
    for row in 0..row_count {
        for col in 0..col_count {
            let subcols = data_set_subcolumn_count(&sheet.archetype, col).max(1);
            for subcol in 0..subcols {
                let Some(cell) = sheet.archetype.data_cell(CellCoord::new(row, col, subcol)) else {
                    continue;
                };
                let formatted = format_cell_display(cell, &locale_preset);
                if text_matches(&formatted, query, needle_lower_ref, match_case, match_entire_cell) {
                    out.push(DataSheetFindMatch::Cell(GridCellCoord::new(row, col, subcol)));
                }
            }
        }
    }

    out
}

/// Find all matches across formatted [`ResultCell`] values and row/column headers in `sheet`.
///
/// Searches only `active_tab_idx` when `all_tabs == false`, or every tab when `all_tabs == true`.
#[must_use]
pub fn find_matches_in_result_sheet(
    sheet: &ResultSheet,
    active_tab_idx: usize,
    all_tabs: bool,
    query: &str,
    match_case: bool,
    match_entire_cell: bool,
) -> Vec<ResultSheetFindMatch> {
    if query.is_empty() || sheet.tabs.is_empty() {
        return Vec::new();
    }
    let needle_lower = (!match_case).then(|| query.to_lowercase());
    let needle_lower_ref = needle_lower.as_deref();
    let tab_range = if all_tabs {
        0..sheet.tabs.len()
    } else {
        let idx = active_tab_idx.min(sheet.tabs.len().saturating_sub(1));
        idx..(idx + 1)
    };

    let mut out = Vec::new();
    for tab_idx in tab_range {
        let Some(tab) = sheet.tabs.get(tab_idx) else {
            continue;
        };
        let mut seen: HashSet<(usize, usize)> = HashSet::new();

        for (col_idx, header) in tab.column_headers.iter().enumerate() {
            if text_matches(header, query, needle_lower_ref, match_case, match_entire_cell) && seen.insert((0, col_idx)) {
                out.push(ResultSheetFindMatch { tab_idx, row_idx: 0, col_idx });
            }
        }

        for (row_idx, row) in tab.rows.iter().enumerate() {
            if let Some(row_label) = tab.row_labels.get(row_idx)
                && text_matches(row_label, query, needle_lower_ref, match_case, match_entire_cell)
                && seen.insert((row_idx, 0))
            {
                out.push(ResultSheetFindMatch { tab_idx, row_idx, col_idx: 0 });
            }
            for (col_idx, cell) in row.iter().enumerate() {
                if matches!(cell, ResultCell::Empty) {
                    continue;
                }
                let formatted = ResultSheetViewState::format_result_cell(cell, sheet.analysis.p_value_format);
                if text_matches(&formatted, query, needle_lower_ref, match_case, match_entire_cell) && seen.insert((row_idx, col_idx)) {
                    out.push(ResultSheetFindMatch { tab_idx, row_idx, col_idx });
                }
            }
        }
    }

    out
}

struct FindWidgetMeta<'a> {
    widget_id: &'a str,
    role: &'a str,
    label: &'a str,
    value: Option<serde_json::Value>,
    enabled: bool,
}

fn record_find_widget(ui: &Ui, meta: &mut FrameInspectMeta, resp: &egui::Response, spec: FindWidgetMeta<'_>) {
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput {
            widget_id: spec.widget_id,
            role: spec.role,
            label: spec.label,
            command_id: None,
            value: spec.value,
            enabled: spec.enabled,
            disabled_reason: (!spec.enabled).then_some("no matches"),
            z_layer: ZLayer::FloatingOverlay,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn render_toggle_pill(
    ui: &mut Ui,
    widget_id: &str,
    label: &str,
    tooltip: &str,
    active: &mut bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) -> bool {
    let fill = if *active { tokens.active_bg } else { tokens.button_bg };
    let border = if *active { tokens.accent } else { tokens.button_border };
    let text_col = if *active { tokens.text_strong } else { tokens.text_primary };
    let btn = egui::Button::new(RichText::new(label).small().color(text_col))
        .fill(fill)
        .stroke(Stroke::new(tokens.metrics.stroke_thin_pt, border))
        .corner_radius(CornerRadius::same(tokens.metrics.radius_sm));
    let resp = ui.add_sized(vec2(0.0, 24.0), btn);
    let ak_label = tooltip.to_owned();
    let is_active = *active;
    resp.widget_info(move || WidgetInfo::selected(egui::Role::Button, true, is_active, ak_label.clone()));
    record_find_widget(ui, meta, &resp, FindWidgetMeta { widget_id, role: "button", label, value: Some(json!(is_active)), enabled: true });
    let clicked = resp.clicked();
    let _ = resp.on_hover_text(tooltip);
    if clicked {
        *active = !*active;
    }
    clicked
}

struct FindBarRowFlags {
    is_data_sheet: bool,
    is_result_sheet: bool,
    match_count: usize,
    query_id: Id,
}

struct FindBarRowOutcome {
    replace_requested: bool,
    query_or_toggle_changed: bool,
}

fn render_find_bar_controls(
    ui: &mut Ui,
    flags: FindBarRowFlags,
    state: &mut SheetFindBarState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> FindBarRowOutcome {
    let mut out = FindBarRowOutcome { replace_requested: false, query_or_toggle_changed: false };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        let query_resp = ui.add(TextEdit::singleline(&mut state.query).id(flags.query_id).desired_width(156.0).hint_text("Find in sheet…"));
        if state.request_focus {
            query_resp.request_focus();
            state.request_focus = false;
        }
        let q_val = state.query.clone();
        query_resp.widget_info(move || WidgetInfo::labeled(egui::Role::TextInput, true, q_val.clone()));
        record_find_widget(
            ui,
            meta,
            &query_resp,
            FindWidgetMeta { widget_id: "sheet_find.query", role: "textbox", label: "Find in sheet", value: Some(json!(state.query)), enabled: true },
        );
        if query_resp.changed() {
            out.query_or_toggle_changed = true;
        }

        let match_count = flags.match_count;
        let count_text = if match_count == 0 { String::from("0 / 0") } else { format!("{} / {match_count}", state.active_match_index + 1) };
        let count_col = if !state.query.is_empty() && match_count == 0 { tokens.error } else { tokens.text_muted };
        let count_resp = ui.add(egui::Label::new(RichText::new(&count_text).small().monospace().color(count_col)));
        record_find_widget(
            ui,
            meta,
            &count_resp,
            FindWidgetMeta { widget_id: "sheet_find.count", role: "label", label: &count_text, value: Some(json!(match_count)), enabled: true },
        );

        let has_matches = match_count > 0;
        let prev_resp = ui.add_enabled(has_matches, egui::Button::new(RichText::new("↑").small()).min_size(vec2(24.0, 24.0)));
        prev_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, has_matches, String::from("Previous Match")));
        record_find_widget(
            ui,
            meta,
            &prev_resp,
            FindWidgetMeta { widget_id: "sheet_find.prev", role: "button", label: "↑", value: None, enabled: has_matches },
        );
        if prev_resp.clicked() {
            state.step_prev(match_count);
        }
        let _ = prev_resp.on_hover_text("Previous match (Shift+Enter)");

        let next_resp = ui.add_enabled(has_matches, egui::Button::new(RichText::new("↓").small()).min_size(vec2(24.0, 24.0)));
        next_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, has_matches, String::from("Next Match")));
        record_find_widget(
            ui,
            meta,
            &next_resp,
            FindWidgetMeta { widget_id: "sheet_find.next", role: "button", label: "↓", value: None, enabled: has_matches },
        );
        if next_resp.clicked() {
            state.step_next(match_count);
        }
        let _ = next_resp.on_hover_text("Next match (Enter)");

        if render_toggle_pill(ui, "sheet_find.match_case", "Aa", "Match case", &mut state.match_case, tokens, meta) {
            out.query_or_toggle_changed = true;
        }
        if render_toggle_pill(ui, "sheet_find.entire_cell", "[]", "Match entire cell", &mut state.match_entire_cell, tokens, meta) {
            out.query_or_toggle_changed = true;
        }
        if flags.is_result_sheet
            && render_toggle_pill(ui, "sheet_find.all_tabs", "All Tabs", "Search across all result tabs", &mut state.all_result_tabs, tokens, meta)
        {
            out.query_or_toggle_changed = true;
        }
        if flags.is_data_sheet {
            let repl_resp = ui.add_sized(vec2(0.0, 24.0), egui::Button::new(RichText::new("Replace…").small()));
            repl_resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, String::from("Replace")));
            record_find_widget(
                ui,
                meta,
                &repl_resp,
                FindWidgetMeta { widget_id: "sheet_find.replace", role: "button", label: "Replace…", value: None, enabled: true },
            );
            if repl_resp.clicked() {
                let initial_query = (!state.query.is_empty()).then(|| state.query.clone());
                intents.push(UiIntent::OpenFindReplace { initial_query });
                out.replace_requested = true;
            }
        }

        let close_resp = ui.add_sized(vec2(24.0, 24.0), egui::Button::new(RichText::new("×").small()));
        close_resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, String::from("Close Find Bar")));
        record_find_widget(
            ui,
            meta,
            &close_resp,
            FindWidgetMeta { widget_id: "sheet_find.close", role: "button", label: "×", value: None, enabled: true },
        );
        if close_resp.clicked() {
            state.close();
        }
        let _ = close_resp.on_hover_text("Close (Esc)");
    });
    out
}

fn handle_find_bar_keys(ctx: &egui::Context, query_focused: bool, match_count: usize, state: &mut SheetFindBarState) {
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        state.close();
        return;
    }
    if !query_focused {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Enter)) {
        state.step_prev(match_count);
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        state.step_next(match_count);
    }
}

/// Render the non-blocking floating in-sheet Find bar anchored top-right inside `anchor_rect`.
///
/// Returns `true` when the user triggers the `Replace…` handoff (`UiIntent::OpenFindReplace`).
#[allow(clippy::too_many_arguments)]
pub fn render_sheet_find_bar(
    ctx: &egui::Context,
    anchor_rect: Rect,
    is_data_sheet: bool,
    is_result_sheet: bool,
    match_count: usize,
    state: &mut SheetFindBarState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> bool {
    if !state.open || !anchor_rect.is_positive() {
        return false;
    }
    state.sync_match_count(match_count);

    let query_id = Id::unique("egui_shell.sheet_find.query");
    let query_focused = ctx.memory(|m| m.focused()) == Some(query_id);
    handle_find_bar_keys(ctx, query_focused, match_count, state);
    if !state.open {
        return false;
    }

    let est_w = if is_data_sheet || is_result_sheet { 480.0_f32 } else { 400.0_f32 };
    let bar_w = est_w.min((anchor_rect.width() - 16.0).max(240.0));
    let pos_x = (anchor_rect.right() - bar_w - 12.0).max(anchor_rect.left() + 8.0);
    let pos_y = anchor_rect.top() + 8.0;

    let mut outcome = FindBarRowOutcome { replace_requested: false, query_or_toggle_changed: false };
    let area_id = Id::unique("egui_shell.sheet_find.area");
    egui::Area::new(area_id).fixed_pos(pos2(pos_x, pos_y)).order(Order::Foreground).show(ctx, |ui| {
        egui::Frame::popup(ui.style())
            .fill(tokens.panel_bg)
            .stroke(Stroke::new(tokens.metrics.stroke_thin_pt, tokens.border))
            .corner_radius(CornerRadius::same(tokens.metrics.radius_lg))
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                outcome = render_find_bar_controls(
                    ui,
                    FindBarRowFlags { is_data_sheet, is_result_sheet, match_count, query_id },
                    state,
                    tokens,
                    meta,
                    intents,
                );
            });
    });

    if outcome.query_or_toggle_changed {
        state.active_match_index = 0;
        if !state.query.is_empty() {
            state.pending_scroll_to_active = true;
        }
    }

    outcome.replace_requested
}

#[cfg(test)]
mod tests {
    use super::*;
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSetColumn, DescriptiveSpec, ResultStatus, ResultTab, ResultTabKind,
    };

    fn sample_data_sheet() -> DataSheet {
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).expect("column table");
        table.data_sets[0] = DataSetColumn {
            title: String::from("Control"),
            cells: vec![vec![Cell::number(12.5)], vec![Cell::text("Alpha")], vec![Cell::text("alpha")]],
        };
        table.data_sets[1] = DataSetColumn {
            title: String::from("Treated"),
            cells: vec![vec![Cell::number(125.0)], vec![Cell::text("Beta")], vec![Cell::text("AlphaBeta")]],
        };
        table.row_titles = vec![String::from("Baseline"), String::from("Peak"), String::from("Recovery")];
        DataSheet::new("ds_find", "Dose Table", TableArchetype::Column(table)).expect("data sheet")
    }

    fn sample_result_sheet() -> ResultSheet {
        let tab0 = ResultTab {
            id: String::from("summary"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Estimate"), String::from("P Value")],
            row_labels: vec![String::from("Slope"), String::from("Intercept")],
            rows: vec![
                vec![ResultCell::Number { value: 2.5, prefix: None, units: None }, ResultCell::Text(String::from("Significant"))],
                vec![ResultCell::Number { value: 0.75, prefix: None, units: None }, ResultCell::Text(String::from("ns"))],
            ],
            footnotes: Vec::new(),
        };
        let tab1 = ResultTab {
            id: String::from("diag"),
            title: String::from("Diagnostics"),
            kind: ResultTabKind::Diagnostics,
            column_headers: vec![String::from("Metric")],
            row_labels: vec![String::from("Slope Check")],
            rows: vec![vec![ResultCell::Text(String::from("Significant drift"))]],
            footnotes: Vec::new(),
        };
        let spec = AnalysisSpec::new("rs_find", "ds_find", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("analysis spec");
        let mut rs = ResultSheet::new(spec).expect("result sheet");
        rs.status = ResultStatus::UpToDate;
        rs.tabs = vec![tab0, tab1];
        rs
    }

    #[test]
    fn data_sheet_find_case_and_entire_cell() {
        let ds = sample_data_sheet();

        let ci_sub = find_matches_in_data_sheet(&ds, "alpha", false, false);
        assert_eq!(
            ci_sub,
            vec![
                DataSheetFindMatch::Cell(GridCellCoord::new(1, 0, 0)),
                DataSheetFindMatch::Cell(GridCellCoord::new(2, 0, 0)),
                DataSheetFindMatch::Cell(GridCellCoord::new(2, 1, 0)),
            ]
        );

        let cs_sub = find_matches_in_data_sheet(&ds, "Alpha", true, false);
        assert_eq!(cs_sub, vec![DataSheetFindMatch::Cell(GridCellCoord::new(1, 0, 0)), DataSheetFindMatch::Cell(GridCellCoord::new(2, 1, 0)),]);

        let cs_exact = find_matches_in_data_sheet(&ds, "Alpha", true, true);
        assert_eq!(cs_exact, vec![DataSheetFindMatch::Cell(GridCellCoord::new(1, 0, 0))]);

        let title_matches = find_matches_in_data_sheet(&ds, "control", false, true);
        assert_eq!(title_matches, vec![DataSheetFindMatch::ColumnTitle { col: 0 }]);

        let row_matches = find_matches_in_data_sheet(&ds, "peak", false, true);
        assert_eq!(row_matches, vec![DataSheetFindMatch::RowTitle { row: 1 }]);
    }

    #[test]
    fn result_sheet_find_active_tab_vs_all_tabs() {
        let rs = sample_result_sheet();

        let active_only = find_matches_in_result_sheet(&rs, 0, false, "significant", false, false);
        assert_eq!(active_only, vec![ResultSheetFindMatch { tab_idx: 0, row_idx: 0, col_idx: 1 }]);

        let all_tabs = find_matches_in_result_sheet(&rs, 0, true, "significant", false, false);
        assert_eq!(
            all_tabs,
            vec![ResultSheetFindMatch { tab_idx: 0, row_idx: 0, col_idx: 1 }, ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 0 },]
        );

        let exact = find_matches_in_result_sheet(&rs, 0, true, "Significant", true, true);
        assert_eq!(exact, vec![ResultSheetFindMatch { tab_idx: 0, row_idx: 0, col_idx: 1 }]);

        let row_hdr = find_matches_in_result_sheet(&rs, 0, true, "slope", false, false);
        assert_eq!(
            row_hdr,
            vec![ResultSheetFindMatch { tab_idx: 0, row_idx: 0, col_idx: 0 }, ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 0 },]
        );
    }

    #[test]
    fn wrap_around_navigation_and_overlays() {
        let mut state = SheetFindBarState::default();
        assert!(!state.all_result_tabs);
        state.step_next(3);
        assert_eq!(state.active_match_index, 1);
        assert!(state.pending_scroll_to_active);
        state.step_next(3);
        assert_eq!(state.active_match_index, 2);
        state.step_next(3);
        assert_eq!(state.active_match_index, 0);
        state.step_prev(3);
        assert_eq!(state.active_match_index, 2);

        let matches = vec![
            DataSheetFindMatch::ColumnTitle { col: 1 },
            DataSheetFindMatch::RowTitle { row: 2 },
            DataSheetFindMatch::Cell(GridCellCoord::new(0, 1, 0)),
        ];
        let overlay = DataSheetFindOverlay::from_matches(&matches, 2);
        let grid_hl = overlay.to_grid_highlight();
        assert_eq!(grid_hl.active_cell, Some(GridCellCoord::new(0, 1, 0)));
        assert!(grid_hl.matching_col_titles.contains(&1));
        assert!(grid_hl.matching_row_titles.contains(&2));
        assert!(grid_hl.matching_cells.contains(&GridCellCoord::new(0, 1, 0)));
    }
}
