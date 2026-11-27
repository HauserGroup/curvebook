//! Exhaustive summary-mode matrix enforcement and D1 textbook reference oracles for `curvebook-fit`
//! across `Xy` and `Grouped` tables × all eight pre-summarized `SubcolumnMode` layouts.
//!
//! # References
//!
//! - Draper, N. R., & Smith, H. (1998). *Applied Regression Analysis* (3rd ed.). Wiley.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 17.
//! - Seber, G. A. F., & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Motulsky, H. J., & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear
//!   and Nonlinear Regression*. Oxford University Press. Ch. 16.
//! - Deming, W. E. (1943). *Statistical Adjustment of Data*. Wiley.
//! - Motulsky, H. J., & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear
//!   regression – a new method based on robust nonlinear regression and the false discovery rate.
//!   *BMC Bioinformatics* 7:123.

use curvebook_doc::{
    AnalysisDomainKind, AnalysisKind, AnalysisSpec, Cell, DataSheet, FitMethod, GroupedTable, LinearRegressionSpec, NonlinearFitSpec,
    OutlierHandling, ResultStatus, SUMMARY_SUBCOLUMN_MODES, SubcolumnMode, SummaryModeProcedure, SummaryModeRefusalReason, SummaryModeSupport,
    TableArchetype, TableKind, WeightingScheme, XyTable, summary_mode_matrix_rows,
};
use curvebook_fit::{FitError, HookContext, execute_analysis};
use curvebook_math::sum::SummaryMoments;

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tol,
        "{label}: expected {expected:.15}, got {actual:.15} (diff = {:.3e}, tol = {tol:.1e})",
        (actual - expected).abs()
    );
}

fn summary_cells_for_mode(mode: SubcolumnMode, mean: f64, sd: f64, n: f64) -> Vec<Cell> {
    match mode {
        SubcolumnMode::MeanSdN => vec![Cell::number(mean), Cell::number(sd), Cell::number(n)],
        SubcolumnMode::MeanSemN => vec![Cell::number(mean), Cell::number(sd / n.sqrt()), Cell::number(n)],
        SubcolumnMode::MeanCvN => vec![Cell::number(mean), Cell::number((sd / mean.abs()) * 100.0), Cell::number(n)],
        SubcolumnMode::MeanSd => vec![Cell::number(mean), Cell::number(sd)],
        SubcolumnMode::MeanSem => vec![Cell::number(mean), Cell::number(sd / n.sqrt())],
        SubcolumnMode::MeanLowHigh => vec![Cell::number(mean), Cell::number(sd), Cell::number(1.5 * sd)],
        SubcolumnMode::UpperLowerLimits => vec![Cell::number(mean), Cell::number(mean - sd), Cell::number(mean + 1.5 * sd)],
        SubcolumnMode::MedianIqr => vec![Cell::number(mean), Cell::number(mean - 0.5 * sd), Cell::number(mean + 0.5 * sd)],
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => panic!("expected summary mode"),
    }
}

fn make_fit_sheet(table_kind: TableKind, mode: SubcolumnMode, xs: &[f64], ys: &[f64], sds: &[f64], ns: &[f64]) -> DataSheet {
    match table_kind {
        TableKind::Xy => {
            let mut xy = XyTable::new(mode, 1, xs.len()).unwrap();
            xy.data_sets[0].title = String::from("Curve");
            for r in 0..xs.len() {
                xy.x_cells[r] = vec![Cell::number(xs[r])];
                xy.data_sets[0].cells[r] = summary_cells_for_mode(mode, ys[r], sds[r], ns[r]);
            }
            DataSheet::new("xy-fit", "XY Fit", TableArchetype::Xy(xy)).unwrap()
        }
        TableKind::Grouped => {
            let mut grp = GroupedTable::new(mode, 1, xs.len()).unwrap();
            grp.data_sets[0].title = String::from("Curve");
            grp.row_titles = xs.iter().map(|x| format!("{x}")).collect();
            grp.row_numeric_spacing = Some(xs.iter().copied().map(Some).collect());
            for r in 0..xs.len() {
                grp.data_sets[0].cells[r] = summary_cells_for_mode(mode, ys[r], sds[r], ns[r]);
            }
            DataSheet::new("grp-fit", "Grouped Fit", TableArchetype::Grouped(grp)).unwrap()
        }
        other => panic!("unexpected fit table kind {other:?}"),
    }
}

fn representative_fit_kinds(proc_id: SummaryModeProcedure) -> Vec<AnalysisKind> {
    match proc_id {
        SummaryModeProcedure::LinearRegressionOls => vec![AnalysisKind::LinearRegression(LinearRegressionSpec::default())],
        SummaryModeProcedure::LinearRegressionDeming => {
            vec![AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() })]
        }
        SummaryModeProcedure::NonlinearRegressionStandard => vec![
            AnalysisKind::NonlinearRegression(NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("straight_line") }),
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                method: FitMethod::PoissonDeviance,
                weighting: WeightingScheme::None,
                profile_likelihood_ci: false,
                ..NonlinearFitSpec::new("straight_line")
            }),
        ],
        SummaryModeProcedure::NonlinearRegressionObservedSdWeighted => vec![AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            weighting: WeightingScheme::ObservedSd,
            profile_likelihood_ci: false,
            ..NonlinearFitSpec::new("straight_line")
        })],
        SummaryModeProcedure::NonlinearRegressionRobustOrOutlier => vec![
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                method: FitMethod::RobustCauchy,
                profile_likelihood_ci: false,
                ..NonlinearFitSpec::new("straight_line")
            }),
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
                profile_likelihood_ci: false,
                ..NonlinearFitSpec::new("straight_line")
            }),
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                method: FitMethod::LeastSquares,
                outlier_handling: OutlierHandling::Eliminate,
                profile_likelihood_ci: false,
                ..NonlinearFitSpec::new("straight_line")
            }),
        ],
        other => panic!("unexpected non-fit procedure {other:?}"),
    }
}

#[test]
fn exhaustive_fit_summary_mode_matrix_matches_runtime_verdicts() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [5.0, 7.0, 9.0, 11.0, 13.0];
    let sds = [0.5, 0.8, 0.6, 1.0, 0.7];
    let ns = [4.0, 4.0, 4.0, 4.0, 4.0];

    let mut supported_cells = 0_usize;
    let mut refused_cells = 0_usize;

    for row in summary_mode_matrix_rows().iter().filter(|r| r.analysis_kind_id.descriptor().domain == AnalysisDomainKind::Fit) {
        let kinds = representative_fit_kinds(row.procedure);
        for &table_kind in row.table_kinds {
            for mode in SUMMARY_SUBCOLUMN_MODES {
                let expected = row.support_for(mode).expect("every summary mode has a verdict");
                let sheet = make_fit_sheet(table_kind, mode, &xs, &ys, &sds, &ns);

                for kind in &kinds {
                    let spec = AnalysisSpec::new("fit-m", sheet.id.as_str(), kind.clone()).unwrap();
                    let outcome = execute_analysis(&spec, &sheet, &HookContext::default());
                    match expected {
                        SummaryModeSupport::Supported(_) => {
                            let res = outcome.unwrap_or_else(|e| {
                                panic!("expected Supported for {:?} on {table_kind:?}/{mode:?}, got error: {e:?}", row.procedure)
                            });
                            assert_eq!(res.status, ResultStatus::UpToDate);
                            assert!(res.validate().is_ok());
                            supported_cells += 1;
                        }
                        SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates) => {
                            assert!(
                                matches!(outcome, Err(FitError::RequiresRawReplicates)),
                                "expected RequiresRawReplicates for {:?} on {table_kind:?}/{mode:?}, got {outcome:?}",
                                row.procedure
                            );
                            refused_cells += 1;
                        }
                        SummaryModeSupport::Refused(reason) => {
                            assert!(
                                matches!(
                                    outcome,
                                    Err(FitError::UnsupportedSubcolumnMode { table_kind: actual_tk, mode: actual_mode, .. })
                                        if actual_tk == table_kind && actual_mode == mode
                                ),
                                "expected UnsupportedSubcolumnMode ({reason:?}) for {:?} on {table_kind:?}/{mode:?}, got {outcome:?}",
                                row.procedure
                            );
                            refused_cells += 1;
                        }
                    }
                }
            }
        }
    }

    assert!(supported_cells > 0 && refused_cells > 0);
}

#[test]
fn d1_oracle_linear_and_nonlinear_parametric_vs_raw_and_observed_sd_wls() {
    // 1. Exact raw-replicate equivalence for Model I OLS & Michaelis-Menten nonlinear regression
    //    across MeanSdN, MeanSemN, MeanCvN (Draper & Smith 1998; Motulsky & Christopoulos 2004 Ch. 16).
    let xs = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0];
    let raw_reps: [[f64; 3]; 6] =
        [[16.2, 17.1, 16.7], [28.1, 29.0, 28.6], [43.8, 44.9, 44.5], [66.0, 67.2, 66.8], [79.5, 80.6, 79.9], [88.2, 89.4, 89.0]];
    let mut raw_xy = XyTable::new(SubcolumnMode::RawReplicates(3), 1, xs.len()).unwrap();
    for (r, (&x, reps)) in xs.iter().zip(raw_reps.iter()).enumerate() {
        raw_xy.x_cells[r] = vec![Cell::number(x)];
        raw_xy.data_sets[0].cells[r] = reps.iter().map(|&v| Cell::number(v)).collect();
    }
    let raw_sheet = DataSheet::new("raw-mm", "Raw MM", TableArchetype::Xy(raw_xy)).unwrap();

    let raw_lin = execute_analysis(
        &AnalysisSpec::new("rl", "raw-mm", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap(),
        &raw_sheet,
        &HookContext::default(),
    )
    .unwrap();
    let raw_nl = execute_analysis(
        &AnalysisSpec::new(
            "rn",
            "raw-mm",
            AnalysisKind::NonlinearRegression(NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("michaelis_menten") }),
        )
        .unwrap(),
        &raw_sheet,
        &HookContext::default(),
    )
    .unwrap();

    let means: Vec<f64> = raw_reps.iter().map(|r| SummaryMoments::from_slice(r).unwrap().mean).collect();
    let sds: Vec<f64> = raw_reps.iter().map(|r| SummaryMoments::from_slice(r).unwrap().sample_sd().unwrap()).collect();
    let ns = vec![3.0; xs.len()];

    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let sum_sheet = make_fit_sheet(TableKind::Xy, mode, &xs, &means, &sds, &ns);
        let sum_lin = execute_analysis(
            &AnalysisSpec::new("sl", sum_sheet.id.as_str(), AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap(),
            &sum_sheet,
            &HookContext::default(),
        )
        .unwrap();
        for key in ["Slope", "YIntercept", "R2"] {
            assert_close(
                sum_lin.lookup_scalar(key, Some(0)).unwrap(),
                raw_lin.lookup_scalar(key, Some(0)).unwrap(),
                1e-11,
                &format!("Linear OLS eq {mode:?} {key}"),
            );
        }

        let sum_nl = execute_analysis(
            &AnalysisSpec::new(
                "sn",
                sum_sheet.id.as_str(),
                AnalysisKind::NonlinearRegression(NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("michaelis_menten") }),
            )
            .unwrap(),
            &sum_sheet,
            &HookContext::default(),
        )
        .unwrap();
        for key in ["Vmax", "Km"] {
            assert_close(
                sum_nl.lookup_scalar(key, Some(0)).unwrap(),
                raw_nl.lookup_scalar(key, Some(0)).unwrap(),
                1e-7,
                &format!("Nonlinear MM eq {mode:?} {key}"),
            );
        }
    }

    // 2. Closed-form WLS normal-equations oracle for WeightingScheme::ObservedSd (w_i = 1 / SD_i^2; Seber & Wild 1989).
    let wxs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let wys = [3.1, 5.4, 6.8, 9.6, 10.5];
    let wsds = [0.2, 0.5, 0.25, 1.0, 0.4];
    let wns = [4.0, 6.0, 5.0, 3.0, 8.0];
    // Textbook 1/SD^2 WLS (Seber & Wild 1989): minimizes sum_i (1 / SD_i^2) * (y_i - (a + b*x_i))^2.
    let mut s_w = 0.0;
    let mut s_wx = 0.0;
    let mut s_wy = 0.0;
    let mut s_wxx = 0.0;
    let mut s_wxy = 0.0;
    for i in 0..wxs.len() {
        let wi = 1.0 / (wsds[i] * wsds[i]);
        s_w += wi;
        s_wx += wi * wxs[i];
        s_wy += wi * wys[i];
        s_wxx += wi * wxs[i] * wxs[i];
        s_wxy += wi * wxs[i] * wys[i];
    }
    let det = s_w * s_wxx - s_wx * s_wx;
    let expected_intercept = (s_wxx * s_wy - s_wx * s_wxy) / det;
    let expected_slope = (s_w * s_wxy - s_wx * s_wy) / det;

    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let sheet = make_fit_sheet(TableKind::Xy, mode, &wxs, &wys, &wsds, &wns);
        let nl_spec =
            NonlinearFitSpec { weighting: WeightingScheme::ObservedSd, profile_likelihood_ci: false, ..NonlinearFitSpec::new("straight_line") };
        let res = execute_analysis(
            &AnalysisSpec::new("wls", sheet.id.as_str(), AnalysisKind::NonlinearRegression(nl_spec)).unwrap(),
            &sheet,
            &HookContext::default(),
        )
        .unwrap();
        assert_close(res.lookup_scalar("Slope", Some(0)).unwrap(), expected_slope, 1e-9, &format!("ObservedSd WLS {mode:?} Slope"));
        assert_close(res.lookup_scalar("YIntercept", Some(0)).unwrap(), expected_intercept, 1e-9, &format!("ObservedSd WLS {mode:?} YIntercept"));
    }

    // 3. MeanLowHigh, UpperLowerLimits, MedianIqr explicitly fit central location values (subcolumn 0 only).
    let exact_mm_ys: Vec<f64> = xs.iter().map(|&x| 100.0 * x / (2.5 + x)).collect();
    for mode in [SubcolumnMode::MeanLowHigh, SubcolumnMode::UpperLowerLimits, SubcolumnMode::MedianIqr] {
        let sheet = make_fit_sheet(TableKind::Xy, mode, &xs, &exact_mm_ys, &[1.0; 6], &[4.0; 6]);
        let nl = execute_analysis(
            &AnalysisSpec::new(
                "cent",
                sheet.id.as_str(),
                AnalysisKind::NonlinearRegression(NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("michaelis_menten") }),
            )
            .unwrap(),
            &sheet,
            &HookContext::default(),
        )
        .unwrap();
        assert_close(nl.lookup_scalar("Vmax", Some(0)).unwrap(), 100.0, 1e-6, &format!("Central MM {mode:?} Vmax"));
        assert_close(nl.lookup_scalar("Km", Some(0)).unwrap(), 2.5, 1e-6, &format!("Central MM {mode:?} Km"));
    }
}
