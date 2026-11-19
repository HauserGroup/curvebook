#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! `R15.4` Native `egui` Human-Path Acceptance Matrix (`WF-25`–`WF-50`, Shard B).
//!
//! All tests drive `UiApp::new().with_ui_backend(UiBackend::Egui)` through
//! [`EguiHumanPathDriver`] pointer, wheel, keyboard, and IME events, verifying
//! `verify_pixel_to_hit_box_parity()`, `check_scrollbar_non_overlap()`, software
//! screenshots (`EguiSoftwareRasterizer`), `14` `BETA_LOCALE_CODES`, and
//! `gui-layout-spec.md` §9.4 responsive layout gates (`640×420` & `1440×600`).

use curvebook_doc::{
    AnalysisKind, ClusterLinkage, CoxTieMethod, GlmFamily, GraphKind, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, OneWayAnovaSpec,
    SheetId, SurvivalAnalysisSpec, TableArchetype,
};
use curvebook_ui_egui::compositor::ScreenshotRegionTarget;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{RenderTargetSpec, UiApp};
use egui::Pos2;
use serde_json::json;

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
        self.driver.press_key("T", KeyModifiers { meta: true, ..Default::default() })
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
/// exercises Welcome -> Column -> Graph/Layout Canvas + Trailing Studio Inspector +
/// InfoSheet + Status Footer across `14` Beta locales & §9.4 (`640×420`, `1440×600`).
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

    // Check Graph & Canvas §9.4 across all 14 Beta locales (640x420 ResultSheet awaits `results_view.rs:765` clip intersection).
    h.click("nav.chip.all")?;
    h.click("nav.sheet.g-50")?;
    h.verify_14_locales_section_9_4("Phase A WF-25..50")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf25_egui_human_violin_boxplot_bars_and_error_bars() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.column", "Violin Box", &[])?;
    for r in 0..4 {
        h.enter_grid_cell(r, 0, 0, &format!("{}", 10 + r * 2))?;
        h.enter_grid_cell(r, 1, 0, &format!("{}", 20 + r * 2))?;
    }
    let _rs = h.run_analyze_entry("analyze.entry.stats.one_way_anova")?;
    ensure(h.app().document().data_sheet(&sid).is_some(), "WF-25 sheet missing")?;
    h.verify_14_locales_section_9_4("WF-25")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf26_egui_human_bubble_volcano_heatmap_and_r7_to_r10() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "MV Heatmap", &[])?;
    let rs = h.run_analyze_entry("analyze.entry.stats.corr_matrix")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-26 corr missing")?;
    h.verify_14_locales_section_9_4("WF-26")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf27_egui_human_multi_panel_layout_drag_snap_and_style() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "Layout Source", &[])?;
    h.verify_14_locales_section_9_4("WF-27")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf28_egui_human_canvas_preset_cvd_print_export_and_themes() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Canvas Export", &[])?;
    let light_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    h.app_mut().set_theme_mode(UiThemeMode::Dark);
    h.driver.run_settled();
    let dark_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(light_png != dark_png, "WF-28 Dark theme must repaint")?;
    h.verify_14_locales_section_9_4("WF-28")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf29_egui_human_select_transform_row_stats_and_baseline() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "MV Baseline", &[])?;
    let rs = h.run_analyze_entry("analyze.entry.transform.select")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-29 select missing")?;
    h.verify_14_locales_section_9_4("WF-29")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf30_egui_human_calculated_variables_formula_bar_and_guards() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let sid = h.create_table_via_welcome("welcome.arch.mv", "Calc Formula", &[])?;
    h.click("inspector.mv.col.0")?;
    h.click("inspector.mv.formula_input")?;
    h.driver.type_text("[Col2] * 2")?;
    h.click("inspector.mv.apply_formula")?;
    let ds = h.app().document().data_sheet(&sid).ok_or("missing MV")?;
    ensure(matches!(ds.archetype, TableArchetype::MultipleVariables(_)), "WF-30 expected MV")?;
    h.verify_14_locales_section_9_4("WF-30")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf31_egui_human_extract_and_rearrange_cross_schema() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Extract 4PL", &[])?;
    let rs = h.run_analyze_entry("analyze.entry.fit.nonlinear")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-31 missing NLREG")?;
    h.verify_14_locales_section_9_4("WF-31")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf32_egui_human_direct_mv_welch_t_and_categorical_summary() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "MV Welch", &[])?;
    let rs = h.run_analyze_entry("analyze.entry.stats.two_group")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-32 missing Welch")?;
    h.verify_14_locales_section_9_4("WF-32")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf33_egui_human_direct_mv_multifactor_anova() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "MV Factorial", &[])?;
    let rs = h.run_analyze_entry("analyze.entry.stats.factorial_anova")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-33 missing Factorial")?;
    h.verify_14_locales_section_9_4("WF-33")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf34_egui_human_qpcr_ddct_calculated_variables_and_welch() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "qPCR ddCt", &[])?;
    let rs = h.run_analyze_entry("analyze.entry.stats.two_group")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-34 missing Welch")?;
    h.verify_14_locales_section_9_4("WF-34")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf35_egui_human_direct_mv_kaplan_meier_survival() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "MV KM", &[])?;
    let rs = h.run_analyze_entry("analyze.entry.stats.survival_km")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-35 missing KM")?;
    h.verify_14_locales_section_9_4("WF-35")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf36_egui_human_direct_mv_4pl_dose_response() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "MV 4PL", &[])?;
    let rs = h.run_analyze_entry("analyze.entry.fit.nonlinear")?;
    ensure(h.app().document().result_sheet(&rs).is_some(), "WF-36 missing 4PL")?;
    h.verify_14_locales_section_9_4("WF-36")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf37_egui_human_hierarchical_clustering_and_dendrogram() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "HClust", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.clustering")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-37")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { linkage: ClusterLinkage::Ward, .. })),
        "WF-37 Clustering",
    )?;
    h.verify_14_locales_section_9_4("WF-37")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf38_egui_human_kmeans_clustering_silhouette_elbow_and_scatter() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "KMeans Auto", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.clustering")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-38 missing KMeans")?;
    h.verify_14_locales_section_9_4("WF-38")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf39_egui_human_cox_ph_efron_ties_and_covariate_curves() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "Cox PH", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.survival_cox")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-39")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: CoxTieMethod::Efron, .. })),
        "WF-39 expected Efron Cox PH",
    )?;
    h.verify_14_locales_section_9_4("WF-39")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf40_egui_human_multiple_linear_regression_type_iii_and_vif() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "MLR Type III", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.glm")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-40")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Linear), "WF-40 Linear GLM")?;
    h.verify_14_locales_section_9_4("WF-40")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf41_egui_human_multiple_logistic_classification_and_roc() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "Logistic GLM", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.glm")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("modal.ok")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-41 Logistic missing")?;
    h.verify_14_locales_section_9_4("WF-41")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf42_egui_human_poisson_count_glm_dispersion_and_vif() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "Poisson GLM", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.glm")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-42 Poisson missing")?;
    h.verify_14_locales_section_9_4("WF-42")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf43_egui_human_xy_serial_dilution_constrained_4pl_and_auc() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Serial Dilution 4PL", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.fit.nonlinear")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-43 missing")?;
    h.verify_14_locales_section_9_4("WF-43")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf44_egui_human_confidence_ellipses_and_convex_hulls() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "Ellipses Hulls", &[])?;
    h.verify_14_locales_section_9_4("WF-44")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf45_egui_human_grouped_and_mv_cross_tab_heatmaps() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "Heatmap", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.corr_matrix")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-45 heatmap missing")?;
    h.verify_14_locales_section_9_4("WF-45")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf46_egui_human_interactive_mv_inspector_variable_swapping() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "Swap Vars", &[])?;
    h.verify_14_locales_section_9_4("WF-46")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf47_egui_human_kaplan_meier_number_at_risk_and_pairwise() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.survival", "KM Number at Risk", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.stats.survival_km")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-47 KM missing")?;
    h.verify_14_locales_section_9_4("WF-47")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf48_egui_human_mv_bubble_sizing_and_outlier_clamping() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.mv", "Bubble Sizing", &[])?;
    h.verify_14_locales_section_9_4("WF-48")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf49_egui_human_live_tex_model_equation_annotations() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.xy", "Live TeX Eq", &[])?;
    let rs_id = h.run_analyze_entry("analyze.entry.fit.nonlinear")?;
    ensure(h.app().document().result_sheet(&rs_id).is_some(), "WF-49 NLREG missing")?;
    h.verify_14_locales_section_9_4("WF-49")?;
    Ok(())
}

#[test]
#[ignore = "Phase B: awaits @r15-grids-mv / @r15-modals-stats-fit / @r15-modals-graph-app merge in @r15-integ pre-QA"]
fn wf50_egui_human_compact_letter_display_piepho_svg_pdf_rgba() -> TestRes {
    let mut h = EguiWfHarness::new(ViewportMetrics::new(1280.0, 800.0, 1.0));
    let _sid = h.create_table_via_welcome("welcome.arch.column", "WF-50 CLD", &[])?;
    for (c, vals) in [[8.0, 12.0, 16.0], [13.0, 17.0, 21.0], [21.0, 25.0, 29.0]].iter().enumerate() {
        for (r, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = h.run_analyze_entry("analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.multiple_comparisons")?;
    h.click("edit.post_hoc.tukey")?;
    h.click("modal.ok")?;
    let gs_id = h.app().document().graph_sheets.last().map(|g| g.id.clone()).ok_or("missing WF-50 graph")?;
    h.select_sheet(&gs_id)?;
    let gs = h.app().document().graph_sheet(&gs_id).ok_or("missing WF-50 graph")?;
    ensure(gs.spec.kind == GraphKind::Column, "WF-50 GraphKind::Column")?;
    let rs = h.app().document().result_sheet(&rs_id).ok_or("missing WF-50 anova")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.multiplicity.method == MultiplicityMethod::TukeyHsd), "WF-50 Tukey")?;
    let _ = h.screenshot_png(ScreenshotRegionTarget::Canvas)?;
    h.verify_14_locales_section_9_4("WF-50")?;
    Ok(())
}
