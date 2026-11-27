//! Consolidated sheet-ID remapping across all typed [`SheetId`] references and inline template
//! hooks (`{{info:...}}`, `{{result:...}}`, `{{scalar:...}}`) in a [`ProjectDocument`].
//!
//! Used both by external archive import sanitization and by project-merge collision renumbering
//! so every `SheetId`-bearing field in the document model is handled in one canonical place.

use std::collections::BTreeMap;

use crate::analysis::{AnalysisKind, NormalizeAnchor, ParameterScope, TransformAnalysisSpec};
use crate::graph::{GraphAnnotation, GraphSheet, GraphSpec};
use crate::ids::{FloatingNote, HookSource, SheetId, extract_template_hooks};
use crate::layout::{LayoutItem, LayoutSheet, LayoutSpec};
use crate::links::{DataSheet, LinkSource};
use crate::project::ProjectDocument;

/// Remap every top-level sheet `.id` AND every typed or text-embedded [`SheetId`] reference
/// across `doc` according to `remap`.
///
/// Sheet IDs not present in `remap` are left unchanged.
pub fn remap_document_sheet_ids(doc: &mut ProjectDocument, remap: &BTreeMap<SheetId, SheetId>) {
    if remap.is_empty() {
        return;
    }
    for ds in &mut doc.data_sheets {
        apply_sheet_remap(&mut ds.id, remap);
    }
    for info in &mut doc.info_sheets {
        apply_sheet_remap(&mut info.id, remap);
    }
    for rs in &mut doc.result_sheets {
        apply_sheet_remap(&mut rs.id, remap);
    }
    for gs in &mut doc.graph_sheets {
        apply_sheet_remap(&mut gs.id, remap);
    }
    for ls in &mut doc.layout_sheets {
        apply_sheet_remap(&mut ls.id, remap);
    }
    remap_document_sheet_references(doc, remap);
}

/// Remap every typed and text-embedded [`SheetId`] reference across `doc` according to `remap`,
/// leaving top-level sheet `.id` fields as-is (for callers that already assigned sanitized IDs
/// while constructing `remap`).
///
/// Note: nested `rs.analysis.id` and `rs.analyzable_data.id` ARE updated to stay synchronized
/// with their owning [`crate::ResultSheet`].
pub fn remap_document_sheet_references(doc: &mut ProjectDocument, remap: &BTreeMap<SheetId, SheetId>) {
    if remap.is_empty() {
        return;
    }
    apply_opt_sheet_remap(&mut doc.active_sheet_id, remap);
    apply_opt_sheet_remap(&mut doc.family_anchor_sheet_id, remap);

    for ds in &mut doc.data_sheets {
        remap_data_sheet_references(ds, remap);
    }

    for info in &mut doc.info_sheets {
        for linked in &mut info.linked_data_sheets {
            apply_sheet_remap(linked, remap);
        }
        remap_inline_hook_tokens(&mut info.notes_markdown, remap);
        remap_floating_notes(&mut info.floating_notes, remap);
    }

    for rs in &mut doc.result_sheets {
        apply_sheet_remap(&mut rs.analysis.id, remap);
        apply_opt_sheet_remap(&mut rs.analysis.source_sheet_id, remap);
        remap_analysis_kind_references(&mut rs.analysis.kind, remap);
        if let Some(ref mut derived) = rs.analyzable_data {
            apply_sheet_remap(&mut derived.id, remap);
            remap_data_sheet_references(derived, remap);
        }
        for cell_id in &mut rs.eliminated_outliers {
            apply_sheet_remap(&mut cell_id.sheet_id, remap);
        }
        remap_floating_notes(&mut rs.floating_notes, remap);
    }

    for gs in &mut doc.graph_sheets {
        remap_graph_sheet_references(gs, remap);
    }

    for ls in &mut doc.layout_sheets {
        remap_layout_sheet_references(ls, remap);
    }
}

/// Remap all internal [`SheetId`] references and inline template hook tokens inside a [`GraphSpec`].
pub fn remap_graph_spec_references(spec: &mut GraphSpec, remap: &BTreeMap<SheetId, SheetId>) {
    if remap.is_empty() {
        return;
    }
    apply_sheet_remap(&mut spec.primary_source_sheet_id, remap);
    remap_inline_hook_tokens(&mut spec.title, remap);
    remap_inline_hook_tokens(&mut spec.x_axis.title, remap);
    remap_inline_hook_tokens(&mut spec.y_axis.title, remap);
    if let Some(ref mut y2) = spec.y2_axis {
        remap_inline_hook_tokens(&mut y2.title, remap);
    }
    for series in &mut spec.series {
        apply_sheet_remap(&mut series.source_sheet_id, remap);
        if let Some(ref mut ov) = series.regression_overlay {
            apply_sheet_remap(&mut ov.result_sheet_id, remap);
        }
        if let Some(ref mut label) = series.legend_label {
            remap_inline_hook_tokens(label, remap);
        }
    }
    for cmp in &mut spec.comparisons {
        apply_sheet_remap(&mut cmp.result_sheet_id, remap);
    }
    if let Some(ref mut cld) = spec.compact_letter_display {
        apply_sheet_remap(&mut cld.result_sheet_id, remap);
    }
    for ann in &mut spec.annotations {
        match ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                apply_sheet_remap(result_sheet_id, remap);
            }
            GraphAnnotation::TextBox { markdown, .. } => {
                remap_inline_hook_tokens(markdown, remap);
            }
            GraphAnnotation::LineOrArrow { .. } | GraphAnnotation::Shape { .. } | GraphAnnotation::EmbeddedImage { .. } => {}
        }
    }
}

/// Remap all internal [`SheetId`] references and inline template hook tokens inside a [`LayoutSpec`].
pub fn remap_layout_spec_references(spec: &mut LayoutSpec, remap: &BTreeMap<SheetId, SheetId>) {
    if remap.is_empty() {
        return;
    }
    for item in &mut spec.items {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => apply_sheet_remap(graph_sheet_id, remap),
            LayoutItem::LiveTable { source_sheet_id, .. } => apply_sheet_remap(source_sheet_id, remap),
            LayoutItem::TextCallout { markdown, .. } => remap_inline_hook_tokens(markdown, remap),
            LayoutItem::EmbeddedImage { .. } => {}
        }
    }
}

/// Rewrite inline `{{info:<sheet>:<key>}}`, `{{result:<sheet>:[<ds>:]<key>}}`, and
/// `{{scalar:<sheet>:<rest>}}` template hooks inside `text` according to `remap`.
///
/// Supports legacy sheet IDs containing `:` via longest-prefix matching against `remap`.
pub fn remap_inline_hook_tokens(text: &mut String, remap: &BTreeMap<SheetId, SheetId>) {
    if remap.is_empty() || !text.contains("{{") {
        return;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    let mut changed = false;

    while let Some((before, after_open)) = rest.split_once("{{") {
        out.push_str(before);
        let Some((raw_inner, after_close)) = after_open.split_once("}}") else {
            out.push_str("{{");
            out.push_str(after_open);
            rest = "";
            break;
        };
        if let Some(rewritten) = rewrite_hook_token(raw_inner, remap) {
            out.push_str(&rewritten);
            changed = true;
        } else {
            out.push_str("{{");
            out.push_str(raw_inner);
            out.push_str("}}");
        }
        rest = after_close;
    }
    out.push_str(rest);
    if changed {
        *text = out;
    }
}

fn apply_sheet_remap(id: &mut SheetId, remap: &BTreeMap<SheetId, SheetId>) {
    if let Some(mapped) = remap.get(id) {
        *id = mapped.clone();
    }
}

fn apply_opt_sheet_remap(id: &mut Option<SheetId>, remap: &BTreeMap<SheetId, SheetId>) {
    if let Some(inner) = id {
        apply_sheet_remap(inner, remap);
    }
}

fn remap_hook_source(hook: &mut HookSource, remap: &BTreeMap<SheetId, SheetId>) {
    match hook {
        HookSource::InfoConstant { info_sheet_id, .. } => apply_opt_sheet_remap(info_sheet_id, remap),
        HookSource::ResultScalar { result_sheet_id, .. } => apply_sheet_remap(result_sheet_id, remap),
        HookSource::ColumnTitleValue { data_sheet_id, .. } => apply_sheet_remap(data_sheet_id, remap),
    }
}

fn remap_analysis_kind_references(kind: &mut AnalysisKind, remap: &BTreeMap<SheetId, SheetId>) {
    match kind {
        AnalysisKind::PowerAnalysis(power) => {
            apply_opt_sheet_remap(&mut power.info_sheet_id, remap);
        }
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. }) => {
            if let NormalizeAnchor::Hooked(hook) = zero_anchor {
                remap_hook_source(hook, remap);
            }
            if let NormalizeAnchor::Hooked(hook) = hundred_anchor {
                remap_hook_source(hook, remap);
            }
        }
        AnalysisKind::NonlinearRegression(fit) => {
            for rule in &mut fit.parameter_rules {
                if let ParameterScope::HookedConstant(hook) = &mut rule.scope {
                    remap_hook_source(hook, remap);
                }
            }
            if let Some(dist) = &mut fit.plot_distribution {
                for hook in dist.parameter_hooks.iter_mut().flatten() {
                    remap_hook_source(hook, remap);
                }
            }
        }
        AnalysisKind::MonteCarloSimulation(mc) => {
            apply_opt_sheet_remap(&mut mc.downstream_result_sheet_id, remap);
            for scalar in &mut mc.tracked_scalars {
                apply_opt_sheet_remap(&mut scalar.result_sheet_id, remap);
            }
        }
        _ => {}
    }
}

fn remap_data_sheet_references(ds: &mut DataSheet, remap: &BTreeMap<SheetId, SheetId>) {
    apply_opt_sheet_remap(&mut ds.info_sheet_id, remap);
    for region in &mut ds.linked_regions {
        if let LinkSource::InternalSheet { ref mut sheet_id, .. } = region.spec.source {
            apply_sheet_remap(sheet_id, remap);
        }
    }
    remap_floating_notes(&mut ds.floating_notes, remap);
}

fn remap_graph_sheet_references(gs: &mut GraphSheet, remap: &BTreeMap<SheetId, SheetId>) {
    remap_graph_spec_references(&mut gs.spec, remap);
    remap_floating_notes(&mut gs.floating_notes, remap);
}

fn remap_layout_sheet_references(ls: &mut LayoutSheet, remap: &BTreeMap<SheetId, SheetId>) {
    remap_layout_spec_references(&mut ls.spec, remap);
    remap_floating_notes(&mut ls.floating_notes, remap);
}

fn remap_floating_notes(notes: &mut [FloatingNote], remap: &BTreeMap<SheetId, SheetId>) {
    for note in notes {
        remap_inline_hook_tokens(&mut note.body_markdown, remap);
    }
}

fn find_longest_remapped_prefix<'a>(after_kind: &'a str, remap: &'a BTreeMap<SheetId, SheetId>) -> Option<(&'a SheetId, &'a str)> {
    remap
        .iter()
        .filter_map(|(old_id, new_id)| {
            let rest = after_kind.strip_prefix(old_id.as_str())?;
            let after_colon = rest.strip_prefix(':')?;
            Some((old_id.as_str().len(), new_id, after_colon))
        })
        .max_by_key(|(len, _, _)| *len)
        .map(|(_, new_id, rest)| (new_id, rest))
}

fn rewrite_hook_token(raw_inner: &str, remap: &BTreeMap<SheetId, SheetId>) -> Option<String> {
    let wrapped = format!("{{{{{raw_inner}}}}}");
    if let Some(hook) = extract_template_hooks(&wrapped).into_iter().next() {
        let rewritten = match hook {
            HookSource::InfoConstant { info_sheet_id: Some(ref old_id), ref constant_name } => {
                remap.get(old_id).map(|new_id| format!("{{{{info:{new_id}:{constant_name}}}}}"))
            }
            HookSource::ResultScalar { ref result_sheet_id, ref parameter_key, data_set_index } => remap.get(result_sheet_id).map(|new_id| {
                if let Some(ds) = data_set_index {
                    format!("{{{{result:{new_id}:{ds}:{parameter_key}}}}}")
                } else {
                    format!("{{{{result:{new_id}:{parameter_key}}}}}")
                }
            }),
            _ => None,
        };
        if rewritten.is_some() {
            return rewritten;
        }
    }

    let trimmed = raw_inner.trim();
    if let Some(after_info) = trimmed.strip_prefix("info:") {
        let after_info = after_info.trim_start();
        if let Some((new_id, rest)) = find_longest_remapped_prefix(after_info, remap) {
            let name = rest.trim();
            if !name.is_empty() {
                return Some(format!("{{{{info:{new_id}:{name}}}}}"));
            }
        }
    } else if let Some(after_res) = trimmed.strip_prefix("result:") {
        let after_res = after_res.trim_start();
        if let Some((new_id, rest)) = find_longest_remapped_prefix(after_res, remap) {
            if let Some((ds_str, param)) = rest.split_once(':')
                && let Ok(ds) = ds_str.trim().parse::<usize>()
                && !param.trim().is_empty()
            {
                return Some(format!("{{{{result:{new_id}:{ds}:{}}}}}", param.trim()));
            }
            let param = rest.trim();
            if !param.is_empty() {
                return Some(format!("{{{{result:{new_id}:{param}}}}}"));
            }
        }
    } else if let Some(after_scalar) = trimmed.strip_prefix("scalar:") {
        let after_scalar = after_scalar.trim_start();
        if let Some((new_id, rest)) = find_longest_remapped_prefix(after_scalar, remap) {
            let tail = rest.trim();
            if !tail.is_empty() {
                return Some(format!("{{{{scalar:{new_id}:{tail}}}}}"));
            }
        }
    }
    None
}
