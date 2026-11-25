#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, DataSheet, HookSource, LinearRegressionSpec, MultipleVariablesTable, NonlinearFitSpec, ParameterRule,
    ParameterScope, ResultCell, ResultSheet, ResultTab, SheetId, SubcolumnMode, TableArchetype, VariableRole, XColumnMode, XyTable,
};
use curvebook_fit::diagnostics::CovarianceDiagnostics;
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitError, FitStatus, FittedParameterSummary, HookContext, NonlinearFitResult, ParametricModel,
    SolverConfig, execute_analysis, fit_nonlinear,
};
use curvebook_testkit::assert_approx_eq;

pub fn xy_pts(xs: &[f64], ys: &[f64]) -> Vec<DataPoint> {
    assert_eq!(xs.len(), ys.len(), "xy_pts length mismatch");
    xs.iter().zip(ys.iter()).enumerate().map(|(row, (&x, &y))| DataPoint { row, ..DataPoint::xy(x, y) }).collect()
}

pub fn series_from_pts(idx: usize, label: &str, points: Vec<DataPoint>) -> DatasetSeries {
    DatasetSeries {
        data_set_index: idx,
        label: label.to_owned(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

pub fn series_xy(idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    assert_eq!(xs.len(), ys.len(), "series_xy length mismatch for {label}");
    let points = xs.iter().zip(ys.iter()).enumerate().map(|(row, (&x, &y))| DataPoint { row, data_set_index: idx, ..DataPoint::xy(x, y) }).collect();
    series_from_pts(idx, label, points)
}

pub fn series_replicates<const R: usize>(idx: usize, label: &str, xs: &[f64], reps: &[[f64; R]]) -> DatasetSeries {
    assert_eq!(xs.len(), reps.len(), "series_replicates length mismatch for {label}");
    let mut pts = Vec::with_capacity(xs.len() * R);
    for (row, (&x, trip)) in xs.iter().zip(reps.iter()).enumerate() {
        for (sub, &y) in trip.iter().enumerate() {
            pts.push(DataPoint { source_subcolumn: sub, data_set_index: idx, row, ..DataPoint::xy(x, y) });
        }
    }
    series_from_pts(idx, label, pts)
}

pub fn series_eval(model: &dyn ParametricModel, idx: usize, label: &str, xs: &[f64], params: &[f64]) -> DatasetSeries {
    let ys: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], params, idx).unwrap()).collect();
    series_xy(idx, label, xs, &ys)
}

pub fn mean_sd_n(v: &[f64]) -> (f64, f64, f64) {
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    (mean, sd, n)
}

pub fn linspace(start: f64, end: f64, n: usize) -> Vec<f64> {
    if n <= 1 {
        return vec![start];
    }
    let step = (end - start) / ((n - 1) as f64);
    (0..n).map(|i| start + (i as f64) * step).collect()
}

pub fn rule_const(name: &str, val: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Constant(val), initial_override: None, lower_bound: None, upper_bound: None }
}

pub fn rule_local(name: &str) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Local, initial_override: None, lower_bound: None, upper_bound: None }
}

pub fn rule_local_init(name: &str, init: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Local, initial_override: Some(init), lower_bound: None, upper_bound: None }
}

pub fn rule_local_bounds(name: &str, init: Option<f64>, lower: Option<f64>, upper: Option<f64>) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Local, initial_override: init, lower_bound: lower, upper_bound: upper }
}

pub fn rule_shared(name: &str) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::SharedGlobal, initial_override: None, lower_bound: None, upper_bound: None }
}

pub fn rule_shared_init(name: &str, init: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::SharedGlobal, initial_override: Some(init), lower_bound: None, upper_bound: None }
}

pub fn rule_shared_bounds(name: &str, init: Option<f64>, lower: Option<f64>, upper: Option<f64>) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::SharedGlobal, initial_override: init, lower_bound: lower, upper_bound: upper }
}

pub fn rule_per_ds(name: &str, vals: &[(usize, f64)]) -> ParameterRule {
    ParameterRule {
        name: name.to_owned(),
        scope: ParameterScope::PerDatasetConstant(vals.to_vec()),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }
}

pub fn rule_col_title(name: &str) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::ColumnTitleConstant, initial_override: None, lower_bound: None, upper_bound: None }
}

pub fn rule_hook(name: &str, source: HookSource) -> ParameterRule {
    ParameterRule {
        name: name.to_owned(),
        scope: ParameterScope::HookedConstant(source),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }
}

pub fn cfg_fast() -> SolverConfig {
    SolverConfig { compute_profile_ci: false, compute_hougaard: false, compute_bands: false, compute_runs_test: false, ..SolverConfig::default() }
}

pub fn cfg_fast_rules(parameter_rules: Vec<ParameterRule>) -> SolverConfig {
    SolverConfig { parameter_rules, ..cfg_fast() }
}

pub fn cfg_rules(parameter_rules: Vec<ParameterRule>) -> SolverConfig {
    SolverConfig { parameter_rules, ..SolverConfig::default() }
}

pub fn nl_fast(eq_id: &str) -> NonlinearFitSpec {
    NonlinearFitSpec { profile_likelihood_ci: false, ..NonlinearFitSpec::new(eq_id) }
}

pub fn fit_default(model: &dyn ParametricModel, ds: &[DatasetSeries]) -> NonlinearFitResult {
    fit_nonlinear(model, ds, &HookContext::default(), &SolverConfig::default()).unwrap()
}

pub fn fit_with(model: &dyn ParametricModel, ds: &[DatasetSeries], cfg: &SolverConfig) -> NonlinearFitResult {
    fit_nonlinear(model, ds, &HookContext::default(), cfg).unwrap()
}

pub fn xy_sheet(id: &str, title: &str, xs: &[f64], cols: &[(&str, &[f64])]) -> DataSheet {
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, cols.len(), xs.len())
            .unwrap();
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
    }
    for (c, (col_title, ys)) in cols.iter().enumerate() {
        assert_eq!(ys.len(), xs.len(), "xy_sheet column `{col_title}` len mismatch");
        xy.data_sets[c].title = (*col_title).to_owned();
        for (r, &y) in ys.iter().enumerate() {
            xy.data_sets[c].cells[r][0] = Cell::number(y);
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Xy(xy)).unwrap()
}

pub fn xy_sheet_single(id: &str, title: &str, col_title: &str, xs: &[f64], ys: &[f64]) -> DataSheet {
    xy_sheet(id, title, xs, &[(col_title, ys)])
}

pub fn xy_sheet_staggered(id: &str, title: &str, cols: &[(&str, &[f64], &[f64])]) -> DataSheet {
    let total_rows: usize = cols.iter().map(|(_, xs, _)| xs.len()).sum();
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, cols.len(), total_rows)
            .unwrap();
    let mut row_off = 0;
    for (c, (col_title, xs, ys)) in cols.iter().enumerate() {
        assert_eq!(xs.len(), ys.len(), "xy_sheet_staggered `{col_title}` len mismatch");
        xy.data_sets[c].title = (*col_title).to_owned();
        for (i, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
            xy.set_x_cell(row_off + i, Cell::number(x)).unwrap();
            xy.data_sets[c].cells[row_off + i][0] = Cell::number(y);
        }
        row_off += xs.len();
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Xy(xy)).unwrap()
}

pub fn xy_sheet_replicates<const R: usize>(id: &str, title: &str, xs: &[f64], cols: &[(&str, &[[f64; R]])]) -> DataSheet {
    let mut xy = XyTable::new_with_x_mode(
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
        SubcolumnMode::RawReplicates(R),
        cols.len(),
        xs.len(),
    )
    .unwrap();
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
    }
    for (c, (col_title, reps)) in cols.iter().enumerate() {
        xy.data_sets[c].title = (*col_title).to_owned();
        for (r, trip) in reps.iter().enumerate() {
            for (s, &y) in trip.iter().enumerate() {
                xy.data_sets[c].cells[r][s] = Cell::number(y);
            }
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Xy(xy)).unwrap()
}

pub fn mv_sheet_num(id: &str, title: &str, cols: &[(&str, &[f64])]) -> DataSheet {
    let rows = cols.first().map_or(0, |(_, v)| v.len());
    let mut mv = MultipleVariablesTable::new(cols.len(), rows).unwrap();
    for (c, (col_title, vals)) in cols.iter().enumerate() {
        mv.variables[c].title = (*col_title).to_owned();
        mv.variables[c].lock_role(VariableRole::Continuous);
        for (r, &v) in vals.iter().enumerate() {
            mv.variables[c].cells[r] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::MultipleVariables(mv)).unwrap()
}

pub fn exec_nl_res(sheet: &DataSheet, nl: NonlinearFitSpec) -> Result<ResultSheet, FitError> {
    let spec = AnalysisSpec::new("fit", sheet.id.clone(), AnalysisKind::NonlinearRegression(nl)).unwrap();
    execute_analysis(&spec, sheet, &HookContext::default())
}

pub fn exec_nl(sheet: &DataSheet, nl: NonlinearFitSpec) -> ResultSheet {
    exec_nl_res(sheet, nl).unwrap()
}

pub fn exec_nl_hooks(sheet: &DataSheet, nl: NonlinearFitSpec, hooks: &HookContext<'_>) -> ResultSheet {
    let spec = AnalysisSpec::new("fit", sheet.id.clone(), AnalysisKind::NonlinearRegression(nl)).unwrap();
    execute_analysis(&spec, sheet, hooks).unwrap()
}

pub fn exec_lin_res(sheet: &DataSheet, lin: LinearRegressionSpec) -> Result<ResultSheet, FitError> {
    let spec = AnalysisSpec::new("lin", sheet.id.clone(), AnalysisKind::LinearRegression(lin)).unwrap();
    execute_analysis(&spec, sheet, &HookContext::default())
}

pub fn exec_lin(sheet: &DataSheet, lin: LinearRegressionSpec) -> ResultSheet {
    exec_lin_res(sheet, lin).unwrap()
}

pub fn find_param<'a>(res: &'a NonlinearFitResult, name: &str) -> &'a FittedParameterSummary {
    res.parameters.iter().find(|p| p.name.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("missing parameter `{name}` in {:?}", res.parameters))
}

pub fn find_param_ds<'a>(res: &'a NonlinearFitResult, ds_idx: usize, name: &str) -> &'a FittedParameterSummary {
    res.parameters
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name) && (p.dataset_index == Some(ds_idx) || p.dataset_index.is_none()))
        .unwrap_or_else(|| panic!("missing parameter `{name}` for ds {ds_idx} in {:?}", res.parameters))
}

pub fn find_param_exact_ds<'a>(res: &'a NonlinearFitResult, ds_idx: Option<usize>, name: &str) -> &'a FittedParameterSummary {
    res.parameters
        .iter()
        .find(|p| p.name == name && p.dataset_index == ds_idx)
        .unwrap_or_else(|| panic!("missing parameter `{name}` ({ds_idx:?}) in {:?}", res.parameters))
}

pub fn param_val(res: &NonlinearFitResult, name: &str) -> f64 {
    find_param(res, name).estimate
}

pub fn param_ds_val(res: &NonlinearFitResult, ds_idx: usize, name: &str) -> f64 {
    find_param_ds(res, ds_idx, name).estimate
}

pub fn dataset_param_val(res: &NonlinearFitResult, ds_idx: usize, name: &str) -> f64 {
    let idx = res.datasets[ds_idx]
        .param_names
        .iter()
        .position(|p| p.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("missing dataset param `{name}` on ds {ds_idx}"));
    res.datasets[ds_idx].params[idx]
}

pub fn find_derived(res: &NonlinearFitResult, ds_idx: usize, name: &str) -> f64 {
    res.derived_scalars[ds_idx]
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("missing derived scalar `{name}` on ds {ds_idx}"))
        .value
}

pub fn unwrap_finite_ci(ci: (CiBound, CiBound), ctx: &str) -> (f64, f64) {
    match ci {
        (CiBound::Finite(lo), CiBound::Finite(hi)) => (lo, hi),
        other => panic!("{ctx}: expected finite CI bounds, got {other:?}"),
    }
}

pub fn find_tab<'a>(rs: &'a ResultSheet, id: &str) -> &'a ResultTab {
    rs.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

pub fn tab_row<'a>(rs: &'a ResultSheet, tab_id: &str, label: &str) -> &'a [ResultCell] {
    let tab = find_tab(rs, tab_id);
    let idx = tab.row_labels.iter().position(|r| r == label).unwrap_or_else(|| panic!("missing row `{label}` in `{tab_id}`"));
    &tab.rows[idx]
}

pub fn cell_num(cell: &ResultCell) -> f64 {
    match *cell {
        ResultCell::Number { value, .. } => value,
        ref other => panic!("expected numeric cell, got {other:?}"),
    }
}

pub fn assert_num_cell(cell: &ResultCell, expected: f64, abs_tol: f64, rel_tol: f64, ctx: &str) {
    assert_approx_eq(cell_num(cell), expected, abs_tol, rel_tol, ctx);
}

pub fn assert_ci_cell(cell: &ResultCell, expected: (f64, f64), abs_tol: f64, rel_tol: f64, ctx: &str) {
    let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } = *cell else {
        panic!("{ctx}: expected finite ConfidenceInterval cell, got {cell:?}");
    };
    assert_approx_eq(lo, expected.0, abs_tol, rel_tol, &format!("{ctx} lower"));
    assert_approx_eq(hi, expected.1, abs_tol, rel_tol, &format!("{ctx} upper"));
}

pub fn assert_gof_num(rs: &ResultSheet, label: &str, expected: f64, abs_tol: f64, rel_tol: f64) {
    assert_num_cell(&tab_row(rs, "nonlinear_gof", label)[0], expected, abs_tol, rel_tol, label);
}

pub fn assert_converged(res: &NonlinearFitResult, ctx: &str) {
    assert_eq!(res.status, FitStatus::Converged, "{ctx}: expected FitStatus::Converged, got {:?}", res.status);
}

pub fn assert_params_approx(res: &NonlinearFitResult, ds_idx: usize, expected: &[(&str, f64, f64, f64)], ctx: &str) {
    for &(name, val, abs_tol, rel_tol) in expected {
        assert_approx_eq(param_ds_val(res, ds_idx, name), val, abs_tol, rel_tol, &format!("{ctx} {name}"));
    }
}

pub fn assert_grad_matches_fd(model: &dyn ParametricModel, x: &[f64], params: &[f64], ds: usize, rel_tol: f64) {
    let mut analytic = vec![0.0_f64; params.len()];
    assert!(model.grad(x, params, ds, &mut analytic), "{} grad returned false at x={x:?}, p={params:?}", model.id());
    let y0 = model.eval(x, params, ds).unwrap_or_else(|| panic!("{} eval None at x={x:?}, p={params:?}", model.id()));
    for (k, &ak) in analytic.iter().enumerate() {
        let h = if params[k] == 0.0 { 1e-6_f64 } else { 1e-6_f64 * params[k].abs() };
        let mut p_plus = params.to_vec();
        p_plus[k] += h;
        let mut p_minus = params.to_vec();
        p_minus[k] -= h;
        let fd = if params[k] == 0.0 && model.eval(x, &p_minus, ds).is_none() {
            (model.eval(x, &p_plus, ds).unwrap() - y0) / h
        } else {
            match (model.eval(x, &p_plus, ds), model.eval(x, &p_minus, ds)) {
                (Some(yp), Some(ym)) => (yp - ym) / (2.0 * h),
                (Some(yp), None) => (yp - y0) / h,
                (None, Some(ym)) => (y0 - ym) / h,
                (None, None) => panic!("{} FD eval failed both sides for param[{k}] at x={x:?}", model.id()),
            }
        };
        assert!((ak - fd).abs() <= rel_tol * fd.abs().max(1.0), "{} grad[{k}] mismatch at x={x:?}, p={params:?}: analytic={ak}, fd={fd}", model.id());
    }
}

pub fn dsl_compile(src: &str) -> CompiledDslModel {
    CompiledDslModel::compile("test_dsl", src).unwrap_or_else(|e| panic!("DSL compile failed for `{src}`: {e:?}"))
}

pub fn assert_dsl_compile_err(src: &str) {
    let err = CompiledDslModel::compile("bad_dsl", src);
    assert!(matches!(err, Err(FitError::DslCompileError { .. })), "expected DslCompileError for:\n{src}");
}

pub fn unit_cov_1d() -> CovarianceDiagnostics {
    CovarianceDiagnostics {
        standard_errors: vec![1.0],
        symmetric_ci_lower: vec![-1.96],
        symmetric_ci_upper: vec![1.96],
        covariance_matrix: vec![vec![1.0]],
        correlation_matrix: vec![vec![1.0]],
        dependencies: vec![0.0],
        numerical_rank: 1,
        ambiguous: false,
    }
}
