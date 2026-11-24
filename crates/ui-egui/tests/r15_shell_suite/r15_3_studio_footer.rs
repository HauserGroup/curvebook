#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R15.3` Trailing 7-Context Studio Rail (`280 pt -> 36 pt`) & `26 pt` Status & Telemetry Footer
//! integration tests:
//! - Deterministic auto-switching across all 7 [`StudioContextSurface`] modes
//! - MV formula operand column outline highlights in sync with active formula + role/levels
//! - Table/Selection decimals, row-subset graph exclusion, linked provenance, archetype escalation
//! - `InfoSheet` typed constant editor + computed `Referenced By` downstream jump list
//! - `ResultSheet` `Assumptions` (severity filter), diagnostics-only `Guidance`, `Methods` + `Copy Methods`
//! - `GraphSheet` 5-tab Inspector (`Data`, `Axes`, `Legends`, `Annotations`, `Canvas`) + `More…` triggers
//! - `DataPointDrillBack` exact datum readout, exclude toggle, `Jump to Datasheet Cell`, and `← Back`
//! - `PageLayoutComposer` `85 mm` / `114 mm` / `174 mm` presets, margins, DPI, snap, alignment
//! - `Cmd/Ctrl+Shift+I` toggle & responsive breakpoints (`< 960 pt` collapses Trailing before `< 760 pt` Leading)
//! - `26 pt` Footer: `< >` history, section pills, sheet switcher, `LinkedDataRegion` indicator,
//!   Quick-Stats (`N`, `Mean`, `SD`, `SEM`, `Median`, `Range`, `50,000` cells `< 0.5 ms` + `O(1)` cache),
//!   live `(X, Y)` cursor readout on `ChartScene`/`LayoutScene`, `CVD` preview pill, `Rulers`,
//!   and strictly `Graph`/`Layout`/`Gallery`-only Zoom controls
//! - `640×420` & `1440×600` AccessKit `Role::ScrollBar` non-overlap, themes, and all `14` Beta locales.

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ChecklistItem, ChecklistStatus, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, HookSource,
    ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LinkSource, LinkedDataRegion, MultipleVariablesTable,
    NonlinearFitSpec, ParameterRule, ParameterScope, PhysicalUnit, ProjectDocument, ResultScalarEntry, ResultSheet, ResultStatus, SheetCellRange,
    SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_math::AxisScale;
use curvebook_palette::PaletteId;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::canvas_view::CanvasHoverReadout;
use curvebook_ui_egui::egui_shell::driver::{
    BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, accesskit_accessible_name, build_driver_font_definitions,
    collect_tooltip_texts_from_accesskit,
};
use curvebook_ui_egui::egui_shell::inspect_bridge::{check_accesskit_scrollbar_non_overlap, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::status_footer::{QUICK_STATS_CELL_CAP, QuickStatsCache, compute_cached_quick_stats};
use curvebook_ui_egui::egui_shell::trailing_studio::{
    GraphStudioTab, ResultsStudioTab, StudioContextSurface, collect_info_downstream_references, resolve_formula_operand_highlights,
    resolve_studio_surface,
};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{UiApp, UiThemeSpec};
use serde_json::json;

struct ShellDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    trailing_rect_pt: Option<egui::Rect>,
    footer_rect_pt: Option<egui::Rect>,
    leading_rect_pt: Option<egui::Rect>,
    hover_readout_override: Option<CanvasHoverReadout>,
}

impl ShellDriverState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self {
            app,
            views,
            intents: PendingUiIntents::new(),
            trailing_rect_pt: None,
            footer_rect_pt: None,
            leading_rect_pt: None,
            hover_readout_override: None,
        }
    }
}

fn make_shell_driver<'a>(state: ShellDriverState, metrics: ViewportMetrics, tokens: Tokens) -> EguiHumanPathDriver<'a, ShellDriverState> {
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(state, metrics, tokens, build_driver_font_definitions(), |ui, meta, st| {
        if let Some(ref ov) = st.hover_readout_override {
            st.views.canvas_view.hover_readout = Some(ov.clone());
        }
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.trailing_rect_pt = rep.trailing_dock_rect_pt;
        st.footer_rect_pt = rep.footer_rect_pt;
        st.leading_rect_pt = rep.leading_dock_rect_pt;
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
    driver
}

fn seed_full_workspace_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("R15.3 Studio & Footer Test")?;

    // 1. Multiple Variables table (`mv-1`) with calculated formula referencing `[Dose]`
    let mut mv = MultipleVariablesTable::new(3, 4)?;
    mv.variables[0].title = String::from("Dose");
    mv.variables[1].title = String::from("Response");
    mv.variables[2].title = String::from("LogDose");
    for r in 0..4 {
        let d = ((r + 1) * 10) as f64;
        mv.variables[0].cells[r] = Cell::number(d);
        mv.variables[1].cells[r] = Cell::number(d * 0.25 + 1.0);
    }
    mv.variables[2].set_calculated_formula("ln([Dose])");
    let ds_mv = DataSheet::new("mv-1", "PK Multiple Variables", TableArchetype::MultipleVariables(mv))?;
    doc.add_data_sheet(ds_mv)?;

    // 2. Column table (`col-1`) with values & a LinkedDataRegion pointing upstream to `mv-1`
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 3, 5)?;
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Treated Low");
    col.data_sets[2].title = String::from("Treated High");
    for r in 0..5 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        col.data_sets[1].cells[r][0] = Cell::number(15.0 + r as f64);
        col.data_sets[2].cells[r][0] = Cell::number(22.0 + r as f64);
    }
    let mut ds_col = DataSheet::new("col-1", "Column Assay", TableArchetype::Column(col))?;
    ds_col.linked_regions.push(LinkedDataRegion {
        id: String::from("lnk-1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("mv-1"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(3, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 2, 0), CellCoord::new(3, 2, 0)),
        provenance_summary: String::from("Linked from PK Multiple Variables"),
    });
    doc.add_data_sheet(ds_col)?;

    // 3. InfoSheet (`info-1`) with typed constants and linked sheet
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata")?;
    info.linked_data_sheets.push(SheetId::new("col-1"));
    info.set_constant("VehicleMass", ConstantValue::Number(12.5), Some(String::from("mg")))?;
    info.set_constant("DrugName", ConstantValue::Text(String::from("Compound-X")), None)?;
    doc.add_info_sheet(info)?;

    // 4. ResultSheet (`res-anova`) with Pass/Warn/Violated checklist diagnostics & scalars, plus hooked NLR (`res-nlr`)
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_gui_only(true);
    let _ = app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-anova", "source_sheet_id": "col-1", "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;

    let mut doc2 = app.document().clone();
    if let Some(rs) = doc2.result_sheet_mut(&SheetId::new("res-anova")) {
        rs.checklist = vec![
            ChecklistItem {
                assumption: String::from("Normality of residuals"),
                status: ChecklistStatus::Pass,
                diagnostic_metric: Some(String::from("Shapiro-Wilk W = 0.96, p = 0.62")),
                explanation: String::from("Residuals are consistent with Gaussian noise."),
                remediation: None,
            },
            ChecklistItem {
                assumption: String::from("Homogeneity of variances"),
                status: ChecklistStatus::Warn,
                diagnostic_metric: Some(String::from("Brown-Forsythe F = 3.12, p = 0.048")),
                explanation: String::from("Mild variance inequality across treatment arms."),
                remediation: None,
            },
            ChecklistItem {
                assumption: String::from("Extreme leverage outlier"),
                status: ChecklistStatus::Violated,
                diagnostic_metric: Some(String::from("ROUT Q = 1% flagged row 4")),
                explanation: String::from("High residual leverage observed."),
                remediation: None,
            },
        ];
        rs.scalars.push(ResultScalarEntry { key: String::from("R2"), value: 0.912, units: None, data_set_index: None });
    }

    let mut nlr_fit = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nlr_fit.parameter_rules = vec![ParameterRule {
        name: String::from("Bottom"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-1")),
            constant_name: String::from("VehicleMass"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }];
    let nlr_spec = AnalysisSpec::new("res-nlr", "col-1", AnalysisKind::NonlinearRegression(nlr_fit))?;
    let mut rs_nlr = ResultSheet::new(nlr_spec)?;
    rs_nlr.status = ResultStatus::UpToDate;
    doc2.add_result_sheet(rs_nlr)?;
    *app.session_mut() = Session::from_document(doc2);

    // 5. GraphSheet (`graph-1`) with inline `{{info:...}}` hook & LayoutSheet (`layout-1`) with `{{info:...}}` hook
    let _ =
        app.execute_command("graph.create", json!({ "data_sheet_id": "col-1", "sheet_id": "graph-1", "kind": "column", "title": "Response Graph" }))?;
    let _ = app.execute_command("graph.setSpec", json!({ "sheet_id": "graph-1", "spec": { "title": "Response ({{info:VehicleMass}})" } }))?;
    let _ = app.execute_command("layout.create", json!({ "sheet_id": "layout-1", "title": "Main Figure", "preset": "double_column" }))?;
    let _ =
        app.execute_command("layout.addPanel", json!({ "sheet_id": "layout-1", "kind": "graph", "graph_sheet_id": "graph-1", "row": 0, "col": 0 }))?;
    let _ = app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "layout-1", "kind": "text", "markdown": "Compound: {{info:DrugName}}", "bounds_pt": [20.0, 20.0, 120.0, 24.0] }),
    )?;

    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "mv-1" }))?;
    app.sync_views();
    Ok(app)
}

fn verify_studio_mv_surface(driver: &mut EguiHumanPathDriver<'_, ShellDriverState>) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::MultipleVariablesFormula);
    if let Some(ds) = driver.state().app.document().data_sheet(&SheetId::new("mv-1"))
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
    {
        let pure_hl = resolve_formula_operand_highlights(&mv.variables, "ln([Dose]) + [Response]");
        assert_eq!(pure_hl.len(), 2);
    }
    driver.click_widget_id("inspector.mv.col.2")?;
    let mv_hls = &driver.state().views.trailing_studio.mv_operand_highlights;
    assert_eq!(mv_hls.len(), 1, "calculated ln([Dose]) should highlight [Dose]");
    assert_eq!(mv_hls[0].variable_title, "Dose");

    driver.state_mut().views.trailing_studio.mv_formula_draft = String::from("=[Dose] * 2 + [Response]");
    driver.run_settled();
    let hls = &driver.state().views.trailing_studio.mv_operand_highlights;
    assert_eq!(hls.len(), 2, "two unique operands should emit two distinct colored outlines: {hls:?}");
    assert_ne!(hls[0].color_hex, hls[1].color_hex);

    driver.click_widget_id("inspector.mv.apply_formula")?;
    let ds_mv = driver.state().app.document().data_sheet(&SheetId::new("mv-1")).ok_or("mv-1 missing")?;
    if let TableArchetype::MultipleVariables(mv) = &ds_mv.archetype {
        assert_eq!(mv.variables[2].source, curvebook_doc::VariableSource::Calculated { formula: String::from("[Dose] * 2 + [Response]") });
    }
    Ok(())
}

fn verify_studio_table_and_info_surfaces(driver: &mut EguiHumanPathDriver<'_, ShellDriverState>) -> Result<(), Box<dyn std::error::Error>> {
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
    driver.state_mut().app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    driver.state_mut().app.grid.tool.extend_selection(GridCellCoord::new(1, 0, 0));
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::TableSelection);
    driver.click_widget_id("inspector.table.decimals.3")?;
    assert_eq!(driver.state().views.trailing_studio.table_decimals, Some(3));
    driver.click_widget_id("inspector.table.exclude_subset")?;
    let ds_col = driver.state().app.document().data_sheet(&SheetId::new("col-1")).ok_or("col-1")?;
    assert!(ds_col.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded));
    assert!(ds_col.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| c.excluded));
    driver.click_widget_id("inspector.table.include_subset")?;
    let ds_col2 = driver.state().app.document().data_sheet(&SheetId::new("col-1")).ok_or("col-1")?;
    assert!(!ds_col2.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded));

    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "info-1" }))?;
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::MetadataHooks);
    let info_sheet = driver.state().app.document().info_sheet(&SheetId::new("info-1")).cloned().ok_or("info-1")?;
    let refs = collect_info_downstream_references(driver.state().app.document(), &info_sheet);
    assert!(refs.iter().any(|r| r.target_sheet_id.as_str() == "res-nlr"));
    assert!(refs.iter().any(|r| r.target_sheet_id.as_str() == "graph-1"));
    assert!(refs.iter().any(|r| r.target_sheet_id.as_str() == "layout-1"));
    let vm_idx = info_sheet.constants.iter().position(|c| c.name == "VehicleMass").ok_or("VehicleMass missing")?;
    driver.click_widget_id(&format!("inspector.info.const.{vm_idx}"))?;
    driver.state_mut().views.trailing_studio.info_value_draft = String::from("25.0");
    driver.click_widget_id("inspector.info.save_const")?;
    assert_eq!(
        driver
            .state()
            .app
            .document()
            .info_sheet(&SheetId::new("info-1"))
            .and_then(|i| i.get_constant("VehicleMass"))
            .and_then(|c| c.value.as_finite_f64()),
        Some(25.0)
    );
    driver.click_widget_id("inspector.info.ref.0")?;
    assert_ne!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("info-1"));
    Ok(())
}

fn verify_studio_results_and_graph_surfaces(driver: &mut EguiHumanPathDriver<'_, ShellDriverState>) -> Result<(), Box<dyn std::error::Error>> {
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "res-anova" }))?;
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::ResultsInterpret);
    driver.click_widget_id("inspector.results.filter_flagged")?;
    assert!(driver.state().views.trailing_studio.results_severity_flagged_only);
    driver.click_widget_id("inspector.results.tab.guidance")?;
    assert_eq!(driver.state().views.trailing_studio.results_tab, ResultsStudioTab::Guidance);
    driver.click_widget_id("inspector.results.tab.methods")?;
    assert_eq!(driver.state().views.trailing_studio.results_tab, ResultsStudioTab::Methods);
    driver.click_widget_id("inspector.methods.fmt.plain")?;
    driver.click_widget_id("inspector.copy_methods")?;
    assert!(driver.state().views.trailing_studio.last_copied_methods);
    assert!(!driver.state().views.trailing_studio.last_copied_methods_text.trim().is_empty());
    assert!(driver.state().app.canvas_dialogs.result_view.last_methods_report.is_some());
    driver.click_widget_id("inspector.results.edit_params")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::None;

    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "graph-1" }))?;
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::GraphInspector);
    for (tab_wid, expected_tab) in [
        ("inspector.graph.tab.data", GraphStudioTab::Data),
        ("inspector.graph.tab.axes", GraphStudioTab::Axes),
        ("inspector.graph.tab.legends", GraphStudioTab::Legends),
        ("inspector.graph.tab.annotations", GraphStudioTab::Annotations),
        ("inspector.graph.tab.canvas", GraphStudioTab::Canvas),
    ] {
        driver.click_widget_id(tab_wid)?;
        assert_eq!(driver.state().views.trailing_studio.graph_tab, expected_tab);
    }
    driver.click_widget_id("inspector.graph.tab.data")?;
    driver.click_widget_id("inspector.graph.palette.PaulTolBright")?;
    assert_eq!(driver.state().app.document().graph_sheet(&SheetId::new("graph-1")).unwrap().spec.palette, PaletteId::PaulTolBright);
    driver.click_widget_id("inspector.graph.tab.axes")?;
    driver.click_widget_id("inspector.graph.scale.Log10")?;
    assert_eq!(driver.state().app.document().graph_sheet(&SheetId::new("graph-1")).unwrap().spec.y_axis.segments[0].scale, AxisScale::Log10);
    driver.click_widget_id("inspector.graph.more_axes")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::None;
    Ok(())
}

fn verify_studio_drillback_and_layout_surfaces(driver: &mut EguiHumanPathDriver<'_, ShellDriverState>) -> Result<(), Box<dyn std::error::Error>> {
    driver.state_mut().app.brush_from_grid_cell(&SheetId::new("col-1"), GridCellCoord::new(1, 0, 0));
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::DataPointDrillBack);
    driver.click_widget_id("inspector.drill_back.toggle_exclude")?;
    assert!(driver.state().app.document().data_sheet(&SheetId::new("col-1")).unwrap().cell(CellCoord::new(1, 0, 0)).unwrap().excluded);
    driver.click_widget_id("inspector.drill_back.back")?;
    assert!(driver.state().app.brushed_cell.is_none());
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::GraphInspector);

    driver.state_mut().app.brush_from_grid_cell(&SheetId::new("col-1"), GridCellCoord::new(2, 1, 0));
    driver.run_settled();
    driver.click_widget_id("inspector.drill_back.jump")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
    assert_eq!(driver.state().app.grid.tool.active, Some(GridCellCoord::new(2, 1, 0)));

    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "layout-1" }))?;
    driver.state_mut().app.clear_brush();
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::PageLayoutComposer);
    for (wid, expected_w) in
        [("inspector.layout.preset.85mm", 85.0_f64), ("inspector.layout.preset.114mm", 114.0_f64), ("inspector.layout.preset.174mm", 174.0_f64)]
    {
        driver.click_widget_id(wid)?;
        let spec = &driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec;
        assert!((spec.width - expected_w).abs() < 0.05, "{wid}: expected {expected_w} mm, got {}", spec.width);
        assert_eq!(spec.unit, PhysicalUnit::Millimeters);
    }
    driver.click_widget_id("inspector.layout.dpi.600")?;
    assert_eq!(driver.state().views.trailing_studio.layout_dpi, 600);
    let snap_before = driver.state().views.trailing_studio.snap_enabled;
    driver.click_widget_id("inspector.layout.snap")?;
    assert_ne!(driver.state().views.trailing_studio.snap_enabled, snap_before);
    driver.click_widget_id("inspector.layout.grid.2x2")?;
    assert_eq!(driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec.grid_template, Some((2, 2)));
    let align_before = driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec.align_plot_origins;
    driver.click_widget_id("inspector.layout.align_origins")?;
    assert_ne!(driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec.align_plot_origins, align_before);
    Ok(())
}

#[test]
fn studio_resolves_all_seven_context_surfaces_and_commits_each_domain() -> Result<(), Box<dyn std::error::Error>> {
    let app = seed_full_workspace_app()?;
    let metrics = ViewportMetrics::new(1440.0, 800.0, 1.0);
    let mut driver = make_shell_driver(ShellDriverState::new(app), metrics, Tokens::default());
    verify_studio_mv_surface(&mut driver)?;
    verify_studio_table_and_info_surfaces(&mut driver)?;
    verify_studio_results_and_graph_surfaces(&mut driver)?;
    verify_studio_drillback_and_layout_surfaces(&mut driver)?;
    Ok(())
}

#[test]
fn shortcut_cmd_shift_i_and_responsive_auto_collapse_breakpoints() -> Result<(), Box<dyn std::error::Error>> {
    let app = seed_full_workspace_app()?;
    let metrics = ViewportMetrics::new(1440.0, 600.0, 1.0);
    let mut driver = make_shell_driver(ShellDriverState::new(app), metrics, Tokens::default());

    // 1440×600: both Leading (~240 pt) and Trailing (~280 pt) expanded!
    let l_1440 = driver.state().leading_rect_pt.ok_or("leading 1440")?.width();
    let t_1440 = driver.state().trailing_rect_pt.ok_or("trailing 1440")?.width();
    assert!(l_1440 > 200.0 && l_1440 < 270.0, "expected ~240 pt leading at 1440, got {l_1440}");
    assert!(t_1440 > 260.0 && t_1440 < 300.0, "expected ~280 pt trailing at 1440, got {t_1440}");

    // `Cmd/Ctrl+Shift+I` manually collapses Trailing `280 pt -> 36 pt`, then expands back!
    driver.press_egui_key(egui::Key::I, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(driver.state().views.trailing_collapsed_rail);
    let t_collapsed = driver.state().trailing_rect_pt.ok_or("trailing collapsed")?.width();
    assert!((t_collapsed - 36.0).abs() < 1.0, "collapsed rail must be 36 pt, got {t_collapsed}");
    driver.press_egui_key(egui::Key::I, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(!driver.state().views.trailing_collapsed_rail);
    let t_reexpanded = driver.state().trailing_rect_pt.ok_or("trailing expanded")?.width();
    assert!((t_reexpanded - 280.0).abs() < 1.0);

    // 900×600 (`< 960 pt` but `>= 760 pt`): Trailing auto-collapses FIRST to 36 pt before Leading!
    driver.set_viewport(ViewportMetrics::new(900.0, 600.0, 1.0));
    let l_900 = driver.state().leading_rect_pt.ok_or("leading 900")?.width();
    let t_900 = driver.state().trailing_rect_pt.ok_or("trailing 900")?.width();
    assert!(l_900 > 200.0, "Leading must remain expanded >= 760 pt (got {l_900})");
    assert!((t_900 - 36.0).abs() < 1.0, "Trailing must auto-collapse < 960 pt to 36 pt (got {t_900})");

    // 640×420 (`< 760 pt`): BOTH collapse to 36 pt rails cleanly with zero overlap!
    driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
    let l_640 = driver.state().leading_rect_pt.ok_or("leading 640")?.width();
    let t_640 = driver.state().trailing_rect_pt.ok_or("trailing 640")?.width();
    assert!((l_640 - 36.0).abs() < 1.0, "Leading must collapse < 760 pt to 36 pt (got {l_640})");
    assert!((t_640 - 36.0).abs() < 1.0, "Trailing must be 36 pt at 640 (got {t_640})");
    let parity = driver.verify_pixel_to_hit_box_parity();
    assert!(parity.is_clean(), "640x420 collapsed parity violations: {:?}", parity.violations);
    assert!(driver.check_scrollbar_non_overlap().is_empty());
    Ok(())
}

fn verify_footer_visibility_and_cursor_readout(driver: &mut EguiHumanPathDriver<'_, ShellDriverState>) -> Result<(), Box<dyn std::error::Error>> {
    let f_rect = driver.state().footer_rect_pt.ok_or("footer rect")?;
    assert!((f_rect.height() - 26.0).abs() < 1.0, "expected 26 pt footer height, got {}", f_rect.height());

    {
        let mut doc = driver.state().app.document().clone();
        doc.active_sheet_id = None;
        *driver.state_mut().app.session_mut() = Session::from_document(doc);
        driver.run_settled();
    }
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none(), "rulers hidden on Welcome");
    assert!(driver.frame_meta().find_by_widget_id("status.zoom.fit").is_none(), "zoom hidden on Welcome");

    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("status.link_indicator").is_some(), "expected LinkedDataRegion badge on col-1");
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none(), "rulers hidden on DataSheet");
    assert!(driver.frame_meta().find_by_widget_id("status.zoom.fit").is_none(), "zoom hidden on DataSheet");

    driver.click_widget_id("status.section.info")?;
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none());
    assert!(driver.frame_meta().find_by_widget_id("status.zoom.in").is_none());

    driver.click_widget_id("status.section.results")?;
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none());
    assert!(driver.frame_meta().find_by_widget_id("status.zoom.in").is_none());

    driver.click_widget_id("status.section.graphs")?;
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_some());
    assert!(driver.frame_meta().find_by_widget_id("status.zoom.in").is_some());
    driver.dispatch_events(vec![egui::Event::PointerMoved(egui::pos2(520.0, 260.0))]);
    let ptr_summary = driver.frame_meta().find_by_widget_id("status.summary").ok_or("status.summary ptr")?;
    assert!(ptr_summary.label.contains("X:") && ptr_summary.label.contains("Y:"), "live pointer coords missing: {:?}", ptr_summary.label);
    driver.state_mut().hover_readout_override = Some(CanvasHoverReadout {
        source_sheet_id: Some(SheetId::new("col-1")),
        point_index: Some(0),
        row: Some(0),
        col: Some(0),
        label: String::from("Control (X=1, Y=10.0)"),
        value_summary: String::from("10.00"),
    });
    driver.run_settled();
    let summary_entry = driver.frame_meta().find_by_widget_id("status.summary").ok_or("status.summary")?;
    assert!(summary_entry.label.contains("10.00"));
    driver.state_mut().hover_readout_override = None;
    Ok(())
}

#[test]
fn status_footer_history_sections_switcher_links_cursor_cvd_rulers_and_zoom_gates() -> Result<(), Box<dyn std::error::Error>> {
    let app = seed_full_workspace_app()?;
    let metrics = ViewportMetrics::new(1440.0, 600.0, 1.0);
    let mut driver = make_shell_driver(ShellDriverState::new(app), metrics, Tokens::default());

    verify_footer_visibility_and_cursor_readout(&mut driver)?;

    driver.click_widget_id("status.rulers")?;
    assert!(driver.state().app.canvas_dialogs.rulers_visible);
    driver.click_widget_id("status.zoom.in")?;
    assert!(driver.state().app.canvas_dialogs.canvas.zoom > 1.0);
    driver.click_widget_id("status.zoom.reset")?;
    assert!((driver.state().app.canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);

    driver.click_widget_id("status.section.layouts")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_some());
    assert!(driver.frame_meta().find_by_widget_id("status.zoom.in").is_some());

    driver.click_widget_id("status.section.gallery")?;
    assert!(driver.state().app.nav.gallery.open);
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none());
    assert!(driver.frame_meta().find_by_widget_id("status.zoom.in").is_some());
    driver.click_widget_id("status.section.gallery")?;
    assert!(!driver.state().app.nav.gallery.open);

    driver.click_widget_id("status.history.back")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    driver.click_widget_id("status.history.forward")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));

    driver.click_widget_id("status.sheet_switcher")?;
    driver.click_widget_id("status.sheet_switcher.item.col-1")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));

    // Upstream LinkedDataRegion badge popup jump -> `mv-1`
    driver.click_widget_id("status.link_indicator")?;
    driver.click_widget_id("status.link_indicator.item.0")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("mv-1"));

    // CVD preview filter now lives in Preferences (Options) rather than the main status footer.
    assert!(driver.frame_meta().find_by_widget_id("status.preview_filter").is_none());
    Ok(())
}

#[test]
fn quick_stats_50k_cells_under_half_ms_and_cached_steady_frames() -> Result<(), Box<dyn std::error::Error>> {
    // Construct 60,000-row table (`> QUICK_STATS_CELL_CAP`) so we test both exactness on `50,000`
    // selected cells AND cap truncation cleanly on `60,000` selected cells!
    let mut table = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 1, 60_000)?;
    for r in 0..60_000 {
        table.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let mut doc = ProjectDocument::new("50k Perf")?;
    let ds = DataSheet::new("col-50k", "Big Sheet", TableArchetype::Column(table))?;
    doc.add_data_sheet(ds)?;
    doc.select_in_navigator(&SheetId::new("col-50k"))?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    app.grid.tool.extend_selection(GridCellCoord::new(49_999, 0, 0));

    let mut cache = QuickStatsCache::default();
    let t0 = Instant::now();
    let stats = compute_cached_quick_stats(&app, &mut cache).clone();
    let first_ms = t0.elapsed().as_secs_f64() * 1000.0;

    assert_eq!(stats.n, QUICK_STATS_CELL_CAP);
    assert!(!stats.truncated_at_cap);
    assert!((stats.mean.unwrap() - 25_000.5).abs() < 1e-6);
    assert!((stats.median.unwrap() - 25_000.5).abs() < 1e-6);
    assert!((stats.range.unwrap() - 49_999.0).abs() < 1e-6);
    assert!(stats.sd.unwrap() > 14_433.0 && stats.sd.unwrap() < 14_435.0);
    assert!((stats.sem.unwrap() - stats.sd.unwrap() / (50_000.0_f64).sqrt()).abs() < 1e-6);
    assert_eq!(cache.recompute_count, 1);

    // Measure true 50,000-cell reduction speed across 5 distinct cache-miss selection windows (zero OS cold-alloc noise)
    let mut miss_samples_ms = Vec::with_capacity(5);
    for shift in 1..=5_usize {
        app.grid.tool.select_single(GridCellCoord::new(shift, 0, 0));
        app.grid.tool.extend_selection(GridCellCoord::new(shift + 49_999, 0, 0));
        let ts = Instant::now();
        let next_stats = compute_cached_quick_stats(&app, &mut cache);
        let dt_ms = ts.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(next_stats.n, QUICK_STATS_CELL_CAP);
        miss_samples_ms.push(dt_ms);
    }
    miss_samples_ms.sort_by(f64::total_cmp);
    let median_miss_ms = miss_samples_ms[2];
    let median_budget_ms = if cfg!(debug_assertions) { 2.0 } else { 0.5 };
    assert!(
        first_ms < 25.0 && median_miss_ms < median_budget_ms,
        "50,000-cell Quick-Stats timing out of budget: first={first_ms:.3} ms, median={median_miss_ms:.3} ms (samples={miss_samples_ms:?})"
    );

    // Steady-frame cache hit over 1,000 unchanged frames must do ZERO recomputations!
    let count_before_steady = cache.recompute_count;
    let t1 = Instant::now();
    for _ in 0..1_000 {
        let hit = compute_cached_quick_stats(&app, &mut cache);
        assert_eq!(hit.n, QUICK_STATS_CELL_CAP);
    }
    let steady_elapsed = t1.elapsed();
    assert_eq!(cache.recompute_count, count_before_steady, "steady frames must NEVER recompute Quick-Stats");
    assert!(steady_elapsed.as_secs_f64() * 1000.0 < 0.5, "1,000 steady cache hits took {:.3} ms", steady_elapsed.as_secs_f64() * 1000.0);

    // Selecting 60,000 cells respects `QUICK_STATS_CELL_CAP` (`50,000`) cleanly.
    app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    app.grid.tool.extend_selection(GridCellCoord::new(59_999, 0, 0));
    let capped = compute_cached_quick_stats(&app, &mut cache).clone();
    assert_eq!(capped.n, QUICK_STATS_CELL_CAP);
    assert!(capped.truncated_at_cap);
    assert_eq!(cache.recompute_count, count_before_steady + 1);
    Ok(())
}

#[test]
fn responsive_640x420_1440x600_scrollbar_non_overlap_themes_and_all_14_beta_locales() -> Result<(), Box<dyn std::error::Error>> {
    let base_app = seed_full_workspace_app()?;

    // Verify all 14 Beta locales × Themes across 1440×600 & 640×420 (plus ellipsis AccessKit & tooltip parity!)
    for (idx, locale) in BETA_LOCALE_CODES.iter().enumerate() {
        let mode = match idx % 3 {
            0 => UiThemeMode::Light,
            1 => UiThemeMode::Dark,
            _ => UiThemeMode::HighContrast,
        };
        let mut app = base_app.clone();
        assert_eq!(app.set_locale(locale), *locale);
        app.set_theme_mode(mode);

        // Long Info constant triggers ellipsizing inside Studio with AccessKit + hover tooltip fallback
        let long_title = "UltraLongAssayCalibrationConstantThatTriggersStudioEllipsisCorrectlyAcrossLocales";
        let mut doc = app.document().clone();
        if let Some(ds) = doc.data_sheet_mut(&SheetId::new("col-1")) {
            ds.title = String::from(long_title);
        }
        *app.session_mut() = Session::from_document(doc);
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
        app.sync_views();

        let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), locale);
        let mut driver = make_shell_driver(ShellDriverState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0), tokens);

        let wide_rep = driver.verify_pixel_to_hit_box_parity();
        assert!(wide_rep.is_clean(), "{locale}/{mode:?} 1440x600 parity violations: {:?}", wide_rep.violations);
        assert!(driver.check_scrollbar_non_overlap().is_empty(), "{locale}/{mode:?} 1440x600 scrollbar overlap");

        // Switch to narrow sheet switcher item vs MV Studio to verify ellipsized widget AccessKit + tooltip
        if let Some(sw) = driver.frame_meta().find_by_widget_id("status.sheet_switcher").cloned()
            && sw.ellipsized
        {
            let expected_full = sw.full_label.clone().unwrap_or_default();
            driver.verify_ellipsis_tooltip_and_accesskit("status.sheet_switcher", &expected_full)?;
            let ak = driver.accesskit_tree().ok_or("missing accesskit")?;
            let ak_node = ak.nodes.iter().find(|(id, _)| *id == sw.accesskit_id).map(|(_, n)| n).ok_or("missing sw ak")?;
            assert_eq!(accesskit_accessible_name(ak_node), Some(expected_full.as_str()));
            let tips = collect_tooltip_texts_from_accesskit(&ak.nodes);
            assert!(tips.iter().any(|t| t.contains(&expected_full)));
        }

        // Compact 640×420 auto-collapse check
        driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
        let compact_rep = driver.verify_pixel_to_hit_box_parity();
        assert!(compact_rep.is_clean(), "{locale}/{mode:?} 640x420 parity violations: {:?}", compact_rep.violations);
        assert!(driver.check_scrollbar_non_overlap().is_empty(), "{locale}/{mode:?} 640x420 scrollbar overlap");
    }

    // Verify AccessKit `Role::ScrollBar` non-overlap directly on scrollable MV Studio at 1440×600 & 640×420
    for (w, h) in [(1440.0_f32, 600.0), (640.0, 420.0)] {
        let mut ak_app = base_app.clone();
        let _ = ak_app.execute_command("sheet.select", json!({ "sheet_id": "mv-1" }))?;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_fonts(build_driver_font_definitions());
        let mut views = EguiShellViewState::default();
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::new();
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))), ..Default::default() },
            |ui| {
                meta.begin_pass(ui.ctx());
                let _ = render_egui_shell_frame(ui, &ak_app, &mut views, &mut meta, &mut intents);
            },
        );
        out.textures_delta.clear();
        let ak_update = out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?;
        check_accesskit_scrollbar_non_overlap(ak_update)?;
    }

    Ok(())
}

#[test]
fn studio_assumptions_wrap_and_offscreen_scroll_buttons_culled_at_1440x600() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = seed_full_workspace_app()?;
    let mut doc = app.document().clone();
    if let Some(rs) = doc.result_sheet_mut(&SheetId::new("res-anova")) {
        rs.checklist.insert(
            0,
            ChecklistItem {
                assumption: String::from("Repeated-measures condition normality (Shapiro-Wilk)"),
                status: ChecklistStatus::Warn,
                diagnostic_metric: Some(String::from("VeryLongDiagnosticMetricHeader W = 0.8421, p = 0.0142 across treatment conditions")),
                explanation: String::from(
                    "Long assumption diagnostic explanation verifying word wrap inside 280pt Studio card without horizontal overflow.",
                ),
                remediation: None,
            },
        );
    }
    *app.session_mut() = Session::from_document(doc);

    for sheet_id in ["res-anova", "graph-1", "layout-1"] {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": sheet_id }))?;
        app.sync_views();
        let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), "en");
        let driver = make_shell_driver(ShellDriverState::new(app.clone()), ViewportMetrics::new(1440.0, 600.0, 1.0), tokens);
        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "{sheet_id} studio 1440x600 parity violations: {:?}", parity.violations);
        assert!(driver.check_scrollbar_non_overlap().is_empty(), "{sheet_id} scrollbar overlap at 1440x600");
    }
    Ok(())
}
