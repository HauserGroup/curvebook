//! Vectorized and scalar formula evaluation over `DataSheet`, `MultipleVariablesTable`, and
//! `InfoSheet` constants (`docs/implementation/data-tables-and-transforms.md` §4.1).

use std::cmp::Ordering;
use std::collections::HashMap;

use curvebook_doc::{Cell, CellValue, DataSheet, InfoSheet, MultipleVariablesTable, TableArchetype, VariableRole, default_column_label};
use curvebook_math::{dist, special, sum};

use crate::error::TransformError;
use crate::formula::parser::{
    BinaryOp, Expr, FrameBound, NullsOrder, OrderByExpr, SortDirection, UnaryOp, VarRef, WindowFrame, WindowSpec, parse_formula,
};

/// Upper memory budget (64 MiB) for broadcasting a single string value across `N` rows.
const MAX_MATERIALIZED_TEXT_BYTES: usize = 64 * 1024 * 1024;

fn broadcast_formula_value(val: FormulaValue, n: usize) -> Result<Vec<FormulaValue>, TransformError> {
    if let FormulaValue::Text(ref s) = val {
        let total_bytes = s.len().saturating_mul(n);
        if total_bytes > MAX_MATERIALIZED_TEXT_BYTES {
            return Err(TransformError::InvalidSpec {
                message: format!(
                    "formula text broadcast ({} bytes across {n} rows = {total_bytes} bytes) exceeds maximum memory budget ({MAX_MATERIALIZED_TEXT_BYTES} bytes)",
                    s.len()
                ),
            });
        }
    }
    Ok(vec![val; n])
}

/// Typed runtime value produced by evaluating a formula expression.
#[derive(Debug, Clone, PartialEq)]
pub enum FormulaValue {
    /// Finite IEEE-754 floating-point number (non-finite `NaN`/`±Inf` results map to [`Self::Missing`] via [`Self::from_f64`]).
    Number(f64),
    /// Non-empty string value.
    Text(String),
    /// Propagated missing/null value produced by empty cells, excluded cells, domain errors (`ln(-1)`), or division by zero.
    Missing,
}

impl FormulaValue {
    /// Construct from an `f64`, mapping `NaN` or `±Inf` to [`Self::Missing`].
    #[must_use]
    pub fn from_f64(value: f64) -> Self {
        if value.is_finite() { Self::Number(value) } else { Self::Missing }
    }

    /// Construct from a spreadsheet [`Cell`], treating excluded or empty cells as [`Self::Missing`].
    #[must_use]
    pub fn from_cell(cell: &Cell) -> Self {
        if cell.excluded {
            return Self::Missing;
        }
        match &cell.value {
            CellValue::Empty => Self::Missing,
            CellValue::Number(v) => Self::from_f64(*v),
            CellValue::Text(s) if s.trim().is_empty() => Self::Missing,
            CellValue::Text(s) => Self::Text(s.clone()),
        }
    }

    /// Convert back into an unexcluded [`Cell`].
    #[must_use]
    pub fn into_cell(self) -> Cell {
        match self {
            Self::Number(v) => Cell::number(v),
            Self::Text(s) => Cell::text(s),
            Self::Missing => Cell::empty(),
        }
    }

    /// Extract finite `f64` when numeric.
    #[must_use]
    pub fn as_finite_f64(&self) -> Option<f64> {
        match self {
            Self::Number(v) if v.is_finite() => Some(*v),
            _ => None,
        }
    }

    /// Extract truthiness (`None` if missing or non-numeric).
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Number(v) if v.is_finite() => Some(*v != 0.0),
            Self::Text(s) => {
                if s.eq_ignore_ascii_case("true") {
                    Some(true)
                } else if s.eq_ignore_ascii_case("false") {
                    Some(false)
                } else {
                    None
                }
            }
            Self::Number(_) | Self::Missing => None,
        }
    }

    /// True if value is non-missing.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !matches!(self, Self::Missing)
    }
}

/// Read-only table & constant environment exposed to the formula evaluator.
pub trait FormulaContext {
    /// Validate structural invariants of the underlying table before evaluating expressions.
    fn validate_context(&self) -> Result<(), TransformError> {
        Ok(())
    }
    /// Number of logical rows in the evaluation domain.
    fn row_count(&self) -> usize;
    /// Resolve a case-insensitive or alias column reference (`name` or `"value"`) to a zero-based column index.
    fn resolve_column_index(&self, name: &str) -> Option<usize>;
    /// Resolve an identifier strictly against exact/trimmed column titles without fallback aliases.
    fn resolve_column_index_exact(&self, name: &str) -> Option<usize> {
        self.resolve_column_index(name)
    }
    /// Total number of addressable data/variable columns.
    fn column_count(&self) -> usize;
    /// Extract a `row_count()`-length vector of [`FormulaValue`]s for zero-based `col_idx` (mapping excluded/empty cells to [`FormulaValue::Missing`]).
    fn column_values(&self, col_idx: usize) -> Option<Vec<FormulaValue>>;
    /// Extract the independent *X* or elapsed-time vector when the backing table has one (`Xy` or `Survival`).
    fn resolve_x_column(&self) -> Option<Vec<FormulaValue>> {
        None
    }
    /// Extract the independent *X* or elapsed-time vector, propagating summary reconstruction errors.
    fn resolve_x_column_checked(&self) -> Result<Option<Vec<FormulaValue>>, TransformError> {
        Ok(self.resolve_x_column())
    }
    /// Look up a named constant from the paired [`InfoSheet`], if available.
    fn resolve_info_constant(&self, name: &str) -> Option<FormulaValue>;
    /// Look up a scalar variable binding (such as `"x"`, `"y"`, or parameter names in scalar expressions).
    fn resolve_scalar(&self, _name: &str) -> Option<FormulaValue> {
        None
    }
}

/// Scalar-only environment for `UserExpression` or custom `Simulate` formulas (`x`, `y`, params).
#[derive(Debug, Clone, Default)]
pub struct ScalarEnv<'a> {
    /// Case-insensitive `(name, value)` scalar variable bindings.
    pub bindings: Vec<(&'a str, f64)>,
    /// Optional paired [`InfoSheet`] for `Info("Key")` constant lookups.
    pub info_sheet: Option<&'a InfoSheet>,
}

impl<'a> ScalarEnv<'a> {
    /// Construct a scalar evaluation environment from `(name, value)` bindings and an optional [`InfoSheet`].
    #[must_use]
    pub fn new(bindings: Vec<(&'a str, f64)>, info_sheet: Option<&'a InfoSheet>) -> Self {
        Self { bindings, info_sheet }
    }
}

impl FormulaContext for ScalarEnv<'_> {
    fn row_count(&self) -> usize {
        1
    }

    fn resolve_column_index(&self, _name: &str) -> Option<usize> {
        None
    }

    fn column_count(&self) -> usize {
        0
    }

    fn column_values(&self, _col_idx: usize) -> Option<Vec<FormulaValue>> {
        None
    }

    fn resolve_info_constant(&self, name: &str) -> Option<FormulaValue> {
        lookup_info_value(self.info_sheet, name)
    }

    fn resolve_scalar(&self, name: &str) -> Option<FormulaValue> {
        let needle = name.trim();
        self.bindings.iter().find(|(k, _)| k.trim().eq_ignore_ascii_case(needle)).map(|(_, v)| FormulaValue::from_f64(*v))
    }
}

/// Full table context wrapping any [`DataSheet`] and optional [`InfoSheet`] + scalar bindings.
#[derive(Debug, Clone)]
pub struct TableEvalContext<'a> {
    /// Borrowed [`DataSheet`] whose columns and *X* values are exposed to formulas.
    pub sheet: &'a DataSheet,
    /// Optional paired [`InfoSheet`] for `Info("Key")` lookups.
    pub info_sheet: Option<&'a InfoSheet>,
    /// Additional case-insensitive scalar `(name, value)` bindings overriding or supplementing columns.
    pub extra_scalars: Vec<(&'a str, f64)>,
}

impl<'a> TableEvalContext<'a> {
    /// Construct a `TableEvalContext` over `sheet` and optional `info_sheet` with empty `extra_scalars`.
    #[must_use]
    pub fn new(sheet: &'a DataSheet, info_sheet: Option<&'a InfoSheet>) -> Self {
        Self { sheet, info_sheet, extra_scalars: Vec::new() }
    }
}

impl FormulaContext for TableEvalContext<'_> {
    fn validate_context(&self) -> Result<(), TransformError> {
        Ok(self.sheet.archetype.validate()?)
    }

    fn row_count(&self) -> usize {
        self.sheet.archetype.row_count()
    }

    fn resolve_column_index(&self, name: &str) -> Option<usize> {
        resolve_archetype_column(&self.sheet.archetype, name).or_else(|| {
            if !name.trim().eq_ignore_ascii_case("value") {
                return None;
            }
            match &self.sheet.archetype {
                TableArchetype::MultipleVariables(mv) => mv
                    .variables
                    .iter()
                    .position(|v| {
                        matches!(v.role, VariableRole::Continuous | VariableRole::Calculated) && v.cells.iter().any(|c| c.active_number().is_some())
                    })
                    .or_else(|| mv.variables.iter().position(|v| v.cells.iter().any(|c| c.active_number().is_some())))
                    .or_else(|| (!mv.variables.is_empty()).then_some(0)),
                _ => (self.column_count() > 0).then_some(0),
            }
        })
    }

    fn resolve_column_index_exact(&self, name: &str) -> Option<usize> {
        resolve_archetype_column_exact(&self.sheet.archetype, name)
    }

    fn column_count(&self) -> usize {
        self.sheet.archetype.column_count()
    }

    fn column_values(&self, col_idx: usize) -> Option<Vec<FormulaValue>> {
        archetype_column_values(&self.sheet.archetype, col_idx)
    }

    fn resolve_x_column(&self) -> Option<Vec<FormulaValue>> {
        archetype_x_values(&self.sheet.archetype)
    }

    fn resolve_x_column_checked(&self) -> Result<Option<Vec<FormulaValue>>, TransformError> {
        archetype_x_values_checked(&self.sheet.archetype)
    }

    fn resolve_info_constant(&self, name: &str) -> Option<FormulaValue> {
        lookup_info_value(self.info_sheet, name)
    }

    fn resolve_scalar(&self, name: &str) -> Option<FormulaValue> {
        let needle = name.trim();
        self.extra_scalars.iter().find(|(k, _)| k.trim().eq_ignore_ascii_case(needle)).map(|(_, v)| FormulaValue::from_f64(*v))
    }
}

/// Context wrapping a [`MultipleVariablesTable`] directly (used during DAG column updates).
#[derive(Debug, Clone)]
pub struct MultipleVariablesContext<'a> {
    /// Borrowed [`MultipleVariablesTable`] whose variable columns are evaluated.
    pub table: &'a MultipleVariablesTable,
    /// Optional paired [`InfoSheet`] for `Info("Key")` constant lookups.
    pub info_sheet: Option<&'a InfoSheet>,
}

impl FormulaContext for MultipleVariablesContext<'_> {
    fn validate_context(&self) -> Result<(), TransformError> {
        Ok(self.table.validate()?)
    }

    fn row_count(&self) -> usize {
        self.table.row_count()
    }

    fn resolve_column_index(&self, name: &str) -> Option<usize> {
        resolve_mv_column(self.table, name)
    }

    fn resolve_column_index_exact(&self, name: &str) -> Option<usize> {
        resolve_ident_mv_column(self.table, name)
    }

    fn column_count(&self) -> usize {
        self.table.variables.len()
    }

    fn column_values(&self, col_idx: usize) -> Option<Vec<FormulaValue>> {
        let var = self.table.variables.get(col_idx)?;
        Some(var.cells.iter().map(FormulaValue::from_cell).collect())
    }

    fn resolve_info_constant(&self, name: &str) -> Option<FormulaValue> {
        lookup_info_value(self.info_sheet, name)
    }
}

fn lookup_info_value(info_sheet: Option<&InfoSheet>, name: &str) -> Option<FormulaValue> {
    let info = info_sheet?;
    let c = info.get_constant(name)?;
    match &c.value {
        curvebook_doc::ConstantValue::Empty => Some(FormulaValue::Missing),
        curvebook_doc::ConstantValue::Number(v) => Some(FormulaValue::from_f64(*v)),
        curvebook_doc::ConstantValue::Text(s) | curvebook_doc::ConstantValue::Date(s) => {
            if let Ok(num) = s.trim().parse::<f64>()
                && num.is_finite()
            {
                Some(FormulaValue::Number(num))
            } else if s.trim().is_empty() {
                Some(FormulaValue::Missing)
            } else {
                Some(FormulaValue::Text(s.clone()))
            }
        }
    }
}

pub(crate) fn resolve_mv_column(mv: &MultipleVariablesTable, name: &str) -> Option<usize> {
    let needle = name.trim();
    if let Some(pos) = mv.variables.iter().position(|v| v.title.trim() == needle) {
        return Some(pos);
    }
    if let Some(pos) = mv.variables.iter().position(|v| v.title.trim().eq_ignore_ascii_case(needle)) {
        return Some(pos);
    }
    (0..mv.variables.len()).find(|&i| default_column_label(i).eq_ignore_ascii_case(needle))
}

pub(crate) fn resolve_ident_mv_column(mv: &MultipleVariablesTable, name: &str) -> Option<usize> {
    let needle = name.trim();
    if !is_math_constant(needle) {
        return resolve_mv_column(mv, needle);
    }
    if let Some(pos) = mv.variables.iter().position(|v| v.title.trim() == needle) {
        return Some(pos);
    }
    if !needle.eq_ignore_ascii_case("e")
        && let Some(pos) = mv.variables.iter().position(|v| v.title.trim().eq_ignore_ascii_case(needle))
    {
        return Some(pos);
    }
    (0..mv.variables.len()).find(|&i| default_column_label(i) == needle)
}

fn resolve_dataset_titles(titles: &[String], name: &str) -> Option<usize> {
    let needle = name.trim();
    if let Some(pos) = titles.iter().position(|t| t.trim() == needle) {
        return Some(pos);
    }
    if let Some(pos) = titles.iter().position(|t| t.trim().eq_ignore_ascii_case(needle)) {
        return Some(pos);
    }
    (0..titles.len()).find(|&i| default_column_label(i).eq_ignore_ascii_case(needle))
}

fn resolve_ident_dataset_titles(titles: &[String], name: &str) -> Option<usize> {
    let needle = name.trim();
    if !is_math_constant(needle) {
        return resolve_dataset_titles(titles, needle);
    }
    if let Some(pos) = titles.iter().position(|t| t.trim() == needle) {
        return Some(pos);
    }
    if !needle.eq_ignore_ascii_case("e")
        && let Some(pos) = titles.iter().position(|t| t.trim().eq_ignore_ascii_case(needle))
    {
        return Some(pos);
    }
    (0..titles.len()).find(|&i| default_column_label(i) == needle)
}

fn resolve_archetype_column_exact(arch: &TableArchetype, name: &str) -> Option<usize> {
    match arch {
        TableArchetype::MultipleVariables(mv) => resolve_ident_mv_column(mv, name),
        TableArchetype::Xy(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_ident_dataset_titles(&titles, name)
                .or_else(|| name.trim().eq_ignore_ascii_case("y").then_some(0).filter(|_| !t.data_sets.is_empty()))
        }
        TableArchetype::Column(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_ident_dataset_titles(&titles, name)
        }
        TableArchetype::Grouped(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_ident_dataset_titles(&titles, name)
        }
        TableArchetype::Contingency(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_ident_dataset_titles(&titles, name)
        }
        TableArchetype::Survival(t) => {
            let titles: Vec<String> = t.cohorts.iter().map(|d| d.title.clone()).collect();
            resolve_ident_dataset_titles(&titles, name)
        }
        TableArchetype::PartsOfWhole(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_ident_dataset_titles(&titles, name)
        }
        TableArchetype::Nested(t) => {
            let titles: Vec<String> = t.groups.iter().map(|g| g.title.clone()).collect();
            resolve_ident_dataset_titles(&titles, name)
        }
    }
}

fn resolve_archetype_column(arch: &TableArchetype, name: &str) -> Option<usize> {
    match arch {
        TableArchetype::MultipleVariables(mv) => resolve_mv_column(mv, name),
        TableArchetype::Xy(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_dataset_titles(&titles, name).or_else(|| name.trim().eq_ignore_ascii_case("y").then_some(0).filter(|_| !t.data_sets.is_empty()))
        }
        TableArchetype::Column(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_dataset_titles(&titles, name)
        }
        TableArchetype::Grouped(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_dataset_titles(&titles, name)
        }
        TableArchetype::Contingency(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_dataset_titles(&titles, name)
        }
        TableArchetype::Survival(t) => {
            let titles: Vec<String> = t.cohorts.iter().map(|d| d.title.clone()).collect();
            resolve_dataset_titles(&titles, name)
        }
        TableArchetype::PartsOfWhole(t) => {
            let titles: Vec<String> = t.data_sets.iter().map(|d| d.title.clone()).collect();
            resolve_dataset_titles(&titles, name)
        }
        TableArchetype::Nested(t) => {
            let titles: Vec<String> = t.groups.iter().map(|g| g.title.clone()).collect();
            resolve_dataset_titles(&titles, name)
        }
    }
}

fn dataset_row_value(cells: &[Vec<Cell>], row: usize) -> FormulaValue {
    let Some(subcells) = cells.get(row) else {
        return FormulaValue::Missing;
    };
    if subcells.len() <= 1 {
        return subcells.first().map_or(FormulaValue::Missing, FormulaValue::from_cell);
    }
    let active: Vec<f64> = subcells.iter().filter_map(Cell::active_number).collect();
    if active.is_empty() {
        subcells.first().map_or(FormulaValue::Missing, FormulaValue::from_cell)
    } else {
        sum::mean(&active).map_or(FormulaValue::Missing, FormulaValue::from_f64)
    }
}

fn archetype_column_values(arch: &TableArchetype, col_idx: usize) -> Option<Vec<FormulaValue>> {
    match arch {
        TableArchetype::MultipleVariables(mv) => {
            let var = mv.variables.get(col_idx)?;
            Some(var.cells.iter().map(FormulaValue::from_cell).collect())
        }
        TableArchetype::Xy(t) => {
            let ds = t.data_sets.get(col_idx)?;
            if t.y_subcolumn_mode.is_pre_summarized() {
                Some(ds.cells.iter().map(|r| r.first().map_or(FormulaValue::Missing, FormulaValue::from_cell)).collect())
            } else {
                Some((0..ds.cells.len()).map(|r| dataset_row_value(&ds.cells, r)).collect())
            }
        }
        TableArchetype::Column(t) => {
            let ds = t.data_sets.get(col_idx)?;
            Some(ds.cells.iter().map(|r| r.first().map_or(FormulaValue::Missing, FormulaValue::from_cell)).collect())
        }
        TableArchetype::Grouped(t) => {
            let ds = t.data_sets.get(col_idx)?;
            if t.subcolumn_mode.is_pre_summarized() {
                Some(ds.cells.iter().map(|r| r.first().map_or(FormulaValue::Missing, FormulaValue::from_cell)).collect())
            } else {
                Some((0..ds.cells.len()).map(|r| dataset_row_value(&ds.cells, r)).collect())
            }
        }
        TableArchetype::Contingency(t) => {
            let ds = t.data_sets.get(col_idx)?;
            Some(ds.cells.iter().map(|r| r.first().map_or(FormulaValue::Missing, FormulaValue::from_cell)).collect())
        }
        TableArchetype::Survival(t) => {
            let ds = t.cohorts.get(col_idx)?;
            Some(ds.cells.iter().map(|r| r.first().map_or(FormulaValue::Missing, FormulaValue::from_cell)).collect())
        }
        TableArchetype::PartsOfWhole(t) => {
            let ds = t.data_sets.get(col_idx)?;
            Some(ds.cells.iter().map(|r| r.first().map_or(FormulaValue::Missing, FormulaValue::from_cell)).collect())
        }
        TableArchetype::Nested(t) => {
            let grp = t.groups.get(col_idx)?;
            Some((0..grp.cells.len()).map(|r| dataset_row_value(&grp.cells, r)).collect())
        }
    }
}

fn archetype_x_values(arch: &TableArchetype) -> Option<Vec<FormulaValue>> {
    match arch {
        TableArchetype::Xy(t) => {
            Some((0..t.row_count()).map(|r| t.x_summary_at(r).ok().flatten().map_or(FormulaValue::Missing, FormulaValue::from_f64)).collect())
        }
        TableArchetype::Survival(t) => Some(t.times.iter().map(FormulaValue::from_cell).collect()),
        _ => None,
    }
}

fn archetype_x_values_checked(arch: &TableArchetype) -> Result<Option<Vec<FormulaValue>>, TransformError> {
    match arch {
        TableArchetype::Xy(t) => {
            let mut out = Vec::with_capacity(t.row_count());
            for r in 0..t.row_count() {
                let v = t.x_summary_at(r)?;
                out.push(v.map_or(FormulaValue::Missing, FormulaValue::from_f64));
            }
            Ok(Some(out))
        }
        TableArchetype::Survival(t) => Ok(Some(t.times.iter().map(FormulaValue::from_cell).collect())),
        _ => Ok(None),
    }
}

/// Parse and evaluate a formula string against a [`DataSheet`] (and optional [`InfoSheet`]).
pub fn evaluate_formula_for_sheet(formula: &str, sheet: &DataSheet, info_sheet: Option<&InfoSheet>) -> Result<Vec<FormulaValue>, TransformError> {
    let expr = parse_formula(formula)?;
    let ctx = TableEvalContext::new(sheet, info_sheet);
    eval_column_expr(&expr, &ctx)
}

/// Evaluate a parsed [`Expr`] in a scalar (`row_count = 1`) environment.
pub fn eval_scalar_expr(expr: &Expr, ctx: &dyn FormulaContext) -> Result<FormulaValue, TransformError> {
    let vals = eval_column_expr(expr, ctx)?;
    Ok(vals.into_iter().next().unwrap_or(FormulaValue::Missing))
}

fn normalize_col_len(mut vals: Vec<FormulaValue>, n: usize) -> Vec<FormulaValue> {
    vals.resize(n, FormulaValue::Missing);
    vals
}

/// Evaluate a parsed [`Expr`] across every row `0..ctx.row_count()` in `O(N)` vectorized passes.
pub fn eval_column_expr(expr: &Expr, ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    ctx.validate_context()?;
    eval_column_expr_inner(expr, ctx)
}

enum EvalStep<'a> {
    Visit(&'a Expr),
    ApplyUnary(UnaryOp),
    ApplyBinary(BinaryOp),
}

fn eval_column_expr_inner(expr: &Expr, ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    let n = ctx.row_count();
    let mut steps = vec![EvalStep::Visit(expr)];
    let mut stack: Vec<Vec<FormulaValue>> = Vec::new();
    while let Some(step) = steps.pop() {
        match step {
            EvalStep::Visit(node) => match node {
                Expr::Number(v) => stack.push(vec![FormulaValue::from_f64(*v); n]),
                Expr::Text(s) => stack.push(broadcast_formula_value(FormulaValue::Text(s.clone()), n)?),
                Expr::Var(vref) => stack.push(eval_var_ref(vref, ctx)?),
                Expr::Unary { op, expr: inner } => {
                    steps.push(EvalStep::ApplyUnary(*op));
                    steps.push(EvalStep::Visit(inner));
                }
                Expr::Binary { op, left, right } => {
                    steps.push(EvalStep::ApplyBinary(*op));
                    steps.push(EvalStep::Visit(right));
                    steps.push(EvalStep::Visit(left));
                }
                Expr::Call { name, args } => stack.push(eval_windowed_call(name, args, None, None, ctx)?),
                Expr::WindowCall { name, args, filter, window } => {
                    stack.push(eval_windowed_call(name, args, filter.as_deref(), window.as_ref(), ctx)?);
                }
            },
            EvalStep::ApplyUnary(op) => {
                let vals = stack.pop().ok_or_else(|| TransformError::InvalidSpec { message: String::from("formula evaluation stack underflow") })?;
                stack.push(vals.into_iter().map(|v| apply_unary(op, &v)).collect());
            }
            EvalStep::ApplyBinary(op) => {
                let rhs = stack.pop().ok_or_else(|| TransformError::InvalidSpec { message: String::from("formula evaluation stack underflow") })?;
                let lhs = stack.pop().ok_or_else(|| TransformError::InvalidSpec { message: String::from("formula evaluation stack underflow") })?;
                let combined = (0..n)
                    .map(|r| {
                        let a = lhs.get(r).unwrap_or(&FormulaValue::Missing);
                        let b = rhs.get(r).unwrap_or(&FormulaValue::Missing);
                        apply_binary(op, a, b)
                    })
                    .collect();
                stack.push(combined);
            }
        }
    }
    stack.pop().ok_or_else(|| TransformError::InvalidSpec { message: String::from("formula evaluation produced no result") })
}

fn eval_var_ref(vref: &VarRef, ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    let n = ctx.row_count();
    match vref {
        VarRef::RowOneBased => {
            if n == 1
                && let Some(v) = ctx.resolve_scalar("$row")
            {
                return Ok(vec![v]);
            }
            #[allow(clippy::cast_precision_loss)]
            Ok((1..=n).map(|r| FormulaValue::Number(r as f64)).collect())
        }
        VarRef::IndexZeroBased => {
            if n == 1
                && let Some(v) = ctx.resolve_scalar("$index")
            {
                return Ok(vec![v]);
            }
            #[allow(clippy::cast_precision_loss)]
            Ok((0..n).map(|r| FormulaValue::Number(r as f64)).collect())
        }
        VarRef::ColumnByIndex(idx) => {
            ctx.column_values(*idx).map(|v| normalize_col_len(v, n)).ok_or_else(|| TransformError::UnknownVariable { name: format!("col({idx})") })
        }
        VarRef::ColumnByName(name) => {
            if name.trim().eq_ignore_ascii_case("x")
                && let Some(xs) = ctx.resolve_x_column_checked()?
            {
                return Ok(normalize_col_len(xs, n));
            }
            if let Some(idx) = ctx.resolve_column_index(name)
                && let Some(vals) = ctx.column_values(idx)
            {
                return Ok(normalize_col_len(vals, n));
            }
            Err(TransformError::UnknownVariable { name: name.clone() })
        }
        VarRef::InfoConstant(key) => {
            let v = ctx.resolve_info_constant(key).ok_or_else(|| TransformError::UnknownVariable { name: format!("Info(\"{key}\")") })?;
            broadcast_formula_value(v, n)
        }
        VarRef::Bracketed(name) => {
            if let Some(idx) = ctx.resolve_column_index(name)
                && let Some(vals) = ctx.column_values(idx)
            {
                return Ok(normalize_col_len(vals, n));
            }
            if name.trim().eq_ignore_ascii_case("x")
                && let Some(xs) = ctx.resolve_x_column_checked()?
            {
                return Ok(normalize_col_len(xs, n));
            }
            if let Some(v) = ctx.resolve_info_constant(name) {
                return broadcast_formula_value(v, n);
            }
            if let Some(v) = ctx.resolve_scalar(name) {
                return broadcast_formula_value(v, n);
            }
            Err(TransformError::UnknownVariable { name: name.clone() })
        }
        VarRef::Ident(name) => resolve_ident_var(name, ctx),
    }
}

fn resolve_ident_var(name: &str, ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    let n = ctx.row_count();
    let trimmed = name.trim();
    if let Some(v) = ctx.resolve_scalar(trimmed) {
        return broadcast_formula_value(v, n);
    }
    if let Some(c) = reserved_literal(trimmed) {
        return Ok(vec![c; n]);
    }
    if is_math_constant(trimmed) {
        if let Some(idx) = ctx.resolve_column_index_exact(trimmed)
            && let Some(vals) = ctx.column_values(idx)
        {
            return Ok(normalize_col_len(vals, n));
        }
        if let Some(c) = math_constant(trimmed) {
            return Ok(vec![c; n]);
        }
        if let Some(v) = ctx.resolve_info_constant(trimmed) {
            return broadcast_formula_value(v, n);
        }
        return Err(TransformError::UnknownVariable { name: name.to_owned() });
    }
    if trimmed.eq_ignore_ascii_case("x")
        && let Some(xs) = ctx.resolve_x_column_checked()?
    {
        return Ok(normalize_col_len(xs, n));
    }
    if let Some(idx) = ctx.resolve_column_index(trimmed)
        && let Some(vals) = ctx.column_values(idx)
    {
        return Ok(normalize_col_len(vals, n));
    }
    if let Some(v) = ctx.resolve_info_constant(trimmed) {
        return broadcast_formula_value(v, n);
    }
    Err(TransformError::UnknownVariable { name: name.to_owned() })
}

/// Returns `true` only for reserved non-column keyword literals (`true`, `false`, `nan`) so static
/// dependency extraction never skips column names such as `E`, `e`, `pi`, or `tau`.
pub(crate) fn is_builtin_constant(name: &str) -> bool {
    reserved_literal(name.trim()).is_some()
}

pub(crate) fn is_math_constant(name: &str) -> bool {
    math_constant(name).is_some()
}

fn reserved_literal(name: &str) -> Option<FormulaValue> {
    if name.eq_ignore_ascii_case("true") {
        Some(FormulaValue::Number(1.0))
    } else if name.eq_ignore_ascii_case("false") {
        Some(FormulaValue::Number(0.0))
    } else if name.eq_ignore_ascii_case("nan") {
        Some(FormulaValue::Missing)
    } else {
        None
    }
}

fn math_constant(name: &str) -> Option<FormulaValue> {
    if name.eq_ignore_ascii_case("pi") {
        Some(FormulaValue::Number(std::f64::consts::PI))
    } else if name.eq_ignore_ascii_case("e") {
        Some(FormulaValue::Number(std::f64::consts::E))
    } else if name.eq_ignore_ascii_case("tau") {
        Some(FormulaValue::Number(std::f64::consts::TAU))
    } else {
        None
    }
}

fn apply_unary(op: UnaryOp, val: &FormulaValue) -> FormulaValue {
    match op {
        UnaryOp::Pos => val.as_finite_f64().map_or(FormulaValue::Missing, FormulaValue::from_f64),
        UnaryOp::Neg => val.as_finite_f64().map_or(FormulaValue::Missing, |x| FormulaValue::from_f64(-x)),
        UnaryOp::Not => val.as_bool().map_or(FormulaValue::Missing, |b| FormulaValue::Number(if b { 0.0 } else { 1.0 })),
    }
}

fn apply_binary(op: BinaryOp, lhs: &FormulaValue, rhs: &FormulaValue) -> FormulaValue {
    match op {
        // Strong Kleene three-valued logic (Kleene, *Introduction to Metamathematics*, 1952, §64):
        // `false and Missing -> false`, `true or Missing -> true`, otherwise `Missing`.
        BinaryOp::And => match (lhs.as_bool(), rhs.as_bool()) {
            (Some(false), _) | (_, Some(false)) => FormulaValue::Number(0.0),
            (Some(true), Some(true)) => FormulaValue::Number(1.0),
            _ => FormulaValue::Missing,
        },
        BinaryOp::Or => match (lhs.as_bool(), rhs.as_bool()) {
            (Some(true), _) | (_, Some(true)) => FormulaValue::Number(1.0),
            (Some(false), Some(false)) => FormulaValue::Number(0.0),
            _ => FormulaValue::Missing,
        },
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => apply_comparison(op, lhs, rhs),
        _ => {
            let (Some(a), Some(b)) = (lhs.as_finite_f64(), rhs.as_finite_f64()) else {
                return FormulaValue::Missing;
            };
            let out = match op {
                BinaryOp::Add => a + b,
                BinaryOp::Sub => a - b,
                BinaryOp::Mul => a * b,
                BinaryOp::Div => a / b,
                BinaryOp::Mod => a % b,
                BinaryOp::Pow => a.powf(b),
                BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge | BinaryOp::And | BinaryOp::Or => f64::NAN,
            };
            FormulaValue::from_f64(out)
        }
    }
}

fn apply_comparison(op: BinaryOp, lhs: &FormulaValue, rhs: &FormulaValue) -> FormulaValue {
    let ord = match (lhs, rhs) {
        (FormulaValue::Number(a), FormulaValue::Number(b)) if a.is_finite() && b.is_finite() => a.partial_cmp(b),
        (FormulaValue::Text(a), FormulaValue::Text(b)) => Some(a.cmp(b)),
        _ => None,
    };
    let Some(ordering) = ord else {
        return FormulaValue::Missing;
    };
    let flag = match op {
        BinaryOp::Eq => ordering == Ordering::Equal,
        BinaryOp::Ne => ordering != Ordering::Equal,
        BinaryOp::Lt => ordering == Ordering::Less,
        BinaryOp::Le => ordering != Ordering::Greater,
        BinaryOp::Gt => ordering == Ordering::Greater,
        BinaryOp::Ge => ordering != Ordering::Less,
        _ => false,
    };
    FormulaValue::Number(if flag { 1.0 } else { 0.0 })
}

fn eval_windowed_call(
    name: &str,
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Vec<FormulaValue>, TransformError> {
    let lower = name.to_ascii_lowercase();
    let has_clauses = filter.is_some() || window.is_some();

    if !has_clauses && let Some(res) = try_eval_special_call(&lower, args, ctx)? {
        return Ok(res);
    }
    if let Some(res) = try_eval_aggregate_call(&lower, args, filter, window, ctx)? {
        return Ok(res);
    }
    if let Some(res) = try_eval_window_call(&lower, args, filter, window, ctx)? {
        return Ok(res);
    }
    if has_clauses {
        return Err(TransformError::InvalidSpec { message: format!("function `{lower}` does not support `OVER` or `WHERE`/`FILTER` clauses") });
    }
    eval_scalar_call(&lower, args, ctx)
}

fn check_arity(name: &str, args: &[Expr], expected: usize) -> Result<(), TransformError> {
    if args.len() != expected {
        return Err(TransformError::ArityMismatch { function: name.to_owned(), expected: expected.to_string(), actual: args.len() });
    }
    Ok(())
}

fn check_min_arity(name: &str, args: &[Expr], min: usize) -> Result<(), TransformError> {
    if args.len() < min {
        return Err(TransformError::ArityMismatch { function: name.to_owned(), expected: format!("at least {min}"), actual: args.len() });
    }
    Ok(())
}

fn try_eval_special_call(lower: &str, args: &[Expr], ctx: &dyn FormulaContext) -> Result<Option<Vec<FormulaValue>>, TransformError> {
    let n = ctx.row_count();
    match lower {
        "if" => {
            let [cond_expr, t_expr, f_expr] = args else {
                return Err(TransformError::ArityMismatch { function: String::from("if"), expected: String::from("3"), actual: args.len() });
            };
            let cond = eval_column_expr_inner(cond_expr, ctx)?;
            let t_vals = eval_column_expr_inner(t_expr, ctx)?;
            let f_vals = eval_column_expr_inner(f_expr, ctx)?;
            let mut out = Vec::with_capacity(n);
            for r in 0..n {
                let val = match cond.get(r).and_then(FormulaValue::as_bool) {
                    Some(true) => t_vals.get(r).cloned().unwrap_or(FormulaValue::Missing),
                    Some(false) => f_vals.get(r).cloned().unwrap_or(FormulaValue::Missing),
                    None => FormulaValue::Missing,
                };
                out.push(val);
            }
            Ok(Some(out))
        }
        "isnan" => {
            let [arg0] = args else {
                return Err(TransformError::ArityMismatch { function: String::from("isnan"), expected: String::from("1"), actual: args.len() });
            };
            let vals = eval_column_expr_inner(arg0, ctx)?;
            Ok(Some(vals.into_iter().map(|v| FormulaValue::Number(if v.as_finite_f64().is_none() { 1.0 } else { 0.0 })).collect()))
        }
        "isvalid" | "is_defined" => {
            let [arg0] = args else {
                return Err(TransformError::ArityMismatch { function: lower.to_owned(), expected: String::from("1"), actual: args.len() });
            };
            let vals = eval_column_expr_inner(arg0, ctx)?;
            Ok(Some(vals.into_iter().map(|v| FormulaValue::Number(if v.is_valid() { 1.0 } else { 0.0 })).collect()))
        }
        "coalesce" => {
            check_min_arity("coalesce", args, 1)?;
            let mut out = vec![FormulaValue::Missing; n];
            for arg in args {
                let col = eval_column_expr_inner(arg, ctx)?;
                for (slot, val) in out.iter_mut().zip(col) {
                    if !slot.is_valid() && val.is_valid() {
                        *slot = val;
                    }
                }
            }
            Ok(Some(out))
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PartitionKeyPart {
    Number(u64),
    Text(String),
    Missing,
}

impl PartitionKeyPart {
    fn from_value(v: &FormulaValue) -> Self {
        match v {
            FormulaValue::Number(x) if x.is_finite() => {
                let norm = if *x == 0.0 { 0.0 } else { *x };
                Self::Number(norm.to_bits())
            }
            FormulaValue::Text(s) => Self::Text(s.clone()),
            _ => Self::Missing,
        }
    }
}

struct OrderKeyColumn {
    values: Vec<FormulaValue>,
    direction: SortDirection,
    nulls: NullsOrder,
}

fn eval_where_mask(filter: Option<&Expr>, n: usize, ctx: &dyn FormulaContext) -> Result<Option<Vec<bool>>, TransformError> {
    let Some(fexpr) = filter else {
        return Ok(None);
    };
    let vals = eval_column_expr_inner(fexpr, ctx)?;
    Ok(Some((0..n).map(|r| vals.get(r).and_then(FormulaValue::as_bool) == Some(true)).collect()))
}

fn build_partitions(
    partition_by: &[Expr],
    where_mask: Option<&[bool]>,
    n: usize,
    ctx: &dyn FormulaContext,
) -> Result<Vec<Vec<usize>>, TransformError> {
    if partition_by.is_empty() {
        let rows: Vec<usize> = match where_mask {
            Some(mask) => (0..n).filter(|&r| mask.get(r).copied().unwrap_or(false)).collect(),
            None => (0..n).collect(),
        };
        return Ok(if rows.is_empty() && n == 0 { Vec::new() } else { vec![rows] });
    }

    let key_cols: Vec<Vec<FormulaValue>> = partition_by.iter().map(|e| eval_column_expr_inner(e, ctx)).collect::<Result<_, _>>()?;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    if key_cols.len() == 1 {
        let col0 = &key_cols[0];
        let mut index_by_key: HashMap<PartitionKeyPart, usize> = HashMap::new();
        for r in 0..n {
            if let Some(mask) = where_mask
                && !mask.get(r).copied().unwrap_or(false)
            {
                continue;
            }
            let key = PartitionKeyPart::from_value(col0.get(r).unwrap_or(&FormulaValue::Missing));
            if let Some(&gid) = index_by_key.get(&key) {
                groups[gid].push(r);
            } else {
                let gid = groups.len();
                index_by_key.insert(key, gid);
                groups.push(vec![r]);
            }
        }
    } else {
        let mut index_by_key: HashMap<Vec<PartitionKeyPart>, usize> = HashMap::new();
        for r in 0..n {
            if let Some(mask) = where_mask
                && !mask.get(r).copied().unwrap_or(false)
            {
                continue;
            }
            let key: Vec<PartitionKeyPart> =
                key_cols.iter().map(|col| PartitionKeyPart::from_value(col.get(r).unwrap_or(&FormulaValue::Missing))).collect();
            if let Some(&gid) = index_by_key.get(&key) {
                groups[gid].push(r);
            } else {
                let gid = groups.len();
                index_by_key.insert(key, gid);
                groups.push(vec![r]);
            }
        }
    }
    Ok(groups)
}

fn eval_order_columns(order_by: &[OrderByExpr], ctx: &dyn FormulaContext) -> Result<Vec<OrderKeyColumn>, TransformError> {
    order_by
        .iter()
        .map(|o| {
            let values = eval_column_expr_inner(&o.expr, ctx)?;
            Ok(OrderKeyColumn { values, direction: o.direction, nulls: o.nulls })
        })
        .collect()
}

fn cmp_formula_value_for_order(a: &FormulaValue, b: &FormulaValue, dir: SortDirection, nulls: NullsOrder) -> Ordering {
    let a_missing = !a.is_valid();
    let b_missing = !b.is_valid();
    match (a_missing, b_missing) {
        (true, true) => return Ordering::Equal,
        (true, false) => return if nulls == NullsOrder::First { Ordering::Less } else { Ordering::Greater },
        (false, true) => return if nulls == NullsOrder::First { Ordering::Greater } else { Ordering::Less },
        (false, false) => {}
    }
    let base = match (a, b) {
        (FormulaValue::Number(x), FormulaValue::Number(y)) => {
            let xn = if *x == 0.0 { 0.0 } else { *x };
            let yn = if *y == 0.0 { 0.0 } else { *y };
            xn.total_cmp(&yn)
        }
        (FormulaValue::Text(x), FormulaValue::Text(y)) => x.cmp(y),
        (FormulaValue::Number(_), FormulaValue::Text(_)) => Ordering::Less,
        (FormulaValue::Text(_), FormulaValue::Number(_)) => Ordering::Greater,
        _ => Ordering::Equal,
    };
    match dir {
        SortDirection::Asc => base,
        SortDirection::Desc => base.reverse(),
    }
}

fn cmp_row_order(r_a: usize, r_b: usize, order_cols: &[OrderKeyColumn]) -> Ordering {
    for col in order_cols {
        let va = col.values.get(r_a).unwrap_or(&FormulaValue::Missing);
        let vb = col.values.get(r_b).unwrap_or(&FormulaValue::Missing);
        let ord = cmp_formula_value_for_order(va, vb, col.direction, col.nulls);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn sort_partitions(partitions: &mut [Vec<usize>], order_cols: &[OrderKeyColumn]) {
    if order_cols.is_empty() {
        return;
    }
    for part in partitions {
        part.sort_by(|&a, &b| cmp_row_order(a, b, order_cols));
    }
}

#[derive(Debug, Clone, Copy)]
enum ResolvedBound {
    UnboundedPreceding,
    Preceding(usize),
    CurrentRow,
    Following(usize),
    UnboundedFollowing,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedFrame {
    start: ResolvedBound,
    end: ResolvedBound,
}

impl ResolvedFrame {
    fn resolve(frame: &WindowFrame, ctx: &dyn FormulaContext) -> Result<Self, TransformError> {
        let start = resolve_bound(&frame.start, ctx)?;
        let end = resolve_bound(&frame.end, ctx)?;
        validate_resolved_frame(start, end)?;
        Ok(Self { start, end })
    }

    fn bounds_for(self, pos: usize, part_len: usize) -> Option<(usize, usize)> {
        if part_len == 0 || pos >= part_len {
            return None;
        }
        let max_idx = (part_len - 1) as isize;
        let p = pos as isize;
        let lo_raw = match self.start {
            ResolvedBound::UnboundedPreceding => 0,
            ResolvedBound::Preceding(k) => p.saturating_sub(isize::try_from(k).unwrap_or(isize::MAX)),
            ResolvedBound::CurrentRow => p,
            ResolvedBound::Following(k) => p.saturating_add(isize::try_from(k).unwrap_or(isize::MAX)),
            ResolvedBound::UnboundedFollowing => max_idx,
        };
        let hi_raw = match self.end {
            ResolvedBound::UnboundedPreceding => 0,
            ResolvedBound::Preceding(k) => p.saturating_sub(isize::try_from(k).unwrap_or(isize::MAX)),
            ResolvedBound::CurrentRow => p,
            ResolvedBound::Following(k) => p.saturating_add(isize::try_from(k).unwrap_or(isize::MAX)),
            ResolvedBound::UnboundedFollowing => max_idx,
        };
        if hi_raw < 0 || lo_raw > max_idx || lo_raw > hi_raw {
            return None;
        }
        #[allow(clippy::cast_sign_loss)]
        let lo = lo_raw.max(0) as usize;
        #[allow(clippy::cast_sign_loss)]
        let hi = hi_raw.min(max_idx) as usize;
        (lo <= hi).then_some((lo, hi))
    }
}

fn resolve_bound(bound: &FrameBound, ctx: &dyn FormulaContext) -> Result<ResolvedBound, TransformError> {
    match bound {
        FrameBound::UnboundedPreceding => Ok(ResolvedBound::UnboundedPreceding),
        FrameBound::CurrentRow => Ok(ResolvedBound::CurrentRow),
        FrameBound::UnboundedFollowing => Ok(ResolvedBound::UnboundedFollowing),
        FrameBound::Preceding(expr) => Ok(ResolvedBound::Preceding(eval_nonneg_int_arg(expr, ctx, "ROWS PRECEDING")?)),
        FrameBound::Following(expr) => Ok(ResolvedBound::Following(eval_nonneg_int_arg(expr, ctx, "ROWS FOLLOWING")?)),
    }
}

fn validate_resolved_frame(start: ResolvedBound, end: ResolvedBound) -> Result<(), TransformError> {
    match (start, end) {
        (ResolvedBound::UnboundedFollowing, _) => {
            Err(TransformError::InvalidSpec { message: String::from("`UNBOUNDED FOLLOWING` cannot be used as `ROWS` frame start") })
        }
        (_, ResolvedBound::UnboundedPreceding) => {
            Err(TransformError::InvalidSpec { message: String::from("`UNBOUNDED PRECEDING` cannot be used as `ROWS` frame end") })
        }
        (ResolvedBound::CurrentRow, ResolvedBound::Preceding(_)) => {
            Err(TransformError::InvalidSpec { message: String::from("`ROWS` frame start `CURRENT ROW` cannot follow `PRECEDING`") })
        }
        (ResolvedBound::Following(_), ResolvedBound::Preceding(_) | ResolvedBound::CurrentRow) => {
            Err(TransformError::InvalidSpec { message: String::from("`ROWS` frame start `FOLLOWING` cannot precede `PRECEDING` or `CURRENT ROW`") })
        }
        (ResolvedBound::Preceding(a), ResolvedBound::Preceding(b)) if a < b => {
            Err(TransformError::InvalidSpec { message: format!("invalid `ROWS` frame: `{a} PRECEDING` cannot follow `{b} PRECEDING`") })
        }
        (ResolvedBound::Following(a), ResolvedBound::Following(b)) if a > b => {
            Err(TransformError::InvalidSpec { message: format!("invalid `ROWS` frame: `{a} FOLLOWING` cannot precede `{b} FOLLOWING`") })
        }
        _ => Ok(()),
    }
}

fn compute_peer_ends(part: &[usize], order_cols: &[OrderKeyColumn]) -> Vec<usize> {
    let m = part.len();
    let mut ends = vec![0_usize; m];
    let mut i = 0;
    while i < m {
        let mut j = i + 1;
        while j < m && cmp_row_order(part[i], part[j], order_cols) == Ordering::Equal {
            j += 1;
        }
        let end = j - 1;
        for slot in ends.iter_mut().take(j).skip(i) {
            *slot = end;
        }
        i = j;
    }
    ends
}

fn is_condition_ast(expr: &Expr) -> bool {
    match expr {
        Expr::Binary { op, .. } => {
            matches!(op, BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge | BinaryOp::And | BinaryOp::Or)
        }
        Expr::Unary { op: UnaryOp::Not, .. } => true,
        Expr::Var(VarRef::Ident(name)) => {
            let t = name.trim();
            t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false")
        }
        Expr::Call { name, .. } | Expr::WindowCall { name, .. } => {
            let lower = name.to_ascii_lowercase();
            matches!(lower.as_str(), "not" | "and" | "or" | "isnan" | "isvalid" | "is_defined")
        }
        _ => false,
    }
}

/// Resolve a 2-argument conditional aggregate `(arg0, arg1)` into `(condition_values, value_column)`.
///
/// Enforces the documented canonical `(values, condition)` order regardless of evaluated cell
/// contents, while preserving backwards compatibility only when `arg0` is syntactically an explicit
/// boolean/comparison expression and `arg1` is not (legacy `(condition, values)` syntax).
fn resolve_cond_val_pair(arg0: &Expr, arg1: &Expr, ctx: &dyn FormulaContext) -> Result<(Vec<FormulaValue>, Vec<FormulaValue>), TransformError> {
    let v0 = eval_column_expr_inner(arg0, ctx)?;
    let v1 = eval_column_expr_inner(arg1, ctx)?;
    let a0_cond = is_condition_ast(arg0);
    let a1_cond = is_condition_ast(arg1);
    if a0_cond && !a1_cond { Ok((v0, v1)) } else { Ok((v1, v0)) }
}

enum AggOp {
    Sum,
    Count,
    Mean,
    Sd,
    Sem,
    Var,
    Median,
    Iqr,
    MinCol,
    MaxCol,
    GeoMean,
    Percentile(f64),
    CountIf { has_value_expr: bool },
    CountAIf,
    SumIf,
    MeanIf,
    MinIf,
    MaxIf,
    SdIf,
    SemIf,
}

impl AggOp {
    fn empty_frame_value(&self) -> FormulaValue {
        match self {
            Self::Count | Self::CountIf { .. } | Self::CountAIf => FormulaValue::Number(0.0),
            _ => FormulaValue::Missing,
        }
    }
}

fn linear_percentile(sorted: &[f64], p_percent: f64) -> FormulaValue {
    if !p_percent.is_finite() || !(0.0..=100.0).contains(&p_percent) {
        return FormulaValue::Missing;
    }
    curvebook_math::quantile_type7_sorted(sorted, p_percent / 100.0).map_or(FormulaValue::Missing, FormulaValue::from_f64)
}

fn compute_agg_over_rows(
    op: &AggOp,
    rows: &[usize],
    where_mask: Option<&[bool]>,
    vals: &[FormulaValue],
    conds: Option<&[FormulaValue]>,
) -> FormulaValue {
    let row_active = |r: usize| -> bool { where_mask.is_none_or(|m| m.get(r).copied().unwrap_or(false)) };
    match op {
        AggOp::Count => {
            let cnt = rows.iter().filter(|&&r| row_active(r) && vals.get(r).is_some_and(FormulaValue::is_valid)).count();
            #[allow(clippy::cast_precision_loss)]
            FormulaValue::Number(cnt as f64)
        }
        AggOp::CountIf { has_value_expr } => {
            let cnt = rows
                .iter()
                .filter(|&&r| {
                    row_active(r)
                        && conds.and_then(|c| c.get(r)).and_then(FormulaValue::as_bool) == Some(true)
                        && (!has_value_expr || vals.get(r).and_then(FormulaValue::as_finite_f64).is_some())
                })
                .count();
            #[allow(clippy::cast_precision_loss)]
            FormulaValue::Number(cnt as f64)
        }
        AggOp::CountAIf => {
            let cnt = rows
                .iter()
                .filter(|&&r| {
                    row_active(r)
                        && conds.and_then(|c| c.get(r)).and_then(FormulaValue::as_bool) == Some(true)
                        && vals.get(r).is_some_and(FormulaValue::is_valid)
                })
                .count();
            #[allow(clippy::cast_precision_loss)]
            FormulaValue::Number(cnt as f64)
        }
        _ => {
            let is_cond = matches!(op, AggOp::SumIf | AggOp::MeanIf | AggOp::MinIf | AggOp::MaxIf | AggOp::SdIf | AggOp::SemIf);
            let mut nums: Vec<f64> = Vec::new();
            for &r in rows {
                if !row_active(r) {
                    continue;
                }
                if is_cond && conds.and_then(|c| c.get(r)).and_then(FormulaValue::as_bool) != Some(true) {
                    continue;
                }
                if let Some(x) = vals.get(r).and_then(FormulaValue::as_finite_f64) {
                    nums.push(x);
                }
            }
            match op {
                AggOp::Sum | AggOp::SumIf => FormulaValue::from_f64(sum::sum(&nums)),
                AggOp::Mean | AggOp::MeanIf => sum::mean(&nums).map_or(FormulaValue::Missing, FormulaValue::from_f64),
                AggOp::Sd | AggOp::SdIf => sum::sample_sd(&nums).map_or(FormulaValue::Missing, FormulaValue::from_f64),
                AggOp::Sem | AggOp::SemIf => {
                    sum::SummaryMoments::from_slice(&nums).and_then(|m| m.sem()).map_or(FormulaValue::Missing, FormulaValue::from_f64)
                }
                AggOp::Var => sum::sample_variance(&nums).map_or(FormulaValue::Missing, FormulaValue::from_f64),
                AggOp::MinCol | AggOp::MinIf => nums.into_iter().reduce(f64::min).map_or(FormulaValue::Missing, FormulaValue::from_f64),
                AggOp::MaxCol | AggOp::MaxIf => nums.into_iter().reduce(f64::max).map_or(FormulaValue::Missing, FormulaValue::from_f64),
                AggOp::GeoMean => {
                    if nums.is_empty() || nums.iter().any(|&v| v <= 0.0) {
                        FormulaValue::Missing
                    } else {
                        let logs: Vec<f64> = nums.iter().map(|v| v.ln()).collect();
                        FormulaValue::from_f64((sum::sum(&logs) / (logs.len() as f64)).exp())
                    }
                }
                AggOp::Median => {
                    nums.sort_by(f64::total_cmp);
                    linear_percentile(&nums, 50.0)
                }
                AggOp::Iqr => {
                    nums.sort_by(f64::total_cmp);
                    match (linear_percentile(&nums, 75.0).as_finite_f64(), linear_percentile(&nums, 25.0).as_finite_f64()) {
                        (Some(q3), Some(q1)) => FormulaValue::from_f64(q3 - q1),
                        _ => FormulaValue::Missing,
                    }
                }
                AggOp::Percentile(p_pct) => {
                    nums.sort_by(f64::total_cmp);
                    linear_percentile(&nums, *p_pct)
                }
                AggOp::Count | AggOp::CountIf { .. } | AggOp::CountAIf => FormulaValue::Missing,
            }
        }
    }
}

fn try_eval_aggregate_call(
    lower: &str,
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Option<Vec<FormulaValue>>, TransformError> {
    let n = ctx.row_count();
    let has_clauses = filter.is_some() || window.is_some();

    let (op, vals, conds): (AggOp, Vec<FormulaValue>, Option<Vec<FormulaValue>>) = match lower {
        "sum" => {
            if args.len() != 1 {
                if has_clauses {
                    return Err(TransformError::ArityMismatch { function: String::from("sum"), expected: String::from("1"), actual: args.len() });
                }
                return Ok(None);
            }
            (AggOp::Sum, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "mean" => {
            if args.len() != 1 {
                if has_clauses {
                    return Err(TransformError::ArityMismatch { function: String::from("mean"), expected: String::from("1"), actual: args.len() });
                }
                return Ok(None);
            }
            (AggOp::Mean, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "count" | "counta" => {
            check_arity(lower, args, 1)?;
            (AggOp::Count, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "sd" | "stdev" => {
            check_arity(lower, args, 1)?;
            (AggOp::Sd, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "sem" | "stderr" => {
            check_arity(lower, args, 1)?;
            (AggOp::Sem, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "var" => {
            check_arity("var", args, 1)?;
            (AggOp::Var, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "median" => {
            check_arity("median", args, 1)?;
            (AggOp::Median, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "iqr" => {
            check_arity("iqr", args, 1)?;
            (AggOp::Iqr, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "min_col" | "col_min" => {
            check_arity(lower, args, 1)?;
            (AggOp::MinCol, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "max_col" | "col_max" => {
            check_arity(lower, args, 1)?;
            (AggOp::MaxCol, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "geo_mean" => {
            check_arity("geo_mean", args, 1)?;
            (AggOp::GeoMean, eval_column_expr_inner(&args[0], ctx)?, None)
        }
        "percentile" | "quantile" => {
            check_arity(lower, args, 2)?;
            let vals = eval_column_expr_inner(&args[0], ctx)?;
            let p_val = eval_scalar_const_arg(&args[1], ctx)?;
            let p_pct = if lower == "quantile" { p_val * 100.0 } else { p_val };
            (AggOp::Percentile(p_pct), vals, None)
        }
        "countif" => {
            if args.len() == 1 {
                let cond_vals = eval_column_expr_inner(&args[0], ctx)?;
                (AggOp::CountIf { has_value_expr: false }, Vec::new(), Some(cond_vals))
            } else if args.len() == 2 {
                if let Expr::Text(lit) = &args[1]
                    && !is_condition_ast(&args[0])
                {
                    let val_col = eval_column_expr_inner(&args[0], ctx)?;
                    let cond_vals: Vec<FormulaValue> = val_col
                        .iter()
                        .map(|v| match v {
                            FormulaValue::Text(s) => FormulaValue::Number(if s == lit { 1.0 } else { 0.0 }),
                            _ => FormulaValue::Number(0.0),
                        })
                        .collect();
                    (AggOp::CountIf { has_value_expr: false }, Vec::new(), Some(cond_vals))
                } else {
                    let (cond_vals, val_col) = resolve_cond_val_pair(&args[0], &args[1], ctx)?;
                    (AggOp::CountIf { has_value_expr: true }, val_col, Some(cond_vals))
                }
            } else {
                return Err(TransformError::ArityMismatch {
                    function: String::from("countif"),
                    expected: String::from("1 or 2"),
                    actual: args.len(),
                });
            }
        }
        "countaif" => {
            check_arity("countaif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(&args[0], &args[1], ctx)?;
            (AggOp::CountAIf, val_col, Some(cond_vals))
        }
        "sumif" => {
            check_arity("sumif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(&args[0], &args[1], ctx)?;
            (AggOp::SumIf, val_col, Some(cond_vals))
        }
        "meanif" => {
            check_arity("meanif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(&args[0], &args[1], ctx)?;
            (AggOp::MeanIf, val_col, Some(cond_vals))
        }
        "minif" => {
            check_arity("minif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(&args[0], &args[1], ctx)?;
            (AggOp::MinIf, val_col, Some(cond_vals))
        }
        "maxif" => {
            check_arity("maxif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(&args[0], &args[1], ctx)?;
            (AggOp::MaxIf, val_col, Some(cond_vals))
        }
        "stdevif" | "sdif" => {
            check_arity(lower, args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(&args[0], &args[1], ctx)?;
            (AggOp::SdIf, val_col, Some(cond_vals))
        }
        "stderrif" | "semif" => {
            check_arity(lower, args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(&args[0], &args[1], ctx)?;
            (AggOp::SemIf, val_col, Some(cond_vals))
        }
        _ => return Ok(None),
    };

    let where_mask = eval_where_mask(filter, n, ctx)?;
    let Some(wspec) = window else {
        let all_rows: Vec<usize> = (0..n).collect();
        let val = compute_agg_over_rows(&op, &all_rows, where_mask.as_deref(), &vals, conds.as_deref());
        return Ok(Some(broadcast_formula_value(val, n)?));
    };

    let mut partitions = build_partitions(&wspec.partition_by, None, n, ctx)?;
    let order_cols = eval_order_columns(&wspec.order_by, ctx)?;
    sort_partitions(&mut partitions, &order_cols);
    let resolved_frame = wspec.frame.as_ref().map(|f| ResolvedFrame::resolve(f, ctx)).transpose()?;

    let mut out = vec![FormulaValue::Missing; n];
    for part in &partitions {
        if resolved_frame.is_none() && order_cols.is_empty() {
            let agg_val = compute_agg_over_rows(&op, part, where_mask.as_deref(), &vals, conds.as_deref());
            for &r in part {
                if let Some(slot) = out.get_mut(r) {
                    *slot = agg_val.clone();
                }
            }
            continue;
        }
        let peer_ends = if resolved_frame.is_none() && !order_cols.is_empty() { Some(compute_peer_ends(part, &order_cols)) } else { None };
        eval_framed_agg_partition(&op, part, resolved_frame, peer_ends.as_deref(), where_mask.as_deref(), &vals, conds.as_deref(), &mut out);
    }
    Ok(Some(out))
}

/// Amortized `O(1)`-per-step two-stack sliding-window aggregator for any associative semigroup.
///
/// Maintains monotonic window ranges `[lo, hi_excl)` without subtraction, avoiding catastrophic
/// cancellation and `Inf - Inf` overflow while bounding total work to `<= 3 * m` combines (`O(m)`).
struct SwagWindow<T: Copy, F: Fn(T, T) -> T> {
    front: Vec<(T, T)>,
    back: Vec<(T, T)>,
    combine: F,
}

impl<T: Copy, F: Fn(T, T) -> T> SwagWindow<T, F> {
    fn new(combine: F) -> Self {
        Self { front: Vec::new(), back: Vec::new(), combine }
    }

    fn clear(&mut self) {
        self.front.clear();
        self.back.clear();
    }

    fn push(&mut self, val: T) {
        let agg = match self.back.last() {
            Some(&(_, prev)) => (self.combine)(prev, val),
            None => val,
        };
        self.back.push((val, agg));
    }

    fn pop(&mut self) {
        if self.front.is_empty() {
            while let Some((v, _)) = self.back.pop() {
                let agg = match self.front.last() {
                    Some(&(_, prev)) => (self.combine)(v, prev),
                    None => v,
                };
                self.front.push((v, agg));
            }
        }
        let _ = self.front.pop();
    }

    fn query(&self) -> Option<T> {
        match (self.front.last(), self.back.last()) {
            (Some(&(_, f)), Some(&(_, b))) => Some((self.combine)(f, b)),
            (Some(&(_, f)), None) => Some(f),
            (None, Some(&(_, b))) => Some(b),
            (None, None) => None,
        }
    }

    fn advance_to<G: Fn(usize) -> T>(&mut self, cur_lo: &mut usize, cur_hi: &mut usize, lo: usize, hi_excl: usize, elem: G) {
        if *cur_hi <= lo {
            self.clear();
            *cur_lo = lo;
            *cur_hi = lo;
        }
        while *cur_hi < hi_excl {
            self.push(elem(*cur_hi));
            *cur_hi += 1;
        }
        while *cur_lo < lo {
            self.pop();
            *cur_lo += 1;
        }
    }
}

/// Associative parallel summary moments (Chan, Golub, & LeVeque 1979) with Neumaier sum
/// and exact power-of-two `M2` rescaling so neither `mean([1e308, 1e308])` nor `sd([-1e160, 1e160])` overflows.
#[derive(Clone, Copy)]
struct MomentsAgg {
    count: usize,
    sum: sum::NeumaierSum,
    mean: f64,
    scale: f64,
    m2_scaled: f64,
}

impl MomentsAgg {
    const fn empty() -> Self {
        Self { count: 0, sum: sum::NeumaierSum::new(), mean: 0.0, scale: 0.0, m2_scaled: 0.0 }
    }

    fn single(x: f64) -> Self {
        let mut s = sum::NeumaierSum::new();
        s.add(x);
        Self { count: 1, sum: s, mean: x, scale: 0.0, m2_scaled: 0.0 }
    }

    fn combine(a: Self, b: Self) -> Self {
        if a.count == 0 {
            return b;
        }
        if b.count == 0 {
            return a;
        }
        let count = a.count + b.count;
        let mut s = a.sum;
        s.add(b.sum.total());
        #[allow(clippy::cast_precision_loss)]
        let na = a.count as f64;
        #[allow(clippy::cast_precision_loss)]
        let nb = b.count as f64;
        #[allow(clippy::cast_precision_loss)]
        let n_f = count as f64;
        let total = s.total();
        let mean = if total.is_finite() { total / n_f } else { (na / n_f) * a.mean + (nb / n_f) * b.mean };

        let raw_delta = b.mean - a.mean;
        let ref_mag = a.scale.max(b.scale).max(if raw_delta.is_finite() { raw_delta.abs() } else { a.mean.abs().max(b.mean.abs()) });
        if ref_mag == 0.0 {
            return Self { count, sum: s, mean, scale: 0.0, m2_scaled: 0.0 };
        }
        let new_scale = if ref_mag > 1e130 || ref_mag < 1e-140 {
            #[allow(clippy::cast_possible_truncation)]
            let shift = (ref_mag.log2().floor() as i32).clamp(-1000, 1000);
            2.0_f64.powi(shift)
        } else {
            a.scale.max(b.scale).max(1.0)
        };
        let fa = a.scale / new_scale;
        let fb = b.scale / new_scale;
        let delta_s = if raw_delta.is_finite() { raw_delta / new_scale } else { (b.mean / new_scale) - (a.mean / new_scale) };
        let weight = na * (nb / n_f);
        let m2_scaled = a.m2_scaled * (fa * fa) + b.m2_scaled * (fb * fb) + delta_s * delta_s * weight;
        Self { count, sum: s, mean, scale: new_scale, m2_scaled }
    }

    fn sum_val(self, empty_as_zero: bool) -> FormulaValue {
        if self.count == 0 {
            return if empty_as_zero { FormulaValue::Number(0.0) } else { FormulaValue::Missing };
        }
        FormulaValue::from_f64(self.sum.total())
    }

    fn mean_val(self) -> FormulaValue {
        if self.count == 0 { FormulaValue::Missing } else { FormulaValue::from_f64(self.mean) }
    }

    fn sd_val(self) -> FormulaValue {
        if self.count < 2 {
            return FormulaValue::Missing;
        }
        #[allow(clippy::cast_precision_loss)]
        let denom = (self.count - 1) as f64;
        FormulaValue::from_f64(self.scale * (self.m2_scaled / denom).max(0.0).sqrt())
    }

    fn sem_val(self) -> FormulaValue {
        if self.count < 2 {
            return FormulaValue::Missing;
        }
        #[allow(clippy::cast_precision_loss)]
        let denom = ((self.count - 1) as f64) * (self.count as f64);
        FormulaValue::from_f64(self.scale * (self.m2_scaled / denom).max(0.0).sqrt())
    }

    fn var_val(self) -> FormulaValue {
        if self.count < 2 {
            return FormulaValue::Missing;
        }
        #[allow(clippy::cast_precision_loss)]
        let denom = (self.count - 1) as f64;
        let v = (self.m2_scaled / denom).max(0.0);
        FormulaValue::from_f64((v * self.scale) * self.scale)
    }
}

#[derive(Clone, Copy)]
struct GeoMeanAgg {
    count: usize,
    has_non_pos: bool,
    log_sum: sum::NeumaierSum,
}

impl GeoMeanAgg {
    const fn empty() -> Self {
        Self { count: 0, has_non_pos: false, log_sum: sum::NeumaierSum::new() }
    }

    fn single(x: f64) -> Self {
        if x <= 0.0 {
            Self { count: 1, has_non_pos: true, log_sum: sum::NeumaierSum::new() }
        } else {
            let mut s = sum::NeumaierSum::new();
            s.add(x.ln());
            Self { count: 1, has_non_pos: false, log_sum: s }
        }
    }

    fn combine(a: Self, b: Self) -> Self {
        let mut s = a.log_sum;
        s.add(b.log_sum.total());
        Self { count: a.count + b.count, has_non_pos: a.has_non_pos || b.has_non_pos, log_sum: s }
    }

    fn finish(self) -> FormulaValue {
        if self.count == 0 || self.has_non_pos {
            return FormulaValue::Missing;
        }
        #[allow(clippy::cast_precision_loss)]
        let mean_ln = self.log_sum.total() / (self.count as f64);
        FormulaValue::from_f64(mean_ln.exp())
    }
}

/// Coordinate-compressed Fenwick tree supporting `O(log m)` multiset insert/remove and Type-7
/// quantile queries (`Median`, `Iqr`, `Percentile`) over sliding/cumulative window frames.
struct OrderStatWindow {
    unique: Vec<f64>,
    bit: Vec<usize>,
    count: usize,
    highest_pow2: usize,
}

impl OrderStatWindow {
    fn from_values(active: &[Option<f64>]) -> (Self, Vec<Option<usize>>) {
        let mut unique: Vec<f64> = active.iter().copied().flatten().collect();
        unique.sort_by(f64::total_cmp);
        unique.dedup();
        let ranks: Vec<Option<usize>> = active.iter().map(|opt| opt.map(|x| unique.partition_point(|&v| v < x))).collect();
        let len = unique.len();
        let highest_pow2 = if len == 0 { 0 } else { 1_usize << (usize::BITS - 1 - len.leading_zeros()) };
        let bit = vec![0_usize; len + 1];
        (Self { unique, bit, count: 0, highest_pow2 }, ranks)
    }

    fn insert(&mut self, rank: usize) {
        self.count += 1;
        let mut i = rank + 1;
        while i < self.bit.len() {
            self.bit[i] += 1;
            i += i.isolate_lowest_one();
        }
    }

    fn remove(&mut self, rank: usize) {
        self.count = self.count.saturating_sub(1);
        let mut i = rank + 1;
        while i < self.bit.len() {
            self.bit[i] = self.bit[i].saturating_sub(1);
            i += i.isolate_lowest_one();
        }
    }

    fn kth(&self, k: usize) -> Option<f64> {
        if k >= self.count || self.unique.is_empty() {
            return None;
        }
        let mut idx = 0_usize;
        let mut rem = k;
        let mut step = self.highest_pow2;
        while step > 0 {
            let next = idx + step;
            if next < self.bit.len() && self.bit[next] <= rem {
                rem -= self.bit[next];
                idx = next;
            }
            step >>= 1;
        }
        self.unique.get(idx).copied()
    }

    fn percentile(&self, p_percent: f64) -> FormulaValue {
        if self.count == 0 || !p_percent.is_finite() || !(0.0..=100.0).contains(&p_percent) {
            return FormulaValue::Missing;
        }
        if self.count == 1 {
            return self.kth(0).map_or(FormulaValue::Missing, FormulaValue::from_f64);
        }
        let pc = (p_percent / 100.0).clamp(0.0, 1.0);
        #[allow(clippy::cast_precision_loss)]
        let idx = pc * ((self.count - 1) as f64);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lo = (idx.floor() as usize).min(self.count - 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let hi = (idx.ceil() as usize).min(self.count - 1);
        let Some(lo_v) = self.kth(lo) else {
            return FormulaValue::Missing;
        };
        if lo == hi {
            return FormulaValue::from_f64(lo_v);
        }
        let Some(hi_v) = self.kth(hi) else {
            return FormulaValue::Missing;
        };
        #[allow(clippy::cast_precision_loss)]
        let frac = idx - (lo as f64);
        let diff = hi_v - lo_v;
        let out = if diff.is_finite() { lo_v + frac * diff } else { (1.0 - frac) * lo_v + frac * hi_v };
        FormulaValue::from_f64(out)
    }
}

fn frame_range_for_pos(pos: usize, part_len: usize, resolved_frame: Option<ResolvedFrame>, peer_ends: Option<&[usize]>) -> Option<(usize, usize)> {
    if let Some(frame) = resolved_frame {
        frame.bounds_for(pos, part_len)
    } else if let Some(ends) = peer_ends {
        let end = ends.get(pos).copied().unwrap_or(pos);
        Some((0, end))
    } else if part_len > 0 {
        Some((0, part_len - 1))
    } else {
        None
    }
}

fn active_numeric_for_agg(
    op: &AggOp,
    part: &[usize],
    where_mask: Option<&[bool]>,
    vals: &[FormulaValue],
    conds: Option<&[FormulaValue]>,
) -> Vec<Option<f64>> {
    let is_cond = matches!(op, AggOp::SumIf | AggOp::MeanIf | AggOp::MinIf | AggOp::MaxIf | AggOp::SdIf | AggOp::SemIf);
    part.iter()
        .map(|&r| {
            if where_mask.is_some_and(|m| !m.get(r).copied().unwrap_or(false)) {
                return None;
            }
            if is_cond && conds.and_then(|c| c.get(r)).and_then(FormulaValue::as_bool) != Some(true) {
                return None;
            }
            vals.get(r).and_then(FormulaValue::as_finite_f64)
        })
        .collect()
}

fn active_count_flags(op: &AggOp, part: &[usize], where_mask: Option<&[bool]>, vals: &[FormulaValue], conds: Option<&[FormulaValue]>) -> Vec<usize> {
    part.iter()
        .map(|&r| {
            if where_mask.is_some_and(|m| !m.get(r).copied().unwrap_or(false)) {
                return 0;
            }
            let keep = match op {
                AggOp::Count => vals.get(r).is_some_and(FormulaValue::is_valid),
                AggOp::CountIf { has_value_expr } => {
                    conds.and_then(|c| c.get(r)).and_then(FormulaValue::as_bool) == Some(true)
                        && (!has_value_expr || vals.get(r).and_then(FormulaValue::as_finite_f64).is_some())
                }
                AggOp::CountAIf => {
                    conds.and_then(|c| c.get(r)).and_then(FormulaValue::as_bool) == Some(true) && vals.get(r).is_some_and(FormulaValue::is_valid)
                }
                _ => false,
            };
            usize::from(keep)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn eval_framed_agg_partition(
    op: &AggOp,
    part: &[usize],
    resolved_frame: Option<ResolvedFrame>,
    peer_ends: Option<&[usize]>,
    where_mask: Option<&[bool]>,
    vals: &[FormulaValue],
    conds: Option<&[FormulaValue]>,
    out: &mut [FormulaValue],
) {
    let m = part.len();
    if m == 0 {
        return;
    }
    match op {
        AggOp::Count | AggOp::CountIf { .. } | AggOp::CountAIf => {
            let flags = active_count_flags(op, part, where_mask, vals, conds);
            let mut prefix = Vec::with_capacity(m + 1);
            prefix.push(0_usize);
            let mut acc = 0_usize;
            for f in flags {
                acc += f;
                prefix.push(acc);
            }
            for (pos, &r) in part.iter().enumerate() {
                let agg_val = match frame_range_for_pos(pos, m, resolved_frame, peer_ends) {
                    Some((lo, hi)) =>
                    {
                        #[allow(clippy::cast_precision_loss)]
                        FormulaValue::Number((prefix[hi + 1] - prefix[lo]) as f64)
                    }
                    None => op.empty_frame_value(),
                };
                if let Some(slot) = out.get_mut(r) {
                    *slot = agg_val;
                }
            }
        }
        AggOp::MinCol | AggOp::MinIf | AggOp::MaxCol | AggOp::MaxIf => {
            let is_min = matches!(op, AggOp::MinCol | AggOp::MinIf);
            let active = active_numeric_for_agg(op, part, where_mask, vals, conds);
            let mut swag = SwagWindow::new(|a: Option<f64>, b: Option<f64>| match (a, b) {
                (Some(x), Some(y)) => Some(if is_min { x.min(y) } else { x.max(y) }),
                (Some(x), None) | (None, Some(x)) => Some(x),
                (None, None) => None,
            });
            let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
            for (pos, &r) in part.iter().enumerate() {
                let agg_val = match frame_range_for_pos(pos, m, resolved_frame, peer_ends) {
                    Some((lo, hi)) => {
                        swag.advance_to(&mut cur_lo, &mut cur_hi, lo, hi + 1, |i| active[i]);
                        swag.query().flatten().map_or(FormulaValue::Missing, FormulaValue::from_f64)
                    }
                    None => op.empty_frame_value(),
                };
                if let Some(slot) = out.get_mut(r) {
                    *slot = agg_val;
                }
            }
        }
        AggOp::GeoMean => {
            let active = active_numeric_for_agg(op, part, where_mask, vals, conds);
            let mut swag = SwagWindow::new(GeoMeanAgg::combine);
            let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
            for (pos, &r) in part.iter().enumerate() {
                let agg_val = match frame_range_for_pos(pos, m, resolved_frame, peer_ends) {
                    Some((lo, hi)) => {
                        swag.advance_to(&mut cur_lo, &mut cur_hi, lo, hi + 1, |i| active[i].map_or(GeoMeanAgg::empty(), GeoMeanAgg::single));
                        swag.query().map_or(FormulaValue::Missing, GeoMeanAgg::finish)
                    }
                    None => op.empty_frame_value(),
                };
                if let Some(slot) = out.get_mut(r) {
                    *slot = agg_val;
                }
            }
        }
        AggOp::Median | AggOp::Iqr | AggOp::Percentile(_) => {
            let active = active_numeric_for_agg(op, part, where_mask, vals, conds);
            let (mut tree, ranks) = OrderStatWindow::from_values(&active);
            let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
            for (pos, &r) in part.iter().enumerate() {
                let agg_val = match frame_range_for_pos(pos, m, resolved_frame, peer_ends) {
                    Some((lo, hi)) => {
                        let hi_excl = hi + 1;
                        if cur_hi <= lo {
                            for rk in ranks[cur_lo..cur_hi].iter().flatten() {
                                tree.remove(*rk);
                            }
                            cur_lo = lo;
                            cur_hi = lo;
                        }
                        while cur_hi < hi_excl {
                            if let Some(rk) = ranks[cur_hi] {
                                tree.insert(rk);
                            }
                            cur_hi += 1;
                        }
                        while cur_lo < lo {
                            if let Some(rk) = ranks[cur_lo] {
                                tree.remove(rk);
                            }
                            cur_lo += 1;
                        }
                        match op {
                            AggOp::Median => tree.percentile(50.0),
                            AggOp::Percentile(p) => tree.percentile(*p),
                            AggOp::Iqr => match (tree.percentile(75.0).as_finite_f64(), tree.percentile(25.0).as_finite_f64()) {
                                (Some(q3), Some(q1)) => FormulaValue::from_f64(q3 - q1),
                                _ => FormulaValue::Missing,
                            },
                            _ => FormulaValue::Missing,
                        }
                    }
                    None => op.empty_frame_value(),
                };
                if let Some(slot) = out.get_mut(r) {
                    *slot = agg_val;
                }
            }
        }
        AggOp::Sum | AggOp::SumIf | AggOp::Mean | AggOp::MeanIf | AggOp::Sd | AggOp::SdIf | AggOp::Sem | AggOp::SemIf | AggOp::Var => {
            let active = active_numeric_for_agg(op, part, where_mask, vals, conds);
            let mut swag = SwagWindow::new(MomentsAgg::combine);
            let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
            for (pos, &r) in part.iter().enumerate() {
                let agg_val = match frame_range_for_pos(pos, m, resolved_frame, peer_ends) {
                    Some((lo, hi)) => {
                        swag.advance_to(&mut cur_lo, &mut cur_hi, lo, hi + 1, |i| active[i].map_or(MomentsAgg::empty(), MomentsAgg::single));
                        let agg = swag.query().unwrap_or_else(MomentsAgg::empty);
                        match op {
                            AggOp::Sum | AggOp::SumIf => agg.sum_val(true),
                            AggOp::Mean | AggOp::MeanIf => agg.mean_val(),
                            AggOp::Sd | AggOp::SdIf => agg.sd_val(),
                            AggOp::Sem | AggOp::SemIf => agg.sem_val(),
                            AggOp::Var => agg.var_val(),
                            _ => FormulaValue::Missing,
                        }
                    }
                    None => op.empty_frame_value(),
                };
                if let Some(slot) = out.get_mut(r) {
                    *slot = agg_val;
                }
            }
        }
    }
}

struct ScalarOnlyContext<'a> {
    inner: &'a dyn FormulaContext,
}

impl FormulaContext for ScalarOnlyContext<'_> {
    fn row_count(&self) -> usize {
        1
    }

    fn column_count(&self) -> usize {
        0
    }

    fn resolve_column_index(&self, _name: &str) -> Option<usize> {
        None
    }

    fn column_values(&self, _idx: usize) -> Option<Vec<FormulaValue>> {
        None
    }

    fn resolve_x_column(&self) -> Option<Vec<FormulaValue>> {
        None
    }

    fn resolve_info_constant(&self, key: &str) -> Option<FormulaValue> {
        self.inner.resolve_info_constant(key)
    }

    fn resolve_scalar(&self, name: &str) -> Option<FormulaValue> {
        self.inner.resolve_scalar(name)
    }
}

fn is_scalar_disallowed_column_ident(name: &str, ctx: &dyn FormulaContext) -> bool {
    let trimmed = name.trim();
    if reserved_literal(trimmed).is_some() {
        return false;
    }
    if is_math_constant(trimmed) {
        return ctx.resolve_column_index_exact(trimmed).is_some();
    }
    ctx.resolve_scalar(trimmed).is_none()
        && (ctx.resolve_column_index(trimmed).is_some() || (trimmed.eq_ignore_ascii_case("x") && ctx.resolve_x_column().is_some()))
}

fn is_window_or_series_function_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "row_number"
            | "rank"
            | "dense_rank"
            | "lag"
            | "lead"
            | "diff"
            | "cumsum"
            | "cumprod"
            | "cummean"
            | "center"
            | "standardize"
            | "zscore"
            | "rolling_mean"
            | "moving_avg"
            | "rolling_sum"
    )
}

fn ensure_scalar_only_expr(expr: &Expr, ctx: &dyn FormulaContext) -> Result<(), TransformError> {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Number(_) | Expr::Text(_) => {}
            Expr::Unary { expr: inner, .. } => stack.push(inner),
            Expr::Binary { left, right, .. } => {
                stack.push(left);
                stack.push(right);
            }
            Expr::Call { name, args } => {
                if is_window_or_series_function_name(name) {
                    return Err(TransformError::InvalidSpec { message: format!("function `{name}` is not allowed in scalar parameter expression") });
                }
                for arg in args {
                    stack.push(arg);
                }
            }
            Expr::WindowCall { name, .. } => {
                return Err(TransformError::InvalidSpec {
                    message: format!("window expression `{name}` is not allowed in scalar parameter expression"),
                });
            }
            Expr::Var(VarRef::InfoConstant(_)) => {}
            Expr::Var(VarRef::RowOneBased | VarRef::IndexZeroBased) => {
                return Err(TransformError::InvalidSpec {
                    message: String::from("row index variable is not allowed in scalar parameter expression"),
                });
            }
            Expr::Var(VarRef::ColumnByIndex(_) | VarRef::ColumnByName(_) | VarRef::Bracketed(_)) => {
                return Err(TransformError::InvalidSpec { message: String::from("column reference is not allowed in scalar parameter expression") });
            }
            Expr::Var(VarRef::Ident(name)) => {
                if is_scalar_disallowed_column_ident(name, ctx) {
                    return Err(TransformError::InvalidSpec { message: format!("column `{name}` is not allowed in scalar parameter expression") });
                }
            }
        }
    }
    Ok(())
}

fn eval_scalar_const_arg(expr: &Expr, ctx: &dyn FormulaContext) -> Result<f64, TransformError> {
    ensure_scalar_only_expr(expr, ctx)?;
    let scalar_ctx = ScalarOnlyContext { inner: ctx };
    let vals = eval_column_expr_inner(expr, &scalar_ctx)?;
    vals.first()
        .and_then(FormulaValue::as_finite_f64)
        .ok_or_else(|| TransformError::EvalError { message: String::from("expected finite numeric parameter") })
}

fn eval_nonneg_int_arg(expr: &Expr, ctx: &dyn FormulaContext, fn_name: &str) -> Result<usize, TransformError> {
    let v = eval_scalar_const_arg(expr, ctx)?;
    if v < 0.0 || v.fract() != 0.0 || v > (usize::MAX as f64) {
        return Err(TransformError::InvalidSpec { message: format!("`{fn_name}` requires a non-negative integer argument, got {v}") });
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(v as usize)
}

fn var_ref_name(v: &VarRef) -> Option<&str> {
    match v {
        VarRef::Ident(s) | VarRef::Bracketed(s) | VarRef::ColumnByName(s) => Some(s.trim()),
        _ => None,
    }
}

fn expr_matches_primary_order(expr: &Expr, order_by: &[OrderByExpr]) -> Option<SortDirection> {
    let ord = order_by.first()?;
    if &ord.expr == expr {
        return Some(ord.direction);
    }
    if let (Expr::Var(va), Expr::Var(vb)) = (expr, &ord.expr)
        && let (Some(na), Some(nb)) = (var_ref_name(va), var_ref_name(vb))
        && na.eq_ignore_ascii_case(nb)
    {
        return Some(ord.direction);
    }
    None
}

struct PreparedPartitions {
    partitions: Vec<Vec<usize>>,
    order_cols: Vec<OrderKeyColumn>,
}

fn prepare_filtered_partitions(
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    n: usize,
    ctx: &dyn FormulaContext,
) -> Result<PreparedPartitions, TransformError> {
    let where_mask = eval_where_mask(filter, n, ctx)?;
    let partition_by = window.map_or(&[][..], |w| w.partition_by.as_slice());
    let order_by = window.map_or(&[][..], |w| w.order_by.as_slice());
    let mut partitions = build_partitions(partition_by, where_mask.as_deref(), n, ctx)?;
    let order_cols = eval_order_columns(order_by, ctx)?;
    sort_partitions(&mut partitions, &order_cols);
    Ok(PreparedPartitions { partitions, order_cols })
}

fn try_eval_window_call(
    lower: &str,
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Option<Vec<FormulaValue>>, TransformError> {
    match lower {
        "row_number" => eval_window_row_number(args, filter, window, ctx).map(Some),
        "rank" | "dense_rank" => eval_window_rank(lower, args, filter, window, ctx).map(Some),
        "center" | "standardize" | "zscore" => eval_window_standardize(lower, args, filter, window, ctx).map(Some),
        "cumsum" | "cumprod" | "cummean" => eval_window_cumulative(lower, args, filter, window, ctx).map(Some),
        "lag" | "lead" => eval_window_shift(lower, args, filter, window, ctx).map(Some),
        "diff" => eval_window_diff(args, filter, window, ctx).map(Some),
        "rolling_mean" | "moving_avg" | "rolling_sum" => eval_window_rolling(lower, args, filter, window, ctx).map(Some),
        _ => Ok(None),
    }
}

fn eval_window_row_number(
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Vec<FormulaValue>, TransformError> {
    check_arity("row_number", args, 0)?;
    let n = ctx.row_count();
    let prep = prepare_filtered_partitions(filter, window, n, ctx)?;
    let mut out = vec![FormulaValue::Missing; n];
    for part in &prep.partitions {
        for (pos, &r) in part.iter().enumerate() {
            if let Some(slot) = out.get_mut(r) {
                #[allow(clippy::cast_precision_loss)]
                {
                    *slot = FormulaValue::Number((pos + 1) as f64);
                }
            }
        }
    }
    Ok(out)
}

/// Evaluate `rank(...)` and `dense_rank(...)` in both statistical 1-argument and SQL 0-argument modes:
///
/// - **1-argument `rank(expr)` / `dense_rank(expr)`**: ranks all non-missing values (both numeric
///   and text) within each partition, honoring the primary `ORDER BY` key direction when `order_by.first()`
///   matches `expr` (even when secondary tie-breaker keys follow). `rank(expr)` assigns statistical
///   average midranks (`1.5, 1.5, 3`) and `dense_rank(expr)` assigns consecutive dense ranks (`1, 1, 2`);
///   `Missing` inputs propagate as `FormulaValue::Missing`.
/// - **0-argument `rank() OVER (ORDER BY ...)` / `dense_rank() OVER (ORDER BY ...)`**: evaluates
///   standard SQL competition ranking (`1, 1, 3`) / dense ranking (`1, 1, 2`) across the full
///   `ORDER BY` tuple, honoring explicit `NULLS FIRST` / `NULLS LAST` ordering on every key.
fn eval_window_rank(
    lower: &str,
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Vec<FormulaValue>, TransformError> {
    let dense = lower == "dense_rank";
    if args.len() > 1 {
        return Err(TransformError::ArityMismatch { function: lower.to_owned(), expected: String::from("0 or 1"), actual: args.len() });
    }
    let order_by_slice = window.map_or(&[][..], |w| w.order_by.as_slice());
    if args.is_empty() && order_by_slice.is_empty() {
        return Err(TransformError::InvalidSpec { message: format!("`{lower}()` with 0 arguments requires an `OVER (... ORDER BY ...)` clause") });
    }
    let vals = if let Some(arg0) = args.first() { Some(eval_column_expr_inner(arg0, ctx)?) } else { None };
    let n = ctx.row_count();
    let prep = prepare_filtered_partitions(filter, window, n, ctx)?;
    let mut out = vec![FormulaValue::Missing; n];
    if let (Some(val_vec), Some(arg0)) = (vals, args.first()) {
        let desc = expr_matches_primary_order(arg0, order_by_slice) == Some(SortDirection::Desc);
        for part in &prep.partitions {
            let part_vals: Vec<FormulaValue> = part.iter().map(|&r| val_vec.get(r).cloned().unwrap_or(FormulaValue::Missing)).collect();
            let ranked = eval_rank_directed(&part_vals, dense, desc);
            for (&r, rval) in part.iter().zip(ranked) {
                if let Some(slot) = out.get_mut(r) {
                    *slot = rval;
                }
            }
        }
    } else {
        for part in &prep.partitions {
            eval_order_rank_into(part, &prep.order_cols, dense, &mut out);
        }
    }
    Ok(out)
}

fn frame_finite_nums(part: &[usize], lo: usize, hi: usize, vals: &[FormulaValue]) -> Vec<f64> {
    part.get(lo..=hi).unwrap_or(&[]).iter().filter_map(|&idx| vals.get(idx).and_then(FormulaValue::as_finite_f64)).collect()
}

fn eval_window_standardize(
    lower: &str,
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Vec<FormulaValue>, TransformError> {
    check_arity(lower, args, 1)?;
    let standardize = lower != "center";
    let vals = eval_column_expr_inner(&args[0], ctx)?;
    let n = ctx.row_count();
    let prep = prepare_filtered_partitions(filter, window, n, ctx)?;
    let resolved_frame = window.and_then(|w| w.frame.as_ref()).map(|f| ResolvedFrame::resolve(f, ctx)).transpose()?;
    let mut out = vec![FormulaValue::Missing; n];

    for part in &prep.partitions {
        if let Some(frame) = resolved_frame {
            let mut swag = SwagWindow::new(MomentsAgg::combine);
            let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
            for (pos, &r) in part.iter().enumerate() {
                let Some((lo, hi)) = frame.bounds_for(pos, part.len()) else {
                    continue;
                };
                swag.advance_to(&mut cur_lo, &mut cur_hi, lo, hi + 1, |i| {
                    part.get(i).and_then(|&idx| vals.get(idx)).and_then(FormulaValue::as_finite_f64).map_or(MomentsAgg::empty(), MomentsAgg::single)
                });
                let Some(x) = vals.get(r).and_then(FormulaValue::as_finite_f64) else {
                    continue;
                };
                let agg = swag.query().unwrap_or_else(MomentsAgg::empty);
                if let Some(slot) = out.get_mut(r) {
                    *slot = compute_center_or_z_from_agg(x, agg, standardize);
                }
            }
        } else {
            let nums = frame_finite_nums(part, 0, part.len().saturating_sub(1), &vals);
            for &r in part {
                if let Some(x) = vals.get(r).and_then(FormulaValue::as_finite_f64)
                    && let Some(slot) = out.get_mut(r)
                {
                    *slot = compute_center_or_z(x, &nums, standardize);
                }
            }
        }
    }
    Ok(out)
}

fn eval_window_cumulative(
    lower: &str,
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Vec<FormulaValue>, TransformError> {
    check_arity(lower, args, 1)?;
    let vals = eval_column_expr_inner(&args[0], ctx)?;
    let n = ctx.row_count();
    let prep = prepare_filtered_partitions(filter, window, n, ctx)?;
    let resolved_frame = window.and_then(|w| w.frame.as_ref()).map(|f| ResolvedFrame::resolve(f, ctx)).transpose()?;
    let mut out = vec![FormulaValue::Missing; n];

    for part in &prep.partitions {
        if let Some(frame) = resolved_frame {
            eval_framed_cumulative_partition(lower, part, frame, &vals, &mut out);
        } else {
            eval_cum_partition(lower, part, &vals, &mut out);
        }
    }
    Ok(out)
}

fn eval_framed_cumulative_partition(lower: &str, part: &[usize], frame: ResolvedFrame, vals: &[FormulaValue], out: &mut [FormulaValue]) {
    if lower == "cumprod" {
        let mut swag = SwagWindow::new(|a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(x), Some(y)) => Some(x * y),
            (Some(x), None) | (None, Some(x)) => Some(x),
            (None, None) => None,
        });
        let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
        for (pos, &r) in part.iter().enumerate() {
            let Some((lo, hi)) = frame.bounds_for(pos, part.len()) else {
                continue;
            };
            swag.advance_to(&mut cur_lo, &mut cur_hi, lo, hi + 1, |i| {
                part.get(i).and_then(|&idx| vals.get(idx)).and_then(FormulaValue::as_finite_f64)
            });
            if let Some(prod) = swag.query().flatten()
                && let Some(slot) = out.get_mut(r)
            {
                *slot = FormulaValue::from_f64(prod);
            }
        }
        return;
    }

    let as_mean = lower == "cummean";
    let mut swag = SwagWindow::new(MomentsAgg::combine);
    let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
    for (pos, &r) in part.iter().enumerate() {
        let Some((lo, hi)) = frame.bounds_for(pos, part.len()) else {
            continue;
        };
        swag.advance_to(&mut cur_lo, &mut cur_hi, lo, hi + 1, |i| {
            part.get(i).and_then(|&idx| vals.get(idx)).and_then(FormulaValue::as_finite_f64).map_or(MomentsAgg::empty(), MomentsAgg::single)
        });
        let agg = swag.query().unwrap_or_else(MomentsAgg::empty);
        if agg.count > 0
            && let Some(slot) = out.get_mut(r)
        {
            *slot = if as_mean { agg.mean_val() } else { agg.sum_val(false) };
        }
    }
}

fn eval_shift_or_diff_k(args: &[Expr], ctx: &dyn FormulaContext, fn_name: &str) -> Result<usize, TransformError> {
    if args.is_empty() || args.len() > 2 {
        return Err(TransformError::ArityMismatch { function: fn_name.to_owned(), expected: String::from("1 or 2"), actual: args.len() });
    }
    if let Some(arg1) = args.get(1) { eval_nonneg_int_arg(arg1, ctx, fn_name) } else { Ok(1) }
}

fn eval_window_shift(
    lower: &str,
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Vec<FormulaValue>, TransformError> {
    let k = eval_shift_or_diff_k(args, ctx, lower)?;
    let vals = eval_column_expr_inner(&args[0], ctx)?;
    let n = ctx.row_count();
    let prep = prepare_filtered_partitions(filter, window, n, ctx)?;
    let mut out = vec![FormulaValue::Missing; n];
    let is_lag = lower == "lag";
    for part in &prep.partitions {
        let m = part.len();
        for (pos, &r) in part.iter().enumerate() {
            let src_pos = if is_lag { pos.checked_sub(k) } else { pos.checked_add(k).filter(|&idx| idx < m) };
            let val = src_pos.and_then(|sp| part.get(sp)).and_then(|&sr| vals.get(sr)).cloned().unwrap_or(FormulaValue::Missing);
            if let Some(slot) = out.get_mut(r) {
                *slot = val;
            }
        }
    }
    Ok(out)
}

fn eval_window_diff(
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Vec<FormulaValue>, TransformError> {
    let k = eval_shift_or_diff_k(args, ctx, "diff")?;
    let vals = eval_column_expr_inner(&args[0], ctx)?;
    let n = ctx.row_count();
    let prep = prepare_filtered_partitions(filter, window, n, ctx)?;
    let mut out = vec![FormulaValue::Missing; n];
    for part in &prep.partitions {
        for (pos, &r) in part.iter().enumerate() {
            let Some(prev_r) = pos.checked_sub(k).and_then(|pp| part.get(pp)).copied() else {
                continue;
            };
            if let (Some(cur), Some(prev)) =
                (vals.get(r).and_then(FormulaValue::as_finite_f64), vals.get(prev_r).and_then(FormulaValue::as_finite_f64))
                && let Some(slot) = out.get_mut(r)
            {
                *slot = FormulaValue::from_f64(cur - prev);
            }
        }
    }
    Ok(out)
}

fn eval_window_rolling(
    lower: &str,
    args: &[Expr],
    filter: Option<&Expr>,
    window: Option<&WindowSpec>,
    ctx: &dyn FormulaContext,
) -> Result<Vec<FormulaValue>, TransformError> {
    let resolved_frame = window.and_then(|w| w.frame.as_ref()).map(|f| ResolvedFrame::resolve(f, ctx)).transpose()?;
    let as_mean = lower != "rolling_sum";
    let w = if args.len() == 2 {
        let width = eval_nonneg_int_arg(&args[1], ctx, lower)?;
        if width == 0 {
            return Err(TransformError::InvalidSpec { message: format!("`{lower}` window width must be >= 1") });
        }
        Some(width)
    } else if args.len() == 1 && resolved_frame.is_some() {
        None
    } else {
        return Err(TransformError::ArityMismatch { function: lower.to_owned(), expected: String::from("2"), actual: args.len() });
    };
    let vals = eval_column_expr_inner(&args[0], ctx)?;
    let n = ctx.row_count();
    let prep = prepare_filtered_partitions(filter, window, n, ctx)?;
    let mut out = vec![FormulaValue::Missing; n];

    for part in &prep.partitions {
        if let Some(frame) = resolved_frame {
            let mut swag = SwagWindow::new(MomentsAgg::combine);
            let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
            for (pos, &r) in part.iter().enumerate() {
                let Some((lo, hi)) = frame.bounds_for(pos, part.len()) else {
                    continue;
                };
                swag.advance_to(&mut cur_lo, &mut cur_hi, lo, hi + 1, |i| {
                    part.get(i).and_then(|&idx| vals.get(idx)).and_then(FormulaValue::as_finite_f64).map_or(MomentsAgg::empty(), MomentsAgg::single)
                });
                let agg = swag.query().unwrap_or_else(MomentsAgg::empty);
                if agg.count > 0
                    && let Some(slot) = out.get_mut(r)
                {
                    *slot = if as_mean { agg.mean_val() } else { agg.sum_val(false) };
                }
            }
        } else if let Some(width) = w {
            let part_vals: Vec<FormulaValue> = part.iter().map(|&r| vals.get(r).cloned().unwrap_or(FormulaValue::Missing)).collect();
            let rolled = eval_rolling(&part_vals, width, as_mean);
            for (&r, val) in part.iter().zip(rolled) {
                if let Some(slot) = out.get_mut(r) {
                    *slot = val;
                }
            }
        }
    }
    Ok(out)
}

fn compute_center_or_z(x: f64, nums: &[f64], standardize: bool) -> FormulaValue {
    let Ok(moments) = sum::SummaryMoments::from_slice(nums) else {
        return FormulaValue::Missing;
    };
    if !standardize {
        return FormulaValue::from_f64(x - moments.mean);
    }
    let Ok(sd) = moments.sample_sd() else {
        return FormulaValue::Missing;
    };
    if sd == 0.0 || !sd.is_finite() { FormulaValue::Missing } else { FormulaValue::from_f64((x - moments.mean) / sd) }
}

fn compute_center_or_z_from_agg(x: f64, agg: MomentsAgg, standardize: bool) -> FormulaValue {
    if agg.count == 0 || !agg.mean.is_finite() {
        return FormulaValue::Missing;
    }
    if !standardize {
        return FormulaValue::from_f64(x - agg.mean);
    }
    let Some(sd) = agg.sd_val().as_finite_f64() else {
        return FormulaValue::Missing;
    };
    if sd == 0.0 { FormulaValue::Missing } else { FormulaValue::from_f64((x - agg.mean) / sd) }
}

fn eval_cum_partition(lower: &str, part: &[usize], vals: &[FormulaValue], out: &mut [FormulaValue]) {
    let mut acc = sum::NeumaierSum::new();
    let mut mean_est = 0.0_f64;
    let mut prod = 1.0_f64;
    let mut count = 0_usize;
    for &r in part {
        let Some(x) = vals.get(r).and_then(FormulaValue::as_finite_f64) else {
            continue;
        };
        let res = match lower {
            "cumsum" => {
                acc.add(x);
                FormulaValue::from_f64(acc.total())
            }
            "cumprod" => {
                prod *= x;
                FormulaValue::from_f64(prod)
            }
            _ => {
                acc.add(x);
                count += 1;
                #[allow(clippy::cast_precision_loss)]
                let c = count as f64;
                mean_est += (x / c) - (mean_est / c);
                let total = acc.total();
                FormulaValue::from_f64(if total.is_finite() { total / c } else { mean_est })
            }
        };
        if let Some(slot) = out.get_mut(r) {
            *slot = res;
        }
    }
}

fn eval_order_rank_into(part: &[usize], order_cols: &[OrderKeyColumn], dense: bool, out: &mut [FormulaValue]) {
    let m = part.len();
    let mut pos = 0;
    let mut dense_level = 1_usize;
    while pos < m {
        let mut end = pos + 1;
        while end < m && cmp_row_order(part[pos], part[end], order_cols) == Ordering::Equal {
            end += 1;
        }
        #[allow(clippy::cast_precision_loss)]
        let rank_val = if dense { dense_level as f64 } else { (pos + 1) as f64 };
        for &r in &part[pos..end] {
            if let Some(slot) = out.get_mut(r) {
                *slot = FormulaValue::Number(rank_val);
            }
        }
        pos = end;
        dense_level += 1;
    }
}

fn eval_rolling(vals: &[FormulaValue], window: usize, as_mean: bool) -> Vec<FormulaValue> {
    let n = vals.len();
    let mut out = Vec::with_capacity(n);
    let mut swag = SwagWindow::new(MomentsAgg::combine);
    let (mut cur_lo, mut cur_hi) = (0_usize, 0_usize);
    for i in 0..n {
        if i + 1 < window {
            out.push(FormulaValue::Missing);
            continue;
        }
        let start = i + 1 - window;
        swag.advance_to(&mut cur_lo, &mut cur_hi, start, i + 1, |idx| {
            vals.get(idx).and_then(FormulaValue::as_finite_f64).map_or(MomentsAgg::empty(), MomentsAgg::single)
        });
        let agg = swag.query().unwrap_or_else(MomentsAgg::empty);
        if agg.count == 0 {
            out.push(FormulaValue::Missing);
        } else {
            out.push(if as_mean { agg.mean_val() } else { agg.sum_val(false) });
        }
    }
    out
}

fn eval_rank_directed(vals: &[FormulaValue], dense: bool, descending: bool) -> Vec<FormulaValue> {
    let dir = if descending { SortDirection::Desc } else { SortDirection::Asc };
    let mut indexed: Vec<(usize, &FormulaValue)> = vals.iter().enumerate().filter(|(_, v)| v.is_valid()).collect();
    indexed.sort_by(|a, b| cmp_formula_value_for_order(a.1, b.1, dir, NullsOrder::Last));
    let mut out = vec![FormulaValue::Missing; vals.len()];
    let mut pos = 0;
    let mut dense_level = 1_usize;
    while let Some(&(_, pos_val)) = indexed.get(pos) {
        let mut end = pos + 1;
        while indexed.get(end).is_some_and(|&(_, end_val)| cmp_formula_value_for_order(pos_val, end_val, dir, NullsOrder::Last) == Ordering::Equal) {
            end += 1;
        }
        #[allow(clippy::cast_precision_loss)]
        let rank_val = if dense { dense_level as f64 } else { ((pos + 1 + end) as f64) * 0.5 };
        for &(orig_idx, _) in indexed.get(pos..end).unwrap_or(&[]) {
            if let Some(slot) = out.get_mut(orig_idx) {
                *slot = FormulaValue::Number(rank_val);
            }
        }
        pos = end;
        dense_level += 1;
    }
    out
}

fn eval_scalar_call(lower: &str, args: &[Expr], ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    validate_scalar_call_arity(lower, args.len())?;
    match lower {
        "min" | "max" => return eval_streaming_minmax(args, ctx, lower == "min"),
        "sum" | "mean" => return eval_streaming_sum_or_mean(args, ctx, lower == "mean"),
        "and" | "or" => return eval_streaming_logic(args, ctx, lower == "and"),
        _ => {}
    }

    let n = ctx.row_count();
    let evaled: Vec<Vec<FormulaValue>> = args.iter().map(|a| eval_column_expr_inner(a, ctx)).collect::<Result<_, _>>()?;
    let mut out = Vec::with_capacity(n);
    for row in 0..n {
        let val = eval_scalar_row(lower, &evaled, row)?;
        out.push(val);
    }
    Ok(out)
}

fn eval_streaming_minmax(args: &[Expr], ctx: &dyn FormulaContext, is_min: bool) -> Result<Vec<FormulaValue>, TransformError> {
    let n = ctx.row_count();
    let mut acc: Vec<Option<f64>> = vec![None; n];
    for arg in args {
        let col = eval_column_expr_inner(arg, ctx)?;
        for (slot, val) in acc.iter_mut().zip(col) {
            if let Some(v) = val.as_finite_f64() {
                *slot = Some(match *slot {
                    None => v,
                    Some(cur) if is_min => cur.min(v),
                    Some(cur) => cur.max(v),
                });
            }
        }
    }
    Ok(acc.into_iter().map(|opt| opt.map_or(FormulaValue::Missing, FormulaValue::from_f64)).collect())
}

#[derive(Clone, Copy)]
struct StreamingMean {
    sum: sum::NeumaierSum,
    mean_est: f64,
    count: usize,
}

impl StreamingMean {
    const fn new() -> Self {
        Self { sum: sum::NeumaierSum::new(), mean_est: 0.0, count: 0 }
    }

    fn add(&mut self, v: f64) {
        self.count += 1;
        self.sum.add(v);
        #[allow(clippy::cast_precision_loss)]
        let c = self.count as f64;
        self.mean_est += (v / c) - (self.mean_est / c);
    }

    fn finish(self, is_mean: bool) -> FormulaValue {
        if self.count == 0 {
            return FormulaValue::Missing;
        }
        let total = self.sum.total();
        if !is_mean {
            return FormulaValue::from_f64(total);
        }
        #[allow(clippy::cast_precision_loss)]
        let c = self.count as f64;
        FormulaValue::from_f64(if total.is_finite() { total / c } else { self.mean_est })
    }
}

fn eval_streaming_sum_or_mean(args: &[Expr], ctx: &dyn FormulaContext, is_mean: bool) -> Result<Vec<FormulaValue>, TransformError> {
    let n = ctx.row_count();
    let mut accs = vec![StreamingMean::new(); n];
    for arg in args {
        let col = eval_column_expr_inner(arg, ctx)?;
        for (acc, val) in accs.iter_mut().zip(col) {
            if let Some(v) = val.as_finite_f64() {
                acc.add(v);
            }
        }
    }
    Ok(accs.into_iter().map(|acc| acc.finish(is_mean)).collect())
}

fn eval_streaming_logic(args: &[Expr], ctx: &dyn FormulaContext, is_and: bool) -> Result<Vec<FormulaValue>, TransformError> {
    // 0 = neutral (true for AND, false for OR), 1 = saw Missing, 2 = absorbing (false for AND, true for OR)
    let n = ctx.row_count();
    let mut states = vec![0_u8; n];
    for arg in args {
        let col = eval_column_expr_inner(arg, ctx)?;
        for (state, val) in states.iter_mut().zip(col) {
            if *state == 2 {
                continue;
            }
            match val.as_bool() {
                Some(false) if is_and => *state = 2,
                Some(true) if !is_and => *state = 2,
                Some(_) => {}
                None => *state = 1,
            }
        }
    }
    Ok(states
        .into_iter()
        .map(|s| match s {
            1 => FormulaValue::Missing,
            2 => FormulaValue::Number(if is_and { 0.0 } else { 1.0 }),
            _ => FormulaValue::Number(if is_and { 1.0 } else { 0.0 }),
        })
        .collect())
}

fn validate_scalar_call_arity(lower: &str, argc: usize) -> Result<(), TransformError> {
    match lower {
        "abs" | "sqrt" | "cbrt" | "sqr" | "ln" | "log10" | "log2" | "exp" | "exp10" | "sin" | "cos" | "tan" | "asin" | "arcsin" | "acos"
        | "arccos" | "atan" | "arctan" | "sinh" | "cosh" | "tanh" | "floor" | "ceil" | "trunc" | "int" | "sign" | "sgn" | "erf" | "erfc"
        | "lgamma" | "logit" | "probit" | "norm_ppf" | "norm_inv" | "qnorm" | "inv_norm" | "arcsin_sqrt" | "not" => {
            if argc != 1 {
                return Err(TransformError::ArityMismatch { function: lower.to_owned(), expected: String::from("1"), actual: argc });
            }
        }
        "log" | "round" => {
            if argc != 1 && argc != 2 {
                return Err(TransformError::ArityMismatch { function: lower.to_owned(), expected: String::from("1 or 2"), actual: argc });
            }
        }
        "pow" | "atan2" | "arctan2" => {
            if argc != 2 {
                return Err(TransformError::ArityMismatch { function: lower.to_owned(), expected: String::from("2"), actual: argc });
            }
        }
        "clamp" => {
            if argc != 3 {
                return Err(TransformError::ArityMismatch { function: String::from("clamp"), expected: String::from("3"), actual: argc });
            }
        }
        "min" | "max" | "sum" | "mean" | "and" | "or" => {
            if argc == 0 {
                return Err(TransformError::ArityMismatch { function: lower.to_owned(), expected: String::from("at least 1"), actual: argc });
            }
        }
        _ => return Err(TransformError::UnknownFunction { name: lower.to_owned() }),
    }
    Ok(())
}

fn eval_scalar_row(lower: &str, cols: &[Vec<FormulaValue>], row: usize) -> Result<FormulaValue, TransformError> {
    if lower == "not" {
        let b = cols.first().and_then(|c| c.get(row)).and_then(FormulaValue::as_bool);
        return Ok(b.map_or(FormulaValue::Missing, |flag| FormulaValue::Number(if flag { 0.0 } else { 1.0 })));
    }

    let Some(x) = cols.first().and_then(|c| c.get(row)).and_then(FormulaValue::as_finite_f64) else {
        return Ok(FormulaValue::Missing);
    };
    if cols.len() == 1 {
        return Ok(FormulaValue::from_f64(eval_unary_math(lower, x)));
    }
    let Some(y) = cols.get(1).and_then(|c| c.get(row)).and_then(FormulaValue::as_finite_f64) else {
        return Ok(FormulaValue::Missing);
    };
    if cols.len() == 2 {
        let out = match lower {
            "pow" => x.powf(y),
            "log" => {
                if x > 0.0 && y > 0.0 && y != 1.0 {
                    x.ln() / y.ln()
                } else {
                    f64::NAN
                }
            }
            "round" => eval_round_digits(x, y),
            "atan2" | "arctan2" => x.atan2(y),
            _ => f64::NAN,
        };
        return Ok(FormulaValue::from_f64(out));
    }
    let Some(z) = cols.get(2).and_then(|c| c.get(row)).and_then(FormulaValue::as_finite_f64) else {
        return Ok(FormulaValue::Missing);
    };
    if lower == "clamp" {
        if y > z {
            return Ok(FormulaValue::Missing);
        }
        return Ok(FormulaValue::from_f64(x.clamp(y, z)));
    }
    Ok(FormulaValue::Missing)
}

fn eval_round_digits(x: f64, digits: f64) -> f64 {
    if !x.is_finite() || !digits.is_finite() {
        return f64::NAN;
    }
    let d = digits.trunc();
    if !(-308.0..=308.0).contains(&d) {
        return if d > 0.0 { x } else { 0.0 };
    }
    #[allow(clippy::cast_possible_truncation)]
    let d_i32 = d as i32;
    if d_i32 == 0 {
        return x.round();
    }
    if d_i32 > 0 {
        let factor = 10.0_f64.powi(d_i32);
        let scaled = x * factor;
        if scaled.is_finite() && factor.is_finite() { scaled.round() / factor } else { x }
    } else {
        let factor = 10.0_f64.powi(-d_i32);
        if factor.is_finite() { (x / factor).round() * factor } else { 0.0 }
    }
}

fn eval_unary_math(lower: &str, x: f64) -> f64 {
    match lower {
        "abs" => x.abs(),
        "sqrt" => x.sqrt(),
        "cbrt" => x.cbrt(),
        "sqr" => x * x,
        "ln" | "log" => x.ln(),
        "log10" => x.log10(),
        "log2" => x.log2(),
        "exp" => x.exp(),
        "exp10" => 10.0_f64.powf(x),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "asin" | "arcsin" => x.asin(),
        "acos" | "arccos" => x.acos(),
        "atan" | "arctan" => x.atan(),
        "sinh" => x.sinh(),
        "cosh" => x.cosh(),
        "tanh" => x.tanh(),
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "round" => x.round(),
        "trunc" => x.trunc(),
        "int" => x.floor(),
        "sign" | "sgn" => {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        "erf" => special::erf(x),
        "erfc" => special::erfc(x),
        "lgamma" => special::ln_gamma(x).unwrap_or(f64::NAN),
        "logit" => {
            if x > 0.0 && x < 1.0 {
                x.ln() - (-x).ln_1p()
            } else {
                f64::NAN
            }
        }
        "probit" => {
            if x > 0.0 && x < 1.0 {
                dist::norm_ppf(x).map_or(f64::NAN, |z| 5.0 + z)
            } else {
                f64::NAN
            }
        }
        "norm_ppf" | "norm_inv" | "qnorm" | "inv_norm" => {
            if x > 0.0 && x < 1.0 {
                dist::norm_ppf(x).unwrap_or(f64::NAN)
            } else {
                f64::NAN
            }
        }
        "arcsin_sqrt" => {
            if (0.0..=1.0).contains(&x) {
                x.sqrt().asin()
            } else {
                f64::NAN
            }
        }
        _ => f64::NAN,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use curvebook_doc::{Cell, DataSheet, MultipleVariablesTable, TableArchetype};

    use super::{FormulaContext, FormulaValue, ScalarEnv, eval_column_expr, eval_scalar_expr, evaluate_formula_for_sheet};
    use crate::error::TransformError;
    use crate::formula::parser::parse_formula;

    fn mv_sheet_from_cols(cols: &[(&str, Vec<Cell>)]) -> DataSheet {
        let rows = cols.first().map_or(0, |(_, c)| c.len());
        let mut mv = MultipleVariablesTable::new(cols.len(), rows).expect("mv");
        for (idx, (title, cells)) in cols.iter().enumerate() {
            mv.variables[idx].title = (*title).to_owned();
            mv.variables[idx].cells = cells.clone();
        }
        DataSheet::new("eval-test", "Eval", TableArchetype::MultipleVariables(mv)).expect("sheet")
    }

    #[test]
    fn aud_04_07_two_arg_log_rejects_invalid_base_and_nonpositive_input() {
        let env = ScalarEnv::default();
        for bad in ["log(100, 0)", "log(100, 1)", "log(100, -2)", "log(0, 10)", "log(-5, 10)"] {
            let val = eval_scalar_expr(&parse_formula(bad).expect("parse"), &env).expect("eval");
            assert_eq!(val, FormulaValue::Missing, "`{bad}` must yield Missing, got {val:?}");
        }
        let ok10 = eval_scalar_expr(&parse_formula("log(100, 10)").expect("parse"), &env).expect("eval");
        assert!((ok10.as_finite_f64().unwrap() - 2.0).abs() < 1e-12);
        let ok2 = eval_scalar_expr(&parse_formula("log(8, 2)").expect("parse"), &env).expect("eval");
        assert!((ok2.as_finite_f64().unwrap() - 3.0).abs() < 1e-12);
    }

    #[test]
    fn aud_04_20_arity_before_eval_streaming_variadics_text_budget_and_fast_windows() {
        // 1. Unknown function & arity mismatch fail BEFORE evaluating arguments (even with unknown vars inside args).
        let env = ScalarEnv::default();
        assert!(matches!(eval_scalar_expr(&parse_formula("bogus(NoSuchCol, 1, 2)").unwrap(), &env), Err(TransformError::UnknownFunction { .. })));
        assert!(matches!(eval_scalar_expr(&parse_formula("sqrt(NoSuchCol, 2)").unwrap(), &env), Err(TransformError::ArityMismatch { .. })));

        // 2. Streaming variadics (min/max/sum/mean/and/or/coalesce) with overflow-safe mean on 1e308.
        let s = mv_sheet_from_cols(&[("A", vec![Cell::number(1e308), Cell::empty()]), ("B", vec![Cell::number(1e308), Cell::number(5.0)])]);
        let mean_vals = evaluate_formula_for_sheet("mean(A, B)", &s, None).expect("streaming mean");
        assert!((mean_vals[0].as_finite_f64().unwrap() - 1e308).abs() <= 1e295);
        assert_eq!(mean_vals[1], FormulaValue::Number(5.0));

        // 3. Text broadcast memory budget blocks multi-GB allocation cleanly.
        struct VirtualRowsContext(usize);
        impl FormulaContext for VirtualRowsContext {
            fn row_count(&self) -> usize {
                self.0
            }
            fn resolve_column_index(&self, _: &str) -> Option<usize> {
                None
            }
            fn column_count(&self) -> usize {
                0
            }
            fn column_values(&self, _: usize) -> Option<Vec<FormulaValue>> {
                None
            }
            fn resolve_info_constant(&self, _: &str) -> Option<FormulaValue> {
                None
            }
        }
        let lit = format!("\"{}\"", "x".repeat(4_000));
        let ast = parse_formula(&lit).expect("4000-byte literal parses");
        assert!(
            matches!(eval_column_expr(&ast, &VirtualRowsContext(50_000)), Err(TransformError::InvalidSpec { .. })),
            "200 MB text broadcast must be rejected before allocation"
        );

        // 4. O(N) / O(N log N) window frames run quickly and accurately on N = 20,000 rows.
        let n = 20_000_usize;
        let big = mv_sheet_from_cols(&[("X", (1..=n).map(|i| Cell::number(i as f64)).collect())]);
        let run_sum = evaluate_formula_for_sheet("sum(X) OVER (ORDER BY X ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)", &big, None)
            .expect("20k running sum");
        assert!((run_sum[n - 1].as_finite_f64().unwrap() - ((n * (n + 1)) as f64) * 0.5).abs() < 1e-3);
        let run_med = evaluate_formula_for_sheet("median(X) OVER (ORDER BY X ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)", &big, None)
            .expect("20k running median");
        assert!((run_med[n - 1].as_finite_f64().unwrap() - ((n + 1) as f64) * 0.5).abs() < 1e-12);
        let roll_mean = evaluate_formula_for_sheet("rolling_mean(X, 500)", &big, None).expect("20k rolling mean");
        assert!((roll_mean[n - 1].as_finite_f64().unwrap() - ((n as f64) - 249.5)).abs() < 1e-9);
    }

    #[test]
    fn aud_04_27_rank_multikey_desc_text_keys_and_deep_expr_equality() {
        let s = mv_sheet_from_cols(&[
            ("Grp", vec![Cell::text("G1"), Cell::text("G1"), Cell::text("G1"), Cell::text("G1")]),
            ("Score", vec![Cell::number(10.0), Cell::number(20.0), Cell::number(30.0), Cell::empty()]),
            ("Id", vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0)]),
            ("Label", vec![Cell::text("B"), Cell::text("A"), Cell::text("A"), Cell::text("C")]),
        ]);

        // Multi-key ORDER BY Score DESC, Id ASC must honor DESC for rank(Score).
        let r_desc = evaluate_formula_for_sheet("rank(Score) OVER (PARTITION BY Grp ORDER BY Score DESC, Id ASC)", &s, None).expect("multikey rank");
        assert_eq!(r_desc, vec![FormulaValue::Number(3.0), FormulaValue::Number(2.0), FormulaValue::Number(1.0), FormulaValue::Missing]);

        // Text ordering in 1-arg rank(Label) and dense_rank(Label).
        let r_txt = evaluate_formula_for_sheet("rank(Label)", &s, None).expect("text rank");
        assert_eq!(r_txt, vec![FormulaValue::Number(3.0), FormulaValue::Number(1.5), FormulaValue::Number(1.5), FormulaValue::Number(4.0)]);
        let dr_txt = evaluate_formula_for_sheet("dense_rank(Label) OVER (ORDER BY Label DESC, Id ASC)", &s, None).expect("text dense_rank desc");
        assert_eq!(dr_txt, vec![FormulaValue::Number(2.0), FormulaValue::Number(3.0), FormulaValue::Number(3.0), FormulaValue::Number(1.0)]);

        // AUD-04-02: deep left-leaning expression inside rank(expr) OVER (ORDER BY expr) uses iterative PartialEq.
        let deep = vec!["1"; 4_000].join("+");
        let formula = format!("rank({deep}) OVER (ORDER BY {deep} DESC)");
        let handle = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || evaluate_formula_for_sheet(&formula, &s, None).expect("deep rank over"))
            .expect("spawn 1 MiB worker");
        let res = handle.join().expect("rank PartialEq must not stack overflow");
        assert_eq!(res.len(), 4);
    }

    #[test]
    fn aud_04_28_explicit_rows_frames_and_rolling_aggregate_over_missing_current_row_and_large_mean() {
        let s = mv_sheet_from_cols(&[
            ("Y", vec![Cell::number(10.0), Cell::number(20.0), Cell::empty(), Cell::number(40.0)]),
            ("Big", vec![Cell::number(1e308), Cell::number(1e308), Cell::empty(), Cell::number(1e308)]),
        ]);

        // Explicit ROWS frame on rolling_mean & cumsum must NOT blank row 2 when neighbors are finite.
        let r_mean =
            evaluate_formula_for_sheet("rolling_mean(Y) OVER (ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING)", &s, None).expect("rows rolling_mean");
        assert_eq!(r_mean[2], FormulaValue::Number(30.0));
        let c_sum = evaluate_formula_for_sheet("cumsum(Y) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)", &s, None).expect("rows cumsum");
        assert_eq!(c_sum[2], FormulaValue::Number(30.0));
        let c_mean =
            evaluate_formula_for_sheet("cummean(Y) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)", &s, None).expect("rows cummean");
        assert_eq!(c_mean[2], FormulaValue::Number(15.0));

        // 2-arg rolling_mean(Y, 3) on [10, 20, Missing, 40]: row 2 window [10, 20, Missing] -> mean 15.0.
        let r2 = evaluate_formula_for_sheet("rolling_mean(Y, 3)", &s, None).expect("2-arg rolling_mean");
        assert_eq!(r2[2], FormulaValue::Number(15.0));

        // Overflow-safe rolling mean on [1e308, 1e308].
        let big_roll = evaluate_formula_for_sheet("rolling_mean(Big, 2)", &s, None).expect("big rolling_mean");
        assert!((big_roll[1].as_finite_f64().unwrap() - 1e308).abs() <= 1e295);
        let big_rows =
            evaluate_formula_for_sheet("rolling_mean(Big) OVER (ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)", &s, None).expect("big rows mean");
        assert!((big_rows[1].as_finite_f64().unwrap() - 1e308).abs() <= 1e295);
    }

    #[test]
    fn aud_04_20_moments_agg_tiny_scale_sd_sem_and_zscore_do_not_underflow() {
        let s =
            mv_sheet_from_cols(&[("Tiny", vec![Cell::number(1e-200), Cell::number(2e-200)]), ("Const", vec![Cell::number(5.0), Cell::number(5.0)])]);
        let expected_sd = std::f64::consts::FRAC_1_SQRT_2 * 1e-200;
        let expected_sem = 0.5e-200;

        let sd_res =
            evaluate_formula_for_sheet("sd(Tiny) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)", &s, None).expect("tiny framed sd");
        let sem_res = evaluate_formula_for_sheet("sem(Tiny) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)", &s, None)
            .expect("tiny framed sem");
        let z_res = evaluate_formula_for_sheet("zscore(Tiny) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)", &s, None)
            .expect("tiny framed zscore");
        let const_sd = evaluate_formula_for_sheet("sd(Const) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)", &s, None)
            .expect("constant framed sd");

        let sd_val = sd_res[0].as_finite_f64().expect("tiny sd finite");
        assert!((sd_val - expected_sd).abs() <= 1e-214, "expected {expected_sd:e}, got {sd_val:e}");
        let sem_val = sem_res[0].as_finite_f64().expect("tiny sem finite");
        assert!((sem_val - expected_sem).abs() <= 1e-214, "expected {expected_sem:e}, got {sem_val:e}");
        let z0 = z_res[0].as_finite_f64().expect("tiny zscore[0] finite");
        let z1 = z_res[1].as_finite_f64().expect("tiny zscore[1] finite");
        assert!((z0 + std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12, "z0={z0}");
        assert!((z1 - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12, "z1={z1}");
        assert_eq!(const_sd[0], FormulaValue::Number(0.0));
    }
}
