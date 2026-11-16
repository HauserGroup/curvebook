//! Scrollable multi-column results viewer, overflow sub-tabs, Interpret / Assumption Checklist
//! drawer, diagnostic banners, and `Cmd/Ctrl+T` parameter re-entry (`R10.5`, `R9.7` diagnostics-only).

use std::collections::BTreeMap;

use curvebook_doc::{ChecklistItem, ChecklistStatus, DataSheet, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind};
use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{AssumptionDiagnosticBanner, ChecklistSeverityFilter, ResultSheetViewState};
use crate::dialogs_r9::sanitize_diagnostics_only_text;
use crate::grid_view::format_cell_display;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta, scrollbar_thickness_px};
use crate::layout::widgets::{ButtonWidgetSpec, FittedTextSpec, emit_button, emit_scrollbars_for_meta, emit_tab, paint_text_fitted, widget_state};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::{TextMeasurer, UiTextStyle};

use super::a11y::labeled_meta;

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

fn tr_res(locale: &str, key: &str, fallback: &'static str) -> &'static str {
    crate::i18n::try_tr(locale, key).unwrap_or(fallback)
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `results` domain.
///
/// # Errors
/// Returns an error if a requested result sheet cannot be resolved or parameter dialog fails.
pub fn handle_custom_action(app: &mut UiApp, state: &mut ResultsViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if let Some(sheet_id) = payload.strip_prefix("edit_params:") {
        return handle_edit_params_action(app, sheet_id);
    }
    if let Some(sheet_id) = payload.strip_prefix("toggle_interpret:") {
        return Ok(handle_toggle_interpret_action(app, state, sheet_id));
    }
    if let Some(tab_name) = payload.strip_prefix("interpret_tab:") {
        return Ok(handle_interpret_tab_action(state, tab_name));
    }
    if let Some(filter_mode) = payload.strip_prefix("checklist_filter:") {
        return Ok(handle_checklist_filter_action(app, filter_mode));
    }
    if let Some(fmt) = payload.strip_prefix("report_format:") {
        return Ok(handle_report_format_action(app, fmt));
    }
    if let Some(rest) = payload.strip_prefix("tabs_scroll:") {
        return Ok(handle_tabs_scroll_action(app, rest));
    }
    if let Some(rest) = payload.strip_prefix("select_cell:") {
        return Ok(handle_select_cell_action(state, rest));
    }
    if let Some(rest) = payload.strip_prefix("select_row:") {
        return Ok(handle_select_row_action(state, rest));
    }
    if let Some(rest) = payload.strip_prefix("select_col:") {
        return Ok(handle_select_col_action(state, rest));
    }
    if let Some(sheet_id) = payload.strip_prefix("copy_selection:") {
        return Ok(handle_copy_action(app, state, sheet_id, false));
    }
    if let Some(sheet_id) = payload.strip_prefix("copy_table:") {
        return Ok(handle_copy_action(app, state, sheet_id, true));
    }
    Ok(UiEventOutcome::default())
}

fn handle_edit_params_action(app: &mut UiApp, sheet_id: &str) -> Result<UiEventOutcome, String> {
    let rs_id = curvebook_doc::SheetId::try_new(sheet_id).map_err(|e| e.to_string())?;
    let doc = app.document().clone();
    if doc.result_sheet(&rs_id).is_none() {
        return Err(format!("result sheet `{}` not found", rs_id.as_str()));
    }
    app.canvas_dialogs.open_edit_parameters(&doc, &rs_id);
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("results.edit_params"),
        command_id: Some(String::from("ui.editParams.open")),
        command_result: Some(json!({ "dialog": "EditAnalysisParameters", "result_sheet_id": rs_id.as_str() })),
    })
}

fn handle_toggle_interpret_action(app: &mut UiApp, state: &mut ResultsViewState, sheet_id: &str) -> UiEventOutcome {
    let next = !state.is_interpret_open(sheet_id);
    state.interpret_open_by_sheet.insert(sheet_id.to_owned(), next);
    app.canvas_dialogs.result_view.checklist_expanded = next;
    UiEventOutcome {
        handled: true,
        action: format!("results.toggle_interpret:{next}"),
        command_id: None,
        command_result: Some(json!({ "sheet_id": sheet_id, "interpret_open": next })),
    }
}

fn handle_interpret_tab_action(state: &mut ResultsViewState, tab_name: &str) -> UiEventOutcome {
    state.interpret_tab = match tab_name {
        "narrative" => InterpretDrawerTab::Narrative,
        "methods" => InterpretDrawerTab::Methods,
        _ => InterpretDrawerTab::Assumptions,
    };
    UiEventOutcome { handled: true, action: format!("results.interpret_tab:{tab_name}"), ..Default::default() }
}

fn handle_checklist_filter_action(app: &mut UiApp, filter_mode: &str) -> UiEventOutcome {
    app.canvas_dialogs.result_view.checklist_filter =
        if filter_mode == "flagged" { ChecklistSeverityFilter::WarningsAndViolations } else { ChecklistSeverityFilter::All };
    UiEventOutcome { handled: true, action: format!("results.checklist_filter:{filter_mode}"), ..Default::default() }
}

fn handle_report_format_action(app: &mut UiApp, fmt: &str) -> UiEventOutcome {
    if matches!(fmt, "plain" | "markdown" | "latex") {
        app.canvas_dialogs.result_view.report_format = fmt.to_owned();
    }
    UiEventOutcome { handled: true, action: format!("results.report_format:{fmt}"), ..Default::default() }
}

fn handle_tabs_scroll_action(app: &mut UiApp, rest: &str) -> UiEventOutcome {
    let Some((raw_off, sheet_id)) = rest.split_once(':') else {
        return UiEventOutcome::default();
    };
    let Ok(offset_px) = raw_off.parse::<f64>() else {
        return UiEventOutcome::default();
    };
    let clean_x = if offset_px.is_finite() { offset_px.max(0.0) } else { 0.0 };
    for key in [format!("scroll.result.tabs.{sheet_id}"), format!("scroll.result.sec.tabs.{sheet_id}")] {
        let entry = app.interaction.scroll_offsets.offsets.entry(key).or_insert([0.0, 0.0]);
        entry[0] = clean_x;
    }
    UiEventOutcome {
        handled: true,
        action: String::from("results.tabs_scroll"),
        command_id: None,
        command_result: Some(json!({ "sheet_id": sheet_id, "offset_x": clean_x })),
    }
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

fn handle_select_row_action(state: &mut ResultsViewState, rest: &str) -> UiEventOutcome {
    let mut parts = rest.splitn(4, ':');
    let (Some(t_s), Some(r_s), Some(mc_s), Some(sheet_id)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return UiEventOutcome::default();
    };
    let (Ok(tab_idx), Ok(row), Ok(max_col)) = (t_s.parse::<usize>(), r_s.parse::<usize>(), mc_s.parse::<usize>()) else {
        return UiEventOutcome::default();
    };
    state.bound_sheet_id = Some(sheet_id.to_owned());
    state.bound_tab_idx = Some(tab_idx);
    state.selected_anchor = Some((row, 0));
    state.selected_focus = Some((row, max_col));
    UiEventOutcome {
        handled: true,
        action: format!("results.select_row:{row}"),
        command_id: None,
        command_result: Some(json!({ "sheet_id": sheet_id, "tab_index": tab_idx, "row": row, "max_col": max_col })),
    }
}

fn handle_select_col_action(state: &mut ResultsViewState, rest: &str) -> UiEventOutcome {
    let mut parts = rest.splitn(4, ':');
    let (Some(t_s), Some(c_s), Some(mr_s), Some(sheet_id)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return UiEventOutcome::default();
    };
    let (Ok(tab_idx), Ok(col), Ok(max_row)) = (t_s.parse::<usize>(), c_s.parse::<usize>(), mr_s.parse::<usize>()) else {
        return UiEventOutcome::default();
    };
    state.bound_sheet_id = Some(sheet_id.to_owned());
    state.bound_tab_idx = Some(tab_idx);
    state.selected_anchor = Some((0, col));
    state.selected_focus = Some((max_row, col));
    UiEventOutcome {
        handled: true,
        action: format!("results.select_col:{col}"),
        command_id: None,
        command_result: Some(json!({ "sheet_id": sheet_id, "tab_index": tab_idx, "col": col, "max_row": max_row })),
    }
}

fn handle_copy_action(app: &mut UiApp, state: &mut ResultsViewState, sheet_id: &str, force_full: bool) -> UiEventOutcome {
    let Some(rs) = app.document().result_sheets.iter().find(|r| r.id.as_str() == sheet_id) else {
        return UiEventOutcome::default();
    };
    let active_idx = app.canvas_dialogs.result_view.active_tab_index;
    let fallback_tab = rs.tabs.is_empty().then(|| rs.analyzable_data.as_ref().map(|ds| datasheet_to_preview_tab(ds, &app.locale_preset))).flatten();
    let tab_ref = rs.tabs.get(active_idx).or_else(|| rs.tabs.first()).or(fallback_tab.as_ref());
    let Some(tab) = tab_ref else {
        return UiEventOutcome::default();
    };

    let sel = if force_full { None } else { state.selection_bounds(sheet_id, active_idx) };
    let tsv = format_tab_selection_tsv(tab, rs.analysis.p_value_format, sel);
    state.last_copied_tsv = Some(tsv.clone());
    app.interaction.last_clipboard_text = Some(tsv.clone());

    #[cfg(not(target_arch = "wasm32"))]
    if !app.is_gui_only() {
        let _ = crate::os_integration::write_os_pasteboard(&tsv);
    }

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

fn scaled_px(scale: f64, base: f64) -> u32 {
    (base * scale.clamp(0.75, 4.0)).round() as u32
}

/// Build the scrollable multi-tab, multi-column [`ResultSheet`] viewer inside `rect`.
pub fn build_result_viewer_view(builder: &mut LayoutBuilder<'_>, rs: &ResultSheet, rect: LayoutRectPx, slot_tag: &str) {
    let box_rect = rect.inset(6);
    if box_rect.width < 60 || box_rect.height < 60 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, box_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, None, box_rect, builder.palette.border, 1);
    builder.push_clip(box_rect);

    let scale = builder.scale();
    let is_secondary = slot_tag == "secondary";
    let pfx = if is_secondary { "result.sec" } else { "result" };
    let drawer_open = builder.app.view_shards.results.is_interpret_open(rs.id.as_str());

    build_header_strip(builder, rs, box_rect, pfx, is_secondary, drawer_open);

    let active_tab_idx = if rs.tabs.is_empty() { 0 } else { builder.app.canvas_dialogs.result_view.active_tab_index.min(rs.tabs.len() - 1) };
    let hdr_h = scaled_px(scale, 24.0);
    let tab_h = scaled_px(scale, 24.0);
    let tab_y = box_rect.y + 4 + i32::try_from(hdr_h).unwrap_or(24) + 4;
    let tab_row_rect = LayoutRectPx::new(box_rect.x + 8, tab_y, box_rect.width.saturating_sub(16), tab_h);
    build_sub_tab_strip(builder, rs, tab_row_rect, active_tab_idx, pfx, is_secondary);

    let mut body_y = tab_row_rect.bottom() + 4;
    body_y = build_diagnostic_banner(builder, rs, box_rect, body_y, pfx, slot_tag);

    let avail_h = box_rect.bottom() - body_y - 6;
    if avail_h < 24 {
        builder.pop_clip();
        return;
    }
    let body_rect = LayoutRectPx::new(box_rect.x + 8, body_y, box_rect.width.saturating_sub(16), avail_h as u32);
    let (table_area, drawer_area) = compute_body_split(body_rect, drawer_open, builder.tree.is_rtl, scale);

    let fallback_tab =
        rs.tabs.is_empty().then(|| rs.analyzable_data.as_ref().map(|ds| datasheet_to_preview_tab(ds, &builder.app.locale_preset))).flatten();
    let active_tab = rs.tabs.get(active_tab_idx).or_else(|| rs.tabs.first()).or(fallback_tab.as_ref());

    if let Some(tab) = active_tab {
        let mut tbl_rect = table_area;
        let sec_h = scaled_px(scale, 20.0);
        let sec_step = i32::try_from(sec_h + 2).unwrap_or(22);
        if table_area.height >= sec_h * 4 + 12 {
            let sec_rect = LayoutRectPx::new(table_area.x, table_area.y, table_area.width, sec_h);
            build_table_section_bar(builder, rs, tab, sec_rect, active_tab_idx, pfx);
            tbl_rect = LayoutRectPx::new(table_area.x, table_area.y + sec_step, table_area.width, table_area.height.saturating_sub(sec_h + 2));
        }
        build_result_table(builder, rs, tab, tbl_rect, pfx, active_tab_idx);
    } else {
        build_empty_or_failed_body(builder, rs, table_area, pfx);
    }

    if let Some(d_rect) = drawer_area {
        build_interpret_drawer(builder, rs, d_rect, pfx, is_secondary);
    }

    builder.pop_clip();
}

fn build_header_strip(builder: &mut LayoutBuilder<'_>, rs: &ResultSheet, box_rect: LayoutRectPx, pfx: &str, is_secondary: bool, drawer_open: bool) {
    let scale = builder.scale();
    let size_px = (12.0 * scale).clamp(9.0, 22.0);
    let locale = &builder.tree.locale;
    let hdr_h = scaled_px(scale, 24.0);
    let y = box_rect.y + 4;
    let style = UiTextStyle::ui_sans(size_px);

    let params_lbl = if box_rect.width >= 440 { tr_res(locale, "results.parameters", "Parameters (Cmd+T)") } else { "Params" };
    let copy_lbl = tr_res(locale, "results.copy", "Copy");
    let (_, warn, viol, _) = ResultSheetViewState::checklist_summary_counts(rs);
    let interp_base = tr_res(locale, "results.interpret", "Interpret");
    let interp_lbl = if warn + viol > 0 { format!("{interp_base} ({}!) ", warn + viol).trim_end().to_owned() } else { interp_base.to_owned() };

    let measure_btn = |lbl: &str, min_w: u32, max_w: u32| -> u32 {
        (builder.measurer.measure(lbl, &style, locale).width_px.ceil() as u32 + 16).clamp(min_w, max_w)
    };
    let params_w = measure_btn(params_lbl, scaled_px(scale, 64.0), scaled_px(scale, 180.0));
    let interp_w = measure_btn(&interp_lbl, scaled_px(scale, 68.0), scaled_px(scale, 150.0));
    let copy_w = measure_btn(copy_lbl, scaled_px(scale, 48.0), scaled_px(scale, 110.0));

    let mut right_edge = box_rect.right() - 8;
    let min_title_right = box_rect.x + i32::try_from(scaled_px(scale, 84.0)).unwrap_or(84);

    if right_edge - i32::try_from(params_w).unwrap_or(64) > min_title_right {
        let rx = right_edge - i32::try_from(params_w).unwrap_or(64);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("{pfx}.edit_params_btn"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(rx, y, params_w, hdr_h),
                label: params_lbl,
                keyboard_path: "Cmd+T",
                focus_scope: FocusScope::Workspace,
                tab_order: if is_secondary { None } else { Some(500) },
                primary: false,
                selected: false,
                shortcut: Some("Cmd+T"),
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("results"), payload: format!("edit_params:{}", rs.id.as_str()) },
            },
        );
        right_edge = rx - 4;
    }

    if right_edge - i32::try_from(interp_w).unwrap_or(68) > min_title_right {
        let ix = right_edge - i32::try_from(interp_w).unwrap_or(68);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("{pfx}.interpret_toggle_btn"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(ix, y, interp_w, hdr_h),
                label: &interp_lbl,
                keyboard_path: "result.interpret",
                focus_scope: FocusScope::Workspace,
                tab_order: if is_secondary { None } else { Some(501) },
                primary: false,
                selected: drawer_open,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("results"), payload: format!("toggle_interpret:{}", rs.id.as_str()) },
            },
        );
        right_edge = ix - 4;
    }

    if right_edge - i32::try_from(copy_w).unwrap_or(48) > min_title_right {
        let cx = right_edge - i32::try_from(copy_w).unwrap_or(48);
        let can_copy = !rs.tabs.is_empty() || rs.analyzable_data.is_some();
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("{pfx}.copy_btn"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(cx, y, copy_w, hdr_h),
                label: copy_lbl,
                keyboard_path: "Cmd+C",
                focus_scope: FocusScope::Workspace,
                tab_order: if is_secondary { None } else { Some(502) },
                primary: false,
                selected: false,
                shortcut: Some("Cmd+C"),
                enablement: if can_copy { ControlEnablement::enabled() } else { ControlEnablement::disabled("No result table data to copy") },
                action: WidgetAction::Custom { domain: String::from("results"), payload: format!("copy_selection:{}", rs.id.as_str()) },
            },
        );
        right_edge = cx - 6;
    }

    let title_w = u32::try_from((right_edge - (box_rect.x + 8)).max(24)).unwrap_or(24);
    let hdr_rect = LayoutRectPx::new(box_rect.x + 8, y, title_w, hdr_h);
    let title_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{pfx}.title.{}", rs.id.as_str())),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: hdr_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, rs.title().to_owned(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &title_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: hdr_rect,
            pad_x: 2,
            text: rs.title(),
            style: &UiTextStyle::ui_bold(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
}

fn build_sub_tab_strip(
    builder: &mut LayoutBuilder<'_>,
    rs: &ResultSheet,
    tab_row_rect: LayoutRectPx,
    active_tab_idx: usize,
    pfx: &str,
    is_secondary: bool,
) {
    if rs.tabs.is_empty() || tab_row_rect.width < 40 {
        return;
    }
    let scale = builder.scale();
    let size_px = (11.5 * scale).clamp(9.0, 20.0);
    let tab_style = UiTextStyle::ui_sans(size_px);
    let locale = &builder.tree.locale;
    let min_tab_w = scaled_px(scale, 58.0);
    let max_tab_w = scaled_px(scale, 200.0);

    let mut tab_widths = Vec::with_capacity(rs.tabs.len());
    let mut total_tabs_w = 0u32;
    for (idx, tab) in rs.tabs.iter().enumerate() {
        let tw = (builder.measurer.measure(&tab.title, &tab_style, locale).width_px.ceil() as u32 + 16).clamp(min_tab_w, max_tab_w);
        tab_widths.push(tw);
        total_tabs_w = total_tabs_w.saturating_add(tw);
        if idx + 1 < rs.tabs.len() {
            total_tabs_w = total_tabs_w.saturating_add(4);
        }
    }

    let arrow_w = scaled_px(scale, 20.0);
    let inner_h = tab_row_rect.height.saturating_sub(2).max(12);
    let overflow = total_tabs_w > tab_row_rect.width && tab_row_rect.width >= arrow_w * 4 + 40;
    let strip_rect = if overflow {
        let side_pad = i32::try_from(arrow_w + 2).unwrap_or(22);
        LayoutRectPx::new(tab_row_rect.x + side_pad, tab_row_rect.y, tab_row_rect.width.saturating_sub((arrow_w + 2) * 2), tab_row_rect.height)
    } else {
        tab_row_rect
    };

    let scroll_id = format!("scroll.{pfx}.tabs.{}", rs.id.as_str());
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta =
        ScrollMeta::new(scroll_id.clone(), strip_rect, [total_tabs_w.max(strip_rect.width), strip_rect.height], scroll_off, ScrollAxes::Horizontal);

    let strip_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{pfx}.tab_strip")),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: strip_rect,
        a11y: labeled_meta(WidgetRole::Canvas, "Result Sub-Tabs", FocusScope::Workspace, "Tab / Scroll"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&strip_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::Workspace, Some(&strip_wid), strip_rect, builder.palette.panel);

    builder.push_clip(strip_rect);
    let x_scroll = scroll_meta.scroll_offset_px[0].round() as i32;
    let mut cum_x = 0i32;
    for (idx, (tab, &tw)) in rs.tabs.iter().zip(tab_widths.iter()).enumerate() {
        let tx = strip_rect.x + cum_x - x_scroll;
        cum_x += i32::try_from(tw + 4).unwrap_or(62);
        let tab_rect = LayoutRectPx::new(tx, strip_rect.y + 1, tw, inner_h);
        let vis = tab_rect.intersect(strip_rect);
        if vis.width < 16 || vis.height < 10 {
            continue;
        }
        emit_clipped_tab(builder, &strip_wid, tab, idx, tab_rect, vis, active_tab_idx == idx, pfx, is_secondary, &scroll_id);
    }
    builder.pop_clip();

    if overflow {
        let cur_x_f = scroll_meta.scroll_offset_px[0];
        let max_x = scroll_meta.max_scroll_px[0];
        let step = (f64::from(strip_rect.width) * 0.55).max(120.0);
        let prev_x = (cur_x_f - step).max(0.0);
        let next_x = (cur_x_f + step).min(max_x);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("{pfx}.tabs.prev"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(tab_row_rect.x, tab_row_rect.y + 1, arrow_w, inner_h),
                label: "<",
                keyboard_path: "result.tabs.prev",
                focus_scope: FocusScope::Workspace,
                tab_order: if is_secondary { None } else { Some(509) },
                primary: false,
                selected: false,
                shortcut: None,
                enablement: if cur_x_f > 0.5 { ControlEnablement::enabled() } else { ControlEnablement::disabled("At first tab") },
                action: WidgetAction::Custom { domain: String::from("results"), payload: format!("tabs_scroll:{prev_x:.1}:{}", rs.id.as_str()) },
            },
        );
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("{pfx}.tabs.next"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(tab_row_rect.right() - i32::try_from(arrow_w).unwrap_or(20), tab_row_rect.y + 1, arrow_w, inner_h),
                label: ">",
                keyboard_path: "result.tabs.next",
                focus_scope: FocusScope::Workspace,
                tab_order: if is_secondary { None } else { Some(590) },
                primary: false,
                selected: false,
                shortcut: None,
                enablement: if cur_x_f + 0.5 < max_x { ControlEnablement::enabled() } else { ControlEnablement::disabled("At last tab") },
                action: WidgetAction::Custom { domain: String::from("results"), payload: format!("tabs_scroll:{next_x:.1}:{}", rs.id.as_str()) },
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_clipped_tab(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    tab: &ResultTab,
    idx: usize,
    tab_rect: LayoutRectPx,
    vis: LayoutRectPx,
    selected: bool,
    pfx: &str,
    is_secondary: bool,
    scroll_id: &str,
) {
    let tid = format!("{pfx}.tab.{}", tab.id);
    let st = widget_state(builder, &tid, ControlEnablement::enabled());
    let bg = if selected {
        builder.palette.surface
    } else if st.hovered {
        builder.palette.header
    } else {
        builder.palette.panel
    };
    let mut a11y = AccessibilityMeta::new(WidgetRole::Tab, tab.title.clone(), FocusScope::Workspace).with_keyboard_path(format!("result.tab.{idx}"));
    a11y.selected = Some(selected);
    a11y.tab_order = if is_secondary { None } else { Some(510 + idx as u32) };

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(tid),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: tab_rect,
        a11y,
        state: st.clone(),
        action: WidgetAction::SelectResultTab(tab.id.clone()),
    });
    builder.paint_fill(ZLayer::Workspace, Some(&wid), vis, bg);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), vis, builder.palette.border, 1);
    if selected && vis.height >= 4 {
        builder.paint_fill(ZLayer::Workspace, Some(&wid), LayoutRectPx::new(vis.x, vis.bottom() - 2, vis.width, 2), builder.palette.accent);
    }
    if st.focused && vis.width >= 4 && vis.height >= 4 {
        builder.paint_stroke(ZLayer::Workspace, Some(&wid), vis, builder.palette.accent, 2);
    }
    let size_px = (11.5 * builder.scale()).clamp(9.0, 20.0);
    let style = if selected { UiTextStyle::ui_bold(size_px) } else { UiTextStyle::ui_sans(size_px) };
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: vis,
            pad_x: 5,
            text: &tab.title,
            style: &style,
            color: builder.palette.text,
            scroll_container: Some(scroll_id),
        },
    );
}

fn build_diagnostic_banner(builder: &mut LayoutBuilder<'_>, rs: &ResultSheet, box_rect: LayoutRectPx, body_y: i32, pfx: &str, slot_tag: &str) -> i32 {
    let Some(banner) = AssumptionDiagnosticBanner::from_result(rs) else {
        return body_y;
    };
    if box_rect.width < 100 || box_rect.height < 120 {
        return body_y;
    }

    let scale = builder.scale();
    let is_secondary = slot_tag == "secondary";
    let size_px = (11.5 * scale).clamp(9.0, 20.0);
    let style = UiTextStyle::ui_sans(size_px);
    let locale = &builder.tree.locale;
    let banner_h = scaled_px(scale, 22.0);
    let link_min = scaled_px(scale, 60.0);
    let link_max = scaled_px(scale, 160.0);

    let mut link_specs: Vec<(&str, &str, u32)> = Vec::new();
    let mut links_w = 0u32;
    let max_links_budget = box_rect.width.saturating_sub(scaled_px(scale, 180.0));
    for tab_id in &banner.detail_tab_ids {
        let lbl = rs.tabs.iter().find(|t| &t.id == tab_id).map_or(tab_id.as_str(), |t| t.title.as_str());
        let w = (builder.measurer.measure(lbl, &style, locale).width_px.ceil() as u32 + 14).clamp(link_min, link_max);
        if links_w.saturating_add(w + 4) > max_links_budget {
            break;
        }
        links_w = links_w.saturating_add(w + 4);
        link_specs.push((tab_id.as_str(), lbl, w));
    }

    let banner_w = box_rect.width.saturating_sub(16 + links_w).max(scaled_px(scale, 60.0));
    let banner_rect = LayoutRectPx::new(box_rect.x + 8, body_y, banner_w, banner_h);
    let banner_lbl = banner.format_banner_line();

    let banner_wid = emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{pfx}.diagnostic_banner"),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: banner_rect,
            label: &banner_lbl,
            keyboard_path: "ui.result.inspectDiagnostics",
            focus_scope: FocusScope::Workspace,
            tab_order: if is_secondary { None } else { Some(600) },
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::InvokeCommand {
                command_id: String::from("ui.result.inspectDiagnostics"),
                params: json!({ "result_sheet_id": rs.id.as_str() }),
            },
        },
    );
    let sev_color = match banner.severity {
        ChecklistStatus::Violated => builder.palette.link_red,
        ChecklistStatus::Warn | ChecklistStatus::Info => builder.palette.accent,
        ChecklistStatus::Pass => builder.palette.link_green,
    };
    builder.paint_stroke(ZLayer::Workspace, Some(&banner_wid), banner_rect, sev_color, 1);

    let mut lx = banner_rect.right() + 4;
    for (idx, (tab_id, lbl, w)) in link_specs.into_iter().enumerate() {
        let r = LayoutRectPx::new(lx, body_y, w, banner_h);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("{pfx}.diagnostic_banner.tab.{tab_id}"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: r,
                label: lbl,
                keyboard_path: "ui.result.inspectDiagnostics",
                focus_scope: FocusScope::Workspace,
                tab_order: if is_secondary { None } else { Some(601 + idx as u32) },
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::InvokeCommand {
                    command_id: String::from("ui.result.inspectDiagnostics"),
                    params: json!({ "result_sheet_id": rs.id.as_str(), "tab_id": tab_id }),
                },
            },
        );
        lx += i32::try_from(w + 4).unwrap_or(64);
    }

    if slot_tag != "secondary" || builder.tree.result_banner_rect.is_none() {
        builder.tree.result_banner_rect = Some(banner_rect.to_workspace_rect());
        builder.tree.result_banner_sheet_id = Some(rs.id.clone());
    }
    body_y + i32::try_from(banner_h + 4).unwrap_or(26)
}

fn compute_body_split(body_rect: LayoutRectPx, drawer_open: bool, is_rtl: bool, scale: f64) -> (LayoutRectPx, Option<LayoutRectPx>) {
    if !drawer_open || body_rect.width < scaled_px(scale, 420.0) || body_rect.height < scaled_px(scale, 80.0) {
        return (body_rect, None);
    }
    let drawer_w = (body_rect.width / 3).clamp(scaled_px(scale, 210.0), scaled_px(scale, 340.0));
    let table_w = body_rect.width.saturating_sub(drawer_w + 6);
    if is_rtl {
        let drawer = LayoutRectPx::new(body_rect.x, body_rect.y, drawer_w, body_rect.height);
        let table = LayoutRectPx::new(drawer.right() + 6, body_rect.y, table_w, body_rect.height);
        (table, Some(drawer))
    } else {
        let table = LayoutRectPx::new(body_rect.x, body_rect.y, table_w, body_rect.height);
        let drawer = LayoutRectPx::new(table.right() + 6, body_rect.y, drawer_w, body_rect.height);
        (table, Some(drawer))
    }
}

fn tab_kind_badge(kind: ResultTabKind) -> &'static str {
    match kind {
        ResultTabKind::MainSummary => "Summary",
        ResultTabKind::AnovaTable => "ANOVA",
        ResultTabKind::MultipleComparisons => "Multiple Comparisons",
        ResultTabKind::ParametersAndConfidenceIntervals => "Parameters & CI",
        ResultTabKind::ModelComparison => "Model Comparison",
        ResultTabKind::Residuals => "Residuals",
        ResultTabKind::DescriptiveSummary => "Descriptive",
        ResultTabKind::InterpolatedUnknowns => "Interpolated",
        ResultTabKind::CurveCoordinates => "Curve Points",
        ResultTabKind::NumberAtRisk => "At Risk",
        ResultTabKind::Diagnostics => "Diagnostics",
        ResultTabKind::CovarianceMatrix => "Covariance",
        ResultTabKind::NormalQq => "Normal Q-Q",
        ResultTabKind::BaselineSurvival => "Baseline Survival",
        ResultTabKind::LogMinusLog => "Log(-Log)",
    }
}

fn build_table_section_bar(
    builder: &mut LayoutBuilder<'_>,
    rs: &ResultSheet,
    tab: &ResultTab,
    sec_rect: LayoutRectPx,
    active_tab_idx: usize,
    pfx: &str,
) {
    let num_cols = tab_column_count(tab);
    let badge = tab_kind_badge(tab.kind);
    let summary = format!("[{badge}] {} ({} x {})", tab.title, tab.rows.len(), num_cols);
    let sel_info = builder.app.view_shards.results.selection_bounds(rs.id.as_str(), active_tab_idx).map(|(r0, r1, c0, c1)| {
        if r0 == r1 && c0 == c1 { format!("Sel R{}C{}", r0 + 1, c0 + 1) } else { format!("Sel R{}-{} C{}-{}", r0 + 1, r1 + 1, c0 + 1, c1 + 1) }
    });

    let size_px = (11.0 * builder.scale()).clamp(8.5, 19.0);
    let sel_w = if sel_info.is_some() && sec_rect.width >= 260 { 112u32 } else { 0 };
    let left_rect = LayoutRectPx::new(sec_rect.x, sec_rect.y, sec_rect.width.saturating_sub(sel_w + 4), sec_rect.height);

    let sec_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{pfx}.section_header")),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: sec_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, summary.clone(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&sec_wid), sec_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, Some(&sec_wid), sec_rect, builder.palette.border, 1);
    paint_text_fitted(
        builder,
        &sec_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: left_rect,
            pad_x: 6,
            text: &summary,
            style: &UiTextStyle::ui_bold(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    if let Some(sel_txt) = sel_info
        && sel_w > 0
    {
        let right_rect = LayoutRectPx::new(sec_rect.right() - i32::try_from(sel_w).unwrap_or(112), sec_rect.y, sel_w, sec_rect.height);
        paint_text_fitted(
            builder,
            &sec_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: right_rect,
                pad_x: 4,
                text: &sel_txt,
                style: &UiTextStyle::ui_sans(size_px),
                color: builder.palette.accent,
                scroll_container: None,
            },
        );
    }
}

struct TableGeometry {
    col_widths: Vec<u32>,
    row_label_w: u32,
    hdr_h: u32,
    row_h: u32,
    v_thick: u32,
    data_vp_w: u32,
    data_vp_h: u32,
    scroll_meta: ScrollMeta,
}

fn measure_table_geometry(
    builder: &LayoutBuilder<'_>,
    rs: &ResultSheet,
    tab: &ResultTab,
    table_rect: LayoutRectPx,
    scroll_id: &str,
) -> TableGeometry {
    let t_scale = builder.scale();
    let size_px = (11.5 * t_scale).clamp(9.0, 20.0);
    let hdr_style = UiTextStyle::ui_bold(size_px);
    let cell_style = UiTextStyle::grid_tabular(size_px);
    let locale = &builder.tree.locale;

    let min_col_w = (82.0 * t_scale).round().clamp(72.0, 140.0) as u32;
    let max_col_w = (230.0 * t_scale).round().clamp(160.0, 360.0) as u32;
    let num_cols = tab_column_count(tab).max(1);
    let mut col_widths = Vec::with_capacity(num_cols);

    for c_idx in 0..num_cols {
        let h_lbl = tab_column_header(tab, c_idx);
        let mut w = builder.measurer.measure(&h_lbl, &hdr_style, locale).width_px.ceil() as u32 + 16;
        for row in tab.rows.iter().take(64) {
            if let Some(cell) = row.get(c_idx) {
                let txt = ResultSheetViewState::format_result_cell(cell, rs.analysis.p_value_format);
                if !txt.is_empty() {
                    let cw = builder.measurer.measure(&txt, &cell_style, locale).width_px.ceil() as u32 + 16;
                    w = w.max(cw);
                }
            }
        }
        col_widths.push(w.clamp(min_col_w, max_col_w));
    }

    let row_hdr_lbl = tr_res(locale, "results.row_metric", "Row / Metric");
    let mut rl_w = builder.measurer.measure(row_hdr_lbl, &hdr_style, locale).width_px.ceil() as u32 + 16;
    for r_lbl in tab.row_labels.iter().take(64) {
        if !r_lbl.is_empty() {
            let lw = builder.measurer.measure(r_lbl, &cell_style, locale).width_px.ceil() as u32 + 16;
            rl_w = rl_w.max(lw);
        }
    }
    let rl_min = scaled_px(t_scale, 108.0);
    let rl_max = scaled_px(t_scale, 240.0);
    let rl_cap = ((table_rect.width * 2) / 5).max(86);
    let row_label_w = rl_w.clamp(rl_min.min(rl_cap.max(86)), rl_cap.min(rl_max).max(86)).min(table_rect.width.saturating_sub(60).max(48));

    let hdr_h = (24.0 * t_scale).round().clamp(20.0, 34.0) as u32;
    let row_h = (21.0 * t_scale).round().clamp(18.0, 32.0) as u32;
    let footnotes_h = if tab.footnotes.is_empty() { 0 } else { 8 + (tab.footnotes.len() as u32).saturating_mul(row_h) };
    let total_body_h = (tab.rows.len().max(1) as u32).saturating_mul(row_h).saturating_add(footnotes_h).saturating_add(6);
    let total_data_w: u32 = col_widths.iter().copied().sum();

    let thick = scrollbar_thickness_px(t_scale);
    let need_v0 = total_body_h > table_rect.height.saturating_sub(hdr_h);
    let need_h0 = total_data_w > table_rect.width.saturating_sub(row_label_w + if need_v0 { thick } else { 0 });
    let h_thick = if need_h0 && table_rect.height > hdr_h + thick + 16 { thick } else { 0 };
    let need_v = total_body_h > table_rect.height.saturating_sub(hdr_h + h_thick);
    let v_thick = if need_v && table_rect.width > row_label_w + thick + 24 { thick } else { 0 };
    let need_h = total_data_w > table_rect.width.saturating_sub(row_label_w + v_thick);
    let h_thick = if need_h && table_rect.height > hdr_h + thick + 16 { thick } else { 0 };

    let data_vp_w = table_rect.width.saturating_sub(row_label_w + v_thick);
    let data_vp_h = table_rect.height.saturating_sub(hdr_h + h_thick);
    let content_w = if need_h { table_rect.width.saturating_sub(data_vp_w).saturating_add(total_data_w) } else { table_rect.width };
    let content_h = if need_v { table_rect.height.saturating_sub(data_vp_h).saturating_add(total_body_h) } else { table_rect.height };

    let scroll_off = builder.scroll_offset(scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id, table_rect, [content_w, content_h], scroll_off, ScrollAxes::Both);

    TableGeometry { col_widths, row_label_w, hdr_h, row_h, v_thick, data_vp_w, data_vp_h, scroll_meta }
}

fn build_result_table(
    builder: &mut LayoutBuilder<'_>,
    rs: &ResultSheet,
    tab: &ResultTab,
    table_rect: LayoutRectPx,
    pfx: &str,
    active_tab_idx: usize,
) {
    if table_rect.width < 60 || table_rect.height < 42 {
        return;
    }
    let scroll_id = format!("scroll.{pfx}.{}", rs.id.as_str());
    let geom = measure_table_geometry(builder, rs, tab, table_rect, &scroll_id);

    let tbl_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{pfx}.table")),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: table_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, tab.title.clone(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&tbl_wid, geom.scroll_meta.clone());
    builder.paint_fill(ZLayer::Workspace, Some(&tbl_wid), table_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, Some(&tbl_wid), table_rect, builder.palette.border, 1);

    let sel = builder.app.view_shards.results.selection_bounds(rs.id.as_str(), active_tab_idx);
    emit_table_corner_and_col_headers(builder, rs, tab, table_rect, &tbl_wid, &geom, &scroll_id, pfx, active_tab_idx, sel);
    emit_table_row_labels(builder, rs, tab, table_rect, &tbl_wid, &geom, &scroll_id, pfx, active_tab_idx, sel);
    emit_table_cells_and_footnotes(builder, rs, tab, table_rect, &tbl_wid, &geom, &scroll_id, pfx, active_tab_idx, sel);
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &geom.scroll_meta, Some(tbl_wid));
}

#[allow(clippy::too_many_arguments)]
fn emit_table_corner_and_col_headers(
    builder: &mut LayoutBuilder<'_>,
    rs: &ResultSheet,
    tab: &ResultTab,
    table_rect: LayoutRectPx,
    tbl_wid: &WidgetId,
    geom: &TableGeometry,
    scroll_id: &str,
    pfx: &str,
    active_tab_idx: usize,
    sel: Option<(usize, usize, usize, usize)>,
) {
    let size_px = (11.5 * builder.scale()).clamp(9.0, 20.0);
    let hdr_style = UiTextStyle::ui_bold(size_px);
    let corner_rect = LayoutRectPx::new(table_rect.x, table_rect.y, geom.row_label_w, geom.hdr_h);
    let row_hdr_lbl = tr_res(&builder.tree.locale, "results.row_metric", "Row / Metric");

    let corner_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{pfx}.corner")),
        parent: Some(tbl_wid.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: corner_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, row_hdr_lbl, FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&corner_wid), corner_rect, builder.palette.header);
    builder.paint_stroke(ZLayer::Workspace, Some(&corner_wid), corner_rect, builder.palette.border, 1);
    paint_text_fitted(
        builder,
        &corner_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: corner_rect,
            pad_x: 4,
            text: row_hdr_lbl,
            style: &hdr_style,
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let col_hdr_clip = LayoutRectPx::new(table_rect.x + i32::try_from(geom.row_label_w).unwrap_or(0), table_rect.y, geom.data_vp_w, geom.hdr_h);
    if col_hdr_clip.width < 8 {
        return;
    }
    builder.push_clip(col_hdr_clip);
    let x_scroll = geom.scroll_meta.scroll_offset_px[0].round() as i32;
    let max_row = tab.rows.len().saturating_sub(1);
    let mut cum_x = 0i32;

    for (c_idx, &cw) in geom.col_widths.iter().enumerate() {
        let cx = col_hdr_clip.x + cum_x - x_scroll;
        cum_x += i32::try_from(cw).unwrap_or(82);
        let h_rect = LayoutRectPx::new(cx, col_hdr_clip.y, cw, geom.hdr_h);
        let vis = h_rect.intersect(col_hdr_clip);
        if vis.width < 8 || vis.height < 8 {
            continue;
        }
        let lbl = tab_column_header(tab, c_idx);
        let col_selected = sel.is_some_and(|(r0, r1, c0, c1)| r0 == 0 && r1 == max_row && c_idx >= c0 && c_idx <= c1);
        let hid = format!("{pfx}.col_hdr.{c_idx}");
        let st = widget_state(builder, &hid, ControlEnablement::enabled());
        let mut a11y = labeled_meta(WidgetRole::Button, lbl.clone(), FocusScope::Workspace, format!("result.col.{c_idx}"));
        a11y.selected = Some(col_selected);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(hid),
            parent: Some(tbl_wid.clone()),
            z_layer: ZLayer::Workspace,
            rect_px: h_rect,
            a11y,
            state: st.clone(),
            action: WidgetAction::Custom {
                domain: String::from("results"),
                payload: format!("select_col:{active_tab_idx}:{c_idx}:{max_row}:{}", rs.id.as_str()),
            },
        });
        let bg = if col_selected {
            Rgba8::rgba(59, 130, 246, 56)
        } else if st.hovered {
            builder.palette.surface
        } else {
            builder.palette.header
        };
        builder.paint_fill(ZLayer::Workspace, Some(&wid), vis, bg);
        builder.paint_stroke(ZLayer::Workspace, Some(&wid), vis, if col_selected { builder.palette.accent } else { builder.palette.border }, 1);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: vis,
                pad_x: 4,
                text: &lbl,
                style: &hdr_style,
                color: builder.palette.text,
                scroll_container: Some(scroll_id),
            },
        );
    }
    builder.pop_clip();
}

#[allow(clippy::too_many_arguments)]
fn emit_table_row_labels(
    builder: &mut LayoutBuilder<'_>,
    rs: &ResultSheet,
    tab: &ResultTab,
    table_rect: LayoutRectPx,
    tbl_wid: &WidgetId,
    geom: &TableGeometry,
    scroll_id: &str,
    pfx: &str,
    active_tab_idx: usize,
    sel: Option<(usize, usize, usize, usize)>,
) {
    let row_hdr_clip = LayoutRectPx::new(table_rect.x, table_rect.y + i32::try_from(geom.hdr_h).unwrap_or(24), geom.row_label_w, geom.data_vp_h);
    if row_hdr_clip.width < 8 || row_hdr_clip.height < 8 {
        return;
    }
    builder.push_clip(row_hdr_clip);
    let y_scroll = geom.scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (11.5 * builder.scale()).clamp(9.0, 20.0);
    let reg_style = UiTextStyle::ui_sans(size_px);
    let bold_style = UiTextStyle::ui_bold(size_px);
    let max_col = geom.col_widths.len().saturating_sub(1);

    for (r_idx, row) in tab.rows.iter().enumerate() {
        let ry = row_hdr_clip.y + i32::try_from((r_idx as u32).saturating_mul(geom.row_h)).unwrap_or(0) - y_scroll;
        let r_rect = LayoutRectPx::new(row_hdr_clip.x, ry, geom.row_label_w, geom.row_h);
        let vis = r_rect.intersect(row_hdr_clip);
        if vis.width < 8 || vis.height < 8 {
            continue;
        }
        let r_label = tab.row_labels.get(r_idx).map(String::as_str).filter(|s| !s.is_empty()).unwrap_or("Row");
        let is_section_row = !r_label.is_empty() && row.iter().all(|c| matches!(c, ResultCell::Empty));
        let row_selected = sel.is_some_and(|(r0, r1, c0, c1)| r_idx >= r0 && r_idx <= r1 && c0 == 0 && c1 == max_col);
        let rid = format!("{pfx}.row_hdr.{r_idx}");
        let st = widget_state(builder, &rid, ControlEnablement::enabled());
        let mut a11y = labeled_meta(WidgetRole::Button, r_label.to_owned(), FocusScope::Workspace, format!("result.row.{r_idx}"));
        a11y.selected = Some(row_selected);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(rid),
            parent: Some(tbl_wid.clone()),
            z_layer: ZLayer::Workspace,
            rect_px: r_rect,
            a11y,
            state: st.clone(),
            action: WidgetAction::Custom {
                domain: String::from("results"),
                payload: format!("select_row:{active_tab_idx}:{r_idx}:{max_col}:{}", rs.id.as_str()),
            },
        });
        let bg = if row_selected {
            Rgba8::rgba(59, 130, 246, 56)
        } else if is_section_row || st.hovered {
            builder.palette.header
        } else {
            builder.palette.panel
        };
        builder.paint_fill(ZLayer::Workspace, Some(&wid), vis, bg);
        builder.paint_stroke(ZLayer::Workspace, Some(&wid), vis, builder.palette.border, 1);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: vis,
                pad_x: 4,
                text: r_label,
                style: if is_section_row { &bold_style } else { &reg_style },
                color: if is_section_row { builder.palette.text } else { builder.palette.muted },
                scroll_container: Some(scroll_id),
            },
        );
    }
    builder.pop_clip();
}

#[allow(clippy::too_many_arguments)]
fn emit_table_cells_and_footnotes(
    builder: &mut LayoutBuilder<'_>,
    rs: &ResultSheet,
    tab: &ResultTab,
    table_rect: LayoutRectPx,
    tbl_wid: &WidgetId,
    geom: &TableGeometry,
    scroll_id: &str,
    pfx: &str,
    active_tab_idx: usize,
    sel: Option<(usize, usize, usize, usize)>,
) {
    let data_clip = LayoutRectPx::new(
        table_rect.x + i32::try_from(geom.row_label_w).unwrap_or(0),
        table_rect.y + i32::try_from(geom.hdr_h).unwrap_or(24),
        geom.data_vp_w,
        geom.data_vp_h,
    );
    if data_clip.width < 8 || data_clip.height < 8 {
        return;
    }

    let x_scroll = geom.scroll_meta.scroll_offset_px[0].round() as i32;
    let y_scroll = geom.scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (11.5 * builder.scale()).clamp(9.0, 20.0);
    let tab_style = UiTextStyle::grid_tabular(size_px);
    let sig_style = tab_style.clone().bold();

    builder.push_clip(data_clip);
    for (r_idx, row) in tab.rows.iter().enumerate() {
        let ry = data_clip.y + i32::try_from((r_idx as u32).saturating_mul(geom.row_h)).unwrap_or(0) - y_scroll;
        if ry + i32::try_from(geom.row_h).unwrap_or(21) <= data_clip.y || ry >= data_clip.bottom() {
            continue;
        }
        let is_section_row = tab.row_labels.get(r_idx).is_some_and(|s| !s.is_empty()) && row.iter().all(|c| matches!(c, ResultCell::Empty));
        let mut cum_x = 0i32;

        for (c_idx, &cw) in geom.col_widths.iter().enumerate() {
            let cx = data_clip.x + cum_x - x_scroll;
            cum_x += i32::try_from(cw).unwrap_or(82);
            let cell_rect = LayoutRectPx::new(cx, ry, cw, geom.row_h);
            let vis = cell_rect.intersect(data_clip);
            if vis.width < 8 || vis.height < 8 {
                continue;
            }
            let cell = row.get(c_idx).unwrap_or(&ResultCell::Empty);
            let txt = ResultSheetViewState::format_result_cell(cell, rs.analysis.p_value_format);
            let is_selected = sel.is_some_and(|(r0, r1, c0, c1)| r_idx >= r0 && r_idx <= r1 && c_idx >= c0 && c_idx <= c1);
            let cid = format!("{pfx}.cell.{r_idx}.{c_idx}");
            let st = widget_state(builder, &cid, ControlEnablement::enabled());
            let mut a11y = AccessibilityMeta::new(
                WidgetRole::GridCell,
                format!("R{}C{} {}", r_idx + 1, c_idx + 1, tab_column_header(tab, c_idx)),
                FocusScope::Workspace,
            )
            .with_keyboard_path("ArrowKeys");
            a11y.selected = Some(is_selected);
            a11y.value_text = (!txt.is_empty()).then(|| txt.clone());

            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(cid),
                parent: Some(tbl_wid.clone()),
                z_layer: ZLayer::Workspace,
                rect_px: cell_rect,
                a11y,
                state: st.clone(),
                action: WidgetAction::Custom {
                    domain: String::from("results"),
                    payload: format!("select_cell:{active_tab_idx}:{r_idx}:{c_idx}:{}", rs.id.as_str()),
                },
            });
            let bg = if is_selected {
                Rgba8::rgba(59, 130, 246, 52)
            } else if is_section_row {
                builder.palette.header
            } else if st.hovered {
                builder.palette.panel
            } else {
                builder.palette.surface
            };
            let border = if is_selected { builder.palette.accent } else { builder.palette.border };
            builder.paint_fill(ZLayer::Workspace, Some(&wid), vis, bg);
            builder.paint_stroke(ZLayer::Workspace, Some(&wid), vis, border, 1);

            if !txt.is_empty() {
                let is_sig = matches!(cell, ResultCell::PValue { stars, .. } if !stars.is_empty());
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: vis,
                        pad_x: 4,
                        text: &txt,
                        style: if is_sig { &sig_style } else { &tab_style },
                        color: if is_sig { builder.palette.accent } else { builder.palette.text },
                        scroll_container: Some(scroll_id),
                    },
                );
            }
        }
    }
    builder.pop_clip();

    if !tab.footnotes.is_empty() {
        let fn_clip = LayoutRectPx::new(
            table_rect.x,
            table_rect.y + i32::try_from(geom.hdr_h).unwrap_or(24),
            table_rect.width.saturating_sub(geom.v_thick),
            geom.data_vp_h,
        );
        let rows_end_y = fn_clip.y + i32::try_from((tab.rows.len().max(1) as u32).saturating_mul(geom.row_h)).unwrap_or(0) - y_scroll + 4;
        if fn_clip.width >= 24 && fn_clip.height >= geom.row_h {
            builder.push_clip(fn_clip);
            let fn_style = UiTextStyle::ui_sans((10.5 * builder.scale()).clamp(8.5, 18.0));
            for (idx, note) in tab.footnotes.iter().enumerate() {
                let fy = rows_end_y + i32::try_from((idx as u32).saturating_mul(geom.row_h)).unwrap_or(0);
                if fy + i32::try_from(geom.row_h).unwrap_or(21) <= fn_clip.y || fy >= fn_clip.bottom() {
                    continue;
                }
                let r = LayoutRectPx::new(fn_clip.x + 6, fy, fn_clip.width.saturating_sub(12), geom.row_h);
                let line =
                    if idx == 0 { format!("{}: {note}", tr_res(&builder.tree.locale, "results.footnotes", "Footnotes")) } else { note.clone() };
                paint_text_fitted(
                    builder,
                    tbl_wid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: r,
                        pad_x: 2,
                        text: &line,
                        style: &fn_style,
                        color: builder.palette.muted,
                        scroll_container: Some(scroll_id),
                    },
                );
            }
            builder.pop_clip();
        }
    }
}

fn build_empty_or_failed_body(builder: &mut LayoutBuilder<'_>, rs: &ResultSheet, area: LayoutRectPx, pfx: &str) {
    let msg = match &rs.status {
        ResultStatus::Failed { code, message } => format!("Analysis failed ({}): {message}", code.as_str()),
        ResultStatus::Dirty => String::from("Analysis pending recalculation."),
        ResultStatus::UpToDate => String::from("No tabular output generated for this result sheet."),
    };
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{pfx}.table")),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: area,
        a11y: AccessibilityMeta::new(WidgetRole::Label, msg.clone(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&wid), area, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), area, builder.palette.border, 1);
    let size_px = (12.0 * builder.scale()).clamp(9.0, 20.0);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: LayoutRectPx::new(area.x + 8, area.y + 8, area.width.saturating_sub(16), 24),
            pad_x: 4,
            text: &msg,
            style: &UiTextStyle::ui_sans(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
}

fn build_interpret_drawer(builder: &mut LayoutBuilder<'_>, rs: &ResultSheet, drawer_rect: LayoutRectPx, pfx: &str, is_secondary: bool) {
    if drawer_rect.width < 120 || drawer_rect.height < 80 {
        return;
    }
    let drawer_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{pfx}.interpret.drawer")),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: drawer_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, String::from("Interpret and Assumption Checklist"), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&drawer_wid), drawer_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, Some(&drawer_wid), drawer_rect, builder.palette.border, 1);
    builder.push_clip(drawer_rect);

    let scale = builder.scale();
    let locale = &builder.app.locale;
    let size_px = (11.5 * scale).clamp(9.0, 20.0);
    let row_h = scaled_px(scale, 22.0);
    let close_sz = scaled_px(scale, 20.0);
    let hdr_rect = LayoutRectPx::new(drawer_rect.x + 6, drawer_rect.y + 4, drawer_rect.width.saturating_sub(close_sz + 16), row_h);
    paint_text_fitted(
        builder,
        &drawer_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: hdr_rect,
            pad_x: 2,
            text: tr_res(locale, "results.interpret", "Interpret"),
            style: &UiTextStyle::ui_bold(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{pfx}.interpret.close_btn"),
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::Workspace,
            rect: LayoutRectPx::new(drawer_rect.right() - i32::try_from(close_sz + 6).unwrap_or(26), drawer_rect.y + 4, close_sz, close_sz),
            label: "x",
            keyboard_path: "result.interpret.close",
            focus_scope: FocusScope::Workspace,
            tab_order: if is_secondary { None } else { Some(650) },
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("results"), payload: format!("toggle_interpret:{}", rs.id.as_str()) },
        },
    );

    let active_dtab = builder.app.view_shards.results.interpret_tab;
    let d_tabs = [
        (InterpretDrawerTab::Assumptions, "assumptions", tr_res(locale, "results.assumptions", "Assumptions")),
        (InterpretDrawerTab::Narrative, "narrative", tr_res(locale, "results.narrative", "Narrative")),
        (InterpretDrawerTab::Methods, "methods", tr_res(locale, "results.methods", "Methods")),
    ];
    let dtab_y = drawer_rect.y + 4 + i32::try_from(row_h).unwrap_or(22) + 2;
    let dtab_w = (drawer_rect.width.saturating_sub(20) / 3).max(scaled_px(scale, 48.0));
    for (idx, (variant, slug, lbl)) in d_tabs.into_iter().enumerate() {
        let tx = drawer_rect.x + 6 + i32::try_from(idx as u32 * (dtab_w + 4)).unwrap_or(0);
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &format!("{pfx}.interpret.tab.{slug}"),
                parent: Some(drawer_wid.clone()),
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(tx, dtab_y, dtab_w, row_h),
                label: lbl,
                keyboard_path: &format!("result.interpret.tab.{slug}"),
                focus_scope: FocusScope::Workspace,
                tab_order: if is_secondary { None } else { Some(651 + idx as u32) },
                primary: false,
                selected: active_dtab == variant,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("results"), payload: format!("interpret_tab:{slug}") },
            },
        );
    }

    let ctrl_y = dtab_y + i32::try_from(row_h).unwrap_or(22) + 4;
    emit_drawer_controls_row(builder, &drawer_wid, rs, drawer_rect, ctrl_y, row_h, active_dtab, pfx, is_secondary);

    let scroll_y = ctrl_y + i32::try_from(row_h).unwrap_or(22) + 4;
    let scroll_h = u32::try_from((drawer_rect.bottom() - scroll_y - 4).max(0)).unwrap_or(0);
    if scroll_h >= 24 {
        let body_rect = LayoutRectPx::new(drawer_rect.x + 6, scroll_y, drawer_rect.width.saturating_sub(12), scroll_h);
        build_drawer_scroll_body(builder, rs, body_rect, active_dtab, pfx);
    }

    builder.pop_clip();
}

#[allow(clippy::too_many_arguments)]
fn emit_drawer_controls_row(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rs: &ResultSheet,
    drawer_rect: LayoutRectPx,
    ctrl_y: i32,
    ctrl_h: u32,
    active_dtab: InterpretDrawerTab,
    pfx: &str,
    is_secondary: bool,
) {
    let scale = builder.scale();
    let locale = &builder.app.locale;
    let avail_w = drawer_rect.width.saturating_sub(12);
    match active_dtab {
        InterpretDrawerTab::Assumptions => {
            let bw = (avail_w.saturating_sub(6) / 2).max(scaled_px(scale, 48.0));
            let filter = builder.app.canvas_dialogs.result_view.checklist_filter;
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("{pfx}.interpret.filter_all"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: LayoutRectPx::new(drawer_rect.x + 6, ctrl_y, bw, ctrl_h),
                    label: tr_res(locale, "results.filter_all", "All"),
                    keyboard_path: "result.interpret.filter_all",
                    focus_scope: FocusScope::Workspace,
                    tab_order: if is_secondary { None } else { Some(660) },
                    primary: false,
                    selected: filter == ChecklistSeverityFilter::All,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("results"), payload: String::from("checklist_filter:all") },
                },
            );
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("{pfx}.interpret.filter_flagged"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: LayoutRectPx::new(drawer_rect.x + 10 + i32::try_from(bw).unwrap_or(48), ctrl_y, bw, ctrl_h),
                    label: tr_res(locale, "results.filter_flagged", "Flagged"),
                    keyboard_path: "result.interpret.filter_flagged",
                    focus_scope: FocusScope::Workspace,
                    tab_order: if is_secondary { None } else { Some(661) },
                    primary: false,
                    selected: filter == ChecklistSeverityFilter::WarningsAndViolations,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("results"), payload: String::from("checklist_filter:flagged") },
                },
            );
        }
        InterpretDrawerTab::Narrative => {
            let note = tr_res(locale, "results.diagnostics_only", "Diagnostics only (no automatic remediation)");
            paint_text_fitted(
                builder,
                parent,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: LayoutRectPx::new(drawer_rect.x + 6, ctrl_y, avail_w, ctrl_h),
                    pad_x: 2,
                    text: note,
                    style: &UiTextStyle::ui_sans((10.5 * scale).clamp(8.5, 18.0)),
                    color: builder.palette.muted,
                    scroll_container: None,
                },
            );
        }
        InterpretDrawerTab::Methods => {
            let raw_fmt = builder.app.canvas_dialogs.result_view.report_format.trim();
            let cur_fmt = if raw_fmt.is_empty() { "markdown" } else { raw_fmt };
            let fmt_w = scaled_px(scale, 38.0);
            let copy_reserve = i32::try_from(scaled_px(scale, 72.0)).unwrap_or(72);
            let mut fx = drawer_rect.x + 6;
            for (idx, (slug, lbl)) in [("plain", "Txt"), ("markdown", "MD"), ("latex", "TeX")].into_iter().enumerate() {
                if fx + i32::try_from(fmt_w).unwrap_or(38) > drawer_rect.right() - copy_reserve {
                    break;
                }
                emit_button(
                    builder,
                    ButtonWidgetSpec {
                        id: &format!("{pfx}.interpret.fmt.{slug}"),
                        parent: Some(parent.clone()),
                        z_layer: ZLayer::Workspace,
                        rect: LayoutRectPx::new(fx, ctrl_y, fmt_w, ctrl_h),
                        label: lbl,
                        keyboard_path: &format!("result.interpret.fmt.{slug}"),
                        focus_scope: FocusScope::Workspace,
                        tab_order: if is_secondary { None } else { Some(670 + idx as u32) },
                        primary: false,
                        selected: cur_fmt == slug,
                        shortcut: None,
                        enablement: ControlEnablement::enabled(),
                        action: WidgetAction::Custom { domain: String::from("results"), payload: format!("report_format:{slug}") },
                    },
                );
                fx += i32::try_from(fmt_w + 3).unwrap_or(41);
            }
            let copy_w = u32::try_from((drawer_rect.right() - 6 - fx).max(48)).unwrap_or(48);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("{pfx}.interpret.copy_methods_btn"),
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: LayoutRectPx::new(fx, ctrl_y, copy_w, ctrl_h),
                    label: tr_res(locale, "results.copy_methods", "Copy Methods Report"),
                    keyboard_path: "ui.result.copyMethodsReport",
                    focus_scope: FocusScope::Workspace,
                    tab_order: if is_secondary { None } else { Some(675) },
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::InvokeCommand {
                        command_id: String::from("ui.result.copyMethodsReport"),
                        params: json!({ "result_sheet_id": rs.id.as_str(), "format": cur_fmt }),
                    },
                },
            );
        }
    }
}

struct DrawerLine {
    text: String,
    bold: bool,
    color: Rgba8,
}

fn build_drawer_scroll_body(builder: &mut LayoutBuilder<'_>, rs: &ResultSheet, body_rect: LayoutRectPx, active_dtab: InterpretDrawerTab, pfx: &str) {
    let t_scale = builder.scale();
    let line_h = (17.0 * t_scale).round().clamp(14.0, 26.0) as u32;
    let size_px = (11.0 * t_scale).clamp(8.5, 19.0);
    let reg_style = UiTextStyle::ui_sans(size_px);
    let bold_style = UiTextStyle::ui_bold(size_px);
    let thick = scrollbar_thickness_px(t_scale);
    let text_w = body_rect.width.saturating_sub(thick + 8).max(40);
    let lines = collect_drawer_lines(builder, rs, active_dtab, &reg_style, &bold_style, f64::from(text_w));

    let content_h = (lines.len().max(1) as u32).saturating_mul(line_h) + 8;
    let scroll_id = format!("scroll.{pfx}.interpret.{}", rs.id.as_str());
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), body_rect, [body_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let body_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{pfx}.interpret.body")),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: body_rect,
        a11y: labeled_meta(WidgetRole::Canvas, "Interpret Content", FocusScope::Workspace, "Scroll"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&body_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::Workspace, Some(&body_wid), body_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, Some(&body_wid), body_rect, builder.palette.border, 1);

    let clip = LayoutRectPx::new(body_rect.x + 2, body_rect.y + 2, body_rect.width.saturating_sub(thick + 4), body_rect.height.saturating_sub(4));
    builder.push_clip(clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    for (idx, line) in lines.iter().enumerate() {
        let ly = clip.y + 2 + i32::try_from((idx as u32).saturating_mul(line_h)).unwrap_or(0) - y_scroll;
        if ly + i32::try_from(line_h).unwrap_or(17) <= clip.y || ly >= clip.bottom() {
            continue;
        }
        let r = LayoutRectPx::new(clip.x + 2, ly, clip.width.saturating_sub(4), line_h);
        paint_text_fitted(
            builder,
            &body_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: r,
                pad_x: 2,
                text: &line.text,
                style: if line.bold { &bold_style } else { &reg_style },
                color: line.color,
                scroll_container: Some(&scroll_id),
            },
        );
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(body_wid));
}

fn collect_drawer_lines(
    builder: &LayoutBuilder<'_>,
    rs: &ResultSheet,
    active_dtab: InterpretDrawerTab,
    reg_style: &UiTextStyle,
    bold_style: &UiTextStyle,
    max_w_px: f64,
) -> Vec<DrawerLine> {
    let mut out = Vec::new();
    let locale = &builder.tree.locale;
    match active_dtab {
        InterpretDrawerTab::Assumptions => {
            let (p, w, v, i) = ResultSheetViewState::checklist_summary_counts(rs);
            out.push(DrawerLine { text: format!("Pass: {p} | Warn: {w} | Violated: {v} | Info: {i}"), bold: true, color: builder.palette.text });
            out.push(DrawerLine {
                text: tr_res(locale, "results.diagnostics_only", "Diagnostics only (no automatic remediation)").to_owned(),
                bold: false,
                color: builder.palette.muted,
            });
            let items = builder.app.canvas_dialogs.result_view.visible_checklist(rs);
            if items.is_empty() {
                for wrapped in wrap_text_to_width(
                    builder.measurer,
                    tr_res(locale, "results.no_checklist", "No assumption diagnostics recorded for this analysis."),
                    reg_style,
                    locale,
                    max_w_px,
                ) {
                    out.push(DrawerLine { text: wrapped, bold: false, color: builder.palette.muted });
                }
            } else {
                for item in items {
                    append_checklist_item_lines(builder, item, reg_style, bold_style, max_w_px, &mut out);
                }
            }
        }
        InterpretDrawerTab::Narrative => {
            append_narrative_lines(builder, rs, reg_style, bold_style, max_w_px, &mut out);
        }
        InterpretDrawerTab::Methods => {
            append_methods_lines(builder, rs, reg_style, bold_style, max_w_px, &mut out);
        }
    }
    out
}

fn append_checklist_item_lines(
    builder: &LayoutBuilder<'_>,
    item: &ChecklistItem,
    reg_style: &UiTextStyle,
    bold_style: &UiTextStyle,
    max_w_px: f64,
    out: &mut Vec<DrawerLine>,
) {
    let locale = &builder.tree.locale;
    let (tag, color) = match item.status {
        ChecklistStatus::Pass => ("[PASS]", builder.palette.link_green),
        ChecklistStatus::Warn => ("[WARN]", builder.palette.accent),
        ChecklistStatus::Violated => ("[VIOLATED]", builder.palette.link_red),
        ChecklistStatus::Info => ("[INFO]", builder.palette.muted),
    };
    let head = format!("{tag} {}", item.assumption);
    for wrapped in wrap_text_to_width(builder.measurer, &head, bold_style, locale, max_w_px) {
        out.push(DrawerLine { text: wrapped, bold: true, color });
    }
    if let Some(metric) = &item.diagnostic_metric {
        for wrapped in wrap_text_to_width(builder.measurer, metric, reg_style, locale, max_w_px) {
            out.push(DrawerLine { text: wrapped, bold: false, color: builder.palette.text });
        }
    }
    let clean_note = sanitize_diagnostics_only_text(item);
    for wrapped in wrap_text_to_width(builder.measurer, &clean_note, reg_style, locale, max_w_px) {
        out.push(DrawerLine { text: wrapped, bold: false, color: builder.palette.muted });
    }
}

fn append_narrative_lines(
    builder: &LayoutBuilder<'_>,
    rs: &ResultSheet,
    reg_style: &UiTextStyle,
    bold_style: &UiTextStyle,
    max_w_px: f64,
    out: &mut Vec<DrawerLine>,
) {
    let locale = &builder.tree.locale;
    let proc_title = rs.analysis.kind.derive_auto_title();
    for w in wrap_text_to_width(builder.measurer, &format!("Analysis: {proc_title}"), bold_style, locale, max_w_px) {
        out.push(DrawerLine { text: w, bold: true, color: builder.palette.text });
    }
    let status_line = match &rs.status {
        ResultStatus::UpToDate => format!("Status: Up to date | {} tab(s) | {} checklist item(s)", rs.tabs.len(), rs.checklist.len()),
        ResultStatus::Dirty => String::from("Status: Recalculation pending"),
        ResultStatus::Failed { code, message } => format!("Status: Failed ({}) - {message}", code.as_str()),
    };
    for w in wrap_text_to_width(builder.measurer, &status_line, reg_style, locale, max_w_px) {
        out.push(DrawerLine { text: w, bold: false, color: builder.palette.muted });
    }
    if !rs.pairwise_comparisons.is_empty() {
        let sig_n = rs.pairwise_comparisons.iter().filter(|p| p.significant).count();
        let line = format!("Pairwise comparisons: {sig_n} of {} comparisons below threshold.", rs.pairwise_comparisons.len());
        for w in wrap_text_to_width(builder.measurer, &line, reg_style, locale, max_w_px) {
            out.push(DrawerLine { text: w, bold: false, color: builder.palette.text });
        }
    }
    for scalar in rs.scalars.iter().take(8) {
        if scalar.value.is_finite() {
            let unit_suffix = scalar.units.as_deref().map_or_else(String::new, |u| format!(" {u}"));
            let txt = format!("- {}: {:.4}{unit_suffix}", scalar.key, scalar.value);
            for w in wrap_text_to_width(builder.measurer, &txt, reg_style, locale, max_w_px) {
                out.push(DrawerLine { text: w, bold: false, color: builder.palette.text });
            }
        }
    }
}

fn append_methods_lines(
    builder: &LayoutBuilder<'_>,
    rs: &ResultSheet,
    reg_style: &UiTextStyle,
    bold_style: &UiTextStyle,
    max_w_px: f64,
    out: &mut Vec<DrawerLine>,
) {
    let locale = &builder.tree.locale;
    let summary = format!(
        "{} | CI: {}% | Source: {}",
        rs.analysis.kind.derive_auto_title(),
        rs.analysis.confidence_level.percent(),
        rs.analysis.source_sheet_id.as_str()
    );
    for w in wrap_text_to_width(builder.measurer, &summary, bold_style, locale, max_w_px) {
        out.push(DrawerLine { text: w, bold: true, color: builder.palette.text });
    }
    if let Some(report) = &builder.app.canvas_dialogs.result_view.last_methods_report {
        for raw_line in report.lines().take(36) {
            for w in wrap_text_to_width(builder.measurer, raw_line, reg_style, locale, max_w_px) {
                out.push(DrawerLine { text: w, bold: false, color: builder.palette.text });
            }
        }
    } else {
        let hint = "Click Copy Methods Report above to generate Plain, Markdown, or LaTeX reporting text.";
        for w in wrap_text_to_width(builder.measurer, hint, reg_style, locale, max_w_px) {
            out.push(DrawerLine { text: w, bold: false, color: builder.palette.muted });
        }
    }
}

fn wrap_text_to_width(measurer: &dyn TextMeasurer, text: &str, style: &UiTextStyle, locale: &str, max_width_px: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let max_w = max_width_px.max(24.0);
    for paragraph in text.lines() {
        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut current = String::new();
        for word in trimmed.split_whitespace() {
            let candidate = if current.is_empty() { word.to_owned() } else { format!("{current} {word}") };
            if !current.is_empty() && measurer.measure(&candidate, style, locale).width_px > max_w {
                lines.push(current);
                current = word.to_owned();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() && !text.trim().is_empty() {
        lines.push(text.trim().to_owned());
    }
    lines
}
