//! Shared diagnostic-plot result resolution, cell-source extraction, and subtype dispatch.

mod heatmap;
mod residuals_qq;
mod roc_forest;
mod volcano_ba;

use curvebook_doc::{
    AnalysisKind, DataSheet, GraphSheet, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, SheetId, TableArchetype,
};

pub(super) use heatmap::compile_heatmap;
use residuals_qq::selected_dataset_label_in_tab;
pub(super) use residuals_qq::{compile_normal_qq_plot, compile_residual_plot};
pub(super) use roc_forest::{compile_forest_plot, compile_roc_curve};
pub(super) use volcano_ba::{compile_bland_altman, compile_volcano};

use crate::scene::{ChartWarning, SourceCellId};

// ============================================================================
// 2. All 7 DiagnosticPlotKinds
// ============================================================================

fn push_unique_warning(warnings: &mut Vec<ChartWarning>, code: &'static str, message: String) {
    if !warnings.iter().any(|w| w.code == code && w.message == message) {
        warnings.push(ChartWarning::new(code, message));
    }
}

fn raw_matching_results<'a>(graph: &GraphSheet, doc: &'a ProjectDocument) -> Vec<&'a ResultSheet> {
    if let Some(direct) = doc.result_sheet(&graph.spec.primary_source_sheet_id) {
        return vec![direct];
    }
    doc.result_sheets.iter().filter(|r| r.analysis.source_sheet_id.as_ref() == Some(&graph.spec.primary_source_sheet_id)).collect()
}

fn matching_results<'a>(graph: &GraphSheet, doc: &'a ProjectDocument) -> Vec<&'a ResultSheet> {
    raw_matching_results(graph, doc).into_iter().filter(|r| r.has_displayable_output()).collect()
}

fn find_matching_result_where<'a>(graph: &GraphSheet, doc: &'a ProjectDocument, pred: impl Fn(&ResultSheet) -> bool) -> Option<&'a ResultSheet> {
    matching_results(graph, doc).into_iter().find(|r| pred(r))
}

fn find_matching_result_where_checked<'a>(
    graph: &GraphSheet,
    doc: &'a ProjectDocument,
    warnings: &mut Vec<ChartWarning>,
    pred: impl Fn(&ResultSheet) -> bool,
) -> Option<&'a ResultSheet> {
    let found = find_matching_result_where(graph, doc, &pred);
    if found.is_none() {
        let raw = raw_matching_results(graph, doc);
        if let Some(failed) = raw.iter().copied().find(|r| matches!(r.status, ResultStatus::Failed { .. }) && pred(r)) {
            push_unique_warning(warnings, "failed_result_sheet", format!("result sheet '{}' failed; omitted stale analysis results", failed.id));
        } else if let Some(pending) = raw.iter().copied().find(|r| !r.has_displayable_output() && pred(r)) {
            push_unique_warning(
                warnings,
                "pending_analysis_result",
                format!("result sheet '{}' is pending recalculation; omitted stale analysis results", pending.id),
            );
        }
    }
    found
}

fn has_blocked_matching_result(graph: &GraphSheet, doc: &ProjectDocument, pred: impl Fn(&ResultSheet) -> bool) -> bool {
    raw_matching_results(graph, doc).into_iter().any(|r| !r.has_displayable_output() && pred(r))
}

fn find_matching_result_checked<'a>(graph: &GraphSheet, doc: &'a ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<&'a ResultSheet> {
    find_matching_result_where_checked(graph, doc, warnings, |_| true)
}

#[derive(Debug, Clone)]
struct DiagPoint {
    x: f64,
    y: f64,
    label: Option<String>,
    source: SourceCellId,
    series_label: String,
}

fn active_xy_pairs_with_sources(sheet_id: &SheetId, xy: &curvebook_doc::XyTable, data_set: usize) -> Vec<(f64, f64, SourceCellId, String)> {
    let Some(ds) = xy.data_sets.get(data_set) else {
        return Vec::new();
    };
    let label = ds.title.clone();
    let mut out = Vec::new();
    for (row, subcells) in ds.cells.iter().enumerate() {
        let Some(x) = xy.x_summary_at(row).ok().flatten().filter(|v| v.is_finite()) else {
            continue;
        };
        if xy.y_subcolumn_mode.is_pre_summarized() {
            if let Some(mean) = subcells.first().and_then(|c| c.active_number()).filter(|v| v.is_finite()) {
                out.push((x, mean, SourceCellId { sheet_id: sheet_id.clone(), row, col: data_set, subcol: 0 }, label.clone()));
            }
        } else {
            for (subcol, cell) in subcells.iter().enumerate() {
                if let Some(y) = cell.active_number().filter(|v| v.is_finite()) {
                    out.push((x, y, SourceCellId { sheet_id: sheet_id.clone(), row, col: data_set, subcol }, label.clone()));
                }
            }
        }
    }
    out
}

fn collect_ds_active_point_sources(ds: &DataSheet, col_idx: usize, include_replicates: bool) -> (Vec<SourceCellId>, String) {
    let mut out = Vec::new();
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            let c = col_idx.min(xy.data_sets.len().saturating_sub(1));
            let label = xy.data_sets.get(c).map_or_else(|| String::from("Y"), |d| d.title.clone());
            if let Some(d) = xy.data_sets.get(c) {
                for r in 0..xy.row_count() {
                    if !xy.x_summary_at(r).ok().flatten().is_some_and(f64::is_finite) {
                        continue;
                    }
                    let Some(row_cells) = d.cells.get(r) else {
                        continue;
                    };
                    if include_replicates && !xy.y_subcolumn_mode.is_pre_summarized() {
                        for (subcol, cell) in row_cells.iter().enumerate() {
                            if cell.active_number().is_some_and(f64::is_finite) {
                                out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol });
                            }
                        }
                    } else if xy.y_subcolumn_mode.is_pre_summarized() {
                        if row_cells.first().and_then(|cell| cell.active_number()).is_some_and(f64::is_finite) {
                            out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 });
                        }
                    } else if row_cells.iter().any(|cell| cell.active_number().is_some_and(f64::is_finite)) {
                        out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 });
                    }
                }
            }
            (out, label)
        }
        TableArchetype::Column(col) => {
            collect_grid_point_sources(&ds.id, &col.data_sets, col.row_count(), col_idx, col.subcolumn_mode.is_pre_summarized(), include_replicates)
        }
        TableArchetype::Grouped(g) => {
            collect_grid_point_sources(&ds.id, &g.data_sets, g.row_count(), col_idx, g.subcolumn_mode.is_pre_summarized(), include_replicates)
        }
        TableArchetype::MultipleVariables(mv) => {
            let c = col_idx.min(mv.variables.len().saturating_sub(1));
            let label = mv.variables.get(c).map_or_else(|| format!("Var {}", c + 1), |v| v.title.clone());
            if let Some(v) = mv.variables.get(c) {
                for r in 0..mv.row_count() {
                    if v.cells.get(r).and_then(|cell| cell.active_number()).is_some_and(f64::is_finite) {
                        out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 });
                    }
                }
            }
            (out, label)
        }
        _ => (out, String::new()),
    }
}

fn collect_grid_point_sources(
    sheet_id: &SheetId,
    data_sets: &[curvebook_doc::DataSetColumn],
    row_count: usize,
    col_idx: usize,
    is_pre_summarized: bool,
    include_replicates: bool,
) -> (Vec<SourceCellId>, String) {
    let mut out = Vec::new();
    let c = col_idx.min(data_sets.len().saturating_sub(1));
    let label = data_sets.get(c).map_or_else(|| format!("Group {}", c + 1), |d| d.title.clone());
    if let Some(d) = data_sets.get(c) {
        for r in 0..row_count {
            let Some(row_cells) = d.cells.get(r) else {
                continue;
            };
            if include_replicates && !is_pre_summarized {
                for (subcol, cell) in row_cells.iter().enumerate() {
                    if cell.active_number().is_some_and(f64::is_finite) {
                        out.push(SourceCellId { sheet_id: sheet_id.clone(), row: r, col: c, subcol });
                    }
                }
            } else if row_cells.first().and_then(|cell| cell.active_number()).is_some_and(f64::is_finite) {
                out.push(SourceCellId { sheet_id: sheet_id.clone(), row: r, col: c, subcol: 0 });
            }
        }
    }
    (out, label)
}

fn collect_grid_values_with_sources(
    sheet_id: &SheetId,
    data_sets: &[curvebook_doc::DataSetColumn],
    row_count: usize,
    col_idx: usize,
    is_pre_summarized: bool,
) -> (Vec<(f64, SourceCellId)>, String) {
    let mut out = Vec::new();
    let c = col_idx.min(data_sets.len().saturating_sub(1));
    let label = data_sets.get(c).map_or_else(|| format!("Group {}", c + 1), |d| d.title.clone());
    if let Some(d) = data_sets.get(c) {
        for r in 0..row_count {
            let Some(row_cells) = d.cells.get(r) else {
                continue;
            };
            if is_pre_summarized {
                if let Some(v) = row_cells.first().and_then(|cell| cell.active_number()).filter(|x| x.is_finite()) {
                    out.push((v, SourceCellId { sheet_id: sheet_id.clone(), row: r, col: c, subcol: 0 }));
                }
            } else {
                for (subcol, cell) in row_cells.iter().enumerate() {
                    if let Some(v) = cell.active_number().filter(|x| x.is_finite()) {
                        out.push((v, SourceCellId { sheet_id: sheet_id.clone(), row: r, col: c, subcol }));
                    }
                }
            }
        }
    }
    (out, label)
}

fn is_ds_pre_summarized(ds: &DataSheet) -> bool {
    match &ds.archetype {
        TableArchetype::Column(col) => col.subcolumn_mode.is_pre_summarized(),
        TableArchetype::Grouped(g) => g.subcolumn_mode.is_pre_summarized(),
        TableArchetype::Xy(xy) => xy.y_subcolumn_mode.is_pre_summarized(),
        _ => false,
    }
}

fn collect_ds_active_values_with_sources(ds: &DataSheet, col_idx: usize) -> (Vec<(f64, SourceCellId)>, String) {
    let mut out = Vec::new();
    match &ds.archetype {
        TableArchetype::Column(col) => {
            collect_grid_values_with_sources(&ds.id, &col.data_sets, col.row_count(), col_idx, col.subcolumn_mode.is_pre_summarized())
        }
        TableArchetype::Grouped(g) => {
            collect_grid_values_with_sources(&ds.id, &g.data_sets, g.row_count(), col_idx, g.subcolumn_mode.is_pre_summarized())
        }
        TableArchetype::MultipleVariables(mv) => {
            let c = col_idx.min(mv.variables.len().saturating_sub(1));
            let label = mv.variables.get(c).map_or_else(|| format!("Var {}", c + 1), |v| v.title.clone());
            if let Some(v) = mv.variables.get(c) {
                for r in 0..mv.row_count() {
                    if let Some(val) = v.cells.get(r).and_then(|cell| cell.active_number()).filter(|x| x.is_finite()) {
                        out.push((val, SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 }));
                    }
                }
            }
            (out, label)
        }
        TableArchetype::Xy(xy) => {
            let c = col_idx.min(xy.data_sets.len().saturating_sub(1));
            let pairs = active_xy_pairs_with_sources(&ds.id, xy, c);
            let label = xy.data_sets.get(c).map_or_else(|| String::from("Y"), |d| d.title.clone());
            out.extend(pairs.into_iter().map(|(_, y, src, _)| (y, src)));
            (out, label)
        }
        _ => (out, String::new()),
    }
}

pub(super) fn resolve_source_data_sheet_checked<'a>(
    graph: &GraphSheet,
    doc: &'a ProjectDocument,
    warnings: &mut Vec<ChartWarning>,
) -> Option<&'a DataSheet> {
    resolve_fresh_data_sheet_or_upstream(doc, &graph.spec.primary_source_sheet_id, warnings)
}

fn resolve_fresh_data_sheet_or_upstream<'a>(doc: &'a ProjectDocument, id: &SheetId, warnings: &mut Vec<ChartWarning>) -> Option<&'a DataSheet> {
    let mut visited = std::collections::HashSet::new();
    let mut cursor = id;
    while visited.insert(cursor) {
        if let Some(ds) = doc.data_sheet(cursor) {
            return Some(ds);
        }
        let res = doc.result_sheet(cursor)?;
        match &res.status {
            ResultStatus::UpToDate => {}
            ResultStatus::Failed { .. } => {
                push_unique_warning(warnings, "failed_result_sheet", format!("result sheet '{}' failed; omitted stale analysis results", res.id));
                return None;
            }
            ResultStatus::Dirty => {
                push_unique_warning(
                    warnings,
                    "pending_analysis_result",
                    format!("result sheet '{}' is pending recalculation; omitted stale analysis results", res.id),
                );
                return None;
            }
            ResultStatus::Imported | ResultStatus::ImportedStale => return None,
        }
        if let Some(analyzable) = res.analyzable_data.as_ref() {
            return Some(analyzable);
        }
        let next = res.analysis.source_sheet_id.as_ref()?;
        cursor = next;
    }
    None
}

fn resolve_analysis_source_data_sheet<'a>(res: &ResultSheet, graph: &GraphSheet, doc: &'a ProjectDocument) -> Option<&'a DataSheet> {
    res.analysis
        .source_sheet_id
        .as_ref()
        .and_then(|src| doc.data_sheet(src).or_else(|| crate::resolve_data_sheet_or_upstream(doc, src)))
        .or_else(|| crate::resolve_data_sheet_or_upstream(doc, &graph.spec.primary_source_sheet_id))
}

fn row_label_dataset_prefix(lbl: &str) -> Option<(&str, usize)> {
    let (prefix, num_str) = lbl.rsplit_once(" #")?;
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return None;
    }
    let idx = num_str.trim().parse::<usize>().ok()?;
    (idx >= 1).then_some((trimmed, idx - 1))
}

fn selected_dataset_prefix_in_row_labels(res: &ResultSheet, tab: &ResultTab, graph: &GraphSheet, doc: &ProjectDocument) -> Option<String> {
    let mut seen: Vec<String> = Vec::new();
    for lbl in &tab.row_labels {
        if let Some((prefix, _)) = row_label_dataset_prefix(lbl)
            && !seen.iter().any(|existing| existing == prefix)
        {
            seen.push(prefix.to_owned());
        }
    }
    if seen.len() <= 1 {
        return None;
    }
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    if let Some(ds) = resolve_analysis_source_data_sheet(res, graph, doc) {
        let desired_title = match &ds.archetype {
            TableArchetype::Column(col) => col.data_sets.get(ds_idx).map(|d| d.title.as_str()),
            TableArchetype::Grouped(g) => g.data_sets.get(ds_idx).map(|d| d.title.as_str()),
            TableArchetype::Xy(xy) => xy.data_sets.get(ds_idx).map(|d| d.title.as_str()),
            TableArchetype::MultipleVariables(mv) => mv.variables.get(ds_idx).map(|v| v.title.as_str()),
            _ => None,
        };
        if let Some(title) = desired_title
            && seen.iter().any(|s| s == title)
        {
            return Some(title.to_owned());
        }
    }
    seen.get(ds_idx).or_else(|| seen.first()).cloned()
}

fn result_cell_num(cell: &ResultCell) -> Option<f64> {
    match cell {
        ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn result_cell_p_or_num(cell: &ResultCell) -> Option<f64> {
    match cell {
        ResultCell::PValue { raw, .. } if raw.is_finite() => Some(*raw),
        ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn find_header_col(headers: &[String], pred: impl Fn(&str) -> bool) -> Option<usize> {
    headers.iter().position(|h| pred(&h.to_ascii_lowercase()))
}

fn extract_residuals_cols(headers: &[String]) -> (Option<usize>, Option<usize>, Option<usize>, Option<usize>) {
    let ds_col = find_header_col(headers, |h| h == "data set" || h == "dataset" || h == "group");
    let y_col = find_header_col(headers, |h| {
        (h.contains("residual") || h.contains("resid"))
            && !h.contains("weighted")
            && !h.contains("studentized")
            && !h.contains("normal")
            && !h.contains("qq")
            && !h.contains("q-q")
            && !h.contains("expected")
            && !h.contains("abs")
            && !h.contains("squared")
    })
    .or_else(|| {
        find_header_col(headers, |h| {
            (h.contains("residual") || h.contains("resid")) && !h.contains("normal") && !h.contains("qq") && !h.contains("q-q")
        })
    });
    let x_col = find_header_col(headers, |h| {
        (h.contains("predicted") && !h.contains("class"))
            || h.contains("fitted")
            || h.contains("y_hat")
            || h.contains("yhat")
            || h.contains("y-hat")
            || h.contains('\u{0177}')
            || h.contains("model y")
    })
    .or_else(|| find_header_col(headers, |h| h.contains("predicted")))
    .or_else(|| find_header_col(headers, |h| h == "x" || h.starts_with("x ") || h.ends_with(" (x)")));
    let obs_col = find_header_col(headers, |h| h.contains("observed"));
    (ds_col, x_col, y_col, obs_col)
}

fn extract_residuals_tab_points(res: &ResultSheet, tab: &ResultTab, graph: &GraphSheet, doc: &ProjectDocument) -> Vec<DiagPoint> {
    let (ds_col, x_col, y_col, obs_col) = extract_residuals_cols(&tab.column_headers);
    let target_label = selected_dataset_label_in_tab(res, tab, ds_col, graph, doc);
    let legacy_row_prefix = if ds_col.is_none() { selected_dataset_prefix_in_row_labels(res, tab, graph, doc) } else { None };
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let ds_opt = resolve_analysis_source_data_sheet(res, graph, doc);
    let fallback_sheet_id = ds_opt.map(|d| d.id.clone()).unwrap_or_else(|| graph.spec.primary_source_sheet_id.clone());

    let pairs: Vec<(usize, f64, f64)> = tab
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
            {
                return None;
            }
            if let Some(prefix) = legacy_row_prefix.as_deref() {
                let row_lbl = tab.row_labels.get(i).map(String::as_str).unwrap_or("");
                if !matches!(row_label_dataset_prefix(row_lbl), Some((p, _)) if p == prefix) {
                    return None;
                }
            }
            let (xv, yv) = match (x_col, y_col, obs_col) {
                (Some(xc), Some(yc), _) => (result_cell_num(row.get(xc)?)?, result_cell_num(row.get(yc)?)?),
                (Some(xc), None, Some(oc)) if xc != oc => {
                    let fitted = result_cell_num(row.get(xc)?)?;
                    let observed = result_cell_num(row.get(oc)?)?;
                    (fitted, observed - fitted)
                }
                (None, Some(yc), _) => {
                    let yv = result_cell_num(row.get(yc)?)?;
                    let xv = row.iter().enumerate().find_map(|(ci, c)| if ci != yc { result_cell_num(c) } else { None }).unwrap_or((i + 1) as f64);
                    (xv, yv)
                }
                _ => match (row.first().and_then(result_cell_num), row.get(1).and_then(result_cell_num)) {
                    (Some(a), Some(b)) => (a, b),
                    (Some(r), None) => ((i + 1) as f64, r),
                    _ => return None,
                },
            };
            Some((i, xv, yv))
        })
        .collect();

    if pairs.is_empty() {
        return Vec::new();
    }

    let source_col = match &res.analysis.kind {
        AnalysisKind::GeneralizedLinearModel(glm) => glm.response_column,
        _ => ds_idx,
    };
    let (resolved_sources, series_label) = if let Some(ds) = ds_opt {
        let (all_rep_sources, lbl) = collect_ds_active_point_sources(ds, source_col, true);
        if all_rep_sources.len() == pairs.len() {
            (Some(all_rep_sources), lbl)
        } else {
            let (row_sources, lbl_row) = collect_ds_active_point_sources(ds, source_col, false);
            if row_sources.len() == pairs.len() { (Some(row_sources), lbl_row) } else { (None, lbl) }
        }
    } else {
        (None, target_label.clone().or_else(|| legacy_row_prefix.clone()).unwrap_or_else(|| graph.spec.title.clone()))
    };
    let resolved_label = if series_label.is_empty() { graph.spec.title.clone() } else { series_label };
    let filtered_by_dataset = target_label.is_some() || legacy_row_prefix.is_some();

    pairs
        .into_iter()
        .enumerate()
        .map(|(k, (tab_row_idx, x, y))| {
            let fallback_row = if filtered_by_dataset {
                tab.row_labels.get(tab_row_idx).and_then(|l| row_label_dataset_prefix(l).map(|(_, idx)| idx)).unwrap_or(k)
            } else {
                tab_row_idx
            };
            let source = resolved_sources.as_ref().and_then(|v| v.get(k)).cloned().unwrap_or(SourceCellId {
                sheet_id: fallback_sheet_id.clone(),
                row: fallback_row,
                col: source_col,
                subcol: 0,
            });
            DiagPoint { x, y, label: tab.row_labels.get(tab_row_idx).cloned(), source, series_label: resolved_label.clone() }
        })
        .collect()
}
