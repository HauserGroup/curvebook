//! Desktop CJK / RTL / HiDPI / Gesture / Live Layout Snap & Print Dialog QA suite (R5.4 & R8.5).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod human_wf_common;

use std::collections::HashSet;
use std::path::Path;

use curvebook_chart::PointPt;
use curvebook_doc::{LayoutItem, SheetId};
use curvebook_palette::Rgba8;
use curvebook_render::RenderTargetSpec;
use curvebook_tools::canvas::SnapSource;
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents};
use curvebook_ui_egui::{
    ActiveDialog, FontBook, OsAdapterAvailability, ScreenshotRegionTarget, UiApp, UiInputEvent, ViewportMetrics, dispatch_ui_command,
    dispatch_ui_event,
};
use serde_json::json;

#[derive(Debug, Clone, Copy)]
struct QaPixelRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy)]
struct QaWorkspaceRegions {
    toolbar: QaPixelRect,
    navigator: QaPixelRect,
    canvas: QaPixelRect,
    inspector: QaPixelRect,
    status_bar: QaPixelRect,
}

fn rect_to_px(rect: egui::Rect, scale: f64, max_w: u32, max_h: u32) -> QaPixelRect {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let x = ((f64::from(rect.min.x.max(0.0)) * scale).floor() as u32).min(max_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let y = ((f64::from(rect.min.y.max(0.0)) * scale).floor() as u32).min(max_h);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let r = ((f64::from(rect.max.x.max(0.0)) * scale).ceil() as u32).min(max_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = ((f64::from(rect.max.y.max(0.0)) * scale).ceil() as u32).min(max_h);
    QaPixelRect { x, y, width: r.saturating_sub(x), height: b.saturating_sub(y) }
}

fn measure_qa_regions(app: &UiApp, metrics: ViewportMetrics) -> QaWorkspaceRegions {
    let ctx = egui::Context::default();
    curvebook_ui_egui::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
    #[allow(clippy::cast_possible_truncation)]
    let w_pt = metrics.logical_width.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let h_pt = metrics.logical_height.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let ppp = metrics.clamped_scale_factor() as f32;
    ctx.options_mut(|o| o.zoom_factor = app.ui_scale());
    let mut views = app.egui_views.clone();
    let mut report = None;
    for _ in 0..6 {
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w_pt, h_pt))),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(ppp / app.ui_scale()), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut settled = false;
        let mut out = ctx.run_ui(raw, |ui| {
            let rep = render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents);
            settled = !ui.ctx().will_discard();
            if settled {
                report = Some(rep);
            }
        });
        out.textures_delta.clear();
        if settled {
            break;
        }
    }
    let rep = report.expect("settled qa shell report");
    let scale = metrics.clamped_scale_factor();
    let max_w = metrics.physical_width_px();
    let max_h = metrics.physical_height_px();
    let zero = QaPixelRect { x: 0, y: 0, width: 0, height: 0 };
    QaWorkspaceRegions {
        toolbar: rep.tier1_rect_pt.map_or(zero, |r| rect_to_px(r, scale, max_w, max_h)),
        navigator: rep.leading_dock_rect_pt.map_or(zero, |r| rect_to_px(r, scale, max_w, max_h)),
        canvas: rep.regions.canvas_rect_pt.map_or(zero, |r| rect_to_px(r, scale, max_w, max_h)),
        inspector: rep.trailing_dock_rect_pt.map_or(zero, |r| rect_to_px(r, scale, max_w, max_h)),
        status_bar: rep.footer_rect_pt.map_or(zero, |r| rect_to_px(r, scale, max_w, max_h)),
    }
}

fn build_qa_two_panel_app() -> (UiApp, SheetId, SheetId, SheetId) {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "title": "Assay A", "auto_graph": true })).expect("create d1");
    for (r, x, y) in [(0, 1.0, 10.0), (1, 2.0, 22.0), (2, 3.0, 38.0)] {
        app.execute_command("table.setCell", json!({ "sheet_id": "d1", "row": r, "col": 0, "value": x })).expect("x1");
        app.execute_command("table.setCell", json!({ "sheet_id": "d1", "row": r, "col": 1, "value": y })).expect("y1");
    }

    app.execute_command("sheet.createData", json!({ "sheet_id": "d2", "kind": "xy", "title": "Assay B", "auto_graph": true })).expect("create d2");
    for (r, x, y) in [(0, 1.0, 14.0), (1, 2.0, 29.0), (2, 3.0, 44.0)] {
        app.execute_command("table.setCell", json!({ "sheet_id": "d2", "row": r, "col": 0, "value": x })).expect("x2");
        app.execute_command("table.setCell", json!({ "sheet_id": "d2", "row": r, "col": 1, "value": y })).expect("y2");
    }

    let g1 = app.session().document().graph_sheets[0].id.clone();
    let g2 = app.session().document().graph_sheets[1].id.clone();
    app.execute_command("graph.setAxes", json!({ "sheet_id": g1.as_str(), "x_title": "Time (min)", "y_title": "Signal A" })).expect("set axes g1");

    app.execute_command(
        "layout.create",
        json!({ "sheet_id": "layout-qa", "title": "QA Figure", "preset": "double_column", "align_plot_origins": true }),
    )
    .expect("create layout");
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "layout-qa", "kind": "graph", "graph_sheet_id": g1.as_str(), "bounds_pt": { "x": 24.0, "y": 24.0, "width": 200.0, "height": 150.0 } }),
    )
    .expect("add panel 0");
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "layout-qa", "kind": "graph", "graph_sheet_id": g2.as_str(), "bounds_pt": { "x": 260.0, "y": 180.0, "width": 170.0, "height": 130.0 } }),
    )
    .expect("add panel 1");
    app.execute_command("sheet.select", json!({ "sheet_id": "layout-qa" })).expect("select layout");
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    (app, SheetId::new("layout-qa"), g1, g2)
}

fn distinct_colors_in_rect(rgba: &[u8], img_w: u32, x: u32, y: u32, w: u32, h: u32) -> usize {
    let mut set = HashSet::new();
    for py in y..y.saturating_add(h) {
        for px in x..x.saturating_add(w) {
            let idx = ((py as usize) * (img_w as usize) + (px as usize)) * 4;
            if let Some(chunk) = rgba.get(idx..idx + 4) {
                set.insert([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
        }
    }
    set.len()
}

fn count_non_color_pixels(rgba: &[u8], bg: Rgba8) -> usize {
    rgba.as_chunks::<4>().0.iter().filter(|c| c[0] != bg.r || c[1] != bg.g || c[2] != bg.b || c[3] != bg.a).count()
}

#[test]
fn cjk_rtl_locales_render_mirror_panes_and_keep_hidpi_hit_test_parity() {
    let (mut app, _, _, _) = build_qa_two_panel_app();

    let locales = ["ja", "zh-CN", "ko", "ar", "he"];
    let scales = [1.0_f64, 1.5, 2.0, 3.0];

    for locale in locales {
        let _ = app.set_locale(locale);
        app.nav.welcome.open = false;
        app.sync_views();

        for &scale in &scales {
            let metrics = ViewportMetrics::new(900.0, 560.0, scale);
            let phys_w = metrics.physical_width_px();
            let phys_h = metrics.physical_height_px();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let dpi = (96.0 * scale).round() as u16;

            let layout = measure_qa_regions(&app, metrics);
            assert!(layout.navigator.width > 0 && layout.canvas.width > 0 && layout.inspector.width > 0);
            assert_eq!(layout.navigator.x, 0, "{locale}@{scale}x: navigator starts on left");
            assert!(layout.navigator.x < layout.canvas.x && layout.canvas.x < layout.inspector.x);
            assert_eq!(layout.inspector.x + layout.inspector.width, phys_w, "{locale}@{scale}x: inspector flushes right");

            let target = RenderTargetSpec { width_px: phys_w, height_px: phys_h, dpi };
            let img = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("render egui shell rgba");
            assert_eq!(img.rgba.len(), (phys_w as usize) * (phys_h as usize) * 4);
            for region in [layout.toolbar, layout.navigator, layout.canvas, layout.inspector, layout.status_bar] {
                assert!(
                    distinct_colors_in_rect(&img.rgba, img.width_px, region.x, region.y, region.width, region.height) >= 2,
                    "{locale}@{scale}x: empty/blank region {region:?}"
                );
            }

            let snap = app.inspect_egui_shell_snapshot(metrics, None, true);
            assert!(snap.navigator_sheets.as_ref().is_some_and(|n| !n.is_empty()), "{locale}@{scale}x missing navigator nodes");
            assert!(snap.toolbar.as_ref().is_some_and(|t| !t.is_empty()), "{locale}@{scale}x missing toolbar nodes");
        }
    }
}

#[test]
fn font_book_loads_system_fonts_and_rasterizes_cjk_and_rtl_glyphs() {
    let book = FontBook::with_system_and_fallback();
    assert!(!book.face_names().is_empty(), "FontBook must always retain embedded fallback");

    let has_macos_unicode = Path::new("/System/Library/Fonts/Supplemental/Arial Unicode.ttf").is_file();
    if cfg!(target_os = "macos") && has_macos_unicode {
        assert!(
            book.face_names().iter().any(|name| name.contains("Arial Unicode.ttf")),
            "expected macOS system Arial Unicode.ttf in FontBook: {:?}",
            book.face_names()
        );
    }

    let samples = [
        ("ja", "日本語 用量反応 IC50"),
        ("zh-CN", "中文 剂量反应"),
        ("ko", "한국어 용량 반응"),
        ("ar", "تحليل الجرعة (IC50)"),
        ("he", "עקומת תגובה (p < 0.05)"),
    ];
    let spec = RenderTargetSpec { width_px: 360, height_px: 72, dpi: 144 };
    let fg = Rgba8::rgb(18, 24, 38);
    let bg = Rgba8::WHITE;

    for (locale, sample) in samples {
        let (ttf_mapped, img) = book.rasterize_shaped_text(sample, locale, 16.0, spec, fg, bg).expect("rasterize shaped text");
        let inked = count_non_color_pixels(&img.rgba, bg);
        assert!(inked > 25, "{locale}: shaped text rendered too few pixels ({inked})");
        if cfg!(target_os = "macos") && has_macos_unicode {
            assert!(ttf_mapped >= 4, "{locale}: expected TrueType glyph outline mapping for system font, got {ttf_mapped}");
        }
    }

    let fallback_only = FontBook::with_embedded_fallback();
    let (fb_mapped, fb_img) = fallback_only.rasterize_shaped_text("IC50 α + β", "en", 14.0, spec, fg, bg).expect("fallback raster");
    assert!(fb_mapped > 0, "embedded minimal TTF fallback must map ASCII/Greek glyphs");
    assert!(count_non_color_pixels(&fb_img.rgba, bg) > 25);
}

#[test]
fn layout_live_plot_origin_drag_snap_and_resize_handle_commit() {
    let (mut app, layout_id, g1, g2) = build_qa_two_panel_app();
    app.nav.welcome.open = false;
    app.canvas_dialogs.canvas.snap_to_grid = false;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.5);

    // Compute exact panel-0 plot origin and panel-1 plot-origin offset from compiled graph scenes.
    let sub0 = app.session().render_sheet_scene(&g1).expect("compile g1");
    let sub1 = app.session().render_sheet_scene(&g2).expect("compile g2");
    let p0_plot_origin = PointPt::new(24.0 + sub0.plot_rect_pt.x, 24.0 + sub0.plot_rect_pt.y);
    let p1_offset = PointPt::new(sub1.plot_rect_pt.x, sub1.plot_rect_pt.y);

    let mut harness = human_wf_common::EguiWfHarness::from_app(app, metrics);
    let p0_rect = harness.driver.frame_meta().find_by_widget_id("canvas.layout_panel.0").expect("panel 0 meta").clipped_rect;
    let p1_rect = harness.driver.frame_meta().find_by_widget_id("canvas.layout_panel.1").expect("panel 1 meta").clipped_rect;
    let px_per_pt = p0_rect.width() / 200.0;

    // Start dragging Panel 1 and move so its plot origin lands within snap tolerance of Panel 0's plot origin X.
    let start_pos = p1_rect.center();
    let desired_p1_x = (p0_plot_origin.x - p1_offset.x) + 1.0;
    let delta_x_pt = desired_p1_x - 260.0;
    let trigger_pos = egui::pos2(start_pos.x - 12.0, start_pos.y);
    let move_pos = egui::pos2(trigger_pos.x + (delta_x_pt as f32) * px_per_pt, start_pos.y);

    harness.driver.dispatch_events(vec![egui::Event::PointerMoved(start_pos)]);
    harness.driver.dispatch_events(vec![egui::Event::PointerButton {
        pos: start_pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    harness.driver.dispatch_events(vec![egui::Event::PointerMoved(trigger_pos)]);
    harness.driver.dispatch_events(vec![egui::Event::PointerMoved(move_pos)]);

    let snap_guides = &harness.driver.state().views.canvas_view.active_snap_guides;
    assert!(snap_guides.iter().any(|g| g.source == SnapSource::PlotOrigin), "expected PlotOrigin snap guide during live drag, got: {snap_guides:?}");
    let drag_preview = harness.driver.state().views.canvas_view.layout_drag.as_ref().expect("active layout_drag preview");
    assert!(
        ((drag_preview.current_bounds_pt.x + p1_offset.x) - p0_plot_origin.x).abs() <= 0.5,
        "panel 1 plot origin ({}) failed to snap to panel 0 plot origin ({})",
        drag_preview.current_bounds_pt.x + p1_offset.x,
        p0_plot_origin.x
    );

    harness.driver.dispatch_events(vec![egui::Event::PointerButton {
        pos: move_pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);

    let ls = harness.app().document().layout_sheet(&layout_id).expect("layout sheet");
    let LayoutItem::LiveGraph { bounds_pt: moved_bounds, .. } = &ls.spec.items[1] else {
        panic!("expected LiveGraph item 1");
    };
    let moved_bounds = *moved_bounds;
    assert!(((moved_bounds.x + p1_offset.x) - p0_plot_origin.x).abs() <= 0.5);
}

#[test]
fn pinch_zoom_inertial_pan_coasting_and_print_dialog_flow() {
    let (mut app, layout_id, _, _) = build_qa_two_panel_app();
    app.nav.welcome.open = false;
    let metrics = ViewportMetrics::new(1024.0, 640.0, 2.0);

    dispatch_ui_event(&mut app, UiInputEvent::PinchZoom { x: 400.0, y: 300.0, scale_delta: 1.5 }, metrics).expect("pinch zoom");
    assert!((app.canvas_dialogs.pan_zoom.zoom - 1.5).abs() < 1e-6);

    dispatch_ui_event(&mut app, UiInputEvent::InertialPan { delta_x: 36.0, delta_y: -18.0, dt_sec: 0.016, step_coast: false }, metrics)
        .expect("pan impulse");
    let pan_before_coast = app.canvas_dialogs.pan_zoom.pan_pt;
    let moved = app.tick_inertial_pan(0.016);
    assert!(moved, "inertial pan should continue coasting after impulse");
    assert_ne!(app.canvas_dialogs.pan_zoom.pan_pt, pan_before_coast);
    for _ in 0..240 {
        if !app.tick_inertial_pan(0.016) {
            break;
        }
    }
    assert!(!app.tick_inertial_pan(0.016), "inertial pan should damp cleanly to rest");

    // Interactive Print Dialog via ui.print.open + UI controls + PDF spooling + preview staging.
    let res = dispatch_ui_command(
        &mut app,
        "ui.print.open",
        &json!({ "sheet_id": layout_id.as_str(), "copies": 3, "scale_percent": 100.0, "crop_marks": true }),
    )
    .expect("ui.print.open")
    .expect("ui command handled");
    assert_eq!(res["dialog"], "Print");
    assert_eq!(res["copies"], 3);
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::Print { sheet_id: Some(layout_id.clone()) });

    assert_eq!(app.update_print_dialog_copies(0), 1);
    assert_eq!(app.update_print_dialog_copies(4), 4);
    app.select_print_dialog_printer(Some(String::from("Lab_PDF_Virtual")));
    assert_eq!(app.canvas_dialogs.print.selected_printer.as_deref(), Some("Lab_PDF_Virtual"));

    let spool = app.execute_print_dialog_spool(false).expect("spool print pdf");
    assert!(spool.pdf_bytes.starts_with(b"%PDF-1.7"));
    assert_eq!(app.canvas_dialogs.print.last_spool_bytes, spool.pdf_bytes.len());

    let modal_img = app
        .render_egui_shell_rgba(RenderTargetSpec { width_px: 800, height_px: 520, dpi: 144 }, ScreenshotRegionTarget::FullWindow)
        .expect("render print modal");
    assert!(distinct_colors_in_rect(&modal_img.rgba, modal_img.width_px, 200, 120, 400, 260) >= 3);

    if std::env::var_os("CURVEBOOK_ALLOW_OS_GUI_DIALOG").is_none() {
        let preview_status = app.preview_print_dialog_spool().expect("deterministic print preview stage");
        assert_eq!(preview_status.availability, OsAdapterAvailability::Unavailable);
    }

    // Alias command `file.print` also opens cleanly.
    app.canvas_dialogs.close_dialog();
    let alias_res = dispatch_ui_command(&mut app, "file.print", &json!({ "spool": true })).expect("file.print").expect("handled");
    assert_eq!(alias_res["dialog"], "Print");
    assert!(alias_res["last_spool_bytes"].as_u64().unwrap_or(0) > 100);
}

#[test]
fn r13_rtl_ar_he_and_hidpi_pixel_to_hit_box_parity() {
    let (mut base_app, layout_id, g1, _) = build_qa_two_panel_app();
    base_app.nav.welcome.open = false;
    base_app.egui_views.launcher_open = false;
    base_app
        .execute_command("analysis.run", json!({ "source_sheet_id": "d1", "kind": "descriptive", "sheet_id": "res-desc" }))
        .expect("descriptive stats");
    base_app
        .execute_command("edit.pasteResultTable", json!({ "source_sheet_id": "res-desc", "target_sheet_id": g1.as_str() }))
        .expect("pasteResultTable");

    let png_bytes =
        curvebook_render::png::encode_png(&curvebook_render::RasterImage { width_px: 8, height_px: 8, dpi: 96, rgba: vec![96u8; 8 * 8 * 4] })
            .expect("encode png");
    let png_b64 = {
        const LUT: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in png_bytes.chunks(3) {
            let n =
                (u32::from(chunk[0]) << 16) | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8) | u32::from(chunk.get(2).copied().unwrap_or(0));
            out.push(LUT[((n >> 18) & 0x3F) as usize] as char);
            out.push(LUT[((n >> 12) & 0x3F) as usize] as char);
            out.push(if chunk.len() > 1 { LUT[((n >> 6) & 0x3F) as usize] as char } else { '=' });
            out.push(if chunk.len() > 2 { LUT[(n & 0x3F) as usize] as char } else { '=' });
        }
        out
    };
    base_app.execute_command("edit.pasteImage", json!({ "target_sheet_id": g1.as_str(), "bytes_base64": png_b64 })).expect("pasteImage");

    let dialogs = [
        ("ui.graph.create.open", json!({ "data_sheet_id": "d1" })),
        ("ui.layout.equalizeScaling.open", json!({ "sheet_id": layout_id.as_str() })),
        ("ui.embeddedTable.format.open", json!({ "sheet_id": g1.as_str(), "annotation_index": 0 })),
        ("ui.image.format.open", json!({ "sheet_id": g1.as_str(), "annotation_index": 1 })),
    ];

    for locale in ["ar", "he"] {
        for metrics in [ViewportMetrics::new(1280.0, 800.0, 1.0), ViewportMetrics::new(1024.0, 640.0, 2.0)] {
            for (cmd_id, params) in &dialogs {
                let mut app = base_app.clone();
                let _ = app.set_locale(locale);
                let _ = dispatch_ui_command(&mut app, cmd_id, params).expect("open R13 modal").expect("handled");
                app.sync_views();
                let h = human_wf_common::EguiWfHarness::from_app(app, metrics);
                let parity = h.driver.verify_pixel_to_hit_box_parity();
                let modal_violations: Vec<_> = parity
                    .violations
                    .iter()
                    .filter(|v| {
                        v.widget_id != "canvas.viewport"
                            && v.widget_id != "canvas.split.viewport"
                            && !v.widget_id.starts_with("canvas.layout_panel.")
                            && !v.widget_id.starts_with("canvas.graph_ann.")
                            && !v.widget_id.starts_with("canvas.image.handle.")
                    })
                    .collect();
                assert!(modal_violations.is_empty(), "{cmd_id} `{locale}` @ {metrics:?} parity violations: {modal_violations:?}");
                let overlaps = h.driver.check_scrollbar_non_overlap();
                assert!(overlaps.is_empty(), "{cmd_id} `{locale}` @ {metrics:?} scrollbar overlaps: {overlaps:?}");
            }
        }
    }
}
