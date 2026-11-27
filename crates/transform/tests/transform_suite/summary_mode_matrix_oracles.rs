//! Exhaustive summary-mode matrix enforcement and D1 textbook reference oracles for
//! `curvebook-transform` across `Column`, `Xy`, and `Grouped` tables × all eight pre-summarized
//! `SubcolumnMode` layouts.
//!
//! # References
//!
//! - Ku, H. H. (1966). Notes on the use of propagation of error formulas. *Journal of Research of
//!   the National Bureau of Standards* 70C(4):263–273.
//! - Bevington, P. R., & Robinson, D. K. (2003). *Data Reduction and Error Analysis for the
//!   Physical Sciences* (3rd ed.). McGraw-Hill.
//! - Yeh, K. C., & Kwan, K. C. (1978). *J. Pharmacokinet. Biopharm.* 6(1):79–98.
//! - Bailer, A. J. (1988). *J. Pharmacokinet. Biopharm.* 16(3):303–309.
//! - Chan, T. F., Golub, G. H., & LeVeque, R. J. (1983). *The American Statistician* 37(3):242–247.

use curvebook_doc::{
    AnalysisDomainKind, AnalysisKind, AnalysisSpec, AreaUnderCurveSpec, BaselineOperation, Cell, ColumnDesignHint, ColumnTable, DataSheet,
    ErrorPropagationMode, GroupedTable, MathTransformPreset, NormalizeAnchor, ResultStatus, SUMMARY_SUBCOLUMN_MODES, SimulationNoise,
    SplineLowessMethod, SplineLowessSpec, SubcolumnMode, SummaryModeProcedure, SummaryModeSupport, TableArchetype, TableKind, TransformAnalysisSpec,
    XyTable, summary_mode_matrix_rows,
};
use curvebook_transform::{TargetArchetypeKind, TransformContext, TransformError, convert_datasheet_archetype, execute_analysis_transform};

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tol,
        "{label}: expected {expected:.15}, got {actual:.15} (diff = {:.3e}, tol = {tol:.1e})",
        (actual - expected).abs()
    );
}

fn summary_cells_for_mode(mode: SubcolumnMode, mean: f64, sd: f64, n: f64) -> Vec<Cell> {
    match mode {
        SubcolumnMode::MeanSdN => vec![Cell::number(mean), Cell::number(sd), Cell::number(n)],
        SubcolumnMode::MeanSemN => vec![Cell::number(mean), Cell::number(sd / n.sqrt()), Cell::number(n)],
        SubcolumnMode::MeanCvN => vec![Cell::number(mean), Cell::number((sd / mean.abs()) * 100.0), Cell::number(n)],
        SubcolumnMode::MeanSd => vec![Cell::number(mean), Cell::number(sd)],
        SubcolumnMode::MeanSem => vec![Cell::number(mean), Cell::number(sd / n.sqrt())],
        SubcolumnMode::MeanLowHigh => vec![Cell::number(mean), Cell::number(sd), Cell::number(2.0 * sd)],
        SubcolumnMode::UpperLowerLimits => vec![Cell::number(mean), Cell::number(mean - sd), Cell::number(mean + 2.0 * sd)],
        SubcolumnMode::MedianIqr => vec![Cell::number(mean), Cell::number(mean - 0.5 * sd), Cell::number(mean + 1.5 * sd)],
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => panic!("expected summary mode"),
    }
}

fn make_tx_sheet(table_kind: TableKind, mode: SubcolumnMode) -> DataSheet {
    match table_kind {
        TableKind::Column => {
            let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 2, 4).unwrap();
            col.data_sets[0].title = String::from("Base");
            col.data_sets[1].title = String::from("Treated");
            for r in 0..4 {
                col.data_sets[0].cells[r] = summary_cells_for_mode(mode, 10.0 + (r as f64) * 2.0, 1.0, 5.0);
                col.data_sets[1].cells[r] = summary_cells_for_mode(mode, 20.0 + (r as f64) * 4.0, 2.0, 5.0);
            }
            DataSheet::new("col-tx", "Col Tx", TableArchetype::Column(col)).unwrap()
        }
        TableKind::Xy => {
            let mut xy = XyTable::new(mode, 2, 4).unwrap();
            xy.data_sets[0].title = String::from("Base");
            xy.data_sets[1].title = String::from("Treated");
            for r in 0..4 {
                let x = r as f64;
                xy.x_cells[r] = vec![Cell::number(x)];
                xy.data_sets[0].cells[r] = summary_cells_for_mode(mode, 10.0 + x * 2.0, 1.0, 5.0);
                xy.data_sets[1].cells[r] = summary_cells_for_mode(mode, 20.0 + x * 4.0, 2.0, 5.0);
            }
            DataSheet::new("xy-tx", "XY Tx", TableArchetype::Xy(xy)).unwrap()
        }
        TableKind::Grouped => {
            let mut grp = GroupedTable::new(mode, 2, 4).unwrap();
            grp.row_titles = vec![String::from("R1"), String::from("R2"), String::from("R3"), String::from("R4")];
            grp.data_sets[0].title = String::from("Base");
            grp.data_sets[1].title = String::from("Treated");
            for r in 0..4 {
                grp.data_sets[0].cells[r] = summary_cells_for_mode(mode, 10.0 + (r as f64) * 2.0, 1.0, 5.0);
                grp.data_sets[1].cells[r] = summary_cells_for_mode(mode, 20.0 + (r as f64) * 4.0, 2.0, 5.0);
            }
            DataSheet::new("grp-tx", "Grp Tx", TableArchetype::Grouped(grp)).unwrap()
        }
        other => panic!("unexpected table kind {other:?}"),
    }
}

fn representative_transform_kind(proc_id: SummaryModeProcedure, table_kind: TableKind) -> AnalysisKind {
    match proc_id {
        SummaryModeProcedure::FitSplineLowess => {
            AnalysisKind::FitSplineLowess(SplineLowessSpec { method: SplineLowessMethod::NaturalCubicSpline, ..SplineLowessSpec::default() })
        }
        SummaryModeProcedure::AreaUnderCurve => {
            AnalysisKind::AreaUnderCurve(AreaUnderCurveSpec { independent_time_points: true, ..AreaUnderCurveSpec::default() })
        }
        SummaryModeProcedure::TransformMath => AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: Some(MathTransformPreset::LinearScale { slope: 2.0, intercept: 5.0 }),
            error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
        }),
        SummaryModeProcedure::TransformNormalize => AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::FixedValue(100.0),
            output_percent: true,
            per_data_set: true,
        }),
        SummaryModeProcedure::TransformRemoveBaseline => {
            AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Difference })
        }
        SummaryModeProcedure::TransformTransposeColumn | SummaryModeProcedure::TransformTransposeXyGrouped => {
            AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true })
        }
        SummaryModeProcedure::TransformPruneDecimate => {
            AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: false })
        }
        SummaryModeProcedure::TransformPruneAverageBins => {
            AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: true })
        }
        SummaryModeProcedure::TransformSelectAndTransform => AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
            filter_formula: None,
            target_archetype: table_kind,
            derived_columns: Vec::new(),
        }),
        SummaryModeProcedure::TransformSimulate => AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
            seed: 42,
            row_count: 5,
            replicates: 1,
            x_min: 0.0,
            x_max: 4.0,
            equation_id: String::from("straight_line"),
            parameters: vec![(String::from("Slope"), 2.0), (String::from("YIntercept"), 1.0)],
            noise: SimulationNoise::None,
        }),
        SummaryModeProcedure::TransformRandomSample => AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
            seed: 7,
            sample_size: 2,
            stratify_column: None,
            proportional_allocation: false,
        }),
        other => panic!("unexpected non-transform procedure {other:?}"),
    }
}

#[test]
fn exhaustive_transform_summary_mode_matrix_matches_runtime_verdicts() {
    let ctx = TransformContext::default();
    let mut supported_cells = 0_usize;
    let mut refused_cells = 0_usize;

    for row in summary_mode_matrix_rows().iter().filter(|r| r.analysis_kind_id.descriptor().domain == AnalysisDomainKind::Transform) {
        for &table_kind in row.table_kinds {
            let kind = representative_transform_kind(row.procedure, table_kind);
            for mode in SUMMARY_SUBCOLUMN_MODES {
                let expected = row.support_for(mode).expect("every summary mode has a verdict");
                let sheet = make_tx_sheet(table_kind, mode);
                let spec = AnalysisSpec::new("tx-m", sheet.id.as_str(), kind.clone()).unwrap();
                let outcome = execute_analysis_transform(&sheet, &spec, &ctx);

                match expected {
                    SummaryModeSupport::Supported(_) => {
                        let res = outcome
                            .unwrap_or_else(|e| panic!("expected Supported for {:?} on {table_kind:?}/{mode:?}, got error: {e:?}", row.procedure));
                        assert_eq!(res.status, ResultStatus::UpToDate);
                        assert!(res.validate().is_ok());
                        supported_cells += 1;
                    }
                    SummaryModeSupport::Refused(reason) => {
                        assert!(
                            matches!(
                                outcome,
                                Err(TransformError::UnsupportedSubcolumnMode { table_kind: actual_tk, mode: actual_mode, .. })
                                    if actual_tk == table_kind && actual_mode == mode
                            ),
                            "expected UnsupportedSubcolumnMode ({reason:?}) for {:?} on {table_kind:?}/{mode:?}, got {outcome:?}",
                            row.procedure
                        );
                        refused_cells += 1;
                    }
                }
            }
        }
    }

    assert!(supported_cells > 0 && refused_cells == 8, "expected 8 refused Column Transpose cells, got {refused_cells}");
}

#[test]
fn d1_oracles_column_transforms_auc_bailer_spline_and_lossless_conversions() {
    let ctx = TransformContext::default();

    // 1. Ku (1966) first-order error propagation & decreasing-map bound swap on Column tables across all 8 modes.
    for mode in SUMMARY_SUBCOLUMN_MODES {
        let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 1, 2).unwrap();
        col.data_sets[0].cells = vec![summary_cells_for_mode(mode, 10.0, 2.0, 5.0), summary_cells_for_mode(mode, 16.0, 3.0, 7.0)];
        let sheet = DataSheet::new("col-math", "Col Math", TableArchetype::Column(col)).unwrap();

        // Decreasing affine map y' = -2*y + 50: reverses MeanLowHigh offsets and UpperLowerLimits/MedianIqr bounds.
        let dec_spec = AnalysisSpec::new(
            "m-dec",
            "col-math",
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(MathTransformPreset::LinearScale { slope: -2.0, intercept: 50.0 }),
                error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
            }),
        )
        .unwrap();
        let dec_out = execute_analysis_transform(&sheet, &dec_spec, &ctx).unwrap();
        let TableArchetype::Column(dec_col) = &dec_out.analyzable_data.as_ref().unwrap().archetype else {
            panic!("expected Column output");
        };
        assert_eq!(dec_col.subcolumn_mode, mode);
        let row0 = &dec_col.data_sets[0].cells[0];
        assert_close(row0[0].active_number().unwrap(), 30.0, 1e-12, &format!("Math dec {mode:?} center"));
        match mode {
            SubcolumnMode::MeanSdN | SubcolumnMode::MeanSd => assert_close(row0[1].active_number().unwrap(), 4.0, 1e-12, "scaled SD"),
            SubcolumnMode::MeanSemN | SubcolumnMode::MeanSem => {
                assert_close(row0[1].active_number().unwrap(), 4.0 / 5.0_f64.sqrt(), 1e-12, "scaled SEM");
            }
            SubcolumnMode::MeanCvN => assert_close(row0[1].active_number().unwrap(), (4.0 / 30.0) * 100.0, 1e-12, "scaled %CV"),
            SubcolumnMode::MeanLowHigh => {
                // Input [10, -Err=2, +Err=4] under -2y + 50 => center=30, -Err=8 (from +4), +Err=4 (from -2).
                assert_close(row0[1].active_number().unwrap(), 8.0, 1e-12, "MeanLowHigh swapped -Err");
                assert_close(row0[2].active_number().unwrap(), 4.0, 1e-12, "MeanLowHigh swapped +Err");
            }
            SubcolumnMode::UpperLowerLimits => {
                // Input [10, 8, 14] under -2y + 50 => [30, 22, 34].
                assert_close(row0[1].active_number().unwrap(), 22.0, 1e-12, "UL swapped Lower");
                assert_close(row0[2].active_number().unwrap(), 34.0, 1e-12, "UL swapped Upper");
            }
            SubcolumnMode::MedianIqr => {
                // Input [10, 9, 13] under -2y + 50 => [30, 24, 32].
                assert_close(row0[1].active_number().unwrap(), 24.0, 1e-12, "MedianIqr swapped Q1");
                assert_close(row0[2].active_number().unwrap(), 32.0, 1e-12, "MedianIqr swapped Q3");
            }
            SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => panic!("expected summary mode, got {mode:?}"),
        }

        // Chan-Golub-LeVeque (1983) exact bin pooling on Column: (10,2,5) + (16,3,7) -> Mean=13.5, SD=sqrt(175/11), N=12.
        let bin_spec = AnalysisSpec::new(
            "m-bin",
            "col-math",
            AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: true }),
        )
        .unwrap();
        let bin_out = execute_analysis_transform(&sheet, &bin_spec, &ctx).unwrap();
        let TableArchetype::Column(bin_col) = &bin_out.analyzable_data.as_ref().unwrap().archetype else {
            panic!("expected Column output");
        };
        let brow = &bin_col.data_sets[0].cells[0];
        if mode.supports_parametric_moments() {
            assert_close(brow[0].active_number().unwrap(), 13.5, 1e-12, &format!("Bin pool {mode:?} mean"));
            assert_close(brow[2].active_number().unwrap(), 12.0, 1e-12, &format!("Bin pool {mode:?} N"));
            let sd_p = (175.0_f64 / 11.0).sqrt();
            let expected_s = match mode {
                SubcolumnMode::MeanSdN => sd_p,
                SubcolumnMode::MeanSemN => sd_p / 12.0_f64.sqrt(),
                SubcolumnMode::MeanCvN => (sd_p / 13.5) * 100.0,
                other => panic!("expected parametric summary mode, got {other:?}"),
            };
            assert_close(brow[1].active_number().unwrap(), expected_s, 1e-12, &format!("Bin pool {mode:?} spread"));
        } else {
            assert_close(brow[0].active_number().unwrap(), 13.0, 1e-12, &format!("Bin avg {mode:?} center"));
            assert_eq!(brow[1].active_number(), None, "unpoolable spread must be cleared for {mode:?}");
        }
    }

    // 2. Yeh & Kwan (1978) AUC + Bailer (1988) SE oracle across all 8 XY summary modes.
    // Knots [(0, 1), (1, 5), (2, 7), (4, 3)]:
    // Trapezoidal AUC = 0.5*(1*6 + 1*12 + 2*10) = 19.0.
    // Bailer weights c = [0.5, 1.0, 1.5, 1.0]; SD = [0.4, 0.6, 0.8, 0.5], N = [4, 4, 4, 4].
    // Var(AUC) = sum_i c_i^2 * SD_i^2 / N_i.
    let xs = [0.0, 1.0, 2.0, 4.0];
    let ys = [1.0, 5.0, 7.0, 3.0];
    let sds = [0.4, 0.6, 0.8, 0.5];
    let ns = [4.0, 4.0, 4.0, 4.0];
    let cs = [0.5, 1.0, 1.5, 1.0];
    let expected_bailer_se = (0..4).map(|i| cs[i] * cs[i] * sds[i] * sds[i] / ns[i]).sum::<f64>().sqrt();

    for mode in SUMMARY_SUBCOLUMN_MODES {
        let mut xy = XyTable::new(mode, 1, 4).unwrap();
        for i in 0..4 {
            xy.x_cells[i] = vec![Cell::number(xs[i])];
            xy.data_sets[0].cells[i] = summary_cells_for_mode(mode, ys[i], sds[i], ns[i]);
        }
        let sheet = DataSheet::new("xy-auc", "AUC", TableArchetype::Xy(xy)).unwrap();
        let spec = AnalysisSpec::new(
            "auc-1",
            "xy-auc",
            AnalysisKind::AreaUnderCurve(AreaUnderCurveSpec { independent_time_points: true, ..AreaUnderCurveSpec::default() }),
        )
        .unwrap();
        let res = execute_analysis_transform(&sheet, &spec, &ctx).unwrap();
        assert_close(res.lookup_scalar("TotalArea", Some(0)).unwrap(), 19.0, 1e-12, &format!("AUC {mode:?}"));
        assert_close(res.lookup_scalar("NetArea", Some(0)).unwrap(), 19.0, 1e-12, &format!("Net AUC {mode:?}"));
        if mode.supports_parametric_moments() {
            assert_close(res.lookup_scalar("NetAreaSe", Some(0)).unwrap(), expected_bailer_se, 1e-12, &format!("Bailer SE {mode:?}"));
        } else {
            assert_eq!(res.lookup_scalar("NetAreaSe", Some(0)), None, "Bailer SE must be omitted for {mode:?}");
        }
    }

    // 3. Lossless Grouped -> Column and XY -> Column conversion and XY Transpose for MeanLowHigh, UpperLowerLimits, MedianIqr.
    for mode in [SubcolumnMode::MeanLowHigh, SubcolumnMode::UpperLowerLimits, SubcolumnMode::MedianIqr] {
        let grp_sheet = make_tx_sheet(TableKind::Grouped, mode);
        let (col_from_grp, _) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::Column, false).unwrap();
        let TableArchetype::Column(col_t) = &col_from_grp.archetype else {
            panic!("expected Column output");
        };
        assert_eq!(col_t.subcolumn_mode, mode);
        assert_eq!(col_t.data_sets[0].subcolumn_count(), 3);

        let xy_sheet = make_tx_sheet(TableKind::Xy, mode);
        let tr_spec = AnalysisSpec::new(
            "tr-xy",
            xy_sheet.id.as_str(),
            AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true }),
        )
        .unwrap();
        let tr_res = execute_analysis_transform(&xy_sheet, &tr_spec, &ctx).unwrap();
        let TableArchetype::Column(tr_col) = &tr_res.analyzable_data.as_ref().unwrap().archetype else {
            panic!("expected transposed Column output");
        };
        assert_eq!(tr_col.subcolumn_mode, mode);
        assert_eq!(tr_col.data_sets[0].subcolumn_count(), 3);
    }
}
