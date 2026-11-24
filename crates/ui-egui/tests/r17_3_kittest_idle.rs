#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Deterministic `egui_kittest::Harness` + `UiApp` idle-repaint regression tests (`R17.3`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use curvebook_doc::{LayoutItem, LayoutSheet, LayoutSpec, RectPt, SheetId};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::build_driver_font_definitions;
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens, UiBackend};
use egui_kittest::Harness;
use serde_json::json;

struct KitIdleState {
    app: UiApp,
    views: EguiShellViewState,
}

fn build_harness(mut app: UiApp) -> (Harness<'static, KitIdleState>, Arc<AtomicUsize>) {
    app.sync_views();
    let repaints = Arc::new(AtomicUsize::new(0));
    let repaints_cb = Arc::clone(&repaints);
    let state = KitIdleState { app, views: EguiShellViewState::default() };
    let harness = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).allow_missing_glyphs().build_ui_state(
        |ui, st| {
            let tokens = Tokens::from_theme_spec(&st.app.theme, &st.app.locale);
            tokens.apply_to_ctx(ui.ctx());
            tokens.apply_to_ui(ui);
            let mut meta = FrameInspectMeta::new();
            meta.begin_pass(ui.ctx());
            let mut intents = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, &mut meta, &mut intents);
            let _ = intents.commit_if_settled(&mut st.app, ui.ctx().will_discard());
        },
        state,
    );
    harness.ctx.set_fonts(build_driver_font_definitions());
    harness.ctx.set_request_repaint_callback(move |_info| {
        repaints_cb.fetch_add(1, Ordering::Relaxed);
    });
    (harness, repaints)
}

fn assert_kittest_settles_to_zero_repaints(label: &str, harness: &mut Harness<'_, KitIdleState>, repaints: &AtomicUsize, wait_async: bool) {
    // Allow initial layout & optional ScrollArea animation / async raster worker to finish.
    if wait_async {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            harness.step();
            let in_flight = harness.state().views.canvas_view.slot.rendering_in_flight
                || harness.state().views.center_stage.secondary_canvas_view.slot.rendering_in_flight;
            if !in_flight {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    harness.run_ok();
    for _ in 0..8 {
        harness.input_mut().time = Some(harness.input().time.unwrap_or(1.0) + 0.05);
        harness.step();
    }

    for pass in 0..8 {
        harness.input_mut().time = Some(harness.input().time.unwrap_or(1.0) + 0.15);
        let before = repaints.load(Ordering::Relaxed);
        harness.step();
        let after = repaints.load(Ordering::Relaxed);
        assert_eq!(
            after - before,
            0,
            "[{label}] kittest idle pass {pass} emitted {} repaint callback(s) (causes={:?})",
            after - before,
            harness.ctx.repaint_causes()
        );
    }
}

#[test]
fn egui_kittest_surfaces_and_idle_dialog_emit_zero_ongoing_repaints() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("sheet.createData", json!({ "sheet_id": "xy-1", "kind": "xy", "cols": 2, "rows": 25, "auto_graph": false }))?;
    s.execute("table.setXCells", json!({ "sheet_id": "xy-1", "values": [1.0, 2.0, 3.0, 4.0] }))?;
    s.execute("table.setCells", json!({ "sheet_id": "xy-1", "rows": [[2.0, 3.0], [4.0, 5.0], [6.0, 7.0], [8.0, 9.0]] }))?;
    s.execute("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "title": "Curve" }))?;

    // 1. Data Grid
    {
        let mut doc = s.document().clone();
        doc.select_in_navigator(&SheetId::from("xy-1"))?;
        let app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
        let (mut harness, repaints) = build_harness(app);
        assert_kittest_settles_to_zero_repaints("kittest_data_grid", &mut harness, &repaints, false);
    }

    // 2. Graph Canvas
    {
        let mut doc = s.document().clone();
        doc.select_in_navigator(&SheetId::from("g-1"))?;
        let app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
        let (mut harness, repaints) = build_harness(app);
        assert_kittest_settles_to_zero_repaints("kittest_graph_canvas", &mut harness, &repaints, true);
    }

    // 3. Layout Canvas
    {
        let mut doc = s.document().clone();
        let layout = LayoutSheet::new(
            SheetId::from("lay-1"),
            "Figure 1",
            LayoutSpec {
                items: vec![LayoutItem::LiveGraph {
                    id: String::from("p1"),
                    graph_sheet_id: SheetId::from("g-1"),
                    bounds_pt: RectPt::new(20.0, 20.0, 240.0, 180.0),
                    scale_percent: 100.0,
                    badge_override: None,
                }],
                ..LayoutSpec::default()
            },
        )?;
        doc.add_layout_sheet(layout)?;
        doc.select_in_navigator(&SheetId::from("lay-1"))?;
        let app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
        let (mut harness, repaints) = build_harness(app);
        assert_kittest_settles_to_zero_repaints("kittest_layout_canvas", &mut harness, &repaints, true);
    }

    // 4. Idle AnalyzeSheet dialog (focuses search TextEdit cleanly without cursor blink wakeups)
    {
        let mut doc = s.document().clone();
        doc.select_in_navigator(&SheetId::from("xy-1"))?;
        let mut app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
        app.canvas_dialogs.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::from("xy-1") };
        let (mut harness, repaints) = build_harness(app);
        assert_kittest_settles_to_zero_repaints("kittest_analyze_dialog", &mut harness, &repaints, false);
    }

    Ok(())
}
