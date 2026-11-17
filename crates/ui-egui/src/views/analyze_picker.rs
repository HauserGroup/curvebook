//! `Analyze...` searchable categorized tree and duplicate-analysis disambiguation view (`R10.3`).

use curvebook_doc::{SheetId, TableKind};
use curvebook_palette::Rgba8;
use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{
    ANALYSIS_CATALOG, ActiveDialog, AnalysisCatalogItem, AnalysisCategory, AnalyzeDuplicateChoice, default_analysis_kind_for_id,
};
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta, scrollbar_thickness_px};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldSpec, ToggleWidgetSpec, emit_button, emit_scrollbars_for_meta, emit_text_field, emit_toggle,
    maybe_paint_focus_ring, paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::{ControlEnablement, r10_pcr_outcome_column};
use crate::text_bridge::{TextMeasurer, UiTextStyle};

use super::a11y::{disclosure_meta, labeled_meta, selectable_meta};

const SEARCH_WIDGET_ID: &str = "analyze.search";
const CLEAR_BUTTON_ID: &str = "analyze.search.clear";
const ENTRIES_SCROLL_ID: &str = "scroll.analyze.entries";
const DUP_TARGETS_SCROLL_ID: &str = "scroll.analyze_picker.dup_targets";

/// User choice in the re-analysis disambiguation prompt (`DuplicateAnalysisPrompt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DuplicateChoice {
    ModifyExisting,
    #[default]
    CreateNew,
}

/// Transient view state owned by the `Analyze...` picker shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnalyzePickerViewState {
    pub search_query: String,
    #[serde(default)]
    pub collapsed_categories: Vec<String>,
    #[serde(default)]
    pub duplicate_choice: DuplicateChoice,
    #[serde(default)]
    pub duplicate_target_id: Option<String>,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `analyze_picker` domain.
///
/// # Errors
/// Returns an error if an explicit duplicate-analysis execution command fails.
pub fn handle_custom_action(app: &mut UiApp, state: &mut AnalyzePickerViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if payload == "clear_search" {
        app.interaction.text_field_buffers.remove(SEARCH_WIDGET_ID);
        app.canvas_dialogs.analyze.search_query.clear();
        state.search_query.clear();
        app.interaction.scroll_offsets.offsets.insert(String::from(ENTRIES_SCROLL_ID), [0.0, 0.0]);
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.clear_search"), ..Default::default() });
    }
    if let Some(slug) = payload.strip_prefix("toggle_category:") {
        sync_search_text(app, state);
        if let Some(pos) = state.collapsed_categories.iter().position(|s| s == slug) {
            state.collapsed_categories.remove(pos);
        } else if !slug.is_empty() {
            state.collapsed_categories.push(slug.to_owned());
        }
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.toggle_category:{slug}"), ..Default::default() });
    }
    if payload == "expand_all" {
        state.collapsed_categories.clear();
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.expand_all"), ..Default::default() });
    }
    if payload == "collapse_all" {
        state.collapsed_categories = AnalysisCategory::ALL.into_iter().map(|c| String::from(category_slug(c))).collect();
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.collapse_all"), ..Default::default() });
    }
    if payload == "dup_choice:modify_existing" {
        state.duplicate_choice = DuplicateChoice::ModifyExisting;
        if state.duplicate_target_id.is_none()
            && let ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } = &app.canvas_dialogs.active_dialog
        {
            state.duplicate_target_id = existing_result_ids.first().map(|id| id.as_str().to_owned());
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.dup_choice:modify_existing"), ..Default::default() });
    }
    if payload == "dup_choice:create_new" {
        state.duplicate_choice = DuplicateChoice::CreateNew;
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.dup_choice:create_new"), ..Default::default() });
    }
    if let Some(target_id) = payload.strip_prefix("dup_target:") {
        state.duplicate_target_id = Some(target_id.to_owned());
        state.duplicate_choice = DuplicateChoice::ModifyExisting;
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.dup_target:{target_id}"), ..Default::default() });
    }
    if payload == "dup_confirm" {
        return execute_duplicate_confirm(app, state);
    }
    if let Some(col_raw) = payload.strip_prefix("toggle_pca_pred:")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        app.canvas_dialogs.analyze.toggle_pca_predictor(col);
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.toggle_pca_pred:{col}"), ..Default::default() });
    }
    if let Some(col_raw) = payload.strip_prefix("set_pcr_outcome:")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        app.canvas_dialogs.analyze.select_pcr_outcome(col)?;
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.set_pcr_outcome:{col}"), ..Default::default() });
    }
    if payload == "outlier_mode:rout" {
        app.canvas_dialogs.analyze.outlier_method = String::from("rout");
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.outlier_mode:rout"), ..Default::default() });
    }
    if payload == "outlier_mode:grubbs" {
        app.canvas_dialogs.analyze.outlier_method = String::from("grubbs");
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.outlier_mode:grubbs"), ..Default::default() });
    }
    if let Some(delta_raw) = payload.strip_prefix("outlier_param_step:")
        && let Ok(delta) = delta_raw.parse::<f64>()
    {
        let an = &mut app.canvas_dialogs.analyze;
        if an.outlier_method == "grubbs" {
            an.outlier_alpha = (an.outlier_alpha + delta).clamp(0.001, 0.25);
        } else {
            an.outlier_fdr_q_percent = (an.outlier_fdr_q_percent + delta).clamp(0.1, 20.0);
        }
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.outlier_param_step:{delta_raw}"), ..Default::default() });
    }
    if payload == "outlier_toggle_iterative" {
        let an = &mut app.canvas_dialogs.analyze;
        if an.outlier_method == "grubbs" {
            an.outlier_iterative = !an.outlier_iterative;
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.outlier_toggle_iterative"), ..Default::default() });
    }
    Ok(UiEventOutcome::default())
}

fn sync_search_text(app: &mut UiApp, state: &mut AnalyzePickerViewState) {
    if let Some(buf) = app.interaction.text_field_buffers.get(SEARCH_WIDGET_ID) {
        state.search_query = buf.text.clone();
        app.canvas_dialogs.analyze.search_query = buf.text.clone();
    }
}

fn execute_duplicate_confirm(app: &mut UiApp, state: &mut AnalyzePickerViewState) -> Result<UiEventOutcome, String> {
    let existing = match &app.canvas_dialogs.active_dialog {
        ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } => existing_result_ids.clone(),
        _ => return Err(String::from("DuplicateAnalysisPrompt dialog is not open")),
    };
    let intent = resolve_duplicate_intent(app, state, &existing).ok_or_else(|| String::from("failed to resolve duplicate analysis choice"))?;
    let cmd_id = intent.command_id.clone();
    let outcome = app.execute_command(&cmd_id, intent.params).map_err(|e| e.to_string())?;
    if let Some(new_id) = outcome.payload.get("sheet_id").and_then(serde_json::Value::as_str) {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
    }
    app.canvas_dialogs.close_dialog();
    state.duplicate_choice = DuplicateChoice::CreateNew;
    state.duplicate_target_id = None;
    let result_json = serde_json::to_value(&outcome.payload).ok();
    Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.dup_confirm"), command_id: Some(cmd_id), command_result: result_json })
}

/// Resolve the active [`AnalyzePickerViewState`] choice into an engine [`ToolCommandIntent`].
#[must_use]
pub fn resolve_duplicate_intent(app: &UiApp, state: &AnalyzePickerViewState, existing_result_ids: &[SheetId]) -> Option<ToolCommandIntent> {
    let choice = match state.duplicate_choice {
        DuplicateChoice::ModifyExisting => {
            let target = state
                .duplicate_target_id
                .as_deref()
                .and_then(|tid| existing_result_ids.iter().find(|id| id.as_str() == tid))
                .or_else(|| existing_result_ids.first())?;
            AnalyzeDuplicateChoice::ModifyExisting(target.clone())
        }
        DuplicateChoice::CreateNew => AnalyzeDuplicateChoice::CreateNew,
    };
    app.canvas_dialogs.analyze.resolve_duplicate_choice(&choice)
}

/// Build the searchable, categorized `Analyze Data` tree, filter sidebar, and summary pane inside `body_rect`.
pub fn build_analyze_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let gap = scaled_u32(6.0, scale, 4, 14);
    let top_h = scaled_u32(28.0, scale, 22, 48).min(body_rect.height);
    let live_query = effective_search_query(builder.app);

    let top_rect = LayoutRectPx::new(body_rect.x, body_rect.y, body_rect.width, top_h);
    build_search_and_archetype_bar(builder, &parent, top_rect, live_query);

    let used_top = top_h.saturating_add(gap);
    let remaining_h = body_rect.height.saturating_sub(used_top);
    if remaining_h < 24 {
        return;
    }

    let show_pcr_panel = matches!(builder.app.canvas_dialogs.analyze.selected_entry_id.as_deref(), Some("stats.pcr" | "stats.pca"))
        && !builder.app.canvas_dialogs.analyze.variable_candidates.is_empty();
    let summary_h = if show_pcr_panel && remaining_h >= scaled_u32(220.0, scale, 160, 440) {
        scaled_u32(118.0, scale, 92, 220)
    } else if remaining_h >= scaled_u32(220.0, scale, 160, 440) {
        scaled_u32(72.0, scale, 54, 140)
    } else if remaining_h >= scaled_u32(140.0, scale, 110, 280) {
        scaled_u32(48.0, scale, 36, 96)
    } else {
        0
    };
    let summary_reserve = if summary_h > 0 { summary_h.saturating_add(gap) } else { 0 };
    let mid_h = remaining_h.saturating_sub(summary_reserve);
    let mid_y = body_rect.y + i32::try_from(used_top).unwrap_or(34);

    if mid_h >= 24 {
        let left_w = (body_rect.width / 3).clamp(scaled_u32(120.0, scale, 100, 240), scaled_u32(220.0, scale, 170, 440));
        let left_rect = LayoutRectPx::new(body_rect.x, mid_y, left_w, mid_h);
        let right_x = left_rect.right() + i32::try_from(gap).unwrap_or(6);
        let right_w = body_rect.width.saturating_sub(left_w.saturating_add(gap));
        let right_rect = LayoutRectPx::new(right_x, mid_y, right_w, mid_h);

        build_category_sidebar(builder, &parent, left_rect, live_query);
        if right_rect.width >= 60 {
            build_categorized_tree(builder, &parent, right_rect, live_query);
        }
    }

    if summary_h >= 24 {
        let sum_y = mid_y + i32::try_from(mid_h.saturating_add(gap)).unwrap_or(0);
        let sum_rect = LayoutRectPx::new(body_rect.x, sum_y, body_rect.width, summary_h);
        build_selected_summary_pane(builder, &parent, sum_rect);
    }
}

/// Build the `DuplicateAnalysisPrompt` modal body when that dialog is active.
pub fn build_duplicate_analysis_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    if let ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id, existing_result_ids, pending_entry_id } =
        &builder.app.canvas_dialogs.active_dialog
    {
        let src = source_sheet_id.clone();
        let existing = existing_result_ids.clone();
        let entry_id = pending_entry_id.clone();
        build_duplicate_analysis_controls(builder, parent, body_rect, &src, &existing, &entry_id);
    }
}

/// Render interactive two-choice re-analysis disambiguation options (`Modify existing result sheet`
/// vs `Create new result sheet`, including target existing result selection when multiple match).
pub fn build_duplicate_analysis_controls(
    builder: &mut LayoutBuilder<'_>,
    parent: WidgetId,
    body_rect: LayoutRectPx,
    source_sheet_id: &SheetId,
    existing_result_ids: &[SheetId],
    pending_entry_id: &str,
) {
    if body_rect.width < 60 || body_rect.height < 40 {
        return;
    }
    let scale = builder.scale();
    let locale = builder.app.locale.as_str();
    let gap = scaled_u32(6.0, scale, 4, 14);
    let line_h = scaled_u32(20.0, scale, 16, 40);
    let hdr_h = line_h.saturating_mul(2).saturating_add(scaled_u32(10.0, scale, 6, 20)).min(body_rect.height);

    let hdr_rect = LayoutRectPx::new(body_rect.x, body_rect.y, body_rect.width, hdr_h);
    let hdr_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze_picker.dup.prompt_header"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: hdr_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, "Re-analysis disambiguation", FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&hdr_wid), hdr_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&hdr_wid), hdr_rect, builder.palette.border, 1);

    let entry_label = ANALYSIS_CATALOG.iter().find(|it| it.id == pending_entry_id).map_or(pending_entry_id, |it| it.label);
    let source_title = builder
        .app
        .document()
        .data_sheet(source_sheet_id)
        .map(|d| d.title.as_str())
        .or_else(|| builder.app.document().result_sheet(source_sheet_id).map(curvebook_doc::ResultSheet::title))
        .unwrap_or(source_sheet_id.as_str());
    let prompt_title = picker_tr(locale, "analyze_picker.dup.prompt_title", "An analysis of this family already exists on the selected table.");
    let subtitle = format!("{entry_label} — {source_title} ({} matching)", existing_result_ids.len());
    let size_bold = (12.0 * scale).clamp(9.0, 24.0);
    let size_sub = size_bold;
    let pad_y = i32::try_from(scaled_u32(4.0, scale, 2, 10)).unwrap_or(4);

    builder.push_clip(hdr_rect);
    paint_text_fitted(
        builder,
        &hdr_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(hdr_rect.x + 4, hdr_rect.y + pad_y, hdr_rect.width.saturating_sub(8), line_h),
            pad_x: 6,
            text: prompt_title,
            style: &UiTextStyle::ui_bold(size_bold),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    paint_text_fitted(
        builder,
        &hdr_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(
                hdr_rect.x + 4,
                hdr_rect.y + pad_y + i32::try_from(line_h).unwrap_or(20),
                hdr_rect.width.saturating_sub(8),
                line_h,
            ),
            pad_x: 6,
            text: &subtitle,
            style: &UiTextStyle::ui_sans(size_sub),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    builder.pop_clip();

    let state = &builder.app.view_shards.analyze_picker;
    let row_h = scaled_u32(26.0, scale, 20, 48);
    let mut y = hdr_rect.bottom() + i32::try_from(gap).unwrap_or(6);

    if y + i32::try_from(row_h).unwrap_or(26) <= body_rect.bottom() {
        let mod_en = if existing_result_ids.is_empty() {
            ControlEnablement::disabled("No matching existing result sheet is available to modify.")
        } else {
            ControlEnablement::enabled()
        };
        let mod_label = picker_tr(locale, "analyze_picker.dup.modify_existing", "Modify existing result sheet");
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "analyze_picker.dup.modify_existing",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(body_rect.x, y, body_rect.width, row_h - 2),
                label: mod_label,
                keyboard_path: "analyze_picker.dup.modify_existing",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(10),
                checked: state.duplicate_choice == DuplicateChoice::ModifyExisting,
                is_radio: true,
                enablement: mod_en,
                scroll_container: None,
                action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: String::from("dup_choice:modify_existing") },
            },
        );
        y += i32::try_from(row_h).unwrap_or(26);
    }

    if y + i32::try_from(row_h).unwrap_or(26) <= body_rect.bottom() {
        let new_label = picker_tr(locale, "analyze_picker.dup.create_new", "Create new result sheet");
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "analyze_picker.dup.create_new",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(body_rect.x, y, body_rect.width, row_h - 2),
                label: new_label,
                keyboard_path: "analyze_picker.dup.create_new",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(11),
                checked: state.duplicate_choice == DuplicateChoice::CreateNew,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: String::from("dup_choice:create_new") },
            },
        );
        y += i32::try_from(row_h + gap).unwrap_or(32);
    }

    if !existing_result_ids.is_empty() && y + i32::try_from(line_h + 24).unwrap_or(44) <= body_rect.bottom() {
        build_duplicate_targets_list(
            builder,
            &parent,
            LayoutRectPx::new(body_rect.x, y, body_rect.width, (body_rect.bottom() - y) as u32),
            existing_result_ids,
        );
    }
}

fn build_duplicate_targets_list(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, existing_result_ids: &[SheetId]) {
    let scale = builder.scale();
    let locale = builder.app.locale.as_str();
    let label_h = scaled_u32(20.0, scale, 16, 38);
    let target_title = picker_tr(locale, "analyze_picker.dup.target_sheet", "Target existing result sheet");

    let lbl_rect = LayoutRectPx::new(rect.x, rect.y, rect.width, label_h);
    let lbl_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze_picker.dup.target_label"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: lbl_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, target_title, FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &lbl_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: lbl_rect,
            pad_x: 4,
            text: target_title,
            style: &UiTextStyle::ui_bold((11.5 * scale).clamp(9.0, 22.0)),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let list_y = rect.y + i32::try_from(label_h + 2).unwrap_or(22);
    let list_h = u32::try_from((rect.bottom() - list_y).max(0)).unwrap_or(0);
    if list_h < 22 {
        return;
    }
    let list_rect = LayoutRectPx::new(rect.x, list_y, rect.width, list_h);
    let row_h = scaled_u32(24.0, scale, 20, 44);
    let content_h = (existing_result_ids.len() as u32).saturating_mul(row_h) + scaled_u32(8.0, scale, 4, 16);
    let scroll_id = String::from(DUP_TARGETS_SCROLL_ID);
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), list_rect, [list_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let targets_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze_picker.dup.targets_list"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: list_rect,
        a11y: labeled_meta(WidgetRole::Dialog, target_title, FocusScope::ModalDialog, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&targets_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&targets_wid), list_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&targets_wid), list_rect, builder.palette.border, 1);

    let sb_reserve = scrollbar_thickness_px(scale) + scaled_u32(4.0, scale, 2, 10);
    let list_clip =
        LayoutRectPx::new(list_rect.x + 2, list_rect.y + 2, list_rect.width.saturating_sub(sb_reserve), list_rect.height.saturating_sub(4));
    builder.push_clip(list_clip);

    let state = &builder.app.view_shards.analyze_picker;
    let active_target = state
        .duplicate_target_id
        .as_deref()
        .filter(|tid| existing_result_ids.iter().any(|id| id.as_str() == *tid))
        .or_else(|| existing_result_ids.first().map(SheetId::as_str));
    #[allow(clippy::cast_possible_truncation)]
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let row_h_i32 = i32::try_from(row_h).unwrap_or(24);

    for (idx, res_id) in existing_result_ids.iter().enumerate() {
        let ry = list_clip.y + i32::try_from((idx as u32).saturating_mul(row_h)).unwrap_or(0) - y_scroll;
        if ry + row_h_i32 <= list_clip.y + 2 || ry >= list_clip.bottom() - 2 {
            continue;
        }
        let item_rect = LayoutRectPx::new(list_clip.x, ry, list_clip.width, row_h.saturating_sub(2));
        let label = builder
            .app
            .document()
            .result_sheet(res_id)
            .map_or_else(|| res_id.as_str().to_owned(), |rs| format!("{} ({})", rs.title(), res_id.as_str()));
        let checked = state.duplicate_choice == DuplicateChoice::ModifyExisting && active_target == Some(res_id.as_str());
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: &format!("analyze_picker.dup.target.{}", res_id.as_str()),
                parent: Some(targets_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: item_rect,
                label: &label,
                keyboard_path: &format!("analyze_picker.dup.target.{idx}"),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(30 + idx as u32),
                checked,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&scroll_id),
                action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: format!("dup_target:{}", res_id.as_str()) },
            },
        );
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(targets_wid));
}

fn build_search_and_archetype_bar(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, live_query: &str) {
    let scale = builder.scale();
    let locale = builder.app.locale.as_str();
    let gap = scaled_u32(6.0, scale, 4, 14);
    let badge_w = (rect.width / 4).clamp(scaled_u32(96.0, scale, 80, 192), scaled_u32(170.0, scale, 130, 340));
    let clear_w = scaled_u32(68.0, scale, 54, 136);
    let search_w = rect.width.saturating_sub(badge_w.saturating_add(clear_w).saturating_add(gap.saturating_mul(2))).max(64);

    let badge_rect = LayoutRectPx::new(rect.x, rect.y, badge_w, rect.height);
    let search_rect = LayoutRectPx::new(badge_rect.right() + i32::try_from(gap).unwrap_or(6), rect.y, search_w, rect.height);
    let clear_rect = LayoutRectPx::new(search_rect.right() + i32::try_from(gap).unwrap_or(6), rect.y, clear_w, rect.height);

    let table_kind = builder.app.canvas_dialogs.analyze.source_table_kind;
    let badge_text = table_kind.map_or("Sheet", |kind| crate::i18n::table_kind_label(locale, kind));
    let badge_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze.archetype_badge"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: badge_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, badge_text, FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&badge_wid), badge_rect, builder.palette.header);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&badge_wid), badge_rect, builder.palette.border, 1);
    paint_text_fitted(
        builder,
        &badge_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: badge_rect,
            pad_x: 6,
            text: badge_text,
            style: &UiTextStyle::ui_bold((12.0 * scale).clamp(9.0, 24.0)),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let search_label = picker_tr(locale, "analyze_picker.search_analyses", "Search analyses");
    emit_text_field(
        builder,
        TextFieldSpec {
            id: SEARCH_WIDGET_ID,
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: search_rect,
            label: search_label,
            fallback_text: live_query,
            keyboard_path: SEARCH_WIDGET_ID,
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(1),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );

    let clear_label = picker_tr(locale, "analyze_picker.clear", "Clear");
    let clear_en = if live_query.is_empty() { ControlEnablement::disabled("Search filter is empty.") } else { ControlEnablement::enabled() };
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: CLEAR_BUTTON_ID,
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: clear_rect,
            label: clear_label,
            keyboard_path: CLEAR_BUTTON_ID,
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(2),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: clear_en,
            action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: String::from("clear_search") },
        },
    );
}

fn build_category_sidebar(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, left_rect: LayoutRectPx, live_query: &str) {
    let scale = builder.scale();
    let locale = builder.app.locale.as_str();
    let cat_row_h = scaled_u32(24.0, scale, 20, 48);
    let table_kind = builder.app.canvas_dialogs.analyze.source_table_kind;
    let terms = split_query_terms(live_query);
    let mut cy = left_rect.y;

    for (idx, cat_opt) in std::iter::once(None).chain(AnalysisCategory::ALL.into_iter().map(Some)).enumerate() {
        if cy + i32::try_from(cat_row_h).unwrap_or(24) > left_rect.bottom() {
            break;
        }
        let cat_val = cat_opt.map_or("all", category_slug);
        let base_label =
            cat_opt.map_or_else(|| picker_tr(locale, "analyze_picker.all_compatible", "All Compatible"), |c| localized_category_label(locale, c));
        let compat_count = count_entries(table_kind, cat_opt, &[]);
        let match_count = count_entries(table_kind, cat_opt, &terms);
        let label = format!("{base_label} ({match_count})");
        let selected = builder.app.canvas_dialogs.analyze.category_filter == cat_opt;
        let en = if cat_opt.is_none() || compat_count > 0 {
            ControlEnablement::enabled()
        } else {
            let table_name = table_kind.map_or("active", TableKind::label);
            ControlEnablement::disabled(format!("No {base_label} analyses are compatible with {table_name} tables."))
        };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("analyze.cat.{cat_val}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(left_rect.x, cy, left_rect.width, cat_row_h.saturating_sub(2)),
                label: &label,
                keyboard_path: &format!("analyze.cat.{idx}"),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(10 + idx as u32),
                primary: false,
                selected,
                shortcut: None,
                enablement: en,
                action: WidgetAction::SelectAnalyzeCategory(cat_val.to_owned()),
            },
        );
        cy += i32::try_from(cat_row_h).unwrap_or(24);
    }
}

enum TreeRow {
    GroupHeader { cat: AnalysisCategory, slug: &'static str, count: usize, collapsed: bool },
    Entry { item: &'static AnalysisCatalogItem, flat_index: usize },
}

fn build_categorized_tree(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, right_rect: LayoutRectPx, live_query: &str) {
    let scale = builder.scale();
    let locale = builder.app.locale.as_str();
    let table_kind = builder.app.canvas_dialogs.analyze.source_table_kind;
    let cat_filter = builder.app.canvas_dialogs.analyze.category_filter;
    let collapsed_list = &builder.app.view_shards.analyze_picker.collapsed_categories;
    let terms = split_query_terms(live_query);

    let mut rows = Vec::new();
    let mut flat_entry_idx = 0usize;
    for cat in AnalysisCategory::ALL {
        if cat_filter.is_some_and(|f| f != cat) {
            continue;
        }
        let items: Vec<&'static AnalysisCatalogItem> =
            ANALYSIS_CATALOG.iter().filter(|item| item.category == cat && entry_matches(item, table_kind, &terms)).collect();
        if items.is_empty() {
            continue;
        }
        let slug = category_slug(cat);
        let collapsed = collapsed_list.iter().any(|s| s == slug);
        rows.push(TreeRow::GroupHeader { cat, slug, count: items.len(), collapsed });
        if !collapsed {
            for item in items {
                rows.push(TreeRow::Entry { item, flat_index: flat_entry_idx });
                flat_entry_idx += 1;
            }
        }
    }

    let row_h = scaled_u32(22.0, scale, 18, 44);
    let content_h =
        if rows.is_empty() { scaled_u32(48.0, scale, 36, 96) } else { (rows.len() as u32).saturating_mul(row_h) + scaled_u32(8.0, scale, 4, 16) };
    let scroll_id = String::from(ENTRIES_SCROLL_ID);
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), right_rect, [right_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let list_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze.entries_list"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: right_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Compatible Analyses", FocusScope::ModalDialog, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&list_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&list_wid), right_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&list_wid), right_rect, builder.palette.border, 1);

    let sb_reserve = scrollbar_thickness_px(scale) + scaled_u32(4.0, scale, 2, 10);
    let list_clip =
        LayoutRectPx::new(right_rect.x + 2, right_rect.y + 2, right_rect.width.saturating_sub(sb_reserve), right_rect.height.saturating_sub(4));
    builder.push_clip(list_clip);

    if rows.is_empty() {
        emit_empty_tree_feedback(builder, &list_wid, list_clip, table_kind, live_query, &scroll_id);
    } else {
        emit_tree_rows(builder, &list_wid, list_clip, &rows, row_h, &scroll_meta, &scroll_id, locale);
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(list_wid));
}

#[allow(clippy::too_many_arguments)]
fn emit_tree_rows(
    builder: &mut LayoutBuilder<'_>,
    list_wid: &WidgetId,
    list_clip: LayoutRectPx,
    rows: &[TreeRow],
    row_h: u32,
    scroll_meta: &ScrollMeta,
    scroll_id: &str,
    locale: &str,
) {
    let scale = builder.scale();
    #[allow(clippy::cast_possible_truncation)]
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (12.0 * scale).clamp(9.0, 24.0);
    let row_h_i32 = i32::try_from(row_h).unwrap_or(22);
    let indent = scaled_u32(14.0, scale, 8, 28);

    for (row_idx, row) in rows.iter().enumerate() {
        let ry = list_clip.y + i32::try_from((row_idx as u32).saturating_mul(row_h)).unwrap_or(0) - y_scroll;
        if ry + row_h_i32 <= list_clip.y + 2 || ry >= list_clip.bottom() - 2 {
            continue;
        }
        let tab_idx = 100 + row_idx as u32;
        match row {
            TreeRow::GroupHeader { cat, slug, count, collapsed } => {
                let hdr_rect = LayoutRectPx::new(list_clip.x, ry, list_clip.width, row_h.saturating_sub(2));
                let id = format!("analyze.tree.group.{slug}");
                let st = widget_state(builder, &id, ControlEnablement::enabled());
                let cat_label = localized_category_label(locale, *cat);
                let marker = if *collapsed { "[+]" } else { "[-]" };
                let text = format!("{marker} {cat_label} ({count})");
                let a11y =
                    disclosure_meta(cat_label, FocusScope::ModalDialog, format!("analyze.tree.group.{slug}"), !collapsed).with_tab_order(tab_idx);
                let wid = builder.push_node(NodeSpec {
                    id: WidgetId::from(id),
                    parent: Some(list_wid.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect_px: hdr_rect,
                    a11y,
                    state: st.clone(),
                    action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: format!("toggle_category:{slug}") },
                });
                let bg = if st.hovered { builder.palette.header } else { builder.palette.panel };
                builder.paint_fill(ZLayer::ModalDialog, Some(&wid), hdr_rect, bg);
                maybe_paint_focus_ring(builder, ZLayer::ModalDialog, &wid, hdr_rect, st.focused);
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec {
                        z_layer: ZLayer::ModalDialog,
                        rect: hdr_rect,
                        pad_x: 4,
                        text: &text,
                        style: &UiTextStyle::ui_bold(size_px),
                        color: builder.palette.text,
                        scroll_container: Some(scroll_id),
                    },
                );
            }
            TreeRow::Entry { item, flat_index } => {
                let item_rect = LayoutRectPx::new(
                    list_clip.x + i32::try_from(indent).unwrap_or(14),
                    ry,
                    list_clip.width.saturating_sub(indent),
                    row_h.saturating_sub(2),
                );
                let id = format!("analyze.entry.{}", item.id);
                let selected = builder.app.canvas_dialogs.analyze.selected_entry_id.as_deref() == Some(item.id);
                let st = widget_state(builder, &id, ControlEnablement::enabled());
                let mut a11y =
                    selectable_meta(WidgetRole::TreeItem, item.label, FocusScope::ModalDialog, format!("analyze.entry.{flat_index}"), selected);
                a11y.checked = Some(selected);
                a11y.tab_order = Some(tab_idx);
                let wid = builder.push_node(NodeSpec {
                    id: WidgetId::from(id),
                    parent: Some(list_wid.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect_px: item_rect,
                    a11y,
                    state: st.clone(),
                    action: WidgetAction::SelectAnalyzeEntry(item.id.to_owned()),
                });
                let bg = if selected {
                    builder.palette.accent
                } else if st.hovered {
                    builder.palette.header
                } else {
                    builder.palette.surface
                };
                let fg = if selected { Rgba8::WHITE } else { builder.palette.text };
                builder.paint_fill(ZLayer::ModalDialog, Some(&wid), item_rect, bg);
                maybe_paint_focus_ring(builder, ZLayer::ModalDialog, &wid, item_rect, st.focused);
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec {
                        z_layer: ZLayer::ModalDialog,
                        rect: item_rect,
                        pad_x: 6,
                        text: item.label,
                        style: &UiTextStyle::ui_sans(size_px),
                        color: fg,
                        scroll_container: Some(scroll_id),
                    },
                );
            }
        }
    }
}

fn emit_empty_tree_feedback(
    builder: &mut LayoutBuilder<'_>,
    list_wid: &WidgetId,
    list_clip: LayoutRectPx,
    table_kind: Option<TableKind>,
    live_query: &str,
    scroll_id: &str,
) {
    let scale = builder.scale();
    let table_label = table_kind.map_or("active sheet", TableKind::label);
    let q = live_query.trim();
    let msg = if q.is_empty() {
        format!("No analyses in the selected category are compatible with {table_label} tables.")
    } else {
        format!("No analyses compatible with {table_label} match \"{q}\".")
    };
    let msg_rect = LayoutRectPx::new(list_clip.x + 4, list_clip.y + 4, list_clip.width.saturating_sub(8), scaled_u32(28.0, scale, 20, 56));
    let empty_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze.empty_feedback"),
        parent: Some(list_wid.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: msg_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, msg.clone(), FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &empty_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: msg_rect,
            pad_x: 6,
            text: &msg,
            style: &UiTextStyle::ui_sans((12.0 * scale).clamp(9.0, 24.0)),
            color: builder.palette.muted,
            scroll_container: Some(scroll_id),
        },
    );
}

fn build_selected_summary_pane(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, sum_rect: LayoutRectPx) {
    let scale = builder.scale();
    let locale = builder.app.locale.as_str();
    let selected_item = builder.app.canvas_dialogs.analyze.selected_entry_id.as_deref().and_then(|id| ANALYSIS_CATALOG.iter().find(|it| it.id == id));

    let title_str = selected_item.map_or_else(
        || String::from(picker_tr(locale, "analyze_picker.selected_analysis", "Selected Analysis")),
        |item| format!("{}  [{}]", item.label, localized_category_label(locale, item.category)),
    );

    let sum_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze.summary_pane"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: sum_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, title_str.clone(), FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&sum_wid), sum_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&sum_wid), sum_rect, builder.palette.border, 1);

    let show_pcr_panel = matches!(builder.app.canvas_dialogs.analyze.selected_entry_id.as_deref(), Some("stats.pcr" | "stats.pca"))
        && !builder.app.canvas_dialogs.analyze.variable_candidates.is_empty()
        && sum_rect.width >= 320
        && sum_rect.height >= 72;
    let show_outlier_panel =
        builder.app.canvas_dialogs.analyze.selected_entry_id.as_deref() == Some("stats.outliers") && sum_rect.width >= 320 && sum_rect.height >= 72;
    let text_rect = if show_pcr_panel || show_outlier_panel {
        let left_w = (sum_rect.width * 42) / 100;
        LayoutRectPx::new(sum_rect.x, sum_rect.y, left_w, sum_rect.height)
    } else {
        sum_rect
    };

    builder.push_clip(text_rect);
    let line_h = scaled_u32(20.0, scale, 15, 40);
    let line_gap = scaled_u32(2.0, scale, 1, 6);
    let pad_top = i32::try_from(scaled_u32(4.0, scale, 2, 10)).unwrap_or(4);
    let y0 = text_rect.y + pad_top;
    let bold_style = UiTextStyle::ui_bold((12.0 * scale).clamp(9.0, 24.0));
    let body_style = UiTextStyle::ui_sans((12.0 * scale).clamp(9.0, 24.0));

    if y0 + i32::try_from(line_h).unwrap_or(20) <= text_rect.bottom() {
        paint_text_fitted(
            builder,
            &sum_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(text_rect.x + 2, y0, text_rect.width.saturating_sub(4), line_h),
                pad_x: 6,
                text: &title_str,
                style: &bold_style,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
    }

    let body_text = selected_item.map_or_else(
        || picker_tr(locale, "analyze_picker.no_selection", "Select an analysis from the list to view its summary and run it."),
        |item| item.summary,
    );
    let avail_w = f64::from(text_rect.width.saturating_sub(16));
    let wrapped = wrap_two_lines(builder, body_text, &body_style, locale, avail_w);

    let y1 = y0 + i32::try_from(line_h + line_gap).unwrap_or(22);
    if !wrapped.0.is_empty() && y1 + i32::try_from(line_h).unwrap_or(20) <= text_rect.bottom() {
        paint_text_fitted(
            builder,
            &sum_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(text_rect.x + 2, y1, text_rect.width.saturating_sub(4), line_h),
                pad_x: 6,
                text: &wrapped.0,
                style: &body_style,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
    }

    let y2 = y1 + i32::try_from(line_h + line_gap).unwrap_or(22);
    if y2 + i32::try_from(line_h).unwrap_or(20) <= text_rect.bottom() {
        let line3 = if !wrapped.1.is_empty() {
            wrapped.1
        } else if let Some(item) = selected_item {
            format!("ID: {} | Tags: {}", item.id, item.keywords)
        } else {
            String::new()
        };
        if !line3.is_empty() {
            paint_text_fitted(
                builder,
                &sum_wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(text_rect.x + 2, y2, text_rect.width.saturating_sub(4), line_h),
                    pad_x: 6,
                    text: &line3,
                    style: &body_style,
                    color: builder.palette.muted,
                    scroll_container: None,
                },
            );
        }
    }

    builder.pop_clip();

    if show_pcr_panel {
        let pcr_x = text_rect.right() + i32::try_from(scaled_u32(6.0, scale, 4, 12)).unwrap_or(6);
        let pcr_w = sum_rect.width.saturating_sub(text_rect.width.saturating_add(10));
        let pcr_rect = LayoutRectPx::new(pcr_x, sum_rect.y + 4, pcr_w, sum_rect.height.saturating_sub(8));
        build_pcr_predictor_outcome_panel(builder, &sum_wid, pcr_rect);
    } else if show_outlier_panel {
        let out_x = text_rect.right() + i32::try_from(scaled_u32(6.0, scale, 4, 12)).unwrap_or(6);
        let out_w = sum_rect.width.saturating_sub(text_rect.width.saturating_add(10));
        let out_rect = LayoutRectPx::new(out_x, sum_rect.y + 4, out_w, sum_rect.height.saturating_sub(8));
        build_outlier_options_panel(builder, &sum_wid, out_rect);
    }
}

fn build_outlier_options_panel(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, panel_rect: LayoutRectPx) {
    if panel_rect.width < 140 || panel_rect.height < 48 {
        return;
    }
    let locale = builder.app.locale.as_str();
    let an = builder.app.canvas_dialogs.analyze.clone();
    let is_grubbs = an.outlier_method == "grubbs";
    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze.outlier_panel"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: panel_rect,
        a11y: labeled_meta(
            WidgetRole::Dialog,
            picker_tr(locale, "analyze_picker.outlier.section", "Outlier Method"),
            FocusScope::ModalDialog,
            "dialog.analyze.outliers",
        ),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), panel_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&panel_wid), panel_rect, builder.palette.border, 1);

    let half_w = panel_rect.width.saturating_sub(12) / 2;
    let row_h = ((panel_rect.height.saturating_sub(10)) / 3).max(16);
    let rout_label = picker_tr(locale, "analyze_picker.outlier.rout", "ROUT (FDR Q%)");
    let grubbs_label = picker_tr(locale, "analyze_picker.outlier.grubbs", "Grubbs' ESD (alpha)");
    emit_toggle(
        builder,
        ToggleWidgetSpec {
            id: "analyze.outlier.rout",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(panel_rect.x + 4, panel_rect.y + 4, half_w, row_h.saturating_sub(2)),
            label: rout_label,
            keyboard_path: "dialog.analyze.outlier.rout",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(410),
            checked: !is_grubbs,
            is_radio: true,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: String::from("outlier_mode:rout") },
        },
    );
    emit_toggle(
        builder,
        ToggleWidgetSpec {
            id: "analyze.outlier.grubbs",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(panel_rect.x + 8 + half_w as i32, panel_rect.y + 4, half_w, row_h.saturating_sub(2)),
            label: grubbs_label,
            keyboard_path: "dialog.analyze.outlier.grubbs",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(411),
            checked: is_grubbs,
            is_radio: true,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: String::from("outlier_mode:grubbs") },
        },
    );

    let param_y = panel_rect.y + 4 + row_h as i32;
    let btn_w = 24u32.min(panel_rect.width / 6);
    let val_w = panel_rect.width.saturating_sub(btn_w.saturating_mul(2).saturating_add(16));
    let (step, val_text) =
        if is_grubbs { ("0.01", format!("alpha: {:.3}", an.outlier_alpha)) } else { ("0.5", format!("FDR Q: {:.2}%", an.outlier_fdr_q_percent)) };
    let dec_payload = format!("outlier_param_step:-{step}");
    let inc_payload = format!("outlier_param_step:{step}");
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "analyze.outlier.param.dec",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(panel_rect.x + 4, param_y, btn_w, row_h.saturating_sub(2)),
            label: "-",
            keyboard_path: "dialog.analyze.outlier.param.dec",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(412),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: dec_payload },
        },
    );
    let val_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze.outlier.param.value"),
        parent: Some(panel_wid.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: LayoutRectPx::new(panel_rect.x + 8 + btn_w as i32, param_y, val_w, row_h.saturating_sub(2)),
        a11y: AccessibilityMeta::new(WidgetRole::Label, val_text.clone(), FocusScope::ModalDialog),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &val_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(panel_rect.x + 8 + btn_w as i32, param_y, val_w, row_h.saturating_sub(2)),
            pad_x: 4,
            text: &val_text,
            style: &UiTextStyle::ui_sans(11.0),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "analyze.outlier.param.inc",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(panel_rect.x + 12 + (btn_w + val_w) as i32, param_y, btn_w, row_h.saturating_sub(2)),
            label: "+",
            keyboard_path: "dialog.analyze.outlier.param.inc",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(413),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: inc_payload },
        },
    );

    let iter_y = param_y + row_h as i32;
    let iter_label = picker_tr(locale, "analyze_picker.outlier.iterative", "Iterative Grubbs (multiple outliers)");
    let iter_en =
        if is_grubbs { ControlEnablement::enabled() } else { ControlEnablement::disabled("Iterative mode applies only to Grubbs' outlier test.") };
    emit_toggle(
        builder,
        ToggleWidgetSpec {
            id: "analyze.outlier.iterative",
            parent: Some(panel_wid),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(panel_rect.x + 4, iter_y, panel_rect.width.saturating_sub(8), row_h.saturating_sub(2)),
            label: iter_label,
            keyboard_path: "dialog.analyze.outlier.iterative",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(414),
            checked: an.outlier_iterative,
            is_radio: false,
            enablement: iter_en,
            scroll_container: None,
            action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: String::from("outlier_toggle_iterative") },
        },
    );
}

fn build_pcr_predictor_outcome_panel(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, panel_rect: LayoutRectPx) {
    if panel_rect.width < 120 || panel_rect.height < 36 {
        return;
    }
    let scale = builder.scale();
    let analyze = builder.app.canvas_dialogs.analyze.clone();
    let scroll_id = String::from("scroll.analyze.pcr");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let row_h = scaled_u32(22.0, scale, 18, 38);
    let content_h = (analyze.variable_candidates.len() as u32).saturating_mul(row_h).saturating_add(8);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), panel_rect, [panel_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let sb_w = if scroll_meta.has_vertical_overflow() { scrollbar_thickness_px(scale) } else { 0 };
    let inner_w = panel_rect.width.saturating_sub(sb_w.saturating_add(4));
    let pcr_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze.pcr_panel"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: panel_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "PCA predictors and PCR outcome", FocusScope::ModalDialog, "dialog.analyze.pcr"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&pcr_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&pcr_wid), panel_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&pcr_wid), panel_rect, builder.palette.border, 1);

    let inner_rect = LayoutRectPx::new(panel_rect.x + 2, panel_rect.y + 2, inner_w, panel_rect.height.saturating_sub(4));
    let half_w = inner_w.saturating_sub(6) / 2;
    let off_y = scroll_meta.scroll_offset_px[1].round() as i32;

    builder.push_clip(inner_rect);
    for (idx, cand) in analyze.variable_candidates.iter().enumerate() {
        let ry = inner_rect.y + 2 + i32::try_from(idx as u32 * row_h).unwrap_or(0) - off_y;
        let row_r = LayoutRectPx::new(inner_rect.x + 2, ry, inner_w.saturating_sub(4), row_h.saturating_sub(2));
        if row_r.y < inner_rect.y || row_r.bottom() > inner_rect.bottom() {
            continue;
        }
        let col = cand.col_index;
        let pred_id = format!("analyze.pca.pred.{col}");
        let pred_label = format!("Pred: {}", cand.title);
        let pred_rect = LayoutRectPx::new(row_r.x, row_r.y, half_w, row_r.height);
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: &pred_id,
                parent: Some(pcr_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: pred_rect,
                label: &pred_label,
                keyboard_path: "dialog.analyze.pca_pred",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(400 + idx as u32 * 2),
                checked: analyze.selected_data_sets.contains(&col),
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some(&scroll_id),
                action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: format!("toggle_pca_pred:{col}") },
            },
        );

        let out_id = format!("analyze.pcr.outcome.{col}");
        let out_label = format!("Out: {}", cand.title);
        let out_rect = LayoutRectPx::new(pred_rect.right() + 4, row_r.y, half_w, row_r.height);
        let out_enable = r10_pcr_outcome_column(col, &analyze.selected_data_sets);
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: &out_id,
                parent: Some(pcr_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: out_rect,
                label: &out_label,
                keyboard_path: "dialog.analyze.pcr_outcome",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(401 + idx as u32 * 2),
                checked: analyze.pcr_outcome_column == Some(col),
                is_radio: true,
                enablement: out_enable,
                scroll_container: Some(&scroll_id),
                action: WidgetAction::Custom { domain: String::from("analyze_picker"), payload: format!("set_pcr_outcome:{col}") },
            },
        );
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(pcr_wid));
}

fn wrap_two_lines(builder: &LayoutBuilder<'_>, text: &str, style: &UiTextStyle, locale: &str, max_w: f64) -> (String, String) {
    if max_w <= 24.0 || builder.measurer.measure(text, style, locale).width_px <= max_w {
        return (text.to_owned(), String::new());
    }
    let mut line1 = String::new();
    let mut line2 = String::new();
    for word in text.split_whitespace() {
        if line2.is_empty() {
            let cand = if line1.is_empty() { word.to_owned() } else { format!("{line1} {word}") };
            if builder.measurer.measure(&cand, style, locale).width_px <= max_w || line1.is_empty() {
                line1 = cand;
                continue;
            }
        }
        if !line2.is_empty() {
            line2.push(' ');
        }
        line2.push_str(word);
    }
    (line1, line2)
}

fn effective_search_query(app: &UiApp) -> &str {
    if let Some(buf) = app.interaction.text_field_buffers.get(SEARCH_WIDGET_ID) {
        return buf.text.as_str();
    }
    if !app.canvas_dialogs.analyze.search_query.is_empty() {
        return app.canvas_dialogs.analyze.search_query.as_str();
    }
    app.view_shards.analyze_picker.search_query.as_str()
}

fn split_query_terms(query: &str) -> Vec<String> {
    query.trim().to_ascii_lowercase().split_whitespace().map(str::to_owned).collect()
}

fn entry_matches(item: &AnalysisCatalogItem, table_kind: Option<TableKind>, terms: &[String]) -> bool {
    if let Some(tk) = table_kind {
        let Some(kind) = default_analysis_kind_for_id(item.id) else {
            return false;
        };
        if !kind.is_compatible_with(tk) {
            return false;
        }
    }
    if terms.is_empty() {
        return true;
    }
    let haystack = format!("{} {} {} {}", item.id, item.label, item.category.label(), item.keywords).to_ascii_lowercase();
    terms.iter().all(|term| haystack.contains(term.as_str()))
}

fn count_entries(table_kind: Option<TableKind>, cat_opt: Option<AnalysisCategory>, terms: &[String]) -> usize {
    ANALYSIS_CATALOG.iter().filter(|item| cat_opt.is_none_or(|c| item.category == c) && entry_matches(item, table_kind, terms)).count()
}

fn category_slug(cat: AnalysisCategory) -> &'static str {
    match cat {
        AnalysisCategory::Transform => "transform",
        AnalysisCategory::Column => "column",
        AnalysisCategory::Grouped => "grouped",
        AnalysisCategory::XyAndRegression => "xy",
        AnalysisCategory::Survival => "survival",
        AnalysisCategory::Contingency => "contingency",
        AnalysisCategory::Multivariate => "multivariate",
    }
}

fn localized_category_label(locale: &str, cat: AnalysisCategory) -> &'static str {
    let key = match cat {
        AnalysisCategory::Transform => "analyze_picker.cat.transform",
        AnalysisCategory::Column => "analyze_picker.cat.column",
        AnalysisCategory::Grouped => "analyze_picker.cat.grouped",
        AnalysisCategory::XyAndRegression => "analyze_picker.cat.xy",
        AnalysisCategory::Survival => "analyze_picker.cat.survival",
        AnalysisCategory::Contingency => "analyze_picker.cat.contingency",
        AnalysisCategory::Multivariate => "analyze_picker.cat.multivariate",
    };
    picker_tr(locale, key, cat.label())
}

fn picker_tr<'a>(locale: &str, key: &str, fallback: &'a str) -> &'a str {
    let canonical = crate::i18n::normalize_locale(locale);
    crate::i18n::try_tr(canonical, key).unwrap_or(fallback)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn scaled_u32(base: f64, scale: f64, min: u32, max: u32) -> u32 {
    ((base * scale).round().max(0.0) as u32).clamp(min, max)
}
