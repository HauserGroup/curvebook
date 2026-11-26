#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression and D1 primary-literature oracle tests for all 20 AUD-05 ANOVA remediation findings:
//! `AUD-05-13`, `AUD-05-14`, `AUD-05-15`, `AUD-05-17`, `AUD-05-18`, `AUD-05-19`, `AUD-05-20`,
//! `AUD-05-21`, `AUD-05-22`, `AUD-05-23`, `AUD-05-24`, `AUD-05-40`, `AUD-05-41`, `AUD-05-42`,
//! `AUD-05-43`, `AUD-05-55`, `AUD-05-56`, `AUD-05-57`, `AUD-05-58`, and `AUD-05-59`.
//!
//! # References
//! - Bartlett, M. S. (1937). Properties of sufficiency and statistical tests. *Proceedings of the
//!   Royal Society of London A*, 160(901), 268-282.
//! - Fai, A. H.-T., & Cornelius, P. L. (1996). Approximate F-tests of multiple degree of freedom
//!   hypotheses in generalized least squares analyses of unbalanced split-plot experiments.
//!   *Journal of Statistical Computation and Simulation*, 54(4), 363-378.
//! - Gilmour, A. R., Thompson, R., & Cullis, B. R. (1995). Average Information REML: an efficient
//!   algorithm for variance parameter estimation in linear mixed models. *Biometrics*, 51(4),
//!   1440-1450.
//! - Greenhouse, S. W., & Geisser, S. (1959). On methods in the analysis of profile data.
//!   *Psychometrika*, 24(2), 95-112.
//! - Kenward, M. G., & Roger, J. H. (1997). Small sample inference for fixed effects from
//!   restricted maximum likelihood. *Biometrics*, 53(3), 983-997.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Nakagawa, S., & Schielzeth, H. (2013). A general and simple method for obtaining R^2 from
//!   generalized linear mixed-effects models. *Methods in Ecology and Evolution*, 4(2), 133-142.
//! - Patterson, H. D., & Thompson, R. (1971). Recovery of inter-block information when block sizes
//!   are unequal. *Biometrika*, 58(3), 545-554.
//! - Pinheiro, J. C., & Bates, D. M. (1996). Unconstrained parametrizations for variance-covariance
//!   matrices. *Statistics and Computing*, 6(3), 289-296.
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.
//! - Steiger, J. H. (2004). Beyond the F test: effect size confidence intervals and tests of close
//!   fit in the analysis of variance and contrast analysis. *Psychological Methods*, 9(2), 164-182.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, DataSheet, FactorialAnovaSpec, GroupedTable,
    MixedModelDfMethod, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, NestedAnalysisSpec, NestedTable, OneWayAnovaSpec, OneWayMethod,
    RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultSheet, ResultTab, SheetId, SubcolumnMode, SumOfSquaresType, TableArchetype,
    VariableRole,
};
use curvebook_math::dist::{f_sf, max_studentized_range_sf, student_t_two_sided_isf, student_t_two_sided_p};
use curvebook_math::linalg::{DenseMatrix, sym_eigen};
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::StatsError;
use curvebook_stats::anova::{
    FactorialObservations, RmSphericityCorrection, compute_anova_effect_sizes_for_n, fit_factorial_anova, fit_multi_vc_reml,
    fit_random_intercept_reml, fit_rm_two_way_anova, fit_rm_two_way_anova_with_sphericity, fit_unstructured_rm_reml, nested_one_way_anova,
    nested_one_way_anova_with_df_method, ordinary_one_way_anova, ordinary_one_way_from_summary, rm_one_way_anova, run_factorial_anova,
    run_nested_analysis, run_one_way_anova,
};
use curvebook_stats::multiplicity::GroupComparisonInput;
use curvebook_testkit::assert_approx_eq;

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tol,
        "{label}: expected {expected:.12e}, got {actual:.12e} (diff={:.3e}, tol={tol:.3e})",
        (actual - expected).abs()
    );
}

fn scalar_val(sheet: &ResultSheet, key: &str) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet
        .tabs
        .iter()
        .find(|t| t.id == id)
        .unwrap_or_else(|| panic!("missing tab `{id}` in {:?}", sheet.tabs.iter().map(|t| &t.id).collect::<Vec<_>>()))
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected Number cell, got {other:?}"),
    }
}

fn cell_p(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::PValue { raw, .. } => *raw,
        other => panic!("expected PValue cell, got {other:?}"),
    }
}

fn cell_ci_finite(cell: &ResultCell) -> (f64, f64) {
    match cell {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } => (*lo, *hi),
        other => panic!("expected finite ConfidenceInterval cell, got {other:?}"),
    }
}

fn make_col_sheet_opts(id: &str, hint: ColumnDesignHint, cols: &[(&str, &[Option<f64>])]) -> DataSheet {
    let n_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut tab = ColumnTable::new(hint, cols.len(), n_rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        tab.data_sets[c].title = (*title).to_owned();
        for (r, val) in vals.iter().enumerate() {
            if let Some(v) = val {
                tab.data_sets[c].cells[r][0] = Cell::number(*v);
            }
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Column(tab)).unwrap()
}

fn make_nested_table(groups: &[(&str, &[&[f64]])]) -> NestedTable {
    let max_sub = groups.iter().map(|(_, s)| s.len()).max().unwrap_or(1).max(1);
    let max_rows = groups.iter().flat_map(|(_, s)| s.iter().map(|r| r.len())).max().unwrap_or(1).max(1);
    let mut t = NestedTable::new(groups.len(), max_sub, max_rows).unwrap();
    for (g, (title, subs)) in groups.iter().enumerate() {
        t.groups[g].title = (*title).to_owned();
        for (b, reps) in subs.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                t.groups[g].cells[r][b] = Cell::number(v);
            }
        }
    }
    t
}

fn make_nested_sheet(id: &str, groups: &[(&str, &[&[f64]])]) -> DataSheet {
    DataSheet::new(SheetId::new(id), id, TableArchetype::Nested(make_nested_table(groups))).unwrap()
}

fn spec_for(id: &str, source: &str, kind: AnalysisKind) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new(id), SheetId::new(source), kind).unwrap()
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

fn sum_contrast_two_way_matrix(a_ids: &[usize], b_ids: &[usize], a_lvls: usize, b_lvls: usize) -> DenseMatrix {
    let da = a_lvls - 1;
    let db = b_lvls - 1;
    let p = 1 + da + db + da * db;
    let mut raw = Vec::with_capacity(a_ids.len() * p);
    for (&a, &b) in a_ids.iter().zip(b_ids.iter()) {
        let mut ca = vec![0.0_f64; da];
        if a == da {
            ca.fill(-1.0);
        } else {
            ca[a] = 1.0;
        }
        let mut cb = vec![0.0_f64; db];
        if b == db {
            cb.fill(-1.0);
        } else {
            cb[b] = 1.0;
        }
        raw.push(1.0);
        raw.extend_from_slice(&ca);
        raw.extend_from_slice(&cb);
        for &va in &ca {
            for &vb in &cb {
                raw.push(va * vb);
            }
        }
    }
    DenseMatrix::from_row_major(a_ids.len(), p, raw).unwrap()
}

fn cluster_g_matrix(cluster_ids: &[usize]) -> (DenseMatrix, usize) {
    let n = cluster_ids.len();
    let mut remap = std::collections::BTreeMap::<usize, usize>::new();
    let mut mapped = Vec::with_capacity(n);
    for &cid in cluster_ids {
        let next = remap.len();
        mapped.push(*remap.entry(cid).or_insert(next));
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
    (DenseMatrix::from_row_major(n, n, g).unwrap(), remap.len())
}

fn lcg_u01(state: &mut u64) -> f64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*state >> 11) as f64) / ((1_u64 << 53) as f64)
}

fn lcg_norm(state: &mut u64) -> f64 {
    let u1 = lcg_u01(state).max(1e-12);
    let u2 = lcg_u01(state);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

// ============================================================================
// AUD-05-13: Unbounded factorial interaction expansion & unchecked allocation
// ============================================================================

#[test]
fn rem_aud05_13_factorial_interaction_expansion_cap() {
    // 1. 25 two-level factors with max_interaction_order = 25 on MultipleVariablesTable (2^25 - 1 terms).
    let n_rows = 30;
    let mut mv25 = MultipleVariablesTable::new(26, n_rows).unwrap();
    mv25.variables[0].title = String::from("Y");
    mv25.variables[0].lock_role(VariableRole::Continuous);
    for r in 0..n_rows {
        mv25.variables[0].cells[r] = Cell::number(10.0 + (r as f64) * 0.5);
    }
    for f in 1..=25 {
        mv25.variables[f].title = format!("F{f}");
        mv25.variables[f].lock_role(VariableRole::Categorical);
        for r in 0..n_rows {
            let lbl = if ((r >> (f % 4)) & 1) == 0 { "L0" } else { "L1" };
            mv25.variables[f].cells[r] = Cell::text(lbl);
        }
    }
    let sheet25 = DataSheet::new("mv25", "MV25", TableArchetype::MultipleVariables(mv25)).unwrap();
    let spec25 =
        FactorialAnovaSpec { response_column: Some(0), multifactor_columns: (1..=25).collect(), max_interaction_order: 25, ..Default::default() };
    let err25 = run_factorial_anova(&sheet25, &spec_for("r25", "mv25", AnalysisKind::FactorialAnova(spec25.clone())), &spec25).unwrap_err();
    assert!(
        matches!(err25, StatsError::InvalidParameter { name: "factorial_terms" | "factorial_design_columns", .. }),
        "expected factorial cap error for 25-factor full order, got {err25:?}"
    );

    // 2. 200 ten-level factors at order 2 (1 + 1_800 + C(200,2)*81 = 1_613_701 columns).
    let n_rows2 = 20;
    let mut mv200 = MultipleVariablesTable::new(201, n_rows2).unwrap();
    mv200.variables[0].title = String::from("Y");
    mv200.variables[0].lock_role(VariableRole::Continuous);
    for r in 0..n_rows2 {
        mv200.variables[0].cells[r] = Cell::number(1.0 + r as f64);
    }
    for f in 1..=200 {
        mv200.variables[f].title = format!("G{f}");
        mv200.variables[f].lock_role(VariableRole::Categorical);
        for r in 0..n_rows2 {
            mv200.variables[f].cells[r] = Cell::text(format!("L{}", (r + f) % 10));
        }
    }
    let sheet200 = DataSheet::new("mv200", "MV200", TableArchetype::MultipleVariables(mv200)).unwrap();
    let spec200 =
        FactorialAnovaSpec { response_column: Some(0), multifactor_columns: (1..=200).collect(), max_interaction_order: 2, ..Default::default() };
    let err200 = run_factorial_anova(&sheet200, &spec_for("r200", "mv200", AnalysisKind::FactorialAnova(spec200.clone())), &spec200).unwrap_err();
    assert!(
        matches!(err200, StatsError::InvalidParameter { name: "factorial_terms" | "factorial_design_columns", .. }),
        "expected factorial cap error for 200x10-level order 2, got {err200:?}"
    );

    // 3. Direct fit_factorial_anova rejects 25 two-level factors upfront.
    let obs = FactorialObservations {
        factor_names: (0..25).map(|i| format!("F{i}")).collect(),
        factor_levels: (0..25).map(|_| vec![String::from("A"), String::from("B")]).collect(),
        rows: (0..30).map(|r| ((0..25).map(|f| (r + f) % 2).collect(), r as f64)).collect(),
    };
    let err_direct = fit_factorial_anova(&obs, 25, SumOfSquaresType::TypeIII).unwrap_err();
    assert!(
        matches!(err_direct, StatsError::InvalidParameter { name: "factorial_terms" | "factorial_design_columns", .. }),
        "expected direct fit_factorial_anova cap error, got {err_direct:?}"
    );

    // 4. REML design builder (build_reml_effect_design) also enforces MAX_FACTORIAL_DESIGN_COLS (100 x 60 = 6000 cols > 4096).
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 60, 100).unwrap();
    for c in 0..60 {
        for r in 0..100 {
            grp.data_sets[c].cells[r][0] = Cell::number((r + c) as f64);
        }
    }
    let grp_sheet = DataSheet::new("g-big", "GBig", TableArchetype::Grouped(grp)).unwrap();
    let grp_spec =
        FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched, mixed_effects_reml: true, ..Default::default() };
    let err_reml_design =
        run_factorial_anova(&grp_sheet, &spec_for("r-gbig", "g-big", AnalysisKind::FactorialAnova(grp_spec.clone())), &grp_spec).unwrap_err();
    assert!(
        matches!(err_reml_design, StatsError::InvalidParameter { name: "factorial_design_columns", .. }),
        "expected REML design cap error on 100x60 grouped table, got {err_reml_design:?}"
    );
}

// ============================================================================
// AUD-05-14: Unstructured REML O(t^4 p^2) Fisher cap & contracted KR oracle
// ============================================================================

#[test]
fn rem_aud05_14_unstructured_reml_fisher_cap_and_contracted_oracle() {
    // 1. t = 40 columns, n = 50 subjects (k_dim = 820 > MAX_UNSTRUCTURED_COV_PARAMS = 300) cleanly errors upfront without OOM.
    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 40, 50).unwrap();
    for c in 0..40 {
        tab.data_sets[c].title = format!("T{c}");
        for r in 0..50 {
            tab.data_sets[c].cells[r][0] = Cell::number(10.0 + 0.1 * (c as f64) + 0.3 * (r as f64) + ((r * 7 + c * 13) % 5) as f64 * 0.05);
        }
    }
    let sheet40 = DataSheet::new("col40", "Col40", TableArchetype::Column(tab)).unwrap();
    let spec40 = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
        mixed_effects_reml: true,
        residual_covariance: ResidualCovariance::Unstructured,
        ..Default::default()
    };
    let err40 = run_one_way_anova(&sheet40, &spec_for("r40", "col40", AnalysisKind::OneWayAnova(spec40.clone())), &spec40).unwrap_err();
    assert!(
        matches!(err40, StatsError::InvalidParameter { name: "unstructured_covariance_params" | "unstructured_fisher_work", .. }),
        "expected upfront Fisher budget error on t=40 unstructured RM, got {err40:?}"
    );

    // 2. t = 65 direct call exceeds MAX_UNSTRUCTURED_TIMEPOINTS = 64 immediately.
    let t65 = 65;
    let c_ids: Vec<usize> = (0..2).flat_map(|s| std::iter::repeat_n(s, t65)).collect();
    let t_ids: Vec<usize> = (0..2).flat_map(|_| 0..t65).collect();
    let y65: Vec<f64> = (0..(2 * t65)).map(|i| (i % 7) as f64 + 1.0).collect();
    let labels65: Vec<String> = (0..t65).map(|i| format!("T{i}")).collect();
    let x65 = DenseMatrix::from_row_major(2 * t65, 1, vec![1.0; 2 * t65]).unwrap();
    let err65 = fit_unstructured_rm_reml(&x65, &y65, &c_ids, &t_ids, &labels65, &[]).unwrap_err();
    assert!(
        matches!(err65, StatsError::InvalidParameter { name: "unstructured_timepoints", .. }),
        "expected unstructured_timepoints cap error on t=65, got {err65:?}"
    );

    // 3. 6 x 8 GroupedTable x 2 subjects (t_obs = 48, n_clusters = 2) triggers SmallSampleRankDeficient fallback
    // skipping Fisher allocation completely (zero OOM).
    let mut g68 = GroupedTable::new(SubcolumnMode::RawReplicates(2), 8, 6).unwrap();
    for c in 0..8 {
        g68.data_sets[c].title = format!("C{c}");
        for r in 0..6 {
            for s in 0..2 {
                g68.data_sets[c].cells[r][s] =
                    Cell::number(5.0 + (r as f64) + 0.5 * (c as f64) + (s as f64) * 1.3 + ((r * 3 + c + s) % 4) as f64 * 0.2);
            }
        }
    }
    for r in 0..6 {
        g68.row_titles[r] = format!("R{r}");
    }
    let sheet68 = DataSheet::new("g68", "G68", TableArchetype::Grouped(g68)).unwrap();
    let spec68 = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        residual_covariance: ResidualCovariance::Unstructured,
        mixed_effects_reml: true,
        ..Default::default()
    };
    let res68 = run_factorial_anova(&sheet68, &spec_for("r68", "g68", AnalysisKind::FactorialAnova(spec68.clone())), &spec68).unwrap();
    assert!(
        res68.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.assumption.contains("Unstructured")),
        "expected SmallSampleRankDeficient warning on 6x8x2 BothFactorsMatched Unstructured"
    );

    // 4. Verify contracted on-the-fly Fisher/KR matches exact analytical Hotelling T^2 D1 oracle (C = 10, t = 4).
    // D1 Provenance: exact Hotelling T^2 identity / R nlme::gls + KR97 on balanced C = 10, t = 4 dataset.
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
    let mut cid = Vec::with_capacity(40);
    let mut tid = Vec::with_capacity(40);
    let mut y = Vec::with_capacity(40);
    for (s, subj) in rows.iter().enumerate() {
        for (t, &val) in subj.iter().enumerate() {
            cid.push(s);
            tid.push(t);
            y.push(val);
        }
    }
    let lbls: Vec<String> = (1..=4).map(|k| format!("T{k}")).collect();
    let x = sum_contrast_one_way_matrix(&tid, 4);
    let fit = fit_unstructured_rm_reml(&x, &y, &cid, &tid, &lbls, &[("Time", 1, 3)]).unwrap();
    assert!(fit.converged);
    assert_eq!(fit.fallback_reason, None);
    assert_approx_eq(fit.fixed_terms[0].f_statistic.unwrap(), 17.066_630_776_420_915, 1e-5, 1e-6, "Contracted Hotelling F_KR");
    assert_approx_eq(fit.satterthwaite_df[0], 9.0, 1e-5, 1e-5, "Contracted nu_Sat");
    assert_approx_eq(fit.kenward_roger_df[0], 7.0, 1e-5, 1e-5, "Contracted Hotelling nu_KR");
    assert_approx_eq(fit.fixed_terms[0].p_value.unwrap(), 0.001_337_46, 1e-6, 1e-4, "Contracted p_KR");
}

// ============================================================================
// AUD-05-15: Checked dense allocations across RM sphericity, multi-VC, nested
// ============================================================================

#[test]
fn rem_aud05_15_checked_dense_allocations_prevent_oom() {
    // 1. RM two-way sphericity: a = 101, b = 100 => k = 10_100 => k * k = 102_010_000 > MAX_MATRIX_ELEMENTS (100_000_000).
    let (a, b, n_subj) = (101_usize, 100_usize, 2_usize);
    let mut rm_data = vec![vec![vec![0.0_f64; b]; a]; n_subj];
    for (si, subj) in rm_data.iter_mut().enumerate() {
        for (ri, row) in subj.iter_mut().enumerate() {
            for (ci, val) in row.iter_mut().enumerate() {
                *val = (ri + ci + si) as f64;
            }
        }
    }
    let err_sph = fit_rm_two_way_anova(&rm_data, "A", "B", RepeatedMeasuresDesign::BothFactorsMatched).unwrap_err();
    assert!(
        matches!(err_sph, StatsError::Math(MathError::InvalidParameter { name: "matrix_elements", .. })),
        "expected matrix_elements guard error in RM two-way sphericity, got {err_sph:?}"
    );

    // 2. Multi-VC REML via GroupedTable (500 rows x 3 cols x 7 subs = 10_500 obs, 1_500 design cols <= 4_096):
    // cluster_ids_to_g_matrix checks n * n = 110_250_000 > MAX_MATRIX_ELEMENTS upfront without allocating 882 MB.
    let mut g_mvc = GroupedTable::new(SubcolumnMode::RawReplicates(7), 3, 500).unwrap();
    for c in 0..3 {
        for r in 0..500 {
            for s in 0..7 {
                g_mvc.data_sets[c].cells[r][s] = Cell::number((r + c + s) as f64);
            }
        }
    }
    let sheet_mvc = DataSheet::new("g-mvc", "GMVC", TableArchetype::Grouped(g_mvc)).unwrap();
    let spec_mvc =
        FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched, mixed_effects_reml: true, ..Default::default() };
    let err_mvc =
        run_factorial_anova(&sheet_mvc, &spec_for("r-gmvc", "g-mvc", AnalysisKind::FactorialAnova(spec_mvc.clone())), &spec_mvc).unwrap_err();
    assert!(
        matches!(err_mvc, StatsError::Math(MathError::InvalidParameter { name: "matrix_elements", .. })),
        "expected matrix_elements guard error in multi-VC REML, got {err_mvc:?}"
    );

    // 3. Nested ANOVA: g_count = 1_000, 2 subclusters, 51 rows => total_obs = 102_000 => 102_000 * 1_000 > MAX_MATRIX_ELEMENTS.
    let mut n_tab = NestedTable::new(1_000, 2, 51).unwrap();
    for (g, grp) in n_tab.groups.iter_mut().enumerate() {
        for (r, row) in grp.cells.iter_mut().enumerate() {
            row[0] = Cell::number((g + r) as f64);
            row[1] = Cell::number((g + r + 1) as f64);
        }
    }
    let err_nested = nested_one_way_anova(&n_tab, false).unwrap_err();
    assert!(
        matches!(err_nested, StatsError::Math(MathError::InvalidParameter { name: "matrix_elements", .. })),
        "expected matrix_elements guard error in nested ANOVA, got {err_nested:?}"
    );
}

// ============================================================================
// AUD-05-17: REML Type III estimability & relabeling invariance on empty cells
// ============================================================================

#[test]
fn rem_aud05_17_reml_empty_cell_type3_estimability_and_relabel_invariance() {
    // 1. 3 x 3 pure-interaction design with cell (2, 2) empty (AUD-05-17 exact failure scenario).
    // Cell means: [[10, -10, 0], [-10, 10, 0], [0, 0, missing]].
    // Every observed marginal mean of A is identically 0; OLS Type III has df_A = 1, tiny F_A.
    let cell_means: [[Option<f64>; 3]; 3] =
        [[Some(10.0), Some(-10.0), Some(0.0)], [Some(-10.0), Some(10.0), Some(0.0)], [Some(0.0), Some(0.0), None]];
    let noise: [f64; 5] = [-0.012, -0.004, 0.001, 0.006, 0.009];

    let build_dataset = |perm_a: [usize; 3]| {
        let mut a_ids = Vec::new();
        let mut b_ids = Vec::new();
        let mut y = Vec::new();
        let mut subj_ids = Vec::new();
        let mut tp_ids = Vec::new();
        let mut rows_obs = Vec::new();
        for a_orig in 0..3 {
            let a_lbl = perm_a[a_orig];
            for b in 0..3 {
                if let Some(mu) = cell_means[a_orig][b] {
                    for (s, &eps) in noise.iter().enumerate() {
                        // Cluster subjects within each Factor A level (split-plot: B is repeated across 3 timepoints)
                        // so unstructured residual covariance across B has 15 subjects x 3 timepoints.
                        let subj = a_orig * 5 + s;
                        let val = mu + eps + 0.002 * (s as f64 - 2.0);
                        a_ids.push(a_lbl);
                        b_ids.push(b);
                        y.push(val);
                        subj_ids.push(subj);
                        tp_ids.push(b);
                        rows_obs.push((vec![a_lbl, b], val));
                    }
                }
            }
        }
        (a_ids, b_ids, y, subj_ids, tp_ids, rows_obs)
    };

    let (a0, b0, y0, subj0, tp0, obs_rows0) = build_dataset([0, 1, 2]);
    let (a1, b1, y1, subj1, tp1, _) = build_dataset([2, 1, 0]);

    // Reference OLS Type III
    let ols = fit_factorial_anova(
        &FactorialObservations {
            factor_names: vec![String::from("A"), String::from("B")],
            factor_levels: vec![
                vec![String::from("A0"), String::from("A1"), String::from("A2")],
                vec![String::from("B0"), String::from("B1"), String::from("B2")],
            ],
            rows: obs_rows0,
        },
        2,
        SumOfSquaresType::TypeIII,
    )
    .unwrap();
    let ols_a = ols.terms.iter().find(|t| t.name == "A").unwrap();
    assert_close(ols_a.df, 1.0, 1e-12, "OLS Type III df_A");
    assert!(ols_a.f_statistic.unwrap() < 1.0, "OLS Type III F_A must be tiny (got {:?})", ols_a.f_statistic);

    let x0 = sum_contrast_two_way_matrix(&a0, &b0, 3, 3);
    let x1 = sum_contrast_two_way_matrix(&a1, &b1, 3, 3);
    let terms: &[(&str, usize, usize)] = &[("A", 1, 2), ("B", 3, 2), ("A × B", 5, 4)];

    // (a) Random-intercept REML
    let ri0 = fit_random_intercept_reml(&x0, &y0, &subj0, terms).unwrap();
    let ri1 = fit_random_intercept_reml(&x1, &y1, &subj1, terms).unwrap();
    assert_close(ri0.fixed_terms[0].df, 1.0, 1e-12, "RI REML df_A");
    assert_close(ri0.fixed_terms[1].df, 1.0, 1e-12, "RI REML df_B");
    assert_close(ri0.fixed_terms[2].df, 3.0, 1e-12, "RI REML df_AB");
    assert!(ri0.fixed_terms[0].f_statistic.unwrap() < 1.0, "RI REML F_A must not be spurious (got {:?})", ri0.fixed_terms[0].f_statistic);
    assert_approx_eq(ri0.fixed_terms[0].f_statistic.unwrap(), ri1.fixed_terms[0].f_statistic.unwrap(), 1e-6, 1e-6, "RI REML relabel F_A");
    assert_approx_eq(ri0.fixed_terms[0].p_value.unwrap(), ri1.fixed_terms[0].p_value.unwrap(), 1e-6, 1e-6, "RI REML relabel P_A");

    // (b) Multi-VC REML
    let (g_subj0, c_subj0) = cluster_g_matrix(&subj0);
    let (g_subj1, c_subj1) = cluster_g_matrix(&subj1);
    let mvc0 = fit_multi_vc_reml(&x0, &y0, &[("Subject", g_subj0, c_subj0)], terms).unwrap();
    let mvc1 = fit_multi_vc_reml(&x1, &y1, &[("Subject", g_subj1, c_subj1)], terms).unwrap();
    assert_close(mvc0.fixed_terms[0].df, 1.0, 1e-12, "Multi-VC REML df_A");
    assert_close(mvc0.fixed_terms[2].df, 3.0, 1e-12, "Multi-VC REML df_AB");
    assert!(mvc0.fixed_terms[0].f_statistic.unwrap() < 1.0, "Multi-VC REML F_A must not be spurious (got {:?})", mvc0.fixed_terms[0].f_statistic);
    assert_approx_eq(mvc0.fixed_terms[0].f_statistic.unwrap(), mvc1.fixed_terms[0].f_statistic.unwrap(), 1e-5, 1e-5, "Multi-VC relabel F_A");

    // (c) Unstructured RM REML
    let tp_labels = vec![String::from("B0"), String::from("B1"), String::from("B2")];
    let un0 = fit_unstructured_rm_reml(&x0, &y0, &subj0, &tp0, &tp_labels, terms).unwrap();
    let un1 = fit_unstructured_rm_reml(&x1, &y1, &subj1, &tp1, &tp_labels, terms).unwrap();
    assert_close(un0.fixed_terms[0].df, 1.0, 1e-12, "Unstructured REML df_A");
    assert_close(un0.fixed_terms[2].df, 3.0, 1e-12, "Unstructured REML df_AB");
    assert!(un0.fixed_terms[0].f_statistic.unwrap() < 1.0, "Unstructured REML F_A must not be spurious (got {:?})", un0.fixed_terms[0].f_statistic);
    assert_approx_eq(un0.fixed_terms[0].f_statistic.unwrap(), un1.fixed_terms[0].f_statistic.unwrap(), 1e-4, 1e-4, "Unstructured relabel F_A");

    // 2. 2 x 2 design with empty cell (1, 1): all three terms are unestimable under Type III and MUST be emitted
    // with df = 0.0, F = None, P = None (never dropped from fixed_terms!).
    let mut a22 = Vec::new();
    let mut b22 = Vec::new();
    let mut y22 = Vec::new();
    let mut s22 = Vec::new();
    for (a, b, mu) in [(0_usize, 0_usize, 10.0), (0, 1, 20.0), (1, 0, 15.0)] {
        for s in 0..4_usize {
            a22.push(a);
            b22.push(b);
            y22.push(mu + 0.3 * (s as f64));
            s22.push(a * 4 + s);
        }
    }
    let x22 = sum_contrast_two_way_matrix(&a22, &b22, 2, 2);
    let terms22: &[(&str, usize, usize)] = &[("A", 1, 1), ("B", 2, 1), ("A × B", 3, 1)];

    let ri22 = fit_random_intercept_reml(&x22, &y22, &s22, terms22).unwrap();
    assert_eq!(ri22.fixed_terms.len(), 3, "2x2 empty cell must retain A x B row in RI REML");
    for t in &ri22.fixed_terms {
        assert_close(t.df, 0.0, 1e-12, &format!("RI 2x2 {} df", t.name));
        assert!(t.f_statistic.is_none() && t.p_value.is_none(), "RI 2x2 {} should be unestimable", t.name);
    }
    assert_eq!(ri22.satterthwaite_df.len(), 3);
    assert!(ri22.kenward_roger_df.iter().all(|v| v.is_nan()));

    let (g_s22, c_s22) = cluster_g_matrix(&s22);
    let mvc22 = fit_multi_vc_reml(&x22, &y22, &[("Subject", g_s22, c_s22)], terms22).unwrap();
    assert_eq!(mvc22.fixed_terms.len(), 3, "2x2 empty cell must retain A x B row in Multi-VC REML");
    assert!(mvc22.fixed_terms[2].f_statistic.is_none());

    let un22 = fit_unstructured_rm_reml(&x22, &y22, &s22, &b22, &[String::from("B0"), String::from("B1")], terms22).unwrap();
    assert_eq!(un22.fixed_terms.len(), 3, "2x2 empty cell must retain A x B row in Unstructured REML");
    assert!(un22.fixed_terms[2].f_statistic.is_none());

    // Sheet-level 2x2 empty-cell RM REML must produce clean finite scalars (skipping NaN df scalars).
    let mut g22 = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 2).unwrap();
    for s in 0..4 {
        g22.data_sets[0].cells[0][s] = Cell::number(10.0 + 0.3 * s as f64);
        g22.data_sets[1].cells[0][s] = Cell::number(20.0 + 0.3 * s as f64);
        g22.data_sets[0].cells[1][s] = Cell::number(15.0 + 0.3 * s as f64);
    }
    let sheet22 = DataSheet::new("g22", "G22", TableArchetype::Grouped(g22)).unwrap();
    let spec22 =
        FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched, fit_reml_on_missing: true, ..Default::default() };
    let res22 = run_factorial_anova(&sheet22, &spec_for("r22", "g22", AnalysisKind::FactorialAnova(spec22.clone())), &spec22).unwrap();
    assert!(res22.scalars.iter().all(|s| s.value.is_finite()), "2x2 empty-cell REML sheet must have only finite scalars");
}

// ============================================================================
// AUD-05-18: Zero within-group error effect-size CIs must be None, not [0, 0]
// ============================================================================

#[test]
fn rem_aud05_18_zero_within_error_effect_size_cis_are_none() {
    let res = ordinary_one_way_anova(&[("A", &[10.0, 10.0, 10.0]), ("B", &[20.0, 20.0, 20.0])], ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(res.f_statistic.is_infinite() && res.f_statistic.is_sign_positive());
    assert_close(res.p_value, 0.0, 1e-15, "zero-error p");
    assert_close(res.effect_sizes.eta_squared, 1.0, 1e-12, "zero-error eta^2");
    assert_close(res.effect_sizes.partial_eta_squared, 1.0, 1e-12, "zero-error partial eta^2");
    assert!(res.effect_sizes.cohens_f.is_infinite() && res.effect_sizes.cohens_f.is_sign_positive());
    assert_eq!(res.effect_sizes.eta_squared_ci, None, "degenerate ms_error == 0 must return None eta_squared_ci");
    assert_eq!(res.effect_sizes.partial_eta_squared_ci, None, "degenerate ms_error == 0 must return None partial_eta_squared_ci");
    assert_eq!(res.effect_sizes.cohens_f_ci, None, "degenerate ms_error == 0 must return None cohens_f_ci");

    let direct = compute_anova_effect_sizes_for_n(150.0, 0.0, 150.0, 1.0, 4.0, 6, ConfidenceLevel::DEFAULT_95);
    assert_eq!(direct.eta_squared_ci, None);
    assert_eq!(direct.partial_eta_squared_ci, None);
    assert_eq!(direct.cohens_f_ci, None);
}

// ============================================================================
// AUD-05-19: Unbalanced nested 2-group t sign follows REML GLS, not raw mean
// ============================================================================

#[test]
fn rem_aud05_19_nested_two_group_t_sign_matches_reml_gls_under_simpson_reversal() {
    // Group 0: Unit 0A (10 reps ~100.0), Unit 0B (2 reps ~10.0) -> raw pooled mean = 85.0, REML GLS mean ~= 55.0.
    // Group 1: Unit 1A (2 reps ~70.0), Unit 1B (2 reps ~70.0) -> raw pooled mean = 70.0, REML GLS mean = 70.0.
    // Raw pooled diff = +15.0 > 0, whereas REML GLS contrast mu_0 - mu_1 ~= -15.0 < 0.
    let u0a: &[f64] = &[99.6, 100.4, 99.8, 100.2, 100.1, 99.9, 100.3, 99.7, 100.0, 100.0];
    let u0b: &[f64] = &[9.8, 10.2];
    let u1a: &[f64] = &[69.8, 70.2];
    let u1b: &[f64] = &[69.9, 70.1];
    let groups: &[(&str, &[&[f64]])] = &[("Group 0", &[u0a, u0b]), ("Group 1", &[u1a, u1b])];

    let tab = make_nested_table(groups);
    let res = nested_one_way_anova(&tab, true).unwrap();
    let t_val = res.t_nested.expect("two_group_t_test must populate t_nested");
    assert!(t_val < 0.0, "Simpson reversal: REML GLS mu_0 - mu_1 < 0 must produce negative t_nested, got {t_val}");
    assert_approx_eq(t_val * t_val, res.f_among_groups, 1e-9, 1e-9, "t_nested^2 == F_among_groups");

    let sheet = make_nested_sheet("nest-simpson", groups);
    let spec = NestedAnalysisSpec { two_group_t_test: true, ..Default::default() };
    let out = run_nested_analysis(&sheet, &spec_for("r-ns", "nest-simpson", AnalysisKind::NestedComparison(spec.clone())), &spec).unwrap();
    let mean_diff = scalar_val(&out, "Mean difference");
    let se_diff = scalar_val(&out, "SE of difference");
    let t_scalar = scalar_val(&out, "Nested t");
    assert!(mean_diff < -14.0 && mean_diff > -16.0, "REML GLS mean diff should be ~ -15.0, got {mean_diff}");
    assert_approx_eq(t_scalar, mean_diff / se_diff, 1e-8, 1e-8, "Nested t == mean_diff / se_diff");
}

// ============================================================================
// AUD-05-20: Nested post-hoc uses REML GLS, exact KR/Sat df, valid LinearTrend R^2
// ============================================================================

#[test]
fn rem_aud05_20_nested_posthoc_reml_gls_contrasts_linear_trend_and_incompatible_methods() {
    // 1. Unbalanced 3-group nested table with Simpson reversal on Group 0 vs Group 1.
    let u0a: &[f64] = &[99.6, 100.4, 99.8, 100.2, 100.1, 99.9, 100.3, 99.7, 100.0, 100.0];
    let u0b: &[f64] = &[9.8, 10.2];
    let u1a: &[f64] = &[69.8, 70.2];
    let u1b: &[f64] = &[69.9, 70.1];
    let u2a: &[f64] = &[89.7, 90.3];
    let u2b: &[f64] = &[89.9, 90.1];
    let sheet_unbal = make_nested_sheet("nest-3g-unbal", &[("G0", &[u0a, u0b]), ("G1", &[u1a, u1b]), ("G2", &[u2a, u2b])]);

    let spec_tukey = NestedAnalysisSpec {
        two_group_t_test: false,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
        ..Default::default()
    };
    let out_tukey =
        run_nested_analysis(&sheet_unbal, &spec_for("r-nt", "nest-3g-unbal", AnalysisKind::NestedComparison(spec_tukey.clone())), &spec_tukey)
            .unwrap();
    let mc_tab = tab_by_id(&out_tukey, "multiple-comparisons");
    let g0_vs_g1_diff = cell_num(&mc_tab.rows[0][0]);
    assert!(
        g0_vs_g1_diff < -14.0 && g0_vs_g1_diff > -16.0,
        "Nested Tukey G0 vs G1 must use REML GLS diff (~ -15), not raw pooled (+15); got {g0_vs_g1_diff}"
    );

    // 2. Zero biological-unit variance (sigma_u^2 = 0): balanced 3 groups x 2 subclusters x 5 reps has
    // exact REML KR/Satterthwaite df = 3.0 (c - g = 6 - 3), NOT Welch 2*(m_g - 1) = 2.0, and every pairwise
    // contrast uses exact REML df == active_df2 (and when one group has m_g = 1 subcluster, REML still uses
    // full model residual/subcluster df instead of bogus Welch df = 1.0).
    let g_zero_u: &[(&str, &[&[f64]])] = &[
        ("Low", &[&[10.0, 12.0, 8.0, 11.0, 9.0], &[9.5, 11.5, 8.5, 10.5, 10.0]]),
        ("Mid", &[&[15.0, 17.0, 13.0, 16.0, 14.0], &[14.5, 16.5, 13.5, 15.5, 15.0]]),
        ("High", &[&[20.0, 22.0, 18.0, 21.0, 19.0], &[19.5, 21.5, 18.5, 20.5, 20.0]]),
    ];
    let sheet_zu = make_nested_sheet("nest-zu", g_zero_u);
    let out_zu =
        run_nested_analysis(&sheet_zu, &spec_for("r-zu", "nest-zu", AnalysisKind::NestedComparison(spec_tukey.clone())), &spec_tukey).unwrap();
    assert_close(scalar_val(&out_zu, "Subcluster variance"), 0.0, 1e-10, "sigma_u^2 == 0");
    let zu_df = scalar_val(&out_zu, "Group denominator df");
    assert_close(zu_df, 3.0, 1e-6, "balanced 3x2x5 REML df = c - g = 3.0");
    let zu_mc = tab_by_id(&out_zu, "multiple-comparisons");
    for (idx, row) in zu_mc.rows.iter().enumerate() {
        assert_close(cell_num(&row[3]), zu_df, 1e-6, &format!("pair {idx} uses exact REML df (not Welch 2*(m_g-1)=2)"));
    }

    // Also verify a group with m_g = 1 subcluster gets exact REML KR df (> 1.0 when other groups supply df), not bogus Welch df = 1.0.
    let g_m1: &[(&str, &[&[f64]])] = &[
        ("Single", &[&[10.0, 11.0, 9.0, 10.5, 9.5]]),
        ("Multi1", &[&[15.0, 16.0, 14.0, 15.5, 14.5], &[15.2, 16.2, 14.2, 15.7, 14.7], &[14.8, 15.8, 13.8, 15.3, 14.3]]),
        ("Multi2", &[&[20.0, 21.0, 19.0, 20.5, 19.5], &[20.2, 21.2, 19.2, 20.7, 19.7], &[19.8, 20.8, 18.8, 20.3, 19.3]]),
    ];
    let sheet_m1 = make_nested_sheet("nest-m1", g_m1);
    let out_m1 =
        run_nested_analysis(&sheet_m1, &spec_for("r-m1", "nest-m1", AnalysisKind::NestedComparison(spec_tukey.clone())), &spec_tukey).unwrap();
    let m1_mc = tab_by_id(&out_m1, "multiple-comparisons");
    assert!(cell_num(&m1_mc.rows[0][3]) > 3.5, "m_g = 1 group pair must inherit REML df (~4.0), got {}", cell_num(&m1_mc.rows[0][3]));

    // 3. LinearTrend uses Nakagawa & Schielzeth (2013) marginal R^2 reflecting var_subclusters + var_error,
    // and is strictly scale-invariant.
    let g_trend: &[(&str, &[&[f64]])] = &[
        ("D1", &[&[10.0, 10.4, 9.6], &[14.0, 14.3, 13.7], &[12.0, 12.2, 11.8]]),
        ("D2", &[&[15.0, 15.4, 14.6], &[19.0, 19.3, 18.7], &[17.0, 17.2, 16.8]]),
        ("D3", &[&[20.0, 20.4, 19.6], &[24.0, 24.3, 23.7], &[22.0, 22.2, 21.8]]),
    ];
    let sheet_tr = make_nested_sheet("nest-tr", g_trend);
    let spec_tr = NestedAnalysisSpec {
        two_group_t_test: false,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false },
        ..Default::default()
    };
    let out_tr = run_nested_analysis(&sheet_tr, &spec_for("r-tr", "nest-tr", AnalysisKind::NestedComparison(spec_tr.clone())), &spec_tr).unwrap();
    let tr_tab = tab_by_id(&out_tr, "linear-trend");
    let r2_col = tr_tab.column_headers.iter().position(|h| h == "R squared").expect("R squared column");
    let r2_val = cell_num(&tr_tab.rows[0][r2_col]);
    // Balanced means [12, 17, 22] => var_fixed = ((12-17)^2 + 0 + (22-17)^2) / 3 = 50/3 = 16.6667;
    // subcluster variance ~= 4.0, residual variance ~= 0.10 => marginal R^2 ~= 16.67 / (16.67 + 4.0 + 0.1) ~= 0.80.
    let s2_u = scalar_val(&out_tr, "Subcluster variance");
    let s2_e = scalar_val(&out_tr, "Residual variance");
    let expected_r2 = (50.0 / 3.0) / ((50.0 / 3.0) + s2_u + s2_e);
    assert_approx_eq(r2_val, expected_r2, 1e-4, 1e-4, "Nested LinearTrend Nakagawa-Schielzeth marginal R^2");

    // 4. Incompatible procedures (GamesHowell, TamhaneT2, DunnettT3, DunnRankSum) return IncompatibleAnalysisSpec.
    for bad_method in [MultiplicityMethod::GamesHowell, MultiplicityMethod::TamhaneT2, MultiplicityMethod::DunnettT3, MultiplicityMethod::DunnRankSum]
    {
        let bad_spec = NestedAnalysisSpec {
            two_group_t_test: false,
            multiplicity: MultiplicitySpec { method: bad_method, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        };
        let err = run_nested_analysis(&sheet_unbal, &spec_for("r-bad", "nest-3g-unbal", AnalysisKind::NestedComparison(bad_spec.clone())), &bad_spec)
            .unwrap_err();
        assert!(
            matches!(err, StatsError::IncompatibleAnalysisSpec { .. }),
            "Nested ANOVA must reject {bad_method:?} with IncompatibleAnalysisSpec, got {err:?}"
        );
    }
}

// ============================================================================
// AUD-05-21: Factorial rejects LinearTrend & DunnRankSum; cross-slice Bonferroni
// ============================================================================

#[test]
fn rem_aud05_21_factorial_rejects_linear_trend_and_dunn_rank_sum() {
    let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(3), 3, 3).unwrap();
    for c in 0..3 {
        g.data_sets[c].title = format!("B{c}");
        for r in 0..3 {
            for s in 0..3 {
                g.data_sets[c].cells[r][s] = Cell::number(10.0 + 2.0 * (r as f64) + 3.0 * (c as f64) + 0.5 * ((r * c) as f64) + 0.4 * (s as f64));
            }
        }
    }
    for r in 0..3 {
        g.row_titles[r] = format!("A{r}");
    }
    let sheet = DataSheet::new("g33", "G33", TableArchetype::Grouped(g)).unwrap();

    for rm in [RepeatedMeasuresDesign::None, RepeatedMeasuresDesign::ColumnFactorMatched] {
        for bad_method in [MultiplicityMethod::LinearTrend, MultiplicityMethod::DunnRankSum] {
            let spec = FactorialAnovaSpec {
                repeated_measures: rm,
                mixed_effects_reml: rm != RepeatedMeasuresDesign::None,
                multiplicity: MultiplicitySpec { method: bad_method, threshold: 0.05, report_confidence_intervals: false },
                ..Default::default()
            };
            let err = run_factorial_anova(&sheet, &spec_for("r-fbad", "g33", AnalysisKind::FactorialAnova(spec.clone())), &spec).unwrap_err();
            assert!(
                matches!(err, StatsError::IncompatibleAnalysisSpec { .. }),
                "Factorial ({rm:?}) must reject {bad_method:?} with IncompatibleAnalysisSpec, got {err:?}"
            );
        }
    }

    // Valid Bonferroni on 3x3 simple effects adjusts across all 9 comparisons (3 slices x 3 pairs) consistently with footnote.
    let spec_bonf = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: false },
        ..Default::default()
    };
    let res_bonf = run_factorial_anova(&sheet, &spec_for("r-fbonf", "g33", AnalysisKind::FactorialAnova(spec_bonf.clone())), &spec_bonf).unwrap();
    let simple_tab = res_bonf.tabs.iter().find(|t| t.id.starts_with("multiple-comparisons-simple-")).expect("simple effects tab");
    assert_eq!(simple_tab.rows.len(), 9);
    assert!(simple_tab.footnotes.iter().any(|f| f.contains("9 simple-effect")));
    // Verify adjusted P = min(1, 9 * raw_p) using t statistic and df from row 0.
    let diff = cell_num(&simple_tab.rows[0][0]);
    let se = cell_num(&simple_tab.rows[0][1]);
    let df = cell_num(&simple_tab.rows[0][3]);
    let adj_col = simple_tab
        .column_headers
        .iter()
        .position(|h| h.to_ascii_lowercase().contains("adj"))
        .unwrap_or_else(|| panic!("missing Adjusted P column in {:?}", simple_tab.column_headers));
    let adj_p = cell_p(&simple_tab.rows[0][adj_col]);
    let raw_p = student_t_two_sided_p((diff / se).abs(), df).unwrap();
    assert_approx_eq(adj_p, (raw_p * 9.0).min(1.0), 1e-10, 1e-10, "3x3 simple-effects Bonferroni factor 9");
}

// ============================================================================
// AUD-05-22: RM one-way GG=true subject-level contrasts for Tukey/Dunnett/NK/Trend
// ============================================================================

#[test]
fn rem_aud05_22_rm_one_way_gg_subject_level_posthoc_contrasts() {
    // Complete 10-subject x 3-condition non-spherical RM dataset:
    // C1 and C2 track tightly (C2 - C1 ~= 0.52, paired SE ~= 0.017, df = 9),
    // while C3 has huge subject-by-treatment variance (eps_GG ~= 0.50, pooled SE > 3.0).
    let c1: [f64; 10] = [10.0, 12.0, 11.0, 13.0, 9.5, 14.0, 10.5, 12.5, 11.5, 13.5];
    let d12: [f64; 10] = [0.50, 0.54, 0.48, 0.56, 0.52, 0.51, 0.55, 0.49, 0.53, 0.55];
    let d13: [f64; 10] = [-18.0, 15.0, -14.0, 20.0, -16.0, 17.0, -15.0, 19.0, -17.0, 14.0];
    let c2: Vec<f64> = c1.iter().zip(d12.iter()).map(|(&a, &d)| a + d).collect();
    let c3: Vec<f64> = c1.iter().zip(d13.iter()).map(|(&a, &d)| a + d).collect();

    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 10).unwrap();
    tab.data_sets[0].title = String::from("C1");
    tab.data_sets[1].title = String::from("C2");
    tab.data_sets[2].title = String::from("C3");
    for r in 0..10 {
        tab.data_sets[0].cells[r][0] = Cell::number(c1[r]);
        tab.data_sets[1].cells[r][0] = Cell::number(c2[r]);
        tab.data_sets[2].cells[r][0] = Cell::number(c3[r]);
    }
    let sheet = DataSheet::new("rm-gg-ph", "RM-GG-PH", TableArchetype::Column(tab)).unwrap();

    // Analytical paired-difference SE for C1 - C2 on df = 9:
    let mean_d12 = d12.iter().sum::<f64>() / 10.0;
    let var_d12 = d12.iter().map(|&d| (d - mean_d12).powi(2)).sum::<f64>() / 9.0;
    let expected_se_12 = (var_d12 / 10.0).sqrt();

    for method in [MultiplicityMethod::TukeyHsd, MultiplicityMethod::Dunnett { control_column: 0 }, MultiplicityMethod::NewmanKeuls] {
        let spec_gg = OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            multiplicity: MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        };
        let res_gg = run_one_way_anova(&sheet, &spec_for("r-gg", "rm-gg-ph", AnalysisKind::OneWayAnova(spec_gg.clone())), &spec_gg).unwrap();
        let mc = tab_by_id(&res_gg, "multiple-comparisons");
        // Row 0 is C1 vs C2 (or C2 vs C1 for Dunnett).
        let se_12 = cell_num(&mc.rows[0][1]);
        let df_12 = cell_num(&mc.rows[0][3]);
        let adj_p_12 = cell_p(&mc.rows[0][4]);
        assert_approx_eq(se_12, expected_se_12, 1e-12, 1e-12, &format!("RM GG {method:?} C1-C2 subject-level SE"));
        assert_close(df_12, 9.0, 1e-12, &format!("RM GG {method:?} df = n - 1"));
        assert!(adj_p_12 < 1e-6, "RM GG {method:?} C1-C2 must be strongly significant (got P = {adj_p_12})");
    }

    // LinearTrend under geisser_greenhouse = true uses subject-level linear contrast L_i = -y_{i,0} + y_{i,2} on df = n - 1 = 9.
    let spec_lt = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false },
        ..Default::default()
    };
    let res_lt = run_one_way_anova(&sheet, &spec_for("r-lt", "rm-gg-ph", AnalysisKind::OneWayAnova(spec_lt.clone())), &spec_lt).unwrap();
    let lt_tab = tab_by_id(&res_lt, "linear-trend");
    let scores: Vec<f64> = (0..10).map(|i| c3[i] - c1[i]).collect();
    let mean_l = scores.iter().sum::<f64>() / 10.0;
    let var_l = scores.iter().map(|&v| (v - mean_l).powi(2)).sum::<f64>() / 9.0;
    let expected_f_lt = (mean_l / (var_l / 10.0).sqrt()).powi(2);
    let expected_p_lt = f_sf(expected_f_lt, 1.0, 9.0).unwrap();
    assert_approx_eq(scalar_val(&res_lt, "Linear trend F"), expected_f_lt, 1e-10, 1e-10, "RM GG subject-level LinearTrend F");
    assert_approx_eq(scalar_val(&res_lt, "Linear trend P"), expected_p_lt, 1e-10, 1e-10, "RM GG subject-level LinearTrend P");
    assert_close(cell_num(&lt_tab.rows[0][2]), 9.0, 1e-12, "RM GG LinearTrend DFd = n - 1");

    // Contrast: sphericity-assuming geisser_greenhouse = false uses pooled df = (n - 1)(k - 1) = 18 and pooled SE > 2.0.
    let spec_sph = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false },
        ..Default::default()
    };
    let res_sph = run_one_way_anova(&sheet, &spec_for("r-sph", "rm-gg-ph", AnalysisKind::OneWayAnova(spec_sph.clone())), &spec_sph).unwrap();
    let mc_sph = tab_by_id(&res_sph, "multiple-comparisons");
    assert_close(cell_num(&mc_sph.rows[0][3]), 18.0, 1e-12, "Sphericity-assumed Tukey df = 18");
    assert!(cell_num(&mc_sph.rows[0][1]) > 2.0, "Sphericity-assumed pooled SE should be large (>2.0)");
}

// ============================================================================
// AUD-05-23: Newman-Keuls preserves known level means when another level is None
// ============================================================================

#[test]
fn rem_aud05_23_newman_keuls_empty_cell_level_ordering_preserved() {
    // 3 x 3 factorial design with empty cell (A2, B2).
    // Marginal means: A0 = 100.0, A1 = 105.0, A2 = None (estimable over B0, B1 with mean 110.0 so diff(A0, A2) = -10.0).
    // True rank order is [A0 (100), A1 (105), A2 (110)]:
    //   - (A0, A1) has span r = 2
    //   - (A0, A2) has span r = 3
    let means: [[Option<f64>; 3]; 3] =
        [[Some(100.0), Some(100.0), Some(100.0)], [Some(105.0), Some(105.0), Some(105.0)], [Some(110.0), Some(110.0), None]];
    let jitter: [f64; 3] = [-6.0, 0.0, 6.0];
    let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(3), 3, 3).unwrap();
    g.row_factor_title = String::from("FactorA");
    g.column_factor_title = String::from("FactorB");
    for r in 0..3 {
        g.row_titles[r] = format!("A{r}");
    }
    for c in 0..3 {
        g.data_sets[c].title = format!("B{c}");
    }
    for r in 0..3 {
        for c in 0..3 {
            if let Some(mu) = means[r][c] {
                for (s, &j) in jitter.iter().enumerate() {
                    g.data_sets[c].cells[r][s] = Cell::number(mu + j);
                }
            }
        }
    }
    let sheet = DataSheet::new("nk-empty", "NK-Empty", TableArchetype::Grouped(g)).unwrap();
    let spec = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::NewmanKeuls, threshold: 0.05, report_confidence_intervals: false },
        ..Default::default()
    };
    let res = run_factorial_anova(&sheet, &spec_for("r-nk", "nk-empty", AnalysisKind::FactorialAnova(spec.clone())), &spec).unwrap();
    let tab_a = tab_by_id(&res, "multiple-comparisons-factor-a");
    // Locate A0 vs A1 and A0 vs A2 rows regardless of "vs" / "vs." formatting.
    let idx_01 = tab_a
        .row_labels
        .iter()
        .position(|l| l.contains("A0") && l.contains("A1"))
        .unwrap_or_else(|| panic!("missing A0/A1 row in {:?}", tab_a.row_labels));
    let idx_02 = tab_a
        .row_labels
        .iter()
        .position(|l| l.contains("A0") && l.contains("A2"))
        .unwrap_or_else(|| panic!("missing A0/A2 row in {:?}", tab_a.row_labels));
    let adj_col = tab_a
        .column_headers
        .iter()
        .position(|h| h.to_ascii_lowercase().contains("adj"))
        .unwrap_or_else(|| panic!("missing Adjusted P column in {:?}", tab_a.column_headers));

    let q_01 = cell_num(&tab_a.rows[idx_01][2]);
    let df_01 = cell_num(&tab_a.rows[idx_01][3]);
    let p_01 = cell_p(&tab_a.rows[idx_01][adj_col]);

    let q_02 = cell_num(&tab_a.rows[idx_02][2]);
    let df_02 = cell_num(&tab_a.rows[idx_02][3]);
    let p_02 = cell_p(&tab_a.rows[idx_02][adj_col]);

    // Under true order [A0, A1, A2]:
    // (A0, A2) has span c = 3 -> exact studentized range p at (r = 1 family, c = 3 means).
    let exp_p_02_span3 = max_studentized_range_sf(q_02, 1, 3, df_02).unwrap();
    assert_approx_eq(p_02, exp_p_02_span3, 1e-10, 1e-10, "A0 vs A2 must have Newman-Keuls span c = 3");

    // (A0, A1) has span c = 2 (bounded below by enclosing span-3 pair p_02).
    let raw_p_01_span2 = max_studentized_range_sf(q_01, 1, 2, df_01).unwrap();
    let exp_p_01 = raw_p_01_span2.max(exp_p_02_span3);
    assert_approx_eq(p_01, exp_p_01, 1e-10, 1e-10, "A0 vs A1 must have Newman-Keuls span c = 2 (not scrambled to c = 3)");
    // Verify span 3 would have given a strictly different p-value for (A0, A1) so the test discriminates!
    let wrong_p_01_span3 = max_studentized_range_sf(q_01, 1, 3, df_01).unwrap();
    assert!((p_01 - wrong_p_01_span3).abs() > 1e-4, "test must discriminate span 2 vs scrambled span 3");
}

// ============================================================================
// AUD-05-24: Scale-invariant REML ICC & % variance on picomolar (1e-11) data
// ============================================================================

#[test]
fn rem_aud05_24_small_scale_reml_icc_and_variance_percentages() {
    let scale = 1e-11_f64;

    // 1. Multi-VC RM REML (BothFactorsMatched) at unit scale vs 1e-11 picomolar scale.
    let build_rm_grp = |sc: f64| {
        let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(6), 2, 2).unwrap();
        let subj_eff = [-3.0, -1.8, -0.6, 0.7, 1.9, 2.8];
        for s in 0..6 {
            for r in 0..2 {
                for c in 0..2 {
                    let y = 10.0 + subj_eff[s] + 1.5 * (r as f64) + 2.0 * (c as f64) + 0.15 * (((s + r + c) % 3) as f64 - 1.0);
                    g.data_sets[c].cells[r][s] = Cell::number(y * sc);
                }
            }
        }
        DataSheet::new("rm-pico", "RM-Pico", TableArchetype::Grouped(g)).unwrap()
    };
    let rm_spec =
        FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched, mixed_effects_reml: true, ..Default::default() };
    let rm_unit =
        run_factorial_anova(&build_rm_grp(1.0), &spec_for("ru", "rm-pico", AnalysisKind::FactorialAnova(rm_spec.clone())), &rm_spec).unwrap();
    let rm_pico =
        run_factorial_anova(&build_rm_grp(scale), &spec_for("rp", "rm-pico", AnalysisKind::FactorialAnova(rm_spec.clone())), &rm_spec).unwrap();

    let icc_unit = scalar_val(&rm_unit, "REML ICC");
    let icc_pico = scalar_val(&rm_pico, "REML ICC");
    assert!(icc_unit > 0.5, "unit-scale REML ICC should be substantial, got {icc_unit}");
    assert_approx_eq(icc_pico, icc_unit, 1e-5, 1e-5, "picomolar multi-VC REML ICC invariance");

    let vc_tab = tab_by_id(&rm_pico, "mixed-reml");
    let pct_col = vc_tab.column_headers.iter().position(|h| h == "% of total").expect("% of total column");
    let pct_sum: f64 = vc_tab.rows.iter().map(|row| cell_num(&row[pct_col])).sum();
    assert_approx_eq(pct_sum, 100.0, 1e-4, 1e-4, "picomolar multi-VC % of total sums to 100%");

    // 2. Nested ANOVA at unit scale vs 1e-11 picomolar scale.
    let base_nested: &[(&str, &[&[f64]])] = &[
        ("Ctrl", &[&[10.0, 10.2, 9.8], &[13.0, 13.1, 12.9], &[16.0, 16.2, 15.8]]),
        ("Drug", &[&[20.0, 20.1, 19.9], &[23.0, 23.2, 22.8], &[26.0, 26.1, 25.9]]),
    ];
    let build_nested = |sc: f64| {
        let owned: Vec<(&str, Vec<Vec<f64>>)> =
            base_nested.iter().map(|(lbl, subs)| (*lbl, subs.iter().map(|reps| reps.iter().map(|&v| v * sc).collect()).collect())).collect();
        let mut t = NestedTable::new(2, 3, 3).unwrap();
        for (g, (lbl, subs)) in owned.iter().enumerate() {
            t.groups[g].title = (*lbl).to_owned();
            for (b, reps) in subs.iter().enumerate() {
                for (r, &v) in reps.iter().enumerate() {
                    t.groups[g].cells[r][b] = Cell::number(v);
                }
            }
        }
        DataSheet::new("nest-pico", "Nest-Pico", TableArchetype::Nested(t)).unwrap()
    };
    let nest_spec = NestedAnalysisSpec { two_group_t_test: false, ..Default::default() };
    let n_unit =
        run_nested_analysis(&build_nested(1.0), &spec_for("nu", "nest-pico", AnalysisKind::NestedComparison(nest_spec.clone())), &nest_spec).unwrap();
    let n_pico =
        run_nested_analysis(&build_nested(scale), &spec_for("np", "nest-pico", AnalysisKind::NestedComparison(nest_spec.clone())), &nest_spec)
            .unwrap();

    assert_approx_eq(scalar_val(&n_pico, "ICC (subcluster)"), scalar_val(&n_unit, "ICC (subcluster)"), 1e-5, 1e-5, "picomolar nested ICC");
    let n_vc_tab = tab_by_id(&n_pico, "variance-components");
    let n_pct_sum: f64 = n_vc_tab.rows.iter().map(|row| cell_num(&row[2])).sum();
    assert_approx_eq(n_pct_sum, 100.0, 1e-4, 1e-4, "picomolar nested % of total sums to 100%");
}

// ============================================================================
// AUD-05-40: Incomplete MCAR unstructured REML does not false-flag singularity
// ============================================================================

#[test]
fn rem_aud05_40_incomplete_mcar_unstructured_reml_no_false_singularity() {
    // Construct a deterministic n = 15, t = 4 incomplete MCAR dataset whose pairwise OLS scatter matrix
    // has a strictly negative eigenvalue (scatter_ratio < 0), while the true covariance is well-conditioned SPD.
    // Search deterministic seeds (n = 15, t = 4, 15% MCAR missingness, true Sigma = I_4, matching AUD-05-40)
    // where the raw pairwise OLS scatter matrix has a strictly negative minimum eigenvalue (`min_ev < -1e-3`),
    // confirming the AUD-05-40 trigger is exercised without false `SingularCovarianceEstimate`.
    let mut verified_cases = 0_usize;
    for trial in 0..800 {
        let mut state: u64 = 0x1000_0000_0000_0000_u64.wrapping_add((trial as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (n_subj, t_obs) = (15_usize, 4_usize);
        let mut grid = vec![vec![None; t_obs]; n_subj];
        let mut c_ids = Vec::new();
        let mut t_ids = Vec::new();
        let mut y = Vec::new();
        for s in 0..n_subj {
            for t in 0..t_obs {
                let val = 10.0 + (t as f64) + lcg_norm(&mut state);
                if lcg_u01(&mut state) >= 0.15 {
                    grid[s][t] = Some(val);
                    c_ids.push(s);
                    t_ids.push(t);
                    y.push(val);
                }
            }
        }
        // Require at least 8 pairwise co-observations per pair so the population MLE is well-identified.
        let mut min_pair_cnt = usize::MAX;
        for i in 0..4 {
            for j in i..4 {
                let cnt = (0..n_subj).filter(|&s| grid[s][i].is_some() && grid[s][j].is_some()).count();
                min_pair_cnt = min_pair_cnt.min(cnt);
            }
        }
        if min_pair_cnt < 8 {
            continue;
        }
        // Compute pairwise OLS residual scatter matrix and check its minimum eigenvalue.
        let mut means = [0.0_f64; 4];
        for t in 0..4 {
            let vals: Vec<f64> = (0..n_subj).filter_map(|s| grid[s][t]).collect();
            means[t] = vals.iter().sum::<f64>() / (vals.len() as f64);
        }
        let mut cov_raw = vec![0.0_f64; 16];
        for i in 0..4 {
            for j in i..4 {
                let mut acc = 0.0;
                let mut cnt = 0_usize;
                for s in 0..n_subj {
                    if let (Some(vi), Some(vj)) = (grid[s][i], grid[s][j]) {
                        acc += (vi - means[i]) * (vj - means[j]);
                        cnt += 1;
                    }
                }
                if cnt > 0 {
                    let v = acc / (cnt as f64);
                    cov_raw[i * 4 + j] = v;
                    cov_raw[j * 4 + i] = v;
                }
            }
        }
        let raw_mat = DenseMatrix::from_row_major(4, 4, cov_raw).unwrap();
        let eig = sym_eigen(&raw_mat).unwrap();
        let min_ev = eig.eigenvalues.iter().copied().fold(f64::INFINITY, f64::min);
        if min_ev >= -1e-3 {
            continue;
        }

        // Found a well-identified MCAR dataset whose raw pairwise scatter matrix is indefinite (min_ev < -0.01)!
        let x = sum_contrast_one_way_matrix(&t_ids, 4);
        let labels = vec![String::from("T0"), String::from("T1"), String::from("T2"), String::from("T3")];
        let fit = fit_unstructured_rm_reml(&x, &y, &c_ids, &t_ids, &labels, &[("Time", 1, 3)]).unwrap();
        assert!(
            fit.converged,
            "trial {trial}: unstructured REML must converge cleanly (iters={}, fallback={:?}, cov={:?})",
            fit.iterations, fit.fallback_reason, fit.covariance_matrix
        );
        assert_eq!(
            fit.fallback_reason, None,
            "trial {trial} (raw min_ev={min_ev:.4}): well-conditioned incomplete MCAR must NOT trigger false SingularCovarianceEstimate"
        );
        // Fitted covariance must be strictly SPD.
        let flat: Vec<f64> = fit.covariance_matrix.iter().flatten().copied().collect();
        let fitted_mat = DenseMatrix::from_row_major(4, 4, flat).unwrap();
        let fitted_eig = sym_eigen(&fitted_mat).unwrap();
        assert!(
            fitted_eig.eigenvalues.iter().all(|&ev| ev > 0.0),
            "trial {trial}: fitted Sigma must be strictly positive definite: {:?}",
            fitted_eig.eigenvalues
        );
        verified_cases += 1;
        if verified_cases >= 5 {
            break;
        }
    }
    assert!(verified_cases >= 5, "expected to find at least 5 indefinite pairwise-scatter MCAR trials, found {verified_cases}");
}

// ============================================================================
// AUD-05-41: Multi-VC REML AI stall checks stationarity & escapes via EM
// ============================================================================

#[test]
fn rem_aud05_41_multi_vc_reml_stationarity_guard_on_ill_conditioned_design() {
    // Ill-conditioned 2 x 3 BothFactorsMatched design with extreme variance ratios across components
    // (large subject variance ~1e4, tiny subject*A variance ~1e-3, moderate residual variance ~1.0)
    // and missing cells to stress AI backtracking and stationarity checking.
    let n_subj = 8_usize;
    let subj_eff = [-120.0, -85.0, -40.0, -10.0, 15.0, 45.0, 80.0, 115.0];
    let mut a_ids = Vec::new();
    let mut b_ids = Vec::new();
    let mut c_subj = Vec::new();
    let mut c_sa = Vec::new();
    let mut c_sb = Vec::new();
    let mut y = Vec::new();
    for s in 0..n_subj {
        for a in 0..2_usize {
            for b in 0..3_usize {
                if (s == 1 && a == 1 && b == 2) || (s == 6 && a == 0 && b == 1) {
                    continue;
                }
                let sa_eff = if a == 0 { -0.02 } else { 0.02 } * ((s as f64) - 3.5);
                let sb_eff = ((b as f64) - 1.0) * (((s % 4) as f64) - 1.5) * 2.5;
                let err = (((s * 11 + a * 5 + b * 3 + s * a * b) % 7) as f64 - 3.0) * 0.4;
                a_ids.push(a);
                b_ids.push(b);
                c_subj.push(s);
                c_sa.push(s * 2 + a);
                c_sb.push(s * 3 + b);
                y.push(50.0 + subj_eff[s] + 4.0 * (a as f64) + 3.0 * (b as f64) + sa_eff + sb_eff + err);
            }
        }
    }
    let x = sum_contrast_two_way_matrix(&a_ids, &b_ids, 2, 3);
    let (g_subj, cnt_subj) = cluster_g_matrix(&c_subj);
    let (g_sa, cnt_sa) = cluster_g_matrix(&c_sa);
    let (g_sb, cnt_sb) = cluster_g_matrix(&c_sb);
    let fit = fit_multi_vc_reml(
        &x,
        &y,
        &[("Subject", g_subj.clone(), cnt_subj), ("Subject × A", g_sa.clone(), cnt_sa), ("Subject × B", g_sb.clone(), cnt_sb)],
        &[("A", 1, 1), ("B", 2, 2), ("A × B", 4, 2)],
    )
    .unwrap();
    assert!(
        fit.converged,
        "Multi-VC REML should converge via AI+EM stationarity guard (ll={}, s2e={}, vcs={:?}, terms={:?})",
        fit.reml_log_lik, fit.sigma2_error, fit.variance_components, fit.fixed_terms
    );
    assert!(fit.reml_log_lik.is_finite());
    assert!(fit.sigma2_error > 0.0 && fit.sigma2_error.is_finite());
    assert!(fit.variance_components[0].1 > 1000.0, "Subject variance should capture large ~1e4 signal, got {:?}", fit.variance_components);
    // Verify scale invariance: scaling y by 1e-4 leaves F, df, and p-values invariant.
    let y_sc: Vec<f64> = y.iter().map(|&v| v * 1e-4).collect();
    let fit_sc = fit_multi_vc_reml(
        &x,
        &y_sc,
        &[("Subject", g_subj, cnt_subj), ("Subject × A", g_sa, cnt_sa), ("Subject × B", g_sb, cnt_sb)],
        &[("A", 1, 1), ("B", 2, 2), ("A × B", 4, 2)],
    )
    .unwrap();
    assert!(fit_sc.converged);
    for (t0, t1) in fit.fixed_terms.iter().zip(fit_sc.fixed_terms.iter()) {
        assert_approx_eq(t0.f_statistic.unwrap(), t1.f_statistic.unwrap(), 1e-4, 1e-4, &format!("Multi-VC scale F {}", t0.name));
    }
}

// ============================================================================
// AUD-05-42: Missing-cell RM one-way GG=true + CompoundSymmetry emits warning
// ============================================================================

#[test]
fn rem_aud05_42_one_way_rm_gg_missing_data_cs_warns_user() {
    let sheet = make_col_sheet_opts(
        "rm-gg-miss",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[
            ("T1", &[Some(10.0), Some(12.0), Some(11.0), Some(13.0), Some(9.5), Some(14.0)]),
            ("T2", &[Some(11.0), None, Some(12.2), Some(14.1), Some(10.4), Some(15.2)]),
            ("T3", &[Some(18.0), Some(15.0), Some(20.0), Some(17.0), Some(22.0), Some(16.0)]),
        ],
    );

    // 1. geisser_greenhouse = true + CompoundSymmetry on missing data -> keeps CS REML AND emits Warn checklist + footnote.
    let spec_cs_gg = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
        residual_covariance: ResidualCovariance::CompoundSymmetry,
        ..Default::default()
    };
    let out_cs_gg =
        run_one_way_anova(&sheet, &spec_for("r-cs-gg", "rm-gg-miss", AnalysisKind::OneWayAnova(spec_cs_gg.clone())), &spec_cs_gg).unwrap();
    assert!(out_cs_gg.scalars.iter().any(|s| s.key == "REML sigma^2 (subject)"), "must keep CompoundSymmetry REML fit");
    let warn_item = out_cs_gg
        .checklist
        .iter()
        .find(|c| c.status == ChecklistStatus::Warn && c.assumption == "Geisser-Greenhouse sphericity adjustment")
        .expect("must emit ChecklistStatus::Warn when GG=true runs CompoundSymmetry REML");
    assert!(warn_item.remediation.is_none(), "checklist item must remain diagnostics-only");
    let rm_tab = tab_by_id(&out_cs_gg, "rm-one-way-anova");
    assert!(
        rm_tab.footnotes.iter().any(|f| f.contains("Geisser-Greenhouse") && f.contains("CompoundSymmetry")),
        "rm-one-way-anova tab must emit warning footnote: {:?}",
        rm_tab.footnotes
    );

    // 2. geisser_greenhouse = false + CompoundSymmetry must NOT emit the GG warning.
    let spec_cs_nogg = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
        residual_covariance: ResidualCovariance::CompoundSymmetry,
        ..Default::default()
    };
    let out_cs_nogg =
        run_one_way_anova(&sheet, &spec_for("r-cs-nogg", "rm-gg-miss", AnalysisKind::OneWayAnova(spec_cs_nogg.clone())), &spec_cs_nogg).unwrap();
    assert!(!out_cs_nogg.checklist.iter().any(|c| c.assumption == "Geisser-Greenhouse sphericity adjustment"));

    // 3. geisser_greenhouse = true + Unstructured must NOT emit the GG-not-applied warning.
    let spec_un_gg = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
        residual_covariance: ResidualCovariance::Unstructured,
        ..Default::default()
    };
    let out_un_gg =
        run_one_way_anova(&sheet, &spec_for("r-un-gg", "rm-gg-miss", AnalysisKind::OneWayAnova(spec_un_gg.clone())), &spec_un_gg).unwrap();
    assert!(!out_un_gg.checklist.iter().any(|c| c.assumption == "Geisser-Greenhouse sphericity adjustment"));
}

// ============================================================================
// AUD-05-43: MultipleVariablesTable factorial rejects RM/REML & bad columns
// ============================================================================

#[test]
fn rem_aud05_43_mv_factorial_rejects_rm_reml_and_duplicate_or_response_columns() {
    let mut mv = MultipleVariablesTable::new(3, 8).unwrap();
    mv.variables[0].title = String::from("Y");
    mv.variables[0].lock_role(VariableRole::Continuous);
    mv.variables[1].title = String::from("A");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("B");
    mv.variables[2].lock_role(VariableRole::Categorical);
    let rows = [
        (10.1, "A0", "B0"),
        (10.5, "A0", "B0"),
        (14.2, "A0", "B1"),
        (14.8, "A0", "B1"),
        (12.0, "A1", "B0"),
        (12.6, "A1", "B0"),
        (19.1, "A1", "B1"),
        (19.7, "A1", "B1"),
    ];
    for (r, (y, a, b)) in rows.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(*y);
        mv.variables[1].cells[r] = Cell::text(*a);
        mv.variables[2].cells[r] = Cell::text(*b);
    }
    let sheet = DataSheet::new("mv-fac", "MV-Fac", TableArchetype::MultipleVariables(mv)).unwrap();

    // Unsupported repeated_measures
    let s_rm = FactorialAnovaSpec {
        response_column: Some(0),
        multifactor_columns: vec![1, 2],
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        ..Default::default()
    };
    assert!(matches!(
        run_factorial_anova(&sheet, &spec_for("r1", "mv-fac", AnalysisKind::FactorialAnova(s_rm.clone())), &s_rm),
        Err(StatsError::IncompatibleAnalysisSpec { .. })
    ));

    // Unsupported mixed_effects_reml
    let s_reml = FactorialAnovaSpec { response_column: Some(0), multifactor_columns: vec![1, 2], mixed_effects_reml: true, ..Default::default() };
    assert!(matches!(
        run_factorial_anova(&sheet, &spec_for("r2", "mv-fac", AnalysisKind::FactorialAnova(s_reml.clone())), &s_reml),
        Err(StatsError::IncompatibleAnalysisSpec { .. })
    ));

    // Unsupported Unstructured residual covariance
    let s_un = FactorialAnovaSpec {
        response_column: Some(0),
        multifactor_columns: vec![1, 2],
        residual_covariance: ResidualCovariance::Unstructured,
        ..Default::default()
    };
    assert!(matches!(
        run_factorial_anova(&sheet, &spec_for("r3", "mv-fac", AnalysisKind::FactorialAnova(s_un.clone())), &s_un),
        Err(StatsError::IncompatibleAnalysisSpec { .. })
    ));

    // Duplicate factor columns
    let s_dup = FactorialAnovaSpec { response_column: Some(0), multifactor_columns: vec![1, 1], ..Default::default() };
    assert!(matches!(
        run_factorial_anova(&sheet, &spec_for("r4", "mv-fac", AnalysisKind::FactorialAnova(s_dup.clone())), &s_dup),
        Err(StatsError::InvalidParameter { name: "multifactor_columns", .. })
    ));

    // Factor column overlaps response column
    let s_overlap = FactorialAnovaSpec { response_column: Some(0), multifactor_columns: vec![0, 1], ..Default::default() };
    assert!(matches!(
        run_factorial_anova(&sheet, &spec_for("r5", "mv-fac", AnalysisKind::FactorialAnova(s_overlap.clone())), &s_overlap),
        Err(StatsError::InvalidParameter { name: "multifactor_columns", .. })
    ));

    // Valid MV factorial still succeeds cleanly.
    let s_ok = FactorialAnovaSpec { response_column: Some(0), multifactor_columns: vec![1, 2], ..Default::default() };
    let res_ok = run_factorial_anova(&sheet, &spec_for("r6", "mv-fac", AnalysisKind::FactorialAnova(s_ok.clone())), &s_ok).unwrap();
    assert!(scalar_val(&res_ok, "A: F") > 0.0);
}

// ============================================================================
// AUD-05-55: Multi-way zero-other-SS effect sizes keep eta_squared_ci = None
// ============================================================================

#[test]
fn rem_aud05_55_effect_sizes_multiway_zero_other_ss_keeps_eta2_ci_none() {
    // Balanced 2 x 2 factorial (total_n = 40, df_A = 1, df_B = 1, df_AB = 1, df_error = 36)
    // where SS_B = 0 and SS_AB = 0 so SS_A + SS_error == SS_total.
    let multiway = compute_anova_effect_sizes_for_n(40.0, 100.0, 140.0, 1.0, 36.0, 40, ConfidenceLevel::DEFAULT_95);
    assert_eq!(
        multiway.eta_squared_ci, None,
        "multi-way design (df_effect + df_error + 1 = 38 != total_n = 40) must NOT copy partial_eta_squared_ci into eta_squared_ci"
    );
    assert!(multiway.partial_eta_squared_ci.is_some(), "partial_eta_squared_ci should still be computed via Steiger (2004)");
    assert!(multiway.cohens_f_ci.is_some(), "cohens_f_ci should still be computed");

    // True one-way design (total_n = 38 == df_effect + df_error + 1) populates eta_squared_ci == partial_eta_squared_ci.
    let oneway = compute_anova_effect_sizes_for_n(40.0, 100.0, 140.0, 1.0, 36.0, 38, ConfidenceLevel::DEFAULT_95);
    assert!(oneway.eta_squared_ci.is_some());
    assert_eq!(oneway.eta_squared_ci, oneway.partial_eta_squared_ci);
}

// ============================================================================
// AUD-05-56: Zero-SE REML linear trend & ordinary_one_way_from_summary overflow
// ============================================================================

#[test]
fn rem_aud05_56_reml_zero_se_linear_trend_and_summary_anova_overflow_guards() {
    // 1. Noiseless incomplete RM one-way table with exact additive subject + linear condition trend:
    // y_{s, c} = 10 + 2*s + 5*c with one missing cell => linear trend must NOT report F = 0, P = 1!
    let sheet_noiseless = make_col_sheet_opts(
        "rm-noiseless-lt",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[
            ("C1", &[Some(10.0), Some(12.0), Some(14.0), Some(16.0), Some(18.0)]),
            ("C2", &[Some(15.0), None, Some(19.0), Some(21.0), Some(23.0)]),
            ("C3", &[Some(20.0), Some(22.0), Some(24.0), Some(26.0), Some(28.0)]),
        ],
    );
    let spec_lt = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false },
        ..Default::default()
    };
    let out_lt =
        run_one_way_anova(&sheet_noiseless, &spec_for("r-nlt", "rm-noiseless-lt", AnalysisKind::OneWayAnova(spec_lt.clone())), &spec_lt).unwrap();
    let f_tr = scalar_val(&out_lt, "Linear trend F");
    let p_tr = scalar_val(&out_lt, "Linear trend P");
    assert!(f_tr > 1e8 || f_tr.is_infinite(), "noiseless REML linear trend F must diverge (not 0.0), got {f_tr}");
    assert!(p_tr < 1e-10, "noiseless REML linear trend P must be ~0 (not 1.0), got {p_tr}");

    // 2. ordinary_one_way_from_summary rejects huge g.n > 2^53 (preventing usize::MAX saturation & NaN coercion).
    let huge_n = [
        GroupComparisonInput { index: 0, label: String::from("A"), n: 1e200, mean: 1e200, sd: 0.0 },
        GroupComparisonInput { index: 1, label: String::from("B"), n: 1e200, mean: -1e200, sd: 0.0 },
    ];
    let err_n = ordinary_one_way_from_summary(&huge_n, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err_n, StatsError::InvalidParameter { name: "group_n", .. }), "expected group_n cap error, got {err_n:?}");

    // 3. ordinary_one_way_from_summary rejects overflowing mean / sd arithmetic instead of coercing NaN -> 0.0.
    let overflow_mean = [
        GroupComparisonInput { index: 0, label: String::from("A"), n: 10.0, mean: 1e308, sd: 0.0 },
        GroupComparisonInput { index: 1, label: String::from("B"), n: 10.0, mean: -1e308, sd: 0.0 },
    ];
    let err_m = ordinary_one_way_from_summary(&overflow_mean, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err_m, StatsError::Math(MathError::DomainError { .. })), "expected DomainError on overflowing mean sum, got {err_m:?}");

    let overflow_sd = [
        GroupComparisonInput { index: 0, label: String::from("A"), n: 10.0, mean: 1.0, sd: 1e200 },
        GroupComparisonInput { index: 1, label: String::from("B"), n: 10.0, mean: 2.0, sd: 1e200 },
    ];
    let err_s = ordinary_one_way_from_summary(&overflow_sd, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err_s, StatsError::Math(MathError::DomainError { .. })), "expected DomainError on overflowing sd^2 sum, got {err_s:?}");
}

// ============================================================================
// AUD-05-57: 2-level RM sphericity, Bartlett zero-var docs, nested t-summary
// ============================================================================

#[test]
fn rem_aud05_57_sphericity_2level_bartlett_docs_and_nested_t_summary() {
    // 1. 2-level RM one-way returns mauchly_p = NaN, mauchly_df = 0.0, eps = 1.0.
    let rm1 = rm_one_way_anova(&[vec![10.0, 12.5], vec![11.2, 13.4], vec![9.8, 12.1], vec![10.5, 13.0]]).unwrap();
    assert!(rm1.sphericity.mauchly_p.is_nan(), "2-level RM mauchly_p must be NaN, got {}", rm1.sphericity.mauchly_p);
    assert_close(rm1.sphericity.mauchly_df, 0.0, 1e-12, "2-level RM mauchly_df");
    assert_close(rm1.sphericity.greenhouse_geisser_eps, 1.0, 1e-12, "2-level RM GG eps");

    // 2. 2 x 2 BothFactorsMatched RM: mauchly_p is NaN on all 3 factors AND sheet suppresses trivial df = 0 rm-sphericity tab.
    // subject_data[subject][row][col]
    let rm2_data = vec![
        vec![vec![10.0, 14.0], vec![12.0, 18.0]],
        vec![vec![11.0, 15.2], vec![13.5, 19.1]],
        vec![vec![12.0, 16.1], vec![14.2, 20.4]],
        vec![vec![13.0, 17.5], vec![15.0, 21.2]],
    ];
    let rm2 =
        fit_rm_two_way_anova_with_sphericity(&rm2_data, "A", "B", RepeatedMeasuresDesign::BothFactorsMatched, RmSphericityCorrection::None).unwrap();
    for (lbl, sph_opt) in [("row", &rm2.sphericities.row), ("col", &rm2.sphericities.column), ("int", &rm2.sphericities.interaction)] {
        let s = sph_opt.as_ref().unwrap();
        assert!(s.mauchly_p.is_nan(), "2x2 RM {lbl} mauchly_p must be NaN");
        assert_close(s.mauchly_df, 0.0, 1e-12, &format!("2x2 RM {lbl} mauchly_df"));
        assert_close(s.greenhouse_geisser_eps, 1.0, 1e-12, &format!("2x2 RM {lbl} GG eps"));
    }
    let mut g22 = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 2).unwrap();
    for s in 0..4 {
        for r in 0..2 {
            for c in 0..2 {
                g22.data_sets[c].cells[r][s] = Cell::number(rm2_data[s][r][c]);
            }
        }
    }
    let sheet22 = DataSheet::new("rm22", "RM22", TableArchetype::Grouped(g22)).unwrap();
    let spec22 = FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched, ..Default::default() };
    let out22 = run_factorial_anova(&sheet22, &spec_for("r-rm22", "rm22", AnalysisKind::FactorialAnova(spec22.clone())), &spec22).unwrap();
    assert!(!out22.tabs.iter().any(|t| t.id == "rm-sphericity"), "2x2 RM sheet must omit trivial df == 0 rm-sphericity tab");

    // 2 x 3 BothFactorsMatched RM: rm-sphericity tab is emitted ONLY for factors with mauchly_df > 0 (not the 2-level row factor).
    let mut g23 = GroupedTable::new(SubcolumnMode::RawReplicates(4), 3, 2).unwrap();
    g23.row_factor_title = String::from("Row2");
    g23.column_factor_title = String::from("Col3");
    for r in 0..2 {
        for c in 0..3 {
            for s in 0..4 {
                g23.data_sets[c].cells[r][s] =
                    Cell::number(10.0 + 2.0 * (r as f64) + 3.0 * (c as f64) + 0.5 * (s as f64) + ((r + c * 2 + s) % 3) as f64 * 0.2);
            }
        }
    }
    let sheet23 = DataSheet::new("rm23", "RM23", TableArchetype::Grouped(g23)).unwrap();
    let out23 = run_factorial_anova(&sheet23, &spec_for("r-rm23", "rm23", AnalysisKind::FactorialAnova(spec22.clone())), &spec22).unwrap();
    let sph_tab = tab_by_id(&out23, "rm-sphericity");
    assert!(!sph_tab.row_labels.iter().any(|l| l == "Row2"), "2-level Row2 must be suppressed from rm-sphericity tab");
    assert!(sph_tab.row_labels.iter().any(|l| l == "Col3"), "3-level Col3 must appear in rm-sphericity tab");

    // 3. Bartlett zero-variance contract matches reconciled VarianceHomogeneityResult docs.
    let one_zero = ordinary_one_way_anova(&[("A", &[5.0, 5.0, 5.0]), ("B", &[5.0, 6.0, 7.0])], ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(one_zero.variance_tests.bartlett_chi2.is_infinite() && one_zero.variance_tests.bartlett_chi2.is_sign_positive());
    assert_close(one_zero.variance_tests.bartlett_p, 0.0, 1e-15, "Bartlett one zero-var group p");
    let all_zero = ordinary_one_way_anova(&[("A", &[5.0, 5.0, 5.0]), ("B", &[8.0, 8.0, 8.0])], ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(all_zero.variance_tests.bartlett_chi2, 0.0, 1e-15, "Bartlett all zero-var chi2");
    assert_close(all_zero.variance_tests.bartlett_p, 1.0, 1e-15, "Bartlett all zero-var p");

    // 4. NestedAnalysisSpec { two_group_t_test: true } emits Mean difference, SE, CI scalars, and nested-t-summary tab.
    let nest_sheet = make_nested_sheet(
        "nest-t-ci",
        &[
            ("Control", &[&[10.0, 10.4, 9.6], &[11.2, 11.5, 10.9], &[10.5, 10.8, 10.2]]),
            ("Treated", &[&[15.0, 15.3, 14.7], &[16.1, 16.4, 15.8], &[15.6, 15.9, 15.3]]),
        ],
    );
    let nest_spec = NestedAnalysisSpec { two_group_t_test: true, df_method: MixedModelDfMethod::KenwardRoger, ..Default::default() };
    let mut aspec = spec_for("r-ntci", "nest-t-ci", AnalysisKind::NestedComparison(nest_spec.clone()));
    aspec.set_confidence_percent(90.0).unwrap();
    let out_nt = run_nested_analysis(&nest_sheet, &aspec, &nest_spec).unwrap();
    let diff = scalar_val(&out_nt, "Mean difference");
    let se = scalar_val(&out_nt, "SE of difference");
    let t_val = scalar_val(&out_nt, "Nested t");
    let df = scalar_val(&out_nt, "Group denominator df");
    let ci_lo = scalar_val(&out_nt, "90% CI lower");
    let ci_hi = scalar_val(&out_nt, "90% CI upper");
    let t_crit = student_t_two_sided_isf(0.10, df).unwrap();
    assert_approx_eq(t_val, diff / se, 1e-9, 1e-9, "nested-t-summary t == diff / se");
    assert_approx_eq(ci_lo, diff - t_crit * se, 1e-9, 1e-9, "nested-t-summary 90% CI lower");
    assert_approx_eq(ci_hi, diff + t_crit * se, 1e-9, 1e-9, "nested-t-summary 90% CI upper");
    let sum_tab = tab_by_id(&out_nt, "nested-t-summary");
    assert_eq!(sum_tab.row_labels, vec![String::from("Control - Treated")]);
    let (tab_lo, tab_hi) = cell_ci_finite(&sum_tab.rows[0][5]);
    assert_approx_eq(tab_lo, ci_lo, 1e-12, 1e-12, "nested-t-summary tab CI lo");
    assert_approx_eq(tab_hi, ci_hi, 1e-12, 1e-12, "nested-t-summary tab CI hi");
}

// ============================================================================
// AUD-05-58: RowFactorMatched Unstructured ignores blank trailing rows
// ============================================================================

#[test]
fn rem_aud05_58_row_factor_matched_unstructured_ignores_blank_trailing_rows() {
    // 1. 10-row GroupedTable with complete data in rows 0..4 and blank untitled trailing rows 4..10.
    let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(6), 2, 10).unwrap();
    g.row_factor_title = String::from("Time");
    g.column_factor_title = String::from("Group");
    g.data_sets[0].title = String::from("Ctrl");
    g.data_sets[1].title = String::from("Drug");
    for r in 0..4 {
        g.row_titles[r] = format!("Day {}", r + 1);
        for c in 0..2 {
            for s in 0..6 {
                let y = 10.0 + 1.5 * (r as f64) + 2.0 * (c as f64) + 0.8 * (s as f64) + (((r * 3 + c * 5 + s) % 4) as f64 - 1.5) * 0.25;
                g.data_sets[c].cells[r][s] = Cell::number(y);
            }
        }
    }
    let sheet_trailing = DataSheet::new("rm-rfm-trail", "RFM-Trail", TableArchetype::Grouped(g.clone())).unwrap();
    let spec = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::RowFactorMatched,
        residual_covariance: ResidualCovariance::Unstructured,
        ..Default::default()
    };
    let out_trailing =
        run_factorial_anova(&sheet_trailing, &spec_for("r-tr", "rm-rfm-trail", AnalysisKind::FactorialAnova(spec.clone())), &spec).unwrap();
    let un_cl = out_trailing.checklist.iter().find(|c| c.assumption == "Unstructured covariance REML").expect("unstructured checklist item");
    assert_eq!(un_cl.status, ChecklistStatus::Info, "blank trailing rows must NOT flip Unstructured REML checklist to Warn: {un_cl:?}");
    assert!(
        !un_cl.explanation.contains("Excluded all-missing timepoint"),
        "blank trailing rows must NOT be reported as excluded timepoints: {}",
        un_cl.explanation
    );

    // 2. Adding a titled empty trailing row ("Day 5") DOES flag that row as excluded with ChecklistStatus::Warn.
    let mut g_titled = g.clone();
    g_titled.row_titles[4] = String::from("Day 5");
    let sheet_titled = DataSheet::new("rm-rfm-titled", "RFM-Titled", TableArchetype::Grouped(g_titled)).unwrap();
    let out_titled =
        run_factorial_anova(&sheet_titled, &spec_for("r-ti", "rm-rfm-titled", AnalysisKind::FactorialAnova(spec.clone())), &spec).unwrap();
    let un_cl_t = out_titled.checklist.iter().find(|c| c.assumption == "Unstructured covariance REML").expect("unstructured checklist item");
    assert_eq!(un_cl_t.status, ChecklistStatus::Warn);
    assert!(un_cl_t.explanation.contains("Day 5"), "titled empty row must be flagged: {}", un_cl_t.explanation);
    assert!(!un_cl_t.explanation.contains("Row 6"), "untitled blank rows 6..10 must still be ignored: {}", un_cl_t.explanation);

    // 3. An interior blank row (gap between active rows 0..2 and row 3) DOES flag the interior row.
    let mut g_interior = GroupedTable::new(SubcolumnMode::RawReplicates(6), 2, 8).unwrap();
    g_interior.data_sets[0].title = String::from("Ctrl");
    g_interior.data_sets[1].title = String::from("Drug");
    for (idx, &r) in [0_usize, 1, 3].iter().enumerate() {
        for c in 0..2 {
            for s in 0..6 {
                let y = 10.0 + 1.5 * (idx as f64) + 2.0 * (c as f64) + 0.8 * (s as f64) + (((idx * 3 + c * 5 + s) % 4) as f64 - 1.5) * 0.25;
                g_interior.data_sets[c].cells[r][s] = Cell::number(y);
            }
        }
    }
    let sheet_int = DataSheet::new("rm-rfm-int", "RFM-Int", TableArchetype::Grouped(g_interior)).unwrap();
    let out_int = run_factorial_anova(&sheet_int, &spec_for("r-in", "rm-rfm-int", AnalysisKind::FactorialAnova(spec.clone())), &spec).unwrap();
    let un_cl_i = out_int.checklist.iter().find(|c| c.assumption == "Unstructured covariance REML").expect("unstructured checklist item");
    assert_eq!(un_cl_i.status, ChecklistStatus::Warn);
    assert!(un_cl_i.explanation.contains("Row 3"), "interior blank row 2 ('Row 3') must be flagged: {}", un_cl_i.explanation);
}

// ============================================================================
// AUD-05-59: Lazy nested f_sf fallback & unstructured BFGS restart convergence
// ============================================================================

#[test]
fn rem_aud05_59_nested_lazy_p_and_unstructured_bfgs_restart() {
    // 1. Nested ANOVA with both KR and Satterthwaite df methods cleanly matches exact f_sf(F, df1, df2).
    let tab = make_nested_table(&[
        ("A", &[&[10.0, 10.5, 9.5], &[12.0, 12.2, 11.8]]),
        ("B", &[&[15.0, 15.4, 14.6], &[17.5, 17.8, 17.2]]),
        ("C", &[&[21.0, 21.3, 20.7], &[23.0, 23.1, 22.9]]),
    ]);
    for df_m in [MixedModelDfMethod::KenwardRoger, MixedModelDfMethod::Satterthwaite] {
        let res = nested_one_way_anova_with_df_method(&tab, false, df_m).unwrap();
        let active_df2 = match df_m {
            MixedModelDfMethod::KenwardRoger => res.df2_groups_kenward_roger,
            MixedModelDfMethod::Satterthwaite => res.df2_groups_satterthwaite,
        };
        let exp_p = f_sf(res.f_among_groups, res.df1_groups, active_df2).unwrap();
        assert_approx_eq(res.p_among_groups, exp_p, 1e-10, 1e-10, &format!("nested lazy p ({df_m:?})"));
    }

    // 2. Curved, strongly anisotropic t = 5, C = 30 unstructured covariance surface (condition ratio ~ 150,
    // alternating strong positive/negative correlations) with missing observations converges cleanly.
    let l_true: [[f64; 5]; 5] =
        [[0.5, 0.0, 0.0, 0.0, 0.0], [0.4, 0.7, 0.0, 0.0, 0.0], [-0.6, 0.5, 1.0, 0.0, 0.0], [0.8, -0.7, 0.8, 1.4, 0.0], [-1.0, 0.9, -1.1, 1.2, 2.0]];
    let mut state: u64 = 0xABCD_1234_9876_5432;
    let mut c_ids = Vec::new();
    let mut t_ids = Vec::new();
    let mut y = Vec::new();
    for s in 0..30_usize {
        let z = [lcg_norm(&mut state), lcg_norm(&mut state), lcg_norm(&mut state), lcg_norm(&mut state), lcg_norm(&mut state)];
        for t in 0..5_usize {
            if (s == 3 && t == 2) || (s == 11 && t == 4) || (s == 19 && t == 1) {
                continue;
            }
            let mut e_t = 0.0;
            for k in 0..=t {
                e_t += l_true[t][k] * z[k];
            }
            c_ids.push(s);
            t_ids.push(t);
            y.push(20.0 + 1.5 * (t as f64) + e_t);
        }
    }
    let x = sum_contrast_one_way_matrix(&t_ids, 5);
    let labels: Vec<String> = (1..=5).map(|i| format!("T{i}")).collect();
    let fit = fit_unstructured_rm_reml(&x, &y, &c_ids, &t_ids, &labels, &[("Time", 1, 4)]).unwrap();
    assert!(
        fit.converged,
        "curved anisotropic unstructured REML must converge cleanly (iters={}, fallback={:?})",
        fit.iterations, fit.fallback_reason
    );
    assert_eq!(fit.fallback_reason, None, "iters={}, cov={:?}", fit.iterations, fit.covariance_matrix);
    assert!(fit.kenward_roger_df[0].is_finite() && fit.kenward_roger_df[0] > 1.0);
    assert!(fit.fixed_terms[0].f_statistic.is_some_and(f64::is_finite));
}
