//! Shared integration test helpers for `curvebook-ui-egui` suites.
#![forbid(unsafe_code)]
#![allow(dead_code, unused_imports, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSetColumn, DataSheet, GraphKind, GraphSheet, ProjectDocument, ResultSheet,
    SheetId, SubcolumnMode, TableArchetype, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_render::RenderTargetSpec;
pub use curvebook_testkit::harness::{TempDirGuard, assert_close, assert_rel_close};
use curvebook_ui_egui::control_io::handle_control_line;
use curvebook_ui_egui::egui_shell::canvas_view::CanvasHoverReadout;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens};
use curvebook_ui_egui::inspect::{ParityReport, ParityViolation};
use curvebook_ui_egui::interaction::{KeyModifiers, UiEventOutcome, UiInputEvent};
use curvebook_ui_egui::{UiApp, ViewportMetrics, dispatch_ui_event};
use egui::{Key, Modifiers, Pos2, Rect};
use serde_json::{Value, json};

/// Shared result alias for integration tests.
pub type TestRes = Result<(), String>;

/// Standard 8-byte PNG file signature.
pub const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Create a fresh RAII temporary directory with a `String` error type.
pub fn temp_dir(label: &str) -> Result<TempDirGuard, String> {
    TempDirGuard::new(label).map_err(|err| format!("create temp dir `{label}`: {err}"))
}

/// Encode bytes as standard Base64 via `curvebook_platform::base64`.
#[must_use]
pub fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

/// Decode standard Base64 via `curvebook_platform::base64`.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    curvebook_platform::base64::decode(input).map_err(|err| err.to_string())
}

/// Return `Ok(())` when `cond` holds, otherwise `Err(msg)`.
pub fn ensure(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
}

/// Result-returning absolute-tolerance floating-point comparison for test workflows.
pub fn assert_close_res(actual: f64, expected: f64, tol: f64, label: &str) -> Result<(), String> {
    if !actual.is_finite() || (actual - expected).abs() > tol {
        return Err(format!("{label}: expected {expected} +/- {tol}, got {actual}"));
    }
    Ok(())
}

/// Default serialized [`ViewportMetrics`] payload for `ui.inspect` / `ui.screenshot` calls.
#[must_use]
pub fn default_metrics() -> Value {
    serde_json::to_value(ViewportMetrics::default()).unwrap_or_else(|_| json!({}))
}

/// Recursively locate a widget JSON object by `"widget_id"` inside a `ui.inspect` snapshot.
#[must_use]
pub fn find_widget<'a>(snapshot: &'a Value, wid: &str) -> Option<&'a Value> {
    match snapshot {
        Value::Object(map) => {
            if map.get("widget_id").and_then(Value::as_str) == Some(wid) {
                return Some(snapshot);
            }
            for child in map.values() {
                if let Some(hit) = find_widget(child, wid) {
                    return Some(hit);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_widget(item, wid)),
        _ => None,
    }
}

/// Unified shell driver state replacing per-file `ShellDriverState` / `TestState` / `ModalState` structs.
pub struct ShellTestState {
    pub app: UiApp,
    pub views: EguiShellViewState,
    pub intents: PendingUiIntents,
    pub last_intent_error: Option<String>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub leading_rect_pt: Option<Rect>,
    pub trailing_rect_pt: Option<Rect>,
    pub footer_rect_pt: Option<Rect>,
    pub tier1_rect_pt: Option<Rect>,
    pub canvas_rect_pt: Option<Rect>,
    pub hover_readout_override: Option<CanvasHoverReadout>,
    pub synchronous_raster: bool,
    pub auto_commit: bool,
    pub commit_all: bool,
    pub merge_passes: bool,
}

impl ShellTestState {
    /// Wrap an existing [`UiApp`] with default settled single-intent commit behavior.
    #[must_use]
    pub fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self {
            app,
            views,
            intents: PendingUiIntents::new(),
            last_intent_error: None,
            outside_click_captured: false,
            outside_wheel_captured: false,
            dialog_rect_pt: None,
            leading_rect_pt: None,
            trailing_rect_pt: None,
            footer_rect_pt: None,
            tier1_rect_pt: None,
            canvas_rect_pt: None,
            hover_readout_override: None,
            synchronous_raster: false,
            auto_commit: true,
            commit_all: false,
            merge_passes: false,
        }
    }

    /// Wrap a [`ProjectDocument`] into a [`ShellTestState`].
    #[must_use]
    pub fn from_doc(doc: ProjectDocument) -> Self {
        Self::new(UiApp::from_session(Session::from_document(doc)))
    }

    /// Wrap a [`ProjectDocument`] with `human_path_only(true)`, welcome closed, and synchronous raster.
    #[must_use]
    pub fn from_doc_human(doc: ProjectDocument) -> Self {
        let mut app = UiApp::from_session(Session::from_document(doc));
        app.set_human_path_only(true);
        app.nav.welcome.open = false;
        app.egui_views.launcher_open = false;
        let mut st = Self::new(app);
        st.synchronous_raster = true;
        st
    }

    /// Enable multi-intent `commit_all_if_settled` inside the settled post-frame hook.
    #[must_use]
    pub fn with_commit_all(mut self) -> Self {
        self.commit_all = true;
        self
    }

    /// Enable synchronous chart rasterization inside frames.
    #[must_use]
    pub fn with_sync_raster(mut self) -> Self {
        self.synchronous_raster = true;
        self
    }

    /// Disable automatic post-frame intent commits (for tests inspecting queued intents).
    #[must_use]
    pub fn without_auto_commit(mut self) -> Self {
        self.auto_commit = false;
        self
    }

    /// Carry intents across multi-pass renders via `PendingUiIntents::merge_pass`.
    #[must_use]
    pub fn with_merge_passes(mut self) -> Self {
        self.merge_passes = true;
        self
    }
}

/// Build a settled [`EguiHumanPathDriver`] over [`ShellTestState`] with explicit viewport and tokens.
#[must_use]
pub fn make_shell_driver_with_state(state: ShellTestState, metrics: ViewportMetrics, tokens: Tokens) -> EguiHumanPathDriver<'static, ShellTestState> {
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(state, metrics, tokens, build_driver_font_definitions(), |ui, meta, st| {
        if st.synchronous_raster {
            st.views.canvas_view.synchronous_raster = true;
        }
        if let Some(ref ov) = st.hover_readout_override {
            st.views.canvas_view.hover_readout = Some(ov.clone());
        }
        let rep = if st.merge_passes {
            let mut pass = PendingUiIntents::new();
            let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass);
            st.intents.merge_pass(pass);
            rep
        } else {
            render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents)
        };
        st.outside_click_captured |= rep.outside_click_captured;
        st.outside_wheel_captured |= rep.outside_wheel_captured;
        st.dialog_rect_pt = rep.regions.dialog_rect_pt;
        st.leading_rect_pt = rep.leading_dock_rect_pt;
        st.trailing_rect_pt = rep.trailing_dock_rect_pt;
        st.footer_rect_pt = rep.footer_rect_pt;
        st.tier1_rect_pt = rep.tier1_rect_pt;
        st.canvas_rect_pt = rep.regions.canvas_rect_pt;
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if st.auto_commit && !st.intents.is_empty() {
                if st.commit_all {
                    let batch = st.intents.commit_all_if_settled(&mut st.app, false);
                    if let Some(err) = batch.first_error() {
                        st.last_intent_error = Some(err.to_string());
                    }
                } else if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                    st.last_intent_error = Some(err.to_string());
                }
            }
            st.views = st.app.egui_views.clone();
        } else if !st.merge_passes {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

/// Build a settled [`EguiHumanPathDriver`] from `app` and `metrics` using tokens derived from `app`.
#[must_use]
pub fn make_shell_driver(app: UiApp, metrics: ViewportMetrics) -> EguiHumanPathDriver<'static, ShellTestState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app), metrics, tokens)
}

/// Build a settled [`EguiHumanPathDriver`] from `app` at the default desktop viewport (`1280×800`).
#[must_use]
pub fn make_default_driver(app: UiApp) -> EguiHumanPathDriver<'static, ShellTestState> {
    make_shell_driver(app, ViewportMetrics::default())
}

/// Build a settled [`EguiHumanPathDriver`] from `doc` at logical dimensions `w × h` (`1.0x`).
#[must_use]
pub fn make_doc_driver(doc: ProjectDocument, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    let app = UiApp::from_session(Session::from_document(doc));
    make_shell_driver(app, ViewportMetrics::new(w, h, 1.0))
}

/// Manually commit any pending intents on `driver` and settle.
pub fn commit_shell_intents(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) -> Result<(), String> {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    let mut intents = std::mem::take(&mut st.intents);
    let res = intents.commit_if_settled(&mut st.app, false).map(|_| ()).map_err(|e| e.to_string());
    if let Err(ref err) = res {
        st.last_intent_error = Some(err.clone());
    }
    st.views = st.app.egui_views.clone();
    driver.run_settled();
    res
}

/// Synchronize mutated `app` views back into `driver.state_mut().views` and settle.
pub fn sync_app_to_driver(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) {
    let st = driver.state_mut();
    st.app.sync_views();
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

/// Select `sheet_id` on `driver.state_mut().app` and settle.
pub fn select_sheet_and_settle(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, sheet_id: &str) -> TestRes {
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": sheet_id })).map_err(|e| e.to_string())?;
    sync_app_to_driver(driver);
    Ok(())
}

/// Locate the center of `wid` in the current frame's [`FrameInspectMeta`].
pub fn widget_center<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> Result<Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
    Ok(entry.clipped_rect.center())
}

/// Locate the clipped rect of `wid` in the current frame's [`FrameInspectMeta`].
pub fn widget_rect<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> Result<Rect, String> {
    let entry = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
    Ok(entry.clipped_rect)
}

/// Read the boolean `"selected"` metadata flag of `wid` if present.
#[must_use]
pub fn widget_selected<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> Option<bool> {
    driver.frame_meta().find_by_widget_id(wid).and_then(|e| e.value.as_ref()).and_then(|v| v.get("selected")).and_then(Value::as_bool)
}

/// Select-all, clear, and type `text` into the currently focused `egui` textbox, then settle.
pub fn replace_focused_text<S>(driver: &mut EguiHumanPathDriver<'_, S>, text: &str) -> TestRes {
    driver.press_key("a", KeyModifiers { meta: cfg!(target_os = "macos"), ctrl: !cfg!(target_os = "macos"), ..KeyModifiers::default() })?;
    driver.press_key("Backspace", KeyModifiers::default())?;
    if !text.is_empty() {
        driver.type_text(text)?;
    }
    driver.run_settled();
    Ok(())
}

/// Click `wid`, replace its text with `text`, and settle.
pub fn set_textbox<S>(driver: &mut EguiHumanPathDriver<'_, S>, wid: &str, text: &str) -> TestRes {
    driver.click_widget_id(wid)?;
    replace_focused_text(driver, text)
}

/// Click `wid`, replace its text with `text`, press `Enter`, and settle.
pub fn set_textbox_and_commit<S>(driver: &mut EguiHumanPathDriver<'_, S>, wid: &str, text: &str) -> TestRes {
    set_textbox(driver, wid, text)?;
    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    driver.run_settled();
    Ok(())
}

/// Filter [`ParityReport`] violations excluding backdrop-covered canvas viewports during modal checks.
#[must_use]
pub fn modal_parity_violations(report: &ParityReport) -> Vec<&ParityViolation> {
    report.violations.iter().filter(|v| v.widget_id != "canvas.viewport" && v.widget_id != "canvas.split.viewport").collect()
}

/// Assert zero modal parity violations and zero scrollbar overlaps on `driver`.
pub fn assert_modal_parity<S>(driver: &EguiHumanPathDriver<'_, S>, label: &str) -> TestRes {
    let parity = driver.verify_pixel_to_hit_box_parity();
    let filtered = modal_parity_violations(&parity);
    ensure(filtered.is_empty(), format!("{label}: parity violations: {filtered:?}"))?;
    let sb = driver.check_scrollbar_non_overlap();
    ensure(sb.is_empty(), format!("{label}: scrollbar overlap: {sb:?}"))
}

/// Assert full pixel-to-hit-box parity and zero scrollbar overlap on `driver`.
pub fn assert_clean_parity<S>(driver: &EguiHumanPathDriver<'_, S>, label: &str) -> TestRes {
    let parity = driver.verify_pixel_to_hit_box_parity();
    ensure(parity.is_clean(), format!("{label}: parity violations: {:?}", parity.violations))?;
    let sb = driver.check_scrollbar_non_overlap();
    ensure(sb.is_empty(), format!("{label}: scrollbar overlap: {sb:?}"))
}

/// Send a JSON control request through [`handle_control_line`] and parse the JSON response.
pub fn call_control(app: &mut UiApp, req: &Value) -> Result<Value, String> {
    let line = serde_json::to_string(req).map_err(|e| format!("serialize request: {e}"))?;
    let resp_line = handle_control_line(app, &line);
    serde_json::from_str(&resp_line).map_err(|e| format!("parse control response `{resp_line}`: {e}"))
}

/// Assert that a control JSON response failed with `error_detail.code == expected_code`.
pub fn assert_control_error_code(resp: &Value, expected_code: &str, ctx: &str) -> TestRes {
    ensure(resp.get("ok") == Some(&Value::Bool(false)), format!("{ctx}: expected ok=false, got {resp}"))?;
    let actual = resp.pointer("/error_detail/code").and_then(Value::as_str);
    ensure(actual == Some(expected_code), format!("{ctx}: expected error_detail.code={expected_code}, got {actual:?} in {resp}"))
}

/// Dispatch a synthetic `KeyDown` [`UiInputEvent`] at default viewport metrics.
pub fn key_down(app: &mut UiApp, key: &str, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
    dispatch_ui_event(app, UiInputEvent::KeyDown { key: key.to_owned(), modifiers }, ViewportMetrics::default())
}

/// Dispatch a synthetic `TextInput` [`UiInputEvent`] at default viewport metrics.
pub fn text_input(app: &mut UiApp, text: &str) -> Result<UiEventOutcome, String> {
    dispatch_ui_event(app, UiInputEvent::TextInput { text: text.to_owned() }, ViewportMetrics::default())
}

/// Parse `(width, height)` from a PNG IHDR header.
#[must_use]
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || !png.starts_with(PNG_SIGNATURE) || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    Some((w, h))
}

/// Assert `png` is a valid PNG with exact `expected_width × expected_height` IHDR dimensions.
pub fn assert_valid_png_with_dimensions(png: &[u8], expected_width: u32, expected_height: u32) {
    assert!(png.starts_with(PNG_SIGNATURE), "missing PNG magic header");
    assert!(png.len() >= 24, "PNG too short for IHDR chunk");
    assert_eq!(&png[12..16], b"IHDR");
    assert_eq!(png_dimensions(png), Some((expected_width, expected_height)));
}

/// Integer pixel rectangle in a rasterized frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Pixel-space workspace chrome regions measured from a settled `egui` shell frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkspacePixelRegions {
    pub toolbar: PixelRect,
    pub navigator: PixelRect,
    pub canvas: PixelRect,
    pub inspector: PixelRect,
    pub status_bar: PixelRect,
    pub modal_overlay: Option<PixelRect>,
}

/// Convert a logical `egui::Rect` to clamped integer pixel bounds.
#[must_use]
pub fn egui_rect_to_px(rect: Rect, scale: f64, max_w: u32, max_h: u32) -> PixelRect {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let x = ((f64::from(rect.min.x.max(0.0)) * scale).floor() as u32).min(max_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let y = ((f64::from(rect.min.y.max(0.0)) * scale).floor() as u32).min(max_h);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let r = ((f64::from(rect.max.x.max(0.0)) * scale).ceil() as u32).min(max_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = ((f64::from(rect.max.y.max(0.0)) * scale).ceil() as u32).min(max_h);
    PixelRect { x, y, width: r.saturating_sub(x), height: b.saturating_sub(y) }
}

/// Render a settled headless `egui` shell frame and return pixel-space region rects.
#[must_use]
pub fn measure_egui_regions(app: &UiApp, metrics: ViewportMetrics) -> WorkspacePixelRegions {
    let ctx = egui::Context::default();
    curvebook_ui_egui::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
    #[allow(clippy::cast_possible_truncation)]
    let w_pt = metrics.logical_width.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let h_pt = metrics.logical_height.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let ppp = metrics.clamped_scale_factor() as f32;
    ctx.options_mut(|o| o.zoom_factor = app.ui_scale());
    let mut views = app.egui_views.clone();
    let mut report = None;
    for _ in 0..6 {
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(w_pt, h_pt))),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(ppp / app.ui_scale()), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut settled = false;
        let mut out = ctx.run_ui(raw, |ui| {
            let rep = render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents);
            settled = !ui.ctx().will_discard();
            if settled {
                report = Some(rep);
            }
        });
        out.textures_delta.clear();
        if settled {
            break;
        }
    }
    let rep = report.expect("settled egui shell report");
    let scale = metrics.clamped_scale_factor();
    let max_w = metrics.physical_width_px();
    let max_h = metrics.physical_height_px();
    let to_px = |r: Option<Rect>| r.map(|rect| egui_rect_to_px(rect, scale, max_w, max_h));
    let zero = PixelRect::default();
    WorkspacePixelRegions {
        toolbar: to_px(rep.tier1_rect_pt).unwrap_or(zero),
        navigator: to_px(rep.leading_dock_rect_pt).unwrap_or(zero),
        canvas: to_px(rep.regions.canvas_rect_pt).unwrap_or(zero),
        inspector: to_px(rep.trailing_dock_rect_pt).unwrap_or(zero),
        status_bar: to_px(rep.footer_rect_pt).unwrap_or(zero),
        modal_overlay: to_px(rep.regions.dialog_rect_pt),
    }
}

/// Convenience wrapper over [`measure_egui_regions`] taking a [`RenderTargetSpec`].
#[must_use]
pub fn measure_egui_regions_for_target(app: &UiApp, target: RenderTargetSpec) -> WorkspacePixelRegions {
    let scale = f64::from(target.dpi) / 96.0;
    measure_egui_regions(app, ViewportMetrics::from_physical(target.width_px, target.height_px, scale))
}

/// Count distinct RGBA pixel values inside `rect` up to `cap`.
#[must_use]
pub fn region_distinct_colors(rgba: &[u8], frame_width: u32, rect: PixelRect, cap: usize) -> usize {
    let mut colors = Vec::new();
    for py in rect.y..rect.y.saturating_add(rect.height) {
        for px in rect.x..rect.x.saturating_add(rect.width) {
            let idx = ((py as usize) * (frame_width as usize) + (px as usize)) * 4;
            if let Some(&[r, g, b, a]) = rgba.get(idx..idx + 4) {
                let packed = u32::from_be_bytes([r, g, b, a]);
                if !colors.contains(&packed) {
                    colors.push(packed);
                    if colors.len() >= cap {
                        return colors.len();
                    }
                }
            }
        }
    }
    colors.len()
}

/// Fill a [`DataSetColumn`]'s title and subcolumn-0 numeric cells from `values`.
pub fn fill_col_values(col: &mut DataSetColumn, title: &str, values: &[f64]) {
    col.title = title.to_owned();
    for (r, &v) in values.iter().enumerate() {
        if let Some(cell) = col.cell_mut(r, 0) {
            *cell = Cell::number(v);
        }
    }
}

/// Read a numeric cell at `(row, col, subcol=0)` from a [`DataSheet`].
#[must_use]
pub fn cell_number(ds: &DataSheet, row: usize, col: usize) -> Option<f64> {
    match ds.cell(CellCoord::new(row, col, 0))?.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

/// Build a [`ProjectDocument`] containing a single seeded Column [`DataSheet`].
pub fn seed_column_doc(
    project_title: &str,
    sheet_id: &str,
    sheet_title: &str,
    design: ColumnDesignHint,
    cols: &[(&str, &[f64])],
) -> Result<(ProjectDocument, SheetId), String> {
    let max_rows = cols.iter().map(|(_, vals)| vals.len()).max().unwrap_or(1).max(1);
    let mut table = ColumnTable::new(design, cols.len().max(1), max_rows).map_err(|e| e.to_string())?;
    for (c, &(title, values)) in cols.iter().enumerate() {
        fill_col_values(&mut table.data_sets[c], title, values);
    }
    let sid = SheetId::try_new(sheet_id).map_err(|e| e.to_string())?;
    let ds = DataSheet::new(sid.clone(), sheet_title, TableArchetype::Column(table)).map_err(|e| e.to_string())?;
    let mut doc = ProjectDocument::new(project_title).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(sid.clone());
    Ok((doc, sid))
}

/// Build a [`ProjectDocument`] containing a single seeded XY [`DataSheet`] (optionally with an initialized XY graph).
pub fn seed_xy_doc(
    project_title: &str,
    sheet_id: &str,
    sheet_title: &str,
    x_title: &str,
    xs: &[f64],
    ys: &[(&str, &[f64])],
) -> Result<(ProjectDocument, SheetId), String> {
    let rows = xs.len().max(ys.iter().map(|(_, v)| v.len()).max().unwrap_or(1)).max(1);
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, ys.len().max(1), rows)
            .map_err(|e| e.to_string())?;
    xy.x_title = x_title.to_owned();
    for (r, &x) in xs.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
    }
    for (c, &(title, values)) in ys.iter().enumerate() {
        fill_col_values(&mut xy.data_sets[c], title, values);
    }
    let sid = SheetId::try_new(sheet_id).map_err(|e| e.to_string())?;
    let ds = DataSheet::new(sid.clone(), sheet_title, TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    let mut doc = ProjectDocument::new(project_title).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(sid.clone());
    Ok((doc, sid))
}

/// Build a [`UiApp`] with a seeded 2-series XY table and its initialized [`GraphSheet`].
pub fn seed_xy_data_and_graph_app(sheet_id: &str, graph_id: &str) -> Result<(UiApp, SheetId, SheetId), String> {
    let (mut doc, ds_id) = seed_xy_doc(
        "XY Workspace",
        sheet_id,
        "Assay XY",
        "X",
        &[1.0, 2.0, 3.0, 4.0],
        &[("Y1", &[10.0, 20.0, 30.0, 40.0]), ("Y2", &[15.0, 25.0, 35.0, 45.0])],
    )?;
    let ds = doc.data_sheet(&ds_id).ok_or_else(|| String::from("missing seeded xy sheet"))?.clone();
    let gid = SheetId::try_new(graph_id).map_err(|e| e.to_string())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(gid.clone(), &ds).map_err(|e| e.to_string())?;
    gs.initialized = true;
    gs.spec.kind = GraphKind::Xy;
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(ds_id.clone());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    Ok((app, ds_id, gid))
}

/// Look up a top-level scalar by `key` on a [`ResultSheet`].
pub fn result_scalar(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    rs.scalars
        .iter()
        .find(|s| s.key == key && s.data_set_index.is_none())
        .map(|s| s.value)
        .ok_or_else(|| format!("missing scalar `{key}` on `{}`", rs.id.as_str()))
}
