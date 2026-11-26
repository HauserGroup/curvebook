//! Integration tests for canonical command schemas, Stage-4-retained deprecation aliases,
//! `ops.journal` v1 -> v2 raw-CRC-first migration + clean `Caller::Replay`, and `curvebook.batch.v1` normalization.

use std::sync::atomic::AtomicBool;

use curvebook_engine::cmd::registry;
use curvebook_engine::{Caller, Session};
use curvebook_format::{
    BATCH_SCHEMA_V1, CURRENT_JOURNAL_VERSION, JournalBackend, JournalOpRecord, MemoryJournalBackend, compute_record_crc32,
    normalize_batch_manifest_v1, normalize_batch_manifest_v1_with_warnings, normalize_command_params_in_place,
};
use serde_json::{Value, json};

use super::common::{assert_bad_inputs, ok_cmd};

#[test]
fn only_stage4_retained_aliases_remain_in_registry_schemas() {
    fn assert_no_legacy_x_aliases(val: &Value, command_id: &str) {
        match val {
            Value::Object(map) => {
                assert!(!map.contains_key("x-aliases"), "command `{command_id}` schema still emits retired `x-aliases`");
                map.values().for_each(|v| assert_no_legacy_x_aliases(v, command_id));
            }
            Value::Array(items) => items.iter().for_each(|v| assert_no_legacy_x_aliases(v, command_id)),
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
        vec![("sheet.createData", json!({ "kind": ["archetype"] })), ("sheet.rename", json!({ "sheet_id": ["id"], "title": ["new_title"] }))],
        "only the R16.5 CLI deprecation-window aliases may emit x-deprecated-aliases"
    );

    // Retired aliases outside the remaining retained entries must now fail with BadInput.
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "rows": 3, "cols": 1 }));
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "mv1", "kind": "multiple_variables", "rows": 2, "cols": 1 }));
    assert_bad_inputs(
        &mut session,
        &[
            ("table.setCells", json!({ "sheet_id": "d1", "cells": [{ "row": 0, "col": 0, "value": 1.0 }] })),
            ("table.convertArchetype", json!({ "sheet_id": "d1", "target_kind": "column", "previewOnly": true })),
            ("graph.create", json!({ "source_sheet_id": "d1" })),
            ("table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "raw": "1.5" })),
            ("table.importCsv", json!({ "text": "x,y\n1,2\n", "kind": "xy" })),
            ("table.setCategoricalOrder", json!({ "sheet_id": "mv1", "column": 0, "order": ["A"] })),
            (
                "table.importPlateMap",
                json!({ "values": vec![1.0_f64; 96], "template": { "name": "P96", "geometry": "96", "x_title": "Conc", "wells": [{ "kind": "treatment", "group": "Drug", "x": 1.0, "rep": 1 }] } }),
            ),
        ],
    );
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
    assert!(clean.warnings.is_empty(), "expected zero warnings: {:?}", clean.warnings);

    for (idx, caller) in [Caller::Ui, Caller::Cli, Caller::Mcp, Caller::Control].iter().copied().enumerate() {
        let out = session
            .execute_with_caller("sheet.rename", json!({ "id": "d1", "new_title": format!("Renamed {idx}") }), caller, &cancel)
            .expect("retained alias");
        assert!(out.mutated && out.warnings.len() == 2);
        assert!(out.warnings.iter().any(|w| w.contains("`id`") && w.contains("`sheet_id`") && w.contains("sheet.rename")));
        assert!(out.warnings.iter().any(|w| w.contains("`new_title`") && w.contains("`title`") && w.contains("sheet.rename")));
    }

    let replay_out =
        session.execute_with_caller("sheet.rename", json!({ "id": "d1", "new_title": "Replayed" }), Caller::Replay, &cancel).expect("replay");
    assert!(replay_out.warnings.is_empty(), "Replay caller must suppress deprecation warnings: {:?}", replay_out.warnings);
}

#[test]
fn ops_journal_v1_migrates_to_v2_crc_first_replays_cleanly_and_new_appends_write_v2_canonical() {
    let v1_commands = [
        json!({ "command_id": "sheet.createData", "params": { "id": "xy1", "name": "Legacy XY", "archetype": "xy", "row_count": 3, "columns": 1, "autoGraph": false } }),
        json!({ "command_id": "table.setCell", "params": { "sheetId": "xy1", "coordinate": { "row": 0, "column": 0 }, "value": 42.5 } }),
        json!({ "command_id": "sheet.rename", "params": { "id": "xy1", "new_title": "Migrated XY" } }),
        json!({ "command_id": "graph.create", "params": { "id": "g1", "source_sheet": "xy1", "kind": "xy" } }),
    ];

    let mut backend = MemoryJournalBackend::new();
    for (idx, cmd) in v1_commands.iter().enumerate() {
        let seq = (idx + 1) as u64;
        let line = json!({ "format_version": 1, "seq": seq, "crc32": compute_record_crc32(seq, cmd).unwrap(), "command_json": cmd }).to_string();
        backend.append_journal_line(&line).unwrap();
    }

    let (mut session, mut journal, report) = Session::open_with_recovery(backend, "r165-test", 10_000, 1_000).expect("open v1 journal");
    assert!(report.warnings.is_empty() && report.recovered_commands.len() == 4);
    assert_eq!(report.recovered_commands[2], json!({ "command_id": "sheet.rename", "params": { "sheet_id": "xy1", "title": "Migrated XY" } }));
    assert_eq!(session.document().data_sheets[0].title, "Migrated XY");
    assert_eq!(session.document().graph_sheets.len(), 1);

    // Live append with Stage-4 alias warns AND persists v2 canonical.
    let cancel = AtomicBool::new(false);
    let live_out = session
        .execute_with_caller_and_record(&mut journal, "sheet.rename", json!({ "id": "xy1", "new_title": "Updated XY" }), Caller::Cli, &cancel)
        .expect("live record");
    assert_eq!(live_out.warnings.len(), 2);

    let journal_text = std::str::from_utf8(journal.backend().journal_bytes()).unwrap();
    let tail_rec = JournalOpRecord::from_json_str(journal_text.lines().last().unwrap()).unwrap();
    assert_eq!(tail_rec.format_version, CURRENT_JOURNAL_VERSION);
    assert_eq!(tail_rec.command_json, json!({ "command_id": "sheet.rename", "params": { "sheet_id": "xy1", "title": "Updated XY" } }));

    // Tampered v1 payload fails CRC verification before upgrade.
    let mut tampered = MemoryJournalBackend::new();
    let mut bad_cmd = v1_commands[0].clone();
    bad_cmd["params"]["name"] = json!("Tampered");
    let orig_crc = compute_record_crc32(1, &v1_commands[0]).unwrap();
    tampered.append_journal_line(&json!({ "format_version": 1, "seq": 1, "crc32": orig_crc, "command_json": bad_cmd }).to_string()).unwrap();
    let err = Session::open_with_recovery(tampered, "tampered", 10_000, 1_000).expect_err("tampered v1 must fail CRC");
    assert!(err.to_string().to_ascii_lowercase().contains("crc") || err.to_string().contains("checksum"), "got: {err}");
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

    assert!(!normalize_batch_manifest_v1_with_warnings(&mut manifest).expect("normalize batch").is_empty());

    let mut session = Session::new();
    let cancel = AtomicBool::new(false);
    for cmd_obj in manifest["commands"].as_array().unwrap() {
        let out = session.execute_with_caller(cmd_obj["cmd"].as_str().unwrap(), cmd_obj["params"].clone(), Caller::Cli, &cancel).unwrap();
        assert!(out.warnings.is_empty(), "normalized batch emitted warning: {:?}", out.warnings);
    }
    assert!(normalize_batch_manifest_v1(&mut manifest).is_ok());

    let mut plate_params =
        json!({ "sheetId": "plate1", "wells": [{ "type": "blank" }], "template": { "plateGeometry": "wells_96", "roles": [{ "kind": "blank" }] } });
    let _ = normalize_command_params_in_place("table.importPlateMap", &mut plate_params);
    assert_eq!(
        plate_params,
        json!({ "sheet_id": "plate1", "roles": [{ "role": "blank" }], "template": { "geometry": "wells_96", "wells": [{ "role": "blank" }] } })
    );
}
