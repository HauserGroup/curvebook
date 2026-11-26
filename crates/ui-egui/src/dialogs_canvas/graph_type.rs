//! `Change Graph Type` thumbnail gallery catalog and dialog state machine.

use curvebook_doc::{
    ColumnDesignHint, ConnectingLineStyle, DataSheet, DiagnosticPlotKind, ErrorBarMetric, ErrorBarSpec, GraphKind, GraphSheet, GraphSpec,
    GraphicalDataObject, GroupedArrangement, LineConnectionKind, PartsOfWholeLayout, ProjectDocument, ReplicateLineMode, ReplicateOverlayStyle,
    SheetId, SummaryMeasure, SurvivalDisplayMetric, TableArchetype, TableKind, WhiskerMethod,
};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::edit_params::default_true_flag;

// ============================================================================
// Change Graph Type Gallery, Format Graph, Format Axes, Brackets, Bubble, Clone Style, Layout
// ============================================================================

/// Thumbnail glyph kind rendered in the first-visit `Change Graph Type` chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThumbnailGlyphKind {
    Scatter,
    ScatterLine,
    BarReplicates,
    BeforeAfter,
    BoxWhisker,
    Violin,
    AlignedDot,
    GroupedInterleaved,
    StackedBar,
    KaplanMeier,
    Donut,
    BubbleScatter,
    Heatmap,
    Volcano,
    BlandAltman,
    RocCurve,
    ForestPlot,
    ResidualPlot,
    NormalQqPlot,
}

/// Entry in the `Change Graph Type` schematic thumbnail chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphTypeOption {
    pub id: &'static str,
    pub label: &'static str,
    pub subtitle: &'static str,
    pub graph_kind: GraphKind,
    pub thumbnail: ThumbnailGlyphKind,
    pub compatible_tables: &'static [TableKind],
}

pub const GRAPH_TYPE_OPTIONS: &[GraphTypeOption] = &[
    GraphTypeOption {
        id: "xy.scatter",
        label: "XY Scatter (Points Only)",
        subtitle: "Individual replicate markers or Mean +/- SD error bars",
        graph_kind: GraphKind::Xy,
        thumbnail: ThumbnailGlyphKind::Scatter,
        compatible_tables: &[TableKind::Xy],
    },
    GraphTypeOption {
        id: "xy.scatter_line",
        label: "XY Points + Connecting Lines",
        subtitle: "Point-to-point or spline curve connecting series points",
        graph_kind: GraphKind::Xy,
        thumbnail: ThumbnailGlyphKind::ScatterLine,
        compatible_tables: &[TableKind::Xy],
    },
    GraphTypeOption {
        id: "col.bar_replicates",
        label: "Bar with Individual Points",
        subtitle: "Mean/Median bar + deterministic beeswarm replicate overlay",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::BarReplicates,
        compatible_tables: &[TableKind::Column, TableKind::Nested],
    },
    GraphTypeOption {
        id: "col.before_after",
        label: "Before-After Paired Lines",
        subtitle: "Connect matched subjects across paired conditions",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::BeforeAfter,
        compatible_tables: &[TableKind::Column],
    },
    GraphTypeOption {
        id: "col.box_whisker",
        label: "Box and Whiskers (Tukey / Min-Max)",
        subtitle: "Median, IQR box, Tukey whiskers, and outlier points",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::BoxWhisker,
        compatible_tables: &[TableKind::Column, TableKind::Grouped],
    },
    GraphTypeOption {
        id: "col.violin",
        label: "Violin Plot",
        subtitle: "Kernel density contour clipped to observed data range",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::Violin,
        compatible_tables: &[TableKind::Column, TableKind::Grouped],
    },
    GraphTypeOption {
        id: "col.raincloud",
        label: "Raincloud Plot (Half-Violin + Points)",
        subtitle: "Half-violin kernel density with aligned individual replicate points",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::Violin,
        compatible_tables: &[TableKind::Column, TableKind::Grouped],
    },
    GraphTypeOption {
        id: "col.aligned_dot",
        label: "Scatter Dot Plot + Crossbar",
        subtitle: "Beeswarm or aligned points with Mean/Median summary line",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::AlignedDot,
        compatible_tables: &[TableKind::Column, TableKind::Nested],
    },
    GraphTypeOption {
        id: "grouped.interleaved",
        label: "Grouped Interleaved Bars",
        subtitle: "Clustered bars across Row Factor levels with error bars",
        graph_kind: GraphKind::Grouped,
        thumbnail: ThumbnailGlyphKind::GroupedInterleaved,
        compatible_tables: &[TableKind::Grouped, TableKind::Contingency],
    },
    GraphTypeOption {
        id: "grouped.stacked",
        label: "Grouped / Contingency Stacked",
        subtitle: "Stacked component bars across categorical rows",
        graph_kind: GraphKind::Grouped,
        thumbnail: ThumbnailGlyphKind::StackedBar,
        compatible_tables: &[TableKind::Grouped, TableKind::Contingency],
    },
    GraphTypeOption {
        id: "grouped.superimposed",
        label: "Grouped Superimposed",
        subtitle: "Overlaid series across Row Factor levels for direct comparison",
        graph_kind: GraphKind::Grouped,
        thumbnail: ThumbnailGlyphKind::GroupedInterleaved,
        compatible_tables: &[TableKind::Grouped],
    },
    GraphTypeOption {
        id: "survival.km",
        label: "Kaplan-Meier Survival Curve",
        subtitle: "Step survival function with censoring ticks & Greenwood CIs",
        graph_kind: GraphKind::Survival,
        thumbnail: ThumbnailGlyphKind::KaplanMeier,
        compatible_tables: &[TableKind::Survival],
    },
    GraphTypeOption {
        id: "parts.donut",
        label: "Donut / Pie Parts of Whole",
        subtitle: "Proportional slices or 10x10 waffle grid with center total",
        graph_kind: GraphKind::PartsOfWhole,
        thumbnail: ThumbnailGlyphKind::Donut,
        compatible_tables: &[TableKind::PartsOfWhole],
    },
    GraphTypeOption {
        id: "mv.bubble",
        label: "Bubble Plot (5-Channel Multivariate)",
        subtitle: "Map X, Y, Color (categorical/continuous), Size, and Label",
        graph_kind: GraphKind::MultipleVariables,
        thumbnail: ThumbnailGlyphKind::BubbleScatter,
        compatible_tables: &[TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.heatmap",
        label: "Heatmap (2D Matrix)",
        subtitle: "Perceptual Viridis / Cividis / Diverging tile grid",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
        thumbnail: ThumbnailGlyphKind::Heatmap,
        compatible_tables: &[TableKind::Grouped, TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.volcano",
        label: "Volcano Plot",
        subtitle: "Log2 fold change vs -log10(q) with dual threshold cutoffs",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::Volcano),
        thumbnail: ThumbnailGlyphKind::Volcano,
        compatible_tables: &[TableKind::Xy, TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.bland_altman",
        label: "Bland-Altman Method Comparison",
        subtitle: "Mean vs Difference with bias and +/- 1.96 SD limits",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman),
        thumbnail: ThumbnailGlyphKind::BlandAltman,
        compatible_tables: &[TableKind::Column, TableKind::Xy],
    },
    GraphTypeOption {
        id: "diag.roc",
        label: "ROC Curve Diagnostic",
        subtitle: "Sensitivity vs 1-Specificity with diagonal identity line",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve),
        thumbnail: ThumbnailGlyphKind::RocCurve,
        compatible_tables: &[TableKind::Column, TableKind::Xy],
    },
    GraphTypeOption {
        id: "diag.forest",
        label: "Forest Plot (Effects & 95% CIs)",
        subtitle: "Horizontal point-estimate and confidence-interval whiskers",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot),
        thumbnail: ThumbnailGlyphKind::ForestPlot,
        compatible_tables: &[TableKind::Grouped, TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.residual",
        label: "Residual Diagnostic Plot",
        subtitle: "Fitted/predicted vs residual scatter diagnostic",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot),
        thumbnail: ThumbnailGlyphKind::ResidualPlot,
        compatible_tables: &[TableKind::Xy, TableKind::Column, TableKind::Grouped, TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.normal_qq",
        label: "Normal Q-Q Plot",
        subtitle: "Theoretical normal quantiles vs standardized residuals",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot),
        thumbnail: ThumbnailGlyphKind::NormalQqPlot,
        compatible_tables: &[TableKind::Xy, TableKind::Column, TableKind::Grouped, TableKind::MultipleVariables],
    },
];

/// First-visit `Change Graph Type` chooser state (`docs/implementation/graphing-and-layouts.md` §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeGraphTypeState {
    pub graph_sheet_id: Option<SheetId>,
    pub source_table_kind: Option<TableKind>,
    pub column_design_hint: Option<ColumnDesignHint>,
    pub selected_option_id: String,
    pub summary_measure: SummaryMeasure,
    pub error_bar_metric: ErrorBarMetric,
    #[serde(default)]
    pub survival_metric: SurvivalDisplayMetric,
    #[serde(default = "default_true_flag")]
    pub survival_staircase: bool,
    #[serde(default)]
    pub survival_error_display: Option<ErrorBarMetric>,
}

impl Default for ChangeGraphTypeState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            source_table_kind: None,
            column_design_hint: None,
            selected_option_id: String::from("xy.scatter"),
            summary_measure: SummaryMeasure::Mean,
            error_bar_metric: ErrorBarMetric::Sd,
            survival_metric: SurvivalDisplayMetric::PercentSurvival,
            survival_staircase: true,
            survival_error_display: None,
        }
    }
}

impl ChangeGraphTypeState {
    fn resolved_graph_kind(&self, opt: &GraphTypeOption, doc: Option<&ProjectDocument>) -> GraphKind {
        if opt.id == "grouped.stacked" {
            let is_contingency = self.source_table_kind == Some(TableKind::Contingency)
                || self.graph_sheet_id.as_ref().and_then(|gid| doc?.graph_sheet(gid)).is_some_and(|gs| {
                    doc.and_then(|d| d.data_sheet(&gs.spec.primary_source_sheet_id)).is_some_and(|ds| ds.kind() == TableKind::Contingency)
                });
            if is_contingency {
                return GraphKind::Contingency;
            }
            return GraphKind::Grouped;
        }
        opt.graph_kind
    }

    /// Synchronize chooser state when opening or first visiting `graph`.
    pub fn open_for_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        let src = doc.data_sheet(&graph.spec.primary_source_sheet_id);
        self.source_table_kind = src.map(DataSheet::kind);
        self.column_design_hint = src.and_then(|ds| match &ds.archetype {
            TableArchetype::Column(c) => Some(c.design_hint),
            _ => None,
        });

        let recommended = self.recommended_options();
        if let Some(first) = recommended.first() {
            self.selected_option_id = String::from(first.id);
        }
        if let Some((metric, staircase, err_disp)) = graph.spec.series.iter().find_map(|s| match s.gdo {
            GraphicalDataObject::KaplanMeierStep { display_metric, staircase, error_display, show_greenwood_ci, .. } => {
                let effective_err = error_display.or_else(|| show_greenwood_ci.then_some(ErrorBarMetric::ConfidenceInterval95));
                Some((display_metric, staircase, effective_err))
            }
            _ => None,
        }) {
            self.survival_metric = metric;
            self.survival_staircase = staircase;
            self.survival_error_display = err_disp;
        } else {
            self.survival_metric = SurvivalDisplayMetric::PercentSurvival;
            self.survival_staircase = true;
            self.survival_error_display = None;
        }
    }

    /// Options tailored to the active `TableArchetype` and `ColumnDesignHint` (`PairedRepeatedMeasures`
    /// prioritizes `Before-After Paired Lines` first).
    #[must_use]
    pub fn recommended_options(&self) -> Vec<&'static GraphTypeOption> {
        let mut out: Vec<&'static GraphTypeOption> = match self.source_table_kind {
            Some(kind) => GRAPH_TYPE_OPTIONS.iter().filter(|opt| opt.compatible_tables.contains(&kind)).collect(),
            None => GRAPH_TYPE_OPTIONS.iter().collect(),
        };
        if self.column_design_hint == Some(ColumnDesignHint::PairedRepeatedMeasures) {
            out.sort_by_key(|opt| usize::from(opt.id != "col.before_after"));
        }
        out
    }

    /// Apply the selected thumbnail choice onto `spec` cleanly.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) -> bool {
        let Some(opt) = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == self.selected_option_id) else {
            return false;
        };
        spec.kind = self.resolved_graph_kind(opt, None);
        if opt.id == "grouped.stacked" {
            spec.grouped_arrangement = GroupedArrangement::Stacked;
        } else if opt.id == "grouped.superimposed" {
            spec.grouped_arrangement = GroupedArrangement::Superimposed;
        } else if opt.id == "grouped.interleaved" {
            spec.grouped_arrangement = GroupedArrangement::Interleaved;
        }
        for (idx, series) in spec.series.iter_mut().enumerate() {
            let color = spec.palette.color_at(idx);
            let marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 5.0,
                fill_color: color,
                stroke_color: color,
                stroke_width_pt: 1.0,
            };
            series.gdo = build_gdo_for_option(opt.id, color, marker, self.summary_measure);
            if let GraphicalDataObject::KaplanMeierStep { show_greenwood_ci, display_metric, staircase, error_display, .. } = &mut series.gdo {
                *display_metric = self.survival_metric;
                *staircase = self.survival_staircase;
                *error_display = self.survival_error_display;
                *show_greenwood_ci = self.survival_error_display.is_some();
            }
            if opt.id == "grouped.stacked" {
                series.y_error_bars = None;
            } else if matches!(series.gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::ScatterAndLine { .. }) {
                series.y_error_bars = Some(ErrorBarSpec { metric: self.error_bar_metric, ..ErrorBarSpec::default() });
            } else {
                series.y_error_bars = None;
            }
        }
        true
    }

    /// Emit `graph.setKind` (or `graph.setSpec` for `grouped.superimposed`, which carries
    /// `GroupedArrangement::Superimposed`) command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        let opt = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == self.selected_option_id)?;
        let kind = self.resolved_graph_kind(opt, None);
        if opt.id == "grouped.superimposed" {
            return Some(ToolCommandIntent::new(
                "graph.setSpec",
                json!({
                    "sheet_id": id.as_str(),
                    "kind": kind,
                    "grouped_arrangement": GroupedArrangement::Superimposed,
                    "initialized": true,
                }),
            ));
        }
        Some(ToolCommandIntent::new(
            "graph.setKind",
            json!({
                "sheet_id": id.as_str(),
                "option_id": opt.id,
                "kind": kind,
                "reset_series_style": true,
            }),
        ))
    }

    /// Emit `graph.setSpec` + `graph.setSeries` command intents applying full `option_id` layout,
    /// `grouped_arrangement`, GDO style, and error-bar rules cleanly via `Caller::Ui`.
    #[must_use]
    pub fn commit_intents(&self, doc: &ProjectDocument) -> Vec<ToolCommandIntent> {
        let Some(id) = self.graph_sheet_id.as_ref() else {
            return Vec::new();
        };
        let Some(opt) = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == self.selected_option_id) else {
            return Vec::new();
        };
        let Some(mut spec) = doc.graph_sheet(id).map(|g| g.spec.clone()) else {
            let kind = self.resolved_graph_kind(opt, None);
            let mut intents = vec![ToolCommandIntent::new(
                "graph.setKind",
                json!({
                    "sheet_id": id.as_str(),
                    "option_id": opt.id,
                    "kind": kind,
                    "reset_series_style": true,
                }),
            )];
            if opt.id == "grouped.superimposed" {
                intents.push(ToolCommandIntent::new(
                    "graph.setSpec",
                    json!({
                        "sheet_id": id.as_str(),
                        "grouped_arrangement": GroupedArrangement::Superimposed,
                    }),
                ));
            }
            return intents;
        };
        let _ = self.apply_to_spec(&mut spec);
        spec.kind = self.resolved_graph_kind(opt, Some(doc));

        let mut intents = vec![ToolCommandIntent::new(
            "graph.setKind",
            json!({
                "sheet_id": id.as_str(),
                "option_id": opt.id,
                "kind": spec.kind,
                "reset_series_style": true,
            }),
        )];
        if spec.kind == GraphKind::Grouped {
            intents.push(ToolCommandIntent::new(
                "graph.setSpec",
                json!({
                    "sheet_id": id.as_str(),
                    "grouped_arrangement": spec.grouped_arrangement,
                }),
            ));
        }
        if !matches!(spec.kind, GraphKind::Diagnostic(_)) && !spec.series.is_empty() {
            let series_patches: Vec<Value> = spec
                .series
                .iter()
                .map(|s| {
                    json!({
                        "gdo": s.gdo,
                        "y_error_bars": s.y_error_bars,
                    })
                })
                .collect();
            intents.push(ToolCommandIntent::new(
                "graph.setSeries",
                json!({
                    "sheet_id": id.as_str(),
                    "series": series_patches,
                }),
            ));
        }
        intents
    }
}

pub(super) fn build_gdo_for_option(option_id: &str, color: Rgba8, marker: MarkerStyle, summary: SummaryMeasure) -> GraphicalDataObject {
    match option_id {
        "xy.scatter_line" => GraphicalDataObject::ScatterAndLine {
            marker: Some(marker),
            line: Some(ConnectingLineStyle { kind: LineConnectionKind::PointToPoint, dash_style: LineDashStyle::Solid, stroke_width_pt: 1.5, color }),
            area_fill: None,
            replicate_lines: ReplicateLineMode::default(),
        },
        "col.before_after" => GraphicalDataObject::BeforeAfterLines { marker: Some(marker), line_color: color, stroke_width_pt: 1.2 },
        "col.box_whisker" => GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::Tukey1_5Iqr,
            fill_color: color.with_alpha(190),
            stroke_color: color,
            show_outliers: true,
            show_all_points: false,
        },
        "col.violin" => GraphicalDataObject::Violin {
            fill_color: color.with_alpha(190),
            stroke_color: color,
            clip_to_observed_range: true,
            show_median_and_quartiles: true,
            half_violin_with_points: false,
        },
        "col.raincloud" => GraphicalDataObject::Violin {
            fill_color: color.with_alpha(190),
            stroke_color: color,
            clip_to_observed_range: true,
            show_median_and_quartiles: true,
            half_violin_with_points: true,
        },
        "col.aligned_dot" => GraphicalDataObject::AlignedDotPlot { summary, marker, show_crossbar: true },
        "col.bar_replicates" | "grouped.interleaved" | "grouped.stacked" | "grouped.superimposed" => GraphicalDataObject::BarWithReplicates {
            summary,
            fill_color: color.with_alpha(210),
            border_color: color,
            border_width_pt: 1.0,
            replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
        },
        "survival.km" => GraphicalDataObject::KaplanMeierStep {
            line_color: color,
            stroke_width_pt: 1.5,
            show_censored_ticks: true,
            show_greenwood_ci: false,
            ci_dashed: true,
            display_metric: SurvivalDisplayMetric::default(),
            staircase: true,
            error_display: None,
        },
        "parts.donut" => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Donut, show_center_total: true },
        _ => GraphicalDataObject::ScatterAndLine { marker: Some(marker), line: None, area_fill: None, replicate_lines: ReplicateLineMode::default() },
    }
}
