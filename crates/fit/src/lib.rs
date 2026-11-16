#![forbid(unsafe_code)]
//! Linear, Deming (Model II), and Nonlinear Regression + Custom Bytecode/ODE Equation DSL
//! engine for CurveBook (`Layer L2`).

pub mod diagnostics;
pub mod dsl;
pub mod error;
pub mod library;
pub mod linear;
pub mod result;
pub mod solver;
pub mod tex_format;
pub mod types;

pub use diagnostics::{
    AiccComparisonResult, CovarianceDiagnostics, ExtraSumOfSquaresResult, HomoscedasticityResult, InformationCriteria, LackOfFitResult,
    LikelihoodRatioResult, NormalityTestResult, RoutDetectionResult, RoutPointAudit, RunsTestResult, anderson_darling_test, compare_models_aicc,
    compare_models_f_test, compare_models_lrt, compute_covariance_diagnostics, compute_deviance_profile_likelihood_cis, compute_hougaard_skewness,
    compute_information_criteria, compute_profile_likelihood_cis, compute_rsdr, dagostino_pearson_test, homoscedasticity_spearman_test,
    lilliefors_ks_test, replicates_lack_of_fit, rout_detect_outliers, runs_test, shapiro_wilk_test,
};
pub use dsl::{CompiledDslModel, DslEquationKind, integrate_rk45};
pub use error::{FitError, FitStatus, ParameterNote};
pub use library::{EquationCatalogEntry, ParametricModel, get_builtin_model, list_builtin_equations};
pub use linear::{
    DemingOptions, DemingResult, LinearComparisonResult, LinearInterpolation, LinearResidual, SimpleLinearOptions, SimpleLinearResult,
    compare_linear_lines, deming_regression, simple_linear_regression,
};
pub use result::{execute_analysis, execute_linear_analysis, execute_nonlinear_analysis, resolve_model_from_spec};
pub use solver::{
    FittedDatasetResult, FittedParameterSummary, InferenceAvailability, InterpolatedUnknown, ModelComparisonOutcome, ModelComparisonResult,
    NonlinearFitResult, SolverConfig, compare_models, compare_models_checked, fit_nonlinear,
};
pub use tex_format::{format_model_equation_tex, format_model_equation_tex_for_dataset, format_number_sig_figs, format_polynomial_equation_tex};
pub use types::{
    DataPoint, DatasetSeries, DerivedScalar, EquationCategory, HookContext, MultivarExtractionSpec, extract_datasets_from_sheet,
    extract_datasets_from_sheet_with_options, filter_datasets_by_range,
};

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, FitMethod, LinearRegressionSpec, NonlinearFitSpec, ParameterRule, ParameterScope,
        SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
    };
    use curvebook_math::ConfidenceLevel;

    use super::*;

    fn sample_xy_points(xs: &[f64], ys: &[f64]) -> Vec<DataPoint> {
        xs.iter().zip(ys.iter()).map(|(&x, &y)| DataPoint::xy(x, y)).collect()
    }

    #[test]
    fn simple_linear_and_deming_roundtrip() {
        let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let ys = [3.0, 5.0, 7.0, 9.0, 11.0, 13.0];
        let pts = sample_xy_points(&xs, &ys);

        let opts = SimpleLinearOptions { unknown_y_values: vec![8.0], unknown_x_values: vec![3.5], ..SimpleLinearOptions::default() };
        let res = simple_linear_regression(&pts, &opts).unwrap();
        assert!((res.slope - 2.0).abs() < 1e-12);
        assert!((res.intercept - 1.0).abs() < 1e-12);
        assert!((res.r_squared - 1.0).abs() < 1e-12);
        assert!((res.interpolated_x_from_y[0].estimate - 3.5).abs() < 1e-10);
        assert!((res.interpolated_y_from_x[0].estimate - 8.0).abs() < 1e-10);

        let dem = deming_regression(&pts, &DemingOptions::default()).unwrap();
        assert!((dem.slope - 2.0).abs() < 1e-12);
        assert!((dem.intercept - 1.0).abs() < 1e-12);
    }

    #[test]
    fn ancova_line_comparison_detects_parallel_shift() {
        let line1 = sample_xy_points(&[1.0, 2.0, 3.0, 4.0, 5.0], &[3.1, 4.9, 7.0, 9.1, 10.9]);
        let line2 = sample_xy_points(&[1.0, 2.0, 3.0, 4.0, 5.0], &[8.0, 10.1, 12.0, 13.9, 16.0]);
        let cmp = compare_linear_lines(&[&line1, &line2], false).unwrap();
        assert!(!cmp.slopes_significantly_different);
        assert_eq!(cmp.intercepts_significantly_different, Some(true));
    }

    #[test]
    fn dsl_if_conditional_and_global_dataset_selector() {
        let src = "\
            <A> Y = Top * X / (Kd + X)\n\
            <B> Y = IF(X < X0, Base, Base + Slope * (X - X0))\n\
        ";
        let model = CompiledDslModel::compile("mixed_ds", src).unwrap();
        // Order of first appearance: Top, Kd, X0, Base, Slope
        assert_eq!(model.param_names(), &["Top", "Kd", "X0", "Base", "Slope"]);
        let params = vec![100.0, 2.0, 5.0, 10.0, 3.0];
        // DS 0: Top * X / (Kd + X) at X=2 -> 100*2/(2+2) = 50
        let y_a = model.eval(&[2.0], &params, 0).unwrap();
        assert!((y_a - 50.0).abs() < 1e-12);
        // DS 1: Base=10, X0=5, Slope=3 at X=7 -> 10 + 3*(7-5) = 16
        let y_b1 = model.eval(&[7.0], &params, 1).unwrap();
        assert!((y_b1 - 16.0).abs() < 1e-12);
        let y_b0 = model.eval(&[2.0], &params, 1).unwrap();
        assert!((y_b0 - 10.0).abs() < 1e-12);
    }

    #[test]
    fn dsl_implicit_equation_solves_kepler_like_curve() {
        // Y + a * sin(Y) = X  => implicit equation Y + a*sin(Y) - X = 0
        let model = CompiledDslModel::compile("kepler", "Y + a * sin(Y) = X").unwrap();
        assert_eq!(model.kind(), DslEquationKind::Implicit);
        let a = 0.5_f64;
        let y_true = 1.2_f64;
        let x_val = y_true + a * y_true.sin();
        let y_solved = model.eval(&[x_val], &[a], 0).unwrap();
        assert!((y_solved - y_true).abs() < 1e-9);
    }

    #[test]
    fn dsl_ode_rk45_matches_analytical_decay() {
        let model = CompiledDslModel::compile("ode_decay", "Y0 = A0\ndY/dt = -k * Y").unwrap();
        assert_eq!(model.kind(), DslEquationKind::OdeFirstOrder);
        let ts = [0.0, 0.5, 1.0, 2.0, 4.0, 6.0];
        let a0_true = 120.0_f64;
        let k_true = 0.45_f64;
        let pts: Vec<DataPoint> = ts.iter().map(|&t| DataPoint::xy(t, a0_true * (-k_true * t).exp())).collect();
        let ds = DatasetSeries {
            data_set_index: 0,
            label: "A".to_owned(),
            points: pts,
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: None,
        };
        let fit = fit_nonlinear(&model, &[ds], &HookContext::default(), &SolverConfig::default()).unwrap();
        assert_eq!(fit.status, FitStatus::Converged);
        let k_hat = fit.parameters.iter().find(|p| p.name == "k").unwrap().estimate;
        let a0_hat = fit.parameters.iter().find(|p| p.name == "A0").unwrap().estimate;
        assert!((k_hat - k_true).abs() < 1e-6);
        assert!((a0_hat - a0_true).abs() < 1e-5);
    }

    #[test]
    fn rout_detects_injected_outlier_and_recovers_4pl() {
        let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
        let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0];
        let true_p = [10.0, 100.0, -6.5, 1.2];
        let mut pts: Vec<DataPoint> = xs
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let mut y = model.eval(&[x], &true_p, 0).unwrap();
                if i == 3 {
                    y += 85.0; // Severe positive spike outlier
                }
                DataPoint { row: i, ..DataPoint::xy(x, y) }
            })
            .collect();
        let ds = DatasetSeries {
            data_set_index: 0,
            label: "Curve".to_owned(),
            points: std::mem::take(&mut pts),
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: None,
        };
        let cfg = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() };
        let fit = fit_nonlinear(model.as_ref(), &[ds], &HookContext::default(), &cfg).unwrap();
        assert_eq!(fit.n_outliers_excluded, 1);
        assert!((fit.parameters[2].estimate - (-6.5)).abs() < 1e-3);
    }

    #[test]
    fn poisson_deviance_and_global_shared_fit_work() {
        let model = get_builtin_model("one_phase_decay").unwrap();
        let xs = [0.0, 1.0, 2.0, 4.0, 8.0, 12.0];
        let p0 = [100.0, 10.0, 0.3];
        let p1 = [200.0, 20.0, 0.3];
        let make_ds = |idx: usize, lbl: &str, params: &[f64]| DatasetSeries {
            data_set_index: idx,
            label: lbl.to_owned(),
            points: xs
                .iter()
                .enumerate()
                .map(|(r, &x)| DataPoint { row: r, data_set_index: idx, ..DataPoint::xy(x, model.eval(&[x], params, idx).unwrap()) })
                .collect(),
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: None,
        };
        let ds0 = make_ds(0, "A", &p0);
        let ds1 = make_ds(1, "B", &p1);
        let cfg = SolverConfig {
            method: FitMethod::PoissonDeviance,
            parameter_rules: vec![ParameterRule {
                name: "K".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: None,
                lower_bound: Some(0.0),
                upper_bound: None,
            }],
            ..SolverConfig::default()
        };
        let fit = fit_nonlinear(model.as_ref(), &[ds0, ds1], &HookContext::default(), &cfg).unwrap();
        let k_shared = fit.parameters.iter().find(|p| p.name == "K").unwrap();
        assert!(k_shared.notes.contains(&ParameterNote::SharedGlobal));
        assert!((k_shared.estimate - 0.3).abs() < 1e-4);
    }

    #[test]
    fn execute_analysis_builds_complete_result_sheet() {
        let xy = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
        let mut sheet = DataSheet::new("sheet-1", "Dose-Response", TableArchetype::Xy(xy)).unwrap();
        let xs = [-8.0, -7.0, -6.0, -5.0, -4.0];
        let ys = [5.0, 20.0, 52.0, 88.0, 98.0];
        for r in 0..5 {
            sheet.set_x_or_time_cell(r, 0, Cell::number(xs[r])).unwrap();
            sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(ys[r])).unwrap();
        }

        let nl = NonlinearFitSpec {
            equation_id: "log_agonist_vs_response_4pl".to_owned(),
            weighting: WeightingScheme::None,
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        };
        let mut spec = AnalysisSpec::new("fit-1", "sheet-1", AnalysisKind::NonlinearRegression(nl)).unwrap();
        spec.confidence_level = ConfidenceLevel::DEFAULT_95;
        let out = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
        assert_eq!(out.fitted_curves.len(), 1);
        assert!(out.lookup_scalar("LogEC50", Some(0)).is_some());

        let lin_spec = AnalysisSpec::new("lin-1", "sheet-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
        let lin_out = execute_analysis(&lin_spec, &sheet, &HookContext::default()).unwrap();
        assert!(lin_out.lookup_scalar("Slope", Some(0)).is_some());
    }
}
