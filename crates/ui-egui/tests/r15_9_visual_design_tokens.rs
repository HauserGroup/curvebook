#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `R15.9` Visual Design System & Token Tests (`gui-layout-spec.md` §10 & `R15-GUI-37`).

use std::path::PathBuf;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, ErrorCode, ImportLinkSpec, InternalLinkKind, LinkSource, LinkedDataRegion, ProjectDocument, ResultSheet,
    ResultStatus, SheetCellRange, SheetId,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, accesskit_accessible_name, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::welcome_hub::WelcomeLeftMode;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, LayoutMetrics, PendingUiIntents, Tokens, color32_to_rgba8, fonts};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec, wcag_contrast_ratio};
use curvebook_ui_egui::{RenderTargetSpec, UiApp};
use serde_json::json;

struct R159ShellState {
    app: UiApp,
    views: EguiShellViewState,
    carried_intents: PendingUiIntents,
    last_err: Option<String>,
}

fn make_shell_driver(app: UiApp, width: f64, height: f64) -> EguiHumanPathDriver<'static, R159ShellState> {
    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        R159ShellState { app, views, carried_intents: PendingUiIntents::new(), last_err: None },
        ViewportMetrics::new(width, height, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass_intents);
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled && !st.carried_intents.is_empty() {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.carried_intents.commit_if_settled(&mut st.app, false) {
                st.last_err = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        }
    });
    driver.run_settled();
    driver
}

fn seed_xy_data_and_graph_app() -> UiApp {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-xy",
            "title": "Dose Response",
            "kind": "xy",
            "cols": 2,
            "rows": 3,
            "auto_graph": false
        }),
    )
    .expect("create xy sheet");
    app.execute_command("table.setRowTitles", json!({ "sheet_id": "ds-xy", "titles": ["Control", "Low Dose", "High Dose"] }))
        .expect("set row titles");
    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-xy", "values": [1.0, 2.0, 4.0] })).expect("set x cells");
    app.execute_command("table.setCells", json!({ "sheet_id": "ds-xy", "rows": [[10.0, 12.0], [25.0, 28.0], [50.0, 52.0]] })).expect("set y cells");
    app.execute_command("graph.create", json!({ "sheet_id": "gs-1", "data_sheet_id": "ds-xy", "title": "Dose Curve" })).expect("create graph");
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).expect("select data sheet");
    app.sync_views();
    app
}

fn add_linked_region_to_doc(doc: &mut ProjectDocument, target_id: &SheetId, region: LinkedDataRegion) {
    let ds = doc.data_sheet_mut(target_id).expect("target data sheet");
    ds.add_linked_region(region).expect("add linked region");
}

#[test]
fn tokens_meet_wcag_aa_and_r15_gui_37_hover_contrast_across_all_modes() {
    for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        let spec = UiThemeSpec::for_mode(mode);
        assert!(spec.meets_wcag_aa(), "UiThemeSpec WCAG AA text failed for {mode:?}");
        assert!(spec.meets_wcag_aa_borders(), "UiThemeSpec WCAG AA border failed for {mode:?}");

        let t = Tokens::for_mode(mode);
        assert!(t.meets_wcag_aa(), "Tokens::meets_wcag_aa() failed for {mode:?}");

        let hover_vs_panel = wcag_contrast_ratio(color32_to_rgba8(t.hover_bg), color32_to_rgba8(t.panel_bg));
        assert!(hover_vs_panel >= 1.30, "{mode:?} hover_bg vs panel_bg ratio {hover_vs_panel:.2} < 1.30 (R15-GUI-37)");

        let hover_vs_btn = wcag_contrast_ratio(color32_to_rgba8(t.hover_bg), color32_to_rgba8(t.button_bg));
        assert!(hover_vs_btn >= 1.25, "{mode:?} hover_bg vs button_bg ratio {hover_vs_btn:.2} < 1.25 (R15-GUI-37)");

        let hover_border_ratio = wcag_contrast_ratio(color32_to_rgba8(t.hover_border), color32_to_rgba8(t.hover_bg));
        assert!(hover_border_ratio >= 3.0, "{mode:?} hover_border vs hover_bg ratio {hover_border_ratio:.2} < 3.0");

        let on_accent_strong = wcag_contrast_ratio(color32_to_rgba8(t.on_accent), color32_to_rgba8(t.accent_strong));
        assert!(on_accent_strong >= 4.5, "{mode:?} on_accent vs accent_strong {on_accent_strong:.2} < 4.5");

        let on_accent_hover = wcag_contrast_ratio(color32_to_rgba8(t.on_accent), color32_to_rgba8(t.accent_hover));
        assert!(on_accent_hover >= 4.5, "{mode:?} on_accent vs accent_hover {on_accent_hover:.2} < 4.5");

        for (label, bg) in [("panel_bg", t.panel_bg), ("surface_bg", t.surface_bg)] {
            let muted_ratio = wcag_contrast_ratio(color32_to_rgba8(t.text_muted), color32_to_rgba8(bg));
            assert!(muted_ratio >= 4.5, "{mode:?} text_muted vs {label} {muted_ratio:.2} < 4.5");
        }
    }
}

#[test]
fn layout_metrics_canonical_scale_motion_cap_and_style_tokens() {
    let m = LayoutMetrics::CANONICAL;
    assert_eq!((m.space_xxs, m.space_xs, m.space_sm, m.space_md, m.space_lg, m.space_xl, m.space_xxl), (2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0));
    assert_eq!(m.font_ui_small_pt, 11.5);
    assert_eq!(m.font_grid_numeral_pt, 12.5);
    assert_eq!(m.font_ui_body_pt, 13.0);
    assert_eq!(m.font_ui_section_pt, 13.0);
    assert_eq!(m.font_ui_title_pt, 15.0);
    assert_eq!(m.font_ui_display_pt, 18.0);
    assert_eq!((m.radius_sm, m.radius_md, m.radius_lg), (2, 4, 6));
    assert!((m.max_motion_sec - 0.15).abs() < 1e-6);

    let tokens = Tokens::for_mode(UiThemeMode::Light);
    let mut style = egui::Style { animation_time: 0.25, ..Default::default() };
    tokens.configure_style(&mut style);
    assert!(style.animation_time <= 0.15 + 1e-6, "motion cap violated: {}", style.animation_time);
    assert_eq!(style.text_styles[&egui::TextStyle::Small].size, 11.5);
    assert_eq!(style.text_styles[&egui::TextStyle::Body].size, 13.0);
    assert_eq!(style.text_styles[&egui::TextStyle::Button].size, 13.0);
    assert_eq!(style.text_styles[&egui::TextStyle::Monospace].size, 12.5);
    assert_eq!(style.text_styles[&egui::TextStyle::Heading].size, 15.0);
    assert_eq!(style.visuals.window_corner_radius, egui::CornerRadius::same(6));
    assert_eq!(style.visuals.widgets.inactive.corner_radius, egui::CornerRadius::same(4));
    assert_eq!(style.visuals.widgets.hovered.bg_stroke, egui::Stroke::new(1.0, tokens.hover_border));

    let mut reduced = egui::Style { animation_time: 0.0, ..Default::default() };
    tokens.configure_style(&mut reduced);
    assert_eq!(reduced.animation_time, 0.0, "configure_style must preserve reduced motion (0.0s)");

    // Verify section heading helper respects Context bold-face availability.
    let ctx = egui::Context::default();
    let built = fonts::build_egui_font_definitions();
    fonts::install_font_definitions(&ctx, built.definitions);
    let expected_size = if fonts::has_bold_font(&ctx) { 13.0 } else { 14.0 };
    assert_eq!(fonts::section_heading_font_id(&ctx, &tokens).size, expected_size);
}

#[test]
fn broken_link_provenance_exposes_non_color_text_and_tooltip() {
    // Case 1: missing upstream DataSheet.
    let mut app = seed_xy_data_and_graph_app();
    let target_id = SheetId::from("ds-xy");
    let mut doc = app.document().clone();
    add_linked_region_to_doc(
        &mut doc,
        &target_id,
        LinkedDataRegion {
            id: String::from("link-missing-ds"),
            spec: ImportLinkSpec {
                source: LinkSource::InternalSheet {
                    sheet_id: SheetId::from("ds-deleted"),
                    kind: InternalLinkKind::DataSheet,
                    tab_id: None,
                    range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
                },
                filter: Default::default(),
                placement: Default::default(),
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            provenance_summary: String::from("Deleted Table › A1:A2"),
        },
    );
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 800.0);
    let indicator =
        driver.frame_meta().find_by_widget_id("status.link_indicator").expect("status.link_indicator should render for linked sheet").clone();
    assert!(indicator.label.contains("Broken"), "broken link must use explicit non-color label, got `{}`", indicator.label);

    driver.hover_widget_id("status.link_indicator").expect("hover broken link indicator");
    let tooltips = driver.visible_tooltip_texts();
    let ak_desc = driver
        .accesskit_tree()
        .and_then(|u| u.nodes.iter().find(|(id, _)| *id == indicator.accesskit_id))
        .and_then(|(_, node)| node.description())
        .map(str::to_owned);
    let ak_name = driver
        .accesskit_tree()
        .and_then(|u| u.nodes.iter().find(|(id, _)| *id == indicator.accesskit_id))
        .and_then(|(_, node)| accesskit_accessible_name(node))
        .map(str::to_owned);
    let exposed_broken_copy =
        tooltips.iter().any(|t| {
            let low = t.to_ascii_lowercase();
            low.contains("broken") || low.contains("missing")
        }) || ak_desc.as_deref().is_some_and(|s| s.to_ascii_lowercase().contains("broken") || s.to_ascii_lowercase().contains("missing"))
            || ak_name.as_deref().is_some_and(|s| s.contains("Broken"));
    assert!(
        exposed_broken_copy,
        "hovering broken link indicator must expose non-color text/tooltip (tooltips: {tooltips:?}, ak_desc: {ak_desc:?}, ak_name: {ak_name:?})"
    );

    // Case 2: upstream ResultSheet in Failed status.
    let mut app_failed = seed_xy_data_and_graph_app();
    let mut doc2 = app_failed.document().clone();
    let mut spec = AnalysisSpec::new("res-failed", "ds-xy", AnalysisKind::DescriptiveStats(Default::default())).expect("analysis spec");
    spec.custom_title = Some(String::from("Failed Descriptive"));
    let mut failed_rs = ResultSheet::new(spec).expect("result sheet");
    failed_rs.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("insufficient numeric observations") };
    doc2.add_result_sheet(failed_rs).expect("add failed result sheet");
    add_linked_region_to_doc(
        &mut doc2,
        &target_id,
        LinkedDataRegion {
            id: String::from("link-failed-rs"),
            spec: ImportLinkSpec {
                source: LinkSource::InternalSheet {
                    sheet_id: SheetId::from("res-failed"),
                    kind: InternalLinkKind::AnalyzableResultSheet,
                    tab_id: None,
                    range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
                },
                filter: Default::default(),
                placement: Default::default(),
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            provenance_summary: String::from("Failed Descriptive › A1"),
        },
    );
    *app_failed.session_mut() = Session::from_document(doc2);
    app_failed.sync_views();

    let driver2 = make_shell_driver(app_failed, 1280.0, 800.0);
    let indicator2 = driver2.frame_meta().find_by_widget_id("status.link_indicator").expect("status.link_indicator for failed result");
    assert!(indicator2.label.contains("Broken"), "failed upstream ResultSheet must mark link as Broken, got `{}`", indicator2.label);
}

#[test]
fn modal_footers_follow_cancel_secondary_primary_order_and_render_cleanly() {
    // 1. Unsaved Changes modal: Cancel -> Don't Save -> Save.
    let mut app_unsaved = seed_xy_data_and_graph_app();
    app_unsaved.egui_views.app_modals.unsaved_changes_open = true;
    let driver_unsaved = make_shell_driver(app_unsaved, 1280.0, 800.0);
    let cancel_r = driver_unsaved.frame_meta().find_by_widget_id("dialog.button.cancel").expect("unsaved cancel").clipped_rect;
    let dont_save_r = driver_unsaved.frame_meta().find_by_widget_id("dialog.button.dont_save").expect("unsaved dont_save").clipped_rect;
    let save_r = driver_unsaved.frame_meta().find_by_widget_id("dialog.button.save").expect("unsaved save").clipped_rect;
    assert!(cancel_r.left() < dont_save_r.left() && dont_save_r.left() < save_r.left(), "UnsavedChanges footer order mismatch");
    let parity_unsaved = driver_unsaved.verify_pixel_to_hit_box_parity();
    assert!(parity_unsaved.is_clean(), "UnsavedChanges parity violations: {:?}", parity_unsaved.violations);

    // 2. Feedback modal: Clear -> Copy Markdown -> Open GitHub Issue.
    let mut app_feedback = seed_xy_data_and_graph_app();
    app_feedback.canvas_dialogs.active_dialog = ActiveDialog::Feedback;
    app_feedback.sync_views();
    let driver_feedback = make_shell_driver(app_feedback, 1280.0, 800.0);
    let reset_r = driver_feedback.frame_meta().find_by_widget_id("dialog.feedback.reset").expect("feedback reset").clipped_rect;
    let copy_r = driver_feedback.frame_meta().find_by_widget_id("dialog.feedback.copy_markdown").expect("feedback copy").clipped_rect;
    let open_r = driver_feedback.frame_meta().find_by_widget_id("dialog.feedback.open_github").expect("feedback open").clipped_rect;
    assert!(reset_r.left() < copy_r.left() && copy_r.left() < open_r.left(), "Feedback footer order mismatch");
    let parity_feedback = driver_feedback.verify_pixel_to_hit_box_parity();
    assert!(parity_feedback.is_clean(), "Feedback parity violations: {:?}", parity_feedback.violations);

    // 3. Welcome Hub Clone Options modal: Close -> Apply.
    let mut app_clone = seed_xy_data_and_graph_app();
    app_clone.nav.welcome.open = true;
    app_clone.sync_views();
    app_clone.egui_views.welcome_hub.left_mode = WelcomeLeftMode::ClonePipeline;
    app_clone.egui_views.welcome_hub.clone_options_open = true;
    let driver_clone = make_shell_driver(app_clone, 1280.0, 800.0);
    let close_r = driver_clone.frame_meta().find_by_widget_id("welcome.clone_modal.close").expect("clone close").clipped_rect;
    let apply_r = driver_clone.frame_meta().find_by_widget_id("welcome.clone_modal.apply").expect("clone apply").clipped_rect;
    assert!(close_r.left() < apply_r.left(), "CloneOptions footer order mismatch");
    let parity_clone = driver_clone.verify_pixel_to_hit_box_parity();
    assert!(parity_clone.is_clean(), "CloneOptions parity violations: {:?}", parity_clone.violations);

    // 4. Manage Variables modal: renders cleanly with zero parity violations.
    let mut app_mv = UiApp::new();
    app_mv.set_gui_only(true);
    app_mv
        .execute_command(
            "sheet.createData",
            json!({
                "sheet_id": "ds-mv",
                "title": "Multi Variables",
                "kind": "multiple_variables",
                "cols": 3,
                "rows": 4,
                "auto_graph": false
            }),
        )
        .expect("create mv sheet");
    app_mv.execute_command("sheet.select", json!({ "sheet_id": "ds-mv" })).expect("select mv sheet");
    let mv_ds = app_mv.document().data_sheet(&SheetId::from("ds-mv")).expect("mv sheet").clone();
    app_mv.grid.variables_dialog.open_for_column(&mv_ds, 0);
    app_mv.sync_views();
    let driver_mv = make_shell_driver(app_mv, 1280.0, 800.0);
    assert!(driver_mv.frame_meta().find_by_widget_id("dialog.manage_variables").is_some(), "ManageVariables modal missing");
    let parity_mv = driver_mv.verify_pixel_to_hit_box_parity();
    assert!(parity_mv.is_clean(), "ManageVariables parity violations: {:?}", parity_mv.violations);
}

#[test]
fn visual_review_renders_all_viewports_and_themes() {
    let out_dir: Option<PathBuf> = std::env::var_os("CURVEBOOK_VISUAL_REVIEW_DIR").filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = &out_dir {
        std::fs::create_dir_all(dir).expect("create visual review dir");
    }

    let cases: [(&str, UiThemeMode, u32, u32, Option<ActiveDialog>); 7] = [
        ("light_1280x800.png", UiThemeMode::Light, 1280, 800, None),
        ("dark_1280x800.png", UiThemeMode::Dark, 1280, 800, None),
        ("hc_1280x800.png", UiThemeMode::HighContrast, 1280, 800, None),
        ("light_1440x600.png", UiThemeMode::Light, 1440, 600, None),
        ("dark_640x420.png", UiThemeMode::Dark, 640, 420, None),
        ("modal_light_1280x800.png", UiThemeMode::Light, 1280, 800, Some(ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::from("ds-xy") })),
        ("modal_dark_1280x800.png", UiThemeMode::Dark, 1280, 800, Some(ActiveDialog::Feedback)),
    ];

    let mut rendered_full: Vec<(&str, Vec<u8>)> = Vec::new();
    for (filename, mode, width_px, height_px, dialog) in cases {
        let mut app = seed_xy_data_and_graph_app();
        app.theme = UiThemeSpec::for_mode(mode);
        let has_dialog = dialog.is_some();
        if let Some(dlg) = dialog {
            app.canvas_dialogs.active_dialog = dlg;
        }
        app.sync_views();
        let (png_bytes, rendered_w, rendered_h) = app
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px, height_px, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
            .expect("render screenshot png");
        assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{filename} did not encode as PNG");
        assert_eq!((rendered_w, rendered_h), (width_px, height_px), "{filename} dimensions mismatch");

        if has_dialog {
            let (dlg_bytes, dlg_w, dlg_h) = app
                .render_egui_shell_screenshot_png(RenderTargetSpec { width_px, height_px, dpi: 96 }, ScreenshotRegionTarget::Dialog)
                .expect("render dialog crop png");
            assert!(dlg_bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{filename} dialog crop did not encode as PNG");
            assert!(dlg_w > 100 && dlg_h > 100 && dlg_w < width_px && dlg_h < height_px, "{filename} dialog crop unexpected size {dlg_w}x{dlg_h}");
        }

        if let Some(dir) = &out_dir {
            std::fs::write(dir.join(filename), &png_bytes).expect("write visual review png");
        }
        rendered_full.push((filename, png_bytes));
    }

    let find_png = |name: &str| rendered_full.iter().find(|(n, _)| *n == name).map(|(_, b)| b.as_slice()).expect("png present");
    assert_ne!(find_png("modal_light_1280x800.png"), find_png("light_1280x800.png"), "light modal must visually overlay workspace");
    assert_ne!(find_png("modal_dark_1280x800.png"), find_png("dark_1280x800.png"), "dark modal must visually overlay workspace");
}
