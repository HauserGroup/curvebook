use curvebook_doc::{
    AdditionalTick, AxisSegment, AxisSpec, AxisThreshold, Cell, CellValue, ColorChannelMapping, DataSheet, DocError, ErrorBarDirection,
    ErrorBarMetric, ErrorBarSpec, ExplicitErrorBounds, FloatingNote, GraphAnnotation, GraphSheet, GraphicalDataObject, GroupedTable,
    JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MultivariateGraphEncoding, MultivariatePlotMode, PairwiseBracketSpec,
    PointStyleOverride, RectPt, SeriesSpec, SheetId, SizeChannelMapping, SizePt, SubcolumnMode, SummarizedPoint, SummaryMeasure, SummaryPointError,
    TableArchetype,
};
use curvebook_palette::{ColorRamp, ColorStop, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};

#[test]
fn r1_s05_07_reconstruct_rejects_impossible_moments_and_overflows() -> Result<(), SummaryPointError> {
    let r1 = SummarizedPoint::reconstruct(SubcolumnMode::MeanSdN, &[Cell::number(10.0), Cell::number(1e155), Cell::number(3.0)]);
    assert!(r1.is_err(), "overflowing ss_within must return Err");

    let r2 = SummarizedPoint::reconstruct(SubcolumnMode::MeanSemN, &[Cell::number(10.0), Cell::number(1e200), Cell::number(1e250)]);
    assert!(r2.is_err(), "overflowing sd in MeanSemN must return Err");

    let r2b = SummarizedPoint::reconstruct(SubcolumnMode::MeanSdN, &[Cell::number(10.0), Cell::number(1e200), Cell::number(1.0)]);
    assert!(r2b.is_err(), "overflowing sd^2 at N=1 must return Err instead of silently evaluating 0*inf");

    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let frac = SummarizedPoint::reconstruct(mode, &[Cell::number(10.0), Cell::number(2.0), Cell::number(2.5)]);
        assert!(matches!(frac, Err(SummaryPointError::SampleSizeBelowOne { n }) if (n - 2.5).abs() < 1e-12));
    }

    let zero_cv_nz = SummarizedPoint::reconstruct(SubcolumnMode::MeanCvN, &[Cell::number(0.0), Cell::number(20.0), Cell::number(5.0)]);
    assert_eq!(zero_cv_nz, Err(SummaryPointError::MissingSpread));
    let zero_cv_z = SummarizedPoint::reconstruct(SubcolumnMode::MeanCvN, &[Cell::number(0.0), Cell::number(0.0), Cell::number(5.0)]);
    assert_eq!(zero_cv_z, Err(SummaryPointError::MissingSpread));

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let c = Cell { value: CellValue::Number(bad), excluded: false };
        assert!(!matches!(SummarizedPoint::reconstruct(SubcolumnMode::MeanSdN, &[c.clone(), Cell::number(1.0), Cell::number(3.0)]), Ok(Some(_))));
        assert!(SummarizedPoint::reconstruct(SubcolumnMode::MeanSdN, &[Cell::number(1.0), c.clone(), Cell::number(3.0)]).is_err());
        assert!(SummarizedPoint::reconstruct(SubcolumnMode::MeanSdN, &[Cell::number(1.0), Cell::number(1.0), c]).is_err());
    }

    let valid_n1 = SummarizedPoint::reconstruct(SubcolumnMode::MeanSdN, &[Cell::number(10.0), Cell::number(5.0), Cell::number(1.0)])?
        .ok_or(SummaryPointError::MissingSpread)?;
    assert_eq!(valid_n1.df_within, 0.0);
    assert_eq!(valid_n1.ss_within, 0.0);

    let valid_cv = SummarizedPoint::reconstruct(SubcolumnMode::MeanCvN, &[Cell::number(10.0), Cell::number(20.0), Cell::number(5.0)])?
        .ok_or(SummaryPointError::MissingSpread)?;
    assert!((valid_cv.sd - 2.0).abs() < 1e-12);
    assert!((valid_cv.df_within - 4.0).abs() < 1e-12);
    assert!((valid_cv.ss_within - 16.0).abs() < 1e-12);
    Ok(())
}

#[test]
fn r1_s05_07_explicit_bounds_rejects_zero_mean_cv_and_overflow() {
    assert_eq!(SubcolumnMode::MeanCvN.explicit_bounds(&[Cell::number(0.0), Cell::number(20.0), Cell::number(5.0)]), None);
    assert_eq!(
        SubcolumnMode::MeanCvN.explicit_bounds(&[Cell::number(10.0), Cell::number(20.0), Cell::number(5.0)]),
        Some(ExplicitErrorBounds { center: 10.0, lower: 8.0, upper: 12.0 })
    );
    assert_eq!(SubcolumnMode::MeanSdN.explicit_bounds(&[Cell::number(f64::MAX), Cell::number(f64::MAX), Cell::number(3.0)]), None);
    assert_eq!(SubcolumnMode::MeanLowHigh.explicit_bounds(&[Cell::number(f64::MAX), Cell::number(1.0), Cell::number(f64::MAX)]), None);
}

fn sample_marker() -> MarkerStyle {
    MarkerStyle {
        shape: MarkerShape::Circle,
        size_pt: 5.0,
        stroke_width_pt: 1.0,
        fill_color: Rgba8::BLACK,
        stroke_color: Rgba8::BLACK,
        fill_mode: MarkerFillMode::Filled,
    }
}

fn sample_error_bars() -> ErrorBarSpec {
    ErrorBarSpec {
        metric: ErrorBarMetric::Sd,
        direction: ErrorBarDirection::Both,
        dash_style: LineDashStyle::Solid,
        stroke_width_pt: 1.0,
        cap_width_pt: 4.0,
        color: None,
    }
}

fn sample_data_sheet() -> Result<DataSheet, DocError> {
    let tbl = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2)?;
    DataSheet::new("data-1", "Data", TableArchetype::Grouped(tbl))
}

#[test]
fn r1_s06_06_axis_graph_and_layout_validate_reject_nonfinite_floats() -> Result<(), DocError> {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut ax = AxisSpec::linear("X");
        ax.gap_width_pt = bad;
        assert!(matches!(ax.validate(), Err(DocError::InvalidCellValue { .. })));

        let mut ax = AxisSpec::linear("X");
        ax.tick_angle_deg = bad;
        assert!(ax.validate().is_err());

        let mut ax = AxisSpec::linear("X");
        ax.stroke_width_pt = bad;
        assert!(ax.validate().is_err());

        let mut ax = AxisSpec::linear("X");
        ax.segments[0] = AxisSegment { min: Some(bad), ..AxisSegment::default() };
        assert!(ax.validate().is_err());

        let mut ax = AxisSpec::linear("X");
        ax.segments[0].major_tick_interval = Some(bad);
        assert!(ax.validate().is_err());

        let mut ax = AxisSpec::linear("X");
        ax.thresholds.push(AxisThreshold {
            value: bad,
            shade_to: Some(1.0),
            color: Rgba8::BLACK,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.0,
            label: None,
        });
        assert!(ax.validate().is_err());

        let mut ax = AxisSpec::linear("X");
        ax.additional_ticks.push(AdditionalTick { value: bad, label: None, draw_gridline: false });
        assert!(ax.validate().is_err());
    }

    let ds = sample_data_sheet()?;
    let base_graph = GraphSheet::new_uninitialized_for_data("g1", &ds)?;
    assert!(base_graph.validate().is_ok());

    let mut g = base_graph.clone();
    g.spec.canvas_size_pt = SizePt { width: f64::NAN, height: 100.0 };
    assert!(g.spec.validate().is_err());
    assert!(g.validate().is_err());

    let mut g = base_graph.clone();
    g.spec.series.push(SeriesSpec {
        source_sheet_id: SheetId::new("data-1"),
        data_set_index: 0,
        legend_label: None,
        bind_to_right_y2: false,
        gdo: GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: Rgba8::BLACK,
            border_color: Rgba8::BLACK,
            border_width_pt: f64::INFINITY,
            replicate_overlay: None,
        },
        y_error_bars: Some(sample_error_bars()),
        x_error_bars: None,
        regression_overlay: None,
    });
    assert!(g.spec.validate().is_err());

    let mut g = base_graph.clone();
    g.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::CategoricalSummary {
            gdo: GraphicalDataObject::AlignedDotPlot { summary: SummaryMeasure::Mean, marker: sample_marker(), show_crossbar: true },
            y_error_bars: None,
        },
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Continuous {
            column: 2,
            ramp: ColorRamp::Custom { stops: vec![ColorStop { position: f64::NAN, color: Rgba8::BLACK }] },
            domain_clamp: None,
        },
        size: SizeChannelMapping::Constant { radius_pt: 4.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    assert!(g.spec.validate().is_err());

    let mut g = base_graph.clone();
    g.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("r1"),
        group_a: 0,
        group_b: 1,
        style: Default::default(),
        label_mode: Default::default(),
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 10.0,
        bracket_step_pt: f64::NEG_INFINITY,
    });
    assert!(g.spec.validate().is_err());

    let mut g = base_graph.clone();
    g.spec.annotations.push(GraphAnnotation::LineOrArrow {
        start_pt: (0.0, 0.0),
        end_pt: (f64::NAN, 10.0),
        arrowhead: true,
        dash: LineDashStyle::Solid,
        color: Rgba8::BLACK,
    });
    assert!(g.spec.validate().is_err());

    let mut g = base_graph.clone();
    g.spec.point_overrides.push(PointStyleOverride {
        series_index: 0,
        point_index: 0,
        color: None,
        symbol: None,
        size_pt: Some(f64::INFINITY),
        border_width_pt: None,
        fill_color: None,
        fill_mode: None,
        hidden: false,
        ..PointStyleOverride::default()
    });
    assert!(g.spec.validate().is_err());

    let mut g = base_graph.clone();
    g.spec.legend.position_pt = Some((10.0, f64::NAN));
    assert!(g.spec.validate().is_err());

    let mut g = base_graph;
    g.floating_notes.push(FloatingNote::new("n1", RectPt { x: f64::NAN, y: 0.0, width: 10.0, height: 10.0 }, "bad note"));
    assert!(g.validate().is_err());

    let mut bad_lspec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    bad_lspec.width = f64::NAN;
    assert!(bad_lspec.validate().is_err());
    assert!(LayoutSheet::new("l1", "L1", bad_lspec).is_err());

    let mut bad_item_spec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    bad_item_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("p1"),
        graph_sheet_id: SheetId::new("g1"),
        bounds_pt: RectPt::new(0.0, 0.0, 100.0, 100.0),
        scale_percent: f64::INFINITY,
        badge_override: None,
    });
    assert!(LayoutSheet::new("l1", "L1", bad_item_spec).is_err());

    let mut lsheet = LayoutSheet::new("l1", "L1", LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm))?;
    lsheet.floating_notes.push(FloatingNote::new("ln1", RectPt { x: 0.0, y: f64::NEG_INFINITY, width: 20.0, height: 20.0 }, "note"));
    assert!(lsheet.validate().is_err());
    Ok(())
}
