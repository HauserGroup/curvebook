#![forbid(unsafe_code)]

//! Phase 1 (`R15.6` wave A) human-path regression tests covering the shared `egui` GUI intent
//! pipeline (`R15-GUI-02`, `R15-GUI-04`, `R15-GUI-05`, `R15-GUI-07`, `R15-GUI-13`, `R15-GUI-18`,
//! `R15-GUI-23`), driven through [`EguiHumanPathDriver`].

use std::time::{Duration, Instant};

use curvebook_doc::{CellCoord, CellValue, SheetId};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, canvas_async_worker_stats, render_canvas_view};
use curvebook_ui_egui::egui_shell::command_palette::filter_palette_commands;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_unit_field};
use curvebook_ui_egui::egui_shell::welcome_hub::WelcomeLeftMode;
use curvebook_ui_egui::egui_shell::{FrameInspectEntryInput, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::inspect::{ZLayer, inspect_ui_tree};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{DESKTOP_HOST_ROUTED_COMMANDS, UiApp, is_desktop_host_routed_command};
use egui::{Key, Modifiers, Pos2, Vec2};
use serde_json::json;

use crate::common::{ShellTestState, make_shell_driver_with_state};

fn seed_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-1", "title": "Dose Response", "kind": "xy", "cols": 2, "rows": 4, "auto_graph": false }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-1", "values": [1.0, 2.0, 4.0, 8.0] })).map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": "ds-1", "rows": [[10.0, 12.0], [25.0, 28.0], [50.0, 52.0], [85.0, 88.0]] }))
        .map_err(|e| e.to_string())?;
    app.execute_command("graph.create", json!({ "sheet_id": "gs-1", "data_sheet_id": "ds-1", "title": "Dose Curve" })).map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-1" })).map_err(|e| e.to_string())?;
    app.sync_views();
    Ok(app)
}

fn record_btn(ui: &egui::Ui, meta: &mut curvebook_ui_egui::egui_shell::FrameInspectMeta, resp: &egui::Response, widget_id: &str, label: &str) {
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput {
            widget_id,
            role: "button",
            label,
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
}

#[test]
fn a1_partial_batch_error_still_syncs_views_updates_mutation_seq_and_surfaces_banner() -> Result<(), String> {
    struct A1State {
        app: UiApp,
        carried: PendingUiIntents,
        last_error: Option<String>,
    }

    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-a1", "title": "Batch Table", "kind": "column", "rows": 3, "cols": 2, "auto_graph": false }),
    )
    .map_err(|e| e.to_string())?;
    app.sync_views();
    let doc_seed = app.document().clone();
    app.egui_views.sync_history_with_document(&doc_seed);
    let seq_before = app.mutation_seq();

    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        A1State { app, carried: PendingUiIntents::new(), last_error: None },
        ViewportMetrics::new(640.0, 360.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass = PendingUiIntents::new();
            egui::CentralPanel::default().show(ui, |ui| {
                let btn = ui.button("Apply Mixed Batch");
                record_btn(ui, meta, &btn, "test.mixed_batch", "Apply Mixed Batch");
                if btn.clicked() {
                    pass.push_engine("sheet.createData", json!({ "sheet_id": "ds-a1-new", "title": "Synced After Error", "kind": "column", "rows": 2, "cols": 2, "auto_graph": false }));
                    pass.push_engine("table.setCells", json!({ "sheet_id": "ds-a1", "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 42.5 }] }));
                    pass.push_engine("sheet.delete", json!({ "sheet_id": "does-not-exist" }));
                    pass.push(UiIntent::SetUiScale(1.25));
                }
            });
            st.carried.merge_pass(pass);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled
            && !st.carried.is_empty()
            && let Err(err) = st.carried.commit_if_settled(&mut st.app, false)
        {
            st.last_error = Some(err.to_string());
        }
    });
    driver.run_settled();
    driver.click_widget_id("test.mixed_batch")?;

    let st = driver.state();
    assert!(st.app.mutation_seq() > seq_before);
    assert_eq!(
        st.app.document().data_sheet(&SheetId::from("ds-a1")).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).map(|c| &c.value),
        Some(&CellValue::Number(42.5))
    );
    assert!((st.app.ui_scale() - 1.25).abs() < 1e-4);
    assert!(st.app.nav.entries.iter().any(|e| e.sheet_id == SheetId::from("ds-a1-new")));
    assert!(st.app.egui_views.can_go_back() && st.last_error.is_some() && st.app.canvas_dialogs.status_banner.is_some());
    assert!(inspect_ui_tree(&st.app, None, false).status_banner.is_some());
    Ok(())
}

#[test]
fn a2_discarded_pass_preserves_grid_tab_click_and_unit_spinner_transitions() -> Result<(), String> {
    struct DiscardedGridState {
        app: UiApp,
        show_grid_tab: bool,
        discard_count: usize,
        carried: PendingUiIntents,
    }
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        DiscardedGridState { app: UiApp::new(), show_grid_tab: false, discard_count: 0, carried: PendingUiIntents::new() },
        ViewportMetrics::new(640.0, 360.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass = PendingUiIntents::new();
            egui::CentralPanel::default().show(ui, |ui| {
                let btn = ui.button("Open Grid Tab");
                record_btn(ui, meta, &btn, "test.open_grid_tab", "Open Grid Tab");
                if btn.clicked() {
                    st.show_grid_tab = true;
                    pass.push_engine(
                        "sheet.createData",
                        json!({ "sheet_id": "ds-from-discard", "title": "Survived Discard", "kind": "column", "auto_graph": false }),
                    );
                }
                if st.show_grid_tab {
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
            st.carried.merge_pass(pass);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let _ = st.carried.commit_if_settled(&mut st.app, false);
        }
    });
    driver.run_settled();
    driver.click_widget_id("test.open_grid_tab")?;
    assert!(driver.state().show_grid_tab && driver.state().discard_count >= 1);
    assert_eq!((driver.state().app.document().data_sheets.len(), driver.state().app.session().undo_depth()), (1, 1));

    struct SpinnerDiscardState {
        spinner: ScrubUnitFieldState,
        value_pt: f64,
        force_discard: bool,
        discard_seen: usize,
        committed: Vec<f64>,
        badge: Option<Pos2>,
        chip: Option<Pos2>,
    }
    let mut sp = EguiHumanPathDriver::with_tokens_and_fonts(
        SpinnerDiscardState {
            spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            value_pt: 72.0,
            force_discard: false,
            discard_seen: 0,
            committed: Vec::new(),
            badge: None,
            chip: None,
        },
        ViewportMetrics::new(420.0, 180.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            if st.force_discard {
                st.force_discard = false;
                ui.ctx().request_discard("simulated pass-0 discard for unit_spinner");
            }
            if ui.ctx().will_discard() {
                st.discard_seen += 1;
            }
            let tokens = Tokens::get(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                ui.set_max_width(220.0);
                let out = render_scrub_unit_field(
                    ui,
                    st.value_pt,
                    &mut st.spinner,
                    ScrubUnitFieldConfig::new("test.spinner.width", "Width", 10.0, 500.0),
                    &tokens,
                    meta,
                );
                st.badge = Some(Pos2::new(out.response.rect.right() - 18.0, out.response.rect.center().y));
                st.chip = Some(Pos2::new(out.response.rect.right() - 56.0, out.response.rect.center().y));
                if let Some(v) = out.committed_value {
                    st.value_pt = v;
                    st.committed.push(v);
                }
            });
        },
    );
    sp.run_settled();
    let (badge_pos, chip_pos) = (sp.state().badge.ok_or("missing badge")?, sp.state().chip.ok_or("missing chip")?);
    sp.state_mut().force_discard = true;
    sp.click_at(badge_pos)?;
    assert!(sp.state().discard_seen >= 1 && sp.state().spinner.unit == ScrubUnitKind::Millimeters);

    sp.state_mut().spinner.unit = ScrubUnitKind::Points;
    sp.state_mut().committed.clear();
    sp.run_settled();
    sp.state_mut().force_discard = true;
    sp.drag(chip_pos, chip_pos + Vec2::new(24.0, 0.0))?;
    assert!(!sp.state().spinner.dragging && !sp.state().committed.is_empty());

    sp.click_at(chip_pos)?;
    assert!(sp.state().spinner.editing);
    sp.state_mut().force_discard = true;
    sp.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!sp.state().spinner.editing);
    Ok(())
}

#[test]
fn a3_host_routed_parity_welcome_open_path_and_export_emf_intent() -> Result<(), String> {
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
        assert!(is_desktop_host_routed_command(required) && DESKTOP_HOST_ROUTED_COMMANDS.contains(&required), "missing `{required}`");
    }

    let mut app = seed_app()?;
    app.egui_views.launcher_open = true;
    app.egui_views.welcome_hub.left_mode = WelcomeLeftMode::OpenFile;
    app.egui_views.welcome_hub.open_file_path_draft = String::from("/tmp/sample-assay.curvebook");
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = make_shell_driver_with_state(
        ShellTestState::new(app).with_merge_passes().without_auto_commit(),
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        tokens,
    );

    driver.click_widget_id("welcome.btn.open_file")?;
    let open_intent = driver.state().intents.intents().iter().find_map(|i| match i {
        UiIntent::EngineCommand { command_id, params } if command_id == "project.open" => Some(params.clone()),
        _ => None,
    });
    assert_eq!(open_intent, Some(json!({ "path": "/tmp/sample-assay.curvebook" })));

    let mut dirty_check = PendingUiIntents::new();
    dirty_check.push_engine("project.open", json!({ "path": "/tmp/sample-assay.curvebook" }));
    assert!(driver.state().app.session().is_dirty() && dirty_check.commit_if_settled(&mut driver.state_mut().app, false).is_err());

    driver.state_mut().intents.clear();
    driver.state_mut().views.launcher_open = false;
    driver.state_mut().app.egui_views.launcher_open = false;
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::ExportFigure { sheet_id: SheetId::from("gs-1") };
    driver.state_mut().views.app_modals.export_emf_selected = true;
    driver.run_settled();

    driver.click_widget_id("modal.ok")?;
    let (emf_cmd, emf_params) = driver
        .state()
        .intents
        .intents()
        .iter()
        .find_map(|i| match i {
            UiIntent::EngineCommand { command_id, params } if command_id == "graph.exportEmf" => Some((command_id.clone(), params.clone())),
            _ => None,
        })
        .ok_or("missing graph.exportEmf")?;
    assert!(is_desktop_host_routed_command(&emf_cmd) && emf_params == json!({ "sheet_id": "gs-1" }));
    Ok(())
}

#[test]
fn a4_command_palette_routes_parameterized_commands_and_hides_unrunnable() -> Result<(), String> {
    let app = seed_app()?;
    for hidden_id in ["sheet.rename", "sheet.reorder", "table.setCell", "info.setConstant", "ui.event.dispatch"] {
        assert!(filter_palette_commands(&app, hidden_id, 24).iter().all(|r| r.command_id != hidden_id), "`{hidden_id}` must stay hidden");
    }
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = make_shell_driver_with_state(ShellTestState::new(app).with_merge_passes(), ViewportMetrics::new(1280.0, 800.0, 1.0), tokens);

    let cmd_mod = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    driver.press_egui_key(Key::K, cmd_mod);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    driver.state_mut().views.command_palette.query = String::from("graph.setSpec");
    driver.run_settled();
    driver.click_widget_id("palette.item.0")?;
    assert!(driver.state().last_intent_error.is_none());
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. }));

    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::None;
    driver.run_settled();
    let (sheets_before, undo_before) = (driver.state().app.document().total_sheet_count(), driver.state().app.session().undo_depth());
    driver.press_egui_key(Key::K, cmd_mod);
    driver.state_mut().views.command_palette.query = String::from("sheet.duplicate");
    driver.run_settled();
    driver.click_widget_id("palette.item.0")?;
    assert!(driver.state().last_intent_error.is_none());
    assert_eq!((driver.state().app.document().total_sheet_count(), driver.state().app.session().undo_depth()), (sheets_before + 1, undo_before + 1));
    Ok(())
}

#[test]
fn a5_async_canvas_raster_reuses_single_worker_and_converges_on_resize_drag() -> Result<(), String> {
    let mut app = seed_app()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "gs-1" })).map_err(|e| e.to_string())?;
    app.sync_views();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, CanvasViewEguiState { synchronous_raster: false, ..CanvasViewEguiState::default() }),
        ViewportMetrics::new(600.0, 420.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (app, canvas)| {
            let tokens = Tokens::get(ui.ctx());
            let mut intents = PendingUiIntents::new();
            egui::CentralPanel::default().show(ui, |ui| {
                let _ = render_canvas_view(
                    ui,
                    app,
                    canvas,
                    CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false },
                    meta,
                    &mut intents,
                );
            });
        },
    );
    for step in 0..25_u32 {
        driver.set_viewport(ViewportMetrics::new(600.0 + f64::from(step) * 20.0, 420.0 + f64::from(step) * 8.0, 1.0));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        driver.run_settled();
        if !driver.state().1.slot.rendering_in_flight && driver.state().1.slot.cached_texture.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let stats = canvas_async_worker_stats(driver.ctx()).ok_or("missing async worker")?;
    assert_eq!(stats.worker_spawn_count, 1);
    assert!(stats.submitted_count >= 10 && !driver.state().1.slot.rendering_in_flight && driver.state().1.slot.cached_texture.is_some());
    assert!(driver.state().1.slot.last_width_px >= 500);
    drop(driver);
    Ok(())
}
