//! Large-project and interactive responsiveness stress benchmarks & regression checks (`R17.3`).
//!
//! Covers all 5 real-shaped stress scenarios:
//!   (a) 50+ sheet project (sheet switching, Leading Dock/Navigator rendering, project recalc, `sync_views`)
//!   (b) 100k-row data grids (initial & steady frame, wheel scroll, hit-testing, column stats)
//!   (c) multi-panel Page Layouts (compile + software raster + interactive drag/HUD frame)
//!   (d) rapid undo/redo bursts (large document + saved checkpoints + atomic groups)
//!   (e) live cell typing / editing on large sheets (mutating `table.setCell`, validation/recalc, inline editor frames)
#![forbid(unsafe_code)]

use std::time::Instant;

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime, DesktopWindowId};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook_chart::compile_layout;
use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, LayoutItem, LayoutSheet, LayoutSpec, MultipleVariablesTable, ProjectDocument, RectPt, SheetId,
    SubcolumnMode, TableArchetype, VariableSource, XyTable,
};
use curvebook_engine::Session;
use curvebook_render::{RenderTargetSpec, render_scene};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::workspace_nav::NavigatorNestingMode;
use eframe::egui;
use serde_json::json;

#[derive(Debug, Clone, Copy)]
struct StatsSummary {
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

impl StatsSummary {
    fn from_ms(mut samples: Vec<f64>) -> Self {
        samples.sort_by(f64::total_cmp);
        let n = samples.len().max(1);
        let total_ms: f64 = samples.iter().sum();
        let mean_ms = total_ms / (n as f64);
        let p50_ms = samples[n / 2];
        let p95_ms = samples[((n - 1) * 95) / 100];
        let max_ms = samples.last().copied().unwrap_or(0.0);
        Self { mean_ms, p50_ms, p95_ms, max_ms }
    }
}

fn build_native_bench_app(doc: ProjectDocument) -> Result<(egui::Context, CurveBookNativeApp, DesktopWindowId), String> {
    let viewport = ViewportMetrics::new(1440.0, 900.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: true,
        gui_only: false,
        ui_backend: UiBackend::Egui,
        initial_viewport: viewport,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window_with_session(Session::from_document(doc), viewport, false)?;
    let app = CurveBookNativeApp::new(rt, win_id, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    Ok((ctx, app, win_id))
}

fn step_native_frame(ctx: &egui::Context, app: &mut CurveBookNativeApp, sim_time: &mut f64, events: Vec<egui::Event>) -> f64 {
    *sim_time += 1.0 / 60.0;
    let t0 = Instant::now();
    let mut ev_opt = Some(events);
    for _pass in 0..4 {
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(*sim_time),
            predicted_dt: 1.0 / 60.0,
            focused: true,
            events: ev_opt.take().unwrap_or_default(),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw_input, |ui| app.step_ui(ui));
        let discard = out.platform_output.requested_discard();
        let _clipped = ctx.tessellate(std::mem::take(&mut out.shapes), 1.0);
        out.textures_delta.clear();
        if !discard {
            break;
        }
        *sim_time += 1.0 / 60.0;
    }
    t0.elapsed().as_secs_f64() * 1000.0
}

fn build_60_sheet_project(rows_per_sheet: usize) -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut s = Session::new();
    // 20 data sheets + 20 graphs + ~13 results + 4 layouts = ~57 sheets
    for i in 0..20 {
        let ds_id = format!("ds-{i}");
        let kind = if i % 3 == 0 { "xy" } else { "column" };
        s.execute(
            "sheet.createData",
            json!({ "sheet_id": ds_id, "title": format!("Dataset {i:02}"), "kind": kind, "cols": 3, "rows": rows_per_sheet, "auto_graph": false }),
        )?;
        if kind == "xy" {
            let xs: Vec<f64> = (0..rows_per_sheet.min(200)).map(|r| r as f64 * 0.5).collect();
            let ys: Vec<Vec<f64>> =
                (0..rows_per_sheet.min(200)).map(|r| vec![(r as f64) * 0.8 + i as f64, (r as f64) * 1.1, (r as f64) * 1.3]).collect();
            s.execute("table.setXCells", json!({ "sheet_id": ds_id, "values": xs }))?;
            s.execute("table.setCells", json!({ "sheet_id": ds_id, "rows": ys }))?;
        } else {
            let ys: Vec<Vec<f64>> = (0..rows_per_sheet.min(200)).map(|r| vec![10.0 + r as f64, 15.0 + r as f64, 20.0 + r as f64]).collect();
            s.execute("table.setCells", json!({ "sheet_id": ds_id, "rows": ys }))?;
            s.execute("stats.run", json!({ "source_sheet_id": ds_id, "sheet_id": format!("res-{i}"), "kind": "unpaired_t" }))?;
        }
        s.execute("graph.create", json!({ "sheet_id": format!("gs-{i}"), "data_sheet_id": ds_id, "title": format!("Graph {i:02}") }))?;
    }
    for l in 0..4 {
        let g0 = format!("gs-{}", l * 3);
        let g1 = format!("gs-{}", l * 3 + 1);
        s.execute("layout.create", json!({ "sheet_id": format!("lay-{l}"), "title": format!("Layout {l}") }))?;
        s.execute(
            "layout.addPanel",
            json!({ "sheet_id": format!("lay-{l}"), "graph_sheet_id": g0, "bounds_pt": { "x": 20.0, "y": 20.0, "width": 240.0, "height": 180.0 } }),
        )?;
        s.execute(
            "layout.addPanel",
            json!({ "sheet_id": format!("lay-{l}"), "graph_sheet_id": g1, "bounds_pt": { "x": 280.0, "y": 20.0, "width": 240.0, "height": 180.0 } }),
        )?;
    }
    let doc = s.document().clone();
    assert!(doc.total_sheet_count() >= 55, "expected >= 55 sheets, got {}", doc.total_sheet_count());
    Ok(doc)
}

fn build_100k_grid_doc(rows: usize) -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    let mut mv = MultipleVariablesTable::new(6, rows)?;
    for c in 0..5 {
        mv.variables[c].title = format!("Var {}", c + 1);
        for r in 0..rows {
            mv.variables[c].cells[r] = Cell::number(((r % 997) as f64) * 0.25 + (c as f64));
        }
    }
    mv.variables[5].title = String::from("CalcSum");
    mv.variables[5].source = VariableSource::Calculated { formula: String::from("[Var 1] + [Var 2]") };
    for r in 0..rows {
        mv.variables[5].cells[r] = Cell::number(((r % 997) as f64) * 0.5 + 1.0);
    }
    let ds = DataSheet::new(SheetId::from("mv-100k"), "100k MV Assay", TableArchetype::MultipleVariables(mv))?;
    doc.add_data_sheet(ds)?;
    doc.select_in_navigator(&SheetId::from("mv-100k"))?;
    Ok(doc)
}

fn build_multipanel_layout_doc(panel_count: usize, pts_per_curve: usize) -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    let mut xy = XyTable::new(SubcolumnMode::Single, 3, pts_per_curve)?;
    for r in 0..pts_per_curve {
        xy.x_cells[r][0] = Cell::number((r as f64) * 0.1);
        for c in 0..3 {
            xy.data_sets[c].cells[r][0] = Cell::number(((r as f64) * 0.05 + c as f64).sin() * 50.0 + 50.0);
        }
    }
    let ds = DataSheet::new(SheetId::from("xy-src"), "Curve Data", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds)?;

    let mut s = Session::from_document(doc);
    let mut items = Vec::with_capacity(panel_count);
    let cols = 4usize;
    for idx in 0..panel_count {
        let gid_str = format!("gs-{idx}");
        s.execute("graph.create", json!({ "sheet_id": gid_str, "data_sheet_id": "xy-src", "title": format!("Response {idx}") }))?;
        let gid = SheetId::from(gid_str);
        let c = idx % cols;
        let r = idx / cols;
        items.push(LayoutItem::LiveGraph {
            id: format!("panel-{idx}"),
            graph_sheet_id: gid,
            bounds_pt: RectPt::new(24.0 + (c as f64) * 145.0, 24.0 + (r as f64) * 120.0, 130.0, 105.0),
            scale_percent: 100.0,
            badge_override: None,
        });
    }
    let mut doc = s.document().clone();
    let layout =
        LayoutSheet::new(SheetId::from("lay-multi"), "Multi-Panel Figure", LayoutSpec { items, align_plot_origins: true, ..LayoutSpec::default() })?;
    doc.add_layout_sheet(layout)?;
    doc.select_in_navigator(&SheetId::from("lay-multi"))?;
    Ok(doc)
}

#[test]
fn stress_profile_all_five_scenarios() -> Result<(), Box<dyn std::error::Error>> {
    let is_release = !cfg!(debug_assertions);
    let scale = if is_release { 1 } else { 4 }; // run full 100k in release, scaled in debug tests if needed
    let rows_100k = 100_000 / scale;

    profile_scenario_a(is_release)?;
    profile_scenario_b(is_release, rows_100k)?;
    profile_scenario_c(is_release)?;
    profile_scenario_d(is_release, rows_100k)?;
    profile_scenario_e(is_release, rows_100k)?;
    profile_scenario_f(is_release)?;
    Ok(())
}

fn profile_scenario_a(is_release: bool) -> Result<(), Box<dyn std::error::Error>> {
    // --- (a) 50+ Sheet Project (switching, tree/dock rendering, recalc/sync) ---
    let doc = build_60_sheet_project(if is_release { 500 } else { 50 })?;
    let sheet_count = doc.total_sheet_count();
    let sheet_ids: Vec<SheetId> = doc
        .data_sheets
        .iter()
        .map(|s| s.id.clone())
        .chain(doc.result_sheets.iter().map(|s| s.id.clone()))
        .chain(doc.graph_sheets.iter().map(|s| s.id.clone()))
        .take(20)
        .collect();

    let (ctx, mut app, win_id) = build_native_bench_app(doc.clone())?;
    app.runtime_mut().window_mut(win_id)?.app_mut().nav.nesting_mode = NavigatorNestingMode::NestedResultsUnderData;
    app.runtime_mut().window_mut(win_id)?.app_mut().sync_views();
    let mut sim = 1.0_f64;
    let _ = step_native_frame(&ctx, &mut app, &mut sim, vec![]);

    let mut switch_samples = Vec::new();
    for sid in &sheet_ids {
        let t0 = Instant::now();
        {
            let ui_app = app.runtime_mut().window_mut(win_id)?.app_mut();
            let _ = ui_app.execute_command("sheet.select", json!({ "sheet_id": sid.as_str() }))?;
        }
        let _ = step_native_frame(&ctx, &mut app, &mut sim, vec![]);
        switch_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let sw = StatsSummary::from_ms(switch_samples);

    let mut session_recalc = Session::from_document(doc);
    let t_recalc0 = Instant::now();
    session_recalc.recalculate_all()?;
    let recalc_ms = t_recalc0.elapsed().as_secs_f64() * 1000.0;

    println!(
        "[R17.3 Scenario A | 60-sheet ({sheet_count} sheets) NestedDock switch+frame] mean={:.3}ms p50={:.3}ms p95={:.3}ms max={:.3}ms | recalc_all={:.3}ms",
        sw.mean_ms, sw.p50_ms, sw.p95_ms, sw.max_ms, recalc_ms
    );
    if is_release {
        assert!(sw.mean_ms < 10.0, "Scenario A sheet switch+frame regressed: mean={:.3}ms", sw.mean_ms);
    }
    Ok(())
}

fn profile_scenario_b(is_release: bool, rows_100k: usize) -> Result<(), Box<dyn std::error::Error>> {
    // --- (b) 100k-Row Data Grids (paint, scroll, hit-test, column stats) ---
    let doc = build_100k_grid_doc(rows_100k)?;
    let (ctx, mut app, win_id) = build_native_bench_app(doc.clone())?;
    let mut sim = 1.0_f64;
    let init_ms = step_native_frame(&ctx, &mut app, &mut sim, vec![]);

    let cell_pt = app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").map(|e| e.clipped_rect.center()).unwrap_or(egui::pos2(450.0, 260.0));
    let mut scroll_samples = Vec::new();
    for _ in 0..20 {
        let ms = step_native_frame(
            &ctx,
            &mut app,
            &mut sim,
            vec![
                egui::Event::PointerMoved(cell_pt),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -360.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        scroll_samples.push(ms);
    }
    let scroll = StatsSummary::from_ms(scroll_samples);

    // Selection stats on ~50k cells (via grid row range click + refresh_selection_stats)
    let ds = doc.data_sheet(&SheetId::from("mv-100k")).ok_or("missing mv-100k")?;
    let mut grid_for_stats = curvebook_ui_egui::GridViewState::default();
    let row_span = (50_000 / 6).min(rows_100k).saturating_sub(1);
    let t_stats = Instant::now();
    grid_for_stats.select_row_click(ds, 0, false, false);
    grid_for_stats.select_row_click(ds, row_span, true, false);
    let stats_ms = t_stats.elapsed().as_secs_f64() * 1000.0;
    let qs_n = grid_for_stats.quick_stats.as_ref().map_or(0, |s| s.numeric_count);
    assert_eq!(qs_n, (row_span + 1) * 6);

    // Frame hit-test cost (pointer moves across grid rows)
    let mut hit_samples = Vec::new();
    for k in 0..20 {
        let pt = egui::pos2(450.0, 180.0 + ((k * 17) % 400) as f32);
        hit_samples.push(step_native_frame(&ctx, &mut app, &mut sim, vec![egui::Event::PointerMoved(pt)]));
    }
    let hit = StatsSummary::from_ms(hit_samples);
    let _ = win_id;

    println!(
        "[R17.3 Scenario B | {rows_100k}-row MV Grid] init={init_ms:.3}ms | scroll_frame mean={:.3}ms p95={:.3}ms max={:.3}ms | hover/hit mean={:.3}ms | col_stats_50k={stats_ms:.3}ms",
        scroll.mean_ms, scroll.p95_ms, scroll.max_ms, hit.mean_ms
    );
    if is_release {
        assert!(scroll.mean_ms < 10.0, "Scenario B 100k grid scroll regressed: mean={:.3}ms", scroll.mean_ms);
    }
    Ok(())
}

fn profile_scenario_c(is_release: bool) -> Result<(), Box<dyn std::error::Error>> {
    // --- (c) Multi-panel Page Layouts (compile + raster + interactive drag frame) ---
    let panel_count = 16usize;
    let doc = build_multipanel_layout_doc(panel_count, if is_release { 400 } else { 80 })?;
    let ls = doc.layout_sheet(&SheetId::from("lay-multi")).ok_or("missing lay-multi")?;

    let mut compile_samples = Vec::new();
    let mut last_scene = None;
    for _ in 0..5 {
        let t0 = Instant::now();
        let sc = compile_layout(ls, &doc)?;
        compile_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
        last_scene = Some(sc);
    }
    let comp = StatsSummary::from_ms(compile_samples);

    let scene = last_scene.ok_or("missing compiled layout scene")?;
    let t_rast = Instant::now();
    let img = render_scene(&scene, RenderTargetSpec { width_px: 1200, height_px: 900, dpi: 144 })?;
    let rast_ms = t_rast.elapsed().as_secs_f64() * 1000.0;
    assert!(img.width_px == 1200);

    // Interactive UI frame while layout HUD renders (simulating interactive layout drag frame!)
    let mut ui_app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
    ui_app.sync_views();
    ui_app.egui_views.canvas_view.synchronous_raster = true;
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut views = std::mem::take(&mut ui_app.egui_views);
    let mut sim = 1.0_f64;
    let mut frame_samples = Vec::new();
    for pass in 0..10 {
        sim += 1.0 / 60.0;
        // Simulate active panel drag on frames 2..8
        if (2..=8).contains(&pass) {
            views.canvas_view.layout_drag = Some(curvebook_ui_egui::egui_shell::canvas_view::LayoutPanelDragState {
                sheet_id: SheetId::from("lay-multi"),
                item_index: 0,
                start_scene_pt: [50.0, 50.0],
                initial_bounds_pt: curvebook_doc::RectPt::new(24.0, 24.0, 130.0, 105.0),
                current_bounds_pt: curvebook_doc::RectPt::new(24.0 + (pass as f64) * 3.0, 24.0 + (pass as f64) * 2.0, 130.0, 105.0),
                resizing_corner: false,
            });
        }
        let t0 = Instant::now();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(sim),
            focused: true,
            ..Default::default()
        };
        let mut meta = FrameInspectMeta::new();
        let mut intents = PendingUiIntents::new();
        let mut out = ctx.run_ui(raw_input, |ui| {
            let tokens = Tokens::from_theme_spec(&ui_app.theme, &ui_app.locale);
            tokens.apply_to_ctx(ui.ctx());
            tokens.apply_to_ui(ui);
            meta.begin_pass(ui.ctx());
            let _ = render_egui_shell_frame(ui, &ui_app, &mut views, &mut meta, &mut intents);
        });
        out.textures_delta.clear();
        let _ = intents.commit_if_settled(&mut ui_app, out.platform_output.requested_discard());
        if pass >= 1 {
            frame_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let drag = StatsSummary::from_ms(frame_samples);
    println!(
        "[R17.3 Scenario C | {panel_count}-panel Layout] compile mean={:.3}ms | render_1200x900={rast_ms:.3}ms | interactive_drag_frame mean={:.3}ms p95={:.3}ms max={:.3}ms",
        comp.mean_ms, drag.mean_ms, drag.p95_ms, drag.max_ms
    );
    if is_release {
        assert!(drag.mean_ms < 20.0, "Scenario C layout drag frame regressed: mean={:.3}ms", drag.mean_ms);
    }
    Ok(())
}

fn profile_scenario_d(is_release: bool, rows_100k: usize) -> Result<(), Box<dyn std::error::Error>> {
    // --- (d) Rapid Undo/Redo Bursts (100k-row doc + checkpoint, and 57-sheet doc + 5 checkpoints + atomic groups) ---
    let doc = build_100k_grid_doc(rows_100k)?;
    let mut s = Session::from_document(doc);
    // 1 realistic checkpoint on a 600k-cell doc (~20-30 MiB snapshot_json, under 64 MiB limit).
    s.execute("project.createCheckpoint", json!({ "name": "CP-0", "note": "stress" }))?;
    let n_ops = if is_release { 25 } else { 10 };
    let mut edit_samples = Vec::with_capacity(n_ops);
    for i in 0..n_ops {
        let t0 = Instant::now();
        s.execute("table.setCell", json!({ "sheet_id": "mv-100k", "row": i, "col": 0, "value": (i as f64) * 3.25 }))?;
        edit_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let mut undo_samples = Vec::with_capacity(n_ops);
    for _ in 0..n_ops {
        let t0 = Instant::now();
        s.execute("edit.undo", json!({}))?;
        undo_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let mut redo_samples = Vec::with_capacity(n_ops);
    for _ in 0..n_ops {
        let t0 = Instant::now();
        s.execute("edit.redo", json!({}))?;
        redo_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let mut atomic_samples = Vec::with_capacity(5);
    for k in 0..5 {
        let t0 = Instant::now();
        s.execute_atomic_group(|sess| sess.execute("table.setCell", json!({ "sheet_id": "mv-100k", "row": k, "col": 1, "value": 42.0 + k as f64 })))?;
        atomic_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    let ed = StatsSummary::from_ms(edit_samples);
    let un = StatsSummary::from_ms(undo_samples);
    let re = StatsSummary::from_ms(redo_samples);
    let at = StatsSummary::from_ms(atomic_samples);
    println!(
        "[R17.3 Scenario D1 | {rows_100k}-row + 1 checkpoint Undo/Redo] setCell mean={:.3}ms | undo mean={:.3}ms max={:.3}ms | redo mean={:.3}ms max={:.3}ms | atomic_group mean={:.3}ms",
        ed.mean_ms, un.mean_ms, un.max_ms, re.mean_ms, re.max_ms, at.mean_ms
    );

    // Also burst undo/redo on 57-sheet project with 5 saved checkpoints!
    let doc60 = build_60_sheet_project(if is_release { 400 } else { 50 })?;
    let mut s60 = Session::from_document(doc60);
    for cp in 0..5 {
        s60.execute("project.createCheckpoint", json!({ "name": format!("CP-{cp}"), "note": "stress" }))?;
    }
    let mut e60 = Vec::with_capacity(15);
    for i in 0..15 {
        let t0 = Instant::now();
        s60.execute("table.setCell", json!({ "sheet_id": "ds-0", "row": i, "col": 0, "value": 100.0 + i as f64 }))?;
        e60.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let mut u60 = Vec::with_capacity(15);
    for _ in 0..15 {
        let t0 = Instant::now();
        s60.execute("edit.undo", json!({}))?;
        u60.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let mut r60 = Vec::with_capacity(15);
    for _ in 0..15 {
        let t0 = Instant::now();
        s60.execute("edit.redo", json!({}))?;
        r60.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let e60_stats = StatsSummary::from_ms(e60);
    println!(
        "[R17.3 Scenario D2 | 57-sheet + 5 checkpoints] setCell mean={:.3}ms | undo mean={:.3}ms | redo mean={:.3}ms",
        e60_stats.mean_ms,
        StatsSummary::from_ms(u60).mean_ms,
        StatsSummary::from_ms(r60).mean_ms
    );
    if is_release {
        assert!(ed.mean_ms < 20.0, "Scenario D1 100k+cp setCell regressed: mean={:.3}ms", ed.mean_ms);
        assert!(e60_stats.mean_ms < 3.0, "Scenario D2 57-sheet+5cp setCell regressed: mean={:.3}ms", e60_stats.mean_ms);
    }
    Ok(())
}

fn profile_scenario_e(is_release: bool, rows_100k: usize) -> Result<(), Box<dyn std::error::Error>> {
    // --- (e) Live Cell Typing / Editing on Large Sheet (key-to-frame + commit) ---
    let doc = build_100k_grid_doc(rows_100k)?;
    let (ctx, mut app, win_id) = build_native_bench_app(doc)?;
    {
        let ui_app = app.runtime_mut().window_mut(win_id)?.app_mut();
        ui_app.session_mut().execute("project.createCheckpoint", json!({ "name": "CP-0", "note": "live" }))?;
        ui_app.sync_views();
    }
    let mut sim = 1.0_f64;
    let _ = step_native_frame(&ctx, &mut app, &mut sim, vec![]);

    let n_commits = if is_release { 15 } else { 5 };
    let mut commit_frame_samples = Vec::with_capacity(n_commits);
    for r in 0..n_commits {
        let t0 = Instant::now();
        {
            let ui_app = app.runtime_mut().window_mut(win_id)?.app_mut();
            ui_app.execute_command("table.setCell", json!({ "sheet_id": "mv-100k", "row": r, "col": 0, "value": 99.5 + (r as f64) }))?;
        }
        let _ = step_native_frame(&ctx, &mut app, &mut sim, vec![]);
        commit_frame_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let comm = StatsSummary::from_ms(commit_frame_samples);
    println!(
        "[R17.3 Scenario E | {rows_100k}-row Live Cell Commit+Sync+Frame (with 1 checkpoint)] mean={:.3}ms p50={:.3}ms p95={:.3}ms max={:.3}ms",
        comm.mean_ms, comm.p50_ms, comm.p95_ms, comm.max_ms
    );
    assert!(
        comm.mean_ms < if is_release { 25.0 } else { 250.0 },
        "Scenario E 100k Live Cell Commit+Sync+Frame regressed: mean={:.3}ms",
        comm.mean_ms
    );
    Ok(())
}

fn profile_scenario_f(is_release: bool) -> Result<(), Box<dyn std::error::Error>> {
    // --- (f) 1M-cell (1000x1000) Column repeated interactive cell edits + navigation ---
    let dim = if is_release { 1_000 } else { 80 };
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, dim, dim)?;
    for (c_idx, ds_col) in col.data_sets.iter_mut().enumerate() {
        ds_col.title = format!("Col {}", c_idx + 1);
        for (r_idx, row) in ds_col.cells.iter_mut().enumerate() {
            row[0] = Cell::number(((c_idx % 100) as f64) * 0.25 + (r_idx as f64) * 0.1);
        }
    }
    let ds = DataSheet::new(SheetId::from("col-1m"), "Col 1M", TableArchetype::Column(col))?;
    doc.add_data_sheet(ds)?;
    doc.select_in_navigator(&SheetId::from("col-1m"))?;
    let mut s = Session::from_document(doc);
    let mut edit_samples = Vec::with_capacity(10);
    for k in 0..10 {
        let t0 = Instant::now();
        s.execute("table.setCell", json!({ "sheet_id": "col-1m", "row": k, "col": k % dim, "value": 123.5 + (k as f64) }))?;
        edit_samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let first_ms = edit_samples.first().copied().unwrap_or(0.0);
    let steady = StatsSummary::from_ms(edit_samples.get(1..).unwrap_or(&[]).to_vec());
    println!(
        "[R17.3 Scenario F | {}x{} Column setCell] first={first_ms:.3}ms | steady mean={:.3}ms max={:.3}ms",
        dim, dim, steady.mean_ms, steady.max_ms
    );
    if is_release {
        assert!(first_ms < 45.0, "1M Column first setCell regressed: {first_ms:.3}ms");
        assert!(steady.max_ms < 2.0, "1M Column steady setCell regressed: max={:.3}ms", steady.max_ms);
    }

    Ok(())
}
