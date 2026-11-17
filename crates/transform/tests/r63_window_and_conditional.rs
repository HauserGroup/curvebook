//! Integration tests for `R6.3`: window clauses (`OVER`, `WHERE`, `ORDER BY`, `ROWS`),
//! expanded conditional aggregations, DAG dependency tracking, and `SelectAndTransform`
//! derived column-transform formulas (`docs/implementation/data-tables-and-transforms.md` §4.1, §5.4).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    Cell, CellValue, ConstantValue, DataSheet, DerivedColumnFormula, InfoSheet, MultipleVariablesTable, SubcolumnMode, TableArchetype, TableKind,
    TransformAnalysisSpec, XyTable,
};
use curvebook_transform::{
    DependencyTarget, FormulaValue, TransformContext, TransformError, evaluate_calculated_variables, evaluate_data_sheet_calculated_variables,
    evaluate_formula_for_sheet, execute_select_with_derived, execute_transform, extract_dependencies, parse_formula, validate_no_formula_cycle,
};

fn assert_approx(actual: f64, expected: f64, tol: f64) {
    assert!((actual - expected).abs() <= tol, "expected {expected} ± {tol}, got {actual} (diff = {})", (actual - expected).abs());
}

fn build_assay_mv_sheet() -> (DataSheet, InfoSheet) {
    // 8 rows across Batch A (rows 0..4) and Batch B (rows 4..8).
    // Row 3 has Excluded Response = 999.0; Row 7 has Empty Note.
    let mut mv = MultipleVariablesTable::new(6, 8).unwrap();
    mv.variables[0].title = String::from("Batch");
    mv.variables[1].title = String::from("Role");
    mv.variables[2].title = String::from("Time");
    mv.variables[3].title = String::from("Response");
    mv.variables[4].title = String::from("Flag");
    mv.variables[5].title = String::from("Note");

    let batches = ["A", "A", "A", "A", "B", "B", "B", "B"];
    let roles = ["Ctrl", "Ctrl", "Trt", "Trt", "Ctrl", "Trt", "Trt", "Trt"];
    // Deliberately unsorted Time inside each Batch to exercise ORDER BY!
    let times = [3.0, 1.0, 2.0, 4.0, 2.0, 1.0, 3.0, 4.0];
    let responses = [20.0, 10.0, 45.0, 999.0, 100.0, 150.0, 250.0, 300.0];
    let flags = [1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0];
    let notes = ["ok", "ok", "hi", "bad", "ok", "skip", "hi", ""];

    for r in 0..8 {
        mv.variables[0].cells[r] = Cell::text(batches[r]);
        mv.variables[1].cells[r] = Cell::text(roles[r]);
        mv.variables[2].cells[r] = Cell::number(times[r]);
        mv.variables[3].cells[r] = Cell { value: CellValue::Number(responses[r]), excluded: r == 3 };
        mv.variables[4].cells[r] = Cell::number(flags[r]);
        mv.variables[5].cells[r] = if notes[r].is_empty() { Cell::empty() } else { Cell::text(notes[r]) };
    }

    let mut info = InfoSheet::new_preseeded("info-r63", "Assay Meta").unwrap();
    info.set_constant("WinWidth", ConstantValue::Number(1.0), None).unwrap();

    let sheet = DataSheet::new("mv-r63", "Assay MV", TableArchetype::MultipleVariables(mv)).unwrap();
    (sheet, info)
}

#[test]
fn r63_window_over_where_aggregates_and_non_destructive_filtering() {
    let (sheet, _) = build_assay_mv_sheet();

    // 1. Partitioned aggregate with non-destructive WHERE / FILTER:
    // Batch A Ctrl responses = [20, 10] -> mean = 15.0 (emitted at ALL rows of Batch A, including Trt rows 2 & 3!)
    // Batch B Ctrl responses = [100] -> mean = 100.0
    for expr in [
        "mean(Response) FILTER (WHERE Role == \"Ctrl\") OVER (PARTITION BY Batch)",
        "mean(Response) WHERE Role == \"Ctrl\" OVER (PARTITION BY Batch)",
        "mean(Response) OVER (PARTITION BY Batch WHERE Role == \"Ctrl\")",
    ] {
        let out = evaluate_formula_for_sheet(expr, &sheet, None).unwrap();
        assert_eq!(out.len(), 8);
        for r in 0..4 {
            assert_approx(out[r].as_finite_f64().unwrap(), 15.0, 1e-12);
        }
        for r in 4..8 {
            assert_approx(out[r].as_finite_f64().unwrap(), 100.0, 1e-12);
        }
    }

    // 2. Arithmetic composition with partitioned non-destructive WHERE:
    // Fold change over per-batch control mean. Excluded row 3 remains Missing.
    let fc = evaluate_formula_for_sheet("Response / mean(Response) FILTER (WHERE Role == \"Ctrl\") OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    assert_approx(fc[0].as_finite_f64().unwrap(), 20.0 / 15.0, 1e-12);
    assert_approx(fc[1].as_finite_f64().unwrap(), 10.0 / 15.0, 1e-12);
    assert_approx(fc[2].as_finite_f64().unwrap(), 45.0 / 15.0, 1e-12);
    assert_eq!(fc[3], FormulaValue::Missing);
    assert_approx(fc[5].as_finite_f64().unwrap(), 1.5, 1e-12);
    assert_approx(fc[7].as_finite_f64().unwrap(), 3.0, 1e-12);

    // 3. All aggregate functions across partitions: sum, count, sd, sem, median, iqr, min_col, max_col.
    // Batch A active responses: [20, 10, 45] (sorted: [10, 20, 45]) -> sum=75, count=3, min=10, max=45, median=20, iqr=32.5-15=17.5
    // Batch B active responses: [100, 150, 250, 300] -> sum=800, count=4, min=100, max=300, median=200, iqr=262.5-137.5=125
    let agg_sum = evaluate_formula_for_sheet("sum(Response) OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    assert_approx(agg_sum[0].as_finite_f64().unwrap(), 75.0, 1e-12);
    assert_approx(agg_sum[4].as_finite_f64().unwrap(), 800.0, 1e-12);

    let agg_cnt = evaluate_formula_for_sheet("count(Response) OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    assert_approx(agg_cnt[0].as_finite_f64().unwrap(), 3.0, 1e-12);
    assert_approx(agg_cnt[4].as_finite_f64().unwrap(), 4.0, 1e-12);

    let agg_sd = evaluate_formula_for_sheet("sd(Response) OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    let expected_sd_a = ((5.0_f64.powi(2) + 15.0_f64.powi(2) + 20.0_f64.powi(2)) / 2.0).sqrt();
    assert_approx(agg_sd[1].as_finite_f64().unwrap(), expected_sd_a, 1e-12);

    let agg_sem = evaluate_formula_for_sheet("sem(Response) OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    assert_approx(agg_sem[1].as_finite_f64().unwrap(), expected_sd_a / 3.0_f64.sqrt(), 1e-12);

    let agg_med = evaluate_formula_for_sheet("median(Response) OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    assert_approx(agg_med[2].as_finite_f64().unwrap(), 20.0, 1e-12);
    assert_approx(agg_med[5].as_finite_f64().unwrap(), 200.0, 1e-12);

    let agg_iqr = evaluate_formula_for_sheet("iqr(Response) OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    assert_approx(agg_iqr[0].as_finite_f64().unwrap(), 17.5, 1e-12);
    assert_approx(agg_iqr[6].as_finite_f64().unwrap(), 125.0, 1e-12);

    let agg_minmax =
        evaluate_formula_for_sheet("max_col(Response) OVER (PARTITION BY Batch) - min_col(Response) OVER (PARTITION BY Batch)", &sheet, None)
            .unwrap();
    assert_approx(agg_minmax[0].as_finite_f64().unwrap(), 35.0, 1e-12);
    assert_approx(agg_minmax[7].as_finite_f64().unwrap(), 200.0, 1e-12);
}

#[test]
fn r63_window_order_by_and_rows_frame_bounds() {
    let (sheet, info) = build_assay_mv_sheet();

    // Batch A sorted by Time ASC:
    // Time=1 -> row 1 (Resp=10)
    // Time=2 -> row 2 (Resp=45)
    // Time=3 -> row 0 (Resp=20)
    // Time=4 -> row 3 (Resp=Excluded)
    //
    // Batch B sorted by Time ASC:
    // Time=1 -> row 5 (Resp=150)
    // Time=2 -> row 4 (Resp=100)
    // Time=3 -> row 6 (Resp=250)
    // Time=4 -> row 7 (Resp=300)

    // 1. Cumulative ROWS UNBOUNDED PRECEDING (shorthand for BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW):
    let cum_sum = evaluate_formula_for_sheet("sum(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS UNBOUNDED PRECEDING)", &sheet, None).unwrap();
    assert_approx(cum_sum[1].as_finite_f64().unwrap(), 10.0, 1e-12);
    assert_approx(cum_sum[2].as_finite_f64().unwrap(), 55.0, 1e-12);
    assert_approx(cum_sum[0].as_finite_f64().unwrap(), 75.0, 1e-12);
    assert_approx(cum_sum[3].as_finite_f64().unwrap(), 75.0, 1e-12);
    assert_approx(cum_sum[5].as_finite_f64().unwrap(), 150.0, 1e-12);
    assert_approx(cum_sum[4].as_finite_f64().unwrap(), 250.0, 1e-12);
    assert_approx(cum_sum[6].as_finite_f64().unwrap(), 500.0, 1e-12);
    assert_approx(cum_sum[7].as_finite_f64().unwrap(), 800.0, 1e-12);

    // 2. Centered sliding frame with Info constant offset: ROWS BETWEEN Info("WinWidth") PRECEDING AND 1 FOLLOWING
    let slide_mean = evaluate_formula_for_sheet(
        "mean(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS BETWEEN Info(\"WinWidth\") PRECEDING AND 1 FOLLOWING)",
        &sheet,
        Some(&info),
    )
    .unwrap();
    // Batch B in Time order: [150 (r5), 100 (r4), 250 (r6), 300 (r7)]
    assert_approx(slide_mean[5].as_finite_f64().unwrap(), (150.0 + 100.0) / 2.0, 1e-12);
    assert_approx(slide_mean[4].as_finite_f64().unwrap(), (150.0 + 100.0 + 250.0) / 3.0, 1e-12);
    assert_approx(slide_mean[6].as_finite_f64().unwrap(), (100.0 + 250.0 + 300.0) / 3.0, 1e-12);
    assert_approx(slide_mean[7].as_finite_f64().unwrap(), (250.0 + 300.0) / 2.0, 1e-12);

    // 3. Strictly-preceding frame: ROWS BETWEEN 2 PRECEDING AND 1 PRECEDING
    // At first position in partition, frame is out-of-bounds empty -> Missing for sum/mean, 0 for count!
    let prev2_sum =
        evaluate_formula_for_sheet("sum(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS BETWEEN 2 PRECEDING AND 1 PRECEDING)", &sheet, None)
            .unwrap();
    let prev2_cnt =
        evaluate_formula_for_sheet("count(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS BETWEEN 2 PRECEDING AND 1 PRECEDING)", &sheet, None)
            .unwrap();
    assert_eq!(prev2_sum[5], FormulaValue::Missing);
    assert_approx(prev2_cnt[5].as_finite_f64().unwrap(), 0.0, 1e-12);
    assert_approx(prev2_sum[4].as_finite_f64().unwrap(), 150.0, 1e-12);
    assert_approx(prev2_sum[6].as_finite_f64().unwrap(), 250.0, 1e-12);
    assert_approx(prev2_sum[7].as_finite_f64().unwrap(), 350.0, 1e-12);

    // 4. Forward-looking frame: ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING
    let rem_max = evaluate_formula_for_sheet(
        "max_col(Response) OVER (PARTITION BY Batch ORDER BY Time ASC ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING)",
        &sheet,
        None,
    )
    .unwrap();
    assert_approx(rem_max[1].as_finite_f64().unwrap(), 45.0, 1e-12);
    assert_approx(rem_max[2].as_finite_f64().unwrap(), 45.0, 1e-12);
    assert_approx(rem_max[0].as_finite_f64().unwrap(), 20.0, 1e-12);
    assert_eq!(rem_max[3], FormulaValue::Missing);

    // 5. SQL default RANGE peer behavior when ORDER BY is present without ROWS:
    let mut mv_ties = MultipleVariablesTable::new(2, 4).unwrap();
    mv_ties.variables[0].title = String::from("T");
    mv_ties.variables[1].title = String::from("V");
    for (r, (t, v)) in [(1.0, 10.0), (2.0, 20.0), (2.0, 30.0), (3.0, 40.0)].into_iter().enumerate() {
        mv_ties.variables[0].cells[r] = Cell::number(t);
        mv_ties.variables[1].cells[r] = Cell::number(v);
    }
    let ties_sheet = DataSheet::new("ties", "Ties", TableArchetype::MultipleVariables(mv_ties)).unwrap();
    let range_sum = evaluate_formula_for_sheet("sum(V) OVER (ORDER BY T)", &ties_sheet, None).unwrap();
    assert_approx(range_sum[0].as_finite_f64().unwrap(), 10.0, 1e-12);
    assert_approx(range_sum[1].as_finite_f64().unwrap(), 60.0, 1e-12);
    assert_approx(range_sum[2].as_finite_f64().unwrap(), 60.0, 1e-12);
    assert_approx(range_sum[3].as_finite_f64().unwrap(), 100.0, 1e-12);
}

#[test]
fn r63_row_wise_window_functions_center_standardize_rank_cum_lag_lead_diff_moving_avg() {
    let (sheet, _) = build_assay_mv_sheet();

    // 1. center / standardize / zscore per Batch (non-destructive WHERE Flag == 1):
    // Batch A with Flag == 1: rows 0 (20), 1 (10), 2 (45) -> mean = 25, sd = sqrt((25+225+400)/2) = sqrt(325)
    // Row 3 has Flag == 0 -> Missing!
    let centered = evaluate_formula_for_sheet("center(Response) WHERE Flag == 1 OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    assert_approx(centered[0].as_finite_f64().unwrap(), -5.0, 1e-12);
    assert_approx(centered[1].as_finite_f64().unwrap(), -15.0, 1e-12);
    assert_approx(centered[2].as_finite_f64().unwrap(), 20.0, 1e-12);
    assert_eq!(centered[3], FormulaValue::Missing);
    // Batch B with Flag == 1: rows 4 (100), 6 (250), 7 (300); row 5 has Flag == 0 -> Missing!
    // mean = 650 / 3
    let mean_b_flag = 650.0 / 3.0;
    assert_approx(centered[4].as_finite_f64().unwrap(), 100.0 - mean_b_flag, 1e-12);
    assert_eq!(centered[5], FormulaValue::Missing);

    let std_a = evaluate_formula_for_sheet("standardize(Response) OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    let z_a = evaluate_formula_for_sheet("zscore(Response) OVER (PARTITION BY Batch)", &sheet, None).unwrap();
    assert_eq!(std_a, z_a);
    let sd_a = 325.0_f64.sqrt();
    assert_approx(std_a[0].as_finite_f64().unwrap(), -5.0 / sd_a, 1e-12);
    assert_approx(std_a[2].as_finite_f64().unwrap(), 20.0 / sd_a, 1e-12);

    // Sliding ROWS frame on center / zscore:
    let local_center =
        evaluate_formula_for_sheet("center(Response) OVER (PARTITION BY Batch ORDER BY Time ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)", &sheet, None)
            .unwrap();
    // Batch B in Time order: r5=150, r4=100, r6=250, r7=300
    assert_approx(local_center[5].as_finite_f64().unwrap(), 0.0, 1e-12); // [150] -> 150 - 150
    assert_approx(local_center[4].as_finite_f64().unwrap(), -25.0, 1e-12); // [150, 100] -> 100 - 125

    // 2. row_number, rank, dense_rank (both 1-arg and 0-arg with ORDER BY DESC):
    let rnum = evaluate_formula_for_sheet("row_number() OVER (PARTITION BY Batch ORDER BY Time ASC)", &sheet, None).unwrap();
    assert_approx(rnum[1].as_finite_f64().unwrap(), 1.0, 1e-12); // Time=1
    assert_approx(rnum[2].as_finite_f64().unwrap(), 2.0, 1e-12); // Time=2
    assert_approx(rnum[0].as_finite_f64().unwrap(), 3.0, 1e-12); // Time=3
    assert_approx(rnum[3].as_finite_f64().unwrap(), 4.0, 1e-12); // Time=4

    let rnk_desc = evaluate_formula_for_sheet("rank(Response) OVER (PARTITION BY Batch ORDER BY Response DESC)", &sheet, None).unwrap();
    assert_approx(rnk_desc[2].as_finite_f64().unwrap(), 1.0, 1e-12); // 45
    assert_approx(rnk_desc[0].as_finite_f64().unwrap(), 2.0, 1e-12); // 20
    assert_approx(rnk_desc[1].as_finite_f64().unwrap(), 3.0, 1e-12); // 10
    assert_eq!(rnk_desc[3], FormulaValue::Missing);

    let dense0 = evaluate_formula_for_sheet("dense_rank() OVER (PARTITION BY Batch ORDER BY Role ASC)", &sheet, None).unwrap();
    assert_approx(dense0[0].as_finite_f64().unwrap(), 1.0, 1e-12); // Ctrl
    assert_approx(dense0[1].as_finite_f64().unwrap(), 1.0, 1e-12); // Ctrl
    assert_approx(dense0[2].as_finite_f64().unwrap(), 2.0, 1e-12); // Trt

    // 3. cumsum, cummean, lag, lead, diff, moving_avg with PARTITION BY + ORDER BY + WHERE:
    // Batch B with Flag == 1 sorted by Time ASC: r4 (Time=2, Resp=100), r6 (Time=3, Resp=250), r7 (Time=4, Resp=300); r5 (Flag=0) masked!
    let cummean_b = evaluate_formula_for_sheet("cummean(Response) WHERE Flag == 1 OVER (PARTITION BY Batch ORDER BY Time)", &sheet, None).unwrap();
    assert_approx(cummean_b[4].as_finite_f64().unwrap(), 100.0, 1e-12);
    assert_eq!(cummean_b[5], FormulaValue::Missing);
    assert_approx(cummean_b[6].as_finite_f64().unwrap(), 175.0, 1e-12);
    assert_approx(cummean_b[7].as_finite_f64().unwrap(), 650.0 / 3.0, 1e-12);

    let lag_b = evaluate_formula_for_sheet("lag(Response, 1) WHERE Flag == 1 OVER (PARTITION BY Batch ORDER BY Time)", &sheet, None).unwrap();
    assert_eq!(lag_b[4], FormulaValue::Missing);
    assert_eq!(lag_b[5], FormulaValue::Missing);
    assert_approx(lag_b[6].as_finite_f64().unwrap(), 100.0, 1e-12);
    assert_approx(lag_b[7].as_finite_f64().unwrap(), 250.0, 1e-12);

    let lead_b = evaluate_formula_for_sheet("lead(Response, 1) OVER (PARTITION BY Batch ORDER BY Time)", &sheet, None).unwrap();
    assert_approx(lead_b[5].as_finite_f64().unwrap(), 100.0, 1e-12); // next after Time=1 (r5) is Time=2 (r4=100)
    assert_approx(lead_b[4].as_finite_f64().unwrap(), 250.0, 1e-12);
    assert_approx(lead_b[6].as_finite_f64().unwrap(), 300.0, 1e-12);
    assert_eq!(lead_b[7], FormulaValue::Missing);

    let diff_b = evaluate_formula_for_sheet("diff(Response, 1) OVER (PARTITION BY Batch ORDER BY Time)", &sheet, None).unwrap();
    assert_eq!(diff_b[5], FormulaValue::Missing);
    assert_approx(diff_b[4].as_finite_f64().unwrap(), -50.0, 1e-12); // 100 - 150
    assert_approx(diff_b[6].as_finite_f64().unwrap(), 150.0, 1e-12); // 250 - 100
    assert_approx(diff_b[7].as_finite_f64().unwrap(), 50.0, 1e-12); // 300 - 250

    let mavg = evaluate_formula_for_sheet("moving_avg(Response, 2) OVER (PARTITION BY Batch ORDER BY Time)", &sheet, None).unwrap();
    assert_eq!(mavg[1], FormulaValue::Missing); // first in Batch A
    assert_approx(mavg[2].as_finite_f64().unwrap(), 27.5, 1e-12); // (10+45)/2
    assert_approx(mavg[0].as_finite_f64().unwrap(), 32.5, 1e-12); // (45+20)/2
    assert_eq!(mavg[5], FormulaValue::Missing); // partition boundary resets moving_avg!
    assert_approx(mavg[4].as_finite_f64().unwrap(), 125.0, 1e-12); // (150+100)/2
}

#[test]
fn r63_conditional_aggregates_spreadsheet_parity_and_window_composition() {
    let (sheet, _) = build_assay_mv_sheet();

    // 1. Both (value_expr, condition_expr) and legacy (condition_expr, value_expr) work cleanly:
    // Active Response where Flag == 1: [20, 10, 45, 100, 250, 300] (row 3 excluded/Flag=0, row 5 Flag=0).
    let expected_vals = [20.0, 10.0, 45.0, 100.0, 250.0, 300.0];
    let n = expected_vals.len() as f64;
    let mean = expected_vals.iter().sum::<f64>() / n;
    let var = expected_vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let sd = var.sqrt();
    let sem = sd / n.sqrt();

    assert_approx(evaluate_formula_for_sheet("minif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), 10.0, 1e-12);
    assert_approx(evaluate_formula_for_sheet("minif(Flag == 1, Response)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), 10.0, 1e-12);
    assert_approx(evaluate_formula_for_sheet("maxif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), 300.0, 1e-12);
    assert_approx(evaluate_formula_for_sheet("stdevif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), sd, 1e-12);
    assert_approx(evaluate_formula_for_sheet("sdif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), sd, 1e-12);
    assert_approx(evaluate_formula_for_sheet("stderrif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), sem, 1e-12);
    assert_approx(evaluate_formula_for_sheet("semif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), sem, 1e-12);

    // sumif / meanif with spreadsheet (value_expr, condition_expr) order too:
    assert_approx(
        evaluate_formula_for_sheet("sumif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(),
        expected_vals.iter().sum::<f64>(),
        1e-12,
    );
    assert_approx(evaluate_formula_for_sheet("meanif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), mean, 1e-12);

    // countaif counts non-empty text + numbers (row 7 has empty Note!):
    // Rows with Flag == 1: 0("ok"), 1("ok"), 2("hi"), 4("ok"), 6("hi"), 7(Empty) -> 5 non-empty Notes!
    assert_approx(evaluate_formula_for_sheet("countaif(Note, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), 5.0, 1e-12);
    // 2-arg countif(Response, Flag == 1) counts finite numeric Response where Flag == 1 -> 6.
    assert_approx(evaluate_formula_for_sheet("countif(Response, Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), 6.0, 1e-12);
    // 1-arg countif(Flag == 1) counts rows where Flag == 1 -> 6.
    assert_approx(evaluate_formula_for_sheet("countif(Flag == 1)", &sheet, None).unwrap()[0].as_finite_f64().unwrap(), 6.0, 1e-12);

    // 2. Insufficient samples (< 2) returns Missing for stdevif/stderrif:
    assert_eq!(
        evaluate_formula_for_sheet("stdevif(Response, Role == \"Ctrl\") OVER (PARTITION BY Batch)", &sheet, None).unwrap()[4],
        FormulaValue::Missing
    );
    assert_eq!(
        evaluate_formula_for_sheet("stderrif(Response, Role == \"Ctrl\") OVER (PARTITION BY Batch)", &sheet, None).unwrap()[4],
        FormulaValue::Missing
    );

    // 3. Full composition with OVER, WHERE, ORDER BY, ROWS, and arithmetic:
    // Batch A Ctrl: [20, 10] -> sd = sqrt(50), sem = 5.0
    // Batch B Trt with Flag == 1: [250, 300] -> min=250, max=300, sd=sqrt(1250), mean=275
    let composed = evaluate_formula_for_sheet(
        "(maxif(Response, Role == \"Trt\") OVER (PARTITION BY Batch) - minif(Response, Role == \"Trt\") OVER (PARTITION BY Batch)) + stdevif(Response, Flag == 1) OVER (PARTITION BY Batch)",
        &sheet,
        None,
    )
    .unwrap();
    let mean_b_flag: f64 = 650.0 / 3.0;
    let sd_b_flag = ((mean_b_flag - 100.0).powi(2) + (250.0 - mean_b_flag).powi(2) + (300.0 - mean_b_flag).powi(2)) / 2.0;
    let expected_b = (300.0 - 150.0) + sd_b_flag.sqrt();
    assert_approx(composed[4].as_finite_f64().unwrap(), expected_b, 1e-10);
}

#[test]
fn r63_dag_tracks_partition_order_filter_and_frame_dependencies() {
    // 1. Static dependency extraction across PARTITION BY, ORDER BY, FILTER/WHERE, and ROWS offset:
    let ast = parse_formula(
        "=mean(Val) FILTER (WHERE Active == 1) OVER (PARTITION BY [Grp], Info(\"Split\") ORDER BY [SortKey] DESC ROWS BETWEEN Info(\"W\") PRECEDING AND CURRENT ROW)",
    )
    .unwrap();
    let deps = extract_dependencies(&ast);
    assert!(deps.contains(&DependencyTarget::ColumnName(String::from("Val"))));
    assert!(deps.contains(&DependencyTarget::ColumnName(String::from("Active"))));
    assert!(deps.contains(&DependencyTarget::ColumnName(String::from("Grp"))));
    assert!(deps.contains(&DependencyTarget::ColumnName(String::from("SortKey"))));
    assert!(deps.contains(&DependencyTarget::InfoConstant(String::from("Split"))));
    assert!(deps.contains(&DependencyTarget::InfoConstant(String::from("W"))));

    // 2. Topological ordering when calculated column 0 depends on calculated PARTITION BY (col 2),
    //    calculated ORDER BY (col 3), and calculated WHERE (col 4):
    let mut mv = MultipleVariablesTable::new(5, 4).unwrap();
    mv.variables[0].title = String::from("Running");
    mv.variables[0].set_calculated_formula("=cumsum(Raw) WHERE Keep == 1 OVER (PARTITION BY GrpCalc ORDER BY OrdCalc ASC)");
    mv.variables[1].title = String::from("Raw");
    mv.variables[2].title = String::from("GrpCalc");
    mv.variables[2].set_calculated_formula("=if($index < 2, \"G1\", \"G2\")");
    mv.variables[3].title = String::from("OrdCalc");
    mv.variables[3].set_calculated_formula("=-$index");
    mv.variables[4].title = String::from("Keep");
    mv.variables[4].set_calculated_formula("=if(Raw > 5, 1, 0)");

    for (r, v) in [10.0, 20.0, 3.0, 30.0].into_iter().enumerate() {
        mv.variables[1].cells[r] = Cell::number(v);
    }

    let order = evaluate_calculated_variables(&mut mv, None).unwrap();
    assert_eq!(order.last().copied(), Some(0));
    // G1 ordered by -$index: row 1 (20) then row 0 (10) -> cumsum: row 1 = 20, row 0 = 30.
    // G2: row 2 has Raw=3 (Keep=0 -> Missing!), row 3 = 30.
    assert_approx(mv.variables[0].cells[1].active_number().unwrap(), 20.0, 1e-12);
    assert_approx(mv.variables[0].cells[0].active_number().unwrap(), 30.0, 1e-12);
    assert!(mv.variables[0].cells[2].is_empty());
    assert_approx(mv.variables[0].cells[3].active_number().unwrap(), 30.0, 1e-12);

    // 3. Cycle detection through PARTITION BY, ORDER BY, and WHERE:
    assert!(matches!(validate_no_formula_cycle(&mv, 2, "=mean(Raw) OVER (PARTITION BY Running)"), Err(TransformError::CircularDependency { .. })));
    assert!(matches!(validate_no_formula_cycle(&mv, 3, "=lag(Raw) OVER (ORDER BY Running)"), Err(TransformError::CircularDependency { .. })));
    assert!(matches!(validate_no_formula_cycle(&mv, 4, "=sum(Raw) FILTER (WHERE Running > 0)"), Err(TransformError::CircularDependency { .. })));

    // Self-reference in window clause:
    let mut sheet = DataSheet::new("cyc-self", "Self", TableArchetype::MultipleVariables(mv)).unwrap();
    if let TableArchetype::MultipleVariables(ref mut inner) = sheet.archetype {
        inner.variables[0].set_calculated_formula("=mean(Raw) OVER (PARTITION BY Running)");
    }
    assert!(matches!(evaluate_data_sheet_calculated_variables(&mut sheet, None), Err(TransformError::CircularDependency { .. })));
}

#[test]
fn r63_contextual_keywords_diagnostics_and_zero_row_safety() {
    // Columns named `Order`, `Filter`, `Rows`, `Partition`, `Over`, `Where` remain usable as regular variables:
    let mut mv = MultipleVariablesTable::new(4, 3).unwrap();
    mv.variables[0].title = String::from("Order");
    mv.variables[1].title = String::from("Filter");
    mv.variables[2].title = String::from("Rows");
    mv.variables[3].title = String::from("Over");
    for r in 0..3 {
        mv.variables[0].cells[r] = Cell::number((3 - r) as f64);
        mv.variables[1].cells[r] = Cell::number(if r < 2 { 1.0 } else { 0.0 });
        mv.variables[2].cells[r] = Cell::number((r + 1) as f64 * 10.0);
        mv.variables[3].cells[r] = Cell::number(1.0);
    }
    let sheet = DataSheet::new("kw-mv", "Keywords", TableArchetype::MultipleVariables(mv)).unwrap();
    let out = evaluate_formula_for_sheet("sum(Rows) WHERE Filter == 1 OVER (PARTITION BY Over ORDER BY Order ASC)", &sheet, None).unwrap();
    // Order ASC: r2 (Order=1, Filter=0 -> 0), r1 (Order=2, Filter=1, Rows=20 -> 20), r0 (Order=3, Filter=1, Rows=10 -> 30).
    assert_approx(out[2].as_finite_f64().unwrap(), 0.0, 1e-12);
    assert_approx(out[1].as_finite_f64().unwrap(), 20.0, 1e-12);
    assert_approx(out[0].as_finite_f64().unwrap(), 30.0, 1e-12);

    let out_desc = evaluate_formula_for_sheet("sum(Rows) WHERE Filter == 1 OVER (PARTITION BY Over ORDER BY Order DESC)", &sheet, None).unwrap();
    // Order DESC: r0 (Order=3 -> 10), r1 (Order=2 -> 30), r2 (Order=1, Filter=0 -> 30).
    assert_approx(out_desc[0].as_finite_f64().unwrap(), 10.0, 1e-12);
    assert_approx(out_desc[1].as_finite_f64().unwrap(), 30.0, 1e-12);
    assert_approx(out_desc[2].as_finite_f64().unwrap(), 30.0, 1e-12);

    // Invalid window clauses & scalar misuse fail cleanly:
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

    assert!(matches!(evaluate_formula_for_sheet("sin(Rows) OVER ()", &sheet, None), Err(TransformError::InvalidSpec { .. })));
    assert!(matches!(evaluate_formula_for_sheet("rank()", &sheet, None), Err(TransformError::InvalidSpec { .. })));

    // Zero-row table evaluates cleanly without panicking:
    let empty_mv = MultipleVariablesTable::new(2, 0).unwrap();
    let empty_sheet = DataSheet::new("empty", "Empty", TableArchetype::MultipleVariables(empty_mv)).unwrap();
    assert!(evaluate_formula_for_sheet("lag(A, 1) OVER (PARTITION BY B ORDER BY A)", &empty_sheet, None).unwrap().is_empty());
    assert!(
        evaluate_formula_for_sheet("mean(A) OVER (PARTITION BY B ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING)", &empty_sheet, None).unwrap().is_empty()
    );
}

#[test]
fn r63_select_and_transform_derived_columns_windowed_conditional_and_chained() {
    let (sheet, info) = build_assay_mv_sheet();
    let ctx = TransformContext { info_sheet: Some(&info), selected_data_sets: vec![2], ..Default::default() };

    // 1. Derived columns referencing non-selected source columns (Batch, Role, Response) + projecting to XY:
    let spec = TransformAnalysisSpec::SelectAndTransform {
        filter_formula: Some(String::from("Flag == 1")),
        target_archetype: TableKind::Xy,
        derived_columns: vec![
            DerivedColumnFormula::new("FoldVsCtrl", "Response / mean(Response) FILTER (WHERE Role == \"Ctrl\") OVER (PARTITION BY Batch)"),
            DerivedColumnFormula::new("FoldZ", "zscore(FoldVsCtrl) OVER (PARTITION BY Batch)"),
        ],
    };
    let out = execute_transform(&sheet, &spec, &ctx).unwrap();
    let TableArchetype::Xy(xy) = &out.data_sheet.archetype else {
        panic!("expected Xy projection");
    };
    assert_eq!(xy.x_title, "Time");
    assert_eq!(xy.data_sets.len(), 2);
    assert_eq!(xy.data_sets[0].title, "FoldVsCtrl");
    assert_eq!(xy.data_sets[1].title, "FoldZ");
    assert_eq!(xy.row_count(), 6); // 6 rows where Flag == 1

    // 2. filter_formula referencing a derived column (evaluates derived columns before filtering):
    let spec_filter_derived = TransformAnalysisSpec::SelectAndTransform {
        filter_formula: Some(String::from("abs(RespZ) <= 1.0")),
        target_archetype: TableKind::MultipleVariables,
        derived_columns: vec![DerivedColumnFormula::new("RespZ", "zscore(Response) OVER (PARTITION BY Batch)")],
    };
    let mv_out = execute_transform(&sheet, &spec_filter_derived, &TransformContext::default()).unwrap();
    let TableArchetype::MultipleVariables(out_mv) = &mv_out.data_sheet.archetype else {
        panic!("expected MV");
    };
    assert_eq!(out_mv.variables.len(), 7);
    assert!(out_mv.row_count() < 8);
    for r in 0..out_mv.row_count() {
        let z = out_mv.variables[6].cells[r].active_number().unwrap();
        assert!(z.abs() <= 1.0);
    }

    // 3. In-place column transform on XY source table:
    let mut xy_src = XyTable::new(SubcolumnMode::Single, 1, 4).unwrap();
    xy_src.x_title = String::from("Time");
    xy_src.data_sets[0].title = String::from("Signal");
    for r in 0..4 {
        xy_src.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy_src.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 10.0);
    }
    let xy_sheet = DataSheet::new("xy-src", "XY", TableArchetype::Xy(xy_src)).unwrap();
    let xy_derived = execute_select_with_derived(
        &xy_sheet,
        &[],
        &[DerivedColumnFormula::new("Signal", "Signal - minif(Signal, X > 1)")],
        TableKind::Xy,
        &TransformContext::default(),
    )
    .unwrap();
    let TableArchetype::Xy(xy_after) = &xy_derived.data_sheet.archetype else {
        panic!("expected XY");
    };
    assert_eq!(xy_after.data_sets.len(), 1);
    assert_approx(xy_after.data_sets[0].cells[0][0].active_number().unwrap(), -10.0, 1e-12); // 10 - 20
    assert_approx(xy_after.data_sets[0].cells[1][0].active_number().unwrap(), 0.0, 1e-12);
    assert_approx(xy_after.data_sets[0].cells[3][0].active_number().unwrap(), 20.0, 1e-12);
}
