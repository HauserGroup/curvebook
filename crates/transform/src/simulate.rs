//! Deterministic curve and distribution simulation (`TransformAnalysisSpec::Simulate`,
//! `docs/data-tables-and-transforms.md` §5.5).

use lumencraft_doc::{
    Cell, DataSheet, MAX_ROWS, MAX_SUBCOLUMNS, SimulationNoise, SubcolumnMode, TableArchetype, XColumnMode, XyTable, validate_rows,
    validate_subcolumns,
};
use lumencraft_math::DeterministicRng;

use crate::error::TransformError;
use crate::formula::{ScalarEnv, eval_scalar_expr, parse_formula};
use crate::import::MAX_IMPORT_CELLS;

/// Evaluate a named built-in simulation equation or custom formula expression at `x`.
pub fn evaluate_equation(equation_id: &str, x: f64, parameters: &[(String, f64)]) -> Result<f64, TransformError> {
    if !x.is_finite() {
        return Err(TransformError::InvalidSpec { message: format!("simulation x coordinate must be finite, got {x}") });
    }
    let id = equation_id.trim();
    if id.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("simulation equation_id must not be empty") });
    }
    let normalized = id.to_ascii_lowercase();
    if let Some(custom) = id.strip_prefix("expr:").or_else(|| id.strip_prefix("custom:")) {
        return eval_custom_formula(custom, x, parameters);
    }
    if id.starts_with('=') {
        return eval_custom_formula(id, x, parameters);
    }

    let y = match normalized.as_str() {
        "linear" | "straight_line" => {
            let slope = param(parameters, &["slope", "m", "a"]).unwrap_or(1.0);
            let intercept = param(parameters, &["intercept", "b", "y0"]).unwrap_or(0.0);
            slope * x + intercept
        }
        "quadratic" => eval_quadratic(x, parameters),
        "cubic" => eval_cubic(x, parameters),
        "exponential_decay" | "one_phase_decay" => eval_one_phase_decay(x, parameters),
        "one_phase_association" => eval_one_phase_association(x, parameters),
        "two_phase_decay" => eval_two_phase_decay(x, parameters),
        "hill_4pl" | "four_pl" | "4pl" | "sigmoidal_dose_response" => eval_hill(x, parameters, true),
        "hill_3pl" | "three_pl" | "3pl" => eval_hill(x, parameters, false),
        "michaelis_menten" => {
            let vmax = param(parameters, &["vmax", "v_max"]).unwrap_or(100.0);
            let km = param(parameters, &["km", "k_m"]).unwrap_or(1.0);
            if (km + x) == 0.0 {
                return Err(TransformError::EvalError { message: String::from("Michaelis-Menten denominator km + x is zero") });
            }
            vmax * x / (km + x)
        }
        "gaussian" => {
            let amp = param(parameters, &["amplitude", "amp", "a"]).unwrap_or(1.0);
            let mean = param(parameters, &["mean", "mu", "xc", "x0"]).unwrap_or(0.0);
            let sd = param(parameters, &["sd", "sigma", "w"]).unwrap_or(1.0);
            let base = param(parameters, &["baseline", "b", "y0"]).unwrap_or(0.0);
            if sd <= 0.0 {
                return Err(TransformError::InvalidSpec { message: format!("gaussian sd must be > 0, got {sd}") });
            }
            let z = (x - mean) / sd;
            base + amp * (-0.5 * z * z).exp()
        }
        "sine" | "sinusoid" => {
            let amp = param(parameters, &["amplitude", "amp", "a"]).unwrap_or(1.0);
            let freq = param(parameters, &["frequency", "freq", "omega"])
                .unwrap_or_else(|| param(parameters, &["wavelength"]).map_or(1.0, |w| if w != 0.0 { std::f64::consts::TAU / w } else { 1.0 }));
            let phase = param(parameters, &["phase", "phi"]).unwrap_or(0.0);
            let base = param(parameters, &["baseline", "offset", "y0"]).unwrap_or(0.0);
            base + amp * (freq * x + phase).sin()
        }
        _ => {
            if looks_like_expression(id) {
                return eval_custom_formula(id, x, parameters);
            }
            return Err(TransformError::UnknownEquation { equation_id: id.to_owned() });
        }
    };

    if !y.is_finite() {
        return Err(TransformError::EvalError { message: format!("equation `{id}` produced non-finite value at x={x}") });
    }
    Ok(y)
}

/// Generate a synthetic [`DataSheet`] deterministically using `lumencraft_math::DeterministicRng`.
#[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
pub fn simulate_xy_sheet(
    sheet_id: &str,
    title: &str,
    seed: u64,
    row_count: usize,
    replicates: usize,
    x_min: f64,
    x_max: f64,
    equation_id: &str,
    parameters: &[(String, f64)],
    noise: SimulationNoise,
) -> Result<DataSheet, TransformError> {
    if row_count == 0 || row_count > MAX_ROWS {
        validate_rows(row_count)?;
        if row_count == 0 {
            return Err(TransformError::InvalidSpec { message: String::from("simulation row_count must be >= 1") });
        }
    }
    if replicates == 0 || replicates > MAX_SUBCOLUMNS {
        validate_subcolumns(replicates)?;
    }
    if !x_min.is_finite() || !x_max.is_finite() {
        return Err(TransformError::InvalidSpec { message: String::from("simulation x_min and x_max must be finite") });
    }
    validate_noise(noise)?;
    check_simulation_cell_budget(row_count, 1, replicates, 1)?;

    let mode = if replicates == 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(replicates) };
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 1, row_count)?;
    xy.data_sets[0].title = String::from("Simulated");

    let mut rng = DeterministicRng::from_seed(seed);
    for row in 0..row_count {
        let x = if row_count == 1 {
            x_min
        } else {
            let frac = (row as f64) / ((row_count - 1) as f64);
            x_min + frac * (x_max - x_min)
        };
        xy.set_x_cell(row, Cell::number(x))?;
        let y_true = evaluate_equation(equation_id, x, parameters)?;
        for rep in 0..replicates {
            let y_obs = sample_noisy_observation(&mut rng, y_true, noise)?;
            xy.data_sets[0].cells[row][rep] = Cell::number(y_obs);
        }
    }

    let sheet = DataSheet::new(sheet_id, title, TableArchetype::Xy(xy))?;
    Ok(sheet)
}

/// Enforce [`MAX_IMPORT_CELLS`] using overflow-checked arithmetic before allocating simulation grids or drawing RNG samples.
fn check_simulation_cell_budget(row_count: usize, data_sets: usize, replicates: usize, x_subcolumns: usize) -> Result<usize, TransformError> {
    let overflow = || TransformError::InvalidSpec { message: format!("simulation cell allocation overflows budget limit {MAX_IMPORT_CELLS}") };
    let y_cells = row_count.checked_mul(replicates).and_then(|v| v.checked_mul(data_sets)).ok_or_else(overflow)?;
    if y_cells > MAX_IMPORT_CELLS {
        return Err(TransformError::InvalidSpec {
            message: format!(
                "simulation cell allocation ({row_count} rows x {replicates} replicates x {data_sets} dataset(s) = {y_cells} Y cells) exceeds budget limit {MAX_IMPORT_CELLS}"
            ),
        });
    }
    let x_cells = row_count.checked_mul(x_subcolumns).ok_or_else(overflow)?;
    let total_cells = y_cells.checked_add(x_cells).ok_or_else(overflow)?;
    if total_cells > MAX_IMPORT_CELLS {
        return Err(TransformError::InvalidSpec {
            message: format!(
                "simulation cell allocation {total_cells} ({row_count} rows x ({x_subcolumns} X + {data_sets} x {replicates} Y)) exceeds budget limit {MAX_IMPORT_CELLS}"
            ),
        });
    }
    Ok(total_cells)
}

fn validate_noise(noise: SimulationNoise) -> Result<(), TransformError> {
    match noise {
        SimulationNoise::None | SimulationNoise::Poisson => Ok(()),
        SimulationNoise::Gaussian { sd } => {
            if !sd.is_finite() || sd < 0.0 {
                return Err(TransformError::InvalidSpec { message: format!("Gaussian noise sd must be finite and >= 0, got {sd}") });
            }
            Ok(())
        }
        SimulationNoise::RelativePercent { cv_percent } => {
            if !cv_percent.is_finite() || cv_percent < 0.0 {
                return Err(TransformError::InvalidSpec { message: format!("RelativePercent cv_percent must be finite and >= 0, got {cv_percent}") });
            }
            Ok(())
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn sample_noisy_observation(rng: &mut DeterministicRng, y_true: f64, noise: SimulationNoise) -> Result<f64, TransformError> {
    let val = match noise {
        SimulationNoise::None => y_true,
        SimulationNoise::Gaussian { sd } => {
            if sd == 0.0 {
                y_true
            } else {
                rng.normal(y_true, sd)?
            }
        }
        SimulationNoise::RelativePercent { cv_percent } => {
            let sd = (cv_percent / 100.0) * y_true.abs();
            if sd == 0.0 { y_true } else { rng.normal(y_true, sd)? }
        }
        SimulationNoise::Poisson => {
            let lambda = y_true.max(0.0);
            rng.poisson(lambda)? as f64
        }
    };
    Ok(val)
}

fn param(parameters: &[(String, f64)], names: &[&str]) -> Option<f64> {
    for candidate in names {
        if let Some((_, val)) = parameters.iter().find(|(k, _)| k.trim().eq_ignore_ascii_case(candidate)) {
            return Some(*val);
        }
    }
    None
}

fn eval_quadratic(x: f64, parameters: &[(String, f64)]) -> f64 {
    if let (Some(b0), Some(b1), Some(b2)) = (param(parameters, &["b0"]), param(parameters, &["b1"]), param(parameters, &["b2"])) {
        return b0 + b1 * x + b2 * x * x;
    }
    let a = param(parameters, &["a", "b2"]).unwrap_or(1.0);
    let b = param(parameters, &["b", "b1"]).unwrap_or(0.0);
    let c = param(parameters, &["c", "b0", "intercept"]).unwrap_or(0.0);
    a * x * x + b * x + c
}

fn eval_cubic(x: f64, parameters: &[(String, f64)]) -> f64 {
    let b0 = param(parameters, &["b0", "d", "intercept"]).unwrap_or(0.0);
    let b1 = param(parameters, &["b1", "c"]).unwrap_or(0.0);
    let b2 = param(parameters, &["b2", "b"]).unwrap_or(0.0);
    let b3 = param(parameters, &["b3", "a"]).unwrap_or(1.0);
    b0 + x * (b1 + x * (b2 + x * b3))
}

fn eval_one_phase_decay(x: f64, parameters: &[(String, f64)]) -> f64 {
    let plateau = param(parameters, &["plateau", "ns", "bottom"]).unwrap_or(0.0);
    let span = param(parameters, &["span"]).unwrap_or_else(|| param(parameters, &["y0", "top"]).unwrap_or(100.0) - plateau);
    let k = param(parameters, &["k", "rate"]).unwrap_or(1.0);
    plateau + span * (-k * x).exp()
}

fn eval_one_phase_association(x: f64, parameters: &[(String, f64)]) -> f64 {
    let y0 = param(parameters, &["y0", "bottom"]).unwrap_or(0.0);
    let plateau = param(parameters, &["plateau", "top", "ymax"]).unwrap_or(100.0);
    let k = param(parameters, &["k", "rate"]).unwrap_or(1.0);
    y0 + (plateau - y0) * (1.0 - (-k * x).exp())
}

fn eval_two_phase_decay(x: f64, parameters: &[(String, f64)]) -> f64 {
    let plateau = param(parameters, &["plateau", "bottom"]).unwrap_or(0.0);
    let span_fast = param(parameters, &["spanfast", "span_fast"]).unwrap_or(50.0);
    let span_slow = param(parameters, &["spanslow", "span_slow"]).unwrap_or(50.0);
    let k_fast = param(parameters, &["kfast", "k_fast"]).unwrap_or(2.0);
    let k_slow = param(parameters, &["kslow", "k_slow"]).unwrap_or(0.2);
    plateau + span_fast * (-k_fast * x).exp() + span_slow * (-k_slow * x).exp()
}

fn eval_hill(x: f64, parameters: &[(String, f64)], allow_hill_slope: bool) -> f64 {
    let bottom = param(parameters, &["bottom", "min", "y0"]).unwrap_or(0.0);
    let top = param(parameters, &["top", "max", "ymax"]).unwrap_or(100.0);
    let hill_slope = if allow_hill_slope { param(parameters, &["hillslope", "hill_slope", "n", "slope"]).unwrap_or(1.0) } else { 1.0 };

    if let Some(log_ec50) = param(parameters, &["logic50", "logec50", "log_ic50", "log_ec50"]) {
        return bottom + (top - bottom) / (1.0 + 10.0_f64.powf((log_ec50 - x) * hill_slope));
    }
    if let Some(ec50) = param(parameters, &["ec50", "ic50", "k50"]) {
        if x <= 0.0 {
            return if hill_slope >= 0.0 { bottom } else { top };
        }
        return bottom + (top - bottom) / (1.0 + (ec50 / x).powf(hill_slope));
    }
    bottom + (top - bottom) / (1.0 + 10.0_f64.powf(-x * hill_slope))
}

fn looks_like_expression(s: &str) -> bool {
    s.contains('+') || s.contains('-') || s.contains('*') || s.contains('/') || s.contains('^') || s.contains('(')
}

fn eval_custom_formula(formula: &str, x: f64, parameters: &[(String, f64)]) -> Result<f64, TransformError> {
    let expr = parse_formula(formula)?;
    let mut bindings: Vec<(&str, f64)> = Vec::with_capacity(parameters.len() + 1);
    bindings.push(("x", x));
    for (k, v) in parameters {
        bindings.push((k.as_str(), *v));
    }
    let env = ScalarEnv::new(bindings, None);
    let val = eval_scalar_expr(&expr, &env)?;
    val.as_finite_f64()
        .ok_or_else(|| TransformError::EvalError { message: format!("custom equation `{formula}` evaluated to missing/non-finite at x={x}") })
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{MAX_ROWS, MAX_SUBCOLUMNS, SimulationNoise, TransformAnalysisSpec};

    use super::*;
    use crate::ops::{TransformContext, execute_transform};

    #[test]
    fn r1_s04_08_simulate_enforces_cell_budget_before_allocation() {
        // 1,000,000 rows x 512 replicates passes per-axis limits but requests 513M total cells.
        let err = simulate_xy_sheet(
            "sim-oom",
            "Too Big",
            1,
            MAX_ROWS,
            MAX_SUBCOLUMNS,
            0.0,
            1.0,
            "linear",
            &[(String::from("m"), 1.0), (String::from("b"), 0.0)],
            SimulationNoise::Gaussian { sd: 1.0 },
        )
        .unwrap_err();
        assert!(matches!(err, TransformError::InvalidSpec { ref message } if message.contains("budget")), "unexpected error: {err:?}");

        // Exact Y boundary (1,000,000 * 10 = 10M Y cells) still exceeds MAX_IMPORT_CELLS once X column is included (11M > 10M).
        let err_with_x = simulate_xy_sheet("sim-x-over", "With X", 1, 1_000_000, 10, 0.0, 1.0, "linear", &[], SimulationNoise::None).unwrap_err();
        assert!(matches!(err_with_x, TransformError::InvalidSpec { .. }), "expected InvalidSpec when X+Y exceeds cap, got {err_with_x:?}");

        // Execute transform path must also enforce the same budget.
        let seed_sheet = DataSheet::new("seed", "Seed", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 1).unwrap())).unwrap();
        let spec = TransformAnalysisSpec::Simulate {
            seed: 42,
            row_count: 50_000,
            replicates: 250,
            x_min: 0.0,
            x_max: 10.0,
            equation_id: String::from("linear"),
            parameters: vec![],
            noise: SimulationNoise::None,
        };
        let exec_err = execute_transform(&seed_sheet, &spec, &TransformContext::default()).unwrap_err();
        assert!(matches!(exec_err, TransformError::InvalidSpec { .. }), "expected InvalidSpec, got {exec_err:?}");

        // Overflow-checked budget helper accepts exact cap and rejects usize overflow cleanly.
        assert_eq!(check_simulation_cell_budget(1_000_000, 1, 9, 1).unwrap(), MAX_IMPORT_CELLS);
        assert!(matches!(check_simulation_cell_budget(usize::MAX, 1, 2, 1), Err(TransformError::InvalidSpec { .. })));

        // Small trace within per-axis max subcolumns remains accepted.
        let ok = simulate_xy_sheet("sim-ok", "Small", 1, 2, MAX_SUBCOLUMNS, 0.0, 1.0, "linear", &[], SimulationNoise::None).unwrap();
        assert_eq!(ok.archetype.allocated_cell_slots(), 2 * (1 + MAX_SUBCOLUMNS));
    }
}
