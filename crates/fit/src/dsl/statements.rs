//! Line and section parsing: dataset prefixes, statements, equation branches, initial-value rules, and transforms.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::dsl::ast::{BinaryOp, CompiledInitRule, CompiledProgram, DatasetFilter, DslCompileWarning, DslEquationKind, DslWarningKind, Expr, OpCode};
use crate::dsl::compile::compile_target_resolved;
use crate::dsl::limits::{MAX_DATASET_CLASSES, MAX_DATASET_EXPANSION, MAX_TOTAL_OPCODES};
use crate::dsl::parser::{LocalResolver, parse_expr_collecting_locs};
use crate::error::FitError;
use crate::library::estimate_x_at_ymid;
use crate::types::DataPoint;

pub(in crate::dsl) enum ParsedStatement {
    OdeDerivative { rhs: Expr, uses_t_axis: bool },
    OdeInitial { lhs_name: Option<String>, t0: Option<f64>, rhs: Expr },
    Assign { name: String, rhs: Expr },
    ImplicitEquation { lhs: Expr, rhs: Expr },
    BareExpr(Expr),
}

pub(in crate::dsl) fn byte_offset_in_line(raw_line: &str, slice: &str) -> usize {
    let base = raw_line.as_ptr() as usize;
    let sub = slice.as_ptr() as usize;
    if sub >= base && sub <= base.saturating_add(raw_line.len()) { sub - base } else { 0 }
}

pub(in crate::dsl) fn strip_comment<'a>(line: &'a str, line_no: usize, hints: &mut HashMap<String, f64>) -> Result<&'a str, FitError> {
    let trimmed = line.trim();
    if let Some((before, after)) = trimmed.split_once('#').into_iter().chain(trimmed.split_once(';')).min_by_key(|(before, _)| before.len()) {
        let comment = after.trim();
        if let Some(rest) = comment.strip_prefix("initial:") {
            for pair in rest.split(',') {
                if let Some((k, v)) = pair.split_once('=') {
                    let key = k.trim();
                    if !key.is_empty() && is_builtin_constant_or_var(key) {
                        return Err(FitError::DslCompileError {
                            message: format!("Cannot assign `# initial:` hint to reserved identifier `{key}`"),
                            line: line_no,
                            column: 1,
                        });
                    }
                    if let Ok(num) = v.trim().parse::<f64>()
                        && num.is_finite()
                    {
                        hints.insert(key.to_owned(), num);
                    }
                }
            }
        }
        return Ok(before.trim());
    }
    Ok(trimmed)
}

pub(in crate::dsl) fn parse_dataset_prefix(line: &str, line_no: usize) -> Result<(DatasetFilter, &str), FitError> {
    let Some(after_open) = line.strip_prefix('<') else {
        return Ok((DatasetFilter::Any, line));
    };
    let Some((inside_raw, rest_raw)) = after_open.split_once('>') else {
        return Err(FitError::DslCompileError { message: "Unclosed `<...>` dataset prefix".to_owned(), line: line_no, column: 1 });
    };
    let inside = inside_raw.trim();
    let rest = rest_raw.trim();
    let (negated, body) = if let Some(s) = inside.strip_prefix('~') { (true, s.trim()) } else { (false, inside) };
    let mut indices = Vec::new();
    for part in body.split(',') {
        let p = part.trim();
        if let Some((start_s, end_s)) = p.split_once(':') {
            let s_idx = parse_ds_token(start_s.trim(), line_no)?;
            let e_idx = parse_ds_token(end_s.trim(), line_no)?;
            if s_idx <= e_idx {
                let count = e_idx.checked_sub(s_idx).and_then(|d| d.checked_add(1));
                if count.is_none_or(|c| c > MAX_DATASET_EXPANSION || indices.len().saturating_add(c) > MAX_DATASET_EXPANSION) {
                    return Err(FitError::DslCompileError {
                        message: format!("Dataset range `{p}` exceeds maximum expansion limit ({MAX_DATASET_EXPANSION})"),
                        line: line_no,
                        column: 1,
                    });
                }
                indices.extend(s_idx..=e_idx);
            }
        } else if !p.is_empty() {
            if indices.len() >= MAX_DATASET_EXPANSION {
                return Err(FitError::DslCompileError {
                    message: format!("Dataset selector exceeds maximum limit ({MAX_DATASET_EXPANSION})"),
                    line: line_no,
                    column: 1,
                });
            }
            indices.push(parse_ds_token(p, line_no)?);
        }
    }
    indices.sort_unstable();
    indices.dedup();
    let filter = if negated { DatasetFilter::Exclude(indices) } else { DatasetFilter::Include(indices) };
    Ok((filter, rest))
}

pub(in crate::dsl) fn parse_ds_token(tok: &str, line_no: usize) -> Result<usize, FitError> {
    if let Ok(idx) = tok.parse::<usize>() {
        return Ok(idx);
    }
    let upper = tok.to_ascii_uppercase();
    if upper.len() == 1 {
        let b = upper.as_bytes()[0];
        if b.is_ascii_uppercase() {
            return Ok((b - b'A') as usize);
        }
    }
    Err(FitError::DslCompileError { message: format!("Invalid dataset selector `{tok}`"), line: line_no, column: 1 })
}

pub(in crate::dsl) fn parse_statement(rest: &str, line_no: usize) -> Result<ParsedStatement, FitError> {
    parse_statement_with_locs(rest, line_no, 0, None)
}

pub(in crate::dsl) fn parse_statement_with_locs(
    rest: &str,
    line_no: usize,
    col_offset: usize,
    mut ident_locs: Option<&mut HashMap<String, (usize, usize)>>,
) -> Result<ParsedStatement, FitError> {
    // Detect `LHS = RHS` (without `==`) cleanly at top level.
    if let Some((lhs_s, rhs_s)) = split_top_level_assignment(rest) {
        let lhs_trim = lhs_s.trim();
        let lhs_compact: String = lhs_trim.chars().filter(|c| !c.is_whitespace()).collect();
        let lhs_upper = lhs_compact.to_ascii_uppercase();
        let rhs_offset = col_offset + byte_offset_in_line(rest, rhs_s);
        if lhs_upper == "DY/DX" || lhs_upper == "DY/DT" || lhs_upper == "D(Y,X)" || lhs_upper == "D(Y,T)" {
            let rhs = parse_expr_collecting_locs(rhs_s, line_no, rhs_offset, ident_locs.as_deref_mut())?;
            let uses_t_axis = lhs_upper == "DY/DT" || lhs_upper == "D(Y,T)";
            return Ok(ParsedStatement::OdeDerivative { rhs, uses_t_axis });
        }
        if lhs_upper == "Y0" {
            let rhs = parse_expr_collecting_locs(rhs_s, line_no, rhs_offset, ident_locs.as_deref_mut())?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: Some(lhs_trim.to_owned()), t0: Some(0.0), rhs });
        }
        if lhs_upper == "Y(0)" {
            let rhs = parse_expr_collecting_locs(rhs_s, line_no, rhs_offset, ident_locs.as_deref_mut())?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: None, t0: Some(0.0), rhs });
        }
        if let Some(inner) = lhs_upper.strip_prefix("Y(").and_then(|s| s.strip_suffix(')'))
            && let Ok(t0_val) = inner.parse::<f64>()
        {
            if !t0_val.is_finite() {
                return Err(FitError::DslCompileError {
                    message: "ODE initial time `Y(...)` must be finite".to_owned(),
                    line: line_no,
                    column: col_offset + 1,
                });
            }
            let rhs = parse_expr_collecting_locs(rhs_s, line_no, rhs_offset, ident_locs.as_deref_mut())?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: None, t0: Some(t0_val), rhs });
        }
        if is_valid_ident(lhs_trim) {
            let rhs = parse_expr_collecting_locs(rhs_s, line_no, rhs_offset, ident_locs.as_deref_mut())?;
            return Ok(ParsedStatement::Assign { name: lhs_trim.to_owned(), rhs });
        }
        let lhs_offset = col_offset + byte_offset_in_line(rest, lhs_trim);
        let lhs = parse_expr_collecting_locs(lhs_trim, line_no, lhs_offset, ident_locs.as_deref_mut())?;
        let rhs = parse_expr_collecting_locs(rhs_s, line_no, rhs_offset, ident_locs)?;
        return Ok(ParsedStatement::ImplicitEquation { lhs, rhs });
    }
    Ok(ParsedStatement::BareExpr(parse_expr_collecting_locs(rest, line_no, col_offset, ident_locs)?))
}

pub(in crate::dsl) fn split_top_level_assignment(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut depth = 0_i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'=' if depth == 0 => {
                let prev = if i > 0 { bytes[i - 1] } else { 0 };
                let next = bytes.get(i + 1).copied().unwrap_or(0);
                if prev != b'!'
                    && prev != b'<'
                    && prev != b'>'
                    && prev != b'='
                    && next != b'='
                    && let Some((lhs, after_eq)) = s.split_at_checked(i)
                    && let Some(rhs) = after_eq.strip_prefix('=')
                {
                    return Some((lhs, rhs));
                }
                if next == b'=' {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub(in crate::dsl) fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::dsl) enum DslSection {
    Equation,
    InitialValues,
    Transforms,
}

pub(in crate::dsl) const INIT_STAT_NAMES: [&str; 11] =
    ["YMIN", "YMAX", "XMIN", "XMAX", "XMID", "YMID", "XATYMIN", "XATYMAX", "XATY50", "SPAN", "SIGN"];

pub(in crate::dsl) type OutputBranch = (DatasetFilter, Vec<(String, Expr)>, Expr);
pub(in crate::dsl) type OdeInitBranch = (DatasetFilter, f64, Vec<(String, Expr)>, Expr);

pub(in crate::dsl) enum EqItem {
    Local { filter: DatasetFilter, name: String, rhs: Expr, line_no: usize },
    TargetExplicit { filter: DatasetFilter, expr: Expr, line_no: usize },
    TargetImplicitDiff { filter: DatasetFilter, diff: Expr, line_no: usize },
    TargetBare { filter: DatasetFilter, expr: Expr, line_no: usize },
    TargetOde { filter: DatasetFilter, rhs: Expr, line_no: usize },
    OdeInit { filter: DatasetFilter, t0: f64, rhs: Expr, line_no: usize },
}

impl EqItem {
    pub(in crate::dsl) fn filter(&self) -> &DatasetFilter {
        match self {
            Self::Local { filter, .. }
            | Self::TargetExplicit { filter, .. }
            | Self::TargetImplicitDiff { filter, .. }
            | Self::TargetBare { filter, .. }
            | Self::TargetOde { filter, .. }
            | Self::OdeInit { filter, .. } => filter,
        }
    }

    pub(in crate::dsl) fn line_no(&self) -> usize {
        match self {
            Self::Local { line_no, .. }
            | Self::TargetExplicit { line_no, .. }
            | Self::TargetImplicitDiff { line_no, .. }
            | Self::TargetBare { line_no, .. }
            | Self::TargetOde { line_no, .. }
            | Self::OdeInit { line_no, .. } => *line_no,
        }
    }

    pub(in crate::dsl) fn is_target_for_mode(&self, has_ode: bool) -> bool {
        if has_ode {
            matches!(self, Self::TargetOde { .. })
        } else {
            matches!(self, Self::TargetExplicit { .. } | Self::TargetImplicitDiff { .. } | Self::TargetBare { .. })
        }
    }

    pub(in crate::dsl) fn target_raw_expr(&self, has_ode: bool) -> Option<&Expr> {
        match (has_ode, self) {
            (true, Self::TargetOde { rhs, .. }) => Some(rhs),
            (false, Self::TargetExplicit { expr, .. } | Self::TargetBare { expr, .. }) => Some(expr),
            (false, Self::TargetImplicitDiff { diff, .. }) => Some(diff),
            _ => None,
        }
    }
}

pub(in crate::dsl) fn parse_section_header(trimmed: &str, line_no: usize) -> Result<Option<DslSection>, FitError> {
    let Some(stripped) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Ok(None);
    };
    let inner: String = stripped.chars().filter(|c| !c.is_ascii_whitespace() && *c != '_' && *c != '-').flat_map(char::to_lowercase).collect();
    match inner.as_str() {
        "equation" | "equations" | "body" | "model" => Ok(Some(DslSection::Equation)),
        "initialvalues" | "initialvalue" | "initials" | "init" => Ok(Some(DslSection::InitialValues)),
        "transforms" | "transform" | "derived" | "derivedscalars" => Ok(Some(DslSection::Transforms)),
        _ => Err(FitError::DslCompileError { message: format!("Unknown DSL section `{trimmed}`"), line: line_no, column: 1 }),
    }
}

pub(in crate::dsl) struct BuiltEqItems {
    pub(in crate::dsl) has_ode: bool,
    pub(in crate::dsl) uses_t_axis: bool,
    pub(in crate::dsl) has_ode_init: bool,
    pub(in crate::dsl) items: Vec<EqItem>,
}

pub(in crate::dsl) fn build_eq_items(parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)>) -> Result<BuiltEqItems, FitError> {
    let has_ode = parsed_lines.iter().any(|(_, _, s)| matches!(s, ParsedStatement::OdeDerivative { .. }));
    let uses_t_axis = parsed_lines.iter().any(|(_, _, s)| matches!(s, ParsedStatement::OdeDerivative { uses_t_axis: true, .. }));
    let mut has_ode_init = false;
    let mut items = Vec::with_capacity(parsed_lines.len());
    for (line_no, filter, stmt) in parsed_lines {
        match stmt {
            ParsedStatement::OdeDerivative { rhs, .. } => {
                items.push(EqItem::TargetOde { filter, rhs, line_no });
            }
            ParsedStatement::OdeInitial { lhs_name, t0, rhs } => {
                if has_ode {
                    has_ode_init = true;
                    items.push(EqItem::OdeInit { filter, t0: t0.unwrap_or(0.0), rhs, line_no });
                } else if let Some(name) = lhs_name {
                    // Non-ODE `Y0 = expr` is an ordinary local assignment rather than a hidden/discarded ODE IC.
                    items.push(EqItem::Local { filter, name, rhs, line_no });
                } else {
                    return Err(FitError::DslCompileError {
                        message: "ODE initial condition `Y(t0) = ...` requires an ODE derivative equation (`dY/dX = ...`)".to_owned(),
                        line: line_no,
                        column: 1,
                    });
                }
            }
            ParsedStatement::Assign { name, rhs } => {
                if name.eq_ignore_ascii_case("Y") {
                    items.push(EqItem::TargetExplicit { filter, expr: rhs, line_no });
                } else if is_builtin_constant_or_var(&name) && name != "t" {
                    // Uppercase `T` and all other built-in symbols (`X`, `E`, `PI`, `DS`, `INF`, `NAN`, ...)
                    // cannot be assigned as locals, whereas lowercase `t` may act as an explicit `[Equation]` temporary.
                    return Err(FitError::DslCompileError {
                        message: format!("Cannot assign to reserved built-in identifier `{name}`"),
                        line: line_no,
                        column: 1,
                    });
                } else {
                    items.push(EqItem::Local { filter, name, rhs, line_no });
                }
            }
            ParsedStatement::ImplicitEquation { lhs, rhs } => {
                let diff = Expr::Binary { op: BinaryOp::Sub, lhs: Box::new(lhs), rhs: Box::new(rhs) };
                items.push(EqItem::TargetImplicitDiff { filter, diff, line_no });
            }
            ParsedStatement::BareExpr(expr) => {
                items.push(EqItem::TargetBare { filter, expr, line_no });
            }
        }
    }
    Ok(BuiltEqItems { has_ode, uses_t_axis, has_ode_init, items })
}

pub(in crate::dsl) struct FilterClassPartition<'a> {
    pub(in crate::dsl) explicit_classes: Vec<DatasetFilter>,
    pub(in crate::dsl) outside_filter: DatasetFilter,
    pub(in crate::dsl) filter_ids: HashMap<&'a DatasetFilter, usize>,
    pub(in crate::dsl) matching_classes: Vec<Vec<usize>>,
}

pub(in crate::dsl) fn collect_filter_classes(eq_items: &[EqItem]) -> Result<FilterClassPartition<'_>, FitError> {
    let mut unique_filters: Vec<&DatasetFilter> = Vec::new();
    let mut filter_ids: HashMap<&DatasetFilter, usize> = HashMap::new();
    for item in eq_items {
        let f = item.filter();
        if *f != DatasetFilter::Any && !filter_ids.contains_key(f) {
            let fi = unique_filters.len();
            unique_filters.push(f);
            filter_ids.insert(f, fi);
        }
    }

    let mut events_by_ds: HashMap<usize, Vec<i32>> = HashMap::new();
    for (fi, &f) in unique_filters.iter().enumerate() {
        let tag = i32::try_from(fi + 1).map_err(|_| FitError::DslCompileError {
            message: "Too many distinct dataset filters".to_owned(),
            line: 1,
            column: 1,
        })?;
        match f {
            DatasetFilter::Any => {}
            DatasetFilter::Include(v) => {
                for &ds in v {
                    events_by_ds.entry(ds).or_default().push(tag);
                }
            }
            DatasetFilter::Exclude(v) => {
                for &ds in v {
                    events_by_ds.entry(ds).or_default().push(-tag);
                }
            }
        }
    }

    let mut mentioned: Vec<usize> = events_by_ds.keys().copied().collect();
    mentioned.sort_unstable();
    let outside_filter = if mentioned.is_empty() { DatasetFilter::Any } else { DatasetFilter::Exclude(mentioned) };

    let mut groups: HashMap<Vec<i32>, Vec<usize>> = HashMap::new();
    for (ds, ev) in events_by_ds {
        groups.entry(ev).or_default().push(ds);
    }
    let mut explicit_classes: Vec<(usize, DatasetFilter)> = groups
        .into_values()
        .map(|mut indices| {
            indices.sort_unstable();
            let rep = indices[0];
            (rep, DatasetFilter::Include(indices))
        })
        .collect();
    explicit_classes.sort_by_key(|(rep, _)| *rep);
    if explicit_classes.len() > MAX_DATASET_CLASSES {
        return Err(FitError::DslCompileError {
            message: format!("Distinct dataset equivalence classes exceed limit ({MAX_DATASET_CLASSES})"),
            line: 1,
            column: 1,
        });
    }

    let explicit_filters: Vec<DatasetFilter> = explicit_classes.into_iter().map(|(_, f)| f).collect();
    let num_explicit = explicit_filters.len();
    let total_classes = num_explicit + 1;

    let mut ds_to_class = HashMap::new();
    for (c_idx, f) in explicit_filters.iter().enumerate() {
        if let DatasetFilter::Include(v) = f {
            for &ds in v {
                ds_to_class.insert(ds, c_idx);
            }
        }
    }

    let mut matching_classes = vec![Vec::new(); unique_filters.len()];
    let mut stamp = vec![usize::MAX; total_classes];
    for (fi, &f) in unique_filters.iter().enumerate() {
        match f {
            DatasetFilter::Any => {}
            DatasetFilter::Include(v) => {
                for &ds in v {
                    if let Some(&c_idx) = ds_to_class.get(&ds)
                        && stamp[c_idx] != fi
                    {
                        stamp[c_idx] = fi;
                        matching_classes[fi].push(c_idx);
                    }
                }
            }
            DatasetFilter::Exclude(v) => {
                for &ds in v {
                    if let Some(&c_idx) = ds_to_class.get(&ds) {
                        stamp[c_idx] = fi;
                    }
                }
                for (c_idx, st) in stamp.iter().enumerate().take(total_classes) {
                    if *st != fi {
                        matching_classes[fi].push(c_idx);
                    }
                }
            }
        }
    }

    Ok(FilterClassPartition { explicit_classes: explicit_filters, outside_filter, filter_ids, matching_classes })
}

pub(in crate::dsl) fn target_for_resolved_kind(item: &EqItem, kind: DslEquationKind) -> Option<Expr> {
    match (kind, item) {
        (DslEquationKind::OdeFirstOrder, EqItem::TargetOde { rhs, .. }) => Some(rhs.clone()),
        (DslEquationKind::Implicit, EqItem::TargetImplicitDiff { diff, .. }) => Some(diff.clone()),
        (DslEquationKind::Implicit, EqItem::TargetExplicit { expr, .. }) => {
            Some(Expr::Binary { op: BinaryOp::Sub, lhs: Box::new(Expr::Ident("Y".to_owned())), rhs: Box::new(expr.clone()) })
        }
        (DslEquationKind::Implicit | DslEquationKind::Explicit, EqItem::TargetBare { expr, .. }) => Some(expr.clone()),
        (DslEquationKind::Explicit, EqItem::TargetExplicit { expr, .. }) => Some(expr.clone()),
        _ => None,
    }
}

fn expr_depends_on_y_or_locals(expr: &Expr, y_locals: &HashSet<String>) -> bool {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(n) => {
                if n.eq_ignore_ascii_case("Y") || y_locals.contains(n) {
                    return true;
                }
            }
            Expr::Unary { rhs, .. } => stack.push(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            Expr::Call { args, .. } => stack.extend(args.iter()),
            Expr::If { cond, then_br, else_br } => {
                stack.push(else_br);
                stack.push(then_br);
                stack.push(cond);
            }
        }
    }
    false
}

/// Returns `true` when `target_expr` references `Y` directly or transitively through `locals` (#44).
fn branch_target_depends_on_y(locals: &[(String, Expr)], target_expr: &Expr) -> bool {
    let mut y_locals: HashSet<String> = HashSet::new();
    for (name, rhs) in locals {
        if expr_depends_on_y_or_locals(rhs, &y_locals) {
            y_locals.insert(name.clone());
        } else {
            y_locals.remove(name);
        }
    }
    expr_depends_on_y_or_locals(target_expr, &y_locals)
}

pub(in crate::dsl) fn branch_uses_unbound_scoped_indices(
    eq_items: &[EqItem],
    local_indices: &[usize],
    target_idx: usize,
    has_ode: bool,
    scoped_names: &HashSet<String>,
) -> bool {
    let Some(target) = eq_items[target_idx].target_raw_expr(has_ode) else {
        return false;
    };
    let declared_scoped: HashSet<&str> = scoped_names.iter().map(String::as_str).collect();
    let mut free = Vec::new();
    let mut seen = BTreeSet::new();
    let mut bound = BTreeSet::new();
    for &idx in local_indices {
        if let EqItem::Local { name, rhs, .. } = &eq_items[idx] {
            collect_free_idents_with_declared_locals(rhs, &bound, &declared_scoped, &mut seen, &mut free);
            bound.insert(name.clone());
        }
    }
    collect_free_idents_with_declared_locals(target, &bound, &declared_scoped, &mut seen, &mut free);
    free.iter().any(|ident| scoped_names.contains(ident))
}

fn collect_all_expr_idents(expr: &Expr, out: &mut HashSet<String>) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(n) => {
                out.insert(n.clone());
            }
            Expr::Unary { rhs, .. } => stack.push(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            Expr::Call { args, .. } => stack.extend(args.iter()),
            Expr::If { cond, then_br, else_br } => {
                stack.push(else_br);
                stack.push(then_br);
                stack.push(cond);
            }
        }
    }
}

/// Keep only the minimal backward-live prefix of `locals` needed to evaluate `rhs`,
/// preserving assignment order and shadowing semantics.
fn needed_locals_for_expr(locals: &[(String, Expr)], rhs: &Expr) -> Vec<(String, Expr)> {
    let mut needed = HashSet::new();
    collect_all_expr_idents(rhs, &mut needed);
    let mut keep = vec![false; locals.len()];
    for idx in (0..locals.len()).rev() {
        let (name, expr) = &locals[idx];
        if needed.contains(name) {
            keep[idx] = true;
            needed.remove(name);
            collect_all_expr_idents(expr, &mut needed);
        }
    }
    locals.iter().zip(keep).filter_map(|(pair, k)| if k { Some(pair.clone()) } else { None }).collect()
}

struct BranchValidationCtx<'a> {
    eq_items: &'a [EqItem],
    has_ode: bool,
    scoped_names: &'a HashSet<String>,
    last_any_target_idx: Option<usize>,
}

fn validate_branch_locals_and_trailing(
    ctx: &BranchValidationCtx<'_>,
    all_class_locals: &[usize],
    loc_len: usize,
    target_idx: usize,
    skip_scoped_check: bool,
    target_is_specific: bool,
) -> Result<Vec<(String, Expr)>, FitError> {
    let eq_items = ctx.eq_items;
    let is_overridden_any_local = |pos: usize, idx: usize| -> bool {
        pos >= loc_len
            && target_is_specific
            && *eq_items[idx].filter() == DatasetFilter::Any
            && ctx.last_any_target_idx.is_some_and(|t_idx| idx < t_idx)
    };

    if let Some(trailing_slice) = all_class_locals.get(loc_len..) {
        for (offset, &idx) in trailing_slice.iter().enumerate() {
            if !is_overridden_any_local(loc_len + offset, idx)
                && let EqItem::Local { name, line_no, .. } = &eq_items[idx]
            {
                return Err(FitError::DslCompileError {
                    message: format!("Local variable `{name}` assigned after equation target; move local assignments above the equation line"),
                    line: *line_no,
                    column: 1,
                });
            }
        }
    }

    let branch_local_names: HashSet<&str> = all_class_locals
        .iter()
        .enumerate()
        .filter_map(|(pos, &idx)| {
            if is_overridden_any_local(pos, idx) {
                return None;
            }
            match &eq_items[idx] {
                EqItem::Local { name, .. } => Some(name.as_str()),
                _ => None,
            }
        })
        .collect();
    let mut declared_locals = branch_local_names.clone();
    if !skip_scoped_check {
        declared_locals.extend(ctx.scoped_names.iter().map(String::as_str));
    }

    let mut bound = BTreeSet::new();
    let mut materialized = Vec::with_capacity(loc_len);
    for &idx in &all_class_locals[..loc_len] {
        if let EqItem::Local { name, rhs, line_no, .. } = &eq_items[idx] {
            let mut free = Vec::new();
            let mut seen = BTreeSet::new();
            collect_free_idents_with_declared_locals(rhs, &bound, &declared_locals, &mut seen, &mut free);
            for ident in &free {
                if branch_local_names.contains(ident.as_str()) {
                    return Err(FitError::DslCompileError {
                        message: format!("Local variable `{ident}` referenced before assignment; assign `{ident}` before using it"),
                        line: *line_no,
                        column: 1,
                    });
                }
                if !skip_scoped_check && ctx.scoped_names.contains(ident) {
                    return Err(FitError::DslCompileError {
                        message: format!("Local variable `{ident}` is out of scope for this dataset branch"),
                        line: *line_no,
                        column: 1,
                    });
                }
            }
            bound.insert(name.clone());
            materialized.push((name.clone(), rhs.clone()));
        }
    }

    if let Some(target_expr) = eq_items[target_idx].target_raw_expr(ctx.has_ode) {
        let target_line = eq_items[target_idx].line_no();
        let mut free = Vec::new();
        let mut seen = BTreeSet::new();
        collect_free_idents_with_declared_locals(target_expr, &bound, &declared_locals, &mut seen, &mut free);
        for ident in &free {
            if branch_local_names.contains(ident.as_str()) {
                return Err(FitError::DslCompileError {
                    message: format!("Local variable `{ident}` referenced before assignment; assign `{ident}` before using it"),
                    line: target_line,
                    column: 1,
                });
            }
            if !skip_scoped_check && ctx.scoped_names.contains(ident) {
                return Err(FitError::DslCompileError {
                    message: format!("Local variable `{ident}` is out of scope for this dataset branch"),
                    line: target_line,
                    column: 1,
                });
            }
        }
    }

    Ok(materialized)
}

type ResolvedOdeInit = (f64, Vec<(String, Expr)>, Expr);

fn resolve_ode_init_for_class(
    eq_items: &[EqItem],
    preceding_local_indices: &[usize],
    init_item_idx: usize,
    all_local_names: &HashSet<&str>,
) -> Result<ResolvedOdeInit, FitError> {
    let EqItem::OdeInit { t0, rhs, line_no, .. } = &eq_items[init_item_idx] else {
        return Err(FitError::DslCompileError { message: "Invalid ODE initial condition item".to_owned(), line: 1, column: 1 });
    };
    let all_prior_locals: Vec<(String, Expr)> = preceding_local_indices
        .iter()
        .filter_map(|&idx| match &eq_items[idx] {
            EqItem::Local { name, rhs, .. } => Some((name.clone(), rhs.clone())),
            _ => None,
        })
        .collect();
    let live_locals = needed_locals_for_expr(&all_prior_locals, rhs);

    let mut bound = BTreeSet::new();
    for (name, lrhs) in &live_locals {
        if expr_references_y(lrhs) {
            return Err(FitError::DslCompileError {
                message: format!("ODE initial condition cannot depend on `Y` (transitively via `{name}`)"),
                line: *line_no,
                column: 1,
            });
        }
        let mut free = Vec::new();
        let mut seen = BTreeSet::new();
        collect_free_idents_with_declared_locals(lrhs, &bound, all_local_names, &mut seen, &mut free);
        for ident in &free {
            if all_local_names.contains(ident.as_str()) {
                return Err(FitError::DslCompileError {
                    message: format!(
                        "ODE initial condition references out-of-scope or later-defined local `{ident}`; assign `{ident}` before the initial condition in this dataset scope"
                    ),
                    line: *line_no,
                    column: 1,
                });
            }
        }
        bound.insert(name.clone());
    }
    if expr_references_y(rhs) {
        return Err(FitError::DslCompileError { message: "ODE initial condition cannot depend on `Y`".to_owned(), line: *line_no, column: 1 });
    }
    let mut free = Vec::new();
    let mut seen = BTreeSet::new();
    collect_free_idents_with_declared_locals(rhs, &bound, all_local_names, &mut seen, &mut free);
    for ident in &free {
        if all_local_names.contains(ident.as_str()) {
            return Err(FitError::DslCompileError {
                message: format!(
                    "ODE initial condition references out-of-scope or later-defined local `{ident}`; assign `{ident}` before the initial condition in this dataset scope"
                ),
                line: *line_no,
                column: 1,
            });
        }
    }

    Ok((*t0, live_locals, rhs.clone()))
}

fn reserved_ident_info(raw: &str) -> Option<(String, &'static str)> {
    let upper = raw.to_ascii_uppercase();
    let desc = match upper.as_str() {
        "PI" => "built-in constant pi",
        "E" => "built-in constant e",
        "LN2" => "built-in constant ln(2)",
        "LN10" => "built-in constant ln(10)",
        "SQRT2" => "built-in constant sqrt(2)",
        "INF" => "IEEE-754 +Infinity",
        "NAN" => "IEEE-754 NaN",
        "T" => "predictor variable X (x[0] alias)",
        "DS" | "DATASET" => "0-based dataset index",
        _ if parse_predictor_index(&upper).is_some() => "predictor column X_k",
        _ => return None,
    };
    Some((upper, desc))
}

fn collect_dsl_warnings(ident_locs: &HashMap<String, (usize, usize)>, local_names: &HashSet<&str>, uses_t_axis: bool) -> Vec<DslCompileWarning> {
    let uses_x_predictor = ident_locs.keys().any(|k| k.eq_ignore_ascii_case("X"));
    let mut warnings = Vec::new();
    for (raw, &(line, column)) in ident_locs {
        if local_names.contains(raw.as_str()) || raw.eq_ignore_ascii_case("X") || raw.eq_ignore_ascii_case("Y") {
            continue;
        }
        let Some((upper, resolves_to)) = reserved_ident_info(raw) else {
            continue;
        };
        if upper == "T" && uses_t_axis && !uses_x_predictor {
            continue;
        }
        let (kind, message) = if upper == "T" && uses_x_predictor {
            (
                DslWarningKind::AliasTWithPredictorX,
                format!(
                    "Identifier `{raw}` is a reserved alias for predictor `X` (x[0]) alongside `X`; use a different parameter name if not meant as `X`"
                ),
            )
        } else if raw != &upper {
            (
                DslWarningKind::NonCanonicalReservedCase,
                format!("Identifier `{raw}` resolves case-insensitively to reserved symbol `{upper}` ({resolves_to}), not a fitted parameter"),
            )
        } else {
            (
                DslWarningKind::ReservedIdentifierUsed,
                format!("Identifier `{raw}` is reserved ({resolves_to}) and will not be treated as a fitted parameter"),
            )
        };
        warnings.push(DslCompileWarning { kind, identifier: raw.clone(), resolves_to, line, column, message });
    }
    warnings.sort_by(|a, b| (a.line, a.column, a.identifier.as_str()).cmp(&(b.line, b.column, b.identifier.as_str())));
    warnings
}

struct PendingBranch {
    class_idx: usize,
    filter: DatasetFilter,
    locals: Vec<(String, Expr)>,
    item_idx: usize,
    init_sel: Option<(usize, usize)>,
    target_was_any: bool,
}

fn validate_ode_branch_inits(eq_items: &[EqItem], pending_branches: &[PendingBranch], outside_class: usize) -> Result<(), FitError> {
    let missing_ic_line = pending_branches.iter().find_map(|b| {
        if b.init_sel.is_some() {
            return None;
        }
        if b.class_idx != outside_class || !b.target_was_any {
            return Some(eq_items[b.item_idx].line_no());
        }
        let shared_with_init_branch = pending_branches.iter().any(|other| other.init_sel.is_some() && other.item_idx == b.item_idx);
        (!shared_with_init_branch).then(|| eq_items[b.item_idx].line_no())
    });
    if let Some(line) = missing_ic_line {
        return Err(FitError::DslCompileError {
            message: "ODE branch is missing a matching initial condition (`Y0 = ...` or `Y(t0) = ...`)".to_owned(),
            line,
            column: 1,
        });
    }
    Ok(())
}

fn update_specific_slot(slot: &mut Option<(usize, usize, bool)>, loc_len: usize, item_idx: usize, is_specific: bool) {
    let prev_specific = slot.is_some_and(|(_, _, s)| s);
    if is_specific || !prev_specific {
        *slot = Some((loc_len, item_idx, is_specific));
    }
}

fn infer_final_equation_kind(eq_items: &[EqItem], pending_branches: &[PendingBranch], has_ode: bool) -> DslEquationKind {
    if has_ode {
        return DslEquationKind::OdeFirstOrder;
    }
    let is_implicit = pending_branches.iter().any(|b| {
        let item = &eq_items[b.item_idx];
        matches!(item, EqItem::TargetImplicitDiff { .. })
            || item.target_raw_expr(false).is_some_and(|expr| branch_target_depends_on_y(&b.locals, expr))
    });
    if is_implicit { DslEquationKind::Implicit } else { DslEquationKind::Explicit }
}

fn select_outside_branch(
    val_ctx: &BranchValidationCtx<'_>,
    outside_class: usize,
    outside_filter: DatasetFilter,
    outside_locals: &[usize],
    outside_outputs: &[(usize, usize, bool)],
    outside_inits: &[(usize, usize, bool)],
    any_explicit_branch: bool,
) -> Result<Option<PendingBranch>, FitError> {
    let outside_candidates = outside_outputs
        .iter()
        .copied()
        .rev()
        .filter(|&(_, _, was_any)| !was_any)
        .chain(outside_outputs.iter().copied().rev().filter(|&(_, _, was_any)| was_any));
    for (loc_len, item_idx, was_any) in outside_candidates {
        let local_slice = &outside_locals[..loc_len];
        if any_explicit_branch
            && was_any
            && branch_uses_unbound_scoped_indices(val_ctx.eq_items, local_slice, item_idx, val_ctx.has_ode, val_ctx.scoped_names)
        {
            continue;
        }
        let locals = validate_branch_locals_and_trailing(val_ctx, outside_locals, loc_len, item_idx, any_explicit_branch && was_any, !was_any)?;
        let selected_outside_init =
            outside_inits.iter().rev().find(|&&(_, _, spec)| spec).or_else(|| outside_inits.last()).map(|&(i_loc, i_idx, _)| (i_loc, i_idx));
        return Ok(Some(PendingBranch {
            class_idx: outside_class,
            filter: outside_filter,
            locals,
            item_idx,
            init_sel: selected_outside_init,
            target_was_any: was_any,
        }));
    }
    Ok(None)
}

pub(in crate::dsl) struct ResolvedEquationBody {
    pub(in crate::dsl) kind: DslEquationKind,
    pub(in crate::dsl) body_stmts: Vec<OutputBranch>,
    pub(in crate::dsl) ode_init_branches: Vec<OdeInitBranch>,
    pub(in crate::dsl) warnings: Vec<DslCompileWarning>,
}

pub(in crate::dsl) fn resolve_equation_branches(
    parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)>,
    ident_locs: &HashMap<String, (usize, usize)>,
) -> Result<ResolvedEquationBody, FitError> {
    let BuiltEqItems { has_ode, uses_t_axis, has_ode_init, items: eq_items } = build_eq_items(parsed_lines)?;
    if has_ode && !eq_items.iter().any(|it| matches!(it, EqItem::TargetOde { .. })) {
        return Err(FitError::DslCompileError { message: "Missing ODE derivative expression".to_owned(), line: 1, column: 1 });
    }

    let scoped_local_names: HashSet<String> = eq_items
        .iter()
        .filter_map(|it| match it {
            EqItem::Local { filter, name, .. } if *filter != DatasetFilter::Any => Some(name.clone()),
            _ => None,
        })
        .collect();
    let all_local_names: HashSet<&str> = eq_items
        .iter()
        .filter_map(|it| match it {
            EqItem::Local { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();

    let partition = collect_filter_classes(&eq_items)?;
    let num_explicit = partition.explicit_classes.len();
    let outside_class = num_explicit;
    let total_classes = num_explicit + 1;

    let mut class_locals: Vec<Vec<usize>> = vec![Vec::new(); total_classes];
    let mut explicit_outputs: Vec<Option<(usize, usize, bool)>> = vec![None; num_explicit];
    let mut outside_outputs: Vec<(usize, usize, bool)> = Vec::new();
    let mut explicit_inits: Vec<Option<(usize, usize, bool)>> = vec![None; num_explicit];
    let mut outside_inits: Vec<(usize, usize, bool)> = Vec::new();
    let mut total_branch_refs = 0_usize;
    let mut last_any_target_idx: Option<usize> = None;

    for (item_idx, item) in eq_items.iter().enumerate() {
        let is_local = matches!(item, EqItem::Local { .. });
        let is_ode_init = matches!(item, EqItem::OdeInit { .. });
        let is_target = item.is_target_for_mode(has_ode);
        if !is_local && !is_ode_init && !is_target {
            continue;
        }
        let f = item.filter();
        let was_any = *f == DatasetFilter::Any;
        if is_target && was_any {
            last_any_target_idx = Some(item_idx);
        }
        let classes_slice: &[usize] = if was_any {
            &[]
        } else if let Some(&fi) = partition.filter_ids.get(f) {
            &partition.matching_classes[fi]
        } else {
            &[]
        };

        if is_local {
            let count_matched = if was_any { total_classes } else { classes_slice.len() };
            total_branch_refs = total_branch_refs.saturating_add(count_matched);
            if total_branch_refs > MAX_TOTAL_OPCODES {
                return Err(FitError::DslCompileError {
                    message: format!("Compiled equation exceeds branch-expansion budget ({MAX_TOTAL_OPCODES})"),
                    line: 1,
                    column: 1,
                });
            }
        }

        let target_classes: Box<dyn Iterator<Item = usize>> =
            if was_any { Box::new(0..total_classes) } else { Box::new(classes_slice.iter().copied()) };
        for c_idx in target_classes {
            if is_local {
                class_locals[c_idx].push(item_idx);
            } else if is_ode_init {
                let is_specific = !was_any;
                if c_idx == outside_class {
                    outside_inits.push((class_locals[outside_class].len(), item_idx, is_specific));
                } else {
                    update_specific_slot(&mut explicit_inits[c_idx], class_locals[c_idx].len(), item_idx, is_specific);
                }
            } else if c_idx == outside_class {
                outside_outputs.push((class_locals[outside_class].len(), item_idx, was_any));
            } else {
                update_specific_slot(&mut explicit_outputs[c_idx], class_locals[c_idx].len(), item_idx, !was_any);
            }
        }
    }

    let val_ctx = BranchValidationCtx { eq_items: &eq_items, has_ode, scoped_names: &scoped_local_names, last_any_target_idx };
    let mut pending_branches: Vec<PendingBranch> = Vec::new();
    for (c_idx, class_filter) in partition.explicit_classes.into_iter().enumerate() {
        if let Some((loc_len, item_idx, is_specific)) = explicit_outputs[c_idx] {
            let locals = validate_branch_locals_and_trailing(&val_ctx, &class_locals[c_idx], loc_len, item_idx, false, is_specific)?;
            let init_sel = explicit_inits[c_idx].map(|(i_loc, i_idx, _)| (i_loc, i_idx));
            pending_branches.push(PendingBranch { class_idx: c_idx, filter: class_filter, locals, item_idx, init_sel, target_was_any: !is_specific });
        }
    }

    if let Some(outside_branch) = select_outside_branch(
        &val_ctx,
        outside_class,
        partition.outside_filter,
        &class_locals[outside_class],
        &outside_outputs,
        &outside_inits,
        !pending_branches.is_empty(),
    )? {
        pending_branches.push(outside_branch);
    }

    if has_ode && has_ode_init {
        validate_ode_branch_inits(&eq_items, &pending_branches, outside_class)?;
    }

    let final_kind = infer_final_equation_kind(&eq_items, &pending_branches, has_ode);

    let mut body_stmts = Vec::with_capacity(pending_branches.len());
    let mut ode_init_branches = Vec::new();
    for b in pending_branches {
        if let Some(target) = target_for_resolved_kind(&eq_items[b.item_idx], final_kind) {
            if has_ode && let Some((init_loc_len, init_idx)) = b.init_sel {
                let (t0, init_locals, init_rhs) =
                    resolve_ode_init_for_class(&eq_items, &class_locals[b.class_idx][..init_loc_len], init_idx, &all_local_names)?;
                ode_init_branches.push((b.filter.clone(), t0, init_locals, init_rhs));
            }
            body_stmts.push((b.filter, b.locals, target));
        }
    }
    if has_ode && has_ode_init && ode_init_branches.is_empty() {
        return Err(FitError::DslCompileError {
            message: "No dataset scope pairs an ODE derivative (`dY/dX = ...`) with a matching initial condition (`Y0 = ...` or `Y(t0) = ...`)"
                .to_owned(),
            line: 1,
            column: 1,
        });
    }
    let warnings = collect_dsl_warnings(ident_locs, &all_local_names, uses_t_axis);
    Ok(ResolvedEquationBody { kind: final_kind, body_stmts, ode_init_branches, warnings })
}

pub(in crate::dsl) fn compute_dataset_stats(points: &[DataPoint]) -> [f64; 11] {
    let mut finite_pts: Vec<(f64, f64)> = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
    if finite_pts.is_empty() {
        return [0.0, 1.0, 0.0, 1.0, 0.5, 0.5, 0.0, 1.0, 0.5, 1.0, 1.0];
    }
    finite_pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let xmin = finite_pts[0].0;
    let xmax = finite_pts[finite_pts.len() - 1].0;
    let mut ymin = finite_pts[0].1;
    let mut ymax = finite_pts[0].1;
    let mut x_at_ymin = finite_pts[0].0;
    let mut x_at_ymax = finite_pts[0].0;
    for &(x, y) in &finite_pts {
        if y < ymin {
            ymin = y;
            x_at_ymin = x;
        }
        if y > ymax {
            ymax = y;
            x_at_ymax = x;
        }
    }
    let eps = 1e-12 * (xmax - xmin).max(1e-12);
    let left_vals: Vec<f64> = finite_pts.iter().filter(|&&(x, _)| (x - xmin).abs() <= eps).map(|&(_, y)| y).collect();
    let right_vals: Vec<f64> = finite_pts.iter().filter(|&&(x, _)| (xmax - x).abs() <= eps).map(|&(_, y)| y).collect();
    let y_left = if left_vals.is_empty() { finite_pts[0].1 } else { left_vals.iter().sum::<f64>() / left_vals.len() as f64 };
    let y_right = if right_vals.is_empty() { finite_pts[finite_pts.len() - 1].1 } else { right_vals.iter().sum::<f64>() / right_vals.len() as f64 };
    let xmid = 0.5 * (xmin + xmax);
    let ymid = 0.5 * (ymin + ymax);
    let span = ymax - ymin;
    let sign = if y_right >= y_left { 1.0 } else { -1.0 };
    let est_50 = estimate_x_at_ymid(points);
    let x_at_y50 = if est_50.is_finite() { est_50 } else { xmid };
    [ymin, ymax, xmin, xmax, xmid, ymid, x_at_ymin, x_at_ymax, x_at_y50, span, sign]
}

pub(in crate::dsl) fn canonicalize_init_expr(
    expr: &mut Expr,
    any_slots: &HashMap<String, (usize, usize)>,
    specific_slots: Option<&HashMap<String, (usize, usize)>>,
) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(name) => {
                let is_shadowed = any_slots.contains_key(name.as_str()) || specific_slots.is_some_and(|m| m.contains_key(name.as_str()));
                if !is_shadowed && let Some(&canon) = INIT_STAT_NAMES.iter().find(|&&s| s.eq_ignore_ascii_case(name)) {
                    *name = canon.to_owned();
                }
            }
            Expr::Unary { rhs, .. } => stack.push(rhs.as_mut()),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(lhs.as_mut());
                stack.push(rhs.as_mut());
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    stack.push(arg);
                }
            }
            Expr::If { cond, then_br, else_br } => {
                stack.push(cond.as_mut());
                stack.push(then_br.as_mut());
                stack.push(else_br.as_mut());
            }
        }
    }
}

pub(in crate::dsl) fn with_line(mut err: FitError, line_no: usize) -> FitError {
    if let FitError::DslCompileError { line, .. } = &mut err
        && *line <= 1
    {
        *line = line_no;
    }
    err
}

pub(in crate::dsl) fn ensure_no_data_vars(prog: &CompiledProgram, section_name: &str, line_no: usize) -> Result<(), FitError> {
    if prog.code.iter().any(|op| matches!(op, OpCode::LoadPredictor(_) | OpCode::LoadY)) {
        return Err(FitError::DslCompileError { message: format!("{section_name} cannot reference X or Y"), line: line_no, column: 1 });
    }
    Ok(())
}

pub(in crate::dsl) fn compile_initial_rules(
    raw_init_lines: Vec<(usize, DatasetFilter, String, Expr)>,
    param_names: &[String],
    param_map: &HashMap<String, usize>,
    total_opcodes: &mut usize,
) -> Result<(Vec<CompiledInitRule>, usize), FitError> {
    if raw_init_lines.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let stat_map: HashMap<String, usize> = INIT_STAT_NAMES.iter().enumerate().map(|(i, &n)| (n.to_owned(), i)).collect();
    let mut any_slots: HashMap<String, (usize, usize)> = HashMap::new();
    let mut scoped_slots: HashMap<DatasetFilter, HashMap<String, (usize, usize)>> = HashMap::new();
    let total_slots = raw_init_lines.len();
    let mut compiled = Vec::with_capacity(total_slots);

    for (rule_idx, (line_no, filter, name, mut rhs)) in raw_init_lines.into_iter().enumerate() {
        let specific = if filter == DatasetFilter::Any { None } else { scoped_slots.get(&filter) };
        canonicalize_init_expr(&mut rhs, &any_slots, specific);
        let resolver = LocalResolver::Scoped { any: &any_slots, specific };
        let remaining = MAX_TOTAL_OPCODES.saturating_sub(*total_opcodes);
        let prog = compile_target_resolved(&rhs, &resolver, &stat_map, remaining).map_err(|e| with_line(e, line_no))?;
        *total_opcodes = total_opcodes.saturating_add(prog.code.len());
        if *total_opcodes > MAX_TOTAL_OPCODES {
            return Err(FitError::DslCompileError {
                message: format!("[InitialValues] exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
                line: line_no,
                column: 1,
            });
        }
        ensure_no_data_vars(&prog, "[InitialValues]", line_no)?;
        let param_idx = param_map.get(&name).copied().or_else(|| param_names.iter().position(|p| p.eq_ignore_ascii_case(&name)));
        let slot = rule_idx;
        if filter == DatasetFilter::Any {
            any_slots.insert(name, (slot, rule_idx));
        } else {
            scoped_slots.entry(filter.clone()).or_default().insert(name, (slot, rule_idx));
        }
        compiled.push(CompiledInitRule { filter, slot, param_idx, prog });
    }
    Ok((compiled, total_slots))
}

pub(in crate::dsl) type CompiledTransforms = Vec<(String, usize, CompiledProgram)>;

pub(in crate::dsl) fn compile_transforms(
    raw_transform_lines: Vec<(usize, String, Expr)>,
    param_map: &HashMap<String, usize>,
    total_opcodes: &mut usize,
) -> Result<(CompiledTransforms, usize), FitError> {
    if raw_transform_lines.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let mut local_map: HashMap<String, usize> = HashMap::new();
    let mut compiled = Vec::with_capacity(raw_transform_lines.len());
    for (line_no, name, rhs) in raw_transform_lines {
        let resolver = LocalResolver::Flat(&local_map);
        let remaining = MAX_TOTAL_OPCODES.saturating_sub(*total_opcodes);
        let prog = compile_target_resolved(&rhs, &resolver, param_map, remaining).map_err(|e| with_line(e, line_no))?;
        *total_opcodes = total_opcodes.saturating_add(prog.code.len());
        if *total_opcodes > MAX_TOTAL_OPCODES {
            return Err(FitError::DslCompileError {
                message: format!("[Transforms] exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
                line: line_no,
                column: 1,
            });
        }
        ensure_no_data_vars(&prog, "[Transforms]", line_no)?;
        let slot = if let Some(&s) = local_map.get(&name) {
            s
        } else {
            let s = local_map.len();
            local_map.insert(name.clone(), s);
            s
        };
        compiled.push((name, slot, prog));
    }
    Ok((compiled, local_map.len()))
}

pub(in crate::dsl) fn expr_references_y(expr: &Expr) -> bool {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(n) => {
                if n.eq_ignore_ascii_case("Y") {
                    return true;
                }
            }
            Expr::Unary { rhs, .. } => stack.push(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            Expr::Call { args, .. } => {
                for a in args {
                    stack.push(a);
                }
            }
            Expr::If { cond, then_br, else_br } => {
                stack.push(else_br);
                stack.push(then_br);
                stack.push(cond);
            }
        }
    }
    false
}

/// Parse a 1-based multi-column predictor identifier (`X1`, `X2`, ..., `x2`) into its
/// zero-based predictor index (`0`, `1`, ...). Returns `None` for `"X"`, `"X0"`, `"X00"`,
/// non-digit suffixes, or integer overflow.
pub(in crate::dsl) fn parse_predictor_index(name: &str) -> Option<usize> {
    let rest = name.strip_prefix(['X', 'x'])?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse::<usize>().ok()?.checked_sub(1)
}

pub(in crate::dsl) fn is_builtin_constant_or_var(name: &str) -> bool {
    if parse_predictor_index(name).is_some() {
        return true;
    }
    let u = name.to_ascii_uppercase();
    matches!(u.as_str(), "X" | "T" | "Y" | "DS" | "DATASET" | "PI" | "E" | "LN2" | "LN10" | "SQRT2" | "INF" | "NAN")
}

fn collect_free_idents_with_declared_locals(
    expr: &Expr,
    bound_locals: &BTreeSet<String>,
    declared_locals: &HashSet<&str>,
    seen: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(name) => {
                if !bound_locals.contains(name)
                    && (!is_builtin_constant_or_var(name) || declared_locals.contains(name.as_str()))
                    && !seen.contains(name)
                {
                    seen.insert(name.clone());
                    order.push(name.clone());
                }
            }
            Expr::Unary { rhs, .. } => stack.push(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            Expr::Call { args, .. } => {
                for a in args.iter().rev() {
                    stack.push(a);
                }
            }
            Expr::If { cond, then_br, else_br } => {
                stack.push(else_br);
                stack.push(then_br);
                stack.push(cond);
            }
        }
    }
}

pub(in crate::dsl) fn collect_free_idents(expr: &Expr, bound_locals: &BTreeSet<String>, seen: &mut BTreeSet<String>, order: &mut Vec<String>) {
    let empty_declared = HashSet::new();
    collect_free_idents_with_declared_locals(expr, bound_locals, &empty_declared, seen, order);
}
