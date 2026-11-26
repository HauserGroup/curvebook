use std::time::Instant;

use crate::common::{
    RawZipEntry, build_archive_zip_from_value, build_raw_zip, build_zip_entries, col_sheet, make_import_link_spec, make_result_sheet, raw_datasheet,
    result_sheet_with_kind, sample_column_project, sample_dose_project, xlsx_single_sheet_zip, xy_sheet,
};
use curvebook_doc::{
    AnalysisKind, BaselineOperation, CURRENT_SCHEMA_VERSION, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, DemingErrorSpec,
    DocError, FactorialAnovaSpec, FloatingNote, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoConstant, InfoSheet,
    InternalLinkKind, LinearRegressionSpec, LinkBorderColor, LinkSource, LinkedDataRegion, MAX_SUBCOLUMNS, MAX_TITLE_BYTES, MixedModelDfMethod,
    MultipleVariablesTable, NestedAnalysisSpec, NestedTable, NonlinearFitSpec, NoteId, OneWayAnovaSpec, PartsOfWholeTable, PlacementMode,
    PowerAnalysisSpec, ProjectDocument, RectPt, RepeatedMeasuresDesign, ResidualCovariance, ResultStatus, SequenceProgression, SheetCellRange,
    SheetId, SourceCellId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable,
};
use curvebook_format::{
    ArchiveKind, CsvOptions, FormatError, ImportBaselineAdjustment, XlsxImportOptions, ZipArchive, ZipLimits, ZipWriter, apply_csv_import_link,
    export_datasheet_to_csv, filter_delimited_text, import_csv_to_datasheet, import_csv_to_datasheet_with_x_mode, import_xlsx_bytes_with_options,
    instantiate_template_bytes, migrate_document_json, open_archive_bytes, open_archive_bytes_with_limits, open_project_bytes, open_template_bytes,
    parse_cell_token, parse_delimited_records, rearrange_and_baseline_grid, save_project_bytes, save_template_bytes, validate_project_document,
};
use serde_json::{Value, json};

#[test]
fn curvebook_archive_is_deterministic_and_round_trips() {
    let doc = sample_dose_project();
    let bytes1 = save_project_bytes(&doc).unwrap();
    assert_eq!(bytes1, save_project_bytes(&doc).unwrap());
    assert_eq!(open_archive_bytes(&bytes1).unwrap(), (ArchiveKind::Project, doc));
    assert!(matches!(open_template_bytes(&bytes1), Err(FormatError::InvalidArchiveKind { expected: "template", found: "project" })));
}

#[test]
fn cbtemplate_strips_raw_cells_and_cached_results_while_preserving_schemas() {
    let doc = sample_dose_project();
    let tmpl_bytes = save_template_bytes(&doc).unwrap();
    assert!(matches!(open_project_bytes(&tmpl_bytes), Err(FormatError::InvalidArchiveKind { expected: "project", found: "template" })));

    let tmpl = open_template_bytes(&tmpl_bytes).unwrap();
    let TableArchetype::Xy(xy) = &tmpl.data_sheets[0].archetype else { panic!("expected XY table") };
    assert_eq!((xy.x_title.as_str(), xy.data_sets[0].title.as_str()), ("Log[Agonist]", "Wildtype"));
    assert!(xy.x_cells[0][0].is_empty() && xy.data_sets[0].cell(0, 0).unwrap().is_empty());
    assert!(tmpl.result_sheets[0].status == ResultStatus::Dirty && tmpl.result_sheets[0].scalars.is_empty());
    assert_eq!(tmpl.info_sheets[0].get_constant("HotNM").unwrap().value.as_finite_f64(), Some(2.5));
    assert_eq!((tmpl.graph_sheets.len(), tmpl.layout_sheets.len()), (1, 1));

    let inst = instantiate_template_bytes(&tmpl_bytes, Some("Replicate Run #2")).unwrap();
    assert_eq!((inst.title.as_str(), &inst.result_sheets[0].status), ("Replicate Run #2", &ResultStatus::Dirty));
}

#[test]
fn archive_detects_crc_corruption_and_missing_entries() {
    let doc = sample_dose_project();
    let doc_bytes = serde_json::to_vec(&doc).unwrap();
    let bad_manifest = json!({
        "format": "curvebook-archive", "schema_version": CURRENT_SCHEMA_VERSION, "kind": "Project",
        "title": doc.title, "document_crc32": crc32fast::hash(&doc_bytes) ^ 0xFFFF_FFFF, "sheets": []
    });
    let corrupt_zip = build_zip_entries(&[("document.json", &doc_bytes), ("manifest.json", &serde_json::to_vec(&bad_manifest).unwrap())]);
    assert!(matches!(open_project_bytes(&corrupt_zip), Err(FormatError::CorruptCrc { .. })));
    assert!(matches!(open_project_bytes(&build_zip_entries(&[("document.json", &doc_bytes)])), Err(FormatError::MissingArchiveEntry { .. })));
}

#[test]
fn zip_rejects_path_traversal_duplicates_and_zip_bombs() {
    let mut w = ZipWriter::new();
    for bad in ["../escape.txt", "/abs/path.txt", "dir\\win.txt"] {
        assert!(matches!(w.add_file(bad, b"x".to_vec()), Err(FormatError::UnsafePath { .. })), "{bad}");
    }
    w.add_file("a.txt", b"1".to_vec()).unwrap();
    assert!(matches!(w.add_file("a.txt", b"2".to_vec()), Err(FormatError::DuplicateEntry { .. })));

    let bytes = save_project_bytes(&sample_dose_project()).unwrap();
    for limits in [
        ZipLimits { max_entries: 1, max_total_uncompressed_bytes: 1024 * 1024, max_entry_uncompressed_bytes: 1024 * 1024 },
        ZipLimits { max_entries: 100, max_total_uncompressed_bytes: 64, max_entry_uncompressed_bytes: 64 },
    ] {
        assert!(matches!(open_archive_bytes_with_limits(&bytes, limits), Err(FormatError::ZipBomb { .. })));
    }
}

#[test]
fn schema_migration_v0_to_v1_and_future_version_rejection() {
    let v0_doc = json!({ "schema_version": 0, "title": "Legacy Study", "data_sheets": [], "info_sheets": [], "result_sheets": [], "graph_sheets": [], "layout_sheets": [] });
    let migrated = open_project_bytes(&build_archive_zip_from_value(&v0_doc, 0)).unwrap();
    assert_eq!((migrated.title.as_str(), migrated.schema_version), ("Legacy Study", CURRENT_SCHEMA_VERSION));

    let fut = CURRENT_SCHEMA_VERSION + 5;
    let future_doc = json!({ "schema_version": fut, "title": "Future Study", "data_sheets": [], "info_sheets": [], "result_sheets": [], "graph_sheets": [], "layout_sheets": [], "active_sheet_id": null, "family_anchor_sheet_id": null });
    assert!(matches!(open_project_bytes(&build_archive_zip_from_value(&future_doc, fut)), Err(FormatError::UnsupportedSchemaVersion { .. })));

    // V1 legacy FactorialAnova missing `residual_covariance`.
    let mut v1_src = ProjectDocument::new("V1 RM Study").unwrap();
    v1_src
        .add_data_sheet(
            DataSheet::new("data-1", "Grouped Data", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).unwrap()))
                .unwrap(),
        )
        .unwrap();
    let v1_fac = FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched, ..Default::default() };
    v1_src.add_result_sheet(result_sheet_with_kind("res-rm-1", "data-1", AnalysisKind::FactorialAnova(v1_fac))).unwrap();
    let mut v1_doc = serde_json::to_value(&v1_src).unwrap();
    v1_doc["schema_version"] = json!(1_u32);
    v1_doc["result_sheets"][0]["analysis"]["kind"]["FactorialAnova"].as_object_mut().unwrap().remove("residual_covariance");

    let mig_v1 = open_project_bytes(&build_archive_zip_from_value(&v1_doc, 1)).unwrap();
    assert_eq!((mig_v1.schema_version, mig_v1.result_sheets.len()), (CURRENT_SCHEMA_VERSION, 1));
    assert!(matches!(
        &mig_v1.result_sheets[0].analysis.kind,
        AnalysisKind::FactorialAnova(s) if s.residual_covariance == ResidualCovariance::CompoundSymmetry && s.df_method == MixedModelDfMethod::KenwardRoger && !s.mixed_effects_reml
    ));

    // V2 missing `df_method` / `mixed_effects_reml` across OneWayAnova, FactorialAnova, NestedComparison.
    let mut v2_src = ProjectDocument::new("V2 RM Study").unwrap();
    v2_src
        .add_data_sheet(
            DataSheet::new(
                "data-col",
                "RM Column",
                TableArchetype::Column(ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 4).unwrap()),
            )
            .unwrap(),
        )
        .unwrap();
    v2_src
        .add_data_sheet(
            DataSheet::new("data-grp", "RM Grouped", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).unwrap()))
                .unwrap(),
        )
        .unwrap();
    v2_src.add_data_sheet(DataSheet::new("data-nst", "Nested", TableArchetype::Nested(NestedTable::new(2, 2, 2).unwrap())).unwrap()).unwrap();
    for (id, src, kind) in [
        ("res-ow", "data-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())),
        ("res-fac", "data-grp", AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())),
        ("res-nst", "data-nst", AnalysisKind::NestedComparison(NestedAnalysisSpec::default())),
    ] {
        v2_src.add_result_sheet(result_sheet_with_kind(id, src, kind)).unwrap();
    }
    let mut v2_doc = serde_json::to_value(&v2_src).unwrap();
    v2_doc["schema_version"] = json!(2_u32);
    for (idx, variant, keys) in [
        (0, "OneWayAnova", &["df_method", "mixed_effects_reml", "residual_covariance"][..]),
        (1, "FactorialAnova", &["df_method", "mixed_effects_reml"][..]),
        (2, "NestedComparison", &["df_method"][..]),
    ] {
        let obj = v2_doc["result_sheets"][idx]["analysis"]["kind"][variant].as_object_mut().unwrap();
        for k in keys {
            obj.remove(*k);
        }
    }
    let mig_v2 = open_project_bytes(&build_archive_zip_from_value(&v2_doc, 2)).unwrap();
    assert_eq!(mig_v2.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(
        matches!(&mig_v2.result_sheets[0].analysis.kind, AnalysisKind::OneWayAnova(s) if s.df_method == MixedModelDfMethod::KenwardRoger && !s.mixed_effects_reml && s.residual_covariance == ResidualCovariance::CompoundSymmetry)
    );
    assert!(
        matches!(&mig_v2.result_sheets[1].analysis.kind, AnalysisKind::FactorialAnova(s) if s.df_method == MixedModelDfMethod::KenwardRoger && !s.mixed_effects_reml)
    );
    assert!(matches!(&mig_v2.result_sheets[2].analysis.kind, AnalysisKind::NestedComparison(s) if s.df_method == MixedModelDfMethod::KenwardRoger));
}

#[test]
fn detects_circular_hook_and_import_link_cycles_and_bounds() {
    let make_link = |id: &str, target: &str| LinkedDataRegion {
        id: String::from(id),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new(target),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::default(),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::default(),
        provenance_summary: format!("{target}->{id}"),
    };
    let mut doc = ProjectDocument::new("Cycle Test").unwrap();
    let (mut ds_a, mut ds_b) = (col_sheet("data-a", 1, 2), col_sheet("data-b", 1, 2));
    ds_a.linked_regions.push(make_link("l-ab", "data-b"));
    ds_b.linked_regions.push(make_link("l-ba", "data-a"));
    doc.add_data_sheet(ds_a).unwrap();
    doc.add_data_sheet(ds_b).unwrap();
    assert!(matches!(save_project_bytes(&doc), Err(FormatError::DependencyCycle { .. })));

    let mut doc2 = ProjectDocument::new("Self Loop").unwrap();
    doc2.add_result_sheet(result_sheet_with_kind("res-1", "res-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))).unwrap();
    assert!(matches!(save_project_bytes(&doc2), Err(FormatError::DependencyCycle { .. })));

    let mut doc3 = ProjectDocument::new("Bounds").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
    col.data_sets[0].cells[0][0].value = CellValue::Number(f64::NAN);
    doc3.add_data_sheet(DataSheet::new("d1", "D1", TableArchetype::Column(col)).unwrap()).unwrap();
    assert!(matches!(save_project_bytes(&doc3), Err(FormatError::NonFiniteFloat { .. })));

    let mut doc4 = ProjectDocument::new("Title Bounds").unwrap();
    doc4.title = "X".repeat(MAX_TITLE_BYTES + 1);
    assert!(matches!(save_project_bytes(&doc4), Err(FormatError::Doc(DocError::TitleTooLong { .. }))));
}

#[test]
fn csv_rfc4180_regional_sentinels_and_exclusions() {
    let csv = "\u{FEFF}\"Group, A\";\"Group \"\"B\"\"\"\r\n1,234;45,6*\r\nNA;*\r\nNULL;.\r\n\"line1\nline2\";*7,5\r";
    let detected = CsvOptions::detect(csv);
    assert_eq!((detected.delimiter, detected.decimal_separator), (';', ','));

    let opts = CsvOptions { thousands_separator: Some('.'), ..CsvOptions::european_csv() };
    let records = parse_delimited_records(csv, &opts).unwrap();
    assert_eq!(
        (records.len(), &records[0], records[4][0].as_str()),
        (5, &vec![String::from("Group, A"), String::from("Group \"B\"")], "line1\nline2")
    );

    for (tok, expected) in [
        ("1.234,5", Cell::number(1234.5)),
        ("45,6*", Cell::excluded_number(45.6)),
        ("*", Cell { value: CellValue::Empty, excluded: true }),
        ("NA", Cell::empty()),
        ("NULL", Cell::empty()),
        (".", Cell::empty()),
    ] {
        assert_eq!(parse_cell_token(tok, &opts), expected, "token={tok:?}");
    }

    let sheet = import_csv_to_datasheet("csv-1", "Regional Import", csv, TableKind::Column, SubcolumnMode::Single, &opts).unwrap();
    let re = import_csv_to_datasheet(
        "csv-2",
        "Regional Import",
        &export_datasheet_to_csv(&sheet, &opts).unwrap(),
        TableKind::Column,
        SubcolumnMode::Single,
        &opts,
    )
    .unwrap();
    assert_eq!(sheet.archetype, re.archetype);
}

#[test]
fn multi_stage_import_link_pipeline_filter_transpose_baseline_and_freeze() {
    let filter = ImportFilterSpec {
        row_window: Some((0, 3)),
        col_window: Some((1, 2)),
        decimate_every_k: Some(2),
        parse_asterisk_exclusions: true,
        ..Default::default()
    };
    let filtered = filter_delimited_text("Dose,RespA,RespB\n0,10,20\n1,15,25*\n2,20,30\n3,25,35\n", ',', '.', true, &filter).unwrap();
    assert_eq!(
        (&filtered.headers, filtered.cells.len(), filtered.cells[0][0].active_number(), filtered.cells[1][1].active_number()),
        (&vec![String::from("RespA"), String::from("RespB")], 2, Some(10.0), Some(30.0))
    );

    let placement = ImportPlacementSpec {
        mode: PlacementMode::Overwrite,
        anchor: CellCoord::new(0, 0, 0),
        propagate_title_header: true,
        skip_contiguous_blanks: true,
        transpose: true,
    };
    let adjusted = rearrange_and_baseline_grid(&filtered.cells, &placement, ImportBaselineAdjustment::SubtractFirstRow);
    assert_eq!((adjusted[0][0].active_number(), adjusted[1][1].active_number()), (Some(0.0), Some(10.0)));

    let mut target = col_sheet("tgt-1", 2, 2);
    let spec = make_import_link_spec(PlacementMode::Overwrite, CellCoord::new(0, 0, 0), true);
    let range = apply_csv_import_link(
        &mut target,
        "ColA,ColB\n10,20\n15,25*\n",
        &spec,
        true,
        ImportBaselineAdjustment::SubtractFirstRow,
        "lnk-csv",
        "From assay.csv",
    )
    .unwrap();
    assert_eq!(
        (range.start, range.end, target.linked_regions[0].border_color()),
        (CellCoord::new(0, 0, 0), CellCoord::new(1, 1, 0), Some(LinkBorderColor::Charcoal))
    );
    assert_eq!(
        (target.cell(CellCoord::new(1, 0, 0)).unwrap().active_number(), target.cell(CellCoord::new(1, 1, 0)).unwrap().excluded),
        (Some(5.0), true)
    );

    let unstacked = filter_delimited_text(
        "Group,Value\nCtrl,10\nTreat,20\nCtrl,12\nTreat,24*\n",
        ',',
        '.',
        true,
        &ImportFilterSpec { unstack_group_column: Some(0), ..Default::default() },
    )
    .unwrap();
    assert_eq!(
        (&unstacked.headers, unstacked.cells[0][0].active_number(), unstacked.cells[1][0].active_number(), unstacked.cells[1][1].excluded),
        (&vec![String::from("Ctrl"), String::from("Treat")], Some(10.0), Some(12.0), true)
    );

    target.frozen = true;
    assert!(matches!(
        apply_csv_import_link(&mut target, "A\n1\n", &spec, true, ImportBaselineAdjustment::None, "lnk-2", "Blocked"),
        Err(FormatError::Doc(DocError::SheetFrozen { .. }))
    ));
}

#[test]
fn forged_zip_header_inflating_past_budget_is_blocked_as_zip_bomb() {
    let payload = vec![b'A'; 4096];
    let mut entry = RawZipEntry::deflated(b"bomb.txt", &payload);
    entry.declared_uncompressed = Some(16);
    let limits = ZipLimits { max_entries: 16, max_total_uncompressed_bytes: 512, max_entry_uncompressed_bytes: 512 };
    assert!(matches!(ZipArchive::from_bytes_with_limits(&build_raw_zip(&[entry], b""), limits), Err(FormatError::ZipBomb { .. })));
}

#[test]
fn tsv_and_csv_round_trip_multiple_archetypes() {
    let tsv_opts = CsvOptions::tsv();
    let xy_sheet = import_csv_to_datasheet(
        "xy-1",
        "XY TSV",
        "Conc\tResp:Y1\tResp:Y2\n1\t10.5\t11.0\n10\t52.3*\t54.1\n",
        TableKind::Xy,
        SubcolumnMode::RawReplicates(2),
        &tsv_opts,
    )
    .unwrap();
    let c = xy_sheet.cell(CellCoord::new(1, 0, 0)).unwrap();
    assert!(c.active_number().is_none() && c.excluded);
    let xy_back = import_csv_to_datasheet(
        "xy-2",
        "XY TSV",
        &export_datasheet_to_csv(&xy_sheet, &tsv_opts).unwrap(),
        TableKind::Xy,
        SubcolumnMode::RawReplicates(2),
        &tsv_opts,
    )
    .unwrap();
    assert_eq!(xy_sheet.archetype, xy_back.archetype);

    let mv_sheet = import_csv_to_datasheet(
        "mv-1",
        "Cohort",
        "Subject,Dose,Outcome\nS1,10,Responder\nS2,20,NonResponder\n",
        TableKind::MultipleVariables,
        SubcolumnMode::Single,
        &CsvOptions::csv(),
    )
    .unwrap();
    let mv_back = import_csv_to_datasheet(
        "mv-2",
        "Cohort",
        &export_datasheet_to_csv(&mv_sheet, &CsvOptions::csv()).unwrap(),
        TableKind::MultipleVariables,
        SubcolumnMode::Single,
        &CsvOptions::csv(),
    )
    .unwrap();
    assert_eq!(mv_sheet.archetype, mv_back.archetype);
}

#[test]
fn r1_s05_01_archive_validation_and_open_reject_ragged_and_disallowed_tables() {
    let mut mv = MultipleVariablesTable::new(2, 4).unwrap();
    mv.variables[1].cells.truncate(2);
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 1).unwrap();
    xy.data_sets[1].cells.push(vec![Cell::number(3.0)]);
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
    grp.data_sets[0].cells[1].pop();
    let mut nested = NestedTable::new(1, 2, 2).unwrap();
    nested.groups[0].cells[0].pop();
    let mut surv = SurvivalTable::new(1, 2).unwrap();
    surv.times.pop();
    let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
    pow.slice_labels.pop();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    col.subcolumn_mode = SubcolumnMode::RawReplicates(3);
    let mut xy_nan = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy_nan.x_mode = XColumnMode::GeneratedSequence { start: f64::NAN, step: 1.0, progression: SequenceProgression::Arithmetic };

    type CheckErr = fn(&Result<(), FormatError>) -> bool;
    for (label, arch, check) in [
        ("mv", TableArchetype::MultipleVariables(mv), (|r| matches!(r, Err(FormatError::Doc(DocError::InvalidCellValue { .. })))) as CheckErr),
        ("xy", TableArchetype::Xy(xy), |r| matches!(r, Err(FormatError::Doc(DocError::InvalidCellValue { .. })))),
        ("grp", TableArchetype::Grouped(grp), |r| matches!(r, Err(FormatError::Doc(DocError::InvalidCellValue { .. })))),
        ("nest", TableArchetype::Nested(nested), |r| matches!(r, Err(FormatError::Doc(DocError::InvalidCellValue { .. })))),
        ("surv", TableArchetype::Survival(surv), |r| matches!(r, Err(FormatError::Doc(DocError::InvalidCellValue { .. })))),
        ("pow", TableArchetype::PartsOfWhole(pow), |r| matches!(r, Err(FormatError::Doc(DocError::InvalidCellValue { .. })))),
        ("col_mode", TableArchetype::Column(col), |r| matches!(r, Err(FormatError::Doc(DocError::IncompatibleSubcolumnMode { .. })))),
        ("xy_nan_seq", TableArchetype::Xy(xy_nan), |r| matches!(r, Err(FormatError::NonFiniteFloat { .. }))),
    ] {
        let mut doc = ProjectDocument::new(label).unwrap();
        doc.data_sheets.push(raw_datasheet(label, arch));
        assert!(check(&validate_project_document(&doc)), "{label}");
    }

    let forged_mv = json!({
        "schema_version": CURRENT_SCHEMA_VERSION, "title": "Forged",
        "data_sheets": [{ "id": "d1", "title": "MV", "archetype": { "MultipleVariables": {
            "row_titles": ["a", "b"], "row_titles_expanded": false,
            "variables": [
                { "title": "Y", "role": "Continuous", "source": "Entered", "cells": [{"value":"Empty","excluded":false}, {"value":"Empty","excluded":false}], "category_order": null },
                { "title": "G", "role": "Categorical", "source": "Entered", "cells": [{"value":"Empty","excluded":false}], "category_order": null }
            ]
        }}}],
        "info_sheets": [], "result_sheets": [], "graph_sheets": [], "layout_sheets": []
    });
    assert!(open_project_bytes(&build_archive_zip_from_value(&forged_mv, CURRENT_SCHEMA_VERSION)).is_err());
}

#[test]
fn r1_s05_03_x_column_mode_rejected_by_archive_csv_export_and_placement() {
    let spec = make_import_link_spec(PlacementMode::Overwrite, CellCoord::new(0, 0, 0), false);
    for bad_sub in [0_usize, MAX_SUBCOLUMNS + 1, usize::MAX] {
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
        xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(bad_sub) };
        let mut sheet = raw_datasheet("xy-bad-x", TableArchetype::Xy(xy));
        let mut doc = ProjectDocument::new("BadXMode").unwrap();
        doc.data_sheets.push(sheet.clone());
        assert!(matches!(validate_project_document(&doc), Err(FormatError::Doc(_))));
        assert!(matches!(export_datasheet_to_csv(&sheet, &CsvOptions::csv()), Err(FormatError::Doc(_))));
        assert!(matches!(
            apply_csv_import_link(&mut sheet, "X,Y\n1,2\n3,4\n", &spec, true, ImportBaselineAdjustment::None, "lnk", "test"),
            Err(FormatError::Doc(_))
        ));
    }

    let forged_xmode = json!({
        "schema_version": CURRENT_SCHEMA_VERSION, "title": "ForgedX",
        "data_sheets": [{ "id": "xy1", "title": "XY", "archetype": { "Xy": {
            "x_title": "X", "x_mode": { "Numbers": { "subcolumn_mode": { "RawReplicates": usize::MAX } } },
            "y_subcolumn_mode": "Single", "row_titles": ["", ""], "row_titles_expanded": false,
            "x_cells": [[{"value":"Empty","excluded":false}], [{"value":"Empty","excluded":false}]],
            "data_sets": [{ "title": "A", "cells": [[{"value":"Empty","excluded":false}], [{"value":"Empty","excluded":false}]] }]
        }}}],
        "info_sheets": [], "result_sheets": [], "graph_sheets": [], "layout_sheets": []
    });
    assert!(open_project_bytes(&build_archive_zip_from_value(&forged_xmode, CURRENT_SCHEMA_VERSION)).is_err());
}

fn migrate_doc_from(doc: &ProjectDocument, from_ver: u32, patch_legacy: impl FnOnce(&mut Value)) -> ProjectDocument {
    let mut legacy = serde_json::to_value(doc).unwrap();
    legacy["schema_version"] = json!(from_ver);
    patch_legacy(&mut legacy);
    let migrated: ProjectDocument = serde_json::from_value(migrate_document_json(legacy, from_ver).unwrap()).unwrap();
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
    migrated
}

#[test]
fn linear_fit_range_schema_migrates_v3_and_round_trips() {
    let mut doc = ProjectDocument::new("Linear Regression").unwrap();
    doc.add_data_sheet(xy_sheet("xy", SubcolumnMode::Single, 1, 3)).unwrap();
    doc.add_result_sheet(result_sheet_with_kind("linear", "xy", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))).unwrap();
    let mut migrated = migrate_doc_from(&doc, 3, |_| {});
    let AnalysisKind::LinearRegression(spec) = &mut migrated.result_sheets[0].analysis.kind else { panic!("expected linear fit") };
    assert_eq!(*spec, LinearRegressionSpec::default());
    (spec.x_range_filter, spec.plot_x_max, spec.plot_segments) = (Some((Some(10.0), None)), Some(20.0), Some(12));
    let expected = *spec;
    assert_eq!(
        open_project_bytes(&save_project_bytes(&migrated).unwrap()).unwrap().result_sheets[0].analysis.kind,
        AnalysisKind::LinearRegression(expected)
    );
}

#[test]
fn nonlinear_fit_diagnostics_migrate_v4_and_round_trip() {
    let mut doc = ProjectDocument::new("Dose Response").unwrap();
    doc.add_data_sheet(xy_sheet("xy", SubcolumnMode::Single, 1, 4)).unwrap();
    doc.add_result_sheet(result_sheet_with_kind("fit", "xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line")))).unwrap();
    let mut migrated = migrate_doc_from(&doc, 4, |v| {
        v["result_sheets"][0]["analysis"]["kind"]["NonlinearRegression"].as_object_mut().unwrap().remove("identify_unstable_parameters");
    });
    let AnalysisKind::NonlinearRegression(spec) = &mut migrated.result_sheets[0].analysis.kind else { panic!("expected nonlinear fit") };
    assert!(!spec.identify_unstable_parameters && spec.flag_r_squared_below.is_none());
    (spec.identify_unstable_parameters, spec.flag_r_squared_below) = (true, Some(0.92));
    let expected = spec.clone();
    assert_eq!(
        open_project_bytes(&save_project_bytes(&migrated).unwrap()).unwrap().result_sheets[0].analysis.kind,
        AnalysisKind::NonlinearRegression(expected)
    );
}

#[test]
fn rout_outlier_cells_migrate_v5_and_round_trip() {
    let mut doc = sample_dose_project();
    let migrated = migrate_doc_from(&doc, 5, |v| assert!(v["result_sheets"][0].get("eliminated_outliers").is_none()));
    assert!(migrated.result_sheets[0].eliminated_outliers.is_empty());

    let source = SourceCellId { sheet_id: SheetId::new("data-xy"), row: 1, col: 0, subcol: 1 };
    doc.result_sheets[0].eliminated_outliers.push(source.clone());
    assert_eq!(open_project_bytes(&save_project_bytes(&doc).unwrap()).unwrap().result_sheets[0].eliminated_outliers, [source]);
}

#[test]
fn gap_06_05_levene_test_schema_v8_upgrades_v7_and_rejects_future_before_serde() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 8);

    // 1. Persisting `AnalysisKind::LeveneTest` stamps both manifest and document with schema v8 and round-trips cleanly.
    let mut doc = ProjectDocument::new("Levene Schema V8").unwrap();
    doc.add_data_sheet(col_sheet("data-col", 3, 4)).unwrap();
    doc.add_result_sheet(result_sheet_with_kind(
        "res-bf",
        "data-col",
        AnalysisKind::LeveneTest(curvebook_doc::LeveneTestSpec { center: curvebook_doc::LeveneCenter::Median }),
    ))
    .unwrap();
    doc.add_result_sheet(result_sheet_with_kind(
        "res-lev",
        "data-col",
        AnalysisKind::LeveneTest(curvebook_doc::LeveneTestSpec { center: curvebook_doc::LeveneCenter::Mean }),
    ))
    .unwrap();

    let saved = save_project_bytes(&doc).unwrap();
    let zip = curvebook_format::zip::ZipArchive::from_bytes(&saved).unwrap();
    let manifest_val: Value = serde_json::from_str(zip.get_utf8("manifest.json").unwrap().unwrap()).unwrap();
    let doc_val: Value = serde_json::from_str(zip.get_utf8("document.json").unwrap().unwrap()).unwrap();
    assert_eq!(manifest_val["schema_version"], json!(8));
    assert_eq!(doc_val["schema_version"], json!(8));

    let reopened = open_project_bytes(&saved).unwrap();
    assert_eq!(reopened.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        reopened.result_sheets[0].analysis.kind,
        AnalysisKind::LeveneTest(curvebook_doc::LeveneTestSpec { center: curvebook_doc::LeveneCenter::Median })
    );
    assert_eq!(
        reopened.result_sheets[1].analysis.kind,
        AnalysisKind::LeveneTest(curvebook_doc::LeveneTestSpec { center: curvebook_doc::LeveneCenter::Mean })
    );

    // 2. An older schema-v7 reader (max_supported = 7) rejects the v8 LeveneTest payload with
    //    `UnsupportedSchemaVersion` instead of attempting serde deserialization and failing with `unknown variant 'LeveneTest'`.
    fn noop_step(_: &mut Value) -> Result<(), FormatError> {
        Ok(())
    }
    let v7_steps: Vec<curvebook_format::versioned::VersionStep> =
        (0..7).map(|from_version| curvebook_format::versioned::VersionStep { from_version, apply: noop_step }).collect();
    let v7_reader_err = curvebook_format::versioned::upgrade(doc_val.clone(), 8, 0, 7, &v7_steps).unwrap_err();
    assert!(
        matches!(v7_reader_err, FormatError::UnsupportedSchemaVersion { version: 8, max_supported: 7 }),
        "expected UnsupportedSchemaVersion(8, 7), got {v7_reader_err:?}"
    );

    // 3. A schema-v7 fixture (no LeveneTest) upgrades cleanly through `v7 -> v8` to `CURRENT_SCHEMA_VERSION == 8`.
    let base_v7 = sample_dose_project();
    let mut v7_val = serde_json::to_value(&base_v7).unwrap();
    v7_val["schema_version"] = json!(7_u32);
    let upgraded_v7 = open_project_bytes(&build_archive_zip_from_value(&v7_val, 7)).unwrap();
    assert_eq!(upgraded_v7.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(upgraded_v7.data_sheets.len(), base_v7.data_sheets.len());
    assert_eq!(upgraded_v7.result_sheets.len(), base_v7.result_sheets.len());

    // 4. Forward-compat guard on current v8 reader: a v9 archive carrying an unknown future `AnalysisKind`
    //    variant must fail with `UnsupportedSchemaVersion { version: 9, max_supported: 8 }`, NOT raw serde `unknown variant`.
    let mut v9_val = doc_val;
    v9_val["schema_version"] = json!(CURRENT_SCHEMA_VERSION + 1);
    v9_val["result_sheets"][0]["analysis"]["kind"] = json!({ "FutureAnalysisV9": { "option": true } });
    let future_err = open_project_bytes(&build_archive_zip_from_value(&v9_val, CURRENT_SCHEMA_VERSION + 1)).unwrap_err();
    assert!(
        matches!(
            future_err,
            FormatError::UnsupportedSchemaVersion { version, max_supported }
                if version == CURRENT_SCHEMA_VERSION + 1 && max_supported == CURRENT_SCHEMA_VERSION
        ),
        "expected UnsupportedSchemaVersion(9, 8) before serde, got {future_err:?}"
    );
}

#[test]
fn save_validation_parity_and_false_eocd_comment_skip() {
    // Save must reject every invalid sheet state that open rejects.
    let mut docs: Vec<(&str, ProjectDocument)> = Vec::new();
    let mut d = sample_column_project("Neg Note");
    d.data_sheets[0].floating_notes.push(FloatingNote {
        id: NoteId::new("n1"),
        fill_color: FloatingNote::DEFAULT_FILL,
        bounds_pt: RectPt { x: 0.0, y: 0.0, width: -10.0, height: 20.0 },
        minimized: false,
        body_markdown: String::from("bad"),
    });
    docs.push(("neg_note", d));
    let mut d = sample_column_project("Ws Const");
    let mut info = InfoSheet::new_preseeded("info-1", "Info").unwrap();
    info.constants.push(InfoConstant { name: String::from("   "), value: curvebook_doc::ConstantValue::Empty, units: None });
    d.info_sheets.push(info);
    docs.push(("ws_const", d));
    let mut d = sample_column_project("Empty Info Note");
    let mut info = InfoSheet::new_preseeded("info-1", "Info").unwrap();
    info.floating_notes.push(FloatingNote::new("  ", RectPt::new(0.0, 0.0, 50.0, 50.0), "note"));
    d.info_sheets.push(info);
    docs.push(("empty_info_note", d));
    let mut d = sample_column_project("Bad Power");
    let mut rs = make_result_sheet("res-1", "data-1", ResultStatus::UpToDate);
    rs.analysis.kind = AnalysisKind::PowerAnalysis(PowerAnalysisSpec { alpha: 1.5, ..Default::default() });
    d.result_sheets.push(rs);
    docs.push(("bad_power", d));
    let mut d = sample_column_project("NaN Linear");
    let mut rs = make_result_sheet("res-1", "data-1", ResultStatus::UpToDate);
    rs.analysis.kind = AnalysisKind::LinearRegression(LinearRegressionSpec {
        fixed_point: Some((f64::NAN, 0.0)),
        deming_error: DemingErrorSpec::VarianceRatio { lambda: 1.0 },
        ..Default::default()
    });
    d.result_sheets.push(rs);
    docs.push(("nan_linear", d));
    for (label, doc) in docs {
        assert!(save_project_bytes(&doc).is_err(), "{label}");
    }
    assert!(open_project_bytes(&save_project_bytes(&sample_column_project("Valid")).unwrap()).is_ok());

    // ZIP EOCD scan skips false EOCD signature embedded in trailing comment, while rejecting multi-disk-only ZIP.
    let mut fake_eocd = Vec::with_capacity(32);
    fake_eocd.extend_from_slice(b"note:");
    fake_eocd.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    for v in [1_u16, 0, 1, 1] {
        fake_eocd.extend_from_slice(&v.to_le_bytes());
    }
    fake_eocd.extend_from_slice(&[0_u8; 10]);
    let zip = build_raw_zip(&[RawZipEntry::stored(b"hello.txt", b"world")], &fake_eocd);
    assert_eq!(ZipArchive::from_bytes(&zip).unwrap().get("hello.txt"), Some(b"world".as_slice()));
    let mut only_bad = vec![0_u8; 22];
    only_bad[0..4].copy_from_slice(&0x0605_4b50_u32.to_le_bytes());
    only_bad[4..6].copy_from_slice(&2_u16.to_le_bytes());
    assert!(matches!(ZipArchive::from_bytes(&only_bad), Err(FormatError::InvalidZip { .. })));
}

#[test]
fn csv_baseline_edge_cases_predicate_rejection_quoted_asterisks_and_detect_bounds() {
    // Baseline adjustments clear zero, missing, excluded, and overflow to Empty.
    let pct = rearrange_and_baseline_grid(
        &[vec![Cell::number(0.0), Cell::number(10.0)], vec![Cell::number(5.0), Cell::number(20.0)]],
        &ImportPlacementSpec::default(),
        ImportBaselineAdjustment::PercentOfFirstRow,
    );
    assert_eq!(pct, vec![vec![Cell::empty(), Cell::number(100.0)], vec![Cell::empty(), Cell::number(200.0)]]);
    assert_eq!(
        rearrange_and_baseline_grid(
            &[vec![Cell::number(1e-308)], vec![Cell::number(1e5)]],
            &ImportPlacementSpec::default(),
            ImportBaselineAdjustment::PercentOfFirstRow
        )[1][0],
        Cell::empty()
    );
    let sub_row = rearrange_and_baseline_grid(
        &[vec![Cell::excluded_number(10.0), Cell::number(-f64::MAX)], vec![Cell::number(42.0), Cell::number(f64::MAX)]],
        &ImportPlacementSpec::default(),
        ImportBaselineAdjustment::SubtractFirstRow,
    );
    assert_eq!(sub_row[1], vec![Cell::empty(), Cell::empty()]);
    let sub_col = rearrange_and_baseline_grid(
        &[vec![Cell::empty(), Cell::number(99.0)], vec![Cell::number(10.0), Cell::number(25.0)]],
        &ImportPlacementSpec::default(),
        ImportBaselineAdjustment::SubtractColumn(0),
    );
    assert_eq!((&sub_col[0][1], &sub_col[1][0], &sub_col[1][1]), (&Cell::empty(), &Cell::number(0.0), &Cell::number(15.0)));

    // filter_delimited_text baseline Difference/Ratio, thousands grouping, and exclude_row_predicate rejection.
    let diff = filter_delimited_text(
        "Base,Resp\n10,25\n20,50\n",
        ',',
        '.',
        true,
        &ImportFilterSpec { baseline: Some((0, BaselineOperation::Difference)), ..Default::default() },
    )
    .unwrap();
    assert_eq!(diff.cells, vec![vec![Cell::number(0.0), Cell::number(15.0)], vec![Cell::number(0.0), Cell::number(30.0)]]);
    let ratio = filter_delimited_text(
        "Base,Resp\n0,25\n5,20\n",
        ',',
        '.',
        true,
        &ImportFilterSpec { baseline: Some((0, BaselineOperation::Ratio)), ..Default::default() },
    )
    .unwrap();
    assert_eq!(ratio.cells, vec![vec![Cell::empty(), Cell::empty()], vec![Cell::number(1.0), Cell::number(4.0)]]);
    let tsv = filter_delimited_text("Dose\tSignal\n1,234.56\t2,500.00\n", '\t', '.', true, &ImportFilterSpec::default()).unwrap();
    assert_eq!(tsv.cells, vec![vec![Cell::number(1234.56), Cell::number(2500.0)]]);
    let pred = ImportFilterSpec { exclude_row_predicate: Some(String::from("B > 10")), ..Default::default() };
    assert!(matches!(filter_delimited_text("A,B\n1,20\n", ',', '.', true, &pred), Err(FormatError::InvalidPipeline { .. })));
    let mut target = col_sheet("c1", 2, 2);
    let link = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("in.csv"), delimiter: ',', decimal_separator: '.' },
        filter: pred,
        placement: ImportPlacementSpec::default(),
    };
    assert!(matches!(
        apply_csv_import_link(&mut target, "A,B\n1,20\n", &link, true, ImportBaselineAdjustment::None, "lnk-1", "test"),
        Err(FormatError::InvalidPipeline { .. })
    ));
    assert!(target.linked_regions.is_empty());

    // Quoted asterisk & [excluded] escape round-trip + external CSV active quoted text.
    let cells = vec![
        Cell::text("\"hello\"*"),
        Cell::text("\"\"*"),
        Cell::text("[excluded]:literal"),
        Cell::text("[excluded]\\:literal"),
        Cell::excluded_text("\"hello\"*"),
        Cell::excluded_text("[excluded]:nested"),
        Cell::excluded_text("a,b"),
        Cell::excluded_text("NA"),
    ];
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, cells.len()).unwrap();
    col.data_sets[0].title = String::from("Labels");
    col.data_sets[0].cells = cells.iter().map(|c| vec![c.clone()]).collect();
    let opts = CsvOptions::csv();
    let exported = export_datasheet_to_csv(&DataSheet::new("s-q", "Q", TableArchetype::Column(col)).unwrap(), &opts).unwrap();
    parse_delimited_records(&exported, &opts).unwrap();
    let TableArchetype::Column(ref re_col) =
        import_csv_to_datasheet("s-q-re", "Re", &exported, TableKind::Column, SubcolumnMode::Single, &opts).unwrap().archetype
    else {
        panic!("expected Column")
    };
    assert_eq!(re_col.data_sets[0].cells.iter().map(|r| r[0].clone()).collect::<Vec<_>>(), cells);
    let ext = import_csv_to_datasheet("ext", "Ext", "Col\n\"\"\"label\"\"*\"\n", TableKind::Column, SubcolumnMode::Single, &opts).unwrap();
    assert_eq!(ext.cell(CellCoord::new(0, 0, 0)), Some(&Cell::text("\"label\"*")));

    // CsvOptions::detect stops early on tall (100k rows) and wide (50k cols) inputs.
    let mut tall = String::from("X;Y\n");
    for i in 0..100_000 {
        tall.push_str(&format!("{i},5;{},25\n", i + 1));
    }
    let t0 = Instant::now();
    let det = CsvOptions::detect(&tall);
    assert!((det.delimiter, det.decimal_separator) == (';', ',') && t0.elapsed().as_millis() < 500);
    let wide = (0..50_000).map(|i| format!("c{i}")).collect::<Vec<_>>().join(",");
    let t1 = Instant::now();
    assert!(CsvOptions::detect(&wide).delimiter == ',' && t1.elapsed().as_millis() < 500);
}

#[test]
fn multi_subcolumn_x_and_quoted_locale_numbers_roundtrip() {
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::MeanSdN, 2, 2).unwrap();
    (xy.x_title, xy.data_sets[0].title, xy.data_sets[1].title) = (String::from("Dose"), String::from("DrugA"), String::from("DrugB"));
    xy.x_cells = vec![vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)], vec![Cell::number(2.0), Cell::number(0.2), Cell::number(3.0)]];
    xy.data_sets[0].cells =
        vec![vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)], vec![Cell::number(20.0), Cell::number(2.0), Cell::number(3.0)]];
    xy.data_sets[1].cells =
        vec![vec![Cell::number(30.0), Cell::number(3.0), Cell::number(4.0)], vec![Cell::number(40.0), Cell::number(4.0), Cell::number(4.0)]];
    let csv = export_datasheet_to_csv(&DataSheet::new("xy-mx", "Multi X", TableArchetype::Xy(xy)).unwrap(), &CsvOptions::csv()).unwrap();
    let TableArchetype::Xy(ref re_xy) =
        import_csv_to_datasheet("xy-re", "Multi X", &csv, TableKind::Xy, SubcolumnMode::MeanSdN, &CsvOptions::csv()).unwrap().archetype
    else {
        panic!("expected Xy")
    };
    assert_eq!(re_xy.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN });
    assert_eq!(
        (re_xy.x_title.as_str(), re_xy.data_sets.len(), re_xy.data_sets[0].title.as_str(), re_xy.data_sets[1].title.as_str()),
        ("Dose", 2, "DrugA", "DrugB")
    );
    assert_eq!(
        (&re_xy.x_cells[0], &re_xy.data_sets[0].cells[0], &re_xy.data_sets[1].cells[1]),
        (
            &vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)],
            &vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)],
            &vec![Cell::number(40.0), Cell::number(4.0), Cell::number(4.0)]
        )
    );

    let TableArchetype::Xy(ref leg_xy) = import_csv_to_datasheet(
        "xy-leg",
        "Leg",
        "X,X:2,X:3,A:Mean,A:SD,A:N\n1,0.1,3,10,1,3\n",
        TableKind::Xy,
        SubcolumnMode::MeanSdN,
        &CsvOptions::csv(),
    )
    .unwrap()
    .archetype
    else {
        panic!("expected Xy")
    };
    assert_eq!(leg_xy.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(3) });
    assert_eq!(
        (leg_xy.data_sets.len(), leg_xy.data_sets[0].title.as_str(), &leg_xy.x_cells[0], &leg_xy.data_sets[0].cells[0]),
        (1, "A", &vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)], &vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)])
    );

    let xlsx_rows = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Dose:Mean</t></is></c><c r="B1" t="inlineStr"><is><t>Dose:SD</t></is></c><c r="C1" t="inlineStr"><is><t>Dose:N</t></is></c><c r="D1" t="inlineStr"><is><t>DrugA:Mean</t></is></c><c r="E1" t="inlineStr"><is><t>DrugA:SD</t></is></c><c r="F1" t="inlineStr"><is><t>DrugA:N</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>0.1</v></c><c r="C2"><v>3</v></c><c r="D2"><v>10</v></c><c r="E2"><v>1</v></c><c r="F2"><v>3</v></c></row>"#;
    let TableArchetype::Xy(ref x_xy) = import_xlsx_bytes_with_options(
        &xlsx_single_sheet_zip("MultiX", xlsx_rows),
        &XlsxImportOptions { table_kind: Some(TableKind::Xy), subcolumn_mode: SubcolumnMode::MeanSdN, ..Default::default() },
    )
    .unwrap()
    .document
    .data_sheets[0]
        .archetype
    else {
        panic!("expected Xy")
    };
    assert_eq!(
        (&x_xy.x_mode, x_xy.data_sets.len(), &x_xy.x_cells[0], &x_xy.data_sets[0].cells[0]),
        (
            &XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN },
            1,
            &vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)],
            &vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)]
        )
    );

    let no_hdr = CsvOptions { header_rows: 0, ..CsvOptions::csv() };
    let TableArchetype::Xy(ref h_xy) =
        import_csv_to_datasheet_with_x_mode("xy-h", "H", "1,0.1,3,10\n", TableKind::Xy, SubcolumnMode::Single, Some(SubcolumnMode::MeanSdN), &no_hdr)
            .unwrap()
            .archetype
    else {
        panic!("expected Xy")
    };
    assert_eq!(
        (&h_xy.x_cells[0], &h_xy.data_sets[0].cells[0]),
        (&vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)], &vec![Cell::number(10.0)])
    );

    // Quoted comma-grouped US/int and EU decimal-comma numbers.
    let us_csv = "X,Y\n10,950\n20,\"1,250.5\"\n30,\"2,500.0*\"\n40,\"a,b*\"\n";
    let us_opts = CsvOptions::detect(us_csv);
    assert_eq!((us_opts.delimiter, us_opts.decimal_separator, us_opts.thousands_separator), (',', '.', Some(',')));
    let us = import_csv_to_datasheet("us-q", "US", us_csv, TableKind::Xy, SubcolumnMode::Single, &us_opts).unwrap();
    for (r, exp) in [(0, Cell::number(950.0)), (1, Cell::number(1250.5)), (2, Cell::excluded_number(2500.0)), (3, Cell::text("a,b*"))] {
        assert_eq!(us.cell(CellCoord::new(r, 0, 0)), Some(&exp));
    }
    let int_csv = "X,Y\n10,\"1,250\"\n20,\"2,500\"\n";
    let int_opts = CsvOptions::detect(int_csv);
    assert_eq!(int_opts.thousands_separator, Some(','));
    let int_s = import_csv_to_datasheet("us-i", "Int", int_csv, TableKind::Xy, SubcolumnMode::Single, &int_opts).unwrap();
    assert_eq!(
        (int_s.cell(CellCoord::new(0, 0, 0)), int_s.cell(CellCoord::new(1, 0, 0))),
        (Some(&Cell::number(1250.0)), Some(&Cell::number(2500.0)))
    );
    let eu_csv = "X,Y\n10,\"1,25\"\n20,\"2,50\"\n";
    let eu_opts = CsvOptions::detect(eu_csv);
    assert_eq!((eu_opts.delimiter, eu_opts.decimal_separator), (',', ','));
    let eu = import_csv_to_datasheet("eu-q", "EU", eu_csv, TableKind::Xy, SubcolumnMode::Single, &eu_opts).unwrap();
    assert_eq!((eu.cell(CellCoord::new(0, 0, 0)), eu.cell(CellCoord::new(1, 0, 0))), (Some(&Cell::number(1.25)), Some(&Cell::number(2.50))));
}
