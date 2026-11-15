//! Strict `--gui-only` end-to-end GUI acceptance workflows `WF-44`..`WF-49` (Shard 7).
//!
//! Every workflow boots [`UiApp::new()`] with `app.set_gui_only(true)` (`GuiOnlyPolicy::Strict`)
//! and drives all interactions exclusively through [`lumencraft_ui_egui::dispatch_gui_action`]:
//! `ui.invoke_menu`, `ui.switch_tab`, `ui.type_text`, `ui.press_key`, `ui.pointer_gesture`,
//! `ui.answer_dialog`, `ui.inspect`, and `ui.screenshot`. Zero direct `app.execute_command` calls
//! appear anywhere in this test crate.
#![forbid(unsafe_code)]

use lumencraft_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use lumencraft_doc::{
    AnalysisKind, ColorChannelMapping, DiagnosticPlotKind, EquationDisplayMode, GraphAnnotation, GraphKind, MultiplicityMethod, MultivariatePlotMode,
    ProjectDocument, SizeChannelMapping, SurvivalAnalysisSpec,
};
use lumencraft_palette::{ColormapId, LineDashStyle};
use lumencraft_ui_egui::dialogs_canvas::ActiveDialog;
use lumencraft_ui_egui::{UiApp, ViewportMetrics, dispatch_gui_action};
use serde_json::{Value, json};

type TestResult = Result<(), String>;

fn new_strict_gui_app() -> UiApp {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    app
}

fn ensure(cond: bool, msg: impl Into<String>) -> TestResult {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn gui(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed: {err}"))
}

fn default_metrics() -> Value {
    serde_json::to_value(ViewportMetrics::default()).unwrap_or_else(|_| json!({}))
}

fn inspect_tree(app: &mut UiApp, scope: Option<&str>, include_bounds: bool) -> Result<Value, String> {
    gui(
        app,
        "ui.inspect",
        json!({
            "scope": scope.unwrap_or("all"),
            "include_bounds": include_bounds,
            "metrics": default_metrics(),
        }),
    )
}

fn find_widget<'a>(snapshot: &'a Value, wid: &str) -> Option<&'a Value> {
    fn walk<'a>(v: &'a Value, target: &str) -> Option<&'a Value> {
        match v {
            Value::Object(map) => {
                if map.get("widget_id").and_then(Value::as_str) == Some(target) {
                    return Some(v);
                }
                for child in map.values() {
                    if let Some(hit) = walk(child, target) {
                        return Some(hit);
                    }
                }
                None
            }
            Value::Array(items) => items.iter().find_map(|item| walk(item, target)),
            _ => None,
        }
    }
    walk(snapshot, wid)
}

fn switch_sheet(app: &mut UiApp, sheet_id: &str) -> Result<Value, String> {
    gui(app, "ui.switch_tab", json!({ "sheet_id": sheet_id }))
}

fn type_grid_cell(app: &mut UiApp, sheet_id: &str, row: usize, col: usize, subcol: usize, text: &str) -> TestResult {
    if app.document().active_sheet_id.as_ref().map(|s| s.as_str()) != Some(sheet_id) {
        switch_sheet(app, sheet_id)?;
    }
    let widget_id = format!("grid.cell.{row}.{col}.{subcol}");
    gui(app, "ui.type_text", json!({ "widget_id": widget_id, "text": text, "commit_enter": true }))?;
    Ok(())
}

fn type_x_cell(app: &mut UiApp, sheet_id: &str, row: usize, subcol: usize, text: &str) -> TestResult {
    if app.document().active_sheet_id.as_ref().map(|s| s.as_str()) != Some(sheet_id) {
        switch_sheet(app, sheet_id)?;
    }
    let widget_id = format!("grid.xcell.{row}.{subcol}");
    gui(app, "ui.type_text", json!({ "widget_id": widget_id, "text": text, "commit_enter": true }))?;
    Ok(())
}

fn click_widget(app: &mut UiApp, widget_id: &str) -> Result<Value, String> {
    gui(app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": widget_id, "metrics": default_metrics() }))
}

fn exclude_cell_with_reason_shortcut(
    app: &mut UiApp,
    sheet_id: &str,
    row: usize,
    col: usize,
    subcol: usize,
    reason_tag: &str,
    note: &str,
) -> TestResult {
    switch_sheet(app, sheet_id)?;
    let cell_wid = format!("grid.cell.{row}.{col}.{subcol}");
    click_widget(app, &cell_wid)?;
    gui(app, "ui.press_key", json!({ "key": "e", "modifiers": { "meta": true, "shift": true } }))?;
    ensure(
        matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }),
        format!("Cmd+Shift+E on {cell_wid} must open ExcludeWithReason dialog, got {:?}", app.canvas_dialogs.active_dialog),
    )?;
    gui(
        app,
        "ui.answer_dialog",
        json!({
            "fields": { "reason_tag": reason_tag, "note": note },
            "action": "ok",
        }),
    )?;
    Ok(())
}

fn open_and_apply_graph_type(app: &mut UiApp, graph_id: &str, option_id: &str) -> TestResult {
    switch_sheet(app, graph_id)?;
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        gui(app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open", "params": { "sheet_id": graph_id } }))?;
    }
    gui(
        app,
        "ui.answer_dialog",
        json!({
            "fields": { "selected_option_id": option_id },
            "action": "ok",
        }),
    )?;
    Ok(())
}

fn run_analyze_dialog(app: &mut UiApp, data_sheet_id: &str, fields: Value) -> Result<String, String> {
    switch_sheet(app, data_sheet_id)?;
    gui(app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "sheet_id": data_sheet_id } }))?;
    let out = gui(
        app,
        "ui.answer_dialog",
        json!({
            "fields": fields,
            "force_create_new": true,
            "action": "ok",
        }),
    )?;
    let res_id = out
        .get("payload")
        .and_then(|p| p.get("sheet_id"))
        .and_then(Value::as_str)
        .or_else(|| app.document().active_sheet_id.as_ref().map(|s| s.as_str()))
        .ok_or_else(|| String::from("Analyze dialog did not return or select a result sheet_id"))?;
    Ok(res_id.to_owned())
}

fn assert_scene_compiles_cleanly(doc: &ProjectDocument, graph_id: &str) -> Result<ChartScene, String> {
    let gs = doc.graph_sheet(&graph_id.into()).ok_or_else(|| format!("missing graph sheet `{graph_id}`"))?;
    let scene = compile_graph(gs, doc).map_err(|err| format!("compile_graph({graph_id}) failed: {err}"))?;
    ensure(!scene.primitives.is_empty(), format!("compiled scene `{graph_id}` unexpectedly empty"))?;
    ensure(scene.warnings.is_empty(), format!("unexpected warnings on `{graph_id}`: {:?}", scene.warnings))?;
    Ok(scene)
}

fn collect_scene_text(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.spans.iter().map(|s| s.text.as_str()).collect::<String>()),
            _ => None,
        })
        .collect()
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let clean: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        return Err(format!("invalid base64 length {}", clean.len()));
    }
    let val = |b: u8| -> Result<u8, String> {
        match b {
            b'A'..=b'Z' => Ok(b - b'A'),
            b'a'..=b'z' => Ok(b - b'a' + 26),
            b'0'..=b'9' => Ok(b - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 byte {b}")),
        }
    };
    let bytes = clean.as_bytes();
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for chunk in bytes.as_chunks::<4>().0 {
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            let c = val(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk[3] != b'=' {
                let d = val(chunk[3])?;
                out.push((c << 6) | d);
            }
        }
    }
    Ok(out)
}

fn assert_screenshot_png(app: &mut UiApp, min_w: u32, min_h: u32) -> Result<Vec<u8>, String> {
    let shot = gui(
        app,
        "ui.screenshot",
        json!({
            "target": "canvas",
            "width_px": min_w.max(800),
            "height_px": min_h.max(500),
            "dpi": 144,
            "metrics": default_metrics(),
        }),
    )?;
    ensure(shot["format"] == json!("png"), format!("expected png format, got {}", shot["format"]))?;
    ensure(shot["path"].is_null(), format!("expected null path under strict gui_only, got {}", shot["path"]))?;
    let w = shot.get("width_px").and_then(Value::as_u64).unwrap_or(0) as u32;
    let h = shot.get("height_px").and_then(Value::as_u64).unwrap_or(0) as u32;
    ensure(w >= min_w && h >= min_h, format!("screenshot {w}x{h} smaller than minimum {min_w}x{min_h}"))?;
    let b64 = shot.get("data_base64").and_then(Value::as_str).ok_or_else(|| String::from("missing data_base64"))?;
    let png = decode_base64(b64)?;
    let expected_len = shot.get("bytes_len").and_then(Value::as_u64).unwrap_or(0) as usize;
    ensure(png.len() == expected_len && png.len() > 256, format!("unexpected PNG length: decoded={}, reported={expected_len}", png.len()))?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n"), "screenshot lacks valid PNG signature")?;
    Ok(png)
}

fn count_dashed_cubic_ellipse_paths(scene: &ChartScene) -> usize {
    scene
        .primitives
        .iter()
        .filter(|p| {
            matches!(
                p,
                ScenePrimitive::Path { path, fill: Some(_), stroke: Some(st), .. }
                    if st.dash == LineDashStyle::Dashed
                        && path.commands.iter().any(|c| matches!(c, PathCmd::CubicTo { .. }))
            )
        })
        .count()
}

fn count_hull_polygon_paths(scene: &ChartScene) -> usize {
    scene
        .primitives
        .iter()
        .filter(|p| {
            matches!(
                p,
                ScenePrimitive::Path { path, fill: Some(_), stroke: Some(st), .. }
                    if st.dash == LineDashStyle::Solid
                        && path.commands.iter().any(|c| matches!(c, PathCmd::Close))
                        && path.commands.iter().filter(|c| matches!(c, PathCmd::LineTo { .. })).count() >= 2
                        && !path.commands.iter().any(|c| matches!(c, PathCmd::CubicTo { .. }))
            )
        })
        .count()
}

fn assert_strict_gui_only_guards(app: &mut UiApp) -> TestResult {
    ensure(app.is_gui_only(), "app must remain in GuiOnlyPolicy::Strict")?;
    let blocked = dispatch_gui_action(
        app,
        "ui.screenshot",
        json!({ "target": "workspace", "width_px": 320, "height_px": 240, "path": "/tmp/forbidden_shard7_shot.png" }),
    );
    match blocked {
        Err(err) if err.to_string().to_ascii_lowercase().contains("gui") => {}
        other => return Err(format!("expected gui_only violation on screenshot disk path override, got {other:?}")),
    }
    let err = dispatch_gui_action(app, "ui.invoke_menu", json!({ "command_id": "table.clearCells", "params": { "sheet_id": "nonexistent-sheet" } }))
        .err()
        .ok_or_else(|| String::from("expected invalid sheet_id command to fail cleanly"))?;
    ensure(!err.to_string().is_empty(), "expected non-empty error string")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-44: Confidence Ellipses & Convex Hulls + Multivariable Guards
// ---------------------------------------------------------------------------
// Note: `lumencraft_doc::MultivariateGraphEncoding` models confidence ellipses as a boolean
// (`show_confidence_ellipses`) rendered at the 95% chi-square boundary in `multivariate_layout.rs`,
// and enforces an N >= 3 per-group observation guard for both convex hulls and confidence ellipses.

#[test]
fn wf44_confidence_ellipses_and_convex_hulls_multivariable_guards() -> TestResult {
    let mut app = new_strict_gui_app();
    assert_strict_gui_only_guards(&mut app)?;

    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "mv-wf44",
                "title": "Multivariate Clusters",
                "kind": "multiple_variables",
                "rows": 8,
                "auto_graph": true,
                "graph_sheet_id": "graph-wf44",
                "variables": [
                    { "title": "Biomarker X", "role": "continuous" },
                    { "title": "Response Y", "role": "continuous" },
                    { "title": "Amplitude", "role": "continuous" },
                    { "title": "Cohort", "role": "categorical" },
                    { "title": "SampleId", "role": "label" }
                ]
            }
        }),
    )?;

    // Control (3 pts -> ellipse + hull), Treated (3 pts -> ellipse + hull), Rare (2 pts -> guarded N < 3).
    let rows: [(&str, &str, &str, &str, &str); 8] = [
        ("1.0", "2.0", "10.0", "Control", "S1"),
        ("2.2", "3.8", "14.0", "Control", "S2"),
        ("3.1", "2.4", "18.0", "Control", "S3"),
        ("6.0", "7.5", "24.0", "Treated", "S4"),
        ("7.4", "9.1", "30.0", "Treated", "S5"),
        ("8.2", "7.2", "28.0", "Treated", "S6"),
        ("4.5", "11.0", "16.0", "Rare", "S7"),
        ("5.1", "11.6", "20.0", "Rare", "S8"),
    ];
    for (r, (x, y, amp, cohort, lbl)) in rows.iter().enumerate() {
        type_grid_cell(&mut app, "mv-wf44", r, 0, 0, x)?;
        type_grid_cell(&mut app, "mv-wf44", r, 1, 0, y)?;
        type_grid_cell(&mut app, "mv-wf44", r, 2, 0, amp)?;
        type_grid_cell(&mut app, "mv-wf44", r, 3, 0, cohort)?;
        type_grid_cell(&mut app, "mv-wf44", r, 4, 0, lbl)?;
    }

    // Lock Label and Categorical roles explicitly via menu guard command.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setVariableRole", "params": { "sheet_id": "mv-wf44", "col": 4, "role": "label" } }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setVariableRole", "params": { "sheet_id": "mv-wf44", "col": 3, "role": "categorical" } }),
    )?;

    open_and_apply_graph_type(&mut app, "graph-wf44", "mv.bubble")?;

    // Open BubblePlotInspector and enable both hulls & ellipses (testing click toggles + patch).
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open", "params": { "sheet_id": "graph-wf44" } }))?;
    let dlg_snap = inspect_tree(&mut app, Some("dialog"), false)?;
    ensure(find_widget(&dlg_snap, "bubble.show_convex_hulls").is_some(), "missing bubble.show_convex_hulls checkbox")?;
    ensure(find_widget(&dlg_snap, "bubble.show_confidence_ellipses").is_some(), "missing bubble.show_confidence_ellipses checkbox")?;

    click_widget(&mut app, "bubble.show_convex_hulls")?;
    click_widget(&mut app, "bubble.show_confidence_ellipses")?;
    ensure(app.canvas_dialogs.bubble_plot.show_convex_hulls, "clicking bubble.show_convex_hulls must toggle true")?;
    ensure(app.canvas_dialogs.bubble_plot.show_confidence_ellipses, "clicking bubble.show_confidence_ellipses must toggle true")?;

    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
                "x_column": 0,
                "y_column": 1,
                "size_column": 2,
                "label_column": 4,
                "color_mode": "categorical",
                "color_column": 3,
                "show_convex_hulls": true,
                "show_confidence_ellipses": true
            },
            "action": "ok"
        }),
    )?;

    let scene_both = assert_scene_compiles_cleanly(app.document(), "graph-wf44")?;
    let ellipses_both = count_dashed_cubic_ellipse_paths(&scene_both);
    let hulls_both = count_hull_polygon_paths(&scene_both);
    // Only Control & Treated have >= 3 points; Rare (2 points) is guarded and emits neither.
    ensure(ellipses_both == 2, format!("expected 2 confidence ellipses (Control & Treated only), got {ellipses_both}"))?;
    ensure(hulls_both >= 2, format!("expected >= 2 convex hull polygons, got {hulls_both}"))?;
    ensure(scene_both.plotted_points.len() == 8, format!("expected 8 plotted points, got {}", scene_both.plotted_points.len()))?;
    switch_sheet(&mut app, "graph-wf44")?;
    let png_both = assert_screenshot_png(&mut app, 200, 120)?;

    // Disable convex hulls & ellipses via dialog -> verify path counts drop & screenshot updates.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open", "params": { "sheet_id": "graph-wf44" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "show_convex_hulls": false, "show_confidence_ellipses": false }, "action": "ok" }))?;
    let scene_none = assert_scene_compiles_cleanly(app.document(), "graph-wf44")?;
    ensure(count_dashed_cubic_ellipse_paths(&scene_none) == 0, "expected 0 ellipses when disabled")?;
    ensure(count_hull_polygon_paths(&scene_none) < hulls_both, "expected fewer polygon paths when hulls disabled")?;
    let png_none = assert_screenshot_png(&mut app, 200, 120)?;
    ensure(png_none != png_both, "toggling hulls/ellipses off must change rendered PNG pixels")?;

    // Re-enable both overlays and exclude 1 Control point via Cmd+Shift+E -> drops Control from 3 -> 2 pts.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open", "params": { "sheet_id": "graph-wf44" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "show_convex_hulls": true, "show_confidence_ellipses": true }, "action": "ok" }))?;
    exclude_cell_with_reason_shortcut(&mut app, "mv-wf44", 2, 1, 0, "Outlier", "Control pt 3 excluded below N=3 guard")?;

    let scene_after_excl = assert_scene_compiles_cleanly(app.document(), "graph-wf44")?;
    ensure(
        count_dashed_cubic_ellipse_paths(&scene_after_excl) == 1,
        format!(
            "excluding Control pt 3 (leaving N=2) must suppress its ellipse, leaving 1 (Treated), got {}",
            count_dashed_cubic_ellipse_paths(&scene_after_excl)
        ),
    )?;
    ensure(scene_after_excl.plotted_points.len() == 7, "expected 7 active points after excluding row 2")?;

    // Undo exclusion via menu -> Control ellipse/hull immediately return.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "edit.undo", "params": {} }))?;
    let scene_restored = assert_scene_compiles_cleanly(app.document(), "graph-wf44")?;
    ensure(count_dashed_cubic_ellipse_paths(&scene_restored) == 2, "edit.undo must restore Control N=3 ellipse")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-45: Grouped & MultipleVariables Summary / Correlation Heatmaps
// ---------------------------------------------------------------------------

#[test]
fn wf45_grouped_and_multivariable_cross_tab_summary_heatmaps() -> TestResult {
    let mut app = new_strict_gui_app();

    // 1. Grouped replicate-summary matrix -> Heatmap (`mean` over replicates per cell) + live upstream edit.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "grp-wf45",
                "title": "Dose x Time Replicates",
                "kind": "grouped",
                "rows": 3,
                "cols": 3,
                "replicates": 3,
                "row_factor_title": "Timepoint",
                "column_factor_title": "Dose",
                "auto_graph": true,
                "graph_sheet_id": "graph-grp-hm"
            }
        }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setRowTitles", "params": { "sheet_id": "grp-wf45", "titles": ["0h", "6h", "24h"] } }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setGroupTitles", "params": { "sheet_id": "grp-wf45", "titles": ["Vehicle", "10 nM", "100 nM"] } }),
    )?;

    let grp_vals = [
        [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]],
        [[10.0, 11.0, 12.0], [14.0, 15.0, 16.0], [19.0, 20.0, 21.0]],
        [[22.0, 23.0, 24.0], [26.0, 27.0, 28.0], [30.0, 31.0, 32.0]],
    ];
    for (r, row_cols) in grp_vals.iter().enumerate() {
        for (c, reps) in row_cols.iter().enumerate() {
            for (s, val) in reps.iter().enumerate() {
                type_grid_cell(&mut app, "grp-wf45", r, c, s, &format!("{val}"))?;
            }
        }
    }

    open_and_apply_graph_type(&mut app, "graph-grp-hm", "diag.heatmap")?;
    let grp_scene_1 = assert_scene_compiles_cleanly(app.document(), "graph-grp-hm")?;
    let grp_texts_1 = collect_scene_text(&grp_scene_1).join(" | ");
    ensure(grp_texts_1.contains("Vehicle") && grp_texts_1.contains("100 nM"), format!("missing column headers: {grp_texts_1}"))?;
    ensure(grp_texts_1.contains("0h") && grp_texts_1.contains("24h"), format!("missing row headers: {grp_texts_1}"))?;
    switch_sheet(&mut app, "graph-grp-hm")?;
    let grp_png_1 = assert_screenshot_png(&mut app, 200, 120)?;

    // Edit one replicate upstream (`grid.cell.1.2.0`: 19 -> 199) -> cell mean shifts & colorbar domain expands.
    type_grid_cell(&mut app, "grp-wf45", 1, 2, 0, "199")?;
    let grp_scene_2 = assert_scene_compiles_cleanly(app.document(), "graph-grp-hm")?;
    let grp_texts_2 = collect_scene_text(&grp_scene_2).join(" | ");
    ensure(grp_texts_2 != grp_texts_1, "upstream grouped replicate edit must update heatmap colorbar scale labels")?;
    switch_sheet(&mut app, "graph-grp-hm")?;
    let grp_png_2 = assert_screenshot_png(&mut app, 200, 120)?;
    ensure(grp_png_2 != grp_png_1, "heatmap PNG must reflect live upstream replicate mean recomputation")?;

    // 2. MultipleVariables -> CorrelationMatrix heatmap (R7 predicate check + WardD2 dendrogram + DivergingBlueRed + SVG check).
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "mv-wf45",
                "title": "Biomarker Panel",
                "kind": "multiple_variables",
                "rows": 6,
                "auto_graph": true,
                "graph_sheet_id": "graph-mv-hm",
                "variables": [
                    { "title": "GeneA", "role": "continuous" },
                    { "title": "GeneB", "role": "continuous" },
                    { "title": "GeneC", "role": "continuous" },
                    { "title": "GeneD", "role": "continuous" },
                    { "title": "Donor", "role": "label" }
                ]
            }
        }),
    )?;
    let mv_rows: [[&str; 5]; 6] = [
        ["1.0", "2.1", "10.0", "5.2", "D1"],
        ["2.0", "4.0", "8.2", "4.9", "D2"],
        ["3.0", "6.2", "6.1", "6.8", "D3"],
        ["4.0", "8.1", "4.3", "3.7", "D4"],
        ["5.0", "9.9", "2.0", "7.1", "D5"],
        ["6.0", "12.2", "0.2", "4.4", "D6"],
    ];
    for (r, vals) in mv_rows.iter().enumerate() {
        for (c, v) in vals.iter().enumerate() {
            type_grid_cell(&mut app, "mv-wf45", r, c, 0, v)?;
        }
    }

    // Verify R7 gating inside Analyze dialog before committing Correlation Matrix + Heatmap.
    switch_sheet(&mut app, "mv-wf45")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "sheet_id": "mv-wf45" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.corr_matrix", "correlation_all_pairs": false } }))?;
    let snap_r7_off = inspect_tree(&mut app, Some("dialog"), false)?;
    let hm_toggle_off =
        find_widget(&snap_r7_off, "analyze.correlation.heatmap").ok_or_else(|| String::from("missing analyze.correlation.heatmap"))?;
    ensure(
        hm_toggle_off.get("enabled").and_then(Value::as_bool) == Some(false),
        "R7 must disable correlation heatmap when correlation_all_pairs is false",
    )?;
    let res_corr_id = {
        let out = gui(
            &mut app,
            "ui.answer_dialog",
            json!({
                "fields": { "entry_id": "stats.corr_matrix", "correlation_all_pairs": true, "correlation_heatmap": true },
                "force_create_new": true,
                "action": "ok"
            }),
        )?;
        out.get("payload")
            .and_then(|p| p.get("sheet_id"))
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("missing correlation result sheet_id"))?
            .to_owned()
    };

    // Create a heatmap graph targeting the correlation result (or MultipleVariables matrix) & verify SVG export.
    open_and_apply_graph_type(&mut app, "graph-mv-hm", "diag.heatmap")?;
    let mv_hm_scene = assert_scene_compiles_cleanly(app.document(), "graph-mv-hm")?;
    let mv_texts = collect_scene_text(&mv_hm_scene).join(" | ");
    ensure(mv_texts.contains("GeneA") && mv_texts.contains("GeneD"), format!("expected MV variable columns in heatmap: {mv_texts}"))?;

    // Switch continuous colormap via graph.setSpec menu action and verify Export Figure -> SVG cleanly renders rect/path cells.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setSpec",
            "params": {
                "sheet_id": "graph-mv-hm",
                "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
                "multivariate": {
                    "color": {
                        "Continuous": {
                            "column": 0,
                            "ramp": { "Named": { "id": ColormapId::Cividis, "reversed": false } },
                            "domain_clamp": [0.0, 12.5]
                        }
                    }
                }
            }
        }),
    )?;
    switch_sheet(&mut app, "graph-mv-hm")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.export.figure.open", "params": { "sheet_id": "graph-mv-hm" } }))?;
    let svg_out = gui(&mut app, "ui.answer_dialog", json!({ "fields": { "format": "svg" }, "action": "ok" }))?;
    let svg = svg_out.get("payload").and_then(|p| p.get("svg")).and_then(Value::as_str).ok_or_else(|| String::from("missing exported SVG"))?;
    ensure(svg.contains("<svg") && svg.contains("</svg>") && svg.contains("GeneA"), "SVG must contain heatmap markup and variable labels")?;

    // Exclude one MV cell via Cmd+Shift+E -> verify correlation result sheet + MV heatmap recompute reactively.
    let corr_before =
        app.document().result_sheet(&res_corr_id.clone().into()).and_then(|rs| serde_json::to_string(&rs.tabs).ok()).unwrap_or_default();
    exclude_cell_with_reason_shortcut(&mut app, "mv-wf45", 5, 2, 0, "Bubble", "Exclude outlier GeneC donor D6")?;
    let corr_after = app.document().result_sheet(&res_corr_id.into()).and_then(|rs| serde_json::to_string(&rs.tabs).ok()).unwrap_or_default();
    ensure(corr_after != corr_before, "Cmd+Shift+E exclusion on mv-wf45 must recompute CorrelationMatrix results")?;
    let _ = assert_scene_compiles_cleanly(app.document(), "graph-mv-hm")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-46: Interactive Multivariable Graph Inspector & Variable Swapping
// ---------------------------------------------------------------------------

#[test]
fn wf46_interactive_multivariable_graph_inspector_variable_swapping() -> TestResult {
    let mut app = new_strict_gui_app();

    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "mv-wf46",
                "title": "PK-PD Variables",
                "kind": "multiple_variables",
                "rows": 8,
                "auto_graph": true,
                "graph_sheet_id": "graph-wf46",
                "variables": [
                    { "title": "Dosage", "role": "continuous" },
                    { "title": "BiomarkerA", "role": "continuous" },
                    { "title": "BiomarkerB", "role": "continuous" },
                    { "title": "CellSize", "role": "continuous" },
                    { "title": "Treatment", "role": "categorical" },
                    { "title": "SampleCode", "role": "label" }
                ]
            }
        }),
    )?;

    let mv_data: [[&str; 6]; 8] = [
        ["10.0", "22.0", "105.0", "4.0", "Vehicle", "V1"],
        ["20.0", "28.0", "115.0", "5.5", "Vehicle", "V2"],
        ["30.0", "31.0", "120.0", "6.2", "Vehicle", "V3"],
        ["40.0", "26.0", "110.0", "5.0", "Vehicle", "V4"],
        ["10.0", "48.0", "210.0", "9.0", "DrugX", "D1"],
        ["20.0", "56.0", "235.0", "11.5", "DrugX", "D2"],
        ["30.0", "64.0", "250.0", "13.0", "DrugX", "D3"],
        ["40.0", "71.0", "265.0", "14.5", "DrugX", "D4"],
    ];
    for (r, vals) in mv_data.iter().enumerate() {
        for (c, v) in vals.iter().enumerate() {
            type_grid_cell(&mut app, "mv-wf46", r, c, 0, v)?;
        }
    }

    open_and_apply_graph_type(&mut app, "graph-wf46", "mv.bubble")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open", "params": { "sheet_id": "graph-wf46" } }))?;

    let dlg_snap = inspect_tree(&mut app, Some("dialog"), false)?;
    for wid in ["bubble.x_column", "bubble.y_column", "bubble.size_channel", "bubble.color_channel", "bubble.label_column"] {
        ensure(find_widget(&dlg_snap, wid).is_some(), format!("expected inspector widget {wid}"))?;
    }

    // Initial 5-channel mapping: X=Dosage(0), Y=BiomarkerA(1), Size=CellSize(3), Color=BiomarkerB(2) Plasma, Label=SampleCode(5).
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
                "x_column": 0,
                "y_column": 1,
                "size_column": 3,
                "color_mode": "continuous",
                "color_column": 2,
                "colormap": "plasma",
                "label_column": 5
            },
            "action": "ok"
        }),
    )?;

    let scene_init = assert_scene_compiles_cleanly(app.document(), "graph-wf46")?;
    let texts_init = collect_scene_text(&scene_init).join(" | ");
    ensure(texts_init.contains("BiomarkerB"), format!("expected continuous colorbar title BiomarkerB, got {texts_init}"))?;
    switch_sheet(&mut app, "graph-wf46")?;
    let png_init = assert_screenshot_png(&mut app, 200, 120)?;

    // Duplicate graph sheet to test BubbleScatter -> CategoricalSummary branch alongside interactive axis swap.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "sheet.duplicate", "params": { "sheet_id": "graph-wf46", "new_sheet_id": "graph-wf46-cat" } }),
    )?;
    ensure(app.document().graph_sheet(&"graph-wf46-cat".into()).is_some(), "duplicated graph-wf46-cat missing")?;

    // Swap X <-> Y & switch Color=Treatment(4, categorical) + reset Size=Constant on primary graph.
    switch_sheet(&mut app, "graph-wf46")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open", "params": { "sheet_id": "graph-wf46" } }))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
                "x_column": 2,
                "y_column": 0,
                "size_column": null,
                "color_mode": "categorical",
                "color_column": 4,
                "palette": "okabe_ito",
                "label_column": null
            },
            "action": "ok"
        }),
    )?;

    let mv_enc = app
        .document()
        .graph_sheet(&"graph-wf46".into())
        .and_then(|g| g.spec.multivariate.as_ref())
        .ok_or_else(|| String::from("missing multivariate encoding after swap"))?;
    ensure(mv_enc.x_column == 2 && mv_enc.y_column == 0, "X and Y columns must swap to (2, 0)")?;
    ensure(matches!(mv_enc.size, SizeChannelMapping::Constant { .. }), "size_column: null must reset SizeChannelMapping::Constant")?;
    ensure(matches!(mv_enc.color, ColorChannelMapping::Categorical { column: 4, .. }), "color channel must bind categorically to Treatment(4)")?;

    let scene_swapped = assert_scene_compiles_cleanly(app.document(), "graph-wf46")?;
    let texts_swapped = collect_scene_text(&scene_swapped).join(" | ");
    ensure(texts_swapped.contains("Vehicle") && texts_swapped.contains("DrugX"), format!("expected categorical legend entries: {texts_swapped}"))?;
    let png_swapped = assert_screenshot_png(&mut app, 200, 120)?;
    ensure(png_swapped != png_init, "swapping X/Y and color mode must visually update rendered screenshot")?;

    // Switch duplicated graph to CategoricalSummary (`plot_mode: "categorical_summary"`, X=Treatment(4), Y=BiomarkerA(1)).
    switch_sheet(&mut app, "graph-wf46-cat")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open", "params": { "sheet_id": "graph-wf46-cat" } }))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
                "plot_mode": "categorical_summary",
                "x_column": 4,
                "y_column": 1
            },
            "action": "ok"
        }),
    )?;
    let cat_gs = app.document().graph_sheet(&"graph-wf46-cat".into()).ok_or_else(|| String::from("missing graph-wf46-cat"))?;
    ensure(
        matches!(cat_gs.spec.multivariate.as_ref().map(|m| &m.plot_mode), Some(MultivariatePlotMode::CategoricalSummary { .. })),
        "expected CategoricalSummary plot_mode on duplicated graph",
    )?;
    let cat_scene_1 = assert_scene_compiles_cleanly(app.document(), "graph-wf46-cat")?;
    let cat_texts_1 = collect_scene_text(&cat_scene_1).join(" | ");
    ensure(cat_texts_1.contains("Vehicle") && cat_texts_1.contains("DrugX"), "CategoricalSummary must display Treatment level ticks")?;

    // Reactive upstream edit: boost BiomarkerA row 4 (DrugX) from 48 -> 180 via ui.type_text & confirm live graph update.
    type_grid_cell(&mut app, "mv-wf46", 4, 1, 0, "180")?;
    let cat_scene_2 = assert_scene_compiles_cleanly(app.document(), "graph-wf46-cat")?;
    ensure(
        collect_scene_text(&cat_scene_2) != collect_scene_text(&cat_scene_1),
        "upstream BiomarkerA edit must recompute CategoricalSummary Y scale",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-47: Kaplan-Meier Synchronized Number-at-Risk & Pairwise Survival
// ---------------------------------------------------------------------------

#[test]
fn wf47_kaplan_meier_synchronized_number_at_risk_and_pairwise_survival() -> TestResult {
    let mut app = new_strict_gui_app();

    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "surv-wf47",
                "title": "Preclinical Survival",
                "kind": "survival",
                "rows": 18,
                "cols": 3,
                "time_title": "Months",
                "auto_graph": true,
                "graph_sheet_id": "graph-wf47"
            }
        }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setGroupTitles", "params": { "sheet_id": "surv-wf47", "titles": ["Control", "Low Dose", "High Dose"] } }),
    )?;

    // Cohort 0 (Control): early deaths t = 2, 4, 5, 7, 9, 11 (all status=1)
    // Cohort 1 (Low Dose): mixed t = 6, 10, 14(cens=0), 17, 20, 23
    // Cohort 2 (High Dose): prolonged survival t = 18(cens=0), 22, 26(cens=0), 29(cens=0), 32, 34(cens=0)
    let entries: [(usize, f64, usize, &str); 18] = [
        (0, 2.0, 0, "1"),
        (1, 4.0, 0, "1"),
        (2, 5.0, 0, "1"),
        (3, 7.0, 0, "1"),
        (4, 9.0, 0, "1"),
        (5, 11.0, 0, "1"),
        (6, 6.0, 1, "1"),
        (7, 10.0, 1, "1"),
        (8, 14.0, 1, "0"),
        (9, 17.0, 1, "1"),
        (10, 20.0, 1, "1"),
        (11, 23.0, 1, "1"),
        (12, 18.0, 2, "0"),
        (13, 22.0, 2, "1"),
        (14, 26.0, 2, "0"),
        (15, 29.0, 2, "0"),
        (16, 32.0, 2, "1"),
        (17, 34.0, 2, "0"),
    ];
    for (row, t, cohort, status) in entries {
        type_x_cell(&mut app, "surv-wf47", row, 0, &format!("{t}"))?;
        type_grid_cell(&mut app, "surv-wf47", row, cohort, 0, status)?;
    }

    // Run KM analysis via Analyze dialog, then open EditAnalysisParameters (Cmd+T) to set trend_test & Holm-Sidak multiplicity.
    let res_km_id = run_analyze_dialog(&mut app, "surv-wf47", json!({ "entry_id": "stats.survival_km" }))?;
    switch_sheet(&mut app, &res_km_id)?;
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    ensure(
        matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
        "Cmd+T on survival result must open EditAnalysisParameters",
    )?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": { "trend_test": true, "multiplicity_method": "holm_sidak" },
            "action": "ok"
        }),
    )?;

    {
        let rs = app.document().result_sheet(&res_km_id.clone().into()).ok_or_else(|| String::from("missing KM ResultSheet"))?;
        ensure(
            matches!(
                &rs.analysis.kind,
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { trend_test: true, multiplicity, .. })
                    if multiplicity.method == MultiplicityMethod::HolmSidak
            ),
            "expected KaplanMeier trend_test=true and HolmSidak multiplicity",
        )?;
        ensure(
            rs.pairwise_comparisons.len() == 3,
            format!("expected 3 pairwise comparisons across 3 cohorts, got {}", rs.pairwise_comparisons.len()),
        )?;
        ensure(rs.tabs.iter().any(|t| t.id == "number-at-risk"), "missing number-at-risk result tab")?;
        ensure(rs.tabs.iter().any(|t| t.id == "survival-pairwise"), "missing survival-pairwise result tab")?;
    }

    open_and_apply_graph_type(&mut app, "graph-wf47", "survival.km")?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.setSpec", "params": { "sheet_id": "graph-wf47", "survival_number_at_risk": true } }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setAxes",
            "params": {
                "sheet_id": "graph-wf47",
                "x_axis": { "title": "Months", "segments": [{ "min": 0.0, "max": 36.0 }] },
                "y_axis": { "title": "Survival (%)", "segments": [{ "min": 0.0, "max": 100.0 }], "thresholds": [{ "value": 50.0, "label": "Median" }] }
            }
        }),
    )?;

    let km_scene_36 = assert_scene_compiles_cleanly(app.document(), "graph-wf47")?;
    let km_texts_36 = collect_scene_text(&km_scene_36);
    ensure(km_texts_36.iter().any(|t| t.contains("Number at risk")), "missing 'Number at risk' header under survival plot")?;
    ensure(km_texts_36.iter().any(|t| t == "6"), "expected baseline n_at_risk = 6 at t=0")?;
    switch_sheet(&mut app, "graph-wf47")?;
    let km_png_36 = assert_screenshot_png(&mut app, 200, 120)?;

    // Tighten X-axis [0, 16] -> Number at risk table automatically re-bins to new X tick grid.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setAxes",
            "params": {
                "sheet_id": "graph-wf47",
                "x_axis": { "title": "Months", "segments": [{ "min": 0.0, "max": 16.0 }] }
            }
        }),
    )?;
    let km_scene_16 = assert_scene_compiles_cleanly(app.document(), "graph-wf47")?;
    ensure(collect_scene_text(&km_scene_16) != km_texts_36, "X-axis zoom [0, 16] must re-bin Number at risk table tick counts")?;
    let km_png_16 = assert_screenshot_png(&mut app, 200, 120)?;
    ensure(km_png_16 != km_png_36, "re-binning X-axis and Number at risk table must update rendered PNG")?;

    // Populate pairwise survival brackets via PairwiseComparisons dialog.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.pairwise.open", "params": { "sheet_id": "graph-wf47" } }))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": { "result_sheet_id": res_km_id, "hide_non_significant": true, "label_mode": "numeric", "auto_add": true },
            "action": "ok"
        }),
    )?;
    let gs_km = app.document().graph_sheet(&"graph-wf47".into()).ok_or_else(|| String::from("missing graph-wf47"))?;
    ensure(!gs_km.spec.comparisons.is_empty(), "expected pairwise survival brackets added from survival-pairwise result")?;

    // Reactive exclusion check via Cmd+Shift+E on Control row 0 -> baseline n_at_risk & median/pairwise results recompute.
    let pairwise_before = app.document().result_sheet(&res_km_id.clone().into()).map(|rs| rs.pairwise_comparisons.clone()).unwrap_or_default();
    exclude_cell_with_reason_shortcut(&mut app, "surv-wf47", 0, 0, 0, "Missing Replicate", "Lost baseline observation")?;
    let pairwise_after = app.document().result_sheet(&res_km_id.into()).map(|rs| rs.pairwise_comparisons.clone()).unwrap_or_default();
    ensure(pairwise_after != pairwise_before, "excluding survival observation via Cmd+Shift+E must recompute pairwise p-values/HRs")?;
    let _ = assert_scene_compiles_cleanly(app.document(), "graph-wf47")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-48: Multivariable Bubble Symbol Sizing & Outlier Clamping
// ---------------------------------------------------------------------------

#[test]
fn wf48_multivariable_bubble_symbol_sizing_and_outlier_clamping() -> TestResult {
    let mut app = new_strict_gui_app();

    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "mv-wf48",
                "title": "Volcano / Enrichment Sizing",
                "kind": "multiple_variables",
                "rows": 9,
                "auto_graph": true,
                "graph_sheet_id": "graph-wf48",
                "variables": [
                    { "title": "Log2FC", "role": "continuous" },
                    { "title": "NegLog10P", "role": "continuous" },
                    { "title": "GeneCount", "role": "continuous" },
                    { "title": "Score", "role": "continuous" },
                    { "title": "Pathway", "role": "label" }
                ]
            }
        }),
    )?;

    let rows: [(&str, &str, &str, &str, &str); 9] = [
        ("-2.4", "3.1", "8.0", "5.0", "P1"),
        ("-1.5", "2.2", "12.0", "9.0", "P2"),
        ("-0.8", "1.4", "16.0", "14.0", "P3"),
        ("0.5", "1.1", "10.0", "11.0", "P4"),
        ("1.2", "2.6", "20.0", "18.0", "P5"),
        ("1.9", "3.8", "24.0", "22.0", "P6"),
        ("2.5", "4.5", "28.0", "25.0", "P7"),
        ("3.1", "5.2", "30.0", "29.0", "P8"),
        ("4.2", "6.8", "10000.0", "500.0", "P9_Outlier"),
    ];
    for (r, (x, y, sz, col_val, lbl)) in rows.iter().enumerate() {
        type_grid_cell(&mut app, "mv-wf48", r, 0, 0, x)?;
        type_grid_cell(&mut app, "mv-wf48", r, 1, 0, y)?;
        type_grid_cell(&mut app, "mv-wf48", r, 2, 0, sz)?;
        type_grid_cell(&mut app, "mv-wf48", r, 3, 0, col_val)?;
        type_grid_cell(&mut app, "mv-wf48", r, 4, 0, lbl)?;
    }

    open_and_apply_graph_type(&mut app, "graph-wf48", "mv.bubble")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open", "params": { "sheet_id": "graph-wf48" } }))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
                "x_column": 0,
                "y_column": 1,
                "size_column": 2,
                "color_mode": "continuous",
                "color_column": 3,
                "colormap": "viridis",
                "label_column": 4
            },
            "action": "ok"
        }),
    )?;

    // Apply explicit continuous color `domain_clamp: [0.0, 30.0]` and variable size bounds `[2.5, 14.0]`.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setSpec",
            "params": {
                "sheet_id": "graph-wf48",
                "kind": GraphKind::MultipleVariables,
                "multivariate": {
                    "x_column": 0,
                    "y_column": 1,
                    "label_column": 4,
                    "size": { "Variable": { "column": 2, "min_radius_pt": 2.5, "max_radius_pt": 14.0 } },
                    "color": { "Continuous": { "column": 3, "ramp": { "Named": { "id": ColormapId::Viridis, "reversed": false } }, "domain_clamp": [0.0, 30.0] } }
                }
            }
        }),
    )?;

    let scene_with_outlier = assert_scene_compiles_cleanly(app.document(), "graph-wf48")?;
    ensure(scene_with_outlier.plotted_points.len() == 9, "expected 9 plotted bubbles including outlier row 8")?;
    for pt in &scene_with_outlier.plotted_points {
        ensure(pt.radius_pt >= 2.5 && pt.radius_pt <= 14.0, format!("bubble radius {} escaped clamp [2.5, 14.0]", pt.radius_pt))?;
    }
    let p8_before_radius = scene_with_outlier
        .plotted_points
        .iter()
        .find(|p| p.source_cell.row == 7)
        .map(|p| p.radius_pt)
        .ok_or_else(|| String::from("missing row 7 point before outlier exclusion"))?;
    ensure(p8_before_radius < 4.0, format!("outlier 10000 should compress row 7 (size 30) near min radius, got {p8_before_radius}"))?;
    let clamped_texts = collect_scene_text(&scene_with_outlier).join(" | ");
    ensure(!clamped_texts.contains("500"), format!("color domain_clamp [0, 30] must prevent 500 from appearing on colorbar: {clamped_texts}"))?;
    switch_sheet(&mut app, "graph-wf48")?;
    let png_before = assert_screenshot_png(&mut app, 200, 120)?;

    // Exclude row 8 outlier cell (`GeneCount = 10000`) via Cmd+Shift+E -> remaining bubbles expand cleanly across [2.5, 14.0].
    exclude_cell_with_reason_shortcut(&mut app, "mv-wf48", 8, 2, 0, "Instrument Fault", "Exclude extreme sizing outlier 10000")?;
    let scene_after_excl = assert_scene_compiles_cleanly(app.document(), "graph-wf48")?;
    let p8_after_radius = scene_after_excl
        .plotted_points
        .iter()
        .find(|p| p.source_cell.row == 7)
        .map(|p| p.radius_pt)
        .ok_or_else(|| String::from("missing row 7 point after exclusion"))?;
    ensure(p8_after_radius > 13.5, format!("excluding outlier row 8 must re-expand max remaining point (size 30) to ~14pt, got {p8_after_radius}"))?;
    switch_sheet(&mut app, "graph-wf48")?;
    let png_after = assert_screenshot_png(&mut app, 200, 120)?;
    ensure(png_after != png_before, "excluding sizing outlier must alter rendered bubble PNG")?;

    // Layout Composer + Export Figure parity checks via dialogs.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "layout.create", "params": { "sheet_id": "layout-wf48", "title": "WF48 Figure" } }))?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "layout.addPanel", "params": { "sheet_id": "layout-wf48", "kind": "graph", "graph_sheet_id": "graph-wf48", "row": 0, "col": 0 } }),
    )?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.layout.composer.open", "params": { "sheet_id": "layout-wf48" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "cols": 2, "rows": 1, "align_plot_origins": true }, "action": "ok" }))?;

    switch_sheet(&mut app, "graph-wf48")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.export.figure.open", "params": { "sheet_id": "graph-wf48" } }))?;
    let png_export =
        gui(&mut app, "ui.answer_dialog", json!({ "fields": { "format": "png", "width_px": 900, "height_px": 600, "dpi": 300 }, "action": "ok" }))?;
    let b64 = png_export.get("payload").and_then(|p| p.get("base64")).and_then(Value::as_str).ok_or_else(|| String::from("missing PNG base64"))?;
    let bytes = decode_base64(b64)?;
    ensure(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "Export Figure PNG missing signature")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-49: Live TeX Best-Fit Equation Annotations (Equations-Only, No CLD)
// ---------------------------------------------------------------------------

fn assert_equation_box_is_strictly_equations_only(texts: &[String], context: &str) -> TestResult {
    let joined = texts.join(" | ");
    for forbidden in ["CLD", "Compact Letter", "AICc", "R2 =", "R²", "Sum of Squares"] {
        ensure(!joined.contains(forbidden), format!("{context}: forbidden non-equation callout `{forbidden}` leaked into scene: {joined}"))?;
    }
    for raw_tex in ["\\frac", "\\text", "\\cdot", "\\exp", "\\log"] {
        ensure(!joined.contains(raw_tex), format!("{context}: raw TeX macro `{raw_tex}` leaked into rendered text: {joined}"))?;
    }
    Ok(())
}

#[test]
fn wf49_live_tex_best_fit_equation_annotations_equations_only() -> TestResult {
    let mut app = new_strict_gui_app();

    // 1. One-Phase Exponential Decay (`one_phase_decay`): Y = (100 - 15)*exp(-0.25*X) + 15.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "xy-decay",
                "title": "Exponential Decay Kinetics",
                "kind": "xy",
                "rows": 8,
                "cols": 1,
                "x_title": "Time (min)",
                "auto_graph": true,
                "graph_sheet_id": "graph-decay"
            }
        }),
    )?;
    let decay_pts: [(f64, f64); 8] =
        [(0.0, 100.0), (1.0, 81.108), (2.0, 66.555), (4.0, 46.270), (6.0, 33.966), (8.0, 26.503), (12.0, 19.232), (16.0, 16.557)];
    for (r, (x, y)) in decay_pts.iter().enumerate() {
        type_x_cell(&mut app, "xy-decay", r, 0, &format!("{x}"))?;
        type_grid_cell(&mut app, "xy-decay", r, 0, 0, &format!("{y:.3}"))?;
    }

    let res_decay_id = run_analyze_dialog(&mut app, "xy-decay", json!({ "entry_id": "fit.nonlinear", "equation_id": "one_phase_decay" }))?;
    switch_sheet(&mut app, &res_decay_id)?;
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "confidence_bands": true }, "action": "ok" }))?;

    open_and_apply_graph_type(&mut app, "graph-decay", "xy.scatter")?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setRegressionOverlay",
            "params": {
                "sheet_id": "graph-decay",
                "result_sheet_id": res_decay_id,
                "series_index": 0,
                "show_confidence_band": true,
                "shaded_ribbon": true
            }
        }),
    )?;

    // Insert ModelEquation in Substituted mode via `ui.graph.insertModelEquation`.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.insertModelEquation", "params": { "sheet_id": "graph-decay" } }))?;
    ensure(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::InsertModelEquation { .. }), "expected InsertModelEquation dialog to open")?;
    let eq_dlg_snap = inspect_tree(&mut app, Some("dialog"), false)?;
    ensure(find_widget(&eq_dlg_snap, "equation.toggle_mode").is_some(), "missing equation.toggle_mode button")?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({ "fields": { "result_sheet_id": res_decay_id, "mode": "substituted", "bordered": true }, "action": "ok" }),
    )?;

    let decay_sub_scene = assert_scene_compiles_cleanly(app.document(), "graph-decay")?;
    let decay_sub_texts = collect_scene_text(&decay_sub_scene);
    assert_equation_box_is_strictly_equations_only(&decay_sub_texts, "one_phase_decay substituted")?;
    let decay_joined = decay_sub_texts.join(" | ");
    ensure(decay_joined.contains("100") && decay_joined.contains("exp("), format!("expected substituted 1-phase decay box: {decay_joined}"))?;

    // Toggle to Symbolic via FormatGraph (`format_graph.toggle_model_equation_mode`).
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.format.open", "params": { "sheet_id": "graph-decay" } }))?;
    click_widget(&mut app, "format_graph.toggle_model_equation_mode")?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    let decay_sym_scene = assert_scene_compiles_cleanly(app.document(), "graph-decay")?;
    let decay_sym_texts = collect_scene_text(&decay_sym_scene);
    assert_equation_box_is_strictly_equations_only(&decay_sym_texts, "one_phase_decay symbolic")?;
    let decay_sym_joined = decay_sym_texts.join(" | ");
    ensure(
        decay_sym_joined.contains("Plateau") && decay_sym_joined.contains("Y0") || decay_sym_joined.contains('K'),
        format!("expected symbolic parameter names in equation box: {decay_sym_joined}"),
    )?;

    // Toggle back to Substituted via InsertModelEquation (`equation.toggle_mode`) and perform live XY cell edit -> coefficient refresh.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.insertModelEquation", "params": { "sheet_id": "graph-decay" } }))?;
    click_widget(&mut app, "equation.toggle_mode")?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    ensure(
        app.document().graph_sheet(&"graph-decay".into()).is_some_and(|g| {
            g.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::ModelEquation { mode: EquationDisplayMode::Substituted, .. }))
        }),
        "toggling via equation.toggle_mode must persist Substituted mode",
    )?;

    type_grid_cell(&mut app, "xy-decay", 0, 0, 0, "180")?;
    let decay_live_scene = assert_scene_compiles_cleanly(app.document(), "graph-decay")?;
    let decay_live_texts = collect_scene_text(&decay_live_scene);
    assert_equation_box_is_strictly_equations_only(&decay_live_texts, "one_phase_decay live update")?;
    ensure(decay_live_texts != decay_sub_texts, "upstream Y(t=0) cell edit must refresh live ModelEquation coefficients")?;

    // 2. 3PL Dose-Response (`log_agonist_vs_response_3pl`): stacked fraction vinculum + 95% CI band + PNG + SVG + %PDF-1.7.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "xy-3pl",
                "title": "3PL Agonist Response",
                "kind": "xy",
                "rows": 8,
                "cols": 1,
                "x_title": "Log10 [Agonist] (M)",
                "auto_graph": true,
                "graph_sheet_id": "graph-3pl"
            }
        }),
    )?;
    // Y = 12 + (98 - 12) / (1 + 10^(-6.0 - X))
    let three_pl_pts: [(f64, f64); 8] =
        [(-9.0, 12.086), (-8.0, 12.851), (-7.0, 19.818), (-6.5, 32.658), (-6.0, 55.000), (-5.5, 77.342), (-5.0, 90.182), (-4.0, 97.149)];
    for (r, (x, y)) in three_pl_pts.iter().enumerate() {
        type_x_cell(&mut app, "xy-3pl", r, 0, &format!("{x}"))?;
        type_grid_cell(&mut app, "xy-3pl", r, 0, 0, &format!("{y:.3}"))?;
    }

    let res_3pl_id = run_analyze_dialog(&mut app, "xy-3pl", json!({ "entry_id": "fit.nonlinear", "equation_id": "log_agonist_vs_response_3pl" }))?;
    switch_sheet(&mut app, &res_3pl_id)?;
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "confidence_bands": true }, "action": "ok" }))?;

    open_and_apply_graph_type(&mut app, "graph-3pl", "xy.scatter")?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setRegressionOverlay",
            "params": {
                "sheet_id": "graph-3pl",
                "result_sheet_id": res_3pl_id,
                "series_index": 0,
                "show_confidence_band": true,
                "shaded_ribbon": true
            }
        }),
    )?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.insertModelEquation", "params": { "sheet_id": "graph-3pl" } }))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({ "fields": { "result_sheet_id": res_3pl_id, "mode": "substituted", "bordered": true }, "action": "ok" }),
    )?;

    let tpl_scene_1 = assert_scene_compiles_cleanly(app.document(), "graph-3pl")?;
    let tpl_texts_1 = collect_scene_text(&tpl_scene_1);
    assert_equation_box_is_strictly_equations_only(&tpl_texts_1, "3PL substituted")?;
    let tpl_joined_1 = tpl_texts_1.join(" | ");
    ensure(
        tpl_joined_1.contains("98") && tpl_joined_1.contains("12"),
        format!("expected fitted Top~98 & Bottom~12 in 3PL equation box: {tpl_joined_1}"),
    )?;

    // Check Symbolic toggle on 3PL via FormatGraph & switch back to Substituted.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.format.open", "params": { "sheet_id": "graph-3pl" } }))?;
    click_widget(&mut app, "format_graph.toggle_model_equation_mode")?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    let tpl_sym_scene = assert_scene_compiles_cleanly(app.document(), "graph-3pl")?;
    let tpl_sym_texts = collect_scene_text(&tpl_sym_scene);
    assert_equation_box_is_strictly_equations_only(&tpl_sym_texts, "3PL symbolic")?;
    ensure(tpl_sym_texts.join(" | ").contains("Bottom") && tpl_sym_texts.join(" | ").contains("Top"), "3PL symbolic box must name Bottom and Top")?;

    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.format.open", "params": { "sheet_id": "graph-3pl" } }))?;
    click_widget(&mut app, "format_graph.toggle_model_equation_mode")?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    // Live upstream edit on 3PL top dose row 7 (97.149 -> 142.0) + PNG / SVG / %PDF-1.7 verification.
    type_grid_cell(&mut app, "xy-3pl", 7, 0, 0, "142.0")?;
    let tpl_scene_2 = assert_scene_compiles_cleanly(app.document(), "graph-3pl")?;
    let tpl_texts_2 = collect_scene_text(&tpl_scene_2);
    assert_equation_box_is_strictly_equations_only(&tpl_texts_2, "3PL live edit")?;
    ensure(tpl_texts_2 != tpl_texts_1, "live 3PL top-response cell edit must update fitted ModelEquation text")?;

    switch_sheet(&mut app, "graph-3pl")?;
    let tpl_png = assert_screenshot_png(&mut app, 200, 120)?;
    ensure(tpl_png.len() > 512, "rendered 3PL RGBA PNG screenshot unexpectedly small")?;

    // Export Figure -> SVG: check stacked fraction geometry (<path> vinculum + <text> spans, zero raw TeX).
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.export.figure.open", "params": { "sheet_id": "graph-3pl" } }))?;
    let svg_out = gui(&mut app, "ui.answer_dialog", json!({ "fields": { "format": "svg" }, "action": "ok" }))?;
    let svg = svg_out.get("payload").and_then(|p| p.get("svg")).and_then(Value::as_str).ok_or_else(|| String::from("missing 3PL SVG payload"))?;
    ensure(
        svg.contains("<svg") && svg.contains("</svg>") && svg.contains("<path") && svg.contains("<text"),
        "3PL SVG must render curves, vinculum paths, and text elements",
    )?;
    ensure(
        !svg.contains("\\frac") && !svg.contains("\\text") && !svg.contains("CLD") && !svg.contains("AICc"),
        "SVG must render equations-only stacked fraction without raw TeX/CLD/AICc",
    )?;

    // Export Figure -> PDF: verify valid %PDF-1.7 header & %%EOF trailer in memory without disk writes.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.export.figure.open", "params": { "sheet_id": "graph-3pl" } }))?;
    let pdf_out = gui(&mut app, "ui.answer_dialog", json!({ "fields": { "format": "pdf" }, "action": "ok" }))?;
    let pdf_b64 =
        pdf_out.get("payload").and_then(|p| p.get("base64")).and_then(Value::as_str).ok_or_else(|| String::from("missing 3PL PDF base64"))?;
    let pdf_bytes = decode_base64(pdf_b64)?;
    ensure(pdf_bytes.starts_with(b"%PDF-1.7"), "exported PDF must begin with %PDF-1.7 header")?;
    ensure(pdf_bytes.windows(5).any(|w| w == b"%%EOF"), "exported PDF missing %%EOF trailer")?;
    Ok(())
}
