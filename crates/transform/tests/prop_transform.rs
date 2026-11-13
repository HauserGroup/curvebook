use lumencraft_doc::{
    Cell, CellValue, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, ErrorPropagationMode, MathTransformPreset, NormalizeAnchor,
    TableArchetype, TransformAnalysisSpec, XColumnMode, XyTable,
};
use lumencraft_transform::{MathTransformOp, TransformContext, evaluate_formula_for_sheet, execute_math_transform, execute_transform, parse_formula};
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_exp_ln_and_log10_antilog_inverses(xs in prop::collection::vec(0.01_f64..1e4, 2..20)) {
        let n = xs.len();
        let mut xy = XyTable::new_with_x_mode(
            XColumnMode::Numbers { subcolumn_mode: lumencraft_doc::SubcolumnMode::Single },
            lumencraft_doc::SubcolumnMode::Single,
            1,
            n,
        ).unwrap();
        for (r, &x) in xs.iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x)).unwrap();
            xy.data_sets[0].cells[r][0] = Cell::number(x);
        }
        let sheet = DataSheet::new("p1", "Prop1", TableArchetype::Xy(xy)).unwrap();
        let ctx = TransformContext::default();

        // exp(ln(y)) == y
        let ln_out = execute_transform(
            &sheet,
            &TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(MathTransformPreset::Ln),
                error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
            },
            &ctx,
        ).unwrap();
        let exp_out = execute_transform(
            &ln_out.data_sheet,
            &TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(MathTransformPreset::Exp),
                error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
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
        let n = fracs.len() + 2;
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, n).unwrap();
        col.data_sets[0].cells[0][0] = Cell::number(base);
        col.data_sets[0].cells[1][0] = Cell::number(base + span);
        for (i, &f) in fracs.iter().enumerate() {
            col.data_sets[0].cells[i + 2][0] = Cell::number(base + f * span);
        }
        let sheet = DataSheet::new("pnorm", "Norm", TableArchetype::Column(col.clone())).unwrap();
        let ctx = TransformContext::default();

        let norm_out = execute_transform(
            &sheet,
            &TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::ColumnMin,
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: true,
            },
            &ctx,
        ).unwrap();

        // Denormalize: orig = base + (norm / 100.0) * span
        let denorm_out = execute_math_transform(
            &norm_out.data_sheet,
            None,
            Some(&MathTransformOp::Preset(MathTransformPreset::LinearScale {
                slope: span / 100.0,
                intercept: base,
            })),
            ErrorPropagationMode::FirstOrderDeltaMethod,
            &ctx,
        ).unwrap();

        let TableArchetype::Column(denorm_col) = &denorm_out.data_sheet.archetype else {
            prop_assert!(false, "expected Column");
            return Ok(());
        };
        for r in 0..n {
            let orig = col.data_sets[0].cells[r][0].active_number().unwrap();
            let got = denorm_col.data_sets[0].cells[r][0].active_number().unwrap();
            prop_assert!((got - orig).abs() <= 1e-8 * (orig.abs() + span));
        }
    }

    #[test]
    fn prop_cumsum_diff_inverse(vals in prop::collection::vec(-1e3_f64..1e3, 2..25)) {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, vals.len()).unwrap();
        col.data_sets[0].title = String::from("V");
        col.data_sets[1].title = String::from("C");
        let mut running = 0.0;
        for (r, &v) in vals.iter().enumerate() {
            running += v;
            col.data_sets[0].cells[r][0] = Cell::number(v);
            col.data_sets[1].cells[r][0] = Cell::number(running);
        }
        let sheet = DataSheet::new("pcum", "Cum", TableArchetype::Column(col)).unwrap();
        let cum_eval = evaluate_formula_for_sheet("cumsum(V)", &sheet, None).unwrap();
        let diff_eval = evaluate_formula_for_sheet("diff(C)", &sheet, None).unwrap();

        let mut acc = 0.0;
        for r in 0..vals.len() {
            acc += vals[r];
            prop_assert!((cum_eval[r].as_finite_f64().unwrap() - acc).abs() <= 1e-8);
            if r >= 1 {
                prop_assert!((diff_eval[r].as_finite_f64().unwrap() - vals[r]).abs() <= 1e-8);
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

        let sheet = DataSheet::new("pt", "Transpose", TableArchetype::Column(tbl.clone())).unwrap();
        let ctx = TransformContext::default();
        let spec = TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true };
        let t1 = execute_transform(&sheet, &spec, &ctx).unwrap();
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

        // Also check ContingencyTable transpose involution
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
        let c_sheet = DataSheet::new("pc", "Cont", TableArchetype::Contingency(cont.clone())).unwrap();
        let c1 = execute_transform(&c_sheet, &spec, &ctx).unwrap();
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
}
