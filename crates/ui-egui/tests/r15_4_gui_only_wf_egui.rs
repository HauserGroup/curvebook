#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `R15.4` `--gui-only` + `--ui-backend=egui` Acceptance Matrix (`WF-01`–`WF-50` + `R34`).
//!
//! Every workflow constructs `UiApp::new().with_ui_backend(UiBackend::Egui)` with
//! `app.set_gui_only(true)` (`GuiOnlyPolicy::Strict`) and dispatches ALL 8 `ui.*`
//! actions exclusively over JSON-lines `app.dispatch_control_line(...)`, routing
//! `ui.inspect` -> `inspect_egui_shell_snapshot` and `ui.screenshot` ->
//! `render_egui_shell_screenshot_png` (`EguiSoftwareRasterizer`), while enforcing
//! `gui_only_violation` against direct control/disk bypasses.

use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ClusterLinkage, ContingencyMethod, CoxTieMethod, GlmFamily, GraphKind, MathTransformPreset, MultiplicityMethod,
    MultivariateSpec, OneWayMethod, PValueFormat, RepeatedMeasuresDesign, ResidualCovariance, ResultTabKind, SheetId, SurvivalAnalysisSpec,
    TableKind, TransformAnalysisSpec, TwoGroupMethod,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::power_wizard::PowerWizardStage;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

struct EguiGuiClient {
    app: UiApp,
    next_id: AtomicU64,
}

impl EguiGuiClient {
    fn new() -> Result<Self, String> {
        let mut app = UiApp::new().with_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        if app.ui_backend() != UiBackend::Egui {
            return Err(String::from("expected UiBackend::Egui"));
        }
        if !app.is_gui_only() {
            return Err(String::from("expected gui_only == true"));
        }
        Ok(Self { app, next_id: AtomicU64::new(1) })
    }

    fn ctrl(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = json!({ "id": id, "method": method, "params": params }).to_string();
        let raw = self.app.dispatch_control_line(&req);
        serde_json::from_str(&raw).map_err(|e| format!("parse control reply for `{method}`: {e}; raw={raw}"))
    }

    fn gui(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let reply = self.ctrl(method, params)?;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let code = reply.pointer("/error_detail/code").or_else(|| reply.pointer("/error/code")).and_then(Value::as_str).unwrap_or("unknown");
            let msg = reply
                .pointer("/error_detail/message")
                .or_else(|| reply.pointer("/error/message"))
                .or_else(|| reply.get("error"))
                .map_or_else(|| reply.to_string(), ToString::to_string);
            return Err(format!("{method} failed [{code}]: {msg}"));
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    fn expect_err_code(&mut self, method: &str, params: Value, expected_code: &str, ctx: &str) -> TestRes {
        let reply = self.ctrl(method, params)?;
        if reply.get("ok").and_then(Value::as_bool) != Some(false) {
            return Err(format!("{ctx}: expected error `{expected_code}`, got {reply}"));
        }
        let code = reply.pointer("/error_detail/code").or_else(|| reply.pointer("/error/code")).and_then(Value::as_str);
        if code != Some(expected_code) {
            return Err(format!("{ctx}: expected code `{expected_code}`, got {reply}"));
        }
        Ok(())
    }

    fn assert_strict_blocks(&mut self) -> TestRes {
        self.expect_err_code(
            "engine.execute",
            json!({ "command_id": "sheet.createData", "params": { "kind": "column", "title": "Bypass" } }),
            "gui_only_violation",
            "engine.execute under gui-only",
        )?;
        self.expect_err_code(
            "command.run",
            json!({ "command_id": "sheet.createData", "params": { "kind": "xy", "title": "Bypass" } }),
            "gui_only_violation",
            "command.run under gui-only",
        )?;
        self.expect_err_code("ui.layout.snap", json!({}), "gui_only_violation", "ui.layout.snap under gui-only")?;
        self.expect_err_code(
            "ui.screenshot",
            json!({ "target": "full_window", "path": "/tmp/forbidden_r15_4_gui_only.png" }),
            "gui_only_violation",
            "ui.screenshot disk path under gui-only",
        )?;
        let direct_err =
            dispatch_gui_action(&mut self.app, "ui.screenshot", json!({ "target": "full_window", "path": "/tmp/forbidden_r15_4_direct.png" }));
        if direct_err.is_ok() {
            return Err(String::from("expected direct ui.screenshot path write to fail"));
        }
        Ok(())
    }

    fn menu_cmd(&mut self, command_id: &str, params: Value) -> Result<Value, String> {
        self.gui("ui.invoke_menu", json!({ "command_id": command_id, "params": params }))
    }

    fn menu_path(&mut self, path: &str, params: Value) -> Result<Value, String> {
        self.gui("ui.invoke_menu", json!({ "path": path, "params": params }))
    }

    fn create_table(&mut self, params: Value) -> Result<String, String> {
        let out = self.menu_path("Insert/New Data Table", params)?;
        out.pointer("/payload/sheet_id")
            .or_else(|| out.get("sheet_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("missing sheet_id in create_table reply: {out}"))
    }

    fn select_sheet(&mut self, sheet_id: &str) -> TestRes {
        self.menu_path("View/Select Sheet", json!({ "sheet_id": sheet_id }))?;
        Ok(())
    }

    fn paste_tsv(&mut self, row: usize, col: usize, subcol: usize, tsv: &str) -> TestRes {
        let wid = format!("grid.cell.{row}.{col}.{subcol}");
        self.gui("ui.pointer_gesture", json!({ "action": "click", "widget_id": wid }))?;
        self.gui("ui.type_text", json!({ "widget_id": wid, "text": tsv, "clear_existing": true, "commit_enter": true }))?;
        Ok(())
    }

    fn type_x_cell(&mut self, row: usize, text: &str) -> TestRes {
        let wid = format!("grid.xcell.{row}.0");
        self.gui("ui.pointer_gesture", json!({ "action": "click", "widget_id": wid }))?;
        self.gui("ui.type_text", json!({ "widget_id": wid, "text": text, "clear_existing": true, "commit_enter": true }))?;
        Ok(())
    }

    fn run_analyze(&mut self, sheet_id: &str, fields: Value) -> Result<String, String> {
        self.select_sheet(sheet_id)?;
        let before: Vec<String> = self.app.document().result_sheets.iter().map(|r| r.id.as_str().to_owned()).collect();
        self.menu_path("Analyze/Analyze...", json!({}))?;
        let mut req = fields;
        if let Some(obj) = req.as_object_mut() {
            obj.entry("entry_id").or_insert_with(|| json!("stats.descriptive"));
        }
        let ans = self.gui("ui.answer_dialog", json!({ "action": "ok", "fields": req }))?;
        if let Some(id) = ans.pointer("/payload/sheet_id").and_then(Value::as_str) {
            return Ok(id.to_owned());
        }
        self.app
            .document()
            .result_sheets
            .iter()
            .map(|r| r.id.as_str().to_owned())
            .find(|id| !before.contains(id))
            .ok_or_else(|| format!("Analyze did not create ResultSheet for `{sheet_id}`"))
    }

    fn inspect(&mut self, scope: &str) -> Result<Value, String> {
        self.gui(
            "ui.inspect",
            json!({
                "scope": scope,
                "include_bounds": true,
                "metrics": { "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 }
            }),
        )
    }

    fn screenshot_png(&mut self, target: &str) -> Result<Vec<u8>, String> {
        let shot = self.gui("ui.screenshot", json!({ "target": target, "width_px": 960, "height_px": 600, "dpi": 96 }))?;
        if shot.get("path").is_some_and(|p| !p.is_null()) {
            return Err(format!("expected null screenshot path under --gui-only, got {shot}"));
        }
        let encoded = shot.get("data_base64").and_then(Value::as_str).ok_or("screenshot missing data_base64")?;
        let png = decode_base64(encoded)?;
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") || png.len() < 128 {
            return Err(format!("invalid egui PNG screenshot for `{target}` (len={})", png.len()));
        }
        Ok(png)
    }

    fn press_key(&mut self, key: &str, meta: bool, shift: bool) -> Result<Value, String> {
        self.gui("ui.press_key", json!({ "key": key, "modifiers": { "meta": meta, "shift": shift } }))
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
            _ => Err(format!("invalid base64 byte {b}")),
        }
    }
    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err(String::from("invalid base64 payload"));
    }
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for chunk in bytes.as_chunks::<4>().0 {
        let (a, b) = (val(chunk[0])?, val(chunk[1])?);
        out.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            let c = val(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk[3] != b'=' {
                out.push((c << 6) | val(chunk[3])?);
            }
        }
    }
    Ok(out)
}

fn ensure(cond: bool, msg: impl Into<String>) -> TestRes {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn verify_egui_inspect_and_shot(c: &mut EguiGuiClient, expected_sheet: &str) -> Result<Vec<u8>, String> {
    let snap = c.inspect("all")?;
    ensure(snap.get("gui_only").and_then(Value::as_bool) == Some(true), "expected gui_only=true in ui.inspect")?;
    ensure(
        snap.get("active_sheet_id").and_then(Value::as_str) == Some(expected_sheet),
        format!("expected active_sheet_id `{expected_sheet}`, got {:?}", snap.get("active_sheet_id")),
    )?;
    ensure(snap.get("menus").and_then(Value::as_array).is_some_and(|m| m.len() == 7), "expected 7 top-level menus in ui.inspect")?;
    let png = c.screenshot_png("full_window")?;
    let _canvas_png = c.screenshot_png("canvas")?;
    Ok(png)
}

#[test]
fn wf01_egui_gui_only_xy_dose_response_exclude_and_screenshot() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    c.assert_strict_blocks()?;
    let sid = c.create_table(json!({ "sheet_id": "xy-1", "kind": "xy", "cols": 2, "rows": 6, "replicates": 2, "auto_graph": true }))?;
    for (r, &x) in [1.0, 3.0, 10.0, 30.0, 100.0, 300.0].iter().enumerate() {
        c.type_x_cell(r, &format!("{x}"))?;
    }
    c.paste_tsv(0, 0, 0, "98\t101\n88\t91\n68\t72\n42\t46\n19\t21\n6\t8")?;
    c.gui("ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.2.0.1" }))?;
    c.menu_path("Edit/Exclude with Reason...", json!({}))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "reason_tag": "Outlier", "note": "Bubble" } }))?;
    let ds = c.app.document().data_sheet(&SheetId::from(sid.as_str())).ok_or("missing xy-1")?;
    ensure(ds.cell(CellCoord::new(2, 0, 1)).is_some_and(|cell| cell.excluded), "cell (2,0,1) must be excluded")?;
    c.gui("ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.1.0.0" }))?;
    c.gui("ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.3.0.0", "modifiers": { "shift": true } }))?;
    c.menu_cmd("ui.formatPoints.open", json!({}))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "color": "#DC2626", "symbol": "diamond", "size_pt": 9.0 } }))?;
    verify_egui_inspect_and_shot(&mut c, &sid)?;
    Ok(())
}

#[test]
fn wf02_egui_gui_only_normalize_to_result_chaining_and_log_x() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "xy-2", "kind": "xy", "cols": 1, "rows": 4, "auto_graph": true }))?;
    for (r, x) in [1.0, 10.0, 100.0, 1000.0].iter().enumerate() {
        c.type_x_cell(r, &format!("{x}"))?;
    }
    c.paste_tsv(0, 0, 0, "10\n40\n70\n100")?;
    let norm_id = c.run_analyze(&sid, json!({ "entry_id": "transform.normalize" }))?;
    c.select_sheet(&norm_id)?;
    let log_id = c.run_analyze(&norm_id, json!({ "entry_id": "transform.math", "x_transform": "log10", "y_transform": null }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(log_id.as_str())).ok_or("missing log result")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: Some(MathTransformPreset::Log10), .. })),
        "expected chained log10 transform",
    )?;
    verify_egui_inspect_and_shot(&mut c, &log_id)?;
    Ok(())
}

#[test]
fn wf03_egui_gui_only_column_brushing_format_and_undo_redo() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-3", "kind": "column", "cols": 2, "rows": 4, "auto_graph": true }))?;
    c.paste_tsv(0, 0, 0, "10\t20\n12\t24\n14\t28\n16\t32")?;
    c.gui("ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.0.0.0" }))?;
    c.gui("ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.2.0.0", "modifiers": { "shift": true } }))?;
    ensure(c.app.grid.selected_rows == [0, 1, 2], "expected Shift range 0..=2")?;
    c.menu_cmd("ui.formatPoints.open", json!({}))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "color": "#2563EB", "symbol": "triangle", "size_pt": 8.0 } }))?;
    let val_before = c.app.document().data_sheet(&SheetId::from("col-3")).unwrap().cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number);
    c.paste_tsv(0, 0, 0, "99.5")?;
    c.press_key("z", true, false)?;
    let val_undone = c.app.document().data_sheet(&SheetId::from("col-3")).unwrap().cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number);
    ensure(val_undone == val_before, "Cmd+Z must undo cell edit")?;
    c.press_key("z", true, true)?;
    let val_redone = c.app.document().data_sheet(&SheetId::from("col-3")).unwrap().cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number);
    ensure(val_redone == Some(99.5), "Cmd+Shift+Z must redo cell edit")?;
    verify_egui_inspect_and_shot(&mut c, &sid)?;
    Ok(())
}

#[test]
fn wf04_egui_gui_only_import_reshape_plate_and_convert_archetype() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    c.menu_path("File/Import Reshape...", json!({}))?;
    c.expect_err_code("ui.answer_dialog", json!({ "action": "ok" }), "bad_input", "unconfigured ImportReshape")?;
    let tidy = c.gui(
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": { "mode": "TidyLongPivot", "title": "Tidy", "target_kind": "Column", "tidy_csv": "Group,Value\nCtrl,10\nDrug,20" }
        }),
    )?;
    let sid = tidy.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or("missing tidy sheet")?.to_owned();
    c.select_sheet(&sid)?;
    c.menu_cmd("ui.table.convertArchetype.open", json!({ "sheet_id": sid }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "target_kind": "Xy" } }))?;
    let converted = c.app.document().data_sheet(&SheetId::from(sid.as_str())).ok_or("missing converted sheet")?;
    ensure(converted.kind() == TableKind::Xy, "expected Xy after ConvertArchetype")?;
    verify_egui_inspect_and_shot(&mut c, &sid)?;
    Ok(())
}

#[test]
fn wf05_egui_gui_only_prune_pca_and_clustering_pipeline() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "mv-5", "kind": "multiple_variables", "cols": 4, "rows": 12 }))?;
    c.paste_tsv(
        0,
        0,
        0,
        "1.0\t2.0\t10.0\t11.0\n1.2\t2.1\t10.5\t11.2\n5.0\t6.0\t4.0\t3.0\n5.3\t6.2\t4.1\t3.2\n\
         9.0\t10.0\t1.0\t2.0\n9.4\t10.3\t1.2\t2.1\n1.5\t2.4\t9.8\t10.9\n1.6\t2.6\t9.6\t10.7\n\
         5.1\t6.1\t4.3\t3.1\n5.4\t6.3\t4.4\t3.4\n9.2\t10.2\t1.3\t2.2\n9.5\t10.5\t1.4\t2.3",
    )?;
    let prune_id = c.run_analyze(&sid, json!({ "entry_id": "transform.prune" }))?;
    let pca_id = c.run_analyze(&prune_id, json!({ "entry_id": "stats.pca" }))?;
    c.select_sheet(&pca_id)?;
    c.gui("ui.switch_tab", json!({ "tab": "pca-loadings" }))?;
    let cl_id = c.run_analyze(&pca_id, json!({ "entry_id": "stats.clustering" }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(prune_id.as_str())).is_some(), "missing prune")?;
    ensure(c.app.document().result_sheet(&SheetId::from(pca_id.as_str())).is_some(), "missing pca")?;
    ensure(
        matches!(
            c.app.document().result_sheet(&SheetId::from(cl_id.as_str())).unwrap().analysis.kind,
            AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })
        ),
        "expected Clustering",
    )?;
    verify_egui_inspect_and_shot(&mut c, &cl_id)?;
    Ok(())
}

#[test]
fn wf06_egui_gui_only_prune_descriptive_survival_imputation() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-6", "kind": "column", "cols": 2, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "10\t15\n12\t\n14\t19\n16\t21\n18\t25")?;
    let _p = c.run_analyze(&sid, json!({ "entry_id": "transform.prune" }))?;
    let desc_id = c.run_analyze(&sid, json!({ "entry_id": "stats.descriptive" }))?;
    let _mi_id = c.run_analyze(&sid, json!({ "entry_id": "stats.multiple_imputation" }))?;
    let surv = c.create_table(json!({ "sheet_id": "surv-6", "kind": "survival", "cols": 2, "rows": 4 }))?;
    for (r, &(t, ev)) in [(2.0, "1"), (5.0, "1"), (4.0, "0"), (8.0, "1")].iter().enumerate() {
        c.type_x_cell(r, &format!("{t}"))?;
        c.paste_tsv(r, r % 2, 0, ev)?;
    }
    let km_id = c.run_analyze(&surv, json!({ "entry_id": "stats.survival_km" }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(desc_id.as_str())).is_some(), "missing descriptive")?;
    verify_egui_inspect_and_shot(&mut c, &km_id)?;
    Ok(())
}

#[test]
fn wf07_egui_gui_only_unpaired_student_and_welch_t_with_predicates() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-7", "kind": "column", "cols": 2, "rows": 7, "design_hint": "unpaired" }))?;
    c.paste_tsv(0, 0, 0, "10\t15\n11\t18\n12\t21\n10.5\t14\n11.5\t20\n11\t17\n\t19")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.two_group" }))?;
    c.press_key("t", true, false)?;
    ensure(matches!(c.app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }), "Cmd+T opens EditParams")?;
    c.expect_err_code(
        "ui.pointer_gesture",
        json!({ "action": "click", "widget_id": "edit.two_group.graph_differences" }),
        "conflict",
        "R4 disables graph_differences when unpaired",
    )?;
    c.gui(
        "ui.answer_dialog",
        json!({ "action": "ok", "fields": { "two_group_method": "welch_t", "p_value_format": "custom", "confidence_percent": 99.0 } }),
    )?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing res")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::WelchT), "expected WelchT")?;
    ensure(matches!(rs.analysis.p_value_format, PValueFormat::Custom { .. }), "expected Custom PValueFormat")?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf08_egui_gui_only_paired_t_equal_n_and_before_after() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-8", "kind": "column", "cols": 2, "rows": 5, "design_hint": "paired" }))?;
    c.paste_tsv(0, 0, 0, "10\t14\n12\t17\n14\t19\n16\t22\n18\t24")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.two_group", "paired_or_matched": true }))?;
    c.select_sheet(&res_id)?;
    c.press_key("t", true, false)?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "paired_or_matched": true } }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing res")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::PairedT), "expected PairedT")?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf09_egui_gui_only_mann_whitney_and_wilcoxon() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-9", "kind": "column", "cols": 2, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "1\t10\n2\t20\n3\t30\n4\t40\n5\t50")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.two_group" }))?;
    c.press_key("t", true, false)?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "assume_gaussian": false } }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing res")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref g) if g.method == TwoGroupMethod::MannWhitneyU), "expected MannWhitneyU")?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf10_egui_gui_only_one_way_anova_r1_r2_r3_and_qol7_diagnostics() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-10", "kind": "column", "cols": 4, "rows": 6, "design_hint": "unpaired" }))?;
    c.paste_tsv(0, 0, 0, "10\t12\t20\t25\n10.2\t12.5\t21\t45\n9.9\t11.8\t19.5\t65\n10.1\t12.2\t20.5\t35\n10\t12.1\t20.2\t55\n10.1\t12.3\t20.3\t40")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.one_way_anova" }))?;
    c.press_key("t", true, false)?;
    c.expect_err_code("ui.switch_tab", json!({ "tab": "repeated_measures" }), "conflict", "R1 RM tab disabled when unpaired")?;
    c.gui("ui.switch_tab", json!({ "tab": "multiple_comparisons" }))?;
    c.gui(
        "ui.answer_dialog",
        json!({ "action": "ok", "fields": { "compare_against_control": true, "control_column": 0, "multiplicity_method": "dunnett" } }),
    )?;
    c.expect_err_code(
        "ui.invoke_menu",
        json!({ "command_id": "ui.result.applyRemediation", "params": { "result_sheet_id": res_id } }),
        "conflict",
        "QOL-7 rejects auto remediation",
    )?;
    c.press_key("t", true, false)?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "anova_method": "welch" } }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing anova")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::BrownForsytheAndWelch), "expected Welch ANOVA")?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf11_egui_gui_only_kruskal_wallis_and_dunn() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-11", "kind": "column", "cols": 3, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "5\t15\t30\n6\t16\t35\n7\t18\t40\n8\t19\t45\n9\t20\t50")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.one_way_anova" }))?;
    c.press_key("t", true, false)?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "assume_gaussian": false, "multiplicity_method": "dunn" } }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing kw")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::KruskalWallis && a.multiplicity.method == MultiplicityMethod::DunnRankSum),
        "expected KruskalWallis + Dunn",
    )?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf12_egui_gui_only_two_way_anova_interaction() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "grp-12", "kind": "grouped", "cols": 2, "rows": 2, "replicates": 3 }))?;
    c.paste_tsv(0, 0, 0, "10\t11\t9\t20\t22\t21\n12\t13\t11\t35\t36\t34")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.factorial_anova" }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing factorial")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::FactorialAnova(_)), "expected FactorialAnova")?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf13_egui_gui_only_rm_anova_geisser_greenhouse() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-13", "kind": "column", "cols": 4, "rows": 6, "design_hint": "paired" }))?;
    c.paste_tsv(0, 0, 0, "10\t14\t18\t35\n12\t16\t20\t22\n14\t18\t22\t52\n16\t20\t24\t28\n18\t22\t26\t60\n20\t24\t28\t31")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.one_way_anova" }))?;
    c.press_key("t", true, false)?;
    c.expect_err_code("ui.switch_tab", json!({ "tab": "repeated_measures" }), "conflict", "RM disabled until paired checked")?;
    c.gui("ui.answer_dialog", json!({ "action": "apply", "fields": { "paired_or_matched": true } }))?;
    c.press_key("t", true, false)?;
    c.gui("ui.switch_tab", json!({ "tab": "repeated_measures" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "geisser_greenhouse": true } }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing rm anova")?;
    ensure(rs.title() == "RM one-way ANOVA", "expected RM one-way ANOVA")?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf14_egui_gui_only_friedman_and_remediation() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-14", "kind": "column", "cols": 3, "rows": 6, "design_hint": "paired" }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t5\n1.1\t2.1\t5.1\n1.2\t2.2\t5.2\n1.3\t2.3\t5.3\n1.4\t2.4\t5.4\n500\t600\t900")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.one_way_anova" }))?;
    c.select_sheet(&res_id)?;
    c.press_key("t", true, false)?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "paired_or_matched": true } }))?;
    c.expect_err_code(
        "ui.invoke_menu",
        json!({ "command_id": "ui.result.applyRemediation", "params": { "result_sheet_id": res_id } }),
        "conflict",
        "QOL-7 rejects auto remediation",
    )?;
    c.press_key("t", true, false)?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "assume_gaussian": false } }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing friedman")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::Friedman), "expected Friedman")?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf15_egui_gui_only_power_wizard_four_stages_hover_and_create() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    c.menu_path("Analyze/Power and Sample Size...", json!({}))?;
    ensure(matches!(c.app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize), "expected Power wizard")?;
    c.expect_err_code("ui.answer_dialog", json!({ "action": "back" }), "conflict", "back disabled on Stage 1")?;
    c.gui("ui.answer_dialog", json!({ "action": "next", "fields": { "family": "unpaired_t", "solve_target": "sample_size", "two_tailed": true } }))?;
    ensure(c.app.canvas_dialogs.power_wizard.stage == PowerWizardStage::EffectSize, "expected Stage 2")?;
    c.gui("ui.answer_dialog", json!({ "action": "next", "fields": { "effect_mode": "standardized", "d": 0.5 } }))?;
    c.gui("ui.answer_dialog", json!({ "action": "next", "fields": { "alpha": 0.05, "target_power": 0.80 } }))?;
    ensure(c.app.canvas_dialogs.power_wizard.stage == PowerWizardStage::Results, "expected Stage 4")?;
    c.gui("ui.pointer_gesture", json!({ "action": "click", "widget_id": "power.tab.curve" }))?;
    let hover = c.gui("ui.pointer_gesture", json!({ "action": "hover", "widget_id": "power.curve_hover", "hover_fraction_x": 0.5 }))?;
    ensure(hover.get("hover_n").and_then(Value::as_f64).is_some_and(|n| n > 0.0), "expected hover_n")?;
    let created = c.gui("ui.answer_dialog", json!({ "action": "create" }))?;
    let rs_id = created.get("result_sheet_id").and_then(Value::as_str).ok_or("missing power result")?.to_owned();
    ensure(created.get("graph_sheet_id").and_then(Value::as_str).is_some(), "missing power graph")?;
    verify_egui_inspect_and_shot(&mut c, &rs_id)?;
    Ok(())
}

#[test]
fn r34_egui_gui_only_unstructured_covariance_reml() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "grp-r34", "kind": "grouped", "cols": 3, "rows": 2, "replicates": 6 }))?;
    c.paste_tsv(
        0,
        0,
        0,
        "10.1\t10.4\t9.8\t10.2\t9.9\t10.5\t11.0\t12.1\t10.7\t11.6\t10.8\t11.9\t12.4\t13.8\t\t12.9\t12.0\t13.1\n\
         10.0\t10.3\t10.2\t9.7\t10.4\t10.1\t14.2\t15.8\t14.9\t13.7\t15.4\t14.6\t20.1\t23.4\t19.5\t21.8\t22.0\t20.9",
    )?;
    let res_id = c.run_analyze(
        &sid,
        json!({ "entry_id": "stats.factorial_anova", "repeated_measures": "column_factor_matched", "residual_covariance": "unstructured" }),
    )?;
    {
        let rs_pre = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing r34 pre")?;
        ensure(rs_pre.tabs.iter().filter(|t| t.kind == ResultTabKind::CovarianceMatrix).count() == 2, "expected 2 cov matrix tabs")?;
    }
    c.select_sheet(&res_id)?;
    c.press_key("t", true, false)?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "residual_covariance": "compound_symmetry" } }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing r34 res")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::FactorialAnova(ref f) if f.repeated_measures == RepeatedMeasuresDesign::ColumnFactorMatched && f.residual_covariance == ResidualCovariance::CompoundSymmetry),
        "expected CompoundSymmetry update",
    )?;
    ensure(rs.tabs.iter().any(|t| t.id == "mixed-reml"), "expected mixed-reml tab")?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf16_egui_gui_only_linear_passing_bablok_and_predicates() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "xy-16", "kind": "xy", "cols": 2, "rows": 5 }))?;
    for (r, &x) in [1.0, 2.0, 3.0, 4.0, 5.0].iter().enumerate() {
        c.type_x_cell(r, &format!("{x}"))?;
    }
    c.paste_tsv(0, 0, 0, "2.1\t3.0\n4.1\t5.9\n6.0\t8.8\n8.2\t11.9\n10.1\t14.8")?;
    let lin = c.run_analyze(&sid, json!({ "entry_id": "fit.linear" }))?;
    let pb = c.run_analyze(&sid, json!({ "entry_id": "stats.passing_bablok" }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(lin.as_str())).is_some(), "missing linear fit")?;
    verify_egui_inspect_and_shot(&mut c, &pb)?;
    Ok(())
}

#[test]
fn wf17_egui_gui_only_dose_response_4pl_and_corr_matrix_r7() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "xy-17", "kind": "xy", "cols": 1, "rows": 6, "replicates": 4 }))?;
    for (r, &x) in [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0].iter().enumerate() {
        c.type_x_cell(r, &format!("{x}"))?;
    }
    c.paste_tsv(0, 0, 0, "4\t5\t6\t5\n14\t16\t15\t17\n48\t52\t50\t51\n82\t85\t84\t83\n94\t96\t95\t95\n98\t99\t99\t98")?;
    let fit_id = c.run_analyze(&sid, json!({ "entry_id": "fit.nonlinear", "equation_id": "log_agonist_vs_response_4pl" }))?;
    let mv = c.create_table(json!({ "sheet_id": "mv-17", "kind": "multiple_variables", "cols": 3, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t5\n2\t4\t4\n3\t6\t3\n4\t8\t2\n5\t10\t1")?;
    let corr_id = c.run_analyze(&mv, json!({ "entry_id": "stats.corr_matrix", "correlation_all_pairs": false }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(fit_id.as_str())).is_some(), "missing 4pl")?;
    verify_egui_inspect_and_shot(&mut c, &corr_id)?;
    Ok(())
}

#[test]
fn wf18_egui_gui_only_enzyme_kinetics_michaelis_menten() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "xy-18", "kind": "xy", "cols": 2, "rows": 5 }))?;
    for (r, &x) in [0.5, 1.0, 2.5, 5.0, 10.0].iter().enumerate() {
        c.type_x_cell(r, &format!("{x}"))?;
    }
    c.paste_tsv(0, 0, 0, "10\t12\n18\t21\n32\t36\n45\t50\n56\t62")?;
    let fit_id = c.run_analyze(&sid, json!({ "entry_id": "fit.nonlinear", "equation_id": "michaelis_menten" }))?;
    verify_egui_inspect_and_shot(&mut c, &fit_id)?;
    Ok(())
}

#[test]
fn wf19_egui_gui_only_contingency_fisher_and_chi_square() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "cont-19", "kind": "contingency", "cols": 2, "rows": 2 }))?;
    c.paste_tsv(0, 0, 0, "18\t4\n6\t22")?;
    let res_id = c.run_analyze(&sid, json!({ "entry_id": "stats.contingency" }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res_id.as_str())).ok_or("missing cont")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Contingency(ref ct) if matches!(ct.method, ContingencyMethod::FisherExact | ContingencyMethod::PearsonChiSquare { .. })),
        "expected Contingency analysis",
    )?;
    verify_egui_inspect_and_shot(&mut c, &res_id)?;
    Ok(())
}

#[test]
fn wf20_egui_gui_only_roc_curve_and_trend() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "col-20", "kind": "column", "cols": 2, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "1.1\t3.2\n1.5\t4.0\n2.0\t4.8\n2.4\t5.5\n2.9\t6.1")?;
    let roc_id = c.run_analyze(&sid, json!({ "entry_id": "stats.roc" }))?;
    verify_egui_inspect_and_shot(&mut c, &roc_id)?;
    Ok(())
}

#[test]
fn wf21_egui_gui_only_kaplan_meier_survival() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "surv-21", "kind": "survival", "cols": 2, "rows": 6 }))?;
    for (r, &(t, col, v)) in [(2.0, 0, "1"), (4.0, 0, "1"), (6.0, 0, "0"), (5.0, 1, "0"), (8.0, 1, "1"), (12.0, 1, "0")].iter().enumerate() {
        c.type_x_cell(r, &format!("{t}"))?;
        c.paste_tsv(r, col, 0, v)?;
    }
    let km_id = c.run_analyze(&sid, json!({ "entry_id": "stats.survival_km" }))?;
    verify_egui_inspect_and_shot(&mut c, &km_id)?;
    Ok(())
}

#[test]
fn wf22_egui_gui_only_mv_pca_clustering_and_row_t_tests() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "mv-22", "kind": "multiple_variables", "cols": 3, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t4\n2\t4\t7\n3\t6\t9\n4\t9\t11\n5\t11\t14")?;
    let pca = c.run_analyze(&sid, json!({ "entry_id": "stats.pca", "pca_predictors": [0, 1, 2] }))?;
    let grp = c.create_table(json!({ "sheet_id": "grp-22", "kind": "grouped", "cols": 2, "rows": 3, "replicates": 3 }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t1.5\t8\t9\t8.5\n2\t3\t2.5\t12\t13\t12.5\n3\t4\t3.5\t15\t16\t15.5")?;
    let rt = c.run_analyze(&grp, json!({ "entry_id": "stats.multiple_row_t_tests" }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(pca.as_str())).is_some(), "missing pca")?;
    verify_egui_inspect_and_shot(&mut c, &rt)?;
    Ok(())
}

#[test]
fn wf23_egui_gui_only_pcr_r10_and_result_chaining() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let mv = c.create_table(json!({ "sheet_id": "mv-23", "kind": "multiple_variables", "cols": 3, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t3\n2\t3.9\t5.1\n3\t6.1\t7.2\n4\t8.0\t9.3\n5\t9.9\t11.1")?;
    let pcr = c.run_analyze(&mv, json!({ "entry_id": "stats.pcr", "pca_predictors": [0, 1], "pcr_outcome_column": 2 }))?;
    verify_egui_inspect_and_shot(&mut c, &pcr)?;
    Ok(())
}

#[test]
fn wf24_egui_gui_only_significance_brackets_and_axes() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid =
        c.create_table(json!({ "sheet_id": "col-24", "kind": "column", "cols": 3, "rows": 4, "auto_graph": true, "graph_sheet_id": "g-24" }))?;
    c.paste_tsv(0, 0, 0, "10\t20\t35\n11\t21\t37\n9\t19\t34\n12\t22\t38")?;
    let _res = c.run_analyze(&sid, json!({ "entry_id": "stats.one_way_anova" }))?;
    c.select_sheet("g-24")?;
    c.menu_cmd("ui.graph.axes.open", json!({ "sheet_id": "g-24" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "target_axis": "y", "title": "Signal (AU)" } }))?;
    verify_egui_inspect_and_shot(&mut c, "g-24")?;
    Ok(())
}

#[test]
fn wf25_egui_gui_only_violin_boxplot_and_svg() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid =
        c.create_table(json!({ "sheet_id": "col-25", "kind": "column", "cols": 3, "rows": 4, "auto_graph": true, "graph_sheet_id": "g-25" }))?;
    c.paste_tsv(0, 0, 0, "10\t15\t25\n12\t17\t27\n14\t19\t29\n11\t16\t26")?;
    c.menu_cmd("ui.changeGraphType.open", json!({ "sheet_id": "g-25" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "selected_option_id": "col.box_whisker" } }))?;
    let _ = c.run_analyze(&sid, json!({ "entry_id": "stats.one_way_anova" }))?;
    c.select_sheet("g-25")?;
    verify_egui_inspect_and_shot(&mut c, "g-25")?;
    Ok(())
}

#[test]
fn wf26_egui_gui_only_corr_heatmap_volcano_and_descriptive() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let mv = c.create_table(json!({ "sheet_id": "mv-26", "kind": "multiple_variables", "cols": 3, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t5\n2\t4\t8\n3\t6\t11\n4\t8\t14\n5\t10\t17")?;
    let desc = c.run_analyze(&mv, json!({ "entry_id": "stats.descriptive" }))?;
    let corr = c.run_analyze(&mv, json!({ "entry_id": "stats.corr_matrix", "correlation_all_pairs": true }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(desc.as_str())).is_some(), "missing desc")?;
    verify_egui_inspect_and_shot(&mut c, &corr)?;
    Ok(())
}

#[test]
fn wf27_egui_gui_only_multi_panel_layout_and_composer() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid =
        c.create_table(json!({ "sheet_id": "col-27", "kind": "column", "cols": 2, "rows": 4, "auto_graph": true, "graph_sheet_id": "g-27" }))?;
    c.paste_tsv(0, 0, 0, "10\t20\n12\t22\n14\t24\n16\t26")?;
    let _ = sid;
    c.menu_cmd("layout.create", json!({ "sheet_id": "lay-27", "title": "WF27 Figure" }))?;
    c.menu_cmd("layout.addPanel", json!({ "sheet_id": "lay-27", "kind": "graph", "graph_sheet_id": "g-27", "row": 0, "col": 0 }))?;
    c.select_sheet("lay-27")?;
    c.menu_cmd("ui.layout.composer.open", json!({ "sheet_id": "lay-27" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok" }))?;
    verify_egui_inspect_and_shot(&mut c, "lay-27")?;
    Ok(())
}

#[test]
fn wf28_egui_gui_only_canvas_preset_cvd_export_and_themes() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let _sid = c.create_table(json!({ "sheet_id": "xy-28", "kind": "xy", "cols": 1, "rows": 4, "auto_graph": true, "graph_sheet_id": "g-28" }))?;
    c.paste_tsv(0, 0, 0, "10\n20\n30\n40")?;
    c.select_sheet("g-28")?;
    c.menu_cmd("ui.canvas.applyPreset", json!({ "sheet_id": "g-28", "preset": "SingleColumn88mm" }))?;
    c.menu_cmd("ui.export.figure.open", json!({ "sheet_id": "g-28" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "format": "svg" } }))?;
    let light_png = verify_egui_inspect_and_shot(&mut c, "g-28")?;
    c.app.set_theme_mode(UiThemeMode::Dark);
    let dark_png = c.screenshot_png("full_window")?;
    ensure(light_png != dark_png, "Dark theme screenshot must differ from Light theme")?;
    Ok(())
}

#[test]
fn wf29_egui_gui_only_mv_select_math_row_stats_and_baseline() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-29",
        "kind": "multiple_variables",
        "rows": 4,
        "cols": 3,
        "variables": [
            { "title": "Cond", "role": "categorical" },
            { "title": "Conc", "role": "continuous" },
            { "title": "Signal", "role": "continuous" }
        ]
    }))?;
    c.paste_tsv(0, 0, 0, "Ctrl\t0.1\t100\nCtrl\t1.0\t102\nTreated\t0.1\t120\nTreated\t10.0\t180")?;
    let sel = c.run_analyze(&sid, json!({ "entry_id": "transform.select", "selected_data_sets": [1, 2], "target_archetype": "MultipleVariables", "filter_formula": "[Cond] == \"Treated\"" }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(sel.as_str())).ok_or("missing select")?;
    ensure(rs.analyzable_data.as_ref().is_some_and(|d| d.archetype.row_count() == 2), "expected 2 filtered rows")?;
    verify_egui_inspect_and_shot(&mut c, &sel)?;
    Ok(())
}

#[test]
fn wf30_egui_gui_only_calculated_variables_formula_bar() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "mv-30", "kind": "multiple_variables", "cols": 3, "rows": 4 }))?;
    c.paste_tsv(0, 0, 0, "10\t2\t0\n20\t4\t0\n30\t6\t0\n40\t8\t0")?;
    c.menu_cmd("ui.table.openFormulaBar", json!({ "column": 2 }))?;
    let _ = c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "formula": "[Col1] / [Col2]" } }));
    verify_egui_inspect_and_shot(&mut c, &sid)?;
    Ok(())
}

#[test]
fn wf31_egui_gui_only_extract_and_convert_cross_schema() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "xy-31", "kind": "xy", "cols": 1, "rows": 5, "replicates": 4 }))?;
    for (r, &x) in [-2.0, -1.0, 0.0, 1.0, 2.0].iter().enumerate() {
        c.type_x_cell(r, &format!("{x}"))?;
    }
    c.paste_tsv(0, 0, 0, "94\t96\t95\t95\n79\t81\t80\t82\n49\t51\t50\t52\n19\t21\t20\t18\n4\t6\t5\t5")?;
    let fit = c.run_analyze(&sid, json!({ "entry_id": "fit.nonlinear", "equation_id": "log_inhibitor_vs_response_4pl" }))?;
    verify_egui_inspect_and_shot(&mut c, &fit)?;
    Ok(())
}

#[test]
fn wf32_egui_gui_only_direct_mv_welch_t_and_bubble() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-32",
        "kind": "multiple_variables",
        "cols": 3,
        "rows": 6,
        "variables": [
            { "title": "Group", "role": "categorical" },
            { "title": "Response", "role": "continuous" },
            { "title": "Size", "role": "continuous" }
        ],
        "auto_graph": true,
        "graph_sheet_id": "g-32"
    }))?;
    c.paste_tsv(0, 0, 0, "A\t10\t2\nA\t11\t3\nA\t12\t4\nB\t20\t5\nB\t24\t6\nB\t28\t7")?;
    let res = c.run_analyze(&sid, json!({ "entry_id": "stats.two_group", "two_group_method": "welch_t" }))?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf33_egui_gui_only_direct_mv_multifactor_anova() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-33",
        "kind": "multiple_variables",
        "cols": 3,
        "rows": 8,
        "variables": [
            { "title": "Genotype", "role": "categorical" },
            { "title": "Dose", "role": "categorical" },
            { "title": "Readout", "role": "continuous" }
        ]
    }))?;
    c.paste_tsv(0, 0, 0, "WT\tLo\t10\nWT\tLo\t11\nWT\tHi\t15\nWT\tHi\t16\nKO\tLo\t20\nKO\tLo\t21\nKO\tHi\t35\nKO\tHi\t36")?;
    let res = c.run_analyze(&sid, json!({ "entry_id": "stats.factorial_anova" }))?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf34_egui_gui_only_qpcr_ddct_calculated_and_welch() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-34",
        "kind": "multiple_variables",
        "cols": 3,
        "rows": 6,
        "variables": [
            { "title": "Arm", "role": "categorical" },
            { "title": "TargetCt", "role": "continuous" },
            { "title": "RefCt", "role": "continuous" }
        ]
    }))?;
    c.paste_tsv(0, 0, 0, "Ctrl\t22\t18\nCtrl\t22.2\t18.1\nCtrl\t21.9\t18\nDrug\t19\t18\nDrug\t19.2\t18.1\nDrug\t18.8\t17.9")?;
    let res = c.run_analyze(&sid, json!({ "entry_id": "stats.two_group", "two_group_method": "welch_t" }))?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf35_egui_gui_only_direct_mv_kaplan_meier() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-35",
        "kind": "multiple_variables",
        "cols": 3,
        "rows": 6,
        "variables": [
            { "title": "Time", "role": "continuous" },
            { "title": "Status", "role": "continuous" },
            { "title": "Arm", "role": "categorical" }
        ]
    }))?;
    c.paste_tsv(0, 0, 0, "2\t1\tCtrl\n4\t1\tCtrl\n6\t0\tCtrl\n8\t1\tDrug\n12\t1\tDrug\n14\t0\tDrug")?;
    let res = c.run_analyze(&sid, json!({ "entry_id": "stats.survival_km" }))?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf36_egui_gui_only_direct_mv_4pl_and_equation() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-36",
        "kind": "multiple_variables",
        "cols": 2,
        "rows": 5,
        "variables": [{ "title": "LogDose", "role": "continuous" }, { "title": "Resp", "role": "continuous" }],
        "auto_graph": true,
        "graph_sheet_id": "g-36"
    }))?;
    c.paste_tsv(0, 0, 0, "-2\t95\n-1\t80\n0\t50\n1\t20\n2\t5")?;
    let fit = c.run_analyze(&sid, json!({ "entry_id": "fit.nonlinear", "equation_id": "log_inhibitor_vs_response_4pl" }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(fit.as_str())).is_some(), "missing mv 4pl")?;
    verify_egui_inspect_and_shot(&mut c, &fit)?;
    Ok(())
}

#[test]
fn wf37_egui_gui_only_hierarchical_clustering_linkages() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "mv-37", "kind": "multiple_variables", "cols": 3, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t1\n1.1\t2.2\t0.9\n5\t6\t5\n5.2\t5.9\t4.8\n9\t10\t9")?;
    let res = c.run_analyze(&sid, json!({ "entry_id": "stats.clustering", "pca_predictors": [0, 1, 2], "cluster_linkage": "upgma" }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res.as_str())).ok_or("missing clustering")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { linkage: ClusterLinkage::Upgma, .. })),
        "expected Upgma linkage",
    )?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf38_egui_gui_only_kmeans_clustering_and_silhouette() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "mv-38", "kind": "multiple_variables", "cols": 3, "rows": 6 }))?;
    c.paste_tsv(0, 0, 0, "1\t1\t1\n1.2\t1.1\t0.9\n1.1\t0.9\t1.1\n10\t10\t1.0\n10.2\t9.9\t1.1\n9.8\t10.1\t0.9")?;
    let res = c.run_analyze(&sid, json!({ "entry_id": "stats.clustering", "selected_data_sets": [0, 1, 2], "distance": "euclidean", "linkage": "ward", "k_means_k": 2, "k_sweep_max": 4 }))?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf39_egui_gui_only_cox_ph_efron_ties_and_covariate_curves() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-39",
        "kind": "multiple_variables",
        "rows": 10,
        "cols": 4,
        "variables": [
            { "title": "Time", "role": "continuous" },
            { "title": "Status", "role": "continuous" },
            { "title": "Arm", "role": "categorical" },
            { "title": "Age", "role": "continuous" }
        ]
    }))?;
    c.paste_tsv(
        0,
        0,
        0,
        "2\t1\tPlacebo\t68\n4\t1\tPlacebo\t66\n4\t1\tPlacebo\t64\n6\t1\tPlacebo\t62\n8\t0\tPlacebo\t60\n\
         4\t1\tDrug\t65\n7\t1\tDrug\t63\n11\t0\tDrug\t56\n12\t1\tDrug\t61\n16\t1\tDrug\t59",
    )?;
    let cox = c.run_analyze(
        &sid,
        json!({ "entry_id": "stats.survival_cox", "cox_time_column": 0, "cox_status_column": 1, "cox_predictor_columns": [2, 3], "tie_method": "efron" }),
    )?;
    let rs = c.app.document().result_sheet(&SheetId::from(cox.as_str())).ok_or("missing cox")?;
    ensure(
        matches!(rs.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: CoxTieMethod::Efron, .. })),
        "expected Efron Cox PH",
    )?;
    verify_egui_inspect_and_shot(&mut c, &cox)?;
    Ok(())
}

#[test]
fn wf40_egui_gui_only_multiple_linear_regression_glm() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-40",
        "kind": "multiple_variables",
        "cols": 3,
        "rows": 6,
        "variables": [{ "title": "X1", "role": "continuous" }, { "title": "X2", "role": "continuous" }, { "title": "Y", "role": "continuous" }]
    }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t5.1\n2\t1\t6.0\n3\t4\t11.2\n4\t3\t12.1\n5\t5\t16.0\n6\t4\t17.1")?;
    let res = c.run_analyze(&sid, json!({ "entry_id": "stats.glm", "glm_family": "linear", "response_column": 2, "selected_data_sets": [0, 1] }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res.as_str())).ok_or("missing linear glm")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Linear), "expected Linear GLM")?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf41_egui_gui_only_multiple_logistic_glm_and_roc() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-41",
        "kind": "multiple_variables",
        "cols": 3,
        "rows": 6,
        "variables": [{ "title": "Marker", "role": "continuous" }, { "title": "Age", "role": "continuous" }, { "title": "Outcome", "role": "continuous" }]
    }))?;
    c.paste_tsv(0, 0, 0, "0.2\t40\t0\n0.5\t45\t0\n1.0\t50\t0\n2.5\t55\t1\n3.5\t60\t1\n4.5\t65\t1")?;
    let res =
        c.run_analyze(&sid, json!({ "entry_id": "stats.glm", "glm_family": "logistic", "response_column": 2, "selected_data_sets": [0, 1] }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res.as_str())).ok_or("missing logistic glm")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Logistic), "expected Logistic GLM")?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf42_egui_gui_only_poisson_count_glm() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({
        "sheet_id": "mv-42",
        "kind": "multiple_variables",
        "cols": 3,
        "rows": 6,
        "variables": [{ "title": "Dose", "role": "continuous" }, { "title": "Age", "role": "continuous" }, { "title": "Counts", "role": "continuous" }]
    }))?;
    c.paste_tsv(0, 0, 0, "0\t30\t2\n1\t42\t4\n2\t31\t7\n3\t48\t13\n4\t35\t24\n5\t52\t42")?;
    let res = c.run_analyze(&sid, json!({ "entry_id": "stats.glm", "glm_family": "poisson", "response_column": 2, "selected_data_sets": [0, 1] }))?;
    let rs = c.app.document().result_sheet(&SheetId::from(res.as_str())).ok_or("missing poisson glm")?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Poisson), "expected Poisson GLM")?;
    verify_egui_inspect_and_shot(&mut c, &res)?;
    Ok(())
}

#[test]
fn wf43_egui_gui_only_serial_dilution_4pl_and_auc() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "xy-43", "kind": "xy", "cols": 1, "rows": 5, "replicates": 4 }))?;
    for (r, &x) in [0.01, 0.1, 1.0, 10.0, 100.0].iter().enumerate() {
        c.type_x_cell(r, &format!("{x}"))?;
    }
    c.paste_tsv(0, 0, 0, "95\t97\t96\t94\n81\t83\t82\t80\n49\t51\t50\t52\n17\t19\t18\t16\n3\t5\t4\t4")?;
    let log_rs = c.run_analyze(&sid, json!({ "entry_id": "transform.math", "x_transform": "log10" }))?;
    let desc = c.run_analyze(&log_rs, json!({ "entry_id": "stats.descriptive" }))?;
    let fit = c.run_analyze(&log_rs, json!({ "entry_id": "fit.nonlinear", "equation_id": "log_inhibitor_vs_response_4pl" }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(desc.as_str())).is_some(), "missing desc")?;
    verify_egui_inspect_and_shot(&mut c, &fit)?;
    Ok(())
}

#[test]
fn wf44_egui_gui_only_confidence_ellipses_and_hulls() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let _sid = c.create_table(json!({
        "sheet_id": "mv-44",
        "kind": "multiple_variables",
        "cols": 3,
        "rows": 6,
        "variables": [{ "title": "Group", "role": "categorical" }, { "title": "PC1", "role": "continuous" }, { "title": "PC2", "role": "continuous" }],
        "auto_graph": true,
        "graph_sheet_id": "g-44"
    }))?;
    c.paste_tsv(0, 0, 0, "A\t1\t2\nA\t1.5\t2.5\nA\t2\t1.8\nB\t6\t7\nB\t6.5\t7.4\nB\t7\t6.8")?;
    c.select_sheet("g-44")?;
    c.menu_cmd("ui.graph.bubblePlot.open", json!({ "sheet_id": "g-44" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok" }))?;
    verify_egui_inspect_and_shot(&mut c, "g-44")?;
    Ok(())
}

#[test]
fn wf45_egui_gui_only_summary_heatmap() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let mv = c.create_table(json!({ "sheet_id": "mv-45", "kind": "multiple_variables", "cols": 3, "rows": 5 }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t3\n2\t4\t5\n3\t6\t7\n4\t8\t9\n5\t10\t11")?;
    let corr = c.run_analyze(&mv, json!({ "entry_id": "stats.corr_matrix", "correlation_all_pairs": true, "correlation_heatmap": true }))?;
    verify_egui_inspect_and_shot(&mut c, &corr)?;
    Ok(())
}

#[test]
fn wf46_egui_gui_only_mv_graph_variable_swapping() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let _mv = c.create_table(json!({
        "sheet_id": "mv-46",
        "kind": "multiple_variables",
        "cols": 4,
        "rows": 4,
        "auto_graph": true,
        "graph_sheet_id": "g-46"
    }))?;
    c.paste_tsv(0, 0, 0, "1\t2\t3\t4\n2\t4\t6\t8\n3\t6\t9\t12\n4\t8\t12\t16")?;
    c.select_sheet("g-46")?;
    c.menu_cmd("ui.graph.bubblePlot.open", json!({ "sheet_id": "g-46" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "x_column": 1, "y_column": 2 } }))?;
    verify_egui_inspect_and_shot(&mut c, "g-46")?;
    Ok(())
}

#[test]
fn wf47_egui_gui_only_km_number_at_risk_and_pairwise() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid =
        c.create_table(json!({ "sheet_id": "surv-47", "kind": "survival", "cols": 3, "rows": 9, "auto_graph": true, "graph_sheet_id": "g-47" }))?;
    for r in 0..9 {
        c.type_x_cell(r, &format!("{}", (r + 1) * 3))?;
        c.paste_tsv(r, r % 3, 0, if r % 2 == 0 { "1" } else { "0" })?;
    }
    let km = c.run_analyze(&sid, json!({ "entry_id": "stats.survival_km" }))?;
    c.press_key("t", true, false)?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "trend_test": true, "multiplicity_method": "holm_sidak" } }))?;
    c.select_sheet("g-47")?;
    c.menu_cmd("ui.changeGraphType.open", json!({ "sheet_id": "g-47" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "selected_option_id": "survival.km" } }))?;
    ensure(c.app.document().result_sheet(&SheetId::from(km.as_str())).is_some(), "missing wf47 km")?;
    verify_egui_inspect_and_shot(&mut c, "g-47")?;
    Ok(())
}

#[test]
fn wf48_egui_gui_only_mv_bubble_and_layout_export() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let _mv = c.create_table(
        json!({ "sheet_id": "mv-48", "kind": "multiple_variables", "cols": 4, "rows": 5, "auto_graph": true, "graph_sheet_id": "g-48" }),
    )?;
    c.paste_tsv(0, 0, 0, "1\t10\t5\t2\n2\t20\t10\t4\n3\t30\t15\t6\n4\t40\t20\t8\n5\t50\t25\t10")?;
    c.select_sheet("g-48")?;
    c.menu_cmd("ui.graph.bubblePlot.open", json!({ "sheet_id": "g-48" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok" }))?;
    verify_egui_inspect_and_shot(&mut c, "g-48")?;
    Ok(())
}

#[test]
fn wf49_egui_gui_only_live_tex_model_equation_annotation() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(
        json!({ "sheet_id": "xy-49", "kind": "xy", "cols": 1, "rows": 5, "replicates": 3, "auto_graph": true, "graph_sheet_id": "g-49" }),
    )?;
    for (r, &x) in [0.0, 1.0, 2.0, 4.0, 8.0].iter().enumerate() {
        c.type_x_cell(r, &format!("{x}"))?;
    }
    c.paste_tsv(0, 0, 0, "99\t100\t101\n64\t65\t66\n41\t42\t43\n19\t20\t21\n10\t11\t12")?;
    let fit = c.run_analyze(&sid, json!({ "entry_id": "fit.nonlinear", "equation_id": "one_phase_decay" }))?;
    c.select_sheet("g-49")?;
    c.menu_cmd("graph.setRegressionOverlay", json!({ "sheet_id": "g-49", "result_sheet_id": fit }))?;
    c.menu_cmd("ui.graph.insertModelEquation", json!({ "sheet_id": "g-49" }))?;
    verify_egui_inspect_and_shot(&mut c, "g-49")?;
    Ok(())
}

#[test]
fn wf50_egui_gui_only_cld_piepho_reactive_svg_pdf_rgba() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    c.assert_strict_blocks()?;
    let sid =
        c.create_table(json!({ "sheet_id": "col-50", "kind": "column", "cols": 3, "rows": 5, "auto_graph": true, "graph_sheet_id": "g-50" }))?;
    c.paste_tsv(0, 0, 0, "10\t22\t40\n11\t24\t42\n9\t21\t39\n10.5\t23\t41\n10.2\t22.5\t40.5")?;
    c.select_sheet(&sid)?;
    c.menu_cmd("ui.analyze.open", json!({ "sheet_id": sid }))?;
    let ans = c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.one_way_anova" }, "force_create_new": true }))?;
    let anova_id = ans.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or("missing anova sheet")?.to_owned();
    c.select_sheet("g-50")?;
    c.menu_cmd("ui.changeGraphType.open", json!({ "sheet_id": "g-50" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "selected_option_id": "col.bar_replicates" } }))?;
    let before_png = verify_egui_inspect_and_shot(&mut c, "g-50")?;
    c.menu_cmd("ui.graph.pairwise.open", json!({ "sheet_id": "g-50" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "result_sheet_id": anova_id, "compact_letter_display": true } }))?;
    let gs = c.app.document().graph_sheet(&SheetId::from("g-50")).ok_or("missing g-50")?;
    ensure(gs.spec.kind == GraphKind::Column, "expected Column graph")?;
    c.menu_cmd("ui.export.figure.open", json!({ "sheet_id": "g-50" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "format": "svg" } }))?;
    c.menu_cmd("ui.export.figure.open", json!({ "sheet_id": "g-50" }))?;
    c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "format": "pdf" } }))?;
    let after_png = c.screenshot_png("canvas")?;
    ensure(!before_png.is_empty() && !after_png.is_empty(), "expected non-empty egui CLD screenshots")?;
    Ok(())
}
