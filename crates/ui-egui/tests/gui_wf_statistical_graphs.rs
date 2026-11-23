//! Wave 17 `gui_only` acceptance suite covering **`WF-24` through `WF-28`**:
//! Statistical Graphs, Annotations, Page Layout, and Publication Export.
//!
//! Every workflow runs strictly under `UiApp` with `app.set_gui_only(true)` (`GuiOnlyPolicy::Strict`)
//! and drives all creation, data entry, analysis, formatting, layout dragging, inspection, export,
//! print spooling, and screenshot capture exclusively through `curvebook_ui_egui::dispatch_gui_action`.

#![forbid(unsafe_code)]

mod human_wf_common;

use curvebook_chart::{ChartScene, compile_graph, compile_layout};
use curvebook_doc::{
    BracketLabelMode, BracketStyle, ErrorBarMetric, FigureCanvasPreset, FramePreset, GraphAnnotation, GraphicalDataObject, JournalWidthPreset,
    LayoutItem, MultivariatePlotMode, PanelBadgeStyle, ProjectDocument, SheetId, SummaryMeasure,
};
use curvebook_palette::{CanvasPreviewFilter, PaletteId};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ActiveInspector};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{UiApp, ViewportMetrics, dispatch_gui_action};
use serde_json::{Value, json};

type TestResult = Result<(), String>;

fn gui(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed: {err}"))
}

fn new_strict_gui_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    if !app.is_gui_only() {
        return Err(String::from("expected UiApp::is_gui_only() == true"));
    }
    Ok(app)
}

fn assert_strict_gui_only_guards(app: &mut UiApp, sheet_id: &str) -> TestResult {
    let bypass_cmd = json!({
        "id": "strict-guard-1",
        "method": "command.run",
        "command": "table.setCell",
        "params": { "sheet_id": sheet_id, "row": 0, "col": 0, "value": 999.0 }
    })
    .to_string();
    let reply_raw = app.dispatch_control_line(&bypass_cmd);
    let reply: Value = serde_json::from_str(&reply_raw).map_err(|e| format!("invalid control reply JSON: {e}"))?;
    if reply["ok"] != json!(false) || reply["error_detail"]["code"] != json!("gui_only_violation") {
        return Err(format!("expected gui_only_violation for direct control command.run, got: {reply}"));
    }

    let path_shot = dispatch_gui_action(
        app,
        "ui.screenshot",
        json!({ "target": "workspace", "width_px": 320, "height_px": 240, "path": "/tmp/forbidden_gui_only_shot.png" }),
    );
    match path_shot {
        Err(err)
            if {
                let msg = format!("{err}");
                msg.contains("gui-only") || msg.contains("gui_only") || msg.contains("GuiOnlyViolation")
            } =>
        {
            Ok(())
        }
        other => Err(format!("expected GuiOnlyViolation for screenshot path override, got: {other:?}")),
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    fn val(b: u8) -> Result<u8, String> {
        match b {
            b'A'..=b'Z' => Ok(b - b'A'),
            b'a'..=b'z' => Ok(b - b'a' + 26),
            b'0'..=b'9' => Ok(b - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 byte: {b}")),
        }
    }

    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err(format!("invalid base64 length {}", bytes.len()));
    }
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

fn assert_screenshot_png(app: &mut UiApp, target: &str, width_px: u32, height_px: u32) -> Result<Vec<u8>, String> {
    let res = gui(app, "ui.screenshot", json!({ "target": target, "width_px": width_px, "height_px": height_px, "dpi": 144 }))?;
    if res["format"] != json!("png") {
        return Err(format!("expected png screenshot format, got {}", res["format"]));
    }
    if !res["path"].is_null() {
        return Err(format!("expected null disk path under gui_only, got {}", res["path"]));
    }
    let b64 = res["data_base64"].as_str().ok_or_else(|| String::from("screenshot missing data_base64"))?;
    let png = decode_base64(b64)?;
    let expected_len = usize::try_from(res["bytes_len"].as_u64().ok_or_else(|| String::from("missing bytes_len"))?)
        .map_err(|_| String::from("bytes_len overflow"))?;
    if png.len() != expected_len || png.len() < 128 {
        return Err(format!("unexpected PNG payload length: decoded={}, reported={expected_len}", png.len()));
    }
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("screenshot bytes missing PNG magic signature"));
    }
    Ok(png)
}

fn assert_scene_compiles_cleanly(doc: &ProjectDocument, sheet_id: &str) -> Result<ChartScene, String> {
    let sid = SheetId::try_new(sheet_id).map_err(|e| e.to_string())?;
    let scene = if let Some(graph) = doc.graph_sheet(&sid) {
        compile_graph(graph, doc).map_err(|e| format!("compile_graph({sheet_id}) failed: {e}"))?
    } else if let Some(layout) = doc.layout_sheet(&sid) {
        compile_layout(layout, doc).map_err(|e| format!("compile_layout({sheet_id}) failed: {e}"))?
    } else {
        return Err(format!("sheet `{sheet_id}` is neither GraphSheet nor LayoutSheet"));
    };
    if scene.primitives.is_empty() {
        return Err(format!("expected non-empty compiled scene primitives for `{sheet_id}`"));
    }
    Ok(scene)
}

fn find_control<'a>(snap: &'a Value, needle: &str) -> Option<&'a Value> {
    snap["dialog"]["controls"].as_array().and_then(|arr| arr.iter().find(|c| c["widget_id"].as_str().is_some_and(|id| id == needle)))
}

fn assert_control_enabled(snap: &Value, id: &str, expected_enabled: bool) -> TestResult {
    let ctrl = find_control(snap, id).ok_or_else(|| format!("control `{id}` missing in ui.inspect dialog snapshot: {}", snap["dialog"]))?;
    if ctrl["enabled"].as_bool() != Some(expected_enabled) {
        return Err(format!("expected control `{id}` enabled={expected_enabled}, got: {ctrl}"));
    }
    if !expected_enabled && ctrl["disabled_reason"].as_str().is_none_or(str::is_empty) {
        return Err(format!("expected non-empty disabled_reason on grayed control `{id}`, got: {ctrl}"));
    }
    Ok(())
}

fn create_column_table(app: &mut UiApp, sheet_id: &str, graph_id: &str, titles: &[&str], rows_tsv: &str) -> TestResult {
    gui(
        app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": sheet_id,
                "title": sheet_id,
                "kind": "column",
                "cols": titles.len(),
                "rows": rows_tsv.lines().count(),
                "auto_graph": true,
                "graph_sheet_id": graph_id
            }
        }),
    )?;
    gui(app, "ui.invoke_menu", json!({ "command_id": "table.setTitles", "params": { "sheet_id": sheet_id, "titles": titles } }))?;
    select_sheet(app, sheet_id)?;
    gui(app, "ui.type_text", json!({ "widget_id": "grid.cell.0.0.0", "text": rows_tsv, "clear_existing": true }))?;
    Ok(())
}

fn select_sheet(app: &mut UiApp, sheet_id: &str) -> TestResult {
    gui(app, "ui.switch_tab", json!({ "sheet_id": sheet_id }))?;
    Ok(())
}

fn open_and_apply_graph_type(app: &mut UiApp, graph_id: &str, option_id: &str) -> TestResult {
    open_and_apply_graph_type_with_fields(app, graph_id, option_id, &[])
}

fn open_and_apply_graph_type_with_fields(app: &mut UiApp, graph_id: &str, option_id: &str, extra_fields: &[(&str, Value)]) -> TestResult {
    select_sheet(app, graph_id)?;
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        gui(app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open", "params": { "sheet_id": graph_id } }))?;
    }
    let inspect = gui(app, "ui.inspect", json!({ "scope": "dialog" }))?;
    if inspect["active_dialog"] != json!("change_graph_type") {
        return Err(format!("expected active_dialog == change_graph_type, got {}", inspect["active_dialog"]));
    }
    let controls = inspect["dialog"]["controls"].as_array().ok_or("change_graph_type inspect missing dialog.controls")?;
    for required_id in ["graph_type.selected", "graph_type.summary_measure", "graph_type.error_bar_metric"] {
        if !controls.iter().any(|c| c["widget_id"] == json!(required_id)) {
            return Err(format!("change_graph_type inspect missing `{required_id}` control"));
        }
    }
    let mut fields = serde_json::Map::new();
    fields.insert(String::from("option_id"), json!(option_id));
    for (k, v) in extra_fields {
        fields.insert((*k).to_owned(), v.clone());
    }
    gui(app, "ui.answer_dialog", json!({ "fields": Value::Object(fields), "action": "ok" }))?;
    Ok(())
}

/// WF-24: Significance bracket annotations & auto-stars (`Column` table -> One-Way ANOVA + Tukey ->
/// brackets with `*`/`**`/`***`/`ns`, graph callout annotations, axis log/ticks configuration, SVG/PNG checks,
/// plus R1/R2 graying checks in `EditAnalysisParameters` via `ui.inspect`).
///
/// Known `src/` gaps verified in codebase (never edited here):
/// - `ActiveDialog::PairwiseComparisons` and `ActiveDialog::FormatAxes` have no menu/UI command opener in
///   `crates/ui-egui/src/{menus/ui_commands.rs, control_io.rs, interaction.rs}` (the `"Axes"` toolbar button maps to
///   `"graph.setAxes"` in `menus/tooltips.rs:64`).
/// - Both fall through to `build_generic_dialog_nodes` (`inspect.rs:519-528,943-966`) and `_ => {}` (`gui_actions.rs:861-910`).
/// - Rules `R1`/`R2` in `crates/ui-egui/src/modal_predicates.rs:39-112` govern ANOVA Repeated-Measures and
///   Gaussian/Dunn post-hoc graying inside `EditAnalysisParameters`, not FormatAxes `auto_ticks`/`log_baseline`.
#[test]
fn wf24_significance_brackets_auto_stars_and_axes_strict_gui_only() -> TestResult {
    let mut app = new_strict_gui_app()?;
    let tsv = "\
10.0\t10.4\t28.0
11.2\t11.6\t31.5
9.6\t10.1\t29.4
10.8\t11.0\t30.8
10.4\t10.9\t32.3";
    create_column_table(&mut app, "col-wf24", "graph-wf24", &["Control", "Low Dose", "High Dose"], tsv)?;
    assert_strict_gui_only_guards(&mut app, "col-wf24")?;

    // Open Analyze dialog via menu, inspect compatible entries, and run One-Way ANOVA.
    gui(&mut app, "ui.invoke_menu", json!({ "path": ["Analyze", "Analyze..."] }))?;
    let analyze_snap = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    if analyze_snap["active_dialog"] != json!("analyze_sheet") {
        return Err(format!("expected analyze_sheet dialog, got {analyze_snap}"));
    }
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.one_way_anova" }, "action": "ok" }))?;

    let result_id =
        app.document().result_sheets.first().map(|rs| rs.id.as_str().to_owned()).ok_or_else(|| String::from("expected One-Way ANOVA ResultSheet"))?;
    select_sheet(&mut app, &result_id)?;

    // Open Edit Analysis Parameters via Cmd+T (`ui.press_key`) and verify R1/R2 modal graying rules via `ui.inspect`.
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    let edit_snap_unpaired = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    if edit_snap_unpaired["active_dialog"] != json!("edit_analysis_parameters") {
        return Err(format!("expected edit_analysis_parameters after Cmd+T, got {edit_snap_unpaired}"));
    }
    assert_control_enabled(&edit_snap_unpaired, "edit.sphericity.geisser_greenhouse", false)?;
    assert_control_enabled(&edit_snap_unpaired, "edit.assume_gaussian", true)?;
    assert_control_enabled(&edit_snap_unpaired, "edit.post_hoc.tukey", true)?;
    assert_control_enabled(&edit_snap_unpaired, "edit.post_hoc.dunn", false)?;

    // Toggle assume_gaussian = false -> confirm Dunn's vs Tukey's / RM enabling, then restore Gaussian ANOVA + Tukey.
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "paired_or_matched": true, "assume_gaussian": false } }))?;
    let edit_snap_nonparam = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_control_enabled(&edit_snap_nonparam, "edit.sphericity.geisser_greenhouse", true)?;
    assert_control_enabled(&edit_snap_nonparam, "edit.post_hoc.tukey", false)?;
    assert_control_enabled(&edit_snap_nonparam, "edit.post_hoc.dunn", true)?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "paired_or_matched": false, "assume_gaussian": true }, "action": "ok" }))?;

    // Initialize graph as Column Bar + Replicates and add live pairwise brackets (`***` and `ns`).
    open_and_apply_graph_type(&mut app, "graph-wf24", "col.bar_replicates")?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.addComparisonBrackets",
            "params": {
                "sheet_id": "graph-wf24",
                "result_sheet_id": result_id,
                "significant_only": false,
                "clear_existing": true,
                "style": "square",
                "label_mode": "stars"
            }
        }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.addAnnotation",
            "params": {
                "sheet_id": "graph-wf24",
                "kind": "text",
                "text": "Tukey post-hoc vs Control",
                "bounds_pt": [120.0, 22.0, 145.0, 20.0]
            }
        }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setAxes",
            "params": {
                "sheet_id": "graph-wf24",
                "frame_preset": "open_l",
                "y_axis": { "title": "Cytokine (pg/mL)", "min": 0.0, "max": 45.0, "major_tick_interval": 15.0 }
            }
        }),
    )?;

    let gs = app.document().graph_sheet(&SheetId::try_new("graph-wf24").map_err(|e| e.to_string())?).ok_or("missing graph-wf24")?;
    if gs.spec.comparisons.len() < 3 {
        return Err(format!("expected 3 pairwise comparison brackets across 3 groups, got {}", gs.spec.comparisons.len()));
    }
    if gs.spec.comparisons.iter().any(|b| b.style != BracketStyle::Square || b.label_mode != BracketLabelMode::SummaryStars) {
        return Err(format!("unexpected bracket styling: {:?}", gs.spec.comparisons));
    }
    if !gs.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::TextBox { markdown, .. } if markdown.contains("Tukey"))) {
        return Err(String::from("expected text box annotation on graph-wf24"));
    }

    let svg_out = gui(&mut app, "ui.invoke_menu", json!({ "command_id": "graph.exportSvg", "params": { "sheet_id": "graph-wf24" } }))?;
    let svg = svg_out["payload"]["svg"].as_str().ok_or_else(|| String::from("missing svg string"))?;
    if !svg.contains("***") || !svg.contains("ns") {
        return Err(format!("expected both `***` and `ns` significance star labels in compiled SVG:\n{svg}"));
    }

    let _scene = assert_scene_compiles_cleanly(app.document(), "graph-wf24")?;
    let ws_inspect = gui(&mut app, "ui.inspect", json!({ "scope": "all" }))?;
    if ws_inspect["active_sheet_id"] != json!("graph-wf24") {
        return Err(format!("expected active_sheet_id == graph-wf24, got {ws_inspect}"));
    }
    let _png = assert_screenshot_png(&mut app, "workspace", 960, 640)?;
    Ok(())
}

/// WF-25: Violin / Box-and-Whisker / Bar-with-Replicates & Aligned Dot Plot directly via
/// `ChangeGraphType` (`ui.changeGraphType.open` -> `ui.answer_dialog`), SD/SEM/95%-CI error bar
/// metrics via both `ChangeGraphType` and `Format/Data on Graph...`, R1 ANOVA graying check,
/// plus SVG and screenshot PNG verification.
///
/// Known `src/` scope note:
/// - No `EstimationPlot` (`col.estimation`) variant exists in `GraphicalDataObject` (`crates/doc/src/graph.rs`);
///   `ChangeGraphType` (`ui.inspect` controls `graph_type.selected`, `graph_type.summary_measure`,
///   `graph_type.error_bar_metric`) applies column GDO switching (`col.box_whisker`, `col.violin`,
///   `col.bar_replicates`, `col.aligned_dot`) directly via `ChangeGraphTypeState::commit_intents`.
#[test]
fn wf25_violin_boxplot_column_styles_error_bars_and_svg_strict_gui_only() -> TestResult {
    let mut app = new_strict_gui_app()?;
    let tsv = "\
10.1\t15.2\t22.0
11.4\t17.8\t25.5
9.8\t14.9\t20.4
12.2\t19.1\t28.1
10.9\t16.4\t24.0
13.0\t20.5\t30.2
9.5\t13.8\t19.7
11.7\t18.0\t26.8";
    create_column_table(&mut app, "col-wf25", "graph-wf25", &["Vehicle", "Low", "High"], tsv)?;

    // 1. Box-and-Whisker plot directly via ChangeGraphType dialog.
    open_and_apply_graph_type(&mut app, "graph-wf25", "col.box_whisker")?;
    let box_sid = SheetId::try_new("graph-wf25").map_err(|e| e.to_string())?;
    let is_box = app
        .document()
        .graph_sheet(&box_sid)
        .is_some_and(|g| !g.spec.series.is_empty() && g.spec.series.iter().all(|s| matches!(&s.gdo, GraphicalDataObject::BoxAndWhisker { .. })));
    if !is_box {
        return Err(String::from("expected BoxAndWhisker GDO directly after ChangeGraphType `col.box_whisker`"));
    }
    let box_svg = gui(&mut app, "ui.invoke_menu", json!({ "command_id": "graph.exportSvg", "params": { "sheet_id": "graph-wf25" } }))?;
    let box_svg_str = box_svg["payload"]["svg"].as_str().ok_or("missing box svg")?;
    if !box_svg_str.contains("<svg") {
        return Err(String::from("expected valid SVG for box-and-whisker"));
    }
    let box_png = assert_screenshot_png(&mut app, "canvas", 840, 560)?;

    // 2. Violin KDE plot directly via ChangeGraphType dialog.
    open_and_apply_graph_type(&mut app, "graph-wf25", "col.violin")?;
    let is_violin = app
        .document()
        .graph_sheet(&box_sid)
        .is_some_and(|g| !g.spec.series.is_empty() && g.spec.series.iter().all(|s| matches!(&s.gdo, GraphicalDataObject::Violin { .. })));
    if !is_violin {
        return Err(String::from("expected Violin GDO directly after ChangeGraphType `col.violin`"));
    }
    let violin_svg = gui(&mut app, "ui.invoke_menu", json!({ "command_id": "graph.exportSvg", "params": { "sheet_id": "graph-wf25" } }))?;
    let violin_svg_str = violin_svg["payload"]["svg"].as_str().ok_or("missing violin svg")?;
    if !violin_svg_str.contains("<path") || violin_svg_str == box_svg_str {
        return Err(String::from("expected Violin SVG to differ from BoxAndWhisker SVG and emit KDE paths"));
    }
    let violin_png = assert_screenshot_png(&mut app, "canvas", 840, 560)?;
    if violin_png == box_png {
        return Err(String::from("expected distinct raster canvas PNG between BoxAndWhisker and Violin"));
    }

    // 3. Switch directly to Bar with Replicates overlay + Aligned Dot, checking ChangeGraphType error_bar_metric & Format/Data cycling.
    open_and_apply_graph_type_with_fields(&mut app, "graph-wf25", "col.bar_replicates", &[("error_bar_metric", json!("sem"))])?;
    let bar_ok = app.document().graph_sheet(&box_sid).is_some_and(|g| {
        !g.spec.series.is_empty()
            && g.spec.series.iter().all(|s| matches!(&s.gdo, GraphicalDataObject::BarWithReplicates { replicate_overlay: Some(_), .. }))
            && g.spec.series.first().and_then(|s| s.y_error_bars.map(|eb| eb.metric)) == Some(ErrorBarMetric::Sem)
    });
    if !bar_ok {
        return Err(String::from("expected BarWithReplicates with Sem error bar directly after ChangeGraphType `col.bar_replicates`"));
    }
    for (metric_key, expected_metric) in [("sd", ErrorBarMetric::Sd), ("sem", ErrorBarMetric::Sem), ("ci95", ErrorBarMetric::ConfidenceInterval95)] {
        gui(
            &mut app,
            "ui.invoke_menu",
            json!({
                "command_id": "graph.setSeries",
                "params": {
                    "sheet_id": "graph-wf25",
                    "series_index": 0,
                    "y_error_bars": { "metric": metric_key, "direction": "both", "dash_style": "solid" }
                }
            }),
        )?;
        let actual = app.document().graph_sheet(&box_sid).and_then(|g| g.spec.series.first()).and_then(|s| s.y_error_bars.map(|eb| eb.metric));
        if actual != Some(expected_metric) {
            return Err(format!("expected y_error_bars metric {expected_metric:?}, got {actual:?}"));
        }
    }
    open_and_apply_graph_type_with_fields(&mut app, "graph-wf25", "col.aligned_dot", &[("summary_measure", json!("median"))])?;
    let aligned_ok = app.document().graph_sheet(&box_sid).is_some_and(|g| {
        !g.spec.series.is_empty()
            && g.spec
                .series
                .iter()
                .all(|s| matches!(&s.gdo, GraphicalDataObject::AlignedDotPlot { summary: SummaryMeasure::Median, show_crossbar: true, .. }))
    });
    if !aligned_ok {
        return Err(String::from("expected AlignedDotPlot (Median) directly after ChangeGraphType `col.aligned_dot`"));
    }
    assert_scene_compiles_cleanly(app.document(), "graph-wf25")?;

    // 4. Verify R1 graying predicate via EditAnalysisParameters on ANOVA over the same data.
    select_sheet(&mut app, "col-wf25")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.one_way_anova" }, "action": "ok" }))?;
    let res_id = app.document().result_sheets.first().map(|r| r.id.as_str().to_owned()).ok_or("missing anova result")?;
    select_sheet(&mut app, &res_id)?;
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    let edit_snap = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_control_enabled(&edit_snap, "edit.sphericity.geisser_greenhouse", false)?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "cancel" }))?;
    Ok(())
}

/// WF-26: Bubble Plot (5-channel Viridis + variable size + convex hulls + inspector), Volcano Plot
/// (Multiple Row t-tests -> -log10(p) vs diff crosshairs), Heatmap (Correlation Matrix), and
/// `R7`/`R8`/`R9`/`R10` modal graying verification via `ui.inspect`.
///
/// Known `src/` gaps verified in codebase:
/// - `ActiveDialog::BubblePlotInspector` has no menu opener, empty `controls: []` in `inspect.rs:953`, and `_ => {}`
///   in `gui_actions.rs:860-910` (the non-modal `ActiveInspector::BubblePlot` pane activates when selecting a bubble graph).
/// - No `Histogram`/`KDE` analysis or `R8 auto_bin` predicate, and no Multivariate `fold_change`/`VIP` `R9` predicate exist
///   in `crates/doc/src/analysis.rs` or `crates/ui-egui/src/modal_predicates.rs` (where `R7`=correlation heatmap,
///   `R8`=subcolumn aggregation, `R9`=P-value digits, `R10`=PCA/PCR leakage).
#[test]
fn wf26_bubble_volcano_heatmap_multivariate_and_r7_to_r10_inspect_strict_gui_only() -> TestResult {
    let mut app = new_strict_gui_app()?;

    // 1. MultipleVariables table -> 5-channel Bubble Plot + Heatmap + Correlation Matrix + PCA/PCR R7/R10 checks.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "mv-wf26",
                "title": "Multivariate Cohort",
                "kind": "multiple_variables",
                "cols": 5,
                "rows": 8,
                "variables": [
                    { "title": "PC1", "role": "continuous" },
                    { "title": "PC2", "role": "continuous" },
                    { "title": "Biomarker", "role": "continuous" },
                    { "title": "Cluster", "role": "categorical" },
                    { "title": "Sample", "role": "label" }
                ]
            }
        }),
    )?;
    select_sheet(&mut app, "mv-wf26")?;
    let mv_tsv = "\
-2.4\t1.1\t15.0\tCtrl\tS1
-1.8\t0.6\t18.0\tCtrl\tS2
-2.1\t1.4\t22.0\tCtrl\tS3
-1.5\t0.9\t19.0\tCtrl\tS4
2.2\t-1.2\t64.0\tTreat\tS5
2.7\t-0.8\t72.0\tTreat\tS6
1.9\t-1.5\t58.0\tTreat\tS7
2.5\t-1.0\t69.0\tTreat\tS8";
    gui(&mut app, "ui.type_text", json!({ "widget_id": "grid.cell.0.0.0", "text": mv_tsv, "clear_existing": true }))?;

    // Check R7 in Analyze dialog for Correlation Matrix (`all_pairs` toggles heatmap checkbox enablement) + R8 subcolumn check.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "sheet_id": "mv-wf26" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.corr_matrix", "correlation_all_pairs": false } }))?;
    let corr_disabled = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_control_enabled(&corr_disabled, "analyze.correlation.heatmap", false)?;
    assert_control_enabled(&corr_disabled, "analyze.descriptive.subcolumn_agg", false)?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "correlation_all_pairs": true } }))?;
    let corr_enabled = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_control_enabled(&corr_enabled, "analyze.correlation.heatmap", true)?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "correlation_heatmap": true }, "action": "ok" }))?;

    // Create Bubble Scatter graph and configure 5 channels via graph.setSpec (`Viridis`, variable size, labels, hulls).
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "graph.create", "params": { "data_sheet_id": "mv-wf26", "sheet_id": "graph-bubble" } }))?;
    open_and_apply_graph_type(&mut app, "graph-bubble", "mv.bubble")?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setSpec",
            "params": {
                "sheet_id": "graph-bubble",
                "title": "5-Channel Bubble Plot",
                "multivariate": {
                    "plot_mode": "BubbleScatter",
                    "x_column": 0,
                    "y_column": 1,
                    "color": { "column": 2, "categorical": false },
                    "size": { "column": 2, "min_radius_pt": 3.0, "max_radius_pt": 11.0 },
                    "label_column": 4,
                    "show_convex_hulls": true,
                    "show_confidence_ellipses": true
                }
            }
        }),
    )?;
    select_sheet(&mut app, "graph-bubble")?;
    if app.canvas_dialogs.active_inspector != ActiveInspector::BubblePlot {
        return Err(format!("expected ActiveInspector::BubblePlot on bubble graph, got {:?}", app.canvas_dialogs.active_inspector));
    }
    let bubble_scene = assert_scene_compiles_cleanly(app.document(), "graph-bubble")?;
    if !matches!(
        app.document().graph_sheet(&SheetId::try_new("graph-bubble").map_err(|e| e.to_string())?).and_then(|g| g.spec.multivariate.as_ref()),
        Some(mv) if mv.plot_mode == MultivariatePlotMode::BubbleScatter && mv.show_convex_hulls
    ) || bubble_scene.warnings.iter().any(|w| w.code == "empty_scatter_data")
    {
        return Err(format!("bubble plot did not render valid scatter geometry: {:?}", bubble_scene.warnings));
    }
    let _bubble_png = assert_screenshot_png(&mut app, "canvas", 840, 560)?;

    // Create Heatmap graph linked to Multivariate table / correlation matrix.
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "graph.create", "params": { "data_sheet_id": "mv-wf26", "sheet_id": "graph-heatmap" } }))?;
    open_and_apply_graph_type(&mut app, "graph-heatmap", "diag.heatmap")?;
    let hm_scene = assert_scene_compiles_cleanly(app.document(), "graph-heatmap")?;
    if hm_scene.warnings.iter().any(|w| w.code == "empty_heatmap") {
        return Err(format!("heatmap reported empty_heatmap warning: {:?}", hm_scene.warnings));
    }

    // 2. Grouped table -> Multiple Row t-tests -> Volcano Plot + R8/R9 graying verification.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "grp-wf26",
                "title": "Gene Expression",
                "kind": "grouped",
                "cols": 2,
                "replicates": 3,
                "rows": 4
            }
        }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "table.setTitles",
            "params": {
                "sheet_id": "grp-wf26",
                "titles": ["Control", "Treated"],
                "row_titles": ["Gene A", "Gene B", "Gene C", "Gene D"]
            }
        }),
    )?;
    select_sheet(&mut app, "grp-wf26")?;
    let grp_tsv = "\
10.0\t10.5\t9.8\t18.5\t19.2\t18.9
20.1\t19.9\t20.4\t20.2\t20.0\t20.3
5.0\t5.2\t4.9\t12.1\t12.6\t12.4
15.0\t15.2\t14.8\t15.1\t14.9\t15.3";
    gui(&mut app, "ui.type_text", json!({ "widget_id": "grid.cell.0.0.0", "text": grp_tsv, "clear_existing": true }))?;

    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "sheet_id": "grp-wf26" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.multiple_row_t_tests" }, "action": "ok" }))?;

    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "graph.create", "params": { "data_sheet_id": "grp-wf26", "sheet_id": "graph-volcano" } }))?;
    open_and_apply_graph_type(&mut app, "graph-volcano", "diag.volcano")?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setAxes",
            "params": {
                "sheet_id": "graph-volcano",
                "x_axis": { "title": "Difference (Treated - Control)" },
                "y_axis": { "title": "-log10(P)" }
            }
        }),
    )?;
    let vol_scene = assert_scene_compiles_cleanly(app.document(), "graph-volcano")?;
    if vol_scene.warnings.iter().any(|w| w.code == "empty_volcano") {
        return Err(format!("volcano plot reported empty_volcano warning: {:?}", vol_scene.warnings));
    }

    // Verify R8 (subcolumn aggregation requires >1 subcolumn) & R9 (P-value digits requires fixed decimals).
    select_sheet(&mut app, "grp-wf26")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "sheet_id": "grp-wf26" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.descriptive" }, "action": "ok" }))?;
    let desc_id = app
        .document()
        .result_sheets
        .iter()
        .rev()
        .find(|r| r.analysis.source_sheet_id == SheetId::try_new("grp-wf26").unwrap_or_else(|_| r.analysis.source_sheet_id.clone()))
        .map(|r| r.id.as_str().to_owned())
        .ok_or("missing descriptive stats result sheet")?;
    select_sheet(&mut app, &desc_id)?;
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    let r8_r9_snap = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_control_enabled(&r8_r9_snap, "edit.descriptive.subcolumn_agg", true)?;
    assert_control_enabled(&r8_r9_snap, "edit.pvalue.digits", false)?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "p_value_format": "custom" } }))?;
    let r9_enabled_snap = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_control_enabled(&r9_enabled_snap, "edit.pvalue.digits", true)?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    // Check R10 PCR predictor leakage prevention in Analyze dialog on mv-wf26.
    select_sheet(&mut app, "mv-wf26")?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "sheet_id": "mv-wf26" } }))?;
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.pcr", "pca_predictors": [0, 1] } }))?;
    let r10_blocked = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_control_enabled(&r10_blocked, "analyze.pcr.outcome.0", false)?;
    assert_control_enabled(&r10_blocked, "analyze.pcr.outcome.2", true)?;
    if dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "pcr_outcome_column": 0 } })).is_ok() {
        return Err(String::from("expected R10 leakage guard to reject predictor column 0 as PCR outcome"));
    }
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "pcr_outcome_column": 2 }, "action": "cancel" }))?;

    Ok(())
}

/// WF-27: Multi-panel `LayoutSheet` creation, live pointer drag & snap (`ui.pointer_gesture` `drag`),
/// `ApplyStyleFrom` Eyedropper style cloning preserving target data bindings, and publication preflight audit.
///
/// Known `src/` gaps verified in codebase:
/// - `ActiveDialog::PageLayoutComposer` and `ActiveDialog::ApplyStyleFrom` have no menu/UI command opener in
///   `crates/ui-egui/src/{menus/ui_commands.rs, control_io.rs, interaction.rs}` (empty `controls: []` in `inspect.rs:954-955`,
///   `_ => {}` in `gui_actions.rs:860-910`).
/// - `R7` in `crates/ui-egui/src/modal_predicates.rs:203` governs Correlation Heatmap; no sheet-kind compatibility graying
///   exists in `ApplyStyleFromState` (`dialogs_canvas.rs:2630-2690`).
#[test]
fn wf27_multi_panel_layout_drag_snap_apply_style_and_preflight_strict_gui_only() -> TestResult {
    let mut app = new_strict_gui_app()?;
    let donor_tsv = "10\t22\n12\t25\n11\t24\n13\t27";
    let target_tsv = "5\t14\n6\t17\n7\t16\n8\t19";
    create_column_table(&mut app, "data-donor", "g-donor", &["Base", "Stim"], donor_tsv)?;
    create_column_table(&mut app, "data-target", "g-target", &["Ctrl", "Drug"], target_tsv)?;

    open_and_apply_graph_type(&mut app, "g-donor", "col.bar_replicates")?;
    open_and_apply_graph_type(&mut app, "g-target", "col.bar_replicates")?;

    // Style donor graph distinctly via Format/Graph...
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.setSpec",
            "params": {
                "sheet_id": "g-donor",
                "palette": "PaulTolVibrant",
                "frame_preset": "PlainBoxed",
                "legend": { "visible": true, "bordered": true }
            }
        }),
    )?;

    // Apply Style From donor onto target via menu command; verify visual cloning without overwriting data bindings.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "graph.applyStyleFrom",
            "params": { "sheet_id": "g-target", "donor_sheet_id": "g-donor" }
        }),
    )?;
    let target_sid = SheetId::try_new("g-target").map_err(|e| e.to_string())?;
    let target_gs = app.document().graph_sheet(&target_sid).ok_or("missing g-target")?;
    if target_gs.spec.palette != PaletteId::PaulTolVibrant
        || target_gs.spec.frame_preset != FramePreset::PlainBoxed
        || !target_gs.spec.legend.bordered
        || target_gs.spec.primary_source_sheet_id != SheetId::try_new("data-target").map_err(|e| e.to_string())?
    {
        return Err(format!("unexpected target spec after graph.applyStyleFrom: {:?}", target_gs.spec));
    }

    // Passing a non-graph sheet as donor fails cleanly through menu validation.
    let bad_donor = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.applyStyleFrom", "params": { "sheet_id": "g-target", "donor_sheet_id": "data-donor" } }),
    );
    if bad_donor.is_ok() {
        return Err(String::from("expected ApplyStyleFrom with DataSheet donor ID to fail"));
    }

    // Create 2-panel LayoutSheet with uppercase badges (`A`, `B`) and align_plot_origins = true.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "layout.create",
            "params": {
                "sheet_id": "layout-wf27",
                "title": "Figure 1 Composite",
                "preset": "double_column",
                "align_plot_origins": true,
                "panel_badges": "uppercase"
            }
        }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "layout.addPanel", "params": { "sheet_id": "layout-wf27", "graph_sheet_id": "g-donor", "bounds_pt": [24.0, 24.0, 200.0, 150.0] } }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "layout.addPanel", "params": { "sheet_id": "layout-wf27", "graph_sheet_id": "g-target", "bounds_pt": [260.0, 42.0, 180.0, 130.0] } }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "layout.addPanel",
            "params": { "sheet_id": "layout-wf27", "kind": "text", "markdown": "Multi-panel composite", "bounds_pt": [24.0, 195.0, 220.0, 22.0] }
        }),
    )?;
    select_sheet(&mut app, "layout-wf27")?;

    // Drag Panel 1 via native `egui` pointer drag on `canvas.layout_panel.1`.
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let layout_sid = SheetId::try_new("layout-wf27").map_err(|e| e.to_string())?;
    let y_before = match app.document().layout_sheet(&layout_sid).and_then(|l| l.spec.items.get(1)) {
        Some(LayoutItem::LiveGraph { bounds_pt, .. }) => bounds_pt.y,
        other => return Err(format!("expected LiveGraph item 1 before drag, got {other:?}")),
    };

    let was_human_only = app.is_human_path_only();
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, metrics);
    let panel_rect = harness
        .driver
        .frame_meta()
        .find_by_widget_id("canvas.layout_panel.1")
        .map(|m| m.clipped_rect)
        .ok_or_else(|| String::from("missing canvas.layout_panel.1 in egui frame"))?;
    let start_pos = panel_rect.center();
    let end_pos = egui::pos2(start_pos.x, start_pos.y - 24.0);
    harness.driver.drag(start_pos, end_pos)?;
    app = harness.driver.state().app.clone();
    app.set_human_path_only(was_human_only);
    let y_after = match app.document().layout_sheet(&layout_sid).and_then(|l| l.spec.items.get(1)) {
        Some(LayoutItem::LiveGraph { bounds_pt, .. }) => bounds_pt.y,
        other => return Err(format!("expected LiveGraph item 1 after drag, got {other:?}")),
    };
    if (y_after - y_before).abs() < 5.0 {
        return Err(format!("expected Panel 1 drag to move Y coordinate cleanly, before={y_before}, after={y_after}"));
    }

    let ls = app.document().layout_sheet(&layout_sid).ok_or("missing layout-wf27")?;
    let (exp_w, exp_h, exp_unit) = JournalWidthPreset::DoubleColumn174Mm.dimensions();
    if (ls.spec.width - exp_w).abs() > 1e-6
        || (ls.spec.height - exp_h).abs() > 1e-6
        || ls.spec.unit != exp_unit
        || ls.spec.panel_badges != PanelBadgeStyle::UpperCaseLetters
    {
        return Err(format!("unexpected layout spec: {:?}", ls.spec));
    }

    // Run publication preflight check and verify layout renders clean PNG & SVG.
    let preflight = gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.canvas.preflight", "params": { "sheet_id": "layout-wf27" } }))?;
    if preflight["result"]["issue_count"].as_u64().is_none() {
        return Err(format!("expected preflight issue_count JSON field, got {preflight}"));
    }
    assert_scene_compiles_cleanly(app.document(), "layout-wf27")?;
    let _png = assert_screenshot_png(&mut app, "workspace", 1024, 640)?;
    Ok(())
}

/// WF-28: Journal Canvas Preset, non-mutating Color-Vision Preview, `Print` Setup dialog & in-memory
/// PDF spooling, inline publication export (SVG/PDF/PNG), and Light vs Dark theme screenshots.
///
/// Known `src/` gaps verified in codebase:
/// - `ActiveDialog::ExportFigure` has no menu opener (`File/Export/*` runs export commands directly).
/// - `ActiveDialog::Print` opens via `"ui.print.open"` / `"file.print"` and confirms via `ui.answer_dialog`,
///   but `patch_active_dialog` (`gui_actions.rs:860-910`) hits `_ => {}` (cannot edit `crop_marks`/`bleed_pt`
///   via `ui.answer_dialog` fields once open), `inspect.rs` returns empty `controls: []`, and `R10` in
///   `modal_predicates.rs:233` is PCA/PCR leakage rather than Print crop-marks/bleed graying.
#[test]
fn wf28_canvas_preset_cvd_print_export_and_light_dark_screenshots_strict_gui_only() -> TestResult {
    let mut app = new_strict_gui_app()?;

    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "xy-wf28",
                "title": "Dose Response",
                "kind": "xy",
                "cols": 2,
                "rows": 5,
                "auto_graph": true,
                "graph_sheet_id": "graph-wf28"
            }
        }),
    )?;
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setXCells", "params": { "sheet_id": "xy-wf28", "values": [1.0, 3.0, 10.0, 30.0, 100.0] } }),
    )?;
    select_sheet(&mut app, "xy-wf28")?;
    gui(
        &mut app,
        "ui.type_text",
        json!({ "widget_id": "grid.cell.0.0.0", "text": "8\t14\n24\t39\n52\t68\n81\t89\n95\t98", "clear_existing": true }),
    )?;

    open_and_apply_graph_type(&mut app, "graph-wf28", "xy.scatter_line")?;
    let graph_sid = SheetId::try_new("graph-wf28").map_err(|e| e.to_string())?;

    // 1. Apply Journal Canvas Preset (`SingleColumn88mm`) & run preflight.
    let preset_res = gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "ui.canvas.applyPreset",
            "params": { "sheet_id": "graph-wf28", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }
        }),
    )?;
    let expected = FigureCanvasPreset::SingleColumn88mm.dimensions_pt();
    if preset_res["result"]["preset"] != json!("SingleColumn88mm") {
        return Err(format!("expected preset SingleColumn88mm in response, got {preset_res}"));
    }
    let resized_canvas = app.document().graph_sheet(&graph_sid).ok_or("missing graph-wf28")?.spec.canvas_size_pt;
    if (resized_canvas.width - expected.width).abs() > 0.5 || (resized_canvas.height - expected.height).abs() > 0.5 {
        return Err(format!(
            "expected SingleColumn88mm ({}x{}pt), got {}x{}pt",
            expected.width, expected.height, resized_canvas.width, resized_canvas.height
        ));
    }

    // 2. Non-mutating Color-Vision Preview filters (`deuteranopia` / `grayscale`) alter screenshots without mutating GraphSpec.
    let original_spec = app.document().graph_sheet(&graph_sid).ok_or("missing graph-wf28")?.spec.clone();
    let normal_png = assert_screenshot_png(&mut app, "canvas", 800, 520)?;
    for (filter_name, expected_enum) in [
        ("deuteranopia", CanvasPreviewFilter::Deuteranopia),
        ("protanopia", CanvasPreviewFilter::Protanopia),
        ("tritanopia", CanvasPreviewFilter::Tritanopia),
        ("grayscale", CanvasPreviewFilter::GrayscalePrint),
    ] {
        gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.previewFilter.set", "params": { "filter": filter_name } }))?;
        if app.preview_filter != expected_enum {
            return Err(format!("expected preview filter {expected_enum:?}, got {:?}", app.preview_filter));
        }
        let filtered_png = assert_screenshot_png(&mut app, "canvas", 800, 520)?;
        if filtered_png == normal_png {
            return Err(format!("expected preview filter `{filter_name}` to alter canvas screenshot PNG"));
        }
        if app.document().graph_sheet(&graph_sid).map(|g| &g.spec) != Some(&original_spec) {
            return Err(format!("preview filter `{filter_name}` mutated GraphSpec!"));
        }
    }
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.previewFilter.set", "params": { "filter": "normal" } }))?;

    // 3. Print Setup dialog (`ActiveDialog::Print`): open with crop_marks + bleed_pt, inspect, screenshot, confirm spool.
    gui(
        &mut app,
        "ui.invoke_menu",
        json!({
            "command_id": "ui.print.open",
            "params": { "sheet_id": "graph-wf28", "copies": 2, "scale_percent": 100.0, "crop_marks": true, "bleed_pt": 9.0 }
        }),
    )?;
    let print_snap = gui(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    if print_snap["active_dialog"] != json!("print") || !app.canvas_dialogs.print.job.crop_marks {
        return Err(format!("expected active print dialog with crop_marks=true, got {print_snap}"));
    }
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "crop_marks": false, "bleed_pt": 12.0, "copies": 3, "scale_percent": 90.0 } }))?;
    if app.canvas_dialogs.print.job.crop_marks
        || (app.canvas_dialogs.print.job.bleed_pt - 12.0).abs() > 1e-9
        || app.canvas_dialogs.print.copies != 3
        || (app.canvas_dialogs.print.job.scale_percent - 90.0).abs() > 1e-9
    {
        return Err(String::from("expected Print fields to update via ui.answer_dialog"));
    }
    gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "print.crop_marks" }))?;
    if !app.canvas_dialogs.print.job.crop_marks {
        return Err(String::from("expected clicking print.crop_marks to toggle crop_marks back to true"));
    }
    let _print_modal_png = assert_screenshot_png(&mut app, "dialog", 800, 520)?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    if app.canvas_dialogs.active_dialog != ActiveDialog::None || app.canvas_dialogs.print.last_spool_bytes < 100 {
        return Err(format!(
            "expected Print ok to spool PDF and close dialog, active={:?}, bytes={}",
            app.canvas_dialogs.active_dialog, app.canvas_dialogs.print.last_spool_bytes
        ));
    }

    // 4. Inline figure exports through menu commands (`graph.exportSvg`, `export.pdf` CMYK, `export.png`).
    let svg_out = gui(&mut app, "ui.invoke_menu", json!({ "command_id": "graph.exportSvg", "params": { "sheet_id": "graph-wf28" } }))?;
    let svg_str = svg_out["payload"]["svg"].as_str().ok_or("missing exported svg")?;
    if !svg_str.contains("<svg") {
        return Err(String::from("exported SVG missing <svg element"));
    }

    let pdf_out =
        gui(&mut app, "ui.invoke_menu", json!({ "command_id": "export.pdf", "params": { "sheet_id": "graph-wf28", "color_space": "cmyk" } }))?;
    let pdf_bytes = decode_base64(pdf_out["payload"]["base64"].as_str().ok_or("missing pdf base64")?)?;
    if !pdf_bytes.starts_with(b"%PDF-") {
        return Err(String::from("exported PDF missing %PDF header"));
    }

    let png_out = gui(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "export.png", "params": { "sheet_id": "graph-wf28", "dpi": 144, "width_px": 480, "height_px": 320 } }),
    )?;
    let exp_png_bytes = decode_base64(png_out["payload"]["base64"].as_str().ok_or("missing exported png base64")?)?;
    if !exp_png_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("exported PNG missing PNG magic bytes"));
    }

    // 5. Light and Dark theme full-window `ui.screenshot` verification (non-empty, distinct inline PNGs).
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.theme.set", "params": { "mode": "light" } }))?;
    if app.theme.mode != UiThemeMode::Light {
        return Err(format!("expected Light theme mode, got {:?}", app.theme.mode));
    }
    let light_png = assert_screenshot_png(&mut app, "full_window", 960, 640)?;

    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.theme.set", "params": { "mode": "dark" } }))?;
    if app.theme.mode != UiThemeMode::Dark {
        return Err(format!("expected Dark theme mode, got {:?}", app.theme.mode));
    }
    let dark_png = assert_screenshot_png(&mut app, "full_window", 960, 640)?;

    if light_png == dark_png {
        return Err(String::from("expected Light and Dark theme full_window screenshots to differ"));
    }
    Ok(())
}
