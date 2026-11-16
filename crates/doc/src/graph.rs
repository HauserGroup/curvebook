//! Declarative `GraphSpec` and `GraphSheet` schemas: graphical data objects (GDOs), nonlinear and
//! discontinuous 1–3 segment axes, dashed/solid error bars, five-channel `MultipleVariables`
//! bubble/categorical encodings, live pairwise significance brackets, and style cloning
//! (`docs/graphing-and-layouts.md` §1–§5).

pub use curvebook_math::{AxisScale, LogitDomain};
use curvebook_palette::{ColorRamp, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};

use crate::archetypes::{ColumnDesignHint, LevelOrderSpec, TableArchetype};
use crate::error::{DocError, validate_title};
use crate::ids::{FloatingNote, HookSource, RectPt, SheetId, SizePt, extract_template_hooks};
use crate::links::DataSheet;

/// Specialized diagnostic plot subtypes (`docs/graphing-and-layouts.md` §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DiagnosticPlotKind {
    Heatmap,
    Volcano,
    BlandAltman,
    RocCurve,
    ForestPlot,
    ResidualPlot,
    NormalQqPlot,
}

/// Primary visual family of a [`GraphSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GraphKind {
    Xy,
    Column,
    Grouped,
    Contingency,
    PartsOfWhole,
    Survival,
    MultipleVariables,
    Diagnostic(DiagnosticPlotKind),
}

/// Axis frame geometry preset (`docs/graphing-and-layouts.md` §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum FramePreset {
    #[default]
    OpenL,
    OffsetRangeTufte,
    PlainBoxed,
    DualYSecondaryRight,
    OriginCrossing,
}

/// Guarded axis scale bijection (`docs/graphing-and-layouts.md` §3.1), unified with [`curvebook_math::AxisScale`].
pub type GraphAxisScale = AxisScale;

/// Break-mark ornament between discontinuous axis segments (`docs/graphing-and-layouts.md` §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum BreakOrnament {
    #[default]
    DoubleSlash,
    Zigzag,
    BlankGap,
}

/// Source for categorical tick labels on `Column` and `Grouped` X axes (`docs/graphing-and-layouts.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum CategoricalTickSource {
    #[default]
    RowTitles,
    ColumnTitles,
    RowNumbers,
}

/// Numeric or categorical tick formatting mode (`docs/graphing-and-layouts.md` §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum TickLabelFormat {
    #[default]
    Decimal,
    PowerOfTenSuperscript,
    ScientificE,
    Antilog,
    EngineeringSi,
    Categorical(CategoricalTickSource),
}

/// Reference threshold rule or shaded band drawn across a plot area (e.g. Volcano alpha/fold-change cutoffs).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AxisThreshold {
    pub value: f64,
    pub shade_to: Option<f64>,
    pub color: Rgba8,
    pub dash_style: LineDashStyle,
    pub stroke_width_pt: f64,
    pub label: Option<String>,
}

/// Custom extra tick mark (with optional label override and gridline) placed on an axis.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct AdditionalTick {
    pub value: f64,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub draw_gridline: bool,
}

/// One continuous segment (`1..=3` per axis) of a standard or discontinuous axis (`docs/graphing-and-layouts.md` §3.2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AxisSegment {
    /// `None` selects automatic data-driven lower bound.
    pub min: Option<f64>,
    /// `None` selects automatic data-driven upper bound.
    pub max: Option<f64>,
    pub scale: GraphAxisScale,
    /// Relative physical length weight on the rendered axis (`> 0.0`).
    pub length_share: f64,
    pub major_tick_interval: Option<f64>,
    pub minor_ticks_per_interval: u8,
}

impl Default for AxisSegment {
    fn default() -> Self {
        Self { min: None, max: None, scale: GraphAxisScale::Linear, length_share: 1.0, major_tick_interval: None, minor_ticks_per_interval: 0 }
    }
}

/// Complete X, Y, or secondary Y2 axis model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AxisSpec {
    pub title: String,
    pub segments: Vec<AxisSegment>,
    pub gap_width_pt: f64,
    pub break_ornament: BreakOrnament,
    pub tick_format: TickLabelFormat,
    pub tick_angle_deg: f64,
    pub stroke_width_pt: f64,
    pub thresholds: Vec<AxisThreshold>,
    #[serde(default)]
    pub additional_ticks: Vec<AdditionalTick>,
}

impl AxisSpec {
    #[must_use]
    pub fn linear(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            segments: vec![AxisSegment::default()],
            gap_width_pt: 8.0,
            break_ornament: BreakOrnament::DoubleSlash,
            tick_format: TickLabelFormat::Decimal,
            tick_angle_deg: 0.0,
            stroke_width_pt: 1.0,
            thresholds: Vec::new(),
            additional_ticks: Vec::new(),
        }
    }

    /// Return the first non-finite float field name on this axis specification, if any.
    #[must_use]
    pub fn first_non_finite_float(&self) -> Option<&'static str> {
        if !self.gap_width_pt.is_finite() {
            return Some("axis.gap_width_pt");
        }
        if !self.tick_angle_deg.is_finite() {
            return Some("axis.tick_angle_deg");
        }
        if !self.stroke_width_pt.is_finite() {
            return Some("axis.stroke_width_pt");
        }
        for seg in &self.segments {
            if seg.min.is_some_and(|v| !v.is_finite()) {
                return Some("axis.segments.min");
            }
            if seg.max.is_some_and(|v| !v.is_finite()) {
                return Some("axis.segments.max");
            }
            if !seg.length_share.is_finite() {
                return Some("axis.segments.length_share");
            }
            if seg.major_tick_interval.is_some_and(|v| !v.is_finite()) {
                return Some("axis.segments.major_tick_interval");
            }
        }
        for threshold in &self.thresholds {
            if !threshold.value.is_finite() {
                return Some("axis.thresholds.value");
            }
            if threshold.shade_to.is_some_and(|v| !v.is_finite()) {
                return Some("axis.thresholds.shade_to");
            }
            if !threshold.stroke_width_pt.is_finite() {
                return Some("axis.thresholds.stroke_width_pt");
            }
        }
        for tick in &self.additional_ticks {
            if !tick.value.is_finite() {
                return Some("axis.additional_ticks.value");
            }
        }
        None
    }

    /// Enforce that discontinuous axes carry `1`, `2`, or `3` segments (`docs/graphing-and-layouts.md` §3.2)
    /// and validate axis labels, custom tick labels, and float finiteness.
    pub fn validate(&self) -> Result<(), DocError> {
        validate_title(&self.title)?;
        if !(1..=3).contains(&self.segments.len()) {
            return Err(DocError::InvalidAxisSegments { count: self.segments.len() });
        }
        for tick in &self.additional_ticks {
            if let Some(label) = &tick.label {
                validate_title(label)?;
            }
        }
        if let Some(reason) = self.first_non_finite_float() {
            return Err(DocError::InvalidCellValue { archetype: crate::TableKind::Xy, row: 0, col: 0, reason });
        }
        Ok(())
    }
}

/// Central tendency statistic for bars and summary crossbars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SummaryMeasure {
    #[default]
    Mean,
    Median,
    GeometricMean,
}

/// Whisker extent convention for box-and-whisker plots (`docs/graphing-and-layouts.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum WhiskerMethod {
    #[default]
    Tukey1_5Iqr,
    MinToMax,
    Percentile10To90,
    Percentile5To95,
    Percentile2_5To97_5,
    Percentile1To99,
}

/// Deterministic replicate point overlay on bars, boxes, and violins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ReplicateOverlayStyle {
    #[default]
    DeterministicBeeswarm,
    AlignedColumn,
}

/// Curve interpolation rule for XY connecting lines (`docs/graphing-and-layouts.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum LineConnectionKind {
    #[default]
    PointToPoint,
    Staircase,
    MonotonicCubicSpline,
    AkimaSpline,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConnectingLineStyle {
    pub kind: LineConnectionKind,
    pub dash_style: LineDashStyle,
    pub stroke_width_pt: f64,
    pub color: Rgba8,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AreaFillSpec {
    pub baseline_y: f64,
    pub fill_color: Rgba8,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum PartsOfWholeLayout {
    Pie,
    #[default]
    Donut,
    StackedFractionBar,
    Waffle10x10,
}

/// Composable Graphical Data Object (GDO) describing how a series renders (`docs/graphing-and-layouts.md` §2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GraphicalDataObject {
    ScatterAndLine {
        marker: Option<MarkerStyle>,
        line: Option<ConnectingLineStyle>,
        area_fill: Option<AreaFillSpec>,
    },
    BarWithReplicates {
        summary: SummaryMeasure,
        fill_color: Rgba8,
        border_color: Rgba8,
        border_width_pt: f64,
        replicate_overlay: Option<ReplicateOverlayStyle>,
    },
    AlignedDotPlot {
        summary: SummaryMeasure,
        marker: MarkerStyle,
        show_crossbar: bool,
    },
    BoxAndWhisker {
        whiskers: WhiskerMethod,
        fill_color: Rgba8,
        stroke_color: Rgba8,
        show_outliers: bool,
        show_all_points: bool,
    },
    Violin {
        fill_color: Rgba8,
        stroke_color: Rgba8,
        clip_to_observed_range: bool,
        show_median_and_quartiles: bool,
        half_violin_with_points: bool,
    },
    FloatingBar {
        summary: Option<SummaryMeasure>,
        fill_color: Rgba8,
        border_color: Rgba8,
        border_width_pt: f64,
    },
    BeforeAfterLines {
        marker: Option<MarkerStyle>,
        line_color: Rgba8,
        stroke_width_pt: f64,
    },
    KaplanMeierStep {
        line_color: Rgba8,
        stroke_width_pt: f64,
        show_censored_ticks: bool,
        show_greenwood_ci: bool,
        ci_dashed: bool,
    },
    PartsOfWhole {
        layout: PartsOfWholeLayout,
        show_center_total: bool,
    },
}

/// Dispersion statistic encoded by error bars (`docs/graphing-and-layouts.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ErrorBarMetric {
    #[default]
    Sd,
    Sem,
    ConfidenceInterval95,
    InterquartileRange,
    Range,
    ExplicitSubcolumns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ErrorBarDirection {
    #[default]
    Both,
    UpperOnly,
    LowerOnly,
}

/// Vertical or horizontal error-bar styling (supporting solid, dashed, and dotted strokes).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ErrorBarSpec {
    pub metric: ErrorBarMetric,
    pub direction: ErrorBarDirection,
    pub dash_style: LineDashStyle,
    pub stroke_width_pt: f64,
    pub cap_width_pt: f64,
    pub color: Option<Rgba8>,
}

impl Default for ErrorBarSpec {
    fn default() -> Self {
        Self {
            metric: ErrorBarMetric::Sd,
            direction: ErrorBarDirection::Both,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.0,
            cap_width_pt: 4.0,
            color: None,
        }
    }
}

/// Overlaid smooth curve bound to an upstream nonlinear/linear regression `ResultSheet`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegressionOverlaySpec {
    pub result_sheet_id: SheetId,
    /// Adaptive polyline subdivision count in `[256, 1024]`.
    pub sample_segments: u16,
    pub show_confidence_band: bool,
    pub show_prediction_band: bool,
    pub shaded_ribbon: bool,
}

/// Single plotted data series inside a [`GraphSpec`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeriesSpec {
    pub source_sheet_id: SheetId,
    pub data_set_index: usize,
    pub legend_label: Option<String>,
    pub bind_to_right_y2: bool,
    pub gdo: GraphicalDataObject,
    pub y_error_bars: Option<ErrorBarSpec>,
    pub x_error_bars: Option<ErrorBarSpec>,
    pub regression_overlay: Option<RegressionOverlaySpec>,
}

/// Horizontal cluster arrangement for `Grouped` and `Contingency` graphs (`docs/graphing-and-layouts.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum GroupedArrangement {
    #[default]
    Interleaved,
    Separated,
    Stacked,
    Superimposed,
}

/// Per-point visual style override (`graph.formatSelectedPoints`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PointStyleOverride {
    pub series_index: usize,
    pub point_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subcol: Option<usize>,
    #[serde(default)]
    pub color: Option<Rgba8>,
    #[serde(default)]
    pub symbol: Option<MarkerShape>,
    #[serde(default)]
    pub size_pt: Option<f64>,
    #[serde(default)]
    pub border_width_pt: Option<f64>,
    #[serde(default)]
    pub fill_color: Option<Rgba8>,
    #[serde(default)]
    pub fill_mode: Option<MarkerFillMode>,
    #[serde(default)]
    pub hidden: bool,
}

/// Fill-colour visual channel in a five-channel `MultipleVariables` plot (`docs/graphing-and-layouts.md` §2.5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ColorChannelMapping {
    Constant(Rgba8),
    Categorical { column: usize, palette: PaletteId, level_order_override: Option<LevelOrderSpec> },
    Continuous { column: usize, ramp: ColorRamp, domain_clamp: Option<(f64, f64)> },
}

/// Symbol-area / radius channel in a five-channel `MultipleVariables` bubble plot.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SizeChannelMapping {
    Constant { radius_pt: f64 },
    Variable { column: usize, min_radius_pt: f64, max_radius_pt: f64 },
}

/// Rendering mode for a `MultipleVariables` graph (`docs/graphing-and-layouts.md` §2.5).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum MultivariatePlotMode {
    /// Five-channel continuous/categorical scatter and bubble plot.
    #[default]
    BubbleScatter,
    /// Categorical × continuous group summary plot (bar, box-and-whisker, violin, dot plot, floating bar).
    CategoricalSummary { gdo: GraphicalDataObject, y_error_bars: Option<ErrorBarSpec> },
}

/// Declarative five-channel (`X`, `Y`, `Color`, `Size`, `Label`) or categorical × continuous
/// encoding for `MultipleVariables` plots.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MultivariateGraphEncoding {
    pub plot_mode: MultivariatePlotMode,
    pub x_column: usize,
    pub y_column: usize,
    pub color: ColorChannelMapping,
    pub size: SizeChannelMapping,
    pub label_column: Option<usize>,
    pub x_level_order_override: Option<LevelOrderSpec>,
    pub show_convex_hulls: bool,
    pub show_confidence_ellipses: bool,
}

/// Geometry style of pairwise significance brackets (`docs/graphing-and-layouts.md` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum BracketStyle {
    #[default]
    Square,
    HalfTick,
    FlatLine,
    CurlyBrace,
}

/// Text label mode for pairwise significance brackets.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BracketLabelMode {
    #[default]
    SummaryStars,
    NumericPOrQ,
    Custom(String),
}

/// Live pairwise comparison bracket hooked to an upstream statistical `ResultSheet`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PairwiseBracketSpec {
    pub result_sheet_id: SheetId,
    pub group_a: usize,
    pub group_b: usize,
    pub style: BracketStyle,
    pub label_mode: BracketLabelMode,
    pub hide_if_non_significant: bool,
    pub stroke_width_pt: f64,
    pub font_size_pt: f64,
    pub bracket_step_pt: f64,
}

/// Geometric shape primitive for canvas callout regions (`docs/graphing-parity.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum AnnotationShapeKind {
    #[default]
    Rectangle,
    RoundedRectangle,
    Ellipse,
}

/// Display mode for a fitted model equation callout on a graph canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum EquationDisplayMode {
    #[default]
    Substituted,
    Symbolic,
}

const fn default_model_equation_font_size() -> f64 {
    10.0
}

/// Canvas annotation primitive placed on a [`GraphSpec`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GraphAnnotation {
    TextBox {
        bounds_pt: RectPt,
        markdown: String,
        bordered: bool,
    },
    LineOrArrow {
        start_pt: (f64, f64),
        end_pt: (f64, f64),
        arrowhead: bool,
        dash: LineDashStyle,
        color: Rgba8,
    },
    Shape {
        kind: AnnotationShapeKind,
        bounds_pt: RectPt,
        stroke_color: Rgba8,
        stroke_width_pt: f64,
        dash: LineDashStyle,
        fill_color: Option<Rgba8>,
    },
    EmbeddedResultTable {
        result_sheet_id: SheetId,
        tab_id: Option<String>,
        bounds_pt: RectPt,
    },
    ModelEquation {
        result_sheet_id: SheetId,
        #[serde(default)]
        data_set_index: usize,
        #[serde(default)]
        mode: EquationDisplayMode,
        bounds_pt: RectPt,
        #[serde(default = "default_model_equation_font_size")]
        font_size_pt: f64,
        #[serde(default)]
        bordered: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LegendSpec {
    pub visible: bool,
    pub position_pt: Option<(f64, f64)>,
    pub bordered: bool,
}

impl Default for LegendSpec {
    fn default() -> Self {
        Self { visible: true, position_pt: None, bordered: false }
    }
}

/// Monotonic ordering direction for Piepho (2004) Compact Letter Display assignments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CldOrdering {
    /// Highest group mean/rank receives `'a'`; lower groups progress monotonically toward `'b'`, `'c'`, ....
    #[default]
    DescendingMean,
    /// Lowest group mean/rank receives `'a'`; higher groups progress monotonically toward `'b'`, `'c'`, ....
    AscendingMean,
}

/// Pairwise significance presentation mode on categorical/column/grouped graphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PairwiseDisplayMode {
    /// Render pairwise significance brackets only (`comparisons`).
    #[default]
    BracketsOnly,
    /// Render Piepho (2004) compact letter display annotations above plotted groups only.
    CompactLetterDisplayOnly,
    /// Render both compact letter display letters and non-overlapping significance brackets.
    BracketsAndCompactLetterDisplay,
}

fn default_cld_font_size() -> f64 {
    9.5
}

fn default_cld_offset_pt() -> f64 {
    6.0
}

fn default_cld_color() -> Rgba8 {
    Rgba8::BLACK
}

fn is_default_pairwise_display_mode(mode: &PairwiseDisplayMode) -> bool {
    *mode == PairwiseDisplayMode::BracketsOnly
}

/// Compact Letter Display (CLD) overlay binding a graph to an all-pairwise [`crate::ResultSheet`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompactLetterDisplaySpec {
    pub result_sheet_id: SheetId,
    /// Optional explicit alpha threshold overriding precomputed `significant` decisions when adjusted p/q values are present.
    #[serde(default)]
    pub alpha_override: Option<f64>,
    #[serde(default)]
    pub ordering: CldOrdering,
    #[serde(default)]
    pub uppercase: bool,
    #[serde(default = "default_cld_font_size")]
    pub font_size_pt: f64,
    #[serde(default = "default_cld_offset_pt")]
    pub offset_pt: f64,
    #[serde(default = "default_cld_color")]
    pub color: Rgba8,
}

impl CompactLetterDisplaySpec {
    /// Default CLD overlay targeting `result_sheet_id`.
    #[must_use]
    pub fn new(result_sheet_id: impl Into<SheetId>) -> Self {
        Self {
            result_sheet_id: result_sheet_id.into(),
            alpha_override: None,
            ordering: CldOrdering::DescendingMean,
            uppercase: false,
            font_size_pt: default_cld_font_size(),
            offset_pt: default_cld_offset_pt(),
            color: default_cld_color(),
        }
    }
}

/// Resolution-independent figure specification compiled by `curvebook-chart`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraphSpec {
    pub kind: GraphKind,
    pub primary_source_sheet_id: SheetId,
    pub palette: PaletteId,
    pub canvas_size_pt: SizePt,
    pub paper_fill: Option<Rgba8>,
    pub title: String,
    pub frame_preset: FramePreset,
    pub x_axis: AxisSpec,
    pub y_axis: AxisSpec,
    pub y2_axis: Option<AxisSpec>,
    pub grouped_arrangement: GroupedArrangement,
    pub survival_number_at_risk: bool,
    pub series: Vec<SeriesSpec>,
    pub multivariate: Option<MultivariateGraphEncoding>,
    pub comparisons: Vec<PairwiseBracketSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_letter_display: Option<CompactLetterDisplaySpec>,
    #[serde(default, skip_serializing_if = "is_default_pairwise_display_mode")]
    pub pairwise_display_mode: PairwiseDisplayMode,
    pub annotations: Vec<GraphAnnotation>,
    #[serde(default)]
    pub point_overrides: Vec<PointStyleOverride>,
    pub legend: LegendSpec,
}

impl GraphSpec {
    /// Synthesize the canonical default [`GraphSpec`] for a newly created [`DataSheet`], honoring
    /// `TableArchetype` and `ColumnDesignHint` (`docs/data-tables-and-transforms.md` §2.2 &
    /// `docs/graphing-and-layouts.md` §2).
    #[must_use]
    pub fn default_for_data_sheet(sheet: &DataSheet) -> Self {
        let palette = PaletteId::OkabeIto;
        let kind = match &sheet.archetype {
            TableArchetype::Xy(_) => GraphKind::Xy,
            TableArchetype::Column(_) | TableArchetype::Nested(_) => GraphKind::Column,
            TableArchetype::Grouped(_) => GraphKind::Grouped,
            TableArchetype::Contingency(_) => GraphKind::Contingency,
            TableArchetype::PartsOfWhole(_) => GraphKind::PartsOfWhole,
            TableArchetype::Survival(_) => GraphKind::Survival,
            TableArchetype::MultipleVariables(_) => GraphKind::MultipleVariables,
        };

        let cols = sheet.archetype.column_count();
        let mut series = Vec::with_capacity(cols);
        for idx in 0..cols {
            let color = palette.color_at(idx);
            let shape = default_marker_shape(idx);
            let marker = MarkerStyle { shape, fill_color: color, stroke_color: color, ..MarkerStyle::default() };
            let gdo = default_gdo_for_archetype(&sheet.archetype, color, marker);
            let y_error_bars = match &sheet.archetype {
                TableArchetype::Xy(_) | TableArchetype::Column(_) | TableArchetype::Grouped(_) => Some(ErrorBarSpec::default()),
                _ => None,
            };
            series.push(SeriesSpec {
                source_sheet_id: sheet.id.clone(),
                data_set_index: idx,
                legend_label: None,
                bind_to_right_y2: false,
                gdo,
                y_error_bars,
                x_error_bars: None,
                regression_overlay: None,
            });
        }

        let x_axis = match &sheet.archetype {
            TableArchetype::Column(_) | TableArchetype::Nested(_) => {
                let mut ax = AxisSpec::linear("");
                ax.tick_format = TickLabelFormat::Categorical(CategoricalTickSource::ColumnTitles);
                ax
            }
            TableArchetype::Grouped(g) => {
                let mut ax = AxisSpec::linear(g.row_factor_title.clone());
                ax.tick_format = TickLabelFormat::Categorical(CategoricalTickSource::RowTitles);
                ax
            }
            TableArchetype::Xy(xy) => AxisSpec::linear(xy.x_title.clone()),
            TableArchetype::Survival(s) => AxisSpec::linear(s.time_title.clone()),
            _ => AxisSpec::linear("X"),
        };

        let multivariate = match &sheet.archetype {
            TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => Some(MultivariateGraphEncoding {
                plot_mode: MultivariatePlotMode::default(),
                x_column: 0,
                y_column: 1,
                color: ColorChannelMapping::Constant(palette.color_at(0)),
                size: SizeChannelMapping::Constant { radius_pt: 5.0 },
                label_column: None,
                x_level_order_override: None,
                show_convex_hulls: false,
                show_confidence_ellipses: false,
            }),
            _ => None,
        };

        Self {
            kind,
            primary_source_sheet_id: sheet.id.clone(),
            palette,
            canvas_size_pt: SizePt::default(),
            paper_fill: Some(Rgba8::WHITE),
            title: sheet.title.clone(),
            frame_preset: FramePreset::OpenL,
            x_axis,
            y_axis: AxisSpec::linear("Y"),
            y2_axis: None,
            grouped_arrangement: GroupedArrangement::Interleaved,
            survival_number_at_risk: matches!(sheet.archetype, TableArchetype::Survival(_)),
            series,
            multivariate,
            comparisons: Vec::new(),
            compact_letter_display: None,
            pairwise_display_mode: PairwiseDisplayMode::default(),
            annotations: Vec::new(),
            point_overrides: Vec::new(),
            legend: LegendSpec::default(),
        }
    }

    /// Look up the effective per-point override for `(series_index, point_index)`, if present.
    #[must_use]
    pub fn point_override(&self, series_index: usize, point_index: usize) -> Option<&PointStyleOverride> {
        self.point_override_with_subcol(series_index, point_index, None)
    }

    /// Look up the effective per-point override for `(series_index, point_index)` and optional replicate `subcol`.
    ///
    /// When `subcol` is `Some(sc)`, an exact replicate match (`ov.subcol == Some(sc)`) takes precedence
    /// over a whole-point override (`ov.subcol.is_none()`).
    #[must_use]
    pub fn point_override_with_subcol(&self, series_index: usize, point_index: usize, subcol: Option<usize>) -> Option<&PointStyleOverride> {
        if let Some(sc) = subcol
            && let Some(exact) =
                self.point_overrides.iter().rfind(|ov| ov.series_index == series_index && ov.point_index == point_index && ov.subcol == Some(sc))
        {
            return Some(exact);
        }
        self.point_overrides.iter().rfind(|ov| ov.series_index == series_index && ov.point_index == point_index && ov.subcol.is_none())
    }

    /// Copy visual styling tokens (`palette`, `frame_preset`, `paper_fill`, axis stroke widths,
    /// legend chrome, and series palette swatches) from `donor` without mutating data bindings
    /// (`graph.applyStyleFrom`; `docs/graphing-and-layouts.md` §5).
    pub fn apply_style_from(&mut self, donor: &Self) {
        self.apply_style_from_selective(donor, true, true, true);
    }

    /// Selectively copy visual styling groups from `donor` without mutating data bindings
    /// (`graph.applyStyleFrom` / `ApplyStyleFrom` dialog toggles).
    pub fn apply_style_from_selective(&mut self, donor: &Self, clone_palette: bool, clone_frame_and_axes: bool, clone_error_bars_and_legend: bool) {
        if clone_palette {
            self.palette = donor.palette;
            self.paper_fill = donor.paper_fill;
        }
        if clone_frame_and_axes {
            self.canvas_size_pt = donor.canvas_size_pt;
            self.frame_preset = donor.frame_preset;
            self.x_axis.stroke_width_pt = donor.x_axis.stroke_width_pt;
            self.x_axis.break_ornament = donor.x_axis.break_ornament;
            self.y_axis.stroke_width_pt = donor.y_axis.stroke_width_pt;
            self.y_axis.break_ornament = donor.y_axis.break_ornament;
        }
        if clone_error_bars_and_legend {
            self.legend.bordered = donor.legend.bordered;
            self.legend.visible = donor.legend.visible;
        }

        let donor_eb = if clone_error_bars_and_legend {
            donor.series.first().and_then(|s| s.y_error_bars).or_else(|| match donor.multivariate.as_ref().map(|m| &m.plot_mode) {
                Some(MultivariatePlotMode::CategoricalSummary { y_error_bars, .. }) => *y_error_bars,
                _ => None,
            })
        } else {
            None
        };

        for (idx, series) in self.series.iter_mut().enumerate() {
            if clone_palette {
                let swatch = self.palette.color_at(idx);
                recolor_gdo(&mut series.gdo, swatch);
            }
            if let Some(d_eb) = donor_eb
                && let Some(eb) = &mut series.y_error_bars
            {
                eb.dash_style = d_eb.dash_style;
                eb.stroke_width_pt = d_eb.stroke_width_pt;
                eb.cap_width_pt = d_eb.cap_width_pt;
            }
        }

        if let Some(mv) = &mut self.multivariate {
            let primary_swatch = self.palette.color_at(0);
            if clone_palette {
                match &mut mv.color {
                    ColorChannelMapping::Constant(color) => *color = primary_swatch,
                    ColorChannelMapping::Categorical { palette, .. } => *palette = self.palette,
                    ColorChannelMapping::Continuous { .. } => {}
                }
            }
            if let MultivariatePlotMode::CategoricalSummary { gdo, y_error_bars } = &mut mv.plot_mode {
                if clone_palette {
                    recolor_gdo(gdo, primary_swatch);
                }
                if let Some(d_eb) = donor_eb
                    && let Some(eb) = y_error_bars
                {
                    eb.dash_style = d_eb.dash_style;
                    eb.stroke_width_pt = d_eb.stroke_width_pt;
                    eb.cap_width_pt = d_eb.cap_width_pt;
                }
            }
        }
    }

    /// Return the first non-finite float field path in this graph specification, if any.
    #[must_use]
    pub fn first_non_finite_float(&self) -> Option<&'static str> {
        if !self.canvas_size_pt.width.is_finite() || !self.canvas_size_pt.height.is_finite() {
            return Some("spec.canvas_size_pt");
        }
        if let Some(field) = self.x_axis.first_non_finite_float() {
            return Some(field);
        }
        if let Some(field) = self.y_axis.first_non_finite_float() {
            return Some(field);
        }
        if let Some(field) = self.y2_axis.as_ref().and_then(AxisSpec::first_non_finite_float) {
            return Some(field);
        }
        for series in &self.series {
            if let Some(field) = gdo_non_finite(&series.gdo) {
                return Some(field);
            }
            if let Some(field) = series.y_error_bars.as_ref().and_then(error_bars_non_finite) {
                return Some(field);
            }
            if let Some(field) = series.x_error_bars.as_ref().and_then(error_bars_non_finite) {
                return Some(field);
            }
        }
        if let Some(mv) = &self.multivariate
            && let Some(field) = multivariate_non_finite(mv)
        {
            return Some(field);
        }
        for cmp in &self.comparisons {
            if !cmp.stroke_width_pt.is_finite() || !cmp.font_size_pt.is_finite() || !cmp.bracket_step_pt.is_finite() {
                return Some("spec.comparisons");
            }
        }
        if let Some(cld) = &self.compact_letter_display {
            if !cld.font_size_pt.is_finite() || !cld.offset_pt.is_finite() {
                return Some("spec.compact_letter_display");
            }
            if cld.alpha_override.is_some_and(|a| !a.is_finite()) {
                return Some("spec.compact_letter_display.alpha_override");
            }
        }
        for ann in &self.annotations {
            if let Some(field) = annotation_non_finite(ann) {
                return Some(field);
            }
        }
        for ov in &self.point_overrides {
            if ov.size_pt.is_some_and(|v| !v.is_finite()) || ov.border_width_pt.is_some_and(|v| !v.is_finite()) {
                return Some("spec.point_overrides");
            }
        }
        if self.legend.position_pt.is_some_and(|(x, y)| !x.is_finite() || !y.is_finite()) {
            return Some("spec.legend.position_pt");
        }
        None
    }

    /// Validate axis segment counts, title byte ceilings, and float finiteness.
    pub fn validate(&self) -> Result<(), DocError> {
        validate_title(&self.title)?;
        self.x_axis.validate()?;
        self.y_axis.validate()?;
        if let Some(y2) = &self.y2_axis {
            y2.validate()?;
        }
        if let Some(reason) = self.first_non_finite_float() {
            return Err(DocError::InvalidCellValue { archetype: crate::TableKind::Xy, row: 0, col: 0, reason });
        }
        if self.canvas_size_pt.width <= 0.0 || self.canvas_size_pt.height <= 0.0 {
            return Err(DocError::InvalidCellValue {
                archetype: crate::TableKind::Xy,
                row: 0,
                col: 0,
                reason: "spec.canvas_size_pt width and height must be > 0",
            });
        }
        for ann in &self.annotations {
            match ann {
                GraphAnnotation::ModelEquation { bounds_pt, font_size_pt, .. } => {
                    if bounds_pt.width < 0.0 || bounds_pt.height < 0.0 {
                        return Err(DocError::InvalidCellValue {
                            archetype: crate::TableKind::Xy,
                            row: 0,
                            col: 0,
                            reason: "spec.annotations.model_equation bounds_pt dimensions must be >= 0",
                        });
                    }
                    if *font_size_pt <= 0.0 || *font_size_pt > 144.0 {
                        return Err(DocError::InvalidCellValue {
                            archetype: crate::TableKind::Xy,
                            row: 0,
                            col: 0,
                            reason: "spec.annotations.model_equation font_size_pt must be in (0, 144]",
                        });
                    }
                }
                GraphAnnotation::TextBox { bounds_pt, .. } | GraphAnnotation::EmbeddedResultTable { bounds_pt, .. } => {
                    if bounds_pt.width < 0.0 || bounds_pt.height < 0.0 {
                        return Err(DocError::InvalidCellValue {
                            archetype: crate::TableKind::Xy,
                            row: 0,
                            col: 0,
                            reason: "spec.annotations.bounds_pt dimensions must be >= 0",
                        });
                    }
                }
                GraphAnnotation::Shape { bounds_pt, stroke_width_pt, .. } => {
                    if bounds_pt.width < 0.0 || bounds_pt.height < 0.0 {
                        return Err(DocError::InvalidCellValue {
                            archetype: crate::TableKind::Xy,
                            row: 0,
                            col: 0,
                            reason: "spec.annotations.shape bounds_pt dimensions must be >= 0",
                        });
                    }
                    if *stroke_width_pt < 0.0 {
                        return Err(DocError::InvalidCellValue {
                            archetype: crate::TableKind::Xy,
                            row: 0,
                            col: 0,
                            reason: "spec.annotations.shape stroke_width_pt must be >= 0",
                        });
                    }
                }
                GraphAnnotation::LineOrArrow { .. } => {}
            }
        }
        for ov in &self.point_overrides {
            if ov.size_pt.is_some_and(|v| v <= 0.0) {
                return Err(DocError::InvalidCellValue {
                    archetype: crate::TableKind::Xy,
                    row: 0,
                    col: 0,
                    reason: "spec.point_overrides.size_pt must be finite and > 0",
                });
            }
            if ov.border_width_pt.is_some_and(|v| v < 0.0) {
                return Err(DocError::InvalidCellValue {
                    archetype: crate::TableKind::Xy,
                    row: 0,
                    col: 0,
                    reason: "spec.point_overrides.border_width_pt must be finite and >= 0",
                });
            }
        }
        if let Some(cld) = &self.compact_letter_display {
            if cld.font_size_pt <= 0.0 || cld.font_size_pt > 144.0 {
                return Err(DocError::InvalidCellValue {
                    archetype: crate::TableKind::Xy,
                    row: 0,
                    col: 0,
                    reason: "spec.compact_letter_display.font_size_pt must be in (0, 144]",
                });
            }
            if cld.offset_pt < 0.0 || cld.offset_pt > 200.0 {
                return Err(DocError::InvalidCellValue {
                    archetype: crate::TableKind::Xy,
                    row: 0,
                    col: 0,
                    reason: "spec.compact_letter_display.offset_pt must be in [0, 200]",
                });
            }
            if cld.alpha_override.is_some_and(|a| a <= 0.0 || a >= 1.0) {
                return Err(DocError::InvalidCellValue {
                    archetype: crate::TableKind::Xy,
                    row: 0,
                    col: 0,
                    reason: "spec.compact_letter_display.alpha_override must be in (0, 1)",
                });
            }
        }
        Ok(())
    }

    /// Collect all inline template hooks (`{{info:...}}`, `{{result:...}}`) referenced in titles and text annotations.
    #[must_use]
    pub fn inline_hooks(&self) -> Vec<HookSource> {
        let mut hooks = extract_template_hooks(&self.title);
        hooks.extend(extract_template_hooks(&self.x_axis.title));
        hooks.extend(extract_template_hooks(&self.y_axis.title));
        if let Some(y2) = &self.y2_axis {
            hooks.extend(extract_template_hooks(&y2.title));
        }
        for ann in &self.annotations {
            if let GraphAnnotation::TextBox { markdown, .. } = ann {
                hooks.extend(extract_template_hooks(markdown));
            }
        }
        hooks
    }

    /// Update any multivariate `x_level_order_override` or categorical colour `level_order_override`
    /// bound to `(source_sheet_id, col_idx)` when categorical levels are recoded.
    pub fn recode_variable_levels(&mut self, source_sheet_id: &SheetId, col_idx: usize, mappings: &[(String, String)]) {
        if self.primary_source_sheet_id != *source_sheet_id || mappings.is_empty() {
            return;
        }
        let Some(enc) = &mut self.multivariate else {
            return;
        };
        if enc.x_column == col_idx
            && let Some(ov) = &mut enc.x_level_order_override
        {
            ov.recode_levels(mappings);
        }
        if let ColorChannelMapping::Categorical { column, level_order_override: Some(ov), .. } = &mut enc.color
            && *column == col_idx
        {
            ov.recode_levels(mappings);
        }
    }
}

fn default_marker_shape(index: usize) -> MarkerShape {
    const SHAPES: [MarkerShape; 6] =
        [MarkerShape::Circle, MarkerShape::Square, MarkerShape::TriangleUp, MarkerShape::TriangleDown, MarkerShape::Diamond, MarkerShape::Hexagon];
    SHAPES.get(index % SHAPES.len()).copied().unwrap_or(MarkerShape::Circle)
}

fn default_gdo_for_archetype(archetype: &TableArchetype, color: Rgba8, marker: MarkerStyle) -> GraphicalDataObject {
    match archetype {
        TableArchetype::Xy(_) | TableArchetype::MultipleVariables(_) => {
            GraphicalDataObject::ScatterAndLine { marker: Some(marker), line: None, area_fill: None }
        }
        TableArchetype::Column(col) if col.design_hint == ColumnDesignHint::PairedRepeatedMeasures => {
            GraphicalDataObject::BeforeAfterLines { marker: Some(marker), line_color: color, stroke_width_pt: 1.2 }
        }
        TableArchetype::Column(_) | TableArchetype::Grouped(_) | TableArchetype::Contingency(_) | TableArchetype::Nested(_) => {
            GraphicalDataObject::BarWithReplicates {
                summary: SummaryMeasure::Mean,
                fill_color: color.with_alpha(210),
                border_color: color,
                border_width_pt: 1.0,
                replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
            }
        }
        TableArchetype::Survival(_) => GraphicalDataObject::KaplanMeierStep {
            line_color: color,
            stroke_width_pt: 1.5,
            show_censored_ticks: true,
            show_greenwood_ci: false,
            ci_dashed: true,
        },
        TableArchetype::PartsOfWhole(_) => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Donut, show_center_total: true },
    }
}

fn recolor_gdo(gdo: &mut GraphicalDataObject, swatch: Rgba8) {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                m.fill_color = swatch;
                m.stroke_color = swatch;
            }
            if let Some(l) = line {
                l.color = swatch;
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => {
            *fill_color = swatch.with_alpha(fill_color.a);
            *border_color = swatch;
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.fill_color = swatch;
            marker.stroke_color = swatch;
        }
        GraphicalDataObject::BoxAndWhisker { fill_color, stroke_color, .. } | GraphicalDataObject::Violin { fill_color, stroke_color, .. } => {
            *fill_color = swatch.with_alpha(fill_color.a);
            *stroke_color = swatch;
        }
        GraphicalDataObject::BeforeAfterLines { marker, line_color, .. } => {
            *line_color = swatch;
            if let Some(m) = marker {
                m.fill_color = swatch;
                m.stroke_color = swatch;
            }
        }
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => {
            *line_color = swatch;
        }
        GraphicalDataObject::PartsOfWhole { .. } => {}
    }
}

fn rect_is_finite(rect: RectPt) -> bool {
    rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite()
}

fn marker_non_finite(marker: &MarkerStyle) -> Option<&'static str> {
    if !marker.size_pt.is_finite() || !marker.stroke_width_pt.is_finite() {
        return Some("spec.series.marker");
    }
    None
}

fn error_bars_non_finite(bars: &ErrorBarSpec) -> Option<&'static str> {
    if !bars.stroke_width_pt.is_finite() || !bars.cap_width_pt.is_finite() {
        return Some("spec.series.error_bars");
    }
    None
}

fn gdo_non_finite(gdo: &GraphicalDataObject) -> Option<&'static str> {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill } => {
            if let Some(m) = marker
                && let Some(field) = marker_non_finite(m)
            {
                return Some(field);
            }
            if let Some(l) = line
                && !l.stroke_width_pt.is_finite()
            {
                return Some("spec.series.line.stroke_width_pt");
            }
            if let Some(area) = area_fill
                && !area.baseline_y.is_finite()
            {
                return Some("spec.series.area_fill.baseline_y");
            }
            None
        }
        GraphicalDataObject::BarWithReplicates { border_width_pt, .. } | GraphicalDataObject::FloatingBar { border_width_pt, .. } => {
            (!border_width_pt.is_finite()).then_some("spec.series.border_width_pt")
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker_non_finite(marker),
        GraphicalDataObject::BeforeAfterLines { marker, stroke_width_pt, .. } => {
            if let Some(m) = marker
                && let Some(field) = marker_non_finite(m)
            {
                return Some(field);
            }
            (!stroke_width_pt.is_finite()).then_some("spec.series.stroke_width_pt")
        }
        GraphicalDataObject::KaplanMeierStep { stroke_width_pt, .. } => (!stroke_width_pt.is_finite()).then_some("spec.series.stroke_width_pt"),
        GraphicalDataObject::BoxAndWhisker { .. } | GraphicalDataObject::Violin { .. } | GraphicalDataObject::PartsOfWhole { .. } => None,
    }
}

fn multivariate_non_finite(mv: &MultivariateGraphEncoding) -> Option<&'static str> {
    if let ColorChannelMapping::Continuous { ramp, domain_clamp, .. } = &mv.color {
        if domain_clamp.is_some_and(|(lo, hi)| !lo.is_finite() || !hi.is_finite()) {
            return Some("spec.multivariate.color.domain_clamp");
        }
        if let ColorRamp::Custom { stops } = ramp
            && stops.iter().any(|s| !s.position.is_finite())
        {
            return Some("spec.multivariate.color.ramp.stops");
        }
    }
    match mv.size {
        SizeChannelMapping::Constant { radius_pt } => {
            if !radius_pt.is_finite() {
                return Some("spec.multivariate.size.radius_pt");
            }
        }
        SizeChannelMapping::Variable { min_radius_pt, max_radius_pt, .. } => {
            if !min_radius_pt.is_finite() || !max_radius_pt.is_finite() {
                return Some("spec.multivariate.size.radius_range");
            }
        }
    }
    match &mv.plot_mode {
        MultivariatePlotMode::BubbleScatter => None,
        MultivariatePlotMode::CategoricalSummary { gdo, y_error_bars } => {
            gdo_non_finite(gdo).or_else(|| y_error_bars.as_ref().and_then(error_bars_non_finite))
        }
    }
}

fn annotation_non_finite(ann: &GraphAnnotation) -> Option<&'static str> {
    match ann {
        GraphAnnotation::TextBox { bounds_pt, .. } | GraphAnnotation::EmbeddedResultTable { bounds_pt, .. } => {
            (!rect_is_finite(*bounds_pt)).then_some("spec.annotations.bounds_pt")
        }
        GraphAnnotation::LineOrArrow { start_pt, end_pt, .. } => {
            (!start_pt.0.is_finite() || !start_pt.1.is_finite() || !end_pt.0.is_finite() || !end_pt.1.is_finite())
                .then_some("spec.annotations.line_endpoints")
        }
        GraphAnnotation::Shape { bounds_pt, stroke_width_pt, .. } => {
            if !rect_is_finite(*bounds_pt) || !stroke_width_pt.is_finite() {
                return Some("spec.annotations.shape");
            }
            None
        }
        GraphAnnotation::ModelEquation { bounds_pt, font_size_pt, .. } => {
            if !rect_is_finite(*bounds_pt) || !font_size_pt.is_finite() {
                return Some("spec.annotations.model_equation");
            }
            None
        }
    }
}

/// Navigator sheet wrapping a [`GraphSpec`] and tracking first-visit gallery initialization.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraphSheet {
    pub id: SheetId,
    pub title: String,
    /// Starts `false` when auto-provisioned alongside a new `DataSheet` so the first visit presents
    /// the visual family schematic chooser (`docs/graphing-and-layouts.md` §2).
    pub initialized: bool,
    pub spec: GraphSpec,
    pub floating_notes: Vec<FloatingNote>,
    pub color_tag: Option<Rgba8>,
}

impl GraphSheet {
    /// Return the first non-finite float field path in this graph sheet, if any.
    #[must_use]
    pub fn first_non_finite_float(&self) -> Option<&'static str> {
        if let Some(field) = self.spec.first_non_finite_float() {
            return Some(field);
        }
        if self.floating_notes.iter().any(|note| !rect_is_finite(note.bounds_pt)) {
            return Some("floating_notes.bounds_pt");
        }
        None
    }

    /// Validate sheet identity, title, graph specification, and floating-note geometry.
    pub fn validate(&self) -> Result<(), DocError> {
        if self.id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        validate_title(&self.title)?;
        self.spec.validate()?;
        if self.floating_notes.iter().any(|note| !rect_is_finite(note.bounds_pt)) {
            return Err(DocError::InvalidCellValue { archetype: crate::TableKind::Xy, row: 0, col: 0, reason: "floating_notes.bounds_pt" });
        }
        Ok(())
    }
}

impl GraphSheet {
    pub fn new_uninitialized_for_data(id: impl Into<SheetId>, data_sheet: &DataSheet) -> Result<Self, DocError> {
        let id = id.into();
        if id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        Ok(Self {
            id,
            title: data_sheet.title.clone(),
            initialized: false,
            spec: GraphSpec::default_for_data_sheet(data_sheet),
            floating_notes: Vec::new(),
            color_tag: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{GraphSpec, GroupedArrangement, PointStyleOverride};
    use crate::{ColumnDesignHint, ColumnTable, DataSheet, SheetId, TableArchetype};
    use curvebook_palette::{MarkerFillMode, MarkerShape, Rgba8};

    #[test]
    fn graph_spec_point_overrides_round_trip_and_backward_compat() {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 1).expect("column table");
        let ds = DataSheet::new(SheetId::new("col_1"), "Dose Response", TableArchetype::Column(col)).expect("data sheet");
        let mut spec = GraphSpec::default_for_data_sheet(&ds);
        spec.grouped_arrangement = GroupedArrangement::Superimposed;
        spec.point_overrides.push(PointStyleOverride {
            series_index: 0,
            point_index: 2,
            color: Some(Rgba8::rgb(213, 94, 0)),
            symbol: Some(MarkerShape::Diamond),
            size_pt: Some(6.5),
            border_width_pt: Some(1.5),
            fill_color: Some(Rgba8::rgb(255, 255, 255)),
            fill_mode: Some(MarkerFillMode::Open),
            hidden: false,
            ..PointStyleOverride::default()
        });
        spec.point_overrides.push(PointStyleOverride { series_index: 1, point_index: 0, hidden: true, ..PointStyleOverride::default() });

        let serialized = serde_json::to_value(&spec).expect("serialize GraphSpec");
        let decoded: GraphSpec = serde_json::from_value(serialized.clone()).expect("deserialize GraphSpec");
        assert_eq!(decoded, spec);
        assert_eq!(decoded.point_override(0, 2).and_then(|ov| ov.symbol), Some(MarkerShape::Diamond));
        assert_eq!(decoded.point_override(1, 0).map(|ov| ov.hidden), Some(true));

        let mut legacy = serialized.as_object().cloned().expect("object JSON");
        legacy.remove("point_overrides");
        let legacy_decoded: GraphSpec = serde_json::from_value(serde_json::Value::Object(legacy)).expect("deserialize legacy GraphSpec");
        assert!(legacy_decoded.point_overrides.is_empty());
        assert_eq!(legacy_decoded.grouped_arrangement, GroupedArrangement::Superimposed);
    }
}
