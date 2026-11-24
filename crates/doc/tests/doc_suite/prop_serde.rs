#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AdditionalTick, AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisSpec, BlandAltmanSpec, Cell, CellCoord, CellValue, ChecklistItem,
    ChecklistStatus, CiBound, ClusterDistance, ClusterLinkage, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConfidenceLevel, ConstantValue,
    ContingencyMethod, ContingencySpec, ContingencyTable, CurveSamplePoint, DataSheet, DescriptiveSpec, DiagnosticPlotKind, DocError,
    EquationDisplayMode, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FactorialAnovaSpec, FitMethod, FittedCurveOverlay, FloatingNote, GlmFamily,
    GlmSpec, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject, GroupedArrangement, GroupedTable, HookSource, ImportFilterSpec,
    ImportLinkSpec, ImportPlacementSpec, InfoSheet, JournalWidthPreset, KappaWeighting, LayoutItem, LayoutSheet, LayoutSpec, LegendSpec,
    LevelOrderKind, LevelOrderSpec, LinearRegressionSpec, LinkSource, LinkedDataRegion, LogitDomain, MathTransformPreset, ModelComparisonSpec,
    MultipleRowTestsSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, MultivariateGraphEncoding, MultivariatePlotMode,
    MultivariateSpec, NestedAnalysisSpec, NestedTable, NonlinearFitSpec, NormalizeAnchor, NoteId, OneWayAnovaSpec, OneWayMethod, OutlierSpec,
    PValueFormat, PairwiseBracketSpec, PairwiseComparisonRecord, PanelBadgeStyle, ParameterRule, ParameterScope, PartsOfWholeLayout,
    PartsOfWholeTable, PassingBablokSpec, PhysicalUnit, PlacementMode, ProjectDocument, RectPt, RegressionOverlaySpec, RepeatedMeasuresDesign,
    ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, RocCiMethod, RocSpec, SequenceProgression, SheetCellRange,
    SheetFolderKind, SheetId, SimulationNoise, SizeChannelMapping, SizePt, SubcolumnMode, SumOfSquaresType, SummaryMeasure, SurvivalAnalysisSpec,
    SurvivalTable, TableArchetype, TableKind, TickLabelFormat, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole, WeightingScheme,
    WhiskerMethod, XColumnMode, XyTable,
};
use curvebook_palette::{
    CategoricalPalette, Cmyk, ColorRamp, ColorStop, ColormapId, CvdKind, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8,
};
use proptest::prelude::*;

fn assert_json_roundtrip<T>(value: &T)
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de> + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value);
}

fn arb_finite_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        (-1e12_f64..1e12_f64),
        Just(0.0_f64),
        Just(-0.0_f64),
        Just(1e-300_f64),
        Just(-1e-300_f64),
        Just(f64::MIN_POSITIVE),
        Just(-f64::MIN_POSITIVE),
        Just(-1.234_567_890_123_456_7e12_f64),
        Just(core::f64::consts::PI),
    ]
}

fn arb_cell() -> impl Strategy<Value = Cell> {
    let val_strat =
        prop_oneof![Just(CellValue::Empty), arb_finite_f64().prop_map(CellValue::Number), "[a-zA-Z0-9 _.-]{0,16}".prop_map(CellValue::Text),];
    (val_strat, any::<bool>()).prop_map(|(value, excluded)| Cell { value, excluded })
}

fn arb_subcolumn_mode() -> impl Strategy<Value = SubcolumnMode> {
    prop_oneof![
        Just(SubcolumnMode::Single),
        (1usize..=8usize).prop_map(SubcolumnMode::RawReplicates),
        Just(SubcolumnMode::MeanSdN),
        Just(SubcolumnMode::MeanSemN),
        Just(SubcolumnMode::MeanCvN),
        Just(SubcolumnMode::MeanLowHigh),
        Just(SubcolumnMode::UpperLowerLimits),
        Just(SubcolumnMode::MedianIqr),
    ]
}

fn arb_rgba8() -> impl Strategy<Value = Rgba8> {
    (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(r, g, b, a)| Rgba8::rgba(r, g, b, a))
}

fn arb_axis_scale() -> impl Strategy<Value = AxisScale> {
    prop_oneof![
        Just(AxisScale::Linear),
        Just(AxisScale::Log10),
        Just(AxisScale::Log2),
        Just(AxisScale::Ln),
        Just(AxisScale::Logit(LogitDomain::Percent)),
        Just(AxisScale::Logit(LogitDomain::Proportion)),
        Just(AxisScale::Probit),
        Just(AxisScale::Reciprocal),
    ]
}

fn arb_pvalue_format() -> impl Strategy<Value = PValueFormat> {
    prop_oneof![
        Just(PValueFormat::FourDecimal),
        Just(PValueFormat::Apa),
        Just(PValueFormat::Nejm),
        (1u8..=15u8, prop::option::of(1e-12_f64..1.0_f64))
            .prop_map(|(decimals, scientific_below)| PValueFormat::new_custom(decimals, scientific_below)),
    ]
}

fn arb_simulation_noise() -> impl Strategy<Value = SimulationNoise> {
    prop_oneof![
        Just(SimulationNoise::None),
        (0.0_f64..100.0_f64).prop_map(|sd| SimulationNoise::Gaussian { sd }),
        (0.0_f64..50.0_f64).prop_map(|cv_percent| SimulationNoise::RelativePercent { cv_percent }),
        Just(SimulationNoise::Poisson),
    ]
}

fn arb_simulate_spec() -> impl Strategy<Value = TransformAnalysisSpec> {
    (
        any::<u64>(),
        1usize..=200usize,
        1usize..=16usize,
        -1000.0_f64..0.0_f64,
        0.01_f64..1000.0_f64,
        "[a-z0-9_]{3,20}",
        prop::collection::vec(("[a-zA-Z0-9_]{1,12}", arb_finite_f64()), 0..=6),
        arb_simulation_noise(),
    )
        .prop_map(|(seed, row_count, replicates, x_min, x_max, equation_id, parameters, noise)| TransformAnalysisSpec::Simulate {
            seed,
            row_count,
            replicates,
            x_min,
            x_max,
            equation_id,
            parameters,
            noise,
        })
}

#[test]
fn extreme_finite_floats_round_trip_bit_exact() {
    let extremes = [1e-300_f64, -1e-300_f64, f64::MIN_POSITIVE, -f64::MIN_POSITIVE, -1.234_567_890_123_456_7e12_f64, 0.0_f64];
    for &x in &extremes {
        let cell = Cell::number(x);
        let json = serde_json::to_string(&cell).unwrap();
        let back: Cell = serde_json::from_str(&json).unwrap();
        let CellValue::Number(y) = back.value else {
            panic!("expected Number cell");
        };
        assert_eq!(x.to_bits(), y.to_bits(), "bit mismatch for {x:e}: json={json}");
    }
}

#[test]
fn invalid_confidence_levels_rejected_on_json_deserialize() {
    for raw in ["0.0", "-5.0", "100.0", "100.001", "0.0099", "null", "\"95.0\""] {
        let parsed: Result<ConfidenceLevel, _> = serde_json::from_str(raw);
        assert!(parsed.is_err(), "expected {raw} to be rejected, got {parsed:?}");
    }
    for valid in ["0.01", "50.0", "95.0", "99.99"] {
        let level: ConfidenceLevel = serde_json::from_str(valid).unwrap();
        assert_json_roundtrip(&level);
    }
}

#[test]
fn full_project_document_covers_all_eight_archetypes_specs_results_graphs_layouts_and_palette() {
    let mut proj = ProjectDocument::new("Comprehensive Schema Project").unwrap();

    // InfoSheet
    let mut info = InfoSheet::new_preseeded("info-1", "Lab Notes").unwrap();
    info.set_constant("HotNM", ConstantValue::Number(2.5), Some(String::from("nM"))).unwrap();
    info.set_constant("Operator", ConstantValue::Text(String::from("Dr. Ada")), None).unwrap();
    info.set_constant("RunDate", ConstantValue::Date(String::from("2026-11-13")), None).unwrap();
    info.notes_markdown = String::from("# Assay\nAll controls verified.");
    info.color_tag = Some(Rgba8::rgb(0x00, 0x72, 0xB2));
    info.floating_notes.push(FloatingNote::new("note-info", RectPt::new(10.0, 20.0, 150.0, 80.0), "Pinned note"));
    proj.add_info_sheet(info).unwrap();

    // 1. XY
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::RawReplicates(3), 2, 3).unwrap();
    xy.x_cells[0] = vec![Cell::number(1e-9), Cell::number(1e-10), Cell::number(3.0)];
    xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::excluded_number(99.9), Cell::number(12.0)];
    let mut xy_sheet = DataSheet::new("ds-xy", "Dose Response", TableArchetype::Xy(xy)).unwrap();
    xy_sheet.info_sheet_id = Some(SheetId::new("info-1"));
    xy_sheet
        .add_linked_region(LinkedDataRegion {
            id: String::from("link-ext"),
            spec: ImportLinkSpec {
                source: LinkSource::ExternalDelimitedFile { path: String::from("data/assay.csv"), delimiter: ',', decimal_separator: '.' },
                filter: ImportFilterSpec {
                    row_window: Some((0, 10)),
                    col_window: Some((0, 3)),
                    decimate_every_k: Some(2),
                    unstack_group_column: None,
                    parse_asterisk_exclusions: true,
                    baseline: None,
                    exclude_row_predicate: None,
                },
                placement: ImportPlacementSpec {
                    mode: PlacementMode::Overwrite,
                    anchor: CellCoord::new(0, 0, 0),
                    propagate_title_header: true,
                    skip_contiguous_blanks: false,
                    transpose: false,
                },
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 2)),
            provenance_summary: String::from("CSV import"),
        })
        .unwrap();
    proj.add_data_sheet(xy_sheet.clone()).unwrap();

    // 2. Column
    let mut col_tbl = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanCvN, 2, 2).unwrap();
    col_tbl.data_sets[0].cells[0] = vec![Cell::number(50.0), Cell::number(5.0), Cell::number(6.0)];
    proj.add_data_sheet(DataSheet::new("ds-col", "Column", TableArchetype::Column(col_tbl)).unwrap()).unwrap();

    // 3. Grouped
    let grp_tbl = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 2, 2).unwrap();
    proj.add_data_sheet(DataSheet::new("ds-grp", "Grouped", TableArchetype::Grouped(grp_tbl)).unwrap()).unwrap();

    // 4. Contingency
    let mut cont_tbl = ContingencyTable::new(2, 2).unwrap();
    cont_tbl.data_sets[0].cells[0][0] = Cell::number(18.0);
    proj.add_data_sheet(DataSheet::new("ds-cont", "Contingency", TableArchetype::Contingency(cont_tbl)).unwrap()).unwrap();

    // 5. Survival
    let mut surv_tbl = SurvivalTable::new(2, 2).unwrap();
    surv_tbl.times = vec![Cell::number(4.5), Cell::number(12.0)];
    proj.add_data_sheet(DataSheet::new("ds-surv", "Survival", TableArchetype::Survival(surv_tbl)).unwrap()).unwrap();

    // 6. Parts of Whole
    let pow_tbl = PartsOfWholeTable::new(1, 3).unwrap();
    proj.add_data_sheet(DataSheet::new("ds-pow", "Parts", TableArchetype::PartsOfWhole(pow_tbl)).unwrap()).unwrap();

    // 7. Multiple Variables
    let mut mv_tbl = MultipleVariablesTable::new(2, 2).unwrap();
    mv_tbl.variables[0].lock_role(VariableRole::Categorical);
    mv_tbl.variables[0].level_order = LevelOrderSpec { order: LevelOrderKind::Explicit(vec![String::from("Control")]), reversed: true };
    mv_tbl.variables[1].set_calculated_formula("= [A] * 2");
    proj.add_data_sheet(DataSheet::new("ds-mv", "Multivariate", TableArchetype::MultipleVariables(mv_tbl)).unwrap()).unwrap();

    // 8. Nested
    let nest_tbl = NestedTable::new(2, 3, 4).unwrap();
    proj.add_data_sheet(DataSheet::new("ds-nest", "Nested", TableArchetype::Nested(nest_tbl)).unwrap()).unwrap();

    // Analyses & ResultSheet
    let mut sim_spec = AnalysisSpec::new(
        "res-sim",
        "ds-xy",
        AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
            seed: 9001,
            row_count: 12,
            replicates: 4,
            x_min: -10.0,
            x_max: -4.0,
            equation_id: String::from("log_agonist_vs_response_variable_slope"),
            parameters: vec![
                (String::from("Bottom"), 0.0),
                (String::from("Top"), 100.0),
                (String::from("LogEC50"), -7.0),
                (String::from("HillSlope"), 1.0),
            ],
            noise: SimulationNoise::Gaussian { sd: 2.5 },
        }),
    )
    .unwrap();
    sim_spec.confidence_level = ConfidenceLevel::new(99.5).unwrap();
    sim_spec.p_value_format = PValueFormat::new_custom(3, Some(1e-4));

    let mut res_sheet = ResultSheet::new(sim_spec).unwrap();
    res_sheet.status = ResultStatus::Failed { code: curvebook_doc::ErrorCode::SolverFailed, message: String::from("Synthetic diagnostic") };
    res_sheet.tabs.push(ResultTab {
        id: String::from("tab-1"),
        title: String::from("Summary"),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: vec![String::from("Parameter"), String::from("Estimate")],
        row_labels: vec![String::from("LogEC50")],
        rows: vec![vec![
            ResultCell::ConfidenceInterval { lower: CiBound::Finite(-7.2), upper: CiBound::Open },
            ResultCell::PValue { raw: 2.41e-9, formatted: String::from("2.41e-9"), stars: String::from("****") },
        ]],
        footnotes: vec![String::from("Profile likelihood CI")],
    });
    res_sheet.scalars.push(ResultScalarEntry { key: String::from("EC50"), data_set_index: Some(0), value: 1e-7, units: Some(String::from("M")) });
    res_sheet.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 0,
        group_b: 1,
        label_a: String::from("Ctrl"),
        label_b: String::from("Drug"),
        estimate_diff: Some(-12.4),
        raw_p_or_q: 0.0012,
        is_fdr_q: true,
        significant: true,
    });
    res_sheet.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("four_pl"),
        converged: true,
        ambiguous: false,
        points: vec![CurveSamplePoint {
            x: -7.0,
            y: 50.0,
            ci_lower: Some(45.0),
            ci_upper: Some(55.0),
            pred_lower: Some(40.0),
            pred_upper: Some(60.0),
        }],
    });
    res_sheet.checklist.push(ChecklistItem {
        assumption: String::from("Normality of residuals"),
        status: ChecklistStatus::Pass,
        diagnostic_metric: Some(String::from("W = 0.98")),
        explanation: String::from("No evidence of non-normality."),
        remediation: None,
    });
    proj.add_result_sheet(res_sheet).unwrap();

    // GraphSheet covering palette, axis scales, GDOs, multivariate, brackets, annotations
    let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &xy_sheet).unwrap();
    gsheet.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Volcano);
    gsheet.spec.palette = PaletteId::PaulTolVibrant;
    gsheet.spec.x_axis = AxisSpec {
        segments: vec![AxisSegment {
            min: Some(0.01),
            max: Some(99.99),
            scale: AxisScale::Logit(LogitDomain::Percent),
            length_share: 1.0,
            major_tick_interval: Some(10.0),
            minor_ticks_per_interval: 2,
        }],
        tick_format: TickLabelFormat::PowerOfTenSuperscript,
        ..AxisSpec::linear("Logit %")
    };
    gsheet.spec.grouped_arrangement = GroupedArrangement::Separated;
    gsheet.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
        marker: Some(MarkerStyle {
            shape: MarkerShape::Star,
            fill_mode: MarkerFillMode::SemiFilledTop,
            size_pt: 5.5,
            fill_color: Rgba8::rgb(10, 20, 30),
            stroke_color: Rgba8::BLACK,
            stroke_width_pt: 1.0,
        }),
        line: None,
        area_fill: None,
        replicate_lines: curvebook_doc::ReplicateLineMode::default(),
    };
    gsheet.spec.series[0].y_error_bars = Some(ErrorBarSpec {
        metric: ErrorBarMetric::ConfidenceInterval95,
        direction: ErrorBarDirection::UpperOnly,
        dash_style: LineDashStyle::DashDotDot,
        stroke_width_pt: 1.25,
        cap_width_pt: 4.5,
        color: Some(Rgba8::BLACK),
    });
    gsheet.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-sim"),
        sample_segments: 512,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    gsheet.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::BoxAndWhisker {
                whiskers: WhiskerMethod::Tukey1_5Iqr,
                fill_color: Rgba8::WHITE,
                stroke_color: Rgba8::BLACK,
                show_outliers: true,
                show_all_points: false,
            },
            y_error_bars: None,
        },
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Continuous {
            column: 1,
            ramp: ColorRamp::Custom {
                stops: vec![ColorStop { position: 0.0, color: Rgba8::BLACK }, ColorStop { position: 1.0, color: Rgba8::WHITE }],
            },
            domain_clamp: Some((0.0, 1.0)),
        },
        size: SizeChannelMapping::Variable { column: 1, min_radius_pt: 2.0, max_radius_pt: 8.0 },
        label_column: Some(0),
        x_level_order_override: None,
        show_convex_hulls: true,
        show_confidence_ellipses: false,
    });
    gsheet.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res-sim"),
        group_a: 0,
        group_b: 1,
        style: curvebook_doc::BracketStyle::CurlyBrace,
        label_mode: curvebook_doc::BracketLabelMode::SummaryStars,
        hide_if_non_significant: true,
        stroke_width_pt: 1.0,
        font_size_pt: 9.0,
        bracket_step_pt: 12.0,
    });
    gsheet.spec.pairwise_display_mode = curvebook_doc::PairwiseDisplayMode::BracketsAndCompactLetterDisplay;
    gsheet.spec.compact_letter_display = Some(curvebook_doc::CompactLetterDisplaySpec {
        result_sheet_id: SheetId::new("res-sim"),
        alpha_override: Some(0.05),
        ordering: curvebook_doc::CldOrdering::AscendingMean,
        uppercase: true,
        font_size_pt: 10.0,
        offset_pt: 7.0,
        color: Rgba8::BLACK,
    });
    gsheet.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 90.0, 30.0),
        markdown: String::from("EC50 = {{result:res-sim:EC50}}"),
        bordered: true,
    });
    gsheet.spec.legend = LegendSpec { visible: true, position_pt: Some((220.0, 30.0)), bordered: true };
    proj.add_graph_sheet(gsheet).unwrap();

    // LayoutSheet
    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    lspec.unit = PhysicalUnit::Millimeters;
    lspec.panel_badges = PanelBadgeStyle::LowerCaseLetters;
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("g-1"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: RectPt::new(10.0, 10.0, 200.0, 160.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    lspec.items.push(LayoutItem::LiveTable {
        id: String::from("t-1"),
        source_sheet_id: SheetId::new("res-sim"),
        tab_id: Some(String::from("tab-1")),
        bounds_pt: RectPt::new(220.0, 10.0, 180.0, 120.0),
        selected_rows: None,
        selected_cols: None,
        style: curvebook_doc::EmbeddedTableStyle::default(),
    });
    lspec.items.push(LayoutItem::TextCallout {
        id: String::from("c-1"),
        markdown: String::from("{{info:HotNM}}"),
        bounds_pt: RectPt::new(10.0, 180.0, 200.0, 40.0),
        font_size_pt: 10.0,
    });
    proj.add_layout_sheet(LayoutSheet::new("layout-1", "Main Figure", lspec).unwrap()).unwrap();

    assert_json_roundtrip(&proj);

    // Palette standalone types
    assert_json_roundtrip(&Rgba8::BLACK.to_linear());
    assert_json_roundtrip(&Rgba8::BLACK.to_oklab());
    assert_json_roundtrip(&Rgba8::BLACK.to_oklch());
    assert_json_roundtrip(&Cmyk::from_srgb(Rgba8::rgb(12, 34, 56)));
    assert_json_roundtrip(&CategoricalPalette::custom("MyPalette", vec![Rgba8::WHITE]));
    assert_json_roundtrip(&ColormapId::Cividis);
    assert_json_roundtrip(&CvdKind::Tritanopia);
    assert_json_roundtrip(&SummaryMeasure::GeometricMean);
    assert_json_roundtrip(&PartsOfWholeLayout::Waffle10x10);
    assert_json_roundtrip(&SizePt::default());
    assert_json_roundtrip(&NoteId::new("note-1"));

    // Exercise additional AnalysisKind variants to ensure every nested enum serializes cleanly.
    let extra_analyses = [
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-9 }),
            y_transform: Some(MathTransformPreset::LinearScale { slope: 2.0, intercept: -1.0 }),
            error_propagation: curvebook_doc::ErrorPropagationMode::FirstOrderDeltaMethod,
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Max") }),
            output_percent: true,
            per_data_set: true,
        }),
        AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 1.0 }, ..TwoGroupSpec::default() }),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Dunnett { control_column: 0 },
                threshold: 0.05,
                report_confidence_intervals: true,
            },
            ..OneWayAnovaSpec::default()
        }),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            ss_type: SumOfSquaresType::TypeIII,
            repeated_measures: RepeatedMeasuresDesign::RowFactorMatched,
            residual_covariance: curvebook_doc::ResidualCovariance::Unstructured,
            ..FactorialAnovaSpec::default()
        }),
        AnalysisKind::NestedComparison(NestedAnalysisSpec {
            two_group_t_test: true,
            multiplicity: MultiplicitySpec::default(),
            ..NestedAnalysisSpec::default()
        }),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::FisherExact, ..ContingencySpec::default() }),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: Some(0),
            mv_status_column: Some(1),
            mv_group_column: Some(2),
        }),
        AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong }),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: vec![(1, String::from("Baseline"))],
            compute_vif: true,
        }),
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(123), pcr_outcome_column: None }),
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 5,
            max_iters: 20,
            seed: 42,
            response_column: Some(0),
            predictor_columns: vec![1, 2, 3],
        }),
        AnalysisKind::BlandAltman(BlandAltmanSpec::default()),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired: true,
            nonparametric: false,
            assume_equal_sd: false,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::NewmanKeuls, threshold: 0.05, report_confidence_intervals: true },
        }),
        AnalysisKind::PassingBablok(PassingBablokSpec::default()),
        AnalysisKind::LinearRegression(LinearRegressionSpec::default()),
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
            weighting: WeightingScheme::GeneralPowerY(2.0),
            parameter_rules: vec![ParameterRule {
                name: String::from("Top"),
                scope: ParameterScope::Constant(100.0),
                initial_override: Some(90.0),
                lower_bound: Some(0.0),
                upper_bound: Some(200.0),
            }],
            comparison: Some(ModelComparisonSpec::Aicc { alternative_equation_id: String::from("three_pl") }),
            ..NonlinearFitSpec::new("four_pl")
        }),
    ];
    for kind in extra_analyses {
        assert_json_roundtrip(&kind);
    }
}

#[test]
fn new_analysis_specs_compatibility_auto_titles_and_copy_traits() {
    fn assert_copy_spec<T: Copy + Clone + PartialEq + Default + serde::Serialize + for<'de> serde::Deserialize<'de>>() {}
    assert_copy_spec::<TwoGroupSpec>();
    assert_copy_spec::<BlandAltmanSpec>();
    assert_copy_spec::<MultipleRowTestsSpec>();
    assert_copy_spec::<PassingBablokSpec>();

    assert_eq!(BlandAltmanSpec::default(), BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: None });
    assert_eq!(PassingBablokSpec::default(), PassingBablokSpec { x_column: 0, y_column: 1 });
    assert!(MultipleRowTestsSpec::default().assume_equal_sd);
    assert!(NonlinearFitSpec::new("four_pl").emit_residuals_tab);

    // MultiplicityMethod::NewmanKeuls & FisherLsd, ContingencyMethod::BarnardExact
    assert_json_roundtrip(&MultiplicityMethod::NewmanKeuls);
    assert_json_roundtrip(&MultiplicityMethod::FisherLsd);
    assert_json_roundtrip(&ContingencyMethod::BarnardExact);
    assert_eq!(
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::BarnardExact, ..ContingencySpec::default() }).derive_auto_title(),
        "Barnard's exact test"
    );

    // BlandAltman compatibility & auto-title
    let ba = AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 2, percent_difference: true, xy_compare_x_vs_y: Some(false) });
    assert_eq!(ba.derive_auto_title(), "Bland-Altman method comparison");
    assert!(ba.is_compatible_with(TableKind::Column));
    assert!(ba.is_compatible_with(TableKind::Xy));
    assert!(ba.is_compatible_with(TableKind::MultipleVariables));
    assert!(!ba.is_compatible_with(TableKind::Grouped));
    assert!(!ba.is_compatible_with(TableKind::Contingency));
    assert!(ba.upstream_sheet_ids().is_empty());
    assert_json_roundtrip(&ba);

    // MultipleRowTTests compatibility & auto-titles
    for (paired, nonparametric, expected_title) in [
        (false, false, "Multiple unpaired t tests"),
        (true, false, "Multiple paired t tests"),
        (false, true, "Multiple Mann-Whitney tests"),
        (true, true, "Multiple Wilcoxon tests"),
    ] {
        let mrow = AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired,
            nonparametric,
            assume_equal_sd: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::FisherLsd, threshold: 0.05, report_confidence_intervals: false },
        });
        assert_eq!(mrow.derive_auto_title(), expected_title);
        assert!(mrow.is_compatible_with(TableKind::Grouped));
        assert!(mrow.is_compatible_with(TableKind::Xy));
        assert!(mrow.is_compatible_with(TableKind::Column));
        assert!(mrow.is_compatible_with(TableKind::MultipleVariables));
        assert!(!mrow.is_compatible_with(TableKind::Survival));
        assert!(mrow.upstream_sheet_ids().is_empty());
        assert_json_roundtrip(&mrow);
    }

    // PassingBablok compatibility & auto-title
    let pb = AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 1, y_column: 3 });
    assert_eq!(pb.derive_auto_title(), "Passing-Bablok regression");
    assert!(pb.is_compatible_with(TableKind::Xy));
    assert!(pb.is_compatible_with(TableKind::Column));
    assert!(pb.is_compatible_with(TableKind::MultipleVariables));
    assert!(!pb.is_compatible_with(TableKind::Grouped));
    assert!(pb.upstream_sheet_ids().is_empty());
    assert_json_roundtrip(&pb);
}

#[test]
fn extended_nonlinear_parameters_tabs_and_axis_additional_ticks() {
    // MultivariateSpec::MultipleImputation
    let mi = AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
        imputations_m: 10,
        max_iters: 25,
        seed: 777,
        response_column: None,
        predictor_columns: vec![0, 1, 2],
    });
    assert_eq!(mi.derive_auto_title(), "Multiple imputation");
    assert!(mi.is_compatible_with(TableKind::MultipleVariables));
    assert_json_roundtrip(&mi);

    // TwoGroupSpec MV columns
    let tg = TwoGroupSpec { mv_group_column: Some(0), mv_response_column: Some(1), ..TwoGroupSpec::default() };
    assert_json_roundtrip(&tg);

    // ParameterScope, ModelComparisonSpec, NonlinearFitSpec extended fields & upstream_sheet_ids
    let mut nl = NonlinearFitSpec::new("one_site_fit_ki");
    nl.parameter_rules = vec![
        ParameterRule {
            name: String::from("Top"),
            scope: ParameterScope::PerDatasetConstant(vec![(0, 100.0), (1, 120.0)]),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("Ligand"),
            scope: ParameterScope::ColumnTitleConstant,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("HotNM"),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: String::from("HotNM"),
            }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];
    nl.comparison = Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: String::from("two_site") });
    nl.x_range_filter = Some((Some(0.1), Some(100.0)));
    nl.y_range_filter = Some((None, Some(500.0)));
    nl.plot_function_only = true;
    nl.plot_x_min = Some(-10.0);
    nl.plot_x_max = Some(-2.0);
    nl.homoscedasticity_test = true;
    nl.emit_residuals_tab = true;
    nl.emit_covariance_tab = true;
    nl.mv_group_columns = vec![0, 1];
    nl.mv_x_columns = vec![2];
    nl.mv_y_column = Some(3);
    let nl_kind = AnalysisKind::NonlinearRegression(nl);
    assert_eq!(nl_kind.upstream_sheet_ids(), vec![SheetId::new("info-1")]);
    assert_json_roundtrip(&nl_kind);

    // ResultTabKind new variants
    for tab_kind in [ResultTabKind::CovarianceMatrix, ResultTabKind::NormalQq, ResultTabKind::BaselineSurvival, ResultTabKind::LogMinusLog] {
        assert_json_roundtrip(&tab_kind);
    }

    // AdditionalTick on AxisSpec + validation
    let mut axis = AxisSpec::linear("Concentration");
    axis.additional_ticks = vec![
        AdditionalTick { value: 2.5, label: Some(String::from("IC50")), draw_gridline: true },
        AdditionalTick { value: 10.0, label: None, draw_gridline: false },
    ];
    axis.validate().unwrap();
    assert_json_roundtrip(&axis);

    let mut bad_axis = AxisSpec::linear("TooLongTick");
    bad_axis.additional_ticks.push(AdditionalTick { value: 1.0, label: Some("x".repeat(curvebook_doc::MAX_TITLE_BYTES + 1)), draw_gridline: false });
    assert!(matches!(bad_axis.validate(), Err(DocError::TitleTooLong { .. })));
}

#[test]
fn legacy_json_deserializes_without_new_schema_fields() {
    let legacy_two_group = r#"{"method":"WelchT","two_tailed":false,"compute_effect_sizes":true}"#;
    let decoded_tg: TwoGroupSpec = serde_json::from_str(legacy_two_group).unwrap();
    assert_eq!(decoded_tg.method, TwoGroupMethod::WelchT);
    assert!(!decoded_tg.two_tailed);
    assert_eq!(decoded_tg.mv_group_column, None);
    assert_eq!(decoded_tg.mv_response_column, None);

    let legacy_km = r#"{"KaplanMeier":{"log_rank_mantel_cox":true,"gehan_breslow_wilcoxon":true,"trend_test":false,"asymmetrical_greenwood_ci":true,"multiplicity":{"method":"Bonferroni","threshold":0.05,"report_confidence_intervals":false}}}"#;
    let decoded_km: SurvivalAnalysisSpec = serde_json::from_str(legacy_km).unwrap();
    assert!(matches!(
        decoded_km,
        SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: true,
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
            ..
        }
    ));

    let legacy_nl = r#"{
        "equation_id":"four_pl",
        "custom_dsl":null,
        "method":"LeastSquares",
        "weighting":"None",
        "parameter_rules":[],
        "profile_likelihood_ci":true,
        "comparison":null,
        "confidence_bands":true,
        "prediction_bands":false,
        "interpolate_unknowns":false,
        "replicates_lack_of_fit_test":false,
        "normality_of_residuals":false,
        "max_iters":1000
    }"#;
    let decoded_nl: NonlinearFitSpec = serde_json::from_str(legacy_nl).unwrap();
    assert_eq!(decoded_nl.equation_id, "four_pl");
    assert_eq!(decoded_nl.x_range_filter, None);
    assert_eq!(decoded_nl.y_range_filter, None);
    assert!(!decoded_nl.plot_function_only);
    assert_eq!(decoded_nl.plot_x_min, None);
    assert_eq!(decoded_nl.plot_x_max, None);
    assert!(!decoded_nl.homoscedasticity_test);
    assert!(!decoded_nl.emit_residuals_tab);
    assert!(!decoded_nl.emit_covariance_tab);
    assert!(decoded_nl.mv_group_columns.is_empty());
    assert!(decoded_nl.mv_x_columns.is_empty());
    assert_eq!(decoded_nl.mv_y_column, None);

    let legacy_axis = r#"{
        "title":"Dose",
        "segments":[{"min":null,"max":null,"scale":"Linear","length_share":1.0,"major_tick_interval":null,"minor_ticks_per_interval":0}],
        "gap_width_pt":8.0,
        "break_ornament":"DoubleSlash",
        "tick_format":"Decimal",
        "tick_angle_deg":0.0,
        "stroke_width_pt":1.0,
        "thresholds":[]
    }"#;
    let decoded_axis: AxisSpec = serde_json::from_str(legacy_axis).unwrap();
    assert_eq!(decoded_axis, AxisSpec::linear("Dose"));

    assert_eq!(serde_json::from_str::<BlandAltmanSpec>("{}").unwrap(), BlandAltmanSpec::default());
    assert_eq!(serde_json::from_str::<MultipleRowTestsSpec>("{}").unwrap(), MultipleRowTestsSpec::default());
    assert_eq!(serde_json::from_str::<PassingBablokSpec>("{}").unwrap(), PassingBablokSpec::default());
    assert_eq!(serde_json::from_str::<AdditionalTick>(r#"{"value":3.5}"#).unwrap(), AdditionalTick { value: 3.5, label: None, draw_gridline: false });
}

#[test]
fn wave20_contingency_and_two_group_seams() {
    assert_eq!(KappaWeighting::default(), KappaWeighting::Unweighted);
    for w in [KappaWeighting::Unweighted, KappaWeighting::Linear, KappaWeighting::Quadratic] {
        assert_json_roundtrip(&w);
        assert_json_roundtrip(&ContingencyMethod::CohenKappa { weighting: w });
    }
    let legacy_kappa: ContingencyMethod = serde_json::from_str(r#"{"CohenKappa":{}}"#).unwrap();
    assert_eq!(legacy_kappa, ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted });

    assert_json_roundtrip(&ContingencyMethod::FleissKappa);
    assert_json_roundtrip(&ContingencyMethod::ExactBinomial);

    let default_cont = ContingencySpec::default();
    assert_eq!(default_cont.expected_proportions, None);
    assert_eq!(default_cont.null_proportion, None);
    let default_cont_json = serde_json::to_value(&default_cont).unwrap();
    assert!(default_cont_json.get("expected_proportions").is_none());
    assert!(default_cont_json.get("null_proportion").is_none());
    assert_eq!(serde_json::from_value::<ContingencySpec>(default_cont_json).unwrap(), default_cont);

    let legacy_cont_json = r#"{
        "method":"FisherExact",
        "compute_odds_and_risk_ratios":true,
        "compute_sensitivity_specificity":false,
        "compute_phi_and_cramers_v":true
    }"#;
    let legacy_cont: ContingencySpec = serde_json::from_str(legacy_cont_json).unwrap();
    assert_eq!(legacy_cont.method, ContingencyMethod::FisherExact);
    assert!(legacy_cont.compute_odds_and_risk_ratios);
    assert!(legacy_cont.compute_phi_and_cramers_v);
    assert_eq!(legacy_cont.expected_proportions, None);
    assert_eq!(legacy_cont.null_proportion, None);

    let custom_cont = ContingencySpec {
        method: ContingencyMethod::ExactBinomial,
        compute_odds_and_risk_ratios: false,
        compute_sensitivity_specificity: false,
        compute_phi_and_cramers_v: false,
        expected_proportions: Some(vec![0.25, 0.75]),
        null_proportion: Some(0.5),
    };
    assert_json_roundtrip(&custom_cont);

    let sign_spec = TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..TwoGroupSpec::default() };
    assert_json_roundtrip(&TwoGroupMethod::PairedSignTest);
    assert_json_roundtrip(&sign_spec);
    assert_eq!(AnalysisKind::TwoGroupComparison(sign_spec).derive_auto_title(), "Sign test");

    let ak_cohen = AnalysisKind::Contingency(ContingencySpec {
        method: ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic },
        ..ContingencySpec::default()
    });
    assert_eq!(ak_cohen.derive_auto_title(), "Cohen's kappa");
    assert!(ak_cohen.is_compatible_with(TableKind::Contingency));
    assert!(!ak_cohen.is_compatible_with(TableKind::PartsOfWhole));
    assert!(!ak_cohen.is_compatible_with(TableKind::Column));

    let ak_fleiss = AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::FleissKappa, ..ContingencySpec::default() });
    assert_eq!(ak_fleiss.derive_auto_title(), "Fleiss' kappa");
    assert!(ak_fleiss.is_compatible_with(TableKind::Contingency));
    assert!(!ak_fleiss.is_compatible_with(TableKind::PartsOfWhole));
    assert!(!ak_fleiss.is_compatible_with(TableKind::Column));

    let ak_binom = AnalysisKind::Contingency(custom_cont);
    assert_eq!(ak_binom.derive_auto_title(), "Exact binomial test");
    assert!(ak_binom.is_compatible_with(TableKind::Contingency));
    assert!(ak_binom.is_compatible_with(TableKind::PartsOfWhole));
    assert!(ak_binom.is_compatible_with(TableKind::Column));
    for incompatible in [TableKind::Xy, TableKind::Grouped, TableKind::Survival, TableKind::MultipleVariables, TableKind::Nested] {
        assert!(!ak_binom.is_compatible_with(incompatible));
    }
}

#[test]
fn wave20_random_sample_and_clustering_seams() {
    let sample_full = TransformAnalysisSpec::RandomSample { seed: 42, sample_size: 25, stratify_column: Some(1), proportional_allocation: true };
    assert_json_roundtrip(&sample_full);
    let legacy_sample: TransformAnalysisSpec = serde_json::from_str(r#"{"RandomSample":{"seed":7,"sample_size":10}}"#).unwrap();
    assert_eq!(legacy_sample, TransformAnalysisSpec::RandomSample { seed: 7, sample_size: 10, stratify_column: None, proportional_allocation: true });

    let ak_sample = AnalysisKind::Transform(sample_full);
    assert_eq!(ak_sample.derive_auto_title(), "Random sample");
    for kind in TableKind::ALL {
        assert!(ak_sample.is_compatible_with(kind));
    }

    assert_eq!(ClusterDistance::default(), ClusterDistance::Euclidean);
    for d in [ClusterDistance::Euclidean, ClusterDistance::Manhattan, ClusterDistance::Pearson, ClusterDistance::Cosine] {
        assert_json_roundtrip(&d);
    }
    assert_eq!(ClusterLinkage::default(), ClusterLinkage::Ward);
    for l in [
        ClusterLinkage::Ward,
        ClusterLinkage::Complete,
        ClusterLinkage::Single,
        ClusterLinkage::Upgma,
        ClusterLinkage::Wpgma,
        ClusterLinkage::Centroid,
        ClusterLinkage::Median,
    ] {
        assert_json_roundtrip(&l);
    }

    let full_cluster =
        MultivariateSpec::Clustering { k_means_k: Some(4), distance: ClusterDistance::Cosine, linkage: ClusterLinkage::Upgma, k_sweep_max: Some(8) };
    assert_json_roundtrip(&full_cluster);
    let default_cluster =
        MultivariateSpec::Clustering { k_means_k: None, distance: ClusterDistance::default(), linkage: ClusterLinkage::default(), k_sweep_max: None };
    let default_cluster_val = serde_json::to_value(&default_cluster).unwrap();
    assert!(default_cluster_val["Clustering"].get("k_sweep_max").is_none());

    let legacy_cluster: MultivariateSpec = serde_json::from_str(r#"{"Clustering":{"k_means_k":3}}"#).unwrap();
    assert_eq!(
        legacy_cluster,
        MultivariateSpec::Clustering { k_means_k: Some(3), distance: ClusterDistance::Euclidean, linkage: ClusterLinkage::Ward, k_sweep_max: None }
    );
}

#[test]
fn wave20_model_equation_annotation_seam() {
    assert_eq!(EquationDisplayMode::default(), EquationDisplayMode::Substituted);
    assert_json_roundtrip(&EquationDisplayMode::Substituted);
    assert_json_roundtrip(&EquationDisplayMode::Symbolic);

    let eq_ann = GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("fit-1"),
        data_set_index: 2,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: RectPt::new(24.0, 36.0, 180.0, 48.0),
        font_size_pt: 12.5,
        bordered: true,
    };
    assert_json_roundtrip(&eq_ann);

    let minimal_eq_json = r#"{
        "ModelEquation":{
            "result_sheet_id":"fit-2",
            "bounds_pt":{"x":10.0,"y":20.0,"width":140.0,"height":32.0}
        }
    }"#;
    let decoded_eq: GraphAnnotation = serde_json::from_str(minimal_eq_json).unwrap();
    assert_eq!(
        decoded_eq,
        GraphAnnotation::ModelEquation {
            result_sheet_id: SheetId::new("fit-2"),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(10.0, 20.0, 140.0, 32.0),
            font_size_pt: 10.0,
            bordered: false,
        }
    );

    // Verify ProjectDocument::upstream_dependencies tracks ModelEquation result_sheet_id.
    let mut proj = ProjectDocument::new("ModelEquation DAG").unwrap();
    let ds = DataSheet::new("data-1", "XY", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).unwrap())).unwrap();
    proj.add_data_sheet(ds.clone()).unwrap();
    let fit_spec = AnalysisSpec::new("fit-1", "data-1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("four_pl"))).unwrap();
    proj.add_result_sheet(ResultSheet::new(fit_spec).unwrap()).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", &ds).unwrap();
    gs.spec.annotations.push(eq_ann);
    proj.add_graph_sheet(gs).unwrap();
    assert_eq!(proj.upstream_dependencies(&SheetId::new("graph-1")), vec![SheetId::new("data-1"), SheetId::new("fit-1")]);
}

#[test]
fn project_document_reorder_sheet_across_all_folders_and_bounds() {
    let mut proj = ProjectDocument::new("Reorder Test").unwrap();

    // Empty folder bounds check
    assert!(matches!(
        proj.reorder_sheet(SheetFolderKind::Data, 0, 0),
        Err(DocError::SheetIndexOutOfBounds { folder: SheetFolderKind::Data, index: 0, len: 0 })
    ));

    let make_xy = |id: &str| DataSheet::new(id, id, TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).unwrap())).unwrap();
    let ds_a = make_xy("d-a");
    let ds_b = make_xy("d-b");
    let ds_c = make_xy("d-c");
    proj.add_data_sheet(ds_a.clone()).unwrap();
    proj.add_data_sheet(ds_b).unwrap();
    proj.add_data_sheet(ds_c).unwrap();

    for id in ["i-a", "i-b", "i-c"] {
        proj.add_info_sheet(InfoSheet::new_preseeded(id, id).unwrap()).unwrap();
    }
    for id in ["r-a", "r-b", "r-c"] {
        let spec = AnalysisSpec::new(id, "d-a", AnalysisKind::BlandAltman(BlandAltmanSpec::default())).unwrap();
        proj.add_result_sheet(ResultSheet::new(spec).unwrap()).unwrap();
    }
    for id in ["g-a", "g-b", "g-c"] {
        proj.add_graph_sheet(GraphSheet::new_uninitialized_for_data(id, &ds_a).unwrap()).unwrap();
    }
    for id in ["l-a", "l-b", "l-c"] {
        proj.add_layout_sheet(LayoutSheet::new(id, id, LayoutSpec::default()).unwrap()).unwrap();
    }

    // Forward move (0 -> 2): [a, b, c] -> [b, c, a]
    proj.reorder_sheet(SheetFolderKind::Data, 0, 2).unwrap();
    let data_ids: Vec<&str> = proj.data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(data_ids, vec!["d-b", "d-c", "d-a"]);

    // Backward move (2 -> 0): [b, c, a] -> [a, b, c]
    proj.reorder_sheet(SheetFolderKind::Data, 2, 0).unwrap();
    let data_ids: Vec<&str> = proj.data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(data_ids, vec!["d-a", "d-b", "d-c"]);

    // Adjacent swaps + no-op across Info, Results, Graphs, Layouts
    proj.reorder_sheet(SheetFolderKind::Info, 1, 2).unwrap();
    assert_eq!(proj.info_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["i-a", "i-c", "i-b"]);
    proj.reorder_sheet(SheetFolderKind::Info, 1, 1).unwrap();
    assert_eq!(proj.info_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["i-a", "i-c", "i-b"]);

    proj.reorder_sheet(SheetFolderKind::Results, 2, 1).unwrap();
    assert_eq!(proj.result_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["r-a", "r-c", "r-b"]);

    proj.reorder_sheet(SheetFolderKind::Graphs, 0, 1).unwrap();
    assert_eq!(proj.graph_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["g-b", "g-a", "g-c"]);

    proj.reorder_sheet(SheetFolderKind::Layouts, 2, 0).unwrap();
    assert_eq!(proj.layout_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["l-c", "l-a", "l-b"]);

    // Out-of-bounds checks preserve slice state
    assert!(matches!(
        proj.reorder_sheet(SheetFolderKind::Graphs, 3, 0),
        Err(DocError::SheetIndexOutOfBounds { folder: SheetFolderKind::Graphs, index: 3, len: 3 })
    ));
    assert!(matches!(
        proj.reorder_sheet(SheetFolderKind::Layouts, 0, 3),
        Err(DocError::SheetIndexOutOfBounds { folder: SheetFolderKind::Layouts, index: 3, len: 3 })
    ));
    assert_eq!(proj.graph_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["g-b", "g-a", "g-c"]);
    assert_eq!(proj.layout_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["l-c", "l-a", "l-b"]);

    assert_eq!(SheetFolderKind::Results.section(), curvebook_doc::SheetSection::Analyses);
    assert_json_roundtrip(&proj);
}

proptest! {
    #[test]
    fn prop_cell_and_xy_table_round_trip(
        cells in prop::collection::vec(arb_cell(), 1..=16),
        submode in arb_subcolumn_mode(),
        progression in prop_oneof![Just(SequenceProgression::Arithmetic), Just(SequenceProgression::Geometric)],
        start in arb_finite_f64(),
        step in arb_finite_f64(),
    ) {
        for cell in &cells {
            assert_json_roundtrip(cell);
        }

        let mut xy = XyTable::new(submode, 1, 2).unwrap();
        xy.x_mode = XColumnMode::GeneratedSequence { start, step, progression };
        let subs = submode.subcolumn_count();
        for (i, cell) in cells.iter().take(subs).enumerate() {
            xy.data_sets[0].cells[0][i] = cell.clone();
        }
        assert_json_roundtrip(&xy);
    }

    #[test]
    fn prop_confidence_level_pvalue_format_and_axis_scale_round_trip(
        pct in 0.01_f64..=99.99_f64,
        pfmt in arb_pvalue_format(),
        scale in arb_axis_scale(),
        rgba in arb_rgba8(),
    ) {
        let level = ConfidenceLevel::from_percent(pct).unwrap();
        assert_json_roundtrip(&level);
        assert_json_roundtrip(&pfmt);
        assert_json_roundtrip(&scale);
        assert_json_roundtrip(&rgba);
    }

    #[test]
    fn prop_simulate_analysis_spec_round_trip(
        sim in arb_simulate_spec(),
        pct in 0.01_f64..=99.99_f64,
        pfmt in arb_pvalue_format(),
        sheet_id in "[a-z][a-z0-9-]{1,12}",
        source_id in "[a-z][a-z0-9-]{1,12}",
    ) {
        let mut spec = AnalysisSpec::new(sheet_id, source_id, AnalysisKind::Transform(sim)).unwrap();
        spec.confidence_level = ConfidenceLevel::from_percent(pct).unwrap();
        spec.p_value_format = pfmt;
        assert_json_roundtrip(&spec);
    }
}

#[test]
fn pvalue_stack_spec_defaults_compatibility_and_serde_roundtrip() {
    use curvebook_doc::{PValueStackMethod, PValueStackSpec};

    let minimal_json = r#"{}"#;
    let decoded_default: PValueStackSpec = serde_json::from_str(minimal_json).unwrap();
    assert_eq!(decoded_default, PValueStackSpec::default());
    assert_eq!(decoded_default.method, PValueStackMethod::BenjaminiHochberg);
    assert!((decoded_default.threshold - 0.05).abs() < 1e-12);
    assert_eq!(decoded_default.column, None);

    for method in PValueStackMethod::ALL {
        let spec = PValueStackSpec { method, threshold: 0.1, column: Some(2) };
        let kind = AnalysisKind::PValueStack(spec);
        assert!(kind.is_compatible_with(TableKind::Column));
        assert!(kind.is_compatible_with(TableKind::MultipleVariables));
        assert!(kind.is_compatible_with(TableKind::Xy));
        assert!(kind.is_compatible_with(TableKind::Grouped));
        assert!(!kind.is_compatible_with(TableKind::Contingency));
        assert!(!kind.is_compatible_with(TableKind::Survival));
        assert!(!kind.is_compatible_with(TableKind::Nested));
        assert!(kind.derive_auto_title().contains(method.label()));

        let full = AnalysisSpec::new("res-p", "col-p", kind).unwrap();
        assert_json_roundtrip(&full);
    }
}

#[test]
fn r63_select_and_transform_derived_columns_serde_and_legacy() {
    use curvebook_doc::DerivedColumnFormula;

    let legacy_json = r#"{"SelectAndTransform":{"filter_formula":"X > 0","target_archetype":"Column"}}"#;
    let legacy: TransformAnalysisSpec = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(
        legacy,
        TransformAnalysisSpec::SelectAndTransform {
            filter_formula: Some(String::from("X > 0")),
            target_archetype: TableKind::Column,
            derived_columns: Vec::new(),
        }
    );
    let reserialized = serde_json::to_value(&legacy).unwrap();
    assert!(reserialized["SelectAndTransform"].get("derived_columns").is_none());

    let with_derived = TransformAnalysisSpec::SelectAndTransform {
        filter_formula: Some(String::from("Flag == 1")),
        target_archetype: TableKind::Xy,
        derived_columns: vec![
            DerivedColumnFormula::new("Norm", "Response / mean(Response) OVER (PARTITION BY Batch)"),
            DerivedColumnFormula::new("Z", "zscore(Norm)"),
        ],
    };
    assert_json_roundtrip(&with_derived);

    let alias_json = r#"{"name":"Delta","expr":"Response - minif(Response, Role == \"Ctrl\")"}"#;
    let alias_dc: DerivedColumnFormula = serde_json::from_str(alias_json).unwrap();
    assert_eq!(alias_dc.title, "Delta");
    assert_eq!(alias_dc.formula, "Response - minif(Response, Role == \"Ctrl\")");
}

#[test]
fn fa_x2_01_import_filter_spec_baseline_and_predicate_serde_and_legacy_compat() {
    use curvebook_doc::{BaselineOperation, InternalLinkKind};

    // 1. Legacy ImportFilterSpec JSON without `baseline` or `exclude_row_predicate` keys deserializes cleanly.
    let legacy_filter_json = r#"{
        "row_window": [0, 9],
        "col_window": null,
        "decimate_every_k": 2,
        "unstack_group_column": null,
        "parse_asterisk_exclusions": true
    }"#;
    let legacy_filter: ImportFilterSpec = serde_json::from_str(legacy_filter_json).unwrap();
    assert_eq!(legacy_filter.row_window, Some((0, 9)));
    assert_eq!(legacy_filter.decimate_every_k, Some(2));
    assert_eq!(legacy_filter.baseline, None);
    assert_eq!(legacy_filter.exclude_row_predicate, None);

    // Re-serializing `None` fields omits them cleanly for wire compatibility.
    let reserialized_filter = serde_json::to_value(&legacy_filter).unwrap();
    assert!(reserialized_filter.get("baseline").is_none());
    assert!(reserialized_filter.get("exclude_row_predicate").is_none());

    // 2. Explicit `null` values also deserialize cleanly to `None`.
    let null_fields_json = r#"{
        "row_window": null,
        "col_window": null,
        "decimate_every_k": null,
        "unstack_group_column": null,
        "parse_asterisk_exclusions": false,
        "baseline": null,
        "exclude_row_predicate": null
    }"#;
    let null_decoded: ImportFilterSpec = serde_json::from_str(null_fields_json).unwrap();
    assert!(!null_decoded.parse_asterisk_exclusions);
    assert_eq!(null_decoded.baseline, None);
    assert_eq!(null_decoded.exclude_row_predicate, None);

    // 3. Round-trip all BaselineOperation variants + exclude_row_predicate on ImportLinkSpec and ProjectDocument.
    for op in [BaselineOperation::Difference, BaselineOperation::Ratio, BaselineOperation::PercentChange, BaselineOperation::FractionalChange] {
        let spec = ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("src-1"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(5, 2, 0)),
            },
            filter: ImportFilterSpec {
                baseline: Some((0, op)),
                exclude_row_predicate: Some(String::from("B > 25.0")),
                ..ImportFilterSpec::default()
            },
            placement: ImportPlacementSpec::default(),
        };
        assert_json_roundtrip(&spec);
    }

    // 4. Full ProjectDocument with legacy LinkedDataRegion missing `baseline`/`exclude_row_predicate` deserializes cleanly.
    let mut doc = ProjectDocument::new("Legacy Link Compat").unwrap();
    let mut dst =
        DataSheet::new("dst-1", "Dest", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap())).unwrap();
    dst.add_linked_region(LinkedDataRegion {
        id: String::from("lnk-1"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("plate.csv"), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec {
                baseline: Some((0, BaselineOperation::PercentChange)),
                exclude_row_predicate: Some(String::from("A < 0")),
                ..ImportFilterSpec::default()
            },
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 1, 0)),
        provenance_summary: String::from("CSV link"),
    })
    .unwrap();
    doc.add_data_sheet(dst).unwrap();
    assert_json_roundtrip(&doc);

    let mut legacy_doc_val = serde_json::to_value(&doc).unwrap();
    let filter_obj = legacy_doc_val["data_sheets"][0]["linked_regions"][0]["spec"]["filter"].as_object_mut().unwrap();
    assert!(filter_obj.remove("baseline").is_some());
    assert!(filter_obj.remove("exclude_row_predicate").is_some());
    let legacy_doc: ProjectDocument = serde_json::from_value(legacy_doc_val).unwrap();
    let restored_filter = &legacy_doc.data_sheets[0].linked_regions[0].spec.filter;
    assert_eq!(restored_filter.baseline, None);
    assert_eq!(restored_filter.exclude_row_predicate, None);
}
