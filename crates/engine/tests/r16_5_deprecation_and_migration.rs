//! Integration tests for R16.5: parameter deprecation warnings, schema `x-deprecated-aliases`,
//! `ops.journal` v1 -> v2 raw-CRC-first migration + clean `Caller::Replay`, and `curvebook.batch.v1` normalization.

use std::sync::atomic::AtomicBool;

use curvebook_doc::param_spellings::canonical_command_param_alias;
use curvebook_engine::cmd::registry;
use curvebook_engine::{Caller, GuiOnlyPolicy, Session};
use curvebook_format::{
    BATCH_SCHEMA_V1, CURRENT_JOURNAL_VERSION, JournalBackend, JournalOpRecord, MemoryJournalBackend, compute_record_crc32,
    normalize_batch_manifest_v1, normalize_batch_manifest_v1_with_warnings, normalize_command_params_in_place,
};
use serde_json::{Value, json};

fn collect_schema_deprecated_aliases(node: &Value, path: &[String], out: &mut Vec<(Vec<String>, String, String)>) {
    let Some(obj) = node.as_object() else {
        return;
    };
    if let Some(Value::Object(alias_map)) = obj.get("x-deprecated-aliases") {
        let props = obj.get("properties").and_then(Value::as_object);
        for (canonical, aliases_val) in alias_map {
            let Some(alias_list) = aliases_val.as_array() else {
                continue;
            };
            for alias in alias_list.iter().filter_map(Value::as_str) {
                let is_declared_prop = props.is_some_and(|p| p.contains_key(alias));
                if alias != canonical && !is_declared_prop {
                    out.push((path.to_vec(), alias.to_owned(), canonical.clone()));
                }
            }
        }
    }
    if let Some(Value::Object(props)) = obj.get("properties") {
        for (prop_name, child) in props {
            let mut next = path.to_vec();
            next.push(prop_name.clone());
            collect_schema_deprecated_aliases(child, &next, out);
        }
    }
    if let Some(items) = obj.get("items") {
        collect_schema_deprecated_aliases(items, path, out);
    }
    for kw in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(arr)) = obj.get(kw) {
            for sub in arr {
                collect_schema_deprecated_aliases(sub, path, out);
            }
        }
    }
}

#[test]
fn all_registry_schemas_use_x_deprecated_aliases_and_match_doc_migration_tables() {
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

    let mut total_checked = 0usize;
    let mut mismatches = Vec::new();
    for spec in registry().specs() {
        let schema = (spec.input_schema)();
        assert_no_legacy_x_aliases(&schema, spec.id);

        let mut aliases = Vec::new();
        collect_schema_deprecated_aliases(&schema, &[], &mut aliases);
        for (path, deprecated, canonical) in aliases {
            let path_refs: Vec<&str> = path.iter().map(String::as_str).collect();
            let resolved = canonical_command_param_alias(spec.id, &path_refs, None, &deprecated);
            if resolved != Some(canonical.as_str()) {
                mismatches.push(format!("`{}` at {:?}: `{deprecated}` -> expected `{canonical}`, got `{resolved:?}`", spec.id, path_refs));
            }
            total_checked += 1;
        }
    }
    assert!(mismatches.is_empty(), "doc param_spellings drifted from schema:\n{}", mismatches.join("\n"));
    assert!(total_checked > 100, "expected full command alias coverage, checked only {total_checked}");
}

#[test]
fn command_outcome_warnings_emit_across_live_callers_and_stay_empty_for_canonical_and_replay() {
    let mut session = Session::new();
    let cancel = AtomicBool::new(false);

    // 1. Canonical invocation emits zero warnings across callers.
    let clean = session
        .execute_with_caller(
            "sheet.createData",
            json!({ "sheet_id": "d1", "title": "Dose", "kind": "xy", "rows": 4, "cols": 1 }),
            Caller::Mcp,
            &cancel,
        )
        .expect("canonical createData");
    assert!(clean.warnings.is_empty(), "expected zero warnings for canonical params: {:?}", clean.warnings);

    // 2. Deprecated aliases emit warnings naming deprecated key, canonical key, and command id across Ui/Cli/Mcp/Control.
    for (idx, caller) in [Caller::Ui, Caller::Cli, Caller::Mcp, Caller::Control].iter().copied().enumerate() {
        let out = session
            .execute_with_caller("sheet.rename", json!({ "id": "d1", "new_title": format!("Renamed {idx}") }), caller, &cancel)
            .expect("deprecated sheet.rename should still succeed during deprecation window");
        assert!(out.mutated);
        assert_eq!(out.warnings.len(), 2, "expected 2 warnings for {caller:?}, got {:?}", out.warnings);
        assert!(out.warnings.iter().any(|w| w.contains("`id`") && w.contains("`sheet_id`") && w.contains("sheet.rename")));
        assert!(out.warnings.iter().any(|w| w.contains("`new_title`") && w.contains("`title`") && w.contains("sheet.rename")));
    }

    // 3. Read-only / preview commands (including Strict GuiOnlyPolicy) preserve warnings.
    let mut strict = Session::new();
    strict.execute("sheet.createData", json!({ "sheet_id": "xy", "kind": "xy", "rows": 3, "cols": 1 })).expect("create xy");
    strict.set_gui_only(GuiOnlyPolicy::Strict);
    let preview_out = strict
        .execute_with_caller(
            "table.convertArchetype",
            json!({ "sheet_id": "xy", "target_kind": "column", "previewOnly": true }),
            Caller::Mcp,
            &cancel,
        )
        .expect("previewOnly allowed in Strict mode");
    assert!(!preview_out.mutated);
    assert!(
        preview_out.warnings.iter().any(|w| w.contains("`previewOnly`") && w.contains("`preview_only`") && w.contains("table.convertArchetype")),
        "expected deprecation warning on previewOnly: {:?}",
        preview_out.warnings
    );

    // 4. Caller::Replay emits zero deprecation warnings even if legacy spellings reach it directly.
    let replay_out = session
        .execute_with_caller("sheet.rename", json!({ "id": "d1", "new_title": "Replayed" }), Caller::Replay, &cancel)
        .expect("replay succeeds");
    assert!(replay_out.warnings.is_empty(), "Replay caller must suppress deprecation warnings, got {:?}", replay_out.warnings);

    // 5. Nested schema & serde aliases also emit deprecation warnings.
    session
        .execute_with_caller("graph.create", json!({ "sheet_id": "g1", "source_sheet_id": "d1", "kind": "xy" }), Caller::Cli, &cancel)
        .expect("graph.create with deprecated source_sheet_id");
    let pts_out = session
        .execute_with_caller(
            "graph.formatSelectedPoints",
            json!({
                "sheet_id": "g1",
                "overrides": [{ "series_index": 0, "point_index": 0, "subcolumn": 0, "shape": "square", "stroke_width_pt": 2.0 }]
            }),
            Caller::Cli,
            &cancel,
        )
        .expect("graph.formatSelectedPoints nested aliases");
    assert!(pts_out.warnings.iter().any(|w| w.contains("`subcolumn`") && w.contains("`subcol`")));
    assert!(pts_out.warnings.iter().any(|w| w.contains("`shape`") && w.contains("`symbol`")));
    assert!(pts_out.warnings.iter().any(|w| w.contains("`stroke_width_pt`") && w.contains("`border_width_pt`")));
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

    // 3. New append using a deprecated alias still warns live caller AND writes format_version=2 with canonical params.
    let cancel = AtomicBool::new(false);
    let live_out = session
        .execute_with_caller_and_record(&mut journal, "sheet.rename", json!({ "sheetId": "xy1", "new_title": "Updated XY" }), Caller::Cli, &cancel)
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
