//! Integration tests for Wave 3 Track 3B (`§4.2`, `§6.1`, `§6.3`):
//! - `project.exportPzfx` atomic write, `ExternalPathPolicy`, warnings, round-trip, and session
//!   `file_path`/`dirty` invariance
//! - `project.exportPrism` fail-closed refusal without `experimental: true` and opt-in round-trip
//! - `table.importPrism` live-analysis recalculation in one undo step preserving `ResultStatus::Imported` snapshots
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, ErrorCode, ImportedSnapshotSpec, InfoSheet, ProjectDocument,
    ResultCell, ResultProvenance, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, TableArchetype,
};
use curvebook_engine::services::load_project_from_bytes;
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::registry::DESC_PRISM;
use curvebook_format::{ZipArchive, ZipWriter, compare_pzfx_data_layer, import_pzfx_bytes, open_project_bytes, save_project_bytes};
use curvebook_palette::Rgba8;
use serde_json::json;

use super::common::{TempDir, decode_base64, encode_base64, err_cmd, ok_cmd, xy_sheet};

fn seed_rich_project_session(dir: &TempDir) -> (Session, std::path::PathBuf) {
    let mut doc = ProjectDocument::new("Track 3B Export Study").unwrap();
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol").unwrap();
    info.set_constant("Experiment Date", ConstantValue::Date(String::from("2026-07-01")), None).unwrap();
    info.notes_markdown = String::from("Buffer: PBS");
    doc.add_info_sheet(info).unwrap();
    doc.add_data_sheet(xy_sheet("xy-1", "Dose Curve", "Dose", &[1.0, 10.0, 100.0], &[("Signal", &[12.5, 48.0, 91.5])])).unwrap();

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "xy-1", "kind": "descriptive" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "xy-1", "kind": "xy" }));
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" }));
    ok_cmd(&mut session, "project.createCheckpoint", json!({ "name": "Baseline" }));

    let cb_path = dir.child("study.curvebook");
    ok_cmd(&mut session, "project.save", json!({ "path": cb_path.to_string_lossy() }));
    assert!(!session.is_dirty());
    assert_eq!(session.file_path(), Some(cb_path.as_path()));

    // Edit one cell so `dirty == true` while `file_path == Some(cb_path)`.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "value": 15.0 }));
    assert!(session.is_dirty());
    assert_eq!(session.file_path(), Some(cb_path.as_path()));

    (session, cb_path)
}

#[test]
fn project_export_pzfx_atomic_write_preserves_session_state_reports_warnings_and_roundtrips() {
    let dir = TempDir::new("export-pzfx");
    let (mut session, cb_path) = seed_rich_project_session(&dir);
    let undo_before = session.undo_depth();

    let out_path = dir.child("nested/export_copy.pzfx");
    let outcome = ok_cmd(&mut session, "project.exportPzfx", json!({ "path": out_path.to_string_lossy(), "return_bytes": true }));

    assert!(!outcome.mutated, "project.exportPzfx must not mutate document or undo stack");
    assert_eq!(session.undo_depth(), undo_before);
    assert_eq!(session.file_path(), Some(cb_path.as_path()), "exporting .pzfx must never change session file_path");
    assert!(session.is_dirty(), "exporting .pzfx must never clear unsaved dirty flag");

    assert_eq!(outcome.payload["format"], json!("pzfx"));
    assert_eq!(outcome.payload["data_sheet_count"], json!(1));
    assert_eq!(outcome.payload["info_sheet_count"], json!(1));
    assert!(outcome.payload["total_dropped_count"].as_u64().unwrap() >= 4);
    assert!(!outcome.warnings.is_empty(), "expected dropped-content warnings on outcome.warnings");
    assert!(!outcome.payload["warnings"].as_array().unwrap().is_empty());
    assert!(
        outcome.payload["notice"].as_str().unwrap().contains("not tested in other applications"),
        "expected honest §6.3 notice, got {:?}",
        outcome.payload["notice"]
    );

    let disk_bytes = fs::read(&out_path).unwrap();
    let b64_bytes = decode_base64(outcome.payload["bytes_base64"].as_str().unwrap());
    assert_eq!(disk_bytes, b64_bytes);

    // Re-import exported `.pzfx` and verify exact data-layer fidelity.
    let reimported = import_pzfx_bytes(&disk_bytes).unwrap();
    let diff = compare_pzfx_data_layer(session.document(), &reimported.document);
    assert!(diff.is_identical(), "round-trip data-layer diff: {diff:?}");

    // Path policy enforcement (`RejectExternal`) blocks writing outside policy without creating target.
    let mut blocked_sess = session.clone().with_path_policy(ExternalPathPolicy::RejectExternal);
    let blocked_path = dir.child("blocked.pzfx");
    let err = err_cmd(&mut blocked_sess, "project.exportPzfx", json!({ "path": blocked_path.to_string_lossy() }));
    assert_eq!(err.code, ErrorCode::PathEscape);
    assert!(!blocked_path.exists());
}

#[test]
fn project_export_prism_requires_experimental_opt_in_and_preserves_session_state() {
    let dir = TempDir::new("export-archive");
    let (mut session, cb_path) = seed_rich_project_session(&dir);
    let out_path = dir.child("study.prism");

    // 1. Omitted or `experimental: false` fails closed with UnsupportedFormat pointing to `.pzfx`.
    for params in [json!({ "path": out_path.to_string_lossy() }), json!({ "path": out_path.to_string_lossy(), "experimental": false })] {
        let err = err_cmd(&mut session, "project.exportPrism", params);
        assert_eq!(err.code, ErrorCode::UnsupportedFormat);
        assert!(err.message().contains(".pzfx"), "refusal should guide users to .pzfx: {}", err.message());
        assert!(!out_path.exists());
        assert_eq!(session.file_path(), Some(cb_path.as_path()));
        assert!(session.is_dirty());
    }

    // 2. Explicit `experimental: true` writes atomically, returns warnings, keeps session state intact, and re-imports.
    let outcome =
        ok_cmd(&mut session, "project.exportPrism", json!({ "path": out_path.to_string_lossy(), "experimental": true, "return_bytes": true }));
    assert!(!outcome.mutated);
    assert_eq!(session.file_path(), Some(cb_path.as_path()));
    assert!(session.is_dirty());
    assert_eq!(outcome.payload["format"], json!(DESC_PRISM.id));
    assert_eq!(outcome.payload["experimental"], json!(true));
    assert!(!outcome.warnings.is_empty());

    let disk_bytes = fs::read(&out_path).unwrap();
    let b64_bytes = decode_base64(outcome.payload["bytes_base64"].as_str().unwrap());
    assert_eq!(disk_bytes, b64_bytes);

    let mut target = Session::new();
    let imported = ok_cmd(&mut target, "table.importPrism", json!({ "bytes_base64": outcome.payload["bytes_base64"] }));
    assert!(imported.mutated);
    assert_eq!(target.document().data_sheets.len(), 1);
    assert_eq!(target.document().info_sheets.len(), 1);
}

#[test]
fn table_import_prism_recalculates_live_analyses_in_single_undo_step_preserving_imported_snapshots() {
    let mut doc = ProjectDocument::new("Mixed Live And Snapshot Archive").unwrap();
    doc.add_data_sheet(xy_sheet("xy-1", "Dose Response", "Dose", &[1.0, 2.0, 3.0], &[("Y", &[10.0, 20.0, 30.0])])).unwrap();

    // 1. Live-translated (non-ImportedSnapshot) analysis arriving Dirty.
    let live_spec = AnalysisSpec::new("res-live", "xy-1", AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    let mut live_res = ResultSheet::new(live_spec).unwrap();
    live_res.status = ResultStatus::Dirty;
    doc.add_result_sheet(live_res).unwrap();

    // 2. Sibling read-only ImportedSnapshot on the same data sheet.
    let snap_spec = AnalysisSpec::new(
        "res-snap",
        "xy-1",
        AnalysisKind::ImportedSnapshot(ImportedSnapshotSpec { original_kind_label: String::from("Cached Fit") }),
    )
    .unwrap();
    let mut snap_res = ResultSheet::new_imported(snap_spec, DESC_PRISM.id, "Cached Fit", "unverified mapping").unwrap();
    snap_res.tabs.push(ResultTab {
        id: String::from("cached-main"),
        title: String::from("Cached Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric"), String::from("Value")],
        row_labels: vec![String::from("Row 1")],
        rows: vec![vec![ResultCell::Text(String::from("IC50")), ResultCell::Number { value: 42.0, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(snap_res).unwrap();

    // Package as a `.prism` ZIP containing `document.json` so `import_prism_bytes` yields both sheets intact.
    let doc_bytes = serde_json::to_vec(&doc).unwrap();
    let mut zip = ZipWriter::new();
    zip.add_file("document.json", doc_bytes).unwrap();
    let b64 = encode_base64(&zip.finish().unwrap());

    let mut session = Session::new();
    let before_doc = session.document().clone();
    let outcome = ok_cmd(&mut session, "table.importPrism", json!({ "bytes_base64": b64 }));
    assert!(outcome.mutated);
    assert!(
        outcome.payload["recalculated_sheets"].as_u64().unwrap_or(0) >= 1,
        "expected live-translated analysis recalculation count in payload: {:?}",
        outcome.payload
    );

    let live_after = session.document().result_sheet(&SheetId::new("res-live")).unwrap();
    assert_eq!(live_after.status, ResultStatus::UpToDate, "live-translated analysis must be recalculated during table.importPrism");
    assert!(!live_after.tabs.is_empty(), "recalculated live analysis must populate result tabs");

    let snap_after = session.document().result_sheet(&SheetId::new("res-snap")).unwrap();
    assert_eq!(snap_after.status, ResultStatus::Imported, "sibling ImportedSnapshot must remain untouched as ResultStatus::Imported");
    assert_eq!(snap_after.tabs[0].id, "cached-main");

    // Single undo step must roll back the entire import + recalculation cleanly.
    session.undo().unwrap();
    assert_eq!(session.document(), &before_doc);
}

fn build_promoted_archive_fixture_bytes() -> Vec<u8> {
    let mut zip = ZipWriter::new();
    let add_json = |zw: &mut ZipWriter, path: &str, value: serde_json::Value| {
        zw.add_file(path, serde_json::to_vec(&value).unwrap()).unwrap();
    };

    add_json(
        &mut zip,
        "document.json",
        json!({
            "formatVersion": "1-6-0",
            "title": "Promoted Archive Fixture",
            "sheets": {
                "data": ["d-groups", "d-single"],
                "info": [],
                "analyses": ["an-ttest", "an-anova-ok", "an-stale", "an-fallback"],
                "graphs": [],
                "layouts": []
            },
            "sheetAttributesMap": {
                "d-groups": { "title": "Dose Groups" },
                "d-single": { "title": "Single Arm" },
                "an-ttest": { "title": "Agreeing T-Test" },
                "an-anova-ok": { "title": "Agreeing ANOVA" },
                "an-stale": { "title": "Stale ANOVA" },
                "an-fallback": { "title": "Degenerate T-Test", "highlightColor": "#224488" }
            }
        }),
    );

    // 3-group Column table: Vehicle=[10,12,14], Low=[20,22,24], High=[30,32,34]
    add_json(
        &mut zip,
        "data/sheets/d-groups/sheet.json",
        json!({
            "uid": "d-groups",
            "title": "Dose Groups",
            "table": {
                "uid": "t-groups",
                "format": "column",
                "dataFormat": "y_single",
                "dataSets": ["ds-veh", "ds-low", "ds-high"]
            }
        }),
    );
    add_json(&mut zip, "data/sets/ds-veh.json", json!({"title": "Vehicle", "replicates": [{"valueType": "real"}]}));
    add_json(&mut zip, "data/sets/ds-low.json", json!({"title": "Low", "replicates": [{"valueType": "real"}]}));
    add_json(&mut zip, "data/sets/ds-high.json", json!({"title": "High", "replicates": [{"valueType": "real"}]}));
    zip.add_file("data/tables/t-groups/data.csv", b"10,20,30\n12,22,32\n14,24,34\n".to_vec()).unwrap();

    // 1-group degenerate Column table for failing promoted t-test fallback.
    add_json(
        &mut zip,
        "data/sheets/d-single/sheet.json",
        json!({
            "uid": "d-single",
            "title": "Single Arm",
            "table": {
                "uid": "t-single",
                "format": "column",
                "dataFormat": "y_single",
                "dataSets": ["ds-only"]
            }
        }),
    );
    add_json(&mut zip, "data/sets/ds-only.json", json!({"title": "OnlyGroup", "replicates": [{"valueType": "real"}]}));
    zip.add_file("data/tables/t-single/data.csv", b"1\n2\n3\n".to_vec()).unwrap();

    // 1. Agreeing promoted two-tailed unpaired t-test on Vehicle vs Low:
    //    t = -6.123724356957945, df = 4, two-tailed p = 0.0036022326091
    add_json(
        &mut zip,
        "analyses/an-ttest/sheet.json",
        json!({
            "uid": "an-ttest",
            "title": "Agreeing T-Test",
            "analysisClass": "T_TESTS_AND_NONPARAMETRIC",
            "isAllDataAnalyzed": false,
            "inputSheets": [{ "uid": "d-groups" }],
            "inputDataSets": [{ "uid": "ds-veh" }, { "uid": "ds-low" }]
        }),
    );
    add_json(
        &mut zip,
        "analyses/an-ttest/parameters.json",
        json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}}),
    );
    add_json(
        &mut zip,
        "analyses/an-ttest/results.json",
        json!({"content": {"unpairedTTestModel": {"unpairedTTest": {
            "t": -6.123_724_356_957_945,
            "pValue": { "state": "P_VALUE_EQUAL", "value": 0.003_602_232_609_1 }
        }}}}),
    );

    // 2. Agreeing promoted ordinary one-way ANOVA with Tukey post-test on d-groups:
    //    F(2,6) = 75.0, MS_residual = 4.0, R^2 = 25/26, P = 1/17576; Tukey Vehicle vs Low |diff| = 10.0, p < 0.01
    let anova_tukey_params = json!({"content": {
        "experimentalDesign": "ORDINARY_ANOVA",
        "postTests": {
            "type": "PAIRWISE_TESTS",
            "pairwiseTests": {
                "pairwiseDesign": { "type": "ALL_VS_ALL" },
                "multipleComparisons": { "type": "STATISTICAL", "statistical": { "test": "TUKEY" } }
            }
        }
    }});
    add_json(
        &mut zip,
        "analyses/an-anova-ok/sheet.json",
        json!({
            "uid": "an-anova-ok",
            "title": "Agreeing ANOVA",
            "analysisClass": "ONE_WAY_ANOVA",
            "isAllDataAnalyzed": true,
            "inputSheets": [{ "uid": "d-groups" }]
        }),
    );
    add_json(&mut zip, "analyses/an-anova-ok/parameters.json", anova_tukey_params.clone());
    add_json(
        &mut zip,
        "analyses/an-anova-ok/results.json",
        json!({"content": {"ordinaryAnovaModel": {
            "anovaTable": {
                "colFactor": {
                    "fTest": { "f": 75.0, "pValue": { "state": "P_VALUE_EQUAL", "value": 1.0_f64 / 17_576.0 } },
                    "r2": 25.0_f64 / 26.0
                },
                "residual": { "ms": 4.0 }
            },
            "multipleComparisons": {
                "tests": {
                    "c0": {
                        "id": { "col": { "idx": 0 } },
                        "value": {
                            "c1": {
                                "id": { "col": { "idx": 1 } },
                                "value": {
                                    "status": "VALID",
                                    "meansDifference": -10.0,
                                    "pValueAdjusted": { "state": "P_VALUE_LESS", "value": 0.01 }
                                }
                            }
                        }
                    }
                }
            }
        }}}),
    );

    // 3. Stale-cache disagreement: promoted ordinary one-way ANOVA + Tukey with altered omnibus P and pairwise P_VALUE_LESS
    add_json(
        &mut zip,
        "analyses/an-stale/sheet.json",
        json!({
            "uid": "an-stale",
            "title": "Stale ANOVA",
            "analysisClass": "ONE_WAY_ANOVA",
            "isAllDataAnalyzed": true,
            "inputSheets": [{ "uid": "d-groups" }]
        }),
    );
    add_json(&mut zip, "analyses/an-stale/parameters.json", anova_tukey_params);
    add_json(
        &mut zip,
        "analyses/an-stale/results.json",
        json!({"content": {"ordinaryAnovaModel": {
            "anovaTable": {
                "colFactor": {
                    "fTest": { "f": 75.0, "pValue": { "state": "P_VALUE_EQUAL", "value": 0.156 } }
                }
            },
            "multipleComparisons": {
                "tests": {
                    "c0": {
                        "id": { "col": { "idx": 0 } },
                        "value": {
                            "c1": {
                                "id": { "col": { "idx": 1 } },
                                "value": {
                                    "status": "VALID",
                                    "meansDifference": -10.0,
                                    "pValueAdjusted": { "state": "P_VALUE_LESS", "value": 1e-15 }
                                }
                            }
                        }
                    }
                }
            }
        }}}),
    );

    // 4. Failing promoted two-tailed unpaired t-test targeting 1-column table with 3-hop cached result tab
    add_json(
        &mut zip,
        "analyses/an-fallback/sheet.json",
        json!({
            "uid": "an-fallback",
            "title": "Degenerate T-Test",
            "analysisClass": "T_TESTS_AND_NONPARAMETRIC",
            "inputSheets": [{ "uid": "d-single" }],
            "preferredResultSheet": "rs-fb",
            "resultSheets": [{ "uid": "rs-fb", "title": "Cached Fallback" }]
        }),
    );
    add_json(
        &mut zip,
        "analyses/an-fallback/parameters.json",
        json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}}),
    );
    add_json(
        &mut zip,
        "analyses/an-fallback/results.json",
        json!({"content": {"unpairedTTestModel": {"unpairedTTest": {"pValue": {"state": "P_VALUE_EQUAL", "value": 0.042}}}}}),
    );
    add_json(&mut zip, "analyses/an-fallback/result_sheets/rs-fb.json", json!({"uid": "rs-fb", "name": "Cached Fallback", "dataSheet": "view-fb"}));
    add_json(
        &mut zip,
        "data/sheets/view-fb/sheet.json",
        json!({
            "uid": "view-fb",
            "flags": ["ANALYSIS_VIEW"],
            "table": {
                "uid": "t-view-fb",
                "format": "view",
                "dataFormat": "text",
                "rowTitlesDataSet": "ds-fb-rt",
                "dataSets": ["ds-fb-val"]
            }
        }),
    );
    add_json(&mut zip, "data/sets/ds-fb-rt.json", json!({"title": "Metric", "replicates": [{"valueType": "string"}]}));
    add_json(&mut zip, "data/sets/ds-fb-val.json", json!({"title": "Value", "replicates": [{"valueType": "string"}]}));
    zip.add_file("data/tables/t-view-fb/data.csv", b"P value,0.042\n".to_vec()).unwrap();

    zip.finish().unwrap()
}

fn assert_curvebook_roundtrip_strips_transient_refs(doc: &ProjectDocument) {
    let cb_bytes = save_project_bytes(doc).unwrap();
    let cb_zip = ZipArchive::from_bytes(&cb_bytes).unwrap();
    let doc_json_str = cb_zip.get_utf8("document.json").unwrap().unwrap();
    assert!(!doc_json_str.contains("0.156"), "stale cached omnibus P must not persist to .curvebook");
    assert!(!doc_json_str.contains("1e-15"), "stale cached pairwise P must not persist to .curvebook");
    assert!(!doc_json_str.contains("promoted_analyses"), "transient promoted_analyses must not persist to .curvebook");
    let reopened = open_project_bytes(&cb_bytes).unwrap();
    assert_eq!(reopened.result_sheet(&SheetId::new("an-ttest")).unwrap().status, ResultStatus::UpToDate);
    assert_eq!(reopened.result_sheet(&SheetId::new("an-stale")).unwrap().status, ResultStatus::UpToDate);
    assert_eq!(reopened.result_sheet(&SheetId::new("an-fallback")).unwrap().status, ResultStatus::Imported);
    assert_eq!(reopened.result_sheet(&SheetId::new("an-fallback")).unwrap().tabs.len(), 1);
}

fn assert_load_project_from_bytes_path(zip_bytes: &[u8]) {
    let loaded = load_project_from_bytes(Path::new("promoted_study.prism"), zip_bytes, false).unwrap();
    assert_eq!(loaded.format_tag, "prism");
    assert_eq!(loaded.doc.result_sheets.len(), 4);

    // Agreeing analyses: live UpToDate, Computed, zero warnings targeting them.
    for id in ["an-ttest", "an-anova-ok"] {
        let rs = loaded.doc.result_sheet(&SheetId::new(id)).unwrap();
        assert_eq!(rs.status, ResultStatus::UpToDate, "{id} should be UpToDate");
        assert_eq!(rs.provenance, ResultProvenance::Computed, "{id} should be Computed");
        assert!(!loaded.warnings.iter().any(|w| w.contains(&format!("analyses/{id}"))), "unexpected warning for {id}: {:?}", loaded.warnings);
    }

    // Stale-cache analysis: live UpToDate, CurveBook's recomputed values stand, typed warnings emitted.
    let stale = loaded.doc.result_sheet(&SheetId::new("an-stale")).unwrap();
    assert_eq!(stale.status, ResultStatus::UpToDate);
    assert_eq!(stale.provenance, ResultProvenance::Computed);
    let live_omnibus_p = stale.lookup_scalar("P value", None).unwrap();
    assert!((live_omnibus_p - (1.0 / 17_576.0)).abs() < 1e-10, "CurveBook P must stand, got {live_omnibus_p}");
    let pair01 = stale.pairwise_comparisons.iter().find(|c| (c.group_a.min(c.group_b), c.group_a.max(c.group_b)) == (0, 1)).unwrap();
    assert!((pair01.estimate_diff.unwrap().abs() - 10.0).abs() < 1e-9);
    assert!(pair01.raw_p_or_q > 1e-4, "CurveBook Tukey p must stand, got {}", pair01.raw_p_or_q);
    assert!(
        loaded.warnings.iter().any(|w| {
            w.contains("[RecomputedResultDisagreement]")
                && w.contains("Stale ANOVA")
                && w.contains("P value")
                && w.contains("0.156")
                && w.contains("CurveBook's value stands")
        }),
        "missing omnibus disagreement warning: {:?}",
        loaded.warnings
    );
    assert!(
        loaded.warnings.iter().any(|w| {
            w.contains("[RecomputedResultDisagreement]")
                && w.contains("Stale ANOVA")
                && w.contains("Vehicle vs Low adjusted p-value")
                && w.contains("p < 1e-15")
        }),
        "missing pairwise P_VALUE_LESS disagreement warning: {:?}",
        loaded.warnings
    );

    // Degenerate t-test fallback: ImportedSnapshot with preserved 3-hop tab & color tag + typed warning.
    let fb = loaded.doc.result_sheet(&SheetId::new("an-fallback")).unwrap();
    assert_eq!(fb.status, ResultStatus::Imported);
    assert!(matches!(fb.provenance, ResultProvenance::Imported { ref source_format, .. } if source_format == "prism"));
    assert!(matches!(fb.analysis.kind, AnalysisKind::ImportedSnapshot(_)));
    assert_eq!(fb.color_tag, Some(Rgba8::rgb(0x22, 0x44, 0x88)));
    assert_eq!(fb.tabs.len(), 1);
    assert_eq!(fb.tabs[0].title, "Cached Fallback");
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| { w.contains("[AnalysisRecomputeFallback]") && w.contains("Degenerate T-Test") && w.contains("kept cached snapshot") }),
        "missing fallback warning: {:?}",
        loaded.warnings
    );

    assert_curvebook_roundtrip_strips_transient_refs(&loaded.doc);
}

fn assert_table_import_prism_replace_path(b64: &str) {
    let mut session = Session::new();
    let before_doc = session.document().clone();
    let outcome = ok_cmd(&mut session, "table.importPrism", json!({ "bytes_base64": b64 }));
    assert_eq!(outcome.payload["failed_count"], json!(0), "fallback must be removed from failed_result_sheets: {:?}", outcome.payload);
    let payload_warns = outcome.payload["warnings"].as_array().unwrap();
    assert!(payload_warns.iter().any(|w| w["kind"] == json!("RecomputedResultDisagreement")));
    assert!(payload_warns.iter().any(|w| w["kind"] == json!("AnalysisRecomputeFallback")));
    assert!(outcome.warnings.iter().any(|w| w.contains("[RecomputedResultDisagreement]")));
    assert!(outcome.warnings.iter().any(|w| w.contains("[AnalysisRecomputeFallback]")));

    let p_before_edit = session.document().result_sheet(&SheetId::new("an-ttest")).unwrap().lookup_scalar("PValue", None).unwrap();
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "d-groups", "row": 0, "col": 0, "value": 19.0 }));
    let p_after_edit = session.document().result_sheet(&SheetId::new("an-ttest")).unwrap().lookup_scalar("PValue", None).unwrap();
    assert!(
        (p_after_edit - p_before_edit).abs() > 1e-3,
        "live cell edit must recompute promoted t-test: before={p_before_edit}, after={p_after_edit}"
    );

    // Undo the cell edit, then a single undo rolls back the entire import + recompute + fallback.
    session.undo().unwrap();
    session.undo().unwrap();
    assert_eq!(session.document(), &before_doc);
}

fn assert_table_import_prism_merge_collision_path(b64: &str) {
    let mut base_doc = ProjectDocument::new("Existing Study").unwrap();
    for colliding_data_id in ["d-groups", "d-single"] {
        let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
        tbl.data_sets[0].title = String::from("Existing");
        tbl.data_sets[0].cells[0][0] = curvebook_doc::Cell::number(1.0);
        tbl.data_sets[0].cells[1][0] = curvebook_doc::Cell::number(2.0);
        base_doc.add_data_sheet(DataSheet::new(colliding_data_id, "Pre-existing Data", TableArchetype::Column(tbl)).unwrap()).unwrap();
    }
    for colliding_res_id in ["an-ttest", "an-anova-ok", "an-stale", "an-fallback"] {
        let spec = AnalysisSpec::new(colliding_res_id, "d-groups", AnalysisKind::DescriptiveStats(Default::default())).unwrap();
        base_doc.add_result_sheet(ResultSheet::new(spec).unwrap()).unwrap();
    }
    curvebook_engine::dag::recalculate_project(&mut base_doc).unwrap();
    let mut merge_session = Session::from_document(base_doc.clone());
    let merged = ok_cmd(&mut merge_session, "table.importPrism", json!({ "bytes_base64": b64, "merge": true, "merged_at": "unix:1700000123" }));
    assert_eq!(merged.payload["failed_count"], json!(0), "merged fallback must not be counted as failed: {:?}", merged.payload);
    let merge_warns = merged.payload["warnings"].as_array().unwrap();
    assert!(
        merge_warns.iter().any(|w| w["kind"] == json!("RecomputedResultDisagreement") && w["message"].as_str().unwrap_or("").contains("Stale ANOVA")),
        "SheetId remapping on merge must preserve disagreement detection: {merge_warns:?}"
    );
    assert!(
        merge_warns
            .iter()
            .any(|w| w["kind"] == json!("AnalysisRecomputeFallback") && w["message"].as_str().unwrap_or("").contains("Degenerate T-Test")),
        "SheetId remapping on merge must preserve fallback warning: {merge_warns:?}"
    );

    // Pre-existing sheets remain untouched; collided imported sheets were renumbered with `-2`.
    assert_eq!(merge_session.document().data_sheet(&SheetId::new("d-groups")).unwrap().title, "Pre-existing Data");
    assert_eq!(merge_session.document().data_sheet(&SheetId::new("d-groups-2")).unwrap().title, "Dose Groups");
    assert_eq!(merge_session.document().result_sheet(&SheetId::new("an-ttest-2")).unwrap().status, ResultStatus::UpToDate);
    assert_eq!(merge_session.document().result_sheet(&SheetId::new("an-stale-2")).unwrap().status, ResultStatus::UpToDate);
    let merged_fb = merge_session.document().result_sheet(&SheetId::new("an-fallback-2")).unwrap();
    assert_eq!(merged_fb.status, ResultStatus::Imported);
    assert!(matches!(merged_fb.analysis.kind, AnalysisKind::ImportedSnapshot(_)));
    assert_eq!(merged_fb.analysis.source_sheet_id, Some(SheetId::new("d-single-2")));
    assert_eq!(merged_fb.tabs.len(), 1);
    assert_eq!(merged_fb.tabs[0].title, "Cached Fallback");
    let fb_origin = merged_fb.origin.as_ref().expect("merged fallback must retain SheetOrigin");
    assert_eq!(fb_origin.source_label, "inline.prism");
    assert_eq!(fb_origin.merged_at, "unix:1700000123");

    merge_session.undo().unwrap();
    assert_eq!(merge_session.document(), &base_doc);
}

#[test]
fn promoted_archive_analyses_recompute_warn_fallback_and_remap_across_all_engine_paths() {
    let zip_bytes = build_promoted_archive_fixture_bytes();
    let b64 = encode_base64(&zip_bytes);
    assert_load_project_from_bytes_path(&zip_bytes);
    assert_table_import_prism_replace_path(&b64);
    assert_table_import_prism_merge_collision_path(&b64);
}
