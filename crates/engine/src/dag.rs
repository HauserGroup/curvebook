//! Reactive `Data -> Analysis -> Graph -> Layout` topological DAG and deterministic recalculation.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, ErrorCode, HookSource, ImportLinkSpec, InfoSheet, InternalLinkKind, LayoutItem,
    LinkSource, PlacementMode, ProjectDocument, ResultSheet, ResultStatus, SheetCellRange, SheetId,
};
use lumencraft_fit::HookContext;
use lumencraft_transform::{
    ImportPipelineOptions, ImportSourcePayload, TransformContext, apply_import_link_pipeline, detect_link_cycles,
    evaluate_data_sheet_calculated_variables, execute_analysis_transform,
};

use crate::EngineError;

/// Summary of a deterministic topological recalculation pass across a [`ProjectDocument`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecalcReport {
    /// Complete topological sheet evaluation order computed before mutating the document.
    pub evaluated_order: Vec<SheetId>,
    /// Data sheets that were unfrozen and had linked regions or calculated variables evaluated.
    pub updated_data_sheets: Vec<SheetId>,
    /// Result sheets successfully recomputed.
    pub updated_result_sheets: Vec<SheetId>,
    /// Frozen data sheets whose own links and calculated formulas were left unmutated.
    pub skipped_frozen_sheets: Vec<SheetId>,
    /// Result sheets whose recalculation produced [`ResultStatus::Failed`].
    pub failed_result_sheets: Vec<SheetId>,
    /// Downstream graph sheets whose styling, GDOs, axis breaks, brackets, and annotations were preserved.
    pub preserved_graph_sheets: Vec<SheetId>,
}

impl RecalcReport {
    /// Total count of data and result sheets successfully recomputed during the pass.
    #[must_use]
    pub fn total_updated(&self) -> usize {
        self.updated_data_sheets.len() + self.updated_result_sheets.len()
    }
}

/// Break any circular `HookSource` loops in `doc` while preserving valid acyclic hooks.
pub fn sanitize_circular_hooks(doc: &mut ProjectDocument) -> Vec<String> {
    lumencraft_format::sanitize_circular_hooks(doc)
}

/// Sanitize circular `HookSource` loops on a staged copy of `doc` and recalculate the reactive DAG atomically.
///
/// If a structural data-link, self-reference, or formula cycle remains after hook sanitization,
/// `doc` is rolled back untouched and an [`EngineError`] is returned.
pub fn recalculate_project_sanitized(doc: &mut ProjectDocument) -> Result<(usize, Vec<String>), EngineError> {
    let mut working = doc.clone();
    let diagnostics = lumencraft_format::sanitize_circular_hooks(&mut working);
    let recalculated = recalculate_project(&mut working)?;
    *doc = working;
    Ok((recalculated, diagnostics))
}

fn link_source_sheet_id(source: &LinkSource) -> Option<&SheetId> {
    match source {
        LinkSource::InternalSheet { sheet_id, .. } => Some(sheet_id),
        _ => None,
    }
}

fn hook_source_sheet_id(hook: &HookSource) -> Option<&SheetId> {
    match hook {
        HookSource::InfoConstant { info_sheet_id, .. } => info_sheet_id.as_ref(),
        HookSource::ResultScalar { result_sheet_id, .. } => Some(result_sheet_id),
        HookSource::ColumnTitleValue { data_sheet_id, .. } => Some(data_sheet_id),
    }
}

fn collect_sheet_ids(doc: &ProjectDocument) -> BTreeSet<SheetId> {
    let mut nodes = BTreeSet::new();
    for s in &doc.data_sheets {
        nodes.insert(s.id.clone());
    }
    for s in &doc.info_sheets {
        nodes.insert(s.id.clone());
    }
    for s in &doc.result_sheets {
        nodes.insert(s.id.clone());
    }
    for s in &doc.graph_sheets {
        nodes.insert(s.id.clone());
    }
    for s in &doc.layout_sheets {
        nodes.insert(s.id.clone());
    }
    nodes
}

fn check_explicit_self_cycles(doc: &ProjectDocument) -> Result<(), EngineError> {
    for ds in &doc.data_sheets {
        if ds.info_sheet_id.as_ref() == Some(&ds.id) {
            return Err(EngineError::new(format!("self-referential info sheet binding detected on `{}`", ds.id)));
        }
        for region in &ds.linked_regions {
            if link_source_sheet_id(&region.spec.source) == Some(&ds.id) {
                return Err(EngineError::new(format!("self-referential link dependency detected on sheet `{}`", ds.id)));
            }
        }
    }

    for res in &doc.result_sheets {
        if res.analysis.source_sheet_id == res.id {
            return Err(EngineError::new(format!("self-referential analysis source detected on result sheet `{}`", res.id)));
        }
        for hook in res.analysis.kind.hooked_sources() {
            if hook_source_sheet_id(&hook) == Some(&res.id) {
                return Err(EngineError::new(format!("self-referential hook binding detected on result sheet `{}`", res.id)));
            }
        }
    }

    for graph in &doc.graph_sheets {
        if graph.spec.primary_source_sheet_id == graph.id {
            return Err(EngineError::new(format!("self-referential primary source detected on graph sheet `{}`", graph.id)));
        }
        for ann in &graph.spec.annotations {
            if let lumencraft_doc::GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. }
            | lumencraft_doc::GraphAnnotation::ModelEquation { result_sheet_id, .. } = ann
                && result_sheet_id == &graph.id
            {
                return Err(EngineError::new(format!("self-referential annotation detected on graph sheet `{}`", graph.id)));
            }
        }
    }

    for layout in &doc.layout_sheets {
        for item in &layout.spec.items {
            let self_ref = match item {
                LayoutItem::LiveGraph { graph_sheet_id, .. } => graph_sheet_id == &layout.id,
                LayoutItem::LiveTable { source_sheet_id, .. } => source_sheet_id == &layout.id,
                _ => false,
            };
            if self_ref {
                return Err(EngineError::new(format!("self-referential item detected on layout sheet `{}`", layout.id)));
            }
        }
    }

    Ok(())
}

/// Compute a deterministic topological evaluation order for all sheets in `doc`.
///
/// # Errors
/// Returns [`EngineError`] if any link cycle, self-reference, or cross-sheet dependency cycle exists.
pub fn topological_order(doc: &ProjectDocument) -> Result<Vec<SheetId>, EngineError> {
    detect_link_cycles(&doc.data_sheets).map_err(|err| EngineError::new(format!("link cycle detected: {err}")))?;
    check_explicit_self_cycles(doc)?;

    let nodes = collect_sheet_ids(doc);
    let mut in_degree: BTreeMap<SheetId, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<SheetId, Vec<SheetId>> = BTreeMap::new();
    for id in &nodes {
        in_degree.insert(id.clone(), 0);
        dependents.insert(id.clone(), Vec::new());
    }

    for id in &nodes {
        let deps = doc.upstream_dependencies(id);
        let mut unique_existing: BTreeSet<SheetId> = BTreeSet::new();
        for dep in deps {
            if &dep == id {
                return Err(EngineError::new(format!("self-referential dependency detected on sheet `{id}`")));
            }
            if nodes.contains(&dep) {
                unique_existing.insert(dep);
            }
        }
        if let Some(deg) = in_degree.get_mut(id) {
            *deg = unique_existing.len();
        }
        for dep in unique_existing {
            if let Some(list) = dependents.get_mut(&dep) {
                list.push(id.clone());
            }
        }
    }

    let mut queue: VecDeque<SheetId> = in_degree.iter().filter(|&(_, &deg)| deg == 0).map(|(id, _)| id.clone()).collect();
    let mut order = Vec::with_capacity(nodes.len());

    while let Some(curr) = queue.pop_front() {
        order.push(curr.clone());
        if let Some(children) = dependents.get(&curr) {
            for child in children {
                if let Some(deg) = in_degree.get_mut(child) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(child.clone());
                    }
                }
            }
        }
    }

    if order.len() != nodes.len() {
        return Err(EngineError::new("cyclic dependency detected across project sheets"));
    }
    Ok(order)
}

/// Recalculate linked data regions, calculated variables, and analytical result sheets in topological order.
///
/// Freezing, cycle detection, and atomic rollback guarantee `doc` is not corrupted if an error occurs.
///
/// # Errors
/// Returns [`EngineError`] if any cyclic dependency or fatal formula cycle is detected.
/// Individual analysis errors are recorded on the corresponding [`ResultSheet`] with [`ResultStatus::Failed`].
pub fn recalculate_project(doc: &mut ProjectDocument) -> Result<usize, EngineError> {
    let report = recalculate_project_with_report(doc)?;
    Ok(report.total_updated())
}

/// Recalculate the full project DAG and return a detailed [`RecalcReport`].
///
/// # Errors
/// Returns [`EngineError`] on cyclic sheet/link/formula dependencies; `doc` remains unmodified on error.
pub fn recalculate_project_with_report(doc: &mut ProjectDocument) -> Result<RecalcReport, EngineError> {
    let order = topological_order(doc)?;
    let mut working = doc.clone();
    let mut report = RecalcReport { evaluated_order: order.clone(), ..RecalcReport::default() };

    for sheet_id in order {
        if let Some(ds_idx) = working.data_sheets.iter().position(|s| s.id == sheet_id) {
            evaluate_data_sheet_node(&mut working, ds_idx, &mut report)?;
        } else if let Some(res_idx) = working.result_sheets.iter().position(|s| s.id == sheet_id) {
            let succeeded = recompute_result_sheet(&mut working, res_idx);
            if succeeded {
                report.updated_result_sheets.push(sheet_id);
            } else {
                report.failed_result_sheets.push(sheet_id);
            }
        } else if working.graph_sheets.iter().any(|g| g.id == sheet_id) {
            report.preserved_graph_sheets.push(sheet_id);
        }
    }

    *doc = working;
    Ok(report)
}

fn evaluate_data_sheet_node(working: &mut ProjectDocument, ds_idx: usize, report: &mut RecalcReport) -> Result<(), EngineError> {
    let sheet_id = working.data_sheets[ds_idx].id.clone();
    if working.data_sheets[ds_idx].frozen {
        report.skipped_frozen_sheets.push(sheet_id);
        return Ok(());
    }

    let had_links = !working.data_sheets[ds_idx].linked_regions.is_empty();
    refresh_data_sheet_links(working, ds_idx)?;

    let info_opt =
        working.data_sheets[ds_idx].info_sheet_id.as_ref().and_then(|id| working.info_sheet(id)).or_else(|| working.info_sheets.first()).cloned();

    let eval_cols = evaluate_data_sheet_calculated_variables(&mut working.data_sheets[ds_idx], info_opt.as_ref())
        .map_err(|err| EngineError::new(format!("calculated variables on sheet `{sheet_id}` failed: {err}")))?;

    if had_links || !eval_cols.is_empty() {
        report.updated_data_sheets.push(sheet_id);
    }
    Ok(())
}

fn clear_cell_range(target: &mut DataSheet, range: SheetCellRange) {
    let row_count = target.archetype.row_count();
    if row_count == 0 {
        return;
    }
    let min_row = range.start.row.min(range.end.row);
    let max_row = range.start.row.max(range.end.row).min(row_count - 1);
    let min_col = range.start.col.min(range.end.col);
    let max_col = range.start.col.max(range.end.col);
    let min_sub = range.start.subcol.min(range.end.subcol);
    let max_sub = range.start.subcol.max(range.end.subcol);

    for r in min_row..=max_row {
        for c in min_col..=max_col {
            if target.archetype.data_cell(CellCoord::new(r, c, min_sub)).is_none() {
                break;
            }
            for s in min_sub..=max_sub {
                if target.set_cell(CellCoord::new(r, c, s), Cell::empty()).is_err() {
                    break;
                }
            }
        }
    }
}

fn make_refresh_spec(spec: &ImportLinkSpec, existing_range_start: CellCoord) -> ImportLinkSpec {
    let mut refresh = spec.clone();
    match refresh.placement.mode {
        PlacementMode::AppendRows | PlacementMode::AppendColumns | PlacementMode::InsertRows => {
            refresh.placement.mode = PlacementMode::Overwrite;
            refresh.placement.anchor = existing_range_start;
        }
        PlacementMode::Overwrite => {}
    }
    refresh
}

fn refresh_single_region(working: &ProjectDocument, target: &mut DataSheet, reg_idx: usize) -> Result<(), EngineError> {
    let region = target.linked_regions[reg_idx].clone();
    let refresh_spec = make_refresh_spec(&region.spec, region.target_range.start);
    let opts = ImportPipelineOptions {
        baseline: None,
        exclude_coords: Vec::new(),
        exclude_row_predicate: None,
        link_id: Some(region.id.clone()),
        provenance_summary: Some(region.provenance_summary.clone()),
    };

    let has_internal_source = matches!(
        &region.spec.source,
        LinkSource::InternalSheet { sheet_id, .. }
            if working.data_sheet(sheet_id).is_some() || working.result_sheet(sheet_id).is_some()
    );
    if has_internal_source && region.target_range.start == refresh_spec.placement.anchor {
        clear_cell_range(target, region.target_range);
    }

    let saved_regions = std::mem::take(&mut target.linked_regions);
    let result = match &region.spec.source {
        LinkSource::InternalSheet { sheet_id, kind, tab_id, .. } => {
            if let Some(src_ds) = working.data_sheet(sheet_id) {
                Some(apply_import_link_pipeline(target, &refresh_spec, ImportSourcePayload::InternalData(src_ds), &opts, None))
            } else if let Some(src_rs) = working.result_sheet(sheet_id) {
                if *kind == InternalLinkKind::AnalyzableResultSheet
                    && tab_id.is_none()
                    && let Some(analyzable) = src_rs.analyzable_data.as_ref()
                {
                    Some(apply_import_link_pipeline(target, &refresh_spec, ImportSourcePayload::InternalData(analyzable), &opts, None))
                } else {
                    Some(apply_import_link_pipeline(target, &refresh_spec, ImportSourcePayload::InternalResult(src_rs), &opts, None))
                }
            } else {
                None
            }
        }
        LinkSource::Clipboard | LinkSource::ExternalDelimitedFile { .. } => None,
    };
    target.linked_regions = saved_regions;

    if let Some(pipe_res) = result {
        let report =
            pipe_res.map_err(|err| EngineError::new(format!("linked region `{}` refresh on sheet `{}` failed: {err}", region.id, target.id)))?;
        target.linked_regions[reg_idx].target_range = report.target_range;
        target.linked_regions[reg_idx].provenance_summary = report.provenance_summary;
    }

    Ok(())
}

fn refresh_data_sheet_links(working: &mut ProjectDocument, ds_idx: usize) -> Result<(), EngineError> {
    let region_count = working.data_sheets[ds_idx].linked_regions.len();
    if region_count == 0 {
        return Ok(());
    }

    // Clone target temporarily so `working` can be read immutably to resolve upstream payloads.
    let mut target = working.data_sheets[ds_idx].clone();
    for reg_idx in 0..region_count {
        refresh_single_region(working, &mut target, reg_idx)?;
    }
    working.data_sheets[ds_idx] = target;
    Ok(())
}

fn resolve_source_table(doc: &ProjectDocument, source_id: &SheetId) -> Option<DataSheet> {
    if let Some(ds) = doc.data_sheet(source_id) {
        return Some(ds.clone());
    }
    doc.result_sheet(source_id).and_then(|res| res.analyzable_data.clone())
}

fn resolve_origin_info_sheet_id<'a>(doc: &'a ProjectDocument, source_ds: &'a DataSheet, analysis: &AnalysisSpec) -> Option<&'a SheetId> {
    if let Some(ref id) = source_ds.info_sheet_id
        && doc.info_sheet(id).is_some()
    {
        return Some(id);
    }

    // Walk back through chained ResultSheets if `source_ds` came from `analyzable_data`.
    let mut cursor = &analysis.source_sheet_id;
    let mut seen: HashSet<&SheetId> = HashSet::new();
    while seen.insert(cursor) {
        if let Some(ds) = doc.data_sheet(cursor) {
            if let Some(ref id) = ds.info_sheet_id
                && doc.info_sheet(id).is_some()
            {
                return Some(id);
            }
            break;
        }
        if let Some(res) = doc.result_sheet(cursor) {
            cursor = &res.analysis.source_sheet_id;
        } else {
            break;
        }
    }

    for hook in analysis.kind.hooked_sources() {
        if let HookSource::InfoConstant { info_sheet_id: Some(id), .. } = hook
            && doc.info_sheet(&id).is_some()
        {
            return doc.info_sheet(&id).map(|info| &info.id);
        }
    }

    doc.info_sheets.first().map(|info| &info.id)
}

fn resolve_transform_info_sheet<'a>(doc: &'a ProjectDocument, source_ds: &'a DataSheet, analysis: &AnalysisSpec) -> Option<&'a InfoSheet> {
    for hook in analysis.kind.hooked_sources() {
        if let HookSource::InfoConstant { info_sheet_id: Some(id), .. } = hook
            && let Some(info) = doc.info_sheet(&id)
        {
            return Some(info);
        }
    }
    resolve_origin_info_sheet_id(doc, source_ds, analysis).and_then(|id| doc.info_sheet(id))
}

fn recompute_result_sheet(doc: &mut ProjectDocument, res_idx: usize) -> bool {
    let existing = doc.result_sheets[res_idx].clone();
    let Some(source_ds) = resolve_source_table(doc, &existing.analysis.source_sheet_id) else {
        doc.result_sheets[res_idx].status =
            ResultStatus::Failed { code: ErrorCode::NotFound, message: format!("source sheet `{}` not found", existing.analysis.source_sheet_id) };
        return false;
    };

    let output: Result<ResultSheet, String> = match &existing.analysis.kind {
        AnalysisKind::Transform(_) => {
            let info_opt = resolve_transform_info_sheet(doc, &source_ds, &existing.analysis);
            let ctx = TransformContext {
                info_sheet: info_opt,
                result_sheets: &doc.result_sheets,
                data_sheets: &doc.data_sheets,
                selected_data_sets: existing.analysis.selected_data_sets.clone(),
                output_id: Some(existing.id.as_str().to_owned()),
                output_title: Some(existing.title().to_owned()),
            };
            execute_analysis_transform(&source_ds, &existing.analysis, &ctx).map_err(|err| err.to_string())
        }
        AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) => {
            let primary_info_id = resolve_origin_info_sheet_id(doc, &source_ds, &existing.analysis);
            let hooks = HookContext {
                info_sheets: &doc.info_sheets,
                result_sheets: &doc.result_sheets,
                data_sheets: &doc.data_sheets,
                primary_info_sheet_id: primary_info_id,
                visited_sheets: HashSet::new(),
            };
            lumencraft_fit::execute_analysis(&existing.analysis, &source_ds, &hooks).map_err(|err| err.to_string())
        }
        _ => lumencraft_stats::run_analysis(&source_ds, &existing.analysis).map_err(|err| err.to_string()),
    };

    let target = &mut doc.result_sheets[res_idx];
    match output {
        Ok(mut computed) => {
            computed.id = existing.id;
            computed.analysis = existing.analysis;
            computed.floating_notes = existing.floating_notes;
            computed.color_tag = existing.color_tag;
            computed.status = ResultStatus::UpToDate;
            *target = computed;
            true
        }
        Err(message) => {
            target.status = ResultStatus::Failed { code: ErrorCode::SolverFailed, message };
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{
        AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, ResultSheet, TableArchetype,
        TransformAnalysisSpec,
    };

    use super::*;

    #[test]
    fn self_cycle_on_result_sheet_is_rejected_without_corrupting_doc() {
        let mut doc = ProjectDocument::new("self-cycle-test").expect("doc");
        let tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("col table");
        let ds = DataSheet::new("data-1", "Data", TableArchetype::Column(tbl)).expect("data sheet");
        doc.data_sheets.push(ds);

        let analysis =
            AnalysisSpec::new("res-1", "res-1", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false }))
                .expect("analysis spec");
        let mut res = ResultSheet::new(analysis).expect("result sheet");
        let out_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("out table");
        res.analyzable_data = Some(DataSheet::new("res-1-out", "Out", TableArchetype::Column(out_tbl)).expect("out"));
        doc.result_sheets.push(res);

        let before = doc.clone();
        let err = recalculate_project(&mut doc).expect_err("self-cycle must fail");
        assert!(err.to_string().contains("self-referential"));
        assert_eq!(doc.result_sheets.len(), before.result_sheets.len());
        assert_eq!(doc.result_sheets[0].status, before.result_sheets[0].status);
    }

    #[test]
    fn empty_unlinked_data_sheet_counts_zero_and_recalc_report_tracks_order() {
        let mut doc = ProjectDocument::new("report-test").expect("doc");
        let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("col table");
        tbl.data_sets[0].cells[0][0] = Cell::number(4.0);
        let ds = DataSheet::new("data-1", "Data", TableArchetype::Column(tbl)).expect("ds");
        doc.data_sheets.push(ds);

        let report = recalculate_project_with_report(&mut doc).expect("recalc succeeds");
        assert_eq!(report.evaluated_order, vec![SheetId::new("data-1")]);
        assert!(report.updated_data_sheets.is_empty());
        assert_eq!(report.total_updated(), 0);
    }
}
