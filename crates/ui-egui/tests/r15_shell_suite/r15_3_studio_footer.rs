#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R15.3` Trailing 7-Context Studio Rail (`280 pt -> 36 pt`) & `26 pt` Status & Telemetry Footer
//! integration tests.

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
    BETA_LOCALE_CODES, EguiHumanPathDriver, accesskit_accessible_name, build_driver_font_definitions, collect_tooltip_texts_from_accesskit,
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

use crate::common::{ShellTestState, assert_clean_parity, fill_col_values, make_shell_driver_with_state};

type BoxRes = Result<(), Box<dyn std::error::Error>>;

fn select_sheet(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, sheet_id: &str) -> BoxRes {
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": sheet_id }))?;
    driver.run_settled();
    Ok(())
}

fn checklist_item(assumption: &str, status: ChecklistStatus, metric: &str, explanation: &str) -> ChecklistItem {
    ChecklistItem {
        assumption: assumption.to_owned(),
        status,
        diagnostic_metric: Some(metric.to_owned()),
        explanation: explanation.to_owned(),
        remediation: None,
    }
}

fn seed_full_workspace_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("R15.3 Studio & Footer Test")?;

    let mut mv = MultipleVariablesTable::new(3, 4)?;
    for (idx, title) in ["Dose", "Response", "LogDose"].iter().enumerate() {
        mv.variables[idx].title = (*title).to_owned();
    }
    for r in 0..4 {
        let d = ((r + 1) * 10) as f64;
        mv.variables[0].cells[r] = Cell::number(d);
        mv.variables[1].cells[r] = Cell::number(d * 0.25 + 1.0);
    }
    mv.variables[2].set_calculated_formula("ln([Dose])");
    doc.add_data_sheet(DataSheet::new("mv-1", "PK Multiple Variables", TableArchetype::MultipleVariables(mv))?)?;

    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 3, 5)?;
    fill_col_values(&mut col.data_sets[0], "Control", &[10.0, 11.0, 12.0, 13.0, 14.0]);
    fill_col_values(&mut col.data_sets[1], "Treated Low", &[15.0, 16.0, 17.0, 18.0, 19.0]);
    fill_col_values(&mut col.data_sets[2], "Treated High", &[22.0, 23.0, 24.0, 25.0, 26.0]);
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

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata")?;
    info.linked_data_sheets.push(SheetId::new("col-1"));
    info.set_constant("VehicleMass", ConstantValue::Number(12.5), Some(String::from("mg")))?;
    info.set_constant("DrugName", ConstantValue::Text(String::from("Compound-X")), None)?;
    doc.add_info_sheet(info)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let _ = app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-anova", "source_sheet_id": "col-1", "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;

    let mut doc2 = app.document().clone();
    if let Some(rs) = doc2.result_sheet_mut(&SheetId::new("res-anova")) {
        rs.checklist = vec![
            checklist_item(
                "Normality of residuals",
                ChecklistStatus::Pass,
                "Shapiro-Wilk W = 0.96, p = 0.62",
                "Residuals are consistent with Gaussian noise.",
            ),
            checklist_item(
                "Homogeneity of variances",
                ChecklistStatus::Warn,
                "Brown-Forsythe F = 3.12, p = 0.048",
                "Mild variance inequality across treatment arms.",
            ),
            checklist_item("Extreme leverage outlier", ChecklistStatus::Violated, "ROUT Q = 1% flagged row 4", "High residual leverage observed."),
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
    let mut rs_nlr = ResultSheet::new(AnalysisSpec::new("res-nlr", "col-1", AnalysisKind::NonlinearRegression(nlr_fit))?)?;
    rs_nlr.status = ResultStatus::UpToDate;
    doc2.add_result_sheet(rs_nlr)?;
    *app.session_mut() = Session::from_document(doc2);

    for (cmd, params) in [
        ("graph.create", json!({ "data_sheet_id": "col-1", "sheet_id": "graph-1", "kind": "column", "title": "Response Graph" })),
        ("graph.setSpec", json!({ "sheet_id": "graph-1", "spec": { "title": "Response ({{info:VehicleMass}})" } })),
        ("layout.create", json!({ "sheet_id": "layout-1", "title": "Main Figure", "preset": "double_column" })),
        ("layout.addPanel", json!({ "sheet_id": "layout-1", "kind": "graph", "graph_sheet_id": "graph-1", "row": 0, "col": 0 })),
        (
            "layout.addPanel",
            json!({ "sheet_id": "layout-1", "kind": "text", "markdown": "Compound: {{info:DrugName}}", "bounds_pt": [20.0, 20.0, 120.0, 24.0] }),
        ),
        ("sheet.select", json!({ "sheet_id": "mv-1" })),
    ] {
        let _ = app.execute_command(cmd, params)?;
    }
    app.sync_views();
    Ok(app)
}

fn verify_studio_mv_and_table_surfaces(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) -> BoxRes {
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::MultipleVariablesFormula);
    if let Some(ds) = driver.state().app.document().data_sheet(&SheetId::new("mv-1"))
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
    {
        assert_eq!(resolve_formula_operand_highlights(&mv.variables, "ln([Dose]) + [Response]").len(), 2);
    }
    driver.click_widget_id("inspector.mv.col.2")?;
    let mv_hls = &driver.state().views.trailing_studio.mv_operand_highlights;
    assert_eq!(mv_hls.len(), 1);
    assert_eq!(mv_hls[0].variable_title, "Dose");

    driver.state_mut().views.trailing_studio.mv_formula_draft = String::from("=[Dose] * 2 + [Response]");
    driver.run_settled();
    let hls = &driver.state().views.trailing_studio.mv_operand_highlights;
    assert_eq!(hls.len(), 2);
    assert_ne!(hls[0].color_hex, hls[1].color_hex);

    driver.click_widget_id("inspector.mv.apply_formula")?;
    if let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::new("mv-1")).ok_or("mv-1 missing")?.archetype {
        assert_eq!(mv.variables[2].source, curvebook_doc::VariableSource::Calculated { formula: String::from("[Dose] * 2 + [Response]") });
    }

    select_sheet(driver, "col-1")?;
    driver.state_mut().app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    driver.state_mut().app.grid.tool.extend_selection(GridCellCoord::new(1, 0, 0));
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::TableSelection);
    driver.click_widget_id("inspector.table.decimals.3")?;
    assert_eq!(driver.state().views.trailing_studio.table_decimals, Some(3));
    driver.click_widget_id("inspector.table.exclude_subset")?;
    let ds_col = driver.state().app.document().data_sheet(&SheetId::new("col-1")).ok_or("col-1")?;
    assert!(ds_col.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded) && ds_col.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| c.excluded));
    driver.click_widget_id("inspector.table.include_subset")?;
    assert!(
        !driver.state().app.document().data_sheet(&SheetId::new("col-1")).ok_or("col-1")?.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded)
    );

    select_sheet(driver, "info-1")?;
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::MetadataHooks);
    let info_sheet = driver.state().app.document().info_sheet(&SheetId::new("info-1")).cloned().ok_or("info-1")?;
    let refs = collect_info_downstream_references(driver.state().app.document(), &info_sheet);
    for expected in ["res-nlr", "graph-1", "layout-1"] {
        assert!(refs.iter().any(|r| r.target_sheet_id.as_str() == expected), "missing downstream ref `{expected}`");
    }
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

fn verify_studio_results_graph_drillback_and_layout(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) -> BoxRes {
    select_sheet(driver, "res-anova")?;
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::ResultsInterpret);
    driver.click_widget_id("inspector.results.filter_flagged")?;
    assert!(driver.state().views.trailing_studio.results_severity_flagged_only);
    for (wid, expected) in
        [("inspector.results.tab.guidance", ResultsStudioTab::Guidance), ("inspector.results.tab.methods", ResultsStudioTab::Methods)]
    {
        driver.click_widget_id(wid)?;
        assert_eq!(driver.state().views.trailing_studio.results_tab, expected);
    }
    driver.click_widget_id("inspector.methods.fmt.plain")?;
    driver.click_widget_id("inspector.copy_methods")?;
    assert!(driver.state().views.trailing_studio.last_copied_methods);
    assert!(!driver.state().views.trailing_studio.last_copied_methods_text.trim().is_empty());
    assert!(driver.state().app.canvas_dialogs.result_view.last_methods_report.is_some());
    driver.click_widget_id("inspector.results.edit_params")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::None;

    select_sheet(driver, "graph-1")?;
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::GraphInspector);
    for (wid, expected) in [
        ("inspector.graph.tab.data", GraphStudioTab::Data),
        ("inspector.graph.tab.axes", GraphStudioTab::Axes),
        ("inspector.graph.tab.legends", GraphStudioTab::Legends),
        ("inspector.graph.tab.annotations", GraphStudioTab::Annotations),
        ("inspector.graph.tab.canvas", GraphStudioTab::Canvas),
    ] {
        driver.click_widget_id(wid)?;
        assert_eq!(driver.state().views.trailing_studio.graph_tab, expected);
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

    driver.state_mut().app.clear_brush();
    select_sheet(driver, "layout-1")?;
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::PageLayoutComposer);
    for (wid, expected_w) in
        [("inspector.layout.preset.85mm", 85.0_f64), ("inspector.layout.preset.114mm", 114.0), ("inspector.layout.preset.174mm", 174.0)]
    {
        driver.click_widget_id(wid)?;
        let spec = &driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec;
        assert!((spec.width - expected_w).abs() < 0.05 && spec.unit == PhysicalUnit::Millimeters, "{wid}: got {} {:?}", spec.width, spec.unit);
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
fn studio_resolves_all_seven_context_surfaces_and_commits_each_domain() -> BoxRes {
    let mut driver =
        make_shell_driver_with_state(ShellTestState::new(seed_full_workspace_app()?), ViewportMetrics::new(1440.0, 800.0, 1.0), Tokens::default());
    verify_studio_mv_and_table_surfaces(&mut driver)?;
    verify_studio_results_graph_drillback_and_layout(&mut driver)
}

#[test]
fn shortcut_cmd_shift_i_and_responsive_auto_collapse_breakpoints() -> BoxRes {
    let mut driver =
        make_shell_driver_with_state(ShellTestState::new(seed_full_workspace_app()?), ViewportMetrics::new(1440.0, 600.0, 1.0), Tokens::default());

    let l_1440 = driver.state().leading_rect_pt.ok_or("leading 1440")?.width();
    let t_1440 = driver.state().trailing_rect_pt.ok_or("trailing 1440")?.width();
    assert!((200.0..270.0).contains(&l_1440) && (260.0..300.0).contains(&t_1440), "1440 widths: l={l_1440}, t={t_1440}");

    driver.press_egui_key(egui::Key::I, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(driver.state().views.trailing_collapsed_rail);
    assert!((driver.state().trailing_rect_pt.ok_or("trailing collapsed")?.width() - 36.0).abs() < 1.0);
    driver.press_egui_key(egui::Key::I, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(!driver.state().views.trailing_collapsed_rail);
    assert!((driver.state().trailing_rect_pt.ok_or("trailing expanded")?.width() - 280.0).abs() < 1.0);

    driver.set_viewport(ViewportMetrics::new(900.0, 600.0, 1.0));
    assert!(driver.state().leading_rect_pt.ok_or("leading 900")?.width() > 200.0);
    assert!((driver.state().trailing_rect_pt.ok_or("trailing 900")?.width() - 36.0).abs() < 1.0);

    driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
    assert!((driver.state().leading_rect_pt.ok_or("leading 640")?.width() - 36.0).abs() < 1.0);
    assert!((driver.state().trailing_rect_pt.ok_or("trailing 640")?.width() - 36.0).abs() < 1.0);
    assert_clean_parity(&driver, "640x420 collapsed")?;
    Ok(())
}

#[test]
fn status_footer_history_sections_switcher_links_cursor_cvd_rulers_and_zoom_gates() -> BoxRes {
    let mut driver =
        make_shell_driver_with_state(ShellTestState::new(seed_full_workspace_app()?), ViewportMetrics::new(1440.0, 600.0, 1.0), Tokens::default());
    assert!((driver.state().footer_rect_pt.ok_or("footer rect")?.height() - 26.0).abs() < 1.0);

    // Welcome: rulers and zoom hidden.
    {
        let mut doc = driver.state().app.document().clone();
        doc.active_sheet_id = None;
        *driver.state_mut().app.session_mut() = Session::from_document(doc);
        driver.run_settled();
    }
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none() && driver.frame_meta().find_by_widget_id("status.zoom.fit").is_none());

    select_sheet(&mut driver, "col-1")?;
    assert!(driver.frame_meta().find_by_widget_id("status.link_indicator").is_some());
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none() && driver.frame_meta().find_by_widget_id("status.zoom.fit").is_none());
    assert!(driver.frame_meta().find_by_widget_id("status.section.info").is_none());

    for (chip, sheet, has_rulers, has_zoom) in [
        ("nav.chip.info", "nav.sheet.info-1", false, false),
        ("nav.chip.results", "nav.sheet.res-anova", false, false),
        ("nav.chip.graphs", "nav.sheet.graph-1", true, true),
    ] {
        driver.click_widget_id(chip)?;
        driver.click_widget_id(sheet)?;
        assert_eq!(driver.frame_meta().find_by_widget_id("status.rulers").is_some(), has_rulers, "{sheet} rulers");
        assert_eq!(driver.frame_meta().find_by_widget_id("status.zoom.in").is_some(), has_zoom, "{sheet} zoom");
    }

    driver.dispatch_events(vec![egui::Event::PointerMoved(egui::pos2(520.0, 260.0))]);
    let ptr_summary = driver.frame_meta().find_by_widget_id("status.summary").ok_or("status.summary ptr")?;
    assert!(ptr_summary.label.contains("X:") && ptr_summary.label.contains("Y:"));
    driver.state_mut().hover_readout_override = Some(CanvasHoverReadout {
        source_sheet_id: Some(SheetId::new("col-1")),
        point_index: Some(0),
        row: Some(0),
        col: Some(0),
        label: String::from("Control (X=1, Y=10.0)"),
        value_summary: String::from("10.00"),
    });
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("status.summary").ok_or("status.summary")?.label.contains("10.00"));
    driver.state_mut().hover_readout_override = None;

    driver.click_widget_id("status.rulers")?;
    assert!(driver.state().app.canvas_dialogs.rulers_visible);
    driver.click_widget_id("status.zoom.in")?;
    assert!(driver.state().app.canvas_dialogs.canvas.zoom > 1.0);
    driver.click_widget_id("status.zoom.reset")?;
    assert!((driver.state().app.canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);

    driver.click_widget_id("nav.chip.layouts")?;
    driver.click_widget_id("nav.sheet.layout-1")?;
    driver.click_widget_id("nav.chip.all")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_some() && driver.frame_meta().find_by_widget_id("status.zoom.in").is_some());

    driver.click_widget_id("status.section.gallery")?;
    assert!(
        driver.state().app.nav.gallery.open
            && driver.frame_meta().find_by_widget_id("status.rulers").is_none()
            && driver.frame_meta().find_by_widget_id("status.zoom.in").is_some()
    );
    driver.click_widget_id("status.section.gallery")?;
    assert!(!driver.state().app.nav.gallery.open);

    driver.click_widget_id("status.history.back")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    driver.click_widget_id("status.history.forward")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));

    assert!(driver.frame_meta().find_by_widget_id("status.sheet_switcher").is_none());
    driver.click_widget_id("nav.collapse_rail")?;
    driver.click_widget_id("status.sheet_switcher")?;
    driver.click_widget_id("status.sheet_switcher.item.col-1")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));

    driver.click_widget_id("status.link_indicator")?;
    driver.click_widget_id("status.link_indicator.item.0")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("mv-1"));
    assert!(driver.frame_meta().find_by_widget_id("status.preview_filter").is_none());
    Ok(())
}

#[test]
fn quick_stats_50k_cells_under_half_ms_and_cached_steady_frames() -> BoxRes {
    let mut table = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 1, 60_000)?;
    for r in 0..60_000 {
        table.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let mut doc = ProjectDocument::new("50k Perf")?;
    doc.add_data_sheet(DataSheet::new("col-50k", "Big Sheet", TableArchetype::Column(table))?)?;
    doc.select_in_navigator(&SheetId::new("col-50k"))?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    app.grid.tool.extend_selection(GridCellCoord::new(49_999, 0, 0));

    let mut cache = QuickStatsCache::default();
    let t0 = Instant::now();
    let stats = compute_cached_quick_stats(&app, &mut cache).clone();
    let first_ms = t0.elapsed().as_secs_f64() * 1000.0;

    assert_eq!((stats.n, stats.truncated_at_cap, cache.recompute_count), (QUICK_STATS_CELL_CAP, false, 1));
    assert!((stats.mean.unwrap() - 25_000.5).abs() < 1e-6);
    assert!((stats.median.unwrap() - 25_000.5).abs() < 1e-6);
    assert!((stats.range.unwrap() - 49_999.0).abs() < 1e-6);
    assert!(stats.sd.unwrap() > 14_433.0 && stats.sd.unwrap() < 14_435.0);
    assert!((stats.sem.unwrap() - stats.sd.unwrap() / (50_000.0_f64).sqrt()).abs() < 1e-6);

    let mut miss_samples_ms = Vec::with_capacity(5);
    for shift in 1..=5_usize {
        app.grid.tool.select_single(GridCellCoord::new(shift, 0, 0));
        app.grid.tool.extend_selection(GridCellCoord::new(shift + 49_999, 0, 0));
        let ts = Instant::now();
        let next_stats = compute_cached_quick_stats(&app, &mut cache);
        miss_samples_ms.push(ts.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(next_stats.n, QUICK_STATS_CELL_CAP);
    }
    miss_samples_ms.sort_by(f64::total_cmp);
    let median_budget_ms = if cfg!(debug_assertions) { 2.0 } else { 0.5 };
    assert!(
        first_ms < 25.0 && miss_samples_ms[2] < median_budget_ms,
        "Quick-Stats timing: first={first_ms:.3} ms, median={:.3} ms",
        miss_samples_ms[2]
    );

    let count_before_steady = cache.recompute_count;
    let t1 = Instant::now();
    for _ in 0..1_000 {
        assert_eq!(compute_cached_quick_stats(&app, &mut cache).n, QUICK_STATS_CELL_CAP);
    }
    assert_eq!(cache.recompute_count, count_before_steady);
    assert!(t1.elapsed().as_secs_f64() * 1000.0 < 0.5);

    app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    app.grid.tool.extend_selection(GridCellCoord::new(59_999, 0, 0));
    let capped = compute_cached_quick_stats(&app, &mut cache).clone();
    assert_eq!((capped.n, capped.truncated_at_cap, cache.recompute_count), (QUICK_STATS_CELL_CAP, true, count_before_steady + 1));
    Ok(())
}

#[test]
fn responsive_640x420_1440x600_scrollbar_non_overlap_themes_and_all_14_beta_locales() -> BoxRes {
    let base_app = seed_full_workspace_app()?;
    let long_title = "UltraLongAssayCalibrationConstantThatTriggersStudioEllipsisCorrectlyAcrossLocales";

    for (idx, locale) in BETA_LOCALE_CODES.iter().enumerate() {
        let mode = [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast][idx % 3];
        let mut app = base_app.clone();
        assert_eq!(app.set_locale(locale), *locale);
        app.set_theme_mode(mode);
        let mut doc = app.document().clone();
        if let Some(ds) = doc.data_sheet_mut(&SheetId::new("col-1")) {
            ds.title = long_title.to_owned();
        }
        *app.session_mut() = Session::from_document(doc);
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
        app.sync_views();

        let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), locale);
        let mut driver = make_shell_driver_with_state(ShellTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0), tokens);
        assert_clean_parity(&driver, &format!("{locale}/{mode:?} 1440x600"))?;

        if let Some(sw) = driver.frame_meta().find_by_widget_id("status.sheet_switcher").cloned()
            && sw.ellipsized
        {
            let expected_full = sw.full_label.clone().unwrap_or_default();
            driver.verify_ellipsis_tooltip_and_accesskit("status.sheet_switcher", &expected_full)?;
            let ak = driver.accesskit_tree().ok_or("missing accesskit")?;
            let ak_node = ak.nodes.iter().find(|(id, _)| *id == sw.accesskit_id).map(|(_, n)| n).ok_or("missing sw ak")?;
            assert_eq!(accesskit_accessible_name(ak_node), Some(expected_full.as_str()));
            assert!(collect_tooltip_texts_from_accesskit(&ak.nodes).iter().any(|t| t.contains(&expected_full)));
        }

        driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
        assert_clean_parity(&driver, &format!("{locale}/{mode:?} 640x420"))?;
    }

    for (w, h) in [(1440.0_f32, 600.0), (640.0, 420.0)] {
        let mut ak_app = base_app.clone();
        let _ = ak_app.execute_command("sheet.select", json!({ "sheet_id": "mv-1" }))?;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_fonts(build_driver_font_definitions());
        let (mut views, mut meta, mut intents) = (EguiShellViewState::default(), FrameInspectMeta::default(), PendingUiIntents::new());
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))), ..Default::default() },
            |ui| {
                meta.begin_pass(ui.ctx());
                let _ = render_egui_shell_frame(ui, &ak_app, &mut views, &mut meta, &mut intents);
            },
        );
        out.textures_delta.clear();
        check_accesskit_scrollbar_non_overlap(out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?)?;
    }
    Ok(())
}

#[test]
fn studio_assumptions_wrap_and_offscreen_scroll_buttons_culled_at_1440x600() -> BoxRes {
    let mut app = seed_full_workspace_app()?;
    let mut doc = app.document().clone();
    if let Some(rs) = doc.result_sheet_mut(&SheetId::new("res-anova")) {
        rs.checklist.insert(
            0,
            checklist_item(
                "Repeated-measures condition normality (Shapiro-Wilk)",
                ChecklistStatus::Warn,
                "VeryLongDiagnosticMetricHeader W = 0.8421, p = 0.0142 across treatment conditions",
                "Long assumption diagnostic explanation verifying word wrap inside 280pt Studio card without horizontal overflow.",
            ),
        );
    }
    *app.session_mut() = Session::from_document(doc);

    for sheet_id in ["res-anova", "graph-1", "layout-1"] {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": sheet_id }))?;
        app.sync_views();
        let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), "en");
        let driver = make_shell_driver_with_state(ShellTestState::new(app.clone()), ViewportMetrics::new(1440.0, 600.0, 1.0), tokens);
        assert_clean_parity(&driver, &format!("{sheet_id} studio 1440x600"))?;
    }
    Ok(())
}
