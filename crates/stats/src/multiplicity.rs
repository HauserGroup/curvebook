//! Multiple comparisons (FWER) and False Discovery Rate (FDR) adjustment engines.
//!
//! Supports all 14+ procedures: Unadjusted, Tukey–Kramer, Šídák, Bonferroni, Holm–Šídák,
//! Holm–Bonferroni, Dunnett, Dunnett's T3, Games–Howell, Tamhane's T2, Dunn's rank-sum,
//! Fisher's LSD, Student–Newman–Keuls, Benjamini–Hochberg, Benjamini–Yekutieli, and
//! two-stage Benjamini–Krieger–Yekutieli, plus post-hoc linear trend testing.

use lumencraft_doc::{MultiplicityMethod, MultiplicitySpec, PairwiseComparisonRecord, ResultCell, ResultTab};
use lumencraft_math::dist::{
    dunnett_loadings_from_sizes, dunnett_two_sided_crit, dunnett_two_sided_p, f_sf, norm_sf, norm_two_sided_p, student_t_two_sided_isf,
    student_t_two_sided_p, studentized_range_isf, studentized_range_sf,
};
use lumencraft_math::quad::gauss_legendre_32;
use lumencraft_math::roots::{RootOptions, brent_root};
use lumencraft_math::special::gamma::ln_gamma;
use lumencraft_math::sum::NeumaierSum;
use serde::{Deserialize, Serialize};

use crate::error::StatsError;

/// Unified familywise error-rate and FDR adjustment selector (extends [`MultiplicityMethod`] with
/// standalone Holm–Bonferroni, Fisher LSD, and Newman–Keuls options).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MultiplicityProcedure {
    None,
    TukeyKramer,
    Sidak,
    Bonferroni,
    HolmSidak,
    HolmBonferroni,
    Dunnett { control_column: usize },
    DunnettT3,
    GamesHowell,
    TamhaneT2,
    DunnRankSum,
    FisherLsd,
    NewmanKeuls,
    BenjaminiHochbergFdr,
    BenjaminiYekutieliFdr,
    TwoStageBkyFdr,
}

impl From<MultiplicityMethod> for MultiplicityProcedure {
    fn from(value: MultiplicityMethod) -> Self {
        match value {
            MultiplicityMethod::None => Self::None,
            MultiplicityMethod::TukeyHsd => Self::TukeyKramer,
            MultiplicityMethod::Dunnett { control_column } => Self::Dunnett { control_column },
            MultiplicityMethod::Sidak => Self::Sidak,
            MultiplicityMethod::Bonferroni => Self::Bonferroni,
            MultiplicityMethod::HolmSidak => Self::HolmSidak,
            MultiplicityMethod::DunnettT3 => Self::DunnettT3,
            MultiplicityMethod::GamesHowell => Self::GamesHowell,
            MultiplicityMethod::TamhaneT2 => Self::TamhaneT2,
            MultiplicityMethod::DunnRankSum => Self::DunnRankSum,
            MultiplicityMethod::LinearTrend => Self::None,
            MultiplicityMethod::BenjaminiHochbergFdr => Self::BenjaminiHochbergFdr,
            MultiplicityMethod::BenjaminiYekutieliFdr => Self::BenjaminiYekutieliFdr,
            MultiplicityMethod::TwoStageBkyFdr => Self::TwoStageBkyFdr,
        }
    }
}

/// Group summary statistics consumed by pairwise comparison engines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupComparisonInput {
    pub index: usize,
    pub label: String,
    pub n: f64,
    pub mean: f64,
    pub sd: f64,
}

/// Single detailed pairwise comparison output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairwiseComparisonDetail {
    pub group_a: usize,
    pub group_b: usize,
    pub label_a: String,
    pub label_b: String,
    pub mean_diff: f64,
    pub se_diff: f64,
    pub statistic: f64,
    pub df: f64,
    pub raw_p: f64,
    pub adjusted_p_or_q: f64,
    pub is_fdr_q: bool,
    pub ci: Option<(f64, f64)>,
    pub significant: bool,
}

impl PairwiseComparisonDetail {
    /// Convert to [`PairwiseComparisonRecord`] for [`lumencraft_doc::ResultSheet`].
    #[must_use]
    pub fn to_record(&self) -> PairwiseComparisonRecord {
        PairwiseComparisonRecord {
            group_a: self.group_a,
            group_b: self.group_b,
            label_a: self.label_a.clone(),
            label_b: self.label_b.clone(),
            estimate_diff: Some(self.mean_diff),
            raw_p_or_q: self.adjusted_p_or_q,
            is_fdr_q: self.is_fdr_q,
            significant: self.significant,
        }
    }
}

/// Post-hoc linear trend test across ordered groups.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearTrendResult {
    pub ss_trend: f64,
    pub df_trend: f64,
    pub df_error: f64,
    pub f_statistic: f64,
    pub p_value: f64,
    pub slope_estimate: f64,
}

/// Compute post-hoc linear trend test across ordered groups using pooled `mse` and `df_error`.
pub fn linear_trend_test(groups: &[GroupComparisonInput], mse: f64, df_error: f64) -> Result<LinearTrendResult, StatsError> {
    let k = groups.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "linear trend test", required: 2, actual: k });
    }
    if !mse.is_finite() || mse < 0.0 || !df_error.is_finite() || df_error <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "mse_or_df", value: mse });
    }
    let center = 0.5 * ((k - 1) as f64);
    let mut num = NeumaierSum::new();
    let mut denom_ss = NeumaierSum::new();
    let mut denom_slope = NeumaierSum::new();
    for (i, g) in groups.iter().enumerate() {
        if g.n <= 0.0 {
            return Err(StatsError::InvalidParameter { name: "group_n", value: g.n });
        }
        let c = (i as f64) - center;
        num.add(c * g.mean);
        denom_ss.add((c * c) / g.n);
        denom_slope.add(c * c);
    }
    let l_val = num.total();
    let d_val = denom_ss.total();
    let ss_trend = if d_val > 0.0 { (l_val * l_val) / d_val } else { 0.0 };
    let f_stat = if mse > 0.0 { ss_trend / mse } else { 0.0 };
    let p_value = if mse > 0.0 { f_sf(f_stat, 1.0, df_error)? } else { 1.0 };
    let slope_estimate = if denom_slope.total() > 0.0 { l_val / denom_slope.total() } else { 0.0 };
    Ok(LinearTrendResult { ss_trend, df_trend: 1.0, df_error, f_statistic: f_stat, p_value, slope_estimate })
}

// ============================================================================
// Standalone P-Value & FDR Adjustment Functions
// ============================================================================

/// Adjust a slice of raw *p*-values using Bonferroni (`min(1, m * p)`).
#[must_use]
pub fn adjust_bonferroni(raw_p: &[f64]) -> Vec<f64> {
    let m = raw_p.len() as f64;
    raw_p.iter().map(|&p| (p.clamp(0.0, 1.0) * m).min(1.0)).collect()
}

/// Adjust a slice of raw *p*-values using Šídák (`1 - (1 - p)^m`, cancellation-free).
#[must_use]
pub fn adjust_sidak(raw_p: &[f64]) -> Vec<f64> {
    let m = raw_p.len() as f64;
    raw_p.iter().map(|&p| sidak_single(p, m)).collect()
}

#[inline]
fn sidak_single(p: f64, m: f64) -> f64 {
    let pc = p.clamp(0.0, 1.0);
    if pc >= 1.0 {
        return 1.0;
    }
    if pc <= 0.0 {
        return 0.0;
    }
    (-(m * (-pc).ln_1p()).exp_m1()).clamp(pc, 1.0)
}

/// Step-down Holm–Bonferroni adjusted *p*-values (monotone and `>= raw_p`).
#[must_use]
pub fn adjust_holm_bonferroni(raw_p: &[f64]) -> Vec<f64> {
    step_down_adjust(raw_p, false)
}

/// Step-down Holm–Šídák adjusted *p*-values (monotone and `>= raw_p`).
#[must_use]
pub fn adjust_holm_sidak(raw_p: &[f64]) -> Vec<f64> {
    step_down_adjust(raw_p, true)
}

fn step_down_adjust(raw_p: &[f64], sidak: bool) -> Vec<f64> {
    let m = raw_p.len();
    if m == 0 {
        return Vec::new();
    }
    let mut order: Vec<(usize, f64)> = raw_p.iter().copied().enumerate().map(|(i, p)| (i, p.clamp(0.0, 1.0))).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut out = vec![0.0_f64; m];
    let mut running_max = 0.0_f64;
    for (rank, &(orig_idx, p)) in order.iter().enumerate() {
        let mult = (m - rank) as f64;
        let cand = if sidak { sidak_single(p, mult) } else { (p * mult).min(1.0) };
        running_max = running_max.max(cand).max(p).min(1.0);
        out[orig_idx] = running_max;
    }
    out
}

/// Benjamini–Hochberg (BH) step-up FDR *q*-values (monotone and `>= raw_p`).
#[must_use]
pub fn adjust_benjamini_hochberg(raw_p: &[f64]) -> Vec<f64> {
    step_up_fdr(raw_p, 1.0)
}

/// Benjamini–Yekutieli (BY) step-up FDR *q*-values under arbitrary dependency (`c(m) = H_m`).
#[must_use]
pub fn adjust_benjamini_yekutieli(raw_p: &[f64]) -> Vec<f64> {
    let m = raw_p.len();
    let harmonic: f64 = (1..=m).map(|i| 1.0 / (i as f64)).sum();
    step_up_fdr(raw_p, harmonic.max(1.0))
}

fn step_up_fdr(raw_p: &[f64], multiplier: f64) -> Vec<f64> {
    let m = raw_p.len();
    if m == 0 {
        return Vec::new();
    }
    let mf = m as f64;
    let mut order: Vec<(usize, f64)> = raw_p.iter().copied().enumerate().map(|(i, p)| (i, p.clamp(0.0, 1.0))).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut out = vec![1.0_f64; m];
    let mut running_min = 1.0_f64;
    for (rank, &(orig_idx, p)) in order.iter().enumerate().rev() {
        let cand = (p * mf * multiplier / ((rank + 1) as f64)).min(1.0);
        running_min = running_min.min(cand);
        out[orig_idx] = running_min.max(p).min(1.0);
    }
    out
}

/// Two-stage linear step-up Benjamini–Krieger–Yekutieli (2006) FDR *q*-values evaluated at target `fdr_q`.
/// Discoveries satisfy `q_i <= fdr_q` iff rejected by the two-stage BKY procedure at `fdr_q`.
#[must_use]
pub fn adjust_two_stage_bky(raw_p: &[f64], fdr_q: f64) -> Vec<f64> {
    let m = raw_p.len();
    if m == 0 {
        return Vec::new();
    }
    let q = fdr_q.clamp(1e-6, 0.9999);
    let q_stage1 = q / (1.0 + q);
    let bh1 = adjust_benjamini_hochberg(raw_p);
    let r1 = bh1.iter().filter(|&&qv| qv <= q_stage1).count();
    if r1 == 0 {
        return bh1.into_iter().zip(raw_p.iter()).map(|(qv, &p)| (qv * (1.0 + q)).max(p.clamp(0.0, 1.0)).min(1.0)).collect();
    }
    if r1 == m {
        return bh1.into_iter().zip(raw_p.iter()).map(|(qv, &p)| (qv * (1.0 + q)).max(p.clamp(0.0, 1.0)).min(1.0)).collect();
    }
    let m0_hat = (m - r1) as f64;
    let mut order: Vec<(usize, f64)> = raw_p.iter().copied().enumerate().map(|(i, p)| (i, p.clamp(0.0, 1.0))).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut out = vec![1.0_f64; m];
    let mut running_min = 1.0_f64;
    for (rank, &(orig_idx, p)) in order.iter().enumerate().rev() {
        let cand = (p * m0_hat * (1.0 + q) / ((rank + 1) as f64)).min(1.0);
        running_min = running_min.min(cand);
        out[orig_idx] = running_min.max(p).min(1.0);
    }
    out
}

/// Apply any p-value-only multiplicity procedure (`None`, `Bonferroni`, `Sidak`, `HolmBonferroni`,
/// `HolmSidak`, `BenjaminiHochbergFdr`, `BenjaminiYekutieliFdr`, `TwoStageBkyFdr`) directly to raw *p*-values.
#[must_use]
pub fn adjust_p_values(raw_p: &[f64], procedure: MultiplicityProcedure, threshold: f64) -> (Vec<f64>, bool) {
    match procedure {
        MultiplicityProcedure::Bonferroni => (adjust_bonferroni(raw_p), false),
        MultiplicityProcedure::Sidak => (adjust_sidak(raw_p), false),
        MultiplicityProcedure::HolmBonferroni => (adjust_holm_bonferroni(raw_p), false),
        MultiplicityProcedure::HolmSidak => (adjust_holm_sidak(raw_p), false),
        MultiplicityProcedure::BenjaminiHochbergFdr => (adjust_benjamini_hochberg(raw_p), true),
        MultiplicityProcedure::BenjaminiYekutieliFdr => (adjust_benjamini_yekutieli(raw_p), true),
        MultiplicityProcedure::TwoStageBkyFdr => (adjust_two_stage_bky(raw_p, threshold), true),
        _ => (raw_p.iter().map(|p| p.clamp(0.0, 1.0)).collect(), false),
    }
}

// ============================================================================
// Pairwise Group Comparison Engines (Pooled-Variance, Unequal-Variance, Rank)
// ============================================================================

/// Compute pairwise comparisons across `groups` using pooled `mse` and `df_error` (for parametric
/// equal-variance tests) or group SDs (for `GamesHowell`, `DunnettT3`, `TamhaneT2`).
pub fn compare_groups_pairwise(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    spec: &MultiplicitySpec,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    compare_groups_with_procedure(groups, mse, df_error, spec.method.into(), spec.threshold, spec.report_confidence_intervals)
}

/// Compute pairwise comparisons with explicit [`MultiplicityProcedure`].
pub fn compare_groups_with_procedure(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    procedure: MultiplicityProcedure,
    threshold: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    if k < 2 {
        return Ok(Vec::new());
    }
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    match procedure {
        MultiplicityProcedure::Dunnett { control_column } => compare_dunnett(groups, mse, df_error, control_column, threshold, report_ci),
        MultiplicityProcedure::TukeyKramer => compare_tukey_kramer(groups, mse, df_error, threshold, report_ci),
        MultiplicityProcedure::NewmanKeuls => compare_newman_keuls(groups, mse, df_error, threshold),
        MultiplicityProcedure::GamesHowell => compare_games_howell(groups, threshold, report_ci),
        MultiplicityProcedure::TamhaneT2 => compare_tamhane_t2(groups, threshold, report_ci),
        MultiplicityProcedure::DunnettT3 => compare_dunnett_t3(groups, threshold, report_ci),
        _ => compare_pooled_or_fdr(groups, mse, df_error, procedure, threshold, report_ci),
    }
}

fn compare_pooled_or_fdr(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    procedure: MultiplicityProcedure,
    threshold: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let m = k * (k - 1) / 2;
    let mut raw_p = Vec::with_capacity(m);
    let mut entries = Vec::with_capacity(m);

    let per_comp_alpha = match procedure {
        MultiplicityProcedure::Bonferroni => Some(threshold / (m as f64)),
        MultiplicityProcedure::Sidak => Some(1.0 - (1.0 - threshold).powf(1.0 / (m as f64))),
        MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd => Some(threshold),
        _ => None,
    };
    let t_crit_opt = match (report_ci, per_comp_alpha) {
        (true, Some(a)) => Some(student_t_two_sided_isf(a.clamp(1e-12, 0.9999), df_error)?),
        _ => None,
    };

    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let se = (mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
            let (t_stat, p_val) = if se > 0.0 {
                let t = diff / se;
                (t, student_t_two_sided_p(t, df_error)?)
            } else {
                (0.0, 1.0)
            };
            raw_p.push(p_val);
            let ci = t_crit_opt.map(|tc| (diff - tc * se, diff + tc * se));
            entries.push((ga, gb, diff, se, t_stat, p_val, ci));
        }
    }

    let (adj, is_fdr) = adjust_p_values(&raw_p, procedure, threshold);
    let out = entries
        .into_iter()
        .zip(adj)
        .map(|((ga, gb, diff, se, t_stat, p_val, ci), adj_p)| PairwiseComparisonDetail {
            group_a: ga.index,
            group_b: gb.index,
            label_a: ga.label.clone(),
            label_b: gb.label.clone(),
            mean_diff: diff,
            se_diff: se,
            statistic: t_stat,
            df: df_error,
            raw_p: p_val,
            adjusted_p_or_q: adj_p,
            is_fdr_q: is_fdr,
            ci,
            significant: adj_p <= threshold,
        })
        .collect();
    Ok(out)
}

fn compare_tukey_kramer(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    threshold: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let q_crit = if report_ci { Some(studentized_range_isf(threshold, k, df_error)?) } else { None };
    let mut out = Vec::with_capacity(k * (k - 1) / 2);
    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let se_tukey = (0.5 * mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
            let (q_stat, p_val) = if se_tukey > 0.0 {
                let q = diff.abs() / se_tukey;
                (q, studentized_range_sf(q, k, df_error)?)
            } else {
                (0.0, 1.0)
            };
            let ci = q_crit.map(|qc| (diff - qc * se_tukey, diff + qc * se_tukey));
            out.push(PairwiseComparisonDetail {
                group_a: ga.index,
                group_b: gb.index,
                label_a: ga.label.clone(),
                label_b: gb.label.clone(),
                mean_diff: diff,
                se_diff: se_tukey * std::f64::consts::SQRT_2,
                statistic: q_stat,
                df: df_error,
                raw_p: p_val,
                adjusted_p_or_q: p_val,
                is_fdr_q: false,
                ci,
                significant: p_val <= threshold,
            });
        }
    }
    Ok(out)
}

fn compare_newman_keuls(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    threshold: f64,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let mut sorted_idx: Vec<usize> = (0..k).collect();
    sorted_idx.sort_by(|&a, &b| groups[a].mean.total_cmp(&groups[b].mean));

    // Compute p-values in sorted-mean matrix `[p_lo][p_hi]` for `p_lo < p_hi`, enforcing superset
    // step-down monotonicity: `adj_p[i][j] = max(raw_p[i][j], adj_p[i-1][j], adj_p[i][j+1])`.
    let mut p_mat = vec![vec![1.0_f64; k]; k];
    for span in (2..=k).rev() {
        for lo in 0..=(k - span) {
            let hi = lo + span - 1;
            let ga = &groups[sorted_idx[lo]];
            let gb = &groups[sorted_idx[hi]];
            let se_q = (0.5 * mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
            let raw_p = if se_q > 0.0 {
                let q = (gb.mean - ga.mean).abs() / se_q;
                studentized_range_sf(q, span, df_error)?
            } else {
                1.0
            };
            let mut adj = raw_p;
            if lo > 0 {
                adj = adj.max(p_mat[lo - 1][hi]);
            }
            if hi + 1 < k {
                adj = adj.max(p_mat[lo][hi + 1]);
            }
            p_mat[lo][hi] = adj.clamp(0.0, 1.0);
        }
    }

    let mut pos_of = vec![0_usize; k];
    for (pos, &orig) in sorted_idx.iter().enumerate() {
        pos_of[orig] = pos;
    }

    let mut out = Vec::with_capacity(k * (k - 1) / 2);
    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let se_q = (0.5 * mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
            let q_stat = if se_q > 0.0 { diff.abs() / se_q } else { 0.0 };
            let (p_lo, p_hi) = if pos_of[i] < pos_of[j] { (pos_of[i], pos_of[j]) } else { (pos_of[j], pos_of[i]) };
            let adj_p = p_mat[p_lo][p_hi];
            out.push(PairwiseComparisonDetail {
                group_a: ga.index,
                group_b: gb.index,
                label_a: ga.label.clone(),
                label_b: gb.label.clone(),
                mean_diff: diff,
                se_diff: se_q * std::f64::consts::SQRT_2,
                statistic: q_stat,
                df: df_error,
                raw_p: adj_p,
                adjusted_p_or_q: adj_p,
                is_fdr_q: false,
                ci: None,
                significant: adj_p <= threshold,
            });
        }
    }
    Ok(out)
}

fn compare_dunnett(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
    control_column: usize,
    threshold: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let ctrl_pos = groups.iter().position(|g| g.index == control_column).unwrap_or(0);
    let ctrl = &groups[ctrl_pos];
    let treatments: Vec<&GroupComparisonInput> = groups.iter().enumerate().filter(|&(i, _)| i != ctrl_pos).map(|(_, g)| g).collect();
    if treatments.is_empty() {
        return Ok(Vec::new());
    }
    let treat_sizes: Vec<f64> = treatments.iter().map(|g| g.n).collect();
    let loadings = dunnett_loadings_from_sizes(ctrl.n, &treat_sizes)?;
    let d_crit = if report_ci { Some(dunnett_two_sided_crit(threshold, df_error, ctrl.n, &treat_sizes)?) } else { None };

    let mut out = Vec::with_capacity(treatments.len());
    for (idx, tr) in treatments.iter().enumerate() {
        let diff = tr.mean - ctrl.mean;
        let se = (mse * (1.0 / tr.n + 1.0 / ctrl.n)).max(0.0).sqrt();
        let (t_stat, p_val) = if se > 0.0 {
            let t = diff / se;
            // Exact Dunnett multivariate-t p-value for this comparison against all m loadings.
            let p =
                if loadings.len() == 1 { student_t_two_sided_p(t, df_error)? } else { dunnett_two_sided_p(t.abs(), df_error, ctrl.n, &treat_sizes)? };
            (t, p)
        } else {
            (0.0, 1.0)
        };
        let _ = idx;
        let ci = d_crit.map(|dc| (diff - dc * se, diff + dc * se));
        out.push(PairwiseComparisonDetail {
            group_a: tr.index,
            group_b: ctrl.index,
            label_a: tr.label.clone(),
            label_b: ctrl.label.clone(),
            mean_diff: diff,
            se_diff: se,
            statistic: t_stat,
            df: df_error,
            raw_p: p_val,
            adjusted_p_or_q: p_val,
            is_fdr_q: false,
            ci,
            significant: p_val <= threshold,
        });
    }
    Ok(out)
}

fn welch_pair_se_df(ga: &GroupComparisonInput, gb: &GroupComparisonInput) -> (f64, f64) {
    let va = (ga.sd * ga.sd) / ga.n;
    let vb = (gb.sd * gb.sd) / gb.n;
    let v_sum = va + vb;
    if v_sum <= 0.0 {
        return (0.0, (ga.n + gb.n - 2.0).max(1.0));
    }
    let denom = (va * va) / (ga.n - 1.0).max(1.0) + (vb * vb) / (gb.n - 1.0).max(1.0);
    let df = if denom > 0.0 { (v_sum * v_sum) / denom } else { (ga.n + gb.n - 2.0).max(1.0) };
    (v_sum.sqrt(), df.max(1.0))
}

fn compare_games_howell(groups: &[GroupComparisonInput], threshold: f64, report_ci: bool) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let mut out = Vec::with_capacity(k * (k - 1) / 2);
    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let (se, df) = welch_pair_se_df(ga, gb);
            let se_q = se * std::f64::consts::FRAC_1_SQRT_2;
            let (q_stat, p_val) = if se_q > 0.0 {
                let q = diff.abs() / se_q;
                (q, studentized_range_sf(q, k, df)?)
            } else {
                (0.0, 1.0)
            };
            let ci = if report_ci {
                let qc = studentized_range_isf(threshold, k, df)?;
                Some((diff - qc * se_q, diff + qc * se_q))
            } else {
                None
            };
            out.push(PairwiseComparisonDetail {
                group_a: ga.index,
                group_b: gb.index,
                label_a: ga.label.clone(),
                label_b: gb.label.clone(),
                mean_diff: diff,
                se_diff: se,
                statistic: q_stat,
                df,
                raw_p: p_val,
                adjusted_p_or_q: p_val,
                is_fdr_q: false,
                ci,
                significant: p_val <= threshold,
            });
        }
    }
    Ok(out)
}

fn compare_tamhane_t2(groups: &[GroupComparisonInput], threshold: f64, report_ci: bool) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let m = (k * (k - 1) / 2) as f64;
    let alpha_sidak = 1.0 - (1.0 - threshold).powf(1.0 / m);
    let mut out = Vec::with_capacity(k * (k - 1) / 2);
    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let (se, df) = welch_pair_se_df(ga, gb);
            let (t_stat, raw_p) = if se > 0.0 {
                let t = diff / se;
                (t, student_t_two_sided_p(t, df)?)
            } else {
                (0.0, 1.0)
            };
            let adj_p = sidak_single(raw_p, m);
            let ci = if report_ci {
                let tc = student_t_two_sided_isf(alpha_sidak.clamp(1e-12, 0.9999), df)?;
                Some((diff - tc * se, diff + tc * se))
            } else {
                None
            };
            out.push(PairwiseComparisonDetail {
                group_a: ga.index,
                group_b: gb.index,
                label_a: ga.label.clone(),
                label_b: gb.label.clone(),
                mean_diff: diff,
                se_diff: se,
                statistic: t_stat,
                df,
                raw_p,
                adjusted_p_or_q: adj_p,
                is_fdr_q: false,
                ci,
                significant: adj_p <= threshold,
            });
        }
    }
    Ok(out)
}

/// Studentized Maximum Modulus survival probability `P(max_{1..m} |T_l| > t)` with Welch df `nu`,
/// used for Dunnett's T3 unequal-variance pairwise comparisons.
fn studentized_max_modulus_sf(t_abs: f64, m: usize, df: f64) -> Result<f64, StatsError> {
    if t_abs <= 0.0 {
        return Ok(1.0);
    }
    if df >= 1e7 {
        let p1 = norm_two_sided_p(t_abs);
        return Ok(sidak_single(p1, m as f64));
    }
    let mf = m as f64;
    let half_df = 0.5 * df;
    let ln_c = (1.0 - half_df) * std::f64::consts::LN_2 + half_df.ln() * half_df - ln_gamma(half_df)?;
    let integrand = |u: f64| -> f64 {
        if u <= 0.0 {
            return 0.0;
        }
        let log_chi = ln_c + (df - 1.0) * u.ln() - 0.5 * df * u * u;
        if log_chi < -700.0 {
            return 0.0;
        }
        let two_tail = (2.0 * norm_sf(t_abs * u)).min(1.0);
        if two_tail >= 1.0 {
            return log_chi.exp();
        }
        let p_out = -((mf * (-two_tail).ln_1p()).exp_m1());
        log_chi.exp() * p_out.clamp(0.0, 1.0)
    };
    let u_sd = 1.0 / (2.0 * df).sqrt();
    let u_lo = (1.0 - 7.0 * u_sd).max(0.0);
    let u_hi = (1.0 + 7.0 * u_sd).max(2.0);
    let val = gauss_legendre_32(integrand, u_lo, 1.0) + gauss_legendre_32(integrand, 1.0, u_hi);
    Ok(val.clamp(0.0, 1.0))
}

fn studentized_max_modulus_crit(alpha: f64, m: usize, df: f64) -> Result<f64, StatsError> {
    let hi_guess = student_t_two_sided_isf((alpha / (m as f64)).clamp(1e-12, 0.999), df)?.max(2.0) * 1.25;
    let f = |t: f64| studentized_max_modulus_sf(t, m, df).unwrap_or(0.0) - alpha;
    brent_root(f, 0.0, hi_guess, RootOptions { rel_tol: 1e-8, abs_tol: 1e-9, max_iters: 64 }).map_err(StatsError::from)
}

fn compare_dunnett_t3(groups: &[GroupComparisonInput], threshold: f64, report_ci: bool) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = groups.len();
    let m = k * (k - 1) / 2;
    let mut out = Vec::with_capacity(m);
    for i in 0..k {
        for j in (i + 1)..k {
            let ga = &groups[i];
            let gb = &groups[j];
            let diff = ga.mean - gb.mean;
            let (se, df) = welch_pair_se_df(ga, gb);
            let (t_stat, adj_p) = if se > 0.0 {
                let t = diff / se;
                (t, studentized_max_modulus_sf(t.abs(), m, df)?)
            } else {
                (0.0, 1.0)
            };
            let ci = if report_ci {
                let crit = studentized_max_modulus_crit(threshold, m, df)?;
                Some((diff - crit * se, diff + crit * se))
            } else {
                None
            };
            out.push(PairwiseComparisonDetail {
                group_a: ga.index,
                group_b: gb.index,
                label_a: ga.label.clone(),
                label_b: gb.label.clone(),
                mean_diff: diff,
                se_diff: se,
                statistic: t_stat,
                df,
                raw_p: adj_p,
                adjusted_p_or_q: adj_p,
                is_fdr_q: false,
                ci,
                significant: adj_p <= threshold,
            });
        }
    }
    Ok(out)
}

/// Dunn's nonparametric rank-sum pairwise comparison with tie correction and configurable
/// post-adjustment (Bonferroni, Šídák, Holm, or BH/BY/BKY FDR; defaults to Bonferroni when
/// `DunnRankSum` or `None` is specified on nonparametric ANOVA).
pub fn compare_dunn_ranks(
    group_labels: &[(usize, String)],
    group_n: &[usize],
    mean_ranks: &[f64],
    total_n: usize,
    tie_sum_t3_minus_t: f64,
    spec: &MultiplicitySpec,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k = group_labels.len();
    if k < 2 || total_n < 2 {
        return Ok(Vec::new());
    }
    let nf = total_n as f64;
    let tie_factor = (nf * (nf + 1.0) / 12.0 - tie_sum_t3_minus_t / (12.0 * (nf - 1.0))).max(0.0);
    let m = k * (k - 1) / 2;
    let mut raw_p = Vec::with_capacity(m);
    let mut entries = Vec::with_capacity(m);

    for i in 0..k {
        for j in (i + 1)..k {
            let ni = group_n[i] as f64;
            let nj = group_n[j] as f64;
            let diff = mean_ranks[i] - mean_ranks[j];
            let se = (tie_factor * (1.0 / ni + 1.0 / nj)).sqrt();
            let (z_stat, p_val) = if se > 0.0 {
                let z = diff / se;
                (z, norm_two_sided_p(z.abs()))
            } else {
                (0.0, 1.0)
            };
            raw_p.push(p_val);
            entries.push((&group_labels[i], &group_labels[j], diff, se, z_stat, p_val));
        }
    }

    let proc_eff = match MultiplicityProcedure::from(spec.method) {
        MultiplicityProcedure::DunnRankSum | MultiplicityProcedure::None => MultiplicityProcedure::Bonferroni,
        other => other,
    };
    let (adj, is_fdr) = adjust_p_values(&raw_p, proc_eff, spec.threshold);
    let out = entries
        .into_iter()
        .zip(adj)
        .map(|(((idx_a, lbl_a), (idx_b, lbl_b), diff, se, z_stat, p_val), adj_p)| PairwiseComparisonDetail {
            group_a: *idx_a,
            group_b: *idx_b,
            label_a: lbl_a.clone(),
            label_b: lbl_b.clone(),
            mean_diff: diff,
            se_diff: se,
            statistic: z_stat,
            df: f64::INFINITY,
            raw_p: p_val,
            adjusted_p_or_q: adj_p,
            is_fdr_q: is_fdr,
            ci: None,
            significant: adj_p <= spec.threshold,
        })
        .collect();
    Ok(out)
}

/// Convert pairwise details into a [`lumencraft_doc::ResultTab`] and attach
/// [`PairwiseComparisonRecord`] entries (emitting an informational note if `m == 1`).
pub fn build_multiplicity_tab(
    details: &[PairwiseComparisonDetail],
    title: &str,
    p_fmt: lumencraft_doc::PValueFormat,
    confidence: lumencraft_math::ConfidenceLevel,
) -> ResultTab {
    let headers = vec![
        String::from("Mean Diff."),
        String::from("SE of Diff."),
        String::from("Statistic"),
        String::from("df"),
        String::from("Raw P"),
        String::from("Adjusted P / q"),
        format!("{}% CI of Diff.", confidence.percent()),
        String::from("Significant"),
    ];
    let mut row_labels = Vec::with_capacity(details.len());
    let mut rows = Vec::with_capacity(details.len());
    for d in details {
        row_labels.push(format!("{} vs. {}", d.label_a, d.label_b));
        let ci_cell_val = match d.ci {
            Some((lo, hi)) => crate::common::ci_cell(lo, hi),
            None => ResultCell::Empty,
        };
        rows.push(vec![
            crate::common::num_cell(d.mean_diff),
            crate::common::num_cell(d.se_diff),
            crate::common::num_cell(d.statistic),
            crate::common::num_cell(d.df),
            crate::common::p_cell(d.raw_p, p_fmt),
            crate::common::p_cell(d.adjusted_p_or_q, p_fmt),
            ci_cell_val,
            ResultCell::Boolean(d.significant),
        ]);
    }
    let mut tab =
        crate::common::build_tab("multiple-comparisons", title, lumencraft_doc::ResultTabKind::MultipleComparisons, headers, row_labels, rows);
    if details.len() == 1 {
        tab.footnotes.push(String::from("Only m = 1 comparison in family; multiplicity adjustment leaves the unadjusted P value unchanged."));
    }
    tab
}
