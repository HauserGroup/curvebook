//! Offline certified NIST StRD reference benchmarks and 106-bit `DoubleDouble`-certified stiff
//! generators (`docs/development.md` §Verified NIST StRD corpus; `docs/implementation/statistics-implementation.md` §1.3).
//!
//! All datasets here run offline with zero network I/O:
//! - Exact NIST StRD `NumAcc1`-`NumAcc4` + binary-exact high-stiffness `BinaryStiff1e8` (`mean / sd = 8e8`),
//! - Exact NIST StRD `Wampler1` & `Wampler2` + `DoubleDouble`-certified orthogonal-residual polynomial
//!   stiffness benchmarks (`WamplerOrtho1`-`WamplerOrtho3`), `NoInt1` & `NoInt2`, and degree-10 Vandermonde,
//! - Simon-Lesage-style (`SmLs`) One-Way ANOVA stiffness benchmarks across `Lower`, `Average`, and `Higher`
//!   shift magnitudes certified in 106-bit `DoubleDouble` arithmetic.

use crate::dd::DoubleDouble;
use crate::nist_strd::parser::{AnovaDataset, LinearRegressionDataset, NistDifficulty, UnivariateDataset};

// ============================================================================
// 1. Univariate Stiffness Benchmarks (NumAcc1 - NumAcc4 + BinaryStiff1e8)
// ============================================================================

/// NIST StRD `NumAcc1` (Lower difficulty): 3 consecutive integers around `10^7`.
#[must_use]
pub fn numacc1() -> UnivariateDataset {
    UnivariateDataset {
        name: "NumAcc1".to_owned(),
        difficulty: NistDifficulty::Lower,
        certified_mean: 10_000_002.0,
        certified_sd: 1.0,
        certified_r1: 0.0,
        values: vec![10_000_001.0, 10_000_002.0, 10_000_003.0],
    }
}

fn build_alternating_numacc(name: &str, difficulty: NistDifficulty, center: f64, delta: f64) -> UnivariateDataset {
    let mut values = Vec::with_capacity(1001);
    values.push(center);
    for _ in 0..500 {
        values.push(center - delta);
        values.push(center + delta);
    }
    UnivariateDataset { name: name.to_owned(), difficulty, certified_mean: center, certified_sd: delta, certified_r1: -0.999, values }
}

/// NIST StRD `NumAcc2` (Average difficulty): 1001 observations centered at `1.2`, `sd = 0.1`.
#[must_use]
pub fn numacc2() -> UnivariateDataset {
    build_alternating_numacc("NumAcc2", NistDifficulty::Average, 1.2, 0.1)
}

/// NIST StRD `NumAcc3` (Average difficulty): 1001 observations centered at `1_000_000.2`, `sd = 0.1`.
#[must_use]
pub fn numacc3() -> UnivariateDataset {
    build_alternating_numacc("NumAcc3", NistDifficulty::Average, 1_000_000.2, 0.1)
}

/// NIST StRD `NumAcc4` (Higher difficulty): 1001 observations centered at `10_000_000.2`, `sd = 0.1`.
#[must_use]
pub fn numacc4() -> UnivariateDataset {
    build_alternating_numacc("NumAcc4", NistDifficulty::Higher, 10_000_000.2, 0.1)
}

/// Binary-exact high-stiffness univariate benchmark (`mean / sd = 8 * 10^8 > 10^8`) whose observations
/// have zero decimal-to-binary64 rounding so two-pass Neumaier retains all 15+ digits.
#[must_use]
pub fn binary_stiff_1e8() -> UnivariateDataset {
    build_alternating_numacc("BinaryStiff1e8", NistDifficulty::Higher, 100_000_000.25, 0.125)
}

/// All offline univariate stiffness benchmarks.
#[must_use]
pub fn all_offline_univariate_benchmarks() -> Vec<UnivariateDataset> {
    vec![numacc1(), numacc2(), numacc3(), numacc4(), binary_stiff_1e8()]
}

// ============================================================================
// 2. Linear Least-Squares Stiffness Benchmarks (Wampler, NoInt, Vandermonde)
// ============================================================================

/// NIST StRD `Wampler1` (Wampler 1970 `Y1`): exact 5th-degree polynomial `1 + x + x^2 + x^3 + x^4 + x^5`
/// on `x = 0..=20` (`kappa(X) ~ 1.9e6`, `kappa(X^T X) ~ 3.6e12`).
#[must_use]
pub fn wampler1() -> LinearRegressionDataset {
    build_exact_polynomial_dataset("Wampler1", NistDifficulty::Lower, &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0])
}

/// NIST StRD `Wampler2` (Wampler 1970 `Y2`): exact 5th-degree polynomial `sum_{k=0}^5 (x / 10)^k`
/// on `x = 0..=20` with coefficients spanning 5 orders of magnitude.
#[must_use]
pub fn wampler2() -> LinearRegressionDataset {
    build_exact_polynomial_dataset("Wampler2", NistDifficulty::Lower, &[1.0, 0.1, 0.01, 0.001, 0.0001, 0.000_01])
}

fn build_exact_polynomial_dataset(name: &str, difficulty: NistDifficulty, betas: &[f64]) -> LinearRegressionDataset {
    let p = betas.len();
    let n = 21_usize;
    let mut y = Vec::with_capacity(n);
    let mut design_rows = Vec::with_capacity(n);
    let mut y_dd = Vec::with_capacity(n);

    for i in 0..n {
        let x = i as f64;
        let mut row = Vec::with_capacity(p);
        let mut acc = DoubleDouble::ZERO;
        for (k, &bk) in betas.iter().enumerate() {
            let xk = x.powi(k as i32);
            row.push(xk);
            acc += DoubleDouble::from_f64(bk) * DoubleDouble::from_f64(xk);
        }
        y.push(acc.to_f64());
        y_dd.push(acc);
        design_rows.push(row);
    }

    let mean_y = y_dd.iter().copied().fold(DoubleDouble::ZERO, |a, b| a + b) / DoubleDouble::from_f64(n as f64);
    let tss = y_dd.iter().copied().fold(DoubleDouble::ZERO, |a, yi| a + (yi - mean_y).sqr());

    LinearRegressionDataset {
        name: name.to_owned(),
        difficulty,
        has_intercept: true,
        certified_betas: betas.to_vec(),
        certified_se_betas: vec![0.0; p],
        certified_residual_sd: 0.0,
        certified_r_squared: 1.0,
        certified_df_reg: p - 1,
        certified_df_res: n - p,
        certified_ss_reg: tss.to_f64(),
        certified_ss_res: 0.0,
        certified_f: Some(f64::INFINITY),
        y,
        design_rows,
    }
}

/// Wampler-class stiff 5th-degree polynomial regression on `x = 0..=20` with an exact orthogonal
/// residual vector scaled to `target_residual_sd` and certified to 106-bit `DoubleDouble` precision
/// (`Wampler3`/`Wampler4`/`Wampler5` condition-number regime).
#[must_use]
pub fn wampler_orthogonal_stiff(name: &str, difficulty: NistDifficulty, target_residual_sd: f64) -> LinearRegressionDataset {
    let n = 21_usize;
    let p = 6_usize;

    // Build 106-bit Vandermonde columns and orthonormalize via modified Gram-Schmidt in DoubleDouble!
    let mut cols: Vec<Vec<DoubleDouble>> = (0..p).map(|k| (0..n).map(|i| DoubleDouble::from_f64((i as f64).powi(k as i32))).collect()).collect();
    let mut q_cols: Vec<Vec<DoubleDouble>> = Vec::with_capacity(p);
    for col in &mut cols {
        for q in &q_cols {
            let proj = dd_dot(q, col);
            for (c_val, &q_val) in col.iter_mut().zip(q.iter()) {
                *c_val -= proj * q_val;
            }
        }
        let norm = dd_dot(col, col).sqrt();
        q_cols.push(col.iter().map(|&v| v / norm).collect());
    }

    // Project alternating perturbation (-1)^i onto the orthogonal complement of col(X):
    let mut delta: Vec<DoubleDouble> = (0..n).map(|i| if i % 2 == 0 { DoubleDouble::ONE } else { -DoubleDouble::ONE }).collect();
    for _ in 0..2 {
        for q in &q_cols {
            let proj = dd_dot(q, &delta);
            for (d_val, &q_val) in delta.iter_mut().zip(q.iter()) {
                *d_val -= proj * q_val;
            }
        }
    }
    let df_res = n - p;
    let target_rss = DoubleDouble::from_f64(df_res as f64) * DoubleDouble::from_f64(target_residual_sd).sqr();
    let raw_rss = dd_dot(&delta, &delta);
    let scale = (target_rss / raw_rss).sqrt();

    let mut design_rows = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for (i, &di) in delta.iter().enumerate().take(n) {
        let x = i as f64;
        let row: Vec<f64> = (0..p).map(|k| x.powi(k as i32)).collect();
        let mut poly_val = DoubleDouble::ZERO;
        for &v in &row {
            poly_val += DoubleDouble::from_f64(v);
        }
        let yi = (poly_val + scale * di).to_f64();
        y.push(yi);
        design_rows.push(row);
    }

    // Re-certify the exact rounded `f64` data vector `y` in 106-bit DoubleDouble Householder QR!
    certify_linear_dataset_dd(name, difficulty, true, &design_rows, &y)
}

/// NIST StRD `NoInt1` exact 11-row no-intercept regression (`y = 130..=140`, `x = 60..=70`).
#[must_use]
pub fn noint1_exact() -> LinearRegressionDataset {
    let y: Vec<f64> = (130..=140).map(f64::from).collect();
    let design_rows: Vec<Vec<f64>> = (60..=70).map(|x| vec![f64::from(x)]).collect();
    certify_linear_dataset_dd("NoInt1", NistDifficulty::Average, false, &design_rows, &y)
}

/// NIST StRD `NoInt2` exact 3-row no-intercept regression (`y = [3, 5, 6]`, `x = [4, 5, 6]`).
#[must_use]
pub fn noint2_exact() -> LinearRegressionDataset {
    let y = vec![3.0, 5.0, 6.0];
    let design_rows = vec![vec![4.0], vec![5.0], vec![6.0]];
    certify_linear_dataset_dd("NoInt2", NistDifficulty::Higher, false, &design_rows, &y)
}

/// High-degree Filip-class polynomial Vandermonde benchmark (degree 9, 10 parameters on shifted `x`)
/// certified in 106-bit `DoubleDouble` precision where naive normal equations `X^T X` fail completely.
#[must_use]
pub fn filip_scale_vandermonde_benchmark() -> LinearRegressionDataset {
    let n = 41_usize;
    let p = 10_usize;
    let mut design_rows = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for i in 0..n {
        // Filip-regime predictor x in [-8.5, -3.1] gives kappa(X) ~ 3e7 where X^T X (kappa ~ 1e15) collapses!
        let x = -8.5 + 0.135 * (i as f64);
        let row: Vec<f64> = (0..p).map(|k| x.powi(k as i32)).collect();
        // Smooth target evaluated around centered u = x + 5.8 so exact degree-9 fit is well-defined:
        let u = x + 5.8;
        let signal = 2.5 - 0.75 * u + 0.25 * u.powi(2) - 0.1 * u.powi(3) + 0.05 * u.powi(4) - 0.02 * u.powi(5) + 0.01 * u.powi(6) - 0.004 * u.powi(7)
            + 0.002 * u.powi(8)
            + 0.001 * u.powi(9);
        let noise = 1e-4 * ((i as f64) * 1.7).sin();
        y.push(signal + noise);
        design_rows.push(row);
    }
    certify_linear_dataset_dd("FilipScaleVandermonde", NistDifficulty::Higher, true, &design_rows, &y)
}

/// All offline linear least-squares reference benchmarks.
#[must_use]
pub fn all_offline_linear_regression_benchmarks() -> Vec<LinearRegressionDataset> {
    vec![
        wampler1(),
        wampler2(),
        wampler_orthogonal_stiff("WamplerOrtho1", NistDifficulty::Average, 1.0),
        wampler_orthogonal_stiff("WamplerOrtho2", NistDifficulty::Higher, 100.0),
        wampler_orthogonal_stiff("WamplerOrtho3", NistDifficulty::Higher, 10_000.0),
        noint1_exact(),
        noint2_exact(),
        filip_scale_vandermonde_benchmark(),
    ]
}

fn dd_dot(a: &[DoubleDouble], b: &[DoubleDouble]) -> DoubleDouble {
    let mut acc = DoubleDouble::ZERO;
    for (&x, &y) in a.iter().zip(b.iter()) {
        acc += x * y;
    }
    acc
}

/// Certify any full-rank `n x p` (`n > p`) linear regression problem in 106-bit `DoubleDouble`
/// Householder QR arithmetic, yielding 30-digit oracle `betas`, `se_betas`, `rss`, `r_squared`, and `F`.
fn certify_linear_dataset_dd(
    name: &str,
    difficulty: NistDifficulty,
    has_intercept: bool,
    design_rows: &[Vec<f64>],
    y: &[f64],
) -> LinearRegressionDataset {
    let n = design_rows.len();
    let p = design_rows.first().map_or(0, Vec::len);
    let mut r = vec![vec![DoubleDouble::ZERO; p]; n];
    for (i, row) in design_rows.iter().enumerate() {
        for (j, &v) in row.iter().enumerate() {
            r[i][j] = DoubleDouble::from_f64(v);
        }
    }
    let mut qty: Vec<DoubleDouble> = y.iter().map(|&v| DoubleDouble::from_f64(v)).collect();

    for k in 0..p {
        let mut norm_sq = DoubleDouble::ZERO;
        for row in r.iter().skip(k) {
            norm_sq += row[k].sqr();
        }
        let norm = norm_sq.sqrt();
        let alpha = if r[k][k].hi >= 0.0 { -norm } else { norm };
        let v0 = r[k][k] - alpha;
        let mut v_norm_sq = v0.sqr();
        for row in r.iter().skip(k + 1) {
            v_norm_sq += row[k].sqr();
        }
        let two_over_v2 = DoubleDouble::from_f64(2.0) / v_norm_sq;

        // Apply reflector to trailing columns k..p and to qty:
        r[k][k] = alpha;
        for col in (k + 1)..p {
            let mut dot = v0 * r[k][col];
            for row in r.iter().take(n).skip(k + 1) {
                dot += row[k] * row[col];
            }
            let tau_dot = two_over_v2 * dot;
            r[k][col] -= tau_dot * v0;
            for row in r.iter_mut().take(n).skip(k + 1) {
                let vk = row[k];
                row[col] -= tau_dot * vk;
            }
        }
        let mut y_dot = v0 * qty[k];
        for row_i in (k + 1)..n {
            y_dot += r[row_i][k] * qty[row_i];
        }
        let tau_y = two_over_v2 * y_dot;
        qty[k] -= tau_y * v0;
        for row_i in (k + 1)..n {
            qty[row_i] -= tau_y * r[row_i][k];
        }
    }

    let betas_dd = back_substitute_dd(&r, &qty[..p]);
    let mut rss = DoubleDouble::ZERO;
    for &rem in &qty[p..] {
        rss += rem.sqr();
    }
    let df_res = n - p;
    let df_reg = if has_intercept { p - 1 } else { p };
    let sigma2 = rss / DoubleDouble::from_f64(df_res as f64);
    let rsd = sigma2.sqrt();

    let r_inv = invert_upper_dd(&r, p);
    let mut se_betas = Vec::with_capacity(p);
    for row in &r_inv {
        let mut row_sq = DoubleDouble::ZERO;
        for &v in row {
            row_sq += v.sqr();
        }
        se_betas.push((sigma2 * row_sq).sqrt().to_f64());
    }

    let ss_total = total_ss_dd(y, has_intercept);
    let ss_reg = ss_total - rss;
    let r_squared = (DoubleDouble::ONE - rss / ss_total).to_f64();
    let f_stat = if df_reg > 0 && rss.hi > 0.0 { Some(((ss_reg / DoubleDouble::from_f64(df_reg as f64)) / sigma2).to_f64()) } else { None };

    LinearRegressionDataset {
        name: name.to_owned(),
        difficulty,
        has_intercept,
        certified_betas: betas_dd.into_iter().map(DoubleDouble::to_f64).collect(),
        certified_se_betas: se_betas,
        certified_residual_sd: rsd.to_f64(),
        certified_r_squared: r_squared,
        certified_df_reg: df_reg,
        certified_df_res: df_res,
        certified_ss_reg: ss_reg.to_f64(),
        certified_ss_res: rss.to_f64(),
        certified_f: f_stat,
        y: y.to_vec(),
        design_rows: design_rows.to_vec(),
    }
}

fn back_substitute_dd(r: &[Vec<DoubleDouble>], rhs: &[DoubleDouble]) -> Vec<DoubleDouble> {
    let p = rhs.len();
    let mut x = vec![DoubleDouble::ZERO; p];
    for i in (0..p).rev() {
        let mut acc = rhs[i];
        for j in (i + 1)..p {
            acc -= r[i][j] * x[j];
        }
        x[i] = acc / r[i][i];
    }
    x
}

fn invert_upper_dd(r: &[Vec<DoubleDouble>], p: usize) -> Vec<Vec<DoubleDouble>> {
    let mut inv = vec![vec![DoubleDouble::ZERO; p]; p];
    for col in 0..p {
        let mut e = vec![DoubleDouble::ZERO; p];
        e[col] = DoubleDouble::ONE;
        let sol = back_substitute_dd(r, &e);
        for row in 0..p {
            inv[row][col] = sol[row];
        }
    }
    inv
}

fn total_ss_dd(y: &[f64], has_intercept: bool) -> DoubleDouble {
    if !has_intercept {
        return y.iter().fold(DoubleDouble::ZERO, |acc, &v| acc + DoubleDouble::from_f64(v).sqr());
    }
    let n = DoubleDouble::from_f64(y.len() as f64);
    let sum = y.iter().fold(DoubleDouble::ZERO, |acc, &v| acc + DoubleDouble::from_f64(v));
    let mean = sum / n;
    y.iter().fold(DoubleDouble::ZERO, |acc, &v| acc + (DoubleDouble::from_f64(v) - mean).sqr())
}

// ============================================================================
// 3. Simon-Lesage-Style Stiff One-Way ANOVA Benchmarks (SmLs Tiered Suite)
// ============================================================================

/// Build a Simon-Lesage-style (`SmLs01`-`SmLs09` pattern) stiff 9-group One-Way ANOVA dataset
/// shifted by `base_offset` (`1e2` Lower, `1e5` Average, `1e7` / `1e8` Higher) using binary-exact
/// offsets (`0.125`, `0.25`) certified in 106-bit `DoubleDouble` arithmetic.
#[must_use]
pub fn smls_stiff_anova(name: &str, difficulty: NistDifficulty, base_offset: f64) -> AnovaDataset {
    let groups = 9_usize;
    let reps_per_group = 11_usize;
    let mut observations = Vec::with_capacity(groups * reps_per_group);
    let mut group_sums = vec![DoubleDouble::ZERO; groups];

    for g in 1..=groups {
        let group_center = base_offset + 0.125 * (g as f64);
        // 1 center + 5 pairs (-0.25, +0.25) -> exact within-group mean = group_center!
        observations.push((g, group_center));
        group_sums[g - 1] += DoubleDouble::from_f64(group_center);
        for _ in 0..5 {
            let lo = group_center - 0.25;
            let hi = group_center + 0.25;
            observations.push((g, lo));
            observations.push((g, hi));
            group_sums[g - 1] += DoubleDouble::from_f64(lo) + DoubleDouble::from_f64(hi);
        }
    }

    let n_total = observations.len();
    let rep_dd = DoubleDouble::from_f64(reps_per_group as f64);
    let grand_mean = group_sums.iter().copied().fold(DoubleDouble::ZERO, |a, b| a + b) / DoubleDouble::from_f64(n_total as f64);
    let group_means: Vec<DoubleDouble> = group_sums.iter().map(|&s| s / rep_dd).collect();

    let mut ss_between = DoubleDouble::ZERO;
    for &gm in &group_means {
        ss_between += rep_dd * (gm - grand_mean).sqr();
    }
    let mut ss_within = DoubleDouble::ZERO;
    for &(g, val) in &observations {
        ss_within += (DoubleDouble::from_f64(val) - group_means[g - 1]).sqr();
    }

    let df_between = groups - 1;
    let df_within = n_total - groups;
    let ms_between = ss_between / DoubleDouble::from_f64(df_between as f64);
    let ms_within = ss_within / DoubleDouble::from_f64(df_within as f64);
    let f_stat = ms_between / ms_within;
    let r_squared = ss_between / (ss_between + ss_within);

    AnovaDataset {
        name: name.to_owned(),
        difficulty,
        certified_df_between: df_between,
        certified_df_within: df_within,
        certified_ss_between: ss_between.to_f64(),
        certified_ss_within: ss_within.to_f64(),
        certified_f: f_stat.to_f64(),
        certified_r_squared: r_squared.to_f64(),
        certified_residual_sd: ms_within.sqrt().to_f64(),
        observations,
    }
}

/// All offline One-Way ANOVA stiffness benchmarks.
#[must_use]
pub fn all_offline_anova_benchmarks() -> Vec<AnovaDataset> {
    vec![
        smls_stiff_anova("SmLsStiff1e2", NistDifficulty::Lower, 1e2),
        smls_stiff_anova("SmLsStiff1e5", NistDifficulty::Average, 1e5),
        smls_stiff_anova("SmLsStiff1e7", NistDifficulty::Higher, 1e7),
        smls_stiff_anova("SmLsStiff1e8", NistDifficulty::Higher, 1e8),
    ]
}
