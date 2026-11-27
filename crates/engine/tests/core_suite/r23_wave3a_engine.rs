//! Integration tests for Wave 3A (`R23.1`–`R23.6`) engine features:
//! - End-to-end DAG dispatch & `analyzable_data` chaining for `RowStatistics`, `FractionOfTotal`, `CurveCalculus`, and §6.2 `plot_distribution`
//! - End-to-end dispatch & deterministic Methods Report prose for Wave 1–2 `R23` analyses
//! - `table.unlinkCalculatedVariable` value freezing, DAG invalidation, undo/redo, and error handling
//! - L5 Chained Monte Carlo (`Hits`/`Not Hits`/`All`, `exclude_flagged_from_summary`, determinism, cancellation, bounds, `analyzable_data`)
//! - Atomic `UserEquationLibrary` (`list`/`get`/`save`/`delete`/`cloneBuiltinEquation`, `[Constraints]`/`[Transforms]`, `--workdir` confinement, `fit.run` resolution)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::AtomicBool;

use curvebook_doc::{Cell, CellCoord, ErrorCode, InfoSheet, ResultStatus, SheetId, TableArchetype, VariableRole, VariableSource};
use curvebook_engine::{Caller, ExternalPathPolicy, Session};
use serde_json::{Value, json};

use super::common::{
    TempDir, approx_eq, assert_err_code, cell_num, col_sheet, grouped_rep_sheet, mv_sheet, ok_cmd, rs, scalar, session_with, survival_sheet,
    xy_rep_sheet, xy_sheet,
};

#[test]
fn r23_row_statistics_fraction_curve_calculus_and_plot_distribution_expose_chainable_analyzable_data() {
    let mut session = session_with(
        "R23 Transform & Distribution Chaining",
        vec![
            xy_rep_sheet(
                "xy-quad",
                "Quadratic Replicates",
                "X",
                &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
                &[("Signal", &[&[0.0, 0.2, -0.2], &[0.9, 1.0, 1.1], &[3.8, 4.0, 4.2], &[8.9, 9.0, 9.1], &[15.8, 16.0, 16.2], &[24.9, 25.0, 25.1]])],
            ),
            col_sheet("col-frac", "Counts", &[("A", &[10.0, 20.0, 70.0]), ("B", &[25.0, 25.0, 50.0])]),
        ],
    );

    // 1. RowStatistics (mean_ci & median_percentiles) + downstream chaining.
    ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "res-row-ci",
            "source_sheet_id": "xy-quad",
            "kind": "row_statistics",
            "summary": "mean_ci"
        }),
    );
    assert_eq!(rs(&session, "res-row-ci").status, ResultStatus::UpToDate);
    assert!(rs(&session, "res-row-ci").analyzable_data.is_some(), "RowStatistics must publish analyzable_data");

    ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "res-row-pct",
            "source_sheet_id": "xy-quad",
            "kind": "row_statistics",
            "summary": "median_percentiles",
            "upper_percentile": 90.0
        }),
    );
    assert_eq!(rs(&session, "res-row-pct").status, ResultStatus::UpToDate);
    ok_cmd(&mut session, "transform.run", json!({ "sheet_id": "chain-row-auc", "source_sheet_id": "res-row-ci", "kind": "auc" }));
    assert_eq!(rs(&session, "chain-row-auc").status, ResultStatus::UpToDate);

    // 2. FractionOfTotal (Wilson & Clopper-Pearson CIs) + downstream chaining.
    ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "res-frac-wilson",
            "source_sheet_id": "col-frac",
            "kind": "fraction_of_total",
            "denominator": "column_total",
            "output_percent": true,
            "ci_method": "wilson"
        }),
    );
    assert_eq!(rs(&session, "res-frac-wilson").status, ResultStatus::UpToDate);
    assert!(rs(&session, "res-frac-wilson").analyzable_data.is_some(), "FractionOfTotal must publish analyzable_data");
    ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "res-frac-cp",
            "source_sheet_id": "col-frac",
            "kind": "fraction_of_total",
            "denominator": "all_three",
            "output_percent": false,
            "ci_method": "clopper_pearson"
        }),
    );
    assert_eq!(rs(&session, "res-frac-cp").status, ResultStatus::UpToDate);
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "chain-frac-desc", "source_sheet_id": "res-frac-wilson", "kind": "descriptive_stats" }));
    assert_eq!(rs(&session, "chain-frac-desc").status, ResultStatus::UpToDate);

    // 3. CurveCalculus (1st/2nd derivative, cumulative integral, Savitzky-Golay, moving avg, LOWESS) + chaining linear regression off 1st derivative of y = x^2 -> y' = 2x.
    ok_cmd(
        &mut session,
        "transform.run",
        json!({ "sheet_id": "res-d1", "source_sheet_id": "xy-quad", "kind": "curve_calculus", "operation": "first_derivative" }),
    );
    assert_eq!(rs(&session, "res-d1").status, ResultStatus::UpToDate);
    assert!(rs(&session, "res-d1").analyzable_data.is_some(), "CurveCalculus must publish analyzable_data");
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "chain-d1-lin", "source_sheet_id": "res-d1", "kind": "linear_regression" }));
    assert_eq!(rs(&session, "chain-d1-lin").status, ResultStatus::UpToDate);
    approx_eq(scalar(&session, "chain-d1-lin", "Slope"), 2.0, 0.15, "slope of d/dx(x^2) should be ~2");

    for (id, op) in [
        ("res-d2", json!("second_derivative")),
        ("res-int", json!({ "CumulativeIntegral": { "baseline": 0.0 } })),
        ("res-sg", json!({ "SmoothSavitzkyGolay": { "window": 5, "order": 2 } })),
        ("res-ma", json!({ "SmoothMovingAverage": { "window": 3 } })),
        ("res-lo", json!({ "SmoothLowess": { "span": 0.6 } })),
    ] {
        ok_cmd(&mut session, "transform.run", json!({ "sheet_id": id, "source_sheet_id": "xy-quad", "kind": "curve_calculus", "operation": op }));
        assert_eq!(rs(&session, id).status, ResultStatus::UpToDate, "CurveCalculus `{id}` failed");
        assert!(rs(&session, id).analyzable_data.is_some(), "CurveCalculus `{id}` missing analyzable_data");
    }

    // 4. §6.2 plot_distribution (Normal + tail shading, StudentT, Binomial) + downstream chaining.
    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "res-dist-norm",
            "source_sheet_id": "xy-quad",
            "kind": "nonlinear",
            "plot_distribution": {
                "family": "Normal",
                "parameters": [0.0, 1.0],
                "tail_shade": { "TwoTailed": { "lower": -1.96, "upper": 1.96 } },
                "cumulative": false
            }
        }),
    );
    assert_eq!(rs(&session, "res-dist-norm").status, ResultStatus::UpToDate);
    assert!(rs(&session, "res-dist-norm").analyzable_data.is_some(), "plot_distribution must publish analyzable_data");
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "chain-dist-desc", "source_sheet_id": "res-dist-norm", "kind": "descriptive_stats" }));
    assert_eq!(rs(&session, "chain-dist-desc").status, ResultStatus::UpToDate);

    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "res-dist-binom",
            "source_sheet_id": "xy-quad",
            "kind": "nonlinear",
            "plot_distribution": { "family": "Binomial", "parameters": [12.0, 0.4], "cumulative": true }
        }),
    );
    assert_eq!(rs(&session, "res-dist-binom").status, ResultStatus::UpToDate);
}

#[test]
fn r23_wave1_wave2_extensions_dispatch_and_generate_clean_methods_reports() {
    let mut session = session_with(
        "R23 Wave 1-2 Full Dispatch",
        vec![
            col_sheet(
                "col-pos",
                "Positive Biomarkers",
                &[
                    ("Ctrl", &[10.0, 12.0, 11.5, 13.0, 10.8, 12.4, 11.2, 12.9]),
                    ("Low", &[24.0, 28.0, 22.0, 31.0, 25.0, 27.0, 29.0, 26.0]),
                    ("High", &[60.0, 80.0, 55.0, 92.0, 70.0, 74.0, 68.0, 85.0]),
                ],
            ),
            grouped_rep_sheet(
                "grp-rm",
                "Two-Way RM",
                &["Baseline", "Week2", "Week4"],
                &[
                    ("Vehicle", &[&[10.0, 11.0, 12.0], &[11.0, 12.5, 13.0], &[12.0, 13.0, 14.0]]),
                    ("Drug", &[&[10.5, 11.5, 12.0], &[18.0, 19.5, 20.0], &[25.0, 26.0, 27.5]]),
                ],
            ),
            mv_sheet(
                "mv-multi",
                "Multivariate & GLM",
                &[],
                &[
                    (
                        "X1",
                        Some(VariableRole::Continuous),
                        &[
                            Cell::number(1.0),
                            Cell::number(1.2),
                            Cell::number(1.5),
                            Cell::number(2.0),
                            Cell::number(8.0),
                            Cell::number(8.5),
                            Cell::number(9.0),
                            Cell::number(9.5),
                        ],
                    ),
                    (
                        "X2",
                        Some(VariableRole::Continuous),
                        &[
                            Cell::number(2.0),
                            Cell::number(2.1),
                            Cell::number(1.9),
                            Cell::number(2.4),
                            Cell::number(10.0),
                            Cell::number(10.4),
                            Cell::number(9.8),
                            Cell::number(10.2),
                        ],
                    ),
                    (
                        "Count",
                        Some(VariableRole::Continuous),
                        &[
                            Cell::number(2.0),
                            Cell::number(3.0),
                            Cell::number(2.0),
                            Cell::number(4.0),
                            Cell::number(15.0),
                            Cell::number(18.0),
                            Cell::number(19.0),
                            Cell::number(22.0),
                        ],
                    ),
                ],
            ),
            survival_sheet(
                "surv-1",
                "Survival Cohorts",
                &["Ctrl", "Drug"],
                &[
                    (2.0, &[Some(1.0), None]),
                    (4.0, &[Some(1.0), Some(0.0)]),
                    (6.0, &[Some(0.0), Some(1.0)]),
                    (9.0, &[Some(1.0), None]),
                    (12.0, &[None, Some(1.0)]),
                    (15.0, &[None, Some(0.0)]),
                ],
            ),
            xy_sheet(
                "xy-two-phase",
                "Two-Phase Decay",
                "Time",
                &[0.0, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 12.0, 16.0],
                &[("Signal", &[100.0, 77.6, 62.6, 44.8, 30.1, 20.6, 13.7, 11.2, 10.4])],
            ),
        ],
    );
    // Add an InfoSheet with LogRankSurvival hooks.
    let mut info = InfoSheet::new_preseeded("info-pwr", "Trial Design").expect("info sheet");
    info.set_constant("Event Prob Group 1", curvebook_doc::ConstantValue::Number(0.60), None).expect("c1");
    info.set_constant("Event Prob Group 2", curvebook_doc::ConstantValue::Number(0.40), None).expect("c2");
    session.document_mut_for_test_or_noop(info);

    let cases: Vec<(&str, Value)> = vec![
        ("stats.run", json!({ "sheet_id": "r-osrt", "source_sheet_id": "col-pos", "kind": "one_sample_ratio_t", "hypothetical": 10.0 })),
        ("stats.run", json!({ "sheet_id": "r-lnwt", "source_sheet_id": "col-pos", "kind": "lognormal_welch_t", "selected_data_sets": [0, 1] })),
        ("stats.run", json!({ "sheet_id": "r-ln1", "source_sheet_id": "col-pos", "kind": "lognormal_one_way_anova" })),
        ("stats.run", json!({ "sheet_id": "r-lnw", "source_sheet_id": "col-pos", "kind": "lognormal_welch_anova" })),
        ("stats.run", json!({ "sheet_id": "r-lnrm", "source_sheet_id": "col-pos", "kind": "lognormal_rm_anova" })),
        (
            "stats.run",
            json!({ "sheet_id": "r-norm", "source_sheet_id": "col-pos", "kind": "normality", "normality_only": true, "emit_qq_tabs": true }),
        ),
        (
            "stats.run",
            json!({
                "sheet_id": "r-mrow-ln",
                "source_sheet_id": "grp-rm",
                "kind": "multiple_row_t_tests",
                "lognormal_mode": "lognormal"
            }),
        ),
        (
            "stats.run",
            json!({
                "sheet_id": "r-rm2",
                "source_sheet_id": "grp-rm",
                "kind": "two_way_anova",
                "repeated_measures": "row",
                "sphericity_correction": "huynh_feldt",
                "reml_zero_variance_policy": "bound_at_zero"
            }),
        ),
        (
            "stats.run",
            json!({
                "sheet_id": "r-km",
                "source_sheet_id": "surv-1",
                "kind": "kaplan_meier",
                "event_code": 1.0,
                "censored_code": 0.0,
                "treat_other_as": "Exclude"
            }),
        ),
        (
            "stats.run",
            json!({
                "sheet_id": "r-kmeans",
                "source_sheet_id": "mv-multi",
                "kind": "kmeans",
                "k_means_k": 2,
                "kmeans_algorithm": "hartigan_wong",
                "k_sweep_max": 4,
                "consensus_optimal_k": true
            }),
        ),
        (
            "stats.run",
            json!({
                "sheet_id": "r-pca",
                "source_sheet_id": "mv-multi",
                "kind": "pca",
                "retention_rule": { "CumulativeVariancePercent": 90.0 }
            }),
        ),
        (
            "stats.run",
            json!({
                "sheet_id": "r-glm",
                "source_sheet_id": "mv-multi",
                "kind": "poisson_glm",
                "response_column": 2,
                "predictor_columns": [0, 1],
                "dispersion_mode": "quasi_poisson",
                "ci_method": "profile_likelihood"
            }),
        ),
        (
            "stats.run",
            json!({
                "sheet_id": "r-pwr-surv",
                "source_sheet_id": "surv-1",
                "kind": "power",
                "family": "LogRankSurvival",
                "solve_target": "SampleSize",
                "alpha": 0.05,
                "target_power": 0.80,
                "effect": { "Standardized": 0.65 },
                "survival_method": "freedman",
                "info_sheet_id": "info-pwr"
            }),
        ),
        (
            "fit.run",
            json!({
                "sheet_id": "r-fit-rel",
                "source_sheet_id": "xy-two-phase",
                "equation_id": "two_phase_decay",
                "relational_constraints": [{ "lhs_param": "KFast", "op": "GreaterThan", "rhs_param": "KSlow", "margin": 0.1 }],
                "emit_extra_gof": true
            }),
        ),
    ];

    for (cmd_id, params) in &cases {
        let sid = params["sheet_id"].as_str().unwrap();
        ok_cmd(&mut session, cmd_id, params.clone());
        assert_eq!(rs(&session, sid).status, ResultStatus::UpToDate, "analysis `{sid}` failed");
    }

    // Verify deterministic Methods Report across PlainText, Markdown, and LaTeX without debug `{:?}` leaks.
    for sid in ["r-osrt", "r-lnwt", "r-ln1", "r-lnrm", "r-norm", "r-rm2", "r-kmeans", "r-glm", "r-pwr-surv", "r-fit-rel"] {
        for fmt in ["plain", "markdown", "latex"] {
            let out = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": sid, "format": fmt }));
            let text = out.payload["report"].as_str().unwrap_or("");
            assert!(text.len() > 40, "report too short for `{sid}` ({fmt}): {text}");
            assert!(!text.contains("MonteCarloSimulation(") && !text.contains("RowStatistics("), "debug leak in `{sid}` ({fmt})");
        }
    }
}

trait InfoSeedExt {
    fn document_mut_for_test_or_noop(&mut self, info: InfoSheet);
}

impl InfoSeedExt for Session {
    fn document_mut_for_test_or_noop(&mut self, info: InfoSheet) {
        let mut doc = self.document().clone();
        doc.add_info_sheet(info).expect("add info sheet");
        *self = Session::from_document(doc);
    }
}

#[test]
fn r23_unlink_calculated_variable_freezes_values_invalidates_dag_and_undo_redoes() {
    let mut session = session_with(
        "Unlink Calculated Variable",
        vec![
            mv_sheet(
                "mv-calc",
                "Cohort",
                &[],
                &[
                    ("Score", Some(VariableRole::Continuous), &[Cell::number(10.0), Cell::number(20.0), Cell::number(30.0)]),
                    ("Doubled", Some(VariableRole::Continuous), &[]),
                ],
            ),
            col_sheet("col-nonmv", "Column", &[("A", &[1.0, 2.0])]),
        ],
    );

    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-calc", "col": 1, "formula": "= [Score] * 2" }));
    ok_cmd(&mut session, "table.excludeCells", json!({ "sheet_id": "mv-calc", "cells": [{ "row": 1, "col": 1 }], "excluded": true }));
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "mv-calc", "kind": "descriptive_stats" }));
    approx_eq(cell_num(&session, "mv-calc", 0, 1, 0).unwrap(), 20.0, 1e-12, "calculated row 0");
    approx_eq(cell_num(&session, "mv-calc", 2, 1, 0).unwrap(), 60.0, 1e-12, "calculated row 2");

    // Unlink column 1 -> values frozen into VariableSource::Entered, exclusion preserved.
    let unlink_out = ok_cmd(&mut session, "table.unlinkCalculatedVariable", json!({ "sheet_id": "mv-calc", "col": 1 }));
    assert!(unlink_out.mutated);
    assert_eq!(unlink_out.payload["unlinked"], json!(true));
    assert_eq!(unlink_out.payload["previous_formula"], json!("[Score] * 2"));

    let check_source = |s: &Session| -> VariableSource {
        match &s.document().data_sheet(&SheetId::new("mv-calc")).unwrap().archetype {
            TableArchetype::MultipleVariables(mv) => mv.variables[1].source.clone(),
            _ => panic!("expected mv"),
        }
    };
    assert_eq!(check_source(&session), VariableSource::Entered);
    assert!(session.document().data_sheet(&SheetId::new("mv-calc")).unwrap().cell(CellCoord::new(1, 1, 0)).unwrap().excluded);

    // Mutating upstream `Score` must NOT alter unlinked `Doubled`.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "mv-calc", "row": 0, "col": 0, "value": 100.0 }));
    approx_eq(cell_num(&session, "mv-calc", 0, 1, 0).unwrap(), 20.0, 1e-12, "unlinked cell must stay frozen");

    // Undo cell edit + undo unlink -> live formula restored!
    ok_cmd(&mut session, "edit.undo", json!({}));
    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!(matches!(check_source(&session), VariableSource::Calculated { .. }));

    // Redo unlink -> frozen again; then undo unlink and verify live formula resumes on upstream edit.
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(check_source(&session), VariableSource::Entered);
    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!(matches!(check_source(&session), VariableSource::Calculated { .. }));
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "mv-calc", "row": 0, "col": 0, "value": 50.0 }));
    approx_eq(cell_num(&session, "mv-calc", 0, 1, 0).unwrap(), 100.0, 1e-12, "live formula resumed after undo");

    // Re-unlink and verify idempotency on already-entered column (mutated == false).
    ok_cmd(&mut session, "table.unlinkCalculatedVariable", json!({ "sheet_id": "mv-calc", "col": 1 }));
    let noop = ok_cmd(&mut session, "table.unlinkCalculatedVariable", json!({ "sheet_id": "mv-calc", "col": 1 }));
    assert!(!noop.mutated);
    assert_eq!(noop.payload["unlinked"], json!(false));

    // Error cases: non-MV table, frozen sheet, out-of-bounds col.
    assert_err_code(&mut session, "table.unlinkCalculatedVariable", json!({ "sheet_id": "col-nonmv", "col": 0 }), ErrorCode::BadInput);
    assert_err_code(&mut session, "table.unlinkCalculatedVariable", json!({ "sheet_id": "mv-calc", "col": 9 }), ErrorCode::BadInput);
    ok_cmd(&mut session, "sheet.freeze", json!({ "sheet_id": "mv-calc" }));
    assert_err_code(&mut session, "table.unlinkCalculatedVariable", json!({ "sheet_id": "mv-calc", "col": 1 }), ErrorCode::FrozenSheet);
}

#[test]
fn r23_chained_monte_carlo_hits_not_hits_all_flagged_exclusion_and_bounds() {
    let mut session = session_with(
        "Chained Monte Carlo",
        vec![
            xy_sheet("xy-seed", "XY Seed", "X", &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], &[("Y", &[100.0, 60.0, 36.0, 22.0, 13.0, 8.0])]),
            col_sheet("col-seed", "Col Seed", &[("A", &[10.0, 11.0, 12.0, 13.0]), ("B", &[20.0, 21.0, 22.0, 23.0])]),
        ],
    );

    // 1. Simulate XY (noisy) -> Nonlinear fit with tight FitFlagRules { exclude_flagged_from_summary: true } -> Monte Carlo.
    ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "sim-xy",
            "source_sheet_id": "xy-seed",
            "kind": "simulate",
            "seed": 7,
            "row_count": 7,
            "replicates": 1,
            "x_min": 0.0,
            "x_max": 6.0,
            "equation_id": "one_phase_decay",
            "parameters": { "Y0": 100.0, "Plateau": 10.0, "K": 0.5 },
            "noise": { "Gaussian": { "sd": 8.0 } }
        }),
    );
    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-decay",
            "source_sheet_id": "sim-xy",
            "equation_id": "one_phase_decay",
            "fit_flag_rules": {
                "min_r_squared": 0.99,
                "exclude_flagged_from_summary": true
            }
        }),
    );
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "mc-xy-1",
            "source_sheet_id": "sim-xy",
            "kind": "monte_carlo",
            "seed": 12345,
            "iterations": 25,
            "downstream_result_sheet_id": "fit-decay",
            "tracked_scalars": [
                { "label": "RateK", "parameter_key": "K", "data_set_index": 0 },
                { "label": "Y0", "parameter_key": "Y0", "data_set_index": 0 }
            ],
            "hit_rules": [
                { "scalar_index": 0, "op": "Between", "threshold": 0.35, "upper_threshold": 0.65 }
            ],
            "exclude_flagged_fits": true
        }),
    );

    let mc1 = rs(&session, "mc-xy-1");
    assert_eq!(mc1.status, ResultStatus::UpToDate);
    let tab_ids: Vec<&str> = mc1.tabs.iter().map(|t| t.id.as_str()).collect();
    for required in [
        "mc-overview",
        "mc-summary-all",
        "mc-summary-hits",
        "mc-summary-not-hits",
        "mc-iterations-all",
        "mc-iterations-hits",
        "mc-iterations-not-hits",
    ] {
        assert!(tab_ids.contains(&required), "missing Monte Carlo tab `{required}` in {tab_ids:?}");
    }
    let retained = scalar(&session, "mc-xy-1", "RetainedIterations");
    let excluded = scalar(&session, "mc-xy-1", "ExcludedIterations");
    let hits = scalar(&session, "mc-xy-1", "Hits");
    let not_hits = scalar(&session, "mc-xy-1", "NotHits");
    assert!(excluded > 0.0, "tight min_r_squared=0.99 with Gaussian sd=8 must exclude at least one flagged fit");
    approx_eq(hits + not_hits, retained, 1e-9, "Hits + NotHits == Retained");
    assert!(mc1.analyzable_data.is_some(), "Monte Carlo must expose analyzable_data");

    // Chain descriptive_stats off Monte Carlo's analyzable_data.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "mc-desc", "source_sheet_id": "mc-xy-1", "kind": "descriptive_stats" }));
    assert_eq!(rs(&session, "mc-desc").status, ResultStatus::UpToDate);

    // Determinism check: identical seed reproduces exact same HitRate & RetainedIterations.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "mc-xy-dup",
            "source_sheet_id": "sim-xy",
            "kind": "monte_carlo",
            "seed": 12345,
            "iterations": 25,
            "downstream_result_sheet_id": "fit-decay",
            "tracked_scalars": [
                { "label": "RateK", "parameter_key": "K", "data_set_index": 0 },
                { "label": "Y0", "parameter_key": "Y0", "data_set_index": 0 }
            ],
            "hit_rules": [
                { "scalar_index": 0, "op": "Between", "threshold": 0.35, "upper_threshold": 0.65 }
            ],
            "exclude_flagged_fits": true
        }),
    );
    approx_eq(scalar(&session, "mc-xy-dup", "RetainedIterations"), retained, 1e-12, "deterministic retained count");
    approx_eq(scalar(&session, "mc-xy-dup", "HitRate"), scalar(&session, "mc-xy-1", "HitRate"), 1e-12, "deterministic hit rate");

    // 2. Simulate Contingency 2x2 -> Contingency chi-square -> Monte Carlo.
    ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "sim-2x2",
            "source_sheet_id": "col-seed",
            "kind": "simulate",
            "seed": 11,
            "simulation_mode": {
                "Contingency2x2": {
                    "design": "FixedRowTotals",
                    "cell_params": [0.7, 0.3, 0.2, 0.8],
                    "fixed_totals": [30, 30]
                }
            }
        }),
    );
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "stat-2x2", "source_sheet_id": "sim-2x2", "kind": "contingency" }));
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "mc-2x2",
            "source_sheet_id": "sim-2x2",
            "kind": "monte_carlo",
            "seed": 505,
            "iterations": 15,
            "downstream_result_sheet_id": "stat-2x2",
            "tracked_scalars": [{ "label": "ChiSqP", "parameter_key": "Chi-square P value" }],
            "hit_rules": [{ "scalar_index": 0, "op": "LessThan", "threshold": 0.05 }]
        }),
    );
    assert_eq!(rs(&session, "mc-2x2").status, ResultStatus::UpToDate);
    assert_eq!(scalar(&session, "mc-2x2", "Iterations"), 15.0);
    assert_eq!(scalar(&session, "mc-2x2", "RetainedIterations"), 15.0);

    // 3. Direct DataSheet bootstrap resampling -> One-Way ANOVA -> Monte Carlo.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "stat-boot", "source_sheet_id": "col-seed", "kind": "unpaired_t" }));
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "mc-boot",
            "source_sheet_id": "col-seed",
            "kind": "monte_carlo",
            "seed": 314,
            "iterations": 10,
            "downstream_result_sheet_id": "stat-boot",
            "tracked_scalars": [{ "label": "TStat", "parameter_key": "t" }]
        }),
    );
    assert_eq!(rs(&session, "mc-boot").status, ResultStatus::UpToDate);
    assert_eq!(scalar(&session, "mc-boot", "RetainedIterations"), 10.0);

    // 4. Bounds & cancellation checks.
    assert_err_code(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "mc-zero", "source_sheet_id": "col-seed", "kind": "monte_carlo", "iterations": 0 }),
        ErrorCode::BadInput,
    );
    assert_err_code(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "mc-huge", "source_sheet_id": "col-seed", "kind": "monte_carlo", "iterations": 100_001 }),
        ErrorCode::BadInput,
    );
    let cancel_flag = AtomicBool::new(true);
    let cancel_err = session
        .execute_with_caller(
            "stats.run",
            json!({
                "sheet_id": "mc-cancel",
                "source_sheet_id": "col-seed",
                "kind": "monte_carlo",
                "iterations": 20,
                "downstream_result_sheet_id": "stat-boot",
                "tracked_scalars": [{ "label": "TStat", "parameter_key": "t" }]
            }),
            Caller::Cli,
            &cancel_flag,
        )
        .expect_err("pre-cancelled Monte Carlo must return Cancelled");
    assert_eq!(cancel_err.code, ErrorCode::Cancelled);
}

#[test]
fn r23_atomic_user_equation_library_save_list_get_delete_clone_and_fit_run() {
    let dir = TempDir::new("r23-user-eq");
    let mut session = session_with(
        "User Equation Library",
        vec![xy_sheet(
            "xy-enz",
            "Enzyme Velocity",
            "Substrate",
            &[0.5, 1.0, 2.0, 5.0, 10.0, 25.0],
            &[("Rate", &[9.091, 16.667, 28.571, 50.0, 66.667, 83.333])],
        )],
    );
    session.set_path_policy(ExternalPathPolicy::workdir(dir.path()).expect("workdir policy"));

    // Empty initially.
    let list0 = ok_cmd(&mut session, "fit.listUserEquations", json!({}));
    assert_eq!(list0.payload["count"], json!(0));

    // Save custom equation with [InitialValues], [Constraints], and [Transforms].
    let saved = ok_cmd(
        &mut session,
        "fit.saveUserEquation",
        json!({
            "id": "custom_two_exp",
            "name": "Custom Two-Phase Decay",
            "category": "Exponential",
            "description": "Ordered two-phase exponential with derived half-lives",
            "dsl_source": "Y = A1 * exp(-KFast * X) + A2 * exp(-KSlow * X)\n[InitialValues]\nA1 = 50\nA2 = 50\nKFast = 1.0\nKSlow = 0.1\n[Constraints]\nKFast > KSlow\nKFast >= 0\nKSlow >= 0\n[Transforms]\nHalfLifeFast = ln(2) / KFast\nHalfLifeSlow = ln(2) / KSlow"
        }),
    );
    assert_eq!(saved.payload["equation"]["id"], json!("custom_two_exp"));
    assert_eq!(saved.payload["equation"]["relational_constraints_count"], json!(1));
    assert_eq!(saved.payload["equation"]["transforms_count"], json!(2));

    // Verify atomic JSON file on disk has schema_version == 1.
    let raw_store = std::fs::read_to_string(dir.child("user_equations.json")).expect("read user_equations.json");
    let store_json: Value = serde_json::from_str(&raw_store).expect("valid store JSON");
    assert_eq!(store_json["schema_version"], json!(1));

    // Duplicate save without overwrite rejected; overwrite: true succeeds.
    assert_err_code(
        &mut session,
        "fit.saveUserEquation",
        json!({ "id": "custom_two_exp", "name": "Dup", "dsl_source": "Y = A * X" }),
        ErrorCode::AlreadyExists,
    );
    ok_cmd(
        &mut session,
        "fit.saveUserEquation",
        json!({ "id": "custom_two_exp", "name": "Updated Two-Phase", "dsl_source": "Y = A * exp(-K * X)", "overwrite": true }),
    );

    // Invalid DSL & built-in ID collision rejected.
    assert_err_code(&mut session, "fit.saveUserEquation", json!({ "id": "bad_eq", "name": "Broken", "dsl_source": "Y = ((" }), ErrorCode::BadInput);
    assert_err_code(
        &mut session,
        "fit.saveUserEquation",
        json!({ "id": "michaelis_menten", "name": "Collide", "dsl_source": "Y = Vmax * X / (Km + X)" }),
        ErrorCode::Conflict,
    );

    // Clone built-in preview (`save: false`) does not persist.
    let preview =
        ok_cmd(&mut session, "fit.cloneBuiltinEquation", json!({ "builtin_id": "one_phase_decay", "new_id": "preview_only", "save": false }));
    assert_eq!(preview.payload["saved"], json!(false));
    assert_err_code(&mut session, "fit.getUserEquation", json!({ "id": "preview_only" }), ErrorCode::NotFound);

    // Clone built-in and save (`save: true`), then execute `fit.run` referencing only `equation_id`!
    let cloned = ok_cmd(
        &mut session,
        "fit.cloneBuiltinEquation",
        json!({ "builtin_id": "michaelis_menten", "new_id": "my_mm_cloned", "new_name": "Cloned MM", "save": true }),
    );
    assert_eq!(cloned.payload["saved"], json!(true));
    assert_eq!(cloned.payload["equation"]["cloned_from_builtin"], json!("michaelis_menten"));

    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "fit-cloned-mm", "source_sheet_id": "xy-enz", "equation_id": "my_mm_cloned" }));
    assert_eq!(rs(&session, "fit-cloned-mm").status, ResultStatus::UpToDate);
    approx_eq(scalar(&session, "fit-cloned-mm", "Vmax"), 100.0, 0.5, "cloned MM Vmax");
    approx_eq(scalar(&session, "fit-cloned-mm", "Km"), 5.0, 0.2, "cloned MM Km");

    // Path escape via `dir: "../outside"` rejected with PathEscape.
    assert_err_code(&mut session, "fit.listUserEquations", json!({ "dir": "../outside" }), ErrorCode::PathEscape);

    // Regression (#51 item 5): `analysis.updateParams` with `equation_id` (without `kind` or
    // `custom_dsl`) resolves saved user equations and clears stale `custom_dsl` when switching back
    // to a built-in equation (or an unknown equation).
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "fit-switch", "source_sheet_id": "xy-enz", "equation_id": "straight_line" }));
    assert_eq!(rs(&session, "fit-switch").status, ResultStatus::UpToDate);
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "fit-switch", "equation_id": "my_mm_cloned" }));
    assert_eq!(rs(&session, "fit-switch").status, ResultStatus::UpToDate);
    approx_eq(scalar(&session, "fit-switch", "Vmax"), 100.0, 0.5, "updateParams switched to user eq Vmax");
    approx_eq(scalar(&session, "fit-switch", "Km"), 5.0, 0.2, "updateParams switched to user eq Km");

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "fit-switch", "equation_id": "straight_line" }));
    assert_eq!(rs(&session, "fit-switch").status, ResultStatus::UpToDate);
    match &rs(&session, "fit-switch").analysis.kind {
        curvebook_doc::AnalysisKind::NonlinearRegression(nl) => {
            assert_eq!(nl.equation_id, "straight_line");
            assert!(nl.custom_dsl.is_none(), "switching back to built-in must clear stale custom_dsl");
        }
        other => panic!("expected NonlinearRegression, got {other:?}"),
    }
    let bad_eq = assert_err_code(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "fit-switch", "equation_id": "nonexistent_user_eq_999" }),
        ErrorCode::SolverFailed,
    );
    assert!(bad_eq.message().contains("nonexistent_user_eq_999"), "unexpected error: {bad_eq}");

    // Delete & verify count drops.
    let del = ok_cmd(&mut session, "fit.deleteUserEquation", json!({ "id": "custom_two_exp" }));
    assert_eq!(del.payload["deleted"], json!(true));
    let list_final = ok_cmd(&mut session, "fit.listUserEquations", json!({}));
    assert_eq!(list_final.payload["count"], json!(1));
}

#[test]
fn r23_monte_carlo_auto_tracks_downstream_scalars_and_rejects_mixed_independent_targets() {
    let mut session = session_with(
        "MC Regression Checks",
        vec![
            xy_sheet("xy-src", "XY Src", "X", &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], &[("Y", &[100.0, 60.0, 36.0, 22.0, 13.0, 8.0])]),
            col_sheet("col-indep", "Independent Column", &[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])]),
        ],
    );

    ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "sim-xy",
            "source_sheet_id": "xy-src",
            "kind": "simulate",
            "seed": 33,
            "row_count": 7,
            "replicates": 1,
            "x_min": 0.0,
            "x_max": 6.0,
            "equation_id": "one_phase_decay",
            "parameters": { "Y0": 100.0, "Plateau": 10.0, "K": 0.5 },
            "noise": { "Gaussian": { "sd": 1.5 } }
        }),
    );
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "fit-dep", "source_sheet_id": "sim-xy", "equation_id": "one_phase_decay" }));
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "stat-indep", "source_sheet_id": "col-indep", "kind": "unpaired_t" }));

    // Regression (#51 item 3): `downstream_result_sheet_id` with empty `tracked_scalars`
    // auto-tracks up to 8 finite scalars from the downstream sheet rather than summarizing `sim_ds`.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "mc-auto-track",
            "source_sheet_id": "sim-xy",
            "kind": "monte_carlo",
            "seed": 777,
            "iterations": 10,
            "downstream_result_sheet_id": "fit-dep"
        }),
    );
    let mc_auto = rs(&session, "mc-auto-track");
    assert_eq!(mc_auto.status, ResultStatus::UpToDate);
    assert_eq!(scalar(&session, "mc-auto-track", "RetainedIterations"), 10.0);
    let summary_all = mc_auto.tabs.iter().find(|t| t.id == "mc-summary-all").expect("mc-summary-all tab");
    assert!(
        summary_all.row_labels.iter().any(|lbl| lbl == "K" || lbl == "Y0" || lbl == "Plateau"),
        "expected downstream fit parameter scalars in auto-tracked summary, got {:?}",
        summary_all.row_labels
    );
    assert!(
        !summary_all.row_labels.iter().any(|lbl| lbl == "Simulated Mean"),
        "must not fall back to Simulated Mean when downstream sheet publishes scalars"
    );

    // Regression (#51 item 4): mixed dependent `downstream_result_sheet_id` + non-dependent
    // `tracked_scalars[i].result_sheet_id` must fail explicitly with a domain error instead of
    // silently producing 100% MissingScalar iterations.
    let err = session
        .execute(
            "stats.run",
            json!({
                "sheet_id": "mc-mixed-bad",
                "source_sheet_id": "sim-xy",
                "kind": "monte_carlo",
                "seed": 888,
                "iterations": 5,
                "downstream_result_sheet_id": "fit-dep",
                "tracked_scalars": [
                    { "label": "RateK", "result_sheet_id": "fit-dep", "parameter_key": "K", "data_set_index": 0 },
                    { "label": "UnrelatedT", "result_sheet_id": "stat-indep", "parameter_key": "t" }
                ]
            }),
        )
        .expect_err("mixed dependent + independent target sheet must fail");
    assert!(
        err.message().contains("stat-indep") && err.message().contains("does not depend"),
        "expected explicit domain error naming `stat-indep`, got: {err}"
    );
}

fn decode_base64_for_test(input: &str) -> Vec<u8> {
    const fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let (chunks, remainder) = bytes.as_chunks::<4>();
    assert!(remainder.is_empty(), "invalid base64 length {}", bytes.len());
    let mut out = Vec::with_capacity(chunks.len() * 3);
    for chunk in chunks {
        let a = val(chunk[0]).expect("base64 char 0");
        let b = val(chunk[1]).expect("base64 char 1");
        out.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            let c = val(chunk[2]).expect("base64 char 2");
            out.push((b << 4) | (c >> 2));
            if chunk[3] != b'=' {
                let d = val(chunk[3]).expect("base64 char 3");
                out.push((c << 6) | d);
            }
        }
    }
    out
}

fn assert_deterministic_four_format_exports(session: &mut Session, graph_id: &str, label: &str) -> String {
    // 0. Preflight check must pass cleanly with zero issues.
    let preflight = ok_cmd(session, "graph.preflightCheck", json!({ "sheet_id": graph_id }));
    let issues = preflight.payload["issues"].as_array().expect("preflight issues array");
    assert!(issues.is_empty(), "{label} ({graph_id}): preflightCheck reported issues: {issues:?}");

    // 1. SVG export (two consecutive passes must be byte-identical).
    let svg1 = ok_cmd(session, "export.svg", json!({ "sheet_id": graph_id }));
    let svg2 = ok_cmd(session, "export.svg", json!({ "sheet_id": graph_id }));
    let svg_str = svg1.payload["svg"].as_str().expect("svg payload string").to_owned();
    assert_eq!(svg_str, svg2.payload["svg"].as_str().expect("svg2 payload string"), "{label}: SVG export not deterministic");
    assert!(svg_str.contains("<svg") && svg_str.contains("</svg>"), "{label}: invalid SVG markup");
    assert!(svg_str.len() > 256, "{label}: suspiciously small SVG ({} bytes)", svg_str.len());

    // 2. RGBA PNG export (valid PNG signature, non-empty, byte-identical).
    let png1 = ok_cmd(session, "export.png", json!({ "sheet_id": graph_id, "width_px": 320, "height_px": 220, "dpi": 150 }));
    let png2 = ok_cmd(session, "export.png", json!({ "sheet_id": graph_id, "width_px": 320, "height_px": 220, "dpi": 150 }));
    let png_b64 = png1.payload["base64"].as_str().expect("png base64");
    assert_eq!(png_b64, png2.payload["base64"].as_str().expect("png2 base64"), "{label}: PNG export not deterministic");
    let png_bytes = decode_base64_for_test(png_b64);
    assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{label}: invalid PNG magic header");
    assert!(png_bytes.len() > 500, "{label}: suspiciously small PNG ({} bytes)", png_bytes.len());

    // 3. Vector PDF-1.7 export (valid %PDF-1.7 header + %%EOF trailer, byte-identical).
    let pdf1 = ok_cmd(session, "export.pdf", json!({ "sheet_id": graph_id }));
    let pdf2 = ok_cmd(session, "export.pdf", json!({ "sheet_id": graph_id }));
    let pdf_b64 = pdf1.payload["base64"].as_str().expect("pdf base64");
    assert_eq!(pdf_b64, pdf2.payload["base64"].as_str().expect("pdf2 base64"), "{label}: PDF export not deterministic");
    assert_eq!(pdf1.payload["page_count"], json!(1), "{label}: expected 1 PDF page");
    let pdf_bytes = decode_base64_for_test(pdf_b64);
    assert!(pdf_bytes.starts_with(b"%PDF-1.7"), "{label}: expected %PDF-1.7 header");
    assert!(pdf_bytes.windows(5).any(|w| w == b"%%EOF"), "{label}: missing PDF %%EOF trailer");
    assert!(pdf_bytes.len() > 500, "{label}: suspiciously small PDF ({} bytes)", pdf_bytes.len());

    // 4. Windows EMF export (EMR_HEADER record type 1 + " EMF" signature at offset 40, byte-identical).
    let emf1 = ok_cmd(session, "graph.exportEmf", json!({ "sheet_id": graph_id }));
    let emf2 = ok_cmd(session, "graph.exportEmf", json!({ "sheet_id": graph_id }));
    assert_eq!(emf1.payload["header_magic"], json!("0x464D4520"), "{label}: unexpected EMF header_magic");
    let emf_b64 = emf1.payload["base64"].as_str().expect("emf base64");
    assert_eq!(emf_b64, emf2.payload["base64"].as_str().expect("emf2 base64"), "{label}: EMF export not deterministic");
    let emf_bytes = decode_base64_for_test(emf_b64);
    assert!(emf_bytes.len() >= 108, "{label}: EMF shorter than EMR_HEADER");
    assert_eq!(&emf_bytes[0..4], &1_u32.to_le_bytes(), "{label}: EMF must begin with EMR_HEADER (type 1)");
    assert_eq!(&emf_bytes[40..44], b" EMF", "{label}: EMF missing signature at offset 40");

    svg_str
}

#[test]
fn r23_wave3_diagnostics_overlays_and_distributions_export_png_svg_pdf17_emf_deterministically() {
    // 1. One-Way ANOVA column table for HomoscedasticityPlot + positive sample for LognormalQqPlot.
    let col_anova = col_sheet(
        "col-anova",
        "Dose Groups",
        &[("Low", &[5.1, 5.5, 4.9, 5.3, 5.0, 5.4]), ("Mid", &[14.8, 16.0, 13.5, 15.2, 17.1, 14.3]), ("High", &[30.2, 35.5, 27.1, 33.8, 41.0, 25.9])],
    );
    let col_pos = col_sheet("col-ln", "Positive Biomarker", &[("Conc", &[1.2, 2.5, 4.1, 6.8, 11.2, 18.5, 31.0, 54.0])]);

    // 2. Multiple Variables table for Linear & Logistic GLMs (ActualVsPredicted, ResidualVsRow,
    //    Homoscedasticity, ParameterCorrelationHeatmap, and Logistic ROC / Calibration).
    let nums = |xs: &[f64]| -> Vec<Cell> { xs.iter().copied().map(Cell::number).collect() };
    let glm_x1 = nums(&[0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5, 8.0]);
    let glm_x2 = nums(&[1.2, 0.8, 2.1, 1.5, 3.0, 2.2, 3.8, 2.9, 4.1, 3.6, 4.9, 4.2, 5.4, 4.8, 6.0, 5.5]);
    let glm_ylin = nums(&[3.1, 3.8, 5.7, 6.4, 8.2, 8.9, 11.1, 11.4, 13.5, 14.2, 16.4, 16.9, 18.8, 19.6, 21.7, 22.5]);
    let glm_ybin = nums(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
    let mv_glm = mv_sheet(
        "mv-glm",
        "GLM Cohort",
        &[],
        &[
            ("X1", Some(VariableRole::Continuous), &glm_x1),
            ("X2", Some(VariableRole::Continuous), &glm_x2),
            ("Y_lin", Some(VariableRole::Continuous), &glm_ylin),
            ("Y_bin", Some(VariableRole::Continuous), &glm_ybin),
        ],
    );

    // 3. Multiple Variables table for Consensus K-Means / GapStatisticPlot.
    let cl_d1 = nums(&[0.0, 0.2, -0.1, 0.1, -0.2, 0.05, 10.0, 10.2, 9.8, 10.1, 9.9, 10.15, 20.0, 20.2, 19.9, 20.1, 19.8, 20.05]);
    let cl_d2 = nums(&[0.1, -0.1, 0.2, 0.0, -0.1, 0.15, 10.1, 9.9, 10.2, 10.0, 9.8, 10.05, 0.1, -0.2, 0.2, 0.0, -0.1, 0.15]);
    let mv_clust = mv_sheet(
        "mv-clust",
        "Three Clusters",
        &[],
        &[("Dim1", Some(VariableRole::Continuous), &cl_d1), ("Dim2", Some(VariableRole::Continuous), &cl_d2)],
    );

    // 4. Multiple Variables table for Cox PH covariate profiles & Cox-Snell residuals.
    let mut mv_cox_tbl = curvebook_doc::MultipleVariablesTable::new(4, 14).expect("cox mv table");
    mv_cox_tbl.variables[0].title = String::from("Time");
    mv_cox_tbl.variables[1].title = String::from("Status");
    mv_cox_tbl.variables[2].title = String::from("Age");
    mv_cox_tbl.variables[3].title = String::from("Arm");
    mv_cox_tbl.variables[3].lock_role(VariableRole::Categorical);
    let cox_rows = [
        (2.0, 1.0, 48.0, "Ctrl"),
        (4.0, 1.0, 52.0, "Ctrl"),
        (6.0, 0.0, 55.0, "Ctrl"),
        (7.0, 1.0, 60.0, "Ctrl"),
        (9.0, 1.0, 64.0, "Ctrl"),
        (11.0, 1.0, 68.0, "Ctrl"),
        (14.0, 0.0, 71.0, "Ctrl"),
        (5.0, 1.0, 46.0, "Drug"),
        (8.0, 1.0, 51.0, "Drug"),
        (10.0, 0.0, 54.0, "Drug"),
        (13.0, 1.0, 59.0, "Drug"),
        (16.0, 1.0, 63.0, "Drug"),
        (19.0, 1.0, 67.0, "Drug"),
        (23.0, 0.0, 70.0, "Drug"),
    ];
    for (r, (t, st, age, arm)) in cox_rows.into_iter().enumerate() {
        mv_cox_tbl.variables[0].cells[r] = Cell::number(t);
        mv_cox_tbl.variables[1].cells[r] = Cell::number(st);
        mv_cox_tbl.variables[2].cells[r] = Cell::number(age);
        mv_cox_tbl.variables[3].cells[r] = Cell::text(arm);
    }
    let mv_cox = curvebook_doc::DataSheet::new(SheetId::new("mv-cox"), "Cox Trial", TableArchetype::MultipleVariables(mv_cox_tbl)).expect("cox ds");

    // 5. Minimal XY seed sheet for §6.2 theoretical distribution plotting.
    let xy_seed = xy_sheet("xy-seed", "Seed", "X", &[0.0, 1.0], &[("Y", &[0.0, 1.0])]);

    let mut session = session_with("R23 Wave 3 Export Smoke", vec![col_anova, col_pos, mv_glm, mv_clust, mv_cox, xy_seed]);

    // Execute real L2 stats & fit analyses.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-ow", "source_sheet_id": "col-anova", "kind": "one_way_anova" }));
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "res-lin-glm",
            "source_sheet_id": "mv-glm",
            "kind": "linear_glm",
            "response_column": 2,
            "predictor_columns": [0, 1],
            "emit_influence_diagnostics": true
        }),
    );
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "res-log-glm",
            "source_sheet_id": "mv-glm",
            "kind": "logistic_glm",
            "response_column": 3,
            "predictor_columns": [0, 1],
            "emit_influence_diagnostics": true
        }),
    );
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "res-gap",
            "source_sheet_id": "mv-clust",
            "kind": "kmeans",
            "k_sweep_max": 5,
            "consensus_optimal_k": true
        }),
    );
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "res-lnqq", "source_sheet_id": "col-ln", "kind": "descriptive_stats", "emit_qq_tabs": true }),
    );
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "res-cox",
            "source_sheet_id": "mv-cox",
            "kind": "cox_ph",
            "time_column": 0,
            "status_column": 1,
            "predictor_columns": [2, 3],
            "reference_levels": [[3, "Ctrl"]],
            "emit_cox_snell_residuals": true,
            "covariate_profiles": [
                { "label": "Young Drug", "numeric_values": [[2, 48.0]], "categorical_values": [[3, "Drug"]] },
                { "label": "Older Ctrl", "numeric_values": [[2, 68.0]], "categorical_values": [[3, "Ctrl"]] }
            ]
        }),
    );
    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "res-dist-norm",
            "source_sheet_id": "xy-seed",
            "equation_id": "plot_distribution",
            "plot_distribution": {
                "family": "Normal",
                "parameters": [0.0, 1.0],
                "cumulative": false,
                "tail_shade": { "TwoTailed": { "lower": -1.96, "upper": 1.96 } }
            }
        }),
    );
    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "res-dist-binom",
            "source_sheet_id": "xy-seed",
            "equation_id": "plot_distribution",
            "plot_distribution": {
                "family": "Binomial",
                "parameters": [10.0, 0.4],
                "cumulative": false,
                "tail_shade": { "Between": { "lower": 3.0, "upper": 6.0 } }
            }
        }),
    );

    // Create & export all 6 new DiagnosticPlotKind charts (also verifying `graph.setKind` diag.* option_ids).
    for (gid, src, kind, opt_id, expect_text) in [
        ("g-homo", "res-ow", "homoscedasticity", "diag.homoscedasticity", ""),
        ("g-avp-log", "res-log-glm", "actual_vs_predicted", "diag.actual_vs_predicted", "Calibration bins"),
        ("g-rvr-lin", "res-lin-glm", "residual_vs_row", "diag.residual_vs_row", ""),
        ("g-gap", "res-gap", "gap_statistic", "diag.gap_statistic", "k* ="),
        ("g-corr-log", "res-log-glm", "param_corr_heatmap", "diag.param_corr_heatmap", "1.00"),
        ("g-lnqq", "res-lnqq", "lognormal_qq", "diag.lognormal_qq", ""),
    ] {
        ok_cmd(&mut session, "graph.create", json!({ "sheet_id": gid, "data_sheet_id": src, "kind": kind }));
        // Verify `graph.setKind` resolves the matching `diag.*` option_id cleanly.
        ok_cmd(&mut session, "graph.setKind", json!({ "sheet_id": gid, "kind": "diagnostic", "option_id": opt_id }));
        let svg = assert_deterministic_four_format_exports(&mut session, gid, kind);
        if !expect_text.is_empty() {
            assert!(svg.contains(expect_text), "{kind}: expected `{expect_text}` in exported SVG");
        }
    }

    // Logistic ROC overlay (`RocCurve` from `res-log-glm`).
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-log-roc", "data_sheet_id": "res-log-glm", "kind": "roc_curve" }));
    let roc_svg = assert_deterministic_four_format_exports(&mut session, "g-log-roc", "logistic-roc");
    assert!(roc_svg.contains("AUC ="), "logistic ROC SVG missing AUC callout");

    // Cox PH covariate-profile survival overlay (`GraphKind::Survival` from `res-cox`) + Cox-Snell ResidualVsRowPlot.
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-cox-surv", "data_sheet_id": "res-cox", "kind": "survival" }));
    let cox_svg = assert_deterministic_four_format_exports(&mut session, "g-cox-surv", "cox-profile-survival");
    assert!(cox_svg.contains("Baseline"), "Cox survival SVG missing Baseline curve");
    assert!(cox_svg.contains("Young Drug"), "Cox survival SVG missing Young Drug profile");
    assert!(cox_svg.contains("Older Ctrl"), "Cox survival SVG missing Older Ctrl profile");

    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-cox-rvr", "data_sheet_id": "res-cox", "kind": "residual_vs_row" }));
    assert_deterministic_four_format_exports(&mut session, "g-cox-rvr", "cox-snell-residual-vs-row");

    // §6.2 Continuous tail-shaded Normal PDF (via direct analyzable_data AND regression overlay)
    // and discrete Binomial PMF (via direct analyzable_data AND regression overlay).
    for (gid, src, label) in
        [("g-dist-norm-dir", "res-dist-norm", "dist-normal-twotailed-direct"), ("g-dist-binom-dir", "res-dist-binom", "dist-binomial-pmf-direct")]
    {
        ok_cmd(&mut session, "graph.create", json!({ "sheet_id": gid, "data_sheet_id": src, "kind": "xy" }));
        assert_deterministic_four_format_exports(&mut session, gid, label);
    }
    for (gid, res_id, label) in
        [("g-dist-norm-ov", "res-dist-norm", "dist-normal-twotailed-overlay"), ("g-dist-binom-ov", "res-dist-binom", "dist-binomial-pmf-overlay")]
    {
        ok_cmd(&mut session, "graph.create", json!({ "sheet_id": gid, "data_sheet_id": "xy-seed", "kind": "xy" }));
        ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": gid, "series_index": 0, "show_raw_data": false }));
        ok_cmd(
            &mut session,
            "graph.setRegressionOverlay",
            json!({ "sheet_id": gid, "series_index": 0, "result_sheet_id": res_id, "shaded_ribbon": true }),
        );
        assert_deterministic_four_format_exports(&mut session, gid, label);
    }
}
