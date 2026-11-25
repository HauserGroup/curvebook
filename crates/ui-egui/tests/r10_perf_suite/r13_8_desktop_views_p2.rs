#![forbid(unsafe_code)]

//! `R13.8` Phase B2 Part 2 (`R13.4`, `R13.7`) desktop `egui 0.37` verification tests.

use std::sync::Arc;

use crate::common::{TestRes, encode_base64, seed_xy_doc, select_sheet_and_settle, widget_center};
use crate::suite_helpers::{
    click_all, dbl_click_widget, make_sync_driver, new_shell_app, render_grid_overlay_shown, run_cmd, verify_r13_matrix_and_accesskit,
};
use curvebook_doc::{GraphAnnotation, LayoutItem, SheetId};
use curvebook_palette::Rgba8;
use curvebook_render::png::encode_png;
use curvebook_render::{RasterImage, RenderTargetSpec, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use serde_json::json;

fn encode_synthetic_png_bytes(width_px: u32, height_px: u32) -> Result<Vec<u8>, String> {
    let mut rgba = Vec::with_capacity((width_px * height_px * 4) as usize);
    for y in 0..height_px {
        for x in 0..width_px {
            let r = u8::try_from((x * 255) / width_px.max(1)).unwrap_or(255);
            let g = u8::try_from((y * 255) / height_px.max(1)).unwrap_or(255);
            rgba.extend_from_slice(&[r, g, 180, 255]);
        }
    }
    encode_png(&RasterImage { width_px, height_px, dpi: 96, rgba }).map_err(|e| e.to_string())
}

fn build_seeded_project() -> Result<(UiApp, Arc<StubPlatformClipboardAdapter>), String> {
    let (doc, _) = seed_xy_doc(
        "R13.8 Part 2 Suite",
        "xy-1",
        "Dose Response",
        "X",
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[("Control", &[3.0, 5.0, 7.0, 9.0, 11.0]), ("Treated", &[5.5, 9.0, 12.5, 16.0, 19.5])],
    )?;
    let mut app = new_shell_app(doc);
    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip.clone());
    run_cmd(&mut app, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy", "title": "Primary XY" }))?;
    run_cmd(&mut app, "analysis.run", json!({ "source_sheet_id": "xy-1", "kind": "linear_regression", "sheet_id": "res-lin" }))?;
    run_cmd(&mut app, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" }))?;
    run_cmd(&mut app, "layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-xy", "panel_id": "p-a" }))?;
    app.sync_views();
    Ok((app, clip))
}

fn parse_result_cell_coords(widget_id: &str) -> Option<(usize, usize)> {
    let (r_str, c_str) = widget_id.strip_prefix("result.cell.")?.split_once('.')?;
    Some((r_str.parse().ok()?, c_str.parse().ok()?))
}

#[test]
fn r13_4_result_range_copy_graph_paste_table_and_format_embedded_table() -> TestRes {
    let (mut app, clip) = build_seeded_project()?;
    run_cmd(&mut app, "sheet.select", json!({ "sheet_id": "res-lin" }))?;
    let mut drv = make_sync_driver(app, 1280.0, 800.0);

    // 1. Select rectangular cell range in ResultSheet via click + Shift+Click, then copy via ContextMenu.
    let mut cells: Vec<(usize, usize, String, egui::Pos2)> = drv
        .frame_meta()
        .entries
        .iter()
        .filter_map(|e| parse_result_cell_coords(&e.widget_id).map(|(r, c)| (r, c, e.widget_id.clone(), e.clipped_rect.center())))
        .collect();
    cells.sort_by_key(|(r, c, _, _)| (*r, *c));
    assert!(cells.len() >= 2, "expected at least 2 rendered result cells, got {cells:?}");
    let (r0, c0, cell0_id, _) = cells[0].clone();
    let (r1, c1, _, cell1_pos) = cells
        .iter()
        .find(|(r, c, _, _)| *r > r0 && *c >= c0)
        .or_else(|| cells.iter().find(|(r, c, _, _)| *r != r0 || *c != c0))
        .cloned()
        .ok_or("missing second result cell")?;

    drv.click_widget_id(&cell0_id)?;
    drv.click_at_button(cell1_pos, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    drv.click_at_button(cell1_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert_eq!(drv.frame_meta().find_by_widget_id("result.ctx.copy_selection").ok_or("missing copy_selection")?.z_layer, ZLayer::ContextMenu);
    drv.click_widget_id("result.ctx.copy_selection")?;
    assert_eq!(drv.state().last_intent_error, None);

    let copied = drv.state().app.interaction.last_copied_result_table.clone().ok_or("expected last_copied_result_table")?;
    let expected_rows: Vec<usize> = (r0.min(r1)..=r0.max(r1)).collect();
    let expected_cols: Vec<usize> = (c0.min(c1)..=c0.max(c1)).collect();
    assert_eq!(
        (&copied.source_sheet_id, &copied.selected_rows, &copied.selected_cols),
        (&SheetId::new("res-lin"), &Some(expected_rows.clone()), &Some(expected_cols.clone()))
    );
    assert!(!clip.last_text().unwrap_or_default().trim().is_empty());

    // 2. Switch to GraphSheet, right-click empty canvas -> `Paste Table` (`ZLayer::ContextMenu`).
    select_sheet_and_settle(&mut drv, "g-xy")?;
    let paper = drv.state().canvas_rect_pt.ok_or("missing canvas rect")?;
    drv.click_at_button(egui::pos2(paper.right() - 40.0, paper.bottom() - 40.0), PointerButton::Secondary, KeyModifiers::default())?;
    let paste_tbl = drv.frame_meta().find_by_widget_id("canvas.graph_ctx.paste_table").ok_or("missing paste_table")?;
    assert!(paste_tbl.z_layer == ZLayer::ContextMenu && paste_tbl.enabled);
    drv.click_widget_id("canvas.graph_ctx.paste_table")?;
    assert_eq!(drv.state().last_intent_error, None);
    {
        let gs = drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
        let Some(GraphAnnotation::EmbeddedResultTable { result_sheet_id, selected_rows, selected_cols, .. }) = gs.spec.annotations.first() else {
            return Err(format!("expected EmbeddedResultTable, got {:?}", gs.spec.annotations));
        };
        assert_eq!((result_sheet_id, selected_rows, selected_cols), (&SheetId::new("res-lin"), &Some(expected_rows), &Some(expected_cols)));
    }

    // 3. Double-click embedded result table -> Format Embedded Table -> Apply & Cancel -> single Undo.
    dbl_click_widget(&mut drv, "canvas.graph_ann.0")?;
    assert!(matches!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatEmbeddedTable { .. }));
    click_all(
        &mut drv,
        &[
            "embedded_table.transpose",
            "embedded_table.bold_headers",
            "embedded_table.font_size.9",
            "embedded_table.outer_border.15",
            "embedded_table.gridline.0",
            "embedded_table.fill.cream",
            "embedded_table.border_color.navy",
            "modal.apply",
        ],
    )?;
    assert_eq!(drv.state().last_intent_error, None);
    assert!(matches!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatEmbeddedTable { .. }));
    drv.click_widget_id("modal.cancel")?;
    assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    {
        let gs = drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy post-format")?;
        let Some(GraphAnnotation::EmbeddedResultTable { style, .. }) = gs.spec.annotations.first() else {
            return Err("expected formatted EmbeddedResultTable".into());
        };
        assert!(style.transpose && !style.bold_headers);
        assert!(
            (style.font_size_pt - 9.0).abs() < 1e-6 && (style.outer_border_width_pt - 1.5).abs() < 1e-6 && (style.gridline_width_pt).abs() < 1e-6
        );
        assert_eq!((style.fill_color, style.border_color), (Some(Rgba8::rgb(250, 248, 242)), Rgba8::rgb(30, 58, 138)));
    }

    run_cmd(&mut drv.state_mut().app, "edit.undo", json!({}))?;
    match drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy post-undo")?.spec.annotations.first() {
        Some(GraphAnnotation::EmbeddedResultTable { style, .. }) => assert!(!style.transpose, "single undo must revert transpose"),
        _ => return Err("expected EmbeddedResultTable after undo".into()),
    }
    Ok(())
}

#[test]
fn r13_7_clipboard_paste_handles_shift_aspect_rotation_format_image_and_non_exported_grid() -> TestRes {
    let (mut app, clip) = build_seeded_project()?;
    let synth_png = encode_synthetic_png_bytes(32, 16)?;
    clip.seed_image_bytes(synth_png.clone());
    run_cmd(&mut app, "sheet.select", json!({ "sheet_id": "g-xy" }))?;
    app.nav.inspector_open = true;
    app.egui_views.trailing_collapsed_rail = false;
    app.sync_views();
    let mut drv = make_sync_driver(app, 1280.0, 800.0);

    // 1. Cmd+V pastes clipboard PNG onto active GraphSheet (`2:1` aspect preserved).
    drv.press_key("V", KeyModifiers { meta: true, ..Default::default() })?;
    assert_eq!(drv.state().last_intent_error, None);
    let get_img =
        |d: &curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver<'_, crate::common::ShellTestState>| -> Result<GraphAnnotation, String> {
            d.state()
                .app
                .document()
                .graph_sheet(&SheetId::new("g-xy"))
                .and_then(|g| g.spec.annotations.first().cloned())
                .ok_or_else(|| "missing annotation".into())
        };
    let (init_w, init_h) = match get_img(&drv)? {
        GraphAnnotation::EmbeddedImage { asset_id, bounds_pt, lock_aspect_ratio, .. } => {
            assert!(lock_aspect_ratio && (bounds_pt.width / bounds_pt.height - 2.0).abs() < 0.05);
            assert!(drv.state().app.document().image_asset(&asset_id).is_some());
            (bounds_pt.width, bounds_pt.height)
        }
        other => return Err(format!("expected EmbeddedImage, got {other:?}")),
    };

    // 2. Click embedded image -> corner & rotation handles at `ZLayer::FloatingOverlay`.
    drv.click_widget_id("canvas.graph_ann.0")?;
    for wid in ["canvas.image.handle.nw", "canvas.image.handle.ne", "canvas.image.handle.sw", "canvas.image.handle.se", "canvas.image.handle.rotate"]
    {
        assert_eq!(drv.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}`"))?.z_layer, ZLayer::FloatingOverlay);
    }

    // 3-5. Locked SE drag -> Shift-unlocked SE drag -> rotate handle drag.
    let se1 = widget_center(&drv, "canvas.image.handle.se")?;
    drv.drag(se1, se1 + egui::vec2(60.0, 10.0))?;
    if let GraphAnnotation::EmbeddedImage { bounds_pt, .. } = get_img(&drv)? {
        assert!(bounds_pt.width > init_w + 10.0 && bounds_pt.height > init_h + 5.0 && (bounds_pt.width / bounds_pt.height - 2.0).abs() < 0.05);
    }
    let se2 = widget_center(&drv, "canvas.image.handle.se")?;
    drv.drag_with_modifiers(se2, se2 + egui::vec2(100.0, -25.0), KeyModifiers { shift: true, ..Default::default() })?;
    if let GraphAnnotation::EmbeddedImage { bounds_pt, .. } = get_img(&drv)? {
        assert!((bounds_pt.width / bounds_pt.height - 2.0).abs() > 0.25, "Shift drag must unlock aspect ratio");
    }
    let rot = widget_center(&drv, "canvas.image.handle.rotate")?;
    drv.drag(rot, rot + egui::vec2(45.0, 20.0))?;
    if let GraphAnnotation::EmbeddedImage { rotation_deg, .. } = get_img(&drv)? {
        assert!(rotation_deg.abs() > 1.0, "expected non-zero rotation_deg, got {rotation_deg}");
    }

    // 6. Double-click embedded image -> Format Image -> 4-edge crop + border + rotation.
    dbl_click_widget(&mut drv, "canvas.graph_ann.0")?;
    assert!(matches!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatImage { .. }));
    click_all(
        &mut drv,
        &[
            "format_image.crop.left_plus4",
            "format_image.crop.right_plus4",
            "format_image.crop.top_plus4",
            "format_image.crop.bottom_plus4",
            "format_image.border.enabled",
            "format_image.border.width.1",
            "format_image.rotation.90",
            "modal.ok",
        ],
    )?;
    assert_eq!(drv.state().last_intent_error, None);
    assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    if let GraphAnnotation::EmbeddedImage { crop, border, rotation_deg, .. } = get_img(&drv)? {
        assert!([crop.left_pt, crop.right_pt, crop.top_pt, crop.bottom_pt].iter().all(|v| (*v - 4.0).abs() < 1e-6));
        assert!(border.is_some() && (rotation_deg - 90.0).abs() < 1e-6);
    }

    // 7. Toggle Canvas Grid via Trailing Studio -> non-exported SVG & PNG parity.
    assert!(!drv.state().app.canvas_dialogs.grid_overlay_visible && !render_grid_overlay_shown(&drv.state().app));
    let export_pair = |app: &UiApp| -> Result<(String, Vec<u8>), String> {
        let sc = app.session().render_sheet_scene(&SheetId::new("g-xy")).map_err(|e| e.to_string())?;
        let svg = render_svg(&sc, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
        let png = encode_png(&render_scene(&sc, RenderTargetSpec { width_px: 360, height_px: 260, dpi: 96 }).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok((svg, png))
    };
    let (svg_off, png_off) = export_pair(&drv.state().app)?;
    click_all(&mut drv, &["inspector.graph.tab.canvas", "inspector.canvas.grid_overlay"])?;
    assert!(drv.state().app.canvas_dialogs.grid_overlay_visible && render_grid_overlay_shown(&drv.state().app));
    assert_eq!(export_pair(&drv.state().app)?, (svg_off, png_off), "Canvas Grid must NOT appear in exported SVG/PNG");

    // 8. LayoutItem::EmbeddedImage shows corner & rotate handles when selected.
    run_cmd(&mut drv.state_mut().app, "edit.pasteImage", json!({ "target_sheet_id": "lay-1", "bytes_base64": encode_base64(&synth_png) }))?;
    select_sheet_and_settle(&mut drv, "lay-1")?;
    let ls_img_idx = drv
        .state()
        .app
        .document()
        .layout_sheet(&SheetId::new("lay-1"))
        .and_then(|ls| ls.spec.items.iter().position(|it| matches!(it, LayoutItem::EmbeddedImage { .. })))
        .ok_or("missing layout EmbeddedImage")?;
    drv.click_widget_id(&format!("canvas.layout_panel.{ls_img_idx}"))?;
    assert!(drv.frame_meta().find_by_widget_id("canvas.image.handle.nw").is_some());
    assert!(drv.frame_meta().find_by_widget_id("canvas.image.handle.rotate").is_some());
    Ok(())
}

#[test]
fn r13_8_p2_14_beta_locales_3_themes_2_sizes_2_dpis_and_modal_capture() -> TestRes {
    let (mut base_app, clip) = build_seeded_project()?;
    clip.seed_image_bytes(encode_synthetic_png_bytes(32, 16)?);
    run_cmd(&mut base_app, "edit.pasteResultTable", json!({ "source_sheet_id": "res-lin", "target_sheet_id": "g-xy" }))?;
    run_cmd(&mut base_app, "sheet.select", json!({ "sheet_id": "g-xy" }))?;
    base_app.sync_views();
    let mut b_drv = make_sync_driver(base_app, 1280.0, 800.0);
    b_drv.press_key("V", KeyModifiers { meta: true, ..Default::default() })?;
    let cases = [
        ("ui.embeddedTable.format.open", json!({ "sheet_id": "g-xy", "annotation_index": 0 })),
        ("ui.image.format.open", json!({ "sheet_id": "g-xy", "annotation_index": 1 })),
    ];
    verify_r13_matrix_and_accesskit(b_drv.state().app.clone(), &cases, &cases, |app| {
        let doc = app.document().clone();
        app.canvas_dialogs.open_format_image(&doc, &SheetId::new("g-xy"), 1)
    })
}
