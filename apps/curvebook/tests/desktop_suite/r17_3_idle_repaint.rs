//! Deterministic idle-repaint and power regression tests (`R17.3`).
//!
//! Verifies across `CurveBookNativeApp` (`DesktopHostRuntime` + `egui::Context`) and `egui_kittest`
//! that settled idle frames emit ZERO `request_repaint*` calls and keep `repaint_delay == Duration::MAX`
//! across: Welcome Hub, Data Grids (including after wheel scroll and during inline cell edit),
//! Graph & Layout async canvases (including split view and uninitialized graphs), Navigator/Family
//! hover timers once matured, and idle application/canvas dialogs.
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime, DesktopWindowId};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook_doc::{
    ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, RectPt, SheetId, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_ui_egui::ActiveDialog;
use curvebook_ui_egui::egui_shell::center_stage::ProgressModalState;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::json;

#[derive(Debug, Clone)]
struct FrameTrace {
    repaint_callbacks: usize,
    repaint_delay: Duration,
    requested_discard: bool,
}

fn build_idle_app(doc: Option<ProjectDocument>) -> Result<(egui::Context, CurveBookNativeApp, Arc<AtomicUsize>, DesktopWindowId, f64), String> {
    let viewport = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: true,
        initial_viewport: viewport,
        ..DesktopHostConfig::default()
    });
    let session = match doc {
        Some(d) => Session::from_document(d),
        None => Session::new(),
    };
    let win_id = rt.open_window_with_session(session, viewport, false)?;
    let app = CurveBookNativeApp::new(rt, win_id, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let count = Arc::new(AtomicUsize::new(0));
    let count_cb = Arc::clone(&count);
    ctx.set_request_repaint_callback(move |_info| {
        count_cb.fetch_add(1, Ordering::Relaxed);
    });
    Ok((ctx, app, count, win_id, 1.0))
}

fn step_one(
    ctx: &egui::Context,
    app: &mut CurveBookNativeApp,
    count: &AtomicUsize,
    sim_time: &mut f64,
    dt: f64,
    events: Vec<egui::Event>,
) -> FrameTrace {
    *sim_time += dt;
    let before = count.load(Ordering::Relaxed);
    let raw_input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        time: Some(*sim_time),
        predicted_dt: dt as f32,
        focused: true,
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw_input, |ui| app.step_ui(ui));
    let requested_discard = out.platform_output.requested_discard();
    let repaint_delay = out.viewport_output.get(&egui::ViewportId::ROOT).map_or(Duration::MAX, |v| v.repaint_delay);
    out.textures_delta.clear();
    let after = count.load(Ordering::Relaxed);
    FrameTrace { repaint_callbacks: after - before, repaint_delay, requested_discard }
}

fn wait_for_async_worker_settle(ctx: &egui::Context, app: &mut CurveBookNativeApp, win_id: DesktopWindowId, count: &AtomicUsize, sim_time: &mut f64) {
    let _ = step_one(ctx, app, count, sim_time, 1.0 / 60.0, vec![]);
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(15));
        let t = step_one(ctx, app, count, sim_time, 0.05, vec![]);
        let in_flight = app.runtime().window(win_id).ok().is_some_and(|w| {
            let views = &w.app().egui_views;
            views.canvas_view.slot.rendering_in_flight || views.center_stage.secondary_canvas_view.slot.rendering_in_flight
        });
        if !in_flight && t.repaint_callbacks == 0 && !t.requested_discard && t.repaint_delay == Duration::MAX {
            break;
        }
    }
}

fn assert_settles_to_zero_idle_repaints(
    label: &str,
    ctx: &egui::Context,
    app: &mut CurveBookNativeApp,
    count: &AtomicUsize,
    sim_time: &mut f64,
    settle_steps: usize,
    idle_checks: usize,
) {
    const SETTLE_DT: f64 = 0.05;
    for _ in 0..settle_steps {
        let t = step_one(ctx, app, count, sim_time, SETTLE_DT, vec![]);
        if t.repaint_callbacks == 0 && !t.requested_discard && t.repaint_delay == Duration::MAX {
            break;
        }
    }
    for (idx, dt) in [0.016, 0.08, 0.2, 0.25, 0.5, 1.0].into_iter().cycle().take(idle_checks).enumerate() {
        let t = step_one(ctx, app, count, sim_time, dt, vec![]);
        assert_eq!(
            t.repaint_callbacks,
            0,
            "[{label}] idle frame {idx} unexpectedly triggered {} repaint callback(s) (causes={:?})",
            t.repaint_callbacks,
            ctx.repaint_causes()
        );
        assert!(
            t.repaint_delay == Duration::MAX,
            "[{label}] idle frame {idx} scheduled repaint_delay={:?} instead of Duration::MAX (causes={:?})",
            t.repaint_delay,
            ctx.repaint_causes()
        );
        assert!(!t.requested_discard, "[{label}] idle frame {idx} unexpectedly requested_discard");
    }
}

#[test]
fn welcome_hub_and_data_sheets_settle_to_zero_idle_repaints() -> Result<(), Box<dyn std::error::Error>> {
    // Welcome Hub
    {
        let (ctx, mut app, count, _, mut sim) = build_idle_app(None)?;
        assert_settles_to_zero_idle_repaints("welcome_hub", &ctx, &mut app, &count, &mut sim, 10, 8);
    }

    // Column, MV (with calculated formula), Result, and Info sheets
    let mut s = Session::new();
    s.execute("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 4, "rows": 80, "auto_graph": false }))?;
    s.execute("table.setCells", json!({ "sheet_id": "col-1", "rows": [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0]] }))?;
    s.execute("sheet.createData", json!({ "sheet_id": "mv-1", "kind": "multiple_variables", "cols": 3, "rows": 20, "auto_graph": false }))?;
    s.execute("table.setTitles", json!({ "sheet_id": "mv-1", "titles": ["Dose", "Weight", "Ratio"] }))?;
    s.execute("table.setCells", json!({ "sheet_id": "mv-1", "rows": [[10.0, 2.0], [20.0, 4.0]] }))?;
    s.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "col": 2, "formula": "[Dose] + [Weight]" }))?;
    s.execute("stats.run", json!({ "source_sheet_id": "col-1", "sheet_id": "res-1", "kind": "unpaired_t" }))?;

    let info_id = s.document().info_sheets.first().map(|i| i.id.as_str().to_owned());
    let mut ids = vec![String::from("col-1"), String::from("mv-1"), String::from("res-1")];
    if let Some(id) = info_id {
        ids.push(id);
    }
    for sid in ids {
        let mut doc = s.document().clone();
        doc.select_in_navigator(&SheetId::from(sid.as_str()))?;
        let (ctx, mut app, count, _, mut sim) = build_idle_app(Some(doc))?;
        assert_settles_to_zero_idle_repaints(&format!("sheet:{sid}"), &ctx, &mut app, &count, &mut sim, 12, 8);
    }

    Ok(())
}

#[test]
fn data_grid_scroll_hover_and_inline_edit_settle_to_zero_idle_repaints() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 5, "rows": 120, "auto_graph": false }))?;
    s.execute("table.setCells", json!({ "sheet_id": "col-1", "rows": [[1.5, 2.5], [3.5, 4.5]] }))?;
    let mut doc = s.document().clone();
    doc.select_in_navigator(&SheetId::from("col-1"))?;

    let (ctx, mut app, count, _, mut sim) = build_idle_app(Some(doc))?;
    assert_settles_to_zero_idle_repaints("grid_initial", &ctx, &mut app, &count, &mut sim, 12, 6);

    let cell_pt = app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?.clipped_rect.center();

    // Mouse wheel scroll -> animates/fades scrollbar briefly, then settles cleanly to ZERO ongoing repaints.
    let _ = step_one(
        &ctx,
        &mut app,
        &count,
        &mut sim,
        1.0 / 60.0,
        vec![
            egui::Event::PointerMoved(cell_pt),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -90.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_settles_to_zero_idle_repaints("grid_after_scroll", &ctx, &mut app, &count, &mut sim, 20, 8);

    // Click a currently visible grid cell and press F2 to leave inline cell TextEdit focused while idle -> ZERO blinking/repaints!
    let cell_pt = app
        .inspect_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.starts_with("grid.cell."))
        .ok_or("missing visible grid.cell entry after scroll")?
        .clipped_rect
        .center();
    let _ = step_one(
        &ctx,
        &mut app,
        &count,
        &mut sim,
        0.05,
        vec![
            egui::Event::PointerMoved(cell_pt),
            egui::Event::PointerButton { pos: cell_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: cell_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
    );
    let _ = step_one(
        &ctx,
        &mut app,
        &count,
        &mut sim,
        0.05,
        vec![egui::Event::Key {
            key: egui::Key::F2,
            physical_key: Some(egui::Key::F2),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_settles_to_zero_idle_repaints("grid_inline_edit_idle", &ctx, &mut app, &count, &mut sim, 8, 8);
    Ok(())
}

#[test]
fn graph_layout_split_and_uninitialized_canvases_settle_to_zero_idle_repaints() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("sheet.createData", json!({ "sheet_id": "xy-1", "kind": "xy", "cols": 1, "rows": 5, "auto_graph": false }))?;
    s.execute("table.setXCells", json!({ "sheet_id": "xy-1", "values": [1.0, 2.0, 3.0, 4.0, 5.0] }))?;
    s.execute("table.setCells", json!({ "sheet_id": "xy-1", "rows": [[2.0], [4.0], [6.0], [8.0], [10.0]] }))?;
    s.execute("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "title": "Curve" }))?;

    // 1. Live Graph canvas (async raster worker)
    {
        let mut doc = s.document().clone();
        doc.select_in_navigator(&SheetId::from("g-1"))?;
        let (ctx, mut app, count, win_id, mut sim) = build_idle_app(Some(doc))?;
        wait_for_async_worker_settle(&ctx, &mut app, win_id, &count, &mut sim);
        assert_settles_to_zero_idle_repaints("graph_canvas", &ctx, &mut app, &count, &mut sim, 8, 8);
    }

    // 2. Multi-panel Layout canvas (async raster worker + HUD)
    {
        let mut doc = s.document().clone();
        let layout = LayoutSheet::new(
            SheetId::from("lay-1"),
            "Figure 1",
            LayoutSpec {
                items: vec![
                    LayoutItem::LiveGraph {
                        id: String::from("p1"),
                        graph_sheet_id: SheetId::from("g-1"),
                        bounds_pt: RectPt::new(20.0, 20.0, 240.0, 180.0),
                        scale_percent: 100.0,
                        badge_override: None,
                    },
                    LayoutItem::LiveGraph {
                        id: String::from("p2"),
                        graph_sheet_id: SheetId::from("g-1"),
                        bounds_pt: RectPt::new(280.0, 20.0, 240.0, 180.0),
                        scale_percent: 100.0,
                        badge_override: None,
                    },
                ],
                ..LayoutSpec::default()
            },
        )?;
        doc.add_layout_sheet(layout)?;
        doc.select_in_navigator(&SheetId::from("lay-1"))?;
        let (ctx, mut app, count, win_id, mut sim) = build_idle_app(Some(doc))?;
        wait_for_async_worker_settle(&ctx, &mut app, win_id, &count, &mut sim);
        assert_settles_to_zero_idle_repaints("layout_canvas", &ctx, &mut app, &count, &mut sim, 8, 8);
    }

    // 3. Split View (Data + Graph)
    {
        let mut doc = s.document().clone();
        doc.select_in_navigator(&SheetId::from("xy-1"))?;
        let (ctx, mut app, count, win_id, mut sim) = build_idle_app(Some(doc))?;
        {
            let ui_app = app.runtime_mut().window_mut(win_id)?.app_mut();
            ui_app.split.enabled = true;
            ui_app.split.primary_sheet_id = Some(SheetId::from("xy-1"));
            ui_app.split.secondary_sheet_id = Some(SheetId::from("g-1"));
            ui_app.sync_views();
        }
        wait_for_async_worker_settle(&ctx, &mut app, win_id, &count, &mut sim);
        assert_settles_to_zero_idle_repaints("split_data_graph", &ctx, &mut app, &count, &mut sim, 8, 8);
    }

    // 4. Uninitialized graph canvas
    {
        let mut doc = ProjectDocument::default();
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5)?;
        let ds = DataSheet::new(SheetId::from("col-1"), "Data 1", TableArchetype::Column(col))?;
        let gs = GraphSheet::new_uninitialized_for_data(SheetId::from("g-uninit"), &ds)?;
        doc.add_data_sheet(ds)?;
        doc.add_graph_sheet(gs)?;
        doc.select_in_navigator(&SheetId::from("g-uninit"))?;
        let (ctx, mut app, count, win_id, mut sim) = build_idle_app(Some(doc))?;
        wait_for_async_worker_settle(&ctx, &mut app, win_id, &count, &mut sim);
        assert_settles_to_zero_idle_repaints("uninit_graph", &ctx, &mut app, &count, &mut sim, 8, 8);
    }

    Ok(())
}

#[test]
fn idle_dialogs_and_progress_modal_schedule_zero_ongoing_repaints() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 2, "rows": 5, "auto_graph": false }))?;
    s.execute("table.setCells", json!({ "sheet_id": "col-1", "rows": [[1.0, 3.0], [2.0, 4.0], [3.0, 5.0]] }))?;
    s.execute("stats.run", json!({ "source_sheet_id": "col-1", "sheet_id": "res-1", "kind": "unpaired_t" }))?;
    s.execute("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "col-1", "title": "G1" }))?;
    let mut doc = s.document().clone();
    doc.select_in_navigator(&SheetId::from("col-1"))?;

    let dialog_cases = vec![
        ("about", ActiveDialog::About),
        ("analyze_sheet", ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::from("col-1") }),
        ("command_palette", ActiveDialog::CommandPalette),
        ("format_axes", ActiveDialog::FormatAxes { sheet_id: SheetId::from("g-1") }),
        ("format_graph", ActiveDialog::FormatGraph { graph_sheet_id: SheetId::from("g-1") }),
        ("edit_analysis_params", ActiveDialog::EditAnalysisParameters { result_sheet_id: SheetId::from("res-1") }),
        ("import_reshape", ActiveDialog::ImportReshape),
        ("convert_archetype", ActiveDialog::ConvertArchetype { sheet_id: SheetId::from("col-1") }),
        ("power_and_sample_size", ActiveDialog::PowerAndSampleSize),
        ("export_figure", ActiveDialog::ExportFigure { sheet_id: SheetId::from("g-1") }),
        ("print", ActiveDialog::Print { sheet_id: Some(SheetId::from("g-1")) }),
        ("checkpoints", ActiveDialog::Checkpoints),
        ("randomize_groups", ActiveDialog::RandomizeGroups),
        ("simulate_dist", ActiveDialog::SimulateDistributions),
        ("create_graph", ActiveDialog::CreateGraph { source_sheet_id: Some(SheetId::from("col-1")) }),
    ];

    for (name, dlg) in dialog_cases {
        let (ctx, mut app, count, win_id, mut sim) = build_idle_app(Some(doc.clone()))?;
        {
            let ui_app = app.runtime_mut().window_mut(win_id)?.app_mut();
            if name == "command_palette" {
                let shortcuts = ui_app.shortcuts.clone();
                ui_app.canvas_dialogs.command_palette.sync_shortcuts(&shortcuts);
                ui_app.egui_views.command_palette.just_opened = true;
            }
            ui_app.canvas_dialogs.active_dialog = dlg;
        }
        assert_settles_to_zero_idle_repaints(&format!("dialog:{name}"), &ctx, &mut app, &count, &mut sim, 10, 8);
    }

    // App-managed Preferences and Find & Replace dialogs
    for special in ["preferences", "find_replace"] {
        let (ctx, mut app, count, win_id, mut sim) = build_idle_app(Some(doc.clone()))?;
        {
            let ui_app = app.runtime_mut().window_mut(win_id)?.app_mut();
            if special == "preferences" {
                let shortcuts = ui_app.shortcuts.clone();
                ui_app.egui_views.app_modals.open_preferences(&shortcuts, None);
            } else {
                ui_app.egui_views.app_modals.find_replace_open = true;
            }
        }
        assert_settles_to_zero_idle_repaints(&format!("dialog:{special}"), &ctx, &mut app, &count, &mut sim, 10, 8);
    }

    // Progress modal: indeterminate is throttled (>= 30ms), determinate & closed emit ZERO ongoing repaints.
    {
        let (ctx, mut app, count, win_id, mut sim) = build_idle_app(Some(doc))?;
        app.runtime_mut().window_mut(win_id)?.app_mut().egui_views.progress_modal = Some(ProgressModalState {
            operation_label: String::from("Fitting"),
            detail: String::from("Running..."),
            progress_0_to_1: None,
            cancellable: true,
            cancel_requested: false,
        });
        for _ in 0..4 {
            let _ = step_one(&ctx, &mut app, &count, &mut sim, 0.05, vec![]);
        }
        let t_indet = step_one(&ctx, &mut app, &count, &mut sim, 0.05, vec![]);
        assert!(
            t_indet.repaint_delay >= Duration::from_millis(30) && t_indet.repaint_delay <= Duration::from_millis(40),
            "indeterminate progress modal should throttle pulse to ~30 FPS, got {:?}",
            t_indet.repaint_delay
        );

        if let Some(pm) = app.runtime_mut().window_mut(win_id)?.app_mut().egui_views.progress_modal.as_mut() {
            pm.progress_0_to_1 = Some(1.0);
        }
        assert_settles_to_zero_idle_repaints("progress_determinate", &ctx, &mut app, &count, &mut sim, 8, 6);

        app.runtime_mut().window_mut(win_id)?.app_mut().egui_views.progress_modal = None;
        assert_settles_to_zero_idle_repaints("progress_closed", &ctx, &mut app, &count, &mut sim, 8, 6);
    }

    Ok(())
}

#[test]
fn stationary_navigator_hover_timer_settles_to_zero_idle_repaints() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 2, "rows": 5, "auto_graph": false }))?;
    s.execute("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "col-1", "title": "G1" }))?;
    let mut doc = s.document().clone();
    doc.select_in_navigator(&SheetId::from("col-1"))?;

    let (ctx, mut app, count, _, mut sim) = build_idle_app(Some(doc))?;
    assert_settles_to_zero_idle_repaints("nav_pre_hover", &ctx, &mut app, &count, &mut sim, 10, 4);

    // Find a sheet row inside the Leading Dock navigator (or family entry) and hover stationary for > 0.6s.
    let target_pt = app
        .inspect_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.contains("nav.") && (e.label.contains("G1") || e.label.contains("Column") || e.label.contains("Data")))
        .map(|e| e.clipped_rect.center());
    if let Some(pt) = target_pt {
        let _ = step_one(&ctx, &mut app, &count, &mut sim, 0.05, vec![egui::Event::PointerMoved(pt)]);
        // Advance past the 0.45s family hover preview threshold and verify stationary hover settles cleanly to 0.
        assert_settles_to_zero_idle_repaints("nav_stationary_hover", &ctx, &mut app, &count, &mut sim, 20, 8);
    }

    Ok(())
}

#[test]
fn portfolio_gallery_live_thumbnails_settle_to_zero_idle_repaints() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("sheet.createData", json!({ "sheet_id": "xy-gal", "kind": "xy", "cols": 1, "rows": 4, "auto_graph": false }))?;
    s.execute("table.setXCells", json!({ "sheet_id": "xy-gal", "values": [1.0, 2.0, 3.0, 4.0] }))?;
    s.execute("table.setCells", json!({ "sheet_id": "xy-gal", "rows": [[10.0], [20.0], [30.0], [40.0]] }))?;
    s.execute("graph.create", json!({ "sheet_id": "g-gal", "data_sheet_id": "xy-gal", "title": "Gallery Curve" }))?;
    s.execute("layout.create", json!({ "sheet_id": "lay-gal", "title": "Gallery Layout" }))?;

    let mut doc = s.document().clone();
    doc.select_in_navigator(&SheetId::from("g-gal"))?;
    let (ctx, mut app, count, win_id, mut sim) = build_idle_app(Some(doc))?;
    {
        let ui_app = app.runtime_mut().window_mut(win_id)?.app_mut();
        ui_app.nav.gallery.open = true;
        ui_app.sync_views();
    }
    wait_for_async_worker_settle(&ctx, &mut app, win_id, &count, &mut sim);
    assert_settles_to_zero_idle_repaints("portfolio_gallery_cached_thumbnails", &ctx, &mut app, &count, &mut sim, 10, 8);
    Ok(())
}
