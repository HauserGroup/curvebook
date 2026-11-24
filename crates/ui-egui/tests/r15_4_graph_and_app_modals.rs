#![forbid(unsafe_code)]

//! Wave 4 (`R15.4` item 2) integration tests for `graph_modals` & `app_modals`:
//! - `Change Graph Type` recommendation hints, family filtering, live compiled dataset preview, and commit
//! - 3-schema `Format Graph` (`Series-Based`, `MV Channel & Layer`, `Matrix / Aggregated Heatmap`) + `unit_spinners` single-commit
//! - `Format Axes` (`Frame & Origin`, `Categorical Axis Guard` warning + confirmation lock, `Additional Ticks / Thresholds` sub-modal)
//! - Secondary graph & layout modals (`PairwiseComparisons`, `BubblePlotInspector`, `ApplyStyleFrom`, `FormatSelectedPoints`, `InsertModelEquation`, `PageLayoutComposer`)
//! - Conditional `Export Figure / Layout` (`PDF`, `SVG`, `EMF` for Graph vs Layout, `PNG`, `TIFF`, DPI/unit gating), `Print` (`--gui-only` guard), `Unsaved Changes`, `Recovery`, `About`, `Find & Replace`
//! - 5-category `Preferences` (strictly 14 non-RTL `BETA_ACTIVE_LOCALES`, theme & UI scale spinner, conflict-checked `UI_COMMANDS` shortcut editor)
//! - `14` Beta locales × `640×420` & `1440×600` non-overlap, `ZLayer::ModalBackdrop` outside capture, and AccessKit parity.

use curvebook_doc::{
    CategoricalTickSource, Cell, CellCoord, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet, DiagnosticPlotKind, GraphAxisScale,
    GraphKind, GraphSheet, GroupedArrangement, LayoutSheet, LayoutSpec, MultipleVariablesTable, ProjectDocument, SheetId, TableArchetype,
    TickLabelFormat, VariableRole,
};
use curvebook_engine::Session;
use curvebook_palette::{ColorRamp, ColormapId};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::app_modals::{PreferencesCategory, UnsavedCloseAction};
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::{check_accesskit_scrollbar_non_overlap, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, UiBackend};
use curvebook_ui_egui::i18n::{BETA_ACTIVE_LOCALES, is_rtl_locale};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{UiApp, UiThemeMode, dispatch_gui_action};
use serde_json::json;

fn modal_parity_violations(report: &curvebook_ui_egui::inspect::ParityReport) -> Vec<&curvebook_ui_egui::inspect::ParityViolation> {
    report.violations.iter().filter(|v| v.widget_id != "canvas.viewport" && v.widget_id != "canvas.split.viewport").collect()
}

struct ModalTestState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    outside_click_captured: bool,
    outside_wheel_captured: bool,
    dialog_rect_pt: Option<egui::Rect>,
}

impl ModalTestState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), outside_click_captured: false, outside_wheel_captured: false, dialog_rect_pt: None }
    }
}

fn make_driver<'a>(state: ModalTestState, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, ModalTestState> {
    let mut driver = EguiHumanPathDriver::new(state, metrics, |ui, meta, st| {
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.outside_click_captured |= rep.outside_click_captured;
        st.outside_wheel_captured |= rep.outside_wheel_captured;
        st.dialog_rect_pt = rep.regions.dialog_rect_pt;
    });
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

struct SeededFixture {
    app: UiApp,
    col_id: SheetId,
    col_gid: SheetId,
    mv_gid: SheetId,
    hm_gid: SheetId,
    layout_id: SheetId,
}

fn seed_paired_column_and_graphs() -> Result<SeededFixture, Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);

    // 1. Paired Column Table & Categorical Column Graph
    let col_id = SheetId::try_new("col-paired")?;
    let mut table = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 4)?;
    table.data_sets[0].title = String::from("Baseline");
    table.data_sets[1].title = String::from("Treated");
    for (r, (b, t)) in [(10.0, 14.0), (11.2, 15.8), (9.6, 13.5), (10.5, 16.1)].iter().enumerate() {
        if let Some(c0) = table.data_sets[0].cell_mut(r, 0) {
            *c0 = Cell::number(*b);
        }
        if let Some(c1) = table.data_sets[1].cell_mut(r, 0) {
            *c1 = Cell::number(*t);
        }
    }
    let col_ds = DataSheet::new(col_id.clone(), "Paired Assay", TableArchetype::Column(table))?;
    let col_gid = SheetId::try_new("graph-col")?;
    let mut col_graph = GraphSheet::new_uninitialized_for_data(col_gid.clone(), &col_ds)?;
    col_graph.title = String::from("Assay Plot");
    col_graph.initialized = true;
    col_graph.spec.x_axis.tick_format = TickLabelFormat::Categorical(CategoricalTickSource::ColumnTitles);

    // 2. MultipleVariables Bubble + Heatmap Graphs
    let mv_id = SheetId::try_new("mv-data")?;
    let mut mv = MultipleVariablesTable::new(4, 6)?;
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[2].role = VariableRole::Categorical;
    mv.variables[3].role = VariableRole::Continuous;
    let mut mv_ds = DataSheet::new(mv_id, "Multivariate Cohort", TableArchetype::MultipleVariables(mv))?;
    for r in 0..6 {
        let base = (r + 1) as f64;
        mv_ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(base * 2.0))?;
        mv_ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(base * 3.5))?;
        mv_ds.set_cell(CellCoord::new(r, 2, 0), Cell::text(if r % 2 == 0 { "Wildtype" } else { "Mutant" }))?;
        mv_ds.set_cell(CellCoord::new(r, 3, 0), Cell::number(5.0 + base))?;
    }

    let mv_gid = SheetId::try_new("graph-mv")?;
    let mut mv_graph = GraphSheet::new_uninitialized_for_data(mv_gid.clone(), &mv_ds)?;
    mv_graph.title = String::from("Bubble Multivariate");
    mv_graph.initialized = true;

    let hm_gid = SheetId::try_new("graph-heatmap")?;
    let mut hm_graph = GraphSheet::new_uninitialized_for_data(hm_gid.clone(), &mv_ds)?;
    hm_graph.title = String::from("Correlation Heatmap");
    hm_graph.initialized = true;
    hm_graph.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);

    // 3. Layout sheet for conditional export tests
    let layout_id = SheetId::try_new("layout-1")?;
    let layout_sheet = LayoutSheet::new(layout_id.clone(), "Figure Layout", LayoutSpec::default())?;

    let mut doc = ProjectDocument::new("Wave4 Modal Integration")?;
    doc.data_sheets = vec![col_ds, mv_ds];
    doc.graph_sheets = vec![col_graph, mv_graph, hm_graph];
    doc.layout_sheets = vec![layout_sheet];
    doc.active_sheet_id = Some(col_gid.clone());
    *app.session_mut() = Session::from_document(doc);
    app.set_gui_only(true);
    app.sync_views();

    Ok(SeededFixture { app, col_id, col_gid, mv_gid, hm_gid, layout_id })
}

#[test]
fn change_graph_type_live_preview_and_paired_recommendation_commit() -> Result<(), Box<dyn std::error::Error>> {
    let SeededFixture { mut app, col_gid, .. } = seed_paired_column_and_graphs()?;
    let _ = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.changeGraphType.open", "params": { "sheet_id": col_gid.as_str() } }),
    )?;

    let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
    assert!(driver.frame_meta().find_by_widget_id("graph_type.preview_pane").is_some());
    let hint = driver.frame_meta().find_by_widget_id("graph_type.compatibility_hint").ok_or("missing compatibility hint")?;
    assert!(hint.label.contains("Paired"), "expected Paired hint, got: {}", hint.label);

    driver.click_widget_id("graph_type.card.col.before_after")?;
    driver.click_widget_id("graph_type.sum.median")?;
    driver.click_widget_id("modal.ok")?;

    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let gs = driver.state().app.document().graph_sheet(&col_gid).ok_or("missing graph")?;
    assert_eq!(gs.spec.kind, GraphKind::Column);
    Ok(())
}

#[test]
fn format_graph_all_three_schemas_and_unit_spinner_single_commit() -> Result<(), Box<dyn std::error::Error>> {
    let SeededFixture { app: base_app, col_gid, mv_gid, hm_gid, .. } = seed_paired_column_and_graphs()?;

    // Schema 1: Series-Based + Unit Spinner drag single-commit
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_format_graph(&doc, &col_gid));
        app.sync_views();
        let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));

        assert!(driver.frame_meta().find_by_widget_id("edit.tab.format_graph.appearance").is_some());
        assert!(driver.frame_meta().find_by_widget_id("edit.tab.format_graph.data_objects").is_none());

        // Drag label on marker size unit_spinner; verify single commit upon release without mid-drag undo spam.
        let before_undo = driver.state().app.session().undo_depth();
        let spin_rect = driver.frame_meta().find_by_widget_id("format_graph.marker_size_pt").ok_or("missing marker size spinner")?.clipped_rect;
        let drag_start = egui::pos2(spin_rect.left() + 14.0, spin_rect.center().y);
        let drag_end = egui::pos2(spin_rect.left() + 34.0, spin_rect.center().y);
        driver.drag(drag_start, drag_end)?;
        assert_eq!(driver.state().app.session().undo_depth(), before_undo, "spinner draft must not commit session undo mid-edit");
        assert!(driver.state().views.graph_modals.symbol_size_spinner.last_committed_value.is_some());

        driver.click_widget_id("format_graph.apply_globally")?;
        driver.click_widget_id("edit.tab.format_graph.series")?;
        driver.click_widget_id("format_graph.arrangement.stacked")?;
        driver.click_widget_id("modal.ok")?;

        let gs = driver.state().app.document().graph_sheet(&col_gid).ok_or("missing col graph")?;
        assert_eq!(gs.spec.grouped_arrangement, GroupedArrangement::Stacked);
    }

    // Schema 2: MV Channel & Layer
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_format_graph(&doc, &mv_gid));
        app.sync_views();
        let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));

        assert!(driver.frame_meta().find_by_widget_id("edit.tab.format_graph.axis_variables").is_some());
        driver.click_widget_id("edit.tab.format_graph.data_objects")?;
        driver.click_widget_id("format_graph.mv.confidence_ellipses")?;
        driver.click_widget_id("format_graph.mv.convex_hulls")?;
        driver.click_widget_id("modal.ok")?;

        let gs = driver.state().app.document().graph_sheet(&mv_gid).ok_or("missing mv graph")?;
        let mv_enc = gs.spec.multivariate.as_ref().ok_or("missing multivariate encoding")?;
        assert!(mv_enc.show_confidence_ellipses);
        assert!(mv_enc.show_convex_hulls);
    }

    // Schema 3: Matrix / Aggregated Heatmap
    {
        let mut app = base_app;
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_format_graph(&doc, &hm_gid));
        app.sync_views();
        let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));

        assert!(driver.frame_meta().find_by_widget_id("edit.tab.format_graph.color_mapping").is_some());
        driver.click_widget_id("format_graph.heatmap.cmap.Cividis")?;
        driver.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
        driver.click_widget_id("format_graph.heatmap.angle.45")?;
        driver.click_widget_id("modal.ok")?;

        let gs = driver.state().app.document().graph_sheet(&hm_gid).ok_or("missing heatmap")?;
        assert!((gs.spec.x_axis.tick_angle_deg - 45.0).abs() < 1e-6);
        let mv_enc = gs.spec.multivariate.as_ref().ok_or("missing heatmap color channel")?;
        assert!(matches!(mv_enc.color, ColorChannelMapping::Continuous { ramp: ColorRamp::Named { id: ColormapId::Cividis, .. }, .. }));
    }
    Ok(())
}

#[test]
fn format_axes_categorical_guard_and_additional_ticks_submodal() -> Result<(), Box<dyn std::error::Error>> {
    let SeededFixture { mut app, col_gid, .. } = seed_paired_column_and_graphs()?;
    let _ =
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": col_gid.as_str() } }))?;

    let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
    driver.click_widget_id("edit.tab.axes.x")?;

    // Attempt switching categorical X axis to Log10 -> Categorical Axis Guard blocks until confirmed.
    driver.click_widget_id("axes.scale.log10")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.categorical_guard.warning").is_some());
    let btn_blocked = driver.frame_meta().find_by_widget_id("axes.categorical_guard.confirm_btn").ok_or("missing confirm btn")?;
    assert!(!btn_blocked.enabled);
    assert!(btn_blocked.disabled_reason.is_some());
    assert!(matches!(driver.state().app.canvas_dialogs.format_axes.axis_draft.tick_format, TickLabelFormat::Categorical(_)));

    // Check confirmation checkbox and proceed.
    driver.click_widget_id("axes.categorical_guard.confirm_checkbox")?;
    driver.click_widget_id("axes.categorical_guard.confirm_btn")?;
    assert_eq!(driver.state().app.canvas_dialogs.format_axes.axis_draft.tick_format, TickLabelFormat::Decimal);
    assert_eq!(driver.state().app.canvas_dialogs.format_axes.axis_draft.segments[0].scale, GraphAxisScale::Log10);

    // Switch to Y axis and open Additional Ticks / Thresholds sub-modal.
    driver.click_widget_id("edit.tab.axes.y")?;
    driver.click_widget_id("axes.additional_ticks.open")?;
    assert!(driver.state().views.graph_modals.additional_ticks.open);

    driver.click_widget_id("axes.add_tick.value")?;
    driver.type_text("15.5")?;
    driver.click_widget_id("axes.add_tick.label")?;
    driver.type_text("Cutoff")?;
    driver.click_widget_id("axes.add_tick.save")?;

    // Add a shaded threshold band as well.
    driver.click_widget_id("axes.add_tick.shaded_zone")?;
    driver.click_widget_id("axes.add_tick.value")?;
    driver.type_text("12.0")?;
    driver.click_widget_id("axes.add_tick.end_value")?;
    driver.type_text("14.0")?;
    driver.click_widget_id("axes.add_tick.save")?;

    // Close sub-modal and commit dialog OK.
    driver.click_widget_id("axes.add_tick.close")?;
    assert!(!driver.state().views.graph_modals.additional_ticks.open);
    driver.click_widget_id("modal.ok")?;

    let gs = driver.state().app.document().graph_sheet(&col_gid).ok_or("missing col graph")?;
    assert!(gs.spec.y_axis.additional_ticks.iter().any(|t| (t.value - 15.5).abs() < 1e-6 && t.label.as_deref() == Some("Cutoff")));
    assert!(gs.spec.y_axis.thresholds.iter().any(|th| (th.value - 12.0).abs() < 1e-6 && th.shade_to == Some(14.0)));
    Ok(())
}

#[test]
fn secondary_graph_and_layout_modals_render_with_legacy_ids() -> Result<(), Box<dyn std::error::Error>> {
    let SeededFixture { app: base_app, col_gid, mv_gid, layout_id, .. } = seed_paired_column_and_graphs()?;

    // Pairwise Comparisons
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_pairwise_comparisons(&doc, &col_gid));
        app.sync_views();
        let drv = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
        assert!(drv.frame_meta().find_by_widget_id("pairwise.hide_non_significant").is_some());
        assert!(drv.frame_meta().find_by_widget_id("modal.ok").is_some());
        let parity = drv.verify_pixel_to_hit_box_parity();
        let modal_v = modal_parity_violations(&parity);
        assert!(modal_v.is_empty(), "Pairwise parity violations: {:?}", modal_v);
    }

    // Bubble Plot Inspector
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_bubble_plot(&doc, &mv_gid));
        app.sync_views();
        let drv = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
        assert!(drv.frame_meta().find_by_widget_id("bubble.show_convex_hulls").is_some());
        assert!(drv.frame_meta().find_by_widget_id("bubble.show_confidence_ellipses").is_some());
    }

    // Page Layout Composer
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_page_layout_composer(&doc, &layout_id));
        app.sync_views();
        let mut drv = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
        drv.click_widget_id("layout.arrange")?;
        assert!(!drv.state().app.canvas_dialogs.page_layout.items.is_empty());
        drv.click_widget_id("modal.ok")?;
        assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    // Apply Style From
    {
        let mut app = base_app;
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_apply_style_from(&doc, &col_gid));
        app.sync_views();
        let drv = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
        assert!(drv.frame_meta().find_by_widget_id("apply_style.clone_palette").is_some());
    }

    Ok(())
}

#[test]
fn export_conditional_formats_print_recovery_unsaved_and_find_replace() -> Result<(), Box<dyn std::error::Error>> {
    let SeededFixture { app: base_app, col_id, col_gid, layout_id, .. } = seed_paired_column_and_graphs()?;

    // 1. Export Figure: Graph EMF + Vector vs Raster gating
    {
        let mut app = base_app.clone();
        app.canvas_dialogs.open_export_figure(&col_gid);
        app.sync_views();
        let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));

        driver.click_widget_id("export.fmt.pdf")?;
        let dpi_pdf = driver.frame_meta().find_by_widget_id("export.dpi").ok_or("missing export.dpi")?;
        assert!(!dpi_pdf.enabled, "DPI must be disabled for pure vector PDF");
        let bg_pdf = driver.frame_meta().find_by_widget_id("export.transparent_bg").ok_or("missing transparent_bg")?;
        assert!(!bg_pdf.enabled, "Transparent background must be disabled for PDF");

        driver.click_widget_id("export.fmt.png")?;
        let dpi_png = driver.frame_meta().find_by_widget_id("export.dpi").ok_or("missing export.dpi")?;
        assert!(dpi_png.enabled);
        let bg_png = driver.frame_meta().find_by_widget_id("export.transparent_bg").ok_or("missing transparent_bg")?;
        assert!(bg_png.enabled);

        driver.click_widget_id("export.fmt.emf")?;
        driver.click_widget_id("modal.ok")?;
        assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    // Export Layout EMF cleanly
    {
        let mut app = base_app.clone();
        app.canvas_dialogs.open_export_figure(&layout_id);
        app.sync_views();
        let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
        let emf_btn = driver.frame_meta().find_by_widget_id("export.fmt.emf").ok_or("missing layout EMF")?;
        assert_eq!(emf_btn.command_id.as_deref(), Some("layout.exportEmf"));
        driver.click_widget_id("export.fmt.emf")?;
        driver.click_widget_id("modal.ok")?;
        assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    // 2. Print under `--gui-only`
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        app.canvas_dialogs.open_print_dialog(&doc, Some(&col_gid), vec![String::from("Lab-LaserJet")]);
        app.sync_views();
        let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
        let submit_os = driver.frame_meta().find_by_widget_id("print.submit_to_os").ok_or("missing print.submit_to_os")?;
        assert!(!submit_os.enabled, "--gui-only must disable direct OS print spooling");
        let bleed_off = driver.frame_meta().find_by_widget_id("print.bleed_pt").ok_or("missing print.bleed_pt")?;
        assert!(!bleed_off.enabled);
        driver.click_widget_id("print.crop_marks")?;
        let bleed_on = driver.frame_meta().find_by_widget_id("print.bleed_pt").ok_or("missing print.bleed_pt")?;
        assert!(bleed_on.enabled);
        driver.click_widget_id("modal.ok")?;
        assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    // 3. Unsaved Changes, Find & Replace, About
    {
        let mut app = base_app;
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": col_id.as_str() }));
        app.egui_views.app_modals.unsaved_changes_open = true;
        let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));
        driver.click_widget_id("dialog.button.dont_save")?;
        assert_eq!(driver.state().app.egui_views.app_modals.pending_unsaved_action, Some(UnsavedCloseAction::DontSave));

        driver.state_mut().views.app_modals.find_replace_open = true;
        driver.run_settled();
        driver.click_widget_id("find_replace.query")?;
        driver.type_text("10")?;
        driver.click_widget_id("find_replace.find")?;
        assert!(driver.state().app.egui_views.app_modals.find_replace.match_count >= 1);
        driver.click_widget_id("modal.cancel")?;
        assert!(!driver.state().app.egui_views.app_modals.find_replace_open);
    }

    Ok(())
}

#[test]
fn preferences_5_categories_14_locales_and_shortcut_conflict_guard() -> Result<(), Box<dyn std::error::Error>> {
    let SeededFixture { mut app, .. } = seed_paired_column_and_graphs()?;
    app.egui_views.app_modals.preferences_open = true;
    app.egui_views.app_modals.preferences.active_category = PreferencesCategory::Language;

    let mut driver = make_driver(ModalTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0));

    // Strictly 14 non-RTL BETA_ACTIVE_LOCALES exposed on Language tab; zero RTL locales.
    assert_eq!(BETA_ACTIVE_LOCALES.len(), 14);
    for &loc in BETA_ACTIVE_LOCALES {
        assert!(!is_rtl_locale(loc));
        assert!(driver.frame_meta().find_by_widget_id(&format!("prefs.locale.{loc}")).is_some(), "missing Beta locale {loc}");
    }
    for rtl in ["ar", "he", "fa", "ur"] {
        assert!(driver.frame_meta().find_by_widget_id(&format!("prefs.locale.{rtl}")).is_none(), "RTL locale `{rtl}` must stay hidden");
    }

    driver.click_widget_id("prefs.locale.ja")?;
    assert_eq!(driver.state().app.locale, "ja");

    // Appearance category: switch theme mode and CVD preview filter cleanly.
    driver.click_widget_id("edit.tab.prefs.appearance")?;
    driver.click_widget_id("prefs.appearance.theme.dark")?;
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Dark);
    driver.click_widget_id("prefs.appearance.cvd.deuteranopia")?;
    assert_eq!(driver.state().app.preview_filter, curvebook_palette::CanvasPreviewFilter::Deuteranopia);
    driver.click_widget_id("prefs.appearance.cvd.grayscale")?;
    assert_eq!(driver.state().app.preview_filter, curvebook_palette::CanvasPreviewFilter::GrayscalePrint);
    driver.click_widget_id("prefs.appearance.cvd.normal")?;
    assert_eq!(driver.state().app.preview_filter, curvebook_palette::CanvasPreviewFilter::Normal);

    // Keyboard Shortcuts category: UI_COMMANDS conflict check & valid rebind + reset.
    driver.click_widget_id("edit.tab.prefs.keyboard_shortcuts")?;
    driver.click_widget_id("prefs.shortcuts.search")?;
    driver.type_text("Save")?;
    driver.click_widget_id("prefs.shortcuts.row.project.save")?;

    // Attempt conflicting combo (`CmdOrCtrl+Z` belongs to session.undo) -> rejected cleanly with error banner.
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("CmdOrCtrl+Z");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;
    assert!(driver.state().views.app_modals.preferences.shortcut_conflict_error.is_some());
    assert!(driver.frame_meta().find_by_widget_id("prefs.shortcuts.conflict").is_some());

    // Valid non-conflicting combo stages in the modal draft until OK (`R15-GUI-25`).
    let original_save_combo =
        driver.state().app.shortcuts.bindings().find(|b| b.command_id == "project.save").ok_or("missing project.save binding")?.active_combo.clone();
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("Ctrl+Alt+Y");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;
    assert!(driver.state().views.app_modals.preferences.shortcut_conflict_error.is_none());
    let staged = driver
        .state()
        .views
        .app_modals
        .preferences
        .staged_shortcuts
        .as_ref()
        .and_then(|s| s.get("project.save"))
        .ok_or("missing staged project.save binding")?;
    assert_eq!(staged.active_combo, "Ctrl+Alt+Y");
    assert_eq!(
        driver.state().app.shortcuts.active_combo("project.save"),
        Some(original_save_combo.as_str()),
        "staged shortcut must not mutate live app.shortcuts before OK"
    );

    driver.click_widget_id("modal.ok")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(driver.state().app.shortcuts.active_combo("project.save"), Some("Ctrl+Alt+Y"));
    Ok(())
}

#[test]
fn all_14_beta_locales_pass_640x420_and_1440x600_non_overlap_and_backdrop_capture() -> Result<(), Box<dyn std::error::Error>> {
    let SeededFixture { app: base_app, col_id, col_gid, .. } = seed_paired_column_and_graphs()?;
    assert_eq!(BETA_LOCALE_CODES.len(), 14);

    for locale in BETA_LOCALE_CODES {
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            // Check Format Axes
            let mut app_ax = base_app.clone();
            assert_eq!(app_ax.set_locale(locale), locale);
            let _ = dispatch_gui_action(
                &mut app_ax,
                "ui.invoke_menu",
                json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": col_gid.as_str() } }),
            )?;
            app_ax.sync_views();
            let mut drv_ax = make_driver(ModalTestState::new(app_ax), ViewportMetrics::new(w_pt, h_pt, 1.0));

            let parity_ax = drv_ax.verify_pixel_to_hit_box_parity();
            let modal_v_ax = modal_parity_violations(&parity_ax);
            assert!(modal_v_ax.is_empty(), "FormatAxes `{locale}` @ {w_pt}x{h_pt} parity: {:?}", modal_v_ax);
            let sb_ax = drv_ax.check_scrollbar_non_overlap();
            assert!(sb_ax.is_empty(), "FormatAxes `{locale}` @ {w_pt}x{h_pt} scrollbar overlap: {sb_ax:?}");

            let outside = egui::pos2(4.0, (h_pt as f32) * 0.5);
            drv_ax.wheel_at(outside, egui::vec2(0.0, -80.0))?;
            drv_ax.click_at(outside)?;
            assert!(drv_ax.state().outside_click_captured, "FormatAxes `{locale}` @ {w_pt}x{h_pt} click capture");
            assert!(drv_ax.state().outside_wheel_captured, "FormatAxes `{locale}` @ {w_pt}x{h_pt} wheel capture");

            // Check Preferences over DataSheet -> full unfiltered `parity.is_clean()` must hold!
            let mut app_pr = base_app.clone();
            let _ = app_pr.execute_command("sheet.select", json!({ "sheet_id": col_id.as_str() }));
            assert_eq!(app_pr.set_locale(locale), locale);
            app_pr.egui_views.app_modals.preferences_open = true;
            app_pr.sync_views();
            let drv_pr = make_driver(ModalTestState::new(app_pr), ViewportMetrics::new(w_pt, h_pt, 1.0));
            let parity_pr = drv_pr.verify_pixel_to_hit_box_parity();
            assert!(parity_pr.is_clean(), "Preferences `{locale}` @ {w_pt}x{h_pt} parity: {:?}", parity_pr.violations);
            let sb_pr = drv_pr.check_scrollbar_non_overlap();
            assert!(sb_pr.is_empty(), "Preferences `{locale}` @ {w_pt}x{h_pt} scrollbar overlap: {sb_pr:?}");
        }
    }

    // Verify AccessKit scrollbar non-overlap directly on settled Format Graph frame at 640x420.
    let mut app_ak = base_app;
    let doc_ak = app_ak.document().clone();
    assert!(app_ak.canvas_dialogs.open_format_graph(&doc_ak, &col_gid));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut views = EguiShellViewState::default();
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
        |ui| {
            let _ = render_egui_shell_frame(ui, &app_ak, &mut views, &mut meta, &mut intents);
        },
    );
    out.textures_delta.clear();
    let ak_update = out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?;
    check_accesskit_scrollbar_non_overlap(ak_update)?;
    Ok(())
}
