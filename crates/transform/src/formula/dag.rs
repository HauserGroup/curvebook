//! Topological dependency DAG, cycle detection, and deterministic evaluation of calculated
//! variables in `MultipleVariablesTable` (`docs/data-tables-and-transforms.md` §4).

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use lumencraft_doc::{DataSheet, InfoSheet, MultipleVariablesTable, TableArchetype, VariableSource};

use crate::error::TransformError;
use crate::formula::eval::{MultipleVariablesContext, eval_column_expr, resolve_mv_column};
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
    match expr {
        Expr::Number(_) | Expr::Text(_) => {}
        Expr::Var(vref) => match vref {
            VarRef::Ident(name) | VarRef::Bracketed(name) | VarRef::ColumnByName(name) => {
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
        Expr::Unary { expr: inner, .. } => collect_deps(inner, out),
        Expr::Binary { left, right, .. } => {
            collect_deps(left, out);
            collect_deps(right, out);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_deps(arg, out);
            }
        }
    }
}

/// Evaluate all `VariableSource::Calculated` columns in `table` in deterministic topological order.
///
/// Rejects self-references and multi-column dependency cycles with [`TransformError::CircularDependency`].
/// Returns the evaluated column indices in topological execution order.
pub fn evaluate_calculated_variables(table: &mut MultipleVariablesTable, info_sheet: Option<&InfoSheet>) -> Result<Vec<usize>, TransformError> {
    let n_cols = table.variables.len();
    let mut parsed_cols: Vec<(usize, Expr)> = Vec::new();
    let mut is_calc = vec![false; n_cols];

    for (idx, var) in table.variables.iter().enumerate() {
        if let VariableSource::Calculated { formula } = &var.source {
            let expr = parse_formula(formula)?;
            parsed_cols.push((idx, expr));
            is_calc[idx] = true;
        }
    }

    if parsed_cols.is_empty() {
        return Ok(Vec::new());
    }

    let (dependents, mut in_degree) = build_dependency_graph(table, &parsed_cols, &is_calc)?;
    let order = topo_sort_columns(table, &parsed_cols, &dependents, &mut in_degree)?;

    let expr_by_col: Vec<Option<Expr>> = {
        let mut map = vec![None; n_cols];
        for (idx, expr) in parsed_cols {
            map[idx] = Some(expr);
        }
        map
    };

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

/// Evaluate calculated variables on a [`DataSheet`] if its archetype is `MultipleVariables`.
pub fn evaluate_data_sheet_calculated_variables(sheet: &mut DataSheet, info_sheet: Option<&InfoSheet>) -> Result<Vec<usize>, TransformError> {
    sheet.ensure_mutable()?;
    match &mut sheet.archetype {
        TableArchetype::MultipleVariables(mv) => evaluate_calculated_variables(mv, info_sheet),
        _ => Ok(Vec::new()),
    }
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
                let name = table.variables[*col_idx].title.clone();
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
    for target in extract_dependencies(expr) {
        match target {
            DependencyTarget::ColumnName(name) => {
                if let Some(idx) = resolve_mv_column(table, &name) {
                    out.insert(idx);
                }
            }
            DependencyTarget::ColumnIndex(idx) => {
                if idx < table.variables.len() {
                    out.insert(idx);
                }
            }
            DependencyTarget::InfoConstant(_) => {}
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
            return cycle_idxs.into_iter().map(|i| table.variables[i].title.clone()).collect();
        }
    }
    remaining.iter().map(|&i| table.variables[i].title.clone()).collect()
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
