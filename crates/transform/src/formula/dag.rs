//! Topological dependency DAG, cycle detection, and deterministic evaluation of calculated
//! variables in `MultipleVariablesTable` (`docs/implementation/data-tables-and-transforms.md` §4).

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use curvebook_doc::{DataSheet, InfoSheet, MultipleVariablesTable, TableArchetype, VariableSource, default_column_label};

use crate::error::TransformError;
use crate::formula::eval::{MultipleVariablesContext, eval_column_expr, is_builtin_constant, resolve_mv_column};
use crate::formula::parser::{Expr, VarRef, parse_formula};

/// Referenced dependency target extracted statically from a formula AST.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyTarget {
    ColumnName(String),
    ColumnIndex(usize),
    InfoConstant(String),
}

/// Extract all static column and `InfoSheet` references from an [`Expr`].
#[must_use]
pub fn extract_dependencies(expr: &Expr) -> Vec<DependencyTarget> {
    let mut targets = BTreeSet::new();
    collect_deps(expr, &mut targets);
    targets.into_iter().collect()
}

fn collect_deps(expr: &Expr, out: &mut BTreeSet<DependencyTarget>) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Number(_) | Expr::Text(_) => {}
            Expr::Var(vref) => match vref {
                VarRef::Ident(name) => {
                    if !is_builtin_constant(name) {
                        out.insert(DependencyTarget::ColumnName(name.clone()));
                    }
                }
                VarRef::Bracketed(name) | VarRef::ColumnByName(name) => {
                    out.insert(DependencyTarget::ColumnName(name.clone()));
                }
                VarRef::ColumnByIndex(idx) => {
                    out.insert(DependencyTarget::ColumnIndex(*idx));
                }
                VarRef::InfoConstant(key) => {
                    out.insert(DependencyTarget::InfoConstant(key.clone()));
                }
                VarRef::RowOneBased | VarRef::IndexZeroBased => {}
            },
            Expr::Unary { expr: inner, .. } => stack.push(inner),
            Expr::Binary { left, right, .. } => {
                stack.push(left);
                stack.push(right);
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    stack.push(arg);
                }
            }
        }
    }
}

/// Validate that assigning `formula` to column `target_col` in `mv` introduces neither
/// direct self-references nor multi-hop dependency cycles across calculated variables.
pub fn validate_no_formula_cycle(mv: &MultipleVariablesTable, target_col: usize, formula: &str) -> Result<(), TransformError> {
    let n_cols = mv.variables.len();
    if target_col >= n_cols {
        return Err(TransformError::InvalidSpec {
            message: format!("target column index {target_col} out of bounds for table with {n_cols} variables"),
        });
    }
    let target_expr = parse_formula(formula)?;
    let _ = plan_calculated_variables(mv, Some((target_col, target_expr)))?;
    Ok(())
}

/// Evaluate all `VariableSource::Calculated` columns in `table` in deterministic topological order.
///
/// Rejects self-references and multi-column dependency cycles with [`TransformError::CircularDependency`].
/// Returns the evaluated column indices in topological execution order.
pub fn evaluate_calculated_variables(table: &mut MultipleVariablesTable, info_sheet: Option<&InfoSheet>) -> Result<Vec<usize>, TransformError> {
    let (order, expr_by_col) = plan_calculated_variables(table, None)?;
    if order.is_empty() {
        return Ok(Vec::new());
    }

    for &col_idx in &order {
        let Some(expr) = expr_by_col.get(col_idx).and_then(Option::as_ref) else {
            continue;
        };
        let evaluated = {
            let ctx = MultipleVariablesContext { table, info_sheet };
            eval_column_expr(expr, &ctx)?
        };
        if let Some(var) = table.variables.get_mut(col_idx) {
            let prev_excluded: Vec<bool> = var.cells.iter().map(|c| c.excluded).collect();
            var.cells = evaluated
                .into_iter()
                .enumerate()
                .map(|(r, val)| {
                    let mut cell = val.into_cell();
                    if prev_excluded.get(r).copied().unwrap_or(false) {
                        cell.excluded = true;
                    }
                    cell
                })
                .collect();
        }
    }

    Ok(order)
}

fn plan_calculated_variables(
    table: &MultipleVariablesTable,
    override_col: Option<(usize, Expr)>,
) -> Result<(Vec<usize>, Vec<Option<Expr>>), TransformError> {
    let n_cols = table.variables.len();
    let mut parsed_cols: Vec<(usize, Expr)> = Vec::new();
    let mut is_calc = vec![false; n_cols];
    let mut expr_by_col: Vec<Option<Expr>> = vec![None; n_cols];

    if let Some((target_idx, expr)) = override_col {
        is_calc[target_idx] = true;
        expr_by_col[target_idx] = Some(expr.clone());
        parsed_cols.push((target_idx, expr));
    }

    for (idx, var) in table.variables.iter().enumerate() {
        if is_calc[idx] {
            continue;
        }
        if let VariableSource::Calculated { formula } = &var.source {
            let expr = parse_formula(formula)?;
            is_calc[idx] = true;
            expr_by_col[idx] = Some(expr.clone());
            parsed_cols.push((idx, expr));
        }
    }

    if parsed_cols.is_empty() {
        return Ok((Vec::new(), expr_by_col));
    }

    let (dependents, mut in_degree) = build_dependency_graph(table, &parsed_cols, &is_calc)?;
    let order = topo_sort_columns(table, &parsed_cols, &dependents, &mut in_degree)?;
    Ok((order, expr_by_col))
}

/// Evaluate calculated variables on a [`DataSheet`] if its archetype is `MultipleVariables`.
pub fn evaluate_data_sheet_calculated_variables(sheet: &mut DataSheet, info_sheet: Option<&InfoSheet>) -> Result<Vec<usize>, TransformError> {
    sheet.ensure_mutable()?;
    match &mut sheet.archetype {
        TableArchetype::MultipleVariables(mv) => evaluate_calculated_variables(mv, info_sheet),
        _ => Ok(Vec::new()),
    }
}

fn col_display_name(table: &MultipleVariablesTable, idx: usize) -> String {
    let title = &table.variables[idx].title;
    if title.trim().is_empty() { default_column_label(idx) } else { title.clone() }
}

fn build_dependency_graph(
    table: &MultipleVariablesTable,
    parsed_cols: &[(usize, Expr)],
    is_calc: &[bool],
) -> Result<(Vec<Vec<usize>>, Vec<usize>), TransformError> {
    let n_cols = table.variables.len();
    let mut dependents = vec![Vec::new(); n_cols];
    let mut in_degree = vec![0_usize; n_cols];

    for (col_idx, expr) in parsed_cols {
        let deps = resolve_col_deps(table, expr);
        for dep in deps {
            if dep == *col_idx {
                let name = col_display_name(table, *col_idx);
                return Err(TransformError::CircularDependency { cycle: vec![name.clone(), name] });
            }
            if is_calc[dep] && !dependents[dep].contains(col_idx) {
                dependents[dep].push(*col_idx);
                in_degree[*col_idx] += 1;
            }
        }
    }

    Ok((dependents, in_degree))
}

fn resolve_col_deps(table: &MultipleVariablesTable, expr: &Expr) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Number(_) | Expr::Text(_) => {}
            Expr::Var(vref) => match vref {
                VarRef::Ident(name) => {
                    if !is_builtin_constant(name)
                        && let Some(idx) = crate::formula::eval::resolve_ident_mv_column(table, name)
                    {
                        out.insert(idx);
                    }
                }
                VarRef::Bracketed(name) | VarRef::ColumnByName(name) => {
                    if let Some(idx) = resolve_mv_column(table, name) {
                        out.insert(idx);
                    }
                }
                VarRef::ColumnByIndex(idx) => {
                    if *idx < table.variables.len() {
                        out.insert(*idx);
                    }
                }
                VarRef::InfoConstant(_) | VarRef::RowOneBased | VarRef::IndexZeroBased => {}
            },
            Expr::Unary { expr: inner, .. } => stack.push(inner),
            Expr::Binary { left, right, .. } => {
                stack.push(left);
                stack.push(right);
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    stack.push(arg);
                }
            }
        }
    }
    out
}

fn topo_sort_columns(
    table: &MultipleVariablesTable,
    parsed_cols: &[(usize, Expr)],
    dependents: &[Vec<usize>],
    in_degree: &mut [usize],
) -> Result<Vec<usize>, TransformError> {
    let mut ready = BinaryHeap::new();
    for &(col_idx, _) in parsed_cols {
        if in_degree[col_idx] == 0 {
            ready.push(Reverse(col_idx));
        }
    }

    let mut order = Vec::with_capacity(parsed_cols.len());
    while let Some(Reverse(col)) = ready.pop() {
        order.push(col);
        for &next in &dependents[col] {
            in_degree[next] = in_degree[next].saturating_sub(1);
            if in_degree[next] == 0 {
                ready.push(Reverse(next));
            }
        }
    }

    if order.len() < parsed_cols.len() {
        let remaining: Vec<usize> = parsed_cols.iter().map(|(c, _)| *c).filter(|c| in_degree[*c] > 0).collect();
        let cycle = find_cycle_names(table, dependents, &remaining);
        return Err(TransformError::CircularDependency { cycle });
    }

    Ok(order)
}

fn find_cycle_names(table: &MultipleVariablesTable, dependents: &[Vec<usize>], remaining: &[usize]) -> Vec<String> {
    let mut state = vec![0_u8; table.variables.len()];
    let mut stack = Vec::new();
    for &start in remaining {
        if state[start] == 0
            && let Some(cycle_idxs) = dfs_cycle(start, dependents, &mut state, &mut stack)
        {
            return cycle_idxs.into_iter().map(|i| col_display_name(table, i)).collect();
        }
    }
    remaining.iter().map(|&i| col_display_name(table, i)).collect()
}

fn dfs_cycle(node: usize, adj: &[Vec<usize>], state: &mut [u8], stack: &mut Vec<usize>) -> Option<Vec<usize>> {
    state[node] = 1;
    stack.push(node);
    for &next in &adj[node] {
        if state[next] == 1 {
            let pos = stack.iter().position(|&x| x == next).unwrap_or(0);
            let mut cycle = stack[pos..].to_vec();
            cycle.push(next);
            return Some(cycle);
        }
        if state[next] == 0
            && let Some(found) = dfs_cycle(next, adj, state, stack)
        {
            return Some(found);
        }
    }
    let _ = stack.pop();
    state[node] = 2;
    None
}
