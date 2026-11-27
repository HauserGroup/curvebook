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

    let hook_node = driver.frame_meta().find_by_widget_id("info.hook_badge.0").ok_or("missing info.hook_badge.0")?;
    assert_eq!(hook_node.role, "link");
    assert!(hook_node.unclipped_rect.height() >= 24.0);
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

    let cld_node = driver.frame_meta().find_by_widget_id("canvas.pairwise_cld_recommend").ok_or("missing canvas.pairwise_cld_recommend")?;
    assert_eq!(cld_node.role, "link");
    assert!(cld_node.unclipped_rect.height() >= 24.0);
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
                let is_active = entry.value.as_ref().and_then(|v| v.get("selected")).and_then(serde_json::Value::as_bool) == Some(true);
                let max_expected = if is_active { ((vp.logical_width as f32) * 0.55).max(200.0) } else { 205.0_f32 };
                assert!(
                    entry.clipped_rect.width() <= max_expected,
                    "tab {} width {} exceeded content cap {max_expected}",
                    entry.widget_id,
                    entry.clipped_rect.width()
                );
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
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::About;
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

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.layouts")?;
    driver.click_widget_id("nav.sheet.lay-1")?;
    assert!(driver.state().app.egui_views.canvas_view.fit_zoom_active);
    for wid in ["inspector.layout.panel.width_pt", "inspector.layout.panel.height_pt"] {
        let r = driver.frame_meta().find_by_widget_id(wid).unwrap_or_else(|| panic!("missing {wid}")).clipped_rect;
        assert!(r.width() >= 24.0 && r.height() >= 24.0, "{wid}: {r:?}");
    }

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.all")?;
    for wid in ["nav.sheet.col-1", "toolbar.split", "toolbar.split.preset.data_graph"] {
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
            ..GlmSpec::default()
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
        find_matches_in_result_sheet(&rs, 1, false, "Not bounded", false, true),
        vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 1 }]
    );
    assert_eq!(find_matches_in_result_sheet(&rs, 1, false, "Mean", true, true), vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 0 }]);
    assert_eq!(
        find_matches_in_result_sheet_cached(1, &rs, 1, false, "12.3", false, true),
        vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 1, col_idx: 1 }]
    );
    assert_eq!(format_result_cell_display_text(&rs.tabs[1].rows[0][1], &rs, &rs.tabs[1]), "Not bounded");

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

struct SplitShortcutTarget<'a> {
    pane_wid: &'a str,
    focused_slot: SplitPaneSlot,
    focused_sid: &'a str,
    other_sid: &'a str,
    focused_r0: f64,
    focused_r1: f64,
    focused_c1: f64,
    other_r0: f64,
    other_r1: f64,
    other_c1: f64,
}

fn click_cell_in_pane(drv: &mut EguiHumanPathDriver<'_, ShellTestState>, pane_wid: &str, cell_wid: &str) -> Result<(), String> {
    let pane_rect = drv.frame_meta().find_by_widget_id(pane_wid).ok_or_else(|| format!("missing split pane `{pane_wid}`"))?.clipped_rect;
    let cell_pos = drv
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id == cell_wid && pane_rect.contains(e.clipped_rect.center()))
        .map(|e| e.clipped_rect.center())
        .ok_or_else(|| format!("missing `{cell_wid}` inside `{pane_wid}`"))?;
    drv.click_at(cell_pos)
}

fn read_split_cell(drv: &EguiHumanPathDriver<'_, ShellTestState>, sid: &str, row: usize, col: usize) -> Cell {
    drv.state().app.document().data_sheet(&SheetId::new(sid)).and_then(|ds| ds.cell(CellCoord::new(row, col, 0))).cloned().unwrap_or_default()
}

fn verify_split_pane_shortcuts(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, t: &SplitShortcutTarget<'_>) -> BoxRes {
    use curvebook_doc::CellValue;

    let pane_wid = t.pane_wid;
    // 1. Delete + single Undo.
    click_cell_in_pane(driver, pane_wid, "grid.cell.0.0.0")?;
    assert_eq!(driver.state().views.center_stage.focused_split_pane, t.focused_slot);
    driver.press_egui_key(Key::Delete, Modifiers::NONE);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 0).value, CellValue::Empty, "{pane_wid} Delete must clear focused cell");
    assert_eq!(read_split_cell(driver, t.other_sid, 0, 0).value, CellValue::Number(t.other_r0), "{pane_wid} Delete must not clear unfocused table");
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 0).value, CellValue::Number(t.focused_r0), "{pane_wid} single Undo must restore Delete");
    assert_eq!(read_split_cell(driver, t.other_sid, 0, 0).value, CellValue::Number(t.other_r0));

    // 2. Backspace + single Undo (cell (0,0,0) remains selected in the focused pane).
    driver.press_egui_key(Key::Backspace, Modifiers::NONE);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 0).value, CellValue::Empty, "{pane_wid} Backspace must clear focused cell");
    assert_eq!(
        read_split_cell(driver, t.other_sid, 0, 0).value,
        CellValue::Number(t.other_r0),
        "{pane_wid} Backspace must not clear unfocused table"
    );
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 0).value, CellValue::Number(t.focused_r0), "{pane_wid} single Undo must restore Backspace");
    assert_eq!(read_split_cell(driver, t.other_sid, 0, 0).value, CellValue::Number(t.other_r0));

    // 3. Cut (Cmd+X) + single Undo + Paste (Cmd+V) + single Undo.
    driver.press_egui_key(Key::X, Modifiers::COMMAND);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 0).value, CellValue::Empty, "{pane_wid} Cut must clear focused cell");
    assert_eq!(read_split_cell(driver, t.other_sid, 0, 0).value, CellValue::Number(t.other_r0), "{pane_wid} Cut must not clear unfocused table");
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 0).value, CellValue::Number(t.focused_r0), "{pane_wid} single Undo must restore Cut");

    click_cell_in_pane(driver, pane_wid, "grid.cell.1.0.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    assert_eq!(read_split_cell(driver, t.focused_sid, 1, 0).value, CellValue::Number(t.focused_r0), "{pane_wid} Paste must write focused cell");
    assert_eq!(read_split_cell(driver, t.other_sid, 1, 0).value, CellValue::Number(t.other_r1), "{pane_wid} Paste must not touch unfocused table");
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(read_split_cell(driver, t.focused_sid, 1, 0).value, CellValue::Number(t.focused_r1), "{pane_wid} single Undo must restore Paste");
    assert_eq!(read_split_cell(driver, t.other_sid, 1, 0).value, CellValue::Number(t.other_r1));

    // 4. Exclude (Cmd+E) + single Undo on (0,1,0).
    click_cell_in_pane(driver, pane_wid, "grid.cell.0.1.0")?;
    driver.press_egui_key(Key::E, Modifiers::COMMAND);
    assert!(read_split_cell(driver, t.focused_sid, 0, 1).excluded, "{pane_wid} Cmd+E must exclude focused cell");
    assert!(!read_split_cell(driver, t.other_sid, 0, 1).excluded, "{pane_wid} Cmd+E must not exclude unfocused table");
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert!(!read_split_cell(driver, t.focused_sid, 0, 1).excluded, "{pane_wid} single Undo must restore Exclude");
    assert!(!read_split_cell(driver, t.other_sid, 0, 1).excluded);

    // 5. F2 inline edit targets only focused pane; typing + Enter commits only focused table; single Undo restores.
    driver.press_egui_key(Key::F2, Modifiers::NONE);
    let primary_open = driver.state().views.column_grid.inline_edit.is_some();
    let secondary_open = driver.state().views.center_stage.secondary_column_grid.inline_edit.is_some();
    assert_eq!((primary_open, secondary_open), (t.focused_slot == SplitPaneSlot::Primary, t.focused_slot == SplitPaneSlot::Secondary));
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    driver.type_text("777")?;
    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 1).value, CellValue::Number(777.0), "{pane_wid} typing must update focused table");
    assert_eq!(read_split_cell(driver, t.other_sid, 0, 1).value, CellValue::Number(t.other_c1), "{pane_wid} typing must not update unfocused table");
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 1).value, CellValue::Number(t.focused_c1), "{pane_wid} single Undo must restore typed edit");

    // 6. Arrow navigation + Delete + single Undo.
    click_cell_in_pane(driver, pane_wid, "grid.cell.0.0.0")?;
    driver.press_egui_key(Key::ArrowDown, Modifiers::NONE);
    driver.press_egui_key(Key::Delete, Modifiers::NONE);
    assert_eq!(read_split_cell(driver, t.focused_sid, 0, 0).value, CellValue::Number(t.focused_r0), "row 0 must stay untouched after ArrowDown");
    assert_eq!(read_split_cell(driver, t.focused_sid, 1, 0).value, CellValue::Empty, "{pane_wid} ArrowDown+Delete must clear row 1");
    assert_eq!(read_split_cell(driver, t.other_sid, 1, 0).value, CellValue::Number(t.other_r1), "{pane_wid} must not touch unfocused table");
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(read_split_cell(driver, t.focused_sid, 1, 0).value, CellValue::Number(t.focused_r1), "{pane_wid} single Undo must restore row 1");
    Ok(())
}

#[test]
fn r20_19_split_two_table_keyboard_shortcuts_single_recipient_and_single_undo() -> BoxRes {
    use std::sync::Arc;

    use curvebook_ui_egui::StubPlatformClipboardAdapter;

    let mut doc = ProjectDocument::new("R20.19 Split Two-Table Shortcuts")?;
    let mut left_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    left_tbl.data_sets[0].title = String::from("Left A");
    left_tbl.data_sets[1].title = String::from("Left B");
    left_tbl.data_sets[0].cells[0][0] = Cell::number(1.0);
    left_tbl.data_sets[0].cells[1][0] = Cell::number(11.0);
    left_tbl.data_sets[1].cells[0][0] = Cell::number(2.0);
    doc.add_data_sheet(DataSheet::new("tbl-left", "Left Table", TableArchetype::Column(left_tbl))?)?;

    let mut right_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    right_tbl.data_sets[0].title = String::from("Right A");
    right_tbl.data_sets[1].title = String::from("Right B");
    right_tbl.data_sets[0].cells[0][0] = Cell::number(42.0);
    right_tbl.data_sets[0].cells[1][0] = Cell::number(84.0);
    right_tbl.data_sets[1].cells[0][0] = Cell::number(99.0);
    doc.add_data_sheet(DataSheet::new("tbl-right", "Right Table", TableArchetype::Column(right_tbl))?)?;

    doc.select_in_navigator(&SheetId::new("tbl-left"))?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::default()));
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("tbl-left"));
    app.split.secondary_sheet_id = Some(SheetId::new("tbl-right"));
    app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    app.sync_views();

    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 820.0, 1.0));
    for target in [
        SplitShortcutTarget {
            pane_wid: "center.pane.secondary",
            focused_slot: SplitPaneSlot::Secondary,
            focused_sid: "tbl-right",
            other_sid: "tbl-left",
            focused_r0: 42.0,
            focused_r1: 84.0,
            focused_c1: 99.0,
            other_r0: 1.0,
            other_r1: 11.0,
            other_c1: 2.0,
        },
        SplitShortcutTarget {
            pane_wid: "center.pane.primary",
            focused_slot: SplitPaneSlot::Primary,
            focused_sid: "tbl-left",
            other_sid: "tbl-right",
            focused_r0: 1.0,
            focused_r1: 11.0,
            focused_c1: 2.0,
            other_r0: 42.0,
            other_r1: 84.0,
            other_c1: 99.0,
        },
    ] {
        verify_split_pane_shortcuts(&mut driver, &target)?;
    }
    Ok(())
}

#[test]
fn r20_19_wf59_layout_panel_size_below_fold_scroll_into_view_and_edit() -> BoxRes {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" }))?;
    app.sync_views();

    // At 1440x520 (reproducing WF-59's short viewport where Selected Panel Size sits below the ScrollArea fold),
    // `inspector.layout.panel.{width_pt,height_pt}` must NOT be recorded with empty/inverted bounds.
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 520.0, 1.0));
    driver.click_widget_id("canvas.layout_panel.0")?;

    for entry in &driver.frame_meta().entries {
        assert!(
            entry.clipped_rect.is_positive(),
            "widget `{}` must never be recorded with empty/inverted clipped bounds: {:?}",
            entry.widget_id,
            entry.clipped_rect
        );
    }
    assert!(
        driver.frame_meta().find_by_widget_id("inspector.layout.panel.width_pt").is_none(),
        "below-the-fold width_pt spinbutton must not be recorded before scrolling into view"
    );
    assert!(
        driver.frame_meta().find_by_widget_id("inspector.layout.panel.height_pt").is_none(),
        "below-the-fold height_pt spinbutton must not be recorded before scrolling into view"
    );

    let dock_rect = driver.state().trailing_rect_pt.ok_or("missing trailing studio rect")?;
    let scroll_pos = pos2(dock_rect.center().x, dock_rect.top() + 140.0);
    let mut found_both = false;
    for _ in 0..8 {
        if driver.frame_meta().find_by_widget_id("inspector.layout.panel.width_pt").is_some()
            && driver.frame_meta().find_by_widget_id("inspector.layout.panel.height_pt").is_some()
        {
            found_both = true;
            break;
        }
        driver.wheel_at(scroll_pos, egui::vec2(0.0, -180.0))?;
    }
    assert!(found_both, "scrolling Trailing Studio must expose both layout panel size spinbuttons");

    let initial_bounds = match &driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?.spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => *bounds_pt,
        other => return Err(format!("expected LiveGraph at panel 0, got {other:?}").into()),
    };

    for wid in ["inspector.layout.panel.width_pt", "inspector.layout.panel.height_pt"] {
        let r = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}` after scroll"))?.clipped_rect;
        assert!(r.is_positive() && r.width() >= 24.0 && r.height() >= 20.0, "scrolled `{wid}` must have actionable bounds: {r:?}");
    }

    // Apply edits to both scrolled-into-view DragValues via click + type + Enter and verify LayoutSpec updates.
    let w_pos = widget_center(&driver, "inspector.layout.panel.width_pt")?;
    driver.click_at(w_pos)?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text("265")?;
    driver.press_egui_key(Key::Enter, Modifiers::NONE);

    let h_pos = widget_center(&driver, "inspector.layout.panel.height_pt")?;
    driver.click_at(h_pos)?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text("195")?;
    driver.press_egui_key(Key::Enter, Modifiers::NONE);

    let edited_bounds = match &driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?.spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => *bounds_pt,
        other => return Err(format!("expected LiveGraph at panel 0 after edit, got {other:?}").into()),
    };
    assert!(
        (edited_bounds.width - initial_bounds.width).abs() > 1.0 && (edited_bounds.width - 265.0).abs() < 0.5,
        "width edit did not apply: {edited_bounds:?}"
    );
    assert!(
        (edited_bounds.height - initial_bounds.height).abs() > 1.0 && (edited_bounds.height - 195.0).abs() < 0.5,
        "height edit did not apply: {edited_bounds:?}"
    );
    Ok(())
}

#[test]
fn r20_6_and_r20_20_nl_single_banner_compare_horizontal_per_sheet_tabs_and_right_click_copy() -> BoxRes {
    let mut xy = curvebook_doc::XyTable::new(curvebook_doc::SubcolumnMode::Single, 2, 6)?;
    xy.data_sets[0].title = String::from("VEHICLE");
    xy.data_sets[1].title = String::from("DRUG_A");
    let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let y0 = [10.0, 10.2, 9.8, 10.1, 9.9, 10.0];
    let y1 = [10.0, 12.0, 25.0, 65.0, 92.0, 95.0];
    for i in 0..xs.len() {
        xy.x_cells[i][0] = Cell::number(xs[i]);
        xy.data_sets[0].cells[i][0] = Cell::number(y0[i]);
        xy.data_sets[1].cells[i][0] = Cell::number(y1[i]);
    }
    let mut doc = ProjectDocument::default();
    doc.data_sheets.push(DataSheet::new("xy-nl", "Sparse DR", TableArchetype::Xy(xy))?);
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.execute_command(
        "analysis.run",
        json!({
            "source_sheet_id": "xy-nl",
            "sheet_id": "res-nl",
            "kind": "nonlinear_regression",
            "equation_id": "log_agonist_vs_response_4pl"
        }),
    )
    .map_err(|e| format!("analysis.run: {e}"))?;
    app.execute_command("sheet.select", json!({ "sheet_id": "res-nl" })).map_err(|e| format!("sheet.select: {e}"))?;
    app.nav.welcome.open = false;
    app.sync_views();

    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0), Tokens::default());
    assert_clean_parity(&driver, "r20_6 nl fit initial")?;

    // R20.6: Exactly ONE warning banner on ambiguous/unstable NL fit, naming failing dataset, zero legacy ambiguity_callout.
    assert!(driver.frame_meta().find_by_widget_id("result.ambiguity_callout").is_none());
    let banner = driver.frame_meta().find_by_widget_id("result.diagnostic_banner").ok_or("missing unified result.diagnostic_banner")?;
    assert!(banner.label.contains("VEHICLE") || banner.label.contains("Ambiguous"), "unexpected NL banner label: {}", banner.label);
    assert!(!banner.label.contains("???"), "banner must not contain literal ???: {}", banner.label);

    let constraints = driver.frame_meta().find_by_widget_id("result.diagnostic_banner.constraints").ok_or("missing Constraints button")?;
    let cmp_btn = driver.frame_meta().find_by_widget_id("result.diagnostic_banner.compare_horizontal").ok_or("missing Compare button")?;
    assert_eq!(constraints.label, "Constraints");
    assert_eq!(cmp_btn.label, "Compare vs. Horizontal Line");

    // R20.6: Clicking Compare vs. Horizontal Line invokes R19.7 service & opens comparison readout.
    driver.click_widget_id("result.diagnostic_banner.compare_horizontal")?;
    assert_clean_parity(&driver, "r20_6 compare horizontal readout")?;
    assert!(
        driver.frame_meta().find_by_widget_id("result.diagnostic_banner.compare_horizontal.summary").is_some(),
        "expected horizontal-line comparison summary popup"
    );
    driver.click_widget_id("result.diagnostic_banner.compare_horizontal.close")?;

    // R20.20 #3: Select range at row 0..=1, then right-click cell (2, 0) OUTSIDE range -> Copy Selection copies only (2, 0).
    let tab0 = &driver.state().app.document().result_sheet(&SheetId::new("res-nl")).ok_or("missing res-nl")?.tabs[0];
    let expected_2_0 = curvebook_ui_egui::egui_shell::results_view::format_result_cell_display_text(
        &tab0.rows[2][0],
        driver.state().app.document().result_sheet(&SheetId::new("res-nl")).unwrap(),
        tab0,
    );
    driver.click_widget_id("result.cell.0.0")?;
    let p_2_0 = widget_center(&driver, "result.cell.2.0")?;
    driver.click_at_button(
        p_2_0,
        curvebook_ui_egui::interaction::PointerButton::Secondary,
        curvebook_ui_egui::interaction::KeyModifiers::default(),
    )?;
    driver.click_widget_id("result.ctx.copy_selection")?;
    assert_eq!(driver.state().app.canvas_dialogs.result_view.last_copied_tsv.as_deref(), Some(expected_2_0.as_str()));

    // R20.20 #2: Sheet-scoped SelectResultTab updates target sheet independently without clobbering unrelated active sheet.
    let mut intents = PendingUiIntents::new();
    let other_id = SheetId::new("res-nl");
    intents.push(curvebook_ui_egui::egui_shell::UiIntent::SelectResultTab { index: 1, sheet_id: Some(other_id.clone()) });
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "xy-nl" }))?;
    let batch = intents.commit_all_if_settled(&mut driver.state_mut().app, false);
    assert!(batch.results.iter().all(Result::is_ok));
    assert_eq!(driver.state().app.egui_views.results_view.per_sheet_active_tab.get(&other_id), Some(&1));

    // Clicking Constraints opens EditAnalysisParameters dialog.
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "res-nl" }))?;
    driver.state_mut().app.sync_views();
    driver.run_settled();
    driver.click_widget_id("result.diagnostic_banner.constraints")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    Ok(())
}

fn collect_shell_shapes(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<egui::Shape>) {
    fn push_sh(sh: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match sh {
            egui::Shape::Vec(children) => {
                for ch in children {
                    push_sh(ch, out);
                }
            }
            other => out.push(other.clone()),
        }
    }
    for cs in shapes {
        push_sh(&cs.shape, out);
    }
}

fn run_shell_raw_frame(
    ctx: &egui::Context,
    app: &mut UiApp,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    t: f64,
) -> (Vec<egui::Shape>, curvebook_ui_egui::egui_shell::FrameInspectMeta, Option<std::time::Duration>) {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    tokens.apply_to_ctx(ctx);
    ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
        style.scroll_animation = egui::style::ScrollAnimation::none();
        style.animation_time = 0.0;
    });
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        time: Some(t),
        predicted_dt: 1.0 / 60.0,
        focused: true,
        events,
        ..Default::default()
    };
    raw.viewports.entry(egui::ViewportId::ROOT).and_modify(|vp| vp.native_pixels_per_point = Some(1.0));
    let mut meta = curvebook_ui_egui::egui_shell::FrameInspectMeta::new();
    let mut views = app.egui_views.clone();
    let mut intents = PendingUiIntents::new();
    let output = ctx.run_ui(raw, |ui| {
        let _ = curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents);
    });
    let will_discard = ctx.will_discard();
    app.egui_views = views;
    if !will_discard {
        let _ = intents.commit_all_if_settled(app, false);
    }
    let mut shapes = Vec::new();
    collect_shell_shapes(&output.shapes, &mut shapes);
    let repaint_delay = output.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
    output.drop_without_applying_deltas();
    (shapes, meta, repaint_delay)
}

fn settle_shell_raw_frames(
    ctx: &egui::Context,
    app: &mut UiApp,
    size: egui::Vec2,
    t0: f64,
) -> (Vec<egui::Shape>, curvebook_ui_egui::egui_shell::FrameInspectMeta) {
    let mut last = run_shell_raw_frame(ctx, app, size, Vec::new(), t0);
    for step in 1..6 {
        if !last.1.entries.is_empty() && last.2 != Some(std::time::Duration::ZERO) {
            break;
        }
        last = run_shell_raw_frame(ctx, app, size, Vec::new(), t0 + f64::from(step) * 0.016);
    }
    (last.0, last.1)
}

fn find_widget_in_pane(meta: &curvebook_ui_egui::egui_shell::FrameInspectMeta, pane_wid: &str, widget_id: &str) -> Option<egui::Rect> {
    let pane_rect = meta.find_by_widget_id(pane_wid)?.clipped_rect;
    meta.entries.iter().find(|e| e.widget_id == widget_id && pane_rect.contains(e.clipped_rect.center())).map(|e| e.clipped_rect)
}

#[test]
fn r20_11_split_data_and_results_focus_and_unemphasized_colors_on_click_f6_and_high_contrast() -> BoxRes {
    use curvebook_ui_egui::UiThemeMode;

    for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        let mut app = make_full_project_app()?;
        app.set_theme_mode(mode);
        app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
        app.nav.welcome.open = false;
        app.split.enabled = true;
        app.split.primary_sheet_id = Some(SheetId::new("col-1"));
        app.split.secondary_sheet_id = Some(SheetId::new("res-1"));
        app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
        app.sync_views();

        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let ctx = egui::Context::default();
        ctx.set_fonts(build_driver_font_definitions());
        let vp = egui::vec2(1360.0, 800.0);

        let (_, meta0) = settle_shell_raw_frames(&ctx, &mut app, vp, 0.05);
        let data_cell_rect = find_widget_in_pane(&meta0, "center.pane.primary", "grid.cell.1.0.0").ok_or("missing primary grid.cell.1.0.0")?;
        let res_cell_rect = find_widget_in_pane(&meta0, "center.pane.secondary", "result.cell.1.0").ok_or("missing secondary result.cell.1.0")?;

        // 1. Click Data cell in Primary -> Primary emphasized (active_bg + focus_ring), active_sheet_id = col-1.
        let _ = run_shell_raw_frame(
            &ctx,
            &mut app,
            vp,
            vec![
                egui::Event::PointerMoved(data_cell_rect.center()),
                egui::Event::PointerButton {
                    pos: data_cell_rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: data_cell_rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ],
            0.12,
        );
        let (shapes1, meta1) = settle_shell_raw_frames(&ctx, &mut app, vp, 0.14);
        assert_eq!(app.egui_views.center_stage.focused_split_pane, SplitPaneSlot::Primary);
        assert_eq!(app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
        let prim_rect = meta1.find_by_widget_id("center.pane.primary").unwrap().clipped_rect;
        let sec_rect = meta1.find_by_widget_id("center.pane.secondary").unwrap().clipped_rect;
        assert!(shapes1.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.active_bg && prim_rect.contains(r.rect.center()))));
        assert!(shapes1.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.stroke.color == tokens.focus_ring && (r.stroke.width - 2.0).abs() < 0.05 && prim_rect.contains(r.rect.center()))));

        // 2. Click Results cell in Secondary -> Secondary emphasized, Primary Data switches to selection_unemphasized_bg (no focus_ring!).
        let res_cell_rect2 = find_widget_in_pane(&meta1, "center.pane.secondary", "result.cell.1.0").ok_or("missing secondary result.cell.1.0")?;
        let _ = run_shell_raw_frame(
            &ctx,
            &mut app,
            vp,
            vec![
                egui::Event::PointerMoved(res_cell_rect2.center()),
                egui::Event::PointerButton {
                    pos: res_cell_rect2.center(),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: res_cell_rect2.center(),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ],
            0.18,
        );
        let (shapes2, meta2) = settle_shell_raw_frames(&ctx, &mut app, vp, 0.20);
        assert_eq!(app.egui_views.center_stage.focused_split_pane, SplitPaneSlot::Secondary);
        assert_eq!(app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("res-1"));
        let prim_body2 = meta2.find_by_widget_id("center.pane.primary").unwrap().clipped_rect;
        let sec_body2 = meta2.find_by_widget_id("center.pane.secondary").unwrap().clipped_rect;
        let data_cell_2 = find_widget_in_pane(&meta2, "center.pane.primary", "grid.cell.1.0.0").unwrap();
        assert!(
            shapes2
                .iter()
                .any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.selection_unemphasized_bg && data_cell_2.contains(r.rect.center()))),
            "{mode:?}: unfocused Primary Data cell must paint selection_unemphasized_bg"
        );
        assert!(
            !shapes2.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.stroke.color == tokens.focus_ring && (r.stroke.width - 2.0).abs() < 0.05 && prim_body2.contains(r.rect.center()))),
            "{mode:?}: unfocused Primary Data pane must not paint blue focus_ring"
        );
        assert!(
            shapes2.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.active_bg && sec_body2.contains(r.rect.center()))),
            "{mode:?}: focused Secondary Results cell must paint active_bg"
        );

        // 3. Shift+F6 cycles back to Primary -> doc.active_sheet_id = col-1, Results switches to selection_unemphasized_bg (+ HighContrast dashed!).
        let _ = run_shell_raw_frame(
            &ctx,
            &mut app,
            vp,
            vec![egui::Event::Key { key: Key::F6, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::SHIFT }],
            0.24,
        );
        let (shapes3, meta3) = settle_shell_raw_frames(&ctx, &mut app, vp, 0.26);
        assert_eq!(app.egui_views.center_stage.focused_split_pane, SplitPaneSlot::Primary);
        assert_eq!(app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
        assert_eq!(app.split.effective_inspector_target(app.document()).map(SheetId::as_str), Some("col-1"));
        let res_cell_3 = find_widget_in_pane(&meta3, "center.pane.secondary", "result.cell.1.0").unwrap();
        assert!(
            shapes3
                .iter()
                .any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.selection_unemphasized_bg && res_cell_3.contains(r.rect.center()))),
            "{mode:?}: unfocused Secondary Results cell must paint selection_unemphasized_bg"
        );
        if mode == UiThemeMode::HighContrast {
            let hc_dashed_res = shapes3
                .iter()
                .filter(|s| matches!(s, egui::Shape::LineSegment { points, stroke } if stroke.color == tokens.border && (stroke.width - 1.0).abs() < 0.05 && res_cell_3.expand(2.0).contains(points[0])))
                .count();
            assert!(hc_dashed_res >= 4, "HighContrast unfocused Results cell must draw dashed border, got {hc_dashed_res}");
        }

        // 4. F6 cycles to Secondary -> doc.active_sheet_id = res-1, Inspector targets res-1, Primary Data shows unemphasized (+ HighContrast dashed!).
        let _ = run_shell_raw_frame(
            &ctx,
            &mut app,
            vp,
            vec![egui::Event::Key { key: Key::F6, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
            0.30,
        );
        let (shapes4, meta4) = settle_shell_raw_frames(&ctx, &mut app, vp, 0.32);
        assert_eq!(app.egui_views.center_stage.focused_split_pane, SplitPaneSlot::Secondary);
        assert_eq!(app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("res-1"));
        assert_eq!(app.split.effective_inspector_target(app.document()).map(SheetId::as_str), Some("res-1"));
        let data_cell_4 = find_widget_in_pane(&meta4, "center.pane.primary", "grid.cell.1.0.0").unwrap();
        assert!(
            shapes4
                .iter()
                .any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.selection_unemphasized_bg && data_cell_4.contains(r.rect.center())))
        );
        if mode == UiThemeMode::HighContrast {
            let hc_dashed_data = shapes4
                .iter()
                .filter(|s| matches!(s, egui::Shape::LineSegment { points, stroke } if stroke.color == tokens.border && (stroke.width - 1.0).abs() < 0.05 && data_cell_4.expand(2.0).contains(points[0])))
                .count();
            assert!(hc_dashed_data >= 4, "HighContrast unfocused Data cell must draw dashed border, got {hc_dashed_data}");
        }
        let _ = (res_cell_rect, sec_rect);
    }
    Ok(())
}

#[test]
fn r20_11_header_coordinate_ownership_results_omission_and_graph_brush_pill() -> BoxRes {
    let mut app = make_full_project_app()?;
    let other_ds = make_column_sheet("col-other", "Unlinked Table", 2, 4)?;
    {
        let mut doc = app.document().clone();
        doc.add_data_sheet(other_ds)?;
        doc.select_in_navigator(&SheetId::new("col-1"))?;
        *app.session_mut() = Session::from_document(doc);
    }
    app.nav.welcome.open = false;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("col-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("res-1"));
    app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    app.sync_views();

    let mut driver = make_shell_driver(app, ViewportMetrics::new(1360.0, 800.0, 1.0));
    // 1. Data || Results: DataSheet owns its R{r}C{c} coordinate; ResultSheet header NEVER displays DataSheet/brushed coords.
    click_cell_in_pane(&mut driver, "center.pane.primary", "grid.cell.2.1.0")?;
    let coord_p = driver.frame_meta().find_by_widget_id("split.header.coord.primary").ok_or("missing DataSheet primary header coord")?;
    assert_eq!(coord_p.label, "R3C2");
    assert!(driver.frame_meta().find_by_widget_id("split.header.coord.secondary").is_none());
    assert!(driver.frame_meta().find_by_widget_id("split.header.brush.secondary").is_none());
    assert!(driver.frame_meta().find_by_widget_id("split.clear_brush").is_none());

    driver.press_egui_key(Key::F6, Modifiers::NONE);
    assert_eq!(driver.state().views.center_stage.focused_split_pane, SplitPaneSlot::Secondary);
    let coord_p_unfocused =
        driver.frame_meta().find_by_widget_id("split.header.coord.primary").ok_or("DataSheet header coord must stay on DataSheet")?;
    assert_eq!(coord_p_unfocused.label, "R3C2");
    assert!(driver.frame_meta().find_by_widget_id("split.header.coord.secondary").is_none());
    assert!(driver.frame_meta().find_by_widget_id("split.header.brush.secondary").is_none());

    // 2. Linked Data || Graph (`col-1` -> `g-1`): Graph header displays `Brushed: R{r}C{c}` + `split.clear_brush` `×`.
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
    driver.state_mut().app.split.secondary_sheet_id = Some(SheetId::new("g-1"));
    driver.state_mut().app.sync_views();
    driver.run_settled();

    click_cell_in_pane(&mut driver, "center.pane.primary", "grid.cell.1.1.0")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("split.header.coord.primary").ok_or("missing primary coord")?.label, "R2C2");
    let brush_pill = driver.frame_meta().find_by_widget_id("split.header.brush.secondary").ok_or("missing linked Graph brushed pill")?;
    assert_eq!(brush_pill.label, "Brushed: R2C2");
    assert!(driver.frame_meta().find_by_widget_id("split.clear_brush").is_some());

    driver.click_widget_id("split.clear_brush")?;
    assert!(driver.state().app.brushed_cell.is_none());
    assert!(driver.frame_meta().find_by_widget_id("split.header.brush.secondary").is_none());
    assert!(driver.frame_meta().find_by_widget_id("split.clear_brush").is_none());

    // 3. Unlinked Data || Graph (`col-other` not in `doc.upstream_dependencies("g-1")`): Graph header must NOT show `Brushed:`.
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "col-other" }))?;
    driver.state_mut().app.split.primary_sheet_id = Some(SheetId::new("col-other"));
    driver.state_mut().app.sync_views();
    driver.run_settled();
    click_cell_in_pane(&mut driver, "center.pane.primary", "grid.cell.0.0.0")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("split.header.coord.primary").ok_or("missing col-other coord")?.label, "R1C1");
    assert!(driver.frame_meta().find_by_widget_id("split.header.brush.secondary").is_none());
    assert!(driver.frame_meta().find_by_widget_id("split.clear_brush").is_none());
    Ok(())
}

#[test]
fn r20_20_compact_split_f6_and_shift_f6_landmark_traversal() -> BoxRes {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("col-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("g-1"));
    app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    app.sync_views();

    let mut driver = make_shell_driver(app, ViewportMetrics::new(640.0, 420.0, 1.0));
    assert!(
        driver.frame_meta().find_by_widget_id("split.compact.primary").is_some()
            && driver.frame_meta().find_by_widget_id("split.compact.secondary").is_some(),
        "640x420 must render CompactSinglePane split header"
    );

    // Forward F6 must leave CenterStage immediately without stalling and traverse all visible landmarks back to MainCanvasOrGrid.
    let expected_order = driver.state().app.visible_workspace_panes();
    assert!(expected_order.len() >= 3);
    let mut visited_fwd = Vec::new();
    for _ in 0..expected_order.len() {
        let prev = driver.state().app.nav.active_pane;
        driver.press_egui_key(Key::F6, Modifiers::NONE);
        let next = driver.state().app.nav.active_pane;
        assert_ne!(next, prev, "compact split F6 stalled on {prev:?}");
        visited_fwd.push(next);
        assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
        assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(SheetId::as_str), Some("g-1"));
        assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
    }
    for pane in &expected_order {
        assert!(visited_fwd.contains(pane), "compact F6 missed landmark {pane:?}");
    }
    assert_eq!(driver.state().app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);

    // Switch visible compact pane to Secondary (`g-1`) and verify reverse Shift+F6 traverses cleanly without stalling.
    driver.click_widget_id("split.compact.secondary")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("g-1"));
    let expected_rev = driver.state().app.visible_workspace_panes();
    let mut visited_rev = Vec::new();
    for _ in 0..expected_rev.len() {
        let prev = driver.state().app.nav.active_pane;
        driver.press_egui_key(Key::F6, Modifiers::SHIFT);
        let next = driver.state().app.nav.active_pane;
        assert_ne!(next, prev, "compact split Shift+F6 stalled on {prev:?}");
        visited_rev.push(next);
        assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
        assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(SheetId::as_str), Some("g-1"));
        assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("g-1"));
    }
    for pane in &expected_rev {
        assert!(visited_rev.contains(pane), "compact Shift+F6 missed landmark {pane:?}");
    }
    assert_eq!(driver.state().app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
    Ok(())
}

#[test]
fn r20_13_data_results_content_aware_initial_split_ratio_and_manual_drag_preservation() -> BoxRes {
    let mut doc = ProjectDocument::default();
    let ds = make_column_sheet("col-wide", "Multi-Group Assay", 6, 8)?;
    let mut rs_narrow = ResultSheet::new(AnalysisSpec::new("res-narrow", "col-wide", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    rs_narrow.status = ResultStatus::UpToDate;
    rs_narrow.tabs = vec![
        ResultTab {
            id: String::from("summary"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Val")],
            row_labels: vec![String::from("Mean"), String::from("SD")],
            rows: vec![vec![num_result_cell(4.2)], vec![num_result_cell(1.1)]],
            footnotes: Vec::new(),
        },
        ResultTab {
            id: String::from("details"),
            title: String::from("Details"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("A"), String::from("B"), String::from("C"), String::from("D"), String::from("E")],
            row_labels: vec![String::from("Metric 1")],
            rows: vec![vec![num_result_cell(1.0); 5]],
            footnotes: Vec::new(),
        },
    ];
    let mut rs_wide = ResultSheet::new(AnalysisSpec::new("res-wide", "col-wide", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    rs_wide.status = ResultStatus::UpToDate;
    rs_wide.tabs = vec![ResultTab {
        id: String::from("wide_ci"),
        title: String::from("Wide Summary"),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: vec![
            String::from("95% CI Group A"),
            String::from("95% CI Group B"),
            String::from("95% CI Group C"),
            String::from("95% CI Group D"),
            String::from("95% CI Group E"),
        ],
        row_labels: vec![String::from("Parameter Estimate")],
        rows: vec![vec![ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.23), upper: CiBound::Finite(4.56) }; 5]],
        footnotes: Vec::new(),
    }];

    doc.add_data_sheet(ds)?;
    doc.add_result_sheet(rs_narrow)?;
    doc.add_result_sheet(rs_wide)?;
    doc.select_in_navigator(&SheetId::new("col-wide"))?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.sync_views();

    let mut driver = make_shell_driver(app, ViewportMetrics::new(1360.0, 800.0, 1.0));
    driver.click_widget_id("toolbar.split")?;
    driver.click_widget_id("toolbar.split.preset.data_results")?;

    // 1. Narrow Results table gets capped to content width (< 50%), giving surplus width (> 50%, <= 72%) to wide Data table.
    let initial_ratio = driver.state().app.split.ratio;
    assert!(initial_ratio > 0.52 && initial_ratio <= 0.72, "expected content-aware Data || res-narrow ratio in (0.52, 0.72], got {initial_ratio}");
    let prim_rect = driver.frame_meta().find_by_widget_id("center.pane.primary").unwrap().clipped_rect;
    let sec_rect = driver.frame_meta().find_by_widget_id("center.pane.secondary").unwrap().clipped_rect;
    assert!(prim_rect.width() > sec_rect.width(), "Primary Data pane must receive surplus width over narrow Results");

    // 2. Manual divider drag updates ratio and is NEVER overridden on subsequent frames, tab switches, or F6 cycling.
    let div_pt = widget_center(&driver, "split.divider")?;
    let stage_left = prim_rect.left();
    let total_stage_w = sec_rect.right() - prim_rect.left();
    let target_x = stage_left + total_stage_w * 0.44;
    driver.drag(div_pt, pos2(target_x, div_pt.y))?;
    let manual_ratio = driver.state().app.split.ratio;
    assert!((manual_ratio - 0.44).abs() < 0.04, "expected manual ratio ~0.44, got {manual_ratio}");

    driver.click_widget_id("result.tab.details")?;
    driver.press_egui_key(Key::F6, Modifiers::NONE);
    driver.run_settled();
    assert!(
        (driver.state().app.split.ratio - manual_ratio).abs() < 1e-4,
        "manual user split ratio {manual_ratio} must not be auto-overridden after tab switch or F6 (got {})",
        driver.state().app.split.ratio
    );

    // 3. Switching sheet pair to wide Results (`res-wide`) computes new initial ratio within [0.35, 0.72] favoring wide Results (< 0.50).
    driver.click_widget_id("split.secondary_selector")?;
    let wide_entry = driver.frame_meta().entries.iter().find(|e| e.label.contains("res-wide")).map(|e| e.clipped_rect.center());
    if let Some(p) = wide_entry {
        driver.click_at(p)?;
    } else {
        driver.state_mut().app.split.secondary_sheet_id = Some(SheetId::new("res-wide"));
        driver.state_mut().app.sync_views();
        driver.run_settled();
    }
    let wide_ratio = driver.state().app.split.ratio;
    assert!((0.35..0.50).contains(&wide_ratio), "expected wide Results pair ratio in [0.35, 0.50), got {wide_ratio}");
    Ok(())
}

mod r10_35_portfolio_gallery {
    use curvebook_doc::{
        AnalysisKind, Cell, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, DescriptiveSpec, GraphKind, GraphSheet, InfoSheet, LayoutItem,
        LayoutSheet, LayoutSpec, ProjectDocument, RectPt, SheetFolderKind, SheetId, SubcolumnMode, TableArchetype, XColumnMode, XyTable,
    };
    use curvebook_engine::Session;
    use curvebook_palette::Rgba8;
    use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, CsvExportTargetKind};
    use curvebook_ui_egui::egui_shell::Tokens;
    use curvebook_ui_egui::egui_shell::driver::extract_rendered_text_runs;
    use curvebook_ui_egui::interaction::KeyModifiers;
    use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec};
    use curvebook_ui_egui::workspace_nav::GallerySortOrder;
    use curvebook_ui_egui::{PointerButton, UiApp};
    use egui::{Color32, Key, Modifiers, Shape};
    use serde_json::json;

    use crate::common::{ShellTestState, ensure, make_default_driver, make_shell_driver_with_state};

    type BoxRes = Result<(), String>;

    fn seed_rich_gallery_app() -> Result<UiApp, String> {
        let mut col = ColumnTable::new(ColumnDesignHint::default(), 2, 2).map_err(|e| e.to_string())?;
        col.data_sets[0].title = String::from("Dose");
        col.data_sets[1].title = String::from("Response");
        col.data_sets[0].cells[0][0] = Cell::number(10.0);
        col.data_sets[1].cells[0][0] = Cell::number(95.5);
        col.data_sets[0].cells[1][0] = Cell::number(20.0);
        col.data_sets[1].cells[1][0] = Cell::number(82.0);
        let mut ds_frozen =
            DataSheet::new(SheetId::new("ds-frozen"), "Bravo Frozen Table", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
        ds_frozen.frozen = true;
        ds_frozen.color_tag = Some(Rgba8::rgb(220, 38, 38));

        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 3)
            .map_err(|e| e.to_string())?;
        xy.x_title = String::from("Time");
        xy.data_sets[0].title = String::from("Signal");
        for (r, (x, y)) in [(1.0, 4.0), (2.0, 9.0), (3.0, 15.0)].into_iter().enumerate() {
            xy.x_cells[r][0] = Cell::number(x);
            xy.data_sets[0].cells[r][0] = Cell::number(y);
        }
        let mut ds_live = DataSheet::new(SheetId::new("ds-live"), "Alpha Live XY", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
        ds_live.color_tag = Some(Rgba8::rgb(22, 163, 74));

        let mut info = InfoSheet::new_preseeded(SheetId::new("info-exp"), "Echo Assay Info").map_err(|e| e.to_string())?;
        info.linked_data_sheets.push(SheetId::new("ds-live"));
        info.constants[0].name = String::from("Concentration");
        info.constants[0].value = ConstantValue::Number(50.0);
        info.notes_markdown = String::from("Assay temperature 37C");
        info.color_tag = Some(Rgba8::rgb(234, 88, 12));

        let mut gs_init = GraphSheet::new_uninitialized_for_data(SheetId::new("gs-init"), &ds_live).map_err(|e| e.to_string())?;
        gs_init.title = String::from("Delta Dose Curve");
        gs_init.initialized = true;
        gs_init.spec.kind = GraphKind::Xy;
        gs_init.color_tag = Some(Rgba8::rgb(147, 51, 234));

        let mut gs_uninit = GraphSheet::new_uninitialized_for_data(SheetId::new("gs-uninit"), &ds_frozen).map_err(|e| e.to_string())?;
        gs_uninit.title = String::from("Foxtrot Pending Graph");
        gs_uninit.initialized = false;

        let mut lay_spec = LayoutSpec::default();
        lay_spec.items.push(LayoutItem::LiveGraph {
            id: String::from("panel-1"),
            graph_sheet_id: SheetId::new("gs-init"),
            bounds_pt: RectPt::new(16.0, 16.0, 180.0, 120.0),
            scale_percent: 100.0,
            badge_override: Some(String::from("A")),
        });
        let mut lay = LayoutSheet::new(SheetId::new("lay-main"), "Gamma Summary Layout", lay_spec).map_err(|e| e.to_string())?;
        lay.color_tag = Some(Rgba8::rgb(13, 148, 136));

        let mut doc = ProjectDocument::new("Portfolio Gallery Workspace").map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds_frozen).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds_live).map_err(|e| e.to_string())?;
        doc.add_info_sheet(info).map_err(|e| e.to_string())?;
        doc.add_graph_sheet(gs_init).map_err(|e| e.to_string())?;
        doc.add_graph_sheet(gs_uninit).map_err(|e| e.to_string())?;
        doc.add_layout_sheet(lay).map_err(|e| e.to_string())?;
        doc.active_sheet_id = Some(SheetId::new("ds-live"));
        doc.family_anchor_sheet_id = Some(SheetId::new("ds-live"));

        let mut session = Session::from_document(doc);
        let kind_val = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
        session
            .execute("analysis.run", json!({ "sheet_id": "res-ok", "source_sheet_id": "ds-frozen", "kind": kind_val }))
            .map_err(|e| e.to_string())?;
        let mut res_doc = session.document().clone();
        if let Some(rs) = res_doc.result_sheet_mut(&SheetId::new("res-ok")) {
            rs.color_tag = Some(Rgba8::rgb(217, 119, 6));
        }
        res_doc.select_in_navigator(&SheetId::new("ds-live")).map_err(|e| e.to_string())?;
        let mut app = UiApp::from_session(Session::from_document(res_doc));
        app.sync_views();
        Ok(app)
    }

    fn collect_mesh_count(shapes: &[egui::epaint::ClippedShape]) -> usize {
        fn walk(shape: &Shape, out: &mut usize) {
            match shape {
                Shape::Mesh(_) => *out += 1,
                Shape::Vec(v) => {
                    for s in v {
                        walk(s, out);
                    }
                }
                _ => {}
            }
        }
        let mut n = 0;
        for cs in shapes {
            walk(&cs.shape, &mut n);
        }
        n
    }

    fn collect_circle_fills(shapes: &[egui::epaint::ClippedShape]) -> Vec<Color32> {
        fn walk(shape: &Shape, out: &mut Vec<Color32>) {
            match shape {
                Shape::Circle(c) if c.fill.a() > 0 => out.push(c.fill),
                Shape::Vec(v) => {
                    for s in v {
                        walk(s, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for cs in shapes {
            walk(&cs.shape, &mut out);
        }
        out
    }

    fn collect_rect_strokes_and_fills(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Rect, Color32, Color32, f32)> {
        fn walk(shape: &Shape, out: &mut Vec<(egui::Rect, Color32, Color32, f32)>) {
            match shape {
                Shape::Rect(r) => out.push((r.rect, r.fill, r.stroke.color, r.stroke.width)),
                Shape::Vec(v) => {
                    for s in v {
                        walk(s, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for cs in shapes {
            walk(&cs.shape, &mut out);
        }
        out
    }

    fn collect_rendered_text_colors(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Color32)> {
        fn walk(shape: &Shape, out: &mut Vec<(String, Color32)>) {
            match shape {
                Shape::Text(t) => {
                    let text = t.galley.job.text.clone();
                    if let Some(col) = t.override_text_color.filter(|c| *c != Color32::PLACEHOLDER) {
                        out.push((text, col));
                    } else {
                        for sec in &t.galley.job.sections {
                            out.push((text.clone(), sec.format.color));
                        }
                    }
                }
                Shape::Vec(v) => {
                    for s in v {
                        walk(s, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for cs in shapes {
            walk(&cs.shape, &mut out);
        }
        out
    }

    #[test]
    fn r10_35_live_thumbnails_color_tags_and_status_badges_all_five_kinds() -> BoxRes {
        let mut app = seed_rich_gallery_app()?;
        app.nav.gallery.open = true;
        app.sync_views();
        let mut st = ShellTestState::new(app);
        st.synchronous_raster = true;
        let mut driver =
            make_shell_driver_with_state(st, Default::default(), Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), "en"));

        let initial_parity = driver.verify_pixel_to_hit_box_parity();
        ensure(initial_parity.is_clean(), format!("gallery initial parity failed: {:?}", initial_parity.violations))?;

        // Collapse Navigator & flatten via Sort = Name A-Z so all 5 sheet kinds and 7 cards fit simultaneously on screen.
        driver.click_widget_id("nav.collapse_rail")?;
        driver.click_widget_id("gallery.sort")?;
        driver.click_widget_id("gallery.sort.name_az")?;

        let parity = driver.verify_pixel_to_hit_box_parity();
        ensure(parity.is_clean(), format!("gallery flat parity failed: {:?}", parity.violations))?;

        let runs = extract_rendered_text_runs(driver.clipped_shapes());
        let joined: String = runs.iter().map(|r| r.full_text.as_str()).collect::<Vec<_>>().join(" | ");
        for expected in ["Dose", "95.5", "2 × 2", "Concentration: 50", "Assay temperature", "Warn", "Frozen", "Uninitialized", "Active"] {
            ensure(joined.contains(expected), format!("missing live thumbnail or badge text `{expected}` in rendered gallery: {joined}"))?;
        }

        ensure(collect_mesh_count(driver.clipped_shapes()) >= 2, "expected textured raster meshes for GraphSheet & LayoutSheet thumbnails")?;

        let circles = collect_circle_fills(driver.clipped_shapes());
        for (label, col) in [
            ("Data", Color32::from_rgb(220, 38, 38)),
            ("Info", Color32::from_rgb(234, 88, 12)),
            ("Result", Color32::from_rgb(217, 119, 6)),
            ("Graph", Color32::from_rgb(147, 51, 234)),
            ("Layout", Color32::from_rgb(13, 148, 136)),
        ] {
            ensure(circles.contains(&col), format!("missing color_tag circle for {label}"))?;
        }

        // R20.14: zero blue non-link text inside gallery text runs.
        let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), "en");
        for (txt, col) in collect_rendered_text_colors(driver.clipped_shapes()) {
            ensure(col != tokens.accent && col != tokens.link_blue, format!("R20.14 violation: found blue text `{txt}` ({col:?})"))?;
        }

        // R20.15: zero trailing ellipsis on toolbar/batch action buttons.
        for entry in driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("gallery.") && e.role == "button") {
            ensure(
                !entry.label.ends_with('…') && !entry.label.ends_with("..."),
                format!("R20.15 violation: gallery button `{}` ends with ellipsis (`{}`)", entry.widget_id, entry.label),
            )?;
        }
        Ok(())
    }

    #[test]
    fn r10_35_corner_checkbox_shift_click_and_active_vs_selected_across_all_themes() -> BoxRes {
        for mode in UiThemeMode::ALL {
            let mut app = seed_rich_gallery_app()?;
            app.nav.gallery.open = true;
            app.sync_views();
            let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), "en");
            let mut st = ShellTestState::new(app);
            st.synchronous_raster = true;
            let mut driver = make_shell_driver_with_state(st, Default::default(), tokens);
            driver.click_widget_id("nav.collapse_rail")?;
            driver.click_widget_id("gallery.sort")?;
            driver.click_widget_id("gallery.sort.name_az")?;

            // Corner checkbox toggles selection without closing Gallery.
            driver.click_widget_id("gallery.card.select.gs-init")?;
            ensure(driver.state().app.nav.gallery.open, "corner checkbox click must keep Gallery open")?;
            ensure(
                driver.state().app.nav.selected_gallery_ids() == vec![SheetId::new("gs-init")],
                format!("expected gs-init selected via corner checkbox in {mode:?}"),
            )?;

            driver.click_widget_id("gallery.card.select.lay-main")?;
            let selected_two = driver.state().app.nav.selected_gallery_ids();
            ensure(
                selected_two == vec![SheetId::new("gs-init"), SheetId::new("lay-main")],
                format!("expected [gs-init, lay-main] selected in {mode:?}, got {selected_two:?}"),
            )?;

            // Shift+Click range selection on card body agrees with unified visual card order.
            let p_target = driver.frame_meta().find_by_widget_id("gallery.card.ds-frozen").ok_or("missing ds-frozen")?.clipped_rect.center();
            driver.click_at_button(p_target, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
            ensure(driver.state().app.nav.selected_gallery_ids().len() >= 4, format!("Shift+Click should expand range selection in {mode:?}"))?;

            // Clear then select only gs-init (`!active && selected`) while ds-live remains `active && !selected`.
            driver.click_widget_id("gallery.clear_selection")?;
            driver.click_widget_id("gallery.card.select.gs-init")?;
            let active_rect = driver.frame_meta().find_by_widget_id("gallery.card.ds-live").ok_or("missing active card")?.clipped_rect;
            let selected_rect = driver.frame_meta().find_by_widget_id("gallery.card.gs-init").ok_or("missing selected card")?.clipped_rect;
            let rects = collect_rect_strokes_and_fills(driver.clipped_shapes());
            let active_style = rects
                .iter()
                .find(|(r, _, _, w)| (*r == active_rect) && *w >= 1.4)
                .ok_or_else(|| format!("missing active card frame rect in {mode:?}"))?;
            let selected_style = rects
                .iter()
                .find(|(r, _, _, w)| (*r == selected_rect) && *w >= 1.9)
                .ok_or_else(|| format!("missing selected card frame rect in {mode:?}"))?;
            ensure(
                active_style.2 != selected_style.2 || active_style.1 != selected_style.1,
                format!("active and selected cards must be visually distinct in {mode:?}"),
            )?;

            let parity = driver.verify_pixel_to_hit_box_parity();
            ensure(parity.is_clean(), format!("theme {mode:?} parity failed: {:?}", parity.violations))?;
        }
        Ok(())
    }

    #[test]
    fn r10_35_sort_collapsible_sections_folder_counts_and_current_family_filter() -> BoxRes {
        let mut app = seed_rich_gallery_app()?;
        app.nav.gallery.open = true;
        app.sync_views();
        let mut driver = make_default_driver(app);

        // Filter pills show per-folder counts.
        let all_lbl = &driver.frame_meta().find_by_widget_id("gallery.filter.all").ok_or("all pill")?.label;
        ensure(all_lbl.contains("(7)"), format!("expected `All (7)` pill label, got `{all_lbl}`"))?;
        let graphs_lbl = &driver.frame_meta().find_by_widget_id("gallery.filter.graphs").ok_or("graphs pill")?.label;
        ensure(graphs_lbl.contains("(2)"), format!("expected `Graphs (2)` pill label, got `{graphs_lbl}`"))?;

        // Locked Decision #1: collapsible section headers appear in All + NavigatorOrder.
        for sec in ["gallery.section.data", "gallery.section.info"] {
            ensure(driver.frame_meta().find_by_widget_id(sec).is_some(), format!("missing visible header `{sec}`"))?;
        }
        // Collapse Data, Info, then Results so Graphs and Layouts scroll into view cleanly.
        driver.click_widget_id("gallery.section.data")?;
        driver.click_widget_id("gallery.section.info")?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.section.results").is_some(), "missing visible header `gallery.section.results`")?;
        driver.click_widget_id("gallery.section.results")?;
        for sec in ["gallery.section.graphs", "gallery.section.layouts"] {
            ensure(driver.frame_meta().find_by_widget_id(sec).is_some(), format!("missing collapsible header `{sec}`"))?;
        }
        // Collapsing Graphs hides graph cards without affecting other folders.
        driver.click_widget_id("gallery.section.graphs")?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.card.gs-init").is_none(), "collapsed Graphs section must hide cards")?;
        driver.click_widget_id("gallery.section.graphs")?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.card.gs-init").is_some(), "re-expanding Graphs must restore cards")?;
        // Re-expand sections bottom-up so each header remains in view when clicked.
        driver.click_widget_id("gallery.section.results")?;
        driver.click_widget_id("gallery.section.info")?;
        driver.click_widget_id("gallery.section.data")?;

        // Sort -> Type preserves collapsible section headers.
        driver.click_widget_id("gallery.sort")?;
        driver.click_widget_id("gallery.sort.type")?;
        ensure(driver.state().app.nav.gallery.sort_order == GallerySortOrder::Type, "expected Sort = Type")?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.section.graphs").is_some(), "Type sort keeps collapsible headers")?;

        // Sort -> Name A-Z flattens to one continuous grid with zero section headers.
        driver.click_widget_id("gallery.sort")?;
        driver.click_widget_id("gallery.sort.name_az")?;
        ensure(driver.state().app.nav.gallery.sort_order == GallerySortOrder::NameAsc, "expected Sort = NameAsc")?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.section.graphs").is_none(), "NameAsc must flatten away section headers")?;

        // Select All + selected_gallery_ids() agrees with alphabetical A-Z visual order.
        driver.click_widget_id("gallery.select_all")?;
        let selected = driver.state().app.nav.selected_gallery_ids();
        ensure(selected.first() == Some(&SheetId::new("ds-live")), format!("expected `Alpha Live XY` first in Name A-Z order, got {selected:?}"))?;

        // Current Family filter restricts gallery to active sheet `ds-live`'s DAG family (`ds-live`, `info-exp`, `gs-init`, `lay-main`).
        driver.click_widget_id("gallery.filter.current_family")?;
        ensure(driver.state().app.nav.gallery.current_family_only, "Current Family filter must be active")?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.card.gs-init").is_some(), "family member gs-init must be visible")?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.card.ds-frozen").is_none(), "unrelated ds-frozen must be filtered out")?;
        Ok(())
    }

    #[test]
    fn r10_35_batch_split_layout_export_print_csv_dup_freeze_and_cascade_delete() -> BoxRes {
        // 1. Compare in Split View
        {
            let mut app = seed_rich_gallery_app()?;
            app.nav.gallery.open = true;
            app.sync_views();
            let mut driver = make_default_driver(app);
            driver.click_widget_id("gallery.sort")?;
            driver.click_widget_id("gallery.sort.name_az")?;
            driver.click_widget_id("gallery.card.select.ds-frozen")?;
            driver.click_widget_id("gallery.card.select.gs-init")?;
            driver.click_widget_id("gallery.batch.compare_split")?;
            let app = &driver.state().app;
            ensure(app.split.enabled && !app.nav.gallery.open, "Compare in Split View must enable split and close gallery")?;
            ensure(
                app.split.primary_sheet_id == Some(SheetId::new("ds-frozen")) && app.split.secondary_sheet_id == Some(SheetId::new("gs-init")),
                "split pair mismatch",
            )?;
        }

        // 2. Place on New Layout + Export Selected (Target Scope) + Print Selected
        {
            let mut app = seed_rich_gallery_app()?;
            app.nav.gallery.open = true;
            app.sync_views();
            let mut driver = make_default_driver(app);
            driver.click_widget_id("gallery.filter.graphs")?;
            driver.click_widget_id("gallery.card.select.gs-init")?;
            driver.click_widget_id("gallery.card.select.gs-uninit")?;

            // Print Selected
            driver.click_widget_id("gallery.batch.print")?;
            ensure(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Print { .. }), "expected Print modal")?;
            ensure(driver.state().app.canvas_dialogs.print.job.sheet_ids.len() == 2, "Print job must carry 2 selected sheets")?;
            driver.click_widget_id("modal.cancel")?;

            // Export Selected + Target Scope toggle
            driver.click_widget_id("gallery.batch.export")?;
            ensure(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }), "expected ExportFigure modal")?;
            ensure(driver.state().app.egui_views.app_modals.export_selected_gallery_scope, "Target Scope should default to Selected gallery sheets")?;
            driver.click_widget_id("export.scope.active")?;
            ensure(!driver.state().app.egui_views.app_modals.export_selected_gallery_scope, "toggle Active sheet failed")?;
            driver.click_widget_id("export.scope.selected")?;
            ensure(driver.state().app.egui_views.app_modals.export_selected_gallery_scope, "toggle Selected gallery sheets failed")?;
            driver.click_widget_id("export.fmt.pdf")?;
            driver.click_widget_id("modal.ok")?;
            ensure(
                !matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }),
                "ExportFigure should close after batch PDF",
            )?;

            // Place Selected onto New Layout
            let before_layouts = driver.state().app.document().layout_sheets.len();
            driver.click_widget_id("gallery.batch.place_layout")?;
            let app = &driver.state().app;
            ensure(!app.nav.gallery.open, "Place on New Layout must close gallery")?;
            ensure(app.document().layout_sheets.len() == before_layouts + 1, "expected new LayoutSheet created")?;
            let new_lay = app.document().layout_sheets.last().ok_or("missing created layout")?;
            ensure(new_lay.spec.items.len() == 2, "new LayoutSheet must contain 2 graph panels")?;
        }

        // 3. Batch CSV Export, Duplicate, Freeze/Unfreeze, and Cascade Delete Confirmation
        {
            let mut app = seed_rich_gallery_app()?;
            app.nav.gallery.open = true;
            app.sync_views();
            let mut driver = make_default_driver(app);
            driver.click_widget_id("gallery.card.select.ds-frozen")?;
            driver.click_widget_id("gallery.card.select.ds-live")?;

            // Batch CSV Export
            driver.click_widget_id("gallery.batch.export_csv")?;
            ensure(
                matches!(
                    &driver.state().app.canvas_dialogs.active_dialog,
                    ActiveDialog::CsvExportOptions(CsvExportTargetKind::DataTables { sheet_ids }) if sheet_ids.len() == 2
                ),
                "expected CsvExportOptions(DataTables) for 2 selected data tables",
            )?;
            driver.click_widget_id("modal.ok")?;
            ensure(driver.state().app.canvas_dialogs.active_dialog == ActiveDialog::None, "CSV batch export modal should close after OK")?;

            // Freeze selected DataSheets (`ds-live` transitions false -> true)
            driver.click_widget_id("gallery.batch.freeze")?;
            ensure(driver.state().app.document().data_sheet(&SheetId::new("ds-live")).is_some_and(|d| d.frozen), "batch Freeze must freeze ds-live")?;
            // Unfreeze all frozen DataSheets
            driver.click_widget_id("gallery.batch.freeze")?;
            ensure(
                !driver.state().app.document().data_sheet(&SheetId::new("ds-live")).is_some_and(|d| d.frozen),
                "batch Unfreeze must unfreeze ds-live",
            )?;

            // Duplicate selected
            let prev_data_count = driver.state().app.document().data_sheets.len();
            driver.click_widget_id("gallery.batch.duplicate")?;
            ensure(driver.state().app.document().data_sheets.len() == prev_data_count + 2, "batch Duplicate must duplicate both data tables")?;

            // Cascade delete confirmation when deleting root sheets with dependents
            driver.click_widget_id("gallery.batch.delete")?;
            ensure(
                driver.state().views.leading_dock.delete_confirm.is_some(),
                "deleting sheets with downstream dependents must open cascade confirmation modal",
            )?;
            driver.click_widget_id("nav.delete_confirm.confirm")?;
            ensure(driver.state().app.document().data_sheet(&SheetId::new("ds-live")).is_none(), "ds-live must be deleted after confirm")?;
            ensure(driver.state().app.document().graph_sheet(&SheetId::new("gs-init")).is_none(), "downstream gs-init must cascade delete")?;
        }
        Ok(())
    }

    #[test]
    fn r10_35_context_menu_keyboard_nav_footer_fit_and_navigator_folder_gallery() -> BoxRes {
        let mut app = seed_rich_gallery_app()?;
        app.nav.gallery.open = true;
        app.sync_views();
        let mut driver = make_default_driver(app);

        // Footer filtered + selected readout & Fit zoom
        driver.click_widget_id("gallery.card.select.ds-live")?;
        let summary = &driver.frame_meta().find_by_widget_id("status.summary").ok_or("status.summary")?.label;
        ensure(
            summary.contains("shown") && summary.contains("selected"),
            format!("footer summary must report filtered and selected counts, got `{summary}`"),
        )?;
        ensure(driver.frame_meta().find_by_widget_id("status.zoom.fit").is_some(), "footer Fit zoom must be shown in gallery")?;
        driver.click_widget_id("status.zoom.fit")?;

        // Card right-click context menu (`action_menu_popup`, R20.16, dot-free labels R20.15)
        let card_pt = driver.frame_meta().find_by_widget_id("gallery.card.ds-live").ok_or("missing ds-live card")?.clipped_rect.center();
        driver.click_at_button(card_pt, PointerButton::Secondary, KeyModifiers::default())?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.card.context_menu").is_some(), "expected card right-click context menu")?;
        for entry in driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("gallery.ctx.") && e.role == "menuitem") {
            ensure(
                !entry.label.ends_with('…') && !entry.label.ends_with("..."),
                format!("R20.15 violation: context item `{}` ends with ellipsis (`{}`)", entry.widget_id, entry.label),
            )?;
        }
        let ctx_parity = driver.verify_pixel_to_hit_box_parity();
        ensure(ctx_parity.is_clean(), format!("card context menu parity failed: {:?}", ctx_parity.violations))?;

        // Trigger inline Rename from card context menu
        driver.click_widget_id("gallery.ctx.rename")?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.card.rename_input").is_some(), "expected inline rename textbox")?;
        driver.type_text("Renamed XY Sheet")?;
        driver.press_egui_key(Key::Enter, Modifiers::NONE);
        ensure(
            driver.state().app.document().data_sheet(&SheetId::new("ds-live")).is_some_and(|s| s.title == "Renamed XY Sheet"),
            "inline card rename must commit sheet.rename",
        )?;

        // Keyboard grid navigation: Cmd+A -> ArrowRight -> Space -> Enter
        driver.press_egui_key(Key::A, Modifiers::COMMAND);
        ensure(driver.state().app.nav.selected_gallery_ids().len() == 7, "Cmd+A must select all 7 visible gallery cards")?;
        driver.click_widget_id("gallery.clear_selection")?;
        driver.press_egui_key(Key::ArrowRight, Modifiers::NONE);
        driver.press_egui_key(Key::Space, Modifiers::NONE);
        ensure(!driver.state().app.nav.selected_gallery_ids().is_empty(), "Space after ArrowRight must toggle selection")?;
        driver.press_egui_key(Key::Enter, Modifiers::NONE);
        ensure(!driver.state().app.nav.gallery.open, "Enter on focused card must open sheet and close Gallery")?;

        // Navigator Folder Gallery quick-launch opens Gallery filtered to that folder!
        driver.click_widget_id("nav.section.data")?;
        driver.click_widget_id("nav.section.info")?;
        driver.click_widget_id("nav.folder.gallery.graphs")?;
        ensure(
            driver.state().app.nav.gallery.open && driver.state().app.nav.gallery.filter_folder == Some(SheetFolderKind::Graphs),
            "Navigator Folder Gallery button must open Gallery filtered to Graphs",
        )?;
        ensure(driver.frame_meta().find_by_widget_id("gallery.card.gs-init").is_some(), "Graphs card missing after Folder Gallery launch")?;
        ensure(
            driver.frame_meta().find_by_widget_id("gallery.card.ds-live").is_none(),
            "non-Graph cards must be filtered out after Folder Gallery launch",
        )?;

        Ok(())
    }
}
