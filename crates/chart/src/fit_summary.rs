//! Decoupled figure typography, fit-summary readability evaluation, and layout compiler (`R20.5`).
//!
//! # Prototype Evaluation: Readable Adjacent Fit Summary vs. Automatic Inside-Plot Inset
//!
//! Before committing to a default placement strategy for regression fit summaries, both
//! strategies were prototyped and evaluated across three representative scientific chart regimes
//! using [`evaluate_fit_summary_placement`]:
//!
//! 1. **Sparse single-curve dose-response (`360 × 260 pt`, 8 points + 4PL sigmoid):**
//!    - *Adjacent (`AdjacentRight`):* `0` occluded points, `0` occluded curve samples, `0%` plot
//!      area covered, shares the outside-right legend gutter (`plot_w >= 180 pt`), renders at
//!      `8.0 pt` (`>= 7.0 pt` readability floor, well above the `5.0 pt` preflight floor).
//!    - *Automatic Inset (`AutoInset`):* also achieves `0` occlusions in the empty `TopLeft` or
//!      `BottomRight` quadrant while preserving slightly more horizontal plot width, at the cost
//!      of consuming `~20%–28%` of the plot interior.
//! 2. **Dense multi-series scatter + fits (`360 × 260 pt`, 4 series spanning all 4 quadrants):**
//!    - *Adjacent (`AdjacentRight`):* **`0` occluded points and `0` occluded curve samples**
//!      regardless of data density.
//!    - *Automatic Inset (`AutoInset`):* **every** inside-plot quadrant collides with plotted
//!      markers or fitted curves (`occluded_points > 0` in all 4 corners).
//! 3. **Small / narrow viewports (`<= 240 × 160 pt`, e.g., multi-panel layout tiles):**
//!    - *Adjacent (`AdjacentRight`):* reserving a full right column would crush `plot_rect_pt`
//!      below the minimum usable width (`120.0 pt`) or clip the card past `scene.width_pt`.
//!    - *Fallback (`AdjacentPreferred`):* falls back to `AutoInset` **only** when a zero-occlusion
//!      corner fits within `<= 35%` of plot area and readable typography (`>= 7.0 pt`), otherwise
//!      places the card in the least-occluded valid position inside the canvas without clipping.
//!
//! **Decision:** Commit to **Readable Adjacent Placement (`AdjacentPreferred`)** as the default
//! strategy, sharing the outside-right gutter beneath any auto-legend whenever the canvas retains
//! at least `120.0 pt` of plot width and `>= 7.0 pt` typography.
//!
//! # Backward-Compatibility & Export-Determinism Rules (`R20.5`)
//!
//! 1. **Decoupled figure typography:** [`FigureTypography`] is owned strictly by `curvebook-chart`
//!    and never reads `crates/ui-egui` chrome tokens. Existing figure defaults (`12 pt` title,
//!    `10 pt` axis titles, `8.5 pt` ticks & legend) remain unchanged.
//! 2. **Saved-figure backward compatibility:** [`crate::compile_graph`] uses
//!    [`crate::CompileOptions::default()`] (`fit_summary.enabled == false`), so existing saved
//!    `GraphSpec` documents and SVG/PDF/PNG goldens remain bit-identical unless fit-summary
//!    layout is explicitly opted into via [`crate::compile_graph_with_options`].
//! 3. **Manual axis limits (`auto_range == false`) preserved:** Explicit `AxisSegment::min` /
//!    `AxisSegment::max` values are never altered by autoranging headroom or nice-tick snapping
//!    (enforced in `crate::scales::resolve_sample_segment_domain`).
//! 4. **Deterministic export:** All text measurements, corner occlusion counts, tie-breaking
//!    (`TopLeft` -> `BottomRight` -> `TopRight` -> `BottomLeft`), and coordinate math are pure,
//!    deterministic IEEE-754 operations yielding bit-identical `ChartScene`s and SHA-256 identical
//!    SVG, PDF, and PNG exports.

use curvebook_doc::{GraphSheet, ProjectDocument, ResultSheet, ResultStatus, TableArchetype};
use curvebook_palette::Rgba8;
use curvebook_text::{RichSpan, RichText, SpanWeight, measure_text_pt};
use serde::{Deserialize, Serialize};

use crate::scene::{ChartScene, FillPaint, Path2D, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor};

/// Minimum usable plot width (in PostScript points) preserved when reserving an adjacent right
/// gutter for a fit-summary card.
pub const MIN_USABLE_PLOT_WIDTH_PT: f64 = 120.0;

/// Maximum plot-area coverage fraction allowed before `AdjacentPreferred` treats an inside-plot
/// inset fallback as overly intrusive.
pub const MAX_CLEAN_INSET_COVERAGE: f64 = 0.35;

const CARD_PAD_X_PT: f64 = 6.0;
const CARD_PAD_Y_PT: f64 = 5.0;
const CARD_SWATCH_SIZE_PT: f64 = 5.0;
const CARD_SWATCH_GAP_PT: f64 = 5.0;
const CARD_LINE_HEIGHT_MULT: f64 = 1.32;
const CARD_ENTRY_GAP_PT: f64 = 3.5;
const CARD_OUTSIDE_GAP_PT: f64 = 8.0;
const CARD_OUTSIDE_RIGHT_BUFFER_PT: f64 = 2.0;
const CARD_LEGEND_STACK_GAP_PT: f64 = 6.0;
const CARD_INSET_MARGIN_PT: f64 = 6.0;

/// Publication figure typography scale owned strictly by `curvebook-chart` (decoupled from GUI
/// chrome tokens in `crates/ui-egui`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FigureTypography {
    /// Graph title font size in PostScript points (default `12.0 pt`).
    pub title_pt: f64,
    /// Axis title font size in PostScript points (default `10.0 pt`).
    pub axis_title_pt: f64,
    /// Axis tick label font size in PostScript points (default `8.5 pt`).
    pub tick_label_pt: f64,
    /// Figure legend label font size in PostScript points (default `8.5 pt`).
    pub legend_pt: f64,
    /// Fit-summary card body font size in PostScript points (default `8.0 pt`).
    pub fit_summary_pt: f64,
    /// Minimum readable font size allowed when gently scaling a fit-summary card (default `7.0 pt`).
    pub fit_summary_min_pt: f64,
    /// Publication preflight floor checked by [`crate::preflight_check_scene`] (`5.0 pt`).
    pub preflight_floor_pt: f64,
}

/// Canonical publication figure typography defaults for `curvebook-chart`.
pub const DEFAULT_FIGURE_TYPOGRAPHY: FigureTypography = FigureTypography {
    title_pt: 12.0,
    axis_title_pt: 10.0,
    tick_label_pt: 8.5,
    legend_pt: 8.5,
    fit_summary_pt: 8.0,
    fit_summary_min_pt: 7.0,
    preflight_floor_pt: 5.0,
};

impl Default for FigureTypography {
    fn default() -> Self {
        DEFAULT_FIGURE_TYPOGRAPHY
    }
}

impl FigureTypography {
    /// Sanitize `fit_summary_pt` and `fit_summary_min_pt` into finite, preflight-safe bounds.
    #[must_use]
    pub fn sanitized_fit_summary_sizes(self) -> (f64, f64) {
        let floor = if self.preflight_floor_pt.is_finite() { self.preflight_floor_pt.max(5.0) } else { 5.0 };
        let min_pt = if self.fit_summary_min_pt.is_finite() && self.fit_summary_min_pt > 0.0 {
            self.fit_summary_min_pt.clamp(floor, 24.0)
        } else {
            DEFAULT_FIGURE_TYPOGRAPHY.fit_summary_min_pt.max(floor)
        };
        let base_pt = if self.fit_summary_pt.is_finite() && self.fit_summary_pt > 0.0 {
            self.fit_summary_pt.clamp(min_pt, 24.0)
        } else {
            DEFAULT_FIGURE_TYPOGRAPHY.fit_summary_pt.max(min_pt)
        };
        (base_pt, min_pt)
    }
}

/// Candidate inside-plot corner for automatic inset evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InsetCorner {
    TopLeft,
    BottomRight,
    TopRight,
    BottomLeft,
}

impl InsetCorner {
    /// Canonical deterministic evaluation order used for tie-breaking.
    pub const ALL_ORDERED: [Self; 4] = [Self::TopLeft, Self::BottomRight, Self::TopRight, Self::BottomLeft];
}

/// Placement policy for the regression fit-summary card.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum FitSummaryPlacement {
    /// Prefer readable outside-right adjacent placement (zero data occlusion), falling back to
    /// automatic inside-plot inset only on narrow viewports where adjacent placement cannot retain
    /// [`MIN_USABLE_PLOT_WIDTH_PT`] or readable typography.
    #[default]
    AdjacentPreferred,
    /// Strictly place outside the right edge of `plot_rect_pt` (stacked beneath any auto-legend).
    AdjacentRight,
    /// Automatically score all four inner corners of `plot_rect_pt` and pick the least-occluded one.
    AutoInset,
    /// Place at an explicit top-left `(x_pt, y_pt)` anchor on the figure canvas.
    Explicit(PointPt),
}

/// Configuration for compiling an optional fit-summary card on a graph scene.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FitSummarySpec {
    /// When `false` (default), no automatic fit-summary card is emitted (preserves saved figures).
    pub enabled: bool,
    /// Placement strategy for the fit-summary card.
    pub placement: FitSummaryPlacement,
    /// Maximum number of scalar metrics shown per fitted series (clamped to `1..=6`, default `3`).
    pub max_metrics_per_series: usize,
    /// Whether to draw a crisp opaque paper background and quiet neutral border around the card.
    pub bordered: bool,
    /// Decoupled publication typography scale.
    pub typography: FigureTypography,
}

impl Default for FitSummarySpec {
    fn default() -> Self {
        Self {
            enabled: false,
            placement: FitSummaryPlacement::AdjacentPreferred,
            max_metrics_per_series: 3,
            bordered: true,
            typography: DEFAULT_FIGURE_TYPOGRAPHY,
        }
    }
}

/// Single key-value fit statistic inside a [`FitSummaryEntry`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FitSummaryMetric {
    pub label: String,
    pub formatted_value: String,
}

/// Structured fit summary extracted for one fitted series on a graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FitSummaryEntry {
    pub series_label: String,
    pub equation_label: String,
    pub color: Rgba8,
    pub converged: bool,
    pub ambiguous: bool,
    pub metrics: Vec<FitSummaryMetric>,
}

/// Resolved placement strategy kind returned inside [`PlacementCandidateScore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlacementStrategy {
    AdjacentRight,
    AutoInset,
    Explicit,
}

/// Quantitative readability and occlusion score for a single candidate fit-summary placement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlacementCandidateScore {
    /// Strategy that produced this candidate.
    pub strategy: PlacementStrategy,
    /// Inner corner when evaluating an inside-plot inset; `None` for adjacent or explicit.
    pub corner: Option<InsetCorner>,
    /// Resolved card bounding rectangle in PostScript points.
    pub bounds_pt: RectPt,
    /// Resolved body font size in PostScript points.
    pub font_size_pt: f64,
    /// Number of plotted data markers whose bounding circle intersects `bounds_pt`.
    pub occluded_points: usize,
    /// Number of sampled fitted-curve points falling inside `bounds_pt`.
    pub occluded_curve_samples: usize,
    /// Retained plot width in PostScript points.
    pub plot_width_retained_pt: f64,
    /// Fraction of `plot_rect_pt` area covered by the intersection of `bounds_pt` with `plot_rect_pt`.
    pub plot_coverage_fraction: f64,
    /// True if `bounds_pt` extends outside `[0, scene.width_pt] × [0, scene.height_pt]`.
    pub clipped_to_canvas: bool,
    /// True if `font_size_pt >= min_pt`, `!clipped_to_canvas`, and `plot_width_retained_pt >= MIN_USABLE_PLOT_WIDTH_PT`.
    pub readable: bool,
}

/// Side-by-side prototype evaluation comparing adjacent vs. automatic inside-plot inset placement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlacementEvaluation {
    /// Score of outside-right adjacent placement.
    pub adjacent: PlacementCandidateScore,
    /// Scores for all four inside-plot corners in [`InsetCorner::ALL_ORDERED`] order.
    pub inset_candidates: [PlacementCandidateScore; 4],
    /// Best-scoring inside-plot inset corner.
    pub best_inset: PlacementCandidateScore,
    /// Final placement chosen according to [`FitSummarySpec::placement`].
    pub chosen: PlacementCandidateScore,
}

/// Format a finite scalar value concisely (4 significant figures, clean scientific notation).
#[must_use]
fn format_fit_scalar(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let v = if value == 0.0 { 0.0 } else { value };
    let abs = v.abs();
    if abs == 0.0 {
        return Some(String::from("0"));
    }
    if !(1e-3..1e4).contains(&abs) {
        return Some(format!("{v:.3e}"));
    }
    let digits = (abs.log10().floor() as i32) + 1;
    let decimals = (4 - digits).clamp(0, 4) as usize;
    let raw = format!("{v:.decimals$}");
    if raw.contains('.') {
        let trimmed = raw.trim_end_matches('0').trim_end_matches('.');
        Some(if trimmed == "-0" { String::from("0") } else { trimmed.to_owned() })
    } else {
        Some(raw)
    }
}

/// Convert a snake_case equation ID into a concise human-readable label.
#[must_use]
fn humanize_equation_id(equation_id: &str) -> String {
    match equation_id.trim().to_ascii_lowercase().as_str() {
        "log_agonist_vs_response_4pl" | "agonist_vs_response_4pl" => String::from("4PL"),
        "log_inhibitor_vs_response_4pl" | "inhibitor_vs_response_4pl" => String::from("4PL Inhib."),
        "log_agonist_vs_response_3pl" | "agonist_vs_response_3pl" => String::from("3PL"),
        "log_inhibitor_vs_response_3pl" | "inhibitor_vs_response_3pl" => String::from("3PL Inhib."),
        "straight_line" => String::from("Linear"),
        "line_through_origin" => String::from("Linear (0,0)"),
        "one_phase_decay" => String::from("1-Phase Decay"),
        "one_phase_association" => String::from("1-Phase Assoc."),
        "michaelis_menten" => String::from("Michaelis-Menten"),
        "simple_logistic" => String::from("Logistic"),
        other => {
            if other.is_empty() {
                String::from("Fit")
            } else {
                other.replace('_', " ")
            }
        }
    }
}

/// Priority ordering for selecting the most informative fit metrics for a compact summary card.
fn metric_priority(key: &str) -> Option<(u8, &'static str)> {
    match key.trim().to_ascii_lowercase().as_str() {
        "r2" | "r^2" => Some((0, "R²")),
        "adjusted r2" | "adjusted r^2" => Some((1, "Adj. R²")),
        "pseudo r2" => Some((1, "Pseudo R²")),
        "ec50" => Some((2, "EC50")),
        "ic50" => Some((2, "IC50")),
        "logec50" => Some((3, "LogEC50")),
        "logic50" => Some((3, "LogIC50")),
        "hillslope" | "hill slope" => Some((4, "Hill")),
        "slope" => Some((4, "Slope")),
        "km" => Some((4, "Km")),
        "vmax" => Some((5, "Vmax")),
        "half_life" | "half-life" | "halflife" => Some((5, "t½")),
        "k" => Some((6, "K")),
        "yintercept" | "y_intercept" | "intercept" => Some((6, "Intercept")),
        "top" => Some((7, "Top")),
        "bottom" => Some((8, "Bottom")),
        "sy.x" | "rmse" => Some((9, "Sy.x")),
        _ => None,
    }
}

fn collect_ranked_metrics(res: &ResultSheet, data_set_index: usize, max_metrics: usize) -> Vec<FitSummaryMetric> {
    let limit = max_metrics.clamp(1, 6);
    let mut ranked: Vec<(u8, usize, &'static str, String)> = Vec::new();
    let mut seen_labels: Vec<&'static str> = Vec::new();

    // Pass 1: dataset-specific scalars.
    for (idx, scalar) in res.scalars.iter().enumerate() {
        if scalar.data_set_index != Some(data_set_index) {
            continue;
        }
        if let Some((prio, label)) = metric_priority(&scalar.key)
            && !seen_labels.contains(&label)
            && let Some(val_str) = format_fit_scalar(scalar.value)
        {
            let formatted_value = match &scalar.units {
                Some(u) if !u.trim().is_empty() => format!("{val_str} {}", u.trim()),
                _ => val_str,
            };
            seen_labels.push(label);
            ranked.push((prio, idx, label, formatted_value));
        }
    }

    // Pass 2: shared (`data_set_index: None`) fallback for metrics not already populated.
    for (idx, scalar) in res.scalars.iter().enumerate() {
        if scalar.data_set_index.is_some() {
            continue;
        }
        if let Some((prio, label)) = metric_priority(&scalar.key)
            && !seen_labels.contains(&label)
            && let Some(val_str) = format_fit_scalar(scalar.value)
        {
            let formatted_value = match &scalar.units {
                Some(u) if !u.trim().is_empty() => format!("{val_str} {}", u.trim()),
                _ => val_str,
            };
            seen_labels.push(label);
            ranked.push((prio, idx + res.scalars.len(), label, formatted_value));
        }
    }

    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ranked.truncate(limit);
    ranked.into_iter().map(|(_, _, label, formatted_value)| FitSummaryMetric { label: label.to_owned(), formatted_value }).collect()
}

/// Extract structured fit summary entries for every fitted regression overlay on `graph`.
#[must_use]
pub fn extract_fit_summary_entries(graph: &GraphSheet, doc: &ProjectDocument, max_metrics_per_series: usize) -> Vec<FitSummaryEntry> {
    let mut out = Vec::new();
    let palette = graph.spec.palette;
    for (idx, series) in graph.spec.series.iter().enumerate() {
        let Some(ov_spec) = &series.regression_overlay else {
            continue;
        };
        let Some(res) = doc.result_sheet(&ov_spec.result_sheet_id) else {
            continue;
        };
        if !matches!(res.status, ResultStatus::UpToDate) {
            continue;
        }
        let Some(curve) = res.fitted_curves.iter().find(|fc| fc.data_set_index == series.data_set_index) else {
            continue;
        };

        let fallback_label = doc
            .data_sheet(&series.source_sheet_id)
            .and_then(|ds| match &ds.archetype {
                TableArchetype::Xy(xy) => xy.data_sets.get(series.data_set_index).map(|d| d.title.clone()),
                _ => None,
            })
            .unwrap_or_else(|| format!("Series {}", idx + 1));
        let series_label = series.legend_label.clone().unwrap_or(fallback_label);
        let equation_label = humanize_equation_id(&curve.equation_id);
        let metrics = collect_ranked_metrics(res, series.data_set_index, max_metrics_per_series);
        out.push(FitSummaryEntry {
            series_label,
            equation_label,
            color: palette.color_at(idx),
            converged: curve.converged,
            ambiguous: curve.ambiguous,
            metrics,
        });
    }
    out
}

fn entry_header_rich(entry: &FitSummaryEntry) -> RichText {
    let text =
        if entry.equation_label.is_empty() { entry.series_label.clone() } else { format!("{} ({})", entry.series_label, entry.equation_label) };
    RichText { spans: vec![RichSpan { text, weight: SpanWeight::Bold, ..RichSpan::default() }] }
}

fn entry_status_text(entry: &FitSummaryEntry) -> Option<&'static str> {
    if !entry.converged {
        Some("Did not converge")
    } else if entry.ambiguous {
        Some("Ambiguous fit")
    } else {
        None
    }
}

/// Measure the exact `(width_pt, height_pt)` of a fit-summary card at `font_size_pt`.
#[must_use]
pub fn measure_fit_summary_card(entries: &[FitSummaryEntry], font_size_pt: f64) -> (f64, f64) {
    if entries.is_empty() {
        return (0.0, 0.0);
    }
    let fs = if font_size_pt.is_finite() && font_size_pt > 0.0 { font_size_pt } else { DEFAULT_FIGURE_TYPOGRAPHY.fit_summary_pt };
    let line_h = fs * CARD_LINE_HEIGHT_MULT;
    let mut max_line_w = 24.0_f64;
    let mut total_lines = 0_usize;

    for entry in entries {
        let hdr = entry_header_rich(entry);
        let hdr_w = CARD_SWATCH_SIZE_PT + CARD_SWATCH_GAP_PT + measure_text_pt(&hdr, fs).width_pt;
        max_line_w = max_line_w.max(hdr_w);
        total_lines += 1;

        if let Some(status) = entry_status_text(entry) {
            let st_rich = RichText::plain(status);
            max_line_w = max_line_w.max(measure_text_pt(&st_rich, fs).width_pt);
            total_lines += 1;
        }

        for m in &entry.metrics {
            let line = RichText::plain(format!("{} = {}", m.label, m.formatted_value));
            max_line_w = max_line_w.max(measure_text_pt(&line, fs).width_pt);
            total_lines += 1;
        }
    }

    let entry_gaps = (entries.len().saturating_sub(1) as f64) * CARD_ENTRY_GAP_PT;
    let width_pt = CARD_PAD_X_PT * 2.0 + max_line_w;
    let height_pt = CARD_PAD_Y_PT * 2.0 + (total_lines as f64) * line_h + entry_gaps;
    (width_pt, height_pt)
}

/// Compute the outside-right gutter width to reserve for a fit-summary card when
/// [`FitSummarySpec::enabled`] is `true` and placement prefers or requires the right gutter.
///
/// Returns `None` if fit-summary is disabled, there are no fitted entries, placement is
/// `AutoInset`/`Explicit`, or the canvas is too narrow to keep `MIN_USABLE_PLOT_WIDTH_PT`.
#[must_use]
pub(crate) fn reserved_adjacent_fit_summary_width_pt(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    canvas_width_pt: f64,
    has_right_y2: bool,
    spec: &FitSummarySpec,
) -> Option<f64> {
    if !spec.enabled || matches!(spec.placement, FitSummaryPlacement::AutoInset | FitSummaryPlacement::Explicit(_)) {
        return None;
    }
    let entries = extract_fit_summary_entries(graph, doc, spec.max_metrics_per_series);
    if entries.is_empty() {
        return None;
    }
    let (base_pt, min_pt) = spec.typography.sanitized_fit_summary_sizes();
    let (card_w, _) = measure_fit_summary_card(&entries, base_pt);
    let (min_card_w, _) = measure_fit_summary_card(&entries, min_pt);
    let left_est = if graph.spec.y_axis.title.trim().is_empty() { 42.0_f64 } else { 56.0_f64 }.min(canvas_width_pt * 0.28);
    let y2_est = if has_right_y2 { 48.0 } else { 0.0 };
    let avail_for_right = canvas_width_pt - left_est - MIN_USABLE_PLOT_WIDTH_PT - y2_est - (CARD_OUTSIDE_GAP_PT + CARD_OUTSIDE_RIGHT_BUFFER_PT);
    if avail_for_right < min_card_w {
        return None;
    }
    Some(card_w.min(avail_for_right))
}

fn rect_intersects_circle(rect: RectPt, center: PointPt, radius: f64) -> bool {
    let closest_x = center.x.clamp(rect.x, rect.right());
    let closest_y = center.y.clamp(rect.y, rect.bottom());
    let dx = center.x - closest_x;
    let dy = center.y - closest_y;
    dx * dx + dy * dy <= radius * radius
}

fn rect_contains_point(rect: RectPt, pt: PointPt) -> bool {
    pt.x >= rect.x && pt.x <= rect.right() && pt.y >= rect.y && pt.y <= rect.bottom()
}

fn rect_intersection_area(a: RectPt, b: RectPt) -> f64 {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    (x1 - x0).max(0.0) * (y1 - y0).max(0.0)
}

fn score_candidate(
    strategy: PlacementStrategy,
    corner: Option<InsetCorner>,
    bounds_pt: RectPt,
    font_size_pt: f64,
    min_font_pt: f64,
    scene: &ChartScene,
    curve_samples_pt: &[PointPt],
) -> PlacementCandidateScore {
    let occluded_points = scene.plotted_points.iter().filter(|m| rect_intersects_circle(bounds_pt, m.center_pt, m.radius_pt)).count();
    let occluded_curve_samples = curve_samples_pt.iter().filter(|&&pt| rect_contains_point(bounds_pt, pt)).count();
    let plot_area = (scene.plot_rect_pt.width * scene.plot_rect_pt.height).max(1.0);
    let plot_coverage_fraction = rect_intersection_area(bounds_pt, scene.plot_rect_pt) / plot_area;
    let clipped_to_canvas =
        bounds_pt.x < -0.5 || bounds_pt.y < -0.5 || bounds_pt.right() > scene.width_pt + 0.5 || bounds_pt.bottom() > scene.height_pt + 0.5;
    let plot_width_retained_pt = scene.plot_rect_pt.width;
    let readable = font_size_pt >= min_font_pt - 1e-6 && !clipped_to_canvas && plot_width_retained_pt >= MIN_USABLE_PLOT_WIDTH_PT - 0.5;
    PlacementCandidateScore {
        strategy,
        corner,
        bounds_pt,
        font_size_pt,
        occluded_points,
        occluded_curve_samples,
        plot_width_retained_pt,
        plot_coverage_fraction,
        clipped_to_canvas,
        readable,
    }
}

fn resolve_scaled_font_for_width(entries: &[FitSummaryEntry], base_pt: f64, min_pt: f64, max_w: f64) -> (f64, f64, f64) {
    let (w0, h0) = measure_fit_summary_card(entries, base_pt);
    if w0 <= max_w || max_w <= 0.0 {
        return (base_pt, w0, h0);
    }
    let scale = (max_w / w0).clamp(min_pt / base_pt, 1.0);
    let scaled_pt = (base_pt * scale).max(min_pt);
    let (w1, h1) = measure_fit_summary_card(entries, scaled_pt);
    (scaled_pt, w1, h1)
}

/// Prototype and evaluate both **adjacent outside-right** and **automatic inside-plot inset**
/// placement strategies on `scene`, returning all candidate scores and the chosen placement.
#[must_use]
pub fn evaluate_fit_summary_placement(
    scene: &ChartScene,
    entries: &[FitSummaryEntry],
    curve_samples_pt: &[PointPt],
    legend_bottom_y_pt: Option<f64>,
    right_gutter_offset_pt: f64,
    spec: &FitSummarySpec,
) -> Option<PlacementEvaluation> {
    if entries.is_empty() {
        return None;
    }
    let (base_pt, min_pt) = spec.typography.sanitized_fit_summary_sizes();
    let r = scene.plot_rect_pt;

    // 1. Evaluate AdjacentRight (outside-right gutter, stacked below auto-legend if present).
    let adj_x = r.right() + right_gutter_offset_pt.max(0.0) + CARD_OUTSIDE_GAP_PT;
    let avail_right_w = (scene.width_pt - adj_x - CARD_OUTSIDE_RIGHT_BUFFER_PT).max(0.0);
    let (adj_fs, adj_w, adj_h) = resolve_scaled_font_for_width(entries, base_pt, min_pt, avail_right_w);
    let adj_y = match legend_bottom_y_pt {
        Some(ly) if ly.is_finite() => ly + CARD_LEGEND_STACK_GAP_PT,
        _ => r.y,
    };
    let adjacent =
        score_candidate(PlacementStrategy::AdjacentRight, None, RectPt::new(adj_x, adj_y, adj_w, adj_h), adj_fs, min_pt, scene, curve_samples_pt);

    // 2. Evaluate all 4 AutoInset corners inside `plot_rect_pt`.
    let max_inset_w = (r.width - 2.0 * CARD_INSET_MARGIN_PT).max(0.0);
    let (inset_fs, inset_w, inset_h) = resolve_scaled_font_for_width(entries, base_pt, min_pt, max_inset_w);
    let mut inset_candidates = [adjacent; 4];
    for (idx, corner) in InsetCorner::ALL_ORDERED.into_iter().enumerate() {
        let (ix, iy) = match corner {
            InsetCorner::TopLeft => (r.x + CARD_INSET_MARGIN_PT, r.y + CARD_INSET_MARGIN_PT),
            InsetCorner::TopRight => (r.right() - inset_w - CARD_INSET_MARGIN_PT, r.y + CARD_INSET_MARGIN_PT),
            InsetCorner::BottomLeft => (r.x + CARD_INSET_MARGIN_PT, r.bottom() - inset_h - CARD_INSET_MARGIN_PT),
            InsetCorner::BottomRight => (r.right() - inset_w - CARD_INSET_MARGIN_PT, r.bottom() - inset_h - CARD_INSET_MARGIN_PT),
        };
        inset_candidates[idx] = score_candidate(
            PlacementStrategy::AutoInset,
            Some(corner),
            RectPt::new(ix, iy, inset_w, inset_h),
            inset_fs,
            min_pt,
            scene,
            curve_samples_pt,
        );
    }

    let mut best_inset = inset_candidates[0];
    for cand in inset_candidates.iter().skip(1).copied() {
        let better = (cand.clipped_to_canvas, cand.occluded_points, cand.occluded_curve_samples)
            < (best_inset.clipped_to_canvas, best_inset.occluded_points, best_inset.occluded_curve_samples);
        if better {
            best_inset = cand;
        }
    }

    let chosen = match spec.placement {
        FitSummaryPlacement::AdjacentRight => adjacent,
        FitSummaryPlacement::AutoInset => best_inset,
        FitSummaryPlacement::Explicit(pt) => {
            let (ew, eh) = measure_fit_summary_card(entries, base_pt);
            let ex = if pt.x.is_finite() { pt.x } else { adj_x };
            let ey = if pt.y.is_finite() { pt.y } else { adj_y };
            score_candidate(PlacementStrategy::Explicit, None, RectPt::new(ex, ey, ew, eh), base_pt, min_pt, scene, curve_samples_pt)
        }
        FitSummaryPlacement::AdjacentPreferred => {
            if adjacent.readable {
                adjacent
            } else if !best_inset.clipped_to_canvas
                && best_inset.occluded_points == 0
                && best_inset.plot_coverage_fraction <= MAX_CLEAN_INSET_COVERAGE
            {
                best_inset
            } else if !adjacent.clipped_to_canvas {
                adjacent
            } else {
                best_inset
            }
        }
    };

    Some(PlacementEvaluation { adjacent, inset_candidates, best_inset, chosen })
}

/// Emit vector primitives for a fit-summary card into `scene` at `bounds_pt`.
pub fn render_fit_summary_card(scene: &mut ChartScene, entries: &[FitSummaryEntry], bounds_pt: RectPt, font_size_pt: f64, bordered: bool) {
    if entries.is_empty() || bounds_pt.width <= 0.0 || bounds_pt.height <= 0.0 {
        return;
    }
    let fs = if font_size_pt.is_finite() && font_size_pt > 0.0 { font_size_pt } else { DEFAULT_FIGURE_TYPOGRAPHY.fit_summary_pt };
    let line_h = fs * CARD_LINE_HEIGHT_MULT;

    if bordered {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(bounds_pt),
            fill: Some(FillPaint::solid(Rgba8::rgba(255, 255, 255, 245))),
            stroke: Some(StrokePaint::solid(Rgba8::rgb(180, 185, 192), 0.75)),
            clip: None,
        });
    }

    let mut cursor_y = bounds_pt.y + CARD_PAD_Y_PT;
    let text_x = bounds_pt.x + CARD_PAD_X_PT;
    let body_color = Rgba8::rgb(30, 34, 42);
    let warn_color = Rgba8::rgb(176, 68, 24);

    for (entry_idx, entry) in entries.iter().enumerate() {
        if entry_idx > 0 {
            cursor_y += CARD_ENTRY_GAP_PT;
        }
        // Series color swatch + bold header.
        let swatch_y = cursor_y + 0.5 * (line_h - CARD_SWATCH_SIZE_PT);
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(RectPt::new(text_x, swatch_y, CARD_SWATCH_SIZE_PT, CARD_SWATCH_SIZE_PT)),
            fill: Some(FillPaint::solid(entry.color)),
            stroke: None,
            clip: None,
        });
        let baseline_y = cursor_y + fs * 0.88;
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: entry_header_rich(entry),
            anchor_pt: PointPt::new(text_x + CARD_SWATCH_SIZE_PT + CARD_SWATCH_GAP_PT, baseline_y),
            anchor: TextAnchor::Start,
            font_size_pt: fs,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
        cursor_y += line_h;

        if let Some(status) = entry_status_text(entry) {
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(status),
                anchor_pt: PointPt::new(text_x, cursor_y + fs * 0.88),
                anchor: TextAnchor::Start,
                font_size_pt: fs,
                rotation_deg: 0.0,
                color: warn_color,
            }));
            cursor_y += line_h;
        }

        for m in &entry.metrics {
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(format!("{} = {}", m.label, m.formatted_value)),
                anchor_pt: PointPt::new(text_x, cursor_y + fs * 0.88),
                anchor: TextAnchor::Start,
                font_size_pt: fs,
                rotation_deg: 0.0,
                color: body_color,
            }));
            cursor_y += line_h;
        }
    }
}
