//! Permanent Round 2 (`R0.19`) regression tests for Slice `S05` (`crates/doc`) and L1 doc contracts
//! backing Slice `S07` (`R2-S05-01`, `R2-S05-03`, `R2-S07-07`, `R2-S07-09`, `R2-S07-10`,
//! `R2-S07-16`, `R2-S07-18`, and `#357` `PointStyleOverride::subcol`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnnotationShapeKind, BreakOrnament, Cell, CellCoord, CloneDataSheetOptions, ColumnDesignHint, ColumnTable, ContingencyMethod,
    ContingencySpec, CorrelationPowerMethod, DataSheet, EquationDisplayMode, ErrorBarSpec, ErrorCode, FramePreset, GraphAnnotation, GraphSpec,
    JsonSchema, KappaWeighting, PointStyleOverride, PowerAnalysisSpec, PowerEffectSpec, PowerOneSidedDirection, PowerSolveTarget, PowerTailMode,
    PowerTestFamily, ProjectDocument, RectPt, SheetId, SizePt, SubcolumnMode, TableArchetype, TransformAnalysisSpec, TwoProportionsMethod, XyTable,
    project_document_schema,
};
use curvebook_palette::{LineDashStyle, MarkerShape, PaletteId, Rgba8};
use serde_json::json;

fn sample_column_sheet(id: &str) -> DataSheet {
    let table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
    DataSheet::new(id, "Observations", TableArchetype::Column(table)).unwrap()
}

#[test]
fn r2_s05_01_set_cell_and_clone_structure_prune_stale_exclusion_notes() {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
    table.data_sets[0].cells[0][0] = Cell::number(10.0);
    table.data_sets[1].cells[1][0] = Cell::number(20.0);
    let mut sheet = DataSheet::new("ds-ex", "Observations", TableArchetype::Column(table)).unwrap();

    let c0 = CellCoord::new(0, 0, 0);
    let c1 = CellCoord::new(1, 1, 0);
    sheet.set_excluded_with_reason(c0, "outlier", "Instrument spike", "2026-11-16T00:00:00Z").unwrap();
    sheet.set_excluded_with_reason(c1, "qc_fail", "Hemolyzed sample", "2026-11-16T00:01:00Z").unwrap();
    assert_eq!(sheet.exclusion_notes.len(), 2);

    // Overwriting an excluded cell with an active cell removes its stale exclusion note.
    sheet.set_cell(c0, Cell::number(11.0)).unwrap();
    assert!(!sheet.archetype.data_cell(c0).unwrap().excluded);
    assert_eq!(sheet.exclusion_note(c0), None);
    assert_eq!(sheet.exclusion_notes.len(), 1);
    assert_eq!(sheet.exclusion_notes[0].coord, c1);

    // Writing an already-excluded cell preserves its exclusion note.
    sheet.set_cell(c1, Cell { value: Cell::number(25.0).value, excluded: true }).unwrap();
    assert_eq!(sheet.exclusion_notes.len(), 1);
    assert_eq!(sheet.exclusion_notes[0].coord, c1);

    // Cloning structure with clear_y_values clears cells (and exclusions), pruning stale exclusion notes.
    let clear_opts = CloneDataSheetOptions {
        title: String::from("Cloned Cleared"),
        keep_x_values: true,
        clear_y_values: true,
        clear_column_titles: false,
        clear_row_titles: false,
        override_replicates: None,
    };
    let cleared_clone = sheet.clone_structure("ds-ex-clone-clear", &clear_opts).unwrap();
    assert!(cleared_clone.exclusion_notes.is_empty(), "cleared clone must not retain orphaned exclusion notes");

    // Cloning XY while retaining Y values preserves active exclusion notes, while clearing Y prunes them.
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(1), 1, 3).unwrap();
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.data_sets[0].cells[0][0] = Cell::number(5.0);
    let mut xy_sheet = DataSheet::new("xy-ex", "XY Exclusions", TableArchetype::Xy(xy)).unwrap();
    let y0 = CellCoord::new(0, 0, 0);
    xy_sheet.set_excluded_with_reason(y0, "y_bad", "Bubble", "2026-11-16T00:03:00Z").unwrap();

    let keep_opts = CloneDataSheetOptions { clear_y_values: false, ..clear_opts };
    let xy_kept = xy_sheet.clone_structure("xy-ex-keep", &keep_opts).unwrap();
    assert!(xy_kept.exclusion_note(y0).is_some(), "retained excluded Y cell keeps its exclusion note");

    let xy_cleared = xy_sheet.clone_structure("xy-ex-clear", &CloneDataSheetOptions { clear_y_values: true, ..keep_opts }).unwrap();
    assert!(xy_cleared.exclusion_note(y0).is_none(), "cleared Y cell must drop its exclusion note");
}

#[test]
fn r2_s05_03_schema_matches_runtime_checkpoints_errors_sheet_id_and_utf8_titles() {
    let schema = project_document_schema();
    assert_eq!(ProjectDocument::json_schema(), schema);
    let val = serde_json::to_value(&schema).unwrap();

    // NamedCheckpoint: created_utc has non-whitespace pattern (no RFC-3339 date-time mismatch with "unix:<secs>"),
    // and id rejects whitespace-only identifiers.
    let cp_props = &val["$defs"]["NamedCheckpoint"]["properties"];
    assert!(cp_props["created_utc"]["format"].is_null(), "created_utc schema must match runtime unix:<secs> strings");
    assert_eq!(cp_props["created_utc"]["minLength"], 1);
    assert_eq!(cp_props["created_utc"]["pattern"], "\\S");
    assert_eq!(cp_props["id"]["minLength"], 1);
    assert_eq!(cp_props["id"]["pattern"], "\\S");

    // SheetId enforces non-whitespace pattern matching SheetId::try_new.
    assert_eq!(val["$defs"]["SheetId"]["minLength"], 1);
    assert_eq!(val["$defs"]["SheetId"]["pattern"], "\\S");

    // ResultStatus::Failed accepts canonical $defs.ErrorCode and legacy error code strings,
    // while keeping $defs.ErrorCode.enum strictly aligned with ErrorCode::ALL.
    assert_eq!(val["$defs"]["ErrorCode"]["enum"].as_array().unwrap().len(), ErrorCode::ALL.len());
    let variants = val["$defs"]["ResultStatus"]["oneOf"].as_array().unwrap();
    let failed_variant = variants.iter().find(|v| v["properties"]["Failed"].is_object()).expect("Failed variant schema");
    let code_schema = &failed_variant["properties"]["Failed"]["properties"]["code"];
    let any_of = code_schema["anyOf"].as_array().expect("ResultStatus::Failed.code anyOf");
    assert!(any_of.iter().any(|item| item["$ref"] == "#/$defs/ErrorCode"));
    assert!(any_of.iter().any(|item| item["type"] == "string"));

    // UTF-8 byte ceiling description on titles matches runtime MAX_TITLE_BYTES semantics.
    let proj_title = &val["properties"]["title"];
    assert_eq!(proj_title["maxLength"], curvebook_doc::MAX_TITLE_BYTES);
    let desc = proj_title["description"].as_str().unwrap_or_default();
    assert!(desc.contains("UTF-8") || desc.contains("bytes"), "expected UTF-8 byte ceiling note in description: {desc}");
}

#[test]
fn r2_s07_07_parts_of_whole_rejects_kappa_and_other_contingency_methods() {
    for method in [
        ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
        ContingencyMethod::FleissKappa,
        ContingencyMethod::FisherExact,
        ContingencyMethod::BarnardExact,
        ContingencyMethod::LikelihoodRatioG2,
        ContingencyMethod::CochranArmitageTrend,
        ContingencyMethod::McNemarPaired,
    ] {
        let kind = AnalysisKind::Contingency(ContingencySpec { method, ..ContingencySpec::default() });
        assert!(!kind.is_compatible_with(curvebook_doc::TableKind::PartsOfWhole), "{method:?} must be incompatible with PartsOfWhole");
    }

    for allowed in [ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ContingencyMethod::ExactBinomial] {
        let kind = AnalysisKind::Contingency(ContingencySpec { method: allowed, ..ContingencySpec::default() });
        assert!(kind.is_compatible_with(curvebook_doc::TableKind::PartsOfWhole), "{allowed:?} must remain compatible with PartsOfWhole");
    }
}

#[test]
fn r2_s07_09_apply_style_from_selective_respects_individual_toggles() {
    let ds_target = sample_column_sheet("ds-target");
    let ds_donor = sample_column_sheet("ds-donor");

    let base_target = || {
        let mut g = GraphSpec::default_for_data_sheet(&ds_target);
        g.palette = PaletteId::OkabeIto;
        g.frame_preset = FramePreset::OpenL;
        g.x_axis.stroke_width_pt = 1.0;
        g.x_axis.break_ornament = BreakOrnament::DoubleSlash;
        g.legend.bordered = false;
        g.legend.visible = true;
        g.series[0].y_error_bars = Some(ErrorBarSpec { stroke_width_pt: 0.75, cap_width_pt: 3.0, ..ErrorBarSpec::default() });
        g
    };

    let mut donor = GraphSpec::default_for_data_sheet(&ds_donor);
    donor.palette = PaletteId::PaulTolVibrant;
    donor.paper_fill = Some(Rgba8::rgba(240, 240, 240, 255));
    donor.canvas_size_pt = SizePt::new(320.0, 240.0);
    donor.frame_preset = FramePreset::PlainBoxed;
    donor.x_axis.stroke_width_pt = 2.5;
    donor.x_axis.break_ornament = BreakOrnament::Zigzag;
    donor.legend.bordered = true;
    donor.legend.visible = false;
    donor.series[0].y_error_bars =
        Some(ErrorBarSpec { dash_style: LineDashStyle::Dashed, stroke_width_pt: 2.25, cap_width_pt: 6.5, ..ErrorBarSpec::default() });

    // 1. Only palette enabled: frame/axes & error/legend remain unchanged.
    let mut only_palette = base_target();
    only_palette.apply_style_from_selective(&donor, true, false, false);
    assert_eq!(only_palette.palette, PaletteId::PaulTolVibrant);
    assert_eq!(only_palette.frame_preset, FramePreset::OpenL);
    assert!((only_palette.x_axis.stroke_width_pt - 1.0).abs() < 1e-12);
    assert!(!only_palette.legend.bordered);
    assert!((only_palette.series[0].y_error_bars.unwrap().cap_width_pt - 3.0).abs() < 1e-12);

    // 2. Only frame/axes enabled: palette & error/legend remain unchanged.
    let mut only_frame = base_target();
    only_frame.apply_style_from_selective(&donor, false, true, false);
    assert_eq!(only_frame.palette, PaletteId::OkabeIto);
    assert_eq!(only_frame.frame_preset, FramePreset::PlainBoxed);
    assert_eq!(only_frame.canvas_size_pt, SizePt::new(320.0, 240.0));
    assert!((only_frame.x_axis.stroke_width_pt - 2.5).abs() < 1e-12);
    assert_eq!(only_frame.x_axis.break_ornament, BreakOrnament::Zigzag);
    assert!(!only_frame.legend.bordered);

    // 3. Only error bars / legend enabled: palette & frame/axes remain unchanged.
    let mut only_eb_legend = base_target();
    only_eb_legend.apply_style_from_selective(&donor, false, false, true);
    assert_eq!(only_eb_legend.palette, PaletteId::OkabeIto);
    assert_eq!(only_eb_legend.frame_preset, FramePreset::OpenL);
    assert!(only_eb_legend.legend.bordered);
    assert!(!only_eb_legend.legend.visible);
    let eb = only_eb_legend.series[0].y_error_bars.unwrap();
    assert_eq!(eb.dash_style, LineDashStyle::Dashed);
    assert!((eb.cap_width_pt - 6.5).abs() < 1e-12);
}

#[test]
fn r2_s07_10_power_analysis_spec_serde_validation_and_schema() {
    let spec = PowerAnalysisSpec {
        family: PowerTestFamily::TwoProportions,
        solve_target: PowerSolveTarget::SampleSize,
        tail_mode: PowerTailMode::OneSided(PowerOneSidedDirection::Greater),
        alpha: 0.025,
        target_power: Some(0.90),
        sample_size: None,
        sample_size_group2: None,
        allocation_ratio: 2.0,
        effect: PowerEffectSpec::TwoProportions { p1: 0.30, p2: 0.55 },
        num_groups: None,
        chi_square_df: 1.0,
        two_proportions_mode: TwoProportionsMethod::PooledNullUnpooledAlt,
        continuity_correction: true,
        correlation_method: CorrelationPowerMethod::FisherZ,
        info_sheet_id: Some(SheetId::new("info-power")),
    };
    spec.validate().expect("valid two-proportions PowerAnalysisSpec");

    let encoded = serde_json::to_value(&spec).unwrap();
    let decoded: PowerAnalysisSpec = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, spec);

    // One-sided rejected for F / Chi-square families.
    let bad_tail = PowerAnalysisSpec {
        family: PowerTestFamily::OneWayAnova,
        tail_mode: PowerTailMode::OneSided(PowerOneSidedDirection::Less),
        num_groups: Some(3),
        ..PowerAnalysisSpec::default()
    };
    assert!(bad_tail.validate().is_err());

    // Invalid alpha & probability bounds rejected.
    let bad_alpha = PowerAnalysisSpec { alpha: 1.0, ..PowerAnalysisSpec::default() };
    assert!(bad_alpha.validate().is_err());
    let bad_prop = PowerAnalysisSpec { effect: PowerEffectSpec::TwoProportions { p1: -0.1, p2: 0.5 }, ..spec };
    assert!(bad_prop.validate().is_err());

    let schema_val = serde_json::to_value(PowerAnalysisSpec::json_schema()).unwrap();
    assert_eq!(schema_val["title"], "PowerAnalysisSpec");
    assert!(schema_val["properties"]["family"].is_object());
    assert!(schema_val["properties"]["solve_target"].is_object());

    // First-class AnalysisKind::PowerAnalysis wiring (`R2-S07-10`).
    let mut wired = decoded;
    wired.info_sheet_id = Some(SheetId::new("info-pwr"));
    let kind = AnalysisKind::PowerAnalysis(wired);
    assert!(kind.derive_auto_title().contains("Two proportions"));
    assert!(kind.is_compatible_with(curvebook_doc::TableKind::Xy));
    assert_eq!(kind.upstream_sheet_ids(), vec![SheetId::new("info-pwr")]);
    let kind_json = serde_json::to_value(&kind).unwrap();
    let kind_back: AnalysisKind = serde_json::from_value(kind_json).unwrap();
    assert_eq!(kind_back, kind);
}

#[test]
fn r2_s07_16_graph_spec_validate_rejects_invalid_model_equation_and_point_overrides() {
    let ds = sample_column_sheet("ds-gval");
    let mut base = GraphSpec::default_for_data_sheet(&ds);
    base.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-fit"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: RectPt::new(10.0, 10.0, 0.0, 0.0), // (0, 0) auto-size sentinel remains valid
        font_size_pt: 10.0,
        bordered: false,
    });
    assert!(base.validate().is_ok(), "auto-sized ModelEquation (w=0, h=0) with valid font size must pass");

    let mut bad_font_zero = base.clone();
    bad_font_zero.annotations[0] = GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-fit"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: RectPt::new(10.0, 10.0, 120.0, 40.0),
        font_size_pt: 0.0,
        bordered: false,
    };
    assert!(bad_font_zero.validate().is_err(), "zero ModelEquation font_size_pt must fail");

    let mut bad_font_huge = base.clone();
    if let GraphAnnotation::ModelEquation { font_size_pt, .. } = &mut bad_font_huge.annotations[0] {
        *font_size_pt = 200.0;
    }
    assert!(bad_font_huge.validate().is_err(), ">144pt ModelEquation font_size_pt must fail");

    let mut bad_neg_bounds = base.clone();
    bad_neg_bounds.annotations.push(GraphAnnotation::Shape {
        kind: AnnotationShapeKind::Rectangle,
        bounds_pt: RectPt { x: 0.0, y: 0.0, width: -5.0, height: 10.0 },
        stroke_color: Rgba8::BLACK,
        stroke_width_pt: 1.0,
        dash: LineDashStyle::Solid,
        fill_color: None,
    });
    assert!(bad_neg_bounds.validate().is_err(), "negative shape width must fail");

    let mut bad_pt_size = base.clone();
    bad_pt_size.point_overrides.push(PointStyleOverride { size_pt: Some(0.0), ..PointStyleOverride::default() });
    assert!(bad_pt_size.validate().is_err(), "non-positive PointStyleOverride::size_pt must fail");

    let mut bad_pt_border = base;
    bad_pt_border.point_overrides.push(PointStyleOverride { border_width_pt: Some(-0.5), ..PointStyleOverride::default() });
    assert!(bad_pt_border.validate().is_err(), "negative PointStyleOverride::border_width_pt must fail");
}

#[test]
fn r2_s07_18_random_sample_proportional_allocation_defaults_to_true() {
    let decoded: TransformAnalysisSpec = serde_json::from_value(json!({
        "RandomSample": { "seed": 42, "sample_size": 10 }
    }))
    .unwrap();
    match decoded {
        TransformAnalysisSpec::RandomSample { proportional_allocation, .. } => {
            assert!(proportional_allocation, "proportional_allocation must default to true when omitted");
        }
        other => panic!("expected RandomSample, got {other:?}"),
    }
}

#[test]
fn coord_357_point_style_override_subcol_lookup_and_serde() {
    let ds = sample_column_sheet("ds-subcol");
    let mut spec = GraphSpec::default_for_data_sheet(&ds);
    spec.point_overrides.push(PointStyleOverride {
        series_index: 0,
        point_index: 1,
        subcol: None,
        symbol: Some(MarkerShape::Circle),
        ..PointStyleOverride::default()
    });
    spec.point_overrides.push(PointStyleOverride {
        series_index: 0,
        point_index: 1,
        subcol: Some(2),
        symbol: Some(MarkerShape::Diamond),
        ..PointStyleOverride::default()
    });

    assert_eq!(spec.point_override_with_subcol(0, 1, Some(2)).and_then(|o| o.symbol), Some(MarkerShape::Diamond));
    assert_eq!(spec.point_override_with_subcol(0, 1, Some(0)).and_then(|o| o.symbol), Some(MarkerShape::Circle));

    let whole_json = serde_json::to_value(&spec.point_overrides[0]).unwrap();
    assert!(whole_json.get("subcol").is_none(), "subcol: None should be omitted on serialization");

    let rep_json = serde_json::to_value(&spec.point_overrides[1]).unwrap();
    assert_eq!(rep_json["subcol"], 2);
}
