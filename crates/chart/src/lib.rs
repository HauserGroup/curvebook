//! Scientific graph scene compiler for CurveBook: maps typed data tables, error-bar summaries
//! (SD, SEM, 95% CI, range, IQR), fitted regression curves, survival step functions, segmented axes
//! and pairwise significance brackets into device-independent figure geometry.
//!
//! Layer L2 domain crate. Governing spec: `docs/implementation/graphing-and-layouts.md` and `docs/evidence/graphing.md`.
#![forbid(unsafe_code)]
#![allow(clippy::indexing_slicing)] // Legacy dense layout/UI/lexer indexing (FA-SEC23-03); string_slice remains denied and all 14 I/O/boundary/app crates enforce workspace indexing_slicing = "deny".

pub mod column_grouped;
pub mod equation_box;
pub mod image_decode;
pub mod markers;
pub mod multivariate_layout;
pub mod scales;
pub mod scene;
pub mod survival_cat;
pub mod xy;

use curvebook_doc::{GraphKind, GraphSheet, GraphSpec, LayoutSheet, ProjectDocument};
use curvebook_palette::{MarkerStyle, Rgba8};
pub use scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, PreflightIssue,
    RectPt, ScenePrimitive, SceneRasterImage, SourceCellId, StrokePaint, TextAnchor, preflight_check_scene,
};

use serde::{Deserialize, Serialize};

/// Select the effective [`curvebook_doc::PointStyleOverride`] from `overrides`,
/// matching `(series_index, point_index)` and optional replicate subcolumn `subcol`.
///
/// Prefers exact `subcol` matches over wildcard (`None`) overrides so editing or hiding
/// one replicate never leaks onto sibling replicates in the same row.
#[must_use]
pub fn select_point_override(
    overrides: &[curvebook_doc::PointStyleOverride],
    series_index: usize,
    point_index: usize,
    subcol: Option<usize>,
    override_subcol: impl Fn(&curvebook_doc::PointStyleOverride) -> Option<usize>,
) -> Option<&curvebook_doc::PointStyleOverride> {
    if let Some(target_sub) = subcol
        && let Some(exact) =
            overrides.iter().rfind(|ov| ov.series_index == series_index && ov.point_index == point_index && override_subcol(ov) == Some(target_sub))
    {
        return Some(exact);
    }
    overrides.iter().rfind(|ov| ov.series_index == series_index && ov.point_index == point_index && override_subcol(ov).is_none())
}

/// Apply a [`curvebook_doc::PointStyleOverride`] onto `base`.
///
/// Returns `None` when the point override hides the point (`hidden == true`).
#[must_use]
pub fn apply_point_override(ov: &curvebook_doc::PointStyleOverride, mut base: MarkerStyle) -> Option<MarkerStyle> {
    if ov.hidden {
        return None;
    }
    if let Some(color) = ov.color {
        base.fill_color = color;
        base.stroke_color = color;
    }
    if let Some(fill_color) = ov.fill_color {
        base.fill_color = fill_color;
    }
    if let Some(symbol) = ov.symbol {
        base.shape = symbol;
    }
    if let Some(size_pt) = ov.size_pt
        && size_pt.is_finite()
        && size_pt > 0.0
    {
        base.size_pt = size_pt;
    }
    if let Some(border_width_pt) = ov.border_width_pt
        && border_width_pt.is_finite()
        && border_width_pt >= 0.0
    {
        base.stroke_width_pt = border_width_pt;
    }
    if let Some(fill_mode) = ov.fill_mode {
        base.fill_mode = fill_mode;
    }
    Some(base)
}

/// Apply any [`curvebook_doc::PointStyleOverride`] for `(series_index, point_index, subcol)` onto `base`.
///
/// Returns `None` when the point override hides the point (`hidden == true`).
#[must_use]
pub(crate) fn resolve_overridden_marker_for_cell(
    spec: &GraphSpec,
    series_index: usize,
    point_index: usize,
    subcol: Option<usize>,
    base: MarkerStyle,
) -> Option<MarkerStyle> {
    let Some(ov) = select_point_override(&spec.point_overrides, series_index, point_index, subcol, |ov| ov.subcol) else {
        return Some(base);
    };
    apply_point_override(ov, base)
}

/// Resolve the canonical `PointStyleOverride::series_index` for `(sheet_id, col)` on `spec`.
///
/// When `spec.series` is non-empty, returns the zero-based slot index of the matching entry in
/// `spec.series` (`None` when no slot binds `(sheet_id, col)`). Falls back to `Some(col)` ONLY
/// when `spec.series.is_empty()` (e.g., `MultipleVariables` or unseeded `Diagnostic` plots).
#[must_use]
pub(crate) fn override_series_index_for_col(spec: &GraphSpec, sheet_id: &curvebook_doc::SheetId, col: usize) -> Option<usize> {
    if spec.series.is_empty() { Some(col) } else { spec.series.iter().position(|s| s.source_sheet_id == *sheet_id && s.data_set_index == col) }
}

/// Resolve marker style overrides for a plotted point identified by `(sheet_id, col, row, subcol)`.
#[must_use]
pub(crate) fn resolve_overridden_marker_for_col(
    spec: &GraphSpec,
    sheet_id: &curvebook_doc::SheetId,
    col: usize,
    point_index: usize,
    subcol: Option<usize>,
    base: MarkerStyle,
) -> Option<MarkerStyle> {
    let Some(series_index) = override_series_index_for_col(spec, sheet_id, col) else {
        return Some(base);
    };
    resolve_overridden_marker_for_cell(spec, series_index, point_index, subcol, base)
}

/// High-level graph families corresponding to typed scientific data tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GraphFamily {
    XyScatterAndLine,
    ColumnBarBoxViolin,
    GroupedComparison,
    ContingencyBar,
    SurvivalKaplanMeier,
    PartsOfWhole,
    MultipleVariablesBubbleHeatmap,
    NestedSubcolumn,
}

/// Clamp `value` into `[min, max]` without panicking if `min > max` or any argument is `NaN`.
#[must_use]
pub(crate) fn clamp_f64_safe(value: f64, min: f64, max: f64) -> f64 {
    if min.is_nan() && max.is_nan() {
        return if value.is_nan() { 0.0 } else { value };
    }
    if min.is_nan() {
        return if value.is_nan() { max } else { value.min(max) };
    }
    if max.is_nan() {
        return if value.is_nan() { min } else { value.max(min) };
    }
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    if value.is_nan() { lo } else { value.clamp(lo, hi) }
}

/// Compile a single [`GraphSheet`] within `doc` into a resolution-independent [`ChartScene`].
///
/// # Errors
/// Returns [`ChartError::InvalidSpec`] when required canvas dimensions or parameters are invalid.
pub fn compile_graph(graph: &GraphSheet, doc: &ProjectDocument) -> Result<ChartScene, ChartError> {
    let size = graph.spec.canvas_size_pt;
    if !size.width.is_finite() || !size.height.is_finite() || size.width <= 0.0 || size.height <= 0.0 {
        return Err(ChartError::InvalidSpec { message: format!("invalid graph canvas dimensions ({}, {})", size.width, size.height) });
    }
    let mut scene = ChartScene::new(graph.id.as_str(), graph.title.clone(), size.width, size.height, graph.spec.paper_fill);

    match graph.spec.kind {
        GraphKind::Xy => xy::compile_xy_graph(graph, doc, &mut scene)?,
        GraphKind::Column | GraphKind::Grouped => column_grouped::compile_column_or_grouped_graph(graph, doc, &mut scene)?,
        GraphKind::Survival | GraphKind::Contingency | GraphKind::PartsOfWhole => {
            survival_cat::compile_survival_or_categorical_graph(graph, doc, &mut scene)?;
        }
        GraphKind::MultipleVariables | GraphKind::Diagnostic(_) => {
            multivariate_layout::compile_multivariate_or_diagnostic_graph(graph, doc, &mut scene)?;
        }
    }

    if !graph.spec.title.trim().is_empty() {
        let info_id = xy::primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: xy::expand_doc_text(&graph.spec.title, doc, info_id),
            anchor_pt: PointPt::new(scene.width_pt * 0.5, 18.0),
            anchor: TextAnchor::Middle,
            font_size_pt: 12.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }

    Ok(scene)
}

/// Compile a multi-panel [`LayoutSheet`] within `doc` into a physical-page [`ChartScene`].
///
/// # Errors
/// Returns [`ChartError::InvalidSpec`] when page dimensions are zero or non-finite.
pub fn compile_layout(layout: &LayoutSheet, doc: &ProjectDocument) -> Result<ChartScene, ChartError> {
    let size = layout.spec.page_size_pt();
    if !size.width.is_finite() || !size.height.is_finite() || size.width <= 0.0 || size.height <= 0.0 {
        return Err(ChartError::InvalidSpec { message: format!("invalid layout page dimensions ({}, {})", size.width, size.height) });
    }
    multivariate_layout::compile_layout_scene(layout, doc)
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{DataSheet, GraphSheet, LayoutSheet, LayoutSpec, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};

    use super::{GraphFamily, compile_graph, compile_layout};

    #[test]
    fn eight_graph_families_defined() {
        assert_ne!(GraphFamily::XyScatterAndLine, GraphFamily::SurvivalKaplanMeier);
    }

    #[test]
    fn compile_graph_and_layout_smoke() {
        let mut doc = ProjectDocument::default();
        let archetype = TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 12).expect("xy table"));
        let data = DataSheet::new(SheetId::new("data-1"), "Dose Response", archetype).expect("valid datasheet");
        let graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-1"), &data).expect("valid graph");
        let layout = LayoutSheet::new(SheetId::new("layout-1"), "Figure 1", LayoutSpec::default()).expect("valid layout");
        doc.add_data_sheet(data).expect("add data");
        doc.add_graph_sheet(graph.clone()).expect("add graph");
        doc.add_layout_sheet(layout.clone()).expect("add layout");

        let g_scene = compile_graph(&graph, &doc).expect("compile graph");
        assert!(!g_scene.primitives.is_empty());

        let l_scene = compile_layout(&layout, &doc).expect("compile layout");
        assert!(l_scene.width_pt > 0.0);
    }
}
