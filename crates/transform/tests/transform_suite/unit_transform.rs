use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, Cell, CellCoord, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, DocError,
    ErrorPropagationMode, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind, LinkSource,
    LinkedDataRegion, MAX_ROWS, MathTransformPreset, MultipleVariablesTable, NestedTable, NormalizeAnchor, PartsOfWholeTable, PlacementMode,
    SheetCellRange, SheetId, SimulationNoise, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, TransformAnalysisSpec, VariableRole,
    VariableSource, XColumnMode, XyTable,
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

use crate::common::{
    ExpectVal::{Missing, Num},
    FormulaInfoCase, SubExp, Xy3Col, as_col, as_contingency, as_grouped, as_mv, as_nested, as_xy, assert_approx, assert_cells_3, assert_formula_rows,
    assert_scalar_cases, clipboard_spec, col_empty, col_sheet, empty, empty_cells, excl, import_csv, import_tsv, info_with_numbers, is_invalid_cell,
    mv_empty, mv_sheet, mv_sheet_roles, mv_table, num, raw_sheet, sheet, str_vec, txt, xy_3sub, xy_3sub_clean, xy_single,
};

// ============================================================================
// 1. Formula Parser, Evaluator, Functions, NA Propagation, and DAG
// ============================================================================

#[test]
fn formula_precedence_associativity_and_scalar_math() {
    assert_scalar_cases(
        &[
            ("=-2^2", -4.0),
            ("2^3^2", 512.0),
            ("1 + 2 * 3 - 4 / 2", 5.0),
            ("if(1 < 2 AND 3 >= 3, 10, 20)", 10.0),
            ("if(NOT (1 == 2 OR 2 <> 2), 42, 0)", 42.0),
            ("abs(-7) + sqrt(16) + cbrt(27)", 14.0),
            ("floor(2.9) + ceil(2.1) + round(2.6) + trunc(-2.9) + sign(-5)", 5.0),
            ("clamp(15, 0, 10) + min(3, 5, 2) + max(1, 9, 4)", 21.0),
            ("pow(2, 5) + exp(0) + ln(1) + log10(1000) + log2(8)", 39.0),
            ("sin(0) + cos(0) + tan(0) + atan2(1, 1) * 4 / 3.141592653589793", 2.0),
            ("sinh(0) + cosh(0) + tanh(0)", 1.0),
            ("asin(0) + acos(1) + atan(0)", 0.0),
            ("erf(0) + erfc(0) + lgamma(1) + logit(0.5)", 1.0),
            ("probit(0.5)", 5.0),
            ("norm_inv(0.5)", 0.0),
        ],
        1e-12,
    );
}

#[test]
fn formula_aggregates_window_and_excluded_propagation() {
    let s = mv_sheet(
        "mv1",
        &[
            ("Dose", &[num(1.0), num(2.0), num(3.0), num(4.0), excl(100.0)]),
            ("Group", &[txt("ctrl"), txt("ctrl"), txt("trt"), txt("trt"), txt("ctrl")]),
        ],
    );
    let info = info_with_numbers("info1", &[("Factor", 10.0)]);

    let cases: &[FormulaInfoCase<'_>] = &[
        ("sum(Dose)", true, &[(0, Num(10.0))]),
        ("mean(Dose) * Info(\"Factor\")", true, &[(0, Num(25.0))]),
        ("count(Dose)", false, &[(0, Num(4.0))]),
        ("percentile(Dose, 50) + median(Dose) + min_col(Dose) + max_col(Dose)", false, &[(0, Num(10.0))]),
        ("geo_mean(Dose)", false, &[(0, Num((24.0_f64).powf(0.25)))]),
        ("sumif(Group == \"trt\", Dose)", false, &[(0, Num(7.0))]),
        ("countif(Group == \"ctrl\")", false, &[(0, Num(3.0))]),
        ("meanif(Group == \"ctrl\", Dose)", false, &[(0, Num(1.5))]),
        ("cumsum(Dose)", false, &[(0, Num(1.0)), (3, Num(10.0)), (4, Missing)]),
        ("cumprod(Dose)", false, &[(3, Num(24.0))]),
        ("diff(Dose)", false, &[(0, Missing), (1, Num(1.0))]),
        ("lag(Dose, 2)", false, &[(0, Missing), (1, Missing), (2, Num(1.0))]),
        ("lead(Dose, 1)", false, &[(0, Num(2.0))]),
        ("rolling_mean(Dose, 2)", false, &[(1, Num(1.5))]),
        ("rolling_sum(Dose, 3)", false, &[(2, Num(6.0))]),
        ("rank(Dose)", false, &[(0, Num(1.0)), (3, Num(4.0))]),
        ("if(isvalid(Dose), $row + $index, coalesce(Dose, -99))", false, &[(0, Num(1.0)), (4, Num(-99.0))]),
        ("isnan(Dose)", false, &[(4, Num(1.0))]),
    ];
    for &(expr, use_info, checks) in cases {
        assert_formula_rows(&s, use_info.then_some(&info), expr, checks, 1e-12);
    }

    assert!(evaluate_formula_for_sheet("var(Dose) + sd(Dose) + sem(Dose) + iqr(Dose)", &s, None).unwrap()[0].as_finite_f64().is_some());
    let z = evaluate_formula_for_sheet("zscore(Dose)", &s, None).unwrap();
    assert_approx(z[0].as_finite_f64().unwrap() + z[3].as_finite_f64().unwrap(), 0.0, 1e-12);
}

#[test]
fn formula_error_diagnostics() {
    for bad in ["1 + * 2", "sqrt(1, 2"] {
        assert!(matches!(parse_formula(bad), Err(TransformError::ParseError { .. })), "{bad}");
    }
    let env = ScalarEnv::default();
    assert!(matches!(eval_scalar_expr(&parse_formula("unknown_fn(1)").unwrap(), &env), Err(TransformError::UnknownFunction { .. })));
    assert!(matches!(eval_scalar_expr(&parse_formula("sqrt(1, 2)").unwrap(), &env), Err(TransformError::ArityMismatch { .. })));
    assert!(matches!(eval_scalar_expr(&parse_formula("MissingVar + 1").unwrap(), &env), Err(TransformError::UnknownVariable { .. })));
}

#[test]
fn formula_dag_topological_order_and_cycle_detection() {
    let mut mv =
        mv_table(&[("Final", &[empty(), empty(), empty()]), ("Mid", &[empty(), empty(), empty()]), ("Raw", &[num(1.0), num(2.0), num(3.0)])]);
    mv.variables[0].set_calculated_formula("=Mid + [InfoConst]");
    mv.variables[1].set_calculated_formula("=col(\"Raw\") * 2");
    let info = info_with_numbers("i1", &[("InfoConst", 10.0)]);

    assert!(extract_dependencies(&parse_formula("=Mid + [InfoConst]").unwrap()).contains(&DependencyTarget::ColumnName(String::from("Mid"))));
    assert_eq!(evaluate_calculated_variables(&mut mv, Some(&info)).unwrap(), vec![1, 0]);
    assert_approx(mv.variables[0].cells[0].active_number().unwrap(), 12.0, 1e-12);
    assert_approx(mv.variables[0].cells[2].active_number().unwrap(), 16.0, 1e-12);

    let mut ds = sheet("mv", TableArchetype::MultipleVariables(mv.clone()));
    assert_eq!(evaluate_data_sheet_calculated_variables(&mut ds, Some(&info)).unwrap(), vec![1, 0]);

    let mut self_cyc = mv;
    self_cyc.variables[0].set_calculated_formula("=Final + 1");
    assert!(matches!(evaluate_calculated_variables(&mut self_cyc, None), Err(TransformError::CircularDependency { .. })));

    let mut three_cyc = mv_table(&[("V0", &[empty(), empty()]), ("V1", &[empty(), empty()]), ("V2", &[empty(), empty()])]);
    three_cyc.variables[0].set_calculated_formula("=V1 + 1");
    three_cyc.variables[1].set_calculated_formula("=V2 + 1");
    three_cyc.variables[2].set_calculated_formula("=V0 + 1");
    assert!(matches!(evaluate_calculated_variables(&mut three_cyc, None), Err(TransformError::CircularDependency { cycle }) if cycle.len() >= 3));
}

// ============================================================================
// 2. TransformAnalysisSpec Operations & Error Propagation
// ============================================================================

#[test]
fn math_transform_and_error_propagation_vs_finite_difference() {
    let s = xy_3sub(
        "xy_err",
        SubcolumnMode::MeanSdN,
        &[100.0, 1000.0],
        &[Xy3Col { title: "A", triples: &[[Some(10.0), Some(0.5), Some(4.0)], [Some(20.0), Some(1.0), Some(5.0)]], excl_primary: &[false, true] }],
    );
    let ctx = TransformContext::default();

    // 2a. FirstOrderDeltaMethod vs central finite-difference derivative.
    let delta_out =
        execute_math_transform(&s, Some(&MathTransformOp::Log10), Some(&MathTransformOp::Ln), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx)
            .unwrap();
    let res_xy = as_xy(&delta_out.data_sheet);
    assert_approx(res_xy.x_value_at(0).unwrap(), 2.0, 1e-12);
    let eps = 1e-6_f64;
    let fd_deriv = ((10.0_f64 + eps).ln() - (10.0_f64 - eps).ln()) / (2.0 * eps);
    assert_cells_3(
        &res_xy.data_sets[0].cells[0],
        [SubExp::Num(10.0_f64.ln()), SubExp::Num(fd_deriv.abs() * 0.5), SubExp::Num(4.0)],
        (1e-8, 0.0),
        "ln delta",
    );
    assert!(res_xy.data_sets[0].cells[1][0].excluded);

    // 2b. ReplicateEndpoints on Log10.
    let ep_out = execute_transform(
        &s,
        &TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: Some(MathTransformPreset::Log10),
            error_propagation: ErrorPropagationMode::ReplicateEndpoints,
        },
        &ctx,
    )
    .unwrap();
    assert_approx(as_xy(&ep_out.data_sheet).data_sets[0].cells[0][1].active_number().unwrap(), 0.5 * (10.5_f64.log10() - 9.5_f64.log10()), 1e-12);

    // 2c. MeanLowHigh asymmetric intervals under UserExpression & MultiplyByConstant.
    let lh_s = xy_3sub_clean("col_lh", SubcolumnMode::MeanLowHigh, &[2.0], &[("A", &[[Some(4.0), Some(0.5), Some(1.0)]])]);
    let user_out = execute_math_transform(
        &lh_s,
        None,
        Some(&MathTransformOp::UserExpression(String::from("Y^2 + 1"))),
        ErrorPropagationMode::ReplicateEndpoints,
        &ctx,
    )
    .unwrap();
    assert_cells_3(
        &as_xy(&user_out.data_sheet).data_sets[0].cells[0],
        [SubExp::Num(17.0), SubExp::Num(3.75), SubExp::Num(9.0)],
        (1e-10, 0.0),
        "Y^2+1",
    );

    let mul_out =
        execute_math_transform(&lh_s, None, Some(&MathTransformOp::MultiplyByConstant(2.0)), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx)
            .unwrap();
    assert_cells_3(&as_xy(&mul_out.data_sheet).data_sets[0].cells[0], [SubExp::Num(8.0), SubExp::Num(1.0), SubExp::Any], (1e-12, 0.0), "mul 2");
}

#[test]
fn normalize_baseline_transpose_prune_and_select() {
    let info = info_with_numbers("info_norm", &[("MaxRef", 50.0)]);
    let ctx = TransformContext::with_info(&info);
    let xy = xy_single("s1", &[1.0, 2.0, 3.0, 4.0], &[&[10.0, 20.0, 30.0, 50.0], &[0.0, 25.0, 50.0, 100.0]]);

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
    let n_xy = as_xy(&norm_out.data_sheet);
    for (ds, r, exp) in [(0, 0, 0.0), (0, 2, 50.0), (0, 3, 100.0), (1, 1, 50.0), (1, 3, 200.0)] {
        assert_approx(n_xy.data_sets[ds].cells[r][0].active_number().unwrap(), exp, 1e-12);
    }

    // RemoveBaseline via AnalysisSpec & FirstAndLastLinear.
    let analysis = AnalysisSpec::new(
        "an1",
        "s1",
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Difference }),
    )
    .unwrap();
    let base_xy = as_xy(execute_analysis_transform(&xy, &analysis, &ctx).unwrap().analyzable_data.as_ref().unwrap()).clone();
    assert_approx(base_xy.data_sets[1].cells[0][0].active_number().unwrap(), -10.0, 1e-12);
    assert_approx(base_xy.data_sets[1].cells[1][0].active_number().unwrap(), 5.0, 1e-12);

    let lb_xy = as_xy(
        &execute_remove_baseline_kind(&xy, &BaselineKind::FirstAndLastLinear { first_k: 1, last_k: 1 }, BaselineOperation::Difference, &ctx)
            .unwrap()
            .data_sheet,
    )
    .clone();
    assert_approx(lb_xy.data_sets[1].cells[0][0].active_number().unwrap(), 0.0, 1e-12);
    assert_approx(lb_xy.data_sets[1].cells[3][0].active_number().unwrap(), 0.0, 1e-12);

    // Transpose.
    let col_s = col_sheet("c1", &[("Ctrl", &[num(1.0), excl(2.0)]), ("Drug", &[num(3.0), num(4.0)])]);
    let tr_col =
        as_col(&execute_transform(&col_s, &TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true }, &ctx).unwrap().data_sheet).clone();
    assert_eq!(tr_col.row_titles, vec!["Ctrl", "Drug"]);
    assert!(tr_col.data_sets[1].cells[0][0].excluded);
    assert_approx(tr_col.data_sets[1].cells[1][0].active_number().unwrap(), 4.0, 1e-12);

    // Prune: AverageBins, MinMaxBins, LogarithmicSpacing.
    let pa_xy =
        as_xy(&execute_transform(&xy, &TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: true }, &ctx).unwrap().data_sheet).clone();
    assert_eq!(pa_xy.row_count(), 2);
    assert_approx(pa_xy.x_value_at(0).unwrap(), 1.5, 1e-12);
    assert_approx(pa_xy.data_sets[0].cells[0][0].active_number().unwrap(), 15.0, 1e-12);
    assert_eq!(execute_prune(&xy, PruneMethod::MinMaxBins(4), &ctx).unwrap().data_sheet.archetype.row_count(), 2);

    let xs100: Vec<f64> = (1..=100).map(|i| i as f64).collect();
    let ys100: Vec<f64> = (1..=100).map(|i| (i * 2) as f64).collect();
    let log_n = execute_prune(&xy_single("big", &xs100, &[&ys100]), PruneMethod::LogarithmicSpacing { target_points: 10 }, &ctx)
        .unwrap()
        .data_sheet
        .archetype
        .row_count();
    assert!((5..=10).contains(&log_n));

    // Select & Transform (predicate -> Column, TopN -> XY).
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
    assert_eq!((sel_out.data_sheet.kind(), sel_out.data_sheet.archetype.row_count()), (TableKind::Column, 2));

    let top1 = execute_select(&xy, &[SelectRule::TopN { column: 0, n: 1 }], TableKind::Xy, &ctx).unwrap();
    assert_eq!(top1.data_sheet.archetype.row_count(), 1);
    assert_approx(as_xy(&top1.data_sheet).data_sets[0].cells[0][0].active_number().unwrap(), 50.0, 1e-12);
}

#[test]
fn simulate_deterministic_equations_and_noise() {
    let hill_p = [("Bottom", 10.0), ("Top", 100.0), ("LogEC50", -6.0), ("HillSlope", 1.0)];
    let mm_p = [("Vmax", 100.0), ("Km", 2.0)];
    let custom_p = [("A", 2.0), ("B", 5.0)];
    for (eq, x, params, exp) in
        [("hill_4pl", -6.0, &hill_p[..], 55.0), ("michaelis_menten", 2.0, &mm_p[..], 50.0), ("expr: A * X^2 + B", 3.0, &custom_p[..], 23.0)]
    {
        let p: Vec<(String, f64)> = params.iter().map(|&(k, v)| (k.to_owned(), v)).collect();
        assert_approx(evaluate_equation(eq, x, &p).unwrap(), exp, 1e-12);
    }

    let decay_p = vec![(String::from("y0"), 100.0), (String::from("k"), 0.2), (String::from("plateau"), 10.0)];
    let s1 = simulate_xy_sheet("sim1", "Sim", 42, 10, 3, 0.0, 9.0, "exponential_decay", &decay_p, SimulationNoise::Gaussian { sd: 1.5 }).unwrap();
    let s2 = simulate_xy_sheet("sim1", "Sim", 42, 10, 3, 0.0, 9.0, "exponential_decay", &decay_p, SimulationNoise::Gaussian { sd: 1.5 }).unwrap();
    assert_eq!(s1, s2);

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
    assert_eq!(execute_transform(&s1, &spec, &TransformContext::default()).unwrap().data_sheet.archetype.row_count(), 5);
}

// ============================================================================
// 3. Curve Calculus: AUC, Peaks, SG/MA/LOWESS Smoothing, Derivatives, Integral
// ============================================================================

#[test]
fn curve_auc_peaks_se_sg_derivatives_and_integral() {
    let auc = compute_auc(&[(0.0, -2.0), (2.0, 2.0), (4.0, 0.0), (5.0, -1.0)], &AucOptions::default()).unwrap();
    assert_approx(auc.positive_area, 3.0, 1e-12);
    assert_approx(auc.negative_area, 1.5, 1e-12);
    assert_approx(auc.net_area, 1.5, 1e-12);
    assert_eq!(auc.peaks.len(), 1);
    assert_approx(auc.peaks[0].peak_x, 2.0, 1e-12);
    assert_approx(auc.peaks[0].area, 3.0, 1e-12);

    let auc_se = compute_auc_with_se(&[(0.0, 1.0, 0.1), (1.0, 2.0, 0.2), (2.0, 1.0, 0.1)], &AucOptions::default()).unwrap();
    assert_approx(auc_se.standard_error.unwrap(), (0.045_f64).sqrt(), 1e-12);

    let xy_s = xy_3sub_clean(
        "xy_auc",
        SubcolumnMode::MeanSemN,
        &[0.0, 1.0, 2.0],
        &[("A", &[[Some(1.0), Some(0.25), Some(4.0)], [Some(2.0), Some(0.25), Some(4.0)], [Some(3.0), Some(0.25), Some(4.0)]])],
    );
    let sheet_auc = compute_sheet_auc(&xy_s, 0, &AucOptions::default()).unwrap();
    assert!(sheet_auc.standard_error.is_some());
    assert_approx(sheet_auc.positive_area, 4.0, 1e-12);

    // Savitzky-Golay 5/7/9 exactness on cubic polynomials.
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

    let noisy = [(0.0, 0.0), (1.0, 1.0), (2.0, 8.0), (3.0, 3.0), (4.0, 4.0), (5.0, 5.0)];
    assert!(smooth_xy(&noisy, SmoothMethod::MovingAverage { window: 3 }).unwrap()[2].1 < 8.0);
    assert!(smooth_xy(&noisy, SmoothMethod::Lowess { span: 0.6, robustness_iters: 2 }).unwrap()[2].1 < 8.0);

    // Exact 3-pt Lagrange derivatives on quadratic f(x) = 3x^2 - 4x + 5 on non-uniform grid.
    let quad: Vec<(f64, f64)> = [0.0, 0.5, 1.5, 3.0, 4.2].into_iter().map(|x| (x, 3.0 * x * x - 4.0 * x + 5.0)).collect();
    for &(x, dy) in &differentiate_xy(&quad, DerivativeOrder::First).unwrap() {
        assert_approx(dy, 6.0 * x - 4.0, 1e-11);
    }
    for &(_, d2y) in &differentiate_xy(&quad, DerivativeOrder::Second).unwrap() {
        assert_approx(d2y, 6.0, 1e-11);
    }

    let integ = integrate_xy(&[(0.0, 2.0), (2.0, 6.0), (4.0, 6.0)], 0.0).unwrap();
    for (i, exp) in [0.0, 8.0, 20.0].into_iter().enumerate() {
        assert_approx(integ[i].1, exp, 1e-12);
    }

    let deriv_sheet = apply_curve_calculus_to_sheet(
        &xy_single("curve_s", &[0.0, 1.0, 2.0, 3.0], &[&[0.0, 1.0, 4.0, 9.0]]),
        CurveSheetOp::Differentiate(DerivativeOrder::Second),
    )
    .unwrap();
    assert_approx(as_xy(&deriv_sheet).data_sets[0].cells[1][0].active_number().unwrap(), 2.0, 1e-11);
}

// ============================================================================
// 4. 4-Stage ImportLinkSpec Pipeline & Link Cycle Detection
// ============================================================================

fn internal_link_region(link_id: &str, src_sheet: &str, provenance: &str) -> LinkedDataRegion {
    let range = SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0));
    LinkedDataRegion {
        id: link_id.to_owned(),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet { sheet_id: SheetId::new(src_sheet), kind: InternalLinkKind::DataSheet, tab_id: None, range },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: range,
        provenance_summary: provenance.to_owned(),
    }
}

#[test]
fn import_link_pipeline_four_stages_and_cycle_detection() {
    let mut target = col_empty("dst", 2, 1);
    let mut info = info_with_numbers("info_imp", &[]);
    let csv = "Group;Value\nG1;1,5\nG1;*2,5\nG2;3,5\nG2;4,5";
    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("plate.csv"), delimiter: ';', decimal_separator: ',' },
        filter: ImportFilterSpec {
            row_window: Some((1, 4)),
            unstack_group_column: Some(0),
            parse_asterisk_exclusions: true,
            ..ImportFilterSpec::default()
        },
        placement: ImportPlacementSpec { propagate_title_header: true, ..ImportPlacementSpec::default() },
    };
    let opts = ImportPipelineOptions {
        link_id: Some(String::from("lnk1")),
        provenance_summary: Some(String::from("Plate CSV import")),
        ..ImportPipelineOptions::default()
    };
    let payload = ImportSourcePayload::DelimitedText { text: csv, delimiter: ';', decimal_separator: ',' };

    let report = apply_import_link_pipeline(&mut target, &spec, payload.clone(), &opts, Some(&mut info)).unwrap();
    assert_eq!((report.cols_placed, report.rows_placed, target.linked_regions.len()), (2, 2, 1));
    assert!(info.notes_markdown.contains("Plate CSV import"));
    let col = as_col(&target);
    assert_eq!((col.data_sets[0].title.as_str(), col.data_sets[1].title.as_str()), ("G1", "G2"));
    assert_approx(col.data_sets[0].cells[0][0].active_number().unwrap(), 1.5, 1e-12);
    assert!(col.data_sets[0].cells[1][0].excluded);

    // Frozen target guard.
    target.frozen = true;
    assert!(matches!(apply_import_link_pipeline(&mut target, &spec, payload, &opts, None), Err(TransformError::Doc(DocError::SheetFrozen { .. }))));

    // Link cycle detection across s1 -> s2 -> s3 and cyclic s3 -> s1.
    let mut s1 = col_sheet("s1", &[("A", &[num(1.0)])]);
    let mut s2 = col_empty("s2", 1, 1);
    let mut s3 = col_empty("s3", 1, 1);
    s2.linked_regions.push(internal_link_region("l1", "s1", "s1->s2"));
    s3.linked_regions.push(internal_link_region("l2", "s2", "s2->s3"));
    assert_eq!(detect_link_cycles(&[s3.clone(), s1.clone(), s2.clone()]).unwrap(), vec![SheetId::new("s1"), SheetId::new("s2"), SheetId::new("s3")]);

    s1.linked_regions.push(internal_link_region("l3", "s3", "s3->s1"));
    assert!(matches!(detect_link_cycles(&[s1, s2, s3]), Err(TransformError::CircularDependency { .. })));
}

fn build_wf29_wf31_master_sheet() -> DataSheet {
    let mut s = mv_sheet_roles(
        "master",
        &[
            ("SampleID", VariableRole::Label, &[txt("S1"), txt("S2"), txt("S3"), txt("S4"), txt("S5")]),
            ("Group", VariableRole::Categorical, &[txt("Ctrl"), txt("Drug"), txt("Drug"), txt("Drug"), txt("Drug")]),
            ("RefGroup", VariableRole::Categorical, &[txt("Ctrl"), txt("Drug"), txt("Drug"), txt("Alt"), txt("Drug")]),
            ("Dose", VariableRole::Continuous, &[num(0.0), num(10.0), num(25.0), num(50.0), num(0.0)]),
            ("Baseline", VariableRole::Continuous, &[num(10.0), num(20.0), num(20.0), num(30.0), num(15.0)]),
            ("Response", VariableRole::Continuous, &[num(12.0), num(18.0), num(35.0), num(52.0), num(40.0)]),
            ("CountYes", VariableRole::Continuous, &[num(5.0), num(8.0), num(14.0), num(18.0), num(9.0)]),
            ("CountNo", VariableRole::Continuous, &[num(15.0), num(12.0), num(6.0), num(2.0), num(11.0)]),
        ],
    );
    let mv = crate::common::as_mv_mut(&mut s);
    mv.row_titles = str_vec(&["R1", "R2", "R3", "R4", "R5"]);
    mv.row_titles_expanded = true;
    s
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
    let xy = as_xy(&xy_out.data_sheet);
    assert_eq!(
        (xy.x_title.as_str(), xy.data_sets.len(), xy.data_sets[0].title.as_str(), xy.row_count(), xy.row_titles.as_slice()),
        ("Dose", 1, "Response", 2, str_vec(&["R3", "R4"]).as_slice())
    );
    assert_approx(xy.x_value_at(0).unwrap(), 25.0, 1e-12);
    assert_approx(xy.data_sets[0].cells[0][0].active_number().unwrap(), 35.0, 1e-12);

    let ctx_cols = TransformContext { selected_data_sets: vec![4, 5], ..Default::default() };
    let rule_cat = SelectRule::Predicate(String::from("Group == \"Drug\" AND Group == RefGroup AND Dose > 0"));
    let col_tbl = as_col(&execute_select(&master, std::slice::from_ref(&rule_cat), TableKind::Column, &ctx_cols).unwrap().data_sheet).clone();
    assert_eq!(
        (col_tbl.data_sets.len(), col_tbl.data_sets[0].title.as_str(), col_tbl.data_sets[1].title.as_str(), col_tbl.row_titles.as_slice()),
        (2, "Baseline", "Response", str_vec(&["R2", "R3"]).as_slice())
    );

    let grp_tbl = as_grouped(&execute_select(&master, std::slice::from_ref(&rule_cat), TableKind::Grouped, &ctx_cols).unwrap().data_sheet).clone();
    assert_eq!(
        (grp_tbl.data_sets.len(), grp_tbl.data_sets[0].title.as_str(), grp_tbl.row_titles.as_slice()),
        (2, "Baseline", str_vec(&["R2", "R3"]).as_slice())
    );
}

#[test]
fn select_and_transform_contingency_mv_subset_and_bounds() {
    let master = build_wf29_wf31_master_sheet();
    let sel_ctx = |cols: Vec<usize>| TransformContext { selected_data_sets: cols, ..Default::default() };

    let cont_out = execute_select(&master, &[SelectRule::RowRange(2, 3)], TableKind::Contingency, &sel_ctx(vec![6, 7])).unwrap();
    let cont_tbl = as_contingency(&cont_out.data_sheet);
    assert_eq!((cont_tbl.data_sets.len(), cont_tbl.data_sets[0].title.as_str(), cont_tbl.data_sets[1].title.as_str()), (2, "CountYes", "CountNo"));
    assert_eq!(
        (cont_tbl.row_titles.as_slice(), cont_tbl.count_at(0, 0).unwrap(), cont_tbl.count_at(1, 1).unwrap()),
        (str_vec(&["R3", "R4"]).as_slice(), Some(14), Some(2))
    );

    assert!(matches!(
        execute_select(&master, &[], TableKind::Contingency, &sel_ctx(vec![0, 5])),
        Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. }))
    ));

    let sub_out =
        execute_select(&master, &[SelectRule::Predicate(String::from("Group != \"Ctrl\""))], TableKind::MultipleVariables, &sel_ctx(vec![1, 5]))
            .unwrap();
    let sub = as_mv(&sub_out.data_sheet);
    assert_eq!(
        (sub.variables.len(), sub.variables[0].title.as_str(), sub.variables[0].role, sub.variables[1].title.as_str(), sub.row_count()),
        (2, "Group", VariableRole::Categorical, "Response", 4)
    );

    assert_eq!(execute_select(&master, &[], TableKind::Column, &TransformContext::default()).unwrap().data_sheet.archetype.column_count(), 8);
    assert!(matches!(execute_select(&master, &[], TableKind::Column, &sel_ctx(vec![99])), Err(TransformError::InvalidSpec { .. })));
}

#[test]
fn validate_no_formula_cycle_guards_and_errors() {
    let mut mv = mv_table(&[
        ("ColA", &[empty(), empty(), empty()]),
        ("ColB", &[empty(), empty(), empty()]),
        ("ColC", &[empty(), empty(), empty()]),
        ("Raw", &[num(10.0), num(20.0), num(30.0)]),
    ]);
    mv.variables[1].set_calculated_formula("=ColA + 1");
    mv.variables[2].set_calculated_formula("=ColB * 2");

    validate_no_formula_cycle(&mv, 0, "=Raw / 2").unwrap();
    assert!(matches!(mv.variables[0].source, VariableSource::Entered));

    for cyc in ["=ColA + 1", "=A * 2", "=col(0) + Raw", "=ColC - 1"] {
        assert!(matches!(validate_no_formula_cycle(&mv, 0, cyc), Err(TransformError::CircularDependency { .. })), "{cyc}");
    }
    let TransformError::CircularDependency { cycle } = validate_no_formula_cycle(&mv, 0, "=ColB + 5").unwrap_err() else {
        panic!("expected 2-hop CircularDependency");
    };
    assert!(cycle.contains(&String::from("ColA")) && cycle.contains(&String::from("ColB")));

    assert!(matches!(validate_no_formula_cycle(&mv, 9, "=1"), Err(TransformError::InvalidSpec { .. })));
    assert!(matches!(validate_no_formula_cycle(&mv, 0, "=1 + * 2"), Err(TransformError::ParseError { .. })));
}

#[test]
fn rowwise_round_sum_mean_if_log10_and_qpcr_ddct_chain() {
    assert_scalar_cases(
        &[("ROUND(2.71828, 2)", 2.72), ("ROUND(1234, -2)", 1200.0), ("ROUND(2.6)", 3.0), ("SUM(1, 2, 3.5)", 6.5), ("MEAN(2, 4, 9)", 5.0)],
        1e-12,
    );

    let e4 = [empty(), empty(), empty(), empty()];
    let mut qpcr = mv_table(&[
        ("Group", &[txt("Control"), txt("Control"), txt("Treated"), txt("Treated")]),
        ("Ct_T1", &[num(21.9), num(22.0), num(19.8), num(20.0)]),
        ("Ct_T2", &[num(22.1), num(22.0), num(20.2), num(20.0)]),
        ("Ct_T3", &[num(22.0), num(22.0), num(20.0), num(20.0)]),
        ("Ct_R1", &[num(18.0), num(17.8), num(18.0), num(17.9)]),
        ("Ct_R2", &[num(18.0), num(18.2), num(18.0), num(18.1)]),
        ("Ct_R3", &[excl(99.0), num(18.0), num(18.0), num(18.0)]),
        ("TargetMean", &e4),
        ("RefMean", &e4),
        ("DeltaCt", &e4),
        ("CalDeltaCt", &e4),
        ("ddCt", &e4),
        ("FoldChange", &e4),
        ("NotesAndMetrics", &e4),
    ]);
    qpcr.variables[0].role = VariableRole::Categorical;
    for (idx, f) in [
        (12, "=ROUND(2 ^ (-ddCt), 4)"),
        (11, "=DeltaCt - CalDeltaCt"),
        (10, "=MEANIF(Group == \"Control\", DeltaCt)"),
        (9, "=TargetMean - RefMean"),
        (8, "=MEAN(Ct_R1, Ct_R2, Ct_R3)"),
        (7, "=MEAN(Ct_T1, Ct_T2, Ct_T3)"),
        (13, "=IF(FoldChange > 2, ROUND(LOG10(SUM(Ct_T1, Ct_T2) / RefMean), 3), 0)"),
    ] {
        qpcr.variables[idx].set_calculated_formula(f);
    }
    for col_idx in 7..=13 {
        if let VariableSource::Calculated { formula } = &qpcr.variables[col_idx].source {
            validate_no_formula_cycle(&qpcr, col_idx, formula).unwrap();
        }
    }

    assert_eq!(evaluate_calculated_variables(&mut qpcr, None).unwrap().len(), 7);
    for (col, row, exp) in [(8, 0, 18.0), (9, 0, 4.0), (12, 0, 1.0), (12, 2, 4.0), (13, 0, 0.0), (13, 2, 0.347)] {
        assert_approx(qpcr.variables[col].cells[row].active_number().unwrap(), exp, 1e-12);
    }
}

#[test]
fn append_columns_grows_multiple_variables_and_column_tables() {
    // 1. MultipleVariables: append text column, shorter numeric columns, and unstacked header columns.
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    for c in 0..3 {
        for r in 0..4 {
            mv.variables[c].cells[r] = num((c * 10 + r + 1) as f64);
        }
    }
    let mut mv_s = sheet("mv-append", TableArchetype::MultipleVariables(mv));
    let app_spec = clipboard_spec(PlacementMode::AppendColumns, CellCoord::new(0, 0, 0), ImportFilterSpec::default());

    let r1 = import_tsv(&mut mv_s, "Cluster 1\nCluster 2\nCluster 1\nCluster 3", &app_spec).unwrap();
    assert_eq!((r1.rows_placed, r1.cols_placed, r1.target_range), (4, 1, SheetCellRange::new(CellCoord::new(0, 3, 0), CellCoord::new(3, 3, 0))));
    let mv1 = as_mv(&mv_s);
    assert_eq!((mv1.variables.len(), mv1.variables[3].title.as_str(), mv1.variables[3].role), (4, "D", VariableRole::Categorical));
    assert_eq!(
        mv1.variables[3].cells.iter().map(Cell::active_text).collect::<Vec<_>>(),
        vec![Some("Cluster 1"), Some("Cluster 2"), Some("Cluster 1"), Some("Cluster 3")]
    );
    assert_approx(mv1.variables[0].cells[0].active_number().unwrap(), 1.0, 1e-12);

    let r2 = import_tsv(&mut mv_s, "1.5\t2.5\n3.5\t4.5", &app_spec).unwrap();
    assert_eq!(r2.target_range, SheetCellRange::new(CellCoord::new(0, 4, 0), CellCoord::new(1, 5, 0)));
    let mv2 = as_mv(&mv_s);
    assert_eq!(
        (mv2.variables.len(), mv2.variables[4].cells.len(), mv2.variables[5].cells.len(), mv2.variables[4].role),
        (6, 4, 4, VariableRole::Continuous)
    );
    assert_approx(mv2.variables[4].cells[0].active_number().unwrap(), 1.5, 1e-12);
    assert_approx(mv2.variables[5].cells[1].active_number().unwrap(), 4.5, 1e-12);
    assert!(mv2.variables[4].cells[2].is_empty());

    let unstack_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec { unstack_group_column: Some(0), ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec { mode: PlacementMode::AppendColumns, propagate_title_header: true, ..ImportPlacementSpec::default() },
    };
    let r3 = import_tsv(&mut mv_s, "G1\t10\nG2\t20", &unstack_spec).unwrap();
    assert_eq!(r3.target_range, SheetCellRange::new(CellCoord::new(0, 6, 0), CellCoord::new(0, 7, 0)));
    let mv3 = as_mv(&mv_s);
    assert_eq!((mv3.variables[6].title.as_str(), mv3.variables[7].title.as_str()), ("G1", "G2"));
    assert_approx(mv3.variables[6].cells[0].active_number().unwrap(), 10.0, 1e-12);
    assert_approx(mv3.variables[7].cells[0].active_number().unwrap(), 20.0, 1e-12);

    // 2. Column (Single): grows both cols and rows.
    let mut col_s = col_sheet("col-append", &[("Ctrl", &[num(1.0), num(2.0), num(3.0)]), ("Drug", &[empty(), empty(), empty()])]);
    let col_rep = import_tsv(&mut col_s, "10\t20\n30\t40\n50\t60\n70\t80", &app_spec).unwrap();
    assert_eq!(col_rep.target_range, SheetCellRange::new(CellCoord::new(0, 2, 0), CellCoord::new(3, 3, 0)));
    let col_after = as_col(&col_s);
    assert_eq!(
        (
            col_after.data_sets.len(),
            col_after.row_count(),
            col_after.data_sets[2].title.as_str(),
            col_after.data_sets[3].title.as_str(),
            col_after.data_sets[0].cells.len()
        ),
        (4, 4, "C", "D", 4)
    );
    assert_approx(col_after.data_sets[2].cells[3][0].active_number().unwrap(), 70.0, 1e-12);
    assert_approx(col_after.data_sets[3].cells[1][0].active_number().unwrap(), 40.0, 1e-12);

    // 3. Column (MeanSdN, 3 subcolumns): appends cols 2 & 3 across subcolumns (4 cells -> ends at col 3, subcol 0).
    let mut rep_s = sheet(
        "col-rep-append",
        TableArchetype::Column(ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 2, 2).unwrap()),
    );
    let rep_res = import_tsv(&mut rep_s, "1\t2\t3\t4\n5\t6\t7\t8", &app_spec).unwrap();
    assert_eq!(rep_res.target_range, SheetCellRange::new(CellCoord::new(0, 2, 0), CellCoord::new(1, 3, 0)));
    let rep_after = as_col(&rep_s);
    assert_eq!(rep_after.data_sets.len(), 4);
    assert_cells_3(&rep_after.data_sets[2].cells[0], [SubExp::Num(1.0), SubExp::Num(2.0), SubExp::Num(3.0)], (1e-12, 0.0), "rep col2");
    assert_approx(rep_after.data_sets[3].cells[0][0].active_number().unwrap(), 4.0, 1e-12);
    assert_approx(rep_after.data_sets[3].cells[1][0].active_number().unwrap(), 8.0, 1e-12);
}

#[test]
fn append_columns_grows_all_remaining_archetypes() {
    let spec = clipboard_spec(PlacementMode::AppendColumns, CellCoord::new(0, 0, 0), ImportFilterSpec::default());
    let mut sheets = [
        sheet("xy-app", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).unwrap())),
        sheet("grp-app", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap())),
        sheet("cnt-app", TableArchetype::Contingency(ContingencyTable::new(1, 2).unwrap())),
        sheet("srv-app", TableArchetype::Survival(SurvivalTable::new(1, 2).unwrap())),
        sheet("pow-app", TableArchetype::PartsOfWhole(PartsOfWholeTable::new(1, 2).unwrap())),
        sheet("nst-app", TableArchetype::Nested(NestedTable::new(1, 2, 2).unwrap())),
    ];
    for s in &mut sheets {
        let is_two_sub = matches!(s.archetype, TableArchetype::Grouped(_) | TableArchetype::Nested(_));
        let report = import_tsv(s, "11\t22\n33\t44", &spec).unwrap();
        assert_eq!((s.archetype.column_count(), report.target_range.start.col), (if is_two_sub { 2 } else { 3 }, 1), "{:?}", s.kind());
        assert_approx(s.cell(CellCoord::new(0, 1, 0)).unwrap().active_number().unwrap(), 11.0, 1e-12);
        let corner = if is_two_sub { CellCoord::new(1, 1, 1) } else { CellCoord::new(1, 2, 0) };
        assert_approx(s.cell(corner).unwrap().active_number().unwrap(), 44.0, 1e-12);
    }
}

#[test]
fn import_pipeline_enforces_anchor_overflow_and_max_rows_across_archetypes() {
    let mut xy_s = sheet("xy-ovf", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 1, 1).unwrap()));
    for bad_anchor in [CellCoord::new(usize::MAX, 0, 0), CellCoord::new(0, usize::MAX, 0), CellCoord::new(0, 0, usize::MAX)] {
        let spec = clipboard_spec(PlacementMode::Overwrite, bad_anchor, ImportFilterSpec::default());
        assert!(matches!(import_csv(&mut xy_s, "42", &spec), Err(TransformError::InvalidSpec { .. })));
        assert_eq!(xy_s.archetype.row_count(), 1);
    }

    let row_cap_spec = clipboard_spec(PlacementMode::Overwrite, CellCoord::new(MAX_ROWS, 0, 0), ImportFilterSpec::default());
    let mut mv_cap = mv_empty("mv-rows", 2, 1);
    assert!(matches!(import_csv(&mut mv_cap, "42", &row_cap_spec), Err(TransformError::Doc(DocError::TooManyRows { .. }))));
    assert_eq!((as_mv(&mv_cap).row_titles.len(), as_mv(&mv_cap).variables[0].cells.len()), (1, 1));

    let mut nst_cap = sheet("nst-rows", TableArchetype::Nested(NestedTable::new(2, 2, 1).unwrap()));
    assert!(matches!(import_csv(&mut nst_cap, "42", &row_cap_spec), Err(TransformError::Doc(DocError::TooManyRows { .. }))));
    assert_eq!((as_nested(&nst_cap).row_titles.len(), as_nested(&nst_cap).groups[0].cells.len()), (1, 1));
}

#[test]
fn import_pipeline_enforces_cell_budget_before_allocation_across_archetypes() {
    assert_eq!(MAX_IMPORT_CELLS, 10_000_000);
    let cases = [
        (sheet("grp-budget", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(16), 25, 1).unwrap())), 30_000),
        (
            sheet(
                "col-budget",
                TableArchetype::Column(ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 120, 1).unwrap()),
            ),
            30_000,
        ),
        (mv_empty("mv-budget", 20, 1), 600_000),
        (sheet("nst-budget", TableArchetype::Nested(NestedTable::new(5, 10, 1).unwrap())), 250_000),
    ];
    for (mut s, anchor_row) in cases {
        let spec = clipboard_spec(PlacementMode::Overwrite, CellCoord::new(anchor_row, 0, 0), ImportFilterSpec::default());
        assert!(matches!(import_csv(&mut s, "42", &spec), Err(TransformError::InvalidSpec { .. })), "{:?}", s.kind());
        assert_eq!(s.archetype.row_count(), 1);
        match &s.archetype {
            TableArchetype::MultipleVariables(mv) => assert_eq!((mv.row_titles.len(), mv.variables[0].cells.len()), (1, 1)),
            TableArchetype::Nested(nst) => assert_eq!((nst.row_titles.len(), nst.groups[0].cells.len()), (1, 1)),
            _ => {}
        }
    }
}

#[test]
fn import_pipeline_enforces_atomic_append_columns_and_unstack_cell_budget() {
    let mut mv_append = mv_empty("mv-app-cap", 10, 1);
    let append_spec = clipboard_spec(PlacementMode::AppendColumns, CellCoord::new(600_000, 0, 0), ImportFilterSpec::default());
    assert!(matches!(import_csv(&mut mv_append, "1,2,3,4,5,6,7,8,9,10", &append_spec), Err(TransformError::InvalidSpec { .. })));
    assert_eq!((mv_append.archetype.row_count(), mv_append.archetype.column_count()), (1, 10));

    let mut unstack_csv = (0..1_001).map(|g| format!("G{g},1\n")).collect::<String>();
    unstack_csv.push_str(&"G0,2\n".repeat(10_005));
    let mut unstack_target = col_empty("unstack-cap", 1, 1);
    let unstack_spec = clipboard_spec(
        PlacementMode::Overwrite,
        CellCoord::new(0, 0, 0),
        ImportFilterSpec { unstack_group_column: Some(0), ..ImportFilterSpec::default() },
    );
    assert!(matches!(import_csv(&mut unstack_target, &unstack_csv, &unstack_spec), Err(TransformError::InvalidSpec { .. })));
}

#[test]
fn r1_s04_01_ragged_tables_and_mismatched_context_handled_cleanly() {
    let e5 = empty_cells(5);
    let mut mv = mv_table(&[("A", &[num(1.0), num(2.0), num(3.0), num(4.0), num(5.0)]), ("B", &e5), ("C", &e5)]);
    mv.variables[1].cells.truncate(2);
    let ragged_sheet = raw_sheet("mv-rag", TableArchetype::MultipleVariables(mv.clone()));

    assert!(is_invalid_cell(evaluate_formula_for_sheet("abs(B)", &ragged_sheet, None)));

    let mut calc_mv = mv;
    calc_mv.variables[2].set_calculated_formula("=B + 1");
    assert!(is_invalid_cell(evaluate_calculated_variables(&mut calc_mv, None)));
    assert_eq!(calc_mv.variables[0].cells.len(), 5, "ragged DAG validation must fail before shrinking columns");

    let tctx = TransformContext::default();
    assert!(is_invalid_cell(execute_transpose(&ragged_sheet, true, &tctx)));
    assert!(is_invalid_cell(execute_prune(&ragged_sheet, PruneMethod::EveryNth(2), &tctx)));
    assert!(is_invalid_cell(execute_select(&ragged_sheet, &[SelectRule::ExcludeMissing], TableKind::Column, &tctx)));

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    grp.data_sets[1].cells.truncate(1);
    assert!(is_invalid_cell(execute_transpose(&raw_sheet("grp-rag", TableArchetype::Grouped(grp)), false, &tctx)));

    // Defense-in-depth: custom FormulaContext returning short column vectors never panics.
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
                0 => Some([1.0, 2.0, 3.0, 4.0, 5.0].into_iter().map(FormulaValue::Number).collect()),
                1 => Some([-2.0, 4.0].into_iter().map(FormulaValue::Number).collect()),
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
        let out = eval_column_expr(&parse_formula(formula).unwrap(), &RaggedContext).unwrap();
        assert_eq!(out.len(), 5, "formula `{formula}` changed row length");
    }
}

#[test]
fn r1_s05_02_import_transpose_and_x_mode_enforce_budgets() {
    let mut wide_first_row = (0..4_000).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    wide_first_row.push('\n');
    for r in 0..4_000 {
        wide_first_row.push_str(&format!("{r}\n"));
    }
    let mut target = col_empty("imp-rag", 1, 1);
    let tr_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { transpose: true, ..ImportPlacementSpec::default() },
    };
    assert!(matches!(import_csv(&mut target, &wide_first_row, &tr_spec), Err(TransformError::InvalidSpec { .. })));

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
    let report = import_csv(&mut target, &bounded_csv, &bounded_spec).unwrap();
    assert_eq!((report.rows_placed, report.cols_placed), (1_000, 51));
    assert_eq!(target.cell(CellCoord::new(999, 1, 0)).and_then(Cell::active_number), Some(999.0));
    assert_eq!(target.cell(CellCoord::new(0, 51, 0)).and_then(Cell::active_number), Some(1_049.0));

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 1).unwrap();
    xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(usize::MAX) };
    let mut bad_xy = raw_sheet("bad-xy", TableArchetype::Xy(xy));
    let simple_spec = clipboard_spec(PlacementMode::Overwrite, CellCoord::new(0, 0, 0), ImportFilterSpec::default());
    assert!(matches!(import_csv(&mut bad_xy, "1,2\n3,4\n", &simple_spec), Err(TransformError::Doc(_))));
}
