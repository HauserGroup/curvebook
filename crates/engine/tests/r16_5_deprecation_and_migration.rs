//! Integration tests for canonical command schemas, Stage-4-retained deprecation aliases,
//! `ops.journal` v1 -> v2 raw-CRC-first migration + clean `Caller::Replay`, and `curvebook.batch.v1` normalization.

use std::sync::atomic::AtomicBool;

use curvebook_engine::cmd::registry;
use curvebook_engine::{Caller, ErrorCode, Session};
use curvebook_format::{
    BATCH_SCHEMA_V1, CURRENT_JOURNAL_VERSION, JournalBackend, JournalOpRecord, MemoryJournalBackend, compute_record_crc32,
    normalize_batch_manifest_v1, normalize_batch_manifest_v1_with_warnings, normalize_command_params_in_place,
};
use serde_json::{Value, json};

#[test]
fn only_stage4_retained_aliases_remain_in_registry_schemas() {
    fn assert_no_legacy_x_aliases(val: &Value, command_id: &str) {
        match val {
            Value::Object(map) => {
                assert!(!map.contains_key("x-aliases"), "command `{command_id}` schema still emits retired `x-aliases`");
                for v in map.values() {
                    assert_no_legacy_x_aliases(v, command_id);
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_no_legacy_x_aliases(item, command_id);
                }
            }
            _ => {}
        }
    }

    let mut alias_commands = Vec::new();
    for spec in registry().specs() {
        let schema = (spec.input_schema)();
        assert_no_legacy_x_aliases(&schema, spec.id);
        if let Some(alias_map) = schema.get("x-deprecated-aliases") {
            alias_commands.push((spec.id, alias_map.clone()));
        }
    }
    assert_eq!(
        alias_commands,
        vec![
            ("graph.create", json!({ "data_sheet_id": ["source_sheet_id"] })),
            ("info.setNote", json!({ "text": ["note"] })),
            ("sheet.createData", json!({ "kind": ["archetype"], "cols": ["data_sets"], "replicates": ["subcolumns"] })),
            ("sheet.rename", json!({ "sheet_id": ["id"], "title": ["new_title"] })),
            ("table.importCsv", json!({ "csv": ["text"], "archetype": ["kind"] })),
            ("table.setCategoricalOrder", json!({ "col": ["column"] })),
            ("table.setCell", json!({ "value": ["raw"] })),
        ],
        "only the Stage-3/Stage-4-retained locked-caller aliases may emit x-deprecated-aliases"
    );

    // Retired aliases outside the Stage-3/Stage-4-retained entries must now fail with BadInput.
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "rows": 3, "cols": 1 })).expect("create d1");
    let bad_cells = session
        .execute("table.setCells", json!({ "sheet_id": "d1", "cells": [{ "row": 0, "col": 0, "value": 1.0 }] }))
        .expect_err("retired cells alias rejected");
    assert_eq!(bad_cells.code, ErrorCode::BadInput);
    let bad_preview = session
        .execute("table.convertArchetype", json!({ "sheet_id": "d1", "target_kind": "column", "previewOnly": true }))
        .expect_err("retired previewOnly alias rejected");
    assert_eq!(bad_preview.code, ErrorCode::BadInput);
}

#[test]
fn canonical_calls_emit_zero_warnings_and_retained_aliases_warn_only_live_callers() {
    let mut session = Session::new();
    let cancel = AtomicBool::new(false);

    let clean = session
        .execute_with_caller(
            "sheet.createData",
            json!({ "sheet_id": "d1", "title": "Dose", "kind": "xy", "rows": 4, "cols": 1 }),
            Caller::Mcp,
            &cancel,
        )
        .expect("canonical createData");
    assert!(clean.warnings.is_empty(), "expected zero warnings for canonical params: {:?}", clean.warnings);

    // Retained Stage-4 alias on `sheet.rename` warns live callers across Ui/Cli/Mcp/Control.
    for (idx, caller) in [Caller::Ui, Caller::Cli, Caller::Mcp, Caller::Control].iter().copied().enumerate() {
        let out = session
            .execute_with_caller("sheet.rename", json!({ "id": "d1", "new_title": format!("Renamed {idx}") }), caller, &cancel)
            .expect("Stage-4-retained sheet.rename aliases still succeed");
        assert!(out.mutated);
        assert_eq!(out.warnings.len(), 2, "expected 2 warnings for {caller:?}, got {:?}", out.warnings);
        assert!(out.warnings.iter().any(|w| w.contains("`id`") && w.contains("`sheet_id`") && w.contains("sheet.rename")));
        assert!(out.warnings.iter().any(|w| w.contains("`new_title`") && w.contains("`title`") && w.contains("sheet.rename")));
    }

    // Caller::Replay suppresses deprecation warnings.
    let replay_out = session
        .execute_with_caller("sheet.rename", json!({ "id": "d1", "new_title": "Replayed" }), Caller::Replay, &cancel)
        .expect("replay succeeds");
    assert!(replay_out.warnings.is_empty(), "Replay caller must suppress deprecation warnings, got {:?}", replay_out.warnings);
}

#[test]
fn ops_journal_v1_migrates_to_v2_crc_first_replays_cleanly_and_new_appends_write_v2_canonical() {
    // 1. Build a v1 ops.journal manually using retired parameter spellings + valid v1 CRC32.
    let v1_commands = [
        json!({
            "command_id": "sheet.createData",
            "params": { "id": "xy1", "name": "Legacy XY", "archetype": "xy", "row_count": 3, "columns": 1, "autoGraph": false }
        }),
        json!({
            "command_id": "table.setCell",
            "params": { "sheetId": "xy1", "coordinate": { "row": 0, "column": 0 }, "value": 42.5 }
        }),
        json!({
            "command_id": "sheet.rename",
            "params": { "id": "xy1", "new_title": "Migrated XY" }
        }),
        json!({
            "command_id": "graph.create",
            "params": { "id": "g1", "source_sheet": "xy1", "kind": "xy" }
        }),
    ];

    let mut backend = MemoryJournalBackend::new();
    for (idx, cmd) in v1_commands.iter().enumerate() {
        let seq = (idx + 1) as u64;
        let crc32 = compute_record_crc32(seq, cmd).unwrap();
        let line = json!({
            "format_version": 1,
            "seq": seq,
            "crc32": crc32,
            "command_json": cmd,
        })
        .to_string();
        backend.append_journal_line(&line).unwrap();
    }

    // 2. Open & recover: v1 records upgrade cleanly to canonical v2 commands and replay via Caller::Replay without warnings.
    let (mut session, mut journal, report) = Session::open_with_recovery(backend, "r165-test", 10_000, 1_000).expect("open v1 journal cleanly");
    assert!(report.warnings.is_empty(), "clean v1 replay should emit zero recovery warnings: {:?}", report.warnings);
    assert_eq!(report.recovered_commands.len(), 4);
    assert_eq!(report.recovered_commands[2], json!({ "command_id": "sheet.rename", "params": { "sheet_id": "xy1", "title": "Migrated XY" } }));
    assert_eq!(session.document().data_sheets[0].title, "Migrated XY");
    assert_eq!(session.document().graph_sheets.len(), 1);

    // 3. New append using Stage-4-retained alias warns live caller AND writes format_version=2 with canonical params.
    let cancel = AtomicBool::new(false);
    let live_out = session
        .execute_with_caller_and_record(&mut journal, "sheet.rename", json!({ "id": "xy1", "new_title": "Updated XY" }), Caller::Cli, &cancel)
        .expect("live execute_and_record");
    assert_eq!(live_out.warnings.len(), 2);

    let journal_text = std::str::from_utf8(journal.backend().journal_bytes()).unwrap();
    let tail_rec = JournalOpRecord::from_json_str(journal_text.lines().last().unwrap()).unwrap();
    assert_eq!(tail_rec.format_version, CURRENT_JOURNAL_VERSION);
    assert_eq!(tail_rec.command_json, json!({ "command_id": "sheet.rename", "params": { "sheet_id": "xy1", "title": "Updated XY" } }));

    // 4. Tampering raw persisted v1 payload (without updating v1 CRC32) fails CRC verification before upgrade.
    let mut tampered = MemoryJournalBackend::new();
    let mut bad_cmd = v1_commands[0].clone();
    bad_cmd["params"]["name"] = json!("Tampered");
    let orig_v1_crc = compute_record_crc32(1, &v1_commands[0]).unwrap();
    tampered
        .append_journal_line(
            &json!({
                "format_version": 1,
                "seq": 1,
                "crc32": orig_v1_crc,
                "command_json": bad_cmd,
            })
            .to_string(),
        )
        .unwrap();
    let err = Session::open_with_recovery(tampered, "tampered", 10_000, 1_000).expect_err("tampered v1 record must fail CRC check");
    assert!(err.to_string().contains("checksum") || err.to_string().contains("CRC") || err.to_string().contains("crc"), "got: {err}");
}

#[test]
fn batch_manifest_v1_normalization_upgrades_commands_to_zero_warning_execution() {
    let mut manifest = json!({
        "schema": BATCH_SCHEMA_V1,
        "inputs": [{ "csv": "data.csv", "sheetId": "raw", "tableKind": "xy", "subcolumnMode": "replicate", "decimalSeparator": "." }],
        "commands": [
            { "command": "sheet.createData", "params": { "id": "d1", "name": "Assay", "archetype": "xy", "rows": 4, "cols": 1 } },
            { "method": "table.setXCells", "params": { "sheetId": "d1", "x": [1.0, 2.0, 3.0] } },
            { "method": "table.setCells", "params": { "sheetId": "d1", "cells": [{ "row": 0, "column": 0, "value": 2.0 }, { "row": 1, "column": 0, "value": 4.0 }, { "row": 2, "column": 0, "value": 6.0 }] } },
            { "cmd": "analysis.run", "params": { "id": "fit1", "source_sheet": "d1", "kind": "linear", "deming_model_ii": true } },
            { "cmd": "graph.create", "params": { "id": "g1", "source_sheet_id": "d1", "kind": "xy" } }
        ],
        "exports": [{ "out": "fit.csv", "sheetId": "fit1", "tabIndex": 0, "pdfVersion": "1.7", "ttfFont": "font.ttf" }]
    });

    let warnings = normalize_batch_manifest_v1_with_warnings(&mut manifest).expect("normalize batch manifest");
    assert!(!warnings.is_empty());

    // Executing normalized commands yields zero deprecation warnings.
    let mut session = Session::new();
    let cancel = AtomicBool::new(false);
    for cmd_obj in manifest["commands"].as_array().unwrap() {
        let cmd_id = cmd_obj["cmd"].as_str().unwrap();
        let params = cmd_obj["params"].clone();
        let out = session.execute_with_caller(cmd_id, params, Caller::Cli, &cancel).unwrap();
        assert!(out.warnings.is_empty(), "normalized batch command `{cmd_id}` emitted unexpected deprecation warning: {:?}", out.warnings);
    }
    assert!(normalize_batch_manifest_v1(&mut manifest).is_ok());

    // Direct normalize_command_params_in_place preserves template.wells vs top-level roles polarity.
    let mut plate_params = json!({
        "sheetId": "plate1",
        "wells": [{ "type": "blank" }],
        "template": {
            "plateGeometry": "wells_96",
            "roles": [{ "kind": "blank" }]
        }
    });
    let _ = normalize_command_params_in_place("table.importPlateMap", &mut plate_params);
    assert_eq!(
        plate_params,
        json!({
            "sheet_id": "plate1",
            "roles": [{ "role": "blank" }],
            "template": {
                "geometry": "wells_96",
                "wells": [{ "role": "blank" }]
            }
        })
    );
}
