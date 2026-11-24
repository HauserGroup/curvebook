#![forbid(unsafe_code)]

//! `R13.8` Phase B2 Part 2 (`R13.4`, `R13.7`) desktop `egui 0.37` verification tests.
//!
//! Exercises every desktop view surface added for `R13.4` and `R13.7` through real
//! `egui` pointer/keyboard events via [`EguiHumanPathDriver`], plus `gui-layout-spec.md` §9.4
//! layout gates across all `14` active Beta locales, `Light`/`Dark`/`HighContrast` themes,
//! `640x420` & `1440x600` viewports, and `1.0x`/`2.0x` DPI scales.

use std::error::Error;
use std::sync::Arc;

use curvebook_doc::{Cell, DataSheet, GraphAnnotation, LayoutItem, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_palette::Rgba8;
use curvebook_render::png::encode_png;
use curvebook_render::{RasterImage, RenderTargetSpec, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::{check_accesskit_scrollbar_non_overlap, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents};
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::{UiApp, UiThemeMode, dispatch_gui_action};
use serde_json::json;

type TestRes = Result<(), Box<dyn Error>>;

struct TestState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    outside_click_captured: bool,
    outside_wheel_captured: bool,
    paper_rect_pt: Option<egui::Rect>,
    grid_overlay_shown: bool,
}

impl TestState {
    fn new(app: UiApp) -> Self {
        let mut views = app.egui_views.clone();
        views.canvas_view.synchronous_raster = true;
        Self {
            app,
            views,
            intents: PendingUiIntents::new(),
            outside_click_captured: false,
            outside_wheel_captured: false,
            paper_rect_pt: None,
            grid_overlay_shown: false,
        }
    }
}

fn make_driver<'a>(state: TestState, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, TestState> {
    let mut driver = EguiHumanPathDriver::new(state, metrics, |ui, meta, st| {
        st.views.canvas_view.synchronous_raster = true;
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.outside_click_captured |= rep.outside_click_captured;
        st.outside_wheel_captured |= rep.outside_wheel_captured;
        st.paper_rect_pt = rep.regions.canvas_rect_pt;
        st.grid_overlay_shown = rep.grid_overlay_shown;
    });
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let res = st.intents.commit_if_settled(&mut st.app, false);
            assert!(res.is_ok(), "intent commit failed: {:?}", res.err());
            st.views = st.app.egui_views.clone();
            st.views.canvas_view.synchronous_raster = true;
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn modal_parity_violations(report: &curvebook_ui_egui::inspect::ParityReport) -> Vec<&curvebook_ui_egui::inspect::ParityViolation> {
    report
        .violations
        .iter()
        .filter(|v| {
            v.widget_id != "canvas.viewport"
                && v.widget_id != "canvas.split.viewport"
                && !v.widget_id.starts_with("canvas.layout_panel.")
                && !v.widget_id.starts_with("canvas.graph_ann.")
                && !v.widget_id.starts_with("canvas.image.handle.")
        })
        .collect()
}

fn encode_synthetic_png_bytes(width_px: u32, height_px: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut rgba = Vec::with_capacity((width_px * height_px * 4) as usize);
    for y in 0..height_px {
        for x in 0..width_px {
            let r = u8::try_from((x * 255) / width_px.max(1)).unwrap_or(255);
            let g = u8::try_from((y * 255) / height_px.max(1)).unwrap_or(255);
            rgba.extend_from_slice(&[r, g, 180, 255]);
        }
    }
    Ok(encode_png(&RasterImage { width_px, height_px, dpi: 96, rgba })?)
}

fn build_seeded_project() -> Result<(UiApp, Arc<StubPlatformClipboardAdapter>), Box<dyn Error>> {
    let mut doc = ProjectDocument::new("R13.8 Part 2 Suite")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5)?;
    xy.data_sets[0].title = String::from("Control");
    xy.data_sets[1].title = String::from("Treated");
    for r in 0..5 {
        let x = (r + 1) as f64;
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + 1.0);
        xy.data_sets[1].cells[r][0] = Cell::number(3.5 * x + 2.0);
    }
    doc.add_data_sheet(DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy))?)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip.clone());
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;

    app.execute_command("graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy", "title": "Primary XY" }))?;
    app.execute_command("analysis.run", json!({ "source_sheet_id": "xy-1", "kind": "linear_regression", "sheet_id": "res-lin" }))?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" }))?;
    app.execute_command("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-xy", "panel_id": "p-a" }))?;
    app.sync_views();
    Ok((app, clip))
}

fn parse_result_cell_coords(widget_id: &str) -> Option<(usize, usize)> {
    let rest = widget_id.strip_prefix("result.cell.")?;
    let (r_str, c_str) = rest.split_once('.')?;
    Some((r_str.parse().ok()?, c_str.parse().ok()?))
}

#[test]
fn r13_4_result_range_copy_graph_paste_table_and_format_embedded_table() -> TestRes {
    let (mut app, clip) = build_seeded_project()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "res-lin" }))?;
    app.sync_views();

    // 1. Select rectangular cell range in ResultSheet via click + Shift+Click, then copy via ContextMenu.
    let mut res_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    let mut cells: Vec<(usize, usize, String, egui::Pos2)> = res_drv
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

    res_drv.click_widget_id(&cell0_id)?;
    res_drv.click_at_button(cell1_pos, PointerButton::Primary, KeyModifiers { shift: true, ..KeyModifiers::default() })?;
    res_drv.click_at_button(cell1_pos, PointerButton::Secondary, KeyModifiers::default())?;

    let ctx_copy = res_drv.frame_meta().find_by_widget_id("result.ctx.copy_selection").ok_or("missing result.ctx.copy_selection")?;
    assert_eq!(ctx_copy.z_layer, ZLayer::ContextMenu);
    res_drv.click_widget_id("result.ctx.copy_selection")?;

    let mut app = res_drv.state().app.clone();
    let copied = app.interaction.last_copied_result_table.clone().ok_or("expected last_copied_result_table after Copy Selection")?;
    assert_eq!(copied.source_sheet_id, SheetId::new("res-lin"));
    let expected_rows: Vec<usize> = (r0.min(r1)..=r0.max(r1)).collect();
    let expected_cols: Vec<usize> = (c0.min(c1)..=c0.max(c1)).collect();
    assert_eq!(copied.selected_rows, Some(expected_rows.clone()));
    assert_eq!(copied.selected_cols, Some(expected_cols.clone()));
    assert!(!clip.last_text().unwrap_or_default().trim().is_empty());

    // 2. Switch to GraphSheet, right-click empty canvas -> `Paste Table` (`ZLayer::ContextMenu`).
    app.execute_command("sheet.select", json!({ "sheet_id": "g-xy" }))?;
    app.sync_views();
    let mut g_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    let paper = g_drv.state().paper_rect_pt.ok_or("missing canvas rect")?;
    let paste_anchor = egui::pos2(paper.right() - 40.0, paper.bottom() - 40.0);
    g_drv.click_at_button(paste_anchor, PointerButton::Secondary, KeyModifiers::default())?;

    let paste_tbl = g_drv.frame_meta().find_by_widget_id("canvas.graph_ctx.paste_table").ok_or("missing canvas.graph_ctx.paste_table")?;
    assert_eq!(paste_tbl.z_layer, ZLayer::ContextMenu);
    assert!(paste_tbl.enabled);
    g_drv.click_widget_id("canvas.graph_ctx.paste_table")?;

    {
        let gs = g_drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
        let Some(GraphAnnotation::EmbeddedResultTable { result_sheet_id, selected_rows, selected_cols, .. }) = gs.spec.annotations.first() else {
            return Err(format!("expected EmbeddedResultTable annotation, got {:?}", gs.spec.annotations).into());
        };
        assert_eq!(*result_sheet_id, SheetId::new("res-lin"));
        assert_eq!(*selected_rows, Some(expected_rows));
        assert_eq!(*selected_cols, Some(expected_cols));
    }

    // 3. Double-click embedded result table to open `Format Embedded Table`, edit style, Apply & OK, single Undo.
    let ann_center = g_drv.frame_meta().find_by_widget_id("canvas.graph_ann.0").ok_or("missing canvas.graph_ann.0")?.clipped_rect.center();
    g_drv.click_at(ann_center)?;
    g_drv.click_at(ann_center)?;
    assert!(
        matches!(g_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatEmbeddedTable { .. }),
        "expected FormatEmbeddedTable dialog, got {:?}",
        g_drv.state().app.canvas_dialogs.active_dialog
    );

    g_drv.click_widget_id("embedded_table.transpose")?;
    g_drv.click_widget_id("embedded_table.bold_headers")?;
    g_drv.click_widget_id("embedded_table.font_size.9")?;
    g_drv.click_widget_id("embedded_table.outer_border.15")?;
    g_drv.click_widget_id("embedded_table.gridline.0")?;
    g_drv.click_widget_id("embedded_table.fill.cream")?;
    g_drv.click_widget_id("embedded_table.border_color.navy")?;
    g_drv.click_widget_id("modal.apply")?;
    assert!(matches!(g_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatEmbeddedTable { .. }));
    g_drv.click_widget_id("modal.cancel")?;
    assert_eq!(g_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let mut app = g_drv.state().app.clone();
    {
        let gs = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy post-format")?;
        let Some(GraphAnnotation::EmbeddedResultTable { style, .. }) = gs.spec.annotations.first() else {
            return Err("expected formatted EmbeddedResultTable".into());
        };
        assert!(style.transpose);
        assert!(!style.bold_headers);
        assert!((style.font_size_pt - 9.0).abs() < 1e-6);
        assert!((style.outer_border_width_pt - 1.5).abs() < 1e-6);
        assert!((style.gridline_width_pt - 0.0).abs() < 1e-6);
        assert_eq!(style.fill_color, Some(Rgba8::rgb(250, 248, 242)));
        assert_eq!(style.border_color, Rgba8::rgb(30, 58, 138));
    }

    app.execute_command("edit.undo", json!({}))?;
    let gs_undo = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy post-undo")?;
    if let Some(GraphAnnotation::EmbeddedResultTable { style, .. }) = gs_undo.spec.annotations.first() {
        assert!(!style.transpose, "single undo should revert FormatEmbeddedTable transpose");
    } else {
        return Err("expected EmbeddedResultTable after undo".into());
    }
    Ok(())
}

#[test]
fn r13_7_clipboard_paste_handles_shift_aspect_rotation_format_image_and_non_exported_grid() -> TestRes {
    let (mut app, clip) = build_seeded_project()?;
    clip.seed_image_bytes(encode_synthetic_png_bytes(32, 16)?);
    app.execute_command("sheet.select", json!({ "sheet_id": "g-xy" }))?;
    app.nav.inspector_open = true;
    app.egui_views.trailing_collapsed_rail = false;
    app.sync_views();

    // 1. Cmd/Ctrl+V pastes clipboard PNG onto active GraphSheet (`2:1` aspect preserved).
    let mut g_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    g_drv.press_key("V", KeyModifiers { meta: true, ..KeyModifiers::default() })?;

    let (init_w, init_h, init_asset) = {
        let gs = g_drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy after paste")?;
        let Some(GraphAnnotation::EmbeddedImage { asset_id, bounds_pt, lock_aspect_ratio, .. }) = gs.spec.annotations.first() else {
            return Err(format!("expected EmbeddedImage after Cmd+V, got {:?}", gs.spec.annotations).into());
        };
        assert!(*lock_aspect_ratio);
        let ratio = bounds_pt.width / bounds_pt.height;
        assert!((ratio - 2.0).abs() < 0.05, "expected 2:1 pasted aspect ratio, got {ratio}");
        (bounds_pt.width, bounds_pt.height, asset_id.clone())
    };
    assert!(g_drv.state().app.document().image_asset(&init_asset).is_some());

    // 2. Click embedded image -> corner & rotation handles appear at `ZLayer::FloatingOverlay`.
    g_drv.click_widget_id("canvas.graph_ann.0")?;
    for wid in ["canvas.image.handle.nw", "canvas.image.handle.ne", "canvas.image.handle.sw", "canvas.image.handle.se", "canvas.image.handle.rotate"]
    {
        let entry = g_drv.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}`"))?;
        assert_eq!(entry.z_layer, ZLayer::FloatingOverlay);
    }

    // 3. Drag `se` corner WITHOUT Shift -> grows cleanly while keeping 2:1 aspect ratio locked.
    let se_center = g_drv.frame_meta().find_by_widget_id("canvas.image.handle.se").ok_or("missing se handle")?.clipped_rect.center();
    g_drv.drag(se_center, se_center + egui::vec2(60.0, 10.0))?;
    {
        let gs = g_drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy post locked resize")?;
        let Some(GraphAnnotation::EmbeddedImage { bounds_pt, .. }) = gs.spec.annotations.first() else {
            return Err("expected EmbeddedImage".into());
        };
        assert!(bounds_pt.width > init_w + 10.0, "expected locked resize growth: {} -> {}", init_w, bounds_pt.width);
        assert!(bounds_pt.height > init_h + 5.0);
        assert!((bounds_pt.width / bounds_pt.height - 2.0).abs() < 0.05);
    }

    // 4. Drag `se` corner WITH Shift -> unlocks aspect ratio cleanly.
    let se_center_2 = g_drv.frame_meta().find_by_widget_id("canvas.image.handle.se").ok_or("missing se handle 2")?.clipped_rect.center();
    g_drv.drag_with_modifiers(se_center_2, se_center_2 + egui::vec2(100.0, -25.0), KeyModifiers { shift: true, ..KeyModifiers::default() })?;
    {
        let gs = g_drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy post Shift resize")?;
        let Some(GraphAnnotation::EmbeddedImage { bounds_pt, .. }) = gs.spec.annotations.first() else {
            return Err("expected EmbeddedImage".into());
        };
        let ratio = bounds_pt.width / bounds_pt.height;
        assert!((ratio - 2.0).abs() > 0.25, "expected Shift drag to unlock aspect ratio away from 2.0, got {ratio}");
    }

    // 5. Drag `rotate` handle -> updates `rotation_deg`.
    let rot_center = g_drv.frame_meta().find_by_widget_id("canvas.image.handle.rotate").ok_or("missing rotate handle")?.clipped_rect.center();
    g_drv.drag(rot_center, rot_center + egui::vec2(45.0, 20.0))?;
    {
        let gs = g_drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy post rotate")?;
        let Some(GraphAnnotation::EmbeddedImage { rotation_deg, .. }) = gs.spec.annotations.first() else {
            return Err("expected EmbeddedImage".into());
        };
        assert!(rotation_deg.abs() > 1.0, "expected non-zero rotation_deg after drag, got {rotation_deg}");
    }

    // 6. Double-click embedded image -> `Format Image` dialog -> 4-edge crop + border + rotation.
    let img_center = g_drv.frame_meta().find_by_widget_id("canvas.graph_ann.0").ok_or("missing canvas.graph_ann.0")?.clipped_rect.center();
    g_drv.click_at(img_center)?;
    g_drv.click_at(img_center)?;
    assert!(matches!(g_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatImage { .. }));

    g_drv.click_widget_id("format_image.crop.left_plus4")?;
    g_drv.click_widget_id("format_image.crop.right_plus4")?;
    g_drv.click_widget_id("format_image.crop.top_plus4")?;
    g_drv.click_widget_id("format_image.crop.bottom_plus4")?;
    g_drv.click_widget_id("format_image.border.enabled")?;
    g_drv.click_widget_id("format_image.border.width.1")?;
    g_drv.click_widget_id("format_image.rotation.90")?;
    g_drv.click_widget_id("modal.ok")?;
    assert_eq!(g_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    {
        let gs = g_drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy post FormatImage")?;
        let Some(GraphAnnotation::EmbeddedImage { crop, border, rotation_deg, .. }) = gs.spec.annotations.first() else {
            return Err("expected EmbeddedImage".into());
        };
        assert!((crop.left_pt - 4.0).abs() < 1e-6);
        assert!((crop.right_pt - 4.0).abs() < 1e-6);
        assert!((crop.top_pt - 4.0).abs() < 1e-6);
        assert!((crop.bottom_pt - 4.0).abs() < 1e-6);
        assert!(border.is_some());
        assert!((*rotation_deg - 90.0).abs() < 1e-6);
    }

    // 7. Toggle Canvas Grid via Trailing Studio (`ui.gridOverlay.toggle`), assert non-exported SVG & PNG parity.
    assert!(!g_drv.state().app.canvas_dialogs.grid_overlay_visible);
    assert!(!g_drv.state().grid_overlay_shown);
    let scene_off = g_drv.state().app.session().render_sheet_scene(&SheetId::new("g-xy"))?;
    let svg_off = render_svg(&scene_off, &SvgExportOptions::default())?;
    let png_off = encode_png(&render_scene(&scene_off, RenderTargetSpec { width_px: 360, height_px: 260, dpi: 96 })?)?;

    g_drv.click_widget_id("inspector.graph.tab.canvas")?;
    g_drv.click_widget_id("inspector.canvas.grid_overlay")?;
    assert!(g_drv.state().app.canvas_dialogs.grid_overlay_visible);
    assert!(g_drv.state().grid_overlay_shown);

    let scene_on = g_drv.state().app.session().render_sheet_scene(&SheetId::new("g-xy"))?;
    let svg_on = render_svg(&scene_on, &SvgExportOptions::default())?;
    let png_on = encode_png(&render_scene(&scene_on, RenderTargetSpec { width_px: 360, height_px: 260, dpi: 96 })?)?;
    assert_eq!(svg_off, svg_on, "Canvas Grid must NOT appear in exported SVG");
    assert_eq!(png_off, png_on, "Canvas Grid must NOT appear in exported PNG");

    // 8. Also check LayoutItem::EmbeddedImage shows corner & rotate handles when selected.
    let mut lay_app = g_drv.state().app.clone();
    let img_b64 = {
        let mut b = String::new();
        const LUT: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for chunk in encode_synthetic_png_bytes(32, 16)?.chunks(3) {
            let n =
                (u32::from(chunk[0]) << 16) | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8) | u32::from(chunk.get(2).copied().unwrap_or(0));
            b.push(LUT[((n >> 18) & 0x3F) as usize] as char);
            b.push(LUT[((n >> 12) & 0x3F) as usize] as char);
            b.push(if chunk.len() > 1 { LUT[((n >> 6) & 0x3F) as usize] as char } else { '=' });
            b.push(if chunk.len() > 2 { LUT[(n & 0x3F) as usize] as char } else { '=' });
        }
        b
    };
    lay_app.execute_command("edit.pasteImage", json!({ "target_sheet_id": "lay-1", "bytes_base64": img_b64 }))?;
    lay_app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" }))?;
    lay_app.sync_views();
    let ls_img_idx = lay_app
        .document()
        .layout_sheet(&SheetId::new("lay-1"))
        .and_then(|ls| ls.spec.items.iter().position(|it| matches!(it, LayoutItem::EmbeddedImage { .. })))
        .ok_or("missing layout EmbeddedImage")?;
    let mut l_drv = make_driver(TestState::new(lay_app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    l_drv.click_widget_id(&format!("canvas.layout_panel.{ls_img_idx}"))?;
    assert!(l_drv.frame_meta().find_by_widget_id("canvas.image.handle.nw").is_some());
    assert!(l_drv.frame_meta().find_by_widget_id("canvas.image.handle.rotate").is_some());
    Ok(())
}

#[test]
fn r13_8_p2_14_beta_locales_3_themes_2_sizes_2_dpis_and_modal_capture() -> TestRes {
    let (mut base_app, clip) = build_seeded_project()?;
    let png = encode_synthetic_png_bytes(32, 16)?;
    clip.seed_image_bytes(png);
    base_app.execute_command("edit.pasteResultTable", json!({ "source_sheet_id": "res-lin", "target_sheet_id": "g-xy" }))?;
    base_app.execute_command("sheet.select", json!({ "sheet_id": "g-xy" }))?;
    base_app.sync_views();
    let mut b_drv = make_driver(TestState::new(base_app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    b_drv.press_key("V", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    let base_app = b_drv.state().app.clone();
    assert_eq!(BETA_LOCALE_CODES.len(), 14);

    for locale in BETA_LOCALE_CODES {
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            for (cmd_id, params) in [
                ("ui.embeddedTable.format.open", json!({ "sheet_id": "g-xy", "annotation_index": 0 })),
                ("ui.image.format.open", json!({ "sheet_id": "g-xy", "annotation_index": 1 })),
            ] {
                let mut app = base_app.clone();
                assert_eq!(app.set_locale(locale), locale);
                let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": cmd_id, "params": params }))?;
                app.sync_views();
                let mut drv = make_driver(TestState::new(app), ViewportMetrics::new(w_pt, h_pt, 1.0));
                let parity = drv.verify_pixel_to_hit_box_parity();
                let modal_v = modal_parity_violations(&parity);
                assert!(modal_v.is_empty(), "{cmd_id} `{locale}` @ {w_pt}x{h_pt} parity: {modal_v:?}");
                let sb = drv.check_scrollbar_non_overlap();
                assert!(sb.is_empty(), "{cmd_id} `{locale}` @ {w_pt}x{h_pt} scrollbar overlap: {sb:?}");

                let outside = egui::pos2(4.0, (h_pt as f32) * 0.5);
                drv.wheel_at(outside, egui::vec2(0.0, -60.0))?;
                drv.click_at(outside)?;
                assert!(drv.state().outside_click_captured, "{cmd_id} `{locale}` click capture");
                assert!(drv.state().outside_wheel_captured, "{cmd_id} `{locale}` wheel capture");
            }
        }
    }

    for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        for dpi in [1.0_f64, 2.0] {
            for (cmd_id, params) in [
                ("ui.embeddedTable.format.open", json!({ "sheet_id": "g-xy", "annotation_index": 0 })),
                ("ui.image.format.open", json!({ "sheet_id": "g-xy", "annotation_index": 1 })),
            ] {
                let mut app = base_app.clone();
                app.set_theme_mode(theme);
                let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": cmd_id, "params": params }))?;
                app.sync_views();
                let drv = make_driver(TestState::new(app), ViewportMetrics::new(1440.0, 600.0, dpi));
                let parity = drv.verify_pixel_to_hit_box_parity();
                let modal_v = modal_parity_violations(&parity);
                assert!(modal_v.is_empty(), "{cmd_id} {theme:?} @ {dpi}x parity: {modal_v:?}");
                assert!(drv.check_scrollbar_non_overlap().is_empty());
            }
        }
    }

    let mut app_ak = base_app;
    let doc_ak = app_ak.document().clone();
    assert!(app_ak.canvas_dialogs.open_format_image(&doc_ak, &SheetId::new("g-xy"), 1));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut views = EguiShellViewState::default();
    views.canvas_view.synchronous_raster = true;
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
