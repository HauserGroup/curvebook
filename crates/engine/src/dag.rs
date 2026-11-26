//! Reactive `Data -> Analysis -> Graph -> Layout` topological DAG and deterministic recalculation.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};

use curvebook_doc::analysis::{AnalysisRecomputeStrategy, descriptor_for_kind};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, CiBound, DataSetColumn, DataSheet, ErrorCode, HookSource, ImportLinkSpec,
    InternalLinkKind, LayoutItem, LinkSource, PlacementMode, ProjectDocument, ResultCell, ResultSheet, ResultStatus, SheetCellRange, SheetId,
    TableArchetype, XColumnMode,
};
use curvebook_fit::HookContext;
use curvebook_format::MAX_CSV_FILE_BYTES;
use curvebook_transform::{
    ImportPipelineOptions, ImportSourcePayload, TransformContext, apply_import_link_pipeline, detect_link_cycles,
    evaluate_data_sheet_calculated_variables, execute_analysis_transform,
};

use crate::{EngineError, ExternalPathPolicy};

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

/// Summary of a [`ResultSheet`] whose evaluation ended in [`ResultStatus::Failed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedResultSheet {
    /// Failed result sheet identifier.
    pub sheet_id: SheetId,
    /// User-visible title of the failed result sheet.
    pub title: String,
    /// Structured engine error code recorded on the sheet.
    pub code: ErrorCode,
    /// Diagnostic failure reason recorded on the sheet.
    pub message: String,
}

impl FailedResultSheet {
    /// Format a compact JSON object describing this failed result sheet (`sheet_id`, `title`, `code`, `message`).
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "sheet_id": self.sheet_id.as_str(),
            "title": self.title,
            "code": self.code.as_str(),
            "message": self.message,
        })
    }
}

/// Collect structured diagnostics for every [`ResultSheet`] currently in [`ResultStatus::Failed`].
#[must_use]
pub fn collect_failed_result_sheets(doc: &ProjectDocument) -> Vec<FailedResultSheet> {
    doc.result_sheets
        .iter()
        .filter_map(|rs| match &rs.status {
            ResultStatus::Failed { code, message } => {
                Some(FailedResultSheet { sheet_id: rs.id.clone(), title: rs.title().to_owned(), code: *code, message: message.clone() })
            }
            _ => None,
        })
        .collect()
}

pub(crate) fn collect_failed_result_sheets_for_ids(doc: &ProjectDocument, ids: &[SheetId]) -> Vec<FailedResultSheet> {
    ids.iter()
        .filter_map(|id| {
            let rs = doc.result_sheet(id)?;
            match &rs.status {
                ResultStatus::Failed { code, message } => {
                    Some(FailedResultSheet { sheet_id: rs.id.clone(), title: rs.title().to_owned(), code: *code, message: message.clone() })
                }
                _ => None,
            }
        })
        .collect()
}

/// Break any circular `HookSource` loops in `doc` while preserving valid acyclic hooks.
pub fn sanitize_circular_hooks(doc: &mut ProjectDocument) -> Vec<String> {
    curvebook_format::sanitize_circular_hooks(doc)
}

/// Sanitize circular `HookSource` loops on a staged copy of `doc` and recalculate the reactive DAG atomically,
/// returning the full [`RecalcReport`] (including any failed result sheet IDs).
///
/// # Errors
/// Returns [`EngineError`] if a structural data-link, self-reference, or formula cycle remains after sanitization.
pub fn recalculate_project_sanitized_with_report(doc: &mut ProjectDocument) -> Result<(RecalcReport, Vec<String>), EngineError> {
    let mut working = doc.clone();
    let diagnostics = curvebook_format::sanitize_circular_hooks(&mut working);
    let report = recalculate_project_with_report(&mut working)?;
    *doc = working;
    Ok((report, diagnostics))
}

/// Sanitize circular `HookSource` loops on a staged copy of `doc` and recalculate the reactive DAG atomically.
///
/// If a structural data-link, self-reference, or formula cycle remains after hook sanitization,
/// `doc` is rolled back untouched and an [`EngineError`] is returned.
pub fn recalculate_project_sanitized(doc: &mut ProjectDocument) -> Result<(usize, Vec<String>), EngineError> {
    let (report, diagnostics) = recalculate_project_sanitized_with_report(doc)?;
    Ok((report.total_updated(), diagnostics))
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
            return Err(EngineError::with_code(
                crate::ErrorCode::CycleDetected,
                format!("self-referential info sheet binding detected on `{}`", ds.id),
            ));
        }
        for region in &ds.linked_regions {
            if link_source_sheet_id(&region.spec.source) == Some(&ds.id) {
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("self-referential link dependency detected on sheet `{}`", ds.id),
                ));
            }
        }
    }

    for res in &doc.result_sheets {
        if res.analysis.source_sheet_id == res.id {
            return Err(EngineError::with_code(
                crate::ErrorCode::CycleDetected,
                format!("self-referential analysis source detected on result sheet `{}`", res.id),
            ));
        }
        for hook in res.analysis.kind.hooked_sources() {
            if hook_source_sheet_id(&hook) == Some(&res.id) {
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("self-referential hook binding detected on result sheet `{}`", res.id),
                ));
            }
        }
    }

    for graph in &doc.graph_sheets {
        if graph.spec.primary_source_sheet_id == graph.id {
            return Err(EngineError::with_code(
                crate::ErrorCode::CycleDetected,
                format!("self-referential primary source detected on graph sheet `{}`", graph.id),
            ));
        }
        for series in &graph.spec.series {
            if series.source_sheet_id == graph.id {
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("self-referential series source detected on graph sheet `{}`", graph.id),
                ));
            }
            if let Some(ov) = &series.regression_overlay
                && ov.result_sheet_id == graph.id
            {
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("self-referential regression overlay detected on graph sheet `{}`", graph.id),
                ));
            }
        }
        for cmp in &graph.spec.comparisons {
            if cmp.result_sheet_id == graph.id {
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("self-referential comparison bracket detected on graph sheet `{}`", graph.id),
                ));
            }
        }
        if let Some(cld) = &graph.spec.compact_letter_display
            && cld.result_sheet_id == graph.id
        {
            return Err(EngineError::with_code(
                crate::ErrorCode::CycleDetected,
                format!("self-referential compact letter display detected on graph sheet `{}`", graph.id),
            ));
        }
        for ann in &graph.spec.annotations {
            if let curvebook_doc::GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. }
            | curvebook_doc::GraphAnnotation::ModelEquation { result_sheet_id, .. } = ann
                && result_sheet_id == &graph.id
            {
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("self-referential annotation detected on graph sheet `{}`", graph.id),
                ));
            }
        }
        for hook in graph.spec.inline_hooks() {
            if hook.upstream_sheet_id() == Some(&graph.id) {
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("self-referential inline hook detected on graph sheet `{}`", graph.id),
                ));
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
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("self-referential item detected on layout sheet `{}`", layout.id),
                ));
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
    detect_link_cycles(&doc.data_sheets)
        .map_err(|err| EngineError::with_code(crate::ErrorCode::CycleDetected, format!("link cycle detected: {err}")))?;
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
                return Err(EngineError::with_code(crate::ErrorCode::CycleDetected, format!("self-referential dependency detected on sheet `{id}`")));
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
        return Err(EngineError::with_code(crate::ErrorCode::CycleDetected, "cyclic dependency detected across project sheets"));
    }
    curvebook_format::detect_dependency_cycles(doc)?;
    Ok(order)
}

fn push_spec_hook_dep(deps: &mut Vec<SheetId>, hook: &curvebook_doc::HookSource, fallback_info: Option<&SheetId>) {
    match hook {
        curvebook_doc::HookSource::InfoConstant { info_sheet_id: Some(id), .. } => deps.push(id.clone()),
        curvebook_doc::HookSource::InfoConstant { info_sheet_id: None, .. } => {
            if let Some(id) = fallback_info {
                deps.push(id.clone());
            }
        }
        curvebook_doc::HookSource::ResultScalar { result_sheet_id, .. } => deps.push(result_sheet_id.clone()),
        curvebook_doc::HookSource::ColumnTitleValue { data_sheet_id, .. } => deps.push(data_sheet_id.clone()),
    }
}

fn validate_spec_deps_acyclic(doc: &ProjectDocument, sheet_id: &SheetId, spec_deps: Vec<SheetId>, kind: &str) -> Result<(), EngineError> {
    if spec_deps.iter().any(|dep| dep == sheet_id) {
        return Err(EngineError::with_code(
            crate::ErrorCode::CycleDetected,
            format!("self-referential dependency detected on {kind} sheet `{sheet_id}`"),
        ));
    }
    let mut visited = BTreeSet::new();
    let mut queue: VecDeque<SheetId> = spec_deps.into_iter().collect();
    while let Some(curr) = queue.pop_front() {
        if &curr == sheet_id {
            return Err(EngineError::with_code(
                crate::ErrorCode::CycleDetected,
                format!("cyclic dependency detected involving {kind} sheet `{sheet_id}`"),
            ));
        }
        if !visited.insert(curr.clone()) {
            continue;
        }
        for up in doc.upstream_dependencies(&curr) {
            if &up == sheet_id {
                return Err(EngineError::with_code(
                    crate::ErrorCode::CycleDetected,
                    format!("cyclic dependency detected involving {kind} sheet `{sheet_id}`"),
                ));
            }
            if !visited.contains(&up) {
                queue.push_back(up);
            }
        }
    }
    Ok(())
}

/// Validate that inline template hooks do not self-reference `sheet_id` or point at wrong/missing sheets.
pub(crate) fn validate_inline_hook_refs(doc: &ProjectDocument, sheet_id: &SheetId, hooks: &[curvebook_doc::HookSource]) -> Result<(), EngineError> {
    for hook in hooks {
        if hook.upstream_sheet_id() == Some(sheet_id) {
            return Err(EngineError::with_code(
                crate::ErrorCode::CycleDetected,
                format!("self-referential inline hook target `{sheet_id}` is not allowed on `{sheet_id}`"),
            ));
        }
        match hook {
            curvebook_doc::HookSource::InfoConstant { info_sheet_id: Some(info_id), .. } => {
                let _ = crate::cmd::helpers::require_info_sheet(doc, info_id)?;
            }
            curvebook_doc::HookSource::InfoConstant { info_sheet_id: None, .. } => {}
            curvebook_doc::HookSource::ResultScalar { result_sheet_id, .. } => {
                let _ = crate::cmd::helpers::require_result_sheet(doc, result_sheet_id)?;
            }
            curvebook_doc::HookSource::ColumnTitleValue { data_sheet_id, .. } => {
                let _ = crate::cmd::helpers::require_data_sheet(doc, data_sheet_id)?;
            }
        }
    }
    Ok(())
}

/// Validate that candidate `spec` on `graph_id` introduces no self-reference or dependency cycle.
pub(crate) fn validate_graph_spec_acyclic(doc: &ProjectDocument, graph_id: &SheetId, spec: &curvebook_doc::GraphSpec) -> Result<(), EngineError> {
    let default_info = doc.info_sheets.first().map(|s| &s.id);
    let paired_info = doc.paired_info_sheet_id(&spec.primary_source_sheet_id).or(default_info);
    let mut deps = Vec::new();
    deps.push(spec.primary_source_sheet_id.clone());
    for series in &spec.series {
        deps.push(series.source_sheet_id.clone());
        if let Some(ov) = &series.regression_overlay {
            deps.push(ov.result_sheet_id.clone());
        }
    }
    for cmp in &spec.comparisons {
        deps.push(cmp.result_sheet_id.clone());
    }
    if let Some(cld) = &spec.compact_letter_display {
        deps.push(cld.result_sheet_id.clone());
    }
    for ann in &spec.annotations {
        if let curvebook_doc::GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. }
        | curvebook_doc::GraphAnnotation::ModelEquation { result_sheet_id, .. } = ann
        {
            deps.push(result_sheet_id.clone());
        }
    }
    for hook in spec.inline_hooks() {
        push_spec_hook_dep(&mut deps, &hook, paired_info);
    }
    validate_spec_deps_acyclic(doc, graph_id, deps, "graph")
}

/// Validate that candidate `spec` on `layout_id` introduces no self-reference or dependency cycle.
pub(crate) fn validate_layout_spec_acyclic(doc: &ProjectDocument, layout_id: &SheetId, spec: &curvebook_doc::LayoutSpec) -> Result<(), EngineError> {
    let default_info = doc.info_sheets.first().map(|s| s.id.clone());
    let mut deps = Vec::new();
    for item in &spec.items {
        match item {
            curvebook_doc::LayoutItem::LiveGraph { graph_sheet_id, .. } => deps.push(graph_sheet_id.clone()),
            curvebook_doc::LayoutItem::LiveTable { source_sheet_id, .. } => deps.push(source_sheet_id.clone()),
            curvebook_doc::LayoutItem::TextCallout { .. } | curvebook_doc::LayoutItem::EmbeddedImage { .. } => {}
        }
    }
    for hook in spec.inline_hooks() {
        push_spec_hook_dep(&mut deps, &hook, default_info.as_ref());
    }
    validate_spec_deps_acyclic(doc, layout_id, deps, "layout")
}

/// Recalculate linked data regions, calculated variables, and analytical result sheets in topological order.
///
/// Direct Rust callers modifying `ProjectDocument` structs outside `Session` commands use this
/// full pass; incremental dirty-node recalculation is exposed via [`recalculate_dirty_with_report`].
/// Freezing, cycle detection, and atomic rollback guarantee `doc` is not corrupted if an error occurs.
///
/// # Errors
/// Returns [`EngineError`] if any cyclic dependency or fatal formula cycle is detected.
/// Individual analysis errors are recorded on the corresponding [`ResultSheet`] with [`ResultStatus::Failed`].
pub fn recalculate_project(doc: &mut ProjectDocument) -> Result<usize, EngineError> {
    let report = recalculate_project_with_report(doc)?;
    Ok(report.total_updated())
}

fn check_recalc_cancelled(cancelled: Option<&AtomicBool>) -> Result<(), EngineError> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(EngineError::with_code(ErrorCode::Cancelled, "project recalculation cancelled").with_retryable(true));
    }
    Ok(())
}

/// Recalculate the full project DAG and return a detailed [`RecalcReport`].
///
/// # Errors
/// Returns [`EngineError`] on cyclic sheet/link/formula dependencies; `doc` remains unmodified on error.
pub fn recalculate_project_with_report(doc: &mut ProjectDocument) -> Result<RecalcReport, EngineError> {
    let (report, _) = recalculate_project_with_links_report(doc, None, None)?;
    Ok(report)
}

/// Recalculate the full project DAG with cooperative cancellation polling (`AUD-13-22`).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::Cancelled`] when `cancelled` is set, or on cyclic
/// sheet/link/formula dependencies; `doc` remains unmodified on error.
pub fn recalculate_project_with_report_cancellable(doc: &mut ProjectDocument, cancelled: Option<&AtomicBool>) -> Result<RecalcReport, EngineError> {
    let (report, _) = recalculate_project_with_links_report(doc, None, cancelled)?;
    Ok(report)
}

/// Incrementally recalculate the dirty transitive closure seeded by `dirty_seeds` (plus any
/// [`ResultStatus::Dirty`] sheets) in deterministic topological order.
///
/// Unrelated branches of the DAG are skipped so edits do not recompute unaffected analyses.
///
/// # Errors
/// Returns [`EngineError`] on cyclic dependencies or failing linked/calculated updates; `doc` remains
/// unmodified on error.
pub fn recalculate_dirty_with_report(doc: &mut ProjectDocument, dirty_seeds: &[SheetId]) -> Result<RecalcReport, EngineError> {
    let (report, _) = recalculate_dirty_with_links_report(doc, dirty_seeds, None, None)?;
    Ok(report)
}

/// Incrementally recalculate the dirty transitive closure of `dirty_seeds` with cooperative
/// cancellation polling (`AUD-13-22`).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::Cancelled`] when `cancelled` is set, or on cyclic
/// dependencies / failing linked or calculated updates; `doc` remains unmodified on error.
pub fn recalculate_dirty_with_report_cancellable(
    doc: &mut ProjectDocument,
    dirty_seeds: &[SheetId],
    cancelled: Option<&AtomicBool>,
) -> Result<RecalcReport, EngineError> {
    let (report, _) = recalculate_dirty_with_links_report(doc, dirty_seeds, None, cancelled)?;
    Ok(report)
}

/// Recalculate the full project DAG, optionally refreshing external delimited file links under `external_policy`,
/// and return the [`RecalcReport`] alongside the number of linked regions refreshed.
///
/// # Errors
/// Returns [`EngineError`] on cyclic dependencies, cancellation, unreadable/blocked external links,
/// or failing linked refreshes / calculated variables; `doc` remains unmodified on error.
pub(crate) fn recalculate_project_with_links_report(
    doc: &mut ProjectDocument,
    external_policy: Option<&ExternalPathPolicy>,
    cancelled: Option<&AtomicBool>,
) -> Result<(RecalcReport, usize), EngineError> {
    recalculate_project_scoped(doc, external_policy, None, cancelled)
}

/// Incrementally recalculate the dirty transitive closure of `dirty_seeds` (plus any [`ResultStatus::Dirty`]
/// sheets and external-link sheets when `external_policy` is `Some`), optionally refreshing external
/// delimited file links under `external_policy`.
///
/// # Errors
/// Returns [`EngineError`] on cyclic dependencies, cancellation, unreadable/blocked external links,
/// or failing linked refreshes / calculated variables; `doc` remains unmodified on error.
pub(crate) fn recalculate_dirty_with_links_report(
    doc: &mut ProjectDocument,
    dirty_seeds: &[SheetId],
    external_policy: Option<&ExternalPathPolicy>,
    cancelled: Option<&AtomicBool>,
) -> Result<(RecalcReport, usize), EngineError> {
    recalculate_project_scoped(doc, external_policy, Some(dirty_seeds), cancelled)
}

pub(crate) fn data_sheet_has_calculated_variables(ds: &DataSheet) -> bool {
    matches!(
        &ds.archetype,
        TableArchetype::MultipleVariables(mv) if mv.variables.iter().any(|v| matches!(v.source, curvebook_doc::VariableSource::Calculated { .. }))
    )
}

fn recalculate_project_scoped(
    doc: &mut ProjectDocument,
    external_policy: Option<&ExternalPathPolicy>,
    dirty_seeds: Option<&[SheetId]>,
    cancelled: Option<&AtomicBool>,
) -> Result<(RecalcReport, usize), EngineError> {
    check_recalc_cancelled(cancelled)?;
    let order = topological_order(doc)?;
    let dirty_closure = dirty_seeds.map(|seeds| compute_dirty_closure(doc, &order, seeds, external_policy));
    let in_scope = |sheet_id: &SheetId| dirty_closure.as_ref().is_none_or(|dirty| dirty.contains(sheet_id));

    // Only dirty unfrozen data sheets with links or calculated variables can mutate data or fail.
    // Also stage a lightweight working clone when cancellation could abort mid-pass across multiple
    // in-scope result sheets so direct DAG callers observe atomic rollback on cancellation.
    let has_fallible_dirty_ds =
        doc.data_sheets.iter().any(|ds| in_scope(&ds.id) && !ds.frozen && (!ds.linked_regions.is_empty() || data_sheet_has_calculated_variables(ds)));
    let multi_result_cancellable = cancelled.is_some() && doc.result_sheets.iter().filter(|rs| in_scope(&rs.id)).take(2).count() > 1;

    if !has_fallible_dirty_ds && !multi_result_cancellable {
        let mut report = RecalcReport { evaluated_order: order.clone(), ..RecalcReport::default() };
        for sheet_id in order {
            check_recalc_cancelled(cancelled)?;
            if !in_scope(&sheet_id) {
                continue;
            }
            if let Some(ds) = doc.data_sheet(&sheet_id) {
                if ds.frozen {
                    report.skipped_frozen_sheets.push(sheet_id);
                }
            } else if let Some(res_idx) = doc.result_sheets.iter().position(|s| s.id == sheet_id) {
                if recompute_result_sheet(doc, res_idx, cancelled)? {
                    report.updated_result_sheets.push(sheet_id);
                } else {
                    report.failed_result_sheets.push(sheet_id);
                }
            } else if doc.graph_sheets.iter().any(|g| g.id == sheet_id) {
                report.preserved_graph_sheets.push(sheet_id);
            }
        }
        crate::cmd::helpers::prune_out_of_bounds_point_overrides(doc);
        return Ok((report, 0));
    }

    let saved_checkpoints = std::mem::take(&mut doc.checkpoints);
    let saved_assets = std::mem::take(&mut doc.image_assets);
    let mut working = doc.clone();
    let mut report = RecalcReport { evaluated_order: order.clone(), ..RecalcReport::default() };
    let mut refreshed_links = 0usize;

    for sheet_id in order {
        if let Err(err) = check_recalc_cancelled(cancelled) {
            doc.checkpoints = saved_checkpoints;
            doc.image_assets = saved_assets;
            return Err(err);
        }
        if !in_scope(&sheet_id) {
            continue;
        }
        if let Some(ds_idx) = working.data_sheets.iter().position(|s| s.id == sheet_id) {
            let evaluated = evaluate_data_sheet_node(&mut working, ds_idx, &mut report, external_policy, cancelled);
            match evaluated {
                Ok(refreshed) => refreshed_links = refreshed_links.saturating_add(refreshed),
                Err(err) => {
                    doc.checkpoints = saved_checkpoints;
                    doc.image_assets = saved_assets;
                    return Err(err);
                }
            }
        } else if let Some(res_idx) = working.result_sheets.iter().position(|s| s.id == sheet_id) {
            match recompute_result_sheet(&mut working, res_idx, cancelled) {
                Ok(true) => report.updated_result_sheets.push(sheet_id),
                Ok(false) => report.failed_result_sheets.push(sheet_id),
                Err(err) => {
                    doc.checkpoints = saved_checkpoints;
                    doc.image_assets = saved_assets;
                    return Err(err);
                }
            }
        } else if working.graph_sheets.iter().any(|g| g.id == sheet_id) {
            report.preserved_graph_sheets.push(sheet_id);
        }
    }

    working.checkpoints = saved_checkpoints;
    working.image_assets = saved_assets;
    *doc = working;
    crate::cmd::helpers::prune_out_of_bounds_point_overrides(doc);
    Ok((report, refreshed_links))
}

fn compute_dirty_closure(
    doc: &ProjectDocument,
    order: &[SheetId],
    seeds: &[SheetId],
    external_policy: Option<&ExternalPathPolicy>,
) -> HashSet<SheetId> {
    let known: HashSet<&SheetId> = order.iter().collect();
    let mut dependents: BTreeMap<SheetId, Vec<SheetId>> = BTreeMap::new();
    for id in order {
        for dep in doc.upstream_dependencies(id) {
            dependents.entry(dep).or_default().push(id.clone());
        }
    }

    // Chained ResultSheets with unpinned InfoConstant hooks resolve their InfoSheet by walking
    // back to the origin DataSheet; keep that edge explicit even if intermediate sheets are frozen.
    for rs in &doc.result_sheets {
        if rs.analysis.kind.hooked_sources().iter().any(|h| matches!(h, HookSource::InfoConstant { info_sheet_id: None, .. }))
            && let Some(origin_info) = walk_origin_info_sheet_id(doc, &rs.analysis.source_sheet_id)
        {
            dependents.entry(origin_info.clone()).or_default().push(rs.id.clone());
        }
    }

    let mut dirty: HashSet<SheetId> = HashSet::new();
    let mut queue: VecDeque<SheetId> = VecDeque::new();
    let mut enqueue = |id: &SheetId| {
        if known.contains(id) && dirty.insert(id.clone()) {
            queue.push_back(id.clone());
        }
    };

    for seed in seeds {
        enqueue(seed);
    }
    for rs in &doc.result_sheets {
        if matches!(rs.status, ResultStatus::Dirty) {
            enqueue(&rs.id);
        }
    }
    if external_policy.is_some() {
        for ds in &doc.data_sheets {
            if !ds.frozen && ds.linked_regions.iter().any(|r| matches!(r.spec.source, LinkSource::ExternalDelimitedFile { .. })) {
                enqueue(&ds.id);
            }
        }
    }

    while let Some(curr) = queue.pop_front() {
        // Frozen DataSheets are skipped by evaluation so their stored values remain unchanged;
        // do not propagate stale invalidation past a frozen table (direct upstream edges still reach dependents).
        if doc.data_sheet(&curr).is_some_and(|ds| ds.frozen) {
            continue;
        }
        if let Some(children) = dependents.get(&curr) {
            for child in children {
                if dirty.insert(child.clone()) {
                    queue.push_back(child.clone());
                }
            }
        }
    }

    dirty
}

fn evaluate_data_sheet_node(
    working: &mut ProjectDocument,
    ds_idx: usize,
    report: &mut RecalcReport,
    external_policy: Option<&ExternalPathPolicy>,
    cancelled: Option<&AtomicBool>,
) -> Result<usize, EngineError> {
    check_recalc_cancelled(cancelled)?;
    let Some(ds) = working.data_sheets.get(ds_idx) else {
        return Ok(0);
    };
    let sheet_id = ds.id.clone();
    if ds.frozen {
        report.skipped_frozen_sheets.push(sheet_id);
        return Ok(0);
    }
    if ds.linked_regions.is_empty() && !data_sheet_has_calculated_variables(ds) {
        return Ok(0);
    }

    let before_active = crate::cmd::helpers::snapshot_active_series_lengths(working, &sheet_id);
    let had_links = !ds.linked_regions.is_empty();
    let info_sheet_id = ds.info_sheet_id.clone();
    let refreshed = refresh_data_sheet_links(working, ds_idx, external_policy, cancelled)?;
    check_recalc_cancelled(cancelled)?;

    // Resolve Info constants strictly from this sheet's bound `info_sheet_id` (never fall back to `info_sheets.first()`).
    let info_opt = info_sheet_id.as_ref().and_then(|id| working.info_sheet(id)).cloned();

    let Some(ds_mut) = working.data_sheets.get_mut(ds_idx) else {
        return Ok(refreshed);
    };
    let eval_cols = evaluate_data_sheet_calculated_variables(ds_mut, info_opt.as_ref())
        .map_err(|err| EngineError::new(format!("calculated variables on sheet `{sheet_id}` failed: {err}")))?;

    if had_links || !eval_cols.is_empty() {
        crate::cmd::helpers::prune_shortened_series_overrides(working, &sheet_id, &before_active);
        report.updated_data_sheets.push(sheet_id);
    }
    Ok(refreshed)
}

fn clear_cell_range(target: &mut DataSheet, range: SheetCellRange) {
    let row_count = target.archetype.row_count();
    let col_count = target.archetype.column_count();
    if row_count == 0 || col_count == 0 {
        return;
    }
    let min_row = range.start.row.min(range.end.row);
    if min_row >= row_count {
        return;
    }
    let max_row = range.start.row.max(range.end.row).min(row_count - 1);
    let ((min_col, start_sub), (raw_max_col, raw_end_sub)) = if range.start.col <= range.end.col {
        ((range.start.col, range.start.subcol), (range.end.col, range.end.subcol))
    } else {
        ((range.end.col, range.end.subcol), (range.start.col, range.start.subcol))
    };
    if min_col >= col_count {
        return;
    }
    let max_col = raw_max_col.min(col_count - 1);
    let end_sub_at_max = if raw_max_col > max_col { None } else { Some(raw_end_sub) };

    for r in min_row..=max_row {
        for c in min_col..=max_col {
            let s_start = if c == min_col { if min_col == raw_max_col { start_sub.min(raw_end_sub) } else { start_sub } } else { 0 };
            let s_end = if c == max_col { if min_col == raw_max_col { Some(start_sub.max(raw_end_sub)) } else { end_sub_at_max } } else { None };
            let mut s = s_start;
            while s_end.is_none_or(|end_s| s <= end_s) {
                if target.archetype.data_cell(CellCoord::new(r, c, s)).is_none() {
                    break;
                }
                if target.set_cell(CellCoord::new(r, c, s), Cell::empty()).is_err() {
                    break;
                }
                let Some(next_s) = s.checked_add(1) else {
                    break;
                };
                s = next_s;
            }
        }
    }
}

fn make_refresh_spec(spec: &ImportLinkSpec, existing_range_start: CellCoord) -> ImportLinkSpec {
    let mut refresh = spec.clone();
    refresh.placement.mode = PlacementMode::Overwrite;
    refresh.placement.anchor = existing_range_start;
    refresh
}

/// Propagate column or row titles from an internal source [`DataSheet`] into `target_ds` starting at `base_col`.
pub(crate) fn propagate_internal_source_titles(target_ds: &mut DataSheet, spec: &ImportLinkSpec, base_col: usize, src: &DataSheet) {
    if !spec.placement.propagate_title_header {
        return;
    }
    let LinkSource::InternalSheet { range, .. } = &spec.source else {
        return;
    };

    let titles: Vec<String> = if spec.placement.transpose {
        let Some(last_row) = src.archetype.row_count().checked_sub(1) else {
            return;
        };
        let min_r = range.start.row.min(range.end.row);
        let max_r = range.start.row.max(range.end.row);
        let r0 = spec.filter.row_window.map_or(min_r, |(lo, _)| lo.max(min_r));
        let r1 = spec.filter.row_window.map_or(max_r, |(_, hi)| hi.min(max_r)).min(last_row);
        if r0 > r1 {
            return;
        }
        (r0..=r1).filter_map(|r| source_row_title(&src.archetype, r)).collect()
    } else {
        let Some(last_col) = src.archetype.column_count().checked_sub(1) else {
            return;
        };
        let min_c = range.start.col.min(range.end.col);
        let max_c = range.start.col.max(range.end.col);
        let c0 = spec.filter.col_window.map_or(min_c, |(lo, _)| lo.max(min_c));
        let c1 = spec.filter.col_window.map_or(max_c, |(_, hi)| hi.min(max_c)).min(last_col);
        if c0 > c1 {
            return;
        }
        (c0..=c1).filter_map(|c| source_col_title(&src.archetype, c)).collect()
    };

    for (offset, title) in titles.into_iter().enumerate() {
        if !title.is_empty() {
            set_target_group_title(&mut target_ds.archetype, base_col.saturating_add(offset), &title);
        }
    }
}

fn source_col_title(arch: &TableArchetype, col: usize) -> Option<String> {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Column(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Survival(t) => t.cohorts.get(col).map(|d| d.title.clone()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col).map(|v| v.title.clone()),
        TableArchetype::Nested(t) => t.groups.get(col).map(|g| g.title.clone()),
    }
}

fn source_row_title(arch: &TableArchetype, row: usize) -> Option<String> {
    let titles = match arch {
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    };
    titles.get(row).cloned()
}

fn set_target_group_title(arch: &mut TableArchetype, col: usize, title: &str) {
    match arch {
        TableArchetype::Xy(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Column(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Grouped(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Contingency(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::PartsOfWhole(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Survival(t) => assign_ds_title(&mut t.cohorts, col, title),
        TableArchetype::MultipleVariables(t) => {
            if let Some(var) = t.variables.get_mut(col) {
                var.title = title.to_owned();
            }
        }
        TableArchetype::Nested(t) => {
            if let Some(grp) = t.groups.get_mut(col) {
                grp.title = title.to_owned();
            }
        }
    }
}

fn assign_ds_title(sets: &mut [DataSetColumn], col: usize, title: &str) {
    if let Some(ds) = sets.get_mut(col) {
        ds.title = title.to_owned();
    }
}

pub(crate) enum ResolvedInternalLinkSource<'a> {
    Data { sheet: &'a DataSheet, propagate_titles: bool },
    Result(&'a ResultSheet),
}

pub(crate) fn resolve_internal_link_source<'a>(
    doc: &'a ProjectDocument,
    sheet_id: &SheetId,
    kind: InternalLinkKind,
    tab_id: Option<&str>,
) -> Option<ResolvedInternalLinkSource<'a>> {
    match kind {
        InternalLinkKind::DataSheet => doc.data_sheet(sheet_id).map(|sheet| ResolvedInternalLinkSource::Data { sheet, propagate_titles: true }),
        InternalLinkKind::AnalyzableResultSheet => {
            if let Some(sheet) = doc.data_sheet(sheet_id) {
                return Some(ResolvedInternalLinkSource::Data { sheet, propagate_titles: true });
            }
            let src_rs = doc.result_sheet(sheet_id)?;
            if tab_id.is_none()
                && let Some(analyzable) = src_rs.analyzable_data.as_ref()
            {
                return Some(ResolvedInternalLinkSource::Data { sheet: analyzable, propagate_titles: false });
            }
            Some(ResolvedInternalLinkSource::Result(src_rs))
        }
        InternalLinkKind::TerminalResultSheet => {
            if let Some(sheet) = doc.data_sheet(sheet_id) {
                return Some(ResolvedInternalLinkSource::Data { sheet, propagate_titles: true });
            }
            doc.result_sheet(sheet_id).map(ResolvedInternalLinkSource::Result)
        }
    }
}

enum ResolvedRefreshSource<'a> {
    InternalData { sheet: &'a DataSheet, propagate_titles: bool },
    InternalResult(&'a ResultSheet),
    Delimited { text: String, delimiter: char, decimal_separator: char },
}

fn resolve_refresh_source<'a>(
    working: &'a ProjectDocument,
    target_id: &SheetId,
    region_id: &str,
    source: &LinkSource,
    external_policy: Option<&ExternalPathPolicy>,
) -> Result<Option<ResolvedRefreshSource<'a>>, EngineError> {
    match source {
        LinkSource::Clipboard => Ok(None),
        LinkSource::InternalSheet { sheet_id, kind, tab_id, .. } => {
            if working.data_sheet(sheet_id).is_none()
                && let Some(src_rs) = working.result_sheet(sheet_id)
                && let Err((_, detail)) = check_upstream_result_status(src_rs, sheet_id)
            {
                return Err(EngineError::new(format!("linked region `{region_id}` refresh on sheet `{target_id}` failed: {detail}")));
            }
            Ok(resolve_internal_link_source(working, sheet_id, *kind, tab_id.as_deref()).map(|resolved| match resolved {
                ResolvedInternalLinkSource::Data { sheet, propagate_titles } => ResolvedRefreshSource::InternalData { sheet, propagate_titles },
                ResolvedInternalLinkSource::Result(src_rs) => ResolvedRefreshSource::InternalResult(src_rs),
            }))
        }
        LinkSource::ExternalDelimitedFile { path, delimiter, decimal_separator } => {
            let Some(policy) = external_policy else {
                return Ok(None);
            };
            let text = policy.read_delimited_text_file(path, "linked file", MAX_CSV_FILE_BYTES)?;
            Ok(Some(ResolvedRefreshSource::Delimited { text, delimiter: *delimiter, decimal_separator: *decimal_separator }))
        }
    }
}

fn refresh_single_region(
    working: &ProjectDocument,
    target: &mut DataSheet,
    reg_idx: usize,
    external_policy: Option<&ExternalPathPolicy>,
) -> Result<bool, EngineError> {
    let Some(region) = target.linked_regions.get(reg_idx).cloned() else {
        return Ok(false);
    };
    let Some(resolved) = resolve_refresh_source(working, &target.id, &region.id, &region.spec.source, external_policy)? else {
        return Ok(false);
    };

    let refresh_spec = make_refresh_spec(&region.spec, region.target_range.start);
    let opts = ImportPipelineOptions {
        baseline: region.spec.filter.baseline,
        exclude_coords: Vec::new(),
        exclude_row_predicate: region.spec.filter.exclude_row_predicate.clone(),
        link_id: Some(region.id.clone()),
        provenance_summary: Some(region.provenance_summary.clone()),
    };

    clear_cell_range(target, region.target_range);

    let saved_regions = std::mem::take(&mut target.linked_regions);
    let pipe_res = match &resolved {
        ResolvedRefreshSource::InternalData { sheet, .. } => {
            apply_import_link_pipeline(target, &refresh_spec, ImportSourcePayload::InternalData(sheet), &opts, None)
        }
        ResolvedRefreshSource::InternalResult(src_rs) => {
            apply_import_link_pipeline(target, &refresh_spec, ImportSourcePayload::InternalResult(src_rs), &opts, None)
        }
        ResolvedRefreshSource::Delimited { text, delimiter, decimal_separator } => {
            let payload = ImportSourcePayload::DelimitedText { text, delimiter: *delimiter, decimal_separator: *decimal_separator };
            apply_import_link_pipeline(target, &refresh_spec, payload, &opts, None)
        }
    };
    target.linked_regions = saved_regions;

    let report = pipe_res.map_err(|err| EngineError::new(format!("linked region `{}` refresh on sheet `{}` failed: {err}", region.id, target.id)))?;
    target.prune_stale_exclusion_notes();
    if let ResolvedRefreshSource::InternalData { sheet, propagate_titles: true } = resolved {
        propagate_internal_source_titles(target, &refresh_spec, report.target_range.start.col, sheet);
    } else if let ResolvedRefreshSource::InternalResult(src_rs) = resolved
        && region.id.starts_with("result-tab-")
        && let LinkSource::InternalSheet { tab_id, .. } = &refresh_spec.source
        && let Some(tab) = tab_id.as_ref().and_then(|tid| src_rs.tabs.iter().find(|t| &t.id == tid)).or_else(|| src_rs.tabs.first())
    {
        crate::cmd::helpers::apply_result_tab_metadata_to_sheet(target, tab);
    }
    if let Some(slot) = target.linked_regions.get_mut(reg_idx) {
        slot.target_range = report.target_range;
        slot.provenance_summary = report.provenance_summary;
    }
    Ok(true)
}

fn refresh_data_sheet_links(
    working: &mut ProjectDocument,
    ds_idx: usize,
    external_policy: Option<&ExternalPathPolicy>,
    cancelled: Option<&AtomicBool>,
) -> Result<usize, EngineError> {
    let Some(ds) = working.data_sheets.get(ds_idx) else {
        return Ok(0);
    };
    let region_count = ds.linked_regions.len();
    if region_count == 0 {
        return Ok(0);
    }

    // Clone target temporarily so `working` can be read immutably to resolve upstream payloads.
    let mut target = ds.clone();
    let mut refreshed = 0usize;
    for reg_idx in 0..region_count {
        check_recalc_cancelled(cancelled)?;
        if refresh_single_region(working, &mut target, reg_idx, external_policy)? {
            refreshed = refreshed.saturating_add(1);
        }
    }
    if let Some(slot) = working.data_sheets.get_mut(ds_idx) {
        *slot = target;
    }
    Ok(refreshed)
}

pub(crate) fn clear_failed_result_sheet(target: &mut ResultSheet, code: ErrorCode, message: String) {
    target.analyzable_data = None;
    target.tabs.clear();
    target.scalars.clear();
    target.pairwise_comparisons.clear();
    target.fitted_curves.clear();
    target.eliminated_outliers.clear();
    target.checklist.clear();
    // Preserve user-authored annotations (`floating_notes`, `color_tag`) across transient recomputation failures.
    target.status = ResultStatus::Failed { code, message };
}

fn check_upstream_result_status(res: &ResultSheet, context_id: &SheetId) -> Result<(), (ErrorCode, String)> {
    match &res.status {
        ResultStatus::UpToDate => Ok(()),
        ResultStatus::Failed { message, .. } => Err((ErrorCode::BadInput, format!("upstream result sheet `{context_id}` failed: {message}"))),
        ResultStatus::Dirty => Err((ErrorCode::BadInput, format!("upstream result sheet `{context_id}` is not up to date"))),
    }
}

fn resolve_source_table(doc: &ProjectDocument, source_id: &SheetId) -> Result<DataSheet, (ErrorCode, String)> {
    if let Some(ds) = doc.data_sheet(source_id) {
        return Ok(ds.clone());
    }
    let Some(res) = doc.result_sheet(source_id) else {
        return Err((ErrorCode::NotFound, format!("source sheet `{source_id}` not found")));
    };
    check_upstream_result_status(res, source_id)?;
    res.analyzable_data.clone().ok_or_else(|| (ErrorCode::NotFound, format!("upstream result sheet `{source_id}` has no analyzable table")))
}

fn walk_origin_info_sheet_id<'a>(doc: &'a ProjectDocument, start_id: &SheetId) -> Option<&'a SheetId> {
    let mut cursor = start_id;
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
    None
}

fn resolve_origin_info_sheet_id<'a>(doc: &'a ProjectDocument, source_ds: &'a DataSheet, analysis: &AnalysisSpec) -> Option<&'a SheetId> {
    if let Some(ref id) = source_ds.info_sheet_id
        && doc.info_sheet(id).is_some()
    {
        return Some(id);
    }

    // Walk back through chained ResultSheets if `source_ds` came from `analyzable_data`.
    walk_origin_info_sheet_id(doc, &analysis.source_sheet_id)
}

fn validate_info_constant_hooks(doc: &ProjectDocument, analysis: &AnalysisSpec, primary_info_id: Option<&SheetId>) -> Result<(), String> {
    for hook in analysis.kind.hooked_sources() {
        if let HookSource::InfoConstant { info_sheet_id, constant_name } = hook {
            let target_id = info_sheet_id.as_ref().or(primary_info_id);
            match target_id {
                None => {
                    return Err(format!(
                        "info constant hook `{constant_name}` requires a bound Info sheet on source sheet `{}`",
                        analysis.source_sheet_id
                    ));
                }
                Some(iid) if doc.info_sheet(iid).is_none() => {
                    return Err(format!("referenced Info sheet `{iid}` for constant `{constant_name}` was not found"));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn recompute_result_sheet(doc: &mut ProjectDocument, res_idx: usize, cancelled: Option<&AtomicBool>) -> Result<bool, EngineError> {
    check_recalc_cancelled(cancelled)?;
    let Some(existing) = doc.result_sheets.get(res_idx).cloned() else {
        return Ok(false);
    };
    let source_ds = match resolve_source_table(doc, &existing.analysis.source_sheet_id) {
        Ok(ds) => ds,
        Err((code, message)) => {
            if let Some(target) = doc.result_sheets.get_mut(res_idx) {
                clear_failed_result_sheet(target, code, message);
            }
            return Ok(false);
        }
    };
    for hook in existing.analysis.kind.hooked_sources() {
        if let HookSource::ResultScalar { result_sheet_id, .. } = hook
            && let Some(res) = doc.result_sheet(&result_sheet_id)
            && let Err((code, message)) = check_upstream_result_status(res, &result_sheet_id)
        {
            if let Some(target) = doc.result_sheets.get_mut(res_idx) {
                clear_failed_result_sheet(target, code, message);
            }
            return Ok(false);
        }
    }

    check_recalc_cancelled(cancelled)?;
    let desc = descriptor_for_kind(&existing.analysis.kind);
    let output: Result<ResultSheet, String> = match desc.recompute_strategy {
        AnalysisRecomputeStrategy::Transform => {
            let primary_info_id = resolve_origin_info_sheet_id(doc, &source_ds, &existing.analysis);
            if let Err(message) = validate_info_constant_hooks(doc, &existing.analysis, primary_info_id) {
                Err(message)
            } else {
                let info_opt = primary_info_id.and_then(|id| doc.info_sheet(id));
                let has_explicit_info_hook =
                    existing.analysis.kind.hooked_sources().iter().any(|h| matches!(h, HookSource::InfoConstant { info_sheet_id: Some(_), .. }));
                let ctx = TransformContext {
                    info_sheet: info_opt,
                    info_sheets: if primary_info_id.is_some() || has_explicit_info_hook { &doc.info_sheets } else { &[] },
                    result_sheets: &doc.result_sheets,
                    data_sheets: &doc.data_sheets,
                    selected_data_sets: existing.analysis.selected_data_sets.clone(),
                    output_id: Some(existing.id.as_str().to_owned()),
                    output_title: Some(existing.title().to_owned()),
                };
                execute_analysis_transform(&source_ds, &existing.analysis, &ctx).map_err(|err| err.to_string())
            }
        }
        AnalysisRecomputeStrategy::Fit => {
            let primary_info_id = resolve_origin_info_sheet_id(doc, &source_ds, &existing.analysis);
            if let Err(message) = validate_info_constant_hooks(doc, &existing.analysis, primary_info_id) {
                Err(message)
            } else {
                let has_explicit_info_hook =
                    existing.analysis.kind.hooked_sources().iter().any(|h| matches!(h, HookSource::InfoConstant { info_sheet_id: Some(_), .. }));
                let hooks = HookContext {
                    info_sheets: if primary_info_id.is_some() || has_explicit_info_hook { &doc.info_sheets } else { &[] },
                    result_sheets: &doc.result_sheets,
                    data_sheets: &doc.data_sheets,
                    primary_info_sheet_id: primary_info_id,
                    visited_sheets: HashSet::new(),
                };
                curvebook_fit::execute_analysis(&existing.analysis, &source_ds, &hooks).map_err(|err| err.to_string())
            }
        }
        AnalysisRecomputeStrategy::PowerWithInfoHooks => {
            if let AnalysisKind::PowerAnalysis(p_spec) = &existing.analysis.kind {
                resolve_power_analysis_with_info_hooks(doc, &source_ds, &existing, p_spec)
            } else {
                Err(format!("descriptor strategy mismatch: `{}` expected PowerAnalysis", desc.slug))
            }
        }
        AnalysisRecomputeStrategy::Stats => {
            if !existing.analysis.kind.is_compatible_with(source_ds.kind()) {
                Err(format!(
                    "analysis `{}` is incompatible with {} table `{}`",
                    existing.analysis.effective_title(),
                    source_ds.kind().label(),
                    source_ds.id
                ))
            } else {
                curvebook_stats::run_analysis(&source_ds, &existing.analysis).map_err(|err| err.to_string())
            }
        }
    };

    check_recalc_cancelled(cancelled)?;
    let Some(target) = doc.result_sheets.get_mut(res_idx) else {
        return Ok(false);
    };
    match output {
        Ok(mut computed) => {
            // Preserve `ResultStatus::Failed` emitted by solvers (e.g. unconverged/diverged
            // nonlinear fits) rather than overwriting to `UpToDate` (`AUD-13-04`).
            if let ResultStatus::Failed { code, message } = computed.status {
                computed.id = existing.id.clone();
                computed.analysis = existing.analysis;
                computed.floating_notes = existing.floating_notes;
                computed.color_tag = existing.color_tag;
                computed.scalars.clear();
                computed.pairwise_comparisons.clear();
                computed.fitted_curves.clear();
                computed.analyzable_data = None;
                computed.eliminated_outliers.clear();
                computed.status = ResultStatus::Failed { code, message: message.clone() };
                if validate_result_sheet_finite(&computed, &existing.id).is_ok() {
                    *target = computed;
                } else {
                    clear_failed_result_sheet(target, code, message);
                }
                return Ok(false);
            }
            if let Err(reason) = validate_result_sheet_finite(&computed, &existing.id) {
                clear_failed_result_sheet(target, ErrorCode::SolverFailed, reason);
                return Ok(false);
            }
            computed.id = existing.id;
            computed.analysis = existing.analysis;
            computed.floating_notes = existing.floating_notes;
            computed.color_tag = existing.color_tag;
            if let AnalysisKind::Survival(curvebook_doc::SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. }) = &computed.analysis.kind {
                let (tie_label, tie_desc) = match tie_method {
                    curvebook_doc::CoxTieMethod::Efron => ("Efron", "Efron approximation"),
                    curvebook_doc::CoxTieMethod::Breslow => ("Breslow", "Breslow approximation"),
                    curvebook_doc::CoxTieMethod::Exact => ("Exact", "Exact discrete conditional partial likelihood"),
                };
                if let Some(tab) = computed.tabs.iter_mut().find(|t| t.id == "cox-ph") {
                    tab.footnotes.push(format!("Tie handling method: {tie_label}."));
                }
                computed.checklist.push(curvebook_doc::ChecklistItem {
                    assumption: String::from("Cox tie handling"),
                    status: curvebook_doc::ChecklistStatus::Info,
                    diagnostic_metric: Some(format!("tie_method = {tie_label}")),
                    explanation: format!("Partial likelihood ties handled via {tie_desc}."),
                    remediation: None,
                });
            }
            computed.status = ResultStatus::UpToDate;
            *target = computed;
            Ok(true)
        }
        Err(message) => {
            clear_failed_result_sheet(target, ErrorCode::SolverFailed, message);
            Ok(false)
        }
    }
}

fn lookup_power_info_constant_f64<'a>(info: &curvebook_doc::InfoSheet, names: &[&'a str]) -> Result<Option<(&'a str, f64)>, String> {
    for &name in names {
        let Some(entry) = info.get_constant(name) else {
            continue;
        };
        match &entry.value {
            curvebook_doc::ConstantValue::Empty => return Ok(None),
            curvebook_doc::ConstantValue::Number(v) => {
                if v.is_finite() {
                    return Ok(Some((name, *v)));
                }
                return Err(format!("hooked Info constant `{name}` must be a finite number (got {v})"));
            }
            curvebook_doc::ConstantValue::Text(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    return Ok(None);
                }
                let parsed = trimmed
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("hooked Info constant `{name}` (`{s}`) is not a finite number"))?;
                return Ok(Some((name, parsed)));
            }
            curvebook_doc::ConstantValue::Date(s) => {
                return Err(format!("hooked Info constant `{name}` has date value `{s}`, expected a finite number"));
            }
        }
    }
    Ok(None)
}

fn resolve_power_analysis_with_info_hooks(
    doc: &ProjectDocument,
    source_ds: &DataSheet,
    existing: &ResultSheet,
    base: &curvebook_doc::PowerAnalysisSpec,
) -> Result<ResultSheet, String> {
    let mut patched = base.clone();
    let info_opt = match base.info_sheet_id.as_ref() {
        Some(id) => Some(doc.info_sheet(id).ok_or_else(|| format!("referenced Info sheet `{id}` not found"))?),
        None => None,
    };
    if let Some(info) = info_opt {
        if let Some((name, alpha)) = lookup_power_info_constant_f64(info, &["Alpha", "alpha"])? {
            if !(alpha > 0.0 && alpha < 1.0) {
                return Err(format!("hooked Info constant `{name}` ({alpha}) must lie strictly in (0, 1)"));
            }
            patched.alpha = alpha;
        }
        if patched.solve_target != curvebook_doc::PowerSolveTarget::Power
            && let Some((name, target_power)) = lookup_power_info_constant_f64(info, &["Target Power", "target_power", "Target power"])?
        {
            if !(target_power > 0.0 && target_power < 1.0) {
                return Err(format!("hooked Info constant `{name}` ({target_power}) must lie strictly in (0, 1)"));
            }
            patched.target_power = Some(target_power);
        }
        if patched.solve_target != curvebook_doc::PowerSolveTarget::DetectableEffect
            && let Some((_, eff)) = lookup_power_info_constant_f64(info, &["Effect Size", "effect_size", "Effect size"])?
        {
            let matches_rich_baseline = match &base.effect {
                curvebook_doc::PowerEffectSpec::Standardized(_) | curvebook_doc::PowerEffectSpec::Unspecified => false,
                _ => curvebook_stats::power::solve_power(&curvebook_stats::power::power_request_from_doc_spec(base))
                    .ok()
                    .is_some_and(|r| (eff - r.effect_size).abs() <= 1e-12 * r.effect_size.abs().max(1.0)),
            };
            if !matches_rich_baseline {
                patched.effect = curvebook_doc::PowerEffectSpec::Standardized(eff);
            }
        }
        if patched.solve_target != curvebook_doc::PowerSolveTarget::SampleSize {
            if let Some((name, n1)) = lookup_power_info_constant_f64(info, &["N per group", "sample_size", "n1"])? {
                if n1 <= 0.0 {
                    return Err(format!("hooked Info constant `{name}` ({n1}) must be strictly positive"));
                }
                if base.sample_size.is_none_or(|prev| (n1 - prev).abs() > 1e-12 * prev.abs().max(1.0)) {
                    patched.sample_size = Some(n1);
                }
            }
            if let Some((name, n2)) = lookup_power_info_constant_f64(info, &["N group 2", "sample_size_group2", "n2"])? {
                if n2 <= 0.0 {
                    return Err(format!("hooked Info constant `{name}` ({n2}) must be strictly positive"));
                }
                if base.sample_size_group2.is_none_or(|prev| (n2 - prev).abs() > 1e-12 * prev.abs().max(1.0)) {
                    patched.sample_size_group2 = Some(n2);
                }
            }
        }
    }
    let mut patched_spec = existing.analysis.clone();
    patched_spec.kind = AnalysisKind::PowerAnalysis(patched);
    curvebook_stats::run_analysis(source_ds, &patched_spec).map_err(|err| err.to_string())
}

fn validate_result_sheet_finite(res: &ResultSheet, sheet_id: &SheetId) -> Result<(), String> {
    for (idx, scalar) in res.scalars.iter().enumerate() {
        if !scalar.value.is_finite() {
            return Err(format!("result sheet `{sheet_id}` produced non-finite value in scalar `{}` (index {idx}): {}", scalar.key, scalar.value));
        }
    }
    for (tab_idx, tab) in res.tabs.iter().enumerate() {
        for (r, row) in tab.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if !result_cell_is_finite(cell) {
                    return Err(format!("result sheet `{sheet_id}` produced non-finite value at tab[{tab_idx}].rows[{r}][{c}]"));
                }
            }
        }
    }
    for (idx, cmp) in res.pairwise_comparisons.iter().enumerate() {
        if cmp.estimate_diff.is_some_and(|v| !v.is_finite()) || !cmp.raw_p_or_q.is_finite() {
            return Err(format!("result sheet `{sheet_id}` produced non-finite value at pairwise_comparisons[{idx}]"));
        }
    }
    for (c_idx, curve) in res.fitted_curves.iter().enumerate() {
        for (p_idx, pt) in curve.points.iter().enumerate() {
            if [Some(pt.x), Some(pt.y), pt.ci_lower, pt.ci_upper, pt.pred_lower, pt.pred_upper].into_iter().flatten().any(|v| !v.is_finite()) {
                return Err(format!("result sheet `{sheet_id}` produced non-finite value at fitted_curves[{c_idx}].points[{p_idx}]"));
            }
        }
    }
    if let Some(ref data) = res.analyzable_data
        && let Some(loc) = first_non_finite_data_location(data)
    {
        return Err(format!("result sheet `{sheet_id}` produced non-finite value in analyzable_data.{loc}"));
    }
    Ok(())
}

fn result_cell_is_finite(cell: &ResultCell) -> bool {
    match cell {
        ResultCell::Number { value, .. } => value.is_finite(),
        ResultCell::PValue { raw, .. } => raw.is_finite(),
        ResultCell::ConfidenceInterval { lower, upper } => ci_bound_is_finite(*lower) && ci_bound_is_finite(*upper),
        ResultCell::Empty | ResultCell::Text(_) | ResultCell::Boolean(_) => true,
    }
}

const fn ci_bound_is_finite(bound: CiBound) -> bool {
    match bound {
        CiBound::Finite(v) => v.is_finite(),
        CiBound::Open => true,
    }
}

fn first_non_finite_data_location(data: &DataSheet) -> Option<String> {
    match &data.archetype {
        TableArchetype::Xy(t) => {
            if let XColumnMode::GeneratedSequence { start, step, .. } = t.x_mode
                && (!start.is_finite() || !step.is_finite())
            {
                return Some(String::from("x_mode"));
            }
            first_non_finite_rows(&t.x_cells, "x_cells").or_else(|| first_non_finite_columns(&t.data_sets, "data_sets"))
        }
        TableArchetype::Column(t) => first_non_finite_columns(&t.data_sets, "data_sets"),
        TableArchetype::Grouped(t) => first_non_finite_columns(&t.data_sets, "data_sets"),
        TableArchetype::Contingency(t) => first_non_finite_columns(&t.data_sets, "data_sets"),
        TableArchetype::PartsOfWhole(t) => first_non_finite_columns(&t.data_sets, "data_sets"),
        TableArchetype::Survival(t) => first_non_finite_1d(&t.times, "times").or_else(|| first_non_finite_columns(&t.cohorts, "cohorts")),
        TableArchetype::MultipleVariables(t) => {
            for (v_idx, var) in t.variables.iter().enumerate() {
                if let Some(loc) = first_non_finite_1d(&var.cells, &format!("variables[{v_idx}]")) {
                    return Some(loc);
                }
            }
            None
        }
        TableArchetype::Nested(t) => {
            for (g_idx, grp) in t.groups.iter().enumerate() {
                if let Some(loc) = first_non_finite_rows(&grp.cells, &format!("groups[{g_idx}]")) {
                    return Some(loc);
                }
            }
            None
        }
    }
}

fn first_non_finite_columns(cols: &[DataSetColumn], prefix: &str) -> Option<String> {
    for (c_idx, col) in cols.iter().enumerate() {
        if let Some(loc) = first_non_finite_rows(&col.cells, &format!("{prefix}[{c_idx}]")) {
            return Some(loc);
        }
    }
    None
}

fn first_non_finite_rows(rows: &[Vec<Cell>], prefix: &str) -> Option<String> {
    for (r_idx, row) in rows.iter().enumerate() {
        for (s_idx, cell) in row.iter().enumerate() {
            if matches!(cell.value, CellValue::Number(v) if !v.is_finite()) {
                return Some(format!("{prefix}[{r_idx}][{s_idx}]"));
            }
        }
    }
    None
}

fn first_non_finite_1d(cells: &[Cell], prefix: &str) -> Option<String> {
    for (r_idx, cell) in cells.iter().enumerate() {
        if matches!(cell.value, CellValue::Number(v) if !v.is_finite()) {
            return Some(format!("{prefix}[{r_idx}]"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
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

    #[test]
    fn validate_result_sheet_finite_rejects_non_finite_outputs() {
        use curvebook_doc::{
            CiBound, CurveSamplePoint, FittedCurveOverlay, LinearRegressionSpec, PairwiseComparisonRecord, ResultCell, ResultScalarEntry, ResultTab,
            ResultTabKind, SequenceProgression, XColumnMode, XyTable,
        };

        let make_base = || -> ResultSheet {
            let spec = AnalysisSpec::new("res-nf", "data-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).expect("spec");
            ResultSheet::new(spec).expect("res")
        };
        let id = SheetId::new("res-nf");

        assert!(validate_result_sheet_finite(&make_base(), &id).is_ok());

        let mut r_scalar = make_base();
        r_scalar.scalars.push(ResultScalarEntry { key: String::from("Slope"), data_set_index: None, value: f64::NAN, units: None });
        assert!(validate_result_sheet_finite(&r_scalar, &id).is_err());

        let mut r_tab = make_base();
        r_tab.tabs.push(ResultTab {
            id: String::from("summary"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("CI")],
            row_labels: vec![String::from("Top")],
            rows: vec![vec![ResultCell::ConfidenceInterval { lower: CiBound::Finite(f64::NEG_INFINITY), upper: CiBound::Open }]],
            footnotes: Vec::new(),
        });
        assert!(validate_result_sheet_finite(&r_tab, &id).is_err());

        let mut r_cmp = make_base();
        r_cmp.pairwise_comparisons.push(PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("A"),
            label_b: String::from("B"),
            estimate_diff: Some(f64::INFINITY),
            raw_p_or_q: 0.05,
            is_fdr_q: false,
            significant: true,
        });
        assert!(validate_result_sheet_finite(&r_cmp, &id).is_err());

        let mut r_curve = make_base();
        r_curve.fitted_curves.push(FittedCurveOverlay {
            data_set_index: 0,
            equation_id: String::from("ba"),
            converged: true,
            ambiguous: false,
            points: vec![CurveSamplePoint { x: 1.0, y: f64::INFINITY, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None }],
        });
        assert!(validate_result_sheet_finite(&r_curve, &id).is_err());

        let mut r_data = make_base();
        let mut xy = XyTable::new(curvebook_doc::SubcolumnMode::RawReplicates(1), 1, 1).expect("xy");
        xy.x_mode = XColumnMode::GeneratedSequence { start: 0.0, step: f64::NAN, progression: SequenceProgression::Arithmetic };
        r_data.analyzable_data = Some(DataSheet::new("res-nf-out", "Out", TableArchetype::Xy(xy)).expect("ds"));
        assert!(validate_result_sheet_finite(&r_data, &id).is_err());

        let note =
            curvebook_doc::FloatingNote::new("note-keep", curvebook_doc::RectPt::new(10.0, 10.0, 120.0, 40.0), "Keep across transient failure");
        r_curve.floating_notes.push(note.clone());
        clear_failed_result_sheet(&mut r_curve, ErrorCode::SolverFailed, String::from("non-finite"));
        assert!(matches!(r_curve.status, ResultStatus::Failed { code: ErrorCode::SolverFailed, .. }));
        assert!(r_curve.fitted_curves.is_empty());
        assert!(r_curve.scalars.is_empty());
        assert!(r_curve.tabs.is_empty());
        assert!(r_curve.analyzable_data.is_none());
        assert_eq!(r_curve.floating_notes, vec![note]);
    }

    #[test]
    fn recompute_result_sheet_routes_all_descriptor_strategies() {
        use curvebook_doc::{
            DescriptiveSpec, LinearRegressionSpec, PowerAnalysisSpec, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, SubcolumnMode, XyTable,
        };

        let mut doc = ProjectDocument::new("descriptor-routing-test").expect("doc");

        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4).expect("xy");
        for (r, (x, y)) in [(1.0, 2.0), (2.0, 4.0), (3.0, 6.0), (4.0, 8.0)].into_iter().enumerate() {
            xy.x_cells[r][0] = Cell::number(x);
            xy.data_sets[0].cells[r][0] = Cell::number(y);
        }
        doc.add_data_sheet(DataSheet::new("xy-1", "XY", TableArchetype::Xy(xy)).expect("xy ds")).expect("add xy");

        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 4).expect("col");
        for (r, y) in [10.0, 12.0, 14.0, 16.0].into_iter().enumerate() {
            col.data_sets[0].cells[r][0] = Cell::number(y);
        }
        doc.add_data_sheet(DataSheet::new("col-1", "Column", TableArchetype::Column(col)).expect("col ds")).expect("add col");

        // Transform strategy
        let tr_spec =
            AnalysisSpec::new("res-tr", "col-1", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true }))
                .expect("tr spec");
        doc.add_result_sheet(ResultSheet::new(tr_spec).expect("tr res")).expect("add tr");

        // Fit strategy
        let fit_spec = AnalysisSpec::new("res-fit", "xy-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).expect("fit spec");
        doc.add_result_sheet(ResultSheet::new(fit_spec).expect("fit res")).expect("add fit");

        // Stats strategy
        let stats_spec = AnalysisSpec::new("res-stats", "col-1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("stats spec");
        doc.add_result_sheet(ResultSheet::new(stats_spec).expect("stats res")).expect("add stats");

        // PowerWithInfoHooks strategy
        let pwr = PowerAnalysisSpec {
            family: PowerTestFamily::OneSampleT,
            solve_target: PowerSolveTarget::SampleSize,
            effect: PowerEffectSpec::Standardized(0.8),
            ..PowerAnalysisSpec::default()
        };
        let pwr_spec = AnalysisSpec::new("res-pwr", "col-1", AnalysisKind::PowerAnalysis(pwr)).expect("pwr spec");
        doc.add_result_sheet(ResultSheet::new(pwr_spec).expect("pwr res")).expect("add pwr");

        let report = recalculate_project_with_report(&mut doc).expect("recalc succeeds");
        assert!(report.failed_result_sheets.is_empty());
        assert_eq!(report.updated_result_sheets.len(), 4);
        for id in ["res-tr", "res-fit", "res-stats", "res-pwr"] {
            let rs = doc.result_sheet(&SheetId::new(id)).expect("result sheet present");
            assert_eq!(rs.status, ResultStatus::UpToDate, "{id} should be UpToDate");
        }
    }
}
