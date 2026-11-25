#![forbid(unsafe_code)]
#![allow(dead_code)]

//! Suite-local helper functions shared across `r10_perf_suite` modules.

use crate::common::{ShellTestState, TestRes, ensure, make_shell_driver_with_state, replace_focused_text, sync_app_to_driver, widget_center};
use curvebook_doc::{DataSheet, GraphKind, GraphSheet, MultipleVariablesTable, ProjectDocument, SheetId, TableArchetype, VariableRole};
use curvebook_engine::Session;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver};
use curvebook_ui_egui::egui_shell::inspect_bridge::{check_accesskit_scrollbar_non_overlap, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens};
use curvebook_ui_egui::inspect::ParityViolation;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::{UiApp, UiThemeMode, dispatch_gui_action};
use serde_json::{Value, json};

/// Build a [`UiApp`] with welcome/launcher closed and `human_path_only` enabled.
pub fn new_human_app(doc: ProjectDocument) -> UiApp {
    let mut app = new_shell_app(doc);
    app.set_human_path_only(true);
    app
}

/// Build a [`UiApp`] with welcome/launcher closed and views synced (without `human_path_only`).
pub fn new_shell_app(doc: ProjectDocument) -> UiApp {
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;
    app.sync_views();
    app
}

/// Build a settled [`EguiHumanPathDriver`] with synchronous chart rasterization.
pub fn make_sync_driver_metrics(app: UiApp, metrics: ViewportMetrics) -> EguiHumanPathDriver<'static, ShellTestState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app).with_sync_raster(), metrics, tokens)
}

/// Build a settled [`EguiHumanPathDriver`] with synchronous rasterization at `w × h` (`1.0x`).
pub fn make_sync_driver(app: UiApp, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    make_sync_driver_metrics(app, ViewportMetrics::new(w, h, 1.0))
}

/// Build a settled `human_path_only` driver from `doc` at `w × h` (`1.0x`).
pub fn make_human_driver(doc: ProjectDocument, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    make_sync_driver(new_human_app(doc), w, h)
}

/// Execute an engine command on `app` and return its JSON payload.
pub fn run_cmd(app: &mut UiApp, cmd: &str, params: Value) -> Result<Value, String> {
    app.execute_command(cmd, params).map(|o| o.payload).map_err(|e| format!("{cmd}: {e}"))
}

/// Execute an analysis/transform command on `app` and parse its returned `sheet_id`.
pub fn run_sheet_cmd(app: &mut UiApp, cmd: &str, params: Value) -> Result<SheetId, String> {
    let payload = run_cmd(app, cmd, params)?;
    let raw = payload.get("sheet_id").and_then(Value::as_str).ok_or_else(|| format!("{cmd}: missing sheet_id"))?;
    SheetId::try_new(raw).map_err(|e| e.to_string())
}

/// Open a dialog via `ui.invoke_menu` on `app` and sync views.
pub fn dispatch_open_dialog(app: &mut UiApp, cmd_id: &str, params: Value) -> TestRes {
    let _ = dispatch_gui_action(app, "ui.invoke_menu", json!({ "command_id": cmd_id, "params": params })).map_err(|e| e.to_string())?;
    app.sync_views();
    Ok(())
}

/// Open a dialog via `ui.invoke_menu` on `driver` and settle.
pub fn open_menu_dialog(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, cmd_id: &str, params: Value) -> TestRes {
    dispatch_open_dialog(&mut driver.state_mut().app, cmd_id, params)?;
    sync_app_to_driver(driver);
    Ok(())
}

fn open_dialog_sync(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, ok: bool, msg: String) -> TestRes {
    ensure(ok, msg)?;
    sync_app_to_driver(driver);
    Ok(())
}

/// Open `FormatGraph` for `gid` on `driver` and settle.
pub fn open_format_graph(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, gid: &SheetId) -> TestRes {
    let doc = driver.state().app.document().clone();
    let ok = driver.state_mut().app.canvas_dialogs.open_format_graph(&doc, gid);
    open_dialog_sync(driver, ok, format!("failed to open FormatGraph for `{gid}`"))
}

/// Open `BubblePlotInspector` for `gid` on `driver` and settle.
pub fn open_bubble_plot(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, gid: &SheetId) -> TestRes {
    let doc = driver.state().app.document().clone();
    let ok = driver.state_mut().app.canvas_dialogs.open_bubble_plot(&doc, gid);
    open_dialog_sync(driver, ok, format!("failed to open BubblePlotInspector for `{gid}`"))
}

/// Open `EditAnalysisParameters` for `rid` on `driver` and settle.
pub fn open_edit_params(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, rid: &SheetId) -> TestRes {
    let doc = driver.state().app.document().clone();
    let ok = driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc, rid);
    open_dialog_sync(driver, ok, format!("failed to open EditParams for `{rid}`"))
}

/// Click a sequence of widget IDs in order.
pub fn click_all<S>(driver: &mut EguiHumanPathDriver<'_, S>, wids: &[&str]) -> TestRes {
    for &wid in wids {
        driver.click_widget_id(wid)?;
    }
    Ok(())
}

/// Assert every widget ID in `wids` is present in `driver.frame_meta()`.
pub fn assert_widgets<S>(driver: &EguiHumanPathDriver<'_, S>, wids: &[&str]) -> TestRes {
    for &wid in wids {
        ensure(driver.frame_meta().find_by_widget_id(wid).is_some(), format!("missing widget `{wid}`"))?;
    }
    Ok(())
}

/// Assert `wid` is rendered in `driver.frame_meta()` with `entry.enabled == expected`.
pub fn assert_widget_enabled<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str, expected: bool) -> TestRes {
    let entry = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
    ensure(entry.enabled == expected, format!("widget `{wid}`: expected enabled={expected}, got {}", entry.enabled))
}

/// Double-click `wid` at its rendered center.
pub fn dbl_click_widget<S>(driver: &mut EguiHumanPathDriver<'_, S>, wid: &str) -> TestRes {
    let pos = widget_center(driver, wid)?;
    driver.click_at(pos)?;
    driver.click_at(pos)
}

/// Double-click an inline-editable widget, replace its text, and press `commit_key` (`"Enter"` or `"Escape"`).
pub fn inline_edit_widget<S>(driver: &mut EguiHumanPathDriver<'_, S>, wid: &str, text: &str, commit_key: &str) -> TestRes {
    dbl_click_widget(driver, wid)?;
    replace_focused_text(driver, text)?;
    driver.press_key(commit_key, KeyModifiers::default())
}

/// Attach an initialized [`GraphSheet`] of `kind` backed by `ds_id` to `doc`.
pub fn add_init_graph(doc: &mut ProjectDocument, ds_id: &SheetId, gid: &str, kind: GraphKind) -> Result<SheetId, String> {
    let ds = doc.data_sheet(ds_id).ok_or_else(|| format!("missing data sheet `{ds_id}`"))?.clone();
    let id = SheetId::try_new(gid).map_err(|e| e.to_string())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(id.clone(), &ds).map_err(|e| e.to_string())?;
    gs.initialized = true;
    gs.spec.kind = kind;
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Build a seeded [`MultipleVariablesTable`] [`DataSheet`].
pub fn build_mv_sheet(
    id: &str,
    title: &str,
    cols: &[(&str, VariableRole)],
    rows: usize,
    fill: impl FnOnce(&mut MultipleVariablesTable),
) -> Result<DataSheet, String> {
    let mut mv = MultipleVariablesTable::new(cols.len(), rows).map_err(|e| e.to_string())?;
    for (c, &(col_title, role)) in cols.iter().enumerate() {
        mv.variables[c].title = col_title.to_owned();
        mv.variables[c].role = role;
        mv.variables[c].role_inferred = false;
    }
    fill(&mut mv);
    DataSheet::new(SheetId::try_new(id).map_err(|e| e.to_string())?, title, TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())
}

/// Filter modal parity violations excluding canvas backdrop/overlay widgets covered by R13 modals.
pub fn r13_modal_parity_violations(report: &curvebook_ui_egui::inspect::ParityReport) -> Vec<&ParityViolation> {
    report
        .violations
        .iter()
        .filter(|v| {
            v.widget_id != "canvas.viewport"
                && v.widget_id != "canvas.split.viewport"
                && !v.widget_id.starts_with("canvas.layout_panel.")
                && !v.widget_id.starts_with("canvas.graph_ann.")
                && !v.widget_id.starts_with("canvas.image.handle.")
        })
        .collect()
}

/// Measure whether `render_egui_shell_frame` reports `grid_overlay_shown` for `app`.
pub fn render_grid_overlay_shown(app: &UiApp) -> bool {
    let ctx = egui::Context::default();
    let mut views = app.egui_views.clone();
    views.canvas_view.synchronous_raster = true;
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    let mut shown = false;
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() },
        |ui| {
            shown = render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents).grid_overlay_shown;
        },
    );
    out.textures_delta.clear();
    shown
}

/// Verify all 14 Beta locales × `{640x420, 1440x600}` modal capture, 3 themes × `{1.0x, 2.0x}` DPI, and AccessKit non-overlap.
pub fn verify_r13_matrix_and_accesskit(
    base_app: UiApp,
    locale_cases: &[(&str, Value)],
    theme_cases: &[(&str, Value)],
    open_ak_dialog: impl FnOnce(&mut UiApp) -> bool,
) -> TestRes {
    ensure(BETA_LOCALE_CODES.len() == 14, format!("expected 14 beta locales, got {}", BETA_LOCALE_CODES.len()))?;
    for locale in BETA_LOCALE_CODES {
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            for (cmd_id, params) in locale_cases {
                let mut app = base_app.clone();
                ensure(app.set_locale(locale) == locale, format!("set_locale({locale}) fell back"))?;
                dispatch_open_dialog(&mut app, cmd_id, params.clone())?;
                let mut drv = make_sync_driver(app, w_pt, h_pt);
                let parity = drv.verify_pixel_to_hit_box_parity();
                let modal_v = r13_modal_parity_violations(&parity);
                ensure(modal_v.is_empty(), format!("{cmd_id} `{locale}` @ {w_pt}x{h_pt} parity: {modal_v:?}"))?;
                let sb = drv.check_scrollbar_non_overlap();
                ensure(sb.is_empty(), format!("{cmd_id} `{locale}` @ {w_pt}x{h_pt} scrollbar overlap: {sb:?}"))?;

                let outside = egui::pos2(4.0, (h_pt as f32) * 0.5);
                drv.wheel_at(outside, egui::vec2(0.0, -60.0))?;
                drv.click_at(outside)?;
                ensure(drv.state().outside_click_captured, format!("{cmd_id} `{locale}` click capture"))?;
                ensure(drv.state().outside_wheel_captured, format!("{cmd_id} `{locale}` wheel capture"))?;
            }
        }
    }

    for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        for dpi in [1.0_f64, 2.0] {
            for (cmd_id, params) in theme_cases {
                let mut app = base_app.clone();
                app.set_theme_mode(theme);
                dispatch_open_dialog(&mut app, cmd_id, params.clone())?;
                let drv = make_sync_driver_metrics(app, ViewportMetrics::new(1440.0, 600.0, dpi));
                let parity = drv.verify_pixel_to_hit_box_parity();
                let modal_v = r13_modal_parity_violations(&parity);
                ensure(modal_v.is_empty(), format!("{cmd_id} {theme:?} @ {dpi}x parity: {modal_v:?}"))?;
                ensure(drv.check_scrollbar_non_overlap().is_empty(), format!("{cmd_id} {theme:?} @ {dpi}x scrollbar"))?;
            }
        }
    }

    let mut app_ak = base_app;
    ensure(open_ak_dialog(&mut app_ak), "failed to open AccessKit test dialog")?;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut views = EguiShellViewState::default();
    views.canvas_view.synchronous_raster = true;
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
        |ui| {
            let _ = render_egui_shell_frame(ui, &app_ak, &mut views, &mut meta, &mut intents);
        },
    );
    out.textures_delta.clear();
    let ak_update = out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?;
    check_accesskit_scrollbar_non_overlap(ak_update)
}
