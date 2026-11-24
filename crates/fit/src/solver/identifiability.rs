//! Parameter identifiability assessment, greedy Hessian condition-number reduction, and
//! automated 6-rule fit-quality flagging (`docs/implementation/curve-fitting-implementation.md` §2.5).
//!
//! References:
//! - Golub, G. H. & Van Loan, C. F. (2013). *Matrix Computations* (4th ed.). Johns Hopkins University Press.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Venzon, D. J. & Moolgavkar, S. H. (1988). A method for computing profile-likelihood-based confidence intervals. *Applied Statistics* 37(1), 87–94. `doi:10.2307/2347496`.

use std::collections::HashSet;

use curvebook_doc::{CiBound, FitFlagRules, IdentifiabilityMode};
use curvebook_math::linalg::{DenseMatrix, qr_col_pivoted, svd};
use curvebook_math::sum::NeumaierSum;

use crate::error::{FitError, FitIdentifiability, FitStatus, ParameterNote};
use crate::solver::layout::{ObsRecord, ParameterLayout};
use crate::solver::results::NonlinearFitResult;

const MIN_SCALED_RCOND: f64 = 1e-12;
// Dependency = 1 - 1 / VIF (Motulsky & Christopoulos, 2004).
const MAX_DEPENDENCY: f64 = 1.0 - 1.0 / 10_000.0;

/// Spectral condition-number threshold `10 / sqrt(eps_mach) ~= 6.7108864e8` for greedy Hessian
/// row/column reduction (§2.5.1, IEEE-754 `f64`).
#[must_use]
pub fn hessian_kappa_threshold() -> f64 {
    10.0 / f64::EPSILON.sqrt()
}

/// Outcome of greedy Hessian row/column reduction (§2.5.1).
#[derive(Debug, Clone, PartialEq)]
pub struct GreedyHessianReduction {
    /// Zero-based parameter indices dropped in greedy order as `"Unstable"`.
    pub unstable_indices: Vec<usize>,
    /// Remaining well-conditioned parameter indices in ascending original order.
    pub stable_indices: Vec<usize>,
    /// Spectral condition number `kappa(H)` before reduction (`f64::INFINITY` if singular).
    pub initial_kappa: f64,
    /// Spectral condition number `kappa(H_reduced)` after reduction (`1.0` if `<= 1` positive param remains).
    pub final_kappa: f64,
    /// Per-step audit `(dropped_index, kappa_before, kappa_after)`.
    pub steps: Vec<(usize, f64, f64)>,
}

/// Assemble the converged objective Hessian `H in R^{K x K}` from the weighted Jacobian `J_w`:
/// `H = 2 J_w^T J_w` under weighted least squares, or `H = J_w^T J_w` (exact Fisher information)
/// under Poisson deviance regression (§2.5.1).
pub fn assemble_objective_hessian(j_mat: &DenseMatrix, is_poisson: bool) -> Result<DenseMatrix, FitError> {
    let n = j_mat.nrows();
    let k = j_mat.ncols();
    let scale = if is_poisson { 1.0 } else { 2.0 };
    let mut data = vec![0.0_f64; k * k];
    for a in 0..k {
        for b in a..k {
            let mut acc = NeumaierSum::new();
            for i in 0..n {
                let ja = j_mat.get(i, a).unwrap_or(0.0);
                let jb = j_mat.get(i, b).unwrap_or(0.0);
                acc.add(ja * jb);
            }
            let val = scale * acc.total();
            data[a * k + b] = val;
            data[b * k + a] = val;
        }
    }
    Ok(DenseMatrix::from_row_major(k, k, data)?)
}

/// Compute the 2-norm / spectral condition number `kappa(H) = sigma_max(H) / sigma_min(H)` of a
/// symmetric square matrix `H` via SVD (Golub & Van Loan 2013).
///
/// Returns `1.0` for an empty `0x0` matrix or a `1x1` matrix with strictly positive finite entry,
/// and `f64::INFINITY` when `H` is singular, non-positive-definite on a 1x1 diagonal, or contains
/// non-finite entries.
pub fn hessian_condition_number(h: &DenseMatrix) -> Result<f64, FitError> {
    let k = h.nrows();
    if k != h.ncols() {
        return Err(FitError::InvalidParameter { name: "hessian", reason: format!("Hessian must be square, got {}x{}", h.nrows(), h.ncols()) });
    }
    if k == 0 {
        return Ok(1.0);
    }
    for r in 0..k {
        for c in 0..k {
            if !h.get(r, c).is_some_and(f64::is_finite) {
                return Ok(f64::INFINITY);
            }
        }
    }
    if k == 1 {
        let d = h.get(0, 0).unwrap_or(0.0);
        return Ok(if d.is_finite() && d > 0.0 { 1.0 } else { f64::INFINITY });
    }
    let spectrum = svd(h)?;
    let Some(&s_max) = spectrum.singular_values.first() else {
        return Ok(f64::INFINITY);
    };
    let s_min = spectrum.singular_values.last().copied().unwrap_or(0.0);
    if !s_max.is_finite() || !s_min.is_finite() || s_max <= 0.0 || s_min <= 0.0 {
        return Ok(f64::INFINITY);
    }
    let kappa = s_max / s_min;
    Ok(if kappa.is_finite() { kappa } else { f64::INFINITY })
}

fn principal_submatrix(h: &DenseMatrix, indices: &[usize]) -> Result<DenseMatrix, FitError> {
    let m = indices.len();
    let mut data = Vec::with_capacity(m * m);
    for &r in indices {
        for &c in indices {
            data.push(h.get(r, c).unwrap_or(0.0));
        }
    }
    Ok(DenseMatrix::from_row_major(m, m, data)?)
}

/// Extract a column subset of a Jacobian matrix for reduced-subspace covariance estimation.
pub(crate) fn submatrix_columns(j_mat: &DenseMatrix, cols: &[usize]) -> Result<DenseMatrix, FitError> {
    let rows = j_mat.nrows();
    let k = cols.len();
    let mut data = Vec::with_capacity(rows * k);
    for r in 0..rows {
        for &c in cols {
            data.push(j_mat.get(r, c).unwrap_or(0.0));
        }
    }
    Ok(DenseMatrix::from_row_major(rows, k, data)?)
}

/// Greedily delete rows/columns from Hessian `H` until `kappa(H) <= threshold` (§2.5.1).
///
/// At each step where `kappa(H_active) > threshold`, chooses `k* = argmin_k kappa(H_{-k,-k})`
/// (breaking ties deterministically by smallest diagonal entry `H[k, k]`, then highest index)
/// so the maximally ill-conditioned parameter is dropped first while remaining well-conditioned
/// parameters retain valid covariance and confidence intervals.
pub fn greedy_hessian_reduction(h: &DenseMatrix, threshold: f64) -> Result<GreedyHessianReduction, FitError> {
    let k = h.nrows();
    if k != h.ncols() {
        return Err(FitError::InvalidParameter { name: "hessian", reason: format!("Hessian must be square, got {}x{}", h.nrows(), h.ncols()) });
    }
    let mut active: Vec<usize> = (0..k).collect();
    let initial_kappa = hessian_condition_number(h)?;
    let mut current_kappa = initial_kappa;
    let mut unstable_indices = Vec::new();
    let mut steps = Vec::new();

    while !active.is_empty() && (!current_kappa.is_finite() || current_kappa > threshold) {
        if active.len() == 1 {
            let dropped = active.remove(0);
            unstable_indices.push(dropped);
            steps.push((dropped, current_kappa, 1.0));
            current_kappa = 1.0;
            break;
        }
        let mut best_pos = 0_usize;
        let mut best_kappa = f64::INFINITY;
        let mut best_diag = f64::INFINITY;
        let mut best_orig_idx = 0_usize;
        let mut candidate = Vec::with_capacity(active.len() - 1);

        for pos in 0..active.len() {
            candidate.clear();
            for (i, &orig) in active.iter().enumerate() {
                if i != pos {
                    candidate.push(orig);
                }
            }
            let sub = principal_submatrix(h, &candidate)?;
            let cand_kappa = hessian_condition_number(&sub)?;
            let orig_idx = active[pos];
            let diag = h.get(orig_idx, orig_idx).unwrap_or(0.0).abs();

            let strictly_better = if cand_kappa.is_finite() && !best_kappa.is_finite() {
                true
            } else if !cand_kappa.is_finite() && best_kappa.is_finite() {
                false
            } else if cand_kappa.is_finite() && best_kappa.is_finite() {
                let scale = cand_kappa.abs().max(best_kappa.abs()).max(1.0);
                if (cand_kappa - best_kappa).abs() <= 1e-12 * scale {
                    if (diag - best_diag).abs() <= 1e-12 * diag.max(best_diag).max(1.0) { orig_idx > best_orig_idx } else { diag < best_diag }
                } else {
                    cand_kappa < best_kappa
                }
            } else if (diag - best_diag).abs() <= 1e-12 * diag.max(best_diag).max(1.0) {
                orig_idx > best_orig_idx
            } else {
                diag < best_diag
            };

            if pos == 0 || strictly_better {
                best_pos = pos;
                best_kappa = cand_kappa;
                best_diag = diag;
                best_orig_idx = orig_idx;
            }
        }

        let dropped = active.remove(best_pos);
        unstable_indices.push(dropped);
        steps.push((dropped, current_kappa, best_kappa));
        current_kappa = best_kappa;
    }

    Ok(GreedyHessianReduction { unstable_indices, stable_indices: active, initial_kappa, final_kappa: current_kappa, steps })
}

/// Result of evaluating automated 6-rule [`FitFlagRules`] on a converged nonlinear fit (§2.5.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FitFlagOutcome {
    /// `true` when at least one configured rule was violated (`"Flagged"`).
    pub flagged: bool,
    /// Human-readable descriptions of every violated quality rule.
    pub reasons: Vec<String>,
}

/// Evaluate the 6 automated fit-quality flag rules (`R^2`, `N`, max dependency, residual normality *p*,
/// runs/lack-of-fit *p*, ROUT outlier count; §2.5.4).
#[must_use]
pub fn evaluate_fit_flags(fit: &NonlinearFitResult, rules: &FitFlagRules) -> FitFlagOutcome {
    let mut reasons = Vec::new();
    if let Some(min_r2) = rules.min_r_squared
        && fit.r_squared.is_finite()
        && fit.r_squared < min_r2
    {
        reasons.push(format!("R^2 = {:.4} is below minimum threshold {:.4}.", fit.r_squared, min_r2));
    }
    if let Some(min_n) = rules.min_observations
        && fit.n_obs < min_n
    {
        reasons.push(format!("Analyzed observations N = {} is below minimum threshold {}.", fit.n_obs, min_n));
    }
    if let Some(max_dep) = rules.max_dependency
        && !fit.is_robust_cauchy()
        && let Some(observed_max_dep) = fit.parameters.iter().map(|p| p.dependency).filter(|d| d.is_finite()).reduce(f64::max)
        && observed_max_dep > max_dep
    {
        reasons.push(format!("Maximum parameter dependency = {:.6} exceeds threshold {:.6}.", observed_max_dep, max_dep));
    }
    if let Some(min_norm_p) = rules.min_normality_p {
        for norm in [&fit.normality, &fit.normality_anderson_darling, &fit.normality_lilliefors_ks].into_iter().flatten() {
            if norm.p_value.is_finite() && norm.p_value < min_norm_p {
                reasons.push(format!("Residual normality ({}) p = {:.4} is below threshold {:.4}.", norm.test_name, norm.p_value, min_norm_p));
            }
        }
    }
    if let Some(min_lof_p) = rules.min_runs_or_lof_p {
        if let Some(runs) = &fit.runs_test
            && runs.p_too_few_runs.is_finite()
            && runs.p_too_few_runs < min_lof_p
        {
            reasons.push(format!("Runs test p = {:.4} is below threshold {:.4}.", runs.p_too_few_runs, min_lof_p));
        }
        if let Some(lof) = &fit.lack_of_fit
            && lof.p_value.is_finite()
            && lof.p_value < min_lof_p
        {
            reasons.push(format!("Replicates lack-of-fit test p = {:.4} is below threshold {:.4}.", lof.p_value, min_lof_p));
        }
    }
    if let Some(max_out) = rules.max_rout_outliers {
        let count = fit.rout.as_ref().map_or(fit.n_outliers_excluded, |r| r.outlier_indices.len()).max(fit.n_outliers_excluded);
        if count > max_out {
            reasons.push(format!("ROUT outlier count = {} exceeds maximum threshold {}.", count, max_out));
        }
    }
    FitFlagOutcome { flagged: !reasons.is_empty(), reasons }
}

/// Determine whether a converged fit is clean/well-defined (neither `Ambiguous`, `Unstable`, nor
/// `"Flagged"` under optional [`FitFlagRules`]) for `prefer_well_defined_fit` model selection (§2.5.2).
#[must_use]
pub fn is_fit_well_defined(fit: &NonlinearFitResult, rules: Option<&FitFlagRules>) -> (bool, Vec<String>) {
    let mut issues = Vec::new();
    if fit.status == FitStatus::Ambiguous || fit.covariance.ambiguous {
        issues.push("Fit status is Ambiguous (severe multicollinearity or rank deficiency).".to_owned());
    }
    if let FitIdentifiability::Ambiguous { reasons } = &fit.identifiability {
        issues.extend(reasons.iter().cloned());
    }
    for p in &fit.parameters {
        if p.notes.contains(&ParameterNote::Unstable) {
            issues.push(format!("Parameter {} is Unstable.", p.name));
        } else if p.notes.contains(&ParameterNote::Ambiguous) {
            issues.push(format!("Parameter {} is Ambiguous.", p.name));
        }
    }
    if let Some(flag_rules) = rules {
        let flag_out = evaluate_fit_flags(fit, flag_rules);
        if flag_out.flagged {
            for r in flag_out.reasons {
                issues.push(format!("Flagged: {r}"));
            }
        }
    }
    (issues.is_empty(), issues)
}

#[derive(Default)]
pub(super) struct Assessment {
    pub(super) fit: FitIdentifiability,
    pub(super) unstable: Vec<bool>,
}

/// Combine independent assessments while keeping each ambiguous reason tied to its dataset.
pub(super) fn include_dataset(combined: &mut FitIdentifiability, dataset: FitIdentifiability, label: &str) {
    match dataset {
        FitIdentifiability::NotAssessed => {}
        FitIdentifiability::Identifiable => {
            if matches!(combined, FitIdentifiability::NotAssessed) {
                *combined = FitIdentifiability::Identifiable;
            }
        }
        FitIdentifiability::Ambiguous { reasons } => {
            let tagged = reasons.into_iter().map(|reason| format!("{label} — {reason}"));
            if let FitIdentifiability::Ambiguous { reasons } = combined {
                reasons.extend(tagged);
            } else {
                *combined = FitIdentifiability::Ambiguous { reasons: tagged.collect() };
            }
        }
    }
}

/// Assess free-parameter identifiability. See Seber & Wild (1989), chapter 3, for rank and
/// conditioning, and Venzon & Moolgavkar (1988), doi:10.2307/2347496, for profile limits.
/// The numeric warning thresholds are CurveBook's policy.
pub(super) fn assess(
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    jacobian: &DenseMatrix,
    df: usize,
    dependencies: &[f64],
    profile_cis: Option<&[(CiBound, CiBound)]>,
) -> Result<Assessment, FitError> {
    let mut reasons = Vec::new();
    let mut unstable = vec![false; layout.k_free()];
    if df == 0 {
        reasons.push("Residual degrees of freedom are zero.".to_owned());
        unstable.fill(true);
    }
    flag_sparse_datasets(layout, obs, &mut unstable, &mut reasons);

    if let Some((rcond, weak_slots)) = weak_jacobian_slots(jacobian)? {
        reasons.push(format!("Scaled Jacobian reciprocal condition number {rcond:.3e} is below {MIN_SCALED_RCOND:.0e}."));
        for idx in weak_slots {
            if let Some(mark) = unstable.get_mut(idx) {
                *mark = true;
            }
        }
    }

    for (i, (slot, mark)) in layout.free_slots.iter().zip(&mut unstable).enumerate() {
        if dependencies.get(i).is_some_and(|&d| d > MAX_DEPENDENCY) {
            reasons.push(format!("{} has variance inflation above 10,000.", slot.name));
            *mark = true;
        }
        if let Some(&(lower, upper)) = profile_cis.and_then(|cis| cis.get(i))
            && ((slot.lower.is_none() && matches!(lower, CiBound::Open)) || (slot.upper.is_none() && matches!(upper, CiBound::Open)))
        {
            reasons.push(format!("{} has an open profile-likelihood limit on an unconstrained side.", slot.name));
            *mark = true;
        }
    }

    let fit = if reasons.is_empty() { FitIdentifiability::Identifiable } else { FitIdentifiability::Ambiguous { reasons } };
    Ok(Assessment { fit, unstable })
}

/// Three-way identifiability assessment honoring [`IdentifiabilityMode`] (§2.5.1).
#[allow(clippy::too_many_arguments)]
pub(super) fn assess_for_mode(
    mode: IdentifiabilityMode,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    jacobian: &DenseMatrix,
    df: usize,
    dependencies: &[f64],
    profile_cis: Option<&[(CiBound, CiBound)]>,
    greedy: Option<&GreedyHessianReduction>,
    cov_ambiguous: bool,
) -> Result<Assessment, FitError> {
    let k_free = layout.k_free();
    match mode {
        IdentifiabilityMode::ReportAll => Ok(Assessment { fit: FitIdentifiability::NotAssessed, unstable: vec![false; k_free] }),
        IdentifiabilityMode::IdentifyAmbiguousFits => {
            let mut reasons = Vec::new();
            if df == 0 {
                reasons.push("Residual degrees of freedom are zero.".to_owned());
            }
            for (i, slot) in layout.free_slots.iter().enumerate() {
                if dependencies.get(i).is_some_and(|&d| d > MAX_DEPENDENCY) {
                    reasons.push(format!("{} has Dependency > 0.9999 (variance inflation above 10,000).", slot.name));
                }
            }
            if cov_ambiguous && reasons.is_empty() {
                reasons.push("Jacobian is rank-deficient or severely collinear.".to_owned());
            }
            let fit = if reasons.is_empty() { FitIdentifiability::Identifiable } else { FitIdentifiability::Ambiguous { reasons } };
            // Legacy Ambiguous mode prefixes tangled parameters with `~` (`ParameterNote::Ambiguous`),
            // never replacing them with `"Unstable"`.
            Ok(Assessment { fit, unstable: vec![false; k_free] })
        }
        IdentifiabilityMode::IdentifyUnstableParameters => {
            let mut base = assess(layout, obs, jacobian, df, dependencies, profile_cis)?;
            if let Some(red) = greedy
                && !red.unstable_indices.is_empty()
            {
                let threshold = hessian_kappa_threshold();
                let mut extra_reasons = Vec::new();
                for &(idx, kappa_before, kappa_after) in &red.steps {
                    if let Some(mark) = base.unstable.get_mut(idx) {
                        *mark = true;
                    }
                    let pname = layout.free_slots.get(idx).map_or("parameter", |s| s.name.as_str());
                    extra_reasons.push(format!(
                        "{pname} marked Unstable by greedy Hessian reduction (kappa(H) = {kappa_before:.3e} > {threshold:.3e}; reduced to {kappa_after:.3e})."
                    ));
                }
                match &mut base.fit {
                    FitIdentifiability::Ambiguous { reasons } => {
                        for r in extra_reasons {
                            reasons.push(r);
                        }
                    }
                    FitIdentifiability::Identifiable | FitIdentifiability::NotAssessed => {
                        base.fit = FitIdentifiability::Ambiguous { reasons: extra_reasons };
                    }
                }
            }
            Ok(base)
        }
    }
}

fn flag_sparse_datasets(layout: &ParameterLayout, obs: &[ObsRecord], unstable: &mut [bool], reasons: &mut Vec<String>) {
    for ds in 0..layout.n_datasets {
        let slots: Vec<_> = layout.free_slots.iter().zip(unstable.iter_mut()).filter(|(slot, _)| slot.ds_local_idx == Some(ds)).collect();
        let Some((slot, _)) = slots.first() else {
            continue;
        };
        let dataset_number = slot.dataset_idx.unwrap_or(ds).saturating_add(1);
        let mut unique = HashSet::with_capacity(slots.len());
        for rec in obs.iter().filter(|rec| rec.ds_local_idx == ds) {
            let coords = rec.point.x.iter().map(|&x| if x == 0.0 { 0 } else { x.to_bits() }).collect::<Vec<_>>();
            unique.insert(coords);
            if unique.len() >= slots.len() {
                break;
            }
        }
        if unique.len() < slots.len() {
            reasons.push(format!("Dataset {dataset_number} has {} distinct predictor values for {} local parameters.", unique.len(), slots.len()));
            for (_, mark) in slots {
                *mark = true;
            }
        }
    }
}

/// Normalize columns and use the singular values of pivoted QR's R factor. Multiplication by Q
/// preserves singular values (Golub & Van Loan, Matrix Computations, 4th ed., 2013).
/// Map weak right singular directions back through the column permutation.
fn weak_jacobian_slots(j: &DenseMatrix) -> Result<Option<(f64, Vec<usize>)>, FitError> {
    let (rows, cols) = (j.nrows(), j.ncols());
    if cols == 0 {
        return Ok(None);
    }
    if rows < cols {
        return Ok(Some((0.0, (0..cols).collect())));
    }
    let norms: Vec<_> = (0..cols).map(|i| j.col_norm2(i)).collect();
    let scaled = DenseMatrix::from_fn(rows, cols, |row, col| match (j.get(row, col), norms.get(col)) {
        (Some(value), Some(norm)) if *norm > 0.0 && norm.is_finite() => value / norm,
        _ => 0.0,
    })?;
    let qr = qr_col_pivoted(&scaled)?;
    let spectrum = svd(&qr.r()?)?;
    let Some(&largest) = spectrum.singular_values.first() else {
        return Ok(None);
    };
    let smallest = spectrum.singular_values.last().copied().unwrap_or(0.0);
    let rcond = if largest > 0.0 { smallest / largest } else { 0.0 };
    if rcond >= MIN_SCALED_RCOND {
        return Ok(None);
    }
    let weak = qr
        .permutation()
        .iter()
        .enumerate()
        .filter_map(|(pivot, &original)| {
            spectrum
                .singular_values
                .iter()
                .enumerate()
                .any(|(k, &sigma)| sigma <= largest * MIN_SCALED_RCOND && spectrum.v.get(pivot, k).is_some_and(|v| v.abs() > f64::EPSILON.sqrt()))
                .then_some(original)
        })
        .collect();
    Ok(Some((rcond, weak)))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{CiBound, WeightingScheme};
    use curvebook_math::ConfidenceLevel;

    use super::{DenseMatrix, ParameterLayout, assess, weak_jacobian_slots};
    use crate::diagnostics::compute_covariance_diagnostics;
    use crate::error::{FitIdentifiability, ParameterNote};
    use crate::library::get_builtin_model;
    use crate::solver::layout::build_observations;
    use crate::solver::{SolverConfig, fit_nonlinear};
    use crate::types::{DataPoint, DatasetSeries, HookContext};

    fn line_data(xs: &[f64]) -> DatasetSeries {
        DatasetSeries {
            data_set_index: 0,
            label: "Line".to_owned(),
            points: xs.iter().map(|&x| DataPoint::xy(x, 1.0 + x)).collect(),
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: None,
        }
    }

    #[test]
    fn incomplete_predictor_coverage_marks_local_slots() {
        let model = get_builtin_model("straight_line").unwrap();
        let ds = [line_data(&[-0.0, 0.0, 0.0])];
        let layout = ParameterLayout::build(model.as_ref(), &ds, &HookContext::default(), &[]).unwrap();
        let obs = build_observations(&ds, WeightingScheme::None, false).unwrap();
        // Use an orthogonal Jacobian to isolate the distinct-predictor criterion.
        let j = DenseMatrix::from_row_major(3, 2, vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
        let assessment = assess(&layout, &obs, &j, 1, &[0.0, 0.0], None).unwrap();
        assert!(assessment.fit.is_ambiguous());
        assert_eq!(assessment.unstable, [true, true]);
    }

    #[test]
    fn variance_inflation_oracle_flags_well_above_threshold() {
        let model = get_builtin_model("straight_line").unwrap();
        let ds = [line_data(&[0.0, 1.0, 2.0])];
        let layout = ParameterLayout::build(model.as_ref(), &ds, &HookContext::default(), &[]).unwrap();
        let obs = build_observations(&ds, WeightingScheme::None, false).unwrap();
        // For columns [1,0,0] and [1,e,0], VIF = 1 + 1/e². At e=0.005, VIF=40,001
        // although rcond is comfortably above 1e-12 (Motulsky & Christopoulos, 2004).
        let j = DenseMatrix::from_row_major(3, 2, vec![1.0, 1.0, 0.0, 0.005, 0.0, 0.0]).unwrap();
        assert!(weak_jacobian_slots(&j).unwrap().is_none());
        let cov = compute_covariance_diagnostics(&j, 1.0, 1, &[1.0, 1.0], ConfidenceLevel::DEFAULT_95).unwrap();
        assert!(cov.dependencies.iter().all(|&d| ((1.0 - d).recip() - 40_001.0).abs() < 1e-4));
        let assessment = assess(&layout, &obs, &j, 1, &cov.dependencies, None).unwrap();
        assert!(assessment.fit.is_ambiguous());
        assert_eq!(assessment.unstable, [true, true]);

        let mut layout = layout;
        layout.free_slots[0].lower = Some(0.0);
        let orthogonal = DenseMatrix::from_row_major(3, 2, vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
        let profile = [(CiBound::Open, CiBound::Finite(2.0)), (CiBound::Finite(0.0), CiBound::Open)];
        let assessment = assess(&layout, &obs, &orthogonal, 1, &[0.0, 0.0], Some(&profile)).unwrap();
        assert!(assessment.fit.is_ambiguous());
        assert_eq!(assessment.unstable, [false, true]);
    }

    #[test]
    fn independent_fits_keep_unstable_flags_with_their_dataset() {
        let model = get_builtin_model("straight_line").unwrap();
        let mut saturated = line_data(&[0.0, 1.0]);
        saturated.label = "Two observations".to_owned();
        let mut supported = line_data(&[0.0, 1.0, 2.0, 3.0]);
        supported.data_set_index = 1;
        supported.label = "Four observations".to_owned();
        let config = SolverConfig { identify_unstable_parameters: true, compute_profile_ci: false, ..SolverConfig::default() };

        let fit = fit_nonlinear(model.as_ref(), &[saturated, supported], &HookContext::default(), &config).unwrap();
        let FitIdentifiability::Ambiguous { reasons } = fit.identifiability else {
            panic!("expected ambiguity for the saturated dataset");
        };
        assert!(reasons.iter().any(|reason| reason.starts_with("Two observations") && reason.contains("degrees of freedom")));
        assert!(reasons.iter().all(|reason| !reason.starts_with("Four observations")));
        assert_eq!(fit.parameters.len(), 4);
        for parameter in &fit.parameters {
            assert_eq!(parameter.notes.contains(&ParameterNote::Unstable), parameter.dataset_index == Some(0));
        }
    }

    #[test]
    fn scaled_jacobian_condition_matches_two_column_oracle() {
        // Normalized [1, 0] and [1, e] have singular values sqrt(1 +/- 1/sqrt(1+e^2)).
        // Rationalizing their ratio gives rcond = e / (sqrt(1+e^2)+1).
        let e = 1e-13;
        let j = DenseMatrix::from_row_major(3, 2, vec![1.0, 1e8, 0.0, e * 1e8, 0.0, 0.0]).unwrap();
        let (rcond, weak) = weak_jacobian_slots(&j).unwrap().unwrap();
        let expected = e / (1.0_f64.hypot(e) + 1.0);
        assert!((rcond / expected - 1.0).abs() < 1e-8, "{rcond} vs {expected}");
        assert_eq!(weak.len(), 2);
    }
}
