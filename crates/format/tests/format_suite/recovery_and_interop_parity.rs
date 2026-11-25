#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use crate::common::{TempDirGuard, build_zip_entries, callout_item, open_dir_journal, param_rule, pzfx_doc, result_sheet_with_kind, xy_sheet};
use curvebook_doc::{
    AnalysisKind, CURRENT_SCHEMA_VERSION, Cell, CellValue, ConstantValue, FitMethod, GraphSheet, HookSource, InfoSheet, LayoutSheet, LayoutSpec,
    NonlinearFitSpec, NormalizeAnchor, ParameterScope, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, VariableRole,
    WeightingScheme,
};
use curvebook_format::{
    CrashRecoveryJournal, CsvOptions, DirectoryJournalBackend, FormatError, JOURNAL_FILE_NAME, JournalBackend, JournalLockLease, JournalOpRecord,
    LOCKFILE_NAME, MemoryJournalBackend, SNAPSHOT_FILE_NAME, detect_dependency_cycles, export_datasheet_to_csv, import_prism_bytes, import_pzfx_str,
    migrate_document_json, open_project_bytes, sanitize_circular_hooks, save_project_bytes, validate_project_document,
};
use serde_json::json;

#[test]
fn directory_journal_clean_close_checkpoint_and_reopen() {
    let dir = TempDirGuard::new("clean-close").unwrap();
    let (mut journal, report) = open_dir_journal(dir.path(), "session-alpha", 10_000, 1_000).unwrap();
    assert!(
        report.snapshot.is_none()
            && report.recovered_commands.is_empty()
            && report.discarded_corrupt_tail_lines == 0
            && report.recovered_from_stale_lock.is_none()
    );
    assert!(report.active_lease.owner_id == "session-alpha" && dir.child(LOCKFILE_NAME).exists());

    assert_eq!(
        (
            journal.record_command(json!({ "op": "set_cell", "row": 0, "col": 0, "val": 12.5 })).unwrap(),
            journal.record_command(json!({ "op": "set_cell", "row": 1, "col": 0, "val": 25.0 })).unwrap()
        ),
        (1, 2)
    );
    assert!(dir.child(JOURNAL_FILE_NAME).exists());

    journal.checkpoint(b"SNAPSHOT-CHECKPOINT-1").unwrap();
    assert_eq!(
        (fs::read(dir.child(SNAPSHOT_FILE_NAME)).unwrap().as_slice(), dir.child(JOURNAL_FILE_NAME).exists()),
        (b"SNAPSHOT-CHECKPOINT-1".as_slice(), false)
    );

    assert_eq!(journal.record_command(json!({ "op": "rename_sheet", "title": "Assay" })).unwrap(), 3);
    assert!(dir.child(JOURNAL_FILE_NAME).exists());

    let backend_after = journal.close_cleanly(b"SNAPSHOT-FINAL").unwrap();
    assert!(!dir.child(JOURNAL_FILE_NAME).exists() && !dir.child(LOCKFILE_NAME).exists());
    assert_eq!(backend_after.read_snapshot().unwrap().as_deref(), Some(b"SNAPSHOT-FINAL".as_slice()));

    let (reopened, rep) = CrashRecoveryJournal::open_or_recover(backend_after, "session-beta", 10_000, 2_000).unwrap();
    assert!(
        rep.snapshot.as_deref() == Some(b"SNAPSHOT-FINAL".as_slice())
            && rep.recovered_commands.is_empty()
            && rep.discarded_corrupt_tail_lines == 0
            && rep.recovered_from_stale_lock.is_none()
            && reopened.next_seq() == 1
    );
    let _ = reopened.close_cleanly(b"SNAPSHOT-FINAL").unwrap();
}

#[test]
fn directory_journal_recovers_stale_lock_and_truncates_torn_tail_without_crashing() {
    let dir = TempDirGuard::new("crash-torn-tail").unwrap();
    let (mut journal, _) = open_dir_journal(dir.path(), "writer-1", 5_000, 10_000).unwrap();
    journal.checkpoint(b"BASELINE-DOC-V1").unwrap();

    let (cmd1, cmd2) = (json!({ "op": "insert_row", "sheet": "d1", "row": 0 }), json!({ "op": "edit_cell", "sheet": "d1", "row": 0, "val": 3.25 }));
    journal.record_command(cmd1.clone()).unwrap();
    journal.record_command(cmd2.clone()).unwrap();
    journal.renew_lease(11_000).unwrap();
    assert_eq!(journal.lease().expires_epoch_ms(), 16_000);
    drop(journal);

    let mut raw = fs::read(dir.child(JOURNAL_FILE_NAME)).unwrap();
    raw.extend_from_slice(b"{\"seq\":3,\"crc32\":123456,\"command_json\":{\"op\":\"partial\"");
    raw.push(0xFF);
    fs::write(dir.child(JOURNAL_FILE_NAME), &raw).unwrap();
    assert!(open_dir_journal(dir.path(), "writer-2", 5_000, 15_999).is_err());

    let (mut recovered, rep) = open_dir_journal(dir.path(), "writer-2", 5_000, 16_000).unwrap();
    assert_eq!(
        (rep.snapshot.as_deref(), &rep.recovered_commands, &rep.recovered_seqs, rep.discarded_corrupt_tail_lines),
        (Some(b"BASELINE-DOC-V1".as_slice()), &vec![cmd1.clone(), cmd2.clone()], &vec![1, 2], 1)
    );
    assert_eq!(
        (rep.recovered_from_stale_lock, rep.active_lease.owner_id.as_str(), recovered.next_seq()),
        (Some(JournalLockLease::new("writer-1", 11_000, 5_000).unwrap()), "writer-2", 3)
    );

    let cmd3 = json!({ "op": "post_recovery_edit", "ok": true });
    assert_eq!(recovered.record_command(cmd3.clone()).unwrap(), 3);
    drop(recovered);

    let (reopened_again, rep2) = open_dir_journal(dir.path(), "writer-3", 5_000, 22_000).unwrap();
    assert_eq!((rep2.recovered_commands, rep2.discarded_corrupt_tail_lines), (vec![cmd1, cmd2, cmd3], 0));
    let _ = reopened_again.close_cleanly(b"DONE").unwrap();
}

#[test]
fn directory_journal_detects_mid_journal_crc_corruption_without_leaving_phantom_lock() {
    let dir = TempDirGuard::new("crc-corrupt").unwrap();
    let (mut journal, _) = open_dir_journal(dir.path(), "w1", 1_000, 0).unwrap();
    journal.record_command(json!({ "step": 1, "val": 10 })).unwrap();
    journal.record_command(json!({ "step": 2, "val": 20 })).unwrap();
    drop(journal);

    let text = fs::read_to_string(dir.child(JOURNAL_FILE_NAME)).unwrap();
    let tampered = text.replacen("\"val\":10", "\"val\":99", 1);
    assert_ne!(text, tampered);
    fs::write(dir.child(JOURNAL_FILE_NAME), tampered).unwrap();

    assert!(matches!(open_dir_journal(dir.path(), "w2", 1_000, 5_000), Err(FormatError::CorruptCrc { .. })));
    assert!(!dir.child(LOCKFILE_NAME).exists());
}

#[test]
fn memory_journal_backend_web_storage_json_snapshot_and_recovery_parity() {
    let lease = JournalLockLease::new("browser-tab-1", 1_000, 4_000).unwrap();
    assert_eq!((lease.expires_epoch_ms(), lease.is_stale(4_999), lease.can_recover(5_000)), (5_000, false, true));

    let (mut journal, _) = CrashRecoveryJournal::open_or_recover(MemoryJournalBackend::new(), "browser-tab-1", 4_000, 1_000).unwrap();
    journal.checkpoint(b"WEB-SNAPSHOT-V1").unwrap();
    let (c1, c2) = (json!({ "action": "add_point", "x": 1.0, "y": 2.5 }), json!({ "action": "exclude_cell", "row": 0, "col": 1 }));
    journal.record_command(c1.clone()).unwrap();
    journal.record_command(c2.clone()).unwrap();

    let mut crashed = journal.abandon();
    crashed.append_raw_journal_bytes(b"{\"seq\":3,\"crc32\":42\n");
    let restored = MemoryJournalBackend::from_web_storage_json(&crashed.to_web_storage_json().unwrap()).unwrap();
    assert_eq!(restored.snapshot_bytes(), Some(b"WEB-SNAPSHOT-V1".as_slice()));
    assert!(CrashRecoveryJournal::open_or_recover(restored.clone(), "browser-tab-2", 4_000, 4_999).is_err());

    let (reopened, rep) = CrashRecoveryJournal::open_or_recover(restored, "browser-tab-2", 4_000, 5_000).unwrap();
    assert_eq!(
        (
            rep.snapshot.as_deref(),
            rep.recovered_commands,
            rep.discarded_corrupt_tail_lines,
            rep.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str())
        ),
        (Some(b"WEB-SNAPSHOT-V1".as_slice()), vec![c1, c2], 1, Some("browser-tab-1"))
    );
    assert_eq!(JournalOpRecord::new(1, json!({"ok": true})).unwrap().verify_crc(), Ok(()));

    let fin = reopened.close_cleanly(b"WEB-FINAL").unwrap();
    assert!(fin.lock_lease().is_none() && fin.journal_bytes().is_empty());
    assert_eq!(
        MemoryJournalBackend::from_web_storage_json(&fin.to_web_storage_json().unwrap()).unwrap().snapshot_bytes(),
        Some(b"WEB-FINAL".as_slice())
    );
}

#[test]
fn migrate_document_json_fills_backwards_compatible_fields_and_rejects_future_schemas() {
    let legacy_v0 = json!({
        "title": "Legacy Study",
        "data_sheets": [{ "id": "d1", "title": "Dose", "archetype": { "Column": { "design_hint": "UnpairedReplicates", "subcolumn_mode": "Single", "row_titles": [], "row_titles_expanded": false, "data_sets": [{ "title": "Ctrl", "cells": [[{"value": {"Number": 1.0}, "excluded": false}]] }] } } }],
        "result_sheets": [{
            "id": "r1",
            "analysis": {
                "id": "a1", "source_sheet_id": "d1", "selected_data_sets": [], "auto_title": "Nonlinear fit", "custom_title": null, "confidence_level": 0.95, "p_value_format": "FourDecimal",
                "kind": { "NonlinearRegression": { "equation_id": "sigmoidal-4pl", "custom_dsl": null, "method": "LeastSquares", "weighting": "None", "parameter_rules": [], "profile_likelihood_ci": false, "comparison": null, "confidence_bands": true, "prediction_bands": false, "interpolate_unknowns": false, "replicates_lack_of_fit_test": false, "normality_of_residuals": false, "max_iters": 200 } }
            },
            "status": "UpToDate", "tabs": [], "scalars": [], "pairwise_comparisons": [], "fitted_curves": [], "analyzable_data": null
        }]
    });
    let migrated_val = migrate_document_json(legacy_v0, 0).unwrap();
    assert_eq!(migrated_val["schema_version"].as_u64(), Some(u64::from(CURRENT_SCHEMA_VERSION)));
    assert!(
        migrated_val["data_sheets"][0].get("floating_notes").is_some()
            && migrated_val["result_sheets"][0]["analysis"]["kind"]["NonlinearRegression"].get("emit_residuals_tab").is_some()
    );

    let doc: ProjectDocument = serde_json::from_value(migrated_val).unwrap();
    validate_project_document(&doc).unwrap();
    assert_eq!((doc.schema_version, doc.data_sheets.len(), doc.result_sheets.len()), (CURRENT_SCHEMA_VERSION, 1, 1));
    assert!(matches!(
        migrate_document_json(json!({"schema_version": CURRENT_SCHEMA_VERSION + 1}), CURRENT_SCHEMA_VERSION + 1),
        Err(FormatError::UnsupportedSchemaVersion { .. })
    ));
}

#[test]
fn sanitize_circular_hooks_breaks_cycles_while_preserving_valid_hooks() {
    let mut doc = ProjectDocument::new("Hook Cycle Recovery").unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Assay Info").unwrap()).unwrap();
    doc.add_data_sheet(xy_sheet("data-1", SubcolumnMode::Single, 1, 2)).unwrap();

    let mut fit_self = NonlinearFitSpec::new("sigmoidal-4pl");
    (fit_self.method, fit_self.weighting) = (FitMethod::LeastSquares, WeightingScheme::None);
    fit_self.parameter_rules.extend([
        param_rule(
            "Top",
            ParameterScope::HookedConstant(HookSource::ResultScalar {
                result_sheet_id: SheetId::new("res-self"),
                parameter_key: String::from("Top"),
                data_set_index: None,
            }),
        ),
        param_rule(
            "Bottom",
            ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: String::from("Km"),
            }),
        ),
    ]);
    doc.add_result_sheet(result_sheet_with_kind("res-self", "data-1", AnalysisKind::NonlinearRegression(fit_self))).unwrap();

    let norm = |zero_anchor| {
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: false,
        })
    };
    doc.add_result_sheet(result_sheet_with_kind("res-a", "res-b", norm(NormalizeAnchor::ColumnMin))).unwrap();
    doc.add_result_sheet(result_sheet_with_kind(
        "res-b",
        "data-1",
        norm(NormalizeAnchor::Hooked(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("res-a"),
            parameter_key: String::from("Slope"),
            data_set_index: None,
        })),
    ))
    .unwrap();

    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", doc.data_sheet(&SheetId::new("data-1")).unwrap()).unwrap();
    gs.spec.title = String::from("Response ({{result:res-self:Bottom}})");
    doc.add_graph_sheet(gs).unwrap();
    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(callout_item("callout-1", "Notes: {{info:info-1:Km}}"));
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Fig 1", layout_spec).unwrap()).unwrap();

    assert!(matches!(detect_dependency_cycles(&doc), Err(FormatError::DependencyCycle { .. })));
    assert_eq!(sanitize_circular_hooks(&mut doc).len(), 2);
    detect_dependency_cycles(&doc).unwrap();

    let AnalysisKind::NonlinearRegression(ref cleaned) = doc.result_sheet(&SheetId::new("res-self")).unwrap().analysis.kind else {
        panic!("expected NonlinearRegression")
    };
    assert!(
        cleaned.parameter_rules[0].scope == ParameterScope::Local
            && matches!(cleaned.parameter_rules[1].scope, ParameterScope::HookedConstant(HookSource::InfoConstant { .. }))
    );
    assert_eq!(open_project_bytes(&save_project_bytes(&doc).unwrap()).unwrap().title, "Hook Cycle Recovery");
}

#[test]
fn cleanroom_pzfx_and_prism_edge_archetypes_entities_info_and_exclusions_roundtrip() {
    let pzfx = pzfx_doc(
        r#"<Title>Cleanroom Edge Parity (&alpha;-Assay)</Title>
  <InfoSequence><Info ID="info-main"><Title>Assay Metadata</Title>
    <Constant><Name>K<Sub>m</Sub></Name><Value>2.5</Value><Units>&mu;M</Units></Constant>
    <Constant Name="&Delta;G" Value="-14.2" Units="kJ/mol"/>
    <Constant><Name>Vehicle</Name><Value>10<Sup>-6</Sup> M &alpha;-Toxin</Value></Constant>
    <Notes>Incubated at 37&deg;C &plusmn; 0.5&deg;C in 5% CO<Sub>2</Sub>.</Notes>
  </Info></InfoSequence>
  <TableSequence>
    <Table ID="tbl-nested" TableType="Nested"><Title>Tissue CO<Sub>2</Sub> Nested (&mu;g/mL)</Title><RowTitlesColumn><Subcolumn><d>Rep &alpha;</d><d>Rep &beta;</d></Subcolumn></RowTitlesColumn><YColumn><Title>WT</Title><Subcolumn><Title>WT:1</Title><d>1.1</d><d Excluded="1">1.2</d></Subcolumn><Subcolumn><Title>WT:2</Title><d Excluded="1">9.9</d><d>1.4</d></Subcolumn></YColumn><YColumn><Title>KO</Title><Subcolumn><Title>KO:1</Title><d Excluded="1"/><d>2.2</d></Subcolumn><Subcolumn><Title>KO:2</Title><d>2.3</d><d Excluded="1">2.4</d></Subcolumn></YColumn></Table>
    <Table ID="tbl-surv" TableType="Survival"><Title>Survival Curve (&tau; &le; 30d)</Title><RowTitlesColumn><Subcolumn><d>Subject 1</d><d>Subject 2</d><d>Subject 3</d></Subcolumn></RowTitlesColumn><XColumn><Title>Time (days)</Title><Subcolumn><d>5</d><d Excluded="1">10</d><d Excluded="1">15</d></Subcolumn></XColumn><YColumn><Title>Cohort &alpha;</Title><Subcolumn><d>1</d><d Excluded="1">0</d><d Excluded="1"/></Subcolumn></YColumn><YColumn><Title>Cohort &beta;</Title><Subcolumn><d>1</d><d>1</d><d>0</d></Subcolumn></YColumn></Table>
    <Table ID="tbl-mv" TableType="MultipleVariables"><Title>Cohort Variables (10<Sup>-6</Sup> M)</Title><RowTitlesColumn><Subcolumn><d>P1</d><d>P2</d><d>P3</d></Subcolumn></RowTitlesColumn><YColumn><Title>Arm</Title><Subcolumn><d>&alpha;-Arm</d><d>&beta;-Arm</d><d>&alpha;-Arm</d></Subcolumn></YColumn><YColumn><Title>IC<Sub>50</Sub></Title><Subcolumn><d>12.5</d><d Excluded="1">99.0</d><d Excluded="1"/></Subcolumn></YColumn></Table>
    <Table ID="tbl-cont" TableType="Contingency"><Title>Responder Matrix</Title><RowTitlesColumn><Subcolumn><d>Exposed</d><d>Control</d></Subcolumn></RowTitlesColumn><YColumn><Title>Positive</Title><Subcolumn><d>18</d><d>4</d></Subcolumn></YColumn><YColumn><Title>Negative</Title><Subcolumn><d Excluded="1">2</d><d>16</d></Subcolumn></YColumn></Table>
    <Table ID="tbl-pow" TableType="PartsOfWhole"><Title>Lineage Fractions (&Sigma;)</Title><RowTitlesColumn><Subcolumn><d>CD4+</d><d>CD8+</d><d>B-cell</d></Subcolumn></RowTitlesColumn><YColumn><Title>Percent</Title><Subcolumn><d>45</d><d Excluded="1">35</d><d>20</d></Subcolumn></YColumn></Table>
  </TableSequence>"#,
    );
    let doc = import_pzfx_str(&pzfx).unwrap().document;
    let info = &doc.info_sheets[0];
    assert_eq!((doc.title.as_str(), doc.info_sheets.len(), doc.data_sheets.len()), ("Cleanroom Edge Parity (α-Assay)", 1, 5));
    assert_eq!(
        (
            info.get_constant("Km").and_then(|c| c.units.as_deref()),
            info.get_constant("ΔG").and_then(|c| c.value.as_finite_f64()),
            info.get_constant("Vehicle").map(|c| &c.value),
            info.notes_markdown.as_str()
        ),
        (Some("μM"), Some(-14.2), Some(&ConstantValue::Text(String::from("10-6 M α-Toxin"))), "Incubated at 37°C ± 0.5°C in 5% CO2.")
    );

    let TableArchetype::Nested(ref n) = doc.data_sheets[0].archetype else { panic!("expected Nested") };
    assert_eq!(
        (doc.data_sheets[0].title.as_str(), &n.row_titles, n.groups.len(), n.groups[0].biological_unit_count(), &n.groups[0].subcolumn_titles),
        ("Tissue CO2 Nested (μg/mL)", &vec![String::from("Rep α"), String::from("Rep β")], 2, 2, &vec![String::from("WT:1"), String::from("WT:2")])
    );
    assert_eq!(
        (&n.groups[0].cells[0][1], &n.groups[0].cells[1][0], &n.groups[1].cells[0][0], &n.groups[1].cells[1][1]),
        (&Cell::excluded_number(9.9), &Cell::excluded_number(1.2), &Cell { value: CellValue::Empty, excluded: true }, &Cell::excluded_number(2.4))
    );

    let TableArchetype::Survival(ref s) = doc.data_sheets[1].archetype else { panic!("expected Survival") };
    assert_eq!(
        (doc.data_sheets[1].title.as_str(), &s.times[1], &s.times[2], &s.cohorts[0].cells[1][0], &s.cohorts[0].cells[2][0]),
        (
            "Survival Curve (τ ≤ 30d)",
            &Cell::excluded_number(10.0),
            &Cell::excluded_number(15.0),
            &Cell::excluded_number(0.0),
            &Cell { value: CellValue::Empty, excluded: true }
        )
    );

    let TableArchetype::MultipleVariables(ref mv) = doc.data_sheets[2].archetype else { panic!("expected MV") };
    assert_eq!(
        (
            mv.variables[0].role,
            mv.variables[1].role,
            mv.variables[1].title.as_str(),
            mv.variables[1].cells[1].excluded,
            mv.variables[1].cells[2].excluded
        ),
        (VariableRole::Categorical, VariableRole::Continuous, "IC50", true, true)
    );

    let TableArchetype::Contingency(ref cont) = doc.data_sheets[3].archetype else { panic!("expected Contingency") };
    assert_eq!(
        (&cont.row_titles, &cont.data_sets[1].cells[0][0]),
        (&vec![String::from("Exposed"), String::from("Control")], &Cell::excluded_number(2.0))
    );

    let TableArchetype::PartsOfWhole(ref pow) = doc.data_sheets[4].archetype else { panic!("expected PoW") };
    assert_eq!(
        (&pow.slice_labels, &pow.data_sets[0].cells[1][0]),
        (&vec![String::from("CD4+"), String::from("CD8+"), String::from("B-cell")], &Cell::excluded_number(35.0))
    );

    assert_eq!(open_project_bytes(&save_project_bytes(&doc).unwrap()).unwrap(), doc);

    let specs = [
        ("tbl-nested", &doc.data_sheets[0], "Nested", "sheets/01_nested.csv"),
        ("tbl-surv", &doc.data_sheets[1], "Survival", "sheets/02_survival.csv"),
        ("tbl-mv", &doc.data_sheets[2], "MultipleVariables", "sheets/03_multivar.csv"),
        ("tbl-cont", &doc.data_sheets[3], "Contingency", "sheets/04_contingency.csv"),
        ("tbl-pow", &doc.data_sheets[4], "PartsOfWhole", "sheets/05_pow.csv"),
    ];
    let csv_blobs: Vec<(&str, Vec<u8>)> =
        specs.iter().map(|&(_, s, _, p)| (p, export_datasheet_to_csv(s, &CsvOptions::csv()).unwrap().into_bytes())).collect();
    let manifest_bytes = serde_json::to_vec(&json!({
        "title": doc.title,
        "sheets": specs.iter().map(|&(id, s, k, p)| json!({ "id": id, "title": s.title, "kind": k, "csv_path": p })).collect::<Vec<_>>(),
        "info_sheets": [{ "id": "info-main", "title": info.title, "notes": info.notes_markdown, "constants": { "Km": { "value": 2.5, "units": "μM" }, "ΔG": { "value": -14.2, "units": "kJ/mol" }, "Vehicle": "10-6 M α-Toxin" } }]
    })).unwrap();
    let mut entries: Vec<(&str, &[u8])> = csv_blobs.iter().map(|(p, b)| (*p, b.as_slice())).collect();
    entries.push(("manifest.json", &manifest_bytes));
    let imported = import_prism_bytes(&build_zip_entries(&entries)).unwrap().document;
    assert_eq!(
        (
            imported.title.as_str(),
            imported.data_sheets.len(),
            imported.info_sheets.len(),
            imported.info_sheets[0].get_constant("Km").and_then(|c| c.units.as_deref()),
            imported.info_sheets[0].notes_markdown.as_str()
        ),
        (doc.title.as_str(), 5, 1, Some("μM"), info.notes_markdown.as_str())
    );
    for (a, b) in imported.data_sheets.iter().zip(&doc.data_sheets) {
        assert_eq!((a.kind(), &a.archetype), (b.kind(), &b.archetype));
    }

    let pzfx_prism = import_prism_bytes(&build_zip_entries(&[("data/embedded.pzfx", pzfx.as_bytes())])).unwrap().document;
    assert_eq!((pzfx_prism.data_sheets.len(), pzfx_prism.info_sheets.len()), (5, 1));
}

#[test]
fn r1_s06_02_snapshot_covered_seq_prevents_double_apply_across_mid_checkpoint_crash_and_legacy_fail_clear() {
    struct FailClearBackend(MemoryJournalBackend);
    impl JournalBackend for FailClearBackend {
        fn acquire_lease(&mut self, l: &JournalLockLease, t: u64) -> Result<Option<JournalLockLease>, FormatError> {
            self.0.acquire_lease(l, t)
        }
        fn renew_lease(&mut self, l: &JournalLockLease, t: u64) -> Result<(), FormatError> {
            self.0.renew_lease(l, t)
        }
        fn release_lease(&mut self, o: &str) -> Result<(), FormatError> {
            self.0.release_lease(o)
        }
        fn read_lease(&self) -> Result<Option<JournalLockLease>, FormatError> {
            self.0.read_lease()
        }
        fn write_snapshot_atomic(&mut self, b: &[u8]) -> Result<(), FormatError> {
            self.0.write_snapshot_atomic(b)
        }
        fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
            self.0.read_snapshot()
        }
        fn append_journal_line(&mut self, s: &str) -> Result<(), FormatError> {
            self.0.append_journal_line(s)
        }
        fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError> {
            self.0.read_journal_bytes()
        }
        fn replace_journal_bytes_atomic(&mut self, b: &[u8]) -> Result<(), FormatError> {
            self.0.replace_journal_bytes_atomic(b)
        }
        fn clear_journal(&mut self) -> Result<(), FormatError> {
            Err(FormatError::Io(String::from("simulated clear_journal failure")))
        }
    }

    let (mut legacy, _) = CrashRecoveryJournal::open_or_recover(FailClearBackend(MemoryJournalBackend::new()), "legacy-1", 2_000, 1_000).unwrap();
    assert_eq!(
        (legacy.record_command(json!({"op":"inc","delta":1})).unwrap(), legacy.record_command(json!({"op":"inc","delta":2})).unwrap()),
        (1, 2)
    );
    assert!(legacy.checkpoint(b"SNAPSHOT-SEQ-2").is_err());
    let (reopened_legacy, leg_rep) = CrashRecoveryJournal::open_or_recover(legacy.abandon(), "legacy-2", 2_000, 4_000).unwrap();
    assert!(
        leg_rep.snapshot.as_deref() == Some(b"SNAPSHOT-SEQ-2".as_slice())
            && leg_rep.snapshot_covered_seq == Some(2)
            && leg_rep.recovered_commands.is_empty()
            && reopened_legacy.next_seq() >= 3
    );

    let dir = TempDirGuard::new("s06-02-covered-seq").unwrap();
    let (mut dir_j, _) = open_dir_journal(dir.path(), "dir-1", 2_000, 1_000).unwrap();
    dir_j.record_command(json!({ "op": "c1" })).unwrap();
    dir_j.record_command(json!({ "op": "c2" })).unwrap();
    dir_j.backend_mut().write_snapshot_with_seq(b"DIR-SNAP-2", 2).unwrap();
    assert_eq!(dir_j.record_command(json!({ "op": "c3" })).unwrap(), 3);
    drop(dir_j.abandon());

    let (dir_reopened, dir_rep) = open_dir_journal(dir.path(), "dir-2", 2_000, 4_000).unwrap();
    assert_eq!(
        (dir_rep.snapshot.as_deref(), dir_rep.snapshot_covered_seq, dir_rep.recovered_seqs, dir_rep.recovered_commands),
        (Some(b"DIR-SNAP-2".as_slice()), Some(2), vec![3], vec![json!({ "op": "c3" })])
    );

    let crashed_meta = dir_reopened.abandon();
    fs::write(dir.child(SNAPSHOT_FILE_NAME), b"DIR-SNAP-3").unwrap();
    fs::write(
        dir.child("snapshot.meta.json"),
        serde_json::to_vec(&json!({ "format_version": 1, "covered_seq": 1, "snapshot_crc32": 0, "snapshot_len": 1 })).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.child("snapshot.pending.meta.json"),
        serde_json::to_vec(
            &json!({ "format_version": 1, "covered_seq": 3, "snapshot_crc32": crc32fast::hash(b"DIR-SNAP-3"), "snapshot_len": b"DIR-SNAP-3".len() }),
        )
        .unwrap(),
    )
    .unwrap();
    let (dir_pending, pending_rep) = CrashRecoveryJournal::open_or_recover(crashed_meta, "dir-3", 2_000, 7_000).unwrap();
    assert!(pending_rep.snapshot_covered_seq == Some(3) && pending_rep.recovered_commands.is_empty());

    let (mut fresh_j, fresh_rep) =
        CrashRecoveryJournal::open_or_recover(dir_pending.close_cleanly(b"DIR-FINAL").unwrap(), "dir-4", 2_000, 8_000).unwrap();
    assert!(
        fresh_j.next_seq() == 1
            && fresh_rep.snapshot_covered_seq.is_none()
            && fresh_j.record_command(json!({ "op": "fresh-after-clean-close" })).unwrap() == 1
    );
    drop(fresh_j.abandon());
    assert_eq!(open_dir_journal(dir.path(), "dir-5", 2_000, 12_000).unwrap().1.recovered_seqs, vec![1]);
}

#[test]
fn r1_s06_03_interior_corrupt_and_short_write_lines_are_quarantined_with_warnings() {
    let (cmd1, cmd2, cmd3) = (json!({ "step": 1 }), json!({ "step": 2 }), json!({ "step": 3 }));

    let (mut mem_j, _) = CrashRecoveryJournal::open_or_recover(MemoryJournalBackend::new(), "mem-1", 3_000, 1_000).unwrap();
    mem_j.record_command(cmd1.clone()).unwrap();
    mem_j.backend_mut().append_raw_journal_bytes(b"{\"format_version\":1,\"seq\":99");
    mem_j.record_command(cmd2.clone()).unwrap();
    mem_j.record_command(cmd3.clone()).unwrap();
    mem_j.backend_mut().append_raw_journal_bytes(b"\xff\xfe{\"truncated\":");

    let (mem_rec, mem_rep) = CrashRecoveryJournal::open_or_recover(mem_j.abandon(), "mem-2", 3_000, 5_000).unwrap();
    assert_eq!((&mem_rep.recovered_commands, &mem_rep.recovered_seqs), (&vec![cmd1.clone(), cmd2.clone(), cmd3.clone()], &vec![1, 2, 3]));
    assert_eq!((mem_rep.discarded_corrupt_interior_lines, mem_rep.discarded_corrupt_tail_lines, mem_rep.total_discarded_corrupt_lines()), (1, 1, 2));
    assert_eq!(
        (mem_rep.warnings.len(), mem_rep.warnings[0].kind.as_str(), mem_rep.warnings[1].kind.as_str()),
        (2, "corrupt_interior_line", "corrupt_tail_line")
    );
    drop(mem_rec.abandon());

    let unused_dir = TempDirGuard::new("unused").unwrap();
    assert!(open_dir_journal(unused_dir.path(), "x", 1, 1).unwrap().1.warnings.is_empty());

    let dir = TempDirGuard::new("s06-03-dir-interior").unwrap();
    let (mut dir_j, _) = open_dir_journal(dir.path(), "d1", 2_000, 1_000).unwrap();
    dir_j.record_command(cmd1.clone()).unwrap();
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(dir.child(JOURNAL_FILE_NAME)).unwrap();
        f.write_all(b"{\"format_version\":1,\"seq\":2,\"partial").unwrap();
        f.sync_data().unwrap();
    }
    dir_j.record_command(cmd2.clone()).unwrap();
    dir_j.record_command(cmd3.clone()).unwrap();
    drop(dir_j.abandon());

    let (reopened_dir, dir_rep) = open_dir_journal(dir.path(), "d2", 2_000, 4_000).unwrap();
    assert_eq!(
        (
            dir_rep.recovered_commands,
            dir_rep.recovered_seqs,
            dir_rep.discarded_corrupt_interior_lines,
            dir_rep.warnings.len(),
            dir_rep.warnings[0].kind.as_str()
        ),
        (vec![cmd1, cmd2, cmd3], vec![1, 2, 3], 1, 1, "corrupt_interior_line")
    );
    assert!(reopened_dir.close_cleanly(b"DONE").unwrap().read_journal_bytes().unwrap_or_default().is_empty());
}

#[test]
fn r1_s11_02_dead_local_pid_lock_reclaimed_without_waiting_ttl() {
    use curvebook_format::recovery::{is_dead_local_owner, is_lease_reclaimable};

    let self_owner = format!("curvebook-pid-{}", std::process::id());
    assert!(!is_dead_local_owner("writer-1") && !is_dead_local_owner("curvebook-pid-0") && !is_dead_local_owner(&self_owner));

    let dir_live = TempDirGuard::new("s11-02-live-pid").unwrap();
    let (live_j, _) = open_dir_journal(dir_live.path(), &self_owner, 300_000, 1_000_000).unwrap();
    assert!(open_dir_journal(dir_live.path(), &format!("{self_owner}-win-2"), 300_000, 1_000_050).unwrap_err().to_string().contains("actively held"));
    drop(live_j.abandon());

    #[cfg(unix)]
    {
        let dead_owner = "curvebook-pid-999999999";
        assert!(is_dead_local_owner(dead_owner));
        let dir_dead = TempDirGuard::new("s11-02-dead-pid").unwrap();
        let (mut crashed, _) = open_dir_journal(dir_dead.path(), dead_owner, 300_000, 1_000_000).unwrap();
        crashed.record_command(json!({ "op": "edit" })).unwrap();
        let stale_lease = crashed.lease().clone();
        assert!(!stale_lease.is_stale(1_000_050) && is_lease_reclaimable(&stale_lease, 1_000_050));
        drop(crashed.abandon());

        let (reclaimed, rep) = open_dir_journal(dir_dead.path(), &self_owner, 300_000, 1_000_050).unwrap();
        assert_eq!(
            (rep.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str()), rep.recovered_commands),
            (Some(dead_owner), vec![json!({ "op": "edit" })])
        );
        let _ = reclaimed.close_cleanly(b"RECLAIMED");
    }
}

#[test]
fn r1_s11_03_stale_owner_cannot_clobber_active_owner_snapshot_or_journal() {
    let dir = TempDirGuard::new("s11-03-clobber").unwrap();
    let (mut owner_a, _) = open_dir_journal(dir.path(), "owner-A", 1_000, 1_000).unwrap();
    owner_a.record_command(json!({ "from": "A" })).unwrap();

    let (mut owner_b, _) = open_dir_journal(dir.path(), "owner-B", 5_000, 5_000).unwrap();
    owner_b.checkpoint(b"LIVE-SNAPSHOT-B").unwrap();
    owner_b.record_command(json!({ "from": "B" })).unwrap();

    assert!(owner_a.checkpoint(b"STALE-A-CLOBBER").unwrap_err().to_string().contains("owned by `owner-B`"));
    assert_eq!(fs::read(dir.child(SNAPSHOT_FILE_NAME)).unwrap(), b"LIVE-SNAPSHOT-B");
    assert!(owner_a.close_cleanly(b"STALE-A-CLOSE").unwrap_err().to_string().contains("owned by `owner-B`"));
    assert_eq!(fs::read(dir.child(SNAPSHOT_FILE_NAME)).unwrap(), b"LIVE-SNAPSHOT-B");
    assert_eq!(DirectoryJournalBackend::open_dir(dir.path()).unwrap().read_lease().unwrap().unwrap().owner_id, "owner-B");

    let (_, post_b) = open_dir_journal(dir.path(), "owner-C", 5_000, 20_000).unwrap();
    assert_eq!((post_b.snapshot.as_deref(), post_b.recovered_commands), (Some(b"LIVE-SNAPSHOT-B".as_slice()), vec![json!({ "from": "B" })]));
}
