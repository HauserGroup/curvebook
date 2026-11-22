//! Scrollable multi-column results viewer state and live clipboard/selection actions (`R10.5`).

use std::collections::BTreeMap;

use curvebook_doc::{DataSheet, ResultCell, ResultTab, ResultTabKind};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::ResultSheetViewState;
use crate::grid_view::format_cell_display;
use crate::interaction::UiEventOutcome;

/// Active section tab inside the expandable Interpret & Diagnostics drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InterpretDrawerTab {
    #[default]
    Assumptions,
    Narrative,
    Methods,
}

/// Transient view state owned by the results viewer shard (`R10.5`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResultsViewState {
    pub filter_text: String,
    #[serde(default)]
    pub bound_sheet_id: Option<String>,
    #[serde(default)]
    pub bound_tab_idx: Option<usize>,
    #[serde(default)]
    pub selected_anchor: Option<(usize, usize)>,
    #[serde(default)]
    pub selected_focus: Option<(usize, usize)>,
    #[serde(default)]
    pub interpret_open_by_sheet: BTreeMap<String, bool>,
    #[serde(default)]
    pub interpret_tab: InterpretDrawerTab,
    #[serde(default)]
    pub last_copied_tsv: Option<String>,
}

impl ResultsViewState {
    /// Effective Interpret drawer open state for `sheet_id` (collapsed by default until toggled).
    #[must_use]
    pub fn is_interpret_open(&self, sheet_id: &str) -> bool {
        self.interpret_open_by_sheet.get(sheet_id).copied().unwrap_or(false)
    }

    /// Active rectangular selection `(min_row, max_row, min_col, max_col)` if bound to `(sheet_id, tab_idx)`.
    #[must_use]
    pub fn selection_bounds(&self, sheet_id: &str, tab_idx: usize) -> Option<(usize, usize, usize, usize)> {
        if self.bound_sheet_id.as_deref() != Some(sheet_id) || self.bound_tab_idx != Some(tab_idx) {
            return None;
        }
        let a = self.selected_anchor?;
        let b = self.selected_focus.unwrap_or(a);
        Some((a.0.min(b.0), a.0.max(b.0), a.1.min(b.1), a.1.max(b.1)))
    }
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `results` domain.
///
/// # Errors
/// Returns an error if a requested result sheet cannot be resolved or parameter dialog fails.
pub fn handle_custom_action(app: &mut UiApp, state: &mut ResultsViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if let Some(rest) = payload.strip_prefix("select_cell:") {
        return Ok(handle_select_cell_action(state, rest));
    }
    if let Some(sheet_id) = payload.strip_prefix("copy_selection:") {
        return Ok(handle_copy_action(app, state, sheet_id, false));
    }
    if let Some(sheet_id) = payload.strip_prefix("copy_table:") {
        return Ok(handle_copy_action(app, state, sheet_id, true));
    }
    Ok(UiEventOutcome::default())
}

fn handle_select_cell_action(state: &mut ResultsViewState, rest: &str) -> UiEventOutcome {
    let mut parts = rest.splitn(4, ':');
    let (Some(t_s), Some(r_s), Some(c_s), Some(sheet_id)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return UiEventOutcome::default();
    };
    let (Ok(tab_idx), Ok(row), Ok(col)) = (t_s.parse::<usize>(), r_s.parse::<usize>(), c_s.parse::<usize>()) else {
        return UiEventOutcome::default();
    };
    state.bound_sheet_id = Some(sheet_id.to_owned());
    state.bound_tab_idx = Some(tab_idx);
    state.selected_anchor = Some((row, col));
    state.selected_focus = Some((row, col));
    UiEventOutcome {
        handled: true,
        action: format!("results.select_cell:{row}:{col}"),
        command_id: None,
        command_result: Some(json!({ "sheet_id": sheet_id, "tab_index": tab_idx, "row": row, "col": col })),
    }
}

fn handle_copy_action(app: &mut UiApp, state: &mut ResultsViewState, sheet_id: &str, force_full: bool) -> UiEventOutcome {
    let (source_sheet_id, tab_id, tsv) = {
        let Some(rs) = app.document().result_sheets.iter().find(|r| r.id.as_str() == sheet_id) else {
            return UiEventOutcome::default();
        };
        let requested_idx = if !force_full && state.bound_sheet_id.as_deref() == Some(sheet_id) {
            state.bound_tab_idx.unwrap_or(app.canvas_dialogs.result_view.active_tab_index)
        } else {
            app.canvas_dialogs.result_view.active_tab_index
        };
        let fallback_tab =
            rs.tabs.is_empty().then(|| rs.analyzable_data.as_ref().map(|ds| datasheet_to_preview_tab(ds, &app.locale_preset))).flatten();
        let (resolved_idx, tab_ref) = if let Some(tab) = rs.tabs.get(requested_idx) {
            (requested_idx, Some(tab))
        } else if let Some(tab) = rs.tabs.first() {
            (0, Some(tab))
        } else {
            (0, fallback_tab.as_ref())
        };
        let Some(tab) = tab_ref else {
            return UiEventOutcome::default();
        };
        let sel = if force_full { None } else { state.selection_bounds(sheet_id, resolved_idx) };
        let tsv = format_tab_selection_tsv(tab, rs.analysis.p_value_format, sel);
        (rs.id.clone(), tab.id.clone(), (tsv, sel))
    };
    let (tsv, sel) = tsv;
    state.last_copied_tsv = Some(tsv.clone());
    app.interaction.last_clipboard_bundle = None;
    app.interaction.last_clipboard_image_png = None;
    app.interaction.last_clipboard_text = Some(tsv.clone());
    app.interaction.last_copied_result_table = Some(crate::interaction::ResultTableClipboard {
        source_sheet_id,
        tab_id: Some(tab_id),
        selected_rows: sel.map(|(r0, r1, _, _)| (r0..=r1).collect()),
        selected_cols: sel.map(|(_, _, c0, c1)| (c0..=c1).collect()),
    });
    let _ = app.write_clipboard_text(&tsv);

    UiEventOutcome {
        handled: true,
        action: if force_full { String::from("results.copy_table") } else { String::from("results.copy_selection") },
        command_id: None,
        command_result: Some(json!({ "sheet_id": sheet_id, "copied_text": tsv, "chars": tsv.len() })),
    }
}

fn format_tab_selection_tsv(tab: &ResultTab, p_fmt: curvebook_doc::PValueFormat, sel: Option<(usize, usize, usize, usize)>) -> String {
    let num_cols = tab_column_count(tab);
    if tab.rows.is_empty() || num_cols == 0 {
        return String::new();
    }
    let max_r = tab.rows.len().saturating_sub(1);
    let max_c = num_cols.saturating_sub(1);
    let (r0, r1, c0, c1) = sel.map_or((0, max_r, 0, max_c), |(a, b, c, d)| (a.min(max_r), b.min(max_r), c.min(max_c), d.min(max_c)));

    if r0 == r1 && c0 == c1 {
        let cell = tab.rows.get(r0).and_then(|row| row.get(c0)).unwrap_or(&ResultCell::Empty);
        return ResultSheetViewState::format_result_cell(cell, p_fmt);
    }

    let include_headers = r0 == 0 && r1 == max_r && c1 > c0;
    let include_row_labels = c0 == 0 && c1 == max_c;
    let mut lines = Vec::new();
    if include_headers {
        let mut hdr_parts = Vec::new();
        if include_row_labels {
            hdr_parts.push(String::from("Row"));
        }
        for c in c0..=c1 {
            hdr_parts.push(tab_column_header(tab, c));
        }
        lines.push(hdr_parts.join("\t"));
    } else if r1 > r0 && c0 == c1 {
        lines.push(tab_column_header(tab, c0));
    }

    for r in r0..=r1 {
        let mut parts = Vec::new();
        if include_row_labels {
            parts.push(tab.row_labels.get(r).cloned().unwrap_or_else(|| format!("Row {}", r + 1)));
        }
        for c in c0..=c1 {
            let cell = tab.rows.get(r).and_then(|row| row.get(c)).unwrap_or(&ResultCell::Empty);
            parts.push(ResultSheetViewState::format_result_cell(cell, p_fmt));
        }
        lines.push(parts.join("\t"));
    }
    lines.join("\n")
}

fn datasheet_to_preview_tab(ds: &DataSheet, preset: &crate::i18n::LocalePreset) -> ResultTab {
    let (row_total, col_total) = crate::inspect::table_dimensions_summary(&ds.archetype);
    let col_count = col_total.max(1);
    let headers: Vec<String> =
        (0..col_count).map(|c| crate::dialogs_canvas::data_column_title(ds, c).map_or_else(|| format!("Col {}", c + 1), str::to_owned)).collect();
    let row_count = row_total.min(200);
    let mut row_labels = Vec::with_capacity(row_count);
    let mut rows = Vec::with_capacity(row_count);
    for r in 0..row_count {
        row_labels.push(format!("Row {}", r + 1));
        let mut row = Vec::with_capacity(col_count);
        for c in 0..col_count {
            let txt = ds.cell(curvebook_doc::CellCoord::new(r, c, 0)).map(|cell| format_cell_display(cell, preset)).unwrap_or_default();
            row.push(if txt.is_empty() { ResultCell::Empty } else { ResultCell::Text(txt) });
        }
        rows.push(row);
    }
    ResultTab {
        id: String::from("analyzable-data"),
        title: ds.title.clone(),
        kind: ResultTabKind::MainSummary,
        column_headers: headers,
        row_labels,
        rows,
        footnotes: Vec::new(),
    }
}

fn tab_column_count(tab: &ResultTab) -> usize {
    tab.column_headers.len().max(tab.rows.iter().map(Vec::len).max().unwrap_or(0))
}

fn tab_column_header(tab: &ResultTab, c_idx: usize) -> String {
    tab.column_headers.get(c_idx).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Col {}", c_idx + 1))
}
