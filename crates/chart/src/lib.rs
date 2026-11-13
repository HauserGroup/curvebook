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

use lumencraft_doc::{GraphKind, GraphSheet, LayoutSheet, ProjectDocument};
use lumencraft_palette::Rgba8;
use lumencraft_text::parse_markup;
pub use scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive,
    StrokePaint, TextAnchor,
};

use serde::{Deserialize, Serialize};

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
