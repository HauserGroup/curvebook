//! Permanent AUD-03 model-library remediation regression suite.
//!
//! Covers all 10 model-library findings in `crates/fit/src/library/{curves.rs, pharma.rs}`:
//! - `AUD-03-13`: `GaddumSchildUnity::grad` and `GaddumSchildConc::grad` at `AntagConc == 0.0`
//! - `AUD-03-14`: `RichardsFivePl::eval` log-space evaluation for small `S`
//! - `AUD-03-15`: Multi-dataset `initial_guess` partitioning by minimum `data_set_index`
//! - `AUD-03-16`: `OneSiteTotalLigandDepletion` and `OneSiteSpecificLigandDepletion` scaling + `hypot`
//! - `AUD-03-27`: `default_parameter_rules` for global binding, Gaddum/Schild, Allosteric, and ECanything
//! - `AUD-03-28`: Unclamped `raw_span` in `PlateauThenDecay`, `PlateauThenAssoc`, and `TwoGaussians`
//! - `AUD-03-29`: `ExponentialGrowth` log-linear OLS seed + 22 linear-concentration `pharma.rs` seeds
//! - `AUD-03-40`: Domain guards & `default_bounds` in `LinearDoseResponse`, `GaddumSchild`, `EnzymeInhibition`
//! - `AUD-03-41`: `AllostericModulatorCompetition::eval` underflow/mixed-regime handling
//! - `AUD-03-42`: Finite `DerivedScalar` guards, `OperationalAgonism` delegation, legacy `eval`/`grad` guards
//!
//! D1 Literature References:
//! - Hill (1910), *J. Physiol.* 40:iv–vii; Gaddum (1937), *J. Physiol.* 89:7P–9P;
//!   Arunlakshana & Schild (1959), *Br. J. Pharmacol.* 14:48–58; Richards (1959), *J. Exp. Bot.* 10;
//!   Segel (1975), *Enzyme Kinetics*; Black & Leff (1983), *Proc. R. Soc. B* 220:141–162;
//!   Ehlert (1988), *Mol. Pharmacol.* 33:187–194; Seber & Wild (1989), *Nonlinear Regression*;
//!   Swillens (1995), *Mol. Pharmacol.* 47:1197–1203; Motulsky & Christopoulos (2004), OUP;
//!   Gottschalk & Dunn (2005), *Anal. Biochem.* 343:54–65; Colquhoun (2007), *TIPS* 28:608–614.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use curvebook_doc::ParameterScope;
use curvebook_fit::library::curves::CURVE_EQUATION_IDS;
use curvebook_fit::library::pharma::PHARMA_EQUATION_IDS;
use curvebook_fit::{DataPoint, FitStatus, ParametricModel, SolverConfig, get_builtin_model, list_builtin_equations};
use curvebook_testkit::assert_approx_eq;

use common::{
    assert_converged, cfg_fast, cfg_fast_rules, dataset_param_val, find_derived, find_param_ds, fit_default, fit_with, linspace, param_ds_val,
    rule_const, rule_per_ds, series_xy,
};

fn logspace(exp_start: f64, exp_end: f64, n: usize) -> Vec<f64> {
    linspace(exp_start, exp_end, n).into_iter().map(|e| 10.0_f64.powf(e)).collect()
}

fn check_grad_vs_fd_general(model: &dyn ParametricModel, x: f64, params: &[f64], ds: usize, one_sided_pos: &[usize], rel_tol: f64) {
    let mut grad = vec![0.0; params.len()];
    assert!(model.grad(&[x], params, ds, &mut grad), "{} grad() returned false at x={x}, params={params:?}", model.id());
    for k in 0..params.len() {
        let fd = if one_sided_pos.contains(&k) {
            let h = 1e-12;
            let mut p_hi = params.to_vec();
            p_hi[k] += h;
            let y0 = model.eval(&[x], params, ds).unwrap();
            let y1 = model.eval(&[x], &p_hi, ds).unwrap();
            (y1 - y0) / h
        } else {
            let h = 1e-6 * params[k].abs().max(1e-8);
            let (mut p_hi, mut p_lo) = (params.to_vec(), params.to_vec());
            p_hi[k] += h;
            p_lo[k] -= h;
            let y_hi = model.eval(&[x], &p_hi, ds).unwrap();
            let y_lo = model.eval(&[x], &p_lo, ds).unwrap();
            (y_hi - y_lo) / (2.0 * h)
        };
        assert_approx_eq(grad[k], fd, 1e-6, rel_tol, &format!("{} grad[{k}] at x={x} params={params:?}", model.id()));
    }
}

#[test]
fn aud_03_13_gaddum_schild_unity_and_conc_grad_at_zero_antag() {
    let unity_log = get_builtin_model("ec50_shift_gaddum_schild_unity_log").unwrap();
    let unity_conc = get_builtin_model("ec50_shift_gaddum_schild_unity_conc").unwrap();
    let schild_conc = get_builtin_model("ec50_shift_gaddum_schild_conc").unwrap();

    // Exact analytical derivative at AntagConc == 0.0:
    // dY/dAntagConc = -Span * sig*(1-sig) * HillSlope * 10^pKB (at x = LogEC50 -> sig = 0.5, dsig = 0.25).
    let p_ul_zero = [5.0, 95.0, -7.0, 1.2, 7.8, 0.0];
    let mut g_ul = vec![0.0; 6];
    assert!(unity_log.grad(&[-7.0], &p_ul_zero, 0, &mut g_ul));
    let expected_ul = -90.0 * 0.25 * 1.2 * 10.0_f64.powf(7.8);
    assert_approx_eq(g_ul[5], expected_ul, 1e-3, 1e-12, "unity_log dY/dAntag at 0");
    check_grad_vs_fd_general(unity_log.as_ref(), -7.0, &p_ul_zero, 0, &[5], 1e-4);
    check_grad_vs_fd_general(unity_log.as_ref(), -6.5, &[5.0, 95.0, -7.0, 1.2, 7.8, 1e-7], 0, &[], 1e-6);

    let p_uc_zero = [5.0, 95.0, 1e-7, 1.2, 7.8, 0.0];
    let mut g_uc = vec![0.0; 6];
    assert!(unity_conc.grad(&[1e-7], &p_uc_zero, 0, &mut g_uc));
    assert_approx_eq(g_uc[5], expected_ul, 1e-3, 1e-12, "unity_conc dY/dAntag at 0");
    check_grad_vs_fd_general(unity_conc.as_ref(), 1e-7, &p_uc_zero, 0, &[5], 1e-4);
    check_grad_vs_fd_general(unity_conc.as_ref(), 3e-7, &[5.0, 95.0, 1e-7, 1.2, 7.8, 1e-7], 0, &[], 1e-6);

    // GaddumSchildConc at AntagConc == 0.0 across SchildSlope == 1, < 1, > 1:
    let p_sc_one = [5.0, 95.0, 1e-7, 1.2, 7.5, 1.0, 0.0];
    let mut g_sc = vec![0.0; 7];
    assert!(schild_conc.grad(&[1e-7], &p_sc_one, 0, &mut g_sc));
    let expected_sc = -90.0 * 0.25 * 1.2 * 10.0_f64.powf(7.5);
    assert_approx_eq(g_sc[6], expected_sc, 1e-3, 1e-12, "schild_conc dY/dAntag at 0, s=1");
    check_grad_vs_fd_general(schild_conc.as_ref(), 1e-7, &p_sc_one, 0, &[6], 1e-4);
    check_grad_vs_fd_general(schild_conc.as_ref(), 3e-7, &[5.0, 95.0, 1e-7, 1.2, 7.5, 1.15, 1e-7], 0, &[], 1e-6);

    let p_sc_lt1 = [5.0, 95.0, 1e-7, 1.2, 7.5, 0.5, 0.0];
    assert!(!schild_conc.grad(&[1e-7], &p_sc_lt1, 0, &mut g_sc), "SchildSlope < 1 at AntagConc=0 has infinite derivative and must return false");

    let p_sc_gt1 = [5.0, 95.0, 1e-7, 1.2, 7.5, 1.5, 0.0];
    assert!(schild_conc.grad(&[1e-7], &p_sc_gt1, 0, &mut g_sc));
    assert_eq!(g_sc[6], 0.0, "SchildSlope > 1 at AntagConc=0 has zero right-hand derivative");

    // Optimization escape from AntagConc = 0.0 with simplex disabled:
    let log_xs = linspace(-9.0, -3.0, 15);
    let true_antag = 5e-7;
    let ys: Vec<f64> = log_xs.iter().map(|&x| unity_log.eval(&[x], &[5.0, 95.0, -7.0, 1.2, 7.8, true_antag], 0).unwrap()).collect();
    let cfg = SolverConfig {
        use_simplex_warm_start: false,
        parameter_rules: vec![
            rule_const("Bottom", 5.0),
            rule_const("Top", 95.0),
            rule_const("LogEC50", -7.0),
            rule_const("HillSlope", 1.2),
            rule_const("pKB", 7.8),
        ],
        ..cfg_fast()
    };
    let fit = fit_with(unity_log.as_ref(), &[series_xy(0, "UnknownB", &log_xs, &ys)], &cfg);
    assert_converged(&fit, "GaddumSchildUnity escape from AntagConc=0");
    assert_approx_eq(param_ds_val(&fit, 0, "AntagConc"), true_antag, 1e-13, 1e-6, "recovered AntagConc");
}

#[test]
fn aud_03_14_richards_5pl_small_s_continuity_and_exact_logec50() {
    let model = get_builtin_model("log_agonist_vs_response_5pl").unwrap();
    for s in [1e-4_f64, 1e-3_f64] {
        let params = [0.0, 100.0, -6.0, 1.0, s];
        let x_before = -6.0 - 700.0 / core::f64::consts::LN_10;
        let x_after = -6.0 - 700.001 / core::f64::consts::LN_10;
        let y_before = model.eval(&[x_before], &params, 0).unwrap();
        let y_after = model.eval(&[x_after], &params, 0).unwrap();
        let expected_before = 100.0 * (-s * 700.0).exp();
        let expected_after = 100.0 * (-s * 700.001).exp();
        assert_approx_eq(y_before, expected_before, 1e-10, 1e-12, &format!("5PL u=700 S={s}"));
        assert_approx_eq(y_after, expected_after, 1e-10, 1e-12, &format!("5PL u=700.001 S={s}"));
        assert!((y_before - y_after).abs() < 1e-3, "discontinuity at u=700 for S={s}: {y_before} -> {y_after}");

        let scalars = model.derived_transforms(&params);
        let log_ec50 = scalars.iter().find(|d| d.name == "LogEC50").unwrap().value;
        let y_at_ec50 = model.eval(&[log_ec50], &params, 0).unwrap();
        assert_approx_eq(y_at_ec50, 50.0, 1e-10, 1e-11, &format!("5PL eval(LogEC50) S={s}"));
    }
    // Extreme tails and non-finite guards:
    assert_approx_eq(model.eval(&[1000.0], &[5.0, 95.0, -6.0, 1.0, 1e-4], 0).unwrap(), 95.0, 1e-12, 1e-12, "5PL upper tail");
    assert_approx_eq(model.eval(&[-1e9], &[5.0, 95.0, -6.0, 1.0, 1e-4], 0).unwrap(), 5.0, 1e-12, 1e-12, "5PL lower tail");
    assert!(model.eval(&[f64::NAN], &[5.0, 95.0, -6.0, 1.0, 1.0], 0).is_none());
    assert!(model.eval(&[-6.0], &[f64::NAN, 95.0, -6.0, 1.0, 1.0], 0).is_none());
    assert!(model.eval(&[-6.0], &[5.0, 95.0, -6.0, 1.0, 0.0], 0).is_none());
}

#[test]
fn aud_03_15_multidataset_initial_guess_partitions_by_min_index() {
    let make_two_ds_pts = |ds_a: usize, ds_b: usize, xs: &[f64], ys_a: &[f64], ys_b: &[f64]| -> Vec<DataPoint> {
        let mut pts = Vec::with_capacity(xs.len() * 2);
        for (row, (&x, &y)) in xs.iter().zip(ys_a).enumerate() {
            pts.push(DataPoint { data_set_index: ds_a, row, ..DataPoint::xy(x, y) });
        }
        for (row, (&x, &y)) in xs.iter().zip(ys_b).enumerate() {
            pts.push(DataPoint { data_set_index: ds_b, row, ..DataPoint::xy(x, y) });
        }
        pts
    };

    // 1. One-site and two-site global binding models:
    let xs_bind = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let y_tot: Vec<f64> = xs_bind.iter().map(|&x| 1200.0 * x / (3.0 + x) + 15.0 * x + 40.0).collect();
    let y_ns: Vec<f64> = xs_bind.iter().map(|&x| 15.0 * x + 40.0).collect();
    for id in ["one_site_total_and_ns_global", "two_site_total_and_ns_global"] {
        let m = get_builtin_model(id).unwrap();
        let g01 = m.initial_guess(&make_two_ds_pts(0, 1, &xs_bind, &y_tot, &y_ns));
        let g12 = m.initial_guess(&make_two_ds_pts(1, 2, &xs_bind, &y_tot, &y_ns));
        let g37 = m.initial_guess(&make_two_ds_pts(3, 7, &xs_bind, &y_tot, &y_ns));
        assert_eq!(g01, g12, "{id} initial_guess differed for (0,1) vs (1,2)");
        assert_eq!(g01, g37, "{id} initial_guess differed for (0,1) vs (3,7)");
    }

    // 2. EC50 shift ratio (log and linear concentration):
    let lx = linspace(-11.0, -5.0, 13);
    let y_ctrl: Vec<f64> = lx.iter().map(|&x| 5.0 + 95.0 / (1.0 + 10.0_f64.powf(-8.0 - x))).collect();
    let y_shift: Vec<f64> = lx.iter().map(|&x| 5.0 + 95.0 / (1.0 + 10.0_f64.powf(-7.0 - x))).collect();
    let m_sr_log = get_builtin_model("ec50_shift_ratio_log").unwrap();
    let gl01 = m_sr_log.initial_guess(&make_two_ds_pts(0, 1, &lx, &y_ctrl, &y_shift));
    let gl12 = m_sr_log.initial_guess(&make_two_ds_pts(1, 2, &lx, &y_ctrl, &y_shift));
    assert_eq!(gl01, gl12, "ec50_shift_ratio_log initial_guess mismatch for (1,2)");
    assert_approx_eq(gl12[4], 10.0, 0.5, 0.05, "ec50_shift_ratio_log ratio guess");

    let cx = logspace(-11.0, -5.0, 13);
    let m_sr_conc = get_builtin_model("ec50_shift_ratio_conc").unwrap();
    let gc01 = m_sr_conc.initial_guess(&make_two_ds_pts(0, 1, &cx, &y_ctrl, &y_shift));
    let gc12 = m_sr_conc.initial_guess(&make_two_ds_pts(1, 2, &cx, &y_ctrl, &y_shift));
    assert_eq!(gc01, gc12, "ec50_shift_ratio_conc initial_guess mismatch for (1,2)");
    assert_approx_eq(gc12[2], 1e-8, 5e-9, 0.25, "ec50_shift_ratio_conc EC50Control guess");
    assert_approx_eq(gc12[4], 10.0, 1.0, 0.15, "ec50_shift_ratio_conc EC50Ratio guess");

    // 3. TwoLinesIntersection on columns (1, 2): y = 2x + 1 and y = -x + 10 intersect at (3, 7).
    let xs_l = [0.0, 1.0, 2.0, 4.0, 5.0];
    let y_l0: Vec<f64> = xs_l.iter().map(|&x| 2.0 * x + 1.0).collect();
    let y_l1: Vec<f64> = xs_l.iter().map(|&x| -x + 10.0).collect();
    let m_lines = get_builtin_model("two_lines_intersection").unwrap();
    let g_l01 = m_lines.initial_guess(&make_two_ds_pts(0, 1, &xs_l, &y_l0, &y_l1));
    let g_l12 = m_lines.initial_guess(&make_two_ds_pts(1, 2, &xs_l, &y_l0, &y_l1));
    assert_eq!(g_l01, g_l12, "two_lines_intersection initial_guess mismatch for (1,2)");
    assert_approx_eq(g_l12[0], 3.0, 1e-12, 1e-12, "XCross guess on (1,2)");
    assert_approx_eq(g_l12[1], 7.0, 1e-12, 1e-12, "YCross guess on (1,2)");
}

#[test]
fn aud_03_16_ligand_depletion_overflow_huge_ns_and_tight_binding() {
    let total_dep = get_builtin_model("one_site_total_ligand_depletion").unwrap();
    let spec_dep = get_builtin_model("one_site_specific_ligand_depletion").unwrap();

    // (a) Overflow regime: Bmax = Kd_cpm = X = 8e153, NS = 0 -> exact Y = 8e153 * (3 - sqrt(5)) / 2.
    let y_ovf = total_dep.eval(&[8e153], &[8e153, 8e147, 0.0, 2000.0, 0.5], 0).unwrap();
    let expected_ovf = 8e153 * 0.5 * (3.0 - 5.0_f64.sqrt());
    assert_approx_eq(y_ovf, expected_ovf, 1e140, 1e-13, "Swillens total 8e153 overflow");
    assert!(y_ovf <= 8e153);

    // (b) Normal X = 1e5 with huge NS = 1e150 (and 1e300): physical invariant Y <= X and Y -> X.
    for huge_ns in [1e150_f64, 1e300_f64] {
        let y_ns = total_dep.eval(&[1e5], &[3200.0, 2.5e-3, huge_ns, 2000.0, 0.5], 0).unwrap();
        assert!(y_ns.is_finite() && y_ns > 0.0 && y_ns <= 1e5, "NS={huge_ns}: Y={y_ns} violated 0 < Y <= X");
        assert_approx_eq(y_ns, 1e5, 1e-6, 1e-12, &format!("Swillens total NS={huge_ns}"));
    }

    // (c) Tight-binding cancellation: Bmax = 1e5, X = 1e5, Kd_cpm = 1e-11 (Kd = 1e-17), NS = 0.
    // Free ligand solves L^2 + 1e-11 * L - 1e-6 = 0 => L = (-1e-11 + sqrt(1e-22 + 4e-6)) / 2 = 0.000999999995.
    let y_tb = total_dep.eval(&[1e5], &[1e5, 1e-17, 0.0, 2000.0, 0.5], 0).unwrap();
    let l_exact = 2e-6 / (1e-11 + (1e-11_f64).hypot(2e-3));
    let y_tb_exact = 1e5 - l_exact;
    assert_approx_eq(y_tb, y_tb_exact, 1e-10, 1e-12, "Swillens tight-binding Kd_cpm=1e-11");
    assert!(y_tb < 1e5, "tight-binding must not collapse free ligand to 0");

    // (d) Ordinary & mixed regimes verified against exact mass-balance conservation:
    // X = L + Bmax * L / (Kd_cpm + L) + NS * L with L = X - Y.
    for &(bmax, kd, ns, x) in &[(3200.0, 0.0025, 0.04, 5000.0), (500.0, 0.001, 0.0, 200.0), (1e5, 1e-17, 1.0, 1e5), (1e5, 0.01, 50.0, 2e4)] {
        let y = total_dep.eval(&[x], &[bmax, kd, ns, 2000.0, 0.5], 0).unwrap();
        let kd_cpm = kd * 0.5 * 1000.0 * 2000.0;
        let l_free = x - y;
        let y_reconstructed = bmax * l_free / (kd_cpm + l_free) + ns * l_free;
        assert!(y >= 0.0 && y <= x);
        assert_approx_eq(y, y_reconstructed, 1e-9, 1e-11, &format!("mass balance bmax={bmax} kd={kd} ns={ns} x={x}"));
    }

    // Specific ligand depletion hardened with scaling + hypot:
    let y_spec_ovf = spec_dep.eval(&[8e153], &[8e153, 8e153, 0.0], 0).unwrap();
    assert_approx_eq(y_spec_ovf, expected_ovf, 1e140, 1e-13, "Swillens specific 8e153 overflow");
    let y_spec_tb = spec_dep.eval(&[1e5], &[1e5, 1e-11, 0.0], 0).unwrap();
    assert_approx_eq(y_spec_tb, y_tb_exact, 1e-10, 1e-12, "Swillens specific tight-binding");
    let y_spec_huge_ns = spec_dep.eval(&[1e5], &[3200.0, 2500.0, 1e150], 0).unwrap();
    // Free = X / (1 + NS) ~ 1e-145 => Y = Rt * Free / (Kd + Free) ~ 3200 * 1e-145 / 2500 = 1.28e-145.
    assert_approx_eq(y_spec_huge_ns, 1.28e-145, 1e-156, 1e-12, "Swillens specific huge NS");
}

#[test]
fn aud_03_27_default_parameter_rules_global_and_ecf() {
    // 1. OneSiteTotalAndNsGlobal with empty parameter_rules:
    let m1 = get_builtin_model("one_site_total_and_ns_global").unwrap();
    let xs = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let p1 = [1800.0, 3.5, 42.0, 85.0];
    let y_tot1: Vec<f64> = xs.iter().map(|&x| m1.eval(&[x], &p1, 0).unwrap()).collect();
    let y_ns1: Vec<f64> = xs.iter().map(|&x| m1.eval(&[x], &p1, 1).unwrap()).collect();
    let fit1 = fit_with(m1.as_ref(), &[series_xy(0, "Total", &xs, &y_tot1), series_xy(1, "NS", &xs, &y_ns1)], &cfg_fast());
    assert_converged(&fit1, "one_site_total_and_ns_global default rules");
    assert_eq!(fit1.k_free, 4);
    assert_approx_eq(dataset_param_val(&fit1, 0, "Bmax"), p1[0], 1e-4, 1e-5, "global1 Bmax");
    assert_approx_eq(dataset_param_val(&fit1, 0, "Kd"), p1[1], 1e-5, 1e-5, "global1 Kd");
    assert_approx_eq(dataset_param_val(&fit1, 1, "NS"), p1[2], 1e-5, 1e-5, "global1 NS");
    assert_approx_eq(dataset_param_val(&fit1, 1, "Background"), p1[3], 1e-4, 1e-5, "global1 Background");

    // 2. TwoSiteTotalAndNsGlobal with empty parameter_rules:
    let m2 = get_builtin_model("two_site_total_and_ns_global").unwrap();
    let xs2 = [0.05, 0.15, 0.4, 1.0, 2.5, 6.0, 15.0, 35.0, 80.0, 160.0];
    let p2 = [120.0, 0.45, 240.0, 22.0, 0.75, 14.0];
    let y_tot2: Vec<f64> = xs2.iter().map(|&x| m2.eval(&[x], &p2, 0).unwrap()).collect();
    let y_ns2: Vec<f64> = xs2.iter().map(|&x| m2.eval(&[x], &p2, 1).unwrap()).collect();
    let fit2 = fit_with(m2.as_ref(), &[series_xy(0, "Total", &xs2, &y_tot2), series_xy(1, "NS", &xs2, &y_ns2)], &cfg_fast());
    assert_converged(&fit2, "two_site_total_and_ns_global default rules");
    assert_eq!(fit2.k_free, 6);
    assert_approx_eq(dataset_param_val(&fit2, 1, "NS"), p2[4], 1e-5, 1e-5, "global2 NS");
    assert_approx_eq(dataset_param_val(&fit2, 1, "Background"), p2[5], 1e-4, 1e-5, "global2 Background");

    // 3. GaddumSchild & GaddumSchildConc default SharedGlobal (passing ONLY AntagConc):
    let lx = linspace(-9.0, -3.5, 12);
    let cx: Vec<f64> = lx.iter().map(|&v| 10.0_f64.powf(v)).collect();
    let gs_log = get_builtin_model("ec50_shift_gaddum_schild").unwrap();
    let ds_gs_log: Vec<_> = [0.0, 1e-7, 1e-6]
        .iter()
        .enumerate()
        .map(|(k, &b)| {
            let ys: Vec<f64> = lx.iter().map(|&x| gs_log.eval(&[x], &[5.0, 95.0, -7.0, 1.1, 7.5, 1.15, b], k).unwrap()).collect();
            series_xy(k, &format!("B={b}"), &lx, &ys)
        })
        .collect();
    let fit_gs_log = fit_with(gs_log.as_ref(), &ds_gs_log, &cfg_fast_rules(vec![rule_per_ds("AntagConc", &[(0, 0.0), (1, 1e-7), (2, 1e-6)])]));
    assert_converged(&fit_gs_log, "ec50_shift_gaddum_schild default SharedGlobal");
    assert_eq!(fit_gs_log.k_free, 6);
    assert_approx_eq(param_ds_val(&fit_gs_log, 0, "pA2"), 7.5, 1e-4, 1e-4, "gs_log pA2");

    let gs_conc = get_builtin_model("ec50_shift_gaddum_schild_conc").unwrap();
    let ds_gs_conc: Vec<_> = [0.0, 1e-7, 1e-6]
        .iter()
        .enumerate()
        .map(|(k, &b)| {
            let ys: Vec<f64> = cx.iter().map(|&x| gs_conc.eval(&[x], &[5.0, 95.0, 1e-7, 1.1, 7.5, 1.15, b], k).unwrap()).collect();
            series_xy(k, &format!("B={b}"), &cx, &ys)
        })
        .collect();
    let fit_gs_conc = fit_with(gs_conc.as_ref(), &ds_gs_conc, &cfg_fast_rules(vec![rule_per_ds("AntagConc", &[(0, 0.0), (1, 1e-7), (2, 1e-6)])]));
    assert_converged(&fit_gs_conc, "ec50_shift_gaddum_schild_conc default SharedGlobal");
    assert_eq!(fit_gs_conc.k_free, 6);
    assert_approx_eq(param_ds_val(&fit_gs_conc, 0, "pA2"), 7.5, 1e-4, 1e-4, "gs_conc pA2");

    // 4. AllostericEc50Shift & AllostericEc50ShiftConc default SharedGlobal:
    let allo_log = get_builtin_model("allosteric_ec50_shift").unwrap();
    let ds_al_log: Vec<_> = [0.0, 1e-6, 1e-5, 1e-4]
        .iter()
        .enumerate()
        .map(|(k, &m)| {
            let ys: Vec<f64> = lx.iter().map(|&x| allo_log.eval(&[x], &[4.0, 96.0, -7.0, 1.1, 2e-6, 0.1, m], k).unwrap()).collect();
            series_xy(k, &format!("M={m}"), &lx, &ys)
        })
        .collect();
    let fit_al_log =
        fit_with(allo_log.as_ref(), &ds_al_log, &cfg_fast_rules(vec![rule_per_ds("ModConc", &[(0, 0.0), (1, 1e-6), (2, 1e-5), (3, 1e-4)])]));
    assert_converged(&fit_al_log, "allosteric_ec50_shift default SharedGlobal");
    assert_eq!(fit_al_log.k_free, 6);
    assert_approx_eq(param_ds_val(&fit_al_log, 0, "Kb"), 2e-6, 1e-10, 1e-4, "allo_log Kb");
    assert_approx_eq(param_ds_val(&fit_al_log, 0, "alpha"), 0.1, 1e-4, 1e-4, "allo_log alpha");

    let allo_conc = get_builtin_model("allosteric_ec50_shift_conc").unwrap();
    let ds_al_conc: Vec<_> = [0.0, 1e-6, 1e-5, 1e-4]
        .iter()
        .enumerate()
        .map(|(k, &m)| {
            let ys: Vec<f64> = cx.iter().map(|&x| allo_conc.eval(&[x], &[4.0, 96.0, 1e-7, 1.1, 2e-6, 0.1, m], k).unwrap()).collect();
            series_xy(k, &format!("M={m}"), &cx, &ys)
        })
        .collect();
    let fit_al_conc =
        fit_with(allo_conc.as_ref(), &ds_al_conc, &cfg_fast_rules(vec![rule_per_ds("ModConc", &[(0, 0.0), (1, 1e-6), (2, 1e-5), (3, 1e-4)])]));
    assert_converged(&fit_al_conc, "allosteric_ec50_shift_conc default SharedGlobal");
    assert_eq!(fit_al_conc.k_free, 6);
    assert_approx_eq(param_ds_val(&fit_al_conc, 0, "Kb"), 2e-6, 1e-10, 1e-4, "allo_conc Kb");
    assert_approx_eq(param_ds_val(&fit_al_conc, 0, "alpha"), 0.1, 1e-4, 1e-4, "allo_conc alpha");

    // 5. DoseResponseEcF & DoseResponseEcFConc default Constant(50.0) for F:
    for (id, xs_v, true_mid) in [("log_agonist_vs_response_ecf", lx.as_slice(), -7.0_f64), ("agonist_vs_response_ecf", cx.as_slice(), 1e-7_f64)] {
        let m = get_builtin_model(id).unwrap();
        let rules = m.default_parameter_rules(&[]);
        assert!(rules.iter().any(|r| r.name == "F" && matches!(r.scope, ParameterScope::Constant(v) if (v - 50.0).abs() < 1e-12)));
        let ys: Vec<f64> = xs_v.iter().map(|&x| m.eval(&[x], &[5.0, 95.0, true_mid, 1.3, 50.0], 0).unwrap()).collect();
        let fit = fit_with(m.as_ref(), &[series_xy(0, id, xs_v, &ys)], &cfg_fast());
        assert_converged(&fit, &format!("{id} default F=50"));
        assert_eq!(fit.k_free, 4);
        let p_name = if id.starts_with("log_") { "LogECF" } else { "ECF" };
        assert_approx_eq(param_ds_val(&fit, 0, p_name), true_mid, 1e-12, 1e-5, &format!("{id} {p_name}"));
    }
}

#[test]
fn aud_03_28_short_span_plateau_and_two_gaussians_initial_guess_and_fit() {
    let ts = linspace(0.0, 0.1, 21);

    let decay = get_builtin_model("plateau_followed_by_one_phase_decay").unwrap();
    let y_dec: Vec<f64> = ts.iter().map(|&t| if t < 0.02 { 100.0 } else { 10.0 + 90.0 * (-50.0 * (t - 0.02)).exp() }).collect();
    let g_dec = decay.initial_guess(&ts.iter().zip(&y_dec).map(|(&x, &y)| DataPoint::xy(x, y)).collect::<Vec<_>>());
    assert!(g_dec[1] > 0.0 && g_dec[1] < 0.1, "PlateauThenDecay X0 seed out of data span: {}", g_dec[1]);
    let fit_dec = fit_default(decay.as_ref(), &[series_xy(0, "ms_decay", &ts, &y_dec)]);
    assert_converged(&fit_dec, "millisecond PlateauThenDecay");
    assert!(fit_dec.r_squared > 0.999_999, "R^2={}", fit_dec.r_squared);
    assert_approx_eq(param_ds_val(&fit_dec, 0, "Y0"), 100.0, 1e-3, 1e-4, "ms decay Y0");
    assert_approx_eq(param_ds_val(&fit_dec, 0, "X0"), 0.02, 1e-4, 1e-3, "ms decay X0");
    assert_approx_eq(param_ds_val(&fit_dec, 0, "Plateau"), 10.0, 1e-3, 1e-4, "ms decay Plateau");
    assert_approx_eq(param_ds_val(&fit_dec, 0, "K"), 50.0, 1e-2, 1e-3, "ms decay K");

    let assoc = get_builtin_model("plateau_followed_by_one_phase_association").unwrap();
    let y_ass: Vec<f64> = ts.iter().map(|&t| if t < 0.02 { 10.0 } else { 10.0 + 90.0 * (1.0 - (-50.0 * (t - 0.02)).exp()) }).collect();
    let g_ass = assoc.initial_guess(&ts.iter().zip(&y_ass).map(|(&x, &y)| DataPoint::xy(x, y)).collect::<Vec<_>>());
    assert!(g_ass[1] > 0.0 && g_ass[1] < 0.1, "PlateauThenAssoc X0 seed out of data span: {}", g_ass[1]);
    let fit_ass = fit_default(assoc.as_ref(), &[series_xy(0, "ms_assoc", &ts, &y_ass)]);
    assert_converged(&fit_ass, "millisecond PlateauThenAssoc");
    assert!(fit_ass.r_squared > 0.999_999, "R^2={}", fit_ass.r_squared);
    assert_approx_eq(param_ds_val(&fit_ass, 0, "X0"), 0.02, 1e-4, 1e-3, "ms assoc X0");
    assert_approx_eq(param_ds_val(&fit_ass, 0, "K"), 50.0, 1e-2, 1e-3, "ms assoc K");

    let two_g = get_builtin_model("sum_of_two_gaussians").unwrap();
    let short_pts: Vec<DataPoint> = linspace(0.0, 0.05, 11).into_iter().map(|x| DataPoint::xy(x, 10.0)).collect();
    let g_tg = two_g.initial_guess(&short_pts);
    assert!(g_tg[1] > 0.0 && g_tg[1] < 0.05 && g_tg[4] > 0.0 && g_tg[4] < 0.05, "TwoGaussians means out of span: {g_tg:?}");
    assert_approx_eq(g_tg[2], 0.05 / 8.0, 1e-12, 1e-12, "TwoGaussians SD1");
}

#[test]
fn aud_03_29_exponential_growth_offset_x_and_nanomolar_pharma_seeds() {
    // 1. ExponentialGrowth & MalthusianGrowth on calendar years X = 2000..2010 and offset X = 100..200:
    let years: [f64; 6] = [2000.0, 2002.0, 2004.0, 2006.0, 2008.0, 2010.0];
    let offset_x: [f64; 6] = [100.0, 120.0, 140.0, 160.0, 180.0, 200.0];
    for id in ["exponential_growth", "malthusian_growth"] {
        let model = get_builtin_model(id).unwrap();
        // Representable offset-X exponential (k = 0.05, Y(2000) = 10 -> Y0 = 10 * exp(-100)):
        // Note: uncentered Y = Y0 * exp(k * X) over [2000, 2010] has exact Jacobian collinearity
        // R^2 = 1 / (1 + CV(X)^2) = 0.999997 > 0.9999 (Ambiguous), while still recovering exact k & Y(X).
        let ys_mod: Vec<f64> = years.iter().map(|&x| 10.0 * (0.05 * (x - 2000.0)).exp()).collect();
        let fit = fit_default(model.as_ref(), &[series_xy(0, "Years", &years, &ys_mod)]);
        assert!(matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous), "{id} X=2000..2010 status={:?}", fit.status);
        assert_approx_eq(param_ds_val(&fit, 0, "k"), 0.05, 1e-7, 1e-6, &format!("{id} k"));
        let y0_fit = param_ds_val(&fit, 0, "Y0");
        let k_fit = param_ds_val(&fit, 0, "k");
        assert_approx_eq(model.eval(&[2000.0], &[y0_fit, k_fit], 0).unwrap(), 10.0, 1e-5, 1e-5, &format!("{id} Y(2000)"));
        assert_approx_eq(model.eval(&[2010.0], &[y0_fit, k_fit], 0).unwrap(), 10.0 * 0.5_f64.exp(), 1e-5, 1e-5, &format!("{id} Y(2010)"));

        // Wider relative-span offset X = 100..200 achieves full FitStatus::Converged:
        let ys_off: Vec<f64> = offset_x.iter().map(|&x| 5.0 * (0.04 * (x - 100.0)).exp()).collect();
        let fit_off = fit_default(model.as_ref(), &[series_xy(0, "Offset", &offset_x, &ys_off)]);
        assert_converged(&fit_off, &format!("{id} X=100..200 k=0.04"));
        assert_approx_eq(param_ds_val(&fit_off, 0, "k"), 0.04, 1e-7, 1e-6, &format!("{id} offset k"));
        assert_approx_eq(param_ds_val(&fit_off, 0, "Y0"), 5.0 * (-4.0_f64).exp(), 1e-9, 1e-6, &format!("{id} offset Y0"));

        // Steep offset-X (Y = 10..1000, k_ols * 2000 = 921 > 700): seed must remain safely evaluable:
        let ys_steep: Vec<f64> = years.iter().map(|&x| 10.0 * (100.0_f64.ln() / 10.0 * (x - 2000.0)).exp()).collect();
        let pts_steep: Vec<DataPoint> = years.iter().zip(&ys_steep).map(|(&x, &y)| DataPoint::xy(x, y)).collect();
        let seed = model.initial_guess(&pts_steep);
        assert!(seed[0].is_finite() && seed[0] > 0.0 && seed[1].is_finite());
        for &x in &years {
            assert!((seed[1] * x).abs() <= 600.0 + 1e-9, "{id} seed k*x exceeded 600: {}", seed[1] * x);
            assert!(model.eval(&[x], &seed, 0).is_some(), "{id} seed eval returned None at x={x}");
        }
    }

    // 2. All 22 linear-concentration pharma structs preserve nanomolar/picomolar seeds [1e-12, 1e-8] M:
    let nm_xs = logspace(-12.0, -8.0, 17);
    let nm_ys: Vec<f64> = nm_xs.iter().map(|&x| 100.0 * x / (1e-10 + x)).collect();
    let nm_pts: Vec<DataPoint> = nm_xs.iter().zip(&nm_ys).map(|(&x, &y)| DataPoint::xy(x, y)).collect();

    // (model_id, index_of_concentration_param_in_initial_guess)
    let linear_conc_models: &[(&str, usize)] = &[
        ("agonist_vs_response_3pl", 2),
        ("agonist_vs_response_4pl", 2),
        ("agonist_vs_response_normalized", 0),
        ("agonist_vs_response_normalized_var", 0),
        ("agonist_vs_response_5pl", 2),
        ("bell_shaped_dose_response_conc", 3),
        ("biphasic_dose_response_conc", 3),
        ("agonist_vs_response_ecf", 2),
        ("absolute_ic50_4pl_conc", 2),
        ("ec50_shift_gaddum_schild_conc", 2),
        ("ec50_shift_gaddum_schild_unity_conc", 2),
        ("ec50_shift_ratio_conc", 2),
        ("allosteric_ec50_shift_conc", 2),
        ("operational_model_agonist_conc", 2),
        ("operational_model_depletion_conc", 2),
        ("one_site_specific_binding", 1),
        ("two_sites_specific_binding", 1),
        ("one_site_total_and_ns_global", 1),
        ("one_site_specific_ligand_depletion", 1),
        ("allosteric_modulator_saturation", 1),
        ("michaelis_menten", 1),
        ("allosteric_sigmoidal_kinetics", 1),
        ("substrate_inhibition", 1),
        ("competitive_inhibition", 1),
    ];
    for &(id, p_idx) in linear_conc_models {
        let model = get_builtin_model(id).unwrap();
        let guess = model.initial_guess(&nm_pts);
        let c_seed = guess[p_idx];
        assert!(
            (1e-13..1e-8).contains(&c_seed),
            "{id} linear-concentration seed param[{p_idx}]={c_seed:e} was clamped instead of preserving [1e-12, 1e-8] M scale"
        );
    }

    // Full default nonlinear fits at 100 pM (1e-10 M):
    for (id, p_name) in [
        ("michaelis_menten", "Km"),
        ("one_site_specific_binding", "Kd"),
        ("agonist_vs_response_4pl", "EC50"),
        ("agonist_vs_response_normalized", "EC50"),
    ] {
        let model = get_builtin_model(id).unwrap();
        let fit = fit_with(model.as_ref(), &[series_xy(0, id, &nm_xs, &nm_ys)], &cfg_fast());
        assert_converged(&fit, &format!("{id} 100 pM fit"));
        assert_approx_eq(param_ds_val(&fit, 0, p_name), 1e-10, 1e-15, 1e-5, &format!("{id} {p_name} at 100 pM"));
    }

    // Ec50ShiftRatio linear concentration fit at 100 pM control -> 1 nM shifted (10x ratio):
    let sr_conc = get_builtin_model("ec50_shift_ratio_conc").unwrap();
    let ys_ctrl: Vec<f64> = nm_xs.iter().map(|&x| 5.0 + 95.0 * x / (1e-10 + x)).collect();
    let ys_shft: Vec<f64> = nm_xs.iter().map(|&x| 5.0 + 95.0 * x / (1e-9 + x)).collect();
    let fit_sr = fit_with(sr_conc.as_ref(), &[series_xy(0, "Ctrl", &nm_xs, &ys_ctrl), series_xy(1, "Shifted", &nm_xs, &ys_shft)], &cfg_fast());
    assert_converged(&fit_sr, "ec50_shift_ratio_conc 100 pM fit");
    assert_approx_eq(param_ds_val(&fit_sr, 0, "EC50Control"), 1e-10, 1e-15, 1e-5, "sr_conc 100 pM EC50Control");
    assert_approx_eq(param_ds_val(&fit_sr, 0, "EC50Ratio"), 10.0, 1e-4, 1e-5, "sr_conc 100 pM EC50Ratio");
}

#[test]
fn aud_03_40_domain_guards_linear_dr_gaddum_schild_enzyme_inhibition() {
    // 1. LinearDoseResponse: NaN must return None (not top/bottom), while genuine overflow saturates.
    for id in ["agonist_vs_response_4pl", "inhibitor_vs_response_4pl"] {
        let m = get_builtin_model(id).unwrap();
        assert!(m.eval(&[1.0], &[0.0, 100.0, f64::NAN, 1.0], 0).is_none());
        assert!(m.eval(&[1.0], &[0.0, 100.0, 1.0, f64::NAN], 0).is_none());
        assert!(m.eval(&[1.0], &[f64::NAN, 100.0, 1.0, 1.0], 0).is_none());
        assert!(m.eval(&[f64::NAN], &[0.0, 100.0, 1.0, 1.0], 0).is_none());
        let expected_sat = if id.starts_with("inhibitor") { 0.0 } else { 100.0 };
        assert_eq!(m.eval(&[1e200], &[0.0, 100.0, 1e-200, 10.0], 0), Some(expected_sat));
    }

    // 2. GaddumSchild: negative AntagConc & NaN rejected + default_bounds present.
    let gs = get_builtin_model("ec50_shift_gaddum_schild").unwrap();
    assert!(gs.eval(&[-7.0], &[0.0, 100.0, -7.0, 1.0, 7.0, 1.0, -5.0], 0).is_none());
    assert!(gs.eval(&[-7.0], &[0.0, 100.0, -7.0, 1.0, f64::NAN, 1.0, 0.0], 0).is_none());
    assert!(gs.eval(&[-7.0], &[0.0, 100.0, -7.0, 1.0, 7.0, 1.0, f64::NAN], 0).is_none());
    let gs_b = gs.default_bounds();
    assert_eq!(gs_b[5].0, Some(1e-4));
    assert_eq!(gs_b[6].0, Some(0.0));

    // 3. EnzymeInhibition (all 4 modes): negative I & non-finite rejected + default_bounds present.
    for (id, valid, neg_i) in [
        ("competitive_inhibition", &[100.0, 5.0, 1.0, 2.0][..], &[100.0, 5.0, 1.0, -2.0][..]),
        ("noncompetitive_inhibition", &[100.0, 5.0, 1.0, 2.0][..], &[100.0, 5.0, 1.0, -2.0][..]),
        ("uncompetitive_inhibition", &[100.0, 5.0, 1.0, 2.0][..], &[100.0, 5.0, 1.0, -2.0][..]),
        ("mixed_model_inhibition", &[100.0, 5.0, 1.0, 2.0, 2.0][..], &[100.0, 5.0, 1.0, 2.0, -2.0][..]),
    ] {
        let m = get_builtin_model(id).unwrap();
        assert!(m.eval(&[2.5], valid, 0).is_some());
        assert!(m.eval(&[2.5], neg_i, 0).is_none(), "{id} accepted negative inhibitor concentration");
        assert!(m.eval(&[5.0], neg_i, 0).is_none(), "{id} accepted negative inhibitor pole");
        let mut with_nan = valid.to_vec();
        with_nan[0] = f64::NAN;
        assert!(m.eval(&[2.5], &with_nan, 0).is_none(), "{id} accepted NaN");
        let b = m.default_bounds();
        assert_eq!(b[0].0, Some(0.0), "{id} Vmax lower bound");
        assert!(b[1].0.is_some_and(|v| v > 0.0), "{id} Km lower bound");
        assert!(b[2].0.is_some_and(|v| v > 0.0), "{id} Ki lower bound");
        assert_eq!(b.last().unwrap().0, Some(0.0), "{id} I lower bound");
    }
}

#[test]
fn aud_03_41_allosteric_modulator_competition_extreme_log_alpha() {
    let model = get_builtin_model("allosteric_modulator_competition").unwrap();
    // Mixed regime: LogAlpha = -320 (pure competitive NAM limit), log_u = 10, log_au = -310.
    // Exact competitive limit: K_app = HotKdNM * (1 + 10^10), Y = Y0 * (HotNM + HotKdNM) / (HotNM + K_app).
    let (y0, log_kb, hot_nm, hot_kd_nm) = (1000.0_f64, -6.0, 2.0, 1.0);
    let y_nam = model.eval(&[4.0], &[y0, log_kb, -320.0, hot_nm, hot_kd_nm], 0).unwrap();
    let expected_nam = y0 * (hot_nm + hot_kd_nm) / (hot_nm + hot_kd_nm * (1.0 + 1e10));
    assert_approx_eq(y_nam, expected_nam, 1e-18, 1e-12, "NAM LogAlpha=-320 log_u=10");

    // Moderate negative log_u with LogAlpha = -320:
    let y_nam_lo = model.eval(&[-8.0], &[y0, log_kb, -320.0, hot_nm, hot_kd_nm], 0).unwrap();
    let expected_nam_lo = y0 * (hot_nm + hot_kd_nm) / (hot_nm + hot_kd_nm * 1.01);
    assert_approx_eq(y_nam_lo, expected_nam_lo, 1e-12, 1e-12, "NAM LogAlpha=-320 log_u=-2");

    // Strong PAM: LogAlpha = +320 with log_u = 10 (log_au = 330 > 300 -> factor -> 0):
    let y_pam = model.eval(&[4.0], &[y0, log_kb, 320.0, hot_nm, hot_kd_nm], 0).unwrap();
    assert_approx_eq(y_pam, y0 * (hot_nm + hot_kd_nm) / hot_nm, 1e-10, 1e-12, "PAM LogAlpha=+320 log_u=10");

    // Both > 100 with LogAlpha = +350 (so -LogAlpha < -300 -> factor = 0):
    let y_pam_both = model.eval(&[120.0], &[y0, 0.0, 350.0, hot_nm, hot_kd_nm], 0).unwrap();
    assert_approx_eq(y_pam_both, y0 * (hot_nm + hot_kd_nm) / hot_nm, 1e-10, 1e-12, "PAM both > 100");
}

fn assert_all_builtin_models_reject_nan_and_emit_finite() {
    // 4. Exhaustive sweep of all 124 pharma/curves equations (strict NaN -> None) + all 151 built-ins (finite):
    for &id in PHARMA_EQUATION_IDS.iter().chain(CURVE_EQUATION_IDS.iter()) {
        let m = get_builtin_model(id).unwrap();
        let n_p = m.param_count();
        let base_p = vec![2.0; n_p];
        if let Some(y) = m.eval(&[f64::NAN], &base_p, 0) {
            panic!("{id} returned Some({y}) for x = NaN");
        }
        for k in 0..n_p {
            let mut p_nan = base_p.clone();
            p_nan[k] = f64::NAN;
            if let Some(y) = m.eval(&[1.0], &p_nan, 0) {
                panic!("{id} returned Some({y}) when param[{k}] = NaN");
            }
        }
    }
    for entry in list_builtin_equations() {
        let m = get_builtin_model(&entry.id).unwrap();
        let n_p = m.param_count();
        let base_p = vec![2.0; n_p];
        for x_bad in [f64::NAN, f64::INFINITY] {
            if let Some(y) = m.eval(&[x_bad], &base_p, 0) {
                assert!(y.is_finite(), "{} returned non-finite Some({y}) for x = {x_bad}", entry.id);
            }
        }
        for k in 0..n_p {
            for p_bad in [f64::NAN, f64::INFINITY] {
                let mut p_vec = base_p.clone();
                p_vec[k] = p_bad;
                if let Some(y) = m.eval(&[1.0], &p_vec, 0) {
                    assert!(y.is_finite(), "{} returned non-finite Some({y}) when param[{k}] = {p_bad}", entry.id);
                }
            }
        }
    }
}

#[test]
fn aud_03_42_derived_scalars_operational_agonism_and_legacy_eval_grad_guards() {
    // 1. Subnormal rates / extreme params never emit non-finite DerivedScalar values:
    let derived_stress: &[(&str, &[f64])] = &[
        ("one_phase_decay", &[100.0, 10.0, 1e-320]),
        ("one_phase_association", &[10.0, 100.0, 1e-320]),
        ("exponential_plateau", &[100.0, 10.0, 1e-320]),
        ("two_phase_association", &[10.0, 100.0, 50.0, 1e-320, 1e-320]),
        ("log_exponential_growth", &[1.0, 1e-320]),
        ("linear_quadratic_percent_surviving", &[1e305, 1e-14]),
        ("gaussian_area", &[1e305, 0.0, 1e-14]),
        ("standard_sine_wave", &[1.0, 1e-320, 0.0]),
        ("boltzmann_sigmoid", &[-1e308, 1e308, 0.0, 1.0]),
    ];
    for &(id, params) in derived_stress {
        let m = get_builtin_model(id).unwrap();
        let scalars = m.derived_transforms(params);
        assert!(scalars.iter().all(|s| s.value.is_finite()), "{id} emitted non-finite derived scalar: {scalars:?}");
    }

    // 2. OperationalAgonism: rejects n <= 0, saturates on overflow, has accurate grad/bounds/derived_transforms:
    let op = get_builtin_model("operational_model_agonist").unwrap();
    assert!(op.eval(&[-6.0], &[5.0, 95.0, -6.0, 0.8, 0.0], 0).is_none());
    assert!(op.eval(&[-6.0], &[5.0, 95.0, -6.0, 0.8, -1.5], 0).is_none());
    let mut g_op = vec![0.0; 5];
    assert!(!op.grad(&[-6.0], &[5.0, 95.0, -6.0, 0.8, -1.5], 0, &mut g_op));
    assert_eq!(op.eval(&[0.0], &[5.0, 95.0, -6.0, 10.0, 100.0], 0), Some(95.0), "OperationalAgonism must saturate to Emax");
    check_grad_vs_fd_general(op.as_ref(), -5.5, &[5.0, 95.0, -6.0, 0.8, 1.3], 0, &[], 1e-6);
    assert_eq!(op.default_bounds()[4].0, Some(1e-4));
    let op_derived = op.derived_transforms(&[5.0, 95.0, -6.0, 0.5, 1.3]);
    assert_approx_eq(op_derived.iter().find(|d| d.name == "KA").unwrap().value, 1e-6, 1e-18, 1e-12, "op derived KA");
    assert_approx_eq(op_derived.iter().find(|d| d.name == "Tau").unwrap().value, 10.0_f64.powf(0.5), 1e-12, 1e-12, "op derived Tau");

    // 3. OneSiteBinding::grad and MichaelisMenten::grad reject kd <= 0 / km <= 0 even when denom > 0:
    let osb = get_builtin_model("one_site_specific_binding").unwrap();
    let mut g2 = vec![0.0; 2];
    assert!(!osb.grad(&[5.0], &[100.0, -1.0], 0, &mut g2));
    assert!(!osb.grad(&[5.0], &[100.0, 0.0], 0, &mut g2));
    assert!(!osb.grad(&[5.0], &[f64::NAN, 2.0], 0, &mut g2));

    let mm = get_builtin_model("michaelis_menten").unwrap();
    assert!(!mm.grad(&[5.0], &[100.0, -1.0], 0, &mut g2));
    assert!(!mm.grad(&[5.0], &[100.0, 0.0], 0, &mut g2));
    assert!(!mm.grad(&[5.0], &[f64::NAN, 2.0], 0, &mut g2));

    let kcat = get_builtin_model("kcat_enzyme_kinetics").unwrap();
    let mut g3 = vec![0.0; 3];
    assert!(!kcat.grad(&[5.0], &[10.0, -1.0, 2.0], 0, &mut g3));
    assert!(!kcat.grad(&[5.0], &[10.0, 0.0, 2.0], 0, &mut g3));

    assert_all_builtin_models_reject_nan_and_emit_finite();

    // Verify full fit + derived scalar sanity on a flat one_phase_decay dataset (K -> 0):
    let flat_xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let flat_ys = [42.0; 6];
    let decay = get_builtin_model("one_phase_decay").unwrap();
    let fit_flat = fit_with(decay.as_ref(), &[series_xy(0, "Flat", &flat_xs, &flat_ys)], &cfg_fast());
    assert!(matches!(fit_flat.status, FitStatus::Converged | FitStatus::Ambiguous));
    assert!(fit_flat.derived_scalars[0].iter().all(|s| s.value.is_finite()));
    let _ = find_derived(&fit_flat, 0, "Span");
    let _ = find_param_ds(&fit_flat, 0, "K");
}
