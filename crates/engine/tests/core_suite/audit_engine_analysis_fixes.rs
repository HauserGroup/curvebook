//! Regression tests for Wave 3 engine analysis & validation audit findings
//! (#4, #7, #32, #58, #78, #79, #80, #93, #113, #114, #120).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, BlandAltmanSpec, Cell, DataSheet, FitMethod, ResultCell, ResultStatus, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype,
    VariableRole, XyTable,
};
use serde_json::json;

use super::common::{
    assert_bad_input, assert_bad_inputs, col_sheet, grouped_rep_sheet, mv_sheet, ok_cmd, rs, run_cmds, scalar, session_with, xy_rep_sheet, xy_sheet,
};

fn make_single_xy(id: &str) -> DataSheet {
    xy_sheet(id, "Single XY", "X", &[1.0, 2.0, 3.0, 4.0, 5.0], &[("Y0", &[4.0, 5.0, 6.0, 7.0, 8.0])])
}

fn make_two_xy(id: &str) -> DataSheet {
    xy_sheet(id, "Two XY", "X", &[1.0, 2.0, 3.0, 4.0, 5.0], &[("Y0", &[4.0, 5.0, 6.0, 7.0, 8.0]), ("Y1", &[201.0, 202.0, 203.0, 204.0, 205.0])])
}

fn make_audit_col(id: &str, cols: usize, rows: usize) -> DataSheet {
    let data: Vec<(String, Vec<f64>)> =
        (0..cols).map(|c| (format!("G{c}"), (0..rows).map(|r| (c * 10 + r + 1) as f64 + 0.1 * (r as f64)).collect())).collect();
    let refs: Vec<(&str, &[f64])> = data.iter().map(|(n, v)| (n.as_str(), v.as_slice())).collect();
    col_sheet(id, "Col Sheet", &refs)
}

#[test]
fn audit_4_bland_altman_xy_mode_persists_across_column_additions_updates_and_retarget() {
    let mut session = session_with("Audit #4 Bland-Altman XY", vec![make_single_xy("xy1"), make_two_xy("xy2"), make_audit_col("col2", 2, 5)]);

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "ba-xy1", "source_sheet_id": "xy1", "kind": "bland_altman" }));
    let bias_before = scalar(&session, "ba-xy1", "Bias");
    assert!((bias_before.abs() - 3.0).abs() < 1e-9);
    assert!(matches!(
        rs(&session, "ba-xy1").analysis.kind,
        AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(true), column_a: 0, column_b: 1, .. })
    ));

    // Pasting a 2nd Y data set or updating unrelated params preserves xy_compare_x_vs_y = Some(true).
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "xy1", "start_row": 0, "start_col": 1, "text": "200\n201\n202\n203\n204" }));
    assert!((scalar(&session, "ba-xy1", "Bias") - bias_before).abs() < 1e-9);
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "ba-xy1", "confidence_percent": 99.0 }));
    assert!((scalar(&session, "ba-xy1", "Bias") - bias_before).abs() < 1e-9);
    assert!(matches!(rs(&session, "ba-xy1").analysis.kind, AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(true), .. })));

    // 2-dataset XY defaults to Some(false) unless xy_compare_x_vs_y: true; invalid column_b rejected.
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "ba-xy2", "source_sheet_id": "xy2", "kind": "bland_altman" })),
            ("analysis.run", json!({ "sheet_id": "ba-xy2-x", "source_sheet_id": "xy2", "kind": "bland_altman", "xy_compare_x_vs_y": true })),
        ],
    );
    assert!(matches!(rs(&session, "ba-xy2").analysis.kind, AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(false), .. })));
    assert!(matches!(rs(&session, "ba-xy2-x").analysis.kind, AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(true), .. })));
    assert!((scalar(&session, "ba-xy2-x", "Bias").abs() - 3.0).abs() < 1e-9);
    assert_bad_input(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "xy2", "kind": "bland_altman", "xy_compare_x_vs_y": true, "column_b": 2 }),
    );

    // Retargeting recomputes xy_compare_x_vs_y for XY and clears to None for Column.
    ok_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "ba-xy1", "source_sheet_id": "xy2" }));
    assert!(matches!(rs(&session, "ba-xy1").analysis.kind, AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(false), .. })));
    ok_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "ba-xy1", "source_sheet_id": "col2" }));
    assert!(matches!(rs(&session, "ba-xy1").analysis.kind, AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: None, .. })));
}

#[test]
fn audit_7_paired_multiple_row_tests_return_bad_input() {
    let grp3 = grouped_rep_sheet(
        "grp3",
        "Grp3",
        &["R0", "R1", "R2", "R3"],
        &[
            ("Arm0", &[&[1.1, 1.7, 0.7], &[2.1, 2.7, 1.7], &[3.1, 3.7, 2.7], &[4.1, 4.7, 3.7]]),
            ("Arm1", &[&[7.2, 8.4, 6.4], &[8.2, 9.4, 7.4], &[9.2, 10.4, 8.4], &[10.2, 11.4, 9.4]]),
            ("Arm2", &[&[13.3, 15.1, 12.1], &[14.3, 16.1, 13.1], &[15.3, 17.1, 14.1], &[16.3, 18.1, 15.1]]),
        ],
    );
    let xy3 = xy_rep_sheet(
        "xy3",
        "Xy3",
        "X",
        &[1.0, 2.0, 3.0, 4.0],
        &[
            ("A", &[&[1.0, 2.0], &[2.0, 3.0], &[3.0, 4.0], &[4.0, 5.0]]),
            ("B", &[&[6.0, 7.0], &[7.0, 8.0], &[8.0, 9.0], &[9.0, 10.0]]),
            ("C", &[&[11.0, 12.0], &[12.0, 13.0], &[13.0, 14.0], &[14.0, 15.0]]),
        ],
    );
    let mv_cols: Vec<Vec<Cell>> = (0..3).map(|c| (0..4).map(|r| Cell::number((c * 4 + r + 1) as f64)).collect()).collect();
    let mv3 = mv_sheet("mv3", "Mv3", &[], &[("V0", None, &mv_cols[0]), ("V1", None, &mv_cols[1]), ("V2", None, &mv_cols[2])]);
    let mut session = session_with("Audit #7 Paired Row Tests", vec![grp3, xy3, make_audit_col("col3", 3, 4), make_audit_col("col4", 4, 4), mv3]);

    assert_bad_inputs(
        &mut session,
        &[
            ("stats.run", json!({ "source_sheet_id": "grp3", "kind": "multiple_row_t_tests", "paired": true })),
            ("stats.run", json!({ "source_sheet_id": "grp3", "kind": "multiple_row_t_tests", "paired": true, "selected_data_sets": [0, 1, 2] })),
            ("stats.run", json!({ "source_sheet_id": "grp3", "kind": "multiple_row_t_tests", "paired": false, "selected_data_sets": [0, 1, 2] })),
            ("stats.run", json!({ "source_sheet_id": "xy3", "kind": "multiple_row_t_tests", "paired": true })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "multiple_row_t_tests", "paired": true })),
            ("stats.run", json!({ "source_sheet_id": "mv3", "kind": "multiple_row_t_tests", "paired": true })),
        ],
    );

    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "mr-ok", "source_sheet_id": "grp3", "kind": "multiple_row_t_tests", "paired": true, "selected_data_sets": [0, 1] }),
    );
    assert_eq!(rs(&session, "mr-ok").status, ResultStatus::UpToDate);

    // Delayed recalc after expanding Column sheet from 4 to 5 columns maps to BadInput.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "mr-col", "source_sheet_id": "col4", "kind": "multiple_row_t_tests", "paired": true }));
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "col4", "start_row": 0, "start_col": 4, "text": "9\n10\n11\n12" }));
    assert_bad_input(&mut session, "analysis.run", json!({ "sheet_id": "mr-col" }));
}

#[test]
fn audit_32_selected_data_sets_rejects_out_of_range_and_duplicates() {
    let mv_nl = mv_sheet(
        "mv-nl",
        "MV Nonlinear",
        &[],
        &[
            (
                "Arm",
                Some(VariableRole::Categorical),
                &[Cell::text("Vehicle"), Cell::text("Vehicle"), Cell::text("Vehicle"), Cell::text("Drug"), Cell::text("Drug"), Cell::text("Drug")],
            ),
            (
                "LogConc",
                None,
                &[Cell::number(-8.0), Cell::number(-7.0), Cell::number(-6.0), Cell::number(-8.0), Cell::number(-7.0), Cell::number(-6.0)],
            ),
            ("Resp", None, &[Cell::number(10.0), Cell::number(35.0), Cell::number(60.0), Cell::number(20.0), Cell::number(45.0), Cell::number(70.0)]),
        ],
    );
    let mut session = session_with("Audit #32", vec![make_audit_col("col2", 2, 5), make_two_xy("xy2"), mv_nl]);

    for (cmd, src, kind) in [
        ("stats.run", "col2", "descriptive"),
        ("analysis.run", "col2", "one_way_anova"),
        ("fit.run", "xy2", "linear"),
        ("transform.run", "xy2", "normalize"),
    ] {
        assert_bad_inputs(
            &mut session,
            &[
                (cmd, json!({ "source_sheet_id": src, "kind": kind, "selected_data_sets": [0, 0] })),
                (cmd, json!({ "source_sheet_id": src, "kind": kind, "selected_data_sets": [0, 5] })),
            ],
        );
    }

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "desc-1", "source_sheet_id": "col2", "kind": "descriptive" }));
    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "nl-grp-ok", "source_sheet_id": "mv-nl", "kind": "straight_line", "mv_group_columns": [0], "mv_x_columns": [1], "mv_y_column": 2, "selected_data_sets": [1] }),
    );
    assert_bad_inputs(
        &mut session,
        &[
            ("analysis.updateParams", json!({ "sheet_id": "desc-1", "selected_data_sets": [1, 1] })),
            ("analysis.updateParams", json!({ "sheet_id": "desc-1", "selected_data_sets": [99] })),
            ("analysis.retargetData", json!({ "sheet_id": "desc-1", "source_sheet_id": "xy2", "selected_data_sets": [0, 0] })),
            ("analysis.retargetData", json!({ "sheet_id": "desc-1", "source_sheet_id": "xy2", "selected_data_sets": [4] })),
            (
                "fit.run",
                json!({ "source_sheet_id": "mv-nl", "kind": "straight_line", "mv_group_columns": [0], "mv_x_columns": [1], "mv_y_column": 2, "selected_data_sets": [2] }),
            ),
        ],
    );
}

#[test]
fn audit_58_transform_rejects_unknown_operation_and_anchors() {
    let mut session = session_with("Audit #58", vec![make_two_xy("xy2")]);
    assert_bad_inputs(
        &mut session,
        &[
            ("transform.run", json!({ "source_sheet_id": "xy2", "kind": "normalize", "hundred_anchor": "maximum" })),
            ("transform.run", json!({ "source_sheet_id": "xy2", "kind": "normalize", "zero_anchor": "bogus" })),
            ("transform.run", json!({ "source_sheet_id": "xy2", "kind": "remove_baseline", "operation": "exponentiate" })),
        ],
    );
    run_cmds(
        &mut session,
        &[
            (
                "transform.run",
                json!({ "sheet_id": "norm-ok", "source_sheet_id": "xy2", "kind": "normalize", "zero_anchor": "smallest", "hundred_anchor": "largest" }),
            ),
            (
                "transform.run",
                json!({ "sheet_id": "base-ok", "source_sheet_id": "xy2", "kind": "remove_baseline", "baseline_columns": [0], "operation": "pct_change" }),
            ),
            (
                "transform.run",
                json!({ "sheet_id": "base-ok2", "source_sheet_id": "xy2", "kind": "remove_baseline", "baseline_columns": [0], "operation": "pctchange" }),
            ),
            (
                "transform.run",
                json!({ "sheet_id": "base-add", "source_sheet_id": "xy2", "kind": "remove_baseline", "baseline_columns": [0], "operation": "add" }),
            ),
            (
                "transform.run",
                json!({ "sheet_id": "base-mul", "source_sheet_id": "xy2", "kind": "remove_baseline", "baseline_columns": [0], "operation": "multiply" }),
            ),
        ],
    );
    assert_bad_inputs(
        &mut session,
        &[
            ("analysis.updateParams", json!({ "sheet_id": "norm-ok", "zero_anchor": "minimum" })),
            ("analysis.updateParams", json!({ "sheet_id": "base-ok", "operation": "bogus_op" })),
        ],
    );
}

#[test]
fn audit_78_confidence_level_fraction_rejected_across_all_commands() {
    let mut session = session_with("Audit #78", vec![make_audit_col("col2", 2, 5), make_single_xy("xy1")]);
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-base", "source_sheet_id": "col2", "kind": "unpaired_t" }));

    for (cmd, payload) in [
        ("analysis.run", json!({ "source_sheet_id": "col2", "kind": "unpaired_t", "confidence_percent": 0.95 })),
        ("analysis.run", json!({ "source_sheet_id": "col2", "kind": "unpaired_t", "confidence_percent": 1.0 })),
        ("stats.run", json!({ "source_sheet_id": "col2", "kind": "descriptive", "confidence_percent": 0.99 })),
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "confidence_percent": 0.95 })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "math", "confidence_percent": 0.95 })),
        ("analysis.updateParams", json!({ "sheet_id": "res-base", "confidence_percent": 0.95 })),
        ("analysis.updateParams", json!({ "sheet_id": "res-base", "params": { "confidence_percent": 0.95 } })),
    ] {
        let err = assert_bad_input(&mut session, cmd, payload);
        let combined = format!("{} {}", err.message(), err.remediation.as_deref().unwrap_or_default());
        assert!(combined.contains("95") && combined.contains("percentage"), "missing remediation: {combined}");
    }

    assert_bad_input(
        &mut session,
        "analysis.run",
        json!({
            "spec": {
                "id": "bad-conf-spec", "source_sheet_id": "col2", "selected_data_sets": [], "auto_title": "Unpaired t test",
                "custom_title": null, "confidence_level": 0.95, "p_value_format": "FourDecimal",
                "kind": { "DescriptiveStats": { "aggregation": "RowMeans", "custom_upper_percentile": null, "run_normality_battery": true, "hypothetical_value": null } }
            }
        }),
    );
}

#[test]
fn audit_79_rejects_unknown_and_inapplicable_keys_per_kind() {
    let mut session = session_with("Audit #79", vec![make_audit_col("col3", 3, 5), make_single_xy("xy1")]);
    let bad_hook_rule = json!([{ "name": "ConstraintA", "scope": { "HookedConstant": { "InfoConstant": { "info_sheet_id": "info-1", "constant_name": "DoseRef" } } } }]);

    assert_bad_inputs(
        &mut session,
        &[
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "unpaired_t", "alpha": 0.01 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "unpaired_t", "hypothetical": 5.0 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "unpaired_t", "bogus_param": true })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "descriptive", "hypothetical_mean": 1.0 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "geisser_greenhouse": true })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "multiplicity": "tukey", "control_column": 0 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "grubbs", "q_percent": 1.0 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "rout", "alpha": 0.05 })),
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "equation_id": "4pl" })),
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "alpha": 0.05 })),
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "q_percent": 2.0 })),
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "alternative_equation_id": "log_agonist_vs_response_3pl" })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "normalize", "every_k_rows": 2 })),
            (
                "analysis.run",
                json!({ "source_sheet_id": "col3", "kind": { "DescriptiveStats": { "aggregation": "RowMeans", "custom_upper_percentile": null, "run_normality_battery": true, "hypothetical_value": null, "bogus": 1 } } }),
            ),
            ("analysis.run", json!({ "sheet_id": "nonexistent", "bogus": 1 })),
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "parameter_rules": bad_hook_rule })),
            ("analysis.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "parameter_rules": bad_hook_rule })),
        ],
    );

    run_cmds(
        &mut session,
        &[
            (
                "stats.run",
                json!({ "sheet_id": "rm-ok", "source_sheet_id": "col3", "kind": "rm_one_way_anova", "multiplicity": "tukey", "geisser_greenhouse": false }),
            ),
            (
                "stats.run",
                json!({ "sheet_id": "dun-ok", "source_sheet_id": "col3", "kind": "one_way_anova", "multiplicity": "dunnett", "control_column": 0 }),
            ),
            ("fit.run", json!({ "sheet_id": "lin-hook", "source_sheet_id": "xy1", "kind": "linear" })),
            ("analysis.updateParams", json!({ "sheet_id": "lin-hook", "parameter_rules": bad_hook_rule })),
        ],
    );
    assert_bad_inputs(
        &mut session,
        &[
            ("analysis.updateParams", json!({ "sheet_id": "rm-ok", "bogus": 1 })),
            ("analysis.updateParams", json!({ "sheet_id": "rm-ok", "params": { "bogus": 1 } })),
            ("analysis.updateParams", json!({ "sheet_id": "rm-ok", "control_column": 0 })),
            ("analysis.updateParams", json!({ "sheet_id": "lin-hook", "parameter_rules": "invalid" })),
            ("analysis.updateParams", json!({ "sheet_id": "lin-hook", "params": { "parameter_rules": bad_hook_rule } })),
        ],
    );
}

#[test]
fn audit_80_nonlinear_rout_q_override_and_bounds() {
    let xy_dr = xy_sheet(
        "xy-dr",
        "Dose",
        "X",
        &[-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0],
        &[("Y", &[10.0, 12.0, 22.0, 52.0, 84.0, 96.0, 99.0, 100.0])],
    );
    let mut session = session_with("Audit #80 ROUT Q", vec![xy_dr]);

    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "rout-5", "source_sheet_id": "xy-dr", "kind": "4pl", "method": "rout", "q_percent": 5.0 }));
    assert!(
        matches!(rs(&session, "rout-5").analysis.kind, AnalysisKind::NonlinearRegression(ref nl) if matches!(nl.method, FitMethod::RoutOutlierElimination { fdr_q_percent } if (fdr_q_percent - 5.0).abs() < 1e-12))
    );
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "rout-50", "source_sheet_id": "xy-dr", "kind": "4pl", "method": "rout", "q_percent": 50.0 }));

    for bad_q in [0.0, -1.0, 50.1] {
        assert_bad_inputs(
            &mut session,
            &[
                ("fit.run", json!({ "source_sheet_id": "xy-dr", "kind": "4pl", "method": "rout", "q_percent": bad_q })),
                ("analysis.updateParams", json!({ "sheet_id": "rout-5", "q_percent": bad_q })),
            ],
        );
    }

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "rout-5", "q_percent": 2.5 }));
    assert!(
        matches!(rs(&session, "rout-5").analysis.kind, AnalysisKind::NonlinearRegression(ref nl) if matches!(nl.method, FitMethod::RoutOutlierElimination { fdr_q_percent } if (fdr_q_percent - 2.5).abs() < 1e-12))
    );
}

#[test]
fn audit_93_mv_default_columns_persisted_before_appending_calculated_var() {
    let doses = [-8.0, -7.0, -6.0, -5.0, -4.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let bio_g: Vec<Cell> = (0..10).map(|r| Cell::text(if r < 5 { "A" } else { "B" })).collect();
    let bio_d: Vec<Cell> = doses.iter().map(|&d| Cell::number(d)).collect();
    let bio_y: Vec<Cell> = doses
        .iter()
        .enumerate()
        .map(|(r, &d)| {
            Cell::number(if r < 5 { 10.0 + 90.0 / (1.0 + 10.0_f64.powf(-6.0 - d)) } else { 15.0 + 80.0 / (1.0 + 10.0_f64.powf(-5.5 - d)) })
        })
        .collect();
    let mv_bio =
        mv_sheet("mv-bio", "MV Bio", &[], &[("Group", Some(VariableRole::Categorical), &bio_g), ("Dose", None, &bio_d), ("Resp", None, &bio_y)]);

    let ca: Vec<Cell> = (1..=6).map(|i| Cell::number(i as f64)).collect();
    let cb: Vec<Cell> = (1..=6).map(|i| Cell::number(2.0 * (i as f64) + 1.0)).collect();
    let cc: Vec<Cell> = (0..6).map(|r| Cell::number(0.5 * ((r + 1) as f64) + (r % 2) as f64)).collect();
    let mv_cont = mv_sheet("mv-cont", "MV Cont", &[], &[("A", None, &ca), ("B", None, &cb), ("C", None, &cc)]);

    let km_a: Vec<Cell> = (0..8).map(|r| Cell::text(if r < 4 { "Placebo" } else { "Active" })).collect();
    let km_t: Vec<Cell> = (0..8).map(|r| Cell::number((r * 2 + 5) as f64)).collect();
    let km_s: Vec<Cell> = (0..8).map(|r| Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 })).collect();
    let mv_km = mv_sheet(
        "mv-km",
        "MV KM",
        &[],
        &[("Arm", Some(VariableRole::Categorical), &km_a), ("FollowupTime", None, &km_t), ("EventStatus", None, &km_s)],
    );

    let all_cols: Vec<Vec<Cell>> = (0..4).map(|c| (0..6).map(|r| Cell::number(((r + 1) * (c + 1)) as f64)).collect()).collect();
    let mv_all = mv_sheet(
        "mv-all-cont",
        "MV All Cont",
        &[],
        &[("V0", None, &all_cols[0]), ("V1", None, &all_cols[1]), ("V2", None, &all_cols[2]), ("V3", None, &all_cols[3])],
    );

    let mut session = session_with("Audit #93", vec![mv_bio, mv_cont, mv_km, mv_all]);
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "tg-mv", "source_sheet_id": "mv-bio", "kind": "unpaired_t" })),
            ("fit.run", json!({ "sheet_id": "nl-mv", "source_sheet_id": "mv-bio", "kind": "4pl" })),
            ("fit.run", json!({ "sheet_id": "lin-mv", "source_sheet_id": "mv-cont", "kind": "linear" })),
            ("stats.run", json!({ "sheet_id": "pca-mv", "source_sheet_id": "mv-cont", "kind": "pca" })),
            ("stats.run", json!({ "sheet_id": "km-mv", "source_sheet_id": "mv-km", "kind": "kaplan_meier" })),
        ],
    );

    assert!(
        matches!(rs(&session, "tg-mv").analysis.kind, AnalysisKind::TwoGroupComparison(ref tg) if tg.mv_group_column == Some(0) && tg.mv_response_column == Some(2))
    );
    assert!(
        matches!(rs(&session, "nl-mv").analysis.kind, AnalysisKind::NonlinearRegression(ref nl) if nl.mv_group_columns == vec![0] && nl.mv_x_columns == vec![1] && nl.mv_y_column == Some(2))
    );
    assert_eq!(rs(&session, "lin-mv").analysis.selected_data_sets, vec![0, 2]);
    assert_eq!(rs(&session, "pca-mv").analysis.selected_data_sets, vec![0, 1, 2]);
    assert!(matches!(
        rs(&session, "km-mv").analysis.kind,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            mv_time_column: Some(1),
            mv_status_column: Some(2),
            mv_group_column: Some(0),
            ..
        })
    ));

    let (tg_before, nl_before, km_before) =
        (rs(&session, "tg-mv").scalars.clone(), rs(&session, "nl-mv").scalars.clone(), rs(&session, "km-mv").scalars.clone());
    let (lin_before, pca_before) = (scalar(&session, "lin-mv", "Slope"), rs(&session, "pca-mv").tabs[0].row_labels.len());
    assert!(!tg_before.is_empty());

    run_cmds(
        &mut session,
        &[
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-bio", "title": "Spike", "formula": "= [Resp] + 10000" })),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-cont", "title": "Spike", "formula": "= [C] + 10000" })),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-km", "title": "StatusAlt", "formula": "= 1 - [EventStatus]" })),
            (
                "analysis.run",
                json!({ "sheet_id": "pca-explicit-empty", "source_sheet_id": "mv-all-cont", "selected_data_sets": [], "kind": { "Multivariate": { "Pca": { "standardize": true, "parallel_analysis_seed": 7 } } } }),
            ),
            (
                "analysis.run",
                json!({ "sheet_id": "pca-mixed-explicit-empty", "source_sheet_id": "mv-bio", "selected_data_sets": [], "kind": { "Multivariate": { "Pca": { "standardize": true, "parallel_analysis_seed": 7 } } } }),
            ),
        ],
    );
    assert_eq!(
        (&rs(&session, "tg-mv").scalars, &rs(&session, "nl-mv").scalars, &rs(&session, "km-mv").scalars),
        (&tg_before, &nl_before, &km_before)
    );
    assert!((scalar(&session, "lin-mv", "Slope") - lin_before).abs() < 1e-12 && rs(&session, "pca-mv").tabs[0].row_labels.len() == pca_before);
    assert!(rs(&session, "pca-explicit-empty").analysis.selected_data_sets.is_empty());
    assert_eq!(rs(&session, "pca-mixed-explicit-empty").analysis.selected_data_sets, vec![1, 2]);
}

#[test]
fn audit_113_multiple_imputation_m_and_iters_bounds() {
    let v0: Vec<Cell> = (0..8).map(|r| Cell::number((r + 1) as f64)).collect();
    let v1: Vec<Cell> = (0..8).map(|r| if r == 2 { Cell::empty() } else { Cell::number((r * 2 + 1) as f64) }).collect();
    let v2: Vec<Cell> = (0..8).map(|r| Cell::number((r * 3 + 2) as f64)).collect();
    let mut session = session_with("Audit #113", vec![mv_sheet("mv-mi", "MV MI", &[], &[("A", None, &v0), ("B", None, &v1), ("C", None, &v2)])]);

    for bad_m in [0_usize, 1, 51] {
        assert_bad_input(&mut session, "stats.run", json!({ "source_sheet_id": "mv-mi", "kind": "mice", "imputations_m": bad_m }));
    }
    for bad_iters in [0_usize, 101] {
        assert_bad_input(&mut session, "stats.run", json!({ "source_sheet_id": "mv-mi", "kind": "mice", "max_iters": bad_iters }));
    }
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "mi-ok", "source_sheet_id": "mv-mi", "kind": "mice", "imputations_m": 2, "max_iters": 1 }));
    assert_bad_inputs(
        &mut session,
        &[
            ("analysis.updateParams", json!({ "sheet_id": "mi-ok", "imputations_m": 1 })),
            ("analysis.updateParams", json!({ "sheet_id": "mi-ok", "max_iters": 101 })),
        ],
    );
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "mi-ok", "imputations_m": 50, "max_iters": 100 }));
}

#[test]
fn audit_114_nonlinear_parameter_rules_unknown_and_duplicate_rejected() {
    let mut session = session_with("Audit #114", vec![make_single_xy("xy1")]);
    assert_bad_inputs(
        &mut session,
        &[
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "parameter_rules": [{ "name": "BogusParam", "scope": "shared" }] })),
            (
                "fit.run",
                json!({ "source_sheet_id": "xy1", "kind": "straight_line", "parameter_rules": [{ "name": "Slope", "scope": "constant", "value": 1.0 }, { "name": "slope", "scope": "local" }] }),
            ),
            (
                "fit.run",
                json!({ "source_sheet_id": "xy1", "equation_id": "custom_line", "custom_dsl": "y = a * x + b", "parameter_rules": [{ "name": "c", "scope": "constant", "value": 0.0 }] }),
            ),
        ],
    );
    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "fit-dsl", "source_sheet_id": "xy1", "equation_id": "custom_line", "custom_dsl": "y = a * x + b", "parameter_rules": [{ "name": "a", "scope": "constant", "value": 1.0 }] }),
    );
    assert_bad_input(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "fit-dsl", "parameter_rules": [{ "name": "missing", "scope": "local" }] }),
    );
}

#[test]
fn audit_120_xy_glm_raw_replicates_uses_all_replicates_and_rejects_presummarized() {
    let xy_rep = xy_rep_sheet(
        "xy-rep",
        "XY RawReplicates",
        "X",
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[
            ("Control", &[&[1.0, 1.2], &[2.0, 2.2], &[3.0, 3.2], &[4.0, 4.2], &[5.0, 5.2]]),
            ("Extreme", &[&[10.0, 9000.0], &[11.0, 9001.0], &[12.0, 9002.0], &[13.0, 9003.0], &[14.0, 9004.0]]),
        ],
    );
    let mut xy_sum = XyTable::new(SubcolumnMode::MeanSdN, 1, 5).unwrap();
    for r in 0..5 {
        xy_sum.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy_sum.data_sets[0].cells[r] = vec![Cell::number(10.0 + (r as f64)), Cell::number(1.0), Cell::number(3.0)];
    }
    let mut session = session_with("Audit #120", vec![xy_rep, DataSheet::new("xy-sum", "XY Summary", TableArchetype::Xy(xy_sum)).unwrap()]);

    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "glm-rep", "source_sheet_id": "xy-rep", "kind": "linear_glm", "predictor_columns": [0], "response_column": 1 }),
    );
    let res = rs(&session, "glm-rep");
    assert_eq!(res.status, ResultStatus::UpToDate);
    let ResultCell::Number { value: intercept, .. } = res.tabs[0].rows[0][0] else { panic!("expected numeric Intercept") };
    assert!(intercept > 4000.0);
    let resid_tab = res.tabs.iter().find(|t| t.row_labels.iter().any(|l| l.contains("rep 2"))).expect("rep 2 rows");
    assert_eq!(resid_tab.row_labels.len(), 10);
    assert_bad_input(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "xy-sum", "kind": "linear_glm", "predictor_columns": [0], "response_column": 0 }),
    );
}
