#![forbid(unsafe_code)]

//! Wave 1 (`R15.1`) tests for `host_draws_menu_bar` suppression, `PendingUiIntents` single-commit
//! multi-pass settlement, AccessKit `ui.inspect` bridge, CPU `egui` rasterizer screenshots,
//! `ZLayer::ModalBackdrop` outside input capture, `Role::ScrollBar` non-overlap, and `ui_scale`
//! preferences migration.

use curvebook_doc::TableKind;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    build_ui_tree_snapshot_from_egui, check_accesskit_scrollbar_non_overlap, clipped_rect_to_bounds_px, logical_pt_to_physical_px,
    physical_px_to_logical_pt, render_egui_shell_frame, render_minimal_shell_and_about,
};
use curvebook_ui_egui::egui_shell::rasterizer::EguiSoftwareRasterizer;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, GeometryStamp, PendingUiIntents, Tokens, UiIntent, UiIntentOutcome};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{
    DEFAULT_UI_SCALE, MAX_UI_SCALE, MIN_UI_SCALE, RenderTargetSpec, UiApp, UiThemeMode, inspect_ui_tree_for_metrics,
    migrate_preferences_value_with_ui_scale,
};
use serde_json::json;

use crate::common::png_dimensions;

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
    assert!(!app.host_draws_menu_bar);
    assert!(capture_shell_meta(&app, 1200.0, 760.0).entries.iter().any(|n| n.widget_id.starts_with("menu.")));

    app.host_draws_menu_bar = true;
    let host_meta = capture_shell_meta(&app, 1200.0, 760.0);
    assert!(host_meta.entries.iter().all(|n| !n.widget_id.starts_with("menu.")));
    assert!(host_meta.entries.iter().any(|n| n.widget_id.starts_with("toolbar.")));
}

#[test]
fn canonical_ui_inspect_about_dialog_and_toolbar_disablement() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    app.sync_views();

    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let canonical_json = serde_json::to_value(inspect_ui_tree_for_metrics(&app, metrics, Some("dialog"), false))?;
    let egui_json = serde_json::to_value(app.inspect_egui_shell_snapshot(metrics, Some("dialog"), false))?;
    assert_eq!(egui_json["active_dialog"], json!("about"));
    assert_eq!(egui_json["dialog"]["dialog_kind"], json!("about"));
    assert_eq!(egui_json, canonical_json);

    let bounded = app.inspect_egui_shell_snapshot(metrics, None, true);
    let palette_btn = bounded
        .toolbar
        .as_ref()
        .and_then(|t| t.iter().find(|n| n.widget_id == "toolbar.command_palette"))
        .ok_or("missing toolbar.command_palette")?;
    assert_eq!((palette_btn.role.as_str(), palette_btn.command_id.as_deref()), ("button", Some("ui.palette.open")));
    assert!(!palette_btn.enabled && palette_btn.disabled_reason.as_deref() == Some("modal dialog is active"));
    assert!(palette_btn.bounds.is_some_and(|b| b[2] > 0.0 && b[3] > 0.0));
    Ok(())
}

#[test]
fn pending_ui_intents_commit_strictly_once_after_request_discard_settles() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let (sheet_before, undo_before) = (app.document().data_sheets.len(), app.session().undo_depth());
    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))), ..Default::default() };

    let (mut meta, mut pass_count, mut pass1_discard, mut pass2_discard, mut sheets_after_p1, mut outcomes) =
        (FrameInspectMeta::default(), 0usize, false, true, usize::MAX, Vec::new());
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
        let res = intents.commit_if_settled(&mut app, will_discard).expect("commit gate");
        if pass_count == 1 {
            pass1_discard = will_discard;
            sheets_after_p1 = app.document().data_sheets.len();
        } else {
            pass2_discard = will_discard;
            outcomes = res;
        }
    });
    out.textures_delta.clear();
    assert_eq!((pass_count, pass1_discard, pass2_discard, sheets_after_p1), (2, true, false, sheet_before));
    assert_eq!(outcomes.len(), 2);
    assert!(matches!(outcomes.first(), Some(UiIntentOutcome::Engine { .. })));
    assert_eq!((app.document().data_sheets.len(), app.session().undo_depth()), (sheet_before + 1, undo_before + 1));
    assert!((app.ui_scale() - 1.25).abs() < 1e-4);
    Ok(())
}

#[test]
fn cpu_egui_rasterizer_renders_themes_regions_and_multi_dpi_screenshots() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    assert!(
        app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 800, height_px: 520, dpi: 96 }, ScreenshotRegionTarget::Dialog).is_err()
    );

    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    app.sync_views();
    let mut theme_pngs = Vec::new();
    for mode in UiThemeMode::ALL {
        app.set_theme_mode(mode);
        let (png, w, h) =
            app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 840, height_px: 540, dpi: 144 }, ScreenshotRegionTarget::FullWindow)?;
        assert_eq!((w, h, png_dimensions(&png)), (840, 540, Some((840, 540))));
        theme_pngs.push(png);
    }
    assert!(theme_pngs[0] != theme_pngs[1] && theme_pngs[1] != theme_pngs[2]);

    for dpi in [96_u16, 144, 192] {
        for (target, strict_lt) in [(ScreenshotRegionTarget::Canvas, false), (ScreenshotRegionTarget::Dialog, true)] {
            let (png, w, h) = app.render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 960, height_px: 640, dpi }, target)?;
            assert!(w > 0 && h > 0 && if strict_lt { w < 960 && h < 640 } else { w <= 960 && h <= 640 });
            assert_eq!(png_dimensions(&png), Some((w, h)));
        }
    }
    Ok(())
}

#[test]
fn modal_backdrop_captures_100pct_outside_clicks_and_wheels_and_scrollbars_never_overlap() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    app.sync_views();

    for [w_pt, h_pt] in [[640.0_f32, 420.0], [1440.0, 600.0]] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let (mut meta, mut intents) = (FrameInspectMeta::default(), PendingUiIntents::new());
        let mut app_closed = app.clone();
        app_closed.canvas_dialogs.active_dialog = ActiveDialog::None;
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w_pt, h_pt))), ..Default::default() },
            |ui| {
                let _ = render_minimal_shell_and_about(ui, &app_closed, &mut meta, &mut intents);
            },
        );
        out.textures_delta.clear();
        let ak = out.platform_output.accesskit_update.as_ref().ok_or("missing AccessKit TreeUpdate")?;
        assert!(ak.nodes.iter().any(|(_, n)| n.role() == egui::accesskit::Role::ScrollBar));
        check_accesskit_scrollbar_non_overlap(ak)?;
        let snap =
            build_ui_tree_snapshot_from_egui(&app_closed, &meta, Some(ak), None, true, ViewportMetrics::new(f64::from(w_pt), f64::from(h_pt), 1.0));
        assert!(snap.toolbar.as_ref().is_some_and(|t| !t.is_empty()));
    }

    let ctx = egui::Context::default();
    let (mut meta, mut intents, mut report) = (FrameInspectMeta::default(), PendingUiIntents::new(), Default::default());
    let outside = egui::pos2(12.0, 120.0);
    for events in [
        vec![],
        vec![egui::Event::PointerMoved(outside)],
        vec![egui::Event::PointerButton { pos: outside, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }],
        vec![
            egui::Event::PointerButton { pos: outside, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -48.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    ] {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))),
                events,
                ..Default::default()
            },
            |ui| {
                report = render_minimal_shell_and_about(ui, &app, &mut meta, &mut intents);
            },
        );
        out.textures_delta.clear();
    }
    assert!(report.outside_click_captured && report.outside_wheel_captured);
    assert!(meta.find_by_widget_id("modal.backdrop").is_some() && intents.is_empty());
    Ok(())
}

#[test]
fn dpi_roundtrip_geometry_stamp_and_preferences_ui_scale_migration() -> Result<(), Box<dyn std::error::Error>> {
    for scale in [1.0_f32, 1.5, 2.0] {
        let pt = [320.5_f64, 184.25_f64];
        let phys = logical_pt_to_physical_px(pt, scale);
        let back = physical_px_to_logical_pt(phys, scale);
        assert!((phys[0] - pt[0] * f64::from(scale)).abs() < 1e-6 && (back[0] - pt[0]).abs() < 1e-6 && (back[1] - pt[1]).abs() < 1e-6);
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 50.0));
        assert_eq!(
            clipped_rect_to_bounds_px(rect, scale).ok_or("missing bounds")?,
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
    assert!(!stamp.matches_target(960, 600, 1.0) && stamp.matches_target(1920, 1200, ctx.pixels_per_point()));

    let mut app = UiApp::new();
    assert!((app.set_ui_scale(1.35) - 1.35).abs() < 1e-4);
    app.set_theme_mode(UiThemeMode::Dark);
    assert!((app.ui_scale() - 1.35).abs() < 1e-4);
    assert!((app.set_ui_scale(10.0) - MAX_UI_SCALE).abs() < 1e-4 && (app.set_ui_scale(0.1) - MIN_UI_SCALE).abs() < 1e-4);
    assert!((app.reset_ui_scale() - DEFAULT_UI_SCALE).abs() < 1e-4);

    let (migrated, scale) = migrate_preferences_value_with_ui_scale(&json!({ "locale": "de", "scale": 1.4 }))?;
    assert_eq!((&migrated["version"], &migrated["ui_scale"], migrated.get("scale").is_none()), (&json!(1), &json!(1.4), true));
    assert!((scale - 1.4).abs() < 1e-4);
    assert!(migrate_preferences_value_with_ui_scale(&json!({ "version": 99, "ui_scale": 1.25 })).is_err());

    let mut egui_app = UiApp::new();
    let ok_shot: serde_json::Value = serde_json::from_str(
        &egui_app.dispatch_control_line(r#"{"id":"s2","method":"ui.screenshot","params":{"width_px":320,"height_px":200,"dpi":96}}"#),
    )?;
    assert_eq!((&ok_shot["ok"], &ok_shot["result"]["width_px"]), (&json!(true), &json!(320)));
    assert!(ok_shot["result"]["data_base64"].as_str().is_some_and(|s| s.starts_with("iVBORw0K")));
    let _ = (Tokens::for_mode(UiThemeMode::Light).text_primary, EguiSoftwareRasterizer::new());
    Ok(())
}

#[test]
fn ui_scale_1_5_does_not_double_scale_offscreen_inspect_or_screenshot() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app_1x, mut app_15x) = (UiApp::new(), UiApp::new());
    app_15x.set_ui_scale(1.5);
    for (id, app) in [("a1", &mut app_1x), ("a2", &mut app_15x)] {
        let open: serde_json::Value = serde_json::from_str(&app.dispatch_control_line(&format!(r#"{{"id":"{id}","method":"app.about"}}"#)))?;
        assert_eq!((&open["ok"], &app.canvas_dialogs.active_dialog), (&json!(true), &ActiveDialog::About));
    }

    let metrics = ViewportMetrics::new(800.0, 500.0, 1.5);
    let (ctx, mut out_frame, meta, stamp) = app_15x.run_settled_egui_shell_frame([800.0, 500.0], 1.5, None, |ui, app_ref, m, intents| {
        let _ = render_minimal_shell_and_about(ui, app_ref, m, intents);
    })?;
    out_frame.textures_delta.clear();
    assert!((ctx.zoom_factor() - 1.5).abs() < 1e-3 && (out_frame.pixels_per_point - 1.5).abs() < 1e-3 && (meta.scale_factor - 1.5).abs() < 1e-3);
    assert!(stamp.matches_target(1200, 750, 1.5));

    let dlg_1x = app_1x.inspect_egui_shell_snapshot(metrics, Some("dialog"), true).dialog.and_then(|d| d.bounds).ok_or("missing 1x bounds")?;
    let dlg_15x = app_15x.inspect_egui_shell_snapshot(metrics, Some("dialog"), true).dialog.and_then(|d| d.bounds).ok_or("missing 1.5x bounds")?;
    assert!(dlg_15x[0] >= 0.0 && dlg_15x[1] >= 0.0 && dlg_15x[0] + dlg_15x[2] <= 1202.0 && dlg_15x[1] + dlg_15x[3] <= 752.0);
    assert!((dlg_15x[2] - dlg_1x[2]).abs() <= 2.0 && (dlg_15x[3] - dlg_1x[3]).abs() <= 2.0);

    let spec = RenderTargetSpec { width_px: 1200, height_px: 750, dpi: 144 };
    let (png_full, fw, fh) = app_15x.render_egui_shell_screenshot_png(spec, ScreenshotRegionTarget::FullWindow)?;
    assert_eq!((fw, fh, png_dimensions(&png_full)), (1200, 750, Some((1200, 750))));
    let (p1, d1w, d1h) = app_1x.render_egui_shell_screenshot_png(spec, ScreenshotRegionTarget::Dialog)?;
    let (p15, d15w, d15h) = app_15x.render_egui_shell_screenshot_png(spec, ScreenshotRegionTarget::Dialog)?;
    assert!(!p1.is_empty() && !p15.is_empty() && (i64::from(d15w) - i64::from(d1w)).abs() <= 2 && (i64::from(d15h) - i64::from(d1h)).abs() <= 2);
    Ok(())
}

#[test]
fn aud_07_08_13_control_routing_batch_budget_raster_and_headless_guards() -> Result<(), Box<dyn std::error::Error>> {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, CellValue, ClusterDistance, ClusterLinkage, DataSheet, GraphSheet, MultipleVariablesTable,
        MultivariateSpec, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, XyTable,
    };
    use curvebook_engine::{ExternalPathPolicy, Session};
    use curvebook_ui_egui::control_io::host_routing::{
        MAX_READ_ONLY_NESTING_DEPTH, command_replaces_project, contextual_command_params, has_non_null_path_param, is_command_read_only,
        is_read_only_ui_command,
    };
    use curvebook_ui_egui::i18n::LOCALES;
    use curvebook_ui_egui::menus::curated_menu_bar;
    use curvebook_ui_egui::{ControlResponse, handle_control_line};
    use serde_json::Value;

    let ctrl = |app: &mut UiApp, req: &Value| -> Result<ControlResponse, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(&handle_control_line(app, &serde_json::to_string(req)?))?)
    };

    // 1. AUD-07-05: Cluster append handles duplicate row labels in a single atomic undo step.
    let mut cdoc = ProjectDocument::new("ClusterDup")?;
    let mut mv = MultipleVariablesTable::new(1, 3)?;
    mv.row_titles = vec!["Ctrl".into(), "Ctrl".into(), "Treated".into()];
    mv.variables[0].cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(10.0)];
    cdoc.data_sheets.push(DataSheet::new(SheetId::new("mv-1"), "MV", TableArchetype::MultipleVariables(mv))?);
    let mut rs = ResultSheet::new(AnalysisSpec::new(
        "res-cl",
        "mv-1",
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(2),
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: None,
        }),
    )?)?;
    rs.tabs.push(ResultTab {
        id: "kmeans-assignments".into(),
        title: "K-Means".into(),
        kind: ResultTabKind::MainSummary,
        column_headers: vec!["Cluster".into()],
        row_labels: vec!["Ctrl".into(), "Ctrl".into(), "Treated".into()],
        rows: [1.0, 2.0, 2.0].into_iter().map(|v| vec![ResultCell::Number { value: v, prefix: None, units: None }]).collect(),
        footnotes: Vec::new(),
    });
    cdoc.result_sheets.push(rs);
    cdoc.active_sheet_id = Some(SheetId::new("res-cl"));
    let mut capp = UiApp::from_session(Session::from_document(cdoc));
    let u0 = capp.session().undo_depth();
    let cl_resp = ctrl(
        &mut capp,
        &json!({ "id": "cl", "method": "ui.result.appendClusterColumn", "params": { "sheet_id": "res-cl", "column_title": "Cluster" } }),
    )?;
    assert!(cl_resp.ok && cl_resp.result["assigned_rows"] == json!(3) && capp.session().undo_depth() == u0 + 1);
    if let TableArchetype::MultipleVariables(ref m) = capp.document().data_sheet(&SheetId::new("mv-1")).ok_or("mv")?.archetype {
        let cells: Vec<&CellValue> = m.variables.last().ok_or("col")?.cells.iter().map(|c| &c.value).collect();
        assert_eq!(cells, vec![&CellValue::Text("Cluster 1".into()), &CellValue::Text("Cluster 2".into()), &CellValue::Text("Cluster 2".into())]);
    }

    // 2. AUD-07-14/15/17/19/26 & AUD-13-10/11/12/13/23: Control/routing/locale/budget/batch invariants.
    let mut doc = ProjectDocument::new("GraphProj")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4)?;
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(((r + 1) * 2) as f64);
    }
    let ds = DataSheet::new("data-xy", "XY", TableArchetype::Xy(xy))?;
    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", &ds)?;
    gs.initialized = true;
    doc.data_sheets.push(ds);
    doc.graph_sheets.push(gs);
    doc.active_sheet_id = Some(SheetId::new("graph-1"));
    let mut app = UiApp::from_session(Session::from_document(doc));

    app.set_ui_scale(1.5);
    assert!(ctrl(&mut app, &json!({ "id": "t", "method": "ui.theme.set", "params": { "mode": "dark" } }))?.ok);
    assert!((app.ui_scale() - 1.5).abs() < 1e-4);

    assert!(command_replaces_project("table.importXlsx", &json!({})) && !command_replaces_project("table.importXlsx", &json!({ "merge": true })));
    let mut deep = json!({ "command": "ping" });
    for _ in 0..=MAX_READ_ONLY_NESTING_DEPTH + 2 {
        deep = json!({ "command": "command.run", "params": deep });
    }
    assert!(command_replaces_project("command.run", &deep));
    assert!(is_read_only_ui_command("ui.editParams.open") && !is_read_only_ui_command("ui.export.allTablesCsv"));
    assert!(
        is_command_read_only("ui.print.open", Some(&json!({ "submit_to_os": false })))
            && !is_command_read_only("ui.print.open", Some(&json!({ "submit_to_os": true })))
    );

    let dir = crate::common::temp_dir("cb-aud07-17")?;
    app.session_mut().set_path_policy(ExternalPathPolicy::workdir(dir.path())?);
    let pdf = ctrl(&mut app, &json!({ "id": "p", "method": "render.pdf", "params": { "sheet_id": "graph-1", "path": "out.pdf" } }))?;
    assert!(pdf.ok && pdf.result.get("data_base64").is_none() && dir.path().join("out.pdf").exists());

    for (m, sid) in [("ui.export.figure.open", "nope"), ("ui.export.resultsCsv.open", "data-xy"), ("ui.brush.point", "graph-1")] {
        assert!(!ctrl(&mut app, &json!({ "id": m, "method": m, "params": { "sheet_id": sid } }))?.ok);
    }

    let mut visited = Vec::new();
    app.set_locale("en");
    for _ in 0..LOCALES.len() {
        let next = contextual_command_params(&app, "ui.locale.set")["locale"].as_str().ok_or("locale")?.to_owned();
        visited.push(next.clone());
        app.set_locale(&next);
    }
    assert!(visited.len() == 16 && visited.iter().any(|l| l == "ar") && visited.iter().any(|l| l == "he"));
    for (loc, lbl) in [("ar", "التفضيلات..."), ("he", "העדפות...")] {
        app.set_locale(loc);
        assert!(format!("{:?}", curated_menu_bar(&app)).contains(lbl));
    }
    for key in ["dir", "directory", "destination_path", "unpacked_dir", "output_path", "file_path", "path"] {
        assert!(has_non_null_path_param(&json!({ key: "x" })) && has_non_null_path_param(&json!({ "nested": { key: "x" } })));
    }
    Ok(())
}

#[test]
fn aud_13_10_11_12_13_invoke_menu_tabs_screenshot_budget_and_batch_cap() -> Result<(), Box<dyn std::error::Error>> {
    use curvebook_doc::ErrorCode;
    use curvebook_engine::ExternalPathPolicy;
    use curvebook_ui_egui::egui_shell::inspect_bridge::{ParsedSwitchTabTarget, finalize_screenshot_payload_with_limit, parse_switch_tab_params};
    use curvebook_ui_egui::{ControlResponse, dispatch_gui_action, handle_control_line};
    use serde_json::Value;

    let ctrl = |app: &mut UiApp, req: &Value| -> Result<ControlResponse, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(&handle_control_line(app, &serde_json::to_string(req)?))?)
    };
    let (mut app, _) = crate::helpers::seed_xy_app(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]);
    let _ = app.execute_command("graph.create", json!({ "sheet_id": "graph-1", "data_sheet_id": "xy-1", "kind": "xy", "initialized": true }))?;
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "graph-1" }))?;
    let dir = crate::common::temp_dir("cb-aud13-12")?;
    app.session_mut().set_path_policy(ExternalPathPolicy::workdir(dir.path())?);

    // AUD-13-10/11/12: human_path_only empty params, int tab, screenshot payload & budget caps.
    app.set_human_path_only(true);
    assert_eq!(dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.rulers.toggle", "params": {} }))?["invoked"], json!(true));
    assert!(dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.rulers.toggle", "params": { "visible": true } })).is_err());
    app.set_human_path_only(false);
    assert!(matches!(parse_switch_tab_params(&json!({ "tab": 2 }))?.target, ParsedSwitchTabTarget::Tab(t) if t == "2"));

    let shot = dispatch_gui_action(&mut app, "ui.screenshot", json!({ "width_px": 160, "height_px": 120 }))?;
    assert!(shot.get("data_base64").is_some() && shot.get("png_base64").is_none() && shot.get("data_base64_omitted").is_none());
    let fake_png = vec![0x89u8; 4096];
    let target = RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 144 };
    let shot_out = dir.path().join("large.png");
    let shot_str = shot_out.to_str().ok_or("utf8")?;
    let omitted = finalize_screenshot_payload_with_limit("full_window", target, &fake_png, Some(shot_str), 1024, |p, b| {
        std::fs::write(p, b).map_err(|e| curvebook_engine::EngineError::with_code(ErrorCode::InternalError, e.to_string()))
    })?;
    assert!(omitted.get("data_base64").is_none() && omitted["data_base64_omitted"] == json!(true) && std::fs::read(&shot_out)? == fake_png);
    assert_eq!(
        finalize_screenshot_payload_with_limit("full_window", target, &fake_png, None, 1024, |_, _| Ok(())).unwrap_err().code,
        ErrorCode::ResourceLimit
    );
    let mut wrote = false;
    assert_eq!(
        finalize_screenshot_payload_with_limit("full_window", target, &fake_png, Some(shot_str), 64, |_, _| {
            wrote = true;
            Ok(())
        })
        .unwrap_err()
        .code,
        ErrorCode::ResourceLimit
    );
    assert!(!wrote);

    // AUD-13-13: command.batch structured failure details & 1000-step cap.
    let bfail = ctrl(
        &mut app,
        &json!({ "id": "bf", "method": "command.batch", "params": { "atomic": true, "commands": [
        { "command": "render.pdf", "params": { "sheet_id": "graph-1", "path": "step0.pdf" } },
        { "command": "sheet.select", "params": { "sheet_id": "missing" } }
    ] } }),
    )?;
    let det = bfail.error_detail.ok_or("missing detail")?;
    for obj in [&det, det.get("details").ok_or("details")?] {
        assert_eq!(
            (
                &obj["failed_index"],
                &obj["failed_command_id"],
                &obj["completed_steps"],
                &obj["rolled_back"],
                &obj["filesystem_side_effects_persisted"]
            ),
            (&json!(1), &json!("sheet.select"), &json!(1), &json!(true), &json!(true))
        );
    }
    let too_many: Vec<Value> = (0..1005).map(|_| json!({ "command": "ping" })).collect();
    for (atomic, rolled) in [(true, true), (false, false)] {
        let lf = ctrl(&mut app, &json!({ "id": "lim", "method": "command.batch", "params": { "atomic": atomic, "commands": too_many } }))?;
        let d = lf.error_detail.ok_or("lim detail")?;
        assert_eq!(
            (&d["code"], &d["details"]["completed_steps"], &d["details"]["rolled_back"]),
            (&json!("RESOURCE_LIMIT"), &json!(0), &json!(rolled))
        );
    }
    Ok(())
}

#[test]
fn aud_08_progress_modal_pointer_hover_crop_raster_and_rebound_copy_paste_guards() -> Result<(), Box<dyn std::error::Error>> {
    use curvebook_doc::SheetId;
    use curvebook_render::RasterImage;
    use curvebook_ui_egui::ZLayer;
    use curvebook_ui_egui::egui_shell::FrameInspectNodeMeta;
    use curvebook_ui_egui::egui_shell::center_stage::ProgressModalState;
    use curvebook_ui_egui::egui_shell::inspect_bridge::{
        HybridCoexistenceArbiter, HybridModalSurface, parse_pointer_gesture_params, resolve_power_curve_hover_target,
    };
    use curvebook_ui_egui::egui_shell::rasterizer::crop_raster_rgba;

    let (mut app, _) = crate::helpers::seed_xy_app(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]);
    let _ = app.execute_command("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy-1", "kind": "xy", "initialized": true }))?;
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "g1" }))?;
    let _ = SheetId::new("g1");

    // AUD-08-07/08/11/15/16: progress modal backdrop, pointer/hover validation, crop_raster_rgba, launcher & Event::Copy/Paste guard.
    let pviews = EguiShellViewState {
        progress_modal: Some(ProgressModalState {
            operation_label: "Fit".into(),
            detail: "1".into(),
            progress_0_to_1: Some(0.5),
            cancellable: true,
            cancel_requested: false,
        }),
        ..Default::default()
    };
    let dec = HybridCoexistenceArbiter::evaluate_with_views(&app, &pviews, false);
    assert!(dec.modal_surface == HybridModalSurface::EguiProgressModal && dec.modal_backdrop_active);
    assert!(parse_pointer_gesture_params(&json!({ "kind": "click", "x": 1e300, "y": 10.0 }), 0.0).is_err());
    let mut dmeta = FrameInspectMeta::default();
    dmeta.record(FrameInspectNodeMeta {
        egui_id: egui::Id::unique("power.curve_hover"),
        accesskit_id: egui::Id::unique("power.curve_hover").accesskit_id(),
        widget_id: "power.curve_hover".into(),
        role: "canvas".into(),
        label: "Power".into(),
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        unclipped_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 50.0)),
        clipped_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 50.0)),
        z_layer: ZLayer::ModalDialog,
        ellipsized: false,
        full_label: None,
        action_rank: None,
    });
    assert!(resolve_power_curve_hover_target(&mut app, &dmeta, &json!({ "hover_n": "bad" })).is_err());
    assert!(crop_raster_rgba(&RasterImage { width_px: 1000, height_px: 1000, dpi: 72, rgba: vec![0u8; 16] }, 0, 0, 1000, 1000).is_err());

    let (mut ctx_app, _) = crate::helpers::seed_column_app(&[("A", &[1.0, 2.0])], curvebook_doc::ColumnDesignHint::UnpairedReplicates);
    let ctx = egui::Context::default();
    let mut lv = EguiShellViewState { launcher_open: true, ..Default::default() };
    let (mut m1, mut i1) = (FrameInspectMeta::default(), PendingUiIntents::new());
    crate::helpers::run_egui_pass_with_ctx(&ctx, egui::vec2(1024.0, 640.0), Default::default(), false, |_, ui| {
        let _ = render_egui_shell_frame(ui, &ctx_app, &mut lv, &mut m1, &mut i1);
    })?;
    assert_eq!(m1.find_by_widget_id("status.summary").ok_or("summary")?.label, "Welcome Hub");

    lv.launcher_open = false;
    ctx_app.shortcuts.set_shortcut("ui.gallery.toggle", "Cmd+C")?;
    ctx_app.shortcuts.set_shortcut("table.paste", "F8")?;
    let (mut m2, mut i2) = (FrameInspectMeta::default(), PendingUiIntents::new());
    let copy_in = egui::RawInput { events: vec![egui::Event::Copy, egui::Event::Paste("99".into())], ..Default::default() };
    crate::helpers::run_egui_pass_with_ctx(&ctx, egui::vec2(1024.0, 640.0), copy_in, false, |_, ui| {
        let _ = render_egui_shell_frame(ui, &ctx_app, &mut lv, &mut m2, &mut i2);
    })?;
    assert!(!i2.intents().iter().any(|i| matches!(i, UiIntent::GridCopySelection | UiIntent::GridPasteFromClipboard { .. })));

    ctx_app.canvas_dialogs.active_dialog = ActiveDialog::About;
    let (mut m3, mut i3) = (FrameInspectMeta::default(), PendingUiIntents::new());
    crate::helpers::run_egui_pass_with_ctx(&ctx, egui::vec2(1024.0, 640.0), Default::default(), false, |_, ui| {
        let _ = render_egui_shell_frame(ui, &ctx_app, &mut lv, &mut m3, &mut i3);
    })?;
    assert!(!m3.find_by_widget_id("status.summary").ok_or("summary modal")?.enabled);
    Ok(())
}
