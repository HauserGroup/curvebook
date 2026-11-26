#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable,
    DataSheet, DocError, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, LinkSource, MultipleVariablesTable,
    NestedTable, PlacementMode, ResultScalarEntry, ResultSheet, ResultTab, SheetId, SubcolumnMode, SurvivalTable, TableArchetype,
    TransformAnalysisSpec, VariableRole, XColumnMode, XyTable,
};
use curvebook_testkit::{assert_approx_eq, assert_close};
use curvebook_transform::{
    ConversionPreview, FormulaValue, ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, ScalarEnv, TargetArchetypeKind,
    TransformError, apply_import_link_pipeline, convert_datasheet_archetype, eval_scalar_expr, evaluate_formula_for_sheet, parse_formula,
    preview_datasheet_conversion,
};

pub fn assert_approx(actual: f64, expected: f64, tol: f64) {
    assert_close(actual, expected, tol, "value");
}

pub fn raw_number(cell: &Cell) -> Option<f64> {
    match cell.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

pub fn num(v: f64) -> Cell {
    Cell::number(v)
}
pub fn excl(v: f64) -> Cell {
    Cell::excluded_number(v)
}
pub fn txt(s: &str) -> Cell {
    Cell::text(s)
}
pub fn excl_txt(s: &str) -> Cell {
    Cell::excluded_text(s)
}
pub fn empty() -> Cell {
    Cell::empty()
}
pub fn empty_cells(n: usize) -> Vec<Cell> {
    vec![Cell::empty(); n]
}
pub fn str_vec(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}
pub fn str_rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|r| str_vec(r)).collect()
}

pub fn is_invalid_cell<T>(res: Result<T, TransformError>) -> bool {
    matches!(res, Err(TransformError::Doc(DocError::InvalidCellValue { .. })))
}

macro_rules! downcast_archetype {
    ($fn_ref:ident, $var:ident, $ty:ty) => {
        pub fn $fn_ref(sheet: &DataSheet) -> &$ty {
            match &sheet.archetype {
                TableArchetype::$var(t) => t,
                other => panic!("expected {}, got {:?}", stringify!($var), other.kind()),
            }
        }
    };
    ($fn_ref:ident, $fn_mut:ident, $var:ident, $ty:ty) => {
        downcast_archetype!($fn_ref, $var, $ty);
        pub fn $fn_mut(sheet: &mut DataSheet) -> &mut $ty {
            match &mut sheet.archetype {
                TableArchetype::$var(t) => t,
                other => panic!("expected {}, got {:?}", stringify!($var), other.kind()),
            }
        }
    };
}

downcast_archetype!(as_xy, Xy, XyTable);
downcast_archetype!(as_col, as_col_mut, Column, ColumnTable);
downcast_archetype!(as_grouped, Grouped, GroupedTable);
downcast_archetype!(as_mv, as_mv_mut, MultipleVariables, MultipleVariablesTable);
downcast_archetype!(as_contingency, Contingency, ContingencyTable);
downcast_archetype!(as_survival, Survival, SurvivalTable);
downcast_archetype!(as_nested, Nested, NestedTable);

pub fn assert_lossy_convert(sheet: &DataSheet, target: TargetArchetypeKind) -> (DataSheet, ConversionPreview) {
    let prev = preview_datasheet_conversion(sheet, target).unwrap();
    assert!(prev.lossy, "expected lossy conversion to {target:?}");
    assert!(matches!(convert_datasheet_archetype(sheet, target, false), Err(TransformError::InvalidSpec { .. })));
    let (out, forced) = convert_datasheet_archetype(sheet, target, true).unwrap();
    assert!(forced.lossy);
    (out, forced)
}

pub fn sheet(id: &str, archetype: TableArchetype) -> DataSheet {
    DataSheet::new(id, format!("Sheet {id}"), archetype).unwrap()
}

pub fn raw_sheet(id: &str, archetype: TableArchetype) -> DataSheet {
    DataSheet {
        id: SheetId::new(id),
        title: format!("Sheet {id}"),
        archetype,
        frozen: false,
        info_sheet_id: None,
        linked_regions: Vec::new(),
        floating_notes: Vec::new(),
        color_tag: None,
        exclusion_notes: Vec::new(),
        plate_well_origins: Vec::new(),
    }
}

pub fn col_empty(id: &str, n_cols: usize, n_rows: usize) -> DataSheet {
    sheet(id, TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, n_cols, n_rows).unwrap()))
}

pub fn col_sheet(id: &str, cols: &[(&str, &[Cell])]) -> DataSheet {
    let n_rows = cols.first().map_or(0, |(_, cells)| cells.len());
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len().max(1), n_rows).unwrap();
    for (c, (title, cells)) in cols.iter().enumerate() {
        tbl.data_sets[c].title = (*title).to_owned();
        for (r, cell) in cells.iter().enumerate() {
            tbl.data_sets[c].cells[r][0] = cell.clone();
        }
    }
    sheet(id, TableArchetype::Column(tbl))
}

pub fn col_nums(id: &str, cols: &[(&str, &[f64])]) -> DataSheet {
    let owned: Vec<(&str, Vec<Cell>)> = cols.iter().map(|&(t, vs)| (t, vs.iter().copied().map(Cell::number).collect())).collect();
    let refs: Vec<(&str, &[Cell])> = owned.iter().map(|(t, cs)| (*t, cs.as_slice())).collect();
    col_sheet(id, &refs)
}

pub fn xy_single(id: &str, xs: &[f64], ys: &[&[f64]]) -> DataSheet {
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, ys.len(), xs.len()).unwrap();
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
    }
    for (c, col) in ys.iter().enumerate() {
        for (r, &y) in col.iter().enumerate() {
            xy.data_sets[c].cells[r][0] = Cell::number(y);
        }
    }
    sheet(id, TableArchetype::Xy(xy))
}

pub fn xy_opt(id: &str, xs: &[f64], cols: &[(&str, &[Option<f64>])]) -> DataSheet {
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, cols.len(), xs.len())
            .unwrap();
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
    }
    for (c, (title, col)) in cols.iter().enumerate() {
        if !title.is_empty() {
            xy.data_sets[c].title = (*title).to_owned();
        }
        for (r, &opt_y) in col.iter().enumerate() {
            xy.data_sets[c].cells[r][0] = opt_y.map_or_else(Cell::empty, Cell::number);
        }
    }
    sheet(id, TableArchetype::Xy(xy))
}

pub struct Xy3Col<'a> {
    pub title: &'a str,
    pub triples: &'a [[Option<f64>; 3]],
    pub excl_primary: &'a [bool],
}

pub fn xy_3sub(id: &str, mode: SubcolumnMode, xs: &[f64], cols: &[Xy3Col<'_>]) -> DataSheet {
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, cols.len(), xs.len()).unwrap();
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
    }
    for (c, col) in cols.iter().enumerate() {
        xy.data_sets[c].title = col.title.to_owned();
        for (r, vals) in col.triples.iter().enumerate() {
            let ex = col.excl_primary.get(r).copied().unwrap_or(false);
            xy.data_sets[c].cells[r] = vec![
                vals[0].map_or_else(|| Cell { value: CellValue::Empty, excluded: ex }, |v| Cell { value: CellValue::Number(v), excluded: ex }),
                vals[1].map_or_else(Cell::empty, Cell::number),
                vals[2].map_or_else(Cell::empty, Cell::number),
            ];
        }
    }
    sheet(id, TableArchetype::Xy(xy))
}

pub fn xy_3sub_clean(id: &str, mode: SubcolumnMode, xs: &[f64], cols: &[(&str, &[[Option<f64>; 3]])]) -> DataSheet {
    let specs: Vec<Xy3Col<'_>> = cols.iter().map(|&(title, triples)| Xy3Col { title, triples, excl_primary: &[] }).collect();
    xy_3sub(id, mode, xs, &specs)
}

pub fn grouped_sheet(id: &str, mode: SubcolumnMode, row_titles: &[&str], cols: &[(&str, &[&[Cell]])]) -> DataSheet {
    let n_rows = cols.first().map_or(0, |(_, rows)| rows.len());
    let mut grp = GroupedTable::new(mode, cols.len(), n_rows).unwrap();
    if !row_titles.is_empty() {
        grp.row_titles = str_vec(row_titles);
    }
    for (c, (title, rows)) in cols.iter().enumerate() {
        if !title.is_empty() {
            grp.data_sets[c].title = (*title).to_owned();
        }
        for (r, row) in rows.iter().enumerate() {
            grp.data_sets[c].cells[r] = row.to_vec();
        }
    }
    sheet(id, TableArchetype::Grouped(grp))
}

pub fn mv_table(cols: &[(&str, &[Cell])]) -> MultipleVariablesTable {
    let n_rows = cols.first().map_or(0, |(_, cells)| cells.len());
    let mut mv = MultipleVariablesTable::new(cols.len(), n_rows).unwrap();
    for (c, (title, cells)) in cols.iter().enumerate() {
        if !title.is_empty() {
            mv.variables[c].title = (*title).to_owned();
        }
        mv.variables[c].cells = cells.to_vec();
    }
    mv
}

pub fn mv_sheet(id: &str, cols: &[(&str, &[Cell])]) -> DataSheet {
    sheet(id, TableArchetype::MultipleVariables(mv_table(cols)))
}

pub fn mv_sheet_inferred(id: &str, cols: &[(&str, &[Cell])]) -> DataSheet {
    let mut mv = mv_table(cols);
    for v in &mut mv.variables {
        v.refresh_inferred_role();
    }
    sheet(id, TableArchetype::MultipleVariables(mv))
}

pub fn mv_sheet_roles(id: &str, cols: &[(&str, VariableRole, &[Cell])]) -> DataSheet {
    let n_rows = cols.first().map_or(0, |(_, _, cells)| cells.len());
    let mut mv = MultipleVariablesTable::new(cols.len(), n_rows).unwrap();
    for (c, (title, role, cells)) in cols.iter().enumerate() {
        if !title.is_empty() {
            mv.variables[c].title = (*title).to_owned();
        }
        mv.variables[c].lock_role(*role);
        mv.variables[c].cells = cells.to_vec();
    }
    sheet(id, TableArchetype::MultipleVariables(mv))
}

pub fn mv_empty(id: &str, n_vars: usize, n_rows: usize) -> DataSheet {
    sheet(id, TableArchetype::MultipleVariables(MultipleVariablesTable::new(n_vars, n_rows).unwrap()))
}

pub fn survival_sheet(id: &str, time_title: &str, cohort_title: &str, rows: &[(f64, f64)]) -> DataSheet {
    let mut srv = SurvivalTable::new(1, rows.len()).unwrap();
    if !time_title.is_empty() {
        srv.time_title = time_title.to_owned();
    }
    if !cohort_title.is_empty() {
        srv.cohorts[0].title = cohort_title.to_owned();
    }
    for (r, &(t, code)) in rows.iter().enumerate() {
        srv.times[r] = Cell::number(t);
        srv.cohorts[0].cells[r][0] = Cell::number(code);
    }
    sheet(id, TableArchetype::Survival(srv))
}

pub fn contingency_sheet(id: &str, cols: &[&[f64]]) -> DataSheet {
    let n_rows = cols.first().map_or(0, |c| c.len());
    let mut cont = ContingencyTable::new(cols.len(), n_rows).unwrap();
    for (c, col) in cols.iter().enumerate() {
        for (r, &v) in col.iter().enumerate() {
            cont.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    sheet(id, TableArchetype::Contingency(cont))
}

pub fn info_with_numbers(id: &str, entries: &[(&str, f64)]) -> InfoSheet {
    let mut info = InfoSheet::new_preseeded(id, format!("Info {id}")).unwrap();
    for &(k, v) in entries {
        info.set_constant(k, ConstantValue::Number(v), None).unwrap();
    }
    info
}

pub fn add_excl(sheet: &mut DataSheet, coord: CellCoord, tag: &str, note: &str) {
    sheet.set_excluded_with_reason(coord, tag, note, "2026-11-19T00:00:00Z").unwrap();
}

pub fn excl_note(coord: CellCoord, tag: &str, note: &str) -> CellExclusionNote {
    CellExclusionNote { coord, reason_tag: tag.to_owned(), note: note.to_owned(), timestamp_utc: String::from("2026-11-19T00:00:00Z") }
}

pub fn clipboard_spec(mode: PlacementMode, anchor: CellCoord, filter: ImportFilterSpec) -> ImportLinkSpec {
    ImportLinkSpec { source: LinkSource::Clipboard, filter, placement: ImportPlacementSpec { mode, anchor, ..ImportPlacementSpec::default() } }
}

pub fn import_delim(
    target: &mut DataSheet,
    text: &str,
    delim: char,
    dec: char,
    spec: &ImportLinkSpec,
) -> Result<ImportPipelineReport, TransformError> {
    apply_import_link_pipeline(
        target,
        spec,
        ImportSourcePayload::DelimitedText { text, delimiter: delim, decimal_separator: dec },
        &ImportPipelineOptions::default(),
        None,
    )
}
pub fn import_csv(target: &mut DataSheet, text: &str, spec: &ImportLinkSpec) -> Result<ImportPipelineReport, TransformError> {
    import_delim(target, text, ',', '.', spec)
}
pub fn import_tsv(target: &mut DataSheet, text: &str, spec: &ImportLinkSpec) -> Result<ImportPipelineReport, TransformError> {
    import_delim(target, text, '\t', '.', spec)
}

pub fn result_sheet_with_tab(id: &str, tab: ResultTab) -> ResultSheet {
    let analysis =
        AnalysisSpec::new(id, "src", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false })).unwrap();
    let mut res = ResultSheet::new(analysis).unwrap();
    res.tabs.push(tab);
    res
}

#[derive(Clone, Copy, Debug)]
pub enum ExpectVal {
    Num(f64),
    Missing,
}

pub type RowCheck = (usize, ExpectVal);
pub type FormulaInfoCase<'a> = (&'a str, bool, &'a [RowCheck]);

#[derive(Clone, Copy, Debug)]
pub enum SubExp {
    Num(f64),
    Empty,
    Any,
}

pub fn assert_formula_rows(sheet: &DataSheet, info: Option<&InfoSheet>, expr: &str, checks: &[RowCheck], tol: f64) {
    let out = evaluate_formula_for_sheet(expr, sheet, info).unwrap_or_else(|e| panic!("formula `{expr}` failed: {e:?}"));
    for &(row, expected) in checks {
        match expected {
            ExpectVal::Num(v) => {
                let got = out[row].as_finite_f64().unwrap_or_else(|| panic!("`{expr}` row {row}: expected {v}, got {:?}", out[row]));
                assert_close(got, v, tol, &format!("`{expr}` row {row}"));
            }
            ExpectVal::Missing => assert_eq!(out[row], FormulaValue::Missing, "`{expr}` row {row}: expected Missing"),
        }
    }
}

pub fn assert_formula_first(sheet: &DataSheet, info: Option<&InfoSheet>, expr: &str, expected: ExpectVal, tol: f64) {
    assert_formula_rows(sheet, info, expr, &[(0, expected)], tol);
}

pub fn assert_scalar_cases(cases: &[(&str, f64)], tol: f64) {
    let env = ScalarEnv::default();
    for &(src, expected) in cases {
        let ast = parse_formula(src).unwrap_or_else(|e| panic!("parse `{src}` failed: {e:?}"));
        let val = eval_scalar_expr(&ast, &env)
            .unwrap_or_else(|e| panic!("eval `{src}` failed: {e:?}"))
            .as_finite_f64()
            .unwrap_or_else(|| panic!("`{src}` non-finite"));
        assert_close(val, expected, tol, src);
    }
}

pub fn assert_cells_3(cells: &[Cell], expected: [SubExp; 3], tol: (f64, f64), label: &str) {
    for (s, exp) in expected.into_iter().enumerate() {
        match exp {
            SubExp::Num(v) => assert_approx_eq(
                cells[s].active_number().unwrap_or_else(|| panic!("{label} sub {s}: {:?}", cells[s])),
                v,
                tol.0,
                tol.1,
                &format!("{label} sub {s}"),
            ),
            SubExp::Empty => assert_eq!(cells[s].value, CellValue::Empty, "{label} sub {s}: {:?}", cells[s]),
            SubExp::Any => {}
        }
    }
}

pub fn assert_col_vals(sheet: &DataSheet, col: usize, vals: &[Option<f64>], excl_rows: &[usize], tol: f64, label: &str) {
    let tbl = as_col(sheet);
    assert_eq!(tbl.data_sets[col].cells.len(), vals.len(), "{label} col {col} row count");
    for (r, &exp_val) in vals.iter().enumerate() {
        let cell = &tbl.data_sets[col].cells[r][0];
        assert_eq!(cell.excluded, excl_rows.contains(&r), "{label} col {col} row {r} excluded mismatch");
        match exp_val {
            Some(v) => assert_approx_eq(
                cell.value.finite_f64().unwrap_or_else(|| panic!("{label} c{col} r{r}: {cell:?}")),
                v,
                tol,
                tol,
                &format!("{label} c{col} r{r}"),
            ),
            None => assert_eq!(cell.value, CellValue::Empty, "{label} c{col} r{r} expected Empty"),
        }
    }
}

pub fn assert_scalar_anchor(scalars: &[ResultScalarEntry], key: &str, ds_idx: Option<usize>, expected: f64, tol: f64, label: &str) {
    let entry = scalars.iter().find(|s| s.key == key && s.data_set_index == ds_idx).unwrap_or_else(|| panic!("{label}: missing ({key}, {ds_idx:?})"));
    assert_eq!(entry.units, None, "{label}: scalar {key} units");
    assert_approx_eq(entry.value, expected, tol, 0.0, &format!("{label} {key} {ds_idx:?}"));
}
