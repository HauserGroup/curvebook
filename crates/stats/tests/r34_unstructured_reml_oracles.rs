//! R3.4 D1 primary-literature oracle and edge-case tests for unstructured residual covariance
//! REML (`ResidualCovariance::Unstructured`, Pinheiro & Bates 1996; Kenward & Roger 1997;
//! Fai & Cornelius 1996) and factorial Dunnett control-column validation.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, DataSheet, FactorialAnovaSpec, GroupedTable, MultiplicityMethod, MultiplicitySpec,
    RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultTabKind, SheetId, SubcolumnMode, TableArchetype,
};
use lumencraft_math::linalg::{DenseMatrix, sym_eigen};
use lumencraft_stats::StatsError;
use lumencraft_stats::anova::{UnstructuredFallbackReason, fit_random_intercept_reml, fit_unstructured_rm_reml, run_factorial_anova};

fn assert_approx_eq(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
    assert!(actual.is_finite(), "{label}: expected finite {expected}, got {actual}");
    let diff = (actual - expected).abs();
    let scale = expected.abs().max(1e-300);
    assert!(
        diff <= abs_tol || diff / scale <= rel_tol,
        "{label}: actual={actual:.15}, expected={expected:.15}, abs_diff={diff:.3e} (abs_tol={abs_tol:.1e}, rel_tol={rel_tol:.1e})"
    );
}

fn sum_contrast_one_way_matrix(tps: &[usize], t_levels: usize) -> DenseMatrix {
    let mut raw = Vec::with_capacity(tps.len() * t_levels);
    for &tp in tps {
        raw.push(1.0);
        for col in 0..(t_levels - 1) {
            if tp == t_levels - 1 {
                raw.push(-1.0);
            } else if tp == col {
                raw.push(1.0);
            } else {
                raw.push(0.0);
            }
        }
    }
    DenseMatrix::from_row_major(tps.len(), t_levels, raw).unwrap()
}

fn extract_cov_grid(tab: &lumencraft_doc::ResultTab) -> Vec<Vec<f64>> {
    tab.rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    assert!(matches!(c, ResultCell::Number { .. }), "expected numeric covariance/correlation cell, got {c:?}");
                    match c {
                        ResultCell::Number { value, .. } => *value,
                        _ => f64::NAN,
                    }
                })
                .collect()
        })
        .collect()
}

// ============================================================================
// 1. D1 Oracles (R 4.5.2 nlme::gls + KR97 / Fai & Cornelius 1996 + Hotelling T2)
// ============================================================================

#[test]
fn r34_oracle_1_balanced_rm_matches_nlme_gls_and_exact_hotelling_t2_with_scale_invariance() {
    // D1 Provenance: R 4.5.2 nlme::gls(corSymm + varIdent, method="REML") + exact Hotelling T^2
    // identity on balanced C=10, t=4 heteroscedastic correlated repeated-measures table (2026-11-16).
    let rows: [[f64; 4]; 10] = [
        [7.8619, 9.5358, 12.6828, 18.6416],
        [9.6090, 15.6633, 15.1899, 19.2729],
        [9.9696, 10.9532, 12.2620, 12.5323],
        [8.0537, 10.1767, 13.6374, 20.0171],
        [10.1693, 12.4881, 14.9148, 15.2261],
        [9.7270, 11.3560, 16.1260, 12.6722],
        [8.6510, 9.8738, 16.7219, 21.2478],
        [10.0390, 10.2092, 15.1012, 15.8994],
        [9.8063, 10.9133, 14.3235, 9.7072],
        [10.5437, 11.8846, 11.0903, 9.0893],
    ];
    let mut c_ids = Vec::with_capacity(40);
    let mut t_ids = Vec::with_capacity(40);
    let mut y = Vec::with_capacity(40);
    for (s, subj) in rows.iter().enumerate() {
        for (t, &val) in subj.iter().enumerate() {
            c_ids.push(s);
            t_ids.push(t);
            y.push(val);
        }
    }
    let labels: Vec<String> = (1..=4).map(|k| format!("T{k}")).collect();
    let x = sum_contrast_one_way_matrix(&t_ids, 4);
    let fit = fit_unstructured_rm_reml(&x, &y, &c_ids, &t_ids, &labels, &[("Time", 1, 3)]).unwrap();

    assert!(fit.converged);
    assert_eq!(fit.fallback_reason, None);
    assert_approx_eq(fit.reml_log_lik, -39.351_648_260_674_99, 1e-4, 1e-4, "Oracle1 logLik");

    // Exact analytical Hotelling T^2 equivalence: F = 17.066630776420915, df1 = 3, df2 = C - t + 1 = 7.
    assert_approx_eq(fit.fixed_terms[0].f_statistic.unwrap(), 17.066_630_776_420_915, 1e-5, 1e-6, "Oracle1 Hotelling F_KR");
    assert_approx_eq(fit.satterthwaite_df[0], 9.0, 1e-5, 1e-5, "Oracle1 nu_Sat");
    assert_approx_eq(fit.kenward_roger_df[0], 7.0, 1e-5, 1e-5, "Oracle1 Hotelling nu_KR");
    assert_approx_eq(fit.fixed_terms[0].p_value.unwrap(), 0.001_337_46, 1e-6, 1e-4, "Oracle1 p");

    let exp_sigma: [[f64; 4]; 4] = [
        [0.852_117_699_295_783, 0.750_320_129_673_608, -0.160_870_213_620_055, -2.961_804_531_457_644],
        [0.750_320_129_673_608, 3.184_719_126_485_574, 0.312_370_613_381_618, -0.416_194_933_179_488],
        [-0.160_870_213_620_055, 0.312_370_613_381_618, 3.159_107_648_101_08, 3.401_672_459_015_9],
        [-2.961_804_531_457_644, -0.416_194_933_179_488, 3.401_672_459_015_9, 18.798_828_346_752_792],
    ];
    for i in 0..4 {
        for j in 0..4 {
            assert_approx_eq(fit.covariance_matrix[i][j], exp_sigma[i][j], 5e-4, 5e-4, &format!("Oracle1 Sigma[{i},{j}]"));
        }
    }

    // Scale + shift invariance: y -> 1e5 * y + 250 must leave F*, df, p, and correlations invariant,
    // and scale Sigma by 1e10.
    let y_scaled: Vec<f64> = y.iter().map(|&v| 1e5 * v + 250.0).collect();
    let fit_scaled = fit_unstructured_rm_reml(&x, &y_scaled, &c_ids, &t_ids, &labels, &[("Time", 1, 3)]).unwrap();
    assert_approx_eq(fit_scaled.fixed_terms[0].f_statistic.unwrap(), fit.fixed_terms[0].f_statistic.unwrap(), 1e-8, 1e-8, "Scale F_KR");
    assert_approx_eq(fit_scaled.kenward_roger_df[0], fit.kenward_roger_df[0], 1e-8, 1e-8, "Scale nu_KR");
    assert_approx_eq(fit_scaled.covariance_matrix[3][3], fit.covariance_matrix[3][3] * 1e10, 1e3, 1e-7, "Scale Sigma[3,3]");
    assert_approx_eq(fit_scaled.correlation_matrix[0][3], fit.correlation_matrix[0][3], 1e-7, 1e-7, "Scale Corr[0,3]");
}

#[test]
fn r34_oracle_2_missing_cell_heteroscedastic_rm_matches_nlme_gls_and_dominates_cs() {
    // D1 Provenance: R 4.5.2 nlme::gls + KR97 on C=12, t=4 unbalanced RM with 7 missing cells (2026-11-16).
    let obs: &[(usize, usize, f64)] = &[
        (0, 0, 6.371),
        (0, 1, 6.651),
        (0, 3, 9.574),
        (1, 0, 4.435),
        (1, 2, 9.777),
        (1, 3, 7.115),
        (2, 0, 5.363),
        (2, 1, 7.627),
        (2, 2, 10.596),
        (2, 3, 2.325),
        (3, 0, 5.633),
        (3, 1, 9.024),
        (3, 2, 8.210),
        (4, 1, 7.437),
        (4, 2, 12.021),
        (4, 3, 11.144),
        (5, 0, 4.894),
        (5, 1, 3.420),
        (5, 2, 6.838),
        (5, 3, 7.632),
        (6, 0, 6.512),
        (6, 1, 5.200),
        (6, 2, 9.971),
        (7, 0, 4.905),
        (7, 1, 9.395),
        (7, 2, 14.174),
        (7, 3, 9.209),
        (8, 0, 7.018),
        (8, 2, 13.745),
        (8, 3, 6.576),
        (9, 0, 4.937),
        (9, 1, 4.772),
        (9, 2, 7.882),
        (9, 3, 10.215),
        (10, 0, 6.305),
        (10, 1, 8.417),
        (10, 3, 7.816),
        (11, 0, 7.287),
        (11, 1, 11.380),
        (11, 2, 9.445),
        (11, 3, 11.357),
    ];
    let c_ids: Vec<usize> = obs.iter().map(|r| r.0).collect();
    let t_ids: Vec<usize> = obs.iter().map(|r| r.1).collect();
    let y: Vec<f64> = obs.iter().map(|r| r.2).collect();
    let labels: Vec<String> = (1..=4).map(|k| format!("T{k}")).collect();
    let x = sum_contrast_one_way_matrix(&t_ids, 4);

    let fit = fit_unstructured_rm_reml(&x, &y, &c_ids, &t_ids, &labels, &[("Time", 1, 3)]).unwrap();
    assert!(fit.converged);
    assert_eq!(fit.fallback_reason, None);
    assert_approx_eq(fit.reml_log_lik, -46.162_436_773_863_48, 5e-4, 1e-4, "Oracle2 logLik");
    assert_approx_eq(fit.fixed_terms[0].f_statistic.unwrap(), 11.868_177_295_447_34, 1e-3, 1e-4, "Oracle2 F_KR");
    assert_approx_eq(fit.satterthwaite_df[0], 9.503_088_829_482_67, 1e-3, 1e-4, "Oracle2 nu_Sat");
    assert_approx_eq(fit.kenward_roger_df[0], 7.809_874_394_312_24, 1e-3, 1e-4, "Oracle2 nu_KR");
    assert_approx_eq(fit.fixed_terms[0].p_value.unwrap(), 0.002_779_428_67, 5e-6, 1e-3, "Oracle2 p");

    let exp_sigma: [[f64; 4]; 4] = [
        [0.895_309_019_836_032, 1.317_487_304_435_461, 0.494_551_791_633_832, 0.555_686_169_360_805],
        [1.317_487_304_435_461, 6.536_437_810_748_605, 3.156_703_276_207_105, 0.411_170_708_095_448],
        [0.494_551_791_633_832, 3.156_703_276_207_105, 5.628_159_129_062_982, -0.481_384_394_718_286],
        [0.555_686_169_360_805, 0.411_170_708_095_448, -0.481_384_394_718_286, 7.089_771_301_295_769],
    ];
    let exp_corr: [[f64; 4]; 4] = [
        [1.0, 0.544_614_741_152_458, 0.220_314_010_160_736, 0.220_560_122_675_057],
        [0.544_614_741_152_458, 1.0, 0.520_451_195_168_550, 0.060_399_798_253_551],
        [0.220_314_010_160_736, 0.520_451_195_168_550, 1.0, -0.076_206_621_856_629],
        [0.220_560_122_675_057, 0.060_399_798_253_551, -0.076_206_621_856_629, 1.0],
    ];
    for i in 0..4 {
        for j in 0..4 {
            assert_approx_eq(fit.covariance_matrix[i][j], exp_sigma[i][j], 2e-3, 1e-3, &format!("Oracle2 Sigma[{i},{j}]"));
            assert_approx_eq(fit.correlation_matrix[i][j], exp_corr[i][j], 2e-3, 1e-3, &format!("Oracle2 Corr[{i},{j}]"));
        }
    }

    // Nested-model dominance: unstructured REML log-likelihood strictly beats CompoundSymmetry.
    let cs = fit_random_intercept_reml(&x, &y, &c_ids, &[("Time", 1, 3)]).unwrap();
    assert!(fit.reml_log_lik > cs.reml_log_lik, "unstructured REML logLik ({}) must exceed CS logLik ({})", fit.reml_log_lik, cs.reml_log_lik);
}

#[test]
fn r34_oracle_3_split_plot_column_matched_grouped_table_matches_nlme_gls() {
    // D1 Provenance: R 4.5.2 nlme::gls + KR97 on 2-row x 4-col split-plot GroupedTable
    // (10 subjects per treatment arm, ColumnFactorMatched across 4 timepoints, 7 missing cells).
    let obs: &[(usize, usize, usize, f64)] = &[
        (0, 0, 0, 10.171),
        (0, 1, 0, 13.426),
        (0, 3, 0, 17.120),
        (0, 0, 1, 10.884),
        (0, 1, 1, 15.643),
        (0, 2, 1, 16.751),
        (0, 3, 1, 20.484),
        (1, 0, 0, 10.070),
        (1, 2, 0, 14.463),
        (1, 3, 0, 15.590),
        (1, 0, 1, 10.855),
        (1, 1, 1, 14.308),
        (1, 2, 1, 15.984),
        (1, 3, 1, 21.195),
        (2, 0, 0, 9.710),
        (2, 1, 0, 12.086),
        (2, 2, 0, 12.644),
        (2, 3, 0, 12.735),
        (2, 0, 1, 10.598),
        (2, 2, 1, 17.078),
        (2, 3, 1, 17.842),
        (3, 0, 0, 9.309),
        (3, 1, 0, 12.388),
        (3, 2, 0, 13.784),
        (3, 3, 0, 16.505),
        (3, 0, 1, 10.892),
        (3, 1, 1, 14.090),
        (3, 2, 1, 17.046),
        (3, 3, 1, 21.430),
        (4, 0, 0, 9.709),
        (4, 1, 0, 10.177),
        (4, 2, 0, 16.678),
        (4, 3, 0, 17.488),
        (4, 1, 1, 15.533),
        (4, 2, 1, 19.722),
        (4, 3, 1, 23.565),
        (5, 0, 0, 9.403),
        (5, 1, 0, 13.275),
        (5, 2, 0, 13.249),
        (5, 3, 0, 16.850),
        (5, 1, 1, 15.501),
        (5, 2, 1, 20.713),
        (5, 3, 1, 24.633),
        (6, 0, 0, 10.600),
        (6, 1, 0, 12.199),
        (6, 2, 0, 15.613),
        (6, 3, 0, 15.496),
        (6, 0, 1, 8.493),
        (6, 1, 1, 13.399),
        (6, 2, 1, 17.897),
        (6, 3, 1, 22.537),
        (7, 0, 0, 7.567),
        (7, 1, 0, 7.726),
        (7, 2, 0, 10.380),
        (7, 3, 0, 12.727),
        (7, 0, 1, 10.500),
        (7, 2, 1, 17.632),
        (7, 3, 1, 22.337),
        (8, 0, 0, 9.685),
        (8, 1, 0, 11.566),
        (8, 2, 0, 13.797),
        (8, 3, 0, 18.048),
        (8, 0, 1, 9.104),
        (8, 1, 1, 13.709),
        (8, 2, 1, 19.417),
        (8, 3, 1, 26.140),
        (9, 0, 0, 10.399),
        (9, 1, 0, 12.358),
        (9, 3, 0, 14.740),
        (9, 0, 1, 10.717),
        (9, 1, 1, 15.023),
        (9, 2, 1, 17.997),
        (9, 3, 1, 22.298),
    ];
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(10), 4, 2).unwrap();
    grouped.row_factor_title = String::from("Group");
    grouped.column_factor_title = String::from("Timepoint");
    grouped.row_titles = vec![String::from("Control"), String::from("Treated")];
    for c in 0..4 {
        grouped.data_sets[c].title = format!("W{}", c + 1);
    }
    for &(s, c, r, y) in obs {
        grouped.data_sets[c].cells[r][s] = Cell::number(y);
    }

    let ds_id = SheetId::new("g-split-un");
    let sheet = DataSheet::new(ds_id.clone(), "SplitPlot Unstructured", TableArchetype::Grouped(grouped)).unwrap();
    let fac_spec = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        residual_covariance: ResidualCovariance::Unstructured,
        ..FactorialAnovaSpec::default()
    };
    let spec = AnalysisSpec::new("an-split-un", ds_id, AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap();
    let res = run_factorial_anova(&sheet, &spec, &fac_spec).unwrap();
    let scalar = |k: &str| res.scalars.iter().find(|s| s.key == k).map(|s| s.value).unwrap();

    assert_approx_eq(scalar("REML log-likelihood"), -58.855_282_374_891_84, 5e-4, 1e-4, "SplitPlot logLik");
    assert_approx_eq(scalar("Group: F"), 49.402_631_992_718_19, 1e-2, 1e-4, "SplitPlot F_Group");
    assert_approx_eq(scalar("Group: Satterthwaite df"), 17.946_724_786_416_87, 1e-2, 5e-4, "SplitPlot Sat_Group");
    assert_approx_eq(scalar("Group: Kenward-Roger df"), 17.946_724_786_416_87, 1e-2, 5e-4, "SplitPlot KR_Group");
    assert_approx_eq(scalar("Group: P"), 1.495_365_4e-6, 1e-7, 2e-3, "SplitPlot P_Group");

    assert_approx_eq(scalar("Timepoint: F"), 110.198_796_574_603_78, 2e-2, 1e-4, "SplitPlot F_Time");
    assert_approx_eq(scalar("Timepoint: Satterthwaite df"), 16.942_325_765_385_55, 1e-2, 5e-4, "SplitPlot Sat_Time");
    assert_approx_eq(scalar("Timepoint: Kenward-Roger df"), 14.805_710_844_222_21, 1e-2, 5e-4, "SplitPlot KR_Time");
    assert_approx_eq(scalar("Timepoint: P"), 2.363_291e-10, 1e-11, 5e-3, "SplitPlot P_Time");

    assert_approx_eq(scalar("Group × Timepoint: F"), 11.142_726_923_477_96, 5e-3, 2e-4, "SplitPlot F_Inter");
    assert_approx_eq(scalar("Group × Timepoint: Kenward-Roger df"), 14.805_710_844_222_2, 1e-2, 5e-4, "SplitPlot KR_Inter");
    assert_approx_eq(scalar("Group × Timepoint: P"), 0.000_439_08, 2e-6, 2e-3, "SplitPlot P_Inter");

    let cov_tab = res.tabs.iter().find(|t| t.id == "unstructured-reml-covariance").unwrap();
    let corr_tab = res.tabs.iter().find(|t| t.id == "unstructured-reml-correlation").unwrap();
    assert_eq!(cov_tab.kind, ResultTabKind::CovarianceMatrix);
    assert_eq!(corr_tab.kind, ResultTabKind::CovarianceMatrix);
    let cov_grid = extract_cov_grid(cov_tab);
    assert_eq!(cov_grid.len(), 4);
    assert_approx_eq(cov_grid[0][0], 0.762_368_282_347_766, 2e-3, 1e-3, "SplitPlot Sigma[0,0]");
    assert_approx_eq(cov_grid[2][3], 2.360_240_039_452_824, 2e-3, 1e-3, "SplitPlot Sigma[2,3]");
    assert_approx_eq(cov_grid[3][3], 4.347_783_375_225_633, 5e-3, 1e-3, "SplitPlot Sigma[3,3]");
}

#[test]
fn r34_oracle_4_both_factors_matched_2x3_6x6_unstructured_matches_nlme_gls() {
    // D1 Provenance: R 4.5.2 nlme::gls + KR97 on BothFactorsMatched 2x3 GroupedTable
    // (14 subjects, full 6x6 unstructured covariance, 9 missing observations).
    let obs: &[(usize, usize, usize, f64)] = &[
        (0, 0, 0, 7.034),
        (0, 1, 0, 11.573),
        (0, 2, 0, 14.216),
        (0, 1, 1, 16.038),
        (0, 2, 1, 19.509),
        (1, 0, 0, 8.222),
        (1, 1, 0, 10.990),
        (1, 0, 1, 8.423),
        (1, 1, 1, 12.663),
        (1, 2, 1, 17.032),
        (2, 0, 0, 8.868),
        (2, 1, 0, 10.872),
        (2, 2, 0, 15.615),
        (2, 0, 1, 8.718),
        (2, 2, 1, 19.966),
        (3, 0, 0, 6.123),
        (3, 1, 0, 9.059),
        (3, 2, 0, 12.362),
        (3, 0, 1, 6.961),
        (3, 1, 1, 11.742),
        (3, 2, 1, 17.050),
        (4, 1, 0, 10.251),
        (4, 2, 0, 12.603),
        (4, 0, 1, 7.597),
        (4, 1, 1, 14.295),
        (4, 2, 1, 22.086),
        (5, 0, 0, 8.405),
        (5, 1, 0, 13.860),
        (5, 2, 0, 14.557),
        (5, 1, 1, 17.447),
        (5, 2, 1, 20.801),
        (6, 0, 0, 7.540),
        (6, 1, 0, 10.918),
        (6, 2, 0, 11.508),
        (6, 0, 1, 7.745),
        (6, 1, 1, 13.331),
        (6, 2, 1, 14.999),
        (7, 0, 0, 7.563),
        (7, 1, 0, 10.242),
        (7, 2, 0, 11.894),
        (7, 0, 1, 7.775),
        (7, 1, 1, 12.527),
        (8, 0, 0, 7.548),
        (8, 1, 0, 10.290),
        (8, 2, 0, 10.397),
        (8, 0, 1, 6.201),
        (8, 1, 1, 12.830),
        (8, 2, 1, 16.790),
        (9, 0, 0, 7.288),
        (9, 1, 0, 11.150),
        (9, 2, 0, 12.040),
        (9, 0, 1, 7.811),
        (9, 2, 1, 18.316),
        (10, 0, 0, 7.618),
        (10, 1, 0, 10.046),
        (10, 2, 0, 13.116),
        (10, 0, 1, 7.533),
        (10, 1, 1, 12.023),
        (10, 2, 1, 16.636),
        (11, 0, 0, 7.201),
        (11, 1, 0, 9.008),
        (11, 2, 0, 12.377),
        (11, 1, 1, 15.046),
        (11, 2, 1, 17.786),
        (12, 0, 0, 7.379),
        (12, 1, 0, 11.322),
        (12, 2, 0, 16.306),
        (12, 0, 1, 9.373),
        (12, 1, 1, 16.168),
        (12, 2, 1, 18.091),
        (13, 0, 0, 8.052),
        (13, 1, 0, 9.900),
        (13, 0, 1, 8.792),
        (13, 1, 1, 14.031),
        (13, 2, 1, 18.186),
    ];
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(14), 3, 2).unwrap();
    grouped.row_factor_title = String::from("Drug");
    grouped.column_factor_title = String::from("Dose");
    grouped.row_titles = vec![String::from("Veh"), String::from("Act")];
    grouped.data_sets[0].title = String::from("Low");
    grouped.data_sets[1].title = String::from("Mid");
    grouped.data_sets[2].title = String::from("High");
    for &(s, c, r, y) in obs {
        grouped.data_sets[c].cells[r][s] = Cell::number(y);
    }

    let ds_id = SheetId::new("g-both-un");
    let sheet = DataSheet::new(ds_id.clone(), "BothMatched Unstructured", TableArchetype::Grouped(grouped)).unwrap();
    let fac_spec = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        residual_covariance: ResidualCovariance::Unstructured,
        ..FactorialAnovaSpec::default()
    };
    let spec = AnalysisSpec::new("an-both-un", ds_id, AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap();
    let res = run_factorial_anova(&sheet, &spec, &fac_spec).unwrap();
    let scalar = |k: &str| res.scalars.iter().find(|s| s.key == k).map(|s| s.value).unwrap();

    assert_approx_eq(scalar("REML log-likelihood"), -43.645_038_667_826_17, 1e-3, 1e-4, "BothMatched logLik");
    assert_approx_eq(scalar("Drug: F"), 140.500_151_804_572_8, 5e-2, 5e-4, "BothMatched F_Drug");
    assert_approx_eq(scalar("Drug: Kenward-Roger df"), 12.719_849_702_508_15, 1e-2, 5e-4, "BothMatched KR_Drug");
    assert_approx_eq(scalar("Dose: F"), 304.884_669_175_560_1, 1e-1, 5e-4, "BothMatched F_Dose");
    assert_approx_eq(scalar("Dose: Kenward-Roger df"), 11.711_779_481_914_91, 1e-2, 1e-3, "BothMatched KR_Dose");
    assert_approx_eq(scalar("Drug × Dose: F"), 32.334_661_362_381_35, 2e-2, 5e-4, "BothMatched F_Inter");
    assert_approx_eq(scalar("Drug × Dose: Kenward-Roger df"), 11.545_660_806_703_91, 1e-2, 1e-3, "BothMatched KR_Inter");

    let cov_tab = res.tabs.iter().find(|t| t.id == "unstructured-reml-covariance").unwrap();
    assert_eq!(cov_tab.column_headers.len(), 6);
    let cov_grid = extract_cov_grid(cov_tab);
    assert_approx_eq(cov_grid[0][0], 0.461_167_598_399_404, 2e-3, 2e-3, "BothMatched Sigma[0,0]");
    assert_approx_eq(cov_grid[2][2], 3.188_556_142_887_321, 5e-3, 2e-3, "BothMatched Sigma[2,2]");
    assert_approx_eq(cov_grid[5][5], 3.694_711_204_546_756, 5e-3, 2e-3, "BothMatched Sigma[5,5]");
}

#[test]
fn r34_oracle_5_more_timepoints_than_complete_subjects_converges_cleanly() {
    // D1 Provenance: R 4.5.2 nlme::gls + KR97 on t = 5 timepoints, C = 16 subjects, with ONLY
    // n_complete = 2 complete subjects (t > n_complete), demonstrating full-rank incomplete REML.
    let obs: &[(usize, usize, f64)] = &[
        (0, 0, 4.343),
        (0, 1, 7.112),
        (0, 2, 11.236),
        (0, 3, 10.456),
        (0, 4, 11.194),
        (1, 0, 3.721),
        (1, 1, 5.336),
        (1, 2, 7.002),
        (1, 3, 11.903),
        (1, 4, 8.136),
        (2, 0, 4.358),
        (2, 1, 6.824),
        (2, 3, 13.722),
        (2, 4, 8.761),
        (3, 0, 3.721),
        (3, 1, 6.381),
        (3, 2, 7.390),
        (3, 3, 7.676),
        (4, 1, 4.461),
        (4, 2, 10.848),
        (4, 3, 14.129),
        (4, 4, 9.778),
        (5, 0, 4.435),
        (5, 2, 8.510),
        (5, 4, 7.391),
        (6, 0, 4.142),
        (6, 1, 5.278),
        (6, 2, 7.917),
        (6, 4, 10.043),
        (7, 0, 4.776),
        (7, 2, 10.112),
        (7, 3, 11.985),
        (7, 4, 6.869),
        (8, 1, 7.376),
        (8, 2, 9.112),
        (8, 3, 9.749),
        (8, 4, 4.370),
        (9, 0, 3.735),
        (9, 1, 6.313),
        (9, 3, 10.582),
        (10, 0, 3.787),
        (10, 2, 7.217),
        (10, 3, 4.269),
        (10, 4, 3.025),
        (11, 0, 4.038),
        (11, 1, 5.991),
        (11, 2, 7.833),
        (11, 4, 9.571),
        (12, 1, 3.947),
        (12, 2, 8.908),
        (12, 3, 7.655),
        (12, 4, 6.862),
        (13, 0, 3.976),
        (13, 1, 5.448),
        (13, 3, 10.134),
        (13, 4, 8.867),
        (14, 0, 5.618),
        (14, 1, 7.355),
        (14, 2, 8.705),
        (14, 3, 11.262),
        (15, 0, 4.681),
        (15, 2, 8.368),
        (15, 3, 7.946),
        (15, 4, 7.466),
    ];
    let c_ids: Vec<usize> = obs.iter().map(|r| r.0).collect();
    let t_ids: Vec<usize> = obs.iter().map(|r| r.1).collect();
    let y: Vec<f64> = obs.iter().map(|r| r.2).collect();
    let labels: Vec<String> = (0..5).map(|k| format!("T{k}")).collect();
    let x = sum_contrast_one_way_matrix(&t_ids, 5);

    let fit = fit_unstructured_rm_reml(&x, &y, &c_ids, &t_ids, &labels, &[("Time", 1, 4)]).unwrap();
    assert!(fit.converged, "incomplete cohort with t=5 > n_complete=2 should converge");
    assert_eq!(fit.fallback_reason, None);
    assert_approx_eq(fit.reml_log_lik, -49.981_453_239_617_99, 1e-3, 1e-4, "Oracle5 logLik");
    assert_approx_eq(fit.fixed_terms[0].f_statistic.unwrap(), 43.755_100_273_105_59, 5e-2, 1e-3, "Oracle5 F_KR");
    assert_approx_eq(fit.satterthwaite_df[0], 12.866_290_198_387_55, 2e-2, 1e-3, "Oracle5 nu_Sat");
    assert_approx_eq(fit.kenward_roger_df[0], 10.216_563_497_929_79, 2e-2, 1e-3, "Oracle5 nu_KR");
    assert_approx_eq(fit.fixed_terms[0].p_value.unwrap(), 2.172_863e-6, 5e-7, 1e-2, "Oracle5 p");
    assert_approx_eq(fit.covariance_matrix[0][0], 0.327_669_587_909_473, 2e-3, 2e-3, "Oracle5 Sigma[0,0]");
    assert_approx_eq(fit.covariance_matrix[3][3], 6.925_672_713_847_423, 2e-2, 2e-3, "Oracle5 Sigma[3,3]");
    assert_approx_eq(fit.covariance_matrix[4][4], 5.034_863_470_110_342, 2e-2, 2e-3, "Oracle5 Sigma[4,4]");
}

// ============================================================================
// 2. Required Edge Cases + Task 2 Dunnett Control Column Fix
// ============================================================================

#[test]
fn r34_edge_single_subject_engages_spd_isotropic_fallback() {
    // 1 subject measured at 4 timepoints with 2-parameter linear trend (n=4 > p=2, C=1):
    // covariance matrix is unidentifiable, so fallback must yield finite SPD diagonal Sigma = s^2 I.
    let c_ids = [0, 0, 0, 0];
    let t_ids = [0, 1, 2, 3];
    let y = [10.2, 11.9, 14.3, 15.8];
    let x = DenseMatrix::from_row_major(4, 2, vec![1.0, 0.0, 1.0, 1.0, 1.0, 2.0, 1.0, 3.0]).unwrap();
    let labels: Vec<String> = (1..=4).map(|k| format!("T{k}")).collect();

    let fit = fit_unstructured_rm_reml(&x, &y, &c_ids, &t_ids, &labels, &[("Slope", 1, 1)]).unwrap();
    assert_eq!(fit.fallback_reason, Some(UnstructuredFallbackReason::SingleSubject));
    assert!(fit.mean_diagonal_variance > 0.0 && fit.mean_diagonal_variance.is_finite());
    assert!(fit.fixed_terms[0].f_statistic.unwrap().is_finite());
    assert!(fit.fixed_terms[0].p_value.unwrap().is_finite());
    for i in 0..4 {
        assert_approx_eq(fit.covariance_matrix[i][i], fit.mean_diagonal_variance, 1e-12, 1e-12, "SingleSubject diag");
        assert_approx_eq(fit.correlation_matrix[i][i], 1.0, 1e-12, 1e-12, "SingleSubject corr diag");
        for j in 0..4 {
            if i != j {
                assert_approx_eq(fit.covariance_matrix[i][j], 0.0, 1e-12, 1e-12, "SingleSubject offdiag cov");
                assert_approx_eq(fit.correlation_matrix[i][j], 0.0, 1e-12, 1e-12, "SingleSubject offdiag corr");
            }
        }
    }

    // End-to-end 1-subject additive GroupedTable (max_interaction_order = 1 so n - p = 2 > 0) warns cleanly.
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(1), 3, 2).unwrap();
    for c in 0..3 {
        grouped.data_sets[c].cells[0][0] = Cell::number(10.0 + (c as f64) * 2.0);
        grouped.data_sets[c].cells[1][0] = Cell::number(14.0 + (c as f64) * 2.1);
    }
    let sheet = DataSheet::new("g-1sub", "OneSubject", TableArchetype::Grouped(grouped)).unwrap();
    let fac_spec = FactorialAnovaSpec {
        max_interaction_order: 1,
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        residual_covariance: ResidualCovariance::Unstructured,
        ..FactorialAnovaSpec::default()
    };
    let spec = AnalysisSpec::new("an-1sub", "g-1sub", AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap();
    let res = run_factorial_anova(&sheet, &spec, &fac_spec).unwrap();
    assert!(res.checklist.iter().any(|c| c.assumption == "Unstructured covariance REML" && c.status == ChecklistStatus::Warn));
}

#[test]
fn r34_edge_all_cells_missing_for_timepoint_compacts_cleanly_and_matches_dropped_table() {
    // 2 rows x 3 cols split-plot GroupedTable where Column 1 ("Week 2") has ZERO active observations.
    let vals = [
        [10.2, 14.5, 11.1, 16.0],
        [9.8, 13.9, 10.6, 15.2],
        [11.4, 16.1, 12.0, 17.5],
        [10.0, 14.2, 11.4, 15.9],
        [10.7, 15.1, 11.8, 16.8],
        [9.5, 13.7, 10.2, 14.9],
    ];
    let mut g_with_empty = GroupedTable::new(SubcolumnMode::RawReplicates(6), 3, 2).unwrap();
    g_with_empty.data_sets[0].title = String::from("W1");
    g_with_empty.data_sets[1].title = String::from("W2_Empty");
    g_with_empty.data_sets[2].title = String::from("W3");
    let mut g_trimmed = GroupedTable::new(SubcolumnMode::RawReplicates(6), 2, 2).unwrap();
    g_trimmed.data_sets[0].title = String::from("W1");
    g_trimmed.data_sets[1].title = String::from("W3");
    for (s, row) in vals.iter().enumerate() {
        g_with_empty.data_sets[0].cells[0][s] = Cell::number(row[0]);
        g_with_empty.data_sets[2].cells[0][s] = Cell::number(row[1]);
        g_with_empty.data_sets[0].cells[1][s] = Cell::number(row[2]);
        g_with_empty.data_sets[2].cells[1][s] = Cell::number(row[3]);

        g_trimmed.data_sets[0].cells[0][s] = Cell::number(row[0]);
        g_trimmed.data_sets[1].cells[0][s] = Cell::number(row[1]);
        g_trimmed.data_sets[0].cells[1][s] = Cell::number(row[2]);
        g_trimmed.data_sets[1].cells[1][s] = Cell::number(row[3]);
    }

    let fac_spec = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        residual_covariance: ResidualCovariance::Unstructured,
        ..FactorialAnovaSpec::default()
    };
    let s1 = DataSheet::new("g-empty-tp", "With Empty Tp", TableArchetype::Grouped(g_with_empty)).unwrap();
    let s2 = DataSheet::new("g-trim-tp", "Trimmed Tp", TableArchetype::Grouped(g_trimmed)).unwrap();
    let r1 =
        run_factorial_anova(&s1, &AnalysisSpec::new("a1", "g-empty-tp", AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap(), &fac_spec).unwrap();
    let r2 =
        run_factorial_anova(&s2, &AnalysisSpec::new("a2", "g-trim-tp", AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap(), &fac_spec).unwrap();

    let cov1 = r1.tabs.iter().find(|t| t.id == "unstructured-reml-covariance").unwrap();
    let cov2 = r2.tabs.iter().find(|t| t.id == "unstructured-reml-covariance").unwrap();
    assert_eq!(cov1.column_headers, vec![String::from("W1"), String::from("W3")]);
    assert!(cov1.footnotes.iter().any(|f| f.contains("W2_Empty")));
    assert!(
        r1.checklist
            .iter()
            .any(|c| c.assumption == "Unstructured covariance REML" && c.status == ChecklistStatus::Warn && c.explanation.contains("W2_Empty"))
    );

    let g1 = extract_cov_grid(cov1);
    let g2 = extract_cov_grid(cov2);
    for i in 0..2 {
        for j in 0..2 {
            assert_approx_eq(g1[i][j], g2[i][j], 1e-8, 1e-8, "All-missing tp Sigma matches dropped table");
        }
    }
    let sc1 = |k: &str| r1.scalars.iter().find(|s| s.key == k).map(|s| s.value).unwrap();
    let sc2 = |k: &str| r2.scalars.iter().find(|s| s.key == k).map(|s| s.value).unwrap();
    assert_approx_eq(sc1("REML log-likelihood"), sc2("REML log-likelihood"), 1e-7, 1e-7, "All-missing tp logLik invariant");
}

#[test]
fn r34_edge_small_cluster_count_below_timepoints_triggers_rank_deficient_fallback() {
    // 3 subjects x 5 timepoints (C=3 < t=5, total obs=15, df_resid=10 < K=15):
    // covariance has at most rank C-1=2 < 5, so shrinkage SPD fallback must engage cleanly.
    let mut c_ids = Vec::with_capacity(15);
    let mut t_ids = Vec::with_capacity(15);
    let mut y = Vec::with_capacity(15);
    let data = [[5.1, 6.2, 8.0, 9.5, 11.0], [4.8, 7.1, 8.9, 10.2, 12.1], [5.9, 6.8, 8.4, 9.9, 10.7]];
    for (s, row) in data.iter().enumerate() {
        for (t, &val) in row.iter().enumerate() {
            c_ids.push(s);
            t_ids.push(t);
            y.push(val);
        }
    }
    let labels: Vec<String> = (0..5).map(|k| format!("T{k}")).collect();
    let x = sum_contrast_one_way_matrix(&t_ids, 5);
    let fit = fit_unstructured_rm_reml(&x, &y, &c_ids, &t_ids, &labels, &[("Time", 1, 4)]).unwrap();

    assert_eq!(fit.fallback_reason, Some(UnstructuredFallbackReason::SmallSampleRankDeficient));
    assert!(fit.fixed_terms[0].f_statistic.unwrap().is_finite());
    assert!(fit.fixed_terms[0].p_value.unwrap().is_finite());
    let flat_cov: Vec<f64> = fit.covariance_matrix.iter().flatten().copied().collect();
    let eig = sym_eigen(&DenseMatrix::from_row_major(5, 5, flat_cov).unwrap()).unwrap();
    assert!(eig.eigenvalues.iter().all(|&ev| ev > 0.0), "regularized Sigma must remain strictly SPD: {:?}", eig.eigenvalues);
}

#[test]
fn r34_edge_singular_covariance_estimate_triggers_spd_shrinkage_fallback() {
    // C=10 subjects, t=4 timepoints, but T3 residuals equal 2 * T1 residuals - T2 residuals (exact rank <= 3).
    // Unconstrained REML diverges to singular boundary unless safeguarded; shrinkage fallback must keep SPD & finite KR df.
    let mut c_ids = Vec::with_capacity(40);
    let mut t_ids = Vec::with_capacity(40);
    let mut y = Vec::with_capacity(40);
    let base = [
        [10.0, 12.0, 14.0],
        [11.2, 12.4, 15.1],
        [9.4, 11.7, 13.5],
        [10.8, 13.1, 14.9],
        [9.9, 11.2, 14.3],
        [12.1, 13.8, 16.0],
        [8.7, 10.9, 12.8],
        [10.5, 12.6, 15.2],
        [9.6, 11.5, 13.9],
        [11.0, 12.9, 14.4],
    ];
    for (s, b) in base.iter().enumerate() {
        let t3 = 5.0 + 2.0 * (b[1] - 12.0) - (b[0] - 10.0);
        for (t, &val) in [b[0], b[1], b[2], t3].iter().enumerate() {
            c_ids.push(s);
            t_ids.push(t);
            y.push(val);
        }
    }
    let labels: Vec<String> = (1..=4).map(|k| format!("T{k}")).collect();
    let x = sum_contrast_one_way_matrix(&t_ids, 4);
    let fit = fit_unstructured_rm_reml(&x, &y, &c_ids, &t_ids, &labels, &[("Time", 1, 3)]).unwrap();

    assert_eq!(fit.fallback_reason, Some(UnstructuredFallbackReason::SingularCovarianceEstimate));
    assert!(fit.fixed_terms[0].f_statistic.unwrap().is_finite());
    assert!(fit.fixed_terms[0].p_value.unwrap().is_finite());
    assert!(fit.kenward_roger_df[0].is_finite() && fit.kenward_roger_df[0] >= 1.0);
    let flat_cov: Vec<f64> = fit.covariance_matrix.iter().flatten().copied().collect();
    let eig = sym_eigen(&DenseMatrix::from_row_major(4, 4, flat_cov).unwrap()).unwrap();
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    let min_ev = eig.eigenvalues.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(min_ev > 0.0 && min_ev / max_ev >= 5e-4, "shrunk Sigma must stay well-conditioned SPD: {:?}", eig.eigenvalues);
}

#[test]
fn r34_factorial_dunnett_unmatched_control_column_returns_typed_error() {
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap();
    for r in 0..2 {
        for c in 0..2 {
            for s in 0..3 {
                grouped.data_sets[c].cells[r][s] = Cell::number(10.0 + (r as f64) * 2.0 + (c as f64) * 3.0 + (s as f64) * 0.25);
            }
        }
    }
    let sheet = DataSheet::new("g-dun-err", "Factorial Dunnett Err", TableArchetype::Grouped(grouped)).unwrap();
    let fac_spec = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 9 },
            threshold: 0.05,
            report_confidence_intervals: true,
        },
        ..FactorialAnovaSpec::default()
    };
    let spec = AnalysisSpec::new("an-dun-err", "g-dun-err", AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap();
    let err = run_factorial_anova(&sheet, &spec, &fac_spec).unwrap_err();
    assert!(
        matches!(err, StatsError::InvalidParameter { name: "control_column", value } if (value - 9.0).abs() < 1e-12),
        "expected InvalidParameter control_column=9, got {err:?}"
    );
}
