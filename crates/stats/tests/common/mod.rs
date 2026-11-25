//! Shared fixture builders, spec constructors, and result assertion helpers for
//! `curvebook-stats` integration test suites (`audit_suite` and `core_suite`).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, GroupedTable,
    MultipleVariablesTable, NestedTable, PartsOfWholeTable, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SubcolumnMode,
    SurvivalObservation, SurvivalStatus, SurvivalTable, TableArchetype, VariableRole, XyTable,
};
use curvebook_stats::multiplicity::GroupComparisonInput;
use curvebook_stats::{StatsError, run_analysis};

/// Phrases forbidden in diagnostic checklist copy (diagnostics-only policy).
pub const FORBIDDEN_RECOMMENDATION_PHRASES: &[&str] = &[
    "use mann-whitney",
    "use mann\u{2013}whitney",
    "use wilcoxon",
    "use welch",
    "consider welch",
    "consider kruskal",
    "use kruskal",
    "use friedman",
    "consider friedman",
    "consider fisher",
    "prefer fisher",
    "try spearman",
    "apply log",
    "log transform",
    "remove outliers",
    "is recommended",
    "consider enabling",
    "consider brown-forsythe",
    "warrants larger",
];

// ============================================================================
// Assertion Helpers
// ============================================================================

pub fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    curvebook_testkit::assert_close(actual, expected, tol, label);
}

pub fn assert_all_sheet_numbers_finite(res: &ResultSheet, label: &str) {
    for sc in &res.scalars {
        assert!(sc.value.is_finite(), "{label}: non-finite scalar `{}` = {}", sc.key, sc.value);
    }
    for tab in &res.tabs {
        for (r_idx, row) in tab.rows.iter().enumerate() {
            for (c_idx, cell) in row.iter().enumerate() {
                match cell {
                    ResultCell::Number { value, .. } => {
                        assert!(value.is_finite(), "{label}: non-finite Number at tab `{}`[{r_idx}][{c_idx}]: {value}", tab.id);
                    }
                    ResultCell::ConfidenceInterval { lower, upper } => {
                        if let CiBound::Finite(v) = lower {
                            assert!(v.is_finite(), "{label}: non-finite CI lower at tab `{}`[{r_idx}][{c_idx}]: {v}", tab.id);
                        }
                        if let CiBound::Finite(v) = upper {
                            assert!(v.is_finite(), "{label}: non-finite CI upper at tab `{}`[{r_idx}][{c_idx}]: {v}", tab.id);
                        }
                    }
                    ResultCell::PValue { raw, .. } => {
                        assert!(raw.is_finite() && (0.0..=1.0).contains(raw), "{label}: bad PValue at tab `{}`[{r_idx}][{c_idx}]: {raw}", tab.id);
                    }
                    _ => {}
                }
            }
        }
    }
    for rec in &res.pairwise_comparisons {
        if let Some(d) = rec.estimate_diff {
            assert!(d.is_finite(), "{label}: non-finite pairwise diff = {d}");
        }
        assert!(rec.raw_p_or_q.is_finite(), "{label}: non-finite pairwise p = {}", rec.raw_p_or_q);
    }
    for curve in &res.fitted_curves {
        for pt in &curve.points {
            assert!(pt.x.is_finite() && pt.y.is_finite(), "{label}: non-finite curve pt ({}, {})", pt.x, pt.y);
            assert!(pt.ci_lower.is_none_or(f64::is_finite), "{label}: non-finite ci_lower");
            assert!(pt.ci_upper.is_none_or(f64::is_finite), "{label}: non-finite ci_upper");
            assert!(pt.pred_lower.is_none_or(f64::is_finite), "{label}: non-finite pred_lower");
            assert!(pt.pred_upper.is_none_or(f64::is_finite), "{label}: non-finite pred_upper");
        }
    }
}

pub fn assert_diagnostics_only(res: &ResultSheet, context: &str) {
    for item in &res.checklist {
        assert!(item.remediation.is_none(), "{context}: expected remediation: None on {:?}, got {:?}", item.assumption, item.remediation);
        let lower = item.explanation.to_ascii_lowercase();
        for phrase in FORBIDDEN_RECOMMENDATION_PHRASES {
            assert!(
                !lower.contains(phrase),
                "{context}: checklist item {:?} contains forbidden phrase {:?} in {:?}",
                item.assumption,
                phrase,
                item.explanation
            );
        }
    }
}

pub fn assert_p_stars_ns(cell: &ResultCell, ctx: &str) {
    match cell {
        ResultCell::PValue { raw, stars, .. } => {
            assert!(raw.is_finite(), "{ctx}: raw P must be finite");
            assert!(!stars.contains('*') && (stars == "ns" || stars.is_empty()), "{ctx}: expected non-starred P cell for P={raw}, got `{stars}`");
        }
        other => panic!("{ctx}: expected PValue cell, got {other:?}"),
    }
}

/// Verifies every analysis kind rejects out-of-bounds and duplicate `selected_data_sets`.
pub fn assert_bad_selections(sheet: &DataSheet, kinds: &[AnalysisKind], oob: &[usize], dup: &[usize]) {
    for kind in kinds {
        let oob_err = run_kind_sel(sheet, kind.clone(), oob.to_vec()).unwrap_err();
        assert!(matches!(oob_err, StatsError::MissingColumn { .. }), "kind={kind:?} oob={oob:?} expected MissingColumn, got {oob_err:?}");
        let dup_err = run_kind_sel(sheet, kind.clone(), dup.to_vec()).unwrap_err();
        assert!(
            matches!(dup_err, StatsError::InvalidParameter { name: "selected_data_sets", .. }),
            "kind={kind:?} dup={dup:?} expected duplicate InvalidParameter, got {dup_err:?}"
        );
    }
}

// ============================================================================
// Table & DataSheet Builders
// ============================================================================

pub fn col_sheet(id: &str, cols: &[(&str, &[f64])]) -> DataSheet {
    col_sheet_hint(id, ColumnDesignHint::UnpairedReplicates, cols)
}

pub fn col_sheet_hint(id: &str, hint: ColumnDesignHint, cols: &[(&str, &[f64])]) -> DataSheet {
    let rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut t = ColumnTable::new(hint, cols.len(), rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            t.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Column(t)).unwrap()
}

pub fn col_sheet_opts(id: &str, hint: ColumnDesignHint, cols: &[(&str, &[Option<f64>])]) -> DataSheet {
    let rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut t = ColumnTable::new(hint, cols.len(), rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
        for (r, v) in vals.iter().enumerate() {
            if let Some(num) = v {
                t.data_sets[c].cells[r][0] = Cell::number(*num);
            }
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Column(t)).unwrap()
}

pub fn col_sheet_cells(id: &str, hint: ColumnDesignHint, cols: &[(&str, &[Cell])]) -> DataSheet {
    let rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut t = ColumnTable::new(hint, cols.len(), rows).unwrap();
    for (c, (title, cells)) in cols.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
        for (r, cell) in cells.iter().enumerate() {
            t.data_sets[c].cells[r][0] = cell.clone();
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Column(t)).unwrap()
}

pub fn presum_col_sheet(id: &str, mode: SubcolumnMode, cols: &[(&str, &[&[f64]])]) -> DataSheet {
    let rows = cols.iter().map(|(_, r)| r.len()).max().unwrap_or(1).max(1);
    let mut t = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, cols.len(), rows).unwrap();
    for (c, (title, row_slices)) in cols.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
        for (r, sub) in row_slices.iter().enumerate() {
            for (s, &v) in sub.iter().enumerate() {
                t.data_sets[c].cells[r][s] = Cell::number(v);
            }
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Column(t)).unwrap()
}

pub fn grouped_sheet(id: &str, mode: SubcolumnMode, row_titles: &[&str], cols: &[(&str, &[&[f64]])]) -> DataSheet {
    let n_rows = row_titles.len().max(1);
    let mut t = GroupedTable::new(mode, cols.len(), n_rows).unwrap();
    t.row_titles = row_titles.iter().map(|s| (*s).to_owned()).collect();
    for (c, (title, rows)) in cols.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
        for (r, reps) in rows.iter().enumerate() {
            for (s, &v) in reps.iter().enumerate() {
                t.data_sets[c].cells[r][s] = Cell::number(v);
            }
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Grouped(t)).unwrap()
}

pub fn grouped_sheet_fill(id: &str, n_sets: usize, n_rows: usize, subs: usize, fill: impl Fn(usize, usize, usize) -> Option<f64>) -> DataSheet {
    let mut t = GroupedTable::new(SubcolumnMode::RawReplicates(subs), n_sets, n_rows).unwrap();
    for c in 0..n_sets {
        for r in 0..n_rows {
            for s in 0..subs {
                if let Some(v) = fill(c, r, s) {
                    t.data_sets[c].cells[r][s] = Cell::number(v);
                }
            }
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Grouped(t)).unwrap()
}

pub fn grouped_sheet_entries(
    id: &str,
    row_factor: &str,
    col_factor: &str,
    row_titles: &[&str],
    col_titles: &[&str],
    n_reps: usize,
    entries: &[(usize, usize, usize, f64)],
) -> DataSheet {
    let mut t = GroupedTable::new(SubcolumnMode::RawReplicates(n_reps), col_titles.len(), row_titles.len()).unwrap();
    t.row_factor_title = row_factor.to_owned();
    t.column_factor_title = col_factor.to_owned();
    t.row_titles = row_titles.iter().map(|s| (*s).to_owned()).collect();
    for (c, title) in col_titles.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
    }
    for &(s, c, r, y) in entries {
        t.data_sets[c].cells[r][s] = Cell::number(y);
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Grouped(t)).unwrap()
}

pub fn xy_sheet(id: &str, x: &[f64], y: &[f64]) -> DataSheet {
    xy_multi_sheet(id, x, &[("Y", y)])
}

pub fn xy_multi_sheet(id: &str, x: &[f64], ys: &[(&str, &[f64])]) -> DataSheet {
    let n = x.len().max(1);
    let mut t = XyTable::new(SubcolumnMode::Single, ys.len(), n).unwrap();
    t.x_title = String::from("X");
    for (r, &xv) in x.iter().enumerate() {
        t.set_x_cell(r, Cell::number(xv)).unwrap();
    }
    for (c, (title, vals)) in ys.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
        for (r, &yv) in vals.iter().enumerate() {
            t.data_sets[c].cells[r][0] = Cell::number(yv);
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Xy(t)).unwrap()
}

pub fn xy_rep_sheet(id: &str, mode: SubcolumnMode, x: &[f64], ys: &[(&str, &[&[f64]])]) -> DataSheet {
    let n = x.len().max(1);
    let mut t = XyTable::new(mode, ys.len(), n).unwrap();
    t.x_title = String::from("X");
    for (r, &xv) in x.iter().enumerate() {
        t.set_x_cell(r, Cell::number(xv)).unwrap();
    }
    for (c, (title, rows)) in ys.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
        for (r, reps) in rows.iter().enumerate() {
            for (s, &yv) in reps.iter().enumerate() {
                t.data_sets[c].cells[r][s] = Cell::number(yv);
            }
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Xy(t)).unwrap()
}

pub fn mv_num_sheet(id: &str, cols: &[(&str, &[f64])]) -> DataSheet {
    let n = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut t = MultipleVariablesTable::new(cols.len(), n).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        t.variables[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            t.variables[c].cells[r] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::MultipleVariables(t)).unwrap()
}

pub fn mv_opt_sheet(id: &str, cols: &[(&str, &[Option<f64>])]) -> DataSheet {
    let n = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut t = MultipleVariablesTable::new(cols.len(), n).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        t.variables[c].title = (*title).to_owned();
        for (r, v) in vals.iter().enumerate() {
            if let Some(num) = v {
                t.variables[c].cells[r] = Cell::number(*num);
            }
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::MultipleVariables(t)).unwrap()
}

pub fn mv_cells_sheet(id: &str, cols: &[(&str, Option<VariableRole>, &[Cell])]) -> DataSheet {
    let n = cols.iter().map(|(_, _, v)| v.len()).max().unwrap_or(1).max(1);
    let mut t = MultipleVariablesTable::new(cols.len(), n).unwrap();
    for (c, (title, role, cells)) in cols.iter().enumerate() {
        t.variables[c].title = (*title).to_owned();
        for (r, cell) in cells.iter().enumerate() {
            t.variables[c].cells[r] = cell.clone();
        }
        if let Some(rl) = role {
            t.variables[c].lock_role(*rl);
        } else {
            t.variables[c].refresh_inferred_role();
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::MultipleVariables(t)).unwrap()
}

pub fn contingency_sheet(id: &str, rows: &[&[u64]]) -> DataSheet {
    let n_rows = rows.len().max(1);
    let n_cols = rows.first().map_or(1, |r| r.len()).max(1);
    let mut t = ContingencyTable::new(n_cols, n_rows).unwrap();
    for (r, row) in rows.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            t.data_sets[c].cells[r][0] = Cell::number(v as f64);
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Contingency(t)).unwrap()
}

pub fn parts_sheet(id: &str, slices: &[(&str, f64)]) -> DataSheet {
    let labels: Vec<&str> = slices.iter().map(|(l, _)| *l).collect();
    let vals: Vec<f64> = slices.iter().map(|(_, v)| *v).collect();
    parts_multi_sheet(id, &labels, &[("Counts", &vals)])
}

pub fn parts_multi_sheet(id: &str, labels: &[&str], cols: &[(&str, &[f64])]) -> DataSheet {
    let mut t = PartsOfWholeTable::new(cols.len(), labels.len()).unwrap();
    t.slice_labels = labels.iter().map(|s| (*s).to_owned()).collect();
    for (c, (title, vals)) in cols.iter().enumerate() {
        t.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            t.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::PartsOfWhole(t)).unwrap()
}

pub fn survival_sheet(id: &str, cohorts: &[(&str, &[(f64, bool)])]) -> DataSheet {
    let total: usize = cohorts.iter().map(|(_, rows)| rows.len()).sum();
    let mut t = SurvivalTable::new(cohorts.len(), total.max(1)).unwrap();
    let mut row = 0;
    for (c, (title, pairs)) in cohorts.iter().enumerate() {
        t.cohorts[c].title = (*title).to_owned();
        for &(time, ev) in *pairs {
            t.times[row] = Cell::number(time);
            t.cohorts[c].cells[row][0] = Cell::number(if ev { 1.0 } else { 0.0 });
            row += 1;
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Survival(t)).unwrap()
}

pub fn nested_table(groups: &[(&str, &[&[f64]])]) -> NestedTable {
    let max_sub = groups.iter().map(|(_, s)| s.len()).max().unwrap_or(1).max(1);
    let max_rows = groups.iter().flat_map(|(_, s)| s.iter().map(|r| r.len())).max().unwrap_or(1).max(1);
    let mut t = NestedTable::new(groups.len(), max_sub, max_rows).unwrap();
    for (g, (title, subs)) in groups.iter().enumerate() {
        t.groups[g].title = (*title).to_owned();
        for (b, reps) in subs.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                t.groups[g].cells[r][b] = Cell::number(v);
            }
        }
    }
    t
}

pub fn nested_sheet(id: &str, groups: &[(&str, &[&[f64]])]) -> DataSheet {
    DataSheet::new(SheetId::new(id), id, TableArchetype::Nested(nested_table(groups))).unwrap()
}

pub fn obs(pairs: &[(f64, bool)]) -> Vec<SurvivalObservation> {
    pairs
        .iter()
        .enumerate()
        .map(|(row, &(time, ev))| SurvivalObservation { row, time, status: if ev { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect()
}

pub fn group_input(index: usize, label: &str, n: f64, mean: f64, sd: f64) -> GroupComparisonInput {
    GroupComparisonInput { index, label: label.to_owned(), n, mean, sd }
}

pub fn group_input_slice(index: usize, label: &str, data: &[f64]) -> GroupComparisonInput {
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    let var = data.iter().map(|&x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
    GroupComparisonInput { index, label: label.to_owned(), n, mean, sd: var.sqrt() }
}

// ============================================================================
// Spec & Runner Helpers
// ============================================================================

pub fn spec_for(id: &str, source: &str, kind: AnalysisKind) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new(id), SheetId::new(source), kind).unwrap()
}

pub fn spec_sel(source: &str, kind: AnalysisKind, sel: Vec<usize>) -> AnalysisSpec {
    let mut s = spec_for("res", source, kind);
    s.selected_data_sets = sel;
    s
}

pub fn spec_conf(source: &str, kind: AnalysisKind, pct: f64) -> AnalysisSpec {
    let mut s = spec_for("res", source, kind);
    s.set_confidence_percent(pct).unwrap();
    s
}

pub fn run_kind(sheet: &DataSheet, kind: AnalysisKind) -> Result<ResultSheet, StatsError> {
    run_analysis(sheet, &spec_for("res", sheet.id.as_str(), kind))
}

pub fn run_kind_sel(sheet: &DataSheet, kind: AnalysisKind, sel: Vec<usize>) -> Result<ResultSheet, StatsError> {
    run_analysis(sheet, &spec_sel(sheet.id.as_str(), kind, sel))
}

pub fn run_kind_conf(sheet: &DataSheet, kind: AnalysisKind, pct: f64) -> Result<ResultSheet, StatsError> {
    run_analysis(sheet, &spec_conf(sheet.id.as_str(), kind, pct))
}

// ============================================================================
// Result Inspection Helpers
// ============================================================================

pub fn tab_by_id<'a>(res: &'a ResultSheet, id: &str) -> &'a ResultTab {
    res.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}` in {:?}", res.tabs.iter().map(|t| &t.id).collect::<Vec<_>>()))
}

pub fn tab_by_kind(res: &ResultSheet, kind: ResultTabKind) -> &ResultTab {
    res.tabs.iter().find(|t| t.kind == kind).unwrap_or_else(|| panic!("missing tab kind `{kind:?}`"))
}

pub fn scalar_opt(res: &ResultSheet, key: &str, ds: Option<usize>) -> Option<f64> {
    res.scalars.iter().find(|s| s.key == key && s.data_set_index == ds).map(|s| s.value)
}

pub fn scalar(res: &ResultSheet, key: &str) -> f64 {
    res.scalars.iter().find(|s| s.key == key).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

pub fn scalar_ds(res: &ResultSheet, key: &str, ds: Option<usize>) -> f64 {
    scalar_opt(res, key, ds).unwrap_or_else(|| panic!("missing scalar `{key}` (ds={ds:?})"))
}

pub fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected Number cell, got {other:?}"),
    }
}

pub fn cell_p(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::PValue { raw, .. } => *raw,
        other => panic!("expected PValue cell, got {other:?}"),
    }
}

pub fn cell_p_stars(cell: &ResultCell) -> (f64, &str) {
    match cell {
        ResultCell::PValue { raw, stars, .. } => (*raw, stars.as_str()),
        other => panic!("expected PValue cell, got {other:?}"),
    }
}

pub fn cell_ci_finite(cell: &ResultCell) -> (f64, f64) {
    match cell {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } => (*lo, *hi),
        other => panic!("expected finite ConfidenceInterval cell, got {other:?}"),
    }
}

pub fn cell_text(cell: &ResultCell) -> &str {
    match cell {
        ResultCell::Text(s) => s.as_str(),
        other => panic!("expected Text cell, got {other:?}"),
    }
}

pub fn cell_bool(cell: &ResultCell) -> bool {
    match cell {
        ResultCell::Boolean(b) => *b,
        other => panic!("expected Boolean cell, got {other:?}"),
    }
}

pub fn row_idx(tab: &ResultTab, label: &str) -> usize {
    tab.row_labels.iter().position(|r| r == label).unwrap_or_else(|| panic!("missing row `{label}` in tab `{}`: {:?}", tab.id, tab.row_labels))
}

pub fn row_cell<'a>(tab: &'a ResultTab, label: &str, col: usize) -> &'a ResultCell {
    &tab.rows[row_idx(tab, label)][col]
}

pub fn has_checklist(res: &ResultSheet, substr: &str, status: ChecklistStatus) -> bool {
    res.checklist.iter().any(|c| c.status == status && (c.assumption.contains(substr) || c.explanation.contains(substr)))
}

pub fn has_ignored_text_warn(res: &ResultSheet) -> bool {
    let cl = res.checklist.iter().any(|c| {
        c.status == ChecklistStatus::Warn && c.assumption == "Numeric input validation" && c.explanation.contains("Non-numeric values ignored:")
    });
    let fn_note = res.tabs.first().is_some_and(|t| t.footnotes.iter().any(|f| f.contains("Non-numeric values ignored:")));
    cl && fn_note
}

pub fn has_listwise_info(res: &ResultSheet) -> bool {
    let cl = res.checklist.iter().any(|c| c.status == ChecklistStatus::Info && c.assumption == "Complete-case row alignment");
    let fn_note = res.tabs.first().is_some_and(|t| t.footnotes.iter().any(|f| f.contains("complete-case listwise alignment")));
    cl && fn_note
}
