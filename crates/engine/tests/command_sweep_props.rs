//! Proptest command-sweep and corrupt-input fuzzing for `curvebook_engine::Session` (Gap G12).
//!
//! Covers:
//! 1. Exhaustive valid execution of every `CommandSpec` in `curvebook_engine::cmd::registry()`.
//! 2. Exhaustive hostile/malformed-input sweep across every `CommandSpec` (wrong types,
//!    non-finite/`NaN` strings, `u64::MAX` indices, nonexistent sheets, cyclic links/formulas,
//!    frozen sheet protections, malformed archives/XML/CSV).
//! 3. `proptest!` arbitrary corrupt JSON fuzzing across all registered commands.
//! 4. `proptest!` random multi-step command sequences verifying zero internal panics, strict
//!    transactional rollback on error, and exact `ProjectDocument` undo/redo equality.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{InfoSheet, ProjectDocument, TableKind};
use curvebook_engine::cmd;
use curvebook_engine::{CommandOutcome, EngineError, Session};
use proptest::collection::vec;
use proptest::prelude::TestCaseError;
use proptest::prop_assert;
use proptest::prop_assert_eq;
use proptest::proptest;
use proptest::strategy::{BoxedStrategy, Just, Strategy};
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_temp_file(stem: &str, ext: &str) -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("cb_sweep_{pid}_{n}_{stem}.{ext}"))
}

fn check_no_panic(command_id: &str, res: &Result<CommandOutcome, EngineError>) {
    if let Err(err) = res {
        assert!(!err.message().contains("recovered from internal panic"), "command `{command_id}` triggered an internal panic: {}", err.message());
    }
}

fn execute_checked(session: &mut Session, command_id: &str, params: Value) -> Result<CommandOutcome, EngineError> {
    let before = session.clone();
    let res = session.execute(command_id, params);
    check_no_panic(command_id, &res);
    if let Err(ref err) = res {
        assert_eq!(session.document(), before.document(), "failed command `{command_id}` mutated ProjectDocument: {}", err.message());
        assert_eq!(session, &before, "failed command `{command_id}` mutated Session state: {}", err.message());
    }
    res
}

fn run_valid(session: &mut Session, covered: &mut BTreeSet<&'static str>, command_id: &'static str, params: Value) -> Result<CommandOutcome, String> {
    covered.insert(command_id);
    let outcome = execute_checked(session, command_id, params).map_err(|err| format!("valid sweep command `{command_id}` failed: {err}"))?;
    assert_eq!(outcome.command_id, command_id);
    Ok(outcome)
}

fn seed_rich_fixture() -> Result<Session, String> {
    let mut doc = ProjectDocument::new("Sweep Fixture").map_err(|e| e.to_string())?;
    let info = InfoSheet::new_preseeded("info-1", "Experiment Info").map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    let mut s = Session::from_document(doc);
    let setup: [(&str, Value); 16] = [
        (
            "sheet.createData",
            json!({
                "sheet_id": "xy-1",
                "title": "Dose Response",
                "archetype": "xy",
                "cols": 2,
                "rows": 6
            }),
        ),
        (
            "table.setXCells",
            json!({
                "sheet_id": "xy-1",
                "edits": [
                    { "row": 0, "value": 0.1 },
                    { "row": 1, "value": 1.0 },
                    { "row": 2, "value": 10.0 },
                    { "row": 3, "value": 100.0 },
                    { "row": 4, "value": 1000.0 },
                    { "row": 5, "value": 10000.0 }
                ]
            }),
        ),
        (
            "table.setCells",
            json!({
                "sheet_id": "xy-1",
                "edits": [
                    { "row": 0, "col": 0, "value": 98.0 },
                    { "row": 1, "col": 0, "value": 85.0 },
                    { "row": 2, "col": 0, "value": 52.0 },
                    { "row": 3, "col": 0, "value": 21.0 },
                    { "row": 4, "col": 0, "value": 8.0 },
                    { "row": 5, "col": 0, "value": 4.0 },
                    { "row": 0, "col": 1, "value": 100.0 },
                    { "row": 1, "col": 1, "value": 90.0 },
                    { "row": 2, "col": 1, "value": 60.0 },
                    { "row": 3, "col": 1, "value": 30.0 },
                    { "row": 4, "col": 1, "value": 12.0 },
                    { "row": 5, "col": 1, "value": 6.0 }
                ]
            }),
        ),
        (
            "sheet.createData",
            json!({
                "sheet_id": "col-1",
                "title": "Column Groups",
                "archetype": "column",
                "cols": 3,
                "rows": 4
            }),
        ),
        (
            "table.setCells",
            json!({
                "sheet_id": "col-1",
                "edits": [
                    { "row": 0, "col": 0, "value": 10.0 },
                    { "row": 1, "col": 0, "value": 12.0 },
                    { "row": 2, "col": 0, "value": 11.0 },
                    { "row": 3, "col": 0, "value": 13.0 },
                    { "row": 0, "col": 1, "value": 24.0 },
                    { "row": 1, "col": 1, "value": 26.0 },
                    { "row": 2, "col": 1, "value": 25.0 },
                    { "row": 3, "col": 1, "value": 27.0 },
                    { "row": 0, "col": 2, "value": 40.0 },
                    { "row": 1, "col": 2, "value": 42.0 },
                    { "row": 2, "col": 2, "value": 41.0 },
                    { "row": 3, "col": 2, "value": 43.0 }
                ]
            }),
        ),
        (
            "sheet.createData",
            json!({
                "sheet_id": "col-2",
                "title": "Column Retarget",
                "archetype": "column",
                "cols": 3,
                "rows": 4
            }),
        ),
        (
            "table.setCells",
            json!({
                "sheet_id": "col-2",
                "edits": [
                    { "row": 0, "col": 0, "value": 5.0 },
                    { "row": 1, "col": 0, "value": 6.0 },
                    { "row": 2, "col": 0, "value": 7.0 },
                    { "row": 0, "col": 1, "value": 15.0 },
                    { "row": 1, "col": 1, "value": 16.0 },
                    { "row": 2, "col": 1, "value": 17.0 },
                    { "row": 0, "col": 2, "value": 25.0 },
                    { "row": 1, "col": 2, "value": 26.0 },
                    { "row": 2, "col": 2, "value": 27.0 }
                ]
            }),
        ),
        (
            "sheet.createData",
            json!({
                "sheet_id": "nested-1",
                "title": "Nested Plates",
                "archetype": "nested",
                "cols": 2,
                "rows": 3,
                "replicates": 2
            }),
        ),
        (
            "sheet.createData",
            json!({
                "sheet_id": "mv-1",
                "title": "Cohort Table",
                "archetype": "multiple_variables",
                "cols": 3,
                "rows": 3,
                "variables": [
                    { "title": "Cohort", "role": "Categorical" },
                    { "title": "Score", "role": "Continuous" },
                    { "title": "Scaled", "role": "Continuous" }
                ]
            }),
        ),
        (
            "table.setCells",
            json!({
                "sheet_id": "mv-1",
                "edits": [
                    { "row": 0, "col": 0, "value": "Low" },
                    { "row": 1, "col": 0, "value": "High" },
                    { "row": 2, "col": 0, "value": "Low" },
                    { "row": 0, "col": 1, "value": 10.0 },
                    { "row": 1, "col": 1, "value": 20.0 },
                    { "row": 2, "col": 1, "value": 30.0 }
                ]
            }),
        ),
        (
            "sheet.createData",
            json!({
                "sheet_id": "xy-dest",
                "title": "XY Scratch",
                "archetype": "xy",
                "cols": 2,
                "rows": 6
            }),
        ),
        (
            "transform.run",
            json!({
                "sheet_id": "res-xform-seed",
                "source_sheet_id": "xy-1",
                "kind": "normalize"
            }),
        ),
        (
            "stats.run",
            json!({
                "sheet_id": "res-anova-seed",
                "source_sheet_id": "col-1",
                "kind": "one_way_anova"
            }),
        ),
        (
            "fit.run",
            json!({
                "sheet_id": "res-fit-seed",
                "source_sheet_id": "xy-1",
                "kind": "linear_regression"
            }),
        ),
        (
            "graph.create",
            json!({
                "sheet_id": "gr-seed",
                "data_sheet_id": "col-1",
                "kind": "column"
            }),
        ),
        (
            "graph.create",
            json!({
                "sheet_id": "gr-uninit",
                "data_sheet_id": "xy-1",
                "initialized": false
            }),
        ),
    ];
    for (cmd, params) in setup {
        execute_checked(&mut s, cmd, params).map_err(|err| format!("fixture setup `{cmd}` failed: {err}"))?;
    }
    Ok(s)
}

#[test]
fn sweep_every_catalog_command_valid_realistic_params() -> Result<(), String> {
    let mut s = seed_rich_fixture()?;
    let mut covered = BTreeSet::new();

    // Mark setup commands that we will also explicitly invoke here for crystal clarity.
    sweep_valid_sheet_commands(&mut s, &mut covered)?;
    sweep_valid_table_commands(&mut s, &mut covered)?;
    sweep_valid_info_and_analysis_commands(&mut s, &mut covered)?;
    sweep_valid_graph_and_layout_commands(&mut s, &mut covered)?;
    sweep_valid_export_and_project_commands(&mut s, &mut covered)?;

    let registry_ids: BTreeSet<&'static str> = cmd::registry().specs().map(|s| s.id).collect();
    assert_eq!(covered, registry_ids, "valid sweep must cover every registered command");
    Ok(())
}

fn sweep_valid_sheet_commands(s: &mut Session, covered: &mut BTreeSet<&'static str>) -> Result<(), String> {
    run_valid(
        s,
        covered,
        "sheet.createData",
        json!({
            "sheet_id": "xy-extra",
            "title": "Extra XY",
            "archetype": "xy",
            "cols": 1,
            "rows": 3
        }),
    )?;
    run_valid(s, covered, "sheet.rename", json!({ "sheet_id": "xy-extra", "title": "Extra XY Renamed" }))?;
    run_valid(s, covered, "sheet.duplicate", json!({ "sheet_id": "xy-extra", "new_sheet_id": "xy-dup", "title": "Duplicated XY" }))?;
    run_valid(s, covered, "sheet.duplicateFamily", json!({ "sheet_id": "xy-1", "prefix": "fam", "clear_data": false }))?;
    run_valid(s, covered, "sheet.clonePipeline", json!({ "sheet_id": "col-1", "prefix": "pipe", "clear_data": false }))?;
    run_valid(s, covered, "sheet.freeze", json!({ "sheet_id": "xy-dup" }))?;
    run_valid(s, covered, "sheet.unfreeze", json!({ "sheet_id": "xy-dup" }))?;
    run_valid(s, covered, "sheet.reorder", json!({ "sheet_id": "xy-extra", "index": 0 }))?;
    run_valid(s, covered, "sheet.select", json!({ "sheet_id": "xy-1" }))?;
    run_valid(s, covered, "sheet.delete", json!({ "sheet_id": "xy-dup", "cascade": false }))?;
    run_valid(
        s,
        covered,
        "sheet.extractResultTab",
        json!({ "result_sheet_id": "res-fit-seed", "sheet_id": "res-tab-sweep", "archetype": "column" }),
    )?;
    Ok(())
}

fn sweep_valid_table_commands(s: &mut Session, covered: &mut BTreeSet<&'static str>) -> Result<(), String> {
    run_valid(s, covered, "table.setCell", json!({ "sheet_id": "xy-extra", "row": 0, "col": 0, "value": 42.0 }))?;
    run_valid(
        s,
        covered,
        "table.setCells",
        json!({
            "sheet_id": "xy-extra",
            "edits": [{ "row": 1, "col": 0, "value": 84.0 }]
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.setXCells",
        json!({
            "sheet_id": "xy-extra",
            "edits": [{ "row": 0, "value": 1.0 }, { "row": 1, "value": 2.0 }]
        }),
    )?;
    run_valid(s, covered, "table.setRowTitles", json!({ "sheet_id": "xy-extra", "titles": ["Row A", "Row B"] }))?;
    run_valid(s, covered, "table.setGroupTitles", json!({ "sheet_id": "xy-extra", "titles": ["Series One"] }))?;
    run_valid(s, covered, "table.setTitles", json!({ "sheet_id": "xy-extra", "x_title": "Time", "titles": ["Signal"] }))?;
    run_valid(s, covered, "table.setSubcolumnTitles", json!({ "sheet_id": "nested-1", "col": 0, "titles": ["Rep 1", "Rep 2"] }))?;
    run_valid(s, covered, "table.excludeCells", json!({ "sheet_id": "xy-extra", "cells": [{ "row": 1, "col": 0 }], "excluded": true }))?;
    run_valid(
        s,
        covered,
        "table.excludeCellsWithReason",
        json!({ "sheet_id": "xy-extra", "coords": [{ "row": 0, "col": 0 }], "reason_tag": "outlier", "note": "sweep exclusion" }),
    )?;
    run_valid(s, covered, "table.exclusionAuditReport", json!({ "sheet_id": "xy-extra" }))?;
    run_valid(
        s,
        covered,
        "table.clearCells",
        json!({
            "sheet_id": "xy-extra",
            "row_start": 0,
            "row_end": 0,
            "col_start": 0,
            "col_end": 0
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.paste",
        json!({
            "sheet_id": "xy-dest",
            "start_row": 0,
            "start_col": 0,
            "rows": [[1.0, 2.0], [3.0, 4.0]]
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.pasteLink",
        json!({
            "target_sheet_id": "xy-dest",
            "source_sheet_id": "xy-1",
            "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 1, "col": 0 } },
            "start_row": 2,
            "start_col": 0
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.importCsv",
        json!({
            "sheet_id": "csv-1",
            "title": "CSV Import",
            "archetype": "column",
            "csv": "Ctrl,Drug\n10,20\n12,24\n"
        }),
    )?;
    run_valid(s, covered, "table.exportCsv", json!({ "sheet_id": "xy-1" }))?;
    run_valid(s, covered, "table.setVariableRole", json!({ "sheet_id": "mv-1", "col": 0, "role": "Categorical" }))?;
    run_valid(s, covered, "table.recodeLevels", json!({ "sheet_id": "mv-1", "col": 0, "mappings": { "Low": "Control" } }))?;
    run_valid(s, covered, "table.setCategoricalOrder", json!({ "sheet_id": "mv-1", "col": 0, "order": ["Control", "High"] }))?;
    run_valid(
        s,
        covered,
        "table.setCalculatedVariable",
        json!({
            "sheet_id": "mv-1",
            "col": 2,
            "title": "Scaled",
            "formula": "= [Score] * 2"
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.simulateXy",
        json!({
            "sheet_id": "sim-1",
            "row_count": 6,
            "replicates": 1,
            "x_min": 0.0,
            "x_max": 5.0,
            "equation_id": "linear",
            "parameters": { "slope": 2.0, "intercept": 1.0 },
            "seed": 7
        }),
    )?;
    run_valid(s, covered, "table.convertArchetype", json!({ "sheet_id": "csv-1", "target_kind": "Grouped", "allow_lossy": true }))?;
    let plate_values = vec![10.0_f64; 96];
    let mut plate_roles = Vec::with_capacity(96);
    for idx in 0..96_usize {
        if idx < 2 {
            plate_roles.push(json!("Blank"));
        } else if idx < 6 {
            let offset = idx - 2;
            plate_roles.push(json!({
                "role": "treatment",
                "group_name": "Dose",
                "dilution_x": (offset / 2 + 1) as f64,
                "replicate": (offset % 2) + 1
            }));
        } else {
            plate_roles.push(json!("Excluded"));
        }
    }
    run_valid(
        s,
        covered,
        "table.importPlateMap",
        json!({
            "sheet_id": "plate-1",
            "title": "Plate 96",
            "geometry": "Wells96",
            "values": plate_values,
            "roles": plate_roles,
            "subtract_blank_mean": true
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.importTidyLong",
        json!({
            "sheet_id": "tidy-1",
            "title": "Tidy Long",
            "csv": "Group,Value\nCtrl,10\nCtrl,12\nDrug,20\nDrug,22\n",
            "mapping": { "group_col": "Group", "value_col": "Value" }
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.randomizeGroups",
        json!({
            "sheet_id": "rand-1",
            "groups": [{ "label": "Vehicle", "weight": 1 }, { "label": "Drug", "weight": 1 }],
            "subject_count": 8,
            "scheme": "permuted_block",
            "block_sizes": [4],
            "seed": 17
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.randomSampleRows",
        json!({
            "sheet_id": "sample-1",
            "source_sheet_id": "tidy-1",
            "sample_size": 2,
            "seed": 19
        }),
    )?;
    run_valid(
        s,
        covered,
        "table.simulateDistributions",
        json!({
            "sheet_id": "dist-1",
            "n_rows": 8,
            "seed": 23,
            "columns": [
                { "name": "Unif", "distribution": "uniform", "min": 0.0, "max": 1.0 },
                { "name": "Norm", "distribution": "gaussian", "mean": 5.0, "sd": 1.5 }
            ]
        }),
    )?;
    Ok(())
}

fn sweep_valid_info_and_analysis_commands(s: &mut Session, covered: &mut BTreeSet<&'static str>) -> Result<(), String> {
    let info_id = s.document().info_sheets.first().ok_or_else(|| String::from("missing info sheet"))?.id.as_str().to_string();
    run_valid(s, covered, "info.setNote", json!({ "sheet_id": info_id, "text": "Baseline experimental log" }))?;
    run_valid(s, covered, "info.setConstant", json!({ "sheet_id": info_id, "name": "TempC", "value": 37.0 }))?;
    run_valid(s, covered, "info.removeConstant", json!({ "sheet_id": info_id, "name": "TempC" }))?;

    run_valid(s, covered, "analyze.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "col-1", "kind": "descriptive_stats" }))?;
    run_valid(s, covered, "transform.run", json!({ "sheet_id": "res-xform", "source_sheet_id": "xy-1", "kind": "normalize" }))?;
    run_valid(s, covered, "stats.run", json!({ "sheet_id": "res-anova", "source_sheet_id": "col-1", "kind": "one_way_anova" }))?;
    execute_checked(s, "sheet.createData", json!({ "sheet_id": "pval-1", "title": "P Stack", "archetype": "column", "cols": 1, "rows": 4 }))
        .map_err(|e| e.to_string())?;
    execute_checked(
        s,
        "table.setCells",
        json!({
            "sheet_id": "pval-1",
            "edits": [
                { "row": 0, "col": 0, "value": 0.001 },
                { "row": 1, "col": 0, "value": 0.02 },
                { "row": 2, "col": 0, "value": 0.04 },
                { "row": 3, "col": 0, "value": 0.50 }
            ]
        }),
    )
    .map_err(|e| e.to_string())?;
    run_valid(
        s,
        covered,
        "stats.pvalueStack",
        json!({ "sheet_id": "res-pstack", "source_sheet_id": "pval-1", "method": "benjamini_hochberg", "threshold": 0.05 }),
    )?;
    run_valid(s, covered, "fit.run", json!({ "sheet_id": "res-fit", "source_sheet_id": "xy-1", "kind": "linear_regression" }))?;
    run_valid(s, covered, "analysis.add", json!({ "sheet_id": "res-ba", "source_sheet_id": "col-1", "kind": "stats.bland_altman" }))?;
    run_valid(s, covered, "analysis.run", json!({ "sheet_id": "res-col", "source_sheet_id": "col-1", "kind": "descriptive_stats" }))?;
    run_valid(s, covered, "analysis.updateParams", json!({ "sheet_id": "res-anova", "confidence_percent": 99.0 }))?;
    run_valid(s, covered, "analysis.retargetData", json!({ "sheet_id": "res-anova", "source_sheet_id": "col-2" }))?;
    run_valid(s, covered, "analysis.exportJson", json!({ "sheet_id": "res-anova" }))?;
    run_valid(s, covered, "analysis.exportCsv", json!({ "sheet_id": "res-anova" }))?;
    run_valid(s, covered, "result.generateReport", json!({ "result_sheet_id": "res-anova", "format": "markdown" }))?;
    Ok(())
}

fn sweep_valid_graph_and_layout_commands(s: &mut Session, covered: &mut BTreeSet<&'static str>) -> Result<(), String> {
    run_valid(s, covered, "graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "xy-1", "kind": "xy" }))?;
    run_valid(s, covered, "graph.create", json!({ "sheet_id": "gr-col", "data_sheet_id": "col-2", "initialized": false }))?;
    run_valid(
        s,
        covered,
        "graph.initialize",
        json!({
            "sheet_id": "gr-col",
            "kind": "column",
            "title": "Column Summary"
        }),
    )?;
    run_valid(s, covered, "graph.setKind", json!({ "sheet_id": "gr-col", "kind": "column" }))?;
    run_valid(s, covered, "graph.setSpec", json!({ "sheet_id": "gr-col", "title": "Updated Column Title" }))?;
    run_valid(s, covered, "graph.setAxes", json!({ "sheet_id": "gr-1", "x_axis": { "scale": "log10" } }))?;
    run_valid(
        s,
        covered,
        "graph.setAdditionalTicks",
        json!({
            "sheet_id": "gr-1",
            "axis": "x",
            "ticks": [{ "value": 10.0, "label": "EC50", "draw_gridline": true }]
        }),
    )?;
    run_valid(s, covered, "graph.setSeries", json!({ "sheet_id": "gr-1", "series_index": 0, "color": "#1F77B4CC" }))?;
    run_valid(s, covered, "graph.setRegressionOverlay", json!({ "sheet_id": "gr-1", "result_sheet_id": "res-fit", "series_index": 0 }))?;
    run_valid(s, covered, "graph.addComparisonBrackets", json!({ "sheet_id": "gr-col", "result_sheet_id": "res-anova", "significant_only": false }))?;
    run_valid(
        s,
        covered,
        "graph.setCompactLetterDisplay",
        json!({
            "sheet_id": "gr-col",
            "result_sheet_id": "res-anova",
            "pairwise_display_mode": "brackets_and_compact_letter_display",
            "ordering": "descending_mean",
            "uppercase": false
        }),
    )?;
    run_valid(s, covered, "graph.addAnnotation", json!({ "sheet_id": "gr-1", "kind": "text", "text": "Peak", "x_pt": 180.0, "y_pt": 60.0 }))?;
    run_valid(s, covered, "graph.applyStyleFrom", json!({ "sheet_id": "gr-col", "donor_sheet_id": "gr-1" }))?;
    run_valid(
        s,
        covered,
        "graph.applyCanvasPreset",
        json!({ "sheet_id": "gr-col", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }),
    )?;
    run_valid(s, covered, "graph.preflightCheck", json!({ "sheet_id": "gr-col" }))?;
    run_valid(
        s,
        covered,
        "graph.formatSelectedPoints",
        json!({
            "sheet_id": "gr-1",
            "points": [{ "series_index": 0, "point_index": 0 }],
            "color": "#D55E00",
            "symbol": "diamond",
            "size_pt": 7.0
        }),
    )?;

    run_valid(
        s,
        covered,
        "layout.create",
        json!({
            "sheet_id": "lay-1",
            "title": "Figure 1",
            "preset": "double_column",
            "grid_template": [1, 2],
            "panel_badges": "uppercase"
        }),
    )?;
    run_valid(s, covered, "layout.setSpec", json!({ "sheet_id": "lay-1", "orientation": "landscape" }))?;
    run_valid(
        s,
        covered,
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "gr-1", "bounds_pt": [20.0, 20.0, 220.0, 140.0] }),
    )?;
    run_valid(
        s,
        covered,
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "gr-col", "bounds_pt": [260.0, 20.0, 220.0, 140.0] }),
    )?;
    run_valid(s, covered, "layout.alignOrigins", json!({ "sheet_id": "lay-1", "align": true }))?;
    run_valid(
        s,
        covered,
        "layout.applyCanvasPreset",
        json!({ "sheet_id": "lay-1", "preset": "DoubleColumn180mm", "resize_mode": "KeepTypographyPt" }),
    )?;
    run_valid(
        s,
        covered,
        "edit.pasteResultTable",
        json!({ "source_sheet_id": "res-fit", "target_sheet_id": "gr-1", "bounds_pt": [40.0, 40.0, 140.0, 60.0] }),
    )?;
    run_valid(s, covered, "layout.equalizeScaling", json!({ "sheet_id": "lay-1", "mode": "match_smallest" }))?;
    run_valid(s, covered, "layout.reorderPanel", json!({ "sheet_id": "lay-1", "panel_index": 0, "action": "bring_to_front" }))?;

    const SAMPLE_PNG_2X2_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFUlEQVR4nGP4z8DwHwyBdMP///8bAEbOCPny53p1AAAAAElFTkSuQmCC";
    let g_img = run_valid(
        s,
        covered,
        "graph.insertImage",
        json!({
            "sheet_id": "gr-1",
            "bytes_base64": SAMPLE_PNG_2X2_B64,
            "bounds_pt": [12.0, 12.0, 48.0, 48.0],
            "rotation_deg": 0.0,
            "lock_aspect_ratio": true,
            "border": { "width_pt": 1.0, "color": "#222222", "dash": "solid" }
        }),
    )?;
    let g_ann_idx = g_img.payload.get("annotation_index").and_then(Value::as_u64).unwrap_or(0);
    let g_asset_id = g_img.payload.get("asset_id").and_then(Value::as_str).unwrap_or_default().to_string();
    run_valid(
        s,
        covered,
        "graph.updateImage",
        json!({
            "sheet_id": "gr-1",
            "annotation_index": g_ann_idx,
            "width_pt": 64.0,
            "rotation_deg": 15.0,
            "crop": { "left_pt": 2.0, "right_pt": 2.0 },
            "border": null
        }),
    )?;
    run_valid(
        s,
        covered,
        "layout.insertImage",
        json!({
            "sheet_id": "lay-1",
            "panel_id": "img-sweep-1",
            "asset_id": g_asset_id,
            "bounds_pt": [10.0, 170.0, 40.0, 40.0],
            "badge_override": "C"
        }),
    )?;
    run_valid(
        s,
        covered,
        "layout.updateImage",
        json!({
            "sheet_id": "lay-1",
            "panel_id": "img-sweep-1",
            "height_pt": 48.0,
            "rotation_deg": -10.0,
            "crop": { "top_pt": 1.0, "bottom_pt": 1.0 },
            "border": { "width_pt": 0.75, "color": "#000000", "dash": "dashed" }
        }),
    )?;
    run_valid(
        s,
        covered,
        "edit.pasteImage",
        json!({
            "target_sheet_id": "lay-1",
            "panel_id": "img-sweep-pasted",
            "bytes_base64": SAMPLE_PNG_2X2_B64,
            "bounds_pt": [60.0, 170.0, 32.0, 32.0]
        }),
    )?;
    Ok(())
}

fn sweep_valid_export_and_project_commands(s: &mut Session, covered: &mut BTreeSet<&'static str>) -> Result<(), String> {
    let svg_path = next_temp_file("graph", "svg");
    let pdf_path = next_temp_file("graph", "pdf");
    let png_path = next_temp_file("graph", "png");
    let tiff_path = next_temp_file("graph", "tiff");
    let g_svg_path = next_temp_file("graph2", "svg");
    let g_pdf_path = next_temp_file("graph2", "pdf");
    let g_png_path = next_temp_file("graph2", "png");
    let g_emf_path = next_temp_file("graph2", "emf");
    let lay_emf_path = next_temp_file("layout1", "emf");

    run_valid(s, covered, "export.svg", json!({ "sheet_id": "gr-1", "path": svg_path }))?;
    run_valid(s, covered, "export.pdf", json!({ "sheet_id": "gr-1", "path": pdf_path }))?;
    run_valid(s, covered, "export.png", json!({ "sheet_id": "gr-1", "path": png_path, "width_px": 64, "height_px": 48, "dpi": 72 }))?;
    run_valid(s, covered, "export.tiff", json!({ "sheet_id": "gr-1", "path": tiff_path, "width_px": 64, "height_px": 48, "dpi": 72 }))?;
    run_valid(s, covered, "graph.exportSvg", json!({ "sheet_id": "gr-col", "path": g_svg_path }))?;
    run_valid(s, covered, "graph.exportPdf", json!({ "sheet_id": "gr-col", "path": g_pdf_path }))?;
    run_valid(s, covered, "graph.exportPng", json!({ "sheet_id": "gr-col", "path": g_png_path, "width_px": 64, "height_px": 48, "dpi": 72 }))?;
    run_valid(s, covered, "graph.exportEmf", json!({ "sheet_id": "gr-col", "path": g_emf_path }))?;
    run_valid(s, covered, "layout.exportEmf", json!({ "sheet_id": "lay-1", "path": lay_emf_path }))?;

    for p in [&svg_path, &pdf_path, &png_path, &tiff_path, &g_svg_path, &g_pdf_path, &g_png_path, &g_emf_path, &lay_emf_path] {
        let _ = fs::remove_file(p);
    }

    run_valid(s, covered, "project.inspect", json!({}))?;
    run_valid(s, covered, "project.recalculate", json!({}))?;
    run_valid(s, covered, "project.createCheckpoint", json!({ "name": "cp-sweep", "note": "sweep baseline" }))?;
    run_valid(s, covered, "project.listCheckpoints", json!({}))?;
    run_valid(s, covered, "project.restoreCheckpoint", json!({ "name": "cp-sweep" }))?;
    run_valid(s, covered, "project.deleteCheckpoint", json!({ "name": "cp-sweep" }))?;
    run_valid(s, covered, "edit.undo", json!({}))?;
    run_valid(s, covered, "edit.redo", json!({}))?;

    let proj_path = next_temp_file("saved", "cbproj");
    let saved = run_valid(s, covered, "project.save", json!({ "path": proj_path, "return_bytes": true }))?;
    let saved_b64 =
        saved.payload.get("bytes_base64").and_then(Value::as_str).ok_or_else(|| String::from("missing project bytes_base64"))?.to_string();
    let tpl_saved = run_valid(s, covered, "project.saveTemplate", json!({ "return_bytes": true }))?;
    let tpl_b64 =
        tpl_saved.payload.get("bytes_base64").and_then(Value::as_str).ok_or_else(|| String::from("missing template bytes_base64"))?.to_string();

    run_valid(s, covered, "table.importPrism", json!({ "bytes_base64": saved_b64 }))?;
    run_valid(
        s,
        covered,
        "table.importPzfx",
        json!({
            "xml": "<PrismFile><Table ID=\"T1\" TableType=\"XY\"><Title>Imported XY</Title><XColumn><Title>X</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn><YColumn><Title>Y</Title><Subcolumn><d>10</d><d>20</d></Subcolumn></YColumn></Table></PrismFile>"
        }),
    )?;
    let mut xlsx_zip = curvebook_format::ZipWriter::new();
    xlsx_zip
        .add_file(
            "xl/workbook.xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_vec(),
        )
        .map_err(|e| e.to_string())?;
    xlsx_zip
        .add_file(
            "xl/_rels/workbook.xml.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_vec(),
        )
        .map_err(|e| e.to_string())?;
    xlsx_zip
        .add_file(
            "xl/worksheets/sheet1.xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>X</t></is></c><c r="B1" t="inlineStr"><is><t>Y</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>2</v></c></row></sheetData></worksheet>"#.to_vec(),
        )
        .map_err(|e| e.to_string())?;
    let xlsx_b64 = curvebook_engine::cmd::helpers::encode_base64(&xlsx_zip.finish().map_err(|e| e.to_string())?);
    run_valid(s, covered, "table.importXlsx", json!({ "bytes_base64": xlsx_b64 }))?;
    run_valid(s, covered, "file.new", json!({ "title": "Reset File" }))?;
    run_valid(s, covered, "project.open", json!({ "bytes_base64": saved_b64 }))?;
    run_valid(s, covered, "project.new", json!({ "title": "Reset Project" }))?;
    run_valid(s, covered, "project.instantiateTemplate", json!({ "bytes_base64": tpl_b64, "title": "Template Instance" }))?;

    let _ = fs::remove_file(proj_path);
    Ok(())
}

#[test]
fn sweep_every_catalog_command_hostile_and_corrupt_inputs() -> Result<(), String> {
    let base = seed_rich_fixture()?;
    let mut covered = BTreeSet::new();

    let universal_hostile: [Value; 9] = [
        Value::Null,
        json!(false),
        json!(12345),
        json!("not-an-object"),
        json!([]),
        json!({}),
        json!({
            "sheet_id": 999,
            "source_sheet_id": ["bad"],
            "target_sheet_id": null,
            "row": u64::MAX,
            "col": u64::MAX,
            "subcol": u64::MAX,
            "index": u64::MAX,
            "rows": u64::MAX,
            "cols": u64::MAX,
            "value": "NaN",
            "confidence_percent": "Infinity",
            "kind": 42
        }),
        json!({
            "sheet_id": "nonexistent-sheet-id",
            "source_sheet_id": "nonexistent-sheet-id",
            "target_sheet_id": "nonexistent-sheet-id",
            "result_sheet_id": "nonexistent-result",
            "graph_sheet_id": "nonexistent-graph",
            "bytes_base64": "====invalid-b64====",
            "xml": "<unclosed><tag>",
            "csv": "",
            "path": "/nonexistent_dir_cb_sweep/should_fail.out",
            "width_px": 0,
            "height_px": 0,
            "dpi": 0
        }),
        json!({
            "sheet_id": "xy-1",
            "row": "zero",
            "col": -1,
            "cells": "not-an-array",
            "x": "NaN",
            "y": "Infinity",
            "bounds_pt": ["bad", "array"]
        }),
    ];

    for spec in cmd::registry().specs() {
        covered.insert(spec.id);
        for hostile in &universal_hostile {
            let mut s = base.clone();
            let _ = execute_checked(&mut s, spec.id, hostile.clone());
        }
        run_targeted_hostile_cases(&base, spec.id);
    }

    verify_cyclic_and_frozen_protections()?;

    let registry_ids: BTreeSet<&'static str> = cmd::registry().specs().map(|s| s.id).collect();
    assert_eq!(covered, registry_ids, "hostile sweep must cover every registered command");
    Ok(())
}

fn run_targeted_hostile_cases(base: &Session, command_id: &str) {
    let bad_inputs: Vec<Value> = match command_id {
        "sheet.createData" => vec![
            json!({ "sheet_id": "xy-1", "archetype": "xy" }),
            json!({ "sheet_id": "bad-dim", "archetype": "xy", "rows": u64::MAX }),
            json!({ "sheet_id": "bad-cols", "archetype": "xy", "cols": 1025 }),
            json!({ "sheet_id": "bad-rep", "archetype": "nested", "replicates": 513 }),
            json!({ "sheet_id": "bad-arch", "archetype": "unknown_table_type" }),
        ],
        "table.setCell" | "table.setCells" => vec![
            json!({ "sheet_id": "xy-1", "row": u64::MAX, "col": 0, "value": 1.0 }),
            json!({ "sheet_id": "xy-1", "row": 0, "col": u64::MAX, "value": "NaN" }),
            json!({ "sheet_id": "xy-1", "edits": [{ "row": u64::MAX, "col": 0, "value": "Infinity" }] }),
        ],
        "table.excludeCells" | "table.clearCells" => vec![
            json!({ "sheet_id": "xy-1", "range": { "start": { "row": 0, "col": 0 }, "end": { "row": u64::MAX, "col": 0 } } }),
            json!({ "sheet_id": "xy-1", "row_start": 5, "row_end": 1, "col_start": 0, "col_end": 0 }),
        ],
        "table.pasteLink" => vec![
            json!({ "target_sheet_id": "xy-1", "source_sheet_id": "xy-1" }),
            json!({ "target_sheet_id": "xy-dest", "source_sheet_id": "nonexistent" }),
        ],
        "table.setCalculatedVariable" => vec![
            json!({ "sheet_id": "xy-1", "col": 0, "formula": "= [X] + 1" }),
            json!({ "sheet_id": "mv-1", "col": 0, "formula": "= [MissingVar] * 2" }),
            json!({ "sheet_id": "mv-1", "col": 2, "title": "Scaled", "formula": "= [Scaled] + 1" }),
        ],
        "table.simulateXy" => vec![
            json!({ "sheet_id": "sim-bad", "row_count": u64::MAX }),
            json!({ "sheet_id": "sim-bad", "row_count": 0 }),
            json!({ "sheet_id": "sim-bad", "x_min": "NaN", "x_max": 1.0 }),
            json!({ "sheet_id": "sim-bad", "equation": "nonexistent_eq" }),
        ],
        "table.importPzfx" | "table.importPrism" | "table.importXlsx" => {
            vec![json!({ "xml": "<PrismFile><Broken" }), json!({ "bytes_base64": "AA=A" }), json!({ "bytes": [0, 1, 2, 3] })]
        }
        "transform.run" | "stats.run" | "fit.run" | "analyze.run" | "analysis.run" => vec![
            json!({ "source_sheet_id": "missing", "kind": "normalize" }),
            json!({ "source_sheet_id": "xy-1", "kind": "unknown_analysis_kind" }),
            json!({ "source_sheet_id": "col-1", "kind": "one_way_anova", "confidence_percent": 150.0 }),
        ],
        "stats.pvalueStack" => vec![
            json!({}),
            json!({ "source_sheet_id": "missing" }),
            json!({ "source_sheet_id": "col-1", "threshold": 0.05 }),
            json!({ "source_sheet_id": "col-1", "method": "not_a_method", "threshold": 0.05 }),
            json!({ "source_sheet_id": "col-1", "method": "bonferroni", "threshold": 1.5 }),
            json!({ "source_sheet_id": "col-1", "method": "bonferroni", "threshold": 0.0 }),
        ],
        "graph.setAxes" => vec![
            json!({ "sheet_id": "gr-seed", "x_axis": { "min": 100.0, "max": 10.0 } }),
            json!({ "sheet_id": "gr-seed", "axis": "y", "segments": [{}, {}, {}, {}] }),
        ],
        "graph.setSeries" => vec![
            json!({ "sheet_id": "gr-seed", "series_index": 0, "color": "not-a-hex-color" }),
            json!({ "sheet_id": "gr-seed", "series_index": u64::MAX, "color": "#FF0000" }),
        ],
        "graph.initialize" | "graph.addComparisonBrackets" | "graph.applyStyleFrom" => vec![
            json!({ "sheet_id": "gr-seed", "donor_sheet_id": "missing-graph", "result_sheet_id": "missing-res" }),
            json!({ "sheet_id": "missing-graph", "kind": "bad_kind" }),
        ],
        "graph.setCompactLetterDisplay" => vec![
            json!({ "sheet_id": "gr-seed", "enabled": true }),
            json!({ "sheet_id": "gr-seed", "result_sheet_id": "res-anova-seed", "pairwise_display_mode": "invalid_mode" }),
            json!({ "sheet_id": "gr-seed", "result_sheet_id": "res-anova-seed", "alpha_override": 1.5 }),
            json!({ "sheet_id": "gr-seed", "result_sheet_id": "res-anova-seed", "font_size_pt": 0.0 }),
        ],
        "layout.create" | "layout.addPanel" | "layout.alignOrigins" | "layout.equalizeScaling" | "layout.reorderPanel" => vec![
            json!({ "sheet_id": "", "preset": "nonexistent" }),
            json!({ "sheet_id": "missing-layout", "kind": "graph", "graph_sheet_id": "gr-seed", "bounds_pt": ["bad"] }),
            json!({ "sheet_id": "lay-seed", "mode": "explicit_percent", "percent": 0.5, "panel_index": 999, "action": "invalid_action" }),
        ],
        "edit.pasteResultTable" => vec![
            json!({}),
            json!({ "source_sheet_id": "missing-res", "target_sheet_id": "gr-seed" }),
            json!({ "source_sheet_id": "res-anova-seed", "target_sheet_id": "col-1" }),
            json!({ "source_sheet_id": "res-anova-seed", "target_sheet_id": "gr-seed", "bounds_pt": [10.0, 10.0, -5.0, 20.0] }),
        ],
        "graph.insertImage" | "layout.insertImage" | "edit.pasteImage" => vec![
            json!({}),
            json!({ "sheet_id": "gr-seed", "target_sheet_id": "gr-seed" }),
            json!({ "sheet_id": "gr-seed", "target_sheet_id": "gr-seed", "path": "a.png", "bytes_base64": "aGVsbG8=" }),
            json!({ "sheet_id": "gr-seed", "target_sheet_id": "gr-seed", "path": "https://example.com/a.png" }),
            json!({ "sheet_id": "gr-seed", "target_sheet_id": "gr-seed", "bytes_base64": "PHN2Zz48L3N2Zz4=" }),
            json!({ "sheet_id": "gr-seed", "target_sheet_id": "gr-seed", "bytes_base64": "====" }),
        ],
        "graph.updateImage" | "layout.updateImage" => vec![
            json!({}),
            json!({ "sheet_id": "missing-sheet", "annotation_index": 0, "panel_id": "missing-panel" }),
            json!({ "sheet_id": "gr-seed", "annotation_index": 999, "width_pt": 20.0 }),
            json!({ "sheet_id": "lay-seed", "item_index": 999, "width_pt": -10.0 }),
            json!({ "sheet_id": "lay-seed", "panel_id": "missing", "crop": { "left_pt": 10000.0 } }),
        ],
        "export.png" | "export.tiff" | "graph.exportPng" => vec![
            json!({ "sheet_id": "gr-seed", "width_px": u32::MAX, "height_px": u32::MAX }),
            json!({ "sheet_id": "gr-seed", "width_px": 16_384, "height_px": 16_384 }),
            json!({ "sheet_id": "gr-seed", "dpi": 0 }),
            json!({ "sheet_id": "nonexistent", "compression": "bad_codec" }),
        ],
        "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => vec![
            json!({ "sheet_id": "gr-seed", "preset": "nonexistent_preset" }),
            json!({ "sheet_id": "gr-seed", "preset": "SingleColumn88mm", "resize_mode": "unknown_mode" }),
            json!({ "sheet_id": "missing-sheet", "preset": "SingleColumn88mm" }),
        ],
        "graph.preflightCheck" | "graph.exportEmf" | "layout.exportEmf" | "result.generateReport" => {
            vec![json!({ "sheet_id": "missing-sheet", "format": "bad_fmt" }), json!({ "sheet_id": "xy-1" })]
        }
        "project.open" | "project.instantiateTemplate" => vec![
            json!({ "bytes_base64": "====" }),
            json!({ "bytes_base64": "aGVsbG8=" }),
            json!({ "path": next_temp_file("does_not_exist", "cbproj") }),
        ],
        _ => Vec::new(),
    };

    for params in bad_inputs {
        let mut s = base.clone();
        let res = execute_checked(&mut s, command_id, params);
        // Almost all of these are strictly erroneous; ensure any error rolled back cleanly.
        if let Ok(outcome) = res {
            assert_eq!(outcome.command_id, command_id);
        }
    }
}

fn verify_cyclic_and_frozen_protections() -> Result<(), String> {
    let mut s = seed_rich_fixture()?;
    execute_checked(&mut s, "sheet.createData", json!({ "sheet_id": "link-a", "archetype": "xy", "cols": 1, "rows": 4 }))
        .map_err(|e| e.to_string())?;
    execute_checked(&mut s, "sheet.createData", json!({ "sheet_id": "link-b", "archetype": "xy", "cols": 1, "rows": 4 }))
        .map_err(|e| e.to_string())?;
    execute_checked(&mut s, "sheet.createData", json!({ "sheet_id": "link-c", "archetype": "xy", "cols": 1, "rows": 4 }))
        .map_err(|e| e.to_string())?;
    execute_checked(
        &mut s,
        "table.pasteLink",
        json!({
            "target_sheet_id": "link-b",
            "source_sheet_id": "link-a",
            "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 1, "col": 0 } },
            "start_row": 0,
            "start_col": 0
        }),
    )
    .map_err(|e| e.to_string())?;
    execute_checked(
        &mut s,
        "table.pasteLink",
        json!({
            "target_sheet_id": "link-c",
            "source_sheet_id": "link-b",
            "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 1, "col": 0 } },
            "start_row": 0,
            "start_col": 0
        }),
    )
    .map_err(|e| e.to_string())?;

    // Closing 2-cycle (link-b -> link-a) or 3-cycle (link-c -> link-a) must fail atomically.
    for src in ["link-b", "link-c"] {
        let res = execute_checked(
            &mut s,
            "table.pasteLink",
            json!({
                "target_sheet_id": "link-a",
                "source_sheet_id": src,
                "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 1, "col": 0 } },
                "start_row": 0,
                "start_col": 0
            }),
        );
        assert!(res.is_err(), "closing cycle link-a <- {src} must fail");
    }

    // Freeze link-b; mutations and deleting upstream link-a must fail atomically.
    execute_checked(&mut s, "sheet.freeze", json!({ "sheet_id": "link-b" })).map_err(|e| e.to_string())?;
    let frozen_cmds: [(&str, Value); 6] = [
        ("sheet.rename", json!({ "sheet_id": "link-b", "title": "Hacked" })),
        ("sheet.delete", json!({ "sheet_id": "link-b" })),
        ("sheet.delete", json!({ "sheet_id": "link-a", "cascade": false })),
        ("table.setCell", json!({ "sheet_id": "link-b", "row": 0, "col": 0, "value": 9.0 })),
        ("table.excludeCells", json!({ "sheet_id": "link-b", "cells": [{ "row": 0, "col": 0 }], "excluded": true })),
        (
            "table.pasteLink",
            json!({
                "target_sheet_id": "link-b",
                "source_sheet_id": "link-a",
                "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 0, "col": 0 } },
                "start_row": 1,
                "start_col": 0
            }),
        ),
    ];
    for (cmd, params) in frozen_cmds {
        assert!(execute_checked(&mut s, cmd, params).is_err(), "{cmd} should fail on frozen state");
    }

    // Cyclic calculated variable: A = [B] + 1, B = [A] + 1.
    execute_checked(&mut s, "table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "col": 2, "title": "Scaled", "formula": "= [Score] * 2" }))
        .map_err(|e| e.to_string())?;
    assert!(
        execute_checked(
            &mut s,
            "table.setCalculatedVariable",
            json!({ "sheet_id": "mv-1", "col": 1, "title": "Score", "formula": "= [Scaled] + 1" }),
        )
        .is_err()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Proptest 3: Arbitrary corrupt JSON fuzzing across all 73 catalog commands
// ---------------------------------------------------------------------------

fn corrupt_string_strategy() -> BoxedStrategy<String> {
    proptest::sample::select(vec![
        String::new(),
        String::from("NaN"),
        String::from("Infinity"),
        String::from("-Infinity"),
        String::from("xy-1"),
        String::from("col-1"),
        String::from("mv-1"),
        String::from("res-anova-seed"),
        String::from("nonexistent"),
        String::from("===="),
        String::from("<unclosed"),
        String::from("= [Missing] + 1"),
        String::from("18446744073709551615"),
    ])
    .boxed()
}

fn corrupt_value_strategy() -> BoxedStrategy<Value> {
    let leaf = proptest::prop_oneof![
        Just(Value::Null),
        proptest::arbitrary::any::<bool>().prop_map(Value::Bool),
        proptest::sample::select(vec![json!(0), json!(-1), json!(1), json!(1000001), json!(u64::MAX), json!(1e308), json!(-1e308),]),
        corrupt_string_strategy().prop_map(Value::String),
    ];

    leaf.prop_recursive(3, 16, 4, |inner| {
        let keys = proptest::sample::select(vec![
            "sheet_id",
            "source_sheet_id",
            "target_sheet_id",
            "result_sheet_id",
            "graph_sheet_id",
            "row",
            "col",
            "subcol",
            "rows",
            "cols",
            "replicates",
            "index",
            "value",
            "cells",
            "kind",
            "title",
            "formula",
            "confidence_percent",
            "bounds_pt",
            "bytes_base64",
            "xml",
            "csv",
            "x",
            "y",
            "width_px",
            "height_px",
        ]);
        proptest::prop_oneof![
            vec(inner.clone(), 0..4).prop_map(Value::Array),
            vec((keys, inner), 0..5).prop_map(|pairs| {
                let mut map = serde_json::Map::new();
                for (k, v) in pairs {
                    map.insert(k.to_string(), v);
                }
                Value::Object(map)
            }),
        ]
    })
    .boxed()
}

fn command_index_strategy() -> BoxedStrategy<usize> {
    (0..cmd::registry().len()).boxed()
}

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]

    #[test]
    fn proptest_arbitrary_corrupt_json_across_all_commands(
        steps in vec((command_index_strategy(), corrupt_value_strategy()), 1..20)
    ) {
        let mut session = seed_rich_fixture().map_err(TestCaseError::fail)?;
        let specs: Vec<_> = cmd::registry().specs().collect();
        // Keep path-bearing commands safely inside temp or rejected rather than writing workspace files.
        for (cmd_idx, mut params) in steps {
            let Some(spec) = specs.get(cmd_idx) else {
                return Err(TestCaseError::fail(format!("command index out of bounds: {cmd_idx}")));
            };
            let cmd = spec.id;
            let mut created_temp: Option<PathBuf> = None;
            if let Some(obj) = params.as_object_mut() {
                if obj.contains_key("path") || cmd.starts_with("export.") || cmd.contains("export") || cmd == "project.save" {
                    let safe = next_temp_file("fuzz", "tmp");
                    obj.insert(String::from("path"), json!(&safe));
                    created_temp = Some(safe);
                }
                if let Some(w) = obj.get("width_px").and_then(Value::as_u64)
                    && w > 128
                {
                    obj.insert(String::from("width_px"), json!(u32::MAX));
                }
                if let Some(h) = obj.get("height_px").and_then(Value::as_u64)
                    && h > 128
                {
                    obj.insert(String::from("height_px"), json!(u32::MAX));
                }
            }

            let before = session.clone();
            let res = session.execute(cmd, params);
            if let Some(path) = created_temp {
                let _ = fs::remove_file(path);
            }
            if let Err(ref err) = res {
                prop_assert!(
                    !err.message().contains("recovered from internal panic"),
                    "fuzzed `{cmd}` panicked internally: {}",
                    err.message()
                );
                prop_assert_eq!(session.document(), before.document());
                prop_assert_eq!(&session, &before);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Proptest 4: Interleaved multi-step sequences + transactional rollback + undo/redo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum SweepStep {
    CreateData { arch_idx: usize, cols: u8, rows: u8, id_tag: u16, corrupt: bool },
    SetCells { target_idx: usize, row: u8, col: u8, val_kind: u8 },
    ExcludeCells { target_idx: usize, row: u8, col: u8, excluded: bool, huge: bool },
    PasteLink { dst_idx: usize, src_idx: usize, cyclic_or_missing: u8 },
    SetCalculatedVariable { target_idx: usize, formula_kind: u8 },
    TransformRun { src_idx: usize, kind_idx: usize },
    StatsRun { src_idx: usize, kind_idx: usize },
    FitRun { src_idx: usize, kind_idx: usize },
    GraphInitialize { src_idx: usize, kind_idx: usize },
    GraphSetAxes { graph_idx: usize, hostile: bool },
    GraphAddComparisonBrackets { graph_idx: usize, res_idx: usize },
    GraphApplyStyleFrom { dst_idx: usize, src_idx: usize },
    LayoutCreate { hostile: bool },
    LayoutAddPanel { lay_idx: usize, graph_idx: usize, slot_idx: u8, hostile: bool },
    ClonePipeline { src_idx: usize },
    Freeze { target_idx: usize },
    Unfreeze { target_idx: usize },
    DeleteSheet { target_idx: usize, cascade: bool },
    Undo,
    Redo,
}

fn sweep_step_strategy() -> BoxedStrategy<SweepStep> {
    proptest::prop_oneof![
        3 => (0..5usize, 1..4u8, 2..6u8, 1..5000u16, proptest::bool::weighted(0.25))
            .prop_map(|(arch_idx, cols, rows, id_tag, corrupt)| SweepStep::CreateData { arch_idx, cols, rows, id_tag, corrupt }),
        3 => (0..8usize, 0..8u8, 0..4u8, 0..6u8)
            .prop_map(|(target_idx, row, col, val_kind)| SweepStep::SetCells { target_idx, row, col, val_kind }),
        2 => (0..8usize, 0..6u8, 0..4u8, proptest::arbitrary::any::<bool>(), proptest::bool::weighted(0.25))
            .prop_map(|(target_idx, row, col, excluded, huge)| SweepStep::ExcludeCells { target_idx, row, col, excluded, huge }),
        2 => (0..8usize, 0..8usize, 0..5u8)
            .prop_map(|(dst_idx, src_idx, cyclic_or_missing)| SweepStep::PasteLink { dst_idx, src_idx, cyclic_or_missing }),
        2 => (0..8usize, 0..5u8)
            .prop_map(|(target_idx, formula_kind)| SweepStep::SetCalculatedVariable { target_idx, formula_kind }),
        2 => (0..8usize, 0..4usize).prop_map(|(src_idx, kind_idx)| SweepStep::TransformRun { src_idx, kind_idx }),
        2 => (0..8usize, 0..4usize).prop_map(|(src_idx, kind_idx)| SweepStep::StatsRun { src_idx, kind_idx }),
        2 => (0..8usize, 0..4usize).prop_map(|(src_idx, kind_idx)| SweepStep::FitRun { src_idx, kind_idx }),
        2 => (0..8usize, 0..5usize).prop_map(|(src_idx, kind_idx)| SweepStep::GraphInitialize { src_idx, kind_idx }),
        2 => (0..8usize, proptest::bool::weighted(0.3)).prop_map(|(graph_idx, hostile)| SweepStep::GraphSetAxes { graph_idx, hostile }),
        2 => (0..8usize, 0..8usize).prop_map(|(graph_idx, res_idx)| SweepStep::GraphAddComparisonBrackets { graph_idx, res_idx }),
        2 => (0..8usize, 0..8usize).prop_map(|(dst_idx, src_idx)| SweepStep::GraphApplyStyleFrom { dst_idx, src_idx }),
        1 => proptest::bool::weighted(0.25).prop_map(|hostile| SweepStep::LayoutCreate { hostile }),
        2 => (0..6usize, 0..8usize, 0..6u8, proptest::bool::weighted(0.25))
            .prop_map(|(lay_idx, graph_idx, slot_idx, hostile)| SweepStep::LayoutAddPanel { lay_idx, graph_idx, slot_idx, hostile }),
        1 => (0..8usize).prop_map(|src_idx| SweepStep::ClonePipeline { src_idx }),
        1 => (0..8usize).prop_map(|target_idx| SweepStep::Freeze { target_idx }),
        1 => (0..8usize).prop_map(|target_idx| SweepStep::Unfreeze { target_idx }),
        1 => (0..8usize, proptest::arbitrary::any::<bool>()).prop_map(|(target_idx, cascade)| SweepStep::DeleteSheet { target_idx, cascade }),
        3 => Just(SweepStep::Undo),
        2 => Just(SweepStep::Redo),
    ]
    .boxed()
}

fn pick_data_id(doc: &ProjectDocument, idx: usize) -> String {
    if doc.data_sheets.is_empty() {
        String::from("missing-data")
    } else {
        doc.data_sheets.get(idx % doc.data_sheets.len()).map_or_else(|| String::from("missing-data"), |d| d.id.as_str().to_string())
    }
}

fn pick_mv_or_any_id(doc: &ProjectDocument, idx: usize) -> String {
    let mv: Vec<&str> = doc.data_sheets.iter().filter(|d| d.archetype.kind() == TableKind::MultipleVariables).map(|d| d.id.as_str()).collect();
    if idx % 4 != 3
        && !mv.is_empty()
        && let Some(&id) = mv.get(idx % mv.len())
    {
        id.to_string()
    } else {
        pick_data_id(doc, idx)
    }
}

fn pick_result_id(doc: &ProjectDocument, idx: usize, prefer_pairwise: bool) -> String {
    if prefer_pairwise {
        let with_pw: Vec<&str> = doc.result_sheets.iter().filter(|r| !r.pairwise_comparisons.is_empty()).map(|r| r.id.as_str()).collect();
        if !with_pw.is_empty()
            && let Some(&id) = with_pw.get(idx % with_pw.len())
        {
            return id.to_string();
        }
    }
    if doc.result_sheets.is_empty() {
        String::from("missing-result")
    } else {
        doc.result_sheets.get(idx % doc.result_sheets.len()).map_or_else(|| String::from("missing-result"), |r| r.id.as_str().to_string())
    }
}

fn pick_graph_id(doc: &ProjectDocument, idx: usize) -> String {
    if doc.graph_sheets.is_empty() {
        String::from("missing-graph")
    } else {
        doc.graph_sheets.get(idx % doc.graph_sheets.len()).map_or_else(|| String::from("missing-graph"), |g| g.id.as_str().to_string())
    }
}

fn pick_layout_id(doc: &ProjectDocument, idx: usize) -> String {
    if doc.layout_sheets.is_empty() {
        String::from("missing-layout")
    } else {
        doc.layout_sheets.get(idx % doc.layout_sheets.len()).map_or_else(|| String::from("missing-layout"), |l| l.id.as_str().to_string())
    }
}

fn pick_any_sheet_id(doc: &ProjectDocument, idx: usize) -> String {
    let mut all: Vec<&str> = Vec::new();
    all.extend(doc.data_sheets.iter().map(|s| s.id.as_str()));
    all.extend(doc.result_sheets.iter().map(|s| s.id.as_str()));
    all.extend(doc.graph_sheets.iter().map(|s| s.id.as_str()));
    all.extend(doc.layout_sheets.iter().map(|s| s.id.as_str()));
    if all.is_empty() { String::from("missing-any") } else { all.get(idx % all.len()).copied().unwrap_or("missing-any").to_string() }
}

fn build_step_command(doc: &ProjectDocument, step: &SweepStep, step_num: usize) -> (&'static str, Value) {
    match step {
        SweepStep::CreateData { arch_idx, cols, rows, id_tag, corrupt } => {
            if *corrupt {
                let p = match arch_idx % 3 {
                    0 => json!({ "sheet_id": "bad", "rows": u64::MAX }),
                    1 => json!({ "sheet_id": format!("fuzz-{step_num}"), "archetype": "not_a_table" }),
                    _ => json!({ "sheet_id": pick_data_id(doc, *arch_idx), "archetype": "xy" }),
                };
                return ("sheet.createData", p);
            }
            let arch = ["xy", "column", "grouped", "multiple_variables", "nested"].get(arch_idx % 5).copied().unwrap_or("xy");
            let sid = format!("d-{step_num}-{id_tag}");
            if arch == "multiple_variables" {
                (
                    "sheet.createData",
                    json!({
                        "sheet_id": sid,
                        "archetype": "multiple_variables",
                        "cols": 3,
                        "rows": *rows,
                        "variables": [
                            { "title": "Cohort", "role": "Categorical" },
                            { "title": "Score", "role": "Continuous" },
                            { "title": "Scaled", "role": "Continuous" }
                        ]
                    }),
                )
            } else {
                ("sheet.createData", json!({ "sheet_id": sid, "archetype": arch, "cols": *cols, "rows": *rows }))
            }
        }
        SweepStep::SetCells { target_idx, row, col, val_kind } => {
            let sid = pick_data_id(doc, *target_idx);
            let val = match val_kind % 6 {
                0 => json!(10.0 + f64::from(*row)),
                1 => json!(f64::from(*col) * 3.5 + 1.0),
                2 => json!("GroupA"),
                3 => json!("NaN"),
                4 => json!("Infinity"),
                _ => json!([1, 2]),
            };
            let r = if *val_kind == 5 { json!(u64::MAX) } else { json!(*row) };
            ("table.setCells", json!({ "sheet_id": sid, "edits": [{ "row": r, "col": *col, "value": val }] }))
        }
        SweepStep::ExcludeCells { target_idx, row, col, excluded, huge } => {
            let sid = pick_data_id(doc, *target_idx);
            let r = if *huge { u64::MAX } else { u64::from(*row) };
            ("table.excludeCells", json!({ "sheet_id": sid, "cells": [{ "row": r, "col": *col }], "excluded": *excluded }))
        }
        SweepStep::PasteLink { dst_idx, src_idx, cyclic_or_missing } => {
            let dst = pick_data_id(doc, *dst_idx);
            let src = match cyclic_or_missing % 4 {
                0 => dst.clone(),
                1 => String::from("nonexistent-link-source"),
                _ => pick_data_id(doc, *src_idx),
            };
            (
                "table.pasteLink",
                json!({
                    "target_sheet_id": dst,
                    "source_sheet_id": src,
                    "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 1, "col": 0 } },
                    "start_row": 0,
                    "start_col": 0
                }),
            )
        }
        SweepStep::SetCalculatedVariable { target_idx, formula_kind } => {
            let sid = pick_mv_or_any_id(doc, *target_idx);
            let (col, title, formula) = match formula_kind % 5 {
                0 => (2u64, "Scaled", "= [Score] * 2"),
                1 => (2u64, "Scaled", "= [Score] + 10"),
                2 => (2u64, "Scaled", "= [MissingCol] / 2"),
                3 => (2u64, "Scaled", "= [Scaled] + 1"),
                _ => (u64::MAX, "BadCol", "= 1"),
            };
            ("table.setCalculatedVariable", json!({ "sheet_id": sid, "col": col, "title": title, "formula": formula }))
        }
        SweepStep::TransformRun { src_idx, kind_idx } => {
            let src = if *kind_idx == 3 { String::from("missing-src") } else { pick_data_id(doc, *src_idx) };
            let kind = ["normalize", "log10", "baseline_subtract", "bogus_xform"].get(kind_idx % 4).copied().unwrap_or("normalize");
            ("transform.run", json!({ "sheet_id": format!("rx-{step_num}"), "source_sheet_id": src, "kind": kind }))
        }
        SweepStep::StatsRun { src_idx, kind_idx } => {
            let src = if *kind_idx == 3 { String::from("missing-src") } else { pick_data_id(doc, *src_idx) };
            let kind =
                ["descriptive_stats", "one_way_anova", "unpaired_t_test", "bogus_stat"].get(kind_idx % 4).copied().unwrap_or("descriptive_stats");
            ("stats.run", json!({ "sheet_id": format!("rs-{step_num}"), "source_sheet_id": src, "kind": kind }))
        }
        SweepStep::FitRun { src_idx, kind_idx } => {
            let src = if *kind_idx == 3 { String::from("missing-src") } else { pick_data_id(doc, *src_idx) };
            let kind = ["linear_regression", "nonlinear_4pl", "exponential_one_phase_decay", "bogus_fit"]
                .get(kind_idx % 4)
                .copied()
                .unwrap_or("linear_regression");
            ("fit.run", json!({ "sheet_id": format!("rf-{step_num}"), "source_sheet_id": src, "kind": kind }))
        }
        SweepStep::GraphInitialize { src_idx, kind_idx } => {
            let uninit = doc.graph_sheets.iter().find(|g| !g.initialized).map(|g| g.id.as_str().to_string());
            let kind = ["xy", "column", "grouped", "multiple_variables", "bad_graph_kind"].get(kind_idx % 5).copied().unwrap_or("xy");
            if let Some(gid) = uninit
                && *kind_idx != 4
            {
                ("graph.initialize", json!({ "sheet_id": gid, "kind": kind }))
            } else {
                let src = if *kind_idx == 4 { String::from("missing-src") } else { pick_data_id(doc, *src_idx) };
                ("graph.create", json!({ "sheet_id": format!("g-{step_num}"), "data_sheet_id": src, "kind": kind, "initialized": *src_idx % 2 == 0 }))
            }
        }
        SweepStep::GraphSetAxes { graph_idx, hostile } => {
            let gid = pick_graph_id(doc, *graph_idx);
            if *hostile {
                ("graph.setAxes", json!({ "sheet_id": gid, "x_axis": { "min": 50.0, "max": 5.0 }, "axis": "y", "segments": [{}, {}, {}, {}] }))
            } else {
                ("graph.setAxes", json!({ "sheet_id": gid, "x_axis": { "scale": "linear" }, "y_axis": { "scale": "linear" } }))
            }
        }
        SweepStep::GraphAddComparisonBrackets { graph_idx, res_idx } => (
            "graph.addComparisonBrackets",
            json!({
                "sheet_id": pick_graph_id(doc, *graph_idx),
                "result_sheet_id": pick_result_id(doc, *res_idx, true),
                "significant_only": false
            }),
        ),
        SweepStep::GraphApplyStyleFrom { dst_idx, src_idx } => (
            "graph.applyStyleFrom",
            json!({
                "sheet_id": pick_graph_id(doc, *dst_idx),
                "donor_sheet_id": pick_graph_id(doc, *src_idx)
            }),
        ),
        SweepStep::LayoutCreate { hostile } => {
            if *hostile {
                ("layout.create", json!({ "sheet_id": "", "preset": 123 }))
            } else {
                ("layout.create", json!({ "sheet_id": format!("l-{step_num}"), "preset": "double_column", "grid_template": [2, 2] }))
            }
        }
        SweepStep::LayoutAddPanel { lay_idx, graph_idx, slot_idx, hostile } => {
            let lid = pick_layout_id(doc, *lay_idx);
            let gid = pick_graph_id(doc, *graph_idx);
            if *hostile {
                ("layout.addPanel", json!({ "sheet_id": lid, "kind": "graph", "graph_sheet_id": gid, "bounds_pt": ["bad"] }))
            } else {
                let offset = f64::from(*slot_idx % 4) * 30.0;
                (
                    "layout.addPanel",
                    json!({ "sheet_id": lid, "kind": "graph", "graph_sheet_id": gid, "bounds_pt": [20.0 + offset, 20.0 + offset, 180.0, 120.0] }),
                )
            }
        }
        SweepStep::ClonePipeline { src_idx } => {
            ("sheet.clonePipeline", json!({ "sheet_id": pick_data_id(doc, *src_idx), "prefix": format!("cp{step_num}"), "clear_data": false }))
        }
        SweepStep::Freeze { target_idx } => ("sheet.freeze", json!({ "sheet_id": pick_any_sheet_id(doc, *target_idx) })),
        SweepStep::Unfreeze { target_idx } => ("sheet.unfreeze", json!({ "sheet_id": pick_any_sheet_id(doc, *target_idx) })),
        SweepStep::DeleteSheet { target_idx, cascade } => {
            ("sheet.delete", json!({ "sheet_id": pick_any_sheet_id(doc, *target_idx), "cascade": *cascade }))
        }
        SweepStep::Undo => ("edit.undo", json!({})),
        SweepStep::Redo => ("edit.redo", json!({})),
    }
}

fn execute_tracked_step(
    session: &mut Session,
    undo_model: &mut Vec<ProjectDocument>,
    redo_model: &mut Vec<ProjectDocument>,
    cmd: &str,
    params: Value,
) -> Result<(), TestCaseError> {
    let before_session = session.clone();
    let before_doc = before_session.document().clone();
    let res = session.execute(cmd, params);

    match res {
        Err(err) => {
            prop_assert!(!err.message().contains("recovered from internal panic"), "command `{cmd}` panicked internally: {}", err.message());
            prop_assert_eq!(session.document(), &before_doc);
            prop_assert_eq!(session, &before_session);
        }
        Ok(outcome) => {
            prop_assert_eq!(outcome.command_id.as_str(), cmd);
            if cmd == "edit.undo" {
                if let Some(expected_prev) = undo_model.pop() {
                    prop_assert!(outcome.mutated, "expected edit.undo to mutate when undo stack non-empty");
                    prop_assert_eq!(session.document(), &expected_prev);
                    redo_model.push(before_doc);
                } else {
                    prop_assert!(!outcome.mutated, "expected edit.undo no-op when undo stack empty");
                    prop_assert_eq!(session.document(), &before_doc);
                }
            } else if cmd == "edit.redo" {
                if let Some(expected_next) = redo_model.pop() {
                    prop_assert!(outcome.mutated, "expected edit.redo to mutate when redo stack non-empty");
                    prop_assert_eq!(session.document(), &expected_next);
                    undo_model.push(before_doc);
                } else {
                    prop_assert!(!outcome.mutated, "expected edit.redo no-op when redo stack empty");
                    prop_assert_eq!(session.document(), &before_doc);
                }
            } else if outcome.mutated {
                undo_model.push(before_doc);
                redo_model.clear();
            } else {
                prop_assert_eq!(session.document(), &before_doc);
            }
        }
    }
    Ok(())
}

fn verify_full_undo_redo_roundtrip(
    session: &mut Session,
    undo_model: &mut Vec<ProjectDocument>,
    redo_model: &mut Vec<ProjectDocument>,
    initial_doc: &ProjectDocument,
) -> Result<(), TestCaseError> {
    while !undo_model.is_empty() {
        execute_tracked_step(session, undo_model, redo_model, "edit.undo", json!({}))?;
    }
    prop_assert_eq!(session.document(), initial_doc);

    // Extra undo at root must be a clean non-mutating no-op.
    execute_tracked_step(session, undo_model, redo_model, "edit.undo", json!({}))?;
    prop_assert_eq!(session.document(), initial_doc);

    while !redo_model.is_empty() {
        execute_tracked_step(session, undo_model, redo_model, "edit.redo", json!({}))?;
    }

    // Extra redo at tip must be a clean non-mutating no-op.
    let tip_doc = session.document().clone();
    execute_tracked_step(session, undo_model, redo_model, "edit.redo", json!({}))?;
    prop_assert_eq!(session.document(), &tip_doc);
    Ok(())
}

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]

    #[test]
    fn proptest_interleaved_sequences_zero_panic_rollback_and_undo_redo(
        steps in vec(sweep_step_strategy(), 12..36)
    ) {
        let session = seed_rich_fixture().map_err(TestCaseError::fail)?;
        // Reset stacks cleanly onto initial_doc by seeding via from_document so full root undo matches initial_doc.
        let initial_doc = session.document().clone();
        let mut session = Session::from_document(initial_doc.clone());
        let mut undo_model: Vec<ProjectDocument> = Vec::new();
        let mut redo_model: Vec<ProjectDocument> = Vec::new();

        for (step_num, step) in steps.iter().enumerate() {
            let (cmd, params) = build_step_command(session.document(), step, step_num);
            execute_tracked_step(&mut session, &mut undo_model, &mut redo_model, cmd, params)?;
        }

        verify_full_undo_redo_roundtrip(&mut session, &mut undo_model, &mut redo_model, &initial_doc)?;
    }
}
