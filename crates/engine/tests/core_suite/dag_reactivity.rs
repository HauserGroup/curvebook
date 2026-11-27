//! Integration tests for reactive `Data -> Analysis -> Graph -> Layout` DAG recalculation,
//! linked data regions, calculated variables, `HookSource` bindings, frozen sheet protection,
//! cycle rejection rollback, and downstream graph style/annotation preservation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::AtomicBool;

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisSpec, BracketLabelMode, BracketStyle, Cell, CellCoord, ConstantValue, DataSheet,
    DescriptiveSpec, EmbeddedTableStyle, ErrorCode, ErrorPropagationMode, FloatingNote, GraphAnnotation, GraphSheet, GraphicalDataObject, HookSource,
    ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutItem, LayoutSheet, LayoutSpec, LegendSpec,
    LinearRegressionSpec, LinkSource, LinkedDataRegion, MathTransformPreset, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec,
    NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, OneWayMethod, PairwiseBracketSpec, ParameterRule, ParameterScope, PlacementMode,
    ProjectDocument, RectPt, RegressionOverlaySpec, ReplicateLineMode, ResultSheet, ResultStatus, SheetCellRange, SheetId, TableArchetype,
    TickLabelFormat, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, WeightingScheme, WhiskerMethod,
};
use curvebook_engine::dag::{recalculate_project, recalculate_project_with_report, recalculate_project_with_report_cancellable, topological_order};
use curvebook_engine::guard::run_guarded;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::{Caller, EngineError, Session};
use curvebook_fit::get_builtin_model;
use curvebook_format::{open_project_bytes, save_project_bytes};
use curvebook_palette::{MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use curvebook_transform::{ImportPipelineOptions, ImportSourcePayload, apply_import_link_pipeline};
use serde_json::json;

use super::common::{approx_eq, assert_bad_input, assert_err_code, col_sheet, ok_cmd, rs, run_cmds, session_with, xy_sheet};

fn make_dose_response_sheet(id: &str, info_id: Option<&str>, xs: &[f64], true_params: [f64; 4]) -> DataSheet {
    let model = get_builtin_model("log_agonist_vs_response_4pl").expect("4PL builtin");
    let ys: Vec<f64> = xs.iter().map(|&x| model.eval(&[x], &true_params, 0).expect("4PL eval")).collect();
    let mut ds = xy_sheet(id, "Dose Response", "X", xs, &[("Agonist", &ys)]);
    ds.info_sheet_id = info_id.map(SheetId::new);
    ds
}

fn hook_rule(name: &str, src: HookSource) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::HookedConstant(src), initial_override: None, lower_bound: None, upper_bound: None }
}

fn internal_link(sheet_id: &str, kind: InternalLinkKind, end_row: usize, end_col: usize, mode: PlacementMode, transpose: bool) -> ImportLinkSpec {
    ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new(sheet_id),
            kind,
            tab_id: None,
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(end_row, end_col, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose,
        },
    }
}

#[test]
fn multi_hop_data_transform_fit_and_downstream_graph_reactivity() {
    let mut doc = ProjectDocument::new("Multi-hop DAG").unwrap();
    let mut info = InfoSheet::new_preseeded("info-assay", "Assay Metadata").unwrap();
    info.set_constant("ScaleMax", ConstantValue::Number(100.0), None).unwrap();
    info.set_constant("AssayTag", ConstantValue::Text(String::from("Batch-01")), None).unwrap();
    doc.add_info_sheet(info).unwrap();

    let xs = [-10.0, -9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    doc.add_data_sheet(make_dose_response_sheet("ds-dose", Some("info-assay"), &xs, [0.0, 100.0, -7.0, 1.0])).unwrap();

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
    .unwrap();
    doc.add_result_sheet(ResultSheet::new(norm_spec).unwrap()).unwrap();
    doc.add_result_sheet(
        ResultSheet::new(
            AnalysisSpec::new(
                "res-fit",
                "res-norm",
                AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                    weighting: WeightingScheme::None,
                    ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
                }),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &doc.data_sheets[0]).unwrap();
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
                ..AxisSegment::default()
            },
            AxisSegment {
                min: Some(-6.5),
                max: Some(-3.5),
                scale: AxisScale::Linear,
                length_share: 0.55,
                major_tick_interval: Some(1.0),
                minor_ticks_per_interval: 1,
                ..AxisSegment::default()
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
        replicate_lines: ReplicateLineMode::default(),
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
    let (expected_spec, expected_notes, expected_tag) = (gsheet.spec.clone(), gsheet.floating_notes.clone(), gsheet.color_tag);
    doc.add_graph_sheet(gsheet).unwrap();

    let report_v1 = recalculate_project_with_report(&mut doc).unwrap();
    let pos = |id: &str| report_v1.evaluated_order.iter().position(|s| s.as_str() == id).unwrap();
    assert!(
        pos("info-assay") < pos("ds-dose") && pos("ds-dose") < pos("res-norm") && pos("res-norm") < pos("res-fit") && pos("res-fit") < pos("graph-1")
    );
    assert_eq!(report_v1.preserved_graph_sheets, vec![SheetId::new("graph-1")]);

    let fit_v1 = doc.result_sheet(&SheetId::new("res-fit")).unwrap();
    assert_eq!(fit_v1.status, ResultStatus::UpToDate);
    approx_eq(fit_v1.lookup_scalar("LogEC50", Some(0)).unwrap(), -7.0, 1e-4, "v1 LogEC50");
    approx_eq(fit_v1.lookup_scalar("Top", Some(0)).unwrap(), 100.0, 1e-4, "v1 Top");
    let scene_v1 = compile_graph(&doc.graph_sheets[0], &doc).unwrap();

    // Mutate DataSheet (LogEC50 -7 -> -6) and InfoSheet ScaleMax (100 -> 200).
    doc.data_sheets[0].archetype = make_dose_response_sheet("ds-dose", Some("info-assay"), &xs, [0.0, 100.0, -6.0, 1.0]).archetype;
    doc.info_sheets[0].set_constant("ScaleMax", ConstantValue::Number(200.0), None).unwrap();
    assert!(recalculate_project_with_report(&mut doc).unwrap().failed_result_sheets.is_empty());

    let fit_v2 = doc.result_sheet(&SheetId::new("res-fit")).unwrap();
    assert_eq!(fit_v2.status, ResultStatus::UpToDate);
    approx_eq(fit_v2.lookup_scalar("LogEC50", Some(0)).unwrap(), -6.0, 1e-4, "v2 LogEC50");
    approx_eq(fit_v2.lookup_scalar("Top", Some(0)).unwrap(), 50.0, 1e-4, "v2 Top");

    let g_after = &doc.graph_sheets[0];
    assert!(g_after.initialized && g_after.spec == expected_spec && g_after.floating_notes == expected_notes && g_after.color_tag == expected_tag);
    assert_ne!(scene_v1.primitives, compile_graph(g_after, &doc).unwrap().primitives);
}

#[test]
fn calculated_variable_propagation_and_formula_cycle_rollback() {
    let mut doc = ProjectDocument::new("Calc Var Propagation").unwrap();
    let mut info = InfoSheet::new_preseeded("info-mv", "Params").unwrap();
    info.set_constant("Gain", ConstantValue::Number(3.0), None).unwrap();
    info.set_constant("Bias", ConstantValue::Number(5.0), None).unwrap();
    doc.add_info_sheet(info).unwrap();

    let mut mv = MultipleVariablesTable::new(3, 5).unwrap();
    mv.variables[0].title = String::from("X");
    mv.variables[1].title = String::from("Mid");
    mv.variables[1].set_calculated_formula("=X * 2");
    mv.variables[2].title = String::from("Y");
    mv.variables[2].set_calculated_formula("=Mid + [Gain] + [Bias]");
    for r in 0..5 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
    }
    let mut ds_mv = DataSheet::new("ds-mv", "MV Calc", TableArchetype::MultipleVariables(mv)).unwrap();
    ds_mv.info_sheet_id = Some(SheetId::new("info-mv"));
    doc.add_data_sheet(ds_mv).unwrap();
    doc.add_data_sheet(col_sheet("ds-other", "Other", &[("A", &[42.0])])).unwrap();

    let mut lin_spec = AnalysisSpec::new("res-lin", "ds-mv", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    lin_spec.selected_data_sets = vec![0, 2];
    doc.add_result_sheet(ResultSheet::new(lin_spec).unwrap()).unwrap();

    assert!(recalculate_project_with_report(&mut doc).unwrap().updated_data_sheets.contains(&SheetId::new("ds-mv")));
    let TableArchetype::MultipleVariables(ref tbl) = doc.data_sheets[0].archetype else { panic!("expected MV") };
    approx_eq(tbl.variables[2].cells[0].active_number().unwrap(), 10.0, 1e-12, "y0");
    approx_eq(tbl.variables[2].cells[4].active_number().unwrap(), 18.0, 1e-12, "y4");
    approx_eq(doc.result_sheet(&SheetId::new("res-lin")).unwrap().lookup_scalar("Slope", Some(0)).unwrap(), 2.0, 1e-9, "slope 1");
    approx_eq(doc.result_sheet(&SheetId::new("res-lin")).unwrap().lookup_scalar("YIntercept", Some(0)).unwrap(), 8.0, 1e-9, "int 1");

    doc.info_sheets[0].set_constant("Gain", ConstantValue::Number(13.0), None).unwrap();
    recalculate_project(&mut doc).unwrap();
    approx_eq(doc.result_sheet(&SheetId::new("res-lin")).unwrap().lookup_scalar("Slope", Some(0)).unwrap(), 2.0, 1e-9, "slope 2");
    approx_eq(doc.result_sheet(&SheetId::new("res-lin")).unwrap().lookup_scalar("YIntercept", Some(0)).unwrap(), 18.0, 1e-9, "int 2");

    // Cyclic formula Mid = Y + 1 fails and rolls back whole document atomically.
    if let TableArchetype::MultipleVariables(ref mut tbl) = doc.data_sheets[0].archetype {
        tbl.variables[1].set_calculated_formula("=Y + 1");
    }
    doc.info_sheets[0].set_constant("Gain", ConstantValue::Number(99.0), None).unwrap();
    let before_cycle = doc.clone();
    let err = recalculate_project(&mut doc).expect_err("formula cycle must fail");
    assert!(err.to_string().contains("failed") || err.to_string().to_lowercase().contains("circular"));
    assert_eq!(doc, before_cycle);
}

#[test]
fn hook_source_bindings_info_constant_result_scalar_and_column_title_reactivity() {
    let mut doc = ProjectDocument::new("HookSource Reactivity").unwrap();
    let (hot_conc, hot_kd, log_ki_true) = (3.0_f64, 1.0_f64, -8.0_f64);
    let expected_log_ic50 = log_ki_true + (1.0 + hot_conc / hot_kd).log10();
    let xs = [-11.0, -10.0, -9.0, -8.0, -7.0, -6.0, -5.0];
    let ys1: Vec<f64> = xs.iter().map(|&x| 20.0 + 100.0 / (1.0 + 10_f64.powf(x - expected_log_ic50))).collect();

    let mut info = InfoSheet::new_preseeded("info-hook", "Receptor Assay").unwrap();
    info.set_constant("HotConc_nM", ConstantValue::Number(hot_conc), None).unwrap();
    info.set_constant("HotKd_nM", ConstantValue::Number(hot_kd), None).unwrap();
    doc.add_info_sheet(info).unwrap();

    let mut ds1 = xy_sheet("ds-comp", "Competition", "X", &xs, &[("Comp", &ys1)]);
    ds1.info_sheet_id = Some(SheetId::new("info-hook"));
    doc.add_data_sheet(ds1).unwrap();

    doc.add_result_sheet(
        ResultSheet::new(
            AnalysisSpec::new(
                "res-scale",
                "ds-comp",
                AnalysisKind::Transform(TransformAnalysisSpec::Math {
                    x_transform: None,
                    y_transform: Some(MathTransformPreset::LinearScale { slope: 1.0, intercept: 0.0 }),
                    error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
                    pharmacology_preset: None,
                    interchange_xy: false,
                    zero_x_substitute: None,
                    asymmetric_ci_policy: Default::default(),
                }),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let fit1_spec = AnalysisSpec::new(
        "res-ki",
        "res-scale",
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            weighting: WeightingScheme::None,
            parameter_rules: vec![
                hook_rule("HotConc_nM", HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("HotConc_nM") }),
                hook_rule("HotKd_nM", HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("HotKd_nM") }),
            ],
            ..NonlinearFitSpec::new("one_site_fit_ki")
        }),
    )
    .unwrap();
    doc.add_result_sheet(ResultSheet::new(fit1_spec).unwrap()).unwrap();

    let x_enz = [10.0, 20.0, 50.0, 120.0, 240.0];
    let y_enz: Vec<f64> = x_enz.iter().map(|&x| 10.0 * x / (120.0 + x)).collect();
    doc.add_data_sheet(xy_sheet("ds-enz", "Enzyme", "X", &x_enz, &[("10", &y_enz)])).unwrap();

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
                hook_rule("A", HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("ds-enz"), data_set_index: 0 }),
                hook_rule(
                    "B",
                    HookSource::ResultScalar { result_sheet_id: SheetId::new("res-ki"), parameter_key: String::from("Top"), data_set_index: Some(0) },
                ),
            ],
            ..NonlinearFitSpec::new("custom_enz")
        }),
    )
    .unwrap();
    doc.add_result_sheet(ResultSheet::new(fit2_spec).unwrap()).unwrap();

    recalculate_project(&mut doc).unwrap();
    let res_ki_1 = doc.result_sheet(&SheetId::new("res-ki")).unwrap();
    assert_eq!(res_ki_1.status, ResultStatus::UpToDate);
    approx_eq(res_ki_1.lookup_scalar("LogKi", Some(0)).unwrap(), log_ki_true, 1e-4, "LogKi 1");
    approx_eq(res_ki_1.lookup_scalar("Top", Some(0)).unwrap(), 120.0, 1e-4, "Top 1");
    approx_eq(doc.result_sheet(&SheetId::new("res-chained")).unwrap().lookup_scalar("K", Some(0)).unwrap(), 1.0, 1e-4, "K1");

    doc.info_sheets[0].set_constant("HotConc_nM", ConstantValue::Number(9.0), None).unwrap();
    if let TableArchetype::Xy(ref mut tbl) = doc.data_sheets[1].archetype {
        tbl.data_sets[0].title = String::from("20");
    }
    recalculate_project(&mut doc).unwrap();
    approx_eq(
        doc.result_sheet(&SheetId::new("res-ki")).unwrap().lookup_scalar("LogKi", Some(0)).unwrap(),
        expected_log_ic50 - (1.0 + 9.0 / hot_kd).log10(),
        1e-4,
        "LogKi 2",
    );
    approx_eq(doc.result_sheet(&SheetId::new("res-chained")).unwrap().lookup_scalar("K", Some(0)).unwrap(), 0.5, 1e-4, "K2");
}

#[test]
fn reactive_linked_region_and_transpose_propagation_without_duplication() {
    let mut doc = ProjectDocument::new("Linked Region Reactivity").unwrap();
    let mut info = InfoSheet::new_preseeded("info-link", "Notes").unwrap();
    info.notes_markdown = String::from("Initial notes.");
    doc.add_info_sheet(info).unwrap();

    let src_ds = col_sheet("ds-src", "Source", &[("S1", &[10.0, 12.0]), ("S2", &[20.0, 22.0]), ("S3", &[30.0, 32.0])]);
    let mut dst_ds = DataSheet::new(
        "ds-linked",
        "Transposed Link",
        TableArchetype::Column(curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap()),
    )
    .unwrap();
    apply_import_link_pipeline(
        &mut dst_ds,
        &internal_link("ds-src", InternalLinkKind::DataSheet, 1, 2, PlacementMode::Overwrite, true),
        ImportSourcePayload::InternalData(&src_ds),
        &ImportPipelineOptions { link_id: Some(String::from("lnk-tr")), ..Default::default() },
        Some(&mut doc.info_sheets[0]),
    )
    .unwrap();
    let notes_after_initial = doc.info_sheets[0].notes_markdown.clone();

    doc.add_data_sheet(src_ds).unwrap();
    doc.add_data_sheet(dst_ds).unwrap();
    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("res-desc", "ds-linked", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap()).unwrap(),
    )
    .unwrap();
    doc.add_result_sheet(
        ResultSheet::new(
            AnalysisSpec::new(
                "res-anova",
                "ds-linked",
                AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..Default::default() }),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    recalculate_project(&mut doc).unwrap();
    approx_eq(doc.result_sheet(&SheetId::new("res-desc")).unwrap().lookup_scalar("Mean", Some(0)).unwrap(), 20.0, 1e-12, "mean v1");

    if let TableArchetype::Column(ref mut tbl) = doc.data_sheets[0].archetype {
        tbl.data_sets[0].cells[0][0] = Cell::number(20.0);
        tbl.data_sets[1].cells[0][0] = Cell::number(40.0);
        tbl.data_sets[2].cells[0][0] = Cell::excluded_number(60.0);
    }
    recalculate_project(&mut doc).unwrap();
    recalculate_project(&mut doc).unwrap();

    let linked_ds = doc.data_sheet(&SheetId::new("ds-linked")).unwrap();
    assert_eq!(linked_ds.linked_regions.len(), 1);
    assert_eq!(doc.info_sheets[0].notes_markdown, notes_after_initial);
    let TableArchetype::Column(ref tbl) = linked_ds.archetype else { panic!("expected Column") };
    approx_eq(tbl.data_sets[0].cells[0][0].active_number().unwrap(), 20.0, 1e-12, "tr r0");
    approx_eq(tbl.data_sets[0].cells[1][0].active_number().unwrap(), 40.0, 1e-12, "tr r1");
    assert!(tbl.data_sets[0].cells[2][0].excluded);
    approx_eq(doc.result_sheet(&SheetId::new("res-desc")).unwrap().lookup_scalar("Mean", Some(0)).unwrap(), 30.0, 1e-12, "mean v2");

    // ResultSheet -> LinkedDataRegion (AnalyzableResultSheet) + AppendRows in-place refresh.
    doc.add_result_sheet(
        ResultSheet::new(
            AnalysisSpec::new(
                "res-src-tr",
                "ds-src",
                AnalysisKind::Transform(TransformAnalysisSpec::Math {
                    x_transform: None,
                    y_transform: Some(MathTransformPreset::LinearScale { slope: 10.0, intercept: 0.0 }),
                    error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
                    pharmacology_preset: None,
                    interchange_xy: false,
                    zero_x_substitute: None,
                    asymmetric_ci_policy: Default::default(),
                }),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    recalculate_project(&mut doc).unwrap();

    let mut append_ds = DataSheet::new(
        "ds-append",
        "Appended From Result",
        TableArchetype::Column(curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 3, 5).unwrap()),
    )
    .unwrap();
    append_ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(999.0)).unwrap();
    let analyzable = doc.result_sheet(&SheetId::new("res-src-tr")).unwrap().analyzable_data.as_ref().unwrap();
    apply_import_link_pipeline(
        &mut append_ds,
        &internal_link("res-src-tr", InternalLinkKind::AnalyzableResultSheet, 1, 2, PlacementMode::AppendRows, false),
        ImportSourcePayload::InternalData(analyzable),
        &ImportPipelineOptions { link_id: Some(String::from("lnk-append")), ..Default::default() },
        None,
    )
    .unwrap();
    let initial_range = append_ds.linked_regions[0].target_range;
    doc.add_data_sheet(append_ds).unwrap();

    if let TableArchetype::Column(ref mut tbl) = doc.data_sheets[0].archetype {
        tbl.data_sets[0].cells[0][0] = Cell::number(50.0);
    }
    recalculate_project(&mut doc).unwrap();
    recalculate_project(&mut doc).unwrap();

    let append_after = doc.data_sheet(&SheetId::new("ds-append")).unwrap();
    assert_eq!(append_after.linked_regions.len(), 1);
    assert_eq!(append_after.linked_regions[0].target_range, initial_range);
    let TableArchetype::Column(ref tbl) = append_after.archetype else { panic!("expected Column") };
    approx_eq(tbl.data_sets[0].cells[0][0].active_number().unwrap(), 999.0, 1e-12, "row 0 seed");
    approx_eq(tbl.data_sets[0].cells[initial_range.start.row][0].active_number().unwrap(), 500.0, 1e-12, "appended refreshed");
}

#[test]
fn preserves_graph_styles_gdos_axis_breaks_brackets_and_annotations() {
    let mut doc = ProjectDocument::new("Graph Preservation").unwrap();
    doc.add_data_sheet(col_sheet("ds-col", "Cohort", &[("Vehicle", &[10.0, 11.0, 9.5, 10.5]), ("Treated", &[20.0, 21.0, 19.5, 20.5])])).unwrap();
    doc.add_result_sheet(
        ResultSheet::new(
            AnalysisSpec::new(
                "res-anova",
                "ds-col",
                AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                    method: OneWayMethod::OrdinaryAnova,
                    compute_effect_sizes: true,
                    multiplicity: MultiplicitySpec {
                        method: MultiplicityMethod::TukeyHsd,
                        threshold: 0.05,
                        report_confidence_intervals: true,
                        control_column: None,
                    },
                    ..Default::default()
                }),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-box", &doc.data_sheets[0]).unwrap();
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
                ..AxisSegment::default()
            },
            AxisSegment {
                min: Some(18.0),
                max: Some(60.0),
                scale: AxisScale::Linear,
                length_share: 0.6,
                major_tick_interval: Some(10.0),
                minor_ticks_per_interval: 2,
                ..AxisSegment::default()
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
    let (expected_spec, expected_notes, expected_tag) = (gsheet.spec.clone(), gsheet.floating_notes.clone(), gsheet.color_tag);
    doc.add_graph_sheet(gsheet).unwrap();

    recalculate_project(&mut doc).unwrap();
    let scene1 = compile_graph(&doc.graph_sheets[0], &doc).unwrap();
    if let TableArchetype::Column(ref mut tbl) = doc.data_sheets[0].archetype {
        for (r, &v) in [50.0, 51.0, 49.5, 50.5].iter().enumerate() {
            tbl.data_sets[1].cells[r][0] = Cell::number(v);
        }
    }
    assert_eq!(recalculate_project_with_report(&mut doc).unwrap().preserved_graph_sheets, vec![SheetId::new("graph-box")]);
    let g_after = &doc.graph_sheets[0];
    assert!(g_after.initialized && g_after.spec == expected_spec && g_after.floating_notes == expected_notes && g_after.color_tag == expected_tag);
    assert_ne!(scene1.primitives, compile_graph(g_after, &doc).unwrap().primitives);
}

#[test]
fn frozen_sheet_protection_skips_links_and_formulas_while_propagating_downstream() {
    let mut doc = ProjectDocument::new("Frozen Sheet Protection").unwrap();
    let mut info = InfoSheet::new_preseeded("info-frz", "Frozen Test").unwrap();
    info.set_constant("Factor", ConstantValue::Number(2.0), None).unwrap();
    doc.add_info_sheet(info).unwrap();

    let src_ds = col_sheet("ds-src", "Src", &[("A", &[10.0, 20.0, 30.0])]);
    let mut lnk_ds = col_sheet("ds-frozen-link", "Frozen Link", &[("A", &[0.0, 0.0, 0.0])]);
    apply_import_link_pipeline(
        &mut lnk_ds,
        &internal_link("ds-src", InternalLinkKind::DataSheet, 2, 0, PlacementMode::Overwrite, false),
        ImportSourcePayload::InternalData(&src_ds),
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    lnk_ds.frozen = true;

    let mut mv = MultipleVariablesTable::new(2, 3).unwrap();
    mv.variables[0].title = String::from("X");
    mv.variables[1].title = String::from("Scaled");
    mv.variables[1].set_calculated_formula("=X * [Factor]");
    for r in 0..3 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[1].cells[r] = Cell::number(((r + 1) * 2) as f64);
    }
    let mut mv_ds = DataSheet::new("ds-frozen-mv", "Frozen MV", TableArchetype::MultipleVariables(mv)).unwrap();
    mv_ds.info_sheet_id = Some(SheetId::new("info-frz"));
    mv_ds.frozen = true;

    for s in [src_ds, lnk_ds, mv_ds] {
        doc.add_data_sheet(s).unwrap();
    }
    doc.add_result_sheet(
        ResultSheet::new(
            AnalysisSpec::new("res-from-frozen-link", "ds-frozen-link", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut lin_spec =
        AnalysisSpec::new("res-from-frozen-mv", "ds-frozen-mv", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    lin_spec.selected_data_sets = vec![0, 1];
    doc.add_result_sheet(ResultSheet::new(lin_spec).unwrap()).unwrap();

    if let TableArchetype::Column(ref mut tbl) = doc.data_sheets[0].archetype {
        for (r, v) in [100.0, 200.0, 300.0].into_iter().enumerate() {
            tbl.data_sets[0].cells[r][0] = Cell::number(v);
        }
    }
    doc.info_sheets[0].set_constant("Factor", ConstantValue::Number(10.0), None).unwrap();

    let rep_frozen = recalculate_project_with_report(&mut doc).unwrap();
    for id in ["ds-frozen-link", "ds-frozen-mv"] {
        assert!(rep_frozen.skipped_frozen_sheets.contains(&SheetId::new(id)) && !rep_frozen.updated_data_sheets.contains(&SheetId::new(id)));
    }
    approx_eq(
        doc.data_sheet(&SheetId::new("ds-frozen-link")).unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap(),
        10.0,
        1e-12,
        "frozen link",
    );
    approx_eq(
        doc.data_sheet(&SheetId::new("ds-frozen-mv")).unwrap().cell(CellCoord::new(0, 1, 0)).unwrap().active_number().unwrap(),
        2.0,
        1e-12,
        "frozen mv",
    );
    approx_eq(doc.result_sheet(&SheetId::new("res-from-frozen-link")).unwrap().lookup_scalar("Mean", Some(0)).unwrap(), 20.0, 1e-12, "frozen mean");
    approx_eq(doc.result_sheet(&SheetId::new("res-from-frozen-mv")).unwrap().lookup_scalar("Slope", Some(0)).unwrap(), 2.0, 1e-9, "frozen slope");

    doc.data_sheets[1].frozen = false;
    doc.data_sheets[2].frozen = false;
    let rep_unfrozen = recalculate_project_with_report(&mut doc).unwrap();
    assert!(rep_unfrozen.skipped_frozen_sheets.is_empty());
    assert!(
        rep_unfrozen.updated_data_sheets.contains(&SheetId::new("ds-frozen-link"))
            && rep_unfrozen.updated_data_sheets.contains(&SheetId::new("ds-frozen-mv"))
    );
    approx_eq(
        doc.result_sheet(&SheetId::new("res-from-frozen-link")).unwrap().lookup_scalar("Mean", Some(0)).unwrap(),
        200.0,
        1e-12,
        "unfrozen mean",
    );
    approx_eq(doc.result_sheet(&SheetId::new("res-from-frozen-mv")).unwrap().lookup_scalar("Slope", Some(0)).unwrap(), 10.0, 1e-9, "unfrozen slope");
}

#[test]
fn cycle_detection_and_atomic_rollback_across_all_topologies() {
    fn base_doc() -> ProjectDocument {
        let mut doc = ProjectDocument::new("Cycle Rollback").unwrap();
        let mut mv = MultipleVariablesTable::new(2, 2).unwrap();
        mv.variables[0].title = String::from("X");
        mv.variables[1].title = String::from("Y");
        mv.variables[1].set_calculated_formula("=X * 3");
        mv.variables[0].cells[0] = Cell::number(5.0);
        mv.variables[0].cells[1] = Cell::number(7.0);
        doc.add_data_sheet(DataSheet::new("ds-valid", "Valid", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();
        doc
    }
    let make_region = |id: &str, src: &str, kind: InternalLinkKind| LinkedDataRegion {
        id: id.to_owned(),
        spec: internal_link(src, kind, 1, 0, PlacementMode::Overwrite, false),
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: format!("{src}->dst"),
    };

    // a) Direct 2-node DataSheet link cycle (A <-> B)
    let mut doc_a = base_doc();
    let (mut ds_a, mut ds_b) = (col_sheet("ds-a", "A", &[("A", &[0.0, 0.0])]), col_sheet("ds-b", "B", &[("A", &[0.0, 0.0])]));
    ds_a.linked_regions.push(make_region("l1", "ds-b", InternalLinkKind::DataSheet));
    ds_b.linked_regions.push(make_region("l2", "ds-a", InternalLinkKind::DataSheet));
    doc_a.add_data_sheet(ds_a).unwrap();
    doc_a.add_data_sheet(ds_b).unwrap();
    let snap = doc_a.clone();
    assert!(topological_order(&doc_a).is_err() && recalculate_project(&mut doc_a).is_err() && doc_a == snap);

    // b) ResultSheet <-> ResultSheet HookSource::ResultScalar mutual cycle
    let mut doc_b = base_doc();
    doc_b.add_data_sheet(make_dose_response_sheet("ds-xy", None, &[-8.0, -7.0, -6.0, -5.0], [0.0, 100.0, -6.5, 1.0])).unwrap();
    for (rid, target) in [("res-1", "res-2"), ("res-2", "res-1")] {
        let nl = NonlinearFitSpec {
            parameter_rules: vec![hook_rule(
                "Top",
                HookSource::ResultScalar { result_sheet_id: SheetId::new(target), parameter_key: String::from("Top"), data_set_index: Some(0) },
            )],
            ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
        };
        doc_b.add_result_sheet(ResultSheet::new(AnalysisSpec::new(rid, "ds-xy", AnalysisKind::NonlinearRegression(nl)).unwrap()).unwrap()).unwrap();
    }
    let snap = doc_b.clone();
    assert!(recalculate_project(&mut doc_b).is_err() && doc_b == snap);

    // c) Mixed DataSheet -> Transform ResultSheet -> LinkedDataRegion -> DataSheet cycle
    let mut doc_c = base_doc();
    let mut ds_m = col_sheet("ds-m", "M", &[("A", &[0.0, 0.0])]);
    ds_m.linked_regions.push(make_region("l-res", "res-m", InternalLinkKind::AnalyzableResultSheet));
    doc_c.add_data_sheet(ds_m).unwrap();
    doc_c
        .add_result_sheet(
            ResultSheet::new(
                AnalysisSpec::new("res-m", "ds-m", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false }))
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let snap = doc_c.clone();
    assert!(recalculate_project(&mut doc_c).is_err() && doc_c == snap);

    // d) Self-referential GraphSheet and LayoutSheet detection
    let mut doc_g = ProjectDocument::new("Self Graph").unwrap();
    let ds_g = col_sheet("ds-g", "DG", &[("A", &[0.0, 0.0])]);
    let mut gs = GraphSheet::new_uninitialized_for_data("graph-self", &ds_g).unwrap();
    gs.spec.primary_source_sheet_id = SheetId::new("graph-self");
    doc_g.add_data_sheet(ds_g).unwrap();
    doc_g.graph_sheets.push(gs);
    let snap = doc_g.clone();
    assert!(recalculate_project(&mut doc_g).is_err() && doc_g == snap);

    let mut doc_l = ProjectDocument::new("Self Layout").unwrap();
    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveTable {
        id: String::from("item-1"),
        source_sheet_id: SheetId::new("layout-self"),
        tab_id: None,
        bounds_pt: RectPt::new(0.0, 0.0, 100.0, 100.0),
        selected_rows: None,
        selected_cols: None,
        style: EmbeddedTableStyle::default(),
    });
    doc_l.layout_sheets.push(LayoutSheet::new("layout-self", "Self", lspec).unwrap());
    let snap = doc_l.clone();
    assert!(recalculate_project(&mut doc_l).is_err() && doc_l == snap);
}

#[test]
fn session_guarded_recalculate_and_panic_rollback() {
    let mut doc = ProjectDocument::new("Session Guard").unwrap();
    let mut mv = MultipleVariablesTable::new(2, 2).unwrap();
    mv.variables[0].title = String::from("A");
    mv.variables[1].title = String::from("B");
    mv.variables[1].set_calculated_formula("=A + 10");
    mv.variables[0].cells[0] = Cell::number(5.0);
    mv.variables[0].cells[1] = Cell::number(15.0);
    doc.add_data_sheet(DataSheet::new("ds-1", "S1", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);
    session.recalculate_all().unwrap();
    let TableArchetype::MultipleVariables(ref tbl) = session.document().data_sheets[0].archetype else { panic!("expected MV") };
    approx_eq(tbl.variables[1].cells[0].active_number().unwrap(), 15.0, 1e-12, "b0");
    approx_eq(tbl.variables[1].cells[1].active_number().unwrap(), 25.0, 1e-12, "b1");

    let before = session.document().clone();
    let mut scratch = before.clone();
    let res: Result<(), EngineError> = run_guarded("dag.synthetic_panic", || {
        scratch.data_sheets.clear();
        panic!("simulated downstream panic");
    });
    let err = res.expect_err("panic must yield Err");
    assert!(err.to_string().contains("dag.synthetic_panic") && err.to_string().contains("simulated downstream panic"));
    assert_eq!(session.document(), &before);
}

#[test]
fn transient_failed_analysis_preserves_result_sheet_floating_notes() {
    let mut doc = ProjectDocument::new("Floating Note Recovery").unwrap();
    doc.add_data_sheet(col_sheet("ds-col", "Cohort", &[("Control", &[10.0, 12.0]), ("Treated", &[20.0, 22.0])])).unwrap();
    doc.add_result_sheet(
        ResultSheet::new(
            AnalysisSpec::new(
                "res-t",
                "ds-col",
                AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    recalculate_project(&mut doc).unwrap();
    let expected_note = FloatingNote::new("res-note-1", RectPt::new(24.0, 32.0, 180.0, 64.0), "Verify baseline outlier exclusion");
    doc.result_sheets[0].floating_notes.push(expected_note.clone());
    assert!(doc.result_sheets[0].status == ResultStatus::UpToDate && !doc.result_sheets[0].tabs.is_empty());

    let mut session = Session::from_document(open_project_bytes(&save_project_bytes(&doc).unwrap()).unwrap());

    ok_cmd(&mut session, "table.excludeCells", json!({ "sheet_id": "ds-col", "cells": [{ "row": 1, "col": 0 }], "excluded": true }));
    let failed = rs(&session, "res-t");
    assert!(
        matches!(failed.status, ResultStatus::Failed { .. })
            && failed.tabs.is_empty()
            && failed.scalars.is_empty()
            && failed.floating_notes == vec![expected_note.clone()]
    );

    ok_cmd(&mut session, "table.excludeCells", json!({ "sheet_id": "ds-col", "cells": [{ "row": 1, "col": 0 }], "excluded": false }));
    let recovered = rs(&session, "res-t");
    assert!(
        recovered.status == ResultStatus::UpToDate
            && !recovered.tabs.is_empty()
            && !recovered.scalars.is_empty()
            && recovered.floating_notes == vec![expected_note]
    );
}

#[test]
fn aud_02_27_and_13_04_07_22_dag_cycles_failed_fits_source_resolution_and_cancellation() {
    // AUD-02-27: self-referential ResultSheet returns ErrorCode::CycleDetected.
    let mut doc_cyc = session_with("Cyc", vec![col_sheet("col-1", "G", &[("A", &[1.0, 2.0])])]).document().clone();
    doc_cyc
        .result_sheets
        .push(ResultSheet::new(AnalysisSpec::new("self-cyc", "self-cyc", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap());
    assert_eq!(topological_order(&doc_cyc).unwrap_err().code, ErrorCode::CycleDetected);

    // AUD-13-04: DAG preserves Failed(SolverFailed) from Ok nonlinear fit (empty scalars/curves) and fit.run surfaces SolverFailed.
    let mut s_fail =
        session_with("FailFit", vec![xy_sheet("xy-1", "D", "X", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[("A", &[100.0, 1.0, 99.0, 2.0, 98.0, 3.0])])]);
    let mut doc_f = s_fail.document().clone();
    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.max_iters = 1;
    doc_f.add_result_sheet(ResultSheet::new(AnalysisSpec::new("fit-fail", "xy-1", AnalysisKind::NonlinearRegression(nl)).unwrap()).unwrap()).unwrap();
    let rep_f = recalculate_project_with_report(&mut doc_f).unwrap();
    let rf = doc_f.result_sheet(&SheetId::new("fit-fail")).unwrap();
    assert!(matches!(rf.status, ResultStatus::Failed { code: ErrorCode::SolverFailed, .. }) && rf.scalars.is_empty() && rf.fitted_curves.is_empty());
    assert!(rep_f.failed_result_sheets.iter().any(|f| f.as_str() == "fit-fail"));
    assert_err_code(
        &mut s_fail,
        "fit.run",
        json!({ "sheet_id": "fit-fail-cmd", "source_sheet_id": "xy-1", "equation_id": "log_agonist_vs_response_4pl", "max_iters": 1 }),
        ErrorCode::SolverFailed,
    );

    // AUD-13-07: multi-hop graph source resolution prefers analyzable_data, keeps terminal tab_rows bounds, and rejects Failed upstream/direct sources with Conflict.
    let mut s_hop = session_with("Hop", vec![xy_sheet("xy-1", "D", "X", &[1.0, 2.0, 3.0, 4.0], &[("A", &[10.0, 40.0, 70.0, 100.0])])]);
    run_cmds(
        &mut s_hop,
        &[
            ("transform.run", json!({ "sheet_id": "norm-1", "source_sheet_id": "xy-1", "kind": "normalize" })),
            ("transform.run", json!({ "sheet_id": "prune-2", "source_sheet_id": "norm-1", "kind": "prune_rows", "every_k_rows": 2 })),
            ("stats.run", json!({ "sheet_id": "desc-3", "source_sheet_id": "prune-2", "kind": "descriptive" })),
            ("graph.create", json!({ "sheet_id": "g-prune", "data_sheet_id": "prune-2" })),
        ],
    );
    assert_eq!(rs(&s_hop, "prune-2").analyzable_data.as_ref().unwrap().archetype.row_count(), 2);
    assert_bad_input(
        &mut s_hop,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g-prune", "overrides": [{ "series_index": 0, "point_index": 2, "hidden": true }] }),
    );
    let g_out = ok_cmd(&mut s_hop, "graph.create", json!({ "sheet_id": "g-chain", "data_sheet_id": "desc-3", "kind": "ResidualPlot" }));
    assert_eq!((&g_out.payload["data_sheet_id"], &g_out.payload["title"]), (&json!("desc-3"), &json!("Descriptive statistics")));
    let desc_rows = rs(&s_hop, "desc-3").tabs.iter().map(|t| t.rows.len()).max().unwrap_or(0);
    assert!(desc_rows > 4);
    ok_cmd(
        &mut s_hop,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g-chain", "overrides": [{ "series_index": 0, "point_index": desc_rows - 1, "hidden": true }] }),
    );
    assert!(!generate_methods_report(s_hop.document(), &SheetId::new("desc-3"), ReportFormat::PlainText).unwrap().is_empty());

    let mut d_fail1 = s_hop.document().clone();
    d_fail1.result_sheet_mut(&SheetId::new("norm-1")).unwrap().status =
        ResultStatus::Failed { code: ErrorCode::SolverFailed, message: "boom".into() };
    assert_err_code(
        &mut Session::from_document(d_fail1),
        "graph.create",
        json!({ "sheet_id": "g-f1", "data_sheet_id": "norm-1" }),
        ErrorCode::Conflict,
    );

    let mut d_fail2 = s_hop.document().clone();
    d_fail2.result_sheet_mut(&SheetId::new("prune-2")).unwrap().status =
        ResultStatus::Failed { code: ErrorCode::SolverFailed, message: "upstream failed".into() };
    let mut s_f2 = Session::from_document(d_fail2);
    for (cmd_id, params) in [
        ("graph.create", json!({ "sheet_id": "g-f2", "data_sheet_id": "desc-3" })),
        ("graph.formatSelectedPoints", json!({ "sheet_id": "g-chain", "overrides": [{ "series_index": 0, "point_index": 0, "hidden": true }] })),
        ("graph.formatSelectedPoints", json!({ "sheet_id": "g-prune", "overrides": [{ "series_index": 0, "point_index": 0, "hidden": true }] })),
    ] {
        assert_err_code(&mut s_f2, cmd_id, params, ErrorCode::Conflict);
    }

    // AUD-13-22: cooperative cancellation stops DAG, project.recalculate, and analyze.run leaving ResultStatus::Dirty.
    let mut doc_c = session_with("Cancel", vec![col_sheet("col-1", "G", &[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])])]).document().clone();
    doc_c
        .add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-c", "col-1", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap())
        .unwrap();
    let cancelled = AtomicBool::new(true);
    assert_eq!(recalculate_project_with_report_cancellable(&mut doc_c, Some(&cancelled)).unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(doc_c.result_sheet(&SheetId::new("res-c")).unwrap().status, ResultStatus::Dirty);
    let mut s_c = Session::from_document(doc_c);
    assert_eq!(s_c.execute_with_caller("project.recalculate", json!({}), Caller::Cli, &cancelled).unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(s_c.execute_with_caller("analyze.run", json!({}), Caller::Cli, &cancelled).unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(rs(&s_c, "res-c").status, ResultStatus::Dirty);
}
