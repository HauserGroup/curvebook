#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use curvebook_doc::{
    Cell, CellValue, ConstantValue, DataSheet, MAX_TITLE_BYTES, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TableKind, XColumnMode,
};
use curvebook_format::{ImportWarningKind, ZipWriter, import_prism_bytes, import_pzfx_str};

fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = ZipWriter::new();
    for (name, bytes) in entries {
        zw.add_file(name, bytes.to_vec()).unwrap();
    }
    zw.finish().unwrap()
}

fn sheet<'a>(doc: &'a ProjectDocument, id: &str) -> &'a DataSheet {
    doc.data_sheet(&SheetId::new(id)).unwrap()
}

#[test]
fn pzfx_hardens_evformat_exclusions_sdn_plusminus_asymmetric_limits_and_titles() {
    let xml = concat!(
        "\u{FEFF}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<PrismFile PrismXMLVersion=\"5.00\" Title=\"Attr Project &copy; 2026\">\n",
        "  <Notes>Root project note</Notes>\n",
        "  <InfoSequence>\n",
        "    <Info ID=\"info-1\" Title=\"Protocol Info\">\n",
        "      <Constant><Name>Conc &frac12;</Name><Value>2,5</Value><Units>uM</Units></Constant>\n",
        "      <Note>Step 1 &rarr; Step 2</Note>\n",
        "      <Description><![CDATA[Check <= 10 & >= 1]]></Description>\n",
        "    </Info>\n",
        "  </InfoSequence>\n",
        "  <TableSequence>\n",
        // 1. EVFormat="AsteriskAfterNumber" MUST NOT collide with YFormat="replicates", and must mark 12.5* as excluded
        "    <Table ID=\"xy-ev\" TableType=\"XY\" YFormat=\"replicates\" Replicates=\"2\" EVFormat=\"AsteriskAfterNumber\">\n",
        "      <Title>Asterisk Excluded Replicates</Title>\n",
        "      <Note>Exclude spiked sample</Note>\n",
        "      <XColumn Title=\"Time (min)\"><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>\n",
        "      <YColumn Title=\"Signal\">\n",
        "        <Subcolumn><d>10.0</d><d>12.5*</d></Subcolumn>\n",
        "        <Subcolumn><d>10.5</d><d Excluded=\"true\">14.0</d></Subcolumn>\n",
        "      </YColumn>\n",
        "    </Table>\n",
        // 2. SDN MUST map to MeanSdN (NOT MeanLowHigh!), plus AsteriskBeforeNumber on XY default
        "    <Table ID=\"col-sdn\" TableType=\"OneWay\" YFormat=\"SDN\" EVFormat=\"AsteriskBeforeNumber\">\n",
        "      <Title>SDN Summary</Title>\n",
        "      <YColumn Subcolumns=\"3\"><Title>Dose</Title>\n",
        "        <Subcolumn><d>5.0</d></Subcolumn>\n",
        "        <Subcolumn><d>0.5</d></Subcolumn>\n",
        "        <Subcolumn><d>*3</d></Subcolumn>\n",
        "      </YColumn>\n",
        "    </Table>\n",
        // 3. Grouped table preserves RowTitlesColumn Title into row_factor_title + column-level Excluded="yes"
        "    <Table ID=\"grp-rf\" TableType=\"Grouped\" Replicates=\"1\">\n",
        "      <RowTitlesColumn><Title>Genotype</Title><d>WT</d><d>KO</d></RowTitlesColumn>\n",
        "      <YColumn Excluded=\"yes\"><Title>Masked</Title><d>*9.5</d><d>11.0</d></YColumn>\n",
        "    </Table>\n",
        "  </TableSequence>\n",
        "</PrismFile>",
    );

    let report = import_pzfx_str(xml).unwrap();
    let doc = &report.document;
    assert_eq!(doc.title, "Attr Project \u{00A9} 2026");

    // Info sheet notes & locale constant
    assert_eq!(doc.info_sheets.len(), 1);
    let info = &doc.info_sheets[0];
    assert_eq!(info.title, "Protocol Info");
    assert_eq!(info.constants[0].name, "Conc \u{00BD}");
    assert_eq!(info.constants[0].value, ConstantValue::Number(2.5));
    assert!(info.notes_markdown.contains("Step 1 \u{2192} Step 2"));
    assert!(info.notes_markdown.contains("Check <= 10 & >= 1"));
    assert!(info.notes_markdown.contains("Root project note"));

    // 1. xy-ev: asterisk exclusion + note preserved, NO false YFormat CoercedValue warning
    let xy_ev = sheet(doc, "xy-ev");
    assert_eq!(xy_ev.floating_notes.len(), 1);
    assert_eq!(xy_ev.floating_notes[0].body_markdown, "Exclude spiked sample");
    let TableArchetype::Xy(ref xy) = xy_ev.archetype else { panic!("expected XY") };
    assert_eq!(xy.x_title, "Time (min)");
    assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(2));
    assert_eq!(xy.data_sets[0].cell(1, 0), Some(&Cell::excluded_number(12.5)));
    assert_eq!(xy.data_sets[0].cell(1, 1), Some(&Cell::excluded_number(14.0)));
    assert!(
        !report.warnings.iter().any(|w| w.source == "xy-ev" && w.message.contains("unrecognized YFormat")),
        "unexpected xy-ev warning: {:?}",
        report.warnings
    );

    // 2. col-sdn: SDN maps to MeanSdN and *3 becomes excluded number 3.0
    let TableArchetype::Column(ref col_sdn) = sheet(doc, "col-sdn").archetype else { panic!("expected Column") };
    assert_eq!(col_sdn.subcolumn_mode, SubcolumnMode::MeanSdN);
    assert_eq!(col_sdn.data_sets[0].cell(0, 2), Some(&Cell::excluded_number(3.0)));

    let TableArchetype::Grouped(ref grp) = sheet(doc, "grp-rf").archetype else { panic!("expected Grouped") };
    assert_eq!(grp.row_factor_title, "Genotype");
    assert_eq!(grp.row_titles, vec!["WT", "KO"]);
    assert_eq!(grp.data_sets[0].cell(0, 0), Some(&Cell::excluded_number(9.5)));
    assert_eq!(grp.data_sets[0].cell(1, 0), Some(&Cell::excluded_number(11.0)));
}

#[test]
fn pzfx_hardens_plusminus_asymmetric_limits_and_missing_n_summary_modes() {
    let xml = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<PrismFile>\n",
        "  <TableSequence>\n",
        "    <Table ID=\"xy-pm\" TableType=\"XY\" YFormat=\"PlusMinus\">\n",
        "      <XColumn><d>1</d><d>2</d></XColumn>\n",
        "      <YColumn Title=\"SymErr\">\n",
        "        <Subcolumn><d>10</d><d>20</d></Subcolumn>\n",
        "        <Subcolumn><d>1.5</d><d>2.5</d></Subcolumn>\n",
        "      </YColumn>\n",
        "    </Table>\n",
        "    <Table ID=\"grp-ul\" TableType=\"Grouped\" YFormat=\"upper-lower-limits\">\n",
        "      <YColumn>\n",
        "        <Subcolumn><Title>Mean</Title><d>50</d></Subcolumn>\n",
        "        <Subcolumn><Title>Upper CI</Title><d>62</d></Subcolumn>\n",
        "        <Subcolumn><Title>Lower CI</Title><d>38</d></Subcolumn>\n",
        "      </YColumn>\n",
        "    </Table>\n",
        "    <Table ID=\"col-ul-coerce\" TableType=\"Column\" YFormat=\"upper-lower-limits\">\n",
        "      <YColumn><Subcolumn><d>50</d></Subcolumn><Subcolumn><d>38</d></Subcolumn><Subcolumn><d>62</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        "    <Table ID=\"col-sd2\" TableType=\"Column\" YFormat=\"SEM\">\n",
        "      <YColumn Title=\"MissingN\"><Subcolumn><d>8</d></Subcolumn><Subcolumn><d>0.4</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        "  </TableSequence>\n",
        "</PrismFile>",
    );

    let report = import_pzfx_str(xml).unwrap();
    let doc = &report.document;

    let TableArchetype::Xy(ref xy_pm) = sheet(doc, "xy-pm").archetype else { panic!("expected XY") };
    assert_eq!(xy_pm.y_subcolumn_mode, SubcolumnMode::MeanLowHigh);
    assert_eq!(xy_pm.data_sets[0].cell(0, 1).unwrap().active_number(), Some(1.5));
    assert_eq!(xy_pm.data_sets[0].cell(0, 2).unwrap().active_number(), Some(1.5));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "xy-pm"));

    let TableArchetype::Grouped(ref grp_ul) = sheet(doc, "grp-ul").archetype else { panic!("expected Grouped") };
    assert_eq!(grp_ul.subcolumn_mode, SubcolumnMode::UpperLowerLimits);
    assert_eq!(grp_ul.data_sets[0].cell(0, 1).unwrap().active_number(), Some(38.0));
    assert_eq!(grp_ul.data_sets[0].cell(0, 2).unwrap().active_number(), Some(62.0));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "grp-ul"));

    let TableArchetype::Column(ref col_ul_c) = sheet(doc, "col-ul-coerce").archetype else { panic!("expected Column") };
    assert_eq!(col_ul_c.subcolumn_mode, SubcolumnMode::Single);
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "col-ul-coerce"));

    let TableArchetype::Column(ref col_sd2) = sheet(doc, "col-sd2").archetype else { panic!("expected Column") };
    assert_eq!(col_sd2.subcolumn_mode, SubcolumnMode::MeanSemN);
    assert_eq!(col_sd2.data_sets[0].cell(0, 2).unwrap().value, CellValue::Empty);
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "col-sd2"));
}

#[test]
fn pzfx_hardens_xformat_locale_numbers_placeholders_hugetables_and_unsupported_sections() {
    let xml = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<PrismFile SchemaVersion=\"14.2\">\n",
        "  <TableSequence>\n",
        // Text XFormat + EU decimal commas + Unicode minus + placeholder text
        "    <Table ID=\"xy-text\" TableType=\"XY\" XFormat=\"labels\">\n",
        "      <XColumn><Subcolumn><d>Vehicle</d><d>DrugA</d></Subcolumn></XColumn>\n",
        "      <YColumn><Subcolumn><d>1,25</d><d>\u{2212}3,5e-1</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        // Date XFormat with ISO text (kept verbatim without calendar conversion) + CoercedValue warning
        "    <Table ID=\"xy-date\" TableType=\"XY\" XFormat=\"date\">\n",
        "      <XColumn Title=\"Sample Day\"><Subcolumn><d>2026-01-15</d><d>2026-02-01</d></Subcolumn></XColumn>\n",
        "      <YColumn><Subcolumn><d>42</d><d>#N/A</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        // Sequence XFormat with child <X0>/<DeltaX> + invalid Replicates
        "    <Table ID=\"xy-seq\" TableType=\"XY\" XFormat=\"sequence\" Replicates=\"oops\">\n",
        "      <XColumn><X0>1,5</X0><DeltaX>0,25</DeltaX></XColumn>\n",
        "      <YColumn Subcolumns=\"0\"><Subcolumn><d>1.250</d><d>1,234.56</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        // Domain warnings on Contingency and Survival
        "    <Table ID=\"cont-bad\" TableType=\"Contingency\">\n",
        "      <YColumn Title=\"Counts\"><Subcolumn><d>-2</d><d>3.5</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        "    <Table ID=\"surv-bad\" TableType=\"Survival\">\n",
        "      <XColumn><Subcolumn><d>-5</d></Subcolumn></XColumn>\n",
        "      <YColumn Title=\"ArmA\"><Subcolumn><d>2</d></Subcolumn></YColumn>\n",
        "    </Table>\n",
        "  </TableSequence>\n",
        "  <HugeTableSequence>\n",
        "    <HugeTable ID=\"huge-1\"><Title>Large MultiVar</Title><YColumn><Subcolumn><d>1</d><d>x</d></Subcolumn></YColumn></HugeTable>\n",
        "  </HugeTableSequence>\n",
        "  <AnalysisSequence><Analysis/></AnalysisSequence>\n",
        "  <UserEquations><Equation Name=\"Hill\"/></UserEquations>\n",
        "  <Graph ID=\"g1\"/>\n",
        "  <Script ID=\"s1\"/>\n",
        "</PrismFile>",
    );

    let report = import_pzfx_str(xml).unwrap();
    let doc = &report.document;

    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "SchemaVersion"));

    // xy-text: text X labels copied to row_titles + decimal commas & Unicode minus parsed cleanly
    let TableArchetype::Xy(ref xy_text) = sheet(doc, "xy-text").archetype else { panic!("expected XY") };
    assert_eq!(xy_text.row_titles, vec!["Vehicle", "DrugA"]);
    assert!((xy_text.data_sets[0].cell(0, 0).unwrap().active_number().unwrap() - 1.25).abs() < 1e-12);
    assert!((xy_text.data_sets[0].cell(1, 0).unwrap().active_number().unwrap() + 0.35).abs() < 1e-12);
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "xy-text"));

    // xy-date: string dates kept verbatim as text + #N/A kept verbatim with warning
    let TableArchetype::Xy(ref xy_date) = sheet(doc, "xy-date").archetype else { panic!("expected XY") };
    assert_eq!(xy_date.x_mode, XColumnMode::DateOrElapsedTime { unit_label: String::from("Sample Day") });
    assert_eq!(xy_date.materialized_x_cell_at(0).value, CellValue::Text(String::from("2026-01-15")));
    assert_eq!(xy_date.data_sets[0].cell(1, 0).unwrap().value, CellValue::Text(String::from("#N/A")));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "xy-date"));

    // xy-seq: child <X0>/<DeltaX> comma numbers + US thousands + invalid Replicates/Subcolumns warnings
    let TableArchetype::Xy(ref xy_seq) = sheet(doc, "xy-seq").archetype else { panic!("expected XY") };
    assert_eq!(xy_seq.x_mode, XColumnMode::GeneratedSequence { start: 1.5, step: 0.25, progression: curvebook_doc::SequenceProgression::Arithmetic });
    assert!((xy_seq.data_sets[0].cell(1, 0).unwrap().active_number().unwrap() - 1234.56).abs() < 1e-9);
    assert!(report.warnings.iter().any(|w| w.source == "xy-seq" && w.message.contains("Replicates")));
    assert!(report.warnings.iter().any(|w| w.source == "xy-seq" && w.message.contains("Subcolumns=\"0\"")));

    // Domain checks & HugeTable
    assert!(report.warnings.iter().any(|w| w.source == "cont-bad" && w.kind == ImportWarningKind::CoercedValue));
    assert!(report.warnings.iter().any(|w| w.source == "surv-bad" && w.message.contains("time column")));
    assert!(report.warnings.iter().any(|w| w.source == "surv-bad" && w.message.contains("status codes")));
    let huge = sheet(doc, "huge-1");
    assert_eq!(huge.archetype.kind(), TableKind::MultipleVariables);
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedTableType && w.source == "huge-1"));

    // Unsupported section warning mappings
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection && w.source == "AnalysisSequence"));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedEquation && w.source == "UserEquations"));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedGraphOrLayout && w.source == "Graph"));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedScript && w.source == "Script"));
}

#[test]
fn prism_hardens_nested_roots_case_variants_external_doc_json_sqlite_and_metadata_clamping() {
    let long_title = "T".repeat(MAX_TITLE_BYTES + 40);
    let doc_json = format!(
        r#"{{
  "projectTitle": "{}",
  "tables": {{
    "dose_resp": {{
      "name": "Dose Curve",
      "tableType": "XY",
      "yFormat": "SDN",
      "csvPath": "./Data/Dose.CSV"
    }},
    "missing_ref": {{
      "title": "Ghost Sheet",
      "kind": "Column",
      "csv_path": "Data/DoesNotExist.CSV"
    }}
  }},
  "info": [
    {{
      "title": "Reagent Lot",
      "constants": [
        {{ "name": "{}", "value": 100, "units": "nM" }}
      ],
      "notes_markdown": "Freshly prepared"
    }}
  ],
  "equations": ["Y = Bottom + (Top - Bottom)/(1 + 10^((LogEC50 - X)*HillSlope))"]
}}"#,
        long_title,
        "K".repeat(MAX_TITLE_BYTES + 25),
    );
    let csv_payload = "\u{FEFF}\"Row Title\n(Well)\",X,Y_mean,Y_sd,Y_n\n\"A1\",0.1,10,1,3\n\"B1\",1.0,50,2,3\n";
    let dir_meta_json = br#"{"title":"Grouped Sidecar","kind":"Grouped","subcolumn_mode":"plus-minus","replicates":0,"csv_path":"sub.csv"}"#;
    let dir_csv = b"Row Factor,G1-,G1+,G2-,G2+\nT0,5,5,8,8\n";
    let sqlite_blob = b"SQLite format 3\0mock_sqlite_payload";
    let zip = build_zip(&[
        ("ProjectRoot/Document.JSON", doc_json.as_bytes()),
        ("ProjectRoot/Data/Dose.CSV", csv_payload.as_bytes()),
        ("ProjectRoot/Tables/grp/meta.json", dir_meta_json),
        ("ProjectRoot/Tables/grp/sub.csv", dir_csv),
        ("ProjectRoot/Analyses/fit1.json", br#"{"title":"Nonlinear Fit","model":"4PL"}"#),
        ("ProjectRoot/Graphs/curve_plot.json", br#"{"title":"Main Figure","axes":[]}"#),
        ("ProjectRoot/Scripts/export.json", br#"{"title":"Batch Export","code":"print(1)"}"#),
        ("ProjectRoot/db/store.sqlite", sqlite_blob),
        ("ProjectRoot/misc/extra.json", br#"{"title":"Unknown Plugin Config","foo":true}"#),
    ]);

    let report = import_prism_bytes(&zip).unwrap();
    let doc = &report.document;

    assert_eq!(doc.title.len(), MAX_TITLE_BYTES);
    assert_eq!(doc.info_sheets.len(), 1);
    assert_eq!(doc.info_sheets[0].constants[0].name.len(), MAX_TITLE_BYTES);
    assert_eq!(doc.data_sheets.len(), 2);

    let dose = sheet(doc, "dose_resp");
    assert_eq!(dose.title, "Dose Curve");
    let TableArchetype::Xy(ref xy) = dose.archetype else { panic!("expected XY") };
    assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::MeanSdN);
    assert_eq!(xy.row_titles, vec!["A1", "B1"]);

    // TruncatedTitle warnings emitted for oversized project title & info constant name
    assert!(report.warnings.iter().filter(|w| w.kind == ImportWarningKind::TruncatedTitle).count() >= 2);
    // Referenced missing CSV triggers MissingMetadata
    assert!(
        report.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata && w.source == "Data/DoesNotExist.CSV"),
        "warnings: {:?}",
        report.warnings
    );
    // Directory meta.json coerced replicates=0 and unsupported subcolumn mode PlusMinus -> RawReplicates on Grouped
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue));
    // Analyses, graphs, equations, scripts, SQLite, and unrecognized JSON NEVER silently swallowed!
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedEquation));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedAnalysis && w.source.contains("fit1.json")));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedGraphOrLayout && w.source.contains("curve_plot.json")));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedScript && w.source.contains("export.json")));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk && w.message.contains("SQLite")));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk && w.source.contains("extra.json")));
}
