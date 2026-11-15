//! End-to-end execution of [`AnalysisSpec`] (`LinearRegression` & `NonlinearRegression`) against
//! upstream [`DataSheet`]s, assembling complete multi-tab [`ResultSheet`] artifacts with
//! `FittedCurveOverlay`s, hookable scalars, and plain-language [`ChecklistItem`] diagnostics.

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, CiBound, CurveSamplePoint, DataSheet, FittedCurveOverlay, LinearRegressionSpec,
    NonlinearFitSpec, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
};
use lumencraft_math::PValueFormat;
use lumencraft_math::dist::norm_ppf;
use lumencraft_math::sum::NeumaierSum;

use crate::dsl::CompiledDslModel;
use crate::error::{FitError, FitStatus, ParameterNote};
use crate::library::{ParametricModel, get_builtin_model};
use crate::linear::{DemingOptions, SimpleLinearOptions, SimpleLinearResult, compare_linear_lines, deming_regression, simple_linear_regression};
use crate::solver::{ModelComparisonOutcome, NonlinearFitResult, SolverConfig, compare_models, fit_nonlinear};
use crate::types::{
    DataPoint, DatasetSeries, HookContext, MultivarExtractionSpec, extract_datasets_from_sheet, extract_datasets_from_sheet_with_options,
    filter_datasets_by_range,
};

fn p_cell(fmt: PValueFormat, p: f64) -> ResultCell {
    ResultCell::PValue { raw: p, formatted: fmt.format_p(p), stars: fmt.stars(p).to_owned() }
}

fn num_cell(v: f64) -> ResultCell {
    if v.is_finite() { ResultCell::Number { value: v, prefix: None, units: None } } else { ResultCell::Empty }
}

fn ci_cell(lower: CiBound, upper: CiBound) -> ResultCell {
    let sanitize = |bound: CiBound| match bound {
        CiBound::Finite(v) if v.is_finite() => CiBound::Finite(v),
        _ => CiBound::Open,
    };
    ResultCell::ConfidenceInterval { lower: sanitize(lower), upper: sanitize(upper) }
}

/// Execute either a [`AnalysisKind::LinearRegression`] or [`AnalysisKind::NonlinearRegression`]
/// [`AnalysisSpec`] against a [`DataSheet`] and return a populated [`ResultSheet`].
pub fn execute_analysis(spec: &AnalysisSpec, sheet: &DataSheet, hooks: &HookContext) -> Result<ResultSheet, FitError> {
    match &spec.kind {
        AnalysisKind::LinearRegression(lin) => execute_linear_analysis(spec, lin, sheet),
        AnalysisKind::NonlinearRegression(nl) => execute_nonlinear_analysis(spec, nl, sheet, hooks),
        other => Err(FitError::InvalidParameter {
            name: "analysis.kind",
            reason: format!("Unsupported analysis kind `{}` for lumencraft-fit", other.derive_auto_title()),
        }),
    }
}

/// Execute linear regression (OLS, origin-forced, ANCOVA line comparison, or Model II Deming)
/// and populate a [`ResultSheet`].
pub fn execute_linear_analysis(spec: &AnalysisSpec, lin_spec: &LinearRegressionSpec, sheet: &DataSheet) -> Result<ResultSheet, FitError> {
    let mut result = ResultSheet::new(spec.clone())?;
    let datasets = extract_datasets_from_sheet(sheet, &spec.selected_data_sets)?;
    let p_fmt = spec.p_value_format;

    if lin_spec.deming_model_ii {
        return build_deming_result_sheet(result, &datasets, spec.confidence_level);
    }

    let mut fits: Vec<(usize, String, SimpleLinearResult)> = Vec::with_capacity(datasets.len());
    for ds in &datasets {
        let mut opts = SimpleLinearOptions {
            force_through_origin: lin_spec.force_through_origin,
            confidence_level: spec.confidence_level,
            compute_bands: lin_spec.confidence_bands || lin_spec.prediction_bands,
            band_grid_points: 101,
            unknown_y_values: Vec::new(),
            unknown_x_values: Vec::new(),
        };
        if lin_spec.interpolate_unknowns {
            opts.unknown_y_values = ds.unknowns_y.iter().map(|&(_, y)| y).collect();
            opts.unknown_x_values = ds.unknowns_x.iter().map(|&(_, x)| x).collect();
        }
        let fit = simple_linear_regression(&ds.points, &opts)?;
        let overlay_points = if fit.bands.is_empty() {
            build_linear_overlay_points(&ds.points, fit.slope, fit.intercept, opts.band_grid_points)
        } else {
            fit.bands.clone()
        };
        result.fitted_curves.push(FittedCurveOverlay {
            data_set_index: ds.data_set_index,
            equation_id: if lin_spec.force_through_origin { "line_through_origin".to_owned() } else { "straight_line".to_owned() },
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
    let col_headers: Vec<String> = fits.iter().map(|(_, lbl, _)| lbl.clone()).collect();
    let row_labels = vec![
        "Slope".to_owned(),
        "Y-intercept".to_owned(),
        "SE of Slope".to_owned(),
        "SE of Intercept".to_owned(),
        "95% CI Slope".to_owned(),
        "R squared".to_owned(),
        "Sy.x".to_owned(),
        "F".to_owned(),
        "P value".to_owned(),
        "Runs test P value".to_owned(),
    ];
    let mut rows = vec![Vec::with_capacity(fits.len()); row_labels.len()];
    for (_, _, f) in &fits {
        rows[0].push(num_cell(f.slope));
        rows[1].push(num_cell(f.intercept));
        rows[2].push(num_cell(f.se_slope));
        rows[3].push(num_cell(f.se_intercept));
        rows[4].push(ResultCell::ConfidenceInterval { lower: CiBound::Finite(f.ci_slope.0), upper: CiBound::Finite(f.ci_slope.1) });
        rows[5].push(num_cell(f.r_squared));
        rows[6].push(num_cell(f.sy_x));
        rows[7].push(num_cell(f.f_stat));
        rows[8].push(p_cell(p_fmt, f.p_value));
        rows[9].push(p_cell(p_fmt, f.runs_test.p_too_few_runs));
    }
    result.tabs.push(ResultTab {
        id: "linear_summary".to_owned(),
        title: "Best-fit values".to_owned(),
        kind: ResultTabKind::MainSummary,
        column_headers: col_headers,
        row_labels,
        rows,
        footnotes: Vec::new(),
    });

    // Multi-line ANCOVA comparison if >1 dataset.
    if datasets.len() >= 2 {
        let grouped_slices: Vec<&[crate::types::DataPoint]> = datasets.iter().map(|d| d.points.as_slice()).collect();
        if let Ok(cmp) = compare_linear_lines(&grouped_slices, lin_spec.force_through_origin) {
            let mut cmp_rows = vec![vec![
                num_cell(cmp.f_slopes),
                ResultCell::Text(format!("{}, {}", cmp.df_slopes.0, cmp.df_slopes.1)),
                p_cell(p_fmt, cmp.p_slopes),
                ResultCell::Boolean(cmp.slopes_significantly_different),
            ]];
            let mut cmp_labels = vec!["Are slopes equal?".to_owned()];
            if let (Some(fi), Some(dfi), Some(pi), Some(sig_i)) =
                (cmp.f_intercepts, cmp.df_intercepts, cmp.p_intercepts, cmp.intercepts_significantly_different)
            {
                cmp_labels.push("Are intercepts equal?".to_owned());
                cmp_rows.push(vec![num_cell(fi), ResultCell::Text(format!("{}, {}", dfi.0, dfi.1)), p_cell(p_fmt, pi), ResultCell::Boolean(sig_i)]);
            }
            result.tabs.push(ResultTab {
                id: "linear_comparison".to_owned(),
                title: "Comparison of Lines (ANCOVA)".to_owned(),
                kind: ResultTabKind::ModelComparison,
                column_headers: vec!["F".to_owned(), "DFn, DFd".to_owned(), "P value".to_owned(), "Significantly different?".to_owned()],
                row_labels: cmp_labels,
                rows: cmp_rows,
                footnotes: vec![format!("Pooled slope = {:.6}", cmp.pooled_slope)],
            });
        }
    }

    // Interpolated unknowns tab.
    if lin_spec.interpolate_unknowns {
        let mut interp_rows = Vec::new();
        let mut interp_labels = Vec::new();
        for (_, lbl, f) in &fits {
            for item in &f.interpolated_x_from_y {
                interp_labels.push(format!("{lbl} (X from Y={:.4})", item.input_value));
                interp_rows.push(vec![
                    num_cell(item.input_value),
                    num_cell(item.estimate),
                    ResultCell::ConfidenceInterval { lower: item.ci_lower, upper: item.ci_upper },
                ]);
            }
            for item in &f.interpolated_y_from_x {
                interp_labels.push(format!("{lbl} (Y from X={:.4})", item.input_value));
                interp_rows.push(vec![
                    num_cell(item.input_value),
                    num_cell(item.estimate),
                    ResultCell::ConfidenceInterval { lower: item.ci_lower, upper: item.ci_upper },
                ]);
            }
        }
        if !interp_rows.is_empty() {
            result.tabs.push(ResultTab {
                id: "linear_interpolated".to_owned(),
                title: "Interpolated unknowns".to_owned(),
                kind: ResultTabKind::InterpolatedUnknowns,
                column_headers: vec!["Input".to_owned(), "Interpolated".to_owned(), "Confidence Interval".to_owned()],
                row_labels: interp_labels,
                rows: interp_rows,
                footnotes: Vec::new(),
            });
        }
    }

    append_auc_scalars_and_tab(&mut result, &datasets);
    result.status = ResultStatus::UpToDate;
    Ok(result)
}

fn build_deming_result_sheet(
    mut result: ResultSheet,
    datasets: &[crate::types::DatasetSeries],
    confidence: lumencraft_math::ConfidenceLevel,
) -> Result<ResultSheet, FitError> {
    let opts = DemingOptions { variance_ratio_lambda: 1.0, confidence_level: confidence };
    let mut headers = Vec::with_capacity(datasets.len());
    let row_labels = vec![
        "Slope".to_owned(),
        "Y-intercept".to_owned(),
        "SE Slope (Jackknife)".to_owned(),
        "SE Intercept (Jackknife)".to_owned(),
        "CI Slope".to_owned(),
        "CI Intercept".to_owned(),
        "Sy.x".to_owned(),
    ];
    let mut rows = vec![Vec::with_capacity(datasets.len()); row_labels.len()];

    for ds in datasets {
        let dem = deming_regression(&ds.points, &opts)?;
        headers.push(ds.label.clone());
        rows[0].push(num_cell(dem.slope));
        rows[1].push(num_cell(dem.intercept));
        rows[2].push(num_cell(dem.se_slope));
        rows[3].push(num_cell(dem.se_intercept));
        rows[4].push(ResultCell::ConfidenceInterval { lower: CiBound::Finite(dem.ci_slope.0), upper: CiBound::Finite(dem.ci_slope.1) });
        rows[5].push(ResultCell::ConfidenceInterval { lower: CiBound::Finite(dem.ci_intercept.0), upper: CiBound::Finite(dem.ci_intercept.1) });
        rows[6].push(num_cell(dem.sy_x));
        result.scalars.push(ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(ds.data_set_index), value: dem.slope, units: None });
        result.scalars.push(ResultScalarEntry {
            key: "YIntercept".to_owned(),
            data_set_index: Some(ds.data_set_index),
            value: dem.intercept,
            units: None,
        });
    }

    result.tabs.push(ResultTab {
        id: "deming_summary".to_owned(),
        title: "Deming (Model II) linear regression".to_owned(),
        kind: ResultTabKind::MainSummary,
        column_headers: headers,
        row_labels,
        rows,
        footnotes: vec!["Standard errors computed by Leave-One-Out jackknife.".to_owned()],
    });
    append_auc_scalars_and_tab(&mut result, datasets);
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

/// Execute nonlinear regression from [`NonlinearFitSpec`] + [`DataSheet`] and emit a complete [`ResultSheet`].
pub fn execute_nonlinear_analysis(
    spec: &AnalysisSpec,
    nl_spec: &NonlinearFitSpec,
    sheet: &DataSheet,
    hooks: &HookContext,
) -> Result<ResultSheet, FitError> {
    let mut result = ResultSheet::new(spec.clone())?;
    let mv_spec =
        MultivarExtractionSpec { group_columns: &nl_spec.mv_group_columns, x_columns: &nl_spec.mv_x_columns, y_column: nl_spec.mv_y_column };
    let datasets = match extract_datasets_from_sheet_with_options(sheet, &spec.selected_data_sets, &mv_spec) {
        Ok(ds) => ds,
        Err(FitError::EmptyInput) if nl_spec.plot_function_only => vec![DatasetSeries {
            data_set_index: 0,
            label: nl_spec.equation_id.clone(),
            points: Vec::new(),
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: None,
        }],
        Err(err) => return Err(err),
    };
    let model = resolve_model_from_spec(nl_spec)?;

    let config = SolverConfig {
        method: nl_spec.method,
        weighting: nl_spec.weighting,
        parameter_rules: nl_spec.parameter_rules.clone(),
        confidence: spec.confidence_level,
        max_iters: nl_spec.clamped_max_iters(),
        compute_profile_ci: nl_spec.profile_likelihood_ci,
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
    };

    let fit = fit_nonlinear(model.as_ref(), &datasets, hooks, &config)?;
    populate_nonlinear_result_sheet(&mut result, &fit, nl_spec, spec.p_value_format);

    if let Some(comp_spec) = &nl_spec.comparison {
        let alt_id = match comp_spec {
            lumencraft_doc::ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id }
            | lumencraft_doc::ModelComparisonSpec::Aicc { alternative_equation_id }
            | lumencraft_doc::ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id } => alternative_equation_id,
        };
        let alt_model = get_builtin_model(alt_id).ok_or_else(|| FitError::UnknownEquation { id: alt_id.clone() })?;
        let (alt_fit, outcome) = compare_models(&fit, alt_model.as_ref(), &datasets, hooks, &config, comp_spec)?;
        append_comparison_tab(&mut result, &fit, &alt_fit, &outcome, spec.p_value_format);
    }

    let mut filtered_datasets: Option<Vec<DatasetSeries>> = None;
    if !config.plot_function_only && (config.x_range_filter.is_some() || config.y_range_filter.is_some()) {
        let mut copy = datasets.to_vec();
        filter_datasets_by_range(&mut copy, config.x_range_filter, config.y_range_filter);
        filtered_datasets = Some(copy);
    }
    let auc_datasets = filtered_datasets.as_deref().unwrap_or(&datasets);
    append_auc_scalars_and_tab(&mut result, auc_datasets);

    result.status = ResultStatus::UpToDate;
    Ok(result)
}

fn parameter_display_label(fit: &NonlinearFitResult, name: &str, dataset_index: Option<usize>) -> String {
    match dataset_index {
        Some(d) if fit.datasets.len() > 1 => format!("{name} [DS{}]", d + 1),
        _ => name.to_owned(),
    }
}

fn append_nonlinear_parameter_tab(result: &mut ResultSheet, fit: &NonlinearFitResult, nl_spec: &NonlinearFitSpec) {
    let mut param_rows = Vec::with_capacity(fit.parameters.len() + fit.fixed_constants.len());
    let mut param_labels = Vec::with_capacity(param_rows.capacity());

    for p in &fit.parameters {
        param_labels.push(parameter_display_label(fit, &p.name, p.dataset_index));
        let prefix = if p.notes.contains(&ParameterNote::Ambiguous) { Some("~".to_owned()) } else { None };
        let ci_display = if nl_spec.method == lumencraft_doc::FitMethod::RobustCauchy {
            ResultCell::Text("Not applicable".to_owned())
        } else {
            ci_cell(p.ci_lower, p.ci_upper)
        };
        param_rows.push(vec![
            ResultCell::Number { value: p.estimate, prefix, units: None },
            num_cell(p.se),
            ci_display,
            num_cell(p.dependency),
            num_cell(p.hougaard_g1),
        ]);
        result.scalars.push(ResultScalarEntry { key: p.name.clone(), data_set_index: p.dataset_index, value: p.estimate, units: None });
    }

    for (name, val, _) in &fit.fixed_constants {
        param_labels.push(format!("{name} (Constant)"));
        param_rows.push(vec![num_cell(*val), ResultCell::Empty, ResultCell::Text("Fixed".to_owned()), ResultCell::Empty, ResultCell::Empty]);
        result.scalars.push(ResultScalarEntry { key: name.clone(), data_set_index: None, value: *val, units: None });
    }

    result.tabs.push(ResultTab {
        id: "nonlinear_parameters".to_owned(),
        title: "Best-fit values & confidence intervals".to_owned(),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: vec![
            "Best-fit value".to_owned(),
            "Std. Error".to_owned(),
            "Confidence Interval".to_owned(),
            "Dependency".to_owned(),
            "Hougaard |g1|".to_owned(),
        ],
        row_labels: param_labels,
        rows: param_rows,
        footnotes: vec![format!(
            "Status: {:?}, Iterations: {}, DF: {}, R2 = {:.6}, Sy.x = {:.6}, RSS = {:.6}",
            fit.status, fit.iterations, fit.df, fit.r_squared, fit.rsd, fit.rss
        )],
    });
}

fn populate_nonlinear_result_sheet(result: &mut ResultSheet, fit: &NonlinearFitResult, nl_spec: &NonlinearFitSpec, p_fmt: PValueFormat) {
    append_nonlinear_parameter_tab(result, fit, nl_spec);

    // Goodness-of-fit summary tab.
    let mut gof_labels = vec![
        "Degrees of Freedom".to_owned(),
        "R squared".to_owned(),
        "Sum of Squares (RSS)".to_owned(),
        "Sy.x".to_owned(),
        "AICc".to_owned(),
        "BIC".to_owned(),
        "Number of points analyzed".to_owned(),
    ];
    let mut gof_vals = vec![
        vec![num_cell(fit.df as f64)],
        vec![num_cell(fit.r_squared)],
        vec![num_cell(fit.rss)],
        vec![num_cell(fit.rsd)],
        vec![fit.aic_c.map_or(ResultCell::Empty, num_cell)],
        vec![fit.bic.map_or(ResultCell::Empty, num_cell)],
        vec![num_cell(fit.n_obs as f64)],
    ];
    if let Some(r) = &fit.runs_test {
        gof_labels.push("Runs test P value".to_owned());
        gof_vals.push(vec![p_cell(p_fmt, r.p_too_few_runs)]);
    }
    for norm in [&fit.normality, &fit.normality_anderson_darling, &fit.normality_lilliefors_ks].into_iter().flatten() {
        gof_labels.push(format!("Normality ({}) P value", norm.test_name));
        gof_vals.push(vec![p_cell(p_fmt, norm.p_value)]);
    }
    if let Some(homo) = &fit.homoscedasticity {
        gof_labels.push("Homoscedasticity (Spearman rho)".to_owned());
        gof_vals.push(vec![num_cell(homo.spearman_rho)]);
        gof_labels.push("Homoscedasticity P value".to_owned());
        gof_vals.push(vec![p_cell(p_fmt, homo.p_value)]);
    }
    if fit.n_outliers_excluded > 0 {
        gof_labels.push("ROUT outliers excluded".to_owned());
        gof_vals.push(vec![num_cell(fit.n_outliers_excluded as f64)]);
    }
    result.tabs.push(ResultTab {
        id: "nonlinear_gof".to_owned(),
        title: "Goodness of Fit".to_owned(),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![fit.equation_name.clone()],
        row_labels: gof_labels,
        rows: gof_vals,
        footnotes: Vec::new(),
    });

    if nl_spec.emit_residuals_tab {
        append_nonlinear_residuals_tab(result, fit);
    }
    if nl_spec.emit_covariance_tab {
        append_nonlinear_covariance_tab(result, fit);
    }
    append_nonlinear_interpolated_tab(result, fit);

    // Curve overlays + derived scalars.
    for (idx, ds_res) in fit.datasets.iter().enumerate() {
        if let Some(overlay) = &ds_res.curve_overlay {
            result.fitted_curves.push(overlay.clone());
        }
        if let Some(derived) = fit.derived_scalars.get(idx) {
            for d in derived {
                result.scalars.push(ResultScalarEntry {
                    key: d.name.clone(),
                    data_set_index: Some(ds_res.data_set_index),
                    value: d.value,
                    units: d.units.clone(),
                });
            }
        }
    }
    result.scalars.push(ResultScalarEntry { key: "R2".to_owned(), data_set_index: None, value: fit.r_squared, units: None });
    result.scalars.push(ResultScalarEntry { key: "RSS".to_owned(), data_set_index: None, value: fit.rss, units: None });

    // Diagnostics checklist items.
    if fit.status == FitStatus::Ambiguous || fit.covariance.ambiguous {
        result.checklist.push(ChecklistItem {
            assumption: "Parameter identifiability".to_owned(),
            status: ChecklistStatus::Violated,
            diagnostic_metric: Some("Dependency > 0.9999 or rank-deficient Jacobian".to_owned()),
            explanation: "Fit is ambiguous; one or more parameters are redundant or unidentifiable.".to_owned(),
            remediation: None,
        });
    } else if fit.parameters.iter().any(|p| p.dependency > 0.99) {
        result.checklist.push(ChecklistItem {
            assumption: "Multicollinearity (Dependency)".to_owned(),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some("Dependency > 0.99".to_owned()),
            explanation: "Strong correlation among parameters may widen confidence intervals.".to_owned(),
            remediation: None,
        });
    } else {
        result.checklist.push(ChecklistItem {
            assumption: "Parameter identifiability".to_owned(),
            status: ChecklistStatus::Pass,
            diagnostic_metric: None,
            explanation: "All free parameters are cleanly identifiable.".to_owned(),
            remediation: None,
        });
    }

    if let Some(max_g1) = fit.parameters.iter().map(|p| p.hougaard_g1.abs()).reduce(f64::max)
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

    if let Some(runs) = &fit.runs_test {
        result.checklist.push(ChecklistItem {
            assumption: "Runs test for systematic deviation".to_owned(),
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
        result.checklist.push(ChecklistItem {
            assumption: "ROUT outlier detection".to_owned(),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some(format!(
                "Q = {:.2}%, RSDR = {:.4}, outliers = {}",
                rout.q_threshold_percent,
                rout.rsdr,
                rout.outlier_indices.len()
            )),
            explanation: format!("{} outlier(s) identified and excluded before ordinary least-squares refit.", rout.outlier_indices.len()),
            remediation: None,
        });
    }
}

fn append_comparison_tab(
    result: &mut ResultSheet,
    primary: &NonlinearFitResult,
    alt: &NonlinearFitResult,
    outcome: &ModelComparisonOutcome,
    p_fmt: PValueFormat,
) {
    match outcome {
        ModelComparisonOutcome::ExtraSumOfSquaresF(f) => {
            result.tabs.push(ResultTab {
                id: "model_comparison_f".to_owned(),
                title: "Comparison of Models (Extra sum-of-squares F test)".to_owned(),
                kind: ResultTabKind::ModelComparison,
                column_headers: vec!["Value".to_owned()],
                row_labels: vec![
                    "Null hypothesis model".to_owned(),
                    "Alternative model".to_owned(),
                    "F (DFn, DFd)".to_owned(),
                    "P value".to_owned(),
                    "Preferred model".to_owned(),
                ],
                rows: vec![
                    vec![ResultCell::Text(primary.equation_name.clone())],
                    vec![ResultCell::Text(alt.equation_name.clone())],
                    vec![ResultCell::Text(format!("{:.4} ({}, {})", f.f_stat, f.df_num, f.df_den))],
                    vec![p_cell(p_fmt, f.p_value)],
                    vec![ResultCell::Text(if f.preferred_complex_model { "More complex model".to_owned() } else { "Simpler model".to_owned() })],
                ],
                footnotes: Vec::new(),
            });
        }
        ModelComparisonOutcome::Aicc(a) => {
            result.tabs.push(ResultTab {
                id: "model_comparison_aicc".to_owned(),
                title: "Comparison of Models (AICc)".to_owned(),
                kind: ResultTabKind::ModelComparison,
                column_headers: vec![primary.equation_name.clone(), alt.equation_name.clone()],
                row_labels: vec!["AICc".to_owned(), "Akaike weight".to_owned(), "Difference in AICc".to_owned(), "Evidence ratio".to_owned()],
                rows: vec![
                    vec![num_cell(a.aicc_a), num_cell(a.aicc_b)],
                    vec![num_cell(a.weight_a), num_cell(a.weight_b)],
                    vec![num_cell(a.delta_aicc), ResultCell::Empty],
                    vec![num_cell(a.evidence_ratio), ResultCell::Empty],
                ],
                footnotes: Vec::new(),
            });
        }
        ModelComparisonOutcome::LikelihoodRatioTest(lrt) => {
            result.tabs.push(ResultTab {
                id: "model_comparison_lrt".to_owned(),
                title: "Comparison of Models (Likelihood Ratio Test)".to_owned(),
                kind: ResultTabKind::ModelComparison,
                column_headers: vec!["Value".to_owned()],
                row_labels: vec![
                    "Null hypothesis model".to_owned(),
                    "Alternative model".to_owned(),
                    "LR statistic D = 2*(ln L_alt - ln L_null)".to_owned(),
                    "DF (diff)".to_owned(),
                    "P value".to_owned(),
                    "Preferred model".to_owned(),
                ],
                rows: vec![
                    vec![ResultCell::Text(primary.equation_name.clone())],
                    vec![ResultCell::Text(alt.equation_name.clone())],
                    vec![num_cell(lrt.lr_statistic)],
                    vec![num_cell(lrt.df_diff as f64)],
                    vec![p_cell(p_fmt, lrt.p_value)],
                    vec![ResultCell::Text(if lrt.preferred_complex_model { "More complex model".to_owned() } else { "Simpler model".to_owned() })],
                ],
                footnotes: Vec::new(),
            });
        }
    }
}

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
    let mut row_labels = Vec::with_capacity(total_pts);
    let mut rows = Vec::with_capacity(total_pts);
    let mut flat_idx = 0_usize;
    for ds in &fit.datasets {
        for i in 0..ds.y_hat.len() {
            let lbl = if multi_ds { format!("{} #{}", ds.label, i + 1) } else { format!("Pt {}", i + 1) };
            row_labels.push(lbl);
            let yh = ds.y_hat[i];
            let res = ds.residuals.get(i).copied().unwrap_or(f64::NAN);
            let w_res = ds.weighted_residuals.get(i).copied().unwrap_or(f64::NAN);
            let stud_res = ds.studentized_residuals.get(i).copied().unwrap_or(f64::NAN);
            let qq = qq_quantiles[flat_idx];
            rows.push(vec![num_cell(yh), num_cell(res), num_cell(w_res), num_cell(stud_res), num_cell(qq)]);
            flat_idx += 1;
        }
    }

    result.tabs.push(ResultTab {
        id: "nonlinear_residuals".to_owned(),
        title: "Residuals".to_owned(),
        kind: ResultTabKind::Residuals,
        column_headers: vec![
            "Predicted Y".to_owned(),
            "Residual".to_owned(),
            "Weighted Residual".to_owned(),
            "Studentized Residual".to_owned(),
            "Q-Q Normal Quantile".to_owned(),
        ],
        row_labels,
        rows,
        footnotes: Vec::new(),
    });
}

fn append_nonlinear_covariance_tab(result: &mut ResultSheet, fit: &NonlinearFitResult) {
    if fit.covariance.covariance_matrix.is_empty() {
        return;
    }
    let labels: Vec<String> = fit.parameters.iter().map(|p| parameter_display_label(fit, &p.name, p.dataset_index)).collect();
    let rows: Vec<Vec<ResultCell>> = fit.covariance.covariance_matrix.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
    result.tabs.push(ResultTab {
        id: "nonlinear_covariance".to_owned(),
        title: "Covariance Matrix".to_owned(),
        kind: ResultTabKind::CovarianceMatrix,
        column_headers: labels.clone(),
        row_labels: labels,
        rows,
        footnotes: vec![format!("Numerical rank: {}/{}", fit.covariance.numerical_rank, fit.parameters.len())],
    });
}

fn append_nonlinear_interpolated_tab(result: &mut ResultSheet, fit: &NonlinearFitResult) {
    if fit.interpolated_x.is_empty() && fit.interpolated_y.is_empty() {
        return;
    }
    let total = fit.interpolated_x.len() + fit.interpolated_y.len();
    let mut interp_rows = Vec::with_capacity(total);
    let mut interp_labels = Vec::with_capacity(total);
    let mut footnotes = Vec::new();

    for (items, direction) in [(&fit.interpolated_x, "X from Y"), (&fit.interpolated_y, "Y from X")] {
        for item in items {
            let lbl = fit
                .datasets
                .iter()
                .find(|d| d.data_set_index == item.data_set_index)
                .map_or_else(|| format!("DS{}", item.data_set_index + 1), |d| d.label.clone());
            interp_labels.push(format!("{lbl} ({direction}={:.4})", item.input_value));
            interp_rows.push(vec![
                num_cell(item.input_value),
                num_cell(item.output_estimate),
                ci_cell(item.ci_lower, item.ci_upper),
                ResultCell::Text(direction.to_owned()),
                ResultCell::Text(lbl.clone()),
            ]);
            if let Some(note) = &item.note {
                footnotes.push(format!("{lbl} (row {}, {direction}): {note}", item.row + 1));
            }
        }
    }

    result.tabs.push(ResultTab {
        id: "nonlinear_interpolated".to_owned(),
        title: "Interpolated unknowns".to_owned(),
        kind: ResultTabKind::InterpolatedUnknowns,
        column_headers: vec![
            "Input".to_owned(),
            "Interpolated".to_owned(),
            "Confidence Interval".to_owned(),
            "Direction".to_owned(),
            "Dataset".to_owned(),
        ],
        row_labels: interp_labels,
        rows: interp_rows,
        footnotes,
    });
}

#[inline]
fn canonical_zero(x: f64) -> f64 {
    if x == 0.0 { 0.0 } else { x }
}

fn build_linear_overlay_points(points: &[DataPoint], slope: f64, intercept: f64, grid_points: usize) -> Vec<CurveSamplePoint> {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    for pt in points {
        if let Some(&x) = pt.x.first()
            && x.is_finite()
        {
            x_min = x_min.min(x);
            x_max = x_max.max(x);
        }
    }
    if !x_min.is_finite() || !x_max.is_finite() {
        return Vec::new();
    }
    let count = grid_points.max(2);
    let span = x_max - x_min;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let frac = (i as f64) / ((count - 1) as f64);
        let x = x_min + frac * span;
        let y = intercept + slope * x;
        if x.is_finite() && y.is_finite() {
            out.push(CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None });
        }
    }
    out
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

fn compute_curve_overlay_auc(
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
        return trapezoidal_auc_over_interval(scratch, x_min, x_max);
    }
    scratch.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    collapse_duplicate_x(scratch, collapsed);
    if x_max == x_min {
        let covers = collapsed.first().is_some_and(|p| p.0 <= x_min) && collapsed.last().is_some_and(|p| p.0 >= x_max);
        return covers.then_some(0.0);
    }
    trapezoidal_auc_over_interval(collapsed, x_min, x_max)
}

fn append_auc_scalars_and_tab(result: &mut ResultSheet, datasets: &[DatasetSeries]) {
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

        let fitted_auc = overlay.and_then(|ov| compute_curve_overlay_auc(ov, x_min, x_max, &mut curve_raw_scratch, &mut curve_collapsed_scratch));
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

    result.tabs.push(ResultTab {
        id: "fit_auc".to_owned(),
        title: "Area Under the Curve (AUC)".to_owned(),
        kind: ResultTabKind::Diagnostics,
        column_headers: col_headers,
        row_labels: vec!["Trapezoidal AUC".to_owned(), "Fitted curve AUC".to_owned(), "X min".to_owned(), "X max".to_owned()],
        rows: vec![row_emp, row_fit, row_xmin, row_xmax],
        footnotes: vec![
            "Trapezoidal AUC integrates replicate-averaged (X, Mean Y) across [X min, X max]; fitted curve AUC integrates the evaluated curve overlay across [X min, X max]."
                .to_owned(),
        ],
    });
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{
        AnalysisKind, AnalysisSpec, Cell, CellCoord, CiBound, DataSheet, LinearRegressionSpec, NonlinearFitSpec, ParameterRule, ParameterScope,
        ResultCell, ResultStatus, ResultTabKind, SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
    };
    use lumencraft_math::ConfidenceLevel;

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
        assert_eq!(interp_tab.column_headers, vec!["Input", "Interpolated", "Confidence Interval", "Direction", "Dataset"]);
        assert_eq!(interp_tab.rows.len(), 3);
        assert_eq!(interp_tab.row_labels.len(), 3);
        assert!(interp_tab.footnotes.is_empty());

        for (idx, expected_x) in [-6.2_f64, -5.4_f64].iter().enumerate() {
            assert!(interp_tab.row_labels[idx].contains("X from Y="));
            let ResultCell::Number { value, .. } = interp_tab.rows[idx][1] else {
                panic!("expected numeric interpolated X in row {idx}");
            };
            assert!((value - expected_x).abs() < 1e-4, "row {idx}: got {value}, expected {expected_x}");
            assert_eq!(interp_tab.rows[idx][3], ResultCell::Text("X from Y".to_owned()));
            assert_eq!(interp_tab.rows[idx][4], ResultCell::Text("A".to_owned()));
        }

        assert!(interp_tab.row_labels[2].contains("Y from X="));
        let ResultCell::Number { value: est_y, .. } = interp_tab.rows[2][1] else {
            panic!("expected numeric interpolated Y in row 2");
        };
        assert!((est_y - 52.5).abs() < 1e-4, "expected Y=52.5 at LogEC50, got {est_y}");
        let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } = interp_tab.rows[2][2] else {
            panic!("expected finite CI for interpolated Y");
        };
        assert!(lo <= est_y && est_y <= hi);
        assert_eq!(interp_tab.rows[2][3], ResultCell::Text("Y from X".to_owned()));
        assert_eq!(interp_tab.rows[2][4], ResultCell::Text("A".to_owned()));
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
        assert_eq!(interp_tab.rows[2][2], ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open });
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

        // linear_summary must preserve exact original 10 row labels without inserted AUC rows.
        let summary = res.tabs.iter().find(|t| t.id == "linear_summary").expect("linear_summary tab");
        assert_eq!(summary.rows.len(), 10);
        assert_eq!(summary.row_labels[0], "Slope");
        assert_eq!(summary.row_labels[9], "Runs test P value");

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

        // Deming Model II emits no FittedCurveOverlay, so AUC (trapezoidal) is emitted and Fitted curve AUC is Empty.
        let dem_spec = AnalysisSpec::new(
            "dem-auc",
            "lin-auc-ds",
            AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() }),
        )
        .unwrap();
        let dem_res = execute_analysis(&dem_spec, &sheet, &HookContext::default()).unwrap();
        assert!(dem_res.scalars.iter().any(|s| s.key == "AUC (trapezoidal)" && s.data_set_index == Some(0)));
        assert!(!dem_res.scalars.iter().any(|s| s.key == "AUC (fitted curve)"));
        let dem_auc_tab = dem_res.tabs.iter().find(|t| t.id == "fit_auc").expect("Deming fit_auc tab");
        assert_num_cell_approx(&dem_auc_tab.rows[0][0], 12.0, 1e-12, "Deming DS0 emp");
        assert_eq!(dem_auc_tab.rows[1][0], ResultCell::Empty);
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
}
