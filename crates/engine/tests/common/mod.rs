#![forbid(unsafe_code)]
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Shared integration-test helpers (`TempDir`, `TempWorkspace`, `unique_temp_path`, command helpers,
//! sheet builders, inspection accessors, `approx_eq`, and Base64 codecs) for `crates/engine`.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, ErrorCode, GraphSheet, GroupedTable, InfoSheet,
    LayoutSheet, MultipleVariablesTable, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::{CommandOutcome, EngineError, Session};
use curvebook_testkit::assert_approx_eq;
use serde_json::{Value, json};

static SEQ: AtomicU64 = AtomicU64::new(1);

pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> Self {
        Self::try_new(prefix).expect("create temp dir")
    }

    pub fn try_new(prefix: &str) -> Result<Self, String> {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-eng-{prefix}-{}-{seq}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path).map_err(|e| format!("create temp dir `{}`: {e}", path.display()))?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn child(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub struct TempWorkspace {
    pub root: PathBuf,
    pub workdir: PathBuf,
    pub outside: PathBuf,
}

impl TempWorkspace {
    pub fn new(label: &str) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-eng-ws-{label}-{}-{seq}", std::process::id()));
        let workdir = root.join("workdir");
        let outside = root.join("outside");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&workdir).expect("create workdir");
        fs::create_dir_all(&outside).expect("create outside");
        Self { root, workdir, outside }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn unique_temp_path(label: &str, ext: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("cb-eng-{label}-{}-{seq}.{ext}", std::process::id()))
}

pub fn ok_cmd(session: &mut Session, id: &str, params: Value) -> CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

pub fn err_cmd(session: &mut Session, id: &str, params: Value) -> EngineError {
    match session.execute(id, params) {
        Ok(out) => panic!("command `{id}` unexpectedly succeeded: {:?}", out.payload),
        Err(err) => err,
    }
}

pub fn try_cmd(session: &mut Session, id: &str, params: Value) -> Result<CommandOutcome, Box<dyn Error>> {
    Ok(session.execute(id, params)?)
}

pub fn ok_payload(session: &mut Session, cmd: &str, params: Value) -> Result<Value, Box<dyn Error>> {
    Ok(session.execute(cmd, params)?.payload)
}

pub fn try_err_cmd(session: &mut Session, cmd: &str, params: Value) -> Result<EngineError, Box<dyn Error>> {
    match session.execute(cmd, params) {
        Ok(out) => Err(format!("expected `{cmd}` to fail, got payload: {:?}", out.payload).into()),
        Err(err) => Ok(err),
    }
}

pub fn run_cmds(session: &mut Session, cmds: &[(&str, Value)]) {
    for (id, params) in cmds {
        ok_cmd(session, id, params.clone());
    }
}

pub fn assert_err_code(session: &mut Session, id: &str, params: Value, expected: ErrorCode) -> EngineError {
    let err = err_cmd(session, id, params);
    assert_eq!(err.code, expected, "command `{id}` expected {expected:?}, got {err:?}");
    err
}

pub fn assert_bad_input(session: &mut Session, id: &str, params: Value) -> EngineError {
    assert_err_code(session, id, params, ErrorCode::BadInput)
}

pub fn assert_bad_inputs(session: &mut Session, cases: &[(&str, Value)]) {
    for (id, params) in cases {
        let _ = assert_bad_input(session, id, params.clone());
    }
}

pub fn assert_err_cases(session: &mut Session, cases: &[(&str, Value, ErrorCode)]) {
    for (id, params, code) in cases {
        let _ = assert_err_code(session, id, params.clone(), *code);
    }
}

pub fn assert_bad_pointer(session: &mut Session, id: &str, params: Value, expected_ptr: &str) {
    let err = assert_bad_input(session, id, params);
    assert_eq!(err.category, ErrorCode::BadInput.category());
    let ptr = err.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str);
    assert_eq!(ptr, Some(expected_ptr), "`{id}` details: {:?}", err.details);
}

pub fn approx_eq(actual: f64, expected: f64, tol: f64, label: &str) {
    assert_approx_eq(actual, expected, tol, tol, label);
}

pub fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

pub fn decode_base64(input: &str) -> Vec<u8> {
    curvebook_platform::base64::decode(input).expect("decode test base64")
}

// ---------------------------------------------------------------------------
// Sheet & Session builders
// ---------------------------------------------------------------------------

pub fn col_sheet_hint(id: &str, title: &str, hint: ColumnDesignHint, cols: &[(&str, &[f64])]) -> DataSheet {
    let rows = cols.iter().map(|(_, vs)| vs.len()).max().unwrap_or(1).max(1);
    let mut table = ColumnTable::new(hint, cols.len().max(1), rows).expect("column table");
    for (ds, (name, vals)) in table.data_sets.iter_mut().zip(cols.iter()) {
        ds.title = (*name).to_owned();
        for (row, &v) in ds.cells.iter_mut().zip(vals.iter()) {
            row[0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Column(table)).expect("col DataSheet")
}

pub fn col_sheet(id: &str, title: &str, cols: &[(&str, &[f64])]) -> DataSheet {
    col_sheet_hint(id, title, ColumnDesignHint::UnpairedReplicates, cols)
}

pub fn col_sheet_opts(id: &str, title: &str, hint: ColumnDesignHint, cols: &[(&str, &[Option<f64>])]) -> DataSheet {
    let rows = cols.iter().map(|(_, vs)| vs.len()).max().unwrap_or(1).max(1);
    let mut table = ColumnTable::new(hint, cols.len().max(1), rows).expect("column table");
    for (ds, (name, vals)) in table.data_sets.iter_mut().zip(cols.iter()) {
        ds.title = (*name).to_owned();
        for (row, &opt) in ds.cells.iter_mut().zip(vals.iter()) {
            if let Some(v) = opt {
                row[0] = Cell::number(v);
            }
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Column(table)).expect("col opts DataSheet")
}

pub fn xy_sheet(id: &str, title: &str, x_title: &str, xs: &[f64], ys: &[(&str, &[f64])]) -> DataSheet {
    let rows = xs.len().max(ys.iter().map(|(_, vs)| vs.len()).max().unwrap_or(0)).max(1);
    let mut table = XyTable::new(SubcolumnMode::Single, ys.len().max(1), rows).expect("xy table");
    table.x_title = x_title.to_owned();
    for (i, &x) in xs.iter().enumerate() {
        table.x_cells[i][0] = Cell::number(x);
    }
    for (ds, (name, vals)) in table.data_sets.iter_mut().zip(ys.iter()) {
        ds.title = (*name).to_owned();
        for (row, &v) in ds.cells.iter_mut().zip(vals.iter()) {
            row[0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Xy(table)).expect("xy DataSheet")
}

pub fn xy_rep_sheet(id: &str, title: &str, x_title: &str, xs: &[f64], ys: &[(&str, &[&[f64]])]) -> DataSheet {
    let reps = ys.iter().flat_map(|(_, rows)| rows.iter().map(|r| r.len())).max().unwrap_or(1).max(1);
    let mode = if reps == 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(reps) };
    let rows = xs.len().max(ys.iter().map(|(_, r)| r.len()).max().unwrap_or(0)).max(1);
    let mut table = XyTable::new(mode, ys.len().max(1), rows).expect("xy rep table");
    table.x_title = x_title.to_owned();
    for (i, &x) in xs.iter().enumerate() {
        table.x_cells[i][0] = Cell::number(x);
    }
    for (ds, (name, ds_rows)) in table.data_sets.iter_mut().zip(ys.iter()) {
        ds.title = (*name).to_owned();
        for (r, vals) in ds_rows.iter().enumerate() {
            for (s, &v) in vals.iter().enumerate() {
                ds.cells[r][s] = Cell::number(v);
            }
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Xy(table)).expect("xy rep DataSheet")
}

/// Standard 10-point 4PL dose-response XY table (`Y = 10 + 90 / (1 + 10^((-6.5 - X) * 1.1))`, 2 replicates).
pub fn xy_4pl_sheet(id: &str) -> DataSheet {
    let xs = [-9.5_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mut table = XyTable::new(SubcolumnMode::RawReplicates(2), 1, xs.len()).expect("4pl xy");
    table.x_title = String::from("Log10 Concentration (M)");
    let ds0 = &mut table.data_sets[0];
    ds0.title = String::from("Compound A");
    for ((&x, x_row), row) in xs.iter().zip(table.x_cells.iter_mut()).zip(ds0.cells.iter_mut()) {
        x_row[0] = Cell::number(x);
        let y_true = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.1));
        row[0] = Cell::number(y_true - 0.8);
        row[1] = Cell::number(y_true + 0.8);
    }
    DataSheet::new(SheetId::new(id), "Dose Response", TableArchetype::Xy(table)).expect("4pl DataSheet")
}

pub fn grouped_rep_sheet(id: &str, title: &str, row_titles: &[&str], cols: &[(&str, &[&[f64]])]) -> DataSheet {
    let reps = cols.iter().flat_map(|(_, rows)| rows.iter().map(|r| r.len())).max().unwrap_or(1).max(1);
    let rows = row_titles.len().max(cols.iter().map(|(_, r)| r.len()).max().unwrap_or(0)).max(1);
    let mut table = GroupedTable::new(SubcolumnMode::RawReplicates(reps), cols.len().max(1), rows).expect("grouped table");
    for (r, &rt) in row_titles.iter().enumerate() {
        table.row_titles[r] = rt.to_owned();
    }
    for (ds, (name, ds_rows)) in table.data_sets.iter_mut().zip(cols.iter()) {
        ds.title = (*name).to_owned();
        for (r, vals) in ds_rows.iter().enumerate() {
            for (s, &v) in vals.iter().enumerate() {
                ds.cells[r][s] = Cell::number(v);
            }
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Grouped(table)).expect("grouped DataSheet")
}

pub fn survival_sheet(id: &str, title: &str, cohorts: &[&str], rows: &[(f64, &[Option<f64>])]) -> DataSheet {
    let mut table = SurvivalTable::new(cohorts.len().max(1), rows.len().max(1)).expect("survival table");
    for (ch, &name) in table.cohorts.iter_mut().zip(cohorts.iter()) {
        ch.title = name.to_owned();
    }
    for (r, &(t, events)) in rows.iter().enumerate() {
        table.times[r] = Cell::number(t);
        for (c, &ev) in events.iter().enumerate() {
            if let Some(v) = ev {
                table.cohorts[c].cells[r][0] = Cell::number(v);
            }
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Survival(table)).expect("survival DataSheet")
}

pub fn contingency_sheet(id: &str, title: &str, row_titles: &[&str], col_titles: &[&str], rows: &[&[f64]]) -> DataSheet {
    let nrows = rows.len().max(row_titles.len()).max(1);
    let ncols = col_titles.len().max(rows.first().map_or(1, |r| r.len())).max(1);
    let mut table = ContingencyTable::new(ncols, nrows).expect("contingency table");
    for (r, &rt) in row_titles.iter().enumerate() {
        table.row_titles[r] = rt.to_owned();
    }
    for (c, &ct) in col_titles.iter().enumerate() {
        table.data_sets[c].title = ct.to_owned();
    }
    for (r, row) in rows.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            table.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Contingency(table)).expect("contingency DataSheet")
}

pub fn mv_sheet(id: &str, title: &str, row_titles: &[&str], vars: &[(&str, Option<VariableRole>, &[Cell])]) -> DataSheet {
    let rows = row_titles.len().max(vars.iter().map(|(_, _, cs)| cs.len()).max().unwrap_or(0)).max(1);
    let mut table = MultipleVariablesTable::new(vars.len().max(1), rows).expect("mv table");
    for (r, &rt) in row_titles.iter().enumerate() {
        table.row_titles[r] = rt.to_owned();
    }
    for (v, (name, role, cells)) in table.variables.iter_mut().zip(vars.iter()) {
        v.title = (*name).to_owned();
        if let Some(r) = role {
            v.lock_role(*r);
        }
        for (i, cell) in cells.iter().enumerate() {
            v.cells[i] = cell.clone();
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::MultipleVariables(table)).expect("mv DataSheet")
}

pub fn session_with(title: &str, sheets: Vec<DataSheet>) -> Session {
    let mut doc = ProjectDocument::new(title).expect("project doc");
    for sheet in sheets {
        doc.add_data_sheet(sheet).expect("add data sheet");
    }
    Session::from_document(doc)
}

// ---------------------------------------------------------------------------
// Document inspection & round-trip helpers
// ---------------------------------------------------------------------------

pub fn ds<'a>(session: &'a Session, id: &str) -> &'a DataSheet {
    session.document().data_sheet(&SheetId::new(id)).unwrap_or_else(|| panic!("missing DataSheet `{id}`"))
}

pub fn rs<'a>(session: &'a Session, id: &str) -> &'a ResultSheet {
    session.document().result_sheet(&SheetId::new(id)).unwrap_or_else(|| panic!("missing ResultSheet `{id}`"))
}

pub fn gs<'a>(session: &'a Session, id: &str) -> &'a GraphSheet {
    session.document().graph_sheet(&SheetId::new(id)).unwrap_or_else(|| panic!("missing GraphSheet `{id}`"))
}

pub fn ls<'a>(session: &'a Session, id: &str) -> &'a LayoutSheet {
    session.document().layout_sheet(&SheetId::new(id)).unwrap_or_else(|| panic!("missing LayoutSheet `{id}`"))
}

pub fn info<'a>(session: &'a Session, id: &str) -> &'a InfoSheet {
    session.document().info_sheet(&SheetId::new(id)).unwrap_or_else(|| panic!("missing InfoSheet `{id}`"))
}

pub fn cell_val(session: &Session, sheet_id: &str, row: usize, col: usize, subcol: usize) -> Option<Cell> {
    ds(session, sheet_id).cell(CellCoord::new(row, col, subcol)).cloned()
}

pub fn cell_num(session: &Session, sheet_id: &str, row: usize, col: usize, subcol: usize) -> Option<f64> {
    match ds(session, sheet_id).cell(CellCoord::new(row, col, subcol))?.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

pub fn x_num(session: &Session, sheet_id: &str, row: usize) -> Option<f64> {
    match ds(session, sheet_id).x_or_time_cell(row, 0)?.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

pub fn col_nums(session: &Session, sheet_id: &str, col: usize) -> Vec<f64> {
    subcol_nums(session, sheet_id, col, 0)
}

pub fn subcol_nums(session: &Session, sheet_id: &str, col: usize, subcol: usize) -> Vec<f64> {
    let sheet = ds(session, sheet_id);
    (0..sheet.archetype.row_count()).filter_map(|r| sheet.cell(CellCoord::new(r, col, subcol)).and_then(Cell::active_number)).collect()
}

pub fn scalar(session: &Session, res_id: &str, key: &str) -> f64 {
    rs(session, res_id).scalars.iter().find(|s| s.key == key).unwrap_or_else(|| panic!("missing scalar `{key}` in `{res_id}`")).value
}

pub fn scalar_ds(session: &Session, res_id: &str, key: &str, ds_idx: Option<usize>) -> Option<f64> {
    rs(session, res_id).scalars.iter().find(|s| s.key == key && s.data_set_index == ds_idx).map(|s| s.value)
}

pub fn tab_metric(session: &Session, res_id: &str, label: &str) -> Option<f64> {
    for tab in &rs(session, res_id).tabs {
        for (idx, row_label) in tab.row_labels.iter().enumerate() {
            if row_label == label
                && let Some(curvebook_doc::ResultCell::Number { value, .. }) = tab.rows.get(idx).and_then(|r| r.first())
            {
                return Some(*value);
            }
        }
    }
    None
}

pub fn save_and_reopen(session: &mut Session, path: &Path) -> Session {
    ok_cmd(session, "project.save", json!({ "path": path.to_string_lossy() }));
    let mut reopened = Session::new();
    ok_cmd(&mut reopened, "project.open", json!({ "path": path.to_string_lossy() }));
    reopened
}

pub fn cleanroom_pzfx_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <Info Sequence="info-xml">
    <Title>Metadata</Title>
    <Constant><Name>Conc_uM</Name><Value>2.5</Value></Constant>
    <Notes>Clean XML fixture</Notes>
  </Info>
  <Table Sequence="tbl-xml" TableType="OneWay">
    <Title>Cytokine</Title>
    <YColumn Width="80" Subcolumns="1"><Title>Ctrl</Title><Subcolumn><d>11.0</d><d>13.0</d></Subcolumn></YColumn>
    <YColumn Width="80" Subcolumns="1"><Title>Treated</Title><Subcolumn><d>21.0</d><d>24.0</d></Subcolumn></YColumn>
  </Table>
</PrismFile>"#
}
