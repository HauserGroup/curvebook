//! Desktop CJK / RTL / HiDPI / Gesture / Live Layout Snap & Print Dialog QA suite (R5.4 & R8.5).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::Path;

use curvebook_chart::PointPt;
use curvebook_doc::{LayoutItem, SheetId};
use curvebook_palette::Rgba8;
use curvebook_render::RenderTargetSpec;
use curvebook_tools::canvas::SnapSource;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{
    ActiveDialog, CanvasDragTarget, FontBook, KeyModifiers, LayoutResizeHandle, OsAdapterAvailability, PointerButton, UiApp, UiInputEvent,
    ViewportMetrics, WorkspaceHitTarget, compute_workspace_layout_map, dispatch_ui_command, dispatch_ui_event, render_workspace_rgba,
};
use serde_json::json;

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
        let is_rtl = matches!(locale, "ar" | "he");
        app.locale = String::from(locale);
        app.sync_views();

        for &scale in &scales {
            let metrics = ViewportMetrics::new(900.0, 560.0, scale);
            let phys_w = metrics.physical_width_px();
            let phys_h = metrics.physical_height_px();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let dpi = (96.0 * scale).round() as u16;

            let map = compute_workspace_layout_map(&app, metrics);
            let layout = &map.frame;

            if is_rtl {
                assert_eq!(layout.inspector.x, 0, "{locale}@{scale}x: inspector must mirror to left");
                assert!(layout.canvas.x >= layout.inspector.right());
                assert!(layout.navigator.x >= layout.canvas.right());
                assert_eq!(layout.navigator.right(), phys_w, "{locale}@{scale}x: navigator must flush right");
            } else {
                assert_eq!(layout.navigator.x, 0, "{locale}@{scale}x: navigator starts on left");
                assert!(layout.canvas.x >= layout.navigator.right());
                assert!(layout.inspector.x >= layout.canvas.right());
                assert_eq!(layout.inspector.right(), phys_w, "{locale}@{scale}x: inspector flushes right");
            }

            let target = RenderTargetSpec { width_px: phys_w, height_px: phys_h, dpi };
            let img = render_workspace_rgba(&app, target).expect("render workspace rgba");
            assert_eq!(img.rgba.len(), (phys_w as usize) * (phys_h as usize) * 4);
            for region in [layout.toolbar, layout.navigator, layout.canvas, layout.inspector, layout.status_bar] {
                assert!(
                    distinct_colors_in_rect(&img.rgba, img.width_px, region.x, region.y, region.width, region.height) >= 2,
                    "{locale}@{scale}x: empty/blank region {region:?}"
                );
            }

            // Verify logical<->physical HiDPI hit-test parity on both mirrored inspector and navigator.
            let insp_phys = [f64::from(layout.inspector.x) + f64::from(layout.inspector.width) * 0.5, f64::from(layout.inspector.y) + 60.0];
            let insp_logical = metrics.physical_to_logical_px(insp_phys);
            let insp_hit = map.hit_test_logical(insp_logical[0], insp_logical[1]);
            assert_eq!(insp_hit, WorkspaceHitTarget::Pane(ActiveWorkspacePane::Inspector), "{locale}@{scale}x inspector hit parity failed");

            let nav_phys = [f64::from(layout.navigator.x) + f64::from(layout.navigator.width) * 0.5, f64::from(layout.navigator.y) + 32.0];
            let nav_logical = metrics.physical_to_logical_px(nav_phys);
            let nav_hit = map.hit_test_logical(nav_logical[0], nav_logical[1]);
            assert!(
                matches!(nav_hit, WorkspaceHitTarget::Pane(ActiveWorkspacePane::Navigator) | WorkspaceHitTarget::NavigatorEntry { .. }),
                "{locale}@{scale}x navigator hit parity failed: {nav_hit:?}"
            );
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
    let metrics = ViewportMetrics::new(1000.0, 680.0, 1.5);
    let map = compute_workspace_layout_map(&app, metrics);
    let paper = map.scene_paper.clone().expect("layout scene paper mapping");

    // Compute exact panel-0 plot origin and panel-1 plot-origin offset from compiled graph scenes.
    let sub0 = app.session().render_sheet_scene(&g1).expect("compile g1");
    let sub1 = app.session().render_sheet_scene(&g2).expect("compile g2");
    let scale0 = ((200.0 / sub0.width_pt.max(1.0)).min(150.0 / sub0.height_pt.max(1.0))).max(0.05);
    let scale1 = ((170.0 / sub1.width_pt.max(1.0)).min(130.0 / sub1.height_pt.max(1.0))).max(0.05);
    let p0_plot_origin = PointPt::new(24.0 + sub0.plot_rect_pt.x * scale0, 24.0 + sub0.plot_rect_pt.y * scale0);
    let p1_offset = PointPt::new(sub1.plot_rect_pt.x * scale1, sub1.plot_rect_pt.y * scale1);

    // Start dragging Panel 1 at its interior center (away from 8 resize handles).
    let start_pt = PointPt::new(345.0, 245.0);
    let start_phys = paper.scene_pt_to_pixel(start_pt).expect("panel 1 start pixel");
    let start_log = metrics.physical_to_logical_px(start_phys);
    let down = dispatch_ui_event(
        &mut app,
        UiInputEvent::PointerDown { x: start_log[0], y: start_log[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("pointer down panel 1");
    assert_eq!(down.action, "layout.item_drag_start");
    assert!(matches!(app.interaction.canvas_drag, Some(CanvasDragTarget::LayoutItem { item_index: 1, resize_handle: None, .. })));

    // Move Panel 1 so its plot origin lands 2pt right of Panel 0's plot origin X (within default snap tolerance).
    let desired_p1_x = (p0_plot_origin.x - p1_offset.x) + 2.0;
    let delta_x = desired_p1_x - 260.0;
    let move_pt = PointPt::new(start_pt.x + delta_x, start_pt.y);
    let move_phys = paper.scene_pt_to_pixel(move_pt).expect("panel 1 move pixel");
    let move_log = metrics.physical_to_logical_px(move_phys);
    let pre_drag_rgba =
        render_workspace_rgba(&app, RenderTargetSpec { width_px: metrics.physical_width_px(), height_px: metrics.physical_height_px(), dpi: 144 })
            .expect("pre drag render");

    dispatch_ui_event(&mut app, UiInputEvent::PointerMove { x: move_log[0], y: move_log[1], modifiers: KeyModifiers::default() }, metrics)
        .expect("pointer move panel 1");

    assert!(
        app.canvas_dialogs.canvas.active_snap_guides.iter().any(|g| g.source == SnapSource::PlotOrigin),
        "expected PlotOrigin snap guide during live drag, got: {:?}",
        app.canvas_dialogs.canvas.active_snap_guides
    );
    let CanvasDragTarget::LayoutItem { preview_rect_pt: Some(preview), .. } =
        app.interaction.canvas_drag.as_ref().expect("active canvas drag preview")
    else {
        panic!("expected LayoutItem preview_rect_pt during drag");
    };
    assert!(
        ((preview.x + p1_offset.x) - p0_plot_origin.x).abs() <= 0.5,
        "panel 1 plot origin ({}) failed to snap to panel 0 plot origin ({})",
        preview.x + p1_offset.x,
        p0_plot_origin.x
    );

    let mid_drag_rgba =
        render_workspace_rgba(&app, RenderTargetSpec { width_px: metrics.physical_width_px(), height_px: metrics.physical_height_px(), dpi: 144 })
            .expect("mid drag render");
    assert_ne!(pre_drag_rgba.rgba, mid_drag_rgba.rgba, "mid-drag preview and snap guide should update pixels");
    assert!(
        mid_drag_rgba.rgba.as_chunks::<4>().0.iter().any(|px| px[0] == 220 && px[1] == 38 && px[2] == 38),
        "rendered mid-drag workspace should draw PlotOrigin alignment hairline"
    );

    let up = dispatch_ui_event(
        &mut app,
        UiInputEvent::PointerUp { x: move_log[0], y: move_log[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("pointer up panel 1");
    assert_eq!(up.command_id.as_deref(), Some("layout.setSpec"));
    assert!(app.canvas_dialogs.canvas.active_snap_guides.is_empty());

    let ls = app.session().document().layout_sheet(&layout_id).expect("layout sheet");
    let LayoutItem::LiveGraph { bounds_pt: moved_bounds, .. } = &ls.spec.items[1] else {
        panic!("expected LiveGraph item 1");
    };
    let moved_bounds = *moved_bounds;
    assert!(((moved_bounds.x + p1_offset.x) - p0_plot_origin.x).abs() <= 0.5);

    // Exercise live BottomRight resize handle drag on Panel 1 while keeping TopLeft anchored.
    let br_pt = PointPt::new(moved_bounds.x + moved_bounds.width, moved_bounds.y + moved_bounds.height);
    let br_phys = paper.scene_pt_to_pixel(br_pt).expect("bottom right handle px");
    let br_log = metrics.physical_to_logical_px(br_phys);
    dispatch_ui_event(
        &mut app,
        UiInputEvent::PointerDown { x: br_log[0], y: br_log[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("pointer down resize handle");
    assert!(matches!(
        app.interaction.canvas_drag,
        Some(CanvasDragTarget::LayoutItem { item_index: 1, resize_handle: Some(LayoutResizeHandle::BottomRight), .. })
    ));

    let resized_pt = PointPt::new(br_pt.x + 32.0, br_pt.y + 24.0);
    let resized_phys = paper.scene_pt_to_pixel(resized_pt).expect("resized br px");
    let resized_log = metrics.physical_to_logical_px(resized_phys);
    dispatch_ui_event(&mut app, UiInputEvent::PointerMove { x: resized_log[0], y: resized_log[1], modifiers: KeyModifiers::default() }, metrics)
        .expect("pointer move resize");

    if let Some(CanvasDragTarget::LayoutItem {
        preview_rect_pt: Some(resize_preview), resize_handle: Some(LayoutResizeHandle::BottomRight), ..
    }) = &app.interaction.canvas_drag
    {
        assert!((resize_preview.x - moved_bounds.x).abs() < 1e-3, "TopLeft X must stay anchored during BottomRight resize");
        assert!((resize_preview.y - moved_bounds.y).abs() < 1e-3, "TopLeft Y must stay anchored during BottomRight resize");
        assert!(resize_preview.width > moved_bounds.width + 20.0);
        assert!(resize_preview.height > moved_bounds.height + 15.0);
    } else {
        panic!("expected live resize preview_rect_pt");
    }

    let resize_up = dispatch_ui_event(
        &mut app,
        UiInputEvent::PointerUp { x: resized_log[0], y: resized_log[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("pointer up resize");
    assert_eq!(resize_up.command_id.as_deref(), Some("layout.setSpec"));

    let ls_after = app.session().document().layout_sheet(&layout_id).expect("layout sheet after resize");
    let LayoutItem::LiveGraph { bounds_pt: final_bounds, .. } = &ls_after.spec.items[1] else {
        panic!("expected LiveGraph item 1");
    };
    assert!(final_bounds.width > moved_bounds.width + 20.0);
    assert!(final_bounds.height > moved_bounds.height + 15.0);
}

#[test]
fn pinch_zoom_inertial_pan_coasting_and_print_dialog_flow() {
    let (mut app, layout_id, _, _) = build_qa_two_panel_app();
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

    let modal_img = render_workspace_rgba(&app, RenderTargetSpec { width_px: 800, height_px: 520, dpi: 144 }).expect("render print modal");
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
