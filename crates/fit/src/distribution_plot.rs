//! Theoretical continuous and discrete probability distribution plotting (`Plot a function` §6.2).
//!
//! Evaluates continuous PDFs/CDFs (`Normal`, `StudentT`, `FisherF`, `ChiSquare`) with optional
//! multi-dataset tail/interval shading masks, and exact discrete integer-step PMFs/CDFs/survival
//! probabilities (`Binomial`, `Poisson`), resolving dynamic [`curvebook_doc::HookSource`] overrides
//! via [`HookContext`].
//!
//! # References
//!
//! - Abramowitz, M. & Stegun, I. A. (eds.) (1964). *Handbook of Mathematical Functions with
//!   Formulas, Graphs, and Mathematical Tables*. National Bureau of Standards Applied Mathematics
//!   Series 55. U.S. Government Printing Office.
//! - Johnson, N. L., Kotz, S. & Balakrishnan, N. (1994). *Continuous Univariate Distributions*,
//!   Vol. 1, 2nd ed. Wiley.
//! - Johnson, N. L., Kotz, S. & Balakrishnan, N. (1995). *Continuous Univariate Distributions*,
//!   Vol. 2, 2nd ed. Wiley.
//! - Olver, F. W. J. et al. (eds.) (2010). *NIST Handbook of Mathematical Functions* (DLMF).
//!   Cambridge University Press.

use curvebook_doc::result_builder::{KvTabBuilder, TabBuilder, num_cell};
use curvebook_doc::{
    Cell, CellCoord, CurveSamplePoint, DataSheet, DistributionTailShade, FittedCurveOverlay, ResultScalarEntry, ResultSheet, ResultStatus,
    ResultTabKind, SubcolumnMode, TableArchetype, TheoreticalDistributionFamily, TheoreticalDistributionSpec, XyTable,
};
use curvebook_math::dist::{
    chi2_cdf, chi2_isf, chi2_pdf, chi2_sf, f_cdf, f_isf, f_pdf, f_sf, norm_cdf, norm_pdf, norm_sf, student_t_cdf, student_t_pdf, student_t_sf,
};
use curvebook_math::special::{ln_choose, ln_factorial, reg_beta, reg_gamma_p, reg_gamma_q};

use crate::error::FitError;
use crate::types::HookContext;

/// Hard cap on generated grid points (continuous or discrete) for safety against pathological ranges.
const MAX_DISTRIBUTION_GRID_POINTS: usize = 100_001;

/// Largest exact IEEE-754 integer (`2^53`), safely below `u64::MAX` so discrete grid bounds
/// and `k + 1` evaluations never overflow or saturate.
const MAX_EXACT_DISCRETE_K_F64: f64 = 9_007_199_254_740_992.0;

/// One shaded tail or central-interval curve series emitted alongside the main theoretical curve.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadedDistributionSeries {
    /// Human-readable label for this shaded region (for example `"Lower tail"`, `"Upper tail"`, or `"Between"`).
    pub label: String,
    /// One-based overlay dataset index (`1`, `2`, ...) distinct from the main curve (`0`).
    pub data_set_index: usize,
    /// Shaded curve values aligned with [`TheoreticalDistributionPlot::x`] (`y[i]` inside the shaded region, `0.0` outside).
    pub y: Vec<f64>,
    /// Exact theoretical probability mass/area of the shaded region in `[0, 1]`.
    pub area: f64,
}

/// Evaluated theoretical probability distribution grid, tail-shading overlays, and cumulative/survival tables (§6.2).
#[derive(Debug, Clone, PartialEq)]
pub struct TheoreticalDistributionPlot {
    /// Probability distribution family evaluated.
    pub family: TheoreticalDistributionFamily,
    /// Canonical parameter names for [`Self::family`].
    pub parameter_names: Vec<&'static str>,
    /// Resolved finite distribution parameters after applying any [`curvebook_doc::HookSource`] overrides.
    pub resolved_parameters: Vec<f64>,
    /// `true` when [`Self::y`] stores the CDF `P(X <= x)` instead of the PDF/PMF.
    pub cumulative: bool,
    /// `true` for discrete integer-step distributions (`Binomial`, `Poisson`), `false` for continuous families.
    pub is_discrete: bool,
    /// Evaluation abscissae (`x` for continuous families, integer `k` cast to `f64` for discrete families).
    pub x: Vec<f64>,
    /// Primary curve ordinates (`PDF`/`PMF` when `cumulative == false`, `CDF` when `cumulative == true`).
    pub y: Vec<f64>,
    /// Cumulative lower-tail probabilities `P(X <= x)` aligned with [`Self::x`].
    pub cdf_le: Vec<f64>,
    /// Upper-tail probabilities (`P(X >= k)` for discrete families, `P(X > x)` for continuous families).
    pub sf_ge: Vec<f64>,
    /// Optional multi-dataset shaded tail or central-interval series.
    pub shaded_regions: Vec<ShadedDistributionSeries>,
    /// Ready-to-render curve overlays (`data_set_index = 0` for the main curve, `1..` for shaded regions).
    pub overlays: Vec<FittedCurveOverlay>,
}

fn family_meta(family: TheoreticalDistributionFamily) -> (&'static str, &'static str, &'static [&'static str], bool) {
    match family {
        TheoreticalDistributionFamily::Normal => ("dist_normal", "Normal distribution", &["Mean", "SD"], false),
        TheoreticalDistributionFamily::StudentT => ("dist_student_t", "Student's t distribution", &["DF"], false),
        TheoreticalDistributionFamily::FisherF => ("dist_fisher_f", "Fisher–Snedecor F distribution", &["DF1", "DF2"], false),
        TheoreticalDistributionFamily::ChiSquare => ("dist_chi_square", "Chi-square distribution", &["DF"], false),
        TheoreticalDistributionFamily::Binomial => ("dist_binomial", "Binomial distribution", &["N", "p"], true),
        TheoreticalDistributionFamily::Poisson => ("dist_poisson", "Poisson distribution", &["Lambda"], true),
    }
}

fn resolve_distribution_params(spec: &TheoreticalDistributionSpec, hooks: &HookContext) -> Result<Vec<f64>, FitError> {
    let (_, _, param_names, _) = family_meta(spec.family);
    let expected = param_names.len();
    let mut resolved = Vec::with_capacity(expected);
    for (idx, &pname) in param_names.iter().enumerate() {
        let val = if let Some(Some(hook)) = spec.parameter_hooks.get(idx) {
            hooks.resolve(hook)?
        } else if let Some(&v) = spec.parameters.get(idx) {
            v
        } else if spec.family == TheoreticalDistributionFamily::Normal && spec.parameters.is_empty() && spec.parameter_hooks.is_empty() {
            if idx == 0 { 0.0 } else { 1.0 }
        } else {
            return Err(FitError::InvalidParameter {
                name: pname,
                reason: format!("missing parameter `{pname}` (expected {expected} parameter(s))"),
            });
        };
        if !val.is_finite() {
            return Err(FitError::InvalidParameter { name: pname, reason: "distribution parameter must be finite".to_owned() });
        }
        resolved.push(val);
    }

    match spec.family {
        TheoreticalDistributionFamily::Normal => {
            if resolved[1] <= 0.0 {
                return Err(FitError::InvalidParameter { name: "SD", reason: "must be strictly positive".to_owned() });
            }
        }
        TheoreticalDistributionFamily::StudentT | TheoreticalDistributionFamily::ChiSquare => {
            if resolved[0] <= 0.0 {
                return Err(FitError::InvalidParameter { name: "DF", reason: "must be strictly positive".to_owned() });
            }
        }
        TheoreticalDistributionFamily::FisherF => {
            if resolved[0] <= 0.0 {
                return Err(FitError::InvalidParameter { name: "DF1", reason: "must be strictly positive".to_owned() });
            }
            if resolved[1] <= 0.0 {
                return Err(FitError::InvalidParameter { name: "DF2", reason: "must be strictly positive".to_owned() });
            }
        }
        TheoreticalDistributionFamily::Binomial => {
            let n = resolved[0];
            let p = resolved[1];
            if n < 0.0 || n > (MAX_DISTRIBUTION_GRID_POINTS - 1) as f64 || (n - n.round()).abs() > 1e-9 {
                return Err(FitError::InvalidParameter {
                    name: "N",
                    reason: format!("must be a non-negative integer <= {}", MAX_DISTRIBUTION_GRID_POINTS - 1),
                });
            }
            if !(0.0..=1.0).contains(&p) {
                return Err(FitError::InvalidParameter { name: "p", reason: "must lie in [0, 1]".to_owned() });
            }
            resolved[0] = n.round();
        }
        TheoreticalDistributionFamily::Poisson => {
            if resolved[0] <= 0.0 {
                return Err(FitError::InvalidParameter { name: "Lambda", reason: "must be strictly positive".to_owned() });
            }
        }
    }
    Ok(resolved)
}

fn validate_tail_shade(shade: DistributionTailShade) -> Result<(), FitError> {
    match shade {
        DistributionTailShade::None => Ok(()),
        DistributionTailShade::LowerTail { cutoff } | DistributionTailShade::UpperTail { cutoff } => {
            if !cutoff.is_finite() {
                return Err(FitError::InvalidParameter { name: "tail_shade.cutoff", reason: "must be finite".to_owned() });
            }
            Ok(())
        }
        DistributionTailShade::TwoTailed { lower, upper } | DistributionTailShade::Between { lower, upper } => {
            if !lower.is_finite() || !upper.is_finite() || lower > upper {
                return Err(FitError::InvalidParameter {
                    name: "tail_shade",
                    reason: "lower and upper cutoffs must be finite with lower <= upper".to_owned(),
                });
            }
            Ok(())
        }
    }
}

fn eval_continuous_point(family: TheoreticalDistributionFamily, p: &[f64], x: f64) -> (f64, f64, f64) {
    match family {
        TheoreticalDistributionFamily::Normal => {
            let z = (x - p[0]) / p[1];
            let pdf = norm_pdf(z) / p[1];
            let cdf = norm_cdf(z);
            let sf = norm_sf(z);
            (pdf, cdf, sf)
        }
        TheoreticalDistributionFamily::StudentT => {
            let df = p[0];
            let pdf = student_t_pdf(x, df).unwrap_or(0.0);
            let cdf = student_t_cdf(x, df).unwrap_or(0.0);
            let sf = student_t_sf(x, df).unwrap_or(0.0);
            (pdf, cdf, sf)
        }
        TheoreticalDistributionFamily::FisherF => {
            let (df1, df2) = (p[0], p[1]);
            if x < 0.0 {
                (0.0, 0.0, 1.0)
            } else if x == 0.0 {
                let pdf = if df1 > 2.0 { 0.0 } else { f_pdf(0.0, df1, df2).ok().filter(|v| v.is_finite()).unwrap_or(0.0) };
                (pdf, 0.0, 1.0)
            } else {
                let pdf = f_pdf(x, df1, df2).ok().filter(|v| v.is_finite()).unwrap_or(0.0);
                let cdf = f_cdf(x, df1, df2).unwrap_or(0.0);
                let sf = f_sf(x, df1, df2).unwrap_or(0.0);
                (pdf, cdf, sf)
            }
        }
        TheoreticalDistributionFamily::ChiSquare => {
            let df = p[0];
            if x < 0.0 {
                (0.0, 0.0, 1.0)
            } else if x == 0.0 {
                let pdf = if df > 2.0 {
                    0.0
                } else if (df - 2.0).abs() <= 1e-12 {
                    0.5
                } else {
                    chi2_pdf(0.0, df).ok().filter(|v| v.is_finite()).unwrap_or(0.0)
                };
                (pdf, 0.0, 1.0)
            } else {
                let pdf = chi2_pdf(x, df).ok().filter(|v| v.is_finite()).unwrap_or(0.0);
                let cdf = chi2_cdf(x, df).unwrap_or(0.0);
                let sf = chi2_sf(x, df).unwrap_or(0.0);
                (pdf, cdf, sf)
            }
        }
        TheoreticalDistributionFamily::Binomial | TheoreticalDistributionFamily::Poisson => (0.0, 0.0, 0.0),
    }
}

fn eval_binomial_point(n: u64, p: f64, k: u64) -> (f64, f64, f64) {
    if k > n {
        return (0.0, 1.0, 0.0);
    }
    let pmf = if p == 0.0 {
        if k == 0 { 1.0 } else { 0.0 }
    } else if p == 1.0 {
        if k == n { 1.0 } else { 0.0 }
    } else {
        let log_pmf = ln_choose(n, k).unwrap_or(f64::NEG_INFINITY) + (k as f64) * p.ln() + ((n - k) as f64) * (-p).ln_1p();
        if log_pmf < -745.133_219_101_941_2 { 0.0 } else { log_pmf.exp() }
    };
    let cdf_le = if k >= n || p == 0.0 {
        1.0
    } else if p == 1.0 {
        0.0
    } else {
        reg_beta(1.0 - p, (n - k) as f64, (k + 1) as f64).unwrap_or(0.0).clamp(0.0, 1.0)
    };
    let sf_ge = if k == 0 {
        1.0
    } else if p == 0.0 {
        0.0
    } else if p == 1.0 {
        1.0
    } else {
        reg_beta(p, k as f64, (n - k + 1) as f64).unwrap_or(0.0).clamp(0.0, 1.0)
    };
    (pmf, cdf_le, sf_ge)
}

fn eval_poisson_point(lambda: f64, k: u64) -> (f64, f64, f64) {
    if k == u64::MAX {
        return (0.0, 1.0, 0.0);
    }
    let kf = k as f64;
    let log_pmf = if k == 0 { -lambda } else { kf * lambda.ln() - lambda - ln_factorial(k) };
    let pmf = if !log_pmf.is_finite() || log_pmf < -745.133_219_101_941_2 { 0.0 } else { log_pmf.exp() };
    let cdf_le = reg_gamma_q(kf + 1.0, lambda).unwrap_or(0.0).clamp(0.0, 1.0);
    let sf_ge = if k == 0 { 1.0 } else { reg_gamma_p(kf, lambda).unwrap_or(0.0).clamp(0.0, 1.0) };
    (pmf, cdf_le, sf_ge)
}

fn eval_cdf_at(family: TheoreticalDistributionFamily, p: &[f64], x: f64) -> f64 {
    match family {
        TheoreticalDistributionFamily::Binomial => {
            if x < 0.0 {
                0.0
            } else {
                let n = p[0] as u64;
                let xf = x.floor();
                if xf >= n as f64 { 1.0 } else { eval_binomial_point(n, p[1], xf as u64).1 }
            }
        }
        TheoreticalDistributionFamily::Poisson => {
            if x < 0.0 {
                0.0
            } else {
                let lambda = p[0];
                let xf = x.floor();
                if xf >= MAX_EXACT_DISCRETE_K_F64 {
                    let spread = 40.0 * lambda.sqrt().max(1.0);
                    if !xf.is_finite() || xf > lambda + spread {
                        1.0
                    } else if xf < lambda - spread {
                        0.0
                    } else {
                        reg_gamma_q(xf + 1.0, lambda).unwrap_or(1.0).clamp(0.0, 1.0)
                    }
                } else {
                    eval_poisson_point(lambda, xf as u64).1
                }
            }
        }
        _ => eval_continuous_point(family, p, x).1,
    }
}

fn eval_sf_at(family: TheoreticalDistributionFamily, p: &[f64], x: f64) -> f64 {
    match family {
        TheoreticalDistributionFamily::Binomial => {
            if x <= 0.0 {
                1.0
            } else {
                let n = p[0] as u64;
                let xc = x.ceil();
                if xc > n as f64 { 0.0 } else { eval_binomial_point(n, p[1], xc as u64).2 }
            }
        }
        TheoreticalDistributionFamily::Poisson => {
            if x <= 0.0 {
                1.0
            } else {
                let lambda = p[0];
                let xc = x.ceil();
                if xc >= MAX_EXACT_DISCRETE_K_F64 {
                    let spread = 40.0 * lambda.sqrt().max(1.0);
                    if !xc.is_finite() || xc > lambda + spread {
                        0.0
                    } else if xc < lambda - spread {
                        1.0
                    } else {
                        reg_gamma_p(xc, lambda).unwrap_or(0.0).clamp(0.0, 1.0)
                    }
                } else {
                    eval_poisson_point(lambda, xc as u64).2
                }
            }
        }
        _ => eval_continuous_point(family, p, x).2,
    }
}

fn build_shaded_regions(
    family: TheoreticalDistributionFamily,
    params: &[f64],
    shade: DistributionTailShade,
    xs: &[f64],
    ys: &[f64],
) -> Vec<ShadedDistributionSeries> {
    match shade {
        DistributionTailShade::None => Vec::new(),
        DistributionTailShade::LowerTail { cutoff } => {
            let masked = xs.iter().zip(ys).map(|(&x, &y)| if x <= cutoff { y } else { 0.0 }).collect();
            let area = eval_cdf_at(family, params, cutoff).clamp(0.0, 1.0);
            vec![ShadedDistributionSeries { label: "Lower tail".to_owned(), data_set_index: 1, y: masked, area }]
        }
        DistributionTailShade::UpperTail { cutoff } => {
            let masked = xs.iter().zip(ys).map(|(&x, &y)| if x >= cutoff { y } else { 0.0 }).collect();
            let area = eval_sf_at(family, params, cutoff).clamp(0.0, 1.0);
            vec![ShadedDistributionSeries { label: "Upper tail".to_owned(), data_set_index: 1, y: masked, area }]
        }
        DistributionTailShade::TwoTailed { lower, upper } => {
            let left_y = xs.iter().zip(ys).map(|(&x, &y)| if x <= lower { y } else { 0.0 }).collect();
            let right_y = xs.iter().zip(ys).map(|(&x, &y)| if x >= upper { y } else { 0.0 }).collect();
            let left_area = eval_cdf_at(family, params, lower).clamp(0.0, 1.0);
            let right_area = eval_sf_at(family, params, upper).clamp(0.0, 1.0);
            vec![
                ShadedDistributionSeries { label: "Lower tail".to_owned(), data_set_index: 1, y: left_y, area: left_area },
                ShadedDistributionSeries { label: "Upper tail".to_owned(), data_set_index: 2, y: right_y, area: right_area },
            ]
        }
        DistributionTailShade::Between { lower, upper } => {
            let masked = xs.iter().zip(ys).map(|(&x, &y)| if x >= lower && x <= upper { y } else { 0.0 }).collect();
            let area = match family {
                TheoreticalDistributionFamily::Binomial | TheoreticalDistributionFamily::Poisson => {
                    let k_lo = lower.ceil();
                    let k_hi = upper.floor();
                    if k_hi < 0.0 || k_lo > k_hi {
                        0.0
                    } else if k_lo <= 0.0 {
                        eval_cdf_at(family, params, k_hi).clamp(0.0, 1.0)
                    } else {
                        (eval_cdf_at(family, params, k_hi) - eval_cdf_at(family, params, k_lo - 1.0)).clamp(0.0, 1.0)
                    }
                }
                _ => (eval_cdf_at(family, params, upper) - eval_cdf_at(family, params, lower)).clamp(0.0, 1.0),
            };
            vec![ShadedDistributionSeries { label: "Between".to_owned(), data_set_index: 1, y: masked, area }]
        }
    }
}

fn default_continuous_range(family: TheoreticalDistributionFamily, p: &[f64]) -> (f64, f64) {
    match family {
        TheoreticalDistributionFamily::Normal => (p[0] - 4.0 * p[1], p[0] + 4.0 * p[1]),
        TheoreticalDistributionFamily::StudentT => (-4.0, 4.0),
        TheoreticalDistributionFamily::FisherF => {
            let hi = f_isf(0.001, p[0], p[1]).unwrap_or(6.0).clamp(6.0, 50.0);
            (0.0, hi)
        }
        TheoreticalDistributionFamily::ChiSquare => {
            let df = p[0];
            let fallback = (df + 5.0 * (2.0 * df).sqrt()).max(10.0);
            let hi = chi2_isf(0.001, df).unwrap_or(fallback).clamp(10.0, 200.0);
            (0.0, hi)
        }
        TheoreticalDistributionFamily::Binomial | TheoreticalDistributionFamily::Poisson => (0.0, 10.0),
    }
}

fn checked_discrete_range(
    plot_x_min: Option<f64>,
    plot_x_max: Option<f64>,
    default_max: impl FnOnce() -> Result<u64, FitError>,
    upper_cap: Option<u64>,
) -> Result<(u64, u64, usize), FitError> {
    let k_min = match plot_x_min {
        Some(v) => {
            if !v.is_finite() || v > MAX_EXACT_DISCRETE_K_F64 {
                return Err(FitError::InvalidParameter {
                    name: "plot_x_min",
                    reason: "must be finite and <= 2^53 for discrete distributions".to_owned(),
                });
            }
            v.ceil().max(0.0) as u64
        }
        None => 0,
    };
    let k_max = match plot_x_max {
        Some(v) => {
            if !v.is_finite() || v > MAX_EXACT_DISCRETE_K_F64 {
                return Err(FitError::InvalidParameter {
                    name: "plot_x_max",
                    reason: "must be finite and <= 2^53 for discrete distributions".to_owned(),
                });
            }
            if v < 0.0 {
                return Err(FitError::InvalidParameter { name: "plot_x_max", reason: "must be >= 0 for discrete distributions".to_owned() });
            }
            let raw = v.floor() as u64;
            upper_cap.map_or(raw, |cap| raw.min(cap))
        }
        None => default_max()?,
    };
    if k_min > k_max {
        return Err(FitError::InvalidParameter { name: "plot_x_min", reason: "must be <= plot_x_max".to_owned() });
    }
    let count_u64 = k_max.checked_sub(k_min).and_then(|d| d.checked_add(1));
    let Some(count_u64) = count_u64.filter(|&c| c <= MAX_DISTRIBUTION_GRID_POINTS as u64) else {
        return Err(FitError::InvalidParameter {
            name: "plot_x_max",
            reason: format!("discrete grid exceeds {MAX_DISTRIBUTION_GRID_POINTS} points"),
        });
    };
    Ok((k_min, k_max, count_u64 as usize))
}

fn derived_curve_sheet_id(base_id: &str) -> String {
    const SUFFIX: &str = "_curve";
    let max_prefix = curvebook_doc::error::MAX_SHEET_ID_BYTES.saturating_sub(SUFFIX.len());
    let mut end = base_id.len().min(max_prefix);
    while end > 0 && !base_id.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = base_id.get(..end).filter(|s| !s.is_empty()).unwrap_or("dist");
    format!("{prefix}{SUFFIX}")
}

/// Evaluate a theoretical probability distribution specification (§6.2) into grid points,
/// cumulative/survival probabilities, and optional multi-dataset tail-shading overlays.
///
/// For continuous distributions (`Normal`, `StudentT`, `FisherF`, `ChiSquare`), generates
/// `segments + 1` evenly spaced points across `[plot_x_min, plot_x_max]` (or a family-appropriate
/// default range). For discrete distributions (`Binomial`, `Poisson`), generates an integer-step
/// grid `k = k_min..=k_max`.
pub fn evaluate_theoretical_distribution(
    spec: &TheoreticalDistributionSpec,
    hooks: &HookContext,
    plot_x_min: Option<f64>,
    plot_x_max: Option<f64>,
    segments: usize,
) -> Result<TheoreticalDistributionPlot, FitError> {
    let (eq_id, _, param_names, is_discrete) = family_meta(spec.family);
    let resolved_parameters = resolve_distribution_params(spec, hooks)?;
    validate_tail_shade(spec.tail_shade)?;

    let (x, y, cdf_le, sf_ge) = if is_discrete {
        match spec.family {
            TheoreticalDistributionFamily::Binomial => {
                let n = resolved_parameters[0] as u64;
                let p = resolved_parameters[1];
                let (k_min, k_max, count) = checked_discrete_range(plot_x_min, plot_x_max, || Ok(n), Some(n))?;
                let mut xs = Vec::with_capacity(count);
                let mut ys = Vec::with_capacity(count);
                let mut cdfs = Vec::with_capacity(count);
                let mut sfs = Vec::with_capacity(count);
                for k in k_min..=k_max {
                    let (pmf, cdf, sf) = eval_binomial_point(n, p, k);
                    xs.push(k as f64);
                    ys.push(if spec.cumulative { cdf } else { pmf });
                    cdfs.push(cdf);
                    sfs.push(sf);
                }
                (xs, ys, cdfs, sfs)
            }
            TheoreticalDistributionFamily::Poisson => {
                let lambda = resolved_parameters[0];
                let (k_min, k_max, count) = checked_discrete_range(
                    plot_x_min,
                    plot_x_max,
                    || {
                        let raw = (lambda + 6.0 * lambda.sqrt()).ceil().max(14.0);
                        if !raw.is_finite() || raw > MAX_EXACT_DISCRETE_K_F64 {
                            return Err(FitError::InvalidParameter {
                                name: "plot_x_max",
                                reason: "default discrete Poisson range exceeds safe integer ceiling".to_owned(),
                            });
                        }
                        Ok(raw as u64)
                    },
                    None,
                )?;
                let mut xs = Vec::with_capacity(count);
                let mut ys = Vec::with_capacity(count);
                let mut cdfs = Vec::with_capacity(count);
                let mut sfs = Vec::with_capacity(count);
                for k in k_min..=k_max {
                    let (pmf, cdf, sf) = eval_poisson_point(lambda, k);
                    xs.push(k as f64);
                    ys.push(if spec.cumulative { cdf } else { pmf });
                    cdfs.push(cdf);
                    sfs.push(sf);
                }
                (xs, ys, cdfs, sfs)
            }
            _ => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
        }
    } else {
        if segments == 0 || segments >= MAX_DISTRIBUTION_GRID_POINTS {
            return Err(FitError::InvalidParameter { name: "segments", reason: format!("must be in 1..={}", MAX_DISTRIBUTION_GRID_POINTS - 1) });
        }
        let (def_min, def_max) = default_continuous_range(spec.family, &resolved_parameters);
        let x_min = match plot_x_min {
            Some(v) if v.is_finite() => v,
            Some(_) => return Err(FitError::InvalidParameter { name: "plot_x_min", reason: "must be finite".to_owned() }),
            None => def_min,
        };
        let x_max = match plot_x_max {
            Some(v) if v.is_finite() => v,
            Some(_) => return Err(FitError::InvalidParameter { name: "plot_x_max", reason: "must be finite".to_owned() }),
            None => def_max,
        };
        if !x_min.is_finite() || !x_max.is_finite() {
            return Err(FitError::InvalidParameter {
                name: "plot_x_min",
                reason: "distribution plot range overflowed to a non-finite bound".to_owned(),
            });
        }
        if x_min >= x_max {
            return Err(FitError::InvalidParameter { name: "plot_x_min", reason: "must be strictly less than plot_x_max".to_owned() });
        }
        let span = x_max - x_min;
        if !span.is_finite() {
            return Err(FitError::InvalidParameter {
                name: "plot_x_max",
                reason: "distribution plot span (plot_x_max - plot_x_min) overflowed f64".to_owned(),
            });
        }
        let n_pts = segments + 1;
        let mut xs = Vec::with_capacity(n_pts);
        let mut ys = Vec::with_capacity(n_pts);
        let mut cdfs = Vec::with_capacity(n_pts);
        let mut sfs = Vec::with_capacity(n_pts);
        for i in 0..n_pts {
            let xi = if i == segments { x_max } else { x_min + span * ((i as f64) / (segments as f64)) };
            let (pdf, cdf, sf) = eval_continuous_point(spec.family, &resolved_parameters, xi);
            xs.push(xi);
            ys.push(if spec.cumulative { cdf } else { pdf });
            cdfs.push(cdf);
            sfs.push(sf);
        }
        (xs, ys, cdfs, sfs)
    };

    let shaded_regions = build_shaded_regions(spec.family, &resolved_parameters, spec.tail_shade, &x, &y);

    let mut overlays = Vec::with_capacity(1 + shaded_regions.len());
    let main_pts = x
        .iter()
        .zip(&y)
        .map(|(&xi, &yi)| CurveSamplePoint { x: xi, y: yi, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None })
        .collect();
    overlays.push(FittedCurveOverlay { data_set_index: 0, equation_id: eq_id.to_owned(), converged: true, ambiguous: false, points: main_pts });
    for region in &shaded_regions {
        let pts = x
            .iter()
            .zip(&region.y)
            .map(|(&xi, &yi)| CurveSamplePoint { x: xi, y: yi, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None })
            .collect();
        overlays.push(FittedCurveOverlay {
            data_set_index: region.data_set_index,
            equation_id: format!("{eq_id}_shade_{}", region.data_set_index),
            converged: true,
            ambiguous: false,
            points: pts,
        });
    }

    Ok(TheoreticalDistributionPlot {
        family: spec.family,
        parameter_names: param_names.to_vec(),
        resolved_parameters,
        cumulative: spec.cumulative,
        is_discrete,
        x,
        y,
        cdf_le,
        sf_ge,
        shaded_regions,
        overlays,
    })
}

/// Populate a [`ResultSheet`] with tabs, overlays, hookable scalars, and an analyzable coordinate
/// sheet from a evaluated [`TheoreticalDistributionPlot`].
pub(crate) fn populate_distribution_plot_result(result: &mut ResultSheet, spec: &TheoreticalDistributionSpec, plot: &TheoreticalDistributionPlot) {
    let (_, display_name, _, _) = family_meta(plot.family);
    result.fitted_curves.extend(plot.overlays.iter().cloned());

    let mut summary = KvTabBuilder::new("distribution_summary", "Distribution parameters", ResultTabKind::MainSummary, display_name);
    for (&pname, &pval) in plot.parameter_names.iter().zip(&plot.resolved_parameters) {
        summary.push_num(pname, pval);
        result.scalars.push(ResultScalarEntry { key: pname.to_owned(), data_set_index: Some(0), value: pval, units: None });
    }
    summary.push_num("Grid points", plot.x.len() as f64);
    for region in &plot.shaded_regions {
        summary.push_num(format!("{} area", region.label), region.area);
        result.scalars.push(ResultScalarEntry {
            key: format!("{}Area", region.label.replace(' ', "")),
            data_set_index: Some(region.data_set_index),
            value: region.area,
            units: None,
        });
    }
    match spec.tail_shade {
        DistributionTailShade::None => {}
        DistributionTailShade::LowerTail { cutoff } | DistributionTailShade::UpperTail { cutoff } => {
            summary.push_num("Cutoff", cutoff);
            result.scalars.push(ResultScalarEntry { key: "Cutoff".to_owned(), data_set_index: Some(0), value: cutoff, units: None });
        }
        DistributionTailShade::TwoTailed { lower, upper } | DistributionTailShade::Between { lower, upper } => {
            summary.push_num("Lower cutoff", lower).push_num("Upper cutoff", upper);
            result.scalars.push(ResultScalarEntry { key: "LowerCutoff".to_owned(), data_set_index: Some(0), value: lower, units: None });
            result.scalars.push(ResultScalarEntry { key: "UpperCutoff".to_owned(), data_set_index: Some(0), value: upper, units: None });
        }
    }
    result.tabs.push(summary.build());

    let y_col_hdr = if plot.cumulative {
        "CDF"
    } else if plot.is_discrete {
        "PMF"
    } else {
        "PDF"
    };
    let mut headers: Vec<String> = vec![
        if plot.is_discrete { "k".to_owned() } else { "X".to_owned() },
        y_col_hdr.to_owned(),
        "P(X <= x)".to_owned(),
        if plot.is_discrete { "P(X >= k)".to_owned() } else { "P(X > x)".to_owned() },
    ];
    for region in &plot.shaded_regions {
        headers.push(region.label.clone());
    }
    let mut coords =
        TabBuilder::new("distribution_coordinates", "Curve coordinates", ResultTabKind::CurveCoordinates).headers(headers.iter().map(String::as_str));
    for i in 0..plot.x.len() {
        let mut row = Vec::with_capacity(4 + plot.shaded_regions.len());
        row.push(num_cell(plot.x[i]));
        row.push(num_cell(plot.y[i]));
        row.push(num_cell(plot.cdf_le[i]));
        row.push(num_cell(plot.sf_ge[i]));
        for region in &plot.shaded_regions {
            row.push(num_cell(region.y[i]));
        }
        coords.push_row(format!("{}", i + 1), row);
    }
    result.tabs.push(coords.build());

    // Emit an analyzable XY DataSheet containing main curve + shaded tail columns.
    let n_cols = 1 + plot.shaded_regions.len();
    let n_rows = plot.x.len();
    if let Ok(mut xy) = XyTable::new(SubcolumnMode::Single, n_cols, n_rows) {
        xy.x_title = if plot.is_discrete { "k".to_owned() } else { "X".to_owned() };
        if let Some(ds0) = xy.data_sets.first_mut() {
            ds0.title = y_col_hdr.to_owned();
        }
        for (idx, region) in plot.shaded_regions.iter().enumerate() {
            if let Some(ds) = xy.data_sets.get_mut(idx + 1) {
                ds.title = region.label.clone();
            }
        }
        let curve_id = derived_curve_sheet_id(result.id.as_str());
        if let Ok(mut sheet) = DataSheet::new(curve_id, display_name, TableArchetype::Xy(xy)) {
            for r in 0..n_rows {
                let _ = sheet.set_x_or_time_cell(r, 0, Cell::number(plot.x[r]));
                let _ = sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(plot.y[r]));
                for (c_idx, region) in plot.shaded_regions.iter().enumerate() {
                    let _ = sheet.set_cell(CellCoord::new(r, c_idx + 1, 0), Cell::number(region.y[r]));
                }
            }
            result.analyzable_data = Some(sheet);
        }
    }

    result.status = ResultStatus::UpToDate;
}
