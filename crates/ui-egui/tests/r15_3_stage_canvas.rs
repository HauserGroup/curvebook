#![forbid(unsafe_code)]

//! Wave 3 (`R15.3`) integration tests for Center Stage 5 modes, Split Comparison (`15%–85%` +
//! two-way brushing + `F6` pane focus ring cycling), Portfolio Gallery, clamped `FloatingNote`
//! cards, `QOL-7` bottom diagnostic/clipping/linked-region banners, `chrono`-free `ProgressModalState`,
//! `38%/62%` `InfoSheet`, multi-tab reactive `ResultSheet`, async `ChartScene` / `LayoutScene`
//! viewport (`KM Number at risk`, `CLD`, `TeX`, `0.1 pt` snap, double-click drill-down +
//! `← Return to Layout`), Wave 2 QA #2 `grid_core` zero-1M-clone navigation, and `14`-locale parity.

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
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::info_view::{InfoViewEguiState, render_info_view};
use curvebook_ui_egui::egui_shell::inspect_bridge::{HybridCoexistenceArbiter, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::results_view::{ResultTabCategory, ResultsViewEguiState, render_results_view};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, LayoutMetrics, PendingUiIntents, ShellLayoutInput, Tokens, UiBackend};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use egui::{Key, Modifiers, pos2};
use serde_json::json;

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
            rows: vec![
                vec![ResultCell::Empty, ResultCell::Empty],
                vec![num_result_cell(14.2), ResultCell::Empty],
                vec![num_result_cell(19.8), ResultCell::Empty],
            ],
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
    layout_spec.items = vec![
        LayoutItem::LiveGraph {
            id: String::from("panel-a"),
            graph_sheet_id: SheetId::new("g-1"),
            bounds_pt: RectPt::new(10.0, 10.0, 110.0, 90.0),
            scale_percent: 100.0,
            badge_override: None,
        },
        LayoutItem::LiveGraph {
            id: String::from("panel-b"),
            graph_sheet_id: SheetId::new("g-1"),
            bounds_pt: RectPt::new(130.0, 10.0, 110.0, 90.0),
            scale_percent: 100.0,
            badge_override: None,
        },
    ];
    let ls = LayoutSheet::new("lay-1", "Figure 1 Layout", layout_spec)?;

    doc.data_sheets = vec![ds];
    doc.info_sheets = vec![info];
    doc.result_sheets = vec![rs];
    doc.graph_sheets = vec![gs];
    doc.layout_sheets = vec![ls];
    doc.active_sheet_id = Some(SheetId::new("col-1"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);
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
                presentation_mode: false,
                empty_workspace: false,
                navigator_open: true,
                inspector_open: true,
                family_open: false,
                leading_user_collapsed: false,
                trailing_user_collapsed: false,
                leading_width_override_pt: None,
                trailing_width_override_pt: None,
                host_draws_menu_bar: false,
            })
        });
        let tokens = Tokens::for_mode(st.app.theme.mode);
        let ctx_in = CenterStageContext {
            tokens: &tokens,
            decision: &decision,
            layout,
            host_textures: None,
            focused_pane: st.app.nav.active_pane,
            enabled: matches!(st.app.canvas_dialogs.active_dialog, ActiveDialog::None) && st.views.progress_modal.is_none(),
        };
        st.report = render_center_stage(ui, &st.app, &mut st.views, ctx_in, meta, &mut st.intents);
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let res = st.intents.commit_if_settled(&mut st.app, false);
            assert!(res.is_ok(), "intent commit failed: {res:?}");
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

#[test]
fn center_stage_five_modes_and_split_15_to_85_clamping_with_brushing() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;

    {
        let mut empty_doc = app.document().clone();
        empty_doc.active_sheet_id = None;
        let views = app.egui_views.clone();
        let empty_app = UiApp::from_session(Session::from_document(empty_doc));
        assert_eq!(resolve_center_stage_mode(&empty_app, &views), CenterStageMode::Welcome);
    }

    let expected = [
        ("col-1", CenterStageMode::ColumnGrid),
        ("info-1", CenterStageMode::InfoSheet),
        ("res-1", CenterStageMode::ResultSheet),
        ("g-1", CenterStageMode::CanvasViewport),
        ("lay-1", CenterStageMode::CanvasViewport),
    ];
    for (sid, mode) in expected {
        app.execute_command("sheet.select", json!({ "sheet_id": sid }))?;
        assert_eq!(resolve_center_stage_mode(&app, &app.egui_views), mode, "unexpected mode for {sid}");
    }

    // Activate Split Comparison: Column table (col-1) left vs GraphSheet (g-1) right at 1280x720.
    app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("col-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("g-1"));
    app.sync_views();

    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(1280.0, 720.0, 1.0), Tokens::default());
    assert!(driver.state().report.handled_natively);
    let prim = driver.state().report.primary_rect_pt.expect("primary rect");
    let sec = driver.state().report.secondary_rect_pt.expect("secondary rect");
    assert!(prim.right() < sec.left(), "wide stage >=720pt must split horizontally");
    assert!(driver.frame_meta().find_by_widget_id("split.primary_selector").is_some());
    assert!(driver.frame_meta().find_by_widget_id("split.secondary_selector").is_some());

    // Drag divider far left -> must clamp at 15% (0.15).
    let div_center = driver.frame_meta().find_by_widget_id("split.divider").expect("divider").clipped_rect.center();
    driver.drag(div_center, pos2(10.0, div_center.y))?;
    let clamped_min = driver.state().views.center_stage.split_ratio_override.expect("split ratio override");
    assert!((clamped_min - 0.15).abs() < 0.02, "expected ~15% left clamp, got {clamped_min}");

    // Drag divider far right -> must clamp at 85% (0.85).
    let div_center = driver.frame_meta().find_by_widget_id("split.divider").expect("divider").clipped_rect.center();
    driver.drag(div_center, pos2(1270.0, div_center.y))?;
    let clamped_max = driver.state().views.center_stage.split_ratio_override.expect("split ratio override");
    assert!((clamped_max - 0.85).abs() < 0.02, "expected ~85% right clamp, got {clamped_max}");

    // Double-click divider -> resets cleanly to 0.50.
    let div_center = driver.frame_meta().find_by_widget_id("split.divider").expect("divider").clipped_rect.center();
    driver.click_at(div_center)?;
    driver.click_at(div_center)?;
    let reset_ratio = driver.state().views.center_stage.split_ratio_override.expect("split ratio override");
    assert!((reset_ratio - 0.50).abs() < 0.05, "expected ~50% reset on double-click, got {reset_ratio}");

    // Verify vertical split fallback when width < 720 pt.
    driver.set_viewport(ViewportMetrics::new(640.0, 620.0, 1.0));
    driver.run_settled();
    let v_prim = driver.state().report.primary_rect_pt.expect("vertical primary");
    let v_sec = driver.state().report.secondary_rect_pt.expect("vertical secondary");
    assert!(v_prim.bottom() < v_sec.top(), "narrow stage <720pt must stack vertically");
    driver.set_viewport(ViewportMetrics::new(1280.0, 720.0, 1.0));
    driver.run_settled();

    // Two-way point brushing: grid cell click mirrors into brushed_cell, then swap & click split col preview.
    driver.click_widget_id("grid.cell.2.1.0")?;
    let after_grid_brush = driver.state().app.brushed_cell.clone().expect("brushed cell from grid");
    assert_eq!((after_grid_brush.row, after_grid_brush.col), (2, 1));

    driver.click_widget_id("split.swap")?;
    assert_eq!(driver.state().app.split.primary_sheet_id.as_ref().map(SheetId::as_str), Some("g-1"));
    assert_eq!(driver.state().app.split.secondary_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));

    // Secondary still renders full interactive ColumnGrid while active_sheet_id == "col-1".
    driver.click_widget_id("grid.cell.3.0.0")?;
    let brushed2 = driver.state().app.brushed_cell.clone().expect("brushed from swapped column pane");
    assert_eq!((brushed2.row, brushed2.col), (3, 0));

    // When active_sheet_id switches to GraphSheet ("g-1"), non-active Column sheet uses split.col_preview and still syncs two-way brushing.
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;
    driver.run_settled();
    driver.click_widget_id("split.col_preview.4.0")?;
    let brushed3 = driver.state().app.brushed_cell.clone().expect("brushed from split col preview");
    assert_eq!((brushed3.row, brushed3.col), (4, 0));

    driver.click_widget_id("split.close")?;
    assert!(!driver.state().app.split.enabled);
    Ok(())
}

#[test]
fn center_stage_portfolio_gallery_search_filter_and_open() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    app.nav.gallery.open = true;
    let doc_clone = app.document().clone();
    app.nav.sync_gallery(&doc_clone);
    app.sync_views();

    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(1120.0, 700.0, 1.0), Tokens::default());
    assert_eq!(driver.state().views.center_stage.last_active_mode, CenterStageMode::Gallery);
    assert!(driver.frame_meta().find_by_widget_id("gallery.card.g-1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("gallery.card.lay-1").is_some());

    driver.click_widget_id("gallery.filter.graphs")?;
    assert_eq!(driver.state().app.nav.gallery.filter_folder, Some(SheetFolderKind::Graphs));
    assert!(driver.frame_meta().find_by_widget_id("gallery.card.g-1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("gallery.card.lay-1").is_none());

    driver.click_widget_id("gallery.filter.all")?;
    driver.click_widget_id("gallery.search_input")?;
    driver.type_text("Figure")?;
    assert!(driver.frame_meta().find_by_widget_id("gallery.card.lay-1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("gallery.card.g-1").is_none());

    driver.click_widget_id("gallery.card.lay-1")?;
    assert!(!driver.state().app.nav.gallery.open);
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("lay-1"));
    Ok(())
}

#[test]
fn center_stage_f6_cycles_split_panes_and_floating_notes() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("col-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("g-1"));
    app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    app.sync_views();

    struct ShellF6Harness {
        app: UiApp,
        views: EguiShellViewState,
        intents: PendingUiIntents,
    }

    let init = ShellF6Harness { views: app.egui_views.clone(), app, intents: PendingUiIntents::new() };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        init,
        ViewportMetrics::new(1360.0, 820.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let res = st.intents.commit_if_settled(&mut st.app, false);
            assert!(res.is_ok(), "intent commit failed: {res:?}");
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    assert_eq!(driver.state().app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
    assert_eq!(driver.state().views.center_stage.focused_split_pane, SplitPaneSlot::Primary);

    driver.press_egui_key(Key::F6, Modifiers::NONE);
    assert_eq!(driver.state().app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
    assert_eq!(driver.state().views.center_stage.focused_split_pane, SplitPaneSlot::Secondary);

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
    assert!(seen_notes, "F6 cycle must visit FloatingNotes when sheet has notes");

    for _ in 0..4 {
        if driver.state().app.nav.active_pane == ActiveWorkspacePane::MainCanvasOrGrid {
            break;
        }
        driver.press_egui_key(Key::F6, Modifiers::SHIFT);
    }
    assert_eq!(driver.state().app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
    assert_eq!(driver.state().views.center_stage.focused_split_pane, SplitPaneSlot::Secondary);
    driver.press_egui_key(Key::F6, Modifiers::SHIFT);
    assert_eq!(driver.state().app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
    assert_eq!(driver.state().views.center_stage.focused_split_pane, SplitPaneSlot::Primary);
    Ok(())
}

#[test]
fn center_stage_floating_note_clamped_reposition_resize_pin_collapse() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "g-1" }))?;
    app.sync_views();

    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(960.0, 620.0, 1.0), Tokens::default());
    let stage = driver.state().report.primary_rect_pt.expect("stage rect");

    let drag_pt = driver.frame_meta().find_by_widget_id("floating_note.g-1.n1.drag_handle").expect("note drag handle").clipped_rect.center();
    driver.drag(drag_pt, pos2(stage.right() + 400.0, stage.bottom() + 400.0))?;
    let card = driver.frame_meta().find_by_widget_id("floating_note.g-1.n1.card").expect("card meta").clipped_rect;
    assert!(card.right() <= stage.right() + 1.0 && card.bottom() <= stage.bottom() + 1.0, "note card escaped bottom-right: {card:?} vs {stage:?}");
    assert!(card.left() >= stage.left() - 1.0 && card.top() >= stage.top() - 1.0);

    let resize_pt = driver.frame_meta().find_by_widget_id("floating_note.g-1.n1.resize_handle").expect("resize handle").clipped_rect.center();
    driver.drag(resize_pt, pos2(stage.right() + 200.0, stage.bottom() + 200.0))?;
    let resized_card = driver.frame_meta().find_by_widget_id("floating_note.g-1.n1.card").expect("resized card").clipped_rect;
    assert!(resized_card.right() <= stage.right() + 1.0 && resized_card.bottom() <= stage.bottom() + 1.0);

    driver.click_widget_id("floating_note.g-1.n1.pin")?;
    assert_eq!(driver.state().views.center_stage.note_pinned.get(&(SheetId::new("g-1"), String::from("n1"))).copied(), Some(true));

    driver.click_widget_id("floating_note.g-1.n1.collapse")?;
    let collapsed_card = driver.frame_meta().find_by_widget_id("floating_note.g-1.n1.card").expect("collapsed card").clipped_rect;
    assert!(collapsed_card.height() <= 28.0, "collapsed note should collapse to header height, got {}", collapsed_card.height());
    Ok(())
}

#[test]
fn center_stage_qol7_bottom_banners_diagnostic_clipping_and_linked() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "res-1" }))?;
    app.sync_views();
    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(1120.0, 680.0, 1.0), Tokens::default());

    let banner_r = driver.state().report.diagnostic_banner_rect_pt.expect("diagnostic banner rect");
    assert!((28.0..=36.0).contains(&banner_r.height()), "QOL-7 banner height must be 28–36 pt, got {}", banner_r.height());
    assert!(driver.frame_meta().find_by_widget_id("result.diagnostic_banner").is_some());
    driver.click_widget_id("result.diagnostic_banner.why")?;
    assert_eq!(driver.state().views.center_stage.banner_details_open.as_ref().map(SheetId::as_str), Some("res-1"));
    driver.click_widget_id("result.diagnostic_banner.dismiss")?;
    assert!(driver.state().report.diagnostic_banner_rect_pt.is_none());

    // GraphSheet clipping banner -> [Fit Range] clears manual invalid bounds.
    let mut app2 = make_full_project_app()?;
    {
        let mut doc = app2.document().clone();
        doc.active_sheet_id = Some(SheetId::new("g-1"));
        doc.graph_sheets[0].spec.x_axis.segments[0].min = Some(50.0);
        doc.graph_sheets[0].spec.x_axis.segments[0].max = Some(10.0);
        *app2.session_mut() = Session::from_document(doc);
        app2.sync_views();
    }
    let mut d2 = make_stage_driver(StageHarness::new(app2), ViewportMetrics::new(1120.0, 680.0, 1.0), Tokens::default());
    assert!(d2.frame_meta().find_by_widget_id("canvas.clipping_banner").is_some());
    d2.click_widget_id("canvas.clipping_banner.fit_range")?;
    let seg = &d2.state().app.document().graph_sheet(&SheetId::new("g-1")).unwrap().spec.x_axis.segments[0];
    assert_eq!(seg.min, None);
    assert_eq!(seg.max, None);

    // DataSheet Linked-Region banner -> [Unlink to Edit] unlinks without clearing data.
    let mut app3 = make_full_project_app()?;
    {
        let mut doc = app3.document().clone();
        let src_sheet = make_column_sheet("col-src", "Source Cohort", 2, 4)?;
        doc.data_sheets.push(src_sheet);
        let range = SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 0));
        doc.data_sheets[0].linked_regions.push(LinkedDataRegion {
            id: String::from("link-1"),
            spec: ImportLinkSpec {
                source: LinkSource::InternalSheet { sheet_id: SheetId::new("col-src"), kind: InternalLinkKind::DataSheet, tab_id: None, range },
                filter: ImportFilterSpec::default(),
                placement: ImportPlacementSpec::default(),
            },
            target_range: range,
            provenance_summary: String::from("From Source Cohort"),
        });
        doc.active_sheet_id = Some(SheetId::new("col-1"));
        *app3.session_mut() = Session::from_document(doc);
        app3.sync_views();
    }
    let mut d3 = make_stage_driver(StageHarness::new(app3), ViewportMetrics::new(1120.0, 680.0, 1.0), Tokens::default());
    assert!(d3.frame_meta().find_by_widget_id("canvas.linked_banner").is_some());
    d3.click_widget_id("canvas.linked_banner.unlink")?;
    assert!(d3.state().app.document().data_sheet(&SheetId::new("col-1")).unwrap().linked_regions.is_empty());
    Ok(())
}

#[test]
fn center_stage_progress_modal_chrono_free_indeterminate_determinate_cancel() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    app.egui_views.progress_modal = Some(ProgressModalState {
        operation_label: String::from("Opening CurveBook Archive"),
        detail: String::from("Parsing 48 sheets…"),
        progress_0_to_1: None,
        cancellable: true,
        cancel_requested: false,
    });
    let mut driver = make_stage_driver(StageHarness::new(app), ViewportMetrics::new(960.0, 600.0, 1.0), Tokens::default());
    assert!(driver.frame_meta().find_by_widget_id("modal.progress.backdrop").is_some());
    assert!(driver.frame_meta().find_by_widget_id("modal.progress.card").is_some());

    driver.state_mut().views.progress_modal.as_mut().unwrap().progress_0_to_1 = Some(0.42);
    driver.run_settled();
    driver.click_widget_id("modal.progress.cancel")?;
    assert!(driver.state().views.progress_modal.as_ref().unwrap().cancel_requested);
    Ok(())
}

#[test]
fn info_sheet_38_62_split_hook_badges_inline_edit_and_formatted_notes() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "info-1" }))?;
    app.sync_views();

    struct InfoHarness {
        app: UiApp,
        info_view: InfoViewEguiState,
        intents: PendingUiIntents,
        table_w: f32,
        notes_w: f32,
        hooks: usize,
    }
    let init = InfoHarness { app, info_view: InfoViewEguiState::default(), intents: PendingUiIntents::new(), table_w: 0.0, notes_w: 0.0, hooks: 0 };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        init,
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
            let res = st.intents.commit_if_settled(&mut st.app, false);
            assert!(res.is_ok(), "canvas intent commit failed: {res:?}");
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    let total = driver.state().table_w + driver.state().notes_w;
    let frac = driver.state().table_w / total.max(1.0);
    assert!((frac - 0.38).abs() < 0.03, "expected ~38% table width, got {frac}");
    assert!(driver.state().hooks > 0, "constant `conc` must detect downstream hooks");

    let div_pt = driver.frame_meta().find_by_widget_id("info.split_divider").expect("info divider").clipped_rect.center();
    driver.drag(div_pt, pos2(div_pt.x + 120.0, div_pt.y))?;
    assert!(driver.state().info_view.split_ratio > 0.45);
    let div_pt2 = driver.frame_meta().find_by_widget_id("info.split_divider").expect("info divider").clipped_rect.center();
    driver.click_at(div_pt2)?;
    driver.click_at(div_pt2)?;
    assert!((driver.state().info_view.split_ratio - 0.38).abs() < 0.02);

    driver.click_widget_id("info.const.0")?;
    driver.click_widget_id("info.const.0")?;
    assert_eq!(driver.state().info_view.editing_key.as_deref(), Some("conc"));
    driver.state_mut().info_view.editing_value = String::from("42.5");
    driver.run_settled();
    driver.click_widget_id("info.const.editor_save")?;
    let updated_val = driver.state().app.document().info_sheet(&SheetId::new("info-1")).unwrap().get_constant("conc").unwrap().value.clone();
    assert_eq!(updated_val, ConstantValue::Number(42.5));

    driver.click_widget_id("info.format.bold")?;
    driver.click_widget_id("info.notes.preview_toggle")?;
    assert!(driver.state().info_view.notes_preview_mode);
    driver.click_widget_id("info.notes.preview_toggle")?;
    driver.click_widget_id("info.notes.save")?;
    let notes_md = &driver.state().app.document().info_sheet(&SheetId::new("info-1")).unwrap().notes_markdown;
    assert!(notes_md.contains("**bold**"), "expected formatted markdown saved, got {notes_md}");

    driver.click_widget_id("info.hook_badge.0")?;
    assert_ne!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("info-1"));
    Ok(())
}

#[test]
fn result_sheet_multi_tab_cmd_t_navigation_ambiguity_blank_median_and_extract() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "res-1" }))?;
    app.sync_views();

    struct ResHarness {
        app: UiApp,
        views: ResultsViewEguiState,
        intents: PendingUiIntents,
        header_h: f32,
        category: Option<ResultTabCategory>,
        has_ambiguity: bool,
        saw_nan: bool,
    }
    let init = ResHarness {
        app,
        views: ResultsViewEguiState::default(),
        intents: PendingUiIntents::new(),
        header_h: 0.0,
        category: None,
        has_ambiguity: false,
        saw_nan: false,
    };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        init,
        ViewportMetrics::new(1040.0, 640.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let tokens = Tokens::for_mode(st.app.theme.mode);
            let rep = render_results_view(ui, &st.app, &mut st.views, &tokens, true, meta, &mut st.intents);
            st.header_h = rep.header_rect_pt.map_or(0.0, |r| r.height());
            st.category = rep.active_tab_category;
            st.has_ambiguity = rep.has_ambiguity_callout;
            if meta.entries.iter().any(|e| e.label.contains("NaN")) {
                st.saw_nan = true;
            }
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let res = st.intents.commit_if_settled(&mut st.app, false);
            assert!(res.is_ok(), "canvas intent commit failed: {res:?}");
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    assert!((driver.state().header_h - 32.0).abs() < 1.0, "expected 32 pt top bar, got {}", driver.state().header_h);
    assert_eq!(driver.state().category, Some(ResultTabCategory::InferentialReport));
    assert!(!driver.state().saw_nan, "unreached survival median must stay blank, never NaN");

    // Collapsible section row toggles cleanly via `result.section.0`.
    driver.click_widget_id("result.section.0")?;
    assert!(!driver.state().views.collapsed_sections.is_empty());
    driver.click_widget_id("result.section.0")?;
    assert!(driver.state().views.collapsed_sections.is_empty());

    driver.click_widget_id("result.tabs.next")?;
    assert_eq!(driver.state().app.canvas_dialogs.result_view.active_tab_index, 1);
    assert!(driver.state().has_ambiguity, "Fit Params tab with Open CI / ~ prefix must flag ambiguity callout");
    assert!(driver.frame_meta().find_by_widget_id("result.ambiguity_callout").is_some());

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
fn canvas_view_async_raster_hover_pairwise_cld_km_tex_layout_drilldown() -> Result<(), Box<dyn std::error::Error>> {
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

    let nl_spec = NonlinearFitSpec::new("log_inhibitor_vs_response_3pl");
    let mut nl_rs = ResultSheet::new(AnalysisSpec::new("res-nl", "col-1", AnalysisKind::NonlinearRegression(nl_spec))?)?;
    nl_rs.status = ResultStatus::UpToDate;
    nl_rs.scalars = vec![
        ResultScalarEntry { key: String::from("Bottom"), data_set_index: Some(0), value: 5.0, units: None },
        ResultScalarEntry { key: String::from("Top"), data_set_index: Some(0), value: 95.0, units: None },
        ResultScalarEntry { key: String::from("LogIC50"), data_set_index: Some(0), value: -6.2, units: None },
    ];
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
    let init = CanvasHarness {
        app,
        view: CanvasViewEguiState::default(),
        intents: PendingUiIntents::new(),
        paper_rect: None,
        km_rect: None,
        eq_count: 0,
        breadcrumb: false,
    };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        init,
        ViewportMetrics::new(1120.0, 740.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let tokens = Tokens::for_mode(st.app.theme.mode);
            let ctx = CanvasViewContext { tokens: &tokens, host_textures: None, enabled: true, is_secondary_split: false };
            let rep = render_canvas_view(ui, &st.app, &mut st.view, ctx, meta, &mut st.intents);
            st.paper_rect = rep.paper_rect_pt;
            st.km_rect = rep.km_track_rect_pt;
            st.eq_count = rep.tex_equation_rects_pt.len();
            st.breadcrumb = rep.layout_breadcrumb_shown;
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let res = st.intents.commit_if_settled(&mut st.app, false);
            assert!(res.is_ok(), "canvas intent commit failed: {res:?}");
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();

    for _ in 0..12 {
        if driver.state().view.slot.cached_texture.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        driver.run_settled();
    }
    assert!(driver.state().view.slot.cached_texture.is_some(), "async raster slot must cache GPU texture cleanly");
    assert!(driver.state().eq_count >= 1, "TeX equation overlay must render & record bounds");
    assert!(driver.frame_meta().find_by_widget_id("canvas.tex_equation.0").is_some());
    let tex_parity = driver.verify_pixel_to_hit_box_parity();
    assert!(tex_parity.is_clean(), "canvas.tex_equation parity violations: {:?}", tex_parity.violations);

    let paper = driver.state().paper_rect.expect("paper rect");
    let scene = driver.state().app.session().render_sheet_scene(&SheetId::new("g-1"))?;
    let first_pt = scene.plotted_points.first().expect("plotted point");
    let screen_pt = pos2(
        paper.left() + (first_pt.center_pt.x as f32 / scene.width_pt as f32) * paper.width(),
        paper.top() + (first_pt.center_pt.y as f32 / scene.height_pt as f32) * paper.height(),
    );
    driver.hover_at(screen_pt)?;
    let hover = driver.state().view.hover_readout.clone().expect("hover readout");
    assert_eq!(hover.point_index, Some(0));
    driver.click_at(screen_pt)?;
    assert_eq!(driver.state().app.brushed_cell.as_ref().map(|c| c.row), Some(first_pt.source_cell.row));

    assert!(driver.frame_meta().find_by_widget_id("canvas.pairwise_cld_recommend").is_some());
    driver.click_widget_id("canvas.pairwise_cld_recommend")?;
    assert!(driver.state().app.document().graph_sheet(&SheetId::new("g-1")).unwrap().spec.compact_letter_display.is_some());

    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "g-surv" }))?;
    driver.run_settled();
    assert!(driver.state().km_rect.is_some(), "KM Number at risk track must reserve & render");
    assert!(driver.frame_meta().find_by_widget_id("canvas.km_number_at_risk").is_some());
    assert!(driver.state().view.km_at_risk_rows >= 2);
    let km_parity = driver.verify_pixel_to_hit_box_parity();
    assert!(km_parity.is_clean(), "canvas.km_number_at_risk parity violations: {:?}", km_parity.violations);

    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" }))?;
    driver.run_settled();
    let p0_center = driver.frame_meta().find_by_widget_id("canvas.layout_panel.0").expect("panel 0").clipped_rect.center();
    driver.drag(p0_center, pos2(p0_center.x + 8.0, p0_center.y + 1.0))?;
    assert!(driver.state().view.last_snap_readout.as_deref().is_some_and(|s| s.contains("0.1 pt")));

    let p0_center2 = driver.frame_meta().find_by_widget_id("canvas.layout_panel.0").expect("panel 0").clipped_rect.center();
    driver.click_at(p0_center2)?;
    driver.click_at(p0_center2)?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("g-1"));
    assert!(driver.state().breadcrumb, "Return to Layout breadcrumb must show after double-click drill-down");

    driver.click_widget_id("canvas.return_to_layout")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("lay-1"));
    Ok(())
}

#[test]
fn canvas_view_heatmap_colorbar_hover_readout() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    let ds = app.document().data_sheet(&SheetId::new("col-1")).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-heat", ds)?;
    gs.title = String::from("Corr Heatmap");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Diagnostic(curvebook_doc::DiagnosticPlotKind::Heatmap);
    {
        let mut doc = app.document().clone();
        doc.graph_sheets.push(gs);
        doc.active_sheet_id = Some(SheetId::new("g-heat"));
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }

    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, CanvasViewEguiState::default(), PendingUiIntents::new(), None::<egui::Rect>, false),
        ViewportMetrics::new(980.0, 680.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (app, view, intents, paper_out, cb_hovered)| {
            let tokens = Tokens::for_mode(app.theme.mode);
            let ctx = CanvasViewContext { tokens: &tokens, host_textures: None, enabled: true, is_secondary_split: false };
            let rep = render_canvas_view(ui, app, view, ctx, meta, intents);
            *paper_out = rep.paper_rect_pt;
            *cb_hovered = rep.colorbar_hovered;
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();

    let paper = driver.state().3.expect("heatmap paper rect");
    let scene = driver.state().0.session().render_sheet_scene(&SheetId::new("g-heat"))?;
    let cb_rect = scene
        .primitives
        .iter()
        .filter_map(|p| {
            if let curvebook_chart::ScenePrimitive::Path { path, fill: Some(_), .. } = p {
                let xs: Vec<f64> = path
                    .commands
                    .iter()
                    .filter_map(|c| match c {
                        curvebook_chart::PathCmd::MoveTo(pt) | curvebook_chart::PathCmd::LineTo(pt) => Some(pt.x),
                        _ => None,
                    })
                    .collect();
                let ys: Vec<f64> = path
                    .commands
                    .iter()
                    .filter_map(|c| match c {
                        curvebook_chart::PathCmd::MoveTo(pt) | curvebook_chart::PathCmd::LineTo(pt) => Some(pt.y),
                        _ => None,
                    })
                    .collect();
                if xs.len() >= 4 && ys.len() >= 4 {
                    let x0 = xs.iter().copied().fold(f64::INFINITY, f64::min);
                    let x1 = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let y0 = ys.iter().copied().fold(f64::INFINITY, f64::min);
                    let y1 = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    if x1 - x0 <= 24.0 && y1 - y0 >= 40.0 && x0 >= scene.plot_rect_pt.x + scene.plot_rect_pt.width * 0.5 {
                        return Some((x0, x1, y0, y1));
                    }
                }
            }
            None
        })
        .next();

    if let Some((x0, x1, y0, y1)) = cb_rect {
        let cx = ((x0 + x1) * 0.5) as f32 / scene.width_pt as f32 * paper.width() + paper.left();
        let cy = ((y0 + y1) * 0.5) as f32 / scene.height_pt as f32 * paper.height() + paper.top();
        driver.hover_at(pos2(cx, cy))?;
        assert!(driver.state().4, "colorbar hover must set colorbar_hovered = true");
        let summary = driver.state().1.hover_readout.as_ref().map(|h| h.value_summary.as_str()).unwrap_or("");
        assert!(summary.contains('#'), "colorbar hover readout must report hex/value summary, got {summary}");
    }
    Ok(())
}

#[test]
fn grid_core_arrow_down_nav_zero_1m_clone_fast() -> Result<(), Box<dyn std::error::Error>> {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1_000, 1_000)?;
    for c in 0..4 {
        for r in 0..20 {
            table.data_sets[c].cells[r][0] = Cell::number((r + 1) as f64);
        }
    }
    let ds = DataSheet::new("col-1m", "One Million Cell Sheet", TableArchetype::Column(table))?;
    let doc = ProjectDocument { data_sheets: vec![ds], active_sheet_id: Some(SheetId::new("col-1m")), ..Default::default() };
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);
    app.sync_views();

    let start = Instant::now();
    let mut intents = PendingUiIntents::new();
    for _ in 0..10 {
        intents.push_custom_view_action("grid_core", "nav:down");
        let _ = intents.commit_if_settled(&mut app, false)?;
    }
    let elapsed = start.elapsed();

    assert_eq!(app.grid.tool.active.map(|c| c.row), Some(10));
    assert!(elapsed.as_secs_f64() < 2.0, "10 ArrowDown navigations over 1M-cell sheet took too long ({elapsed:?}), likely cloning DataSheet!");
    Ok(())
}

#[test]
fn all_14_beta_locales_and_viewports_pass_parity_and_scrollbar_non_overlap() -> Result<(), Box<dyn std::error::Error>> {
    let sheet_modes = ["info-1", "res-1", "g-1", "gallery"];
    let viewports = [ViewportMetrics::new(640.0, 420.0, 1.0), ViewportMetrics::new(1440.0, 600.0, 1.0)];

    for locale in BETA_LOCALE_CODES {
        for &target in &sheet_modes {
            for &vp in &viewports {
                let mut app = make_full_project_app()?;
                app.set_locale(locale);
                if target == "gallery" {
                    app.nav.gallery.open = true;
                    let doc_clone = app.document().clone();
                    app.nav.sync_gallery(&doc_clone);
                } else {
                    app.execute_command("sheet.select", json!({ "sheet_id": target }))?;
                }
                app.sync_views();

                let tokens = Tokens::for_mode(curvebook_ui_egui::UiThemeMode::Light);
                let driver = make_stage_driver(StageHarness::new(app), vp, tokens);
                let parity = driver.verify_pixel_to_hit_box_parity();
                assert!(parity.is_clean(), "parity violations for locale={locale}, mode={target}, vp={vp:?}: {:?}", parity.violations);
                let sb_violations = driver.check_scrollbar_non_overlap();
                assert!(sb_violations.is_empty(), "scrollbar overlap for locale={locale}, mode={target}, vp={vp:?}: {sb_violations:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn result_sheet_wide_tabs_many_sections_and_clipped_columns_pass_parity() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = make_full_project_app()?;
    let mut wide_tabs = Vec::new();
    let titles = [
        "ANOVA summary",
        "Multiple Comparisons — BH FDR-adjusted q values",
        "Comparison of survival curves & hazard ratios",
        "Homoscedasticity & Residual Diagnostics",
        "Bootstrap Parameter Confidence Intervals",
        "Covariance & Leverage Diagnostics",
    ];
    for (i, title) in titles.into_iter().enumerate() {
        let col_headers: Vec<String> = (0..14).map(|c| format!("Parameter Column {c}")).collect();
        let mut row_labels = Vec::new();
        let mut rows = Vec::new();
        for r in 0..28 {
            if r % 5 == 0 {
                row_labels.push(format!("Section {r} — Omnibus Diagnostic Metrics & Subgroup Totals"));
                rows.push(vec![ResultCell::Empty; 14]);
            } else {
                row_labels.push(format!("Long Diagnostic Metric Row Label Number {r}"));
                rows.push((0..14).map(|c| num_result_cell(100.1234 + (r * 10 + c) as f64)).collect());
            }
        }
        wide_tabs.push(ResultTab {
            id: format!("wide-tab-{i}"),
            title: String::from(title),
            kind: if i % 2 == 0 { ResultTabKind::MainSummary } else { ResultTabKind::MultipleComparisons },
            column_headers: col_headers,
            row_labels,
            rows,
            footnotes: vec![String::from(
                "Footnote: Extended multi-line explanatory note verifying horizontal wrapping inside ResultSheet body without horizontal clip bleed.",
            )],
        });
    }
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
        let tokens = Tokens::for_mode(curvebook_ui_egui::UiThemeMode::Light);
        let mut driver = make_stage_driver(StageHarness::new(stage_app), vp, tokens);
        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "result sheet parity violations at {vp:?}: {:?}", parity.violations);
        assert!(driver.check_scrollbar_non_overlap().is_empty(), "result sheet scrollbar overlap at {vp:?}");
        for entry in &driver.frame_meta().entries {
            if entry.widget_id.starts_with("result.tab.") {
                assert!(entry.clipped_rect.width() <= 140.0, "tab `{}` too wide: {:?}", entry.widget_id, entry.clipped_rect);
            }
        }
        if driver.frame_meta().find_by_widget_id("result.tabs.next").is_some_and(|e| e.enabled) {
            driver.click_widget_id("result.tabs.next")?;
            let after_next = driver.verify_pixel_to_hit_box_parity();
            assert!(after_next.is_clean(), "post-next parity violations at {vp:?}: {:?}", after_next.violations);
        }
    }
    Ok(())
}
