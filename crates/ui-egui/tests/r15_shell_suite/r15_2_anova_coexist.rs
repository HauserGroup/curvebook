#![forbid(unsafe_code)]

//! Wave 2 (`R15.2`) integration tests for `HybridCoexistenceArbiter`, `Parameters: One-Way ANOVA`
//! (`Cmd/Ctrl+T`), `R1`–`R4`/`R9` visible-disabled predicates + unlock tooltips, dynamic banner,
//! `ZLayer::ModalBackdrop` capture, and hybrid screenshots.

use curvebook_doc::{ColumnDesignHint, SheetId};
use curvebook_engine::Session;
use curvebook_ui_egui::RenderTargetSpec;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::BETA_LOCALE_CODES;
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    HybridCoexistenceArbiter, HybridModalSurface, HybridSheetSurface, check_accesskit_scrollbar_non_overlap, render_egui_shell_frame,
};
use curvebook_ui_egui::egui_shell::stat_anova_modal::{AnovaModalEguiState, AnovaModalTab, render_one_way_anova_modal, resolve_one_way_anova_banner};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::{UiApp, UiThemeMode};
use serde_json::json;

use crate::common::{assert_clean_parity, make_shell_driver, png_dimensions, seed_column_doc};

fn seed_column_anova_app(equal_replicates: bool) -> Result<(UiApp, SheetId), Box<dyn std::error::Error>> {
    let low: &[f64] = if equal_replicates { &[12.8, 13.1, 12.6, 13.0] } else { &[12.8, 13.1, 12.6] };
    let (doc, sid) = seed_column_doc(
        "ANOVA Coexist",
        "col-anova-1",
        "Dose Response Column",
        ColumnDesignHint::UnpairedReplicates,
        &[("Control", &[10.1, 10.4, 9.9, 10.2]), ("Low Dose", low), ("High Dose", &[16.4, 16.9, 16.2, 16.7])],
    )?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    Ok((app, sid))
}

#[test]
fn egui_analyze_modal_over_column_grid_and_focus_handoff_to_resultsheet() -> Result<(), Box<dyn std::error::Error>> {
    let (app, col_id) = seed_column_anova_app(true)?;
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let mut driver = make_shell_driver(app, metrics);

    let d0 = HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false);
    assert_eq!((d0.sheet_surface, d0.modal_surface, d0.blocks_egui_grid), (HybridSheetSurface::EguiColumnGrid, HybridModalSurface::None, false));

    driver.click_widget_id("toolbar.analyze")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    let da = HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false);
    assert_eq!(
        (da.sheet_surface, da.modal_surface, da.blocks_egui_grid),
        (HybridSheetSurface::EguiColumnGrid, HybridModalSurface::EguiAnalyzeSheet, true)
    );

    let outside = egui::pos2(8.0, 120.0);
    assert!(!driver.state().dialog_rect_pt.ok_or("missing dialog_rect")?.contains(outside));
    let (prev_scroll, prev_active) = (driver.state().views.column_grid.scroll_offset_pt, driver.state().app.grid.tool.active);
    driver.state_mut().outside_click_captured = false;
    driver.state_mut().outside_wheel_captured = false;
    driver.wheel_at(outside, egui::vec2(0.0, -180.0))?;
    driver.click_at(outside)?;
    assert!(driver.state().outside_click_captured && driver.state().outside_wheel_captured);
    assert_eq!((driver.state().views.column_grid.scroll_offset_pt, driver.state().app.grid.tool.active), (prev_scroll, prev_active));

    for _ in 0..8 {
        if driver.frame_meta().find_by_widget_id("analyze.entry.stats.one_way_anova").is_some_and(|e| e.clipped_rect.height() >= 8.0) {
            break;
        }
        let anchor = driver
            .frame_meta()
            .entries
            .iter()
            .find(|e| e.widget_id.starts_with("analyze.entry.") || e.widget_id == "analyze.search")
            .map(|e| e.clipped_rect.center())
            .ok_or("missing anchor")?;
        driver.wheel_at(anchor, egui::vec2(0.0, -140.0))?;
    }
    driver.run_settled();
    let dlg = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, Some("dialog"), true).dialog.ok_or("missing dialog")?;
    assert!(dlg.controls.iter().any(|c| c.widget_id == "analyze.entry.stats.one_way_anova"));
    assert!(dlg.buttons.iter().any(|b| b.widget_id == "modal.ok") && dlg.buttons.iter().any(|b| b.widget_id == "modal.cancel"));

    driver.click_widget_id("analyze.entry.stats.one_way_anova")?;
    driver.click_widget_id("modal.ok")?;
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let rs_id = driver.state().app.document().active_sheet_id.clone().ok_or("expected ResultSheet")?;
    assert!(rs_id != col_id && driver.state().app.document().result_sheet(&rs_id).is_some());
    let dr = HybridCoexistenceArbiter::evaluate(&driver.state().app, false);
    assert_eq!((dr.sheet_surface, dr.modal_surface), (HybridSheetSurface::EguiStage, HybridModalSurface::None));
    assert!(HybridCoexistenceArbiter::allows_cmd_t_edit_params(&driver.state().app, &dr, false));
    Ok(())
}

fn verify_anova_unequal_and_reml(metrics: ViewportMetrics) -> Result<(), Box<dyn std::error::Error>> {
    // 2. Create-mode label ("Run Analysis"), R1 unequal-N lock, method_ref/save_default, R3 control_column, R9 pvalue.digits.
    let (mut app_u, col_u) = seed_column_anova_app(false)?;
    let _ = app_u.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-unequal", "source_sheet_id": col_u.as_str(), "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let rs_u = SheetId::try_new("res-unequal")?;
    let doc_u = app_u.document().clone();
    assert!(app_u.canvas_dialogs.open_edit_parameters(&doc_u, &rs_u));
    app_u.sync_views();

    let mut create_app = app_u.clone();
    create_app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: SheetId::try_new("unsaved-draft")? };
    let (mut c_state, c_ctx, mut c_meta, mut c_intents, tokens) =
        (AnovaModalEguiState::default(), egui::Context::default(), FrameInspectMeta::default(), PendingUiIntents::new(), Tokens::default());
    let mut out = c_ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))), ..Default::default() },
        |ui| {
            c_meta.begin_pass(ui.ctx());
            let _ = render_one_way_anova_modal(ui.ctx(), &create_app, &mut c_state, &tokens, &mut c_meta, &mut c_intents);
        },
    );
    out.textures_delta.clear();
    assert_eq!(c_meta.find_by_widget_id("modal.ok").ok_or("missing create ok")?.label, "Run Analysis");

    let mut u_drv = make_shell_driver(app_u, metrics);
    assert!(!u_drv.frame_meta().find_by_widget_id("edit.paired_or_matched").ok_or("missing paired")?.enabled);
    assert!(!u_drv.frame_meta().find_by_widget_id("edit.tab.repeated_measures").ok_or("missing rm")?.enabled);
    u_drv.hover_widget_id("edit.tab.repeated_measures")?;
    assert!(u_drv.visible_tooltip_texts().iter().any(|t| t.contains("Paired") || t.contains("Repeated")));
    u_drv.click_widget_id("dialog.button.method_ref")?;
    assert!(u_drv.state().views.anova_modal.show_method_reference);
    u_drv.click_widget_id("dialog.button.method_ref")?;
    assert!(!u_drv.state().views.anova_modal.show_method_reference);
    u_drv.click_widget_id("dialog.button.save_default")?;
    assert!(u_drv.state().views.anova_modal.method_saved_as_default);

    u_drv.click_widget_id("edit.tab.multiple_comparisons")?;
    assert_eq!(u_drv.state().views.anova_modal.active_tab, AnovaModalTab::MultipleComparisons);
    assert!(!u_drv.frame_meta().find_by_widget_id("edit.control_column").ok_or("missing ctrl")?.enabled);
    u_drv.hover_widget_id("edit.control_column")?;
    assert!(u_drv.visible_tooltip_texts().iter().any(|t| t.contains("control") || t.contains("Dunnett")));
    u_drv.click_widget_id("edit.compare_against_control")?;
    assert!(u_drv.frame_meta().find_by_widget_id("edit.control_column").ok_or("missing ctrl")?.enabled);

    u_drv.click_widget_id("edit.tab.options")?;
    assert!(!u_drv.frame_meta().find_by_widget_id("edit.pvalue.digits.val").ok_or("missing digits")?.enabled);
    u_drv.hover_widget_id("edit.pvalue.digits.inc")?;
    assert!(u_drv.visible_tooltip_texts().iter().any(|t| t.contains("Custom")));
    u_drv.click_widget_id("edit.pval.custom")?;
    assert!(u_drv.frame_meta().find_by_widget_id("edit.pvalue.digits.val").ok_or("missing digits")?.enabled);

    // 3. Equal-N REML disables classical sphericity (R1b).
    let (mut app_eq, col_eq) = seed_column_anova_app(true)?;
    let _ = app_eq.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-eq", "source_sheet_id": col_eq.as_str(), "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let rs_eq = SheetId::try_new("res-eq")?;
    let doc_eq = app_eq.document().clone();
    assert!(app_eq.canvas_dialogs.open_edit_parameters(&doc_eq, &rs_eq));
    let mut eq_drv = make_shell_driver(app_eq, metrics);
    eq_drv.click_widget_id("edit.paired_or_matched")?;
    assert!(eq_drv.state().app.canvas_dialogs.edit_params.paired_or_matched);
    assert!(eq_drv.frame_meta().find_by_widget_id("edit.tab.repeated_measures").ok_or("missing unlocked RM tab")?.enabled);
    eq_drv.click_widget_id("edit.tab.repeated_measures")?;
    assert_eq!(eq_drv.state().views.anova_modal.active_tab, AnovaModalTab::AssumptionsAndTest);
    eq_drv.click_widget_id("edit.rm.mixed_effects_reml")?;
    assert!(resolve_one_way_anova_banner(&eq_drv.state().app).contains("Mixed-effects REML"));
    assert!(!eq_drv.frame_meta().find_by_widget_id("edit.sphericity.geisser_greenhouse").ok_or("missing sph")?.enabled);
    Ok(())
}

#[test]
fn egui_cmd_t_opens_anova_modal_and_predicates_r1_to_r9_behave_cleanly() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);

    // 1. Cmd+T over ResultSheet: "Update Analysis" label, backdrop capture, Kruskal-Wallis + R2 disabled Tukey tooltip + commit.
    let (mut app, col_id) = seed_column_anova_app(true)?;
    let _ = app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-anova-a", "source_sheet_id": col_id.as_str(), "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "res-anova-a" }));
    app.sync_views();
    let mut driver = make_shell_driver(app, metrics);

    driver.press_key("t", KeyModifiers { meta: true, ..Default::default() })?;
    let da = HybridCoexistenceArbiter::evaluate(&driver.state().app, false);
    assert_eq!(
        (da.sheet_surface, da.modal_surface, da.renders_egui_anova_modal),
        (HybridSheetSurface::EguiStage, HybridModalSurface::EguiOneWayAnova, true)
    );
    let dlg = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, Some("dialog"), true).dialog.ok_or("missing dialog")?;
    assert_eq!(dlg.buttons.iter().find(|b| b.widget_id == "modal.ok").ok_or("missing ok")?.label, "Update Analysis");

    let outside = egui::pos2(6.0, 120.0);
    driver.state_mut().outside_click_captured = false;
    driver.state_mut().outside_wheel_captured = false;
    driver.wheel_at(outside, egui::vec2(0.0, -180.0))?;
    driver.click_at(outside)?;
    assert!(driver.state().outside_click_captured && driver.state().outside_wheel_captured);

    assert!(resolve_one_way_anova_banner(&driver.state().app).contains("Ordinary one-way ANOVA"));
    driver.click_widget_id("edit.assume_gaussian")?;
    assert!(resolve_one_way_anova_banner(&driver.state().app).contains("Kruskal-Wallis"));
    assert!(!driver.frame_meta().find_by_widget_id("edit.post_hoc.tukey").ok_or("missing tukey")?.enabled);
    driver.hover_widget_id("edit.post_hoc.tukey")?;
    assert!(driver.visible_tooltip_texts().iter().any(|t| t.to_ascii_lowercase().contains("nonparametric") || t.contains("Dunn")));
    driver.click_widget_id("modal.ok")?;
    let rs = driver.state().app.document().result_sheet(&SheetId::try_new("res-anova-a")?).ok_or("missing rs")?;
    assert!(format!("{:?}", rs.analysis.kind).contains("KruskalWallis"));

    verify_anova_unequal_and_reml(metrics)
}

#[test]
fn all_14_beta_locales_and_hybrid_screenshots_pass_at_640x420_and_1440x600() -> Result<(), Box<dyn std::error::Error>> {
    let (mut base_app, col_id) = seed_column_anova_app(true)?;
    let _ = base_app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-locales", "source_sheet_id": col_id.as_str(), "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let rs_id = SheetId::try_new("res-locales")?;

    assert_eq!(BETA_LOCALE_CODES.len(), 14);
    for locale in BETA_LOCALE_CODES {
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            let mut app = base_app.clone();
            assert_eq!(app.set_locale(locale), locale);
            let doc_loc = app.document().clone();
            assert!(app.canvas_dialogs.open_edit_parameters(&doc_loc, &rs_id));
            app.sync_views();
            let mut driver = make_shell_driver(app, ViewportMetrics::new(w_pt, h_pt, 1.0));
            assert_clean_parity(&driver, &format!("OneWayANOVA `{locale}` @ {w_pt}x{h_pt}"))?;

            let outside = egui::pos2(4.0, (h_pt as f32) * 0.5);
            driver.wheel_at(outside, egui::vec2(0.0, -90.0))?;
            driver.click_at(outside)?;
            assert!(driver.state().outside_click_captured, "locale `{locale}` @ {w_pt}x{h_pt} failed outside click capture");
            assert!(driver.state().outside_wheel_captured, "locale `{locale}` @ {w_pt}x{h_pt} failed outside wheel capture");
        }
    }

    for (w_px, h_px) in [(640_u32, 420_u32), (1440_u32, 600_u32)] {
        let (mut app_b, _) = seed_column_anova_app(true)?;
        let doc_b = app_b.document().clone();
        assert!(app_b.canvas_dialogs.open_analyze_dialog(&doc_b, &col_id));
        app_b.set_theme_mode(UiThemeMode::Dark);
        let (png_b, bw, bh) = app_b
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: w_px, height_px: h_px, dpi: 96 }, ScreenshotRegionTarget::FullWindow)?;
        assert_eq!((bw, bh, png_dimensions(&png_b)), (w_px, h_px, Some((w_px, h_px))));

        let mut app_a = base_app.clone();
        let _ = app_a.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() }));
        let doc_a = app_a.document().clone();
        assert!(app_a.canvas_dialogs.open_edit_parameters(&doc_a, &rs_id));
        app_a.set_theme_mode(UiThemeMode::Light);
        let (png_a, aw, ah) = app_a
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: w_px, height_px: h_px, dpi: 96 }, ScreenshotRegionTarget::FullWindow)?;
        assert_eq!((aw, ah, png_dimensions(&png_a)), (w_px, h_px, Some((w_px, h_px))));
        assert_ne!(png_a, png_b);
    }

    let mut app_ak = base_app;
    let doc_ak = app_ak.document().clone();
    assert!(app_ak.canvas_dialogs.open_edit_parameters(&doc_ak, &rs_id));
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
