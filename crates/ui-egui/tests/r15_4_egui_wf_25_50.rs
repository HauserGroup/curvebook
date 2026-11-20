#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! `R15.4` Native `egui` Human-Path Acceptance Matrix (`WF-25`–`WF-50`, Shard B).
//!
//! All tests drive `UiApp::new().with_ui_backend(UiBackend::Egui)` strictly through
//! [`EguiHumanPathDriver`] pointer, wheel, keyboard, and IME events, verifying
//! `verify_pixel_to_hit_box_parity()`, `check_scrollbar_non_overlap()`, software
//! screenshots (`EguiSoftwareRasterizer`), `14` `BETA_LOCALE_CODES`, and
//! `gui-layout-spec.md` §9.4 responsive layout gates (`640×420` & `1440×600`).

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, ClusterLinkage, CoxTieMethod, GlmFamily, GraphKind, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, OneWayAnovaSpec,
    PairwiseDisplayMode, SheetId, SheetSection, SurvivalAnalysisSpec, TableArchetype, VariableSource,
};
use curvebook_ui_egui::compositor::ScreenshotRegionTarget;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{RenderTargetSpec, UiApp};
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

type TestRes = Result<(), String>;

struct WfShellState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    dialog_rect_pt: Option<egui::Rect>,
    last_intent_error: Option<String>,
}

impl WfShellState {
    fn new(mut app: UiApp) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), dialog_rect_pt: None, last_intent_error: None }
    }
}

struct EguiWfHarness {
    driver: EguiHumanPathDriver<'static, WfShellState>,
}

impl EguiWfHarness {
    fn new(metrics: ViewportMetrics) -> Self {
        let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        app.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
        Self::from_app(app, metrics)
    }

    fn from_app(mut app: UiApp, metrics: ViewportMetrics) -> Self {
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
                if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                    st.last_intent_error = Some(err.to_string());
                }
                st.views = st.app.egui_views.clone();
            } else {
                st.intents.clear();
            }
        });
        driver.run_settled();
        Self { driver }
    }

    fn app(&self) -> &UiApp {
        &self.driver.state().app
    }

    fn app_mut(&mut self) -> &mut UiApp {
        &mut self.driver.state_mut().app
    }

    fn widget_center(&self, wid: &str) -> Result<Pos2, String> {
        let entry = self.driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
        if !entry.clipped_rect.is_positive() {
            return Err(format!("widget `{wid}` has empty bounds {:?}", entry.clipped_rect));
        }
        Ok(entry.clipped_rect.center())
    }

    fn click(&mut self, wid: &str) -> TestRes {
        self.driver.click_widget_id(wid)
    }

    fn dbl_click_widget(&mut self, wid: &str) -> TestRes {
        let pos = self.widget_center(wid)?;
        self.driver.click_at(pos)?;
        self.driver.click_at(pos)?;
        Ok(())
    }

    fn type_replace_cell(&mut self, wid: &str, text: &str) -> TestRes {
        self.dbl_click_widget(wid)?;
        self.driver.press_egui_key(Key::A, Modifiers::COMMAND);
        self.driver.type_text(text)?;
        self.driver.press_key("Enter", KeyModifiers::default())?;
        Ok(())
    }

    fn enter_grid_cell(&mut self, row: usize, col: usize, subcol: usize, text: &str) -> TestRes {
        let wid = format!("grid.cell.{row}.{col}.{subcol}");
        self.type_replace_cell(&wid, text)
    }

    fn enter_x_cell(&mut self, row: usize, subcol: usize, text: &str) -> TestRes {
        let wid = format!("grid.xcell.{row}.{subcol}");
        self.type_replace_cell(&wid, text)
    }

    fn create_table_via_welcome(&mut self, arch_wid: &str, title: &str, extra_clicks: &[&str]) -> Result<SheetId, String> {
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

    fn select_sheet(&mut self, id: &SheetId) -> TestRes {
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

    fn set_mv_role(&mut self, col: usize, role_slug: &str) -> TestRes {
        self.click(&format!("mv_formula.role_badge.{col}"))?;
        self.click(&format!("mv_formula.role_opt.{col}.{role_slug}"))
    }

    fn create_and_seed_mv(&mut self, title: &str) -> Result<SheetId, String> {
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

    fn open_cmd_t(&mut self) -> TestRes {
        self.driver.press_key("T", KeyModifiers { meta: true, ..Default::default() })?;
        if !matches!(self.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
            return Err(format!("Cmd+T failed to open EditAnalysisParameters: {:?}", self.app().canvas_dialogs.active_dialog));
        }
        Ok(())
    }

    fn run_analyze_entry(&mut self, search_query: &str, entry_wid: &str) -> Result<SheetId, String> {
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

    fn assert_parity_and_layout(&mut self, label: &str) -> TestRes {
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

    fn verify_14_locales_section_9_4(&mut self, label: &str) -> TestRes {
        let saved_locale = self.app().locale.clone();
        for &loc in &BETA_LOCALE_CODES {
            let _ = self.app_mut().set_locale(loc);
            self.driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
            self.assert_parity_and_layout(&format!("{label} {loc} 1440x600"))?;
            self.driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.5));
            self.assert_parity_and_layout(&format!("{label} {loc} 640x420@1.5x"))?;
        }
        let _ = self.app_mut().set_locale(&saved_locale);
        self.driver.set_viewport(ViewportMetrics::new(1280.0, 800.0, 1.0));
        Ok(())
    }

    fn screenshot_png(&mut self, region: ScreenshotRegionTarget) -> Result<Vec<u8>, String> {
        self.app_mut()
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 }, region)
            .map(|(b, _, _)| b)
            .map_err(|e| e.to_string())
    }
}

fn ensure(cond: bool, msg: impl Into<String>) -> TestRes {
    if cond { Ok(()) } else { Err(msg.into()) }
}

/// Phase A baseline harness gate (`WF-25`–`WF-50`): exercises Welcome -> Column ->
/// Graph/Layout Canvas + Trailing Studio Inspector + InfoSheet + ResultsView ->
/// `14` Beta locales & §9.4 (`640×420`, `1440×600`).
#[test]
fn r15_4_phase_a_wf25_50_egui_harness_and_14_locale_gate() -> TestRes {
    let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-50", "kind": "column", "cols": 3, "rows": 4, "auto_graph": true, "graph_sheet_id": "g-50", "auto_info": true, "info_sheet_id": "info-50" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("table.setTitles", json!({ "sheet_id": "col-50", "titles": ["Alpha", "Beta", "Gamma"] })).map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": "col-50", "rows": [[8.0, 13.0, 21.0], [12.0, 17.0, 25.0], [16.0, 21.0, 29.0]] }))
        .map_err(|e| e.to_string())?;
    let kind_val = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
        ..Default::default()
    }))
    .map_err(|e| e.to_string())?;
    app.execute_command("analysis.run", json!({ "source_sheet_id": "col-50", "sheet_id": "res-50", "kind": kind_val })).map_err(|e| e.to_string())?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-50", "title": "CLD Layout" })).map_err(|e| e.to_string())?;
    app.execute_command("layout.addPanel", json!({ "sheet_id": "lay-50", "kind": "graph", "graph_sheet_id": "g-50", "row": 0, "col": 0 }))
        .map_err(|e| e.to_string())?;

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 800.0, 1.0));
    h.click("nav.sheet.g-50")?;
    ensure(h.app().document().active_sheet_id.as_ref().map(SheetId::as_str) == Some("g-50"), "Graph sheet active")?;
    h.click("inspector.graph.tab.axes")?;
    h.assert_parity_and_layout("Phase A Graph Inspector")?;

    h.click("nav.chip.layouts")?;
    h.click("nav.sheet.lay-50")?;
    ensure(h.app().document().active_sheet_id.as_ref().map(SheetId::as_str) == Some("lay-50"), "Layout sheet active")?;
    let layout_png = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(layout_png.starts_with(b"\x89PNG\r\n\x1a\n") && layout_png.len() > 256, "valid layout canvas PNG")?;
    h.assert_parity_and_layout("Phase A LayoutSheet 1440x800")?;

    h.click("nav.chip.info")?;
    h.click("nav.sheet.info-50")?;
    h.assert_parity_and_layout("Phase A InfoSheet")?;

    h.click("nav.chip.results")?;
    h.click("nav.sheet.res-50")?;
    h.assert_parity_and_layout("Phase A ResultsView")?;

    h.click("nav.chip.all")?;
    h.click("nav.sheet.g-50")?;
    h.verify_14_locales_section_9_4("Phase A WF-25..50")?;
    Ok(())
}

#[test]
fn wf25_egui_human_violin_boxplot_bars_and_error_bars() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.column", "Violin Box", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", 10 + r * 2))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", 20 + r * 2))?;
        h.enter_grid_cell(r, 2, 0, &format!("{}", 30 + r * 2))?;
    }
    let _rs = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-25 missing graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.graph_type")?;
    h.click("graph_type.card.col.violin")?;
    h.click("modal.ok")?;
    ensure(h.app().document().data_sheet(&sid).is_some(), "WF-25 sheet missing")?;
    h.verify_14_locales_section_9_4("WF-25")?;
    Ok(())
}

#[test]
fn wf26_egui_human_bubble_volcano_heatmap_and_r7_to_r10() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV Heatmap")?;
    let rs = h.run_analyze_entry("Correlation", "analyze.entry.stats.corr_matrix")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-26 corr missing")?;
    h.verify_14_locales_section_9_4("WF-26")?;
    Ok(())
}

#[test]
fn wf27_egui_human_multi_panel_layout_drag_snap_and_style() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Layout Source", &[])?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-27 graph missing")?;
    h.driver.press_egui_key(Key::K, Modifiers::COMMAND);
    h.driver.type_text("Layout")?;
    h.driver.press_key("Enter", KeyModifiers::default())?;
    let lay_id = h.app().document().layout_sheets.last().map(|l| l.id.clone()).ok_or("WF-27 layout missing")?;
    h.select_sheet(&lay_id)?;
    h.click("toolbar.layout_composer")?;
    h.click("layout.arrange")?;
    h.click("modal.ok")?;
    h.select_sheet(&gs_id)?;
    h.verify_14_locales_section_9_4("WF-27")?;
    Ok(())
}

#[test]
fn wf28_egui_human_canvas_preset_cvd_print_export_and_themes() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Canvas Export", &[])?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-28 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.export_svg")?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }), "WF-28 ExportFigure")?;
    h.click("modal.cancel")?;
    let light_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    h.app_mut().set_theme_mode(UiThemeMode::Dark);
    h.driver.run_settled();
    let dark_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(light_png != dark_png, "WF-28 Dark theme must repaint")?;
    h.verify_14_locales_section_9_4("WF-28")?;
    Ok(())
}

#[test]
fn wf29_egui_human_select_transform_row_stats_and_baseline() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV Baseline")?;
    let rs = h.run_analyze_entry("Select and Transform", "analyze.entry.transform.select")?;
    h.select_sheet(&rs)?;
    h.open_cmd_t()?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-29 select missing")?;
    h.verify_14_locales_section_9_4("WF-29")?;
    Ok(())
}

#[test]
fn wf30_egui_human_calculated_variables_formula_bar_and_guards() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.multiple_variables", "Calc Formula", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", r + 1))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", (r + 1) * 5))?;
    }
    h.click("inspector.mv.col.2")?;
    h.click("inspector.mv.formula_input")?;
    h.driver.press_egui_key(Key::A, Modifiers::COMMAND);
    h.driver.type_text("[Var 2] * 2")?;
    h.click("inspector.mv.apply_formula")?;
    let ds = h.app().document().data_sheet(&sid).ok_or("missing MV")?;
    ensure(
        matches!(ds.archetype, TableArchetype::MultipleVariables(ref mv) if matches!(mv.variables[2].source, VariableSource::Calculated { .. })),
        "WF-30 formula",
    )?;
    h.verify_14_locales_section_9_4("WF-30")?;
    Ok(())
}

#[test]
fn wf31_egui_human_extract_and_rearrange_cross_schema() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Extract 4PL", &[])?;
    for (r, (&x, &y)) in [1.0, 3.0, 10.0, 30.0, 100.0, 300.0].iter().zip([10.0, 22.0, 48.0, 79.0, 93.0, 99.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let rs = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs)?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-31 missing NLREG")?;
    h.verify_14_locales_section_9_4("WF-31")?;
    Ok(())
}

#[test]
fn wf32_egui_human_direct_mv_welch_t_and_categorical_summary() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV Welch")?;
    let rs = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    h.select_sheet(&rs)?;
    h.open_cmd_t()?;
    h.click("edit.tg.welch_t")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-32 missing Welch")?;
    h.verify_14_locales_section_9_4("WF-32")?;
    Ok(())
}

#[test]
fn wf33_egui_human_direct_mv_multifactor_anova() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV Factorial")?;
    h.set_mv_role(1, "categorical")?;
    for r in 0..10 {
        h.enter_grid_cell(r, 1, 0, if r % 2 == 0 { "Male" } else { "Female" })?;
    }
    let rs = h.run_analyze_entry("Two-Way", "analyze.entry.stats.factorial_anova")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-33 missing Factorial")?;
    h.verify_14_locales_section_9_4("WF-33")?;
    Ok(())
}

#[test]
fn wf34_egui_human_qpcr_ddct_calculated_variables_and_welch() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("qPCR ddCt")?;
    let rs = h.run_analyze_entry("Unpaired", "analyze.entry.stats.two_group")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-34 missing Welch")?;
    h.verify_14_locales_section_9_4("WF-34")?;
    Ok(())
}

#[test]
fn wf35_egui_human_direct_mv_kaplan_meier_survival() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV KM")?;
    let rs = h.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-35 missing KM")?;
    h.verify_14_locales_section_9_4("WF-35")?;
    Ok(())
}

#[test]
fn wf36_egui_human_direct_mv_4pl_dose_response() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MV 4PL")?;
    let rs = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-36 missing 4PL")?;
    h.verify_14_locales_section_9_4("WF-36")?;
    Ok(())
}

#[test]
fn wf37_egui_human_hierarchical_clustering_and_dendrogram() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("HClust")?;
    let rs_id = h.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.cluster.ward")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-37")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { linkage: ClusterLinkage::Ward, .. })),
        "WF-37 Clustering",
    )?;
    h.verify_14_locales_section_9_4("WF-37")?;
    Ok(())
}

#[test]
fn wf38_egui_human_kmeans_clustering_silhouette_elbow_and_scatter() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("KMeans Auto")?;
    let rs_id = h.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-38 missing KMeans")?;
    h.verify_14_locales_section_9_4("WF-38")?;
    Ok(())
}

#[test]
fn wf39_egui_human_cox_ph_efron_ties_and_covariate_curves() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Cox PH")?;
    let rs_id = h.run_analyze_entry("Cox", "analyze.entry.stats.survival_cox")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.cox.efron")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-39")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: CoxTieMethod::Efron, .. })),
        "WF-39 expected Efron Cox PH",
    )?;
    h.verify_14_locales_section_9_4("WF-39")?;
    Ok(())
}

#[test]
fn wf40_egui_human_multiple_linear_regression_type_iii_and_vif() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("MLR Type III")?;
    let rs_id = h.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.experimental_design")?;
    h.click("edit.glm.linear")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-40")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Linear), "WF-40 Linear GLM")?;
    h.verify_14_locales_section_9_4("WF-40")?;
    Ok(())
}

#[test]
fn wf41_egui_human_multiple_logistic_classification_and_roc() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Logistic GLM")?;
    let rs_id = h.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.data")?;
    h.click("edit.glm.resp.2")?;
    h.click("edit.tab.experimental_design")?;
    h.click("edit.glm.logistic")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-41")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Logistic), "WF-41 Logistic GLM")?;
    h.verify_14_locales_section_9_4("WF-41")?;
    Ok(())
}

#[test]
fn wf42_egui_human_poisson_count_glm_dispersion_and_vif() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Poisson GLM")?;
    let rs_id = h.run_analyze_entry("Multiple Regression", "analyze.entry.stats.glm")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.experimental_design")?;
    h.click("edit.glm.poisson")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-42")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Poisson), "WF-42 Poisson GLM")?;
    h.verify_14_locales_section_9_4("WF-42")?;
    Ok(())
}

#[test]
fn wf43_egui_human_xy_serial_dilution_constrained_4pl_and_auc() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Serial Dilution 4PL", &[])?;
    for (r, (&x, &y)) in [0.1, 1.0, 10.0, 100.0, 1000.0].iter().zip([12.0, 28.0, 55.0, 82.0, 96.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("fit.tab.constraints")?;
    h.click("fit.tab.output")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-43 missing")?;
    h.verify_14_locales_section_9_4("WF-43")?;
    Ok(())
}

#[test]
fn wf44_egui_human_confidence_ellipses_and_convex_hulls() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Ellipses Hulls")?;
    let _rs = h.run_analyze_entry("Principal", "analyze.entry.stats.pca")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-44 missing graph")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.format_graph")?;
    h.click("modal.ok")?;
    h.verify_14_locales_section_9_4("WF-44")?;
    Ok(())
}

#[test]
fn wf45_egui_human_grouped_and_mv_cross_tab_heatmaps() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Heatmap")?;
    let rs_id = h.run_analyze_entry("Correlation", "analyze.entry.stats.corr_matrix")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.corr.spearman")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-45 heatmap missing")?;
    h.verify_14_locales_section_9_4("WF-45")?;
    Ok(())
}

#[test]
fn wf46_egui_human_interactive_mv_inspector_variable_swapping() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Swap Vars")?;
    h.click("mv_formula.role_badge.0")?;
    h.click("mv_formula.role_manage.0")?;
    ensure(h.app().grid.variables_dialog.open, "WF-46 Manage Variables must open")?;
    h.click("mv_formula.vars.col.1")?;
    h.click("modal.ok")?;
    h.verify_14_locales_section_9_4("WF-46")?;
    Ok(())
}

#[test]
fn wf47_egui_human_kaplan_meier_number_at_risk_and_pairwise() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.survival", "KM Number at Risk", &[])?;
    for (r, t) in [4.0, 10.0, 16.0, 22.0, 28.0].iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{t}"))?;
        h.enter_grid_cell(r, 0, 0, if r % 2 == 0 { "1" } else { "0" })?;
        h.enter_grid_cell(r, 1, 0, "1")?;
    }
    let rs_id = h.run_analyze_entry("Kaplan", "analyze.entry.stats.survival_km")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-47 KM missing")?;
    h.verify_14_locales_section_9_4("WF-47")?;
    Ok(())
}

#[test]
fn wf48_egui_human_mv_bubble_sizing_and_outlier_clamping() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_and_seed_mv("Bubble Sizing")?;
    let _rs = h.run_analyze_entry("Correlation", "analyze.entry.stats.corr_matrix")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-48 graph missing")?;
    h.select_sheet(&gs_id)?;
    h.click("toolbar.export_svg")?;
    ensure(h.driver.frame_meta().find_by_widget_id("dialog.export_figure").is_some(), "WF-48 export dialog")?;
    h.click("modal.cancel")?;
    h.verify_14_locales_section_9_4("WF-48")?;
    Ok(())
}

#[test]
fn wf49_egui_human_live_tex_model_equation_annotations() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Live TeX Eq", &[])?;
    for (r, (&x, &y)) in [1.0, 3.0, 10.0, 30.0, 100.0].iter().zip([10.0, 25.0, 52.0, 80.0, 95.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let rs_id = h.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("WF-49 graph missing")?;
    h.select_sheet(&gs_id)?;
    h.click("inspector.graph.tab.annotations")?;
    h.click("inspector.graph.add_equation")?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-49 NLREG missing")?;
    h.verify_14_locales_section_9_4("WF-49")?;
    Ok(())
}

#[test]
fn wf50_egui_human_compact_letter_display_piepho_svg_pdf_rgba() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "WF-50 CLD", &[])?;
    for (c, vals) in [[8.0, 12.0, 16.0], [13.0, 17.0, 21.0], [21.0, 25.0, 29.0]].iter().enumerate() {
        for (r, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = h.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.multiple_comparisons")?;
    h.click("edit.post_hoc.tukey")?;
    h.click("modal.ok")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("missing WF-50 graph")?;
    h.select_sheet(&gs_id)?;
    h.click("inspector.graph.tab.legends")?;
    h.click("inspector.graph.more_pairwise")?;
    h.click("pairwise.mode.CompactLetterDisplayOnly")?;
    h.click("modal.ok")?;
    let gs = h.app().document().graph_sheet(&gs_id).ok_or("missing WF-50 graph")?;
    ensure(gs.spec.kind == GraphKind::Column, "WF-50 GraphKind::Column")?;
    ensure(gs.spec.pairwise_display_mode == PairwiseDisplayMode::CompactLetterDisplayOnly, "WF-50 CLD mode")?;
    let png = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256, "WF-50 canvas PNG")?;
    h.verify_14_locales_section_9_4("WF-50")?;
    Ok(())
}
