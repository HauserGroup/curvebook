//! Repeated-measures two-way ANOVA with per-term sphericity estimates and corrected *p* values.
//!
//! References:
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Mauchly, J. W. (1940). Significance test for sphericity of a normal *n*-variate distribution.
//!   *Annals of Mathematical Statistics* 11(2):204–209. doi:10.1214/aoms/1177731915
//! - Greenhouse, S. W., & Geisser, S. (1959). On methods in the analysis of profile data.
//!   *Psychometrika* 24(2):95–112. doi:10.1007/BF02289823
//! - Huynh, H., & Feldt, L. S. (1976). Estimation of the Box correction for degrees of freedom from
//!   sample data in randomized block and split-plot designs. *Journal of Educational Statistics*
//!   1(1):69–82. doi:10.3102/10769986001001069
//! - Lecoutre, B. (1991). A correction for the epsilon-tilde approximate test in repeated measures
//!   designs with two or more independent groups. *Journal of Educational Statistics*
//!   16(4):371–372. doi:10.3102/10769986016004371

use curvebook_doc::{RepeatedMeasuresDesign, SumOfSquaresType};
use curvebook_math::dist::f_sf;
use curvebook_math::sum::NeumaierSum;

use crate::anova::effect_sizes::{clamp_ss_roundoff, f_ratio_p};
use crate::anova::one_way::{
    compute_sphericity_from_cov_contrast, helmert_contrast_matrix, kronecker_dense, pooled_within_group_cov, unit_row_vector,
};
use crate::anova::results::{
    FactorialAnovaResult, FactorialAnovaTerm, RmSphericityCorrection, RmTwoWayAnovaResult, RmTwoWaySphericities, SphericityResult,
};
use crate::error::StatsError;

pub(in crate::anova) fn apply_rm_sphericity_p(
    f_val: f64,
    df_num: f64,
    df_den: f64,
    sph: Option<&SphericityResult>,
    correction: RmSphericityCorrection,
) -> Result<f64, StatsError> {
    let eps = match (correction, sph) {
        (RmSphericityCorrection::None, _) | (_, None) => 1.0,
        (RmSphericityCorrection::GreenhouseGeisser, Some(s)) => s.greenhouse_geisser_eps,
        (RmSphericityCorrection::HuynhFeldt, Some(s)) => s.huynh_feldt_eps,
    };
    let adj_df1 = (df_num * eps).max(1e-12);
    let adj_df2 = (df_den * eps).max(1e-12);
    Ok(f_sf(f_val, adj_df1, adj_df2)?)
}

pub(in crate::anova) fn compute_rm_two_way_sphericities(
    subject_data: &[Vec<Vec<f64>>],
    a: usize,
    b: usize,
    rm_design: RepeatedMeasuresDesign,
) -> Result<RmTwoWaySphericities, StatsError> {
    let s_count = subject_data.len();
    let sf = s_count as f64;
    let af = a as f64;
    let bf = b as f64;
    match rm_design {
        RepeatedMeasuresDesign::BothFactorsMatched => {
            let mut y_flat = Vec::with_capacity(s_count);
            for mat in subject_data {
                let mut row = Vec::with_capacity(a * b);
                for r_vec in mat {
                    row.extend_from_slice(r_vec);
                }
                y_flat.push(row);
            }
            let s_cov = pooled_within_group_cov(&[y_flat])?;
            let ha = helmert_contrast_matrix(a)?;
            let hb = helmert_contrast_matrix(b)?;
            let ua = unit_row_vector(a)?;
            let ub = unit_row_vector(b)?;
            let m_row = kronecker_dense(&ha, &ub)?;
            let m_col = kronecker_dense(&ua, &hb)?;
            let m_int = kronecker_dense(&ha, &hb)?;
            let df_e = sf - 1.0;
            Ok(RmTwoWaySphericities {
                row: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_row, df_e, sf, 1.0)?),
                column: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_col, df_e, sf, 1.0)?),
                interaction: Some(compute_sphericity_from_cov_contrast(&s_cov, &m_int, df_e, sf, 1.0)?),
            })
        }
        RepeatedMeasuresDesign::ColumnFactorMatched => {
            let mut groups = vec![Vec::with_capacity(s_count); a];
            for mat in subject_data {
                for i in 0..a {
                    groups[i].push(mat[i].clone());
                }
            }
            let s_pool = pooled_within_group_cov(&groups)?;
            let m_col = helmert_contrast_matrix(b)?;
            let n_subj = af * sf;
            let df_e = n_subj - af;
            let sph_col = compute_sphericity_from_cov_contrast(&s_pool, &m_col, df_e, n_subj, af)?;
            Ok(RmTwoWaySphericities { row: None, column: Some(sph_col), interaction: None })
        }
        RepeatedMeasuresDesign::RowFactorMatched => {
            let mut groups = vec![Vec::with_capacity(s_count); b];
            for mat in subject_data {
                for j in 0..b {
                    let mut vec_subj = Vec::with_capacity(a);
                    for r_vec in mat.iter().take(a) {
                        vec_subj.push(r_vec[j]);
                    }
                    groups[j].push(vec_subj);
                }
            }
            let s_pool = pooled_within_group_cov(&groups)?;
            let m_row = helmert_contrast_matrix(a)?;
            let n_subj = bf * sf;
            let df_e = n_subj - bf;
            let sph_row = compute_sphericity_from_cov_contrast(&s_pool, &m_row, df_e, n_subj, bf)?;
            Ok(RmTwoWaySphericities { row: Some(sph_row), column: None, interaction: None })
        }
        RepeatedMeasuresDesign::None => Ok(RmTwoWaySphericities::default()),
    }
}

#[derive(Debug, Clone)]
pub(in crate::anova) struct RmBalancedPostHocModel {
    pub(in crate::anova) design: RepeatedMeasuresDesign,
    pub(in crate::anova) s_count: usize,
    pub(in crate::anova) a_count: usize,
    pub(in crate::anova) b_count: usize,
    pub(in crate::anova) row_means: Vec<f64>,
    pub(in crate::anova) col_means: Vec<f64>,
    pub(in crate::anova) cell_means: Vec<Vec<f64>>,
    pub(in crate::anova) err_a_ms: f64,
    pub(in crate::anova) err_a_df: f64,
    pub(in crate::anova) err_b_ms: f64,
    pub(in crate::anova) err_b_df: f64,
    pub(in crate::anova) err_ab_ms: f64,
    pub(in crate::anova) err_ab_df: f64,
    pub(in crate::anova) subject_data: Vec<Vec<Vec<f64>>>,
}

/// Repeated-measures two-way ANOVA (`RowFactorMatched`, `ColumnFactorMatched`, or
/// `BothFactorsMatched`) on complete balanced subject blocks `subject_data[subject][row][col]`,
/// returning separate sphericity diagnostics for all within-subject factors and applying the
/// requested [`RmSphericityCorrection`] (`None`, `GreenhouseGeisser`, or `HuynhFeldt`) directly
/// to the *F*-test *p*-values.
///
/// - When `BothFactorsMatched`, sphericity matrices come from the Kronecker orthonormal Helmert
///   contrasts `H_A x u_B`, `u_A x H_B`, and `H_A x H_B` on the subject covariance matrix (`df_e = S - 1`).
/// - For split-plot designs (`ColumnFactorMatched` / `RowFactorMatched`), sphericity is computed from
///   the pooled within-group covariance matrix on `df_error = N - g` (Huynh & Feldt, 1976, p. 71)
///   and applied to both the within-subject main effect and the interaction term.
pub fn fit_rm_two_way_anova_with_sphericity(
    subject_data: &[Vec<Vec<f64>>],
    row_name: &str,
    col_name: &str,
    rm_design: RepeatedMeasuresDesign,
    correction: RmSphericityCorrection,
) -> Result<RmTwoWayAnovaResult, StatsError> {
    let (res, _) = fit_rm_two_way_anova_with_posthoc(subject_data, row_name, col_name, rm_design, correction)?;
    Ok(res)
}

pub(in crate::anova) fn fit_rm_two_way_anova_with_posthoc(
    subject_data: &[Vec<Vec<f64>>],
    row_name: &str,
    col_name: &str,
    rm_design: RepeatedMeasuresDesign,
    correction: RmSphericityCorrection,
) -> Result<(RmTwoWayAnovaResult, RmBalancedPostHocModel), StatsError> {
    let s_count = subject_data.len();
    if s_count < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "RM two-way subjects", required: 2, actual: s_count });
    }
    let a = subject_data[0].len();
    let b = subject_data[0].first().map_or(0, Vec::len);
    if a < 2 || b < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "RM two-way factor levels", required: 2, actual: a.min(b) });
    }
    let af = a as f64;
    let bf = b as f64;
    let sf = s_count as f64;
    let total_n = s_count * a * b;

    let mut grand_sum = NeumaierSum::new();
    let mut cell_means = vec![vec![0.0_f64; b]; a];
    let mut row_means = vec![0.0_f64; a];
    let mut col_means = vec![0.0_f64; b];
    let mut subj_means = vec![0.0_f64; s_count];
    let mut subj_row_means = vec![vec![0.0_f64; a]; s_count];
    let mut subj_col_means = vec![vec![0.0_f64; b]; s_count];
    let mut data_max_abs = 0.0_f64;

    for (s, mat) in subject_data.iter().enumerate() {
        if mat.len() != a {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: a, actual: mat.len() }));
        }
        let mut s_acc = NeumaierSum::new();
        for (i, r_vec) in mat.iter().enumerate() {
            if r_vec.len() != b {
                return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: b, actual: r_vec.len() }));
            }
            for (j, &v) in r_vec.iter().enumerate() {
                if !v.is_finite() {
                    return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "y", value: v }));
                }
                data_max_abs = data_max_abs.max(v.abs());
                grand_sum.add(v);
                s_acc.add(v);
                cell_means[i][j] += v / sf;
                row_means[i] += v / (sf * bf);
                col_means[j] += v / (sf * af);
                subj_row_means[s][i] += v / bf;
                subj_col_means[s][j] += v / af;
            }
        }
        subj_means[s] = s_acc.total() / (af * bf);
    }
    let gm = grand_sum.total() / (total_n as f64);

    let ss_a = clamp_ss_roundoff(row_means.iter().map(|&m| sf * bf * (m - gm).powi(2)).sum(), data_max_abs, total_n);
    let ss_b = clamp_ss_roundoff(col_means.iter().map(|&m| sf * af * (m - gm).powi(2)).sum(), data_max_abs, total_n);
    let mut ss_ab = 0.0_f64;
    for i in 0..a {
        for j in 0..b {
            let eff = cell_means[i][j] - row_means[i] - col_means[j] + gm;
            ss_ab += sf * eff * eff;
        }
    }
    let ss_ab = clamp_ss_roundoff(ss_ab, data_max_abs, total_n);
    let ss_s = clamp_ss_roundoff(subj_means.iter().map(|&m| af * bf * (m - gm).powi(2)).sum(), data_max_abs, total_n);
    let mut ss_sa = 0.0_f64;
    let mut ss_sb = 0.0_f64;
    let mut ss_sab = 0.0_f64;
    for s in 0..s_count {
        for i in 0..a {
            let ea = subj_row_means[s][i] - subj_means[s] - row_means[i] + gm;
            ss_sa += bf * ea * ea;
        }
        for j in 0..b {
            let eb = subj_col_means[s][j] - subj_means[s] - col_means[j] + gm;
            ss_sb += af * eb * eb;
        }
        for i in 0..a {
            for j in 0..b {
                let e = subject_data[s][i][j] - subj_row_means[s][i] - subj_col_means[s][j] - cell_means[i][j]
                    + subj_means[s]
                    + row_means[i]
                    + col_means[j]
                    - gm;
                ss_sab += e * e;
            }
        }
    }
    let ss_sa = clamp_ss_roundoff(ss_sa, data_max_abs, total_n);
    let ss_sb = clamp_ss_roundoff(ss_sb, data_max_abs, total_n);
    let ss_sab = clamp_ss_roundoff(ss_sab, data_max_abs, total_n);

    let dfa = af - 1.0;
    let dfb = bf - 1.0;
    let dfab = dfa * dfb;
    let ss_total = ss_a + ss_b + ss_ab + ss_s + ss_sa + ss_sb + ss_sab;

    let sphericities = compute_rm_two_way_sphericities(subject_data, a, b, rm_design)?;
    let (err_a_ss, err_a_df, err_b_ss, err_b_df, err_ab_ss, err_ab_df, term_sphs) = match rm_design {
        RepeatedMeasuresDesign::BothFactorsMatched => (
            ss_sa,
            (sf - 1.0) * dfa,
            ss_sb,
            (sf - 1.0) * dfb,
            ss_sab,
            (sf - 1.0) * dfab,
            [sphericities.row.as_ref(), sphericities.column.as_ref(), sphericities.interaction.as_ref()],
        ),
        RepeatedMeasuresDesign::ColumnFactorMatched => {
            let ss_subj_within_a = ss_s + ss_sa;
            let df_subj_within_a = af * (sf - 1.0);
            let ss_err_within = ss_sb + ss_sab;
            let df_err_within = af * (sf - 1.0) * dfb;
            (
                ss_subj_within_a,
                df_subj_within_a,
                ss_err_within,
                df_err_within,
                ss_err_within,
                df_err_within,
                [None, sphericities.column.as_ref(), sphericities.column.as_ref()],
            )
        }
        RepeatedMeasuresDesign::RowFactorMatched => {
            let ss_subj_within_b = ss_s + ss_sb;
            let df_subj_within_b = bf * (sf - 1.0);
            let ss_err_within = ss_sa + ss_sab;
            let df_err_within = bf * (sf - 1.0) * dfa;
            (
                ss_err_within,
                df_err_within,
                ss_subj_within_b,
                df_subj_within_b,
                ss_err_within,
                df_err_within,
                [sphericities.row.as_ref(), None, sphericities.row.as_ref()],
            )
        }
        RepeatedMeasuresDesign::None => {
            let df_res = (total_n as f64) - af * bf;
            let ss_res = ss_s + ss_sa + ss_sb + ss_sab;
            (ss_res, df_res, ss_res, df_res, ss_res, df_res, [None, None, None])
        }
    };

    let make_term =
        |name: String, ss: f64, df: f64, e_ss: f64, e_df: f64, sph_opt: Option<&SphericityResult>| -> Result<FactorialAnovaTerm, StatsError> {
            let ms = ss / df.max(1.0);
            let ms_e = e_ss / e_df.max(1.0);
            let (f, _) = f_ratio_p(ms, ms_e, df.max(1.0), e_df.max(1.0))?;
            let p = apply_rm_sphericity_p(f, df.max(1.0), e_df.max(1.0), sph_opt, correction)?;
            Ok(FactorialAnovaTerm {
                name,
                ss,
                df,
                ms,
                f_statistic: Some(f),
                p_value: Some(p),
                partial_eta_squared: Some(if ss + e_ss > 0.0 { ss / (ss + e_ss) } else { 0.0 }),
                eta_squared: Some(if ss_total > 0.0 { ss / ss_total } else { 0.0 }),
            })
        };

    let terms = vec![
        make_term(row_name.to_owned(), ss_a, dfa, err_a_ss, err_a_df, term_sphs[0])?,
        make_term(col_name.to_owned(), ss_b, dfb, err_b_ss, err_b_df, term_sphs[1])?,
        make_term(format!("{row_name} × {col_name}"), ss_ab, dfab, err_ab_ss, err_ab_df, term_sphs[2])?,
    ];

    let df_resid = err_ab_df;
    let ms_resid = err_ab_ss / df_resid.max(1.0);
    let anova = FactorialAnovaResult {
        terms,
        ss_residual: err_ab_ss,
        df_residual: df_resid,
        ms_residual: ms_resid,
        ss_total,
        df_total: (total_n - 1) as f64,
        total_n,
        unbalanced: false,
        ss_type: SumOfSquaresType::TypeIII,
    };
    let rm_res = RmTwoWayAnovaResult { anova, design: rm_design, sphericities, correction, term_error_dfs: [err_a_df, err_b_df, err_ab_df] };
    let post_hoc = RmBalancedPostHocModel {
        design: rm_design,
        s_count,
        a_count: a,
        b_count: b,
        row_means,
        col_means,
        cell_means,
        err_a_ms: err_a_ss / err_a_df.max(1.0),
        err_a_df,
        err_b_ms: err_b_ss / err_b_df.max(1.0),
        err_b_df,
        err_ab_ms: ms_resid,
        err_ab_df,
        subject_data: subject_data.to_vec(),
    };
    Ok((rm_res, post_hoc))
}

/// Repeated-measures two-way ANOVA (`RowFactorMatched`, `ColumnFactorMatched`, or `BothFactorsMatched`)
/// on complete balanced subject blocks `subject_data[subject][row][col]`, returning uncorrected *p*-values
/// alongside the primary within-subject [`SphericityResult`].
///
/// See [`fit_rm_two_way_anova_with_sphericity`] for full per-factor (`row`, `column`, `interaction`)
/// sphericity results and applied Greenhouse–Geisser / Huynh–Feldt *p*-values.
pub fn fit_rm_two_way_anova(
    subject_data: &[Vec<Vec<f64>>],
    row_name: &str,
    col_name: &str,
    rm_design: RepeatedMeasuresDesign,
) -> Result<(FactorialAnovaResult, Option<SphericityResult>), StatsError> {
    let res = fit_rm_two_way_anova_with_sphericity(subject_data, row_name, col_name, rm_design, RmSphericityCorrection::None)?;
    let primary_sph = match rm_design {
        RepeatedMeasuresDesign::BothFactorsMatched | RepeatedMeasuresDesign::ColumnFactorMatched => res.sphericities.column,
        RepeatedMeasuresDesign::RowFactorMatched => res.sphericities.row,
        RepeatedMeasuresDesign::None => None,
    };
    Ok((res.anova, primary_sph))
}
