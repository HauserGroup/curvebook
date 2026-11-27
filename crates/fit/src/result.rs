//! End-to-end execution of [`AnalysisSpec`] (`LinearRegression` & `NonlinearRegression`) against
//! upstream [`DataSheet`]s, assembling complete multi-tab [`ResultSheet`] artifacts with
//! `FittedCurveOverlay`s, hookable scalars, and plain-language [`ChecklistItem`] diagnostics.
//!
//! The statistical routines and their references live in [`crate::solver`],
//! [`crate::diagnostics`], and [`crate::linear`]; this module only formats their output, apart
//! from the normal Q-Q plotting positions and the trapezoidal AUC documented below.
//!
//! Reference: Blom, G. (1958). *Statistical Estimates and Transformed Beta-Variables*. Wiley.
//!
//! Reference: Ratkowsky, D. A. (1990). *Handbook of Nonlinear Regression Models*. Marcel Dekker.
//!
//! Reference: Press, W. H., Teukolsky, S. A., Vetterling, W. T. & Flannery, B. P. (2007). *Numerical Recipes: The Art of Scientific Computing*, 3rd ed. Cambridge University Press.

use std::collections::{HashMap, HashSet};

use curvebook_doc::result_builder::{KvTabBuilder, TabBuilder, bool_cell, ci_cell, num_cell, opt_num_cell, p_cell, text_cell};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistItem, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, CurveSamplePoint, DataSheet,
    DemingErrorSpec, ErrorCode, FitFlagRules, FittedCurveOverlay, IdentifiabilityMode, LinearRegressionSpec, NonlinearFitSpec, OutlierHandling,
    ReplicateFitMode, ResultCell, ResultCellDetail, ResultScalarEntry, ResultSheet, ResultStatus, ResultTabKind, SourceCellId, SubcolumnMode,
    TableArchetype, XColumnMode, XyTable, format_significant_digits,
};
use curvebook_math::PValueFormat;
use curvebook_math::dist::norm_ppf;
use curvebook_math::sum::NeumaierSum;

use crate::distribution_plot::{evaluate_theoretical_distribution, populate_distribution_plot_result};
use crate::dsl::CompiledDslModel;
use crate::error::{FitError, FitIdentifiability, FitStatus, ParameterNote};
use crate::library::{ParametricModel, get_builtin_model};
use crate::linear::{
    DemingOptions, InverseIntervalSet, SimpleLinearOptions, SimpleLinearResult, compare_linear_lines, deming_regression, fit_linear_points,
    linear_grid, prepare_linear_points, resolve_linear_grid_bounds,
};
use crate::solver::{
    FittedDatasetResult, FittedParameterSummary, HorizontalLineComparisonReport, InterpolatedUnknown, ModelComparisonOutcome, NonlinearFitResult,
    SolverConfig, compare_models_question, compare_vs_horizontal_line, evaluate_fit_flags, fit_nonlinear, ordered_nested_roles,
};
use crate::types::{
    DataPoint, DatasetSeries, HookContext, enforce_summary_mode_matrix, extract_datasets_from_sheet, extract_datasets_from_sheet_multi,
    extract_from_multivar_multi_with_y_map, filter_datasets_by_range, per_dataset_parameter_key,
};

fn ci_bounds_cell(lower: CiBound, upper: CiBound) -> ResultCell {
    let sanitize = |bound: CiBound| match bound {
        CiBound::Finite(v) if v.is_finite() => CiBound::Finite(v),
        CiBound::Open => CiBound::Open,
        CiBound::Finite(_) | CiBound::Unavailable => CiBound::Unavailable,
    };
    ResultCell::ConfidenceInterval { lower: sanitize(lower), upper: sanitize(upper) }
}

fn format_percent_label(pct: f64) -> String {
    if !pct.is_finite() {
        return pct.to_string();
    }
    if (pct - pct.round()).abs() < 1e-9 {
        format!("{:.0}", pct.round())
    } else {
        let rounded = (pct * 1e9).round() / 1e9;
        format!("{rounded}")
    }
}

/// Execute either a [`AnalysisKind::LinearRegression`] or [`AnalysisKind::NonlinearRegression`]
/// [`AnalysisSpec`] against a [`DataSheet`] and return a populated [`ResultSheet`].
pub fn execute_analysis(spec: &AnalysisSpec, sheet: &DataSheet, hooks: &HookContext) -> Result<ResultSheet, FitError> {
    match &spec.kind {
        AnalysisKind::LinearRegression(lin) => execute_linear_analysis(spec, lin, sheet),
        AnalysisKind::NonlinearRegression(nl) => execute_nonlinear_analysis(spec, nl, sheet, hooks),
        other => Err(FitError::InvalidParameter {
            name: "analysis.kind",
            reason: format!("Unsupported analysis kind `{}` for curvebook-fit", other.derive_auto_title()),
        }),
    }
}

/// Execute linear regression (OLS, origin-forced, ANCOVA line comparison, or Model II Deming)
/// and populate a [`ResultSheet`].
pub fn execute_linear_analysis(spec: &AnalysisSpec, lin_spec: &LinearRegressionSpec, sheet: &DataSheet) -> Result<ResultSheet, FitError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::LinearRegression(*lin_spec))?;
    let mut result = ResultSheet::new(spec.clone())?;
    let mut datasets = extract_datasets_from_sheet(sheet, &spec.selected_data_sets)?;
    let p_fmt = spec.p_value_format;
    let mut excluded = vec![0; datasets.len()];
    let x_filter = lin_spec.x_range_filter.filter(|(min, max)| min.is_some() || max.is_some());
    if let Some(bounds) = x_filter {
        for (ds, n_excluded) in datasets.iter_mut().zip(&mut excluded) {
            let (points, count) = prepare_linear_points(&ds.points, bounds, !lin_spec.deming_model_ii)?;
            ds.points = points.into_owned();
            *n_excluded = count;
        }
    }

    let segments = lin_spec.plot_segments.unwrap_or(LinearRegressionSpec::DEFAULT_PLOT_SEGMENTS);
    if !(1..=LinearRegressionSpec::MAX_PLOT_SEGMENTS).contains(&segments) {
        return Err(FitError::InvalidParameter {
            name: "plot_segments",
            reason: format!("must be between 1 and {}", LinearRegressionSpec::MAX_PLOT_SEGMENTS),
        });
    }
    let options = SimpleLinearOptions {
        force_through_origin: lin_spec.force_through_origin,
        fixed_point: lin_spec.fixed_point,
        standard_addition: lin_spec.standard_addition,
        extend_line_to_x_intercept: lin_spec.extend_line_to_x_intercept,
        confidence_level: spec.confidence_level,
        compute_bands: lin_spec.confidence_bands || lin_spec.prediction_bands,
        band_grid_points: segments + 1,
        x_range_filter: x_filter,
        plot_x_range: Some((lin_spec.plot_x_min, lin_spec.plot_x_max)),
        ..SimpleLinearOptions::default()
    };
    if lin_spec.deming_model_ii {
        return build_deming_result_sheet(result, &datasets, &excluded, &options, lin_spec);
    }

    let mut fits: Vec<(usize, String, SimpleLinearResult)> = Vec::with_capacity(datasets.len());
    for (ds, n_excluded) in datasets.iter().zip(excluded) {
        let mut opts = options.clone();
        if lin_spec.interpolate_unknowns {
            opts.unknown_y_values = ds.unknowns_y.iter().map(|value| value.value()).collect();
            opts.unknown_x_values = ds.unknowns_x.iter().map(|&(_, x)| x).collect();
        }
        let mut fit = fit_linear_points(&ds.points, &opts, n_excluded)?;
        let extend_x = if opts.extend_line_to_x_intercept { fit.x_intercept } else { None };
        let overlay_points = if fit.bands.is_empty() {
            build_linear_overlay_points(&ds.points, fit.slope, fit.intercept, &opts, extend_x)?
        } else {
            std::mem::take(&mut fit.bands)
        };
        result.fitted_curves.push(FittedCurveOverlay {
            data_set_index: ds.data_set_index,
            equation_id: if lin_spec.effective_fixed_point() == Some((0.0, 0.0)) {
                "line_through_origin".to_owned()
            } else {
                "straight_line".to_owned()
            },
            converged: true,
            ambiguous: false,
            points: overlay_points,
        });
        result.scalars.push(ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(ds.data_set_index), value: fit.slope, units: None });
        result.scalars.push(ResultScalarEntry {
            key: "YIntercept".to_owned(),
            data_set_index: Some(ds.data_set_index),
            value: fit.intercept,
            units: None,
        });
        result.scalars.push(ResultScalarEntry { key: "R2".to_owned(), data_set_index: Some(ds.data_set_index), value: fit.r_squared, units: None });
        if let Some(x_int) = fit.x_intercept {
            result.scalars.push(ResultScalarEntry {
                key: "XIntercept".to_owned(),
                data_set_index: Some(ds.data_set_index),
                value: x_int,
                units: None,
            });
        }
        if let Some(conc) = fit.standard_addition_concentration {
            result.scalars.push(ResultScalarEntry {
                key: "StandardAdditionConcentration".to_owned(),
                data_set_index: Some(ds.data_set_index),
                value: conc,
                units: None,
            });
        }

        let runs = &fit.runs_test;
        result.checklist.push(ChecklistItem {
            assumption: format!("Linearity (Runs test — {})", ds.label),
            status: if runs.deviates_significantly { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            diagnostic_metric: Some(format!("p = {:.4}, runs = {}", runs.p_too_few_runs, runs.runs)),
            explanation: if runs.deviates_significantly {
                "Points deviate systematically from a straight line (too few runs).".to_owned()
            } else {
                "No significant systematic departure from linearity detected.".to_owned()
            },
            remediation: None,
        });
        if let Some(lof) = &fit.lack_of_fit {
            result.checklist.push(ChecklistItem {
                assumption: format!("Replicates lack-of-fit ({})", ds.label),
                status: if lof.significant_lack_of_fit { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
                diagnostic_metric: Some(format!("F = {:.3}, p = {:.4}", lof.f_stat, lof.p_value)),
                explanation: if lof.significant_lack_of_fit {
                    "Scatter around the line significantly exceeds pure replicate variation.".to_owned()
                } else {
                    "Model agrees with replicate variation.".to_owned()
                },
                remediation: None,
            });
        }
        fits.push((ds.data_set_index, ds.label.clone(), fit));
    }

    // Main summary tab across datasets.
    let conf_pct = format_percent_label(options.confidence_level.percent());
    let mut summary_tab = TabBuilder::new("linear_summary", "Best-fit values", ResultTabKind::MainSummary)
        .headers(fits.iter().map(|(_, lbl, _)| lbl.as_str()))
        .row("Slope", fits.iter().map(|(_, _, f)| num_cell(f.slope)))
        .row("Y-intercept", fits.iter().map(|(_, _, f)| num_cell(f.intercept)))
        .row("SE of Slope", fits.iter().map(|(_, _, f)| num_cell(f.se_slope)))
        .row("SE of Intercept", fits.iter().map(|(_, _, f)| num_cell(f.se_intercept)))
        .row(format!("{conf_pct}% CI Slope"), fits.iter().map(|(_, _, f)| ci_cell(f.ci_slope.0, f.ci_slope.1)))
        .row("R squared", fits.iter().map(|(_, _, f)| num_cell(f.r_squared)))
        .row("Sy.x", fits.iter().map(|(_, _, f)| num_cell(f.sy_x)))
        .row("F", fits.iter().map(|(_, _, f)| num_cell(f.f_stat)))
        .row("P value", fits.iter().map(|(_, _, f)| p_cell(f.p_value, p_fmt)))
        .row("Runs test P value", fits.iter().map(|(_, _, f)| p_cell(f.runs_test.p_too_few_runs, p_fmt)))
        .row("X-intercept", fits.iter().map(|(_, _, f)| opt_num_cell(f.x_intercept)))
        .row(
            format!("{conf_pct}% CI X-intercept"),
            fits.iter().map(|(_, _, f)| {
                let (lo, hi) = f.x_intercept_ci.to_ci_bounds();
                ci_bounds_cell(lo, hi)
            }),
        );
    if lin_spec.standard_addition {
        summary_tab.push_row("Unknown concentration (-X-intercept)", fits.iter().map(|(_, _, f)| opt_num_cell(f.standard_addition_concentration)));
        summary_tab.push_row(
            format!("{conf_pct}% CI Unknown concentration"),
            fits.iter().map(|(_, _, f)| {
                let (lo, hi) = f.standard_addition_ci.unwrap_or(InverseIntervalSet::Undefined).to_ci_bounds();
                ci_bounds_cell(lo, hi)
            }),
        );
    }
    if x_filter.is_some() {
        append_linear_observation_counts(&mut summary_tab, fits.iter().map(|(_, _, f)| [f.n, f.n_excluded_by_range, f.df_residual]));
    }
    result.tabs.push(summary_tab.build());

    // Multi-line ANCOVA comparison if >1 dataset (for unconstrained or origin-forced models).
    if datasets.len() >= 2 && lin_spec.fixed_point.is_none_or(|pt| pt == (0.0, 0.0)) {
        let grouped_slices: Vec<&[crate::types::DataPoint]> = datasets.iter().map(|d| d.points.as_slice()).collect();
        let origin_only = lin_spec.effective_fixed_point() == Some((0.0, 0.0));
        if let Ok(cmp) = compare_linear_lines(&grouped_slices, origin_only) {
            let mut tab = TabBuilder::new("linear_comparison", "Comparison of Lines (ANCOVA)", ResultTabKind::ModelComparison)
                .headers(["F", "DFn, DFd", "P value", "Significantly different?"])
                .row(
                    "Are slopes equal?",
                    [
                        num_cell(cmp.f_slopes),
                        text_cell(format!("{}, {}", cmp.df_slopes.0, cmp.df_slopes.1)),
                        p_cell(cmp.p_slopes, p_fmt),
                        bool_cell(cmp.slopes_significantly_different),
                    ],
                )
                .footnote(format!("Pooled slope = {:.6}", cmp.pooled_slope));
            if let (Some(fi), Some(dfi), Some(pi), Some(sig_i)) =
                (cmp.f_intercepts, cmp.df_intercepts, cmp.p_intercepts, cmp.intercepts_significantly_different)
            {
                tab.push_row(
                    "Are intercepts equal?",
                    [num_cell(fi), text_cell(format!("{}, {}", dfi.0, dfi.1)), p_cell(pi, p_fmt), bool_cell(sig_i)],
                );
            }
            result.tabs.push(tab.build());
        }
    }

    // Interpolated unknowns tab.
    if lin_spec.interpolate_unknowns {
        let mut tab = TabBuilder::new("linear_interpolated", "Interpolated unknowns", ResultTabKind::InterpolatedUnknowns).headers([
            "Input",
            "Interpolated",
            "Mean-Response CI",
            "Calibration / Prediction Interval",
        ]);
        let mut has_rows = false;
        let to_ci_bound = |v: Option<f64>| match v {
            Some(x) if x.is_finite() => CiBound::Finite(x),
            _ => CiBound::Unavailable,
        };
        for (_, lbl, f) in &fits {
            for (dir, items) in [("X from Y", &f.interpolated_x_from_y), ("Y from X", &f.interpolated_y_from_x)] {
                for item in items {
                    has_rows = true;
                    tab.push_row(
                        format!("{lbl} ({dir}={:.4})", item.input_value),
                        [
                            num_cell(item.input_value),
                            num_cell(item.estimate),
                            ResultCell::ConfidenceInterval { lower: item.ci_lower, upper: item.ci_upper },
                            ResultCell::ConfidenceInterval { lower: to_ci_bound(item.pred_lower), upper: to_ci_bound(item.pred_upper) },
                        ],
                    );
                }
            }
        }
        if has_rows {
            result.tabs.push(tab.build());
        }
    }

    append_auc_scalars_and_tab(&mut result, &datasets, None);
    result.status = ResultStatus::UpToDate;
    Ok(result)
}

fn append_linear_observation_counts(tab: &mut TabBuilder, counts: impl Iterator<Item = [usize; 3]>) {
    let mut count_rows = [Vec::new(), Vec::new(), Vec::new()];
    for counts in counts {
        for (row, count) in count_rows.iter_mut().zip(counts) {
            row.push(num_cell(count as f64));
        }
    }
    for (label, row) in ["Observations analyzed", "Excluded by X range", "Residual degrees of freedom"].into_iter().zip(count_rows) {
        tab.push_row(label, row);
    }
}

fn build_deming_result_sheet(
    mut result: ResultSheet,
    datasets: &[DatasetSeries],
    excluded: &[usize],
    options: &SimpleLinearOptions,
    lin_spec: &LinearRegressionSpec,
) -> Result<ResultSheet, FitError> {
    if lin_spec.force_through_origin || lin_spec.fixed_point.is_some() {
        return Err(FitError::InvalidParameter {
            name: "fixed_point",
            reason: String::from("Model II Deming regression does not support forcing the line through the origin or a fixed point"),
        });
    }
    if lin_spec.standard_addition {
        return Err(FitError::InvalidParameter {
            name: "standard_addition",
            reason: String::from("Model II Deming regression does not support standard addition Fieller intervals"),
        });
    }
    if lin_spec.interpolate_unknowns {
        return Err(FitError::InvalidParameter {
            name: "interpolate_unknowns",
            reason: String::from("Model II Deming regression does not support unknown interpolation"),
        });
    }
    let lambda = lin_spec.deming_error.resolved_lambda().ok_or_else(|| FitError::InvalidParameter {
        name: "deming_error",
        reason: String::from("Deming error SDs and variance ratio lambda must be finite and > 0"),
    })?;
    let opts = DemingOptions { variance_ratio_lambda: lambda, confidence_level: options.confidence_level };
    let mut dems = Vec::with_capacity(datasets.len());
    let has_range = options.x_range_filter.is_some();
    let conf_pct = format_percent_label(options.confidence_level.percent());

    for (ds, &n_excluded) in datasets.iter().zip(excluded) {
        if ds.is_pre_summarized {
            return Err(FitError::RequiresRawReplicates);
        }
        let dem = deming_regression(&ds.points, &opts)?;
        let extend_x = if lin_spec.extend_line_to_x_intercept { dem.x_intercept } else { None };
        result.fitted_curves.push(FittedCurveOverlay {
            data_set_index: ds.data_set_index,
            equation_id: "straight_line".to_owned(),
            converged: true,
            ambiguous: false,
            points: build_linear_overlay_points(&ds.points, dem.slope, dem.intercept, options, extend_x)?,
        });
        result.scalars.push(ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(ds.data_set_index), value: dem.slope, units: None });
        result.scalars.push(ResultScalarEntry {
            key: "YIntercept".to_owned(),
            data_set_index: Some(ds.data_set_index),
            value: dem.intercept,
            units: None,
        });
        if let Some(x_int) = dem.x_intercept {
            result.scalars.push(ResultScalarEntry {
                key: "XIntercept".to_owned(),
                data_set_index: Some(ds.data_set_index),
                value: x_int,
                units: None,
            });
        }
        dems.push((ds.label.as_str(), dem, n_excluded));
    }

    let error_footnote = match lin_spec.deming_error {
        DemingErrorSpec::EqualError => String::from("Orthogonal Deming regression uses equal error variances (lambda = 1.0)."),
        DemingErrorSpec::StandardDeviations { sd_x, sd_y } => {
            format!("Deming regression uses SD_X = {sd_x}, SD_Y = {sd_y} (variance ratio lambda = {lambda:.6}).")
        }
        DemingErrorSpec::VarianceRatio { .. } => {
            format!("Deming regression uses error variance ratio lambda = sigma_X^2 / sigma_Y^2 = {lambda:.6}.")
        }
    };
    let mut tab = TabBuilder::new("deming_summary", "Deming (Model II) linear regression", ResultTabKind::MainSummary)
        .headers(dems.iter().map(|(lbl, _, _)| *lbl))
        .row("Slope", dems.iter().map(|(_, d, _)| num_cell(d.slope)))
        .row("Y-intercept", dems.iter().map(|(_, d, _)| num_cell(d.intercept)))
        .row("SE Slope (Jackknife)", dems.iter().map(|(_, d, _)| num_cell(d.se_slope)))
        .row("SE Intercept (Jackknife)", dems.iter().map(|(_, d, _)| num_cell(d.se_intercept)))
        .row(format!("{conf_pct}% CI Slope"), dems.iter().map(|(_, d, _)| ci_cell(d.ci_slope.0, d.ci_slope.1)))
        .row(format!("{conf_pct}% CI Intercept"), dems.iter().map(|(_, d, _)| ci_cell(d.ci_intercept.0, d.ci_intercept.1)))
        .row("Sy.x", dems.iter().map(|(_, d, _)| num_cell(d.sy_x)))
        .row("Variance ratio (lambda)", dems.iter().map(|(_, d, _)| num_cell(d.lambda)))
        .row("X-intercept", dems.iter().map(|(_, d, _)| opt_num_cell(d.x_intercept)))
        .footnotes(["Standard errors computed by Leave-One-Out jackknife.".to_owned(), error_footnote]);
    if has_range {
        append_linear_observation_counts(&mut tab, dems.iter().map(|(_, d, n_excluded)| [d.n, *n_excluded, d.df]));
    }
    result.tabs.push(tab.build());
    append_auc_scalars_and_tab(&mut result, datasets, None);
    result.status = ResultStatus::UpToDate;
    Ok(result)
}

/// Resolve a model from [`NonlinearFitSpec`] (`custom_dsl` first if present, else `equation_id`
/// built-in library).
pub fn resolve_model_from_spec(nl_spec: &NonlinearFitSpec) -> Result<Box<dyn ParametricModel>, FitError> {
    if let Some(src) = &nl_spec.custom_dsl {
        let model = CompiledDslModel::compile(&nl_spec.equation_id, src)?;
        return Ok(Box::new(model));
    }
    get_builtin_model(&nl_spec.equation_id).ok_or_else(|| FitError::UnknownEquation { id: nl_spec.equation_id.clone() })
}

struct PreparedNonlinearFit {
    datasets: Vec<DatasetSeries>,
    mv_y_col_map: Option<HashMap<usize, usize>>,
    model: Box<dyn ParametricModel>,
    config: SolverConfig,
}

fn prepare_nonlinear_fit(spec: &AnalysisSpec, nl_spec: &NonlinearFitSpec, sheet: &DataSheet) -> Result<PreparedNonlinearFit, FitError> {
    if !nl_spec.plot_function_only {
        enforce_summary_mode_matrix(sheet, &AnalysisKind::NonlinearRegression(nl_spec.clone()))?;
    }
    if nl_spec.flag_r_squared_below.is_some_and(|value| !(0.0..=1.0).contains(&value)) {
        return Err(FitError::InvalidParameter { name: "flag_r_squared_below", reason: "threshold must be a finite number from 0 to 1".to_owned() });
    }
    if nl_spec.output_significant_digits.is_some_and(|d| !(1..=15).contains(&d)) {
        return Err(FitError::InvalidParameter { name: "output_significant_digits", reason: "significant digits must be in 1..=15".to_owned() });
    }
    if let Some(rules) = &nl_spec.fit_flag_rules {
        validate_fit_flag_rules(rules)?;
    }
    let effective_y_cols = nl_spec.effective_mv_y_columns();
    let (datasets, mv_y_col_map) = match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            match extract_from_multivar_multi_with_y_map(
                mv,
                &spec.selected_data_sets,
                &nl_spec.mv_group_columns,
                &nl_spec.mv_x_columns,
                &effective_y_cols,
            ) {
                Ok((ds, map)) => (ds, Some(map)),
                Err(FitError::EmptyInput) if nl_spec.plot_function_only => (
                    vec![DatasetSeries {
                        data_set_index: 0,
                        label: nl_spec.equation_id.clone(),
                        points: Vec::new(),
                        unknowns_y: Vec::new(),
                        unknowns_x: Vec::new(),
                        is_pre_summarized: false,
                        column_title_value: None,
                    }],
                    None,
                ),
                Err(err) => return Err(err),
            }
        }
        _ => match extract_datasets_from_sheet_multi(
            sheet,
            &spec.selected_data_sets,
            &nl_spec.mv_group_columns,
            &nl_spec.mv_x_columns,
            &effective_y_cols,
        ) {
            Ok(ds) => (ds, None),
            Err(FitError::EmptyInput) if nl_spec.plot_function_only => (
                vec![DatasetSeries {
                    data_set_index: 0,
                    label: nl_spec.equation_id.clone(),
                    points: Vec::new(),
                    unknowns_y: Vec::new(),
                    unknowns_x: Vec::new(),
                    is_pre_summarized: false,
                    column_title_value: None,
                }],
                None,
            ),
            Err(err) => return Err(err),
        },
    };
    let model = resolve_model_from_spec(nl_spec)?;

    let config = SolverConfig {
        method: nl_spec.method,
        weighting: nl_spec.weighting,
        parameter_rules: nl_spec.parameter_rules.clone(),
        relational_constraints: nl_spec.relational_constraints.clone(),
        confidence: spec.confidence_level,
        max_iters: nl_spec.clamped_max_iters(),
        compute_profile_ci: nl_spec.profile_likelihood_ci,
        identify_unstable_parameters: nl_spec.identify_unstable_parameters,
        identifiability_mode: nl_spec.effective_identifiability_mode(),
        compute_hougaard: true,
        compute_bands: nl_spec.confidence_bands || nl_spec.prediction_bands || nl_spec.plot_function_only,
        compute_prediction_bands: nl_spec.prediction_bands,
        compute_runs_test: !nl_spec.plot_function_only,
        compute_lack_of_fit: nl_spec.replicates_lack_of_fit_test,
        compute_normality: nl_spec.normality_of_residuals,
        compute_homoscedasticity: nl_spec.homoscedasticity_test,
        interpolate_unknowns: nl_spec.interpolate_unknowns,
        use_simplex_warm_start: true,
        x_range_filter: nl_spec.x_range_filter,
        y_range_filter: nl_spec.y_range_filter,
        plot_function_only: nl_spec.plot_function_only,
        plot_x_min: nl_spec.plot_x_min,
        plot_x_max: nl_spec.plot_x_max,
        curve_overlay_segments: 256,
        weighting_source: nl_spec.weighting_source,
        outlier_handling: nl_spec.outlier_handling,
        replicate_fit_mode: nl_spec.replicate_fit_mode,
        convergence_strictness: nl_spec.convergence_strictness,
    };

    Ok(PreparedNonlinearFit { datasets, mv_y_col_map, model, config })
}

/// Compare an existing [`NonlinearFitSpec`] against a diagnostics-only horizontal line (`Y = Mean`)
/// null model without modifying [`ResultSheet`] outputs or excluding any dataset (`R19.7`).
pub fn compare_nonlinear_analysis_vs_horizontal_line(
    spec: &AnalysisSpec,
    sheet: &DataSheet,
    hooks: &HookContext,
) -> Result<HorizontalLineComparisonReport, FitError> {
    let AnalysisKind::NonlinearRegression(nl_spec) = &spec.kind else {
        return Err(FitError::InvalidParameter {
            name: "analysis.kind",
            reason: format!("Horizontal-line comparison requires NonlinearRegression, got `{}`", spec.kind.derive_auto_title()),
        });
    };
    let prepared = prepare_nonlinear_fit(spec, nl_spec, sheet)?;
    let fit = fit_nonlinear(prepared.model.as_ref(), &prepared.datasets, hooks, &prepared.config)?;
    compare_vs_horizontal_line(&fit, &prepared.datasets, hooks, &prepared.config)
}

/// Execute nonlinear regression from [`NonlinearFitSpec`] + [`DataSheet`] and emit a complete [`ResultSheet`].
pub fn execute_nonlinear_analysis(
    spec: &AnalysisSpec,
    nl_spec: &NonlinearFitSpec,
    sheet: &DataSheet,
    hooks: &HookContext,
) -> Result<ResultSheet, FitError> {
    let mut result = ResultSheet::new(spec.clone())?;
    if let Some(dist_spec) = &nl_spec.plot_distribution {
        let plot = evaluate_theoretical_distribution(dist_spec, hooks, nl_spec.plot_x_min, nl_spec.plot_x_max, 256)?;
        populate_distribution_plot_result(&mut result, dist_spec, &plot);
        return Ok(result);
    }
    let PreparedNonlinearFit { datasets, mv_y_col_map, model, config } = prepare_nonlinear_fit(spec, nl_spec, sheet)?;

    let fit = fit_nonlinear(model.as_ref(), &datasets, hooks, &config)?;
    if config.method == curvebook_doc::FitMethod::PoissonDeviance
        && fit.datasets.iter().any(|ds| ds.y_hat.iter().any(|&yh| !yh.is_finite() || yh <= 0.0))
    {
        return Err(FitError::ImpossibleWeights {
            reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
        });
    }
    populate_nonlinear_result_sheet(&mut result, &fit, &datasets, nl_spec, spec.p_value_format);
    if let Some(rout) = &fit.rout {
        let mut outlier_cells: Vec<SourceCellId> = Vec::new();
        for point in rout.audit.iter().filter(|point| point.is_outlier) {
            let col = mv_y_col_map.as_ref().and_then(|m| m.get(&point.data_set_index).copied()).unwrap_or(point.data_set_index);
            if nl_spec.replicate_fit_mode == ReplicateFitMode::RowMeansOnly {
                for subcol in expand_row_outlier_subcolumns(sheet, col, point.row) {
                    outlier_cells.push(SourceCellId { sheet_id: sheet.id.clone(), row: point.row, col, subcol });
                }
            } else {
                outlier_cells.push(SourceCellId { sheet_id: sheet.id.clone(), row: point.row, col, subcol: point.source_subcolumn });
            }
        }
        if config.effective_outlier_handling() == OutlierHandling::Eliminate {
            result.eliminated_outliers.clone_from(&outlier_cells);
        }
        if nl_spec.emit_cleaned_data_tab {
            append_cleaned_data_tab_and_analyzable_sheet(&mut result, sheet, &outlier_cells);
        }
    }

    if let Some((comp_method, comp_question)) = nl_spec.effective_comparison() {
        if matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous) {
            let comp = compare_models_question(
                model.as_ref(),
                &fit,
                &datasets,
                hooks,
                &config,
                comp_method,
                &comp_question,
                nl_spec.prefer_well_defined_fit,
                nl_spec.fit_flag_rules.as_ref(),
            )?;
            append_comparison_tab(
                &mut result,
                &comp.model_a_label,
                &comp.model_a_fit,
                &comp.model_b_label,
                &comp.model_b_fit,
                &comp.outcome,
                spec.p_value_format,
            );
        } else {
            result.checklist.push(ChecklistItem {
                assumption: "Model comparison".to_owned(),
                status: ChecklistStatus::Violated,
                diagnostic_metric: None,
                explanation: "Skipped model comparison because the primary nonlinear fit did not converge.".to_owned(),
                remediation: None,
            });
        }
    }

    let mut filtered_datasets: Option<Vec<DatasetSeries>> = None;
    if !config.plot_function_only && (config.x_range_filter.is_some() || config.y_range_filter.is_some()) {
        let mut copy = datasets.to_vec();
        filter_datasets_by_range(&mut copy, config.x_range_filter, config.y_range_filter);
        filtered_datasets = Some(copy);
    }
    let auc_datasets = filtered_datasets.as_deref().unwrap_or(&datasets);
    let has_custom_plot_bounds = nl_spec.plot_x_min.is_some() || nl_spec.plot_x_max.is_some();
    append_auc_scalars_and_tab(&mut result, auc_datasets, Some((model.as_ref(), &fit, has_custom_plot_bounds)));

    result.status = match fit.status {
        FitStatus::Converged | FitStatus::Ambiguous => ResultStatus::UpToDate,
        FitStatus::HitMaxIters => ResultStatus::Failed {
            code: ErrorCode::SolverFailed,
            message: format!("Nonlinear regression reached the maximum iteration limit ({}) without converging", fit.iterations),
        },
        FitStatus::Diverged => ResultStatus::Failed {
            code: ErrorCode::SolverFailed,
            message: "Nonlinear regression diverged or stalled before reaching convergence".to_owned(),
        },
        FitStatus::BadInitialValues => ResultStatus::Failed {
            code: ErrorCode::SolverFailed,
            message: "Nonlinear regression failed due to invalid initial parameter values".to_owned(),
        },
        FitStatus::ImpossibleWeights => ResultStatus::Failed {
            code: ErrorCode::SolverFailed,
            message: "Nonlinear regression failed due to invalid or non-finite observation weights".to_owned(),
        },
    };
    Ok(result)
}

/// Display names for the fitted datasets, keyed by zero-based source column index.
///
/// Each dataset shows its column title. A blank title falls back to `DS n` (one-based column
/// number), and names shared by several datasets, compared trimmed and case-insensitively, get
/// ` (DS n)` appended so every row label stays unique. These names are display-only: scalar keys,
/// hooks, and equation annotations keep using the parameter name plus `data_set_index`.
fn dataset_display_names(fit: &NonlinearFitResult) -> HashMap<usize, String> {
    let fallback = |idx: usize| format!("DS{}", idx.saturating_add(1));
    let base: Vec<(usize, String)> = fit
        .datasets
        .iter()
        .map(|d| {
            let title = d.label.trim();
            (d.data_set_index, if title.is_empty() { fallback(d.data_set_index) } else { title.to_owned() })
        })
        .collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, name) in &base {
        *counts.entry(name.to_lowercase()).or_insert(0) += 1;
    }
    base.into_iter()
        .map(|(idx, name)| {
            let shared = counts.get(&name.to_lowercase()).copied().unwrap_or(0) > 1;
            let shown = if shared { format!("{name} ({})", fallback(idx)) } else { name };
            (idx, shown)
        })
        .collect()
}

/// Row label for a fitted parameter: the bare name for single-dataset fits, otherwise
/// `"{name} [{dataset display name}]"` (for example `"LogEC50 [VEHICLE]"`).
fn parameter_display_label(fit: &NonlinearFitResult, ds_names: &HashMap<usize, String>, name: &str, dataset_index: Option<usize>) -> String {
    match dataset_index {
        Some(d) if fit.datasets.len() > 1 => {
            let ds = ds_names.get(&d).cloned().unwrap_or_else(|| format!("DS{}", d.saturating_add(1)));
            format!("{name} [{ds}]")
        }
        _ => name.to_owned(),
    }
}

/// Row label for a fixed constant. Per-dataset constants are stored under their stable key
/// (`"AntagConc [DS1]"`), which is matched exactly against [`per_dataset_parameter_key`] and
/// shown with the dataset's display name; any other constant keeps its name.
fn fixed_constant_display_label(fit: &NonlinearFitResult, ds_names: &HashMap<usize, String>, key: &str) -> String {
    if fit.datasets.len() > 1 {
        for ds in &fit.datasets {
            for pname in &ds.param_names {
                if per_dataset_parameter_key(pname, ds.data_set_index) == key {
                    return parameter_display_label(fit, ds_names, pname, Some(ds.data_set_index));
                }
            }
        }
    }
    key.to_owned()
}

fn nonlinear_uses_weighted_objective(fit: &NonlinearFitResult, nl_spec: &NonlinearFitSpec) -> bool {
    if fit.is_robust_cauchy() || nl_spec.method == curvebook_doc::FitMethod::RobustCauchy {
        return false;
    }
    nl_spec.weighting != curvebook_doc::WeightingScheme::None
        || fit.fit_method == curvebook_doc::FitMethod::PoissonDeviance
        || (fit.weighted_ss - fit.rss).abs() > 1e-12 * fit.rss.abs().max(1.0)
}

fn validate_fit_flag_rules(rules: &FitFlagRules) -> Result<(), FitError> {
    if rules.min_r_squared.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
        return Err(FitError::InvalidParameter {
            name: "fit_flag_rules.min_r_squared",
            reason: "min_r_squared must be a finite number in [0, 1]".to_owned(),
        });
    }
    if rules.max_dependency.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
        return Err(FitError::InvalidParameter {
            name: "fit_flag_rules.max_dependency",
            reason: "max_dependency must be a finite number in [0, 1]".to_owned(),
        });
    }
    if rules.min_normality_p.is_some_and(|v| !v.is_finite() || v <= 0.0 || v >= 1.0) {
        return Err(FitError::InvalidParameter {
            name: "fit_flag_rules.min_normality_p",
            reason: "min_normality_p must be a finite probability in (0, 1)".to_owned(),
        });
    }
    if rules.min_runs_or_lof_p.is_some_and(|v| !v.is_finite() || v <= 0.0 || v >= 1.0) {
        return Err(FitError::InvalidParameter {
            name: "fit_flag_rules.min_runs_or_lof_p",
            reason: "min_runs_or_lof_p must be a finite probability in (0, 1)".to_owned(),
        });
    }
    Ok(())
}

fn fmt_fit_scalar(value: f64, sig_figs: Option<u8>) -> String {
    match sig_figs {
        Some(d) => format_significant_digits(value, d),
        None => format!("{value:.6}"),
    }
}

fn dataset_ci_unavailable(fit: &NonlinearFitResult, dataset_index: Option<usize>) -> bool {
    if fit.df == 0 || !matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous) {
        return true;
    }
    dataset_index
        .and_then(|idx| fit.datasets.iter().find(|d| d.data_set_index == idx))
        .is_some_and(|ds| ds.df == 0 || !matches!(ds.status, FitStatus::Converged | FitStatus::Ambiguous))
}

fn nonlinear_param_ci_cell(fit: &NonlinearFitResult, nl_spec: &NonlinearFitSpec, p: &FittedParameterSummary) -> ResultCell {
    if dataset_ci_unavailable(fit, p.dataset_index)
        || fit.is_robust_cauchy()
        || !p.inference_availability.is_available()
        || (!nl_spec.profile_likelihood_ci && (!p.se.is_finite() || fit.covariance.covariance_matrix.is_empty()))
        || (!p.se.is_finite() && matches!(p.ci_lower, CiBound::Open) && matches!(p.ci_upper, CiBound::Open))
    {
        return ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable };
    }
    ci_bounds_cell(p.ci_lower, p.ci_upper)
}

fn dataset_convergence_summary(fit: &NonlinearFitResult) -> Option<String> {
    let total = fit.datasets.len();
    if total <= 1 {
        return None;
    }
    let mut converged = 0_usize;
    let mut ambiguous = 0_usize;
    let mut not_converged = 0_usize;
    for ds in &fit.datasets {
        if !matches!(ds.status, FitStatus::Converged | FitStatus::Ambiguous) {
            not_converged += 1;
        } else if ds.ambiguous || matches!(ds.identifiability, FitIdentifiability::Ambiguous { .. }) || ds.status == FitStatus::Ambiguous {
            ambiguous += 1;
        } else {
            converged += 1;
        }
    }
    let mut parts = vec![format!("{converged} of {total} converged")];
    if ambiguous > 0 {
        parts.push(format!("{ambiguous} ambiguous"));
    }
    if not_converged > 0 {
        parts.push(format!("{not_converged} not converged"));
    }
    Some(parts.join("; "))
}

fn format_unstable_detail(p: &FittedParameterSummary, sig_figs: Option<u8>) -> String {
    let raw_str = if p.estimate.is_finite() { fmt_fit_scalar(p.estimate, sig_figs) } else { String::from("non-finite") };
    let mut parts = vec![format!("Unstable estimate; raw solver iterate = {raw_str}")];
    if p.se.is_finite() {
        parts.push(format!("raw SE = {}", fmt_fit_scalar(p.se, sig_figs)));
    }
    if p.dependency.is_finite() {
        parts.push(format!("Dependency = {:.4}", p.dependency));
    }
    parts.join("; ")
}

fn param_dataset_ambiguous(fit: &NonlinearFitResult, p: &FittedParameterSummary) -> bool {
    p.dataset_index.and_then(|idx| fit.datasets.iter().find(|d| d.data_set_index == idx)).map_or(fit.covariance.ambiguous, |ds| ds.ambiguous)
}

fn should_mask_parameter_as_unstable(p: &FittedParameterSummary, fit: &NonlinearFitResult, mode: Option<IdentifiabilityMode>) -> bool {
    match mode {
        Some(IdentifiabilityMode::ReportAll) => false,
        Some(IdentifiabilityMode::IdentifyAmbiguousFits) => p.notes.contains(&ParameterNote::Unstable) || !p.estimate.is_finite(),
        Some(IdentifiabilityMode::IdentifyUnstableParameters) | None => {
            p.notes.contains(&ParameterNote::Unstable) || !p.estimate.is_finite() || (param_dataset_ambiguous(fit, p) && !p.se.is_finite())
        }
    }
}

fn append_nonlinear_parameter_tab(result: &mut ResultSheet, fit: &NonlinearFitResult, nl_spec: &NonlinearFitSpec) {
    let is_robust = fit.is_robust_cauchy() || nl_spec.method == curvebook_doc::FitMethod::RobustCauchy;
    let show_se = nl_spec.should_show_standard_errors() || is_robust;
    let ident_mode = nl_spec.effective_identifiability_mode();
    let unstable_as_text = !matches!(ident_mode, Some(IdentifiabilityMode::IdentifyAmbiguousFits | IdentifiabilityMode::ReportAll));
    let sig_figs = nl_spec.output_significant_digits;
    let mut tab =
        if show_se {
            TabBuilder::new("nonlinear_parameters", "Best-fit values & confidence intervals", ResultTabKind::ParametersAndConfidenceIntervals)
                .headers(["Best-fit value", "Std. Error", "Confidence Interval", "Dependency", "Hougaard |g1|"])
        } else {
            TabBuilder::new("nonlinear_parameters", "Best-fit values & confidence intervals", ResultTabKind::ParametersAndConfidenceIntervals)
                .headers(["Best-fit value", "Confidence Interval", "Dependency", "Hougaard |g1|"])
        };
    let mut emitted_scalars: HashSet<(String, Option<usize>)> = HashSet::new();
    let ds_names = dataset_display_names(fit);

    for (row_idx, p) in fit.parameters.iter().enumerate() {
        let mut label = parameter_display_label(fit, &ds_names, &p.name, p.dataset_index);
        let flagged_unstable = should_mask_parameter_as_unstable(p, fit, ident_mode);
        if p.notes.contains(&ParameterNote::Unstable) || (flagged_unstable && unstable_as_text) {
            label.push_str(" (Unstable)");
        }
        let prefix = (!flagged_unstable && p.notes.contains(&ParameterNote::Ambiguous)).then(|| "~".to_owned());
        if is_robust || !p.inference_availability.is_available() {
            let na = text_cell("Not applicable");
            if show_se {
                tab.push_row(label, [ResultCell::Number { value: p.estimate, prefix, units: None }, na.clone(), na.clone(), na.clone(), na]);
            } else {
                tab.push_row(label, [ResultCell::Number { value: p.estimate, prefix, units: None }, na.clone(), na.clone(), na]);
            }
        } else if flagged_unstable && unstable_as_text {
            let u_cell = text_cell("Unstable");
            if show_se {
                tab.push_row(label, [u_cell.clone(), u_cell.clone(), u_cell, num_cell(p.dependency), num_cell(p.hougaard_g1.abs())]);
            } else {
                tab.push_row(label, [u_cell.clone(), u_cell, num_cell(p.dependency), num_cell(p.hougaard_g1.abs())]);
            }
            result.cell_details.push(ResultCellDetail {
                tab_id: String::from("nonlinear_parameters"),
                row: row_idx,
                col: 0,
                raw_value: p.estimate.is_finite().then_some(p.estimate),
                detail: format_unstable_detail(p, sig_figs),
            });
        } else if show_se {
            tab.push_row(
                label,
                [
                    ResultCell::Number { value: p.estimate, prefix, units: None },
                    num_cell(p.se),
                    nonlinear_param_ci_cell(fit, nl_spec, p),
                    num_cell(p.dependency),
                    num_cell(p.hougaard_g1.abs()),
                ],
            );
        } else {
            tab.push_row(
                label,
                [
                    ResultCell::Number { value: p.estimate, prefix, units: None },
                    nonlinear_param_ci_cell(fit, nl_spec, p),
                    num_cell(p.dependency),
                    num_cell(p.hougaard_g1.abs()),
                ],
            );
        }
        emitted_scalars.insert((p.name.to_ascii_lowercase(), p.dataset_index));
        if !(flagged_unstable && unstable_as_text) && p.estimate.is_finite() {
            result.scalars.push(ResultScalarEntry { key: p.name.clone(), data_set_index: p.dataset_index, value: p.estimate, units: None });
        }
    }

    for (name, val, _) in &fit.fixed_constants {
        let row_label = format!("{} (Constant)", fixed_constant_display_label(fit, &ds_names, name));
        if show_se {
            tab.push_row(row_label, [num_cell(*val), ResultCell::Empty, text_cell("Fixed"), ResultCell::Empty, ResultCell::Empty]);
        } else {
            tab.push_row(row_label, [num_cell(*val), text_cell("Fixed"), ResultCell::Empty, ResultCell::Empty]);
        }
        emitted_scalars.insert((name.to_ascii_lowercase(), None));
        result.scalars.push(ResultScalarEntry { key: name.clone(), data_set_index: None, value: *val, units: None });
    }

    // Emit raw per-dataset parameter names for per-dataset fixed constants (`HotNM [DS1]`,
    // `AntagConc [DS1]`) with `data_set_index: Some(ds_res.data_set_index)` so TeX substitution
    // and scalar lookups resolve each dataset's constant cleanly.
    for ds_res in &fit.datasets {
        for (pname, &pval) in ds_res.param_names.iter().zip(ds_res.params.iter()) {
            if !pval.is_finite() {
                continue;
            }
            let lower = pname.to_ascii_lowercase();
            if !emitted_scalars.contains(&(lower.clone(), None)) && !emitted_scalars.contains(&(lower.clone(), Some(ds_res.data_set_index))) {
                emitted_scalars.insert((lower, Some(ds_res.data_set_index)));
                result.scalars.push(ResultScalarEntry { key: pname.clone(), data_set_index: Some(ds_res.data_set_index), value: pval, units: None });
            }
        }
    }

    let sy_x_str = if is_robust {
        fit.rsdr.filter(|v| v.is_finite()).map_or_else(
            || "Not applicable (NotApplicableInRobustMode)".to_owned(),
            |rsdr| format!("Not applicable (NotApplicableInRobustMode); RSDR = {}", fmt_fit_scalar(rsdr, sig_figs)),
        )
    } else if fit.rsd.is_finite() {
        fmt_fit_scalar(fit.rsd, sig_figs)
    } else {
        "N/A (DF = 0)".to_owned()
    };

    let status_part = if let Some(conv_summary) = dataset_convergence_summary(fit) {
        format!("Status: {:?} ({conv_summary})", fit.status)
    } else {
        format!("Status: {:?}", fit.status)
    };
    let mut param_footnote = format!(
        "{status_part}; Iterations = {}; DF = {}; R² = {}; Sy.x = {}; RSS = {}",
        fit.iterations,
        fit.df,
        fmt_fit_scalar(fit.r_squared, sig_figs),
        sy_x_str,
        fmt_fit_scalar(fit.rss, sig_figs)
    );
    if nonlinear_uses_weighted_objective(fit, nl_spec) {
        let obj_lbl = if fit.fit_method == curvebook_doc::FitMethod::PoissonDeviance { "Poisson Deviance" } else { "Weighted SS" };
        param_footnote.push_str(&format!("; {obj_lbl} = {}", fmt_fit_scalar(fit.weighted_ss, sig_figs)));
    }
    if !show_se && !is_robust {
        param_footnote
            .push_str("; asymptotic SEs omitted under profile-likelihood CIs (enable show_standard_errors or switch to Wald CIs to display)");
    }

    result.tabs.push(tab.footnote(param_footnote).build());
}

/// Fill the nonlinear result tabs and checklist. Checklist thresholds are heuristics, not
/// published cut-offs: `Dependency > 0.99` warns of near-redundant parameters (the dependency
/// measure is described by Motulsky & Christopoulos 2004), and `|g1| > 1` flags strong
/// nonlinearity on the interpretation scale for Hougaard's skewness given by Ratkowsky (1990).
fn evaluate_dataset_fit_flags(fit: &NonlinearFitResult, ds: &FittedDatasetResult, rules: &FitFlagRules) -> crate::solver::FitFlagOutcome {
    let mut reasons = Vec::new();
    if let Some(min_r2) = rules.min_r_squared
        && ds.r_squared.is_finite()
        && ds.r_squared < min_r2
    {
        reasons.push(format!("R² = {:.4} is below threshold {min_r2:.4}.", ds.r_squared));
    }
    if let Some(min_n) = rules.min_observations
        && ds.n_obs < min_n
    {
        reasons.push(format!("Observations analyzed = {} is below threshold {min_n}.", ds.n_obs));
    }
    if let Some(max_dep) = rules.max_dependency {
        let max_d = fit
            .parameters
            .iter()
            .filter(|p| p.dataset_index == Some(ds.data_set_index) || p.dataset_index.is_none())
            .map(|p| p.dependency)
            .filter(|v| v.is_finite())
            .fold(0.0_f64, f64::max);
        if max_d > max_dep {
            reasons.push(format!("Maximum dependency = {max_d:.4} exceeds threshold {max_dep:.4}."));
        }
    }
    if let Some(min_norm_p) = rules.min_normality_p {
        for norm in [&ds.normality, &ds.normality_anderson_darling, &ds.normality_lilliefors_ks].into_iter().flatten() {
            if norm.p_value.is_finite() && norm.p_value < min_norm_p {
                reasons.push(format!("{} normality p = {:.4} is below threshold {:.4}.", norm.test_name, norm.p_value, min_norm_p));
            }
        }
    }
    if let Some(min_lof_p) = rules.min_runs_or_lof_p {
        if let Some(runs) = &ds.runs_test
            && runs.p_too_few_runs.is_finite()
            && runs.p_too_few_runs < min_lof_p
        {
            reasons.push(format!("Runs test p = {:.4} is below threshold {:.4}.", runs.p_too_few_runs, min_lof_p));
        }
        if let Some(lof) = &ds.lack_of_fit
            && lof.p_value.is_finite()
            && lof.p_value < min_lof_p
        {
            reasons.push(format!("Replicates lack-of-fit test p = {:.4} is below threshold {:.4}.", lof.p_value, min_lof_p));
        }
    }
    if let Some(max_out) = rules.max_rout_outliers
        && ds.n_outliers > max_out
    {
        reasons.push(format!("ROUT outlier count = {} exceeds maximum threshold {max_out}.", ds.n_outliers));
    }
    crate::solver::FitFlagOutcome { flagged: !reasons.is_empty(), reasons }
}

fn append_independent_nonlinear_gof_tab(
    result: &mut ResultSheet,
    fit: &NonlinearFitResult,
    nl_spec: &NonlinearFitSpec,
    p_fmt: PValueFormat,
    weighted_obj: bool,
    weighted_footnote: &str,
) {
    let is_robust = fit.is_robust_cauchy() || nl_spec.method == curvebook_doc::FitMethod::RobustCauchy;
    let is_poisson = fit.fit_method == curvebook_doc::FitMethod::PoissonDeviance;
    let na_cell = || text_cell("Not applicable");
    let ds_names = dataset_display_names(fit);
    let headers: Vec<String> = fit.datasets.iter().map(|ds| ds_names.get(&ds.data_set_index).cloned().unwrap_or_else(|| ds.label.clone())).collect();
    let mut gof = TabBuilder::new("nonlinear_gof", "Goodness of Fit", ResultTabKind::MainSummary).headers(headers);

    gof.push_row("Degrees of Freedom", fit.datasets.iter().map(|ds| num_cell(ds.df as f64)));
    gof.push_row("R squared", fit.datasets.iter().map(|ds| num_cell(ds.r_squared)));
    gof.push_row("Sum of Squares (RSS)", fit.datasets.iter().map(|ds| num_cell(ds.ss_residual)));
    gof.push_row("Sy.x", fit.datasets.iter().map(|ds| if is_robust { na_cell() } else { num_cell(ds.sy_x) }));
    if is_robust && fit.datasets.iter().any(|ds| ds.rsdr.is_some_and(f64::is_finite)) {
        gof.push_row("RSDR (Robust SD of residuals)", fit.datasets.iter().map(|ds| opt_num_cell(ds.rsdr.filter(|v| v.is_finite()))));
    }
    gof.push_row("AICc", fit.datasets.iter().map(|ds| if is_robust { na_cell() } else { opt_num_cell(ds.aic_c) }));
    gof.push_row("BIC", fit.datasets.iter().map(|ds| if is_robust { na_cell() } else { opt_num_cell(ds.bic) }));
    gof.push_row("Number of points analyzed", fit.datasets.iter().map(|ds| num_cell(ds.n_obs as f64)));
    if weighted_obj {
        let label = if is_poisson { "Poisson Deviance" } else { "Weighted Sum of Squares" };
        gof.push_row(label, fit.datasets.iter().map(|ds| num_cell(ds.weighted_ss)));
        gof.push_footnote(weighted_footnote);
    }
    if fit.datasets.iter().any(|ds| ds.runs_test.is_some()) {
        gof.push_row(
            "Runs test P value",
            fit.datasets.iter().map(|ds| ds.runs_test.as_ref().map_or(ResultCell::Empty, |r| p_cell(r.p_too_few_runs, p_fmt))),
        );
    }

    let mut norm_test_names: Vec<String> = Vec::new();
    for ds in &fit.datasets {
        for norm in [&ds.normality, &ds.normality_anderson_darling, &ds.normality_lilliefors_ks].into_iter().flatten() {
            if !norm_test_names.contains(&norm.test_name) {
                norm_test_names.push(norm.test_name.clone());
            }
        }
    }
    for test_name in norm_test_names {
        gof.push_row(
            format!("Normality ({test_name}) P value"),
            fit.datasets.iter().map(|ds| {
                [&ds.normality, &ds.normality_anderson_darling, &ds.normality_lilliefors_ks]
                    .into_iter()
                    .flatten()
                    .find(|norm| norm.test_name == test_name)
                    .map_or(ResultCell::Empty, |norm| p_cell(norm.p_value, p_fmt))
            }),
        );
    }

    if fit.datasets.iter().any(|ds| ds.homoscedasticity.is_some()) {
        gof.push_row(
            "Homoscedasticity (Spearman rho)",
            fit.datasets.iter().map(|ds| ds.homoscedasticity.as_ref().map_or(ResultCell::Empty, |h| num_cell(h.spearman_rho))),
        );
        gof.push_row(
            "Homoscedasticity P value",
            fit.datasets.iter().map(|ds| ds.homoscedasticity.as_ref().map_or(ResultCell::Empty, |h| p_cell(h.p_value, p_fmt))),
        );
    }
    if fit.n_outliers_excluded > 0 {
        gof.push_row("ROUT outliers excluded", fit.datasets.iter().map(|ds| num_cell(ds.n_outliers as f64)));
    } else if fit.rout.is_some() {
        gof.push_row("ROUT outliers identified", fit.datasets.iter().map(|ds| num_cell(ds.n_outliers as f64)));
    }
    if let Some(rules) = nl_spec.fit_flag_rules.as_ref().filter(|r| !nl_spec.plot_function_only && r.has_any_rule()) {
        let per_ds_flags: Vec<_> = fit.datasets.iter().map(|ds| evaluate_dataset_fit_flags(fit, ds, rules)).collect();
        gof.push_row("Fit quality flag", per_ds_flags.iter().map(|f| text_cell(if f.flagged { "Flagged" } else { "Pass" })));
        if per_ds_flags.iter().any(|f| f.flagged) {
            gof.push_row("Flag reasons", per_ds_flags.iter().map(|f| if f.flagged { text_cell(f.reasons.join("; ")) } else { ResultCell::Empty }));
        }
    }
    result.tabs.push(gof.build());
}

fn append_nonlinear_gof_tab(
    result: &mut ResultSheet,
    fit: &NonlinearFitResult,
    nl_spec: &NonlinearFitSpec,
    p_fmt: PValueFormat,
    flag_outcome: Option<&crate::solver::FitFlagOutcome>,
) {
    let is_robust = fit.is_robust_cauchy() || nl_spec.method == curvebook_doc::FitMethod::RobustCauchy;
    let weighted_obj = nonlinear_uses_weighted_objective(fit, nl_spec);
    let is_poisson = fit.fit_method == curvebook_doc::FitMethod::PoissonDeviance;
    let na_cell = || text_cell("Not applicable");
    let weighted_footnote = "R squared, Sum of Squares (RSS), and Sy.x are computed from unweighted residuals in original Y units; parameter SEs, confidence intervals, AICc/BIC, and model comparisons use the weighted / deviance objective; residual normality and homoscedasticity checks use weighted / deviance-standardized residuals.";

    if fit.is_independent_multi_dataset() {
        append_independent_nonlinear_gof_tab(result, fit, nl_spec, p_fmt, weighted_obj, weighted_footnote);
        return;
    }

    let mut gof = KvTabBuilder::new("nonlinear_gof", "Goodness of Fit", ResultTabKind::MainSummary, fit.equation_name.as_str())
        .num("Degrees of Freedom", fit.df as f64)
        .num("R squared", fit.r_squared)
        .num("Sum of Squares (RSS)", fit.rss)
        .cell("Sy.x", if is_robust { na_cell() } else { num_cell(fit.rsd) });
    if is_robust && let Some(rsdr) = fit.rsdr.filter(|v| v.is_finite()) {
        gof.push_num("RSDR (Robust SD of residuals)", rsdr);
    }
    gof.push_cell("AICc", if is_robust { na_cell() } else { opt_num_cell(fit.aic_c) })
        .push_cell("BIC", if is_robust { na_cell() } else { opt_num_cell(fit.bic) })
        .push_num("Number of points analyzed", fit.n_obs as f64);
    if weighted_obj {
        gof.push_num(if is_poisson { "Poisson Deviance" } else { "Weighted Sum of Squares" }, fit.weighted_ss).push_footnote(weighted_footnote);
    }
    if nl_spec.emit_extra_gof {
        gof.push_num("Adjusted R squared", fit.adj_r_squared).push_num("RMSE", fit.rmse);
        if weighted_obj && !is_poisson {
            gof.push_cell("Weighted R squared", opt_num_cell(fit.weighted_r_squared))
                .push_cell("Weighted Adjusted R squared", opt_num_cell(fit.weighted_adj_r_squared))
                .push_cell("Weighted RMSE", opt_num_cell(fit.weighted_rmse))
                .push_cell("Weighted Sy.x", opt_num_cell(fit.weighted_sy_x));
        }
    }
    if let Some(pg) = &fit.poisson_gof {
        gof.push_num("Poisson NLL", pg.nll)
            .push_num("Poisson Null Deviance", pg.null_deviance)
            .push_num("Poisson Deviance Pseudo-R squared", pg.pseudo_r2_deviance)
            .push_num("Poisson Pearson Chi-square", pg.pearson_chi2)
            .push_num("Poisson Dispersion (phi)", pg.dispersion);
    }
    if let Some(r) = &fit.runs_test {
        gof.push_cell("Runs test P value", p_cell(r.p_too_few_runs, p_fmt));
    }
    if fit.datasets.len() > 1 {
        for ds in &fit.datasets {
            if let Some(ds_runs) = &ds.runs_test {
                gof.push_cell(format!("Runs test ({}) P value", ds.label), p_cell(ds_runs.p_too_few_runs, p_fmt));
            }
        }
    }
    for norm in [&fit.normality, &fit.normality_anderson_darling, &fit.normality_lilliefors_ks].into_iter().flatten() {
        gof.push_cell(format!("Normality ({}) P value", norm.test_name), p_cell(norm.p_value, p_fmt));
    }
    if let Some(homo) = &fit.homoscedasticity {
        gof.push_num("Homoscedasticity (Spearman rho)", homo.spearman_rho).push_cell("Homoscedasticity P value", p_cell(homo.p_value, p_fmt));
    }
    if fit.n_outliers_excluded > 0 {
        gof.push_num("ROUT outliers excluded", fit.n_outliers_excluded as f64);
    } else if let Some(rout) = &fit.rout {
        gof.push_num("ROUT outliers identified", rout.outlier_indices.len() as f64);
    }
    if let Some(flags) = flag_outcome {
        gof.push_cell("Fit quality flag", text_cell(if flags.flagged { "Flagged" } else { "Pass" }));
        if flags.flagged {
            gof.push_cell("Flag reasons", text_cell(flags.reasons.join("; ")));
        }
    }
    result.tabs.push(gof.build());
}

fn populate_nonlinear_result_sheet(
    result: &mut ResultSheet,
    fit: &NonlinearFitResult,
    datasets: &[DatasetSeries],
    nl_spec: &NonlinearFitSpec,
    p_fmt: PValueFormat,
) {
    append_nonlinear_parameter_tab(result, fit, nl_spec);
    let is_robust = fit.is_robust_cauchy() || nl_spec.method == curvebook_doc::FitMethod::RobustCauchy;
    let weighted_obj = nonlinear_uses_weighted_objective(fit, nl_spec);
    let flag_outcome =
        nl_spec.fit_flag_rules.as_ref().filter(|r| !nl_spec.plot_function_only && r.has_any_rule()).map(|rules| evaluate_fit_flags(fit, rules));

    append_nonlinear_gof_tab(result, fit, nl_spec, p_fmt, flag_outcome.as_ref());

    if let Some(target_param) = nl_spec.summary_parameter.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        append_parameter_summary_tab_and_data(result, fit, datasets, target_param, nl_spec.should_show_standard_errors() || is_robust, nl_spec);
    }
    if nl_spec.emit_residuals_tab {
        append_nonlinear_residuals_tab(result, fit);
    }
    if nl_spec.emit_covariance_tab && !is_robust {
        append_nonlinear_covariance_tab(result, fit);
    }
    if fit.rout.is_some() {
        append_nonlinear_outliers_tab(result, fit, nl_spec);
    }
    append_nonlinear_interpolated_tab(result, fit);

    // Curve overlays + derived scalars.
    for (idx, ds_res) in fit.datasets.iter().enumerate() {
        if let Some(overlay) = &ds_res.curve_overlay {
            result.fitted_curves.push(overlay.clone());
        }
        if let Some(derived) = fit.derived_scalars.get(idx) {
            for d in derived {
                if d.value.is_finite() {
                    result.scalars.push(ResultScalarEntry {
                        key: d.name.clone(),
                        data_set_index: Some(ds_res.data_set_index),
                        value: d.value,
                        units: d.units.clone(),
                    });
                }
            }
        }
    }
    if fit.r_squared.is_finite() {
        result.scalars.push(ResultScalarEntry { key: "R2".to_owned(), data_set_index: None, value: fit.r_squared, units: None });
    }
    if fit.adj_r_squared.is_finite() {
        result.scalars.push(ResultScalarEntry { key: "AdjR2".to_owned(), data_set_index: None, value: fit.adj_r_squared, units: None });
    }
    if fit.rmse.is_finite() {
        result.scalars.push(ResultScalarEntry { key: "RMSE".to_owned(), data_set_index: None, value: fit.rmse, units: None });
    }
    if fit.rss.is_finite() {
        result.scalars.push(ResultScalarEntry { key: "RSS".to_owned(), data_set_index: None, value: fit.rss, units: None });
    }
    if weighted_obj && fit.weighted_ss.is_finite() {
        result.scalars.push(ResultScalarEntry { key: "Weighted SS".to_owned(), data_set_index: None, value: fit.weighted_ss, units: None });
    }
    if weighted_obj
        && !is_robust
        && let Some(wr2) = fit.weighted_r_squared.filter(|v| v.is_finite())
    {
        result.scalars.push(ResultScalarEntry { key: "WeightedR2".to_owned(), data_set_index: None, value: wr2, units: None });
    }
    if let Some(pg) = &fit.poisson_gof {
        if pg.nll.is_finite() {
            result.scalars.push(ResultScalarEntry { key: "PoissonNLL".to_owned(), data_set_index: None, value: pg.nll, units: None });
        }
        if pg.pseudo_r2_deviance.is_finite() {
            result.scalars.push(ResultScalarEntry {
                key: "PoissonPseudoR2".to_owned(),
                data_set_index: None,
                value: pg.pseudo_r2_deviance,
                units: None,
            });
        }
        if pg.dispersion.is_finite() {
            result.scalars.push(ResultScalarEntry { key: "PoissonDispersion".to_owned(), data_set_index: None, value: pg.dispersion, units: None });
        }
    }
    if is_robust && let Some(rsdr) = fit.rsdr.filter(|v| v.is_finite()) {
        result.scalars.push(ResultScalarEntry { key: "RSDR".to_owned(), data_set_index: None, value: rsdr, units: None });
    }

    append_nonlinear_checklist(result, fit, is_robust);
    if let Some(flags) = flag_outcome {
        if flags.flagged {
            result.checklist.push(ChecklistItem {
                assumption: "Fit quality (Flagged)".to_owned(),
                status: ChecklistStatus::Warn,
                diagnostic_metric: Some("Flagged".to_owned()),
                explanation: flags.reasons.join(" "),
                remediation: None,
            });
        } else {
            result.checklist.push(ChecklistItem {
                assumption: "Fit quality".to_owned(),
                status: ChecklistStatus::Pass,
                diagnostic_metric: Some("Pass".to_owned()),
                explanation: "All configured automated fit-quality flag rules passed.".to_owned(),
                remediation: None,
            });
        }
    }
    if let Some(threshold) = nl_spec.flag_r_squared_below
        && !nl_spec.plot_function_only
        && fit.r_squared.is_finite()
        && fit.r_squared < threshold
    {
        result.checklist.push(ChecklistItem {
            assumption: "R squared below threshold".to_owned(),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(format!("R² = {}, threshold = {threshold}", fit.r_squared)),
            explanation: "The reported unweighted R squared is below the configured threshold.".to_owned(),
            remediation: None,
        });
    }
    if nl_spec.custom_dsl.is_none() && !nl_spec.plot_function_only {
        append_log_x_scale_warning_check(result, &fit.equation_id, datasets, nl_spec.x_range_filter, nl_spec.y_range_filter);
        append_midpoint_extrapolation_checks(result, fit);
    }
}

/// Diagnostic warning emitted when a `log10(X)` dose–response or pharmacological equation is
/// applied to X values that appear to be raw (non-log10) concentrations (`EXT-01`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawXScaleWarning {
    /// Canonical built-in equation ID that expects `log10(X)` predictor values.
    pub equation_id: String,
    /// Suggested non-log counterpart built-in equation ID (e.g. `Some("agonist_vs_response_4pl")`),
    /// or `None` when the model only has a `log10(X)` formulation and `X` should be log10-transformed.
    pub suggested_equation_id: Option<String>,
    /// Minimum finite X value observed across the active datasets.
    pub min_x: f64,
    /// Maximum finite X value observed across the active datasets.
    pub max_x: f64,
    /// Plain-language explanation of why the X values look like raw concentrations.
    pub reason: String,
}

/// Return the non-log concentration counterpart for any paired built-in equation that expects
/// `X = log10(concentration)` (resolving common aliases such as `"4pl"`, `"5pl"`, `"ec50_3pl"`,
/// `"ic50_3pl"`, `"ecf"`, `"gaddum_schild"`).
#[must_use]
pub fn non_log_counterpart_equation_id(equation_id: &str) -> Option<&'static str> {
    let resolved = get_builtin_model(equation_id);
    let canonical = resolved.as_deref().map_or(equation_id, ParametricModel::id);
    match canonical {
        "log_agonist_vs_response_3pl" => Some("agonist_vs_response_3pl"),
        "log_agonist_vs_response_4pl" => Some("agonist_vs_response_4pl"),
        "log_agonist_vs_response_normalized" => Some("agonist_vs_response_normalized"),
        "log_agonist_vs_response_normalized_var" => Some("agonist_vs_response_normalized_var"),
        "log_agonist_vs_response_5pl" => Some("agonist_vs_response_5pl"),
        "log_agonist_vs_response_ecf" => Some("agonist_vs_response_ecf"),
        "log_inhibitor_vs_response_3pl" => Some("inhibitor_vs_response_3pl"),
        "log_inhibitor_vs_response_4pl" => Some("inhibitor_vs_response_4pl"),
        "log_inhibitor_vs_response_normalized" => Some("inhibitor_vs_response_normalized"),
        "log_inhibitor_vs_response_normalized_var" => Some("inhibitor_vs_response_normalized_var"),
        "absolute_ic50_4pl" => Some("absolute_ic50_4pl_conc"),
        "bell_shaped_dose_response" => Some("bell_shaped_dose_response_conc"),
        "biphasic_dose_response" => Some("biphasic_dose_response_conc"),
        "ec50_shift_gaddum_schild" => Some("ec50_shift_gaddum_schild_conc"),
        "ec50_shift_gaddum_schild_unity_log" => Some("ec50_shift_gaddum_schild_unity_conc"),
        "ec50_shift_ratio_log" => Some("ec50_shift_ratio_conc"),
        "allosteric_ec50_shift" => Some("allosteric_ec50_shift_conc"),
        "operational_model_agonist" => Some("operational_model_agonist_conc"),
        "operational_model_depletion_log" => Some("operational_model_depletion_conc"),
        _ => None,
    }
}

/// Return `true` when `equation_id` (or its alias) resolves to any of the 26 built-in equations
/// expecting `X = log10(concentration)` (19 paired dose–response/shift/operational models plus
/// 7 competitive/allosteric binding models).
#[must_use]
pub fn is_log_x_equation_id(equation_id: &str) -> bool {
    if non_log_counterpart_equation_id(equation_id).is_some() {
        return true;
    }
    let resolved = get_builtin_model(equation_id);
    let canonical = resolved.as_deref().map_or(equation_id, ParametricModel::id);
    matches!(
        canonical,
        "one_site_fit_ki"
            | "one_site_fit_logic50"
            | "two_sites_fit_logic50"
            | "two_sites_fit_ki"
            | "homologous_one_site_competition"
            | "one_site_competition_ligand_depletion"
            | "allosteric_modulator_competition"
            | "classic_sigmoidal_dose_response"
            | "classic_sigmoidal_dose_response_variable_slope"
            | "classic_one_site_competition"
            | "classic_two_site_competition"
    )
}

/// Inspect a slice of predictor `X` values and return `(min_x, max_x, reason)` when they appear
/// to be on a raw (non-log10) concentration scale rather than `log10(X)` (`EXT-01`).
///
/// Triggers when at least 2 finite `X` values are present and any of the following hold:
/// - `max(X) > 20` (`10^20` exceeds realistic log10 concentrations),
/// - all `X >= 0` with strictly positive values spanning `>= 2` orders of magnitude (`max / min_pos >= 100`),
/// - all `X >= 0` with `0 < max(X) <= 1e-3` (e.g. raw molar concentrations `1e-9..1e-6`).
#[must_use]
pub fn detect_raw_non_log10_x_scale(xs: &[f64]) -> Option<(f64, f64, String)> {
    let mut count = 0_usize;
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_pos_x = f64::INFINITY;
    for &x in xs {
        if !x.is_finite() {
            continue;
        }
        count += 1;
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        if x > 0.0 {
            min_pos_x = min_pos_x.min(x);
        }
    }
    if count < 2 {
        return None;
    }
    if max_x > 20.0 {
        return Some((
            min_x,
            max_x,
            format!(
                "Observed X values reach {} (> 20), which looks like raw/linear concentrations rather than log10(X).",
                format_significant_digits(max_x, 4)
            ),
        ));
    }
    if min_x >= 0.0 && max_x > 0.0 && max_x <= 1e-3 {
        return Some((
            min_x,
            max_x,
            format!(
                "Observed X values are small non-negative concentrations in [{}, {}], which look like raw molar/linear units rather than log10(X).",
                format_significant_digits(min_x, 4),
                format_significant_digits(max_x, 4)
            ),
        ));
    }
    if min_x >= 0.0 && min_pos_x.is_finite() && min_pos_x > 0.0 && max_x / min_pos_x >= 100.0 {
        return Some((
            min_x,
            max_x,
            format!(
                "Observed non-negative X values span >= 2 orders of magnitude ({} to {}), which looks like raw concentrations rather than log10(X).",
                format_significant_digits(min_pos_x, 4),
                format_significant_digits(max_x, 4)
            ),
        ));
    }
    None
}

/// Detect a mismatch when a `log10(X)` built-in equation is applied to raw/non-log10 `X` values
/// and suggest the corresponding non-log equation when one exists (`EXT-01`).
#[must_use]
pub fn detect_log_x_scale_mismatch(equation_id: &str, xs: &[f64]) -> Option<RawXScaleWarning> {
    if !is_log_x_equation_id(equation_id) {
        return None;
    }
    let (min_x, max_x, reason) = detect_raw_non_log10_x_scale(xs)?;
    let canonical = get_builtin_model(equation_id).map_or_else(|| equation_id.to_owned(), |m| m.id().to_owned());
    let suggested_equation_id = non_log_counterpart_equation_id(&canonical).map(str::to_owned);
    Some(RawXScaleWarning { equation_id: canonical, suggested_equation_id, min_x, max_x, reason })
}

/// Dataset-slice convenience wrapper around [`detect_log_x_scale_mismatch`] (`EXT-01`).
#[must_use]
pub fn detect_log_x_scale_mismatch_datasets(equation_id: &str, datasets: &[DatasetSeries]) -> Option<RawXScaleWarning> {
    let xs: Vec<f64> = datasets.iter().flat_map(|ds| ds.points.iter().map(DataPoint::x0)).collect();
    detect_log_x_scale_mismatch(equation_id, &xs)
}

/// Choose a default 4PL dose–response equation appropriate for the observed `X` scale:
/// `"agonist_vs_response_4pl"` when `xs` looks like raw/non-log10 concentrations, otherwise
/// `"log_agonist_vs_response_4pl"` (`EXT-01`).
#[must_use]
pub fn suggested_default_dose_response_equation(xs: &[f64]) -> &'static str {
    if detect_raw_non_log10_x_scale(xs).is_some() { "agonist_vs_response_4pl" } else { "log_agonist_vs_response_4pl" }
}

fn append_log_x_scale_warning_check(
    result: &mut ResultSheet,
    equation_id: &str,
    datasets: &[DatasetSeries],
    x_range_filter: Option<(Option<f64>, Option<f64>)>,
    y_range_filter: Option<(Option<f64>, Option<f64>)>,
) {
    let xs: Vec<f64> = datasets
        .iter()
        .flat_map(|ds| ds.points.iter())
        .filter(|pt| {
            let x0 = pt.x0();
            let x_ok =
                x_range_filter.is_none_or(|(lo, hi)| lo.is_none_or(|v| !v.is_finite() || x0 >= v) && hi.is_none_or(|v| !v.is_finite() || x0 <= v));
            let y_ok = y_range_filter
                .is_none_or(|(lo, hi)| lo.is_none_or(|v| !v.is_finite() || pt.y >= v) && hi.is_none_or(|v| !v.is_finite() || pt.y <= v));
            x_ok && y_ok
        })
        .map(DataPoint::x0)
        .collect();
    if let Some(warn) = detect_log_x_scale_mismatch(equation_id, &xs) {
        let (metric, explanation) = match &warn.suggested_equation_id {
            Some(alt) => (
                format!("X in [{}, {}]; suggested: {alt}", format_significant_digits(warn.min_x, 4), format_significant_digits(warn.max_x, 4),),
                format!(
                    "{} Switch to the non-log counterpart `{alt}` for raw concentration X values, or transform X to log10(X) before fitting `{}`.",
                    warn.reason, warn.equation_id
                ),
            ),
            None => (
                format!("X in [{}, {}]; transform X to log10(X)", format_significant_digits(warn.min_x, 4), format_significant_digits(warn.max_x, 4),),
                format!("{} Transform X to log10(X) before fitting `{}`.", warn.reason, warn.equation_id),
            ),
        };
        result.checklist.push(ChecklistItem {
            assumption: "X-axis scale matches log10(X) equation".to_owned(),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(metric),
            explanation,
            remediation: None,
        });
    }
}

fn expand_row_outlier_subcolumns(sheet: &DataSheet, col: usize, row: usize) -> Vec<usize> {
    let subcols: Vec<usize> = match &sheet.archetype {
        TableArchetype::Xy(xy) => xy
            .data_sets
            .get(col)
            .and_then(|ds| ds.cells.get(row))
            .map(|row_cells| row_cells.iter().enumerate().filter_map(|(s, c)| c.active_number().map(|_| s)).collect())
            .unwrap_or_default(),
        TableArchetype::Grouped(g) => g
            .data_sets
            .get(col)
            .and_then(|ds| ds.cells.get(row))
            .map(|row_cells| row_cells.iter().enumerate().filter_map(|(s, c)| c.active_number().map(|_| s)).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    if subcols.is_empty() { vec![0] } else { subcols }
}

/// Compare fitted dose–response midpoints to each dataset's observed X extent. Use the
/// parameter in the equation's own X scale. See Sebaugh (2011), Pharmaceutical Statistics
/// 10:128–134, doi:10.1002/pst.426, for the importance of concentration coverage.
fn append_midpoint_extrapolation_checks(result: &mut ResultSheet, fit: &NonlinearFitResult) {
    if !matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous) {
        return;
    }
    for param in &fit.parameters {
        if !matches!(param.name.as_str(), "EC50" | "IC50" | "LogEC50" | "LogIC50") || !param.estimate.is_finite() {
            continue;
        }
        for ds in fit.datasets.iter().filter(|ds| param.dataset_index.is_none_or(|index| index == ds.data_set_index)) {
            let Some((min, max)) = ds.observed_x_range else {
                continue;
            };
            if (min..=max).contains(&param.estimate) {
                continue;
            }
            result.checklist.push(ChecklistItem {
                assumption: format!("{} outside observed X range ({})", param.name, ds.label),
                status: ChecklistStatus::Warn,
                diagnostic_metric: Some(format!("{} = {}, observed X [{min}, {max}]", param.name, param.estimate)),
                explanation: format!("The fitted {} estimate lies outside the X range of observations used for this dataset.", param.name),
                remediation: None,
            });
        }
    }
}

fn solver_convergence_item(fit: &NonlinearFitResult) -> Option<ChecklistItem> {
    let explanation = match fit.status {
        FitStatus::HitMaxIters => format!(
            "The nonlinear solver stopped after reaching the maximum iteration limit ({}) before satisfying convergence tolerances.",
            fit.iterations
        ),
        FitStatus::Diverged => "The nonlinear solver diverged or stalled before reaching a stationary minimum.".to_owned(),
        FitStatus::BadInitialValues => "Initial parameter values failed to evaluate inside the valid model domain.".to_owned(),
        FitStatus::ImpossibleWeights => "Observation weights were non-finite, zero, or incompatible with the data.".to_owned(),
        FitStatus::Converged | FitStatus::Ambiguous => return None,
    };
    Some(ChecklistItem {
        assumption: "Solver convergence".to_owned(),
        status: ChecklistStatus::Violated,
        diagnostic_metric: Some(format!("{:?} (iterations: {})", fit.status, fit.iterations)),
        explanation,
        remediation: None,
    })
}

fn ambiguous_multi_dataset_checklist_item(fit: &NonlinearFitResult, reasons: &[String]) -> ChecklistItem {
    let ds_names = dataset_display_names(fit);
    let mut failing_names: Vec<String> = Vec::new();
    for ds in &fit.datasets {
        let ds_has_unstable = fit.parameters.iter().any(|p| {
            p.dataset_index == Some(ds.data_set_index) && (p.notes.contains(&ParameterNote::Unstable) || p.notes.contains(&ParameterNote::Ambiguous))
        });
        if ds.ambiguous || matches!(ds.identifiability, FitIdentifiability::Ambiguous { .. }) || ds.status == FitStatus::Ambiguous || ds_has_unstable
        {
            let name = ds_names.get(&ds.data_set_index).cloned().unwrap_or_else(|| ds.label.clone());
            if !failing_names.contains(&name) {
                failing_names.push(name);
            }
        }
    }
    let prefix = if failing_names.is_empty() {
        String::from("Ambiguous / unbounded fit detected.")
    } else {
        format!("Ambiguous / unbounded fits in {} of {} datasets ({}).", failing_names.len(), fit.datasets.len(), failing_names.join(", "))
    };
    let metric =
        (!failing_names.is_empty()).then(|| format!("{} of {} datasets ({})", failing_names.len(), fit.datasets.len(), failing_names.join(", ")));
    let joined_reasons = reasons.join(" ");
    ChecklistItem {
        assumption: "Parameter identifiability (Ambiguous)".to_owned(),
        status: ChecklistStatus::Violated,
        diagnostic_metric: metric,
        explanation: if joined_reasons.is_empty() { prefix } else { format!("{prefix} {joined_reasons}") },
        remediation: None,
    }
}

fn parameter_identifiability_checklist_items(fit: &NonlinearFitResult, is_robust: bool) -> Vec<ChecklistItem> {
    if is_robust {
        return vec![ChecklistItem {
            assumption: "Parameter identifiability & asymptotic SEs".to_owned(),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some("NotApplicableInRobustMode".to_owned()),
            explanation: "Asymptotic standard errors, confidence intervals, covariance matrices, confidence bands, and F/AICc comparisons are omitted in Robust (Cauchy) regression.".to_owned(),
            remediation: None,
        }];
    }
    if let FitIdentifiability::Ambiguous { reasons } = &fit.identifiability {
        let ds_names = dataset_display_names(fit);
        if fit.datasets.len() > 1 {
            return vec![ambiguous_multi_dataset_checklist_item(fit, reasons)];
        }
        return reasons
            .iter()
            .map(|reason| ChecklistItem {
                assumption: "Parameter identifiability (Ambiguous)".to_owned(),
                status: ChecklistStatus::Violated,
                diagnostic_metric: fit.datasets.first().map(|d| ds_names.get(&d.data_set_index).cloned().unwrap_or_else(|| d.label.clone())),
                explanation: reason.clone(),
                remediation: None,
            })
            .collect();
    }
    let item = if fit.status == FitStatus::Ambiguous || fit.covariance.ambiguous {
        if fit.datasets.len() > 1 {
            return vec![ambiguous_multi_dataset_checklist_item(fit, &[String::from("One or more parameters are redundant or unidentifiable.")])];
        }
        ChecklistItem {
            assumption: "Parameter identifiability".to_owned(),
            status: ChecklistStatus::Violated,
            diagnostic_metric: Some("Dependency > 0.9999 or rank-deficient Jacobian".to_owned()),
            explanation: "Fit is ambiguous; one or more parameters are redundant or unidentifiable.".to_owned(),
            remediation: None,
        }
    } else if !matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous) {
        ChecklistItem {
            assumption: "Parameter identifiability".to_owned(),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some("Not assessed (unconverged)".to_owned()),
            explanation: "Parameter identifiability was not assessed because the nonlinear solver did not converge.".to_owned(),
            remediation: None,
        }
    } else if fit.parameters.iter().any(|p| p.dependency > 0.99) {
        ChecklistItem {
            assumption: "Multicollinearity (Dependency)".to_owned(),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some("Dependency > 0.99".to_owned()),
            explanation: "Strong correlation among parameters may widen confidence intervals.".to_owned(),
            remediation: None,
        }
    } else if fit.identifiability == FitIdentifiability::Identifiable && fit.status == FitStatus::Converged {
        ChecklistItem {
            assumption: "Parameter identifiability".to_owned(),
            status: ChecklistStatus::Pass,
            diagnostic_metric: None,
            explanation: "All free parameters are cleanly identifiable.".to_owned(),
            remediation: None,
        }
    } else {
        ChecklistItem {
            assumption: "Parameter identifiability".to_owned(),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some("Not assessed".to_owned()),
            explanation: if fit.parameters.is_empty() {
                "No free parameters were fitted; identifiability diagnostics are not applicable.".to_owned()
            } else {
                "Parameter identifiability checks were not run for this fit (enable identify_unstable_parameters to assess).".to_owned()
            },
            remediation: None,
        }
    };
    vec![item]
}

fn append_nonlinear_checklist(result: &mut ResultSheet, fit: &NonlinearFitResult, is_robust: bool) {
    if let Some(conv_item) = solver_convergence_item(fit) {
        result.checklist.push(conv_item);
    }
    result.checklist.extend(parameter_identifiability_checklist_items(fit, is_robust));

    if !is_robust
        && let Some(max_g1) = fit.parameters.iter().map(|p| p.hougaard_g1.abs()).filter(|v| v.is_finite()).reduce(f64::max)
        && max_g1 > 1.0
    {
        result.checklist.push(ChecklistItem {
            assumption: "Hougaard's skewness (nonlinearity)".to_owned(),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(format!("max |g1| = {max_g1:.3}")),
            explanation: "Strong parameter nonlinearity; profile-likelihood CIs are strongly recommended over symmetric CIs.".to_owned(),
            remediation: None,
        });
    }

    if fit.datasets.len() > 1 {
        for ds in &fit.datasets {
            if let Some(ds_runs) = &ds.runs_test {
                result.checklist.push(ChecklistItem {
                    assumption: format!("Runs test for systematic deviation ({})", ds.label),
                    status: if ds_runs.deviates_significantly { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
                    diagnostic_metric: Some(format!("p = {:.4}, runs = {}", ds_runs.p_too_few_runs, ds_runs.runs)),
                    explanation: if ds_runs.deviates_significantly {
                        "Residuals show non-random runs along X within this dataset.".to_owned()
                    } else {
                        "Residuals do not systematically deviate from the fitted curve within this dataset.".to_owned()
                    },
                    remediation: None,
                });
            }
        }
    }

    if let Some(runs) = &fit.runs_test {
        let assumption = if fit.datasets.len() > 1 {
            "Runs test for systematic deviation (combined across datasets)".to_owned()
        } else {
            "Runs test for systematic deviation".to_owned()
        };
        result.checklist.push(ChecklistItem {
            assumption,
            status: if runs.deviates_significantly { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            diagnostic_metric: Some(format!("p = {:.4}, runs = {}", runs.p_too_few_runs, runs.runs)),
            explanation: if runs.deviates_significantly {
                "Residuals show non-random runs along X.".to_owned()
            } else {
                "Residuals do not systematically deviate from the fitted curve.".to_owned()
            },
            remediation: None,
        });
    }

    for norm in [&fit.normality, &fit.normality_anderson_darling, &fit.normality_lilliefors_ks].into_iter().flatten() {
        result.checklist.push(ChecklistItem {
            assumption: format!("Normality of residuals ({})", norm.test_name),
            status: if norm.passed { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
            diagnostic_metric: Some(format!("stat = {:.4}, p = {:.4}", norm.statistic, norm.p_value)),
            explanation: if norm.passed {
                "Residuals pass Gaussian normality check.".to_owned()
            } else {
                "Residuals depart significantly from Gaussian normality.".to_owned()
            },
            remediation: None,
        });
    }

    if let Some(homo) = &fit.homoscedasticity {
        result.checklist.push(ChecklistItem {
            assumption: "Homoscedasticity (Spearman rank |residual| vs Y_hat)".to_owned(),
            status: if homo.homoscedastic { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
            diagnostic_metric: Some(format!("rho = {:.4}, p = {:.4}", homo.spearman_rho, homo.p_value)),
            explanation: if homo.homoscedastic {
                "Residual magnitude shows no significant trend with fitted response.".to_owned()
            } else {
                "Residual magnitude varies systematically with fitted response (heteroscedasticity); consider weighted regression.".to_owned()
            },
            remediation: None,
        });
    }

    if let Some(lof) = &fit.lack_of_fit {
        result.checklist.push(ChecklistItem {
            assumption: "Replicates test for lack of fit".to_owned(),
            status: if lof.significant_lack_of_fit { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            diagnostic_metric: Some(format!("F = {:.3}, p = {:.4}", lof.f_stat, lof.p_value)),
            explanation: if lof.significant_lack_of_fit {
                "Model deviates significantly beyond pure replicate error.".to_owned()
            } else {
                "Model fits within pure replicate error.".to_owned()
            },
            remediation: None,
        });
    }

    if let Some(rout) = &fit.rout {
        let q_label = format_percent_label(rout.q_threshold_percent);
        let explanation = if fit.n_outliers_excluded == 0 && !rout.outlier_indices.is_empty() {
            format!("{} outlier(s) identified and counted (retained in least-squares fit).", rout.outlier_indices.len())
        } else {
            format!("{} outlier(s) identified and excluded before ordinary least-squares refit.", rout.outlier_indices.len())
        };
        result.checklist.push(ChecklistItem {
            assumption: "ROUT outlier detection".to_owned(),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some(format!("Q = {q_label}%, RSDR = {:.4}, outliers = {}", rout.rsdr, rout.outlier_indices.len())),
            explanation,
            remediation: None,
        });
    }
}

fn append_comparison_tab(
    result: &mut ResultSheet,
    model_a_label: &str,
    primary: &NonlinearFitResult,
    model_b_label: &str,
    alt: &NonlinearFitResult,
    outcome: &ModelComparisonOutcome,
    p_fmt: PValueFormat,
) {
    let swapped_note = || {
        format!(
            "Primary fit ({model_a_label}) has fewer residual degrees of freedom ({}) than {model_b_label} ({}); the simpler model serves as the null hypothesis and the primary model serves as the alternative.",
            primary.df, alt.df
        )
    };
    let preferred_label = |null_label: &str, complex_label: &str, prefer_complex: bool| {
        if prefer_complex { format!("{complex_label} (More complex model)") } else { format!("{null_label} (Simpler model)") }
    };
    match outcome {
        ModelComparisonOutcome::ExtraSumOfSquaresF(f) => {
            let (_null_fit, _complex_fit, swapped) = ordered_nested_roles(primary, alt);
            let (null_lbl, complex_lbl) = if swapped { (model_b_label, model_a_label) } else { (model_a_label, model_b_label) };
            let mut kv = KvTabBuilder::new(
                "model_comparison_f",
                "Comparison of Models (Extra sum-of-squares F test)",
                ResultTabKind::ModelComparison,
                "Value",
            )
            .text("Null hypothesis model", null_lbl)
            .text("Alternative model", complex_lbl)
            .text("F (DFn, DFd)", format!("{:.4} ({}, {})", f.f_stat, f.df_num, f.df_den))
            .cell("P value", p_cell(f.p_value, p_fmt))
            .text("Preferred model", preferred_label(null_lbl, complex_lbl, f.preferred_complex_model));
            if swapped {
                kv.push_footnote(swapped_note());
            }
            result.tabs.push(kv.build());
        }
        ModelComparisonOutcome::Aicc(a) => {
            result.tabs.push(
                TabBuilder::new("model_comparison_aicc", "Comparison of Models (AICc)", ResultTabKind::ModelComparison)
                    .headers([model_a_label, model_b_label])
                    .row("AICc", [num_cell(a.aicc_a), num_cell(a.aicc_b)])
                    .row("Akaike weight", [num_cell(a.weight_a), num_cell(a.weight_b)])
                    .row("Difference in AICc", [num_cell(a.delta_aicc), ResultCell::Empty])
                    .row("Evidence ratio", [num_cell(a.evidence_ratio), ResultCell::Empty])
                    .build(),
            );
        }
        ModelComparisonOutcome::LikelihoodRatioTest(lrt) => {
            let (_null_fit, _complex_fit, swapped) = ordered_nested_roles(primary, alt);
            let (null_lbl, complex_lbl) = if swapped { (model_b_label, model_a_label) } else { (model_a_label, model_b_label) };
            let mut kv =
                KvTabBuilder::new("model_comparison_lrt", "Comparison of Models (Likelihood Ratio Test)", ResultTabKind::ModelComparison, "Value")
                    .text("Null hypothesis model", null_lbl)
                    .text("Alternative model", complex_lbl)
                    .num("LR statistic D = 2*(ln L_alt - ln L_null)", lrt.lr_statistic)
                    .num("DF (diff)", lrt.df_diff as f64)
                    .cell("P value", p_cell(lrt.p_value, p_fmt))
                    .text("Preferred model", preferred_label(null_lbl, complex_lbl, lrt.preferred_complex_model));
            if swapped {
                kv.push_footnote(swapped_note());
            }
            result.tabs.push(kv.build());
        }
        ModelComparisonOutcome::PreferredWellDefinedFit { preferred_label, rejected_label, rejection_reasons, .. } => {
            let reasons_text = rejection_reasons.join("; ");
            result.tabs.push(
                KvTabBuilder::new(
                    "model_comparison_well_defined",
                    "Comparison of Models (Well-defined fit pre-check)",
                    ResultTabKind::ModelComparison,
                    "Value",
                )
                .text("Candidate A", model_a_label)
                .text("Candidate B", model_b_label)
                .text("Preferred model", format!("{preferred_label} (Well-defined fit)"))
                .text("Rejected model", rejected_label.as_str())
                .text("Rejection reasons", reasons_text.as_str())
                .footnote("Selected the well-defined fit directly without computing Extra-SS F, LRT, or AICc because the competing candidate was Ambiguous, Unstable, or Flagged.")
                .build(),
            );
        }
        ModelComparisonOutcome::NotApplicableInRobustMode { requested_test, reason } => {
            let id = if requested_test.contains("AICc") {
                "model_comparison_aicc"
            } else if requested_test.contains("Likelihood Ratio") {
                "model_comparison_lrt"
            } else {
                "model_comparison_f"
            };
            result.tabs.push(
                KvTabBuilder::new(id, format!("Comparison of Models ({requested_test})"), ResultTabKind::ModelComparison, "Value")
                    .text("Null hypothesis model", model_a_label)
                    .text("Alternative model", model_b_label)
                    .text("Test statistic", "Not applicable")
                    .text("P value", "Not applicable")
                    .text("Preferred model", "Not applicable")
                    .text("Inference availability", "Not applicable (NotApplicableInRobustMode)")
                    .footnote(reason.as_str())
                    .build(),
            );
        }
    }
}

struct ParamSummaryRow {
    label: String,
    title_x: Option<f64>,
    est: f64,
    se: f64,
    ci_cell: ResultCell,
    is_unstable: bool,
}

/// Emit an analyzable parameter summary `ResultTab` and companion `analyzable_data` (`XyTable`
/// when all dataset column titles parse to finite numeric values, `ColumnTable` otherwise; §2.5.3).
fn append_parameter_summary_tab_and_data(
    result: &mut ResultSheet,
    fit: &NonlinearFitResult,
    datasets: &[DatasetSeries],
    target_param: &str,
    show_se: bool,
    nl_spec: &NonlinearFitSpec,
) {
    let ds_names = dataset_display_names(fit);
    let ident_mode = nl_spec.effective_identifiability_mode();
    let unstable_as_text = !matches!(ident_mode, Some(IdentifiabilityMode::IdentifyAmbiguousFits | IdentifiabilityMode::ReportAll));
    let mut rows_data: Vec<ParamSummaryRow> = Vec::new();
    for (idx, ds_res) in fit.datasets.iter().enumerate() {
        let label = ds_names.get(&ds_res.data_set_index).cloned().unwrap_or_else(|| ds_res.label.clone());
        let title_x = datasets.iter().find(|d| d.data_set_index == ds_res.data_set_index).and_then(|d| d.column_title_value);
        if let Some(p) = fit
            .parameters
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(target_param) && (p.dataset_index == Some(ds_res.data_set_index) || p.dataset_index.is_none()))
        {
            let is_unstable = unstable_as_text && should_mask_parameter_as_unstable(p, fit, ident_mode);
            let ci_cell = if p.estimate.is_finite() {
                nonlinear_param_ci_cell(fit, nl_spec, p)
            } else {
                ci_bounds_cell(CiBound::Unavailable, CiBound::Unavailable)
            };
            rows_data.push(ParamSummaryRow { label, title_x, est: p.estimate, se: p.se, ci_cell, is_unstable });
        } else if let Some(d) = fit.derived_scalars.get(idx).and_then(|list| list.iter().find(|d| d.name.eq_ignore_ascii_case(target_param))) {
            rows_data.push(ParamSummaryRow {
                label,
                title_x,
                est: d.value,
                se: f64::NAN,
                ci_cell: ci_bounds_cell(CiBound::Unavailable, CiBound::Unavailable),
                is_unstable: false,
            });
        } else if let Some(pos) = ds_res.param_names.iter().position(|n| n.eq_ignore_ascii_case(target_param))
            && let Some(&val) = ds_res.params.get(pos)
        {
            rows_data.push(ParamSummaryRow {
                label,
                title_x,
                est: val,
                se: f64::NAN,
                ci_cell: ci_bounds_cell(CiBound::Unavailable, CiBound::Unavailable),
                is_unstable: false,
            });
        }
    }
    if rows_data.is_empty() {
        return;
    }

    let has_numeric_x = rows_data.iter().all(|row| row.title_x.is_some_and(f64::is_finite));
    let mut tab =
        if has_numeric_x && show_se {
            TabBuilder::new("nonlinear_parameter_summary", format!("Parameter Summary ({target_param})"), ResultTabKind::DescriptiveSummary)
                .headers(["X (Column Title)", target_param, "Std. Error", "Confidence Interval"])
        } else if has_numeric_x {
            TabBuilder::new("nonlinear_parameter_summary", format!("Parameter Summary ({target_param})"), ResultTabKind::DescriptiveSummary)
                .headers(["X (Column Title)", target_param, "Confidence Interval"])
        } else if show_se {
            TabBuilder::new("nonlinear_parameter_summary", format!("Parameter Summary ({target_param})"), ResultTabKind::DescriptiveSummary)
                .headers([target_param, "Std. Error", "Confidence Interval"])
        } else {
            TabBuilder::new("nonlinear_parameter_summary", format!("Parameter Summary ({target_param})"), ResultTabKind::DescriptiveSummary)
                .headers([target_param, "Confidence Interval"])
        };

    for (r_idx, row) in rows_data.iter().enumerate() {
        let (val_cell, se_cell, ci_cell) = if row.is_unstable {
            let u = text_cell("Unstable");
            let val_col = usize::from(has_numeric_x);
            let raw_str = if row.est.is_finite() { format_significant_digits(row.est, 6) } else { String::from("non-finite") };
            result.cell_details.push(ResultCellDetail {
                tab_id: String::from("nonlinear_parameter_summary"),
                row: r_idx,
                col: val_col,
                raw_value: row.est.is_finite().then_some(row.est),
                detail: format!("Unstable estimate for {} ({}); raw solver iterate = {raw_str}", target_param, row.label),
            });
            (u.clone(), u.clone(), u)
        } else {
            (num_cell(row.est), num_cell(row.se), row.ci_cell.clone())
        };
        match (has_numeric_x, show_se) {
            (true, true) => {
                tab.push_row(row.label.as_str(), [opt_num_cell(row.title_x), val_cell, se_cell, ci_cell]);
            }
            (true, false) => {
                tab.push_row(row.label.as_str(), [opt_num_cell(row.title_x), val_cell, ci_cell]);
            }
            (false, true) => {
                tab.push_row(row.label.as_str(), [val_cell, se_cell, ci_cell]);
            }
            (false, false) => {
                tab.push_row(row.label.as_str(), [val_cell, ci_cell]);
            }
        }
    }
    result.tabs.push(tab.build());

    // Build companion `analyzable_data` for downstream secondary regression or bar/XY graphing.
    let sheet_id = format!("{}_param_summary", result.id.as_str());
    let sheet_title = format!("Parameter Summary ({target_param})");
    if has_numeric_x {
        if let Ok(mut xy) = XyTable::new(SubcolumnMode::Single, 1, rows_data.len()) {
            xy.x_title = "Column Title X".to_owned();
            if let Some(ds) = xy.data_sets.first_mut() {
                ds.title = target_param.to_owned();
            }
            for (r, row) in rows_data.iter().enumerate() {
                if let Some(rt) = xy.row_titles.get_mut(r) {
                    *rt = row.label.clone();
                }
                if let Some(x) = row.title_x.filter(|v| v.is_finite()) {
                    let _ = xy.set_x_cell(r, Cell::number(x));
                }
                if !row.is_unstable
                    && row.est.is_finite()
                    && let Some(cell) = xy.data_sets.first_mut().and_then(|d| d.cells.get_mut(r)).and_then(|row_cells| row_cells.first_mut())
                {
                    *cell = Cell::number(row.est);
                }
            }
            if let Ok(ds_sheet) = DataSheet::new(sheet_id, sheet_title, TableArchetype::Xy(xy)) {
                result.analyzable_data = Some(ds_sheet);
            }
        }
    } else if let Ok(mut col) = ColumnTable::new(ColumnDesignHint::default(), rows_data.len(), 1) {
        for (c, row) in rows_data.iter().enumerate() {
            if let Some(ds) = col.data_sets.get_mut(c) {
                ds.title = row.label.clone();
                if !row.is_unstable
                    && row.est.is_finite()
                    && let Some(cell) = ds.cells.first_mut().and_then(|row_cells| row_cells.first_mut())
                {
                    *cell = Cell::number(row.est);
                }
            }
        }
        if let Ok(ds_sheet) = DataSheet::new(sheet_id, sheet_title, TableArchetype::Column(col)) {
            result.analyzable_data = Some(ds_sheet);
        }
    }
}

/// Residual tab including normal Q-Q theoretical quantiles at Blom (1958) plotting positions
/// `(i - 3/8) / (n + 1/4)`.
fn append_nonlinear_residuals_tab(result: &mut ResultSheet, fit: &NonlinearFitResult) {
    let total_pts: usize = fit.datasets.iter().map(|d| d.y_hat.len()).sum();
    if total_pts == 0 {
        return;
    }

    let mut raw_list = Vec::with_capacity(total_pts);
    for ds in &fit.datasets {
        for &r in &ds.residuals {
            raw_list.push(r);
        }
    }
    let mut order: Vec<usize> = (0..total_pts).collect();
    order.sort_by(|&a, &b| raw_list[a].total_cmp(&raw_list[b]));
    let mut qq_quantiles = vec![0.0_f64; total_pts];
    let denom = (total_pts as f64) + 0.25;
    for (rank, &orig_idx) in order.iter().enumerate() {
        let prob = ((rank as f64) + 0.625) / denom;
        qq_quantiles[orig_idx] = norm_ppf(prob).unwrap_or(0.0);
    }

    let multi_ds = fit.datasets.len() > 1;
    let mut tab = TabBuilder::new("nonlinear_residuals", "Residuals", ResultTabKind::Residuals).headers([
        "Predicted Y",
        "Residual",
        "Weighted Residual",
        "Studentized Residual",
        "Q-Q Normal Quantile",
        "Data Set",
    ]);
    let mut flat_idx = 0_usize;
    for ds in &fit.datasets {
        let ds_label = if ds.label.trim().is_empty() { format!("DS{}", ds.data_set_index + 1) } else { ds.label.clone() };
        for i in 0..ds.y_hat.len() {
            let lbl = if multi_ds { format!("{} #{}", ds.label, i + 1) } else { format!("Pt {}", i + 1) };
            let yh = ds.y_hat[i];
            let res = ds.residuals.get(i).copied().unwrap_or(f64::NAN);
            let w_res = ds.weighted_residuals.get(i).copied().unwrap_or(f64::NAN);
            let stud_cell = if fit.is_robust_cauchy() {
                text_cell("Not applicable")
            } else {
                num_cell(ds.studentized_residuals.get(i).copied().unwrap_or(f64::NAN))
            };
            let qq = qq_quantiles[flat_idx];
            tab.push_row(lbl, [num_cell(yh), num_cell(res), num_cell(w_res), stud_cell, num_cell(qq), text_cell(ds_label.clone())]);
            flat_idx += 1;
        }
    }
    result.tabs.push(tab.build());
}

fn append_nonlinear_covariance_tab(result: &mut ResultSheet, fit: &NonlinearFitResult) {
    if fit.covariance.covariance_matrix.is_empty() {
        return;
    }
    let ds_names = dataset_display_names(fit);
    let labels: Vec<String> = fit.parameters.iter().map(|p| parameter_display_label(fit, &ds_names, &p.name, p.dataset_index)).collect();
    let mut tab = TabBuilder::new("nonlinear_covariance", "Covariance Matrix", ResultTabKind::CovarianceMatrix)
        .headers(labels.iter().map(String::as_str))
        .footnote(format!("Numerical rank: {}/{}", fit.covariance.numerical_rank, fit.parameters.len()));
    for (label, row) in labels.into_iter().zip(&fit.covariance.covariance_matrix) {
        tab.push_row(label, row.iter().map(|&v| num_cell(v)));
    }
    result.tabs.push(tab.build());
}

fn append_nonlinear_outliers_tab(result: &mut ResultSheet, fit: &NonlinearFitResult, nl_spec: &NonlinearFitSpec) {
    let Some(rout) = &fit.rout else {
        return;
    };
    let eliminated = nl_spec.effective_outlier_handling() == OutlierHandling::Eliminate;
    let action_label = if eliminated { "Eliminated" } else { "Identified (retained)" };
    let ds_names = dataset_display_names(fit);
    let q_label = format_percent_label(rout.q_threshold_percent);
    let mut tab = TabBuilder::new("nonlinear_outliers", "Outliers", ResultTabKind::Diagnostics)
        .headers(["Dataset", "Row", "X", "Y", "Weighted Residual", "Status"])
        .footnote(format!("ROUT outlier detection (Q = {q_label}%, RSDR = {:.4}, df = {}).", rout.rsdr, rout.df));

    let mut per_ds_counts: HashMap<usize, usize> = HashMap::new();
    let mut count = 0_usize;
    for pt in rout.audit.iter().filter(|p| p.is_outlier) {
        count += 1;
        *per_ds_counts.entry(pt.data_set_index).or_insert(0) += 1;
        let ds_label = ds_names.get(&pt.data_set_index).cloned().unwrap_or_else(|| format!("DS{}", pt.data_set_index.saturating_add(1)));
        tab.push_row(
            format!("Outlier {count}"),
            [text_cell(ds_label), num_cell((pt.row + 1) as f64), num_cell(pt.x0), num_cell(pt.y), num_cell(pt.residual), text_cell(action_label)],
        );
    }
    if count == 0 {
        tab.push_row(
            "Summary",
            [
                text_cell("All datasets"),
                ResultCell::Empty,
                ResultCell::Empty,
                ResultCell::Empty,
                ResultCell::Empty,
                text_cell("No outliers detected"),
            ],
        );
    }
    for ds in &fit.datasets {
        let ds_count = per_ds_counts.get(&ds.data_set_index).copied().unwrap_or(0);
        result.scalars.push(ResultScalarEntry {
            key: "ROUT outliers".to_owned(),
            data_set_index: Some(ds.data_set_index),
            value: ds_count as f64,
            units: None,
        });
    }
    result.scalars.push(ResultScalarEntry {
        key: "ROUT outliers".to_owned(),
        data_set_index: None,
        value: rout.outlier_indices.len() as f64,
        units: None,
    });
    result.tabs.push(tab.build());
}

fn append_cleaned_data_tab_and_analyzable_sheet(result: &mut ResultSheet, sheet: &DataSheet, outlier_cells: &[SourceCellId]) {
    let mut cleaned = sheet.clone();
    cleaned.id = result.id.clone();
    cleaned.title = format!("{} (Cleaned data)", sheet.title);
    for cid in outlier_cells {
        match &mut cleaned.archetype {
            TableArchetype::Xy(xy) => {
                if let Some(cell) = xy.data_sets.get_mut(cid.col).and_then(|ds| ds.cells.get_mut(cid.row)).and_then(|row| row.get_mut(cid.subcol)) {
                    cell.excluded = true;
                }
            }
            TableArchetype::Grouped(g) => {
                if let Some(cell) = g.data_sets.get_mut(cid.col).and_then(|ds| ds.cells.get_mut(cid.row)).and_then(|row| row.get_mut(cid.subcol)) {
                    cell.excluded = true;
                }
            }
            TableArchetype::MultipleVariables(mv) => {
                if let Some(cell) = mv.variables.get_mut(cid.col).and_then(|v| v.cells.get_mut(cid.row)) {
                    cell.excluded = true;
                }
            }
            _ => {}
        }
    }

    let mut headers: Vec<String> = Vec::new();
    let mut rows: Vec<(String, Vec<ResultCell>)> = Vec::new();
    match &cleaned.archetype {
        TableArchetype::Xy(xy) => {
            headers.push(if xy.x_title.trim().is_empty() { "X".to_owned() } else { xy.x_title.clone() });
            let subcols = xy.y_subcolumn_mode.subcolumn_count().max(1);
            for ds in &xy.data_sets {
                if subcols == 1 {
                    headers.push(ds.title.clone());
                } else {
                    for s in 0..subcols {
                        headers.push(format!("{} [{}]", ds.title, s + 1));
                    }
                }
            }
            let n_rows = xy.x_cells.len();
            for r in 0..n_rows {
                let mut row_cells = Vec::with_capacity(headers.len());
                let x_cell = xy.x_summary_at(r).ok().flatten().map_or(ResultCell::Empty, num_cell);
                row_cells.push(x_cell);
                for ds in &xy.data_sets {
                    for s in 0..subcols {
                        let c = ds.cells.get(r).and_then(|rc| rc.get(s)).and_then(curvebook_doc::Cell::active_number);
                        row_cells.push(c.map_or(ResultCell::Empty, num_cell));
                    }
                }
                rows.push((format!("Row {}", r + 1), row_cells));
            }
        }
        TableArchetype::Grouped(g) => {
            let subcols = g.subcolumn_mode.subcolumn_count().max(1);
            for ds in &g.data_sets {
                if subcols == 1 {
                    headers.push(ds.title.clone());
                } else {
                    for s in 0..subcols {
                        headers.push(format!("{} [{}]", ds.title, s + 1));
                    }
                }
            }
            let n_rows = g.row_titles.len();
            for r in 0..n_rows {
                let mut row_cells = Vec::with_capacity(headers.len());
                for ds in &g.data_sets {
                    for s in 0..subcols {
                        let c = ds.cells.get(r).and_then(|rc| rc.get(s)).and_then(curvebook_doc::Cell::active_number);
                        row_cells.push(c.map_or(ResultCell::Empty, num_cell));
                    }
                }
                let lbl = g.row_titles.get(r).filter(|t| !t.trim().is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1));
                rows.push((lbl, row_cells));
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            for var in &mv.variables {
                headers.push(var.title.clone());
            }
            let n_rows = mv.row_count();
            for r in 0..n_rows {
                let mut row_cells = Vec::with_capacity(headers.len());
                for var in &mv.variables {
                    let cell = var.cells.get(r);
                    let rc = match cell {
                        Some(c) if !c.excluded => match &c.value {
                            curvebook_doc::CellValue::Number(v) if v.is_finite() => num_cell(*v),
                            curvebook_doc::CellValue::Text(s) if !s.trim().is_empty() => text_cell(s.clone()),
                            _ => ResultCell::Empty,
                        },
                        _ => ResultCell::Empty,
                    };
                    row_cells.push(rc);
                }
                rows.push((format!("Row {}", r + 1), row_cells));
            }
        }
        _ => {}
    }

    let mut tab = TabBuilder::new("cleaned_data", "Cleaned data", ResultTabKind::MainSummary)
        .headers(headers.iter().map(String::as_str))
        .footnote(format!("{} ROUT outlier cell(s) excluded from cleaned data table.", outlier_cells.len()));
    for (lbl, row_cells) in rows {
        tab.push_row(lbl, row_cells);
    }
    result.tabs.push(tab.build());
    if result.analyzable_data.is_none() {
        result.analyzable_data = Some(cleaned);
    }
}

fn append_nonlinear_interpolated_tab(result: &mut ResultSheet, fit: &NonlinearFitResult) {
    let inverse = || fit.interpolated_x.iter().map(|item| (item, "X from Y"));
    append_interpolated_tab(
        result,
        fit,
        "nonlinear_interpolated",
        "Interpolated unknowns",
        inverse()
            .filter(|(item, _)| item.replicate_subcolumn.is_none() && item.replicate_mean_count.is_none())
            .chain(fit.interpolated_y.iter().map(|item| (item, "Y from X"))),
    );
    append_interpolated_tab(
        result,
        fit,
        "nonlinear_interpolated_mean_x",
        "Interpolated Mean X",
        inverse().filter(|(item, _)| item.replicate_mean_count.is_some()),
    );
    append_interpolated_tab(
        result,
        fit,
        "nonlinear_interpolated_replicates",
        "Interpolated Replicates",
        inverse().filter(|(item, _)| item.replicate_subcolumn.is_some()),
    );

    // Expose dual `Interpolated Mean X` and `Interpolated Replicates` rows as `analyzable_data`
    // using `SubcolumnMode::UpperLowerLimits` on X (`[estimate, ci_lower, ci_upper]`) so downstream
    // `Transform` (`10^X` / `e^X` antilog) converts estimates and CI endpoints cleanly (§2.5.5).
    if result.analyzable_data.is_none() {
        let dual_rows: Vec<&InterpolatedUnknown> = fit
            .interpolated_x
            .iter()
            .filter(|item| item.replicate_mean_count.is_some())
            .chain(fit.interpolated_x.iter().filter(|item| item.replicate_subcolumn.is_some()))
            .collect();
        if !dual_rows.is_empty()
            && let Ok(mut xy) = XyTable::new_with_x_mode(
                XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::UpperLowerLimits },
                SubcolumnMode::Single,
                1,
                dual_rows.len(),
            )
        {
            xy.x_title = "Interpolated X".to_owned();
            if let Some(ds) = xy.data_sets.first_mut() {
                ds.title = "Input Y".to_owned();
            }
            for (r, item) in dual_rows.iter().enumerate() {
                let lbl = fit
                    .datasets
                    .iter()
                    .find(|d| d.data_set_index == item.data_set_index)
                    .map_or_else(|| format!("DS{}", item.data_set_index + 1), |d| d.label.clone());
                let source = if let Some(subcol) = item.replicate_subcolumn {
                    format!("row {}, replicate {}", item.row + 1, subcol + 1)
                } else if let Some(count) = item.replicate_mean_count {
                    format!("row {}, mean of {count}", item.row + 1)
                } else {
                    format!("row {}", item.row + 1)
                };
                if let Some(rt) = xy.row_titles.get_mut(r) {
                    *rt = format!("{lbl} ({source})");
                }
                if let Some(x_row) = xy.x_cells.get_mut(r) {
                    if item.output_estimate.is_finite()
                        && let Some(c0) = x_row.first_mut()
                    {
                        *c0 = Cell::number(item.output_estimate);
                    }
                    let suppress_ci = fit.is_robust_cauchy() || !item.inference_availability.is_available();
                    if !suppress_ci {
                        if let CiBound::Finite(lo) = item.ci_lower
                            && lo.is_finite()
                            && let Some(c1) = x_row.get_mut(1)
                        {
                            *c1 = Cell::number(lo);
                        }
                        if let CiBound::Finite(hi) = item.ci_upper
                            && hi.is_finite()
                            && let Some(c2) = x_row.get_mut(2)
                        {
                            *c2 = Cell::number(hi);
                        }
                    }
                }
                if item.input_value.is_finite()
                    && let Some(cell) = xy.data_sets.first_mut().and_then(|d| d.cells.get_mut(r)).and_then(|row| row.first_mut())
                {
                    *cell = Cell::number(item.input_value);
                }
            }
            let sheet_id = format!("{}_interpolated", result.id.as_str());
            if let Ok(ds_sheet) = DataSheet::new(sheet_id, "Interpolated Unknowns", TableArchetype::Xy(xy)) {
                result.analyzable_data = Some(ds_sheet);
            }
        }
    }
}

fn append_interpolated_tab<'a>(
    result: &mut ResultSheet,
    fit: &NonlinearFitResult,
    id: &str,
    title: &str,
    rows: impl Iterator<Item = (&'a InterpolatedUnknown, &'static str)>,
) {
    let mut tab = TabBuilder::new(id, title, ResultTabKind::InterpolatedUnknowns).headers([
        "Input",
        "Interpolated",
        "Mean-Response CI",
        "Calibration / Prediction Interval",
        "Direction",
        "Dataset",
    ]);
    let mut has_rows = false;
    let mut has_robust_ci_suppression = false;

    for (item, direction) in rows {
        has_rows = true;
        let lbl = fit
            .datasets
            .iter()
            .find(|d| d.data_set_index == item.data_set_index)
            .map_or_else(|| format!("DS{}", item.data_set_index + 1), |d| d.label.clone());
        let source = if let Some(subcol) = item.replicate_subcolumn {
            format!("row {}, replicate {}", item.row + 1, subcol + 1)
        } else if let Some(count) = item.replicate_mean_count {
            format!("row {}, mean of {count}", item.row + 1)
        } else {
            format!("{direction}={:.4}", item.input_value)
        };
        let suppress_ci = fit.is_robust_cauchy() || !item.inference_availability.is_available();
        if suppress_ci {
            has_robust_ci_suppression = true;
        }
        let uncomputed_ci = !item.output_estimate.is_finite() || fit.df == 0 || !matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous);
        let ci_rendered = if suppress_ci {
            text_cell("Not applicable")
        } else if uncomputed_ci {
            ci_bounds_cell(CiBound::Unavailable, CiBound::Unavailable)
        } else {
            ci_bounds_cell(item.ci_lower, item.ci_upper)
        };
        let pred_rendered = if suppress_ci {
            text_cell("Not applicable")
        } else if uncomputed_ci {
            ci_bounds_cell(CiBound::Unavailable, CiBound::Unavailable)
        } else {
            ci_bounds_cell(item.pred_lower, item.pred_upper)
        };
        tab.push_row(
            format!("{lbl} ({source})"),
            [num_cell(item.input_value), num_cell(item.output_estimate), ci_rendered, pred_rendered, text_cell(direction), text_cell(lbl.as_str())],
        );
        if let Some(note) = &item.note {
            let source = if item.replicate_subcolumn.is_some() || item.replicate_mean_count.is_some() {
                source
            } else {
                format!("row {}, {direction}", item.row + 1)
            };
            tab.push_footnote(format!("{lbl} ({source}): {note}"));
        }
    }
    if !has_rows {
        return;
    }
    if has_robust_ci_suppression {
        tab.push_footnote(
            "Confidence intervals for interpolated unknowns are not applicable under Robust (Cauchy) regression (NotApplicableInRobustMode).",
        );
    }
    result.tabs.push(tab.build());
}

#[inline]
fn canonical_zero(x: f64) -> f64 {
    if x == 0.0 { 0.0 } else { x }
}

fn build_linear_overlay_points(
    points: &[DataPoint],
    slope: f64,
    intercept: f64,
    options: &SimpleLinearOptions,
    extend_x_intercept: Option<f64>,
) -> Result<Vec<CurveSamplePoint>, FitError> {
    let bounds = resolve_linear_grid_bounds(
        points.iter().map(DataPoint::x0),
        options.x_range_filter.unwrap_or_default(),
        options.plot_x_range.unwrap_or_default(),
        extend_x_intercept,
    )?;
    linear_grid(bounds, options.band_grid_points)
        .map(|x| {
            let y = intercept + slope * x;
            if !y.is_finite() {
                return Err(FitError::InvalidParameter { name: "plot_x_range", reason: "plot range produced a non-finite fitted value".to_owned() });
            }
            Ok(CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None })
        })
        .collect()
}

fn collapse_duplicate_x(sorted_pts: &[(f64, f64)], out: &mut Vec<(f64, f64)>) {
    out.clear();
    let mut idx = 0_usize;
    while idx < sorted_pts.len() {
        let x = sorted_pts[idx].0;
        let mut sum_y = NeumaierSum::new();
        let mut count = 0_usize;
        while idx < sorted_pts.len() && sorted_pts[idx].0 == x {
            sum_y.add(sorted_pts[idx].1);
            count += 1;
            idx += 1;
        }
        let mean_y = sum_y.total() / (count as f64);
        if mean_y.is_finite() {
            out.push((x, mean_y));
        }
    }
}

fn collect_sorted_mean_xy_into(points: &[DataPoint], scratch: &mut Vec<(f64, f64)>, collapsed_out: &mut Vec<(f64, f64)>) {
    scratch.clear();
    collapsed_out.clear();
    if points.iter().any(|pt| pt.x.len() != 1) {
        return;
    }
    for pt in points {
        if let Some(&x) = pt.x.first()
            && x.is_finite()
            && pt.y.is_finite()
        {
            scratch.push((canonical_zero(x), pt.y));
        }
    }
    if scratch.len() < 2 {
        return;
    }
    scratch.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    collapse_duplicate_x(scratch, collapsed_out);
}

/// Composite trapezoidal rule on (possibly unequally spaced) sorted points, with linear
/// interpolation at the interval ends (Press et al. 2007, Sec. 4.1). Exact for piecewise-linear
/// data; no error estimate is attempted for curved data.
fn trapezoidal_auc_over_interval(sorted_unique_pts: &[(f64, f64)], x_min: f64, x_max: f64) -> Option<f64> {
    if !x_min.is_finite() || !x_max.is_finite() || x_max < x_min || sorted_unique_pts.is_empty() {
        return None;
    }
    if x_max == x_min {
        return Some(0.0);
    }
    if sorted_unique_pts.len() < 2 {
        return None;
    }
    let mut sum = NeumaierSum::new();
    for w in sorted_unique_pts.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if x1 <= x0 || x1 <= x_min || x0 >= x_max {
            continue;
        }
        let a = x0.max(x_min);
        let b = x1.min(x_max);
        if b <= a {
            continue;
        }
        let span = x1 - x0;
        let ya = if a == x0 { y0 } else { y0 + (y1 - y0) * ((a - x0) / span) };
        let yb = if b == x1 { y1 } else { y0 + (y1 - y0) * ((b - x0) / span) };
        let area = 0.5 * (ya + yb) * (b - a);
        if !area.is_finite() {
            return None;
        }
        sum.add(area);
    }
    let total = sum.total();
    total.is_finite().then_some(total)
}

fn overlay_covers_interval(sorted_pts: &[(f64, f64)], x_min: f64, x_max: f64) -> bool {
    if !x_min.is_finite() || !x_max.is_finite() || x_max < x_min {
        return false;
    }
    let (Some(first), Some(last)) = (sorted_pts.first(), sorted_pts.last()) else {
        return false;
    };
    let scale = (x_max - x_min).abs().max(x_min.abs()).max(x_max.abs()).max(1.0);
    let tol = 1e-12 * scale;
    first.0 <= x_min + tol && last.0 >= x_max - tol
}

pub(crate) fn compute_curve_overlay_auc(
    overlay: &FittedCurveOverlay,
    x_min: f64,
    x_max: f64,
    scratch: &mut Vec<(f64, f64)>,
    collapsed: &mut Vec<(f64, f64)>,
) -> Option<f64> {
    scratch.clear();
    collapsed.clear();
    for pt in &overlay.points {
        if pt.x.is_finite() && pt.y.is_finite() {
            scratch.push((canonical_zero(pt.x), pt.y));
        }
    }
    if scratch.len() < 2 {
        return None;
    }
    if scratch.windows(2).all(|w| w[0].0 < w[1].0) {
        if !overlay_covers_interval(scratch, x_min, x_max) {
            return None;
        }
        return trapezoidal_auc_over_interval(scratch, x_min, x_max);
    }
    scratch.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    collapse_duplicate_x(scratch, collapsed);
    if !overlay_covers_interval(collapsed, x_min, x_max) {
        return None;
    }
    trapezoidal_auc_over_interval(collapsed, x_min, x_max)
}

fn compute_model_grid_auc(
    model: &dyn ParametricModel,
    params: &[f64],
    data_set_index: usize,
    x_min: f64,
    x_max: f64,
    scratch: &mut Vec<(f64, f64)>,
) -> Option<f64> {
    if !x_min.is_finite() || !x_max.is_finite() || x_max < x_min {
        return None;
    }
    if x_max == x_min {
        let y = model.eval(&[x_min], params, data_set_index)?;
        return y.is_finite().then_some(0.0);
    }
    scratch.clear();
    let use_log = x_min > 0.0 && (x_max / x_min) >= 100.0;
    let log_min = if use_log { x_min.ln() } else { 0.0 };
    let log_span = if use_log { x_max.ln() - log_min } else { 0.0 };
    let n_grid = 256_usize;
    for i in 0..n_grid {
        let frac = (i as f64) / ((n_grid - 1) as f64);
        let x = if use_log {
            (log_min + frac * log_span).exp().clamp(x_min, x_max)
        } else if i + 1 == n_grid {
            x_max
        } else {
            x_min + frac * (x_max - x_min)
        };
        if let Some(y) = model.eval(&[x], params, data_set_index)
            && y.is_finite()
        {
            scratch.push((canonical_zero(x), y));
        }
    }
    if scratch.len() < 2 || !overlay_covers_interval(scratch, x_min, x_max) {
        return None;
    }
    trapezoidal_auc_over_interval(scratch, x_min, x_max)
}

fn append_auc_scalars_and_tab(
    result: &mut ResultSheet,
    datasets: &[DatasetSeries],
    nl_auc: Option<(&dyn ParametricModel, &NonlinearFitResult, bool)>,
) {
    let mut col_headers = Vec::new();
    let mut row_emp = Vec::new();
    let mut row_fit = Vec::new();
    let mut row_xmin = Vec::new();
    let mut row_xmax = Vec::new();

    let mut raw_scratch = Vec::new();
    let mut collapsed_scratch = Vec::new();
    let mut curve_raw_scratch = Vec::new();
    let mut curve_collapsed_scratch = Vec::new();

    for (ds_pos, ds) in datasets.iter().enumerate() {
        if ds.points.iter().any(|pt| pt.x.len() != 1) {
            continue;
        }
        collect_sorted_mean_xy_into(&ds.points, &mut raw_scratch, &mut collapsed_scratch);
        if raw_scratch.len() < 2 || collapsed_scratch.is_empty() {
            continue;
        }
        let x_min = collapsed_scratch[0].0;
        let x_max = collapsed_scratch[collapsed_scratch.len() - 1].0;
        let Some(emp_auc) = trapezoidal_auc_over_interval(&collapsed_scratch, x_min, x_max) else {
            continue;
        };

        result.scalars.push(ResultScalarEntry {
            key: "AUC (trapezoidal)".to_owned(),
            data_set_index: Some(ds.data_set_index),
            value: emp_auc,
            units: None,
        });

        let overlay = result
            .fitted_curves
            .get(ds_pos)
            .filter(|fc| fc.data_set_index == ds.data_set_index)
            .or_else(|| result.fitted_curves.iter().find(|fc| fc.data_set_index == ds.data_set_index));

        let fitted_auc = overlay.and_then(|ov| {
            if let Some((model, fit, custom_bounds)) = nl_auc
                && custom_bounds
                && let Some(ds_res) = fit.datasets.iter().find(|fd| fd.data_set_index == ds.data_set_index)
            {
                compute_model_grid_auc(model, &ds_res.params, ds.data_set_index, x_min, x_max, &mut curve_raw_scratch)
            } else {
                compute_curve_overlay_auc(ov, x_min, x_max, &mut curve_raw_scratch, &mut curve_collapsed_scratch)
            }
        });
        if let Some(fit_auc) = fitted_auc {
            result.scalars.push(ResultScalarEntry {
                key: "AUC (fitted curve)".to_owned(),
                data_set_index: Some(ds.data_set_index),
                value: fit_auc,
                units: None,
            });
        }

        col_headers.push(ds.label.clone());
        row_emp.push(num_cell(emp_auc));
        row_fit.push(fitted_auc.map_or(ResultCell::Empty, num_cell));
        row_xmin.push(num_cell(x_min));
        row_xmax.push(num_cell(x_max));
    }

    if col_headers.is_empty() {
        return;
    }

    result.tabs.push(
        TabBuilder::new("fit_auc", "Area Under the Curve (AUC)", ResultTabKind::Diagnostics)
            .headers(col_headers)
            .row("Trapezoidal AUC", row_emp)
            .row("Fitted curve AUC", row_fit)
            .row("X min", row_xmin)
            .row("X max", row_xmax)
            .footnote(
                "Trapezoidal AUC integrates replicate-averaged (X, Mean Y) across [X min, X max]; fitted curve AUC integrates the fitted curve across [X min, X max].",
            )
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, CellCoord, CiBound, DataSheet, LinearRegressionSpec, NonlinearFitSpec, ParameterRule, ParameterScope,
        ResultCell, ResultStatus, ResultTabKind, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
    };
    use curvebook_math::ConfidenceLevel;

    use super::execute_analysis;
    use crate::library::get_builtin_model;
    use crate::types::HookContext;

    fn hill_4pl(log_x: f64, bottom: f64, top: f64, log_ec50: f64, slope: f64) -> f64 {
        bottom + (top - bottom) / (1.0 + 10_f64.powf((log_ec50 - log_x) * slope))
    }

    fn inhib_4pl(log_x: f64, bottom: f64, top: f64, log_ic50: f64, slope: f64) -> f64 {
        bottom + (top - bottom) / (1.0 + 10_f64.powf((log_ic50 - log_x) * (-slope)))
    }

    fn build_4pl_sheet_with_unknowns(extra_out_of_range_y: Option<f64>) -> DataSheet {
        let std_xs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
        let extra_rows = if extra_out_of_range_y.is_some() { 4 } else { 3 };
        let xy = XyTable::new(SubcolumnMode::Single, 1, std_xs.len() + extra_rows).unwrap();
        let mut sheet = DataSheet::new("dose-4pl", "4PL Standard Curve", TableArchetype::Xy(xy)).unwrap();

        for (r, &lx) in std_xs.iter().enumerate() {
            let y = hill_4pl(lx, 5.0, 100.0, -6.0, 1.15);
            sheet.set_x_or_time_cell(r, 0, Cell::number(lx)).unwrap();
            sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(y)).unwrap();
        }

        // Rows 8..10: blank X with known Y (inverse X from Y), then row 10: known X with blank Y (forward Y from X).
        let base = std_xs.len();
        sheet.set_cell(CellCoord::new(base, 0, 0), Cell::number(hill_4pl(-6.2, 5.0, 100.0, -6.0, 1.15))).unwrap();
        sheet.set_cell(CellCoord::new(base + 1, 0, 0), Cell::number(hill_4pl(-5.4, 5.0, 100.0, -6.0, 1.15))).unwrap();
        sheet.set_x_or_time_cell(base + 2, 0, Cell::number(-6.0)).unwrap();
        if let Some(bad_y) = extra_out_of_range_y {
            sheet.set_cell(CellCoord::new(base + 3, 0, 0), Cell::number(bad_y)).unwrap();
        }

        sheet
    }

    fn make_4pl_spec(id: &str, interpolate_unknowns: bool) -> AnalysisSpec {
        let nl = NonlinearFitSpec { weighting: WeightingScheme::None, interpolate_unknowns, ..NonlinearFitSpec::new("log_agonist_vs_response_4pl") };
        let mut spec = AnalysisSpec::new(id, "dose-4pl", AnalysisKind::NonlinearRegression(nl)).unwrap();
        spec.confidence_level = ConfidenceLevel::DEFAULT_95;
        spec
    }

    fn assert_num_cell_approx(cell: &ResultCell, expected: f64, tol: f64, ctx: &str) {
        let ResultCell::Number { value, .. } = cell else {
            panic!("{ctx}: expected numeric cell, got {cell:?}");
        };
        assert!((*value - expected).abs() <= tol, "{ctx}: got {value}, expected {expected} (tol {tol})");
    }

    #[test]
    fn nonlinear_identifiability_and_r_squared_threshold_reach_result_sheet() {
        let xy = XyTable::new(SubcolumnMode::Single, 1, 4).unwrap();
        let mut sheet = DataSheet::new("xy", "Trend", TableArchetype::Xy(xy)).unwrap();
        for (row, (x, y)) in [(10.0, 24.0), (12.0, 26.0), (14.0, 30.0), (16.0, 36.0)].into_iter().enumerate() {
            sheet.set_x_or_time_cell(row, 0, Cell::number(x)).unwrap();
            sheet.set_cell(CellCoord::new(row, 0, 0), Cell::number(y)).unwrap();
        }

        let mut nl = NonlinearFitSpec::new("straight_line");
        nl.profile_likelihood_ci = false;
        nl.identify_unstable_parameters = true;
        nl.flag_r_squared_below = Some(0.96);
        let run = |nl| {
            let spec = AnalysisSpec::new("fit", "xy", AnalysisKind::NonlinearRegression(nl)).unwrap();
            execute_analysis(&spec, &sheet, &HookContext::default())
        };
        let result = run(nl.clone()).unwrap();
        // The residuals [1, -1, -1, 1] are orthogonal to both columns of the design.
        // RSS = 4 and TSS = 84, giving R² = 20/21 (Draper & Smith, Applied Regression Analysis).
        assert!((result.lookup_scalar("R2", None).unwrap() - 20.0 / 21.0).abs() < 1e-10);
        assert!(result.checklist.iter().any(|item| item.assumption == "R squared below threshold" && item.remediation.is_none()));
        assert!(result.checklist.iter().all(|item| !item.assumption.contains("Ambiguous")));

        nl.flag_r_squared_below = Some(0.95);
        let in_range = run(nl.clone()).unwrap();
        assert!(in_range.checklist.iter().all(|item| item.assumption != "R squared below threshold"));

        nl.x_range_filter = Some((None, Some(12.0)));
        let saturated = run(nl.clone()).unwrap();
        assert!(saturated.checklist.iter().any(|item| item.explanation == "Residual degrees of freedom are zero."));
        let params = saturated.tabs.iter().find(|tab| tab.kind == ResultTabKind::ParametersAndConfidenceIntervals).unwrap();
        assert_eq!(params.row_labels.iter().filter(|label| label.ends_with("(Unstable)")).count(), 2);
        assert!(params.rows.iter().all(|row| !matches!(row.first(), Some(ResultCell::Number { prefix: Some(prefix), .. }) if prefix == "~")));
        assert!(saturated.fitted_curves.iter().all(|overlay| overlay.ambiguous));

        nl.flag_r_squared_below = Some(f64::NAN);
        assert!(AnalysisSpec::new("fit", "xy", AnalysisKind::NonlinearRegression(nl.clone())).is_err());
        let mut mutated_spec = AnalysisSpec::new("fit", "xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap();
        mutated_spec.kind = AnalysisKind::NonlinearRegression(nl);
        assert!(execute_analysis(&mutated_spec, &sheet, &HookContext::default()).is_err());
    }

    #[test]
    fn nonlinear_midpoint_extrapolation_uses_fitted_observations_and_model_scale() {
        // Inverting the normalized one-parameter Hill equation gives the known LogEC50
        // or LogIC50 exactly. The numerical fit must recover it within 1e-7 log units.
        // Sebaugh (2011), doi:10.1002/pst.426, describes concentration coverage at midpoints.
        let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.0, -5.5];
        let midpoint = -6.5;
        for (equation_id, param_name, sign) in
            [("log_agonist_vs_response_normalized", "LogEC50", 1.0), ("log_inhibitor_vs_response_normalized", "LogIC50", -1.0)]
        {
            let xy = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
            let mut sheet = DataSheet::new("dose", "Dose Response", TableArchetype::Xy(xy)).unwrap();
            for (row, x) in xs.into_iter().enumerate() {
                let y = 100.0 / (1.0 + 10_f64.powf(sign * (midpoint - x)));
                sheet.set_x_or_time_cell(row, 0, Cell::number(x)).unwrap();
                sheet.set_cell(CellCoord::new(row, 0, 0), Cell::number(y)).unwrap();
            }
            let mut nl = NonlinearFitSpec::new(equation_id);
            nl.profile_likelihood_ci = false;
            nl.x_range_filter = Some((None, Some(-6.75)));
            nl.plot_x_max = Some(-5.0);
            let make_spec = |nl| AnalysisSpec::new("fit", "dose", AnalysisKind::NonlinearRegression(nl)).unwrap();
            let restricted = execute_analysis(&make_spec(nl.clone()), &sheet, &HookContext::default()).unwrap();
            let estimate = restricted.lookup_scalar(param_name, Some(0)).unwrap();
            assert!((estimate - midpoint).abs() < 1e-7, "{param_name}: {estimate}");
            let warning = restricted.checklist.iter().find(|c| c.assumption.starts_with(&format!("{param_name} outside observed X range"))).unwrap();
            assert_eq!(warning.status, curvebook_doc::ChecklistStatus::Warn);
            assert!(warning.diagnostic_metric.as_deref().is_some_and(|metric| metric.contains("observed X [-9, -7]")));
            assert!(warning.remediation.is_none());

            nl.x_range_filter = None;
            let bracketed = execute_analysis(&make_spec(nl), &sheet, &HookContext::default()).unwrap();
            assert!(bracketed.checklist.iter().all(|c| !c.assumption.contains("outside observed X range")));
        }
    }

    #[test]
    fn replicated_unknowns_use_mean_response_and_keep_actual_subcolumns() {
        let xy = XyTable::new(SubcolumnMode::RawReplicates(4), 1, 6).unwrap();
        let mut sheet = DataSheet::new("xy", "Standard Curve", TableArchetype::Xy(xy)).unwrap();
        for row in 0..5 {
            let x = row as f64;
            sheet.set_x_or_time_cell(row, 0, Cell::number(x)).unwrap();
            sheet.set_cell(CellCoord::new(row, 0, 0), Cell::number(3.0 + 2.0 * x - 0.25)).unwrap();
            sheet.set_cell(CellCoord::new(row, 0, 2), Cell::number(3.0 + 2.0 * x + 0.25)).unwrap();
        }
        sheet.set_cell(CellCoord::new(5, 0, 0), Cell::number(6.0)).unwrap();
        sheet.set_cell(CellCoord::new(5, 0, 2), Cell::number(8.0)).unwrap();
        let mut excluded = Cell::number(1000.0);
        excluded.excluded = true;
        sheet.set_cell(CellCoord::new(5, 0, 3), excluded).unwrap();

        let mut nl = NonlinearFitSpec::new("straight_line");
        nl.interpolate_unknowns = true;
        nl.profile_likelihood_ci = false;
        let spec = AnalysisSpec::new("fit", "xy", AnalysisKind::NonlinearRegression(nl)).unwrap();
        let result = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
        let mean = result.tabs.iter().find(|t| t.title == "Interpolated Mean X").unwrap();
        let reps = result.tabs.iter().find(|t| t.title == "Interpolated Replicates").unwrap();
        assert_eq!(mean.rows.len(), 1);
        assert_eq!(reps.rows.len(), 2);
        assert!(reps.row_labels[0].ends_with("replicate 1)"));
        assert!(reps.row_labels[1].ends_with("replicate 3)"));
        assert_num_cell_approx(&mean.rows[0][0], 7.0, 1e-12, "mean response");
        assert_num_cell_approx(&mean.rows[0][1], 2.0, 1e-9, "inverse of mean response");
        assert_num_cell_approx(&reps.rows[0][1], 1.5, 1e-9, "inverse replicate 1");
        assert_num_cell_approx(&reps.rows[1][1], 2.5, 1e-9, "inverse replicate 3");

        // NIST/SEMATECH e-Handbook 4.5.2.1 and Seber & Lee (2003), §6.1.3.
        // For mean Y = ybar, invert the prediction band analytically:
        // (b² - t²s²/Sxx)(X - xbar)² = t²s²(1/m + 1/n).
        let t_squared_s_squared = 2.306_004_135_204_166_f64.powi(2) * (0.625 / 8.0);
        let half = (t_squared_s_squared * (1.0 / 2.0 + 1.0 / 10.0) / (4.0 - t_squared_s_squared / 20.0)).sqrt();
        let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lower), upper: CiBound::Finite(upper) } = mean.rows[0][3] else {
            panic!("expected finite inverse prediction interval for mean response");
        };
        assert!((lower - (2.0 - half)).abs() < 1e-7 && (upper - (2.0 + half)).abs() < 1e-7);
    }

    #[test]
    fn nonlinear_4pl_unknown_interpolation_emits_interpolated_unknowns_tab() {
        let sheet = build_4pl_sheet_with_unknowns(None);
        let spec = make_4pl_spec("fit-4pl-interp", true);

        let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
        assert_eq!(res.status, ResultStatus::UpToDate);
        assert!(res.analyzable_data.is_none());
        assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals));
        assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::MainSummary));

        let interp_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::InterpolatedUnknowns).expect("InterpolatedUnknowns tab");
        assert_eq!(interp_tab.id, "nonlinear_interpolated");
        assert_eq!(interp_tab.title, "Interpolated unknowns");
        assert_eq!(
            interp_tab.column_headers,
            vec!["Input", "Interpolated", "Mean-Response CI", "Calibration / Prediction Interval", "Direction", "Dataset"]
        );
        assert_eq!(interp_tab.rows.len(), 3);
        assert_eq!(interp_tab.row_labels.len(), 3);
        assert!(interp_tab.footnotes.is_empty());

        for (idx, expected_x) in [-6.2_f64, -5.4_f64].iter().enumerate() {
            assert!(interp_tab.row_labels[idx].contains("X from Y="));
            let ResultCell::Number { value, .. } = interp_tab.rows[idx][1] else {
                panic!("expected numeric interpolated X in row {idx}");
            };
            assert!((value - expected_x).abs() < 1e-4, "row {idx}: got {value}, expected {expected_x}");
            assert_eq!(interp_tab.rows[idx][4], ResultCell::Text("X from Y".to_owned()));
            assert_eq!(interp_tab.rows[idx][5], ResultCell::Text("A".to_owned()));
        }

        assert!(interp_tab.row_labels[2].contains("Y from X="));
        let ResultCell::Number { value: est_y, .. } = interp_tab.rows[2][1] else {
            panic!("expected numeric interpolated Y in row 2");
        };
        assert!((est_y - 52.5).abs() < 1e-4, "expected Y=52.5 at LogEC50, got {est_y}");
        let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } = interp_tab.rows[2][2] else {
            panic!("expected finite Mean-Response CI for interpolated Y");
        };
        let ResultCell::ConfidenceInterval { lower: CiBound::Finite(plo), upper: CiBound::Finite(phi) } = interp_tab.rows[2][3] else {
            panic!("expected finite Calibration / Prediction Interval for interpolated Y");
        };
        assert!(plo <= lo && lo <= est_y && est_y <= hi && hi <= phi);
        assert_eq!(interp_tab.rows[2][4], ResultCell::Text("Y from X".to_owned()));
        assert_eq!(interp_tab.rows[2][5], ResultCell::Text("A".to_owned()));
    }

    #[test]
    fn nonlinear_without_interpolate_unknowns_omits_interpolated_tab() {
        let sheet = build_4pl_sheet_with_unknowns(None);
        let spec = make_4pl_spec("fit-4pl-no-interp", false);

        let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
        assert_eq!(res.tabs.len(), 4);
        assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals));
        assert!(res.tabs.iter().any(|t| t.id == "fit_auc" && t.kind == ResultTabKind::Diagnostics));
        assert!(res.tabs.iter().all(|t| t.kind != ResultTabKind::InterpolatedUnknowns));
    }

    #[test]
    fn nonlinear_out_of_range_unknown_sanitizes_estimate_and_records_footnote() {
        let sheet = build_4pl_sheet_with_unknowns(Some(500.0));
        let spec = make_4pl_spec("fit-4pl-oor", true);

        let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
        let interp_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::InterpolatedUnknowns).expect("InterpolatedUnknowns tab");
        assert_eq!(interp_tab.rows.len(), 4);
        assert_eq!(interp_tab.rows[2][1], ResultCell::Empty);
        assert_eq!(interp_tab.rows[2][2], ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable });
        assert_eq!(interp_tab.rows[2][3], ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable });
        assert_eq!(interp_tab.footnotes.len(), 1);
        assert!(interp_tab.footnotes[0].contains("outside fitted curve range"));
    }

    #[test]
    fn linear_regression_emits_auc_scalars_and_fit_auc_tab() {
        // Multi-dataset XY table with unsorted X and duplicate-X replicates in DS0, plus scatter in DS1.
        let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).unwrap();
        xy.data_sets[0].title = "Exact Line Replicates".to_owned();
        xy.data_sets[1].title = "Scattered Line".to_owned();
        let mut sheet = DataSheet::new("lin-auc-ds", "Linear AUC", TableArchetype::Xy(xy)).unwrap();

        // Unsorted non-uniform X rows with duplicate X = 1.0 across datasets:
        // DS0: (3, 7), (0, 1), (1, 2), (1, 4) [mean at x=1 is 3.0], exact line y = 2x + 1 -> emp & fit AUC = 12.0.
        // DS1: (3, 4), (0, 1), (1, 9), (1, 11) [mean at x=1 is 10.0] -> emp AUC = 19.5 != fitted line AUC (~18.87).
        let x_rows = [3.0, 0.0, 1.0, 1.0];
        let y0_rows = [7.0, 1.0, 2.0, 4.0];
        let y1_rows = [4.0, 1.0, 9.0, 11.0];
        for r in 0..4 {
            sheet.set_x_or_time_cell(r, 0, Cell::number(x_rows[r])).unwrap();
            sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(y0_rows[r])).unwrap();
            sheet.set_cell(CellCoord::new(r, 1, 0), Cell::number(y1_rows[r])).unwrap();
        }

        // Test with confidence_bands = false to ensure fitted curve overlay & AUC still emit for OLS.
        let lin_spec = LinearRegressionSpec { confidence_bands: false, prediction_bands: false, ..LinearRegressionSpec::default() };
        let spec = AnalysisSpec::new("lin-auc-1", "lin-auc-ds", AnalysisKind::LinearRegression(lin_spec)).unwrap();
        let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();

        // linear_summary must preserve exact 12 R18.5 row labels without inserted AUC rows.
        let summary = res.tabs.iter().find(|t| t.id == "linear_summary").expect("linear_summary tab");
        assert_eq!(summary.rows.len(), 12);
        assert_eq!(summary.row_labels[0], "Slope");
        assert_eq!(summary.row_labels[9], "Runs test P value");
        assert_eq!(summary.row_labels[10], "X-intercept");
        assert_eq!(summary.row_labels[11], "95% CI X-intercept");
        assert!(!summary.row_labels.iter().any(|lbl| lbl.contains("AUC")));

        // Exact scalar keys per dataset index.
        let ds0_emp = res.scalars.iter().find(|s| s.key == "AUC (trapezoidal)" && s.data_set_index == Some(0)).expect("DS0 AUC (trapezoidal)");
        let ds0_fit = res.scalars.iter().find(|s| s.key == "AUC (fitted curve)" && s.data_set_index == Some(0)).expect("DS0 AUC (fitted curve)");
        assert!((ds0_emp.value - 12.0).abs() < 1e-12);
        assert!((ds0_fit.value - 12.0).abs() < 1e-10);

        let slope1 = res.lookup_scalar("Slope", Some(1)).unwrap();
        let int1 = res.lookup_scalar("YIntercept", Some(1)).unwrap();
        let expected_fit1 = 0.5 * ((int1 + slope1 * 0.0) + (int1 + slope1 * 3.0)) * 3.0;
        let ds1_emp = res.scalars.iter().find(|s| s.key == "AUC (trapezoidal)" && s.data_set_index == Some(1)).expect("DS1 AUC (trapezoidal)");
        let ds1_fit = res.scalars.iter().find(|s| s.key == "AUC (fitted curve)" && s.data_set_index == Some(1)).expect("DS1 AUC (fitted curve)");
        assert!((ds1_emp.value - 19.5).abs() < 1e-12);
        assert!((ds1_fit.value - expected_fit1).abs() < 1e-10);
        assert!((ds1_emp.value - ds1_fit.value).abs() > 0.5);

        // fit_auc tab assertions.
        let auc_tab = res.tabs.iter().find(|t| t.id == "fit_auc").expect("fit_auc tab");
        assert_eq!(auc_tab.kind, ResultTabKind::Diagnostics);
        assert_eq!(auc_tab.column_headers, vec!["Exact Line Replicates", "Scattered Line"]);
        assert_eq!(auc_tab.row_labels, vec!["Trapezoidal AUC", "Fitted curve AUC", "X min", "X max"]);
        assert_eq!(auc_tab.rows.len(), 4);
        assert_num_cell_approx(&auc_tab.rows[0][0], 12.0, 1e-12, "DS0 emp tab");
        assert_num_cell_approx(&auc_tab.rows[1][0], 12.0, 1e-10, "DS0 fit tab");
        assert_num_cell_approx(&auc_tab.rows[2][0], 0.0, 1e-12, "DS0 xmin");
        assert_num_cell_approx(&auc_tab.rows[3][0], 3.0, 1e-12, "DS0 xmax");
        assert_num_cell_approx(&auc_tab.rows[0][1], 19.5, 1e-12, "DS1 emp tab");
        assert_num_cell_approx(&auc_tab.rows[1][1], expected_fit1, 1e-10, "DS1 fit tab");
        assert_num_cell_approx(&auc_tab.rows[2][1], 0.0, 1e-12, "DS1 xmin");
        assert_num_cell_approx(&auc_tab.rows[3][1], 3.0, 1e-12, "DS1 xmax");

        // Deming Model II emits a default FittedCurveOverlay (AUD-13-17), so both trapezoidal and fitted-curve AUC emit.
        let dem_spec = AnalysisSpec::new(
            "dem-auc",
            "lin-auc-ds",
            AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() }),
        )
        .unwrap();
        let dem_res = execute_analysis(&dem_spec, &sheet, &HookContext::default()).unwrap();
        assert_eq!(dem_res.fitted_curves.len(), 2, "default Deming emits FittedCurveOverlay for every dataset");
        assert!(dem_res.scalars.iter().any(|s| s.key == "AUC (trapezoidal)" && s.data_set_index == Some(0)));
        assert!(dem_res.scalars.iter().any(|s| s.key == "AUC (fitted curve)" && s.data_set_index == Some(0)));
        let dem_slope0 = dem_res.lookup_scalar("Slope", Some(0)).unwrap();
        let dem_int0 = dem_res.lookup_scalar("YIntercept", Some(0)).unwrap();
        let expected_dem_fit0 = 0.5 * ((dem_int0 + dem_slope0 * 0.0) + (dem_int0 + dem_slope0 * 3.0)) * 3.0;
        let dem_auc_tab = dem_res.tabs.iter().find(|t| t.id == "fit_auc").expect("Deming fit_auc tab");
        assert_num_cell_approx(&dem_auc_tab.rows[0][0], 12.0, 1e-12, "Deming DS0 emp");
        assert_num_cell_approx(&dem_auc_tab.rows[1][0], expected_dem_fit0, 1e-10, "Deming DS0 fit");
    }

    #[test]
    fn nonlinear_4pl_ic50_unconstrained_vs_constrained_auc_and_fit_auc_tab() {
        // Partial inhibition dose-response with replicate scatter at identical log[Inhibitor] values.
        // True partial bottom = 0.25, top = 0.95 -> constrained Bottom=0, Top=1 shifts IC50 and fitted-curve AUC
        // while keeping empirical trapezoidal AUC identical.
        let log_concs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, log_concs.len()).unwrap();
        xy.data_sets[0].title = "Inhibitor".to_owned();
        let mut sheet = DataSheet::new("ic50-auc-ds", "IC50 4PL", TableArchetype::Xy(xy)).unwrap();

        let mut expected_emp_auc = 0.0_f64;
        let mut prev: Option<(f64, f64)> = None;
        for (r, &lx) in log_concs.iter().enumerate() {
            let y_true = inhib_4pl(lx, 0.25, 0.95, -6.0, 1.1);
            let y0 = y_true - 0.01;
            let y1 = y_true + 0.01;
            sheet.set_x_or_time_cell(r, 0, Cell::number(lx)).unwrap();
            sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(y0)).unwrap();
            sheet.set_cell(CellCoord::new(r, 0, 1), Cell::number(y1)).unwrap();
            if let Some((px, py)) = prev {
                expected_emp_auc += 0.5 * (py + y_true) * (lx - px);
            }
            prev = Some((lx, y_true));
        }

        // 1. Unconstrained 4PL IC50 fit.
        let nl_free = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("log_inhibitor_vs_response_4pl") };
        let spec_free = AnalysisSpec::new("fit-ic50-free", "ic50-auc-ds", AnalysisKind::NonlinearRegression(nl_free)).unwrap();
        let res_free = execute_analysis(&spec_free, &sheet, &HookContext::default()).unwrap();
        assert_eq!(res_free.status, ResultStatus::UpToDate);

        // Verify nonlinear_parameters row indices stay intact.
        let param_tab_free = res_free.tabs.iter().find(|t| t.id == "nonlinear_parameters").expect("nonlinear_parameters");
        assert_eq!(param_tab_free.row_labels, vec!["Bottom", "Top", "LogIC50", "HillSlope"]);

        let ic50_free = res_free.lookup_scalar("IC50", Some(0)).expect("unconstrained IC50");
        assert!((ic50_free - 1e-6).abs() < 1e-8, "expected free IC50 ~ 1e-6, got {ic50_free}");

        let emp_free =
            res_free.scalars.iter().find(|s| s.key == "AUC (trapezoidal)" && s.data_set_index == Some(0)).expect("free AUC (trapezoidal)").value;
        let fit_auc_free =
            res_free.scalars.iter().find(|s| s.key == "AUC (fitted curve)" && s.data_set_index == Some(0)).expect("free AUC (fitted curve)").value;
        assert!((emp_free - expected_emp_auc).abs() < 1e-12);

        // Verify fitted-curve AUC matches fine Simpson/trapezoidal numerical integral of the fitted 4PL model.
        let model = get_builtin_model("log_inhibitor_vs_response_4pl").unwrap();
        let free_params = [
            res_free.lookup_scalar("Bottom", Some(0)).unwrap(),
            res_free.lookup_scalar("Top", Some(0)).unwrap(),
            res_free.lookup_scalar("LogIC50", Some(0)).unwrap(),
            res_free.lookup_scalar("HillSlope", Some(0)).unwrap(),
        ];
        let fine_n = 2_000_usize;
        let mut fine_auc_free = 0.0_f64;
        for i in 0..fine_n {
            let x0 = -9.0 + 5.0 * (i as f64) / (fine_n as f64);
            let x1 = -9.0 + 5.0 * ((i + 1) as f64) / (fine_n as f64);
            let y0 = model.eval(&[x0], &free_params, 0).unwrap();
            let y1 = model.eval(&[x1], &free_params, 0).unwrap();
            fine_auc_free += 0.5 * (y0 + y1) * (x1 - x0);
        }
        assert!((fit_auc_free - fine_auc_free).abs() < 1e-4, "fit_auc_free={fit_auc_free}, fine={fine_auc_free}");

        // 2. Constrained Bottom = 0.0, Top = 1.0 4PL IC50 fit.
        let mut nl_const = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("log_inhibitor_vs_response_4pl") };
        nl_const.parameter_rules = vec![
            ParameterRule {
                name: "Bottom".to_owned(),
                scope: ParameterScope::Constant(0.0),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "Top".to_owned(),
                scope: ParameterScope::Constant(1.0),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ];
        let spec_const = AnalysisSpec::new("fit-ic50-const", "ic50-auc-ds", AnalysisKind::NonlinearRegression(nl_const)).unwrap();
        let res_const = execute_analysis(&spec_const, &sheet, &HookContext::default()).unwrap();
        assert_eq!(res_const.status, ResultStatus::UpToDate);

        let param_tab_const = res_const.tabs.iter().find(|t| t.id == "nonlinear_parameters").expect("constrained nonlinear_parameters");
        assert_eq!(param_tab_const.row_labels, vec!["LogIC50", "HillSlope", "Bottom (Constant)", "Top (Constant)"]);

        let ic50_const = res_const.lookup_scalar("IC50", Some(0)).expect("constrained IC50");
        assert!(ic50_const > ic50_free * 1.2, "forcing Bottom=0 on partial inhibition shifts IC50 rightward: free={ic50_free}, const={ic50_const}");

        let emp_const = res_const
            .scalars
            .iter()
            .find(|s| s.key == "AUC (trapezoidal)" && s.data_set_index == Some(0))
            .expect("constrained AUC (trapezoidal)")
            .value;
        let fit_auc_const = res_const
            .scalars
            .iter()
            .find(|s| s.key == "AUC (fitted curve)" && s.data_set_index == Some(0))
            .expect("constrained AUC (fitted curve)")
            .value;
        assert!((emp_const - emp_free).abs() < 1e-12, "empirical trapezoidal AUC must be identical");
        let const_params =
            [0.0_f64, 1.0_f64, res_const.lookup_scalar("LogIC50", Some(0)).unwrap(), res_const.lookup_scalar("HillSlope", Some(0)).unwrap()];
        let mut fine_auc_const = 0.0_f64;
        for i in 0..fine_n {
            let x0 = -9.0 + 5.0 * (i as f64) / (fine_n as f64);
            let x1 = -9.0 + 5.0 * ((i + 1) as f64) / (fine_n as f64);
            let y0 = model.eval(&[x0], &const_params, 0).unwrap();
            let y1 = model.eval(&[x1], &const_params, 0).unwrap();
            fine_auc_const += 0.5 * (y0 + y1) * (x1 - x0);
        }
        assert!((fit_auc_const - fine_auc_const).abs() < 1e-4, "fit_auc_const={fit_auc_const}, fine={fine_auc_const}");
        assert!(
            (fit_auc_const - fit_auc_free).abs() > 0.02,
            "constrained Bottom=0/Top=1 curve changes fitted-curve AUC: free={fit_auc_free}, const={fit_auc_const}"
        );

        let auc_tab = res_const.tabs.iter().find(|t| t.id == "fit_auc").expect("fit_auc tab");
        assert_eq!(auc_tab.kind, ResultTabKind::Diagnostics);
        assert_eq!(auc_tab.column_headers, vec!["Inhibitor"]);
        assert_eq!(auc_tab.row_labels, vec!["Trapezoidal AUC", "Fitted curve AUC", "X min", "X max"]);
        assert_num_cell_approx(&auc_tab.rows[0][0], emp_const, 1e-12, "constrained tab emp AUC");
        assert_num_cell_approx(&auc_tab.rows[1][0], fit_auc_const, 1e-12, "constrained tab fit AUC");
        assert_num_cell_approx(&auc_tab.rows[2][0], -9.0, 1e-12, "constrained tab X min");
        assert_num_cell_approx(&auc_tab.rows[3][0], -4.0, 1e-12, "constrained tab X max");
    }

    #[test]
    fn auc_handles_plot_x_bounds_and_skips_insufficient_datasets() {
        // Two datasets: DS0 has 5 points on [0, 4], DS1 has only 1 finite point (plus 1 NaN/excluded).
        let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5).unwrap();
        xy.data_sets[0].title = "Valid".to_owned();
        xy.data_sets[1].title = "TooFew".to_owned();
        let mut sheet = DataSheet::new("edge-auc-ds", "Edge AUC", TableArchetype::Xy(xy)).unwrap();

        for (r, &x) in [0.0, 1.0, 2.0, 3.0, 4.0].iter().enumerate() {
            sheet.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
            sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(2.0 * x + 1.0)).unwrap();
        }
        sheet.set_cell(CellCoord::new(0, 1, 0), Cell::number(10.0)).unwrap();
        sheet.set_cell(CellCoord::new(1, 1, 0), Cell { excluded: true, ..Cell::number(20.0) }).unwrap();

        // Wider plot_x_min / plot_x_max must still integrate fitted curve across data [X min, X max] = [0, 4].
        let mut nl = NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new("straight_line") };
        nl.plot_x_min = Some(-5.0);
        nl.plot_x_max = Some(10.0);
        nl.parameter_rules = vec![ParameterRule {
            name: "Slope".to_owned(),
            scope: ParameterScope::SharedGlobal,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }];
        let mut spec = AnalysisSpec::new("fit-edge-auc", "edge-auc-ds", AnalysisKind::NonlinearRegression(nl)).unwrap();
        // Fit only DS0 with plot_x bounds, or let DS0 & DS1 run: DS1 has 1 finite point so must be skipped by AUC.
        spec.selected_data_sets = vec![0, 1];
        let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();

        // DS0 AUC on [0, 4] for y = 2x + 1 is 0.5 * (1 + 9) * 4 = 20.0 (not integrated over [-5, 10]!).
        let ds0_emp = res.lookup_scalar("AUC (trapezoidal)", Some(0)).expect("DS0 emp AUC");
        let ds0_fit = res.lookup_scalar("AUC (fitted curve)", Some(0)).expect("DS0 fit AUC");
        assert!((ds0_emp - 20.0).abs() < 1e-12);
        assert!((ds0_fit - 20.0).abs() < 1e-10);

        // DS1 has < 2 finite points -> no AUC scalars emitted for DS1, only DS0 column in fit_auc tab.
        assert!(res.lookup_scalar("AUC (trapezoidal)", Some(1)).is_none());
        assert!(res.lookup_scalar("AUC (fitted curve)", Some(1)).is_none());
        let auc_tab = res.tabs.iter().find(|t| t.id == "fit_auc").expect("fit_auc tab");
        assert_eq!(auc_tab.column_headers, vec!["Valid"]);
        assert_num_cell_approx(&auc_tab.rows[2][0], 0.0, 1e-12, "X min");
        assert_num_cell_approx(&auc_tab.rows[3][0], 4.0, 1e-12, "X max");
    }

    #[test]
    fn clipped_overlay_auc_matches_exact_original_point_weights() {
        use curvebook_doc::{CurveSamplePoint, FittedCurveOverlay};
        use curvebook_testkit::assert_approx_eq;

        let xs = [0.0, 2.0, 5.0, 9.0];
        let mut scratch = Vec::new();
        let mut collapsed = Vec::new();
        let mut integrate = |ys: [f64; 4]| {
            let overlay = FittedCurveOverlay {
                data_set_index: 0,
                equation_id: "straight_line".to_owned(),
                converged: true,
                ambiguous: false,
                points: xs
                    .into_iter()
                    .zip(ys)
                    .map(|(x, y)| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None })
                    .collect(),
            };
            super::compute_curve_overlay_auc(&overlay, 0.5, 7.5, &mut scratch, &mut collapsed).expect("both limits lie inside the overlay")
        };

        // Integrate the linear basis functions on [0.5, 2], [2, 5], and [5, 7.5].
        // Their contributions are [9/16, 15/16], [3/2, 3/2], and [55/32, 25/32].
        // With baseline-relative ordinates [-2, 3, -1, 2], the dot product is 145/32.
        let abs_tol = 1e-12;
        assert_approx_eq(integrate([-2.0, 3.0, -1.0, 2.0]), 145.0 / 32.0, abs_tol, 0.0, "clipped signed area");
        for (i, weight) in [9.0 / 16.0, 39.0 / 16.0, 103.0 / 32.0, 25.0 / 32.0].into_iter().enumerate() {
            let mut basis = [0.0; 4];
            basis[i] = 1.0;
            assert_approx_eq(integrate(basis), weight, abs_tol, 0.0, &format!("clipped coefficient at x={}", xs[i]));
        }
    }

    #[test]
    fn compute_curve_overlay_auc_rejects_narrowed_or_disjoint_overlays() {
        use curvebook_doc::{CurveSamplePoint, FittedCurveOverlay};

        let pt = |x: f64, y: f64| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None };
        let narrowed = FittedCurveOverlay {
            data_set_index: 0,
            equation_id: "constant".to_owned(),
            converged: true,
            ambiguous: false,
            points: vec![pt(5.0, 2.0), pt(7.5, 2.0), pt(10.0, 2.0)],
        };
        let mut scratch = Vec::new();
        let mut collapsed = Vec::new();

        // Full [0, 10] requested while overlay only covers [5, 10] -> must return None (not truncated 10.0).
        assert_eq!(super::compute_curve_overlay_auc(&narrowed, 0.0, 10.0, &mut scratch, &mut collapsed), None);
        // Right-truncated overlay on [0, 5] while [0, 10] requested -> None.
        let left_only = FittedCurveOverlay { points: vec![pt(0.0, 2.0), pt(2.5, 2.0), pt(5.0, 2.0)], ..narrowed.clone() };
        assert_eq!(super::compute_curve_overlay_auc(&left_only, 0.0, 10.0, &mut scratch, &mut collapsed), None);
        // Disjoint [20, 30] requested when overlay spans [5, 10] -> None (not 0.0).
        assert_eq!(super::compute_curve_overlay_auc(&narrowed, 20.0, 30.0, &mut scratch, &mut collapsed), None);
        // Degenerate single-point interval outside overlay -> None.
        assert_eq!(super::compute_curve_overlay_auc(&narrowed, 0.0, 0.0, &mut scratch, &mut collapsed), None);
        // Matching sub-interval [5, 10] covered by overlay -> exact trapezoidal area 10.0.
        let covered = super::compute_curve_overlay_auc(&narrowed, 5.0, 10.0, &mut scratch, &mut collapsed).expect("covered [5, 10]");
        assert!((covered - 10.0).abs() < 1e-12);

        // Unsorted / duplicate-X overlay must also enforce coverage after collapse.
        let unsorted_partial = FittedCurveOverlay { points: vec![pt(8.0, 2.0), pt(5.0, 2.0), pt(8.0, 2.0)], ..narrowed };
        assert_eq!(super::compute_curve_overlay_auc(&unsorted_partial, 0.0, 10.0, &mut scratch, &mut collapsed), None);
    }

    #[test]
    fn r19_6_append_nonlinear_parameter_tab_applies_abs_and_empty_for_nan_across_all_branches() {
        use curvebook_doc::{IdentifiabilityMode, ResultSheet};

        use crate::diagnostics::CovarianceDiagnostics;
        use crate::error::{FitIdentifiability, FitStatus, ParameterNote};
        use crate::solver::{FittedParameterSummary, InferenceAvailability, NonlinearFitResult};

        let make_param = |slot_index: usize, name: &str, hougaard_g1: f64, notes: Vec<ParameterNote>| FittedParameterSummary {
            slot_index,
            name: name.to_owned(),
            dataset_index: None,
            estimate: 1.5,
            se: 0.25,
            ci_lower: CiBound::Finite(1.0),
            ci_upper: CiBound::Finite(2.0),
            symmetric_ci_lower: 1.0,
            symmetric_ci_upper: 2.0,
            dependency: 0.2,
            hougaard_g1,
            notes,
            inference_availability: InferenceAvailability::Available,
        };

        let fit = NonlinearFitResult {
            equation_id: "test_eq".to_owned(),
            equation_name: "Test Eq".to_owned(),
            fit_method: curvebook_doc::FitMethod::LeastSquares,
            inference_availability: InferenceAvailability::Available,
            status: FitStatus::Converged,
            identifiability: FitIdentifiability::Identifiable,
            iterations: 5,
            n_obs: 10,
            n_outliers_excluded: 0,
            k_free: 4,
            df: 6,
            rss: 1.0,
            weighted_ss: 1.0,
            predicted_y_neg_log_weight_sum: None,
            rsd: (1.0_f64 / 6.0).sqrt(),
            rsdr: None,
            r_squared: 0.99,
            adj_r_squared: 0.985,
            rmse: (1.0_f64 / 10.0).sqrt(),
            weighted_r_squared: None,
            weighted_adj_r_squared: None,
            weighted_rmse: None,
            weighted_sy_x: None,
            poisson_gof: None,
            aic_c: Some(10.0),
            bic: Some(11.0),
            parameters: vec![
                make_param(0, "NegNormal", -13.0577, Vec::new()),
                make_param(1, "NanNormal", f64::NAN, Vec::new()),
                make_param(2, "NegUnstable", -4.25, vec![ParameterNote::Unstable]),
                make_param(3, "NanUnstable", f64::NAN, vec![ParameterNote::Unstable]),
            ],
            fixed_constants: Vec::new(),
            covariance: CovarianceDiagnostics {
                standard_errors: vec![0.25; 4],
                symmetric_ci_lower: vec![1.0; 4],
                symmetric_ci_upper: vec![2.0; 4],
                covariance_matrix: Vec::new(),
                correlation_matrix: Vec::new(),
                dependencies: vec![0.2; 4],
                numerical_rank: 4,
                ambiguous: false,
            },
            datasets: Vec::new(),
            derived_scalars: Vec::new(),
            runs_test: None,
            lack_of_fit: None,
            normality: None,
            normality_anderson_darling: None,
            normality_lilliefors_ks: None,
            homoscedasticity: None,
            rout: None,
            interpolated_x: Vec::new(),
            interpolated_y: Vec::new(),
            has_shared_parameters: false,
        };

        for (profile_ci, expected_cols) in [(false, 5_usize), (true, 4_usize)] {
            let mut nl = NonlinearFitSpec::new("test_eq");
            nl.profile_likelihood_ci = profile_ci;
            nl.identifiability_mode = Some(IdentifiabilityMode::IdentifyUnstableParameters);
            let spec = AnalysisSpec::new("r196-unit", "ds1", AnalysisKind::NonlinearRegression(nl.clone())).unwrap();
            let mut res = ResultSheet::new(spec).unwrap();
            super::append_nonlinear_parameter_tab(&mut res, &fit, &nl);
            let tab = res.tabs.iter().find(|t| t.id == "nonlinear_parameters").unwrap();
            assert_eq!(tab.column_headers.len(), expected_cols);
            let hc = tab.column_headers.iter().position(|h| h == "Hougaard |g1|").unwrap();

            assert_num_cell_approx(&tab.rows[0][hc], 13.0577, 1e-12, "normal negative g1 rendered as |g1|");
            assert_eq!(tab.rows[1][hc], ResultCell::Empty, "normal NaN g1 rendered as Empty");
            assert_num_cell_approx(&tab.rows[2][hc], 4.25, 1e-12, "unstable negative g1 rendered as |g1|");
            assert_eq!(tab.rows[3][hc], ResultCell::Empty, "unstable NaN g1 rendered as Empty");
        }
    }
}
