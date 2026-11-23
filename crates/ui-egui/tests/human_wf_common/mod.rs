#![forbid(unsafe_code)]
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Shared zero-unwrap native `egui` human-path workflow test helpers (`R15.4`, `R15.5`).
//!
//! Drives `UiApp` strictly through [`EguiHumanPathDriver`] pointer, wheel, keyboard, and IME
//! events against [`render_egui_shell_frame`], enforcing `verify_pixel_to_hit_box_parity()`,
//! `check_scrollbar_non_overlap()`, and `14`-locale `gui-layout-spec.md` §9.4 gates.

use std::sync::Arc;

use curvebook_doc::{ErrorCode, SheetId, SheetSection};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::dispatch_gui_action;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend, UiIntentOutcome};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::{RenderTargetSpec, UiApp};
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

/// Shared workflow test return type.
pub type TestRes = Result<(), String>;

/// Default desktop viewport (`1280x800 @ 1.0x`) for human-path workflow tests.
#[must_use]
pub fn default_viewport() -> ViewportMetrics {
    ViewportMetrics::new(1280.0, 800.0, 1.0)
}

pub mod workflow_run;

/// Live `egui` shell test state tracked by [`EguiHumanPathDriver`] across multi-pass frames.
pub struct WfShellState {
    pub app: UiApp,
    pub views: EguiShellViewState,
    pub intents: PendingUiIntents,
    pub dialog_rect_pt: Option<egui::Rect>,
    pub last_intent_error: Option<String>,
    pub intent_errors: Vec<String>,
    pub recorded_outcomes: Vec<UiIntentOutcome>,
}

impl WfShellState {
    /// Wrap `app` for strict native `egui` GUI-only + human-path execution.
    #[must_use]
    pub fn new(mut app: UiApp) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        app.set_human_path_only(true);
        let views = app.egui_views.clone();
        Self {
            app,
            views,
            intents: PendingUiIntents::new(),
            dialog_rect_pt: None,
            last_intent_error: None,
            intent_errors: Vec::new(),
            recorded_outcomes: Vec::new(),
        }
    }
}

/// Canonical pure-`egui` workflow harness backed by [`EguiHumanPathDriver`].
pub struct EguiWfHarness {
    pub driver: EguiHumanPathDriver<'static, WfShellState>,
}

impl EguiWfHarness {
    /// Create a fresh strict `egui` desktop harness under `metrics`.
    #[must_use]
    pub fn new(metrics: ViewportMetrics) -> Self {
        Self::new_with_clipboard(metrics, Arc::new(StubPlatformClipboardAdapter::new()))
    }

    /// Create a fresh strict `egui` desktop harness wired to a caller-held [`StubPlatformClipboardAdapter`].
    #[must_use]
    pub fn new_with_clipboard(metrics: ViewportMetrics, clipboard: Arc<StubPlatformClipboardAdapter>) -> Self {
        let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        app.set_human_path_only(true);
        app.set_clipboard_adapter(clipboard);
        Self::from_app(app, metrics)
    }

    /// Construct a harness around an existing pre-seeded [`UiApp`].
    #[must_use]
    pub fn from_app(mut app: UiApp, metrics: ViewportMetrics) -> Self {
        if !app.has_custom_clipboard_adapter() {
            app.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
        }
        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let mut driver =
            EguiHumanPathDriver::with_tokens_and_fonts(WfShellState::new(app), metrics, tokens, build_driver_font_definitions(), |ui, meta, st| {
                let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
                st.dialog_rect_pt = rep.regions.dialog_rect_pt;
            });
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        driver.set_post_frame_hook(|st, settled| {
            if settled {
                st.app.egui_views = st.views.clone();
                if !st.intents.is_empty() {
                    let batch = st.intents.commit_all_if_settled(&mut st.app, false);
                    for outcome in &batch.results {
                        match outcome {
                            Ok(out) => st.recorded_outcomes.push(out.clone()),
                            Err(err) => st.intent_errors.push(err.to_string()),
                        }
                    }
                    st.last_intent_error = batch.first_error().map(ToString::to_string);
                }
                st.views = st.app.egui_views.clone();
            } else {
                st.intents.clear();
            }
        });
        driver.run_settled();
        Self { driver }
    }

    /// Borrow the active [`UiApp`].
    #[must_use]
    pub fn app(&self) -> &UiApp {
        &self.driver.state().app
    }

    /// Mutably borrow the active [`UiApp`].
    pub fn app_mut(&mut self) -> &mut UiApp {
        &mut self.driver.state_mut().app
    }

    /// Locate the visible screen center (in logical points) of `wid` in the settled frame.
    pub fn widget_center(&self, wid: &str) -> Result<Pos2, String> {
        let entry = self.driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
        if !entry.clipped_rect.is_positive() {
            return Err(format!("widget `{wid}` has empty bounds {:?}", entry.clipped_rect));
        }
        Ok(entry.clipped_rect.center())
    }

    /// Click `wid` via real pointer move/down/up at its rendered center.
    pub fn click(&mut self, wid: &str) -> TestRes {
        self.driver.click_widget_id(wid)
    }

    /// Double-click `wid` via real pointer events at its rendered center.
    pub fn dbl_click_widget(&mut self, wid: &str) -> TestRes {
        let pos = self.widget_center(wid)?;
        self.driver.click_at(pos)?;
        self.driver.click_at(pos)?;
        Ok(())
    }

    /// Focus textbox `wid`, select all, replace contents with `text`, and commit if needed.
    pub fn replace_textbox(&mut self, wid: &str, text: &str) -> TestRes {
        self.click(wid)?;
        self.driver.press_egui_key(Key::A, Modifiers::COMMAND);
        if text.is_empty() {
            self.driver.press_key("Backspace", KeyModifiers::default())?;
        } else {
            self.driver.type_text(text)?;
        }
        Ok(())
    }

    /// Double-click grid cell `wid`, select all, type `text`, and commit via `Enter`.
    pub fn type_replace_cell(&mut self, wid: &str, text: &str) -> TestRes {
        self.dbl_click_widget(wid)?;
        self.driver.press_egui_key(Key::A, Modifiers::COMMAND);
        self.driver.type_text(text)?;
        self.driver.press_key("Enter", KeyModifiers::default())?;
        Ok(())
    }

    /// Enter `text` into data grid cell `(row, col, subcol)`.
    pub fn enter_grid_cell(&mut self, row: usize, col: usize, subcol: usize, text: &str) -> TestRes {
        let wid = format!("grid.cell.{row}.{col}.{subcol}");
        self.type_replace_cell(&wid, text)
    }

    /// Enter `text` into X-column cell `(row, subcol)`.
    pub fn enter_x_cell(&mut self, row: usize, subcol: usize, text: &str) -> TestRes {
        let wid = format!("grid.xcell.{row}.{subcol}");
        self.type_replace_cell(&wid, text)
    }

    /// Create a data table via the Welcome Hub using real pointer/keyboard input.
    pub fn create_table_via_welcome(&mut self, arch_wid: &str, title: &str, extra_clicks: &[&str]) -> Result<SheetId, String> {
        if !self.driver.state().views.launcher_open && self.app().document().total_sheet_count() > 0 {
            self.click("toolbar.home")?;
        }
        self.click(arch_wid)?;
        for wid in extra_clicks {
            self.click(wid)?;
        }
        let title_pos = self.widget_center("welcome.input.title")?;
        self.driver.click_at(title_pos)?;
        self.driver.press_egui_key(Key::A, Modifiers::COMMAND);
        self.driver.type_text(title)?;
        let before = self.app().document().data_sheets.len();
        self.click("welcome.btn.create")?;
        if self.app().document().data_sheets.len() <= before {
            return Err(format!("Welcome Create did not add a sheet for `{title}`"));
        }
        self.app().document().data_sheets.last().map(|s| s.id.clone()).ok_or_else(|| String::from("missing created sheet"))
    }

    /// Select `id` via the Project Navigator filter chip and sheet tree row.
    pub fn select_sheet(&mut self, id: &SheetId) -> TestRes {
        let wid = format!("nav.sheet.{}", id.as_str());
        if self.driver.frame_meta().find_by_widget_id(&wid).is_none() {
            let chip = match self.app().document().sheet_section(id) {
                Some(SheetSection::Data) => "nav.chip.data",
                Some(SheetSection::Info) => "nav.chip.info",
                Some(SheetSection::Analyses) => "nav.chip.results",
                Some(SheetSection::Graphs) => "nav.chip.graphs",
                Some(SheetSection::Layouts) => "nav.chip.layouts",
                None => "nav.chip.all",
            };
            self.click(chip)?;
        }
        self.click(&wid)
    }

    /// Change an MV column role badge via popup click.
    pub fn set_mv_role(&mut self, col: usize, role_slug: &str) -> TestRes {
        self.click(&format!("mv_formula.role_badge.{col}"))?;
        self.click(&format!("mv_formula.role_opt.{col}.{role_slug}"))
    }

    /// Populate a 4-column, 5-row MV table `sid` with categorical group + 3 continuous columns.
    pub fn seed_mv_4col_5row(&mut self, sid: &SheetId) -> TestRes {
        self.select_sheet(sid)?;
        self.set_mv_role(0, "categorical")?;
        let groups = ["Ctrl", "Ctrl", "Drug", "Drug", "Drug"];
        for (r, grp) in groups.iter().enumerate() {
            self.enter_grid_cell(r, 0, 0, grp)?;
            self.enter_grid_cell(r, 1, 0, &format!("{:.1}", 10.0 + r as f64 * 2.5))?;
            self.enter_grid_cell(r, 2, 0, &format!("{:.1}", 20.0 + r as f64 * 3.0))?;
            self.enter_grid_cell(r, 3, 0, &format!("{:.1}", 5.0 + (r % 2) as f64 * 4.0 + r as f64))?;
        }
        Ok(())
    }

    /// Create and populate a 13-row MV table (`Group` + 3 numeric variables) via Welcome Hub.
    pub fn create_and_seed_mv(&mut self, title: &str) -> Result<SheetId, String> {
        let sid =
            self.create_table_via_welcome("welcome.arch.multiple_variables", title, &["welcome.stepper.cols.inc", "welcome.stepper.rows.inc"])?;
        self.select_sheet(&sid)?;
        self.set_mv_role(0, "categorical")?;
        self.set_mv_role(1, "continuous")?;
        self.set_mv_role(2, "continuous")?;
        self.set_mv_role(3, "continuous")?;
        let col1_vals = [10.0, 14.0, 18.0, 22.0, 26.0, 12.0, 19.0, 25.0, 31.0, 38.0, 16.0, 29.0, 34.0];
        let col2_vals = [0, 1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 1];
        let col3_vals = [5.2, 8.7, 6.1, 11.4, 9.8, 17.3, 24.2, 20.6, 26.5, 23.1, 13.4, 28.9, 19.2];
        for r in 0..13 {
            self.enter_grid_cell(r, 0, 0, if r < 6 { "Ctrl" } else { "Drug" })?;
            self.enter_grid_cell(r, 1, 0, &format!("{:.1}", col1_vals[r]))?;
            self.enter_grid_cell(r, 2, 0, &format!("{}", col2_vals[r]))?;
            self.enter_grid_cell(r, 3, 0, &format!("{:.1}", col3_vals[r]))?;
        }
        Ok(sid)
    }

    /// Open `EditAnalysisParameters` via `Cmd+T`.
    pub fn open_cmd_t(&mut self) -> TestRes {
        self.driver.press_key("T", KeyModifiers { meta: true, ..Default::default() })?;
        if !matches!(self.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
            return Err(format!("Cmd+T failed to open EditAnalysisParameters: {:?}", self.app().canvas_dialogs.active_dialog));
        }
        Ok(())
    }

    /// Open the Analyze modal via `toolbar.analyze`, filter, select `entry_wid`, and confirm.
    pub fn run_analyze_entry(&mut self, search_query: &str, entry_wid: &str) -> Result<SheetId, String> {
        let before = self.app().document().result_sheets.len();
        self.click("toolbar.analyze")?;
        if !search_query.is_empty() && self.driver.frame_meta().find_by_widget_id("analyze.search").is_some() {
            self.click("analyze.search")?;
            self.driver.press_egui_key(Key::A, Modifiers::COMMAND);
            self.driver.type_text(search_query)?;
        }
        self.click(entry_wid)?;
        self.click("modal.ok")?;
        if matches!(self.app().canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
            self.click("analyze_picker.dup.create_new")?;
            self.click("modal.ok")?;
        }
        if self.app().document().result_sheets.len() <= before {
            return Err(format!(
                "Analyze `{entry_wid}` did not create ResultSheet (active_dialog={:?}, err={:?})",
                self.app().canvas_dialogs.active_dialog,
                self.driver.state().last_intent_error
            ));
        }
        self.app().document().result_sheets.last().map(|r| r.id.clone()).ok_or_else(|| String::from("missing created ResultSheet"))
    }

    /// Assert clean `egui` pixel-to-hit-box parity and AccessKit `Role::ScrollBar` non-overlap.
    pub fn assert_parity_and_layout(&mut self, label: &str) -> TestRes {
        let report = self.driver.verify_pixel_to_hit_box_parity();
        if !report.is_clean() {
            return Err(format!("{label}: parity violations: {:?}", report.violations));
        }
        let overlaps = self.driver.check_scrollbar_non_overlap();
        if !overlaps.is_empty() {
            return Err(format!("{label}: scrollbar overlaps: {overlaps:?}"));
        }
        Ok(())
    }

    /// Sweep all 14 active Beta locales across `1440x600@1.0x` and `640x420@1.5x` §9.4 layout gates.
    pub fn verify_14_locales_section_9_4(&mut self, label: &str) -> TestRes {
        let saved_locale = self.app().locale.clone();
        for &loc in &BETA_LOCALE_CODES {
            assert_eq!(self.app_mut().set_locale(loc), loc, "{label}: set_locale({loc}) fell back unexpectedly");
            self.driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
            self.assert_parity_and_layout(&format!("{label} {loc} 1440x600"))?;
            self.driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.5));
            self.assert_parity_and_layout(&format!("{label} {loc} 640x420@1.5x"))?;
        }
        assert_eq!(self.app_mut().set_locale(&saved_locale), saved_locale, "{label}: restoring locale `{saved_locale}` fell back unexpectedly");
        self.driver.set_viewport(ViewportMetrics::new(1280.0, 800.0, 1.0));
        Ok(())
    }

    /// Capture a software rasterized PNG screenshot of `region` through `EguiSoftwareRasterizer`.
    pub fn screenshot_png(&mut self, region: ScreenshotRegionTarget) -> Result<Vec<u8>, String> {
        self.app_mut()
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 }, region)
            .map(|(b, _, _)| b)
            .map_err(|e| e.to_string())
    }
}

/// Assert `cond` holds or return `msg` as a test error.
pub fn ensure(cond: bool, msg: impl Into<String>) -> TestRes {
    if cond { Ok(()) } else { Err(msg.into()) }
}

/// Construct a default `1280x800` strict [`EguiWfHarness`].
#[must_use]
pub fn new_strict_egui_driver() -> EguiWfHarness {
    EguiWfHarness::new(default_viewport())
}

fn assert_bypass_blocked(label: &str, res: Result<serde_json::Value, curvebook_engine::EngineError>) -> TestRes {
    match res {
        Ok(val) => Err(format!("zero-shortcut compliance failed: `{label}` unexpectedly succeeded with {val}")),
        Err(err) if err.code == ErrorCode::GuiOnlyViolation => Ok(()),
        Err(err) => Err(format!("zero-shortcut compliance failed: `{label}` returned {:?} instead of GuiOnlyViolation", err.code)),
    }
}

/// Verify strict `gui_only` + `human_path_only` guard enforcement and `egui` hit-box/scrollbar parity.
pub fn assert_zero_shortcut_compliance(h: &mut EguiWfHarness) -> TestRes {
    if !h.app().is_gui_only() {
        return Err(String::from("zero-shortcut compliance failed: UiApp::is_gui_only() is false"));
    }
    if !h.app().is_human_path_only() {
        return Err(String::from("zero-shortcut compliance failed: UiApp::is_human_path_only() is false"));
    }

    let mut probe = h.app().clone();
    probe.canvas_dialogs.active_dialog = ActiveDialog::About;
    assert_bypass_blocked(
        "ui.invoke_menu(params)",
        dispatch_gui_action(&mut probe, "ui.invoke_menu", json!({ "command_id": "sheet.createData", "params": { "kind": "column" } })),
    )?;
    assert_bypass_blocked("ui.answer_dialog", dispatch_gui_action(&mut probe, "ui.answer_dialog", json!({ "action": "ok", "fields": {} })))?;
    assert_bypass_blocked(
        "ui.pointer_gesture(widget_id)",
        dispatch_gui_action(&mut probe, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" })),
    )?;
    probe.focus_widget_id = Some(String::from("probe.focus.a"));
    assert_bypass_blocked(
        "ui.type_text(focus_jump)",
        dispatch_gui_action(&mut probe, "ui.type_text", json!({ "widget_id": "probe.focus.b", "text": "x" })),
    )?;

    h.assert_parity_and_layout("zero_shortcut_compliance")
}

/// Assert worst-case input-to-paint latency stays within `budget_ms` (applying `1500.0 ms` floor under debug builds).
pub fn assert_input_to_paint_budget(h: &EguiWfHarness, budget_ms: f64) -> TestRes {
    let max_ms = h.driver.max_input_to_paint_ms();
    let effective_budget = if cfg!(debug_assertions) { budget_ms.max(1500.0) } else { budget_ms };
    if max_ms > effective_budget {
        return Err(format!("input-to-paint latency budget exceeded: {max_ms:.2} ms > {effective_budget:.2} ms"));
    }
    Ok(())
}
