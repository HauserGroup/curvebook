#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression tests for GitHub issue #59: `.pzfx` import decimal-comma numbers,
//! `Excluded="1"` preservation, thousands-separator disambiguation, and import warnings
//! when malformed or conflicting numeric-looking cells fall back to text.

mod common;

use common::{pzfx_doc, pzfx_tables};
use curvebook_doc::{Cell, CellValue, ConstantValue, SubcolumnMode, TableArchetype};
use curvebook_format::{CsvOptions, ImportWarningKind, export_datasheet_to_csv, import_pzfx_str};

#[test]
fn issue_59_pzfx_decimal_comma_xy_and_info_parse_as_numbers() {
    let xml = pzfx_doc(
        r#"<InfoSequence><Info ID="info-1"><Title>Assay Metadata</Title>
    <Constant><Name>HotNM</Name><Value>2,5</Value><Units>nM</Units></Constant>
    <Constant><Name>Count</Name><Value>1,000,000</Value></Constant>
  </Info></InfoSequence>
  <TableSequence>
    <Table ID="Table0" XFormat="numbers" YFormat="replicates" Replicates="1" TableType="XY"><Title>comma decimals</Title>
      <XColumn Width="60" Subcolumns="1" Decimals="1"><Title></Title><Subcolumn><d>-9</d><d>-8,5</d><d>-8</d><d>-7,5</d><d>-7</d><d>-6,5</d><d>-6</d></Subcolumn></XColumn>
      <YColumn Width="60" Decimals="3" Subcolumns="1"><Title>Response</Title><Subcolumn><d>0,10</d><d>0,15</d><d>0,30</d><d>0,55</d><d>0,80</d><d>0,92</d><d>0,97</d></Subcolumn></YColumn>
    </Table>
    <Table ID="Seq0" XFormat="sequence" X0="-9,5" DeltaX="0,5" TableType="XY"><Title>sequence comma attrs</Title>
      <YColumn Decimals="2" Subcolumns="1"><Title>Signal</Title><Subcolumn><d>1,25e-2</d><d>2,50e-2</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>"#,
    );
    let report = import_pzfx_str(&xml).expect("import decimal-comma pzfx");
    assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);

    let info = &report.document.info_sheets[0];
    assert_eq!(info.get_constant("HotNM").unwrap().value, ConstantValue::Number(2.5));
    assert_eq!(info.get_constant("Count").unwrap().value, ConstantValue::Number(1_000_000.0));

    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else { panic!("expected XY") };
    let expected_x = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0];
    let expected_y = [0.10, 0.15, 0.30, 0.55, 0.80, 0.92, 0.97];
    for (r, (&ex, &ey)) in expected_x.iter().zip(expected_y.iter()).enumerate() {
        assert_eq!((xy.x_value_at(r), xy.data_sets[0].cell(r, 0).unwrap().active_number()), (Some(ex), Some(ey)), "row {r}");
    }
    assert_eq!(
        export_datasheet_to_csv(&report.document.data_sheets[0], &CsvOptions::csv()).unwrap(),
        "X,Response\n-9,0.1\n-8.5,0.15\n-8,0.3\n-7.5,0.55\n-7,0.8\n-6.5,0.92\n-6,0.97"
    );

    let TableArchetype::Xy(ref seq_xy) = report.document.data_sheets[1].archetype else { panic!("expected sequence XY") };
    assert_eq!((seq_xy.x_value_at(0), seq_xy.x_value_at(1)), (Some(-9.5), Some(-9.0)));
    assert_eq!(
        (seq_xy.data_sets[0].cell(0, 0).unwrap().active_number(), seq_xy.data_sets[0].cell(1, 0).unwrap().active_number()),
        (Some(0.0125), Some(0.025))
    );
}

#[test]
fn issue_59_pzfx_partial_comma_x_with_dot_y_retains_all_rows() {
    let xml = pzfx_tables(
        r#"<Table ID="Table0" XFormat="numbers" YFormat="replicates" Replicates="1" TableType="XY"><Title>partial comma decimals</Title>
      <XColumn Width="60" Subcolumns="1" Decimals="1"><Subcolumn><d>-9</d><d>-8,5</d><d>-8</d><d>-7,5</d><d>-7</d><d>-6,5</d><d>-6</d></Subcolumn></XColumn>
      <YColumn Width="60" Decimals="3" Subcolumns="1"><Title>Response</Title><Subcolumn><d>0.10</d><d>0.15</d><d>0.30</d><d>0.55</d><d>0.80</d><d>0.92</d><d>0.97</d></Subcolumn></YColumn>
    </Table>"#,
    );
    let report = import_pzfx_str(&xml).expect("import partial-comma pzfx");
    assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else { panic!("expected XY") };
    for (r, &ex) in [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0].iter().enumerate() {
        assert_eq!(xy.x_value_at(r), Some(ex), "half-log row {r}");
    }
}

#[test]
fn issue_59_pzfx_excluded_attribute_preserved_on_decimal_comma_and_thousands_cells() {
    let xml = pzfx_tables(
        r#"<Table ID="excl-tbl" TableType="XY" YFormat="replicates" Replicates="2"><Title>Exclusions</Title>
      <XColumn Decimals="1"><Subcolumn><d Excluded="1">-8,5</d><d>-7,5</d><d>-6,5</d></Subcolumn></XColumn>
      <YColumn Decimals="2"><Title>Resp</Title>
        <Subcolumn><d>0,15</d><d Excluded="true">99,95</d><d>0,30*</d></Subcolumn>
        <Subcolumn><d Excluded="1">1,234.56</d><d Excluded="0">0,55</d><d Excluded="1"/></Subcolumn>
      </YColumn>
    </Table>"#,
    );
    let report = import_pzfx_str(&xml).expect("import excluded comma cells");
    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else { panic!("expected XY") };
    assert_eq!(
        (xy.y_subcolumn_mode, &xy.x_cells[0][0], xy.x_value_at(0), xy.x_value_at(1)),
        (SubcolumnMode::RawReplicates(2), &Cell::excluded_number(-8.5), None, Some(-7.5))
    );
    assert_eq!(xy.data_sets[0].cell(0, 0), Some(&Cell::number(0.15)));
    assert_eq!((xy.data_sets[0].cell(1, 0), xy.data_sets[0].cell(1, 0).unwrap().active_number()), (Some(&Cell::excluded_number(99.95)), None));
    assert_eq!(xy.data_sets[0].cell(2, 0), Some(&Cell::text("0,30*")));
    assert_eq!(xy.data_sets[0].cell(0, 1), Some(&Cell::excluded_number(1234.56)));
    assert_eq!(xy.data_sets[0].cell(1, 1), Some(&Cell::number(0.55)));
    assert_eq!(xy.data_sets[0].cell(2, 1), Some(&Cell { value: CellValue::Empty, excluded: true }));
}

#[test]
fn issue_59_pzfx_distinguishes_thousands_separators_from_decimal_commas() {
    struct Case<'a> {
        label: &'a str,
        table_xml: &'a str,
        expected_cols: &'a [&'a [f64]],
        expect_warning: Option<(&'a str, Option<&'a str>)>,
    }

    let cases = [
        Case {
            label: "eu_norm",
            table_xml: r#"<Table ID="eu-norm" TableType="Column"><Title>Normalized EU</Title><YColumn Decimals="3"><Title>NormResp</Title><Subcolumn><d>0,100</d><d>0,550</d><d>1,000</d><d>1.234,560</d><d>1.000.000</d></Subcolumn></YColumn></Table>"#,
            expected_cols: &[&[0.1, 0.55, 1.0, 1234.56, 1_000_000.0]],
            expect_warning: None,
        },
        Case {
            label: "us_thousands",
            table_xml: r#"<Table ID="us-dot" TableType="Column"><Title>US Thousands</Title>
                <YColumn><Title>WithDotContext</Title><Subcolumn><d>500.5</d><d>1,000</d><d>1,234.56</d><d>1,000,000</d><d>-12,500</d></Subcolumn></YColumn>
                <YColumn Decimals="0"><Title>ExplicitZeroDecimals</Title><Subcolumn><d>500</d><d>1,000</d><d>2,500</d></Subcolumn></YColumn>
            </Table>"#,
            expected_cols: &[&[500.5, 1000.0, 1234.56, 1_000_000.0, -12_500.0], &[500.0, 1000.0, 2500.0]],
            expect_warning: None,
        },
        Case {
            label: "dec3_only",
            table_xml: r#"<Table ID="od-3dec" TableType="Column"><Title>Absorbance</Title><YColumn Decimals="3"><Title>OD450</Title><Subcolumn><d>1,125</d><d>2,500</d></Subcolumn></YColumn></Table>"#,
            expected_cols: &[&[1.125, 2.5]],
            expect_warning: Some(("od-3dec", None)),
        },
        Case {
            label: "zero_ctx_thousands",
            table_xml: r#"<Table ID="counts-0" TableType="Column"><Title>Raw Counts</Title><YColumn><Title>Count</Title><Subcolumn><d>500</d><d>1,000</d><d>2,500</d></Subcolumn></YColumn></Table>"#,
            expected_cols: &[&[500.0, 1000.0, 2500.0]],
            expect_warning: Some(("counts-0", Some("2"))),
        },
    ];

    for c in cases {
        let rep = import_pzfx_str(&pzfx_tables(c.table_xml)).unwrap();
        let TableArchetype::Column(ref col) = rep.document.data_sheets[0].archetype else { panic!("{}: expected Column", c.label) };
        for (ci, expected) in c.expected_cols.iter().enumerate() {
            let actual: Vec<Option<f64>> = (0..expected.len()).map(|r| col.data_sets[ci].cell(r, 0).unwrap().active_number()).collect();
            assert_eq!(actual, expected.iter().copied().map(Some).collect::<Vec<_>>(), "{} col {ci}", c.label);
        }
        match c.expect_warning {
            None => assert!(rep.warnings.is_empty(), "{}: unexpected warnings {:?}", c.label, rep.warnings),
            Some((src, needle)) => assert!(
                rep.warnings
                    .iter()
                    .any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == src && needle.is_none_or(|n| w.message.contains(n))),
                "{}: missing warning in {:?}",
                c.label,
                rep.warnings
            ),
        }
    }
}

#[test]
fn issue_59_pzfx_warns_when_numeric_looking_cells_kept_as_text() {
    let xml = pzfx_tables(
        r#"<Table ID="bad-num" TableType="XY"><Title>Malformed Numerics</Title>
      <XColumn><Subcolumn><d>1</d><d>2</d><d>3</d></Subcolumn></XColumn>
      <YColumn><Title>Resp</Title><Subcolumn><d>0.5</d><d>0,5</d><d>1,000</d></Subcolumn></YColumn>
      <YColumn><Title>Malformed</Title><Subcolumn><d>12,34.5</d><d>1,2,3</d><d>1,5e999</d></Subcolumn></YColumn>
    </Table>"#,
    );
    let report = import_pzfx_str(&xml).expect("import malformed numeric cells");
    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else { panic!("expected XY") };
    assert_eq!(
        [xy.data_sets[0].cell(0, 0), xy.data_sets[0].cell(1, 0), xy.data_sets[0].cell(2, 0)],
        [Some(&Cell::number(0.5)), Some(&Cell::number(0.5)), Some(&Cell::text("1,000"))]
    );
    assert_eq!(
        [xy.data_sets[1].cell(0, 0), xy.data_sets[1].cell(1, 0), xy.data_sets[1].cell(2, 0)],
        [Some(&Cell::text("12,34.5")), Some(&Cell::text("1,2,3")), Some(&Cell::text("1,5e999"))]
    );
    assert!(
        report.warnings.iter().any(|w| {
            w.kind == ImportWarningKind::CoercedValue
                && w.source == "bad-num"
                && w.message.contains("Malformed Numerics")
                && w.message.contains("4")
                && w.message.contains("text")
        }),
        "expected warning naming sheet and 4 text cells, got {:?}",
        report.warnings
    );
}
