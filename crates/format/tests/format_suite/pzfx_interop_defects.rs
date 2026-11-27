//! Synthetic regression tests for real-file `.pzfx` and `.prism` compatibility defects:
//! - No-N 2-subcolumn summary warnings (`SD`/`SE`/`CV`) on Y and X columns, plus contradicting
//!   `Subcolumns` declarations.
//! - Binary `PCFF` / `.pzf` refusal before UTF-8 decoding.
//! - External ID-indexed `.prism` dialect routing (refusal instead of headerless CSV data loss).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::path::Path;

use curvebook_doc::{CellValue, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XColumnMode};
use curvebook_format::{
    BINARY_PZF_UNSUPPORTED_MESSAGE, FormatError, FormatRegistry, ImportWarningKind, binary_pzf_unsupported_error, import_prism_bytes,
    import_pzfx_bytes, import_pzfx_str, is_binary_pzf_bytes,
};

use super::common::{build_zip_entries, pzfx_tables, xy_sheet};

#[test]
fn pzfx_no_n_two_subcolumn_summary_formats_map_sd_se_natively_and_warn_on_cv_y_and_x() {
    let xml = pzfx_tables(concat!(
        "<Table ID=\"col-sd\" TableType=\"OneWay\" YFormat=\"SD\">",
        "<YColumn Subcolumns=\"2\"><Title>Dose</Title><Subcolumn><d>12.0</d></Subcolumn><Subcolumn><d>1.5</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"xy-se\" TableType=\"XY\" YFormat=\"SE\">",
        "<XColumn Subcolumns=\"1\"><Subcolumn><d>1</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"2\"><Title>Resp</Title><Subcolumn><d>50</d></Subcolumn><Subcolumn><d>2.5</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"grp-cv\" TableType=\"TwoWay\" YFormat=\"CV\">",
        "<YColumn Subcolumns=\"2\"><Title>Arm</Title><Subcolumn><d>20</d></Subcolumn><Subcolumn><d>10</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"col-sd-empty\" TableType=\"Column\" YFormat=\"SD\">",
        "<YColumn Subcolumns=\"2\"><Title>EmptySD</Title></YColumn></Table>",
        "<Table ID=\"xy-xerr-corpus\" TableType=\"XY\" XFormat=\"error\">",
        "<XColumn Subcolumns=\"2\"><Title>Conc</Title><Subcolumn><d>3.0</d></Subcolumn><Subcolumn><d>0.25</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"1\"><Title>Y</Title><Subcolumn><d>60</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"xy-xerr-sd\" TableType=\"XY\" XFormat=\"err\" XEVFormat=\"SD\">",
        "<XColumn Subcolumns=\"2\"><Title>Time</Title><Subcolumn><d>5.0</d></Subcolumn><Subcolumn><d>0.5</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"1\"><Title>Y</Title><Subcolumn><d>100</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"xy-xerr-se\" TableType=\"XY\" XFormat=\"err\" XEVFormat=\"SE\">",
        "<XColumn Subcolumns=\"2\"><Title>Conc</Title><Subcolumn><d>2.0</d></Subcolumn><Subcolumn><d>0.2</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"1\"><Title>Y</Title><Subcolumn><d>80</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"xy-xerr-cv\" TableType=\"XY\" XFormat=\"err\" XEVFormat=\"CV\">",
        "<XColumn Subcolumns=\"2\"><Title>Dil</Title><Subcolumn><d>4.0</d></Subcolumn><Subcolumn><d>5.0</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"1\"><Title>Y</Title><Subcolumn><d>40</d></Subcolumn></YColumn></Table>",
    ));
    let report = import_pzfx_str(&xml).unwrap();
    let doc = &report.document;

    for (id, expected_mode) in [("col-sd", SubcolumnMode::MeanSd), ("xy-se", SubcolumnMode::MeanSem), ("col-sd-empty", SubcolumnMode::MeanSd)] {
        let sheet = doc.data_sheet(&SheetId::new(id)).unwrap();
        match &sheet.archetype {
            TableArchetype::Column(c) => {
                assert_eq!(c.subcolumn_mode, expected_mode, "{id}");
                if !c.data_sets[0].cells.is_empty() {
                    assert_eq!(c.data_sets[0].cell(0, 2), None, "{id} must have 2 subcolumns");
                }
            }
            TableArchetype::Xy(xy) => {
                assert_eq!(xy.y_subcolumn_mode, expected_mode, "{id}");
                assert_eq!(xy.data_sets[0].cell(0, 2), None, "{id} must have 2 subcolumns");
            }
            other => panic!("unexpected archetype for {id}: {other:?}"),
        }
        assert!(!report.warnings.iter().any(|w| w.source == id), "expected native 2-subcolumn `{id}` without warnings, got: {:?}", report.warnings);
    }

    let TableArchetype::Grouped(ref grp_cv) = doc.data_sheet(&SheetId::new("grp-cv")).unwrap().archetype else {
        panic!("expected Grouped for grp-cv");
    };
    assert_eq!(grp_cv.subcolumn_mode, SubcolumnMode::MeanCvN);
    assert_eq!(grp_cv.data_sets[0].cell(0, 1).unwrap().active_number(), Some(10.0));
    assert_eq!(grp_cv.data_sets[0].cell(0, 2).unwrap().value, CellValue::Empty);
    assert!(
        report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "grp-cv" && w.message.contains("without N")),
        "expected CV MissingMetadata warning on `grp-cv`, got: {:?}",
        report.warnings
    );

    for (id, expected_x_mode) in
        [("xy-xerr-corpus", SubcolumnMode::MeanSd), ("xy-xerr-sd", SubcolumnMode::MeanSd), ("xy-xerr-se", SubcolumnMode::MeanSem)]
    {
        let TableArchetype::Xy(ref xy) = doc.data_sheet(&SheetId::new(id)).unwrap().archetype else {
            panic!("expected XY for {id}");
        };
        assert_eq!(xy.x_mode, XColumnMode::Numbers { subcolumn_mode: expected_x_mode }, "{id}");
        assert_eq!(xy.x_cells[0].len(), 2, "{id} must have 2 X subcolumns");
        assert!(!report.warnings.iter().any(|w| w.source == id), "unexpected warnings on `{id}`: {:?}", report.warnings);
    }

    let TableArchetype::Xy(ref xy_cv) = doc.data_sheet(&SheetId::new("xy-xerr-cv")).unwrap().archetype else {
        panic!("expected XY for xy-xerr-cv");
    };
    assert_eq!(xy_cv.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanCvN });
    assert_eq!(xy_cv.x_cells[0][2].value, CellValue::Empty);
    assert!(
        report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "xy-xerr-cv" && w.message.contains("XColumn")),
        "expected XColumn MissingMetadata warning on `xy-xerr-cv`, got: {:?}",
        report.warnings
    );
}

#[test]
fn pzfx_contradicting_declared_subcolumns_emit_typed_warnings_and_follow_declared_count() {
    let xml = pzfx_tables(concat!(
        "<Table ID=\"y-sd-decl3\" TableType=\"XY\" YFormat=\"SD\">",
        "<XColumn Subcolumns=\"1\"><Subcolumn><d>1</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"3\"><Title>Full</Title><Subcolumn><d>10</d></Subcolumn><Subcolumn><d>1</d></Subcolumn><Subcolumn><d>6</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"y-sdn-decl2\" TableType=\"Column\" YFormat=\"SDN\">",
        "<YColumn Subcolumns=\"2\"><Title>Short</Title><Subcolumn><d>10</d></Subcolumn><Subcolumn><d>1</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"y-sd-decl1\" TableType=\"XY\" YFormat=\"SD\">",
        "<XColumn Subcolumns=\"1\"><Subcolumn><d>1</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"1\"><Title>One</Title><Subcolumn><d>9</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"y-rep-decl2\" TableType=\"XY\" YFormat=\"replicates\" Replicates=\"3\">",
        "<XColumn Subcolumns=\"1\"><Subcolumn><d>1</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"2\"><Title>Two</Title><Subcolumn><d>5</d></Subcolumn><Subcolumn><d>6</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"y-single-decl2\" TableType=\"XY\" YFormat=\"single\">",
        "<XColumn Subcolumns=\"1\"><Subcolumn><d>1</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"2\"><Title>Two</Title><Subcolumn><d>5</d></Subcolumn><Subcolumn><d>6</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"x-sd-decl3\" TableType=\"XY\" XFormat=\"err\" XEVFormat=\"SD\">",
        "<XColumn Subcolumns=\"3\"><Subcolumn><d>1</d></Subcolumn><Subcolumn><d>0.1</d></Subcolumn><Subcolumn><d>4</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"1\"><Subcolumn><d>7</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"x-sdn-decl2\" TableType=\"XY\" XFormat=\"err\" XEVFormat=\"SDN\">",
        "<XColumn Subcolumns=\"2\"><Subcolumn><d>1</d></Subcolumn><Subcolumn><d>0.1</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"1\"><Subcolumn><d>7</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"x-sd-decl1\" TableType=\"XY\" XFormat=\"err\" XEVFormat=\"SD\">",
        "<XColumn Subcolumns=\"1\"><Subcolumn><d>1</d></Subcolumn></XColumn>",
        "<YColumn Subcolumns=\"1\"><Subcolumn><d>7</d></Subcolumn></YColumn></Table>",
    ));
    let report = import_pzfx_str(&xml).unwrap();
    let doc = &report.document;

    for id in ["y-sd-decl3", "y-sdn-decl2", "y-sd-decl1", "y-rep-decl2", "y-single-decl2", "x-sd-decl3", "x-sdn-decl2", "x-sd-decl1"] {
        assert!(
            report.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == id && w.message.contains("contradicts")),
            "expected contradiction CoercedValue warning on `{id}`, got: {:?}",
            report.warnings
        );
    }

    let TableArchetype::Xy(ref y_sd3) = doc.data_sheet(&SheetId::new("y-sd-decl3")).unwrap().archetype else { panic!("expected XY") };
    assert_eq!(y_sd3.y_subcolumn_mode, SubcolumnMode::MeanSdN);
    assert_eq!(y_sd3.data_sets[0].cell(0, 2).unwrap().active_number(), Some(6.0));
    assert!(!report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "y-sd-decl3"));

    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "y-sdn-decl2"));

    let TableArchetype::Xy(ref y_sd1) = doc.data_sheet(&SheetId::new("y-sd-decl1")).unwrap().archetype else { panic!("expected XY") };
    assert_eq!(y_sd1.y_subcolumn_mode, SubcolumnMode::Single);

    let TableArchetype::Xy(ref y_rep2) = doc.data_sheet(&SheetId::new("y-rep-decl2")).unwrap().archetype else { panic!("expected XY") };
    assert_eq!(y_rep2.y_subcolumn_mode, SubcolumnMode::RawReplicates(2));

    let TableArchetype::Xy(ref y_s2) = doc.data_sheet(&SheetId::new("y-single-decl2")).unwrap().archetype else { panic!("expected XY") };
    assert_eq!(y_s2.y_subcolumn_mode, SubcolumnMode::RawReplicates(2));

    let TableArchetype::Xy(ref x_sd3) = doc.data_sheet(&SheetId::new("x-sd-decl3")).unwrap().archetype else { panic!("expected XY") };
    assert_eq!(x_sd3.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN });
    assert_eq!(x_sd3.x_cells[0][2].active_number(), Some(4.0));
    assert!(!report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "x-sd-decl3"));

    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "x-sdn-decl2"));

    let TableArchetype::Xy(ref x_sd1) = doc.data_sheet(&SheetId::new("x-sd-decl1")).unwrap().archetype else { panic!("expected XY") };
    assert_eq!(x_sd1.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single });
}

#[test]
fn binary_pcff_pzf_bytes_are_refused_before_utf8_decode_and_pzf_is_never_registered() {
    let pcff_non_utf8 = b"PCFF\x00\x01\xFF\xFE\x80\x90legacy-binary";
    assert!(is_binary_pzf_bytes(pcff_non_utf8));
    assert!(is_binary_pzf_bytes(b"PCFF"));
    assert!(!is_binary_pzf_bytes(b"PCF"));
    assert!(!is_binary_pzf_bytes(b"<PrismFile/>"));
    assert!(!is_binary_pzf_bytes(b"PK\x03\x04"));

    let expected = binary_pzf_unsupported_error();
    assert_eq!(expected, FormatError::UnsupportedFormat { reason: String::from(BINARY_PZF_UNSUPPORTED_MESSAGE) });

    assert_eq!(import_pzfx_bytes(pcff_non_utf8), Err(expected.clone()));
    assert_eq!(import_pzfx_str("PCFFascii"), Err(expected.clone()));
    assert_eq!(import_prism_bytes(pcff_non_utf8), Err(expected));

    let reg = FormatRegistry::default_registry();
    assert!(reg.importer_by_format("pzf").is_none());
    assert!(reg.importer_by_format(".pzf").is_none());
    assert!(reg.importer_for_path(Path::new("legacy.pzf")).is_none());
    assert!(reg.supported_import_formats().iter().all(|desc| desc.id != "pzf" && !desc.extensions.contains(&"pzf")));
}

#[test]
fn external_indexed_archive_dialect_imports_headerless_csv_without_losing_first_row() {
    // Minimized synthetic archive following the ID-indexed `.prism` structure: `document.json` with
    // `sheets.data`/`sheets.analyses` and `sheetAttributesMap`, per-sheet `sheet.json`, dataset JSONs,
    // and headerless `data.csv`.
    let external_doc_json = br#"{
        "sheets": { "data": ["tbl-uid-1"], "analyses": ["an-uid-1"] },
        "sheetAttributesMap": { "tbl-uid-1": { "title": "Dose Curve" } }
    }"#;
    let sheet_json = br#"{
        "title": "Dose Curve",
        "table": {
            "uid": "grid-uid-1",
            "format": "xy",
            "dataFormat": "y_sd_n",
            "xDataSet": "ds-x",
            "xDataSetFormat": "y_single",
            "dataSets": ["ds-y1"]
        }
    }"#;
    let ds_x_json = br#"{"@class":"DataSet","uid":"ds-x","title":"Conc","format":"y_single","replicates":[{"valueType":"real"}]}"#;
    let ds_y1_json = br#"{"@class":"DataSet","uid":"ds-y1","title":"Response","format":"y_sd_n","replicates":[{"valueType":"real"},{"valueType":"real"},{"valueType":"int"}]}"#;
    let headerless_csv = b"1.0,10.5,1.2,3\n2.0,22.0,1.8,3\n";
    let result_csv = b"0.991,4.25\n";

    // Root `document.json` imports cleanly with row 0 intact (never losing headerless first row).
    let root_zip = build_zip_entries(&[
        ("document.json", external_doc_json),
        ("data/sheets/tbl-uid-1/sheet.json", sheet_json),
        ("data/sets/ds-x.json", ds_x_json),
        ("data/sets/ds-y1.json", ds_y1_json),
        ("data/tables/grid-uid-1/data.csv", headerless_csv),
        ("analyses/an-uid-1/result.csv", result_csv),
    ]);
    let rep = import_prism_bytes(&root_zip).unwrap();
    assert_eq!(rep.document.data_sheets.len(), 1);
    let ds = &rep.document.data_sheets[0];
    assert_eq!(ds.title, "Dose Curve");
    let TableArchetype::Xy(xy) = &ds.archetype else {
        panic!("expected XY table, got {:?}", ds.archetype);
    };
    assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::MeanSdN);
    assert_eq!(xy.row_count(), 2);
    assert_eq!(xy.x_value_at(0), Some(1.0));
    assert_eq!(xy.x_value_at(1), Some(2.0));
    assert_eq!(xy.data_sets[0].title, "Response");
    assert_eq!(xy.data_sets[0].cell(0, 0).unwrap().value, CellValue::Number(10.5));
    assert_eq!(xy.data_sets[0].cell(0, 1).unwrap().value, CellValue::Number(1.2));
    assert_eq!(xy.data_sets[0].cell(0, 2).unwrap().value, CellValue::Number(3.0));
    assert_eq!(xy.data_sets[0].cell(1, 0).unwrap().value, CellValue::Number(22.0));

    // Nested `data/document.json` (found anywhere by filename) also routes to the external importer.
    let nested_zip = build_zip_entries(&[
        ("data/document.json", external_doc_json),
        ("data/sheets/tbl-uid-1/sheet.json", sheet_json),
        ("data/sets/ds-x.json", ds_x_json),
        ("data/sets/ds-y1.json", ds_y1_json),
        ("data/tables/grid-uid-1/data.csv", headerless_csv),
        ("preview.png", b"\x89PNG\r\n\x1a\n"),
    ]);
    let nested_rep = import_prism_bytes(&nested_zip).unwrap();
    assert_eq!(nested_rep.document.data_sheets.len(), 1);
    assert_eq!(nested_rep.document.data_sheets[0].archetype.row_count(), 2);

    // `sheetAttributesMap`-only and `sheets.analyses`-only variants stay on the external path and
    // never fall back to `import_csv_pass` for unindexed CSVs.
    let attrs_only_zip =
        build_zip_entries(&[("document.json", br#"{"sheetAttributesMap":{"s1":{"title":"T"}}}"#), ("data/s1/data.csv", headerless_csv)]);
    let attrs_rep = import_prism_bytes(&attrs_only_zip).unwrap();
    assert!(attrs_rep.document.data_sheets.is_empty(), "unlisted CSV must not be imported as a data sheet");

    let analyses_only_zip = build_zip_entries(&[("document.json", br#"{"sheets":{"analyses":["a1"]}}"#), ("analyses/a1/data.csv", result_csv)]);
    let analyses_rep = import_prism_bytes(&analyses_only_zip).unwrap();
    assert!(analyses_rep.document.data_sheets.is_empty(), "analysis CSV must not be imported as a data sheet");

    // Malformed external `document.json` refuses with a typed JSON error instead of falling back to `import_csv_pass`
    let malformed_indexed_zip = build_zip_entries(&[
        ("document.json", br#"{"sheets":{"data":["tbl-uid-1"]},"sheetAttributesMap":{"tbl-uid-1":{"#),
        ("data/sheets/tbl-uid-1/data.csv", headerless_csv),
    ]);
    assert!(matches!(import_prism_bytes(&malformed_indexed_zip), Err(FormatError::Json(_))));

    let truncated_with_sheet_marker_zip = build_zip_entries(&[
        ("document.json", b"{\"truncated"),
        ("data/sheets/tbl-uid-1/sheet.json", sheet_json),
        ("data/sheets/tbl-uid-1/data.csv", headerless_csv),
    ]);
    assert!(matches!(import_prism_bytes(&truncated_with_sheet_marker_zip), Err(FormatError::Json(_))));

    // Control 1: CurveBook-native embedded `document.json` with `data_sheets` still imports cleanly.
    let mut native_doc = ProjectDocument::new("Native Embedded").unwrap();
    native_doc.add_data_sheet(xy_sheet("d1", SubcolumnMode::Single, 1, 2)).unwrap();
    let native_zip = build_zip_entries(&[("document.json", &serde_json::to_vec(&native_doc).unwrap())]);
    let native_rep = import_prism_bytes(&native_zip).unwrap();
    assert_eq!(native_rep.document.title, "Native Embedded");
    assert_eq!(native_rep.document.data_sheets.len(), 1);

    // Control 2: `manifest.json` + headered CSV stays on the generic path and keeps passing.
    let generic_manifest_zip = build_zip_entries(&[
        ("manifest.json", br#"{"title":"Generic Synthetic","sheets":[{"id":"s1","title":"Dose","kind":"xy","csv_path":"sheets/s1.csv"}]}"#),
        ("sheets/s1.csv", b"X,Y\n1,10\n2,20\n"),
    ]);
    let generic_rep = import_prism_bytes(&generic_manifest_zip).unwrap();
    assert_eq!(generic_rep.document.title, "Generic Synthetic");
    assert_eq!(generic_rep.document.data_sheets.len(), 1);

    // Control 3: Plain headered CSV ZIP (no `document.json`) stays on the generic path.
    let plain_csv_zip = build_zip_entries(&[("tables/dose.csv", b"A,B\n1,2\n3,4\n")]);
    assert_eq!(import_prism_bytes(&plain_csv_zip).unwrap().document.data_sheets.len(), 1);
}
