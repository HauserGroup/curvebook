//! Declarative `GraphSpec` and `GraphSheet` schemas: graphical data objects (GDOs), nonlinear and
//! discontinuous 1–3 segment axes, dashed/solid error bars, five-channel `MultipleVariables`
//! bubble/categorical encodings, live pairwise significance brackets, and style cloning
//! (`docs/implementation/graphing-and-layouts.md` §1–§5).

pub use curvebook_math::{AxisScale, LogitDomain};
use curvebook_palette::{ColorRamp, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};

use crate::archetypes::{ColumnDesignHint, LevelOrderSpec, TableArchetype, VariableRole};
use crate::error::{DocError, validate_title};
use crate::ids::{FloatingNote, HookSource, RectPt, SheetId, SizePt, extract_template_hooks};
use crate::image::{CropInsetsPt, ImageBorderSpec};
use crate::links::DataSheet;

/// Specialized diagnostic plot subtypes (`docs/implementation/graphing-and-layouts.md` §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DiagnosticPlotKind {
    /// Color-mapped 2D matrix of correlation coefficients, expression values, or grouped means.
    Heatmap,
    /// Differential effect (`log2` fold change or difference on *X*) vs `-log10(p)` or `-log10(q)` significance on *Y*.
    Volcano,
    /// Method-comparison difference (or ratio) vs average plot with bias and 95% limits of agreement (Bland & Altman 1986).
    BlandAltman,
    /// Sensitivity (`TPR`) vs `1 - Specificity` (`FPR`) receiver operating characteristic curve with diagonal chance line.
    RocCurve,
    /// Horizontal confidence-interval effect-size / odds-ratio / hazard-ratio plot with null reference line.
    ForestPlot,
    /// Regression residuals vs predicted *Y* or *X* diagnostic scatter plot.
    ResidualPlot,
    /// Theoretical normal quantiles vs observed/residual quantiles Q–Q plot with reference line.
    NormalQqPlot,
}

/// Primary visual family of a [`GraphSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GraphKind {
    /// Continuous *X* vs *Y* scatter, connected line, spline, area, and fitted curve figure.
    Xy,
    /// One-way categorical group comparison figure (bars, box-and-whiskers, violins, dot plots, before–after lines).
    Column,
    /// Two-way or three-way clustered/stacked/superimposed group figure.
    Grouped,
    /// Categorical count/proportion bar or stacked bar figure.
    Contingency,
    /// Pie, donut, stacked fraction bar, or `10 × 10` waffle chart.
    PartsOfWhole,
    /// Kaplan–Meier staircase survival curve with censoring ticks, optional Greenwood CIs, and optional number-at-risk table.
    Survival,
    /// Five-channel multivariate bubble scatter or categorical × continuous summary figure.
    MultipleVariables,
    /// Specialized statistical or regression diagnostic plot ([`DiagnosticPlotKind`]).
    Diagnostic(DiagnosticPlotKind),
}

/// Axis frame geometry preset (`docs/implementation/graphing-and-layouts.md` §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum FramePreset {
    /// Classic open L-shaped left *Y* and bottom *X* axes meeting at the bottom-left corner.
    #[default]
    OpenL,
    /// Tufte range-frame style offsetting left *Y* and bottom *X* spines slightly away from the corner and trimming to data range.
    OffsetRangeTufte,
    /// Four-sided enclosed rectangular frame (left, bottom, top, and right spines).
    PlainBoxed,
    /// Left primary *Y* axis paired with an independent right secondary `y2_axis` spine.
    DualYSecondaryRight,
    /// Axes intersecting directly at data coordinate `(0, 0)`.
    OriginCrossing,
}

/// Guarded axis scale bijection (`docs/implementation/graphing-and-layouts.md` §3.1), unified with [`curvebook_math::AxisScale`].
pub type GraphAxisScale = AxisScale;

/// Break-mark ornament between discontinuous axis segments (`docs/implementation/graphing-and-layouts.md` §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum BreakOrnament {
    /// Parallel slanted slash ticks (`//`) straddling the segment gap.
    #[default]
    DoubleSlash,
    /// Sawtooth zigzag wave bridging the segment gap.
    Zigzag,
    /// Clean whitespace gap with no drawn ornament.
    BlankGap,
}

/// Source for categorical tick labels on `Column` and `Grouped` X axes (`docs/implementation/graphing-and-layouts.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum CategoricalTickSource {
    /// Read category tick labels from the source table's `row_titles`.
    #[default]
    RowTitles,
    /// Read category tick labels from the source table's data-set / group `column` titles.
    ColumnTitles,
    /// Display 1-based row numbers (`"1"`, `"2"`, `"3"`, ...).
    RowNumbers,
}

/// Numeric or categorical tick formatting mode (`docs/implementation/graphing-and-layouts.md` §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum TickLabelFormat {
    /// Standard fixed/trimmed decimal numerals (`0.1`, `1`, `10`, `100`).
    #[default]
    Decimal,
    /// Base-10 superscript powers (`10⁻⁶`, `10⁻³`, `10⁰`, `10³`).
    PowerOfTenSuperscript,
    /// Exponential scientific notation (`1e-6`, `1e-3`, `1e3`).
    ScientificE,
    /// Antilogarithm `10^x` display for pre-logged *X* concentrations (for example `-6` labeled as `1e-6` or `0.000001`).
    Antilog,
    /// SI metric prefix scaling (`p`, `n`, `µ`, `m`, `k`, `M`, `G`).
    EngineeringSi,
    /// Unsigned decimal magnitude (`|x|`) for symmetric diverging axes such as population pyramids.
    AbsoluteDecimal,
    /// Discrete text category labels sourced via [`CategoricalTickSource`].
    Categorical(CategoricalTickSource),
}

/// Direction in which major and minor axis tick marks protrude from the axis spine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum TickDirection {
    /// Ticks protrude outward from the plot frame (default).
    #[default]
    Outside,
    /// Ticks protrude inward into the plot frame.
    Inside,
    /// Ticks straddle the axis spine on both inside and outside edges.
    Both,
    /// Suppress major and minor tick marks completely.
    None,
}

/// Orientation of a cartesian column or grouped graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum CartesianOrientation {
    /// Vertical bars/plots: categories along horizontal *X* and values along vertical *Y* (default).
    #[default]
    Vertical,
    /// Horizontal bars/plots: categories along vertical *Y* and values along horizontal *X* (for example population pyramids).
    Horizontal,
}

/// Source for row labels rendered along the left edge of a heatmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum HeatmapRowLabelMode {
    /// Display row titles from the source table or clustering labels (default).
    #[default]
    RowTitles,
    /// Display 1-based row indices (`"1"`, `"2"`, ...).
    RowNumbers,
    /// Omit row labels.
    None,
}

/// Source for column labels rendered along the bottom edge of a heatmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum HeatmapColumnLabelMode {
    /// Display data-set / variable titles from the source table (default).
    #[default]
    ColumnTitles,
    /// Display spreadsheet column letters (`"A"`, `"B"`, ...).
    ColumnLetters,
    /// Omit column labels.
    None,
}

/// Rotation angle preset for heatmap column header labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum HeatmapColumnLabelAngle {
    /// Horizontal (`0°`) labels below columns (default).
    #[default]
    Horizontal0,
    /// Angled (`45°`) diagonal labels below columns.
    Angled45,
    /// Vertical (`90°`) rotated labels below columns with dynamic margin reservation.
    Vertical90,
}

/// Row/column label display and rotation configuration for diagnostic heatmaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct HeatmapLabelSpec {
    /// Row label text mode along the vertical axis.
    #[serde(default)]
    pub row_labels: HeatmapRowLabelMode,
    /// Column label text mode along the horizontal axis.
    #[serde(default)]
    pub column_labels: HeatmapColumnLabelMode,
    /// Column label rotation preset.
    #[serde(default)]
    pub column_angle: HeatmapColumnLabelAngle,
}

/// Display metric plotted by [`GraphicalDataObject::KaplanMeierStep`] curves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SurvivalDisplayMetric {
    /// Descending survival percentage (`100%` down toward `0%`; default).
    #[default]
    PercentSurvival,
    /// Ascending cumulative failure/event percentage (`0%` up toward `100%`, equal to `100 - %Survival`).
    PercentCumulativeEvent,
    /// Descending survival proportion (`1.0` down toward `0.0`).
    FractionSurvival,
}

/// Connecting-line behavior for multi-replicate XY series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ReplicateLineMode {
    /// Connect row summary centers (`Mean` or `Median`) with one polyline per series (default).
    #[default]
    SummaryCenter,
    /// Trace an independent longitudinal polyline for each replicate subcolumn across rows (spaghetti plot).
    IndividualReplicateTrajectories,
}

/// Per-series coordinate nudge applied in data units and/or post-projection typographic points.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SeriesOffsetSpec {
    /// Additive horizontal shift in *X* data-axis units before axis mapping (for example `+0.5`).
    #[serde(default)]
    pub x_data: f64,
    /// Additive horizontal shift in typographic points (`pt`) after axis mapping.
    #[serde(default)]
    pub x_pt: f64,
    /// Additive vertical shift in typographic points (`pt`, positive shifts upward) after axis mapping.
    #[serde(default)]
    pub y_pt: f64,
}

const fn default_true() -> bool {
    true
}

const fn default_embedded_table_font_pt() -> f64 {
    7.5
}

const fn default_embedded_table_border_pt() -> f64 {
    0.75
}

const fn default_embedded_table_grid_pt() -> f64 {
    0.5
}

const fn default_embedded_table_border_color() -> Rgba8 {
    Rgba8::BLACK
}

const fn default_embedded_table_fill_color() -> Option<Rgba8> {
    Some(Rgba8::WHITE)
}

/// Visual formatting options for embedded live result/data tables on graphs and layouts.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EmbeddedTableStyle {
    /// When `true`, renders the column header row (default `true`).
    #[serde(default = "default_true")]
    pub show_column_titles: bool,
    /// When `true`, renders row title labels in the leading column (default `true`).
    #[serde(default = "default_true")]
    pub show_row_titles: bool,
    /// When `true`, transposes rows and columns when rendering (default `false`).
    #[serde(default)]
    pub transpose: bool,
    /// When `true`, styles column and row headers in bold weight (default `true`).
    #[serde(default = "default_true")]
    pub bold_headers: bool,
    /// Cell text font size in typographic points (`> 0.0`, finite; default `7.5`).
    #[serde(default = "default_embedded_table_font_pt")]
    pub font_size_pt: f64,
    /// Outer border stroke width in typographic points (`>= 0.0`, `0.0` hides border; default `0.75`).
    #[serde(default = "default_embedded_table_border_pt")]
    pub outer_border_width_pt: f64,
    /// Interior gridline stroke width in typographic points (`>= 0.0`, `0.0` hides gridlines; default `0.5`).
    #[serde(default = "default_embedded_table_grid_pt")]
    pub gridline_width_pt: f64,
    /// Stroke color for outer border and interior gridlines (default black).
    #[serde(default = "default_embedded_table_border_color")]
    pub border_color: Rgba8,
    /// Optional solid cell-table background fill color (`None` = transparent; default opaque white).
    #[serde(default = "default_embedded_table_fill_color")]
    pub fill_color: Option<Rgba8>,
}

impl Default for EmbeddedTableStyle {
    fn default() -> Self {
        Self {
            show_column_titles: true,
            show_row_titles: true,
            transpose: false,
            bold_headers: true,
            font_size_pt: default_embedded_table_font_pt(),
            outer_border_width_pt: default_embedded_table_border_pt(),
            gridline_width_pt: default_embedded_table_grid_pt(),
            border_color: default_embedded_table_border_color(),
            fill_color: default_embedded_table_fill_color(),
        }
    }
}

impl EmbeddedTableStyle {
    /// Return the first non-finite field path inside this embedded table style, if any.
    #[must_use]
    pub fn first_non_finite_float(&self, prefix: &'static str) -> Option<&'static str> {
        if !self.font_size_pt.is_finite() || !self.outer_border_width_pt.is_finite() || !self.gridline_width_pt.is_finite() {
            return Some(prefix);
        }
        None
    }

    /// Validate positive font size and non-negative border/gridline widths.
    pub fn validate(&self, archetype: crate::TableKind) -> Result<(), DocError> {
        if !self.font_size_pt.is_finite() || self.font_size_pt <= 0.0 {
            return Err(DocError::InvalidCellValue { archetype, row: 0, col: 0, reason: "embedded_table style.font_size_pt must be finite and > 0" });
        }
        if !self.outer_border_width_pt.is_finite() || self.outer_border_width_pt < 0.0 {
            return Err(DocError::InvalidCellValue {
                archetype,
                row: 0,
                col: 0,
                reason: "embedded_table style.outer_border_width_pt must be finite and >= 0",
            });
        }
        if !self.gridline_width_pt.is_finite() || self.gridline_width_pt < 0.0 {
            return Err(DocError::InvalidCellValue {
                archetype,
                row: 0,
                col: 0,
                reason: "embedded_table style.gridline_width_pt must be finite and >= 0",
            });
        }
        Ok(())
    }
}

/// Reference threshold rule or shaded band drawn across a plot area (e.g. Volcano alpha/fold-change cutoffs).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AxisThreshold {
    /// Primary threshold coordinate in data-space units on the owning axis (finite).
    pub value: f64,
    /// Optional second coordinate in data-space units filling a translucent band between `value` and `shade_to`; `None` draws a single line only.
    pub shade_to: Option<f64>,
    /// Stroke color (and base tint for shaded bands).
    pub color: Rgba8,
    /// Solid, dashed, or dotted line pattern for the threshold rule.
    pub dash_style: LineDashStyle,
    /// Rule stroke width in typographic points (`>= 0.0`, finite).
    pub stroke_width_pt: f64,
    /// Optional text callout rendered adjacent to the threshold line; `None` omits text.
    pub label: Option<String>,
}

/// Custom extra tick mark (with optional label override and gridline) placed on an axis.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct AdditionalTick {
    /// Coordinate in data-space units where the custom tick is placed (finite).
    pub value: f64,
    /// Custom tick text overriding the axis's [`TickLabelFormat`]; `None` formats `value` automatically.
    #[serde(default)]
    pub label: Option<String>,
    /// When `true`, extends a reference gridline perpendicular to the axis across the plot area.
    #[serde(default)]
    pub draw_gridline: bool,
}

/// One continuous segment (`1..=3` per axis) of a standard or discontinuous axis (`docs/implementation/graphing-and-layouts.md` §3.2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AxisSegment {
    /// Explicit lower bound in data-space units; `None` selects an automatic data-driven lower bound.
    pub min: Option<f64>,
    /// Explicit upper bound in data-space units; `None` selects an automatic data-driven upper bound.
    pub max: Option<f64>,
    /// Coordinate scale bijection applied across this segment (`Linear`, `Log10`, `Log2`, `Ln`, `Logit`, `Probit`, or `Reciprocal`).
    pub scale: GraphAxisScale,
    /// Relative physical length weight on the rendered axis (`> 0.0`).
    pub length_share: f64,
    /// Explicit major tick spacing (in data units for `Linear`, or log-decade step where applicable); `None` chooses nice ticks automatically.
    pub major_tick_interval: Option<f64>,
    /// Number of minor tick subdivisions drawn between consecutive major ticks (`0` disables minor ticks).
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
    /// Axis title rendered alongside the spine (supports inline `{{info:...}}` and `{{result:...}}` hooks).
    pub title: String,
    /// Ordered `1..=3` continuous segments composing a standard (`len == 1`) or broken (`len in 2..=3`) axis.
    pub segments: Vec<AxisSegment>,
    /// Physical gap width in typographic points inserted between adjacent discontinuous segments.
    pub gap_width_pt: f64,
    /// Visual break-mark symbol drawn at segment boundaries when `segments.len() >= 2`.
    pub break_ornament: BreakOrnament,
    /// Numeric or categorical formatting applied to major tick labels.
    pub tick_format: TickLabelFormat,
    /// Rotation angle of tick labels in degrees (`0.0` = horizontal, `-45.0` / `45.0` = angled, `90.0` = vertical).
    pub tick_angle_deg: f64,
    /// Spine and tick-mark stroke width in typographic points (`>= 0.0`, finite).
    pub stroke_width_pt: f64,
    /// Reference threshold lines and shaded target zones anchored to this axis.
    pub thresholds: Vec<AxisThreshold>,
    /// Extra user-pinned tick marks and optional gridlines.
    #[serde(default)]
    pub additional_ticks: Vec<AdditionalTick>,
    /// Direction in which major and minor tick marks protrude from the axis spine.
    #[serde(default)]
    pub tick_direction: TickDirection,
    /// When `true`, renders numeric or categorical tick labels alongside tick marks (default `true`).
    #[serde(default = "default_true")]
    pub show_tick_labels: bool,
}

impl AxisSpec {
    /// Construct a single-segment linear `AxisSpec` with default decimal ticks and `1.0 pt` stroke width.
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
            tick_direction: TickDirection::Outside,
            show_tick_labels: true,
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

    /// Enforce that discontinuous axes carry `1`, `2`, or `3` segments (`docs/implementation/graphing-and-layouts.md` §3.2)
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
    /// Sample arithmetic mean.
    #[default]
    Mean,
    /// Sample 50th percentile (median).
    Median,
    /// Geometric mean `exp(mean(ln(x)))` over strictly positive observations (`x > 0`).
    GeometricMean,
}

/// Whisker extent convention for box-and-whisker plots (`docs/implementation/graphing-and-layouts.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum WhiskerMethod {
    /// Tukey (1977) fences extending to the most extreme observations within `[Q1 - 1.5 * IQR, Q3 + 1.5 * IQR]`.
    #[default]
    Tukey1_5Iqr,
    /// Full sample range `[min, max]`.
    MinToMax,
    /// Sample 10th to 90th percentiles.
    Percentile10To90,
    /// Sample 5th to 95th percentiles.
    Percentile5To95,
    /// Sample 2.5th to 97.5th percentiles.
    Percentile2_5To97_5,
    /// Sample 1st to 99th percentiles.
    Percentile1To99,
}

/// Deterministic replicate point overlay on bars, boxes, and violins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ReplicateOverlayStyle {
    /// Symmetric collision-avoiding horizontal beeswarm spread (deterministic; zero random jitter).
    #[default]
    DeterministicBeeswarm,
    /// Vertically aligned column centered on the category's horizontal anchor.
    AlignedColumn,
}

/// Curve interpolation rule for XY connecting lines (`docs/implementation/graphing-and-layouts.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum LineConnectionKind {
    /// Piecewise linear segments connecting consecutive points.
    #[default]
    PointToPoint,
    /// Horizontal-then-vertical right-continuous step function.
    Staircase,
    /// Shape-preserving Fritsch–Carlson (1980) / Steffen (1990) monotonic piecewise cubic Hermite spline (prevents overshoot between monotone points).
    MonotonicCubicSpline,
    /// Akima (1970) local five-point cubic spline resistant to oscillations near outlier slopes.
    AkimaSpline,
}

/// Visual stroke and interpolation style for series connecting lines.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConnectingLineStyle {
    /// Polyline, step, or spline curve geometry.
    pub kind: LineConnectionKind,
    /// Solid, dashed, or dotted stroke pattern.
    pub dash_style: LineDashStyle,
    /// Line stroke width in typographic points (`>= 0.0`, finite).
    pub stroke_width_pt: f64,
    /// Line stroke sRGB color.
    pub color: Rgba8,
}

/// Shaded polygon fill between a plotted XY series and a horizontal baseline.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AreaFillSpec {
    /// Horizontal reference *Y* coordinate in data-space units anchoring the area polygon (commonly `0.0`).
    pub baseline_y: f64,
    /// Fill color (typically carrying translucent alpha).
    pub fill_color: Rgba8,
}

/// Visual geometry for [`GraphicalDataObject::PartsOfWhole`] figures.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum PartsOfWholeLayout {
    /// Circular pie chart with radial wedges proportional to slice fractions.
    Pie,
    /// Annular ring chart leaving a central cutout (with optional total readout).
    #[default]
    Donut,
    /// Single `100%` stacked rectangular bar partitioned into proportional segments.
    StackedFractionBar,
    /// `10 × 10` unit-cell grid (`100` squares) colored by rounded percentage share.
    Waffle10x10,
}

/// Composable Graphical Data Object (GDO) describing how a series renders (`docs/implementation/graphing-and-layouts.md` §2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GraphicalDataObject {
    /// XY scatter symbols, optional connecting curve/spline, and optional shaded area-under-curve fill.
    ScatterAndLine {
        /// Optional point symbol style; `None` hides individual point markers (line-only).
        marker: Option<MarkerStyle>,
        /// Optional connecting line or spline style; `None` draws unconnected scatter points.
        line: Option<ConnectingLineStyle>,
        /// Optional polygon shading to a horizontal baseline; `None` disables area fill.
        area_fill: Option<AreaFillSpec>,
        /// Summary-center connecting polyline vs per-replicate longitudinal spaghetti trajectories.
        #[serde(default)]
        replicate_lines: ReplicateLineMode,
    },
    /// Summary column/grouped bar anchored at zero (or axis baseline) with optional overlaid individual replicate points.
    BarWithReplicates {
        /// Central tendency statistic (`Mean`, `Median`, or `GeometricMean`) setting the bar height.
        summary: SummaryMeasure,
        /// Interior bar fill color.
        fill_color: Rgba8,
        /// Bar outline stroke color.
        border_color: Rgba8,
        /// Bar outline stroke width in typographic points (`>= 0.0`, finite).
        border_width_pt: f64,
        /// Optional individual replicate point overlay (`DeterministicBeeswarm` or `AlignedColumn`); `None` draws summary bars only.
        replicate_overlay: Option<ReplicateOverlayStyle>,
    },
    /// Individual-value dot plot (beeswarm/column) with optional horizontal summary crossbar.
    AlignedDotPlot {
        /// Summary statistic used when `show_crossbar == true`.
        summary: SummaryMeasure,
        /// Marker symbol, size, and colors for individual observations.
        marker: MarkerStyle,
        /// When `true`, draws a horizontal summary crossbar across the category column.
        show_crossbar: bool,
    },
    /// Quartile box (`Q1` to `Q3` with median line) and configurable whiskers.
    BoxAndWhisker {
        /// Rule determining lower and upper whisker endpoints.
        whiskers: WhiskerMethod,
        /// Box interior fill color.
        fill_color: Rgba8,
        /// Box, median line, and whisker stroke color.
        stroke_color: Rgba8,
        /// When `true`, plots individual observations lying outside the whisker endpoints.
        show_outliers: bool,
        /// When `true`, overlays all replicate observations across the box.
        show_all_points: bool,
    },
    /// Kernel density violin plot with optional quartile lines and half-violin point split.
    Violin {
        /// Violin density interior fill color.
        fill_color: Rgba8,
        /// Violin contour stroke color.
        stroke_color: Rgba8,
        /// When `true`, truncates kernel density tails strictly at observed `[min, max]`.
        clip_to_observed_range: bool,
        /// When `true`, draws internal median and `Q1`/`Q3` quartile rules inside the violin.
        show_median_and_quartiles: bool,
        /// When `true`, renders a raincloud-style one-sided half-violin paired with individual points on the opposite half.
        half_violin_with_points: bool,
    },
    /// Min-to-max floating range bar spanning `[min(y), max(y)]` with optional summary line.
    FloatingBar {
        /// Optional central tendency line drawn inside the `[min, max]` bar; `None` omits the internal line.
        summary: Option<SummaryMeasure>,
        /// Bar interior fill color.
        fill_color: Rgba8,
        /// Bar border stroke color.
        border_color: Rgba8,
        /// Bar border width in typographic points (`>= 0.0`, finite).
        border_width_pt: f64,
    },
    /// Matched-subject trajectories connecting paired observations across adjacent columns at the same row index.
    BeforeAfterLines {
        /// Optional symbol drawn at each subject-column vertex; `None` draws connecting lines only.
        marker: Option<MarkerStyle>,
        /// Subject trajectory line color.
        line_color: Rgba8,
        /// Subject trajectory stroke width in typographic points (`>= 0.0`, finite).
        stroke_width_pt: f64,
    },
    /// Right-continuous Kaplan–Meier survival step function (`S(t)` in `[0, 1]` or `[0%, 100%]`).
    KaplanMeierStep {
        /// Step curve stroke color.
        line_color: Rgba8,
        /// Step curve stroke width in typographic points (`>= 0.0`, finite).
        stroke_width_pt: f64,
        /// When `true`, marks right-censored observations with vertical tick marks along the survival curve.
        show_censored_ticks: bool,
        /// When `true`, draws pointwise confidence bands (log-log complementary Greenwood transform) around `S(t)`.
        show_greenwood_ci: bool,
        /// When `true`, strokes confidence band bounds with dashed lines rather than solid lines.
        ci_dashed: bool,
        /// Survival vs cumulative failure event display orientation.
        #[serde(default)]
        display_metric: SurvivalDisplayMetric,
        /// When `true`, draws right-continuous staircase steps; when `false`, connects points directly.
        #[serde(default = "default_true")]
        staircase: bool,
        /// Optional Greenwood error display (`Sem` or `ConfidenceInterval95`).
        #[serde(default)]
        error_display: Option<ErrorBarMetric>,
    },
    /// Proportional categorical composition plot (`Pie`, `Donut`, `StackedFractionBar`, or `Waffle10x10`).
    PartsOfWhole {
        /// Geometric layout of the categorical fractions.
        layout: PartsOfWholeLayout,
        /// When `true` and `layout == PartsOfWholeLayout::Donut`, prints the series sum inside the central cutout.
        show_center_total: bool,
    },
}

/// Dispersion statistic encoded by error bars (`docs/implementation/graphing-and-layouts.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ErrorBarMetric {
    /// Sample standard deviation (`±1 SD`, `n - 1` denominator).
    #[default]
    Sd,
    /// Standard error of the mean (`±1 SEM = SD / sqrt(n)`).
    Sem,
    /// Two-sided 95% Student-*t* confidence interval around the sample mean (`mean ± t_{0.025, n-1} * SEM`).
    ConfidenceInterval95,
    /// Asymmetric interquartile span from `Q1` (25th percentile) to `Q3` (75th percentile).
    InterquartileRange,
    /// Full observed span from sample minimum to sample maximum.
    Range,
    /// Pre-summarized bounds read directly from [`crate::SubcolumnMode::explicit_bounds`].
    ExplicitSubcolumns,
}

/// Which side(s) of the summary anchor draw error whiskers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ErrorBarDirection {
    /// Symmetric/two-sided upper and lower whiskers.
    #[default]
    Both,
    /// Positive/upper whisker only (common above upward bars).
    UpperOnly,
    /// Negative/lower whisker only.
    LowerOnly,
}

/// Vertical or horizontal error-bar styling (supporting solid, dashed, and dotted strokes).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ErrorBarSpec {
    /// Statistical dispersion or interval quantity represented by the whisker endpoints.
    pub metric: ErrorBarMetric,
    /// Upper, lower, or two-sided whisker visibility.
    pub direction: ErrorBarDirection,
    /// Solid, dashed, or dotted whisker stem pattern.
    pub dash_style: LineDashStyle,
    /// Whisker stem and cap stroke width in typographic points (`>= 0.0`, finite).
    pub stroke_width_pt: f64,
    /// End-cap crossbar length in typographic points (`0.0` draws capless T-free whiskers).
    pub cap_width_pt: f64,
    /// Explicit error-bar color override; `None` inherits the parent series stroke color.
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
    /// Upstream regression [`crate::ResultSheet`] supplying [`crate::FittedCurveOverlay`] coordinates for `data_set_index`.
    pub result_sheet_id: SheetId,
    /// Adaptive polyline subdivision count in `[256, 1024]`.
    pub sample_segments: u16,
    /// When `true`, renders the curve's two-sided confidence band at the regression's configured [`ConfidenceLevel`](crate::ConfidenceLevel).
    pub show_confidence_band: bool,
    /// When `true`, renders the curve's two-sided single-observation prediction band at the regression's configured [`ConfidenceLevel`](crate::ConfidenceLevel).
    pub show_prediction_band: bool,
    /// When `true`, fills confidence/prediction bands as translucent ribbons; when `false`, draws dashed boundary curves.
    pub shaded_ribbon: bool,
}

/// Single plotted data series inside a [`GraphSpec`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeriesSpec {
    /// Upstream [`DataSheet`] (or analyzable transform [`crate::ResultSheet`]) supplying observations for this series.
    pub source_sheet_id: SheetId,
    /// Zero-based data-set, cohort, or group column index on `source_sheet_id`.
    pub data_set_index: usize,
    /// Optional custom legend label; `None` inherits the upstream column title dynamically.
    pub legend_label: Option<String>,
    /// When `true`, maps *Y* values against [`GraphSpec::y2_axis`] instead of [`GraphSpec::y_axis`].
    pub bind_to_right_y2: bool,
    /// Visual mark/bar/box/violin/step geometry for this series.
    pub gdo: GraphicalDataObject,
    /// Optional vertical (*Y*) error bars; `None` omits *Y* error bars.
    pub y_error_bars: Option<ErrorBarSpec>,
    /// Optional horizontal (*X*) error bars (for [`crate::XColumnMode::Numbers`] with summary/replicate subcolumns); `None` omits *X* error bars.
    pub x_error_bars: Option<ErrorBarSpec>,
    /// Optional live fitted regression curve and band overlay; `None` renders no regression overlay.
    pub regression_overlay: Option<RegressionOverlaySpec>,
    /// Per-series horizontal/vertical coordinate nudge (`offset.x_data`, `offset.x_pt`, `offset.y_pt`).
    #[serde(default)]
    pub offset: SeriesOffsetSpec,
    /// Explicit bar width override in typographic points (`> 0.0`) for XY and category bars.
    #[serde(default)]
    pub bar_width_override_pt: Option<f64>,
    /// When `true`, renders raw data points/bars/lines/error bars; when `false`, renders only fitted regression overlays.
    #[serde(default = "default_true")]
    pub show_raw_data: bool,
}

/// Horizontal cluster arrangement for `Grouped` and `Contingency` graphs (`docs/implementation/graphing-and-layouts.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum GroupedArrangement {
    /// Place data-set columns side-by-side within each row-factor cluster (`Row 1: [A, B, C]`, `Row 2: [A, B, C]`).
    #[default]
    Interleaved,
    /// Group all row-factor levels within separate blocks per data-set column (`A: [Row 1, Row 2]`, `B: [Row 1, Row 2]`).
    Separated,
    /// Stack data-set magnitudes cumulatively within each row-factor position.
    Stacked,
    /// Overlay all data sets directly at identical horizontal category anchors (common for interaction line plots).
    Superimposed,
}

/// Per-point visual style override (`graph.formatSelectedPoints`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PointStyleOverride {
    /// Zero-based index into [`GraphSpec::series`].
    pub series_index: usize,
    /// Zero-based row or observation index within the series.
    pub point_index: usize,
    /// Optional zero-based replicate subcolumn index; `None` applies the override to the summary point or all replicates at `(series_index, point_index)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subcol: Option<usize>,
    /// Optional stroke/outline color override; `None` keeps the series stroke color.
    #[serde(default)]
    pub color: Option<Rgba8>,
    /// Optional marker glyph shape override; `None` keeps the series marker shape.
    #[serde(default)]
    pub symbol: Option<MarkerShape>,
    /// Optional marker diameter in typographic points (`> 0.0`, finite); `None` keeps the series marker size.
    #[serde(default)]
    pub size_pt: Option<f64>,
    /// Optional marker border stroke width in typographic points (`>= 0.0`, finite); `None` keeps the series border width.
    #[serde(default)]
    pub border_width_pt: Option<f64>,
    /// Optional marker interior fill color override; `None` keeps the series fill color.
    #[serde(default)]
    pub fill_color: Option<Rgba8>,
    /// Optional solid/open/semi-filled marker fill mode override; `None` keeps the series fill mode.
    #[serde(default)]
    pub fill_mode: Option<MarkerFillMode>,
    /// When `true`, suppresses rendering of the targeted point without excluding it from upstream analyses.
    #[serde(default)]
    pub hidden: bool,
}

/// Fill-colour visual channel in a five-channel `MultipleVariables` plot (`docs/implementation/graphing-and-layouts.md` §2.5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ColorChannelMapping {
    /// Uniform fill color across all plotted observations.
    Constant(Rgba8),
    /// Discrete categorical color encoding mapped from a `MultipleVariables` column.
    Categorical {
        /// Zero-based variable column index supplying categorical group tokens.
        column: usize,
        /// Discrete colorblind-safe or publication palette cycled across levels.
        palette: PaletteId,
        /// Optional graph-local level order overriding the column's stored [`LevelOrderSpec`]; `None` uses the column's level order.
        level_order_override: Option<LevelOrderSpec>,
    },
    /// Continuous gradient color encoding mapped from a quantitative `MultipleVariables` column.
    Continuous {
        /// Zero-based variable column index supplying finite numeric values.
        column: usize,
        /// Perceptually uniform or diverging [`ColorRamp`] mapped across the value domain.
        ramp: ColorRamp,
        /// Optional explicit `(min, max)` clamping interval in data units; `None` auto-scales to observed `[min, max]`.
        domain_clamp: Option<(f64, f64)>,
    },
}

/// Symbol-area / radius channel in a five-channel `MultipleVariables` bubble plot.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SizeChannelMapping {
    /// Uniform symbol radius across all observations.
    Constant {
        /// Marker radius in typographic points (`> 0.0`, finite).
        radius_pt: f64,
    },
    /// Proportional bubble sizing mapped from a quantitative `MultipleVariables` column.
    Variable {
        /// Zero-based variable column index supplying numeric magnitudes.
        column: usize,
        /// Minimum marker radius in typographic points mapped to the smallest observed value.
        min_radius_pt: f64,
        /// Maximum marker radius in typographic points mapped to the largest observed value.
        max_radius_pt: f64,
    },
}

/// Rendering mode for a `MultipleVariables` graph (`docs/implementation/graphing-and-layouts.md` §2.5).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum MultivariatePlotMode {
    /// Five-channel continuous/categorical scatter and bubble plot.
    #[default]
    BubbleScatter,
    /// Categorical × continuous group summary plot (bar, box-and-whisker, violin, dot plot, floating bar).
    CategoricalSummary {
        /// Summary GDO rendered for each distinct level of `x_column`.
        gdo: GraphicalDataObject,
        /// Optional vertical error bars computed over each category's active `y_column` values; `None` omits error bars.
        y_error_bars: Option<ErrorBarSpec>,
    },
}

/// Declarative five-channel (`X`, `Y`, `Color`, `Size`, `Label`) or categorical × continuous
/// encoding for `MultipleVariables` plots.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MultivariateGraphEncoding {
    /// Scatter/bubble vs categorical-summary rendering mode.
    pub plot_mode: MultivariatePlotMode,
    /// Zero-based variable column index mapped to the horizontal *X* axis.
    pub x_column: usize,
    /// Zero-based variable column index mapped to the vertical *Y* axis.
    pub y_column: usize,
    /// Point or category color channel mapping.
    pub color: ColorChannelMapping,
    /// Point radius channel mapping (used in [`MultivariatePlotMode::BubbleScatter`]).
    pub size: SizeChannelMapping,
    /// Optional zero-based variable column index supplying per-point text labels; `None` draws no point labels.
    pub label_column: Option<usize>,
    /// Optional graph-local ordering override when `x_column` is categorical; `None` uses the column's [`LevelOrderSpec`].
    pub x_level_order_override: Option<LevelOrderSpec>,
    /// When `true` and `color` is [`ColorChannelMapping::Categorical`], draws convex-hull polygons around each color group.
    pub show_convex_hulls: bool,
    /// When `true` and `color` is [`ColorChannelMapping::Categorical`], draws bivariate normal confidence ellipses around each color group.
    pub show_confidence_ellipses: bool,
}

/// Geometry style of pairwise significance brackets (`docs/implementation/graphing-and-layouts.md` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum BracketStyle {
    /// Classic inverted-U bracket with vertical downward legs at `group_a` and `group_b`.
    #[default]
    Square,
    /// Horizontal bar with short downward end ticks.
    HalfTick,
    /// Plain horizontal line segment with no vertical end ticks.
    FlatLine,
    /// Smooth typographic curly brace pointing upward toward the significance label.
    CurlyBrace,
}

/// Text label mode for pairwise significance brackets.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BracketLabelMode {
    /// Display asterisk significance tiers (`"*"`, `"**"`, `"***"`, `"****"`, or `"ns"`) formatted via the upstream analysis's [`crate::PValueFormat`].
    #[default]
    SummaryStars,
    /// Display exact formatted numeric `p = ...` (or `q = ...` when [`crate::PairwiseComparisonRecord::is_fdr_q`] is `true`).
    NumericPOrQ,
    /// Display an explicit caller-supplied text string.
    Custom(String),
}

/// Live pairwise comparison bracket hooked to an upstream statistical `ResultSheet`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PairwiseBracketSpec {
    /// Upstream statistical [`crate::ResultSheet`] publishing [`crate::PairwiseComparisonRecord`] entries.
    pub result_sheet_id: SheetId,
    /// Zero-based index of the first compared group on the categorical axis.
    pub group_a: usize,
    /// Zero-based index of the second compared group on the categorical axis.
    pub group_b: usize,
    /// Visual bracket leg/cap shape.
    pub style: BracketStyle,
    /// Asterisk, numeric *p*/*q*, or custom text label mode.
    pub label_mode: BracketLabelMode,
    /// When `true`, automatically suppresses rendering when [`crate::PairwiseComparisonRecord::significant`] is `false`.
    pub hide_if_non_significant: bool,
    /// Bracket line stroke width in typographic points (`>= 0.0`, finite).
    pub stroke_width_pt: f64,
    /// Label font size in typographic points (`> 0.0`, finite).
    pub font_size_pt: f64,
    /// Vertical stacking clearance in typographic points between nested/overlapping brackets.
    pub bracket_step_pt: f64,
}

/// Geometric shape primitive for canvas callout regions (`docs/evidence/graphing.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum AnnotationShapeKind {
    /// Sharp-cornered axis-aligned rectangle.
    #[default]
    Rectangle,
    /// Axis-aligned rectangle with rounded corner arcs.
    RoundedRectangle,
    /// Axis-aligned ellipse inscribed in `bounds_pt`.
    Ellipse,
}

/// Display mode for a fitted model equation callout on a graph canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum EquationDisplayMode {
    /// Substitute converged numeric parameter estimates (for example `Y = 100.0 * X / (2.50 + X)`).
    #[default]
    Substituted,
    /// Render symbolic parameter names without numeric substitution (for example `Y = Vmax * X / (Km + X)`).
    Symbolic,
}

const fn default_model_equation_font_size() -> f64 {
    10.0
}

/// Canvas annotation primitive placed on a [`GraphSpec`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GraphAnnotation {
    /// Free-placed Markdown text callout (supports inline `{{info:...}}` and `{{result:...}}` hooks).
    TextBox {
        /// Canvas bounding box in typographic points (`width >= 0`, `height >= 0`).
        bounds_pt: RectPt,
        /// Markdown text content.
        markdown: String,
        /// When `true`, draws a rectangular border frame around the text box.
        bordered: bool,
    },
    /// Straight line segment or single-headed pointer arrow in canvas point coordinates.
    LineOrArrow {
        /// Starting `(x_pt, y_pt)` canvas coordinate in typographic points.
        start_pt: (f64, f64),
        /// Ending `(x_pt, y_pt)` canvas coordinate in typographic points (arrowhead tip when `arrowhead == true`).
        end_pt: (f64, f64),
        /// When `true`, draws a directional arrowhead at `end_pt`.
        arrowhead: bool,
        /// Solid, dashed, or dotted line stroke pattern.
        dash: LineDashStyle,
        /// Stroke and arrowhead sRGB color.
        color: Rgba8,
    },
    /// Geometric highlight region (`Rectangle`, `RoundedRectangle`, or `Ellipse`) in canvas point coordinates.
    Shape {
        /// Rectangle, rounded rectangle, or ellipse primitive.
        kind: AnnotationShapeKind,
        /// Bounding box in typographic points (`width >= 0`, `height >= 0`).
        bounds_pt: RectPt,
        /// Outline stroke color.
        stroke_color: Rgba8,
        /// Outline stroke width in typographic points (`>= 0.0`, finite).
        stroke_width_pt: f64,
        /// Solid, dashed, or dotted outline stroke style.
        dash: LineDashStyle,
        /// Optional interior fill color; `None` leaves the shape interior transparent.
        fill_color: Option<Rgba8>,
    },
    /// Live miniature table projecting an upstream [`crate::ResultSheet`] tab onto the graph canvas.
    EmbeddedResultTable {
        /// Upstream [`crate::ResultSheet`] identifier.
        result_sheet_id: SheetId,
        /// Specific [`crate::ResultTab::id`] to render; `None` renders the primary summary tab.
        tab_id: Option<String>,
        /// Canvas bounding box in typographic points (`width >= 0`, `height >= 0`).
        bounds_pt: RectPt,
        /// Optional zero-based row subset to display; `None` includes all rows.
        #[serde(default)]
        selected_rows: Option<Vec<usize>>,
        /// Optional zero-based column subset to display; `None` includes all columns.
        #[serde(default)]
        selected_cols: Option<Vec<usize>>,
        /// Table formatting, header visibility, transposition, and border styling.
        #[serde(default)]
        style: EmbeddedTableStyle,
    },
    /// Live TeX-formatted regression equation box bound to an upstream regression [`crate::ResultSheet`].
    ModelEquation {
        /// Upstream linear or nonlinear regression [`crate::ResultSheet`] identifier.
        result_sheet_id: SheetId,
        /// Zero-based fitted dataset index whose parameters populate [`EquationDisplayMode::Substituted`].
        #[serde(default)]
        data_set_index: usize,
        /// Numeric parameter substitution vs symbolic formula display.
        #[serde(default)]
        mode: EquationDisplayMode,
        /// Canvas bounding box in typographic points (`width >= 0`, `height >= 0`).
        bounds_pt: RectPt,
        /// Base equation font size in typographic points (`in (0, 144]`, default `10.0`).
        #[serde(default = "default_model_equation_font_size")]
        font_size_pt: f64,
        /// When `true`, draws a rectangular frame around the equation callout.
        #[serde(default)]
        bordered: bool,
    },
    /// Embedded PNG/JPEG/TIFF raster image referencing [`crate::ProjectImageAsset::asset_id`].
    EmbeddedImage {
        /// Content-addressed [`crate::ProjectImageAsset::asset_id`] stored on [`crate::ProjectDocument::image_assets`].
        asset_id: String,
        /// Placement bounding rectangle in canvas typographic points (`width > 0`, `height > 0`).
        bounds_pt: RectPt,
        /// Clockwise rotation angle in degrees around the center of `bounds_pt`.
        #[serde(default)]
        rotation_deg: f64,
        /// When `true`, interactive corner handles preserve the intrinsic aspect ratio.
        #[serde(default = "default_true")]
        lock_aspect_ratio: bool,
        /// Four-edge inward crop insets measured in typographic points.
        #[serde(default)]
        crop: CropInsetsPt,
        /// Optional outline stroke framing the cropped image region.
        #[serde(default)]
        border: Option<ImageBorderSpec>,
    },
}

/// Legend visibility, placement, and frame configuration on a [`GraphSpec`].
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LegendSpec {
    /// When `true`, renders the series/channel legend on the figure canvas.
    pub visible: bool,
    /// Optional explicit top-left `(x_pt, y_pt)` anchor in typographic points; `None` places the legend automatically outside/beside the plot frame.
    pub position_pt: Option<(f64, f64)>,
    /// When `true`, encloses the legend inside a bordered box.
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
    /// Upstream all-pairwise [`crate::ResultSheet`] supplying pairwise significance decisions (Piepho 2004).
    pub result_sheet_id: SheetId,
    /// Optional explicit alpha threshold in `(0, 1)` overriding precomputed `significant` decisions when adjusted *p*/*q* values are present; `None` uses the upstream result sheet's significance flags.
    #[serde(default)]
    pub alpha_override: Option<f64>,
    /// Whether `'a'` starts at the highest (`DescendingMean`) or lowest (`AscendingMean`) group location.
    #[serde(default)]
    pub ordering: CldOrdering,
    /// When `true`, formats CLD labels in uppercase (`"A"`, `"AB"`, `"B"`) instead of lowercase (`"a"`, `"ab"`, `"b"`).
    #[serde(default)]
    pub uppercase: bool,
    /// Font size of CLD group letters in typographic points (`in (0, 144]`, default `9.5`).
    #[serde(default = "default_cld_font_size")]
    pub font_size_pt: f64,
    /// Vertical clearance in typographic points above each group's highest bar/error-bar/point (`in [0, 200]`, default `6.0`).
    #[serde(default = "default_cld_offset_pt")]
    pub offset_pt: f64,
    /// Text color of rendered CLD letters (default black).
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
    /// Visual graph family (`Xy`, `Column`, `Grouped`, `Contingency`, `PartsOfWhole`, `Survival`, `MultipleVariables`, or `Diagnostic`).
    pub kind: GraphKind,
    /// Primary upstream [`DataSheet`] (or analyzable [`crate::ResultSheet`]) bound to this figure.
    pub primary_source_sheet_id: SheetId,
    /// Default categorical color palette applied to series swatches and style cloning.
    pub palette: PaletteId,
    /// Figure canvas dimensions in typographic points (`width > 0`, `height > 0`).
    pub canvas_size_pt: SizePt,
    /// Solid canvas background fill color; `None` renders a transparent figure background.
    pub paper_fill: Option<Rgba8>,
    /// Main figure title above the plot frame (supports inline `{{info:...}}` and `{{result:...}}` hooks).
    pub title: String,
    /// Axis spine arrangement preset (`OpenL`, `OffsetRangeTufte`, `PlainBoxed`, `DualYSecondaryRight`, or `OriginCrossing`).
    pub frame_preset: FramePreset,
    /// Horizontal *X* axis specification (`1..=3` segments).
    pub x_axis: AxisSpec,
    /// Primary left vertical *Y* axis specification (`1..=3` segments).
    pub y_axis: AxisSpec,
    /// Optional secondary right vertical *Y2* axis specification (used when [`FramePreset::DualYSecondaryRight`] or `bind_to_right_y2` is active); `None` when single-*Y*.
    pub y2_axis: Option<AxisSpec>,
    /// Horizontal cluster layout for `Grouped` and `Contingency` series.
    pub grouped_arrangement: GroupedArrangement,
    /// When `true` on [`GraphKind::Survival`] figures, renders an aligned number-at-risk table beneath the time axis.
    pub survival_number_at_risk: bool,
    /// Ordered plotted data series (`Xy`, `Column`, `Grouped`, `Contingency`, `PartsOfWhole`, `Survival`).
    pub series: Vec<SeriesSpec>,
    /// Five-channel or categorical-summary encoding when `kind == GraphKind::MultipleVariables`; `None` for non-multivariate graphs.
    pub multivariate: Option<MultivariateGraphEncoding>,
    /// Live pairwise comparison brackets bound to upstream statistical [`crate::ResultSheet`]s.
    pub comparisons: Vec<PairwiseBracketSpec>,
    /// Optional Piepho (2004) Compact Letter Display overlay bound to an all-pairwise [`crate::ResultSheet`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_letter_display: Option<CompactLetterDisplaySpec>,
    /// Controls whether pairwise significance renders as brackets, CLD letters, or both.
    #[serde(default, skip_serializing_if = "is_default_pairwise_display_mode")]
    pub pairwise_display_mode: PairwiseDisplayMode,
    /// Free-placed text boxes, lines/arrows, geometric shapes, embedded result tables, and model equations.
    pub annotations: Vec<GraphAnnotation>,
    /// Per-observation style overrides (`graph.formatSelectedPoints`).
    #[serde(default)]
    pub point_overrides: Vec<PointStyleOverride>,
    /// Legend visibility, position, and border configuration.
    pub legend: LegendSpec,
    /// Horizontal vs vertical bar/plot orientation (`Column` and `Grouped` plots).
    #[serde(default)]
    pub orientation: CartesianOrientation,
    /// When `true`, renders a fixed-effect pooled summary diamond on [`DiagnosticPlotKind::ForestPlot`].
    #[serde(default)]
    pub forest_show_pooled_diamond: bool,
    /// Row/column header display and rotation settings on [`DiagnosticPlotKind::Heatmap`].
    #[serde(default)]
    pub heatmap_labels: HeatmapLabelSpec,
}

impl GraphSpec {
    /// Synthesize the canonical default [`GraphSpec`] for a newly created [`DataSheet`], honoring
    /// `TableArchetype` and `ColumnDesignHint` (`docs/implementation/data-tables-and-transforms.md` §2.2 &
    /// `docs/implementation/graphing-and-layouts.md` §2).
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
                offset: SeriesOffsetSpec::default(),
                bar_width_override_pt: None,
                show_raw_data: true,
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
            TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
                let first_cat = mv.variables.iter().position(|v| v.role == VariableRole::Categorical);
                let first_cont = mv
                    .variables
                    .iter()
                    .position(|v| v.role == VariableRole::Continuous)
                    .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Calculated));
                let cont_count = mv.variables.iter().filter(|v| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated)).count();
                let (plot_mode, x_column, y_column) = match (first_cat, first_cont) {
                    (Some(cat_col), Some(cont_col)) if cont_count <= 1 => (
                        MultivariatePlotMode::CategoricalSummary {
                            gdo: GraphicalDataObject::BarWithReplicates {
                                summary: SummaryMeasure::Mean,
                                fill_color: palette.color_at(0),
                                border_color: Rgba8::BLACK,
                                border_width_pt: 0.75,
                                replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
                            },
                            y_error_bars: Some(ErrorBarSpec::default()),
                        },
                        cat_col,
                        cont_col,
                    ),
                    (Some(cat_col), Some(cont_col)) => (MultivariatePlotMode::default(), cat_col, cont_col),
                    _ => (MultivariatePlotMode::default(), 0, 1),
                };
                Some(MultivariateGraphEncoding {
                    plot_mode,
                    x_column,
                    y_column,
                    color: ColorChannelMapping::Constant(palette.color_at(0)),
                    size: SizeChannelMapping::Constant { radius_pt: 5.0 },
                    label_column: None,
                    x_level_order_override: None,
                    show_convex_hulls: false,
                    show_confidence_ellipses: false,
                })
            }
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
            orientation: CartesianOrientation::Vertical,
            forest_show_pooled_diamond: matches!(sheet.archetype, TableArchetype::MultipleVariables(_)),
            heatmap_labels: HeatmapLabelSpec::default(),
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
    /// (`graph.applyStyleFrom`; `docs/implementation/graphing-and-layouts.md` §5).
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
            self.x_axis.tick_direction = donor.x_axis.tick_direction;
            self.x_axis.show_tick_labels = donor.x_axis.show_tick_labels;
            self.y_axis.stroke_width_pt = donor.y_axis.stroke_width_pt;
            self.y_axis.break_ornament = donor.y_axis.break_ornament;
            self.y_axis.tick_direction = donor.y_axis.tick_direction;
            self.y_axis.show_tick_labels = donor.y_axis.show_tick_labels;
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
            if !series.offset.x_data.is_finite() || !series.offset.x_pt.is_finite() || !series.offset.y_pt.is_finite() {
                return Some("spec.series.offset");
            }
            if series.bar_width_override_pt.is_some_and(|w| !w.is_finite()) {
                return Some("spec.series.bar_width_override_pt");
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
        for series in &self.series {
            if series.bar_width_override_pt.is_some_and(|w| w <= 0.0) {
                return Err(DocError::InvalidCellValue {
                    archetype: crate::TableKind::Xy,
                    row: 0,
                    col: 0,
                    reason: "spec.series.bar_width_override_pt must be finite and > 0",
                });
            }
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
                GraphAnnotation::TextBox { bounds_pt, .. } => {
                    if bounds_pt.width < 0.0 || bounds_pt.height < 0.0 {
                        return Err(DocError::InvalidCellValue {
                            archetype: crate::TableKind::Xy,
                            row: 0,
                            col: 0,
                            reason: "spec.annotations.bounds_pt dimensions must be >= 0",
                        });
                    }
                }
                GraphAnnotation::EmbeddedResultTable { bounds_pt, style, .. } => {
                    if bounds_pt.width < 0.0 || bounds_pt.height < 0.0 {
                        return Err(DocError::InvalidCellValue {
                            archetype: crate::TableKind::Xy,
                            row: 0,
                            col: 0,
                            reason: "spec.annotations.bounds_pt dimensions must be >= 0",
                        });
                    }
                    style.validate(crate::TableKind::Xy)?;
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
                GraphAnnotation::EmbeddedImage { bounds_pt, crop, border, .. } => {
                    if bounds_pt.width <= 0.0 || bounds_pt.height <= 0.0 {
                        return Err(DocError::InvalidCellValue {
                            archetype: crate::TableKind::Xy,
                            row: 0,
                            col: 0,
                            reason: "spec.annotations.embedded_image bounds_pt width and height must be > 0",
                        });
                    }
                    crop.validate(crate::TableKind::Xy)?;
                    if let Some(b) = border {
                        b.validate(crate::TableKind::Xy)?;
                    }
                }
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
        TableArchetype::Xy(_) | TableArchetype::MultipleVariables(_) => GraphicalDataObject::ScatterAndLine {
            marker: Some(marker),
            line: None,
            area_fill: None,
            replicate_lines: ReplicateLineMode::SummaryCenter,
        },
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
            display_metric: SurvivalDisplayMetric::PercentSurvival,
            staircase: true,
            error_display: None,
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
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill, .. } => {
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
        GraphAnnotation::TextBox { bounds_pt, .. } => (!rect_is_finite(*bounds_pt)).then_some("spec.annotations.bounds_pt"),
        GraphAnnotation::EmbeddedResultTable { bounds_pt, style, .. } => {
            if !rect_is_finite(*bounds_pt) {
                return Some("spec.annotations.bounds_pt");
            }
            style.first_non_finite_float("spec.annotations.embedded_table.style")
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
        GraphAnnotation::EmbeddedImage { bounds_pt, rotation_deg, crop, border, .. } => {
            if !rect_is_finite(*bounds_pt) || !rotation_deg.is_finite() {
                return Some("spec.annotations.embedded_image");
            }
            if let Some(field) = crop.first_non_finite_float("spec.annotations.embedded_image.crop") {
                return Some(field);
            }
            if border.is_some_and(|b| !b.width_pt.is_finite()) {
                return Some("spec.annotations.embedded_image.border.width_pt");
            }
            None
        }
    }
}

/// Navigator sheet wrapping a [`GraphSpec`] and tracking first-visit gallery initialization.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraphSheet {
    /// Project-unique sheet identifier.
    pub id: SheetId,
    /// Display title shown in the `Graphs` navigator folder.
    pub title: String,
    /// Starts `false` when auto-provisioned alongside a new `DataSheet` so the first visit presents
    /// the visual family schematic chooser (`docs/implementation/graphing-and-layouts.md` §2).
    pub initialized: bool,
    /// Declarative resolution-independent figure specification.
    pub spec: GraphSpec,
    /// Sticky-note callouts attached to this graph sheet.
    pub floating_notes: Vec<FloatingNote>,
    /// Optional navigator color swatch; `None` uses no color badge.
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
    /// Provision an uninitialized (`initialized = false`) `GraphSheet` populated with [`GraphSpec::default_for_data_sheet`].
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

    #[test]
    fn multivariate_default_for_data_sheet_assigns_categorical_x_and_continuous_y() {
        use crate::{MultipleVariablesTable, MultivariatePlotMode, VariableRole};

        // 1. [Continuous, Categorical] -> CategoricalSummary with X = col 1 (Cat), Y = col 0 (Cont).
        let mut mv_rev = MultipleVariablesTable::new(2, 4).expect("mv 2");
        mv_rev.variables[0].title = String::from("Biomarker");
        mv_rev.variables[1].title = String::from("Group");
        mv_rev.variables[1].lock_role(VariableRole::Categorical);
        let ds_rev = DataSheet::new(SheetId::new("mv_rev"), "Rev", TableArchetype::MultipleVariables(mv_rev)).expect("ds_rev");
        let spec_rev = GraphSpec::default_for_data_sheet(&ds_rev);
        let enc_rev = spec_rev.multivariate.as_ref().expect("mv enc");
        assert!(matches!(enc_rev.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
        assert_eq!(enc_rev.x_column, 1);
        assert_eq!(enc_rev.y_column, 0);

        // 2. [Label, Categorical, Continuous] -> CategoricalSummary with X = col 1 (Cat), Y = col 2 (Cont).
        let mut mv_lbl = MultipleVariablesTable::new(3, 4).expect("mv 3");
        mv_lbl.variables[0].lock_role(VariableRole::Label);
        mv_lbl.variables[1].lock_role(VariableRole::Categorical);
        mv_lbl.variables[2].lock_role(VariableRole::Continuous);
        let ds_lbl = DataSheet::new(SheetId::new("mv_lbl"), "Lbl", TableArchetype::MultipleVariables(mv_lbl)).expect("ds_lbl");
        let spec_lbl = GraphSpec::default_for_data_sheet(&ds_lbl);
        let enc_lbl = spec_lbl.multivariate.as_ref().expect("mv enc lbl");
        assert!(matches!(enc_lbl.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
        assert_eq!(enc_lbl.x_column, 1);
        assert_eq!(enc_lbl.y_column, 2);

        // 3. All Continuous -> BubbleScatter with X = 0, Y = 1.
        let mv_cont = MultipleVariablesTable::new(3, 4).expect("mv all cont");
        let ds_cont = DataSheet::new(SheetId::new("mv_cont"), "Cont", TableArchetype::MultipleVariables(mv_cont)).expect("ds_cont");
        let spec_cont = GraphSpec::default_for_data_sheet(&ds_cont);
        let enc_cont = spec_cont.multivariate.as_ref().expect("mv enc cont");
        assert_eq!(enc_cont.plot_mode, MultivariatePlotMode::BubbleScatter);
        assert_eq!(enc_cont.x_column, 0);
        assert_eq!(enc_cont.y_column, 1);
    }
}
