//! R3.5 failing reproducers and D1 primary-literature oracle tests for `crates/stats/src/anova/`.
//!
//! Covers R3.5 checkboxes 1, 2, 3, 4, and 8:
//! - Genuine Fai & Cornelius (1996) Satterthwaite and Kenward & Roger (1997) REML df, adjusted covariance, and scaled F.
//! - Hierarchical nested ANOVA post-hoc group-mean SE (`sigma_u^2 sum n_gj^2 / N_g^2 + sigma_e^2 / N_g`).
//! - Repeated-measures two-way sphericity (row, column, interaction), split-plot pooled within-group covariance (`df_e = N - g`),
//!   applied GG/HF p-values, and 4-stratum REML fallback `(S, S×A, S×B, S×A×B)` for incomplete fully-repeated RM designs.
//! - ANOVA effect sizes (multi-way `eta_squared_ci = None` / true `total_n`, `cohens_f = +inf` at `partial_eta^2 = 1`,
//!   Kruskal-Wallis rank `epsilon_squared = H / (N - 1)`).
//! - Minor fixes: Bartlett zero-variance group -> `(+inf, 0.0)`, Kruskal-Wallis all-tied -> `tie_correction = 0.0`,
//!   aliased factorial terms -> `df = 0.0` (`ss = 0.0, ms = 0.0, f_statistic = None, p_value = None`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ConfidenceLevel, DataSheet, FactorialAnovaSpec, GroupedTable, MultiplicityMethod, MultiplicitySpec,
    NestedAnalysisSpec, NestedTable, RepeatedMeasuresDesign, ResultCell, SheetId, SubcolumnMode, SumOfSquaresType, TableArchetype,
};
use curvebook_math::linalg::DenseMatrix;
use curvebook_stats::anova::{
    FactorialObservations, RmSphericityCorrection, compute_anova_effect_sizes, compute_anova_effect_sizes_for_n, fit_factorial_anova,
    fit_multi_vc_reml, fit_random_intercept_reml, fit_rm_two_way_anova, fit_rm_two_way_anova_with_sphericity, fit_rm_two_way_split_plot_anova,
    kruskal_wallis_test, run_factorial_anova, run_nested_analysis, variance_homogeneity_tests,
};
use curvebook_testkit::assert_approx_eq;

fn find_term<'a>(terms: &'a [curvebook_stats::anova::FactorialAnovaTerm], name: &str) -> &'a curvebook_stats::anova::FactorialAnovaTerm {
    let found = terms.iter().find(|t| t.name == name);
    assert!(found.is_some(), "missing term `{name}` in {terms:?}");
    found.unwrap()
}

fn cluster_g_matrix(cluster_ids: &[usize]) -> (DenseMatrix, usize) {
    let n = cluster_ids.len();
    let mut map = std::collections::BTreeMap::<usize, usize>::new();
    let mut mapped = Vec::with_capacity(n);
    for &c in cluster_ids {
        let next = map.len();
        mapped.push(*map.entry(c).or_insert(next));
    }
    let mut g = vec![0.0_f64; n * n];
    for i in 0..n {
        g[i * n + i] = 1.0;
        for j in (i + 1)..n {
            if mapped[i] == mapped[j] {
                g[i * n + j] = 1.0;
                g[j * n + i] = 1.0;
            }
        }
    }
    (DenseMatrix::from_row_major(n, n, g).unwrap(), map.len())
}

// ============================================================================
// R3.5 #1: Genuine REML Satterthwaite (1946) / Fai & Cornelius (1996) + Kenward & Roger (1997)
// ============================================================================

#[test]
fn r35_1a_reml_random_intercept_satterthwaite_and_kenward_roger_oracle() {
    // D1 Provenance: R 4.5.2 + exact Kenward & Roger (1997, Biometrics 53:983-997) and
    // Fai & Cornelius (1996, J. Stat. Comput. Simul. 54:363-378) equations on unbalanced 1-RI design
    // (18 observations in 7 unequal clusters sizes [1, 5, 2, 1, 4, 3, 2], contr.sum 3-level group + covariate).
    let c_id = [0, 1, 1, 1, 1, 1, 2, 2, 3, 4, 4, 4, 4, 5, 5, 5, 6, 6];
    let grp = [0, 0, 0, 1, 1, 2, 1, 2, 0, 1, 1, 2, 2, 0, 2, 2, 0, 1];
    let x_cov = [-1.2, 0.3, 1.1, -0.8, 0.5, 1.4, -0.4, 0.9, 0.0, -1.1, 0.2, 0.8, -0.3, 1.3, -0.7, 0.4, -0.9, 0.6];
    let y_unb = [10.5, 11.2, 12.1, 8.3, 9.6, 13.8, 12.9, 16.4, 7.1, 10.2, 11.5, 14.9, 13.2, 15.1, 14.8, 16.2, 9.0, 10.4];

    let mut x_raw = Vec::with_capacity(y_unb.len() * 4);
    for i in 0..y_unb.len() {
        let (g1, g2) = match grp[i] {
            0 => (1.0, 0.0),
            1 => (0.0, 1.0),
            _ => (-1.0, -1.0),
        };
        x_raw.extend_from_slice(&[1.0, g1, g2, x_cov[i]]);
    }
    let x_mat = DenseMatrix::from_row_major(y_unb.len(), 4, x_raw).unwrap();
    let reml = fit_random_intercept_reml(&x_mat, &y_unb, &c_id, &[("grp", 1, 2), ("x_cov", 3, 1)]).unwrap();

    assert_approx_eq(reml.sigma2_random, 5.622_667_106_009_42, 1e-4, 1e-4, "1-RI unbalanced s2_u");
    assert_approx_eq(reml.sigma2_error, 0.156_760_539_128_263, 1e-5, 1e-4, "1-RI unbalanced s2_e");

    // Multi-df term grp (q = 2): note nu_sat != heuristic 2/Var(log v), and KR adjusts F + df.
    assert_approx_eq(reml.fixed_terms[0].f_statistic.unwrap(), 46.516_167_943_630_7, 1e-3, 1e-4, "grp F_KR");
    assert_approx_eq(reml.satterthwaite_df[0], 8.127_722_012_259_74, 1e-3, 1e-4, "grp nu_Sat");
    assert_approx_eq(reml.kenward_roger_df[0], 8.127_934_132_195_78, 1e-3, 1e-4, "grp nu_KR");
    assert_approx_eq(reml.fixed_terms[0].p_value.unwrap(), 3.546_785_615_422_03e-5, 1e-7, 1e-3, "grp p_KR");

    // Single-df covariate x_cov (q = 1): KR identity nu_KR == nu_Sat == 2/A2 (no fake 1/(1+1/(c-1)) shrink).
    assert_approx_eq(reml.fixed_terms[1].f_statistic.unwrap(), 69.953_046_353_934_9, 1e-3, 1e-4, "x_cov F_KR");
    assert_approx_eq(reml.satterthwaite_df[1], 8.125_720_929_319_66, 1e-3, 1e-4, "x_cov nu_Sat");
    assert_approx_eq(reml.kenward_roger_df[1], 8.125_720_929_319_66, 1e-3, 1e-4, "x_cov nu_KR");
    assert_approx_eq(reml.fixed_terms[1].p_value.unwrap(), 2.885_470_144_059_55e-5, 1e-7, 1e-3, "x_cov p_KR");

    // Boundary tau_hat == 0 case (5 balanced clusters x 4 obs, within-cluster orthogonal covariate):
    // full 2x2 REML information matrix accounts for joint estimation of (sigma2_u, sigma2_e), giving
    // exact within-cluster residual df = c*(m - 1) - 1 = 14.0 instead of n - p = 18.0!
    let c_b: Vec<usize> = (0..5).flat_map(|c| std::iter::repeat_n(c, 4)).collect();
    let x_b: Vec<f64> = (0..5).flat_map(|_| [-1.0, 1.0, -1.0, 1.0]).collect();
    let mut y_b: Vec<f64> = (0..20).map(|i| 10.0 + 2.0 * x_b[i] + [0.5, -0.5, -0.5, 0.5][i % 4]).collect();
    for (i, d) in [0.05, -0.05, 0.05, -0.05].into_iter().enumerate() {
        y_b[i] += d;
    }
    let xb_mat = DenseMatrix::from_row_major(20, 2, x_b.iter().flat_map(|&xv| [1.0, xv]).collect()).unwrap();
    let bnd = fit_random_intercept_reml(&xb_mat, &y_b, &c_b, &[("x_b", 1, 1)]).unwrap();
    assert!(bnd.sigma2_random <= 1e-9, "boundary variance must be zero, got {}", bnd.sigma2_random);
    assert_approx_eq(bnd.sigma2_error, 0.278_222_222_222_222, 1e-6, 1e-6, "boundary s2_e");
    assert_approx_eq(bnd.fixed_terms[0].f_statistic.unwrap(), 284.671_725_239_617, 1e-3, 1e-4, "boundary F_KR");
    assert_approx_eq(bnd.satterthwaite_df[0], 14.0, 1e-3, 1e-4, "boundary nu_Sat == 14");
    assert_approx_eq(bnd.kenward_roger_df[0], 14.0, 1e-3, 1e-4, "boundary nu_KR == 14");
}

#[test]
fn r35_1b_rm_two_way_reml_fallback_4_stratum_and_boundary_oracle() {
    // D1 Provenance: R 4.5.2 + KR97 on 8-subject 2x3 missing-cell RM design with 4 random strata:
    // y ~ A * B + (1 | subj) + (1 | subj:A) + (1 | subj:B) + eps, 44 / 48 observations.
    let rm_missing_rows: &[(usize, usize, usize, f64)] = &[
        (0, 0, 0, 12.4),
        (1, 0, 0, 8.1),
        (2, 0, 0, 11.0),
        (3, 0, 0, 13.5),
        (4, 0, 0, 7.7),
        (5, 0, 0, 11.5),
        (6, 0, 0, 9.1),
        (7, 0, 0, 7.5),
        (0, 0, 1, 14.2),
        (1, 0, 1, 10.0),
        (2, 0, 1, 13.4),
        (4, 0, 1, 10.2),
        (5, 0, 1, 12.8),
        (6, 0, 1, 11.5),
        (7, 0, 1, 9.5),
        (0, 0, 2, 17.1),
        (1, 0, 2, 14.2),
        (2, 0, 2, 15.9),
        (3, 0, 2, 18.5),
        (4, 0, 2, 12.6),
        (5, 0, 2, 16.5),
        (6, 0, 2, 14.1),
        (7, 0, 2, 13.1),
        (0, 1, 0, 16.8),
        (1, 1, 0, 12.7),
        (2, 1, 0, 15.7),
        (4, 1, 0, 12.6),
        (5, 1, 0, 15.4),
        (6, 1, 0, 14.0),
        (7, 1, 0, 11.4),
        (0, 1, 1, 20.2),
        (1, 1, 1, 16.0),
        (2, 1, 1, 18.6),
        (3, 1, 1, 20.3),
        (4, 1, 1, 15.8),
        (5, 1, 1, 18.5),
        (7, 1, 1, 14.8),
        (1, 1, 2, 21.3),
        (2, 1, 2, 23.1),
        (3, 1, 2, 26.0),
        (4, 1, 2, 20.2),
        (5, 1, 2, 23.8),
        (6, 1, 2, 21.6),
        (7, 1, 2, 20.2),
    ];

    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(8), 2, 3).unwrap();
    grouped.row_factor_title = String::from("Timepoint");
    grouped.column_factor_title = String::from("Treatment");
    grouped.row_titles = vec![String::from("T1"), String::from("T2"), String::from("T3")];
    grouped.data_sets[0].title = String::from("Placebo");
    grouped.data_sets[1].title = String::from("Drug");
    for &(subj, col, row, y) in rm_missing_rows {
        grouped.data_sets[col].cells[row][subj] = Cell::number(y);
    }

    let ds_id = SheetId::new("g-r35-reml4");
    let mut sheet = DataSheet::new(ds_id.clone(), "RM Missing 4-Stratum", TableArchetype::Grouped(grouped)).unwrap();
    sheet.id = ds_id.clone();

    let fac_spec = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        fit_reml_on_missing: true,
        compute_effect_sizes: true,
        ..FactorialAnovaSpec::default()
    };
    let spec = AnalysisSpec::new("an-r35-reml4", ds_id, AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap();
    let res = run_factorial_anova(&sheet, &spec, &fac_spec).unwrap();

    let scalar = |key: &str| res.scalars.iter().find(|s| s.key == key).map(|s| s.value).unwrap();
    assert_approx_eq(scalar("REML sigma^2 (subject)"), 4.464_884_941_512_92, 1e-3, 1e-3, "4-stratum s2_S");
    assert_approx_eq(scalar("REML sigma^2 (subject × Timepoint)"), 0.088_333_824_264_694, 1e-3, 1e-2, "4-stratum s2_SA");
    assert_approx_eq(scalar("REML sigma^2 (subject × Treatment)"), 0.008_785_904_276_616, 1e-3, 1e-1, "4-stratum s2_SB");
    assert_approx_eq(scalar("REML sigma^2 (residual)"), 0.047_890_562_833_143, 1e-3, 2e-2, "4-stratum s2_e");
    assert_approx_eq(scalar("REML log-likelihood"), -4.692_444_027_256_86, 1e-3, 1e-3, "4-stratum ll");

    // In GroupedTable, row factor is Timepoint (df=2) and col factor is Treatment (df=1).
    assert_approx_eq(scalar("Timepoint: F"), 761.232_872_969_742, 0.5, 1e-3, "4-stratum F_Timepoint");
    assert_approx_eq(scalar("Timepoint: Satterthwaite df"), 13.836_006_561_939_1, 5e-3, 1e-3, "4-stratum Sat Timepoint");
    assert_approx_eq(scalar("Timepoint: Kenward-Roger df"), 13.835_965_188_095_6, 5e-3, 1e-3, "4-stratum KR Timepoint");
    assert_approx_eq(scalar("Timepoint: P"), 7.071_222_882_724_39e-15, 1e-16, 1e-2, "4-stratum P_Timepoint");

    assert_approx_eq(scalar("Treatment: F"), 4_698.468_491_241_58, 2.0, 1e-3, "4-stratum F_Treatment");
    assert_approx_eq(scalar("Treatment: Satterthwaite df"), 6.605_999_433_703_59, 5e-3, 1e-3, "4-stratum Sat Treatment");
    assert_approx_eq(scalar("Treatment: Kenward-Roger df"), 6.605_999_433_703_59, 5e-3, 1e-3, "4-stratum KR Treatment");
    assert_approx_eq(scalar("Treatment: P"), 1.131_360_780_211_14e-10, 1e-12, 1e-2, "4-stratum P_Treatment");

    assert_approx_eq(scalar("Timepoint × Treatment: F"), 150.392_411_809_038, 0.5, 1e-3, "4-stratum F_interaction");
    assert_approx_eq(scalar("Timepoint × Treatment: Satterthwaite df"), 11.969_170_905_415_0, 5e-3, 1e-3, "4-stratum Sat interaction");
    assert_approx_eq(scalar("Timepoint × Treatment: Kenward-Roger df"), 11.972_359_139_826_7, 5e-3, 1e-3, "4-stratum KR interaction");
    assert_approx_eq(scalar("Timepoint × Treatment: P"), 3.291_687_452_482_49e-9, 1e-10, 1e-2, "4-stratum P_interaction");

    // 4-stratum boundary test where sigma^2_{S x row} converges to 0 on the boundary.
    let s_eff = [-2.1, 1.8, -0.5, 2.4, -1.7, 0.9, -0.2, -0.6];
    let sc_eff = [[0.4, -0.4], [-0.3, 0.3], [0.5, -0.5], [-0.2, 0.2], [0.1, -0.1], [-0.4, 0.4], [0.2, -0.2], [-0.3, 0.3]];
    let row_eff = [0.0, 2.5, 5.0];
    let col_eff = [0.0, 3.0];
    let n_bnd = rm_missing_rows.len();
    let mut x_bnd = Vec::with_capacity(n_bnd * 6);
    let mut y_bnd = Vec::with_capacity(n_bnd);
    let mut s_ids = Vec::with_capacity(n_bnd);
    let mut sr_ids = Vec::with_capacity(n_bnd);
    let mut sc_ids = Vec::with_capacity(n_bnd);
    for (idx, &(s, col, row, _)) in rm_missing_rows.iter().enumerate() {
        let (r1, r2) = match row {
            0 => (1.0, 0.0),
            1 => (0.0, 1.0),
            _ => (-1.0, -1.0),
        };
        let c1 = if col == 0 { 1.0 } else { -1.0 };
        x_bnd.extend_from_slice(&[1.0, r1, r2, c1, r1 * c1, r2 * c1]);
        let y_val = 10.0
            + s_eff[s]
            + row_eff[row]
            + col_eff[col]
            + sc_eff[s][col]
            + 0.4 * (row as f64) * (col as f64)
            + 0.25 * (((idx + 1) as f64) * 1.7).sin();
        y_bnd.push(y_val);
        s_ids.push(s);
        sr_ids.push(s * 3 + row);
        sc_ids.push(s * 2 + col);
    }
    let xb_mat = DenseMatrix::from_row_major(n_bnd, 6, x_bnd).unwrap();
    let (gs, cs) = cluster_g_matrix(&s_ids);
    let (gsr, csr) = cluster_g_matrix(&sr_ids);
    let (gsc, csc) = cluster_g_matrix(&sc_ids);
    let mres = fit_multi_vc_reml(
        &xb_mat,
        &y_bnd,
        &[("S", gs, cs), ("S×row", gsr, csr), ("S×col", gsc, csc)],
        &[("row", 1, 2), ("col", 3, 1), ("row×col", 4, 2)],
    )
    .unwrap();
    assert!(mres.variance_components[1].1 <= 1e-6, "S×row variance should hit zero boundary, got {}", mres.variance_components[1].1);
    assert_approx_eq(mres.sigma2_error, 0.022_842_973_707_870, 1e-4, 1e-3, "boundary multi-vc s2_e");
    assert_approx_eq(mres.variance_components[0].1, 2.244_879_385_480_74, 1e-3, 1e-3, "boundary multi-vc s2_S");
    assert_approx_eq(mres.variance_components[2].1, 0.231_217_150_640_683, 1e-3, 1e-2, "boundary multi-vc s2_SC");
    assert_approx_eq(mres.fixed_terms[0].f_statistic.unwrap(), 4_526.512_854_332_38, 2.0, 1e-3, "boundary multi-vc F_row");
    assert_approx_eq(mres.satterthwaite_df[0], 12.705_998_500_826_6, 5e-3, 1e-3, "boundary multi-vc Sat_row");
    assert_approx_eq(mres.kenward_roger_df[0], 12.701_238_782_279_8, 5e-3, 1e-3, "boundary multi-vc KR_row");
    assert_approx_eq(mres.fixed_terms[1].f_statistic.unwrap(), 189.377_731_822_396, 0.2, 1e-3, "boundary multi-vc F_col");
    assert_approx_eq(mres.kenward_roger_df[1], 6.992_501_735_805_26, 5e-3, 1e-3, "boundary multi-vc KR_col");
}

// ============================================================================
// R3.5 #2: Nested ANOVA Post-hoc Uses Group-Mean Variance (Not Pooled / n_rep)
// ============================================================================

#[test]
fn r35_2_nested_posthoc_uses_hierarchical_group_mean_variance_oracle() {
    // D1 Provenance: R 4.5.2 + Sokal & Rohlf / Montgomery hierarchical group-mean variance
    // Var(mean_g) = sigma_u^2 * sum_j(n_{gj}^2) / N_g^2 + sigma_e^2 / N_g on balanced AND unbalanced nested tables.
    let mut nt = NestedTable::new(3, 4, 5).unwrap();
    let groups_vals = [
        [[10.1, 10.3, 9.8, 10.0, 10.2], [14.2, 14.5, 13.9, 14.1, 14.3], [8.4, 8.6, 8.1, 8.5, 8.3], [19.1, 19.3, 18.8, 19.0, 19.2]],
        [[15.2, 15.5, 14.9, 15.1, 15.4], [18.1, 18.4, 17.9, 18.2, 18.0], [12.3, 12.6, 12.1, 12.4, 12.2], [21.4, 21.7, 21.1, 21.5, 21.3]],
        [[22.1, 22.4, 21.8, 22.0, 22.3], [26.2, 26.5, 25.9, 26.1, 26.4], [19.5, 19.8, 19.2, 19.6, 19.4], [31.0, 31.3, 30.8, 31.1, 31.2]],
    ];
    for g in 0..3 {
        nt.groups[g].title = format!("G{}", g + 1);
        for b in 0..4 {
            for r in 0..5 {
                nt.groups[g].cells[r][b] = Cell::number(groups_vals[g][b][r]);
            }
        }
    }

    let ds_id = SheetId::new("n-posthoc");
    let sheet = DataSheet::new(ds_id.clone(), "Nested 3x4x5", TableArchetype::Nested(nt)).unwrap();
    let nspec = NestedAnalysisSpec {
        two_group_t_test: false,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::TukeyHsd,
            threshold: 0.05,
            report_confidence_intervals: true,
            control_column: None,
            selected_pairs: None,
        },
        ..NestedAnalysisSpec::default()
    };
    let spec = AnalysisSpec::new("an-nest-tuk", ds_id, AnalysisKind::NestedComparison(nspec.clone())).unwrap();
    let res = run_nested_analysis(&sheet, &spec, &nspec).unwrap();

    // Verify exact Tukey-adjusted p-values from exact REML variance components (s2_u = 21.15008333,
    // s2_e = 0.04525, tau = 467.4) and hierarchical SE_diff = 3.252625 on df_KR = 9.0:
    // G1-G2 p_tukey = 0.494566037034, G1-G3 p_tukey = 0.013762765366, G2-G3 p_tukey = 0.085206640850.
    assert_eq!(res.pairwise_comparisons.len(), 3);
    assert_approx_eq(res.pairwise_comparisons[0].raw_p_or_q, 0.494_566_037_034, 1e-5, 1e-4, "Nested Tukey G1-G2");
    assert_approx_eq(res.pairwise_comparisons[1].raw_p_or_q, 0.013_762_765_366, 1e-5, 1e-4, "Nested Tukey G1-G3");
    assert_approx_eq(res.pairwise_comparisons[2].raw_p_or_q, 0.085_206_640_850, 1e-5, 1e-4, "Nested Tukey G2-G3");

    // Unbalanced nested table: 3 groups with (4, 3, 5) biological subclusters and varying technical replicates (2..5).
    let mut nt_unb = NestedTable::new(3, 5, 5).unwrap();
    let unb_data: [&[&[f64]]; 3] = [
        &[&[10.1, 10.3, 9.8], &[14.2, 14.5, 13.9, 14.1], &[8.4, 8.6], &[19.1, 19.3, 18.8, 19.0, 19.2]],
        &[&[15.2, 15.5, 14.9, 15.1], &[18.1, 18.4, 17.9], &[12.3, 12.6, 12.1]],
        &[&[22.1, 22.4], &[26.2, 26.5, 25.9, 26.1], &[19.5, 19.8, 19.2], &[31.0, 31.3, 30.8], &[24.0, 24.2, 23.9, 24.1]],
    ];
    for (g, clusters) in unb_data.iter().enumerate() {
        nt_unb.groups[g].title = format!("U{}", g + 1);
        for (b, reps) in clusters.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                nt_unb.groups[g].cells[r][b] = Cell::number(v);
            }
        }
    }
    let ds_unb = SheetId::new("n-unb-post");
    let sheet_unb = DataSheet::new(ds_unb.clone(), "Nested Unbalanced", TableArchetype::Nested(nt_unb)).unwrap();

    let tuk_spec = NestedAnalysisSpec {
        two_group_t_test: false,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::TukeyHsd,
            threshold: 0.05,
            report_confidence_intervals: true,
            control_column: None,
            selected_pairs: None,
        },
        ..NestedAnalysisSpec::default()
    };
    let res_tuk = run_nested_analysis(
        &sheet_unb,
        &AnalysisSpec::new("an-unb-tuk", ds_unb.clone(), AnalysisKind::NestedComparison(tuk_spec.clone())).unwrap(),
        &tuk_spec,
    )
    .unwrap();
    // Exact REML GLS + Kenward-Roger (1997) Phi_A / df Tukey studentized-range p-values (AUD-05-20; R stats::ptukey):
    // U1-U2: diff=-2.257205, SE_KR=3.217393, df_KR=8.9994 -> p_tukey = 0.768597853075
    // U1-U3: diff=-11.645277, SE_KR=2.825930, df_KR=9.0002 -> p_tukey = 0.006581384112
    // U2-U3: diff=-9.388071, SE_KR=3.076434, df_KR=8.9996 -> p_tukey = 0.033357239531
    assert_approx_eq(res_tuk.pairwise_comparisons[0].raw_p_or_q, 0.768_597_853_075, 1e-5, 1e-4, "Unbal Nested REML GLS Tukey U1-U2");
    assert_approx_eq(res_tuk.pairwise_comparisons[1].raw_p_or_q, 0.006_581_384_112, 1e-5, 1e-4, "Unbal Nested REML GLS Tukey U1-U3");
    assert_approx_eq(res_tuk.pairwise_comparisons[2].raw_p_or_q, 0.033_357_239_531, 1e-5, 1e-4, "Unbal Nested REML GLS Tukey U2-U3");

    // Games-Howell assumes flat independent unequal-variance samples and is incompatible with nested REML (AUD-05-20).
    let gh_spec = NestedAnalysisSpec {
        two_group_t_test: false,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::GamesHowell,
            threshold: 0.05,
            report_confidence_intervals: true,
            control_column: None,
            selected_pairs: None,
        },
        ..NestedAnalysisSpec::default()
    };
    let err_gh =
        run_nested_analysis(&sheet_unb, &AnalysisSpec::new("an-unb-gh", ds_unb, AnalysisKind::NestedComparison(gh_spec.clone())).unwrap(), &gh_spec)
            .unwrap_err();
    assert!(matches!(err_gh, curvebook_stats::StatsError::IncompatibleAnalysisSpec { .. }), "expected IncompatibleAnalysisSpec, got {err_gh:?}");
}

// ============================================================================
// R3.5 #3: Repeated-Measures Two-Way & Split-Plot Sphericity
// ============================================================================

#[test]
fn r35_3a_two_way_rm_all_factors_sphericity_applied_and_scale_invariance() {
    // D1 Provenance: R 4.5.2 orthonormal Kronecker Helmert projections on 3x4 fully-repeated RM design (S=12)
    // checking Row, Column, and Interaction sphericity epsilons, Mauchly tests, applied GG/HF p-values,
    // `1e-8` scale invariance, and the `"rm-sphericity"` diagnostics tab in `run_factorial_anova`.
    let mut subject_data = Vec::with_capacity(12);
    for s_idx in 1..=12_usize {
        let s = s_idx as f64;
        let b_s = (s - 6.5) * 1.1;
        let r_s = [(s * 1.3).sin() * 1.1, (s * 0.8).cos() * 0.6, (s * 2.1).sin() * 0.3];
        let c_s = [(s * 0.9).sin() * 1.4, (s * 0.9 + 0.35).sin() * 1.5, (s * 1.5).cos() * 0.5, -(s * 2.3).sin() * 0.7];
        let mut mat = vec![vec![0.0_f64; 4]; 3];
        for i_idx in 1..=3_usize {
            for j_idx in 1..=4_usize {
                let h_idx = ((s_idx - 1) * 12 + (i_idx - 1) * 4 + (j_idx - 1)) as f64;
                let i = i_idx as f64;
                let j = j_idx as f64;
                let extra = if i_idx == 1 && j_idx == 1 { (s * 1.1).cos() * 1.2 } else { 0.0 };
                let int_noise = 0.4 * (h_idx * h_idx * 0.37 + i * 1.9 - j * 0.7).sin() + extra;
                mat[i_idx - 1][j_idx - 1] = 10.0 + b_s + 2.0 * i + 1.5 * j + 0.5 * i * j + r_s[i_idx - 1] + c_s[j_idx - 1] + int_noise;
            }
        }
        subject_data.push(mat);
    }

    let res_none =
        fit_rm_two_way_anova_with_sphericity(&subject_data, "Dose", "Time", RepeatedMeasuresDesign::BothFactorsMatched, RmSphericityCorrection::None)
            .unwrap();
    let res_gg = fit_rm_two_way_anova_with_sphericity(
        &subject_data,
        "Dose",
        "Time",
        RepeatedMeasuresDesign::BothFactorsMatched,
        RmSphericityCorrection::GreenhouseGeisser,
    )
    .unwrap();
    let res_hf = fit_rm_two_way_anova_with_sphericity(
        &subject_data,
        "Dose",
        "Time",
        RepeatedMeasuresDesign::BothFactorsMatched,
        RmSphericityCorrection::HuynhFeldt,
    )
    .unwrap();

    // Check all three sphericity structures against R 4.5.2.
    let s_row = res_none.sphericities.row.as_ref().unwrap();
    assert_approx_eq(s_row.mauchly_w, 0.410_845_482_232, 1e-8, 1e-8, "RM3x4 Row W");
    assert_approx_eq(s_row.mauchly_chi2, 8.895_380_908_137, 1e-6, 1e-6, "RM3x4 Row chi2");
    assert_approx_eq(s_row.mauchly_p, 0.011_705_570_328, 1e-7, 1e-5, "RM3x4 Row mauchly_p");
    assert_approx_eq(s_row.greenhouse_geisser_eps, 0.629_265_429_396, 1e-8, 1e-8, "RM3x4 Row GG");
    assert_approx_eq(s_row.huynh_feldt_eps, 0.672_504_840_676, 1e-8, 1e-8, "RM3x4 Row HF");

    let s_col = res_none.sphericities.column.as_ref().unwrap();
    assert_approx_eq(s_col.mauchly_w, 0.220_062_075_685, 1e-8, 1e-8, "RM3x4 Col W");
    assert_approx_eq(s_col.mauchly_chi2, 14.717_943_432_755, 1e-6, 1e-6, "RM3x4 Col chi2");
    assert_approx_eq(s_col.mauchly_p, 0.011_638_021_258, 1e-7, 1e-5, "RM3x4 Col mauchly_p");
    assert_approx_eq(s_col.greenhouse_geisser_eps, 0.552_162_025_698, 1e-8, 1e-8, "RM3x4 Col GG");
    assert_approx_eq(s_col.huynh_feldt_eps, 0.637_798_336_994, 1e-8, 1e-8, "RM3x4 Col HF");

    let s_int = res_none.sphericities.interaction.as_ref().unwrap();
    assert_approx_eq(s_int.mauchly_w, 0.026_622_102_074, 1e-8, 1e-8, "RM3x4 Int W");
    assert_approx_eq(s_int.mauchly_chi2, 31.828_340_750_050, 1e-6, 1e-6, "RM3x4 Int chi2");
    assert_approx_eq(s_int.mauchly_p, 0.045_162_131_088, 1e-6, 1e-5, "RM3x4 Int mauchly_p");
    assert_approx_eq(s_int.greenhouse_geisser_eps, 0.381_003_852_402, 1e-8, 1e-8, "RM3x4 Int GG");
    assert_approx_eq(s_int.huynh_feldt_eps, 0.486_426_915_190, 1e-8, 1e-8, "RM3x4 Int HF");

    // Check applied F and uncorrected / GG / HF p-values for all 3 terms.
    let exp_f = [244.422_081_059_023, 219.189_088_171_964, 29.196_518_079_570];
    let exp_p_unc = [9.450_977_031_560e-16, 7.403_586_378_690e-22, 8.593_372_416_268e-17];
    let exp_p_gg = [1.199_411_974_597e-10, 5.599_359_733_449e-13, 1.243_774_472_722e-7];
    let exp_p_hf = [3.034_372_645_866e-11, 1.110_872_861_822e-14, 3.325_388_047_640e-9];
    for idx in 0..3 {
        assert_approx_eq(res_none.anova.terms[idx].f_statistic.unwrap(), exp_f[idx], 1e-7, 1e-7, "RM3x4 F");
        assert_approx_eq(res_none.anova.terms[idx].p_value.unwrap(), exp_p_unc[idx], 1e-18, 1e-4, "RM3x4 p_unc");
        assert_approx_eq(res_gg.anova.terms[idx].p_value.unwrap(), exp_p_gg[idx], 1e-14, 1e-4, "RM3x4 p_GG");
        assert_approx_eq(res_hf.anova.terms[idx].p_value.unwrap(), exp_p_hf[idx], 1e-15, 1e-4, "RM3x4 p_HF");
    }

    // Scale invariance at 1e-8: multiplying all outcomes by 1e-8 must preserve W, chi2, GG, HF, and p.
    let scaled_data: Vec<Vec<Vec<f64>>> = subject_data.iter().map(|m| m.iter().map(|r| r.iter().map(|&v| v * 1e-8).collect()).collect()).collect();
    let res_scaled = fit_rm_two_way_anova_with_sphericity(
        &scaled_data,
        "Dose",
        "Time",
        RepeatedMeasuresDesign::BothFactorsMatched,
        RmSphericityCorrection::GreenhouseGeisser,
    )
    .unwrap();
    let si = res_scaled.sphericities.interaction.as_ref().unwrap();
    assert_approx_eq(si.mauchly_w, s_int.mauchly_w, 1e-10, 1e-7, "1e-8 scale invariance W");
    assert_approx_eq(si.greenhouse_geisser_eps, s_int.greenhouse_geisser_eps, 1e-10, 1e-7, "1e-8 scale invariance GG");
    assert_approx_eq(res_scaled.anova.terms[2].p_value.unwrap(), exp_p_gg[2], 1e-12, 1e-4, "1e-8 scale invariance GG P");

    // Also check ResultSheet `"rm-sphericity"` tab via `run_factorial_anova`.
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(12), 4, 3).unwrap();
    grouped.row_factor_title = String::from("Dose");
    grouped.column_factor_title = String::from("Time");
    for (s, mat) in subject_data.iter().enumerate() {
        for (i, row_vec) in mat.iter().enumerate() {
            for (j, &v) in row_vec.iter().enumerate() {
                grouped.data_sets[j].cells[i][s] = Cell::number(v);
            }
        }
    }
    let ds_id = SheetId::new("g-rm34");
    let sheet = DataSheet::new(ds_id.clone(), "RM 3x4", TableArchetype::Grouped(grouped)).unwrap();
    let fac_spec = FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched, ..Default::default() };
    let sheet_res =
        run_factorial_anova(&sheet, &AnalysisSpec::new("an-rm34", ds_id, AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap(), &fac_spec)
            .unwrap();
    let sph_tab = sheet_res.tabs.iter().find(|t| t.id == "rm-sphericity").unwrap();
    assert_eq!(sph_tab.row_labels, vec!["Dose", "Time", "Dose × Time"]);
    for (r_idx, p_expected) in exp_p_gg.iter().enumerate() {
        let p_gg_cell = match sph_tab.rows[r_idx][6] {
            ResultCell::PValue { raw, .. } => raw,
            _ => f64::NAN,
        };
        assert_approx_eq(p_gg_cell, *p_expected, 1e-14, 1e-4, "rm-sphericity tab G-G P");
    }
}

#[test]
fn r35_3b_split_plot_sphericity_pooled_within_group_cov_n_minus_g_oracle() {
    // D1 Provenance: R 4.5.2 pooled within-group covariance (df_e = N - g = 18) per
    // Huynh & Feldt (1976, J. Educ. Stat. 1:69-82, p. 71) on g=2 groups x s=10 subjects/group x b=4 repeated levels.
    let g0: [[f64; 4]; 10] = [
        [10.0, 12.0, 15.0, 21.0],
        [11.0, 13.0, 14.0, 24.0],
        [9.0, 11.0, 16.0, 18.0],
        [12.0, 14.0, 15.0, 26.0],
        [10.0, 11.0, 13.0, 19.0],
        [13.0, 15.0, 17.0, 22.0],
        [11.0, 12.0, 14.0, 25.0],
        [10.0, 13.0, 16.0, 20.0],
        [12.0, 13.0, 15.0, 23.0],
        [9.0, 12.0, 14.0, 21.0],
    ];
    let g1: [[f64; 4]; 10] = [
        [14.0, 17.0, 21.0, 30.0],
        [15.0, 18.0, 22.0, 28.0],
        [13.0, 16.0, 20.0, 33.0],
        [16.0, 19.0, 23.0, 29.0],
        [14.0, 17.0, 22.0, 35.0],
        [17.0, 20.0, 24.0, 31.0],
        [15.0, 18.0, 21.0, 34.0],
        [14.0, 16.0, 20.0, 29.0],
        [16.0, 18.0, 23.0, 32.0],
        [13.0, 17.0, 21.0, 30.0],
    ];
    let mut subject_data = Vec::with_capacity(10);
    for s in 0..10 {
        subject_data.push(vec![g0[s].to_vec(), g1[s].to_vec()]);
    }

    let (_fac, sph_opt) = fit_rm_two_way_anova(&subject_data, "Group", "Time", RepeatedMeasuresDesign::ColumnFactorMatched).unwrap();
    let sph = sph_opt.unwrap();
    assert_approx_eq(sph.mauchly_w, 6.904_364_731_886_02e-2, 1e-8, 1e-8, "Split-plot Mauchly W");
    assert_approx_eq(sph.mauchly_chi2, 44.698_774_320_267_9, 1e-6, 1e-6, "Split-plot Mauchly chi2");
    assert_approx_eq(sph.mauchly_p, 1.670_537_188_090_64e-8, 1e-11, 1e-4, "Split-plot Mauchly p");
    assert_approx_eq(sph.greenhouse_geisser_eps, 4.320_567_649_415_32e-1, 1e-9, 1e-9, "Split-plot GG eps");
    // Lecoutre (1991, J. Educ. Stat. 16:371-372) split-plot correction uses (N - g + 1) = 19 in numerator.
    assert_approx_eq(sph.huynh_feldt_eps, 4.515_378_808_540_866e-1, 1e-9, 1e-9, "Split-plot HF eps");

    // Check applied GG and HF p-values on the split-plot design: between-subjects Group is unadjusted,
    // while within-subject Time and Group x Time use eps_GG / eps_HF.
    let sp_unc = fit_rm_two_way_anova_with_sphericity(
        &subject_data,
        "Group",
        "Time",
        RepeatedMeasuresDesign::ColumnFactorMatched,
        RmSphericityCorrection::None,
    )
    .unwrap();
    let sp_gg = fit_rm_two_way_anova_with_sphericity(
        &subject_data,
        "Group",
        "Time",
        RepeatedMeasuresDesign::ColumnFactorMatched,
        RmSphericityCorrection::GreenhouseGeisser,
    )
    .unwrap();
    let sp_hf = fit_rm_two_way_anova_with_sphericity(
        &subject_data,
        "Group",
        "Time",
        RepeatedMeasuresDesign::ColumnFactorMatched,
        RmSphericityCorrection::HuynhFeldt,
    )
    .unwrap();

    assert_approx_eq(sp_unc.anova.terms[0].p_value.unwrap(), 4.197_706_552_045e-10, 1e-13, 1e-4, "Split-plot Group p_unc");
    assert_approx_eq(sp_gg.anova.terms[0].p_value.unwrap(), 4.197_706_552_045e-10, 1e-13, 1e-4, "Split-plot Group p_GG unchanged");
    assert_approx_eq(sp_unc.anova.terms[1].p_value.unwrap(), 4.005_805_582_440e-36, 1e-37, 1e-3, "Split-plot Time p_unc");
    assert_approx_eq(sp_gg.anova.terms[1].p_value.unwrap(), 7.252_830_749_460e-17, 1e-18, 1e-4, "Split-plot Time p_GG");
    assert_approx_eq(sp_hf.anova.terms[1].p_value.unwrap(), 1.576_705_607_801e-17, 1e-18, 1e-4, "Split-plot Time p_HF");
    assert_approx_eq(sp_unc.anova.terms[2].p_value.unwrap(), 1.109_537_769_050e-6, 1e-9, 1e-4, "Split-plot Int p_unc");
    assert_approx_eq(sp_gg.anova.terms[2].p_value.unwrap(), 5.764_717_000_559e-4, 1e-7, 1e-4, "Split-plot Int p_GG");
    assert_approx_eq(sp_hf.anova.terms[2].p_value.unwrap(), 4.637_097_027_772e-4, 1e-7, 1e-4, "Split-plot Int p_HF");

    // Transpose symmetry for `RowFactorMatched` (4 rows repeated within 2 column groups).
    let mut row_matched = Vec::with_capacity(10);
    for s in 0..10 {
        let mut mat = vec![vec![0.0_f64; 2]; 4];
        for t in 0..4 {
            mat[t][0] = g0[s][t];
            mat[t][1] = g1[s][t];
        }
        row_matched.push(mat);
    }
    let rm_row = fit_rm_two_way_anova_with_sphericity(
        &row_matched,
        "Time",
        "Group",
        RepeatedMeasuresDesign::RowFactorMatched,
        RmSphericityCorrection::GreenhouseGeisser,
    )
    .unwrap();
    let sph_r = rm_row.sphericities.row.as_ref().unwrap();
    assert_approx_eq(sph_r.greenhouse_geisser_eps, 4.320_567_649_415_32e-1, 1e-9, 1e-9, "RowFactorMatched GG eps");
    assert_approx_eq(rm_row.anova.terms[0].p_value.unwrap(), 7.252_830_749_460e-17, 1e-18, 1e-4, "RowFactorMatched Time p_GG");
    assert_approx_eq(rm_row.anova.terms[1].p_value.unwrap(), 4.197_706_552_045e-10, 1e-13, 1e-4, "RowFactorMatched Group p unchanged");
}

// ============================================================================
// R3.5 #4: ANOVA Effect Sizes (Multi-way Guard, Cohen's f = +inf, KW eps^2 = H/(N-1))
// ============================================================================

#[test]
fn r35_4_anova_effect_sizes_multiway_cohens_f_inf_and_kw_eps2() {
    // 1. Multi-way guard: when SS_effect + SS_error != SS_total, 6-arg API must NOT reuse partial-eta^2 CI
    //    for eta_squared_ci and must NOT silently assume N = df1 + df2 + 1 for partial_eta_squared_ci.
    let multi = compute_anova_effect_sizes(25.0, 60.0, 150.0, 2.0, 24.0, ConfidenceLevel::DEFAULT_95);
    assert!(multi.eta_squared_ci.is_none(), "multi-way eta_squared_ci must be None, got {:?}", multi.eta_squared_ci);
    assert!(multi.partial_eta_squared_ci.is_none(), "6-arg multi-way partial_eta_squared_ci without N must be None");

    // With explicit `total_n` (`compute_anova_effect_sizes_for_n`), Steiger (2004) partial-eta^2 CI uses true N,
    // not `df1 + df2 + 1 = 27`, while keeping `eta_squared_ci = None` for multi-way designs.
    let multi_n30 = compute_anova_effect_sizes_for_n(25.0, 60.0, 150.0, 2.0, 24.0, 30, ConfidenceLevel::DEFAULT_95);
    assert!(multi_n30.eta_squared_ci.is_none());
    let (p_lo30, p_hi30) = multi_n30.partial_eta_squared_ci.unwrap();
    assert_approx_eq(p_lo30, 0.011_009_188_030_756, 1e-6, 5e-5, "Steiger N=30 lo");
    assert_approx_eq(p_hi30, 0.471_085_759_284_659, 1e-6, 5e-5, "Steiger N=30 hi");

    let multi_n36 = compute_anova_effect_sizes_for_n(25.0, 60.0, 150.0, 2.0, 24.0, 36, ConfidenceLevel::DEFAULT_95);
    assert!(multi_n36.eta_squared_ci.is_none());
    let (p_lo36, p_hi36) = multi_n36.partial_eta_squared_ci.unwrap();
    assert_approx_eq(p_lo36, 0.009_191_314_696_754_7, 1e-6, 5e-5, "Steiger N=36 lo");
    assert_approx_eq(p_hi36, 0.426_020_128_451_388, 1e-6, 5e-5, "Steiger N=36 hi");

    // 2. Cohen's f when partial_eta^2 == 1.0 (SS_error == 0, SS_effect > 0) must be +infinity, not 0.0.
    let perfect = compute_anova_effect_sizes(40.0, 0.0, 40.0, 2.0, 15.0, ConfidenceLevel::DEFAULT_95);
    assert_approx_eq(perfect.partial_eta_squared, 1.0, 1e-12, 0.0, "perfect partial eta2");
    assert!(perfect.cohens_f.is_infinite() && perfect.cohens_f.is_sign_positive(), "expected +inf Cohen's f, got {}", perfect.cohens_f);

    // 3. Kruskal-Wallis rank epsilon-squared = H / (N - 1) (Tomczak & Tomczak 2014, Eq. 5).
    let g1 = [12.1, 14.3, 14.3, 15.0, 16.2];
    let g2 = [18.5, 19.0, 19.0, 21.4, 22.0];
    let g3 = [15.0, 16.8, 17.2, 18.5, 20.1];
    let kw = kruskal_wallis_test(&[&g1, &g2, &g3]).unwrap();
    let expected_eps2 = 10.258_273_381_295 / 14.0;
    assert_approx_eq(kw.epsilon_squared, expected_eps2, 1e-10, 1e-10, "KW epsilon^2 = H/(N-1)");
}

// ============================================================================
// R3.5 #8: Minor Fixes (Bartlett Zero-Variance, KW All-Tied, Aliased Factorial df = 0)
// ============================================================================

#[test]
fn r35_8_minor_bartlett_kw_all_tied_and_aliased_factorial_df0() {
    // 1. Bartlett: one zero-variance group + one non-zero variance group -> chi2 = +inf, p = 0.0;
    //    when ALL groups have zero variance -> chi2 = 0.0, p = 1.0.
    let zero_var = [5.0, 5.0, 5.0, 5.0];
    let pos_var = [2.0, 4.0, 6.0, 8.0];
    let vh = variance_homogeneity_tests(&[&zero_var, &pos_var]).unwrap();
    assert!(vh.bartlett_chi2.is_infinite() && vh.bartlett_chi2.is_sign_positive(), "expected +inf Bartlett chi2, got {}", vh.bartlett_chi2);
    assert_eq!(vh.bartlett_p, 0.0, "expected Bartlett p == 0.0");

    let zero_var2 = [9.0, 9.0, 9.0, 9.0];
    let vh_all_zero = variance_homogeneity_tests(&[&zero_var, &zero_var2]).unwrap();
    assert_eq!(vh_all_zero.bartlett_chi2, 0.0);
    assert_eq!(vh_all_zero.bartlett_p, 1.0);

    // 2. Kruskal-Wallis when all observations are tied -> tie_correction = 0.0 (not 1.0), H = 0.0, p = 1.0.
    let t1 = [7.0, 7.0, 7.0];
    let t2 = [7.0, 7.0, 7.0];
    let t3 = [7.0, 7.0, 7.0];
    let kw_tied = kruskal_wallis_test(&[&t1, &t2, &t3]).unwrap();
    assert_eq!(kw_tied.tie_correction, 0.0, "all-tied KW tie_correction must be 0.0, got {}", kw_tied.tie_correction);
    assert_eq!(kw_tied.h_statistic, 0.0);
    assert_eq!(kw_tied.p_value, 1.0);
    assert_eq!(kw_tied.epsilon_squared, 0.0);

    // 3a. Aliased 2x2 factorial missing cell (1, 1).
    let obs_2x2 = FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B")],
        factor_levels: vec![vec![String::from("A0"), String::from("A1")], vec![String::from("B0"), String::from("B1")]],
        rows: vec![
            (vec![0, 0], 10.0),
            (vec![0, 0], 11.0),
            (vec![0, 0], 12.0),
            (vec![0, 0], 13.0),
            (vec![0, 1], 20.0),
            (vec![0, 1], 21.0),
            (vec![0, 1], 22.0),
            (vec![0, 1], 23.0),
            (vec![1, 0], 18.0),
            (vec![1, 0], 19.0),
            (vec![1, 0], 20.0),
            (vec![1, 0], 21.0),
        ],
    };
    let res_type1 = fit_factorial_anova(&obs_2x2, 2, SumOfSquaresType::TypeI).unwrap();
    let ab1 = find_term(&res_type1.terms, "A × B");
    assert_eq!(ab1.df, 0.0, "aliased A×B Type I df must be 0.0");
    assert_eq!(ab1.ss, 0.0);
    assert_eq!(ab1.ms, 0.0);
    assert!(ab1.f_statistic.is_none());
    assert!(ab1.p_value.is_none());

    let res_type3 = fit_factorial_anova(&obs_2x2, 2, SumOfSquaresType::TypeIII).unwrap();
    for term in &res_type3.terms {
        assert_eq!(term.df, 0.0, "Rank-deficient 2x2 missing-cell Type III drop1 df for {} must be 0.0", term.name);
        assert!(term.f_statistic.is_none() && term.p_value.is_none());
    }

    // 3b. Unbalanced 2x3 with missing cell (1, 2) against R `drop1(lm(y ~ A * B))` and `anova(lm(y ~ A * B))`:
    // Type III drops A to df=0 while retaining testable 1-df tests for B and A×B; Type I tests A(df=1), B(df=2), A×B(df=1).
    let obs_2x3 = FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B")],
        factor_levels: vec![vec![String::from("A0"), String::from("A1")], vec![String::from("B0"), String::from("B1"), String::from("B2")]],
        rows: vec![
            (vec![0, 0], 10.0),
            (vec![0, 0], 11.0),
            (vec![0, 0], 12.0),
            (vec![0, 0], 13.0),
            (vec![0, 1], 14.0),
            (vec![0, 1], 16.0),
            (vec![0, 1], 15.0),
            (vec![0, 1], 17.0),
            (vec![0, 2], 20.0),
            (vec![0, 2], 21.0),
            (vec![0, 2], 22.0),
            (vec![0, 2], 23.0),
            (vec![1, 0], 18.0),
            (vec![1, 0], 19.0),
            (vec![1, 0], 20.0),
            (vec![1, 0], 21.0),
            (vec![1, 1], 25.0),
            (vec![1, 1], 27.0),
            (vec![1, 1], 26.0),
            (vec![1, 1], 28.0),
        ],
    };
    let r23_t3 = fit_factorial_anova(&obs_2x3, 2, SumOfSquaresType::TypeIII).unwrap();
    let t3_a = find_term(&r23_t3.terms, "A");
    assert_eq!(t3_a.df, 0.0);
    assert!(t3_a.f_statistic.is_none() && t3_a.p_value.is_none());
    let t3_b = find_term(&r23_t3.terms, "B");
    assert_eq!(t3_b.df, 1.0);
    assert_approx_eq(t3_b.ss, 121.0, 1e-8, 1e-8, "2x3 T3 B ss");
    assert_approx_eq(t3_b.f_statistic.unwrap(), 72.6, 1e-8, 1e-8, "2x3 T3 B F");
    assert_approx_eq(t3_b.p_value.unwrap(), 3.928_298_951_685_58e-7, 1e-11, 1e-8, "2x3 T3 B P");
    let t3_ab = find_term(&r23_t3.terms, "A × B");
    assert_eq!(t3_ab.df, 1.0);
    assert_approx_eq(t3_ab.ss, 9.0, 1e-8, 1e-8, "2x3 T3 A×B ss");
    assert_approx_eq(t3_ab.f_statistic.unwrap(), 5.4, 1e-8, 1e-8, "2x3 T3 A×B F");
    assert_approx_eq(t3_ab.p_value.unwrap(), 0.034_594_159_085_416, 1e-9, 1e-8, "2x3 T3 A×B P");

    let r23_t1 = fit_factorial_anova(&obs_2x3, 2, SumOfSquaresType::TypeI).unwrap();
    let t1_a = find_term(&r23_t1.terms, "A");
    assert_eq!(t1_a.df, 1.0);
    assert_approx_eq(t1_a.ss, 224.133_333_333_333, 1e-8, 1e-8, "2x3 T1 A ss");
    assert_approx_eq(t1_a.f_statistic.unwrap(), 134.48, 1e-8, 1e-8, "2x3 T1 A F");
    let t1_b = find_term(&r23_t1.terms, "B");
    assert_eq!(t1_b.df, 2.0);
    assert_approx_eq(t1_b.ss, 291.666_666_666_667, 1e-8, 1e-8, "2x3 T1 B ss");
    assert_approx_eq(t1_b.f_statistic.unwrap(), 87.5, 1e-8, 1e-8, "2x3 T1 B F");
    let t1_ab = find_term(&r23_t1.terms, "A × B");
    assert_eq!(t1_ab.df, 1.0);
    assert_approx_eq(t1_ab.ss, 9.0, 1e-8, 1e-8, "2x3 T1 A×B ss");
}

// ============================================================================
// Issue #73: Unbalanced-Group-Size Split-Plot RM Two-Way ANOVA & Additive Model
// ============================================================================

#[test]
fn issue_73_unbalanced_split_plot_rm_two_way_type3_sphericity_and_additive_oracle() {
    // D1 Provenance: R 4.5.2 multivariate Type III linear model (`lm(Y ~ G)` with `contr.sum`
    // and orthonormal Helmert within-subject contrasts; Maxwell, Delaney & Kelley 2017, Ch. 12;
    // Greenhouse & Geisser 1959; Huynh & Feldt 1976; Lecoutre 1991) on G = 3 unequal between-subject
    // groups n = [4, 6, 5] (N = 15, df_subj = 12) x W = 4 complete within-subject levels (df_err = 36).
    let g0: Vec<Vec<f64>> =
        vec![vec![10.2, 12.8, 16.5, 22.1], vec![11.4, 13.5, 15.9, 25.0], vec![9.1, 11.2, 17.4, 19.3], vec![12.7, 14.9, 16.2, 26.4]];
    let g1: Vec<Vec<f64>> = vec![
        vec![14.1, 17.5, 22.0, 31.2],
        vec![15.3, 18.2, 23.1, 29.4],
        vec![13.0, 16.1, 20.8, 34.5],
        vec![16.4, 19.3, 24.0, 30.1],
        vec![14.6, 17.8, 22.5, 35.8],
        vec![17.2, 20.4, 24.9, 32.0],
    ];
    let g2: Vec<Vec<f64>> = vec![
        vec![18.5, 20.1, 22.4, 26.8],
        vec![19.2, 21.3, 23.0, 27.5],
        vec![17.6, 19.4, 21.8, 25.2],
        vec![20.1, 22.0, 24.5, 29.1],
        vec![18.9, 20.8, 22.9, 26.4],
    ];
    let groups = vec![g0, g1, g2];

    let unc = fit_rm_two_way_split_plot_anova(
        &groups,
        ["Group", "Time"],
        RepeatedMeasuresDesign::ColumnFactorMatched,
        SumOfSquaresType::TypeIII,
        true,
        RmSphericityCorrection::None,
    )
    .unwrap();
    let gg = fit_rm_two_way_split_plot_anova(
        &groups,
        ["Group", "Time"],
        RepeatedMeasuresDesign::ColumnFactorMatched,
        SumOfSquaresType::TypeIII,
        true,
        RmSphericityCorrection::GreenhouseGeisser,
    )
    .unwrap();
    let hf = fit_rm_two_way_split_plot_anova(
        &groups,
        ["Group", "Time"],
        RepeatedMeasuresDesign::ColumnFactorMatched,
        SumOfSquaresType::TypeIII,
        true,
        RmSphericityCorrection::HuynhFeldt,
    )
    .unwrap();

    // Verify exact Type III SS, F, residual MS, and sphericity (Mauchly, GG, Lecoutre HF).
    assert!(unc.anova.unbalanced);
    assert_approx_eq(unc.anova.terms[0].ss, 467.748_5, 1e-9, 1e-9, "split-plot SS_G");
    assert_approx_eq(unc.anova.terms[0].f_statistic.unwrap(), 45.047_407_070_532, 1e-8, 1e-8, "split-plot F_G");
    assert_approx_eq(unc.anova.terms[0].p_value.unwrap(), 2.636_728_291_682e-6, 1e-11, 1e-5, "split-plot P_G");
    assert_approx_eq(gg.anova.terms[0].p_value.unwrap(), 2.636_728_291_682e-6, 1e-11, 1e-5, "between-subject P unchanged by GG");

    assert_approx_eq(unc.anova.terms[1].ss, 1_295.917_022_522_523, 1e-8, 1e-9, "split-plot Type III SS_W");
    assert_approx_eq(unc.anova.terms[1].f_statistic.unwrap(), 228.532_722_698_769, 1e-7, 1e-8, "split-plot F_W");
    assert_approx_eq(unc.anova.terms[1].p_value.unwrap(), 1.748_189_210_414e-23, 1e-25, 1e-4, "split-plot P_W_unc");
    assert_approx_eq(gg.anova.terms[1].p_value.unwrap(), 6.390_554_042_650e-11, 1e-13, 1e-4, "split-plot P_W_gg");
    assert_approx_eq(hf.anova.terms[1].p_value.unwrap(), 1.907_593_573_645e-11, 1e-13, 1e-4, "split-plot P_W_hf");

    assert_approx_eq(unc.anova.terms[2].ss, 128.251_5, 1e-9, 1e-9, "split-plot SS_GW");
    assert_approx_eq(unc.anova.terms[2].f_statistic.unwrap(), 11.308_464_961_803, 1e-8, 1e-8, "split-plot F_GW");
    assert_approx_eq(unc.anova.terms[2].p_value.unwrap(), 4.480_470_539_554e-7, 1e-11, 1e-4, "split-plot P_GW_unc");
    assert_approx_eq(gg.anova.terms[2].p_value.unwrap(), 6.216_876_351_287e-4, 1e-8, 1e-4, "split-plot P_GW_gg");
    assert_approx_eq(hf.anova.terms[2].p_value.unwrap(), 4.572_593_004_597e-4, 1e-8, 1e-4, "split-plot P_GW_hf");

    assert_approx_eq(unc.anova.ss_residual, 68.047_166_666_667, 1e-8, 1e-9, "split-plot SS_err");
    assert_approx_eq(unc.anova.df_residual, 36.0, 1e-12, 0.0, "split-plot df_err");
    assert_approx_eq(unc.anova.ms_residual, 1.890_199_074_074, 1e-9, 1e-9, "split-plot MS_err");

    let sph = unc.sphericities.column.as_ref().unwrap();
    assert_approx_eq(sph.mauchly_w, 0.008_782_328_980, 1e-8, 1e-8, "split-plot Mauchly W");
    assert_approx_eq(sph.mauchly_chi2, 50.769_868_546_983, 1e-6, 1e-6, "split-plot Mauchly chi2");
    assert_approx_eq(sph.mauchly_p, 9.640_333_157_903e-10, 1e-13, 1e-4, "split-plot Mauchly p");
    assert_approx_eq(sph.greenhouse_geisser_eps, 0.414_128_339_749, 1e-9, 1e-9, "split-plot GG eps");
    assert_approx_eq(sph.huynh_feldt_eps, 0.438_480_254_082, 1e-9, 1e-9, "split-plot HF Lecoutre eps");

    // Scale invariance at 1e-8.
    let scaled: Vec<Vec<Vec<f64>>> = groups.iter().map(|g| g.iter().map(|s| s.iter().map(|&v| v * 1e-8).collect()).collect()).collect();
    let gg_sc = fit_rm_two_way_split_plot_anova(
        &scaled,
        ["Group", "Time"],
        RepeatedMeasuresDesign::ColumnFactorMatched,
        SumOfSquaresType::TypeIII,
        true,
        RmSphericityCorrection::GreenhouseGeisser,
    )
    .unwrap();
    assert_approx_eq(gg_sc.anova.terms[1].f_statistic.unwrap(), gg.anova.terms[1].f_statistic.unwrap(), 1e-7, 1e-7, "1e-8 scale F_W");
    assert_approx_eq(gg_sc.sphericities.column.as_ref().unwrap().greenhouse_geisser_eps, sph.greenhouse_geisser_eps, 1e-9, 1e-9, "1e-8 scale GG");
    assert_approx_eq(gg_sc.anova.terms[2].p_value.unwrap(), gg.anova.terms[2].p_value.unwrap(), 1e-8, 1e-4, "1e-8 scale P_GW_gg");

    // Additive model (include_interaction = false) pools SS_GW into residual: df_err = (N-1)*(W-1) = 42.
    let add = fit_rm_two_way_split_plot_anova(
        &groups,
        ["Group", "Time"],
        RepeatedMeasuresDesign::ColumnFactorMatched,
        SumOfSquaresType::TypeIII,
        false,
        RmSphericityCorrection::None,
    )
    .unwrap();
    assert_eq!(add.anova.terms.len(), 2);
    assert_approx_eq(add.anova.terms[1].ss, 1_399.813_833_333_333, 1e-8, 1e-9, "additive SS_W");
    assert_approx_eq(add.anova.ss_residual, 196.298_666_666_667, 1e-8, 1e-9, "additive SS_err");
    assert_approx_eq(add.anova.df_residual, 42.0, 1e-12, 0.0, "additive df_err");
    assert_approx_eq(add.anova.terms[1].f_statistic.unwrap(), 99.834_573_507_037, 1e-7, 1e-8, "additive F_W");

    // Sheet-level GroupedTable dispatch with unequal subcolumns (6 subcolumns, blank slots in G1 & G3)
    // must take the classical split-plot path (NOT REML), emit rm-sphericity, and verify post-hoc SE/df
    // plus RowFactorMatched transpose symmetry.
    let mut grp_col = GroupedTable::new(SubcolumnMode::RawReplicates(6), 4, 3).unwrap();
    grp_col.row_factor_title = String::from("Group");
    grp_col.column_factor_title = String::from("Time");
    grp_col.row_titles = vec![String::from("G1"), String::from("G2"), String::from("G3")];
    for w in 0..4 {
        grp_col.data_sets[w].title = format!("T{}", w + 1);
        for (g_i, grp) in groups.iter().enumerate() {
            for (s_i, subj) in grp.iter().enumerate() {
                grp_col.data_sets[w].cells[g_i][s_i] = Cell::number(subj[w]);
            }
        }
    }
    let sh_col = DataSheet::new(SheetId::new("g-sp-col"), "SplitPlot ColMatched", TableArchetype::Grouped(grp_col)).unwrap();
    let fac_col = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        sphericity_correction: RmSphericityCorrection::GreenhouseGeisser,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Bonferroni,
            threshold: 0.05,
            report_confidence_intervals: true,
            ..MultiplicitySpec::default()
        },
        ..FactorialAnovaSpec::default()
    };
    let res_col = run_factorial_anova(
        &sh_col,
        &AnalysisSpec::new("an-sp-col", SheetId::new("g-sp-col"), AnalysisKind::FactorialAnova(fac_col.clone())).unwrap(),
        &fac_col,
    )
    .unwrap();
    let scalar_col = |k: &str| res_col.scalars.iter().find(|s| s.key == k).unwrap().value;
    assert!(res_col.tabs.iter().any(|t| t.id == "rm-sphericity"), "classical split-plot must emit rm-sphericity tab");
    assert!(
        res_col
            .checklist
            .iter()
            .any(|c| c.assumption == "Repeated-measures estimator"
                && c.diagnostic_metric.as_deref().is_some_and(|m| m.contains("Classical split-plot"))),
        "unequal-group complete split-plot must not fall back to REML"
    );
    assert_approx_eq(scalar_col("Group: F"), 45.047_407_070_532, 1e-8, 1e-8, "sheet ColMatched F_G");
    assert_approx_eq(scalar_col("Time: F"), 228.532_722_698_769, 1e-7, 1e-8, "sheet ColMatched F_W");
    assert_approx_eq(scalar_col("Time: P"), 6.390_554_042_650e-11, 1e-13, 1e-4, "sheet ColMatched P_W_gg");
    assert_approx_eq(scalar_col("Group × Time: P"), 6.216_876_351_287e-4, 1e-8, 1e-4, "sheet ColMatched P_GW_gg");
    assert_approx_eq(scalar_col("MS residual"), 1.890_199_074_074, 1e-9, 1e-9, "sheet ColMatched MS_err");

    // Verify exact post-hoc SE and Satterthwaite df for unequal-n contrasts (Maxwell, Delaney & Kelley 2017, Ch. 12).
    let cell_num = |c: &ResultCell| match c {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected number, got {other:?}"),
    };
    let find_mc = |tab_id: &str, a: &str, b: &str| {
        let tab = res_col.tabs.iter().find(|t| t.id == tab_id).unwrap_or_else(|| panic!("missing tab `{tab_id}`"));
        let idx = tab
            .row_labels
            .iter()
            .position(|l| l.contains(a) && l.contains(b))
            .unwrap_or_else(|| panic!("missing `{a}` vs `{b}` in {tab_id}: {:?}", tab.row_labels));
        (cell_num(&tab.rows[idx][0]), cell_num(&tab.rows[idx][1]), cell_num(&tab.rows[idx][3]))
    };
    let (d_g, se_g, df_g) = find_mc("multiple-comparisons-factor-a", "G1", "G2");
    assert_approx_eq(d_g, -6.179_166_666_667, 1e-9, 1e-9, "G1-G2 marginal diff");
    assert_approx_eq(se_g, 0.735_395_026_436, 1e-9, 1e-9, "G1-G2 marginal SE");
    assert_approx_eq(df_g, 12.0, 1e-12, 0.0, "G1-G2 marginal df");

    let (d_w, se_w, df_w) = find_mc("multiple-comparisons-factor-b", "T1", "T4");
    assert_approx_eq(d_w, -12.518_888_888_889, 1e-9, 1e-9, "T1-T4 marginal diff");
    assert_approx_eq(se_w, 0.508_947_227_639, 1e-9, 1e-9, "T1-T4 marginal SE (harmonic n_h)");
    assert_approx_eq(df_w, 36.0, 1e-12, 0.0, "T1-T4 marginal df");

    let (d_sw, se_sw, df_sw) = find_mc("multiple-comparisons-simple-b-within-a", "G1 | T1", "G1 | T4");
    assert_approx_eq(d_sw, -12.35, 1e-9, 1e-9, "T1-T4 @ G1 diff");
    assert_approx_eq(se_sw, 0.972_162_299_741, 1e-9, 1e-9, "T1-T4 @ G1 SE");
    assert_approx_eq(df_sw, 36.0, 1e-12, 0.0, "T1-T4 @ G1 df");

    let (d_sb, se_sb, df_sb) = find_mc("multiple-comparisons-simple-a-within-b", "T1 | G1", "T1 | G2");
    assert_approx_eq(d_sb, -4.25, 1e-9, 1e-9, "G1-G2 @ T1 diff");
    assert_approx_eq(se_sb, 1.063_716_623_709, 1e-9, 1e-9, "G1-G2 @ T1 SE");
    assert_approx_eq(df_sb, 37.583_823_880_433, 1e-8, 1e-8, "G1-G2 @ T1 Satterthwaite df");

    // Transpose symmetry: RowFactorMatched on transposed GroupedTable (4 rows x 3 columns x 6 subcolumns).
    let mut grp_row = GroupedTable::new(SubcolumnMode::RawReplicates(6), 3, 4).unwrap();
    grp_row.row_factor_title = String::from("Time");
    grp_row.column_factor_title = String::from("Group");
    grp_row.row_titles = (1..=4).map(|w| format!("T{w}")).collect();
    for (g_i, grp) in groups.iter().enumerate() {
        grp_row.data_sets[g_i].title = format!("G{}", g_i + 1);
        for (s_i, subj) in grp.iter().enumerate() {
            for (w, &v) in subj.iter().enumerate() {
                grp_row.data_sets[g_i].cells[w][s_i] = Cell::number(v);
            }
        }
    }
    let sh_row = DataSheet::new(SheetId::new("g-sp-row"), "SplitPlot RowMatched", TableArchetype::Grouped(grp_row)).unwrap();
    let fac_row = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::RowFactorMatched,
        sphericity_correction: RmSphericityCorrection::GreenhouseGeisser,
        ..FactorialAnovaSpec::default()
    };
    let res_row = run_factorial_anova(
        &sh_row,
        &AnalysisSpec::new("an-sp-row", SheetId::new("g-sp-row"), AnalysisKind::FactorialAnova(fac_row.clone())).unwrap(),
        &fac_row,
    )
    .unwrap();
    let scalar_row = |k: &str| res_row.scalars.iter().find(|s| s.key == k).unwrap().value;
    assert_approx_eq(scalar_row("Time: F"), scalar_col("Time: F"), 1e-10, 1e-10, "RowMatched transpose F_W");
    assert_approx_eq(scalar_row("Group: F"), scalar_col("Group: F"), 1e-10, 1e-10, "RowMatched transpose F_G");
    assert_approx_eq(scalar_row("Time: P"), scalar_col("Time: P"), 1e-12, 1e-6, "RowMatched transpose P_W_gg");
    assert_approx_eq(scalar_row("Time × Group: P"), scalar_col("Group × Time: P"), 1e-10, 1e-6, "RowMatched transpose P_GW_gg");
}

#[test]
fn issue_73_negative_icc_unequal_split_plot_keeps_separate_strata_and_partial_subject_uses_reml() {
    // D1 Provenance: R 4.5.2 exact split-plot Type III GLM on negative-ICC unequal group sizes
    // n = [6, 4], W = 2 (MS_Subj(G) = 0.0059375 << MS_err_within = 1794.7853125). Classical
    // split-plot ANOVA keeps the two error strata separate rather than clamping sigma^2_subj = 0.
    let g0: [[f64; 2]; 6] = [[30.0, 70.2], [69.8, 30.1], [25.2, 74.9], [75.1, 24.8], [40.0, 60.1], [60.2, 39.9]];
    let g1: [[f64; 2]; 4] = [[20.1, 84.9], [85.0, 20.2], [15.0, 90.1], [90.2, 14.9]];

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(6), 2, 2).unwrap();
    grp.row_factor_title = String::from("Group");
    grp.column_factor_title = String::from("Condition");
    for (s, row) in g0.iter().enumerate() {
        grp.data_sets[0].cells[0][s] = Cell::number(row[0]);
        grp.data_sets[1].cells[0][s] = Cell::number(row[1]);
    }
    for (s, row) in g1.iter().enumerate() {
        grp.data_sets[0].cells[1][s] = Cell::number(row[0]);
        grp.data_sets[1].cells[1][s] = Cell::number(row[1]);
    }
    let sheet = DataSheet::new(SheetId::new("g-neg-icc"), "NegICC SplitPlot", TableArchetype::Grouped(grp.clone())).unwrap();
    let fac = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        fit_reml_on_missing: true,
        ..FactorialAnovaSpec::default()
    };
    let res = run_factorial_anova(
        &sheet,
        &AnalysisSpec::new("an-neg-icc", SheetId::new("g-neg-icc"), AnalysisKind::FactorialAnova(fac.clone())).unwrap(),
        &fac,
    )
    .unwrap();
    let scalar = |k: &str| res.scalars.iter().find(|s| s.key == k).unwrap().value;
    assert_approx_eq(scalar("Group: F"), 5_154.189_473_683_564, 1e-6, 1e-8, "neg-ICC separate-strata F_G");
    assert_approx_eq(scalar("Group: P"), 1.578_162_255_336e-12, 1e-15, 1e-4, "neg-ICC separate-strata P_G");
    assert_approx_eq(scalar("Condition: F"), 0.000_006_686_036, 1e-8, 1e-5, "neg-ICC F_W");
    assert_approx_eq(scalar("MS residual"), 1_794.785_312_5, 1e-7, 1e-9, "neg-ICC MS_err_within");

    // Dropping a single within-subject cell creates a genuinely partial subject, which must
    // upgrade to mixed-effects REML when fit_reml_on_missing is true and error when false.
    let mut grp_partial = grp;
    grp_partial.data_sets[1].cells[0][5] = Cell::empty();
    let sh_partial = DataSheet::new(SheetId::new("g-neg-part"), "NegICC Partial", TableArchetype::Grouped(grp_partial)).unwrap();
    let res_reml = run_factorial_anova(
        &sh_partial,
        &AnalysisSpec::new("an-neg-reml", SheetId::new("g-neg-part"), AnalysisKind::FactorialAnova(fac.clone())).unwrap(),
        &fac,
    )
    .unwrap();
    assert!(
        res_reml.checklist.iter().any(|c| c.assumption == "Missing-value mixed-effects REML"),
        "genuinely partial subject must still dispatch to mixed-effects REML"
    );
    let fac_no_reml = FactorialAnovaSpec { fit_reml_on_missing: false, ..fac };
    let err = run_factorial_anova(
        &sh_partial,
        &AnalysisSpec::new("an-neg-err", SheetId::new("g-neg-part"), AnalysisKind::FactorialAnova(fac_no_reml.clone())).unwrap(),
        &fac_no_reml,
    )
    .unwrap_err();
    assert!(matches!(err, curvebook_stats::StatsError::InsufficientSampleSize { .. }), "expected InsufficientSampleSize, got {err:?}");
}
