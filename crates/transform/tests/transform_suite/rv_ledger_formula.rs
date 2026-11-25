//! Regression tests for the 2026-11-16 milestone review ledger (`transform-formula` slice:
//! RV-19, RV-25, RV-55, RV-58, RV-62, RV-64 formula/import, and Savitzky–Golay boundary order).

use curvebook_doc::{
    Cell, CellCoord, CellValue, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind, LinkSource, PlacementMode, ResultCell,
    ResultTab, ResultTabKind, SheetCellRange, SheetId, SimulationNoise, TableArchetype, TableKind,
};
use curvebook_transform::{
    AllocationScheme, BinaryOp, DependencyTarget, Expr, FormulaValue, GroupAllocationWeight, ImportPipelineOptions, ImportSourcePayload, ScalarEnv,
    SelectRule, SmoothMethod, SubjectRandomizationConfig, TransformContext, TransformError, UnaryOp, apply_import_link_pipeline, eval_scalar_expr,
    evaluate_calculated_variables, evaluate_data_sheet_calculated_variables, evaluate_equation, evaluate_formula_for_sheet, execute_select,
    extract_dependencies, parse_formula, randomize_subjects_to_groups, simulate_xy_sheet, smooth_xy, validate_no_formula_cycle,
};

use crate::common::{
    ExpectVal, as_mv, as_mv_mut, as_xy, assert_approx, assert_formula_first, clipboard_spec, col_empty, empty, excl, mv_empty, mv_sheet, mv_table,
    num, result_sheet_with_tab, sheet, str_vec,
};

#[test]
fn rv_19_permuted_block_rejects_huge_block_sizes_without_abort() {
    let two_arms = vec![GroupAllocationWeight::new("Control"), GroupAllocationWeight::new("Treatment")];
    let one_arm = vec![GroupAllocationWeight::new("OnlyArm")];

    for (id, n_subj, arms, block_sizes) in [
        ("huge-1e9", 12, two_arms.clone(), vec![1_000_000_000_usize]),
        ("huge-1e15", 12, two_arms.clone(), vec![1_000_000_000_000_000_usize]),
        ("huge-max", 12, two_arms.clone(), vec![(usize::MAX / 2) & !1_usize]),
        ("single-1e15", 8, one_arm, vec![1_000_000_000_000_000_usize]),
        ("mixed-huge", 8, two_arms.clone(), vec![4, 1_000_000_000]),
    ] {
        let cfg = SubjectRandomizationConfig::from_subject_count(id, id, n_subj, arms, AllocationScheme::PermutedBlock { block_sizes }, 2026);
        assert!(matches!(randomize_subjects_to_groups(&cfg), Err(TransformError::InvalidSpec { .. })), "{id}");
    }

    let ok_cfg = SubjectRandomizationConfig::from_subject_count(
        "rv19-ok",
        "Valid",
        12,
        two_arms,
        AllocationScheme::PermutedBlock { block_sizes: vec![4, 6] },
        42,
    );
    assert_eq!(randomize_subjects_to_groups(&ok_cfg).unwrap().data_sheet.archetype.row_count(), 12);
}

#[test]
fn rv_25_simulate_xy_handles_isolated_poles_and_large_poisson_lambda() {
    // 1. Michaelis-Menten touching x = -Km (-2.0) emits Empty at the pole and finite Y elsewhere.
    let params = vec![(String::from("Vmax"), 100.0), (String::from("Km"), 2.0)];
    let mm_sheet = simulate_xy_sheet("rv25-mm", "MM With Pole", 11, 5, 2, -2.0, 6.0, "michaelis_menten", &params, SimulationNoise::None).unwrap();
    let mm_xy = as_xy(&mm_sheet);
    assert_eq!((mm_xy.row_count(), mm_xy.x_value_at(0)), (5, Some(-2.0)));
    assert_eq!((&mm_xy.data_sets[0].cells[0][0].value, &mm_xy.data_sets[0].cells[0][1].value), (&CellValue::Empty, &CellValue::Empty));
    for (r, exp) in [(1, 0.0), (2, 50.0), (3, 200.0 / 3.0), (4, 75.0)] {
        assert_approx(mm_xy.data_sets[0].cells[r][0].active_number().unwrap(), exp, 1e-12);
    }

    // Custom formula with interior pole at x = 1.0.
    let custom = simulate_xy_sheet("rv25-cust", "Custom Pole", 3, 3, 1, 0.0, 2.0, "expr: 10 / (X - 1)", &[], SimulationNoise::None).unwrap();
    let c_xy = as_xy(&custom);
    assert_approx(c_xy.data_sets[0].cells[0][0].active_number().unwrap(), -10.0, 1e-12);
    assert_eq!(c_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
    assert_approx(c_xy.data_sets[0].cells[2][0].active_number().unwrap(), 10.0, 1e-12);

    // Scalar pole, all-pole sheet, and unknown equation still error.
    assert!(matches!(evaluate_equation("michaelis_menten", -2.0, &params), Err(TransformError::EvalError { .. })));
    assert!(matches!(
        simulate_xy_sheet("rv25-all-pole", "All Pole", 1, 1, 2, -2.0, -2.0, "michaelis_menten", &params, SimulationNoise::None),
        Err(TransformError::EvalError { .. })
    ));
    assert!(matches!(
        simulate_xy_sheet("rv25-bad-eq", "Unknown", 1, 3, 1, 0.0, 2.0, "no_such_model", &[], SimulationNoise::None),
        Err(TransformError::UnknownEquation { .. })
    ));

    // 2. Large Poisson lambda (> 1e15) succeeds via asymptotic normal count approximation.
    let lambda_big = 4.0e16_f64;
    let pois = simulate_xy_sheet(
        "rv25-pois",
        "Large Poisson",
        99,
        1,
        256,
        1.0,
        1.0,
        "linear",
        &[(String::from("slope"), 0.0), (String::from("intercept"), lambda_big)],
        SimulationNoise::Poisson,
    )
    .unwrap();
    let draws: Vec<f64> = as_xy(&pois).data_sets[0].cells[0].iter().filter_map(Cell::active_number).collect();
    assert_eq!(draws.len(), 256);
    assert!(draws.iter().all(|&v| v.is_finite() && v >= 0.0 && v.fract() == 0.0));
    let mean_draw = draws.iter().sum::<f64>() / (draws.len() as f64);
    let se_mean = lambda_big.sqrt() / (draws.len() as f64).sqrt();
    assert!((mean_draw - lambda_big).abs() <= 5.0 * se_mean, "mean {mean_draw} drifted from {lambda_big} (SE={se_mean})");
}

#[test]
fn rv_55_column_e_and_named_constants_resolve_and_recalculate_in_dag() {
    let mut mv = mv_table(&[
        ("A", &[empty(), empty(), empty()]),
        ("B", &[num(1.0), num(2.0), num(3.0)]),
        ("C", &[empty(), empty(), empty()]),
        ("D", &[empty(), empty(), empty()]),
        ("E", &[num(10.0), num(20.0), num(30.0)]),
    ]);

    assert_eq!(extract_dependencies(&parse_formula("=E*2").unwrap()), vec![DependencyTarget::ColumnName(String::from("E"))]);
    assert!(matches!(validate_no_formula_cycle(&mv, 4, "=E + 1"), Err(TransformError::CircularDependency { .. })));
    validate_no_formula_cycle(&mv, 0, "=E*2").unwrap();
    mv.variables[0].set_calculated_formula("=E*2");

    let mut s = sheet("rv55-mv", TableArchetype::MultipleVariables(mv));
    evaluate_data_sheet_calculated_variables(&mut s, None).unwrap();
    for (r, exp) in [20.0, 40.0, 60.0].into_iter().enumerate() {
        assert_approx(as_mv(&s).variables[0].cells[r].active_number().unwrap(), exp, 1e-12);
    }

    // Mutate column E and verify recalculation updates column A.
    as_mv_mut(&mut s).variables[4].cells = vec![num(7.5), num(12.5), num(25.0)];
    evaluate_data_sheet_calculated_variables(&mut s, None).unwrap();
    for (r, exp) in [15.0, 25.0, 50.0].into_iter().enumerate() {
        assert_approx(as_mv(&s).variables[0].cells[r].active_number().unwrap(), exp, 1e-12);
    }

    // Topological ordering when E (index 4) is calculated (`=B + 100`) and A (index 0) is `=E * 2`.
    let t = as_mv_mut(&mut s);
    t.variables[4].set_calculated_formula("=B + 100");
    assert_eq!(evaluate_calculated_variables(t, None).unwrap(), vec![4, 0]);
    assert_approx(t.variables[0].cells[0].active_number().unwrap(), 202.0, 1e-12);
    assert_approx(t.variables[0].cells[2].active_number().unwrap(), 206.0, 1e-12);

    // Explicit columns named `e`, `pi`, `tau` resolve before built-in constants and track deps.
    let mut named_mv =
        mv_table(&[("Out", &[empty(), empty()]), ("e", &[num(10.0), num(20.0)]), ("pi", &[num(3.0), num(4.0)]), ("tau", &[num(1.0), num(2.0)])]);
    named_mv.variables[0].set_calculated_formula("=e + pi + tau");
    assert!(matches!(validate_no_formula_cycle(&named_mv, 2, "=pi + 1"), Err(TransformError::CircularDependency { .. })));
    assert_eq!(evaluate_calculated_variables(&mut named_mv, None).unwrap(), vec![0]);
    assert_approx(named_mv.variables[0].cells[0].active_number().unwrap(), 14.0, 1e-12);
    assert_approx(named_mv.variables[0].cells[1].active_number().unwrap(), 26.0, 1e-12);

    // Without matching columns, built-in constants resolve cleanly.
    let small = col_empty("rv55-small", 3, 1);
    let expected_const = std::f64::consts::E + std::f64::consts::PI + std::f64::consts::TAU + 1.0;
    assert_formula_first(&small, None, "=e + pi + tau + true + false", ExpectVal::Num(expected_const), 1e-12);
}

#[test]
fn rv_58_word_not_binds_looser_than_comparisons_and_tighter_than_and_or() {
    let ast_gt = parse_formula("not Dose > 5").unwrap();
    assert!(matches!(ast_gt, Expr::Unary { op: UnaryOp::Not, expr: ref inner } if matches!(inner.as_ref(), Expr::Binary { op: BinaryOp::Gt, .. })));

    let ast_and = parse_formula("not A == 1 and B == 2").unwrap();
    let Expr::Binary { op: BinaryOp::And, left, right } = &ast_and else {
        panic!("expected `(not (A == 1)) and (B == 2)`, got {ast_and:?}");
    };
    assert!(matches!(left.as_ref(), Expr::Unary { op: UnaryOp::Not, expr } if matches!(expr.as_ref(), Expr::Binary { op: BinaryOp::Eq, .. })));
    assert!(matches!(right.as_ref(), Expr::Binary { op: BinaryOp::Eq, .. }));

    assert_eq!(eval_scalar_expr(&ast_and, &ScalarEnv::new(vec![("A", 2.0), ("B", 2.0)], None)).unwrap().as_finite_f64(), Some(1.0));
    assert_eq!(eval_scalar_expr(&ast_and, &ScalarEnv::new(vec![("A", 1.0), ("B", 2.0)], None)).unwrap().as_finite_f64(), Some(0.0));

    let s = mv_sheet("rv58-sel", &[("Dose", &[num(0.0), num(3.0), num(5.0), num(10.0)])]);
    let filtered =
        execute_select(&s, &[SelectRule::Predicate(String::from("not Dose > 5"))], TableKind::MultipleVariables, &TransformContext::default())
            .unwrap();
    let kept: Vec<f64> = as_mv(&filtered.data_sheet).variables[0].cells.iter().filter_map(Cell::active_number).collect();
    assert_eq!(kept, vec![0.0, 3.0, 5.0]);

    // Symbolic `!` keeps high unary precedence (`!0 == 1` -> `(!0) == 1` -> 1.0).
    assert_eq!(eval_scalar_expr(&parse_formula("!0 == 1").unwrap(), &ScalarEnv::default()).unwrap().as_finite_f64(), Some(1.0));
}

#[test]
fn rv_62_exclude_row_predicate_handles_blank_text_and_excluded_cells_while_rejecting_unknown_columns() {
    let csv = "10,50\n20,\n30,150\n40,N/A\n50,*250\n60";
    let mut target = mv_empty("rv62-dst", 2, 1);
    let spec = clipboard_spec(
        PlacementMode::Overwrite,
        CellCoord::new(0, 0, 0),
        ImportFilterSpec { parse_asterisk_exclusions: true, ..ImportFilterSpec::default() },
    );
    let opts = ImportPipelineOptions { exclude_row_predicate: Some(String::from("B>100")), ..ImportPipelineOptions::default() };

    let report = apply_import_link_pipeline(
        &mut target,
        &spec,
        ImportSourcePayload::DelimitedText { text: csv, delimiter: ',', decimal_separator: '.' },
        &opts,
        None,
    )
    .unwrap();
    assert_eq!(report.rows_placed, 6);

    let mv = as_mv(&target);
    let excl_pairs: Vec<(bool, bool)> = (0..6).map(|r| (mv.variables[0].cells[r].excluded, mv.variables[1].cells[r].excluded)).collect();
    assert_eq!(excl_pairs, vec![(false, false), (false, false), (true, true), (false, false), (false, true), (false, false)]);
    assert!(mv.variables[1].cells[1].is_empty());

    // Genuinely unknown column names still return UnknownVariable.
    let bad_opts = ImportPipelineOptions { exclude_row_predicate: Some(String::from("NoSuchCol > 100")), ..ImportPipelineOptions::default() };
    let mut target2 = mv_empty("rv62-bad", 2, 1);
    let err = apply_import_link_pipeline(
        &mut target2,
        &spec,
        ImportSourcePayload::DelimitedText { text: "10,50\n20,150", delimiter: ',', decimal_separator: '.' },
        &bad_opts,
        None,
    )
    .unwrap_err();
    assert!(matches!(err, TransformError::UnknownVariable { ref name } if name == "NoSuchCol"), "unexpected error: {err:?}");
}

#[test]
fn rv_64_kleene_three_valued_logic_minmax_missing_and_internal_result_short_row_padding() {
    let s = mv_sheet(
        "rv64-mv",
        &[
            ("A", &[num(10.0), num(2.0), num(2.0), empty()]),
            ("B", &[empty(), empty(), num(20.0), empty()]),
            ("C", &[num(4.0), excl(99.0), num(8.0), empty()]),
        ],
    );
    let (m, n) = (FormulaValue::Missing, FormulaValue::Number);
    for (expr, expected) in [
        ("=A > 5 or B > 5", vec![n(1.0), m.clone(), n(1.0), m.clone()]),
        ("=or(B > 5, A > 5)", vec![n(1.0), m.clone(), n(1.0), m.clone()]),
        ("=A > 5 and B > 5", vec![m.clone(), n(0.0), n(0.0), m.clone()]),
        ("=and(B > 5, A > 5)", vec![m.clone(), n(0.0), n(0.0), m.clone()]),
        ("=min(A, B, C)", vec![n(4.0), n(2.0), n(2.0), m.clone()]),
        ("=max(A, B, C)", vec![n(10.0), n(2.0), n(20.0), m]),
    ] {
        assert_eq!(evaluate_formula_for_sheet(expr, &s, None).unwrap(), expected, "{expr}");
    }

    // Internal-result import pads empty/short rows instead of skipping them and shifting later rows up.
    let rc = |value: f64| ResultCell::Number { value, prefix: None, units: None };
    let res_sheet = result_sheet_with_tab(
        "res-rv64",
        ResultTab {
            id: String::from("summary"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: str_vec(&["Stat", "Val"]),
            row_labels: str_vec(&["R0", "Sep", "Short", "R3"]),
            rows: vec![vec![rc(1.0), rc(2.0)], vec![], vec![rc(10.0)], vec![rc(30.0), rc(40.0)]],
            footnotes: Vec::new(),
        },
    );
    let mut dst = mv_empty("rv64-res-dst", 2, 1);
    let link_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("res-rv64"),
            kind: InternalLinkKind::TerminalResultSheet,
            tab_id: Some(String::from("summary")),
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(3, 1, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec::default(),
    };
    let rep =
        apply_import_link_pipeline(&mut dst, &link_spec, ImportSourcePayload::InternalResult(&res_sheet), &ImportPipelineOptions::default(), None)
            .unwrap();
    assert_eq!((rep.rows_placed, rep.cols_placed), (4, 2));
    let dst_mv = as_mv(&dst);
    let grid: Vec<(Option<f64>, Option<f64>)> =
        (0..4).map(|r| (dst_mv.variables[0].cells[r].active_number(), dst_mv.variables[1].cells[r].active_number())).collect();
    assert_eq!(grid, vec![(Some(1.0), Some(2.0)), (None, None), (Some(10.0), None), (Some(30.0), Some(40.0))]);
}

#[test]
fn rv_s2_savitzky_golay_edge_interior_cubic_consistency_and_nanomolar_conditioning() {
    // 1. Short series (n = 6 < width) preserves cubics under SavitzkyGolay7 and SavitzkyGolay9.
    let short_cubic: Vec<(f64, f64)> = (0..6)
        .map(|i| {
            let x = i as f64;
            (x, 1.25 * x.powi(3) - 3.0 * x.powi(2) + 2.0 * x + 4.0)
        })
        .collect();
    for method in [SmoothMethod::SavitzkyGolay7, SmoothMethod::SavitzkyGolay9] {
        for (&(_, y_true), &(_, y_hat)) in short_cubic.iter().zip(smooth_xy(&short_cubic, method).unwrap().iter()) {
            assert_approx(y_hat, y_true, 1e-10);
        }
    }

    // 2. Nanomolar X spacing (dx = 1e-10) preserves exact cubic order.
    let nanomolar_cubic: Vec<(f64, f64)> = (0..11)
        .map(|i| {
            let u = (i as f64) - 5.0;
            ((i as f64) * 1e-10, 0.4 * u.powi(3) - 1.1 * u.powi(2) + 0.7 * u + 2.5)
        })
        .collect();
    for method in [SmoothMethod::SavitzkyGolay5, SmoothMethod::SavitzkyGolay7, SmoothMethod::SavitzkyGolay9] {
        for (&(_, y_true), &(_, y_hat)) in nanomolar_cubic.iter().zip(smooth_xy(&nanomolar_cubic, method).unwrap().iter()) {
            assert_approx(y_hat, y_true, 1e-9);
        }
    }

    // 3. Exact Gorry (1990) Table I 5-point cubic boundary convolution weights:
    //    i = 0: [69, 4, -6, 4, -1] / 70; i = 1: [2, 27, 12, -8, 2] / 35; i = 2: [-3, 12, 17, 12, -3] / 35.
    let ys = [3.0, -1.0, 4.0, 1.0, 5.0];
    let pts: Vec<(f64, f64)> = ys.iter().enumerate().map(|(i, &y)| (i as f64, y)).collect();
    let sg5 = smooth_xy(&pts, SmoothMethod::SavitzkyGolay5).unwrap();
    let expected = [
        (69.0 * ys[0] + 4.0 * ys[1] - 6.0 * ys[2] + 4.0 * ys[3] - ys[4]) / 70.0,
        (2.0 * ys[0] + 27.0 * ys[1] + 12.0 * ys[2] - 8.0 * ys[3] + 2.0 * ys[4]) / 35.0,
        (-3.0 * ys[0] + 12.0 * ys[1] + 17.0 * ys[2] + 12.0 * ys[3] - 3.0 * ys[4]) / 35.0,
    ];
    for (i, exp) in expected.into_iter().enumerate() {
        assert_approx(sg5[i].1, exp, 1e-12);
    }
}
