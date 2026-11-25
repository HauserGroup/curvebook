//! Integration tests for `R6.3`: window clauses (`OVER`, `WHERE`, `ORDER BY`, `ROWS`),
//! expanded conditional aggregations, DAG dependency tracking, and `SelectAndTransform`
//! derived column-transform formulas (`docs/implementation/data-tables-and-transforms.md` §4.1, §5.4).

use curvebook_doc::{DataSheet, DerivedColumnFormula, InfoSheet, MultipleVariablesTable, TableArchetype, TableKind, TransformAnalysisSpec};
use curvebook_transform::{
    DependencyTarget, TransformContext, TransformError, evaluate_calculated_variables, evaluate_data_sheet_calculated_variables,
    evaluate_formula_for_sheet, execute_select_with_derived, execute_transform, extract_dependencies, parse_formula, validate_no_formula_cycle,
};

use crate::common::{
    ExpectVal::{self, Missing, Num},
    FormulaInfoCase, RowCheck, as_mv, as_xy, assert_approx, assert_formula_first, assert_formula_rows, empty, excl, info_with_numbers, mv_sheet,
    mv_table, num, sheet, txt, xy_single,
};

fn build_assay_mv_sheet() -> (DataSheet, InfoSheet) {
    // 8 rows across Batch A (rows 0..4) and Batch B (rows 4..8); row 3 Excluded Response=999, row 7 Empty Note.
    let s = mv_sheet(
        "mv-r63",
        &[
            ("Batch", &[txt("A"), txt("A"), txt("A"), txt("A"), txt("B"), txt("B"), txt("B"), txt("B")]),
            ("Role", &[txt("Ctrl"), txt("Ctrl"), txt("Trt"), txt("Trt"), txt("Ctrl"), txt("Trt"), txt("Trt"), txt("Trt")]),
            ("Time", &[num(3.0), num(1.0), num(2.0), num(4.0), num(2.0), num(1.0), num(3.0), num(4.0)]),
            ("Response", &[num(20.0), num(10.0), num(45.0), excl(999.0), num(100.0), num(150.0), num(250.0), num(300.0)]),
            ("Flag", &[num(1.0), num(1.0), num(1.0), num(0.0), num(1.0), num(0.0), num(1.0), num(1.0)]),
            ("Note", &[txt("ok"), txt("ok"), txt("hi"), txt("bad"), txt("ok"), txt("skip"), txt("hi"), empty()]),
        ],
    );
    (s, info_with_numbers("info-r63", &[("WinWidth", 1.0)]))
}

#[test]
fn r63_window_over_where_aggregates_and_non_destructive_filtering() {
    let (s, _) = build_assay_mv_sheet();

    // 1. All three non-destructive WHERE / FILTER window syntaxes emit identical per-batch Ctrl means across all 8 rows.
    let ctrl_rows: Vec<(usize, ExpectVal)> = (0..8).map(|r| (r, Num(if r < 4 { 15.0 } else { 100.0 }))).collect();
    for expr in [
        "mean(Response) FILTER (WHERE Role == \"Ctrl\") OVER (PARTITION BY Batch)",
        "mean(Response) WHERE Role == \"Ctrl\" OVER (PARTITION BY Batch)",
        "mean(Response) OVER (PARTITION BY Batch WHERE Role == \"Ctrl\")",
    ] {
        assert_eq!(evaluate_formula_for_sheet(expr, &s, None).unwrap().len(), 8);
        assert_formula_rows(&s, None, expr, &ctrl_rows, 1e-12);
    }

    // 2. Arithmetic composition + all aggregate functions across partitions.
    let expected_sd_a = ((5.0_f64.powi(2) + 15.0_f64.powi(2) + 20.0_f64.powi(2)) / 2.0).sqrt();
    let cases: &[(&str, &[RowCheck])] = &[
        (
            "Response / mean(Response) FILTER (WHERE Role == \"Ctrl\") OVER (PARTITION BY Batch)",
            &[(0, Num(20.0 / 15.0)), (1, Num(10.0 / 15.0)), (2, Num(45.0 / 15.0)), (3, Missing), (5, Num(1.5)), (7, Num(3.0))],
        ),
        ("sum(Response) OVER (PARTITION BY Batch)", &[(0, Num(75.0)), (4, Num(800.0))]),
        ("count(Response) OVER (PARTITION BY Batch)", &[(0, Num(3.0)), (4, Num(4.0))]),
        ("sd(Response) OVER (PARTITION BY Batch)", &[(1, Num(expected_sd_a))]),
        ("sem(Response) OVER (PARTITION BY Batch)", &[(1, Num(expected_sd_a / 3.0_f64.sqrt()))]),
        ("median(Response) OVER (PARTITION BY Batch)", &[(2, Num(20.0)), (5, Num(200.0))]),
        ("iqr(Response) OVER (PARTITION BY Batch)", &[(0, Num(17.5)), (6, Num(125.0))]),
        ("max_col(Response) OVER (PARTITION BY Batch) - min_col(Response) OVER (PARTITION BY Batch)", &[(0, Num(35.0)), (7, Num(200.0))]),
    ];
    for &(expr, checks) in cases {
        assert_formula_rows(&s, None, expr, checks, 1e-12);
    }
}

#[test]
fn r63_window_order_by_and_rows_frame_bounds() {
    let (s, info) = build_assay_mv_sheet();
    let cases: &[FormulaInfoCase<'_>] = &[
        (
            "sum(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS UNBOUNDED PRECEDING)",
            false,
            &[(1, Num(10.0)), (2, Num(55.0)), (0, Num(75.0)), (3, Num(75.0)), (5, Num(150.0)), (4, Num(250.0)), (6, Num(500.0)), (7, Num(800.0))],
        ),
        (
            "mean(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS BETWEEN Info(\"WinWidth\") PRECEDING AND 1 FOLLOWING)",
            true,
            &[(5, Num(125.0)), (4, Num(500.0 / 3.0)), (6, Num(650.0 / 3.0)), (7, Num(275.0))],
        ),
        (
            "sum(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS BETWEEN 2 PRECEDING AND 1 PRECEDING)",
            false,
            &[(5, Missing), (4, Num(150.0)), (6, Num(250.0)), (7, Num(350.0))],
        ),
        ("count(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS BETWEEN 2 PRECEDING AND 1 PRECEDING)", false, &[(5, Num(0.0))]),
        (
            "max_col(Response) OVER (PARTITION BY Batch ORDER BY Time ASC ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING)",
            false,
            &[(1, Num(45.0)), (2, Num(45.0)), (0, Num(20.0)), (3, Missing)],
        ),
    ];
    for &(expr, use_info, checks) in cases {
        assert_formula_rows(&s, use_info.then_some(&info), expr, checks, 1e-12);
    }

    // SQL default RANGE peer behavior when ORDER BY is present without ROWS:
    let ties = mv_sheet("ties", &[("T", &[num(1.0), num(2.0), num(2.0), num(3.0)]), ("V", &[num(10.0), num(20.0), num(30.0), num(40.0)])]);
    assert_formula_rows(&ties, None, "sum(V) OVER (ORDER BY T)", &[(0, Num(10.0)), (1, Num(60.0)), (2, Num(60.0)), (3, Num(100.0))], 1e-12);
}

#[test]
fn r63_row_wise_window_functions_center_standardize_rank_cum_lag_lead_diff_moving_avg() {
    let (s, _) = build_assay_mv_sheet();
    let mean_b_flag: f64 = 650.0 / 3.0;
    let sd_a = 325.0_f64.sqrt();

    assert_eq!(
        evaluate_formula_for_sheet("standardize(Response) OVER (PARTITION BY Batch)", &s, None).unwrap(),
        evaluate_formula_for_sheet("zscore(Response) OVER (PARTITION BY Batch)", &s, None).unwrap()
    );

    let cases: &[(&str, &[RowCheck])] = &[
        (
            "center(Response) WHERE Flag == 1 OVER (PARTITION BY Batch)",
            &[(0, Num(-5.0)), (1, Num(-15.0)), (2, Num(20.0)), (3, Missing), (4, Num(100.0 - mean_b_flag)), (5, Missing)],
        ),
        ("standardize(Response) OVER (PARTITION BY Batch)", &[(0, Num(-5.0 / sd_a)), (2, Num(20.0 / sd_a))]),
        ("center(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)", &[(5, Num(0.0)), (4, Num(-25.0))]),
        ("row_number() OVER (PARTITION BY Batch ORDER BY Time ASC)", &[(1, Num(1.0)), (2, Num(2.0)), (0, Num(3.0)), (3, Num(4.0))]),
        ("rank(Response) OVER (PARTITION BY Batch ORDER BY Response DESC)", &[(2, Num(1.0)), (0, Num(2.0)), (1, Num(3.0)), (3, Missing)]),
        ("dense_rank() OVER (PARTITION BY Batch ORDER BY Role ASC)", &[(0, Num(1.0)), (1, Num(1.0)), (2, Num(2.0))]),
        (
            "cummean(Response) WHERE Flag == 1 OVER (PARTITION BY Batch ORDER BY Time)",
            &[(4, Num(100.0)), (5, Missing), (6, Num(175.0)), (7, Num(mean_b_flag))],
        ),
        ("lag(Response, 1) WHERE Flag == 1 OVER (PARTITION BY Batch ORDER BY Time)", &[(4, Missing), (5, Missing), (6, Num(100.0)), (7, Num(250.0))]),
        ("lead(Response, 1) OVER (PARTITION BY Batch ORDER BY Time)", &[(5, Num(100.0)), (4, Num(250.0)), (6, Num(300.0)), (7, Missing)]),
        ("diff(Response, 1) OVER (PARTITION BY Batch ORDER BY Time)", &[(5, Missing), (4, Num(-50.0)), (6, Num(150.0)), (7, Num(50.0))]),
        (
            "moving_avg(Response, 2) OVER (PARTITION BY Batch ORDER BY Time)",
            &[(1, Missing), (2, Num(27.5)), (0, Num(32.5)), (5, Missing), (4, Num(125.0))],
        ),
    ];
    for &(expr, checks) in cases {
        assert_formula_rows(&s, None, expr, checks, 1e-12);
    }
}

#[test]
fn r63_conditional_aggregates_spreadsheet_parity_and_window_composition() {
    let (s, _) = build_assay_mv_sheet();
    let expected_vals = [20.0, 10.0, 45.0, 100.0, 250.0, 300.0];
    let n = expected_vals.len() as f64;
    let sum: f64 = expected_vals.iter().sum();
    let mean = sum / n;
    let sd = (expected_vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    let sem = sd / n.sqrt();

    for (expr, exp) in [
        ("minif(Response, Flag == 1)", 10.0),
        ("minif(Flag == 1, Response)", 10.0),
        ("maxif(Response, Flag == 1)", 300.0),
        ("stdevif(Response, Flag == 1)", sd),
        ("sdif(Response, Flag == 1)", sd),
        ("stderrif(Response, Flag == 1)", sem),
        ("semif(Response, Flag == 1)", sem),
        ("sumif(Response, Flag == 1)", sum),
        ("meanif(Response, Flag == 1)", mean),
        ("countaif(Note, Flag == 1)", 5.0),
        ("countif(Response, Flag == 1)", 6.0),
        ("countif(Flag == 1)", 6.0),
    ] {
        assert_formula_first(&s, None, expr, Num(exp), 1e-12);
    }

    // Insufficient samples (< 2) returns Missing for stdevif/stderrif.
    for expr in ["stdevif(Response, Role == \"Ctrl\") OVER (PARTITION BY Batch)", "stderrif(Response, Role == \"Ctrl\") OVER (PARTITION BY Batch)"] {
        assert_formula_rows(&s, None, expr, &[(4, Missing)], 1e-12);
    }

    // Full composition with OVER, WHERE, and arithmetic.
    let mean_b_flag: f64 = 650.0 / 3.0;
    let sd_b_flag = (((mean_b_flag - 100.0).powi(2) + (250.0 - mean_b_flag).powi(2) + (300.0 - mean_b_flag).powi(2)) / 2.0).sqrt();
    let composed_expr = "(maxif(Response, Role == \"Trt\") OVER (PARTITION BY Batch) - minif(Response, Role == \"Trt\") OVER (PARTITION BY Batch)) + stdevif(Response, Flag == 1) OVER (PARTITION BY Batch)";
    assert_formula_rows(&s, None, composed_expr, &[(4, Num(150.0 + sd_b_flag))], 1e-10);
}

#[test]
fn r63_dag_tracks_partition_order_filter_and_frame_dependencies() {
    let ast = parse_formula("=mean(Val) FILTER (WHERE Active == 1) OVER (PARTITION BY [Grp], Info(\"Split\") ORDER BY [SortKey] DESC ROWS BETWEEN Info(\"W\") PRECEDING AND CURRENT ROW)").unwrap();
    let deps = extract_dependencies(&ast);
    for dep in [
        DependencyTarget::ColumnName(String::from("Val")),
        DependencyTarget::ColumnName(String::from("Active")),
        DependencyTarget::ColumnName(String::from("Grp")),
        DependencyTarget::ColumnName(String::from("SortKey")),
        DependencyTarget::InfoConstant(String::from("Split")),
        DependencyTarget::InfoConstant(String::from("W")),
    ] {
        assert!(deps.contains(&dep), "missing dep {dep:?} in {deps:?}");
    }

    let mut mv = mv_table(&[
        ("Running", &[empty(), empty(), empty(), empty()]),
        ("Raw", &[num(10.0), num(20.0), num(3.0), num(30.0)]),
        ("GrpCalc", &[empty(), empty(), empty(), empty()]),
        ("OrdCalc", &[empty(), empty(), empty(), empty()]),
        ("Keep", &[empty(), empty(), empty(), empty()]),
    ]);
    mv.variables[0].set_calculated_formula("=cumsum(Raw) WHERE Keep == 1 OVER (PARTITION BY GrpCalc ORDER BY OrdCalc ASC)");
    mv.variables[2].set_calculated_formula("=if($index < 2, \"G1\", \"G2\")");
    mv.variables[3].set_calculated_formula("=-$index");
    mv.variables[4].set_calculated_formula("=if(Raw > 5, 1, 0)");

    assert_eq!(evaluate_calculated_variables(&mut mv, None).unwrap().last().copied(), Some(0));
    assert_approx(mv.variables[0].cells[1].active_number().unwrap(), 20.0, 1e-12);
    assert_approx(mv.variables[0].cells[0].active_number().unwrap(), 30.0, 1e-12);
    assert!(mv.variables[0].cells[2].is_empty());
    assert_approx(mv.variables[0].cells[3].active_number().unwrap(), 30.0, 1e-12);

    for (col_idx, formula) in
        [(2, "=mean(Raw) OVER (PARTITION BY Running)"), (3, "=lag(Raw) OVER (ORDER BY Running)"), (4, "=sum(Raw) FILTER (WHERE Running > 0)")]
    {
        assert!(matches!(validate_no_formula_cycle(&mv, col_idx, formula), Err(TransformError::CircularDependency { .. })), "{formula}");
    }

    let mut s = sheet("cyc-self", TableArchetype::MultipleVariables(mv));
    crate::common::as_mv_mut(&mut s).variables[0].set_calculated_formula("=mean(Raw) OVER (PARTITION BY Running)");
    assert!(matches!(evaluate_data_sheet_calculated_variables(&mut s, None), Err(TransformError::CircularDependency { .. })));
}

#[test]
fn r63_contextual_keywords_diagnostics_and_zero_row_safety() {
    let s = mv_sheet(
        "kw-mv",
        &[
            ("Order", &[num(3.0), num(2.0), num(1.0)]),
            ("Filter", &[num(1.0), num(1.0), num(0.0)]),
            ("Rows", &[num(10.0), num(20.0), num(30.0)]),
            ("Over", &[num(1.0), num(1.0), num(1.0)]),
        ],
    );
    assert_formula_rows(
        &s,
        None,
        "sum(Rows) WHERE Filter == 1 OVER (PARTITION BY Over ORDER BY Order ASC)",
        &[(2, Num(0.0)), (1, Num(20.0)), (0, Num(30.0))],
        1e-12,
    );
    assert_formula_rows(
        &s,
        None,
        "sum(Rows) WHERE Filter == 1 OVER (PARTITION BY Over ORDER BY Order DESC)",
        &[(0, Num(10.0)), (1, Num(30.0)), (2, Num(30.0))],
        1e-12,
    );

    for bad_syntax in [
        "mean(Rows) OVER (PARTITION Over)",
        "mean(Rows) OVER (ORDER Order)",
        "mean(Rows) OVER (ROWS BETWEEN CURRENT ROW AND 1 PRECEDING)",
        "mean(Rows) OVER (ROWS BETWEEN UNBOUNDED FOLLOWING AND CURRENT ROW)",
        "mean(Rows) OVER (ROWS BETWEEN 1 PRECEDING AND 2 PRECEDING)",
        "mean(Rows) OVER (ROWS BETWEEN -1 PRECEDING AND CURRENT ROW)",
        "mean(Rows) OVER (ROWS 1 FOLLOWING)",
        "mean(Rows) OVER (PARTITION BY Over PARTITION BY Rows)",
        "mean(Rows) FILTER (WHERE Filter == 1) WHERE Filter == 1",
        "col(\"Rows\") OVER ()",
        "mean(Rows) OVER (",
    ] {
        assert!(parse_formula(bad_syntax).is_err(), "expected ParseError for `{bad_syntax}`");
    }

    for bad_eval in ["sin(Rows) OVER ()", "rank()"] {
        assert!(matches!(evaluate_formula_for_sheet(bad_eval, &s, None), Err(TransformError::InvalidSpec { .. })), "{bad_eval}");
    }

    let empty_s = sheet("empty", TableArchetype::MultipleVariables(MultipleVariablesTable::new(2, 0).unwrap()));
    for expr in ["lag(A, 1) OVER (PARTITION BY B ORDER BY A)", "mean(A) OVER (PARTITION BY B ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING)"] {
        assert!(evaluate_formula_for_sheet(expr, &empty_s, None).unwrap().is_empty(), "{expr}");
    }
}

#[test]
fn r63_select_and_transform_derived_columns_windowed_conditional_and_chained() {
    let (s, info) = build_assay_mv_sheet();
    let ctx = TransformContext { info_sheet: Some(&info), selected_data_sets: vec![2], ..Default::default() };

    // 1. Derived columns referencing non-selected source columns + projecting to XY.
    let spec = TransformAnalysisSpec::SelectAndTransform {
        filter_formula: Some(String::from("Flag == 1")),
        target_archetype: TableKind::Xy,
        derived_columns: vec![
            DerivedColumnFormula::new("FoldVsCtrl", "Response / mean(Response) FILTER (WHERE Role == \"Ctrl\") OVER (PARTITION BY Batch)"),
            DerivedColumnFormula::new("FoldZ", "zscore(FoldVsCtrl) OVER (PARTITION BY Batch)"),
        ],
    };
    let out = execute_transform(&s, &spec, &ctx).unwrap();
    let xy = as_xy(&out.data_sheet);
    assert_eq!(
        (xy.x_title.as_str(), xy.data_sets.len(), xy.data_sets[0].title.as_str(), xy.data_sets[1].title.as_str(), xy.row_count()),
        ("Time", 2, "FoldVsCtrl", "FoldZ", 6)
    );

    // 2. filter_formula referencing a derived column.
    let spec_filter = TransformAnalysisSpec::SelectAndTransform {
        filter_formula: Some(String::from("abs(RespZ) <= 1.0")),
        target_archetype: TableKind::MultipleVariables,
        derived_columns: vec![DerivedColumnFormula::new("RespZ", "zscore(Response) OVER (PARTITION BY Batch)")],
    };
    let mv_out = execute_transform(&s, &spec_filter, &TransformContext::default()).unwrap();
    let out_mv = as_mv(&mv_out.data_sheet);
    assert_eq!(out_mv.variables.len(), 7);
    assert!(out_mv.row_count() < 8);
    assert!((0..out_mv.row_count()).all(|r| out_mv.variables[6].cells[r].active_number().unwrap().abs() <= 1.0));

    // 3. In-place column transform on XY source table.
    let mut xy_s = xy_single("xy-src", &[1.0, 2.0, 3.0, 4.0], &[&[10.0, 20.0, 30.0, 40.0]]);
    if let TableArchetype::Xy(t) = &mut xy_s.archetype {
        t.x_title = String::from("Time");
        t.data_sets[0].title = String::from("Signal");
    }
    let xy_derived = execute_select_with_derived(
        &xy_s,
        &[],
        &[DerivedColumnFormula::new("Signal", "Signal - minif(Signal, X > 1)")],
        TableKind::Xy,
        &TransformContext::default(),
    )
    .unwrap();
    let xy_after = as_xy(&xy_derived.data_sheet);
    assert_eq!(xy_after.data_sets.len(), 1);
    for (r, exp) in [(0, -10.0), (1, 0.0), (3, 20.0)] {
        assert_approx(xy_after.data_sets[0].cells[r][0].active_number().unwrap(), exp, 1e-12);
    }
}

#[test]
fn audit_14_conditional_aggregates_canonical_arg_order_with_boolean_values() {
    // Regression for Audit #14: canonical `(values, condition)` order even when `values` is 0/1 and `condition` has values > 1.
    let s = mv_sheet("audit14-mv", &[("Reading", &[num(1.0), num(1.0), num(0.0)]), ("Flag", &[num(2.0), num(0.0), num(2.0)])]);
    for (expr, expected) in [
        ("sumif(Reading, Flag)", 1.0),
        ("sumif(Reading, Flag != 0)", 1.0),
        ("countif(Reading, Flag)", 2.0),
        ("countif(Reading, Flag != 0)", 2.0),
        ("countaif(Reading, Flag)", 2.0),
        ("countaif(Reading, Flag != 0)", 2.0),
        ("meanif(Reading, Flag)", 0.5),
        ("meanif(Reading, Flag != 0)", 0.5),
        ("minif(Reading, Flag)", 0.0),
        ("minif(Reading, Flag != 0)", 0.0),
        ("maxif(Reading, Flag)", 1.0),
        ("maxif(Reading, Flag != 0)", 1.0),
        ("stdevif(Reading, Flag)", 0.5_f64.sqrt()),
        ("sdif(Reading, Flag != 0)", 0.5_f64.sqrt()),
        ("stderrif(Reading, Flag)", 0.5),
        ("semif(Reading, Flag != 0)", 0.5),
        ("sumif(Flag, Reading)", 2.0),
        ("sumif(Flag != 0, Reading)", 1.0),
    ] {
        assert_formula_first(&s, None, expr, Num(expected), 1e-12);
    }
}
