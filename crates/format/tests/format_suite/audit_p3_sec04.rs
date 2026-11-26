#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::{self, OpenOptions};

use crate::common::{
    TempDirGuard, apply_clip, build_archive_zip, build_zip_entries, callout_item, col_sheet, make_array_bomb_json, make_nested_empty_array_json,
    make_result_sheet, open_dir_journal, param_rule, pzfx_doc, result_sheet_with_kind, rnum, xy_sheet,
};
use curvebook_doc::{
    AnalysisKind, CURRENT_SCHEMA_VERSION, Cell, CellCoord, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, DocError,
    ErrorCode, FloatingNote, GraphAnnotation, GraphSheet, HookSource, InfoConstant, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec,
    LinearRegressionSpec, MAX_TITLE_BYTES, MultipleVariablesTable, NestedGroupColumn, NestedTable, NonlinearFitSpec, NormalizeAnchor,
    OneWayAnovaSpec, ParameterScope, PartsOfWholeTable, PlacementMode, ProjectDocument, RectPt, ResultStatus, ResultTab, ResultTabKind,
    SequenceProgression, SheetId, SourceCellId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_format::recovery::JOURNAL_FILE_NAME;
use curvebook_format::{
    CsvOptions, ExportOptions, ExportSource, FormatError, FormatRegistry, ensure_result_sheet_csv_exportable, export_datasheet_to_csv,
    export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv, import_csv_to_datasheet, import_prism_bytes, import_pzfx_str,
    open_project_bytes, parse_delimited_records, save_project_bytes, try_export_result_sheet_tidy_csv, try_export_result_tab_csv,
    validate_project_document,
};
use serde_json::{Value, json};

#[test]
fn fa_sec04_04_result_csv_reexports_and_registry_refuse_failed_status() {
    let mut doc = ProjectDocument::new("Failed Export Check").unwrap();
    doc.add_data_sheet(col_sheet("d1", 2, 3)).unwrap();
    let failed = make_result_sheet(
        "r_fail",
        "d1",
        ResultStatus::Failed { code: ErrorCode::InsufficientData, message: String::from("at least 2 valid groups are required") },
    );
    doc.add_result_sheet(failed.clone()).unwrap();
    doc.active_sheet_id = Some(SheetId::from("r_fail"));

    assert!(matches!(ensure_result_sheet_csv_exportable(&failed), Err(FormatError::InvalidPipeline { .. })));
    let err_tab = try_export_result_tab_csv(&failed, &failed.tabs[0], &CsvOptions::csv()).unwrap_err();
    let err_tidy = try_export_result_sheet_tidy_csv(&failed, &CsvOptions::csv()).unwrap_err();
    assert!(matches!(err_tab, FormatError::InvalidPipeline { .. }) && err_tab.to_string().contains("r_fail"));
    assert!(matches!(err_tidy, FormatError::InvalidPipeline { .. }));
    let msg = err_tidy.to_string();
    assert!(msg.contains("r_fail") && msg.contains("insufficient_data") && msg.contains("at least 2 valid groups are required"), "{msg}");

    let mut rs_only_doc = ProjectDocument::new("Failed Only").unwrap();
    rs_only_doc.result_sheets.push(failed.clone());
    let reg = FormatRegistry::default_registry();
    let explicit_opts = ExportOptions { sheet_id: Some(String::from("r_fail")), ..ExportOptions::default() };
    for fmt in ["csv", "tsv"] {
        let exp = reg.exporter_by_format(fmt).unwrap();
        for (src, opts) in [
            (ExportSource::ResultSheet(&failed), ExportOptions::default()),
            (ExportSource::Document(&doc), explicit_opts.clone()),
            (ExportSource::Document(&rs_only_doc), ExportOptions::default()),
        ] {
            assert!(matches!(exp.export_bytes(src, &opts), Err(FormatError::InvalidPipeline { .. })), "{fmt}");
        }
    }

    for (idx, status) in [ResultStatus::UpToDate, ResultStatus::Dirty].into_iter().enumerate() {
        let ok_rs = make_result_sheet(&format!("r_ok_{idx}"), "d1", status);
        ensure_result_sheet_csv_exportable(&ok_rs).unwrap();
        let tab_csv = try_export_result_tab_csv(&ok_rs, &ok_rs.tabs[0], &CsvOptions::csv()).unwrap();
        let tidy_csv = try_export_result_sheet_tidy_csv(&ok_rs, &CsvOptions::csv()).unwrap();
        assert!(tab_csv.contains("Alpha,3.25"));
        assert!(["summary", "Summary", "Mean", "Alpha", "3.25"].iter().all(|tok| tidy_csv.contains(tok)));
        assert_eq!(export_result_tab_csv(&ok_rs.tabs[0], &CsvOptions::csv()), tab_csv);
        assert_eq!(export_result_sheet_tidy_csv(&ok_rs, &CsvOptions::csv()), tidy_csv);

        let mut ok_doc = ProjectDocument::new("Valid Export Check").unwrap();
        ok_doc.add_data_sheet(col_sheet("d1", 2, 3)).unwrap();
        ok_doc.add_result_sheet(ok_rs.clone()).unwrap();
        for fmt in ["csv", "tsv"] {
            let exp = reg.exporter_by_format(fmt).unwrap();
            assert!(exp.export_bytes(ExportSource::ResultSheet(&ok_rs), &ExportOptions::default()).is_ok());
            assert!(exp.export_bytes(ExportSource::Document(&ok_doc), &ExportOptions::default()).is_ok());
        }
    }
}

#[test]
fn fa_sec04_05_large_v0_v6_and_container_projects_load_while_bombs_fail() {
    let mut big_doc = ProjectDocument::new("Large Legacy Study").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 140_000).unwrap();
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[139_999][0] = Cell::number(140_000.0);
    xy.data_sets[0].cells[0][0] = Cell::number(2.5);
    xy.data_sets[0].cells[139_999][0] = Cell::number(99.5);
    big_doc.add_data_sheet(DataSheet::new("d1", "Big XY", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut rs = result_sheet_with_kind("r1", "d1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()));
    rs.tabs.push(ResultTab {
        id: String::from("big_tab"),
        title: String::from("Big Tab"),
        kind: ResultTabKind::Residuals,
        column_headers: vec![String::from("A"), String::from("B"), String::from("C")],
        row_labels: (0..120_000).map(|i| format!("r{i}")).collect(),
        rows: vec![vec![rnum(1.0), rnum(2.0), rnum(3.0)]; 120_000],
        footnotes: Vec::new(),
    });
    big_doc.add_result_sheet(rs).unwrap();

    for legacy_ver in [0_u32, 6_u32] {
        let mut doc_json = serde_json::to_value(&big_doc).unwrap();
        doc_json["schema_version"] = json!(legacy_ver);
        let opened =
            open_project_bytes(&build_archive_zip(&serde_json::to_vec(&doc_json).unwrap(), legacy_ver, "Project", "Large Legacy Study")).unwrap();
        assert_eq!(
            (opened.schema_version, opened.data_sheets[0].archetype.row_count(), opened.result_sheets[0].tabs[0].rows.len()),
            (CURRENT_SCHEMA_VERSION, 140_000, 120_000)
        );
    }

    let mut prism_json = serde_json::to_value(&big_doc).unwrap();
    prism_json["schema_version"] = json!(6);
    let rep = import_prism_bytes(&build_zip_entries(&[("document.json", &serde_json::to_vec(&prism_json).unwrap())])).unwrap().document;
    assert_eq!(
        (rep.schema_version, rep.data_sheets[0].archetype.row_count(), rep.result_sheets[0].tabs[0].rows.len()),
        (CURRENT_SCHEMA_VERSION, 140_000, 120_000)
    );

    let top_bomb = make_nested_empty_array_json(
        r#"{"schema_version":0,"title":"Bomb","bomb":["#,
        1_005_000,
        r#"],"data_sheets":[],"info_sheets":[],"result_sheets":[],"graph_sheets":[],"layout_sheets":[]}"#,
    );
    assert!(matches!(open_project_bytes(&build_archive_zip(&top_bomb, 0, "Project", "Bomb")), Err(FormatError::Json(_))));
    assert!(matches!(import_prism_bytes(&build_zip_entries(&[("document.json", &make_array_bomb_json(1_005_000))])), Err(FormatError::Json(_))));

    let seq_bomb = make_nested_empty_array_json(
        r#"{"schema_version":0,"title":"SeqBomb","data_sheets":[{"id":"d1","title":"D1","archetype":{"Xy":{"x_title":"X","x_mode":"Numeric","y_subcolumn_mode":"Single","x_cells":["#,
        1_000_005,
        r#"],"row_titles":[],"data_sets":[]}}}],"info_sheets":[],"result_sheets":[],"graph_sheets":[],"layout_sheets":[]}"#,
    );
    assert!(open_project_bytes(&build_archive_zip(&seq_bomb, 0, "Project", "SeqBomb")).is_err());
}

#[test]
fn fa_sec04_06_generated_sequence_x_cells_rectangular_and_budgeted() {
    let x_mode = XColumnMode::GeneratedSequence { start: 0.0, step: 1.0, progression: SequenceProgression::Arithmetic };
    let mut sheet =
        DataSheet::new("d_gen", "GenSeq", TableArchetype::Xy(XyTable::new_with_x_mode(x_mode.clone(), SubcolumnMode::Single, 1, 2).unwrap()))
            .unwrap();
    sheet.set_x_or_time_excluded(0, 0, true).unwrap();

    apply_clip(&mut sheet, "10\n20\n", PlacementMode::AppendRows, CellCoord::new(0, 0, 0), false).unwrap();
    sheet.validate().unwrap();
    let TableArchetype::Xy(ref t) = sheet.archetype else { panic!("expected XyTable") };
    assert!(t.x_cells.len() == 4 && t.x_cells.iter().all(|r| r.len() == 1) && t.x_cells[0][0].excluded && !t.x_cells[2][0].excluded);

    apply_clip(&mut sheet, "5\n", PlacementMode::InsertRows, CellCoord::new(1, 0, 0), false).unwrap();
    sheet.validate().unwrap();
    let TableArchetype::Xy(ref t2) = sheet.archetype else { panic!("expected XyTable") };
    assert!(t2.x_cells.len() == 5 && t2.x_cells.iter().all(|r| r.len() == 1));

    let mut doc = ProjectDocument::new("GenSeq Save").unwrap();
    doc.add_data_sheet(sheet).unwrap();
    open_project_bytes(&save_project_bytes(&doc).unwrap()).unwrap().data_sheets[0].validate().unwrap();

    // 9,990 * 1,001 <= 10M; growing by 2 rows -> 9,992 * 1,001 = 10,001,992 > MAX_IMPORT_CELLS.
    let mut big =
        DataSheet::new("d_big", "BigGen", TableArchetype::Xy(XyTable::new_with_x_mode(x_mode, SubcolumnMode::RawReplicates(50), 20, 9_990).unwrap()))
            .unwrap();
    big.set_x_or_time_excluded(0, 0, true).unwrap();
    let before = big.clone();
    assert!(matches!(
        apply_clip(&mut big, "1\n2\n", PlacementMode::AppendRows, CellCoord::new(0, 0, 0), false),
        Err(FormatError::InvalidPipeline { .. })
    ));
    assert_eq!(big, before);
}

#[test]
fn fa_sec04_07_nested_biological_units_and_subcolumn_bounds() {
    let nested = NestedTable {
        groups: vec![NestedGroupColumn::new("Control", 2, 2).unwrap(), NestedGroupColumn::new("Treated", 3, 2).unwrap()],
        row_titles: vec![String::new(); 2],
        row_titles_expanded: false,
    };
    let mut sheet = DataSheet::new("n1", "Nested", TableArchetype::Nested(nested)).unwrap();

    let range = apply_clip(&mut sheet, "1,2,3,4,5,6\n", PlacementMode::Overwrite, CellCoord::new(0, 0, 0), false).unwrap();
    assert_eq!(range.end, CellCoord::new(0, 2, 0));
    let TableArchetype::Nested(ref t) = sheet.archetype else { panic!("expected Nested") };
    assert_eq!((t.groups.len(), t.groups[2].biological_unit_count()), (3, 1));
    assert_eq!(t.groups[0].cells[0], vec![Cell::number(1.0), Cell::number(2.0)]);
    assert_eq!(t.groups[1].cells[0], vec![Cell::number(3.0), Cell::number(4.0), Cell::number(5.0)]);
    assert_eq!(t.groups[2].cells[0], vec![Cell::number(6.0)]);

    apply_clip(&mut sheet, "10,20\n", PlacementMode::Overwrite, CellCoord::new(1, 0, 1), false).unwrap();
    let TableArchetype::Nested(ref t) = sheet.archetype else { panic!("expected Nested") };
    assert_eq!((&t.groups[0].cells[1][1], &t.groups[1].cells[1][0]), (&Cell::number(10.0), &Cell::number(20.0)));

    assert!(matches!(
        apply_clip(&mut sheet, "99\n", PlacementMode::Overwrite, CellCoord::new(0, 0, 2), false),
        Err(FormatError::Doc(DocError::CellOutOfBounds { col: 0, subcol: 2, .. }))
    ));

    let single_sheets = [
        xy_sheet("xy1", SubcolumnMode::Single, 2, 2),
        col_sheet("col1", 2, 2),
        DataSheet::new("con1", "Con", TableArchetype::Contingency(ContingencyTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("sur1", "Sur", TableArchetype::Survival(SurvivalTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("pow1", "Pow", TableArchetype::PartsOfWhole(PartsOfWholeTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("mv1", "MV", TableArchetype::MultipleVariables(MultipleVariablesTable::new(2, 2).unwrap())).unwrap(),
    ];
    for mut s in single_sheets {
        let before = s.clone();
        assert!(
            matches!(
                apply_clip(&mut s, "99\n", PlacementMode::Overwrite, CellCoord::new(0, 0, 1), false),
                Err(FormatError::Doc(DocError::CellOutOfBounds { subcol: 1, .. }))
            ),
            "{}",
            s.id
        );
        assert_eq!(s, before);
    }
}

#[test]
fn fa_sec04_08_apply_csv_import_link_is_atomic_on_error() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 2).unwrap();
    xy.data_sets[0].title = String::from("OriginalA");
    xy.data_sets[1].title = String::from("OriginalB");
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[1][0] = Cell::number(2.0);
    xy.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy.data_sets[1].cells[1][0] = Cell::number(20.0);
    let mut sheet = DataSheet::new("d_atomic", "Atomic", TableArchetype::Xy(xy)).unwrap();
    let before = sheet.clone();

    let csv_bad_hdr = format!("ValidNewA,{}\n100,200\n", "X".repeat(MAX_TITLE_BYTES + 10));
    assert!(matches!(
        apply_clip(&mut sheet, &csv_bad_hdr, PlacementMode::InsertRows, CellCoord::new(0, 0, 0), true),
        Err(FormatError::Doc(DocError::TitleTooLong { .. }))
    ));
    assert_eq!(sheet, before);
    assert!(apply_clip(&mut sheet, "100\n", PlacementMode::InsertRows, CellCoord::new(0, 0, 1), false).is_err());
    assert_eq!(sheet, before);
}

#[test]
fn fa_sec04_09_excluded_text_cells_round_trip_losslessly_in_valid_csv() {
    let cases = [
        ("ctrl", "ctrl"),
        ("0012", "0012"),
        ("1.5", "1.5"),
        ("-42", "'-42"),
        ("NA", "NA"),
        ("NaN", "NaN"),
        ("NULL", "NULL"),
        (".", "."),
        (" a ", " a "),
        ("", ""),
        ("a,b", "a,b"),
        ("line1\nline2", "line1\nline2"),
        ("say \"hi\"", "say \"hi\""),
        ("a*", "a*"),
        ("*a", "*a"),
        ("*", "*"),
        ("\"", "\""),
        ("=cmd", "'=cmd"),
        ("1,5", "1,5"),
    ];
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, cases.len()).unwrap();
    for (i, t) in ["ExcludedText", "ExcludedNumber", "NormalText"].iter().enumerate() {
        tbl.data_sets[i].title = String::from(*t);
    }
    for (r, &(txt, _)) in cases.iter().enumerate() {
        tbl.data_sets[0].cells[r][0] = Cell::excluded_text(txt);
        tbl.data_sets[1].cells[r][0] = Cell::excluded_number((r as f64) + 1.5);
        tbl.data_sets[2].cells[r][0] = Cell::text(txt);
    }
    let sheet = DataSheet::new("d_ex", "Excluded", TableArchetype::Column(tbl)).unwrap();

    for (label, opts) in [("us_csv", CsvOptions::csv()), ("eu_semicolon", CsvOptions { delimiter: ';', decimal_separator: ',', ..CsvOptions::csv() })]
    {
        let exported = export_datasheet_to_csv(&sheet, &opts).unwrap();
        parse_delimited_records(&exported, &opts).unwrap_or_else(|e| panic!("{label} invalid CSV: {e}\n{exported}"));
        let re = import_csv_to_datasheet("d_re", "Re", &exported, TableKind::Column, SubcolumnMode::Single, &CsvOptions { header_rows: 1, ..opts })
            .unwrap();
        for (r, &(_, expected)) in cases.iter().enumerate() {
            assert_eq!(re.cell(CellCoord::new(r, 0, 0)), Some(&Cell::excluded_text(expected)), "{label} excl text row {r}");
            assert_eq!(re.cell(CellCoord::new(r, 1, 0)), Some(&Cell::excluded_number((r as f64) + 1.5)), "{label} excl num row {r}");
            assert_eq!(re.cell(CellCoord::new(r, 2, 0)), Some(&Cell::text(expected)), "{label} normal text row {r}");
        }
    }
}

#[test]
fn fa_sec04_10_export_result_sheet_json_rejects_non_finite_analysis_floats() {
    let mut nl_spec = NonlinearFitSpec::new("one_site_binding");
    nl_spec.parameter_rules.push(param_rule("Bmax", ParameterScope::Constant(f64::NEG_INFINITY)));
    let bad_kinds = [
        ("r_lin", AnalysisKind::LinearRegression(LinearRegressionSpec { plot_x_min: Some(f64::NAN), ..Default::default() })),
        (
            "r_norm",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::FixedValue(f64::INFINITY),
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: true,
            }),
        ),
        ("r_nl", AnalysisKind::NonlinearRegression(nl_spec)),
    ];

    let reg = FormatRegistry::default_registry();
    let json_exporter = reg.exporter_by_format("json").unwrap();
    for (id, kind) in bad_kinds {
        let mut bad = make_result_sheet(id, "d1", ResultStatus::UpToDate);
        bad.analysis.kind = kind;
        assert!(matches!(export_result_sheet_json(&bad, false), Err(FormatError::NonFiniteFloat { .. })), "direct {id}");
        assert!(
            matches!(json_exporter.export_bytes(ExportSource::ResultSheet(&bad), &ExportOptions::default()), Err(FormatError::NonFiniteFloat { .. })),
            "registry {id}"
        );
    }
    assert!(export_result_sheet_json(&make_result_sheet("r_good", "d1", ResultStatus::UpToDate), false).is_ok());
}

#[test]
fn fa_sec04_11_nested_pzfx_csv_round_trip_preserves_groups_and_subcolumns() {
    let pzfx_xml = pzfx_doc(
        r#"<Table ID="Table0" TableType="Nested" Replicates="2"><Title>Nested Study</Title>
  <RowTitlesColumn><Subcolumn><d>R1</d><d>R2</d></Subcolumn></RowTitlesColumn>
  <YColumn Width="81" Decimals="1" Subcolumns="2"><Title>Control</Title><Subcolumn><Title>Mouse 1</Title><d>10.5</d><d Excluded="true">11.0</d></Subcolumn><Subcolumn><Title>Mouse 2</Title><d>12.5</d><d>13.0</d></Subcolumn></YColumn>
  <YColumn Width="81" Decimals="1" Subcolumns="2"><Title>Treated</Title><Subcolumn><Title>Mouse 3</Title><d>20.5</d><d>21.0</d></Subcolumn><Subcolumn><Title>Mouse 4</Title><d>22.5</d><d>23.0</d></Subcolumn></YColumn>
</Table>"#,
    );
    let orig = &import_pzfx_str(&pzfx_xml).unwrap().document.data_sheets[0];
    let exported = export_datasheet_to_csv(orig, &CsvOptions::csv()).unwrap();
    assert_eq!(exported.lines().next().unwrap(), "Row Title,Control:Mouse 1,Control:Mouse 2,Treated:Mouse 3,Treated:Mouse 4");

    let re = import_csv_to_datasheet(
        "re_nested",
        "Nested Reimported",
        &exported,
        TableKind::Nested,
        SubcolumnMode::RawReplicates(2),
        &CsvOptions { header_rows: 1, has_row_titles: true, ..CsvOptions::csv() },
    )
    .unwrap();
    let (TableArchetype::Nested(a), TableArchetype::Nested(b)) = (&orig.archetype, &re.archetype) else { panic!("expected NestedTable") };
    assert_eq!((a.groups.len(), &b.row_titles), (b.groups.len(), &a.row_titles));
    assert_eq!(
        (
            b.groups[0].title.as_str(),
            b.groups[0].biological_unit_count(),
            &b.groups[0].subcolumn_titles,
            &b.groups[0].cells,
            b.groups[0].cells[1][0].excluded
        ),
        ("Control", 2, &vec![String::from("Control:Mouse 1"), String::from("Control:Mouse 2")], &a.groups[0].cells, true)
    );
    assert_eq!(
        (b.groups[1].title.as_str(), b.groups[1].biological_unit_count(), &b.groups[1].subcolumn_titles, &b.groups[1].cells),
        ("Treated", 2, &vec![String::from("Treated:Mouse 3"), String::from("Treated:Mouse 4")], &a.groups[1].cells)
    );

    let mut g0 = NestedGroupColumn::new("Control", 2, 2).unwrap();
    g0.subcolumn_titles = vec![String::from("M1"), String::from("M2")];
    g0.cells[0] = vec![Cell::number(1.0), Cell::excluded_number(2.0)];
    let mut g1 = NestedGroupColumn::new("Treated", 1, 2).unwrap();
    g1.subcolumn_titles = vec![String::from("M3")];
    g1.cells[0] = vec![Cell::number(3.0)];
    let unequal = DataSheet::new(
        "n_unequal",
        "Unequal",
        TableArchetype::Nested(NestedTable { groups: vec![g0.clone(), g1.clone()], row_titles: vec![String::new(); 2], row_titles_expanded: false }),
    )
    .unwrap();
    let unequal_csv = export_datasheet_to_csv(&unequal, &CsvOptions::csv()).unwrap();
    assert_eq!(unequal_csv.lines().next().unwrap(), "Control:M1,Control:M2,Treated:M3");
    let TableArchetype::Nested(t) =
        import_csv_to_datasheet("n_u2", "Unequal2", &unequal_csv, TableKind::Nested, SubcolumnMode::RawReplicates(1), &CsvOptions::csv())
            .unwrap()
            .archetype
    else {
        panic!("expected Nested")
    };
    assert_eq!(
        (t.groups.len(), t.groups[0].biological_unit_count(), t.groups[1].biological_unit_count(), &t.groups[0].cells[0], &t.groups[1].cells[0]),
        (2, 2, 1, &g0.cells[0], &g1.cells[0])
    );
}

#[test]
fn fa_sec04_12_container_sheet_id_remap_covers_hooks_outliers_notes_and_inline_tokens() {
    let ds = xy_sheet("Data 1", SubcolumnMode::Single, 1, 3);
    let mut info = InfoSheet::new_preseeded("Info 1", "Info 1").unwrap();
    info.linked_data_sheets.push(SheetId::from("Data 1"));
    info.constants.push(InfoConstant::new("Conc", ConstantValue::Number(10.0), None).unwrap());

    let rs_src = result_sheet_with_kind("Result 1", "Data 1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()));
    let info_hook = HookSource::InfoConstant { info_sheet_id: Some(SheetId::from("Info 1")), constant_name: String::from("Conc") };
    let res_hook =
        HookSource::ResultScalar { result_sheet_id: SheetId::from("Result 1"), parameter_key: String::from("EC50"), data_set_index: Some(0) };

    let mut rs_norm = result_sheet_with_kind(
        "res_norm",
        "Data 1",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::Hooked(info_hook.clone()),
            hundred_anchor: NormalizeAnchor::Hooked(res_hook),
            output_percent: true,
            per_data_set: true,
        }),
    );
    rs_norm.eliminated_outliers.push(SourceCellId { sheet_id: SheetId::from("Data 1"), row: 1, col: 0, subcol: 0 });

    let mut nl_spec = NonlinearFitSpec::new("one_site_binding");
    nl_spec.parameter_rules.push(param_rule("Bmax", ParameterScope::HookedConstant(info_hook)));
    let rs_nl = result_sheet_with_kind("res_nl", "Data 1", AnalysisKind::NonlinearRegression(nl_spec));

    let mut gs = GraphSheet::new_uninitialized_for_data("Graph 1", &ds).unwrap();
    gs.spec.title = String::from("Title {{info:Info 1:Conc}} / {{result:Result 1:0:EC50}}");
    gs.spec.x_axis.title = String::from("X {{info:Info 1:Conc}}");
    gs.spec.y_axis.title = String::from("Y {{result:Result 1:EC50}}");
    if let Some(ref mut y2) = gs.spec.y2_axis {
        y2.title = String::from("Y2 {{info:Info 1:Conc}}");
    }
    gs.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 100.0, 40.0),
        markdown: String::from("Box {{info:Info 1:Conc}}"),
        bordered: true,
    });
    gs.floating_notes.push(FloatingNote::new("n1", RectPt::new(5.0, 5.0, 50.0, 20.0), "Note {{result:Result 1:0:EC50}}"));

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(callout_item("tc1", "Layout {{info:Info 1:Conc}} + {{result:Result 1:EC50}}"));
    let ls = LayoutSheet::new("Layout 1", "Layout 1", layout_spec).unwrap();

    let mut doc = ProjectDocument::new("Remap Test").unwrap();
    doc.data_sheets.push(ds);
    doc.info_sheets.push(info);
    doc.result_sheets.extend([rs_src, rs_norm, rs_nl]);
    doc.graph_sheets.push(gs);
    doc.layout_sheets.push(ls);

    let imported = import_prism_bytes(&build_zip_entries(&[("document.json", &serde_json::to_vec(&doc).unwrap())])).unwrap().document;
    imported.validate().unwrap();
    validate_project_document(&imported).unwrap();

    let (new_data_id, new_info_id, new_rs_src_id) = (&imported.data_sheets[0].id, &imported.info_sheets[0].id, &imported.result_sheets[0].id);
    assert!(new_data_id.as_str() != "Data 1" && new_info_id.as_str() != "Info 1" && new_rs_src_id.as_str() != "Result 1");
    assert_eq!(&imported.result_sheets[1].eliminated_outliers[0].sheet_id, new_data_id);

    assert!(matches!(
        &imported.result_sheets[1].analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id, .. }),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar { result_sheet_id, .. }),
            ..
        }) if info_sheet_id.as_ref() == Some(new_info_id) && result_sheet_id == new_rs_src_id
    ));
    assert!(matches!(
        &imported.result_sheets[2].analysis.kind,
        AnalysisKind::NonlinearRegression(spec)
            if matches!(&spec.parameter_rules[0].scope, ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id, .. }) if info_sheet_id.as_ref() == Some(new_info_id))
    ));

    let (info_tok, res_tok0, res_tok) = (
        format!("{{{{info:{new_info_id}:Conc}}}}"),
        format!("{{{{result:{new_rs_src_id}:0:EC50}}}}"),
        format!("{{{{result:{new_rs_src_id}:EC50}}}}"),
    );
    let g = &imported.graph_sheets[0];
    assert!(
        g.spec.title.contains(&info_tok)
            && g.spec.title.contains(&res_tok0)
            && g.spec.x_axis.title.contains(&info_tok)
            && g.spec.y_axis.title.contains(&res_tok)
            && g.floating_notes[0].body_markdown.contains(&res_tok0)
    );
    let LayoutItem::TextCallout { markdown, .. } = &imported.layout_sheets[0].spec.items[0] else { panic!("expected TextCallout") };
    assert!(markdown.contains(&info_tok) && markdown.contains(&res_tok));
}

#[test]
fn fa_sec04_13_container_schema_version_clamps_u64_and_rejects_non_u64() {
    let zip_with_ver = |ver: Option<Value>| {
        let mut obj = json!({ "title": "Schema Peek", "data_sheets": [], "info_sheets": [], "result_sheets": [], "graph_sheets": [], "layout_sheets": [], "active_sheet_id": null, "family_anchor_sheet_id": null });
        if let Some(v) = ver {
            obj["schema_version"] = v;
        }
        build_zip_entries(&[("document.json", &serde_json::to_vec(&obj).unwrap())])
    };
    assert!(matches!(
        import_prism_bytes(&zip_with_ver(Some(json!(5_000_000_000_u64)))),
        Err(FormatError::UnsupportedSchemaVersion { version: u32::MAX, max_supported: CURRENT_SCHEMA_VERSION })
    ));
    for bad in [json!("7"), json!(-1), json!(1.5), json!(true)] {
        assert!(matches!(import_prism_bytes(&zip_with_ver(Some(bad.clone()))), Err(FormatError::Json(_))), "schema_version={bad}");
    }
    for ok_ver in [None, Some(Value::Null)] {
        assert_eq!(import_prism_bytes(&zip_with_ver(ok_ver)).unwrap().document.schema_version, CURRENT_SCHEMA_VERSION);
    }
}

#[test]
fn fa_sec04_14_recovery_journal_rewrite_has_no_duplicate_trailing_newline() {
    let dir = TempDirGuard::new("journal_nl").unwrap();
    {
        let (mut journal, _) = open_dir_journal(dir.path(), "w1", 60_000, 1_000).unwrap();
        journal.record_command(json!({"op": "noop", "v": 1})).unwrap();
    }
    let journal_path = dir.child(JOURNAL_FILE_NAME);
    {
        use std::io::Write as _;
        OpenOptions::new().append(true).open(&journal_path).unwrap().write_all(b"{\"format_version\":1,\"seq\":2,\"torn").unwrap();
    }

    let (mut reopened, report) = open_dir_journal(dir.path(), "w2", 60_000, 65_000).unwrap();
    assert_eq!(report.discarded_corrupt_tail_lines, 1);
    let after = fs::read_to_string(&journal_path).unwrap();
    assert!(!after.contains("\n\n") && after.ends_with('\n') && after.lines().count() == 1, "{after:?}");

    reopened.record_command(json!({"op": "noop", "v": 2})).unwrap();
    drop(reopened);
    let (_, report2) = open_dir_journal(dir.path(), "w3", 60_000, 130_000).unwrap();
    assert!(report2.discarded_corrupt_tail_lines == 0 && report2.warnings.is_empty());
}
