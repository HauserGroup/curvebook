//! Desktop CJK / RTL / HiDPI / Gesture / Live Layout Snap & Print Dialog QA suite (R5.4 & R8.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use curvebook_chart::PointPt;
use curvebook_doc::{LayoutItem, SheetId};
use curvebook_palette::Rgba8;
use curvebook_render::RenderTargetSpec;
use curvebook_tools::canvas::SnapSource;
use curvebook_ui_egui::{
    ActiveDialog, FontBook, OsAdapterAvailability, ScreenshotRegionTarget, UiApp, UiInputEvent, ViewportMetrics, dispatch_ui_command,
    dispatch_ui_event,
};
use serde_json::json;

use crate::common::{PixelRect, encode_base64, measure_egui_regions, region_distinct_colors};

fn build_qa_two_panel_app() -> (UiApp, SheetId, SheetId, SheetId) {
    let mut app = UiApp::new();
    for (sid, title, rows) in [("d1", "Assay A", [(1.0, 10.0), (2.0, 22.0), (3.0, 38.0)]), ("d2", "Assay B", [(1.0, 14.0), (2.0, 29.0), (3.0, 44.0)])]
    {
        app.execute_command("sheet.createData", json!({ "sheet_id": sid, "kind": "xy", "title": title, "auto_graph": true })).unwrap();
        for (r, (x, y)) in rows.into_iter().enumerate() {
            app.execute_command("table.setCell", json!({ "sheet_id": sid, "row": r, "col": 0, "value": x })).unwrap();
            app.execute_command("table.setCell", json!({ "sheet_id": sid, "row": r, "col": 1, "value": y })).unwrap();
        }
    }
    let (g1, g2) = (app.session().document().graph_sheets[0].id.clone(), app.session().document().graph_sheets[1].id.clone());
    app.execute_command("graph.setAxes", json!({ "sheet_id": g1.as_str(), "x_axis": { "title": "Time (min)" }, "y_axis": { "title": "Signal A" } }))
        .unwrap();
    app.execute_command(
        "layout.create",
        json!({ "sheet_id": "layout-qa", "title": "QA Figure", "preset": "double_column", "align_plot_origins": true }),
    )
    .unwrap();
    for (gid, x, y, w, h) in [(g1.as_str(), 24.0, 24.0, 200.0, 150.0), (g2.as_str(), 260.0, 180.0, 170.0, 130.0)] {
        app.execute_command(
            "layout.addPanel",
            json!({ "sheet_id": "layout-qa", "kind": "graph", "graph_sheet_id": gid, "bounds_pt": { "x": x, "y": y, "width": w, "height": h } }),
        )
        .unwrap();
    }
    app.execute_command("sheet.select", json!({ "sheet_id": "layout-qa" })).unwrap();
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    (app, SheetId::new("layout-qa"), g1, g2)
}

fn count_non_color_pixels(rgba: &[u8], bg: Rgba8) -> usize {
    rgba.as_chunks::<4>().0.iter().filter(|c| c[0] != bg.r || c[1] != bg.g || c[2] != bg.b || c[3] != bg.a).count()
}

#[test]
fn cjk_rtl_locales_render_mirror_panes_and_keep_hidpi_hit_test_parity() {
    let (mut app, _, _, _) = build_qa_two_panel_app();
    for locale in ["ja", "zh-CN", "ko", "ar", "he"] {
        assert_eq!(app.set_locale(locale), locale);
        app.nav.welcome.open = false;
        app.sync_views();

        for scale in [1.0_f64, 1.5, 2.0, 3.0] {
            let metrics = ViewportMetrics::new(900.0, 560.0, scale);
            let (phys_w, phys_h) = (metrics.physical_width_px(), metrics.physical_height_px());
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let dpi = (96.0 * scale).round() as u16;

            let layout = measure_egui_regions(&app, metrics);
            assert!(layout.navigator.width > 0 && layout.canvas.width > 0 && layout.inspector.width > 0);
            if curvebook_ui_egui::i18n::is_rtl_locale(locale) {
                assert_eq!(layout.inspector.x, 0, "{locale}@{scale}x: RTL inspector must dock flush left");
                assert!(layout.inspector.x < layout.canvas.x && layout.canvas.x < layout.navigator.x, "{locale}@{scale}x: {layout:?}");
                assert_eq!(layout.navigator.x + layout.navigator.width, phys_w, "{locale}@{scale}x: RTL navigator must flush right");
            } else {
                assert_eq!(layout.navigator.x, 0, "{locale}@{scale}x: navigator starts left");
                assert!(layout.navigator.x < layout.canvas.x && layout.canvas.x < layout.inspector.x);
                assert_eq!(layout.inspector.x + layout.inspector.width, phys_w, "{locale}@{scale}x: inspector flushes right");
            }

            let img = app
                .render_egui_shell_rgba(RenderTargetSpec { width_px: phys_w, height_px: phys_h, dpi }, ScreenshotRegionTarget::FullWindow)
                .expect("render rgba");
            assert_eq!(img.rgba.len(), (phys_w as usize) * (phys_h as usize) * 4);
            for region in [layout.toolbar, layout.navigator, layout.canvas, layout.inspector, layout.status_bar] {
                assert!(region_distinct_colors(&img.rgba, img.width_px, region, 4) >= 2, "{locale}@{scale}x: blank region {region:?}");
            }

            let snap = app.inspect_egui_shell_snapshot(metrics, None, true);
            assert!(snap.navigator_sheets.as_ref().is_some_and(|n| !n.is_empty()), "{locale}@{scale}x missing navigator");
            assert!(snap.toolbar.as_ref().is_some_and(|t| !t.is_empty()), "{locale}@{scale}x missing toolbar");
        }
    }
}

#[test]
fn font_book_loads_system_fonts_and_rasterizes_cjk_and_rtl_glyphs() {
    let book = FontBook::with_system_and_fallback();
    assert!(!book.face_names().is_empty());
    let has_macos_unicode = Path::new("/System/Library/Fonts/Supplemental/Arial Unicode.ttf").is_file();
    if cfg!(target_os = "macos") && has_macos_unicode {
        assert!(book.face_names().iter().any(|n| n.contains("Arial Unicode.ttf")), "missing Arial Unicode.ttf: {:?}", book.face_names());
    }

    let spec = RenderTargetSpec { width_px: 360, height_px: 72, dpi: 144 };
    let (fg, bg) = (Rgba8::rgb(18, 24, 38), Rgba8::WHITE);
    for (locale, sample) in [
        ("ja", "日本語 用量反応 IC50"),
        ("zh-CN", "中文 剂量反应"),
        ("ko", "한국어 용량 반응"),
        ("ar", "تحليل الجرعة (IC50)"),
        ("he", "עקומת תגובה (p < 0.05)"),
    ] {
        let (ttf_mapped, img) = book.rasterize_shaped_text(sample, locale, 16.0, spec, fg, bg).expect("rasterize");
        assert!(count_non_color_pixels(&img.rgba, bg) > 25, "{locale}: too few inked pixels");
        if cfg!(target_os = "macos") && has_macos_unicode {
            assert!(ttf_mapped >= 4, "{locale}: expected TrueType mapping, got {ttf_mapped}");
        }
    }

    let (fb_mapped, fb_img) = FontBook::with_embedded_fallback().rasterize_shaped_text("IC50 α + β", "en", 14.0, spec, fg, bg).expect("fallback");
    assert!(fb_mapped > 0 && count_non_color_pixels(&fb_img.rgba, bg) > 25);
}

#[test]
fn layout_live_plot_origin_drag_snap_and_resize_handle_commit() {
    let (mut app, layout_id, g1, g2) = build_qa_two_panel_app();
    app.nav.welcome.open = false;
    app.canvas_dialogs.canvas.snap_to_grid = false;
    let sub0 = app.session().render_sheet_scene(&g1).expect("compile g1");
    let sub1 = app.session().render_sheet_scene(&g2).expect("compile g2");
    let p0_origin = PointPt::new(24.0 + sub0.plot_rect_pt.x, 24.0 + sub0.plot_rect_pt.y);
    let p1_offset = PointPt::new(sub1.plot_rect_pt.x, sub1.plot_rect_pt.y);

    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.5));
    let p0_rect = h.driver.frame_meta().find_by_widget_id("canvas.layout_panel.0").expect("panel 0").clipped_rect;
    let p1_rect = h.driver.frame_meta().find_by_widget_id("canvas.layout_panel.1").expect("panel 1").clipped_rect;
    let px_per_pt = p0_rect.width() / 200.0;

    let start_pos = p1_rect.center();
    let trigger_pos = egui::pos2(start_pos.x - 12.0, start_pos.y);
    let move_pos = egui::pos2(trigger_pos.x + (((p0_origin.x - p1_offset.x + 1.0) - 260.0) as f32) * px_per_pt, start_pos.y);

    for ev in [
        egui::Event::PointerMoved(start_pos),
        egui::Event::PointerButton { pos: start_pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerMoved(trigger_pos),
        egui::Event::PointerMoved(move_pos),
    ] {
        h.driver.dispatch_events(vec![ev]);
    }

    assert!(h.driver.state().views.canvas_view.active_snap_guides.iter().any(|g| g.source == SnapSource::PlotOrigin));
    let preview = h.driver.state().views.canvas_view.layout_drag.as_ref().expect("active layout_drag");
    assert!(((preview.current_bounds_pt.x + p1_offset.x) - p0_origin.x).abs() <= 0.5);

    h.driver.dispatch_events(vec![egui::Event::PointerButton {
        pos: move_pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    let LayoutItem::LiveGraph { bounds_pt, .. } = &h.app().document().layout_sheet(&layout_id).expect("layout").spec.items[1] else {
        panic!("expected LiveGraph item 1");
    };
    assert!(((bounds_pt.x + p1_offset.x) - p0_origin.x).abs() <= 0.5);
}

#[test]
fn pinch_zoom_inertial_pan_coasting_and_print_dialog_flow() {
    let (mut app, layout_id, _, _) = build_qa_two_panel_app();
    app.nav.welcome.open = false;
    let metrics = ViewportMetrics::new(1024.0, 640.0, 2.0);

    dispatch_ui_event(&mut app, UiInputEvent::PinchZoom { x: 400.0, y: 300.0, scale_delta: 1.5 }, metrics).unwrap();
    assert!((app.canvas_dialogs.pan_zoom.zoom - 1.5).abs() < 1e-6);

    dispatch_ui_event(&mut app, UiInputEvent::InertialPan { delta_x: 36.0, delta_y: -18.0, dt_sec: 0.016, step_coast: false }, metrics).unwrap();
    let pan_before = app.canvas_dialogs.pan_zoom.pan_pt;
    assert!(app.tick_inertial_pan(0.016) && app.canvas_dialogs.pan_zoom.pan_pt != pan_before);
    for _ in 0..240 {
        if !app.tick_inertial_pan(0.016) {
            break;
        }
    }
    assert!(!app.tick_inertial_pan(0.016));

    let res = dispatch_ui_command(
        &mut app,
        "ui.print.open",
        &json!({ "sheet_id": layout_id.as_str(), "copies": 3, "scale_percent": 100.0, "crop_marks": true }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (res["dialog"].as_str(), res["copies"].as_i64(), &app.canvas_dialogs.active_dialog),
        (Some("Print"), Some(3), &ActiveDialog::Print { sheet_id: Some(layout_id) })
    );
    assert_eq!((app.update_print_dialog_copies(0), app.update_print_dialog_copies(4)), (1, 4));
    app.select_print_dialog_printer(Some("Lab_PDF_Virtual".into()));
    assert_eq!(app.canvas_dialogs.print.selected_printer.as_deref(), Some("Lab_PDF_Virtual"));

    let spool = app.execute_print_dialog_spool(false).unwrap();
    assert!(spool.pdf_bytes.starts_with(b"%PDF-1.7") && app.canvas_dialogs.print.last_spool_bytes == spool.pdf_bytes.len());
    let modal_img =
        app.render_egui_shell_rgba(RenderTargetSpec { width_px: 800, height_px: 520, dpi: 144 }, ScreenshotRegionTarget::FullWindow).unwrap();
    assert!(region_distinct_colors(&modal_img.rgba, modal_img.width_px, PixelRect { x: 200, y: 120, width: 400, height: 260 }, 4) >= 3);

    if std::env::var_os("CURVEBOOK_ALLOW_OS_GUI_DIALOG").is_none() {
        assert_eq!(app.preview_print_dialog_spool().unwrap().availability, OsAdapterAvailability::Unavailable);
    }

    app.canvas_dialogs.close_dialog();
    let alias = dispatch_ui_command(&mut app, "file.print", &json!({ "spool": true })).unwrap().unwrap();
    assert_eq!(alias["dialog"], "Print");
    assert!(alias["last_spool_bytes"].as_u64().unwrap_or(0) > 100);
}

#[test]
fn r13_rtl_ar_he_and_hidpi_pixel_to_hit_box_parity() {
    let (mut base_app, layout_id, g1, _) = build_qa_two_panel_app();
    base_app.nav.welcome.open = false;
    base_app.egui_views.launcher_open = false;
    base_app.execute_command("analysis.run", json!({ "source_sheet_id": "d1", "kind": "descriptive", "sheet_id": "res-desc" })).unwrap();
    base_app.execute_command("edit.pasteResultTable", json!({ "source_sheet_id": "res-desc", "target_sheet_id": g1.as_str() })).unwrap();
    let png_bytes =
        curvebook_render::png::encode_png(&curvebook_render::RasterImage { width_px: 8, height_px: 8, dpi: 96, rgba: vec![96u8; 8 * 8 * 4] })
            .unwrap();
    base_app.execute_command("edit.pasteImage", json!({ "target_sheet_id": g1.as_str(), "bytes_base64": encode_base64(&png_bytes) })).unwrap();

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
                assert_eq!(app.set_locale(locale), locale);
                let _ = dispatch_ui_command(&mut app, cmd_id, params).unwrap().unwrap();
                app.sync_views();
                let h = crate::human_wf_common::EguiWfHarness::from_app(app, metrics);
                let violations: Vec<_> = h
                    .driver
                    .verify_pixel_to_hit_box_parity()
                    .violations
                    .into_iter()
                    .filter(|v| {
                        v.widget_id != "canvas.viewport"
                            && v.widget_id != "canvas.split.viewport"
                            && !v.widget_id.starts_with("canvas.layout_panel.")
                            && !v.widget_id.starts_with("canvas.graph_ann.")
                            && !v.widget_id.starts_with("canvas.image.handle.")
                    })
                    .collect();
                assert!(violations.is_empty(), "{cmd_id} `{locale}` @ {metrics:?}: {violations:?}");
                assert!(h.driver.check_scrollbar_non_overlap().is_empty(), "{cmd_id} `{locale}` scrollbar overlap");
            }
        }
    }
}
