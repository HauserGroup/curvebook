//! Regression tests for Stage 3B S1 fixes in `lumencraft-engine`:
//! - `R1-S07-10`: Cyclic graph/layout/result source chains terminate cleanly in `sheet.duplicateFamily` / `sheet.clonePipeline`.
//! - `R1-S12-03`: Wholesale document-replacement commands clear/update `Session::file_path`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec,
    ProjectDocument, RectPt, ResultSheet, SheetId, TableArchetype,
};
use lumencraft_engine::Session;
use serde_json::{Value, json};

type TestResult = Result<(), String>;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lc-engine-s3b-{label}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|err| format!("create temp dir `{}`: {err}", dir.display()))?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run_with_small_stack<F>(name: &str, f: F) -> TestResult
where
    F: FnOnce() -> TestResult + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(name.to_owned())
        .stack_size(512 * 1024)
        .spawn(move || {
            let res = f();
            let _ = tx.send(res);
        })
        .map_err(|err| format!("spawn thread `{name}`: {err}"))?;

    let outcome = rx.recv_timeout(Duration::from_secs(10)).map_err(|err| format!("timed out or channel closed waiting for `{name}`: {err}"))?;
    handle.join().map_err(|_| format!("thread `{name}` panicked"))?;
    outcome
}

fn make_col_sheet(id: &str) -> Result<DataSheet, String> {
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?;
    DataSheet::new(id, id, TableArchetype::Column(col)).map_err(|e| e.to_string())
}

#[test]
fn r1_s07_10_duplicate_family_and_clone_pipeline_terminate_on_self_and_mutual_graph_cycles() -> TestResult {
    // 1. Self-referential GraphSheet via graph.setSpec (`g -> g`).
    run_with_small_stack("s07-10-self-graph", || {
        let mut base = Session::new();
        base.execute("sheet.createData", json!({ "sheet_id": "d", "graph_sheet_id": "g" })).map_err(|e| e.to_string())?;
        let mut doc = base.document().clone();
        doc.graph_sheets[0].spec.primary_source_sheet_id = "g".into();
        let mut session = Session::from_document(doc);

        let err_dup = session
            .execute("sheet.duplicateFamily", json!({ "sheet_id": "g" }))
            .err()
            .ok_or_else(|| String::from("expected duplicateFamily on self-referential graph to fail"))?;
        if err_dup.code != ErrorCode::CycleDetected {
            return Err(format!("expected CycleDetected for self-graph duplicateFamily, got {:?}", err_dup.code));
        }

        let err_clone = session
            .execute("sheet.clonePipeline", json!({ "sheet_id": "g" }))
            .err()
            .ok_or_else(|| String::from("expected clonePipeline on self-referential graph to fail"))?;
        if err_clone.code != ErrorCode::CycleDetected {
            return Err(format!("expected CycleDetected for self-graph clonePipeline, got {:?}", err_clone.code));
        }
        Ok(())
    })?;

    // 2. Mutual 2-hop graph cycle (`g1 -> g2 -> g1`).
    run_with_small_stack("s07-10-mutual-graphs", || {
        let mut doc = ProjectDocument::new("Mutual Graphs").map_err(|e| e.to_string())?;
        let ds = make_col_sheet("d1")?;
        let mut g1 = GraphSheet::new_uninitialized_for_data("g1", &ds).map_err(|e| e.to_string())?;
        let mut g2 = GraphSheet::new_uninitialized_for_data("g2", &ds).map_err(|e| e.to_string())?;
        g1.spec.primary_source_sheet_id = SheetId::new("g2");
        g2.spec.primary_source_sheet_id = SheetId::new("g1");
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;
        doc.add_graph_sheet(g2).map_err(|e| e.to_string())?;

        let mut session = Session::from_document(doc);
        let err = session
            .execute("sheet.duplicateFamily", json!({ "sheet_id": "g1" }))
            .err()
            .ok_or_else(|| String::from("expected mutual graph cycle to fail"))?;
        if err.code != ErrorCode::CycleDetected {
            return Err(format!("expected CycleDetected for mutual graphs, got {:?}", err.code));
        }
        Ok(())
    })?;

    Ok(())
}

#[test]
fn r1_s07_10_duplicate_family_terminates_on_layout_and_mixed_cycles_while_allowing_valid_fallback() -> TestResult {
    // 1. Cross-section cycle (`l1 -> l2 -> l1` and `l1 -> g1 -> l1`) and result->graph->result cycle.
    run_with_small_stack("s07-10-layout-and-mixed", || {
        let mut doc = ProjectDocument::new("Layout Cycle").map_err(|e| e.to_string())?;
        let ds = make_col_sheet("d1")?;
        let mut g1 = GraphSheet::new_uninitialized_for_data("g1", &ds).map_err(|e| e.to_string())?;
        g1.spec.primary_source_sheet_id = SheetId::new("l1");
        let l1 = LayoutSheet::new(
            "l1",
            "L1",
            LayoutSpec {
                items: vec![LayoutItem::LiveGraph {
                    id: String::from("p1"),
                    graph_sheet_id: SheetId::new("g1"),
                    bounds_pt: RectPt::new(0.0, 0.0, 100.0, 100.0),
                    scale_percent: 100.0,
                    badge_override: None,
                }],
                ..LayoutSpec::default()
            },
        )
        .map_err(|e| e.to_string())?;

        let mut l2 = LayoutSheet::new("l2", "L2", LayoutSpec::default()).map_err(|e| e.to_string())?;
        l2.spec.items.push(LayoutItem::LiveTable {
            id: String::from("t2"),
            source_sheet_id: SheetId::new("l3"),
            tab_id: None,
            bounds_pt: RectPt::new(0.0, 0.0, 50.0, 50.0),
        });
        let l3 = LayoutSheet::new(
            "l3",
            "L3",
            LayoutSpec {
                items: vec![LayoutItem::LiveTable {
                    id: String::from("t3"),
                    source_sheet_id: SheetId::new("l2"),
                    tab_id: None,
                    bounds_pt: RectPt::new(0.0, 0.0, 50.0, 50.0),
                }],
                ..LayoutSpec::default()
            },
        )
        .map_err(|e| e.to_string())?;

        let mut r1 = ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::DescriptiveStats(Default::default())).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        r1.analysis.source_sheet_id = SheetId::new("g_res");
        let mut g_res = GraphSheet::new_uninitialized_for_data("g_res", &ds).map_err(|e| e.to_string())?;
        g_res.spec.primary_source_sheet_id = SheetId::new("r1");

        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;
        doc.add_graph_sheet(g_res).map_err(|e| e.to_string())?;
        doc.add_layout_sheet(l1).map_err(|e| e.to_string())?;
        doc.add_layout_sheet(l2).map_err(|e| e.to_string())?;
        doc.add_layout_sheet(l3).map_err(|e| e.to_string())?;
        doc.add_result_sheet(r1).map_err(|e| e.to_string())?;

        let mut session = Session::from_document(doc);
        for sheet_id in ["l1", "l2", "r1"] {
            let err = session
                .execute("sheet.duplicateFamily", json!({ "sheet_id": sheet_id }))
                .err()
                .ok_or_else(|| format!("expected cycle error when duplicating `{sheet_id}`"))?;
            if err.code != ErrorCode::CycleDetected {
                return Err(format!("expected CycleDetected for `{sheet_id}`, got {:?}", err.code));
            }
        }
        Ok(())
    })?;

    // 2. Layout whose first item cannot resolve (`InfoSheet` with no owning data sheet),
    // followed by a valid data-backed item, recovers cleanly via item 1.
    run_with_small_stack("s07-10-layout-fallback", || {
        let mut doc = ProjectDocument::new("Layout Fallback").map_err(|e| e.to_string())?;
        let ds = make_col_sheet("d1")?;
        let orphan_info = InfoSheet::new_preseeded("info_orphan", "Orphan Info").map_err(|e| e.to_string())?;
        let l_mixed = LayoutSheet::new(
            "l_mixed",
            "Mixed Layout",
            LayoutSpec {
                items: vec![
                    LayoutItem::LiveTable {
                        id: String::from("bad"),
                        source_sheet_id: SheetId::new("info_orphan"),
                        tab_id: None,
                        bounds_pt: RectPt::new(0.0, 0.0, 50.0, 50.0),
                    },
                    LayoutItem::LiveTable {
                        id: String::from("good"),
                        source_sheet_id: SheetId::new("d1"),
                        tab_id: None,
                        bounds_pt: RectPt::new(0.0, 0.0, 50.0, 50.0),
                    },
                ],
                ..LayoutSpec::default()
            },
        )
        .map_err(|e| e.to_string())?;

        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.add_info_sheet(orphan_info).map_err(|e| e.to_string())?;
        doc.add_layout_sheet(l_mixed).map_err(|e| e.to_string())?;

        let mut session = Session::from_document(doc);
        let outcome = session.execute("sheet.duplicateFamily", json!({ "sheet_id": "l_mixed", "prefix": "copy" })).map_err(|e| e.to_string())?;
        if outcome.payload.get("source_sheet_id").and_then(Value::as_str) != Some("d1") {
            return Err(format!("expected source_sheet_id `d1`, got {}", outcome.payload));
        }
        Ok(())
    })
}

#[test]
fn r1_s12_03_wholesale_document_replacements_clear_or_update_session_file_path() -> TestResult {
    let dir = TempDir::new("file-path")?;
    let path_a = dir.path().join("project_a.lumencraft");
    let path_b = dir.path().join("project_b.lumencraft");
    let path_tpl = dir.path().join("plate.lctemplate");

    let mut builder = Session::new();
    builder
        .execute("sheet.createData", json!({ "sheet_id": "d1", "title": "Plate A", "kind": "Column", "rows": 2, "cols": 1 }))
        .map_err(|e| e.to_string())?;
    builder.execute("table.setCells", json!({ "sheet_id": "d1", "edits": [{ "row": 0, "col": 0, "value": 99.0 }] })).map_err(|e| e.to_string())?;
    let save_a = builder.execute("project.save", json!({ "path": path_a.to_string_lossy(), "return_bytes": true })).map_err(|e| e.to_string())?;
    let bytes_b64 =
        save_a.payload.get("bytes_base64").and_then(Value::as_str).ok_or_else(|| String::from("missing bytes_base64 from project.save"))?.to_owned();
    builder.execute("project.saveTemplate", json!({ "path": path_tpl.to_string_lossy() })).map_err(|e| e.to_string())?;
    builder.execute("project.save", json!({ "path": path_b.to_string_lossy() })).map_err(|e| e.to_string())?;

    let mut session = Session::new();
    session.execute("project.open", json!({ "path": path_a.to_string_lossy() })).map_err(|e| e.to_string())?;
    if session.file_path() != Some(path_a.as_path()) {
        return Err(format!("expected file_path {:?} after project.open, got {:?}", path_a, session.file_path()));
    }

    // `project.new` clears file_path.
    session.execute("project.new", json!({ "title": "Fresh" })).map_err(|e| e.to_string())?;
    if session.file_path().is_some() {
        return Err(format!("expected file_path None after project.new, got {:?}", session.file_path()));
    }

    // `project.open` by path sets file_path to the newly opened path.
    session.execute("project.open", json!({ "path": path_b.to_string_lossy() })).map_err(|e| e.to_string())?;
    if session.file_path() != Some(path_b.as_path()) {
        return Err(format!("expected file_path {:?} after second project.open, got {:?}", path_b, session.file_path()));
    }

    // Failed `project.open` rolls back file_path cleanly.
    let nonexistent = dir.path().join("missing.lumencraft");
    if session.execute("project.open", json!({ "path": nonexistent.to_string_lossy() })).is_ok() {
        return Err(String::from("expected missing project.open to fail"));
    }
    if session.file_path() != Some(path_b.as_path()) {
        return Err(format!("expected file_path preserved after failed open, got {:?}", session.file_path()));
    }

    // `file.new` clears file_path.
    session.execute("file.new", json!({ "title": "File New" })).map_err(|e| e.to_string())?;
    if session.file_path().is_some() {
        return Err(format!("expected file_path None after file.new, got {:?}", session.file_path()));
    }

    // `project.open { bytes_base64 }` clears any prior file_path.
    session.execute("project.open", json!({ "path": path_a.to_string_lossy() })).map_err(|e| e.to_string())?;
    session.execute("project.open", json!({ "bytes_base64": bytes_b64 })).map_err(|e| e.to_string())?;
    if session.file_path().is_some() {
        return Err(format!("expected file_path None after project.open bytes_base64, got {:?}", session.file_path()));
    }

    // `project.instantiateTemplate` clears any prior file_path.
    session.execute("project.open", json!({ "path": path_a.to_string_lossy() })).map_err(|e| e.to_string())?;
    session
        .execute("project.instantiateTemplate", json!({ "path": path_tpl.to_string_lossy(), "title": "From Template" }))
        .map_err(|e| e.to_string())?;
    if session.file_path().is_some() {
        return Err(format!("expected file_path None after project.instantiateTemplate, got {:?}", session.file_path()));
    }

    // `table.importPzfx` with `merge: false` replaces the document -> clears file_path.
    // With `merge: true` -> merges into current document and keeps file_path.
    let pzfx_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <Table Sequence="tbl-xml" TableType="OneWay">
    <Title>Cytokine</Title>
    <YColumn Width="80" Subcolumns="1"><Title>Ctrl</Title><Subcolumn><d>11.0</d></Subcolumn></YColumn>
  </Table>
</PrismFile>"#;

    session.execute("project.open", json!({ "path": path_a.to_string_lossy() })).map_err(|e| e.to_string())?;
    session.execute("table.importPzfx", json!({ "xml": pzfx_xml, "merge": true })).map_err(|e| e.to_string())?;
    if session.file_path() != Some(path_a.as_path()) {
        return Err(format!("expected file_path preserved on merge=true importPzfx, got {:?}", session.file_path()));
    }

    session.execute("table.importPzfx", json!({ "xml": pzfx_xml, "merge": false })).map_err(|e| e.to_string())?;
    if session.file_path().is_some() {
        return Err(format!("expected file_path None on merge=false importPzfx, got {:?}", session.file_path()));
    }

    Ok(())
}
