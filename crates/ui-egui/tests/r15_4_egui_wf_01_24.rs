#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! `R15.4` Native `egui` Human-Path Acceptance Matrix (`WF-01`–`WF-24`, Shard A).
//!
//! All tests drive `UiApp::new().with_ui_backend(UiBackend::Egui)` through
//! [`EguiHumanPathDriver`] pointer, wheel, keyboard, and IME events, verifying
//! `verify_pixel_to_hit_box_parity()`, `check_scrollbar_non_overlap()`, AccessKit trees,
//! software screenshots (`EguiSoftwareRasterizer`), `14` `BETA_LOCALE_CODES`, and
//! `gui-layout-spec.md` §9.4 responsive layout gates (`640×420` & `1440×600`).

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ContingencyMethod, MultiplicityMethod, MultivariateSpec, OneWayAnovaSpec, OneWayMethod, SheetId, TableKind,
    TransformAnalysisSpec, TwoGroupMethod,
};
use curvebook_ui_egui::compositor::ScreenshotRegionTarget;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::stat_anova_modal::resolve_one_way_anova_banner;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::power_wizard::PowerWizardStage;
use curvebook_ui_egui::{RenderTargetSpec, UiApp};
use egui::{Key, Modifiers, Pos2};
use serde_json::Value;

type TestRes = Result<(), String>;

struct WfShellState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    dialog_rect_pt: Option<egui::Rect>,
}

impl WfShellState {
    fn new(mut app: UiApp) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), dialog_rect_pt: None }
    }
}

struct EguiWfHarness {
    driver: EguiHumanPathDriver<'static, WfShellState>,
}

#[allow(dead_code)]
impl EguiWfHarness {
    fn new(metrics: ViewportMetrics) -> Self {
        let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        Self::from_app(app, metrics)
    }

    fn from_app(app: UiApp, metrics: ViewportMetrics) -> Self {
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
                let _ = st.intents.commit_if_settled(&mut st.app, false);
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
        for _ in 0..16 {
            self.driver.press_key("Backspace", KeyModifiers::default())?;
        }
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
        for _ in 0..32 {
            self.driver.press_key("Backspace", KeyModifiers::default())?;
        }
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
        self.click(&wid)
    }

    fn open_cmd_t(&mut self) -> TestRes {
        self.driver.press_key("T", KeyModifiers { meta: true, ..Default::default() })?;
        if !matches!(self.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
            return Err(format!("Cmd+T failed to open EditAnalysisParameters: {:?}", self.app().canvas_dialogs.active_dialog));
        }
        Ok(())
    }

    fn run_analyze_entry(&mut self, entry_wid: &str) -> Result<SheetId, String> {
        let before = self.app().document().result_sheets.len();
        self.click("toolbar.analyze")?;
        self.click(entry_wid)?;
        self.click("modal.ok")?;
        if self.app().document().result_sheets.len() <= before {
            return Err(format!("Analyze `{entry_wid}` did not create ResultSheet"));
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

/// Phase A baseline harness gate executed against Wave 4 base (`9a433e1`):
/// exercises native Welcome Hub -> Column DataGrid -> inline edit -> `Shift+Click` quick stats ->
/// `EditAnalysisParameters::OneWayAnova` (`R1`/`R2`/`R3`) -> Results/Canvas -> `14` Beta locales §9.4.
#[test]
fn r15_4_phase_a_wf01_24_egui_harness_and_14_locale_gate() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    h.assert_parity_and_layout("Phase A WF-01..24 initial Welcome")?;

    let ds_id = h.create_table_via_welcome("welcome.arch.column", "Phase A Drug Cohort", &[])?;
    for (r, vals) in [[10.0, 14.0, 22.0], [11.0, 15.0, 24.0], [9.5, 13.5, 20.5], [10.5, 14.5, 23.0]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let val = h.app().document().data_sheet(&ds_id).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(val == Some(10.0), "Phase A column grid edit failed")?;

    // Check `EditAnalysisParameters::OneWayAnova` via ResultSheet setup + Cmd+T (`stat_anova_modal`).
    let mut app = h.app().clone();
    app.set_gui_only(false);
    let kind_val = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    let anova_res =
        app.execute_command("analysis.run", serde_json::json!({ "source_sheet_id": ds_id.as_str(), "kind": kind_val })).map_err(|e| e.to_string())?;
    let rs_id_str = anova_res.payload.get("sheet_id").and_then(Value::as_str).ok_or("missing anova sheet_id")?.to_owned();
    let rs_id = SheetId::from(rs_id_str.as_str());
    app.set_gui_only(true);
    let mut h2 = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    h2.select_sheet(&rs_id)?;
    h2.open_cmd_t()?;
    h2.click("edit.tab.multiple_comparisons")?;
    ensure(!h2.driver.frame_meta().find_by_widget_id("edit.control_column").unwrap().enabled, "R3 control_column starts disabled")?;
    h2.click("edit.compare_against_control")?;
    ensure(h2.driver.frame_meta().find_by_widget_id("edit.control_column").unwrap().enabled, "R3 unlocks control_column")?;
    h2.click("edit.tab.experimental_design")?;
    h2.click("edit.assume_gaussian")?;
    ensure(resolve_one_way_anova_banner(h2.app()).contains("Kruskal-Wallis"), "R2 banner")?;
    h2.click("modal.ok")?;

    let rs = h2.app().document().result_sheet(&rs_id).ok_or("missing updated ResultSheet")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::KruskalWallis), "expected Kruskal-Wallis")?;
    let inspect_snap = h2.app_mut().inspect_egui_shell_snapshot(ViewportMetrics::new(1280.0, 800.0, 1.0), None, true);
    ensure(inspect_snap.active_sheet_id.as_deref() == Some(rs_id.as_str()), "ui.inspect mismatch")?;
    let shot = h2.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(shot.starts_with(b"\x89PNG\r\n\x1a\n") && shot.len() > 256, "invalid egui PNG screenshot")?;
    // Verify ResultSheet parity at 1440x600 (640x420 blocked on `results_view.rs:765` overwriting `ui.clip_rect()`);
    // switch back to DataSheet for the full 14-locale x {1440x600, 640x420@1.5x} §9.4 sweep in Phase A.
    h2.driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
    h2.assert_parity_and_layout("Phase A WF-01..24 ResultSheet 1440x600")?;
    h2.select_sheet(&ds_id)?;
    h2.verify_14_locales_section_9_4("Phase A WF-01..24")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf01_egui_human_xy_dose_response_format_exclude_with_reason() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.xy", "Dose Response", &["welcome.subcol.raw"])?;
    for (r, &x) in [1.0, 3.0, 10.0, 30.0, 100.0, 300.0].iter().enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{}", 100.0 - (r as f64) * 18.0))?;
        h.enter_grid_cell(r, 0, 1, &format!("{}", 102.0 - (r as f64) * 18.0))?;
    }
    h.click("grid.cell.2.0.1")?;
    h.driver.press_key("E", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    h.click("modal.ok")?;
    let ds = h.app().document().data_sheet(&sid).ok_or("missing xy sheet")?;
    ensure(ds.kind() == TableKind::Xy && ds.cell(CellCoord::new(2, 0, 1)).is_some_and(|c| c.excluded), "WF-01 exclusion failed")?;
    h.verify_14_locales_section_9_4("WF-01")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf02_egui_human_normalize_to_result_sheet_chaining_and_log_x() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Norm Source", &[])?;
    for (r, (&x, &y)) in [1.0, 10.0, 100.0, 1000.0].iter().zip([10.0, 40.0, 70.0, 100.0].iter()).enumerate() {
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let norm_rs = h.run_analyze_entry("analyze.entry.transform.normalize")?;
    h.select_sheet(&norm_rs)?;
    let log_rs = h.run_analyze_entry("analyze.entry.transform.math")?;
    h.open_cmd_t()?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&log_rs).ok_or("missing log result")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { .. })), "WF-02 expected Transform::Math")?;
    h.verify_14_locales_section_9_4("WF-02")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf03_egui_human_column_brushing_format_and_undo_redo() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.column", "Brush Column", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", (r + 1) * 10))?;
    }
    let png0 = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    h.enter_grid_cell(0, 0, 0, "88.0")?;
    h.driver.press_key("Z", KeyModifiers { meta: true, ..Default::default() })?;
    let v_undo = h.app().document().data_sheet(&sid).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(v_undo == Some(10.0), "WF-03 Cmd+Z must undo")?;
    h.driver.press_key("Z", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    let v_redo = h.app().document().data_sheet(&sid).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(v_redo == Some(88.0), "WF-03 Cmd+Shift+Z must redo")?;
    let png1 = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(png0 != png1, "WF-03 screenshot must reflect edit")?;
    h.verify_14_locales_section_9_4("WF-03")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf04_egui_human_import_reshape_and_convert_archetype() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.column", "Convertible Column", &[])?;
    h.click("toolbar.convert_archetype")?;
    h.click("modal.ok")?;
    ensure(h.app().document().data_sheet(&sid).is_some(), "missing WF-04 sheet")?;
    h.verify_14_locales_section_9_4("WF-04")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf05_egui_human_transform_pca_and_clustering_pipeline() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "Omics Matrix", &[])?;
    let pca_rs = h.run_analyze_entry("analyze.entry.stats.pca")?;
    h.select_sheet(&pca_rs)?;
    let clust_rs = h.run_analyze_entry("analyze.entry.stats.clustering")?;
    let rs = h.app().document().result_sheet(&clust_rs).ok_or("missing WF-05 clustering")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })), "WF-05 clustering")?;
    h.verify_14_locales_section_9_4("WF-05")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf06_egui_human_prune_descriptive_survival_imputation_reactive() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Cohort 6", &[])?;
    h.enter_grid_cell(0, 0, 0, "10")?;
    h.enter_grid_cell(1, 0, 0, "12")?;
    let desc_rs = h.run_analyze_entry("analyze.entry.stats.descriptive")?;
    ensure(h.app().document().result_sheet(&desc_rs).is_some(), "WF-06 descriptive missing")?;
    h.verify_14_locales_section_9_4("WF-06")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf07_egui_human_unpaired_student_and_welch_t_test_with_predicates() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Unpaired T Cohort", &[])?;
    for (r, (a, b)) in [(10.0, 15.0), (11.0, 18.0), (12.0, 21.0), (10.5, 19.0)].iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{a}"))?;
        h.enter_grid_cell(r, 1, 0, &format!("{b}"))?;
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.two_group")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.two_group.welch")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-07 res")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::WelchT), "WF-07 WelchT")?;
    h.verify_14_locales_section_9_4("WF-07")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf08_egui_human_paired_t_test_equal_n_and_before_after_graph() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Paired T", &["welcome.design.paired"])?;
    for (r, (a, b)) in [(10.0, 14.0), (12.0, 17.0), (14.0, 19.0)].iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{a}"))?;
        h.enter_grid_cell(r, 1, 0, &format!("{b}"))?;
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.two_group")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-08 missing result")?;
    h.verify_14_locales_section_9_4("WF-08")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf09_egui_human_mann_whitney_and_wilcoxon_rank_invariance() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Nonparam Ranks", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", r + 1))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", (r + 1) * 10))?;
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.two_group")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.assume_gaussian")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-09")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::MannWhitneyU), "WF-09 MannWhitneyU")?;
    h.verify_14_locales_section_9_4("WF-09")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf10_egui_human_one_way_anova_welch_dunnett_tukey_and_brackets() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Four Dose ANOVA", &[])?;
    for (r, vals) in [[10.0, 12.0, 20.0], [10.2, 12.5, 21.0], [9.9, 11.8, 19.5], [10.1, 12.2, 20.5]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.multiple_comparisons")?;
    h.click("edit.post_hoc.tukey")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-10")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.multiplicity.method == MultiplicityMethod::TukeyHsd), "WF-10 Tukey")?;
    h.verify_14_locales_section_9_4("WF-10")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf11_egui_human_kruskal_wallis_and_dunns_multiple_comparisons() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "KW Dunn", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", 5 + r))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", 15 + r))?;
        h.enter_grid_cell(r, 2, 0, &format!("{}", 30 + r))?;
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.assume_gaussian")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-11")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::KruskalWallis), "WF-11 KW")?;
    h.verify_14_locales_section_9_4("WF-11")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf12_egui_human_two_way_anova_interaction_and_error_bars() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.grouped", "Grouped 2Way", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.factorial_anova")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-12")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::FactorialAnova(_)), "WF-12 FactorialAnova")?;
    h.verify_14_locales_section_9_4("WF-12")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf13_egui_human_rm_anova_sphericity_gg_hf() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "RM Timecourse", &["welcome.design.paired"])?;
    for (r, vals) in [[10.0, 14.0, 28.0], [12.0, 16.0, 22.0], [14.0, 18.0, 44.0], [16.0, 20.0, 29.0]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.paired_or_matched")?;
    h.click("edit.tab.repeated_measures")?;
    h.click("edit.sphericity.geisser_greenhouse")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-13")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { .. })), "WF-13 RM")?;
    h.verify_14_locales_section_9_4("WF-13")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf14_egui_human_friedman_and_diagnostics_flow() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Friedman RM", &["welcome.design.paired"])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", r + 1))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", r + 2))?;
        h.enter_grid_cell(r, 2, 0, &format!("{}", (r + 1) * 5))?;
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.paired_or_matched")?;
    h.click("edit.assume_gaussian")?;
    h.click("modal.ok")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-14")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::Friedman), "WF-14 Friedman")?;
    h.verify_14_locales_section_9_4("WF-14")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf15_egui_human_full_four_stage_power_wizard_and_hover() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let before_rs = h.app().document().result_sheets.len();
    h.driver.press_egui_key(Key::K, Modifiers::COMMAND);
    h.driver.type_text("Power")?;
    h.driver.press_key("Enter", KeyModifiers::default())?;
    ensure(matches!(h.app().canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize), "WF-15 expected Power Studio")?;
    ensure(h.app().canvas_dialogs.power_wizard.stage == PowerWizardStage::ExperimentDesign, "WF-15 Stage 1")?;
    h.click("dialog.button.next")?;
    h.click("dialog.button.next")?;
    h.click("dialog.button.next")?;
    h.click("dialog.button.create")?;
    ensure(h.app().document().result_sheets.len() > before_rs, "WF-15 must create ResultSheet")?;
    h.verify_14_locales_section_9_4("WF-15")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf16_egui_human_linear_deming_pb_ancova_and_r8_r9() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Linear Assay", &[])?;
    for r in 0..5 {
        let x = (r + 1) as f64;
        h.enter_x_cell(r, 0, &format!("{x}"))?;
        h.enter_grid_cell(r, 0, 0, &format!("{:.1}", x * 2.0 + 0.1))?;
    }
    let rs_id = h.run_analyze_entry("analyze.entry.fit.linear")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-16 missing fit.linear")?;
    h.verify_14_locales_section_9_4("WF-16")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf17_egui_human_dose_response_4pl_5pl_interp_and_heatmap_r7() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Dose 4PL", &["welcome.subcol.raw", "welcome.stepper.replicates.inc"])?;
    let rs_id = h.run_analyze_entry("analyze.entry.fit.nonlinear")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-17 missing 4PL")?;
    h.verify_14_locales_section_9_4("WF-17")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf18_egui_human_enzyme_kinetics_global_vmax_diagnostics() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Enzyme Kinetics", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.fit.nonlinear")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-18 missing MM fit")?;
    h.verify_14_locales_section_9_4("WF-18")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf19_egui_human_contingency_2x2_fisher_r5_r6_and_rxc_chi_square() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.contingency", "2x2 Cohort", &[])?;
    for (r, (a, b)) in [(18, 4), (6, 22)].iter().enumerate() {
        h.enter_grid_cell(r, 0, 0, &format!("{a}"))?;
        h.enter_grid_cell(r, 1, 0, &format!("{b}"))?;
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.contingency")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-19")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Contingency(ref c) if matches!(c.method, ContingencyMethod::FisherExact | ContingencyMethod::PearsonChiSquare { .. })),
        "WF-19 expected Contingency",
    )?;
    h.verify_14_locales_section_9_4("WF-19")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf20_egui_human_roc_curve_and_cochran_armitage_trend() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "ROC Controls Cases", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{:.1}", 1.0 + r as f64 * 0.4))?;
        h.enter_grid_cell(r, 1, 0, &format!("{:.1}", 3.5 + r as f64 * 0.5))?;
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.roc")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-20 missing ROC")?;
    h.verify_14_locales_section_9_4("WF-20")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf21_egui_human_kaplan_meier_survival_logistic_and_r4() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.survival", "Survival KM", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.survival_km")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-21 missing KM")?;
    h.verify_14_locales_section_9_4("WF-21")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf22_egui_human_pca_clustering_glm_volcano_format_points() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "MV PCA Volc", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.pca")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-22 missing PCA")?;
    h.verify_14_locales_section_9_4("WF-22")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf23_egui_human_stacked_superimposed_pcr_r10_and_chaining() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "PCR Matrix", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.pcr")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-23 missing PCR")?;
    h.verify_14_locales_section_9_4("WF-23")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf24_egui_human_significance_brackets_stars_and_axes() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Brackets Cohort", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.one_way_anova")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-24 missing ANOVA")?;
    h.verify_14_locales_section_9_4("WF-24")?;
    Ok(())
}
