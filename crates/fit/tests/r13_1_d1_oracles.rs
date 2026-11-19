//! Domain-oracle (D1) verification suite for R13.1 workflow capabilities against
//! R 4.5.2 reference outputs and analytical statistical formulas:
//! - Multi-replicate (`RawReplicates`) 4PL standard curves with ROUT outlier exclusion
//!   (`Q = 1%`), tracked `SourceCellId` coordinates, and both mean-response and
//!   per-replicate delta-method inverse prediction intervals (Motulsky & Brown 2006;
//!   Motulsky & Christopoulos 2004; R Core Team `stats::nls`).
//! - Sparse-dose 4PL identifiability detection (`FitIdentifiability::Ambiguous`,
//!   `ParameterNote::Unstable`) and constrained global-sharing recovery (R `nls` pooled fit).
//! - Inclusive domain-restricted (`x_range_filter`) ordinary least-squares and orthogonal
//!   Model II Deming regression (`lambda = 1.0`) with leave-one-out jackknife SEs
//!   (R `stats::lm`; Deming 1943; Linnet 1990).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, DataSheet, FitMethod, LinearRegressionSpec, NonlinearFitSpec, ParameterRule,
    ParameterScope, ResultCell, ResultStatus, ResultTab, ResultTabKind, SheetId, SourceCellId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::{DataPoint, DemingOptions, HookContext, SimpleLinearOptions, deming_regression, execute_analysis, simple_linear_regression};

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tol,
        "{label}: expected {expected:.12} (+/- {tol:.2e}), got {actual:.12} (diff = {:.3e})",
        (actual - expected).abs()
    );
}

fn find_tab<'a>(sheet: &'a curvebook_doc::ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|tab| tab.id == id).unwrap_or_else(|| panic!("missing result tab `{id}` in `{}`", sheet.id))
}

fn row_index(tab: &ResultTab, row_label: &str) -> usize {
    tab.row_labels
        .iter()
        .position(|label| label == row_label)
        .unwrap_or_else(|| panic!("missing row `{row_label}` in tab `{}` (rows: {:?})", tab.id, tab.row_labels))
}

fn cell_number(cell: &ResultCell, label: &str) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected numeric cell for {label}, got {other:?}"),
    }
}

fn cell_ci(cell: &ResultCell, label: &str) -> (f64, f64) {
    match cell {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } => (*lo, *hi),
        other => panic!("expected finite CI cell for {label}, got {other:?}"),
    }
}

fn make_nonlinear_analysis(id: &str, source_id: &str, nl: NonlinearFitSpec) -> AnalysisSpec {
    AnalysisSpec::new(id, source_id, AnalysisKind::NonlinearRegression(nl)).unwrap()
}

#[test]
fn r13_1_4pl_replicate_rout_outlier_and_inverse_ci_r_oracle() {
    let sheet_id = "assay-std";
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 1, 10).unwrap();
    let xs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let noise_mat = [
        [-0.6, 0.2, 0.4],
        [0.5, -0.4, -0.1],
        [-0.3, 0.5, -0.2],
        [0.4, 120.0, -0.3], // row 3, col 0, subcol 1 is an extreme ROUT outlier
        [-0.5, 0.3, 0.2],
        [0.3, -0.5, 0.2],
        [-0.2, 0.4, -0.2],
        [0.4, -0.3, -0.1],
    ];
    for (row, &x) in xs.iter().enumerate() {
        xy.set_x_cell(row, Cell::number(x)).unwrap();
        let y0 = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.2 - x) * 1.1));
        for (subcol, &eps) in noise_mat[row].iter().enumerate() {
            xy.data_sets[0].cells[row][subcol] = Cell::number(y0 + eps);
        }
    }

    // Unknown row 8: 3 active replicates -> mean = 52.0 (m = 3).
    for (subcol, &y) in [48.0, 52.0, 56.0].iter().enumerate() {
        xy.data_sets[0].cells[8][subcol] = Cell::number(y);
    }
    // Unknown row 9: subcol 1 excluded -> mean = 38.0 (m = 2), 2 active replicates in Replicates tab.
    xy.data_sets[0].cells[9][0] = Cell::number(35.0);
    xy.data_sets[0].cells[9][1] = Cell::excluded_number(999.0);
    xy.data_sets[0].cells[9][2] = Cell::number(41.0);

    let sheet = DataSheet::new(sheet_id, "4PL Replicate Assay", TableArchetype::Xy(xy)).unwrap();
    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl.interpolate_unknowns = true;
    nl.profile_likelihood_ci = false;

    let result = execute_analysis(&make_nonlinear_analysis("fit-4pl-rout", sheet_id, nl), &sheet, &HookContext::default()).unwrap();
    assert_eq!(result.status, ResultStatus::UpToDate);
    assert_eq!(
        result.eliminated_outliers,
        vec![SourceCellId { sheet_id: SheetId::new(sheet_id), row: 3, col: 0, subcol: 1 }],
        "ROUT must record exact SourceCellId of the eliminated outlier"
    );

    // R 4.5.2 stats::nls oracle on the 23 clean observations (df = 19, rss = 2.867595511063387).
    let param_tab = find_tab(&result, "nonlinear_parameters");
    let param_oracles = [
        ("Bottom", 9.994858702847651, 0.1657578090127751),
        ("Top", 100.00240465552876, 0.19573189791489126),
        ("LogEC50", -6.200340516786204, 0.004156538193438742),
        ("HillSlope", 1.099264683470414, 0.009996435533883631),
    ];
    for (name, est_oracle, se_oracle) in param_oracles {
        let r = row_index(param_tab, name);
        assert_close(cell_number(&param_tab.rows[r][0], name), est_oracle, 1e-6, &format!("{name} estimate"));
        assert_close(cell_number(&param_tab.rows[r][1], name), se_oracle, 1e-5, &format!("{name} SE"));
    }

    let gof_tab = find_tab(&result, "nonlinear_gof");
    assert_close(cell_number(&gof_tab.rows[row_index(gof_tab, "Degrees of Freedom")][0], "df"), 19.0, 1e-12, "df");
    assert_close(cell_number(&gof_tab.rows[row_index(gof_tab, "Sum of Squares (RSS)")][0], "rss"), 2.867595511063387, 1e-6, "RSS");
    assert_close(cell_number(&gof_tab.rows[row_index(gof_tab, "Sy.x")][0], "sy.x"), 0.3884920585155531, 1e-6, "Sy.x");
    assert_close(cell_number(&gof_tab.rows[row_index(gof_tab, "ROUT outliers excluded")][0], "outliers"), 1.0, 1e-12, "outlier count");

    // Verify dual Interpolated Mean X and Interpolated Replicates tabs against R 4.5.2 delta-method oracle.
    let mean_tab = find_tab(&result, "nonlinear_interpolated_mean_x");
    assert_eq!(mean_tab.title, "Interpolated Mean X");
    assert_eq!(mean_tab.kind, ResultTabKind::InterpolatedUnknowns);
    assert_eq!(mean_tab.row_labels, vec!["A (row 9, mean of 3)", "A (row 10, mean of 2)"]);

    let rep_tab = find_tab(&result, "nonlinear_interpolated_replicates");
    assert_eq!(rep_tab.title, "Interpolated Replicates");
    assert_eq!(rep_tab.kind, ResultTabKind::InterpolatedUnknowns);
    assert_eq!(
        rep_tab.row_labels,
        vec!["A (row 9, replicate 1)", "A (row 9, replicate 2)", "A (row 9, replicate 3)", "A (row 10, replicate 1)", "A (row 10, replicate 3)",]
    );

    let mean_oracles = [
        // (y_mean, xhat, ci_lo, ci_hi, pi_lo, pi_hi)
        (52.0, -6.253067166353842, -6.259606340762511, -6.246558635172907, -6.26362976079739, -6.242546775035985),
        (38.0, -6.514342021766931, -6.522568014487132, -6.506200636138463, -6.528791564851771, -6.500109651616661),
    ];
    for (idx, (y_in, xhat, ci_lo, ci_hi, pi_lo, pi_hi)) in mean_oracles.into_iter().enumerate() {
        let lbl = &mean_tab.row_labels[idx];
        assert_close(cell_number(&mean_tab.rows[idx][0], lbl), y_in, 1e-12, &format!("{lbl} Y"));
        assert_close(cell_number(&mean_tab.rows[idx][1], lbl), xhat, 1e-5, &format!("{lbl} Xhat"));
        let (act_ci_lo, act_ci_hi) = cell_ci(&mean_tab.rows[idx][2], lbl);
        let (act_pi_lo, act_pi_hi) = cell_ci(&mean_tab.rows[idx][3], lbl);
        assert_close(act_ci_lo, ci_lo, 5e-5, &format!("{lbl} CI lower"));
        assert_close(act_ci_hi, ci_hi, 5e-5, &format!("{lbl} CI upper"));
        assert_close(act_pi_lo, pi_lo, 5e-5, &format!("{lbl} PI lower"));
        assert_close(act_pi_hi, pi_hi, 5e-5, &format!("{lbl} PI upper"));
    }

    let rep_oracles = [
        (48.0, -6.324224306514777, -6.331055975277963, -6.317442590811065, -6.340430209045572, -6.308152623875469),
        (52.0, -6.253067166353842, -6.259606340751737, -6.246558635177032, -6.268856684676575, -6.237342978599185),
        (56.0, -6.182756072734561, -6.189160208141549, -6.176362469371354, -6.198409407519609, -6.167101780228206),
        (35.0, -6.577774738234313, -6.586607513935807, -6.569030262722512, -6.597843704024396, -6.558150184096406),
        (41.0, -6.454543167982867, -6.462252159023127, -6.446911428003172, -6.472252020617931, -6.437108306983839),
    ];
    for (idx, (y_in, xhat, ci_lo, ci_hi, pi_lo, pi_hi)) in rep_oracles.into_iter().enumerate() {
        let lbl = &rep_tab.row_labels[idx];
        assert_close(cell_number(&rep_tab.rows[idx][0], lbl), y_in, 1e-12, &format!("{lbl} Y"));
        assert_close(cell_number(&rep_tab.rows[idx][1], lbl), xhat, 1e-5, &format!("{lbl} Xhat"));
        let (act_ci_lo, act_ci_hi) = cell_ci(&rep_tab.rows[idx][2], lbl);
        let (act_pi_lo, act_pi_hi) = cell_ci(&rep_tab.rows[idx][3], lbl);
        assert_close(act_ci_lo, ci_lo, 5e-5, &format!("{lbl} CI lower"));
        assert_close(act_ci_hi, ci_hi, 5e-5, &format!("{lbl} CI upper"));
        assert_close(act_pi_lo, pi_lo, 5e-5, &format!("{lbl} PI lower"));
        assert_close(act_pi_hi, pi_hi, 5e-5, &format!("{lbl} PI upper"));
    }

    // At identical Y=52, averaging m=3 replicates strictly narrows the inverse prediction interval vs m=1.
    let (mean_pi_lo, mean_pi_hi) = cell_ci(&mean_tab.rows[0][3], "mean Y=52 PI");
    let (rep_pi_lo, rep_pi_hi) = cell_ci(&rep_tab.rows[1][3], "rep2 Y=52 PI");
    assert!((mean_pi_hi - mean_pi_lo) < (rep_pi_hi - rep_pi_lo), "replicate averaging (m=3) must narrow inverse PI relative to m=1");
}

#[test]
fn r13_1_sparse_4pl_identifiability_recovery_r_oracle() {
    let sheet_id = "sparse-4pl";
    let xa = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let ya = [0.6, 2.8, 22.1, 74.5, 96.2, 99.7];
    let xb = [-9.0, -8.0, -7.2];
    let yb = [0.4, 1.3, 6.2];
    // Preserve distinct X support [-9, -8, -7.2] for Dataset B alongside Dataset A in one XyTable:
    // rows 0..2 share -9, -8; rows 2..6 carry Dataset A only; row 6 carries Dataset B at -7.2.
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 7).unwrap();
    for i in 0..6 {
        xy.set_x_cell(i, Cell::number(xa[i])).unwrap();
        xy.data_sets[0].cells[i][0] = Cell::number(ya[i]);
    }
    xy.set_x_cell(6, Cell::number(xb[2])).unwrap();
    xy.data_sets[1].cells[0][0] = Cell::number(yb[0]);
    xy.data_sets[1].cells[1][0] = Cell::number(yb[1]);
    xy.data_sets[1].cells[6][0] = Cell::number(yb[2]);
    let sheet = DataSheet::new(sheet_id, "Sparse-Dose 4PL", TableArchetype::Xy(xy)).unwrap();

    // 1) Independent local 4PL fails on Dataset B (< 4 observations for 4 free params).
    let mut nl_4pl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_4pl.identify_unstable_parameters = true;
    nl_4pl.profile_likelihood_ci = false;
    let indep_4pl_err = execute_analysis(&make_nonlinear_analysis("fit-sparse-4pl", sheet_id, nl_4pl), &sheet, &HookContext::default()).unwrap_err();
    assert_eq!(indep_4pl_err, curvebook_fit::FitError::InsufficientData { required: 4, actual: 3 });

    // 2) Fixing only Bottom = 0 leaves 3 local params for 3 Dataset B points (df = 0) -> Ambiguous + Unstable.
    let mut nl_indep = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_indep.identify_unstable_parameters = true;
    nl_indep.profile_likelihood_ci = false;
    nl_indep.parameter_rules = vec![ParameterRule {
        name: "Bottom".to_owned(),
        scope: ParameterScope::Constant(0.0),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }];
    let indep_res = execute_analysis(&make_nonlinear_analysis("fit-sparse-indep", sheet_id, nl_indep), &sheet, &HookContext::default()).unwrap();
    assert!(
        indep_res.checklist.iter().any(|item| item.assumption == "Parameter identifiability (Ambiguous)" && item.status == ChecklistStatus::Violated),
        "expected Parameter identifiability (Ambiguous) Violated checklist item, got {:?}",
        indep_res.checklist
    );
    let indep_params = find_tab(&indep_res, "nonlinear_parameters");
    assert!(
        ["Top [DS2] (Unstable)", "LogEC50 [DS2] (Unstable)", "HillSlope [DS2] (Unstable)"]
            .iter()
            .all(|expected| indep_params.row_labels.iter().any(|lbl| lbl == expected)),
        "expected all Dataset B (DS2) parameters tagged (Unstable), got {:?}",
        indep_params.row_labels
    );
    assert!(
        indep_params.row_labels.iter().filter(|lbl| lbl.contains("[DS1]")).all(|lbl| !lbl.ends_with("(Unstable)")),
        "expected Dataset A (DS1) parameters to remain stable, got {:?}",
        indep_params.row_labels
    );

    // 3) WF-52 Recovery: Bottom = Constant(0), Top & HillSlope = SharedGlobal, LogEC50 = Local.
    let mut nl_rec = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_rec.identify_unstable_parameters = true;
    nl_rec.profile_likelihood_ci = false;
    nl_rec.parameter_rules = vec![
        ParameterRule {
            name: "Bottom".to_owned(),
            scope: ParameterScope::Constant(0.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule { name: "Top".to_owned(), scope: ParameterScope::SharedGlobal, initial_override: None, lower_bound: None, upper_bound: None },
        ParameterRule {
            name: "HillSlope".to_owned(),
            scope: ParameterScope::SharedGlobal,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule { name: "LogEC50".to_owned(), scope: ParameterScope::Local, initial_override: None, lower_bound: None, upper_bound: None },
    ];
    let rec_res = execute_analysis(&make_nonlinear_analysis("fit-sparse-rec", sheet_id, nl_rec), &sheet, &HookContext::default()).unwrap();

    assert!(
        rec_res.checklist.iter().all(|item| item.assumption != "Parameter identifiability (Ambiguous)"),
        "pooled global-sharing fit must recover clean identifiability: {:?}",
        rec_res.checklist
    );
    assert!(
        rec_res.checklist.iter().any(|item| item.assumption == "Parameter identifiability" && item.status == ChecklistStatus::Pass),
        "expected Parameter identifiability Pass checklist item: {:?}",
        rec_res.checklist
    );
    let extrap_warn = rec_res
        .checklist
        .iter()
        .find(|item| item.assumption == "LogEC50 outside observed X range (B)" && item.status == ChecklistStatus::Warn)
        .expect("expected LogEC50 outside observed X range (B) Warn checklist item");
    assert!(extrap_warn.remediation.is_none());

    // R 4.5.2 pooled stats::nls oracle (df = 5, rss = 0.4929092239189585).
    let rec_params = find_tab(&rec_res, "nonlinear_parameters");
    let rec_gof = find_tab(&rec_res, "nonlinear_gof");
    assert_close(cell_number(&rec_gof.rows[row_index(rec_gof, "Degrees of Freedom")][0], "df"), 5.0, 1e-12, "pooled df");
    assert_close(cell_number(&rec_gof.rows[row_index(rec_gof, "Sum of Squares (RSS)")][0], "rss"), 0.4929092239189585, 1e-6, "pooled RSS");

    let pooled_oracles = [
        ("Top", 99.77111824194645, 0.26241943098077725),
        ("HillSlope", 1.009530731975136, 0.010931735463543379),
        ("LogEC50 [DS1]", -6.462008648887402, 0.006280679075401399),
        ("LogEC50 [DS2]", -6.036237822355189, 0.026404474352218665),
    ];
    for (row_lbl, est_oracle, se_oracle) in pooled_oracles {
        let r = row_index(rec_params, row_lbl);
        assert_close(cell_number(&rec_params.rows[r][0], row_lbl), est_oracle, 1e-5, &format!("{row_lbl} estimate"));
        assert_close(cell_number(&rec_params.rows[r][1], row_lbl), se_oracle, 5e-5, &format!("{row_lbl} SE"));
    }
}

#[test]
fn r13_1_domain_restricted_ols_and_deming_r_oracle() {
    let xr = [2.0, 6.0, 10.0, 11.5, 13.0, 14.5, 16.0, 20.0, 24.0];
    let yr = [85.0, 42.0, 23.2, 26.5, 29.1, 32.8, 35.4, -12.0, -45.0];
    let points: Vec<DataPoint> = xr.iter().zip(yr.iter()).map(|(&x, &y)| DataPoint::xy(x, y)).collect();

    // Direct OLS domain-restricted oracle matching R 4.5.2 stats::lm on x in [10, 16].
    let ols_opts = SimpleLinearOptions { x_range_filter: Some((Some(10.0), Some(16.0))), ..SimpleLinearOptions::default() };
    let ols = simple_linear_regression(&points, &ols_opts).unwrap();
    assert_eq!(ols.n, 5);
    assert_eq!(ols.n_excluded_by_range, 4);
    assert_eq!(ols.df_residual, 3);
    assert_close(ols.slope, 2.046666666666666, 1e-12, "OLS slope");
    assert_close(ols.se_slope, 0.06097965659560974, 1e-12, "OLS SE(slope)");
    assert_close(ols.ci_slope.0, 1.852602183836498, 1e-10, "OLS CI(slope) lo");
    assert_close(ols.ci_slope.1, 2.240731149496833, 1e-10, "OLS CI(slope) hi");
    assert_close(ols.intercept, 2.793333333333345, 1e-11, "OLS intercept");
    assert_close(ols.se_intercept, 0.803220370112062, 1e-11, "OLS SE(intercept)");
    assert_close(ols.ci_intercept.0, 0.2371276341416011, 1e-9, "OLS CI(intercept) lo");
    assert_close(ols.ci_intercept.1, 5.34953903252509, 1e-9, "OLS CI(intercept) hi");
    assert_close(ols.r_squared, 0.9973439153439153, 1e-12, "OLS R2");
    assert_close(ols.sy_x, 0.2892519086655542, 1e-12, "OLS Sy.x");
    assert_close(ols.f_stat, 1126.482071713155, 1e-9, "OLS F");
    assert_close(ols.p_value, 5.814309492494304e-05, 1e-10, "OLS p-value");
    assert_close(ols.bands.first().unwrap().x, 10.0, 1e-12, "OLS band x_min");
    assert_close(ols.bands.last().unwrap().x, 16.0, 1e-12, "OLS band x_max");

    // Verify ResultSheet OLS and orthogonal Deming (lambda = 1.0) via execute_analysis with x_range_filter.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, xr.len()).unwrap();
    for (row, (&x, &y)) in xr.iter().zip(yr.iter()).enumerate() {
        xy.set_x_cell(row, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[row][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("restricted", "Domain Restricted", TableArchetype::Xy(xy)).unwrap();

    let ols_spec = AnalysisSpec::new(
        "fit-ols-range",
        "restricted",
        AnalysisKind::LinearRegression(LinearRegressionSpec {
            deming_model_ii: false,
            x_range_filter: Some((Some(10.0), Some(16.0))),
            ..LinearRegressionSpec::default()
        }),
    )
    .unwrap();
    let ols_sheet = execute_analysis(&ols_spec, &sheet, &HookContext::default()).unwrap();
    assert_close(ols_sheet.lookup_scalar("Slope", Some(0)).unwrap(), 2.046666666666666, 1e-12, "ResultSheet OLS slope");
    assert_close(ols_sheet.lookup_scalar("YIntercept", Some(0)).unwrap(), 2.793333333333345, 1e-11, "ResultSheet OLS intercept");
    assert_close(ols_sheet.fitted_curves[0].points.first().unwrap().x, 10.0, 1e-12, "OLS overlay min X");
    assert_close(ols_sheet.fitted_curves[0].points.last().unwrap().x, 16.0, 1e-12, "OLS overlay max X");

    let in_range: Vec<DataPoint> = points.iter().filter(|p| (10.0..=16.0).contains(&p.x0())).cloned().collect();
    let dem = deming_regression(&in_range, &DemingOptions::default()).unwrap();
    assert_eq!(dem.n, 5);
    assert_eq!(dem.df, 3);
    assert_close(dem.slope, 2.05106863928226, 1e-12, "Deming slope");
    assert_close(dem.se_slope, 0.05691497183347908, 1e-12, "Deming jackknife SE(slope)");
    assert_close(dem.intercept, 2.73610768933062, 1e-11, "Deming intercept");
    assert_close(dem.se_intercept, 0.6722665887968624, 1e-11, "Deming jackknife SE(intercept)");

    let dem_spec = AnalysisSpec::new(
        "fit-deming-range",
        "restricted",
        AnalysisKind::LinearRegression(LinearRegressionSpec {
            deming_model_ii: true,
            x_range_filter: Some((Some(10.0), Some(16.0))),
            ..LinearRegressionSpec::default()
        }),
    )
    .unwrap();
    let dem_sheet = execute_analysis(&dem_spec, &sheet, &HookContext::default()).unwrap();
    assert_close(dem_sheet.lookup_scalar("Slope", Some(0)).unwrap(), 2.05106863928226, 1e-12, "ResultSheet Deming slope");
    assert_close(dem_sheet.lookup_scalar("YIntercept", Some(0)).unwrap(), 2.73610768933062, 1e-11, "ResultSheet Deming intercept");
    assert_close(dem_sheet.fitted_curves[0].points.first().unwrap().x, 10.0, 1e-12, "Deming overlay min X");
    assert_close(dem_sheet.fitted_curves[0].points.last().unwrap().x, 16.0, 1e-12, "Deming overlay max X");
}
