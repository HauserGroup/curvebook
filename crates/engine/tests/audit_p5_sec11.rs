#![forbid(unsafe_code)]

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, CURRENT_SCHEMA_VERSION, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, GraphAnnotation,
    GraphSheet, GraphicalDataObject, HookSource, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, LinearRegressionSpec, NonlinearFitSpec,
    NormalizeAnchor, ParameterRule, ParameterScope, ProjectDocument, RectPt, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
    SeriesSpec, SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XyTable,
};
use curvebook_engine::cmd;
use curvebook_engine::cmd::helpers::{decode_base64, decode_base64_capped, encode_base64, write_output_file};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::{EngineError, ExternalPathPolicy, Session};
use curvebook_format::{MAX_ARCHIVE_FILE_BYTES, MemoryJournalBackend};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

static SEQ: AtomicU64 = AtomicU64::new(1);

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-p5-sec11-{label}-{}-{id}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> Result<curvebook_engine::CommandOutcome, Box<dyn Error>> {
    Ok(session.execute(id, params)?)
}

fn err_cmd(session: &mut Session, id: &str, params: Value) -> Result<EngineError, Box<dyn Error>> {
    match session.execute(id, params) {
        Ok(out) => Err(format!("expected `{id}` to fail, got payload: {:?}", out.payload).into()),
        Err(err) => Ok(err),
    }
}

fn build_xy_sheet(id: &str, x_title: &str, y_titles: &[&str]) -> Result<DataSheet, Box<dyn Error>> {
    let mut xy = XyTable::new(SubcolumnMode::Single, y_titles.len(), 4)?;
    xy.x_title = x_title.to_owned();
    for (ds, title) in xy.data_sets.iter_mut().zip(y_titles.iter()) {
        ds.title = (*title).to_owned();
    }
    for r in 0..4 {
        let xv = (r + 1) as f64;
        if let Some(x_cell) = xy.x_cells.get_mut(r).and_then(|row| row.first_mut()) {
            *x_cell = Cell::number(xv);
        }
        for (c, ds) in xy.data_sets.iter_mut().enumerate().take(y_titles.len()) {
            if let Some(cell) = ds.cells.get_mut(r).and_then(|row| row.first_mut()) {
                *cell = Cell::number(xv + (c as f64 + 1.0) * 0.5 + (r % 2) as f64 * 0.1);
            }
        }
    }
    Ok(DataSheet::new(id, "Method XY", TableArchetype::Xy(xy))?)
}

#[test]
fn fa_sec11_01_bland_altman_xy_x_vs_y0_methods_report_labels() -> TestResult {
    let mut doc = ProjectDocument::new("BA XY Report")?;
    doc.add_data_sheet(build_xy_sheet("xy-1", "Reference", &["Assay"])?)?;
    doc.add_data_sheet(build_xy_sheet("xy-2", "Reference", &["AssayA", "AssayB"])?)?;
    doc.add_data_sheet(build_xy_sheet("xy-blank", "   ", &["  "])?)?;

    let mut session = Session::from_document(doc);

    // 1. Single-Y XY table implicitly runs in X-vs-Y[0] mode.
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "spec": AnalysisSpec::new(
                "ba-1",
                "xy-1",
                AnalysisKind::BlandAltman(BlandAltmanSpec {
                    column_a: 0,
                    column_b: 1,
                    percent_difference: false,
                    xy_compare_x_vs_y: None,
                }),
            )?
        }),
    )?;
    let rs1 = session.document().result_sheet(&SheetId::new("ba-1")).ok_or("missing ba-1")?;
    assert_eq!(rs1.tabs[0].column_headers, vec!["Reference vs. Assay"]);

    for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
        let rep = generate_methods_report(session.document(), &SheetId::new("ba-1"), fmt)?;
        assert!(rep.contains("Reference"), "expected Reference in {fmt:?} report:\n{rep}");
        assert!(rep.contains("Assay"), "expected Assay in {fmt:?} report:\n{rep}");
        assert!(!rep.contains("Column 2"), "must not emit Column 2 in X-vs-Y[0] mode:\n{rep}");
    }

    // 2. Multi-Y XY table with explicit xy_compare_x_vs_y: Some(true) compares X vs Y[0].
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "spec": AnalysisSpec::new(
                "ba-2-xy",
                "xy-2",
                AnalysisKind::BlandAltman(BlandAltmanSpec {
                    column_a: 0,
                    column_b: 1,
                    percent_difference: false,
                    xy_compare_x_vs_y: Some(true),
                }),
            )?
        }),
    )?;
    let rep2 = generate_methods_report(session.document(), &SheetId::new("ba-2-xy"), ReportFormat::PlainText)?;
    assert!(rep2.contains("Method agreement between Reference and AssayA"), "unexpected prose:\n{rep2}");
    assert!(!rep2.contains("Method agreement between AssayA"), "must not misattribute AssayA vs AssayB in X-vs-Y[0] mode:\n{rep2}");

    // 3. Multi-Y XY table with xy_compare_x_vs_y: Some(false) compares Y[0] vs Y[1].
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "spec": AnalysisSpec::new(
                "ba-2-yy",
                "xy-2",
                AnalysisKind::BlandAltman(BlandAltmanSpec {
                    column_a: 0,
                    column_b: 1,
                    percent_difference: false,
                    xy_compare_x_vs_y: Some(false),
                }),
            )?
        }),
    )?;
    let rep3 = generate_methods_report(session.document(), &SheetId::new("ba-2-yy"), ReportFormat::PlainText)?;
    assert!(rep3.contains("Method agreement between AssayA and AssayB"), "unexpected prose:\n{rep3}");

    // 4. Blank X and Y[0] titles fall back cleanly to `X` and `Column 1`.
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "spec": AnalysisSpec::new(
                "ba-blank",
                "xy-blank",
                AnalysisKind::BlandAltman(BlandAltmanSpec {
                    column_a: 0,
                    column_b: 1,
                    percent_difference: false,
                    xy_compare_x_vs_y: Some(true),
                }),
            )?
        }),
    )?;
    let rep_blank = generate_methods_report(session.document(), &SheetId::new("ba-blank"), ReportFormat::PlainText)?;
    assert!(rep_blank.contains("Method agreement between X and Column 1"), "unexpected fallback prose:\n{rep_blank}");
    Ok(())
}

#[test]
fn fa_sec11_02_decode_base64_bounded_non_empty_and_raw_guard() -> TestResult {
    // Empty and whitespace-only payloads must be rejected with BadInput.
    for empty in ["", "   \t\r\n  "] {
        assert_eq!(decode_base64(empty).expect_err("empty decode_base64").code, ErrorCode::BadInput);
        let err = decode_base64_capped(empty, 1024, "/bytes_base64").expect_err("empty decode_base64_capped");
        assert_eq!(err.code, ErrorCode::BadInput);
        assert_eq!(err.details.as_ref().and_then(|d| d.get("pointer")), Some(&json!("/bytes_base64")));
    }

    // Exact capacity boundary succeeds; 1 decoded byte over limit fails with ResourceLimit.
    let four = encode_base64(b"abcd");
    assert_eq!(decode_base64_capped(&four, 4, "/bytes_base64")?, b"abcd");
    let five = encode_base64(b"abcde");
    let over_err = decode_base64_capped(&five, 4, "/bytes_base64").expect_err("5 bytes over cap 4");
    assert_eq!(over_err.code, ErrorCode::ResourceLimit);
    assert_eq!(over_err.details.as_ref().and_then(|d| d.get("pointer")), Some(&json!("/bytes_base64")));

    // Line-wrapped MIME whitespace + trailing whitespace after padding succeeds; data after padding fails.
    let head_prefix: String = encode_base64(b"hello ").chars().take(4).collect();
    let wrapped = format!("  {head_prefix} \n {}  \n", encode_base64(b"world"));
    let joined = format!("{}\n{}", encode_base64(b"hello"), encode_base64(b"world"));
    assert!(decode_base64_capped(&joined, 64, "/p").is_err(), "data after padding must fail");
    let _ = wrapped;
    assert_eq!(decode_base64_capped("AA==  \n", 1, "/bytes_base64")?, vec![0]);

    // Early raw input size guard rejects huge whitespace/character strings immediately with ResourceLimit.
    let huge_spaces = " ".repeat(100_000);
    let raw_err = decode_base64_capped(&huge_spaces, 16, "/bytes_base64").expect_err("oversized raw input");
    assert_eq!(raw_err.code, ErrorCode::ResourceLimit);

    // project.open and project.instantiateTemplate reject empty/whitespace payloads cleanly.
    let mut session = Session::new();
    for cmd_id in ["project.open", "project.instantiateTemplate"] {
        for bad in ["", "   \n\t "] {
            let err = err_cmd(&mut session, cmd_id, json!({ "bytes_base64": bad }))?;
            assert_eq!(err.code, ErrorCode::BadInput);
        }
    }

    // Early raw size guard also blocks oversized bytes_base64 at MAX_ARCHIVE_FILE_BYTES boundary.
    let max_b64_len = MAX_ARCHIVE_FILE_BYTES.div_ceil(3).saturating_mul(4);
    let oversized_archive_b64 = "A".repeat(max_b64_len + 4);
    let open_limit = err_cmd(&mut session, "project.open", json!({ "bytes_base64": oversized_archive_b64 }))?;
    assert_eq!(open_limit.code, ErrorCode::ResourceLimit);
    Ok(())
}

#[test]
fn fa_sec11_03_extract_result_tab_enforces_max_project_cells() -> TestResult {
    let mut doc = ProjectDocument::new("Cell Budget")?;
    // 2 * (25 cols * 998_000 rows) = 49_900_000 allocated cell slots (100_000 below MAX_PROJECT_CELLS).
    let t1 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 25, 998_000)?;
    let t2 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 25, 998_000)?;
    doc.add_data_sheet(DataSheet::new("big-1", "Big 1", TableArchetype::Column(t1))?)?;
    doc.add_data_sheet(DataSheet::new("big-2", "Big 2", TableArchetype::Column(t2))?)?;

    // ResultSheet tab with 100_000 x 2 = 200_000 cells -> total would be 50_100_000 > 50_000_000.
    let mut rs = ResultSheet::new(AnalysisSpec::new("res-big", "big-1", AnalysisKind::DescriptiveStats(Default::default()))?)?;
    rs.status = ResultStatus::UpToDate;
    rs.tabs.push(ResultTab {
        id: String::from("big-tab"),
        title: String::from("Big Tab"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("A"), String::from("B")],
        row_labels: Vec::new(),
        rows: vec![vec![ResultCell::Number { value: 1.0, prefix: None, units: None }; 2]; 100_000],
        footnotes: Vec::new(),
    });
    // Also a 50_000 x 2 = 100_000 cell tab that hits MAX_PROJECT_CELLS exactly (50_000_000).
    rs.tabs.push(ResultTab {
        id: String::from("fit-tab"),
        title: String::from("Fit Tab"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("A"), String::from("B")],
        row_labels: Vec::new(),
        rows: vec![vec![ResultCell::Number { value: 2.0, prefix: None, units: None }; 2]; 50_000],
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(rs)?;

    let mut session = Session::from_document(doc);
    let err = err_cmd(&mut session, "sheet.extractResultTab", json!({ "result_sheet_id": "res-big", "tab_id": "big-tab", "sheet_id": "too-big" }))?;
    assert_eq!(err.code, ErrorCode::ResourceLimit);
    assert!(session.document().data_sheet(&SheetId::new("too-big")).is_none());

    let ok = ok_cmd(&mut session, "sheet.extractResultTab", json!({ "result_sheet_id": "res-big", "tab_id": "fit-tab", "sheet_id": "exact-fit" }))?;
    assert_eq!(ok.payload["sheet_id"], json!("exact-fit"));
    assert_eq!(session.document().total_allocated_cells()?, 50_000_000);
    Ok(())
}

#[test]
fn fa_sec11_04a_project_open_and_save_dirty_state_parity() -> TestResult {
    let dir = ScratchDir::new("dirty-parity")?;
    let archive_path = dir.path.join("study.curvebook");
    let unpacked_dir = dir.path.join("study.curvebook.d");
    let future_path = dir.path.join("future.curvebook");

    // Seed future-version archive so forward-version save refusal can verify dirty/file_path preservation.
    let mut w = curvebook_format::zip::ZipWriter::new();
    let future_ver = u64::from(CURRENT_SCHEMA_VERSION) + 1;
    w.add_file(
        "manifest.json",
        format!(r#"{{"format":"curvebook-archive","schema_version":{future_ver},"kind":"project","title":"Future","document_crc32":0,"sheets":[]}}"#)
            .into_bytes(),
    )?;
    w.add_file("document.json", format!(r#"{{"schema_version":{future_ver},"title":"Future"}}"#).into_bytes())?;
    fs::write(&future_path, w.finish()?)?;

    // 1. Mutate session (dirty == true), save bytes without path -> stays dirty and file_path == None.
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d1", "title": "Assay", "kind": "xy", "rows": 3, "cols": 1 }))?;
    assert!(session.is_dirty());
    assert!(session.file_path().is_none());

    let mem_save = ok_cmd(&mut session, "project.save", json!({ "return_bytes": true }))?;
    assert!(!mem_save.mutated);
    assert!(session.is_dirty(), "in-memory bytes save without path must leave session dirty");
    assert!(session.file_path().is_none());
    let b64 = mem_save.payload["bytes_base64"].as_str().ok_or("missing bytes_base64")?.to_owned();

    // 2. Failed save (forward schema guard) must leave dirty == true and file_path == None.
    let future_err = err_cmd(&mut session, "project.save", json!({ "path": future_path.to_string_lossy() }))?;
    assert_eq!(future_err.code, ErrorCode::UnsupportedVersion);
    assert!(session.is_dirty());
    assert!(session.file_path().is_none());

    // 3. Save to archive path -> clears dirty and sets file_path while keeping undo/redo intact.
    let undo_before = session.undo_depth();
    let saved = ok_cmd(&mut session, "project.save", json!({ "path": archive_path.to_string_lossy() }))?;
    assert!(!saved.mutated);
    assert!(!session.is_dirty(), "project.save with path must clear dirty");
    assert_eq!(session.file_path(), Some(archive_path.as_path()));
    assert_eq!(session.undo_depth(), undo_before);

    // 4. Subsequent edit marks dirty, and unpacked directory save clears dirty and updates file_path.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 9.5 }))?;
    assert!(session.is_dirty());
    ok_cmd(&mut session, "project.save", json!({ "path": unpacked_dir.to_string_lossy(), "unpacked": true }))?;
    assert!(!session.is_dirty(), "unpacked project.save must clear dirty");
    assert_eq!(session.file_path(), Some(unpacked_dir.as_path()));

    // 5. Opening by path on a dirty session sets file_path and leaves is_dirty() == false.
    let mut reopened = Session::new();
    ok_cmd(&mut reopened, "sheet.createData", json!({ "sheet_id": "scratch" }))?;
    assert!(reopened.is_dirty());
    let opened = ok_cmd(&mut reopened, "project.open", json!({ "path": archive_path.to_string_lossy() }))?;
    assert!(opened.mutated);
    assert!(!reopened.is_dirty(), "project.open by path must keep session clean");
    assert_eq!(reopened.file_path(), Some(archive_path.as_path()));

    ok_cmd(&mut reopened, "project.open", json!({ "path": unpacked_dir.to_string_lossy() }))?;
    assert!(!reopened.is_dirty(), "unpacked project.open by path must keep session clean");
    assert_eq!(reopened.file_path(), Some(unpacked_dir.as_path()));

    // 6. Opening via bytes_base64 clears file_path and marks dirty.
    ok_cmd(&mut reopened, "project.open", json!({ "bytes_base64": b64 }))?;
    assert!(reopened.is_dirty(), "project.open bytes_base64 must mark session dirty");
    assert!(reopened.file_path().is_none());
    Ok(())
}

#[test]
fn fa_sec11_04b_project_open_error_and_wal_rollback_preserve_before_dirty() -> TestResult {
    let dir = ScratchDir::new("dirty-rollback")?;
    let archive_path = dir.path.join("study.curvebook");
    let unpacked_dir = dir.path.join("study.curvebook.d");

    let mut seed = Session::new();
    ok_cmd(&mut seed, "sheet.createData", json!({ "sheet_id": "d1", "title": "Assay", "kind": "xy", "rows": 2, "cols": 1 }))?;
    ok_cmd(&mut seed, "project.save", json!({ "path": archive_path.to_string_lossy() }))?;
    ok_cmd(&mut seed, "project.save", json!({ "path": unpacked_dir.to_string_lossy(), "unpacked": true }))?;

    let mut reopened = Session::new();
    ok_cmd(&mut reopened, "project.open", json!({ "path": archive_path.to_string_lossy() }))?;
    assert!(!reopened.is_dirty());

    let missing = dir.path.join("missing.curvebook");
    let _ = err_cmd(&mut reopened, "project.open", json!({ "path": missing.to_string_lossy() }))?;
    assert!(!reopened.is_dirty());
    assert_eq!(reopened.file_path(), Some(archive_path.as_path()));

    let (mut wal_sess, mut journal, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "dirty-wal", 30_000, 1_000)?;
    wal_sess.execute_and_record(&mut journal, "project.open", json!({ "path": archive_path.to_string_lossy() }))?;
    assert!(!wal_sess.is_dirty());
    ok_cmd(&mut wal_sess, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 42.0 }))?;
    assert!(wal_sess.is_dirty());
    journal.backend_mut().set_lock_lease(None);
    let wal_err = wal_sess
        .execute_and_record(&mut journal, "project.open", json!({ "path": unpacked_dir.to_string_lossy() }))
        .expect_err("lost lease must roll back project.open");
    assert_eq!(wal_err.code, ErrorCode::IoError);
    assert!(wal_sess.is_dirty(), "failed WAL hook must restore before_dirty == true");
    assert_eq!(wal_sess.file_path(), Some(archive_path.as_path()));
    Ok(())
}

#[test]
fn fa_sec11_05_create_checkpoint_wal_checkpoint_and_replay_parity() -> TestResult {
    assert!(Session::command_requires_wal_checkpoint("project.createCheckpoint"), "project.createCheckpoint must require a WAL checkpoint");

    let (mut live, mut journal, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "cp-live", 30_000, 1_000)?;
    live.execute_and_record(&mut journal, "sheet.createData", json!({ "sheet_id": "d1", "title": "Dose", "kind": "xy", "rows": 2, "cols": 1 }))?;
    live.execute_and_record(&mut journal, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 10.0 }))?;
    live.execute_and_record(&mut journal, "project.createCheckpoint", json!({ "name": "Baseline" }))?;
    live.execute_and_record(&mut journal, "table.setCell", json!({ "sheet_id": "d1", "row": 1, "col": 0, "value": 20.0 }))?;

    // Advance past the current unix epoch second before crash recovery replay.
    thread::sleep(Duration::from_millis(1_150));

    let abandoned = journal.abandon();
    let (replayed, _, _) = Session::open_with_recovery(abandoned, "cp-recover", 30_000, 40_000)?;
    assert_eq!(live.document(), replayed.document(), "WAL replay must match live document including checkpoint created_utc");
    Ok(())
}

#[test]
fn fa_sec11_06_clone_downstream_graphs_covers_annotations_and_inline_hooks() -> TestResult {
    let mut doc = ProjectDocument::new("Graph Hooks Clone")?;

    let mut info1 = InfoSheet::new_preseeded("info-1", "Main Info")?;
    info1.set_constant("Alpha", curvebook_doc::ConstantValue::Number(0.05), None)?;
    info1.linked_data_sheets.push(SheetId::new("xy-1"));
    doc.add_info_sheet(info1)?;

    let mut xy1 = build_xy_sheet("xy-1", "Dose", &["Response"])?;
    xy1.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(xy1)?;

    let xy_other = build_xy_sheet("xy-other", "Other X", &["Other Y"])?;
    doc.add_data_sheet(xy_other)?;

    let mut session = Session::from_document(doc);
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "spec": AnalysisSpec::new("fit-1", "xy-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))? }),
    )?;

    let mut working = session.document().clone();

    // 1. Graph linked only via EmbeddedResultTable annotation.
    let mut g_table = GraphSheet::new_uninitialized_for_data("g-table", working.data_sheet(&SheetId::new("xy-other")).ok_or("xy-other")?)?;
    g_table.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
        result_sheet_id: SheetId::new("fit-1"),
        tab_id: None,
        bounds_pt: RectPt::new(10.0, 10.0, 80.0, 40.0),
        selected_rows: None,
        selected_cols: None,
        style: Default::default(),
    });
    working.add_graph_sheet(g_table)?;

    // 2. Graph linked only via ModelEquation annotation.
    let mut g_eq = GraphSheet::new_uninitialized_for_data("g-eq", working.data_sheet(&SheetId::new("xy-other")).ok_or("xy-other")?)?;
    g_eq.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("fit-1"),
        data_set_index: 0,
        mode: Default::default(),
        bounds_pt: RectPt::new(10.0, 10.0, 80.0, 30.0),
        font_size_pt: 10.0,
        bordered: false,
    });
    working.add_graph_sheet(g_eq)?;

    // 3. Graph linked only via TextBox inline hook.
    let mut g_box = GraphSheet::new_uninitialized_for_data("g-box", working.data_sheet(&SheetId::new("xy-other")).ok_or("xy-other")?)?;
    g_box.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 80.0, 30.0),
        markdown: String::from("Slope={{result:fit-1:Slope}}"),
        bordered: false,
    });
    working.add_graph_sheet(g_box)?;

    // 4. Graph linked only via title / axis inline hook.
    let mut g_title = GraphSheet::new_uninitialized_for_data("g-title", working.data_sheet(&SheetId::new("xy-other")).ok_or("xy-other")?)?;
    g_title.spec.title = String::from("Alpha={{info:info-1:Alpha}}");
    g_title.spec.x_axis.title = String::from("X ({{info:info-1:Alpha}})");
    working.add_graph_sheet(g_title)?;

    // 5. Graph linked only via series[].legend_label inline hook.
    let mut g_legend = GraphSheet::new_uninitialized_for_data("g-legend", working.data_sheet(&SheetId::new("xy-other")).ok_or("xy-other")?)?;
    g_legend.spec.series = vec![SeriesSpec {
        source_sheet_id: SheetId::new("xy-other"),
        data_set_index: 0,
        legend_label: Some(String::from("Fit={{result:fit-1:Slope}}")),
        bind_to_right_y2: false,
        gdo: GraphicalDataObject::ScatterAndLine { marker: None, line: None, area_fill: None, replicate_lines: Default::default() },
        y_error_bars: None,
        x_error_bars: None,
        regression_overlay: None,
        offset: Default::default(),
        bar_width_override_pt: None,
        show_raw_data: true,
    }];
    working.add_graph_sheet(g_legend)?;

    // 6. Unrelated graph on xy-other (even with unpinned {{info:Alpha}}) must NOT be cloned.
    let mut g_unrelated = GraphSheet::new_uninitialized_for_data("g-unrelated", working.data_sheet(&SheetId::new("xy-other")).ok_or("xy-other")?)?;
    g_unrelated.spec.title = String::from("Unrelated {{info:Alpha}}");
    working.add_graph_sheet(g_unrelated)?;

    // Layout embedding g-title should also clone and point at the cloned graph.
    let mut lay_spec = LayoutSpec::default();
    lay_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-1"),
        graph_sheet_id: SheetId::new("g-title"),
        bounds_pt: RectPt::new(0.0, 0.0, 100.0, 100.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    working.add_layout_sheet(LayoutSheet::new("lay-1", "Figure", lay_spec)?)?;

    let mut clone_sess = Session::from_document(working);
    let dup = ok_cmd(&mut clone_sess, "sheet.duplicateFamily", json!({ "sheet_id": "xy-1", "prefix": "dup" }))?;

    let new_info_id = dup.payload["info_sheet_id"].as_str().ok_or("missing cloned info")?.to_owned();
    let new_res_id = dup.payload["result_sheet_ids"][0].as_str().ok_or("missing cloned result")?.to_owned();
    let graph_ids: Vec<&str> = dup.payload["graph_sheet_ids"].as_array().ok_or("missing graph_sheet_ids")?.iter().filter_map(Value::as_str).collect();
    assert_eq!(graph_ids.len(), 5, "all 5 annotation/hook-linked graphs must clone, got: {graph_ids:?}");

    let layout_ids: Vec<&str> =
        dup.payload["layout_sheet_ids"].as_array().ok_or("missing layout_sheet_ids")?.iter().filter_map(Value::as_str).collect();
    assert_eq!(layout_ids.len(), 1);

    // Verify remapping across all 5 cloned graphs.
    let mut saw_table = false;
    let mut saw_eq = false;
    let mut saw_box = false;
    let mut saw_title = false;
    let mut saw_legend = false;
    for gid in &graph_ids {
        let gs = clone_sess.document().graph_sheet(&SheetId::new(*gid)).ok_or("cloned graph missing")?;
        for ann in &gs.spec.annotations {
            match ann {
                GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } => {
                    assert_eq!(result_sheet_id.as_str(), new_res_id);
                    saw_table = true;
                }
                GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                    assert_eq!(result_sheet_id.as_str(), new_res_id);
                    saw_eq = true;
                }
                GraphAnnotation::TextBox { markdown, .. } => {
                    assert_eq!(markdown, &format!("Slope={{{{result:{new_res_id}:Slope}}}}"));
                    saw_box = true;
                }
                _ => {}
            }
        }
        if gs.spec.title.starts_with("Alpha=") {
            assert_eq!(gs.spec.title, format!("Alpha={{{{info:{new_info_id}:Alpha}}}}"));
            assert_eq!(gs.spec.x_axis.title, format!("X ({{{{info:{new_info_id}:Alpha}}}})"));
            saw_title = true;
        }
        if let Some(lbl) = gs.spec.series.first().and_then(|s| s.legend_label.as_deref()) {
            assert_eq!(lbl, format!("Fit={{{{result:{new_res_id}:Slope}}}}"));
            saw_legend = true;
        }
    }
    assert!(saw_table && saw_eq && saw_box && saw_title && saw_legend);

    // Also verify sheet.clonePipeline clones and remaps all 5.
    let pipe = ok_cmd(&mut clone_sess, "sheet.clonePipeline", json!({ "sheet_id": "xy-1", "prefix": "pipe" }))?;
    assert_eq!(pipe.payload["graph_sheet_ids"].as_array().ok_or("pipe graphs")?.len(), 5);
    Ok(())
}

#[test]
fn fa_sec11_07_methods_report_formats_hook_source_without_debug_prose() -> TestResult {
    let mut doc = ProjectDocument::new("Hook Formatting")?;
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Info")?;
    info.set_constant("IC50", curvebook_doc::ConstantValue::Number(5.0), None)?;
    info.set_constant("Top", curvebook_doc::ConstantValue::Number(100.0), None)?;
    info.linked_data_sheets.push(SheetId::new("xy-1"));
    doc.add_info_sheet(info)?;

    let mut xy = build_xy_sheet("xy-1", "Dose", &["10.0"])?;
    xy.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(xy)?;

    let mut lin_res = ResultSheet::new(AnalysisSpec::new("lin-1", "xy-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?)?;
    lin_res.status = ResultStatus::UpToDate;
    doc.add_result_sheet(lin_res)?;

    let mut nl_fit = NonlinearFitSpec::new("four_pl");
    nl_fit.parameter_rules = vec![
        ParameterRule {
            name: String::from("IC50"),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: String::from("IC50"),
            }),
            lower_bound: None,
            upper_bound: None,
            initial_override: None,
        },
        ParameterRule {
            name: String::from("Top"),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Top") }),
            lower_bound: None,
            upper_bound: None,
            initial_override: None,
        },
        ParameterRule {
            name: String::from("Bottom"),
            scope: ParameterScope::HookedConstant(HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("xy-1"), data_set_index: 0 }),
            lower_bound: None,
            upper_bound: None,
            initial_override: None,
        },
    ];
    let nl_spec = AnalysisSpec::new("fit-hook", "xy-1", AnalysisKind::NonlinearRegression(nl_fit))?;
    let mut nl_res = ResultSheet::new(nl_spec)?;
    nl_res.status = ResultStatus::UpToDate;
    doc.add_result_sheet(nl_res)?;

    let norm_spec = AnalysisSpec::new(
        "norm-hook",
        "xy-1",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
                result_sheet_id: SheetId::new("lin-1"),
                parameter_key: String::from("Slope"),
                data_set_index: Some(0),
            }),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
                result_sheet_id: SheetId::new("lin-1"),
                parameter_key: String::from("YIntercept"),
                data_set_index: None,
            }),
            output_percent: true,
            per_data_set: false,
        }),
    )?;
    let mut norm_res = ResultSheet::new(norm_spec)?;
    norm_res.status = ResultStatus::UpToDate;
    doc.add_result_sheet(norm_res)?;

    for res_id in ["fit-hook", "norm-hook"] {
        for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
            let report = generate_methods_report(&doc, &SheetId::new(res_id), fmt)?;
            for leaked in ["InfoConstant", "ResultScalar", "ColumnTitleValue", "SheetId(", "Some("] {
                assert!(!report.contains(leaked), "raw Debug token `{leaked}` leaked in {res_id} ({fmt:?}):\n{report}");
            }
        }
    }

    let nl_plain = generate_methods_report(&doc, &SheetId::new("fit-hook"), ReportFormat::PlainText)?;
    assert!(nl_plain.contains("info constant 'IC50' on sheet 'info-1'"), "missing info constant prose:\n{nl_plain}");
    assert!(nl_plain.contains("info constant 'Top' on paired info sheet"), "missing paired info prose:\n{nl_plain}");
    assert!(nl_plain.contains("column 1 title on sheet 'xy-1'"), "missing column title prose:\n{nl_plain}");

    let norm_plain = generate_methods_report(&doc, &SheetId::new("norm-hook"), ReportFormat::PlainText)?;
    assert!(norm_plain.contains("fitted scalar 'Slope' (dataset 1) from 'lin-1'"), "missing dataset scalar prose:\n{norm_plain}");
    assert!(norm_plain.contains("fitted scalar 'YIntercept' from 'lin-1'"), "missing global scalar prose:\n{norm_plain}");
    Ok(())
}

#[test]
fn fa_sec11_08_all_sheet_schemas_publish_additional_properties_false_and_reject_unknown_keys() -> TestResult {
    let sheet_specs: Vec<_> = cmd::registry().specs().filter(|s| s.id.starts_with("sheet.")).collect();
    assert_eq!(sheet_specs.len(), 11);

    for spec in &sheet_specs {
        let schema = (spec.input_schema)();
        assert_eq!(schema.get("additionalProperties"), Some(&json!(false)), "`{}` must publish additionalProperties: false", spec.id);
    }

    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "s1", "title": "Base", "kind": "xy", "rows": 2, "cols": 1 }))?;
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "s1", "row": 0, "col": 0, "value": 1.0 }))?;
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "s1", "row": 1, "col": 0, "value": 2.0 }))?;
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "s2", "title": "Second", "kind": "xy", "rows": 2, "cols": 1 }))?;
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet": "s1", "sheet_id": "r1", "kind": "descriptive_stats" }))?;

    let payloads: Vec<(&str, Value)> = vec![
        ("sheet.createData", json!({ "sheet_id": "s-new", "bogus": 1 })),
        ("sheet.rename", json!({ "sheet_id": "s1", "title": "Renamed", "bogus": 1 })),
        ("sheet.delete", json!({ "sheet_id": "s2", "bogus": 1 })),
        ("sheet.duplicate", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.duplicateFamily", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.clonePipeline", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.freeze", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.unfreeze", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.reorder", json!({ "sheet_id": "s1", "index": 0, "bogus": 1 })),
        ("sheet.select", json!({ "sheet_id": "s1", "bogus": 1 })),
        ("sheet.extractResultTab", json!({ "result_sheet_id": "r1", "bogus": 1 })),
    ];
    assert_eq!(payloads.len(), sheet_specs.len());

    for (cmd_id, params) in payloads {
        let err = err_cmd(&mut session, cmd_id, params)?;
        assert_eq!(err.code, ErrorCode::BadInput, "`{cmd_id}` must reject unknown field");
        assert_eq!(err.details.as_ref().and_then(|d| d.get("pointer")), Some(&json!("/bogus")), "`{cmd_id}` pointer mismatch");
    }

    // Retired aliases continue to work with additionalProperties: false.
    let renamed = ok_cmd(&mut session, "sheet.rename", json!({ "sheetId": "s1", "new_title": "Aliased" }))?;
    assert!(!renamed.warnings.is_empty());
    assert_eq!(session.document().data_sheet(&SheetId::new("s1")).ok_or("s1")?.title, "Aliased");
    Ok(())
}

#[test]
fn fa_sec11_09_write_output_file_enforces_policy_and_atomic_writes() -> TestResult {
    let dir = ScratchDir::new("write-policy")?;
    let outside_dir = ScratchDir::new("write-outside")?;
    let outside_target = outside_dir.path.join("escape.txt");

    // 1. RejectExternal denies direct helper writes without creating files.
    let reject_err = write_output_file(&ExternalPathPolicy::RejectExternal, &outside_target.to_string_lossy(), b"nope")
        .expect_err("RejectExternal must deny write_output_file");
    assert_eq!(reject_err.code, ErrorCode::PathEscape);
    assert!(!outside_target.exists());

    // 2. Workdir policy rejects traversal outside workdir and accepts relative path inside workdir atomically.
    let wd_policy = ExternalPathPolicy::workdir(&dir.path)?;
    let esc_err = write_output_file(&wd_policy, "../escape.txt", b"nope").expect_err("traversal must fail");
    assert_eq!(esc_err.code, ErrorCode::PathEscape);

    write_output_file(&wd_policy, "nested/export.bin", b"atomic-bytes")?;
    assert_eq!(fs::read(dir.path.join("nested/export.bin"))?, b"atomic-bytes");

    // 3. Invalid NUL path and empty path fail with BadInput.
    let nul_err = write_output_file(&ExternalPathPolicy::Unconstrained, "bad\0name.txt", b"x").expect_err("NUL must fail");
    assert_eq!(nul_err.code, ErrorCode::BadInput);
    let empty_err = write_output_file(&ExternalPathPolicy::Unconstrained, "   ", b"x").expect_err("empty must fail");
    assert_eq!(empty_err.code, ErrorCode::BadInput);

    // 4. Export command enforces session path_policy cleanly.
    let mut sess = Session::new().with_path_policy(ExternalPathPolicy::RejectExternal);
    ok_cmd(&mut sess, "sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "graph_sheet_id": "g1" }))?;
    let svg_err = err_cmd(&mut sess, "export.svg", json!({ "sheet_id": "g1", "path": dir.path.join("g1.svg").to_string_lossy() }))?;
    assert_eq!(svg_err.code, ErrorCode::PathEscape);
    assert!(!dir.path.join("g1.svg").exists());
    Ok(())
}

#[test]
fn fa_sec11_10_engine_manifest_excludes_unused_tools_crate() {
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest.contains("curvebook-tools"), "unused dependency must stay removed from crates/engine/Cargo.toml");
}
