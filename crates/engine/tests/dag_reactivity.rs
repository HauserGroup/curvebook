//! Integration tests for reactive `Data -> Analysis -> Graph -> Layout` DAG recalculation,
//! linked data regions, calculated variables, `HookSource` bindings, frozen sheet protection,
//! cycle rejection rollback, and downstream graph style/annotation preservation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisSpec, BracketLabelMode, BracketStyle, Cell, CellCoord, ColumnDesignHint, ColumnTable,
    ConstantValue, DataSheet, DescriptiveSpec, FloatingNote, GraphAnnotation, GraphSheet, GraphicalDataObject, HookSource, ImportFilterSpec,
    ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutItem, LayoutSheet, LayoutSpec, LegendSpec, LinearRegressionSpec,
    LinkSource, MathTransformPreset, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, NonlinearFitSpec, NormalizeAnchor,
    OneWayAnovaSpec, OneWayMethod, PairwiseBracketSpec, ParameterRule, ParameterScope, PlacementMode, ProjectDocument, RectPt, RegressionOverlaySpec,
    ResultSheet, ResultStatus, SheetCellRange, SheetId, SubcolumnMode, TableArchetype, TickLabelFormat, TransformAnalysisSpec, WeightingScheme,
    WhiskerMethod, XColumnMode, XyTable,
};
use curvebook_engine::dag::{recalculate_project, recalculate_project_with_report, topological_order};
use curvebook_engine::guard::run_guarded;
use curvebook_engine::{EngineError, Session};
use curvebook_fit::get_builtin_model;
use curvebook_palette::{MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{ImportPipelineOptions, ImportSourcePayload, apply_import_link_pipeline};

fn approx_eq(actual: f64, expected: f64, tol: f64, label: &str) {
    assert_approx_eq(actual, expected, tol, tol, label);
}

fn make_dose_response_sheet(id: &str, info_id: Option<&str>, xs: &[f64], true_params: [f64; 4]) -> DataSheet {
    let model = get_builtin_model("log_agonist_vs_response_4pl").expect("4PL builtin");
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, xs.len())
        .expect("xy table");
    xy.data_sets[0].title = String::from("Agonist");
    for (r, &x) in xs.iter().enumerate() {
        let y = model.eval(&[x], &true_params, 0).expect("4PL eval");
        xy.set_x_cell(r, Cell::number(x)).expect("set x");
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let mut ds = DataSheet::new(id, "Dose Response", TableArchetype::Xy(xy)).expect("xy sheet");
    ds.info_sheet_id = info_id.map(SheetId::new);
    ds
}

#[test]
fn multi_hop_data_transform_fit_and_downstream_graph_reactivity() {
    let mut doc = ProjectDocument::new("Multi-hop DAG").expect("doc");

    let mut info = InfoSheet::new_preseeded("info-assay", "Assay Metadata").expect("info");
    info.set_constant("ScaleMax", ConstantValue::Number(100.0), None).expect("set constant");
    info.set_constant("AssayTag", ConstantValue::Text(String::from("Batch-01")), None).expect("set text constant");
    doc.add_info_sheet(info).expect("add info");

    // Raw 4PL with Bottom=0, Top=100, LogEC50=-7.0, HillSlope=1.0
    let xs = [-10.0, -9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let ds = make_dose_response_sheet("ds-dose", Some("info-assay"), &xs, [0.0, 100.0, -7.0, 1.0]);
    doc.add_data_sheet(ds).expect("add ds");

    // Step 1: Normalize Transform hooking InfoSheet constant ScaleMax (=100 -> 100%)
    let norm_spec = AnalysisSpec::new(
        "res-norm",
        "ds-dose",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-assay")),
                constant_name: String::from("ScaleMax"),
            }),
            output_percent: true,
            per_data_set: true,
        }),
    )
    .expect("norm spec");
    doc.add_result_sheet(ResultSheet::new(norm_spec).expect("norm res")).expect("add norm");

    // Step 2: 4PL Fit downstream of Transform ResultSheet `res-norm`
    let fit_spec = AnalysisSpec::new(
        "res-fit",
        "res-norm",
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            weighting: WeightingScheme::None,
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        }),
    )
    .expect("fit spec");
    doc.add_result_sheet(ResultSheet::new(fit_spec).expect("fit res")).expect("add fit");

    // Step 3: Downstream GraphSheet referencing `ds-dose` + regression overlay from `res-fit`
    // configured with custom palette, marker GDO, 2-segment axis break, bracket, and annotations.
    let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &doc.data_sheets[0]).expect("graph");
    gsheet.initialized = true;
    gsheet.spec.title = String::from("Dose Curve {{info:AssayTag}}");
    gsheet.spec.palette = PaletteId::OkabeIto;
    gsheet.spec.x_axis = AxisSpec {
        segments: vec![
            AxisSegment {
                min: Some(-10.5),
                max: Some(-7.5),
                scale: AxisScale::Linear,
                length_share: 0.45,
                major_tick_interval: Some(1.0),
                minor_ticks_per_interval: 1,
            },
            AxisSegment {
                min: Some(-6.5),
                max: Some(-3.5),
                scale: AxisScale::Linear,
                length_share: 0.55,
                major_tick_interval: Some(1.0),
                minor_ticks_per_interval: 1,
            },
        ],
        tick_format: TickLabelFormat::Decimal,
        ..AxisSpec::linear("Log[Agonist]")
    };
    gsheet.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
        marker: Some(MarkerStyle {
            shape: MarkerShape::Diamond,
            fill_mode: MarkerFillMode::Open,
            size_pt: 6.0,
            fill_color: Rgba8::WHITE,
            stroke_color: Rgba8::rgb(0x00, 0x72, 0xB2),
            stroke_width_pt: 1.5,
        }),
        line: None,
        area_fill: None,
        replicate_lines: curvebook_doc::ReplicateLineMode::default(),
    };
    gsheet.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-fit"),
        sample_segments: 128,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    gsheet.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(14.0, 14.0, 120.0, 28.0),
        markdown: String::from("LogEC50={{result:res-fit:LogEC50}} | {{info:AssayTag}}"),
        bordered: true,
    });
    gsheet.spec.legend = LegendSpec { visible: true, position_pt: Some((180.0, 20.0)), bordered: true };
    gsheet.floating_notes.push(FloatingNote::new("g-note-1", RectPt::new(20.0, 200.0, 120.0, 40.0), "Do not reset on recalc"));
    gsheet.color_tag = Some(Rgba8::rgb(10, 90, 160));
    let expected_graph_spec = gsheet.spec.clone();
    let expected_notes = gsheet.floating_notes.clone();
    let expected_tag = gsheet.color_tag;
    doc.add_graph_sheet(gsheet).expect("add graph");

    let report_v1 = recalculate_project_with_report(&mut doc).expect("recalc v1");
    let pos_info = report_v1.evaluated_order.iter().position(|id| id.as_str() == "info-assay").expect("info pos");
    let pos_data = report_v1.evaluated_order.iter().position(|id| id.as_str() == "ds-dose").expect("data pos");
    let pos_norm = report_v1.evaluated_order.iter().position(|id| id.as_str() == "res-norm").expect("norm pos");
    let pos_fit = report_v1.evaluated_order.iter().position(|id| id.as_str() == "res-fit").expect("fit pos");
    let pos_graph = report_v1.evaluated_order.iter().position(|id| id.as_str() == "graph-1").expect("graph pos");
    assert!(pos_info < pos_data);
    assert!(pos_data < pos_norm);
    assert!(pos_norm < pos_fit);
    assert!(pos_fit < pos_graph);
    assert_eq!(report_v1.preserved_graph_sheets, vec![SheetId::new("graph-1")]);

    let fit_v1 = doc.result_sheet(&SheetId::new("res-fit")).expect("fit v1");
    assert_eq!(fit_v1.status, ResultStatus::UpToDate);
    approx_eq(fit_v1.lookup_scalar("LogEC50", Some(0)).expect("LogEC50"), -7.0, 1e-4, "v1 LogEC50");
    approx_eq(fit_v1.lookup_scalar("Top", Some(0)).expect("Top"), 100.0, 1e-4, "v1 Top");

    let scene_v1 = compile_graph(&doc.graph_sheets[0], &doc).expect("compile graph v1");

    // Mutate upstream DataSheet (shift LogEC50 from -7.0 to -6.0) AND mutate InfoSheet ScaleMax to 200.0
    let shifted = make_dose_response_sheet("ds-dose", Some("info-assay"), &xs, [0.0, 100.0, -6.0, 1.0]);
    doc.data_sheets[0].archetype = shifted.archetype;
    doc.info_sheets[0].set_constant("ScaleMax", ConstantValue::Number(200.0), None).expect("update constant");

    let report_v2 = recalculate_project_with_report(&mut doc).expect("recalc v2");
    assert!(report_v2.failed_result_sheets.is_empty());

    let fit_v2 = doc.result_sheet(&SheetId::new("res-fit")).expect("fit v2");
    assert_eq!(fit_v2.status, ResultStatus::UpToDate);
    approx_eq(fit_v2.lookup_scalar("LogEC50", Some(0)).expect("LogEC50 v2"), -6.0, 1e-4, "v2 LogEC50");
    approx_eq(fit_v2.lookup_scalar("Top", Some(0)).expect("Top v2"), 50.0, 1e-4, "v2 Top (scaled)");

    // Downstream graph styles/GDOs/axis breaks/brackets/annotations MUST be completely preserved,
    // while compiled scene reflects updated curve/overlay/annotation tokens.
    let g_after = &doc.graph_sheets[0];
    assert!(g_after.initialized);
    assert_eq!(g_after.spec, expected_graph_spec);
    assert_eq!(g_after.floating_notes, expected_notes);
    assert_eq!(g_after.color_tag, expected_tag);

    let scene_v2 = compile_graph(g_after, &doc).expect("compile graph v2");
    assert_ne!(scene_v1.primitives, scene_v2.primitives);
}

#[test]
fn calculated_variable_propagation_and_formula_cycle_rollback() {
    let mut doc = ProjectDocument::new("Calc Var Propagation").expect("doc");

    let mut info = InfoSheet::new_preseeded("info-mv", "Params").expect("info");
    info.set_constant("Gain", ConstantValue::Number(3.0), None).expect("gain");
    info.set_constant("Bias", ConstantValue::Number(5.0), None).expect("bias");
    doc.add_info_sheet(info).expect("add info");

    // Chained calculated variables: Mid = X * 2, Y = Mid + [Gain] + [Bias]
    let mut mv = MultipleVariablesTable::new(3, 5).expect("mv table");
    mv.variables[0].title = String::from("X");
    mv.variables[1].title = String::from("Mid");
    mv.variables[1].set_calculated_formula("=X * 2");
    mv.variables[2].title = String::from("Y");
    mv.variables[2].set_calculated_formula("=Mid + [Gain] + [Bias]");
    for r in 0..5 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
    }
    let mut ds_mv = DataSheet::new("ds-mv", "MV Calc", TableArchetype::MultipleVariables(mv)).expect("ds mv");
    ds_mv.info_sheet_id = Some(SheetId::new("info-mv"));
    doc.add_data_sheet(ds_mv).expect("add ds_mv");

    // A second independent sheet to verify atomic rollback across the whole document on formula cycle.
    let mut other_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("col");
    other_tbl.data_sets[0].cells[0][0] = Cell::number(42.0);
    doc.add_data_sheet(DataSheet::new("ds-other", "Other", TableArchetype::Column(other_tbl)).expect("other")).expect("add other");

    let mut lin_spec = AnalysisSpec::new("res-lin", "ds-mv", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).expect("lin spec");
    lin_spec.selected_data_sets = vec![0, 2];
    doc.add_result_sheet(ResultSheet::new(lin_spec).expect("lin res")).expect("add res");

    let rep1 = recalculate_project_with_report(&mut doc).expect("initial recalc");
    assert!(rep1.updated_data_sheets.contains(&SheetId::new("ds-mv")));

    if let TableArchetype::MultipleVariables(ref tbl) = doc.data_sheets[0].archetype {
        approx_eq(tbl.variables[2].cells[0].active_number().expect("y0"), 10.0, 1e-12, "y0=2*1+3+5");
        approx_eq(tbl.variables[2].cells[4].active_number().expect("y4"), 18.0, 1e-12, "y4=2*5+3+5");
    } else {
        panic!("expected MultipleVariables");
    }
    let res_lin_1 = doc.result_sheet(&SheetId::new("res-lin")).expect("res lin 1");
    approx_eq(res_lin_1.lookup_scalar("Slope", Some(0)).expect("slope"), 2.0, 1e-9, "slope 1");
    approx_eq(res_lin_1.lookup_scalar("YIntercept", Some(0)).expect("int"), 8.0, 1e-9, "intercept 1");

    // Mutate Gain from 3 -> 13 and verify propagation through Mid/Y and into downstream regression
    doc.info_sheets[0].set_constant("Gain", ConstantValue::Number(13.0), None).expect("update gain");
    recalculate_project(&mut doc).expect("recalc after gain edit");

    let res_lin_2 = doc.result_sheet(&SheetId::new("res-lin")).expect("res lin 2");
    approx_eq(res_lin_2.lookup_scalar("Slope", Some(0)).expect("slope 2"), 2.0, 1e-9, "slope 2");
    approx_eq(res_lin_2.lookup_scalar("YIntercept", Some(0)).expect("int 2"), 18.0, 1e-9, "intercept 2");

    // Introduce cyclic formula Mid = Y + 1 -> must fail and preserve document atomically
    if let TableArchetype::MultipleVariables(ref mut tbl) = doc.data_sheets[0].archetype {
        tbl.variables[1].set_calculated_formula("=Y + 1");
    }
    // Also alter Gain before recalc so we confirm EVEN non-cyclic sheets do not partially update on Err.
    doc.info_sheets[0].set_constant("Gain", ConstantValue::Number(99.0), None).expect("set gain");
    let before_cycle = doc.clone();

    let err = recalculate_project(&mut doc).expect_err("formula cycle must fail");
    assert!(err.to_string().contains("failed") || err.to_string().to_lowercase().contains("circular"));
    assert_eq!(doc, before_cycle, "doc must roll back cleanly with zero partial mutation");
}

#[test]
fn hook_source_bindings_info_constant_result_scalar_and_column_title_reactivity() {
    let mut doc = ProjectDocument::new("HookSource Reactivity").expect("doc");

    let hot_conc = 3.0_f64;
    let hot_kd = 1.0_f64; // 1 + hot_conc/hot_kd = 4 => log10(4) offset
    let log_ki_true = -8.0_f64;
    let expected_log_ic50 = log_ki_true + (1.0 + hot_conc / hot_kd).log10();
    let xs = [-11.0, -10.0, -9.0, -8.0, -7.0, -6.0, -5.0];
    let top = 120.0_f64;
    let bottom = 20.0_f64;

    let mut info = InfoSheet::new_preseeded("info-hook", "Receptor Assay").expect("info");
    info.set_constant("HotConc_nM", ConstantValue::Number(hot_conc), None).expect("hot conc");
    info.set_constant("HotKd_nM", ConstantValue::Number(hot_kd), None).expect("hot kd");
    doc.add_info_sheet(info).expect("add info");

    // DataSheet 1: inhibition curve feeding Transform -> Fit 1 (tests InfoSheet inheritance through Transform!)
    let mut xy1 =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, xs.len()).expect("xy1");
    xy1.data_sets[0].title = String::from("Comp");
    for (r, &x) in xs.iter().enumerate() {
        let y = bottom + (top - bottom) / (1.0 + 10_f64.powf(x - expected_log_ic50));
        xy1.set_x_cell(r, Cell::number(x)).expect("set x");
        xy1.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let mut ds1 = DataSheet::new("ds-comp", "Competition", TableArchetype::Xy(xy1)).expect("ds1");
    ds1.info_sheet_id = Some(SheetId::new("info-hook"));
    doc.add_data_sheet(ds1).expect("add ds1");

    // Identity-like LinearScale(1, 0) Transform produces analyzable_data whose own info_sheet_id may be None
    let tr_spec = AnalysisSpec::new(
        "res-scale",
        "ds-comp",
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: Some(MathTransformPreset::LinearScale { slope: 1.0, intercept: 0.0 }),
            error_propagation: curvebook_doc::ErrorPropagationMode::FirstOrderDeltaMethod,
        }),
    )
    .expect("tr spec");
    doc.add_result_sheet(ResultSheet::new(tr_spec).expect("tr res")).expect("add tr");

    // Fit 1 analyzes Transform `res-scale` and hooks `InfoConstant { info_sheet_id: None, .. }`
    let fit1_spec = AnalysisSpec::new(
        "res-ki",
        "res-scale",
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            weighting: WeightingScheme::None,
            parameter_rules: vec![
                ParameterRule {
                    name: String::from("HotConc_nM"),
                    scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                        info_sheet_id: None,
                        constant_name: String::from("HotConc_nM"),
                    }),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                },
                ParameterRule {
                    name: String::from("HotKd_nM"),
                    scope: ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("HotKd_nM") }),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                },
            ],
            ..NonlinearFitSpec::new("one_site_fit_ki")
        }),
    )
    .expect("fit1 spec");
    doc.add_result_sheet(ResultSheet::new(fit1_spec).expect("fit1 res")).expect("add fit1");

    // DataSheet 2 + Fit 2: `expr: A * X / (B + X)` with `A` hooked to ColumnTitleValue ("10")
    // and `B` hooked to ResultScalar from Fit 1 ("Top" == 120.0)!
    let mut xy2 = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 5).expect("xy2");
    xy2.data_sets[0].title = String::from("10");
    let x_enz = [10.0, 20.0, 50.0, 120.0, 240.0];
    for (r, &x) in x_enz.iter().enumerate() {
        xy2.set_x_cell(r, Cell::number(x)).expect("set x2");
        // y = 10 * x / (120 + x) + 1 -> let free param C absorb +1 or fit C*A*X/(B+X)
        xy2.data_sets[0].cells[r][0] = Cell::number(10.0 * x / (120.0 + x));
    }
    doc.add_data_sheet(DataSheet::new("ds-enz", "Enzyme", TableArchetype::Xy(xy2)).expect("ds2")).expect("add ds2");

    let fit2_spec = AnalysisSpec::new(
        "res-chained",
        "ds-enz",
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            custom_dsl: Some(String::from("Y = K * A * X / (B + X)")),
            weighting: WeightingScheme::None,
            parameter_rules: vec![
                ParameterRule {
                    name: String::from("K"),
                    scope: ParameterScope::Local,
                    initial_override: Some(1.0),
                    lower_bound: Some(0.0),
                    upper_bound: Some(100.0),
                },
                ParameterRule {
                    name: String::from("A"),
                    scope: ParameterScope::HookedConstant(HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("ds-enz"), data_set_index: 0 }),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                },
                ParameterRule {
                    name: String::from("B"),
                    scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
                        result_sheet_id: SheetId::new("res-ki"),
                        parameter_key: String::from("Top"),
                        data_set_index: Some(0),
                    }),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                },
            ],
            ..NonlinearFitSpec::new("custom_enz")
        }),
    )
    .expect("fit2 spec");
    doc.add_result_sheet(ResultSheet::new(fit2_spec).expect("fit2 res")).expect("add fit2");

    recalculate_project(&mut doc).expect("hook recalc v1");
    let res_ki_1 = doc.result_sheet(&SheetId::new("res-ki")).expect("res ki 1");
    assert_eq!(res_ki_1.status, ResultStatus::UpToDate);
    approx_eq(res_ki_1.lookup_scalar("LogKi", Some(0)).expect("logki 1"), log_ki_true, 1e-4, "LogKi 1");
    approx_eq(res_ki_1.lookup_scalar("Top", Some(0)).expect("top 1"), 120.0, 1e-4, "Top 1");

    let res_ch_1 = doc.result_sheet(&SheetId::new("res-chained")).expect("res chained 1");
    assert_eq!(res_ch_1.status, ResultStatus::UpToDate);
    approx_eq(res_ch_1.lookup_scalar("K", Some(0)).expect("K1"), 1.0, 1e-4, "K1 == 1.0");

    // Now update:
    // 1) Info constant `HotConc_nM`: 3.0 -> 9.0 (1 + 9/1 = 10 => LogKi shifts left by log10(10/4) ~ -0.39794)
    // 2) `ds-enz` column title: "10" -> "20" (A doubles, so fitted K halves to 0.5!)
    doc.info_sheets[0].set_constant("HotConc_nM", ConstantValue::Number(9.0), None).expect("update hot conc");
    if let TableArchetype::Xy(ref mut tbl) = doc.data_sheets[1].archetype {
        tbl.data_sets[0].title = String::from("20");
    }

    recalculate_project(&mut doc).expect("hook recalc v2");
    let res_ki_2 = doc.result_sheet(&SheetId::new("res-ki")).expect("res ki 2");
    let expected_log_ki_2 = expected_log_ic50 - (1.0 + 9.0 / hot_kd).log10();
    approx_eq(res_ki_2.lookup_scalar("LogKi", Some(0)).expect("logki 2"), expected_log_ki_2, 1e-4, "LogKi shifted when InfoConstant updated");

    let res_ch_2 = doc.result_sheet(&SheetId::new("res-chained")).expect("res chained 2");
    approx_eq(res_ch_2.lookup_scalar("K", Some(0)).expect("K2"), 0.5, 1e-4, "K halved when ColumnTitleValue doubled");
}

#[test]
fn reactive_linked_region_and_transpose_propagation_without_duplication() {
    let mut doc = ProjectDocument::new("Linked Region Reactivity").expect("doc");

    let mut info = InfoSheet::new_preseeded("info-link", "Notes").expect("info");
    info.notes_markdown = String::from("Initial notes.");
    doc.add_info_sheet(info).expect("add info");

    // Upstream 2-row x 3-col Column table, transposed into 3-row x 2-col target table
    let mut src_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2).expect("src col");
    src_tbl.data_sets[0].title = String::from("S1");
    src_tbl.data_sets[1].title = String::from("S2");
    src_tbl.data_sets[2].title = String::from("S3");
    let r0 = [10.0, 20.0, 30.0];
    let r1 = [12.0, 22.0, 32.0];
    for c in 0..3 {
        src_tbl.data_sets[c].cells[0][0] = Cell::number(r0[c]);
        src_tbl.data_sets[c].cells[1][0] = Cell::number(r1[c]);
    }
    let src_ds = DataSheet::new("ds-src", "Source", TableArchetype::Column(src_tbl)).expect("src ds");

    let dst_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).expect("dst col");
    let mut dst_ds = DataSheet::new("ds-linked", "Transposed Link", TableArchetype::Column(dst_tbl)).expect("dst ds");

    let link_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("ds-src"),
            kind: InternalLinkKind::DataSheet,
            tab_id: None,
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 2, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::Overwrite,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: true,
        },
    };
    apply_import_link_pipeline(
        &mut dst_ds,
        &link_spec,
        ImportSourcePayload::InternalData(&src_ds),
        &ImportPipelineOptions { link_id: Some(String::from("lnk-tr")), ..ImportPipelineOptions::default() },
        Some(&mut doc.info_sheets[0]),
    )
    .expect("initial link pipeline");
    let notes_after_initial = doc.info_sheets[0].notes_markdown.clone();

    doc.add_data_sheet(src_ds).expect("add src");
    doc.add_data_sheet(dst_ds).expect("add dst");

    // Downstream stats analysis (One-way ANOVA / Descriptive stats) on `ds-linked`
    let desc_spec = AnalysisSpec::new("res-desc", "ds-linked", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("desc spec");
    doc.add_result_sheet(ResultSheet::new(desc_spec).expect("desc res")).expect("add desc");

    // ANOVA compares column 0 ([10, 20, 30]) vs column 1 ([12, 22, 32])
    let anova_spec = AnalysisSpec::new(
        "res-anova",
        "ds-linked",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..OneWayAnovaSpec::default() }),
    )
    .expect("anova spec");
    doc.add_result_sheet(ResultSheet::new(anova_spec).expect("anova res")).expect("add anova");

    recalculate_project(&mut doc).expect("recalc 1");
    let res1 = doc.result_sheet(&SheetId::new("res-desc")).expect("desc 1");
    approx_eq(res1.lookup_scalar("Mean", Some(0)).expect("mean col0 v1"), 20.0, 1e-12, "mean col0 v1");

    // Mutate upstream `ds-src`: double values AND exclude one value in row 0 col 2 (30 -> 60*)
    if let TableArchetype::Column(ref mut tbl) = doc.data_sheets[0].archetype {
        tbl.data_sets[0].cells[0][0] = Cell::number(20.0);
        tbl.data_sets[1].cells[0][0] = Cell::number(40.0);
        tbl.data_sets[2].cells[0][0] = Cell::excluded_number(60.0);
    }

    // Run recalculate_project TWICE to confirm idempotency & zero duplication of linked_regions/notes
    recalculate_project(&mut doc).expect("recalc 2");
    recalculate_project(&mut doc).expect("recalc 3");

    let linked_ds = doc.data_sheet(&SheetId::new("ds-linked")).expect("linked ds");
    assert_eq!(linked_ds.linked_regions.len(), 1, "must not duplicate linked_regions entries");
    assert_eq!(doc.info_sheets[0].notes_markdown, notes_after_initial, "recalc must not spam InfoSheet notes");

    if let TableArchetype::Column(ref tbl) = linked_ds.archetype {
        approx_eq(tbl.data_sets[0].cells[0][0].active_number().expect("r0c0"), 20.0, 1e-12, "tr r0");
        approx_eq(tbl.data_sets[0].cells[1][0].active_number().expect("r1c0"), 40.0, 1e-12, "tr r1");
        assert!(tbl.data_sets[0].cells[2][0].excluded, "exclusion propagated across transpose");
    } else {
        panic!("expected Column");
    }

    let res2 = doc.result_sheet(&SheetId::new("res-desc")).expect("desc 2");
    // Column 0 active values are now [20.0, 40.0] -> Mean = 30.0
    approx_eq(res2.lookup_scalar("Mean", Some(0)).expect("mean col0 v2"), 30.0, 1e-12, "mean col0 v2");

    // Verify ResultSheet -> LinkedDataRegion (AnalyzableResultSheet) + AppendRows in-place refresh
    let tr_spec = AnalysisSpec::new(
        "res-src-tr",
        "ds-src",
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: Some(MathTransformPreset::LinearScale { slope: 10.0, intercept: 0.0 }),
            error_propagation: curvebook_doc::ErrorPropagationMode::FirstOrderDeltaMethod,
        }),
    )
    .expect("tr spec");
    doc.add_result_sheet(ResultSheet::new(tr_spec).expect("tr res")).expect("add tr res");

    let mut append_ds = DataSheet::new(
        "ds-append",
        "Appended From Result",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).expect("app")),
    )
    .expect("append ds");
    append_ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(999.0)).expect("header row 0");
    recalculate_project(&mut doc).expect("populate res-src-tr before initial append");

    let append_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("res-src-tr"),
            kind: InternalLinkKind::AnalyzableResultSheet,
            tab_id: None,
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 2, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::AppendRows,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    {
        let res_sheet_ref = doc.result_sheet(&SheetId::new("res-src-tr")).expect("res src tr");
        let analyzable = res_sheet_ref.analyzable_data.as_ref().expect("analyzable transform table");
        apply_import_link_pipeline(
            &mut append_ds,
            &append_spec,
            ImportSourcePayload::InternalData(analyzable),
            &ImportPipelineOptions { link_id: Some(String::from("lnk-append")), ..ImportPipelineOptions::default() },
            None,
        )
        .expect("initial append link");
    }
    let initial_range = append_ds.linked_regions[0].target_range;
    doc.add_data_sheet(append_ds).expect("add append ds");

    // Mutate `ds-src` row 0 col 0 from 20 -> 50 -> res-src-tr becomes 500 -> ds-append refreshes in-place!
    if let TableArchetype::Column(ref mut tbl) = doc.data_sheets[0].archetype {
        tbl.data_sets[0].cells[0][0] = Cell::number(50.0);
    }
    recalculate_project(&mut doc).expect("recalc append round 1");
    recalculate_project(&mut doc).expect("recalc append round 2");

    let append_after = doc.data_sheet(&SheetId::new("ds-append")).expect("append after");
    assert_eq!(append_after.linked_regions.len(), 1);
    assert_eq!(
        append_after.linked_regions[0].target_range, initial_range,
        "AppendRows link refresh must update target_range in-place without drifting downward"
    );
    if let TableArchetype::Column(ref tbl) = append_after.archetype {
        // Row 0 manual seed untouched:
        approx_eq(tbl.data_sets[0].cells[0][0].active_number().expect("seed"), 999.0, 1e-12, "row 0 seed");
        // Appended row 1 got 50 * 10 = 500 cleanly in-place:
        let appended_row = initial_range.start.row;
        approx_eq(tbl.data_sets[0].cells[appended_row][0].active_number().expect("appended"), 500.0, 1e-12, "appended row refreshed in-place");
    } else {
        panic!("expected Column");
    }
}

#[test]
fn preserves_graph_styles_gdos_axis_breaks_brackets_and_annotations() {
    let mut doc = ProjectDocument::new("Graph Preservation").expect("doc");

    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).expect("col tbl");
    tbl.data_sets[0].title = String::from("Vehicle");
    tbl.data_sets[1].title = String::from("Treated");
    for (r, &v) in [10.0, 11.0, 9.5, 10.5].iter().enumerate() {
        tbl.data_sets[0].cells[r][0] = Cell::number(v);
    }
    for (r, &v) in [20.0, 21.0, 19.5, 20.5].iter().enumerate() {
        tbl.data_sets[1].cells[r][0] = Cell::number(v);
    }
    doc.add_data_sheet(DataSheet::new("ds-col", "Cohort", TableArchetype::Column(tbl)).expect("ds")).expect("add ds");

    let anova_spec = AnalysisSpec::new(
        "res-anova",
        "ds-col",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ..OneWayAnovaSpec::default()
        }),
    )
    .expect("anova");
    doc.add_result_sheet(ResultSheet::new(anova_spec).expect("anova res")).expect("add anova");

    let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-box", &doc.data_sheets[0]).expect("graph");
    gsheet.initialized = true;
    gsheet.spec.palette = PaletteId::PaulTolVibrant;
    gsheet.spec.y_axis = AxisSpec {
        segments: vec![
            AxisSegment {
                min: Some(0.0),
                max: Some(15.0),
                scale: AxisScale::Linear,
                length_share: 0.4,
                major_tick_interval: Some(5.0),
                minor_ticks_per_interval: 1,
            },
            AxisSegment {
                min: Some(18.0),
                max: Some(60.0),
                scale: AxisScale::Linear,
                length_share: 0.6,
                major_tick_interval: Some(10.0),
                minor_ticks_per_interval: 2,
            },
        ],
        tick_format: TickLabelFormat::Decimal,
        ..AxisSpec::linear("Response")
    };
    for s in &mut gsheet.spec.series {
        s.gdo = GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::Tukey1_5Iqr,
            fill_color: Rgba8::WHITE,
            stroke_color: Rgba8::BLACK,
            show_outliers: true,
            show_all_points: true,
        };
    }
    gsheet.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res-anova"),
        group_a: 0,
        group_b: 1,
        style: BracketStyle::CurlyBrace,
        label_mode: BracketLabelMode::SummaryStars,
        hide_if_non_significant: false,
        stroke_width_pt: 1.2,
        font_size_pt: 9.0,
        bracket_step_pt: 14.0,
    });
    gsheet.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(15.0, 20.0, 110.0, 25.0),
        markdown: String::from("Group comparison"),
        bordered: true,
    });
    gsheet.floating_notes.push(FloatingNote::new("note-keep", RectPt::new(5.0, 5.0, 80.0, 24.0), "Preserved note"));
    gsheet.color_tag = Some(Rgba8::rgb(200, 100, 50));

    let expected_spec = gsheet.spec.clone();
    let expected_notes = gsheet.floating_notes.clone();
    let expected_tag = gsheet.color_tag;
    doc.add_graph_sheet(gsheet).expect("add graph");

    recalculate_project(&mut doc).expect("recalc 1");
    let scene1 = compile_graph(&doc.graph_sheets[0], &doc).expect("compile graph 1");

    // Mutate Treated values upward dramatically (20 -> 50)
    if let TableArchetype::Column(ref mut tbl) = doc.data_sheets[0].archetype {
        for (r, &v) in [50.0, 51.0, 49.5, 50.5].iter().enumerate() {
            tbl.data_sets[1].cells[r][0] = Cell::number(v);
        }
    }
    let report = recalculate_project_with_report(&mut doc).expect("recalc 2");
    assert_eq!(report.preserved_graph_sheets, vec![SheetId::new("graph-box")]);

    let g_after = &doc.graph_sheets[0];
    assert!(g_after.initialized);
    assert_eq!(g_after.spec, expected_spec);
    assert_eq!(g_after.floating_notes, expected_notes);
    assert_eq!(g_after.color_tag, expected_tag);

    let scene2 = compile_graph(g_after, &doc).expect("compile graph 2");
    assert_ne!(scene1.primitives, scene2.primitives);
}

#[test]
fn frozen_sheet_protection_skips_links_and_formulas_while_propagating_downstream() {
    let mut doc = ProjectDocument::new("Frozen Sheet Protection").expect("doc");

    let mut info = InfoSheet::new_preseeded("info-frz", "Frozen Test").expect("info");
    info.set_constant("Factor", ConstantValue::Number(2.0), None).expect("factor");
    doc.add_info_sheet(info).expect("add info");

    // 1. Source sheet -> Frozen linked sheet -> Unfrozen downstream DescriptiveStats
    let mut src_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).expect("src");
    src_tbl.data_sets[0].cells[0][0] = Cell::number(10.0);
    src_tbl.data_sets[0].cells[1][0] = Cell::number(20.0);
    src_tbl.data_sets[0].cells[2][0] = Cell::number(30.0);
    let src_ds = DataSheet::new("ds-src", "Src", TableArchetype::Column(src_tbl)).expect("src ds");

    let lnk_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).expect("lnk");
    let mut lnk_ds = DataSheet::new("ds-frozen-link", "Frozen Link", TableArchetype::Column(lnk_tbl)).expect("lnk ds");
    let link_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("ds-src"),
            kind: InternalLinkKind::DataSheet,
            tab_id: None,
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 0, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::Overwrite,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    apply_import_link_pipeline(&mut lnk_ds, &link_spec, ImportSourcePayload::InternalData(&src_ds), &ImportPipelineOptions::default(), None)
        .expect("seed link");

    // Freeze linked sheet
    lnk_ds.frozen = true;

    // 2. Also create a frozen MultipleVariables sheet with a calculated variable referencing [Factor]
    let mut mv = MultipleVariablesTable::new(2, 3).expect("mv");
    mv.variables[0].title = String::from("X");
    mv.variables[1].title = String::from("Scaled");
    mv.variables[1].set_calculated_formula("=X * [Factor]");
    for r in 0..3 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[1].cells[r] = Cell::number(((r + 1) * 2) as f64);
    }
    let mut mv_ds = DataSheet::new("ds-frozen-mv", "Frozen MV", TableArchetype::MultipleVariables(mv)).expect("mv ds");
    mv_ds.info_sheet_id = Some(SheetId::new("info-frz"));
    mv_ds.frozen = true;

    doc.add_data_sheet(src_ds).expect("add src");
    doc.add_data_sheet(lnk_ds).expect("add frozen link");
    doc.add_data_sheet(mv_ds).expect("add frozen mv");

    // Downstream analyses reading FROM frozen sheets (must STILL run and propagate frozen values!)
    let desc_spec =
        AnalysisSpec::new("res-from-frozen-link", "ds-frozen-link", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("desc spec");
    doc.add_result_sheet(ResultSheet::new(desc_spec).expect("desc res")).expect("add desc");

    let mut lin_spec =
        AnalysisSpec::new("res-from-frozen-mv", "ds-frozen-mv", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).expect("lin spec");
    lin_spec.selected_data_sets = vec![0, 1];
    doc.add_result_sheet(ResultSheet::new(lin_spec).expect("lin res")).expect("add lin");

    // Mutate `ds-src` (10,20,30 -> 100,200,300) AND `info-frz` Factor (2 -> 10) while sheets are frozen
    if let TableArchetype::Column(ref mut tbl) = doc.data_sheets[0].archetype {
        tbl.data_sets[0].cells[0][0] = Cell::number(100.0);
        tbl.data_sets[0].cells[1][0] = Cell::number(200.0);
        tbl.data_sets[0].cells[2][0] = Cell::number(300.0);
    }
    doc.info_sheets[0].set_constant("Factor", ConstantValue::Number(10.0), None).expect("factor 10");

    let rep_frozen = recalculate_project_with_report(&mut doc).expect("recalc with frozen");
    assert!(rep_frozen.skipped_frozen_sheets.contains(&SheetId::new("ds-frozen-link")));
    assert!(rep_frozen.skipped_frozen_sheets.contains(&SheetId::new("ds-frozen-mv")));
    assert!(!rep_frozen.updated_data_sheets.contains(&SheetId::new("ds-frozen-link")));
    assert!(!rep_frozen.updated_data_sheets.contains(&SheetId::new("ds-frozen-mv")));

    // Frozen sheets kept old values:
    let frz_lnk = doc.data_sheet(&SheetId::new("ds-frozen-link")).expect("frz lnk");
    if let TableArchetype::Column(ref tbl) = frz_lnk.archetype {
        approx_eq(tbl.data_sets[0].cells[0][0].active_number().expect("c0"), 10.0, 1e-12, "frozen link unchanged");
    }
    let frz_mv = doc.data_sheet(&SheetId::new("ds-frozen-mv")).expect("frz mv");
    if let TableArchetype::MultipleVariables(ref tbl) = frz_mv.archetype {
        approx_eq(tbl.variables[1].cells[0].active_number().expect("mv0"), 2.0, 1e-12, "frozen formula unchanged");
    }

    // Downstream ResultSheets STILL propagated from the frozen snapshots:
    let res_lnk = doc.result_sheet(&SheetId::new("res-from-frozen-link")).expect("res lnk");
    assert_eq!(res_lnk.status, ResultStatus::UpToDate);
    approx_eq(res_lnk.lookup_scalar("Mean", Some(0)).expect("mean frozen"), 20.0, 1e-12, "downstream uses frozen link");

    let res_mv = doc.result_sheet(&SheetId::new("res-from-frozen-mv")).expect("res mv");
    assert_eq!(res_mv.status, ResultStatus::UpToDate);
    approx_eq(res_mv.lookup_scalar("Slope", Some(0)).expect("slope frozen"), 2.0, 1e-9, "downstream uses frozen mv");

    // Unfreeze both -> recalculate now refreshes both sheets AND downstream ResultSheets!
    doc.data_sheets[1].frozen = false;
    doc.data_sheets[2].frozen = false;
    let rep_unfrozen = recalculate_project_with_report(&mut doc).expect("recalc after unfreeze");
    assert!(rep_unfrozen.skipped_frozen_sheets.is_empty());
    assert!(rep_unfrozen.updated_data_sheets.contains(&SheetId::new("ds-frozen-link")));
    assert!(rep_unfrozen.updated_data_sheets.contains(&SheetId::new("ds-frozen-mv")));

    let res_lnk_after = doc.result_sheet(&SheetId::new("res-from-frozen-link")).expect("res lnk after");
    approx_eq(res_lnk_after.lookup_scalar("Mean", Some(0)).expect("mean unfrozen"), 200.0, 1e-12, "unfrozen mean");

    let res_mv_after = doc.result_sheet(&SheetId::new("res-from-frozen-mv")).expect("res mv after");
    approx_eq(res_mv_after.lookup_scalar("Slope", Some(0)).expect("slope unfrozen"), 10.0, 1e-9, "unfrozen slope");
}

#[test]
fn cycle_detection_and_atomic_rollback_across_all_topologies() {
    fn base_doc_with_valid_dirty_sheet() -> ProjectDocument {
        let mut doc = ProjectDocument::new("Cycle Rollback").expect("doc");
        let mut mv = MultipleVariablesTable::new(2, 2).expect("mv");
        mv.variables[0].title = String::from("X");
        mv.variables[1].title = String::from("Y");
        mv.variables[1].set_calculated_formula("=X * 3");
        mv.variables[0].cells[0] = Cell::number(5.0);
        mv.variables[0].cells[1] = Cell::number(7.0);
        doc.add_data_sheet(DataSheet::new("ds-valid", "Valid", TableArchetype::MultipleVariables(mv)).expect("mv ds")).expect("add valid");
        doc
    }

    // a) Direct 2-node DataSheet link cycle (A <-> B)
    let mut doc_link_cycle = base_doc_with_valid_dirty_sheet();
    let mut ds_a =
        DataSheet::new("ds-a", "A", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("t"))).expect("ds a");
    let mut ds_b =
        DataSheet::new("ds-b", "B", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("t"))).expect("ds b");
    ds_a.linked_regions.push(curvebook_doc::LinkedDataRegion {
        id: String::from("l1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("ds-b"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("b->a"),
    });
    ds_b.linked_regions.push(curvebook_doc::LinkedDataRegion {
        id: String::from("l2"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("ds-a"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("a->b"),
    });
    doc_link_cycle.add_data_sheet(ds_a).expect("add a");
    doc_link_cycle.add_data_sheet(ds_b).expect("add b");
    let snap = doc_link_cycle.clone();
    assert!(topological_order(&doc_link_cycle).is_err());
    assert!(recalculate_project(&mut doc_link_cycle).is_err());
    assert_eq!(doc_link_cycle, snap);

    // b) ResultSheet <-> ResultSheet HookSource::ResultScalar mutual cycle
    let mut doc_hook_cycle = base_doc_with_valid_dirty_sheet();
    let ds_xy = make_dose_response_sheet("ds-xy", None, &[-8.0, -7.0, -6.0, -5.0], [0.0, 100.0, -6.5, 1.0]);
    doc_hook_cycle.add_data_sheet(ds_xy).expect("add xy");

    let r1 = AnalysisSpec::new(
        "res-1",
        "ds-xy",
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            parameter_rules: vec![ParameterRule {
                name: String::from("Top"),
                scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
                    result_sheet_id: SheetId::new("res-2"),
                    parameter_key: String::from("Top"),
                    data_set_index: Some(0),
                }),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            }],
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        }),
    )
    .expect("r1");
    let r2 = AnalysisSpec::new(
        "res-2",
        "ds-xy",
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            parameter_rules: vec![ParameterRule {
                name: String::from("Top"),
                scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
                    result_sheet_id: SheetId::new("res-1"),
                    parameter_key: String::from("Top"),
                    data_set_index: Some(0),
                }),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            }],
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        }),
    )
    .expect("r2");
    doc_hook_cycle.add_result_sheet(ResultSheet::new(r1).expect("res1")).expect("add r1");
    doc_hook_cycle.add_result_sheet(ResultSheet::new(r2).expect("res2")).expect("add r2");
    let snap = doc_hook_cycle.clone();
    assert!(recalculate_project(&mut doc_hook_cycle).is_err());
    assert_eq!(doc_hook_cycle, snap);

    // c) Mixed DataSheet -> Transform ResultSheet -> LinkedDataRegion -> DataSheet cycle
    let mut doc_mixed_cycle = base_doc_with_valid_dirty_sheet();
    let mut ds_m =
        DataSheet::new("ds-m", "M", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("t"))).expect("ds m");
    ds_m.linked_regions.push(curvebook_doc::LinkedDataRegion {
        id: String::from("l-res"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("res-m"),
                kind: InternalLinkKind::AnalyzableResultSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("res->ds"),
    });
    doc_mixed_cycle.add_data_sheet(ds_m).expect("add ds_m");
    let res_m = AnalysisSpec::new("res-m", "ds-m", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false }))
        .expect("res m");
    doc_mixed_cycle.add_result_sheet(ResultSheet::new(res_m).expect("rm")).expect("add rm");
    let snap = doc_mixed_cycle.clone();
    assert!(recalculate_project(&mut doc_mixed_cycle).is_err());
    assert_eq!(doc_mixed_cycle, snap);

    // d) Self-referential GraphSheet and LayoutSheet detection
    let mut doc_self_graph = ProjectDocument::new("Self Graph").expect("doc");
    let ds_g =
        DataSheet::new("ds-g", "DG", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("t"))).expect("ds g");
    let mut gs = GraphSheet::new_uninitialized_for_data("graph-self", &ds_g).expect("gs");
    gs.spec.primary_source_sheet_id = SheetId::new("graph-self");
    doc_self_graph.add_data_sheet(ds_g).expect("add ds g");
    doc_self_graph.graph_sheets.push(gs);
    let snap = doc_self_graph.clone();
    assert!(recalculate_project(&mut doc_self_graph).is_err());
    assert_eq!(doc_self_graph, snap);

    let mut doc_self_layout = ProjectDocument::new("Self Layout").expect("doc");
    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveTable {
        id: String::from("item-1"),
        source_sheet_id: SheetId::new("layout-self"),
        tab_id: None,
        bounds_pt: RectPt::new(0.0, 0.0, 100.0, 100.0),
        selected_rows: None,
        selected_cols: None,
        style: curvebook_doc::EmbeddedTableStyle::default(),
    });
    doc_self_layout.layout_sheets.push(LayoutSheet::new("layout-self", "Self", lspec).expect("layout"));
    let snap = doc_self_layout.clone();
    assert!(recalculate_project(&mut doc_self_layout).is_err());
    assert_eq!(doc_self_layout, snap);
}

#[test]
fn session_guarded_recalculate_and_panic_rollback() {
    let mut doc = ProjectDocument::new("Session Guard").expect("doc");
    let mut mv = MultipleVariablesTable::new(2, 2).expect("mv");
    mv.variables[0].title = String::from("A");
    mv.variables[1].title = String::from("B");
    mv.variables[1].set_calculated_formula("=A + 10");
    mv.variables[0].cells[0] = Cell::number(5.0);
    mv.variables[0].cells[1] = Cell::number(15.0);
    doc.add_data_sheet(DataSheet::new("ds-1", "S1", TableArchetype::MultipleVariables(mv)).expect("ds")).expect("add ds");

    let mut session = Session::from_document(doc);
    session.recalculate_all().expect("recalculate_all");
    if let TableArchetype::MultipleVariables(ref tbl) = session.document().data_sheets[0].archetype {
        approx_eq(tbl.variables[1].cells[0].active_number().expect("b0"), 15.0, 1e-12, "b0");
        approx_eq(tbl.variables[1].cells[1].active_number().expect("b1"), 25.0, 1e-12, "b1");
    }

    // Verify `run_guarded` turns internal panic into EngineError cleanly for caller rollback
    let before = session.document().clone();
    let mut scratch = before.clone();
    let res: Result<(), EngineError> = run_guarded("dag.synthetic_panic", || {
        scratch.data_sheets.clear();
        panic!("simulated downstream panic");
    });
    let err = res.expect_err("panic must yield Err(EngineError)");
    assert!(err.to_string().contains("dag.synthetic_panic"));
    assert!(err.to_string().contains("simulated downstream panic"));
    assert_eq!(session.document(), &before);
}

#[test]
fn transient_failed_analysis_preserves_result_sheet_floating_notes() {
    use curvebook_doc::{TwoGroupMethod, TwoGroupSpec};
    use curvebook_format::{open_project_bytes, save_project_bytes};

    let mut doc = ProjectDocument::new("Floating Note Recovery").expect("doc");
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).expect("col");
    tbl.data_sets[0].title = String::from("Control");
    tbl.data_sets[1].title = String::from("Treated");
    tbl.data_sets[0].cells[0][0] = Cell::number(10.0);
    tbl.data_sets[0].cells[1][0] = Cell::number(12.0);
    tbl.data_sets[1].cells[0][0] = Cell::number(20.0);
    tbl.data_sets[1].cells[1][0] = Cell::number(22.0);
    doc.add_data_sheet(DataSheet::new("ds-col", "Cohort", TableArchetype::Column(tbl)).expect("ds")).expect("add ds");

    let spec = AnalysisSpec::new(
        "res-t",
        "ds-col",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
    )
    .expect("spec");
    doc.add_result_sheet(ResultSheet::new(spec).expect("res")).expect("add res");

    recalculate_project(&mut doc).expect("initial recalc");
    let expected_note = FloatingNote::new("res-note-1", RectPt::new(24.0, 32.0, 180.0, 64.0), "Verify baseline outlier exclusion");
    doc.result_sheets[0].floating_notes.push(expected_note.clone());
    assert_eq!(doc.result_sheets[0].status, ResultStatus::UpToDate);
    assert!(!doc.result_sheets[0].tabs.is_empty());

    // Round-trip through native archive serialization before the transient failure.
    let saved_bytes = save_project_bytes(&doc).expect("save bytes");
    let mut session = Session::from_document(open_project_bytes(&saved_bytes).expect("open bytes"));

    // Exclude one observation in Control so only 1 sample remains and the t-test transiently fails.
    session.execute("table.excludeCells", json_exclude("ds-col", 1, 0, true)).expect("exclude cell triggers reactive recalculation");
    let failed = session.document().result_sheet(&SheetId::new("res-t")).expect("res after exclude");
    assert!(matches!(failed.status, ResultStatus::Failed { .. }), "expected Failed status, got {:?}", failed.status);
    assert!(failed.tabs.is_empty(), "derived tabs must clear on failure");
    assert!(failed.scalars.is_empty(), "derived scalars must clear on failure");
    assert_eq!(failed.floating_notes, vec![expected_note.clone()], "floating notes must survive transient analysis failure");

    // Unexclude the observation and verify the analysis recovers while retaining the floating note.
    session.execute("table.excludeCells", json_exclude("ds-col", 1, 0, false)).expect("unexclude cell restores analysis");
    let recovered = session.document().result_sheet(&SheetId::new("res-t")).expect("res after unexclude");
    assert_eq!(recovered.status, ResultStatus::UpToDate);
    assert!(!recovered.tabs.is_empty());
    assert!(!recovered.scalars.is_empty());
    assert_eq!(recovered.floating_notes, vec![expected_note], "floating notes must remain attached after analysis recovery");
}

fn json_exclude(sheet_id: &str, row: usize, col: usize, excluded: bool) -> serde_json::Value {
    serde_json::json!({
        "sheet_id": sheet_id,
        "coords": [{ "row": row, "col": col }],
        "excluded": excluded
    })
}
