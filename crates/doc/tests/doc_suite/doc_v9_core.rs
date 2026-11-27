//! Schema v9 core invariants, display metadata, selection existence checks, result provenance,
//! `SubcolumnMode::MeanSd` / `MeanSem`, `SheetOrigin`, and §10.2 consolidated sheet-ID remapping.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, AnalysisKindId, AnalysisRecomputeStrategy, AnalysisSpec, BracketLabelMode, BracketStyle, CURRENT_SCHEMA_VERSION, Cell, CellCoord,
    ColumnDesignHint, ColumnTable, CompactLetterDisplaySpec, DataSetColumn, DataSheet, DescriptiveSpec, DocError, EmbeddedTableStyle,
    EquationDisplayMode, FloatingNote, GraphAnnotation, GraphSheet, GraphSpec, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec,
    ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutItem, LayoutSheet, LayoutSpec, LinkSource, LinkedDataRegion, MAX_DISPLAY_DECIMALS,
    MAX_TITLE_BYTES, MultipleVariablesTable, NonlinearFitSpec, NormalizeAnchor, NoteId, PairwiseBracketSpec, ParameterRule, ParameterScope,
    PowerAnalysisSpec, ProjectDocument, RectPt, RegressionOverlaySpec, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SheetCellRange, SheetId, SheetOrigin, SourceCellId, SubcolumnMode, SummarizedPoint, SummaryPointError, TableArchetype, TableKind,
    TransformAnalysisSpec, VariableColumn, XColumnMode, XyTable, remap_document_sheet_ids,
};
use curvebook_palette::Rgba8;

fn make_note(id: &str, body: &str) -> FloatingNote {
    FloatingNote {
        id: NoteId::new(id),
        fill_color: Rgba8::rgba(255, 248, 200, 255),
        bounds_pt: RectPt::new(10.0, 10.0, 120.0, 60.0),
        minimized: false,
        body_markdown: body.to_owned(),
    }
}

#[test]
fn v9_display_metadata_roundtrip_and_validation() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 9);

    let mut xy = XyTable::new(SubcolumnMode::MeanSd, 1, 2).unwrap();
    xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSem };
    xy.x_cells = vec![vec![Cell::number(1.0), Cell::number(0.1)], vec![Cell::number(2.0), Cell::number(0.2)]];
    xy.x_subcolumn_titles = vec![String::from("X Mean"), String::from("X SEM")];
    xy.x_display_decimals = Some(3);
    xy.data_sets[0].subcolumn_titles = vec![String::from("Mean"), String::from("SD")];
    xy.data_sets[0].display_decimals = Some(MAX_DISPLAY_DECIMALS);

    let ds = DataSheet::new("xy-meta", "XY With Metadata", TableArchetype::Xy(xy)).unwrap();
    let rt: DataSheet = serde_json::from_str(&serde_json::to_string(&ds).unwrap()).unwrap();
    assert_eq!(rt, ds);

    // Invalid subcolumn_titles length rejected by validate() and serde.
    let mut bad_titles = ds.clone();
    if let TableArchetype::Xy(ref mut t) = bad_titles.archetype {
        t.data_sets[0].subcolumn_titles = vec![String::from("OnlyOne")];
    }
    assert!(matches!(bad_titles.validate(), Err(DocError::InvalidCellValue { .. })));
    assert!(serde_json::from_str::<DataSheet>(&serde_json::to_string(&bad_titles).unwrap()).is_err());

    // Invalid x_subcolumn_titles length rejected.
    let mut bad_x_titles = ds.clone();
    if let TableArchetype::Xy(ref mut t) = bad_x_titles.archetype {
        t.x_subcolumn_titles = vec![String::from("OnlyOne")];
    }
    assert!(matches!(bad_x_titles.validate(), Err(DocError::InvalidCellValue { .. })));

    // Oversized title bytes rejected.
    let mut huge_title = ds.clone();
    if let TableArchetype::Xy(ref mut t) = huge_title.archetype {
        t.data_sets[0].subcolumn_titles = vec!["a".repeat(MAX_TITLE_BYTES + 1), String::from("SD")];
    }
    assert!(matches!(huge_title.validate(), Err(DocError::TitleTooLong { .. })));

    // Display decimals > 15 rejected on DataSetColumn, XyTable.x_display_decimals, and VariableColumn.
    let mut bad_y_dec = ds.clone();
    if let TableArchetype::Xy(ref mut t) = bad_y_dec.archetype {
        t.data_sets[0].display_decimals = Some(MAX_DISPLAY_DECIMALS + 1);
    }
    assert!(matches!(bad_y_dec.validate(), Err(DocError::InvalidCellValue { .. })));
    assert!(serde_json::from_str::<DataSheet>(&serde_json::to_string(&bad_y_dec).unwrap()).is_err());

    let mut bad_x_dec = ds.clone();
    if let TableArchetype::Xy(ref mut t) = bad_x_dec.archetype {
        t.x_display_decimals = Some(16);
    }
    assert!(matches!(bad_x_dec.validate(), Err(DocError::InvalidCellValue { .. })));

    let mut mv = MultipleVariablesTable::new(1, 2).unwrap();
    mv.variables[0].display_decimals = Some(4);
    let mv_ok = DataSheet::new("mv-meta", "MV", TableArchetype::MultipleVariables(mv.clone())).unwrap();
    assert_eq!(serde_json::from_str::<DataSheet>(&serde_json::to_string(&mv_ok).unwrap()).unwrap(), mv_ok);

    mv.variables[0].display_decimals = Some(16);
    assert!(matches!(DataSheet::new("mv-bad", "MV Bad", TableArchetype::MultipleVariables(mv)), Err(DocError::InvalidCellValue { .. })));
    let _ = (DataSetColumn::new("T", 0, 1).unwrap(), VariableColumn::new("V", 0).unwrap());
}

#[test]
fn v9_active_sheet_and_family_anchor_existence_validation_and_clearing() {
    let mut doc = ProjectDocument::new("Selection Checks").unwrap();
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    doc.add_data_sheet(DataSheet::new("d1", "Table 1", TableArchetype::Column(col)).unwrap()).unwrap();
    assert!(doc.validate_selection_ids().is_ok());

    doc.active_sheet_id = Some(SheetId::new("ghost-active"));
    assert_eq!(doc.validate_selection_ids(), Err(DocError::SheetNotFound { sheet_id: SheetId::new("ghost-active") }));

    doc.active_sheet_id = Some(SheetId::new("d1"));
    doc.family_anchor_sheet_id = Some(SheetId::new("ghost-anchor"));
    assert_eq!(doc.validate_selection_ids(), Err(DocError::SheetNotFound { sheet_id: SheetId::new("ghost-anchor") }));

    doc.active_sheet_id = Some(SheetId::new("ghost-active"));
    let warnings = doc.clear_dangling_selection_ids();
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].contains("active_sheet_id") && warnings[0].contains("ghost-active"));
    assert!(warnings[1].contains("family_anchor_sheet_id") && warnings[1].contains("ghost-anchor"));
    assert_eq!(doc.active_sheet_id, None);
    assert_eq!(doc.family_anchor_sheet_id, None);
    assert!(doc.validate_selection_ids().is_ok());
}

#[test]
fn v9_result_provenance_d1_invariants_and_semantic_helpers() {
    let desc = AnalysisKindId::ImportedSnapshot.descriptor();
    assert!(!desc.user_creatable);
    assert_eq!(desc.recompute_strategy, AnalysisRecomputeStrategy::PreserveSnapshot);

    // Linked imported snapshot validates cleanly.
    let linked_spec = AnalysisSpec::new_imported_snapshot("imp-1", Some(SheetId::new("d1")), "Imported Regression").unwrap();
    let mut imported = ResultSheet::new_imported(linked_spec, "external_xml", "Nonlinear fit", "Cached values from source file").unwrap();
    imported.scalars.push(ResultScalarEntry { key: String::from("IC50"), data_set_index: Some(0), value: 42.0, units: Some(String::from("nM")) });
    assert!(imported.is_imported());
    assert!(imported.has_displayable_output());
    assert!(!imported.is_computed_up_to_date());
    assert_eq!(imported.lookup_scalar("IC50", Some(0)), None, "imported scalars must never leak into hooks");

    // ImportedStale remains displayable, imported, non-computed, and blocks hooks.
    imported.status = ResultStatus::ImportedStale;
    assert!(imported.validate().is_ok());
    assert!(imported.is_imported());
    assert!(imported.has_displayable_output());
    assert!(!imported.is_computed_up_to_date());
    assert_eq!(imported.lookup_scalar("IC50", Some(0)), None);

    // Unlinked imported snapshot (source_sheet_id: None) is allowed strictly for ImportedSnapshot.
    let unlinked_spec = AnalysisSpec::new_imported_snapshot("imp-unlinked", None, "Orphan Table").unwrap();
    let unlinked = ResultSheet::new_imported(unlinked_spec, "external_archive", "Summary", "Source table omitted").unwrap();
    assert!(unlinked.validate().is_ok());

    // Non-ImportedSnapshot rejects source_sheet_id: None (via validate_structural and serde).
    let mut bad_computed_spec = AnalysisSpec::new("bad-desc", "d1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    bad_computed_spec.source_sheet_id = None;
    assert!(bad_computed_spec.validate_structural().is_err(), "computed analyses must require a source_sheet_id");
    assert!(serde_json::from_str::<AnalysisSpec>(&serde_json::to_string(&bad_computed_spec).unwrap()).is_err());

    // ResultSheet::new refuses ImportedSnapshot.
    let snap_spec = AnalysisSpec::new_imported_snapshot("snap-x", None, "Snap").unwrap();
    assert!(ResultSheet::new(snap_spec).is_err());

    // D1 invariant 1: Imported provenance cannot be UpToDate.
    let mut bad_status = imported.clone();
    bad_status.status = ResultStatus::UpToDate;
    assert!(bad_status.validate().is_err());
    assert!(serde_json::from_str::<ResultSheet>(&serde_json::to_string(&bad_status).unwrap()).is_err());

    // D1 invariant 2: Imported provenance forbids analyzable_data.
    let mut bad_analyzable = imported.clone();
    bad_analyzable.status = ResultStatus::Imported;
    let derived =
        DataSheet::new("imp-1-data", "Derived", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap()))
            .unwrap();
    bad_analyzable.analyzable_data = Some(derived);
    assert!(bad_analyzable.validate().is_err());
    assert!(serde_json::from_str::<ResultSheet>(&serde_json::to_string(&bad_analyzable).unwrap()).is_err());

    // D1 invariant 3: Computed provenance cannot hold Imported / ImportedStale status.
    let comp_spec = AnalysisSpec::new("comp-1", "d1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let mut comp = ResultSheet::new(comp_spec).unwrap();
    comp.status = ResultStatus::UpToDate;
    comp.scalars.push(ResultScalarEntry { key: String::from("Mean"), data_set_index: Some(0), value: 10.0, units: None });
    assert!(comp.is_computed_up_to_date());
    assert!(comp.has_displayable_output());
    assert!(!comp.is_imported());
    assert_eq!(comp.lookup_scalar("Mean", Some(0)), Some(10.0));

    comp.status = ResultStatus::Imported;
    assert!(comp.validate().is_err());
    comp.status = ResultStatus::ImportedStale;
    assert!(comp.validate().is_err());
}

#[test]
fn v9_mean_sd_and_mean_sem_subcolumn_modes_and_column_guard() {
    for mode in [SubcolumnMode::MeanSd, SubcolumnMode::MeanSem] {
        assert_eq!(mode.subcolumn_count(), 2);
        assert!(mode.is_pre_summarized());
        assert!(!mode.supports_parametric_moments());
        assert!(TableKind::Xy.allows_subcolumn_mode(mode));
        assert!(TableKind::Column.allows_subcolumn_mode(mode));
        assert!(TableKind::Grouped.allows_subcolumn_mode(mode));
        assert!(!TableKind::Contingency.allows_subcolumn_mode(mode));

        let bounds = mode.explicit_bounds(&[Cell::number(10.0), Cell::number(2.5)]).unwrap();
        assert_eq!(bounds.center, 10.0);
        assert_eq!(bounds.lower, 7.5);
        assert_eq!(bounds.upper, 12.5);

        assert!(mode.explicit_bounds(&[Cell::number(10.0), Cell::number(-1.0)]).is_none());

        assert!(matches!(
            SummarizedPoint::reconstruct(mode, &[Cell::number(10.0), Cell::number(2.0)]),
            Err(SummaryPointError::NotParametricSummaryMode { .. })
        ));
    }

    // Column supports MeanLowHigh, UpperLowerLimits, and MedianIqr alongside MeanSd/MeanSem and parametric modes.
    for interval_mode in [SubcolumnMode::MeanLowHigh, SubcolumnMode::UpperLowerLimits, SubcolumnMode::MedianIqr] {
        assert!(TableKind::Column.allows_subcolumn_mode(interval_mode), "Column must allow {interval_mode:?}");
        let col_i = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, interval_mode, 2, 2).unwrap();
        assert_eq!(col_i.subcolumn_mode.subcolumn_count(), 3);
        assert!(col_i.validate().is_ok());
    }

    let col_sd = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSd, 2, 3).unwrap();
    assert_eq!(col_sd.subcolumn_mode.subcolumn_count(), 2);
    let grp_sem = GroupedTable::new(SubcolumnMode::MeanSem, 2, 3).unwrap();
    assert_eq!(grp_sem.subcolumn_mode.subcolumn_count(), 2);
}

#[test]
fn v9_sheet_origin_validation_and_roundtrip_on_all_five_sheets() {
    let valid = SheetOrigin::new("plate_run_01.curvebook", "curvebook", "unix:1795700000").unwrap();
    assert!(SheetOrigin::new("dir/file.curvebook", "curvebook", "").is_err());
    assert!(SheetOrigin::new(r"C:\file.curvebook", "curvebook", "").is_err());
    assert!(SheetOrigin::new("bad\0name", "curvebook", "").is_err());
    assert!(SheetOrigin::new("   ", "curvebook", "").is_err());
    assert!(SheetOrigin::new("a".repeat(MAX_TITLE_BYTES + 1), "curvebook", "").is_err());

    let mut doc = ProjectDocument::new("Origin Roundtrip").unwrap();
    let mut ds = DataSheet::new("d1", "Data", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap())).unwrap();
    ds.origin = Some(valid.clone());
    doc.add_data_sheet(ds).unwrap();

    let mut info = InfoSheet::new_preseeded("i1", "Info").unwrap();
    info.origin = Some(valid.clone());
    doc.add_info_sheet(info).unwrap();

    let mut res = ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap()).unwrap();
    res.origin = Some(valid.clone());
    doc.add_result_sheet(res).unwrap();

    let mut gs = GraphSheet::new_uninitialized_for_data("g1", &doc.data_sheets[0]).unwrap();
    gs.origin = Some(valid.clone());
    doc.add_graph_sheet(gs).unwrap();

    let mut ls = LayoutSheet::new("l1", "Layout", LayoutSpec::default()).unwrap();
    ls.origin = Some(valid.clone());
    doc.add_layout_sheet(ls).unwrap();

    let rt: ProjectDocument = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(rt.data_sheets[0].origin, Some(valid.clone()));
    assert_eq!(rt.info_sheets[0].origin, Some(valid.clone()));
    assert_eq!(rt.result_sheets[0].origin, Some(valid.clone()));
    assert_eq!(rt.graph_sheets[0].origin, Some(valid.clone()));
    assert_eq!(rt.layout_sheets[0].origin, Some(valid));
}

/// §10.2 field-enumeration collision test: populates EVERY `SheetId`-bearing field (typed and inline
/// template hooks across all 5 sheet types, including a legacy colon-bearing ID), remaps all IDs,
/// serializes the entire `ProjectDocument` to JSON, and asserts ZERO old IDs remain anywhere.
#[test]
fn section_10_2_consolidated_remap_updates_every_sheet_id_field_and_inline_hook() {
    let old_data = SheetId::new("OLD_DATA_SRC");
    let old_data_linked = SheetId::new("OLD_DATA_LNK");
    let old_info = SheetId::new("OLD:INFO:SRC");
    let old_res_fit = SheetId::new("OLD_RES_FIT");
    let old_res_colon_digit = SheetId::new("OLD:0");
    let old_res_norm = SheetId::new("OLD_RES_NRM");
    let old_res_pow = SheetId::new("OLD_RES_POW");
    let old_res_derived = SheetId::new("OLD_RES_DRV");
    let old_graph = SheetId::new("OLD_GRF_MAIN");
    let old_layout = SheetId::new("OLD_LAY_MAIN");

    let mut doc = ProjectDocument::new("Remap Enumeration").unwrap();

    // InfoSheet (with linked_data_sheets, notes_markdown hooks, floating_notes hooks).
    let mut info = InfoSheet::new_preseeded("tmp_info", "Info").unwrap();
    info.id = old_info.clone();
    info.linked_data_sheets = vec![old_data.clone(), old_data_linked.clone()];
    info.notes_markdown = format!(
        "Const {{{{info:{old_info}:Dose}}}} and {{{{result:{old_res_fit}:0:IC50}}}} and {{{{result:{old_res_colon_digit}:IC50}}}} and {{{{result:{old_res_colon_digit}:0:Top}}}} and {{{{scalar:{old_res_fit}:Top}}}}"
    );
    info.floating_notes.push(make_note("fn-info", &format!("See {{{{info:{old_info}:Lot}}}}")));
    doc.info_sheets.push(info);

    // DataSheet 1 (primary source) & DataSheet 2 (linked via InternalSheet + info_sheet_id + floating_notes).
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy.data_sets[0].title = String::from("10 uM");
    let mut ds1 = DataSheet::new("tmp_d1", "Primary Data", TableArchetype::Xy(xy.clone())).unwrap();
    ds1.id = old_data.clone();
    ds1.info_sheet_id = Some(old_info.clone());
    ds1.floating_notes.push(make_note("fn-d1", &format!("Fit {{{{result:{old_res_fit}:HillSlope}}}}")));

    let mut ds2 = DataSheet::new("tmp_d2", "Linked Data", TableArchetype::Xy(xy.clone())).unwrap();
    ds2.id = old_data_linked.clone();
    ds2.info_sheet_id = Some(old_info.clone());
    ds2.linked_regions.push(LinkedDataRegion {
        id: String::from("lr-1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: old_data.clone(),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("Internal link"),
    });
    doc.data_sheets.push(ds1);
    doc.data_sheets.push(ds2);

    // ResultSheet 1: NonlinearRegression with HookedConstant (InfoConstant, ResultScalar, ColumnTitleValue),
    // analyzable_data (with nested id, info_sheet_id, linked_regions, floating_notes), eliminated_outliers, floating_notes.
    let mut fit_spec = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit_spec.parameter_rules = vec![
        ParameterRule {
            name: String::from("Bottom"),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(old_info.clone()),
                constant_name: String::from("Blank"),
            }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("Top"),
            scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
                result_sheet_id: old_res_norm.clone(),
                parameter_key: String::from("Max"),
                data_set_index: Some(0),
            }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("Conc"),
            scope: ParameterScope::HookedConstant(HookSource::ColumnTitleValue { data_sheet_id: old_data.clone(), data_set_index: 0 }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];
    let mut res_fit =
        ResultSheet::new(AnalysisSpec::new(old_res_fit.clone(), old_data.clone(), AnalysisKind::NonlinearRegression(fit_spec)).unwrap()).unwrap();
    res_fit.status = ResultStatus::UpToDate;
    res_fit.eliminated_outliers.push(SourceCellId { sheet_id: old_data.clone(), row: 0, col: 0, subcol: 0 });
    res_fit.floating_notes.push(make_note("fn-fit", &format!("Ref {{{{scalar:{old_res_norm}:Zero}}}}")));
    let mut derived = DataSheet::new("tmp_drv", "Derived", TableArchetype::Xy(xy)).unwrap();
    derived.id = old_res_derived.clone();
    derived.info_sheet_id = Some(old_info.clone());
    derived.linked_regions.push(LinkedDataRegion {
        id: String::from("lr-drv"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: old_data_linked.clone(),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("Derived link"),
    });
    derived.floating_notes.push(make_note("fn-drv", &format!("Info {{{{info:{old_info}:Unit}}}}")));
    res_fit.analyzable_data = Some(derived);
    res_fit.tabs.push(ResultTab {
        id: String::from("tab-0"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Val")],
        row_labels: vec![String::from("Row")],
        rows: vec![vec![ResultCell::Number { value: 1.0, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    doc.result_sheets.push(res_fit);

    // ResultSheet 2: Transform Normalize with hooked zero_anchor & hundred_anchor.
    let norm_kind = AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
        zero_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: Some(old_info.clone()), constant_name: String::from("Zero") }),
        hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
            result_sheet_id: old_res_fit.clone(),
            parameter_key: String::from("Top"),
            data_set_index: None,
        }),
        output_percent: true,
        per_data_set: true,
    });
    let res_norm = ResultSheet::new(AnalysisSpec::new(old_res_norm.clone(), old_data.clone(), norm_kind).unwrap()).unwrap();
    doc.result_sheets.push(res_norm);

    // ResultSheet 3: PowerAnalysis with info_sheet_id.
    let pow_spec = PowerAnalysisSpec { info_sheet_id: Some(old_info.clone()), ..PowerAnalysisSpec::default() };
    let res_pow = ResultSheet::new(AnalysisSpec::new(old_res_pow.clone(), old_data.clone(), AnalysisKind::PowerAnalysis(pow_spec)).unwrap()).unwrap();
    doc.result_sheets.push(res_pow);

    // GraphSheet with primary_source_sheet_id, series.source_sheet_id, regression_overlay, legend_label hook,
    // comparisons, compact_letter_display, title/axis hooks, EmbeddedResultTable, ModelEquation, TextBox hook, floating_notes.
    let mut gspec = GraphSpec::default_for_data_sheet(&doc.data_sheets[0]);
    gspec.primary_source_sheet_id = old_data.clone();
    gspec.title = format!("Plot {{{{info:{old_info}:Assay}}}}");
    gspec.x_axis.title = format!("X {{{{result:{old_res_fit}:X0}}}}");
    gspec.y_axis.title = format!("Y {{{{scalar:{old_res_fit}:Y0}}}}");
    let mut y2 = gspec.y_axis.clone();
    y2.title = format!("Y2 {{{{result:{old_res_fit}:0:R2}}}}");
    gspec.y2_axis = Some(y2);
    gspec.series[0].source_sheet_id = old_data_linked.clone();
    gspec.series[0].legend_label = Some(format!("IC50={{{{result:{old_res_fit}:0:IC50}}}}"));
    gspec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: old_res_fit.clone(),
        sample_segments: 256,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    gspec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: old_res_norm.clone(),
        group_a: 0,
        group_b: 1,
        style: BracketStyle::Square,
        label_mode: BracketLabelMode::SummaryStars,
        hide_if_non_significant: false,
        stroke_width_pt: 0.75,
        font_size_pt: 8.5,
        bracket_step_pt: 12.0,
    });
    gspec.compact_letter_display = Some(CompactLetterDisplaySpec::new(old_res_pow.clone()));
    gspec.annotations.push(GraphAnnotation::EmbeddedResultTable {
        result_sheet_id: old_res_fit.clone(),
        tab_id: Some(String::from("tab-0")),
        bounds_pt: RectPt::new(10.0, 10.0, 100.0, 40.0),
        selected_rows: None,
        selected_cols: None,
        style: EmbeddedTableStyle::default(),
    });
    gspec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: old_res_fit.clone(),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: RectPt::new(10.0, 60.0, 100.0, 30.0),
        font_size_pt: 8.0,
        bordered: true,
    });
    gspec.annotations.push(GraphAnnotation::TextBox {
        markdown: format!("Note {{{{info:{old_info}:Lab}}}} {{{{result:{old_res_fit}:EC50}}}}"),
        bounds_pt: RectPt::new(10.0, 100.0, 100.0, 20.0),
        bordered: false,
    });
    let gs = GraphSheet {
        id: old_graph.clone(),
        title: String::from("Graph"),
        initialized: true,
        spec: gspec,
        floating_notes: vec![make_note("fn-g", &format!("Graph {{{{scalar:{old_res_fit}:Slope}}}}"))],
        color_tag: None,
        origin: None,
    };
    gs.validate().unwrap();
    doc.graph_sheets.push(gs);

    // LayoutSheet with LiveGraph, LiveTable, TextCallout hooks, floating_notes.
    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("p1"),
        graph_sheet_id: old_graph.clone(),
        bounds_pt: RectPt::new(10.0, 10.0, 100.0, 100.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    lspec.items.push(LayoutItem::LiveTable {
        id: String::from("p2"),
        source_sheet_id: old_res_fit.clone(),
        tab_id: Some(String::from("tab-0")),
        bounds_pt: RectPt::new(120.0, 10.0, 100.0, 100.0),
        selected_rows: None,
        selected_cols: None,
        style: EmbeddedTableStyle::default(),
    });
    lspec.items.push(LayoutItem::TextCallout {
        id: String::from("p3"),
        markdown: format!("Callout {{{{info:{old_info}:Title}}}} {{{{result:{old_res_fit}:0:Bottom}}}} {{{{scalar:{old_res_norm}:Max}}}}"),
        bounds_pt: RectPt::new(10.0, 120.0, 100.0, 20.0),
        font_size_pt: 9.0,
    });
    let mut ls = LayoutSheet::new(old_layout.clone(), "Layout", lspec).unwrap();
    ls.floating_notes.push(make_note("fn-l", &format!("Layout {{{{info:{old_info}:Date}}}}")));
    doc.layout_sheets.push(ls);

    // Top-level active_sheet_id & family_anchor_sheet_id.
    doc.active_sheet_id = Some(old_graph.clone());
    doc.family_anchor_sheet_id = Some(old_data.clone());

    let remap: BTreeMap<SheetId, SheetId> = BTreeMap::from([
        (old_data.clone(), SheetId::new("NEW_DATA_SRC")),
        (old_data_linked.clone(), SheetId::new("NEW_DATA_LNK")),
        (old_info.clone(), SheetId::new("NEW_INFO_SRC")),
        (old_res_fit.clone(), SheetId::new("NEW_RES_FIT")),
        (old_res_colon_digit.clone(), SheetId::new("NEW_RES_COLON_DIGIT")),
        (old_res_norm.clone(), SheetId::new("NEW_RES_NRM")),
        (old_res_pow.clone(), SheetId::new("NEW_RES_POW")),
        (old_res_derived.clone(), SheetId::new("NEW_RES_DRV")),
        (old_graph.clone(), SheetId::new("NEW_GRF_MAIN")),
        (old_layout.clone(), SheetId::new("NEW_LAY_MAIN")),
    ]);

    remap_document_sheet_ids(&mut doc, &remap);

    let serialized = serde_json::to_string_pretty(&doc).unwrap();
    for old_id in remap.keys() {
        assert!(!serialized.contains(old_id.as_str()), "old SheetId `{old_id}` leaked after consolidated remap:\n{serialized}");
    }
    for new_id in remap.values() {
        assert!(serialized.contains(new_id.as_str()), "expected remapped SheetId `{new_id}` in serialized document");
    }
    assert!(doc.validate_selection_ids().is_ok());
}
