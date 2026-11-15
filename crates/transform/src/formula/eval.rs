//! Vectorized and scalar formula evaluation over `DataSheet`, `MultipleVariablesTable`, and
//! `InfoSheet` constants (`docs/data-tables-and-transforms.md` §4.1).

use std::cmp::Ordering;

use lumencraft_doc::{Cell, CellValue, DataSheet, InfoSheet, MultipleVariablesTable, TableArchetype, default_column_label};
use lumencraft_math::{dist, special, sum};

use crate::error::TransformError;
use crate::formula::parser::{BinaryOp, Expr, UnaryOp, VarRef, parse_formula};

/// Typed runtime value produced by evaluating a formula expression.
#[derive(Debug, Clone, PartialEq)]
pub enum FormulaValue {
    Number(f64),
    Text(String),
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
    fn row_count(&self) -> usize;
    fn resolve_column_index(&self, name: &str) -> Option<usize>;
    fn column_count(&self) -> usize;
    fn column_values(&self, col_idx: usize) -> Option<Vec<FormulaValue>>;
    fn resolve_x_column(&self) -> Option<Vec<FormulaValue>> {
        None
    }
    fn resolve_info_constant(&self, name: &str) -> Option<FormulaValue>;
    fn resolve_scalar(&self, _name: &str) -> Option<FormulaValue> {
        None
    }
}

/// Scalar-only environment for `UserExpression` or custom `Simulate` formulas (`x`, `y`, params).
#[derive(Debug, Clone, Default)]
pub struct ScalarEnv<'a> {
    pub bindings: Vec<(&'a str, f64)>,
    pub info_sheet: Option<&'a InfoSheet>,
}

impl<'a> ScalarEnv<'a> {
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
    pub sheet: &'a DataSheet,
    pub info_sheet: Option<&'a InfoSheet>,
    pub extra_scalars: Vec<(&'a str, f64)>,
}

impl<'a> TableEvalContext<'a> {
    #[must_use]
    pub fn new(sheet: &'a DataSheet, info_sheet: Option<&'a InfoSheet>) -> Self {
        Self { sheet, info_sheet, extra_scalars: Vec::new() }
    }
}

impl FormulaContext for TableEvalContext<'_> {
    fn row_count(&self) -> usize {
        self.sheet.archetype.row_count()
    }

    fn resolve_column_index(&self, name: &str) -> Option<usize> {
        resolve_archetype_column(&self.sheet.archetype, name)
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
    pub table: &'a MultipleVariablesTable,
    pub info_sheet: Option<&'a InfoSheet>,
}

impl FormulaContext for MultipleVariablesContext<'_> {
    fn row_count(&self) -> usize {
        self.table.row_count()
    }

    fn resolve_column_index(&self, name: &str) -> Option<usize> {
        resolve_mv_column(self.table, name)
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
        lumencraft_doc::ConstantValue::Empty => Some(FormulaValue::Missing),
        lumencraft_doc::ConstantValue::Number(v) => Some(FormulaValue::from_f64(*v)),
        lumencraft_doc::ConstantValue::Text(s) | lumencraft_doc::ConstantValue::Date(s) => {
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

/// Evaluate a parsed [`Expr`] across every row `0..ctx.row_count()` in `O(N)` vectorized passes.
pub fn eval_column_expr(expr: &Expr, ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    let n = ctx.row_count();
    match expr {
        Expr::Number(v) => Ok(vec![FormulaValue::from_f64(*v); n]),
        Expr::Text(s) => Ok(vec![FormulaValue::Text(s.clone()); n]),
        Expr::Var(vref) => eval_var_ref(vref, ctx),
        Expr::Unary { op, expr: inner } => {
            let vals = eval_column_expr(inner, ctx)?;
            Ok(vals.into_iter().map(|v| apply_unary(*op, &v)).collect())
        }
        Expr::Binary { op, left, right } => {
            let lhs = eval_column_expr(left, ctx)?;
            let rhs = eval_column_expr(right, ctx)?;
            Ok(lhs.into_iter().zip(rhs).map(|(a, b)| apply_binary(*op, &a, &b)).collect())
        }
        Expr::Call { name, args } => eval_call(name, args, ctx),
    }
}

fn eval_var_ref(vref: &VarRef, ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    let n = ctx.row_count();
    match vref {
        VarRef::RowOneBased =>
        {
            #[allow(clippy::cast_precision_loss)]
            Ok((1..=n).map(|r| FormulaValue::Number(r as f64)).collect())
        }
        VarRef::IndexZeroBased =>
        {
            #[allow(clippy::cast_precision_loss)]
            Ok((0..n).map(|r| FormulaValue::Number(r as f64)).collect())
        }
        VarRef::ColumnByIndex(idx) => ctx.column_values(*idx).ok_or_else(|| TransformError::UnknownVariable { name: format!("col({idx})") }),
        VarRef::ColumnByName(name) => {
            if name.trim().eq_ignore_ascii_case("x")
                && let Some(xs) = ctx.resolve_x_column()
            {
                return Ok(xs);
            }
            if let Some(idx) = ctx.resolve_column_index(name)
                && let Some(vals) = ctx.column_values(idx)
            {
                return Ok(vals);
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
                return Ok(vals);
            }
            if name.trim().eq_ignore_ascii_case("x")
                && let Some(xs) = ctx.resolve_x_column()
            {
                return Ok(xs);
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
    if let Some(v) = ctx.resolve_scalar(name) {
        return Ok(vec![v; n]);
    }
    if name.eq_ignore_ascii_case("x")
        && let Some(xs) = ctx.resolve_x_column()
    {
        return Ok(xs);
    }
    if let Some(idx) = ctx.resolve_column_index(name)
        && let Some(vals) = ctx.column_values(idx)
    {
        return Ok(vals);
    }
    if let Some(v) = ctx.resolve_info_constant(name) {
        return Ok(vec![v; n]);
    }
    if let Some(c) = builtin_constant(name) {
        return Ok(vec![c; n]);
    }
    Err(TransformError::UnknownVariable { name: name.to_owned() })
}

fn builtin_constant(name: &str) -> Option<FormulaValue> {
    if name.eq_ignore_ascii_case("pi") {
        Some(FormulaValue::Number(std::f64::consts::PI))
    } else if name.eq_ignore_ascii_case("e") {
        Some(FormulaValue::Number(std::f64::consts::E))
    } else if name.eq_ignore_ascii_case("tau") {
        Some(FormulaValue::Number(std::f64::consts::TAU))
    } else if name.eq_ignore_ascii_case("true") {
        Some(FormulaValue::Number(1.0))
    } else if name.eq_ignore_ascii_case("false") {
        Some(FormulaValue::Number(0.0))
    } else if name.eq_ignore_ascii_case("nan") {
        Some(FormulaValue::Missing)
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
        BinaryOp::And => match (lhs.as_bool(), rhs.as_bool()) {
            (Some(a), Some(b)) => FormulaValue::Number(if a && b { 1.0 } else { 0.0 }),
            _ => FormulaValue::Missing,
        },
        BinaryOp::Or => match (lhs.as_bool(), rhs.as_bool()) {
            (Some(a), Some(b)) => FormulaValue::Number(if a || b { 1.0 } else { 0.0 }),
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

fn eval_call(name: &str, args: &[Expr], ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    let lower = name.to_ascii_lowercase();
    if let Some(res) = try_eval_special_call(&lower, args, ctx)? {
        return Ok(res);
    }
    if let Some(res) = try_eval_aggregate_call(&lower, args, ctx)? {
        return Ok(res);
    }
    if let Some(res) = try_eval_window_call(&lower, args, ctx)? {
        return Ok(res);
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
            check_arity("if", args, 3)?;
            let cond = eval_column_expr(&args[0], ctx)?;
            let t_vals = eval_column_expr(&args[1], ctx)?;
            let f_vals = eval_column_expr(&args[2], ctx)?;
            let mut out = Vec::with_capacity(n);
            for ((c, tv), fv) in cond.into_iter().zip(t_vals).zip(f_vals) {
                match c.as_bool() {
                    Some(true) => out.push(tv),
                    Some(false) => out.push(fv),
                    None => out.push(FormulaValue::Missing),
                }
            }
            Ok(Some(out))
        }
        "isnan" => {
            check_arity("isnan", args, 1)?;
            let vals = eval_column_expr(&args[0], ctx)?;
            Ok(Some(vals.into_iter().map(|v| FormulaValue::Number(if v.as_finite_f64().is_none() { 1.0 } else { 0.0 })).collect()))
        }
        "isvalid" | "is_defined" => {
            check_arity(lower, args, 1)?;
            let vals = eval_column_expr(&args[0], ctx)?;
            Ok(Some(vals.into_iter().map(|v| FormulaValue::Number(if v.is_valid() { 1.0 } else { 0.0 })).collect()))
        }
        "coalesce" => {
            check_min_arity("coalesce", args, 1)?;
            let cols: Vec<Vec<FormulaValue>> = args.iter().map(|a| eval_column_expr(a, ctx)).collect::<Result<_, _>>()?;
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
        "row_number" => {
            check_arity("row_number", args, 0)?;
            #[allow(clippy::cast_precision_loss)]
            Ok(Some((1..=n).map(|r| FormulaValue::Number(r as f64)).collect()))
        }
        _ => Ok(None),
    }
}

fn extract_finite_numbers(vals: &[FormulaValue]) -> Vec<f64> {
    vals.iter().filter_map(FormulaValue::as_finite_f64).collect()
}

fn sorted_numbers(vals: &[FormulaValue]) -> Vec<f64> {
    let mut nums = extract_finite_numbers(vals);
    nums.sort_by(f64::total_cmp);
    nums
}

fn linear_percentile(sorted: &[f64], p_percent: f64) -> FormulaValue {
    if sorted.is_empty() || !p_percent.is_finite() || !(0.0..=100.0).contains(&p_percent) {
        return FormulaValue::Missing;
    }
    if sorted.len() == 1 {
        return FormulaValue::from_f64(sorted[0]);
    }
    let pos = (p_percent / 100.0) * ((sorted.len() - 1) as f64);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = (pos.floor() as usize).min(sorted.len() - 1);
    let hi = (lo + 1).min(sorted.len() - 1);
    let frac = pos - (lo as f64);
    FormulaValue::from_f64(sorted[lo] + frac * (sorted[hi] - sorted[lo]))
}

fn try_eval_aggregate_call(lower: &str, args: &[Expr], ctx: &dyn FormulaContext) -> Result<Option<Vec<FormulaValue>>, TransformError> {
    let n = ctx.row_count();
    let val = match lower {
        "sum" if args.len() == 1 => {
            let nums = extract_finite_numbers(&eval_column_expr(&args[0], ctx)?);
            FormulaValue::from_f64(sum::sum(&nums))
        }
        "count" => {
            check_arity("count", args, 1)?;
            let cnt = eval_column_expr(&args[0], ctx)?.iter().filter(|v| v.is_valid()).count();
            #[allow(clippy::cast_precision_loss)]
            FormulaValue::Number(cnt as f64)
        }
        "mean" if args.len() == 1 => {
            let nums = extract_finite_numbers(&eval_column_expr(&args[0], ctx)?);
            sum::mean(&nums).map_or(FormulaValue::Missing, FormulaValue::from_f64)
        }
        "sd" => {
            check_arity("sd", args, 1)?;
            let nums = extract_finite_numbers(&eval_column_expr(&args[0], ctx)?);
            sum::sample_sd(&nums).map_or(FormulaValue::Missing, FormulaValue::from_f64)
        }
        "sem" => {
            check_arity("sem", args, 1)?;
            let nums = extract_finite_numbers(&eval_column_expr(&args[0], ctx)?);
            sum::SummaryMoments::from_slice(&nums).and_then(|m| m.sem()).map_or(FormulaValue::Missing, FormulaValue::from_f64)
        }
        "var" => {
            check_arity("var", args, 1)?;
            let nums = extract_finite_numbers(&eval_column_expr(&args[0], ctx)?);
            sum::sample_variance(&nums).map_or(FormulaValue::Missing, FormulaValue::from_f64)
        }
        "median" => {
            check_arity("median", args, 1)?;
            let sorted = sorted_numbers(&eval_column_expr(&args[0], ctx)?);
            linear_percentile(&sorted, 50.0)
        }
        "iqr" => {
            check_arity("iqr", args, 1)?;
            let sorted = sorted_numbers(&eval_column_expr(&args[0], ctx)?);
            match (linear_percentile(&sorted, 75.0).as_finite_f64(), linear_percentile(&sorted, 25.0).as_finite_f64()) {
                (Some(q3), Some(q1)) => FormulaValue::from_f64(q3 - q1),
                _ => FormulaValue::Missing,
            }
        }
        "min_col" | "col_min" => {
            check_arity(lower, args, 1)?;
            let nums = extract_finite_numbers(&eval_column_expr(&args[0], ctx)?);
            nums.into_iter().reduce(f64::min).map_or(FormulaValue::Missing, FormulaValue::from_f64)
        }
        "max_col" | "col_max" => {
            check_arity(lower, args, 1)?;
            let nums = extract_finite_numbers(&eval_column_expr(&args[0], ctx)?);
            nums.into_iter().reduce(f64::max).map_or(FormulaValue::Missing, FormulaValue::from_f64)
        }
        "geo_mean" => {
            check_arity("geo_mean", args, 1)?;
            let nums = extract_finite_numbers(&eval_column_expr(&args[0], ctx)?);
            if nums.is_empty() || nums.iter().any(|&v| v <= 0.0) {
                FormulaValue::Missing
            } else {
                let logs: Vec<f64> = nums.iter().map(|v| v.ln()).collect();
                FormulaValue::from_f64((sum::sum(&logs) / (logs.len() as f64)).exp())
            }
        }
        "percentile" | "quantile" => {
            check_arity(lower, args, 2)?;
            let sorted = sorted_numbers(&eval_column_expr(&args[0], ctx)?);
            let p_val = eval_scalar_const_arg(&args[1], ctx)?;
            let p_pct = if lower == "quantile" { p_val * 100.0 } else { p_val };
            linear_percentile(&sorted, p_pct)
        }
        "countif" => {
            check_arity("countif", args, 1)?;
            let conds = eval_column_expr(&args[0], ctx)?;
            let cnt = conds.iter().filter(|v| v.as_bool() == Some(true)).count();
            #[allow(clippy::cast_precision_loss)]
            FormulaValue::Number(cnt as f64)
        }
        "sumif" => {
            check_arity("sumif", args, 2)?;
            let conds = eval_column_expr(&args[0], ctx)?;
            let vals = eval_column_expr(&args[1], ctx)?;
            let filtered: Vec<f64> =
                conds.iter().zip(vals.iter()).filter_map(|(c, v)| (c.as_bool() == Some(true)).then(|| v.as_finite_f64()).flatten()).collect();
            FormulaValue::from_f64(sum::sum(&filtered))
        }
        "meanif" => {
            check_arity("meanif", args, 2)?;
            let conds = eval_column_expr(&args[0], ctx)?;
            let vals = eval_column_expr(&args[1], ctx)?;
            let filtered: Vec<f64> =
                conds.iter().zip(vals.iter()).filter_map(|(c, v)| (c.as_bool() == Some(true)).then(|| v.as_finite_f64()).flatten()).collect();
            sum::mean(&filtered).map_or(FormulaValue::Missing, FormulaValue::from_f64)
        }
        _ => return Ok(None),
    };
    Ok(Some(vec![val; n]))
}

fn eval_scalar_const_arg(expr: &Expr, ctx: &dyn FormulaContext) -> Result<f64, TransformError> {
    let vals = eval_column_expr(expr, ctx)?;
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

fn try_eval_window_call(lower: &str, args: &[Expr], ctx: &dyn FormulaContext) -> Result<Option<Vec<FormulaValue>>, TransformError> {
    match lower {
        "cumsum" => {
            check_arity("cumsum", args, 1)?;
            let vals = eval_column_expr(&args[0], ctx)?;
            let mut acc = sum::NeumaierSum::new();
            Ok(Some(
                vals.into_iter()
                    .map(|v| match v.as_finite_f64() {
                        Some(x) => {
                            acc.add(x);
                            FormulaValue::from_f64(acc.total())
                        }
                        None => FormulaValue::Missing,
                    })
                    .collect(),
            ))
        }
        "cumprod" => {
            check_arity("cumprod", args, 1)?;
            let vals = eval_column_expr(&args[0], ctx)?;
            let mut prod = 1.0_f64;
            Ok(Some(
                vals.into_iter()
                    .map(|v| match v.as_finite_f64() {
                        Some(x) => {
                            prod *= x;
                            FormulaValue::from_f64(prod)
                        }
                        None => FormulaValue::Missing,
                    })
                    .collect(),
            ))
        }
        "cummean" => {
            check_arity("cummean", args, 1)?;
            let vals = eval_column_expr(&args[0], ctx)?;
            let mut acc = sum::NeumaierSum::new();
            let mut count = 0_usize;
            Ok(Some(
                vals.into_iter()
                    .map(|v| match v.as_finite_f64() {
                        Some(x) => {
                            acc.add(x);
                            count += 1;
                            FormulaValue::from_f64(acc.total() / (count as f64))
                        }
                        None => FormulaValue::Missing,
                    })
                    .collect(),
            ))
        }
        "lag" | "lead" => {
            if args.is_empty() || args.len() > 2 {
                return Err(TransformError::ArityMismatch { function: lower.to_owned(), expected: String::from("1 or 2"), actual: args.len() });
            }
            let vals = eval_column_expr(&args[0], ctx)?;
            let k = if args.len() == 2 { eval_nonneg_int_arg(&args[1], ctx, lower)? } else { 1 };
            Ok(Some(eval_shift(&vals, k, lower == "lag")))
        }
        "diff" => {
            if args.is_empty() || args.len() > 2 {
                return Err(TransformError::ArityMismatch { function: String::from("diff"), expected: String::from("1 or 2"), actual: args.len() });
            }
            let vals = eval_column_expr(&args[0], ctx)?;
            let k = if args.len() == 2 { eval_nonneg_int_arg(&args[1], ctx, "diff")? } else { 1 };
            Ok(Some(eval_diff(&vals, k)))
        }
        "rolling_mean" | "moving_avg" | "rolling_sum" => {
            check_arity(lower, args, 2)?;
            let vals = eval_column_expr(&args[0], ctx)?;
            let w = eval_nonneg_int_arg(&args[1], ctx, lower)?;
            if w == 0 {
                return Err(TransformError::InvalidSpec { message: format!("`{lower}` window width must be >= 1") });
            }
            Ok(Some(eval_rolling(&vals, w, lower != "rolling_sum")))
        }
        "zscore" => {
            check_arity("zscore", args, 1)?;
            let vals = eval_column_expr(&args[0], ctx)?;
            Ok(Some(eval_zscore(&vals)))
        }
        "rank" | "dense_rank" => {
            check_arity(lower, args, 1)?;
            let vals = eval_column_expr(&args[0], ctx)?;
            Ok(Some(eval_rank(&vals, lower == "dense_rank")))
        }
        _ => Ok(None),
    }
}

fn eval_shift(vals: &[FormulaValue], k: usize, is_lag: bool) -> Vec<FormulaValue> {
    let n = vals.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let src = if is_lag { i.checked_sub(k) } else { i.checked_add(k).filter(|&idx| idx < n) };
        out.push(src.and_then(|idx| vals.get(idx)).cloned().unwrap_or(FormulaValue::Missing));
    }
    out
}

fn eval_diff(vals: &[FormulaValue], k: usize) -> Vec<FormulaValue> {
    let n = vals.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let Some(prev_idx) = i.checked_sub(k) else {
            out.push(FormulaValue::Missing);
            continue;
        };
        match (vals[i].as_finite_f64(), vals[prev_idx].as_finite_f64()) {
            (Some(cur), Some(prev)) => out.push(FormulaValue::from_f64(cur - prev)),
            _ => out.push(FormulaValue::Missing),
        }
    }
    out
}

fn eval_rolling(vals: &[FormulaValue], window: usize, as_mean: bool) -> Vec<FormulaValue> {
    let n = vals.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if i + 1 < window || vals[i].as_finite_f64().is_none() {
            out.push(FormulaValue::Missing);
            continue;
        }
        let start = i + 1 - window;
        let slice_nums: Vec<f64> = vals[start..=i].iter().filter_map(FormulaValue::as_finite_f64).collect();
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

fn eval_zscore(vals: &[FormulaValue]) -> Vec<FormulaValue> {
    let nums = extract_finite_numbers(vals);
    let Ok(moments) = sum::SummaryMoments::from_slice(&nums) else {
        return vec![FormulaValue::Missing; vals.len()];
    };
    let Ok(sd) = moments.sample_sd() else {
        return vec![FormulaValue::Missing; vals.len()];
    };
    if sd == 0.0 || !sd.is_finite() {
        return vec![FormulaValue::Missing; vals.len()];
    }
    vals.iter()
        .map(|v| match v.as_finite_f64() {
            Some(x) => FormulaValue::from_f64((x - moments.mean) / sd),
            None => FormulaValue::Missing,
        })
        .collect()
}

fn eval_rank(vals: &[FormulaValue], dense: bool) -> Vec<FormulaValue> {
    let mut indexed: Vec<(usize, f64)> = vals.iter().enumerate().filter_map(|(i, v)| v.as_finite_f64().map(|x| (i, x))).collect();
    indexed.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut out = vec![FormulaValue::Missing; vals.len()];
    let mut pos = 0;
    let mut dense_level = 1_usize;
    while pos < indexed.len() {
        let mut end = pos + 1;
        while end < indexed.len() && indexed[end].1 == indexed[pos].1 {
            end += 1;
        }
        #[allow(clippy::cast_precision_loss)]
        let rank_val = if dense { dense_level as f64 } else { ((pos + 1 + end) as f64) * 0.5 };
        for &(orig_idx, _) in &indexed[pos..end] {
            out[orig_idx] = FormulaValue::Number(rank_val);
        }
        pos = end;
        dense_level += 1;
    }
    out
}

fn eval_scalar_call(lower: &str, args: &[Expr], ctx: &dyn FormulaContext) -> Result<Vec<FormulaValue>, TransformError> {
    let n = ctx.row_count();
    let evaled: Vec<Vec<FormulaValue>> = args.iter().map(|a| eval_column_expr(a, ctx)).collect::<Result<_, _>>()?;
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
            let b = cols[0][row].as_bool();
            return Ok(b.map_or(FormulaValue::Missing, |flag| FormulaValue::Number(if flag { 0.0 } else { 1.0 })));
        }
        _ => {}
    }

    let Some(x) = cols[0][row].as_finite_f64() else {
        return Ok(FormulaValue::Missing);
    };
    if cols.len() == 1 {
        return Ok(FormulaValue::from_f64(eval_unary_math(lower, x)));
    }
    let Some(y) = cols[1][row].as_finite_f64() else {
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
    let Some(z) = cols[2][row].as_finite_f64() else {
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
    let nums: Vec<f64> = cols.iter().filter_map(|col| col[row].as_finite_f64()).collect();
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
        let Some(v) = col[row].as_finite_f64() else {
            return FormulaValue::Missing;
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
    let mut acc = is_and;
    for col in cols {
        let Some(b) = col[row].as_bool() else {
            return FormulaValue::Missing;
        };
        acc = if is_and { acc && b } else { acc || b };
    }
    FormulaValue::Number(if acc { 1.0 } else { 0.0 })
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
