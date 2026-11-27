//! Comprehensive clean-room synthetic tests for full external `.prism` import (§5.1, §5.2, §5.3),
//! analysis translation & cached-vs-recomputed diagnostics, preview ingestion, and the experimental
//! `.prism` writer (§6.3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, DescriptiveSpec,
    GroupedTable, InfoConstant, InfoSheet, LayoutItem, MultipleVariablesTable, MultiplicitySpec, NestedTable, PairwiseComparisonRecord,
    PartsOfWholeTable, ProjectDocument, ResultCell, ResultProvenance, ResultScalarEntry, ResultSheet, ResultStatus, SequenceProgression, SheetId,
    SubcolumnMode, SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable,
};
use curvebook_format::{
    ANALYSIS_MAPPINGS, AnalysisDesignKey, AnalysisVerificationStatus, ArchiveExportOptions, CACHED_PAIRWISE_LINKS, CACHED_SCALAR_LINKS, CachedBound,
    CorpusDesignEvidence, DesignTail, ExternalDataSetPositions, FormatError, ImportWarningKind, MIN_VERIFIED_DESIGN_AGREEMENT_DEN,
    MIN_VERIFIED_DESIGN_AGREEMENT_NUM, MIN_VERIFIED_DESIGN_ANALYSES, MIN_VERIFIED_DESIGN_SOURCES, VERIFIED_ANALYSIS_DESIGNS, ZipArchive,
    cached_values_for_link, check_control_in_selection, check_source_sheet_option_compatibility, compare_recomputed_against_cached_results,
    compare_recomputed_against_reference, export_prism_bytes_experimental, export_prism_bytes_with_options, extract_analysis_design_key,
    extract_cached_analysis_reference, find_analysis_mapping, find_verified_analysis_design, import_prism_bytes, resolve_selected_data_sets,
    translate_analysis_parameters, try_export_result_sheet_tidy_csv,
};
use curvebook_palette::Rgba8;
use serde_json::json;

use super::common::{RawZipEntry, build_raw_zip, build_zip_entries, encode_rgba_png};

fn json_bytes(v: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&v).unwrap()
}

fn build_zip_owned(entries: Vec<(String, Vec<u8>)>) -> Vec<u8> {
    let refs: Vec<(&str, &[u8])> = entries.iter().map(|(k, v)| (k.as_str(), v.as_slice())).collect();
    build_zip_entries(&refs)
}

#[test]
fn external_archive_headerless_csv_row_titles_before_x_series_timespan_xerr_and_trailing_empty() {
    let doc_json = json_bytes(json!({
        "$id": "doc-1",
        "@class": "Document",
        "formatVersion": "1-6-0",
        "title": "Empirical Layout Study",
        "sheets": {
            "data": ["sh-rt-x", "sh-series", "sh-time", "sh-xerr"],
            "info": [],
            "analyses": [],
            "graphs": [],
            "layouts": []
        },
        "sheetAttributesMap": {
            "sh-rt-x": { "title": "RowTitles Before X", "highlightColor": "#336699", "isFrozen": true },
            "sh-series": { "title": "Series X Zero CSV Cols" },
            "sh-time": { "title": "Elapsed Time XY" },
            "sh-xerr": { "title": "Horizontal X Error" }
        },
        "uiSettings": { "currentSheets": ["sh-series"] }
    }));

    // 1. Row titles FIRST, then X, then 2 Y columns (with trailing empty Y col omitted on row 1).
    let sh_rt_x = json_bytes(json!({
        "uid": "sh-rt-x",
        "title": "Ignored Fallback",
        "flags": [],
        "table": {
            "uid": "tbl-rt-x",
            "format": "xy",
            "dataFormat": "y_single",
            "rowTitlesDataSet": "ds-rt",
            "xDataSet": "ds-x1",
            "xDataSetFormat": "y_single",
            "dataSets": ["ds-y1", "ds-y2"]
        }
    }));
    // 2. `xDataSetFormat == "series"` occupies 0 columns in `data.csv`!
    let sh_series = json_bytes(json!({
        "uid": "sh-series",
        "title": "Series X Zero CSV Cols",
        "table": {
            "uid": "tbl-series",
            "format": "xy",
            "dataFormat": "y_single",
            "xDataSet": "ds-x-series",
            "xDataSetFormat": "series",
            "dataSets": ["ds-ys"]
        }
    }));
    // 3. `xDataSetFormat == "timeSpan"` (1 X col + units).
    let sh_time = json_bytes(json!({
        "uid": "sh-time",
        "title": "Elapsed Time XY",
        "dateTimeConversion": { "units": "Minutes" },
        "table": {
            "uid": "tbl-time",
            "format": "xy",
            "dataFormat": "y_single",
            "xDataSet": "ds-x-time",
            "xDataSetFormat": "timeSpan",
            "dataSets": ["ds-yt"]
        }
    }));
    // 4. `xDataSetFormat == "y_errorBar"` (2 X cols: [X, X_err]).
    let sh_xerr = json_bytes(json!({
        "uid": "sh-xerr",
        "title": "Horizontal X Error",
        "table": {
            "uid": "tbl-xerr",
            "format": "xy",
            "dataFormat": "y_single",
            "xDataSet": "ds-x-err",
            "xDataSetFormat": "y_errorBar",
            "dataSets": ["ds-ye"]
        }
    }));

    let zip = build_zip_owned(vec![
        ("document.json".into(), doc_json),
        ("data/sheets/sh-rt-x/sheet.json".into(), sh_rt_x),
        ("data/sheets/sh-series/sheet.json".into(), sh_series),
        ("data/sheets/sh-time/sheet.json".into(), sh_time),
        ("data/sheets/sh-xerr/sheet.json".into(), sh_xerr),
        ("data/sets/ds-rt.json".into(), json_bytes(json!({"title": "Sample", "replicates": [{"valueType": "string"}]}))),
        ("data/sets/ds-x1.json".into(), json_bytes(json!({"title": "Dose", "replicates": [{"valueType": "real", "decimalsLength": 2}]}))),
        ("data/sets/ds-y1.json".into(), json_bytes(json!({"title": "Ctrl", "replicates": [{"valueType": "real", "decimalsLength": 3}]}))),
        ("data/sets/ds-y2.json".into(), json_bytes(json!({"title": "Drug", "replicates": [{"valueType": "real"}]}))),
        (
            "data/sets/ds-x-series.json".into(),
            json_bytes(json!({"title": "Cycle", "format": "series", "replicates": [{"valueType": "real", "startValue": 10.0, "interval": 2.5}]})),
        ),
        ("data/sets/ds-ys.json".into(), json_bytes(json!({"title": "Signal", "replicates": [{"valueType": "real"}]}))),
        (
            "data/sets/ds-x-time.json".into(),
            json_bytes(json!({"title": "Time", "dateTimeConversion": {"units": "Hours"}, "replicates": [{"valueType": "timespan"}]})),
        ),
        ("data/sets/ds-yt.json".into(), json_bytes(json!({"title": "OD", "replicates": [{"valueType": "real"}]}))),
        ("data/sets/ds-x-err.json".into(), json_bytes(json!({"title": "Conc", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        ("data/sets/ds-ye.json".into(), json_bytes(json!({"title": "Resp", "replicates": [{"valueType": "real"}]}))),
        // Row titles FIRST, then X, then Y1, then Y2 (omitted on row 1 -> padded as Empty).
        ("data/tables/tbl-rt-x/data.csv".into(), b"Sample A,1.0,10.0,20.0\nSample B,2.0,15.0\n".to_vec()),
        // Series X has 0 columns in CSV: only the single Y column appears!
        ("data/tables/tbl-series/data.csv".into(), b"100.0\n150.0\n200.0\n".to_vec()),
        ("data/tables/tbl-time/data.csv".into(), b"0.5,0.12\n1.5,0.48\n".to_vec()),
        ("data/tables/tbl-xerr/data.csv".into(), b"5.0,0.2,40.0\n10.0,0.4,80.0\n".to_vec()),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    let doc = &report.document;
    assert_eq!(doc.title, "Empirical Layout Study");
    assert_eq!(doc.data_sheets.len(), 4);
    assert_eq!(doc.active_sheet_id, Some(SheetId::new("sh-series")));

    // Verify RowTitles-before-X + trailing empty padding + color + frozen + decimals.
    let s1 = doc.data_sheet(&SheetId::new("sh-rt-x")).unwrap();
    assert_eq!(s1.title, "RowTitles Before X");
    assert!(s1.frozen);
    assert_eq!(s1.color_tag, Some(Rgba8::rgb(0x33, 0x66, 0x99)));
    let TableArchetype::Xy(xy1) = &s1.archetype else { panic!("expected Xy") };
    assert_eq!(xy1.row_titles, vec!["Sample A", "Sample B"]);
    assert!(xy1.row_titles_expanded);
    assert_eq!(xy1.x_title, "Dose");
    assert_eq!(xy1.x_display_decimals, Some(2));
    assert_eq!(xy1.x_value_at(0), Some(1.0));
    assert_eq!(xy1.x_value_at(1), Some(2.0));
    assert_eq!(xy1.data_sets[0].display_decimals, Some(3));
    assert_eq!(xy1.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(10.0));
    assert_eq!(xy1.data_sets[1].cell(0, 0).unwrap().value, CellValue::Number(20.0));
    assert_eq!(xy1.data_sets[1].cell(1, 0).unwrap().value, CellValue::Empty);

    // Verify Series X occupies 0 CSV columns and synthesizes GeneratedSequence!
    let s2 = doc.data_sheet(&SheetId::new("sh-series")).unwrap();
    let TableArchetype::Xy(xy2) = &s2.archetype else { panic!("expected Xy") };
    assert_eq!(xy2.x_mode, XColumnMode::GeneratedSequence { start: 10.0, step: 2.5, progression: SequenceProgression::Arithmetic });
    assert_eq!(xy2.x_value_at(0), Some(10.0));
    assert_eq!(xy2.x_value_at(1), Some(12.5));
    assert_eq!(xy2.x_value_at(2), Some(15.0));
    assert_eq!(xy2.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(100.0));
    assert_eq!(xy2.data_sets[0].cell(2, 0).unwrap().value, CellValue::Number(200.0));

    // Verify timeSpan X.
    let s3 = doc.data_sheet(&SheetId::new("sh-time")).unwrap();
    let TableArchetype::Xy(xy3) = &s3.archetype else { panic!("expected Xy") };
    assert_eq!(xy3.x_mode, XColumnMode::DateOrElapsedTime { unit_label: String::from("Hours") });
    assert_eq!(xy3.x_value_at(1), Some(1.5));

    // Verify y_errorBar X.
    let s4 = doc.data_sheet(&SheetId::new("sh-xerr")).unwrap();
    let TableArchetype::Xy(xy4) = &s4.archetype else { panic!("expected Xy") };
    assert_eq!(xy4.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSd });
    assert_eq!(xy4.x_cells[0][0].value, CellValue::Number(5.0));
    assert_eq!(xy4.x_cells[0][1].value, CellValue::Number(0.2));
    assert_eq!(xy4.data_sets[0].cell(1, 0).unwrap().value, CellValue::Number(80.0));
}

#[test]
fn external_archive_all_eight_archetypes_view_exclusion_exclusions_rich_text_and_info_links() {
    let doc_json = json_bytes(json!({
        "formatVersion": "1-6-1",
        "title": "Eight Archetypes",
        "sheets": {
            "data": ["s-xy", "s-col", "s-grp", "s-surv", "s-cont", "s-mv", "s-nest", "s-pow", "s-view"],
            "info": ["info-1"],
            "analyses": [],
            "graphs": [],
            "layouts": []
        },
        "sheetAttributesMap": {
            "s-xy": { "title": "XY" },
            "s-col": { "title": "Column" },
            "s-grp": { "title": "Grouped" },
            "s-surv": { "title": "Survival" },
            "s-cont": { "title": "Contingency" },
            "s-mv": { "title": "Multivar" },
            "s-nest": { "title": "Nested" },
            "s-pow": { "title": "Parts" },
            "s-view": { "title": "Internal Analysis View" },
            "info-1": { "title": "Experiment Info", "highlightColor": "#AA5500" }
        }
    }));

    let rich_title = json!({
        "string": "Ca2+ Flux",
        "version": 1,
        "formatting": [
            { "range": "2~3", "attributes": { "baseline": "superscript" } },
            { "range": "5~8", "attributes": { "fontWeight": "bold" } }
        ]
    });

    let zip = build_zip_owned(vec![
        ("document.json".into(), doc_json),
        // 1. XY
        (
            "data/sheets/s-xy/sheet.json".into(),
            json_bytes(json!({"table": {"uid": "t-xy", "format": "xy", "dataFormat": "y_single", "xDataSet": "dx-xy", "dataSets": ["dy-xy"]}})),
        ),
        ("data/sets/dx-xy.json".into(), json_bytes(json!({"title": "X", "replicates": [{"valueType": "real"}]}))),
        ("data/sets/dy-xy.json".into(), json_bytes(json!({"title": rich_title, "replicates": [{"valueType": "real"}]}))),
        ("data/tables/t-xy/data.csv".into(), b"1,10\n2,20\n".to_vec()),
        // 2. Column with EXCLUDED via cellAttributes
        (
            "data/sheets/s-col/sheet.json".into(),
            json_bytes(json!({"table": {"uid": "t-col", "format": "column", "dataFormat": "y_single", "dataSets": ["dy-col"]}})),
        ),
        (
            "data/sets/dy-col.json".into(),
            json_bytes(json!({
                "title": "Arm A",
                "replicates": [{
                    "valueType": "real",
                    "cellAttributes": [{ "rows": "1", "attributes": ["EXCLUDED"] }]
                }]
            })),
        ),
        ("data/tables/t-col/data.csv".into(), b"5.0\n999.0\n6.0\n".to_vec()),
        // 3. Grouped with `"replicate ranges"` + subcolumn titles
        (
            "data/sheets/s-grp/sheet.json".into(),
            json_bytes(json!({"table": {
                "uid": "t-grp", "format": "grouped", "dataFormat": "y_replicates", "replicatesCount": 2,
                "rowTitlesDataSet": "rt-grp", "subcolumnTitlesDataSet": "st-grp", "dataSets": ["dy-grp"]
            }})),
        ),
        ("data/sets/rt-grp.json".into(), json_bytes(json!({"title": "Time", "replicates": [{"valueType": "string"}]}))),
        (
            "data/sets/dy-grp.json".into(),
            json_bytes(json!({
                "title": "Treated",
                "replicate ranges": [{
                    "range": "0~1",
                    "replicate": { "valueType": "real", "cellAttributes": [{ "rows": "0", "attributes": ["EXCLUDED"] }] }
                }]
            })),
        ),
        (
            "data/sets/st-grp.json".into(),
            json_bytes(
                json!({"@class": "SubColumnTitlesDataSet", "titles": [{"column": 0, "replicates": [{"replicate": 0, "name": "Rep A"}, {"replicate": 1, "name": "Rep B"}]}]}),
            ),
        ),
        ("data/tables/t-grp/data.csv".into(), b"Day 1,10,12\nDay 2,20,22\n".to_vec()),
        // 4. Survival with CENSORED cellAttributes
        (
            "data/sheets/s-surv/sheet.json".into(),
            json_bytes(
                json!({"table": {"uid": "t-surv", "format": "survival", "dataFormat": "y_single", "xDataSet": "dx-surv", "dataSets": ["dy-surv"]}}),
            ),
        ),
        ("data/sets/dx-surv.json".into(), json_bytes(json!({"title": "Days", "replicates": [{"valueType": "real"}]}))),
        (
            "data/sets/dy-surv.json".into(),
            json_bytes(
                json!({"title": "Cohort A", "replicates": [{"valueType": "int", "cellAttributes": [{"rows": "1", "attributes": ["CENSORED"]}]}]}),
            ),
        ),
        ("data/tables/t-surv/data.csv".into(), b"5,1\n12,\n".to_vec()),
        // 5. Contingency
        (
            "data/sheets/s-cont/sheet.json".into(),
            json_bytes(
                json!({"table": {"uid": "t-cont", "format": "contingency", "dataFormat": "y_single", "rowTitlesDataSet": "rt-cont", "dataSets": ["dy-c1", "dy-c2"]}}),
            ),
        ),
        ("data/sets/rt-cont.json".into(), json_bytes(json!({"title": "Outcome", "replicates": [{"valueType": "string"}]}))),
        ("data/sets/dy-c1.json".into(), json_bytes(json!({"title": "Yes", "replicates": [{"valueType": "int"}]}))),
        ("data/sets/dy-c2.json".into(), json_bytes(json!({"title": "No", "replicates": [{"valueType": "int"}]}))),
        ("data/tables/t-cont/data.csv".into(), b"Exposed,12,8\nControl,4,16\n".to_vec()),
        // 6. MultipleVariables
        (
            "data/sheets/s-mv/sheet.json".into(),
            json_bytes(json!({"table": {"uid": "t-mv", "format": "multivariable", "dataFormat": "y_single", "dataSets": ["dy-mv1", "dy-mv2"]}})),
        ),
        ("data/sets/dy-mv1.json".into(), json_bytes(json!({"title": "Strain", "replicates": [{"valueType": "string"}]}))),
        ("data/sets/dy-mv2.json".into(), json_bytes(json!({"title": "Mass", "replicates": [{"valueType": "real", "decimalsLength": 1}]}))),
        ("data/tables/t-mv/data.csv".into(), b"WT,22.4\nKO,19.8\n".to_vec()),
        // 7. Nested
        (
            "data/sheets/s-nest/sheet.json".into(),
            json_bytes(
                json!({"table": {"uid": "t-nest", "format": "nested", "dataFormat": "y_replicates", "replicatesCount": 2, "subcolumnTitlesDataSet": "st-nest", "dataSets": ["dy-nest"]}}),
            ),
        ),
        ("data/sets/dy-nest.json".into(), json_bytes(json!({"title": "Group A", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        (
            "data/sets/st-nest.json".into(),
            json_bytes(
                json!({"@class": "SubColumnTitlesDataSet", "titles": [{"column": 0, "replicates": [{"replicate": 0, "name": "Mouse 1"}, {"replicate": 1, "name": "Mouse 2"}]}]}),
            ),
        ),
        ("data/tables/t-nest/data.csv".into(), b"1.1,2.1\n1.3,2.3\n".to_vec()),
        // 8. PartsOfWhole
        (
            "data/sheets/s-pow/sheet.json".into(),
            json_bytes(
                json!({"table": {"uid": "t-pow", "format": "parts_of_whole", "dataFormat": "y_single", "rowTitlesDataSet": "rt-pow", "dataSets": ["dy-pow"]}}),
            ),
        ),
        ("data/sets/rt-pow.json".into(), json_bytes(json!({"title": "Slice", "replicates": [{"valueType": "string"}]}))),
        ("data/sets/dy-pow.json".into(), json_bytes(json!({"title": "Share", "replicates": [{"valueType": "real"}]}))),
        ("data/tables/t-pow/data.csv".into(), b"Alpha,30\nBeta,70\n".to_vec()),
        // 9. Analysis View — MUST NOT be imported as a DataSheet!
        (
            "data/sheets/s-view/sheet.json".into(),
            json_bytes(
                json!({"flags": ["ANALYSIS_VIEW"], "table": {"uid": "t-view", "format": "view", "dataFormat": "text", "dataSets": ["dy-view"]}}),
            ),
        ),
        ("data/sets/dy-view.json".into(), json_bytes(json!({"title": "Col", "replicates": [{"valueType": "string"}]}))),
        ("data/tables/t-view/data.csv".into(), b"P value,0.001\n".to_vec()),
        // Info sheet with ISO-8601 experimentDate, polymorphic rich-text notes & constants, and linkedDataSheetID
        (
            "info/info-1/sheet.json".into(),
            json_bytes(json!({
                "uid": "info-1",
                "title": "Experiment Info",
                "experimentDate": "2026-11-26",
                "linkedDataSheetID": "s-xy",
                "notesText": { "string": "Protocol A", "version": 1, "formatting": [{ "range": "0~7", "attributes": { "fontWeight": "bold" } }] },
                "constants": [
                    { "name": { "string": "Conc", "version": 1, "formatting": [] }, "value": "25.5" },
                    { "name": "Reassay Date", "value": "2026-11-27" }
                ]
            })),
        ),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    let doc = &report.document;
    // Exactly 8 data sheets: `s-view` was skipped!
    assert_eq!(doc.data_sheets.len(), 8);
    assert!(doc.data_sheet(&SheetId::new("s-view")).is_none());

    let kinds: Vec<TableKind> = doc.data_sheets.iter().map(|s| s.archetype.kind()).collect();
    assert_eq!(
        kinds,
        vec![
            TableKind::Xy,
            TableKind::Column,
            TableKind::Grouped,
            TableKind::Survival,
            TableKind::Contingency,
            TableKind::MultipleVariables,
            TableKind::Nested,
            TableKind::PartsOfWhole,
        ]
    );

    // Rich text title on XY dataset
    let TableArchetype::Xy(xy) = &doc.data_sheet(&SheetId::new("s-xy")).unwrap().archetype else { panic!() };
    assert_eq!(xy.data_sets[0].title, "Ca<sup>2+</sup> <b>Flux</b>");

    // Excluded mask on Column row 1
    let TableArchetype::Column(col) = &doc.data_sheet(&SheetId::new("s-col")).unwrap().archetype else { panic!() };
    assert!(!col.data_sets[0].cell(0, 0).unwrap().excluded);
    assert!(col.data_sets[0].cell(1, 0).unwrap().excluded);
    assert_eq!(col.data_sets[0].cell(1, 0).unwrap().value, CellValue::Number(999.0));

    // Grouped replicate ranges + subcolumn titles
    let TableArchetype::Grouped(grp) = &doc.data_sheet(&SheetId::new("s-grp")).unwrap().archetype else { panic!() };
    assert_eq!(grp.data_sets[0].subcolumn_titles, vec!["Rep A", "Rep B"]);
    assert!(grp.data_sets[0].cell(0, 0).unwrap().excluded);
    assert!(grp.data_sets[0].cell(0, 1).unwrap().excluded);
    assert!(!grp.data_sets[0].cell(1, 0).unwrap().excluded);

    // Survival censored cell populated with censored_code = 0
    let TableArchetype::Survival(surv) = &doc.data_sheet(&SheetId::new("s-surv")).unwrap().archetype else { panic!() };
    assert_eq!(surv.cohorts[0].cell(0, 0).unwrap().value, CellValue::Number(1.0));
    assert_eq!(surv.cohorts[0].cell(1, 0).unwrap().value, CellValue::Number(0.0));

    // Nested custom subcolumn titles
    let TableArchetype::Nested(nest) = &doc.data_sheet(&SheetId::new("s-nest")).unwrap().archetype else { panic!() };
    assert_eq!(nest.groups[0].subcolumn_titles, vec!["Mouse 1", "Mouse 2"]);

    // Info sheet ISO-8601 Date, rich notes, constants, color, and bidirectional link to `s-xy`
    assert_eq!(doc.info_sheets.len(), 1);
    let info = &doc.info_sheets[0];
    assert_eq!(info.color_tag, Some(Rgba8::rgb(0xAA, 0x55, 0x00)));
    assert_eq!(info.notes_markdown, "<b>Protocol</b> A");
    assert_eq!(info.linked_data_sheets, vec![SheetId::new("s-xy")]);
    assert_eq!(doc.data_sheet(&SheetId::new("s-xy")).unwrap().info_sheet_id, Some(info.id.clone()));
    assert_eq!(info.constants[0].name, "Experiment Date");
    assert_eq!(info.constants[0].value, ConstantValue::Date(String::from("2026-11-26")));
    assert_eq!(info.constants[1].value, ConstantValue::Number(25.5));
    assert_eq!(info.constants[2].value, ConstantValue::Date(String::from("2026-11-27")));
}

#[test]
fn external_archive_data_formats_cv_to_sd_high_low_offset_swap_and_unknown_rejection() {
    let doc_json = json_bytes(json!({
        "formatVersion": "1-6-0",
        "sheets": {
            "data": ["s-hl", "s-pm2", "s-pm3", "s-cvn", "s-sd", "s-se", "s-cv", "s-bad-rep", "s-unknown"]
        }
    }));

    let mk_xy_sheet = |uid: &str, t_uid: &str, fmt: &str, ds: &str, reps: Option<usize>| {
        let mut tbl = json!({
            "uid": t_uid, "format": "xy", "dataFormat": fmt, "xDataSet": "dx", "dataSets": [ds]
        });
        if let Some(r) = reps {
            tbl["replicatesCount"] = json!(r);
        }
        json_bytes(json!({ "uid": uid, "title": uid, "table": tbl }))
    };

    let zip = build_zip_owned(vec![
        ("document.json".into(), doc_json),
        ("data/sets/dx.json".into(), json_bytes(json!({"title": "X", "replicates": [{"valueType": "real"}]}))),
        // y_high_low: `.prism` stores `[Y=10.0, +Err=4.0, -Err=2.0]` -> CurveBook `[Mean=10.0, -Err=2.0, +Err=4.0]`!
        ("data/sheets/s-hl/sheet.json".into(), mk_xy_sheet("s-hl", "t-hl", "y_high_low", "dy-hl", None)),
        (
            "data/sets/dy-hl.json".into(),
            json_bytes(json!({"title": "Asym", "replicates": [{"valueType": "real"}, {"valueType": "real"}, {"valueType": "real"}]})),
        ),
        ("data/tables/t-hl/data.csv".into(), b"1,10.0,4.0,2.0\n".to_vec()),
        // y_plus_minus (2 CSV subcols -> duplicate symmetric error)
        ("data/sheets/s-pm2/sheet.json".into(), mk_xy_sheet("s-pm2", "t-pm2", "y_plus_minus", "dy-pm2", None)),
        ("data/sets/dy-pm2.json".into(), json_bytes(json!({"title": "Sym", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        ("data/tables/t-pm2/data.csv".into(), b"1,50.0,3.5\n".to_vec()),
        // y_plus_minus (3 CSV subcols, corpus-observed -> swap `[Y, +Err, -Err]` -> `[Mean, -Err, +Err]`)
        ("data/sheets/s-pm3/sheet.json".into(), mk_xy_sheet("s-pm3", "t-pm3", "y_plus_minus", "dy-pm3", None)),
        (
            "data/sets/dy-pm3.json".into(),
            json_bytes(json!({"title": "Pm3", "replicates": [{"valueType": "real"}, {"valueType": "real"}, {"valueType": "real"}]})),
        ),
        ("data/tables/t-pm3/data.csv".into(), b"1,20.0,5.0,1.5\n".to_vec()),
        // y_cv_n -> MeanSdN (lossless SD)
        ("data/sheets/s-cvn/sheet.json".into(), mk_xy_sheet("s-cvn", "t-cvn", "y_cv_n", "dy-cvn", None)),
        (
            "data/sets/dy-cvn.json".into(),
            json_bytes(json!({"title": "CvN", "replicates": [{"valueType": "real"}, {"valueType": "real"}, {"valueType": "int"}]})),
        ),
        ("data/tables/t-cvn/data.csv".into(), b"1,100.0,8.0,5\n".to_vec()),
        // y_sd -> MeanSd, y_se -> MeanSem, y_cv -> MeanSd
        ("data/sheets/s-sd/sheet.json".into(), mk_xy_sheet("s-sd", "t-sd", "y_sd", "dy-sd", None)),
        ("data/sets/dy-sd.json".into(), json_bytes(json!({"title": "SD", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        ("data/tables/t-sd/data.csv".into(), b"1,12.0,1.1\n".to_vec()),
        ("data/sheets/s-se/sheet.json".into(), mk_xy_sheet("s-se", "t-se", "y_se", "dy-se", None)),
        ("data/sets/dy-se.json".into(), json_bytes(json!({"title": "SE", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        ("data/tables/t-se/data.csv".into(), b"1,12.0,0.4\n".to_vec()),
        ("data/sheets/s-cv/sheet.json".into(), mk_xy_sheet("s-cv", "t-cv", "y_cv", "dy-cv", None)),
        ("data/sets/dy-cv.json".into(), json_bytes(json!({"title": "CV", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        ("data/tables/t-cv/data.csv".into(), b"1,12.0,0.9\n".to_vec()),
        // y_replicates with replicatesCount=0 -> skipped with MissingMetadata
        ("data/sheets/s-bad-rep/sheet.json".into(), mk_xy_sheet("s-bad-rep", "t-bad-rep", "y_replicates", "dy-bad-rep", Some(0))),
        ("data/sets/dy-bad-rep.json".into(), json_bytes(json!({"title": "BadRep", "replicates": []}))),
        ("data/tables/t-bad-rep/data.csv".into(), b"1,5.0\n".to_vec()),
        // Unknown dataFormat -> skipped with UnsupportedTableType (never plain CSV fallback)
        ("data/sheets/s-unknown/sheet.json".into(), mk_xy_sheet("s-unknown", "t-unknown", "y_exotic_future", "dy-unk", None)),
        ("data/sets/dy-unk.json".into(), json_bytes(json!({"title": "Unk", "replicates": [{"valueType": "real"}]}))),
        ("data/tables/t-unknown/data.csv".into(), b"1,99.0\n".to_vec()),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    let doc = &report.document;
    assert_eq!(doc.data_sheets.len(), 7);
    assert!(doc.data_sheet(&SheetId::new("s-bad-rep")).is_none());
    assert!(doc.data_sheet(&SheetId::new("s-unknown")).is_none());
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "s-bad-rep"));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedTableType && w.source == "s-unknown"));

    // Verify y_high_low swap: `[10.0, +Err=4.0, -Err=2.0]` -> `[10.0, -Err=2.0, +Err=4.0]`
    let TableArchetype::Xy(hl) = &doc.data_sheet(&SheetId::new("s-hl")).unwrap().archetype else { panic!() };
    assert_eq!(hl.y_subcolumn_mode, SubcolumnMode::MeanLowHigh);
    assert_eq!(hl.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(10.0));
    assert_eq!(hl.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(2.0));
    assert_eq!(hl.data_sets[0].cell(0, 2).unwrap().value, CellValue::Number(4.0));

    // Verify y_plus_minus 2-col duplication
    let TableArchetype::Xy(pm2) = &doc.data_sheet(&SheetId::new("s-pm2")).unwrap().archetype else { panic!() };
    assert_eq!(pm2.y_subcolumn_mode, SubcolumnMode::MeanLowHigh);
    assert_eq!(pm2.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(3.5));
    assert_eq!(pm2.data_sets[0].cell(0, 2).unwrap().value, CellValue::Number(3.5));

    // Verify y_plus_minus 3-col swap
    let TableArchetype::Xy(pm3) = &doc.data_sheet(&SheetId::new("s-pm3")).unwrap().archetype else { panic!() };
    assert_eq!(pm3.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(1.5));
    assert_eq!(pm3.data_sets[0].cell(0, 2).unwrap().value, CellValue::Number(5.0));

    // Verify y_cv_n -> MeanSdN, y_sd -> MeanSd, y_se -> MeanSem, y_cv -> MeanSd
    let TableArchetype::Xy(cvn) = &doc.data_sheet(&SheetId::new("s-cvn")).unwrap().archetype else { panic!() };
    assert_eq!(cvn.y_subcolumn_mode, SubcolumnMode::MeanSdN);
    assert_eq!(cvn.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(8.0));

    let TableArchetype::Xy(sd) = &doc.data_sheet(&SheetId::new("s-sd")).unwrap().archetype else { panic!() };
    assert_eq!(sd.y_subcolumn_mode, SubcolumnMode::MeanSd);
    let TableArchetype::Xy(se) = &doc.data_sheet(&SheetId::new("s-se")).unwrap().archetype else { panic!() };
    assert_eq!(se.y_subcolumn_mode, SubcolumnMode::MeanSem);
    let TableArchetype::Xy(cv) = &doc.data_sheet(&SheetId::new("s-cv")).unwrap().archetype else { panic!() };
    assert_eq!(cv.y_subcolumn_mode, SubcolumnMode::MeanSd);
}

#[test]
fn external_archive_whole_segment_id_matching_prevents_prefix_collision_and_skips_ambiguous() {
    let doc_json = json_bytes(json!({
        "sheets": { "data": ["id1", "id10", "id-ambig", "id-missing-ds"] }
    }));

    let zip = build_zip_owned(vec![
        ("document.json".into(), doc_json),
        // Prefix pair: `id1` vs `id10`, `ds1` vs `ds10`
        (
            "data/sheets/id1/sheet.json".into(),
            json_bytes(json!({"title": "One", "table": {"uid": "t1", "format": "column", "dataFormat": "y_single", "dataSets": ["ds1"]}})),
        ),
        (
            "data/sheets/id10/sheet.json".into(),
            json_bytes(json!({"title": "Ten", "table": {"uid": "t10", "format": "column", "dataFormat": "y_single", "dataSets": ["ds10"]}})),
        ),
        ("data/sets/ds1.json".into(), json_bytes(json!({"title": "Col1", "replicates": [{"valueType": "real"}]}))),
        ("data/sets/ds10.json".into(), json_bytes(json!({"title": "Col10", "replicates": [{"valueType": "real"}]}))),
        ("data/tables/t1/data.csv".into(), b"11\n".to_vec()),
        ("data/tables/t10/data.csv".into(), b"1010\n".to_vec()),
        // Ambiguous sheet UID: two `sheet.json` files both have segment `id-ambig`
        (
            "a/data/sheets/id-ambig/sheet.json".into(),
            json_bytes(json!({"title": "DupA", "table": {"uid": "ta", "format": "column", "dataFormat": "y_single", "dataSets": ["ds1"]}})),
        ),
        (
            "b/data/sheets/id-ambig/sheet.json".into(),
            json_bytes(json!({"title": "DupB", "table": {"uid": "tb", "format": "column", "dataFormat": "y_single", "dataSets": ["ds1"]}})),
        ),
        // Missing dataset UID
        (
            "data/sheets/id-missing-ds/sheet.json".into(),
            json_bytes(
                json!({"title": "Broken", "table": {"uid": "t-miss", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-does-not-exist"]}}),
            ),
        ),
        ("data/tables/t-miss/data.csv".into(), b"42\n".to_vec()),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    assert_eq!(report.document.data_sheets.len(), 2);
    let TableArchetype::Column(c1) = &report.document.data_sheet(&SheetId::new("id1")).unwrap().archetype else { panic!() };
    let TableArchetype::Column(c10) = &report.document.data_sheet(&SheetId::new("id10")).unwrap().archetype else { panic!() };
    assert_eq!(c1.data_sets[0].title, "Col1");
    assert_eq!(c1.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(11.0));
    assert_eq!(c10.data_sets[0].title, "Col10");
    assert_eq!(c10.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(1010.0));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "id-ambig"));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "ds-does-not-exist"));
}

#[test]
fn external_archive_analysis_mappings_translation_and_disagreement_diagnostics() {
    // 1. Verify all 11 corpus-observed mapping rows exist, have citations, and valid tolerances.
    assert_eq!(ANALYSIS_MAPPINGS.len(), 11);
    for row in ANALYSIS_MAPPINGS {
        assert!(!row.justification.trim().is_empty(), "{}", row.foreign_class);
        assert!(row.relative_tolerance > 0.0 && row.absolute_tolerance > 0.0, "{}", row.foreign_class);
        assert!(find_analysis_mapping(row.foreign_class).is_some());
    }
    // Every declared verified design must name a valid mapping class, satisfy the promotion bar,
    // carry a non-empty verification date & note, and have a unique canonical key.
    assert_eq!(VERIFIED_ANALYSIS_DESIGNS.len(), 10);
    let mut seen_keys = std::collections::HashSet::new();
    for design in VERIFIED_ANALYSIS_DESIGNS {
        assert!(find_analysis_mapping(design.foreign_class).is_some(), "{}", design.foreign_class);
        assert!(design.evidence.meets_bar(), "verified design fails promotion bar: {}", design.design_key());
        assert!(!design.evidence.verified_on.trim().is_empty(), "{}", design.design_key());
        assert!(!design.notes.trim().is_empty(), "{}", design.design_key());
        assert!(find_verified_analysis_design(&design.design_key()).is_some());
        assert!(seen_keys.insert(design.design_key()), "duplicate verified design key: {}", design.design_key());
    }

    // 2. Verify parameter translation across corpus spellings (`TRNSFORM`, `shaproWilks`, nonlinear equations).
    let tr_xform = translate_analysis_parameters(
        "TRNSFORM",
        &json!({"content": {"calc": {"transformType": "STANDARD", "standardFunction": {"xTransform": {"enabled": true, "function": "LOG10"}, "yTransform": {"enabled": false}}}}}),
    )
    .unwrap();
    assert!(tr_xform.blocking_options.is_empty());
    assert!(matches!(tr_xform.kind, AnalysisKind::Transform(_)));

    let tr_norm =
        translate_analysis_parameters("NORMALITY_TESTS", &json!({"content": {"calc": {"tests": {"shaproWilks": true, "andersonDarling": false}}}}))
            .unwrap();
    assert!(tr_norm.blocking_options.is_empty());
    assert!(matches!(tr_norm.kind, AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: true, .. })));

    let tr_nl_unmapped =
        translate_analysis_parameters("NONLINEAR_REGRESSION", &json!({"content": {"model": {"equation": {"name": "Custom Proprietary Model"}}}}))
            .unwrap();
    assert!(!tr_nl_unmapped.blocking_options.is_empty());
    assert_eq!(tr_nl_unmapped.verification, AnalysisVerificationStatus::Unverified);

    // The straight-line equation maps onto CurveBook's built-in `straight_line` model.
    let tr_line =
        translate_analysis_parameters("NONLINEAR_REGRESSION", &json!({"content": {"model": {"equation": {"name": "Straight line"}}}})).unwrap();
    assert!(matches!(&tr_line.kind, AnalysisKind::NonlinearRegression(nl) if nl.equation_id == "straight_line"), "{:?}", tr_line.kind);
}

#[test]
fn external_archive_design_key_normalization_and_evidence_bar() {
    // 1. Experimental design aliases normalize to a single canonical key.
    let alias_cases = [
        ("T_TESTS_AND_NONPARAMETRIC", "UNPAIRED_WELCH_T_TEST", "WELCH_T_TEST"),
        ("T_TESTS_AND_NONPARAMETRIC", "WELCH_T_TEST", "WELCH_T_TEST"),
        ("T_TESTS_AND_NONPARAMETRIC", "MANN_WHITNEY", "MANN_WHITNEY_TEST"),
        ("T_TESTS_AND_NONPARAMETRIC", "MANN_WHITNEY_TEST", "MANN_WHITNEY_TEST"),
        ("T_TESTS_AND_NONPARAMETRIC", "WILCOXON", "WILCOXON_TEST"),
        ("T_TESTS_AND_NONPARAMETRIC", "WILCOXON_MATCHED_PAIRS", "WILCOXON_TEST"),
        ("ONE_WAY_ANOVA", "BROWN_FORSYTHE_AND_WELCH", "BROWN_FORSYTHE_ANOVA"),
        ("ONE_WAY_ANOVA", "BROWN_FORSYTHE_WELCH", "BROWN_FORSYTHE_ANOVA"),
        ("ONE_WAY_ANOVA", "WELCH_ANOVA", "BROWN_FORSYTHE_ANOVA"),
        ("ONE_WAY_ANOVA", "KRUSKAL_WALLIS", "KRUSKAL_WALLIS_TEST"),
        ("ONE_WAY_ANOVA", "FRIEDMAN", "FRIEDMAN_TEST"),
        ("TWO_WAY_ANOVA", "REPEATED_MEASURES_BY_BOTH", "REPEATED_MEASURES_BY_BOTH_FACTORS"),
    ];
    for (class, raw_design, expected) in alias_cases {
        let key = extract_analysis_design_key(class, &json!({"content": {"experimentalDesign": raw_design}}));
        assert_eq!(key.experimental_design.as_deref(), Some(expected), "{class}/{raw_design}");
    }

    // 2. Tail normalization applies only to T_TESTS_AND_NONPARAMETRIC.
    let tt_default = extract_analysis_design_key("T_TESTS_AND_NONPARAMETRIC", &json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST"}}));
    let tt_two = extract_analysis_design_key(
        "T_TESTS_AND_NONPARAMETRIC",
        &json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}}),
    );
    let tt_one = extract_analysis_design_key(
        "T_TESTS_AND_NONPARAMETRIC",
        &json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_ONE_TAILED"}}}),
    );
    assert_eq!(tt_default.tail, DesignTail::TwoTailed);
    assert_eq!(tt_default, tt_two);
    assert_eq!(tt_one.tail, DesignTail::OneTailed);
    assert_ne!(tt_two.canonical_label(), tt_one.canonical_label());
    let anova_tail = extract_analysis_design_key(
        "ONE_WAY_ANOVA",
        &json!({"content": {"experimentalDesign": "ORDINARY_ANOVA", "options": {"pValueTailType": "P_VALUE_ONE_TAILED"}}}),
    );
    assert_eq!(anova_tail.tail, DesignTail::NotApplicable);

    // 3. Post-test normalization: NO_TESTS / NO_TEST / NONE -> (None, None); NO_CORRECTION stays distinct!
    let mk_post = |pt: serde_json::Value| json!({"content": {"experimentalDesign": "ORDINARY_ANOVA", "postTests": pt}});
    let no_post_absent = extract_analysis_design_key("ONE_WAY_ANOVA", &json!({"content": {"experimentalDesign": "ORDINARY_ANOVA"}}));
    let no_post_explicit = extract_analysis_design_key("ONE_WAY_ANOVA", &mk_post(json!({"type": "NO_TESTS"})));
    let no_post_stat_none = extract_analysis_design_key(
        "ONE_WAY_ANOVA",
        &mk_post(json!({
            "type": "PAIRWISE_TESTS",
            "pairwiseTests": {
                "pairwiseDesign": {"type": "ALL_VS_ALL"},
                "multipleComparisons": {"type": "STATISTICAL", "statistical": {"test": "NO_TEST"}}
            }
        })),
    );
    let no_post_fdr_none = extract_analysis_design_key(
        "ONE_WAY_ANOVA",
        &mk_post(json!({
            "type": "PAIRWISE_TESTS",
            "pairwiseTests": {
                "pairwiseDesign": {"type": "ALL_VS_ALL"},
                "multipleComparisons": {"type": "FDR", "fdr": {"test": "NONE"}}
            }
        })),
    );
    assert_eq!((no_post_absent.post_test_family.as_deref(), no_post_absent.post_test.as_deref()), (None, None));
    assert_eq!(no_post_absent, no_post_explicit);
    assert_eq!(no_post_absent, no_post_stat_none);
    assert_eq!(no_post_absent, no_post_fdr_none);

    let uncorrected = extract_analysis_design_key(
        "ONE_WAY_ANOVA",
        &mk_post(json!({
            "type": "PAIRWISE_TESTS",
            "pairwiseTests": {
                "pairwiseDesign": {"type": "ALL_VS_ALL"},
                "multipleComparisons": {"type": "NO_CORRECTION", "statistical": {"test": "TUKEY"}}
            }
        })),
    );
    assert_eq!((uncorrected.post_test_family.as_deref(), uncorrected.post_test.as_deref()), (Some("ALL_VS_ALL"), Some("NO_CORRECTION")));
    assert_ne!(uncorrected, no_post_absent);
    assert_ne!(uncorrected.canonical_label(), no_post_absent.canonical_label());

    let tukey = extract_analysis_design_key(
        "ONE_WAY_ANOVA",
        &mk_post(json!({
            "type": "PAIRWISE_TESTS",
            "pairwiseTests": {
                "pairwiseDesign": {"type": "ALL_VS_ALL"},
                "multipleComparisons": {"type": "STATISTICAL", "statistical": {"test": "TUKEY_HSD"}}
            }
        })),
    );
    assert_eq!((tukey.post_test_family.as_deref(), tukey.post_test.as_deref()), (Some("ALL_VS_ALL"), Some("TUKEY")));
    assert_eq!(tukey.canonical_label(), format!("{tukey}"));

    let dunnett = extract_analysis_design_key(
        "ONE_WAY_ANOVA",
        &mk_post(json!({
            "type": "PAIRWISE_TESTS",
            "pairwiseTests": {
                "pairwiseDesign": {"type": "CONTROL_VS_ALL", "controlID": {"col": {"idx": 0}}},
                "multipleComparisons": {"type": "STATISTICAL", "statistical": {"test": "DUNNET"}}
            }
        })),
    );
    assert_eq!((dunnett.post_test_family.as_deref(), dunnett.post_test.as_deref()), (Some("CONTROL_VS_ALL"), Some("DUNNETT")));

    // 4. Promotion bar constants and `CorpusDesignEvidence::meets_bar` boundary checks.
    assert_eq!(
        (MIN_VERIFIED_DESIGN_ANALYSES, MIN_VERIFIED_DESIGN_SOURCES, MIN_VERIFIED_DESIGN_AGREEMENT_NUM, MIN_VERIFIED_DESIGN_AGREEMENT_DEN),
        (20, 2, 99, 100)
    );
    let ev = |compared, sources, agreed, date: &'static str| CorpusDesignEvidence {
        analyses_compared: compared,
        distinct_source_records: sources,
        analyses_fully_agreed: agreed,
        verified_on: date,
    };
    assert!(ev(20, 2, 20, "2026-11-27").meets_bar());
    assert!(ev(100, 2, 99, "2026-11-27").meets_bar());
    assert!(!ev(100, 2, 98, "2026-11-27").meets_bar());
    assert!(!ev(19, 2, 19, "2026-11-27").meets_bar());
    assert!(!ev(20, 1, 20, "2026-11-27").meets_bar());
    assert!(!ev(20, 2, 21, "2026-11-27").meets_bar());
    assert!(!ev(20, 2, 20, "").meets_bar());

    let probe = AnalysisDesignKey {
        foreign_class: String::from("DESCRIPTIVE_STATISTICS"),
        experimental_design: None,
        tail: DesignTail::NotApplicable,
        post_test_family: None,
        post_test: None,
    };
    assert_eq!(probe.canonical_label(), "DESCRIPTIVE_STATISTICS");
    assert!(find_verified_analysis_design(&probe).is_some());
}

#[test]
fn external_archive_verified_designs_promote_clean_and_block_unverified() {
    let mk_post = |family: &str, mc_type: &str, test: &str| {
        json!({"content": {"experimentalDesign": "ORDINARY_ANOVA", "postTests": {"type": "PAIRWISE_TESTS", "pairwiseTests": {
            "pairwiseDesign": {"type": family, "controlID": {"col": {"idx": 0}}},
            "multipleComparisons": {"type": mc_type, "statistical": {"test": test}},
        }}}})
    };

    // 1. All 10 promoted designs return CorpusVerified with zero blocking options when clean.
    let promoted_cases = [
        (
            "T_TESTS_AND_NONPARAMETRIC",
            json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}}),
        ),
        ("T_TESTS_AND_NONPARAMETRIC", json!({"content": {"experimentalDesign": "PAIRED_T_TEST"}})),
        ("T_TESTS_AND_NONPARAMETRIC", json!({"content": {"experimentalDesign": "UNPAIRED_WELCH_T_TEST"}})),
        ("ONE_WAY_ANOVA", json!({"content": {"experimentalDesign": "ORDINARY_ANOVA", "postTests": {"type": "NO_TESTS"}}})),
        ("ONE_WAY_ANOVA", mk_post("ALL_VS_ALL", "STATISTICAL", "TUKEY_HSD")),
        ("ONE_WAY_ANOVA", mk_post("CONTROL_VS_ALL", "STATISTICAL", "DUNNET")),
        ("TWO_WAY_ANOVA", json!({"content": {"experimentalDesign": "ORDINARY_ANOVA", "options": {"includeInteractionTerm": true}}})),
        ("DESCRIPTIVE_STATISTICS", json!({"content": {"data": {"handleReplicatesAs": "AVERAGE"}}})),
        ("AREA_UNDER_CURVE", json!({"content": {"calc": {"baseline": {"useSpecificValue": true, "value": 0.0}}}})),
        ("IDENTIFY_OUTLIERS", json!({"content": {"calc": {"method": "ROUT", "q": 1.0}, "data": {"handleReplicatesAs": "COLUMN"}}})),
    ];
    for (class, params) in promoted_cases {
        let tr = translate_analysis_parameters(class, &params).unwrap();
        assert!(tr.blocking_options.is_empty(), "{class}: {:?}", tr.blocking_options);
        assert_eq!(tr.verification, AnalysisVerificationStatus::CorpusVerified, "{class}: {params}");
    }

    // 2. Unpromoted or blocked designs stay Unverified.
    let blocked_or_unverified = [
        (
            "T_TESTS_AND_NONPARAMETRIC",
            json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_ONE_TAILED"}}}),
            false,
        ),
        ("T_TESTS_AND_NONPARAMETRIC", json!({"content": {"experimentalDesign": "MANN_WHITNEY"}}), false),
        ("ONE_WAY_ANOVA", json!({"content": {"experimentalDesign": "KRUSKAL_WALLIS"}}), false),
        ("ONE_WAY_ANOVA", mk_post("ALL_VS_ALL", "STATISTICAL", "HOLM_SIDAK"), false),
        ("ONE_WAY_ANOVA", mk_post("ALL_VS_ALL", "NO_CORRECTION", "TUKEY"), false),
        ("TWO_WAY_ANOVA", json!({"content": {"experimentalDesign": "REPEATED_MEASURES_BY_ROWS"}}), false),
        ("TWO_WAY_ANOVA", mk_post("ALL_VS_ALL", "STATISTICAL", "TUKEY"), false),
        ("TWO_WAY_ANOVA", json!({"content": {"experimentalDesign": "ORDINARY_ANOVA", "options": {"includeInteractionTerm": false}}}), true),
        ("NONLINEAR_REGRESSION", json!({"content": {"model": {"equation": {"name": "Straight line"}}}}), false),
        ("NORMALITY_TESTS", json!({"content": {"calc": {"tests": {"shaproWilks": true}}}}), false),
        ("SURVIVAL", json!({"content": {"calc": {"logrankTest": true}}}), false),
        ("DESCRIPTIVE_STATISTICS", json!({"content": {"data": {"handleReplicatesAs": "SUB_COLUMN"}}}), true),
        ("IDENTIFY_OUTLIERS", json!({"content": {"calc": {"method": "ROUT"}, "data": {"handleReplicatesAs": "SUB_COLUMN"}}}), true),
        ("IDENTIFY_OUTLIERS", json!({"content": {"calc": {"method": "ROUT"}, "data": {"handleReplicatesAs": "EXOTIC"}}}), true),
    ];
    for (class, params, expect_blocking) in blocked_or_unverified {
        let tr = translate_analysis_parameters(class, &params).unwrap();
        assert_eq!(tr.verification, AnalysisVerificationStatus::Unverified, "{class}: {params}");
        if expect_blocking {
            assert!(!tr.blocking_options.is_empty(), "{class}: expected blocking option for {params}");
        }
    }
}

#[test]
fn external_archive_source_sheet_option_compatibility_blocks_multi_subcolumn_average_outliers() {
    let avg_params = json!({"content": {"calc": {"method": "ROUT"}, "data": {"handleReplicatesAs": "AVERAGE"}}});
    let default_params = json!({"content": {"calc": {"method": "ROUT"}}});
    let col_params = json!({"content": {"calc": {"method": "ROUT"}, "data": {"handleReplicatesAs": "COLUMN"}}});

    let xy_multi = DataSheet::new("xy-m", "XY Multi", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(5), 2, 4).unwrap())).unwrap();
    let grp_multi =
        DataSheet::new("grp-m", "Grp Multi", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap())).unwrap();
    let col_multi = DataSheet::new(
        "col-m",
        "Col Multi",
        TableArchetype::Column(ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 2, 3).unwrap()),
    )
    .unwrap();
    let nest_multi = DataSheet::new("nest-m", "Nest Multi", TableArchetype::Nested(NestedTable::new(2, 2, 3).unwrap())).unwrap();
    for ds in [&xy_multi, &grp_multi, &col_multi, &nest_multi] {
        assert!(check_source_sheet_option_compatibility("IDENTIFY_OUTLIERS", &avg_params, ds).is_err(), "{}", ds.id);
        assert!(check_source_sheet_option_compatibility("IDENTIFY_OUTLIERS", &default_params, ds).is_err(), "{}", ds.id);
        assert!(check_source_sheet_option_compatibility("IDENTIFY_OUTLIERS", &col_params, ds).is_ok(), "{}", ds.id);
    }

    let col_single =
        DataSheet::new("col-s", "Col Single", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::default(), 2, 4).unwrap())).unwrap();
    let xy_single = DataSheet::new("xy-s", "XY Single", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 4).unwrap())).unwrap();
    for ds in [&col_single, &xy_single] {
        assert!(check_source_sheet_option_compatibility("IDENTIFY_OUTLIERS", &avg_params, ds).is_ok(), "{}", ds.id);
        assert!(check_source_sheet_option_compatibility("IDENTIFY_OUTLIERS", &default_params, ds).is_ok(), "{}", ds.id);
    }
}

#[test]
fn external_archive_pairwise_p_abs_tol_accepts_as190_tail_noise_and_keeps_mean_diff_strict() {
    let results = json!({"content": {
        "ordinaryAnovaModel": {
            "anovaTable": { "colFactor": { "fTest": { "pValue": { "state": "P_VALUE_EQUAL", "value": 0.001 } } } },
            "multipleComparisons": {
                "tests": {
                    "u0": {
                        "id": { "col": { "idx": 0 } },
                        "value": {
                            "u1": {
                                "id": { "col": { "idx": 1 } },
                                "value": {
                                    "status": "VALID",
                                    "meansDifference": 1.0,
                                    "pValueAdjusted": { "state": "P_VALUE_EQUAL", "value": 0.001 }
                                }
                            }
                        }
                    }
                }
            }
        }
    }});
    let reference = extract_cached_analysis_reference("ONE_WAY_ANOVA", &results);
    assert_eq!(reference.pairwise.len(), 1);
    assert_eq!(reference.pairwise[0].p_abs_tol, Some(1e-5));

    let mk_pw = |p: f64, diff: f64| {
        vec![PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("Ctrl"),
            label_b: String::from("Drug"),
            estimate_diff: Some(diff),
            raw_p_or_q: p,
            is_fdr_q: false,
            significant: true,
        }]
    };
    let omnibus_ok = vec![ResultScalarEntry { key: String::from("P value"), data_set_index: None, value: 0.001, units: None }];

    // 4e-6 diff on pairwise adjusted p is accepted under 1e-5 external tail tolerance.
    let diag_tail = compare_recomputed_against_reference(&make_test_result_sheet(omnibus_ok.clone(), mk_pw(0.001_004, 1.0)), &reference, 1e-9, 1e-6);
    assert_eq!((diag_tail.pairwise_compared_count, diag_tail.pairwise_disagreements), (2, 0));
    assert!(diag_tail.disagreements.is_empty());

    // 2e-5 diff on pairwise adjusted p is flagged.
    let diag_p_bad = compare_recomputed_against_reference(&make_test_result_sheet(omnibus_ok.clone(), mk_pw(0.001_020, 1.0)), &reference, 1e-9, 1e-6);
    assert_eq!(diag_p_bad.pairwise_disagreements, 1);
    assert_eq!(diag_p_bad.disagreements[0].key, "Pairwise adjusted P");

    // 4e-6 diff on |meansDifference| stays strict at 1e-9/1e-6 and is flagged.
    let diag_diff_bad = compare_recomputed_against_reference(&make_test_result_sheet(omnibus_ok, mk_pw(0.001, 1.000_004)), &reference, 1e-9, 1e-6);
    assert_eq!(diag_diff_bad.pairwise_disagreements, 1);
    assert_eq!(diag_diff_bad.disagreements[0].key, "Pairwise mean difference");

    // 4e-6 diff on omnibus P value also stays strict at 1e-9/1e-6 and is flagged.
    let omnibus_bad = vec![ResultScalarEntry { key: String::from("P value"), data_set_index: None, value: 0.001_004, units: None }];
    let diag_omni_bad = compare_recomputed_against_reference(&make_test_result_sheet(omnibus_bad, mk_pw(0.001, 1.0)), &reference, 1e-9, 1e-6);
    assert_eq!((diag_omni_bad.pairwise_disagreements, diag_omni_bad.disagreements.len()), (0, 1));
    assert_eq!(diag_omni_bad.disagreements[0].key, "P value");
}

#[test]
fn external_archive_selected_data_sets_resolve_in_listed_order_or_block() {
    let mut positions = ExternalDataSetPositions::new();
    for (uid, sheet, pos) in [("ds-a", "sheet-1", 0), ("ds-b", "sheet-1", 1), ("ds-c", "sheet-1", 2), ("ds-x", "sheet-2", 0)] {
        positions.insert(uid.to_owned(), (sheet.to_owned(), pos));
    }
    let subset = |uids: &[&str]| json!(uids.iter().map(|u| json!({"uid": u})).collect::<Vec<_>>());
    type SelectionCase = (&'static str, serde_json::Value, Option<&'static str>, Result<Vec<usize>, ()>);
    let cases: [SelectionCase; 8] = [
        ("flag absent analyses all data", json!({"inputDataSets": subset(&["ds-b"])}), Some("sheet-1"), Ok(vec![])),
        ("flag true analyses all data", json!({"isAllDataAnalyzed": true, "inputDataSets": subset(&["ds-b"])}), Some("sheet-1"), Ok(vec![])),
        ("listed order is kept", json!({"isAllDataAnalyzed": false, "inputDataSets": subset(&["ds-c", "ds-a"])}), Some("sheet-1"), Ok(vec![2, 0])),
        ("unknown uid blocks", json!({"isAllDataAnalyzed": false, "inputDataSets": subset(&["ds-a", "ds-zz"])}), Some("sheet-1"), Err(())),
        ("foreign sheet blocks", json!({"isAllDataAnalyzed": false, "inputDataSets": subset(&["ds-a", "ds-x"])}), Some("sheet-1"), Err(())),
        ("duplicate blocks", json!({"isAllDataAnalyzed": false, "inputDataSets": subset(&["ds-a", "ds-a"])}), Some("sheet-1"), Err(())),
        ("empty subset blocks", json!({"isAllDataAnalyzed": false, "inputDataSets": []}), Some("sheet-1"), Err(())),
        ("missing source blocks", json!({"isAllDataAnalyzed": false, "inputDataSets": subset(&["ds-a"])}), None, Err(())),
    ];
    for (label, sheet, source, expected) in cases {
        let got = resolve_selected_data_sets(sheet.as_object().unwrap(), source, &positions).map_err(|_| ());
        assert_eq!(got, expected, "{label}");
    }
}

#[test]
fn external_archive_post_tests_and_replicate_handling_translate_or_block() {
    use curvebook_doc::{MultiplicityMethod, OneWayAnovaSpec, SubcolumnAggregation};

    let post = |family: &str, mc_type: &str, test: &str| {
        json!({"content": {"experimentalDesign": "ORDINARY_ANOVA", "postTests": {"type": "PAIRWISE_TESTS", "pairwiseTests": {
            "pairwiseDesign": {"type": family, "controlID": {"col": {"idx": 2}}},
            "multipleComparisons": {"type": mc_type, "statistical": {"test": test}, "fdr": {"test": test}},
        }}}})
    };
    let one_way = |params: serde_json::Value| {
        let tr = translate_analysis_parameters("ONE_WAY_ANOVA", &params).unwrap();
        let AnalysisKind::OneWayAnova(OneWayAnovaSpec { multiplicity, .. }) = tr.kind.clone() else { panic!("not one-way") };
        (multiplicity.method, tr)
    };
    let cases = [
        ("Tukey all pairs", post("ALL_VS_ALL", "STATISTICAL", "TUKEY"), Some(MultiplicityMethod::TukeyHsd)),
        ("Dunn all pairs", post("ALL_VS_ALL", "STATISTICAL", "DUNN"), Some(MultiplicityMethod::DunnRankSum)),
        ("Bonferroni-Dunn is Bonferroni", post("ALL_VS_ALL", "STATISTICAL", "BONFERRONI_DUNN"), Some(MultiplicityMethod::Bonferroni)),
        ("Bonferroni-Sidak is Sidak", post("ALL_VS_ALL", "STATISTICAL", "BONFERRONI_SIDAK"), Some(MultiplicityMethod::Sidak)),
        ("two-stage FDR", post("ALL_VS_ALL", "FDR", "BENJAMINI_KRIEGER_YEKUTIELI"), Some(MultiplicityMethod::TwoStageBkyFdr)),
        ("no correction", post("ALL_VS_ALL", "NO_CORRECTION", "TUKEY"), Some(MultiplicityMethod::None)),
        ("no test", post("ALL_VS_ALL", "STATISTICAL", "NO_TEST"), Some(MultiplicityMethod::None)),
        ("Dunnett keeps sheet control", post("CONTROL_VS_ALL", "STATISTICAL", "DUNNET"), Some(MultiplicityMethod::Dunnett { control_column: 2 })),
        ("unknown test name blocks", post("ALL_VS_ALL", "STATISTICAL", "UNKNOWN_POST_TEST"), None),
        ("selected pairs without pair list block", post("SELECTED_VS_SELECTED", "STATISTICAL", "HOLM_SIDAK"), None),
        ("Tukey against control blocks", post("CONTROL_VS_ALL", "STATISTICAL", "TUKEY"), None),
        ("unadjusted selected pairs without pair list block", post("SELECTED_VS_SELECTED", "NO_CORRECTION", "TUKEY"), None),
        ("unadjusted control family blocks", post("CONTROL_VS_ALL", "STATISTICAL", "NO_TEST"), None),
    ];
    for (label, params, expected) in cases {
        let (method, tr) = one_way(params);
        match expected {
            Some(m) => {
                assert_eq!(method, m, "{label}");
                assert!(tr.blocking_options.is_empty(), "{label}: {:?}", tr.blocking_options);
            }
            None => assert!(!tr.blocking_options.is_empty(), "{label}"),
        }
    }

    // The Dunnett control keeps its sheet index and must be among the analysed data sets.
    let (_, tr) = one_way(post("CONTROL_VS_ALL", "STATISTICAL", "DUNNET"));
    assert!(check_control_in_selection(&tr.kind, &[4, 2, 0]).is_ok());
    assert!(matches!(
        tr.kind,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 2 }, .. },
            ..
        })
    ));
    assert!(check_control_in_selection(&tr.kind, &[0, 1]).is_err());
    assert!(check_control_in_selection(&tr.kind, &[]).is_ok());

    // Two-way Dunnett controls stay blocked.
    let mut two_way = post("CONTROL_VS_ALL", "STATISTICAL", "DUNNET");
    two_way["content"]["experimentalDesign"] = json!("ORDINARY_ANOVA");
    assert!(!translate_analysis_parameters("TWO_WAY_ANOVA", &two_way).unwrap().blocking_options.is_empty());

    // Replicate handling: corpus spellings map; per-subcolumn and unknown values block.
    let descriptive =
        |mode: &str| translate_analysis_parameters("DESCRIPTIVE_STATISTICS", &json!({"content": {"data": {"handleReplicatesAs": mode}}})).unwrap();
    for (mode, agg, blocks) in [
        ("AVERAGE", SubcolumnAggregation::RowMeans, false),
        ("COLUMN", SubcolumnAggregation::PooledGrandSample, false),
        ("SUB_COLUMN", SubcolumnAggregation::PerSubcolumn, true),
        ("SOMETHING_NEW", SubcolumnAggregation::RowMeans, true),
    ] {
        let tr = descriptive(mode);
        assert!(matches!(&tr.kind, AnalysisKind::DescriptiveStats(d) if d.aggregation == agg), "{mode}");
        assert_eq!(!tr.blocking_options.is_empty(), blocks, "{mode}");
    }
}

#[test]
fn issue_77_bonferroni_sidak_maps_to_sidak_across_anova_designs() {
    use curvebook_doc::{FactorialAnovaSpec, MultiplicityMethod, OneWayAnovaSpec};

    let make_params = |design: &str, family: &str, test: &str, adjusted_p: bool| {
        json!({"content": {
            "experimentalDesign": design,
            "postTests": {
                "type": "PAIRWISE_TESTS",
                "pairwiseTests": {
                    "pairwiseDesign": {"type": family, "controlID": {"col": {"idx": 0}}},
                    "multipleComparisons": {
                        "type": "STATISTICAL",
                        "statistical": {
                            "options": {"significanceLevel": 0.05},
                            "report": {"adjustedPValues": adjusted_p},
                            "test": test,
                        }
                    }
                }
            }
        }})
    };

    type Case = (&'static str, &'static str, &'static str, &'static str, &'static str, bool, Option<MultiplicityMethod>);
    let cases: &[Case] = &[
        // Corpus-verified Šidák (1967) mapping for BONFERRONI_SIDAK across one-way & two-way designs.
        (
            "one-way ordinary BONFERRONI_SIDAK",
            "ONE_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "ALL_VS_ALL",
            "BONFERRONI_SIDAK",
            true,
            Some(MultiplicityMethod::Sidak),
        ),
        (
            "one-way RM BONFERRONI_SIDAK (adjustedPValues=false)",
            "ONE_WAY_ANOVA",
            "REPEATED_MEASURES_ANOVA",
            "ALL_VS_ALL",
            "BONFERRONI_SIDAK",
            false,
            Some(MultiplicityMethod::Sidak),
        ),
        (
            "one-way ordinary case-insensitive bonferroni_sidak",
            "ONE_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "ALL_VS_ALL",
            "bonferroni_sidak",
            true,
            Some(MultiplicityMethod::Sidak),
        ),
        (
            "two-way ordinary BONFERRONI_SIDAK",
            "TWO_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "ALL_VS_ALL",
            "BONFERRONI_SIDAK",
            true,
            Some(MultiplicityMethod::Sidak),
        ),
        (
            "two-way RM-rows BONFERRONI_SIDAK",
            "TWO_WAY_ANOVA",
            "REPEATED_MEASURES_BY_ROWS",
            "ALL_VS_ALL",
            "BONFERRONI_SIDAK",
            true,
            Some(MultiplicityMethod::Sidak),
        ),
        (
            "two-way RM-cols BONFERRONI_SIDAK",
            "TWO_WAY_ANOVA",
            "REPEATED_MEASURES_BY_COLS",
            "ALL_VS_ALL",
            "BONFERRONI_SIDAK",
            false,
            Some(MultiplicityMethod::Sidak),
        ),
        (
            "two-way RM-both BONFERRONI_SIDAK",
            "TWO_WAY_ANOVA",
            "REPEATED_MEASURES_BY_BOTH_FACTORS",
            "ALL_VS_ALL",
            "BONFERRONI_SIDAK",
            true,
            Some(MultiplicityMethod::Sidak),
        ),
        // Contrast: BONFERRONI_DUNN / BONFERRONI remain Bonferroni (Dunn 1961); SIDAK & HOLM_SIDAK stay distinct.
        (
            "one-way BONFERRONI_DUNN is Bonferroni",
            "ONE_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "ALL_VS_ALL",
            "BONFERRONI_DUNN",
            true,
            Some(MultiplicityMethod::Bonferroni),
        ),
        (
            "two-way BONFERRONI_DUNN is Bonferroni",
            "TWO_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "ALL_VS_ALL",
            "BONFERRONI_DUNN",
            true,
            Some(MultiplicityMethod::Bonferroni),
        ),
        ("one-way SIDAK is Sidak", "ONE_WAY_ANOVA", "ORDINARY_ANOVA", "ALL_VS_ALL", "SIDAK", true, Some(MultiplicityMethod::Sidak)),
        ("two-way HOLM_SIDAK is HolmSidak", "TWO_WAY_ANOVA", "ORDINARY_ANOVA", "ALL_VS_ALL", "HOLM_SIDAK", true, Some(MultiplicityMethod::HolmSidak)),
        // One-way CONTROL_VS_ALL BONFERRONI_SIDAK also maps to Sidak (with control_column);
        // selected-pairs without selectedPairIDList, two-way CONTROL_VS_ALL without meansDesignType, and unknown tests block.
        (
            "one-way selected-pairs without pair list blocks",
            "ONE_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "SELECTED_VS_SELECTED",
            "BONFERRONI_SIDAK",
            true,
            None,
        ),
        (
            "one-way control-vs-all BONFERRONI_SIDAK maps to Sidak",
            "ONE_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "CONTROL_VS_ALL",
            "BONFERRONI_SIDAK",
            true,
            Some(MultiplicityMethod::Sidak),
        ),
        (
            "two-way selected-pairs without pair list blocks",
            "TWO_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "SELECTED_VS_SELECTED",
            "BONFERRONI_SIDAK",
            true,
            None,
        ),
        (
            "two-way control-vs-all without meansDesignType blocks",
            "TWO_WAY_ANOVA",
            "ORDINARY_ANOVA",
            "CONTROL_VS_ALL",
            "BONFERRONI_SIDAK",
            true,
            None,
        ),
        ("two-way unknown test blocks", "TWO_WAY_ANOVA", "ORDINARY_ANOVA", "ALL_VS_ALL", "UNKNOWN_CORRECTION", true, None),
    ];

    for &(label, class, design, family, test, adjusted_p, expected) in cases {
        let tr = translate_analysis_parameters(class, &make_params(design, family, test, adjusted_p)).unwrap();
        let method = match &tr.kind {
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { multiplicity, .. })
            | AnalysisKind::FactorialAnova(FactorialAnovaSpec { multiplicity, .. }) => multiplicity.method,
            other => panic!("{label}: unexpected kind {other:?}"),
        };
        match expected {
            Some(want) => {
                assert_eq!(method, want, "{label}");
                assert!(tr.blocking_options.is_empty(), "{label}: {:?}", tr.blocking_options);
            }
            None => {
                assert!(!tr.blocking_options.is_empty(), "{label}: expected blocking option");
                assert_eq!(method, MultiplicityMethod::None, "{label}");
            }
        }
    }
}

#[test]
fn issue_78_control_vs_all_one_way_and_two_way_translation_and_selection_validation() {
    use curvebook_doc::{FactorialAnovaSpec, FactorialComparisonFamilyMode, MultiplicityMethod, OneWayAnovaSpec};

    let one_way_params = |design: &str, mc_type: &str, test: &str, ctrl_col: usize| {
        let mc = match mc_type {
            "NO_CORRECTION" => json!({"type": "NO_CORRECTION"}),
            "FDR" => json!({"type": "FDR", "fdr": {"test": test}}),
            _ => json!({"type": "STATISTICAL", "statistical": {"test": test}}),
        };
        json!({"content": {
            "experimentalDesign": design,
            "postTests": {
                "type": "PAIRWISE_TESTS",
                "pairwiseTests": {
                    "pairwiseDesign": {"type": "CONTROL_VS_ALL", "controlID": {"col": {"idx": ctrl_col}}},
                    "multipleComparisons": mc,
                }
            }
        }})
    };

    // One-way CONTROL_VS_ALL: every supported family translates with sheet control_column preserved.
    for (label, design, mc_type, test, want_method, want_extra_ctrl) in [
        ("one-way Dunnett", "ORDINARY_ANOVA", "STATISTICAL", "DUNNET", MultiplicityMethod::Dunnett { control_column: 2 }, None),
        ("one-way KW Dunn", "KRUSKAL_WALLIS", "STATISTICAL", "DUNN", MultiplicityMethod::DunnRankSum, Some(2)),
        ("one-way Friedman Dunn", "FRIEDMAN_TEST", "STATISTICAL", "DUNN", MultiplicityMethod::DunnRankSum, Some(2)),
        ("one-way Holm-Sidak", "ORDINARY_ANOVA", "STATISTICAL", "HOLM_SIDAK", MultiplicityMethod::HolmSidak, Some(2)),
        ("one-way BONFERRONI_SIDAK -> Sidak", "ORDINARY_ANOVA", "STATISTICAL", "BONFERRONI_SIDAK", MultiplicityMethod::Sidak, Some(2)),
        ("one-way BONFERRONI_DUNN -> Bonferroni", "ORDINARY_ANOVA", "STATISTICAL", "BONFERRONI_DUNN", MultiplicityMethod::Bonferroni, Some(2)),
        ("one-way NO_CORRECTION -> FisherLsd", "ORDINARY_ANOVA", "NO_CORRECTION", "", MultiplicityMethod::FisherLsd, Some(2)),
        ("one-way KW NO_CORRECTION -> FisherLsd", "KRUSKAL_WALLIS", "NO_CORRECTION", "", MultiplicityMethod::FisherLsd, Some(2)),
        ("one-way Welch DUNNET_T3", "BROWN_FORSYTHE_ANOVA", "STATISTICAL", "DUNNET_T3", MultiplicityMethod::DunnettT3, Some(2)),
        ("one-way Welch TAMHANE_T2", "BROWN_FORSYTHE_ANOVA", "STATISTICAL", "TAMHANE_T2", MultiplicityMethod::TamhaneT2, Some(2)),
        ("one-way FDR BH", "ORDINARY_ANOVA", "FDR", "BENJAMINI_HOCHBERG", MultiplicityMethod::BenjaminiHochbergFdr, Some(2)),
        ("one-way FDR BKY", "ORDINARY_ANOVA", "FDR", "BENJAMINI_KRIEGER_YEKUTIELI", MultiplicityMethod::TwoStageBkyFdr, Some(2)),
    ] {
        let tr = translate_analysis_parameters("ONE_WAY_ANOVA", &one_way_params(design, mc_type, test, 2)).unwrap();
        assert!(tr.blocking_options.is_empty(), "{label}: {:?}", tr.blocking_options);
        let AnalysisKind::OneWayAnova(OneWayAnovaSpec { multiplicity, .. }) = &tr.kind else {
            panic!("{label}: expected OneWayAnova");
        };
        assert_eq!(multiplicity.method, want_method, "{label}");
        assert_eq!(multiplicity.control_column, want_extra_ctrl, "{label}");
        assert_eq!(multiplicity.effective_control_column(), Some(2), "{label}");
        assert!(check_control_in_selection(&tr.kind, &[4, 2, 0]).is_ok(), "{label} in selection");
        assert!(check_control_in_selection(&tr.kind, &[0, 1]).is_err(), "{label} missing from selection");
        assert!(check_control_in_selection(&tr.kind, &[]).is_ok(), "{label} empty selection");
    }

    // One-way blocked CONTROL_VS_ALL cases: ambiguous STATISTICAL+NO_TEST, Tukey, Games-Howell.
    for (label, test) in [("one-way NO_TEST blocks", "NO_TEST"), ("one-way TUKEY blocks", "TUKEY"), ("one-way GAMES_HOWELL blocks", "GAMES_HOWELL")] {
        let tr = translate_analysis_parameters("ONE_WAY_ANOVA", &one_way_params("ORDINARY_ANOVA", "STATISTICAL", test, 2)).unwrap();
        assert!(!tr.blocking_options.is_empty(), "{label}");
    }

    // Two-way CONTROL_VS_ALL: meansDesignType + controlID.{col|row}.idx -> (control_factor, mode).
    let two_way_params = |means_design: &str, mc_type: &str, test: &str, ctrl_id: serde_json::Value| {
        let mc = match mc_type {
            "NO_CORRECTION" => json!({"type": "NO_CORRECTION"}),
            _ => json!({"type": "STATISTICAL", "statistical": {"test": test}}),
        };
        json!({"content": {
            "experimentalDesign": "ORDINARY_ANOVA",
            "postTests": {
                "type": "PAIRWISE_TESTS",
                "pairwiseTests": {
                    "meansDesignType": means_design,
                    "pairwiseDesign": {"type": "CONTROL_VS_ALL", "controlID": ctrl_id},
                    "multipleComparisons": mc,
                }
            }
        }})
    };

    for (label, means_design, mc_type, test, ctrl_id, want_method, want_extra_ctrl, want_factor, want_mode) in [
        (
            "two-way cols-within-rows separate Dunnett",
            "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SEPARATE_FAMILIES",
            "STATISTICAL",
            "DUNNET",
            json!({"col": {"idx": 1}}),
            MultiplicityMethod::Dunnett { control_column: 1 },
            None,
            Some(1),
            FactorialComparisonFamilyMode::PerSliceFamilies,
        ),
        (
            "two-way rows-within-cols separate Dunnett",
            "COMPARE_CELLS_BY_ROWS_WITHIN_COLUMNS_SEPARATE_FAMILIES",
            "STATISTICAL",
            "DUNNET",
            json!({"row": {"idx": 0}}),
            MultiplicityMethod::Dunnett { control_column: 0 },
            None,
            Some(0),
            FactorialComparisonFamilyMode::PerSliceFamilies,
        ),
        (
            "two-way cols-within-rows single family Sidak",
            "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SINGLE_FAMILY",
            "STATISTICAL",
            "BONFERRONI_SIDAK",
            json!({"col": {"idx": 2}}),
            MultiplicityMethod::Sidak,
            Some(2),
            Some(1),
            FactorialComparisonFamilyMode::TwoFactorFamilies,
        ),
        (
            "two-way rows-within-cols single family Bonferroni",
            "COMPARE_CELLS_BY_ROWS_WITHIN_COLUMNS_SINGLE_FAMILY",
            "STATISTICAL",
            "BONFERRONI_DUNN",
            json!({"row": {"idx": 1}}),
            MultiplicityMethod::Bonferroni,
            Some(1),
            Some(0),
            FactorialComparisonFamilyMode::TwoFactorFamilies,
        ),
        (
            "two-way cols-within-rows separate Holm-Sidak",
            "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SEPARATE_FAMILIES",
            "STATISTICAL",
            "HOLM_SIDAK",
            json!({"col": {"idx": 0}}),
            MultiplicityMethod::HolmSidak,
            Some(0),
            Some(1),
            FactorialComparisonFamilyMode::PerSliceFamilies,
        ),
        (
            "two-way COMPARE_COLUMNS Dunnett",
            "COMPARE_COLUMNS",
            "STATISTICAL",
            "DUNNET",
            json!({"col": {"idx": 3}}),
            MultiplicityMethod::Dunnett { control_column: 3 },
            None,
            Some(1),
            FactorialComparisonFamilyMode::Standard,
        ),
        (
            "two-way COMPARE_ROWS Sidak",
            "COMPARE_ROWS",
            "STATISTICAL",
            "BONFERRONI_SIDAK",
            json!({"row": {"idx": 2}}),
            MultiplicityMethod::Sidak,
            Some(2),
            Some(0),
            FactorialComparisonFamilyMode::Standard,
        ),
        (
            "two-way rows-within-cols NO_CORRECTION -> FisherLsd",
            "COMPARE_CELLS_BY_ROWS_WITHIN_COLUMNS_SEPARATE_FAMILIES",
            "NO_CORRECTION",
            "",
            json!({"row": {"idx": 0}}),
            MultiplicityMethod::FisherLsd,
            Some(0),
            Some(0),
            FactorialComparisonFamilyMode::PerSliceFamilies,
        ),
    ] {
        let tr = translate_analysis_parameters("TWO_WAY_ANOVA", &two_way_params(means_design, mc_type, test, ctrl_id)).unwrap();
        assert!(tr.blocking_options.is_empty(), "{label}: {:?}", tr.blocking_options);
        let AnalysisKind::FactorialAnova(FactorialAnovaSpec { multiplicity, control_factor, comparison_family_mode, .. }) = &tr.kind else {
            panic!("{label}: expected FactorialAnova");
        };
        assert_eq!(multiplicity.method, want_method, "{label}");
        assert_eq!(multiplicity.control_column, want_extra_ctrl, "{label}");
        assert_eq!(*control_factor, want_factor, "{label}");
        assert_eq!(*comparison_family_mode, want_mode, "{label}");
        if want_factor == Some(1) {
            let ctrl = multiplicity.effective_control_column().unwrap();
            assert!(check_control_in_selection(&tr.kind, &[ctrl, 9]).is_ok(), "{label} col ctrl present");
            assert!(check_control_in_selection(&tr.kind, &[99]).is_err(), "{label} col ctrl missing");
        } else {
            // Row-factor controls must not be rejected against selected column data sets.
            assert!(check_control_in_selection(&tr.kind, &[99]).is_ok(), "{label} row ctrl independent of selected cols");
        }
    }

    // Two-way blocked cases: ambiguous STATISTICAL+NO_TEST, 2D COMPARE_CELLS, mismatched axis, DUNN.
    for (label, means_design, test, ctrl_id) in [
        ("two-way STATISTICAL NO_TEST blocks", "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SEPARATE_FAMILIES", "NO_TEST", json!({"col": {"idx": 0}})),
        ("two-way COMPARE_CELLS 2D control blocks", "COMPARE_CELLS", "BONFERRONI_SIDAK", json!({"col": {"idx": 0}, "row": {"idx": 0}})),
        ("two-way axis mismatch (row on COMPARE_COLUMNS) blocks", "COMPARE_COLUMNS", "DUNNET", json!({"row": {"idx": 0}})),
        ("two-way DUNN blocks", "COMPARE_COLUMNS", "DUNN", json!({"col": {"idx": 0}})),
    ] {
        let tr = translate_analysis_parameters("TWO_WAY_ANOVA", &two_way_params(means_design, "STATISTICAL", test, ctrl_id)).unwrap();
        assert!(!tr.blocking_options.is_empty(), "{label}");
    }
}

#[test]
fn issue_76_selected_pairs_one_way_and_two_way_translation_and_selection_validation() {
    use curvebook_doc::{FactorialAnovaSpec, FactorialComparisonFamilyMode, MultiplicityMethod, OneWayAnovaSpec};

    let col_pair = |a: usize, b: usize| json!([{"col": {"idx": a}}, {"col": {"idx": b}}]);
    let row_pair = |a: usize, b: usize| json!([{"row": {"idx": a}}, {"row": {"idx": b}}]);
    let one_way_params = |design: &str, mc_type: &str, test: &str, pair_list: serde_json::Value| {
        let mc = match mc_type {
            "NO_CORRECTION" => json!({"type": "NO_CORRECTION"}),
            "FDR" => json!({"type": "FDR", "fdr": {"test": test}}),
            _ => json!({"type": "STATISTICAL", "statistical": {"test": test}}),
        };
        json!({"content": {
            "experimentalDesign": design,
            "postTests": {
                "type": "PAIRWISE_TESTS",
                "pairwiseTests": {
                    "pairwiseDesign": {"type": "SELECTED_VS_SELECTED", "selectedPairIDList": pair_list},
                    "multipleComparisons": mc,
                }
            }
        }})
    };

    // One-way SELECTED_VS_SELECTED: every supported method translates cleanly and preserves exact sheet pairs.
    let valid_list = json!([col_pair(0, 2), col_pair(1, 3)]);
    for (label, design, mc_type, test, want_method) in [
        ("one-way BONFERRONI_SIDAK -> Sidak", "ORDINARY_ANOVA", "STATISTICAL", "BONFERRONI_SIDAK", MultiplicityMethod::Sidak),
        ("one-way BONFERRONI_DUNN -> Bonferroni", "ORDINARY_ANOVA", "STATISTICAL", "BONFERRONI_DUNN", MultiplicityMethod::Bonferroni),
        ("one-way HOLM_SIDAK", "ORDINARY_ANOVA", "STATISTICAL", "HOLM_SIDAK", MultiplicityMethod::HolmSidak),
        ("one-way RM BONFERRONI_SIDAK", "REPEATED_MEASURES_ANOVA", "STATISTICAL", "BONFERRONI_SIDAK", MultiplicityMethod::Sidak),
        ("one-way KW DUNN", "KRUSKAL_WALLIS_TEST", "STATISTICAL", "DUNN", MultiplicityMethod::DunnRankSum),
        ("one-way NO_CORRECTION -> FisherLsd", "ORDINARY_ANOVA", "NO_CORRECTION", "", MultiplicityMethod::FisherLsd),
        ("one-way Welch DUNNET_T3", "BROWN_FORSYTHE_ANOVA", "STATISTICAL", "DUNNET_T3", MultiplicityMethod::DunnettT3),
        ("one-way Welch TAMHANE_T2", "BROWN_FORSYTHE_ANOVA", "STATISTICAL", "TAMHANE_T2", MultiplicityMethod::TamhaneT2),
        ("one-way FDR BH", "ORDINARY_ANOVA", "FDR", "BENJAMINI_HOCHBERG", MultiplicityMethod::BenjaminiHochbergFdr),
        ("one-way FDR BKY", "ORDINARY_ANOVA", "FDR", "BENJAMINI_KRIEGER_YEKUTIELI", MultiplicityMethod::TwoStageBkyFdr),
    ] {
        let tr = translate_analysis_parameters("ONE_WAY_ANOVA", &one_way_params(design, mc_type, test, valid_list.clone())).unwrap();
        assert!(tr.blocking_options.is_empty(), "{label}: {:?}", tr.blocking_options);
        let AnalysisKind::OneWayAnova(OneWayAnovaSpec { multiplicity, .. }) = &tr.kind else {
            panic!("{label}: expected OneWayAnova");
        };
        assert_eq!(multiplicity.method, want_method, "{label}");
        assert_eq!(multiplicity.control_column, None, "{label}");
        assert_eq!(multiplicity.selected_pairs.as_deref(), Some(&[(0, 2), (1, 3)][..]), "{label}");
        assert!(check_control_in_selection(&tr.kind, &[0, 1, 2, 3]).is_ok(), "{label} all in selection");
        assert!(check_control_in_selection(&tr.kind, &[0, 1, 2]).is_err(), "{label} missing col 3");
        assert!(check_control_in_selection(&tr.kind, &[]).is_ok(), "{label} empty selection");
    }

    // Partial-null placeholder slots ([null, null]) are skipped when valid pairs are present.
    let partial_null = json!([col_pair(0, 1), [null, null], col_pair(2, 4), [null, null]]);
    let tr_partial =
        translate_analysis_parameters("ONE_WAY_ANOVA", &one_way_params("ORDINARY_ANOVA", "STATISTICAL", "BONFERRONI_SIDAK", partial_null)).unwrap();
    assert!(tr_partial.blocking_options.is_empty(), "partial-null should translate cleanly: {:?}", tr_partial.blocking_options);
    let AnalysisKind::OneWayAnova(OneWayAnovaSpec { multiplicity, .. }) = &tr_partial.kind else { panic!("not OneWayAnova") };
    assert_eq!(multiplicity.selected_pairs.as_deref(), Some(&[(0, 1), (2, 4)][..]));

    // Blocked one-way SELECTED_VS_SELECTED cases: empty, all-null placeholders, half-null, self-pair,
    // duplicate (incl. reversed), STATISTICAL+NO_TEST, incompatible methods, and row index.
    for (label, mc_type, test, pair_list) in [
        ("empty pair list (STATISTICAL)", "STATISTICAL", "BONFERRONI_SIDAK", json!([])),
        ("empty pair list (NO_CORRECTION)", "NO_CORRECTION", "", json!([])),
        ("all-null placeholders", "STATISTICAL", "BONFERRONI_SIDAK", json!([[null, null], [null, null]])),
        ("half-null incomplete pair", "STATISTICAL", "BONFERRONI_SIDAK", json!([[{"col": {"idx": 0}}, null]])),
        ("self-pair", "STATISTICAL", "BONFERRONI_SIDAK", json!([col_pair(2, 2)])),
        ("duplicate reversed pair", "STATISTICAL", "BONFERRONI_SIDAK", json!([col_pair(0, 1), col_pair(1, 0)])),
        ("ambiguous STATISTICAL+NO_TEST", "STATISTICAL", "NO_TEST", valid_list.clone()),
        ("incompatible TUKEY", "STATISTICAL", "TUKEY", valid_list.clone()),
        ("incompatible DUNNET", "STATISTICAL", "DUNNET", valid_list.clone()),
        ("incompatible GAMES_HOWELL", "STATISTICAL", "GAMES_HOWELL", valid_list.clone()),
        ("row member in one-way", "STATISTICAL", "BONFERRONI_SIDAK", json!([row_pair(0, 1)])),
    ] {
        let tr = translate_analysis_parameters("ONE_WAY_ANOVA", &one_way_params("ORDINARY_ANOVA", mc_type, test, pair_list)).unwrap();
        assert!(!tr.blocking_options.is_empty(), "{label} should block");
    }

    // Two-way SELECTED_VS_SELECTED: supported 1D row/col designs translate cleanly.
    let two_way_params = |means_design: &str, mc_type: &str, test: &str, pair_list: serde_json::Value| {
        let mc = match mc_type {
            "NO_CORRECTION" => json!({"type": "NO_CORRECTION"}),
            _ => json!({"type": "STATISTICAL", "statistical": {"test": test}}),
        };
        json!({"content": {
            "experimentalDesign": "ORDINARY_ANOVA",
            "postTests": {
                "type": "PAIRWISE_TESTS",
                "pairwiseTests": {
                    "meansDesignType": means_design,
                    "pairwiseDesign": {"type": "SELECTED_VS_SELECTED", "selectedPairIDList": pair_list},
                    "multipleComparisons": mc,
                }
            }
        }})
    };

    for (label, means_design, mc_type, test, pair_list, want_method, want_pairs, want_factor, want_mode) in [
        (
            "two-way COMPARE_ROWS with null slots",
            "COMPARE_ROWS",
            "STATISTICAL",
            "BONFERRONI_SIDAK",
            json!([row_pair(0, 4), row_pair(0, 1), [null, null], row_pair(4, 5)]),
            MultiplicityMethod::Sidak,
            vec![(0, 4), (0, 1), (4, 5)],
            Some(0),
            FactorialComparisonFamilyMode::Standard,
        ),
        (
            "two-way COMPARE_COLUMNS Bonferroni",
            "COMPARE_COLUMNS",
            "STATISTICAL",
            "BONFERRONI_DUNN",
            json!([col_pair(0, 2), col_pair(1, 3)]),
            MultiplicityMethod::Bonferroni,
            vec![(0, 2), (1, 3)],
            Some(1),
            FactorialComparisonFamilyMode::Standard,
        ),
        (
            "two-way cols-within-rows separate Holm-Sidak",
            "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SEPARATE_FAMILIES",
            "STATISTICAL",
            "HOLM_SIDAK",
            json!([col_pair(0, 1), col_pair(2, 3)]),
            MultiplicityMethod::HolmSidak,
            vec![(0, 1), (2, 3)],
            Some(1),
            FactorialComparisonFamilyMode::PerSliceFamilies,
        ),
        (
            "two-way rows-within-cols single NO_CORRECTION -> FisherLsd",
            "COMPARE_CELLS_BY_ROWS_WITHIN_COLUMNS_SINGLE_FAMILY",
            "NO_CORRECTION",
            "",
            json!([row_pair(0, 1), row_pair(1, 2)]),
            MultiplicityMethod::FisherLsd,
            vec![(0, 1), (1, 2)],
            Some(0),
            FactorialComparisonFamilyMode::TwoFactorFamilies,
        ),
    ] {
        let tr = translate_analysis_parameters("TWO_WAY_ANOVA", &two_way_params(means_design, mc_type, test, pair_list)).unwrap();
        assert!(tr.blocking_options.is_empty(), "{label}: {:?}", tr.blocking_options);
        let AnalysisKind::FactorialAnova(FactorialAnovaSpec { multiplicity, control_factor, comparison_family_mode, .. }) = &tr.kind else {
            panic!("{label}: expected FactorialAnova");
        };
        assert_eq!(multiplicity.method, want_method, "{label}");
        assert_eq!(multiplicity.selected_pairs.as_deref(), Some(want_pairs.as_slice()), "{label}");
        assert_eq!(*control_factor, want_factor, "{label}");
        assert_eq!(*comparison_family_mode, want_mode, "{label}");
        if want_factor == Some(1) {
            assert!(check_control_in_selection(&tr.kind, &[0, 1, 2, 3]).is_ok(), "{label} all cols present");
            assert!(check_control_in_selection(&tr.kind, &[0, 1, 2]).is_err(), "{label} col 3 missing");
        } else {
            assert!(check_control_in_selection(&tr.kind, &[99]).is_ok(), "{label} row pairs independent of selected cols");
        }
    }

    // Blocked two-way SELECTED_VS_SELECTED cases: 2D COMPARE_CELLS, STATISTICAL+NO_TEST, empty NO_CORRECTION, axis mismatch, DUNN.
    let cell_pair = json!([[{"col": {"idx": 0}, "row": {"idx": 0}}, {"col": {"idx": 1}, "row": {"idx": 0}}]]);
    for (label, means_design, mc_type, test, pair_list) in [
        ("two-way 2D COMPARE_CELLS blocks", "COMPARE_CELLS", "STATISTICAL", "BONFERRONI_SIDAK", cell_pair),
        ("two-way STATISTICAL NO_TEST blocks", "COMPARE_COLUMNS", "STATISTICAL", "NO_TEST", json!([col_pair(0, 1)])),
        ("two-way NO_CORRECTION empty list blocks", "COMPARE_CELLS_DIFFER_BY_ONE_FACTOR_SINGLE_FAMILY", "NO_CORRECTION", "", json!([])),
        ("two-way axis mismatch blocks", "COMPARE_COLUMNS", "STATISTICAL", "BONFERRONI_SIDAK", json!([row_pair(0, 1)])),
        ("two-way DUNN blocks", "COMPARE_COLUMNS", "STATISTICAL", "DUNN", json!([col_pair(0, 1)])),
    ] {
        let tr = translate_analysis_parameters("TWO_WAY_ANOVA", &two_way_params(means_design, mc_type, test, pair_list)).unwrap();
        assert!(!tr.blocking_options.is_empty(), "{label} should block");
    }
}

#[test]
fn external_archive_cached_result_crosswalk_scopes_signs_and_skips_unmatched() {
    // Every link names a known mapping row, a non-empty path, and a non-empty CurveBook key.
    for l in CACHED_SCALAR_LINKS {
        assert!(find_analysis_mapping(l.foreign_class).is_some(), "{}", l.cached_path);
        assert!(!l.cached_path.is_empty() && !l.curvebook_key.is_empty());
    }

    // Per-data-set values inherit `id.col.idx` from the nearest enclosing entry.
    let results = json!({"content": {
        "samples": {
            "u-1": {"id": {"col": {"idx": 2}}, "value": {"mean": 4.0, "n": 3}},
            "u-2": {"id": {"col": {"idx": 0}}, "value": {"mean": 7.5, "n": 4}},
        },
        "unpairedTTestModel": {"unpairedTTest": {"t": -2.5, "pValue": {"value": 0.04}}},
    }});
    let mean_link = CACHED_SCALAR_LINKS.iter().find(|l| l.foreign_class == "DESCRIPTIVE_STATISTICS" && l.curvebook_key == "Mean").unwrap();
    let mut means = cached_values_for_link(mean_link, &results);
    means.sort_by_key(|m| m.0);
    assert_eq!(means, vec![(Some(0), 7.5), (Some(2), 4.0)]);

    let scalar = |key: &str, ds: Option<usize>, value: f64| ResultScalarEntry { key: key.to_owned(), data_set_index: ds, value, units: None };
    let sheet = |scalars: Vec<ResultScalarEntry>| {
        let spec = AnalysisSpec::new("res", "src", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
        let mut rs = ResultSheet::new(spec).unwrap();
        rs.scalars = scalars;
        rs
    };

    // Matching scope agrees; a scalar on a data set with no cached value is not compared.
    let rs = sheet(vec![scalar("Mean", Some(0), 7.5), scalar("Mean", Some(2), 4.0), scalar("Mean", Some(5), 1.0)]);
    let diag = compare_recomputed_against_cached_results(&rs, "DESCRIPTIVE_STATISTICS", &results, 1e-9, 1e-6);
    assert_eq!(diag.compared_by_key.get("Mean"), Some(&2));
    assert!(diag.disagreements.is_empty());

    // A wrong data-set scope never borrows another column's value.
    let rs = sheet(vec![scalar("Mean", Some(0), 4.0)]);
    let diag = compare_recomputed_against_cached_results(&rs, "DESCRIPTIVE_STATISTICS", &results, 1e-9, 1e-6);
    assert_eq!((diag.compared_count, diag.disagreements.len()), (1, 1));

    // `t` is compared by magnitude (subtraction order), the p-value by signed value.
    let rs = sheet(vec![scalar("t", None, 2.5), scalar("PValue", None, 0.04)]);
    let diag = compare_recomputed_against_cached_results(&rs, "T_TESTS_AND_NONPARAMETRIC", &results, 1e-9, 1e-6);
    assert_eq!((diag.compared_count, diag.disagreements.len()), (2, 0));
    let rs = sheet(vec![scalar("PValue", None, 0.08)]);
    let diag = compare_recomputed_against_cached_results(&rs, "T_TESTS_AND_NONPARAMETRIC", &results, 1e-9, 1e-6);
    assert_eq!(diag.disagreements.len(), 1);
    assert_eq!(diag.disagreements[0].symbol, "p");
    assert_eq!(diag.disagreements[0].quantity_label, "P value");
    assert_eq!(diag.disagreements[0].bound, CachedBound::Equal);
}

fn make_test_result_sheet(scalars: Vec<ResultScalarEntry>, pairwise: Vec<PairwiseComparisonRecord>) -> ResultSheet {
    let spec = AnalysisSpec::new("res-pw", "src", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let mut rs = ResultSheet::new(spec).unwrap();
    rs.scalars = scalars;
    rs.pairwise_comparisons = pairwise;
    rs
}

#[test]
fn external_archive_cached_bounds_handle_less_and_greater_inequalities() {
    let scalar = |key: &str, value: f64| ResultScalarEntry { key: key.to_owned(), data_set_index: None, value, units: None };

    let less_json = json!({"content": {
        "ordinaryAnovaModel": {
            "anovaTable": { "colFactor": { "fTest": { "pValue": { "state": "P_VALUE_LESS", "value": 1e-15 } } } }
        }
    }});
    let less_ref = extract_cached_analysis_reference("ONE_WAY_ANOVA", &less_json);
    assert_eq!(less_ref.scalars.len(), 1);
    assert_eq!(less_ref.scalars[0].bound, CachedBound::LessThan);
    assert!(
        compare_recomputed_against_reference(&make_test_result_sheet(vec![scalar("P value", 2e-18)], vec![]), &less_ref, 1e-9, 1e-6)
            .disagreements
            .is_empty()
    );
    let less_bad = compare_recomputed_against_reference(&make_test_result_sheet(vec![scalar("P value", 0.005)], vec![]), &less_ref, 1e-9, 1e-6);
    assert_eq!(less_bad.disagreements.len(), 1);
    assert_eq!(less_bad.disagreements[0].bound, CachedBound::LessThan);

    let greater_json = json!({"content": {
        "ordinaryAnovaModel": {
            "anovaTable": { "colFactor": { "fTest": { "pValue": { "state": "P_VALUE_GREATER", "value": 0.1 } } } }
        }
    }});
    let greater_ref = extract_cached_analysis_reference("ONE_WAY_ANOVA", &greater_json);
    assert_eq!(greater_ref.scalars[0].bound, CachedBound::GreaterThan);
    assert!(
        compare_recomputed_against_reference(&make_test_result_sheet(vec![scalar("P value", 0.42)], vec![]), &greater_ref, 1e-9, 1e-6)
            .disagreements
            .is_empty()
    );
    assert_eq!(
        compare_recomputed_against_reference(&make_test_result_sheet(vec![scalar("P value", 0.02)], vec![]), &greater_ref, 1e-9, 1e-6)
            .disagreements
            .len(),
        1
    );
}

#[test]
fn external_archive_pairwise_crosswalk_matches_unordered_pairs_and_skips_invalid() {
    assert_eq!(CACHED_PAIRWISE_LINKS.len(), 4);
    for link in CACHED_PAIRWISE_LINKS {
        assert!(find_analysis_mapping(link.foreign_class).is_some(), "{}", link.tests_path);
        assert!(!link.tests_path.is_empty());
    }

    let pw_json = json!({"content": {
        "ordinaryAnovaModel": {
            "multipleComparisons": {
                "tests": {
                    "outer-3": {
                        "id": { "col": { "idx": 3 } },
                        "value": {
                            "inner-1": {
                                "id": { "col": { "idx": 1 } },
                                "value": {
                                    "status": "VALID",
                                    "meansDifference": -4.25,
                                    "pValueAdjusted": { "state": "P_VALUE_LESS", "value": 1e-15 },
                                    "pValueIndividual": { "state": "P_VALUE_EQUAL", "value": 0.5 }
                                }
                            },
                            "inner-invalid": {
                                "id": { "col": { "idx": 2 } },
                                "value": {
                                    "status": "INVALID",
                                    "meansDifference": "NaN",
                                    "pValueAdjusted": { "state": "P_VALUE_EQUAL", "value": 0.99 }
                                }
                            },
                            "inner-2way": {
                                "id": { "col": { "idx": 4 }, "row": { "idx": 0 } },
                                "value": {
                                    "status": "VALID",
                                    "meansDifference": 99.0,
                                    "pValueAdjusted": { "state": "P_VALUE_EQUAL", "value": 0.5 }
                                }
                            }
                        }
                    },
                    "outer-2way": {
                        "id": { "col": { "idx": 0 }, "row": { "idx": 1 } },
                        "value": {
                            "inner-5": {
                                "id": { "col": { "idx": 5 } },
                                "value": {
                                    "status": "VALID",
                                    "pValueAdjusted": { "state": "P_VALUE_EQUAL", "value": 0.5 }
                                }
                            }
                        }
                    }
                }
            }
        }
    }});
    let pw_ref = extract_cached_analysis_reference("ONE_WAY_ANOVA", &pw_json);
    assert_eq!(pw_ref.pairwise.len(), 1, "INVALID and (col, row) entries must be skipped");
    assert_eq!((pw_ref.pairwise[0].col_a, pw_ref.pairwise[0].col_b), (3, 1));
    assert_eq!(pw_ref.pairwise[0].p_or_q, Some((1e-15, CachedBound::LessThan, false)));
    assert_eq!(pw_ref.pairwise[0].means_diff, Some(-4.25));

    let matching_sheet = make_test_result_sheet(
        vec![],
        vec![
            PairwiseComparisonRecord {
                group_a: 1,
                group_b: 3,
                label_a: String::from("Ctrl"),
                label_b: String::from("Drug"),
                estimate_diff: Some(4.25),
                raw_p_or_q: 3e-18,
                is_fdr_q: false,
                significant: true,
            },
            PairwiseComparisonRecord {
                group_a: 1,
                group_b: 2,
                label_a: String::from("Ctrl"),
                label_b: String::from("Low"),
                estimate_diff: Some(1.0),
                raw_p_or_q: 0.01,
                is_fdr_q: false,
                significant: true,
            },
        ],
    );
    let diag_ok = compare_recomputed_against_reference(&matching_sheet, &pw_ref, 1e-9, 1e-6);
    assert_eq!((diag_ok.compared_count, diag_ok.pairwise_compared_count, diag_ok.pairwise_disagreements), (2, 2, 0));
    assert!(diag_ok.disagreements.is_empty());
    assert_eq!(diag_ok, compare_recomputed_against_cached_results(&matching_sheet, "ONE_WAY_ANOVA", &pw_json, 1e-9, 1e-6));

    let mismatch_sheet = make_test_result_sheet(
        vec![],
        vec![PairwiseComparisonRecord {
            group_a: 1,
            group_b: 3,
            label_a: String::from("Ctrl"),
            label_b: String::from("Drug"),
            estimate_diff: Some(9.5),
            raw_p_or_q: 0.156,
            is_fdr_q: false,
            significant: false,
        }],
    );
    let diag_bad = compare_recomputed_against_reference(&mismatch_sheet, &pw_ref, 1e-9, 1e-6);
    assert_eq!((diag_bad.pairwise_compared_count, diag_bad.pairwise_disagreements, diag_bad.disagreements.len()), (2, 2, 2));
    assert_eq!((diag_bad.disagreements[0].key.as_str(), diag_bad.disagreements[0].symbol.as_str()), ("Pairwise adjusted P", "p"));
    assert_eq!(diag_bad.disagreements[0].quantity_label, "Ctrl vs Drug adjusted p-value");
    assert_eq!((diag_bad.disagreements[1].key.as_str(), diag_bad.disagreements[1].symbol.as_str()), ("Pairwise mean difference", "|mean diff|"));
    assert_eq!(diag_bad.disagreements[1].quantity_label, "Ctrl vs Drug mean difference");
}

#[test]
fn external_archive_pairwise_crosswalk_fdr_q_and_individual_p() {
    let fdr_kw_json = json!({"content": {
        "kruskalWallisTestModel": {
            "multipleComparisons": {
                "tests": {
                    "u0": {
                        "id": { "col": { "idx": 0 } },
                        "value": {
                            "u1": {
                                "id": { "col": { "idx": 1 } },
                                "value": {
                                    "status": "VALID",
                                    "meansDifference": 123.0,
                                    "qValue": { "state": "P_VALUE_EQUAL", "value": 0.03 }
                                }
                            }
                        }
                    }
                }
            }
        }
    }});
    let fdr_ref = extract_cached_analysis_reference("ONE_WAY_ANOVA", &fdr_kw_json);
    assert_eq!(fdr_ref.pairwise.len(), 1);
    assert_eq!(fdr_ref.pairwise[0].p_or_q, Some((0.03, CachedBound::Equal, true)));
    assert_eq!(fdr_ref.pairwise[0].means_diff, None);

    let fwer_sheet = make_test_result_sheet(
        vec![],
        vec![PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("A"),
            label_b: String::from("B"),
            estimate_diff: None,
            raw_p_or_q: 0.5,
            is_fdr_q: false,
            significant: false,
        }],
    );
    assert_eq!(compare_recomputed_against_reference(&fwer_sheet, &fdr_ref, 1e-9, 1e-6).pairwise_compared_count, 0);

    let fdr_sheet = make_test_result_sheet(
        vec![],
        vec![PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("A"),
            label_b: String::from("B"),
            estimate_diff: None,
            raw_p_or_q: 0.25,
            is_fdr_q: true,
            significant: false,
        }],
    );
    let fdr_diag = compare_recomputed_against_reference(&fdr_sheet, &fdr_ref, 1e-9, 1e-6);
    assert_eq!((fdr_diag.pairwise_compared_count, fdr_diag.pairwise_disagreements), (1, 1));
    assert_eq!((fdr_diag.disagreements[0].key.as_str(), fdr_diag.disagreements[0].symbol.as_str()), ("Pairwise FDR q", "q"));

    let indiv_json = json!({"content": {
        "brownForsytheAnovaModel": {
            "multipleComparisons": {
                "tests": {
                    "u0": {
                        "id": { "col": { "idx": 0 } },
                        "value": {
                            "u1": {
                                "id": { "col": { "idx": 2 } },
                                "value": {
                                    "status": "VALID",
                                    "pValueIndividual": { "state": "P_VALUE_EQUAL", "value": 0.015 }
                                }
                            }
                        }
                    }
                }
            }
        }
    }});
    let indiv_ref = extract_cached_analysis_reference("ONE_WAY_ANOVA", &indiv_json);
    assert_eq!(indiv_ref.pairwise.len(), 1);
    assert_eq!(indiv_ref.pairwise[0].p_or_q, Some((0.015, CachedBound::Equal, false)));
}

#[test]
fn issue_75_normality_tests_crosswalk_links_and_comparison() {
    let nt_links: Vec<_> = CACHED_SCALAR_LINKS.iter().filter(|l| l.foreign_class == "NORMALITY_TESTS").collect();
    assert_eq!(nt_links.len(), 16, "expected 16 NORMALITY_TESTS crosswalk links");
    // Lilliefors KS p-values are deliberately omitted from the crosswalk because cached `.prism`
    // files use Dallal & Wilkinson (1986) censoring (`> 0.10` stored as `0.1`).
    assert!(nt_links.iter().all(|l| !l.cached_path.ends_with("kolmogorovSmirnovTest.pValue.value")));

    let results = json!({"content": {
        "samples": {
            "sample-a": {
                "id": {"col": {"idx": 0}},
                "value": {
                    "normalityModel": {
                        "shapiroWilkTest": {"W": 0.9123, "pValue": {"value": 0.2284, "state": "P_VALUE_EQUAL"}},
                        "dAgostinoPearsonTest": {"K2": 2.451, "pValue": {"value": 0.2936, "state": "P_VALUE_EQUAL"}},
                        "andersonDarlingTest": {"A2": 0.412, "pValue": {"value": 0.2781, "state": "P_VALUE_EQUAL"}},
                        "kolmogorovSmirnovTest": {"distance": 0.175, "pValue": {"value": 0.1, "state": "P_VALUE_GREATER"}}
                    },
                    "lognormalityModel": {
                        "shapiroWilkTest": {"W": 0.9742, "pValue": {"value": 0.9481, "state": "P_VALUE_EQUAL"}},
                        "dAgostinoPearsonTest": {"K2": 0.312, "pValue": {"value": 0.8556, "state": "P_VALUE_EQUAL"}},
                        "andersonDarlingTest": {"A2": 0.181, "pValue": {"value": 0.8924, "state": "P_VALUE_EQUAL"}},
                        "kolmogorovSmirnovTest": {"distance": 0.098, "pValue": {"value": 0.1, "state": "P_VALUE_GREATER"}}
                    },
                    "relativeLikelihoodTest": {
                        "probabilityNormal": 0.12,
                        "probabilityLognormal": 0.88
                    }
                }
            },
            "sample-b": {
                "id": {"col": {"idx": 2}},
                "value": {
                    "normalityModel": {
                        "shapiroWilkTest": {"W": 0.8069, "pValue": {"value": 0.1152, "state": "P_VALUE_EQUAL"}}
                    }
                }
            }
        }
    }});

    let sw_link = CACHED_SCALAR_LINKS.iter().find(|l| l.foreign_class == "NORMALITY_TESTS" && l.curvebook_key == "ShapiroWilkW").unwrap();
    let mut cached_w = cached_values_for_link(sw_link, &results);
    cached_w.sort_by_key(|v| v.0);
    assert_eq!(cached_w, vec![(Some(0), 0.9123), (Some(2), 0.8069)]);

    let scalar = |key: &str, ds: usize, value: f64| ResultScalarEntry { key: key.to_owned(), data_set_index: Some(ds), value, units: None };
    let build_sheet = |scalars: Vec<ResultScalarEntry>| {
        let spec = AnalysisSpec::new("res-nt", "src", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
        let mut rs = ResultSheet::new(spec).unwrap();
        rs.scalars = scalars;
        rs
    };

    // Matching all 16 linked keys on col 0 plus Shapiro-Wilk on col 2 agrees cleanly, even while
    // `KolmogorovSmirnovP` reports CurveBook's continuous Lilliefors probability (0.58 vs cached bound 0.1).
    let agreeing = build_sheet(vec![
        scalar("ShapiroWilkW", 0, 0.9123),
        scalar("ShapiroWilkP", 0, 0.2284),
        scalar("DAgostinoPearsonK2", 0, 2.451),
        scalar("DAgostinoPearsonP", 0, 0.2936),
        scalar("AndersonDarlingA2", 0, 0.412),
        scalar("AndersonDarlingA2Star", 0, 0.449),
        scalar("AndersonDarlingP", 0, 0.2781),
        scalar("KolmogorovSmirnovD", 0, 0.175),
        scalar("KolmogorovSmirnovP", 0, 0.58),
        scalar("LognormalShapiroWilkW", 0, 0.9742),
        scalar("LognormalShapiroWilkP", 0, 0.9481),
        scalar("LognormalDAgostinoPearsonK2", 0, 0.312),
        scalar("LognormalDAgostinoPearsonP", 0, 0.8556),
        scalar("LognormalAndersonDarlingA2", 0, 0.181),
        scalar("LognormalAndersonDarlingA2Star", 0, 0.197),
        scalar("LognormalAndersonDarlingP", 0, 0.8924),
        scalar("LognormalKolmogorovSmirnovD", 0, 0.098),
        scalar("LognormalKolmogorovSmirnovP", 0, 0.91),
        scalar("PNormal", 0, 0.12),
        scalar("PLognormal", 0, 0.88),
        scalar("ShapiroWilkW", 2, 0.8069),
        scalar("ShapiroWilkP", 2, 0.1152),
    ]);
    let row = find_analysis_mapping("NORMALITY_TESTS").unwrap();
    let diag = compare_recomputed_against_cached_results(&agreeing, "NORMALITY_TESTS", &results, row.absolute_tolerance, row.relative_tolerance);
    assert_eq!(diag.compared_count, 18);
    assert!(diag.disagreements.is_empty(), "{:?}", diag.disagreements);
    assert_eq!(diag.compared_by_key.len(), 16);
    assert!(!diag.compared_by_key.contains_key("KolmogorovSmirnovP"));
    assert!(!diag.compared_by_key.contains_key("LognormalKolmogorovSmirnovP"));

    // A stale cached Shapiro-Wilk W on col 2 surfaces as a typed disagreement with checklist warning.
    let stale = build_sheet(vec![scalar("ShapiroWilkW", 2, 0.8500), scalar("ShapiroWilkP", 2, 0.2200)]);
    let diag_stale = compare_recomputed_against_cached_results(&stale, "NORMALITY_TESTS", &results, row.absolute_tolerance, row.relative_tolerance);
    assert_eq!((diag_stale.compared_count, diag_stale.disagreements.len()), (2, 2));
    assert!(build_disagreement_checklist_item("NORMALITY_TESTS", &diag_stale).is_some());

    // A scalar on an uncached data-set index is skipped rather than borrowing col 0 or col 2.
    let uncached_col = build_sheet(vec![scalar("ShapiroWilkW", 1, 0.5000)]);
    let diag_skip =
        compare_recomputed_against_cached_results(&uncached_col, "NORMALITY_TESTS", &results, row.absolute_tolerance, row.relative_tolerance);
    assert_eq!((diag_skip.compared_count, diag_skip.disagreements.len()), (0, 0));
}

#[test]
fn external_archive_analyses_3hop_preferred_result_snapshot() {
    // Unpromoted design (`MANN_WHITNEY`) stays an imported read-only 3-hop snapshot.
    let zip = build_zip_owned(vec![
        (
            "document.json".into(),
            json_bytes(json!({
                "title": "Analysis Project",
                "sheets": { "data": ["d-src"], "analyses": ["an-1"] },
                "sheetAttributesMap": {
                    "d-src": { "title": "Groups" },
                    "an-1": { "title": "Mann-Whitney test of Groups", "highlightColor": "#228844" }
                }
            })),
        ),
        (
            "data/sheets/d-src/sheet.json".into(),
            json_bytes(
                json!({"title": "Groups", "table": {"uid": "t-src", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-a", "ds-b"]}}),
            ),
        ),
        ("data/sets/ds-a.json".into(), json_bytes(json!({"title": "Ctrl", "replicates": [{"valueType": "real"}]}))),
        ("data/sets/ds-b.json".into(), json_bytes(json!({"title": "Treated", "replicates": [{"valueType": "real"}]}))),
        ("data/tables/t-src/data.csv".into(), b"10,20\n12,22\n11,21\n".to_vec()),
        // Hop 1: `analyses/an-1/sheet.json`
        (
            "analyses/an-1/sheet.json".into(),
            json_bytes(json!({
                "uid": "an-1",
                "title": "Mann-Whitney test of Groups",
                "analysisClass": "T_TESTS_AND_NONPARAMETRIC",
                "preferredResultSheet": "rs-main",
                "inputSheets": [{ "uid": "d-src", "title": "Groups" }],
                "resultSheets": [{ "uid": "rs-main", "title": "Tabular results" }]
            })),
        ),
        (
            "analyses/an-1/parameters.json".into(),
            json_bytes(json!({"content": {"experimentalDesign": "MANN_WHITNEY", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}})),
        ),
        (
            "analyses/an-1/results.json".into(),
            json_bytes(json!({"content": {"status": {"error": "", "warning": []}, "tStatistic": 12.2474, "pValue": 0.00025}})),
        ),
        // Hop 2: `analyses/an-1/result_sheets/rs-main.json` -> `dataSheet: "view-1"`
        ("analyses/an-1/result_sheets/rs-main.json".into(), json_bytes(json!({"uid": "rs-main", "name": "Tabular results", "dataSheet": "view-1"}))),
        // Hop 3: `data/sheets/view-1/sheet.json` -> `table.uid: "t-view-1"` -> `data/tables/t-view-1/data.csv`
        (
            "data/sheets/view-1/sheet.json".into(),
            json_bytes(json!({
                "uid": "view-1",
                "flags": ["ANALYSIS_VIEW"],
                "table": {"uid": "t-view-1", "format": "view", "dataFormat": "text", "rowTitlesDataSet": "ds-vrt", "dataSets": ["ds-vcol"]}
            })),
        ),
        ("data/sets/ds-vrt.json".into(), json_bytes(json!({"title": "Metric", "replicates": [{"valueType": "string"}]}))),
        ("data/sets/ds-vcol.json".into(), json_bytes(json!({"title": "Value", "replicates": [{"valueType": "string"}]}))),
        ("data/tables/t-view-1/data.csv".into(), b"P value,0.00025\nt statistic,12.2474\n".to_vec()),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    assert_eq!(report.document.data_sheets.len(), 1);
    assert_eq!(report.document.result_sheets.len(), 1);
    assert!(report.promoted_analyses.is_empty());
    let rs = &report.document.result_sheets[0];
    assert_eq!(rs.status, ResultStatus::Imported);
    let expected_fmt = ".prism".trim_start_matches('.');
    assert!(matches!(rs.provenance, ResultProvenance::Imported { ref source_format, .. } if source_format == expected_fmt));
    let tidy = try_export_result_sheet_tidy_csv(rs, &curvebook_format::CsvOptions::csv()).unwrap();
    assert!(tidy.contains(&format!("imported:{expected_fmt}")), "tidy CSV should emit canonical imported:<format> without leading dot");
    assert!(matches!(rs.analysis.kind, AnalysisKind::ImportedSnapshot(_)));
    assert!(rs.analyzable_data.is_none());
    assert_eq!(rs.analysis.source_sheet_id, Some(SheetId::new("d-src")));
    assert_eq!(rs.color_tag, Some(Rgba8::rgb(0x22, 0x88, 0x44)));
    assert_eq!(rs.tabs.len(), 1);
    assert_eq!(rs.tabs[0].title, "Tabular results");
    assert_eq!(rs.tabs[0].row_labels, vec!["P value", "t statistic"]);
    assert_eq!(rs.tabs[0].rows[0][0], ResultCell::Number { value: 0.00025, prefix: None, units: None });
    assert!(rs.scalars.iter().any(|s| s.key == "tStatistic" && (s.value - 12.2474).abs() < 1e-9));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedAnalysis && w.source == "analyses/an-1"));
}

#[test]
fn external_archive_promoted_analysis_imports_live_with_reference_and_3hop_fallback() {
    let zip = build_zip_owned(vec![
        (
            "document.json".into(),
            json_bytes(json!({
                "title": "Promoted Live Project",
                "sheets": { "data": ["d-src", "d-xy-rep"], "analyses": ["an-live", "an-blocked-out"] },
                "sheetAttributesMap": {
                    "d-src": { "title": "Groups" },
                    "d-xy-rep": { "title": "Replicated XY" },
                    "an-live": { "title": "Unpaired t test of Groups", "highlightColor": "#228844" },
                    "an-blocked-out": { "title": "Outliers on Replicated XY" }
                }
            })),
        ),
        (
            "data/sheets/d-src/sheet.json".into(),
            json_bytes(
                json!({"title": "Groups", "table": {"uid": "t-src", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-a", "ds-b", "ds-c"]}}),
            ),
        ),
        ("data/sets/ds-a.json".into(), json_bytes(json!({"title": "Ctrl", "replicates": [{"valueType": "real"}]}))),
        ("data/sets/ds-b.json".into(), json_bytes(json!({"title": "Drug", "replicates": [{"valueType": "real"}]}))),
        ("data/sets/ds-c.json".into(), json_bytes(json!({"title": "Extra", "replicates": [{"valueType": "real"}]}))),
        ("data/tables/t-src/data.csv".into(), b"10,20,30\n12,22,32\n11,21,31\n".to_vec()),
        (
            "data/sheets/d-xy-rep/sheet.json".into(),
            json_bytes(
                json!({"title": "Replicated XY", "table": {"uid": "t-xy", "format": "xy", "dataFormat": "y_replicates", "replicatesCount": 2, "xDataSet": "ds-x", "dataSets": ["ds-y"]}}),
            ),
        ),
        ("data/sets/ds-x.json".into(), json_bytes(json!({"title": "X", "replicates": [{"valueType": "real"}]}))),
        ("data/sets/ds-y.json".into(), json_bytes(json!({"title": "Y", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        ("data/tables/t-xy/data.csv".into(), b"1,10,12\n2,20,22\n3,30,32\n".to_vec()),
        (
            "analyses/an-live/sheet.json".into(),
            json_bytes(json!({
                "uid": "an-live",
                "title": "Unpaired t test of Groups",
                "analysisClass": "T_TESTS_AND_NONPARAMETRIC",
                "isAllDataAnalyzed": false,
                "inputSheets": [{ "uid": "d-src" }],
                "inputDataSets": [{ "uid": "ds-b" }, { "uid": "ds-a" }],
                "preferredResultSheet": "rs-main",
                "resultSheets": [{ "uid": "rs-main", "title": "Tabular results" }]
            })),
        ),
        (
            "analyses/an-live/parameters.json".into(),
            json_bytes(json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}})),
        ),
        (
            "analyses/an-live/results.json".into(),
            json_bytes(
                json!({"content": {"unpairedTTestModel": {"unpairedTTest": {"t": -12.2474487139, "pValue": {"state": "P_VALUE_EQUAL", "value": 0.0002546}}}}}),
            ),
        ),
        (
            "analyses/an-live/result_sheets/rs-main.json".into(),
            json_bytes(json!({"uid": "rs-main", "name": "Tabular results", "dataSheet": "view-1"})),
        ),
        (
            "data/sheets/view-1/sheet.json".into(),
            json_bytes(json!({
                "uid": "view-1",
                "flags": ["ANALYSIS_VIEW"],
                "table": {"uid": "t-view-1", "format": "view", "dataFormat": "text", "rowTitlesDataSet": "ds-vrt", "dataSets": ["ds-vcol"]}
            })),
        ),
        ("data/sets/ds-vrt.json".into(), json_bytes(json!({"title": "Metric", "replicates": [{"valueType": "string"}]}))),
        ("data/sets/ds-vcol.json".into(), json_bytes(json!({"title": "Value", "replicates": [{"valueType": "string"}]}))),
        ("data/tables/t-view-1/data.csv".into(), b"P value,0.0002546\n".to_vec()),
        // IDENTIFY_OUTLIERS with AVERAGE on a multi-subcolumn XY table MUST stay read-only snapshot.
        (
            "analyses/an-blocked-out/sheet.json".into(),
            json_bytes(json!({
                "uid": "an-blocked-out",
                "title": "Outliers on Replicated XY",
                "analysisClass": "IDENTIFY_OUTLIERS",
                "inputSheets": [{ "uid": "d-xy-rep" }]
            })),
        ),
        (
            "analyses/an-blocked-out/parameters.json".into(),
            json_bytes(json!({"content": {"calc": {"method": "ROUT", "q": 1.0}, "data": {"handleReplicatesAs": "AVERAGE"}}})),
        ),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    assert_eq!(report.document.result_sheets.len(), 2);
    assert_eq!(report.promoted_analyses.len(), 1);

    let live = &report.document.result_sheets[0];
    assert_eq!(live.status, ResultStatus::Dirty);
    assert_eq!(live.provenance, ResultProvenance::Computed);
    assert!(matches!(live.analysis.kind, AnalysisKind::TwoGroupComparison(_)));
    assert_eq!(live.analysis.selected_data_sets, vec![1, 0]);
    assert_eq!(live.color_tag, Some(Rgba8::rgb(0x22, 0x88, 0x44)));
    assert_eq!(live.analysis.custom_title.as_deref(), Some("Unpaired t test of Groups"));
    assert!(!report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedAnalysis && w.source == "analyses/an-live"));

    let pref = &report.promoted_analyses[0];
    assert_eq!(pref.sheet_id, live.id);
    assert_eq!(pref.analysis_uid, "an-live");
    assert_eq!(pref.foreign_class, "T_TESTS_AND_NONPARAMETRIC");
    assert_eq!(pref.reference.scalars.len(), 2);
    assert_eq!(pref.fallback.original_kind_label, "T_TESTS_AND_NONPARAMETRIC");
    assert_eq!(pref.fallback.tabs.len(), 1);
    assert_eq!(pref.fallback.tabs[0].title, "Tabular results");
    assert!(!pref.fallback.scalars.is_empty());

    let blocked = &report.document.result_sheets[1];
    assert_eq!(blocked.status, ResultStatus::Imported);
    assert!(matches!(blocked.provenance, ResultProvenance::Imported { .. }));
    assert!(report.warnings.iter().any(|w| {
        w.kind == ImportWarningKind::UnsupportedAnalysis && w.source == "analyses/an-blocked-out" && w.message.contains("multi-subcolumn")
    }));
}

#[test]
fn external_archive_graphs_layouts_preview_png_media_and_unread_opaque_chunks() {
    let png_graph = encode_rgba_png(4, 3);
    let png_layout = encode_rgba_png(6, 5);
    let png_media = encode_rgba_png(2, 2);

    let zip = build_zip_owned(vec![
        (
            "document.json".into(),
            json_bytes(json!({
                "title": "Visual Previews",
                "sheets": { "data": [], "graphs": ["g1", "g2"], "layouts": ["l1"] },
                "sheetAttributesMap": {
                    "g1": { "title": "Dose Plot" },
                    "g2": { "title": "Corrupt Preview Plot" },
                    "l1": { "title": "Figure 1", "highlightColor": "#112233" }
                }
            })),
        ),
        (
            "graphs/g1/sheet.json".into(),
            json_bytes(json!({
                "uid": "g1",
                "title": "Dose Plot",
                "mediaCollection": [{ "representations": [{ "type": "bitmap", "format": "png", "uid": "m-extra" }] }]
            })),
        ),
        ("graphs/g1/preview.png".into(), png_graph),
        ("graphs/g1/data.bin".into(), b"PCFFGRA4opaque".to_vec()),
        ("graphs/g2/sheet.json".into(), json_bytes(json!({"uid": "g2", "title": "Corrupt Preview Plot"}))),
        ("graphs/g2/preview.png".into(), b"not-a-valid-png".to_vec()),
        ("graphs/g2/data.bin".into(), b"PCFFGRA4opaque".to_vec()),
        ("layouts/l1/sheet.json".into(), json_bytes(json!({"uid": "l1", "title": "Figure 1"}))),
        ("layouts/l1/preview.png".into(), png_layout),
        ("layouts/l1/data.bin".into(), b"PCFFGRA4layout".to_vec()),
        ("media/m-extra.png".into(), png_media),
        // Unread opaque chunks that must NOT fail import and must be reported via ZipIndex without decompressing
        ("data/tables/t1/data.dt".into(), b"0123456789abcdef0123456789abcdef".to_vec()),
        ("misc/used_fonts.bin".into(), b"PCFF \0\0\0fonts".to_vec()),
        ("cache/data.cache".into(), b"cached-bytes".to_vec()),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    let doc = &report.document;
    // 3 distinct valid PNGs ingested (graph preview, extra media, layout preview); corrupt g2 skipped non-fatally.
    assert_eq!(doc.image_assets.len(), 3);
    assert_eq!(doc.layout_sheets.len(), 1);
    let ls = &doc.layout_sheets[0];
    assert_eq!(ls.title, "Figure 1");
    assert_eq!(ls.color_tag, Some(Rgba8::rgb(0x11, 0x22, 0x33)));
    assert!(matches!(ls.spec.items.first(), Some(LayoutItem::EmbeddedImage { .. })));

    let graph_layout_warns = report.warnings.iter().filter(|w| w.kind == ImportWarningKind::UnsupportedGraphOrLayout).count();
    assert_eq!(graph_layout_warns, 3);
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk && w.source == "graphs/g2/preview.png"));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk && w.source == "archive/cache"));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk && w.source == "archive/opaque/auxiliary"));
}

#[test]
fn experimental_archive_writer_fail_closed_deterministic_and_round_trips_all_archetypes() {
    let mut doc = ProjectDocument::new("Experimental Round Trip").unwrap();

    // 1. XY with GeneratedSequence::Arithmetic + MeanLowHigh (asymmetric offsets!) + exclusions + decimals + subcolumn titles
    let mut xy = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 0.0, step: 5.0, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::MeanLowHigh,
        1,
        2,
    )
    .unwrap();
    xy.x_title = String::from("Time");
    xy.row_titles = vec![String::from("R0"), String::from("R1")];
    xy.data_sets[0].title = String::from("Curve A");
    xy.data_sets[0].display_decimals = Some(2);
    xy.data_sets[0].subcolumn_titles = vec![String::from("Mean"), String::from("-Err"), String::from("+Err")];
    // CurveBook MeanLowHigh order: [0: Mean, 1: -Err, 2: +Err]
    xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(1.5), Cell::number(3.5)];
    xy.data_sets[0].cells[1] = vec![Cell { value: CellValue::Number(20.0), excluded: true }, Cell::number(2.0), Cell::number(4.0)];
    let mut xy_sheet = DataSheet::new("xy-1", "Series Asymmetric", TableArchetype::Xy(xy)).unwrap();
    xy_sheet.frozen = true;
    xy_sheet.color_tag = Some(Rgba8::rgb(0x12, 0x34, 0x56));
    doc.add_data_sheet(xy_sheet).unwrap();

    // 2. Column MeanSdN + Grouped + Survival + Contingency + PartsOfWhole + MultipleVariables + Nested
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 1, 1).unwrap();
    col.data_sets[0].cells[0] = vec![Cell::number(12.5), Cell::number(1.2), Cell::number(4.0)];
    doc.add_data_sheet(DataSheet::new("col-1", "Summary Col", TableArchetype::Column(col)).unwrap()).unwrap();

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 1).unwrap();
    grp.row_titles = vec![String::from("Dose 1")];
    grp.data_sets[0].cells[0] = vec![Cell::number(5.0), Cell::number(6.0)];
    doc.add_data_sheet(DataSheet::new("grp-1", "Two-Way", TableArchetype::Grouped(grp)).unwrap()).unwrap();

    let mut surv = SurvivalTable::new(1, 2).unwrap();
    surv.times = vec![Cell::number(3.0), Cell::number(9.0)];
    surv.cohorts[0].cells[0] = vec![Cell::number(1.0)];
    surv.cohorts[0].cells[1] = vec![Cell::number(0.0)];
    doc.add_data_sheet(DataSheet::new("surv-1", "KM", TableArchetype::Survival(surv)).unwrap()).unwrap();

    let mut cont = ContingencyTable::new(2, 1).unwrap();
    cont.row_titles = vec![String::from("Row1")];
    cont.data_sets[0].cells[0] = vec![Cell::number(7.0)];
    cont.data_sets[1].cells[0] = vec![Cell::number(3.0)];
    doc.add_data_sheet(DataSheet::new("cont-1", "Counts", TableArchetype::Contingency(cont)).unwrap()).unwrap();

    let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
    pow.slice_labels = vec![String::from("A"), String::from("B")];
    pow.data_sets[0].cells[0] = vec![Cell::number(40.0)];
    pow.data_sets[0].cells[1] = vec![Cell::number(60.0)];
    doc.add_data_sheet(DataSheet::new("pow-1", "Slices", TableArchetype::PartsOfWhole(pow)).unwrap()).unwrap();

    let mut mv = MultipleVariablesTable::new(2, 1).unwrap();
    mv.variables[0].title = String::from("Group");
    mv.variables[0].cells[0] = Cell::text("WT");
    mv.variables[0].refresh_inferred_role();
    mv.variables[1].title = String::from("Score");
    mv.variables[1].cells[0] = Cell::number(88.5);
    doc.add_data_sheet(DataSheet::new("mv-1", "Tidy", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    let mut nest = NestedTable::new(1, 2, 1).unwrap();
    nest.groups[0].title = String::from("Arm 1");
    nest.groups[0].subcolumn_titles = vec![String::from("Bio 1"), String::from("Bio 2")];
    nest.groups[0].cells[0] = vec![Cell::number(3.1), Cell::number(3.9)];
    doc.add_data_sheet(DataSheet::new("nest-1", "Hier", TableArchetype::Nested(nest)).unwrap()).unwrap();

    let mut info = InfoSheet::new_preseeded("info-1", "Lab Notes").unwrap();
    info.constants.clear();
    info.constants.push(InfoConstant::new("Experiment Date", ConstantValue::Date(String::from("2026-11-26")), None).unwrap());
    info.constants.push(InfoConstant::new("Batch", ConstantValue::Number(7.0), None).unwrap());
    info.notes_markdown = String::from("Clean-room experiment");
    info.linked_data_sheets.push(SheetId::new("xy-1"));
    doc.add_info_sheet(info).unwrap();
    doc.select_in_navigator(&SheetId::new("col-1")).unwrap();

    // 1. Fail-closed default!
    assert!(!ArchiveExportOptions::default().experimental);
    assert!(matches!(export_prism_bytes_with_options(&doc, ArchiveExportOptions::default()), Err(FormatError::UnsupportedFormat { .. })));

    // 2. Experimental opt-in is deterministic and emits explicit experimental warning.
    let exp1 = export_prism_bytes_with_options(&doc, ArchiveExportOptions { experimental: true }).unwrap();
    let exp2 = export_prism_bytes_experimental(&doc).unwrap();
    assert_eq!(exp1.bytes, exp2.bytes, "experimental `.prism` output must be byte-deterministic");
    assert!(exp1.warnings.iter().any(|w| w.message.contains("experimental")));

    // Verify wire CSV for `xy-1` has `[rowTitle, Y, +Err, -Err]` (0 X cols for `series` AND swapped offsets!).
    let archive = ZipArchive::from_bytes(&exp1.bytes).unwrap();
    let doc_text = archive.get_utf8("document.json").unwrap().unwrap();
    assert!(!doc_text.contains("createdBy"), "must omit vendor/user/timestamp fields on §6.3 exclusion list");
    let xy_csv = archive.get_utf8("data/tables/tbl-1/data.csv").unwrap().unwrap();
    assert_eq!(xy_csv.lines().next().unwrap(), "R0,10,3.5,1.5");

    // 3. Re-import through CurveBook's `.prism` importer and assert exact round-trip fidelity!
    let re = import_prism_bytes(&exp1.bytes).unwrap();
    assert_eq!(re.document.title, "Experimental Round Trip");
    assert_eq!(re.document.data_sheets.len(), 8);
    assert_eq!(re.document.info_sheets.len(), 1);
    assert_eq!(re.document.active_sheet_id, Some(SheetId::new("data-2")));

    let re_xy_sheet = &re.document.data_sheets[0];
    assert!(re_xy_sheet.frozen);
    assert_eq!(re_xy_sheet.color_tag, Some(Rgba8::rgb(0x12, 0x34, 0x56)));
    let TableArchetype::Xy(re_xy) = &re_xy_sheet.archetype else { panic!() };
    assert_eq!(re_xy.x_mode, XColumnMode::GeneratedSequence { start: 0.0, step: 5.0, progression: SequenceProgression::Arithmetic });
    assert_eq!(re_xy.y_subcolumn_mode, SubcolumnMode::MeanLowHigh);
    assert_eq!(re_xy.data_sets[0].display_decimals, Some(2));
    assert_eq!(re_xy.data_sets[0].subcolumn_titles, vec!["Mean", "-Err", "+Err"]);
    assert_eq!(re_xy.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(10.0));
    assert_eq!(re_xy.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(1.5));
    assert_eq!(re_xy.data_sets[0].cell(0, 2).unwrap().value, CellValue::Number(3.5));
    assert!(re_xy.data_sets[0].cell(1, 0).unwrap().excluded);
    assert_eq!(re.document.info_sheets[0].constants[0].value, ConstantValue::Date(String::from("2026-11-26")));
}

#[test]
fn external_interop_zip_normalization_and_post_normalization_collision_rejected() {
    // Benign `./` and `\` paths normalize cleanly in `.prism` import.
    let norm_zip = build_raw_zip(
        &[
            RawZipEntry::stored(b"./document.json", br#"{"title":"Win Paths","sheets":{"data":["s1"]}}"#),
            RawZipEntry::stored(
                b"data\\sheets\\s1\\sheet.json",
                br#"{"title":"T","table":{"uid":"t1","format":"column","dataFormat":"y_single","dataSets":["d1"]}}"#,
            ),
            RawZipEntry::stored(b"data\\sets\\d1.json", br#"{"title":"A","replicates":[{"valueType":"real"}]}"#),
            RawZipEntry::stored(b".\\data\\tables\\t1\\data.csv", b"42\n"),
        ],
        b"",
    );
    let rep = import_prism_bytes(&norm_zip).unwrap();
    assert_eq!(rep.document.title, "Win Paths");
    assert_eq!(rep.document.data_sheets.len(), 1);

    // Post-normalization collision (`data\a.csv` vs `data/a.csv`) MUST fail with `DuplicateEntry`.
    let collide_zip = build_raw_zip(&[RawZipEntry::stored(b"data\\a.csv", b"X\n1\n"), RawZipEntry::stored(b"data/a.csv", b"X\n2\n")], b"");
    assert!(matches!(import_prism_bytes(&collide_zip), Err(FormatError::DuplicateEntry { .. })));
}

#[test]
fn external_archive_rejects_pre_allocation_cell_budget_blowup_without_oom() {
    // 60,000 empty lines (~60 KiB) x 200 repeated datasets x 3 subcolumns = 36,000,000 cells,
    // which exceeds MAX_SHEET_CELLS / MAX_PROJECT_CELLS and must be rejected BEFORE dense allocation.
    let repeated_ds: Vec<&str> = vec!["ds-y"; 200];
    let doc_json = json_bytes(json!({
        "title": "Budget Guard",
        "sheets": { "data": ["s-huge"] }
    }));
    let sheet_json = json_bytes(json!({
        "uid": "s-huge",
        "title": "Huge",
        "table": {
            "uid": "t-huge",
            "format": "grouped",
            "dataFormat": "y_replicates",
            "replicatesCount": 3,
            "dataSets": repeated_ds
        }
    }));
    let ds_json = json_bytes(json!({
        "title": "Col",
        "replicates": [{"valueType": "real"}, {"valueType": "real"}, {"valueType": "real"}]
    }));
    let csv = "\n".repeat(60_000).into_bytes();

    let zip = build_zip_owned(vec![
        ("document.json".into(), doc_json),
        ("data/sheets/s-huge/sheet.json".into(), sheet_json),
        ("data/sets/ds-y.json".into(), ds_json),
        ("data/tables/t-huge/data.csv".into(), csv),
    ]);
    let res = import_prism_bytes(&zip);
    assert!(
        matches!(res, Err(FormatError::InvalidPipeline { .. }) | Err(FormatError::Doc(curvebook_doc::DocError::TooManyCells { .. }))),
        "expected pre-allocation cell-budget rejection, got: {res:?}"
    );
}

#[test]
fn experimental_archive_writer_swaps_excluded_flags_for_upper_lower_limits() {
    let mut doc = ProjectDocument::new("Limits Exclusion Swap").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::UpperLowerLimits, 1, 1).unwrap();
    xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    // CurveBook UpperLowerLimits: [0: Center=10.0, 1: Lower=8.0, 2: Upper=15.0 (excluded)]
    xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(8.0), Cell { value: CellValue::Number(15.0), excluded: true }];
    doc.add_data_sheet(DataSheet::new("xy-lim", "Limits", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let exported = export_prism_bytes_experimental(&doc).unwrap();
    let archive = ZipArchive::from_bytes(&exported.bytes).unwrap();
    let ds_json: serde_json::Value = serde_json::from_slice(archive.get("data/sets/set-1-y0.json").unwrap()).unwrap();
    // In `.prism` `y_high_low`: replicate 1 is +Err (Upper - Center), replicate 2 is -Err (Center - Lower).
    assert!(ds_json["replicates"][1]["cellAttributes"].is_array(), "excluded Upper must map to replicate 1 (+Err)");
    assert!(ds_json["replicates"][2]["cellAttributes"].is_null(), "non-excluded Lower must stay unmasked on replicate 2 (-Err)");

    let re = import_prism_bytes(&exported.bytes).unwrap();
    let TableArchetype::Xy(re_xy) = &re.document.data_sheets[0].archetype else { panic!() };
    assert_eq!(re_xy.y_subcolumn_mode, SubcolumnMode::MeanLowHigh);
    assert_eq!(re_xy.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(10.0));
    assert_eq!(re_xy.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(2.0));
    assert!(!re_xy.data_sets[0].cell(0, 1).unwrap().excluded);
    assert_eq!(re_xy.data_sets[0].cell(0, 2).unwrap().value, CellValue::Number(5.0));
    assert!(re_xy.data_sets[0].cell(0, 2).unwrap().excluded);
}

#[test]
fn external_archive_heterogeneous_per_dataset_replicate_widths_never_shift_columns() {
    let zip = build_zip_owned(vec![
        ("document.json".into(), json_bytes(json!({ "title": "Heterogeneous Widths", "sheets": { "data": ["s-rep", "s-pm"] } }))),
        // Dataset 0 has 2 replicates, dataset 1 has 3 replicates (table replicatesCount=2 < max 3).
        (
            "data/sheets/s-rep/sheet.json".into(),
            json_bytes(json!({
                "title": "Mixed Reps",
                "table": {
                    "uid": "t-rep", "format": "grouped", "dataFormat": "y_replicates", "replicatesCount": 2,
                    "dataSets": ["ds-r2", "ds-r3"]
                }
            })),
        ),
        ("data/sets/ds-r2.json".into(), json_bytes(json!({"title": "Two", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        (
            "data/sets/ds-r3.json".into(),
            json_bytes(json!({"title": "Three", "replicates": [{"valueType": "real"}, {"valueType": "real"}, {"valueType": "real"}]})),
        ),
        ("data/tables/t-rep/data.csv".into(), b"10,20,100,200,300\n".to_vec()),
        // Mixed y_plus_minus: dataset 0 has 2 CSV cols (symmetric), dataset 1 has 3 CSV cols (asymmetric swap).
        (
            "data/sheets/s-pm/sheet.json".into(),
            json_bytes(json!({
                "title": "Mixed PM",
                "table": {
                    "uid": "t-pm", "format": "xy", "dataFormat": "y_plus_minus", "xDataSet": "ds-xpm",
                    "dataSets": ["ds-pm2", "ds-pm3"]
                }
            })),
        ),
        ("data/sets/ds-xpm.json".into(), json_bytes(json!({"title": "X", "replicates": [{"valueType": "real"}]}))),
        ("data/sets/ds-pm2.json".into(), json_bytes(json!({"title": "Sym", "replicates": [{"valueType": "real"}, {"valueType": "real"}]}))),
        (
            "data/sets/ds-pm3.json".into(),
            json_bytes(json!({"title": "Asym", "replicates": [{"valueType": "real"}, {"valueType": "real"}, {"valueType": "real"}]})),
        ),
        ("data/tables/t-pm/data.csv".into(), b"1,50,3,80,7,2\n".to_vec()),
    ]);

    let rep = import_prism_bytes(&zip).unwrap();
    let TableArchetype::Grouped(grp) = &rep.document.data_sheet(&SheetId::new("s-rep")).unwrap().archetype else { panic!() };
    assert_eq!(grp.subcolumn_mode, SubcolumnMode::RawReplicates(3));
    assert_eq!(grp.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(10.0));
    assert_eq!(grp.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(20.0));
    assert_eq!(grp.data_sets[0].cell(0, 2).unwrap().value, CellValue::Empty);
    assert_eq!(grp.data_sets[1].cell(0, 0).unwrap().value, CellValue::Number(100.0));
    assert_eq!(grp.data_sets[1].cell(0, 1).unwrap().value, CellValue::Number(200.0));
    assert_eq!(grp.data_sets[1].cell(0, 2).unwrap().value, CellValue::Number(300.0));

    let TableArchetype::Xy(xy) = &rep.document.data_sheet(&SheetId::new("s-pm")).unwrap().archetype else { panic!() };
    assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::MeanLowHigh);
    assert_eq!(xy.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(50.0));
    assert_eq!(xy.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(3.0));
    assert_eq!(xy.data_sets[0].cell(0, 2).unwrap().value, CellValue::Number(3.0));
    assert_eq!(xy.data_sets[1].cell(0, 0).unwrap().value, CellValue::Number(80.0));
    assert_eq!(xy.data_sets[1].cell(0, 1).unwrap().value, CellValue::Number(2.0));
    assert_eq!(xy.data_sets[1].cell(0, 2).unwrap().value, CellValue::Number(7.0));
}

#[test]
fn external_archive_rich_text_escapes_stars_and_dollars_preserving_plain_text() {
    let val = json!({
        "string": "Rate *a* and $x=1$ plus <b>raw</b>",
        "version": 1,
        "formatting": [
            { "range": "0~3", "attributes": { "fontWeight": "bold" } }
        ]
    });
    let (markup, plain) = curvebook_format::prism::parse_rich_text_value(&val);
    assert_eq!(plain, "Rate *a* and $x=1$ plus <b>raw</b>");
    assert!(markup.starts_with("<b>Rate</b>"), "got: {markup}");
    assert!(markup.contains(r"\*a*"), "paired star trigger must be escaped: {markup}");
    assert!(markup.contains(r"\$x=1\$"), "paired dollar triggers must be escaped: {markup}");
    assert!(markup.contains(r"\<b>raw\</b>"), "literal markup tags must be escaped: {markup}");

    // Single unpaired `*` (significance marker) stays unescaped.
    let single_star = json!({
        "string": "p < 0.05 *",
        "version": 1,
        "formatting": [{ "range": "0", "attributes": { "italic": true } }]
    });
    let (star_markup, _) = curvebook_format::prism::parse_rich_text_value(&single_star);
    assert_eq!(star_markup, "<i>p</i> < 0.05 *");
}

#[test]
fn external_archive_navigator_fallback_and_chained_analysis_source() {
    // 1. Layout-only archive with no `uiSettings.currentSheets` gets non-None navigator fallback.
    let png = encode_rgba_png(4, 4);
    let layout_only = build_zip_owned(vec![
        ("document.json".into(), json_bytes(json!({ "title": "Layout Only", "sheets": { "data": [], "layouts": ["lay-1"] } }))),
        ("layouts/lay-1/sheet.json".into(), json_bytes(json!({"uid": "lay-1", "title": "Plate Fig"}))),
        ("layouts/lay-1/preview.png".into(), png),
    ]);
    let layout_rep = import_prism_bytes(&layout_only).unwrap();
    assert_eq!(layout_rep.document.layout_sheets.len(), 1);
    assert_eq!(layout_rep.document.active_sheet_id, Some(SheetId::new("lay-1")));
    assert_eq!(layout_rep.document.family_anchor_sheet_id, Some(SheetId::new("lay-1")));

    // 2. Chained analysis `an-2` referencing upstream analysis `an-1` preserves `source_sheet_id`.
    let chained = build_zip_owned(vec![
        (
            "document.json".into(),
            json_bytes(json!({
                "title": "Chained Analyses",
                "sheets": { "data": ["d-1"], "analyses": ["an-1", "an-2"] }
            })),
        ),
        (
            "data/sheets/d-1/sheet.json".into(),
            json_bytes(json!({"title": "Raw", "table": {"uid": "t-1", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-1"]}})),
        ),
        ("data/sets/ds-1.json".into(), json_bytes(json!({"title": "Y", "replicates": [{"valueType": "real"}]}))),
        ("data/tables/t-1/data.csv".into(), b"1\n2\n3\n".to_vec()),
        (
            "analyses/an-1/sheet.json".into(),
            json_bytes(json!({
                "uid": "an-1", "title": "Transform", "analysisClass": "TRNSFORM",
                "inputSheets": [{ "uid": "d-1", "title": "Raw" }]
            })),
        ),
        (
            "analyses/an-2/sheet.json".into(),
            json_bytes(json!({
                "uid": "an-2", "title": "Desc of Transform", "analysisClass": "DESCRIPTIVE_STATISTICS",
                "inputSheets": [{ "uid": "an-1", "title": "Transform" }]
            })),
        ),
    ]);
    let chain_rep = import_prism_bytes(&chained).unwrap();
    assert_eq!(chain_rep.document.result_sheets.len(), 2);
    assert_eq!(chain_rep.document.result_sheets[0].analysis.source_sheet_id, Some(SheetId::new("d-1")));
    assert_eq!(chain_rep.document.result_sheets[1].analysis.source_sheet_id, Some(SheetId::new("an-1")));
}

#[test]
fn external_archive_lazy_zip_index_exceeds_4096_entries_while_native_limits_and_byte_caps_hold() {
    use curvebook_format::open_project_bytes;
    use curvebook_format::prism::import::{MAX_ARCHIVE_ZIP_ENTRIES, external_archive_zip_limits, import_prism_bytes_with_limits};
    use curvebook_format::zip::{MAX_ZIP_ENTRIES, ZipLimits};

    // Native default entry cap MUST remain 4_096 while external `.prism` raises only the lazy
    // central-directory entry cap to 32_768 with identical cumulative decompressed byte bounds.
    assert_eq!(MAX_ZIP_ENTRIES, 4_096);
    assert_eq!(ZipLimits::default().max_entries, 4_096);
    assert_eq!(MAX_ARCHIVE_ZIP_ENTRIES, 32_768);
    assert_eq!(external_archive_zip_limits().max_entries, 32_768);
    assert_eq!(external_archive_zip_limits().max_total_uncompressed_bytes, ZipLimits::default().max_total_uncompressed_bytes);
    assert_eq!(external_archive_zip_limits().max_entry_uncompressed_bytes, ZipLimits::default().max_entry_uncompressed_bytes);

    let mut entries = vec![
        (
            String::from("document.json"),
            json_bytes(json!({
                "formatVersion": "1-6-0",
                "title": "Large Multi-Study Archive",
                "sheets": { "data": ["s-1"], "info": [], "analyses": [], "graphs": [], "layouts": [] },
                "sheetAttributesMap": { "s-1": { "title": "Study 1" } }
            })),
        ),
        (
            String::from("data/sheets/s-1/sheet.json"),
            json_bytes(json!({
                "uid": "s-1",
                "title": "Study 1",
                "table": { "uid": "t-1", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-1"] }
            })),
        ),
        (String::from("data/sets/ds-1.json"), json_bytes(json!({ "title": "Arm A", "replicates": [{ "valueType": "real" }] }))),
        (String::from("data/tables/t-1/data.csv"), b"12.5\n25.0\n".to_vec()),
    ];
    // Add 4,200 unread opaque digest stubs so total entry count exceeds native MAX_ZIP_ENTRIES (4,096).
    for i in 0..4_200_usize {
        entries.push((format!("data/digests/stub-{i:05}.dt"), b"ok".to_vec()));
    }
    let raw_entries: Vec<RawZipEntry<'_>> = entries.iter().map(|(k, v)| RawZipEntry::stored(k.as_bytes(), v.as_slice())).collect();
    let big_zip = build_raw_zip(&raw_entries, b"");

    // 1. `.prism` lazy importer succeeds cleanly across >4,096 entries.
    let rep = import_prism_bytes(&big_zip).unwrap();
    assert_eq!(rep.document.data_sheets.len(), 1);
    let TableArchetype::Column(ref col) = rep.document.data_sheets[0].archetype else { panic!("expected Column") };
    assert_eq!(col.data_sets[0].cell(0, 0).unwrap().active_number(), Some(12.5));
    assert_eq!(col.data_sets[0].cell(1, 0).unwrap().active_number(), Some(25.0));

    // 2. Native `.curvebook` opener STILL enforces the 4,096-entry ZipLimits::default() cap!
    let native_err = open_project_bytes(&big_zip).unwrap_err();
    assert!(matches!(native_err, FormatError::ZipBomb { .. }), "expected ZipBomb for >4,096 native entries, got {native_err:?}");

    // 3. Cumulative *decompressed* byte cap is still strictly enforced on `.prism` reads!
    let tiny_byte_cap = ZipLimits { max_total_uncompressed_bytes: 64, ..external_archive_zip_limits() };
    let cap_err = import_prism_bytes_with_limits(&big_zip, tiny_byte_cap).unwrap_err();
    assert!(matches!(cap_err, FormatError::ZipBomb { .. }), "expected ZipBomb for decompressed byte cap, got {cap_err:?}");
}

#[test]
fn issue_72_kruskal_wallis_crosswalk_links_statistic_and_pvalue() {
    let has_h = CACHED_SCALAR_LINKS.iter().any(|l| {
        l.foreign_class == "ONE_WAY_ANOVA"
            && l.cached_path == "kruskalWallisTestModel.kruskalWallisTest.statistic"
            && l.curvebook_key == "Kruskal-Wallis H"
    });
    let has_p = CACHED_SCALAR_LINKS.iter().any(|l| {
        l.foreign_class == "ONE_WAY_ANOVA" && l.cached_path == "kruskalWallisTestModel.kruskalWallisTest.pValue.value" && l.curvebook_key == "P value"
    });
    assert!(has_h && has_p, "ONE_WAY_ANOVA crosswalk must link both Kruskal-Wallis H and P value");

    let results = json!({"content": {
        "kruskalWallisTestModel": {
            "kruskalWallisTest": {
                "statistic": 7.2,
                "pValue": { "value": 1.0 / 280.0 }
            }
        }
    }});
    let spec = AnalysisSpec::new(
        "res",
        "src",
        AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec {
            method: curvebook_doc::OneWayMethod::KruskalWallis,
            ..curvebook_doc::OneWayAnovaSpec::default()
        }),
    )
    .unwrap();
    let mut rs = ResultSheet::new(spec).unwrap();
    rs.scalars = vec![
        ResultScalarEntry { key: String::from("Kruskal-Wallis H"), data_set_index: None, value: 7.2, units: None },
        ResultScalarEntry { key: String::from("P value"), data_set_index: None, value: 1.0 / 280.0, units: None },
    ];
    let diag = compare_recomputed_against_cached_results(&rs, "ONE_WAY_ANOVA", &results, 1e-6, 1e-4);
    assert_eq!(diag.compared_count, 2);
    assert!(diag.disagreements.is_empty(), "exact H and P should agree: {:?}", diag.disagreements);
}
