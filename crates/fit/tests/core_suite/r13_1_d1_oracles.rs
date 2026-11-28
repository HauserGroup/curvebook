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
    Cell, ChecklistStatus, DataSheet, FitMethod, LinearRegressionSpec, ResultStatus, ResultTab, ResultTabKind, SheetId, SourceCellId, SubcolumnMode,
    TableArchetype, XyTable,
};
use curvebook_fit::{DataPoint, DemingOptions, SimpleLinearOptions, deming_regression, simple_linear_regression};
use curvebook_testkit::assert_close;

use crate::common::{
    assert_ci_cell, cell_num, exec_lin, exec_nl, exec_nl_res, find_tab, nl_fast, rule_const, rule_local, rule_shared, tab_row, xy_pts,
    xy_sheet_single,
};

fn assert_interp_rows(tab: &ResultTab, oracles: &[(f64, f64, f64, f64, f64, f64)]) {
    for (idx, &(y_in, xhat, ci_lo, ci_hi, pi_lo, pi_hi)) in oracles.iter().enumerate() {
        let lbl = &tab.row_labels[idx];
        assert_close(cell_num(&tab.rows[idx][0]), y_in, 1e-12, &format!("{lbl} Y"));
        assert_close(cell_num(&tab.rows[idx][1]), xhat, 1e-5, &format!("{lbl} Xhat"));
        assert_ci_cell(&tab.rows[idx][2], (ci_lo, ci_hi), 5e-5, 0.0, &format!("{lbl} CI"));
        assert_ci_cell(&tab.rows[idx][3], (pi_lo, pi_hi), 5e-5, 0.0, &format!("{lbl} PI"));
    }
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
    for (subcol, y) in [48.0, 52.0, 56.0].into_iter().enumerate() {
        xy.data_sets[0].cells[8][subcol] = Cell::number(y);
    }
    // Unknown row 9: subcol 1 excluded -> mean = 38.0 (m = 2), 2 active replicates in Replicates tab.
    xy.data_sets[0].cells[9][0] = Cell::number(35.0);
    xy.data_sets[0].cells[9][1] = Cell::excluded_number(999.0);
    xy.data_sets[0].cells[9][2] = Cell::number(41.0);

    let sheet = DataSheet::new(sheet_id, "4PL Replicate Assay", TableArchetype::Xy(xy)).unwrap();
    let mut nl = nl_fast("log_agonist_vs_response_4pl");
    nl.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl.interpolate_unknowns = true;

    let result = exec_nl(&sheet, nl);
    assert_eq!(result.status, ResultStatus::UpToDate);
    assert_eq!(
        result.eliminated_outliers,
        vec![SourceCellId { sheet_id: SheetId::new(sheet_id), row: 3, col: 0, subcol: 1 }],
        "ROUT must record exact SourceCellId of the eliminated outlier"
    );

    // R 4.5.2 stats::nls oracle on the 23 clean observations (df = 19, rss = 2.867595511063387).
    for (name, est, se) in [
        ("Bottom", 9.994858702847651, 0.1657578090127751),
        ("Top", 100.00240465552876, 0.19573189791489126),
        ("LogEC50", -6.200340516786204, 0.004156538193438742),
        ("HillSlope", 1.099264683470414, 0.009996435533883631),
    ] {
        let row = tab_row(&result, "nonlinear_parameters", name);
        assert_close(cell_num(&row[0]), est, 1e-6, &format!("{name} estimate"));
        assert_close(cell_num(&row[1]), se, 1e-5, &format!("{name} SE"));
    }
    for (label, expected, tol) in [
        ("Degrees of Freedom", 19.0, 1e-12),
        ("Sum of Squares (RSS)", 2.867595511063387, 1e-6),
        ("Sy.x", 0.3884920585155531, 1e-6),
        ("ROUT outliers excluded", 1.0, 1e-12),
    ] {
        assert_close(cell_num(&tab_row(&result, "nonlinear_gof", label)[0]), expected, tol, label);
    }

    // Verify dual Interpolated Mean X and Interpolated Replicates tabs against R 4.5.2 delta-method oracle.
    let mean_tab = find_tab(&result, "nonlinear_interpolated_mean_x");
    assert_eq!((mean_tab.title.as_str(), mean_tab.kind), ("Interpolated Mean X", ResultTabKind::InterpolatedUnknowns));
    assert_eq!(mean_tab.row_labels, vec!["A (row 9, mean of 3)", "A (row 10, mean of 2)"]);
    assert_interp_rows(
        mean_tab,
        &[
            (52.0, -6.253067166353842, -6.259606340762511, -6.246558635172907, -6.26362976079739, -6.242546775035985),
            (38.0, -6.514342021766931, -6.522568014487132, -6.506200636138463, -6.528791564851771, -6.500109651616661),
        ],
    );

    let rep_tab = find_tab(&result, "nonlinear_interpolated_replicates");
    assert_eq!((rep_tab.title.as_str(), rep_tab.kind), ("Interpolated Replicates", ResultTabKind::InterpolatedUnknowns));
    assert_eq!(
        rep_tab.row_labels,
        vec!["A (row 9, replicate 1)", "A (row 9, replicate 2)", "A (row 9, replicate 3)", "A (row 10, replicate 1)", "A (row 10, replicate 3)"]
    );
    assert_interp_rows(
        rep_tab,
        &[
            (48.0, -6.324224306514777, -6.331055975277963, -6.317442590811065, -6.340430209045572, -6.308152623875469),
            (52.0, -6.253067166353842, -6.259606340751737, -6.246558635177032, -6.268856684676575, -6.237342978599185),
            (56.0, -6.182756072734561, -6.189160208141549, -6.176362469371354, -6.198409407519609, -6.167101780228206),
            (35.0, -6.577774738234313, -6.586607513935807, -6.569030262722512, -6.597843704024396, -6.558150184096406),
            (41.0, -6.454543167982867, -6.462252159023127, -6.446911428003172, -6.472252020617931, -6.437108306983839),
        ],
    );

    // At identical Y=52, averaging m=3 replicates strictly narrows the inverse prediction interval vs m=1.
    let (mean_lo, mean_hi) = match mean_tab.rows[0][3] {
        curvebook_doc::ResultCell::ConfidenceInterval { lower: curvebook_doc::CiBound::Finite(l), upper: curvebook_doc::CiBound::Finite(h) } => {
            (l, h)
        }
        ref other => panic!("expected finite CI cell, got {other:?}"),
    };
    let (rep_lo, rep_hi) = match rep_tab.rows[1][3] {
        curvebook_doc::ResultCell::ConfidenceInterval { lower: curvebook_doc::CiBound::Finite(l), upper: curvebook_doc::CiBound::Finite(h) } => {
            (l, h)
        }
        ref other => panic!("expected finite CI cell, got {other:?}"),
    };
    assert!((mean_hi - mean_lo) < (rep_hi - rep_lo), "replicate averaging (m=3) must narrow inverse PI relative to m=1");
}

#[test]
fn r13_1_4pl_inverse_interpolation_uniroot_edge_cases_r_oracle() {
    // D1 Provenance: R 4.5.2 (2025-10-31) `stats::nls` + `stats::uniroot` delta-method band crossing
    // (`crates/fit/tests/reference/d1_extended_fit_oracles.R`, 2026-11-27).
    // Covers: near-top-plateau asymmetric CI / half-open PI & CI, near-bottom-plateau open lower CI/PI,
    // out-of-X-range extrapolation notes, AboveTopPlateau / BelowBottomPlateau NaN + open bounds,
    // and decreasing 4PL inhibitor (`log_inhibitor_vs_response_4pl`).
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 1, 15).unwrap();
    let xs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let noise_mat = [
        [-0.6, 0.2, 0.4],
        [0.5, -0.4, -0.1],
        [-0.3, 0.5, -0.2],
        [0.4, 120.0, -0.3], // eliminated by ROUT Q=1%
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
    // 7 single-replicate edge-case unknown rows (rows 8..15):
    let edge_ys = [99.20, 99.65, 99.90, 10.30, 10.05, 105.0, 5.0];
    for (k, &yv) in edge_ys.iter().enumerate() {
        xy.data_sets[0].cells[8 + k][0] = Cell::number(yv);
    }
    let sheet = DataSheet::new("assay-edge", "4PL Edge Assay", TableArchetype::Xy(xy)).unwrap();
    let mut nl = nl_fast("log_agonist_vs_response_4pl");
    nl.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl.interpolate_unknowns = true;
    let res = exec_nl(&sheet, nl);
    assert_eq!(res.status, ResultStatus::UpToDate);

    let tab = find_tab(&res, "nonlinear_interpolated_replicates");
    assert_eq!(tab.rows.len(), edge_ys.len());

    let assert_bound = |cell: &curvebook_doc::ResultCell, exp_lo: Option<f64>, exp_hi: Option<f64>, tol: f64, lbl: &str| match cell {
        curvebook_doc::ResultCell::ConfidenceInterval { lower, upper } => {
            match (lower, exp_lo) {
                (curvebook_doc::CiBound::Finite(a), Some(e)) => assert_close(*a, e, tol, &format!("{lbl} lo")),
                (curvebook_doc::CiBound::Open, None) => {}
                other => panic!("{lbl} lo mismatch: {other:?}"),
            }
            match (upper, exp_hi) {
                (curvebook_doc::CiBound::Finite(a), Some(e)) => assert_close(*a, e, tol, &format!("{lbl} hi")),
                (curvebook_doc::CiBound::Open, None) => {}
                other => panic!("{lbl} hi mismatch: {other:?}"),
            }
        }
        other => panic!("{lbl}: expected ConfidenceInterval cell, got {other:?}"),
    };
    let has_footnote_for = |row_1based: usize, needle: &str| -> bool {
        let prefix = format!("row {row_1based}, replicate 1");
        tab.footnotes.iter().any(|f| f.contains(&prefix) && f.contains(needle))
    };

    // Row 0: Y = 99.20 -> in-range, strongly asymmetric finite CI, half-open upper PI.
    assert_close(cell_num(&tab.rows[0][1]), -4.339_099_565_047, 5e-5, "y=99.20 x_hat");
    assert_bound(&tab.rows[0][2], Some(-4.482_046_856_425), Some(-4.084_167_824_457), 5e-4, "y=99.20 CI");
    assert_bound(&tab.rows[0][3], Some(-4.634_604_462_587), None, 5e-4, "y=99.20 PI");

    // Row 1: Y = 99.65 -> in-range near top plateau, half-open upper CI & PI.
    assert_close(cell_num(&tab.rows[1][1]), -4.012_029_004_208, 1e-4, "y=99.65 x_hat");
    assert_bound(&tab.rows[1][2], Some(-4.294_086_736_996), None, 5e-4, "y=99.65 CI");
    assert_bound(&tab.rows[1][3], Some(-4.511_221_542_098), None, 5e-4, "y=99.65 PI");

    // Row 2 (table row 11): Y = 99.90 -> extrapolated ABOVE measured X range [-9, -4], half-open upper CI & PI.
    assert_close(cell_num(&tab.rows[2][1]), -3.522_673_462_472, 5e-4, "y=99.90 x_hat");
    assert_bound(&tab.rows[2][2], Some(-4.133_825_384_196), None, 5e-4, "y=99.90 CI");
    assert_bound(&tab.rows[2][3], Some(-4.422_293_522_654), None, 5e-4, "y=99.90 PI");
    assert!(has_footnote_for(11, "Extrapolated above"), "expected above-range footnote, got {:?}", tab.footnotes);

    // Row 3 (table row 12): Y = 10.30 -> near bottom plateau, half-open lower CI & PI.
    assert_close(cell_num(&tab.rows[3][1]), -8.445_753_413_592, 1e-4, "y=10.30 x_hat");
    assert_bound(&tab.rows[3][2], None, Some(-8.164_160_662_917), 5e-4, "y=10.30 CI");
    assert_bound(&tab.rows[3][3], None, Some(-7.910_580_359_981), 5e-4, "y=10.30 PI");

    // Row 4 (table row 13): Y = 10.05 -> extrapolated BELOW measured X range [-9, -4], half-open lower CI & PI.
    assert_close(cell_num(&tab.rows[4][1]), -9.122_781_169_738, 5e-4, "y=10.05 x_hat");
    assert_bound(&tab.rows[4][2], None, Some(-8.358_646_195_336), 5e-4, "y=10.05 CI");
    assert_bound(&tab.rows[4][3], None, Some(-8.005_515_653_211), 5e-4, "y=10.05 PI");
    assert!(has_footnote_for(13, "Extrapolated below"), "expected below-range footnote, got {:?}", tab.footnotes);

    // Row 5 & 6 (table rows 14 & 15): AboveTopPlateau (105.0) and BelowBottomPlateau (5.0) -> Empty/NaN estimate, Unavailable CI/PI, explicit footnote tag.
    for (row_idx, row_1based, expected_tag) in [(5, 14, "AboveTopPlateau"), (6, 15, "BelowBottomPlateau")] {
        assert!(
            matches!(tab.rows[row_idx][1], curvebook_doc::ResultCell::Empty)
                || matches!(tab.rows[row_idx][1], curvebook_doc::ResultCell::Number { value, .. } if value.is_nan()),
            "plateau violation row {row_idx} must not emit finite X_hat: {:?}",
            tab.rows[row_idx][1]
        );
        for col_idx in [2, 3] {
            assert!(
                matches!(
                    tab.rows[row_idx][col_idx],
                    curvebook_doc::ResultCell::ConfidenceInterval {
                        lower: curvebook_doc::CiBound::Unavailable,
                        upper: curvebook_doc::CiBound::Unavailable
                    }
                ),
                "{expected_tag} col {col_idx} expected Unavailable bounds, got {:?}",
                tab.rows[row_idx][col_idx]
            );
        }
        assert!(has_footnote_for(row_1based, expected_tag), "expected {expected_tag} footnote, got {:?}", tab.footnotes);
    }

    // Decreasing 4PL inhibitor (`log_inhibitor_vs_response_4pl`) interior + near-top open-lower CI/PI.
    let x_inh = [-10.0, -9.5, -9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let e_inh = [0.4, -0.5, 0.3, -0.4, 0.5, -0.6, 0.4, -0.5, 0.6, -0.3, 0.4, -0.3];
    let mut xy_inh = XyTable::new(SubcolumnMode::Single, 1, 14).unwrap();
    for (r, (&x, &e)) in x_inh.iter().zip(e_inh.iter()).enumerate() {
        xy_inh.set_x_cell(r, Cell::number(x)).unwrap();
        let y = 12.0 + 86.0 / (1.0 + 10.0_f64.powf((x - (-7.1)) * 1.25)) + e;
        xy_inh.data_sets[0].cells[r][0] = Cell::number(y);
    }
    xy_inh.data_sets[0].cells[12][0] = Cell::number(55.0);
    xy_inh.data_sets[0].cells[13][0] = Cell::number(97.5);
    let sheet_inh = DataSheet::new("inh-edge", "4PL Inhibitor Edge", TableArchetype::Xy(xy_inh)).unwrap();
    let mut nl_inh = nl_fast("log_inhibitor_vs_response_4pl");
    nl_inh.interpolate_unknowns = true;
    let res_inh = exec_nl(&sheet_inh, nl_inh);
    let itab = find_tab(&res_inh, "nonlinear_interpolated");
    assert_eq!(itab.rows.len(), 2);
    assert_close(cell_num(&itab.rows[0][1]), -7.101_152_635_291, 5e-5, "inhibitor y=55 x_hat");
    assert_ci_cell(&itab.rows[0][2], (-7.116_464_093_482, -7.085_836_329_347), 1e-4, 0.0, "inhibitor y=55 CI");
    assert_ci_cell(&itab.rows[0][3], (-7.126_587_381_108, -7.075_712_490_529), 1e-4, 0.0, "inhibitor y=55 PI");
    assert_close(cell_num(&itab.rows[1][1]), -8.872_878_166_856, 2e-4, "inhibitor y=97.5 x_hat");
    match (&itab.rows[1][2], &itab.rows[1][3]) {
        (
            curvebook_doc::ResultCell::ConfidenceInterval { lower: ci_l, upper: ci_u },
            curvebook_doc::ResultCell::ConfidenceInterval { lower: pi_l, upper: pi_u },
        ) => {
            assert_eq!(*ci_l, curvebook_doc::CiBound::Open, "inhibitor y=97.5 CI lo must be Open");
            assert_eq!(*pi_l, curvebook_doc::CiBound::Open, "inhibitor y=97.5 PI lo must be Open");
            let curvebook_doc::CiBound::Finite(ch) = *ci_u else { panic!("expected finite CI hi") };
            let curvebook_doc::CiBound::Finite(ph) = *pi_u else { panic!("expected finite PI hi") };
            assert_close(ch, -8.619_654_796_886, 5e-4, "inhibitor y=97.5 CI hi");
            assert_close(ph, -8.424_767_902_800, 5e-4, "inhibitor y=97.5 PI hi");
        }
        other => panic!("expected CI/PI cells for inhibitor y=97.5, got {other:?}"),
    }
}

#[test]
fn r13_1_sparse_4pl_identifiability_recovery_r_oracle() {
    let xa = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let ya = [0.6, 2.8, 22.1, 74.5, 96.2, 99.7];
    let yb = [0.4, 1.3, 6.2];
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 7).unwrap();
    for i in 0..6 {
        xy.set_x_cell(i, Cell::number(xa[i])).unwrap();
        xy.data_sets[0].cells[i][0] = Cell::number(ya[i]);
    }
    xy.set_x_cell(6, Cell::number(-7.2)).unwrap();
    for (row, y) in [(0, yb[0]), (1, yb[1]), (6, yb[2])] {
        xy.data_sets[1].cells[row][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("sparse-4pl", "Sparse-Dose 4PL", TableArchetype::Xy(xy)).unwrap();

    // 1) Independent local 4PL fails on Dataset B (< 4 observations for 4 free params).
    let mut nl_4pl = nl_fast("log_agonist_vs_response_4pl");
    nl_4pl.identify_unstable_parameters = true;
    assert_eq!(exec_nl_res(&sheet, nl_4pl.clone()).unwrap_err(), curvebook_fit::FitError::InsufficientData { required: 4, actual: 3 });

    // 2) Fixing only Bottom = 0 leaves 3 local params for 3 Dataset B points (df = 0) -> Ambiguous + Unstable.
    let mut nl_indep = nl_4pl.clone();
    nl_indep.parameter_rules = vec![rule_const("Bottom", 0.0)];
    let indep_res = exec_nl(&sheet, nl_indep);
    assert!(indep_res.checklist.iter().any(|i| i.assumption == "Parameter identifiability (Ambiguous)" && i.status == ChecklistStatus::Violated));
    let indep_params = find_tab(&indep_res, "nonlinear_parameters");
    for expected in ["Top [B] (Unstable)", "LogEC50 [B] (Unstable)", "HillSlope [B] (Unstable)"] {
        assert!(indep_params.row_labels.iter().any(|lbl| lbl == expected), "missing {expected} in {:?}", indep_params.row_labels);
    }
    assert!(indep_params.row_labels.iter().filter(|lbl| lbl.contains("[A]")).all(|lbl| !lbl.ends_with("(Unstable)")));

    // 3) WF-52 Recovery: Bottom = Constant(0), Top & HillSlope = SharedGlobal, LogEC50 = Local.
    let mut nl_rec = nl_4pl;
    nl_rec.parameter_rules = vec![rule_const("Bottom", 0.0), rule_shared("Top"), rule_shared("HillSlope"), rule_local("LogEC50")];
    let rec_res = exec_nl(&sheet, nl_rec);
    assert!(rec_res.checklist.iter().all(|i| i.assumption != "Parameter identifiability (Ambiguous)"));
    assert!(rec_res.checklist.iter().any(|i| i.assumption == "Parameter identifiability" && i.status == ChecklistStatus::Pass));
    let extrap_warn = rec_res
        .checklist
        .iter()
        .find(|i| i.assumption == "LogEC50 outside observed X range (B)" && i.status == ChecklistStatus::Warn)
        .expect("expected LogEC50 outside observed X range (B) Warn checklist item");
    assert!(extrap_warn.remediation.is_none());

    // R 4.5.2 pooled stats::nls oracle (df = 5, rss = 0.4929092239189585).
    assert_close(cell_num(&tab_row(&rec_res, "nonlinear_gof", "Degrees of Freedom")[0]), 5.0, 1e-12, "pooled df");
    assert_close(cell_num(&tab_row(&rec_res, "nonlinear_gof", "Sum of Squares (RSS)")[0]), 0.4929092239189585, 1e-6, "pooled RSS");
    for (row_lbl, est, se) in [
        ("Top", 99.77111824194645, 0.26241943098077725),
        ("HillSlope", 1.009530731975136, 0.010931735463543379),
        ("LogEC50 [A]", -6.462008648887402, 0.006280679075401399),
        ("LogEC50 [B]", -6.036237822355189, 0.026404474352218665),
    ] {
        let row = tab_row(&rec_res, "nonlinear_parameters", row_lbl);
        assert_close(cell_num(&row[0]), est, 1e-5, &format!("{row_lbl} estimate"));
        assert_close(cell_num(&row[1]), se, 5e-5, &format!("{row_lbl} SE"));
    }
}

#[test]
fn r13_1_domain_restricted_ols_and_deming_r_oracle() {
    let xr = [2.0, 6.0, 10.0, 11.5, 13.0, 14.5, 16.0, 20.0, 24.0];
    let yr = [85.0, 42.0, 23.2, 26.5, 29.1, 32.8, 35.4, -12.0, -45.0];
    let points = xy_pts(&xr, &yr);

    // Direct OLS domain-restricted oracle matching R 4.5.2 stats::lm on x in [10, 16].
    let ols =
        simple_linear_regression(&points, &SimpleLinearOptions { x_range_filter: Some((Some(10.0), Some(16.0))), ..SimpleLinearOptions::default() })
            .unwrap();
    assert_eq!((ols.n, ols.n_excluded_by_range, ols.df_residual), (5, 4, 3));
    for (actual, expected, tol, lbl) in [
        (ols.slope, 2.046666666666666, 1e-12, "OLS slope"),
        (ols.se_slope, 0.06097965659560974, 1e-12, "OLS SE(slope)"),
        (ols.ci_slope.0, 1.852602183836498, 1e-10, "OLS CI(slope) lo"),
        (ols.ci_slope.1, 2.240731149496833, 1e-10, "OLS CI(slope) hi"),
        (ols.intercept, 2.793333333333345, 1e-11, "OLS intercept"),
        (ols.se_intercept, 0.803220370112062, 1e-11, "OLS SE(intercept)"),
        (ols.ci_intercept.0, 0.2371276341416011, 1e-9, "OLS CI(intercept) lo"),
        (ols.ci_intercept.1, 5.34953903252509, 1e-9, "OLS CI(intercept) hi"),
        (ols.r_squared, 0.9973439153439153, 1e-12, "OLS R2"),
        (ols.sy_x, 0.2892519086655542, 1e-12, "OLS Sy.x"),
        (ols.f_stat, 1126.482071713155, 1e-9, "OLS F"),
        (ols.p_value, 5.814309492494304e-05, 1e-10, "OLS p-value"),
        (ols.bands.first().unwrap().x, 10.0, 1e-12, "OLS band x_min"),
        (ols.bands.last().unwrap().x, 16.0, 1e-12, "OLS band x_max"),
    ] {
        assert_close(actual, expected, tol, lbl);
    }

    let in_range: Vec<DataPoint> = points.iter().filter(|p| (10.0..=16.0).contains(&p.x0())).cloned().collect();
    let dem = deming_regression(&in_range, &DemingOptions::default()).unwrap();
    assert_eq!((dem.n, dem.df), (5, 3));
    for (actual, expected, tol, lbl) in [
        (dem.slope, 2.05106863928226, 1e-12, "Deming slope"),
        (dem.se_slope, 0.05691497183347908, 1e-12, "Deming jackknife SE(slope)"),
        (dem.intercept, 2.73610768933062, 1e-11, "Deming intercept"),
        (dem.se_intercept, 0.6722665887968624, 1e-11, "Deming jackknife SE(intercept)"),
    ] {
        assert_close(actual, expected, tol, lbl);
    }

    // Verify ResultSheet OLS and orthogonal Deming (lambda = 1.0) via execute_analysis with x_range_filter.
    let sheet = xy_sheet_single("restricted", "Domain Restricted", "A", &xr, &yr);
    for (deming_model_ii, exp_slope, exp_int, lbl) in
        [(false, 2.046666666666666, 2.793333333333345, "OLS"), (true, 2.05106863928226, 2.73610768933062, "Deming")]
    {
        let rs = exec_lin(
            &sheet,
            LinearRegressionSpec { deming_model_ii, x_range_filter: Some((Some(10.0), Some(16.0))), ..LinearRegressionSpec::default() },
        );
        assert_close(rs.lookup_scalar("Slope", Some(0)).unwrap(), exp_slope, 1e-12, &format!("ResultSheet {lbl} slope"));
        assert_close(rs.lookup_scalar("YIntercept", Some(0)).unwrap(), exp_int, 1e-11, &format!("ResultSheet {lbl} intercept"));
        assert_close(rs.fitted_curves[0].points.first().unwrap().x, 10.0, 1e-12, &format!("{lbl} overlay min X"));
        assert_close(rs.fitted_curves[0].points.last().unwrap().x, 16.0, 1e-12, &format!("{lbl} overlay max X"));
    }
}
