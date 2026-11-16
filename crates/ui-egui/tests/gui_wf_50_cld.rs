//! Strict `--gui-only` end-to-end GUI acceptance workflow **`WF-50`**:
//! Compact Letter Display (Piepho 2004 insert-and-absorb + sweeping).
//!
//! Runs strictly under [`UiApp`] with `app.set_gui_only(true)` (`GuiOnlyPolicy::Strict`) and drives
//! all interactions exclusively through [`lumencraft_ui_egui::dispatch_gui_action`].
#![forbid(unsafe_code)]

use lumencraft_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use lumencraft_doc::{CldOrdering, GraphKind, PairwiseDisplayMode, ProjectDocument, SheetId};
use lumencraft_svg::{SvgExportOptions, render_svg, validate_svg_structure};
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

fn assert_control_enabled(snap: &Value, wid: &str, expected_enabled: bool) -> TestResult {
    let ctrl = find_widget(snap, wid).ok_or_else(|| format!("missing control `{wid}` in inspect snapshot"))?;
    if ctrl["enabled"].as_bool() != Some(expected_enabled) {
        return Err(format!("expected `{wid}` enabled={expected_enabled}, got {ctrl}"));
    }
    if !expected_enabled && ctrl["disabled_reason"].as_str().is_none_or(str::is_empty) {
        return Err(format!("expected non-empty disabled_reason for `{wid}`, got {ctrl}"));
    }
    Ok(())
}

fn switch_sheet(app: &mut UiApp, sheet_id: &str) -> Result<Value, String> {
    gui(app, "ui.switch_tab", json!({ "sheet_id": sheet_id }))
}

fn click_widget(app: &mut UiApp, widget_id: &str) -> Result<Value, String> {
    gui(app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": widget_id, "metrics": default_metrics() }))
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let clean: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if clean.is_empty() || !clean.len().is_multiple_of(4) {
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

fn assert_screenshot_png(app: &mut UiApp) -> Result<Vec<u8>, String> {
    let shot = gui(
        app,
        "ui.screenshot",
        json!({
            "target": "canvas",
            "width_px": 820,
            "height_px": 540,
            "dpi": 144,
            "metrics": default_metrics(),
        }),
    )?;
    ensure(shot["format"] == json!("png"), format!("expected png format, got {}", shot["format"]))?;
    ensure(shot["path"].is_null(), format!("expected null path under strict gui_only, got {}", shot["path"]))?;
    let b64 = shot.get("data_base64").and_then(Value::as_str).ok_or_else(|| String::from("missing data_base64"))?;
    let png = decode_base64(b64)?;
    let expected_len = shot.get("bytes_len").and_then(Value::as_u64).unwrap_or(0) as usize;
    ensure(png.len() == expected_len && png.len() > 256, format!("unexpected PNG byte count: decoded={}, reported={expected_len}", png.len()))?;
    ensure(png.starts_with(b"\x89PNG\r\n\x1a\n"), "screenshot lacks PNG magic header")?;
    Ok(png)
}

fn export_graph_svg(app: &mut UiApp, graph_id: &str) -> Result<String, String> {
    gui(app, "ui.invoke_menu", json!({ "command_id": "ui.export.figure.open", "params": { "sheet_id": graph_id } }))?;
    let out = gui(app, "ui.answer_dialog", json!({ "fields": { "format": "svg" }, "action": "ok" }))?;
    let svg = out.get("payload").and_then(|p| p.get("svg")).and_then(Value::as_str).ok_or_else(|| String::from("missing SVG export payload"))?;
    ensure(svg.contains("<svg") && svg.contains("</svg>") && svg.contains("<text"), "exported SVG missing structure/text elements")?;
    Ok(svg.to_owned())
}

fn export_graph_pdf(app: &mut UiApp, graph_id: &str) -> Result<Vec<u8>, String> {
    gui(app, "ui.invoke_menu", json!({ "command_id": "ui.export.figure.open", "params": { "sheet_id": graph_id } }))?;
    let out = gui(app, "ui.answer_dialog", json!({ "fields": { "format": "pdf" }, "action": "ok" }))?;
    let b64 = out.get("payload").and_then(|p| p.get("base64")).and_then(Value::as_str).ok_or_else(|| String::from("missing PDF export base64"))?;
    let bytes = decode_base64(b64)?;
    ensure(bytes.starts_with(b"%PDF-1.7"), "exported PDF must start with %PDF-1.7")?;
    ensure(bytes.windows(5).any(|w| w == b"%%EOF"), "exported PDF missing %%EOF trailer")?;
    Ok(bytes)
}

fn assert_scene_compiles_cleanly(doc: &ProjectDocument, graph_id: &str) -> Result<ChartScene, String> {
    let gs = doc.graph_sheet(&graph_id.into()).ok_or_else(|| format!("missing graph `{graph_id}`"))?;
    let scene = compile_graph(gs, doc).map_err(|e| format!("compile_graph({graph_id}) failed: {e}"))?;
    ensure(!scene.primitives.is_empty(), format!("empty compiled scene for `{graph_id}`"))?;
    ensure(scene.warnings.is_empty(), format!("unexpected scene warnings on `{graph_id}`: {:?}", scene.warnings))?;
    Ok(scene)
}

#[derive(Debug, Clone)]
struct TextNode {
    text: String,
    x: f64,
    y: f64,
}

fn scene_texts(scene: &ChartScene) -> Vec<TextNode> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => {
                Some(TextNode { text: t.rich.spans.iter().map(|s| s.text.as_str()).collect::<String>(), x: t.anchor_pt.x, y: t.anchor_pt.y })
            }
            _ => None,
        })
        .collect()
}

/// Find the 3 column-slot CLD labels ordered left-to-right by X inside the plot area.
fn extract_three_cld_labels(scene: &ChartScene) -> Result<[(String, f64, f64); 3], String> {
    let r = scene.plot_rect_pt;
    let mut candidates: Vec<TextNode> = scene_texts(scene)
        .into_iter()
        .filter(|t| {
            let s = t.text.as_str();
            !s.is_empty()
                && s.len() <= 3
                && s.chars().all(|c| c.is_ascii_alphabetic())
                && t.x > r.x + 1.0
                && t.x < r.right() - 1.0
                && t.y < r.bottom() - 2.0
                && t.y > 4.0
        })
        .collect();
    candidates.sort_by(|a, b| a.x.total_cmp(&b.x));
    if candidates.len() != 3 {
        return Err(format!("expected 3 CLD slot labels inside plot, found {}: {candidates:?}", candidates.len()));
    }
    Ok([
        (candidates[0].text.clone(), candidates[0].x, candidates[0].y),
        (candidates[1].text.clone(), candidates[1].x, candidates[1].y),
        (candidates[2].text.clone(), candidates[2].x, candidates[2].y),
    ])
}

fn extract_bracket_paths_and_labels(scene: &ChartScene) -> (Vec<f64>, Vec<TextNode>) {
    let bracket_y: Vec<f64> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: None, stroke: Some(_), .. } if path.commands.len() >= 2 => {
                // Significance brackets span vertically near the top of the plot and start/end inside X bounds.
                let ys: Vec<f64> = path
                    .commands
                    .iter()
                    .filter_map(|cmd| match cmd {
                        PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => Some(pt.y),
                        _ => None,
                    })
                    .collect();
                let xs: Vec<f64> = path
                    .commands
                    .iter()
                    .filter_map(|cmd| match cmd {
                        PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => Some(pt.x),
                        _ => None,
                    })
                    .collect();
                let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
                let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                if min_x > scene.plot_rect_pt.x + 2.0 && max_x < scene.plot_rect_pt.right() - 2.0 && (max_x - min_x) > 20.0 {
                    ys.into_iter().reduce(f64::min)
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();
    let star_or_p_labels: Vec<TextNode> = scene_texts(scene)
        .into_iter()
        .filter(|t| t.text.contains('*') || t.text == "ns" || t.text.contains("P =") || t.text.contains("P <"))
        .collect();
    (bracket_y, star_or_p_labels)
}

fn assert_strict_guards(app: &mut UiApp, sheet_id: &str) -> TestResult {
    ensure(app.is_gui_only(), "UiApp must stay in GuiOnlyPolicy::Strict")?;
    let bypass = json!({
        "id": "wf50-bypass",
        "method": "command.run",
        "command": "table.setCell",
        "params": { "sheet_id": sheet_id, "row": 0, "col": 0, "value": 999.0 }
    })
    .to_string();
    let reply: Value = serde_json::from_str(&app.dispatch_control_line(&bypass)).map_err(|e| format!("invalid control reply: {e}"))?;
    ensure(
        reply["ok"] == json!(false) && reply["error_detail"]["code"] == json!("gui_only_violation"),
        format!("expected gui_only_violation on command.run, got {reply}"),
    )?;

    let blocked = dispatch_gui_action(
        app,
        "ui.screenshot",
        json!({ "target": "canvas", "width_px": 320, "height_px": 240, "path": "/tmp/forbidden_wf50_shot.png" }),
    );
    ensure(blocked.is_err(), "expected screenshot disk path to fail under strict gui_only")?;
    Ok(())
}

#[test]
fn wf50_compact_letter_display_piepho_reactive_svg_pdf_rgba() -> TestResult {
    let mut app = new_strict_gui_app();

    // 1. Create a 3-group Column table (`Ctrl`, `Mid`, `High`) and populate via TSV entry (`ui.type_text`).
    // Ctrl = [7.5, 9, 10, 11, 12.5] (mean 10.0), Mid = [10, 11.5, 12.5, 13.5, 15] (mean 12.5),
    // High = [12.5, 14, 15, 16, 17.5] (mean 15.0) => High vs Ctrl sig (p < 0.01), adjacent pairs NS => (b, ab, a).
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "col-wf50",
                "title": "Dose Cohort CLD",
                "kind": "column",
                "rows": 5,
                "cols": 3,
                "auto_graph": true,
                "graph_sheet_id": "graph-wf50"
            }
        }),
    )?;
    assert_strict_guards(&mut app, "col-wf50")?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setTitles", "params": { "sheet_id": "col-wf50", "titles": ["Ctrl", "Mid", "High"] } }),
    )?;
    switch_sheet(&mut app, "col-wf50")?;
    gui(
        &mut app,
        "ui.type_text",
        json!({
            "widget_id": "grid.cell.0.0.0",
            "text": "7.5\t10.0\t12.5\n9.0\t11.5\t14.0\n10.0\t12.5\t15.0\n11.0\t13.5\t16.0\n12.5\t15.0\t17.5",
            "clear_existing": true
        }),
    )?;

    // 2. Run parametric One-Way ANOVA (Tukey HSD) AND nonparametric Kruskal-Wallis + Dunn's ResultSheets.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "sheet_id": "col-wf50" } }))?;
    let tukey_out =
        gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.one_way_anova" }, "force_create_new": true, "action": "ok" }))?;
    let tukey_res_id = tukey_out["payload"]["sheet_id"].as_str().ok_or_else(|| String::from("missing Tukey result sheet_id"))?.to_owned();

    switch_sheet(&mut app, "col-wf50")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "sheet_id": "col-wf50" } }))?;
    let kw_out =
        gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.one_way_anova" }, "force_create_new": true, "action": "ok" }))?;
    let dunn_res_id = kw_out["payload"]["sheet_id"].as_str().ok_or_else(|| String::from("missing Kruskal-Wallis result sheet_id"))?.to_owned();
    switch_sheet(&mut app, &dunn_res_id)?;
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    click_widget(&mut app, "edit.assume_gaussian")?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    ensure(
        app.document().result_sheet(&dunn_res_id.clone().into()).is_some_and(|rs| rs.title().contains("Kruskal-Wallis")),
        "expected second analysis switched to Kruskal-Wallis + Dunn's",
    )?;

    // 3. Apply Bar-with-Replicates column graph (`col.bar_replicates`) and capture clean baseline PNG.
    switch_sheet(&mut app, "graph-wf50")?;
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open", "params": { "sheet_id": "graph-wf50" } }))?;
    }
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "selected_option_id": "col.bar_replicates" }, "action": "ok" }))?;
    let baseline_png = assert_screenshot_png(&mut app)?;

    // 4. Open Pairwise Comparisons dialog (`ui.graph.pairwise.open`), verify CLD gating in `ui.inspect`,
    //    enable CLD via pointer click (`pairwise.cld_enabled`), confirm OK (`CompactLetterDisplayOnly`).
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.pairwise.open", "params": { "sheet_id": "graph-wf50" } }))?;
    let dlg_off = inspect_tree(&mut app, Some("dialog"), false)?;
    assert_control_enabled(&dlg_off, "pairwise.display_mode", true)?;
    assert_control_enabled(&dlg_off, "pairwise.cld_enabled", true)?;
    assert_control_enabled(&dlg_off, "pairwise.cld_ordering", false)?;
    assert_control_enabled(&dlg_off, "pairwise.cld_uppercase", false)?;
    ensure(click_widget(&mut app, "pairwise.cld_uppercase").is_err(), "clicking disabled pairwise.cld_uppercase must be rejected")?;

    click_widget(&mut app, "pairwise.cld_enabled")?;
    let dlg_on = inspect_tree(&mut app, Some("dialog"), false)?;
    assert_control_enabled(&dlg_on, "pairwise.cld_ordering", true)?;
    assert_control_enabled(&dlg_on, "pairwise.cld_uppercase", true)?;
    ensure(
        find_widget(&dlg_on, "pairwise.display_mode").and_then(|n| n.get("value")).and_then(Value::as_str) == Some("compact_letter_display_only"),
        format!("enabling CLD without brackets should switch display_mode to compact_letter_display_only: {dlg_on}"),
    )?;

    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "result_sheet_id": tukey_res_id }, "action": "ok" }))?;

    let scene_cld_only = assert_scene_compiles_cleanly(app.document(), "graph-wf50")?;
    let cld_only_labels = extract_three_cld_labels(&scene_cld_only)?;
    ensure(
        cld_only_labels[0].0 == "b" && cld_only_labels[1].0 == "ab" && cld_only_labels[2].0 == "a",
        format!("expected Piepho DescendingMean letters [b, ab, a], got {cld_only_labels:?}"),
    )?;

    let cld_only_png = assert_screenshot_png(&mut app)?;
    ensure(cld_only_png != baseline_png, "enabling CLD must change rendered RGBA PNG pixels")?;

    let svg_cld_only = export_graph_svg(&mut app, "graph-wf50")?;
    validate_svg_structure(&svg_cld_only).map_err(|e| format!("invalid CLD SVG: {e}"))?;
    ensure(
        svg_cld_only.contains(">b<") && svg_cld_only.contains(">ab<") && svg_cld_only.contains(">a<"),
        "SVG export missing rendered CLD letters (`a`, `ab`, `b`)",
    )?;
    // Cross-check direct Scene->SVG pipeline as well.
    let direct_svg = render_svg(&scene_cld_only, &SvgExportOptions::default()).map_err(|e| format!("render_svg failed: {e}"))?;
    validate_svg_structure(&direct_svg).map_err(|e| format!("direct render_svg invalid: {e}"))?;
    let pdf_cld_only = export_graph_pdf(&mut app, "graph-wf50")?;

    // 5. Reopen Pairwise Comparisons: auto-add significance brackets + switch to AscendingMean & Uppercase (`A`, `AB`, `B`)
    //    alongside brackets (`BracketsAndCompactLetterDisplay`), verifying non-overlapping vertical layout!
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.pairwise.open", "params": { "sheet_id": "graph-wf50" } }))?;
    let added = click_widget(&mut app, "pairwise.auto_add")?;
    ensure(added["added"].as_u64().unwrap_or(0) >= 1, format!("expected auto_add >= 1 significant bracket: {added}"))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
                "display_mode": "brackets_and_compact_letter_display",
                "cld_ordering": "ascending_mean",
                "cld_uppercase": true,
                "cld_font_size_pt": 10.0,
                "cld_offset_pt": 7.0
            },
            "action": "ok"
        }),
    )?;

    {
        let gs = app.document().graph_sheet(&SheetId::try_new("graph-wf50").map_err(|e| e.to_string())?).ok_or("missing graph-wf50")?;
        ensure(gs.spec.kind == GraphKind::Column, "expected Column graph")?;
        ensure(gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsAndCompactLetterDisplay, "expected BracketsAndCompactLetterDisplay")?;
        let cld_spec = gs.spec.compact_letter_display.as_ref().ok_or("expected compact_letter_display set")?;
        ensure(cld_spec.ordering == CldOrdering::AscendingMean && cld_spec.uppercase, "expected AscendingMean + uppercase CLD spec")?;
        ensure(!gs.spec.comparisons.is_empty(), "expected non-empty comparison brackets alongside CLD")?;
    }

    let scene_both = assert_scene_compiles_cleanly(app.document(), "graph-wf50")?;
    let both_labels = extract_three_cld_labels(&scene_both)?;
    ensure(
        both_labels[0].0 == "A" && both_labels[1].0 == "AB" && both_labels[2].0 == "B",
        format!("expected Piepho AscendingMean uppercase letters [A, AB, B], got {both_labels:?}"),
    )?;
    let (bracket_ys, star_labels) = extract_bracket_paths_and_labels(&scene_both);
    ensure(
        !bracket_ys.is_empty() && !star_labels.is_empty(),
        "expected both bracket geometry and significance labels in BracketsAndCompactLetterDisplay mode",
    )?;
    let max_star_or_bracket_y = bracket_ys.iter().copied().chain(star_labels.iter().map(|t| t.y)).fold(f64::NEG_INFINITY, f64::max);
    let min_cld_y = both_labels.iter().map(|t| t.2).fold(f64::INFINITY, f64::min);
    ensure(
        max_star_or_bracket_y < min_cld_y - 2.0,
        format!("brackets/stars (max_y={max_star_or_bracket_y:.2}) must sit strictly above CLD letters (min_y={min_cld_y:.2}) without overlap"),
    )?;

    let both_png = assert_screenshot_png(&mut app)?;
    ensure(both_png != cld_only_png, "adding brackets + uppercase ascending CLD must update RGBA PNG")?;
    let both_svg = export_graph_svg(&mut app, "graph-wf50")?;
    ensure(both_svg.contains(">A<") && both_svg.contains(">AB<") && both_svg.contains(">B<"), "SVG export missing uppercase [A, AB, B]")?;

    // 6. Switch CLD ResultSheet source to Dunn's nonparametric comparisons (`dunn_res_id`) + Box-and-Whisker plot!
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.pairwise.open", "params": { "sheet_id": "graph-wf50" } }))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": {
                "result_sheet_id": dunn_res_id,
                "display_mode": "compact_letter_display_only",
                "cld_ordering": "descending_mean",
                "cld_uppercase": false
            },
            "action": "ok"
        }),
    )?;
    {
        let gs = app.document().graph_sheet(&SheetId::try_new("graph-wf50").map_err(|e| e.to_string())?).ok_or("missing graph-wf50")?;
        ensure(
            gs.spec.compact_letter_display.as_ref().is_some_and(|c| c.result_sheet_id.as_str() == dunn_res_id),
            "CLD should bind cleanly to Kruskal-Wallis + Dunn ResultSheet",
        )?;
    }
    let scene_dunn = assert_scene_compiles_cleanly(app.document(), "graph-wf50")?;
    let dunn_labels = extract_three_cld_labels(&scene_dunn)?;
    ensure(
        dunn_labels[0].0 == "b" && dunn_labels[1].0 == "ab" && dunn_labels[2].0 == "a",
        format!("expected Dunn DescendingMean letters [b, ab, a], got {dunn_labels:?}"),
    )?;

    // Switch back to Tukey ResultSheet & test Box-and-Whisker plot placement (`col.box_whisker`).
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.pairwise.open", "params": { "sheet_id": "graph-wf50" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "result_sheet_id": tukey_res_id }, "action": "ok" }))?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open", "params": { "sheet_id": "graph-wf50" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "selected_option_id": "col.box_whisker" }, "action": "ok" }))?;
    let scene_box = assert_scene_compiles_cleanly(app.document(), "graph-wf50")?;
    let box_labels = extract_three_cld_labels(&scene_box)?;
    ensure(
        box_labels[0].0 == "b" && box_labels[1].0 == "ab" && box_labels[2].0 == "a",
        format!("expected Box-and-Whisker CLD [b, ab, a], got {box_labels:?}"),
    )?;
    let pre_edit_png = assert_screenshot_png(&mut app)?;

    // 7. Live upstream cell edit on `Mid` (col 1: 12.5 -> 22.0) -> reactive DAG updates ANOVA & CLD letters to `[c, a, b]`!
    switch_sheet(&mut app, "col-wf50")?;
    gui(&mut app, "ui.type_text", json!({ "widget_id": "grid.cell.0.1.0", "text": "19.5\n21.0\n22.0\n23.0\n24.5", "clear_existing": true }))?;
    let scene_edited = assert_scene_compiles_cleanly(app.document(), "graph-wf50")?;
    let edited_labels = extract_three_cld_labels(&scene_edited)?;
    ensure(
        edited_labels[0].0 == "c" && edited_labels[1].0 == "a" && edited_labels[2].0 == "b",
        format!("after elevating Mid above High, expected live CLD [c, a, b], got {edited_labels:?}"),
    )?;

    switch_sheet(&mut app, "graph-wf50")?;
    let edited_png = assert_screenshot_png(&mut app)?;
    ensure(edited_png != pre_edit_png, "live cell edit recomputing CLD must update RGBA PNG screenshot")?;
    let edited_svg = export_graph_svg(&mut app, "graph-wf50")?;
    validate_svg_structure(&edited_svg).map_err(|e| format!("edited SVG invalid: {e}"))?;
    ensure(edited_svg.contains(">c<") && edited_svg.contains(">a<") && edited_svg.contains(">b<"), "edited SVG missing [c, a, b] letters")?;
    let edited_pdf = export_graph_pdf(&mut app, "graph-wf50")?;
    ensure(edited_pdf != pdf_cld_only, "edited %PDF-1.7 export must differ after live CLD change")?;

    // 8. Undo (`Cmd+Z`) & Redo (`Cmd+Shift+Z`) via `ui.press_key` restore `[b, ab, a]` then `[c, a, b]`.
    gui(&mut app, "ui.press_key", json!({ "key": "z", "modifiers": { "meta": true } }))?;
    let scene_undone = assert_scene_compiles_cleanly(app.document(), "graph-wf50")?;
    let undone_labels = extract_three_cld_labels(&scene_undone)?;
    ensure(
        undone_labels[0].0 == "b" && undone_labels[1].0 == "ab" && undone_labels[2].0 == "a",
        format!("Cmd+Z undo must restore CLD letters [b, ab, a], got {undone_labels:?}"),
    )?;

    gui(&mut app, "ui.press_key", json!({ "key": "z", "modifiers": { "meta": true, "shift": true } }))?;
    let scene_redone = assert_scene_compiles_cleanly(app.document(), "graph-wf50")?;
    let redone_labels = extract_three_cld_labels(&scene_redone)?;
    ensure(
        redone_labels[0].0 == "c" && redone_labels[1].0 == "a" && redone_labels[2].0 == "b",
        format!("Cmd+Shift+Z redo must restore CLD letters [c, a, b], got {redone_labels:?}"),
    )?;

    Ok(())
}
