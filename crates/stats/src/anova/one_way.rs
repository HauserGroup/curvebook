//! Ordinary, unequal-variance (Welch and Brown–Forsythe), summary-statistic, and repeated-measures
//! one-way ANOVA, with Mauchly sphericity and Greenhouse–Geisser / Huynh–Feldt corrections.
//!
//! References:
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - NIST Statistical Reference Datasets (StRD), Analysis of Variance:
//!   <https://www.itl.nist.gov/div898/strd/anova/anova.html>
//! - Welch, B. L. (1951). On the comparison of several mean values: an alternative approach.
//!   *Biometrika* 38(3/4):330–336.
//! - Brown, M. B., & Forsythe, A. B. (1974b). The small sample behavior of some statistics which
//!   test the equality of several means. *Technometrics* 16(1):129–132.
//! - Mauchly, J. W. (1940). Significance test for sphericity of a normal *n*-variate distribution.
//!   *Annals of Mathematical Statistics* 11(2):204–209. doi:10.1214/aoms/1177731915
//! - Greenhouse, S. W., & Geisser, S. (1959). On methods in the analysis of profile data.
//!   *Psychometrika* 24(2):95–112. doi:10.1007/BF02289823
//! - Huynh, H., & Feldt, L. S. (1976). Estimation of the Box correction for degrees of freedom from
//!   sample data in randomized block and split-plot designs. *Journal of Educational Statistics*
//!   1(1):69–82. doi:10.3102/10769986001001069

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{chi2_sf, f_sf};
use curvebook_math::linalg::{DenseMatrix, sym_eigen};
use curvebook_math::sum::{NeumaierSum, SummaryMoments};

use crate::anova::effect_sizes::{clamp_ss_roundoff, compute_anova_effect_sizes, f_ratio_p, variance_homogeneity_tests};
use crate::anova::reml::symmetrize_dense;
use crate::anova::results::{OneWayAnovaResult, RmOneWayAnovaResult, SphericityResult, UnequalVarianceAnovaResult, VarianceHomogeneityResult};
use crate::common::validate_finite_slice;
use crate::error::StatsError;
use crate::multiplicity::GroupComparisonInput;

/// Ordinary one-way ANOVA using two-pass `SummaryMoments` and `NeumaierSum` shifted by the grand
/// mean so stiff NIST StRD series (`SmLs01..09`, `SiRstv`, `AtmWtAg`) retain full precision.
///
/// Fixed-effects one-way ANOVA partition `SS_total = SS_between + SS_within` with
/// `F = MS_between / MS_within` on `(k - 1, N - k)` df (Zar, 2010, ch. 10), verified against the
/// NIST StRD ANOVA certified values.
pub fn ordinary_one_way_anova(groups: &[(&str, &[f64])], confidence: ConfidenceLevel) -> Result<OneWayAnovaResult, StatsError> {
    let slices: Vec<&[f64]> = groups.iter().map(|(_, s)| *s).collect();
    let labels: Vec<String> = groups.iter().map(|(l, _)| (*l).to_owned()).collect();
    let mut res = ordinary_one_way_anova_slices(&slices, confidence)?;
    res.group_labels = labels;
    res.variance_tests = variance_homogeneity_tests(&slices)?;
    Ok(res)
}

pub(in crate::anova) fn ordinary_one_way_anova_slices(groups: &[&[f64]], confidence: ConfidenceLevel) -> Result<OneWayAnovaResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: groups.len() });
    }
    let mut group_n = Vec::with_capacity(groups.len());
    let mut group_means = Vec::with_capacity(groups.len());
    let mut group_sds = Vec::with_capacity(groups.len());
    let mut ss_within_acc = NeumaierSum::new();
    let mut grand_sum_acc = NeumaierSum::new();
    let mut total_n = 0_usize;

    for &g in groups {
        validate_finite_slice(g)?;
        let m = SummaryMoments::from_slice(g)?;
        group_n.push(g.len());
        group_means.push(m.mean);
        group_sds.push(m.sample_sd().unwrap_or(0.0));
        ss_within_acc.add(m.css);
        grand_sum_acc.add((g.len() as f64) * m.mean);
        total_n += g.len();
    }
    if total_n <= groups.len() {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA residual df", required: groups.len() + 1, actual: total_n });
    }
    let total_nf = total_n as f64;
    let grand_mean_approx = grand_sum_acc.total() / total_nf;

    // Two-pass refinement on grand mean to preserve precision on large-offset datasets (SmLs07..09).
    let mut shift_corr = NeumaierSum::new();
    for (&n_i, &m_i) in group_n.iter().zip(group_means.iter()) {
        shift_corr.add((n_i as f64) * (m_i - grand_mean_approx));
    }
    let grand_mean = grand_mean_approx + shift_corr.total() / total_nf;

    let mut ss_between_acc = NeumaierSum::new();
    for (&n_i, &m_i) in group_n.iter().zip(group_means.iter()) {
        let diff = m_i - grand_mean;
        ss_between_acc.add((n_i as f64) * diff * diff);
    }

    let ss_between = ss_between_acc.total().max(0.0);
    let ss_within = ss_within_acc.total().max(0.0);
    let ss_total = ss_between + ss_within;

    let df_between = (groups.len() - 1) as f64;
    let df_within = (total_n - groups.len()) as f64;
    let df_total = (total_n - 1) as f64;

    let ms_between = ss_between / df_between;
    let ms_within = ss_within / df_within;
    let (f_stat, p_val) = if ms_within > 0.0 {
        let f = ms_between / ms_within;
        (f, f_sf(f, df_between, df_within)?)
    } else if ms_between > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };

    let r_squared = if ss_total > 0.0 { ss_between / ss_total } else { 0.0 };
    let effect_sizes = compute_anova_effect_sizes(ss_between, ss_within, ss_total, df_between, df_within, confidence);

    Ok(OneWayAnovaResult {
        group_labels: (0..groups.len()).map(|i| format!("Group {}", i + 1)).collect(),
        group_n,
        group_means,
        group_sds,
        total_n,
        ss_between,
        ss_within,
        ss_total,
        df_between,
        df_within,
        df_total,
        ms_between,
        ms_within,
        f_statistic: f_stat,
        p_value: p_val,
        r_squared,
        variance_tests: VarianceHomogeneityResult {
            bartlett_chi2: f64::NAN,
            bartlett_df: df_between,
            bartlett_p: f64::NAN,
            brown_forsythe_f: f64::NAN,
            brown_forsythe_df1: df_between,
            brown_forsythe_df2: df_within,
            brown_forsythe_p: f64::NAN,
            available: false,
        },
        effect_sizes,
    })
}

/// One-way ANOVA from pre-summarized `(label, n, mean, sd)` records.
///
/// Same partition as [`ordinary_one_way_anova`] with `SS_within = sum (n_i - 1) s_i^2` and
/// `SS_between = sum n_i (mean_i - grand_mean)^2` (Zar, 2010, ch. 10).
pub fn ordinary_one_way_from_summary(groups: &[GroupComparisonInput], confidence: ConfidenceLevel) -> Result<OneWayAnovaResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: groups.len() });
    }
    let mut total_n_f = 0.0_f64;
    let mut grand_sum = NeumaierSum::new();
    let mut ss_within = NeumaierSum::new();
    for g in groups {
        if g.n < 1.0 || !g.mean.is_finite() || !g.sd.is_finite() || g.sd < 0.0 {
            return Err(StatsError::InvalidParameter { name: "group_summary", value: g.n });
        }
        total_n_f += g.n;
        grand_sum.add(g.n * g.mean);
        if g.n > 1.0 {
            ss_within.add((g.n - 1.0) * g.sd * g.sd);
        }
    }
    let grand_mean = grand_sum.total() / total_n_f;
    let mut ss_between = NeumaierSum::new();
    for g in groups {
        let d = g.mean - grand_mean;
        ss_between.add(g.n * d * d);
    }
    let ssb = ss_between.total().max(0.0);
    let ssw = ss_within.total().max(0.0);
    let sst = ssb + ssw;
    let dfb = (groups.len() - 1) as f64;
    let dfw = total_n_f - (groups.len() as f64);
    if dfw <= 0.0 {
        return Err(StatsError::InsufficientSampleSize {
            context: "ANOVA summary residual df",
            required: groups.len() + 1,
            actual: total_n_f as usize,
        });
    }
    let msb = ssb / dfb;
    let msw = ssw / dfw;
    let (f_stat, p_val) = if msw > 0.0 {
        let f = msb / msw;
        (f, f_sf(f, dfb, dfw)?)
    } else if msb > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };
    let effect_sizes = compute_anova_effect_sizes(ssb, ssw, sst, dfb, dfw, confidence);

    Ok(OneWayAnovaResult {
        group_labels: groups.iter().map(|g| g.label.clone()).collect(),
        group_n: groups.iter().map(|g| g.n.round() as usize).collect(),
        group_means: groups.iter().map(|g| g.mean).collect(),
        group_sds: groups.iter().map(|g| g.sd).collect(),
        total_n: total_n_f.round() as usize,
        ss_between: ssb,
        ss_within: ssw,
        ss_total: sst,
        df_between: dfb,
        df_within: dfw,
        df_total: total_n_f - 1.0,
        ms_between: msb,
        ms_within: msw,
        f_statistic: f_stat,
        p_value: p_val,
        r_squared: if sst > 0.0 { ssb / sst } else { 0.0 },
        variance_tests: VarianceHomogeneityResult {
            bartlett_chi2: f64::NAN,
            bartlett_df: dfb,
            bartlett_p: f64::NAN,
            brown_forsythe_f: f64::NAN,
            brown_forsythe_df1: dfb,
            brown_forsythe_df2: dfw,
            brown_forsythe_p: f64::NAN,
            available: false,
        },
        effect_sizes,
    })
}

/// Welch's and Brown–Forsythe's unequal-variance one-way ANOVA from group summaries.
///
/// Welch (1951) `W` with weights `w_i = n_i / s_i^2`, denominator
/// `1 + 2 (k - 2) / (k^2 - 1) * sum (1 - w_i / sum w)^2 / (n_i - 1)`, and
/// `df2 = (k^2 - 1) / (3 sum (1 - w_i / sum w)^2 / (n_i - 1))`. Brown & Forsythe (1974b)
/// `F* = sum n_i (mean_i - grand_mean)^2 / sum (1 - n_i / N) s_i^2` on `(k - 1, f)` df, with `f`
/// from the Satterthwaite (1946) approximation.
pub fn unequal_variance_one_way_anova(groups: &[GroupComparisonInput]) -> Result<UnequalVarianceAnovaResult, StatsError> {
    let k = groups.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: k });
    }
    let kf = k as f64;
    let mut weights = Vec::with_capacity(k);
    let mut w_sum = 0.0_f64;
    let mut w_mean_sum = 0.0_f64;
    let mut total_n = 0.0_f64;
    let mut ssb_num = 0.0_f64;
    let mut bf_denom = 0.0_f64;
    let mut bf_satt_denom = 0.0_f64;

    for g in groups {
        if g.n < 2.0 || !g.sd.is_finite() || g.sd <= 0.0 {
            return Err(StatsError::InvalidParameter { name: "group_sd", value: g.sd });
        }
        let v = g.sd * g.sd;
        let w = g.n / v;
        weights.push(w);
        w_sum += w;
        w_mean_sum += w * g.mean;
        total_n += g.n;
    }
    let grand_mean: f64 = groups.iter().map(|g| g.n * g.mean).sum::<f64>() / total_n;
    let x_bar_w = w_mean_sum / w_sum;

    let mut welch_num = 0.0_f64;
    let mut lambda_sum = 0.0_f64;
    for (g, &w) in groups.iter().zip(weights.iter()) {
        let dm_w = g.mean - x_bar_w;
        welch_num += w * dm_w * dm_w;
        let frac = 1.0 - w / w_sum;
        lambda_sum += (frac * frac) / (g.n - 1.0);

        let dm = g.mean - grand_mean;
        ssb_num += g.n * dm * dm;
        let v = g.sd * g.sd;
        let term = (1.0 - g.n / total_n) * v;
        bf_denom += term;
        bf_satt_denom += (term * term) / (g.n - 1.0);
    }

    let welch_df1 = kf - 1.0;
    let lambda_factor = 3.0 * lambda_sum / (kf * kf - 1.0);
    let welch_w = (welch_num / welch_df1) / (1.0 + (2.0 * (kf - 2.0) / 3.0) * lambda_factor);
    let welch_df2 = if lambda_sum > 0.0 { (kf * kf - 1.0) / (3.0 * lambda_sum) } else { 1.0 };
    let welch_p = f_sf(welch_w, welch_df1, welch_df2)?;

    let bf_f_star = if bf_denom > 0.0 { ssb_num / bf_denom } else { 0.0 };
    let bf_df2 = if bf_satt_denom > 0.0 { (bf_denom * bf_denom) / bf_satt_denom } else { 1.0 };
    let bf_p = f_sf(bf_f_star, welch_df1, bf_df2)?;

    Ok(UnequalVarianceAnovaResult {
        welch_w,
        welch_df1,
        welch_df2,
        welch_p,
        brown_forsythe_f_star: bf_f_star,
        brown_forsythe_df1: welch_df1,
        brown_forsythe_df2: bf_df2,
        brown_forsythe_p: bf_p,
    })
}

/// Repeated-measures one-way ANOVA on complete subject blocks (`n` rows × `k` conditions), with
/// Mauchly's sphericity test, Greenhouse–Geisser `epsilon_hat`, and Huynh–Feldt `epsilon_tilde`.
///
/// Subjects × treatments (randomized-block) partition with `F = MS_treatment / MS_residual` on
/// `(k - 1, (n - 1)(k - 1))` df (Zar, 2010, ch. 12; Maxwell & Delaney, 2004, ch. 11). Corrected
/// p-values multiply both df by Greenhouse & Geisser (1959) or Huynh & Feldt (1976) epsilon.
pub fn rm_one_way_anova(blocks: &[Vec<f64>]) -> Result<RmOneWayAnovaResult, StatsError> {
    let n = blocks.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "repeated-measures subjects", required: 2, actual: n });
    }
    let k = blocks[0].len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANOVA groups", required: 2, actual: k });
    }
    for row in blocks {
        if row.len() != k {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
        }
        validate_finite_slice(row)?;
    }

    let nf = n as f64;
    let kf = k as f64;
    let mut grand_sum = NeumaierSum::new();
    let mut row_means = Vec::with_capacity(n);
    let mut col_sums = vec![NeumaierSum::new(); k];
    let mut data_max_abs = 0.0_f64;

    for row in blocks {
        let mut rsum = NeumaierSum::new();
        for (c, &v) in row.iter().enumerate() {
            data_max_abs = data_max_abs.max(v.abs());
            rsum.add(v);
            col_sums[c].add(v);
            grand_sum.add(v);
        }
        row_means.push(rsum.total() / kf);
    }
    let grand_mean = grand_sum.total() / (nf * kf);
    let col_means: Vec<f64> = col_sums.iter().map(|s| s.total() / nf).collect();

    let mut ss_treat = NeumaierSum::new();
    for &cm in &col_means {
        let d = cm - grand_mean;
        ss_treat.add(nf * d * d);
    }
    let mut ss_subj = NeumaierSum::new();
    for &rm in &row_means {
        let d = rm - grand_mean;
        ss_subj.add(kf * d * d);
    }
    let mut ss_resid = NeumaierSum::new();
    for (r, row) in blocks.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            let res = v - row_means[r] - col_means[c] + grand_mean;
            ss_resid.add(res * res);
        }
    }

    let obs_count = n * k;
    let sst = clamp_ss_roundoff(ss_treat.total(), data_max_abs, obs_count);
    let sss = clamp_ss_roundoff(ss_subj.total(), data_max_abs, obs_count);
    let ssr = clamp_ss_roundoff(ss_resid.total(), data_max_abs, obs_count);
    let ss_total = sst + sss + ssr;

    let dft = kf - 1.0;
    let dfs = nf - 1.0;
    let dfr = dft * dfs;
    let mst = sst / dft;
    let mss = sss / dfs;
    let msr = ssr / dfr;

    let (f_treat, p_uncorr) = f_ratio_p(mst, msr, dft, dfr)?;
    let (f_subj, p_subj) = f_ratio_p(mss, msr, dfs, dfr)?;

    let sphericity = compute_sphericity(blocks)?;
    let gg_df1 = (dft * sphericity.greenhouse_geisser_eps).max(1.0);
    let gg_df2 = (dfr * sphericity.greenhouse_geisser_eps).max(1.0);
    let hf_df1 = (dft * sphericity.huynh_feldt_eps).max(1.0);
    let hf_df2 = (dfr * sphericity.huynh_feldt_eps).max(1.0);
    let p_gg = f_sf(f_treat, gg_df1, gg_df2)?;
    let p_hf = f_sf(f_treat, hf_df1, hf_df2)?;

    let partial_eta2 = if sst + ssr > 0.0 { sst / (sst + ssr) } else { 0.0 };

    Ok(RmOneWayAnovaResult {
        n_subjects: n,
        k_conditions: k,
        ss_treatment: sst,
        ss_subjects: sss,
        ss_residual: ssr,
        ss_total,
        df_treatment: dft,
        df_subjects: dfs,
        df_residual: dfr,
        ms_treatment: mst,
        ms_subjects: mss,
        ms_residual: msr,
        f_treatment: f_treat,
        p_uncorrected: p_uncorr,
        p_greenhouse_geisser: p_gg,
        p_huynh_feldt: p_hf,
        f_subjects: f_subj,
        p_subjects: p_subj,
        sphericity,
        partial_eta_squared: partial_eta2,
    })
}

pub(in crate::anova) fn helmert_contrast_matrix(k: usize) -> Result<DenseMatrix, StatsError> {
    if k <= 1 {
        return Ok(DenseMatrix::zeros(0, k)?);
    }
    let p = k - 1;
    let mut h_mat = DenseMatrix::zeros(p, k)?;
    for i in 0..p {
        let m = (i + 1) as f64;
        let norm = (m * (m + 1.0)).sqrt();
        for j in 0..=i {
            h_mat.set(i, j, 1.0 / norm)?;
        }
        h_mat.set(i, i + 1, -m / norm)?;
    }
    Ok(h_mat)
}

pub(in crate::anova) fn unit_row_vector(k: usize) -> Result<DenseMatrix, StatsError> {
    let val = if k > 0 { 1.0 / (k as f64).sqrt() } else { 0.0 };
    Ok(DenseMatrix::from_row_major(1, k, vec![val; k])?)
}

pub(in crate::anova) fn kronecker_dense(a: &DenseMatrix, b: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    let ra = a.nrows();
    let ca = a.ncols();
    let rb = b.nrows();
    let cb = b.ncols();
    let rows = ra * rb;
    let cols = ca * cb;
    let mut data = vec![0.0_f64; rows * cols];
    for ia in 0..ra {
        for ja in 0..ca {
            let av = a.get(ia, ja).unwrap_or(0.0);
            for ib in 0..rb {
                let r = ia * rb + ib;
                for jb in 0..cb {
                    let c = ja * cb + jb;
                    data[r * cols + c] = av * b.get(ib, jb).unwrap_or(0.0);
                }
            }
        }
    }
    Ok(DenseMatrix::from_row_major(rows, cols, data)?)
}

pub(in crate::anova) fn pooled_within_group_cov(groups: &[Vec<Vec<f64>>]) -> Result<DenseMatrix, StatsError> {
    let k = groups.first().and_then(|g| g.first()).map_or(0, Vec::len);
    if k == 0 {
        return Err(StatsError::EmptyInput);
    }
    let mut total_df = 0.0_f64;
    let mut s_sum = vec![0.0_f64; k * k];
    for grp in groups {
        let n_g = grp.len();
        if n_g < 2 {
            continue;
        }
        let ng_f = n_g as f64;
        let mut means = vec![0.0_f64; k];
        for row in grp {
            if row.len() != k {
                return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: k, actual: row.len() }));
            }
            for (c, &v) in row.iter().enumerate() {
                means[c] += v / ng_f;
            }
        }
        total_df += ng_f - 1.0;
        for r in 0..k {
            for c in r..k {
                let mut acc = NeumaierSum::new();
                for row in grp {
                    acc.add((row[r] - means[r]) * (row[c] - means[c]));
                }
                s_sum[r * k + c] += acc.total();
            }
        }
    }
    if total_df <= 0.0 {
        return Err(StatsError::InsufficientSampleSize { context: "within-group covariance df", required: 2, actual: 0 });
    }
    for r in 0..k {
        for c in r..k {
            let val = s_sum[r * k + c] / total_df;
            s_sum[r * k + c] = val;
            s_sum[c * k + r] = val;
        }
    }
    Ok(DenseMatrix::from_row_major(k, k, s_sum)?)
}

/// Compute Mauchly's `W` and Greenhouse–Geisser / Huynh–Feldt sphericity epsilons from a covariance
/// matrix `S` (`df_e` error df, `n_subj` total subjects across `n_groups` independent groups) via
/// an orthonormal contrast projection `M S M^T`.
///
/// - Mauchly (1940) `W = det(C) / (tr(C)/p)^p` using chi-square approximation on `p(p + 1)/2 - 1` df
///   when `df_e >= p`; when `df_e < p` (singular Wishart), Mauchly's `p` is reported as `NaN` (inestimable).
/// - Greenhouse & Geisser (1959) `epsilon_hat = (sum lambda)^2 / (p sum lambda^2)` bounded in `[1/p, 1]`
///   (falling back to the conservative lower bound `1/p` when `df_e < 1` or contrast variance is zero).
/// - Huynh & Feldt (1976, p. 74) with the split-plot `(N - g + 1)` correction of Lecoutre (1991,
///   *J. Educ. Stat.* 16:371–372): `epsilon_tilde = ((N - g + 1) p epsilon_hat - 2) / (p (df_e - p epsilon_hat))`,
///   clamped to `[epsilon_hat, 1]`.
pub(in crate::anova) fn compute_sphericity_from_cov_contrast(
    s_cov: &DenseMatrix,
    contrast: &DenseMatrix,
    df_e: f64,
    n_subj: f64,
    n_groups: f64,
) -> Result<SphericityResult, StatsError> {
    let p = contrast.nrows();
    if p <= 1 {
        return Ok(SphericityResult {
            mauchly_w: 1.0,
            mauchly_chi2: 0.0,
            mauchly_df: 0.0,
            mauchly_p: 1.0,
            greenhouse_geisser_eps: 1.0,
            huynh_feldt_eps: 1.0,
        });
    }
    let pf = p as f64;
    let lower_bound = 1.0 / pf;
    let m_df = 0.5 * pf * (pf + 1.0) - 1.0;
    if !df_e.is_finite() || df_e < 1.0 {
        return Ok(SphericityResult {
            mauchly_w: f64::NAN,
            mauchly_chi2: f64::NAN,
            mauchly_df: m_df,
            mauchly_p: f64::NAN,
            greenhouse_geisser_eps: lower_bound,
            huynh_feldt_eps: lower_bound,
        });
    }
    let c_cov = symmetrize_dense(&contrast.matmul(s_cov)?.matmul(&contrast.transpose())?)?;
    let eig = sym_eigen(&c_cov)?;
    let eigvals: Vec<f64> = eig.eigenvalues.iter().map(|&x| x.max(0.0)).collect();
    let tr: f64 = eigvals.iter().sum();
    let tr_sq: f64 = eigvals.iter().map(|&l| l * l).sum();
    if tr <= 0.0 || tr_sq <= 0.0 {
        return Ok(SphericityResult {
            mauchly_w: f64::NAN,
            mauchly_chi2: f64::NAN,
            mauchly_df: m_df,
            mauchly_p: f64::NAN,
            greenhouse_geisser_eps: lower_bound,
            huynh_feldt_eps: lower_bound,
        });
    }
    let eps_gg = ((tr * tr) / (pf * tr_sq)).clamp(lower_bound, 1.0);
    let n_eff = n_subj - n_groups + 1.0;
    let hf_num = n_eff * pf * eps_gg - 2.0;
    let hf_denom = pf * (df_e - pf * eps_gg);
    let eps_hf = if df_e <= 1.0 {
        eps_gg
    } else if hf_denom > 0.0 {
        (hf_num / hf_denom).clamp(eps_gg, 1.0)
    } else if hf_num > 0.0 {
        1.0
    } else {
        eps_gg
    };

    if df_e < pf {
        return Ok(SphericityResult {
            mauchly_w: 0.0,
            mauchly_chi2: f64::NAN,
            mauchly_df: m_df,
            mauchly_p: f64::NAN,
            greenhouse_geisser_eps: eps_gg,
            huynh_feldt_eps: eps_hf,
        });
    }

    // Normalize eigenvalues by mean_eig = tr / p before taking logs so Mauchly W is scale-invariant.
    let mean_eig = tr / pf;
    let ln_w: f64 = eigvals.iter().map(|&v| (v / mean_eig).max(1e-15).ln()).sum::<f64>().min(0.0);
    let w = ln_w.exp().clamp(1e-15, 1.0);
    let f_scale = (1.0 - (2.0 * pf * pf + pf + 2.0) / (6.0 * pf * df_e)).max(0.0);
    let chi2 = (-df_e * f_scale * ln_w).max(0.0);
    let mauchly_p = if m_df > 0.0 { chi2_sf(chi2, m_df)? } else { 1.0 };

    Ok(SphericityResult { mauchly_w: w, mauchly_chi2: chi2, mauchly_df: m_df, mauchly_p, greenhouse_geisser_eps: eps_gg, huynh_feldt_eps: eps_hf })
}

pub(in crate::anova) fn compute_sphericity(blocks: &[Vec<f64>]) -> Result<SphericityResult, StatsError> {
    let n = blocks.len();
    let k = blocks.first().map_or(0, Vec::len);
    if k <= 2 {
        return Ok(SphericityResult {
            mauchly_w: 1.0,
            mauchly_chi2: 0.0,
            mauchly_df: 0.0,
            mauchly_p: 1.0,
            greenhouse_geisser_eps: 1.0,
            huynh_feldt_eps: 1.0,
        });
    }
    if n < 2 {
        let pf = (k - 1) as f64;
        let lb = 1.0 / pf;
        return Ok(SphericityResult {
            mauchly_w: f64::NAN,
            mauchly_chi2: f64::NAN,
            mauchly_df: 0.5 * pf * (pf + 1.0) - 1.0,
            mauchly_p: f64::NAN,
            greenhouse_geisser_eps: lb,
            huynh_feldt_eps: lb,
        });
    }
    let s_cov = pooled_within_group_cov(&[blocks.to_vec()])?;
    let h_mat = helmert_contrast_matrix(k)?;
    let nf = n as f64;
    compute_sphericity_from_cov_contrast(&s_cov, &h_mat, nf - 1.0, nf, 1.0)
}
