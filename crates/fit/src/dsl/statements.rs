//! Line and section parsing: dataset prefixes, statements, equation branches, initial-value rules, and transforms.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::dsl::ast::{BinaryOp, CompiledInitRule, CompiledProgram, DatasetFilter, DslEquationKind, Expr, OpCode};
use crate::dsl::compile::compile_target_resolved;
use crate::dsl::limits::{MAX_DATASET_CLASSES, MAX_DATASET_EXPANSION, MAX_TOTAL_OPCODES};
use crate::dsl::parser::{LocalResolver, parse_expr};
use crate::error::FitError;
use crate::library::estimate_x_at_ymid;
use crate::types::DataPoint;

pub(in crate::dsl) enum ParsedStatement {
    OdeDerivative(Expr),
    OdeInitial { lhs_name: Option<String>, t0: Option<f64>, rhs: Expr },
    Assign { name: String, rhs: Expr },
    ImplicitEquation { lhs: Expr, rhs: Expr },
    BareExpr(Expr),
}

pub(in crate::dsl) fn strip_comment<'a>(line: &'a str, hints: &mut HashMap<String, f64>, _ode_t0: &mut f64) -> &'a str {
    let trimmed = line.trim();
    for marker in ['#', ';'] {
        if let Some(pos) = trimmed.find(marker) {
            let comment = trimmed[pos + 1..].trim();
            if let Some(rest) = comment.strip_prefix("initial:") {
                for pair in rest.split(',') {
                    if let Some((k, v)) = pair.split_once('=')
                        && let Ok(num) = v.trim().parse::<f64>()
                        && num.is_finite()
                    {
                        hints.insert(k.trim().to_owned(), num);
                    }
                }
            }
            return trimmed[..pos].trim();
        }
    }
    trimmed
}

pub(in crate::dsl) fn parse_dataset_prefix(line: &str, line_no: usize) -> Result<(DatasetFilter, &str), FitError> {
    if !line.starts_with('<') {
        return Ok((DatasetFilter::Any, line));
    }
    let Some(close) = line.find('>') else {
        return Err(FitError::DslCompileError { message: "Unclosed `<...>` dataset prefix".to_owned(), line: line_no, column: 1 });
    };
    let inside = line[1..close].trim();
    let rest = line[close + 1..].trim();
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
    // Detect `LHS = RHS` (without `==`) cleanly at top level.
    if let Some((lhs_s, rhs_s)) = split_top_level_assignment(rest) {
        let lhs_trim = lhs_s.trim();
        let lhs_compact: String = lhs_trim.chars().filter(|c| !c.is_whitespace()).collect();
        let lhs_upper = lhs_compact.to_ascii_uppercase();
        if lhs_upper == "DY/DX" || lhs_upper == "DY/DT" || lhs_upper == "D(Y,X)" || lhs_upper == "D(Y,T)" {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeDerivative(rhs));
        }
        if lhs_upper == "Y0" {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: Some(lhs_trim.to_owned()), t0: Some(0.0), rhs });
        }
        if lhs_upper == "Y(0)" {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: None, t0: Some(0.0), rhs });
        }
        if let Some(inner) = lhs_upper.strip_prefix("Y(").and_then(|s| s.strip_suffix(')'))
            && let Ok(t0_val) = inner.parse::<f64>()
        {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: None, t0: Some(t0_val), rhs });
        }
        if is_valid_ident(lhs_trim) {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::Assign { name: lhs_trim.to_owned(), rhs });
        }
        let lhs = parse_expr(lhs_trim, line_no)?;
        let rhs = parse_expr(rhs_s, line_no)?;
        return Ok(ParsedStatement::ImplicitEquation { lhs, rhs });
    }
    Ok(ParsedStatement::BareExpr(parse_expr(rest, line_no)?))
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
                if prev != b'!' && prev != b'<' && prev != b'>' && prev != b'=' && next != b'=' {
                    return Some((&s[..i], &s[i + 1..]));
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

pub(in crate::dsl) enum EqItem {
    Local { filter: DatasetFilter, name: String, rhs: Expr },
    Explicit { filter: DatasetFilter, expr: Expr },
    Implicit { filter: DatasetFilter, diff: Expr },
    Ode { filter: DatasetFilter, rhs: Expr },
}

impl EqItem {
    pub(in crate::dsl) fn filter(&self) -> &DatasetFilter {
        match self {
            Self::Local { filter, .. } | Self::Explicit { filter, .. } | Self::Implicit { filter, .. } | Self::Ode { filter, .. } => filter,
        }
    }
}

pub(in crate::dsl) fn parse_section_header(trimmed: &str, line_no: usize) -> Result<Option<DslSection>, FitError> {
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Ok(None);
    }
    let inner: String =
        trimmed[1..trimmed.len() - 1].chars().filter(|c| !c.is_ascii_whitespace() && *c != '_' && *c != '-').flat_map(char::to_lowercase).collect();
    match inner.as_str() {
        "equation" | "equations" | "body" | "model" => Ok(Some(DslSection::Equation)),
        "initialvalues" | "initialvalue" | "initials" | "init" => Ok(Some(DslSection::InitialValues)),
        "transforms" | "transform" | "derived" | "derivedscalars" => Ok(Some(DslSection::Transforms)),
        _ => Err(FitError::DslCompileError { message: format!("Unknown DSL section `{trimmed}`"), line: line_no, column: 1 }),
    }
}

pub(in crate::dsl) struct BuiltEqItems {
    pub(in crate::dsl) kind: DslEquationKind,
    pub(in crate::dsl) items: Vec<EqItem>,
    pub(in crate::dsl) init_stmts: Vec<(usize, Expr)>,
    pub(in crate::dsl) ode_t0: f64,
}

pub(in crate::dsl) fn build_eq_items(parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)>, mut ode_t0: f64) -> Result<BuiltEqItems, FitError> {
    let has_ode = parsed_lines.iter().any(|(_, _, s)| matches!(s, ParsedStatement::OdeDerivative(_)));
    let mut kind = DslEquationKind::Explicit;
    let mut items = Vec::with_capacity(parsed_lines.len());
    let mut init_stmts = Vec::new();
    for (line_no, filter, stmt) in parsed_lines {
        match stmt {
            ParsedStatement::OdeDerivative(rhs) => {
                kind = DslEquationKind::OdeFirstOrder;
                items.push(EqItem::Ode { filter, rhs });
            }
            ParsedStatement::OdeInitial { lhs_name, t0, rhs } => {
                if has_ode {
                    if let Some(t) = t0 {
                        ode_t0 = t;
                    }
                    init_stmts.push((line_no, rhs));
                } else if let Some(name) = lhs_name {
                    // Non-ODE `Y0 = expr` is an ordinary local assignment rather than a hidden/discarded ODE IC.
                    items.push(EqItem::Local { filter, name, rhs });
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
                    if expr_references_y(&rhs) {
                        kind = DslEquationKind::Implicit;
                        let diff = Expr::Binary { op: BinaryOp::Sub, lhs: Box::new(Expr::Ident("Y".to_owned())), rhs: Box::new(rhs) };
                        items.push(EqItem::Implicit { filter, diff });
                    } else {
                        items.push(EqItem::Explicit { filter, expr: rhs });
                    }
                } else {
                    items.push(EqItem::Local { filter, name, rhs });
                }
            }
            ParsedStatement::ImplicitEquation { lhs, rhs } => {
                kind = DslEquationKind::Implicit;
                let diff = Expr::Binary { op: BinaryOp::Sub, lhs: Box::new(lhs), rhs: Box::new(rhs) };
                items.push(EqItem::Implicit { filter, diff });
            }
            ParsedStatement::BareExpr(rhs) => {
                if expr_references_y(&rhs) {
                    kind = DslEquationKind::Implicit;
                    items.push(EqItem::Implicit { filter, diff: rhs });
                } else {
                    items.push(EqItem::Explicit { filter, expr: rhs });
                }
            }
        }
    }
    Ok(BuiltEqItems { kind, items, init_stmts, ode_t0 })
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

pub(in crate::dsl) fn target_for_kind(item: &EqItem, kind: DslEquationKind) -> Option<(Expr, bool)> {
    let was_any = *item.filter() == DatasetFilter::Any;
    match (kind, item) {
        (DslEquationKind::OdeFirstOrder, EqItem::Ode { rhs, .. }) => Some((rhs.clone(), was_any)),
        (DslEquationKind::Implicit, EqItem::Implicit { diff, .. }) => Some((diff.clone(), was_any)),
        (DslEquationKind::Implicit, EqItem::Explicit { expr, .. }) => {
            Some((Expr::Binary { op: BinaryOp::Sub, lhs: Box::new(Expr::Ident("Y".to_owned())), rhs: Box::new(expr.clone()) }, was_any))
        }
        (DslEquationKind::Explicit, EqItem::Explicit { expr, .. }) => Some((expr.clone(), was_any)),
        _ => None,
    }
}

pub(in crate::dsl) fn target_expr_for_unbound_check(item: &EqItem, kind: DslEquationKind) -> Option<&Expr> {
    match (kind, item) {
        (DslEquationKind::OdeFirstOrder, EqItem::Ode { rhs, .. }) => Some(rhs),
        (DslEquationKind::Implicit, EqItem::Implicit { diff, .. }) => Some(diff),
        (DslEquationKind::Implicit | DslEquationKind::Explicit, EqItem::Explicit { expr, .. }) => Some(expr),
        _ => None,
    }
}

pub(in crate::dsl) fn branch_uses_unbound_scoped_indices(
    eq_items: &[EqItem],
    local_indices: &[usize],
    target_idx: usize,
    kind: DslEquationKind,
    scoped_names: &HashSet<String>,
) -> bool {
    let Some(target) = target_expr_for_unbound_check(&eq_items[target_idx], kind) else {
        return false;
    };
    let mut free = Vec::new();
    let mut seen = BTreeSet::new();
    let mut bound = BTreeSet::new();
    for &idx in local_indices {
        if let EqItem::Local { name, rhs, .. } = &eq_items[idx] {
            collect_free_idents(rhs, &bound, &mut seen, &mut free);
            bound.insert(name.clone());
        }
    }
    collect_free_idents(target, &bound, &mut seen, &mut free);
    free.iter().any(|ident| scoped_names.contains(ident))
}

pub(in crate::dsl) struct ResolvedEquationBody {
    pub(in crate::dsl) kind: DslEquationKind,
    pub(in crate::dsl) body_stmts: Vec<OutputBranch>,
    pub(in crate::dsl) init_stmts: Vec<(usize, Expr)>,
    pub(in crate::dsl) ode_t0: f64,
}

pub(in crate::dsl) fn resolve_equation_branches(
    parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)>,
    ode_t0: f64,
) -> Result<ResolvedEquationBody, FitError> {
    let BuiltEqItems { kind, items: eq_items, init_stmts, ode_t0 } = build_eq_items(parsed_lines, ode_t0)?;
    let has_ode_output = eq_items.iter().any(|it| matches!(it, EqItem::Ode { .. }));
    if kind == DslEquationKind::OdeFirstOrder && !has_ode_output {
        return Err(FitError::DslCompileError { message: "Missing ODE derivative expression".to_owned(), line: 1, column: 1 });
    }
    let scoped_local_names: HashSet<String> = eq_items
        .iter()
        .filter_map(|it| match it {
            EqItem::Local { filter, name, .. } if *filter != DatasetFilter::Any => Some(name.clone()),
            _ => None,
        })
        .collect();

    let partition = collect_filter_classes(&eq_items)?;
    let num_explicit = partition.explicit_classes.len();
    let outside_class = num_explicit;
    let total_classes = num_explicit + 1;

    let mut class_locals: Vec<Vec<usize>> = vec![Vec::new(); total_classes];
    let mut explicit_outputs: Vec<Option<(usize, usize)>> = vec![None; num_explicit];
    let mut outside_outputs: Vec<(usize, usize, bool)> = Vec::new();
    let mut total_branch_refs = 0_usize;

    for (item_idx, item) in eq_items.iter().enumerate() {
        let f = item.filter();
        let is_local = matches!(item, EqItem::Local { .. });
        let is_target = target_expr_for_unbound_check(item, kind).is_some();
        if !is_local && !is_target {
            continue;
        }
        let was_any = *f == DatasetFilter::Any;
        let classes_slice: &[usize] = if was_any {
            &[]
        } else if let Some(&fi) = partition.filter_ids.get(f) {
            &partition.matching_classes[fi]
        } else {
            &[]
        };

        let count_matched = if was_any { total_classes } else { classes_slice.len() };
        if is_local {
            total_branch_refs = total_branch_refs.saturating_add(count_matched);
            if total_branch_refs > MAX_TOTAL_OPCODES {
                return Err(FitError::DslCompileError {
                    message: format!("Compiled equation exceeds branch-expansion budget ({MAX_TOTAL_OPCODES})"),
                    line: 1,
                    column: 1,
                });
            }
        }

        if was_any {
            for c_idx in 0..total_classes {
                if is_local {
                    class_locals[c_idx].push(item_idx);
                } else if c_idx == outside_class {
                    outside_outputs.push((class_locals[outside_class].len(), item_idx, true));
                } else {
                    explicit_outputs[c_idx] = Some((class_locals[c_idx].len(), item_idx));
                }
            }
        } else {
            for &c_idx in classes_slice {
                if is_local {
                    class_locals[c_idx].push(item_idx);
                } else if c_idx == outside_class {
                    outside_outputs.push((class_locals[outside_class].len(), item_idx, false));
                } else {
                    explicit_outputs[c_idx] = Some((class_locals[c_idx].len(), item_idx));
                }
            }
        }
    }

    let materialize_locals = |indices: &[usize]| -> Vec<(String, Expr)> {
        indices
            .iter()
            .filter_map(|&idx| match &eq_items[idx] {
                EqItem::Local { name, rhs, .. } => Some((name.clone(), rhs.clone())),
                _ => None,
            })
            .collect()
    };

    let mut body_stmts = Vec::new();
    for (c_idx, class_filter) in partition.explicit_classes.into_iter().enumerate() {
        if let Some((loc_len, item_idx)) = explicit_outputs[c_idx]
            && let Some((target, _)) = target_for_kind(&eq_items[item_idx], kind)
        {
            let locals = materialize_locals(&class_locals[c_idx][..loc_len]);
            body_stmts.push((class_filter, locals, target));
        }
    }
    let any_explicit_branch = !body_stmts.is_empty();
    for (loc_len, item_idx, was_any) in outside_outputs.into_iter().rev() {
        let local_slice = &class_locals[outside_class][..loc_len];
        if any_explicit_branch && was_any && branch_uses_unbound_scoped_indices(&eq_items, local_slice, item_idx, kind, &scoped_local_names) {
            continue;
        }
        if let Some((target, _)) = target_for_kind(&eq_items[item_idx], kind) {
            let locals = materialize_locals(local_slice);
            body_stmts.push((partition.outside_filter, locals, target));
            break;
        }
    }

    Ok(ResolvedEquationBody { kind, body_stmts, init_stmts, ode_t0 })
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

pub(in crate::dsl) fn is_builtin_constant_or_var(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    if matches!(u.as_str(), "X" | "T" | "Y" | "DS" | "DATASET" | "PI" | "E" | "LN2" | "LN10" | "SQRT2" | "INF" | "NAN") {
        return true;
    }
    if let Some(rest) = u.strip_prefix('X')
        && !rest.is_empty()
        && rest != "0"
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    false
}

pub(in crate::dsl) fn collect_free_idents(expr: &Expr, bound_locals: &BTreeSet<String>, seen: &mut BTreeSet<String>, order: &mut Vec<String>) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(name) => {
                if !is_builtin_constant_or_var(name) && !bound_locals.contains(name) && !seen.contains(name) {
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
