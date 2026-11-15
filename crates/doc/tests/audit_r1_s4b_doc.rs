//! Stage 4B regression tests for `R1-S02-13` (`ColumnTable::complete_paired_rows_with_report`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumencraft_doc::{Cell, ColumnDesignHint, ColumnTable};

#[test]
fn r1_s02_13_complete_paired_rows_with_report_counts_omissions_and_trailing_blanks() {
    // 7 allocated rows across 3 columns; last 2 rows are trailing blank padding.
    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 7).unwrap();

    // Row 0: complete active numeric block
    tab.data_sets[0].cells[0][0] = Cell::number(1.0);
    tab.data_sets[1].cells[0][0] = Cell::number(2.0);
    tab.data_sets[2].cells[0][0] = Cell::number(3.0);

    // Row 1: internal blank spacer row (all empty, none excluded) -> NOT counted as skipped
    // (leaves default empty cells)

    // Row 2: partial row (missing col 2) -> skipped_incomplete_rows += 1
    tab.data_sets[0].cells[2][0] = Cell::number(4.0);
    tab.data_sets[1].cells[2][0] = Cell::number(5.0);

    // Row 3: excluded numeric cell in col 1 -> skipped_incomplete_rows += 1, non_numeric_cells unchanged
    tab.data_sets[0].cells[3][0] = Cell::number(6.0);
    let mut excl_num = Cell::number(7.0);
    excl_num.excluded = true;
    tab.data_sets[1].cells[3][0] = excl_num;
    tab.data_sets[2].cells[3][0] = Cell::number(8.0);

    // Row 4: unexcluded non-numeric text cell "<0.5" + excluded text cell -> skipped_incomplete_rows += 1, non_numeric_cells == 1
    tab.data_sets[0].cells[4][0] = Cell::number(9.0);
    tab.data_sets[1].cells[4][0] = Cell::text("<0.5");
    let mut excl_txt = Cell::text("bdl");
    excl_txt.excluded = true;
    tab.data_sets[2].cells[4][0] = excl_txt;

    // Rows 5..7 are trailing empty rows -> outside active_end, ignored.

    let report = tab.complete_paired_rows_with_report(&[0, 1, 2]);
    assert_eq!(report.blocks, vec![vec![1.0, 2.0, 3.0]]);
    assert_eq!(report.skipped_incomplete_rows, 3, "partial, excluded, and text rows must be counted once each");
    assert_eq!(report.non_numeric_cells, 1, "only unexcluded non-numeric cells count toward non_numeric_cells");
    assert_eq!(tab.complete_paired_rows(&[0, 1, 2]), report.blocks);

    // Empty column slice yields default empty report.
    let empty = tab.complete_paired_rows_with_report(&[]);
    assert!(empty.blocks.is_empty());
    assert_eq!(empty.skipped_incomplete_rows, 0);
    assert_eq!(empty.non_numeric_cells, 0);
}
