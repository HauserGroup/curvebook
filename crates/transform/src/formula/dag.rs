//! Topological dependency DAG, cycle detection, and deterministic evaluation of calculated
//! variables in `MultipleVariablesTable` (`docs/implementation/data-tables-and-transforms.md` §4).

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use curvebook_doc::{DataSheet, InfoSheet, MultipleVariablesTable, TableArchetype, VariableRole, VariableSource, default_column_label};

use crate::error::TransformError;
use crate::formula::eval::{MultipleVariablesContext, eval_column_expr, is_builtin_constant, resolve_mv_column};
use crate::formula::parser::{Expr, FrameBound, VarRef, parse_formula};

/// Referenced dependency target extracted statically from a formula AST.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyTarget {
    /// Column referenced by identifier or bracketed/quoted title (for example `[Body Mass]` or `col("Dose")`).
    ColumnName(String),
    /// Column referenced by explicit zero-based index (`col(0)`).
    ColumnIndex(usize),
    /// Metadata constant referenced on the paired `InfoSheet` (`Info("Key")`).
    InfoConstant(String),
}

/// Extract all static column and `InfoSheet` references from an [`Expr`].
#[must_use]
pub fn extract_dependencies(expr: &Expr) -> Vec<DependencyTarget> {
    let mut targets = BTreeSet::new();
    collect_deps(expr, &mut targets);
    targets.into_iter().collect()
}

fn push_frame_bound_refs<'a>(bound: &'a FrameBound, stack: &mut Vec<&'a Expr>) {
    match bound {
        FrameBound::Preceding(e) | FrameBound::Following(e) => stack.push(e),
        FrameBound::UnboundedPreceding | FrameBound::CurrentRow | FrameBound::UnboundedFollowing => {}
    }
}

fn walk_expr_vars<'a>(expr: &'a Expr, mut visit_var: impl FnMut(&'a VarRef)) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Number(_) | Expr::Text(_) => {}
            Expr::Var(vref) => visit_var(vref),
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
            Expr::WindowCall { args, filter, window, .. } => {
                for arg in args {
                    stack.push(arg);
                }
                if let Some(f) = filter {
                    stack.push(f);
                }
                if let Some(w) = window {
                    for p in &w.partition_by {
                        stack.push(p);
                    }
                    for o in &w.order_by {
                        stack.push(&o.expr);
                    }
                    if let Some(frame) = &w.frame {
                        push_frame_bound_refs(&frame.start, &mut stack);
                        push_frame_bound_refs(&frame.end, &mut stack);
                    }
                }
            }
        }
    }
}

fn collect_deps(expr: &Expr, out: &mut BTreeSet<DependencyTarget>) {
    walk_expr_vars(expr, |vref| match vref {
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
    });
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
    let mut parsed_cols: Vec<usize> = Vec::new();
    let mut is_calc = vec![false; n_cols];
    let mut expr_by_col: Vec<Option<Expr>> = vec![None; n_cols];

    if let Some((target_idx, expr)) = override_col {
        is_calc[target_idx] = true;
        expr_by_col[target_idx] = Some(expr);
        parsed_cols.push(target_idx);
    }

    for (idx, var) in table.variables.iter().enumerate() {
        if is_calc[idx] {
            continue;
        }
        if let VariableSource::Calculated { formula } = &var.source {
            let expr = parse_formula(formula)?;
            is_calc[idx] = true;
            expr_by_col[idx] = Some(expr);
            parsed_cols.push(idx);
        }
    }

    if parsed_cols.is_empty() {
        return Ok((Vec::new(), expr_by_col));
    }

    let (dependents, mut in_degree) = build_dependency_graph(table, &parsed_cols, &expr_by_col, &is_calc)?;
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

/// Freeze a calculated variable column in a `MultipleVariables` [`DataSheet`] into directly entered
/// values (`VariableSource::Entered`), detaching its formula while preserving evaluated cells,
/// exclusion flags, and inferred column role.
///
/// Returns `Ok(Some(previous_formula))` when the column was calculated, or `Ok(None)` if it was
/// already manually entered.
pub fn unlink_calculated_variable(sheet: &mut DataSheet, col: usize) -> Result<Option<String>, TransformError> {
    sheet.ensure_mutable()?;
    let TableArchetype::MultipleVariables(mv) = &mut sheet.archetype else {
        return Err(TransformError::IncompatibleTable { operation: "unlink_calculated_variable", kind: sheet.archetype.kind() });
    };
    let n_cols = mv.variables.len();
    let Some(var) = mv.variables.get_mut(col) else {
        return Err(TransformError::InvalidSpec { message: format!("column index {col} out of bounds for table with {n_cols} variables") });
    };
    let previous = match std::mem::replace(&mut var.source, VariableSource::Entered) {
        VariableSource::Calculated { formula } => Some(formula),
        VariableSource::Entered => None,
    };
    if previous.is_some() || var.role == VariableRole::Calculated {
        var.role = VariableRole::Continuous;
        var.role_inferred = true;
        var.refresh_inferred_role();
    }
    Ok(previous)
}

fn col_display_name(table: &MultipleVariablesTable, idx: usize) -> String {
    let title = &table.variables[idx].title;
    if title.trim().is_empty() { default_column_label(idx) } else { title.clone() }
}

fn build_dependency_graph(
    table: &MultipleVariablesTable,
    parsed_cols: &[usize],
    expr_by_col: &[Option<Expr>],
    is_calc: &[bool],
) -> Result<(Vec<Vec<usize>>, Vec<usize>), TransformError> {
    let n_cols = table.variables.len();
    let mut dependents = vec![Vec::new(); n_cols];
    let mut in_degree = vec![0_usize; n_cols];

    for &col_idx in parsed_cols {
        let Some(expr) = expr_by_col.get(col_idx).and_then(Option::as_ref) else {
            continue;
        };
        let deps = resolve_col_deps(table, expr);
        for dep in deps {
            if dep == col_idx {
                let name = col_display_name(table, col_idx);
                return Err(TransformError::CircularDependency { cycle: vec![name.clone(), name] });
            }
            if is_calc[dep] && !dependents[dep].contains(&col_idx) {
                dependents[dep].push(col_idx);
                in_degree[col_idx] += 1;
            }
        }
    }

    Ok((dependents, in_degree))
}

fn resolve_col_deps(table: &MultipleVariablesTable, expr: &Expr) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    walk_expr_vars(expr, |vref| match vref {
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
    });
    out
}

fn topo_sort_columns(
    table: &MultipleVariablesTable,
    parsed_cols: &[usize],
    dependents: &[Vec<usize>],
    in_degree: &mut [usize],
) -> Result<Vec<usize>, TransformError> {
    let mut ready = BinaryHeap::new();
    for &col_idx in parsed_cols {
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
        let remaining: Vec<usize> = parsed_cols.iter().copied().filter(|&c| in_degree[c] > 0).collect();
        let cycle = find_cycle_names(table, dependents, &remaining);
        return Err(TransformError::CircularDependency { cycle });
    }

    Ok(order)
}

fn find_cycle_names(table: &MultipleVariablesTable, dependents: &[Vec<usize>], remaining: &[usize]) -> Vec<String> {
    let mut state = vec![0_u8; table.variables.len()];
    for &start in remaining {
        if state[start] == 0
            && let Some(cycle_idxs) = dfs_cycle(start, dependents, &mut state)
        {
            return cycle_idxs.into_iter().map(|i| col_display_name(table, i)).collect();
        }
    }
    remaining.iter().map(|&i| col_display_name(table, i)).collect()
}

/// Iterative three-color DFS cycle finder using an explicit `(node, next_edge_idx)` stack.
fn dfs_cycle(start: usize, adj: &[Vec<usize>], state: &mut [u8]) -> Option<Vec<usize>> {
    let mut frames: Vec<(usize, usize)> = vec![(start, 0)];
    let mut path: Vec<usize> = vec![start];
    state[start] = 1;

    while let Some((node, edge_idx)) = frames.last_mut() {
        let cur = *node;
        if let Some(&next) = adj[cur].get(*edge_idx) {
            *edge_idx += 1;
            if state[next] == 1 {
                let pos = path.iter().position(|&x| x == next).unwrap_or(0);
                let mut cycle = path[pos..].to_vec();
                cycle.push(next);
                return Some(cycle);
            }
            if state[next] == 0 {
                state[next] = 1;
                path.push(next);
                frames.push((next, 0));
            }
        } else {
            let _ = frames.pop();
            let _ = path.pop();
            state[cur] = 2;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, MultipleVariablesTable};

    use super::{evaluate_calculated_variables, validate_no_formula_cycle};
    use crate::error::TransformError;

    #[test]
    fn aud_04_02_dag_handles_deep_ast_and_long_cycle_iteratively() {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let mut mv = MultipleVariablesTable::new(2, 1).unwrap();
                (mv.variables[0].title, mv.variables[1].title) = (String::from("X"), String::from("Sum"));
                mv.variables[0].cells[0] = Cell::number(1.0);
                let deep = format!("={}", vec!["1"; 8_000].join("+"));
                mv.variables[1].set_calculated_formula(&deep);
                validate_no_formula_cycle(&mv, 1, &deep).unwrap();
                evaluate_calculated_variables(&mut mv, None).unwrap();
                assert_eq!(mv.variables[1].cells[0].active_number(), Some(8_000.0));

                let n = 600_usize;
                let mut cyc = MultipleVariablesTable::new(n, 1).unwrap();
                for i in 0..n {
                    cyc.variables[i].title = format!("V{i}");
                    cyc.variables[i].set_calculated_formula(&format!("=[V{}] + 1", (i + n - 1) % n));
                }
                let TransformError::CircularDependency { cycle } = evaluate_calculated_variables(&mut cyc, None).unwrap_err() else {
                    panic!("expected CircularDependency")
                };
                assert_eq!(cycle.len(), n + 1);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
