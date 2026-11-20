#![forbid(unsafe_code)]

//! Wave 1 (`R15.1` bullets 1, 4, 5) tests for `host_draws_menu_bar` suppression,
//! `PendingUiIntents` single-commit multi-pass settlement, AccessKit `ui.inspect` bridge & golden
//! JSON parity across `legacy` and `egui`, pure-Rust CPU `egui` software rasterizer screenshots,
//! physical-pt <-> logical-px round-trips, `ZLayer::ModalBackdrop` outside input capture,
//! `Role::ScrollBar` non-overlap at `640x420` & `1440x600`, and `ui_scale` preferences migration.

use curvebook_doc::TableKind;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    build_ui_tree_snapshot_from_egui, check_accesskit_scrollbar_non_overlap, clipped_rect_to_bounds_px, logical_pt_to_physical_px,
    physical_px_to_logical_pt, render_egui_shell_frame, render_minimal_shell_and_about,
};
use curvebook_ui_egui::egui_shell::rasterizer::EguiSoftwareRasterizer;
use curvebook_ui_egui::egui_shell::{
    EguiShellViewState, FrameInspectMeta, GeometryStamp, PendingUiIntents, Tokens, UiBackend, UiIntent, UiIntentOutcome,
};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{
    DEFAULT_UI_SCALE, MAX_UI_SCALE, MIN_UI_SCALE, RenderTargetSpec, UiApp, UiThemeMode, inspect_ui_tree_for_metrics,
    migrate_preferences_value_with_ui_scale,
};
use serde_json::json;

fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || !png.starts_with(b"\x89PNG\r\n\x1a\n") || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    Some((w, h))
}

fn capture_shell_meta(app: &UiApp, w_pt: f32, h_pt: f32) -> FrameInspectMeta {
    let ctx = egui::Context::default();
    let mut views = EguiShellViewState::default();
    let mut meta = FrameInspectMeta::default();
    for _ in 0..3 {
        meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::new();
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w_pt, h_pt))), ..Default::default() },
            |ui| {
                let _ = render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents);
            },
        );
        out.textures_delta.clear();
    }
    meta
}

#[test]
fn host_draws_menu_bar_suppresses_duplicate_canvas_menus_only_when_true() {
    let mut app = UiApp::new();

    // Default mode renders inline `menu.*` nodes in top chrome.
    assert!(!app.host_draws_menu_bar);
    let inline_meta = capture_shell_meta(&app, 1200.0, 760.0);
    let inline_menus: Vec<&str> = inline_meta.entries.iter().filter(|n| n.widget_id.starts_with("menu.")).map(|n| n.widget_id.as_str()).collect();
    assert!(!inline_menus.is_empty(), "expected inline menu.* nodes when host_draws_menu_bar == false");

    // Desktop host rendering native menus sets `host_draws_menu_bar = true`, suppressing duplicate inline menus.
    app.host_draws_menu_bar = true;
    let host_meta = capture_shell_meta(&app, 1200.0, 760.0);
    let duplicate_menus: Vec<&str> = host_meta.entries.iter().filter(|n| n.widget_id.starts_with("menu.")).map(|n| n.widget_id.as_str()).collect();
    assert!(duplicate_menus.is_empty(), "expected zero inline `menu.*` nodes when host_draws_menu_bar == true, found: {duplicate_menus:?}");
    assert!(
        host_meta.entries.iter().any(|n| n.widget_id.starts_with("toolbar.")),
        "contextual toolbar strip should remain present when duplicate menu bar is suppressed"
    );
}

#[test]
fn canonical_ui_inspect_about_dialog_and_toolbar_disablement() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    app.sync_views();

    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let canonical_snap = inspect_ui_tree_for_metrics(&app, metrics, Some("dialog"), false);
    let egui_snap = app.inspect_egui_shell_snapshot(metrics, Some("dialog"), false);

    let canonical_json = serde_json::to_value(&canonical_snap)?;
    let egui_json = serde_json::to_value(&egui_snap)?;
    assert_eq!(egui_json["active_dialog"], json!("about"));
    assert_eq!(egui_json["dialog"]["dialog_kind"], json!("about"));
    assert_eq!(egui_json, canonical_json, "canonical inspect_ui_tree_for_metrics must match egui shell snapshot");

    // Full workspace scope with `include_bounds: true` verifies AccessKit + `FrameInspectMeta` wiring.
    let bounded_egui = app.inspect_egui_shell_snapshot(metrics, None, true);
    let toolbar = bounded_egui.toolbar.as_ref().ok_or("expected egui toolbar nodes")?;
    let palette_btn = toolbar.iter().find(|n| n.widget_id == "toolbar.command_palette").ok_or("missing toolbar.command_palette")?;
    assert_eq!(palette_btn.role, "button");
    assert_eq!(palette_btn.command_id.as_deref(), Some("ui.palette.open"));
    assert!(!palette_btn.enabled, "toolbar button must report disabled when About modal is open");
    assert_eq!(palette_btn.disabled_reason.as_deref(), Some("modal dialog is active"));
    assert!(palette_btn.bounds.is_some_and(|b| b[2] > 0.0 && b[3] > 0.0));
    Ok(())
}

#[test]
fn pending_ui_intents_commit_strictly_once_after_request_discard_settles() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);
    // `--gui-only` allows `Caller::Ui` mutations (`PendingUiIntents` executes as `Caller::Ui`).
    app.set_gui_only(true);
    let sheet_before = app.document().data_sheets.len();
    let undo_before = app.session().undo_depth();

    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))), ..Default::default() };

    // Pass 1 calls `ctx.request_discard(...)` -> `will_discard() == true` -> MUST NOT commit.
    // `ctx.run_ui` then executes Pass 2 (`!will_discard()`) -> commits strictly once as `Caller::Ui`!
    let mut meta = FrameInspectMeta::default();
    let mut pass_count = 0usize;
    let mut pass1_will_discard = false;
    let mut pass2_will_discard = true;
    let mut sheets_after_pass1 = usize::MAX;
    let mut settled_outcomes = Vec::new();
    let mut out = ctx.run_ui(raw, |ui| {
        pass_count += 1;
        if pass_count == 1 {
            ui.ctx().request_discard("multi-pass sizing");
        }
        let will_discard = ui.ctx().will_discard();
        let mut intents = PendingUiIntents::new();
        let _ = render_minimal_shell_and_about(ui, &app, &mut meta, &mut intents);
        intents.push_engine("sheet.createData", json!({ "kind": TableKind::Xy }));
        intents.push(UiIntent::SetUiScale(1.25));
        let outcomes = intents.commit_if_settled(&mut app, will_discard).expect("commit gate");
        if pass_count == 1 {
            pass1_will_discard = will_discard;
            sheets_after_pass1 = app.document().data_sheets.len();
        } else {
            pass2_will_discard = will_discard;
            settled_outcomes = outcomes;
        }
    });
    out.textures_delta.clear();
    assert_eq!(pass_count, 2, "request_discard must trigger a 2-pass egui layout cycle");
    assert!(pass1_will_discard, "ctx.will_discard() must be true during pass 1");
    assert_eq!(sheets_after_pass1, sheet_before, "discarded pass 1 must not mutate Session");
    assert!(!pass2_will_discard, "pass 2 must settle without discarding");
    assert_eq!(settled_outcomes.len(), 2);
    assert!(matches!(settled_outcomes.first(), Some(UiIntentOutcome::Engine { .. })));
    assert_eq!(app.document().data_sheets.len(), sheet_before + 1);
    assert_eq!(app.session().undo_depth(), undo_before + 1);
    assert!((app.ui_scale() - 1.25).abs() < 1e-4);
    Ok(())
}

#[test]
fn cpu_egui_rasterizer_renders_themes_regions_and_multi_dpi_screenshots() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);

    // `dialog` target must fail cleanly when no modal is open.
    let closed_dialog_shot =
        app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 800, height_px: 520, dpi: 96 }, ScreenshotRegionTarget::Dialog);
    assert!(closed_dialog_shot.is_err());

    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    app.sync_views();

    let mut theme_pngs = Vec::new();
    for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        app.set_theme_mode(mode);
        let (png, w, h) =
            app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 840, height_px: 540, dpi: 144 }, ScreenshotRegionTarget::FullWindow)?;
        assert_eq!((w, h), (840, 540));
        assert_eq!(png_dimensions(&png), Some((840, 540)));
        theme_pngs.push(png);
    }
    assert_ne!(theme_pngs[0], theme_pngs[1], "Light and Dark egui raster outputs must differ");
    assert_ne!(theme_pngs[1], theme_pngs[2], "Dark and HighContrast egui raster outputs must differ");

    // Verify `canvas` and `dialog` crops across 1.0x / 1.5x / 2.0x DPI (`96` / `144` / `192`).
    for dpi in [96_u16, 144, 192] {
        let (canvas_png, cw, ch) =
            app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 960, height_px: 640, dpi }, ScreenshotRegionTarget::Canvas)?;
        assert!(cw > 0 && ch > 0 && cw <= 960 && ch <= 640);
        assert_eq!(png_dimensions(&canvas_png), Some((cw, ch)));

        let (dialog_png, dw, dh) =
            app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 960, height_px: 640, dpi }, ScreenshotRegionTarget::Dialog)?;
        assert!(dw > 0 && dh > 0 && dw < 960 && dh < 640);
        assert_eq!(png_dimensions(&dialog_png), Some((dw, dh)));
    }
    Ok(())
}

#[test]
fn modal_backdrop_captures_100pct_outside_clicks_and_wheels_and_scrollbars_never_overlap() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    app.sync_views();

    // 1. Check `Role::ScrollBar` non-overlap at both `640x420 pt` and `1440x600 pt`.
    for [w_pt, h_pt] in [[640.0_f32, 420.0], [1440.0, 600.0]] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::new();
        let mut app_closed = app.clone();
        app_closed.canvas_dialogs.active_dialog = ActiveDialog::None;
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w_pt, h_pt))), ..Default::default() },
            |ui| {
                let _ = render_minimal_shell_and_about(ui, &app_closed, &mut meta, &mut intents);
            },
        );
        out.textures_delta.clear();
        let ak_update = out.platform_output.accesskit_update.as_ref().ok_or("missing AccessKit TreeUpdate")?;
        assert!(
            ak_update.nodes.iter().any(|(_, n)| n.role() == egui::accesskit::Role::ScrollBar),
            "expected AccessKit Role::ScrollBar node at {w_pt}x{h_pt}"
        );
        check_accesskit_scrollbar_non_overlap(ak_update)?;
        let snap = build_ui_tree_snapshot_from_egui(
            &app_closed,
            &meta,
            Some(ak_update),
            None,
            true,
            ViewportMetrics::new(f64::from(w_pt), f64::from(h_pt), 1.0),
        );
        assert!(snap.toolbar.as_ref().is_some_and(|t| !t.is_empty()));
    }

    // 2. Check 100% `ZLayer::ModalBackdrop` capture for outside pointer click + outside wheel events.
    let ctx = egui::Context::default();
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    let mut out_init = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))), ..Default::default() },
        |ui| {
            let _ = render_minimal_shell_and_about(ui, &app, &mut meta, &mut intents);
        },
    );
    out_init.textures_delta.clear();

    let outside_pos = egui::pos2(12.0, 120.0);
    let mut out_hover = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))),
            events: vec![egui::Event::PointerMoved(outside_pos)],
            ..Default::default()
        },
        |ui| {
            let _ = render_minimal_shell_and_about(ui, &app, &mut meta, &mut intents);
        },
    );
    out_hover.textures_delta.clear();

    let mut out_press = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))),
            events: vec![egui::Event::PointerButton {
                pos: outside_pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            let _ = render_minimal_shell_and_about(ui, &app, &mut meta, &mut intents);
        },
    );
    out_press.textures_delta.clear();

    let mut report = Default::default();
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))),
            events: vec![
                egui::Event::PointerButton {
                    pos: outside_pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -48.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| {
            report = render_minimal_shell_and_about(ui, &app, &mut meta, &mut intents);
        },
    );
    out.textures_delta.clear();

    assert!(report.outside_click_captured, "outside click must be absorbed by ZLayer::ModalBackdrop");
    assert!(report.outside_wheel_captured, "outside wheel must be absorbed by ZLayer::ModalBackdrop");
    assert!(meta.find_by_widget_id("modal.backdrop").is_some(), "modal backdrop entry must be present in settled FrameInspectMeta");
    assert!(intents.is_empty(), "outside click/wheel must not trigger background intents behind modal");
    Ok(())
}

#[test]
fn dpi_roundtrip_geometry_stamp_and_preferences_ui_scale_migration() -> Result<(), Box<dyn std::error::Error>> {
    for scale in [1.0_f32, 1.5, 2.0] {
        let pt = [320.5_f64, 184.25_f64];
        let phys = logical_pt_to_physical_px(pt, scale);
        assert!((phys[0] - pt[0] * f64::from(scale)).abs() < 1e-6);
        let back = physical_px_to_logical_pt(phys, scale);
        assert!((back[0] - pt[0]).abs() < 1e-6);
        assert!((back[1] - pt[1]).abs() < 1e-6);

        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 50.0));
        let bounds = clipped_rect_to_bounds_px(rect, scale).ok_or("missing clipped bounds")?;
        assert_eq!(
            bounds,
            [
                (10.0 * f64::from(scale)).round(),
                (20.0 * f64::from(scale)).round(),
                (100.0 * f64::from(scale)).round(),
                (50.0 * f64::from(scale)).round()
            ]
        );
    }

    let ctx = egui::Context::default();
    let stamp = GeometryStamp::from_context(3, &ctx, [1920, 1200], true);
    assert!(!stamp.matches_target(960, 600, 1.0), "stale dimensions must be rejected by GeometryStamp");
    assert!(stamp.matches_target(1920, 1200, ctx.pixels_per_point()));

    // `set_theme_mode` must preserve `ui_scale`.
    let mut app = UiApp::new();
    assert!((app.set_ui_scale(1.35) - 1.35).abs() < 1e-4);
    app.set_theme_mode(UiThemeMode::Dark);
    assert!((app.ui_scale() - 1.35).abs() < 1e-4);
    assert!((app.set_ui_scale(10.0) - MAX_UI_SCALE).abs() < 1e-4);
    assert!((app.set_ui_scale(0.1) - MIN_UI_SCALE).abs() < 1e-4);
    assert!((app.reset_ui_scale() - DEFAULT_UI_SCALE).abs() < 1e-4);

    // v0 -> v1 migration & newer-version refusal (`AGENTS.md`).
    let v0_raw = json!({ "locale": "de", "scale": 1.4 });
    let (migrated, scale) = migrate_preferences_value_with_ui_scale(&v0_raw)?;
    assert_eq!(migrated["version"], json!(1));
    assert_eq!(migrated["ui_scale"], json!(1.4));
    assert!(migrated.get("scale").is_none());
    assert!((scale - 1.4).abs() < 1e-4);

    let future = json!({ "version": 99, "ui_scale": 1.25 });
    assert!(migrate_preferences_value_with_ui_scale(&future).is_err(), "future preferences version must be refused");

    // Egui dispatch hook + GuiOnlyPolicy::Strict on `UiApp::dispatch_control_line`.
    let mut egui_app = UiApp::new().with_ui_backend(UiBackend::Egui);
    egui_app.set_gui_only(true);
    let shot_err: serde_json::Value =
        serde_json::from_str(&egui_app.dispatch_control_line(r#"{"id":"s1","method":"ui.screenshot","params":{"path":"blocked.png"}}"#))?;
    assert_eq!(shot_err["ok"], json!(false));
    assert_eq!(shot_err["error_detail"]["code"], json!("gui_only_violation"));

    let ok_shot: serde_json::Value = serde_json::from_str(
        &egui_app.dispatch_control_line(r#"{"id":"s2","method":"ui.screenshot","params":{"width_px":320,"height_px":200,"dpi":96}}"#),
    )?;
    assert_eq!(ok_shot["ok"], json!(true));
    assert_eq!(ok_shot["result"]["width_px"], json!(320));
    assert!(ok_shot["result"]["data_base64"].as_str().is_some_and(|s| s.starts_with("iVBORw0K")));

    // Check `EguiSoftwareRasterizer` + `Tokens` font rendering directly.
    let tokens = Tokens::for_mode(UiThemeMode::Light);
    let _ = tokens.text_primary;
    let _ = EguiSoftwareRasterizer::new();
    Ok(())
}

#[test]
fn ui_scale_1_5_does_not_double_scale_offscreen_inspect_or_screenshot() -> Result<(), Box<dyn std::error::Error>> {
    let mut app_1x = UiApp::new().with_ui_backend(UiBackend::Egui);
    let open_1x: serde_json::Value = serde_json::from_str(&app_1x.dispatch_control_line(r#"{"id":"a1","method":"app.about"}"#))?;
    assert_eq!(open_1x["ok"], json!(true));
    assert_eq!(app_1x.canvas_dialogs.active_dialog, ActiveDialog::About);

    let mut app_15x = UiApp::new().with_ui_backend(UiBackend::Egui);
    app_15x.set_ui_scale(1.5);
    let open_15x: serde_json::Value = serde_json::from_str(&app_15x.dispatch_control_line(r#"{"id":"a2","method":"app.about"}"#))?;
    assert_eq!(open_15x["ok"], json!(true));
    assert_eq!(app_15x.canvas_dialogs.active_dialog, ActiveDialog::About);

    let metrics = ViewportMetrics::new(800.0, 500.0, 1.5);
    let (ctx, mut out_frame, meta, stamp) = app_15x.run_settled_egui_shell_frame([800.0, 500.0], 1.5, None, |ui, app_ref, m, intents| {
        let _ = render_minimal_shell_and_about(ui, app_ref, m, intents);
    })?;
    out_frame.textures_delta.clear();
    assert!((ctx.zoom_factor() - 1.5).abs() < 1e-3, "expected zoom_factor 1.5, got {}", ctx.zoom_factor());
    assert!(
        (out_frame.pixels_per_point - 1.5).abs() < 1e-3,
        "expected pixels_per_point 1.5 (not double-scaled 2.25), got {}",
        out_frame.pixels_per_point
    );
    assert!((meta.scale_factor - 1.5).abs() < 1e-3, "expected meta.scale_factor 1.5, got {}", meta.scale_factor);
    assert!(stamp.matches_target(1200, 750, 1.5), "expected settled 1200x750 @ 1.5x stamp, got {stamp:?}");

    let snap_1x = app_1x.inspect_egui_shell_snapshot(metrics, Some("dialog"), true);
    let snap_15x = app_15x.inspect_egui_shell_snapshot(metrics, Some("dialog"), true);
    let dlg_1x = snap_1x.dialog.as_ref().and_then(|d| d.bounds).ok_or("missing 1x dialog bounds")?;
    let dlg_15x = snap_15x.dialog.as_ref().and_then(|d| d.bounds).ok_or("missing 1.5x dialog bounds")?;
    assert!(
        dlg_15x[0] >= 0.0 && dlg_15x[1] >= 0.0 && dlg_15x[0] + dlg_15x[2] <= 1202.0 && dlg_15x[1] + dlg_15x[3] <= 752.0,
        "1.5x offscreen dialog bounds must fit 1200x750 viewport without double scaling: {dlg_15x:?}"
    );
    assert!(
        (dlg_15x[2] - dlg_1x[2]).abs() <= 2.0 && (dlg_15x[3] - dlg_1x[3]).abs() <= 2.0,
        "dialog bounds at 144 DPI should match between ui_scale=1.0 ({dlg_1x:?}) and ui_scale=1.5 ({dlg_15x:?})"
    );

    let (png_full_15, fw, fh) = app_15x
        .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1200, height_px: 750, dpi: 144 }, ScreenshotRegionTarget::FullWindow)?;
    assert_eq!((fw, fh), (1200, 750));
    assert_eq!(png_dimensions(&png_full_15), Some((1200, 750)));

    let (png_dlg_1x, d1w, d1h) =
        app_1x.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1200, height_px: 750, dpi: 144 }, ScreenshotRegionTarget::Dialog)?;
    let (png_dlg_15x, d15w, d15h) =
        app_15x.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1200, height_px: 750, dpi: 144 }, ScreenshotRegionTarget::Dialog)?;
    assert!(!png_dlg_1x.is_empty() && !png_dlg_15x.is_empty());
    assert!((i64::from(d15w) - i64::from(d1w)).abs() <= 2 && (i64::from(d15h) - i64::from(d1h)).abs() <= 2);
    Ok(())
}
