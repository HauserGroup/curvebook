#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `R15.9` Visual Design System & Token Tests (`gui-layout-spec.md` §10 & `R15-GUI-37`).

use std::path::PathBuf;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, ConstantValue, ErrorCode, HookSource, ImportLinkSpec, InfoSheet, InternalLinkKind, LinkSource,
    LinkedDataRegion, ParameterRule, ParameterScope, PlateCellOrigin, ResultSheet, ResultStatus, SheetCellRange, SheetId,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::accesskit_accessible_name;
use curvebook_ui_egui::egui_shell::welcome_hub::WelcomeLeftMode;
use curvebook_ui_egui::egui_shell::{LayoutMetrics, Tokens, color32_to_rgba8, fonts, rgba8_to_color32};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::{UiThemeMode, UiThemeSpec, wcag_contrast_ratio};
use curvebook_ui_egui::{RenderTargetSpec, UiApp};
use serde_json::json;

use crate::common::{PNG_SIGNATURE, ShellTestState, make_shell_driver_with_state, seed_xy_data_and_graph_app};

fn make_driver(app: UiApp) -> curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver<'static, ShellTestState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app).with_merge_passes(), ViewportMetrics::new(1280.0, 800.0, 1.0), tokens)
}

fn seed_xy_app() -> UiApp {
    seed_xy_data_and_graph_app("ds-xy", "gs-1").unwrap().0
}

#[test]
fn tokens_meet_wcag_aa_and_r15_gui_37_hover_contrast_across_all_modes() {
    for mode in UiThemeMode::ALL {
        let spec = UiThemeSpec::for_mode(mode);
        assert!(spec.meets_wcag_aa() && spec.meets_wcag_aa_borders(), "UiThemeSpec WCAG AA failed for {mode:?}");
        let t = Tokens::for_mode(mode);
        assert!(t.meets_wcag_aa(), "Tokens::meets_wcag_aa() failed for {mode:?}");
        for (lbl, fg, bg, min_ratio) in [
            ("hover_bg/panel_bg", t.hover_bg, t.panel_bg, 1.30),
            ("hover_bg/button_bg", t.hover_bg, t.button_bg, 1.25),
            ("hover_border/hover_bg", t.hover_border, t.hover_bg, 3.0),
            ("on_accent/accent_strong", t.on_accent, t.accent_strong, 4.5),
            ("on_accent/accent_hover", t.on_accent, t.accent_hover, 4.5),
            ("text_muted/panel_bg", t.text_muted, t.panel_bg, 4.5),
            ("text_muted/surface_bg", t.text_muted, t.surface_bg, 4.5),
        ] {
            let ratio = wcag_contrast_ratio(color32_to_rgba8(fg), color32_to_rgba8(bg));
            assert!(ratio >= min_ratio, "{mode:?} {lbl} ratio {ratio:.2} < {min_ratio}");
        }
    }
}

#[test]
fn layout_metrics_canonical_scale_motion_cap_and_style_tokens() {
    let m = LayoutMetrics::CANONICAL;
    assert_eq!((m.space_xxs, m.space_xs, m.space_sm, m.space_md, m.space_lg, m.space_xl, m.space_xxl), (2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0));
    assert_eq!(
        (m.font_ui_small_pt, m.font_grid_numeral_pt, m.font_ui_body_pt, m.font_ui_section_pt, m.font_ui_title_pt, m.font_ui_display_pt),
        (11.5, 13.0, 13.0, 13.0, 15.0, 18.0)
    );
    assert_eq!((m.radius_sm, m.radius_md, m.radius_lg), (2, 4, 6));
    assert!((m.max_motion_sec - 0.15).abs() < 1e-6);

    let tokens = Tokens::for_mode(UiThemeMode::Light);
    let mut style = egui::Style { animation_time: 0.25, ..Default::default() };
    tokens.configure_style(&mut style);
    assert!(style.animation_time <= 0.15 + 1e-6);
    for (ts, sz) in [
        (egui::TextStyle::Small, 11.5),
        (egui::TextStyle::Body, 13.0),
        (egui::TextStyle::Button, 13.0),
        (egui::TextStyle::Monospace, 13.0),
        (egui::TextStyle::Heading, 15.0),
    ] {
        assert_eq!(style.text_styles[&ts].size, sz);
    }
    assert_eq!(style.visuals.window_corner_radius, egui::CornerRadius::same(6));
    assert_eq!(style.visuals.widgets.inactive.corner_radius, egui::CornerRadius::same(4));
    assert_eq!(style.visuals.widgets.hovered.bg_stroke, egui::Stroke::new(1.0, tokens.hover_border));

    let mut reduced = egui::Style { animation_time: 0.0, ..Default::default() };
    tokens.configure_style(&mut reduced);
    assert_eq!(reduced.animation_time, 0.0);

    let ctx = egui::Context::default();
    fonts::install_font_definitions(&ctx, fonts::build_egui_font_definitions().definitions);
    assert_eq!(fonts::section_heading_font_id(&ctx, &tokens).size, if fonts::has_bold_font(&ctx) { 13.0 } else { 14.0 });
}

#[test]
fn broken_link_provenance_exposes_non_color_text_and_tooltip() {
    let target_id = SheetId::from("ds-xy");
    let mk_region = |id: &str, src_id: &str, kind: InternalLinkKind, summary: &str| LinkedDataRegion {
        id: id.to_owned(),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::from(src_id),
                kind,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            },
            filter: Default::default(),
            placement: Default::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
        provenance_summary: summary.to_owned(),
    };

    for (check_tooltip, add_failed_rs, region) in [
        (true, false, mk_region("link-missing-ds", "ds-deleted", InternalLinkKind::DataSheet, "Deleted Table › A1:A2")),
        (false, true, mk_region("link-failed-rs", "res-failed", InternalLinkKind::AnalyzableResultSheet, "Failed Descriptive › A1")),
    ] {
        let mut app = seed_xy_app();
        let mut doc = app.document().clone();
        if add_failed_rs {
            let mut spec = AnalysisSpec::new("res-failed", "ds-xy", AnalysisKind::DescriptiveStats(Default::default())).unwrap();
            spec.custom_title = Some(String::from("Failed Descriptive"));
            let mut rs = ResultSheet::new(spec).unwrap();
            rs.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("insufficient numeric observations") };
            doc.add_result_sheet(rs).unwrap();
        }
        doc.data_sheet_mut(&target_id).unwrap().add_linked_region(region).unwrap();
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();

        let mut driver = make_driver(app);
        let indicator = driver.frame_meta().find_by_widget_id("status.link_indicator").expect("status.link_indicator").clone();
        assert!(indicator.label.contains("Broken"), "got `{}`", indicator.label);
        if check_tooltip {
            driver.hover_widget_id("status.link_indicator").unwrap();
            let tooltips = driver.visible_tooltip_texts();
            let ak_node = driver.accesskit_tree().and_then(|u| u.nodes.iter().find(|(id, _)| *id == indicator.accesskit_id)).map(|(_, n)| n);
            let exposed = tooltips.iter().any(|t| t.to_ascii_lowercase().contains("broken") || t.to_ascii_lowercase().contains("missing"))
                || ak_node
                    .and_then(|n| n.description())
                    .is_some_and(|s| s.to_ascii_lowercase().contains("broken") || s.to_ascii_lowercase().contains("missing"))
                || ak_node.and_then(accesskit_accessible_name).is_some_and(|s| s.contains("Broken"));
            assert!(exposed, "expected non-color broken-link text/tooltip, tooltips={tooltips:?}");
        }
    }
}

#[test]
fn modal_footers_follow_cancel_secondary_primary_order_and_render_cleanly() {
    let mut app_unsaved = seed_xy_app();
    app_unsaved.egui_views.app_modals.unsaved_changes_open = true;

    let mut app_about = seed_xy_app();
    app_about.canvas_dialogs.active_dialog = ActiveDialog::About;
    app_about.sync_views();

    let mut app_clone = seed_xy_app();
    app_clone.nav.welcome.open = true;
    app_clone.sync_views();
    app_clone.egui_views.welcome_hub.left_mode = WelcomeLeftMode::ClonePipeline;
    app_clone.egui_views.welcome_hub.clone_options_open = true;

    let mut app_mv = UiApp::new();
    app_mv
        .execute_command(
            "sheet.createData",
            json!({ "sheet_id": "ds-mv", "title": "Multi Variables", "kind": "multiple_variables", "cols": 3, "rows": 4, "auto_graph": false }),
        )
        .unwrap();
    app_mv.execute_command("sheet.select", json!({ "sheet_id": "ds-mv" })).unwrap();
    let mv_ds = app_mv.document().data_sheet(&SheetId::from("ds-mv")).unwrap().clone();
    app_mv.grid.variables_dialog.open_for_column(&mv_ds, 0);
    app_mv.sync_views();

    let cases: [(&str, UiApp, &[&str]); 4] = [
        ("UnsavedChanges", app_unsaved, &["dialog.button.dont_save", "dialog.button.cancel", "dialog.button.save"]),
        ("About", app_about, &["dialog.about.copy_version", "dialog.about.send_feedback", "dialog.about.close"]),
        ("CloneOptions", app_clone, &["welcome.clone_modal.close", "welcome.clone_modal.apply"]),
        ("ManageVariables", app_mv, &["dialog.manage_variables"]),
    ];
    for (name, app, ordered_wids) in cases {
        let driver = make_driver(app);
        let rects: Vec<egui::Rect> = ordered_wids.iter().map(|w| driver.frame_meta().find_by_widget_id(w).expect(w).clipped_rect).collect();
        let lefts: Vec<f32> = rects.iter().map(|r| r.left()).collect();
        for pair in lefts.windows(2) {
            assert!(pair[0] < pair[1], "{name} footer order mismatch: {lefts:?}");
        }
        if name == "UnsavedChanges" {
            let leading_gap = rects[1].left() - rects[0].right();
            let trailing_gap = rects[2].left() - rects[1].right();
            assert!(
                leading_gap > trailing_gap + 12.0,
                "UnsavedChanges must separate Don't Save on the leading edge from [Cancel] [Save]: leading_gap={leading_gap}, trailing_gap={trailing_gap}"
            );
        } else if name == "About" {
            assert!(driver.frame_meta().find_by_widget_id("dialog.button.ok").is_none(), "About panel must omit blocking OK button");
        }
        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "{name} parity violations: {:?}", parity.violations);
    }

    // Additional trailing-primary modals: Preferences, ExportFigure, ImportProjectMode, ExportAllTablesExistingFiles.
    let mut app_prefs = seed_xy_app();
    app_prefs.egui_views.app_modals.open_preferences(&app_prefs.shortcuts.clone(), None);
    let mut app_export = seed_xy_app();
    app_export.canvas_dialogs.active_dialog = ActiveDialog::ExportFigure { sheet_id: SheetId::from("gs-xy") };
    let mut app_ipm = seed_xy_app();
    app_ipm.canvas_dialogs.active_dialog = ActiveDialog::ImportProjectMode {
        command_id: String::from("table.importPrism"),
        path: String::from("assay.prism"),
        has_unsaved_changes: false,
    };
    let mut app_eef = seed_xy_app();
    app_eef.canvas_dialogs.active_dialog =
        ActiveDialog::ExportAllTablesExistingFiles { dir: String::from("/exports"), existing_files: vec![String::from("data.csv")] };

    for (name, app, ordered) in [
        ("Preferences", app_prefs, &["modal.cancel", "modal.ok"][..]),
        ("ExportFigure", app_export, &["modal.cancel", "modal.ok"][..]),
        ("ImportProjectMode", app_ipm, &["import_project_mode.replace", "modal.cancel", "import_project_mode.merge"][..]),
        ("ExportExistingFiles", app_eef, &["export_existing_files.replace", "modal.cancel", "export_existing_files.keep_both"][..]),
    ] {
        let driver = make_driver(app);
        let lefts: Vec<f32> = ordered.iter().map(|w| driver.frame_meta().find_by_widget_id(w).expect(w).clipped_rect.left()).collect();
        for pair in lefts.windows(2) {
            assert!(pair[0] < pair[1], "{name} trailing footer order mismatch: {lefts:?}");
        }
        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "{name} parity violations: {:?}", parity.violations);
    }
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
        ("modal_dark_1280x800.png", UiThemeMode::Dark, 1280, 800, Some(ActiveDialog::Checkpoints)),
    ];

    let mut rendered: Vec<(&str, Vec<u8>)> = Vec::new();
    for (filename, mode, width_px, height_px, dialog) in cases {
        let mut app = seed_xy_app();
        app.theme = UiThemeSpec::for_mode(mode);
        let has_dialog = dialog.is_some();
        if let Some(dlg) = dialog {
            app.canvas_dialogs.active_dialog = dlg;
        }
        app.sync_views();
        let (png_bytes, rw, rh) =
            app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px, height_px, dpi: 96 }, ScreenshotRegionTarget::FullWindow).unwrap();
        assert!(png_bytes.starts_with(PNG_SIGNATURE) && (rw, rh) == (width_px, height_px));
        if has_dialog {
            let (dlg_bytes, dw, dh) =
                app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px, height_px, dpi: 96 }, ScreenshotRegionTarget::Dialog).unwrap();
            assert!(dlg_bytes.starts_with(PNG_SIGNATURE) && dw > 100 && dh > 100 && dw < width_px && dh < height_px);
        }
        if let Some(dir) = &out_dir {
            std::fs::write(dir.join(filename), &png_bytes).unwrap();
        }
        rendered.push((filename, png_bytes));
    }
    let find = |name: &str| rendered.iter().find(|(n, _)| *n == name).map(|(_, b)| b.as_slice()).unwrap();
    assert_ne!(find("modal_light_1280x800.png"), find("light_1280x800.png"));
    assert_ne!(find("modal_dark_1280x800.png"), find("dark_1280x800.png"));
}

#[test]
fn da_05_02_dark_hc_selected_accent_text_and_accent_border_contrast() {
    for mode in UiThemeMode::ALL {
        let t = Tokens::for_mode(mode);
        assert!(t.contrast_violations().is_empty(), "{mode:?} contrast_violations: {:?}", t.contrast_violations());
        assert!(wcag_contrast_ratio(color32_to_rgba8(t.selected_accent_text()), color32_to_rgba8(t.active_bg)) >= 4.5);
        assert!(wcag_contrast_ratio(color32_to_rgba8(t.accent), color32_to_rgba8(t.active_bg)) >= 3.0);
        for (name, bg) in [("surface_bg", t.surface_bg), ("panel_bg", t.panel_bg), ("button_bg", t.button_bg), ("header_bg", t.header_bg)] {
            assert!(wcag_contrast_ratio(color32_to_rgba8(t.accent), color32_to_rgba8(bg)) >= 4.5, "{mode:?} accent on {name}");
        }
    }
}

#[test]
fn r20_14_excluded_cell_role_num_and_text_link_tokens_across_themes() {
    for mode in UiThemeMode::ALL {
        let spec = UiThemeSpec::for_mode(mode);
        let t = Tokens::for_mode(mode);
        assert!(color32_to_rgba8(t.excluded_cell_text).to_oklch().c <= 0.02, "{mode:?} excluded_cell_text must be neutral slate");
        assert!(color32_to_rgba8(t.role_num).to_oklch().c <= 0.02, "{mode:?} role_num must be neutral");
        assert_eq!(t.text_link, rgba8_to_color32(spec.accent));
        assert_ne!(t.text_link, t.text_primary);
        for (lbl, bg) in [("surface_bg", t.surface_bg), ("panel_bg", t.panel_bg), ("active_bg", t.active_bg)] {
            let ratio = wcag_contrast_ratio(color32_to_rgba8(t.excluded_cell_text), color32_to_rgba8(bg));
            assert!(ratio >= 4.5, "{mode:?} excluded_cell_text on {lbl}: {ratio:.2} < 4.5");
        }
        assert!(wcag_contrast_ratio(color32_to_rgba8(t.role_num), color32_to_rgba8(t.surface_recessed)) >= 4.5);
    }
    let hc = Tokens::for_mode(UiThemeMode::HighContrast);
    assert!(wcag_contrast_ratio(color32_to_rgba8(hc.excluded_cell_text), color32_to_rgba8(hc.surface_bg)) >= 7.0);
    assert!(wcag_contrast_ratio(color32_to_rgba8(hc.excluded_cell_text), color32_to_rgba8(hc.panel_bg)) >= 7.0);
}

#[test]
fn r20_14_clickable_links_use_link_button_and_open_destinations() {
    // 1. Search dropdown footer ("Open full search") exposes link role & >= 24 pt height and opens palette.
    let mut driver = make_driver(seed_xy_app());
    driver.click_widget_id("toolbar.search_input").expect("focus search");
    let open_pal = driver.frame_meta().find_by_widget_id("toolbar.search_open_palette").expect("toolbar.search_open_palette");
    assert_eq!(open_pal.role, "link");
    assert!(open_pal.unclipped_rect.height() >= 24.0);
    driver.click_widget_id("toolbar.search_open_palette").expect("click search_open_palette");
    assert!(driver.frame_meta().find_by_widget_id("palette.dialog").is_some(), "expected command palette dialog to open");

    // 2. Fit Constraints "Linked hook: ..." uses ui_kit::link_button and opens inline Hook Constant browser.
    let mut doc = seed_xy_app().document().clone();
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata").unwrap();
    info.set_constant("VehicleMass", ConstantValue::Number(12.5), Some(String::from("mg"))).unwrap();
    doc.add_info_sheet(info).unwrap();

    let fit_id = SheetId::from("res-fit-hook");
    let mut nl = curvebook_doc::NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.parameter_rules = vec![ParameterRule {
        name: String::from("Bottom"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::from("info-1")),
            constant_name: String::from("VehicleMass"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }];
    let mut fit_spec = AnalysisSpec::new("4PL Fit", SheetId::from("ds-xy"), AnalysisKind::NonlinearRegression(nl)).unwrap();
    fit_spec.id = fit_id.clone();
    doc.add_result_sheet(ResultSheet::new(fit_spec).unwrap()).unwrap();

    let mut fit_app = UiApp::from_session(Session::from_document(doc));
    let snap = fit_app.document().clone();
    assert!(fit_app.canvas_dialogs.open_edit_parameters(&snap, &fit_id));
    fit_app.sync_views();
    let mut fit_drv = make_driver(fit_app);
    fit_drv.click_widget_id("fit.tab.constraints").expect("switch to constraints tab");
    let linked_hook = fit_drv.frame_meta().find_by_widget_id("fit.con.linked_hook.Bottom").expect("fit.con.linked_hook.Bottom link");
    assert_eq!(linked_hook.role, "link");
    assert!(linked_hook.unclipped_rect.height() >= 24.0);
    fit_drv.click_widget_id("fit.con.linked_hook.Bottom").expect("click linked hook");
    assert!(
        fit_drv.frame_meta().entries.iter().any(|e| e.widget_id.starts_with("fit.con.hook_opt.Bottom.")),
        "clicking Linked hook link must open the inline Hook Constant browser"
    );
}

#[test]
fn r20_21_cell_exclusion_tooltip_uses_show_tooltip_card_delay_and_dismissal() {
    let mut doc = seed_xy_app().document().clone();
    {
        let ds = doc.data_sheet_mut(&SheetId::from("ds-xy")).unwrap();
        ds.set_excluded_with_reason(CellCoord::new(1, 0, 0), "rout_outlier", "Contaminated well", "2026-11-27T00:00:00Z").unwrap();
        ds.plate_well_origins.push(PlateCellOrigin { coord: CellCoord::new(1, 0, 0), well: String::from("B03") });
    }
    doc.select_in_navigator(&SheetId::from("ds-xy")).unwrap();
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    let mut driver = make_driver(app);

    let cell_center = driver.frame_meta().find_by_widget_id("grid.cell.1.0.0").expect("grid.cell.1.0.0").clipped_rect.center();
    driver.dispatch_events(vec![egui::Event::PointerMoved(cell_center)]);
    assert!(
        !driver.visible_tooltip_texts().iter().any(|t| t.contains("Contaminated well")),
        "cell exclusion tooltip must respect 450ms delay before opening"
    );

    driver.hover_widget_id("grid.cell.1.0.0").expect("hover excluded cell");
    let tips = driver.visible_tooltip_texts();
    assert!(tips.iter().any(|t| t.contains("rout_outlier") && t.contains("Contaminated well")), "missing Band 1 exclusion note: {tips:?}");
    assert!(tips.iter().any(|t| t.contains("Source well: B03")), "missing Band 2 plate well summary: {tips:?}");

    driver.wheel_at(cell_center, egui::Vec2::new(0.0, -24.0)).expect("scroll dismisses tooltip");
    assert!(
        !driver.visible_tooltip_texts().iter().any(|t| t.contains("Contaminated well")),
        "cell exclusion tooltip must dismiss immediately on scroll"
    );

    driver.hover_widget_id("grid.cell.1.0.0").expect("re-hover excluded cell");
    assert!(driver.visible_tooltip_texts().iter().any(|t| t.contains("Contaminated well")));
    driver.dispatch_events(vec![egui::Event::PointerButton {
        pos: cell_center,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(
        !driver.visible_tooltip_texts().iter().any(|t| t.contains("Contaminated well")),
        "cell exclusion tooltip must dismiss immediately on pointer press"
    );
}
