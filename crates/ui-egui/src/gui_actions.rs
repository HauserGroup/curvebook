//! Headless & desktop interactive GUI action handlers (`R8.7`, `docs/gui-workflows.md` §2).
//!
//! Implements the 8 structured GUI actions exported by [`crate::control_io::dispatch_gui_action`]:
//! `ui.inspect`, `ui.screenshot`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`,
//! `ui.invoke_menu`, `ui.switch_tab`, and `ui.answer_dialog`.
//!
//! All document mutations triggered through these GUI actions run exclusively as `Caller::Ui`,
//! allowing automation workflows under `--gui-only` to drive menus, spreadsheets, canvases,
//! wizards, and modal dialogs identically to an interactive desktop user.

use lumencraft_doc::{
    AnalysisKind, Cell, CellValue, ContingencyMethod, ErrorBarMetric, ErrorCode, OneWayMethod, SheetId, SummaryMeasure, TableArchetype, XColumnMode,
};
use lumencraft_engine::{EngineError, cmd};
use lumencraft_render::RenderTargetSpec;
use lumencraft_tools::GridCellCoord;
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::compositor::{ScreenshotRegionTarget, render_workspace_region_png};
use crate::control_io::{
    DEFAULT_SCREENSHOT_DPI, DEFAULT_SCREENSHOT_HEIGHT_PX, DEFAULT_SCREENSHOT_WIDTH_PX, MAX_SCREENSHOT_DIM_PX, MIN_DPI, MIN_SCREENSHOT_DIM_PX,
    dispatch_ui_command_as_ui, encode_base64, parse_bounded_u32, write_host_file,
};
use crate::dialogs_canvas::{ActiveDialog, AnalyzeDuplicateChoice};
use crate::inspect::inspect_ui_tree;
use crate::interaction::{KeyModifiers, PointerButton, UiInputEvent, ViewportMetrics, dispatch_ui_event};
use crate::menus::tooltips::menu_bar_tooltips;
use crate::menus::ui_commands::UI_COMMANDS;
use crate::modal_predicates::{ControlEnablement, evaluate_dialog_button, evaluate_edit_params_tab, r1_paired_or_matched_allowed};
use crate::power_wizard::PowerWizardStage;

/// Returns `true` when `method` is one of the 8 interactive GUI action entry points allowed
/// under strict `--gui-only`.
#[must_use]
pub(crate) fn is_gui_action_method(method: &str) -> bool {
    matches!(
        method,
        "ui.inspect"
            | "ui.screenshot"
            | "ui.pointer_gesture"
            | "ui.type_text"
            | "ui.press_key"
            | "ui.invoke_menu"
            | "ui.switch_tab"
            | "ui.answer_dialog"
    )
}

/// Dispatch one of the 8 interactive GUI actions (`Caller::Ui`) against `app`.
pub(crate) fn dispatch(app: &mut UiApp, method: &str, params: Value) -> Result<Value, EngineError> {
    let res = match method {
        "ui.inspect" => handle_ui_inspect(app, &params),
        "ui.screenshot" => handle_ui_screenshot(app, &params),
        "ui.pointer_gesture" => handle_pointer_gesture(app, &params),
        "ui.type_text" => handle_type_text(app, &params),
        "ui.press_key" => handle_press_key(app, &params),
        "ui.invoke_menu" => handle_invoke_menu(app, &params),
        "ui.switch_tab" => handle_switch_tab(app, &params),
        "ui.answer_dialog" => handle_answer_dialog(app, &params),
        other => Err(EngineError::with_code(ErrorCode::NotFound, format!("unknown GUI action `{other}`"))),
    }?;
    app.sync_views();
    Ok(res)
}

fn bad_input(msg: impl Into<String>) -> EngineError {
    EngineError::with_code(ErrorCode::BadInput, msg)
}

fn disabled_err(reason: impl Into<String>) -> EngineError {
    let msg = reason.into();
    EngineError::with_code(ErrorCode::Conflict, msg.clone())
        .with_remediation("Satisfy the control precondition or select a compatible mode/sheet first.")
        .with_details(json!({ "disabled": true, "disabled_reason": msg }))
}

fn handle_ui_inspect(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    app.sync_views();
    let scope = params.get("scope").and_then(Value::as_str);
    let include_bounds = params.get("include_bounds").and_then(Value::as_bool).unwrap_or(true);
    let snap = inspect_ui_tree(app, scope, include_bounds);
    serde_json::to_value(snap).map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed to serialize ui tree: {err}")))
}

fn handle_ui_screenshot(app: &UiApp, params: &Value) -> Result<Value, EngineError> {
    let target_raw = params.get("target").and_then(Value::as_str).unwrap_or("full_window").trim().to_ascii_lowercase();
    let region = match target_raw.as_str() {
        "" | "full_window" | "full" | "window" | "workspace" | "viewport" => ScreenshotRegionTarget::FullWindow,
        "canvas" | "main" => ScreenshotRegionTarget::Canvas,
        "dialog" | "modal" => ScreenshotRegionTarget::Dialog,
        other => {
            return Err(bad_input(format!("unsupported screenshot target `{other}`; expected full_window, workspace, viewport, canvas, or dialog")));
        }
    };

    let width_px = parse_bounded_u32(params.get("width_px"), DEFAULT_SCREENSHOT_WIDTH_PX, MIN_SCREENSHOT_DIM_PX, MAX_SCREENSHOT_DIM_PX, "width_px")
        .map_err(bad_input)?;
    let height_px =
        parse_bounded_u32(params.get("height_px"), DEFAULT_SCREENSHOT_HEIGHT_PX, MIN_SCREENSHOT_DIM_PX, MAX_SCREENSHOT_DIM_PX, "height_px")
            .map_err(bad_input)?;
    let dpi_u32 = parse_bounded_u32(params.get("dpi"), u32::from(DEFAULT_SCREENSHOT_DPI), u32::from(MIN_DPI), 600, "dpi").map_err(bad_input)?;
    let dpi = u16::try_from(dpi_u32).map_err(|_| bad_input("dpi out of range"))?;

    let opt_path = params.get("path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    if opt_path.is_some() && app.is_gui_only() {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            "direct filesystem writes via `ui.screenshot` `path` are blocked under --gui-only",
        )
        .with_remediation("Omit `path` and inspect the returned in-memory `data_base64` PNG instead."));
    }

    let (png_bytes, out_w, out_h) = render_workspace_region_png(app, RenderTargetSpec { width_px, height_px, dpi }, region)
        .map_err(|err| EngineError::with_code(ErrorCode::Conflict, format!("screenshot capture failed: {err}")))?;

    let saved_path = match opt_path {
        Some(path) => {
            write_host_file(path, &png_bytes).map_err(|err| EngineError::with_code(ErrorCode::IoError, err))?;
            Some(path.to_owned())
        }
        None => None,
    };

    let target_label = match region {
        ScreenshotRegionTarget::FullWindow => "full_window",
        ScreenshotRegionTarget::Canvas => "canvas",
        ScreenshotRegionTarget::Dialog => "dialog",
    };

    Ok(json!({
        "format": "png",
        "target": target_label,
        "width_px": out_w,
        "height_px": out_h,
        "dpi": dpi,
        "bytes_len": png_bytes.len(),
        "data_base64": encode_base64(&png_bytes),
        "path": saved_path,
    }))
}

fn parse_modifiers(val: Option<&Value>) -> KeyModifiers {
    let mut mods = KeyModifiers::default();
    let Some(v) = val else {
        return mods;
    };
    if let Some(obj) = v.as_object() {
        mods.shift = obj.get("shift").and_then(Value::as_bool).unwrap_or(false);
        mods.ctrl = obj.get("ctrl").or_else(|| obj.get("control")).and_then(Value::as_bool).unwrap_or(false);
        mods.alt = obj.get("alt").or_else(|| obj.get("option")).and_then(Value::as_bool).unwrap_or(false);
        mods.meta = obj.get("meta").or_else(|| obj.get("command")).or_else(|| obj.get("cmd")).and_then(Value::as_bool).unwrap_or(false);
    } else if let Some(arr) = v.as_array() {
        for item in arr.iter().filter_map(Value::as_str) {
            match item.trim().to_ascii_lowercase().as_str() {
                "shift" => mods.shift = true,
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "option" => mods.alt = true,
                "cmd" | "command" | "meta" | "super" => mods.meta = true,
                _ => {}
            }
        }
    }
    mods
}

fn parse_metrics(params: &Value) -> Result<ViewportMetrics, EngineError> {
    match params.get("metrics") {
        None | Some(Value::Null) => Ok(ViewportMetrics::default()),
        Some(m) => serde_json::from_value(m.clone()).map_err(|err| bad_input(format!("invalid viewport metrics: {err}"))),
    }
}

fn handle_pointer_gesture(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let action = params
        .get("action")
        .or_else(|| params.get("kind"))
        .or_else(|| params.get("gesture"))
        .and_then(Value::as_str)
        .unwrap_or("click")
        .trim()
        .to_ascii_lowercase();
    if !matches!(action.as_str(), "click" | "double_click" | "right_click" | "drag" | "hover" | "scroll" | "wheel") {
        return Err(bad_input(format!("unsupported pointer gesture action `{action}`")));
    }

    let metrics = parse_metrics(params)?;
    let modifiers = parse_modifiers(params.get("modifiers"));
    let widget_id = params.get("widget_id").or_else(|| params.get("target_id")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());

    let tree = inspect_ui_tree(app, None, true);
    let target_node = match widget_id {
        Some(wid) => Some(tree.find_widget(wid).ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("widget `{wid}` not found")))?),
        None => None,
    };

    if let Some(ref node) = target_node
        && !node.enabled
        && matches!(action.as_str(), "click" | "double_click")
    {
        return Err(disabled_err(node.disabled_reason.clone().unwrap_or_else(|| format!("widget `{}` is disabled", node.widget_id))));
    }

    let (x, y) = resolve_pointer_xy(params, target_node.as_ref().and_then(|n| n.bounds))?;

    if app.canvas_dialogs.active_dialog == ActiveDialog::PowerAndSampleSize
        && let Some(curve_node) = tree.find_widget("power.curve_hover")
        && let Some(cb) = curve_node.bounds
    {
        let on_curve = widget_id == Some("power.curve_hover") || point_in_rect(x, y, cb);
        if on_curve && matches!(action.as_str(), "hover" | "click") {
            return handle_power_curve_hover(app, params, x, y, cb);
        }
    }

    if let Some(ref node) = target_node
        && matches!(action.as_str(), "click" | "double_click")
    {
        app.focus_widget_id = Some(node.widget_id.clone());
        if let Some(res) = try_activate_semantic_widget(app, &node.widget_id)? {
            return Ok(res);
        }
    }

    let ptr = PointerDispatchInput { action: &action, x, y, params, modifiers, metrics, widget_id, tree: &tree };
    dispatch_physical_pointer(app, &ptr)
}

struct PointerDispatchInput<'a> {
    action: &'a str,
    x: f64,
    y: f64,
    params: &'a Value,
    modifiers: KeyModifiers,
    metrics: ViewportMetrics,
    widget_id: Option<&'a str>,
    tree: &'a crate::inspect::UiTreeSnapshot,
}

fn handle_power_curve_hover(app: &mut UiApp, params: &Value, x: f64, y: f64, bounds: [f64; 4]) -> Result<Value, EngineError> {
    if app.canvas_dialogs.power_wizard.result.is_none() {
        let _ = app.canvas_dialogs.power_wizard.recalculate();
    }
    let hovered = if let Some(hn) = params.get("hover_n").and_then(Value::as_f64) {
        app.canvas_dialogs.power_wizard.hover_at_n(hn)
    } else {
        let frac = params.get("hover_fraction_x").and_then(Value::as_f64).unwrap_or_else(|| ((x - bounds[0]) / bounds[2].max(1.0)).clamp(0.0, 1.0));
        app.canvas_dialogs.power_wizard.hover_at_fraction(frac)
    };
    app.interaction.last_logical_pos = Some([x, y]);
    Ok(json!({
        "action": "hover",
        "widget_id": "power.curve_hover",
        "x": x,
        "y": y,
        "hover_n": hovered.map(|(n, _)| n),
        "hover_power": hovered.map(|(_, p)| p),
    }))
}

fn try_activate_semantic_widget(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    if let Some(btn) = wid.strip_prefix("dialog.button.") {
        return handle_answer_dialog(app, &json!({ "action": btn })).map(Some);
    }
    if let Some(cmd_id) = wid.strip_prefix("menu.") {
        return handle_invoke_menu(app, &json!({ "command_id": cmd_id })).map(Some);
    }
    if wid.starts_with("result.tab.") || wid.starts_with("edit.tab.") || wid.starts_with("power.stage.") {
        return handle_switch_tab(app, &json!({ "widget_id": wid })).map(Some);
    }
    if let Some(sheet_id) = wid.strip_prefix("nav.sheet.") {
        return handle_switch_tab(app, &json!({ "sheet_id": sheet_id })).map(Some);
    }
    if wid.starts_with("tb.")
        && let Some(btn) = inspect_ui_tree(app, Some("toolbar"), false).find_widget(wid)
        && let Some(cmd_id) = btn.command_id
    {
        return handle_invoke_menu(app, &json!({ "command_id": cmd_id })).map(Some);
    }
    if let Some(res) = try_toggle_dialog_widget_click(app, wid)? {
        return Ok(Some(res));
    }
    Ok(None)
}

fn try_toggle_dialog_widget_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::EditAnalysisParameters { .. } => {
            let mut patch = Map::new();
            let ep = &app.canvas_dialogs.edit_params;
            let cont_spec = match ep.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Contingency(c)) => *c,
                _ => lumencraft_doc::ContingencySpec::default(),
            };
            let gg_checked = ep.draft.as_ref().is_none_or(|d| {
                !matches!(
                    d.kind,
                    AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })
                )
            });
            match wid {
                "edit.assume_gaussian" => {
                    patch.insert(String::from("assume_gaussian"), json!(!ep.assume_gaussian));
                }
                "edit.paired_or_matched" => {
                    if !ep.paired_or_matched {
                        let gate = r1_paired_or_matched_allowed(app.document(), ep.draft.as_ref().map(|d| &d.source_sheet_id));
                        if !gate.enabled {
                            return Err(disabled_err(
                                gate.disabled_reason.unwrap_or_else(|| String::from("Matched / Repeated Measures requires equal N >= 2")),
                            ));
                        }
                    }
                    patch.insert(String::from("paired_or_matched"), json!(!ep.paired_or_matched));
                }
                "edit.sphericity.geisser_greenhouse" => {
                    if !ep.paired_or_matched {
                        return Err(disabled_err("Geisser-Greenhouse / REML requires Matched or Repeated Measures mode."));
                    }
                    patch.insert(String::from("edit.sphericity.geisser_greenhouse"), json!(!gg_checked));
                }
                "edit.post_hoc.tukey" => {
                    patch.insert(String::from("multiplicity_method"), json!("tukey"));
                }
                "edit.post_hoc.dunn" => {
                    patch.insert(String::from("multiplicity_method"), json!("dunn"));
                }
                "edit.two_group.welch" => {
                    patch.insert(String::from("welch_correction"), json!(!ep.welch_correction));
                }
                "edit.contingency.odds_ratio" => {
                    patch.insert(String::from("compute_odds_and_risk_ratios"), json!(!ep.contingency_odds_ratio_checked));
                }
                "edit.contingency.attributable_risk" => {
                    patch.insert(String::from("attributable_risk"), json!(!ep.contingency_attributable_risk_checked));
                }
                "edit.contingency.sensitivity_specificity" => {
                    patch.insert(String::from("sensitivity_specificity"), json!(!cont_spec.compute_sensitivity_specificity));
                }
                "edit.contingency.fisher" => {
                    patch.insert(String::from("fisher"), json!(cont_spec.method != ContingencyMethod::FisherExact));
                }
                "edit.contingency.mcnemar" => {
                    patch.insert(String::from("mcnemar"), json!(cont_spec.method != ContingencyMethod::McNemarPaired));
                }
                "edit.contingency.cochran_armitage" => {
                    patch.insert(String::from("cochran_armitage"), json!(true));
                }
                "edit.correlation.heatmap" => {
                    patch.insert(String::from("correlation_heatmap"), json!(!ep.correlation_heatmap));
                }
                _ => return Ok(None),
            }
            app.canvas_dialogs.edit_params.apply_field_patch(&patch).map_err(disabled_err)?;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::ImportReshape if wid == "import.subtract_blank_mean" => {
            app.grid.import_reshape.subtract_blank_mean = !app.grid.import_reshape.subtract_blank_mean;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::ConvertArchetype { .. } if wid == "convert.allow_lossy" || wid == "convert.preview_only" => {
            if wid == "convert.allow_lossy" {
                app.canvas_dialogs.convert_archetype.allow_lossy = !app.canvas_dialogs.convert_archetype.allow_lossy;
            } else {
                app.canvas_dialogs.convert_archetype.preview_only = !app.canvas_dialogs.convert_archetype.preview_only;
            }
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::FormatGraph { .. }
            if matches!(wid, "format_graph.show_legend" | "format_graph.legend_bordered" | "format_graph.error_bar_enabled") =>
        {
            match wid {
                "format_graph.show_legend" => {
                    app.canvas_dialogs.format_graph.show_legend = !app.canvas_dialogs.format_graph.show_legend;
                }
                "format_graph.legend_bordered" => {
                    app.canvas_dialogs.format_graph.legend_bordered = !app.canvas_dialogs.format_graph.legend_bordered;
                }
                _ => {
                    app.canvas_dialogs.format_graph.error_bar_enabled = !app.canvas_dialogs.format_graph.error_bar_enabled;
                }
            }
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::Print { .. } if wid == "print.crop_marks" => {
            app.canvas_dialogs.print.job.crop_marks = !app.canvas_dialogs.print.job.crop_marks;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::AnalyzeSheet { .. } if wid == "analyze.correlation_all_pairs" || wid == "analyze.correlation.heatmap" => {
            let mut patch = Map::new();
            if wid == "analyze.correlation_all_pairs" {
                patch.insert(String::from("correlation_all_pairs"), json!(!app.canvas_dialogs.analyze.correlation_all_pairs));
            } else {
                patch.insert(String::from("correlation_heatmap"), json!(!app.canvas_dialogs.analyze.correlation_heatmap));
            }
            app.canvas_dialogs.analyze.apply_field_patch(&patch).map_err(disabled_err)?;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        _ => Ok(None),
    }
}

fn dispatch_physical_pointer(app: &mut UiApp, input: &PointerDispatchInput<'_>) -> Result<Value, EngineError> {
    let PointerDispatchInput { action, x, y, params, modifiers, metrics, widget_id, tree } = *input;
    let map_err = |e: String| EngineError::with_code(ErrorCode::Conflict, e);
    let last_outcome = match action {
        "hover" => {
            let out = dispatch_ui_event(app, UiInputEvent::PointerMove { x, y, modifiers }, metrics).map_err(map_err)?;
            serde_json::to_value(out).unwrap_or(Value::Null)
        }
        "scroll" | "wheel" => {
            let dy = params.get("delta_y").and_then(Value::as_f64).unwrap_or(-120.0);
            let out = dispatch_ui_event(app, UiInputEvent::WheelZoom { x, y, delta_y: dy, modifiers }, metrics).map_err(map_err)?;
            serde_json::to_value(out).unwrap_or(Value::Null)
        }
        "double_click" => {
            let out =
                dispatch_ui_event(app, UiInputEvent::DoubleClick { x, y, button: PointerButton::Primary, modifiers }, metrics).map_err(map_err)?;
            serde_json::to_value(out).unwrap_or(Value::Null)
        }
        "right_click" => {
            dispatch_ui_event(app, UiInputEvent::PointerDown { x, y, button: PointerButton::Secondary, modifiers }, metrics).map_err(map_err)?;
            let out =
                dispatch_ui_event(app, UiInputEvent::PointerUp { x, y, button: PointerButton::Secondary, modifiers }, metrics).map_err(map_err)?;
            if app.interaction.context_menu_target.is_none() && (!app.brushed_points.is_empty() || !app.grid.selected_rows.is_empty()) {
                app.interaction.context_menu_target = Some(String::from("Format Selected Points..."));
            }
            serde_json::to_value(out).unwrap_or(Value::Null)
        }
        "drag" => {
            let to_node_bounds = params.get("to_widget_id").and_then(Value::as_str).and_then(|wid| tree.find_widget(wid).and_then(|n| n.bounds));
            let (tx, ty) = resolve_end_xy(params, to_node_bounds, x, y)?;
            dispatch_ui_event(app, UiInputEvent::PointerDown { x, y, button: PointerButton::Primary, modifiers }, metrics).map_err(map_err)?;
            dispatch_ui_event(app, UiInputEvent::PointerMove { x: tx, y: ty, modifiers }, metrics).map_err(map_err)?;
            let out = dispatch_ui_event(app, UiInputEvent::PointerUp { x: tx, y: ty, button: PointerButton::Primary, modifiers }, metrics)
                .map_err(map_err)?;
            serde_json::to_value(out).unwrap_or(Value::Null)
        }
        _ => {
            dispatch_ui_event(app, UiInputEvent::PointerDown { x, y, button: PointerButton::Primary, modifiers }, metrics).map_err(map_err)?;
            let out =
                dispatch_ui_event(app, UiInputEvent::PointerUp { x, y, button: PointerButton::Primary, modifiers }, metrics).map_err(map_err)?;
            serde_json::to_value(out).unwrap_or(Value::Null)
        }
    };

    Ok(json!({
        "action": action,
        "widget_id": widget_id,
        "x": x,
        "y": y,
        "selected_rows": app.grid.selected_rows,
        "brushed_count": app.brushed_points.len(),
        "context_menu_target": app.interaction.context_menu_target,
        "outcome": last_outcome,
    }))
}

fn resolve_pointer_xy(params: &Value, bounds: Option<[f64; 4]>) -> Result<(f64, f64), EngineError> {
    if let (Some(x), Some(y)) = (params.get("x").and_then(Value::as_f64), params.get("y").and_then(Value::as_f64)) {
        return Ok((x, y));
    }
    if let Some(b) = bounds {
        return Ok((b[0] + b[2] * 0.5, b[1] + b[3] * 0.5));
    }
    Err(bad_input("pointer gesture requires either valid `widget_id` with bounds or numeric `x` and `y` coordinates"))
}

fn resolve_end_xy(params: &Value, to_bounds: Option<[f64; 4]>, start_x: f64, start_y: f64) -> Result<(f64, f64), EngineError> {
    if let (Some(x), Some(y)) = (
        params.get("to_x").or_else(|| params.get("end_x")).and_then(Value::as_f64),
        params.get("to_y").or_else(|| params.get("end_y")).and_then(Value::as_f64),
    ) {
        return Ok((x, y));
    }
    if let Some(b) = to_bounds {
        return Ok((b[0] + b[2] * 0.5, b[1] + b[3] * 0.5));
    }
    if params.get("to_widget_id").is_some() {
        return Err(EngineError::with_code(ErrorCode::NotFound, "drag `to_widget_id` not found"));
    }
    Ok((start_x, start_y))
}

fn point_in_rect(x: f64, y: f64, r: [f64; 4]) -> bool {
    x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]
}

fn handle_type_text(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let text = params.get("text").and_then(Value::as_str).ok_or_else(|| bad_input("ui.type_text requires `text` string"))?;
    let clear_existing = params.get("clear_existing").and_then(Value::as_bool).unwrap_or(true);
    let commit_enter = params.get("commit_enter").and_then(Value::as_bool).unwrap_or(true);
    if let Some(wid) = params.get("widget_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        app.focus_widget_id = Some(wid.to_owned());
    }
    let wid = app.focus_widget_id.clone();

    if app.canvas_dialogs.active_dialog != ActiveDialog::None
        && let Some(ref field_id) = wid
        && !field_id.starts_with("grid.cell.")
        && parse_x_cell_widget_id(field_id).is_none()
    {
        return apply_type_text_to_dialog(app, field_id, text, commit_enter);
    }

    let doc = app.document();
    let Some(active_id) = doc.active_sheet_id.clone() else {
        return Err(EngineError::with_code(ErrorCode::Conflict, "no active sheet to receive typed text"));
    };
    if doc.data_sheet(&active_id).is_some() {
        return apply_type_text_to_grid(app, &active_id, wid.as_deref(), text, clear_existing, commit_enter);
    }
    if doc.info_sheet(&active_id).is_some() {
        app.execute_command("info.setNote", json!({ "sheet_id": active_id.as_str(), "text": text }))?;
        return Ok(json!({ "typed": text, "target": "info.note", "sheet_id": active_id.as_str() }));
    }
    Err(EngineError::with_code(ErrorCode::Conflict, "active sheet does not accept text input"))
}

fn apply_type_text_to_dialog(app: &mut UiApp, field_id: &str, text: &str, commit_enter: bool) -> Result<Value, EngineError> {
    let tree = inspect_ui_tree(app, Some("dialog"), false);
    if let Some(node) = tree.find_widget(field_id)
        && !node.enabled
    {
        return Err(disabled_err(node.disabled_reason.unwrap_or_else(|| format!("control `{field_id}` is disabled"))));
    }
    if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
        app.canvas_dialogs.command_palette.query = text.to_owned();
        if commit_enter && let Some(hit) = app.canvas_dialogs.command_palette.matches_for_session(app.session()).first().cloned() {
            if let Some(res) = dispatch_ui_command_as_ui(app, hit.id, &Value::Null)? {
                return Ok(json!({ "typed": text, "executed": hit.id, "result": res }));
            }
            let out = app.execute_command(hit.id, Value::Null)?;
            app.canvas_dialogs.close_dialog();
            return Ok(json!({ "typed": text, "executed": hit.id, "mutated": out.mutated }));
        }
        return Ok(json!({ "typed": text, "widget_id": field_id, "dialog": "command_palette" }));
    }

    let val = parse_typed_json_scalar(text);
    let mut patch = Map::new();
    patch.insert(field_id.to_owned(), val);
    patch_active_dialog(app, &patch)?;
    Ok(json!({ "typed": text, "widget_id": field_id, "committed": true }))
}

fn apply_type_text_to_grid(
    app: &mut UiApp,
    sheet_id: &SheetId,
    wid: Option<&str>,
    text: &str,
    clear_existing: bool,
    commit_enter: bool,
) -> Result<Value, EngineError> {
    if let Some((x_row, x_subcol)) =
        wid.and_then(parse_x_cell_widget_id).or_else(|| if wid.and_then(parse_grid_cell_widget_id).is_some() { None } else { app.grid.active_x_cell })
    {
        return apply_type_text_to_x_cell(app, sheet_id, wid, x_row, x_subcol, text, clear_existing, commit_enter);
    }

    let coord = wid.and_then(parse_grid_cell_widget_id).or(app.grid.tool.active).unwrap_or_else(|| GridCellCoord::new(0, 0, 0));

    if text.contains('\t') || text.contains('\n') {
        let out = app.execute_command(
            "table.paste",
            json!({
                "sheet_id": sheet_id.as_str(),
                "text": text,
                "start_row": coord.row,
                "start_col": coord.col,
                "start_subcol": coord.subcol,
                "mode": "overwrite",
            }),
        )?;
        app.grid.select_single_sync(coord);
        app.sync_brush_from_grid_selection(sheet_id);
        return Ok(json!({ "pasted": true, "sheet_id": sheet_id.as_str(), "row": coord.row, "col": coord.col, "payload": out.payload }));
    }

    let final_text = if clear_existing {
        text.to_owned()
    } else {
        let existing =
            cell_existing_text(app.document().data_sheet(sheet_id).and_then(|ds| ds.archetype.data_cell(lumencraft_doc::CellCoord::from(coord))));
        format!("{existing}{text}")
    };
    let scalar = parse_grid_scalar_value(&final_text);
    let out = app.execute_command(
        "table.setCell",
        json!({
            "sheet_id": sheet_id.as_str(),
            "row": coord.row,
            "col": coord.col,
            "subcol": coord.subcol,
            "value": scalar,
        }),
    )?;
    let next_coord = if commit_enter { GridCellCoord::new(coord.row + 1, coord.col, coord.subcol) } else { coord };
    app.grid.select_single_sync(next_coord);
    app.sync_brush_from_grid_selection(sheet_id);
    Ok(json!({
        "sheet_id": sheet_id.as_str(),
        "row": coord.row,
        "col": coord.col,
        "subcol": coord.subcol,
        "value": scalar,
        "mutated": out.mutated,
    }))
}

#[allow(clippy::too_many_arguments)]
fn apply_type_text_to_x_cell(
    app: &mut UiApp,
    sheet_id: &SheetId,
    wid: Option<&str>,
    x_row: usize,
    x_subcol: usize,
    text: &str,
    clear_existing: bool,
    commit_enter: bool,
) -> Result<Value, EngineError> {
    {
        let Some(ds) = app.document().data_sheet(sheet_id) else {
            return Err(EngineError::with_code(ErrorCode::NotFound, format!("data sheet `{}` not found", sheet_id.as_str())));
        };
        if ds.frozen {
            return Err(disabled_err("Data sheet is frozen (read-only)."));
        }
        match &ds.archetype {
            TableArchetype::Xy(xy) if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) => {
                return Err(disabled_err("X column is a generated sequence (read-only)."));
            }
            TableArchetype::Xy(_) | TableArchetype::Survival(_) => {}
            _ => return Err(bad_input("active sheet does not have an X/Time column")),
        }
    }

    if text.contains('\n') || text.contains('\t') {
        let vals: Vec<Value> = text
            .lines()
            .map(|line| {
                let first = line.split('\t').next().unwrap_or("");
                parse_grid_scalar_value(first)
            })
            .collect();
        let out = app.execute_command(
            "table.setXCells",
            json!({
                "sheet_id": sheet_id.as_str(),
                "start_row": x_row,
                "subcol": x_subcol,
                "values": vals,
            }),
        )?;
        app.grid.select_x_cell_sync(x_row, x_subcol);
        app.grid.refresh_selection_stats(app.session.document());
        return Ok(json!({
            "pasted": true,
            "x_col": true,
            "sheet_id": sheet_id.as_str(),
            "row": x_row,
            "subcol": x_subcol,
            "payload": out.payload,
        }));
    }

    let final_text = if clear_existing {
        text.to_owned()
    } else {
        let existing = cell_existing_text(app.document().data_sheet(sheet_id).and_then(|ds| ds.x_or_time_cell(x_row, x_subcol)));
        format!("{existing}{text}")
    };
    let scalar = parse_grid_scalar_value(&final_text);
    let out = app.execute_command(
        "table.setXCells",
        json!({
            "sheet_id": sheet_id.as_str(),
            "row": x_row,
            "subcol": x_subcol,
            "value": scalar,
        }),
    )?;
    let next_row = if commit_enter { x_row + 1 } else { x_row };
    app.grid.select_x_cell_sync(next_row, x_subcol);
    if wid.is_some() || app.focus_widget_id.as_deref().and_then(parse_x_cell_widget_id).is_some() {
        app.focus_widget_id = Some(format!("grid.xcell.{next_row}.{x_subcol}"));
    }
    app.grid.refresh_selection_stats(app.session.document());
    Ok(json!({
        "sheet_id": sheet_id.as_str(),
        "x_col": true,
        "row": x_row,
        "subcol": x_subcol,
        "value": scalar,
        "mutated": out.mutated,
    }))
}

fn cell_existing_text(cell: Option<&Cell>) -> String {
    cell.map(|c| match &c.value {
        CellValue::Empty => String::new(),
        CellValue::Number(v) => v.to_string(),
        CellValue::Text(s) => s.clone(),
    })
    .unwrap_or_default()
}

fn parse_grid_scalar_value(final_text: &str) -> Value {
    if final_text.starts_with('*') {
        Value::String(final_text.to_owned())
    } else if let Ok(num) = final_text.trim().parse::<f64>()
        && num.is_finite()
    {
        json!(num)
    } else {
        Value::String(final_text.to_owned())
    }
}

fn parse_grid_cell_widget_id(wid: &str) -> Option<GridCellCoord> {
    let tail = wid.strip_prefix("grid.cell.")?;
    let mut parts = tail.split('.');
    let row = parts.next()?.parse::<usize>().ok()?;
    let col = parts.next()?.parse::<usize>().ok()?;
    let subcol = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    Some(GridCellCoord::new(row, col, subcol))
}

fn parse_x_cell_widget_id(wid: &str) -> Option<(usize, usize)> {
    if let Some(tail) = wid.strip_prefix("grid.xcell.").or_else(|| wid.strip_prefix("grid.x.")) {
        let mut parts = tail.split('.');
        let row = parts.next()?.parse::<usize>().ok()?;
        let subcol = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
        return Some((row, subcol));
    }
    if let Some(tail) = wid.strip_prefix("grid.cell.") {
        let mut parts = tail.split('.');
        let row = parts.next()?.parse::<usize>().ok()?;
        let tag = parts.next()?;
        if tag.eq_ignore_ascii_case("x") {
            let subcol = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
            return Some((row, subcol));
        }
    }
    None
}

fn parse_typed_json_scalar(text: &str) -> Value {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(num) = trimmed.parse::<f64>()
        && num.is_finite()
    {
        return json!(num);
    }
    Value::String(text.to_owned())
}

fn format_shortcut_combo(key: &str, mods: KeyModifiers) -> String {
    let mut parts = Vec::new();
    if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.meta {
        parts.push("Cmd");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.shift {
        parts.push("Shift");
    }
    let key_norm = match key.trim().to_ascii_lowercase().as_str() {
        "return" | "enter" => "Enter",
        "escape" | "esc" => "Escape",
        _ => key.trim(),
    };
    if key_norm.len() == 1 {
        let up = key_norm.to_ascii_uppercase();
        if parts.is_empty() {
            return up;
        }
        return format!("{}+{}", parts.join("+"), up);
    }
    if parts.is_empty() { key_norm.to_owned() } else { format!("{}+{}", parts.join("+"), key_norm) }
}

fn handle_press_key(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let key = params.get("key").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad_input("missing `key`"))?;
    let modifiers = parse_modifiers(params.get("modifiers"));
    let combo = format_shortcut_combo(key, modifiers);

    if (modifiers.meta || modifiers.ctrl) && !modifiers.shift && !modifiers.alt && key.eq_ignore_ascii_case("t") {
        let doc = app.document().clone();
        if let Some(rs_id) = doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()) {
            app.canvas_dialogs.open_edit_parameters(&doc, rs_id);
            return Ok(json!({ "combo": combo, "dialog": "EditAnalysisParameters", "result_sheet_id": rs_id.as_str() }));
        }
    }

    if app.canvas_dialogs.active_dialog != ActiveDialog::None
        && !modifiers.meta
        && !modifiers.ctrl
        && !modifiers.alt
        && matches!(key.to_ascii_lowercase().as_str(), "enter" | "return")
    {
        let action = match app.canvas_dialogs.active_dialog {
            ActiveDialog::PowerAndSampleSize => {
                if app.canvas_dialogs.power_wizard.stage == PowerWizardStage::Results {
                    "create"
                } else {
                    "next"
                }
            }
            _ => "ok",
        };
        return handle_answer_dialog(app, &json!({ "action": action }));
    }

    if app.canvas_dialogs.active_dialog == ActiveDialog::None
        && let Some(exec) = app.handle_key_combo(&combo).map_err(|e| EngineError::with_code(ErrorCode::Conflict, e))?
    {
        return Ok(json!({ "combo": combo, "shortcut_executed": exec.command_id, "ui": exec.ui, "result": exec.result }));
    }

    let out = dispatch_ui_event(app, UiInputEvent::KeyDown { key: key.to_owned(), modifiers }, ViewportMetrics::default())
        .map_err(|e| EngineError::with_code(ErrorCode::Conflict, e))?;
    Ok(json!({ "combo": combo, "key": key, "outcome": out }))
}

fn normalize_menu_token(raw: &str) -> String {
    raw.split(['>', '/']).map(|seg| seg.trim().trim_end_matches("...").to_ascii_lowercase()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/")
}

fn extract_menu_path(val: Option<&Value>) -> Result<Option<String>, EngineError> {
    let Some(v) = val else {
        return Ok(None);
    };
    if let Some(s) = v.as_str() {
        let trimmed = s.trim();
        return Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()));
    }
    if let Some(arr) = v.as_array() {
        let mut segs = Vec::with_capacity(arr.len());
        for item in arr {
            let Some(seg) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                return Err(bad_input("ui.invoke_menu `path` array elements must be non-empty strings"));
            };
            segs.push(seg);
        }
        return Ok((!segs.is_empty()).then(|| segs.join("/")));
    }
    Err(bad_input("ui.invoke_menu `path` must be a string or array of strings"))
}

fn handle_invoke_menu(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let explicit_cmd = params.get("command_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let path = extract_menu_path(params.get("path"))?;
    if explicit_cmd.is_none() && path.is_none() {
        return Err(bad_input("ui.invoke_menu requires `command_id` or `path`"));
    }

    let sections = menu_bar_tooltips(app);
    let norm_path = path.as_deref().map(normalize_menu_token);
    let matched_tip = sections.iter().find_map(|sec| {
        sec.items.iter().find(|tip| {
            if let Some(cid) = explicit_cmd
                && tip.command_id == cid
            {
                return true;
            }
            if let Some(ref target) = norm_path {
                let by_section = normalize_menu_token(&format!("{}/{}", sec.title, tip.label));
                let by_menu_path = normalize_menu_token(tip.menu_path);
                let by_label = normalize_menu_token(tip.label);
                let by_loc = normalize_menu_token(tip.localized_label);
                return &by_section == target || &by_menu_path == target || &by_label == target || &by_loc == target;
            }
            false
        })
    });

    let cmd_id = if let Some(tip) = matched_tip {
        if !tip.enabled {
            return Err(disabled_err(format!("{} is disabled in the current workspace context.", tip.label)));
        }
        tip.command_id.to_owned()
    } else if let Some(cid) = explicit_cmd {
        if let Some(spec) = cmd::registry().get(cid) {
            if !(spec.enabled)(app.session()) {
                return Err(disabled_err(format!("menu command `{cid}` is disabled in the current workspace context.")));
            }
            cid.to_owned()
        } else if UI_COMMANDS.iter().any(|u| u.id == cid) {
            cid.to_owned()
        } else {
            return Err(EngineError::with_code(ErrorCode::NotFound, format!("menu command `{cid}` not found")));
        }
    } else {
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("menu item `{}` not found", path.as_deref().unwrap_or_default())));
    };

    let extra_params = params.get("params").cloned().unwrap_or(Value::Null);
    if let Some(ui_val) = dispatch_ui_command_as_ui(app, &cmd_id, &extra_params)? {
        return Ok(json!({
            "invoked": true,
            "command_id": cmd_id,
            "ui": true,
            "active_dialog": format!("{:?}", app.canvas_dialogs.active_dialog),
            "result": ui_val,
        }));
    }

    let outcome = app.execute_command(&cmd_id, extra_params)?;
    Ok(json!({
        "invoked": true,
        "command_id": cmd_id,
        "ui": false,
        "mutated": outcome.mutated,
        "payload": outcome.payload,
    }))
}

fn handle_switch_tab(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    if let Some(sheet_raw) = params
        .get("sheet_id")
        .and_then(Value::as_str)
        .or_else(|| params.get("widget_id").and_then(Value::as_str).and_then(|w| w.strip_prefix("nav.sheet.")))
    {
        let out = app.execute_command("sheet.select", json!({ "sheet_id": sheet_raw }))?;
        if let Ok(sid) = SheetId::try_new(sheet_raw) {
            let doc = app.document().clone();
            app.canvas_dialogs.on_sheet_selected(&doc, &sid);
        }
        if params.get("tab").is_none() && params.get("tab_id").is_none() {
            return Ok(json!({ "sheet_id": sheet_raw, "switched": true, "payload": out.payload }));
        }
    }

    let tab_val = params
        .get("tab")
        .or_else(|| params.get("tab_id"))
        .or_else(|| params.get("widget_id"))
        .ok_or_else(|| bad_input("ui.switch_tab requires `tab`, `tab_id`, `widget_id`, or `sheet_id`"))?;

    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::EditAnalysisParameters { .. } => {
            let raw = tab_val.as_str().ok_or_else(|| bad_input("EditAnalysisParameters `tab` must be a string"))?;
            let tab_id = raw.strip_prefix("edit.tab.").unwrap_or(raw).trim().to_ascii_lowercase();
            let check = evaluate_edit_params_tab(&app.canvas_dialogs.edit_params, &tab_id);
            if !check.enabled {
                return Err(disabled_err(check.disabled_reason.unwrap_or_else(|| format!("tab `{tab_id}` is disabled"))));
            }
            app.canvas_dialogs.edit_params.active_tab = tab_id.clone();
            Ok(json!({ "dialog": "edit_analysis_parameters", "active_tab": tab_id }))
        }
        ActiveDialog::PowerAndSampleSize => {
            let stage = parse_power_stage_value(tab_val)?;
            let current = app.canvas_dialogs.power_wizard.stage.index();
            if stage.index() > current {
                app.canvas_dialogs.power_wizard.can_advance().map_err(disabled_err)?;
                let _ = app.canvas_dialogs.power_wizard.recalculate();
            }
            app.canvas_dialogs.power_wizard.stage = stage;
            Ok(json!({ "dialog": "power_and_sample_size", "stage": stage.label(), "stage_index": stage.index() }))
        }
        _ => switch_result_sheet_tab(app, tab_val),
    }
}

fn parse_power_stage_value(val: &Value) -> Result<PowerWizardStage, EngineError> {
    if let Some(idx) = val.as_u64() {
        return PowerWizardStage::from_index(idx as usize).ok_or_else(|| bad_input(format!("invalid power wizard stage `{idx}`")));
    }
    let raw = val.as_str().ok_or_else(|| bad_input("power wizard stage must be string or index"))?;
    let trimmed = raw.strip_prefix("power.stage.").unwrap_or(raw).trim();
    if let Ok(idx) = trimmed.parse::<usize>() {
        return PowerWizardStage::from_index(idx).ok_or_else(|| bad_input(format!("invalid power wizard stage `{idx}`")));
    }
    let norm: String = trimmed.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "experimentdesign" | "design" => Ok(PowerWizardStage::ExperimentDesign),
        "effectsize" | "effect" => Ok(PowerWizardStage::EffectSize),
        "significanceandsolve" | "significance" => Ok(PowerWizardStage::SignificanceAndSolve),
        "results" | "resultssensitivity" => Ok(PowerWizardStage::Results),
        _ => Err(bad_input(format!("unknown power wizard stage `{raw}`"))),
    }
}

fn switch_result_sheet_tab(app: &mut UiApp, tab_val: &Value) -> Result<Value, EngineError> {
    let doc = app.document().clone();
    let Some(rs) = doc.active_sheet_id.as_ref().and_then(|id| doc.result_sheet(id)) else {
        return Err(EngineError::with_code(ErrorCode::Conflict, "no active ResultSheet or multi-tab dialog"));
    };
    let idx = if let Some(n) = tab_val.as_u64() {
        let i = n as usize;
        if i >= rs.tabs.len() {
            return Err(EngineError::with_code(ErrorCode::NotFound, format!("tab index `{i}` out of range")));
        }
        i
    } else if let Some(raw) = tab_val.as_str() {
        let key = raw.strip_prefix("result.tab.").unwrap_or(raw).trim();
        rs.tabs
            .iter()
            .position(|t| t.id == key || t.title.eq_ignore_ascii_case(key))
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result tab `{key}` not found")))?
    } else {
        return Err(bad_input("`tab` must be a tab id/title string or zero-based index"));
    };
    app.canvas_dialogs.result_view.select_tab(idx, rs.tabs.len());
    let chosen = &rs.tabs[idx];
    Ok(json!({ "sheet_id": rs.id.as_str(), "tab_index": idx, "tab_id": chosen.id, "tab_title": chosen.title }))
}

fn handle_answer_dialog(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    if app.canvas_dialogs.active_dialog == ActiveDialog::None {
        return Err(EngineError::with_code(ErrorCode::Conflict, "no modal dialog is currently open"));
    }
    let explicit_action = params.get("action").and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase());
    let fields = params.get("fields").or_else(|| params.get("values")).or_else(|| params.get("patches")).and_then(Value::as_object);
    if let Some(f) = fields {
        patch_active_dialog(app, f)?;
        if explicit_action.is_none() {
            return Ok(json!({ "updated": true, "dialog": format!("{:?}", app.canvas_dialogs.active_dialog) }));
        }
    }

    let action = explicit_action.unwrap_or_else(|| String::from("ok"));
    if !matches!(action.as_str(), "ok" | "cancel" | "apply" | "back" | "next" | "create" | "preview") {
        return Err(bad_input(format!("unsupported dialog action `{action}`; expected ok, cancel, apply, back, next, create, or preview")));
    }

    if action == "cancel" {
        let prev = format!("{:?}", app.canvas_dialogs.active_dialog);
        app.canvas_dialogs.close_dialog();
        return Ok(json!({ "action": "cancel", "closed": true, "previous_dialog": prev }));
    }

    let check = evaluate_dialog_button(&app.canvas_dialogs, app.document(), &action).unwrap_or_else(ControlEnablement::enabled);
    if !check.enabled {
        return Err(disabled_err(check.disabled_reason.unwrap_or_else(|| format!("dialog button `{action}` is currently disabled"))));
    }

    commit_active_dialog_action(app, &action, params)
}

fn patch_active_dialog(app: &mut UiApp, fields: &Map<String, Value>) -> Result<(), EngineError> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => app.canvas_dialogs.power_wizard.apply_field_patch(fields).map_err(disabled_err)?,
        ActiveDialog::FormatSelectedPoints { .. } => app.canvas_dialogs.format_points.apply_field_patch(fields).map_err(disabled_err)?,
        ActiveDialog::AnalyzeSheet { .. } => app.canvas_dialogs.analyze.apply_field_patch(fields).map_err(disabled_err)?,
        ActiveDialog::EditAnalysisParameters { .. } => {
            let wants_paired =
                fields.get("paired_or_matched").or_else(|| fields.get("edit.paired_or_matched")).and_then(Value::as_bool) == Some(true);
            if wants_paired {
                let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
                let gate = r1_paired_or_matched_allowed(app.document(), src);
                if !gate.enabled {
                    return Err(disabled_err(
                        gate.disabled_reason.unwrap_or_else(|| String::from("Matched / Repeated Measures requires equal N >= 2")),
                    ));
                }
            }
            app.canvas_dialogs.edit_params.apply_field_patch(fields).map_err(disabled_err)?;
        }
        ActiveDialog::ChangeDataAnalyzed { .. } => {
            if let Some(sid) = fields.get("target_sheet_id").or_else(|| fields.get("source_sheet_id")).and_then(Value::as_str)
                && let Ok(parsed) = SheetId::try_new(sid)
            {
                app.canvas_dialogs.change_data.select_target(&parsed).map_err(disabled_err)?;
            }
        }
        ActiveDialog::ChangeGraphType { .. } => {
            if let Some(opt) = fields
                .get("option_id")
                .or_else(|| fields.get("selected_option_id"))
                .or_else(|| fields.get("graph_type.selected"))
                .and_then(Value::as_str)
            {
                app.canvas_dialogs.change_graph_type.selected_option_id = opt.to_owned();
            }
            if let Some(raw) = fields.get("summary_measure").or_else(|| fields.get("graph_type.summary_measure")).and_then(Value::as_str) {
                app.canvas_dialogs.change_graph_type.summary_measure = parse_summary_measure_str(raw).map_err(bad_input)?;
            }
            if let Some(raw) = fields.get("error_bar_metric").or_else(|| fields.get("graph_type.error_bar_metric")).and_then(Value::as_str) {
                app.canvas_dialogs.change_graph_type.error_bar_metric = parse_error_bar_metric_str(raw).map_err(bad_input)?;
            }
        }
        ActiveDialog::ExcludeWithReason { .. } => {
            if let Some(tag) = fields.get("reason_tag").and_then(Value::as_str) {
                app.canvas_dialogs.exclude_with_reason.reason_tag = tag.to_owned();
            }
            if let Some(note) = fields.get("note").and_then(Value::as_str) {
                app.canvas_dialogs.exclude_with_reason.note = note.to_owned();
            }
        }
        ActiveDialog::ImportReshape => {
            app.grid.import_reshape.apply_field_patch(fields).map_err(bad_input)?;
        }
        ActiveDialog::ConvertArchetype { .. } => {
            let prev_target = app.canvas_dialogs.convert_archetype.target_kind;
            app.canvas_dialogs.convert_archetype.apply_field_patch(fields).map_err(bad_input)?;
            let target_changed = app.canvas_dialogs.convert_archetype.target_kind != prev_target;
            if (target_changed || !app.canvas_dialogs.convert_archetype.preview_ready)
                && let Some(intent) = app.canvas_dialogs.convert_archetype.preview_intent()
            {
                let out = app.session.execute_as_ui(&intent.command_id, intent.params)?;
                app.canvas_dialogs.convert_archetype.apply_preview_payload(&out.payload);
            }
        }
        ActiveDialog::CommandPalette => {
            if let Some(q) = fields.get("query").or_else(|| fields.get("palette.query")).and_then(Value::as_str) {
                app.canvas_dialogs.command_palette.query = q.to_owned();
            }
        }
        ActiveDialog::FormatGraph { .. } => {
            app.canvas_dialogs.format_graph.apply_field_patch(fields).map_err(bad_input)?;
        }
        ActiveDialog::Print { .. } => {
            app.canvas_dialogs.print.apply_field_patch(fields).map_err(bad_input)?;
        }
        _ => {}
    }
    Ok(())
}

fn commit_active_dialog_action(app: &mut UiApp, action: &str, params: &Value) -> Result<Value, EngineError> {
    let dialog_snapshot = app.canvas_dialogs.active_dialog.clone();
    match dialog_snapshot {
        ActiveDialog::PowerAndSampleSize => commit_power_wizard_action(app, action),
        ActiveDialog::FormatSelectedPoints { .. } => commit_format_points_action(app),
        ActiveDialog::AnalyzeSheet { .. } => commit_analyze_sheet_action(app, params),
        ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } => commit_duplicate_prompt(app, &existing_result_ids, params),
        ActiveDialog::EditAnalysisParameters { .. } => commit_edit_params_action(app, action),
        ActiveDialog::ChangeDataAnalyzed { .. } => commit_single_intent(app, app.canvas_dialogs.change_data.commit_intent(), true),
        ActiveDialog::ChangeGraphType { .. } => {
            let intents = app.canvas_dialogs.change_graph_type.commit_intents(app.document());
            commit_many_intents_opt_close(app, intents, action != "apply")
        }
        ActiveDialog::FormatAxes { .. } => commit_single_intent(app, app.canvas_dialogs.format_axes.commit_intent(), true),
        ActiveDialog::PairwiseComparisons { .. } => commit_single_intent(app, app.canvas_dialogs.pairwise.commit_intent(), true),
        ActiveDialog::BubblePlotInspector { .. } => commit_single_intent(app, app.canvas_dialogs.bubble_plot.commit_intent(), true),
        ActiveDialog::ApplyStyleFrom { .. } => commit_single_intent(app, app.canvas_dialogs.apply_style.commit_intent(), true),
        ActiveDialog::FormatGraph { .. } => commit_many_intents_opt_close(app, app.canvas_dialogs.format_graph.commit_intents(), action != "apply"),
        ActiveDialog::PageLayoutComposer { .. } => commit_many_intents_opt_close(app, app.canvas_dialogs.page_layout.commit_intents(), true),
        ActiveDialog::ExportFigure { sheet_id } => {
            let intent = app.canvas_dialogs.export_figure.commit_intent(&sheet_id);
            commit_single_intent(app, Some(intent), true)
        }
        ActiveDialog::ConvertArchetype { .. } => {
            if action == "preview" || app.canvas_dialogs.convert_archetype.preview_only {
                let intent = app
                    .canvas_dialogs
                    .convert_archetype
                    .preview_intent()
                    .ok_or_else(|| bad_input("no data sheet bound for archetype conversion preview"))?;
                let out = app.execute_command(&intent.command_id, intent.params)?;
                app.canvas_dialogs.convert_archetype.apply_preview_payload(&out.payload);
                if action == "ok" {
                    app.canvas_dialogs.close_dialog();
                }
                Ok(json!({ "action": action, "mutated": out.mutated, "preview_only": true, "payload": out.payload }))
            } else {
                app.commit_convert_archetype().map(|o| json!({ "mutated": o.mutated, "payload": o.payload })).map_err(bad_input)
            }
        }
        ActiveDialog::ImportReshape => {
            let intent = app.grid.import_reshape.build_import_intent().map_err(bad_input)?;
            commit_single_intent(app, Some(intent), true)
        }
        ActiveDialog::ExcludeWithReason { .. } => {
            let intent = app.canvas_dialogs.exclude_with_reason.commit_intent().map_err(bad_input)?;
            commit_single_intent(app, Some(intent), true)
        }
        ActiveDialog::Print { .. } => {
            let out = app.execute_print_dialog_spool(false).map_err(bad_input)?;
            app.canvas_dialogs.close_dialog();
            Ok(json!({ "action": action, "printed": true, "page_count": out.page_count }))
        }
        ActiveDialog::CommandPalette => commit_command_palette_action(app),
        ActiveDialog::ConstantHookPicker { .. } => commit_single_intent(app, app.canvas_dialogs.hook_picker.commit_intent(), true),
        ActiveDialog::ResultViewer { .. } | ActiveDialog::About | ActiveDialog::None => {
            app.canvas_dialogs.close_dialog();
            Ok(json!({ "action": action, "closed": true }))
        }
    }
}

fn commit_power_wizard_action(app: &mut UiApp, action: &str) -> Result<Value, EngineError> {
    match action {
        "next" => {
            let next = app.canvas_dialogs.power_wizard.advance().map_err(disabled_err)?;
            Ok(json!({ "action": "next", "stage": next.label(), "stage_index": next.index() }))
        }
        "back" => {
            let prev = app.canvas_dialogs.power_wizard.go_back().map_err(disabled_err)?;
            Ok(json!({ "action": "back", "stage": prev.label(), "stage_index": prev.index() }))
        }
        "create" | "ok" => {
            let created = app.canvas_dialogs.power_wizard.execute_create(&mut app.session)?;
            let state_copy = app.canvas_dialogs.power_wizard.clone();
            app.power_wizards_by_sheet.insert(created.result_sheet_id.clone(), state_copy);
            app.canvas_dialogs.close_dialog();
            app.sync_views();
            Ok(json!({
                "action": action,
                "created": true,
                "result_sheet_id": created.result_sheet_id.as_str(),
                "graph_sheet_id": created.graph_sheet_id.as_ref().map(SheetId::as_str),
                "data_sheet_id": created.data_sheet_id.as_str(),
                "info_sheet_id": created.info_sheet_id.as_ref().map(SheetId::as_str),
            }))
        }
        _ => Err(bad_input(format!("unsupported Power Wizard action `{action}`"))),
    }
}

fn commit_format_points_action(app: &mut UiApp) -> Result<Value, EngineError> {
    if app.canvas_dialogs.format_points.selected_points.is_empty()
        && let Some(active_ds_id) = app.brushed_points.first().map(|p| p.sheet_id.clone())
        && let Some(ds) = app.document().data_sheet(&active_ds_id)
    {
        let pts = app.grid.selected_graph_point_indices(ds, usize::MAX);
        app.canvas_dialogs.format_points.selected_points = pts;
    }
    let mut outcomes = Vec::new();
    if let Some(intent) = app.canvas_dialogs.format_points.build_commit_intent() {
        let out = app.execute_command(&intent.command_id, intent.params)?;
        outcomes.push(out.payload);
    }
    let row_label_intents = app.canvas_dialogs.format_points.build_row_label_intents(app.document());
    for intent in row_label_intents {
        let out = app.execute_command(&intent.command_id, intent.params)?;
        outcomes.push(out.payload);
    }
    let count = app.canvas_dialogs.format_points.selected_points.len();
    app.canvas_dialogs.close_dialog();
    Ok(json!({ "action": "ok", "formatted_count": count, "outcomes": outcomes }))
}

fn commit_analyze_sheet_action(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let tab_id_opt = app.canvas_dialogs.analyze.extracted_result_tab_id.clone();
    let src_id_opt = app.canvas_dialogs.analyze.source_sheet_id.clone();
    if let (Some(src_id), Some(tab_id)) = (src_id_opt, tab_id_opt)
        && app.document().result_sheet(&src_id).is_some_and(|rs| rs.analyzable_data.is_none())
    {
        let ext = app.execute_command(
            "sheet.extractResultTab",
            json!({
                "result_sheet_id": src_id.as_str(),
                "tab_id": tab_id,
                "archetype": "auto",
                "auto_graph": false,
            }),
        )?;
        let extracted_id = ext
            .payload
            .get("sheet_id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "extractResultTab did not return `sheet_id`"))?;
        let sid = SheetId::try_new(extracted_id).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?;
        app.canvas_dialogs.analyze.source_sheet_id = Some(sid);
        app.canvas_dialogs.analyze.extracted_result_tab_id = None;
    }

    let force_new = params.get("force_create_new").and_then(Value::as_bool).unwrap_or(false);
    if !force_new && let Some(intent) = app.canvas_dialogs.submit_analyze_or_prompt(app.session.document()) {
        let out = app.execute_command(&intent.command_id, intent.params)?;
        if let Some(new_id) = out.payload.get("sheet_id").and_then(Value::as_str) {
            let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
        }
        return Ok(json!({ "action": "ok", "mutated": out.mutated, "payload": out.payload }));
    }
    if !force_new && matches!(app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        return Ok(json!({ "action": "ok", "dialog": "duplicate_analysis_prompt", "pending_confirmation": true }));
    }
    let intent = app.canvas_dialogs.analyze.build_create_new_intent().ok_or_else(|| bad_input("Analyze selection incomplete"))?;
    let out = app.execute_command(&intent.command_id, intent.params)?;
    if let Some(new_id) = out.payload.get("sheet_id").and_then(Value::as_str) {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
    }
    app.canvas_dialogs.close_dialog();
    Ok(json!({ "action": "ok", "mutated": out.mutated, "payload": out.payload }))
}

fn commit_duplicate_prompt(app: &mut UiApp, existing_ids: &[SheetId], params: &Value) -> Result<Value, EngineError> {
    let choice_str =
        params.get("choice").or_else(|| params.get("fields").and_then(|f| f.get("choice"))).and_then(Value::as_str).unwrap_or("create_new");
    let choice = if choice_str.eq_ignore_ascii_case("modify_existing")
        && let Some(first) = existing_ids.first()
    {
        AnalyzeDuplicateChoice::ModifyExisting(first.clone())
    } else {
        AnalyzeDuplicateChoice::CreateNew
    };
    let intent =
        app.canvas_dialogs.analyze.resolve_duplicate_choice(&choice).ok_or_else(|| bad_input("failed to resolve duplicate analysis choice"))?;
    commit_single_intent(app, Some(intent), true)
}

fn commit_edit_params_action(app: &mut UiApp, action: &str) -> Result<Value, EngineError> {
    let close = action == "ok";
    let intent = app.canvas_dialogs.edit_params.commit_intent().ok_or_else(|| bad_input("missing analysis parameter draft"))?;
    let out = app.execute_command(&intent.command_id, intent.params)?;
    if close {
        app.canvas_dialogs.close_dialog();
    }
    Ok(json!({ "action": action, "mutated": out.mutated, "closed": close, "payload": out.payload }))
}

fn commit_command_palette_action(app: &mut UiApp) -> Result<Value, EngineError> {
    let hits = app.canvas_dialogs.command_palette.matches_for_session(app.session());
    let selected = hits
        .get(app.canvas_dialogs.command_palette.selected_index)
        .or_else(|| hits.first())
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, "no matching command in palette"))?
        .clone();
    app.canvas_dialogs.close_dialog();
    if let Some(res) = dispatch_ui_command_as_ui(app, selected.id, &Value::Null)? {
        return Ok(json!({ "action": "ok", "command_id": selected.id, "result": res }));
    }
    let out = app.execute_command(selected.id, Value::Null)?;
    Ok(json!({ "action": "ok", "command_id": selected.id, "mutated": out.mutated, "payload": out.payload }))
}

fn commit_single_intent(app: &mut UiApp, intent: Option<lumencraft_tools::ToolCommandIntent>, close: bool) -> Result<Value, EngineError> {
    let Some(i) = intent else {
        return Err(bad_input("active dialog has no valid commit parameters"));
    };
    let out = app.execute_command(&i.command_id, i.params)?;
    if close {
        app.canvas_dialogs.close_dialog();
    }
    Ok(json!({ "action": "ok", "command_id": i.command_id, "mutated": out.mutated, "payload": out.payload }))
}

fn commit_many_intents_opt_close(app: &mut UiApp, intents: Vec<lumencraft_tools::ToolCommandIntent>, close: bool) -> Result<Value, EngineError> {
    let mut payloads = Vec::with_capacity(intents.len());
    for i in intents {
        let out = app.execute_command(&i.command_id, i.params)?;
        payloads.push(out.payload);
    }
    if close {
        app.canvas_dialogs.close_dialog();
    }
    Ok(json!({ "action": if close { "ok" } else { "apply" }, "count": payloads.len(), "closed": close, "payloads": payloads }))
}

fn parse_summary_measure_str(raw: &str) -> Result<SummaryMeasure, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "mean" => Ok(SummaryMeasure::Mean),
        "median" => Ok(SummaryMeasure::Median),
        "geometricmean" | "geomean" | "geometric" => Ok(SummaryMeasure::GeometricMean),
        _ => Err(format!("unsupported summary_measure `{raw}`")),
    }
}

fn parse_error_bar_metric_str(raw: &str) -> Result<ErrorBarMetric, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "sd" => Ok(ErrorBarMetric::Sd),
        "sem" => Ok(ErrorBarMetric::Sem),
        "ci95" | "confidenceinterval95" | "95ci" => Ok(ErrorBarMetric::ConfidenceInterval95),
        "iqr" | "interquartilerange" => Ok(ErrorBarMetric::InterquartileRange),
        "range" | "minmax" => Ok(ErrorBarMetric::Range),
        "explicitsubcolumns" | "explicit" => Ok(ErrorBarMetric::ExplicitSubcolumns),
        _ => Err(format!("unsupported error_bar_metric `{raw}`")),
    }
}
