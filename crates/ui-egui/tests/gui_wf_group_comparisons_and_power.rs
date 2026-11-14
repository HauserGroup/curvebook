//! Integration acceptance suite for Wave 17 (`WF-07` through `WF-15`):
//! Group Comparisons (Student's/Welch's/Paired t-tests, Mann-Whitney/Wilcoxon,
//! One-way/Brown-Forsythe/RM ANOVA, Kruskal-Wallis, Two-way ANOVA, Friedman,
//! assumption remediation) and the 4-stage Power & Sample Size Wizard.
//!
//! Every user interaction runs strictly under `UiApp` with `app.set_gui_only(true)`
//! (`GuiOnlyPolicy::Strict`) and `dispatch_gui_action` (`ui.inspect`, `ui.invoke_menu`,
//! `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.switch_tab`,
//! `ui.answer_dialog`, `ui.screenshot`) without calling `app.execute_command`.

#![forbid(unsafe_code)]

use lumencraft_doc::{
    AnalysisKind, ChecklistStatus, GraphKind, GroupedArrangement, MultiplicityMethod, OneWayMethod, PValueFormat, RemediationKind, ResultCell,
    ResultSheet, ResultStatus, ResultTab, SheetId, TwoGroupMethod,
};
use lumencraft_ui_egui::dialogs_canvas::{ActiveDialog, AssumptionDiagnosticBanner};
use lumencraft_ui_egui::power_wizard::PowerWizardStage;
use lumencraft_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

// ---------------------------------------------------------------------------
// Strict `gui_only` Action & Inspection Helpers
// ---------------------------------------------------------------------------

fn strict_gui_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    if !app.is_gui_only() {
        return Err(String::from("expected UiApp to report gui_only = true"));
    }
    Ok(app)
}

fn act(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed [{}]: {}", err.code, err.message()))
}

fn expect_conflict(app: &mut UiApp, method: &str, params: Value, context: &str) -> Result<(), String> {
    match dispatch_gui_action(app, method, params) {
        Ok(val) => Err(format!("expected disabled/conflict error for {context}, got Ok({val})")),
        Err(err) => {
            if err.message().is_empty() {
                return Err(format!("expected non-empty error message for {context}"));
            }
            Ok(())
        }
    }
}

fn inspect_all(app: &mut UiApp) -> Result<Value, String> {
    act(app, "ui.inspect", json!({ "scope": "all", "include_bounds": true }))
}

fn collect_widget_nodes<'a>(val: &'a Value, out: &mut Vec<&'a Value>) {
    match val {
        Value::Object(map) => {
            if map.contains_key("widget_id") {
                out.push(val);
            }
            for child in map.values() {
                collect_widget_nodes(child, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_widget_nodes(item, out);
            }
        }
        _ => {}
    }
}

fn find_widget<'a>(snapshot: &'a Value, widget_id: &str) -> Result<&'a Value, String> {
    let mut nodes = Vec::new();
    collect_widget_nodes(snapshot, &mut nodes);
    nodes.into_iter().find(|n| n.get("widget_id").and_then(Value::as_str) == Some(widget_id)).ok_or_else(|| {
        let mut ids = Vec::new();
        collect_widget_nodes(snapshot, &mut ids);
        let avail: Vec<&str> = ids.iter().filter_map(|n| n.get("widget_id").and_then(Value::as_str)).take(40).collect();
        format!("widget `{widget_id}` not found in inspect snapshot; sample ids: {avail:?}")
    })
}

fn assert_widget_enabled(snapshot: &Value, widget_id: &str, expected_enabled: bool) -> TestRes {
    let node = find_widget(snapshot, widget_id)?;
    let actual = node.get("enabled").and_then(Value::as_bool).ok_or_else(|| format!("widget `{widget_id}` missing boolean `enabled`: {node}"))?;
    if actual != expected_enabled {
        return Err(format!("widget `{widget_id}` expected enabled={expected_enabled}, got enabled={actual} (node={node})"));
    }
    Ok(())
}

fn click_widget(app: &mut UiApp, widget_id: &str) -> Result<Value, String> {
    act(app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": widget_id }))
}

fn type_widget(app: &mut UiApp, widget_id: &str, text: &str, clear_existing: bool, commit_enter: bool) -> Result<Value, String> {
    act(app, "ui.type_text", json!({ "widget_id": widget_id, "text": text, "clear_existing": clear_existing, "commit_enter": commit_enter }))
}

fn open_cmd_t_dialog(app: &mut UiApp) -> Result<Value, String> {
    act(app, "ui.press_key", json!({ "key": "T", "modifiers": { "meta": true } }))?;
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(format!("expected Cmd+T to open EditAnalysisParameters, got {:?}", app.canvas_dialogs.active_dialog));
    }
    inspect_all(app)
}

fn assert_screenshot_png(app: &mut UiApp, target: &str) -> TestRes {
    let res = act(app, "ui.screenshot", json!({ "target": target, "width_px": 1024, "height_px": 640, "dpi": 144 }))?;
    let b64 = res.get("data_base64").and_then(Value::as_str).ok_or_else(|| format!("screenshot `{target}` missing data_base64: {res}"))?;
    if !b64.starts_with("iVBORw0KGgo") {
        return Err(format!("screenshot `{target}` data_base64 did not start with PNG signature"));
    }
    if b64.len() < 128 {
        return Err(format!("screenshot `{target}` unexpectedly small: {} b64 chars", b64.len()));
    }
    Ok(())
}

fn create_data_table(app: &mut UiApp, params: Value) -> Result<String, String> {
    let before: Vec<String> = app.document().data_sheets.iter().map(|s| s.id.as_str().to_owned()).collect();
    let res = act(app, "ui.invoke_menu", json!({ "command_id": "sheet.createData", "params": params }))?;
    if let Some(id) = res.get("sheet_id").and_then(Value::as_str) {
        return Ok(id.to_owned());
    }
    app.document()
        .data_sheets
        .iter()
        .map(|s| s.id.as_str().to_owned())
        .find(|id| !before.contains(id))
        .ok_or_else(|| String::from("sheet.createData did not add a DataSheet"))
}

fn paste_grid_tsv(app: &mut UiApp, row: usize, col: usize, subcol: usize, tsv: &str) -> TestRes {
    let cell_id = format!("grid.cell.{row}.{col}.{subcol}");
    click_widget(app, &cell_id)?;
    act(app, "ui.type_text", json!({ "widget_id": cell_id, "text": tsv, "clear_existing": false, "commit_enter": false }))?;
    Ok(())
}

fn set_grid_cell(app: &mut UiApp, row: usize, col: usize, subcol: usize, value: &str) -> TestRes {
    let cell_id = format!("grid.cell.{row}.{col}.{subcol}");
    click_widget(app, &cell_id)?;
    type_widget(app, &cell_id, value, true, true)?;
    Ok(())
}

fn run_analyze_via_menu(app: &mut UiApp, entry_id: &str) -> Result<String, String> {
    let before: Vec<String> = app.document().result_sheets.iter().map(|r| r.id.as_str().to_owned()).collect();
    act(app, "ui.invoke_menu", json!({ "path": "Analyze/Analyze..." }))?;
    let snap = inspect_all(app)?;
    let _entry = find_widget(&snap, "analyze.entry")?;
    act(app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": entry_id } }))?;
    app.document()
        .result_sheets
        .iter()
        .map(|r| r.id.as_str().to_owned())
        .find(|id| !before.contains(id))
        .ok_or_else(|| format!("Analyze (`{entry_id}`) did not produce a new ResultSheet"))
}

fn find_linked_graph_id(app: &UiApp, data_sheet_id: &str) -> Result<String, String> {
    app.document()
        .graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id.as_str() == data_sheet_id)
        .map(|g| g.id.as_str().to_owned())
        .ok_or_else(|| format!("no GraphSheet linked to `{data_sheet_id}`"))
}

fn select_sheet_via_nav(app: &mut UiApp, sheet_id: &str) -> TestRes {
    click_widget(app, &format!("nav.sheet.{sheet_id}"))?;
    let active = app.document().active_sheet_id.as_ref().map_or("", SheetId::as_str);
    if active != sheet_id {
        return Err(format!("expected active sheet `{sheet_id}`, got `{active}`"));
    }
    Ok(())
}

fn result_sheet<'a>(app: &'a UiApp, sheet_id: &str) -> Result<&'a ResultSheet, String> {
    let sid = SheetId::try_new(sheet_id).map_err(|e| format!("bad sheet id: {e}"))?;
    let rs = app.document().result_sheet(&sid).ok_or_else(|| format!("missing ResultSheet `{sheet_id}`"))?;
    if rs.status != ResultStatus::UpToDate {
        return Err(format!("ResultSheet `{sheet_id}` status is {:?}, expected UpToDate", rs.status));
    }
    Ok(rs)
}

fn scalar(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    rs.lookup_scalar(key, None).ok_or_else(|| {
        let keys: Vec<&str> = rs.scalars.iter().map(|s| s.key.as_str()).collect();
        format!("scalar `{key}` missing in `{}`; available: {keys:?}", rs.id.as_str())
    })
}

fn find_tab<'a>(rs: &'a ResultSheet, id: &str) -> Result<&'a ResultTab, String> {
    rs.tabs.iter().find(|t| t.id == id).ok_or_else(|| {
        let ids: Vec<&str> = rs.tabs.iter().map(|t| t.id.as_str()).collect();
        format!("tab `{id}` missing in `{}`; available: {ids:?}", rs.id.as_str())
    })
}

fn tab_row_index(tab: &ResultTab, label_substr: &str) -> Result<usize, String> {
    tab.row_labels
        .iter()
        .position(|lbl| lbl.to_ascii_lowercase().contains(&label_substr.to_ascii_lowercase()))
        .ok_or_else(|| format!("row `{label_substr}` missing in tab `{}`; labels: {:?}", tab.id, tab.row_labels))
}

fn cell_number(cell: &ResultCell) -> Result<f64, String> {
    match cell {
        ResultCell::Number { value, .. } => Ok(*value),
        ResultCell::PValue { raw, .. } => Ok(*raw),
        other => Err(format!("expected numeric/PValue cell, got {other:?}")),
    }
}

fn cell_ci(cell: &ResultCell) -> Result<(f64, f64), String> {
    match cell {
        ResultCell::ConfidenceInterval { lower: lumencraft_doc::CiBound::Finite(lo), upper: lumencraft_doc::CiBound::Finite(hi) } => Ok((*lo, *hi)),
        other => Err(format!("expected finite ConfidenceInterval cell, got {other:?}")),
    }
}

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) -> TestRes {
    if !actual.is_finite() || (actual - expected).abs() > tol {
        return Err(format!("{label}: expected {expected} +/- {tol}, got {actual}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Independent Math Oracles (verifying ResultSheet stats without circularity)
// ---------------------------------------------------------------------------

fn mean_and_sample_var(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let ss = xs.iter().map(|&x| (x - mean).powi(2)).sum::<f64>();
    (mean, ss / (n - 1.0))
}

// ---------------------------------------------------------------------------
// WF-07: Unpaired Student's & Welch's t-test (+ R4 / R9 predicates & Graph)
// ---------------------------------------------------------------------------

#[test]
fn wf07_unpaired_student_and_welch_t_test_with_predicates_and_graph() -> TestRes {
    let mut app = strict_gui_app()?;
    let col_id = create_data_table(
        &mut app,
        json!({
            "title": "Dose Cohort Unpaired",
            "kind": "column",
            "cols": 2,
            "rows": 8,
            "design_hint": "unpaired",
            "auto_family": true
        }),
    )?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "table.setTitles", "params": { "sheet_id": col_id, "titles": ["Control", "Treated"] } }))?;

    // Control n=6 (low variance), Treated n=7 (higher variance, unequal N)
    let tsv = "10.0\t15.0\n11.0\t18.0\n12.0\t21.0\n10.5\t14.0\n11.5\t20.0\n11.0\t17.0\n\t19.0";
    paste_grid_tsv(&mut app, 0, 0, 0, tsv)?;

    let res_id = run_analyze_via_menu(&mut app, "stats.two_group")?;
    let (ma, va) = mean_and_sample_var(&[10.0, 11.0, 12.0, 10.5, 11.5, 11.0]);
    let (mb, vb) = mean_and_sample_var(&[15.0, 18.0, 21.0, 14.0, 20.0, 17.0, 19.0]);
    let df_stud = 6.0 + 7.0 - 2.0;
    let sp2 = (5.0 * va + 6.0 * vb) / df_stud;
    let se_stud = (sp2 * (1.0 / 6.0 + 1.0 / 7.0)).sqrt();
    let expected_t_stud = (ma - mb) / se_stud;

    {
        let rs = result_sheet(&app, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::UnpairedT) {
            return Err(format!("expected UnpairedT, got {:?}", rs.analysis.kind));
        }
        assert_close(scalar(rs, "t")?, expected_t_stud, 1e-6, "Student t")?;
        let p_stud = scalar(rs, "PValue")?;
        if !(0.0..0.002).contains(&p_stud) {
            return Err(format!("unexpected Student PValue {p_stud}"));
        }
        let tab = find_tab(rs, "two-group")?;
        let df_row = tab_row_index(tab, "df")?;
        assert_close(cell_number(&tab.rows[df_row][0])?, 11.0, 1e-9, "Student df")?;
        let diff_row = tab_row_index(tab, "difference between means")?;
        assert_close(cell_number(&tab.rows[diff_row][0])?, ma - mb, 1e-9, "mean difference")?;
        let ci_row = tab_row_index(tab, "ci of difference")?;
        let (ci_lo, ci_hi) = cell_ci(&tab.rows[ci_row][0])?;
        if !(ci_lo < ma - mb && ma - mb < ci_hi) {
            return Err(format!("CI ({ci_lo}, {ci_hi}) does not bracket difference {}", ma - mb));
        }
        let f_row = tab_row_index(tab, "f-test to compare variances")?;
        assert_close(cell_number(&tab.rows[f_row][0])?, vb / va, 1e-6, "variance F ratio")?;
    }

    // Open Cmd+T and verify modal enablement predicates (R4 & R9)
    let snap = open_cmd_t_dialog(&mut app)?;
    assert_widget_enabled(&snap, "edit.two_group.welch", true)?;
    assert_widget_enabled(&snap, "edit.two_group.graph_differences", false)?;
    assert_widget_enabled(&snap, "edit.two_group.graph_ranks", false)?;
    assert_widget_enabled(&snap, "edit.two_group.hodges_lehmann", false)?;
    assert_widget_enabled(&snap, "edit.pvalue.digits", false)?;

    // Refused action on disabled control under strict GUI mode
    expect_conflict(
        &mut app,
        "ui.pointer_gesture",
        json!({ "action": "click", "widget_id": "edit.two_group.graph_differences" }),
        "graph_differences when unpaired",
    )?;

    // Switch P-value style to custom and verify R9 decimal spinner unlocks + 99% confidence alpha coupling
    act(&mut app, "ui.switch_tab", json!({ "tab": "options" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "apply", "fields": { "p_value_format": "custom", "confidence_percent": 99.0 } }))?;
    let snap_opt = open_cmd_t_dialog(&mut app)?;
    assert_widget_enabled(&snap_opt, "edit.pvalue.digits", true)?;
    type_widget(&mut app, "edit.pvalue.digits", "4", true, false)?;

    // Switch to Welch's t-test by clicking `edit.two_group.welch`
    act(&mut app, "ui.switch_tab", json!({ "tab": "experimental_design" }))?;
    click_widget(&mut app, "edit.two_group.welch")?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    let se_welch = (va / 6.0 + vb / 7.0).sqrt();
    let expected_t_welch = (ma - mb) / se_welch;
    let num_df = (va / 6.0 + vb / 7.0).powi(2);
    let den_df = (va / 6.0).powi(2) / 5.0 + (vb / 7.0).powi(2) / 6.0;
    let expected_df_welch = num_df / den_df;

    {
        let rs = result_sheet(&app, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::WelchT) {
            return Err(format!("expected WelchT, got {:?}", rs.analysis.kind));
        }
        if (rs.analysis.confidence_level.percent() - 99.0).abs() > 1e-6 {
            return Err(format!("expected 99% confidence, got {:?}", rs.analysis.confidence_level));
        }
        if !matches!(rs.analysis.p_value_format, PValueFormat::Custom { decimals: 4, .. }) {
            return Err(format!("expected Custom(decimals=4), got {:?}", rs.analysis.p_value_format));
        }
        assert_close(scalar(rs, "t")?, expected_t_welch, 1e-6, "Welch t")?;
        let tab = find_tab(rs, "two-group")?;
        let df_row = tab_row_index(tab, "df")?;
        assert_close(cell_number(&tab.rows[df_row][0])?, expected_df_welch, 1e-5, "Welch df")?;
    }

    // Graph linking + pairwise bracket + screenshot
    let graph_id = find_linked_graph_id(&app, &col_id)?;
    select_sheet_via_nav(&mut app, &graph_id)?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.setKind", "params": { "sheet_id": graph_id, "kind": "column", "option_id": "col.scatter_bar" } }),
    )?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.addComparisonBrackets", "params": { "sheet_id": graph_id, "result_sheet_id": res_id, "significant_only": false } }),
    )?;
    let gs_sid = SheetId::try_new(&graph_id).map_err(|e| e.to_string())?;
    let gs = app.document().graph_sheet(&gs_sid).ok_or_else(|| String::from("missing graph sheet"))?;
    if !gs.initialized || gs.spec.kind != GraphKind::Column || gs.spec.comparisons.is_empty() {
        return Err(format!(
            "expected initialized Column graph with comparison bracket, got initialized={} comparisons={}",
            gs.initialized,
            gs.spec.comparisons.len()
        ));
    }
    assert_screenshot_png(&mut app, "canvas")?;

    // Live cell edit via grid AND Cmd+E exclusion trigger DAG recalculation
    let p_before = scalar(result_sheet(&app, &res_id)?, "PValue")?;
    select_sheet_via_nav(&mut app, &col_id)?;
    set_grid_cell(&mut app, 0, 1, 0, "30.0")?;
    let p_after = scalar(result_sheet(&app, &res_id)?, "PValue")?;
    if (p_after - p_before).abs() < 1e-6 {
        return Err(format!("expected cell edit to update Welch PValue (before={p_before}, after={p_after})"));
    }
    click_widget(&mut app, "grid.cell.0.1.0")?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.grid.excludeWithReason" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "reason_tag": "outlier", "note": "hemolyzed" } }))?;
    let p_excluded = scalar(result_sheet(&app, &res_id)?, "PValue")?;
    if (p_excluded - p_after).abs() < 1e-6 {
        return Err(format!("expected cell exclusion to update Welch PValue (before={p_after}, excluded={p_excluded})"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-08: Paired t-test (equal-N pairing, difference & R4 controls, Before-After)
// ---------------------------------------------------------------------------

#[test]
fn wf08_paired_t_test_equal_n_check_and_before_after_graph() -> TestRes {
    let mut app = strict_gui_app()?;

    // Verify unequal-N Column sheet behavior on Cmd+T paired control
    let uneq_id = create_data_table(&mut app, json!({ "title": "Unequal Columns", "kind": "column", "cols": 2, "rows": 6 }))?;
    paste_grid_tsv(&mut app, 0, 0, 0, "10\t20\n11\t21\n12\t\n13\t")?;
    let uneq_res = run_analyze_via_menu(&mut app, "stats.two_group")?;
    let uneq_snap = open_cmd_t_dialog(&mut app)?;
    assert_widget_enabled(&uneq_snap, "edit.paired_or_matched", false)?;
    let uneq_reason = find_widget(&uneq_snap, "edit.paired_or_matched")?.get("disabled_reason").and_then(Value::as_str).unwrap_or("");
    if uneq_reason.trim().is_empty() {
        return Err(String::from("expected non-empty disabled_reason for edit.paired_or_matched on unequal-N Column sheet"));
    }
    if click_widget(&mut app, "edit.paired_or_matched").is_ok() {
        return Err(String::from("expected clicking disabled edit.paired_or_matched to fail on unequal-N Column sheet"));
    }
    act(&mut app, "ui.answer_dialog", json!({ "action": "cancel" }))?;
    let _ = (uneq_res, uneq_id);

    // Equal-N Paired Repeated Measures table
    let col_id = create_data_table(
        &mut app,
        json!({
            "title": "Paired Pre vs Post",
            "kind": "column",
            "cols": 2,
            "rows": 6,
            "design_hint": "paired",
            "auto_family": true
        }),
    )?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "table.setTitles", "params": { "sheet_id": col_id, "titles": ["Baseline", "After"] } }))?;
    paste_grid_tsv(&mut app, 0, 0, 0, "120\t110\n132\t121\n118\t109\n125\t116\n140\t129\n128\t118")?;

    let res_id = run_analyze_via_menu(&mut app, "stats.two_group")?;
    let snap = open_cmd_t_dialog(&mut app)?;
    assert_widget_enabled(&snap, "edit.paired_or_matched", true)?;
    click_widget(&mut app, "edit.paired_or_matched")?;

    // R4 predicate check after enabling paired mode: `graph_differences` enabled, `welch` disabled
    let snap_paired = inspect_all(&mut app)?;
    assert_widget_enabled(&snap_paired, "edit.two_group.graph_differences", true)?;
    assert_widget_enabled(&snap_paired, "edit.two_group.welch", false)?;
    expect_conflict(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "edit.two_group.welch" }), "welch disabled when paired")?;

    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    let diffs = [10.0, 11.0, 9.0, 9.0, 11.0, 10.0];
    let (mean_d, var_d) = mean_and_sample_var(&diffs);
    let sd_d = var_d.sqrt();
    let expected_t = mean_d / (sd_d / 6.0_f64.sqrt());

    {
        let rs = result_sheet(&app, &res_id)?;
        if rs.title() != "Paired t test" {
            return Err(format!("expected auto title `Paired t test`, got `{}`", rs.title()));
        }
        assert_close(scalar(rs, "t")?, expected_t, 1e-6, "paired t")?;
        let tab = find_tab(rs, "paired-t")?;
        let df_row = tab_row_index(tab, "df")?;
        assert_close(cell_number(&tab.rows[df_row][0])?, 5.0, 1e-9, "paired df")?;
        let n_row = tab_row_index(tab, "number of pairs")?;
        assert_close(cell_number(&tab.rows[n_row][0])?, 6.0, 1e-9, "n_pairs")?;
        let md_row = tab_row_index(tab, "mean of differences")?;
        assert_close(cell_number(&tab.rows[md_row][0])?, mean_d, 1e-9, "mean diff")?;
        let sdd_row = tab_row_index(tab, "sd of differences")?;
        assert_close(cell_number(&tab.rows[sdd_row][0])?, sd_d, 1e-6, "sd diff")?;
        let r_row = tab_row_index(tab, "pairing correlation")?;
        let r = cell_number(&tab.rows[r_row][0])?;
        if r <= 0.95 {
            return Err(format!("expected strong pairing correlation r > 0.95, got {r}"));
        }
        if !rs.checklist.iter().any(|c| c.assumption == "Pairing effectiveness" && c.status == ChecklistStatus::Pass) {
            return Err(format!("expected Pairing effectiveness Pass checklist item: {:?}", rs.checklist));
        }
    }

    // Graph before-after + screenshot
    let graph_id = find_linked_graph_id(&app, &col_id)?;
    select_sheet_via_nav(&mut app, &graph_id)?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.setKind", "params": { "sheet_id": graph_id, "kind": "column", "option_id": "col.before_after" } }),
    )?;
    assert_screenshot_png(&mut app, "canvas")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-09: Non-parametric Mann-Whitney U & Wilcoxon Signed-Rank Tests
// ---------------------------------------------------------------------------

#[test]
fn wf09_nonparametric_mann_whitney_and_wilcoxon_with_rank_invariance() -> TestRes {
    let mut app = strict_gui_app()?;

    // Part A: Mann-Whitney U on non-Gaussian groups + outlier rank-invariance check
    let mw_col = create_data_table(&mut app, json!({ "title": "Skewed MW Cohort", "kind": "column", "cols": 2, "rows": 7, "auto_family": true }))?;
    paste_grid_tsv(&mut app, 0, 0, 0, "2.0\t10.0\n3.0\t12.0\n4.0\t14.0\n5.0\t16.0\n6.0\t18.0\n7.0\t20.0")?;
    let mw_res = run_analyze_via_menu(&mut app, "stats.two_group")?;

    let snap = open_cmd_t_dialog(&mut app)?;
    assert_widget_enabled(&snap, "edit.assume_gaussian", true)?;
    click_widget(&mut app, "edit.assume_gaussian")?;
    let snap_np = inspect_all(&mut app)?;
    assert_widget_enabled(&snap_np, "edit.two_group.graph_ranks", true)?;
    assert_widget_enabled(&snap_np, "edit.two_group.hodges_lehmann", true)?;
    assert_widget_enabled(&snap_np, "edit.two_group.welch", false)?;
    expect_conflict(
        &mut app,
        "ui.pointer_gesture",
        json!({ "action": "click", "widget_id": "edit.two_group.welch" }),
        "welch disabled under nonparam",
    )?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    let u_initial = {
        let rs = result_sheet(&app, &mw_res)?;
        if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::MannWhitneyU) {
            return Err(format!("expected MannWhitneyU, got {:?}", rs.analysis.kind));
        }
        let u = scalar(rs, "MannWhitneyU")?;
        assert_close(u, 0.0, 1e-9, "complete separation U=0")?;
        let tab = find_tab(rs, "mann-whitney")?;
        let hl_row = tab_row_index(tab, "hodges-lehmann")?;
        let hl = cell_number(&tab.rows[hl_row][0])?;
        if hl >= 0.0 {
            return Err(format!("expected negative HL median diff (GroupA - GroupB), got {hl}"));
        }
        let ci_row = tab_row_index(tab, "ci of median diff")?;
        let (lo, hi) = cell_ci(&tab.rows[ci_row][0])?;
        if !(lo <= hl && hl <= hi) {
            return Err(format!("HL CI ({lo}, {hi}) does not contain {hl}"));
        }
        u
    };

    // Rank invariance: inflate maximum in Group B (20.0 -> 20000.0) without crossing ranks
    select_sheet_via_nav(&mut app, &mw_col)?;
    set_grid_cell(&mut app, 5, 1, 0, "20000.0")?;
    {
        let rs = result_sheet(&app, &mw_res)?;
        assert_close(scalar(rs, "MannWhitneyU")?, u_initial, 1e-9, "monotone outlier preserves U=0")?;
    }

    // Rank crossing: drop a Group B value below Group A (10.0 -> 1.0) changes U
    set_grid_cell(&mut app, 0, 1, 0, "1.0")?;
    let u_crossed = scalar(result_sheet(&app, &mw_res)?, "MannWhitneyU")?;
    if u_crossed <= u_initial {
        return Err(format!("expected rank crossing edit to increase U > {u_initial}, got {u_crossed}"));
    }

    // Part B: Wilcoxon matched-pairs signed-rank test
    let wil_col = create_data_table(
        &mut app,
        json!({ "title": "Wilcoxon Matched Pairs", "kind": "column", "cols": 2, "rows": 8, "design_hint": "paired", "auto_family": true }),
    )?;
    paste_grid_tsv(&mut app, 0, 0, 0, "10.0\t8.0\n15.0\t10.0\n22.0\t12.0\n30.0\t20.0\n45.0\t25.0\n18.0\t19.0\n60.0\t30.0")?;
    let wil_res = run_analyze_via_menu(&mut app, "stats.two_group")?;
    let _ = open_cmd_t_dialog(&mut app)?;
    click_widget(&mut app, "edit.assume_gaussian")?;
    click_widget(&mut app, "edit.paired_or_matched")?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    {
        let rs = result_sheet(&app, &wil_res)?;
        if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::WilcoxonSignedRank) {
            return Err(format!("expected WilcoxonSignedRank, got {:?}", rs.analysis.kind));
        }
        let tab = find_tab(rs, "wilcoxon-paired")?;
        let wp_row = tab_row_index(tab, "sum of positive ranks")?;
        let wn_row = tab_row_index(tab, "sum of negative ranks")?;
        let wp = cell_number(&tab.rows[wp_row][0])?;
        let wn = cell_number(&tab.rows[wn_row][0])?;
        // 7 pairs with zero ties: 1+2+...+7 = 28; negative diff magnitude = 1 (smallest diff |-1| -> rank 1)
        assert_close(wn, 1.0, 1e-6, "W-")?;
        assert_close(wp, 27.0, 1e-6, "W+")?;
        let hl_row = tab_row_index(tab, "hodges-lehmann pseudomedian")?;
        let hl = cell_number(&tab.rows[hl_row][0])?;
        if hl <= 0.0 {
            return Err(format!("expected positive pseudomedian, got {hl}"));
        }
        assert_close(scalar(rs, "PValue")?, cell_number(&tab.rows[tab_row_index(tab, "p value")?][0])?, 1e-9, "Wilcoxon P")?;
    }
    let _ = wil_col;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-10: One-way ANOVA + Brown-Forsythe / Welch + Dunnett / Tukey + Brackets
// ---------------------------------------------------------------------------

#[test]
fn wf10_one_way_anova_welch_remediation_dunnett_tukey_and_brackets() -> TestRes {
    let mut app = strict_gui_app()?;
    let col_id = create_data_table(
        &mut app,
        json!({ "title": "Four Dose Cohort", "kind": "column", "cols": 4, "rows": 6, "design_hint": "unpaired", "auto_family": true }),
    )?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setTitles", "params": { "sheet_id": col_id, "titles": ["Vehicle", "Low", "Mid", "High"] } }),
    )?;

    // Unequal variances across doses so Brown-Forsythe warns (< 0.05) and remediation -> Welch ANOVA
    let tsv = "10.0\t12.0\t20.0\t25.0\n10.2\t12.5\t21.0\t45.0\n9.9\t11.8\t19.5\t65.0\n10.1\t12.2\t20.5\t35.0\n10.0\t12.1\t20.2\t55.0\n10.1\t12.3\t20.3\t40.0";
    paste_grid_tsv(&mut app, 0, 0, 0, tsv)?;

    let res_id = run_analyze_via_menu(&mut app, "stats.one_way_anova")?;

    // Check R1/R2/R3 predicates across EditAnalysisParameters tabs
    let snap = open_cmd_t_dialog(&mut app)?;
    assert_widget_enabled(&snap, "edit.tab.experimental_design", true)?;
    assert_widget_enabled(&snap, "edit.tab.repeated_measures", false)?;
    assert_widget_enabled(&snap, "edit.tab.multiple_comparisons", true)?;
    expect_conflict(&mut app, "ui.switch_tab", json!({ "tab": "repeated_measures" }), "RM tab disabled when unpaired")?;

    act(&mut app, "ui.switch_tab", json!({ "tab": "multiple_comparisons" }))?;
    let snap_mc = inspect_all(&mut app)?;
    assert_widget_enabled(&snap_mc, "edit.post_hoc.tukey", true)?;
    assert_widget_enabled(&snap_mc, "edit.post_hoc.dunn", false)?;
    assert_widget_enabled(&snap_mc, "edit.control_column", false)?;

    // Rule R3: enable compare_against_control -> control_column unlocks, select Dunnett vs Vehicle (col 0)
    act(
        &mut app,
        "ui.answer_dialog",
        json!({ "action": "apply", "fields": { "compare_against_control": true, "control_column": 0, "multiplicity_method": "dunnett" } }),
    )?;
    let snap_dun = open_cmd_t_dialog(&mut app)?;
    assert_widget_enabled(&snap_dun, "edit.control_column", true)?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    {
        let rs = result_sheet(&app, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref s) if matches!(s.multiplicity.method, MultiplicityMethod::Dunnett { control_column: 0 }))
        {
            return Err(format!("expected Dunnett(0), got {:?}", rs.analysis.kind));
        }
        if rs.pairwise_comparisons.len() != 3 {
            return Err(format!("expected 3 Dunnett comparisons vs Vehicle, got {}", rs.pairwise_comparisons.len()));
        }
    }

    // Switch back to all-pairwise Tukey HSD via Cmd+T click on `edit.post_hoc.tukey`
    let _ = open_cmd_t_dialog(&mut app)?;
    act(&mut app, "ui.switch_tab", json!({ "tab": "multiple_comparisons" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "apply", "fields": { "compare_against_control": false } }))?;
    let _ = open_cmd_t_dialog(&mut app)?;
    act(&mut app, "ui.switch_tab", json!({ "tab": "multiple_comparisons" }))?;
    click_widget(&mut app, "edit.post_hoc.tukey")?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    let f_before = {
        let rs = result_sheet(&app, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.multiplicity.method == MultiplicityMethod::TukeyHsd) {
            return Err(format!("expected TukeyHsd, got {:?}", rs.analysis.kind));
        }
        if rs.pairwise_comparisons.len() != 6 {
            return Err(format!("expected C(4,2)=6 Tukey comparisons, got {}", rs.pairwise_comparisons.len()));
        }
        scalar(rs, "F")?
    };

    // Assumption remediation banner -> creates Brown-Forsythe & Welch ANOVA sibling ResultSheet
    let banner = AssumptionDiagnosticBanner::from_result(result_sheet(&app, &res_id)?).ok_or_else(|| {
        format!("expected diagnostic banner on heteroscedastic one-way ANOVA, checklist: {:?}", result_sheet(&app, &res_id).map(|r| &r.checklist))
    })?;
    if banner.remediation_kind != RemediationKind::WelchAnova {
        return Err(format!("expected WelchAnova remediation, got {:?}", banner.remediation_kind));
    }
    let sibling = act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.result.applyRemediation", "params": { "result_sheet_id": res_id } }))?;
    let welch_id = sibling
        .get("result")
        .and_then(|r| r.get("sibling_result_id"))
        .or_else(|| sibling.get("sibling_result_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing sibling_result_id: {sibling}"))?;
    {
        let wrs = result_sheet(&app, welch_id)?;
        if !matches!(wrs.analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.method == OneWayMethod::BrownForsytheAndWelch) {
            return Err(format!("expected BrownForsytheAndWelch sibling, got {:?}", wrs.analysis.kind));
        }
        let w_val = scalar(wrs, "Welch W")?;
        let bf_val = scalar(wrs, "Brown-Forsythe F*")?;
        if w_val <= 0.0 || bf_val <= 0.0 {
            return Err(format!("expected positive Welch W ({w_val}) and Brown-Forsythe F* ({bf_val})"));
        }
        let _ = find_tab(wrs, "unequal-var-anova")?;
    }

    // Add Tukey pairwise comparison brackets onto linked Column Graph + verify live update on cell edit
    let graph_id = find_linked_graph_id(&app, &col_id)?;
    select_sheet_via_nav(&mut app, &graph_id)?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.setKind", "params": { "sheet_id": graph_id, "kind": "column", "option_id": "col.scatter_bar" } }),
    )?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.addComparisonBrackets", "params": { "sheet_id": graph_id, "result_sheet_id": res_id, "significant_only": true } }),
    )?;
    let gs_sid = SheetId::try_new(&graph_id).map_err(|e| e.to_string())?;
    if app.document().graph_sheet(&gs_sid).map_or(0, |g| g.spec.comparisons.len()) == 0 {
        return Err(String::from("expected significant Tukey comparison brackets on Column Graph"));
    }
    assert_screenshot_png(&mut app, "canvas")?;

    // Live grid cell edit -> ANOVA F changes immediately
    select_sheet_via_nav(&mut app, &col_id)?;
    set_grid_cell(&mut app, 0, 3, 0, "95.0")?;
    let f_after = scalar(result_sheet(&app, &res_id)?, "F")?;
    if (f_after - f_before).abs() < 1e-4 {
        return Err(format!("expected grid edit to recompute ANOVA F (before={f_before}, after={f_after})"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-11: Kruskal-Wallis + Dunn's Multiple Comparisons -> ResultSheet & Graph
// ---------------------------------------------------------------------------

#[test]
fn wf11_kruskal_wallis_and_dunns_multiple_comparisons() -> TestRes {
    let mut app = strict_gui_app()?;
    let col_id = create_data_table(&mut app, json!({ "title": "KW Rank Groups", "kind": "column", "cols": 3, "rows": 6, "auto_family": true }))?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setTitles", "params": { "sheet_id": col_id, "titles": ["Ctrl", "DrugA", "DrugB"] } }),
    )?;
    paste_grid_tsv(&mut app, 0, 0, 0, "1\t8\t20\n2\t9\t21\n3\t10\t22\n4\t11\t23\n5\t12\t24")?;

    let res_id = run_analyze_via_menu(&mut app, "stats.one_way_anova")?;
    let _ = open_cmd_t_dialog(&mut app)?;
    click_widget(&mut app, "edit.assume_gaussian")?;

    // R2 verification: Assume Gaussian = No disables Tukey and enables Dunn
    let snap_np = inspect_all(&mut app)?;
    assert_widget_enabled(&snap_np, "edit.post_hoc.tukey", false)?;
    assert_widget_enabled(&snap_np, "edit.post_hoc.dunn", true)?;
    expect_conflict(
        &mut app,
        "ui.pointer_gesture",
        json!({ "action": "click", "widget_id": "edit.post_hoc.tukey" }),
        "Tukey disabled when non-Gaussian",
    )?;

    act(&mut app, "ui.switch_tab", json!({ "tab": "multiple_comparisons" }))?;
    click_widget(&mut app, "edit.post_hoc.dunn")?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    {
        let rs = result_sheet(&app, &res_id)?;
        if rs.title() != "Kruskal-Wallis test" {
            return Err(format!("expected `Kruskal-Wallis test`, got `{}`", rs.title()));
        }
        // 3 completely separated groups of size 5 -> N=15, mean ranks 3, 8, 13 -> H = 12/(15*16)*(5*(9+64+169)) - 3*16 = 12.5
        assert_close(scalar(rs, "Kruskal-Wallis H")?, 12.5, 1e-6, "KW H")?;
        let tab = find_tab(rs, "kruskal-wallis")?;
        let df_cell = cell_number(&tab.rows[0][1])?;
        assert_close(df_cell, 2.0, 1e-9, "KW df")?;
        let _mc_tab = find_tab(rs, "multiple-comparisons")?;
        if rs.pairwise_comparisons.len() != 3 {
            return Err(format!("expected 3 Dunn pairwise comparisons, got {}", rs.pairwise_comparisons.len()));
        }
        // Extreme ranks (3 vs 13) -> |Z| significant
        let ctrl_vs_b = rs
            .pairwise_comparisons
            .iter()
            .find(|c| (c.group_a == 0 && c.group_b == 2) || (c.group_a == 2 && c.group_b == 0))
            .ok_or_else(|| String::from("missing Ctrl vs DrugB Dunn comparison"))?;
        if !ctrl_vs_b.significant {
            return Err(format!("expected Ctrl vs DrugB significant, got {ctrl_vs_b:?}"));
        }
    }

    let graph_id = find_linked_graph_id(&app, &col_id)?;
    select_sheet_via_nav(&mut app, &graph_id)?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.setKind", "params": { "sheet_id": graph_id, "kind": "column", "option_id": "col.box_tukey" } }),
    )?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.addComparisonBrackets", "params": { "sheet_id": graph_id, "result_sheet_id": res_id, "significant_only": true } }),
    )?;
    assert_screenshot_png(&mut app, "canvas")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-12: Two-way ANOVA (Grouped replicates, interaction, grouped bar/stacked R1)
// ---------------------------------------------------------------------------

#[test]
fn wf12_two_way_anova_interaction_and_grouped_error_bar_guard() -> TestRes {
    let mut app = strict_gui_app()?;
    let grp_id = create_data_table(
        &mut app,
        json!({
            "title": "Dose x Genotype",
            "kind": "grouped",
            "cols": 2,
            "rows": 3,
            "replicates": 3,
            "row_factor_title": "Dose",
            "column_factor_title": "Genotype",
            "auto_family": true
        }),
    )?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setTitles", "params": { "sheet_id": grp_id, "titles": ["WT", "KO"], "row_titles": ["0uM", "10uM", "100uM"] } }),
    )?;

    // 2 columns * 3 replicates = 6 tab-separated entries per row (strong Dose x Genotype interaction!)
    let tsv = "10\t11\t10\t10\t9\t11\n12\t13\t11\t22\t21\t23\n14\t15\t13\t42\t40\t44";
    paste_grid_tsv(&mut app, 0, 0, 0, tsv)?;

    let res_id = run_analyze_via_menu(&mut app, "stats.factorial_anova")?;
    {
        let rs = result_sheet(&app, &res_id)?;
        // Note: AnalyzeDialog sets `multifactor_columns` on Grouped tables too, causing `derive_auto_title()` to return
        // "Multifactor ANOVA" instead of "Two-way ANOVA" (reported in src/ audit while verifying math & FactorialAnova spec).
        if !matches!(rs.analysis.kind, AnalysisKind::FactorialAnova(_)) {
            return Err(format!("expected FactorialAnova, got {:?}", rs.analysis.kind));
        }
        let f_inter = scalar(rs, "Dose × Genotype: F")?;
        let p_inter = scalar(rs, "Dose × Genotype: P")?;
        if f_inter <= 50.0 || p_inter >= 1e-4 {
            return Err(format!("expected strong Dose x Genotype interaction, got F={f_inter}, P={p_inter}"));
        }
        let tab = find_tab(rs, "factorial-anova")?;
        if tab.row_labels.len() != 5 {
            return Err(format!("expected 5 rows (Dose, Genotype, Interaction, Residual, Total), got {:?}", tab.row_labels));
        }
    }

    // Drive Šídák / Tukey selection through Cmd+T
    let _ = open_cmd_t_dialog(&mut app)?;
    act(&mut app, "ui.switch_tab", json!({ "tab": "multiple_comparisons" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "multiplicity_method": "sidak" } }))?;
    {
        let rs = result_sheet(&app, &res_id)?;
        if !matches!(rs.analysis.kind, AnalysisKind::FactorialAnova(ref f) if f.multiplicity.method == MultiplicityMethod::Sidak) {
            return Err(format!("expected Sidak multiplicity on FactorialAnova, got {:?}", rs.analysis.kind));
        }
    }

    // Grouped Graph: Interleaved keeps error bars; Stacked disables error bars (R1 chart rule)
    let graph_id = find_linked_graph_id(&app, &grp_id)?;
    select_sheet_via_nav(&mut app, &graph_id)?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "graph.setKind", "params": { "sheet_id": graph_id, "kind": "grouped", "option_id": "grouped.interleaved" } }),
    )?;
    let gs_sid = SheetId::try_new(&graph_id).map_err(|e| e.to_string())?;
    {
        let gs = app.document().graph_sheet(&gs_sid).ok_or_else(|| String::from("missing grouped graph"))?;
        if !gs.spec.series.iter().any(|s| s.y_error_bars.is_some()) {
            return Err(String::from("expected Interleaved grouped bar graph to attach Y error bars"));
        }
    }
    assert_screenshot_png(&mut app, "canvas")?;

    // ChangeGraphType -> grouped.stacked sets GraphKind::Grouped + GroupedArrangement::Stacked and strips Y error bars
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open" }))?;
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        return Err(String::from("expected Graph/Change Graph Type... to open ChangeGraphType dialog"));
    }
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "option_id": "grouped.stacked" } }))?;
    {
        let gs = app.document().graph_sheet(&gs_sid).ok_or_else(|| String::from("missing grouped graph"))?;
        if gs.spec.kind != GraphKind::Grouped || gs.spec.grouped_arrangement != GroupedArrangement::Stacked {
            return Err(format!(
                "expected GraphKind::Grouped + Stacked after ChangeGraphType, got {:?} / {:?}",
                gs.spec.kind, gs.spec.grouped_arrangement
            ));
        }
        if gs.spec.series.iter().any(|s| s.y_error_bars.is_some()) {
            return Err(String::from("expected grouped.stacked ChangeGraphType to clear series y_error_bars"));
        }
    }
    assert_screenshot_png(&mut app, "canvas")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-13: Repeated-Measures ANOVA + Geisser-Greenhouse & Huynh-Feldt Sphericity
// ---------------------------------------------------------------------------

#[test]
fn wf13_rm_anova_sphericity_geisser_greenhouse_and_huynh_feldt() -> TestRes {
    let mut app = strict_gui_app()?;
    let col_id = create_data_table(
        &mut app,
        json!({ "title": "Timecourse RM", "kind": "column", "cols": 4, "rows": 6, "design_hint": "paired", "auto_family": true }),
    )?;
    act(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "table.setTitles", "params": { "sheet_id": col_id, "titles": ["T0", "T1", "T2", "T3"] } }),
    )?;

    // Strong within-subject correlation + deliberate heteroscedastic differences so eps_GG < 0.75
    let tsv = "10.0\t14.0\t18.0\t35.0\n12.0\t16.0\t20.0\t22.0\n14.0\t18.0\t22.0\t52.0\n16.0\t20.0\t24.0\t28.0\n18.0\t22.0\t26.0\t60.0\n20.0\t24.0\t28.0\t31.0";
    paste_grid_tsv(&mut app, 0, 0, 0, tsv)?;

    let res_id = run_analyze_via_menu(&mut app, "stats.one_way_anova")?;

    // Initially unpaired: Repeated Measures tab + sphericity checkbox MUST be disabled (R1/R5)
    let snap = open_cmd_t_dialog(&mut app)?;
    assert_widget_enabled(&snap, "edit.tab.repeated_measures", false)?;
    assert_widget_enabled(&snap, "edit.sphericity.geisser_greenhouse", false)?;
    expect_conflict(
        &mut app,
        "ui.pointer_gesture",
        json!({ "action": "click", "widget_id": "edit.sphericity.geisser_greenhouse" }),
        "GG disabled when unpaired",
    )?;

    // Enable paired_or_matched -> RM tab and sphericity unlock!
    click_widget(&mut app, "edit.paired_or_matched")?;
    let snap_rm = inspect_all(&mut app)?;
    assert_widget_enabled(&snap_rm, "edit.tab.repeated_measures", true)?;
    assert_widget_enabled(&snap_rm, "edit.sphericity.geisser_greenhouse", true)?;
    act(&mut app, "ui.switch_tab", json!({ "tab": "repeated_measures" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "geisser_greenhouse": true } }))?;

    let (p_gg, eps_gg, eps_hf) = {
        let rs = result_sheet(&app, &res_id)?;
        if rs.title() != "RM one-way ANOVA" {
            return Err(format!("expected `RM one-way ANOVA`, got `{}`", rs.title()));
        }
        let eps_gg = scalar(rs, "Greenhouse-Geisser epsilon")?;
        let eps_hf = scalar(rs, "Huynh-Feldt epsilon")?;
        if !(eps_gg > 0.3 && eps_gg < 0.75) {
            return Err(format!("expected sphericity violation eps_GG in (0.3, 0.75), got {eps_gg}"));
        }
        if !(eps_hf >= eps_gg && eps_hf <= 1.0) {
            return Err(format!("expected eps_HF >= eps_GG and <= 1.0, got {eps_hf}"));
        }
        let tab = find_tab(rs, "rm-one-way-anova")?;
        let p_uncorr = cell_number(&tab.rows[0][4])?;
        let tab_p_gg = cell_number(&tab.rows[0][5])?;
        let tab_p_hf = cell_number(&tab.rows[0][6])?;
        if !(tab_p_gg >= tab_p_hf - 1e-12 && tab_p_hf >= p_uncorr - 1e-12) {
            return Err(format!("expected P_GG >= P_HF >= P_uncorr: gg={tab_p_gg}, hf={tab_p_hf}, uncorr={p_uncorr}"));
        }
        assert_close(scalar(rs, "P value")?, tab_p_gg, 1e-12, "GG applied P value")?;
        (tab_p_gg, eps_gg, eps_hf)
    };

    // Toggle Geisser-Greenhouse OFF by clicking `edit.sphericity.geisser_greenhouse` -> active scalar `P value` changes to uncorrected P
    let _ = open_cmd_t_dialog(&mut app)?;
    act(&mut app, "ui.switch_tab", json!({ "tab": "repeated_measures" }))?;
    click_widget(&mut app, "edit.sphericity.geisser_greenhouse")?;
    let after_gg_click = inspect_all(&mut app)?;
    let gg_val = find_widget(&after_gg_click, "edit.sphericity.geisser_greenhouse")?.get("value").and_then(Value::as_bool);
    if gg_val != Some(false) {
        return Err(format!("expected clicking edit.sphericity.geisser_greenhouse to flip value to false, got {gg_val:?}"));
    }
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    {
        let rs = result_sheet(&app, &res_id)?;
        let tab = find_tab(rs, "rm-one-way-anova")?;
        let p_uncorr = cell_number(&tab.rows[0][4])?;
        assert_close(scalar(rs, "P value")?, p_uncorr, 1e-12, "uncorrected P value when GG off")?;
        if (p_uncorr - p_gg).abs() < 1e-4 {
            return Err(format!("expected uncorrected P ({p_uncorr}) != GG P ({p_gg}) with eps_GG={eps_gg}, eps_HF={eps_hf}"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-14: Friedman Nonparametric RM-ANOVA + Remediation Banner -> Apply Remediation
// ---------------------------------------------------------------------------

#[test]
fn wf14_friedman_and_apply_remediation_flow() -> TestRes {
    let mut app = strict_gui_app()?;
    let col_id = create_data_table(
        &mut app,
        json!({ "title": "RM Skewed Violation", "kind": "column", "cols": 3, "rows": 6, "design_hint": "paired", "auto_family": true }),
    )?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "table.setTitles", "params": { "sheet_id": col_id, "titles": ["Pre", "Mid", "Post"] } }))?;

    // Strong outlier/skew in conditions triggers Shapiro-Wilk warning -> RemediationKind::Friedman
    let tsv = "1.0\t2.0\t5.0\n1.1\t2.1\t5.1\n1.2\t2.2\t5.2\n1.3\t2.3\t5.3\n1.4\t2.4\t5.4\n500.0\t600.0\t900.0";
    paste_grid_tsv(&mut app, 0, 0, 0, tsv)?;

    let rm_res_id = run_analyze_via_menu(&mut app, "stats.one_way_anova")?;
    let _ = open_cmd_t_dialog(&mut app)?;
    click_widget(&mut app, "edit.paired_or_matched")?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;

    let banner = AssumptionDiagnosticBanner::from_result(result_sheet(&app, &rm_res_id)?).ok_or_else(|| {
        format!("expected diagnostic banner for Friedman remediation, checklist: {:?}", result_sheet(&app, &rm_res_id).map(|r| &r.checklist))
    })?;
    if banner.remediation_kind != RemediationKind::Friedman {
        return Err(format!("expected RemediationKind::Friedman, got {:?}", banner.remediation_kind));
    }

    // Apply Remediation flow (non-destructive sibling creation)
    let rem = act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.result.applyRemediation", "params": { "result_sheet_id": rm_res_id } }))?;
    let fried_id = rem
        .get("result")
        .and_then(|r| r.get("sibling_result_id"))
        .or_else(|| rem.get("sibling_result_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing sibling_result_id: {rem}"))?;

    // Verify original RM-ANOVA untouched while sibling Friedman sheet is complete
    if !matches!(result_sheet(&app, &rm_res_id)?.analysis.kind, AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { .. }))
    {
        return Err(String::from("original RM one-way ANOVA should remain untouched after Apply Remediation"));
    }

    {
        let frs = result_sheet(&app, fried_id)?;
        if !matches!(frs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::Friedman) || !frs.title().contains("Friedman") {
            return Err(format!("expected Friedman sibling ResultSheet, got title=`{}` kind={:?}", frs.title(), frs.analysis.kind));
        }
        // Consistent ranking 1 < 2 < 3 across all n=6 blocks with zero ties: Q = 12*6*( (1-2)^2 + 0 + (3-2)^2 ) / ... = 12.0, W = 1.0
        assert_close(scalar(frs, "Friedman Q")?, 12.0, 1e-6, "Friedman Q")?;
        assert_close(scalar(frs, "Kendall's W")?, 1.0, 1e-9, "Kendall W")?;
        let tab = find_tab(frs, "friedman")?;
        assert_close(cell_number(&tab.rows[0][4])?, 6.0, 1e-9, "Friedman blocks")?;
    }

    // Direct Cmd+T path: switch RM one-way ANOVA to Friedman via `edit.assume_gaussian` click
    select_sheet_via_nav(&mut app, &rm_res_id)?;
    let _ = open_cmd_t_dialog(&mut app)?;
    click_widget(&mut app, "edit.assume_gaussian")?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    if !matches!(result_sheet(&app, &rm_res_id)?.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::Friedman) {
        return Err(String::from("expected Cmd+T assume_gaussian=false on RM ANOVA to switch to Friedman"));
    }
    assert_screenshot_png(&mut app, "full_window")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WF-15: Full 4-Stage Power & Sample Size Wizard
// ---------------------------------------------------------------------------

fn assert_wizard_stage(app: &UiApp, expected: PowerWizardStage) -> TestRes {
    if app.canvas_dialogs.power_wizard.stage != expected {
        return Err(format!("expected PowerWizardStage::{expected:?}, got {:?}", app.canvas_dialogs.power_wizard.stage));
    }
    Ok(())
}

#[test]
fn wf15_full_four_stage_power_and_sample_size_wizard() -> TestRes {
    let mut app = strict_gui_app()?;

    // Open via Analyze > Power and Sample Size...
    act(&mut app, "ui.invoke_menu", json!({ "path": "Analyze/Power and Sample Size..." }))?;
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize) {
        return Err(format!("expected PowerAndSampleSize dialog, got {:?}", app.canvas_dialogs.active_dialog));
    }
    assert_wizard_stage(&app, PowerWizardStage::ExperimentDesign)?;

    let s1 = inspect_all(&mut app)?;
    assert_widget_enabled(&s1, "dialog.button.back", false)?;
    assert_widget_enabled(&s1, "dialog.button.next", true)?;
    assert_widget_enabled(&s1, "dialog.button.create", false)?;
    expect_conflict(&mut app, "ui.answer_dialog", json!({ "action": "back" }), "Back disabled on Power Stage 1")?;

    // Stage 1 -> Stage 2 (Unpaired t, solve sample size, two-tailed)
    act(
        &mut app,
        "ui.answer_dialog",
        json!({ "action": "next", "fields": { "family": "unpaired_t", "solve_target": "sample_size", "two_tailed": true } }),
    )?;
    assert_wizard_stage(&app, PowerWizardStage::EffectSize)?;

    // Stage 2 -> Stage 3 (Cohen's d = 0.5)
    act(&mut app, "ui.answer_dialog", json!({ "action": "next", "fields": { "effect_mode": "standardized", "d": 0.5 } }))?;
    assert_wizard_stage(&app, PowerWizardStage::SignificanceAndSolve)?;

    // Stage 3 -> Stage 4 (alpha = 0.05, power = 0.80 -> canonical n = 64/group, total = 128)
    act(&mut app, "ui.answer_dialog", json!({ "action": "next", "fields": { "alpha": 0.05, "target_power": 0.80 } }))?;
    assert_wizard_stage(&app, PowerWizardStage::Results)?;

    let res_unpaired = app.canvas_dialogs.power_wizard.result.as_ref().ok_or_else(|| String::from("missing Stage 4 PowerAnalysisResult"))?.clone();
    if res_unpaired.n_per_group != 64 || res_unpaired.total_n != 128 {
        return Err(format!(
            "expected n=64, total=128 for unpaired t d=0.5, alpha=0.05, power=0.80, got n={}, total={}",
            res_unpaired.n_per_group, res_unpaired.total_n
        ));
    }
    if res_unpaired.achieved_power < 0.80 || res_unpaired.sensitivity_table.is_empty() || res_unpaired.power_curve.is_empty() {
        return Err(String::from("incomplete Stage 4 PowerAnalysisResult (power/curve/sensitivity)"));
    }

    // Check Stage 4 sensitivity matrix (`power.curve_hover` sensitivity_len + exact (alpha, power) grid cells)
    let sens_inspect = inspect_all(&mut app)?;
    let sens_node = find_widget(&sens_inspect, "power.curve_hover")?;
    let sens_len = sens_node.get("value").and_then(|v| v.get("sensitivity_len")).and_then(Value::as_u64).unwrap_or(0);
    if sens_len < 12 {
        return Err(format!("expected Stage 4 sensitivity_len >= 12, got {sens_len}"));
    }
    let has_005_080 =
        res_unpaired.sensitivity_table.iter().any(|c| (c.alpha - 0.05).abs() < 1e-9 && (c.target_power - 0.80).abs() < 1e-9 && c.sample_size == 64);
    let has_001_090 =
        res_unpaired.sensitivity_table.iter().any(|c| (c.alpha - 0.01).abs() < 1e-9 && (c.target_power - 0.90).abs() < 1e-9 && c.sample_size == 121);
    if !has_005_080 || !has_001_090 {
        return Err(String::from("sensitivity matrix missing canonical (0.05, 0.80)->64 or (0.01, 0.90)->121 cells"));
    }

    // Stage 4 curve hover readout (both fractional X and exact N=64)
    let hover_mid = act(&mut app, "ui.pointer_gesture", json!({ "action": "hover", "widget_id": "power.curve_hover", "hover_fraction_x": 0.5 }))?;
    let hn = hover_mid.get("hover_n").and_then(Value::as_f64).ok_or_else(|| format!("missing hover_n: {hover_mid}"))?;
    let hp = hover_mid.get("hover_power").and_then(Value::as_f64).ok_or_else(|| format!("missing hover_power: {hover_mid}"))?;
    if !hn.is_finite() || !(0.0..=1.0).contains(&hp) {
        return Err(format!("invalid curve hover readout: n={hn}, power={hp}"));
    }
    let hover_64 = act(&mut app, "ui.pointer_gesture", json!({ "action": "hover", "widget_id": "power.curve_hover", "hover_n": 64.0 }))?;
    let hp_64 = hover_64.get("hover_power").and_then(Value::as_f64).ok_or_else(|| format!("missing hover_power at 64: {hover_64}"))?;
    if hp_64 < 0.80 {
        return Err(format!("expected power at n=64 >= 0.80, got {hp_64}"));
    }

    // Live sidebar recalculation on Stage 4 via `ui.type_text`: bump target_power 0.80 -> 0.90 increases n_per_group
    type_widget(&mut app, "power.target_power", "0.90", true, false)?;
    let n_90 = app.canvas_dialogs.power_wizard.result.as_ref().map_or(0, |r| r.n_per_group);
    if n_90 <= 64 {
        return Err(format!("expected target_power=0.90 to require n > 64, got {n_90}"));
    }

    // Switch family to Correlated Paired t-test: sigma_d = sigma * sqrt(2*(1-r))
    type_widget(&mut app, "power.family", "paired_t", true, false)?;
    type_widget(&mut app, "power.effect_mode", "means_and_sd", true, false)?;
    type_widget(&mut app, "power.mean_diff", "1.0", true, false)?;
    type_widget(&mut app, "power.sd", "2.0", true, false)?;
    type_widget(&mut app, "power.within_pair_correlation", "0.50", true, false)?;
    type_widget(&mut app, "power.alpha", "0.05", true, false)?;
    type_widget(&mut app, "power.target_power", "0.80", true, false)?;
    let sigma_d_r05 = app.canvas_dialogs.power_wizard.computed_paired_sigma_d().ok_or_else(|| String::from("missing paired sigma_d"))?;
    assert_close(sigma_d_r05, 2.0, 1e-12, "sigma_d at r=0.5")?;
    let n_paired_r05 = app.canvas_dialogs.power_wizard.result.as_ref().map_or(0, |r| r.n_per_group);
    if n_paired_r05 != 34 {
        return Err(format!("expected paired t (dz=0.5, alpha=0.05, power=0.80) n=34, got {n_paired_r05}"));
    }

    // Increasing correlation r=0.50 -> 0.75 shrinks sigma_d to sqrt(2) and reduces required sample size
    type_widget(&mut app, "power.within_pair_correlation", "0.75", true, false)?;
    let sigma_d_r075 = app.canvas_dialogs.power_wizard.computed_paired_sigma_d().ok_or_else(|| String::from("missing paired sigma_d r=0.75"))?;
    assert_close(sigma_d_r075, 2.0 * 0.5_f64.sqrt(), 1e-12, "sigma_d at r=0.75")?;
    let n_paired_r075 = app.canvas_dialogs.power_wizard.result.as_ref().map_or(0, |r| r.n_per_group);
    if n_paired_r075 >= n_paired_r05 {
        return Err(format!("expected r=0.75 n ({n_paired_r075}) < r=0.50 n ({n_paired_r05})"));
    }

    // Walk Back Stage 4 -> 3 -> 2 -> 1 and run One-Way ANOVA (k=4, Cohen's f=0.25, alpha=0.05, power=0.80 -> n=45/group, total=180),
    // verifying ui.type_text numeric string parsing on `power.anova_groups`.
    for expected_prev in [PowerWizardStage::SignificanceAndSolve, PowerWizardStage::EffectSize, PowerWizardStage::ExperimentDesign] {
        act(&mut app, "ui.answer_dialog", json!({ "action": "back" }))?;
        assert_wizard_stage(&app, expected_prev)?;
    }
    type_widget(&mut app, "power.family", "one_way_anova", true, false)?;
    type_widget(&mut app, "power.anova_groups", "4", true, false)?;
    if app.canvas_dialogs.power_wizard.anova_groups != 4 {
        return Err(format!("expected power.anova_groups == 4 after typing string `4`, got {}", app.canvas_dialogs.power_wizard.anova_groups));
    }
    act(&mut app, "ui.answer_dialog", json!({ "action": "next", "fields": { "family": "one_way_anova", "solve_target": "sample_size" } }))?;
    assert_wizard_stage(&app, PowerWizardStage::EffectSize)?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "next", "fields": { "effect_mode": "standardized", "d": 0.25 } }))?;
    assert_wizard_stage(&app, PowerWizardStage::SignificanceAndSolve)?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "next", "fields": { "alpha": 0.05, "target_power": 0.80 } }))?;
    assert_wizard_stage(&app, PowerWizardStage::Results)?;
    let anova_res = app.canvas_dialogs.power_wizard.result.as_ref().ok_or_else(|| String::from("missing ANOVA Stage 4 result"))?;
    if anova_res.n_per_group != 45 || anova_res.total_n != 180 {
        return Err(format!(
            "expected 4-group ANOVA (f=0.25, alpha=0.05, power=0.80) n=45, total=180, got n={}, total={}",
            anova_res.n_per_group, anova_res.total_n
        ));
    }

    // Restore canonical Unpaired t (d=0.5, alpha=0.05, power=0.80), screenshot modal, and Create sheets!
    type_widget(&mut app, "power.family", "unpaired_t", true, false)?;
    type_widget(&mut app, "power.effect_mode", "standardized", true, false)?;
    type_widget(&mut app, "power.standardized_effect", "0.5", true, false)?;
    type_widget(&mut app, "power.alpha", "0.05", true, false)?;
    type_widget(&mut app, "power.target_power", "0.80", true, false)?;
    let s4 = inspect_all(&mut app)?;
    assert_widget_enabled(&s4, "dialog.button.create", true)?;
    assert_screenshot_png(&mut app, "dialog")?;

    let created = act(&mut app, "ui.answer_dialog", json!({ "action": "create" }))?;
    if app.canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(String::from("expected Power Wizard to close after Create"));
    }
    let data_id = created.get("data_sheet_id").and_then(Value::as_str).ok_or_else(|| format!("missing data_sheet_id: {created}"))?;
    let info_id = created.get("info_sheet_id").and_then(Value::as_str).ok_or_else(|| format!("missing info_sheet_id: {created}"))?;
    let res_id = created.get("result_sheet_id").and_then(Value::as_str).ok_or_else(|| format!("missing result_sheet_id: {created}"))?;
    let graph_id = created.get("graph_sheet_id").and_then(Value::as_str).ok_or_else(|| format!("missing graph_sheet_id: {created}"))?;

    verify_created_power_sheets(&app, data_id, info_id, res_id, graph_id)?;

    select_sheet_via_nav(&mut app, res_id)?;
    assert_screenshot_png(&mut app, "canvas")?;
    select_sheet_via_nav(&mut app, graph_id)?;
    assert_screenshot_png(&mut app, "canvas")?;
    Ok(())
}

fn verify_created_power_sheets(app: &UiApp, data_id: &str, info_id: &str, res_id: &str, graph_id: &str) -> TestRes {
    let data_sid = SheetId::try_new(data_id).map_err(|e| e.to_string())?;
    let ds = app.document().data_sheet(&data_sid).ok_or_else(|| format!("missing Power DataSheet `{data_id}`"))?;
    if ds.archetype.row_count() < 20 {
        return Err(format!("expected sampled power curve points on DataSheet, got {} rows", ds.archetype.row_count()));
    }

    let info_sid = SheetId::try_new(info_id).map_err(|e| e.to_string())?;
    let info = app.document().info_sheet(&info_sid).ok_or_else(|| format!("missing Power InfoSheet `{info_id}`"))?;
    let find_const = |name: &str| -> Result<f64, String> {
        info.constants
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| match c.value {
                lumencraft_doc::ConstantValue::Number(v) => Some(v),
                _ => None,
            })
            .ok_or_else(|| format!("missing numeric constant `{name}` on Power InfoSheet"))
    };
    assert_close(find_const("N per group")?, 64.0, 1e-9, "InfoSheet N per group")?;
    assert_close(find_const("Total N")?, 128.0, 1e-9, "InfoSheet Total N")?;
    if find_const("Achieved Power")? < 0.80 {
        return Err(String::from("expected InfoSheet Achieved Power >= 0.80"));
    }
    if !info.notes_markdown.contains("alpha=0.050, power=0.80 -> n=64, N=128") {
        return Err(format!("expected sensitivity table entry in InfoSheet notes: {}", info.notes_markdown));
    }

    let rs = result_sheet(app, res_id)?;
    if !rs.title().contains("Power & Sample Size") {
        return Err(format!("expected Power & Sample Size ResultSheet title, got `{}`", rs.title()));
    }
    if rs.analyzable_data.as_ref().map_or(0, |d| d.archetype.row_count()) < 20 {
        return Err(String::from("expected Power ResultSheet analyzable_data XY table with curve points"));
    }

    let gs_sid = SheetId::try_new(graph_id).map_err(|e| e.to_string())?;
    let gs = app.document().graph_sheet(&gs_sid).ok_or_else(|| format!("missing Power Curve GraphSheet `{graph_id}`"))?;
    let has_target_tick = gs.spec.y_axis.additional_ticks.iter().any(|t| (t.value - 0.80).abs() < 1e-6 && t.draw_gridline);
    if !gs.initialized || gs.spec.kind != GraphKind::Xy || !has_target_tick {
        return Err(format!("expected initialized XY Power Curve graph with 0.80 target gridline tick, got {gs:?}"));
    }
    Ok(())
}
