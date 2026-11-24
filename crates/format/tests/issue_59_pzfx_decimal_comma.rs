#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Regression tests for GitHub issue #59: `.pzfx` import decimal-comma numbers,
//! `Excluded="1"` preservation, thousands-separator disambiguation, and import warnings
//! when malformed or conflicting numeric-looking cells fall back to text.

use curvebook_doc::{Cell, CellValue, ConstantValue, SubcolumnMode, TableArchetype};
use curvebook_format::{CsvOptions, ImportWarningKind, export_datasheet_to_csv, import_pzfx_str};

#[test]
fn issue_59_pzfx_decimal_comma_xy_and_info_parse_as_numbers() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile SchemaVersion="2">
  <InfoSequence>
    <Info ID="info-1">
      <Title>Assay Metadata</Title>
      <Constant><Name>HotNM</Name><Value>2,5</Value><Units>nM</Units></Constant>
      <Constant><Name>Count</Name><Value>1,000,000</Value></Constant>
    </Info>
  </InfoSequence>
  <TableSequence>
    <Table ID="Table0" XFormat="numbers" YFormat="replicates" Replicates="1" TableType="XY">
      <Title>comma decimals</Title>
      <XColumn Width="60" Subcolumns="1" Decimals="1">
        <Title></Title>
        <Subcolumn>
          <d>-9</d><d>-8,5</d><d>-8</d><d>-7,5</d><d>-7</d><d>-6,5</d><d>-6</d>
        </Subcolumn>
      </XColumn>
      <YColumn Width="60" Decimals="3" Subcolumns="1">
        <Title>Response</Title>
        <Subcolumn>
          <d>0,10</d><d>0,15</d><d>0,30</d><d>0,55</d><d>0,80</d><d>0,92</d><d>0,97</d>
        </Subcolumn>
      </YColumn>
    </Table>
    <Table ID="Seq0" XFormat="sequence" X0="-9,5" DeltaX="0,5" TableType="XY">
      <Title>sequence comma attrs</Title>
      <YColumn Decimals="2" Subcolumns="1">
        <Title>Signal</Title>
        <Subcolumn><d>1,25e-2</d><d>2,50e-2</d></Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;

    let report = import_pzfx_str(xml).expect("import decimal-comma pzfx");
    assert!(report.warnings.is_empty(), "unambiguous decimal-comma import should emit zero warnings, got {:?}", report.warnings);

    let info = &report.document.info_sheets[0];
    assert_eq!(info.get_constant("HotNM").unwrap().value, ConstantValue::Number(2.5));
    assert_eq!(info.get_constant("Count").unwrap().value, ConstantValue::Number(1_000_000.0));

    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else {
        panic!("expected XY table");
    };
    let expected_x = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0];
    let expected_y = [0.10, 0.15, 0.30, 0.55, 0.80, 0.92, 0.97];
    for (r, (&ex, &ey)) in expected_x.iter().zip(expected_y.iter()).enumerate() {
        assert_eq!(xy.x_value_at(r), Some(ex), "row {r} X mismatch");
        assert_eq!(xy.data_sets[0].cell(r, 0).unwrap().active_number(), Some(ey), "row {r} Y mismatch");
    }

    let exported_csv = export_datasheet_to_csv(&report.document.data_sheets[0], &CsvOptions::csv()).expect("export CSV");
    assert_eq!(exported_csv, "X,Response\n-9,0.1\n-8.5,0.15\n-8,0.3\n-7.5,0.55\n-7,0.8\n-6.5,0.92\n-6,0.97");

    let TableArchetype::Xy(ref seq_xy) = report.document.data_sheets[1].archetype else {
        panic!("expected sequence XY table");
    };
    assert_eq!(seq_xy.x_value_at(0), Some(-9.5));
    assert_eq!(seq_xy.x_value_at(1), Some(-9.0));
    assert_eq!(seq_xy.data_sets[0].cell(0, 0).unwrap().active_number(), Some(0.0125));
    assert_eq!(seq_xy.data_sets[0].cell(1, 0).unwrap().active_number(), Some(0.025));
}

#[test]
fn issue_59_pzfx_partial_comma_x_with_dot_y_retains_all_rows() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="Table0" XFormat="numbers" YFormat="replicates" Replicates="1" TableType="XY">
      <Title>partial comma decimals</Title>
      <XColumn Width="60" Subcolumns="1" Decimals="1">
        <Subcolumn>
          <d>-9</d><d>-8,5</d><d>-8</d><d>-7,5</d><d>-7</d><d>-6,5</d><d>-6</d>
        </Subcolumn>
      </XColumn>
      <YColumn Width="60" Decimals="3" Subcolumns="1">
        <Title>Response</Title>
        <Subcolumn>
          <d>0.10</d><d>0.15</d><d>0.30</d><d>0.55</d><d>0.80</d><d>0.92</d><d>0.97</d>
        </Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;

    let report = import_pzfx_str(xml).expect("import partial-comma pzfx");
    assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else {
        panic!("expected XY table");
    };
    for (r, &ex) in [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0].iter().enumerate() {
        assert_eq!(xy.x_value_at(r), Some(ex), "half-log row {r} X must not be dropped as text");
    }
}

#[test]
fn issue_59_pzfx_excluded_attribute_preserved_on_decimal_comma_and_thousands_cells() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="excl-tbl" TableType="XY" YFormat="replicates" Replicates="2">
      <Title>Exclusions</Title>
      <XColumn Decimals="1">
        <Subcolumn>
          <d Excluded="1">-8,5</d>
          <d>-7,5</d>
          <d>-6,5</d>
        </Subcolumn>
      </XColumn>
      <YColumn Decimals="2">
        <Title>Resp</Title>
        <Subcolumn>
          <d>0,15</d>
          <d Excluded="true">99,95</d>
          <d>0,30*</d>
        </Subcolumn>
        <Subcolumn>
          <d Excluded="1">1,234.56</d>
          <d Excluded="0">0,55</d>
          <d Excluded="1"/>
        </Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;

    let report = import_pzfx_str(xml).expect("import excluded comma cells");
    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else {
        panic!("expected XY table");
    };
    assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(2));

    assert_eq!(xy.x_cells[0][0], Cell::excluded_number(-8.5));
    assert_eq!(xy.x_value_at(0), None);
    assert_eq!(xy.x_value_at(1), Some(-7.5));

    assert_eq!(xy.data_sets[0].cell(0, 0), Some(&Cell::number(0.15)));
    assert_eq!(xy.data_sets[0].cell(1, 0), Some(&Cell::excluded_number(99.95)));
    assert_eq!(xy.data_sets[0].cell(1, 0).unwrap().active_number(), None);
    // Strict XML exclusions: inline `*` is not stripped in `.pzfx` `<d>` elements.
    assert_eq!(xy.data_sets[0].cell(2, 0), Some(&Cell::text("0,30*")));

    assert_eq!(xy.data_sets[0].cell(0, 1), Some(&Cell::excluded_number(1234.56)));
    assert_eq!(xy.data_sets[0].cell(1, 1), Some(&Cell::number(0.55)));
    assert_eq!(xy.data_sets[0].cell(2, 1), Some(&Cell { value: CellValue::Empty, excluded: true }));
}

#[test]
fn issue_59_pzfx_distinguishes_thousands_separators_from_decimal_commas() {
    // 1. Comma-decimal table with `1,000` alongside `0,100`..`0,920` -> `1,000` is `1.0`, not `1000.0`.
    let eu_curve_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="eu-norm" TableType="Column">
      <Title>Normalized EU</Title>
      <YColumn Decimals="3">
        <Title>NormResp</Title>
        <Subcolumn>
          <d>0,100</d>
          <d>0,550</d>
          <d>1,000</d>
          <d>1.234,560</d>
          <d>1.000.000</d>
        </Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;
    let eu_rep = import_pzfx_str(eu_curve_xml).expect("import EU curve");
    assert!(eu_rep.warnings.is_empty(), "expected unambiguous EU table without warnings, got {:?}", eu_rep.warnings);
    let TableArchetype::Column(ref eu_col) = eu_rep.document.data_sheets[0].archetype else {
        panic!("expected Column");
    };
    let eu_vals: Vec<Option<f64>> = (0..5).map(|r| eu_col.data_sets[0].cell(r, 0).unwrap().active_number()).collect();
    assert_eq!(eu_vals, vec![Some(0.1), Some(0.55), Some(1.0), Some(1234.56), Some(1_000_000.0)]);

    // 2. US thousands-separated table (`1,234.56`, `1,000,000`, `1,000` with dot-decimal context or `Decimals="0"`).
    let us_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="us-dot" TableType="Column">
      <Title>US Thousands</Title>
      <YColumn>
        <Title>WithDotContext</Title>
        <Subcolumn>
          <d>500.5</d>
          <d>1,000</d>
          <d>1,234.56</d>
          <d>1,000,000</d>
          <d>-12,500</d>
        </Subcolumn>
      </YColumn>
      <YColumn Decimals="0">
        <Title>ExplicitZeroDecimals</Title>
        <Subcolumn>
          <d>500</d>
          <d>1,000</d>
          <d>2,500</d>
        </Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;
    let us_rep = import_pzfx_str(us_xml).expect("import US thousands");
    assert!(us_rep.warnings.is_empty(), "expected unambiguous US thousands without warnings, got {:?}", us_rep.warnings);
    let TableArchetype::Column(ref us_col) = us_rep.document.data_sheets[0].archetype else {
        panic!("expected Column");
    };
    let col0: Vec<Option<f64>> = (0..5).map(|r| us_col.data_sets[0].cell(r, 0).unwrap().active_number()).collect();
    assert_eq!(col0, vec![Some(500.5), Some(1000.0), Some(1234.56), Some(1_000_000.0), Some(-12_500.0)]);
    let col1: Vec<Option<f64>> = (0..3).map(|r| us_col.data_sets[1].cell(r, 0).unwrap().active_number()).collect();
    assert_eq!(col1, vec![Some(500.0), Some(1000.0), Some(2500.0)]);

    // 3. `Decimals="3"` column with only `[1-9]\d{0,2},\d{3}` cells -> resolved as 3-decimal comma numbers (`1.125`, `2.5`) with CoercedValue note.
    let dec3_only_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="od-3dec" TableType="Column">
      <Title>Absorbance</Title>
      <YColumn Decimals="3">
        <Title>OD450</Title>
        <Subcolumn><d>1,125</d><d>2,500</d></Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;
    let dec3_rep = import_pzfx_str(dec3_only_xml).expect("import Decimals=3 comma column");
    let TableArchetype::Column(ref dec3_col) = dec3_rep.document.data_sheets[0].archetype else {
        panic!("expected Column");
    };
    assert_eq!(dec3_col.data_sets[0].cell(0, 0).unwrap().active_number(), Some(1.125));
    assert_eq!(dec3_col.data_sets[0].cell(1, 0).unwrap().active_number(), Some(2.5));
    assert!(dec3_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "od-3dec"));

    // 4. Zero-context `1,000` / `2,500` (no `Decimals` attr, no `.` or unambiguous `,` decimals anywhere) -> thousands `1000.0` / `2500.0` + CoercedValue warning.
    let zero_ctx_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="counts-0" TableType="Column">
      <Title>Raw Counts</Title>
      <YColumn>
        <Title>Count</Title>
        <Subcolumn><d>500</d><d>1,000</d><d>2,500</d></Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;
    let zc_rep = import_pzfx_str(zero_ctx_xml).expect("import zero-context thousands");
    let TableArchetype::Column(ref zc_col) = zc_rep.document.data_sheets[0].archetype else {
        panic!("expected Column");
    };
    assert_eq!(zc_col.data_sets[0].cell(0, 0).unwrap().active_number(), Some(500.0));
    assert_eq!(zc_col.data_sets[0].cell(1, 0).unwrap().active_number(), Some(1000.0));
    assert_eq!(zc_col.data_sets[0].cell(2, 0).unwrap().active_number(), Some(2500.0));
    assert!(
        zc_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "counts-0" && w.message.contains("2")),
        "expected warning naming sheet and ambiguous cell count, got {:?}",
        zc_rep.warnings
    );
}

#[test]
fn issue_59_pzfx_warns_when_numeric_looking_cells_kept_as_text() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="bad-num" TableType="XY">
      <Title>Malformed Numerics</Title>
      <XColumn>
        <Subcolumn><d>1</d><d>2</d><d>3</d></Subcolumn>
      </XColumn>
      <YColumn>
        <Title>Resp</Title>
        <Subcolumn>
          <d>0.5</d>
          <d>0,5</d>
          <d>1,000</d>
        </Subcolumn>
      </YColumn>
      <YColumn>
        <Title>Malformed</Title>
        <Subcolumn>
          <d>12,34.5</d>
          <d>1,2,3</d>
          <d>1,5e999</d>
        </Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;

    let report = import_pzfx_str(xml).expect("import malformed numeric cells");
    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else {
        panic!("expected XY");
    };
    // Conflicting `.` and `,` in column 0 leaves `1,000` unresolvable -> kept as Text.
    assert_eq!(xy.data_sets[0].cell(0, 0), Some(&Cell::number(0.5)));
    assert_eq!(xy.data_sets[0].cell(1, 0), Some(&Cell::number(0.5)));
    assert_eq!(xy.data_sets[0].cell(2, 0), Some(&Cell::text("1,000")));
    // Malformed numeric-looking strings in column 1 -> kept as Text.
    assert_eq!(xy.data_sets[1].cell(0, 0), Some(&Cell::text("12,34.5")));
    assert_eq!(xy.data_sets[1].cell(1, 0), Some(&Cell::text("1,2,3")));
    assert_eq!(xy.data_sets[1].cell(2, 0), Some(&Cell::text("1,5e999")));

    // Import warning must name the sheet (`bad-num` / `Malformed Numerics`) and report 4 numeric-looking cells kept as text.
    assert!(
        report.warnings.iter().any(|w| {
            w.kind == ImportWarningKind::CoercedValue
                && w.source == "bad-num"
                && w.message.contains("Malformed Numerics")
                && w.message.contains("4")
                && w.message.contains("text")
        }),
        "expected warning naming sheet and 4 numeric-looking cells kept as text, got {:?}",
        report.warnings
    );
}
