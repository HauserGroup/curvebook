//! Audit regression tests for `FA-SEC18-01` & `FA-SEC19-02` `ExportFigure` host-routed commands
//! and vector/color-space gating.
#![forbid(unsafe_code)]

use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{UiApp, is_desktop_host_routed_command};
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use serde_json::{Value, json};

use crate::common::{ShellTestState, TestRes, make_shell_driver_with_state};
use crate::helpers::{exec, widget_enabled};

fn make_export_driver(sheet_id: &str) -> Result<EguiHumanPathDriver<'static, ShellTestState>, String> {
    let mut app = UiApp::new();
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-exp", "title": "Dose Response", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": false }),
    )?;
    exec(&mut app, "table.setXCells", json!({ "sheet_id": "ds-exp", "values": [1.0, 2.0, 4.0] }))?;
    exec(&mut app, "table.setCells", json!({ "sheet_id": "ds-exp", "rows": [[10.0], [25.0], [60.0]] }))?;
    exec(&mut app, "graph.create", json!({ "sheet_id": "gs-exp", "data_sheet_id": "ds-exp", "title": "Export Graph" }))?;
    exec(&mut app, "layout.create", json!({ "sheet_id": "ls-exp", "title": "Export Layout" }))?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "gs-exp" }))?;
    app.sync_views();
    let doc_seed = app.document().clone();
    app.egui_views.sync_history_with_document(&doc_seed);
    let mut driver =
        make_shell_driver_with_state(ShellTestState::new(app).with_merge_passes().with_commit_all(), ViewportMetrics::default(), Tokens::default());
    driver.state_mut().app.canvas_dialogs.open_export_figure(&sheet_id.into());
    driver.run_settled();
    Ok(driver)
}

fn capture_export_ok(driver: &EguiHumanPathDriver<'_, ShellTestState>) -> Result<(String, Value, bool), String> {
    let app = driver.state().app.clone();
    let mut views = driver.state().views.clone();
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let mut meta = FrameInspectMeta::default();
    let rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0)));

    for pass in 0..4 {
        let mut intents = PendingUiIntents::default();
        let raw = if pass == 3 {
            let center = meta.find_by_widget_id("modal.ok").ok_or("missing modal.ok")?.clipped_rect.center();
            RawInput {
                screen_rect: rect,
                events: vec![
                    Event::PointerMoved(center),
                    Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                    Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
                ],
                ..RawInput::default()
            }
        } else {
            RawInput { screen_rect: rect, ..RawInput::default() }
        };
        ctx.run_ui(raw, |ui| {
            let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
            tokens.apply_to_ctx(ui.ctx());
            tokens.apply_to_ui(ui);
            meta = FrameInspectMeta::default();
            let _ = render_egui_shell_frame(ui, &app, &mut views, &mut meta, &mut intents);
        })
        .textures_delta
        .clear();
        if pass == 3 {
            let close_seen = intents.intents().iter().any(|i| matches!(i, UiIntent::CloseAllModals));
            let (cmd, params) = intents
                .intents()
                .iter()
                .rev()
                .find_map(|i| match i {
                    UiIntent::EngineCommand { command_id, params } => Some((command_id.clone(), params.clone())),
                    _ => None,
                })
                .ok_or("clicking modal.ok must queue EngineCommand")?;
            return Ok((cmd, params, close_seen));
        }
    }
    Err(String::from("failed to capture modal.ok intent"))
}

#[test]
fn export_figure_modal_queues_host_routed_commands_for_all_formats_and_sheets() -> TestRes {
    // 1. GraphSheet exports: PDF (sRGB & CMYK), SVG, PNG, TIFF (Deflate/LZW/None), EMF, transparent_bg.
    let mut driver = make_export_driver("gs-exp")?;
    for (clicks, expected_cmd, expected_params, check_close) in [
        (&["export.fmt.pdf"][..], "graph.exportPdf", json!({ "sheet_id": "gs-exp", "color_space": "srgb" }), true),
        (&["export.cs.cmyk"], "graph.exportPdf", json!({ "sheet_id": "gs-exp", "color_space": "cmyk" }), false),
        (&["export.fmt.svg"], "graph.exportSvg", json!({ "sheet_id": "gs-exp" }), false),
    ] {
        for wid in clicks {
            driver.click_widget_id(wid)?;
        }
        let (cmd, params, close_seen) = capture_export_ok(&driver)?;
        assert_eq!((cmd.as_str(), is_desktop_host_routed_command(&cmd), params), (expected_cmd, true, expected_params));
        if check_close {
            assert!(close_seen);
        }
    }

    driver.click_widget_id("export.fmt.png")?;
    driver.click_widget_id("export.preset.174mm")?;
    let ef = &driver.state().app.canvas_dialogs.export_figure;
    let (w, h, dpi) = (ef.width_px, ef.height_px, ef.dpi);
    let (cmd, params, _) = capture_export_ok(&driver)?;
    assert_eq!(
        (cmd.as_str(), is_desktop_host_routed_command(&cmd), params),
        ("graph.exportPng", true, json!({ "sheet_id": "gs-exp", "width_px": w, "height_px": h, "dpi": dpi }))
    );

    for (wid, comp) in [("export.fmt.tiff_deflate", "deflate"), ("export.fmt.tiff_lzw", "lzw"), ("export.fmt.tiff_none", "none")] {
        driver.click_widget_id(wid)?;
        let (cmd, params, _) = capture_export_ok(&driver)?;
        assert_eq!(
            (cmd.as_str(), is_desktop_host_routed_command(&cmd), params),
            ("export.tiff", true, json!({ "sheet_id": "gs-exp", "width_px": w, "height_px": h, "dpi": dpi, "compression": comp }))
        );
    }

    driver.click_widget_id("export.fmt.emf")?;
    let (cmd, params, _) = capture_export_ok(&driver)?;
    assert_eq!((cmd.as_str(), is_desktop_host_routed_command(&cmd), params), ("graph.exportEmf", true, json!({ "sheet_id": "gs-exp" })));

    driver.click_widget_id("export.fmt.png")?;
    driver.click_widget_id("export.transparent_bg")?;
    for (wid, expected_cmd, expect_bg) in [
        ("export.fmt.png", "graph.exportPng", true),
        ("export.fmt.svg", "graph.exportSvg", true),
        ("export.fmt.tiff_deflate", "export.tiff", true),
        ("export.fmt.emf", "graph.exportEmf", false),
        ("export.fmt.pdf", "graph.exportPdf", false),
    ] {
        driver.click_widget_id(wid)?;
        let (cmd, params, _) = capture_export_ok(&driver)?;
        assert_eq!((cmd.as_str(), params.get("transparent_bg") == Some(&json!(true))), (expected_cmd, expect_bg));
    }

    // 2. LayoutSheet exports route via `export.{pdf,svg,png,tiff}` and `layout.exportEmf`.
    let mut ldrv = make_export_driver("ls-exp")?;
    for (wid, expected_cmd) in [
        ("export.fmt.pdf", "export.pdf"),
        ("export.fmt.svg", "export.svg"),
        ("export.fmt.png", "export.png"),
        ("export.fmt.tiff_lzw", "export.tiff"),
        ("export.fmt.emf", "layout.exportEmf"),
    ] {
        ldrv.click_widget_id(wid)?;
        let (cmd, params, _) = capture_export_ok(&ldrv)?;
        assert_eq!(
            (cmd.as_str(), is_desktop_host_routed_command(&cmd), params.get("sheet_id").and_then(Value::as_str)),
            (expected_cmd, true, Some("ls-exp"))
        );
    }
    Ok(())
}

#[test]
fn export_figure_modal_gates_unsupported_vector_dimensions_and_color_spaces() -> TestRes {
    let mut driver = make_export_driver("gs-exp")?;
    let gating: [(&str, &[&str], &[&str]); 3] = [
        (
            "export.fmt.pdf",
            &["export.cs.srgb", "export.cs.cmyk"],
            &["export.dpi", "export.width", "export.height", "export.unit.px", "export.preset.85mm", "export.cs.p3", "export.cs.gray"],
        ),
        ("export.fmt.svg", &[], &["export.cs.cmyk", "export.width"]),
        (
            "export.fmt.png",
            &["export.dpi", "export.width", "export.height", "export.unit.px", "export.preset.85mm", "export.cs.srgb"],
            &["export.cs.cmyk"],
        ),
    ];
    for (fmt_wid, enabled_wids, disabled_wids) in gating {
        driver.click_widget_id(fmt_wid)?;
        for wid in enabled_wids {
            assert!(widget_enabled(&driver, wid)?, "`{wid}` enabled for {fmt_wid}");
        }
        for wid in disabled_wids {
            assert!(!widget_enabled(&driver, wid)?, "`{wid}` disabled for {fmt_wid}");
        }
    }
    driver.click_widget_id("modal.ok")?;
    assert_eq!((&driver.state().last_intent_error, &driver.state().app.canvas_dialogs.active_dialog), (&None, &ActiveDialog::None));
    Ok(())
}

#[test]
fn desktop_host_routed_commands_include_all_file_backed_exports_imports_and_templates() {
    for cmd in [
        "export.svg",
        "file.export_svg",
        "graph.exportSvg",
        "export.pdf",
        "file.export_pdf",
        "graph.exportPdf",
        "export.png",
        "file.export_png",
        "graph.exportPng",
        "export.tiff",
        "graph.exportEmf",
        "layout.exportEmf",
        "project.saveTemplate",
        "project.instantiateTemplate",
        "table.importCsv",
        "table.importPrism",
        "table.importPzfx",
        "table.importXlsx",
        "table.exportCsv",
        "analysis.exportJson",
        "analysis.exportCsv",
        "result.generateReport",
        "graph.insertImage",
        "layout.insertImage",
    ] {
        assert!(is_desktop_host_routed_command(cmd), "`{cmd}` must be host-routed");
    }
}
