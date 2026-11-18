use std::collections::HashSet;

use curvebook_doc::CiBound;
use curvebook_math::linalg::{DenseMatrix, qr_col_pivoted, svd};

use super::{ObsRecord, ParameterLayout};
use crate::error::{FitError, FitIdentifiability};

const MIN_SCALED_RCOND: f64 = 1e-12;
// Dependency = 1 - 1 / VIF (Motulsky & Christopoulos, 2004).
const MAX_DEPENDENCY: f64 = 1.0 - 1.0 / 10_000.0;

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
    use crate::solver::{SolverConfig, build_observations, fit_nonlinear};
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
