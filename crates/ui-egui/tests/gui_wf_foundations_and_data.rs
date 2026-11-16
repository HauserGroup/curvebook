#![forbid(unsafe_code)]
//! Wave 17 `gui_only` acceptance suite covering `WF-01` through `WF-06` (`docs/gui-workflows.md`).
//!
//! Every workflow runs strictly under `UiApp` with `app.set_gui_only(true)` (`GuiOnlyPolicy::Strict`)
//! and drives all mutations, selections, brushing, dialogs, menu commands, shortcuts, tab
//! switches, and screenshots exclusively through `lumencraft_ui_egui::dispatch_gui_action`.

use lumencraft_doc::{
    AnalysisKind, CellCoord, MathTransformPreset, MultivariateSpec, ResultCell, ResultStatus, SheetId, TableArchetype, TableKind,
    TransformAnalysisSpec,
};
use lumencraft_ui_egui::{UiApp, dispatch_gui_action, handle_control_line};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn gui(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed [{}]: {}", err.code.as_str(), err.message()))
}

fn invoke_menu(app: &mut UiApp, path: &str, params: Value) -> Result<Value, String> {
    gui(app, "ui.invoke_menu", json!({ "path": path, "params": params }))
}

fn click_widget(app: &mut UiApp, widget_id: &str) -> Result<Value, String> {
    gui(app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": widget_id }))
}

fn click_widget_mod(app: &mut UiApp, widget_id: &str, shift: bool, meta: bool) -> Result<Value, String> {
    gui(
        app,
        "ui.pointer_gesture",
        json!({
            "action": "click",
            "widget_id": widget_id,
            "modifiers": { "shift": shift, "meta": meta }
        }),
    )
}

fn type_in_widget(app: &mut UiApp, widget_id: &str, text: &str) -> Result<Value, String> {
    gui(app, "ui.type_text", json!({ "widget_id": widget_id, "text": text, "clear_existing": true, "commit_enter": true }))
}

fn press_shortcut(app: &mut UiApp, key: &str, meta: bool, shift: bool) -> Result<Value, String> {
    gui(app, "ui.press_key", json!({ "key": key, "modifiers": { "meta": meta, "shift": shift } }))
}

fn inspect(app: &mut UiApp, scope: &str, include_bounds: bool) -> Result<Value, String> {
    gui(app, "ui.inspect", json!({ "scope": scope, "include_bounds": include_bounds }))
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for &b in input.as_bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => u32::from(b - b'A'),
            b'a'..=b'z' => u32::from(b - b'a') + 26,
            b'0'..=b'9' => u32::from(b - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            other => return Err(format!("invalid base64 byte {other}")),
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

fn assert_screenshot_png(app: &mut UiApp, target: &str) -> Result<usize, String> {
    let shot = gui(app, "ui.screenshot", json!({ "target": target, "width_px": 960, "height_px": 640 }))?;
    let width = shot.get("width_px").and_then(Value::as_u64).unwrap_or(0);
    let height = shot.get("height_px").and_then(Value::as_u64).unwrap_or(0);
    if width == 0 || height == 0 {
        return Err(format!("expected non-zero screenshot dims for `{target}`, got {width}x{height}"));
    }
    let encoded = shot.get("data_base64").and_then(Value::as_str).ok_or_else(|| String::from("screenshot missing data_base64"))?;
    let png = decode_base64(encoded)?;
    let magic: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if !png.starts_with(&magic) || png.len() < 64 {
        return Err(format!("invalid or empty PNG screenshot payload (len={})", png.len()));
    }
    Ok(png.len())
}

fn control_error_code(app: &mut UiApp, line: &str) -> Result<String, String> {
    let raw = handle_control_line(app, line);
    let parsed: Value = serde_json::from_str(&raw).map_err(|err| format!("invalid JSON `{raw}`: {err}"))?;
    parsed
        .pointer("/error_detail/code")
        .or_else(|| parsed.pointer("/error/code"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("expected error_detail.code in `{raw}`"))
}

fn result_sheet_scalar(app: &UiApp, sheet_id: &str, name_fragment: &str) -> Result<f64, String> {
    let sid = SheetId::try_new(sheet_id).map_err(|e| e.to_string())?;
    let rs = app.document().result_sheet(&sid).ok_or_else(|| format!("missing result sheet `{sheet_id}`"))?;
    rs.scalars
        .iter()
        .find(|s| s.key.contains(name_fragment))
        .map(|s| s.value)
        .ok_or_else(|| format!("missing scalar `{name_fragment}` on `{sheet_id}`"))
}

fn find_first_widgets(snap: &Value, prefix: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for key in ["canvas_elements", "grid_cells", "navigator_sheets", "toolbar", "tabs"] {
        if let Some(arr) = snap.get(key).and_then(Value::as_array) {
            for w in arr {
                if let Some(id) = w.get("widget_id").and_then(Value::as_str)
                    && id.starts_with(prefix)
                {
                    hits.push(id.to_owned());
                }
            }
        }
    }
    hits
}

fn selected_rows_from_inspect(snap: &Value) -> Vec<usize> {
    snap.get("grid_selection")
        .and_then(|g| g.get("selected_rows"))
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_u64).map(|n| n as usize).collect())
        .unwrap_or_default()
}

fn brushed_count_from_inspect(snap: &Value) -> usize {
    snap.get("brushed_points").and_then(Value::as_array).map_or(0, Vec::len)
}

fn new_strict_gui_app() -> UiApp {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    app
}

fn assert_result_cell_close(cell: Option<&ResultCell>, expected: f64, tol: f64, label: &str) -> TestRes {
    let actual = match cell {
        Some(ResultCell::Number { value, .. }) => *value,
        other => return Err(format!("expected numeric cell for `{label}`, got {other:?}")),
    };
    if (actual - expected).abs() > tol {
        return Err(format!("`{label}` expected ~{expected}, got {actual}"));
    }
    Ok(())
}

#[test]
fn wf01_xy_dose_response_format_exclude_with_reason_and_screenshot() -> TestRes {
    let mut app = new_strict_gui_app();
    if !app.is_gui_only() {
        return Err(String::from("expected strict gui_only mode"));
    }

    // Verify direct RPC/control-line command bypasses are blocked under Strict gui_only.
    let bypass_calls = [
        r#"{"id":"b1","method":"sheet.createData","params":{"kind":"xy"}}"#,
        r#"{"id":"b2","method":"engine.execute","params":{"command_id":"sheet.createData","params":{"kind":"xy"}}}"#,
        r#"{"id":"b3","method":"table.setCell","params":{"sheet_id":"xy-dose","row":0,"col":0,"value":10.0}}"#,
        r#"{"id":"b4","method":"ui.screenshot","params":{"target":"full_window","path":"/tmp/forbidden.png"}}"#,
    ];
    for line in bypass_calls {
        let code = control_error_code(&mut app, line)?;
        if code != "gui_only_violation" {
            return Err(format!("expected gui_only_violation for `{line}`, got `{code}`"));
        }
    }

    // Create XY table with linked graph and info sheet through File/Insert menu.
    let create_out = invoke_menu(
        &mut app,
        "Insert/New Data Table",
        json!({
            "sheet_id": "xy-dose",
            "kind": "xy",
            "title": "Dose Response",
            "rows": 6,
            "cols": 1,
            "replicates": 2,
            "auto_graph": true,
            "auto_info": true,
            "x_title": "Concentration (nM)"
        }),
    )?;
    let graph_id_str = create_out
        .pointer("/payload/graph_sheet_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("expected payload.graph_sheet_id in {create_out}"))?
        .to_owned();

    // Set X concentrations via the Data menu command AND verify direct XY X-column cell typing (`grid.xcell.*`).
    invoke_menu(&mut app, "Data/Set X Values", json!({ "sheet_id": "xy-dose", "values": [1.0, 3.0, 10.0, 30.0, 99.0, 300.0] }))?;
    click_widget(&mut app, "grid.xcell.4.0")?;
    type_in_widget(&mut app, "grid.xcell.4.0", "105")?;
    let x_check_snap = inspect(&mut app, "grid", true)?;
    let x_4_text = x_check_snap
        .get("grid_cells")
        .and_then(Value::as_array)
        .and_then(|arr| arr.iter().find(|w| w.get("widget_id").and_then(Value::as_str) == Some("grid.xcell.4.0")))
        .and_then(|w| w.pointer("/value/text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if x_4_text != "105" {
        return Err(format!("expected grid.xcell.4.0 text == `105` after XY X-col typing, got `{x_4_text}`"));
    }
    click_widget(&mut app, "grid.xcell.4.0")?;
    type_in_widget(&mut app, "grid.xcell.4.0", "100")?;

    // Focus grid cell (0, 0, 0) via pointer gesture and paste concentration-response replicate block.
    click_widget(&mut app, "grid.cell.0.0.0")?;
    type_in_widget(&mut app, "grid.cell.0.0.0", "98.0\t101.0\n88.0\t91.0\n68.0\t72.0\n42.0\t46.0\n19.0\t21.0\n6.0\t8.0")?;
    // Update single cell via grid focus + ui.type_text.
    click_widget(&mut app, "grid.cell.4.0.1")?;
    type_in_widget(&mut app, "grid.cell.4.0.1", "21.5")?;

    let grid_snap = inspect(&mut app, "grid", true)?;
    let cell_4_1 = grid_snap
        .get("grid_cells")
        .and_then(Value::as_array)
        .and_then(|arr| arr.iter().find(|w| w.get("widget_id").and_then(Value::as_str) == Some("grid.cell.4.0.1")))
        .and_then(|w| w.pointer("/value/text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if cell_4_1 != "21.5" {
        return Err(format!("expected grid.cell.4.0.1 to show `21.5`, got `{cell_4_1}`"));
    }

    // Multi-row grid selection (Shift range + Meta/Cmd toggle) -> Format Selected Points...
    click_widget(&mut app, "grid.cell.1.0.0")?;
    click_widget_mod(&mut app, "grid.cell.3.0.0", true, false)?;
    click_widget_mod(&mut app, "grid.cell.5.0.0", false, true)?;
    let sel_snap = inspect(&mut app, "grid", false)?;
    let selected_rows = selected_rows_from_inspect(&sel_snap);
    if !selected_rows.contains(&1) || !selected_rows.contains(&3) || !selected_rows.contains(&5) {
        return Err(format!("expected selected_rows to contain 1,3,5, got {selected_rows:?}"));
    }

    invoke_menu(&mut app, "Format/Format Selected Points...", json!({}))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": { "color": "#DC2626", "symbol": "diamond", "size_pt": 9.0, "show_row_labels": true }
        }),
    )?;

    // Select outlier replicate cell (2, 0, 1) and exclude with auditable reason (ExcludeWithReason).
    click_widget(&mut app, "grid.cell.2.0.1")?;
    invoke_menu(&mut app, "Edit/Exclude with Reason...", json!({}))?;
    let dlg_snap = inspect(&mut app, "dialog", false)?;
    let dlg_title = dlg_snap.pointer("/dialog/title").and_then(Value::as_str).unwrap_or("");
    if !dlg_title.contains("Exclude") {
        return Err(format!("expected ExcludeWithReason dialog, got `{dlg_title}`"));
    }
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": { "reason_tag": "Outlier", "note": "Pipette bubble at 10 nM replicate B" }
        }),
    )?;

    // Verify exclusion + audit note + exclusion badge flag on DataSheet, grid window, and ui.inspect.
    let xy_id = SheetId::try_new("xy-dose").map_err(|e| e.to_string())?;
    let ds = app.document().data_sheet(&xy_id).ok_or_else(|| String::from("missing xy-dose"))?;
    let outlier_coord = CellCoord::new(2, 0, 1);
    let excluded_cell = ds.cell(outlier_coord).ok_or_else(|| String::from("missing outlier cell"))?;
    if !excluded_cell.excluded {
        return Err(String::from("expected cell (2,0,1) to be excluded"));
    }
    let note = ds.exclusion_note(outlier_coord).ok_or_else(|| String::from("missing audit exclusion note"))?;
    if note.reason_tag != "Outlier" || !note.note.contains("Pipette bubble") {
        return Err(format!("unexpected exclusion note: {note:?}"));
    }
    let grid_window = app.grid.tool.format_visible_window(ds);
    let badge_cell = grid_window
        .cells
        .iter()
        .find(|c| c.coord.row == 2 && c.coord.col == 0 && c.coord.subcol == 1)
        .ok_or_else(|| String::from("missing visible cell for (2,0,1)"))?;
    if !badge_cell.excluded {
        return Err(String::from("expected excluded flag on visible cell (2,0,1)"));
    }

    // Switch to linked graph, verify overrides & scene update, and capture non-empty PNG screenshot.
    click_widget(&mut app, &format!("nav.sheet.{graph_id_str}"))?;
    let graph_id = SheetId::try_new(&graph_id_str).map_err(|e| e.to_string())?;
    let gs = app.document().graph_sheet(&graph_id).ok_or_else(|| format!("missing `{graph_id_str}`"))?;
    if gs.spec.point_overrides.is_empty() {
        return Err(String::from("expected Format Selected Points to add point_overrides"));
    }
    let diamond_present =
        gs.spec.point_overrides.iter().any(|ov| ov.symbol.is_some_and(|s| format!("{s:?}") == "Diamond") && ov.size_pt == Some(9.0));
    if !diamond_present {
        return Err(format!("expected diamond/size_pt override in {:?}", gs.spec.point_overrides));
    }
    let scene = app.session().render_sheet_scene(&graph_id).map_err(|e| e.to_string())?;
    if scene.plotted_points.is_empty() {
        return Err(String::from("expected linked graph scene to plot XY points"));
    }
    let _png_bytes = assert_screenshot_png(&mut app, "full_window")?;
    Ok(())
}

#[test]
fn wf02_normalize_to_result_sheet_chaining_and_log_x_transform() -> TestRes {
    let mut app = new_strict_gui_app();

    invoke_menu(
        &mut app,
        "Insert/New Data Table",
        json!({
            "sheet_id": "xy-conc",
            "kind": "xy",
            "title": "Raw Assay",
            "rows": 6,
            "cols": 1,
            "replicates": 1,
            "auto_graph": true
        }),
    )?;
    invoke_menu(&mut app, "Data/Set X Values", json!({ "sheet_id": "xy-conc", "values": [1e-9, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4] }))?;
    click_widget(&mut app, "grid.cell.0.0.0")?;
    type_in_widget(&mut app, "grid.cell.0.0.0", "10\n20\n50\n80\n90\n100")?;

    // Step 1: Normalize XY responses (0-100%) via Analyze/Analyze...
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let dlg1 = inspect(&mut app, "dialog", false)?;
    if dlg1.pointer("/dialog/dialog_kind").and_then(Value::as_str) != Some("analyze_sheet")
        || dlg1.pointer("/dialog/target_sheet_id").and_then(Value::as_str) != Some("xy-conc")
    {
        return Err(format!("expected analyze_sheet dialog targeting `xy-conc`, got {dlg1}"));
    }
    if !app.canvas_dialogs.analyze.compatible_entries().iter().any(|e| e.id == "transform.normalize") {
        return Err(String::from("expected `transform.normalize` in Analyze compatible_entries"));
    }
    let norm_out = gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "transform.normalize" } }))?;
    let norm_res_id =
        norm_out.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("normalize did not return sheet_id"))?.to_owned();

    // Verify ResultSheet is active and contains 0-100% normalized analyzable_data.
    if app.document().active_sheet_id.as_ref().map(SheetId::as_str) != Some(norm_res_id.as_str()) {
        return Err(format!("expected `{norm_res_id}` to become active after Analyze"));
    }
    let norm_sid = SheetId::try_new(&norm_res_id).map_err(|e| e.to_string())?;
    {
        let norm_rs = app.document().result_sheet(&norm_sid).ok_or_else(|| String::from("missing normalize result sheet"))?;
        assert_eq!(norm_rs.status, ResultStatus::UpToDate);
        let norm_data = norm_rs.analyzable_data.as_ref().ok_or_else(|| String::from("missing analyzable_data on normalize result"))?;
        let y0 = norm_data.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
        let y5 = norm_data.cell(CellCoord::new(5, 0, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
        if y0.abs() > 1e-9 || (y5 - 100.0).abs() > 1e-9 {
            return Err(format!("expected normalized endpoints 0 and 100, got {y0} and {y5}"));
        }
    }

    // Step 2: With ResultSheet active, open Analyze... directly from the ResultSheet (ResultSheet chaining!).
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let chain_dlg = inspect(&mut app, "dialog", false)?;
    let dlg_source = chain_dlg.pointer("/dialog/target_sheet_id").and_then(Value::as_str).unwrap_or("");
    if dlg_source != norm_res_id {
        return Err(format!("expected Analyze dialog from active ResultSheet to target `{norm_res_id}`, got `{dlg_source}`"));
    }
    if !app.canvas_dialogs.analyze.compatible_entries().iter().any(|e| e.id == "transform.math") {
        return Err(String::from("expected `transform.math` compatible when chaining from Normalize ResultSheet"));
    }
    // Drive transform.math (`x_transform = Log10, y_transform = None`) directly through the Analyze... modal fields!
    let log_out = gui(
        &mut app,
        "ui.answer_dialog",
        json!({ "action": "ok", "fields": { "entry_id": "transform.math", "x_transform": "log10", "y_transform": null } }),
    )?;
    let log_res_id = log_out
        .pointer("/payload/sheet_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("transform.math dialog did not return payload.sheet_id: {log_out}"))?
        .to_owned();

    // Verify chained DAG: xy-conc -> norm_res_id -> log_res_id and propagated values.
    let log_sid = SheetId::try_new(&log_res_id).map_err(|e| e.to_string())?;
    {
        let log_rs = app.document().result_sheet(&log_sid).ok_or_else(|| format!("missing `{log_res_id}`"))?;
        if log_rs.analysis.source_sheet_id.as_str() != norm_res_id {
            return Err(format!("expected `{log_res_id}` parent `{norm_res_id}`, got `{}`", log_rs.analysis.source_sheet_id));
        }
        if !matches!(
            &log_rs.analysis.kind,
            AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: Some(MathTransformPreset::Log10), y_transform: None, .. })
        ) {
            return Err(format!("expected Transform::Math(Log10, None), got {:?}", log_rs.analysis.kind));
        }
        let log_data = log_rs.analyzable_data.as_ref().ok_or_else(|| String::from("missing log-x analyzable_data"))?;
        let TableArchetype::Xy(xy_log) = &log_data.archetype else {
            return Err(String::from("expected XY archetype in log-x-res analyzable_data"));
        };
        let x0 = xy_log.x_value_at(0).unwrap_or(f64::NAN);
        let x5 = xy_log.x_value_at(5).unwrap_or(f64::NAN);
        let y4 = log_data.cell(CellCoord::new(4, 0, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
        if (x0 - (-9.0)).abs() > 1e-9 || (x5 - (-4.0)).abs() > 1e-9 {
            return Err(format!("expected Log10(X) in [-9.0, -4.0], got [{x0}, {x5}]"));
        }
        let expected_y4 = 80.0 / 90.0 * 100.0;
        if (y4 - expected_y4).abs() > 1e-6 {
            return Err(format!("expected normalized Y[4]={expected_y4}, got {y4}"));
        }
    }

    // Edit upstream cell via grid pointer + type_text and verify multi-hop reactive propagation through ResultSheet chain!
    click_widget(&mut app, "nav.sheet.xy-conc")?;
    click_widget(&mut app, "grid.cell.5.0.0")?;
    type_in_widget(&mut app, "grid.cell.5.0.0", "110")?;
    let log_rs_after = app.document().result_sheet(&log_sid).ok_or_else(|| String::from("missing log-x-res after edit"))?;
    let log_after_data = log_rs_after.analyzable_data.as_ref().ok_or_else(|| String::from("missing downstream analyzable_data"))?;
    let y4_after = log_after_data.cell(CellCoord::new(4, 0, 0)).and_then(|c| c.active_number()).unwrap_or(f64::NAN);
    if (y4_after - 80.0).abs() > 1e-6 {
        return Err(format!("expected downstream reactive recompute Y[4]=80.0, got {y4_after}"));
    }
    let _png_bytes = assert_screenshot_png(&mut app, "full_window")?;
    Ok(())
}

#[test]
fn wf03_column_brushing_format_and_undo_redo_shortcuts() -> TestRes {
    let mut app = new_strict_gui_app();

    let create_out = invoke_menu(
        &mut app,
        "Insert/New Data Table",
        json!({
            "sheet_id": "col-3grp",
            "kind": "column",
            "title": "Three Treatments",
            "rows": 5,
            "cols": 3,
            "auto_graph": true,
            "auto_info": true
        }),
    )?;
    let graph_id_str = create_out
        .pointer("/payload/graph_sheet_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("expected payload.graph_sheet_id in {create_out}"))?
        .to_owned();
    invoke_menu(&mut app, "Data/Set Column Titles", json!({ "sheet_id": "col-3grp", "titles": ["Vehicle", "LowDose", "HighDose"] }))?;

    // Enter column data across 3 groups via grid focus + type_text.
    click_widget(&mut app, "grid.cell.0.0.0")?;
    type_in_widget(&mut app, "grid.cell.0.0.0", "10.1\t14.2\t19.8\n10.8\t15.0\t20.4\n11.2\t14.7\t21.1\n10.5\t15.3\t19.5\n11.0\t14.9\t20.8")?;

    // Multi-row grid selection & brushing verification in ui.inspect.
    click_widget(&mut app, "grid.cell.0.0.0")?;
    click_widget_mod(&mut app, "grid.cell.2.0.0", true, false)?;
    click_widget_mod(&mut app, "grid.cell.4.1.0", false, true)?;
    let grid_snap = inspect(&mut app, "all", false)?;
    let rows = selected_rows_from_inspect(&grid_snap);
    if rows.len() < 3 || brushed_count_from_inspect(&grid_snap) < 2 {
        return Err(format!("expected multi-row selection + synced brushed_points, got rows={rows:?}, snap={grid_snap}"));
    }

    // Switch to linked graph, confirm first-visit graph chooser (`B1`/`ModalBackdrop`), and brush points from canvas -> grid.
    click_widget(&mut app, &format!("nav.sheet.{graph_id_str}"))?;
    if inspect(&mut app, "dialog", false)?.get("active_dialog").and_then(Value::as_str) == Some("change_graph_type") {
        gui(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    }
    let canvas_snap = inspect(&mut app, "canvas", true)?;
    let canvas_pts = find_first_widgets(&canvas_snap, "canvas.point.");
    if canvas_pts.len() < 2 {
        return Err(format!("expected clickable canvas.point.* widgets on Column graph, got {canvas_pts:?}"));
    }
    click_widget(&mut app, &canvas_pts[0])?;
    click_widget_mod(&mut app, &canvas_pts[1], true, false)?;
    let brushed_snap = inspect(&mut app, "canvas", false)?;
    if brushed_count_from_inspect(&brushed_snap) != 2 {
        return Err(format!("expected 2 brushed_points after graph Shift+Click, got {}", brushed_count_from_inspect(&brushed_snap)));
    }

    // Apply Format Selected Points override.
    invoke_menu(&mut app, "Format/Format Selected Points...", json!({}))?;
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "color": "#2563EB", "symbol": "square", "size_pt": 10.0 } }))?;
    let graph_sid = SheetId::try_new(&graph_id_str).map_err(|e| e.to_string())?;
    let overrides_after_apply = app.document().graph_sheet(&graph_sid).map_or(0, |g| g.spec.point_overrides.len());
    if overrides_after_apply != 2 {
        return Err(format!("expected 2 point_overrides after Format Selected Points, got {overrides_after_apply}"));
    }

    // Undo / Redo via keyboard shortcuts (Cmd+Z / Cmd+Shift+Z).
    press_shortcut(&mut app, "z", true, false)?;
    if !app.document().graph_sheet(&graph_sid).is_some_and(|g| g.spec.point_overrides.is_empty()) {
        return Err(String::from("expected Cmd+Z to undo point_overrides"));
    }
    press_shortcut(&mut app, "z", true, true)?;
    if app.document().graph_sheet(&graph_sid).map_or(0, |g| g.spec.point_overrides.len()) != 2 {
        return Err(String::from("expected Cmd+Shift+Z to redo point_overrides"));
    }

    // Undo / Redo via Edit menu items.
    invoke_menu(&mut app, "Edit/Undo", json!({}))?;
    if !app.document().graph_sheet(&graph_sid).is_some_and(|g| g.spec.point_overrides.is_empty()) {
        return Err(String::from("expected Edit/Undo menu to clear point_overrides"));
    }
    invoke_menu(&mut app, "Edit/Redo", json!({}))?;
    if app.document().graph_sheet(&graph_sid).map_or(0, |g| g.spec.point_overrides.len()) != 2 {
        return Err(String::from("expected Edit/Redo menu to restore point_overrides"));
    }

    let _png_bytes = assert_screenshot_png(&mut app, "full_window")?;
    Ok(())
}

#[test]
fn wf04_import_reshape_and_convert_archetype() -> TestRes {
    let mut app = new_strict_gui_app();

    // Verify ui.import.reshape.open opens ImportReshape dialog, rejects unconfigured confirm, then imports via modal fields.
    invoke_menu(&mut app, "File/Import Reshape...", json!({}))?;
    let reshape_dlg = inspect(&mut app, "dialog", false)?;
    let dlg_title = reshape_dlg.pointer("/dialog/title").and_then(Value::as_str).unwrap_or("");
    if !dlg_title.contains("Import Reshape") {
        return Err(format!("expected Import Reshape dialog title, got `{dlg_title}`"));
    }
    let unconfigured_err = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok" }));
    if unconfigured_err.is_ok() {
        return Err(String::from("unexpectedly succeeded confirming empty ImportReshape modal"));
    }
    let modal_tidy_out = gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "mode": "TidyLongPivot",
                "title": "Dialog Tidy Pivot",
                "target_kind": "Column",
                "tidy_csv": "Group,Value\nCtrl,10\nCtrl,12\nDrug,20\nDrug,22"
            }
        }),
    )?;
    let modal_tidy_id = modal_tidy_out
        .pointer("/payload/sheet_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("expected sheet_id from ImportReshape modal, got {modal_tidy_out}"))?;
    let modal_tidy_sid = SheetId::try_new(modal_tidy_id).map_err(|e| e.to_string())?;
    let modal_tidy_ds = app.document().data_sheet(&modal_tidy_sid).ok_or_else(|| String::from("missing Dialog Tidy Pivot"))?;
    if modal_tidy_ds.kind() != TableKind::Column || modal_tidy_ds.archetype.column_count() != 2 || modal_tidy_ds.archetype.row_count() != 2 {
        return Err(format!("expected 2x2 Column table from ImportReshape modal, got {:?}", modal_tidy_ds.archetype));
    }

    // Drive File/Import/Plate Map... (96-well plate) and File/Import/Tidy Long Table... through ui.invoke_menu.
    let mut roles_96 = Vec::with_capacity(96);
    let mut values_96 = Vec::with_capacity(96);
    let concs = [0.1_f64, 1.0, 10.0, 100.0];
    for row in 0..8usize {
        for col in 0..12usize {
            if row < 2 && col < 2 {
                roles_96.push(json!("Blank"));
                values_96.push(10.0);
            } else {
                let group_name = if row < 4 { "CompoundA" } else { "CompoundB" };
                let dilution_x = concs[col % concs.len()];
                let rep = (row % 4) + 1;
                roles_96.push(json!({ "group_name": group_name, "dilution_x": dilution_x, "replicate": rep }));
                values_96.push(10.0 + dilution_x * if group_name == "CompoundA" { 2.0 } else { 1.0 });
            }
        }
    }
    let plate_out = invoke_menu(
        &mut app,
        "File/Import/Plate Map...",
        json!({
            "sheet_id": "plate-96",
            "title": "Plate 96 Assay",
            "geometry": "Wells96",
            "values": values_96,
            "roles": roles_96,
            "x_title": "Dose",
            "target_kind": "Grouped",
            "subtract_blank_mean": true
        }),
    )?;
    if plate_out.pointer("/payload/sheet_id").and_then(Value::as_str) != Some("plate-96") {
        return Err(format!("expected plate-96 sheet output, got {plate_out}"));
    }

    let tidy_out = invoke_menu(
        &mut app,
        "File/Import/Tidy Long Table...",
        json!({
            "sheet_id": "tidy-col",
            "title": "Tidy Pivot Column",
            "csv": "Group,Value\nCtrl,11.0\nCtrl,12.5\nTreated,24.0\nTreated,26.0\n",
            "group_col": "Group",
            "value_col": "Value",
            "target_kind": "Column"
        }),
    )?;
    if tidy_out.pointer("/payload/sheet_id").and_then(Value::as_str) != Some("tidy-col") {
        return Err(format!("expected tidy-col sheet output, got {tidy_out}"));
    }
    let tidy_sid = SheetId::try_new("tidy-col").map_err(|e| e.to_string())?;
    let ds_before = app.document().data_sheet(&tidy_sid).ok_or_else(|| String::from("missing tidy-col"))?;
    if ds_before.kind() != TableKind::Column || ds_before.archetype.column_count() != 2 || ds_before.archetype.row_count() != 2 {
        return Err(format!(
            "expected Column 2x2 tidy-col table, got {:?} {}x{}",
            ds_before.kind(),
            ds_before.archetype.row_count(),
            ds_before.archetype.column_count()
        ));
    }

    // Preview Archetype Convert via Change/Convert Table Structure... then apply via ui.table.convertArchetype.open modal.
    click_widget(&mut app, "nav.sheet.tidy-col")?;
    let preview_out = invoke_menu(
        &mut app,
        "Change/Convert Table Structure...",
        json!({ "sheet_id": "tidy-col", "target_kind": "Grouped", "preview_only": true }),
    )?;
    if preview_out.pointer("/payload/preview/preserved_cells").and_then(Value::as_u64).unwrap_or(0) < 4 {
        return Err(format!("expected >= 4 preserved_cells in conversion preview, got {preview_out}"));
    }

    invoke_menu(&mut app, "Data/Convert Table Archetype...", json!({}))?;
    let conv_dlg = inspect(&mut app, "dialog", false)?;
    if !conv_dlg.pointer("/dialog/title").and_then(Value::as_str).unwrap_or("").contains("Convert") {
        return Err(format!("expected Convert Table Archetype dialog, got {conv_dlg}"));
    }
    gui(&mut app, "ui.answer_dialog", json!({ "fields": { "target_kind": "Grouped", "preview_only": true } }))?;
    let conv_preview_dlg = inspect(&mut app, "dialog", false)?;
    let preview_ready = conv_preview_dlg
        .pointer("/dialog/controls")
        .and_then(Value::as_array)
        .and_then(|arr| arr.iter().find(|c| c.get("widget_id").and_then(Value::as_str) == Some("convert.preview_ready")))
        .and_then(|c| c.get("value"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !preview_ready {
        return Err(format!("expected convert.preview_ready=true after preview_only patch: {conv_preview_dlg}"));
    }
    gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "preview_only": false, "allow_lossy": true } }))?;

    let ds_after = app.document().data_sheet(&tidy_sid).ok_or_else(|| String::from("missing tidy-col after convert"))?;
    if ds_after.kind() != TableKind::Grouped {
        return Err(format!("expected tidy-col converted to Grouped, got {:?}", ds_after.kind()));
    }
    let _png_bytes = assert_screenshot_png(&mut app, "full_window")?;
    Ok(())
}

#[test]
fn wf05_transform_pca_and_clustering_pipeline() -> TestRes {
    let mut app = new_strict_gui_app();

    invoke_menu(
        &mut app,
        "Insert/New Data Table",
        json!({
            "sheet_id": "mv-omics",
            "kind": "multiple_variables",
            "title": "Omics Matrix",
            "rows": 12,
            "cols": 4,
            "variables": ["GeneA", "GeneB", "GeneC", "GeneD"],
            "auto_graph": true
        }),
    )?;

    click_widget(&mut app, "grid.cell.0.0.0")?;
    type_in_widget(
        &mut app,
        "grid.cell.0.0.0",
        "1.0\t2.0\t10.0\t11.0\n\
         1.2\t2.1\t10.5\t11.2\n\
         5.0\t6.0\t4.0\t3.0\n\
         5.3\t6.2\t4.1\t3.2\n\
         9.0\t10.0\t1.0\t2.0\n\
         9.4\t10.3\t1.2\t2.1\n\
         1.5\t2.4\t9.8\t10.9\n\
         1.6\t2.6\t9.6\t10.7\n\
         5.1\t6.1\t4.3\t3.1\n\
         5.4\t6.3\t4.4\t3.4\n\
         9.2\t10.2\t1.3\t2.2\n\
         9.5\t10.5\t1.4\t2.3",
    )?;

    // Stage 1: Transform/PruneRows (every 2nd row -> 6 rows x 4 variables).
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let prune_out = gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "transform.prune" } }))?;
    let prune_id =
        prune_out.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("prune did not return sheet_id"))?.to_owned();
    let prune_sid = SheetId::try_new(&prune_id).map_err(|e| e.to_string())?;
    let pruned_rows = app.document().result_sheet(&prune_sid).and_then(|rs| rs.analyzable_data.as_ref()).map_or(0, |d| d.archetype.row_count());
    if pruned_rows != 6 {
        return Err(format!("expected 6 pruned rows, got {pruned_rows}"));
    }

    // Stage 2: ResultSheet active -> Analyze... PCA directly on pruned ResultSheet.
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let pca_out = gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.pca" } }))?;
    let pca_id = pca_out.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("PCA did not return sheet_id"))?.to_owned();
    let pc1_ev = result_sheet_scalar(&app, &pca_id, "PC1 eigenvalue")?;
    if !pc1_ev.is_finite() || pc1_ev <= 0.0 {
        return Err(format!("expected positive finite PC1 eigenvalue, got {pc1_ev}"));
    }

    // Stage 3: Switch ResultSheet tab to `pca-loadings` and chain Cluster Analysis (K-means++ & Ward.D2) via Analyze...
    gui(&mut app, "ui.switch_tab", json!({ "tab": "pca-loadings" }))?;
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let cluster_dlg = inspect(&mut app, "dialog", false)?;
    if cluster_dlg.pointer("/dialog/stage").and_then(Value::as_str) != Some("pca-loadings") {
        return Err(format!("expected dialog.stage == `pca-loadings`, got {cluster_dlg}"));
    }
    let clust_out = gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.clustering" } }))?;
    let clust_id =
        clust_out.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("clustering did not return sheet_id"))?.to_owned();

    let clust_sid = SheetId::try_new(&clust_id).map_err(|e| e.to_string())?;
    let clust_rs = app.document().result_sheet(&clust_sid).ok_or_else(|| String::from("missing clustering ResultSheet"))?;
    if !matches!(clust_rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })) {
        return Err(format!("unexpected clustering analysis kind: {:?}", clust_rs.analysis.kind));
    }
    let has_kmeans = clust_rs.tabs.iter().any(|t| t.id == "kmeans-clusters" && t.rows.len() == 3);
    let has_ward = clust_rs.tabs.iter().any(|t| t.id == "hierarchical-dendrogram" && !t.rows.is_empty());
    if !has_kmeans || !has_ward {
        return Err(format!("expected both k-means and hierarchical dendrogram tabs, got {:?}", clust_rs.tabs));
    }
    let _png_bytes = assert_screenshot_png(&mut app, "full_window")?;
    Ok(())
}

#[test]
fn wf06_prune_descriptive_survival_chaining_and_reactive_recompute() -> TestRes {
    let mut app = new_strict_gui_app();

    // MultipleVariables clinical survival table suitable for PruneRows -> DescriptiveStats & Kaplan-Meier Survival.
    invoke_menu(
        &mut app,
        "Insert/New Data Table",
        json!({
            "sheet_id": "mv-surv",
            "kind": "multiple_variables",
            "title": "Clinical Survival",
            "rows": 12,
            "cols": 4,
            "variables": [
                { "title": "Cohort", "role": "categorical" },
                { "title": "Time", "role": "continuous" },
                { "title": "Status", "role": "continuous" },
                { "title": "Biomarker", "role": "continuous" }
            ],
            "auto_graph": true
        }),
    )?;

    click_widget(&mut app, "grid.cell.0.0.0")?;
    type_in_widget(
        &mut app,
        "grid.cell.0.0.0",
        "Ctrl\t10\t1\t100.0\n\
         Ctrl\t12\t0\t105.0\n\
         Ctrl\t16\t1\t120.0\n\
         Ctrl\t18\t1\t125.0\n\
         Ctrl\t22\t0\t140.0\n\
         Ctrl\t25\t1\t145.0\n\
         Drug\t20\t1\t200.0\n\
         Drug\t24\t0\t205.0\n\
         Drug\t30\t1\t220.0\n\
         Drug\t32\t0\t225.0\n\
         Drug\t40\t1\t240.0\n\
         Drug\t44\t0\t245.0",
    )?;

    // Stage 1: Prune rows (keep even rows 0,2,4,6,8,10 -> 3 Ctrl + 3 Drug).
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let prune_out = gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "transform.prune" } }))?;
    let prune_id =
        prune_out.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("prune did not return sheet_id"))?.to_owned();

    // Stage 2: Active Prune ResultSheet -> Analyze... Descriptive Statistics.
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let desc_out = gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.descriptive" } }))?;
    let desc_id =
        desc_out.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("descriptive did not return sheet_id"))?.to_owned();
    let desc_sid = SheetId::try_new(&desc_id).map_err(|e| e.to_string())?;
    let bio_mean_before = {
        let desc_rs = app.document().result_sheet(&desc_sid).ok_or_else(|| String::from("missing descriptive ResultSheet"))?;
        if desc_rs.analysis.source_sheet_id.as_str() != prune_id {
            return Err(format!("expected descriptive parent `{prune_id}`, got `{}`", desc_rs.analysis.source_sheet_id));
        }
        let tab = desc_rs.tabs.iter().find(|t| t.id == "desc-summary").ok_or_else(|| String::from("missing desc-summary tab"))?;
        let bio_idx = tab.column_headers.iter().position(|h| h == "Biomarker").ok_or_else(|| String::from("missing Biomarker col"))?;
        let mean_row = tab.row_labels.iter().position(|r| r == "Mean").ok_or_else(|| String::from("missing Mean row"))?;
        // Pruned Biomarker values at rows [0,2,4,6,8,10] = [100, 120, 140, 200, 220, 240], mean = 170.0
        assert_result_cell_close(tab.rows.get(mean_row).and_then(|r| r.get(bio_idx)), 170.0, 1e-6, "Biomarker Mean before edit")?;
        170.0
    };

    // Stage 3: Switch back to Prune ResultSheet and chain Kaplan-Meier Survival analysis.
    click_widget(&mut app, &format!("nav.sheet.{prune_id}"))?;
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let surv_out = gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.survival_km" } }))?;
    let surv_id =
        surv_out.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("survival_km did not return sheet_id"))?.to_owned();
    let ctrl_median_before = result_sheet_scalar(&app, &surv_id, "Median survival (Ctrl)")?;
    if !ctrl_median_before.is_finite() {
        return Err(String::from("expected finite Ctrl median survival"));
    }

    // Stage 4: Multiple Imputation check via Analyze... on a table with missing data.
    invoke_menu(
        &mut app,
        "Insert/New Data Table",
        json!({
            "sheet_id": "mv-impute",
            "kind": "multiple_variables",
            "title": "Impute Cohort",
            "rows": 6,
            "cols": 3,
            "variables": ["X1", "X2", "X3"]
        }),
    )?;
    click_widget(&mut app, "grid.cell.0.0.0")?;
    type_in_widget(&mut app, "grid.cell.0.0.0", "10\t20\t30\n12\t\t36\n14\t28\t42\n16\t32\t48\n18\t36\t54\n20\t40\t60")?;
    invoke_menu(&mut app, "Analyze/Analyze...", json!({}))?;
    let mi_out = gui(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.multiple_imputation" } }))?;
    let mi_id = mi_out
        .pointer("/payload/sheet_id")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("multiple_imputation did not return sheet_id"))?
        .to_owned();
    let missing_cnt = result_sheet_scalar(&app, &mi_id, "Total missing cells")?;
    if (missing_cnt - 1.0).abs() > 1e-9 {
        return Err(format!("expected 1 imputed missing cell, got {missing_cnt}"));
    }

    // Stage 5: Reactive recompute when source Biomarker cell in kept row 0 is edited via ui.type_text (+60 -> mean +10).
    click_widget(&mut app, "nav.sheet.mv-surv")?;
    click_widget(&mut app, "grid.cell.0.3.0")?;
    type_in_widget(&mut app, "grid.cell.0.3.0", "160.0")?;
    let desc_rs_after = app.document().result_sheet(&desc_sid).ok_or_else(|| String::from("missing desc after edit"))?;
    let tab_after = desc_rs_after.tabs.iter().find(|t| t.id == "desc-summary").ok_or_else(|| String::from("missing desc-summary after edit"))?;
    let bio_idx_after = tab_after.column_headers.iter().position(|h| h == "Biomarker").ok_or_else(|| String::from("missing Biomarker col after"))?;
    let mean_row_after = tab_after.row_labels.iter().position(|r| r == "Mean").ok_or_else(|| String::from("missing Mean row after"))?;
    assert_result_cell_close(
        tab_after.rows.get(mean_row_after).and_then(|r| r.get(bio_idx_after)),
        bio_mean_before + 10.0,
        1e-6,
        "Biomarker Mean after reactive edit",
    )?;

    let _png_bytes = assert_screenshot_png(&mut app, "full_window")?;
    Ok(())
}
