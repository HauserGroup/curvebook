#![forbid(unsafe_code)]

use curvebook_doc::{
    Cell, CellCoord, ChecklistItem, DataSheet, EquationDisplayMode, GraphAnnotation, GraphSheet, ResultCell, ResultSheet, ResultTab, ResultTabKind,
    SheetId, TableArchetype, TableKind, VariableSource,
};
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn gui(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed: {err}"))
}

fn gui_err(app: &mut UiApp, method: &str, params: Value, ctx: &str) -> Result<String, String> {
    match dispatch_gui_action(app, method, params) {
        Ok(val) => Err(format!("{ctx}: expected error, got {val}")),
        Err(err) => Ok(err.to_string()),
    }
}

fn menu(app: &mut UiApp, command_id: &str, params: Value) -> Result<Value, String> {
    gui(app, "ui.invoke_menu", json!({ "command_id": command_id, "params": params }))
}

fn click(app: &mut UiApp, widget_id: &str) -> Result<Value, String> {
    gui(app, "ui.pointer_gesture", json!({ "widget_id": widget_id, "gesture": "click" }))
}

fn type_widget(app: &mut UiApp, widget_id: &str, text: &str, clear_existing: bool, commit_enter: bool) -> Result<Value, String> {
    gui(
        app,
        "ui.type_text",
        json!({
            "widget_id": widget_id,
            "text": text,
            "clear_existing": clear_existing,
            "commit_enter": commit_enter,
        }),
    )
}

fn answer_fields(app: &mut UiApp, fields: Value) -> Result<Value, String> {
    gui(app, "ui.answer_dialog", json!({ "fields": fields }))
}

fn answer_ok(app: &mut UiApp, fields: Value) -> Result<Value, String> {
    gui(app, "ui.answer_dialog", json!({ "action": "ok", "fields": fields }))
}

fn press_key(app: &mut UiApp, key: &str, meta: bool) -> Result<Value, String> {
    let mut params = json!({ "key": key });
    if meta {
        params["modifiers"] = json!({ "meta": true });
    }
    gui(app, "ui.press_key", params)
}

fn switch_tab(app: &mut UiApp, sheet_id: &str) -> TestRes {
    gui(app, "ui.switch_tab", json!({ "sheet_id": sheet_id }))?;
    Ok(())
}

fn inspect(app: &mut UiApp, target: &str) -> Result<Value, String> {
    gui(app, "ui.inspect", json!({ "target": target }))
}

fn new_strict_app() -> UiApp {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    app
}

fn create_mv_sheet(app: &mut UiApp, sheet_id: &str, title: &str, cols: &[(&str, &str)], rows: usize, auto_family: bool) -> TestRes {
    let variables: Vec<Value> = cols.iter().map(|(col_title, role)| json!({ "title": col_title, "role": role })).collect();
    menu(
        app,
        "sheet.createData",
        json!({
            "sheet_id": sheet_id,
            "title": title,
            "kind": "multiple_variables",
            "cols": cols.len(),
            "rows": rows,
            "variables": variables,
            "auto_family": auto_family,
        }),
    )?;
    Ok(())
}

fn paste_tsv_at(app: &mut UiApp, row: usize, col: usize, tsv: &str) -> TestRes {
    let widget_id = format!("grid.cell.{row}.{col}.0");
    click(app, &widget_id)?;
    type_widget(app, &widget_id, tsv, false, false)?;
    Ok(())
}

fn edit_cell_at(app: &mut UiApp, row: usize, col: usize, value: &str) -> TestRes {
    let widget_id = format!("grid.cell.{row}.{col}.0");
    click(app, &widget_id)?;
    type_widget(app, &widget_id, value, true, true)?;
    Ok(())
}

fn exclude_cell_with_reason(app: &mut UiApp, row: usize, col: usize, reason_tag: &str, note: &str) -> TestRes {
    let widget_id = format!("grid.cell.{row}.{col}.0");
    click(app, &widget_id)?;
    menu(app, "ui.grid.excludeWithReason", json!({}))?;
    answer_ok(
        app,
        json!({
            "reason_tag": reason_tag,
            "note": note,
        }),
    )?;
    Ok(())
}

fn formula_bar_is_valid(val: &Value) -> bool {
    val["is_valid"].as_bool().or_else(|| val["value"]["is_valid"].as_bool()).unwrap_or(false)
}

fn set_formula_via_bar(app: &mut UiApp, col: usize, formula: &str) -> TestRes {
    menu(app, "ui.table.openFormulaBar", json!({ "column": col }))?;
    let typed = type_widget(app, "formula_bar.input", formula, true, false)?;
    if !formula_bar_is_valid(&typed) {
        return Err(format!("formula `{formula}` for col {col} expected valid, inspect: {typed}"));
    }
    press_key(app, "Enter", false)?;
    Ok(())
}

fn open_analyze(app: &mut UiApp, entry_id: &str) -> TestRes {
    menu(app, "ui.analyze.open", json!({}))?;
    let dlg = inspect(app, "dialog")?;
    if dlg["active_dialog"].as_str() != Some("analyze_sheet") {
        answer_ok(app, json!({ "choice": "create_new" }))?;
    }
    answer_fields(app, json!({ "entry_id": entry_id }))?;
    let after = inspect(app, "dialog")?;
    let selected = after["dialog"]["controls"]
        .as_array()
        .and_then(|c| c.iter().find(|w| w["widget_id"].as_str() == Some("analyze.entry")))
        .and_then(|w| w["value"].as_str());
    if selected != Some(entry_id) {
        return Err(format!("open_analyze({entry_id}) did not select entry (got {selected:?}); dlg={after}"));
    }
    Ok(())
}

fn finish_analyze_dialog(app: &mut UiApp, fields: Value, before: &[String]) -> Result<String, String> {
    if fields.as_object().is_some_and(|m| !m.is_empty()) {
        answer_fields(app, fields)?;
    }
    let dlg_before = inspect(app, "dialog")?;
    let mut ans = answer_ok(app, json!({}))?;
    if ans["dialog"].as_str() == Some("duplicate_analysis_prompt") {
        ans = answer_ok(app, json!({ "choice": "create_new" }))?;
    }
    if let Ok(rs_id) = new_result_id_after(app, before) {
        return Ok(rs_id);
    }
    Err(format!("no new ResultSheet created; answer_ok={ans}, dlg_before={dlg_before}"))
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("invalid base64 byte: {b}")),
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

fn assert_screenshot_png(app: &mut UiApp, target: &str) -> TestRes {
    let shot = gui(
        app,
        "ui.screenshot",
        json!({
            "target": target,
            "width_px": 720,
            "height_px": 480,
            "dpi": 144,
        }),
    )?;
    if shot["format"].as_str() != Some("png") {
        return Err(format!("screenshot format != png: {shot}"));
    }
    if !shot["path"].is_null() {
        return Err(format!("screenshot path expected null, got: {shot}"));
    }
    let b64 = shot["data_base64"].as_str().ok_or_else(|| format!("missing data_base64 in screenshot: {shot}"))?;
    let bytes = decode_base64(b64)?;
    if bytes.len() < 128 {
        return Err(format!("PNG too small ({} bytes)", bytes.len()));
    }
    if &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("screenshot bytes lack PNG magic header".to_string());
    }
    Ok(())
}

fn result_sheet_ids(app: &UiApp) -> Vec<String> {
    app.document().result_sheets.iter().map(|rs| rs.id.as_str().to_owned()).collect()
}

fn new_result_id_after(app: &UiApp, before: &[String]) -> Result<String, String> {
    app.document()
        .result_sheets
        .iter()
        .rev()
        .find(|rs| !before.iter().any(|b| b == rs.id.as_str()))
        .map(|rs| rs.id.as_str().to_owned())
        .ok_or_else(|| "no new ResultSheet created".to_string())
}

fn result_sheet<'a>(app: &'a UiApp, id: &str) -> Result<&'a ResultSheet, String> {
    app.document().result_sheet(&SheetId::from(id)).ok_or_else(|| format!("ResultSheet `{id}` not found"))
}

fn data_sheet<'a>(app: &'a UiApp, id: &str) -> Result<&'a DataSheet, String> {
    app.document().data_sheet(&SheetId::from(id)).ok_or_else(|| format!("DataSheet `{id}` not found"))
}

fn graph_sheet<'a>(app: &'a UiApp, id: &str) -> Result<&'a GraphSheet, String> {
    app.document().graph_sheet(&SheetId::from(id)).ok_or_else(|| format!("GraphSheet `{id}` not found"))
}

fn graph_for_source(app: &UiApp, source_id: &str) -> Result<String, String> {
    app.document()
        .graph_sheets
        .iter()
        .find(|gs| gs.spec.primary_source_sheet_id.as_str() == source_id)
        .map(|gs| gs.id.as_str().to_owned())
        .ok_or_else(|| format!("no GraphSheet bound to `{source_id}`"))
}

fn tab_by_id<'a>(rs: &'a ResultSheet, id: &str) -> Result<&'a ResultTab, String> {
    rs.tabs.iter().find(|t| t.id == id).ok_or_else(|| {
        let ids: Vec<&str> = rs.tabs.iter().map(|t| t.id.as_str()).collect();
        format!("tab `{id}` not found in `{}` (have {ids:?})", rs.id.as_str())
    })
}

fn tab_by_kind(rs: &ResultSheet, kind: ResultTabKind) -> Result<&ResultTab, String> {
    rs.tabs.iter().find(|t| t.kind == kind).ok_or_else(|| format!("tab kind {kind:?} not found in `{}`", rs.id.as_str()))
}

fn scalar_f64(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    rs.lookup_scalar(key, None).ok_or_else(|| format!("scalar `{key}` missing in `{}`", rs.id.as_str()))
}

fn cell_f64(cell: Option<&ResultCell>, desc: &str) -> Result<f64, String> {
    match cell {
        Some(ResultCell::Number { value, .. }) => Ok(*value),
        Some(ResultCell::PValue { raw, .. }) => Ok(*raw),
        other => Err(format!("cell {desc} not numeric: {other:?}")),
    }
}

fn tab_cell_f64(tab: &ResultTab, row_label: &str, col_name: &str) -> Result<f64, String> {
    let row_idx = tab
        .row_labels
        .iter()
        .position(|r| r == row_label)
        .ok_or_else(|| format!("row `{row_label}` missing in tab `{}` (rows={:?})", tab.id, tab.row_labels))?;
    let col_idx = tab
        .column_headers
        .iter()
        .position(|c| c == col_name)
        .ok_or_else(|| format!("col `{col_name}` missing in tab `{}` (cols={:?})", tab.id, tab.column_headers))?;
    cell_f64(tab.rows.get(row_idx).and_then(|r| r.get(col_idx)), &format!("({row_label}, {col_name}) in `{}`", tab.id))
}

fn ds_num_cell(ds: &DataSheet, col: usize, row: usize) -> Result<f64, String> {
    ds.cell(CellCoord::new(row, col, 0))
        .and_then(Cell::active_number)
        .ok_or_else(|| format!("expected active numeric cell at {}[r{row}, c{col}]", ds.id.as_str()))
}

fn mv_num_cell(app: &UiApp, sheet_id: &str, col: usize, row: usize) -> Result<f64, String> {
    ds_num_cell(data_sheet(app, sheet_id)?, col, row)
}

fn assert_qol7_diagnostics_only(app: &mut UiApp, sheet_id: &str) -> TestRes {
    for item in &result_sheet(app, sheet_id)?.checklist {
        let item: &ChecklistItem = item;
        if item.remediation.is_some() {
            return Err(format!("QOL-7 violation: checklist item `{}` has remediation {:?}", item.assumption, item.remediation));
        }
    }
    switch_tab(app, sheet_id)?;
    let err = match menu(app, "ui.result.applyRemediation", json!({ "sheet_id": sheet_id })) {
        Ok(val) => return Err(format!("QOL-7 applyRemediation rejection: expected error, got {val}")),
        Err(e) => e,
    };
    if !err.contains("diagnostics_only_policy") {
        return Err(format!("expected diagnostics_only_policy error, got: {err}"));
    }
    let diag = menu(app, "ui.result.inspectDiagnostics", json!({ "sheet_id": sheet_id }))?;
    if diag["result"]["mutated"].as_bool() != Some(false) {
        return Err(format!("inspectDiagnostics expected mutated=false: {diag}"));
    }
    Ok(())
}

fn assert_approx(actual: f64, expected: f64, tol: f64, label: &str) -> TestRes {
    if (actual - expected).abs() > tol || !actual.is_finite() {
        return Err(format!("{label}: expected {expected} +/- {tol}, got {actual}"));
    }
    Ok(())
}

#[test]
fn wf29_select_transform_row_stats_and_baseline_subtraction() -> TestRes {
    let mut app = new_strict_app();
    create_mv_sheet(
        &mut app,
        "mv_wf29",
        "Fluorescence Timecourse",
        &[
            ("SampleID", "label"),
            ("Batch", "categorical"),
            ("Condition", "categorical"),
            ("Conc_uM", "continuous"),
            ("Signal_T0", "continuous"),
            ("Signal_T1", "continuous"),
            ("Signal_T2", "continuous"),
        ],
        8,
        true,
    )?;
    let tsv = "\
S1\tA\tControl\t0.01\t100.0\t105.0\t110.0
S2\tA\tControl\t0.1\t102.0\t108.0\t112.0
S3\tB\tControl\t1.0\t98.0\t101.0\t104.0
S4\tB\tControl\t10.0\t100.0\t103.0\t107.0
S5\tA\tTreated\t0.01\t100.0\t120.0\t150.0
S6\tA\tTreated\t0.1\t100.0\t140.0\t190.0
S7\tB\tTreated\t1.0\t100.0\t160.0\t230.0
S8\tB\tTreated\t10.0\t100.0\t180.0\t270.0";
    paste_tsv_at(&mut app, 0, 0, tsv)?;

    // 1) Select & Transform on MV: keep Treated rows with Conc_uM >= 0.1, subset [Conc_uM, Signal_T0..T2].
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "transform.select")?;
    let sel_id = finish_analyze_dialog(
        &mut app,
        json!({
            "selected_data_sets": [3, 4, 5, 6],
            "target_archetype": "MultipleVariables",
            "filter_formula": "[Condition] == \"Treated\" && [Conc_uM] >= 0.1",
        }),
        &before,
    )?;
    let rs_sel = result_sheet(&app, &sel_id)?;
    let sel_ds = rs_sel.analyzable_data.as_ref().ok_or("transform.select expected analyzable_data")?;
    if sel_ds.kind() != TableKind::MultipleVariables || sel_ds.archetype.column_count() != 4 {
        return Err(format!("unexpected selected MV schema: {:?} cols={}", sel_ds.kind(), sel_ds.archetype.column_count()));
    }
    if sel_ds.archetype.row_count() != 3 {
        return Err(format!("expected 3 filtered Treated rows (>=0.1), got {}", sel_ds.archetype.row_count()));
    }

    // 2) Chain ConcentrationToNegativeLogM on derived filtered MV sheet (Conc_uM -> -log10(M)).
    switch_tab(&mut app, &sel_id)?;
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "transform.math")?;
    let math_id = finish_analyze_dialog(
        &mut app,
        json!({
            "selected_data_sets": [0],
            "x_transform": null,
            "y_transform": { "ConcentrationToNegativeLogM": { "molar_unit_factor": 1e-6 } },
        }),
        &before,
    )?;
    let rs_math = result_sheet(&app, &math_id)?;
    let math_ds = rs_math.analyzable_data.as_ref().ok_or("transform.math expected analyzable_data")?;
    assert_approx(ds_num_cell(math_ds, 0, 0)?, 7.0, 1e-6, "-log10(0.1 uM)")?;
    assert_approx(ds_num_cell(math_ds, 0, 2)?, 5.0, 1e-6, "-log10(10 uM)")?;

    // 3) Row descriptive statistics on MV Signal_T0..T2.
    switch_tab(&mut app, "mv_wf29")?;
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "stats.descriptive")?;
    let stats_id = finish_analyze_dialog(&mut app, json!({ "selected_data_sets": [4, 5, 6] }), &before)?;
    let rs_stats = result_sheet(&app, &stats_id)?;
    let _desc = tab_by_id(rs_stats, "desc-summary")?;
    let row_stats = tab_by_id(rs_stats, "row-descriptive-stats")?;
    if row_stats.kind != ResultTabKind::Diagnostics {
        return Err("row-descriptive-stats expected Diagnostics tab".to_string());
    }
    let mean_col = row_stats.column_headers.iter().position(|c| c == "Mean").ok_or("missing Mean column in row-descriptive-stats")?;
    let mean_r7_before = cell_f64(row_stats.rows.get(7).and_then(|r| r.get(mean_col)), "row 7 mean")?;
    assert_approx(mean_r7_before, (100.0 + 180.0 + 270.0) / 3.0, 1e-4, "row 7 mean")?;

    // 4) Remove Baseline on Signal_T0..T2 using Signal_T0 baseline.
    switch_tab(&mut app, "mv_wf29")?;
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "transform.baseline")?;
    let base_id = finish_analyze_dialog(
        &mut app,
        json!({
            "analyze.transform.selected_data_sets": [4, 5, 6],
            "analyze.transform.baseline_columns": [4],
            "analyze.transform.baseline_operation": "Difference",
        }),
        &before,
    )?;
    let rs_base = result_sheet(&app, &base_id)?;
    let base_ds = rs_base.analyzable_data.as_ref().ok_or("transform.baseline expected analyzable_data")?;
    let s8_t2_diff = ds_num_cell(base_ds, 6, 7)?;
    assert_approx(s8_t2_diff, 170.0, 1e-6, "S8 T2 baseline difference")?;

    // 5) Inspect & screenshot graph, then verify live upstream edit cascades to all downstream sheets.
    let mv_graph = graph_for_source(&app, "mv_wf29")?;
    switch_tab(&mut app, &mv_graph)?;
    let g_insp = inspect(&mut app, "graph")?;
    if g_insp["active_sheet_id"].as_str() != Some(mv_graph.as_str()) {
        return Err(format!("graph inspect mismatch: {g_insp}"));
    }
    assert_screenshot_png(&mut app, "canvas")?;

    switch_tab(&mut app, "mv_wf29")?;
    edit_cell_at(&mut app, 7, 6, "300.0")?;
    let rs_sel_after = result_sheet(&app, &sel_id)?;
    let sel_ds_after = rs_sel_after.analyzable_data.as_ref().ok_or("missing filtered analyzable_data after edit")?;
    assert_approx(ds_num_cell(sel_ds_after, 3, 2)?, 300.0, 1e-6, "filtered MV reactive update")?;

    let rs_base_after = result_sheet(&app, &base_id)?;
    let base_ds_after = rs_base_after.analyzable_data.as_ref().ok_or("missing baseline analyzable_data after edit")?;
    assert_approx(ds_num_cell(base_ds_after, 6, 7)?, 200.0, 1e-6, "baseline reactive update")?;

    let row_stats_after = tab_by_id(result_sheet(&app, &stats_id)?, "row-descriptive-stats")?;
    let mean_r7_after = cell_f64(row_stats_after.rows.get(7).and_then(|r| r.get(mean_col)), "row 7 mean after edit")?;
    assert_approx(mean_r7_after, (100.0 + 180.0 + 300.0) / 3.0, 1e-4, "row stats reactive mean")?;
    Ok(())
}

#[test]
fn wf30_calculated_variables_formula_bar_and_circularity_guards() -> TestRes {
    let mut app = new_strict_app();
    create_mv_sheet(
        &mut app,
        "mv_wf30",
        "Calculated Variables & Guards",
        &[
            ("SampleID", "label"),
            ("Group", "categorical"),
            ("RawSignal", "continuous"),
            ("RefControl", "continuous"),
            ("Delta", "continuous"),
            ("Fold", "continuous"),
        ],
        6,
        false,
    )?;
    let tsv = "\
C1\tCtrl\t10.0\t10.0
C2\tCtrl\t12.0\t10.0
C3\tCtrl\t8.0\t10.0
D1\tDrug\t6.0\t10.0
D2\tDrug\t7.0\t10.0
D3\tDrug\t5.0\t10.0";
    paste_tsv_at(&mut app, 0, 0, tsv)?;

    // Formula on Delta: = [RawSignal] - [RefControl]
    set_formula_via_bar(&mut app, 4, "= [RawSignal] - [RefControl]")?;
    assert_approx(mv_num_cell(&app, "mv_wf30", 4, 0)?, 0.0, 1e-6, "Delta C1")?;
    assert_approx(mv_num_cell(&app, "mv_wf30", 4, 3)?, -4.0, 1e-6, "Delta D1")?;

    // Open Fold formula bar by clicking header button and verify invalid syntax + self-cycle rejection.
    click(&mut app, "formula_bar.open_col.5")?;
    let bad_syntax = type_widget(&mut app, "formula_bar.input", "= [RawSignal +", true, false)?;
    if formula_bar_is_valid(&bad_syntax) {
        return Err(format!("invalid syntax should mark is_valid=false: {bad_syntax}"));
    }
    gui_err(&mut app, "ui.pointer_gesture", json!({ "widget_id": "formula_bar.commit", "gesture": "click" }), "commit invalid formula syntax")?;

    let self_cycle = type_widget(&mut app, "formula_bar.input", "= [Fold] + 1", true, false)?;
    if formula_bar_is_valid(&self_cycle) {
        return Err(format!("self-cycle should mark is_valid=false: {self_cycle}"));
    }
    gui_err(&mut app, "ui.press_key", json!({ "key": "Enter" }), "commit self-cycle formula")?;

    // Commit valid conditional exponential formula referencing Delta.
    let valid_fold = type_widget(&mut app, "formula_bar.input", "= IF([Group] == \"Drug\", 2 ^ (-[Delta]), 1.0)", true, false)?;
    if !formula_bar_is_valid(&valid_fold) {
        return Err(format!("valid conditional formula failed: {valid_fold}"));
    }
    click(&mut app, "formula_bar.commit")?;
    assert_approx(mv_num_cell(&app, "mv_wf30", 5, 0)?, 1.0, 1e-6, "Fold Ctrl")?;
    assert_approx(mv_num_cell(&app, "mv_wf30", 5, 3)?, 16.0, 1e-6, "Fold Drug D1")?;

    // Multi-hop circularity guard: Delta = [Fold] * 2 creates Delta -> Fold -> Delta.
    menu(&mut app, "ui.table.openFormulaBar", json!({ "column": 4 }))?;
    let multi_cycle = type_widget(&mut app, "formula_bar.input", "= [Fold] * 2", true, false)?;
    if formula_bar_is_valid(&multi_cycle) {
        return Err(format!("multi-hop cycle should mark is_valid=false: {multi_cycle}"));
    }
    gui_err(&mut app, "ui.press_key", json!({ "key": "Enter" }), "commit multi-hop circular formula")?;
    press_key(&mut app, "Escape", false)?;

    // Verify original Delta formula preserved.
    let ds = data_sheet(&app, "mv_wf30")?;
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return Err("mv_wf30 expected MultipleVariables".to_string());
    };
    match &mv.variables[4].source {
        VariableSource::Calculated { formula } => {
            if !formula.contains("RawSignal") {
                return Err(format!("unexpected preserved Delta formula: {formula}"));
            }
        }
        other => return Err(format!("Delta expected Calculated source, got {other:?}")),
    }

    // Calculated cell read-only guard.
    let grid_insp = inspect(&mut app, "grid")?;
    let calc_cell_meta = grid_insp["grid_cells"]
        .as_array()
        .and_then(|arr| arr.iter().find(|w| w["widget_id"].as_str() == Some("grid.cell.0.4.0")))
        .ok_or("missing grid.cell.0.4.0 in grid inspect")?;
    if calc_cell_meta["enabled"].as_bool() != Some(false) {
        return Err(format!("calculated cell should be disabled: {calc_cell_meta}"));
    }
    gui_err(&mut app, "ui.pointer_gesture", json!({ "widget_id": "grid.cell.0.4.0", "gesture": "click" }), "click calculated cell")?;
    gui_err(
        &mut app,
        "ui.type_text",
        json!({ "widget_id": "grid.cell.0.4.0", "text": "99", "clear_existing": true, "commit_enter": true }),
        "type into calculated cell",
    )?;

    // Live upstream edit on RawSignal D1 (6.0 -> 8.0) cascades through Delta (-2.0) and Fold (4.0).
    edit_cell_at(&mut app, 3, 2, "8.0")?;
    assert_approx(mv_num_cell(&app, "mv_wf30", 4, 3)?, -2.0, 1e-6, "Delta D1 updated")?;
    assert_approx(mv_num_cell(&app, "mv_wf30", 5, 3)?, 4.0, 1e-6, "Fold D1 cascaded")?;

    // Check sheet metadata + screenshot on canvas.
    let sheets_insp = inspect(&mut app, "sheets")?;
    if !sheets_insp["navigator_sheets"].is_array() {
        return Err(format!("expected navigator_sheets array in inspect: {sheets_insp}"));
    }
    assert_screenshot_png(&mut app, "canvas")?;
    Ok(())
}

#[test]
fn wf31_extract_and_rearrange_cross_schema_chaining() -> TestRes {
    let mut app = new_strict_app();
    create_mv_sheet(
        &mut app,
        "mv_wf31",
        "Compound Screen MV",
        &[("SampleID", "label"), ("Genotype", "categorical"), ("Phenotype", "categorical"), ("LogConc", "continuous"), ("Viability", "continuous")],
        12,
        false,
    )?;
    let tsv = "\
W1\tWT\tResponder\t-9.0\t98.0
W2\tWT\tResponder\t-8.0\t92.0
W3\tWT\tResponder\t-7.0\t75.0
W4\tWT\tNonResponder\t-6.0\t48.0
W5\tWT\tNonResponder\t-5.0\t20.0
W6\tWT\tNonResponder\t-4.0\t6.0
M1\tMut\tResponder\t-9.0\t99.0
M2\tMut\tResponder\t-8.0\t96.0
M3\tMut\tResponder\t-7.0\t94.0
M4\tMut\tResponder\t-6.0\t88.0
M5\tMut\tNonResponder\t-5.0\t52.0
M6\tMut\tNonResponder\t-4.0\t15.0";
    paste_tsv_at(&mut app, 0, 0, tsv)?;

    // Chain 1: MV -> XY via Select & Transform (WT genotype, LogConc -> X, Viability -> Y) -> 4PL Fit.
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "transform.select")?;
    let xy_res_id = finish_analyze_dialog(
        &mut app,
        json!({
            "selected_data_sets": [3, 4],
            "target_archetype": "Xy",
            "filter_formula": "[Genotype] == \"WT\"",
        }),
        &before,
    )?;
    let rs_xy = result_sheet(&app, &xy_res_id)?;
    let xy_ds = rs_xy.analyzable_data.as_ref().ok_or("MV -> XY missing analyzable_data")?;
    if xy_ds.kind() != TableKind::Xy || xy_ds.archetype.row_count() != 6 {
        return Err(format!("expected 6-row XY subset, got {:?}", xy_ds.kind()));
    }

    switch_tab(&mut app, &xy_res_id)?;
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "fit.nonlinear")?;
    let fit_res_id = finish_analyze_dialog(
        &mut app,
        json!({
            "equation_id": "log_inhibitor_vs_response_4pl",
            "selected_data_sets": [0],
        }),
        &before,
    )?;
    let rs_fit = result_sheet(&app, &fit_res_id)?;
    let param_tab = tab_by_id(rs_fit, "nonlinear_parameters")?;
    let log_ic50_before = tab_cell_f64(param_tab, "LogIC50", "Best-fit value")?;
    if !(-7.5..-5.5).contains(&log_ic50_before) {
        return Err(format!("unexpected chained WT LogIC50: {log_ic50_before}"));
    }

    // Chain 2: MV -> Column via Select & Transform -> Descriptive Statistics.
    switch_tab(&mut app, "mv_wf31")?;
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "transform.select")?;
    let col_res_id = finish_analyze_dialog(
        &mut app,
        json!({
            "selected_data_sets": [3, 4],
            "target_archetype": "Column",
            "filter_formula": "",
        }),
        &before,
    )?;
    if result_sheet(&app, &col_res_id)?.analyzable_data.as_ref().map(DataSheet::kind) != Some(TableKind::Column) {
        return Err("expected Column analyzable_data on derived sheet".to_string());
    }
    switch_tab(&mut app, &col_res_id)?;
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "stats.descriptive")?;
    let col_stats_id = finish_analyze_dialog(&mut app, json!({ "selected_data_sets": [1] }), &before)?;
    let col_desc = tab_by_id(result_sheet(&app, &col_stats_id)?, "desc-summary")?;
    let mean_viability = tab_cell_f64(col_desc, "Mean", "Viability")?;
    if !(50.0..80.0).contains(&mean_viability) {
        return Err(format!("unexpected column mean viability: {mean_viability}"));
    }

    // Chain 3: Separate categorical MV table converted via ConvertArchetype -> Contingency -> Contingency test.
    create_mv_sheet(
        &mut app,
        "mv_wf31_cat",
        "Genotype Phenotype Contingency Source",
        &[("Genotype", "categorical"), ("Phenotype", "categorical")],
        12,
        false,
    )?;
    let cat_tsv = "\
WT\tResponder
WT\tResponder
WT\tResponder
WT\tNonResponder
WT\tNonResponder
WT\tNonResponder
Mut\tResponder
Mut\tResponder
Mut\tResponder
Mut\tResponder
Mut\tNonResponder
Mut\tNonResponder";
    paste_tsv_at(&mut app, 0, 0, cat_tsv)?;
    menu(&mut app, "ui.table.convertArchetype.open", json!({ "sheet_id": "mv_wf31_cat" }))?;
    answer_ok(&mut app, json!({ "target_kind": "Contingency" }))?;
    if data_sheet(&app, "mv_wf31_cat")?.kind() != TableKind::Contingency {
        return Err("mv_wf31_cat did not convert to Contingency".to_string());
    }
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "stats.contingency")?;
    let cont_id = finish_analyze_dialog(&mut app, json!({}), &before)?;
    let rs_cont = result_sheet(&app, &cont_id)?;
    let _p = scalar_f64(rs_cont, "Chi-square P value")?;

    // Inspect results & screenshot graph linked to XY/fit, then test MV -> XY -> 4PL reactivity.
    switch_tab(&mut app, &fit_res_id)?;
    let res_insp = inspect(&mut app, "results")?;
    if res_insp["active_sheet_id"].as_str() != Some(fit_res_id.as_str()) {
        return Err(format!("results inspect mismatch: {res_insp}"));
    }
    menu(&mut app, "graph.create", json!({ "data_sheet_id": &xy_res_id, "sheet_id": "g_wf31_xy" }))?;
    switch_tab(&mut app, "g_wf31_xy")?;
    let g_insp = inspect(&mut app, "graph")?;
    if g_insp["active_sheet_id"].as_str() != Some("g_wf31_xy") {
        return Err(format!("graph inspect mismatch: {g_insp}"));
    }
    assert_screenshot_png(&mut app, "canvas")?;

    switch_tab(&mut app, "mv_wf31")?;
    edit_cell_at(&mut app, 3, 4, "80.0")?;
    let log_ic50_after = tab_cell_f64(tab_by_id(result_sheet(&app, &fit_res_id)?, "nonlinear_parameters")?, "LogIC50", "Best-fit value")?;
    if (log_ic50_after - log_ic50_before).abs() < 0.01 {
        return Err(format!("chained 4PL LogIC50 did not update reactively: before={log_ic50_before}, after={log_ic50_after}"));
    }
    Ok(())
}

#[test]
fn wf32_direct_multiple_variables_welch_t_and_effect_sizes() -> TestRes {
    let mut app = new_strict_app();
    create_mv_sheet(
        &mut app,
        "mv_wf32",
        "Biomarker Cohort MV",
        &[("SampleID", "label"), ("Treatment", "categorical"), ("Biomarker", "continuous")],
        16,
        true,
    )?;
    let tsv = "\
V1\tVehicle\t18.5
V2\tVehicle\t20.2
V3\tVehicle\t21.0
V4\tVehicle\t22.4
V5\tVehicle\t19.8
V6\tVehicle\t23.1
V7\tVehicle\t20.9
V8\tVehicle\t21.5
T1\tTreated\t31.0
T2\tTreated\t34.5
T3\tTreated\t42.0
T4\tTreated\t38.2
T5\tTreated\t46.8
T6\tTreated\t35.4
T7\tTreated\t40.1
T8\tTreated\t52.0";
    paste_tsv_at(&mut app, 0, 0, tsv)?;

    // Direct MV Welch t-test + effect sizes (Cohen's d, Hedges' g, Glass's Delta) + F-test.
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "stats.two_group")?;
    answer_ok(
        &mut app,
        json!({
            "two_group_method": "welch",
            "mv_group_column": 1,
            "mv_response_column": 2,
            "compute_effect_sizes": true,
        }),
    )?;
    let welch_id = new_result_id_after(&app, &before)?;
    let rs_welch = result_sheet(&app, &welch_id)?;
    let p_before = scalar_f64(rs_welch, "PValue")?;
    let cohen_before = scalar_f64(rs_welch, "CohensD")?;
    let hedges_before = scalar_f64(rs_welch, "HedgesG")?;
    if !(p_before < 0.005 && cohen_before.abs() > 1.5 && hedges_before.abs() > 1.4) {
        return Err(format!("unexpected Welch stats: p={p_before}, d={cohen_before}, g={hedges_before}"));
    }
    let tg_tab = tab_by_id(rs_welch, "two-group")?;
    let tg_col = tg_tab.column_headers.first().ok_or("two-group tab missing column headers")?.clone();
    let _f_val = tab_cell_f64(tg_tab, "F-test to compare variances (F)", &tg_col)?;
    let _f_p = tab_cell_f64(tg_tab, "F-test P value", &tg_col)?;
    let _glass = tab_cell_f64(tg_tab, "Glass's Delta", &tg_col)?;

    // Switch MV graph into Categorical Summary mode (standalone `col.estimation` preset is absent per wf25).
    let mv_graph = graph_for_source(&app, "mv_wf32")?;
    switch_tab(&mut app, &mv_graph)?;
    menu(&mut app, "ui.graph.bubblePlot.open", json!({}))?;
    answer_ok(
        &mut app,
        json!({
            "plot_mode": "categorical_summary",
            "x_column": 1,
            "y_column": 2,
            "color_mode": "categorical",
            "color_column": 1,
        }),
    )?;
    let mv_g_insp = inspect(&mut app, "graph")?;
    if mv_g_insp["active_sheet_id"].as_str() != Some(mv_graph.as_str()) {
        return Err(format!("MV categorical summary inspect mismatch: {mv_g_insp}"));
    }
    assert_screenshot_png(&mut app, "canvas")?;

    // Live reactivity: exclude Treated outlier T8 via Cmd/Ctrl+E (ui.grid.excludeWithReason).
    switch_tab(&mut app, "mv_wf32")?;
    exclude_cell_with_reason(&mut app, 15, 2, "InstrumentError", "WF32 extreme readout")?;
    let rs_welch_excl = result_sheet(&app, &welch_id)?;
    let cohen_excl = scalar_f64(rs_welch_excl, "CohensD")?;
    let t_excl = scalar_f64(rs_welch_excl, "t")?;
    if (cohen_excl - cohen_before).abs() < 0.05 || !t_excl.is_finite() {
        return Err(format!("Welch effect size did not recompute after exclusion: before={cohen_before}, after={cohen_excl}"));
    }

    // Live cell edit on Vehicle V1 updates Welch P value again.
    edit_cell_at(&mut app, 0, 2, "25.0")?;
    let p_edited = scalar_f64(result_sheet(&app, &welch_id)?, "PValue")?;
    if (p_edited - p_before).abs() < 1e-6 {
        return Err(format!("Welch PValue expected reactive update after edit: before={p_before}, after={p_edited}"));
    }
    Ok(())
}

#[test]
fn wf33_direct_multiple_variables_multifactor_anova() -> TestRes {
    let mut app = new_strict_app();
    create_mv_sheet(
        &mut app,
        "mv_wf33",
        "2x2x2 Factorial Drug Study",
        &[("SampleID", "label"), ("Genotype", "categorical"), ("Drug", "categorical"), ("Timepoint", "categorical"), ("Response", "continuous")],
        24,
        true,
    )?;
    let tsv = "\
S01\tWT\tVehicle\tEarly\t10.0
S02\tWT\tVehicle\tEarly\t11.2
S03\tWT\tVehicle\tEarly\t10.5
S04\tWT\tVehicle\tLate\t12.0
S05\tWT\tVehicle\tLate\t12.8
S06\tWT\tVehicle\tLate\t12.2
S07\tWT\tDrugX\tEarly\t18.0
S08\tWT\tDrugX\tEarly\t19.4
S09\tWT\tDrugX\tEarly\t18.6
S10\tWT\tDrugX\tLate\t26.0
S11\tWT\tDrugX\tLate\t27.5
S12\tWT\tDrugX\tLate\t26.8
S13\tKO\tVehicle\tEarly\t11.0
S14\tKO\tVehicle\tEarly\t10.8
S15\tKO\tVehicle\tEarly\t11.4
S16\tKO\tVehicle\tLate\t11.6
S17\tKO\tVehicle\tLate\t12.1
S18\tKO\tVehicle\tLate\t11.8
S19\tKO\tDrugX\tEarly\t13.0
S20\tKO\tDrugX\tEarly\t13.6
S21\tKO\tDrugX\tEarly\t13.2
S22\tKO\tDrugX\tLate\t14.2
S23\tKO\tDrugX\tLate\t14.9
S24\tKO\tDrugX\tLate\t14.5";
    paste_tsv_at(&mut app, 0, 0, tsv)?;

    // Start with 2-way Factorial ANOVA (Genotype x Drug) on MV.
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "stats.factorial_anova")?;
    answer_fields(&mut app, json!({ "selected_data_sets": [1, 2] }))?;
    answer_ok(
        &mut app,
        json!({
            "response_column": 4,
            "three_way": false,
            "ss_type": "TypeIII",
            "multiplicity_method": "Tukey",
        }),
    )?;
    let anova_id = new_result_id_after(&app, &before)?;
    let rs2 = result_sheet(&app, &anova_id)?;
    let tab2 = tab_by_id(rs2, "factorial-anova")?;
    let _p_eta = tab_cell_f64(tab2, "Genotype × Drug", "Partial eta squared")?;
    let _eta = tab_cell_f64(tab2, "Genotype × Drug", "Eta squared")?;
    let _mc_tab = tab_by_kind(rs2, ResultTabKind::MultipleComparisons)?;

    // Reopen via Cmd+T and expand to 3-way (Genotype x Drug x Timepoint).
    switch_tab(&mut app, &anova_id)?;
    press_key(&mut app, "t", true)?;
    answer_fields(&mut app, json!({ "multifactor_columns": [1, 2, 3] }))?;
    answer_ok(
        &mut app,
        json!({
            "response_column": 4,
            "three_way": true,
            "max_interaction_order": 3,
            "compute_effect_sizes": true,
        }),
    )?;
    let rs3 = result_sheet(&app, &anova_id)?;
    let tab3 = tab_by_id(rs3, "factorial-anova")?;
    let _three_way_f = scalar_f64(rs3, "Genotype × Drug × Timepoint: F").or_else(|_| scalar_f64(rs3, "Genotype:Drug:Timepoint: F"))?;
    let _three_way_p = scalar_f64(rs3, "Genotype × Drug × Timepoint: P").or_else(|_| scalar_f64(rs3, "Genotype:Drug:Timepoint: P"))?;
    let ms_res_before = scalar_f64(rs3, "MS residual")?;
    let _three_way_eta = tab_cell_f64(tab3, "Genotype × Drug × Timepoint", "Partial eta squared")
        .or_else(|_| tab_cell_f64(tab3, "Genotype:Drug:Timepoint", "Partial eta squared"))?;

    // QOL-7 diagnostics-only enforcement.
    assert_qol7_diagnostics_only(&mut app, &anova_id)?;

    // Configure MV graph to show Drug vs Response colored by Genotype + screenshot.
    let mv_graph = graph_for_source(&app, "mv_wf33")?;
    switch_tab(&mut app, &mv_graph)?;
    menu(&mut app, "ui.graph.bubblePlot.open", json!({}))?;
    answer_ok(
        &mut app,
        json!({
            "plot_mode": "categorical_summary",
            "x_column": 2,
            "y_column": 4,
            "color_mode": "categorical",
            "color_column": 1,
        }),
    )?;
    assert_screenshot_png(&mut app, "canvas")?;

    // Live cell edit on Response recomputes MS residual.
    switch_tab(&mut app, "mv_wf33")?;
    edit_cell_at(&mut app, 9, 4, "35.0")?;
    let ms_res_after = scalar_f64(result_sheet(&app, &anova_id)?, "MS residual")?;
    if (ms_res_after - ms_res_before).abs() < 0.05 {
        return Err(format!("3-way ANOVA MS residual did not update: before={ms_res_before}, after={ms_res_after}"));
    }
    Ok(())
}

#[test]
fn wf34_qpcr_ddct_calculated_variables_and_welch_pipeline() -> TestRes {
    let mut app = new_strict_app();
    create_mv_sheet(
        &mut app,
        "mv_wf34",
        "qPCR ddCt Pipeline",
        &[
            ("SampleID", "label"),
            ("Condition", "categorical"),
            ("TargetCt", "continuous"),
            ("RefCt", "continuous"),
            ("DeltaCt", "continuous"),
            ("CalibratorMean", "continuous"),
            ("DeltaDeltaCt", "continuous"),
            ("FoldChange", "continuous"),
            ("Log2FC", "continuous"),
        ],
        8,
        true,
    )?;
    let tsv = "\
C1\tControl\t22.0\t18.0
C2\tControl\t22.2\t18.1
C3\tControl\t21.9\t17.9
C4\tControl\t22.1\t18.2
P1\tPerturbed\t19.0\t18.0
P2\tPerturbed\t19.1\t18.2
P3\tPerturbed\t18.9\t17.9
P4\tPerturbed\t19.2\t18.1";
    paste_tsv_at(&mut app, 0, 0, tsv)?;

    set_formula_via_bar(&mut app, 4, "= [TargetCt] - [RefCt]")?;
    set_formula_via_bar(&mut app, 5, "= MEANIF([Condition] == \"Control\", [DeltaCt])")?;
    set_formula_via_bar(&mut app, 6, "= [DeltaCt] - [CalibratorMean]")?;
    set_formula_via_bar(&mut app, 7, "= 2 ^ (-[DeltaDeltaCt])")?;
    set_formula_via_bar(&mut app, 8, "= -[DeltaDeltaCt]")?;

    // Control DeltaCt mean = (4.0 + 4.1 + 4.0 + 3.9)/4 = 4.0; Perturbed DeltaCt = 1.0 -> ddCt = -3.0 -> Fold = 8.0.
    assert_approx(mv_num_cell(&app, "mv_wf34", 5, 0)?, 4.0, 1e-6, "CalibratorMean")?;
    assert_approx(mv_num_cell(&app, "mv_wf34", 6, 4)?, -3.0, 1e-6, "P1 ddCt")?;
    assert_approx(mv_num_cell(&app, "mv_wf34", 7, 4)?, 8.0, 1e-5, "P1 FoldChange")?;
    assert_approx(mv_num_cell(&app, "mv_wf34", 8, 4)?, 3.0, 1e-6, "P1 Log2FC")?;

    // Direct MV Welch t-test on Log2FC vs Condition.
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "stats.two_group")?;
    answer_ok(
        &mut app,
        json!({
            "two_group_method": "welch",
            "mv_group_column": 1,
            "mv_response_column": 8,
            "compute_effect_sizes": true,
        }),
    )?;
    let welch_id = new_result_id_after(&app, &before)?;
    let rs_welch = result_sheet(&app, &welch_id)?;
    let p_before = scalar_f64(rs_welch, "PValue")?;
    let cohen_before = scalar_f64(rs_welch, "CohensD")?;
    if !(p_before < 0.001 && cohen_before.abs() > 3.0) {
        return Err(format!("unexpected qPCR Welch result: p={p_before}, d={cohen_before}"));
    }

    // Plot MV categorical summary on Condition vs Log2FC & screenshot PNG.
    let mv_graph = graph_for_source(&app, "mv_wf34")?;
    switch_tab(&mut app, &mv_graph)?;
    menu(&mut app, "ui.graph.bubblePlot.open", json!({}))?;
    answer_ok(
        &mut app,
        json!({
            "plot_mode": "categorical_summary",
            "x_column": 1,
            "y_column": 8,
            "color_mode": "categorical",
            "color_column": 1,
        }),
    )?;
    let g_insp = inspect(&mut app, "graph")?;
    if g_insp["active_sheet_id"].as_str() != Some(mv_graph.as_str()) {
        return Err(format!("qPCR graph inspect mismatch: {g_insp}"));
    }
    assert_screenshot_png(&mut app, "canvas")?;

    // Live upstream edit: alter P1 TargetCt 19.0 -> 21.0 and verify full qPCR + Welch cascade.
    switch_tab(&mut app, "mv_wf34")?;
    edit_cell_at(&mut app, 4, 2, "21.0")?;
    assert_approx(mv_num_cell(&app, "mv_wf34", 4, 4)?, 3.0, 1e-6, "P1 DeltaCt updated")?;
    assert_approx(mv_num_cell(&app, "mv_wf34", 7, 4)?, 2.0, 1e-5, "P1 FoldChange updated")?;
    let p_after = scalar_f64(result_sheet(&app, &welch_id)?, "PValue")?;
    if (p_after - p_before).abs() < 1e-8 {
        return Err(format!("qPCR Welch PValue did not cascade from TargetCt edit: before={p_before}, after={p_after}"));
    }
    Ok(())
}

#[test]
fn wf35_direct_multiple_variables_kaplan_meier_survival() -> TestRes {
    let mut app = new_strict_app();
    create_mv_sheet(
        &mut app,
        "mv_wf35",
        "Survival MV Trial",
        &[("PatientID", "label"), ("Cohort", "categorical"), ("Days", "continuous"), ("Event", "continuous")],
        18,
        true,
    )?;
    let tsv = "\
P01\tPlacebo\t10\t1
P02\tPlacebo\t14\t1
P03\tPlacebo\t18\t1
P04\tPlacebo\t22\t1
P05\tPlacebo\t26\t0
P06\tPlacebo\t30\t1
L01\tLowDose\t20\t1
L02\tLowDose\t28\t1
L03\tLowDose\t35\t0
L04\tLowDose\t42\t1
L05\tLowDose\t48\t0
L06\tLowDose\t55\t1
H01\tHighDose\t40\t1
H02\tHighDose\t56\t0
H03\tHighDose\t64\t0
H04\tHighDose\t72\t1
H05\tHighDose\t80\t0
H06\tHighDose\t90\t0";
    paste_tsv_at(&mut app, 0, 0, tsv)?;

    // Direct MV Kaplan-Meier survival analysis + Holm-Sidak pairwise.
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "stats.survival_km")?;
    let km_id = finish_analyze_dialog(
        &mut app,
        json!({
            "analyze.km.mv_time_column": 2,
            "analyze.km.mv_status_column": 3,
            "analyze.km.mv_group_column": 1,
            "analyze.km.log_rank_mantel_cox": true,
            "analyze.km.gehan_breslow_wilcoxon": true,
            "analyze.km.log_rank_for_trend": true,
            "analyze.km.multiplicity_method": "HolmSidak",
        }),
        &before,
    )?;
    let rs_km = result_sheet(&app, &km_id)?;
    let summary_tab = tab_by_id(rs_km, "km-summary")?;
    let _placebo_med = tab_cell_f64(summary_tab, "Placebo", "Median survival")?;
    let _nar_tab = tab_by_id(rs_km, "number-at-risk")?;
    let pw_tab = tab_by_id(rs_km, "survival-pairwise")?;
    if pw_tab.rows.is_empty() {
        return Err("survival-pairwise tab unexpectedly empty".to_string());
    }
    if rs_km.fitted_curves.len() < 3 {
        return Err(format!("expected >= 3 KM survival curves, got {}", rs_km.fitted_curves.len()));
    }

    // Create Survival graph bound to `mv_wf35` and add Median survival dashed Y threshold.
    menu(&mut app, "graph.create", json!({ "data_sheet_id": "mv_wf35", "sheet_id": "g_wf35_km", "kind": "survival" }))?;
    switch_tab(&mut app, "g_wf35_km")?;
    let g_insp = inspect(&mut app, "graph")?;
    let y_max = g_insp["y_axis"]["max"].as_f64().unwrap_or(100.0);
    let median_y = if y_max <= 1.5 { 0.5 } else { 50.0 };

    click(&mut app, "toolbar.axes")?;
    answer_ok(
        &mut app,
        json!({
            "y_thresholds": [{
                "value": median_y,
                "label": "Median survival",
                "dash_style": "dashed",
            }],
        }),
    )?;
    let gs_km = graph_sheet(&app, "g_wf35_km")?;
    if gs_km.spec.y_axis.thresholds.is_empty() {
        return Err("expected Median survival Y threshold on KM graph".to_string());
    }
    assert_screenshot_png(&mut app, "canvas")?;

    // Test Cmd+T parameter edit + live upstream edit reactivity.
    switch_tab(&mut app, &km_id)?;
    press_key(&mut app, "t", true)?;
    answer_ok(&mut app, json!({ "log_rank_for_trend": false }))?;
    switch_tab(&mut app, "mv_wf35")?;
    let events_before = tab_cell_f64(tab_by_id(result_sheet(&app, &km_id)?, "km-summary")?, "Placebo", "Events")?;
    edit_cell_at(&mut app, 1, 3, "0")?;
    let events_after = tab_cell_f64(tab_by_id(result_sheet(&app, &km_id)?, "km-summary")?, "Placebo", "Events")?;
    assert_approx(events_after, events_before - 1.0, 1e-6, "Placebo Events after censoring")?;
    Ok(())
}

#[test]
fn wf36_direct_multiple_variables_4pl_dose_response() -> TestRes {
    let mut app = new_strict_app();
    create_mv_sheet(
        &mut app,
        "mv_wf36",
        "Dose-Response 4PL MV",
        &[("Well", "label"), ("Compound", "categorical"), ("LogConcM", "continuous"), ("Response", "continuous")],
        16,
        true,
    )?;
    let tsv = "\
A1\tInhibitorA\t-10.0\t99.0
A2\tInhibitorA\t-9.0\t98.5
A3\tInhibitorA\t-8.0\t91.0
A4\tInhibitorA\t-7.0\t50.0
A5\tInhibitorA\t-6.0\t9.5
A6\tInhibitorA\t-5.0\t1.5
A7\tInhibitorA\t-4.0\t0.5
A8\tInhibitorA\t-3.0\t0.2
B1\tInhibitorB\t-10.0\t99.5
B2\tInhibitorB\t-9.0\t99.0
B3\tInhibitorB\t-8.0\t98.0
B4\tInhibitorB\t-7.0\t96.5
B5\tInhibitorB\t-6.0\t76.0
B6\tInhibitorB\t-5.0\t24.0
B7\tInhibitorB\t-4.0\t3.0
B8\tInhibitorB\t-3.0\t0.8";
    paste_tsv_at(&mut app, 0, 0, tsv)?;

    // Run direct MV 4PL nonlinear regression.
    let before = result_sheet_ids(&app);
    open_analyze(&mut app, "fit.nonlinear")?;
    answer_ok(
        &mut app,
        json!({
            "equation_id": "log_inhibitor_vs_response_4pl",
            "mv_x_columns": [2],
            "mv_y_column": 3,
            "mv_group_columns": [1],
        }),
    )?;
    let fit_id = new_result_id_after(&app, &before)?;

    // Reopen via Cmd+T to pin Bottom=0 and Top=100 constants, enable profile-likelihood CI & residuals.
    switch_tab(&mut app, &fit_id)?;
    press_key(&mut app, "t", true)?;
    answer_ok(
        &mut app,
        json!({
            "profile_likelihood_ci": true,
            "emit_residuals_tab": true,
            "parameter_rules": [
                {
                    "name": "Bottom",
                    "scope": { "Constant": 0.0 },
                    "initial_override": null,
                    "lower_bound": null,
                    "upper_bound": null
                },
                {
                    "name": "Top",
                    "scope": { "Constant": 100.0 },
                    "initial_override": null,
                    "lower_bound": null,
                    "upper_bound": null
                }
            ],
        }),
    )?;

    let rs_fit = result_sheet(&app, &fit_id)?;
    let params_tab = tab_by_id(rs_fit, "nonlinear_parameters")?;
    let ic50_a_before = tab_cell_f64(params_tab, "LogIC50 [DS1]", "Best-fit value")?;
    let ic50_b_before = tab_cell_f64(params_tab, "LogIC50 [DS2]", "Best-fit value")?;
    let _bot_const = tab_cell_f64(params_tab, "Bottom (Constant)", "Best-fit value")?;
    let _top_const = tab_cell_f64(params_tab, "Top (Constant)", "Best-fit value")?;
    assert_approx(ic50_a_before, -7.0, 0.25, "InhibitorA LogIC50")?;
    assert_approx(ic50_b_before, -5.5, 0.25, "InhibitorB LogIC50")?;
    let _res_tab = tab_by_id(rs_fit, "nonlinear_residuals")?;

    // QOL-7 diagnostics-only enforcement.
    assert_qol7_diagnostics_only(&mut app, &fit_id)?;

    // Graph QOL-16: InsertModelEquation (equations-only, no CLD/R2 callouts) + toggle Symbolic/Substituted.
    let fit_graph = graph_for_source(&app, "mv_wf36")?;
    switch_tab(&mut app, &fit_graph)?;
    menu(&mut app, "ui.graph.insertModelEquation", json!({}))?;
    answer_ok(
        &mut app,
        json!({
            "result_sheet_id": fit_id,
            "mode": "symbolic",
        }),
    )?;
    let gs_eq = graph_sheet(&app, &fit_graph)?;
    if gs_eq.spec.annotations.len() != 1 {
        return Err(format!("QOL-16 expected single ModelEquation annotation, got {:?}", gs_eq.spec.annotations));
    }
    match &gs_eq.spec.annotations[0] {
        GraphAnnotation::ModelEquation { mode, .. } if *mode == EquationDisplayMode::Symbolic => {}
        other => return Err(format!("expected Symbolic ModelEquation, got {other:?}")),
    }

    // Toggle via InsertModelEquation button -> Substituted.
    menu(&mut app, "ui.graph.insertModelEquation", json!({}))?;
    click(&mut app, "equation.toggle_mode")?;
    answer_ok(&mut app, json!({}))?;
    match &graph_sheet(&app, &fit_graph)?.spec.annotations[0] {
        GraphAnnotation::ModelEquation { mode, .. } if *mode == EquationDisplayMode::Substituted => {}
        other => return Err(format!("expected Substituted ModelEquation, got {other:?}")),
    }

    // Toggle back via FormatGraph -> Symbolic.
    menu(&mut app, "ui.graph.format.open", json!({}))?;
    click(&mut app, "format_graph.toggle_model_equation_mode")?;
    answer_ok(&mut app, json!({}))?;
    match &graph_sheet(&app, &fit_graph)?.spec.annotations[0] {
        GraphAnnotation::ModelEquation { mode, .. } if *mode == EquationDisplayMode::Symbolic => {}
        other => {
            return Err(format!("expected Symbolic ModelEquation after format toggle, got {other:?}"));
        }
    }
    assert_screenshot_png(&mut app, "canvas")?;

    // Create residual diagnostic graph bound to `mv_wf36` -> diag.residual.
    menu(&mut app, "graph.create", json!({ "data_sheet_id": "mv_wf36", "sheet_id": "g_wf36_resid" }))?;
    switch_tab(&mut app, "g_wf36_resid")?;
    menu(&mut app, "ui.changeGraphType.open", json!({}))?;
    answer_ok(&mut app, json!({ "selected_option_id": "diag.residual" }))?;
    let resid_insp = inspect(&mut app, "graph")?;
    if resid_insp["active_sheet_id"].as_str() != Some("g_wf36_resid") {
        return Err(format!("residual graph inspect mismatch: {resid_insp}"));
    }
    assert_screenshot_png(&mut app, "canvas")?;

    // Live upstream edit on InhibitorA Response updates LogIC50.
    switch_tab(&mut app, "mv_wf36")?;
    edit_cell_at(&mut app, 3, 3, "85.0")?;
    let ic50_a_after = tab_cell_f64(tab_by_id(result_sheet(&app, &fit_id)?, "nonlinear_parameters")?, "LogIC50 [DS1]", "Best-fit value")?;
    if (ic50_a_after - ic50_a_before).abs() < 0.05 {
        return Err(format!("4PL LogIC50 did not update reactively: before={ic50_a_before}, after={ic50_a_after}"));
    }
    Ok(())
}
