//! Nested (hierarchical) one-way ANOVA and the nested *t*-test.
//!
//! References:
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Kenward, M. G., & Roger, J. H. (1997). Small sample inference for fixed effects from
//!   restricted maximum likelihood. *Biometrics* 53(3):983–997. doi:10.2307/2533558

use curvebook_doc::MixedModelDfMethod;
use curvebook_math::dist::f_sf;
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::{NeumaierSum, SummaryMoments};

use crate::anova::effect_sizes::f_ratio_p;
use crate::anova::reml::fit_random_intercept_reml_with_method_and_core;
use crate::anova::results::NestedAnovaResult;
use crate::error::StatsError;

/// Fit nested hierarchical one-way ANOVA (Groups -> Biological-Unit Subclusters -> Technical
/// Replicates), computing exact Satterthwaite / Fai & Cornelius (1996) and Kenward & Roger (1997)
/// denominator degrees of freedom for unequal cluster sizes and variance components
/// `(sigma_subcluster^2, sigma_error^2)`.
///
/// The two-level nested partition (groups, subclusters within groups, error) and the subcluster
/// *F*-test follow Zar (2010, ch. 15). The among-groups test is the REML small-sample
/// Kenward–Roger Wald *F*-test from [`fit_random_intercept_reml`](crate::anova::fit_random_intercept_reml) (Patterson & Thompson, 1971;
/// Kenward & Roger, 1997). For two groups, the nested `t` is the signed square root of that
/// adjusted Wald *F*.
pub fn nested_one_way_anova(table: &curvebook_doc::NestedTable, two_group_t_test: bool) -> Result<NestedAnovaResult, StatsError> {
    nested_one_way_anova_with_df_method(table, two_group_t_test, MixedModelDfMethod::KenwardRoger)
}

/// Variant of [`nested_one_way_anova`] selecting [`MixedModelDfMethod`] (Kenward–Roger vs Satterthwaite)
/// for the among-groups REML Wald *F* / nested *t* test and denominator *df*.
pub fn nested_one_way_anova_with_df_method(
    table: &curvebook_doc::NestedTable,
    two_group_t_test: bool,
    df_method: MixedModelDfMethod,
) -> Result<NestedAnovaResult, StatsError> {
    let selected: Vec<usize> = (0..table.groups.len()).collect();
    nested_one_way_anova_selected(table, &selected, two_group_t_test, df_method)
}

pub(in crate::anova) fn nested_one_way_anova_selected(
    table: &curvebook_doc::NestedTable,
    selected_groups: &[usize],
    two_group_t_test: bool,
    df_method: MixedModelDfMethod,
) -> Result<NestedAnovaResult, StatsError> {
    let g_count = selected_groups.len();
    if two_group_t_test && g_count != 2 {
        return Err(StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: g_count });
    }
    if g_count < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: g_count });
    }

    let mut group_labels = Vec::with_capacity(g_count);
    let mut group_sums = Vec::with_capacity(g_count);
    let mut group_n = Vec::with_capacity(g_count);
    let mut subcluster_data: Vec<(usize, Vec<f64>)> = Vec::new();
    let mut total_obs = 0_usize;
    let mut grand_sum = NeumaierSum::new();

    for (local_g_idx, &orig_g_idx) in selected_groups.iter().enumerate() {
        let group = table.groups.get(orig_g_idx).ok_or(StatsError::MissingColumn { index: orig_g_idx })?;
        group_labels.push(group.title.clone());
        let mut g_acc = NeumaierSum::new();
        let mut g_n = 0_usize;
        for b_idx in 0..group.biological_unit_count() {
            let reps = table.active_technical_replicates(orig_g_idx, b_idx);
            if !reps.is_empty() {
                let s: f64 = reps.iter().sum();
                g_acc.add(s);
                grand_sum.add(s);
                g_n += reps.len();
                total_obs += reps.len();
                subcluster_data.push((local_g_idx, reps));
            }
        }
        if g_n == 0 {
            return Err(StatsError::EmptyInput);
        }
        group_sums.push(g_acc.total());
        group_n.push(g_n);
    }

    let n_subclusters = subcluster_data.len();
    if n_subclusters <= g_count {
        return Err(StatsError::InsufficientSampleSize { context: "nested subclusters", required: g_count + 1, actual: n_subclusters });
    }
    if total_obs <= n_subclusters {
        return Err(StatsError::InsufficientSampleSize { context: "nested technical replicates", required: n_subclusters + 1, actual: total_obs });
    }

    let grand_mean = grand_sum.total() / (total_obs as f64);
    let mut ss_groups = NeumaierSum::new();
    let mut ss_sub = NeumaierSum::new();
    let mut ss_err = NeumaierSum::new();

    for g_idx in 0..g_count {
        let g_mean = group_sums[g_idx] / (group_n[g_idx] as f64);
        let d = g_mean - grand_mean;
        ss_groups.add((group_n[g_idx] as f64) * d * d);
    }

    // Build REML matrix as well for exact unbalanced Satterthwaite & Kenward-Roger df.
    let mut x_data = vec![0.0_f64; total_obs * g_count];
    let mut y_vec = Vec::with_capacity(total_obs);
    let mut cluster_ids = Vec::with_capacity(total_obs);
    let mut row_cursor = 0_usize;

    for (cid, (g_idx, reps)) in subcluster_data.iter().enumerate() {
        let g_mean = group_sums[*g_idx] / (group_n[*g_idx] as f64);
        let m = SummaryMoments::from_slice(reps)?;
        let n_ij = reps.len() as f64;
        let d_sub = m.mean - g_mean;
        ss_sub.add(n_ij * d_sub * d_sub);
        ss_err.add(m.css);

        for &v in reps {
            x_data[row_cursor * g_count] = 1.0;
            if *g_idx > 0 {
                x_data[row_cursor * g_count + *g_idx] = 1.0;
            }
            y_vec.push(v);
            cluster_ids.push(cid);
            row_cursor += 1;
        }
    }

    let ssg = ss_groups.total().max(0.0);
    let sss = ss_sub.total().max(0.0);
    let sse = ss_err.total().max(0.0);

    let df1_g = (g_count - 1) as f64;
    let df1_sub = (n_subclusters - g_count) as f64;
    let df2_err = (total_obs - n_subclusters) as f64;

    let msg = ssg / df1_g;
    let mss = sss / df1_sub;
    let mse = sse / df2_err;

    let (f_sub, p_sub) = f_ratio_p(mss, mse, df1_sub, df2_err)?;

    let x_mat = DenseMatrix::from_row_major(total_obs, g_count, x_data)?;
    let (reml, _) = fit_random_intercept_reml_with_method_and_core(&x_mat, &y_vec, &cluster_ids, &[("Group", 1, g_count - 1)], df_method)?;

    let f_g = reml.fixed_terms.first().and_then(|t| t.f_statistic).unwrap_or(if mss > 0.0 {
        msg / mss
    } else if msg > 0.0 {
        f64::INFINITY
    } else {
        0.0
    });
    let df2_satt = reml.satterthwaite_df.first().copied().unwrap_or(df1_sub);
    let df2_kr = reml.kenward_roger_df.first().copied().unwrap_or(df1_sub);
    let df2_active = match df_method {
        MixedModelDfMethod::KenwardRoger => df2_kr,
        MixedModelDfMethod::Satterthwaite => df2_satt,
    };
    let p_g = reml.fixed_terms.first().and_then(|t| t.p_value).unwrap_or(f_sf(f_g, df1_g, df2_active)?);

    let t_nested = if two_group_t_test {
        let sign = if (group_sums[0] / (group_n[0] as f64)) >= (group_sums[1] / (group_n[1] as f64)) { 1.0 } else { -1.0 };
        Some(sign * f_g.sqrt())
    } else {
        None
    };

    Ok(NestedAnovaResult {
        group_labels,
        f_among_groups: f_g,
        df1_groups: df1_g,
        df2_groups_satterthwaite: df2_satt,
        df2_groups_kenward_roger: df2_kr,
        p_among_groups: p_g,
        t_nested,
        f_among_subclusters: f_sub,
        df1_subclusters: df1_sub,
        df2_error: df2_err,
        p_among_subclusters: p_sub,
        ss_groups: ssg,
        ss_subclusters: sss,
        ss_error: sse,
        ms_groups: msg,
        ms_subclusters: mss,
        ms_error: mse,
        variance_subclusters: reml.sigma2_random,
        variance_error: reml.sigma2_error,
        icc_subclusters: reml.icc,
        total_observations: total_obs,
    })
}
