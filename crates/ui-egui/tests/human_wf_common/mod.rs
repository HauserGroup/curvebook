#![forbid(unsafe_code)]
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Shared zero-unwrap native `egui` human-path workflow test helpers (`R15.4`, `R15.5`).
//!
//! Drives `UiApp` strictly through [`EguiHumanPathDriver`] pointer, wheel, keyboard, and IME
//! events against [`render_egui_shell_frame`], enforcing `verify_pixel_to_hit_box_parity()`,
//! `check_scrollbar_non_overlap()`, and `14`-locale `gui-layout-spec.md` §9.4 gates.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use curvebook_doc::{ErrorCode, ProjectDocument, SheetId, SheetSection};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::dispatch_gui_action;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend, UiIntent, UiIntentOutcome};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::os_integration::{
    NativeFileDialogKind, NativeFileDialogRequest, PlatformFileDialogAdapter, StubPlatformClipboardAdapter, StubPlatformFileDialogAdapter,
};
use curvebook_ui_egui::{RenderTargetSpec, UiApp, tr};
use egui::{Key, Modifiers, Pos2};
use serde_json::{Value, json};

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
    pub file_dialog: Arc<StubPlatformFileDialogAdapter>,
    pub dialog_rect_pt: Option<egui::Rect>,
    pub last_intent_error: Option<String>,
    pub intent_errors: Vec<String>,
    pub recorded_outcomes: Vec<UiIntentOutcome>,
}

impl WfShellState {
    /// Wrap `app` for strict native `egui` GUI-only + human-path execution.
    #[must_use]
    pub fn new(app: UiApp) -> Self {
        Self::new_with_file_dialog(app, Arc::new(StubPlatformFileDialogAdapter::new()))
    }

    /// Wrap `app` with a caller-held [`StubPlatformFileDialogAdapter`] for native file-picker tests.
    #[must_use]
    pub fn new_with_file_dialog(mut app: UiApp, file_dialog: Arc<StubPlatformFileDialogAdapter>) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        app.set_human_path_only(true);
        let views = app.egui_views.clone();
        Self {
            app,
            views,
            intents: PendingUiIntents::new(),
            file_dialog,
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
        Self::new_with_adapters(metrics, clipboard, Arc::new(StubPlatformFileDialogAdapter::new()))
    }

    /// Create a fresh strict `egui` desktop harness wired to caller-held clipboard and file-dialog stubs.
    #[must_use]
    pub fn new_with_adapters(
        metrics: ViewportMetrics,
        clipboard: Arc<StubPlatformClipboardAdapter>,
        file_dialog: Arc<StubPlatformFileDialogAdapter>,
    ) -> Self {
        let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        app.set_human_path_only(true);
        app.set_clipboard_adapter(clipboard);
        Self::from_app_with_file_dialog(app, metrics, file_dialog)
    }

    /// Construct a harness around an existing pre-seeded [`UiApp`].
    #[must_use]
    pub fn from_app(app: UiApp, metrics: ViewportMetrics) -> Self {
        Self::from_app_with_file_dialog(app, metrics, Arc::new(StubPlatformFileDialogAdapter::new()))
    }

    /// Construct a harness around an existing pre-seeded [`UiApp`] and caller-held [`StubPlatformFileDialogAdapter`].
    #[must_use]
    pub fn from_app_with_file_dialog(mut app: UiApp, metrics: ViewportMetrics, file_dialog: Arc<StubPlatformFileDialogAdapter>) -> Self {
        if !app.has_custom_clipboard_adapter() {
            app.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
        }
        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            WfShellState::new_with_file_dialog(app, file_dialog),
            metrics,
            tokens,
            build_driver_font_definitions(),
            |ui, meta, st| {
                let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
                st.dialog_rect_pt = rep.regions.dialog_rect_pt;
            },
        );
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        driver.set_post_frame_hook(|st, settled| {
            if settled {
                st.app.egui_views = st.views.clone();
                if !st.intents.is_empty() {
                    let raw_intents = std::mem::take(&mut st.intents);
                    let mut remaining = PendingUiIntents::new();
                    let mut host_commands: Vec<(String, Value)> = Vec::new();
                    for intent in raw_intents.intents().iter().cloned() {
                        match &intent {
                            UiIntent::UiCommand { command_id, params } | UiIntent::EngineCommand { command_id, params }
                                if is_intercepted_host_file_command(command_id, params) =>
                            {
                                host_commands.push((command_id.clone(), params.clone()));
                            }
                            _ => remaining.push(intent),
                        }
                    }
                    let mut first_err: Option<String> = None;
                    if !remaining.is_empty() {
                        let batch = remaining.commit_all_if_settled(&mut st.app, false);
                        for outcome in &batch.results {
                            match outcome {
                                Ok(out) => st.recorded_outcomes.push(out.clone()),
                                Err(err) => st.intent_errors.push(err.to_string()),
                            }
                        }
                        first_err = batch.first_error().map(ToString::to_string);
                    }
                    for (command_id, params) in host_commands {
                        match execute_intercepted_host_file_command(&mut st.app, st.file_dialog.as_ref(), &command_id, &params) {
                            Ok(outcomes) => st.recorded_outcomes.extend(outcomes),
                            Err(err) => {
                                if first_err.is_none() {
                                    first_err = Some(err.clone());
                                }
                                st.intent_errors.push(err);
                            }
                        }
                    }
                    st.last_intent_error = first_err;
                }
                st.views = st.app.egui_views.clone();
            } else {
                st.intents.clear();
            }
        });
        driver.run_settled();
        Self { driver }
    }

    /// Borrow the attached stub native file-dialog adapter.
    #[must_use]
    pub fn file_dialog(&self) -> &StubPlatformFileDialogAdapter {
        self.driver.state().file_dialog.as_ref()
    }

    /// Queue a FIFO scripted response for the next native file dialog request.
    pub fn script_file_dialog(&self, path: Option<PathBuf>) {
        self.driver.state().file_dialog.push_next(path);
    }

    /// Queue a command-keyed scripted response for `command_id`.
    pub fn script_file_dialog_for(&self, command_id: impl Into<String>, path: Option<PathBuf>) {
        self.driver.state().file_dialog.push_response(command_id, path);
    }

    /// Return a snapshot of every [`NativeFileDialogRequest`] sent to the stub file picker.
    #[must_use]
    pub fn recorded_file_dialog_requests(&self) -> Vec<NativeFileDialogRequest> {
        self.driver.state().file_dialog.requests()
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
        if self.app().document().active_sheet_id.as_ref() == Some(id)
            && matches!(self.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. })
        {
            return Ok(());
        }
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

    /// Open `EditAnalysisParameters` via `Cmd+T` (no-op if already open, e.g. auto-opened after Nonlinear Fit creation).
    pub fn open_cmd_t(&mut self) -> TestRes {
        if matches!(self.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
            return Ok(());
        }
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

fn has_non_empty_path(params: &Value) -> bool {
    params.get("path").and_then(Value::as_str).map(str::trim).is_some_and(|s| !s.is_empty())
}

fn is_intercepted_host_file_command(command_id: &str, params: &Value) -> bool {
    match command_id {
        "table.importPrism" | "table.importPzfx" | "table.importXlsx" | "ui.export.allTablesCsv" => true,
        "table.exportCsv" | "analysis.exportCsv" | "analysis.exportJson" | "result.generateReport" => !has_non_empty_path(params),
        _ => false,
    }
}

fn commit_single_engine_command(app: &mut UiApp, command_id: &str, params: Value) -> Result<UiIntentOutcome, String> {
    let mut single = PendingUiIntents::new();
    single.push_engine(command_id, params);
    let batch = single.commit_all_if_settled(app, false);
    match batch.results.into_iter().next() {
        Some(Ok(out)) => Ok(out),
        Some(Err(err)) => Err(err.to_string()),
        None => Err(format!("engine command `{command_id}` produced no outcome")),
    }
}

fn execute_host_import_at_path(app: &mut UiApp, command_id: &str, path: &Path, merge: bool) -> Result<Vec<UiIntentOutcome>, String> {
    let path_str = path.to_string_lossy().into_owned();
    let out = commit_single_engine_command(app, command_id, json!({ "path": path_str, "merge": merge }))?;
    if !merge {
        app.session_mut().set_file_path(None, false);
    }
    let source_label = path.file_name().and_then(|n| n.to_str()).unwrap_or(&path_str);
    if let UiIntentOutcome::Engine { outcome, .. } = &out {
        app.record_import_warnings(command_id, source_label, outcome);
    }
    Ok(vec![out])
}

fn sanitize_export_table_stem(title: &str, fallback_id: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_was_sep = false;
    for ch in title.trim().chars() {
        let is_bad = matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || ch.is_control();
        if is_bad {
            if !last_was_sep && !out.is_empty() {
                out.push('_');
                last_was_sep = true;
            }
        } else {
            out.push(ch);
            last_was_sep = false;
        }
    }
    let trimmed = out.trim_matches(|c: char| c.is_whitespace() || c == '.' || c == '_');
    if trimmed.is_empty() { fallback_id.to_owned() } else { trimmed.to_owned() }
}

#[derive(Clone, Copy)]
enum HostExportTargetKind {
    TableOrResult,
    Result,
}

fn resolve_export_sheet_id(doc: &ProjectDocument, kind: HostExportTargetKind, hint: Option<&str>) -> Option<SheetId> {
    if let Some(raw) = hint
        && let Ok(id) = SheetId::try_new(raw)
    {
        let valid = match kind {
            HostExportTargetKind::TableOrResult => doc.data_sheet(&id).is_some() || doc.result_sheet(&id).is_some(),
            HostExportTargetKind::Result => doc.result_sheet(&id).is_some(),
        };
        if valid {
            return Some(id);
        }
    }
    let active = doc.active_sheet_id.as_ref();
    match kind {
        HostExportTargetKind::TableOrResult => active
            .filter(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some())
            .cloned()
            .or_else(|| doc.data_sheets.first().map(|d| d.id.clone()))
            .or_else(|| doc.result_sheets.first().map(|r| r.id.clone())),
        HostExportTargetKind::Result => {
            active.filter(|id| doc.result_sheet(id).is_some()).cloned().or_else(|| doc.result_sheets.first().map(|r| r.id.clone()))
        }
    }
}

fn build_host_export_params(command: &str, target_sheet_id: &SheetId, path: &Path, path_str: &str, caller_params: &Value) -> Value {
    let mut map = serde_json::Map::new();
    if command == "result.generateReport" {
        let report_fmt = caller_params
            .get("format")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| matches!(*s, "plain" | "markdown" | "latex"))
            .unwrap_or_else(|| match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                Some("tex") => "latex",
                Some("txt") => "plain",
                _ => "markdown",
            });
        map.insert(String::from("result_sheet_id"), Value::String(target_sheet_id.as_str().to_owned()));
        map.insert(String::from("format"), Value::String(report_fmt.to_owned()));
        map.insert(String::from("path"), Value::String(path_str.to_owned()));
        return Value::Object(map);
    }

    let allowed: &[&str] = match command {
        "table.exportCsv" => &["delimiter", "decimal_separator"],
        "analysis.exportJson" => &["pretty"],
        "analysis.exportCsv" => &["tab_id", "tab_index", "delimiter", "decimal_separator"],
        _ => &[],
    };
    if let Some(obj) = caller_params.as_object() {
        for &key in allowed {
            if let Some(val) = obj.get(key)
                && !val.is_null()
            {
                map.insert(key.to_owned(), val.clone());
            }
        }
    }
    if matches!(command, "table.exportCsv" | "analysis.exportCsv")
        && path.extension().and_then(|e| e.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("tsv"))
    {
        map.insert(String::from("delimiter"), Value::String(String::from("\t")));
    }
    map.insert(String::from("sheet_id"), Value::String(target_sheet_id.as_str().to_owned()));
    map.insert(String::from("path"), Value::String(path_str.to_owned()));
    Value::Object(map)
}

fn execute_intercepted_host_file_command(
    app: &mut UiApp,
    adapter: &dyn PlatformFileDialogAdapter,
    command_id: &str,
    params: &Value,
) -> Result<Vec<UiIntentOutcome>, String> {
    match command_id {
        "table.importPrism" | "table.importPzfx" | "table.importXlsx" => {
            if let Some(explicit_path) = params.get("path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
                let merge = params.get("merge").and_then(Value::as_bool).unwrap_or(false);
                return execute_host_import_at_path(app, command_id, Path::new(explicit_path), merge);
            }
            let (filter_label, extensions): (String, Vec<String>) = match command_id {
                "table.importPrism" => (tr(&app.locale, "w6.view.shell.filter_archive").to_owned(), vec![String::from("prism")]),
                "table.importXlsx" => (tr(&app.locale, "cmd.table.importXlsx").to_owned(), vec![String::from("xlsx")]),
                _ => (tr(&app.locale, "w6.view.shell.filter_xml_project").to_owned(), vec![String::from("pzfx")]),
            };
            let req = NativeFileDialogRequest {
                command_id: command_id.to_owned(),
                kind: NativeFileDialogKind::OpenFile,
                filter_label,
                extensions,
                default_file_name: None,
            };
            let Some(path) = adapter.pick(&req)? else {
                return Ok(Vec::new());
            };
            if let Some(merge) = params.get("merge").and_then(Value::as_bool) {
                execute_host_import_at_path(app, command_id, &path, merge)
            } else if app.document().total_sheet_count() > 0 || app.session().is_dirty() {
                let has_unsaved = app.session().is_dirty();
                app.open_import_project_mode_dialog(command_id, path.to_string_lossy().into_owned(), has_unsaved);
                Ok(Vec::new())
            } else {
                execute_host_import_at_path(app, command_id, &path, false)
            }
        }
        "ui.export.allTablesCsv" => {
            let sheets: Vec<(SheetId, String)> = app.document().data_sheets.iter().map(|ds| (ds.id.clone(), ds.title.clone())).collect();
            if sheets.is_empty() {
                return Err(String::from("no data tables available to export"));
            }
            let req = NativeFileDialogRequest {
                command_id: String::from("ui.export.allTablesCsv"),
                kind: NativeFileDialogKind::PickFolder,
                filter_label: String::from("Export All Data Tables"),
                extensions: Vec::new(),
                default_file_name: None,
            };
            let Some(dir) = adapter.pick(&req)? else {
                return Ok(Vec::new());
            };
            std::fs::create_dir_all(&dir).map_err(|err| format!("failed to prepare export folder `{}`: {err}", dir.display()))?;
            let delimiter = params.get("delimiter").and_then(Value::as_str).unwrap_or(",").to_owned();
            let default_decimal = app.locale_preset.decimal_separator.to_string();
            let decimal_separator = params.get("decimal_separator").and_then(Value::as_str).map_or(default_decimal, ToOwned::to_owned);
            let ext = if delimiter == "\t" { "tsv" } else { "csv" };
            let mut used_stems: HashSet<String> = HashSet::new();
            let mut planned: Vec<(SheetId, PathBuf)> = Vec::with_capacity(sheets.len());
            for (sheet_id, title) in sheets {
                let base = sanitize_export_table_stem(&title, sheet_id.as_str());
                let stem = if used_stems.insert(base.to_ascii_lowercase()) {
                    base
                } else {
                    let mut suffix = 2usize;
                    loop {
                        let candidate = format!("{base}-{suffix}");
                        if used_stems.insert(candidate.to_ascii_lowercase()) {
                            break candidate;
                        }
                        suffix = suffix.saturating_add(1);
                    }
                };
                planned.push((sheet_id, dir.join(format!("{stem}.{ext}"))));
            }
            let mut outcomes = Vec::with_capacity(planned.len());
            for (sheet_id, file_path) in planned {
                let file_path_str = file_path.to_string_lossy().into_owned();
                let out = commit_single_engine_command(
                    app,
                    "table.exportCsv",
                    json!({
                        "sheet_id": sheet_id.as_str(),
                        "path": file_path_str,
                        "delimiter": delimiter,
                        "decimal_separator": decimal_separator,
                    }),
                )?;
                outcomes.push(out);
            }
            Ok(outcomes)
        }
        "table.exportCsv" | "analysis.exportCsv" | "analysis.exportJson" | "result.generateReport" => {
            let (filter_label, extensions, default_name, target_kind): (&str, &[&str], &str, HostExportTargetKind) = match command_id {
                "table.exportCsv" => ("CSV Table", &["csv", "tsv"], "table.csv", HostExportTargetKind::TableOrResult),
                "analysis.exportCsv" => ("CSV Results", &["csv", "tsv"], "analysis.csv", HostExportTargetKind::Result),
                "analysis.exportJson" => ("JSON Results", &["json"], "analysis.json", HostExportTargetKind::Result),
                _ => ("Methods Report", &["md", "txt", "tex"], "methods-report.md", HostExportTargetKind::Result),
            };
            let sheet_hint = params.get("sheet_id").or_else(|| params.get("result_sheet_id")).and_then(Value::as_str);
            let target_sheet_id = resolve_export_sheet_id(app.document(), target_kind, sheet_hint)
                .ok_or_else(|| format!("no matching sheet available for `{command_id}`"))?;
            let req = NativeFileDialogRequest {
                command_id: command_id.to_owned(),
                kind: NativeFileDialogKind::SaveFile,
                filter_label: filter_label.to_owned(),
                extensions: extensions.iter().map(|s| (*s).to_owned()).collect(),
                default_file_name: Some(default_name.to_owned()),
            };
            let Some(path) = adapter.pick(&req)? else {
                return Ok(Vec::new());
            };
            let path_str = path.to_string_lossy().into_owned();
            let export_params = build_host_export_params(command_id, &target_sheet_id, &path, &path_str, params);
            let out = commit_single_engine_command(app, command_id, export_params)?;
            Ok(vec![out])
        }
        _ => Err(format!("unhandled host file command `{command_id}`")),
    }
}
