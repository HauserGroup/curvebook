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
        FormulaValue::from_f64(sum::sum(&active) / (active.len() as f64))
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
        TableArchetype::Xy(t) => Some((0..t.row_count()).map(|r| t.x_value_at(r).map_or(FormulaValue::Missing, FormulaValue::from_f64)).collect()),
        TableArchetype::Survival(t) => Some(t.times.iter().map(FormulaValue::from_cell).collect()),
        _ => None,
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
                Expr::Text(s) => stack.push(vec![FormulaValue::Text(s.clone()); n]),
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
                && let Some(xs) = ctx.resolve_x_column()
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
            ctx.resolve_info_constant(key).map(|v| vec![v; n]).ok_or_else(|| TransformError::UnknownVariable { name: format!("Info(\"{key}\")") })
        }
        VarRef::Bracketed(name) => {
            if let Some(idx) = ctx.resolve_column_index(name)
                && let Some(vals) = ctx.column_values(idx)
            {
                return Ok(normalize_col_len(vals, n));
            }
            if name.trim().eq_ignore_ascii_case("x")
                && let Some(xs) = ctx.resolve_x_column()
            {
                return Ok(normalize_col_len(xs, n));
            }
            if let Some(v) = ctx.resolve_info_constant(name) {
                return Ok(vec![v; n]);
            }
            if let Some(v) = ctx.resolve_scalar(name) {
                return Ok(vec![v; n]);
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
        return Ok(vec![v; n]);
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
            return Ok(vec![v; n]);
        }
        return Err(TransformError::UnknownVariable { name: name.to_owned() });
    }
    if trimmed.eq_ignore_ascii_case("x")
        && let Some(xs) = ctx.resolve_x_column()
    {
        return Ok(normalize_col_len(xs, n));
    }
    if let Some(idx) = ctx.resolve_column_index(trimmed)
        && let Some(vals) = ctx.column_values(idx)
    {
        return Ok(normalize_col_len(vals, n));
    }
    if let Some(v) = ctx.resolve_info_constant(trimmed) {
        return Ok(vec![v; n]);
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
            let cols: Vec<Vec<FormulaValue>> = args.iter().map(|a| eval_column_expr_inner(a, ctx)).collect::<Result<_, _>>()?;
            let mut out = vec![FormulaValue::Missing; n];
            for (r, slot) in out.iter_mut().enumerate() {
                for col in &cols {
                    if let Some(v) = col.get(r)
                        && v.is_valid()
                    {
                        *slot = v.clone();
                        break;
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
        (FormulaValue::Number(x), FormulaValue::Number(y)) => x.total_cmp(y),
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

fn looks_purely_boolean_values(vals: &[FormulaValue]) -> bool {
    let mut saw_valid = false;
    for v in vals {
        match v {
            FormulaValue::Missing => {}
            FormulaValue::Number(x) if *x == 0.0 || *x == 1.0 => saw_valid = true,
            FormulaValue::Text(s) if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false") => saw_valid = true,
            _ => return false,
        }
    }
    saw_valid
}

fn resolve_cond_val_pair(
    fn_name: &str,
    arg0: &Expr,
    arg1: &Expr,
    ctx: &dyn FormulaContext,
) -> Result<(Vec<FormulaValue>, Vec<FormulaValue>), TransformError> {
    let v0 = eval_column_expr_inner(arg0, ctx)?;
    let v1 = eval_column_expr_inner(arg1, ctx)?;
    let a0_cond = is_condition_ast(arg0);
    let a1_cond = is_condition_ast(arg1);
    if a0_cond && !a1_cond {
        return Ok((v0, v1));
    }
    if !a0_cond && a1_cond {
        return Ok((v1, v0));
    }
    let b0 = looks_purely_boolean_values(&v0);
    let b1 = looks_purely_boolean_values(&v1);
    if b0 && !b1 {
        return Ok((v0, v1));
    }
    if !b0 && b1 {
        return Ok((v1, v0));
    }
    if matches!(fn_name, "sumif" | "meanif") { Ok((v0, v1)) } else { Ok((v1, v0)) }
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
    if sorted.is_empty() || !p_percent.is_finite() || !(0.0..=100.0).contains(&p_percent) {
        return FormulaValue::Missing;
    }
    if sorted.len() == 1 {
        return sorted.first().copied().map_or(FormulaValue::Missing, FormulaValue::from_f64);
    }
    let pos = (p_percent / 100.0) * ((sorted.len() - 1) as f64);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = (pos.floor() as usize).min(sorted.len() - 1);
    let hi = (lo + 1).min(sorted.len() - 1);
    let frac = pos - (lo as f64);
    let (Some(&lo_v), Some(&hi_v)) = (sorted.get(lo), sorted.get(hi)) else {
        return FormulaValue::Missing;
    };
    FormulaValue::from_f64(lo_v + frac * (hi_v - lo_v))
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
                    && !lit.eq_ignore_ascii_case("true")
                    && !lit.eq_ignore_ascii_case("false")
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
                    let (cond_vals, val_col) = resolve_cond_val_pair("countif", &args[0], &args[1], ctx)?;
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
            let (cond_vals, val_col) = resolve_cond_val_pair("countaif", &args[0], &args[1], ctx)?;
            (AggOp::CountAIf, val_col, Some(cond_vals))
        }
        "sumif" => {
            check_arity("sumif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair("sumif", &args[0], &args[1], ctx)?;
            (AggOp::SumIf, val_col, Some(cond_vals))
        }
        "meanif" => {
            check_arity("meanif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair("meanif", &args[0], &args[1], ctx)?;
            (AggOp::MeanIf, val_col, Some(cond_vals))
        }
        "minif" => {
            check_arity("minif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair("minif", &args[0], &args[1], ctx)?;
            (AggOp::MinIf, val_col, Some(cond_vals))
        }
        "maxif" => {
            check_arity("maxif", args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair("maxif", &args[0], &args[1], ctx)?;
            (AggOp::MaxIf, val_col, Some(cond_vals))
        }
        "stdevif" | "sdif" => {
            check_arity(lower, args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(lower, &args[0], &args[1], ctx)?;
            (AggOp::SdIf, val_col, Some(cond_vals))
        }
        "stderrif" | "semif" => {
            check_arity(lower, args, 2)?;
            let (cond_vals, val_col) = resolve_cond_val_pair(lower, &args[0], &args[1], ctx)?;
            (AggOp::SemIf, val_col, Some(cond_vals))
        }
        _ => return Ok(None),
    };

    let where_mask = eval_where_mask(filter, n, ctx)?;
    let Some(wspec) = window else {
        let all_rows: Vec<usize> = (0..n).collect();
        let val = compute_agg_over_rows(&op, &all_rows, where_mask.as_deref(), &vals, conds.as_deref());
        return Ok(Some(vec![val; n]));
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
        for (pos, &r) in part.iter().enumerate() {
            let agg_val = if let Some(frame) = resolved_frame {
                match frame.bounds_for(pos, part.len()) {
                    Some((lo, hi)) => compute_agg_over_rows(&op, &part[lo..=hi], where_mask.as_deref(), &vals, conds.as_deref()),
                    None => op.empty_frame_value(),
                }
            } else if let Some(ends) = &peer_ends {
                let end = ends.get(pos).copied().unwrap_or(pos);
                compute_agg_over_rows(&op, &part[0..=end], where_mask.as_deref(), &vals, conds.as_deref())
            } else {
                compute_agg_over_rows(&op, part, where_mask.as_deref(), &vals, conds.as_deref())
            };
            if let Some(slot) = out.get_mut(r) {
                *slot = agg_val;
            }
        }
    }
    Ok(Some(out))
}

fn eval_scalar_const_arg(expr: &Expr, ctx: &dyn FormulaContext) -> Result<f64, TransformError> {
    if ctx.row_count() == 0 {
        let env = ScalarEnv::default();
        let vals = eval_column_expr_inner(expr, &env)?;
        return vals
            .first()
            .and_then(FormulaValue::as_finite_f64)
            .ok_or_else(|| TransformError::EvalError { message: String::from("expected finite numeric parameter") });
    }
    let vals = eval_column_expr_inner(expr, ctx)?;
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

fn expr_matches_single_order(expr: &Expr, order_by: &[OrderByExpr]) -> Option<SortDirection> {
    if order_by.len() != 1 {
        return None;
    }
    let ord = &order_by[0];
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
        let desc = expr_matches_single_order(arg0, order_by_slice) == Some(SortDirection::Desc);
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
            for (pos, &r) in part.iter().enumerate() {
                let Some(x) = vals.get(r).and_then(FormulaValue::as_finite_f64) else {
                    continue;
                };
                let Some((lo, hi)) = frame.bounds_for(pos, part.len()) else {
                    continue;
                };
                let nums = frame_finite_nums(part, lo, hi, &vals);
                if let Some(slot) = out.get_mut(r) {
                    *slot = compute_center_or_z(x, &nums, standardize);
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
            for (pos, &r) in part.iter().enumerate() {
                if vals.get(r).and_then(FormulaValue::as_finite_f64).is_none() {
                    continue;
                }
                let Some((lo, hi)) = frame.bounds_for(pos, part.len()) else {
                    continue;
                };
                let nums = frame_finite_nums(part, lo, hi, &vals);
                if nums.is_empty() {
                    continue;
                }
                let res = match lower {
                    "cumsum" => FormulaValue::from_f64(sum::sum(&nums)),
                    "cummean" => sum::mean(&nums).map_or(FormulaValue::Missing, FormulaValue::from_f64),
                    _ => FormulaValue::from_f64(nums.iter().copied().product()),
                };
                if let Some(slot) = out.get_mut(r) {
                    *slot = res;
                }
            }
        } else {
            eval_cum_partition(lower, part, &vals, &mut out);
        }
    }
    Ok(out)
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
            for (pos, &r) in part.iter().enumerate() {
                if vals.get(r).and_then(FormulaValue::as_finite_f64).is_none() {
                    continue;
                }
                let Some((lo, hi)) = frame.bounds_for(pos, part.len()) else {
                    continue;
                };
                let slice_nums = frame_finite_nums(part, lo, hi, &vals);
                if !slice_nums.is_empty()
                    && let Some(slot) = out.get_mut(r)
                {
                    let total = sum::sum(&slice_nums);
                    let res = if as_mean { total / (slice_nums.len() as f64) } else { total };
                    *slot = FormulaValue::from_f64(res);
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

fn eval_cum_partition(lower: &str, part: &[usize], vals: &[FormulaValue], out: &mut [FormulaValue]) {
    let mut acc = sum::NeumaierSum::new();
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
                FormulaValue::from_f64(acc.total() / (count as f64))
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
    for i in 0..n {
        if i + 1 < window || vals.get(i).and_then(FormulaValue::as_finite_f64).is_none() {
            out.push(FormulaValue::Missing);
            continue;
        }
        let start = i + 1 - window;
        let slice_nums: Vec<f64> = vals.get(start..=i).unwrap_or(&[]).iter().filter_map(FormulaValue::as_finite_f64).collect();
        if slice_nums.is_empty() {
            out.push(FormulaValue::Missing);
        } else {
            let total = sum::sum(&slice_nums);
            let res = if as_mean { total / (slice_nums.len() as f64) } else { total };
            out.push(FormulaValue::from_f64(res));
        }
    }
    out
}

fn eval_rank_directed(vals: &[FormulaValue], dense: bool, descending: bool) -> Vec<FormulaValue> {
    let mut indexed: Vec<(usize, f64)> = vals.iter().enumerate().filter_map(|(i, v)| v.as_finite_f64().map(|x| (i, x))).collect();
    indexed.sort_by(|a, b| if descending { b.1.total_cmp(&a.1) } else { a.1.total_cmp(&b.1) });
    let mut out = vec![FormulaValue::Missing; vals.len()];
    let mut pos = 0;
    let mut dense_level = 1_usize;
    while let Some(&(_, pos_val)) = indexed.get(pos) {
        let mut end = pos + 1;
        while indexed.get(end).is_some_and(|&(_, end_val)| end_val == pos_val) {
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
    let n = ctx.row_count();
    let evaled: Vec<Vec<FormulaValue>> = args.iter().map(|a| eval_column_expr_inner(a, ctx)).collect::<Result<_, _>>()?;
    validate_scalar_call_arity(lower, args.len())?;

    let mut out = Vec::with_capacity(n);
    for row in 0..n {
        let val = eval_scalar_row(lower, &evaled, row)?;
        out.push(val);
    }
    Ok(out)
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
    match lower {
        "min" | "max" => return Ok(eval_variadic_minmax(cols, row, lower == "min")),
        "sum" | "mean" => return Ok(eval_variadic_sum_or_mean(cols, row, lower == "mean")),
        "and" | "or" => return Ok(eval_variadic_logic(cols, row, lower == "and")),
        "not" => {
            let b = cols.first().and_then(|c| c.get(row)).and_then(FormulaValue::as_bool);
            return Ok(b.map_or(FormulaValue::Missing, |flag| FormulaValue::Number(if flag { 0.0 } else { 1.0 })));
        }
        _ => {}
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
            "log" => x.ln() / y.ln(),
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

fn eval_variadic_sum_or_mean(cols: &[Vec<FormulaValue>], row: usize, is_mean: bool) -> FormulaValue {
    let nums: Vec<f64> = cols.iter().filter_map(|col| col.get(row).and_then(FormulaValue::as_finite_f64)).collect();
    if nums.is_empty() {
        return FormulaValue::Missing;
    }
    if is_mean { sum::mean(&nums).map_or(FormulaValue::Missing, FormulaValue::from_f64) } else { FormulaValue::from_f64(sum::sum(&nums)) }
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

fn eval_variadic_minmax(cols: &[Vec<FormulaValue>], row: usize, is_min: bool) -> FormulaValue {
    let mut acc: Option<f64> = None;
    for col in cols {
        let Some(v) = col.get(row).and_then(FormulaValue::as_finite_f64) else {
            continue;
        };
        acc = Some(match acc {
            None => v,
            Some(cur) => {
                if is_min {
                    cur.min(v)
                } else {
                    cur.max(v)
                }
            }
        });
    }
    acc.map_or(FormulaValue::Missing, FormulaValue::from_f64)
}

fn eval_variadic_logic(cols: &[Vec<FormulaValue>], row: usize, is_and: bool) -> FormulaValue {
    let mut saw_missing = false;
    for col in cols {
        match col.get(row).and_then(FormulaValue::as_bool) {
            Some(false) if is_and => return FormulaValue::Number(0.0),
            Some(true) if !is_and => return FormulaValue::Number(1.0),
            Some(_) => {}
            None => saw_missing = true,
        }
    }
    if saw_missing { FormulaValue::Missing } else { FormulaValue::Number(if is_and { 1.0 } else { 0.0 }) }
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
