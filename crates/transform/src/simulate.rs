//! Deterministic curve and distribution simulation (`TransformAnalysisSpec::Simulate`,
//! `docs/implementation/data-tables-and-transforms.md` §5.5).
//!
//! Built-in pharmacology, binding, and growth models align with `curvebook-fit`'s canonical
//! parameterizations (Hill 1910; Motulsky & Christopoulos 2004; Langmuir 1918; Michaelis & Menten
//! 1913; Seber & Wild 1989).

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnSimulationSpec, ColumnTable, ContingencySimDesign, ContingencySimulationSpec, ContingencyTable, DataSheet,
    MAX_ROWS, MAX_SUBCOLUMNS, SimulationNoise, SubcolumnMode, TableArchetype, XColumnMode, XyTable, validate_data_sets, validate_rows,
    validate_subcolumns,
};
use curvebook_math::DeterministicRng;

use crate::error::TransformError;
use crate::formula::{Expr, ScalarEnv, eval_scalar_expr, parse_formula};
use crate::import::MAX_IMPORT_CELLS;
use crate::randomize::sample_binomial;

/// Pre-compiled simulation equation so built-in parameter lookup/validation and custom formula
/// parsing occur once per simulation rather than once per row.
enum CompiledEquation<'a> {
    Builtin { id: &'a str, kind: BuiltinEquation },
    Custom { formula: &'a str, expr: Expr, env: ScalarEnv<'a> },
}

enum BuiltinEquation {
    Linear { slope: f64, intercept: f64 },
    Quadratic { a: f64, b: f64, c: f64 },
    Cubic { b0: f64, b1: f64, b2: f64, b3: f64 },
    OnePhaseDecay { plateau: f64, span: f64, k: f64 },
    OnePhaseAssociation { y0: f64, plateau: f64, k: f64 },
    TwoPhaseDecay { plateau: f64, span_fast: f64, span_slow: f64, k_fast: f64, k_slow: f64 },
    ExponentialGrowth { y0: f64, k: f64 },
    LogDoseResponse { bottom: f64, top: f64, log_c50: f64, hill_slope: f64, is_inhibition: bool },
    LinearHill { bottom: f64, top: f64, ec50: f64, hill_slope: f64 },
    MichaelisMenten { vmax: f64, km: f64 },
    OneSiteSpecificBinding { bmax: f64, kd: f64 },
    Gaussian { amp: f64, mean: f64, sd: f64, base: f64 },
    Sine { amp: f64, freq: f64, phase: f64, base: f64 },
}

impl<'a> CompiledEquation<'a> {
    fn compile(equation_id: &'a str, parameters: &'a [(String, f64)]) -> Result<Self, TransformError> {
        for (k, v) in parameters {
            if !v.is_finite() {
                return Err(TransformError::InvalidSpec { message: format!("simulation parameter `{k}` must be finite, got {v}") });
            }
        }
        let id = equation_id.trim();
        if id.is_empty() {
            return Err(TransformError::InvalidSpec { message: String::from("simulation equation_id must not be empty") });
        }
        let normalized = id.to_ascii_lowercase();
        let custom_formula = if normalized.starts_with("expr:") {
            id.get(5..)
        } else if normalized.starts_with("custom:") {
            id.get(7..)
        } else if id.starts_with('=') {
            Some(id)
        } else {
            None
        };
        if let Some(custom) = custom_formula {
            return Self::compile_custom(custom, parameters);
        }

        let kind = match normalized.as_str() {
            "linear" | "straight_line" => BuiltinEquation::Linear {
                slope: param(parameters, &["slope", "m", "a"]).unwrap_or(1.0),
                intercept: param(parameters, &["intercept", "b", "y0"]).unwrap_or(0.0),
            },
            "quadratic" => compile_quadratic(parameters),
            "cubic" => BuiltinEquation::Cubic {
                b0: param(parameters, &["b0", "d", "intercept"]).unwrap_or(0.0),
                b1: param(parameters, &["b1", "c"]).unwrap_or(0.0),
                b2: param(parameters, &["b2", "b"]).unwrap_or(0.0),
                b3: param(parameters, &["b3", "a"]).unwrap_or(1.0),
            },
            "exponential_decay" | "one_phase_decay" => {
                let plateau = param(parameters, &["plateau", "ns", "bottom"]).unwrap_or(0.0);
                let span = param(parameters, &["span"]).unwrap_or_else(|| param(parameters, &["y0", "top"]).unwrap_or(100.0) - plateau);
                let k = param(parameters, &["k", "rate"]).unwrap_or(1.0);
                BuiltinEquation::OnePhaseDecay { plateau, span, k }
            }
            "one_phase_association" => BuiltinEquation::OnePhaseAssociation {
                y0: param(parameters, &["y0", "bottom"]).unwrap_or(0.0),
                plateau: param(parameters, &["plateau", "top", "ymax"]).unwrap_or(100.0),
                k: param(parameters, &["k", "rate"]).unwrap_or(1.0),
            },
            "two_phase_decay" => BuiltinEquation::TwoPhaseDecay {
                plateau: param(parameters, &["plateau", "bottom"]).unwrap_or(0.0),
                span_fast: param(parameters, &["spanfast", "span_fast"]).unwrap_or(50.0),
                span_slow: param(parameters, &["spanslow", "span_slow"]).unwrap_or(50.0),
                k_fast: param(parameters, &["kfast", "k_fast"]).unwrap_or(2.0),
                k_slow: param(parameters, &["kslow", "k_slow"]).unwrap_or(0.2),
            },
            "exponential_growth" | "malthusian_growth" => BuiltinEquation::ExponentialGrowth {
                y0: param(parameters, &["y0", "a"]).unwrap_or(1.0),
                k: param(parameters, &["k", "rate"]).unwrap_or(1.0),
            },
            "hill_4pl" | "four_pl" | "4pl" | "sigmoidal_dose_response" | "dose_response_4pl" | "agonist_4pl" => compile_hill(parameters, true)?,
            "hill_3pl" | "three_pl" | "3pl" => compile_hill(parameters, false)?,
            "log_agonist_vs_response_4pl" | "log_agonist_vs_response_variable_slope" => compile_log_dose_response(parameters, true, false, false),
            "log_agonist_vs_response_3pl" | "log_agonist_vs_response" | "ec50_3pl" => compile_log_dose_response(parameters, false, false, false),
            "log_inhibitor_vs_response_4pl" | "log_inhibitor_vs_response_variable_slope" => compile_log_dose_response(parameters, true, true, false),
            "log_inhibitor_vs_response_3pl" | "log_inhibitor_vs_response" | "ic50_3pl" => compile_log_dose_response(parameters, false, true, false),
            "log_agonist_vs_response_normalized" => compile_log_dose_response(parameters, false, false, true),
            "log_agonist_vs_response_normalized_var" => compile_log_dose_response(parameters, true, false, true),
            "log_inhibitor_vs_response_normalized" => compile_log_dose_response(parameters, false, true, true),
            "log_inhibitor_vs_response_normalized_var" => compile_log_dose_response(parameters, true, true, true),
            "michaelis_menten" => BuiltinEquation::MichaelisMenten {
                vmax: param(parameters, &["vmax", "v_max"]).unwrap_or(100.0),
                km: param(parameters, &["km", "k_m"]).unwrap_or(1.0),
            },
            "one_site_specific_binding" => {
                let bmax = param(parameters, &["bmax", "b_max", "vmax"]).unwrap_or(100.0);
                let kd = param(parameters, &["kd", "k_d", "km"]).unwrap_or(1.0);
                if kd <= 0.0 {
                    return Err(TransformError::InvalidSpec { message: format!("one_site_specific_binding Kd must be > 0, got {kd}") });
                }
                BuiltinEquation::OneSiteSpecificBinding { bmax, kd }
            }
            "gaussian" => {
                let amp = param(parameters, &["amplitude", "amp", "a"]).unwrap_or(1.0);
                let mean = param(parameters, &["mean", "mu", "xc", "x0"]).unwrap_or(0.0);
                let sd = param(parameters, &["sd", "sigma", "w"]).unwrap_or(1.0);
                let base = param(parameters, &["baseline", "b", "y0"]).unwrap_or(0.0);
                if sd <= 0.0 {
                    return Err(TransformError::InvalidSpec { message: format!("gaussian sd must be > 0, got {sd}") });
                }
                BuiltinEquation::Gaussian { amp, mean, sd, base }
            }
            "sine" | "sinusoid" => compile_sine(parameters)?,
            _ => {
                if looks_like_expression(id) {
                    return Self::compile_custom(id, parameters);
                }
                return Err(TransformError::UnknownEquation { equation_id: id.to_owned() });
            }
        };

        Ok(Self::Builtin { id, kind })
    }

    fn compile_custom(formula: &'a str, parameters: &'a [(String, f64)]) -> Result<Self, TransformError> {
        let expr = parse_formula(formula)?;
        let mut bindings: Vec<(&'a str, f64)> = Vec::with_capacity(parameters.len() + 1);
        bindings.push(("x", 0.0));
        for (k, v) in parameters {
            bindings.push((k.as_str(), *v));
        }
        Ok(Self::Custom { formula, expr, env: ScalarEnv::new(bindings, None) })
    }

    fn eval(&mut self, x: f64) -> Result<f64, TransformError> {
        if !x.is_finite() {
            return Err(TransformError::InvalidSpec { message: format!("simulation x coordinate must be finite, got {x}") });
        }
        match self {
            Self::Custom { formula, expr, env } => {
                if let Some(slot) = env.bindings.first_mut() {
                    slot.1 = x;
                }
                let val = eval_scalar_expr(expr, env)?;
                val.as_finite_f64().ok_or_else(|| TransformError::EvalError {
                    message: format!("custom equation `{formula}` evaluated to missing/non-finite at x={x}"),
                })
            }
            Self::Builtin { id, kind } => {
                let y = eval_builtin(kind, x)?;
                if !y.is_finite() {
                    return Err(TransformError::EvalError { message: format!("equation `{id}` produced non-finite value at x={x}") });
                }
                Ok(y)
            }
        }
    }
}

/// Evaluate a named built-in simulation equation or custom formula expression at `x`.
pub fn evaluate_equation(equation_id: &str, x: f64, parameters: &[(String, f64)]) -> Result<f64, TransformError> {
    if !x.is_finite() {
        return Err(TransformError::InvalidSpec { message: format!("simulation x coordinate must be finite, got {x}") });
    }
    let mut compiled = CompiledEquation::compile(equation_id, parameters)?;
    compiled.eval(x)
}

/// Generate a synthetic [`DataSheet`] deterministically using `curvebook_math::DeterministicRng`.
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
    let mut compiled = CompiledEquation::compile(equation_id, parameters)?;

    let mode = if replicates == 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(replicates) };
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 1, row_count)?;
    xy.data_sets[0].title = String::from("Simulated");

    let mut rng = DeterministicRng::from_seed(seed);
    let mut any_finite = false;
    let mut first_eval_err: Option<TransformError> = None;
    for row in 0..row_count {
        let x = if row_count == 1 {
            x_min
        } else {
            let frac = (row as f64) / ((row_count - 1) as f64);
            if (x_max - x_min).is_finite() { x_min + frac * (x_max - x_min) } else { (1.0 - frac) * x_min + frac * x_max }
        };
        xy.set_x_cell(row, Cell::number(x))?;
        let y_true = match compiled.eval(x) {
            Ok(v) => v,
            Err(err @ TransformError::EvalError { .. }) => {
                first_eval_err.get_or_insert(err);
                continue;
            }
            Err(other) => return Err(other),
        };
        for rep in 0..replicates {
            match sample_noisy_observation(&mut rng, y_true, noise) {
                Ok(y_obs) if y_obs.is_finite() => {
                    xy.data_sets[0].cells[row][rep] = Cell::number(y_obs);
                    any_finite = true;
                }
                Ok(_) => {
                    first_eval_err.get_or_insert_with(|| TransformError::EvalError {
                        message: format!("simulated observation overflowed finite f64 range at x={x}"),
                    });
                }
                Err(err @ TransformError::EvalError { .. }) => {
                    first_eval_err.get_or_insert(err);
                }
                Err(other) => return Err(other),
            }
        }
    }

    if !any_finite {
        return Err(first_eval_err.unwrap_or_else(|| TransformError::EvalError {
            message: format!("equation `{equation_id}` produced no finite simulated values across [{x_min}, {x_max}]"),
        }));
    }

    let sheet = DataSheet::new(sheet_id, title, TableArchetype::Xy(xy))?;
    Ok(sheet)
}

/// Deterministically simulate a `2 × 2` [`ContingencyTable`] [`DataSheet`] under one of the four
/// canonical sampling designs (`FixedGrandTotal`, `FixedRowTotals`, `FixedColumnTotals`, or
/// `PoissonCounts`) using `curvebook_math::DeterministicRng`.
#[allow(clippy::cast_precision_loss)]
pub fn simulate_contingency_2x2(sheet_id: &str, title: &str, seed: u64, spec: &ContingencySimulationSpec) -> Result<DataSheet, TransformError> {
    for (idx, &v) in spec.cell_params.iter().enumerate() {
        if !v.is_finite() || v < 0.0 {
            return Err(TransformError::InvalidSpec { message: format!("contingency cell_params[{idx}] must be finite and >= 0, got {v}") });
        }
    }

    let mut rng = DeterministicRng::from_seed(seed);
    let [p00, p01, p10, p11] = spec.cell_params;
    let counts: [f64; 4] = match spec.design {
        ContingencySimDesign::FixedGrandTotal => {
            if spec.fixed_totals.len() != 1 {
                return Err(TransformError::InvalidSpec {
                    message: format!("FixedGrandTotal expects exactly 1 entry in fixed_totals ([N]), got {}", spec.fixed_totals.len()),
                });
            }
            let total_n = spec.fixed_totals[0] as u64;
            let sum_p = p00 + p01 + p10 + p11;
            if sum_p <= 0.0 || !sum_p.is_finite() {
                return Err(TransformError::InvalidSpec { message: String::from("FixedGrandTotal cell_params must sum to a positive finite value") });
            }
            if (sum_p - 1.0).abs() > 1e-6 {
                return Err(TransformError::InvalidSpec {
                    message: format!("FixedGrandTotal cell_params probabilities must sum to 1.0 (got {sum_p})"),
                });
            }
            let probs = [p00 / sum_p, p01 / sum_p, p10 / sum_p, p11 / sum_p];
            let mut rem = total_n;
            let mut rem_p = 1.0_f64;
            let mut drawn = [0_u64; 4];
            for i in 0..3 {
                let cond_p = if rem_p > 0.0 { (probs[i] / rem_p).clamp(0.0, 1.0) } else { 0.0 };
                let k = sample_binomial(&mut rng, rem, cond_p)?;
                drawn[i] = k;
                rem = rem.saturating_sub(k);
                rem_p = (rem_p - probs[i]).max(0.0);
            }
            drawn[3] = rem;
            [drawn[0] as f64, drawn[1] as f64, drawn[2] as f64, drawn[3] as f64]
        }
        ContingencySimDesign::FixedRowTotals => {
            if spec.fixed_totals.len() != 2 {
                return Err(TransformError::InvalidSpec {
                    message: format!("FixedRowTotals expects 2 entries in fixed_totals ([n_row0, n_row1]), got {}", spec.fixed_totals.len()),
                });
            }
            let s0 = p00 + p01;
            let s1 = p10 + p11;
            if s0 <= 0.0 || s1 <= 0.0 || !s0.is_finite() || !s1.is_finite() {
                return Err(TransformError::InvalidSpec { message: String::from("FixedRowTotals requires positive probability mass in each row") });
            }
            let n0 = spec.fixed_totals[0] as u64;
            let n1 = spec.fixed_totals[1] as u64;
            let c00 = sample_binomial(&mut rng, n0, (p00 / s0).clamp(0.0, 1.0))?;
            let c01 = n0.saturating_sub(c00);
            let c10 = sample_binomial(&mut rng, n1, (p10 / s1).clamp(0.0, 1.0))?;
            let c11 = n1.saturating_sub(c10);
            [c00 as f64, c01 as f64, c10 as f64, c11 as f64]
        }
        ContingencySimDesign::FixedColumnTotals => {
            if spec.fixed_totals.len() != 2 {
                return Err(TransformError::InvalidSpec {
                    message: format!("FixedColumnTotals expects 2 entries in fixed_totals ([n_col0, n_col1]), got {}", spec.fixed_totals.len()),
                });
            }
            let s0 = p00 + p10;
            let s1 = p01 + p11;
            if s0 <= 0.0 || s1 <= 0.0 || !s0.is_finite() || !s1.is_finite() {
                return Err(TransformError::InvalidSpec {
                    message: String::from("FixedColumnTotals requires positive probability mass in each column"),
                });
            }
            let n0 = spec.fixed_totals[0] as u64;
            let n1 = spec.fixed_totals[1] as u64;
            let c00 = sample_binomial(&mut rng, n0, (p00 / s0).clamp(0.0, 1.0))?;
            let c10 = n0.saturating_sub(c00);
            let c01 = sample_binomial(&mut rng, n1, (p01 / s1).clamp(0.0, 1.0))?;
            let c11 = n1.saturating_sub(c01);
            [c00 as f64, c01 as f64, c10 as f64, c11 as f64]
        }
        ContingencySimDesign::PoissonCounts => {
            let c00 = sample_noisy_observation(&mut rng, p00, SimulationNoise::Poisson)?;
            let c01 = sample_noisy_observation(&mut rng, p01, SimulationNoise::Poisson)?;
            let c10 = sample_noisy_observation(&mut rng, p10, SimulationNoise::Poisson)?;
            let c11 = sample_noisy_observation(&mut rng, p11, SimulationNoise::Poisson)?;
            [c00, c01, c10, c11]
        }
    };

    let mut tbl = ContingencyTable::new(2, 2)?;
    tbl.row_titles = vec![String::from("Row 1"), String::from("Row 2")];
    tbl.data_sets[0].title = String::from("Column 1");
    tbl.data_sets[1].title = String::from("Column 2");
    tbl.data_sets[0].cells[0][0] = Cell::number(counts[0]);
    tbl.data_sets[1].cells[0][0] = Cell::number(counts[1]);
    tbl.data_sets[0].cells[1][0] = Cell::number(counts[2]);
    tbl.data_sets[1].cells[1][0] = Cell::number(counts[3]);
    tbl.validate_counts()?;

    let sheet = DataSheet::new(sheet_id, title, TableArchetype::Contingency(tbl))?;
    sheet.validate()?;
    Ok(sheet)
}

/// Deterministically simulate a multi-group [`ColumnTable`] [`DataSheet`] with optional matched-row
/// random subject effects and additive outlier contamination.
///
/// When `resolved_means` is `Some(slice)`, it overrides `spec.column_means` (used after resolving
/// dynamic [`ColumnSimulationSpec::hooked_means`] in [`crate::ops::execute_transform`]).
pub fn simulate_column_sheet(
    sheet_id: &str,
    title: &str,
    seed: u64,
    row_count: usize,
    spec: &ColumnSimulationSpec,
    resolved_means: Option<&[f64]>,
) -> Result<DataSheet, TransformError> {
    if row_count == 0 || row_count > MAX_ROWS {
        validate_rows(row_count)?;
        if row_count == 0 {
            return Err(TransformError::InvalidSpec { message: String::from("column simulation row_count must be >= 1") });
        }
    }
    let means = resolved_means.unwrap_or(&spec.column_means);
    if means.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("ColumnSimulationSpec requires at least 1 column mean") });
    }
    validate_data_sets(means.len())?;
    for (idx, &m) in means.iter().enumerate() {
        if !m.is_finite() {
            return Err(TransformError::InvalidSpec { message: format!("column mean[{idx}] must be finite, got {m}") });
        }
    }
    if !spec.within_sd.is_finite() || spec.within_sd < 0.0 {
        return Err(TransformError::InvalidSpec { message: format!("column simulation within_sd must be finite and >= 0, got {}", spec.within_sd) });
    }
    let re_sd = spec.random_effect_sd.unwrap_or(0.0);
    if !re_sd.is_finite() || re_sd < 0.0 {
        return Err(TransformError::InvalidSpec { message: format!("column simulation random_effect_sd must be finite and >= 0, got {re_sd}") });
    }
    if !spec.outlier_fraction.is_finite() || !(0.0..=1.0).contains(&spec.outlier_fraction) {
        return Err(TransformError::InvalidSpec {
            message: format!("column simulation outlier_fraction must be in [0, 1], got {}", spec.outlier_fraction),
        });
    }
    if !spec.outlier_shift_sd.is_finite() {
        return Err(TransformError::InvalidSpec {
            message: format!("column simulation outlier_shift_sd must be finite, got {}", spec.outlier_shift_sd),
        });
    }
    check_simulation_cell_budget(row_count, means.len(), 1, 0)?;

    let design_hint = if re_sd > 0.0 { ColumnDesignHint::PairedRepeatedMeasures } else { ColumnDesignHint::UnpairedReplicates };
    let mut table = ColumnTable::new(design_hint, means.len(), row_count)?;
    for (c, ds) in table.data_sets.iter_mut().enumerate() {
        ds.title = format!("Group {}", c + 1);
    }

    let mut rng = DeterministicRng::from_seed(seed);
    let outlier_shift = if spec.within_sd > 0.0 { spec.outlier_shift_sd * spec.within_sd } else { spec.outlier_shift_sd };

    for r in 0..row_count {
        let subject_effect = if re_sd > 0.0 { rng.normal(0.0, re_sd)? } else { 0.0 };
        for (c, &mu) in means.iter().enumerate() {
            let residual = if spec.within_sd > 0.0 { rng.normal(0.0, spec.within_sd)? } else { 0.0 };
            let mut y = mu + subject_effect + residual;
            if spec.outlier_fraction > 0.0 && rng.next_f64() < spec.outlier_fraction {
                y += outlier_shift;
            }
            if !y.is_finite() {
                return Err(TransformError::EvalError { message: format!("simulated column observation overflowed at row {r} col {c}") });
            }
            table.data_sets[c].cells[r][0] = Cell::number(y);
        }
    }

    let sheet = DataSheet::new(sheet_id, title, TableArchetype::Column(table))?;
    sheet.validate()?;
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
            if cv_percent == 0.0 || y_true == 0.0 {
                y_true
            } else {
                y_true * (1.0 + (cv_percent / 100.0) * rng.standard_normal())
            }
        }
        SimulationNoise::Poisson => {
            // Exact PTRS Poisson count generation applies up to lambda = 1e15 (Atkinson, *Appl. Stat.* 1979,
            // 28(1), 29–35). Above 1e15 (where relative SD 1/sqrt(lambda) < 3.2e-8 and counts approach the
            // 2^53 IEEE-754 exact-integer limit), transition smoothly to the asymptotic normal count
            // approximation N(lambda, lambda) clamped at 0 (Johnson, Kotz & Kemp, *Univariate Discrete
            // Distributions*, 3rd ed., 2005, Ch. 4).
            const PTRS_MAX_LAMBDA: f64 = 1e15;
            let lambda = y_true.max(0.0);
            if lambda <= PTRS_MAX_LAMBDA {
                rng.poisson(lambda)? as f64
            } else {
                let z = rng.standard_normal();
                (lambda + lambda.sqrt() * z).max(0.0).round()
            }
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

fn compile_quadratic(parameters: &[(String, f64)]) -> BuiltinEquation {
    if let (Some(b0), Some(b1), Some(b2)) = (param(parameters, &["b0"]), param(parameters, &["b1"]), param(parameters, &["b2"])) {
        return BuiltinEquation::Quadratic { a: b2, b: b1, c: b0 };
    }
    BuiltinEquation::Quadratic {
        a: param(parameters, &["a", "b2"]).unwrap_or(1.0),
        b: param(parameters, &["b", "b1"]).unwrap_or(0.0),
        c: param(parameters, &["c", "b0", "intercept"]).unwrap_or(0.0),
    }
}

fn compile_sine(parameters: &[(String, f64)]) -> Result<BuiltinEquation, TransformError> {
    let amp = param(parameters, &["amplitude", "amp", "a"]).unwrap_or(1.0);
    let freq = if let Some(f) = param(parameters, &["frequency", "freq", "omega"]) {
        f
    } else if let Some(w) = param(parameters, &["wavelength"]) {
        if w == 0.0 {
            return Err(TransformError::InvalidSpec { message: String::from("sine wavelength must be non-zero, got 0") });
        }
        std::f64::consts::TAU / w
    } else {
        1.0
    };
    let phase = param(parameters, &["phase", "phi"]).unwrap_or(0.0);
    let base = param(parameters, &["baseline", "offset", "y0"]).unwrap_or(0.0);
    Ok(BuiltinEquation::Sine { amp, freq, phase, base })
}

fn compile_log_dose_response(parameters: &[(String, f64)], variable_slope: bool, is_inhibition: bool, normalized: bool) -> BuiltinEquation {
    let (bottom, top) = if normalized {
        (0.0, 100.0)
    } else {
        (param(parameters, &["bottom", "min", "y0"]).unwrap_or(0.0), param(parameters, &["top", "max", "ymax"]).unwrap_or(100.0))
    };
    let log_c50 = if is_inhibition {
        param(parameters, &["logic50", "log_ic50", "logec50", "log_ec50"]).unwrap_or(0.0)
    } else {
        param(parameters, &["logec50", "log_ec50", "logic50", "log_ic50"]).unwrap_or(0.0)
    };
    let hill_slope = if variable_slope { param(parameters, &["hillslope", "hill_slope", "n", "slope"]).unwrap_or(1.0) } else { 1.0 };
    BuiltinEquation::LogDoseResponse { bottom, top, log_c50, hill_slope, is_inhibition }
}

fn compile_hill(parameters: &[(String, f64)], allow_hill_slope: bool) -> Result<BuiltinEquation, TransformError> {
    let bottom = param(parameters, &["bottom", "min", "y0"]).unwrap_or(0.0);
    let top = param(parameters, &["top", "max", "ymax"]).unwrap_or(100.0);
    let hill_slope = if allow_hill_slope { param(parameters, &["hillslope", "hill_slope", "n", "slope"]).unwrap_or(1.0) } else { 1.0 };

    if let Some(log_ec50) = param(parameters, &["logic50", "logec50", "log_ic50", "log_ec50"]) {
        return Ok(BuiltinEquation::LogDoseResponse { bottom, top, log_c50: log_ec50, hill_slope, is_inhibition: false });
    }
    if let Some(ec50) = param(parameters, &["ec50", "ic50", "k50"]) {
        if ec50 <= 0.0 {
            return Err(TransformError::InvalidSpec { message: format!("Hill EC50/IC50 must be > 0, got {ec50}") });
        }
        return Ok(BuiltinEquation::LinearHill { bottom, top, ec50, hill_slope });
    }
    Ok(BuiltinEquation::LogDoseResponse { bottom, top, log_c50: 0.0, hill_slope, is_inhibition: false })
}

fn eval_builtin(kind: &BuiltinEquation, x: f64) -> Result<f64, TransformError> {
    let y = match *kind {
        BuiltinEquation::Linear { slope, intercept } => slope * x + intercept,
        BuiltinEquation::Quadratic { a, b, c } => a * x * x + b * x + c,
        BuiltinEquation::Cubic { b0, b1, b2, b3 } => b0 + x * (b1 + x * (b2 + x * b3)),
        BuiltinEquation::OnePhaseDecay { plateau, span, k } => plateau + span * (-k * x).exp(),
        BuiltinEquation::OnePhaseAssociation { y0, plateau, k } => y0 + (plateau - y0) * (1.0 - (-k * x).exp()),
        BuiltinEquation::TwoPhaseDecay { plateau, span_fast, span_slow, k_fast, k_slow } => {
            plateau + span_fast * (-k_fast * x).exp() + span_slow * (-k_slow * x).exp()
        }
        BuiltinEquation::ExponentialGrowth { y0, k } => y0 * (k * x).exp(),
        BuiltinEquation::LogDoseResponse { bottom, top, log_c50, hill_slope, is_inhibition } => {
            eval_log_dose_response(x, bottom, top, log_c50, hill_slope, is_inhibition)
        }
        BuiltinEquation::LinearHill { bottom, top, ec50, hill_slope } => {
            if x <= 0.0 {
                if hill_slope >= 0.0 { bottom } else { top }
            } else {
                bottom + (top - bottom) / (1.0 + (ec50 / x).powf(hill_slope))
            }
        }
        BuiltinEquation::MichaelisMenten { vmax, km } => {
            if (km + x) == 0.0 {
                return Err(TransformError::EvalError { message: String::from("Michaelis-Menten denominator km + x is zero") });
            }
            vmax * x / (km + x)
        }
        BuiltinEquation::OneSiteSpecificBinding { bmax, kd } => {
            if x < 0.0 {
                return Err(TransformError::EvalError { message: format!("one_site_specific_binding requires x >= 0, got {x}") });
            }
            eval_one_site_binding(x, bmax, kd)
        }
        BuiltinEquation::Gaussian { amp, mean, sd, base } => {
            let z = (x - mean) / sd;
            base + amp * (-0.5 * z * z).exp()
        }
        BuiltinEquation::Sine { amp, freq, phase, base } => base + amp * (freq * x + phase).sin(),
    };
    Ok(y)
}

fn eval_log_dose_response(x: f64, bottom: f64, top: f64, log_c50: f64, hill_slope: f64, is_inhibition: bool) -> f64 {
    let eff_slope = if is_inhibition { -hill_slope } else { hill_slope };
    let arg = (log_c50 - x) * eff_slope;
    let exp_ln10 = arg * std::f64::consts::LN_10;
    let sig = if exp_ln10 > 700.0 {
        0.0
    } else if exp_ln10 < -700.0 {
        1.0
    } else {
        1.0 / (1.0 + 10.0_f64.powf(arg))
    };
    bottom + (top - bottom) * sig
}

fn eval_one_site_binding(x: f64, bmax: f64, kd: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let r = x / kd;
    let frac = if r.is_infinite() {
        1.0
    } else if r == 0.0 {
        let log_r = x.ln() - kd.ln();
        if log_r < -745.0 {
            0.0
        } else {
            let rv = log_r.exp();
            rv / (1.0 + rv)
        }
    } else {
        r / (1.0 + r)
    };
    bmax * frac
}

fn looks_like_expression(s: &str) -> bool {
    s.contains('+') || s.contains('-') || s.contains('*') || s.contains('/') || s.contains('^') || s.contains('(')
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{MAX_ROWS, MAX_SUBCOLUMNS, SimulationNoise as Sn, TransformAnalysisSpec};

    use super::*;
    use crate::ops::{TransformContext, execute_transform};
    use crate::test_helpers as th;

    fn params(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs.iter().map(|&(k, v)| (k.to_owned(), v)).collect()
    }

    fn sim(seed: u64, shape: (usize, usize), xr: (f64, f64), eq: &str, ps: &[(&str, f64)], noise: Sn) -> Result<DataSheet, TransformError> {
        simulate_xy_sheet("sim", "Sim", seed, shape.0, shape.1, xr.0, xr.1, eq, &params(ps), noise)
    }

    #[test]
    fn r1_s04_08_simulate_enforces_cell_budget_before_allocation() {
        let err = sim(1, (MAX_ROWS, MAX_SUBCOLUMNS), (0.0, 1.0), "linear", &[("m", 1.0), ("b", 0.0)], Sn::Gaussian { sd: 1.0 }).unwrap_err();
        assert!(matches!(err, TransformError::InvalidSpec { ref message } if message.contains("budget")), "unexpected error: {err:?}");
        assert!(matches!(sim(1, (1_000_000, 10), (0.0, 1.0), "linear", &[], Sn::None), Err(TransformError::InvalidSpec { .. })));

        let seed_sheet = DataSheet::new("seed", "Seed", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 1).unwrap())).unwrap();
        let spec = TransformAnalysisSpec::Simulate {
            seed: 42,
            row_count: 50_000,
            replicates: 250,
            x_min: 0.0,
            x_max: 10.0,
            equation_id: String::from("linear"),
            parameters: vec![],
            noise: Sn::None,
            simulation_mode: Default::default(),
        };
        assert!(matches!(execute_transform(&seed_sheet, &spec, &TransformContext::default()), Err(TransformError::InvalidSpec { .. })));
        assert_eq!(check_simulation_cell_budget(1_000_000, 1, 9, 1).unwrap(), MAX_IMPORT_CELLS);
        assert!(matches!(check_simulation_cell_budget(usize::MAX, 1, 2, 1), Err(TransformError::InvalidSpec { .. })));
        let ok = sim(1, (2, MAX_SUBCOLUMNS), (0.0, 1.0), "linear", &[], Sn::None).unwrap();
        assert_eq!(ok.archetype.allocated_cell_slots(), 2 * (1 + MAX_SUBCOLUMNS));
    }

    #[test]
    fn aud04_25_canonical_fit_library_equations_match_exact_formulas() {
        let p4 = params(&[("Bottom", 10.0), ("Top", 90.0), ("LogEC50", -6.0), ("HillSlope", 2.0)]);
        let pi4 = params(&[("Bottom", 10.0), ("Top", 90.0), ("LogIC50", -6.0), ("HillSlope", 2.0)]);
        let (ag4, ag3) = (10.0 + 80.0 / (1.0 + 10.0_f64.powf(-2.0)), 10.0 + 80.0 / (1.0 + 10.0_f64.powf(-1.0)));
        let (inh4, inh3) = (10.0 + 80.0 / (1.0 + 10.0_f64.powf(2.0)), 10.0 + 80.0 / (1.0 + 10.0_f64.powf(1.0)));
        for (ids, ps, exp) in [
            (&["log_agonist_vs_response_4pl", "log_agonist_vs_response_variable_slope", "dose_response_4pl", "agonist_4pl"][..], &p4, ag4),
            (&["log_agonist_vs_response_3pl", "log_agonist_vs_response"][..], &p4, ag3),
            (&["log_inhibitor_vs_response_4pl", "log_inhibitor_vs_response_variable_slope"][..], &pi4, inh4),
            (&["log_inhibitor_vs_response_3pl", "log_inhibitor_vs_response"][..], &pi4, inh3),
            (&["log_agonist_vs_response_normalized"][..], &p4, 100.0 / 1.1),
            (&["log_agonist_vs_response_normalized_var"][..], &p4, 100.0 / 1.01),
            (&["log_inhibitor_vs_response_normalized"][..], &pi4, 100.0 / 11.0),
            (&["log_inhibitor_vs_response_normalized_var"][..], &pi4, 100.0 / 101.0),
        ] {
            for &id in ids {
                th::assert_close(evaluate_equation(id, -5.0, ps).unwrap(), exp, 1e-12);
            }
        }
        let bind_p = params(&[("Bmax", 120.0), ("Kd", 4.0)]);
        th::assert_close(evaluate_equation("exponential_growth", 2.0, &params(&[("Y0", 5.0), ("k", 0.5)])).unwrap(), 5.0 * 1.0_f64.exp(), 1e-12);
        th::assert_close(evaluate_equation("one_site_specific_binding", 4.0, &bind_p).unwrap(), 60.0, 1e-12);
        assert!(matches!(evaluate_equation("one_site_specific_binding", -1.0, &bind_p), Err(TransformError::EvalError { .. })));
        assert!(matches!(evaluate_equation("one_site_specific_binding", 1.0, &params(&[("Kd", 0.0)])), Err(TransformError::InvalidSpec { .. })));
        let s4 = simulate_xy_sheet("sim-4pl", "4PL", 42, 10, 2, -9.0, -3.0, "log_agonist_vs_response_4pl", &p4, Sn::None).unwrap();
        assert_eq!(s4.archetype.row_count(), 10);
    }

    #[test]
    fn aud04_37_simulate_compiles_custom_formula_once_across_many_rows() {
        let sheet = sim(7, (2_000, 1), (0.0, 1999.0), "EXPR: a * x + b", &[("a", 2.0), ("b", 3.0)], Sn::None).unwrap();
        let cells = &th::as_xy(&sheet).data_sets[0].cells;
        th::assert_close(th::num(&cells[0][0]), 3.0, 1e-12);
        th::assert_close(th::num(&cells[1999][0]), 4001.0, 1e-12);
    }

    #[test]
    fn aud04_47_relative_percent_overflow_and_parameter_validation_and_case_insensitive_prefix() {
        let s = sim(42, (3, 1), (0.0, 10.0), "linear", &[("m", 1e307), ("b", 0.0)], Sn::RelativePercent { cv_percent: 200.0 }).unwrap();
        assert_eq!(th::as_xy(&s).data_sets[0].cells[0][0].active_number(), Some(0.0));
        let bad = [
            ("gaussian", 0.0, "sd", f64::NAN),
            ("linear", 0.0, "slope", f64::INFINITY),
            ("sine", 1.0, "wavelength", 0.0),
            ("hill_4pl", 1.0, "ec50", 0.0),
            ("hill_4pl", 1.0, "ec50", -5.0),
        ];
        for (eq, x, k, v) in bad {
            assert!(matches!(evaluate_equation(eq, x, &params(&[(k, v)])), Err(TransformError::InvalidSpec { .. })), "{eq} {k}={v}");
        }
        for (eq, x, exp) in [("EXPR: 2 * x + 1", 3.0, 7.0), ("CUSTOM: x^2", 4.0, 16.0)] {
            th::assert_close(evaluate_equation(eq, x, &[]).unwrap(), exp, 1e-12);
        }
    }
}
