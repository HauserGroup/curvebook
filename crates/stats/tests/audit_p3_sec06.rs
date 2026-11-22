#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression suite covering SEC-06 audit findings `FA-SEC06-01` through `FA-SEC06-11`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, CoxTieMethod, DataSheet,
    FactorialAnovaSpec, GlmFamily, GlmSpec, GroupedTable, MixedModelDfMethod, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec,
    MultivariateSpec, NestedAnalysisSpec, NestedTable, OneWayAnovaSpec, OneWayMethod, PassingBablokSpec, RepeatedMeasuresDesign, ResidualCovariance,
    ResultCell, ResultSheet, ResultStatus, ResultTab, SheetId, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, VariableRole, XColumnMode,
    XyTable,
};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::anova::{
    FactorialObservations, fit_factorial_anova, fit_multi_vc_reml, fit_multi_vc_reml_with_df_method, fit_random_intercept_reml,
    fit_random_intercept_reml_with_df_method, fit_unstructured_rm_reml, fit_unstructured_rm_reml_with_df_method, nested_one_way_anova,
    run_factorial_anova, run_nested_analysis, run_one_way_anova,
};
use curvebook_stats::multivariate::{kendall_tau_b, run_multivariate_analysis, spearman_correlation};
use curvebook_stats::regression::{multiple_linear_regression, passing_bablok_regression, run_glm_analysis, run_passing_bablok_analysis};
use curvebook_stats::survival::{CoxSubject, cox_ph_regression};
use curvebook_stats::{StatsError, run_analysis};

fn spec_for(id: &str, src: &str, kind: AnalysisKind) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new(id), SheetId::new(src), kind).unwrap()
}

fn spec_99(id: &str, src: &str, kind: AnalysisKind) -> AnalysisSpec {
    let mut spec = spec_for(id, src, kind);
    spec.confidence_level = ConfidenceLevel::PERCENT_99;
    spec
}

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

fn scalar_val(sheet: &ResultSheet, key: &str) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

fn assert_p_cell_stars_empty(cell: &ResultCell, ctx: &str) {
    match cell {
        ResultCell::PValue { raw, stars, .. } => {
            assert!(raw.is_finite(), "{ctx}: raw P must be finite");
            assert!(
                !stars.contains('*') && (stars == "ns" || stars.is_empty()),
                "{ctx}: expected non-starred P cell at alpha=0.01 for P={raw}, got `{stars}`"
            );
        }
        other => panic!("{ctx}: expected PValue cell, got {other:?}"),
    }
}

fn make_presum_column_table(mode: SubcolumnMode) -> DataSheet {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 3, 2).unwrap();
    for c in 0..3 {
        col.data_sets[c].title = format!("G{c}");
        col.data_sets[c].cells[0][0] = Cell::number(10.0 + (c as f64) * 3.0);
        col.data_sets[c].cells[0][1] = Cell::number(1.5);
        col.data_sets[c].cells[0][2] = Cell::number(5.0);
        col.data_sets[c].cells[1][0] = Cell::number(12.0 + (c as f64) * 3.0);
        col.data_sets[c].cells[1][1] = Cell::number(1.8);
        col.data_sets[c].cells[1][2] = Cell::number(5.0);
    }
    DataSheet::new(SheetId::new("col-presum"), "PresumCol", TableArchetype::Column(col)).unwrap()
}

// ============================================================================
// FA-SEC06-01: RM one-way & Friedman reject pre-summarized ColumnTable
// ============================================================================

#[test]
fn fa_sec06_01_rm_one_way_and_friedman_reject_presummarized_column_table() {
    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let sheet = make_presum_column_table(mode);
        for method in [
            OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
            OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            OneWayMethod::Friedman,
        ] {
            let one_way = OneWayAnovaSpec { method, ..OneWayAnovaSpec::default() };
            let spec = spec_for("r-rm", "col-presum", AnalysisKind::OneWayAnova(one_way.clone()));
            let err = run_one_way_anova(&sheet, &spec, &one_way).unwrap_err();
            assert!(matches!(err, StatsError::RequiresRawReplicates), "mode={mode:?} method={method:?} got {err:?}");
        }
    }
}

// ============================================================================
// FA-SEC06-02: Multivariate guards pre-summarized & handles XY RawReplicates
// ============================================================================

#[test]
fn fa_sec06_02_multivariate_guards_presummarized_and_handles_xy_replicates() {
    let mv_kinds = vec![
        MultivariateSpec::CorrelationMatrix { spearman_rank: false },
        MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: None },
        MultivariateSpec::Clustering { k_means_k: Some(2), distance: ClusterDistance::Euclidean, linkage: ClusterLinkage::Ward, k_sweep_max: None },
        MultivariateSpec::MultipleImputation { imputations_m: 3, max_iters: 5, seed: 42, response_column: None, predictor_columns: Vec::new() },
    ];

    // 1. ColumnTable pre-summarized rejected cleanly.
    let col_presum = make_presum_column_table(SubcolumnMode::MeanSdN);
    for kind in &mv_kinds {
        let spec = spec_for("r-mv-col", "col-presum", AnalysisKind::Multivariate(kind.clone()));
        let err = run_multivariate_analysis(&col_presum, &spec, kind).unwrap_err();
        assert!(matches!(err, StatsError::RequiresRawReplicates), "Column(MeanSdN) {kind:?} got {err:?}");
    }

    // 2. XyTable pre-summarized Y rejected cleanly.
    let mut xy_presum_y = XyTable::new(SubcolumnMode::MeanSdN, 2, 5).unwrap();
    for r in 0..5 {
        xy_presum_y.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        for c in 0..2 {
            xy_presum_y.data_sets[c].cells[r][0] = Cell::number(10.0 + (r as f64) + (c as f64));
            xy_presum_y.data_sets[c].cells[r][1] = Cell::number(1.2);
            xy_presum_y.data_sets[c].cells[r][2] = Cell::number(4.0);
        }
    }
    let sheet_xy_presum_y = DataSheet::new(SheetId::new("xy-presum-y"), "XyPresumY", TableArchetype::Xy(xy_presum_y)).unwrap();
    for kind in &mv_kinds {
        let spec = spec_for("r-mv-xyp", "xy-presum-y", AnalysisKind::Multivariate(kind.clone()));
        let err = run_multivariate_analysis(&sheet_xy_presum_y, &spec, kind).unwrap_err();
        assert!(matches!(err, StatsError::RequiresRawReplicates), "Xy(MeanSdN) {kind:?} got {err:?}");
    }

    // 3. XyTable pre-summarized X rejected cleanly when X is selected.
    let mut xy_presum_x =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::Single, 2, 5).unwrap();
    for r in 0..5 {
        xy_presum_x.set_x_subcell(r, 0, Cell::number((r + 1) as f64)).unwrap();
        xy_presum_x.set_x_subcell(r, 1, Cell::number(0.5)).unwrap();
        xy_presum_x.set_x_subcell(r, 2, Cell::number(3.0)).unwrap();
        xy_presum_x.data_sets[0].cells[r][0] = Cell::number(2.0 * (r as f64) + 1.0);
        xy_presum_x.data_sets[1].cells[r][0] = Cell::number(3.0 * (r as f64) + 2.0);
    }
    let sheet_xy_presum_x = DataSheet::new(SheetId::new("xy-presum-x"), "XyPresumX", TableArchetype::Xy(xy_presum_x)).unwrap();
    for kind in &mv_kinds {
        let spec = spec_for("r-mv-xypx", "xy-presum-x", AnalysisKind::Multivariate(kind.clone()));
        let err = run_multivariate_analysis(&sheet_xy_presum_x, &spec, kind).unwrap_err();
        assert!(matches!(err, StatsError::RequiresRawReplicates), "Xy(X=MeanSdN) {kind:?} got {err:?}");
    }

    // 4. XyTable with RawReplicates(3) incorporates replicates 1..=2 across all 4 analyses and matches flattened Xy(Single).
    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(3), 2, 5).unwrap();
    xy_rep.x_title = String::from("X");
    xy_rep.data_sets[0].title = String::from("Y1");
    xy_rep.data_sets[1].title = String::from("Y2");
    let mut xy_flat = XyTable::new(SubcolumnMode::Single, 2, 15).unwrap();
    xy_flat.x_title = String::from("X");
    xy_flat.data_sets[0].title = String::from("Y1");
    xy_flat.data_sets[1].title = String::from("Y2");

    for r in 0..5 {
        let xv = (r + 1) as f64;
        xy_rep.set_x_cell(r, Cell::number(xv)).unwrap();
        let y1_reps = [xv, 10.0 * xv + 1.0, -5.0 * xv + 3.0];
        let y2_reps = [2.0 * xv + 0.5, 4.0 * xv - 1.0, 0.5 * xv + 8.0];
        for s in 0..3 {
            xy_rep.data_sets[0].cells[r][s] = Cell::number(y1_reps[s]);
            xy_rep.data_sets[1].cells[r][s] = Cell::number(y2_reps[s]);

            let flat_r = r * 3 + s;
            xy_flat.set_x_cell(flat_r, Cell::number(xv)).unwrap();
            xy_flat.data_sets[0].cells[flat_r][0] = Cell::number(y1_reps[s]);
            xy_flat.data_sets[1].cells[flat_r][0] = Cell::number(y2_reps[s]);
        }
    }
    let sheet_rep = DataSheet::new(SheetId::new("xy-rep"), "XyRep", TableArchetype::Xy(xy_rep.clone())).unwrap();
    let sheet_flat = DataSheet::new(SheetId::new("xy-flat"), "XyFlat", TableArchetype::Xy(xy_flat)).unwrap();

    for kind in &mv_kinds {
        let res_rep = run_multivariate_analysis(&sheet_rep, &spec_for("r-rep", "xy-rep", AnalysisKind::Multivariate(kind.clone())), kind).unwrap();
        let res_flat =
            run_multivariate_analysis(&sheet_flat, &spec_for("r-flat", "xy-flat", AnalysisKind::Multivariate(kind.clone())), kind).unwrap();
        assert_eq!(res_rep.scalars, res_flat.scalars, "RawReplicates(3) scalars must match flattened 15-row Xy(Single) for {kind:?}");
        assert_eq!(res_rep.tabs[0].rows, res_flat.tabs[0].rows, "RawReplicates(3) tab rows must match flattened 15-row Xy(Single) for {kind:?}");
    }

    // Verify missing cell in replicate subcolumn 1 is seen and imputed by MICE.
    let mut xy_rep_miss = xy_rep;
    xy_rep_miss.data_sets[1].cells[2][1] = Cell::empty();
    let sheet_rep_miss = DataSheet::new(SheetId::new("xy-rep-miss"), "XyRepMiss", TableArchetype::Xy(xy_rep_miss)).unwrap();
    let mi_spec =
        MultivariateSpec::MultipleImputation { imputations_m: 3, max_iters: 5, seed: 7, response_column: Some(1), predictor_columns: vec![0, 2] };
    let mi_res =
        run_multivariate_analysis(&sheet_rep_miss, &spec_for("r-mi-rep", "xy-rep-miss", AnalysisKind::Multivariate(mi_spec.clone())), &mi_spec)
            .unwrap();
    assert_eq!(scalar_val(&mi_res, "Total missing cells"), 1.0);
}

// ============================================================================
// FA-SEC06-03: fit_factorial_anova rejects malformed FactorialObservations
// ============================================================================

#[test]
fn fa_sec06_03_fit_factorial_anova_rejects_malformed_observations_without_panic() {
    // 1. factor_levels.len() != factor_names.len()
    let bad_levels_len = FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B")],
        factor_levels: vec![vec![String::from("A1"), String::from("A2")]],
        rows: vec![(vec![0, 0], 1.0), (vec![0, 1], 2.0), (vec![1, 0], 3.0)],
    };
    let err1 = fit_factorial_anova(&bad_levels_len, 2, curvebook_doc::SumOfSquaresType::TypeIII).unwrap_err();
    assert!(matches!(err1, StatsError::DimensionMismatch { expected: 2, actual: 1 }), "got {err1:?}");

    // 2. Ragged row lvls.len() != factor_names.len()
    let ragged_row = FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B")],
        factor_levels: vec![vec![String::from("A1"), String::from("A2")], vec![String::from("B1"), String::from("B2")]],
        rows: vec![(vec![0, 0], 1.0), (vec![0], 2.0), (vec![1, 1], 3.0)],
    };
    let err2 = fit_factorial_anova(&ragged_row, 2, curvebook_doc::SumOfSquaresType::TypeIII).unwrap_err();
    assert!(matches!(err2, StatsError::DimensionMismatch { expected: 2, actual: 1 }), "got {err2:?}");

    // 3. Out-of-range level index
    let oob_lvl = FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B")],
        factor_levels: vec![vec![String::from("A1"), String::from("A2")], vec![String::from("B1"), String::from("B2")]],
        rows: vec![(vec![0, 0], 1.0), (vec![0, 5], 2.0), (vec![1, 1], 3.0)],
    };
    let err3 = fit_factorial_anova(&oob_lvl, 2, curvebook_doc::SumOfSquaresType::TypeIII).unwrap_err();
    assert!(matches!(err3, StatsError::InvalidParameter { name: "factor_level_index", .. }), "got {err3:?}");
}

// ============================================================================
// FA-SEC06-04: Public REML entrypoints reject invalid/overflowing term ranges
// ============================================================================

#[test]
fn fa_sec06_04_public_reml_entrypoints_reject_invalid_and_overflowing_terms() {
    // Full-rank and rank-deficient 6x3 design matrices.
    let x_full =
        DenseMatrix::from_row_major(6, 3, vec![1.0, 1.0, 1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0, 1.0, -1.0, -1.0])
            .unwrap();
    let x_rank_def =
        DenseMatrix::from_row_major(6, 3, vec![1.0, 1.0, 2.0, 1.0, 1.0, 2.0, 1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 2.0, 1.0, -1.0, 0.0]).unwrap();
    let y = [10.0, 11.2, 12.1, 13.4, 10.5, 13.0];
    let c_ids = [0, 0, 1, 1, 2, 2];
    let t_ids = [0, 1, 0, 1, 0, 1];
    let labels = vec![String::from("T0"), String::from("T1")];
    let g_mat = DenseMatrix::from_row_major(6, 6, (0..36).map(|idx| if c_ids[idx / 6] == c_ids[idx % 6] { 1.0 } else { 0.0 }).collect()).unwrap();

    for x in [&x_full, &x_rank_def] {
        for bad_terms in [[("T", 4, 1)], [("T", usize::MAX, 1)], [("T", 1, usize::MAX)]] {
            let e1 = fit_random_intercept_reml(x, &y, &c_ids, &bad_terms).unwrap_err();
            assert!(matches!(e1, StatsError::InvalidParameter { name: "terms", .. }), "RI got {e1:?}");

            let e2 = fit_random_intercept_reml_with_df_method(x, &y, &c_ids, &bad_terms, MixedModelDfMethod::Satterthwaite).unwrap_err();
            assert!(matches!(e2, StatsError::InvalidParameter { name: "terms", .. }), "RI-df got {e2:?}");

            let e3 = fit_multi_vc_reml(x, &y, &[("S", g_mat.clone(), 3)], &bad_terms).unwrap_err();
            assert!(matches!(e3, StatsError::InvalidParameter { name: "terms", .. }), "MultiVC got {e3:?}");

            let e4 = fit_multi_vc_reml_with_df_method(x, &y, &[("S", g_mat.clone(), 3)], &bad_terms, MixedModelDfMethod::Satterthwaite).unwrap_err();
            assert!(matches!(e4, StatsError::InvalidParameter { name: "terms", .. }), "MultiVC-df got {e4:?}");

            let e5 = fit_unstructured_rm_reml(x, &y, &c_ids, &t_ids, &labels, &bad_terms).unwrap_err();
            assert!(matches!(e5, StatsError::InvalidParameter { name: "terms", .. }), "Unstruct got {e5:?}");

            let e6 =
                fit_unstructured_rm_reml_with_df_method(x, &y, &c_ids, &t_ids, &labels, &bad_terms, MixedModelDfMethod::Satterthwaite).unwrap_err();
            assert!(matches!(e6, StatsError::InvalidParameter { name: "terms", .. }), "Unstruct-df got {e6:?}");
        }
    }
}

// ============================================================================
// FA-SEC06-05: Compound-symmetry RM REML handles all-missing-cell collinearity
// ============================================================================

#[test]
fn fa_sec06_05_compound_symmetry_rm_reml_handles_all_missing_cell_collinearity() {
    // 2x2 GroupedTable with 4 subjects where ALL 4 subjects miss cell (row=1, col=1).
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 2).unwrap();
    grouped.row_factor_title = String::from("Time");
    grouped.column_factor_title = String::from("Group");
    grouped.row_titles = vec![String::from("T1"), String::from("T2")];
    grouped.data_sets[0].title = String::from("Control");
    grouped.data_sets[1].title = String::from("Treated");

    let c0_r0 = [10.0, 10.4, 9.8, 10.2];
    let c0_r1 = [14.0, 14.3, 13.7, 14.1];
    let c1_r0 = [12.5, 12.9, 12.1, 12.7];
    for s in 0..4 {
        grouped.data_sets[0].cells[0][s] = Cell::number(c0_r0[s]);
        grouped.data_sets[0].cells[1][s] = Cell::number(c0_r1[s]);
        grouped.data_sets[1].cells[0][s] = Cell::number(c1_r0[s]);
        // cell (row=1, col=1) left completely missing across all 4 subjects.
    }
    let sheet = DataSheet::new(SheetId::new("g-miss-cell"), "MissCell", TableArchetype::Grouped(grouped)).unwrap();

    for design in [RepeatedMeasuresDesign::ColumnFactorMatched, RepeatedMeasuresDesign::RowFactorMatched, RepeatedMeasuresDesign::BothFactorsMatched]
    {
        let fac_spec = FactorialAnovaSpec {
            repeated_measures: design,
            residual_covariance: ResidualCovariance::CompoundSymmetry,
            fit_reml_on_missing: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
            ..FactorialAnovaSpec::default()
        };
        let spec = spec_for("r-cs-collinear", "g-miss-cell", AnalysisKind::FactorialAnova(fac_spec.clone()));
        let res = run_factorial_anova(&sheet, &spec, &fac_spec).unwrap();
        assert_eq!(res.status, ResultStatus::UpToDate);
        let f_time = scalar_val(&res, "Time: F");
        let f_group = scalar_val(&res, "Group: F");
        assert!(f_time.is_finite() && f_time > 0.0, "{design:?}: Time F={f_time}");
        assert!(f_group.is_finite() && f_group > 0.0, "{design:?}: Group F={f_group}");
        assert!(res.tabs.iter().any(|t| t.id == "factorial-anova"));
    }

    // Direct collinear X calls to fit_random_intercept_reml and fit_multi_vc_reml also succeed.
    let x_collinear = DenseMatrix::from_row_major(
        8,
        4,
        vec![
            1.0, 1.0, 1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0,
            1.0, 1.0, 1.0, -1.0, 1.0, -1.0,
        ],
    )
    .unwrap();
    let y = [10.0, 14.0, 12.5, 10.4, 14.3, 12.9, 9.8, 13.7];
    let c_ids = [0, 0, 0, 1, 1, 1, 2, 2];
    let ri = fit_random_intercept_reml(&x_collinear, &y, &c_ids, &[("A", 1, 1), ("B", 2, 1), ("A × B", 3, 1)]).unwrap();
    assert_eq!(ri.fixed_terms.len(), 2);
    assert!(ri.fixed_terms.iter().all(|t| t.f_statistic.is_some_and(f64::is_finite)));

    let g_mat = DenseMatrix::from_row_major(8, 8, (0..64).map(|idx| if c_ids[idx / 8] == c_ids[idx % 8] { 1.0 } else { 0.0 }).collect()).unwrap();
    let mv = fit_multi_vc_reml(&x_collinear, &y, &[("S", g_mat, 3)], &[("A", 1, 1), ("B", 2, 1), ("A × B", 3, 1)]).unwrap();
    assert_eq!(mv.fixed_terms.len(), 2);
    assert!(mv.converged);
}

// ============================================================================
// FA-SEC06-06: Multi-VC REML convergence & boundary diagnostics
// ============================================================================

#[test]
fn fa_sec06_06_multi_vc_reml_tracks_convergence_and_boundary_diagnostics() {
    // 1. Balanced 2x2 BothFactorsMatched with zero variance in one random stratum hits boundary and emits checklist/footnote.
    let mut grouped_bnd = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 3).unwrap();
    grouped_bnd.row_factor_title = String::from("Row");
    grouped_bnd.column_factor_title = String::from("Col");
    for s in 0..4 {
        let s_off = (s as f64 - 1.5) * 2.0;
        for r in 0..3 {
            for c in 0..2 {
                let sc_off = if c == 0 { -0.5 } else { 0.5 } * (s as f64 - 1.5);
                let val = 10.0 + s_off + (r as f64) * 3.0 + (c as f64) * 4.0 + sc_off + 0.05 * (((s * 6 + r * 2 + c) as f64) * 1.7).sin();
                grouped_bnd.data_sets[c].cells[r][s] = Cell::number(val);
            }
        }
    }
    let sheet_bnd = DataSheet::new(SheetId::new("g-mvc-bnd"), "MvcBnd", TableArchetype::Grouped(grouped_bnd)).unwrap();
    let fac_bnd = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        mixed_effects_reml: true,
        ..FactorialAnovaSpec::default()
    };
    let res_bnd =
        run_factorial_anova(&sheet_bnd, &spec_for("r-mvc-bnd", "g-mvc-bnd", AnalysisKind::FactorialAnova(fac_bnd.clone())), &fac_bnd).unwrap();
    let vc_tab = tab_by_id(&res_bnd, "mixed-reml");
    let conv_chk = res_bnd
        .checklist
        .iter()
        .find(|c| c.assumption == "REML convergence and variance boundaries")
        .expect("REML convergence checklist item must be emitted");
    assert!(matches!(conv_chk.status, ChecklistStatus::Info | ChecklistStatus::Pass));
    if conv_chk.status == ChecklistStatus::Info {
        assert!(vc_tab.footnotes.iter().any(|f| f.contains("boundary")));
    }

    // 2. Redundant identical random-effect strata force MultiVcRemlResult::converged/at_boundary reporting cleanly.
    let x = DenseMatrix::from_row_major(8, 2, vec![1.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, -1.0]).unwrap();
    let y = [10.0, 12.1, 10.5, 12.4, 9.8, 11.9, 10.2, 12.0];
    let cids = [0, 0, 1, 1, 2, 2, 3, 3];
    let g = DenseMatrix::from_row_major(8, 8, (0..64).map(|idx| if cids[idx / 8] == cids[idx % 8] { 1.0 } else { 0.0 }).collect()).unwrap();
    let mres = fit_multi_vc_reml(&x, &y, &[("S1", g.clone(), 4)], &[("T", 1, 1)]).unwrap();
    assert!(mres.converged);
    assert!(!mres.at_boundary);
}

// ============================================================================
// FA-SEC06-07: Passing-Bablok XY RawReplicates & float sanitization
// ============================================================================

#[test]
fn fa_sec06_07_passing_bablok_handles_xy_replicates_and_sanitizes_overflow() {
    // 1. XyTable(RawReplicates(3)) uses all 3 replicate subcolumns (N = 15, matching 15-row Single table).
    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(3), 1, 5).unwrap();
    let mut xy_flat = XyTable::new(SubcolumnMode::Single, 1, 15).unwrap();
    for r in 0..5 {
        let xv = (r + 1) as f64;
        xy_rep.set_x_cell(r, Cell::number(xv)).unwrap();
        let ys = [xv + 0.1, 1.5 * xv + 0.2, 2.0 * xv - 0.1];
        for s in 0..3 {
            xy_rep.data_sets[0].cells[r][s] = Cell::number(ys[s]);
            xy_flat.set_x_cell(r * 3 + s, Cell::number(xv)).unwrap();
            xy_flat.data_sets[0].cells[r * 3 + s][0] = Cell::number(ys[s]);
        }
    }
    let sheet_rep = DataSheet::new(SheetId::new("pb-rep"), "PbRep", TableArchetype::Xy(xy_rep)).unwrap();
    let sheet_flat = DataSheet::new(SheetId::new("pb-flat"), "PbFlat", TableArchetype::Xy(xy_flat)).unwrap();
    let pb_spec = PassingBablokSpec { x_column: 0, y_column: 1 };
    let res_rep = run_passing_bablok_analysis(&sheet_rep, &spec_for("r-pb-rep", "pb-rep", AnalysisKind::PassingBablok(pb_spec)), &pb_spec).unwrap();
    let res_flat =
        run_passing_bablok_analysis(&sheet_flat, &spec_for("r-pb-flat", "pb-flat", AnalysisKind::PassingBablok(pb_spec)), &pb_spec).unwrap();
    assert_eq!(scalar_val(&res_rep, "N"), 15.0);
    assert_eq!(scalar_val(&res_rep, "Slope"), scalar_val(&res_flat, "Slope"));
    assert_eq!(tab_by_id(&res_rep, "pb-residuals").row_labels.len(), 15);
    assert!(tab_by_id(&res_rep, "pb-residuals").row_labels.iter().any(|l| l.contains("(rep 2)")));

    // 2. Extreme magnitude inputs causing curve overflow are rejected by sanitize_result_sheet_floats.
    let mut xy_big = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    let big_pts = [(0.0, 0.0), (1.0, 1e10), (2.0, 2e10), (3.0, 3e10), (1e307, 1e307)];
    for (r, &(x, y)) in big_pts.iter().enumerate() {
        xy_big.set_x_cell(r, Cell::number(x)).unwrap();
        xy_big.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet_big = DataSheet::new(SheetId::new("pb-big"), "PbBig", TableArchetype::Xy(xy_big)).unwrap();
    let err_big =
        run_passing_bablok_analysis(&sheet_big, &spec_for("r-pb-big", "pb-big", AnalysisKind::PassingBablok(pb_spec)), &pb_spec).unwrap_err();
    assert!(matches!(err_big, StatsError::Math(MathError::DomainError { name: "non_finite_curve_point", .. })), "got {err_big:?}");
}

// ============================================================================
// FA-SEC06-08: No-intercept VIF detects collinearity properly
// ============================================================================

#[test]
fn fa_sec06_08_no_intercept_vif_detects_affine_collinearity() {
    let x1 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let jitter = [0.001, -0.001, 0.002, -0.002, 0.001, -0.001];
    let x2: Vec<f64> = x1.iter().zip(jitter.iter()).map(|(&x, &j)| 1007.0 - x + j).collect();
    let y: Vec<f64> = x1.iter().zip(x2.iter()).map(|(&a, &b)| 2.0 * a + 0.5 * b + 1.0).collect();

    let x_int = DenseMatrix::from_row_major(6, 3, (0..6).flat_map(|i| [1.0, x1[i], x2[i]]).collect()).unwrap();
    let mlr_int = multiple_linear_regression(
        &x_int,
        &y,
        &[String::from("Int"), String::from("X1"), String::from("X2")],
        true,
        true,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();

    let x_noint = DenseMatrix::from_row_major(6, 2, (0..6).flat_map(|i| [x1[i], x2[i]]).collect()).unwrap();
    let mlr_noint =
        multiple_linear_regression(&x_noint, &y, &[String::from("X1"), String::from("X2")], false, true, ConfidenceLevel::DEFAULT_95).unwrap();

    let vif_int = mlr_int.terms[1].vif.unwrap();
    let vif_noint_0 = mlr_noint.terms[0].vif.unwrap();
    let vif_noint_1 = mlr_noint.terms[1].vif.unwrap();
    assert!(vif_noint_0 > 1e5, "expected high centered VIF without intercept, got {vif_noint_0}");
    assert!((vif_noint_0 - vif_int).abs() / vif_int < 1e-6);
    assert!((vif_noint_1 - vif_int).abs() / vif_int < 1e-6);
}

// ============================================================================
// FA-SEC06-09: SEC-06 ResultSheets honor confidence.alpha() for P stars
// ============================================================================

#[test]
fn fa_sec06_09_sec06_sheets_honor_confidence_alpha_for_pvalue_stars() {
    // 1. Passing-Bablok CUSUM P ~ 0.0212 at 99% confidence (alpha = 0.01) -> no stars.
    let mut mv_pb = MultipleVariablesTable::new(2, 20).unwrap();
    mv_pb.variables[0].title = String::from("MethodA");
    mv_pb.variables[1].title = String::from("MethodB");
    let xs_pb: Vec<f64> = (1..=20).map(f64::from).collect();
    let ys_pb: Vec<f64> = xs_pb.iter().map(|&x| 0.25 * x * x + 0.5 * x + 1.0).collect();
    for r in 0..20 {
        mv_pb.variables[0].cells[r] = Cell::number(xs_pb[r]);
        mv_pb.variables[1].cells[r] = Cell::number(ys_pb[r]);
    }
    let pb_direct = passing_bablok_regression(&xs_pb, &ys_pb, ConfidenceLevel::PERCENT_99).unwrap();
    assert!(pb_direct.cusum_p_value > 0.01 && pb_direct.cusum_p_value < 0.05, "P={}", pb_direct.cusum_p_value);
    assert!(pb_direct.linearity_passed);
    let sheet_pb = DataSheet::new(SheetId::new("mv-pb-99"), "Pb99", TableArchetype::MultipleVariables(mv_pb)).unwrap();
    let pb_spec = PassingBablokSpec { x_column: 0, y_column: 1 };
    let res_pb = run_passing_bablok_analysis(&sheet_pb, &spec_99("r-pb99", "mv-pb-99", AnalysisKind::PassingBablok(pb_spec)), &pb_spec).unwrap();
    assert_p_cell_stars_empty(&tab_by_id(&res_pb, "pb-summary").rows[5][0], "PB CUSUM 99%");

    // 2. Linear GLM with slope P in (0.01, 0.05) at 99% confidence -> no stars.
    let mut mv_glm = MultipleVariablesTable::new(2, 6).unwrap();
    mv_glm.variables[0].title = String::from("Y");
    mv_glm.variables[1].title = String::from("X");
    let x_glm = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let y_glm = [1.0, 1.0, 3.1, 1.9, 3.8, 4.0];
    for r in 0..6 {
        mv_glm.variables[0].cells[r] = Cell::number(y_glm[r]);
        mv_glm.variables[1].cells[r] = Cell::number(x_glm[r]);
    }
    let sheet_glm = DataSheet::new(SheetId::new("mv-glm-99"), "Glm99", TableArchetype::MultipleVariables(mv_glm)).unwrap();
    let glm_spec =
        GlmSpec { family: GlmFamily::Linear, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: false };
    let res_glm =
        run_glm_analysis(&sheet_glm, &spec_99("r-glm99", "mv-glm-99", AnalysisKind::GeneralizedLinearModel(glm_spec.clone())), &glm_spec).unwrap();
    let p_slope = scalar_val(&res_glm, "P value");
    assert!(p_slope > 0.01 && p_slope < 0.05, "expected GLM P in (0.01, 0.05), got {p_slope}");
    assert_p_cell_stars_empty(&tab_by_id(&res_glm, "glm-linear-params").rows[1][3], "Linear GLM param 99%");
    assert_p_cell_stars_empty(&tab_by_id(&res_glm, "glm-linear-anova").rows[0][4], "Linear GLM ANOVA 99%");
    assert_p_cell_stars_empty(&tab_by_id(&res_glm, "glm-linear-type-iii").rows[0][4], "Linear GLM Type III 99%");

    // 3. One-way ANOVA & Correlation Matrix at 99% confidence -> no stars when P in (0.01, 0.05).
    let mut col_ow = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 6).unwrap();
    let g0 = [1.0, 1.2, 1.5, 1.8, 2.0, 2.1];
    let g1 = [1.7, 1.9, 2.2, 2.5, 2.7, 2.8];
    for r in 0..6 {
        col_ow.data_sets[0].cells[r][0] = Cell::number(g0[r]);
        col_ow.data_sets[1].cells[r][0] = Cell::number(g1[r]);
    }
    let sheet_ow = DataSheet::new(SheetId::new("ow-99"), "Ow99", TableArchetype::Column(col_ow)).unwrap();
    let ow_spec = OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..OneWayAnovaSpec::default() };
    let res_ow = run_one_way_anova(&sheet_ow, &spec_99("r-ow99", "ow-99", AnalysisKind::OneWayAnova(ow_spec.clone())), &ow_spec).unwrap();
    let p_ow = scalar_val(&res_ow, "P value");
    assert!(p_ow > 0.01 && p_ow < 0.05, "expected OneWay ANOVA P in (0.01, 0.05), got {p_ow}");
    assert_p_cell_stars_empty(&tab_by_id(&res_ow, "anova-table").rows[0][4], "OneWay ANOVA 99%");

    let corr_spec = MultivariateSpec::CorrelationMatrix { spearman_rank: false };
    let res_corr =
        run_multivariate_analysis(&sheet_glm, &spec_99("r-corr99", "mv-glm-99", AnalysisKind::Multivariate(corr_spec.clone())), &corr_spec).unwrap();
    assert_p_cell_stars_empty(&tab_by_id(&res_corr, "corr-matrix-p").rows[0][1], "Corr matrix P 99%");
    assert_p_cell_stars_empty(&tab_by_id(&res_corr, "corr-matrix-fdr").rows[0][1], "Corr matrix q 99%");

    // 4. Nested ANOVA at 99% confidence -> no stars when P in (0.01, 0.05).
    let mut nt = NestedTable::new(2, 3, 3).unwrap();
    let n_vals = [[[1.0, 1.1, 1.2], [2.0, 2.1, 2.2], [3.0, 3.1, 3.2]], [[4.0, 4.1, 4.2], [5.0, 5.1, 5.2], [6.0, 6.1, 6.2]]];
    for g in 0..2 {
        for b in 0..3 {
            for r in 0..3 {
                nt.groups[g].cells[r][b] = Cell::number(n_vals[g][b][r]);
            }
        }
    }
    let sheet_nt = DataSheet::new(SheetId::new("nt-99"), "Nt99", TableArchetype::Nested(nt)).unwrap();
    let nt_spec = NestedAnalysisSpec::default();
    let res_nt = run_nested_analysis(&sheet_nt, &spec_99("r-nt99", "nt-99", AnalysisKind::NestedComparison(nt_spec.clone())), &nt_spec).unwrap();
    let p_nt = scalar_val(&res_nt, "P value (groups)");
    assert!(p_nt > 0.01 && p_nt < 0.05, "expected nested P in (0.01, 0.05), got {p_nt}");
    assert_p_cell_stars_empty(&tab_by_id(&res_nt, "nested-anova").rows[0][4], "Nested group P 99%");

    // 5. Survival log-rank P in (0.01, 0.05) at 99% confidence -> no stars.
    let mut mv_surv = MultipleVariablesTable::new(3, 10).unwrap();
    mv_surv.variables[0].title = String::from("Time");
    mv_surv.variables[1].title = String::from("Status");
    mv_surv.variables[2].title = String::from("Arm");
    mv_surv.variables[2].role = VariableRole::Categorical;
    let s_rows = [
        (2.0, 1.0, "A"),
        (3.0, 1.0, "A"),
        (4.0, 1.0, "A"),
        (5.0, 1.0, "A"),
        (9.0, 1.0, "A"),
        (6.0, 1.0, "B"),
        (8.0, 1.0, "B"),
        (11.0, 1.0, "B"),
        (12.0, 1.0, "B"),
        (14.0, 1.0, "B"),
    ];
    for (r, &(t, st, arm)) in s_rows.iter().enumerate() {
        mv_surv.variables[0].cells[r] = Cell::number(t);
        mv_surv.variables[1].cells[r] = Cell::number(st);
        mv_surv.variables[2].cells[r] = Cell::text(arm);
    }
    let sheet_surv = DataSheet::new(SheetId::new("surv-99"), "Surv99", TableArchetype::MultipleVariables(mv_surv)).unwrap();
    let surv_spec = SurvivalAnalysisSpec::KaplanMeier {
        mv_time_column: Some(0),
        mv_status_column: Some(1),
        mv_group_column: Some(2),
        log_rank_mantel_cox: true,
        gehan_breslow_wilcoxon: false,
        trend_test: false,
        asymmetrical_greenwood_ci: true,
        multiplicity: MultiplicitySpec::default(),
    };
    let res_surv = run_analysis(&sheet_surv, &spec_99("r-surv99", "surv-99", AnalysisKind::Survival(surv_spec))).unwrap();
    let p_lr = scalar_val(&res_surv, "Log-rank P value");
    assert!(p_lr > 0.01 && p_lr < 0.05, "expected log-rank P in (0.01, 0.05), got {p_lr}");
    assert_p_cell_stars_empty(&tab_by_id(&res_surv, "curve-comparison").rows[0][2], "Survival log-rank 99%");
}

// ============================================================================
// FA-SEC06-10: Nested error variant on g_count > 2 & ignored-text disclosure
// ============================================================================

#[test]
fn fa_sec06_10_nested_error_variant_and_ignored_text_disclosure() {
    // 1. g_count > 2 with two_group_t_test == true returns InvalidParameter (not InsufficientSampleSize).
    let mut nt3 = NestedTable::new(3, 2, 3).unwrap();
    for g in 0..3 {
        for b in 0..2 {
            for r in 0..3 {
                nt3.groups[g].cells[r][b] = Cell::number(10.0 + (g as f64) * 3.0 + (b as f64) + 0.1 * (r as f64));
            }
        }
    }
    let err_3 = nested_one_way_anova(&nt3, true).unwrap_err();
    assert!(matches!(err_3, StatsError::InvalidParameter { name: "nested_t_test_groups", value } if value == 3.0), "got {err_3:?}");

    // 2. g_count < 2 with two_group_t_test == true returns InsufficientSampleSize.
    let mut nt1 = NestedTable::new(1, 2, 3).unwrap();
    for b in 0..2 {
        for r in 0..3 {
            nt1.groups[0].cells[r][b] = Cell::number(10.0 + (b as f64) + 0.1 * (r as f64));
        }
    }
    let err_1 = nested_one_way_anova(&nt1, true).unwrap_err();
    assert!(matches!(err_1, StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: 1 }), "got {err_1:?}");

    // 3. k == 2 with !two_group_t_test preserves documented k >= 3 multiplicity contract, while ND text triggers disclosure.
    let mut nt2 = NestedTable::new(2, 2, 4).unwrap();
    for g in 0..2 {
        for b in 0..2 {
            for r in 0..3 {
                nt2.groups[g].cells[r][b] = Cell::number(10.0 + (g as f64) * 4.0 + (b as f64) + 0.2 * (r as f64));
            }
            nt2.groups[g].cells[3][b] = Cell::number(10.5 + (g as f64) * 4.0 + (b as f64));
        }
    }
    nt2.groups[0].cells[3][1] = Cell::text("ND");
    let sheet_nt2 = DataSheet::new(SheetId::new("nt2-nd"), "Nt2Nd", TableArchetype::Nested(nt2)).unwrap();
    let nspec = NestedAnalysisSpec {
        two_group_t_test: false,
        df_method: MixedModelDfMethod::KenwardRoger,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..MultiplicitySpec::default() },
    };
    let res_nt2 = run_nested_analysis(&sheet_nt2, &spec_for("r-nt2", "nt2-nd", AnalysisKind::NestedComparison(nspec.clone())), &nspec).unwrap();
    let tab_ids: Vec<&str> = res_nt2.tabs.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(tab_ids, vec!["nested-anova", "variance-components"]);
    assert!(res_nt2.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(res_nt2.tabs[0].footnotes.iter().any(|f| f.contains("Non-numeric values ignored: 1")));
}

// ============================================================================
// FA-SEC06-11: Error contexts, Cox zero time, PB alpha & GLM/PB disclosures
// ============================================================================

#[test]
fn fa_sec06_11_error_contexts_cox_time_pb_alpha_and_listwise_text_disclosures() {
    // 1. Spearman & Kendall error contexts.
    let err_sp = spearman_correlation(&[1.0, 2.0], &[2.0, 3.0]).unwrap_err();
    assert!(matches!(err_sp, StatsError::InsufficientSampleSize { context: "Spearman correlation", required: 3, actual: 2 }), "got {err_sp:?}");
    let err_kd = kendall_tau_b(&[1.0, 2.0], &[2.0, 3.0]).unwrap_err();
    assert!(matches!(err_kd, StatsError::InsufficientSampleSize { context: "Kendall tau-b correlation", required: 3, actual: 2 }), "got {err_kd:?}");

    // 2. CoxSubject time == 0.0 is accepted, time < 0.0 is rejected.
    let cox_subjects = vec![
        CoxSubject { time: 0.0, event: false, x: vec![0.5] },
        CoxSubject { time: 2.0, event: true, x: vec![1.0] },
        CoxSubject { time: 4.0, event: true, x: vec![2.0] },
        CoxSubject { time: 6.0, event: false, x: vec![0.2] },
        CoxSubject { time: 8.0, event: true, x: vec![1.8] },
    ];
    assert!(cox_ph_regression(&cox_subjects, &[String::from("X")], CoxTieMethod::Breslow, ConfidenceLevel::DEFAULT_95).is_ok());
    let mut bad_cox = cox_subjects;
    bad_cox[0].time = -1.0;
    assert!(matches!(
        cox_ph_regression(&bad_cox, &[String::from("X")], CoxTieMethod::Breslow, ConfidenceLevel::DEFAULT_95),
        Err(StatsError::InvalidParameter { name: "cox_subject", .. })
    ));

    // 3. Passing-Bablok linearity_passed tracks conf.alpha().
    let xs_pb: Vec<f64> = (1..=20).map(f64::from).collect();
    let ys_pb: Vec<f64> = xs_pb.iter().map(|&x| 0.25 * x * x + 0.5 * x + 1.0).collect();
    assert!(!passing_bablok_regression(&xs_pb, &ys_pb, ConfidenceLevel::DEFAULT_95).unwrap().linearity_passed);
    assert!(passing_bablok_regression(&xs_pb, &ys_pb, ConfidenceLevel::PERCENT_99).unwrap().linearity_passed);

    // 4. GLM and Passing-Bablok emit listwise & non-numeric disclosures when cells are text/blank.
    let mut mv = MultipleVariablesTable::new(2, 7).unwrap();
    mv.variables[0].title = String::from("Y");
    mv.variables[1].title = String::from("X");
    for r in 0..5 {
        mv.variables[0].cells[r] = Cell::number(2.0 * (r + 1) as f64 + 0.1 * ((r % 2) as f64));
        mv.variables[1].cells[r] = Cell::number((r + 1) as f64);
    }
    mv.variables[0].cells[5] = Cell::text("ND");
    mv.variables[1].cells[5] = Cell::number(6.0);
    mv.variables[0].cells[6] = Cell::number(14.0);
    mv.variables[1].cells[6] = Cell::empty();

    let sheet_mv = DataSheet::new(SheetId::new("mv-nd"), "MvNd", TableArchetype::MultipleVariables(mv)).unwrap();
    let glm_spec =
        GlmSpec { family: GlmFamily::Linear, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: false };
    let glm_res =
        run_glm_analysis(&sheet_mv, &spec_for("r-glm-nd", "mv-nd", AnalysisKind::GeneralizedLinearModel(glm_spec.clone())), &glm_spec).unwrap();
    assert!(glm_res.checklist.iter().any(|c| c.assumption == "Complete-case row alignment" && c.status == ChecklistStatus::Info));
    assert!(glm_res.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(glm_res.tabs[0].footnotes.iter().any(|f| f.contains("omitted 2 incomplete or excluded row(s)")));
    assert!(glm_res.tabs[0].footnotes.iter().any(|f| f.contains("Non-numeric values ignored: 1")));

    let pb_spec = PassingBablokSpec { x_column: 1, y_column: 0 };
    let pb_res = run_passing_bablok_analysis(&sheet_mv, &spec_for("r-pb-nd", "mv-nd", AnalysisKind::PassingBablok(pb_spec)), &pb_spec).unwrap();
    assert!(pb_res.checklist.iter().any(|c| c.assumption == "Complete-case row alignment" && c.status == ChecklistStatus::Info));
    assert!(pb_res.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(pb_res.tabs[0].footnotes.iter().any(|f| f.contains("omitted 2 incomplete or excluded row(s)")));
    assert!(pb_res.tabs[0].footnotes.iter().any(|f| f.contains("Non-numeric values ignored: 1")));
}
