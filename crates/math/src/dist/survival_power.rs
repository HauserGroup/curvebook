//! Two-arm log-rank survival power, required-event, detectable hazard-ratio, and sample-size
//! primitives under proportional hazards (Schoenfeld 1983; Freedman 1982).
//!
//! Conventions:
//! - Allocation ratio `allocation_ratio` is `k = n_2 / n_1 > 0` (so arm proportions are
//!   `p_1 = 1 / (1 + k)` and `p_2 = k / (1 + k)`).
//! - `hazard_ratio` is `HR = h_2 / h_1 > 0` (`HR != 1.0`), the hazard of group 2 relative to
//!   group 1.
//! - Both Schoenfeld (1983) and Freedman (1982) event formulas are invariant under swapping arms
//!   `(k, HR) -> (1 / k, 1 / HR)`.
//!
//! References:
//! - Schoenfeld, D. A. (1983). Sample-size formula for the proportional-hazards regression model.
//!   *Biometrics*, 39(2), 499-503. `doi:10.2307/2531021`.
//! - Freedman, L. S. (1982). Tables of the number of patients required in clinical trials using
//!   the logrank test. *Statistics in Medicine*, 1(2), 121-129. `doi:10.1002/sim.4780010204`.
//! - Collett, D. (2015). *Modelling Survival Data in Medical Research* (3rd ed., Ch. 10).
//!   Chapman & Hall/CRC.

use crate::dist::core::{norm_cdf, norm_isf, norm_ppf, norm_two_sided_isf};
use crate::error::MathError;

fn validate_survival_common(alpha: f64, allocation_ratio: f64) -> Result<(), MathError> {
    if !alpha.is_finite() || alpha <= 0.0 || alpha >= 1.0 {
        return Err(MathError::InvalidParameter { name: "alpha", value: alpha });
    }
    if !allocation_ratio.is_finite() || allocation_ratio <= 0.0 {
        return Err(MathError::InvalidParameter { name: "allocation_ratio", value: allocation_ratio });
    }
    Ok(())
}

fn validate_hazard_ratio(hazard_ratio: f64) -> Result<(), MathError> {
    if !hazard_ratio.is_finite() || hazard_ratio <= 0.0 || (hazard_ratio - 1.0).abs() == 0.0 {
        return Err(MathError::InvalidParameter { name: "hazard_ratio", value: hazard_ratio });
    }
    Ok(())
}

fn validate_power(power: f64) -> Result<(), MathError> {
    if !power.is_finite() || power <= 0.0 || power >= 1.0 {
        return Err(MathError::InvalidParameter { name: "power", value: power });
    }
    Ok(())
}

fn validate_events(total_events: f64) -> Result<(), MathError> {
    if !total_events.is_finite() || total_events <= 0.0 {
        return Err(MathError::InvalidParameter { name: "total_events", value: total_events });
    }
    Ok(())
}

fn z_alpha_and_beta(alpha: f64, power: f64, two_sided: bool) -> Result<(f64, f64), MathError> {
    let z_a = if two_sided { norm_two_sided_isf(alpha)? } else { norm_isf(alpha)? };
    let z_b = norm_ppf(power)?;
    Ok((z_a, z_b))
}

fn normal_power_from_drift(drift: f64, alpha: f64, two_sided: bool) -> Result<f64, MathError> {
    let z_a = if two_sided { norm_two_sided_isf(alpha)? } else { norm_isf(alpha)? };
    if two_sided {
        // Two-sided normal rejection P(|Z| > z_{alpha/2}) for Z ~ N(drift, 1).
        Ok((norm_cdf(drift - z_a) + norm_cdf(-drift - z_a)).clamp(0.0, 1.0))
    } else {
        Ok(norm_cdf(drift - z_a).clamp(0.0, 1.0))
    }
}

/// Required total events `D` for a two-arm log-rank test under Schoenfeld's (1983) formula:
/// `D = ((1 + k)^2 / k) * (z_alpha + z_beta)^2 / (ln HR)^2`.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `hazard_ratio <= 0` or `== 1`, `alpha` or `power`
/// lies outside `(0, 1)`, or `allocation_ratio <= 0`.
pub fn schoenfeld_logrank_events(hazard_ratio: f64, alpha: f64, power: f64, allocation_ratio: f64, two_sided: bool) -> Result<f64, MathError> {
    validate_survival_common(alpha, allocation_ratio)?;
    validate_hazard_ratio(hazard_ratio)?;
    validate_power(power)?;
    let (z_a, z_b) = z_alpha_and_beta(alpha, power, two_sided)?;
    let sum_z = z_a + z_b;
    if sum_z <= 0.0 {
        return Err(MathError::InvalidParameter { name: "power", value: power });
    }
    let ln_hr = hazard_ratio.ln();
    let k = allocation_ratio;
    let inv_p1p2 = ((1.0 + k) * (1.0 + k)) / k;
    let events = inv_p1p2 * (sum_z * sum_z) / (ln_hr * ln_hr);
    if !events.is_finite() {
        return Err(MathError::DomainError { name: "hazard_ratio", value: hazard_ratio });
    }
    Ok(events)
}

/// Achieved statistical power `1 - beta` for a two-arm log-rank test given `total_events` under
/// Schoenfeld's (1983) approximation (`drift = sqrt(D * k) / (1 + k) * |ln HR|`).
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if any argument lies outside its valid domain.
pub fn schoenfeld_logrank_power(total_events: f64, hazard_ratio: f64, alpha: f64, allocation_ratio: f64, two_sided: bool) -> Result<f64, MathError> {
    validate_survival_common(alpha, allocation_ratio)?;
    validate_events(total_events)?;
    validate_hazard_ratio(hazard_ratio)?;
    let k = allocation_ratio;
    let drift = (total_events * k).sqrt() / (1.0 + k) * hazard_ratio.ln().abs();
    normal_power_from_drift(drift, alpha, two_sided)
}

/// Minimum detectable hazard ratio (`HR > 1` when `hr_greater_than_one == true`, else `0 < HR < 1`)
/// for a given `total_events` under Schoenfeld's (1983) formula.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if any argument lies outside its valid domain.
pub fn schoenfeld_logrank_detectable_hr(
    total_events: f64,
    alpha: f64,
    power: f64,
    allocation_ratio: f64,
    two_sided: bool,
    hr_greater_than_one: bool,
) -> Result<f64, MathError> {
    validate_survival_common(alpha, allocation_ratio)?;
    validate_events(total_events)?;
    validate_power(power)?;
    let (z_a, z_b) = z_alpha_and_beta(alpha, power, two_sided)?;
    let sum_z = z_a + z_b;
    if sum_z <= 0.0 {
        return Err(MathError::InvalidParameter { name: "power", value: power });
    }
    let k = allocation_ratio;
    let abs_ln_hr = sum_z * (1.0 + k) / (total_events * k).sqrt();
    let hr = if hr_greater_than_one { abs_ln_hr.exp() } else { (-abs_ln_hr).exp() };
    if !hr.is_finite() || hr <= 0.0 {
        return Err(MathError::DomainError { name: "total_events", value: total_events });
    }
    Ok(hr)
}

/// Required total events `D` for a two-arm log-rank test under Freedman's (1982) formula:
/// `D = (z_alpha + z_beta)^2 * (1 + k * HR)^2 / (k * (HR - 1)^2)` (Freedman 1982; Collett 2015).
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `hazard_ratio <= 0` or `== 1`, `alpha` or `power`
/// lies outside `(0, 1)`, or `allocation_ratio <= 0`.
pub fn freedman_logrank_events(hazard_ratio: f64, alpha: f64, power: f64, allocation_ratio: f64, two_sided: bool) -> Result<f64, MathError> {
    validate_survival_common(alpha, allocation_ratio)?;
    validate_hazard_ratio(hazard_ratio)?;
    validate_power(power)?;
    let (z_a, z_b) = z_alpha_and_beta(alpha, power, two_sided)?;
    let sum_z = z_a + z_b;
    if sum_z <= 0.0 {
        return Err(MathError::InvalidParameter { name: "power", value: power });
    }
    let k = allocation_ratio;
    let ratio = (1.0 + k * hazard_ratio) / (hazard_ratio - 1.0);
    let events = (sum_z * sum_z) * (ratio * ratio) / k;
    if !events.is_finite() {
        return Err(MathError::DomainError { name: "hazard_ratio", value: hazard_ratio });
    }
    Ok(events)
}

/// Achieved statistical power `1 - beta` for a two-arm log-rank test given `total_events` under
/// Freedman's (1982) approximation (`drift = sqrt(D * k) * |HR - 1| / (1 + k * HR)`).
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if any argument lies outside its valid domain.
pub fn freedman_logrank_power(total_events: f64, hazard_ratio: f64, alpha: f64, allocation_ratio: f64, two_sided: bool) -> Result<f64, MathError> {
    validate_survival_common(alpha, allocation_ratio)?;
    validate_events(total_events)?;
    validate_hazard_ratio(hazard_ratio)?;
    let k = allocation_ratio;
    let drift = (total_events * k).sqrt() * (hazard_ratio - 1.0).abs() / (1.0 + k * hazard_ratio);
    normal_power_from_drift(drift, alpha, two_sided)
}

/// Minimum detectable hazard ratio (`HR > 1` when `hr_greater_than_one == true`, else `0 < HR < 1`)
/// for a given `total_events` under Freedman's (1982) formula.
///
/// Note that for `HR > 1`, Freedman's drift is bounded above by `sqrt(D / k)`, so when
/// `(z_alpha + z_beta) / sqrt(D * k) >= 1 / k` no finite `HR > 1` attains `power` and
/// [`MathError::DomainError`] is returned.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if any input lies outside its valid domain, or
/// [`MathError::DomainError`] if `total_events` is too small for any finite detectable HR.
pub fn freedman_logrank_detectable_hr(
    total_events: f64,
    alpha: f64,
    power: f64,
    allocation_ratio: f64,
    two_sided: bool,
    hr_greater_than_one: bool,
) -> Result<f64, MathError> {
    validate_survival_common(alpha, allocation_ratio)?;
    validate_events(total_events)?;
    validate_power(power)?;
    let (z_a, z_b) = z_alpha_and_beta(alpha, power, two_sided)?;
    let sum_z = z_a + z_b;
    if sum_z <= 0.0 {
        return Err(MathError::InvalidParameter { name: "power", value: power });
    }
    let k = allocation_ratio;
    let s = sum_z / (total_events * k).sqrt();
    // Solve |HR - 1| / (1 + k * HR) = s:
    // If HR > 1: HR - 1 = s * (1 + k * HR) => HR * (1 - s * k) = 1 + s => HR = (1 + s) / (1 - s * k).
    // If 0 < HR < 1: 1 - HR = s * (1 + k * HR) => HR * (1 + s * k) = 1 - s => HR = (1 - s) / (1 + s * k).
    let hr = if hr_greater_than_one {
        let denom = 1.0 - s * k;
        if denom <= 0.0 {
            return Err(MathError::DomainError { name: "total_events", value: total_events });
        }
        (1.0 + s) / denom
    } else {
        let num = 1.0 - s;
        if num <= 0.0 {
            return Err(MathError::DomainError { name: "total_events", value: total_events });
        }
        num / (1.0 + s * k)
    };
    if !hr.is_finite() || hr <= 0.0 {
        return Err(MathError::DomainError { name: "total_events", value: total_events });
    }
    Ok(hr)
}

/// Convert required total events `D` into continuous group and total sample sizes `(n_1, n_2, N)`
/// given per-arm event probabilities `(event_prob_group1, event_prob_group2)` in `(0, 1]` and
/// allocation ratio `k = n_2 / n_1 > 0` (`Collett 2015`, §10.2).
///
/// The pooled event probability is `p_bar = (p_E1 + k * p_E2) / (1 + k)`, giving `N = D / p_bar`,
/// `n_1 = N / (1 + k)`, and `n_2 = k * n_1`.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `total_events <= 0`, `allocation_ratio <= 0`, or
/// either event probability lies outside `(0, 1]` (or both cannot produce a positive pooled rate).
pub fn survival_sample_size_from_events(
    total_events: f64,
    event_prob_group1: f64,
    event_prob_group2: f64,
    allocation_ratio: f64,
) -> Result<(f64, f64, f64), MathError> {
    validate_events(total_events)?;
    if !allocation_ratio.is_finite() || allocation_ratio <= 0.0 {
        return Err(MathError::InvalidParameter { name: "allocation_ratio", value: allocation_ratio });
    }
    if !event_prob_group1.is_finite() || event_prob_group1 <= 0.0 || event_prob_group1 > 1.0 {
        return Err(MathError::InvalidParameter { name: "event_prob_group1", value: event_prob_group1 });
    }
    if !event_prob_group2.is_finite() || event_prob_group2 <= 0.0 || event_prob_group2 > 1.0 {
        return Err(MathError::InvalidParameter { name: "event_prob_group2", value: event_prob_group2 });
    }
    let k = allocation_ratio;
    let p_bar = (event_prob_group1 + k * event_prob_group2) / (1.0 + k);
    let total_n = total_events / p_bar;
    let n1 = total_n / (1.0 + k);
    let n2 = k * n1;
    Ok((n1, n2, total_n))
}

#[cfg(test)]
mod tests {
    use super::{
        freedman_logrank_detectable_hr, freedman_logrank_events, freedman_logrank_power, schoenfeld_logrank_detectable_hr, schoenfeld_logrank_events,
        schoenfeld_logrank_power, survival_sample_size_from_events,
    };

    #[test]
    fn schoenfeld_and_freedman_match_published_benchmarks_and_arm_symmetry() {
        // Classic benchmark: HR = 0.5, alpha = 0.05 (two-sided), power = 0.80, k = 1.
        // z_{0.975} = 1.959963984540054, z_{0.80} = 0.8416212335729143, (z_a + z_b)^2 = 7.84887973432116.
        // Schoenfeld: 4 * 7.84887973432116 / (ln 2)^2 = 65.34565925887097 (~66 events).
        // Freedman:   9 * 7.84887973432116 = 70.6399176088904 (~71 events).
        let d_sch = schoenfeld_logrank_events(0.5, 0.05, 0.80, 1.0, true).unwrap();
        assert!((d_sch - 65.345_659_258_870_97).abs() < 1e-10, "d_sch={d_sch}");
        let d_fre = freedman_logrank_events(0.5, 0.05, 0.80, 1.0, true).unwrap();
        assert!((d_fre - 70.639_917_609_141_85).abs() < 1e-10, "d_fre={d_fre}");

        // Arm-relabeling invariance: (HR, k) <-> (1/HR, 1/k).
        for &(hr, k) in &[(0.6, 2.0), (1.75, 0.5), (0.4, 1.5)] {
            let s1 = schoenfeld_logrank_events(hr, 0.05, 0.90, k, true).unwrap();
            let s2 = schoenfeld_logrank_events(1.0 / hr, 0.05, 0.90, 1.0 / k, true).unwrap();
            assert!((s1 - s2).abs() <= 1e-12 * s1);

            let f1 = freedman_logrank_events(hr, 0.05, 0.90, k, true).unwrap();
            let f2 = freedman_logrank_events(1.0 / hr, 0.05, 0.90, 1.0 / k, true).unwrap();
            assert!((f1 - f2).abs() <= 1e-12 * f1);
        }

        // Power and detectable-HR round-trips.
        let p_sch = schoenfeld_logrank_power(d_sch, 0.5, 0.05, 1.0, true).unwrap();
        assert!((p_sch - 0.80).abs() < 1e-5);
        let hr_sch = schoenfeld_logrank_detectable_hr(d_sch, 0.05, 0.80, 1.0, true, false).unwrap();
        assert!((hr_sch - 0.5).abs() < 1e-12);

        let p_fre = freedman_logrank_power(d_fre, 0.5, 0.05, 1.0, true).unwrap();
        assert!((p_fre - 0.80).abs() < 1e-5);
        let hr_fre = freedman_logrank_detectable_hr(d_fre, 0.05, 0.80, 1.0, true, false).unwrap();
        assert!((hr_fre - 0.5).abs() < 1e-12);

        // Sample size conversion with 1:2 allocation and arm event rates (0.7, 0.5).
        let (n1, n2, total_n) = survival_sample_size_from_events(66.0, 0.7, 0.5, 2.0).unwrap();
        let expected_pbar = (0.7 + 2.0 * 0.5) / 3.0;
        assert!((total_n - 66.0 / expected_pbar).abs() < 1e-12);
        assert!((n2 - 2.0 * n1).abs() < 1e-12);
        assert!(((n1 + n2) - total_n).abs() < 1e-12);
    }

    #[test]
    fn survival_power_rejects_invalid_domains() {
        assert!(schoenfeld_logrank_events(1.0, 0.05, 0.80, 1.0, true).is_err());
        assert!(schoenfeld_logrank_events(-0.5, 0.05, 0.80, 1.0, true).is_err());
        assert!(freedman_logrank_events(0.5, 0.0, 0.80, 1.0, true).is_err());
        assert!(freedman_logrank_detectable_hr(1.0, 0.05, 0.99, 1.0, true, true).is_err());
        assert!(survival_sample_size_from_events(50.0, 0.0, 0.5, 1.0).is_err());
    }
}
