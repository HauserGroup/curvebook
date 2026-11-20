#![forbid(unsafe_code)]

//! Phase 1 (`R15.6` wave A) human-path regression tests covering the shared `egui` GUI intent
//! pipeline (`R15-GUI-02`, `R15-GUI-04`, `R15-GUI-05`, `R15-GUI-07`, `R15-GUI-13`, `R15-GUI-18`,
//! `R15-GUI-23`), driven through [`EguiHumanPathDriver`].

use std::time::{Duration, Instant};

use curvebook_doc::{CellCoord, CellValue, SheetId};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, canvas_async_worker_stats, render_canvas_view};
use curvebook_ui_egui::egui_shell::command_palette::filter_palette_commands;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_unit_field};
use curvebook_ui_egui::egui_shell::welcome_hub::WelcomeLeftMode;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectEntryInput, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::inspect::{ZLayer, inspect_ui_tree};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{DESKTOP_HOST_ROUTED_COMMANDS, UiApp, is_desktop_host_routed_command};
use egui::{Key, Modifiers, Pos2, Vec2};
use serde_json::json;

fn seed_data_and_graph_app() -> UiApp {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    let _ = app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-1",
            "title": "Dose Response",
            "kind": "xy",
            "data_sets": 2,
            "rows": 4,
            "auto_graph": false
        }),
    );
    let _ = app.execute_command("table.setXCells", json!({ "sheet_id": "ds-1", "values": [1.0, 2.0, 4.0, 8.0] }));
    let _ = app.execute_command("table.setCells", json!({ "sheet_id": "ds-1", "rows": [[10.0, 12.0], [25.0, 28.0], [50.0, 52.0], [85.0, 88.0]] }));
    let _ = app.execute_command("graph.create", json!({ "sheet_id": "gs-1", "data_sheet_id": "ds-1", "title": "Dose Curve" }));
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "ds-1" }));
    app.sync_views();
    app
}

#[test]
fn a1_partial_batch_error_still_syncs_views_updates_mutation_seq_and_surfaces_banner() -> Result<(), String> {
    struct A1State {
        app: UiApp,
        carried_intents: PendingUiIntents,
        last_error: Option<String>,
    }

    let mut app = UiApp::new();
    app.set_gui_only(true);
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-a1", "title": "Batch Table", "kind": "column", "rows": 3, "cols": 2, "auto_graph": false }),
    )
    .map_err(|e| e.to_string())?;
    app.sync_views();
    let doc_seed = app.document().clone();
    app.egui_views.sync_history_with_document(&doc_seed);
    let seq_before = app.mutation_seq();

    let state = A1State { app, carried_intents: PendingUiIntents::new(), last_error: None };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        state,
        ViewportMetrics::new(640.0, 360.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            egui::CentralPanel::default().show(ui, |ui| {
                let btn = ui.button("Apply Mixed Batch");
                meta.record_response(
                    ui,
                    &btn,
                    FrameInspectEntryInput {
                        widget_id: "test.mixed_batch",
                        role: "button",
                        label: "Apply Mixed Batch",
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if btn.clicked() {
                    // 1. Succeeding document mutations (create sheet + set cell)
                    pass_intents.push_engine(
                        "sheet.createData",
                        json!({ "sheet_id": "ds-a1-new", "title": "Synced After Error", "kind": "column", "rows": 2, "cols": 2, "auto_graph": false }),
                    );
                    pass_intents.push_engine(
                        "table.setCells",
                        json!({
                            "sheet_id": "ds-a1",
                            "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 42.5 }]
                        }),
                    );
                    // 2. Failing command (invalid sheet_id)
                    pass_intents.push_engine("sheet.delete", json!({ "sheet_id": "does-not-exist" }));
                    // 3. Trailing intent AFTER the failure — must NOT be silently dropped!
                    pass_intents.push(UiIntent::SetUiScale(1.25));
                }
            });
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled
            && !st.carried_intents.is_empty()
            && let Err(err) = st.carried_intents.commit_if_settled(&mut st.app, false)
        {
            st.last_error = Some(err.to_string());
        }
    });
    driver.run_settled();

    driver.click_widget_id("test.mixed_batch")?;

    let st = driver.state();
    assert!(st.app.mutation_seq() > seq_before, "mutation_seq must advance for succeeding intent");
    let cell = st.app.document().data_sheet(&SheetId::from("ds-a1")).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).ok_or("missing mutated cell")?;
    assert_eq!(cell.value, CellValue::Number(42.5));
    // Trailing intent after the failing intent must still execute.
    assert!((st.app.ui_scale() - 1.25).abs() < 1e-4, "trailing intent after error must not be dropped");
    // Views (`app.nav.entries`) and navigation history must be synced even when the batch returned an error.
    assert!(
        st.app.nav.entries.iter().any(|e| e.sheet_id == SheetId::from("ds-a1-new")),
        "app.sync_views() must update navigator entries even on partial batch error"
    );
    assert!(st.app.egui_views.can_go_back(), "sync_history_with_document must record newly created active sheet even on partial batch error");
    // First error must surface in both the commit result and `status_banner`.
    assert!(st.last_error.is_some(), "batch commit must return first error");
    assert!(st.app.canvas_dialogs.status_banner.is_some(), "app status_banner must record first error");
    let snap = inspect_ui_tree(&st.app, None, false);
    assert!(snap.status_banner.is_some(), "UiTreeSnapshot must surface status_banner");
    Ok(())
}

#[test]
fn a2_discarded_pass_preserves_grid_tab_click_and_unit_spinner_transitions() -> Result<(), String> {
    // Part 1: Clicking a tab that mounts a first-time `egui::Grid` causes `ctx.will_discard() == true`
    // on pass 0 (when input events arrive). Carrying & deduplicating intents via `merge_pass` preserves
    // the pass-0 click intent and commits strictly once when pass 1 settles.
    struct DiscardedGridState {
        app: UiApp,
        show_grid_tab: bool,
        discard_count: usize,
        carried_intents: PendingUiIntents,
    }

    let mut app = UiApp::new();
    app.set_gui_only(true);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        DiscardedGridState { app, show_grid_tab: false, discard_count: 0, carried_intents: PendingUiIntents::new() },
        ViewportMetrics::new(640.0, 360.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            egui::CentralPanel::default().show(ui, |ui| {
                let tab_btn = ui.button("Open Grid Tab");
                meta.record_response(
                    ui,
                    &tab_btn,
                    FrameInspectEntryInput {
                        widget_id: "test.open_grid_tab",
                        role: "button",
                        label: "Open Grid Tab",
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if tab_btn.clicked() {
                    st.show_grid_tab = true;
                    pass_intents.push_engine(
                        "sheet.createData",
                        json!({ "sheet_id": "ds-from-discard", "title": "Survived Discard", "kind": "column", "auto_graph": false }),
                    );
                }
                if st.show_grid_tab {
                    // First-time `egui::Grid` requests a sizing discard pass on pass 0!
                    egui::Grid::new("first_time_discard_grid").show(ui, |ui| {
                        ui.label("Factor");
                        ui.label("df");
                        ui.end_row();
                        ui.label("Treatment");
                        ui.label("2");
                        ui.end_row();
                    });
                }
            });
            if ui.ctx().will_discard() {
                st.discard_count += 1;
            }
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let _ = st.carried_intents.commit_if_settled(&mut st.app, false);
        }
    });
    driver.run_settled();

    driver.click_widget_id("test.open_grid_tab")?;
    assert!(driver.state().show_grid_tab);
    assert!(driver.state().discard_count >= 1, "first-time egui::Grid must trigger at least one discarded pass");
    assert_eq!(driver.state().app.document().data_sheets.len(), 1, "pass-0 click intent must survive discarded pass and commit once");
    assert_eq!(driver.state().app.session().undo_depth(), 1, "must commit strictly once without duplicate undo");

    // Part 2: `unit_spinners` local state transitions (unit badge click, drag end, Escape) must NOT be
    // dropped when `ctx.will_discard()` is true on pass 0.
    struct SpinnerDiscardState {
        spinner: ScrubUnitFieldState,
        value_pt: f64,
        force_discard_next_pass: bool,
        discard_seen: usize,
        committed_values: Vec<f64>,
        badge_center: Option<Pos2>,
        chip_center: Option<Pos2>,
    }

    let mut sp_driver = EguiHumanPathDriver::with_tokens_and_fonts(
        SpinnerDiscardState {
            spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            value_pt: 72.0,
            force_discard_next_pass: false,
            discard_seen: 0,
            committed_values: Vec::new(),
            badge_center: None,
            chip_center: None,
        },
        ViewportMetrics::new(420.0, 180.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            if st.force_discard_next_pass {
                st.force_discard_next_pass = false;
                ui.ctx().request_discard("simulated pass-0 discard for unit_spinner");
            }
            if ui.ctx().will_discard() {
                st.discard_seen += 1;
            }
            let tokens = Tokens::get(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                ui.set_max_width(220.0);
                let cfg = ScrubUnitFieldConfig::new("test.spinner.width", "Width", 10.0, 500.0);
                let out = render_scrub_unit_field(ui, st.value_pt, &mut st.spinner, cfg, &tokens, meta);
                let r = out.response.rect;
                // Unit badge sits at the right edge of the spinner frame; value chip sits to its left.
                st.badge_center = Some(Pos2::new(r.right() - 18.0, r.center().y));
                st.chip_center = Some(Pos2::new(r.right() - 56.0, r.center().y));
                if let Some(v) = out.committed_value {
                    st.value_pt = v;
                    st.committed_values.push(v);
                }
            });
        },
    );
    sp_driver.run_settled();

    // 2a. Unit-badge click during a discarded pass must switch `pt` -> `mm`.
    let badge_pos = sp_driver.state().badge_center.ok_or("missing badge center")?;
    sp_driver.state_mut().force_discard_next_pass = true;
    sp_driver.click_at(badge_pos)?;
    assert!(sp_driver.state().discard_seen >= 1);
    assert_eq!(sp_driver.state().spinner.unit, ScrubUnitKind::Millimeters, "unit badge click on discarded pass must switch pt -> mm");

    // 2b. Drag release during a discarded pass must clear `state.dragging` and emit committed_value.
    let chip_pos = sp_driver.state().chip_center.ok_or("missing chip center")?;
    sp_driver.state_mut().spinner.unit = ScrubUnitKind::Points;
    sp_driver.state_mut().committed_values.clear();
    sp_driver.run_settled();
    sp_driver.state_mut().force_discard_next_pass = true;
    sp_driver.drag(chip_pos, chip_pos + Vec2::new(24.0, 0.0))?;
    assert!(!sp_driver.state().spinner.dragging, "drag release on discarded pass must not leave spinner stuck dragging");
    assert!(!sp_driver.state().committed_values.is_empty(), "drag release on discarded pass must commit value");

    // 2c. Escape in text editor during a discarded pass must exit editing mode.
    sp_driver.click_at(chip_pos)?;
    assert!(sp_driver.state().spinner.editing, "click on value chip must enter editing mode");
    sp_driver.state_mut().force_discard_next_pass = true;
    sp_driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!sp_driver.state().spinner.editing, "Escape on discarded pass must exit editing mode");
    Ok(())
}

#[test]
fn a3_host_routed_parity_welcome_open_path_and_export_emf_intent() -> Result<(), String> {
    // 1. Canonical host-routed table parity (`R15-GUI-07`, `R15-GUI-23`).
    for required in [
        "project.open",
        "project.save",
        "project.saveAs",
        "edit.copy",
        "edit.paste",
        "export.svg",
        "export.pdf",
        "export.png",
        "graph.exportEmf",
        "layout.exportEmf",
        "window.new",
        "window.close",
        "app.quit",
        "file.quit",
    ] {
        assert!(is_desktop_host_routed_command(required), "missing host-routed command `{required}`");
        assert!(DESKTOP_HOST_ROUTED_COMMANDS.contains(&required));
    }

    // 2. Welcome Hub Open pane carries `{"path": "..."}` in its queued `project.open` intent (`R15-GUI-13`),
    // and on a dirty headless session `commit_if_settled` refuses to silently discard unsaved edits.
    struct CaptureShellState {
        app: UiApp,
        views: EguiShellViewState,
        captured_intents: Vec<UiIntent>,
        carried_intents: PendingUiIntents,
        commit_live: bool,
        last_err: Option<String>,
    }

    let mut app = seed_data_and_graph_app();
    app.egui_views.launcher_open = true;
    app.egui_views.welcome_hub.left_mode = WelcomeLeftMode::OpenFile;
    app.egui_views.welcome_hub.open_file_path_draft = String::from("/tmp/sample-assay.curvebook");
    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);

    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        CaptureShellState { app, views, captured_intents: Vec::new(), carried_intents: PendingUiIntents::new(), commit_live: false, last_err: None },
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass_intents);
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled && !st.carried_intents.is_empty() {
            st.captured_intents.extend(st.carried_intents.intents().iter().cloned());
            if st.commit_live {
                st.app.egui_views = st.views.clone();
                if let Err(err) = st.carried_intents.commit_if_settled(&mut st.app, false) {
                    st.last_err = Some(err.to_string());
                }
                st.views = st.app.egui_views.clone();
            } else {
                st.carried_intents.clear();
            }
        }
    });
    driver.run_settled();

    driver.click_widget_id("welcome.btn.open_file")?;
    let open_intent = driver.state().captured_intents.iter().find_map(|intent| match intent {
        UiIntent::EngineCommand { command_id, params } if command_id == "project.open" => Some(params.clone()),
        _ => None,
    });
    assert_eq!(
        open_intent,
        Some(json!({ "path": "/tmp/sample-assay.curvebook" })),
        "Welcome Hub Open must queue project.open with explicit path param"
    );

    // Dirty headless commit guard: queuing `project.open` on a dirty session without `force: true` must fail cleanly.
    let mut dirty_check = PendingUiIntents::new();
    dirty_check.push_engine("project.open", json!({ "path": "/tmp/sample-assay.curvebook" }));
    assert!(driver.state().app.session().is_dirty());
    let dirty_err = dirty_check.commit_if_settled(&mut driver.state_mut().app, false);
    assert!(dirty_err.is_err(), "dirty headless project.open must not silently discard unsaved changes");

    // 3. Export Figure modal -> EMF format -> Export queues host-routed `graph.exportEmf` with `sheet_id` (`R15-GUI-23`).
    driver.state_mut().captured_intents.clear();
    driver.state_mut().views.launcher_open = false;
    driver.state_mut().app.egui_views.launcher_open = false;
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::ExportFigure { sheet_id: SheetId::from("gs-1") };
    driver.state_mut().views.app_modals.export_emf_selected = true;
    driver.run_settled();

    driver.click_widget_id("modal.ok")?;
    let emf_intent = driver.state().captured_intents.iter().find_map(|intent| match intent {
        UiIntent::EngineCommand { command_id, params } if command_id == "graph.exportEmf" => Some((command_id.clone(), params.clone())),
        _ => None,
    });
    let (emf_cmd, emf_params) = emf_intent.ok_or("Export modal with EMF selected must queue graph.exportEmf")?;
    assert!(is_desktop_host_routed_command(&emf_cmd), "graph.exportEmf must be host-routed so desktop opens native Save dialog");
    assert_eq!(emf_params, json!({ "sheet_id": "gs-1" }));
    Ok(())
}

#[test]
fn a4_command_palette_routes_parameterized_commands_and_hides_unrunnable() -> Result<(), String> {
    struct PaletteShellState {
        app: UiApp,
        views: EguiShellViewState,
        carried_intents: PendingUiIntents,
        last_err: Option<String>,
    }

    let app = seed_data_and_graph_app();

    // Verify unrunnable low-level parameterised commands are hidden from the palette (`R15-GUI-18`).
    for hidden_id in ["sheet.rename", "sheet.reorder", "table.setCell", "project.restoreCheckpoint", "info.setConstant", "ui.event.dispatch"] {
        let rows = filter_palette_commands(&app, hidden_id, 24);
        assert!(rows.iter().all(|r| r.command_id != hidden_id), "unrunnable parameterised command `{hidden_id}` must be hidden from palette");
    }

    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        PaletteShellState { app, views, carried_intents: PendingUiIntents::new(), last_err: None },
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass_intents);
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled && !st.carried_intents.is_empty() {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.carried_intents.commit_if_settled(&mut st.app, false) {
                st.last_err = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        }
    });
    driver.run_settled();

    // 1. Open Command Palette via Cmd+K, search `graph.setSpec`, click row -> opens Format Graph modal cleanly!
    let cmd_mod = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    driver.press_egui_key(Key::K, cmd_mod);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    driver.state_mut().views.command_palette.query = String::from("graph.setSpec");
    driver.run_settled();
    driver.click_widget_id("palette.item.0")?;
    assert!(driver.state().last_err.is_none(), "graph.setSpec via palette must not error: {:?}", driver.state().last_err);
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. }),
        "graph.setSpec via palette must open FormatGraph dialog, got {:?}",
        driver.state().app.canvas_dialogs.active_dialog
    );

    // Dismiss FormatGraph and test active-sheet auto-fill (`sheet.duplicate`).
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::None;
    driver.run_settled();
    let sheets_before = driver.state().app.document().total_sheet_count();
    let undo_before = driver.state().app.session().undo_depth();

    driver.press_egui_key(Key::K, cmd_mod);
    driver.state_mut().views.command_palette.query = String::from("sheet.duplicate");
    driver.run_settled();
    driver.click_widget_id("palette.item.0")?;
    assert!(driver.state().last_err.is_none(), "sheet.duplicate via palette must not error: {:?}", driver.state().last_err);
    assert_eq!(driver.state().app.document().total_sheet_count(), sheets_before + 1, "sheet.duplicate via palette must fill active sheet_id");
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);
    Ok(())
}

#[test]
fn a5_async_canvas_raster_reuses_single_worker_and_converges_on_resize_drag() -> Result<(), String> {
    struct CanvasDragState {
        app: UiApp,
        canvas: CanvasViewEguiState,
    }

    let mut app = seed_data_and_graph_app();
    app.execute_command("sheet.select", json!({ "sheet_id": "gs-1" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let canvas = CanvasViewEguiState { synchronous_raster: false, ..CanvasViewEguiState::default() };

    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        CanvasDragState { app, canvas },
        ViewportMetrics::new(600.0, 420.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let tokens = Tokens::get(ui.ctx());
            let mut intents = PendingUiIntents::new();
            egui::CentralPanel::default().show(ui, |ui| {
                let _ = render_canvas_view(
                    ui,
                    &st.app,
                    &mut st.canvas,
                    CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false },
                    meta,
                    &mut intents,
                );
            });
        },
    );

    // Simulate a rapid window resize drag issuing 25 distinct raster dimensions.
    for step in 0..25_u32 {
        let width = 600.0 + f64::from(step) * 20.0;
        let height = 420.0 + f64::from(step) * 8.0;
        driver.set_viewport(ViewportMetrics::new(width, height, 1.0));
    }

    // Pump frames until the single async worker delivers the final resized texture.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        driver.run_settled();
        if !driver.state().canvas.slot.rendering_in_flight && driver.state().canvas.slot.cached_texture.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let stats = canvas_async_worker_stats(driver.ctx()).ok_or("expected initialized async canvas raster worker")?;
    assert_eq!(stats.worker_spawn_count, 1, "must reuse strictly one long-lived worker thread across all resize frames");
    assert!(stats.submitted_count >= 10, "expected multiple resize raster submissions, got {}", stats.submitted_count);
    assert!(!driver.state().canvas.slot.rendering_in_flight, "worker must converge to final resized key");
    assert!(driver.state().canvas.slot.cached_texture.is_some(), "final resized texture must be uploaded");
    let final_w = driver.state().canvas.slot.last_width_px;
    assert!(final_w >= 500, "cached texture must reflect final wider viewport, got {final_w}px");

    // Dropping the driver drops `egui::Context` and joins the worker thread cleanly.
    drop(driver);
    Ok(())
}
