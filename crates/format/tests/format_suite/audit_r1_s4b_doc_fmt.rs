use std::path::Path;

use crate::common::{TempDirGuard, bracket_spec, callout_item, live_graph_item, make_import_link_spec, param_rule, result_sheet_with_kind};
use curvebook_doc::{
    AdditionalTick, AnalysisKind, AreaFillSpec, AxisThreshold, CellCoord, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConnectingLineStyle,
    ContingencySpec, DataSheet, DescriptiveSpec, ErrorBarSpec, FactorialAnovaSpec, FitMethod, FloatingNote, GraphAnnotation, GraphSheet,
    GraphicalDataObject, GroupedTable, InfoSheet, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, LineConnectionKind, MathTransformPreset,
    MultipleRowTestsSpec, MultiplicitySpec, MultivariateGraphEncoding, MultivariatePlotMode, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor,
    OneWayAnovaSpec, OutlierSpec, ParameterScope, PlacementMode, PointStyleOverride, ProjectDocument, RectPt, ReplicateLineMode, SeriesOffsetSpec,
    SeriesSpec, SheetId, SimulationNoise, SizeChannelMapping, SizePt, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec,
    TwoGroupMethod, TwoGroupSpec, WeightingScheme, XyTable,
};
use curvebook_format::{
    FormatError, ImportBaselineAdjustment, apply_csv_import_link, open_project_bytes, open_project_unpacked_dir, save_project_bytes,
    save_project_unpacked_dir, save_template_bytes, validate_project_document,
};
use curvebook_palette::{ColorRamp, ColorStop, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};

fn build_rich_valid_project() -> Result<ProjectDocument, FormatError> {
    let mut doc = ProjectDocument::new("Rich S06-06 Assay")?;
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Info")?)?;

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2)?;
    grp.row_numeric_spacing = Some(vec![Some(1.0), Some(2.0)]);
    let mut ds = DataSheet::new("data-1", "Grouped", TableArchetype::Grouped(grp))?;
    ds.floating_notes.push(FloatingNote::new("dn-1", RectPt::new(5.0, 5.0, 60.0, 30.0), "Note"));
    doc.add_data_sheet(ds.clone())?;

    let mut fit = NonlinearFitSpec::new("four_pl");
    let mut rule = param_rule("Top", ParameterScope::Constant(100.0));
    (rule.initial_override, rule.lower_bound, rule.upper_bound) = (Some(95.0), Some(0.0), Some(200.0));
    fit.parameter_rules.push(rule);
    let mut res = result_sheet_with_kind("res-1", "data-1", AnalysisKind::NonlinearRegression(fit));
    res.analyzable_data = Some(ds.clone());
    doc.add_result_sheet(res)?;

    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", &ds)?;
    gs.spec.canvas_size_pt = SizePt::new(320.0, 240.0);
    gs.spec.x_axis.thresholds.push(AxisThreshold {
        value: 5.0,
        shade_to: Some(10.0),
        color: Rgba8::BLACK,
        dash_style: LineDashStyle::Solid,
        stroke_width_pt: 1.0,
        label: None,
    });
    gs.spec.x_axis.additional_ticks.push(AdditionalTick { value: 2.5, label: None, draw_gridline: false });
    gs.spec.series.push(SeriesSpec {
        source_sheet_id: SheetId::new("data-1"),
        data_set_index: 0,
        legend_label: None,
        bind_to_right_y2: false,
        gdo: GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle {
                shape: MarkerShape::Circle,
                size_pt: 5.0,
                stroke_width_pt: 1.0,
                fill_color: Rgba8::BLACK,
                stroke_color: Rgba8::BLACK,
                fill_mode: MarkerFillMode::Filled,
            }),
            line: Some(ConnectingLineStyle {
                kind: LineConnectionKind::PointToPoint,
                dash_style: LineDashStyle::Solid,
                stroke_width_pt: 1.5,
                color: Rgba8::BLACK,
            }),
            area_fill: Some(AreaFillSpec { baseline_y: 0.0, fill_color: Rgba8::BLACK }),
            replicate_lines: ReplicateLineMode::default(),
        },
        y_error_bars: Some(ErrorBarSpec::default()),
        x_error_bars: None,
        regression_overlay: None,
        offset: SeriesOffsetSpec::default(),
        bar_width_override_pt: None,
        show_raw_data: true,
    });
    gs.spec.comparisons.push(bracket_spec("res-1"));
    gs.spec.annotations.push(GraphAnnotation::LineOrArrow {
        start_pt: (1.0, 2.0),
        end_pt: (3.0, 4.0),
        arrowhead: true,
        dash: LineDashStyle::Solid,
        color: Rgba8::BLACK,
    });
    gs.spec.point_overrides.push(PointStyleOverride {
        series_index: 0,
        point_index: 0,
        size_pt: Some(6.0),
        border_width_pt: Some(1.0),
        ..Default::default()
    });
    gs.spec.legend.position_pt = Some((20.0, 20.0));
    gs.floating_notes.push(FloatingNote::new("gn-1", RectPt::new(10.0, 10.0, 50.0, 25.0), "Graph Note"));
    doc.add_graph_sheet(gs)?;

    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    lspec.items.extend([live_graph_item("item-g", "graph-1"), callout_item("item-t", "Panel A")]);
    let mut lsheet = LayoutSheet::new("layout-1", "Fig 1", lspec)?;
    lsheet.floating_notes.push(FloatingNote::new("ln-1", RectPt::new(2.0, 2.0, 30.0, 15.0), "Layout Note"));
    doc.add_layout_sheet(lsheet)?;
    Ok(doc)
}

fn assert_save_paths_reject_nonfinite(label: &str, doc: &ProjectDocument, reject_dir: &Path) {
    assert!(matches!(validate_project_document(doc), Err(FormatError::NonFiniteFloat { .. })), "{label}: validate");
    assert!(matches!(save_project_bytes(doc), Err(FormatError::NonFiniteFloat { .. })), "{label}: save_bytes");
    assert!(matches!(save_template_bytes(doc), Err(FormatError::NonFiniteFloat { .. })), "{label}: save_template");
    assert!(matches!(save_project_unpacked_dir(doc, reject_dir), Err(FormatError::NonFiniteFloat { .. })), "{label}: save_unpacked");
}

#[test]
fn r1_s06_06_valid_document_round_trips_archive_and_unpacked_dir() -> Result<(), FormatError> {
    let doc = build_rich_valid_project()?;
    assert_eq!(open_project_bytes(&save_project_bytes(&doc)?)?, doc);
    let tmp = TempDirGuard::new("roundtrip")?;
    save_project_unpacked_dir(&doc, tmp.path())?;
    assert_eq!(open_project_unpacked_dir(tmp.path())?, doc);
    Ok(())
}

#[test]
fn r1_s06_06_nonfinite_floats_rejected_across_graphs_layouts_notes_and_analyses() -> Result<(), FormatError> {
    let base = build_rich_valid_project()?;
    let tmp = TempDirGuard::new("reject")?;

    type DocMutFn = fn(&mut ProjectDocument);
    let doc_mutators: &[(&str, DocMutFn)] = &[
        ("graph.y_axis.stroke_width_pt", |d| d.graph_sheets[0].spec.y_axis.stroke_width_pt = f64::NAN),
        ("graph.x_axis.segments[0].max", |d| d.graph_sheets[0].spec.x_axis.segments[0].max = Some(f64::INFINITY)),
        ("graph.x_axis.thresholds[0].shade_to", |d| d.graph_sheets[0].spec.x_axis.thresholds[0].shade_to = Some(f64::NEG_INFINITY)),
        ("graph.x_axis.additional_ticks[0].value", |d| d.graph_sheets[0].spec.x_axis.additional_ticks[0].value = f64::NAN),
        ("graph.canvas_size_pt.height", |d| d.graph_sheets[0].spec.canvas_size_pt.height = f64::INFINITY),
        ("graph.series.area_fill.baseline_y", |d| {
            if let Some(s) = d.graph_sheets[0].spec.series.last_mut()
                && let GraphicalDataObject::ScatterAndLine { area_fill: Some(ref mut af), .. } = s.gdo
            {
                af.baseline_y = f64::NAN;
            }
        }),
        ("graph.series.y_error_bars.cap_width_pt", |d| {
            if let Some(s) = d.graph_sheets[0].spec.series.last_mut()
                && let Some(ref mut eb) = s.y_error_bars
            {
                eb.cap_width_pt = f64::INFINITY;
            }
        }),
        ("graph.multivariate.domain_clamp_or_ramp", |d| {
            d.graph_sheets[0].spec.multivariate = Some(MultivariateGraphEncoding {
                plot_mode: MultivariatePlotMode::BubbleScatter,
                x_column: 0,
                y_column: 1,
                color: ColorChannelMapping::Continuous {
                    column: 2,
                    ramp: ColorRamp::Custom { stops: vec![ColorStop { position: f64::NAN, color: Rgba8::BLACK }] },
                    domain_clamp: Some((0.0, f64::INFINITY)),
                },
                size: SizeChannelMapping::Variable { column: 3, min_radius_pt: 2.0, max_radius_pt: 8.0 },
                label_column: None,
                x_level_order_override: None,
                show_convex_hulls: false,
                show_confidence_ellipses: false,
            });
        }),
        ("graph.comparisons.bracket_step_pt", |d| d.graph_sheets[0].spec.comparisons[0].bracket_step_pt = f64::NAN),
        ("graph.annotations.end_pt", |d| {
            if let GraphAnnotation::LineOrArrow { ref mut end_pt, .. } = d.graph_sheets[0].spec.annotations[0] {
                end_pt.0 = f64::INFINITY;
            }
        }),
        ("graph.point_overrides.size_pt", |d| d.graph_sheets[0].spec.point_overrides[0].size_pt = Some(f64::NAN)),
        ("graph.legend.position_pt", |d| d.graph_sheets[0].spec.legend.position_pt = Some((f64::NEG_INFINITY, 10.0))),
        ("graph.floating_notes", |d| d.graph_sheets[0].floating_notes[0].bounds_pt.x = f64::NAN),
        ("layout.spec.width", |d| d.layout_sheets[0].spec.width = f64::NAN),
        ("layout.items.live_graph.scale_percent", |d| {
            if let LayoutItem::LiveGraph { ref mut scale_percent, .. } = d.layout_sheets[0].spec.items[0] {
                *scale_percent = f64::INFINITY;
            }
        }),
        ("layout.items.text_callout.font_size_pt", |d| {
            if let LayoutItem::TextCallout { ref mut font_size_pt, .. } = d.layout_sheets[0].spec.items[1] {
                *font_size_pt = f64::NEG_INFINITY;
            }
        }),
        ("layout.floating_notes", |d| d.layout_sheets[0].floating_notes[0].bounds_pt.height = f64::NAN),
        ("data.grouped.row_numeric_spacing", |d| {
            if let TableArchetype::Grouped(ref mut g) = d.data_sheets[0].archetype {
                g.row_numeric_spacing = Some(vec![Some(1.0), Some(f64::NAN)]);
            }
        }),
        ("data.floating_notes", |d| d.data_sheets[0].floating_notes[0].bounds_pt.width = f64::INFINITY),
        ("info.floating_notes", |d| {
            d.info_sheets[0].floating_notes.push(FloatingNote::new("in", RectPt { x: f64::NAN, y: 0.0, width: 10.0, height: 10.0 }, "bad"))
        }),
        ("result.floating_notes", |d| {
            d.result_sheets[0].floating_notes.push(FloatingNote::new("rn", RectPt { x: 0.0, y: 0.0, width: f64::NAN, height: 10.0 }, "bad"))
        }),
        ("result.analyzable_data.row_numeric_spacing", |d| {
            if let Some(ref mut a) = d.result_sheets[0].analyzable_data
                && let TableArchetype::Grouped(ref mut g) = a.archetype
            {
                g.row_numeric_spacing = Some(vec![Some(f64::INFINITY), Some(2.0)]);
            }
        }),
    ];
    for &(label, mutate) in doc_mutators {
        let mut d = base.clone();
        mutate(&mut d);
        assert_save_paths_reject_nonfinite(label, &d, tmp.path());
    }

    let mult = |threshold| MultiplicitySpec { threshold, ..Default::default() };
    let sim = |params, noise| {
        AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
            seed: 1,
            row_count: 5,
            replicates: 1,
            x_min: 0.0,
            x_max: 10.0,
            equation_id: String::from("line"),
            parameters: params,
            noise,
            simulation_mode: None,
        })
    };
    let bad_kinds: Vec<(&str, AnalysisKind)> = vec![
        (
            "transform.linear_scale",
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: Some(MathTransformPreset::LinearScale { slope: f64::NAN, intercept: 0.0 }),
                y_transform: None,
                error_propagation: Default::default(),
                pharmacology_preset: None,
                interchange_xy: false,
                zero_x_substitute: None,
                asymmetric_ci_policy: Default::default(),
            }),
        ),
        (
            "transform.molar",
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: f64::INFINITY }),
                error_propagation: Default::default(),
                pharmacology_preset: None,
                interchange_xy: false,
                zero_x_substitute: None,
                asymmetric_ci_policy: Default::default(),
            }),
        ),
        (
            "transform.normalize",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::FixedValue(f64::NEG_INFINITY),
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: false,
            }),
        ),
        ("transform.simulate.param", sim(vec![(String::from("m"), f64::NAN)], SimulationNoise::None)),
        ("transform.simulate.noise", sim(vec![], SimulationNoise::Gaussian { sd: f64::INFINITY })),
        ("descriptive", AnalysisKind::DescriptiveStats(DescriptiveSpec { custom_upper_percentile: Some(f64::NAN), ..Default::default() })),
        ("outlier.grubbs", AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: f64::NAN, iterative: false })),
        ("outlier.rout", AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: f64::INFINITY })),
        (
            "two_group.one_sample",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: f64::NAN }, ..Default::default() }),
        ),
        ("multi_row.multiplicity", AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { multiplicity: mult(f64::INFINITY), ..Default::default() })),
        ("one_way.multiplicity", AnalysisKind::OneWayAnova(OneWayAnovaSpec { multiplicity: mult(f64::NAN), ..Default::default() })),
        ("factorial.multiplicity", AnalysisKind::FactorialAnova(FactorialAnovaSpec { multiplicity: mult(f64::NEG_INFINITY), ..Default::default() })),
        (
            "nested.multiplicity",
            AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: false, multiplicity: mult(f64::NAN), ..Default::default() }),
        ),
        (
            "contingency.expected",
            AnalysisKind::Contingency(ContingencySpec { expected_proportions: Some(vec![0.5, f64::NAN]), ..Default::default() }),
        ),
        (
            "survival.multiplicity",
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: mult(f64::INFINITY),
                mv_time_column: None,
                mv_status_column: None,
                mv_group_column: None,
                event_code: None,
                censored_code: None,
                treat_other_as: Default::default(),
                ratio_median_ci: false,
                mv_status_columns: Vec::new(),
                mv_group_columns: Vec::new(),
            }),
        ),
        (
            "nonlinear.weighting",
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                weighting: WeightingScheme::GeneralPowerY(f64::NAN),
                ..NonlinearFitSpec::new("four_pl")
            }),
        ),
        (
            "nonlinear.rout",
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                method: FitMethod::RoutOutlierElimination { fdr_q_percent: f64::INFINITY },
                ..NonlinearFitSpec::new("four_pl")
            }),
        ),
        (
            "nonlinear.rule_bound",
            AnalysisKind::NonlinearRegression(NonlinearFitSpec {
                parameter_rules: vec![param_rule("EC50", ParameterScope::Constant(f64::NAN))],
                ..NonlinearFitSpec::new("four_pl")
            }),
        ),
    ];
    for (label, kind) in bad_kinds {
        let mut d = base.clone();
        d.result_sheets[0].analysis.kind = kind;
        assert_save_paths_reject_nonfinite(label, &d, tmp.path());
    }
    Ok(())
}

#[test]
fn r1_s06_13_apply_headers_to_sheet_respects_subcolumns_across_archetypes() -> Result<(), FormatError> {
    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(2), 3, 2)?;
    let mut grp_off = GroupedTable::new(SubcolumnMode::RawReplicates(2), 3, 1)?;
    for i in 0..3 {
        xy_rep.data_sets[i].title = format!("Init{i}");
        grp_off.data_sets[i].title = format!("Keep{i}");
    }

    type CellExpectations<'a> = &'a [(CellCoord, f64)];
    type HeaderCase<'a> = (&'a str, DataSheet, CellCoord, &'a str, &'a [&'a str], CellExpectations<'a>);
    let cases: [HeaderCase<'_>; 5] = [
        (
            "xy_rep2",
            DataSheet::new("xy-rep", "XY Rep", TableArchetype::Xy(xy_rep))?,
            CellCoord::new(0, 0, 0),
            "Ctrl_R1,Ctrl_R2,Treat_R1,Treat_R2\n1,2,3,4\n5,6,7,8\n",
            &["Ctrl_R1", "Treat_R1", "Init2"],
            &[(CellCoord::new(0, 0, 0), 1.0), (CellCoord::new(0, 0, 1), 2.0), (CellCoord::new(0, 1, 0), 3.0), (CellCoord::new(0, 1, 1), 4.0)],
        ),
        (
            "grp_offset",
            DataSheet::new("grp-off", "Grouped2", TableArchetype::Grouped(grp_off))?,
            CellCoord::new(0, 0, 1),
            "Sub1Of0,NewDS1,Sub1Of1,NewDS2\n10,20,30,40\n",
            &["Keep0", "NewDS1", "NewDS2"],
            &[(CellCoord::new(0, 0, 1), 10.0), (CellCoord::new(0, 1, 0), 20.0), (CellCoord::new(0, 1, 1), 30.0), (CellCoord::new(0, 2, 0), 40.0)],
        ),
        (
            "col_mean_sd_n",
            DataSheet::new(
                "col-msd",
                "PreSummarized",
                TableArchetype::Column(ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 2, 1)?),
            )?,
            CellCoord::new(0, 0, 0),
            "GroupA,SD_ignored,N_ignored,GroupB,SD_b,N_b\n10,1.5,4,20,2.5,5\n",
            &["GroupA", "GroupB"],
            &[],
        ),
        (
            "grp_rep3",
            DataSheet::new("grp-1", "Grouped", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 1)?))?,
            CellCoord::new(0, 0, 0),
            "Vehicle_1,Vehicle_2,Vehicle_3,Drug_1,Drug_2,Drug_3\n1,2,3,4,5,6\n",
            &["Vehicle_1", "Drug_1"],
            &[],
        ),
        (
            "xy_mean_sem_n",
            DataSheet::new("xy-1", "XY", TableArchetype::Xy(XyTable::new(SubcolumnMode::MeanSemN, 2, 1)?))?,
            CellCoord::new(0, 0, 0),
            "DoseA,SEM_A,N_A,DoseB,SEM_B,N_B\n10,1,3,20,2,4\n",
            &["DoseA", "DoseB"],
            &[],
        ),
    ];

    for (label, mut sheet, anchor, csv, expected_titles, expected_cells) in cases {
        let spec = make_import_link_spec(PlacementMode::Overwrite, anchor, true);
        apply_csv_import_link(&mut sheet, csv, &spec, true, ImportBaselineAdjustment::None, label, label)?;
        let actual: Vec<&str> = match &sheet.archetype {
            TableArchetype::Xy(t) => t.data_sets.iter().map(|d| d.title.as_str()).collect(),
            TableArchetype::Grouped(t) => t.data_sets.iter().map(|d| d.title.as_str()).collect(),
            TableArchetype::Column(t) => t.data_sets.iter().map(|d| d.title.as_str()).collect(),
            other => panic!("{label}: unexpected archetype {other:?}"),
        };
        assert_eq!(actual, expected_titles, "{label}: titles");
        for &(coord, expected) in expected_cells {
            assert_eq!(sheet.cell(coord).and_then(|c| c.active_number()), Some(expected), "{label}: {coord:?}");
        }
    }
    Ok(())
}
