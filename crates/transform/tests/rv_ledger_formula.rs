//! Regression tests for the 2026-11-16 milestone review ledger (`transform-formula` slice:
//! RV-19, RV-25, RV-55, RV-58, RV-62, RV-64 formula/import, and Savitzky–Golay boundary order).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, ImportFilterSpec, ImportLinkSpec,
    ImportPlacementSpec, InternalLinkKind, LinkSource, MultipleVariablesTable, PlacementMode, ResultCell, ResultSheet, ResultTab, ResultTabKind,
    SheetCellRange, SheetId, SimulationNoise, TableArchetype, TableKind,
};
use curvebook_transform::{
    AllocationScheme, BinaryOp, DependencyTarget, Expr, FormulaValue, GroupAllocationWeight, ImportPipelineOptions, ImportSourcePayload, ScalarEnv,
    SelectRule, SmoothMethod, SubjectRandomizationConfig, TransformContext, TransformError, UnaryOp, apply_import_link_pipeline, eval_scalar_expr,
    evaluate_calculated_variables, evaluate_data_sheet_calculated_variables, evaluate_equation, evaluate_formula_for_sheet, execute_select,
    extract_dependencies, parse_formula, randomize_subjects_to_groups, simulate_xy_sheet, smooth_xy, validate_no_formula_cycle,
};

fn assert_approx(actual: f64, expected: f64, tol: f64) {
    assert!((actual - expected).abs() <= tol, "expected {expected} ± {tol}, got {actual} (diff = {})", (actual - expected).abs());
}

// ============================================================================
// RV-19: Bounded PermutedBlock sizes before allocation
// ============================================================================

#[test]
fn rv_19_permuted_block_rejects_huge_block_sizes_without_abort() {
    let arms = vec![GroupAllocationWeight::new("Control"), GroupAllocationWeight::new("Treatment")];

    for huge_block in [1_000_000_000_usize, 1_000_000_000_000_000_usize, (usize::MAX / 2) & !1_usize] {
        let cfg = SubjectRandomizationConfig::from_subject_count(
            "rv19-huge",
            "Huge Block Guard",
            12,
            arms.clone(),
            AllocationScheme::PermutedBlock { block_sizes: vec![huge_block] },
            2026,
        );
        let err = randomize_subjects_to_groups(&cfg).expect_err("oversized block_size must fail validation before allocation");
        assert!(matches!(err, TransformError::InvalidSpec { .. }), "expected InvalidSpec for block_size={huge_block}, got {err:?}");
    }

    // Weight-1 single-arm case (`1e15` scenario from ledger) and mixed valid + huge block_sizes.
    let single_arm_huge = SubjectRandomizationConfig::from_subject_count(
        "rv19-single",
        "Single Arm Huge",
        8,
        vec![GroupAllocationWeight::new("OnlyArm")],
        AllocationScheme::PermutedBlock { block_sizes: vec![1_000_000_000_000_000_usize] },
        7,
    );
    assert!(matches!(randomize_subjects_to_groups(&single_arm_huge), Err(TransformError::InvalidSpec { .. })));

    let mixed_huge = SubjectRandomizationConfig::from_subject_count(
        "rv19-mixed",
        "Mixed Block Sizes",
        8,
        arms.clone(),
        AllocationScheme::PermutedBlock { block_sizes: vec![4, 1_000_000_000] },
        7,
    );
    assert!(matches!(randomize_subjects_to_groups(&mixed_huge), Err(TransformError::InvalidSpec { .. })));

    // Valid permuted block sizes continue to work deterministically.
    let ok_cfg = SubjectRandomizationConfig::from_subject_count(
        "rv19-ok",
        "Valid Permuted Block",
        12,
        arms,
        AllocationScheme::PermutedBlock { block_sizes: vec![4, 6] },
        42,
    );
    let ok = randomize_subjects_to_groups(&ok_cfg).expect("valid block sizes must succeed");
    assert_eq!(ok.data_sheet.archetype.row_count(), 12);
}

// ============================================================================
// RV-25: Isolated poles emit missing cells; large Poisson lambda transitions cleanly
// ============================================================================

#[test]
fn rv_25_simulate_xy_handles_isolated_poles_and_large_poisson_lambda() {
    // 1. Michaelis-Menten touching x = -Km (-2.0) at row 0 emits Empty at the pole and finite Y elsewhere.
    let params = vec![(String::from("Vmax"), 100.0), (String::from("Km"), 2.0)];
    let mm_sheet = simulate_xy_sheet("rv25-mm", "MM With Pole", 11, 5, 2, -2.0, 6.0, "michaelis_menten", &params, SimulationNoise::None)
        .expect("isolated pole at x = -Km should leave empty cells rather than aborting");
    let TableArchetype::Xy(mm_xy) = &mm_sheet.archetype else {
        panic!("expected XY table");
    };
    assert_eq!(mm_xy.row_count(), 5);
    assert_eq!(mm_xy.x_value_at(0), Some(-2.0));
    assert_eq!(mm_xy.data_sets[0].cells[0][0].value, CellValue::Empty);
    assert_eq!(mm_xy.data_sets[0].cells[0][1].value, CellValue::Empty);
    // Remaining points x = 0, 2, 4, 6 -> y = 0, 50, 200/3, 75
    assert_approx(mm_xy.data_sets[0].cells[1][0].active_number().expect("x=0"), 0.0, 1e-12);
    assert_approx(mm_xy.data_sets[0].cells[2][0].active_number().expect("x=2"), 50.0, 1e-12);
    assert_approx(mm_xy.data_sets[0].cells[3][0].active_number().expect("x=4"), 200.0 / 3.0, 1e-12);
    assert_approx(mm_xy.data_sets[0].cells[4][0].active_number().expect("x=6"), 75.0, 1e-12);

    // Custom formula with interior pole at x = 1.0.
    let custom_pole = simulate_xy_sheet("rv25-cust", "Custom Pole", 3, 3, 1, 0.0, 2.0, "expr: 10 / (X - 1)", &[], SimulationNoise::None)
        .expect("interior custom pole should emit empty cell");
    let TableArchetype::Xy(c_xy) = &custom_pole.archetype else {
        panic!("expected XY table");
    };
    assert_approx(c_xy.data_sets[0].cells[0][0].active_number().expect("x=0"), -10.0, 1e-12);
    assert_eq!(c_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
    assert_approx(c_xy.data_sets[0].cells[2][0].active_number().expect("x=2"), 10.0, 1e-12);

    // Single-point scalar `evaluate_equation` at a pole and all-non-finite `simulate_xy_sheet` still error.
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
    let pois_sheet = simulate_xy_sheet(
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
    .expect("large Poisson lambda should transition cleanly to normal approximation");
    let TableArchetype::Xy(p_xy) = &pois_sheet.archetype else {
        panic!("expected XY table");
    };
    let draws: Vec<f64> = p_xy.data_sets[0].cells[0].iter().filter_map(Cell::active_number).collect();
    assert_eq!(draws.len(), 256);
    assert!(draws.iter().all(|&v| v.is_finite() && v >= 0.0 && v.fract() == 0.0));
    let mean_draw = draws.iter().sum::<f64>() / (draws.len() as f64);
    let se_mean = lambda_big.sqrt() / (draws.len() as f64).sqrt();
    assert!((mean_draw - lambda_big).abs() <= 5.0 * se_mean, "mean {mean_draw} drifted from {lambda_big} (SE={se_mean})");
}

// ============================================================================
// RV-55: Column `E` / `e` / `pi` / `tau` resolution before math constants + DAG tracking
// ============================================================================

#[test]
fn rv_55_column_e_and_named_constants_resolve_and_recalculate_in_dag() {
    // 5-column table A..E: `=E*2` must read column E (index 4), not Euler's constant, and recalculate when E changes.
    let mut mv = MultipleVariablesTable::new(5, 3).expect("5-col MV");
    for r in 0..3 {
        mv.variables[1].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[4].cells[r] = Cell::number(10.0 * ((r + 1) as f64));
    }

    let ast_e = parse_formula("=E*2").expect("parse =E*2");
    assert_eq!(extract_dependencies(&ast_e), vec![DependencyTarget::ColumnName(String::from("E"))]);

    // Self-reference on column E (`=E + 1`) must be caught by cycle detection.
    assert!(matches!(validate_no_formula_cycle(&mv, 4, "=E + 1"), Err(TransformError::CircularDependency { .. })));

    // Column A (index 0) calculated from column E (index 4): `=E*2`.
    validate_no_formula_cycle(&mv, 0, "=E*2").expect("acyclic E -> A");
    mv.variables[0].set_calculated_formula("=E*2");
    let mut sheet = DataSheet::new("rv55-mv", "Five Columns", TableArchetype::MultipleVariables(mv)).expect("sheet");

    evaluate_data_sheet_calculated_variables(&mut sheet, None).expect("initial eval");
    {
        let TableArchetype::MultipleVariables(t) = &sheet.archetype else {
            panic!("expected MV");
        };
        assert_approx(t.variables[0].cells[0].active_number().expect("r0"), 20.0, 1e-12);
        assert_approx(t.variables[0].cells[1].active_number().expect("r1"), 40.0, 1e-12);
        assert_approx(t.variables[0].cells[2].active_number().expect("r2"), 60.0, 1e-12);
    }

    // Mutate column E and verify recalculation updates column A.
    if let TableArchetype::MultipleVariables(t) = &mut sheet.archetype {
        t.variables[4].cells[0] = Cell::number(7.5);
        t.variables[4].cells[1] = Cell::number(12.5);
        t.variables[4].cells[2] = Cell::number(25.0);
    }
    evaluate_data_sheet_calculated_variables(&mut sheet, None).expect("recalc after editing E");
    {
        let TableArchetype::MultipleVariables(t) = &sheet.archetype else {
            panic!("expected MV");
        };
        assert_approx(t.variables[0].cells[0].active_number().expect("r0 updated"), 15.0, 1e-12);
        assert_approx(t.variables[0].cells[1].active_number().expect("r1 updated"), 25.0, 1e-12);
        assert_approx(t.variables[0].cells[2].active_number().expect("r2 updated"), 50.0, 1e-12);
    }

    // Topological ordering when E (index 4) is itself calculated (`=B + 100`) and A (index 0) is `=E * 2`:
    // index 4 MUST evaluate before index 0 despite 0 < 4.
    if let TableArchetype::MultipleVariables(t) = &mut sheet.archetype {
        t.variables[4].set_calculated_formula("=B + 100");
        let order = evaluate_calculated_variables(t, None).expect("chained E -> A");
        assert_eq!(order, vec![4, 0]);
        assert_approx(t.variables[0].cells[0].active_number().expect("chained r0"), 202.0, 1e-12);
        assert_approx(t.variables[0].cells[2].active_number().expect("chained r2"), 206.0, 1e-12);
    }

    // Explicit columns named `e`, `pi`, and `tau` resolve to column values before built-in constants and track deps.
    let mut named_mv = MultipleVariablesTable::new(4, 2).expect("4-col MV");
    named_mv.variables[0].title = String::from("Out");
    named_mv.variables[1].title = String::from("e");
    named_mv.variables[2].title = String::from("pi");
    named_mv.variables[3].title = String::from("tau");
    named_mv.variables[1].cells = vec![Cell::number(10.0), Cell::number(20.0)];
    named_mv.variables[2].cells = vec![Cell::number(3.0), Cell::number(4.0)];
    named_mv.variables[3].cells = vec![Cell::number(1.0), Cell::number(2.0)];
    named_mv.variables[0].set_calculated_formula("=e + pi + tau");
    assert!(matches!(validate_no_formula_cycle(&named_mv, 2, "=pi + 1"), Err(TransformError::CircularDependency { .. })));
    let order = evaluate_calculated_variables(&mut named_mv, None).expect("eval explicit e/pi/tau cols");
    assert_eq!(order, vec![0]);
    assert_approx(named_mv.variables[0].cells[0].active_number().expect("row 0"), 14.0, 1e-12);
    assert_approx(named_mv.variables[0].cells[1].active_number().expect("row 1"), 26.0, 1e-12);

    // Without matching columns, built-in constants still resolve cleanly.
    let small_col = DataSheet::new(
        "rv55-small",
        "Three Cols",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 1).expect("col")),
    )
    .expect("sheet");
    let const_vals = evaluate_formula_for_sheet("=e + pi + tau + true + false", &small_col, None).expect("constants");
    assert_approx(const_vals[0].as_finite_f64().expect("finite"), std::f64::consts::E + std::f64::consts::PI + std::f64::consts::TAU + 1.0, 1e-12);
}

// ============================================================================
// RV-58: Word-form `not` operator precedence
// ============================================================================

#[test]
fn rv_58_word_not_binds_looser_than_comparisons_and_tighter_than_and_or() {
    let ast_gt = parse_formula("not Dose > 5").expect("parse not Dose > 5");
    assert!(
        matches!(ast_gt, Expr::Unary { op: UnaryOp::Not, expr: ref inner } if matches!(inner.as_ref(), Expr::Binary { op: BinaryOp::Gt, .. })),
        "expected `not (Dose > 5)`, got {ast_gt:?}"
    );

    let ast_and = parse_formula("not A == 1 and B == 2").expect("parse not A == 1 and B == 2");
    match &ast_and {
        Expr::Binary { op: BinaryOp::And, left, right } => {
            assert!(
                matches!(left.as_ref(), Expr::Unary { op: UnaryOp::Not, expr } if matches!(expr.as_ref(), Expr::Binary { op: BinaryOp::Eq, .. }))
            );
            assert!(matches!(right.as_ref(), Expr::Binary { op: BinaryOp::Eq, .. }));
        }
        other => panic!("expected `(not (A == 1)) and (B == 2)`, got {other:?}"),
    }

    let env_match = ScalarEnv::new(vec![("A", 2.0), ("B", 2.0)], None);
    assert_eq!(eval_scalar_expr(&ast_and, &env_match).expect("eval").as_finite_f64(), Some(1.0));
    let env_reject_a = ScalarEnv::new(vec![("A", 1.0), ("B", 2.0)], None);
    assert_eq!(eval_scalar_expr(&ast_and, &env_reject_a).expect("eval").as_finite_f64(), Some(0.0));

    // Row selection predicate `not Dose > 5` retains Dose <= 5 instead of dropping every row.
    let mut mv = MultipleVariablesTable::new(1, 4).expect("mv");
    mv.variables[0].title = String::from("Dose");
    for (r, d) in [0.0, 3.0, 5.0, 10.0].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(d);
    }
    let sheet = DataSheet::new("rv58-sel", "Dose Filter", TableArchetype::MultipleVariables(mv)).expect("sheet");
    let filtered =
        execute_select(&sheet, &[SelectRule::Predicate(String::from("not Dose > 5"))], TableKind::MultipleVariables, &TransformContext::default())
            .expect("select not Dose > 5");
    let TableArchetype::MultipleVariables(out_mv) = &filtered.data_sheet.archetype else {
        panic!("expected MV");
    };
    let kept: Vec<f64> = out_mv.variables[0].cells.iter().filter_map(Cell::active_number).collect();
    assert_eq!(kept, vec![0.0, 3.0, 5.0]);

    // Symbolic `!` keeps high unary precedence (`!0 == 1` -> `(!0) == 1` -> 1.0).
    let bang_ast = parse_formula("!0 == 1").expect("parse !0 == 1");
    assert_eq!(eval_scalar_expr(&bang_ast, &ScalarEnv::default()).expect("eval !0 == 1").as_finite_f64(), Some(1.0));
}

// ============================================================================
// RV-62: `exclude_row_predicate` binds blank/text/excluded columns without `UnknownVariable`
// ============================================================================

#[test]
fn rv_62_exclude_row_predicate_handles_blank_text_and_excluded_cells_while_rejecting_unknown_columns() {
    let csv = "10,50\n20,\n30,150\n40,N/A\n50,*250\n60";
    let mut target =
        DataSheet::new("rv62-dst", "Target", TableArchetype::MultipleVariables(MultipleVariablesTable::new(2, 1).expect("mv"))).expect("sheet");
    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("qc.csv"), delimiter: ',', decimal_separator: '.' },
        filter: ImportFilterSpec { parse_asterisk_exclusions: true, ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, anchor: CellCoord::new(0, 0, 0), ..ImportPlacementSpec::default() },
    };
    let opts = ImportPipelineOptions { exclude_row_predicate: Some(String::from("B>100")), ..ImportPipelineOptions::default() };

    let report = apply_import_link_pipeline(
        &mut target,
        &spec,
        ImportSourcePayload::DelimitedText { text: csv, delimiter: ',', decimal_separator: '.' },
        &opts,
        None,
    )
    .expect("CSV with blank/text/excluded/short B cells must not fail with UnknownVariable");
    assert_eq!(report.rows_placed, 6);

    let TableArchetype::MultipleVariables(mv) = &target.archetype else {
        panic!("expected MV");
    };
    // Row 0 (B=50): active
    assert!(!mv.variables[0].cells[0].excluded);
    assert!(!mv.variables[1].cells[0].excluded);
    // Row 1 (blank B): not excluded
    assert!(!mv.variables[0].cells[1].excluded);
    assert!(mv.variables[1].cells[1].is_empty());
    // Row 2 (B=150): entire row excluded
    assert!(mv.variables[0].cells[2].excluded);
    assert!(mv.variables[1].cells[2].excluded);
    // Row 3 (B="N/A"): not excluded
    assert!(!mv.variables[0].cells[3].excluded);
    assert!(!mv.variables[1].cells[3].excluded);
    // Row 4 (B=*250 already excluded): row predicate sees Missing for excluded B, so A stays active
    assert!(!mv.variables[0].cells[4].excluded);
    assert!(mv.variables[1].cells[4].excluded);
    // Row 5 (short row missing B): not excluded
    assert!(!mv.variables[0].cells[5].excluded);

    // Genuinely unknown column names still return UnknownVariable.
    let bad_opts = ImportPipelineOptions { exclude_row_predicate: Some(String::from("NoSuchCol > 100")), ..ImportPipelineOptions::default() };
    let mut target2 =
        DataSheet::new("rv62-bad", "Bad", TableArchetype::MultipleVariables(MultipleVariablesTable::new(2, 1).expect("mv"))).expect("sheet");
    let err = apply_import_link_pipeline(
        &mut target2,
        &spec,
        ImportSourcePayload::DelimitedText { text: "10,50\n20,150", delimiter: ',', decimal_separator: '.' },
        &bad_opts,
        None,
    )
    .expect_err("unknown column in exclude_row_predicate must fail");
    assert!(matches!(err, TransformError::UnknownVariable { ref name } if name == "NoSuchCol"), "unexpected error: {err:?}");
}

// ============================================================================
// RV-64: Three-valued Kleene logic, missing-skipping row `min`/`max`, and ResultTab short-row padding
// ============================================================================

#[test]
fn rv_64_kleene_three_valued_logic_minmax_missing_and_internal_result_short_row_padding() {
    // 1. Kleene three-valued logic (`and`/`or` infix and variadic) + row-wise `min`/`max` missing handling.
    let mut mv = MultipleVariablesTable::new(3, 4).expect("mv");
    mv.variables[0].title = String::from("A");
    mv.variables[1].title = String::from("B");
    mv.variables[2].title = String::from("C");
    // Row 0: A=10, B=blank, C=4
    // Row 1: A=2,  B=blank, C=excluded(99)
    // Row 2: A=2,  B=20,    C=8
    // Row 3: all blank
    mv.variables[0].cells = vec![Cell::number(10.0), Cell::number(2.0), Cell::number(2.0), Cell::empty()];
    mv.variables[1].cells = vec![Cell::empty(), Cell::empty(), Cell::number(20.0), Cell::empty()];
    mv.variables[2].cells = vec![Cell::number(4.0), Cell { value: CellValue::Number(99.0), excluded: true }, Cell::number(8.0), Cell::empty()];
    let sheet = DataSheet::new("rv64-mv", "Kleene & MinMax", TableArchetype::MultipleVariables(mv)).expect("sheet");

    // `A > 5 or B > 5`:
    // Row 0: true or Missing -> 1.0
    // Row 1: false or Missing -> Missing
    // Row 2: false or true -> 1.0
    // Row 3: Missing or Missing -> Missing
    let or_infix = evaluate_formula_for_sheet("=A > 5 or B > 5", &sheet, None).expect("or infix");
    assert_eq!(or_infix, vec![FormulaValue::Number(1.0), FormulaValue::Missing, FormulaValue::Number(1.0), FormulaValue::Missing]);
    let or_fn = evaluate_formula_for_sheet("=or(B > 5, A > 5)", &sheet, None).expect("or variadic");
    assert_eq!(or_fn, or_infix);

    // `A > 5 and B > 5`:
    // Row 0: true and Missing -> Missing
    // Row 1: false and Missing -> 0.0
    // Row 2: false and true -> 0.0
    // Row 3: Missing and Missing -> Missing
    let and_infix = evaluate_formula_for_sheet("=A > 5 and B > 5", &sheet, None).expect("and infix");
    assert_eq!(and_infix, vec![FormulaValue::Missing, FormulaValue::Number(0.0), FormulaValue::Number(0.0), FormulaValue::Missing]);
    let and_fn = evaluate_formula_for_sheet("=and(B > 5, A > 5)", &sheet, None).expect("and variadic");
    assert_eq!(and_fn, and_infix);

    // Row-wise `min(A, B, C)` and `max(A, B, C)` skip Missing/excluded cells and return Missing only when all are Missing.
    let row_min = evaluate_formula_for_sheet("=min(A, B, C)", &sheet, None).expect("row min");
    assert_eq!(row_min, vec![FormulaValue::Number(4.0), FormulaValue::Number(2.0), FormulaValue::Number(2.0), FormulaValue::Missing]);
    let row_max = evaluate_formula_for_sheet("=max(A, B, C)", &sheet, None).expect("row max");
    assert_eq!(row_max, vec![FormulaValue::Number(10.0), FormulaValue::Number(2.0), FormulaValue::Number(20.0), FormulaValue::Missing]);

    // 2. Internal-result import pads empty/short rows instead of skipping them and shifting later rows up.
    let analysis = AnalysisSpec::new(
        "res-rv64",
        "src",
        AnalysisKind::Transform(curvebook_doc::TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false }),
    )
    .expect("analysis");
    let num = |value: f64| ResultCell::Number { value, prefix: None, units: None };
    let tab = ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Stat"), String::from("Val")],
        row_labels: vec![String::from("R0"), String::from("Sep"), String::from("Short"), String::from("R3")],
        rows: vec![
            vec![num(1.0), num(2.0)],
            vec![],          // separator row
            vec![num(10.0)], // short row missing col 1
            vec![num(30.0), num(40.0)],
        ],
        footnotes: Vec::new(),
    };
    let mut res_sheet = ResultSheet::new(analysis).expect("result sheet");
    res_sheet.tabs.push(tab);

    let mut dst =
        DataSheet::new("rv64-res-dst", "Dest", TableArchetype::MultipleVariables(MultipleVariablesTable::new(2, 1).expect("mv"))).expect("dst");
    let link_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("res-rv64"),
            kind: InternalLinkKind::TerminalResultSheet,
            tab_id: Some(String::from("summary")),
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(3, 1, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, anchor: CellCoord::new(0, 0, 0), ..ImportPlacementSpec::default() },
    };
    let rep =
        apply_import_link_pipeline(&mut dst, &link_spec, ImportSourcePayload::InternalResult(&res_sheet), &ImportPipelineOptions::default(), None)
            .expect("import result tab with short rows");
    assert_eq!(rep.rows_placed, 4);
    assert_eq!(rep.cols_placed, 2);

    let TableArchetype::MultipleVariables(dst_mv) = &dst.archetype else {
        panic!("expected MV");
    };
    assert_eq!(dst_mv.variables[0].cells[0].active_number(), Some(1.0));
    assert_eq!(dst_mv.variables[1].cells[0].active_number(), Some(2.0));
    assert!(dst_mv.variables[0].cells[1].is_empty(), "empty separator row must stay at row 1");
    assert!(dst_mv.variables[1].cells[1].is_empty());
    assert_eq!(dst_mv.variables[0].cells[2].active_number(), Some(10.0));
    assert!(dst_mv.variables[1].cells[2].is_empty(), "short row must pad missing trailing cell");
    assert_eq!(dst_mv.variables[0].cells[3].active_number(), Some(30.0), "row 3 must not shift upward");
    assert_eq!(dst_mv.variables[1].cells[3].active_number(), Some(40.0));
}

// ============================================================================
// Section 2 S3 note: Savitzky–Golay boundary/interior/short-series cubic order & scale invariance
// ============================================================================

#[test]
fn rv_s2_savitzky_golay_edge_interior_cubic_consistency_and_nanomolar_conditioning() {
    // 1. Short series (n < width, e.g. n = 6 under SavitzkyGolay7 and SavitzkyGolay9) must also preserve cubics
    //    instead of dropping to degree 2.
    let short_cubic: Vec<(f64, f64)> = (0..6)
        .map(|i| {
            let x = i as f64;
            (x, 1.25 * x.powi(3) - 3.0 * x.powi(2) + 2.0 * x + 4.0)
        })
        .collect();
    for method in [SmoothMethod::SavitzkyGolay7, SmoothMethod::SavitzkyGolay9] {
        let smoothed = smooth_xy(&short_cubic, method).expect("short cubic SG");
        for (&(_, y_true), &(_, y_hat)) in short_cubic.iter().zip(smoothed.iter()) {
            assert_approx(y_hat, y_true, 1e-10);
        }
    }

    // 2. Nanomolar X spacing (dx = 1e-10): boundary Vandermonde conditioning must preserve exact cubic order.
    let nanomolar_cubic: Vec<(f64, f64)> = (0..11)
        .map(|i| {
            let u = (i as f64) - 5.0;
            let x = (i as f64) * 1e-10;
            (x, 0.4 * u.powi(3) - 1.1 * u.powi(2) + 0.7 * u + 2.5)
        })
        .collect();
    for method in [SmoothMethod::SavitzkyGolay5, SmoothMethod::SavitzkyGolay7, SmoothMethod::SavitzkyGolay9] {
        let smoothed = smooth_xy(&nanomolar_cubic, method).expect("nanomolar cubic SG");
        for (&(_, y_true), &(_, y_hat)) in nanomolar_cubic.iter().zip(smoothed.iter()) {
            assert_approx(y_hat, y_true, 1e-9);
        }
    }

    // 3. Exact Gorry (1990) Table I 5-point cubic boundary convolution weights on arbitrary data:
    //    i = 0: [69, 4, -6, 4, -1] / 70
    //    i = 1: [2, 27, 12, -8, 2] / 35
    //    i = 2 (Savitzky & Golay 1964 Table I center): [-3, 12, 17, 12, -3] / 35
    let ys = [3.0, -1.0, 4.0, 1.0, 5.0];
    let pts: Vec<(f64, f64)> = ys.iter().enumerate().map(|(i, &y)| (i as f64, y)).collect();
    let sg5 = smooth_xy(&pts, SmoothMethod::SavitzkyGolay5).expect("sg5");
    let expected_0 = (69.0 * ys[0] + 4.0 * ys[1] - 6.0 * ys[2] + 4.0 * ys[3] - ys[4]) / 70.0;
    let expected_1 = (2.0 * ys[0] + 27.0 * ys[1] + 12.0 * ys[2] - 8.0 * ys[3] + 2.0 * ys[4]) / 35.0;
    let expected_2 = (-3.0 * ys[0] + 12.0 * ys[1] + 17.0 * ys[2] + 12.0 * ys[3] - 3.0 * ys[4]) / 35.0;
    assert_approx(sg5[0].1, expected_0, 1e-12);
    assert_approx(sg5[1].1, expected_1, 1e-12);
    assert_approx(sg5[2].1, expected_2, 1e-12);
}
