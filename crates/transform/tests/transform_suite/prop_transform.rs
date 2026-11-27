use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineKind, BaselineOperation, Cell, CellValue, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet,
    ErrorPropagationMode, FractionDenominator, FractionOfTotalSpec, MathTransformPreset, NormalizeAnchor, ProportionCiMethod, ResultCell,
    RowStatisticsSpec, RowSummaryKind, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_transform::{
    FormulaValue, MathTransformOp, TransformContext, clopper_pearson_ci, evaluate_formula_for_sheet, execute_analysis_transform,
    execute_math_transform, execute_transform, parse_formula, quantile_type6_sorted, wilson_score_ci,
};
use proptest::prelude::*;

use crate::common::{as_col, as_xy, col_nums, mv_sheet, num, sheet, txt, xy_single};

proptest! {
    #[test]
    fn prop_exp_ln_and_log10_antilog_inverses(xs in prop::collection::vec(0.01_f64..1e4, 2..20)) {
        let sheet = xy_single("p1", &xs, &[&xs]);
        let ctx = TransformContext::default();

        // exp(ln(y)) == y
        let ln_out = execute_transform(
            &sheet,
            &TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(MathTransformPreset::Ln),
                error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
                pharmacology_preset: None,
                interchange_xy: false,
                zero_x_substitute: None,
                asymmetric_ci_policy: Default::default(),
            },
            &ctx,
        ).unwrap();
        let exp_out = execute_transform(
            &ln_out.data_sheet,
            &TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(MathTransformPreset::Exp),
                error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
                pharmacology_preset: None,
                interchange_xy: false,
                zero_x_substitute: None,
                asymmetric_ci_policy: Default::default(),
            },
            &ctx,
        ).unwrap();

        let TableArchetype::Xy(round_xy) = &exp_out.data_sheet.archetype else {
            prop_assert!(false, "expected XY");
            return Ok(());
        };
        for (r, &orig) in xs.iter().enumerate() {
            let got = round_xy.data_sets[0].cells[r][0].active_number().unwrap();
            prop_assert!((got - orig).abs() <= 1e-9 * orig.abs().max(1.0));
        }

        // Antilog_2(log2(y)) == y
        let log2_out = execute_math_transform(&sheet, None, Some(&MathTransformOp::Log2), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
        let anti2_out = execute_math_transform(&log2_out.data_sheet, None, Some(&MathTransformOp::Antilog { base: 2.0 }), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
        let TableArchetype::Xy(anti_xy) = &anti2_out.data_sheet.archetype else {
            prop_assert!(false, "expected XY");
            return Ok(());
        };
        for (r, &orig) in xs.iter().enumerate() {
            let got = anti_xy.data_sets[0].cells[r][0].active_number().unwrap();
            prop_assert!((got - orig).abs() <= 1e-9 * orig.abs().max(1.0));
        }
    }

    #[test]
    fn prop_normalize_denormalize_inverse(
        base in -100.0_f64..100.0,
        span in 0.5_f64..500.0,
        fracs in prop::collection::vec(0.0_f64..1.0, 2..20),
    ) {
        let mut orig_vals = vec![base, base + span];
        orig_vals.extend(fracs.iter().map(|&f| base + f * span));
        let sheet = col_nums("pnorm", &[("A", &orig_vals)]);
        let ctx = TransformContext::default();

        let norm_out = execute_transform(
            &sheet,
            &TransformAnalysisSpec::Normalize { zero_anchor: NormalizeAnchor::ColumnMin, hundred_anchor: NormalizeAnchor::ColumnMax, output_percent: true, per_data_set: true },
            &ctx,
        ).unwrap();
        let denorm_out = execute_math_transform(
            &norm_out.data_sheet,
            None,
            Some(&MathTransformOp::Preset(MathTransformPreset::LinearScale { slope: span / 100.0, intercept: base })),
            ErrorPropagationMode::FirstOrderDeltaMethod,
            &ctx,
        ).unwrap();

        let TableArchetype::Column(denorm_col) = &denorm_out.data_sheet.archetype else {
            prop_assert!(false, "expected Column");
            return Ok(());
        };
        for (r, &orig) in orig_vals.iter().enumerate() {
            let got = denorm_col.data_sets[0].cells[r][0].active_number().unwrap();
            prop_assert!((got - orig).abs() <= 1e-8 * (orig.abs() + span));
        }
    }

    #[test]
    fn prop_cumsum_diff_inverse(vals in prop::collection::vec(-1e3_f64..1e3, 2..25)) {
        let mut running = 0.0;
        let cum_vals: Vec<f64> = vals.iter().map(|&v| { running += v; running }).collect();
        let sheet = col_nums("pcum", &[("V", &vals), ("C", &cum_vals)]);
        let cum_eval = evaluate_formula_for_sheet("cumsum(V)", &sheet, None).unwrap();
        let diff_eval = evaluate_formula_for_sheet("diff(C)", &sheet, None).unwrap();

        for (r, (&v, &acc)) in vals.iter().zip(cum_vals.iter()).enumerate() {
            prop_assert!((cum_eval[r].as_finite_f64().unwrap() - acc).abs() <= 1e-8);
            if r >= 1 {
                prop_assert!((diff_eval[r].as_finite_f64().unwrap() - v).abs() <= 1e-8);
            }
        }
    }

    #[test]
    fn prop_transpose_involution_preserves_grid_titles_and_exclusions(
        rows in 1_usize..8,
        cols in 1_usize..8,
        vals in prop::collection::vec((-1000.0_f64..1000.0, any::<bool>()), 64),
    ) {
        let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows).unwrap();
        for c in 0..cols {
            tbl.data_sets[c].title = format!("Col{c}");
        }
        for r in 0..rows {
            tbl.row_titles[r] = format!("Row{r}");
            for c in 0..cols {
                let (v, ex) = vals[(r * cols + c) % vals.len()];
                tbl.data_sets[c].cells[r][0] = Cell { value: CellValue::Number(v), excluded: ex };
            }
        }
        tbl.row_titles_expanded = true;

        let ctx = TransformContext::default();
        let spec = TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true };
        let t1 = execute_transform(&DataSheet::new("pt", "Transpose", TableArchetype::Column(tbl.clone())).unwrap(), &spec, &ctx).unwrap();
        let t2 = execute_transform(&t1.data_sheet, &spec, &ctx).unwrap();
        let TableArchetype::Column(round) = &t2.data_sheet.archetype else {
            prop_assert!(false, "expected Column");
            return Ok(());
        };
        prop_assert_eq!(round.data_sets.len(), tbl.data_sets.len());
        prop_assert_eq!(&round.row_titles, &tbl.row_titles);
        for c in 0..cols {
            prop_assert_eq!(&round.data_sets[c].title, &tbl.data_sets[c].title);
            for r in 0..rows {
                prop_assert_eq!(&round.data_sets[c].cells[r][0], &tbl.data_sets[c].cells[r][0]);
            }
        }

        // ContingencyTable transpose involution.
        let mut cont = ContingencyTable::new(cols, rows).unwrap();
        for c in 0..cols {
            cont.data_sets[c].title = format!("G{c}");
        }
        for r in 0..rows {
            cont.row_titles[r] = format!("R{r}");
            for c in 0..cols {
                cont.data_sets[c].cells[r][0] = Cell::number((r * cols + c) as f64);
            }
        }
        let c1 = execute_transform(&DataSheet::new("pc", "Cont", TableArchetype::Contingency(cont.clone())).unwrap(), &spec, &ctx).unwrap();
        let c2 = execute_transform(&c1.data_sheet, &spec, &ctx).unwrap();
        let TableArchetype::Contingency(c_round) = &c2.data_sheet.archetype else {
            prop_assert!(false, "expected Contingency");
            return Ok(());
        };
        prop_assert_eq!(&c_round.data_sets, &cont.data_sets);
        prop_assert_eq!(&c_round.row_titles, &cont.row_titles);
    }

    #[test]
    fn prop_formula_parser_fuzz_never_panics(s in "\\PC{0,256}") {
        let _ = parse_formula(&s);
    }

    #[test]
    fn prop_row_stats_edge_cases_and_permutation_equivariance(
        vals in prop::collection::vec(-1e4_f64..1e4, 0..8),
        const_val in -500.0_f64..500.0,
        perm_seed in any::<usize>(),
    ) {
        // 1. n=0, n=1, zero-variance rows, and finite-only invariants never panic.
        let mut sorted_vals = vals.clone();
        sorted_vals.sort_by(f64::total_cmp);
        if vals.is_empty() {
            prop_assert_eq!(quantile_type6_sorted(&sorted_vals, 0.25), None);
        } else {
            let q1 = quantile_type6_sorted(&sorted_vals, 0.25).unwrap();
            let q3 = quantile_type6_sorted(&sorted_vals, 0.75).unwrap();
            prop_assert!(q1 <= q3 + 1e-12);
        }

        // Build 3-row XY table: row 0 = arbitrary vals, row 1 = zero-variance constant, row 2 = single observation.
        let width = vals.len().max(2);
        let mut xy = XyTable::new_with_x_mode(
            XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
            SubcolumnMode::RawReplicates(width),
            1,
            3,
        ).unwrap();
        for r in 0..3 {
            xy.set_x_cell(r, Cell::number(r as f64)).unwrap();
        }
        for (c, &v) in vals.iter().enumerate() {
            xy.data_sets[0].cells[0][c] = Cell::number(v);
        }
        for c in 0..width {
            xy.data_sets[0].cells[1][c] = Cell::number(const_val);
        }
        xy.data_sets[0].cells[2][0] = Cell::number(const_val);

        let src = sheet("prop-rs", TableArchetype::Xy(xy.clone()));
        let ctx = TransformContext::default();
        for summary in [RowSummaryKind::MeanSd, RowSummaryKind::MedianQuartiles, RowSummaryKind::GeometricMeanSd, RowSummaryKind::MeanCi] {
            let an = AnalysisSpec::new("an", "prop-rs", AnalysisKind::RowStatistics(RowStatisticsSpec { summary, upper_percentile: None })).unwrap();
            let res = execute_analysis_transform(&src, &an, &ctx).unwrap();
            for cell in res.tabs[0].rows.iter().flatten() {
                if let ResultCell::Number { value, .. } = cell {
                    prop_assert!(value.is_finite(), "non-finite cell emitted for {summary:?}: {value}");
                }
            }
            // Zero-variance row must have SD == 0 for MeanSd.
            if summary == RowSummaryKind::MeanSd
                && let ResultCell::Number { value: sd_val, .. } = res.tabs[0].rows[1][1]
            {
                prop_assert!(sd_val.abs() <= 1e-10, "zero-variance SD should be ~0, got {sd_val}");
            }
        }

        // 2. Row permutation equivariance: permuting rows permutes RowStatistics output rows identically.
        let order = match perm_seed % 6 {
            0 => [0, 1, 2],
            1 => [0, 2, 1],
            2 => [1, 0, 2],
            3 => [1, 2, 0],
            4 => [2, 0, 1],
            _ => [2, 1, 0],
        };
        let mut perm_xy = xy.clone();
        for (dst_r, &src_r) in order.iter().enumerate() {
            perm_xy.set_x_cell(dst_r, Cell::number(src_r as f64)).unwrap();
            perm_xy.data_sets[0].cells[dst_r] = xy.data_sets[0].cells[src_r].clone();
        }
        let perm_src = sheet("prop-rs-p", TableArchetype::Xy(perm_xy));
        let an1 = AnalysisSpec::new("an1", "prop-rs", AnalysisKind::RowStatistics(RowStatisticsSpec { summary: RowSummaryKind::MedianQuartiles, upper_percentile: None })).unwrap();
        let an2 = AnalysisSpec::new("an2", "prop-rs-p", AnalysisKind::RowStatistics(RowStatisticsSpec { summary: RowSummaryKind::MedianQuartiles, upper_percentile: None })).unwrap();
        let r1 = execute_analysis_transform(&src, &an1, &ctx).unwrap();
        let r2 = execute_analysis_transform(&perm_src, &an2, &ctx).unwrap();
        for (dst_r, &src_r) in order.iter().enumerate() {
            prop_assert_eq!(&r2.tabs[0].rows[dst_r], &r1.tabs[0].rows[src_r]);
        }
    }

    #[test]
    fn prop_fraction_of_total_ci_bounds_and_row_permutation_invariance(
        counts in prop::collection::vec((0_u32..50, 0_u32..50), 2..8),
        rotate in 0_usize..8,
    ) {
        // Binomial CI invariant: 0 <= lo <= k/n <= hi <= 1 for all integer 0 <= k <= n, n >= 1.
        for &(a, b) in &counts {
            let n = (a + b).max(1) as f64;
            let k = a.min(a + b) as f64;
            let p = k / n;
            let (cp_lo, cp_hi) = clopper_pearson_ci(k, n, 0.05).unwrap();
            let (w_lo, w_hi) = wilson_score_ci(k, n, 0.05).unwrap();
            prop_assert!(cp_lo >= -1e-15 && cp_lo <= p + 1e-12 && p <= cp_hi + 1e-12 && cp_hi <= 1.0 + 1e-15);
            prop_assert!(w_lo >= -1e-15 && w_lo <= p + 1e-12 && p <= w_hi + 1e-12 && w_hi <= 1.0 + 1e-15);
        }

        // Row rotation leaves ColumnTotal and GrandTotal fractions equivariant with identical totals.
        let n_rows = counts.len();
        let c0: Vec<f64> = counts.iter().map(|&(a, _)| f64::from(a)).collect();
        let c1: Vec<f64> = counts.iter().map(|&(_, b)| f64::from(b)).collect();
        let shift = rotate % n_rows;
        let c0_rot: Vec<f64> = (0..n_rows).map(|i| c0[(i + shift) % n_rows]).collect();
        let c1_rot: Vec<f64> = (0..n_rows).map(|i| c1[(i + shift) % n_rows]).collect();

        let s1 = col_nums("f1", &[("A", &c0), ("B", &c1)]);
        let s2 = col_nums("f2", &[("A", &c0_rot), ("B", &c1_rot)]);
        let spec = FractionOfTotalSpec { denominator: FractionDenominator::GrandTotal, output_percent: false, ci_method: ProportionCiMethod::Wilson };
        let r1 = execute_analysis_transform(&s1, &AnalysisSpec::new("a1", "f1", AnalysisKind::FractionOfTotal(spec)).unwrap(), &TransformContext::default()).unwrap();
        let r2 = execute_analysis_transform(&s2, &AnalysisSpec::new("a2", "f2", AnalysisKind::FractionOfTotal(spec)).unwrap(), &TransformContext::default()).unwrap();
        for i in 0..n_rows {
            let orig_i = (i + shift) % n_rows;
            for col_cell in 0..r1.tabs[0].rows[orig_i].len() {
                match (&r1.tabs[0].rows[orig_i][col_cell], &r2.tabs[0].rows[i][col_cell]) {
                    (ResultCell::Number { value: v1, .. }, ResultCell::Number { value: v2, .. }) => {
                        prop_assert!((v1 - v2).abs() <= 1e-12);
                    }
                    (ResultCell::Empty, ResultCell::Empty) => {}
                    (a, b) => prop_assert!(false, "cell mismatch {a:?} vs {b:?}"),
                }
            }
        }
    }

    #[test]
    fn prop_formula_nan_inf_safety_and_partition_permutation_invariance(
        vals in prop::collection::vec(-1e3_f64..1e3, 2..10),
        groups in prop::collection::vec(0_u8..3, 2..10),
        shift_raw in 0_usize..10,
    ) {
        let n = vals.len().min(groups.len());
        let labels = ["Alpha", "Beta", ""];
        let g_cells: Vec<Cell> = (0..n).map(|i| txt(labels[usize::from(groups[i])])).collect();
        let v_cells: Vec<Cell> = (0..n).map(|i| num(vals[i])).collect();

        let shift = shift_raw % n;
        let g_rot: Vec<Cell> = (0..n).map(|i| g_cells[(i + shift) % n].clone()).collect();
        let v_rot: Vec<Cell> = (0..n).map(|i| v_cells[(i + shift) % n].clone()).collect();

        let s1 = mv_sheet("p1", &[("G", &g_cells), ("V", &v_cells)]);
        let s2 = mv_sheet("p2", &[("G", &g_rot), ("V", &v_rot)]);

        for expr in ["mean(V) OVER G", "min(V) OVER G", "max(V) OVER G", "count(V) OVER G"] {
            let o1 = evaluate_formula_for_sheet(expr, &s1, None).unwrap();
            let o2 = evaluate_formula_for_sheet(expr, &s2, None).unwrap();
            for i in 0..n {
                let orig = (i + shift) % n;
                match (&o1[orig], &o2[i]) {
                    (FormulaValue::Number(a), FormulaValue::Number(b)) => {
                        prop_assert!((a - b).abs() <= 1e-10 * a.abs().max(1.0), "{expr}: {a} vs {b}");
                    }
                    (FormulaValue::Missing, FormulaValue::Missing) => {}
                    (a, b) => prop_assert!(false, "{expr} mismatch: {a:?} vs {b:?}"),
                }
            }
        }

        // Domain-error / non-finite formula outputs always map to Missing cleanly, never NaN/Inf.
        let ood = evaluate_formula_for_sheet("log(-abs(V) - 1) + atanh(abs(V) + 2) + (V / 0)", &s1, None).unwrap();
        for v in ood {
            prop_assert_eq!(v, FormulaValue::Missing);
        }

        // Baseline ColumnsMean subtraction is row-permutation equivariant.
        let b1 = col_nums("b1", &[("Base", &vals[..n]), ("Sig", &vals[..n])]);
        let rot_vals: Vec<f64> = (0..n).map(|i| vals[(i + shift) % n]).collect();
        let b2 = col_nums("b2", &[("Base", &rot_vals), ("Sig", &rot_vals)]);
        let spec = TransformAnalysisSpec::RemoveBaseline {
            baseline_columns: vec![0],
            operation: BaselineOperation::Difference,
            baseline_kind: Some(BaselineKind::ColumnsMean(vec![0])),
            retain_unselected_columns: true,
        };
        let r1 = execute_transform(&b1, &spec, &TransformContext { selected_data_sets: vec![1], ..Default::default() }).unwrap();
        let r2 = execute_transform(&b2, &spec, &TransformContext { selected_data_sets: vec![1], ..Default::default() }).unwrap();
        let t1 = as_col(&r1.data_sheet);
        let t2 = as_col(&r2.data_sheet);
        for i in 0..n {
            let orig = (i + shift) % n;
            let a = t1.data_sets[1].cells[orig][0].active_number().unwrap();
            let b = t2.data_sets[1].cells[i][0].active_number().unwrap();
            prop_assert!((a - b).abs() <= 1e-10 * a.abs().max(1.0));
        }

        // Ensure unused as_xy import stays exercised on 1-row XY baseline edge case.
        let single = xy_single("xy-one", &[1.0], &[&[vals[0]]]);
        let one_out = execute_transform(
            &single,
            &TransformAnalysisSpec::RemoveBaseline {
                baseline_columns: vec![],
                operation: BaselineOperation::Difference,
                baseline_kind: Some(BaselineKind::OlsLinearWithX { first_k: 1, last_k: 1 }),
                retain_unselected_columns: true,
            },
            &TransformContext::default(),
        ).unwrap();
        prop_assert_eq!(as_xy(&one_out.data_sheet).row_count(), 1);
    }
}
