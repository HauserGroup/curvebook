//! Unit tests for the virtualized `Column` / multi-archetype data grid.

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, ImportLinkSpec, LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange, SheetId,
    TableArchetype,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;

use super::{ColumnGridEguiState, ColumnGridRenderContext, GridInlineEditBuffer, GridInlineEditTarget, render_column_grid};
use crate::UiApp;
use crate::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use crate::egui_shell::{PendingUiIntents, Tokens};
use crate::interaction::ViewportMetrics;

fn build_column_app(cols: usize, rows: usize) -> UiApp {
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows).expect("column table");
    for c in 0..cols {
        col.data_sets[c].title = format!("Group {}", c + 1);
        for r in 0..rows {
            col.data_sets[c].cells[r][0] = Cell::number((r as f64) + (c as f64) * 0.1);
        }
    }
    let ds = DataSheet::new("col-1", "Column Test", TableArchetype::Column(col)).expect("data sheet");
    doc.add_data_sheet(ds).expect("add data sheet");
    doc.select_in_navigator(&SheetId::from("col-1")).expect("select sheet");
    UiApp::from_session(Session::from_document(doc))
}

#[test]
fn default_grid_state_enables_inspect_meta_and_inherits_metrics() {
    let state = ColumnGridEguiState::default();
    assert!(state.record_inspect_meta);
    assert!(!state.row_titles_collapsed);
    let tokens = Tokens::default();
    let ctx = ColumnGridRenderContext::new(&tokens, false);
    assert_eq!(ctx.metrics.grid_row_h, 24.0);
    assert!(!ctx.modal_blocked);
}

#[test]
fn one_million_cell_column_grid_renders_strictly_visible_window() {
    let app = build_column_app(1_000, 1_000);
    let tokens = Tokens::default();
    let state = (app, ColumnGridEguiState::default());
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        state,
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (app, grid_state)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let rep = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
            assert!(rep.rendered_cells < 600, "must virtualize 1M cells, rendered {}", rep.rendered_cells);
            assert!(rep.rendered_cells > 100);
            assert!(rep.vertical_scrollbar_rect_pt.is_some());
            assert!(rep.horizontal_scrollbar_rect_pt.is_some());
            let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    let parity = driver.verify_pixel_to_hit_box_parity();
    assert!(parity.is_clean(), "1M-cell parity violations: {:?}", parity.violations);
    assert!(driver.check_scrollbar_non_overlap().is_empty());
}

#[test]
fn linked_region_guard_opens_on_double_click_and_blocks_outside_clicks() {
    let mut app = build_column_app(3, 8);
    {
        let mut doc = app.document().clone();
        let ds = doc.data_sheet_mut(&SheetId::from("col-1")).expect("sheet");
        ds.add_linked_region(LinkedDataRegion {
            id: String::from("link-1"),
            spec: ImportLinkSpec { source: LinkSource::Clipboard, filter: Default::default(), placement: Default::default() },
            target_range: SheetCellRange::new(curvebook_doc::CellCoord::new(0, 0, 0), curvebook_doc::CellCoord::new(2, 1, 0)),
            provenance_summary: String::from("Raw Plate › A1:B3"),
        })
        .expect("add link");
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }
    let tokens = Tokens::default();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, ColumnGridEguiState::default()),
        ViewportMetrics::new(1024.0, 640.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (app, grid_state)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
            let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
        },
    );
    let center = driver.frame_meta().find_by_widget_id("grid.cell.1.0.0").expect("cell meta").clipped_rect.center();
    driver.click_at(center).expect("click 1");
    driver.click_at(center).expect("click 2");
    assert!(driver.state().1.active_link_popover.is_some());
    assert!(driver.frame_meta().find_by_widget_id("grid.link_popover.panel").is_some());
}

#[test]
fn linked_region_popover_unlink_detaches_only_targeted_region() {
    let mut app = build_column_app(4, 8);
    {
        let mut doc = app.document().clone();
        let ds = doc.data_sheet_mut(&SheetId::from("col-1")).expect("sheet");
        ds.add_linked_region(LinkedDataRegion {
            id: String::from("link-1"),
            spec: ImportLinkSpec { source: LinkSource::Clipboard, filter: Default::default(), placement: Default::default() },
            target_range: SheetCellRange::new(curvebook_doc::CellCoord::new(0, 0, 0), curvebook_doc::CellCoord::new(2, 0, 0)),
            provenance_summary: String::from("Plate A › A1:A3"),
        })
        .expect("add link-1");
        ds.add_linked_region(LinkedDataRegion {
            id: String::from("link-2"),
            spec: ImportLinkSpec { source: LinkSource::Clipboard, filter: Default::default(), placement: Default::default() },
            target_range: SheetCellRange::new(curvebook_doc::CellCoord::new(0, 2, 0), curvebook_doc::CellCoord::new(2, 2, 0)),
            provenance_summary: String::from("Plate B › C1:C3"),
        })
        .expect("add link-2");
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }
    let tokens = Tokens::default();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, ColumnGridEguiState::default()),
        ViewportMetrics::new(1024.0, 640.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (app, grid_state)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
            let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
        },
    );

    // Double-click cell (row=1, col=0) inside link-1 to open its popover.
    let cell_center = driver.frame_meta().find_by_widget_id("grid.cell.1.0.0").expect("cell meta").clipped_rect.center();
    driver.click_at(cell_center).expect("click 1");
    driver.click_at(cell_center).expect("click 2");
    assert_eq!(driver.state().1.active_link_popover.as_ref().map(|p| p.region_id.as_str()), Some("link-1"));

    // Click Unlink in the popover and verify only link-1 is detached while link-2 remains intact.
    let unlink_center = driver.frame_meta().find_by_widget_id("grid.link_popover.unlink").expect("unlink button meta").clipped_rect.center();
    driver.click_at(unlink_center).expect("click unlink");

    assert!(driver.state().1.active_link_popover.is_none(), "popover should close after unlink");
    let ds = driver.state().0.document().data_sheet(&SheetId::from("col-1")).expect("sheet");
    let remaining_ids: Vec<&str> = ds.linked_regions.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(remaining_ids, vec!["link-2"], "unlinking link-1 must leave link-2 attached");
}

#[test]
fn buffered_inline_edit_commits_once_on_enter_and_cancels_on_escape() {
    let app = build_column_app(2, 5);
    let tokens = Tokens::default();
    let grid_state = ColumnGridEguiState {
        inline_edit: Some(GridInlineEditBuffer {
            target: GridInlineEditTarget::Cell(GridCellCoord::new(0, 0, 0)),
            draft: String::from("42.75"),
            focused_once: false,
        }),
        ..Default::default()
    };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, grid_state),
        ViewportMetrics::new(900.0, 560.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (app, grid_state)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
            let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
        },
    );
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!(driver.state().1.inline_edit.is_none());
    assert_eq!(driver.state().0.session().undo_depth(), 1);
    let cell = driver.state().0.document().data_sheet(&SheetId::from("col-1")).unwrap().cell(curvebook_doc::CellCoord::new(0, 0, 0)).unwrap();
    assert_eq!(cell.value, curvebook_doc::CellValue::Number(42.75));
}

fn build_xy_app(rows: usize) -> UiApp {
    let mut doc = ProjectDocument::default();
    let mut xy = curvebook_doc::XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, rows).expect("xy table");
    for r in 0..rows {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 10.0);
    }
    let ds = DataSheet::new("xy-1", "XY Test", TableArchetype::Xy(xy)).expect("data sheet");
    doc.add_data_sheet(ds).expect("add data sheet");
    doc.select_in_navigator(&SheetId::from("xy-1")).expect("select sheet");
    UiApp::from_session(Session::from_document(doc))
}

fn build_grouped_app(cols: usize, subs: usize, rows: usize) -> UiApp {
    let mut doc = ProjectDocument::default();
    let mut grp = curvebook_doc::GroupedTable::new(curvebook_doc::SubcolumnMode::RawReplicates(subs), cols, rows).expect("grouped table");
    grp.row_factor_title = String::from("Time");
    grp.column_factor_title = String::from("Treatment");
    for c in 0..cols {
        for r in 0..rows {
            for s in 0..subs {
                grp.data_sets[c].cells[r][s] = Cell::number((r * 10 + c * 3 + s) as f64);
            }
        }
    }
    let ds = DataSheet::new("grp-1", "Grouped Test", TableArchetype::Grouped(grp)).expect("data sheet");
    doc.add_data_sheet(ds).expect("add data sheet");
    doc.select_in_navigator(&SheetId::from("grp-1")).expect("select sheet");
    UiApp::from_session(Session::from_document(doc))
}

struct RawGridFrameOutput {
    shapes: Vec<egui::epaint::ClippedShape>,
    repaint_delay: Option<std::time::Duration>,
}

#[allow(clippy::too_many_arguments)]
fn run_grid_raw_frame(
    ctx: &egui::Context,
    app: &mut UiApp,
    grid_state: &mut ColumnGridEguiState,
    tokens: &Tokens,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    t: f64,
    commit_intents: bool,
) -> (RawGridFrameOutput, crate::egui_shell::FrameInspectMeta, super::DataGridFrameReport, Vec<crate::egui_shell::UiIntent>) {
    run_grid_raw_frame_with_focus(ctx, app, grid_state, tokens, size, events, t, commit_intents, true)
}

#[allow(clippy::too_many_arguments)]
fn run_grid_raw_frame_with_focus(
    ctx: &egui::Context,
    app: &mut UiApp,
    grid_state: &mut ColumnGridEguiState,
    tokens: &Tokens,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    t: f64,
    commit_intents: bool,
    pane_focused: bool,
) -> (RawGridFrameOutput, crate::egui_shell::FrameInspectMeta, super::DataGridFrameReport, Vec<crate::egui_shell::UiIntent>) {
    tokens.apply_to_ctx(ctx);
    ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
        style.scroll_animation = egui::style::ScrollAnimation::none();
        style.animation_time = 0.0;
    });
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        time: Some(t),
        predicted_dt: 1.0 / 60.0,
        focused: true,
        events,
        ..Default::default()
    };
    raw.viewports.entry(egui::ViewportId::ROOT).and_modify(|vp| vp.native_pixels_per_point = Some(1.0));
    let mut meta = crate::egui_shell::FrameInspectMeta::new();
    let mut emitted = Vec::new();
    let mut report = super::DataGridFrameReport::default();
    let mut output = ctx.run_ui(raw, |ui| {
        meta.begin_pass(ui.ctx());
        let mut intents = PendingUiIntents::new();
        let t_ctx = Tokens::get(ui.ctx());
        let rctx = ColumnGridRenderContext::new(&t_ctx, false).with_pane_focused(pane_focused);
        report = super::render_data_grid(ui, app, grid_state, rctx, &mut meta, &mut intents);
        emitted.extend(intents.intents().iter().cloned());
        if commit_intents && !ui.ctx().will_discard() {
            let _ = intents.commit_if_settled(app, false);
        }
    });
    let shapes = std::mem::take(&mut output.shapes);
    let repaint_delay = output.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
    output.drop_without_applying_deltas();
    (RawGridFrameOutput { shapes, repaint_delay }, meta, report, emitted)
}

fn settle_grid_raw_frames(
    ctx: &egui::Context,
    app: &mut UiApp,
    grid_state: &mut ColumnGridEguiState,
    tokens: &Tokens,
    size: egui::Vec2,
    t0: f64,
) -> (RawGridFrameOutput, crate::egui_shell::FrameInspectMeta, super::DataGridFrameReport) {
    let mut last = run_grid_raw_frame(ctx, app, grid_state, tokens, size, Vec::new(), t0, true);
    for step in 1..6 {
        if !last.1.entries.is_empty() && last.0.repaint_delay != Some(std::time::Duration::ZERO) {
            break;
        }
        last = run_grid_raw_frame(ctx, app, grid_state, tokens, size, Vec::new(), t0 + f64::from(step) * 0.016, true);
    }
    (last.0, last.1, last.2)
}

#[test]
fn r20_8_multi_cell_drag_zero_lag_drag_back_clamped_and_deduped() {
    let mut app = build_column_app(4, 8);
    let mut grid_state = ColumnGridEguiState::default();
    let tokens = Tokens::default();
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let vp = egui::vec2(800.0, 480.0);

    let (_, meta0, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.05);
    let cell_rect = |wid: &str| meta0.find_by_widget_id(wid).unwrap_or_else(|| panic!("missing {wid}")).clipped_rect;
    let c_2_1 = cell_rect("grid.cell.2.1.0");
    let c_4_2 = cell_rect("grid.cell.4.2.0");
    let c_0_0 = cell_rect("grid.cell.0.0.0");

    // Frame 1: press on (2,1) AND move to (4,2) in the same frame WITHOUT committing intents first:
    // live preview must already reflect (2..=4, 1..=2) with zero 1-frame lag!
    let (_out1, meta1, _, emitted1) = run_grid_raw_frame(
        &ctx,
        &mut app,
        &mut grid_state,
        &tokens,
        vp,
        vec![
            egui::Event::PointerMoved(c_2_1.center()),
            egui::Event::PointerButton { pos: c_2_1.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerMoved(c_4_2.center()),
        ],
        0.12,
        false,
    );
    assert!(
        emitted1.iter().any(|i| matches!(i, crate::egui_shell::UiIntent::ExtendGridSelection { coord, .. } if *coord == GridCellCoord::new(4, 2, 0)))
    );
    let is_sel = |m: &crate::egui_shell::FrameInspectMeta, wid: &str| {
        m.find_by_widget_id(wid).and_then(|e| e.value.as_ref()).and_then(|v| v.get("selected")).and_then(|b| b.as_bool()).unwrap_or(false)
    };
    assert!(is_sel(&meta1, "grid.cell.2.1.0"));
    assert!(is_sel(&meta1, "grid.cell.3.2.0"));
    assert!(is_sel(&meta1, "grid.cell.4.2.0"));
    assert!(!is_sel(&meta1, "grid.cell.0.0.0"));

    // Frame 2: commit pending intents manually into app to simulate next step, then move 2 px INSIDE (4,2):
    // deduplication must emit ZERO ExtendGridSelection intents.
    app.grid.select_single_sync(GridCellCoord::new(2, 1, 0));
    if let Some(ds) = app.document().data_sheet(&SheetId::from("col-1")).cloned() {
        app.grid.extend_shift_range(&ds, GridCellCoord::new(4, 2, 0));
    }
    let small_jitter = egui::pos2(c_4_2.center().x + 2.0, c_4_2.center().y - 2.0);
    let (_, _, _, emitted_jitter) =
        run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, vec![egui::Event::PointerMoved(small_jitter)], 0.14, true);
    assert!(
        !emitted_jitter.iter().any(|i| matches!(i, crate::egui_shell::UiIntent::ExtendGridSelection { .. })),
        "pointer jitter inside same cell must not emit ExtendGridSelection: {emitted_jitter:?}"
    );

    // Frame 3: drag BACK across anchor (2,1) in both axes to (0,0) -> selection flips cleanly to (0..=2, 0..=1).
    let (_, meta3, _, _) =
        run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, vec![egui::Event::PointerMoved(c_0_0.center())], 0.16, true);
    assert_eq!(app.grid.tool.selection_bounds(), Some((GridCellCoord::new(0, 0, 0), GridCellCoord::new(2, 1, 0))));
    assert!(is_sel(&meta3, "grid.cell.0.0.0"));
    assert!(is_sel(&meta3, "grid.cell.2.1.0"));
    assert!(!is_sel(&meta3, "grid.cell.4.2.0"));

    // Frame 4: out-of-bounds clamping (way past top-left into gutter/negative coords -> clamps cleanly to (0,0)..=(2,1)).
    let (_, _, _, _) =
        run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, vec![egui::Event::PointerMoved(egui::pos2(-50.0, -40.0))], 0.18, true);
    assert_eq!(app.grid.tool.selection_bounds(), Some((GridCellCoord::new(0, 0, 0), GridCellCoord::new(2, 1, 0))));

    // Frame 5: drag back ONTO anchor (2,1) and release -> collapses cleanly to 1 cell and does not open inline edit.
    let (_, _, _, _) = run_grid_raw_frame(
        &ctx,
        &mut app,
        &mut grid_state,
        &tokens,
        vp,
        vec![
            egui::Event::PointerMoved(c_2_1.center()),
            egui::Event::PointerButton {
                pos: c_2_1.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        0.20,
        true,
    );
    assert_eq!(app.grid.tool.selection_bounds(), Some((GridCellCoord::new(2, 1, 0), GridCellCoord::new(2, 1, 0))));
    assert!(grid_state.inline_edit.is_none());
}

#[test]
fn r20_8_edge_autoscroll_bounded_and_zero_idle_repaint_after_release() {
    let mut app = build_column_app(20, 60);
    let mut grid_state = ColumnGridEguiState::default();
    let tokens = Tokens::default();
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let vp = egui::vec2(460.0, 260.0);

    let (_, meta0, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.05);
    let start_c = meta0.find_by_widget_id("grid.cell.1.1.0").expect("cell 1,1").clipped_rect.center();

    // Press at (1,1) then move past bottom-right viewport edge (`vp.x + 30`, `vp.y + 30`).
    let _ = run_grid_raw_frame(
        &ctx,
        &mut app,
        &mut grid_state,
        &tokens,
        vp,
        vec![
            egui::Event::PointerMoved(start_c),
            egui::Event::PointerButton { pos: start_c, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        ],
        0.12,
        true,
    );

    let past_bottom_right = egui::pos2(vp.x + 32.0, vp.y + 32.0);
    let _ = run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, vec![egui::Event::PointerMoved(past_bottom_right)], 0.14, true);

    let mut prev_scroll = grid_state.scroll_offset_pt;
    let mut saw_scroll_repaint = false;
    for step in 0..6 {
        let t = 0.16 + f64::from(step) * (1.0 / 60.0);
        let (out, _, _, _) = run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, Vec::new(), t, true);
        let delta = grid_state.scroll_offset_pt - prev_scroll;
        assert!(delta.x > 0.0 && delta.y > 0.0, "expected positive bounded scroll delta, got {delta:?}");
        assert!(delta.x <= 20.0 && delta.y <= 20.0, "auto-scroll speed must be bounded per frame: {delta:?}");
        if out.repaint_delay == Some(std::time::Duration::ZERO) {
            saw_scroll_repaint = true;
        }
        prev_scroll = grid_state.scroll_offset_pt;
    }
    assert!(saw_scroll_repaint, "active edge auto-scroll must request repaint while scrolling");
    let (_, max_coord) = app.grid.tool.selection_bounds().expect("scrolled selection bounds");
    assert!(max_coord.row >= 8 && max_coord.col >= 3, "expected scrolled selection extension, got {max_coord:?}");

    // Release primary button and advance 2 idle frames -> ZERO idle repaints!
    let _ = run_grid_raw_frame(
        &ctx,
        &mut app,
        &mut grid_state,
        &tokens,
        vp,
        vec![egui::Event::PointerButton {
            pos: past_bottom_right,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        0.30,
        true,
    );
    let _ = run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, Vec::new(), 0.32, true);
    let (idle_out, _, _, _) = run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, Vec::new(), 0.34, true);
    let delay = idle_out.repaint_delay;
    assert_eq!(delay, Some(std::time::Duration::MAX), "must not emit idle repaints after drag release");
}

#[test]
fn r20_8_single_outer_perimeter_highlight_across_all_themes() {
    use crate::theme::{UiThemeMode, UiThemeSpec};

    for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        let mut app = build_column_app(4, 6);
        app.theme = UiThemeSpec::for_mode(mode);
        app.grid.select_single_sync(GridCellCoord::new(1, 0, 0));
        if let Some(ds) = app.document().data_sheet(&SheetId::from("col-1")).cloned() {
            app.grid.extend_shift_range(&ds, GridCellCoord::new(3, 1, 0));
        }
        let mut grid_state = ColumnGridEguiState::default();
        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let ctx = egui::Context::default();
        ctx.set_fonts(build_driver_font_definitions());
        let (out, meta, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, egui::vec2(800.0, 480.0), 0.05);

        let r10 = meta.find_by_widget_id("grid.cell.1.0.0").unwrap().clipped_rect;
        let r31 = meta.find_by_widget_id("grid.cell.3.1.0").unwrap().clipped_rect;
        let expected_perim = r10.union(r31);

        let mut accent_strokes = Vec::new();
        let mut focus_strokes = Vec::new();
        fn collect_rect_strokes(
            shapes: &[egui::epaint::ClippedShape],
            accent: egui::Color32,
            focus: egui::Color32,
            accent_out: &mut Vec<egui::Rect>,
            focus_out: &mut Vec<egui::Rect>,
        ) {
            for clipped in shapes {
                match &clipped.shape {
                    egui::Shape::Rect(rs) if rs.stroke.width > 0.0 => {
                        if rs.stroke.color == accent && (rs.stroke.width - 1.5).abs() < 0.05 {
                            accent_out.push(rs.rect);
                        } else if rs.stroke.color == focus && (rs.stroke.width - 2.0).abs() < 0.05 {
                            focus_out.push(rs.rect);
                        }
                    }
                    egui::Shape::Vec(children) => {
                        for sh in children {
                            if let egui::Shape::Rect(rs) = sh
                                && rs.stroke.width > 0.0
                            {
                                if rs.stroke.color == accent && (rs.stroke.width - 1.5).abs() < 0.05 {
                                    accent_out.push(rs.rect);
                                } else if rs.stroke.color == focus && (rs.stroke.width - 2.0).abs() < 0.05 {
                                    focus_out.push(rs.rect);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        collect_rect_strokes(&out.shapes, tokens.accent, tokens.focus_ring, &mut accent_strokes, &mut focus_strokes);

        assert_eq!(accent_strokes.len(), 1, "{mode:?}: expected single outer accent perimeter, got {accent_strokes:?}");
        assert!(
            (accent_strokes[0].min - expected_perim.min).length() < 1.0 && (accent_strokes[0].max - expected_perim.max).length() < 1.0,
            "{mode:?}: outer perimeter {:?} != expected {:?}",
            accent_strokes[0],
            expected_perim
        );
        assert_eq!(focus_strokes.len(), 1, "{mode:?}: expected single 2.0pt focus_ring on active cell");
        assert_ne!(tokens.active_bg, tokens.text_primary, "{mode:?}: active_bg must contrast cleanly against text_primary");
    }
}

#[test]
fn r20_8_x_column_and_row_and_column_header_drag_preserving_title_clicks() {
    let tokens = Tokens::default();
    let vp = egui::vec2(820.0, 500.0);

    // 1. X/Time multi-cell drag + Cmd+E X-range exclude
    {
        let mut app = build_xy_app(6);
        let mut grid_state = ColumnGridEguiState::default();
        let ctx = egui::Context::default();
        ctx.set_fonts(build_driver_font_definitions());
        let (_, meta0, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.05);
        let x1 = meta0.find_by_widget_id("grid.xcell.1.0").unwrap().clipped_rect.center();
        let x4 = meta0.find_by_widget_id("grid.xcell.4.0").unwrap().clipped_rect.center();
        // Drag from X row 1 down to X row 4 while drifting left into row_num gutter (clamps to X col 0!).
        let _ = run_grid_raw_frame(
            &ctx,
            &mut app,
            &mut grid_state,
            &tokens,
            vp,
            vec![
                egui::Event::PointerMoved(x1),
                egui::Event::PointerButton { pos: x1, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerMoved(egui::pos2(8.0, x4.y)),
                egui::Event::PointerButton {
                    pos: egui::pos2(8.0, x4.y),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            0.14,
            true,
        );
        let aux_id = egui::Id::unique(("r15_4_multi_grid_aux", "primary", "xy-1"));
        let bounds = ctx.data(|d| d.get_temp::<super::shared::MultiGridAuxState>(aux_id).and_then(|a| a.selected_x_bounds()));
        assert_eq!(bounds, Some((1, 4, 0, 0)), "X drag clamped horizontally must select X rows 1..=4");

        let cmd_e = egui::Event::Key { key: egui::Key::E, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        let (_, _, _, emitted_ex) = run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, vec![cmd_e], 0.18, true);
        assert!(emitted_ex.iter().any(|it| matches!(
            it,
            crate::egui_shell::UiIntent::EngineCommand { command_id, params }
            if command_id == "table.excludeCells"
                && params.get("target").and_then(|v| v.as_str()) == Some("x")
                && params.get("row_start").and_then(|v| v.as_u64()) == Some(1)
                && params.get("row_end").and_then(|v| v.as_u64()) == Some(4)
        )));
    }

    // 2. Row `#` gutter drag + drag-back, plus Grouped Tier 1 backward drag, Tier 2 drag vs click-to-edit, and Tier 1 double-click factor edit
    {
        let mut app = build_grouped_app(3, 2, 5);
        let mut grid_state = ColumnGridEguiState::default();
        let ctx = egui::Context::default();
        ctx.set_fonts(build_driver_font_definitions());
        let (_, meta0, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.05);
        let rn1 = meta0.find_by_widget_id("grid.row_num.1").unwrap().clipped_rect.center();
        let rn4 = meta0.find_by_widget_id("grid.row_num.4").unwrap().clipped_rect.center();
        let rn2 = meta0.find_by_widget_id("grid.row_num.2").unwrap().clipped_rect.center();

        let _ = run_grid_raw_frame(
            &ctx,
            &mut app,
            &mut grid_state,
            &tokens,
            vp,
            vec![
                egui::Event::PointerMoved(rn1),
                egui::Event::PointerButton { pos: rn1, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerMoved(rn4),
            ],
            0.12,
            true,
        );
        assert_eq!(app.grid.tool.selection_bounds(), Some((GridCellCoord::new(1, 0, 0), GridCellCoord::new(4, 2, 1))));
        let _ = run_grid_raw_frame(
            &ctx,
            &mut app,
            &mut grid_state,
            &tokens,
            vp,
            vec![
                egui::Event::PointerMoved(rn2),
                egui::Event::PointerButton { pos: rn2, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
            0.14,
            true,
        );
        assert_eq!(app.grid.tool.selection_bounds(), Some((GridCellCoord::new(1, 0, 0), GridCellCoord::new(2, 2, 1))));

        // Tier 1 backward drag from col 2 -> col 1 must include ALL subcolumns of col 2 (0..=1).
        let sup2 = meta0.find_by_widget_id("grid.col_super.2").unwrap().clipped_rect.center();
        let sup1 = meta0.find_by_widget_id("grid.col_super.1").unwrap().clipped_rect.center();
        let _ = run_grid_raw_frame(
            &ctx,
            &mut app,
            &mut grid_state,
            &tokens,
            vp,
            vec![
                egui::Event::PointerMoved(sup2),
                egui::Event::PointerButton { pos: sup2, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerMoved(sup1),
                egui::Event::PointerButton { pos: sup1, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
            0.16,
            true,
        );
        assert_eq!(app.grid.tool.selection_bounds(), Some((GridCellCoord::new(0, 1, 0), GridCellCoord::new(4, 2, 1))));

        // Tier 2 drag col_title.0 -> col_title.1 selects whole cols 0..=1 WITHOUT opening ColumnTitle inline edit!
        let ct0 = meta0.find_by_widget_id("grid.col_title.0").unwrap().clipped_rect.center();
        let ct1 = meta0.find_by_widget_id("grid.col_title.1").unwrap().clipped_rect.center();
        let _ = run_grid_raw_frame(
            &ctx,
            &mut app,
            &mut grid_state,
            &tokens,
            vp,
            vec![
                egui::Event::PointerMoved(ct0),
                egui::Event::PointerButton { pos: ct0, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerMoved(ct1),
                egui::Event::PointerButton { pos: ct1, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
            0.18,
            true,
        );
        assert_eq!(app.grid.tool.selection_bounds(), Some((GridCellCoord::new(0, 0, 0), GridCellCoord::new(4, 1, 1))));
        assert!(grid_state.inline_edit.is_none(), "dragging across col_title must not open title inline edit");

        // Plain non-drag click on col_title.0 STILL opens ColumnTitle inline edit, and Escape closes it.
        let _ = run_grid_raw_frame(
            &ctx,
            &mut app,
            &mut grid_state,
            &tokens,
            vp,
            vec![
                egui::Event::PointerMoved(ct0),
                egui::Event::PointerButton { pos: ct0, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            ],
            0.20,
            true,
        );
        let _ = run_grid_raw_frame(
            &ctx,
            &mut app,
            &mut grid_state,
            &tokens,
            vp,
            vec![egui::Event::PointerButton { pos: ct0, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
            0.22,
            true,
        );
        assert!(
            matches!(grid_state.inline_edit.as_ref().map(|e| &e.target), Some(GridInlineEditTarget::ColumnTitle { col: 0 })),
            "non-drag click on grid.col_title.0 must preserve inline edit"
        );
        grid_state.inline_edit = None;
        let _ = run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, Vec::new(), 0.24, true);

        // Double-click on grid.col_super.0 STILL opens column factor title editor.
        let sup0 = meta0.find_by_widget_id("grid.col_super.0").unwrap().clipped_rect.center();
        for (i, pressed) in [(0, true), (1, false), (2, true), (3, false)] {
            let _ = run_grid_raw_frame(
                &ctx,
                &mut app,
                &mut grid_state,
                &tokens,
                vp,
                vec![
                    egui::Event::PointerMoved(sup0),
                    egui::Event::PointerButton { pos: sup0, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                ],
                0.30 + f64::from(i) * 0.02,
                true,
            );
        }
        assert!(super::headers::is_factor_title_editing(&ctx, &SheetId::from("grp-1")));
    }
}

fn collect_grid_shapes_recursive(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<egui::Shape>) {
    fn push_shape(sh: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match sh {
            egui::Shape::Vec(children) => {
                for ch in children {
                    push_shape(ch, out);
                }
            }
            other => out.push(other.clone()),
        }
    }
    for cs in shapes {
        push_shape(&cs.shape, out);
    }
}

#[test]
fn r20_11_unemphasized_datagrid_selection_colors_and_high_contrast_dashed() {
    use crate::egui_shell::color32_to_rgba8;
    use crate::theme::{UiThemeMode, UiThemeSpec, wcag_contrast_ratio};

    for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        let mut app = build_column_app(4, 6);
        app.theme = UiThemeSpec::for_mode(mode);
        app.grid.select_single_sync(GridCellCoord::new(1, 0, 0));
        if let Some(ds) = app.document().data_sheet(&SheetId::from("col-1")).cloned() {
            app.grid.extend_shift_range(&ds, GridCellCoord::new(2, 1, 0));
        }
        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let ctx = egui::Context::default();
        ctx.set_fonts(build_driver_font_definitions());
        let mut grid_state = ColumnGridEguiState::default();
        let vp = egui::vec2(800.0, 480.0);

        // Focused frame: active_bg + focus_ring + accent perimeter + row # active_bg tint.
        let (focused_out, focused_meta, _, _) =
            run_grid_raw_frame_with_focus(&ctx, &mut app, &mut grid_state, &tokens, vp, Vec::new(), 0.05, true, true);
        let mut focused_shapes = Vec::new();
        collect_grid_shapes_recursive(&focused_out.shapes, &mut focused_shapes);
        assert!(
            focused_shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.active_bg)),
            "{mode:?}: focused grid must paint tokens.active_bg"
        );
        assert!(
            focused_shapes
                .iter()
                .any(|s| matches!(s, egui::Shape::Rect(r) if r.stroke.color == tokens.focus_ring && (r.stroke.width - 2.0).abs() < 0.05)),
            "{mode:?}: focused grid must paint 2.0pt focus_ring"
        );
        assert!(
            !focused_shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.selection_unemphasized_bg)),
            "{mode:?}: focused grid must not paint selection_unemphasized_bg"
        );

        let rn1_rect = focused_meta.find_by_widget_id("grid.row_num.1").unwrap().clipped_rect;
        let ct0_rect = focused_meta.find_by_widget_id("grid.col_title.0").unwrap().clipped_rect;
        let cell_21_rect = focused_meta.find_by_widget_id("grid.cell.2.1.0").unwrap().clipped_rect;

        // Unfocused frame: selection_unemphasized_bg, zero active_bg/focus_ring/accent, neutral header_bg on row # & col header,
        // plus dashed border segments in HighContrast (>= 3:1 against surface_bg).
        let (unfocused_out, _, _, _) = run_grid_raw_frame_with_focus(&ctx, &mut app, &mut grid_state, &tokens, vp, Vec::new(), 0.10, true, false);
        let mut unfocused_shapes = Vec::new();
        collect_grid_shapes_recursive(&unfocused_out.shapes, &mut unfocused_shapes);

        assert!(
            unfocused_shapes
                .iter()
                .any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.selection_unemphasized_bg && cell_21_rect.contains(r.rect.center()))),
            "{mode:?}: unfocused active cell must paint tokens.selection_unemphasized_bg"
        );
        assert!(
            !unfocused_shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.active_bg)),
            "{mode:?}: unfocused grid must not paint tokens.active_bg"
        );
        assert!(
            !unfocused_shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.stroke.width > 0.0 && (r.stroke.color == tokens.focus_ring || r.stroke.color == tokens.accent))),
            "{mode:?}: unfocused grid must not paint blue focus_ring or accent selection strokes"
        );
        assert!(
            unfocused_shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.header_bg && rn1_rect.contains(r.rect.center()))),
            "{mode:?}: unfocused active row # must fall back to neutral tokens.header_bg"
        );
        assert!(
            !unfocused_shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tokens.active_bg && ct0_rect.contains(r.rect.center()))),
            "{mode:?}: unfocused col header must not carry active_bg tint"
        );

        if mode == UiThemeMode::HighContrast {
            assert!(
                wcag_contrast_ratio(color32_to_rgba8(tokens.border), color32_to_rgba8(tokens.surface_bg)) >= 3.0,
                "HighContrast border must have >= 3:1 contrast against surface_bg"
            );
            let dashed_count = unfocused_shapes
                .iter()
                .filter(|s| matches!(s, egui::Shape::LineSegment { points, stroke } if stroke.color == tokens.border && (stroke.width - 1.0).abs() < 0.05 && cell_21_rect.expand(4.0).contains(points[0])))
                .count();
            assert!(dashed_count >= 4, "HighContrast unfocused active cell/range must paint 1.0pt dashed border segments, got {dashed_count}");
        } else {
            assert!(
                unfocused_shapes
                    .iter()
                    .any(|s| matches!(s, egui::Shape::Rect(r) if r.stroke.color == tokens.border && (r.stroke.width - 1.0).abs() < 0.05)),
                "{mode:?}: unfocused active cell must paint 1.0pt tokens.border"
            );
            assert!(
                unfocused_shapes
                    .iter()
                    .any(|s| matches!(s, egui::Shape::Rect(r) if r.stroke.color == tokens.divider && (r.stroke.width - 1.0).abs() < 0.05)),
                "{mode:?}: unfocused range perimeter must paint 1.0pt tokens.divider"
            );
        }
    }
}

#[test]
fn r20_11_x_y_selection_mutual_exclusion_zero_stale_highlights() {
    let mut app = build_xy_app(5);
    let mut grid_state = ColumnGridEguiState::default();
    let tokens = Tokens::default();
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let vp = egui::vec2(800.0, 480.0);

    let (_, meta0, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.05);
    let x2 = meta0.find_by_widget_id("grid.xcell.2.0").unwrap().clipped_rect;
    let y1 = meta0.find_by_widget_id("grid.cell.1.0.0").unwrap().clipped_rect;
    let y3 = meta0.find_by_widget_id("grid.cell.3.0.0").unwrap().clipped_rect;

    let is_sel = |m: &crate::egui_shell::FrameInspectMeta, wid: &str| {
        m.find_by_widget_id(wid).and_then(|e| e.value.as_ref()).and_then(|v| v.get("selected")).and_then(|b| b.as_bool()).unwrap_or(false)
    };
    let count_focus_rings = |out: &RawGridFrameOutput| -> usize {
        let mut shapes = Vec::new();
        collect_grid_shapes_recursive(&out.shapes, &mut shapes);
        shapes
            .iter()
            .filter(|s| matches!(s, egui::Shape::Rect(r) if r.stroke.color == tokens.focus_ring && (r.stroke.width - 2.0).abs() < 0.05))
            .count()
    };

    // 1. Click Y(1,0,0) -> only Y is active/highlighted.
    let _ = run_grid_raw_frame(
        &ctx,
        &mut app,
        &mut grid_state,
        &tokens,
        vp,
        vec![
            egui::Event::PointerMoved(y1.center()),
            egui::Event::PointerButton { pos: y1.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: y1.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
        0.10,
        true,
    );
    let (out_y, meta_y, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.12);
    assert_eq!(app.grid.tool.active, Some(GridCellCoord::new(1, 0, 0)));
    assert_eq!(app.grid.active_x_cell, None);
    assert!(is_sel(&meta_y, "grid.cell.1.0.0"));
    assert!(!is_sel(&meta_y, "grid.xcell.2.0"));
    assert_eq!(count_focus_rings(&out_y), 1);

    // 2. Click X(2,0) -> immediately supersedes Y, syncs app.grid.active_x_cell, zero stale Y highlight.
    let _ = run_grid_raw_frame(
        &ctx,
        &mut app,
        &mut grid_state,
        &tokens,
        vp,
        vec![
            egui::Event::PointerMoved(x2.center()),
            egui::Event::PointerButton { pos: x2.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: x2.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
        0.16,
        true,
    );
    let (out_x, meta_x, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.18);
    assert_eq!(app.grid.active_x_cell, Some((2, 0)));
    assert_eq!(app.grid.tool.active, None);
    assert!(app.grid.selected_coords.is_empty());
    assert!(is_sel(&meta_x, "grid.xcell.2.0"));
    assert!(!is_sel(&meta_x, "grid.cell.1.0.0"));
    assert_eq!(count_focus_rings(&out_x), 1);

    // 3. ArrowRight moves from X(2,0) into Y(2,0,0) -> clears X selection completely.
    let arrow_r = egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let _ = run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, vec![arrow_r], 0.22, true);
    let (out_ar, meta_ar, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.24);
    assert_eq!(app.grid.active_x_cell, None);
    assert_eq!(app.grid.tool.active, Some(GridCellCoord::new(2, 0, 0)));
    assert!(is_sel(&meta_ar, "grid.cell.2.0.0"));
    assert!(!is_sel(&meta_ar, "grid.xcell.2.0"));
    assert_eq!(count_focus_rings(&out_ar), 1);

    // 4. ArrowLeft moves from Y(2,0,0) back into X(2,0) -> clears Y selection completely.
    let arrow_l = egui::Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let _ = run_grid_raw_frame(&ctx, &mut app, &mut grid_state, &tokens, vp, vec![arrow_l], 0.28, true);
    let (out_al, meta_al, _) = settle_grid_raw_frames(&ctx, &mut app, &mut grid_state, &tokens, vp, 0.30);
    assert_eq!(app.grid.active_x_cell, Some((2, 0)));
    assert_eq!(app.grid.tool.active, None);
    assert!(is_sel(&meta_al, "grid.xcell.2.0"));
    assert!(!is_sel(&meta_al, "grid.cell.2.0.0"));
    assert_eq!(count_focus_rings(&out_al), 1);

    // 5. Same-frame click on Y(3,0,0) clears X immediately on that frame (even before commit_intents!).
    let (out_imm, meta_imm, _, emitted_imm) = run_grid_raw_frame(
        &ctx,
        &mut app,
        &mut grid_state,
        &tokens,
        vp,
        vec![
            egui::Event::PointerMoved(y3.center()),
            egui::Event::PointerButton { pos: y3.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: y3.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
        0.34,
        false,
    );
    assert!(
        emitted_imm.iter().any(|i| matches!(i, crate::egui_shell::UiIntent::SelectGridCell { coord, .. } if *coord == GridCellCoord::new(3, 0, 0)))
    );
    assert!(is_sel(&meta_imm, "grid.cell.3.0.0"));
    assert!(!is_sel(&meta_imm, "grid.xcell.2.0"));
    assert_eq!(count_focus_rings(&out_imm), 1);
}
