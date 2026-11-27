#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use super::common;

use common::{build_zip_entries, pzfx_tables};
use curvebook_doc::{
    Cell, CellValue, ConstantValue, DataSheet, MAX_TITLE_BYTES, ProjectDocument, SequenceProgression, SheetId, SubcolumnMode, TableArchetype,
    TableKind, XColumnMode,
};
use curvebook_format::{ImportWarning, ImportWarningKind, import_prism_bytes, import_pzfx_str};

fn sheet<'a>(doc: &'a ProjectDocument, id: &str) -> &'a DataSheet {
    doc.data_sheet(&SheetId::new(id)).unwrap()
}

fn assert_warnings_present(warnings: &[ImportWarning], expected: &[(Option<ImportWarningKind>, Option<&str>, Option<&str>)]) {
    for &(kind, source_substr, msg_substr) in expected {
        assert!(
            warnings.iter().any(|w| {
                kind.is_none_or(|k| w.kind == k)
                    && source_substr.is_none_or(|s| w.source.contains(s))
                    && msg_substr.is_none_or(|m| w.message.contains(m))
            }),
            "missing warning ({kind:?}, src={source_substr:?}, msg={msg_substr:?}) in {warnings:?}"
        );
    }
}

#[test]
fn pzfx_hardens_evformat_exclusions_sdn_plusminus_asymmetric_limits_and_titles() {
    let xml = concat!(
        "\u{FEFF}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<PrismFile PrismXMLVersion=\"5.00\" Title=\"Attr Project &copy; 2026\"><Notes>Root project note</Notes>",
        "<InfoSequence><Info ID=\"info-1\" Title=\"Protocol Info\">",
        "<Constant><Name>Conc &frac12;</Name><Value>2,5</Value><Units>uM</Units></Constant>",
        "<Note>Step 1 &rarr; Step 2</Note><Description><![CDATA[Check <= 10 & >= 1]]></Description></Info></InfoSequence>",
        "<TableSequence>",
        "<Table ID=\"xy-ev\" TableType=\"XY\" YFormat=\"replicates\" Replicates=\"2\" EVFormat=\"AsteriskAfterNumber\">",
        "<Title>Asterisk Excluded Replicates</Title><Note>Exclude spiked sample</Note>",
        "<XColumn Title=\"Time (min)\"><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>",
        "<YColumn Title=\"Signal\"><Subcolumn><d>10.0</d><d>12.5*</d></Subcolumn><Subcolumn><d>10.5</d><d Excluded=\"true\">14.0</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"col-sdn\" TableType=\"OneWay\" YFormat=\"SDN\" EVFormat=\"AsteriskBeforeNumber\"><Title>SDN Summary</Title>",
        "<YColumn Subcolumns=\"3\"><Title>Dose</Title><Subcolumn><d>5.0</d></Subcolumn><Subcolumn><d>0.5</d></Subcolumn><Subcolumn><d>*3</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"grp-rf\" TableType=\"Grouped\" Replicates=\"1\">",
        "<RowTitlesColumn><Title>Genotype</Title><d>WT</d><d>KO</d></RowTitlesColumn>",
        "<YColumn Excluded=\"yes\"><Title>Masked</Title><d>*9.5</d><d>11.0</d></YColumn></Table>",
        "</TableSequence></PrismFile>",
    );

    let report = import_pzfx_str(xml).unwrap();
    let doc = &report.document;
    assert_eq!((doc.title.as_str(), doc.info_sheets.len()), ("Attr Project \u{00A9} 2026", 1));

    let info = &doc.info_sheets[0];
    assert_eq!(
        (info.title.as_str(), info.constants[0].name.as_str(), &info.constants[0].value),
        ("Protocol Info", "Conc \u{00BD}", &ConstantValue::Number(2.5))
    );
    assert!(
        info.notes_markdown.contains("Step 1 \u{2192} Step 2")
            && info.notes_markdown.contains("Check <= 10 & >= 1")
            && info.notes_markdown.contains("Root project note")
    );

    let xy_ev = sheet(doc, "xy-ev");
    assert_eq!((xy_ev.floating_notes.len(), xy_ev.floating_notes[0].body_markdown.as_str()), (1, "Exclude spiked sample"));
    let TableArchetype::Xy(ref xy) = xy_ev.archetype else { panic!("expected XY") };
    assert_eq!((xy.x_title.as_str(), xy.y_subcolumn_mode), ("Time (min)", SubcolumnMode::RawReplicates(2)));
    assert_eq!((xy.data_sets[0].cell(1, 0), xy.data_sets[0].cell(1, 1)), (Some(&Cell::excluded_number(12.5)), Some(&Cell::excluded_number(14.0))));
    assert!(!report.warnings.iter().any(|w| w.source == "xy-ev" && w.message.contains("unrecognized YFormat")));

    let TableArchetype::Column(ref col_sdn) = sheet(doc, "col-sdn").archetype else { panic!("expected Column") };
    assert_eq!((col_sdn.subcolumn_mode, col_sdn.data_sets[0].cell(0, 2)), (SubcolumnMode::MeanSdN, Some(&Cell::excluded_number(3.0))));

    let TableArchetype::Grouped(ref grp) = sheet(doc, "grp-rf").archetype else { panic!("expected Grouped") };
    assert_eq!((grp.row_factor_title.as_str(), &grp.row_titles), ("Genotype", &vec![String::from("WT"), String::from("KO")]));
    assert_eq!((grp.data_sets[0].cell(0, 0), grp.data_sets[0].cell(1, 0)), (Some(&Cell::excluded_number(9.5)), Some(&Cell::excluded_number(11.0))));
}

#[test]
fn pzfx_hardens_plusminus_asymmetric_limits_and_missing_n_summary_modes() {
    let xml = pzfx_tables(concat!(
        "<Table ID=\"xy-pm\" TableType=\"XY\" YFormat=\"PlusMinus\"><XColumn><d>1</d><d>2</d></XColumn>",
        "<YColumn Title=\"SymErr\"><Subcolumn><d>10</d><d>20</d></Subcolumn><Subcolumn><d>1.5</d><d>2.5</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"grp-ul\" TableType=\"Grouped\" YFormat=\"upper-lower-limits\"><YColumn>",
        "<Subcolumn><Title>Mean</Title><d>50</d></Subcolumn><Subcolumn><Title>Upper CI</Title><d>62</d></Subcolumn><Subcolumn><Title>Lower CI</Title><d>38</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"col-ul-coerce\" TableType=\"Column\" YFormat=\"upper-lower-limits\"><YColumn><Subcolumn><d>50</d></Subcolumn><Subcolumn><d>38</d></Subcolumn><Subcolumn><d>62</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"col-sd2\" TableType=\"Column\" YFormat=\"SEM\"><YColumn Title=\"MissingN\"><Subcolumn><d>8</d></Subcolumn><Subcolumn><d>0.4</d></Subcolumn></YColumn></Table>",
    ));
    let report = import_pzfx_str(&xml).unwrap();
    let doc = &report.document;

    let TableArchetype::Xy(ref xy_pm) = sheet(doc, "xy-pm").archetype else { panic!("expected XY") };
    assert_eq!(
        (xy_pm.y_subcolumn_mode, xy_pm.data_sets[0].cell(0, 1).unwrap().active_number(), xy_pm.data_sets[0].cell(0, 2).unwrap().active_number()),
        (SubcolumnMode::MeanLowHigh, Some(1.5), Some(1.5))
    );

    let TableArchetype::Grouped(ref grp_ul) = sheet(doc, "grp-ul").archetype else { panic!("expected Grouped") };
    assert_eq!(
        (grp_ul.subcolumn_mode, grp_ul.data_sets[0].cell(0, 1).unwrap().active_number(), grp_ul.data_sets[0].cell(0, 2).unwrap().active_number()),
        (SubcolumnMode::UpperLowerLimits, Some(38.0), Some(62.0))
    );

    let TableArchetype::Column(ref col_ul_c) = sheet(doc, "col-ul-coerce").archetype else { panic!("expected Column") };
    if TableKind::Column.allows_subcolumn_mode(SubcolumnMode::UpperLowerLimits) {
        assert_eq!(
            (
                col_ul_c.subcolumn_mode,
                col_ul_c.data_sets[0].cell(0, 1).unwrap().active_number(),
                col_ul_c.data_sets[0].cell(0, 2).unwrap().active_number()
            ),
            (SubcolumnMode::UpperLowerLimits, Some(38.0), Some(62.0))
        );
    } else {
        assert_eq!(col_ul_c.subcolumn_mode, SubcolumnMode::Single);
        assert_warnings_present(&report.warnings, &[(Some(ImportWarningKind::CoercedValue), Some("col-ul-coerce"), None)]);
    }

    let TableArchetype::Column(ref col_sd2) = sheet(doc, "col-sd2").archetype else { panic!("expected Column") };
    assert_eq!(
        (
            col_sd2.subcolumn_mode,
            col_sd2.data_sets[0].cell(0, 0).unwrap().active_number(),
            col_sd2.data_sets[0].cell(0, 1).unwrap().active_number(),
            col_sd2.data_sets[0].cell(0, 2),
        ),
        (SubcolumnMode::MeanSem, Some(8.0), Some(0.4), None)
    );
    assert!(!report.warnings.iter().any(|w| w.source == "col-sd2"), "unexpected warnings for native MeanSem: {:?}", report.warnings);

    assert_warnings_present(
        &report.warnings,
        &[(Some(ImportWarningKind::CoercedValue), Some("xy-pm"), None), (Some(ImportWarningKind::CoercedValue), Some("grp-ul"), None)],
    );
}

#[test]
fn pzfx_hardens_xformat_locale_numbers_placeholders_hugetables_and_unsupported_sections() {
    let xml = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><PrismFile SchemaVersion=\"14.2\"><TableSequence>",
        "<Table ID=\"xy-text\" TableType=\"XY\" XFormat=\"labels\"><XColumn><Subcolumn><d>Vehicle</d><d>DrugA</d></Subcolumn></XColumn><YColumn><Subcolumn><d>1,25</d><d>\u{2212}3,5e-1</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"xy-date\" TableType=\"XY\" XFormat=\"date\"><XColumn Title=\"Sample Day\"><Subcolumn><d>2026-01-15</d><d>2026-02-01</d></Subcolumn></XColumn><YColumn><Subcolumn><d>42</d><d>#N/A</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"xy-seq\" TableType=\"XY\" XFormat=\"sequence\" Replicates=\"oops\"><XColumn><X0>1,5</X0><DeltaX>0,25</DeltaX></XColumn><YColumn Subcolumns=\"0\"><Subcolumn><d>1.250</d><d>1,234.56</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"cont-bad\" TableType=\"Contingency\"><YColumn Title=\"Counts\"><Subcolumn><d>-2</d><d>3.5</d></Subcolumn></YColumn></Table>",
        "<Table ID=\"surv-bad\" TableType=\"Survival\"><XColumn><Subcolumn><d>-5</d></Subcolumn></XColumn><YColumn Title=\"ArmA\"><Subcolumn><d>2</d></Subcolumn></YColumn></Table>",
        "</TableSequence><HugeTableSequence><HugeTable ID=\"huge-1\"><Title>Large MultiVar</Title><YColumn><Subcolumn><d>1</d><d>x</d></Subcolumn></YColumn></HugeTable></HugeTableSequence>",
        "<AnalysisSequence><Analysis/></AnalysisSequence><UserEquations><Equation Name=\"Hill\"/></UserEquations><Graph ID=\"g1\"/><Script ID=\"s1\"/></PrismFile>",
    );
    let report = import_pzfx_str(xml).unwrap();
    let doc = &report.document;

    let TableArchetype::Xy(ref xy_text) = sheet(doc, "xy-text").archetype else { panic!("expected XY") };
    assert_eq!(xy_text.row_titles, vec!["Vehicle", "DrugA"]);
    assert!((xy_text.data_sets[0].cell(0, 0).unwrap().active_number().unwrap() - 1.25).abs() < 1e-12);
    assert!((xy_text.data_sets[0].cell(1, 0).unwrap().active_number().unwrap() + 0.35).abs() < 1e-12);

    let TableArchetype::Xy(ref xy_date) = sheet(doc, "xy-date").archetype else { panic!("expected XY") };
    assert_eq!(xy_date.x_mode, XColumnMode::DateOrElapsedTime { unit_label: String::from("Sample Day") });
    assert_eq!(xy_date.materialized_x_cell_at(0).value, CellValue::Text(String::from("2026-01-15")));
    assert_eq!(xy_date.data_sets[0].cell(1, 0).unwrap().value, CellValue::Text(String::from("#N/A")));

    let TableArchetype::Xy(ref xy_seq) = sheet(doc, "xy-seq").archetype else { panic!("expected XY") };
    assert_eq!(xy_seq.x_mode, XColumnMode::GeneratedSequence { start: 1.5, step: 0.25, progression: SequenceProgression::Arithmetic });
    assert!((xy_seq.data_sets[0].cell(1, 0).unwrap().active_number().unwrap() - 1234.56).abs() < 1e-9);
    assert_eq!(sheet(doc, "huge-1").archetype.kind(), TableKind::MultipleVariables);

    assert_warnings_present(
        &report.warnings,
        &[
            (Some(ImportWarningKind::MissingMetadata), Some("SchemaVersion"), None),
            (Some(ImportWarningKind::CoercedValue), Some("xy-text"), None),
            (Some(ImportWarningKind::CoercedValue), Some("xy-date"), None),
            (None, Some("xy-seq"), Some("Replicates")),
            (None, Some("xy-seq"), Some("Subcolumns=\"0\"")),
            (Some(ImportWarningKind::CoercedValue), Some("cont-bad"), None),
            (None, Some("surv-bad"), Some("time column")),
            (None, Some("surv-bad"), Some("status codes")),
            (Some(ImportWarningKind::UnsupportedTableType), Some("huge-1"), None),
            (Some(ImportWarningKind::IgnoredSection), Some("AnalysisSequence"), None),
            (Some(ImportWarningKind::UnsupportedEquation), Some("UserEquations"), None),
            (Some(ImportWarningKind::UnsupportedGraphOrLayout), Some("Graph"), None),
            (Some(ImportWarningKind::UnsupportedScript), Some("Script"), None),
        ],
    );
}

#[test]
fn prism_hardens_nested_roots_case_variants_external_doc_json_sqlite_and_metadata_clamping() {
    let doc_json = format!(
        r#"{{"projectTitle":"{}","tables":{{"dose_resp":{{"name":"Dose Curve","tableType":"XY","yFormat":"SDN","csvPath":"./Data/Dose.CSV"}},"missing_ref":{{"title":"Ghost Sheet","kind":"Column","csv_path":"Data/DoesNotExist.CSV"}}}},"info":[{{"title":"Reagent Lot","constants":[{{"name":"{}","value":100,"units":"nM"}}],"notes_markdown":"Freshly prepared"}}],"equations":["Y = Bottom + (Top - Bottom)/(1 + 10^((LogEC50 - X)*HillSlope))"]}}"#,
        "T".repeat(MAX_TITLE_BYTES + 40),
        "K".repeat(MAX_TITLE_BYTES + 25),
    );
    let zip = build_zip_entries(&[
        ("ProjectRoot/Document.JSON", doc_json.as_bytes()),
        ("ProjectRoot/Data/Dose.CSV", "\u{FEFF}\"Row Title\n(Well)\",X,Y_mean,Y_sd,Y_n\n\"A1\",0.1,10,1,3\n\"B1\",1.0,50,2,3\n".as_bytes()),
        (
            "ProjectRoot/Tables/grp/meta.json",
            br#"{"title":"Grouped Sidecar","kind":"Grouped","subcolumn_mode":"plus-minus","replicates":0,"csv_path":"sub.csv"}"#,
        ),
        ("ProjectRoot/Tables/grp/sub.csv", b"Row Factor,G1-,G1+,G2-,G2+\nT0,5,5,8,8\n"),
        ("ProjectRoot/Analyses/fit1.json", br#"{"title":"Nonlinear Fit","model":"4PL"}"#),
        ("ProjectRoot/Graphs/curve_plot.json", br#"{"title":"Main Figure","axes":[]}"#),
        ("ProjectRoot/Scripts/export.json", br#"{"title":"Batch Export","code":"print(1)"}"#),
        ("ProjectRoot/db/store.sqlite", b"SQLite format 3\0mock_sqlite_payload"),
        ("ProjectRoot/misc/extra.json", br#"{"title":"Unknown Plugin Config","foo":true}"#),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    let doc = &report.document;
    assert_eq!(
        (doc.title.len(), doc.info_sheets.len(), doc.info_sheets[0].constants[0].name.len(), doc.data_sheets.len()),
        (MAX_TITLE_BYTES, 1, MAX_TITLE_BYTES, 2)
    );

    let dose = sheet(doc, "dose_resp");
    assert_eq!(dose.title, "Dose Curve");
    let TableArchetype::Xy(ref xy) = dose.archetype else { panic!("expected XY") };
    assert_eq!((xy.y_subcolumn_mode, &xy.row_titles), (SubcolumnMode::MeanSdN, &vec![String::from("A1"), String::from("B1")]));

    assert!(report.warnings.iter().filter(|w| w.kind == ImportWarningKind::TruncatedTitle).count() >= 2);
    assert_warnings_present(
        &report.warnings,
        &[
            (Some(ImportWarningKind::MissingMetadata), Some("Data/DoesNotExist.CSV"), None),
            (Some(ImportWarningKind::CoercedValue), None, None),
            (Some(ImportWarningKind::UnsupportedEquation), None, None),
            (Some(ImportWarningKind::UnsupportedAnalysis), Some("fit1.json"), None),
            (Some(ImportWarningKind::UnsupportedGraphOrLayout), Some("curve_plot.json"), None),
            (Some(ImportWarningKind::UnsupportedScript), Some("export.json"), None),
            (Some(ImportWarningKind::UnsupportedChunk), None, Some("SQLite")),
            (Some(ImportWarningKind::UnsupportedChunk), Some("extra.json"), None),
        ],
    );
}
