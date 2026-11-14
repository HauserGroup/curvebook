//! Scientific graph scene compiler for LumenCraft: maps typed data tables, error-bar summaries
//! (SD, SEM, 95% CI, range, IQR), fitted regression curves, survival step functions, segmented axes
//! and pairwise significance brackets into device-independent figure geometry.
//!
//! Layer L2 domain crate. Governing spec: `docs/graphing-and-layouts.md` and `docs/graphing-parity.md`.
#![forbid(unsafe_code)]

pub mod column_grouped;
pub mod multivariate_layout;
pub mod scales;
pub mod scene;
pub mod survival_cat;
pub mod xy;

use lumencraft_doc::{GraphKind, GraphSheet, GraphSpec, LayoutSheet, ProjectDocument};
use lumencraft_palette::{MarkerStyle, Rgba8};
use lumencraft_text::parse_markup;
pub use scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, PreflightIssue,
    RectPt, ScenePrimitive, SourceCellId, StrokePaint, TextAnchor, preflight_check_scene,
};

use serde::{Deserialize, Serialize};

/// Apply any [`lumencraft_doc::PointStyleOverride`] for `(series_index, point_index)` onto `base`.
///
/// Returns `None` when the point override hides the point (`hidden == true`).
#[must_use]
pub(crate) fn resolve_overridden_marker(spec: &GraphSpec, series_index: usize, point_index: usize, mut base: MarkerStyle) -> Option<MarkerStyle> {
    let Some(ov) = spec.point_override(series_index, point_index) else {
        return Some(base);
    };
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
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&graph.spec.title),
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
    use lumencraft_doc::{DataSheet, GraphSheet, LayoutSheet, LayoutSpec, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};

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
