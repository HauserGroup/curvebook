#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, CURRENT_SCHEMA_VERSION, Cell, CellValue, ConstantValue, FitMethod, GraphSheet, HookSource, InfoSheet, LayoutItem,
    LayoutSheet, LayoutSpec, NonlinearFitSpec, NormalizeAnchor, ParameterRule, ParameterScope, ProjectDocument, ResultSheet, SheetId, SubcolumnMode,
    TableArchetype, TransformAnalysisSpec, VariableRole, WeightingScheme,
};
use lumencraft_format::{
    CrashRecoveryJournal, CsvOptions, DirectoryJournalBackend, FormatError, JOURNAL_FILE_NAME, JournalBackend, JournalLockLease, JournalOpRecord,
    LOCKFILE_NAME, MemoryJournalBackend, SNAPSHOT_FILE_NAME, ZipWriter, detect_dependency_cycles, export_datasheet_to_csv, import_prism_bytes,
    import_pzfx_str, migrate_document_json, open_project_bytes, sanitize_circular_hooks, save_project_bytes, validate_project_document,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempRecoveryDir {
    path: PathBuf,
}

impl TempRecoveryDir {
    fn new(label: &str) -> Self {
        let id = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("lumencraft-format-{label}-{}-{id}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path).expect("create temp recovery dir");
        Self { path }
    }
}

impl Drop for TempRecoveryDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn directory_journal_clean_close_checkpoint_and_reopen() {
    let dir = TempRecoveryDir::new("clean-close");
    let backend = DirectoryJournalBackend::open_dir(&dir.path).expect("open recovery dir");

    let (mut journal, report) = CrashRecoveryJournal::open_or_recover(backend, "session-alpha", 10_000, 1_000).expect("open clean journal");
    assert!(report.snapshot.is_none());
    assert!(report.recovered_commands.is_empty());
    assert_eq!(report.discarded_corrupt_tail_lines, 0);
    assert!(report.recovered_from_stale_lock.is_none());
    assert_eq!(report.active_lease.owner_id, "session-alpha");
    assert!(dir.path.join(LOCKFILE_NAME).exists());

    let seq1 = journal.record_command(serde_json::json!({ "op": "set_cell", "row": 0, "col": 0, "val": 12.5 })).expect("record cmd 1");
    let seq2 = journal.record_command(serde_json::json!({ "op": "set_cell", "row": 1, "col": 0, "val": 25.0 })).expect("record cmd 2");
    assert_eq!((seq1, seq2), (1, 2));
    assert!(dir.path.join(JOURNAL_FILE_NAME).exists());

    journal.checkpoint(b"SNAPSHOT-CHECKPOINT-1").expect("checkpoint");
    assert_eq!(fs::read(dir.path.join(SNAPSHOT_FILE_NAME)).expect("snapshot bytes"), b"SNAPSHOT-CHECKPOINT-1");
    assert!(!dir.path.join(JOURNAL_FILE_NAME).exists());

    let seq3 = journal.record_command(serde_json::json!({ "op": "rename_sheet", "title": "Assay" })).expect("record cmd 3");
    assert_eq!(seq3, 3);
    assert!(dir.path.join(JOURNAL_FILE_NAME).exists());

    let backend_after = journal.close_cleanly(b"SNAPSHOT-FINAL").expect("close cleanly");
    assert!(!dir.path.join(JOURNAL_FILE_NAME).exists());
    assert!(!dir.path.join(LOCKFILE_NAME).exists());
    assert_eq!(backend_after.read_snapshot().expect("read snapshot").as_deref(), Some(b"SNAPSHOT-FINAL".as_slice()));

    let (reopened, reopen_report) = CrashRecoveryJournal::open_or_recover(backend_after, "session-beta", 10_000, 2_000).expect("reopen clean");
    assert_eq!(reopen_report.snapshot.as_deref(), Some(b"SNAPSHOT-FINAL".as_slice()));
    assert!(reopen_report.recovered_commands.is_empty());
    assert_eq!(reopen_report.discarded_corrupt_tail_lines, 0);
    assert!(reopen_report.recovered_from_stale_lock.is_none());
    assert_eq!(reopened.next_seq(), 1);
    let _ = reopened.close_cleanly(b"SNAPSHOT-FINAL").expect("close reopened");
}

#[test]
fn directory_journal_recovers_stale_lock_and_truncates_torn_tail_without_crashing() {
    let dir = TempRecoveryDir::new("crash-torn-tail");
    let backend = DirectoryJournalBackend::open_dir(&dir.path).expect("open dir");

    let (mut journal, _) = CrashRecoveryJournal::open_or_recover(backend, "writer-1", 5_000, 10_000).expect("open writer-1");
    journal.checkpoint(b"BASELINE-DOC-V1").expect("checkpoint baseline");

    let cmd1 = serde_json::json!({ "op": "insert_row", "sheet": "d1", "row": 0 });
    let cmd2 = serde_json::json!({ "op": "edit_cell", "sheet": "d1", "row": 0, "val": 3.25 });
    journal.record_command(cmd1.clone()).expect("cmd1");
    journal.record_command(cmd2.clone()).expect("cmd2");
    journal.renew_lease(11_000).expect("renew lease");
    assert_eq!(journal.lease().expires_epoch_ms(), 16_000);

    // Crash without close_cleanly, then append a torn incomplete JSON tail line (including invalid UTF-8 bytes).
    drop(journal);
    let mut raw = fs::read(dir.path.join(JOURNAL_FILE_NAME)).expect("read ops.journal");
    raw.extend_from_slice(b"{\"seq\":3,\"crc32\":123456,\"command_json\":{\"op\":\"partial\"");
    raw.push(0xFF);
    fs::write(dir.path.join(JOURNAL_FILE_NAME), &raw).expect("write torn tail");

    // While lease is still active (now = 15_999 < 16_000), another writer must be rejected cleanly.
    let active_contention =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("open dir"), "writer-2", 5_000, 15_999);
    assert!(active_contention.is_err());

    // Once lease expires (now = 16_000), writer-2 evicts stale lock, loads baseline, replays cmd1+cmd2, and drops torn tail.
    let (mut recovered, report) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("open dir"), "writer-2", 5_000, 16_000)
            .expect("recover stale lock");

    assert_eq!(report.snapshot.as_deref(), Some(b"BASELINE-DOC-V1".as_slice()));
    assert_eq!(report.recovered_commands, vec![cmd1.clone(), cmd2.clone()]);
    assert_eq!(report.recovered_seqs, vec![1, 2]);
    assert_eq!(report.discarded_corrupt_tail_lines, 1);
    assert_eq!(
        report.recovered_from_stale_lock,
        Some(JournalLockLease {
            format_version: lumencraft_format::CURRENT_LOCK_VERSION,
            owner_id: String::from("writer-1"),
            acquired_epoch_ms: 11_000,
            ttl_ms: 5_000,
        })
    );
    assert_eq!(report.active_lease.owner_id, "writer-2");
    assert_eq!(recovered.next_seq(), 3);

    // New commands append cleanly after the truncated tail line.
    let cmd3 = serde_json::json!({ "op": "post_recovery_edit", "ok": true });
    let seq3 = recovered.record_command(cmd3.clone()).expect("append cmd3");
    assert_eq!(seq3, 3);
    drop(recovered);

    let (reopened_again, report2) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("open dir"), "writer-3", 5_000, 22_000)
            .expect("reopen cleanly");
    assert_eq!(report2.recovered_commands, vec![cmd1, cmd2, cmd3]);
    assert_eq!(report2.discarded_corrupt_tail_lines, 0);
    let _ = reopened_again.close_cleanly(b"DONE").expect("close");
}

#[test]
fn directory_journal_detects_mid_journal_crc_corruption_without_leaving_phantom_lock() {
    let dir = TempRecoveryDir::new("crc-corrupt");
    let backend = DirectoryJournalBackend::open_dir(&dir.path).expect("open dir");
    let (mut journal, _) = CrashRecoveryJournal::open_or_recover(backend, "w1", 1_000, 0).expect("open");
    journal.record_command(serde_json::json!({ "step": 1, "val": 10 })).expect("cmd 1");
    journal.record_command(serde_json::json!({ "step": 2, "val": 20 })).expect("cmd 2");
    drop(journal);

    // Flip step 1 payload while leaving its old CRC32 intact (mid-journal tampering, NOT torn tail).
    let text = fs::read_to_string(dir.path.join(JOURNAL_FILE_NAME)).expect("read journal");
    let tampered = text.replacen("\"val\":10", "\"val\":99", 1);
    assert_ne!(text, tampered);
    fs::write(dir.path.join(JOURNAL_FILE_NAME), tampered).expect("write tampered");

    let err = CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("open"), "w2", 1_000, 5_000)
        .expect_err("mid-journal CRC mismatch must fail");
    assert!(matches!(err, FormatError::CorruptCrc { .. }));
    assert!(!dir.path.join(LOCKFILE_NAME).exists());
}

#[test]
fn memory_journal_backend_web_storage_json_snapshot_and_recovery_parity() {
    let lease = JournalLockLease::new("browser-tab-1", 1_000, 4_000).expect("lease");
    assert_eq!(lease.expires_epoch_ms(), 5_000);
    assert!(!lease.is_stale(4_999));
    assert!(lease.can_recover(5_000));

    let (mut journal, _) = CrashRecoveryJournal::open_or_recover(MemoryJournalBackend::new(), "browser-tab-1", 4_000, 1_000).expect("open memory");
    journal.checkpoint(b"WEB-SNAPSHOT-V1").expect("checkpoint");
    let c1 = serde_json::json!({ "action": "add_point", "x": 1.0, "y": 2.5 });
    let c2 = serde_json::json!({ "action": "exclude_cell", "row": 0, "col": 1 });
    journal.record_command(c1.clone()).expect("c1");
    journal.record_command(c2.clone()).expect("c2");

    // Simulate browser tab crash leaving a truncated NDJSON tail in localStorage.
    let mut crashed_backend = journal.abandon();
    crashed_backend.append_raw_journal_bytes(b"{\"seq\":3,\"crc32\":42\n");
    let stored_json = crashed_backend.to_web_storage_json().expect("export web storage json");

    let restored_backend = MemoryJournalBackend::from_web_storage_json(&stored_json).expect("import web storage json");
    assert_eq!(restored_backend.snapshot_bytes(), Some(b"WEB-SNAPSHOT-V1".as_slice()));

    // Non-expired lease blocks concurrent tab-2.
    let blocked = CrashRecoveryJournal::open_or_recover(restored_backend.clone(), "browser-tab-2", 4_000, 4_999);
    assert!(blocked.is_err());

    // After TTL expires, tab-2 recovers cleanly.
    let (reopened, report) = CrashRecoveryJournal::open_or_recover(restored_backend, "browser-tab-2", 4_000, 5_000).expect("recover web storage");
    assert_eq!(report.snapshot.as_deref(), Some(b"WEB-SNAPSHOT-V1".as_slice()));
    assert_eq!(report.recovered_commands, vec![c1, c2]);
    assert_eq!(report.discarded_corrupt_tail_lines, 1);
    assert_eq!(report.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str()), Some("browser-tab-1"));
    assert_eq!(JournalOpRecord::new(1, serde_json::json!({"ok": true})).expect("record").verify_crc(), Ok(()));

    let finalized = reopened.close_cleanly(b"WEB-FINAL").expect("close cleanly");
    assert!(finalized.lock_lease().is_none());
    assert!(finalized.journal_bytes().is_empty());
    let final_json = finalized.to_web_storage_json().expect("export final web json");
    let reloaded = MemoryJournalBackend::from_web_storage_json(&final_json).expect("reload final");
    assert_eq!(reloaded.snapshot_bytes(), Some(b"WEB-FINAL".as_slice()));
}

#[test]
fn migrate_document_json_fills_backwards_compatible_fields_and_rejects_future_schemas() {
    let legacy_v0 = serde_json::json!({
        "title": "Legacy Study",
        "data_sheets": [
            {
                "id": "d1",
                "title": "Dose",
                "archetype": {
                    "Column": {
                        "design_hint": "UnpairedReplicates",
                        "subcolumn_mode": "Single",
                        "row_titles": [],
                        "row_titles_expanded": false,
                        "data_sets": [{ "title": "Ctrl", "cells": [[{"value": {"Number": 1.0}, "excluded": false}]] }]
                    }
                }
            }
        ],
        "result_sheets": [
            {
                "id": "r1",
                "analysis": {
                    "id": "a1",
                    "source_sheet_id": "d1",
                    "selected_data_sets": [],
                    "auto_title": "Nonlinear fit",
                    "custom_title": null,
                    "confidence_level": 0.95,
                    "p_value_format": "FourDecimal",
                    "kind": {
                        "NonlinearRegression": {
                            "equation_id": "sigmoidal-4pl",
                            "custom_dsl": null,
                            "method": "LeastSquares",
                            "weighting": "None",
                            "parameter_rules": [],
                            "profile_likelihood_ci": false,
                            "comparison": null,
                            "confidence_bands": true,
                            "prediction_bands": false,
                            "interpolate_unknowns": false,
                            "replicates_lack_of_fit_test": false,
                            "normality_of_residuals": false,
                            "max_iters": 200
                        }
                    }
                },
                "status": "UpToDate",
                "tabs": [],
                "scalars": [],
                "pairwise_comparisons": [],
                "fitted_curves": [],
                "analyzable_data": null
            }
        ]
    });

    let migrated_val = migrate_document_json(legacy_v0, 0).expect("migrate v0");
    assert_eq!(migrated_val.get("schema_version").and_then(serde_json::Value::as_u64), Some(u64::from(CURRENT_SCHEMA_VERSION)));
    assert!(migrated_val["data_sheets"][0].get("floating_notes").is_some());
    assert!(migrated_val["result_sheets"][0]["analysis"]["kind"]["NonlinearRegression"].get("emit_residuals_tab").is_some());

    let doc: ProjectDocument = serde_json::from_value(migrated_val).expect("deserialize migrated v0");
    validate_project_document(&doc).expect("validated migrated doc");
    assert_eq!(doc.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(doc.data_sheets.len(), 1);
    assert_eq!(doc.result_sheets.len(), 1);

    let future_err = migrate_document_json(serde_json::json!({"schema_version": CURRENT_SCHEMA_VERSION + 1}), CURRENT_SCHEMA_VERSION + 1)
        .expect_err("future major schema version must fail");
    assert!(matches!(future_err, FormatError::UnsupportedSchemaVersion { .. }));
}

#[test]
fn sanitize_circular_hooks_breaks_cycles_while_preserving_valid_hooks() {
    let mut doc = ProjectDocument::new("Hook Cycle Recovery").expect("doc");
    let info = InfoSheet::new_preseeded("info-1", "Assay Info").expect("info");
    doc.add_info_sheet(info).expect("add info");

    let ds =
        lumencraft_doc::DataSheet::new("data-1", "Raw", TableArchetype::Xy(lumencraft_doc::XyTable::new(SubcolumnMode::Single, 1, 2).expect("xy")))
            .expect("ds");
    doc.add_data_sheet(ds).expect("add ds");

    // 1. Self-referential HookSource on res-self + valid acyclic HookSource referencing info-1.
    let mut fit_self = NonlinearFitSpec::new("sigmoidal-4pl");
    fit_self.method = FitMethod::LeastSquares;
    fit_self.weighting = WeightingScheme::None;
    fit_self.parameter_rules.push(ParameterRule {
        name: String::from("Top"),
        scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("res-self"),
            parameter_key: String::from("Top"),
            data_set_index: None,
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    fit_self.parameter_rules.push(ParameterRule {
        name: String::from("Bottom"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-1")),
            constant_name: String::from("Km"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let res_self =
        ResultSheet::new(AnalysisSpec::new("res-self", "data-1", AnalysisKind::NonlinearRegression(fit_self)).expect("spec")).expect("res");
    doc.add_result_sheet(res_self).expect("add res-self");

    // 2. Mutual cycle: res-a analyzes res-b, and res-b hooks Normalize anchor back to res-a.
    let res_a = ResultSheet::new(
        AnalysisSpec::new(
            "res-a",
            "res-b",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::ColumnMin,
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: false,
            }),
        )
        .expect("spec a"),
    )
    .expect("res a");
    let res_b = ResultSheet::new(
        AnalysisSpec::new(
            "res-b",
            "data-1",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
                    result_sheet_id: SheetId::new("res-a"),
                    parameter_key: String::from("Slope"),
                    data_set_index: None,
                }),
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: false,
            }),
        )
        .expect("spec b"),
    )
    .expect("res b");
    doc.add_result_sheet(res_a).expect("add res-a");
    doc.add_result_sheet(res_b).expect("add res-b");

    // 3. Inline graph/layout hooks are non-cyclic unless referenced in a cycle; test graph self/cycle edge cleanup too.
    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", doc.data_sheet(&SheetId::new("data-1")).expect("ds")).expect("gs");
    gs.spec.title = String::from("Response ({{result:res-self:Bottom}})");
    doc.add_graph_sheet(gs).expect("add graph");
    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::TextCallout {
        id: String::from("callout-1"),
        markdown: String::from("Notes: {{info:info-1:Km}}"),
        bounds_pt: lumencraft_doc::RectPt::new(10.0, 10.0, 100.0, 40.0),
        font_size_pt: 11.0,
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Fig 1", layout_spec).expect("ls")).expect("add layout");

    assert!(matches!(detect_dependency_cycles(&doc), Err(FormatError::DependencyCycle { .. })));

    let diagnostics = sanitize_circular_hooks(&mut doc);
    assert_eq!(diagnostics.len(), 2, "expected 2 broken hook cycles, got {diagnostics:?}");
    detect_dependency_cycles(&doc).expect("dependency graph must be acyclic after sanitization");

    // Verify valid InfoConstant hook on Bottom was preserved while cyclic Top hook became Local.
    let AnalysisKind::NonlinearRegression(ref cleaned_fit) = doc.result_sheet(&SheetId::new("res-self")).expect("res-self").analysis.kind else {
        panic!("expected NonlinearRegression");
    };
    assert_eq!(cleaned_fit.parameter_rules[0].scope, ParameterScope::Local);
    assert!(matches!(cleaned_fit.parameter_rules[1].scope, ParameterScope::HookedConstant(HookSource::InfoConstant { .. })));

    let roundtrip_bytes = save_project_bytes(&doc).expect("save sanitized doc");
    let reopened = open_project_bytes(&roundtrip_bytes).expect("reopen sanitized doc");
    assert_eq!(reopened.title, "Hook Cycle Recovery");
}

#[test]
fn cleanroom_pzfx_and_prism_edge_archetypes_entities_info_and_exclusions_roundtrip() {
    let pzfx = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile SchemaVersion="2">
  <Title>Cleanroom Edge Parity (&alpha;-Assay)</Title>
  <InfoSequence>
    <Info ID="info-main">
      <Title>Assay Metadata</Title>
      <Constant><Name>K<Sub>m</Sub></Name><Value>2.5</Value><Units>&mu;M</Units></Constant>
      <Constant Name="&Delta;G" Value="-14.2" Units="kJ/mol"/>
      <Constant><Name>Vehicle</Name><Value>10<Sup>-6</Sup> M &alpha;-Toxin</Value></Constant>
      <Notes>Incubated at 37&deg;C &plusmn; 0.5&deg;C in 5% CO<Sub>2</Sub>.</Notes>
    </Info>
  </InfoSequence>
  <TableSequence>
    <Table ID="tbl-nested" TableType="Nested">
      <Title>Tissue CO<Sub>2</Sub> Nested (&mu;g/mL)</Title>
      <RowTitlesColumn><Subcolumn><d>Rep &alpha;</d><d>Rep &beta;</d></Subcolumn></RowTitlesColumn>
      <YColumn>
        <Title>WT</Title>
        <Subcolumn><Title>WT:1</Title><d>1.1</d><d>1.2*</d></Subcolumn>
        <Subcolumn><Title>WT:2</Title><d Excluded="1">9.9</d><d>1.4</d></Subcolumn>
      </YColumn>
      <YColumn>
        <Title>KO</Title>
        <Subcolumn><Title>KO:1</Title><d>*</d><d>2.2</d></Subcolumn>
        <Subcolumn><Title>KO:2</Title><d>2.3</d><d>*2.4</d></Subcolumn>
      </YColumn>
    </Table>
    <Table ID="tbl-surv" TableType="Survival">
      <Title>Survival Curve (&tau; &le; 30d)</Title>
      <RowTitlesColumn><Subcolumn><d>Subject 1</d><d>Subject 2</d><d>Subject 3</d></Subcolumn></RowTitlesColumn>
      <XColumn><Title>Time (days)</Title><Subcolumn><d>5</d><d Excluded="1">10</d><d>15*</d></Subcolumn></XColumn>
      <YColumn><Title>Cohort &alpha;</Title><Subcolumn><d>1</d><d>0*</d><d>*</d></Subcolumn></YColumn>
      <YColumn><Title>Cohort &beta;</Title><Subcolumn><d>1</d><d>1</d><d>0</d></Subcolumn></YColumn>
    </Table>
    <Table ID="tbl-mv" TableType="MultipleVariables">
      <Title>Cohort Variables (10<Sup>-6</Sup> M)</Title>
      <RowTitlesColumn><Subcolumn><d>P1</d><d>P2</d><d>P3</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Arm</Title><Subcolumn><d>&alpha;-Arm</d><d>&beta;-Arm</d><d>&alpha;-Arm</d></Subcolumn></YColumn>
      <YColumn><Title>IC<Sub>50</Sub></Title><Subcolumn><d>12.5</d><d>99.0*</d><d>*</d></Subcolumn></YColumn>
    </Table>
    <Table ID="tbl-cont" TableType="Contingency">
      <Title>Responder Matrix</Title>
      <RowTitlesColumn><Subcolumn><d>Exposed</d><d>Control</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Positive</Title><Subcolumn><d>18</d><d>4</d></Subcolumn></YColumn>
      <YColumn><Title>Negative</Title><Subcolumn><d>2*</d><d>16</d></Subcolumn></YColumn>
    </Table>
    <Table ID="tbl-pow" TableType="PartsOfWhole">
      <Title>Lineage Fractions (&Sigma;)</Title>
      <RowTitlesColumn><Subcolumn><d>CD4+</d><d>CD8+</d><d>B-cell</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Percent</Title><Subcolumn><d>45</d><d>35*</d><d>20</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;

    let report = import_pzfx_str(pzfx).expect("import cleanroom pzfx");
    let doc = &report.document;
    verify_cleanroom_pzfx_edge_contents(doc);

    // Round-trip through .lumencraft archive bytes
    let saved = save_project_bytes(doc).expect("save lumencraft bytes");
    let reopened = open_project_bytes(&saved).expect("open lumencraft bytes");
    assert_eq!(&reopened, doc);

    // Export all 5 sheets to CSV and pack into a clean-room .prism ZIP container, then re-import and compare!
    verify_prism_zip_roundtrip(doc, pzfx);
}

fn verify_cleanroom_pzfx_edge_contents(doc: &ProjectDocument) {
    assert_eq!(doc.title, "Cleanroom Edge Parity (α-Assay)");
    assert_eq!(doc.info_sheets.len(), 1);
    assert_eq!(doc.data_sheets.len(), 5);
    verify_edge_info_and_nested(&doc.info_sheets[0], &doc.data_sheets[0]);
    verify_edge_survival_mv_contingency_pow(&doc.data_sheets[1..]);
}

fn verify_edge_info_and_nested(info: &InfoSheet, nested_sheet: &lumencraft_doc::DataSheet) {
    assert_eq!(info.get_constant("Km").and_then(|c| c.units.as_deref()), Some("μM"));
    assert_eq!(info.get_constant("ΔG").and_then(|c| c.value.as_finite_f64()), Some(-14.2));
    assert_eq!(info.get_constant("Vehicle").map(|c| &c.value), Some(&ConstantValue::Text(String::from("10-6 M α-Toxin"))));
    assert_eq!(info.notes_markdown, "Incubated at 37°C ± 0.5°C in 5% CO2.");

    assert_eq!(nested_sheet.title, "Tissue CO2 Nested (μg/mL)");
    let TableArchetype::Nested(ref nested) = nested_sheet.archetype else {
        panic!("expected Nested");
    };
    assert_eq!(nested.row_titles, vec!["Rep α", "Rep β"]);
    assert_eq!(nested.groups.len(), 2);
    assert_eq!(nested.groups[0].biological_unit_count(), 2);
    assert_eq!(nested.groups[0].subcolumn_titles, vec!["WT:1", "WT:2"]);
    assert_eq!(nested.groups[0].cells[0][1], Cell { value: CellValue::Number(9.9), excluded: true });
    assert_eq!(nested.groups[0].cells[1][0], Cell { value: CellValue::Number(1.2), excluded: true });
    assert_eq!(nested.groups[1].cells[0][0], Cell { value: CellValue::Empty, excluded: true });
    assert_eq!(nested.groups[1].cells[1][1], Cell { value: CellValue::Number(2.4), excluded: true });
}

fn verify_edge_survival_mv_contingency_pow(sheets: &[lumencraft_doc::DataSheet]) {
    let TableArchetype::Survival(ref surv) = sheets[0].archetype else {
        panic!("expected Survival");
    };
    assert_eq!(sheets[0].title, "Survival Curve (τ ≤ 30d)");
    assert_eq!(surv.times[1], Cell { value: CellValue::Number(10.0), excluded: true });
    assert_eq!(surv.times[2], Cell { value: CellValue::Number(15.0), excluded: true });
    assert_eq!(surv.cohorts[0].cells[1][0], Cell { value: CellValue::Number(0.0), excluded: true });
    assert_eq!(surv.cohorts[0].cells[2][0], Cell { value: CellValue::Empty, excluded: true });

    let TableArchetype::MultipleVariables(ref mv) = sheets[1].archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv.variables[0].role, VariableRole::Categorical);
    assert_eq!(mv.variables[1].role, VariableRole::Continuous);
    assert_eq!(mv.variables[1].title, "IC50");
    assert!(mv.variables[1].cells[1].excluded);
    assert!(mv.variables[1].cells[2].excluded);

    let TableArchetype::Contingency(ref cont) = sheets[2].archetype else {
        panic!("expected Contingency");
    };
    assert_eq!(cont.row_titles, vec!["Exposed", "Control"]);
    assert_eq!(cont.data_sets[1].cells[0][0], Cell { value: CellValue::Number(2.0), excluded: true });

    let TableArchetype::PartsOfWhole(ref pow) = sheets[3].archetype else {
        panic!("expected PartsOfWhole");
    };
    assert_eq!(pow.slice_labels, vec!["CD4+", "CD8+", "B-cell"]);
    assert_eq!(pow.data_sets[0].cells[1][0], Cell { value: CellValue::Number(35.0), excluded: true });
}

fn verify_prism_zip_roundtrip(doc: &ProjectDocument, pzfx: &str) {
    let info = &doc.info_sheets[0];
    let csv_opts = CsvOptions::csv();
    let mut prism_zip = ZipWriter::new();
    let sheet_specs = [
        ("tbl-nested", &doc.data_sheets[0], "Nested", "sheets/01_nested.csv"),
        ("tbl-surv", &doc.data_sheets[1], "Survival", "sheets/02_survival.csv"),
        ("tbl-mv", &doc.data_sheets[2], "MultipleVariables", "sheets/03_multivar.csv"),
        ("tbl-cont", &doc.data_sheets[3], "Contingency", "sheets/04_contingency.csv"),
        ("tbl-pow", &doc.data_sheets[4], "PartsOfWhole", "sheets/05_pow.csv"),
    ];
    let mut manifest_sheets = Vec::new();
    for (id, sheet, kind_str, csv_path) in sheet_specs {
        let csv_text = export_datasheet_to_csv(sheet, &csv_opts).expect("export csv");
        prism_zip.add_file(csv_path, csv_text.into_bytes()).expect("add csv to prism");
        manifest_sheets.push(serde_json::json!({
            "id": id,
            "title": sheet.title,
            "kind": kind_str,
            "csv_path": csv_path
        }));
    }
    let prism_manifest = serde_json::json!({
        "title": doc.title,
        "sheets": manifest_sheets,
        "info_sheets": [
            {
                "id": "info-main",
                "title": info.title,
                "notes": info.notes_markdown,
                "constants": {
                    "Km": { "value": 2.5, "units": "μM" },
                    "ΔG": { "value": -14.2, "units": "kJ/mol" },
                    "Vehicle": "10-6 M α-Toxin"
                }
            }
        ]
    });
    prism_zip.add_file("manifest.json", serde_json::to_vec(&prism_manifest).expect("manifest json")).expect("add manifest");
    let prism_bytes = prism_zip.finish().expect("finish prism zip");

    let prism_report = import_prism_bytes(&prism_bytes).expect("import .prism");
    assert_eq!(prism_report.document.title, doc.title);
    assert_eq!(prism_report.document.data_sheets.len(), 5);
    assert_eq!(prism_report.document.info_sheets.len(), 1);
    assert_eq!(prism_report.document.info_sheets[0].get_constant("Km").and_then(|c| c.units.as_deref()), Some("μM"));
    assert_eq!(prism_report.document.info_sheets[0].notes_markdown, info.notes_markdown);

    for idx in 0..5 {
        assert_eq!(prism_report.document.data_sheets[idx].kind(), doc.data_sheets[idx].kind());
        assert_eq!(prism_report.document.data_sheets[idx].archetype, doc.data_sheets[idx].archetype);
    }

    // Also confirm a `.prism` containing an embedded `.pzfx` file imports cleanly.
    let mut pzfx_prism_zip = ZipWriter::new();
    pzfx_prism_zip.add_file("data/embedded.pzfx", pzfx.as_bytes().to_vec()).expect("add pzfx inside prism");
    let pzfx_prism_report = import_prism_bytes(&pzfx_prism_zip.finish().expect("zip")).expect("import pzfx inside prism");
    assert_eq!(pzfx_prism_report.document.data_sheets.len(), 5);
    assert_eq!(pzfx_prism_report.document.info_sheets.len(), 1);
}

#[test]
fn r1_s06_02_snapshot_covered_seq_prevents_double_apply_across_mid_checkpoint_crash_and_legacy_fail_clear() {
    struct FailClearBackend {
        inner: MemoryJournalBackend,
    }

    impl JournalBackend for FailClearBackend {
        fn acquire_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<Option<JournalLockLease>, FormatError> {
            self.inner.acquire_lease(lease, now_epoch_ms)
        }

        fn renew_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<(), FormatError> {
            self.inner.renew_lease(lease, now_epoch_ms)
        }

        fn release_lease(&mut self, owner_id: &str) -> Result<(), FormatError> {
            self.inner.release_lease(owner_id)
        }

        fn read_lease(&self) -> Result<Option<JournalLockLease>, FormatError> {
            self.inner.read_lease()
        }

        fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
            self.inner.write_snapshot_atomic(bytes)
        }

        fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
            self.inner.read_snapshot()
        }

        fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError> {
            self.inner.append_journal_line(ndjson_line)
        }

        fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError> {
            self.inner.read_journal_bytes()
        }

        fn replace_journal_bytes_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
            self.inner.replace_journal_bytes_atomic(bytes)
        }

        fn clear_journal(&mut self) -> Result<(), FormatError> {
            Err(FormatError::Io(String::from("simulated clear_journal failure")))
        }
    }

    // 1. Legacy JournalBackend override (only original 10 methods) where clear_journal fails mid-checkpoint.
    let (mut legacy_journal, _) =
        CrashRecoveryJournal::open_or_recover(FailClearBackend { inner: MemoryJournalBackend::new() }, "legacy-1", 2_000, 1_000)
            .expect("open legacy backend");
    let seq1 = legacy_journal.record_command(serde_json::json!({ "op": "inc", "delta": 1 })).expect("seq1");
    let seq2 = legacy_journal.record_command(serde_json::json!({ "op": "inc", "delta": 2 })).expect("seq2");
    assert_eq!((seq1, seq2), (1, 2));
    assert!(legacy_journal.checkpoint(b"SNAPSHOT-SEQ-2").is_err(), "FailClear must error on clear_journal");
    let crashed_legacy = legacy_journal.abandon();
    let (reopened_legacy, legacy_report) =
        CrashRecoveryJournal::open_or_recover(crashed_legacy, "legacy-2", 2_000, 4_000).expect("recover after FailClear");
    assert_eq!(legacy_report.snapshot.as_deref(), Some(b"SNAPSHOT-SEQ-2".as_slice()));
    assert_eq!(legacy_report.snapshot_covered_seq, Some(2));
    assert!(legacy_report.recovered_commands.is_empty(), "seq <= covered_seq must not double-apply");
    assert!(reopened_legacy.next_seq() >= 3);

    // 2. Native DirectoryJournalBackend: atomic snapshot + covered_seq survives crash before clear_journal
    //    and mid-promotion (when pending sidecar matches new snapshot while old meta sidecar still exists).
    let dir = TempRecoveryDir::new("s06-02-covered-seq");
    let (mut dir_journal, _) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("dir"), "dir-1", 2_000, 1_000)
            .expect("open dir journal");
    dir_journal.record_command(serde_json::json!({ "op": "c1" })).expect("dir c1");
    dir_journal.record_command(serde_json::json!({ "op": "c2" })).expect("dir c2");
    // Simulate crash right after snapshot promotion before ops.journal clear by writing snapshot+meta with covered_seq=2.
    dir_journal.backend_mut().write_snapshot_with_seq(b"DIR-SNAP-2", 2).expect("write snapshot with seq");
    let seq3 = dir_journal.record_command(serde_json::json!({ "op": "c3" })).expect("dir c3");
    assert_eq!(seq3, 3);
    drop(dir_journal.abandon());

    let (dir_reopened, dir_report) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("reopen dir"), "dir-2", 2_000, 4_000)
            .expect("recover dir journal");
    assert_eq!(dir_report.snapshot.as_deref(), Some(b"DIR-SNAP-2".as_slice()));
    assert_eq!(dir_report.snapshot_covered_seq, Some(2));
    assert_eq!(dir_report.recovered_seqs, vec![3]);
    assert_eq!(dir_report.recovered_commands, vec![serde_json::json!({ "op": "c3" })]);

    // Also simulate interrupted pending meta promotion cleanly resolved against snapshot checksum.
    let crashed_after_meta = dir_reopened.abandon();
    fs::write(dir.path.join(SNAPSHOT_FILE_NAME), b"DIR-SNAP-3").expect("write snap3");
    fs::write(
        dir.path.join("snapshot.meta.json"),
        serde_json::to_vec(&serde_json::json!({
            "format_version": 1,
            "covered_seq": 1,
            "snapshot_crc32": 0,
            "snapshot_len": 1
        }))
        .expect("old meta"),
    )
    .expect("write stale meta");
    fs::write(
        dir.path.join("snapshot.pending.meta.json"),
        serde_json::to_vec(&serde_json::json!({
            "format_version": 1,
            "covered_seq": 3,
            "snapshot_crc32": crc32fast::hash(b"DIR-SNAP-3"),
            "snapshot_len": b"DIR-SNAP-3".len()
        }))
        .expect("pending meta"),
    )
    .expect("write pending meta");
    let (dir_pending_reopened, pending_report) =
        CrashRecoveryJournal::open_or_recover(crashed_after_meta, "dir-3", 2_000, 7_000).expect("recover pending meta");
    assert_eq!(pending_report.snapshot_covered_seq, Some(3));
    assert!(pending_report.recovered_commands.is_empty());

    // After clean close, fresh session resets next_seq=1 and covered_seq is cleared so new seq=1 is not skipped.
    let clean_backend = dir_pending_reopened.close_cleanly(b"DIR-FINAL").expect("clean close");
    let (mut fresh_journal, fresh_report) = CrashRecoveryJournal::open_or_recover(clean_backend, "dir-4", 2_000, 8_000).expect("fresh open");
    assert_eq!(fresh_journal.next_seq(), 1);
    assert_eq!(fresh_report.snapshot_covered_seq, None);
    let fresh_seq = fresh_journal.record_command(serde_json::json!({ "op": "fresh-after-clean-close" })).expect("fresh seq1");
    assert_eq!(fresh_seq, 1);
    drop(fresh_journal.abandon());

    let (_, after_fresh_crash) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("open"), "dir-5", 2_000, 12_000)
            .expect("recover fresh seq1");
    assert_eq!(after_fresh_crash.recovered_seqs, vec![1]);
}

#[test]
fn r1_s06_03_interior_corrupt_and_short_write_lines_are_quarantined_with_warnings() {
    // 1. MemoryJournalBackend: torn interior write without trailing '\n' + later records + bad tail.
    let (mut mem_journal, _) = CrashRecoveryJournal::open_or_recover(MemoryJournalBackend::new(), "mem-1", 3_000, 1_000).expect("open mem");
    let cmd1 = serde_json::json!({ "step": 1 });
    let cmd2 = serde_json::json!({ "step": 2 });
    let cmd3 = serde_json::json!({ "step": 3 });
    mem_journal.record_command(cmd1.clone()).expect("cmd1");
    // Simulate torn short write lacking '\n' right in the middle of ops.journal.
    mem_journal.backend_mut().append_raw_journal_bytes(b"{\"format_version\":1,\"seq\":99");
    // Subsequent record_command must NOT glue onto the torn prefix line.
    mem_journal.record_command(cmd2.clone()).expect("cmd2");
    mem_journal.record_command(cmd3.clone()).expect("cmd3");
    // Also inject an invalid-UTF-8 corrupt tail.
    mem_journal.backend_mut().append_raw_journal_bytes(b"\xff\xfe{\"truncated\":");

    let crashed_mem = mem_journal.abandon();
    let (mem_recovered, mem_report) =
        CrashRecoveryJournal::open_or_recover(crashed_mem, "mem-2", 3_000, 5_000).expect("interior corruption must recover intact commands");
    assert_eq!(mem_report.recovered_commands, vec![cmd1.clone(), cmd2.clone(), cmd3.clone()]);
    assert_eq!(mem_report.recovered_seqs, vec![1, 2, 3]);
    assert_eq!(mem_report.discarded_corrupt_interior_lines, 1);
    assert_eq!(mem_report.discarded_corrupt_tail_lines, 1);
    assert_eq!(mem_report.total_discarded_corrupt_lines(), 2);
    assert_eq!(mem_report.warnings.len(), 2);
    assert_eq!(mem_report.warnings[0].kind, "corrupt_interior_line");
    assert_eq!(mem_report.warnings[1].kind, "corrupt_tail_line");
    drop(mem_recovered.abandon());

    // Subsequent open on rewritten journal observes 0 warnings.
    let (_, clean_mem_report) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&TempRecoveryDir::new("unused").path).expect("dir"), "x", 1, 1)
            .expect("empty");
    assert!(clean_mem_report.warnings.is_empty());

    // 2. DirectoryJournalBackend: interior partial JSON + valid later writes on real disk.
    let dir = TempRecoveryDir::new("s06-03-dir-interior");
    let (mut dir_journal, _) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("dir"), "d1", 2_000, 1_000).expect("open dir");
    dir_journal.record_command(cmd1.clone()).expect("d cmd1");
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(dir.path.join(JOURNAL_FILE_NAME)).expect("open raw ops.journal");
        f.write_all(b"{\"format_version\":1,\"seq\":2,\"partial").expect("write short partial");
        f.sync_data().expect("sync partial");
    }
    dir_journal.record_command(cmd2.clone()).expect("d cmd2 after partial");
    dir_journal.record_command(cmd3.clone()).expect("d cmd3");
    drop(dir_journal.abandon());

    let (reopened_dir, dir_report) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(&dir.path).expect("dir"), "d2", 2_000, 4_000)
            .expect("recover dir interior");
    assert_eq!(dir_report.recovered_commands, vec![cmd1, cmd2, cmd3]);
    assert_eq!(dir_report.recovered_seqs, vec![1, 2, 3]);
    assert_eq!(dir_report.discarded_corrupt_interior_lines, 1);
    assert_eq!(dir_report.warnings.len(), 1);
    assert_eq!(dir_report.warnings[0].kind, "corrupt_interior_line");
    let rewritten_backend = reopened_dir.close_cleanly(b"DONE").expect("close cleanly");
    assert!(rewritten_backend.journal_bytes_or_empty().is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
trait DirBackendBytesExt {
    fn journal_bytes_or_empty(&self) -> Vec<u8>;
}

#[cfg(not(target_arch = "wasm32"))]
impl DirBackendBytesExt for DirectoryJournalBackend {
    fn journal_bytes_or_empty(&self) -> Vec<u8> {
        self.read_journal_bytes().unwrap_or_default()
    }
}
