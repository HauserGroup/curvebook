#![forbid(unsafe_code)]

//! Audit regression tests for `FA-SEC18-01` & `FA-SEC19-02` UI intent emission:
//! `ExportFigure` modal must emit host-routed `EngineCommand`s with schema-valid
//! options across PDF, SVG, PNG, TIFF, and EMF for both Graph and Layout sheets,
//! and must gate unsupported dimension / color-space controls honestly.

use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{UiApp, is_desktop_host_routed_command};
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use serde_json::{Value, json};

struct ShellDriverState {
    app: UiApp,
    views: EguiShellViewState,
    pending: PendingUiIntents,
    last_err: Option<String>,
}

fn seed_graph_and_layout_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-exp",
            "title": "Dose Response",
            "kind": "xy",
            "cols": 1,
            "rows": 3,
            "auto_graph": false
        }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-exp", "values": [1.0, 2.0, 4.0] })).map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": "ds-exp", "rows": [[10.0], [25.0], [60.0]] })).map_err(|e| e.to_string())?;
    app.execute_command("graph.create", json!({ "sheet_id": "gs-exp", "data_sheet_id": "ds-exp", "title": "Export Graph" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("layout.create", json!({ "sheet_id": "ls-exp", "title": "Export Layout" })).map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "gs-exp" })).map_err(|e| e.to_string())?;
    app.sync_views();
    Ok(app)
}

fn make_driver(mut app: UiApp) -> EguiHumanPathDriver<'static, ShellDriverState> {
    let doc_seed = app.document().clone();
    app.egui_views.sync_history_with_document(&doc_seed);
    let state = ShellDriverState { app, views: EguiShellViewState::default(), pending: PendingUiIntents::new(), last_err: None };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        state,
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass);
            st.pending.merge_pass(pass);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let batch = st.pending.commit_all_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
            if let Some(err) = batch.results.iter().find_map(|res| res.as_ref().err()) {
                st.last_err = Some(err.to_string());
            }
        }
    });
    driver.run_settled();
    driver
}

fn capture_export_ok_intent(app: &mut UiApp, views: &mut EguiShellViewState) -> Result<(String, Value, bool), String> {
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let mut meta = FrameInspectMeta::default();

    for pass in 0..4 {
        let mut intents = PendingUiIntents::default();
        let raw = if pass == 3 {
            let ok_node = meta.find_by_widget_id("modal.ok").ok_or_else(|| String::from("missing modal.ok inside ExportFigure modal"))?;
            let center = ok_node.clipped_rect.center();
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                events: vec![
                    Event::PointerMoved(center),
                    Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                    Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
                ],
                ..RawInput::default()
            }
        } else {
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))), ..RawInput::default() }
        };
        let mut out = ctx.run_ui(raw, |ui| {
            let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
            tokens.apply_to_ctx(ui.ctx());
            tokens.apply_to_ui(ui);
            meta = FrameInspectMeta::default();
            let _ = render_egui_shell_frame(ui, app, views, &mut meta, &mut intents);
        });
        out.textures_delta.clear();
        if pass == 3 {
            let mut close_seen = false;
            let mut engine_intent: Option<(String, Value)> = None;
            for intent in intents.intents() {
                match intent {
                    UiIntent::EngineCommand { command_id, params } => {
                        engine_intent = Some((command_id.clone(), params.clone()));
                    }
                    UiIntent::CloseAllModals => close_seen = true,
                    _ => {}
                }
            }
            let (cmd, params) = engine_intent.ok_or_else(|| String::from("clicking Export modal.ok must queue UiIntent::EngineCommand"))?;
            return Ok((cmd, params, close_seen));
        }
    }
    Err(String::from("failed to capture Export modal.ok intent"))
}

#[test]
fn export_figure_modal_queues_host_routed_commands_for_all_formats_and_sheets() -> Result<(), String> {
    // 1. GraphSheet exports: PDF (sRGB & CMYK), SVG, PNG, TIFF (Deflate/LZW/None), EMF.
    {
        let mut driver = make_driver(seed_graph_and_layout_app()?);
        driver.state_mut().app.canvas_dialogs.open_export_figure(&"gs-exp".into());
        driver.run_settled();

        // PDF default (sRGB)
        driver.click_widget_id("export.fmt.pdf")?;
        let mut app = driver.state().app.clone();
        let mut views = driver.state().views.clone();
        let (cmd, params, close_seen) = capture_export_ok_intent(&mut app, &mut views)?;
        assert_eq!(cmd, "graph.exportPdf");
        assert!(is_desktop_host_routed_command(&cmd));
        assert_eq!(params, json!({ "sheet_id": "gs-exp", "color_space": "srgb" }));
        assert!(close_seen);

        // PDF with CMYK-Proof
        driver.click_widget_id("export.cs.cmyk")?;
        let mut app = driver.state().app.clone();
        let mut views = driver.state().views.clone();
        let (cmd, params, _) = capture_export_ok_intent(&mut app, &mut views)?;
        assert_eq!(cmd, "graph.exportPdf");
        assert_eq!(params, json!({ "sheet_id": "gs-exp", "color_space": "cmyk" }));

        // SVG
        driver.click_widget_id("export.fmt.svg")?;
        let mut app = driver.state().app.clone();
        let mut views = driver.state().views.clone();
        let (cmd, params, _) = capture_export_ok_intent(&mut app, &mut views)?;
        assert_eq!(cmd, "graph.exportSvg");
        assert!(is_desktop_host_routed_command(&cmd));
        assert_eq!(params, json!({ "sheet_id": "gs-exp" }));

        // PNG with custom width preset & DPI
        driver.click_widget_id("export.fmt.png")?;
        driver.click_widget_id("export.preset.174mm")?;
        let mut app = driver.state().app.clone();
        let expected_w = app.canvas_dialogs.export_figure.width_px;
        let expected_h = app.canvas_dialogs.export_figure.height_px;
        let expected_dpi = app.canvas_dialogs.export_figure.dpi;
        let mut views = driver.state().views.clone();
        let (cmd, params, _) = capture_export_ok_intent(&mut app, &mut views)?;
        assert_eq!(cmd, "graph.exportPng");
        assert!(is_desktop_host_routed_command(&cmd));
        assert_eq!(params, json!({ "sheet_id": "gs-exp", "width_px": expected_w, "height_px": expected_h, "dpi": expected_dpi }));

        // TIFF variants
        for (widget_id, expected_comp) in [("export.fmt.tiff_deflate", "deflate"), ("export.fmt.tiff_lzw", "lzw"), ("export.fmt.tiff_none", "none")] {
            driver.click_widget_id(widget_id)?;
            let mut app = driver.state().app.clone();
            let mut views = driver.state().views.clone();
            let (cmd, params, _) = capture_export_ok_intent(&mut app, &mut views)?;
            assert_eq!(cmd, "export.tiff");
            assert!(is_desktop_host_routed_command(&cmd));
            assert_eq!(
                params,
                json!({
                    "sheet_id": "gs-exp",
                    "width_px": expected_w,
                    "height_px": expected_h,
                    "dpi": expected_dpi,
                    "compression": expected_comp,
                })
            );
        }

        // EMF
        driver.click_widget_id("export.fmt.emf")?;
        let mut app = driver.state().app.clone();
        let mut views = driver.state().views.clone();
        let (cmd, params, _) = capture_export_ok_intent(&mut app, &mut views)?;
        assert_eq!(cmd, "graph.exportEmf");
        assert!(is_desktop_host_routed_command(&cmd));
        assert_eq!(params, json!({ "sheet_id": "gs-exp" }));

        // Transparent canvas background: propagates to SVG, PNG, TIFF, and never to EMF/PDF.
        driver.click_widget_id("export.fmt.png")?;
        driver.click_widget_id("export.transparent_bg")?;
        for (fmt_wid, expected_cmd, expect_transparent) in [
            ("export.fmt.png", "graph.exportPng", true),
            ("export.fmt.svg", "graph.exportSvg", true),
            ("export.fmt.tiff_deflate", "export.tiff", true),
            ("export.fmt.emf", "graph.exportEmf", false),
            ("export.fmt.pdf", "graph.exportPdf", false),
        ] {
            driver.click_widget_id(fmt_wid)?;
            let mut app = driver.state().app.clone();
            let mut views = driver.state().views.clone();
            let (cmd, params, _) = capture_export_ok_intent(&mut app, &mut views)?;
            assert_eq!(cmd, expected_cmd);
            if expect_transparent {
                assert_eq!(params.get("transparent_bg"), Some(&json!(true)), "`{cmd}` must receive transparent_bg=true");
            } else {
                assert_eq!(params.get("transparent_bg"), None, "`{cmd}` must not receive transparent_bg");
            }
        }
    }

    // 2. LayoutSheet exports route via `export.{pdf,svg,png,tiff}` and `layout.exportEmf`.
    {
        let mut driver = make_driver(seed_graph_and_layout_app()?);
        driver.state_mut().app.canvas_dialogs.open_export_figure(&"ls-exp".into());
        driver.run_settled();

        for (fmt_widget, expected_cmd) in [
            ("export.fmt.pdf", "export.pdf"),
            ("export.fmt.svg", "export.svg"),
            ("export.fmt.png", "export.png"),
            ("export.fmt.tiff_lzw", "export.tiff"),
            ("export.fmt.emf", "layout.exportEmf"),
        ] {
            driver.click_widget_id(fmt_widget)?;
            let mut app = driver.state().app.clone();
            let mut views = driver.state().views.clone();
            let (cmd, params, _) = capture_export_ok_intent(&mut app, &mut views)?;
            assert_eq!(cmd, expected_cmd);
            assert!(is_desktop_host_routed_command(&cmd));
            assert_eq!(params.get("sheet_id").and_then(Value::as_str), Some("ls-exp"));
        }
    }

    Ok(())
}

#[test]
fn export_figure_modal_gates_unsupported_vector_dimensions_and_color_spaces() -> Result<(), String> {
    let mut driver = make_driver(seed_graph_and_layout_app()?);
    driver.state_mut().app.canvas_dialogs.open_export_figure(&"gs-exp".into());
    driver.run_settled();

    // PDF: vector dimensions disabled, Display-P3/Grayscale disabled, CMYK enabled.
    driver.click_widget_id("export.fmt.pdf")?;
    for wid in ["export.dpi", "export.width", "export.height", "export.unit.px", "export.preset.85mm", "export.cs.p3", "export.cs.gray"] {
        let node = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}`"))?;
        assert!(!node.enabled, "`{wid}` must be disabled for PDF");
    }
    assert!(driver.frame_meta().find_by_widget_id("export.cs.srgb").is_some_and(|n| n.enabled));
    assert!(driver.frame_meta().find_by_widget_id("export.cs.cmyk").is_some_and(|n| n.enabled));

    // SVG: CMYK disabled as well.
    driver.click_widget_id("export.fmt.svg")?;
    assert!(!driver.frame_meta().find_by_widget_id("export.cs.cmyk").ok_or("missing export.cs.cmyk")?.enabled);
    assert!(!driver.frame_meta().find_by_widget_id("export.width").ok_or("missing export.width")?.enabled);

    // PNG: raster dimensions enabled, CMYK disabled, sRGB active.
    driver.click_widget_id("export.fmt.png")?;
    for wid in ["export.dpi", "export.width", "export.height", "export.unit.px", "export.preset.85mm", "export.cs.srgb"] {
        let node = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}`"))?;
        assert!(node.enabled, "`{wid}` must be enabled for PNG");
    }
    assert!(!driver.frame_meta().find_by_widget_id("export.cs.cmyk").ok_or("missing export.cs.cmyk")?.enabled);

    // Clicking Export in headless mode closes modal cleanly without error.
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_err, None);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
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
        assert!(is_desktop_host_routed_command(cmd), "`{cmd}` must be in DESKTOP_HOST_ROUTED_COMMANDS");
    }
}
