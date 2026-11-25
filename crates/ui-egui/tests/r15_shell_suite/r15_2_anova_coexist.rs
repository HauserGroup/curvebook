#![forbid(unsafe_code)]

//! Wave 2 (`R15.2` items 1 & 2) integration tests for Hybrid Coexistence Arbiter, standardized
//! `Parameters: One-Way ANOVA` (`Cmd/Ctrl+T` / `EditAnalysisParameters`) modal, `R1`–`R4`/`R9`
//! visible-disabled predicates + unlock tooltips, Dynamic Resolved-Test Banner, `ZLayer::ModalBackdrop`
//! outside click/wheel absorption, `14`-locale ellipsis/bleed parity, and hybrid screenshots at
//! `640×420` & `1440×600`.

use curvebook_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_ui_egui::RenderTargetSpec;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    HybridCoexistenceArbiter, HybridModalSurface, HybridSheetSurface, check_accesskit_scrollbar_non_overlap, render_egui_shell_frame,
};
use curvebook_ui_egui::egui_shell::stat_anova_modal::{AnovaModalEguiState, AnovaModalTab, render_one_way_anova_modal, resolve_one_way_anova_banner};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::{UiApp, UiThemeMode};
use serde_json::json;

fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || !png.starts_with(b"\x89PNG\r\n\x1a\n") || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    Some((w, h))
}

fn fill_col_values(col: &mut curvebook_doc::DataSetColumn, title: &str, values: &[f64]) {
    col.title = title.to_owned();
    for (r, &v) in values.iter().enumerate() {
        if let Some(cell) = col.cell_mut(r, 0) {
            *cell = Cell::number(v);
        }
    }
}

fn seed_column_anova_app(equal_replicates: bool) -> Result<(UiApp, SheetId), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let sheet_id = SheetId::try_new("col-anova-1")?;
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4)?;
    fill_col_values(&mut table.data_sets[0], "Control", &[10.1, 10.4, 9.9, 10.2]);
    let low_vals: &[f64] = if equal_replicates { &[12.8, 13.1, 12.6, 13.0] } else { &[12.8, 13.1, 12.6] };
    fill_col_values(&mut table.data_sets[1], "Low Dose", low_vals);
    fill_col_values(&mut table.data_sets[2], "High Dose", &[16.4, 16.9, 16.2, 16.7]);

    let ds = DataSheet::new(sheet_id.clone(), "Dose Response Column", TableArchetype::Column(table))?;
    let mut doc = app.document().clone();
    doc.data_sheets = vec![ds];
    doc.active_sheet_id = Some(sheet_id.clone());
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();
    Ok((app, sheet_id))
}

struct CoexistDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    outside_click_captured: bool,
    outside_wheel_captured: bool,
    dialog_rect_pt: Option<egui::Rect>,
}

impl CoexistDriverState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), outside_click_captured: false, outside_wheel_captured: false, dialog_rect_pt: None }
    }
}

fn make_coexist_driver<'a>(state: CoexistDriverState, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, CoexistDriverState> {
    let mut driver = EguiHumanPathDriver::new(state, metrics, |ui, meta, st| {
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.outside_click_captured |= rep.outside_click_captured;
        st.outside_wheel_captured |= rep.outside_wheel_captured;
        st.dialog_rect_pt = rep.regions.dialog_rect_pt;
    });
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn verify_analyze_backdrop_blocks_grid(
    driver: &mut EguiHumanPathDriver<'_, CoexistDriverState>,
    modal_rect: egui::Rect,
) -> Result<(), Box<dyn std::error::Error>> {
    let outside_pos = egui::pos2(8.0, 120.0);
    assert!(!modal_rect.contains(outside_pos));

    let prev_scroll = driver.state().views.column_grid.scroll_offset_pt;
    let prev_active = driver.state().app.grid.tool.active;
    driver.state_mut().outside_click_captured = false;
    driver.state_mut().outside_wheel_captured = false;
    driver.wheel_at(outside_pos, egui::vec2(0.0, -180.0))?;
    driver.click_at(outside_pos)?;
    assert!(driver.state().outside_click_captured, "modal backdrop must capture outside click over Column grid");
    assert!(driver.state().outside_wheel_captured, "modal backdrop must capture outside wheel over Column grid");
    assert_eq!(driver.state().views.column_grid.scroll_offset_pt, prev_scroll);
    assert_eq!(driver.state().app.grid.tool.active, prev_active);
    Ok(())
}

fn scroll_and_select_one_way_anova(
    driver: &mut EguiHumanPathDriver<'_, CoexistDriverState>,
    metrics: ViewportMetrics,
) -> Result<(), Box<dyn std::error::Error>> {
    let snap_before = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, Some("dialog"), true);
    assert_eq!(snap_before.active_dialog, "analyze_sheet");
    let dlg_before = snap_before.dialog.as_ref().ok_or("missing AnalyzeSheet inspect dialog")?;
    assert!(dlg_before.controls.iter().any(|c| c.widget_id.starts_with("analyze.entry.")));
    assert!(dlg_before.buttons.iter().any(|b| b.widget_id == "modal.ok"));
    assert!(dlg_before.buttons.iter().any(|b| b.widget_id == "modal.cancel"));

    for _ in 0..8 {
        if driver
            .frame_meta()
            .find_by_widget_id("analyze.entry.stats.one_way_anova")
            .is_some_and(|e| e.clipped_rect.height() >= 8.0 && e.clipped_rect.width() >= 8.0)
        {
            break;
        }
        let anchor = driver
            .frame_meta()
            .entries
            .iter()
            .find(|e| e.widget_id.starts_with("analyze.entry.") || e.widget_id == "analyze.search")
            .map(|e| e.clipped_rect.center())
            .ok_or("missing AnalyzeSheet entry anchor")?;
        driver.wheel_at(anchor, egui::vec2(0.0, -140.0))?;
    }
    driver.run_settled();

    let snap_b = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, Some("dialog"), true);
    assert_eq!(snap_b.active_dialog, "analyze_sheet");
    let dlg_b = snap_b.dialog.as_ref().ok_or("missing AnalyzeSheet inspect dialog")?;
    assert!(dlg_b.controls.iter().any(|c| c.widget_id == "analyze.entry.stats.one_way_anova"));
    assert!(dlg_b.buttons.iter().any(|b| b.widget_id == "modal.ok"));
    assert!(dlg_b.buttons.iter().any(|b| b.widget_id == "modal.cancel"));

    driver.click_widget_id("analyze.entry.stats.one_way_anova")?;
    driver.click_widget_id("modal.ok")?;
    Ok(())
}

#[test]
fn egui_analyze_modal_over_column_grid_and_focus_handoff_to_resultsheet() -> Result<(), Box<dyn std::error::Error>> {
    let (app, col_id) = seed_column_anova_app(true)?;
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let mut driver = make_coexist_driver(CoexistDriverState::new(app), metrics);

    // Initially on Egui Column grid with Analyze… button enabled and no modal open.
    let d0 = HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false);
    assert_eq!(d0.sheet_surface, HybridSheetSurface::EguiColumnGrid);
    assert_eq!(d0.modal_surface, HybridModalSurface::None);
    assert!(!d0.blocks_egui_grid);

    // Click `toolbar.analyze` in egui top chrome -> opens native egui `AnalyzeSheet` modal.
    driver.click_widget_id("toolbar.analyze")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));

    let d_analyze = HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false);
    assert_eq!(d_analyze.sheet_surface, HybridSheetSurface::EguiColumnGrid);
    assert_eq!(d_analyze.modal_surface, HybridModalSurface::EguiAnalyzeSheet);
    assert!(d_analyze.blocks_egui_grid);

    let modal_rect = driver.state().dialog_rect_pt.ok_or("expected egui AnalyzeSheet modal rect")?;
    verify_analyze_backdrop_blocks_grid(&mut driver, modal_rect)?;
    scroll_and_select_one_way_anova(&mut driver, metrics)?;

    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let rs_id = driver.state().app.document().active_sheet_id.clone().ok_or("expected active ResultSheet")?;
    assert_ne!(rs_id, col_id);
    assert!(driver.state().app.document().result_sheet(&rs_id).is_some(), "expected clean focus handoff to newly created ResultSheet");

    let d_result = HybridCoexistenceArbiter::evaluate(&driver.state().app, false);
    assert_eq!(d_result.sheet_surface, HybridSheetSurface::EguiStage);
    assert_eq!(d_result.modal_surface, HybridModalSurface::None);
    assert!(HybridCoexistenceArbiter::allows_cmd_t_edit_params(&driver.state().app, &d_result, false));
    Ok(())
}

#[test]
fn egui_cmd_t_opens_anova_modal_over_resultsheet_with_backdrop_capture() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, col_id) = seed_column_anova_app(true)?;
    let _ = app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-anova-a", "source_sheet_id": col_id.as_str(), "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "res-anova-a" }));
    let rs_id_str = "res-anova-a";
    app.sync_views();

    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let mut driver = make_coexist_driver(CoexistDriverState::new(app), metrics);
    let initial_result_tab = driver.state().app.canvas_dialogs.result_view.active_tab_index;

    // Press `Cmd+T` directly over ResultSheet -> opens `egui` One-Way ANOVA modal!
    driver.press_key("t", KeyModifiers { meta: true, ..Default::default() })?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    let d_case_a = HybridCoexistenceArbiter::evaluate(&driver.state().app, false);
    assert_eq!(d_case_a.sheet_surface, HybridSheetSurface::EguiStage);
    assert_eq!(d_case_a.modal_surface, HybridModalSurface::EguiOneWayAnova);
    assert!(d_case_a.renders_egui_anova_modal);

    // Footer button must read `"Update Analysis"` when opened via `Cmd+T` on an existing `ResultSheet`.
    let snap_a = driver.state_mut().app.inspect_egui_shell_snapshot(metrics, Some("dialog"), true);
    let dlg_a = snap_a.dialog.as_ref().ok_or("expected egui ANOVA dialog snapshot")?;
    assert_eq!(dlg_a.dialog_kind, "edit_analysis_parameters");
    let ok_btn = dlg_a.buttons.iter().find(|b| b.widget_id == "modal.ok").ok_or("missing modal.ok")?;
    assert_eq!(ok_btn.label, "Update Analysis");

    // Outside click & outside wheel behind the egui ANOVA modal must NOT leak to ResultSheet behind it!
    let dlg_rect = driver.state().dialog_rect_pt.ok_or("expected egui ANOVA dialog rect")?;
    let outside = egui::pos2(6.0, 120.0);
    assert!(!dlg_rect.contains(outside));
    driver.state_mut().outside_click_captured = false;
    driver.state_mut().outside_wheel_captured = false;
    driver.wheel_at(outside, egui::vec2(0.0, -180.0))?;
    driver.click_at(outside)?;
    assert!(driver.state().outside_click_captured, "Case A modal.backdrop must capture outside clicks");
    assert!(driver.state().outside_wheel_captured, "Case A modal.backdrop must capture outside wheel");
    assert_eq!(driver.state().app.canvas_dialogs.result_view.active_tab_index, initial_result_tab);

    // Toggle `edit.assume_gaussian` OFF -> Dynamic Resolved-Test Banner updates live to Kruskal-Wallis + Dunn,
    // and `edit.post_hoc.tukey` stays VISIBLE & disabled with unlock tooltip (`R2`).
    assert!(resolve_one_way_anova_banner(&driver.state().app).contains("Ordinary one-way ANOVA"));
    driver.click_widget_id("edit.assume_gaussian")?;
    assert!(!driver.state().app.canvas_dialogs.edit_params.assume_gaussian);
    assert!(resolve_one_way_anova_banner(&driver.state().app).contains("Kruskal-Wallis"));

    let tukey = driver.frame_meta().find_by_widget_id("edit.post_hoc.tukey").ok_or("Tukey option must remain visible when disabled (R2)")?.clone();
    assert!(!tukey.enabled, "Tukey must be disabled under nonparametric Kruskal-Wallis");
    driver.hover_widget_id("edit.post_hoc.tukey")?;
    let tooltips = driver.visible_tooltip_texts();
    assert!(
        tooltips.iter().any(|t| t.to_ascii_lowercase().contains("nonparametric") || t.contains("Dunn")),
        "expected visible unlock tooltip on disabled Tukey control, got {tooltips:?}"
    );

    // Confirm via `modal.ok` -> ResultSheet updates cleanly and dialog closes.
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let rs = driver.state().app.document().result_sheet(&SheetId::try_new(rs_id_str)?).ok_or("missing updated ResultSheet")?;
    assert!(
        format!("{:?}", rs.analysis.kind).contains("KruskalWallis"),
        "expected updated ResultSheet analysis kind KruskalWallis, got {:?}",
        rs.analysis.kind
    );
    Ok(())
}

#[test]
fn anova_modal_predicates_r1_to_r9_tabs_and_method_ref_behave_cleanly() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create-mode label check: when target result_sheet_id is not yet saved in `doc.result_sheets`, footer says `"Run Analysis"`.
    let (mut app_unequal, col_unequal) = seed_column_anova_app(false)?;
    let _ = app_unequal.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-unequal", "source_sheet_id": col_unequal.as_str(), "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let rs_id = SheetId::try_new("res-unequal")?;
    let doc_unequal = app_unequal.document().clone();
    assert!(app_unequal.canvas_dialogs.open_edit_parameters(&doc_unequal, &rs_id));
    app_unequal.sync_views();

    let mut create_app = app_unequal.clone();
    create_app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: SheetId::try_new("unsaved-draft")? };
    let mut create_state = AnovaModalEguiState::default();
    let create_ctx = egui::Context::default();
    let mut create_meta = FrameInspectMeta::default();
    let mut create_intents = PendingUiIntents::new();
    let tokens = Tokens::default();
    let mut create_out = create_ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))), ..Default::default() },
        |ui| {
            create_meta.begin_pass(ui.ctx());
            let _ = render_one_way_anova_modal(ui.ctx(), &create_app, &mut create_state, &tokens, &mut create_meta, &mut create_intents);
        },
    );
    create_out.textures_delta.clear();
    let create_ok = create_meta.find_by_widget_id("modal.ok").ok_or("missing modal.ok in create mode")?;
    assert_eq!(create_ok.label, "Run Analysis");

    // 2. On unequal-replicate table, `R1` disables `edit.paired_or_matched` AND blocks `edit.tab.repeated_measures` with visible hover tooltips!
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let mut driver = make_coexist_driver(CoexistDriverState::new(app_unequal), metrics);

    let paired_node = driver.frame_meta().find_by_widget_id("edit.paired_or_matched").ok_or("missing edit.paired_or_matched")?;
    assert!(!paired_node.enabled, "R1 must disable paired_or_matched when replicate counts are unequal");
    let rm_tab = driver.frame_meta().find_by_widget_id("edit.tab.repeated_measures").ok_or("missing edit.tab.repeated_measures")?;
    assert!(!rm_tab.enabled, "R1 must block Repeated Measures tab when paired_or_matched is false");
    driver.hover_widget_id("edit.tab.repeated_measures")?;
    let rm_tips = driver.visible_tooltip_texts();
    assert!(
        rm_tips.iter().any(|t| t.contains("Paired") || t.contains("Repeated")),
        "hovering blocked tab must show disabled-reason tooltip, got {rm_tips:?}"
    );

    // 3. Method Reference & Save as Default footer buttons toggle locally without closing modal.
    driver.click_widget_id("dialog.button.method_ref")?;
    assert!(driver.state().views.anova_modal.show_method_reference);
    driver.click_widget_id("dialog.button.method_ref")?;
    assert!(!driver.state().views.anova_modal.show_method_reference);
    driver.click_widget_id("dialog.button.save_default")?;
    assert!(driver.state().views.anova_modal.method_saved_as_default);

    // 4. Switch to Tab 3 (`Multiple Comparisons`) and verify `R3` keeps `edit.control_column` visible & disabled until `compare_against_control` checked!
    driver.click_widget_id("edit.tab.multiple_comparisons")?;
    assert_eq!(driver.state().views.anova_modal.active_tab, AnovaModalTab::MultipleComparisons);
    let ctrl_col_disabled =
        driver.frame_meta().find_by_widget_id("edit.control_column").ok_or("R3: edit.control_column must remain visible when disabled")?;
    assert!(!ctrl_col_disabled.enabled);
    driver.hover_widget_id("edit.control_column")?;
    let r3_tips = driver.visible_tooltip_texts();
    assert!(r3_tips.iter().any(|t| t.contains("control") || t.contains("Dunnett")), "expected R3 unlock tooltip, got {r3_tips:?}");
    driver.click_widget_id("edit.compare_against_control")?;
    let ctrl_col_enabled = driver.frame_meta().find_by_widget_id("edit.control_column").ok_or("missing enabled control_column")?;
    assert!(ctrl_col_enabled.enabled);

    // 5. Switch to Tab 4 (`Options`) and verify `R9` keeps `edit.pvalue.digits.val` disabled until `edit.pval.custom` is chosen!
    driver.click_widget_id("edit.tab.options")?;
    let pval_digits_off =
        driver.frame_meta().find_by_widget_id("edit.pvalue.digits.val").ok_or("R9: pvalue.digits must stay visible when disabled")?;
    assert!(!pval_digits_off.enabled);
    driver.hover_widget_id("edit.pvalue.digits.inc")?;
    let r9_tips = driver.visible_tooltip_texts();
    assert!(r9_tips.iter().any(|t| t.contains("Custom")), "expected R9 unlock tooltip mentioning Custom, got {r9_tips:?}");
    driver.click_widget_id("edit.pval.custom")?;
    let pval_digits_on = driver.frame_meta().find_by_widget_id("edit.pvalue.digits.val").ok_or("missing enabled pvalue.digits")?;
    assert!(pval_digits_on.enabled);

    // 6. Switch to equal-N table and verify `R1b` REML disables sphericity & Tukey while banner reflects RM/REML!
    let (mut app_eq, col_eq) = seed_column_anova_app(true)?;
    let _ = app_eq.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-eq", "source_sheet_id": col_eq.as_str(), "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let rs_eq = SheetId::try_new("res-eq")?;
    let doc_eq = app_eq.document().clone();
    assert!(app_eq.canvas_dialogs.open_edit_parameters(&doc_eq, &rs_eq));
    let mut eq_driver = make_coexist_driver(CoexistDriverState::new(app_eq), metrics);
    eq_driver.click_widget_id("edit.paired_or_matched")?;
    assert!(eq_driver.state().app.canvas_dialogs.edit_params.paired_or_matched);
    let rm_tab_unlocked = eq_driver.frame_meta().find_by_widget_id("edit.tab.repeated_measures").ok_or("missing unlocked RM tab")?;
    assert!(rm_tab_unlocked.enabled);
    eq_driver.click_widget_id("edit.tab.repeated_measures")?;
    assert_eq!(eq_driver.state().views.anova_modal.active_tab, AnovaModalTab::AssumptionsAndTest);
    eq_driver.click_widget_id("edit.rm.mixed_effects_reml")?;
    assert!(resolve_one_way_anova_banner(&eq_driver.state().app).contains("Mixed-effects REML"));
    let sph_after_reml = eq_driver.frame_meta().find_by_widget_id("edit.sphericity.geisser_greenhouse").ok_or("missing sphericity control")?;
    assert!(!sph_after_reml.enabled, "R1b must disable classical sphericity when REML is active");
    Ok(())
}

#[test]
fn all_14_beta_locales_and_hybrid_screenshots_pass_at_640x420_and_1440x600() -> Result<(), Box<dyn std::error::Error>> {
    let (mut base_app, col_id) = seed_column_anova_app(true)?;
    let _ = base_app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-locales", "source_sheet_id": col_id.as_str(), "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;
    let rs_id = SheetId::try_new("res-locales")?;

    // Check `BETA_LOCALE_CODES` parity at min breakpoint `640x420` and wide `1440x600`.
    assert_eq!(BETA_LOCALE_CODES.len(), 14);
    for locale in BETA_LOCALE_CODES {
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            let mut app = base_app.clone();
            assert_eq!(app.set_locale(locale), locale);
            let doc_loc = app.document().clone();
            assert!(app.canvas_dialogs.open_edit_parameters(&doc_loc, &rs_id));
            app.sync_views();
            let metrics = ViewportMetrics::new(w_pt, h_pt, 1.0);
            let mut driver = make_coexist_driver(CoexistDriverState::new(app), metrics);
            driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);

            let report = driver.verify_pixel_to_hit_box_parity();
            assert!(report.is_clean(), "locale `{locale}` @ {w_pt}x{h_pt} parity violations: {:?}", report.violations);
            let sb_violations = driver.check_scrollbar_non_overlap();
            assert!(sb_violations.is_empty(), "locale `{locale}` @ {w_pt}x{h_pt} scrollbar overlap: {sb_violations:?}");

            // Outside click & wheel capture at both viewport extremes (`640x420` and `1440x600`).
            let outside = egui::pos2(4.0, (h_pt as f32) * 0.5);
            driver.wheel_at(outside, egui::vec2(0.0, -90.0))?;
            driver.click_at(outside)?;
            assert!(driver.state().outside_click_captured, "locale `{locale}` @ {w_pt}x{h_pt} failed outside click capture");
            assert!(driver.state().outside_wheel_captured, "locale `{locale}` @ {w_pt}x{h_pt} failed outside wheel capture");
        }
    }

    // Verify full hybrid CPU software rasterizer screenshots (`ui.screenshot`) for BOTH coexistence modes at 640x420 & 1440x600.
    for (w_px, h_px) in [(640_u32, 420_u32), (1440_u32, 600_u32)] {
        // Dark AnalyzeSheet over egui Column grid.
        let (mut app_b, _) = seed_column_anova_app(true)?;
        let doc_b = app_b.document().clone();
        assert!(app_b.canvas_dialogs.open_analyze_dialog(&doc_b, &col_id));
        app_b.set_theme_mode(UiThemeMode::Dark);
        let (png_b, bw, bh) = app_b
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: w_px, height_px: h_px, dpi: 96 }, ScreenshotRegionTarget::FullWindow)?;
        assert_eq!((bw, bh), (w_px, h_px));
        assert_eq!(png_dimensions(&png_b), Some((w_px, h_px)));

        // Light One-Way ANOVA modal over ResultSheet.
        let mut app_a = base_app.clone();
        let _ = app_a.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() }));
        let doc_a = app_a.document().clone();
        assert!(app_a.canvas_dialogs.open_edit_parameters(&doc_a, &rs_id));
        app_a.set_theme_mode(UiThemeMode::Light);
        let (png_a, aw, ah) = app_a
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: w_px, height_px: h_px, dpi: 96 }, ScreenshotRegionTarget::FullWindow)?;
        assert_eq!((aw, ah), (w_px, h_px));
        assert_eq!(png_dimensions(&png_a), Some((w_px, h_px)));
        assert_ne!(png_a, png_b);
    }

    // Also run `inspect_bridge::check_accesskit_scrollbar_non_overlap` on settled AccessKit TreeUpdate directly.
    let mut app_ak = base_app;
    let doc_ak = app_ak.document().clone();
    assert!(app_ak.canvas_dialogs.open_edit_parameters(&doc_ak, &rs_id));
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
