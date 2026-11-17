use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable,
    DataSheet, DocError, ErrorPropagationMode, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet,
    InternalLinkKind, LinkSource, MAX_ROWS, MAX_SUBCOLUMNS, MathTransformPreset, MultipleVariablesTable, NestedTable, NormalizeAnchor,
    PartsOfWholeTable, PlacementMode, SheetCellRange, SheetId, SimulationNoise, SubcolumnMode, SurvivalTable, TableArchetype, TableKind,
    TransformAnalysisSpec, VariableRole, XColumnMode, XyTable,
};
use curvebook_transform::import::MAX_IMPORT_CELLS;
use curvebook_transform::{
    AucOptions, BaselineKind, CurveSheetOp, DependencyTarget, DerivativeOrder, FormulaContext, FormulaValue, ImportPipelineOptions,
    ImportSourcePayload, MathTransformOp, PruneMethod, ScalarEnv, SelectRule, SmoothMethod, TransformContext, TransformError,
    apply_curve_calculus_to_sheet, apply_import_link_pipeline, compute_auc, compute_auc_with_se, compute_sheet_auc, detect_link_cycles,
    differentiate_xy, eval_column_expr, eval_scalar_expr, evaluate_calculated_variables, evaluate_data_sheet_calculated_variables, evaluate_equation,
    evaluate_formula_for_sheet, execute_analysis_transform, execute_math_transform, execute_prune, execute_remove_baseline_kind, execute_select,
    execute_transform, execute_transpose, extract_dependencies, integrate_xy, parse_formula, simulate_xy_sheet, smooth_xy, validate_no_formula_cycle,
};

fn assert_approx(actual: f64, expected: f64, tol: f64) {
    assert!((actual - expected).abs() <= tol, "expected {expected} ± {tol}, got {actual} (diff = {})", (actual - expected).abs());
}

#[allow(clippy::unwrap_used)]
fn make_xy_single(id: &str, xs: &[f64], ys: &[&[f64]]) -> DataSheet {
    let n_rows = xs.len();
    let n_cols = ys.len();
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, n_cols, n_rows).unwrap();
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
    }
    for (c, col_vals) in ys.iter().enumerate() {
        for (r, &y) in col_vals.iter().enumerate() {
            xy.data_sets[c].cells[r][0] = Cell::number(y);
        }
    }
    DataSheet::new(id, format!("Sheet {id}"), TableArchetype::Xy(xy)).unwrap()
}

#[allow(clippy::unwrap_used)]
fn make_col_single(id: &str, cols: &[(&str, &[Cell])]) -> DataSheet {
    let n_rows = cols.first().map_or(0, |(_, r)| r.len());
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len().max(1), n_rows).unwrap();
    for (c, (title, cells)) in cols.iter().enumerate() {
        tbl.data_sets[c].title = (*title).to_owned();
        for (r, cell) in cells.iter().enumerate() {
            tbl.data_sets[c].cells[r][0] = cell.clone();
        }
    }
    DataSheet::new(id, format!("Sheet {id}"), TableArchetype::Column(tbl)).unwrap()
}

// ============================================================================
// 1. Formula Parser, Evaluator, Functions, NA Propagation, and DAG
// ============================================================================

#[test]
fn formula_precedence_associativity_and_scalar_math() {
    let env = ScalarEnv::default();
    let cases = [
        ("=-2^2", -4.0),
        ("2^3^2", 512.0),
        ("1 + 2 * 3 - 4 / 2", 5.0),
        ("if(1 < 2 AND 3 >= 3, 10, 20)", 10.0),
        ("if(NOT (1 == 2 OR 2 <> 2), 42, 0)", 42.0),
        ("abs(-7) + sqrt(16) + cbrt(27)", 14.0),
        ("floor(2.9) + ceil(2.1) + round(2.6) + trunc(-2.9) + sign(-5)", 5.0),
        ("clamp(15, 0, 10) + min(3, 5, 2) + max(1, 9, 4)", 21.0),
        ("pow(2, 5) + exp(0) + ln(1) + log10(1000) + log2(8)", 39.0),
    ];
    for (src, expected) in cases {
        let ast = parse_formula(src).unwrap();
        let val = eval_scalar_expr(&ast, &env).unwrap().as_finite_f64().unwrap();
        assert_approx(val, expected, 1e-12);
    }

    // Special & trig functions
    let trig = eval_scalar_expr(&parse_formula("sin(0) + cos(0) + tan(0) + atan2(1, 1) * 4 / 3.141592653589793").unwrap(), &env)
        .unwrap()
        .as_finite_f64()
        .unwrap();
    assert_approx(trig, 2.0, 1e-12);

    let hyp = eval_scalar_expr(&parse_formula("sinh(0) + cosh(0) + tanh(0)").unwrap(), &env).unwrap().as_finite_f64().unwrap();
    assert_approx(hyp, 1.0, 1e-12);

    let inv_trig = eval_scalar_expr(&parse_formula("asin(0) + acos(1) + atan(0)").unwrap(), &env).unwrap().as_finite_f64().unwrap();
    assert_approx(inv_trig, 0.0, 1e-12);

    let special = eval_scalar_expr(&parse_formula("erf(0) + erfc(0) + lgamma(1) + logit(0.5)").unwrap(), &env).unwrap().as_finite_f64().unwrap();
    assert_approx(special, 1.0, 1e-12);

    // Bliss probit(0.5) == 5.0 while norm_inv(0.5) == 0.0
    let probit_half = eval_scalar_expr(&parse_formula("probit(0.5)").unwrap(), &env).unwrap().as_finite_f64().unwrap();
    assert_approx(probit_half, 5.0, 1e-12);
    let norm_inv_half = eval_scalar_expr(&parse_formula("norm_inv(0.5)").unwrap(), &env).unwrap().as_finite_f64().unwrap();
    assert_approx(norm_inv_half, 0.0, 1e-12);
}

#[test]
fn formula_aggregates_window_and_excluded_propagation() {
    let mut mv = MultipleVariablesTable::new(2, 5).unwrap();
    mv.variables[0].title = String::from("Dose");
    mv.variables[1].title = String::from("Group");
    let nums = [1.0, 2.0, 3.0, 4.0, 100.0];
    let groups = ["ctrl", "ctrl", "trt", "trt", "ctrl"];
    for i in 0..5 {
        mv.variables[0].cells[i] = Cell { value: CellValue::Number(nums[i]), excluded: i == 4 };
        mv.variables[1].cells[i] = Cell::text(groups[i]);
    }

    let mut info = InfoSheet::new_preseeded("info1", "Meta").unwrap();
    info.set_constant("Factor", ConstantValue::Number(10.0), None).unwrap();

    let sheet = DataSheet::new("mv1", "MV", TableArchetype::MultipleVariables(mv)).unwrap();

    // Excluded row 4 (value 100.0) must NOT pollute sum/mean/count/sd/geo_mean/iqr
    let sums = evaluate_formula_for_sheet("sum(Dose)", &sheet, Some(&info)).unwrap();
    assert_approx(sums[0].as_finite_f64().unwrap(), 10.0, 1e-12);

    let means = evaluate_formula_for_sheet("mean(Dose) * Info(\"Factor\")", &sheet, Some(&info)).unwrap();
    assert_approx(means[0].as_finite_f64().unwrap(), 25.0, 1e-12);

    let cnt = evaluate_formula_for_sheet("count(Dose)", &sheet, None).unwrap();
    assert_approx(cnt[0].as_finite_f64().unwrap(), 4.0, 1e-12);

    let p50 = evaluate_formula_for_sheet("percentile(Dose, 50) + median(Dose) + min_col(Dose) + max_col(Dose)", &sheet, None).unwrap();
    // median=2.5, min=1, max=4 => 2.5 + 2.5 + 1 + 4 = 10
    assert_approx(p50[0].as_finite_f64().unwrap(), 10.0, 1e-12);

    let gmean = evaluate_formula_for_sheet("geo_mean(Dose)", &sheet, None).unwrap();
    assert_approx(gmean[0].as_finite_f64().unwrap(), (24.0_f64).powf(0.25), 1e-12);

    let spreads = evaluate_formula_for_sheet("var(Dose) + sd(Dose) + sem(Dose) + iqr(Dose)", &sheet, None).unwrap();
    assert!(spreads[0].as_finite_f64().is_some());

    // Conditional aggregates
    let sumif = evaluate_formula_for_sheet("sumif(Group == \"trt\", Dose)", &sheet, None).unwrap();
    assert_approx(sumif[0].as_finite_f64().unwrap(), 7.0, 1e-12);
    let countif = evaluate_formula_for_sheet("countif(Group == \"ctrl\")", &sheet, None).unwrap();
    assert_approx(countif[0].as_finite_f64().unwrap(), 3.0, 1e-12);
    let meanif = evaluate_formula_for_sheet("meanif(Group == \"ctrl\", Dose)", &sheet, None).unwrap();
    assert_approx(meanif[0].as_finite_f64().unwrap(), 1.5, 1e-12);

    // Window / cumulative / lag / lead / rank / row variables & NA propagation
    let cum = evaluate_formula_for_sheet("cumsum(Dose)", &sheet, None).unwrap();
    assert_approx(cum[0].as_finite_f64().unwrap(), 1.0, 1e-12);
    assert_approx(cum[3].as_finite_f64().unwrap(), 10.0, 1e-12);
    assert_eq!(cum[4], FormulaValue::Missing);

    let cprod = evaluate_formula_for_sheet("cumprod(Dose)", &sheet, None).unwrap();
    assert_approx(cprod[3].as_finite_f64().unwrap(), 24.0, 1e-12);

    let diffs = evaluate_formula_for_sheet("diff(Dose)", &sheet, None).unwrap();
    assert_eq!(diffs[0], FormulaValue::Missing);
    assert_approx(diffs[1].as_finite_f64().unwrap(), 1.0, 1e-12);

    let lags = evaluate_formula_for_sheet("lag(Dose, 2)", &sheet, None).unwrap();
    assert_eq!(lags[0], FormulaValue::Missing);
    assert_eq!(lags[1], FormulaValue::Missing);
    assert_approx(lags[2].as_finite_f64().unwrap(), 1.0, 1e-12);

    let leads = evaluate_formula_for_sheet("lead(Dose, 1)", &sheet, None).unwrap();
    assert_approx(leads[0].as_finite_f64().unwrap(), 2.0, 1e-12);

    let rmean = evaluate_formula_for_sheet("rolling_mean(Dose, 2)", &sheet, None).unwrap();
    assert_approx(rmean[1].as_finite_f64().unwrap(), 1.5, 1e-12);

    let rsum = evaluate_formula_for_sheet("rolling_sum(Dose, 3)", &sheet, None).unwrap();
    assert_approx(rsum[2].as_finite_f64().unwrap(), 6.0, 1e-12);

    let z = evaluate_formula_for_sheet("zscore(Dose)", &sheet, None).unwrap();
    assert_approx(z[0].as_finite_f64().unwrap() + z[3].as_finite_f64().unwrap(), 0.0, 1e-12);

    let rnk = evaluate_formula_for_sheet("rank(Dose)", &sheet, None).unwrap();
    assert_approx(rnk[0].as_finite_f64().unwrap(), 1.0, 1e-12);
    assert_approx(rnk[3].as_finite_f64().unwrap(), 4.0, 1e-12);

    let row_fn = evaluate_formula_for_sheet("if(isvalid(Dose), $row + $index, coalesce(Dose, -99))", &sheet, None).unwrap();
    assert_approx(row_fn[0].as_finite_f64().unwrap(), 1.0, 1e-12); // 1 + 0
    assert_approx(row_fn[4].as_finite_f64().unwrap(), -99.0, 1e-12);

    let is_missing = evaluate_formula_for_sheet("isnan(Dose)", &sheet, None).unwrap();
    assert_approx(is_missing[4].as_finite_f64().unwrap(), 1.0, 1e-12);
}

#[test]
fn formula_error_diagnostics() {
    assert!(matches!(parse_formula("1 + * 2"), Err(TransformError::ParseError { .. })));
    assert!(matches!(parse_formula("sqrt(1, 2"), Err(TransformError::ParseError { .. })));

    let env = ScalarEnv::default();
    assert!(matches!(eval_scalar_expr(&parse_formula("unknown_fn(1)").unwrap(), &env), Err(TransformError::UnknownFunction { .. })));
    assert!(matches!(eval_scalar_expr(&parse_formula("sqrt(1, 2)").unwrap(), &env), Err(TransformError::ArityMismatch { .. })));
    assert!(matches!(eval_scalar_expr(&parse_formula("MissingVar + 1").unwrap(), &env), Err(TransformError::UnknownVariable { .. })));
}

#[test]
fn formula_dag_topological_order_and_cycle_detection() {
    // Calculated column 0 depends on calculated column 1, which depends on entered column 2 and Info constant
    let mut mv = MultipleVariablesTable::new(3, 3).unwrap();
    mv.variables[0].title = String::from("Final");
    mv.variables[0].set_calculated_formula("=Mid + [InfoConst]");
    mv.variables[1].title = String::from("Mid");
    mv.variables[1].set_calculated_formula("=col(\"Raw\") * 2");
    mv.variables[2].title = String::from("Raw");
    for r in 0..3 {
        mv.variables[2].cells[r] = Cell::number((r + 1) as f64);
    }

    let mut info = InfoSheet::new_preseeded("i1", "I").unwrap();
    info.set_constant("InfoConst", ConstantValue::Number(10.0), None).unwrap();

    let ast0 = parse_formula("=Mid + [InfoConst]").unwrap();
    let deps0 = extract_dependencies(&ast0);
    assert!(deps0.contains(&DependencyTarget::ColumnName(String::from("Mid"))));

    let order = evaluate_calculated_variables(&mut mv, Some(&info)).unwrap();
    assert_eq!(order, vec![1, 0]);
    assert_approx(mv.variables[0].cells[0].active_number().unwrap(), 12.0, 1e-12);
    assert_approx(mv.variables[0].cells[2].active_number().unwrap(), 16.0, 1e-12);

    // Test DataSheet wrapper as well
    let mut ds = DataSheet::new("mv", "MV", TableArchetype::MultipleVariables(mv.clone())).unwrap();
    let ds_order = evaluate_data_sheet_calculated_variables(&mut ds, Some(&info)).unwrap();
    assert_eq!(ds_order, vec![1, 0]);

    // Self-cycle detection
    let mut self_cyc = mv.clone();
    self_cyc.variables[0].set_calculated_formula("=Final + 1");
    assert!(matches!(evaluate_calculated_variables(&mut self_cyc, None), Err(TransformError::CircularDependency { .. })));

    // 3-node cycle detection: 0 -> 1 -> 2 -> 0
    let mut three_cyc = MultipleVariablesTable::new(3, 2).unwrap();
    three_cyc.variables[0].title = String::from("V0");
    three_cyc.variables[1].title = String::from("V1");
    three_cyc.variables[2].title = String::from("V2");
    three_cyc.variables[0].set_calculated_formula("=V1 + 1");
    three_cyc.variables[1].set_calculated_formula("=V2 + 1");
    three_cyc.variables[2].set_calculated_formula("=V0 + 1");
    let err = evaluate_calculated_variables(&mut three_cyc, None).unwrap_err();
    match err {
        TransformError::CircularDependency { cycle } => assert!(cycle.len() >= 3),
        other => panic!("expected CircularDependency, got {other:?}"),
    }
}

// ============================================================================
// 2. TransformAnalysisSpec Operations & Error Propagation
// ============================================================================

#[test]
fn math_transform_and_error_propagation_vs_finite_difference() {
    // MeanSdN table with Y = 10.0, SD = 0.5, N = 4, plus an excluded row
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSdN, 1, 2).unwrap();
    xy.set_x_cell(0, Cell::number(100.0)).unwrap();
    xy.set_x_cell(1, Cell::number(1000.0)).unwrap();
    xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(0.5), Cell::number(4.0)];
    xy.data_sets[0].cells[1] = vec![Cell { value: CellValue::Number(20.0), excluded: true }, Cell::number(1.0), Cell::number(5.0)];
    let sheet = DataSheet::new("xy_err", "Err", TableArchetype::Xy(xy)).unwrap();

    // 2a. FirstOrderDeltaMethod vs finite-difference derivative verification on UserExpression + Ln
    let ctx = TransformContext::default();
    let delta_out =
        execute_math_transform(&sheet, Some(&MathTransformOp::Log10), Some(&MathTransformOp::Ln), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx)
            .unwrap();

    let TableArchetype::Xy(res_xy) = &delta_out.data_sheet.archetype else {
        panic!("expected XY output");
    };
    assert_approx(res_xy.x_value_at(0).unwrap(), 2.0, 1e-12);
    let y0 = res_xy.data_sets[0].cells[0][0].active_number().unwrap();
    let sd0 = res_xy.data_sets[0].cells[0][1].active_number().unwrap();
    let n0 = res_xy.data_sets[0].cells[0][2].active_number().unwrap();
    assert_approx(y0, 10.0_f64.ln(), 1e-12);
    // Central finite difference check: |d/dy ln(y)| * 0.5
    let eps = 1e-6_f64;
    let fd_deriv = ((10.0_f64 + eps).ln() - (10.0_f64 - eps).ln()) / (2.0 * eps);
    assert_approx(sd0, fd_deriv.abs() * 0.5, 1e-8);
    assert_approx(n0, 4.0, 1e-12);
    // Exclusion preserved on row 1
    assert!(res_xy.data_sets[0].cells[1][0].excluded);

    // 2b. ReplicateEndpoints on Log10
    let ep_out = execute_transform(
        &sheet,
        &TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: Some(MathTransformPreset::Log10),
            error_propagation: ErrorPropagationMode::ReplicateEndpoints,
        },
        &ctx,
    )
    .unwrap();
    let TableArchetype::Xy(ep_xy) = &ep_out.data_sheet.archetype else {
        panic!("expected XY");
    };
    let expected_ep_sd = 0.5 * ((10.5_f64).log10() - (9.5_f64).log10());
    assert_approx(ep_xy.data_sets[0].cells[0][1].active_number().unwrap(), expected_ep_sd, 1e-12);

    // 2c. MeanLowHigh asymmetric intervals on XY + UserExpression & Constant ops
    let mut xy_lh =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanLowHigh, 1, 1).unwrap();
    xy_lh.set_x_cell(0, Cell::number(2.0)).unwrap();
    xy_lh.data_sets[0].cells[0] = vec![Cell::number(4.0), Cell::number(0.5), Cell::number(1.0)];
    let col_sheet = DataSheet::new("col_lh", "LH", TableArchetype::Xy(xy_lh)).unwrap();
    let user_out = execute_math_transform(
        &col_sheet,
        None,
        Some(&MathTransformOp::UserExpression(String::from("Y^2 + 1"))),
        ErrorPropagationMode::ReplicateEndpoints,
        &ctx,
    )
    .unwrap();
    let TableArchetype::Xy(u_col) = &user_out.data_sheet.archetype else {
        panic!("expected XY");
    };
    // f(4) = 17, f(3.5) = 13.25 -> low_err = 3.75, f(5) = 26 -> high_err = 9.0
    assert_approx(u_col.data_sets[0].cells[0][0].active_number().unwrap(), 17.0, 1e-10);
    assert_approx(u_col.data_sets[0].cells[0][1].active_number().unwrap(), 3.75, 1e-10);
    assert_approx(u_col.data_sets[0].cells[0][2].active_number().unwrap(), 9.0, 1e-10);

    // Constant ops via MathTransformOp
    let mul_out =
        execute_math_transform(&col_sheet, None, Some(&MathTransformOp::MultiplyByConstant(2.0)), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx)
            .unwrap();
    let TableArchetype::Xy(m_col) = &mul_out.data_sheet.archetype else {
        panic!("expected XY");
    };
    assert_approx(m_col.data_sets[0].cells[0][0].active_number().unwrap(), 8.0, 1e-12);
    assert_approx(m_col.data_sets[0].cells[0][1].active_number().unwrap(), 1.0, 1e-12);
}

#[test]
fn normalize_baseline_transpose_prune_and_select() {
    // Normalize with Hooked InfoConstant
    let mut info = InfoSheet::new_preseeded("info_norm", "Info").unwrap();
    info.set_constant("MaxRef", ConstantValue::Number(50.0), None).unwrap();
    let ctx = TransformContext::with_info(&info);

    let xy = make_xy_single("s1", &[1.0, 2.0, 3.0, 4.0], &[&[10.0, 20.0, 30.0, 50.0], &[0.0, 25.0, 50.0, 100.0]]);
    let norm_out = execute_transform(
        &xy,
        &TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info_norm")),
                constant_name: String::from("MaxRef"),
            }),
            output_percent: true,
            per_data_set: true,
        },
        &ctx,
    )
    .unwrap();
    let TableArchetype::Xy(n_xy) = &norm_out.data_sheet.archetype else {
        panic!("expected XY");
    };
    // Col 0: min=10, max=50 => 30 -> 50%, 50 -> 100%
    assert_approx(n_xy.data_sets[0].cells[0][0].active_number().unwrap(), 0.0, 1e-12);
    assert_approx(n_xy.data_sets[0].cells[2][0].active_number().unwrap(), 50.0, 1e-12);
    assert_approx(n_xy.data_sets[0].cells[3][0].active_number().unwrap(), 100.0, 1e-12);
    // Col 1: min=0, max=50 => 25 -> 50%, 100 -> 200%
    assert_approx(n_xy.data_sets[1].cells[1][0].active_number().unwrap(), 50.0, 1e-12);
    assert_approx(n_xy.data_sets[1].cells[3][0].active_number().unwrap(), 200.0, 1e-12);

    // RemoveBaseline (AnalysisSpec wrapper) + BaselineKind variants
    let analysis = AnalysisSpec::new(
        "an1",
        "s1",
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Difference }),
    )
    .unwrap();
    let res_sheet = execute_analysis_transform(&xy, &analysis, &ctx).unwrap();
    let TableArchetype::Xy(base_xy) = &res_sheet.analyzable_data.as_ref().unwrap().archetype else {
        panic!("expected XY");
    };
    // Col 1 - Col 0: 0-10 = -10, 25-20 = 5
    assert_approx(base_xy.data_sets[1].cells[0][0].active_number().unwrap(), -10.0, 1e-12);
    assert_approx(base_xy.data_sets[1].cells[1][0].active_number().unwrap(), 5.0, 1e-12);

    let lin_base =
        execute_remove_baseline_kind(&xy, &BaselineKind::FirstAndLastLinear { first_k: 1, last_k: 1 }, BaselineOperation::Difference, &ctx).unwrap();
    let TableArchetype::Xy(lb_xy) = &lin_base.data_sheet.archetype else {
        panic!("expected XY");
    };
    // Col 1 is linear 0 -> 100 so endpoints become 0.0
    assert_approx(lb_xy.data_sets[1].cells[0][0].active_number().unwrap(), 0.0, 1e-12);
    assert_approx(lb_xy.data_sets[1].cells[3][0].active_number().unwrap(), 0.0, 1e-12);

    // Transpose
    let col_sheet = make_col_single(
        "c1",
        &[("Ctrl", &[Cell::number(1.0), Cell { value: CellValue::Number(2.0), excluded: true }]), ("Drug", &[Cell::number(3.0), Cell::number(4.0)])],
    );
    let tr_out = execute_transform(&col_sheet, &TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true }, &ctx).unwrap();
    let TableArchetype::Column(tr_col) = &tr_out.data_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(tr_col.row_titles, vec!["Ctrl", "Drug"]);
    assert!(tr_col.data_sets[1].cells[0][0].excluded);
    assert_approx(tr_col.data_sets[1].cells[1][0].active_number().unwrap(), 4.0, 1e-12);

    // Prune: AverageBins, EveryNth, LogarithmicSpacing, MinMaxBins
    let prune_avg = execute_transform(&xy, &TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: true }, &ctx).unwrap();
    let TableArchetype::Xy(pa_xy) = &prune_avg.data_sheet.archetype else {
        panic!("expected XY");
    };
    assert_eq!(pa_xy.row_count(), 2);
    assert_approx(pa_xy.x_value_at(0).unwrap(), 1.5, 1e-12);
    assert_approx(pa_xy.data_sets[0].cells[0][0].active_number().unwrap(), 15.0, 1e-12);

    let prune_minmax = execute_prune(&xy, PruneMethod::MinMaxBins(4), &ctx).unwrap();
    assert_eq!(prune_minmax.data_sheet.archetype.row_count(), 2);

    let big_xy = make_xy_single("big", &(1..=100).map(|i| i as f64).collect::<Vec<_>>(), &[&(1..=100).map(|i| (i * 2) as f64).collect::<Vec<_>>()]);
    let log_pruned = execute_prune(&big_xy, PruneMethod::LogarithmicSpacing { target_points: 10 }, &ctx).unwrap();
    assert!(log_pruned.data_sheet.archetype.row_count() >= 5 && log_pruned.data_sheet.archetype.row_count() <= 10);

    // Select & Transform (predicate + projection to Column)
    let sel_out = execute_transform(
        &xy,
        &TransformAnalysisSpec::SelectAndTransform {
            filter_formula: Some(String::from("X >= 2 AND X <= 3")),
            target_archetype: TableKind::Column,
            derived_columns: Vec::new(),
        },
        &ctx,
    )
    .unwrap();
    assert_eq!(sel_out.data_sheet.kind(), TableKind::Column);
    assert_eq!(sel_out.data_sheet.archetype.row_count(), 2);

    let top1 = execute_select(&xy, &[SelectRule::TopN { column: 0, n: 1 }], TableKind::Xy, &ctx).unwrap();
    assert_eq!(top1.data_sheet.archetype.row_count(), 1);
    let TableArchetype::Xy(top_xy) = &top1.data_sheet.archetype else {
        panic!("expected XY");
    };
    assert_approx(top_xy.data_sets[0].cells[0][0].active_number().unwrap(), 50.0, 1e-12);
}

#[test]
fn simulate_deterministic_equations_and_noise() {
    let params =
        vec![(String::from("Bottom"), 10.0), (String::from("Top"), 100.0), (String::from("LogEC50"), -6.0), (String::from("HillSlope"), 1.0)];
    let y_mid = evaluate_equation("hill_4pl", -6.0, &params).unwrap();
    assert_approx(y_mid, 55.0, 1e-12);

    let y_mm = evaluate_equation("michaelis_menten", 2.0, &[(String::from("Vmax"), 100.0), (String::from("Km"), 2.0)]).unwrap();
    assert_approx(y_mm, 50.0, 1e-12);

    let y_custom = evaluate_equation("expr: A * X^2 + B", 3.0, &[(String::from("A"), 2.0), (String::from("B"), 5.0)]).unwrap();
    assert_approx(y_custom, 23.0, 1e-12);

    let s1 = simulate_xy_sheet(
        "sim1",
        "Sim",
        42,
        10,
        3,
        0.0,
        9.0,
        "exponential_decay",
        &[(String::from("y0"), 100.0), (String::from("k"), 0.2), (String::from("plateau"), 10.0)],
        SimulationNoise::Gaussian { sd: 1.5 },
    )
    .unwrap();
    let s2 = simulate_xy_sheet(
        "sim1",
        "Sim",
        42,
        10,
        3,
        0.0,
        9.0,
        "exponential_decay",
        &[(String::from("y0"), 100.0), (String::from("k"), 0.2), (String::from("plateau"), 10.0)],
        SimulationNoise::Gaussian { sd: 1.5 },
    )
    .unwrap();
    assert_eq!(s1, s2);

    let ctx = TransformContext::default();
    let spec = TransformAnalysisSpec::Simulate {
        seed: 7,
        row_count: 5,
        replicates: 2,
        x_min: 1.0,
        x_max: 5.0,
        equation_id: String::from("linear"),
        parameters: vec![(String::from("slope"), 2.0), (String::from("intercept"), 1.0)],
        noise: SimulationNoise::Poisson,
    };
    let sim_out = execute_transform(&s1, &spec, &ctx).unwrap();
    assert_eq!(sim_out.data_sheet.archetype.row_count(), 5);
}

// ============================================================================
// 3. Curve Calculus: AUC, Peaks, SG/MA/LOWESS Smoothing, Derivatives, Integral
// ============================================================================

#[test]
fn curve_auc_peaks_se_sg_derivatives_and_integral() {
    // 3a. Exact trapezoidal AUC on linear curve + crossing baseline + peak
    // Points: (0, -2), (2, 2), (4, 0), (5, -1). Baseline = 0.
    // Crossing at x=1 -> negative triangle [0,1] area = 0.5*1*2 = 1.0
    // Peak [1,4]: triangle (1,0)->(2,2)->(4,0) area = 0.5*3*2 = 3.0
    // Negative [4,5]: area = 0.5*1*1 = 0.5
    let auc = compute_auc(&[(0.0, -2.0), (2.0, 2.0), (4.0, 0.0), (5.0, -1.0)], &AucOptions::default()).unwrap();
    assert_approx(auc.positive_area, 3.0, 1e-12);
    assert_approx(auc.negative_area, 1.5, 1e-12);
    assert_approx(auc.net_area, 1.5, 1e-12);
    assert_eq!(auc.peaks.len(), 1);
    assert_approx(auc.peaks[0].peak_x, 2.0, 1e-12);
    assert_approx(auc.peaks[0].area, 3.0, 1e-12);

    // 3b. SE propagation on uniform grid [0, 1, 2], SE = [0.1, 0.2, 0.1]
    // Weights: w0 = 0.5, w1 = 1.0, w2 = 0.5 -> var = (0.5*0.1)^2 + (1*0.2)^2 + (0.5*0.1)^2 = 0.0025 + 0.04 + 0.0025 = 0.045
    let auc_se = compute_auc_with_se(&[(0.0, 1.0, 0.1), (1.0, 2.0, 0.2), (2.0, 1.0, 0.1)], &AucOptions::default()).unwrap();
    assert_approx(auc_se.standard_error.unwrap(), (0.045_f64).sqrt(), 1e-12);

    // Sheet AUC on MeanSemN
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSemN, 1, 3).unwrap();
    for r in 0..3 {
        xy.set_x_cell(r, Cell::number(r as f64)).unwrap();
        xy.data_sets[0].cells[r] = vec![Cell::number((r + 1) as f64), Cell::number(0.25), Cell::number(4.0)];
    }
    let xy_sheet = DataSheet::new("xy_auc", "AUC", TableArchetype::Xy(xy)).unwrap();
    let sheet_auc = compute_sheet_auc(&xy_sheet, 0, &AucOptions::default()).unwrap();
    assert!(sheet_auc.standard_error.is_some());
    assert_approx(sheet_auc.positive_area, 4.0, 1e-12);

    // 3c. Savitzky-Golay 5/7/9 exactness on cubic polynomials (including boundary points)
    let cubic: Vec<(f64, f64)> = (0..15)
        .map(|i| {
            let x = i as f64;
            (x, 0.5 * x.powi(3) - 2.0 * x.powi(2) + 3.0 * x - 1.0)
        })
        .collect();
    for method in [SmoothMethod::SavitzkyGolay5, SmoothMethod::SavitzkyGolay7, SmoothMethod::SavitzkyGolay9] {
        let smoothed = smooth_xy(&cubic, method).unwrap();
        assert_eq!(smoothed.len(), cubic.len());
        for (&(x0, y0), &(xs, ys)) in cubic.iter().zip(smoothed.iter()) {
            assert_approx(x0, xs, 1e-12);
            assert_approx(ys, y0, 1e-9);
        }
    }

    // Moving average & LOWESS preserve length and reduce spike noise
    let noisy = vec![(0.0, 0.0), (1.0, 1.0), (2.0, 8.0), (3.0, 3.0), (4.0, 4.0), (5.0, 5.0)];
    let ma = smooth_xy(&noisy, SmoothMethod::MovingAverage { window: 3 }).unwrap();
    assert!(ma[2].1 < 8.0);
    let low = smooth_xy(&noisy, SmoothMethod::Lowess { span: 0.6, robustness_iters: 2 }).unwrap();
    assert!(low[2].1 < 8.0);

    // 3d. Exact 3-pt Lagrange derivatives on quadratic f(x) = 3x^2 - 4x + 5 on non-uniform grid
    let quad: Vec<(f64, f64)> = [0.0, 0.5, 1.5, 3.0, 4.2].into_iter().map(|x| (x, 3.0 * x * x - 4.0 * x + 5.0)).collect();
    let d1 = differentiate_xy(&quad, DerivativeOrder::First).unwrap();
    let d2 = differentiate_xy(&quad, DerivativeOrder::Second).unwrap();
    for &(x, dy) in &d1 {
        assert_approx(dy, 6.0 * x - 4.0, 1e-11);
    }
    for &(_, d2y) in &d2 {
        assert_approx(d2y, 6.0, 1e-11);
    }

    // 3e. Cumulative integral & DataSheet wrapper
    let integ = integrate_xy(&[(0.0, 2.0), (2.0, 6.0), (4.0, 6.0)], 0.0).unwrap();
    assert_approx(integ[0].1, 0.0, 1e-12);
    assert_approx(integ[1].1, 8.0, 1e-12);
    assert_approx(integ[2].1, 20.0, 1e-12);

    let xy_simple = make_xy_single("curve_s", &[0.0, 1.0, 2.0, 3.0], &[&[0.0, 1.0, 4.0, 9.0]]);
    let deriv_sheet = apply_curve_calculus_to_sheet(&xy_simple, CurveSheetOp::Differentiate(DerivativeOrder::Second)).unwrap();
    let TableArchetype::Xy(d_xy) = &deriv_sheet.archetype else {
        panic!("expected XY");
    };
    assert_approx(d_xy.data_sets[0].cells[1][0].active_number().unwrap(), 2.0, 1e-11);
}

// ============================================================================
// 4. 4-Stage ImportLinkSpec Pipeline & Link Cycle Detection
// ============================================================================

#[test]
fn import_link_pipeline_four_stages_and_cycle_detection() {
    let mut target = make_col_single("dst", &[("A", &[Cell::empty()]), ("B", &[Cell::empty()])]);
    let mut info = InfoSheet::new_preseeded("info_imp", "Import Meta").unwrap();

    // Delimited text with European comma decimals, asterisk exclusion, header row, unstacking
    let csv = "Group;Value\nG1;1,5\nG1;*2,5\nG2;3,5\nG2;4,5";
    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("plate.csv"), delimiter: ';', decimal_separator: ',' },
        filter: ImportFilterSpec {
            row_window: Some((1, 4)),
            col_window: None,
            decimate_every_k: None,
            unstack_group_column: Some(0),
            parse_asterisk_exclusions: true,
        },
        placement: ImportPlacementSpec {
            mode: PlacementMode::Overwrite,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: true,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    let opts = ImportPipelineOptions {
        link_id: Some(String::from("lnk1")),
        provenance_summary: Some(String::from("Plate CSV import")),
        ..ImportPipelineOptions::default()
    };

    let report = apply_import_link_pipeline(
        &mut target,
        &spec,
        ImportSourcePayload::DelimitedText { text: csv, delimiter: ';', decimal_separator: ',' },
        &opts,
        Some(&mut info),
    )
    .unwrap();

    assert_eq!(report.cols_placed, 2);
    assert_eq!(report.rows_placed, 2);
    assert_eq!(target.linked_regions.len(), 1);
    assert!(info.notes_markdown.contains("Plate CSV import"));

    let TableArchetype::Column(col) = &target.archetype else {
        panic!("expected Column");
    };
    assert_eq!(col.data_sets[0].title, "G1");
    assert_eq!(col.data_sets[1].title, "G2");
    assert_approx(col.data_sets[0].cells[0][0].active_number().unwrap(), 1.5, 1e-12);
    assert!(col.data_sets[0].cells[1][0].excluded);

    // Frozen target guard
    target.frozen = true;
    let frozen_err = apply_import_link_pipeline(
        &mut target,
        &spec,
        ImportSourcePayload::DelimitedText { text: csv, delimiter: ';', decimal_separator: ',' },
        &opts,
        None,
    );
    assert!(matches!(frozen_err, Err(TransformError::Doc(DocError::SheetFrozen { .. }))));

    // Link cycle detection across multiple sheets
    let s1 = make_col_single("s1", &[("A", &[Cell::number(1.0)])]);
    let mut s2 = make_col_single("s2", &[("A", &[Cell::empty()])]);
    let mut s3 = make_col_single("s3", &[("A", &[Cell::empty()])]);
    let mut link_from_1 = spec.clone();
    link_from_1.source = LinkSource::InternalSheet {
        sheet_id: SheetId::new("s1"),
        kind: InternalLinkKind::DataSheet,
        tab_id: None,
        range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
    };
    let mut link_from_2 = link_from_1.clone();
    if let LinkSource::InternalSheet { ref mut sheet_id, .. } = link_from_2.source {
        *sheet_id = SheetId::new("s2");
    }
    s2.linked_regions.push(curvebook_doc::LinkedDataRegion {
        id: String::from("l1"),
        spec: link_from_1.clone(),
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
        provenance_summary: String::from("s1->s2"),
    });
    s3.linked_regions.push(curvebook_doc::LinkedDataRegion {
        id: String::from("l2"),
        spec: link_from_2,
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
        provenance_summary: String::from("s2->s3"),
    });

    let order = detect_link_cycles(&[s3.clone(), s1.clone(), s2.clone()]).unwrap();
    assert_eq!(order, vec![SheetId::new("s1"), SheetId::new("s2"), SheetId::new("s3")]);

    // Introduce s3 -> s1 cycle
    let mut s1_cyclic = s1;
    let mut link_from_3 = link_from_1;
    if let LinkSource::InternalSheet { ref mut sheet_id, .. } = link_from_3.source {
        *sheet_id = SheetId::new("s3");
    }
    s1_cyclic.linked_regions.push(curvebook_doc::LinkedDataRegion {
        id: String::from("l3"),
        spec: link_from_3,
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
        provenance_summary: String::from("s3->s1"),
    });
    assert!(matches!(detect_link_cycles(&[s1_cyclic, s2, s3]), Err(TransformError::CircularDependency { .. })));

    // Unused helper import check
    let _ = BTreeMap::<String, FormulaValue>::new();
}

#[allow(clippy::unwrap_used)]
fn build_wf29_wf31_master_sheet() -> DataSheet {
    let mut mv = MultipleVariablesTable::new(8, 5).unwrap();
    let specs = [
        ("SampleID", VariableRole::Label),
        ("Group", VariableRole::Categorical),
        ("RefGroup", VariableRole::Categorical),
        ("Dose", VariableRole::Continuous),
        ("Baseline", VariableRole::Continuous),
        ("Response", VariableRole::Continuous),
        ("CountYes", VariableRole::Continuous),
        ("CountNo", VariableRole::Continuous),
    ];
    for (idx, (title, role)) in specs.iter().enumerate() {
        mv.variables[idx].title = (*title).to_owned();
        mv.variables[idx].role = *role;
        mv.variables[idx].role_inferred = false;
    }
    mv.row_titles = vec!["R1", "R2", "R3", "R4", "R5"].into_iter().map(str::to_owned).collect();
    mv.row_titles_expanded = true;

    let rows = [
        ("S1", "Ctrl", "Ctrl", 0.0, 10.0, 12.0, 5.0, 15.0),
        ("S2", "Drug", "Drug", 10.0, 20.0, 18.0, 8.0, 12.0),
        ("S3", "Drug", "Drug", 25.0, 20.0, 35.0, 14.0, 6.0),
        ("S4", "Drug", "Alt", 50.0, 30.0, 52.0, 18.0, 2.0),
        ("S5", "Drug", "Drug", 0.0, 15.0, 40.0, 9.0, 11.0),
    ];
    for (r, (sid, grp, rgrp, dose, base, resp, cy, cn)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(sid);
        mv.variables[1].cells[r] = Cell::text(grp);
        mv.variables[2].cells[r] = Cell::text(rgrp);
        mv.variables[3].cells[r] = Cell::number(dose);
        mv.variables[4].cells[r] = Cell::number(base);
        mv.variables[5].cells[r] = Cell::number(resp);
        mv.variables[6].cells[r] = Cell::number(cy);
        mv.variables[7].cells[r] = Cell::number(cn);
    }
    DataSheet::new("master", "Master MV", TableArchetype::MultipleVariables(mv)).unwrap()
}

#[test]
fn select_and_transform_selected_columns_into_xy_column_and_grouped() {
    let master = build_wf29_wf31_master_sheet();

    let ctx_xy = TransformContext { selected_data_sets: vec![3, 5], ..Default::default() };
    let xy_out = execute_transform(
        &master,
        &TransformAnalysisSpec::SelectAndTransform {
            filter_formula: Some(String::from("Dose > 0 && Response > Baseline")),
            target_archetype: TableKind::Xy,
            derived_columns: Vec::new(),
        },
        &ctx_xy,
    )
    .unwrap();
    let TableArchetype::Xy(xy) = &xy_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xy.x_title, "Dose");
    assert_eq!(xy.data_sets.len(), 1);
    assert_eq!(xy.data_sets[0].title, "Response");
    assert_eq!(xy.row_count(), 2);
    assert_eq!(xy.row_titles, vec!["R3", "R4"]);
    assert_approx(xy.x_value_at(0).unwrap(), 25.0, 1e-12);
    assert_approx(xy.data_sets[0].cells[0][0].active_number().unwrap(), 35.0, 1e-12);

    let ctx_cols = TransformContext { selected_data_sets: vec![4, 5], ..Default::default() };
    let rule_cat = SelectRule::Predicate(String::from("Group == \"Drug\" AND Group == RefGroup AND Dose > 0"));
    let col_out = execute_select(&master, std::slice::from_ref(&rule_cat), TableKind::Column, &ctx_cols).unwrap();
    let TableArchetype::Column(col_tbl) = &col_out.data_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(col_tbl.data_sets.len(), 2);
    assert_eq!(col_tbl.data_sets[0].title, "Baseline");
    assert_eq!(col_tbl.data_sets[1].title, "Response");
    assert_eq!(col_tbl.row_titles, vec!["R2", "R3"]);

    let grp_out = execute_select(&master, std::slice::from_ref(&rule_cat), TableKind::Grouped, &ctx_cols).unwrap();
    let TableArchetype::Grouped(grp_tbl) = &grp_out.data_sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(grp_tbl.data_sets.len(), 2);
    assert_eq!(grp_tbl.data_sets[0].title, "Baseline");
    assert_eq!(grp_tbl.row_titles, vec!["R2", "R3"]);
}

#[test]
fn select_and_transform_contingency_mv_subset_and_bounds() {
    let master = build_wf29_wf31_master_sheet();

    let ctx_cont = TransformContext { selected_data_sets: vec![6, 7], ..Default::default() };
    let cont_out = execute_select(&master, &[SelectRule::RowRange(2, 3)], TableKind::Contingency, &ctx_cont).unwrap();
    let TableArchetype::Contingency(cont_tbl) = &cont_out.data_sheet.archetype else {
        panic!("expected Contingency");
    };
    assert_eq!(cont_tbl.data_sets.len(), 2);
    assert_eq!(cont_tbl.data_sets[0].title, "CountYes");
    assert_eq!(cont_tbl.data_sets[1].title, "CountNo");
    assert_eq!(cont_tbl.row_titles, vec!["R3", "R4"]);
    assert_eq!(cont_tbl.count_at(0, 0).unwrap(), Some(14));
    assert_eq!(cont_tbl.count_at(1, 1).unwrap(), Some(2));

    let ctx_bad_cont = TransformContext { selected_data_sets: vec![0, 5], ..Default::default() };
    let cont_err = execute_select(&master, &[], TableKind::Contingency, &ctx_bad_cont).unwrap_err();
    assert!(matches!(cont_err, TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. })));

    let ctx_mv = TransformContext { selected_data_sets: vec![1, 5], ..Default::default() };
    let mv_sub = execute_select(&master, &[SelectRule::Predicate(String::from("Group != \"Ctrl\""))], TableKind::MultipleVariables, &ctx_mv).unwrap();
    let TableArchetype::MultipleVariables(sub) = &mv_sub.data_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(sub.variables.len(), 2);
    assert_eq!(sub.variables[0].title, "Group");
    assert_eq!(sub.variables[0].role, VariableRole::Categorical);
    assert_eq!(sub.variables[1].title, "Response");
    assert_eq!(sub.row_count(), 4);

    let all_cols = execute_select(&master, &[], TableKind::Column, &TransformContext::default()).unwrap();
    assert_eq!(all_cols.data_sheet.archetype.column_count(), 8);

    let ctx_oob = TransformContext { selected_data_sets: vec![99], ..Default::default() };
    assert!(matches!(execute_select(&master, &[], TableKind::Column, &ctx_oob), Err(TransformError::InvalidSpec { .. })));
}

#[test]
fn validate_no_formula_cycle_guards_and_errors() {
    let mut mv = MultipleVariablesTable::new(4, 3).unwrap();
    for (i, name) in ["ColA", "ColB", "ColC", "Raw"].iter().enumerate() {
        mv.variables[i].title = (*name).to_owned();
    }
    for r in 0..3 {
        mv.variables[3].cells[r] = Cell::number((r + 1) as f64 * 10.0);
    }
    mv.variables[1].set_calculated_formula("=ColA + 1");
    mv.variables[2].set_calculated_formula("=ColB * 2");

    validate_no_formula_cycle(&mv, 0, "=Raw / 2").unwrap();
    assert!(matches!(mv.variables[0].source, curvebook_doc::VariableSource::Entered));

    assert!(matches!(validate_no_formula_cycle(&mv, 0, "=ColA + 1"), Err(TransformError::CircularDependency { .. })));
    assert!(matches!(validate_no_formula_cycle(&mv, 0, "=A * 2"), Err(TransformError::CircularDependency { .. })));
    assert!(matches!(validate_no_formula_cycle(&mv, 0, "=col(0) + Raw"), Err(TransformError::CircularDependency { .. })));

    let err2 = validate_no_formula_cycle(&mv, 0, "=ColB + 5").unwrap_err();
    match err2 {
        TransformError::CircularDependency { cycle } => {
            assert!(cycle.contains(&String::from("ColA")) && cycle.contains(&String::from("ColB")));
        }
        other => panic!("expected 2-hop CircularDependency, got {other:?}"),
    }

    let err3 = validate_no_formula_cycle(&mv, 0, "=ColC - 1").unwrap_err();
    assert!(matches!(err3, TransformError::CircularDependency { .. }));

    assert!(matches!(validate_no_formula_cycle(&mv, 9, "=1"), Err(TransformError::InvalidSpec { .. })));
    assert!(matches!(validate_no_formula_cycle(&mv, 0, "=1 + * 2"), Err(TransformError::ParseError { .. })));
}

#[test]
fn rowwise_round_sum_mean_if_log10_and_qpcr_ddct_chain() {
    let env = ScalarEnv::default();
    assert_approx(eval_scalar_expr(&parse_formula("ROUND(2.71828, 2)").unwrap(), &env).unwrap().as_finite_f64().unwrap(), 2.72, 1e-12);
    assert_approx(eval_scalar_expr(&parse_formula("ROUND(1234, -2)").unwrap(), &env).unwrap().as_finite_f64().unwrap(), 1200.0, 1e-12);
    assert_approx(eval_scalar_expr(&parse_formula("ROUND(2.6)").unwrap(), &env).unwrap().as_finite_f64().unwrap(), 3.0, 1e-12);
    assert_approx(eval_scalar_expr(&parse_formula("SUM(1, 2, 3.5)").unwrap(), &env).unwrap().as_finite_f64().unwrap(), 6.5, 1e-12);
    assert_approx(eval_scalar_expr(&parse_formula("MEAN(2, 4, 9)").unwrap(), &env).unwrap().as_finite_f64().unwrap(), 5.0, 1e-12);

    let mut qpcr = MultipleVariablesTable::new(14, 4).unwrap();
    let titles = [
        "Group",
        "Ct_T1",
        "Ct_T2",
        "Ct_T3",
        "Ct_R1",
        "Ct_R2",
        "Ct_R3",
        "TargetMean",
        "RefMean",
        "DeltaCt",
        "CalDeltaCt",
        "ddCt",
        "FoldChange",
        "NotesAndMetrics",
    ];
    for (i, t) in titles.iter().enumerate() {
        qpcr.variables[i].title = (*t).to_owned();
    }
    qpcr.variables[0].role = VariableRole::Categorical;
    let sample_rows = [
        ("Control", [21.9, 22.1, 22.0], [18.0, 18.0, 99.0], true),
        ("Control", [22.0, 22.0, 22.0], [17.8, 18.2, 18.0], false),
        ("Treated", [19.8, 20.2, 20.0], [18.0, 18.0, 18.0], false),
        ("Treated", [20.0, 20.0, 20.0], [17.9, 18.1, 18.0], false),
    ];
    for (r, (grp, ts, rs, excl_r3)) in sample_rows.into_iter().enumerate() {
        qpcr.variables[0].cells[r] = Cell::text(grp);
        qpcr.variables[1].cells[r] = Cell::number(ts[0]);
        qpcr.variables[2].cells[r] = Cell::number(ts[1]);
        qpcr.variables[3].cells[r] = Cell::number(ts[2]);
        qpcr.variables[4].cells[r] = Cell::number(rs[0]);
        qpcr.variables[5].cells[r] = Cell::number(rs[1]);
        qpcr.variables[6].cells[r] = Cell { value: CellValue::Number(rs[2]), excluded: excl_r3 };
    }
    qpcr.variables[12].set_calculated_formula("=ROUND(2 ^ (-ddCt), 4)");
    qpcr.variables[11].set_calculated_formula("=DeltaCt - CalDeltaCt");
    qpcr.variables[10].set_calculated_formula("=MEANIF(Group == \"Control\", DeltaCt)");
    qpcr.variables[9].set_calculated_formula("=TargetMean - RefMean");
    qpcr.variables[8].set_calculated_formula("=MEAN(Ct_R1, Ct_R2, Ct_R3)");
    qpcr.variables[7].set_calculated_formula("=MEAN(Ct_T1, Ct_T2, Ct_T3)");
    qpcr.variables[13].set_calculated_formula("=IF(FoldChange > 2, ROUND(LOG10(SUM(Ct_T1, Ct_T2) / RefMean), 3), 0)");

    for col_idx in 7..=13 {
        if let curvebook_doc::VariableSource::Calculated { formula } = &qpcr.variables[col_idx].source {
            validate_no_formula_cycle(&qpcr, col_idx, formula).unwrap();
        }
    }

    let order = evaluate_calculated_variables(&mut qpcr, None).unwrap();
    assert_eq!(order.len(), 7);
    assert_approx(qpcr.variables[8].cells[0].active_number().unwrap(), 18.0, 1e-12);
    assert_approx(qpcr.variables[9].cells[0].active_number().unwrap(), 4.0, 1e-12);
    assert_approx(qpcr.variables[12].cells[0].active_number().unwrap(), 1.0, 1e-12);
    assert_approx(qpcr.variables[12].cells[2].active_number().unwrap(), 4.0, 1e-12);
    assert_approx(qpcr.variables[13].cells[0].active_number().unwrap(), 0.0, 1e-12);
    assert_approx(qpcr.variables[13].cells[2].active_number().unwrap(), 0.347, 1e-12);
}

#[test]
fn append_columns_grows_multiple_variables_and_column_tables() {
    // 1. MultipleVariables: 3 existing columns x 4 rows, append a 4th Cluster column at index 3.
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    for c in 0..3 {
        for r in 0..4 {
            mv.variables[c].cells[r] = Cell::number((c * 10 + r + 1) as f64);
        }
    }
    let mut mv_sheet = DataSheet::new("mv-append", "MV Append", TableArchetype::MultipleVariables(mv)).unwrap();
    let cluster_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::AppendColumns,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    let cluster_tsv = "Cluster 1\nCluster 2\nCluster 1\nCluster 3";
    let r1 = apply_import_link_pipeline(
        &mut mv_sheet,
        &cluster_spec,
        ImportSourcePayload::DelimitedText { text: cluster_tsv, delimiter: '\t', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(r1.rows_placed, 4);
    assert_eq!(r1.cols_placed, 1);
    assert_eq!(r1.target_range, SheetCellRange::new(CellCoord::new(0, 3, 0), CellCoord::new(3, 3, 0)));

    let TableArchetype::MultipleVariables(mv_after) = &mv_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv_after.variables.len(), 4);
    assert_eq!(mv_after.variables[3].title, "D");
    assert_eq!(mv_after.variables[3].role, VariableRole::Categorical);
    let actual_clusters: Vec<Option<&str>> = mv_after.variables[3].cells.iter().map(Cell::active_text).collect();
    assert_eq!(actual_clusters, vec![Some("Cluster 1"), Some("Cluster 2"), Some("Cluster 1"), Some("Cluster 3")]);
    assert_approx(mv_after.variables[0].cells[0].active_number().unwrap(), 1.0, 1e-12);

    // Append 2 shorter numeric columns (2 rows on a 4-row sheet) -> new columns still match row_count=4.
    let r2 = apply_import_link_pipeline(
        &mut mv_sheet,
        &cluster_spec,
        ImportSourcePayload::DelimitedText { text: "1.5\t2.5\n3.5\t4.5", delimiter: '\t', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(r2.target_range, SheetCellRange::new(CellCoord::new(0, 4, 0), CellCoord::new(1, 5, 0)));

    let TableArchetype::MultipleVariables(mv_after2) = &mv_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv_after2.variables.len(), 6);
    assert_eq!(mv_after2.variables[4].cells.len(), 4);
    assert_eq!(mv_after2.variables[5].cells.len(), 4);
    assert_eq!(mv_after2.variables[4].role, VariableRole::Continuous);
    assert_approx(mv_after2.variables[4].cells[0].active_number().unwrap(), 1.5, 1e-12);
    assert_approx(mv_after2.variables[5].cells[1].active_number().unwrap(), 4.5, 1e-12);
    assert!(mv_after2.variables[4].cells[2].is_empty());

    // Append with unstacking + propagate_title_header -> titles land on appended cols 6 and 7.
    let unstack_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec { unstack_group_column: Some(0), ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec { mode: PlacementMode::AppendColumns, propagate_title_header: true, ..ImportPlacementSpec::default() },
    };
    let r3 = apply_import_link_pipeline(
        &mut mv_sheet,
        &unstack_spec,
        ImportSourcePayload::DelimitedText { text: "G1\t10\nG2\t20", delimiter: '\t', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(r3.target_range, SheetCellRange::new(CellCoord::new(0, 6, 0), CellCoord::new(0, 7, 0)));
    let TableArchetype::MultipleVariables(mv_after3) = &mv_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv_after3.variables[6].title, "G1");
    assert_eq!(mv_after3.variables[7].title, "G2");
    assert_approx(mv_after3.variables[6].cells[0].active_number().unwrap(), 10.0, 1e-12);
    assert_approx(mv_after3.variables[7].cells[0].active_number().unwrap(), 20.0, 1e-12);

    // 2. Column (Single): 2 existing cols x 3 rows, append 2 cols x 4 rows -> grows both cols and rows.
    let mut col_sheet = make_col_single(
        "col-append",
        &[("Ctrl", &[Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)]), ("Drug", &[Cell::empty(), Cell::empty(), Cell::empty()])],
    );
    let col_rep = apply_import_link_pipeline(
        &mut col_sheet,
        &cluster_spec,
        ImportSourcePayload::DelimitedText { text: "10\t20\n30\t40\n50\t60\n70\t80", delimiter: '\t', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(col_rep.target_range, SheetCellRange::new(CellCoord::new(0, 2, 0), CellCoord::new(3, 3, 0)));
    let TableArchetype::Column(col_after) = &col_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(col_after.data_sets.len(), 4);
    assert_eq!(col_after.row_count(), 4);
    assert_eq!(col_after.data_sets[2].title, "C");
    assert_eq!(col_after.data_sets[3].title, "D");
    assert_eq!(col_after.data_sets[0].cells.len(), 4);
    assert_approx(col_after.data_sets[2].cells[3][0].active_number().unwrap(), 70.0, 1e-12);
    assert_approx(col_after.data_sets[3].cells[1][0].active_number().unwrap(), 40.0, 1e-12);

    // 3. Column (MeanSdN, 3 subcolumns): 2 existing cols x 2 rows, append 4-cell-wide grid -> adds cols 2 & 3.
    let rep_tbl = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 2, 2).unwrap();
    let mut rep_sheet = DataSheet::new("col-rep-append", "Col MeanSdN", TableArchetype::Column(rep_tbl)).unwrap();
    let rep_res = apply_import_link_pipeline(
        &mut rep_sheet,
        &cluster_spec,
        ImportSourcePayload::DelimitedText { text: "1\t2\t3\t4\n5\t6\t7\t8", delimiter: '\t', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(rep_res.target_range, SheetCellRange::new(CellCoord::new(0, 2, 0), CellCoord::new(1, 3, 2)));
    let TableArchetype::Column(rep_after) = &rep_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(rep_after.data_sets.len(), 4);
    assert_approx(rep_after.data_sets[2].cells[0][0].active_number().unwrap(), 1.0, 1e-12);
    assert_approx(rep_after.data_sets[2].cells[0][1].active_number().unwrap(), 2.0, 1e-12);
    assert_approx(rep_after.data_sets[2].cells[0][2].active_number().unwrap(), 3.0, 1e-12);
    assert_approx(rep_after.data_sets[3].cells[0][0].active_number().unwrap(), 4.0, 1e-12);
    assert_approx(rep_after.data_sets[3].cells[1][0].active_number().unwrap(), 8.0, 1e-12);
}

#[test]
fn append_columns_grows_all_remaining_archetypes() {
    let spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::AppendColumns, ..ImportPlacementSpec::default() },
    };
    let payload = ImportSourcePayload::DelimitedText { text: "11\t22\n33\t44", delimiter: '\t', decimal_separator: '.' };

    let mut sheets = [
        DataSheet::new("xy-app", "XY", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).unwrap())).unwrap(),
        DataSheet::new("grp-app", "Grouped", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap())).unwrap(),
        DataSheet::new("cnt-app", "Contingency", TableArchetype::Contingency(ContingencyTable::new(1, 2).unwrap())).unwrap(),
        DataSheet::new("srv-app", "Survival", TableArchetype::Survival(SurvivalTable::new(1, 2).unwrap())).unwrap(),
        DataSheet::new("pow-app", "PartsOfWhole", TableArchetype::PartsOfWhole(PartsOfWholeTable::new(1, 2).unwrap())).unwrap(),
        DataSheet::new("nst-app", "Nested", TableArchetype::Nested(NestedTable::new(1, 2, 2).unwrap())).unwrap(),
    ];

    for sheet in &mut sheets {
        let is_grouped = matches!(sheet.archetype, TableArchetype::Grouped(_));
        let report = apply_import_link_pipeline(sheet, &spec, payload.clone(), &ImportPipelineOptions::default(), None).unwrap();
        let expected_cols = if is_grouped { 2 } else { 3 };
        assert_eq!(sheet.archetype.column_count(), expected_cols, "failed for {:?}", sheet.kind());
        assert_eq!(report.target_range.start.col, 1, "failed start col for {:?}", sheet.kind());
        assert_approx(sheet.cell(CellCoord::new(0, 1, 0)).unwrap().active_number().unwrap(), 11.0, 1e-12);
        if is_grouped {
            assert_approx(sheet.cell(CellCoord::new(1, 1, 1)).unwrap().active_number().unwrap(), 44.0, 1e-12);
        } else {
            assert_approx(sheet.cell(CellCoord::new(1, 2, 0)).unwrap().active_number().unwrap(), 44.0, 1e-12);
        }
    }
}

#[test]
fn import_pipeline_enforces_anchor_overflow_and_max_rows_across_archetypes() {
    let tiny_payload = ImportSourcePayload::DelimitedText { text: "42", delimiter: ',', decimal_separator: '.' };
    let opts = ImportPipelineOptions::default();

    // Anchor overflow (`usize::MAX` row / col / subcol) must fail cleanly without panicking or growing sheets.
    let mut xy_sheet =
        DataSheet::new("xy-ovf", "XY Overflow", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 1, 1).unwrap())).unwrap();
    for bad_anchor in [CellCoord::new(usize::MAX, 0, 0), CellCoord::new(0, usize::MAX, 0), CellCoord::new(0, 0, usize::MAX)] {
        let spec = ImportLinkSpec {
            source: LinkSource::Clipboard,
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, anchor: bad_anchor, ..ImportPlacementSpec::default() },
        };
        let err = apply_import_link_pipeline(&mut xy_sheet, &spec, tiny_payload.clone(), &opts, None).unwrap_err();
        assert!(matches!(err, TransformError::InvalidSpec { .. }), "unexpected overflow error: {err:?}");
        assert_eq!(xy_sheet.archetype.row_count(), 1);
    }

    // MAX_ROWS pre-allocation rejection on MultipleVariables and Nested (must NOT resize row_titles or cell vecs).
    let row_cap_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, anchor: CellCoord::new(MAX_ROWS, 0, 0), ..ImportPlacementSpec::default() },
    };

    let mut mv_row_cap = DataSheet::new("mv-rows", "MV Rows", TableArchetype::MultipleVariables(MultipleVariablesTable::new(2, 1).unwrap())).unwrap();
    let mv_row_err = apply_import_link_pipeline(&mut mv_row_cap, &row_cap_spec, tiny_payload.clone(), &opts, None).unwrap_err();
    assert!(matches!(mv_row_err, TransformError::Doc(DocError::TooManyRows { .. })));
    if let TableArchetype::MultipleVariables(mv) = &mv_row_cap.archetype {
        assert_eq!(mv.row_titles.len(), 1);
        assert_eq!(mv.variables[0].cells.len(), 1);
    }

    let mut nested_row_cap = DataSheet::new("nst-rows", "Nested Rows", TableArchetype::Nested(NestedTable::new(2, 2, 1).unwrap())).unwrap();
    let nested_row_err = apply_import_link_pipeline(&mut nested_row_cap, &row_cap_spec, tiny_payload, &opts, None).unwrap_err();
    assert!(matches!(nested_row_err, TransformError::Doc(DocError::TooManyRows { .. })));
    if let TableArchetype::Nested(nst) = &nested_row_cap.archetype {
        assert_eq!(nst.row_titles.len(), 1);
        assert_eq!(nst.groups[0].cells.len(), 1);
    }
}

#[test]
fn import_pipeline_enforces_cell_budget_before_allocation_across_archetypes() {
    assert_eq!(MAX_IMPORT_CELLS, 10_000_000);
    let tiny_payload = ImportSourcePayload::DelimitedText { text: "42", delimiter: ',', decimal_separator: '.' };
    let opts = ImportPipelineOptions::default();

    // Grouped: 25 datasets * 16 replicates = 400 slots/row; 30,001 rows <= MAX_ROWS yet 12,000,400 cells > MAX_IMPORT_CELLS.
    let mut grp_budget =
        DataSheet::new("grp-budget", "Grouped Budget", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(16), 25, 1).unwrap()))
            .unwrap();
    let grp_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, anchor: CellCoord::new(30_000, 0, 0), ..ImportPlacementSpec::default() },
    };
    assert!(matches!(
        apply_import_link_pipeline(&mut grp_budget, &grp_spec, tiny_payload.clone(), &opts, None),
        Err(TransformError::InvalidSpec { .. })
    ));
    assert_eq!(grp_budget.archetype.row_count(), 1);

    // Column (MeanSdN): 120 datasets * 3 subcolumns = 360 slots/row; 30,001 rows -> 10,800,360 cells > MAX_IMPORT_CELLS.
    let mut col_budget = DataSheet::new(
        "col-budget",
        "Col Budget",
        TableArchetype::Column(ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 120, 1).unwrap()),
    )
    .unwrap();
    assert!(matches!(
        apply_import_link_pipeline(&mut col_budget, &grp_spec, tiny_payload.clone(), &opts, None),
        Err(TransformError::InvalidSpec { .. })
    ));
    assert_eq!(col_budget.archetype.row_count(), 1);

    // MultipleVariables: 20 vars * 600,001 rows = 12,000,020 > MAX_IMPORT_CELLS.
    let mut mv_budget =
        DataSheet::new("mv-budget", "MV Budget", TableArchetype::MultipleVariables(MultipleVariablesTable::new(20, 1).unwrap())).unwrap();
    let mv_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, anchor: CellCoord::new(600_000, 0, 0), ..ImportPlacementSpec::default() },
    };
    assert!(matches!(
        apply_import_link_pipeline(&mut mv_budget, &mv_spec, tiny_payload.clone(), &opts, None),
        Err(TransformError::InvalidSpec { .. })
    ));
    if let TableArchetype::MultipleVariables(mv) = &mv_budget.archetype {
        assert_eq!(mv.row_titles.len(), 1);
        assert_eq!(mv.variables[0].cells.len(), 1);
    }

    // Nested: 5 groups * 10 biological units = 50 units/row; 250,001 rows -> 12,500,050 > MAX_IMPORT_CELLS.
    let mut nested_budget = DataSheet::new("nst-budget", "Nested Budget", TableArchetype::Nested(NestedTable::new(5, 10, 1).unwrap())).unwrap();
    let nested_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, anchor: CellCoord::new(250_000, 0, 0), ..ImportPlacementSpec::default() },
    };
    assert!(matches!(
        apply_import_link_pipeline(&mut nested_budget, &nested_spec, tiny_payload, &opts, None),
        Err(TransformError::InvalidSpec { .. })
    ));
    if let TableArchetype::Nested(nst) = &nested_budget.archetype {
        assert_eq!(nst.row_titles.len(), 1);
        assert_eq!(nst.groups[0].cells.len(), 1);
    }
}

#[test]
fn import_pipeline_enforces_atomic_append_columns_and_unstack_cell_budget() {
    let opts = ImportPipelineOptions::default();

    // AppendColumns combined row+col growth exceeding MAX_IMPORT_CELLS must leave rows & cols completely unchanged.
    let mut mv_append =
        DataSheet::new("mv-app-cap", "MV Append Cap", TableArchetype::MultipleVariables(MultipleVariablesTable::new(10, 1).unwrap())).unwrap();
    let append_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::AppendColumns,
            anchor: CellCoord::new(600_000, 0, 0),
            ..ImportPlacementSpec::default()
        },
    };
    let ten_cols_payload = ImportSourcePayload::DelimitedText { text: "1,2,3,4,5,6,7,8,9,10", delimiter: ',', decimal_separator: '.' };
    assert!(matches!(
        apply_import_link_pipeline(&mut mv_append, &append_spec, ten_cols_payload, &opts, None),
        Err(TransformError::InvalidSpec { .. })
    ));
    assert_eq!(mv_append.archetype.row_count(), 1);
    assert_eq!(mv_append.archetype.column_count(), 10);

    // Unstack producing > MAX_IMPORT_CELLS rejected before dense grid allocation.
    let mut unstack_csv = String::new();
    for g in 0..1_001 {
        unstack_csv.push_str(&format!("G{g},1\n"));
    }
    for _ in 0..10_005 {
        unstack_csv.push_str("G0,2\n");
    }
    let mut unstack_target =
        DataSheet::new("unstack-cap", "Unstack", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap()))
            .unwrap();
    let unstack_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec { unstack_group_column: Some(0), ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec::default(),
    };
    let unstack_payload = ImportSourcePayload::DelimitedText { text: &unstack_csv, delimiter: ',', decimal_separator: '.' };
    assert!(matches!(
        apply_import_link_pipeline(&mut unstack_target, &unstack_spec, unstack_payload, &opts, None),
        Err(TransformError::InvalidSpec { .. })
    ));
}

fn raw_datasheet_for_test(id: &str, archetype: TableArchetype) -> DataSheet {
    DataSheet {
        id: SheetId::new(id),
        title: format!("Sheet {id}"),
        archetype,
        frozen: false,
        info_sheet_id: None,
        linked_regions: Vec::new(),
        floating_notes: Vec::new(),
        color_tag: None,
        exclusion_notes: Vec::new(),
    }
}

#[test]
fn r1_s04_01_ragged_tables_and_mismatched_context_handled_cleanly() {
    // 1. Ragged MultipleVariables rejected cleanly without panic or column truncation.
    let mut mv = MultipleVariablesTable::new(3, 5).expect("mv");
    mv.variables[0].title = String::from("A");
    mv.variables[1].title = String::from("B");
    mv.variables[2].title = String::from("C");
    mv.variables[1].cells.truncate(2);
    for (r, cell) in mv.variables[0].cells.iter_mut().enumerate() {
        *cell = Cell::number((r + 1) as f64);
    }
    let ragged_sheet = raw_datasheet_for_test("mv-rag", TableArchetype::MultipleVariables(mv.clone()));

    assert!(matches!(evaluate_formula_for_sheet("abs(B)", &ragged_sheet, None), Err(TransformError::Doc(DocError::InvalidCellValue { .. }))));

    let mut calc_mv = mv;
    calc_mv.variables[2].set_calculated_formula("=B + 1");
    assert!(matches!(evaluate_calculated_variables(&mut calc_mv, None), Err(TransformError::Doc(DocError::InvalidCellValue { .. }))));
    assert_eq!(calc_mv.variables[0].cells.len(), 5, "ragged DAG validation must fail before shrinking columns");

    let tctx = TransformContext::default();
    assert!(matches!(execute_transpose(&ragged_sheet, true, &tctx), Err(TransformError::Doc(DocError::InvalidCellValue { .. }))));
    assert!(matches!(execute_prune(&ragged_sheet, PruneMethod::EveryNth(2), &tctx), Err(TransformError::Doc(DocError::InvalidCellValue { .. }))));
    assert!(matches!(
        execute_select(&ragged_sheet, &[SelectRule::ExcludeMissing], TableKind::Column, &tctx),
        Err(TransformError::Doc(DocError::InvalidCellValue { .. }))
    ));

    // 2. Ragged Grouped table rejected cleanly by execute_transpose without panic.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).expect("grp");
    grp.data_sets[1].cells.truncate(1);
    let ragged_grp = raw_datasheet_for_test("grp-rag", TableArchetype::Grouped(grp));
    assert!(matches!(execute_transpose(&ragged_grp, false, &tctx), Err(TransformError::Doc(DocError::InvalidCellValue { .. }))));

    // 3. Defense-in-depth: custom FormulaContext returning short column vectors never panics.
    struct RaggedContext;
    impl FormulaContext for RaggedContext {
        fn row_count(&self) -> usize {
            5
        }
        fn resolve_column_index(&self, name: &str) -> Option<usize> {
            match name {
                "A" => Some(0),
                "B" => Some(1),
                _ => None,
            }
        }
        fn column_count(&self) -> usize {
            2
        }
        fn column_values(&self, col_idx: usize) -> Option<Vec<FormulaValue>> {
            match col_idx {
                0 => Some(vec![
                    FormulaValue::Number(1.0),
                    FormulaValue::Number(2.0),
                    FormulaValue::Number(3.0),
                    FormulaValue::Number(4.0),
                    FormulaValue::Number(5.0),
                ]),
                1 => Some(vec![FormulaValue::Number(-2.0), FormulaValue::Number(4.0)]),
                _ => None,
            }
        }
        fn resolve_info_constant(&self, _name: &str) -> Option<FormulaValue> {
            None
        }
    }

    for formula in [
        "abs(B)",
        "A + B",
        "min(A, B)",
        "max(A, B)",
        "sum(A, B)",
        "mean(A, B)",
        "and(A, B)",
        "or(A, B)",
        "not(B)",
        "clamp(B, 0, 10)",
        "if(B > 0, A, B)",
        "sumif(A > 0, B)",
        "meanif(B > 0, A)",
        "diff(B, 1)",
        "rolling_mean(B, 2)",
        "rank(B)",
        "percentile(B, 50)",
    ] {
        let expr = parse_formula(formula).expect("parse formula");
        let out = eval_column_expr(&expr, &RaggedContext).expect("eval ragged context safely");
        assert_eq!(out.len(), 5, "formula `{formula}` changed row length");
    }
}

#[test]
fn r1_s05_02_import_transpose_and_x_mode_enforce_budgets() {
    let opts = ImportPipelineOptions::default();

    // 1. Sparse ragged transposed import whose rectangular envelope (4,000 * 4,001 > MAX_IMPORT_CELLS) rejects before allocation.
    let mut wide_first_row = (0..4_000).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    wide_first_row.push('\n');
    for r in 0..4_000 {
        wide_first_row.push_str(&format!("{r}\n"));
    }
    let mut target =
        DataSheet::new("imp-rag", "Target", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap()))
            .expect("sheet");
    let spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { transpose: true, ..ImportPlacementSpec::default() },
    };
    let payload = ImportSourcePayload::DelimitedText { text: &wide_first_row, delimiter: ',', decimal_separator: '.' };
    assert!(matches!(apply_import_link_pipeline(&mut target, &spec, payload, &opts, None), Err(TransformError::InvalidSpec { .. })));

    // 2. Bounded ragged transposed import within MAX_IMPORT_CELLS (1 row of 1,000 + 50 rows of 1) succeeds cleanly.
    let mut bounded_csv = (0..1_000).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    bounded_csv.push('\n');
    for r in 0..50 {
        bounded_csv.push_str(&format!("{}\n", 1_000 + r));
    }
    let bounded_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::AppendColumns, transpose: true, ..ImportPlacementSpec::default() },
    };
    let bounded_payload = ImportSourcePayload::DelimitedText { text: &bounded_csv, delimiter: ',', decimal_separator: '.' };
    let report = apply_import_link_pipeline(&mut target, &bounded_spec, bounded_payload, &opts, None).expect("bounded ragged transpose");
    assert_eq!(report.rows_placed, 1_000);
    assert_eq!(report.cols_placed, 51);
    assert_eq!(target.cell(CellCoord::new(999, 1, 0)).and_then(Cell::active_number), Some(999.0));
    assert_eq!(target.cell(CellCoord::new(0, 51, 0)).and_then(Cell::active_number), Some(1_049.0));

    // 3. Invalid XColumnMode::Numbers { RawReplicates(usize::MAX) | RawReplicates(0) } rejected cleanly on import growth.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 1).expect("xy");
    xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(usize::MAX) };
    let mut bad_xy = raw_datasheet_for_test("bad-xy", TableArchetype::Xy(xy));
    let simple_spec =
        ImportLinkSpec { source: LinkSource::Clipboard, filter: ImportFilterSpec::default(), placement: ImportPlacementSpec::default() };
    let tiny = ImportSourcePayload::DelimitedText { text: "1,2\n3,4\n", delimiter: ',', decimal_separator: '.' };
    assert!(matches!(apply_import_link_pipeline(&mut bad_xy, &simple_spec, tiny, &opts, None), Err(TransformError::Doc(_))));
    let _ = MAX_SUBCOLUMNS;
    let _ = MAX_IMPORT_CELLS;
}
