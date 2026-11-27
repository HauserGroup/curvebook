#![forbid(unsafe_code)]

//! Wave 4 (`R15.4` item 2) integration tests for `graph_modals` & `app_modals`.

use curvebook_doc::{
    CategoricalTickSource, Cell, CellCoord, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet, DiagnosticPlotKind, GraphAxisScale,
    GraphKind, GraphSheet, GroupedArrangement, LayoutSheet, LayoutSpec, MultipleVariablesTable, ProjectDocument, SheetId, TableArchetype,
    TickLabelFormat, VariableRole,
};
use curvebook_engine::Session;
use curvebook_palette::{ColorRamp, ColormapId};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::app_modals::{PreferencesCategory, UnsavedCloseAction};
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver};
use curvebook_ui_egui::egui_shell::inspect_bridge::{check_accesskit_scrollbar_non_overlap, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents};
use curvebook_ui_egui::i18n::{BETA_ACTIVE_LOCALES, is_rtl_locale};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{UiApp, UiThemeMode, dispatch_gui_action};
use serde_json::json;

use crate::common::{ShellTestState, assert_clean_parity, assert_modal_parity, fill_col_values, make_shell_driver};

type BoxRes = Result<(), Box<dyn std::error::Error>>;

fn make_driver(app: UiApp, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    make_shell_driver(app, ViewportMetrics::new(w, h, 1.0))
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
    let col_id = SheetId::try_new("col-paired")?;
    let mut table = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 4)?;
    fill_col_values(&mut table.data_sets[0], "Baseline", &[10.0, 11.2, 9.6, 10.5]);
    fill_col_values(&mut table.data_sets[1], "Treated", &[14.0, 15.8, 13.5, 16.1]);
    let col_ds = DataSheet::new(col_id.clone(), "Paired Assay", TableArchetype::Column(table))?;
    let col_gid = SheetId::try_new("graph-col")?;
    let mut col_graph = GraphSheet::new_uninitialized_for_data(col_gid.clone(), &col_ds)?;
    col_graph.title = String::from("Assay Plot");
    col_graph.initialized = true;
    col_graph.spec.x_axis.tick_format = TickLabelFormat::Categorical(CategoricalTickSource::ColumnTitles);

    let mut mv = MultipleVariablesTable::new(4, 6)?;
    for (idx, role) in
        [VariableRole::Continuous, VariableRole::Continuous, VariableRole::Categorical, VariableRole::Continuous].into_iter().enumerate()
    {
        mv.variables[idx].role = role;
    }
    let mut mv_ds = DataSheet::new(SheetId::try_new("mv-data")?, "Multivariate Cohort", TableArchetype::MultipleVariables(mv))?;
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

    let layout_id = SheetId::try_new("layout-1")?;
    let layout_sheet = LayoutSheet::new(layout_id.clone(), "Figure Layout", LayoutSpec::default())?;

    let mut doc = ProjectDocument::new("Wave4 Modal Integration")?;
    doc.data_sheets = vec![col_ds, mv_ds];
    doc.graph_sheets = vec![col_graph, mv_graph, hm_graph];
    doc.layout_sheets = vec![layout_sheet];
    doc.active_sheet_id = Some(col_gid.clone());
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();
    Ok(SeededFixture { app, col_id, col_gid, mv_gid, hm_gid, layout_id })
}

#[test]
fn change_graph_type_live_preview_and_paired_recommendation_commit() -> BoxRes {
    let SeededFixture { mut app, col_gid, .. } = seed_paired_column_and_graphs()?;
    let _ = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.changeGraphType.open", "params": { "sheet_id": col_gid.as_str() } }),
    )?;

    let mut driver = make_driver(app, 1440.0, 600.0);
    assert!(driver.frame_meta().find_by_widget_id("graph_type.preview_pane").is_some());
    assert!(driver.frame_meta().find_by_widget_id("graph_type.compatibility_hint").ok_or("missing hint")?.label.contains("Paired"));

    for wid in ["graph_type.card.col.before_after", "graph_type.sum.median", "modal.ok"] {
        driver.click_widget_id(wid)?;
    }
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(driver.state().app.document().graph_sheet(&col_gid).ok_or("missing graph")?.spec.kind, GraphKind::Column);
    Ok(())
}

#[test]
fn format_graph_all_three_schemas_and_unit_spinner_single_commit() -> BoxRes {
    let SeededFixture { app: base_app, col_gid, mv_gid, hm_gid, .. } = seed_paired_column_and_graphs()?;

    // Schema 1: Series-Based + Unit Spinner drag single-commit
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_format_graph(&doc, &col_gid));
        app.sync_views();
        let mut driver = make_driver(app, 1440.0, 600.0);
        assert!(driver.frame_meta().find_by_widget_id("edit.tab.format_graph.appearance").is_some());
        assert!(driver.frame_meta().find_by_widget_id("edit.tab.format_graph.data_objects").is_none());

        let before_undo = driver.state().app.session().undo_depth();
        let spin_rect = driver.frame_meta().find_by_widget_id("format_graph.marker_size_pt").ok_or("missing spinner")?.clipped_rect;
        driver.drag(egui::pos2(spin_rect.left() + 14.0, spin_rect.center().y), egui::pos2(spin_rect.left() + 34.0, spin_rect.center().y))?;
        assert_eq!(driver.state().app.session().undo_depth(), before_undo);
        assert!(driver.state().views.graph_modals.symbol_size_spinner.last_committed_value.is_some());

        for wid in ["format_graph.apply_globally", "edit.tab.format_graph.series", "format_graph.arrangement.stacked", "modal.ok"] {
            driver.click_widget_id(wid)?;
        }
        assert_eq!(
            driver.state().app.document().graph_sheet(&col_gid).ok_or("missing col graph")?.spec.grouped_arrangement,
            GroupedArrangement::Stacked
        );
    }

    // Schema 2: MV Channel & Layer
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_format_graph(&doc, &mv_gid));
        app.sync_views();
        let mut driver = make_driver(app, 1440.0, 600.0);
        assert!(driver.frame_meta().find_by_widget_id("edit.tab.format_graph.axis_variables").is_some());
        for wid in ["edit.tab.format_graph.data_objects", "format_graph.mv.confidence_ellipses", "format_graph.mv.convex_hulls", "modal.ok"] {
            driver.click_widget_id(wid)?;
        }
        let mv_enc = driver.state().app.document().graph_sheet(&mv_gid).ok_or("mv graph")?.spec.multivariate.as_ref().ok_or("mv enc")?;
        assert!(mv_enc.show_confidence_ellipses && mv_enc.show_convex_hulls);
    }

    // Schema 3: Matrix / Aggregated Heatmap
    {
        let mut app = base_app;
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_format_graph(&doc, &hm_gid));
        app.sync_views();
        let mut driver = make_driver(app, 1440.0, 600.0);
        assert!(driver.frame_meta().find_by_widget_id("edit.tab.format_graph.color_mapping").is_some());
        for wid in ["format_graph.heatmap.cmap.Cividis", "edit.tab.format_graph.heatmap_labels", "format_graph.heatmap.angle.45", "modal.ok"] {
            driver.click_widget_id(wid)?;
        }
        let gs = driver.state().app.document().graph_sheet(&hm_gid).ok_or("heatmap")?;
        assert!((gs.spec.x_axis.tick_angle_deg - 45.0).abs() < 1e-6);
        let mv_enc = gs.spec.multivariate.as_ref().ok_or("heatmap color channel")?;
        assert!(matches!(mv_enc.color, ColorChannelMapping::Continuous { ramp: ColorRamp::Named { id: ColormapId::Cividis, .. }, .. }));
    }
    Ok(())
}

#[test]
fn format_axes_categorical_guard_and_additional_ticks_submodal() -> BoxRes {
    let SeededFixture { mut app, col_gid, .. } = seed_paired_column_and_graphs()?;
    let _ =
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": col_gid.as_str() } }))?;

    let mut driver = make_driver(app, 1440.0, 600.0);
    driver.click_widget_id("edit.tab.axes.x")?;
    driver.click_widget_id("axes.scale.log10")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.categorical_guard.warning").is_some());
    let btn_blocked = driver.frame_meta().find_by_widget_id("axes.categorical_guard.confirm_btn").ok_or("missing confirm btn")?;
    assert!(!btn_blocked.enabled && btn_blocked.disabled_reason.is_some());
    assert!(matches!(driver.state().app.canvas_dialogs.format_axes.axis_draft.tick_format, TickLabelFormat::Categorical(_)));

    driver.click_widget_id("axes.categorical_guard.confirm_checkbox")?;
    driver.click_widget_id("axes.categorical_guard.confirm_btn")?;
    assert_eq!(driver.state().app.canvas_dialogs.format_axes.axis_draft.tick_format, TickLabelFormat::Decimal);
    assert_eq!(driver.state().app.canvas_dialogs.format_axes.axis_draft.segments[0].scale, GraphAxisScale::Log10);

    driver.click_widget_id("edit.tab.axes.y")?;
    driver.click_widget_id("axes.additional_ticks.open")?;
    assert!(driver.state().views.graph_modals.additional_ticks.open);

    for (wid, text) in [("axes.add_tick.value", "15.5"), ("axes.add_tick.label", "Cutoff")] {
        driver.click_widget_id(wid)?;
        driver.type_text(text)?;
    }
    driver.click_widget_id("axes.add_tick.save")?;

    driver.click_widget_id("axes.add_tick.shaded_zone")?;
    for (wid, text) in [("axes.add_tick.value", "12.0"), ("axes.add_tick.end_value", "14.0")] {
        driver.click_widget_id(wid)?;
        driver.type_text(text)?;
    }
    driver.click_widget_id("axes.add_tick.save")?;
    driver.click_widget_id("axes.add_tick.close")?;
    assert!(!driver.state().views.graph_modals.additional_ticks.open);
    driver.click_widget_id("modal.ok")?;

    let gs = driver.state().app.document().graph_sheet(&col_gid).ok_or("missing col graph")?;
    assert!(gs.spec.y_axis.additional_ticks.iter().any(|t| (t.value - 15.5).abs() < 1e-6 && t.label.as_deref() == Some("Cutoff")));
    assert!(gs.spec.y_axis.thresholds.iter().any(|th| (th.value - 12.0).abs() < 1e-6 && th.shade_to == Some(14.0)));
    Ok(())
}

#[test]
fn secondary_graph_and_layout_modals_render_with_legacy_ids() -> BoxRes {
    let SeededFixture { app: base_app, col_gid, mv_gid, layout_id, .. } = seed_paired_column_and_graphs()?;

    // Pairwise Comparisons, Bubble Plot Inspector, Apply Style From
    for (label, open_fn, expected_wids, check_parity) in [
        (
            "Pairwise",
            (|a: &mut UiApp, doc: &ProjectDocument, id: &SheetId| a.canvas_dialogs.open_pairwise_comparisons(doc, id))
                as fn(&mut UiApp, &ProjectDocument, &SheetId) -> bool,
            &["pairwise.hide_non_significant", "modal.ok"][..],
            true,
        ),
        (
            "Bubble",
            |a, doc, id| a.canvas_dialogs.open_bubble_plot(doc, id),
            &["bubble.show_convex_hulls", "bubble.show_confidence_ellipses"][..],
            false,
        ),
        ("ApplyStyle", |a, doc, id| a.canvas_dialogs.open_apply_style_from(doc, id), &["apply_style.clone_palette"][..], false),
    ] {
        let mut app = base_app.clone();
        let target = if label == "Bubble" { &mv_gid } else { &col_gid };
        let doc = app.document().clone();
        assert!(open_fn(&mut app, &doc, target));
        app.sync_views();
        let drv = make_driver(app, 1440.0, 600.0);
        for &wid in expected_wids {
            assert!(drv.frame_meta().find_by_widget_id(wid).is_some(), "{label}: missing `{wid}`");
        }
        if check_parity {
            assert_modal_parity(&drv, label)?;
        }
    }

    // Page Layout Composer
    let mut app = base_app;
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_page_layout_composer(&doc, &layout_id));
    app.sync_views();
    let mut drv = make_driver(app, 1440.0, 600.0);
    drv.click_widget_id("layout.arrange")?;
    assert!(!drv.state().app.canvas_dialogs.page_layout.items.is_empty());
    drv.click_widget_id("modal.ok")?;
    assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn export_conditional_formats_print_recovery_unsaved_and_find_replace() -> BoxRes {
    let SeededFixture { app: base_app, col_id, col_gid, layout_id, .. } = seed_paired_column_and_graphs()?;

    // 1. Export Figure (Graph): PDF vector gating vs PNG raster vs EMF
    {
        let mut app = base_app.clone();
        app.canvas_dialogs.open_export_figure(&col_gid);
        app.sync_views();
        let mut driver = make_driver(app, 1440.0, 600.0);
        for (fmt_wid, expect_enabled) in [("export.fmt.pdf", false), ("export.fmt.png", true)] {
            driver.click_widget_id(fmt_wid)?;
            assert_eq!(driver.frame_meta().find_by_widget_id("export.dpi").ok_or("export.dpi")?.enabled, expect_enabled, "{fmt_wid} dpi");
            assert_eq!(
                driver.frame_meta().find_by_widget_id("export.transparent_bg").ok_or("transparent_bg")?.enabled,
                expect_enabled,
                "{fmt_wid} bg"
            );
        }
        driver.click_widget_id("export.fmt.emf")?;
        driver.click_widget_id("modal.ok")?;
        assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    // 2. Export Layout EMF
    {
        let mut app = base_app.clone();
        app.canvas_dialogs.open_export_figure(&layout_id);
        app.sync_views();
        let mut driver = make_driver(app, 1440.0, 600.0);
        assert_eq!(driver.frame_meta().find_by_widget_id("export.fmt.emf").ok_or("layout EMF")?.command_id.as_deref(), Some("layout.exportEmf"));
        driver.click_widget_id("export.fmt.emf")?;
        driver.click_widget_id("modal.ok")?;
        assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    // 3. Print dialog
    {
        let mut app = base_app.clone();
        let doc = app.document().clone();
        app.canvas_dialogs.open_print_dialog(&doc, Some(&col_gid), vec![String::from("Lab-LaserJet")]);
        app.sync_views();
        let mut driver = make_driver(app, 1440.0, 600.0);
        assert!(driver.frame_meta().find_by_widget_id("print.submit_to_os").ok_or("print.submit_to_os")?.enabled);
        assert!(!driver.frame_meta().find_by_widget_id("print.bleed_pt").ok_or("print.bleed_pt")?.enabled);
        driver.click_widget_id("print.crop_marks")?;
        assert!(driver.frame_meta().find_by_widget_id("print.bleed_pt").ok_or("print.bleed_pt")?.enabled);
        driver.click_widget_id("modal.ok")?;
        assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    // 4. Unsaved Changes + Find & Replace
    let mut app = base_app;
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": col_id.as_str() }));
    app.egui_views.app_modals.unsaved_changes_open = true;
    let mut driver = make_driver(app, 1440.0, 600.0);
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
    Ok(())
}

#[test]
fn preferences_5_categories_14_locales_and_shortcut_conflict_guard() -> BoxRes {
    let SeededFixture { mut app, .. } = seed_paired_column_and_graphs()?;
    app.egui_views.app_modals.preferences_open = true;
    app.egui_views.app_modals.preferences.active_category = PreferencesCategory::Language;
    let mut driver = make_driver(app, 1440.0, 600.0);

    assert_eq!(BETA_ACTIVE_LOCALES.len(), 14);
    for &loc in BETA_ACTIVE_LOCALES {
        assert!(!is_rtl_locale(loc) && driver.frame_meta().find_by_widget_id(&format!("prefs.locale.{loc}")).is_some(), "missing {loc}");
    }
    for rtl in ["ar", "he", "fa", "ur"] {
        assert!(driver.frame_meta().find_by_widget_id(&format!("prefs.locale.{rtl}")).is_none());
    }
    driver.click_widget_id("prefs.locale.ja")?;
    assert_eq!(driver.state().app.locale, "ja");

    driver.click_widget_id("edit.tab.prefs.appearance")?;
    driver.click_widget_id("prefs.appearance.theme.dark")?;
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Dark);
    for (wid, expected) in [
        ("prefs.appearance.cvd.deuteranopia", curvebook_palette::CanvasPreviewFilter::Deuteranopia),
        ("prefs.appearance.cvd.grayscale", curvebook_palette::CanvasPreviewFilter::GrayscalePrint),
        ("prefs.appearance.cvd.normal", curvebook_palette::CanvasPreviewFilter::Normal),
    ] {
        driver.click_widget_id(wid)?;
        assert_eq!(driver.state().app.preview_filter, expected);
    }

    driver.click_widget_id("edit.tab.prefs.keyboard_shortcuts")?;
    driver.click_widget_id("prefs.shortcuts.search")?;
    driver.type_text("Save")?;
    driver.click_widget_id("prefs.shortcuts.row.project.save")?;

    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("CmdOrCtrl+Z");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;
    assert!(driver.state().views.app_modals.preferences.shortcut_conflict_error.is_some());
    assert!(driver.frame_meta().find_by_widget_id("prefs.shortcuts.conflict").is_some());

    let original_save_combo =
        driver.state().app.shortcuts.bindings().find(|b| b.command_id == "project.save").ok_or("missing project.save")?.active_combo.clone();
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("Ctrl+Alt+Y");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;
    assert!(driver.state().views.app_modals.preferences.shortcut_conflict_error.is_none());
    assert_eq!(
        driver
            .state()
            .views
            .app_modals
            .preferences
            .staged_shortcuts
            .as_ref()
            .and_then(|s| s.get("project.save"))
            .ok_or("missing staged")?
            .active_combo,
        "Ctrl+Alt+Y"
    );
    assert_eq!(driver.state().app.shortcuts.active_combo("project.save"), Some(original_save_combo.as_str()));

    driver.click_widget_id("modal.ok")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(driver.state().app.shortcuts.active_combo("project.save"), Some("Ctrl+Alt+Y"));
    Ok(())
}

#[test]
fn all_14_beta_locales_pass_640x420_and_1440x600_non_overlap_and_backdrop_capture() -> BoxRes {
    let SeededFixture { app: base_app, col_id, col_gid, .. } = seed_paired_column_and_graphs()?;
    assert_eq!(BETA_LOCALE_CODES.len(), 14);

    for locale in BETA_LOCALE_CODES {
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            let mut app_ax = base_app.clone();
            assert_eq!(app_ax.set_locale(locale), locale);
            let _ = dispatch_gui_action(
                &mut app_ax,
                "ui.invoke_menu",
                json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": col_gid.as_str() } }),
            )?;
            app_ax.sync_views();
            let mut drv_ax = make_driver(app_ax, w_pt, h_pt);
            assert_modal_parity(&drv_ax, &format!("FormatAxes `{locale}` @ {w_pt}x{h_pt}"))?;
            let outside = egui::pos2(4.0, (h_pt as f32) * 0.5);
            drv_ax.wheel_at(outside, egui::vec2(0.0, -80.0))?;
            drv_ax.click_at(outside)?;
            assert!(drv_ax.state().outside_click_captured && drv_ax.state().outside_wheel_captured);

            let mut app_pr = base_app.clone();
            let _ = app_pr.execute_command("sheet.select", json!({ "sheet_id": col_id.as_str() }));
            assert_eq!(app_pr.set_locale(locale), locale);
            app_pr.egui_views.app_modals.preferences_open = true;
            app_pr.sync_views();
            assert_clean_parity(&make_driver(app_pr, w_pt, h_pt), &format!("Preferences `{locale}` @ {w_pt}x{h_pt}"))?;
        }
    }

    let mut app_ak = base_app;
    let doc_ak = app_ak.document().clone();
    assert!(app_ak.canvas_dialogs.open_format_graph(&doc_ak, &col_gid));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut views, mut meta, mut intents) = (EguiShellViewState::default(), FrameInspectMeta::default(), PendingUiIntents::new());
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
        |ui| {
            let _ = render_egui_shell_frame(ui, &app_ak, &mut views, &mut meta, &mut intents);
        },
    );
    out.textures_delta.clear();
    check_accesskit_scrollbar_non_overlap(out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?)?;
    Ok(())
}

#[test]
fn aud_07_09_and_10_13_15_17_18_22_31_43_44_and_13_16_17_graph_and_app_modals() -> BoxRes {
    use crate::common::{set_textbox, sync_app_to_driver};
    use crate::helpers::{make_commit_driver, seed_column_app, seed_mv_app};
    use curvebook_doc::{AnalysisKind, OneWayMethod, SubcolumnMode, XyTable};
    use curvebook_tools::GridCellCoord;
    use curvebook_ui_egui::control_io::host_routing::commit_active_modal_dialog;
    use curvebook_ui_egui::egui_shell::UiIntent;
    use curvebook_ui_egui::{StubPlatformClipboardAdapter, handle_control_line, normalize_shortcut_combo};
    use std::sync::Arc;

    // 1. AUD-07-09: FormatSelectedPoints cross-sheet brush isolation & brushed commit fallback.
    let mut bdoc = ProjectDocument::new("Brush")?;
    let mut xy1 = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    let mut xy2 = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    for r in 0..3 {
        (xy1.x_cells[r][0], xy1.data_sets[0].cells[r][0], xy2.x_cells[r][0], xy2.data_sets[0].cells[r][0]) =
            (Cell::number(r as f64), Cell::number((r + 1) as f64), Cell::number(r as f64), Cell::number((r + 10) as f64));
    }
    let ds1 = DataSheet::new("ds-a", "A", TableArchetype::Xy(xy1))?;
    let mut g1 = GraphSheet::new_uninitialized_for_data("g-a", &ds1)?;
    g1.initialized = true;
    bdoc.data_sheets.extend([ds1, DataSheet::new("ds-b", "B", TableArchetype::Xy(xy2))?]);
    bdoc.graph_sheets.push(g1);
    bdoc.active_sheet_id = Some(SheetId::new("g-a"));
    let mut bapp = UiApp::from_session(Session::from_document(bdoc));
    bapp.brush_from_grid_cell(&SheetId::new("ds-b"), GridCellCoord::new(2, 0, 0));
    let r_open: serde_json::Value =
        serde_json::from_str(&handle_control_line(&mut bapp, r#"{"id":1,"method":"ui.formatPoints.open","params":{"sheet_id":"g-a"}}"#))?;
    assert_eq!(r_open["result"]["selected_count"], json!(0));
    bapp.brush_from_grid_cell(&SheetId::new("ds-a"), GridCellCoord::new(1, 0, 0));
    bapp.canvas_dialogs.open_format_selected_points_with_subcols(&bapp.document().clone(), &SheetId::new("g-a"), Vec::new());
    bapp.canvas_dialogs.format_points.hidden = Some(true);
    assert!(commit_active_modal_dialog(&mut bapp, |_, _| Ok(Default::default()))?.handled);
    assert!(!bapp.document().graph_sheet(&SheetId::new("g-a")).ok_or("g-a")?.spec.point_overrides.is_empty());

    // 2. AUD-10-15/44/17/18/22/43: Preferences shortcut capture, About/Feedback clipboard, REML compare_against_control, Manage Variables 64-undo + staging + frozen, ImportProjectMode & ExportFigure EMF failure/success.
    let mut ap = UiApp::new();
    ap.egui_views.app_modals.open_preferences(&ap.shortcuts, Some(PreferencesCategory::KeyboardShortcuts));
    ap.egui_views.app_modals.preferences.selected_shortcut_command = Some(ap.shortcuts.bindings().next().unwrap().command_id.clone());
    ap.egui_views.app_modals.preferences.shortcut_capture_armed = true;
    let mut dp = make_commit_driver(ap);
    dp.press_egui_key(egui::Key::K, egui::Modifiers::COMMAND);
    let cap = dp.state().views.app_modals.preferences.shortcut_combo_draft.clone();
    assert!(!cap.contains("CmdOrCtrl") && normalize_shortcut_combo(&cap).is_ok());

    let stub = Arc::new(StubPlatformClipboardAdapter::default());
    let url_stub = Arc::new(curvebook_ui_egui::os_integration::StubPlatformUrlOpener::new());
    let mut aclip = UiApp::new();
    aclip.human_path_only = true;
    aclip.set_clipboard_adapter(stub.clone());
    aclip.set_url_opener(url_stub.clone());
    aclip.canvas_dialogs.active_dialog = ActiveDialog::About;
    let mut dclip = make_commit_driver(aclip);
    dclip.click_widget_id("dialog.about.copy_version")?;
    assert!(stub.last_text().is_some_and(|t| t.contains("CurveBook")));
    stub.clear();
    dclip.click_widget_id("dialog.about.send_feedback")?;
    assert_eq!(dclip.state().app.canvas_dialogs.active_dialog, ActiveDialog::About);
    assert_eq!(url_stub.last_url().as_deref(), Some(curvebook_ui_egui::GITHUB_FEEDBACK_URL));
    assert!(stub.last_text().is_none(), "Send Feedback must not write unsolicited data to clipboard");

    let (mut aav, sav) = seed_column_app(&[("A", &[1.0, 2.0]), ("B", &[4.0, 5.0]), ("C", &[7.0, 8.0])], ColumnDesignHint::PairedRepeatedMeasures);
    let avid = SheetId::new(
        aav.execute_command("analysis.run", json!({ "source_sheet_id": sav.as_str(), "kind": "one_way_anova" }))?.payload["sheet_id"]
            .as_str()
            .ok_or("id")?,
    );
    assert!(aav.canvas_dialogs.open_edit_parameters(&aav.document().clone(), &avid));
    if let Some(d) = aav.canvas_dialogs.edit_params.draft.as_mut()
        && let AnalysisKind::OneWayAnova(a) = &mut d.kind
    {
        (a.method, a.mixed_effects_reml) = (OneWayMethod::RepeatedMeasures { geisser_greenhouse: true }, true);
    }
    aav.canvas_dialogs.edit_params.paired_or_matched = true;
    let mut dav = make_commit_driver(aav);
    dav.click_widget_id("edit.tab.multiple_comparisons")?;
    assert!(!dav.frame_meta().find_by_widget_id("edit.compare_against_control").unwrap().enabled);

    let (mut amv, smv) = seed_mv_app(3, 4);
    for i in 0..70usize {
        amv.execute_command("table.setTitles", json!({ "sheet_id": smv.as_str(), "col": 2, "title": format!("S{i}") }))?;
    }
    assert_eq!(amv.session().undo_depth(), 64);
    (amv.grid.variables_dialog.open, amv.grid.variables_dialog.selected_column) = (true, Some(1));
    let mut dmv = make_commit_driver(amv);
    for wid in ["mv_formula.vars.role.categorical", "modal.cancel"] {
        dmv.click_widget_id(wid)?;
    }
    assert!(
        matches!(&dmv.state().app.document().data_sheet(&smv).unwrap().archetype, TableArchetype::MultipleVariables(m) if m.variables[1].role == VariableRole::Continuous)
    );
    (dmv.state_mut().app.grid.variables_dialog.open, dmv.state_mut().app.grid.variables_dialog.selected_column) = (true, Some(0));
    sync_app_to_driver(&mut dmv);
    set_textbox(&mut dmv, "mv_formula.vars.title_input", "ArmR")?;
    dmv.click_widget_id("mv_formula.vars.col.1")?;
    set_textbox(&mut dmv, "mv_formula.vars.title_input", "DoseR")?;
    dmv.click_widget_id("modal.ok")?;
    assert!(
        matches!(&dmv.state().app.document().data_sheet(&smv).unwrap().archetype, TableArchetype::MultipleVariables(m) if m.variables[0].title == "ArmR" && m.variables[1].title == "DoseR")
    );
    dmv.state_mut().app.execute_command("sheet.freeze", json!({ "sheet_id": smv.as_str() }))?;
    dmv.state_mut().app.grid.variables_dialog.open = true;
    sync_app_to_driver(&mut dmv);
    assert!(
        !dmv.frame_meta().find_by_widget_id("mv_formula.vars.title_input").unwrap().enabled
            && !dmv.frame_meta().find_by_widget_id("mv_formula.vars.order.reversed").unwrap().enabled
    );

    let mut aipm = UiApp::new();
    aipm.canvas_dialogs.active_dialog = ActiveDialog::ImportProjectMode {
        command_id: "table.importPrism".into(),
        path: "/nonexistent/rem_missing.prism".into(),
        has_unsaved_changes: false,
    };
    let mut dipm = make_commit_driver(aipm);
    dipm.click_widget_id("import_project_mode.merge")?;
    assert!(matches!(dipm.state().app.canvas_dialogs.active_dialog, ActiveDialog::ImportProjectMode { .. }));

    let mut aexp = UiApp::new();
    let ls_id =
        SheetId::new(aexp.execute_command("layout.create", json!({ "sheet_id": "ls-rem", "title": "L" }))?.payload["sheet_id"].as_str().ok_or("id")?);
    aexp.canvas_dialogs.open_export_figure(&SheetId::new("gs-missing"));
    aexp.egui_views.app_modals.export_emf_selected = true;
    let mut fi = PendingUiIntents::new();
    fi.push(UiIntent::DialogButtonAction { action: "ok".into() });
    assert!(
        fi.commit_if_settled(&mut aexp, false).is_err()
            && matches!(aexp.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. })
            && aexp.egui_views.app_modals.export_emf_selected
    );
    let mut fi2 = PendingUiIntents::new();
    fi2.push_engine("graph.exportEmf", json!({ "sheet_id": "gs-missing" }));
    fi2.push(UiIntent::CloseAllModals);
    assert!(
        fi2.commit_all_if_settled(&mut aexp, false).first_error().is_some()
            && matches!(aexp.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. })
            && aexp.egui_views.app_modals.export_emf_selected
    );
    aexp.canvas_dialogs.open_export_figure(&ls_id);
    let mut oki = PendingUiIntents::new();
    oki.push(UiIntent::DialogButtonAction { action: "ok".into() });
    assert!(
        oki.commit_if_settled(&mut aexp, false).is_ok()
            && aexp.canvas_dialogs.active_dialog == ActiveDialog::None
            && !aexp.egui_views.app_modals.export_emf_selected
    );
    Ok(())
}

#[test]
fn aud_10_13_31_and_13_16_17_format_axes_overlays_and_heatmap_graph_modals() -> BoxRes {
    use crate::common::sync_app_to_driver;
    use crate::helpers::{make_commit_driver, seed_mv_app, seed_xy_app};
    use curvebook_doc::{AnalysisKind, DemingErrorSpec, FramePreset, LinearRegressionSpec, SimpleLogisticSpec};
    use curvebook_ui_egui::dialogs_canvas::AxisTarget;

    // 3. AUD-10-13/31 & AUD-13-16/17: FormatAxes Y2+PlainBoxed atomicity, FormatGraph SimpleLogistic+Deming overlays, Heatmap OK.
    let (mut axy, sxy) = seed_xy_app(&[1.0, 2.0, 3.0, 4.0, 5.0], &[0.0, 0.0, 1.0, 1.0, 1.0]);
    let gid = SheetId::new(
        axy.execute_command("graph.create", json!({ "data_sheet_id": sxy.as_str(), "kind": "xy" }))?.payload["sheet_id"].as_str().ok_or("id")?,
    );
    axy.execute_command("analysis.run", json!({ "sheet_id": "r-sl", "title": "Simple Logistic Fit", "source_sheet_id": sxy.as_str(), "kind": serde_json::to_value(AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default()))? }))?;
    axy.execute_command("analysis.run", json!({ "sheet_id": "r-dem", "title": "Deming Fit", "source_sheet_id": sxy.as_str(), "kind": serde_json::to_value(AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, deming_error: DemingErrorSpec::EqualError, ..Default::default() }))? }))?;
    let gs_init = axy.document().graph_sheet(&gid).unwrap().clone();
    axy.canvas_dialogs.format_axes.sync_from_graph(&gs_init, AxisTarget::Y2);
    axy.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gid.clone() };
    axy.canvas_dialogs.format_axes.enable_y2 = true;
    let mut dxy = make_commit_driver(axy);
    for wid in ["edit.tab.axes.frame_origin", "axes.frame.plain_boxed"] {
        dxy.click_widget_id(wid)?;
    }
    let u0 = dxy.state().app.session().undo_depth();
    dxy.click_widget_id("modal.ok")?;
    assert_eq!(
        (dxy.state().app.session().undo_depth(), dxy.state().app.document().graph_sheet(&gid).unwrap().spec.frame_preset),
        (u0 + 1, FramePreset::PlainBoxed)
    );

    let doc2 = dxy.state().app.document().clone();
    assert!(dxy.state_mut().app.canvas_dialogs.open_format_graph(&doc2, &gid));
    sync_app_to_driver(&mut dxy);
    let lbls: Vec<String> =
        dxy.frame_meta().entries.iter().filter(|n| n.widget_id.starts_with("format_graph.overlay.attach.")).map(|n| n.label.clone()).collect();
    assert!(lbls.iter().any(|l| l.contains("Simple Logistic")) && lbls.iter().any(|l| l.contains("Deming")));

    let (mut ahm, shm) = seed_mv_app(4, 6);
    let cid = SheetId::new(
        ahm.execute_command("analysis.run", json!({ "source_sheet_id": shm.as_str(), "kind": "correlation_matrix" }))?.payload["sheet_id"]
            .as_str()
            .ok_or("id")?,
    );
    let hgid = SheetId::new(
        ahm.execute_command(
            "graph.create",
            json!({ "data_sheet_id": cid.as_str(), "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap), "initialized": true }),
        )?
        .payload["sheet_id"]
            .as_str()
            .ok_or("id")?,
    );
    assert!(ahm.canvas_dialogs.open_format_graph(&ahm.document().clone(), &hgid));
    let mut dhm = make_commit_driver(ahm);
    dhm.click_widget_id("modal.ok")?;
    assert!(dhm.state().last_intent_error.is_none() && dhm.state().app.canvas_dialogs.active_dialog == ActiveDialog::None);
    Ok(())
}
