#![forbid(unsafe_code)]

//! Wave 3 (`R15.3`) integration tests for Center Stage 5 modes, Split Comparison, Portfolio Gallery,
//! `FloatingNote` cards, `QOL-7` bottom banners, `ProgressModalState`, `InfoSheet`, `ResultSheet`,
//! async `ChartScene` / `LayoutScene` viewport, `grid_core` 1M-cell zero-clone navigation, and
//! `14`-locale parity.

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ChecklistItem, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, ConstantValue,
    CurveSamplePoint, DataSheet, DescriptiveSpec, EquationDisplayMode, FittedCurveOverlay, FloatingNote, GraphAnnotation, GraphKind, GraphSheet,
    ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoConstant, InfoSheet, InternalLinkKind, JournalWidthPreset, LayoutItem, LayoutSheet,
    LayoutSpec, LinkSource, LinkedDataRegion, NonlinearFitSpec, PairwiseComparisonRecord, ProjectDocument, RectPt, ResultCell, ResultScalarEntry,
    ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetCellRange, SheetFolderKind, SheetId, SurvivalTable, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view};
use curvebook_ui_egui::egui_shell::center_stage::{
    CenterStageContext, CenterStageFrameReport, CenterStageMode, ProgressModalState, SplitPaneSlot, render_center_stage, resolve_center_stage_mode,
};
use curvebook_ui_egui::egui_shell::driver::{
    BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions, panel_header_baseline_parity, split_targeting_parity,
    split_unified_datagrid_parity,
};
use curvebook_ui_egui::egui_shell::info_view::{InfoViewEguiState, render_info_view};
use curvebook_ui_egui::egui_shell::inspect_bridge::HybridCoexistenceArbiter;
use curvebook_ui_egui::egui_shell::results_view::{ResultTabCategory, ResultsViewEguiState, render_results_view};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, LayoutMetrics, PendingUiIntents, ShellLayoutInput, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use egui::{Key, Modifiers, pos2};
use serde_json::json;

use crate::common::{ShellTestState, assert_clean_parity, make_shell_driver, make_shell_driver_with_state, widget_center};

type BoxRes = Result<(), Box<dyn std::error::Error>>;

struct StageHarness {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    report: CenterStageFrameReport,
}

impl StageHarness {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), report: CenterStageFrameReport::default() }
    }
}

fn num_result_cell(v: f64) -> ResultCell {
    ResultCell::Number { value: v, prefix: None, units: None }
}

fn make_column_sheet(id: &str, title: &str, cols: usize, rows: usize) -> Result<DataSheet, Box<dyn std::error::Error>> {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows)?;
    for c in 0..cols {
        table.data_sets[c].title = format!("Group {}", c + 1);
        for r in 0..rows {
            table.data_sets[c].cells[r][0] = Cell::number((r as f64) + (c as f64) * 10.0 + 1.0);
        }
    }
    Ok(DataSheet::new(id, title, TableArchetype::Column(table))?)
}

fn make_full_project_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    let ds = make_column_sheet("col-1", "Dose Cohort", 3, 6)?;

    let mut info = InfoSheet::new_preseeded("info-1", "Experiment Info")?;
    info.constants = vec![
        InfoConstant::new("conc", ConstantValue::Number(12.5), Some(String::from("uM")))?,
        InfoConstant::new("run_date", ConstantValue::Date(String::from("2026-04-10")), None)?,
        InfoConstant::new("operator", ConstantValue::Text(String::from("Dr. Lin")), None)?,
    ];
    info.notes_markdown = String::from("# Assay Setup\nBaseline concentration: {{info:conc}}.");

    let mut rs = ResultSheet::new(AnalysisSpec::new("res-1", "col-1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    rs.status = ResultStatus::UpToDate;
    rs.scalars = vec![ResultScalarEntry { key: String::from("Mean_0"), data_set_index: Some(0), value: 3.5, units: Some(String::from("uM")) }];
    rs.checklist = vec![ChecklistItem {
        assumption: String::from("Shapiro-Wilk normality check"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: Some(String::from("W = 0.86, p = 0.031")),
        explanation: String::from("Moderate positive skew detected; interpret parametric CIs alongside median quartiles."),
        remediation: None,
    }];
    rs.pairwise_comparisons = (0..7)
        .map(|i| PairwiseComparisonRecord {
            group_a: 0,
            group_b: (i % 2) + 1,
            label_a: String::from("Control"),
            label_b: format!("Dose {}", i + 1),
            estimate_diff: Some(2.5 + i as f64),
            raw_p_or_q: 0.004,
            is_fdr_q: false,
            significant: true,
        })
        .collect();
    rs.tabs = vec![
        ResultTab {
            id: String::from("summary"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Estimate"), String::from("Median Survival")],
            row_labels: vec![String::from("Cohort Metrics"), String::from("Control"), String::from("Treated")],
            rows: vec![vec![ResultCell::Empty; 2], vec![num_result_cell(14.2), ResultCell::Empty], vec![num_result_cell(19.8), ResultCell::Empty]],
            footnotes: vec![String::from("Blank median survival cells indicate cohort median not yet reached.")],
        },
        ResultTab {
            id: String::from("params"),
            title: String::from("Fit Params"),
            kind: ResultTabKind::ParametersAndConfidenceIntervals,
            column_headers: vec![String::from("Best-fit"), String::from("95% CI")],
            row_labels: vec![String::from("IC50")],
            rows: vec![vec![
                ResultCell::Number { value: 8.42, prefix: Some(String::from("~")), units: Some(String::from("uM")) },
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.1), upper: CiBound::Open },
            ]],
            footnotes: vec![String::from("Ambiguous parameter highlighted for user inspection.")],
        },
        ResultTab {
            id: String::from("curve_xy"),
            title: String::from("Curve XY"),
            kind: ResultTabKind::CurveCoordinates,
            column_headers: vec![String::from("X"), String::from("Predicted Y")],
            row_labels: vec![String::from("Pt 1"), String::from("Pt 2")],
            rows: vec![vec![num_result_cell(0.0), num_result_cell(10.0)], vec![num_result_cell(1.0), num_result_cell(25.0)]],
            footnotes: Vec::new(),
        },
    ];

    let mut gs = GraphSheet::new_uninitialized_for_data("g-1", &ds)?;
    gs.title = String::from("Dose Response Plot");
    gs.initialized = true;
    gs.floating_notes.push(FloatingNote::new("n1", RectPt::new(16.0, 20.0, 190.0, 96.0), "Active conc={{info:conc}} and slope hook."));
    gs.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(20.0, 20.0, 110.0, 32.0),
        markdown: String::from("Reference {{info:conc}}"),
        bordered: true,
    });
    gs.spec.series[0].legend_label = Some(String::from("Dose @ {{info:conc}}"));

    let mut layout_spec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    layout_spec.items = [("panel-a", 10.0), ("panel-b", 130.0)]
        .into_iter()
        .map(|(id, x)| LayoutItem::LiveGraph {
            id: id.to_owned(),
            graph_sheet_id: SheetId::new("g-1"),
            bounds_pt: RectPt::new(x, 10.0, 110.0, 90.0),
            scale_percent: 100.0,
            badge_override: None,
        })
        .collect();
    let ls = LayoutSheet::new("lay-1", "Figure 1 Layout", layout_spec)?;

    doc.data_sheets = vec![ds];
    doc.info_sheets = vec![info];
    doc.result_sheets = vec![rs];
    doc.graph_sheets = vec![gs];
    doc.layout_sheets = vec![ls];
    doc.active_sheet_id = Some(SheetId::new("col-1"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    Ok(app)
}

fn make_stage_driver<'a>(state: StageHarness, metrics: ViewportMetrics, tokens: Tokens) -> EguiHumanPathDriver<'a, StageHarness> {
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(state, metrics, tokens, build_driver_font_definitions(), |ui, meta, st| {
        let decision = HybridCoexistenceArbiter::evaluate(&st.app, false);
        let layout = st.views.last_shell_layout.unwrap_or_else(|| {
            LayoutMetrics::default().solve_shell_layout(ShellLayoutInput {
                viewport_w_pt: ui.available_width(),
                viewport_h_pt: ui.available_height(),
                empty_workspace: false,
                navigator_open: true,
                inspector_open: true,
                family_open: false,
                leading_user_collapsed: false,
                trailing_user_collapsed: false,
                leading_width_override_pt: None,
                trailing_width_override_pt: None,
                host_draws_menu_bar: false,
                is_rtl: curvebook_ui_egui::i18n::is_rtl_locale(&st.app.locale),
            })
        });
        let tokens = Tokens::for_mode(st.app.theme.mode);
        let ctx_in = CenterStageContext {
            tokens: &tokens,
            decision: &decision,
            layout,
            focused_pane: st.app.nav.active_pane,
            enabled: matches!(st.app.canvas_dialogs.active_dialog, ActiveDialog::None) && st.views.progress_modal.is_none(),
        };
        st.report = render_center_stage(ui, &st.app, &mut st.views, ctx_in, meta, &mut st.intents);
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            assert!(st.intents.commit_if_settled(&mut st.app, false).is_ok());
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

#[test]
fn center_stage_five_modes_and_split_15_to_85_clamping_with_brushing() -> BoxRes {
    let mut app = make_full_project_app()?;
    {
        let mut empty_doc = app.document().clone();
        empty_doc.active_sheet_id = None;
        assert_eq!(resolve_center_stage_mode(&UiApp::from_session(Session::from_document(empty_doc)), &app.egui_views), CenterStageMode::Welcome);
    }
    for (sid, mode) in [
        ("col-1", CenterStageMode::ColumnGrid),
        ("info-1", CenterStageMode::InfoSheet),
        ("res-1", CenterStageMode::ResultSheet),
        ("g-1", CenterStageMode::CanvasViewport),
        ("lay-1", CenterStageMode::CanvasViewport),
    ] {
        app.execute_command("sheet.select", json!({ "sheet_id": sid }))?;
        assert_eq!(resolve_center_stage_mode(&app, &app.egui_views), mode, "{sid}");
    }

    app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("col-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("g-1"));
    app.sync_views();

    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(1280.0, 720.0, 1.0), Tokens::default());
    assert!(driver.state().report.handled_natively);
    assert!(driver.state().report.primary_rect_pt.expect("prim").right() < driver.state().report.secondary_rect_pt.expect("sec").left());
    assert!(
        driver.frame_meta().find_by_widget_id("split.primary_selector").is_some()
            && driver.frame_meta().find_by_widget_id("split.secondary_selector").is_some()
    );

    for (target_x, expected_ratio, tol) in [(10.0, 0.15_f32, 0.02), (1270.0, 0.85, 0.02)] {
        let div = widget_center(&driver, "split.divider")?;
        driver.drag(div, pos2(target_x, div.y))?;
        assert!((driver.state().views.center_stage.split_ratio_override.expect("ratio") - expected_ratio).abs() < tol);
    }
    let div = widget_center(&driver, "split.divider")?;
    driver.click_at(div)?;
    driver.click_at(div)?;
    assert!((driver.state().views.center_stage.split_ratio_override.expect("reset ratio") - 0.40).abs() < 0.05);

    driver.set_viewport(ViewportMetrics::new(640.0, 620.0, 1.0));
    driver.run_settled();
    assert!(driver.state().report.primary_rect_pt.expect("v_prim").bottom() < driver.state().report.secondary_rect_pt.expect("v_sec").top());
    driver.set_viewport(ViewportMetrics::new(1280.0, 720.0, 1.0));
    driver.run_settled();

    driver.click_widget_id("grid.cell.2.1.0")?;
    let b1 = driver.state().app.brushed_cell.clone().expect("brushed1");
    assert_eq!((b1.row, b1.col), (2, 1));

    driver.click_widget_id("split.swap")?;
    assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(SheetId::as_str), Some("g-1"));
    assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));

    driver.click_widget_id("grid.cell.3.0.0")?;
    let b2 = driver.state().app.brushed_cell.clone().expect("brushed2");
    assert_eq!((b2.row, b2.col), (3, 0));

    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;
    driver.run_settled();
    driver.click_widget_id("grid.cell.4.0.0")?;
    let b3 = driver.state().app.brushed_cell.clone().expect("brushed3");
    assert_eq!((b3.row, b3.col), (4, 0));

    driver.click_widget_id("split.close")?;
    assert!(!driver.state().app.split.enabled && !driver.state().app.split.inspector_pinned);
    Ok(())
}

#[test]
fn r20_3_three_way_split_targeting_shared_headers_and_compact_flip() -> BoxRes {
    let mut app = make_full_project_app()?;
    let (col_id, g_id) = (SheetId::new("col-1"), SheetId::new("g-1"));
    app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
    let initial_family_anchor = app.document().family_anchor_sheet_id.clone();

    let mut driver =
        make_shell_driver_with_state(ShellTestState::new(app).with_commit_all(), ViewportMetrics::new(1440.0, 800.0, 1.0), Tokens::default());
    driver.click_widget_id("toolbar.split")?;
    driver.click_widget_id("toolbar.split.preset.data_graph")?;
    assert!(driver.state().app.split.enabled && driver.state().app.split.inspector_pinned);
    assert_eq!(driver.state().app.split.inspector_target_sheet_id, Some(g_id.clone()));
    assert!((driver.state().app.split.ratio - 0.40).abs() < 1e-6);
    assert!(panel_header_baseline_parity(driver.frame_meta()).is_empty());
    assert!(split_targeting_parity(&driver.state().app).is_empty());
    assert!(split_unified_datagrid_parity(driver.frame_meta()).is_empty());

    driver.click_widget_id("split.swap")?;
    assert_eq!(
        (&driver.state().app.split.primary_sheet_id, &driver.state().app.split.secondary_sheet_id),
        (&Some(g_id.clone()), &Some(col_id.clone()))
    );
    assert_eq!(driver.state().app.document().active_sheet_id, Some(col_id));
    assert!(driver.state().app.split.inspector_pinned && driver.state().app.split.inspector_target_sheet_id == Some(g_id.clone()));
    assert_eq!(driver.state().app.document().family_anchor_sheet_id, initial_family_anchor);

    driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
    driver.run_settled();
    assert!(
        driver.frame_meta().find_by_widget_id("split.compact.primary").is_some()
            && driver.frame_meta().find_by_widget_id("split.compact.secondary").is_some()
    );
    driver.click_widget_id("split.compact.primary")?;
    assert_eq!(driver.state().app.document().active_sheet_id, Some(g_id));
    assert_eq!(driver.state().app.document().family_anchor_sheet_id, initial_family_anchor);
    assert!(driver.state().app.split.inspector_pinned);

    driver.click_widget_id("split.close")?;
    assert!(!driver.state().app.split.enabled && !driver.state().app.split.inspector_pinned);
    assert!(split_targeting_parity(&driver.state().app).is_empty());
    Ok(())
}

#[test]
fn center_stage_portfolio_gallery_search_filter_and_open() -> BoxRes {
    let mut app = make_full_project_app()?;
    app.nav.gallery.open = true;
    let doc_clone = app.document().clone();
    app.nav.sync_gallery(&doc_clone);
    app.sync_views();

    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(1120.0, 700.0, 1.0), Tokens::default());
    assert_eq!(driver.state().views.center_stage.last_active_mode, CenterStageMode::Gallery);
    assert!(
        driver.frame_meta().find_by_widget_id("gallery.card.g-1").is_some() && driver.frame_meta().find_by_widget_id("gallery.card.lay-1").is_some()
    );

    driver.click_widget_id("gallery.filter.graphs")?;
    assert_eq!(driver.state().app.nav.gallery.filter_folder, Some(SheetFolderKind::Graphs));
    assert!(
        driver.frame_meta().find_by_widget_id("gallery.card.g-1").is_some() && driver.frame_meta().find_by_widget_id("gallery.card.lay-1").is_none()
    );

    driver.click_widget_id("gallery.filter.all")?;
    driver.click_widget_id("gallery.search_input")?;
    driver.type_text("Figure")?;
    assert!(
        driver.frame_meta().find_by_widget_id("gallery.card.lay-1").is_some() && driver.frame_meta().find_by_widget_id("gallery.card.g-1").is_none()
    );

    driver.click_widget_id("gallery.card.lay-1")?;
    assert!(!driver.state().app.nav.gallery.open);
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("lay-1"));
    Ok(())
}

#[test]
fn center_stage_f6_cycles_split_panes_and_floating_notes() -> BoxRes {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("col-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("g-1"));
    app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    app.sync_views();

    let mut driver = make_shell_driver(app, ViewportMetrics::new(1360.0, 820.0, 1.0));
    assert_eq!(
        (driver.state().app.nav.active_pane, driver.state().views.center_stage.focused_split_pane),
        (ActiveWorkspacePane::MainCanvasOrGrid, SplitPaneSlot::Primary)
    );

    driver.press_egui_key(Key::F6, Modifiers::NONE);
    assert_eq!(
        (driver.state().app.nav.active_pane, driver.state().views.center_stage.focused_split_pane),
        (ActiveWorkspacePane::MainCanvasOrGrid, SplitPaneSlot::Secondary)
    );

    driver.press_egui_key(Key::F6, Modifiers::NONE);
    assert_ne!(driver.state().app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);

    let mut seen_notes = driver.state().app.nav.active_pane == ActiveWorkspacePane::FloatingNotes;
    for _ in 0..4 {
        driver.press_egui_key(Key::F6, Modifiers::NONE);
        if driver.state().app.nav.active_pane == ActiveWorkspacePane::FloatingNotes {
            seen_notes = true;
            break;
        }
    }
    assert!(seen_notes);

    for _ in 0..4 {
        if driver.state().app.nav.active_pane == ActiveWorkspacePane::MainCanvasOrGrid {
            break;
        }
        driver.press_egui_key(Key::F6, Modifiers::SHIFT);
    }
    assert_eq!(
        (driver.state().app.nav.active_pane, driver.state().views.center_stage.focused_split_pane),
        (ActiveWorkspacePane::MainCanvasOrGrid, SplitPaneSlot::Secondary)
    );
    driver.press_egui_key(Key::F6, Modifiers::SHIFT);
    assert_eq!(
        (driver.state().app.nav.active_pane, driver.state().views.center_stage.focused_split_pane),
        (ActiveWorkspacePane::MainCanvasOrGrid, SplitPaneSlot::Primary)
    );
    Ok(())
}

#[test]
fn center_stage_floating_note_clamped_reposition_resize_pin_collapse() -> BoxRes {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;
    app.sync_views();
    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(960.0, 620.0, 1.0), Tokens::default());
    let stage = driver.state().report.primary_rect_pt.expect("stage rect");

    let drag_pt = widget_center(&driver, "floating_note.g-1.n1.drag_handle")?;
    driver.drag(drag_pt, pos2(stage.right() + 400.0, stage.bottom() + 400.0))?;
    let card = driver.frame_meta().find_by_widget_id("floating_note.g-1.n1.card").expect("card").clipped_rect;
    assert!(
        card.right() <= stage.right() + 1.0
            && card.bottom() <= stage.bottom() + 1.0
            && card.left() >= stage.left() - 1.0
            && card.top() >= stage.top() - 1.0
    );

    let resize_pt = widget_center(&driver, "floating_note.g-1.n1.resize_handle")?;
    driver.drag(resize_pt, pos2(stage.right() + 200.0, stage.bottom() + 200.0))?;
    let resized = driver.frame_meta().find_by_widget_id("floating_note.g-1.n1.card").expect("resized").clipped_rect;
    assert!(resized.right() <= stage.right() + 1.0 && resized.bottom() <= stage.bottom() + 1.0);

    driver.click_widget_id("floating_note.g-1.n1.pin")?;
    assert_eq!(driver.state().views.center_stage.note_pinned.get(&(SheetId::new("g-1"), String::from("n1"))).copied(), Some(true));
    driver.click_widget_id("floating_note.g-1.n1.collapse")?;
    assert!(driver.frame_meta().find_by_widget_id("floating_note.g-1.n1.card").expect("collapsed").clipped_rect.height() <= 28.0);
    Ok(())
}

#[test]
fn center_stage_qol7_bottom_banners_diagnostic_clipping_and_linked() -> BoxRes {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "res-1" }))?;
    app.sync_views();
    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(1120.0, 680.0, 1.0), Tokens::default());
    assert!((28.0..=36.0).contains(&driver.state().report.diagnostic_banner_rect_pt.expect("banner rect").height()));
    assert!(driver.frame_meta().find_by_widget_id("result.diagnostic_banner").is_some());
    driver.click_widget_id("result.diagnostic_banner.why")?;
    assert_eq!(driver.state().views.center_stage.banner_details_open.as_ref().map(SheetId::as_str), Some("res-1"));
    driver.click_widget_id("result.diagnostic_banner.dismiss")?;
    assert!(driver.state().report.diagnostic_banner_rect_pt.is_none());

    let mut app2 = make_full_project_app()?;
    let mut doc2 = app2.document().clone();
    doc2.active_sheet_id = Some(SheetId::new("g-1"));
    doc2.graph_sheets[0].spec.x_axis.segments[0].min = Some(50.0);
    doc2.graph_sheets[0].spec.x_axis.segments[0].max = Some(10.0);
    *app2.session_mut() = Session::from_document(doc2);
    app2.sync_views();
    let mut d2 = make_stage_driver(StageHarness::new(app2), ViewportMetrics::new(1120.0, 680.0, 1.0), Tokens::default());
    assert!(d2.frame_meta().find_by_widget_id("canvas.clipping_banner").is_some());
    d2.click_widget_id("canvas.clipping_banner.fit_range")?;
    let seg = &d2.state().app.document().graph_sheet(&SheetId::new("g-1")).unwrap().spec.x_axis.segments[0];
    assert_eq!((seg.min, seg.max), (None, None));

    let mut app3 = make_full_project_app()?;
    let mut doc3 = app3.document().clone();
    doc3.data_sheets.push(make_column_sheet("col-src", "Source Cohort", 2, 4)?);
    let range = SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 0));
    doc3.data_sheets[0].linked_regions.push(LinkedDataRegion {
        id: String::from("link-1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet { sheet_id: SheetId::new("col-src"), kind: InternalLinkKind::DataSheet, tab_id: None, range },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: range,
        provenance_summary: String::from("From Source Cohort"),
    });
    doc3.active_sheet_id = Some(SheetId::new("col-1"));
    *app3.session_mut() = Session::from_document(doc3);
    app3.sync_views();
    let mut d3 = make_stage_driver(StageHarness::new(app3), ViewportMetrics::new(1120.0, 680.0, 1.0), Tokens::default());
    assert!(d3.frame_meta().find_by_widget_id("canvas.linked_banner").is_some());
    d3.click_widget_id("canvas.linked_banner.unlink")?;
    assert!(d3.state().app.document().data_sheet(&SheetId::new("col-1")).unwrap().linked_regions.is_empty());
    Ok(())
}

#[test]
fn center_stage_progress_modal_chrono_free_indeterminate_determinate_cancel() -> BoxRes {
    let mut app = make_full_project_app()?;
    app.egui_views.progress_modal = Some(ProgressModalState {
        operation_label: String::from("Opening CurveBook Archive"),
        detail: String::from("Parsing 48 sheets…"),
        progress_0_to_1: None,
        cancellable: true,
        cancel_requested: false,
    });
    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(960.0, 600.0, 1.0), Tokens::default());
    assert!(
        driver.frame_meta().find_by_widget_id("modal.progress.backdrop").is_some()
            && driver.frame_meta().find_by_widget_id("modal.progress.card").is_some()
    );
    driver.state_mut().views.progress_modal.as_mut().unwrap().progress_0_to_1 = Some(0.42);
    driver.run_settled();
    driver.click_widget_id("modal.progress.cancel")?;
    assert!(driver.state().views.progress_modal.as_ref().unwrap().cancel_requested);
    Ok(())
}

#[test]
fn info_sheet_38_62_split_hook_badges_inline_edit_and_formatted_notes() -> BoxRes {
    struct InfoHarness {
        app: UiApp,
        info_view: InfoViewEguiState,
        intents: PendingUiIntents,
        table_w: f32,
        notes_w: f32,
        hooks: usize,
    }
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "info-1" }))?;
    app.sync_views();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        InfoHarness { app, info_view: InfoViewEguiState::default(), intents: PendingUiIntents::new(), table_w: 0.0, notes_w: 0.0, hooks: 0 },
        ViewportMetrics::new(1000.0, 640.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let tokens = Tokens::for_mode(st.app.theme.mode);
            let rep = render_info_view(ui, &st.app, &mut st.info_view, &tokens, true, meta, &mut st.intents);
            st.table_w = rep.table_rect_pt.map_or(0.0, |r| r.width());
            st.notes_w = rep.notes_rect_pt.map_or(0.0, |r| r.width());
            st.hooks = rep.hook_badge_count;
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            assert!(st.intents.commit_if_settled(&mut st.app, false).is_ok());
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    let frac = driver.state().table_w / (driver.state().table_w + driver.state().notes_w).max(1.0);
    assert!((frac - 0.38).abs() < 0.03 && driver.state().hooks > 0);

    let div_pt = widget_center(&driver, "info.split_divider")?;
    driver.drag(div_pt, pos2(div_pt.x + 120.0, div_pt.y))?;
    assert!(driver.state().info_view.split_ratio > 0.45);
    let div_pt2 = widget_center(&driver, "info.split_divider")?;
    driver.click_at(div_pt2)?;
    driver.click_at(div_pt2)?;
    assert!((driver.state().info_view.split_ratio - 0.38).abs() < 0.02);

    driver.click_widget_id("info.const.0")?;
    driver.click_widget_id("info.const.0")?;
    assert_eq!(driver.state().info_view.editing_key.as_deref(), Some("conc"));
    driver.state_mut().info_view.editing_value = String::from("42.5");
    driver.run_settled();
    driver.click_widget_id("info.const.editor_save")?;
    assert_eq!(
        driver.state().app.document().info_sheet(&SheetId::new("info-1")).unwrap().get_constant("conc").unwrap().value,
        ConstantValue::Number(42.5)
    );

    driver.click_widget_id("info.format.bold")?;
    driver.click_widget_id("info.notes.preview_toggle")?;
    assert!(driver.state().info_view.notes_preview_mode);
    driver.click_widget_id("info.notes.preview_toggle")?;
    driver.click_widget_id("info.notes.save")?;
    assert!(driver.state().app.document().info_sheet(&SheetId::new("info-1")).unwrap().notes_markdown.contains("**bold**"));

    driver.click_widget_id("info.hook_badge.0")?;
    assert_ne!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("info-1"));
    Ok(())
}

#[test]
fn result_sheet_multi_tab_cmd_t_navigation_ambiguity_blank_median_and_extract() -> BoxRes {
    struct ResHarness {
        app: UiApp,
        views: ResultsViewEguiState,
        intents: PendingUiIntents,
        header_h: f32,
        category: Option<ResultTabCategory>,
        has_ambiguity: bool,
        saw_nan: bool,
    }
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "res-1" }))?;
    app.sync_views();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ResHarness {
            app,
            views: ResultsViewEguiState::default(),
            intents: PendingUiIntents::new(),
            header_h: 0.0,
            category: None,
            has_ambiguity: false,
            saw_nan: false,
        },
        ViewportMetrics::new(1040.0, 640.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let tokens = Tokens::for_mode(st.app.theme.mode);
            let rep = render_results_view(ui, &st.app, &mut st.views, &tokens, true, meta, &mut st.intents);
            st.header_h = rep.header_rect_pt.map_or(0.0, |r| r.height());
            st.category = rep.active_tab_category;
            st.has_ambiguity = rep.has_ambiguity_callout;
            st.saw_nan |= meta.entries.iter().any(|e| e.label.contains("NaN"));
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            assert!(st.intents.commit_if_settled(&mut st.app, false).is_ok());
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    assert!(
        (driver.state().header_h - 32.0).abs() < 1.0
            && driver.state().category == Some(ResultTabCategory::InferentialReport)
            && !driver.state().saw_nan
    );

    driver.click_widget_id("result.section.0")?;
    assert!(!driver.state().views.collapsed_sections.is_empty());
    driver.click_widget_id("result.section.0")?;
    assert!(driver.state().views.collapsed_sections.is_empty());

    let tab1_id = driver.state().app.document().result_sheet(&SheetId::new("res-1")).unwrap().tabs[1].id.clone();
    driver.click_widget_id(&format!("result.tab.{tab1_id}"))?;
    assert_eq!(driver.state().app.canvas_dialogs.result_view.active_tab_index, 1);
    assert!(driver.state().has_ambiguity && driver.frame_meta().find_by_widget_id("result.ambiguity_callout").is_some());

    driver.press_egui_key(Key::PageDown, Modifiers::CTRL);
    assert_eq!(driver.state().app.canvas_dialogs.result_view.active_tab_index, 2);
    assert_eq!(driver.state().category, Some(ResultTabCategory::PlotReadyMatrix));
    assert!(driver.frame_meta().find_by_widget_id("result.new_graph_from_tab").is_some());

    driver.press_egui_key(Key::PageUp, Modifiers::CTRL);
    assert_eq!(driver.state().app.canvas_dialogs.result_view.active_tab_index, 1);
    driver.click_widget_id("result.edit_params")?;
    assert_ne!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn canvas_view_async_raster_hover_pairwise_cld_km_tex_layout_drilldown() -> BoxRes {
    let mut app = make_full_project_app()?;
    let mut surv = SurvivalTable::new(2, 4)?;
    surv.cohorts[0].title = String::from("Placebo");
    surv.cohorts[1].title = String::from("Drug");
    for (r, &t) in [2.0, 4.0, 7.0, 10.0].iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(1.0);
        surv.cohorts[1].cells[r][0] = Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 });
    }
    let surv_ds = DataSheet::new("surv-1", "Survival Study", TableArchetype::Survival(surv))?;
    let mut surv_gs = GraphSheet::new_uninitialized_for_data("g-surv", &surv_ds)?;
    surv_gs.title = String::from("KM Plot");
    surv_gs.initialized = true;
    surv_gs.spec.survival_number_at_risk = true;

    let mut nl_rs = ResultSheet::new(AnalysisSpec::new(
        "res-nl",
        "col-1",
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_inhibitor_vs_response_3pl")),
    )?)?;
    nl_rs.status = ResultStatus::UpToDate;
    nl_rs.scalars = [("Bottom", 5.0), ("Top", 95.0), ("LogIC50", -6.2)]
        .into_iter()
        .map(|(k, v)| ResultScalarEntry { key: k.to_owned(), data_set_index: Some(0), value: v, units: None })
        .collect();
    nl_rs.fitted_curves = vec![FittedCurveOverlay {
        equation_id: String::from("log_inhibitor_vs_response_3pl"),
        converged: true,
        ambiguous: false,
        data_set_index: 0,
        points: vec![CurveSamplePoint { x: 0.0, y: 95.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None }],
    }];

    {
        let mut doc = app.document().clone();
        doc.data_sheets.push(surv_ds);
        doc.result_sheets.push(nl_rs);
        doc.graph_sheets[0].spec.annotations.push(GraphAnnotation::ModelEquation {
            result_sheet_id: SheetId::new("res-nl"),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(140.0, 28.0, 140.0, 48.0),
            font_size_pt: 11.0,
            bordered: true,
        });
        doc.graph_sheets[0].spec.comparisons = (0..7)
            .map(|i| curvebook_doc::PairwiseBracketSpec {
                result_sheet_id: SheetId::new("res-1"),
                group_a: 0,
                group_b: (i % 2) + 1,
                style: Default::default(),
                label_mode: Default::default(),
                hide_if_non_significant: false,
                stroke_width_pt: 1.0,
                font_size_pt: 9.0,
                bracket_step_pt: 10.0,
            })
            .collect();
        doc.graph_sheets.push(surv_gs);
        doc.active_sheet_id = Some(SheetId::new("g-1"));
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }

    struct CanvasHarness {
        app: UiApp,
        view: CanvasViewEguiState,
        intents: PendingUiIntents,
        paper_rect: Option<egui::Rect>,
        km_rect: Option<egui::Rect>,
        eq_count: usize,
        breadcrumb: bool,
    }
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        CanvasHarness {
            app,
            view: CanvasViewEguiState::default(),
            intents: PendingUiIntents::new(),
            paper_rect: None,
            km_rect: None,
            eq_count: 0,
            breadcrumb: false,
        },
        ViewportMetrics::new(1120.0, 740.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let tokens = Tokens::for_mode(st.app.theme.mode);
            let rep = render_canvas_view(
                ui,
                &st.app,
                &mut st.view,
                CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false },
                meta,
                &mut st.intents,
            );
            st.paper_rect = rep.paper_rect_pt;
            st.km_rect = rep.km_track_rect_pt;
            st.eq_count = rep.tex_equation_rects_pt.len();
            st.breadcrumb = rep.layout_breadcrumb_shown;
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            assert!(st.intents.commit_if_settled(&mut st.app, false).is_ok());
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    for _ in 0..40 {
        if driver.state().view.slot.cached_texture.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        driver.run_settled();
    }
    assert!(driver.state().view.slot.cached_texture.is_some() && driver.state().eq_count >= 1);
    assert!(driver.frame_meta().find_by_widget_id("canvas.tex_equation.0").is_some());
    assert_clean_parity(&driver, "canvas.tex_equation")?;

    let paper = driver.state().paper_rect.expect("paper rect");
    let scene = driver.state().app.session().render_sheet_scene(&SheetId::new("g-1"))?;
    let first_pt = scene.plotted_points.first().expect("plotted point");
    let screen_pt = pos2(
        paper.left() + (first_pt.center_pt.x as f32 / scene.width_pt as f32) * paper.width(),
        paper.top() + (first_pt.center_pt.y as f32 / scene.height_pt as f32) * paper.height(),
    );
    driver.hover_at(screen_pt)?;
    assert_eq!(driver.state().view.hover_readout.clone().expect("hover").point_index, Some(0));
    driver.click_at(screen_pt)?;
    assert_eq!(driver.state().app.brushed_cell.as_ref().map(|c| c.row), Some(first_pt.source_cell.row));

    assert!(driver.frame_meta().find_by_widget_id("canvas.pairwise_cld_recommend").is_some());
    driver.click_widget_id("canvas.pairwise_cld_recommend")?;
    assert!(driver.state().app.document().graph_sheet(&SheetId::new("g-1")).unwrap().spec.compact_letter_display.is_some());

    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "g-surv" }))?;
    driver.run_settled();
    assert!(
        driver.state().km_rect.is_some()
            && driver.frame_meta().find_by_widget_id("canvas.km_number_at_risk").is_some()
            && driver.state().view.km_at_risk_rows >= 2
    );
    assert_clean_parity(&driver, "canvas.km_number_at_risk")?;

    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" }))?;
    driver.run_settled();
    for _ in 0..24 {
        if !driver.state().view.slot.rendering_in_flight {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        driver.run_settled();
    }
    let p0_center = widget_center(&driver, "canvas.layout_panel.0")?;
    driver.drag(p0_center, pos2(p0_center.x + 8.0, p0_center.y + 1.0))?;
    assert!(driver.state().view.last_snap_readout.as_deref().is_some_and(|s| s.contains("0.1 pt")));

    let p0_center2 = widget_center(&driver, "canvas.layout_panel.0")?;
    driver.dispatch_events(vec![
        egui::Event::PointerMoved(p0_center2),
        egui::Event::PointerButton { pos: p0_center2, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        egui::Event::PointerButton { pos: p0_center2, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    driver.dispatch_events(vec![
        egui::Event::PointerButton { pos: p0_center2, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        egui::Event::PointerButton { pos: p0_center2, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("g-1"));
    assert!(driver.state().breadcrumb);

    driver.click_widget_id("canvas.return_to_layout")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("lay-1"));
    Ok(())
}

#[test]
fn canvas_view_heatmap_colorbar_hover_readout() -> BoxRes {
    let mut app = make_full_project_app()?;
    let ds = app.document().data_sheet(&SheetId::new("col-1")).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-heat", ds)?;
    gs.title = String::from("Corr Heatmap");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Diagnostic(curvebook_doc::DiagnosticPlotKind::Heatmap);
    let mut doc = app.document().clone();
    doc.graph_sheets.push(gs);
    doc.active_sheet_id = Some(SheetId::new("g-heat"));
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();

    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, CanvasViewEguiState::default(), PendingUiIntents::new(), None::<egui::Rect>, false),
        ViewportMetrics::new(980.0, 680.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (app, view, intents, paper_out, cb_hovered)| {
            let tokens = Tokens::for_mode(app.theme.mode);
            let rep =
                render_canvas_view(ui, app, view, CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false }, meta, intents);
            *paper_out = rep.paper_rect_pt;
            *cb_hovered = rep.colorbar_hovered;
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();

    let paper = driver.state().3.expect("heatmap paper rect");
    let scene = driver.state().0.session().render_sheet_scene(&SheetId::new("g-heat"))?;
    let cb_rect = scene.primitives.iter().find_map(|p| {
        let curvebook_chart::ScenePrimitive::Path { path, fill: Some(_), .. } = p else { return None };
        let pts: Vec<(f64, f64)> = path
            .commands
            .iter()
            .filter_map(|c| match c {
                curvebook_chart::PathCmd::MoveTo(pt) | curvebook_chart::PathCmd::LineTo(pt) => Some((pt.x, pt.y)),
                _ => None,
            })
            .collect();
        if pts.len() < 4 {
            return None;
        }
        let (x0, x1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &(x, _)| (lo.min(x), hi.max(x)));
        let (y0, y1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &(_, y)| (lo.min(y), hi.max(y)));
        (x1 - x0 <= 24.0 && y1 - y0 >= 40.0 && x0 >= scene.plot_rect_pt.x + scene.plot_rect_pt.width * 0.5).then_some((x0, x1, y0, y1))
    });

    if let Some((x0, x1, y0, y1)) = cb_rect {
        let cx = ((x0 + x1) * 0.5) as f32 / scene.width_pt as f32 * paper.width() + paper.left();
        let cy = ((y0 + y1) * 0.5) as f32 / scene.height_pt as f32 * paper.height() + paper.top();
        driver.hover_at(pos2(cx, cy))?;
        assert!(driver.state().4);
        assert!(driver.state().1.hover_readout.as_ref().is_some_and(|h| h.value_summary.contains('#')));
    }
    Ok(())
}

#[test]
fn grid_core_arrow_down_nav_zero_1m_clone_fast() -> BoxRes {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1_000, 1_000)?;
    for c in 0..4 {
        for r in 0..20 {
            table.data_sets[c].cells[r][0] = Cell::number((r + 1) as f64);
        }
    }
    let doc = ProjectDocument {
        data_sheets: vec![DataSheet::new("col-1m", "One Million Cell Sheet", TableArchetype::Column(table))?],
        active_sheet_id: Some(SheetId::new("col-1m")),
        ..Default::default()
    };
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();

    let start = Instant::now();
    let mut intents = PendingUiIntents::new();
    for _ in 0..10 {
        intents.push(curvebook_ui_egui::egui_shell::UiIntent::GridNavigate {
            dir: curvebook_ui_egui::grid_view::MoveDirection::Down,
            extend: false,
            jump: false,
        });
        let _ = intents.commit_if_settled(&mut app, false)?;
    }
    assert_eq!(app.grid.tool.active.map(|c| c.row), Some(10));
    assert!(start.elapsed().as_secs_f64() < 2.0);
    Ok(())
}

#[test]
fn all_14_beta_locales_and_viewports_pass_parity_and_scrollbar_non_overlap() -> BoxRes {
    for locale in BETA_LOCALE_CODES {
        for target in ["info-1", "res-1", "g-1", "gallery"] {
            for vp in [ViewportMetrics::new(640.0, 420.0, 1.0), ViewportMetrics::new(1440.0, 600.0, 1.0)] {
                let mut app = make_full_project_app()?;
                assert_eq!(app.set_locale(locale), locale);
                if target == "gallery" {
                    app.nav.gallery.open = true;
                    let doc_clone = app.document().clone();
                    app.nav.sync_gallery(&doc_clone);
                } else {
                    app.execute_command("sheet.select", json!({ "sheet_id": target }))?;
                }
                app.sync_views();
                let driver = make_stage_driver(StageHarness::new(app), vp, Tokens::for_mode(curvebook_ui_egui::UiThemeMode::Light));
                assert_clean_parity(&driver, &format!("locale={locale} mode={target} vp={vp:?}"))?;
            }
        }
    }
    Ok(())
}

#[test]
fn result_sheet_wide_tabs_many_sections_and_clipped_columns_pass_parity() -> BoxRes {
    let mut app = make_full_project_app()?;
    let titles = [
        "ANOVA summary",
        "Multiple Comparisons — BH FDR-adjusted q values",
        "Comparison of survival curves & hazard ratios",
        "Homoscedasticity & Residual Diagnostics",
        "Bootstrap Parameter Confidence Intervals",
        "Covariance & Leverage Diagnostics",
    ];
    let wide_tabs: Vec<ResultTab> = titles
        .into_iter()
        .enumerate()
        .map(|(i, title)| {
            let (mut row_labels, mut rows) = (Vec::new(), Vec::new());
            for r in 0..28 {
                if r % 5 == 0 {
                    row_labels.push(format!("Section {r} — Omnibus Diagnostic Metrics & Subgroup Totals"));
                    rows.push(vec![ResultCell::Empty; 14]);
                } else {
                    row_labels.push(format!("Long Diagnostic Metric Row Label Number {r}"));
                    rows.push((0..14).map(|c| num_result_cell(100.1234 + (r * 10 + c) as f64)).collect());
                }
            }
            ResultTab {
                id: format!("wide-tab-{i}"),
                title: title.to_owned(),
                kind: if i % 2 == 0 { ResultTabKind::MainSummary } else { ResultTabKind::MultipleComparisons },
                column_headers: (0..14).map(|c| format!("Parameter Column {c}")).collect(),
                row_labels,
                rows,
                footnotes: vec![String::from("Footnote: Extended multi-line explanatory note verifying horizontal wrapping inside ResultSheet body without horizontal clip bleed.")],
            }
        })
        .collect();
    let mut doc = app.document().clone();
    if let Some(rs) = doc.result_sheet_mut(&SheetId::new("res-1")) {
        rs.tabs = wide_tabs;
    }
    *app.session_mut() = Session::from_document(doc);
    app.execute_command("sheet.select", json!({ "sheet_id": "res-1" }))?;
    app.sync_views();

    for vp in [ViewportMetrics::new(640.0, 420.0, 1.0), ViewportMetrics::new(1440.0, 600.0, 1.0)] {
        let mut stage_app = app.clone();
        stage_app.sync_views();
        let mut driver = make_stage_driver(StageHarness::new(stage_app), vp, Tokens::for_mode(curvebook_ui_egui::UiThemeMode::Light));
        assert_clean_parity(&driver, &format!("result sheet {vp:?}"))?;
        for entry in &driver.frame_meta().entries {
            if entry.widget_id.starts_with("result.tab.") && !entry.widget_id.starts_with("result.tab.overflow.") {
                assert!(entry.clipped_rect.width() <= 140.0);
            }
        }
        if driver.frame_meta().find_by_widget_id("result.tabs.overflow").is_some_and(|e| e.enabled) {
            driver.click_widget_id("result.tabs.overflow")?;
            assert_clean_parity(&driver, &format!("overflow open {vp:?}"))?;
            let first_ov = driver
                .frame_meta()
                .entries
                .iter()
                .find(|e| e.widget_id.starts_with("result.tab.overflow."))
                .map(|e| e.widget_id.clone())
                .expect("overflow item");
            driver.click_widget_id(&first_ov)?;
            assert_clean_parity(&driver, &format!("post-overflow {vp:?}"))?;
        }
    }
    Ok(())
}

#[test]
fn da_05_06_result_tabs_overflow_escape_disabled_reset_and_scroll() -> BoxRes {
    let mut app = make_full_project_app()?;
    let many_tabs: Vec<ResultTab> = (0..18)
        .map(|i| ResultTab {
            id: format!("ov-tab-{i}"),
            title: format!("Result Analysis Summary Tab Number {i}"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Metric"), String::from("Value")],
            row_labels: vec![String::from("Row 1")],
            rows: vec![vec![ResultCell::Text(format!("m{i}")), num_result_cell(i as f64)]],
            footnotes: Vec::new(),
        })
        .collect();
    let mut doc = app.document().clone();
    if let Some(rs) = doc.result_sheet_mut(&SheetId::new("res-1")) {
        rs.tabs = many_tabs;
    }
    *app.session_mut() = Session::from_document(doc);
    app.execute_command("sheet.select", json!({ "sheet_id": "res-1" }))?;
    app.sync_views();

    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(640.0, 420.0, 1.0), Tokens::default());
    driver.click_widget_id("result.tabs.overflow")?;
    assert!(driver.frame_meta().find_by_widget_id("popup.result.tabs.overflow").expect("popup").unclipped_rect.height() <= 270.0);

    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(driver.frame_meta().find_by_widget_id("popup.result.tabs.overflow").is_none() && !driver.state().views.results_view.tab_overflow_open);

    driver.click_widget_id("result.tabs.overflow")?;
    assert!(driver.state().views.results_view.tab_overflow_open);
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::Feedback;
    driver.run_settled();
    assert!(!driver.state().views.results_view.tab_overflow_open);
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::None;
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("popup.result.tabs.overflow").is_none());
    Ok(())
}

#[test]
fn r20_4_fit_zoom_contract_and_wcag_258_spacing_equivalent_verification() -> BoxRes {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;
    app.sync_views();

    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    assert!(driver.state().app.egui_views.canvas_view.fit_zoom_active);
    let paper_wide = driver.state().app.egui_views.canvas_view.last_paper_rect_pt.expect("wide paper");
    driver.set_viewport(ViewportMetrics::new(1040.0, 680.0, 1.0));
    driver.run_settled();
    let paper_narrow = driver.state().app.egui_views.canvas_view.last_paper_rect_pt.expect("narrow paper");
    assert!(
        driver.state().app.egui_views.canvas_view.fit_zoom_active
            && (paper_narrow.width() < paper_wide.width() - 5.0 || paper_narrow.height() < paper_wide.height() - 5.0)
    );

    driver.click_widget_id("status.zoom.in")?;
    assert!(!driver.state().app.egui_views.canvas_view.fit_zoom_active);
    let manual_zoom = driver.state().app.canvas_dialogs.canvas.zoom;
    driver.set_viewport(ViewportMetrics::new(1280.0, 820.0, 1.0));
    driver.run_settled();
    assert!(!driver.state().app.egui_views.canvas_view.fit_zoom_active && (driver.state().app.canvas_dialogs.canvas.zoom - manual_zoom).abs() < 1e-4);
    driver.click_widget_id("status.zoom.fit")?;
    assert!(driver.state().app.egui_views.canvas_view.fit_zoom_active);

    driver.click_widget_id("nav.chip.layouts")?;
    driver.click_widget_id("nav.sheet.lay-1")?;
    assert!(driver.state().app.egui_views.canvas_view.fit_zoom_active);
    for wid in ["inspector.layout.panel.width_pt", "inspector.layout.panel.height_pt"] {
        let r = driver.frame_meta().find_by_widget_id(wid).unwrap_or_else(|| panic!("missing {wid}")).clipped_rect;
        assert!(r.width() >= 24.0 && r.height() >= 24.0, "{wid}: {r:?}");
    }

    for wid in ["nav.chip.all", "nav.sheet.col-1", "toolbar.split", "toolbar.split.preset.data_graph"] {
        driver.click_widget_id(wid)?;
    }
    let meta = driver.frame_meta();
    let splitters: Vec<_> = meta.entries.iter().filter(|e| e.enabled && e.role.eq_ignore_ascii_case("splitter")).collect();
    assert!(!splitters.is_empty());
    for sp in &splitters {
        for other in &meta.entries {
            if !other.enabled
                || other.widget_id == sp.widget_id
                || other.z_layer != sp.z_layer
                || matches!(other.role.to_ascii_lowercase().as_str(), "region" | "label" | "heading" | "group" | "gridcell")
            {
                continue;
            }
            let overlap = sp.clipped_rect.intersect(other.clipped_rect);
            assert!(overlap.width() <= 1.0 || overlap.height() <= 1.0, "splitter `{}` overlaps `{}`", sp.widget_id, other.widget_id);
        }
    }

    driver.state_mut().app.canvas_dialogs.active_dialog =
        ActiveDialog::FormatImage { sheet_id: SheetId::new("g-1"), target_index: 0, is_layout: false };
    driver.run_settled();
    for wid in ["format_image.width_pt", "format_image.height_pt", "format_image.rotation_deg"] {
        let r = driver.frame_meta().find_by_widget_id(wid).unwrap_or_else(|| panic!("missing {wid}")).clipped_rect;
        assert!(r.width() >= 24.0 && r.height() >= 24.0, "{wid}: {r:?}");
    }
    Ok(())
}

#[test]
fn aud_08_09_13_canvas_rulers_equation_tex_find_bar_and_split_results() -> BoxRes {
    use crate::helpers::{click_input, run_egui_pass_with_ctx};
    use curvebook_doc::{GlmFamily, GlmSpec, LinearRegressionSpec, SimpleLogisticSpec, SplineLowessSpec, SubcolumnMode, XColumnMode, XyTable};
    use curvebook_tools::GridCellCoord;
    use curvebook_tools::canvas::PhysicalUnit;
    use curvebook_ui_egui::dialogs_canvas::copy_result_table_to_clipboard;
    use curvebook_ui_egui::dialogs_r9::InsertModelEquationDialogState;
    use curvebook_ui_egui::egui_shell::canvas_view::{CanvasPerSheetViewportState, compute_ruler_ticks, resolve_live_equation_tex};
    use curvebook_ui_egui::egui_shell::results_view::{
        format_result_cell_display_text, render_results_view_for_sheet, visible_row_index_for_raw_row,
    };
    use curvebook_ui_egui::egui_shell::sheet_find_bar::{
        DataSheetFindMatch, ResultSheetFindMatch, SheetFindBarState, X_OR_TIME_COL_SENTINEL, find_matches_in_data_sheet_cached,
        find_matches_in_data_sheet_with_locale, find_matches_in_result_sheet, find_matches_in_result_sheet_cached, render_sheet_find_bar,
    };
    use curvebook_ui_egui::egui_shell::{FrameInspectMeta, UiIntent};
    use curvebook_ui_egui::i18n::LocalePreset;
    use curvebook_ui_egui::{StubPlatformClipboardAdapter, handle_control_line};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    // 1. AUD-08-05 & AUD-08-06: ui.zoom.fitSheet resets per-sheet pan_pt; compute_ruler_ticks 1-2-5 increments & wide pan.
    let (mut gapp, gds) = crate::helpers::seed_xy_app(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]);
    gapp.execute_command("graph.create", json!({ "sheet_id": "g-fit", "data_sheet_id": gds.as_str(), "kind": "xy", "initialized": true }))?;
    gapp.execute_command("sheet.select", json!({ "sheet_id": "g-fit" }))?;
    gapp.egui_views.canvas_view.pan_pt = [40.0, -25.0];
    gapp.egui_views
        .canvas_view
        .sheet_viewports
        .insert(SheetId::new("g-fit"), CanvasPerSheetViewportState { pan_pt: [40.0, -25.0], ..Default::default() });
    assert!(handle_control_line(&mut gapp, r#"{"id":"f","method":"ui.zoom.fitSheet"}"#).contains("\"ok\":true"));
    assert_eq!(
        (gapp.egui_views.canvas_view.pan_pt, gapp.egui_views.canvas_view.sheet_viewports.get(&SheetId::new("g-fit")).map(|v| v.pan_pt)),
        ([0.0, 0.0], Some([0.0, 0.0]))
    );
    let low = compute_ruler_ticks(0.0, 0.0, 400.0, 0.25, PhysicalUnit::Inches);
    assert!(low.len() >= 2 && low[0].1 == 0 && low[1].1 == 2 && (low[1].0 - 36.0).abs() < 1e-3);
    assert!(compute_ruler_ticks(-3000.0, 0.0, 600.0, 1.0, PhysicalUnit::Inches).first().is_some_and(|t| t.1 > 32));

    // 2. AUD-13-16 & AUD-13-18: Live TeX parity for SimpleLogistic, Linear fixed_point, Custom DSL <B>, and Spline/GLM ROC suppression.
    let mut eq_doc = ProjectDocument::new("Eq")?;
    let ds = DataSheet::new("d-xy", "XY", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 4)?))?;
    let mut gs = GraphSheet::new_uninitialized_for_data("g-xy", &ds)?;
    gs.initialized = true;
    eq_doc.data_sheets.push(ds);
    eq_doc.graph_sheets.push(gs.clone());
    let roc = FittedCurveOverlay {
        equation_id: "ROC".into(),
        data_set_index: 0,
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 0.0, y: 0.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 1.0, y: 1.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    };
    let mut add_rs = |id: &str, kind: AnalysisKind, scalars: &[(&str, f64, usize)], curve: Option<FittedCurveOverlay>| -> BoxRes {
        let mut r = ResultSheet::new(AnalysisSpec::new(id, "d-xy", kind)?)?;
        r.status = ResultStatus::UpToDate;
        for &(k, v, dsi) in scalars {
            r.scalars.push(ResultScalarEntry { key: k.into(), value: v, units: None, data_set_index: Some(dsi) });
        }
        if let Some(c) = curve {
            r.fitted_curves.push(c);
        }
        eq_doc.result_sheets.push(r);
        Ok(())
    };
    add_rs(
        "r-log",
        AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default()),
        &[("beta0", -2.5, 0), ("beta1", 0.0001234, 0), ("AIC", 42.0, 0)],
        Some(roc.clone()),
    )?;
    add_rs(
        "r-lin",
        AnalysisKind::LinearRegression(LinearRegressionSpec { force_through_origin: true, fixed_point: Some((1.0, 5.0)), ..Default::default() }),
        &[("Slope", 2.0, 0), ("YIntercept", 3.0, 0)],
        None,
    )?;
    let mut nls = NonlinearFitSpec::new("custom_dsl");
    nls.custom_dsl = Some("<A>Y = a*X\n<B>Y = b*X + c".into());
    add_rs("r-nl", AnalysisKind::NonlinearRegression(nls), &[("b", 4.0, 1), ("c", 7.0, 1)], None)?;
    add_rs(
        "r-sp",
        AnalysisKind::FitSplineLowess(SplineLowessSpec::default()),
        &[],
        Some(FittedCurveOverlay { equation_id: "lowess".into(), ..roc.clone() }),
    )?;
    add_rs(
        "r-glm",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: vec![],
            compute_vif: false,
        }),
        &[("beta0", -1.0, 0), ("beta1", 0.5, 0)],
        Some(roc),
    )?;
    let lt = resolve_live_equation_tex(&eq_doc, &SheetId::new("r-log"), 0, EquationDisplayMode::Substituted).ok_or("log tex")?;
    assert!(lt.contains("P(Y=1)") && lt.contains("0.0001234") && !lt.contains("AIC"));
    assert!(resolve_live_equation_tex(&eq_doc, &SheetId::new("r-lin"), 0, EquationDisplayMode::Substituted).unwrap().contains("+ 3"));
    let nlt = resolve_live_equation_tex(&eq_doc, &SheetId::new("r-nl"), 1, EquationDisplayMode::Substituted).unwrap();
    assert!(nlt.contains('4') && nlt.contains('7'));
    assert!(resolve_live_equation_tex(&eq_doc, &SheetId::new("r-sp"), 0, EquationDisplayMode::Substituted).is_none());
    assert!(resolve_live_equation_tex(&eq_doc, &SheetId::new("r-glm"), 0, EquationDisplayMode::Substituted).is_none());
    let mut eq_dlg = InsertModelEquationDialogState::default();
    eq_dlg.open_for_graph(&gs, &eq_doc);
    assert!(
        eq_dlg.available_result_ids.contains(&SheetId::new("r-log"))
            && !eq_dlg.available_result_ids.contains(&SheetId::new("r-sp"))
            && !eq_dlg.available_result_ids.contains(&SheetId::new("r-glm"))
    );

    // 3. AUD-09-22/23/24/25/39/40: Locale find (XY/Survival/Frame1), ResultSheet find/formatter/collapsed rows, split tab sync & copy, find bar open bool.
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 2)?;
    (xy.x_title, xy.x_cells[0][0], xy.data_sets[0].cells[1][0]) = ("Conc_uM".into(), Cell::number(12.5), Cell::number(42.5));
    let ds_xy = DataSheet::new("ds-xy-f", "XY", TableArchetype::Xy(xy))?;
    let de = LocalePreset::for_locale("de");
    assert_eq!(
        find_matches_in_data_sheet_with_locale(&ds_xy, "conc_um", false, true, &de),
        vec![DataSheetFindMatch::ColumnTitle { col: X_OR_TIME_COL_SENTINEL }]
    );
    assert_eq!(
        find_matches_in_data_sheet_with_locale(&ds_xy, "12,5", false, true, &de),
        vec![DataSheetFindMatch::Cell(GridCellCoord::new(0, X_OR_TIME_COL_SENTINEL, 0))]
    );
    assert_eq!(find_matches_in_data_sheet_cached(7, &ds_xy, "42,5", false, true, &de), vec![DataSheetFindMatch::Cell(GridCellCoord::new(1, 0, 0))]);
    let mut surv = SurvivalTable::new(1, 2)?;
    (surv.time_title, surv.times[1]) = ("FollowUpDays".into(), Cell::number(90.0));
    let ds_s = DataSheet::new("ds-s-f", "Surv", TableArchetype::Survival(surv))?;
    assert_eq!(
        find_matches_in_data_sheet_with_locale(&ds_s, "followupdays", false, true, &LocalePreset::default()),
        vec![DataSheetFindMatch::ColumnTitle { col: X_OR_TIME_COL_SENTINEL }]
    );
    assert_eq!(
        find_matches_in_data_sheet_with_locale(&ds_s, "90", false, true, &LocalePreset::default()),
        vec![DataSheetFindMatch::Cell(GridCellCoord::new(1, X_OR_TIME_COL_SENTINEL, 0))]
    );

    let mut nl3 = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl3.output_significant_digits = Some(3);
    let mut rs = ResultSheet::new(AnalysisSpec::new("rs-f", "ds-xy-f", AnalysisKind::NonlinearRegression(nl3))?)?;
    rs.status = ResultStatus::UpToDate;
    rs.tabs = vec![
        ResultTab {
            id: "e".into(),
            title: "E".into(),
            kind: ResultTabKind::Diagnostics,
            column_headers: vec!["Est".into()],
            row_labels: vec![],
            rows: vec![],
            footnotes: vec![],
        },
        ResultTab {
            id: "m".into(),
            title: "M".into(),
            kind: ResultTabKind::MainSummary,
            column_headers: vec!["Mean".into(), "95% CI".into()],
            row_labels: vec!["Mean".into(), "Unreached".into()],
            rows: vec![
                vec![ResultCell::Text("Mean".into()), ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open }],
                vec![
                    ResultCell::Number { value: f64::NAN, prefix: None, units: None },
                    ResultCell::Number { value: 12.3456, prefix: None, units: None },
                ],
            ],
            footnotes: vec![],
        },
    ];
    assert!(
        find_matches_in_result_sheet(&rs, 0, false, "Est", false, false).is_empty()
            && find_matches_in_result_sheet(&rs, 1, false, "NaN", false, false).is_empty()
    );
    assert_eq!(
        find_matches_in_result_sheet(&rs, 1, false, "??? (ambiguous)", false, true),
        vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 1 }]
    );
    assert_eq!(find_matches_in_result_sheet(&rs, 1, false, "Mean", true, true), vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 0 }]);
    assert_eq!(
        find_matches_in_result_sheet_cached(1, &rs, 1, false, "12.3", false, true),
        vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 1, col_idx: 1 }]
    );
    assert_eq!(format_result_cell_display_text(&rs.tabs[1].rows[0][1], &rs, &rs.tabs[1]), "??? (ambiguous)");

    let sec = ResultTab {
        id: "sec".into(),
        title: "S".into(),
        kind: ResultTabKind::MainSummary,
        column_headers: vec!["V".into()],
        row_labels: vec!["SA".into(), "A1".into(), "A2".into(), "SB".into(), "B1".into()],
        rows: vec![
            vec![ResultCell::Empty],
            vec![ResultCell::Number { value: 1.0, prefix: None, units: None }],
            vec![ResultCell::Number { value: 2.0, prefix: None, units: None }],
            vec![ResultCell::Empty],
            vec![ResultCell::Number { value: 3.0, prefix: None, units: None }],
        ],
        footnotes: vec![],
    };
    assert_eq!(visible_row_index_for_raw_row(&SheetId::new("rs-f"), &sec, 4, &BTreeSet::from(["rs-f:sec:0".into()])), 2);

    // Split ResultSheet scroll_to_match + live tab switch copy
    let mut sw_doc = ProjectDocument::new("TabSw")?;
    sw_doc.add_data_sheet(DataSheet::new("ds-sw", "DS", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2)?))?)?;
    let mut sw_rs = ResultSheet::new(AnalysisSpec::new("res-sw", "ds-sw", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    sw_rs.status = ResultStatus::UpToDate;
    sw_rs.tabs = vec![
        ResultTab {
            id: "t0".into(),
            title: "T0".into(),
            kind: ResultTabKind::MainSummary,
            column_headers: vec!["V".into()],
            row_labels: vec!["R0".into()],
            rows: vec![vec![ResultCell::Text("STALE_0".into())]],
            footnotes: vec![],
        },
        ResultTab {
            id: "t1".into(),
            title: "T1".into(),
            kind: ResultTabKind::Diagnostics,
            column_headers: vec!["V".into()],
            row_labels: vec!["R0".into()],
            rows: vec![vec![ResultCell::Text("FRESH_1".into())]],
            footnotes: vec![],
        },
    ];
    sw_doc.add_result_sheet(sw_rs)?;
    sw_doc.active_sheet_id = Some(SheetId::new("ds-sw"));
    let mut sw_app = UiApp::from_session(Session::from_document(sw_doc));
    sw_app.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
    let tok = Tokens::from_theme_spec(&sw_app.theme, &sw_app.locale);
    let mut rv = ResultsViewEguiState::default();
    rv.scroll_to_match(SheetId::new("res-sw"), 0, 0, 0);
    let mut i_sc = PendingUiIntents::new();
    run_egui_pass_with_ctx(&egui::Context::default(), egui::vec2(800.0, 500.0), Default::default(), true, |_, ui| {
        let _ = render_results_view_for_sheet(
            ui,
            &sw_app,
            Some(&SheetId::new("res-sw")),
            &mut rv,
            &tok,
            true,
            &mut FrameInspectMeta::default(),
            &mut i_sc,
        );
    })?;
    assert!(
        i_sc.intents().iter().any(|i| matches!(i, UiIntent::SyncResultSelection { sheet_id, tab_index: 0, .. } if sheet_id.as_str() == "res-sw"))
    );

    sw_app.sync_views();
    sw_app.canvas_dialogs.result_view.bound_sheet_id = Some("res-sw".into());
    sw_app.canvas_dialogs.result_view.bound_tab_idx = Some(0);
    sw_app.canvas_dialogs.result_view.selected_anchor = Some((0, 0));
    sw_app.canvas_dialogs.result_view.selected_focus = Some((0, 0));
    let mut rv2 = ResultsViewEguiState {
        selected_cell: Some((SheetId::new("res-sw"), 0, 0, 0)),
        selection_anchor: Some((SheetId::new("res-sw"), 0, 0, 0)),
        ..ResultsViewEguiState::default()
    };
    let (ctx_sw, mut m_sw) = (egui::Context::default(), FrameInspectMeta::default());
    run_egui_pass_with_ctx(&ctx_sw, egui::vec2(800.0, 500.0), Default::default(), true, |c, ui| {
        m_sw.begin_pass(c);
        render_results_view_for_sheet(ui, &sw_app, Some(&SheetId::new("res-sw")), &mut rv2, &tok, true, &mut m_sw, &mut PendingUiIntents::new());
    })?;
    let t1_r = m_sw.find_by_widget_id("result.tab.t1").ok_or("t1")?.clipped_rect;
    let mut i_t1 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_sw, egui::vec2(800.0, 500.0), click_input(t1_r.center()), true, |c, ui| {
        let mut m2 = FrameInspectMeta::default();
        m2.begin_pass(c);
        render_results_view_for_sheet(ui, &sw_app, Some(&SheetId::new("res-sw")), &mut rv2, &tok, true, &mut m2, &mut i_t1);
    })?;
    assert!(rv2.selected_cell.is_none() && rv2.selection_anchor.is_none());
    let _ = i_t1.commit_all_if_settled(&mut sw_app, false);
    sw_app.egui_views.results_view = rv2;
    let ctxt = copy_result_table_to_clipboard(&mut sw_app, "res-sw", false).command_result.unwrap()["copied_text"].as_str().unwrap().to_owned();
    assert!(ctxt.contains("FRESH_1") && !ctxt.contains("STALE_0"));

    // AUD-09-39 & Wave3 Frame-1 German find
    let mut fs_bar = SheetFindBarState::default();
    fs_bar.open_for_sheet(Some(SheetId::new("ds-sw")));
    let open_ret = run_egui_pass_with_ctx(&egui::Context::default(), egui::vec2(900.0, 500.0), Default::default(), true, |c, _| {
        render_sheet_find_bar(
            c,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0)),
            true,
            false,
            2,
            &mut fs_bar,
            &tok,
            &mut FrameInspectMeta::default(),
            &mut PendingUiIntents::new(),
        )
    })?;
    fs_bar.close();
    let closed_ret = run_egui_pass_with_ctx(&egui::Context::default(), egui::vec2(900.0, 500.0), Default::default(), true, |c, _| {
        render_sheet_find_bar(
            c,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0)),
            true,
            false,
            2,
            &mut fs_bar,
            &tok,
            &mut FrameInspectMeta::default(),
            &mut PendingUiIntents::new(),
        )
    })?;
    assert!(open_ret && !closed_ret);

    let (mut de_app, de_sid) = crate::helpers::seed_column_app(&[("Dose", &[12.5, 99.0])], ColumnDesignHint::UnpairedReplicates);
    de_app.set_locale("de");
    let mut views = EguiShellViewState::default();
    views.sheet_find.open_for_sheet(Some(de_sid));
    views.sheet_find.query = "12,5".into();
    let lay = LayoutMetrics::CANONICAL.solve_shell_layout(ShellLayoutInput {
        viewport_w_pt: 1100.0,
        viewport_h_pt: 720.0,
        empty_workspace: false,
        navigator_open: false,
        inspector_open: false,
        family_open: false,
        leading_user_collapsed: false,
        trailing_user_collapsed: false,
        leading_width_override_pt: None,
        trailing_width_override_pt: None,
        host_draws_menu_bar: false,
        is_rtl: false,
    });
    let dec = HybridCoexistenceArbiter::evaluate_with_views(&de_app, &views, false);
    run_egui_pass_with_ctx(&egui::Context::default(), egui::vec2(1100.0, 720.0), Default::default(), true, |_, ui| {
        let _ = render_center_stage(
            ui,
            &de_app,
            &mut views,
            CenterStageContext {
                tokens: &Tokens::default(),
                decision: &dec,
                layout: lay,
                focused_pane: ActiveWorkspacePane::MainCanvasOrGrid,
                enabled: true,
            },
            &mut FrameInspectMeta::default(),
            &mut PendingUiIntents::new(),
        );
    })?;
    assert!(views.sheet_find.last_match_count == 1 && views.column_grid.active_find_matches.is_some());
    Ok(())
}
