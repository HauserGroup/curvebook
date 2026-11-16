//! Graph creation, styling, axis, series, overlay, bracket, annotation, canvas preset, and preflight commands.

use curvebook_chart::preflight_check_scene;
use curvebook_doc::{
    AdditionalTick, AnnotationShapeKind, AxisSegment, AxisSpec, AxisThreshold, BracketLabelMode, BracketStyle, BreakOrnament, CanvasResizeMode,
    CategoricalTickSource, CldOrdering, ColorChannelMapping, CompactLetterDisplaySpec, ConnectingLineStyle, DiagnosticPlotKind, EquationDisplayMode,
    ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, ErrorCode, FigureCanvasPreset, FramePreset, GraphAnnotation, GraphAxisScale, GraphKind,
    GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, LineConnectionKind, MultivariateGraphEncoding, MultivariatePlotMode,
    PairwiseBracketSpec, PairwiseComparisonRecord, PairwiseDisplayMode, PartsOfWholeLayout, PointStyleOverride, ProjectDocument, RectPt,
    RegressionOverlaySpec, ReplicateOverlayStyle, SeriesSpec, SizeChannelMapping, SizePt, SummaryMeasure, TickLabelFormat, WhiskerMethod,
};
use curvebook_math::LogitDomain;
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::analysis::{de_present_opt, merge_json_struct, strip_keys};
use super::export::{compile_scene_for_sheet, de_opt_u16, de_opt_usize};
use super::helpers::{next_unique_sheet_id, parse_sheet_id, require_graph_sheet, require_graph_sheet_mut, require_result_sheet};
use super::params::{
    any_json_schema, array_schema, bool_schema, enum_str_schema, int_schema, non_empty_str_schema, nullable_schema, num_schema, obj_schema,
    obj_schema_with_aliases, parse_params, str_schema,
};
use super::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::{CommandOutcome, EngineError, Session};

const MUTATE_IDEMPOTENT: Effects = Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT);
const READ_ONLY_IDEMPOTENT: Effects = Effects::NONE.with(Effects::IDEMPOTENT);

/// Return all registered graph configuration and preflight command specifications.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

static SPECS: &[CommandSpec] = &[
    CommandSpec {
        id: "graph.create",
        label: "New Graph",
        menu_path: "Insert/New Graph...",
        shortcut: "",
        summary: "Create a new graph sheet bound to an input data sheet with optional kind, title, and initialization state.",
        beginner: "Creates a new graph linked to one of your data tables.",
        group: CommandGroup::Graph,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_graph_create,
        enabled: has_data_sheet,
        run: run_graph_create,
    },
    CommandSpec {
        id: "graph.initialize",
        label: "Initialize Graph",
        menu_path: "Graph/Initialize...",
        shortcut: "",
        summary: "Initialize an uninitialized graph sheet, applying its graph kind and default series styling.",
        beginner: "Chooses an initial plot type for a newly created graph sheet.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_initialize,
        enabled: has_graph_sheet,
        run: run_graph_initialize,
    },
    CommandSpec {
        id: "graph.setKind",
        label: "Change Graph Type",
        menu_path: "Change/Graph Type...",
        shortcut: "",
        summary: "Switch a graph's visualization type (XY, column, grouped, survival, multivariate, diagnostic) with optional style reset.",
        beginner: "Changes the chart type of an existing graph without recreating the sheet.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_kind,
        enabled: has_graph_sheet,
        run: run_graph_set_kind,
    },
    CommandSpec {
        id: "graph.setSpec",
        label: "Update Graph Specification",
        menu_path: "Format/Graph...",
        shortcut: "",
        summary: "Patch a graph's full GraphSpec, palette, frame, legend, title, survival options, or multivariate encoding.",
        beginner: "Updates visual graph settings such as palette, frame style, legend, or multivariate channels.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_spec,
        enabled: has_graph_sheet,
        run: run_graph_set_spec,
    },
    CommandSpec {
        id: "graph.setAxes",
        label: "Format Axes",
        menu_path: "Format/Axes...",
        shortcut: "",
        summary: "Configure X, Y, and right-hand Y2 axes including log/probit/logit scales, segmented breaks, thresholds, and frame presets.",
        beginner: "Adjusts axis limits, log scales, titles, broken-axis gaps, and frame styling.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_axes,
        enabled: has_graph_sheet,
        run: run_graph_set_axes,
    },
    CommandSpec {
        id: "graph.setAdditionalTicks",
        label: "Set Additional Axis Ticks",
        menu_path: "Format/Additional Ticks...",
        shortcut: "",
        summary: "Set custom callout ticks and optional gridlines along the `x`, `y`, or `y2` axis.",
        beginner: "Places custom reference ticks and gridlines at specific axis coordinates.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_additional_ticks,
        enabled: has_graph_sheet,
        run: run_graph_set_additional_ticks,
    },
    CommandSpec {
        id: "graph.setSeries",
        label: "Format Data Series",
        menu_path: "Format/Data on Graph...",
        shortcut: "",
        summary: "Style one or all data series: colors, markers, connecting lines, error bars, right-Y binding, or multivariate encoding.",
        beginner: "Customizes markers, connecting lines, error bars, and colors for curves or bars.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_series,
        enabled: has_graph_sheet,
        run: run_graph_set_series,
    },
    CommandSpec {
        id: "graph.setRegressionOverlay",
        label: "Overlay Regression Curve",
        menu_path: "Graph/Regression Overlay...",
        shortcut: "",
        summary: "Bind or clear a fitted curve overlay with optional confidence/prediction bands on one or all graph series.",
        beginner: "Overlays a curve fit line and confidence ribbons from a result sheet onto your graph.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_regression_overlay,
        enabled: has_graph_sheet,
        run: run_graph_set_regression_overlay,
    },
    CommandSpec {
        id: "graph.addComparisonBrackets",
        label: "Add Pairwise Significance Brackets",
        menu_path: "Draw/Pairwise Comparison Brackets...",
        shortcut: "",
        summary: "Add statistical comparison brackets (stars or numeric P/q values) driven by a result sheet's pairwise comparisons.",
        beginner: "Draws significance brackets above compared groups using P values from your statistical test.",
        group: CommandGroup::Graph,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_graph_add_comparison_brackets,
        enabled: has_graph_sheet,
        run: run_graph_add_comparison_brackets,
    },
    CommandSpec {
        id: "graph.addAnnotation",
        label: "Add Graph Annotation",
        menu_path: "Draw/Annotation...",
        shortcut: "",
        summary: "Add a text box, line/arrow, shape, live embedded result table, or live fitted model equation annotation to a graph.",
        beginner: "Places labels, arrows, geometric callouts, embedded result summaries, or model equations onto a graph.",
        group: CommandGroup::Graph,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_graph_add_annotation,
        enabled: has_graph_sheet,
        run: run_graph_add_annotation,
    },
    CommandSpec {
        id: "graph.applyStyleFrom",
        label: "Apply Graph Style From...",
        menu_path: "Format/Apply Style From...",
        shortcut: "",
        summary: "Copy visual styling (palette, frame, axis styling, legend, and series appearances) from a donor graph.",
        beginner: "Copies the visual style of another graph sheet onto this graph without changing its data.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_apply_style_from,
        enabled: has_graph_sheet,
        run: run_graph_apply_style_from,
    },
    CommandSpec {
        id: "graph.applyCanvasPreset",
        label: "Apply Graph Canvas Preset",
        menu_path: "Format/Canvas Size Preset...",
        shortcut: "",
        summary: "Resize a graph canvas to a journal column, slide, or poster preset (`KeepTypographyPt` or `ProportionalScale`).",
        beginner: "Resizes a graph to standard journal figure widths and runs a publication preflight check.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_apply_canvas_preset,
        enabled: has_graph_sheet,
        run: run_graph_apply_canvas_preset,
    },
    CommandSpec {
        id: "graph.preflightCheck",
        label: "Preflight Check Figure",
        menu_path: "Analyze/Preflight Check",
        shortcut: "",
        summary: "Compile a graph or layout sheet and check typography, hairlines, clipping, and overlap readiness.",
        beginner: "Audits a graph or layout for publication print issues like tiny fonts or clipped labels.",
        group: CommandGroup::Graph,
        effects: READ_ONLY_IDEMPOTENT,
        input_schema: schema_graph_preflight_check,
        enabled: has_graph_or_layout,
        run: run_graph_preflight_check,
    },
    CommandSpec {
        id: "graph.formatSelectedPoints",
        label: "Format Selected Points",
        menu_path: "Format/Selected Points...",
        shortcut: "",
        summary: "Apply, merge, replace, or clear per-point visual overrides across one or more selected graph points.",
        beginner: "Highlights or hides specific data points (changing color, symbol, size, border, or visibility) without modifying the whole series.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_format_selected_points,
        enabled: has_graph_sheet,
        run: run_graph_format_selected_points,
    },
    CommandSpec {
        id: "graph.setCompactLetterDisplay",
        label: "Compact Letter Display",
        menu_path: "Draw/Compact Letter Display...",
        shortcut: "",
        summary: "Configure or toggle Piepho (2004) compact letter display significance grouping letters and pairwise display mode on a graph.",
        beginner: "Places shared grouping letters (a, b, ab) above graph groups to summarize all-pairwise statistical comparisons.",
        group: CommandGroup::Graph,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_compact_letter_display,
        enabled: has_graph_sheet,
        run: run_graph_set_compact_letter_display,
    },
];

fn has_data_sheet(session: &Session) -> bool {
    !session.document().data_sheets.is_empty() || session.document().result_sheets.iter().any(|r| r.analyzable_data.is_some())
}

fn has_graph_sheet(session: &Session) -> bool {
    !session.document().graph_sheets.is_empty()
}

fn has_graph_or_layout(session: &Session) -> bool {
    !session.document().graph_sheets.is_empty() || !session.document().layout_sheets.is_empty()
}

// ---------------------------------------------------------------------------
// Shared Graph / Layout Helpers (reused by layout.rs)
// ---------------------------------------------------------------------------

/// Parse a named or `#RRGGBB` / `[r, g, b, a]` color specification.
pub(super) fn parse_rgba8(val: &Value) -> Result<Rgba8, EngineError> {
    if let Some(hex) = val.as_str() {
        return parse_color_string(hex);
    }
    if let Some(arr) = val.as_array()
        && (arr.len() == 3 || arr.len() == 4)
    {
        let r = byte_at(arr, 0)?;
        let g = byte_at(arr, 1)?;
        let b = byte_at(arr, 2)?;
        let a = if arr.len() == 4 { byte_at(arr, 3)? } else { 255 };
        return Ok(Rgba8::rgba(r, g, b, a));
    }
    serde_json::from_value::<Rgba8>(val.clone()).map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid color `{val}`: {err}")))
}

fn byte_at(arr: &[Value], idx: usize) -> Result<u8, EngineError> {
    let raw = arr.get(idx).and_then(Value::as_u64).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "color channel must be 0..=255"))?;
    u8::try_from(raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "color channel out of u8 range"))
}

fn parse_color_string(raw: &str) -> Result<Rgba8, EngineError> {
    let s = raw.trim();
    match s.to_ascii_lowercase().as_str() {
        "black" => return Ok(Rgba8::BLACK),
        "white" => return Ok(Rgba8::WHITE),
        "transparent" => return Ok(Rgba8::TRANSPARENT),
        "red" => return Ok(Rgba8::rgb(214, 39, 40)),
        "blue" => return Ok(Rgba8::rgb(31, 119, 180)),
        "green" => return Ok(Rgba8::rgb(44, 160, 44)),
        "orange" => return Ok(Rgba8::rgb(255, 127, 14)),
        "purple" => return Ok(Rgba8::rgb(148, 103, 189)),
        "teal" => return Ok(Rgba8::rgb(23, 190, 207)),
        "gray" | "grey" => return Ok(Rgba8::rgb(127, 127, 127)),
        _ => {}
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    if !hex.is_ascii() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{raw}`")));
    }
    match hex.len() {
        6 => {
            let (Some(r), Some(g), Some(b)) = (hex.get(0..2), hex.get(2..4), hex.get(4..6)) else {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{raw}`")));
            };
            Ok(Rgba8::rgb(parse_hex_u8(r)?, parse_hex_u8(g)?, parse_hex_u8(b)?))
        }
        8 => {
            let (Some(r), Some(g), Some(b), Some(a)) = (hex.get(0..2), hex.get(2..4), hex.get(4..6), hex.get(6..8)) else {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{raw}`")));
            };
            Ok(Rgba8::rgba(parse_hex_u8(r)?, parse_hex_u8(g)?, parse_hex_u8(b)?, parse_hex_u8(a)?))
        }
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{raw}`"))),
    }
}

fn parse_hex_u8(pair: &str) -> Result<u8, EngineError> {
    u8::from_str_radix(pair, 16).map_err(|_| EngineError::with_code(ErrorCode::BadInput, format!("invalid hex byte `{pair}`")))
}

/// Parse a `[x, y, width, height]` array or [`RectPt`] object.
pub(super) fn parse_rect_pt(val: Option<&Value>, default: RectPt) -> Result<RectPt, EngineError> {
    let Some(v) = val else {
        return Ok(default);
    };
    if let Some(arr) = v.as_array()
        && arr.len() == 4
    {
        let x = arr[0].as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "rect x must be f64"))?;
        let y = arr[1].as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "rect y must be f64"))?;
        let w = arr[2].as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "rect width must be f64"))?;
        let h = arr[3].as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "rect height must be f64"))?;
        return Ok(RectPt::new(x, y, w, h));
    }
    serde_json::from_value(v.clone()).map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `bounds_pt`: {err}")))
}

/// Parse a [`FigureCanvasPreset`] from string.
pub(super) fn parse_figure_canvas_preset(raw: &str) -> Result<FigureCanvasPreset, EngineError> {
    FigureCanvasPreset::from_name(raw).ok_or_else(|| {
        EngineError::with_code(ErrorCode::BadInput, format!("unknown canvas preset `{raw}`"))
            .with_remediation("Use one of SingleColumn88mm, ColumnAndHalf130mm, DoubleColumn180mm, FullPagePortrait, Slide16x9, or PosterPanel.")
            .with_details(json!({ "pointer": "/preset", "preset": raw }))
    })
}

/// Parse an optional [`CanvasResizeMode`] from string.
pub(super) fn parse_canvas_resize_mode(raw: Option<&str>) -> Result<CanvasResizeMode, EngineError> {
    let Some(s) = raw else {
        return Ok(CanvasResizeMode::default());
    };
    CanvasResizeMode::from_name(s).ok_or_else(|| {
        EngineError::with_code(ErrorCode::BadInput, format!("unknown canvas resize mode `{s}`"))
            .with_remediation("Use `KeepTypographyPt` or `ProportionalScale`.")
            .with_details(json!({ "pointer": "/resize_mode", "resize_mode": s }))
    })
}

/// Compute a safe, finite, positive scale multiplier between `old_val` and `new_val`.
pub(super) fn safe_scale_ratio(old_val: f64, new_val: f64) -> Result<f64, EngineError> {
    if !old_val.is_finite() || old_val <= 0.0 || !new_val.is_finite() || new_val <= 0.0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "canvas dimensions must be finite and positive before applying canvas preset"));
    }
    let ratio = new_val / old_val;
    if !ratio.is_finite() || ratio <= 0.0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "canvas preset resize scale factor overflowed finite floating-point range"));
    }
    Ok(ratio)
}

/// Multiply `value` by `factor`, returning `BadInput` if the product overflows to non-finite.
pub(super) fn scale_finite(value: f64, factor: f64, field: &str) -> Result<f64, EngineError> {
    let scaled = value * factor;
    if !scaled.is_finite() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("canvas resize caused non-finite overflow on `{field}`")));
    }
    Ok(scaled)
}

/// Scale a [`RectPt`] independently along horizontal and vertical axes, rejecting overflow.
pub(super) fn scale_rect_pt(rect: RectPt, sx: f64, sy: f64) -> Result<RectPt, EngineError> {
    let x = scale_finite(rect.x, sx, "rect.x")?;
    let y = scale_finite(rect.y, sy, "rect.y")?;
    let width = scale_finite(rect.width, sx, "rect.width")?;
    let height = scale_finite(rect.height, sy, "rect.height")?;
    Ok(RectPt::new(x, y, width, height))
}

/// Shared schema for `graph.applyCanvasPreset` and `layout.applyCanvasPreset`.
pub(super) fn schema_apply_canvas_preset() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target graph or layout sheet ID")),
            (
                "preset",
                non_empty_str_schema(
                    "Canvas preset (`SingleColumn88mm`, `ColumnAndHalf130mm`, `DoubleColumn180mm`, `FullPagePortrait`, `Slide16x9`, `PosterPanel`)",
                ),
            ),
            ("resize_mode", nullable_schema(non_empty_str_schema("Resize mode (`KeepTypographyPt` or `ProportionalScale`)"))),
        ],
        &["sheet_id", "preset"],
        vec![("sheet_id", &["id"]), ("resize_mode", &["mode"])],
        false,
    )
}

// ---------------------------------------------------------------------------
// Schemas & Typed Parameter Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GraphCreateParams {
    #[serde(alias = "source_sheet_id", alias = "source_sheet")]
    data_sheet_id: String,
    #[serde(alias = "id")]
    sheet_id: Option<String>,
    title: Option<String>,
    kind: Option<Value>,
    initialized: Option<bool>,
}

fn schema_graph_create() -> Value {
    obj_schema_with_aliases(
        vec![
            ("data_sheet_id", non_empty_str_schema("Source data sheet ID to plot")),
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional explicit graph sheet ID"))),
            ("title", nullable_schema(str_schema("Optional graph title"))),
            ("kind", nullable_schema(any_json_schema("Optional initial graph kind string or GraphKind object"))),
            ("initialized", nullable_schema(bool_schema("Whether the graph starts marked as initialized"))),
        ],
        &["data_sheet_id"],
        vec![("data_sheet_id", &["source_sheet_id", "source_sheet"]), ("sheet_id", &["id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphInitializeParams {
    #[serde(alias = "id")]
    sheet_id: String,
    kind: Option<Value>,
    title: Option<String>,
}

fn schema_graph_initialize() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to initialize")),
            ("kind", nullable_schema(any_json_schema("Optional graph kind string or GraphKind object"))),
            ("title", nullable_schema(str_schema("Optional graph title override"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetKindParams {
    #[serde(alias = "id")]
    sheet_id: String,
    kind: Value,
    reset_series_style: Option<bool>,
    #[serde(default)]
    option_id: Option<String>,
}

fn schema_graph_set_kind() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("kind", any_json_schema("Replacement graph kind string or typed GraphKind object")),
            ("reset_series_style", nullable_schema(bool_schema("Reinitialize series GDOs to defaults for the new kind (default true)"))),
            ("option_id", nullable_schema(str_schema("Optional UI graph type catalog identifier"))),
        ],
        &["sheet_id", "kind"],
        vec![("sheet_id", &["id"])],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetSpecParams {
    #[serde(alias = "id")]
    sheet_id: String,
    spec: Option<Value>,
    title: Option<String>,
    initialized: Option<bool>,
    survival_number_at_risk: Option<bool>,
    multivariate: Option<Value>,
    #[serde(flatten)]
    patch_fields: Map<String, Value>,
}

fn schema_graph_set_spec() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("spec", nullable_schema(json!({ "type": "object", "description": "Partial or complete GraphSpec JSON patch" }))),
            ("title", nullable_schema(str_schema("Optional updated graph title"))),
            ("initialized", nullable_schema(bool_schema("Optional initialized state"))),
            ("survival_number_at_risk", nullable_schema(bool_schema("Show number-at-risk table under survival plots"))),
            ("multivariate", nullable_schema(json!({ "type": "object", "description": "5-channel multivariate encoding specification" }))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetAxesParams {
    #[serde(alias = "id")]
    sheet_id: String,
    frame_preset: Option<Value>,
    x_axis: Option<Value>,
    y_axis: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    y2_axis: Option<Option<Value>>,
    #[serde(alias = "which")]
    axis: Option<String>,
    #[serde(flatten)]
    axis_fields: Map<String, Value>,
}

fn schema_graph_set_axes() -> Value {
    let axis_obj = nullable_schema(
        json!({ "type": "object", "description": "Axis specification fields (title, segments, scale, min, max, tick_format, additional_ticks, etc.)" }),
    );
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("frame_preset", nullable_schema(any_json_schema("Axis frame preset (`open_l`, `offset`, `plain_boxed`, `dual_y`, `origin_crossing`)"))),
            (
                "axis",
                nullable_schema(non_empty_str_schema(
                    "Single axis shorthand target (`x`, `y`, or `y2`) when specifying axis properties at top level",
                )),
            ),
            ("x_axis", axis_obj.clone()),
            ("y_axis", axis_obj.clone()),
            ("y2_axis", axis_obj),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"]), ("axis", &["which"])],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct AdditionalTickParam {
    value: f64,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    draw_gridline: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GraphSetAdditionalTicksParams {
    #[serde(alias = "id", alias = "graph_sheet_id")]
    sheet_id: String,
    #[serde(alias = "which")]
    axis: String,
    #[serde(alias = "additional_ticks")]
    ticks: Vec<AdditionalTickParam>,
}

fn schema_graph_set_additional_ticks() -> Value {
    let tick_schema = json!({
        "type": "object",
        "properties": {
            "value": { "type": "number", "description": "Finite numeric coordinate on the axis" },
            "label": { "anyOf": [{ "type": "string" }, { "type": "null" }], "description": "Optional override tick label" },
            "draw_gridline": { "anyOf": [{ "type": "boolean" }, { "type": "null" }], "description": "Whether to draw a gridline at this tick" }
        },
        "required": ["value"],
        "additionalProperties": false
    });
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("axis", non_empty_str_schema("Target axis (`x`, `y`, or `y2`)")),
            ("ticks", array_schema("Ordered custom ticks along the target axis", tick_schema, None, None)),
        ],
        &["sheet_id", "axis", "ticks"],
        vec![("sheet_id", &["id", "graph_sheet_id"]), ("axis", &["which"]), ("ticks", &["additional_ticks"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetSeriesParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(default, deserialize_with = "de_opt_usize")]
    series_index: Option<usize>,
    #[serde(default, deserialize_with = "de_opt_usize")]
    data_set_index: Option<usize>,
    series: Option<Vec<Value>>,
    multivariate: Option<Value>,
    #[serde(flatten)]
    patch_fields: Map<String, Value>,
}

fn schema_graph_set_series() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("series_index", nullable_schema(int_schema("Zero-based series slot index", Some(0), None))),
            ("data_set_index", nullable_schema(int_schema("Data set column index to match against bound series", Some(0), None))),
            ("series", nullable_schema(array_schema("Batch series patch array", json!({ "type": "object" }), None, None))),
            ("multivariate", nullable_schema(json!({ "type": "object", "description": "Multivariate 5-channel bubble/scatter encoding" }))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetRegressionOverlayParams {
    #[serde(alias = "id")]
    sheet_id: String,
    clear: Option<bool>,
    #[serde(alias = "result_sheet")]
    result_sheet_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_usize")]
    series_index: Option<usize>,
    #[serde(default, deserialize_with = "de_opt_usize")]
    data_set_index: Option<usize>,
    all_series: Option<bool>,
    #[serde(default, deserialize_with = "de_opt_u16")]
    sample_segments: Option<u16>,
    show_confidence_band: Option<bool>,
    show_prediction_band: Option<bool>,
    shaded_ribbon: Option<bool>,
}

fn schema_graph_set_regression_overlay() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("clear", nullable_schema(bool_schema("Clear existing regression overlay instead of attaching one"))),
            ("result_sheet_id", nullable_schema(non_empty_str_schema("Result sheet ID supplying fitted curves and bands"))),
            ("series_index", nullable_schema(int_schema("Target series index", Some(0), None))),
            ("data_set_index", nullable_schema(int_schema("Target bound dataset index", Some(0), None))),
            ("all_series", nullable_schema(bool_schema("Apply overlay to all series on the graph"))),
            ("sample_segments", nullable_schema(int_schema("Curve subdivision density (clamped to 256..=1024)", Some(1), Some(i64::from(u16::MAX))))),
            ("show_confidence_band", nullable_schema(bool_schema("Render 95% confidence band"))),
            ("show_prediction_band", nullable_schema(bool_schema("Render 95% prediction band"))),
            ("shaded_ribbon", nullable_schema(bool_schema("Fill confidence/prediction bands as translucent ribbons"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"]), ("result_sheet_id", &["result_sheet"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphAddComparisonBracketsParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(alias = "result_sheet")]
    result_sheet_id: String,
    group_a: Option<Value>,
    group_b: Option<Value>,
    significant_only: Option<bool>,
    clear_existing: Option<bool>,
    style: Option<String>,
    label_mode: Option<Value>,
    hide_if_non_significant: Option<bool>,
    stroke_width_pt: Option<f64>,
    font_size_pt: Option<f64>,
    bracket_step_pt: Option<f64>,
}

fn schema_graph_add_comparison_brackets() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID")),
            ("result_sheet_id", non_empty_str_schema("Result sheet ID with pairwise comparison records")),
            ("group_a", nullable_schema(any_json_schema("Optional first group index or label"))),
            ("group_b", nullable_schema(any_json_schema("Optional second group index or label"))),
            ("significant_only", nullable_schema(bool_schema("Skip non-significant comparisons when adding all pairs"))),
            ("clear_existing", nullable_schema(bool_schema("Remove existing brackets before appending new ones"))),
            ("style", nullable_schema(str_schema("Bracket shape (`square`, `half_tick`, `flat_line`, `curly`)"))),
            ("label_mode", nullable_schema(any_json_schema("Label format (`stars`, `numeric`, or custom string)"))),
            ("hide_if_non_significant", nullable_schema(bool_schema("Hide bracket automatically if P exceeds alpha"))),
            ("stroke_width_pt", nullable_schema(num_schema("Stroke width in points", Some(0.0), None))),
            ("font_size_pt", nullable_schema(num_schema("Label font size in points", Some(1.0), None))),
            ("bracket_step_pt", nullable_schema(num_schema("Vertical step spacing between stacked brackets in points", Some(1.0), None))),
        ],
        &["sheet_id", "result_sheet_id"],
        vec![("sheet_id", &["id"]), ("result_sheet_id", &["result_sheet"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphAddAnnotationParams {
    #[serde(alias = "id")]
    sheet_id: String,
    annotation: Option<Value>,
    #[serde(flatten)]
    fields: Map<String, Value>,
}

fn schema_graph_add_annotation() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID")),
            ("annotation", nullable_schema(json!({ "type": "object", "description": "Full serialized GraphAnnotation object" }))),
            (
                "kind",
                nullable_schema(non_empty_str_schema(
                    "Annotation shorthand kind (`text`, `line`, `arrow`, `shape`, `table`, `equation` / `model_equation`)",
                )),
            ),
            ("text", nullable_schema(str_schema("Text or markdown body for text annotations"))),
            ("bounds_pt", nullable_schema(any_json_schema("Bounding rectangle [x, y, w, h] or RectPt object"))),
            ("result_sheet_id", nullable_schema(non_empty_str_schema("Result sheet ID for embedded table or model equation annotations"))),
            (
                "data_set_index",
                nullable_schema(int_schema("Zero-based fitted dataset curve index for model equation annotations (default 0)", Some(0), None)),
            ),
            (
                "mode",
                nullable_schema(enum_str_schema(
                    "Equation rendering mode for model equation annotations (`substituted` or `symbolic`, default `substituted`)",
                    &["substituted", "symbolic"],
                )),
            ),
            (
                "font_size_pt",
                nullable_schema(num_schema("Font size in points for model equation annotations (default 10.0)", Some(1.0), Some(144.0))),
            ),
            ("bordered", nullable_schema(bool_schema("Whether to draw a border around text box or model equation annotations"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct GraphApplyStyleFromParams {
    #[serde(alias = "target_sheet_id", alias = "id")]
    sheet_id: String,
    #[serde(alias = "from_sheet_id", alias = "source_graph_id")]
    donor_sheet_id: String,
    #[serde(default)]
    clone_palette: Option<bool>,
    #[serde(default)]
    clone_frame_and_axes: Option<bool>,
    #[serde(default)]
    clone_error_bars_and_legend: Option<bool>,
}

fn schema_graph_apply_style_from() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target graph sheet ID")),
            ("donor_sheet_id", non_empty_str_schema("Donor graph sheet ID to copy styling from")),
            ("clone_palette", nullable_schema(bool_schema("Copy color palette and series colors"))),
            ("clone_frame_and_axes", nullable_schema(bool_schema("Copy frame and axis styling"))),
            ("clone_error_bars_and_legend", nullable_schema(bool_schema("Copy error bar and legend settings"))),
        ],
        &["sheet_id", "donor_sheet_id"],
        vec![("sheet_id", &["target_sheet_id", "id"]), ("donor_sheet_id", &["from_sheet_id", "source_graph_id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ApplyCanvasPresetParams {
    #[serde(alias = "id")]
    sheet_id: String,
    preset: String,
    #[serde(alias = "mode")]
    resize_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PreflightCheckParams {
    #[serde(alias = "id")]
    sheet_id: String,
}

fn schema_graph_preflight_check() -> Value {
    obj_schema_with_aliases(
        vec![("sheet_id", non_empty_str_schema("Graph or layout sheet ID to audit"))],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

fn schema_graph_format_selected_points() -> Value {
    let coord_schema = obj_schema(
        vec![
            ("series_index", int_schema("Zero-based series index", Some(0), None)),
            ("point_index", int_schema("Zero-based data point index within the series", Some(0), None)),
            ("subcol", nullable_schema(int_schema("Optional zero-based replicate subcolumn index", Some(0), None))),
            ("subcolumn", nullable_schema(int_schema("Alias for `subcol`", Some(0), None))),
            ("replicate", nullable_schema(int_schema("Alias for `subcol`", Some(0), None))),
        ],
        &["series_index", "point_index"],
        false,
    );
    let override_item_schema = obj_schema(
        vec![
            ("series_index", int_schema("Zero-based series index", Some(0), None)),
            ("point_index", int_schema("Zero-based data point index within the series", Some(0), None)),
            ("subcol", nullable_schema(int_schema("Optional zero-based replicate subcolumn index", Some(0), None))),
            ("subcolumn", nullable_schema(int_schema("Alias for `subcol`", Some(0), None))),
            ("replicate", nullable_schema(int_schema("Alias for `subcol`", Some(0), None))),
            ("color", nullable_schema(any_json_schema("Override accent/stroke color (hex, color name, [r,g,b], [r,g,b,a], or null to clear)"))),
            (
                "symbol",
                nullable_schema(str_schema(
                    "Override marker shape (circle, square, triangle_up, triangle_down, diamond, cross, plus, star, hexagon, or null)",
                )),
            ),
            ("shape", nullable_schema(str_schema("Alias for `symbol`"))),
            ("size_pt", nullable_schema(num_schema("Override positive marker size in points (> 0, or null to clear)", None, None))),
            ("border_width_pt", nullable_schema(num_schema("Override marker border width in points (>= 0, or null to clear)", None, None))),
            ("stroke_width_pt", nullable_schema(num_schema("Alias for `border_width_pt`", None, None))),
            ("fill_color", nullable_schema(any_json_schema("Override marker fill color (or null to clear)"))),
            (
                "fill_mode",
                nullable_schema(str_schema("Override marker fill mode (filled, open, semi_top, semi_bottom, semi_left, semi_right, or null)")),
            ),
            ("hidden", bool_schema("True hides this point marker while leaving data intact")),
        ],
        &["series_index", "point_index"],
        false,
    );

    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target graph sheet ID")),
            (
                "mode",
                enum_str_schema(
                    "Override update mode: merge (default, updates provided fields), replace (replaces full override table), or clear",
                    &["merge", "replace", "clear"],
                ),
            ),
            ("clear", bool_schema("Convenience flag: true selects clear mode when `mode` is omitted")),
            ("overrides", array_schema("Explicit per-point override records", override_item_schema, None, None)),
            ("points", array_schema("Target (series_index, point_index) points to format uniformly or clear", coord_schema, None, None)),
            ("color", nullable_schema(any_json_schema("Shared stroke/accent color for batch `points` (or null to clear)"))),
            ("symbol", nullable_schema(str_schema("Shared marker shape for batch `points` (or null to clear)"))),
            ("shape", nullable_schema(str_schema("Alias for `symbol`"))),
            ("size_pt", nullable_schema(num_schema("Shared positive marker size in points for batch `points` (or null to clear)", None, None))),
            (
                "border_width_pt",
                nullable_schema(num_schema("Shared marker border width in points for batch `points` (or null to clear)", None, None)),
            ),
            ("stroke_width_pt", nullable_schema(num_schema("Alias for `border_width_pt`", None, None))),
            ("fill_color", nullable_schema(any_json_schema("Shared marker fill color for batch `points` (or null to clear)"))),
            ("fill_mode", nullable_schema(str_schema("Shared marker fill mode for batch `points` (or null to clear)"))),
            ("hidden", bool_schema("Shared hidden flag applied across batch `points`")),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id", "graph_sheet_id"]), ("symbol", &["shape"]), ("border_width_pt", &["stroke_width_pt"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetCompactLetterDisplayParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(alias = "result_sheet")]
    result_sheet_id: Option<String>,
    #[serde(alias = "display_mode")]
    pairwise_display_mode: Option<String>,
    ordering: Option<String>,
    uppercase: Option<bool>,
    #[serde(default, deserialize_with = "de_present_opt")]
    alpha_override: Option<Option<f64>>,
    font_size_pt: Option<f64>,
    offset_pt: Option<f64>,
    color: Option<Value>,
}

fn schema_graph_set_compact_letter_display() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID")),
            ("enabled", nullable_schema(bool_schema("Enable (true) or clear (false) compact letter display annotation"))),
            ("result_sheet_id", nullable_schema(non_empty_str_schema("Result sheet ID supplying all-pairwise adjusted p/q comparisons"))),
            (
                "pairwise_display_mode",
                nullable_schema(str_schema(
                    "Pairwise display mode (`brackets_only`, `compact_letter_display_only` / `cld_only`, or `brackets_and_compact_letter_display` / `both`)",
                )),
            ),
            ("ordering", nullable_schema(str_schema("Monotonic letter assignment direction (`descending_mean` or `ascending_mean`)"))),
            ("uppercase", nullable_schema(bool_schema("Use uppercase letters (A, B, AB) instead of lowercase (a, b, ab)"))),
            ("alpha_override", nullable_schema(num_schema("Optional explicit significance alpha in (0, 1) overriding result decisions", None, None))),
            ("font_size_pt", nullable_schema(num_schema("Letter font size in points in (0, 144]", Some(1.0), Some(144.0)))),
            ("offset_pt", nullable_schema(num_schema("Vertical clearance above plotted elements in points in [0, 200]", Some(0.0), Some(200.0)))),
            ("color", nullable_schema(any_json_schema("Text color (`#RRGGBB`, `#RRGGBBAA`, color name, or `[r, g, b]` / `[r, g, b, a]`)"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"]), ("result_sheet_id", &["result_sheet"]), ("pairwise_display_mode", &["display_mode"])],
        false,
    )
}

// ---------------------------------------------------------------------------
// Command Runners
// ---------------------------------------------------------------------------

fn resolve_graph_create_source_sheet(
    doc: &ProjectDocument,
    source_id: &curvebook_doc::SheetId,
) -> Result<(curvebook_doc::DataSheet, Option<String>), EngineError> {
    if let Some(ds) = doc.data_sheet(source_id) {
        return Ok((ds.clone(), None));
    }
    let Some(initial_res) = doc.result_sheet(source_id) else {
        return Err(curvebook_doc::DocError::SheetNotFound { sheet_id: source_id.clone() }.into());
    };
    if let Some(ref analyzable) = initial_res.analyzable_data {
        return Ok((analyzable.clone(), None));
    }
    let res_title = initial_res.title().to_owned();
    let mut cursor = initial_res.analysis.source_sheet_id.clone();
    let mut visited = std::collections::HashSet::new();
    visited.insert(source_id.clone());
    while visited.insert(cursor.clone()) {
        if let Some(ds) = doc.data_sheet(&cursor) {
            return Ok((ds.clone(), Some(res_title)));
        }
        if let Some(res) = doc.result_sheet(&cursor) {
            if let Some(ref analyzable) = res.analyzable_data {
                return Ok((analyzable.clone(), Some(res_title)));
            }
            cursor = res.analysis.source_sheet_id.clone();
        } else {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("result sheet `{source_id}` has no upstream data sheet (source `{cursor}` is not a data or result sheet)"),
            ));
        }
    }
    Err(EngineError::with_code(ErrorCode::CycleDetected, format!("cyclic dependency detected while resolving upstream data sheet for `{source_id}`")))
}

fn run_graph_create(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_create();
    let params: GraphCreateParams = parse_params(&schema, raw)?;
    let doc = &mut ctx.session.doc;

    let data_id = parse_sheet_id(&params.data_sheet_id)?;
    let (ds, res_title_opt) = resolve_graph_create_source_sheet(doc, &data_id)?;
    let id = match params.sheet_id.as_deref() {
        Some(raw_id) => parse_sheet_id(raw_id)?,
        None => next_unique_sheet_id(doc, "graph")?,
    };

    let mut graph = GraphSheet::new_uninitialized_for_data(id.clone(), &ds)?;
    if params.title.is_none()
        && let Some(ref res_title) = res_title_opt
    {
        graph.title = res_title.clone();
        graph.spec.title = res_title.clone();
    }
    graph.spec.primary_source_sheet_id = data_id.clone();
    for series in &mut graph.spec.series {
        series.source_sheet_id = data_id.clone();
    }
    if let Some(ref title) = params.title {
        curvebook_doc::validate_title(title)?;
        graph.title = title.clone();
    }
    if let Some(ref kind_val) = params.kind {
        let kind = parse_graph_kind(kind_val)?;
        apply_graph_kind(&mut graph.spec, kind, true);
        graph.initialized = params.initialized.unwrap_or(true);
    } else if let Some(init) = params.initialized {
        graph.initialized = init;
    }
    curvebook_doc::validate_title(&graph.title)?;
    validate_graph_spec_in_doc(doc, &graph.id, &graph.spec)?;

    let title = graph.title.clone();
    let initialized = graph.initialized;
    let kind = graph.spec.kind;
    doc.add_graph_sheet(graph)?;

    Ok(CommandOutcome::mutated(
        "graph.create",
        format!("created graph `{id}`"),
        json!({ "sheet_id": id.as_str(), "data_sheet_id": data_id.as_str(), "title": title, "initialized": initialized, "kind": kind }),
    ))
}

fn run_graph_initialize(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_initialize();
    let params: GraphInitializeParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let kind_opt = params.kind.as_ref().map(parse_graph_kind).transpose()?;
    if let Some(ref title) = params.title {
        curvebook_doc::validate_title(title)?;
    }

    let mut updated_spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();
    if let Some(kind) = kind_opt {
        apply_graph_kind(&mut updated_spec, kind, true);
    }
    validate_graph_spec_in_doc(ctx.session.document(), &id, &updated_spec)?;

    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.initialized = true;
    if let Some(title) = params.title {
        graph.title = title;
    }
    graph.spec = updated_spec;

    Ok(CommandOutcome::mutated(
        "graph.initialize",
        format!("initialized graph `{id}`"),
        json!({ "sheet_id": id.as_str(), "kind": graph.spec.kind, "initialized": true }),
    ))
}

#[derive(Debug, Clone, Copy, Default)]
enum OptionalPatch<T> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

#[derive(Debug, Clone, Default)]
struct PointStylePatch {
    color: OptionalPatch<Rgba8>,
    symbol: OptionalPatch<MarkerShape>,
    size_pt: OptionalPatch<f64>,
    border_width_pt: OptionalPatch<f64>,
    fill_color: OptionalPatch<Rgba8>,
    fill_mode: OptionalPatch<MarkerFillMode>,
    hidden: Option<bool>,
}

impl PointStylePatch {
    fn has_any_change(&self) -> bool {
        !matches!(self.color, OptionalPatch::Keep)
            || !matches!(self.symbol, OptionalPatch::Keep)
            || !matches!(self.size_pt, OptionalPatch::Keep)
            || !matches!(self.border_width_pt, OptionalPatch::Keep)
            || !matches!(self.fill_color, OptionalPatch::Keep)
            || !matches!(self.fill_mode, OptionalPatch::Keep)
            || self.hidden.is_some()
    }
}

fn parse_optional_patch<T>(
    obj: &Map<String, Value>,
    keys: &[&str],
    parser: fn(&Value) -> Result<T, EngineError>,
) -> Result<OptionalPatch<T>, EngineError> {
    for key in keys {
        if let Some(val) = obj.get(*key) {
            if val.is_null() {
                return Ok(OptionalPatch::Clear);
            }
            return Ok(OptionalPatch::Set(parser(val)?));
        }
    }
    Ok(OptionalPatch::Keep)
}

fn parse_positive_f64(val: &Value) -> Result<f64, EngineError> {
    let v = val.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`size_pt` must be a finite number"))?;
    if !v.is_finite() || v <= 0.0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("point `size_pt` must be finite and > 0 (got {v})")));
    }
    Ok(v)
}

fn parse_non_negative_f64(val: &Value) -> Result<f64, EngineError> {
    let v = val.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`border_width_pt` must be a finite number"))?;
    if !v.is_finite() || v < 0.0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("point `border_width_pt` must be finite and >= 0 (got {v})")));
    }
    Ok(v)
}

fn parse_point_style_patch(obj: &Map<String, Value>) -> Result<PointStylePatch, EngineError> {
    Ok(PointStylePatch {
        color: parse_optional_patch(obj, &["color"], parse_rgba8)?,
        symbol: parse_optional_patch(obj, &["symbol", "shape"], parse_marker_shape)?,
        size_pt: parse_optional_patch(obj, &["size_pt"], parse_positive_f64)?,
        border_width_pt: parse_optional_patch(obj, &["border_width_pt", "stroke_width_pt"], parse_non_negative_f64)?,
        fill_color: parse_optional_patch(obj, &["fill_color"], parse_rgba8)?,
        fill_mode: parse_optional_patch(obj, &["fill_mode"], parse_marker_fill_mode)?,
        hidden: obj.get("hidden").and_then(Value::as_bool),
    })
}

fn parse_opt_subcol(obj: &Map<String, Value>) -> Result<Option<usize>, EngineError> {
    let Some(val) = obj.get("subcol").or_else(|| obj.get("subcolumn")).or_else(|| obj.get("replicate")) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let raw = val.as_u64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`subcol` must be a non-negative integer or null"))?;
    let sc = usize::try_from(raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "`subcol` out of range"))?;
    Ok(Some(sc))
}

fn parse_point_coord(obj: &Map<String, Value>) -> Result<(usize, usize, Option<usize>), EngineError> {
    let s_raw = obj
        .get("series_index")
        .and_then(Value::as_u64)
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "point entry requires non-negative `series_index`"))?;
    let p_raw = obj
        .get("point_index")
        .and_then(Value::as_u64)
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "point entry requires non-negative `point_index`"))?;
    let series_index = usize::try_from(s_raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "`series_index` out of range"))?;
    let point_index = usize::try_from(p_raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "`point_index` out of range"))?;
    let subcol = parse_opt_subcol(obj)?;
    Ok((series_index, point_index, subcol))
}

fn apply_optional_patch<T>(slot: &mut Option<T>, patch: &OptionalPatch<T>)
where
    T: Clone,
{
    match patch {
        OptionalPatch::Keep => {}
        OptionalPatch::Clear => *slot = None,
        OptionalPatch::Set(v) => *slot = Some(v.clone()),
    }
}

fn apply_point_style_patch(ov: &mut PointStyleOverride, patch: &PointStylePatch) {
    apply_optional_patch(&mut ov.color, &patch.color);
    apply_optional_patch(&mut ov.symbol, &patch.symbol);
    apply_optional_patch(&mut ov.size_pt, &patch.size_pt);
    apply_optional_patch(&mut ov.border_width_pt, &patch.border_width_pt);
    apply_optional_patch(&mut ov.fill_color, &patch.fill_color);
    apply_optional_patch(&mut ov.fill_mode, &patch.fill_mode);
    if let Some(hidden) = patch.hidden {
        ov.hidden = hidden;
    }
}

fn is_empty_point_override(ov: &PointStyleOverride) -> bool {
    ov.color.is_none()
        && ov.symbol.is_none()
        && ov.size_pt.is_none()
        && ov.border_width_pt.is_none()
        && ov.fill_color.is_none()
        && ov.fill_mode.is_none()
        && !ov.hidden
}

type ParsedOverrideEntry = (usize, usize, Option<usize>, PointStylePatch);

fn parse_override_entries(validated: &Map<String, Value>) -> Result<Vec<ParsedOverrideEntry>, EngineError> {
    let mut entries = Vec::new();
    if let Some(arr) = validated.get("overrides").and_then(Value::as_array) {
        for item in arr {
            let Some(obj) = item.as_object() else {
                return Err(EngineError::with_code(ErrorCode::BadInput, "each `overrides` entry must be an object"));
            };
            let (series_index, point_index, subcol) = parse_point_coord(obj)?;
            let patch = parse_point_style_patch(obj)?;
            entries.push((series_index, point_index, subcol, patch));
        }
    }
    if let Some(arr) = validated.get("points").and_then(Value::as_array) {
        let shared = parse_point_style_patch(validated)?;
        for item in arr {
            let Some(obj) = item.as_object() else {
                return Err(EngineError::with_code(ErrorCode::BadInput, "each `points` entry must be an object"));
            };
            let (series_index, point_index, subcol) = parse_point_coord(obj)?;
            entries.push((series_index, point_index, subcol, shared.clone()));
        }
    }
    Ok(entries)
}

fn run_graph_format_selected_points(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_format_selected_points();
    let validated: Value = parse_params(&schema, raw)?;
    let obj = validated.as_object().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "parameters must be a JSON object"))?;

    let sheet_raw = obj.get("sheet_id").and_then(Value::as_str).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing `sheet_id`"))?;
    let id = parse_sheet_id(sheet_raw)?;

    let mode = match obj.get("mode").and_then(Value::as_str) {
        Some(m) => m,
        None if obj.get("clear").and_then(Value::as_bool) == Some(true) => "clear",
        None => "merge",
    };

    let entries = parse_override_entries(obj)?;
    if mode != "clear" && entries.is_empty() {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "graph.formatSelectedPoints requires non-empty `overrides` or `points` unless `mode` is `clear`",
        ));
    }
    if mode != "clear" && entries.iter().all(|(_, _, _, p)| !p.has_any_change()) {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "graph.formatSelectedPoints requires at least one style attribute (`color`, `symbol`, `size_pt`, `border_width_pt`, `fill_color`, `fill_mode`, or `hidden`)",
        ));
    }

    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    if mode == "clear" {
        if entries.is_empty() {
            graph.spec.point_overrides.clear();
        } else {
            graph.spec.point_overrides.retain(|existing| {
                !entries
                    .iter()
                    .any(|(s, p, sc, _)| existing.series_index == *s && existing.point_index == *p && (sc.is_none() || existing.subcol == *sc))
            });
        }
    } else {
        if mode == "replace" {
            graph.spec.point_overrides.clear();
        }
        for (series_index, point_index, subcol, patch) in entries {
            if let Some(existing) = graph
                .spec
                .point_overrides
                .iter_mut()
                .find(|ov| ov.series_index == series_index && ov.point_index == point_index && ov.subcol == subcol)
            {
                apply_point_style_patch(existing, &patch);
            } else {
                let mut ov = PointStyleOverride { series_index, point_index, subcol, ..PointStyleOverride::default() };
                apply_point_style_patch(&mut ov, &patch);
                graph.spec.point_overrides.push(ov);
            }
        }
    }

    graph.spec.point_overrides.retain(|ov| !is_empty_point_override(ov));
    graph.spec.point_overrides.sort_by_key(|ov| (ov.series_index, ov.point_index, ov.subcol));
    graph.spec.validate()?;
    let override_count = graph.spec.point_overrides.len();

    Ok(CommandOutcome::mutated(
        "graph.formatSelectedPoints",
        format!("updated {override_count} point override(s) on `{id}`"),
        json!({
            "sheet_id": id.as_str(),
            "mode": mode,
            "override_count": override_count,
            "point_overrides": graph.spec.point_overrides,
        }),
    ))
}

fn validate_source_sheet_ref(
    doc: &curvebook_doc::ProjectDocument,
    graph_id: &curvebook_doc::SheetId,
    source_id: &curvebook_doc::SheetId,
    role: &str,
) -> Result<(), EngineError> {
    if source_id == graph_id {
        return Err(EngineError::with_code(
            ErrorCode::CycleDetected,
            format!("self-referential {role} `{source_id}` is not allowed on graph `{graph_id}`"),
        ));
    }
    match doc.sheet_section(source_id) {
        Some(curvebook_doc::SheetSection::Data | curvebook_doc::SheetSection::Analyses) => Ok(()),
        Some(_) => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("{role} `{source_id}` on graph `{graph_id}` must reference a data or result sheet"),
        )),
        None => Err(curvebook_doc::DocError::SheetNotFound { sheet_id: source_id.clone() }.into()),
    }
}

fn validate_result_sheet_ref(
    doc: &curvebook_doc::ProjectDocument,
    graph_id: &curvebook_doc::SheetId,
    result_id: &curvebook_doc::SheetId,
    role: &str,
) -> Result<(), EngineError> {
    if result_id == graph_id {
        return Err(EngineError::with_code(
            ErrorCode::CycleDetected,
            format!("self-referential {role} `{result_id}` is not allowed on graph `{graph_id}`"),
        ));
    }
    let _ = crate::cmd::helpers::require_result_sheet(doc, result_id)?;
    Ok(())
}

fn validate_graph_spec_in_doc(doc: &curvebook_doc::ProjectDocument, graph_id: &curvebook_doc::SheetId, spec: &GraphSpec) -> Result<(), EngineError> {
    spec.validate()?;
    validate_source_sheet_ref(doc, graph_id, &spec.primary_source_sheet_id, "primary_source_sheet_id")?;
    for (idx, series) in spec.series.iter().enumerate() {
        validate_source_sheet_ref(doc, graph_id, &series.source_sheet_id, &format!("series[{idx}].source_sheet_id"))?;
        if let Some(ov) = &series.regression_overlay {
            validate_result_sheet_ref(doc, graph_id, &ov.result_sheet_id, &format!("series[{idx}].regression_overlay.result_sheet_id"))?;
        }
    }
    for (idx, cmp) in spec.comparisons.iter().enumerate() {
        validate_result_sheet_ref(doc, graph_id, &cmp.result_sheet_id, &format!("comparisons[{idx}].result_sheet_id"))?;
    }
    if let Some(cld) = &spec.compact_letter_display {
        validate_result_sheet_ref(doc, graph_id, &cld.result_sheet_id, "compact_letter_display.result_sheet_id")?;
    }
    for (idx, ann) in spec.annotations.iter().enumerate() {
        if let GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } = ann {
            validate_result_sheet_ref(doc, graph_id, result_sheet_id, &format!("annotations[{idx}].result_sheet_id"))?;
        }
    }
    crate::dag::validate_inline_hook_refs(doc, graph_id, &spec.inline_hooks())?;
    crate::dag::validate_graph_spec_acyclic(doc, graph_id, spec)
}

fn run_graph_set_kind(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_set_kind();
    let params: GraphSetKindParams = parse_params(&schema, raw)?;
    let option_id = params.option_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let requested_kind = parse_graph_kind(&params.kind).ok();
    let resolved_opt = option_id.and_then(|opt| resolve_graph_option(opt, requested_kind));
    let (kind, opt_arrangement, gdo_preset) = match resolved_opt {
        Some(spec) => spec,
        None => (parse_graph_kind(&params.kind)?, None, None),
    };
    let id = parse_sheet_id(&params.sheet_id)?;
    let reset_series = params.reset_series_style.unwrap_or(true);

    let mut spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();
    apply_graph_kind_with_option(&mut spec, kind, opt_arrangement, gdo_preset, reset_series);
    validate_graph_spec_in_doc(ctx.session.document(), &id, &spec)?;
    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.initialized = true;
    graph.spec = spec;

    let payload = match option_id {
        Some(opt) => json!({ "sheet_id": id.as_str(), "kind": kind, "option_id": opt }),
        None => json!({ "sheet_id": id.as_str(), "kind": kind }),
    };
    Ok(CommandOutcome::mutated("graph.setKind", format!("set graph `{id}` kind"), payload))
}

fn run_graph_set_spec(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_set_spec();
    let params: GraphSetSpecParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let current_spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();

    let patch =
        params.spec.clone().unwrap_or_else(|| strip_keys(&Value::Object(params.patch_fields.clone()), &["sheet_id", "id", "title", "initialized"]));
    let patch_clean = strip_keys(&patch, &["multivariate"]);
    let merged = merge_json_struct(&current_spec, &patch_clean)?;
    let mut new_spec: GraphSpec =
        serde_json::from_value(merged).map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid graph spec on `{id}`: {err}")))?;

    if let Some(mv_val) = patch.get("multivariate").or(params.multivariate.as_ref()) {
        new_spec.multivariate = Some(parse_multivariate_encoding(mv_val, current_spec.multivariate.as_ref())?);
    }
    if let Some(nar) = params.survival_number_at_risk {
        new_spec.survival_number_at_risk = nar;
    }
    if let Some(ref title) = params.title {
        curvebook_doc::validate_title(title)?;
    }
    validate_graph_spec_in_doc(ctx.session.document(), &id, &new_spec)?;

    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = new_spec;
    if let Some(title) = params.title {
        graph.title = title;
    }
    graph.initialized = params.initialized.unwrap_or(true);

    let kind = graph.spec.kind;
    let series_count = graph.spec.series.len();
    Ok(CommandOutcome::mutated(
        "graph.setSpec",
        format!("updated graph spec on `{id}`"),
        json!({ "sheet_id": id.as_str(), "kind": kind, "series_count": series_count }),
    ))
}

fn run_graph_set_axes(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_set_axes();
    let params: GraphSetAxesParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let mut spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();

    if let Some(ref fp) = params.frame_preset {
        spec.frame_preset = parse_frame_preset(fp)?;
    }
    if let Some(ref x_val) = params.x_axis {
        apply_axis_updates(&mut spec.x_axis, x_val)?;
    }
    if let Some(ref y_val) = params.y_axis {
        apply_axis_updates(&mut spec.y_axis, y_val)?;
    }
    if let Some(y2_present) = params.y2_axis.as_ref() {
        match y2_present {
            None => spec.y2_axis = None,
            Some(y2_val) => {
                let mut y2 = spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Right Y"));
                apply_axis_updates(&mut y2, y2_val)?;
                spec.y2_axis = Some(y2);
            }
        }
    }

    if let Some(ref which) = params.axis {
        let top_val = Value::Object(params.axis_fields);
        match which.trim().to_ascii_lowercase().as_str() {
            "x" | "x_axis" => apply_axis_updates(&mut spec.x_axis, &top_val)?,
            "y" | "y_axis" => apply_axis_updates(&mut spec.y_axis, &top_val)?,
            "y2" | "y2_axis" | "right_y" => {
                let mut y2 = spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Right Y"));
                apply_axis_updates(&mut y2, &top_val)?;
                spec.y2_axis = Some(y2);
            }
            other => {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown axis target `{other}`; expected `x`, `y`, or `y2`"))
                    .with_details(json!({ "pointer": "/axis" })));
            }
        }
    }

    validate_graph_spec_in_doc(ctx.session.document(), &id, &spec)?;
    let x_segs = spec.x_axis.segments.len();
    let y_segs = spec.y_axis.segments.len();
    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = spec;
    graph.initialized = true;

    Ok(CommandOutcome::mutated(
        "graph.setAxes",
        format!("updated axes on graph `{id}`"),
        json!({ "sheet_id": id.as_str(), "x_segments": x_segs, "y_segments": y_segs }),
    ))
}

fn run_graph_set_additional_ticks(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_set_additional_ticks();
    let params: GraphSetAdditionalTicksParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let mut spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();

    let mut ticks = Vec::with_capacity(params.ticks.len());
    for (idx, item) in params.ticks.into_iter().enumerate() {
        if !item.value.is_finite() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "additional tick `value` must be finite")
                .with_details(json!({ "pointer": format!("/ticks/{idx}/value") })));
        }
        ticks.push(AdditionalTick { value: item.value, label: item.label, draw_gridline: item.draw_gridline.unwrap_or(false) });
    }
    let tick_count = ticks.len();

    let norm_axis = match params.axis.trim().to_ascii_lowercase().as_str() {
        "x" | "x_axis" => {
            spec.x_axis.additional_ticks = ticks;
            "x"
        }
        "y" | "y_axis" => {
            spec.y_axis.additional_ticks = ticks;
            "y"
        }
        "y2" | "y2_axis" | "right_y" => {
            let mut y2 = spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Right Y"));
            y2.additional_ticks = ticks;
            spec.y2_axis = Some(y2);
            "y2"
        }
        other => {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown axis target `{other}`; expected `x`, `y`, or `y2`"))
                .with_details(json!({ "pointer": "/axis" })));
        }
    };

    spec.validate()?;
    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = spec;
    graph.initialized = true;

    Ok(CommandOutcome::mutated(
        "graph.setAdditionalTicks",
        format!("updated additional ticks on `{id}` ({norm_axis})"),
        json!({ "sheet_id": id.as_str(), "axis": norm_axis, "tick_count": tick_count }),
    ))
}

fn run_graph_set_series(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_set_series();
    let params: GraphSetSeriesParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let mut spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();

    if let Some(ref series_list) = params.series {
        let mut updated_vec = Vec::with_capacity(series_list.len());
        for (idx, item) in series_list.iter().enumerate() {
            let mut base = spec.series.get(idx).cloned().unwrap_or_else(|| default_series_like(&spec, idx));
            apply_single_series_patch(&mut base, item)?;
            updated_vec.push(base);
        }
        spec.series = updated_vec;
    } else if params.series_index.is_some() || params.data_set_index.is_some() || has_series_patch_keys(&params.patch_fields) {
        let target_idx = resolve_series_target_index(&spec, params.series_index, params.data_set_index)?;
        let patch_val = Value::Object(params.patch_fields);
        if let Some(s) = spec.series.get_mut(target_idx) {
            apply_single_series_patch(s, &patch_val)?;
        }
    }

    if let Some(ref mv_val) = params.multivariate {
        spec.multivariate = Some(parse_multivariate_encoding(mv_val, spec.multivariate.as_ref())?);
    }

    validate_graph_spec_in_doc(ctx.session.document(), &id, &spec)?;
    let count = spec.series.len();
    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = spec;
    graph.initialized = true;

    Ok(CommandOutcome::mutated(
        "graph.setSeries",
        format!("updated series styling on `{id}`"),
        json!({ "sheet_id": id.as_str(), "series_count": count }),
    ))
}

fn run_graph_set_regression_overlay(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_set_regression_overlay();
    let params: GraphSetRegressionOverlayParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let clear = params.clear.unwrap_or(false);

    let overlay = if clear {
        None
    } else {
        let res_raw = params.result_sheet_id.as_deref().ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "missing required parameter `result_sheet_id`")
                .with_remediation("Supply `result_sheet_id` of the curve fit result sheet or pass `clear: true`.")
                .with_details(json!({ "pointer": "/result_sheet_id" }))
        })?;
        let res_id = parse_sheet_id(res_raw)?;
        let _ = require_result_sheet(ctx.session.document(), &res_id)?;
        let sample_segments = params.sample_segments.unwrap_or(256).clamp(256, 1024);
        Some(RegressionOverlaySpec {
            result_sheet_id: res_id,
            sample_segments,
            show_confidence_band: params.show_confidence_band.unwrap_or(false),
            show_prediction_band: params.show_prediction_band.unwrap_or(false),
            shaded_ribbon: params.shaded_ribbon.unwrap_or(false),
        })
    };

    let mut spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();
    if spec.series.is_empty() {
        return Err(EngineError::with_code(ErrorCode::Conflict, format!("graph `{id}` has no series to bind a regression overlay")));
    }

    if params.all_series.unwrap_or(false) {
        for s in &mut spec.series {
            s.regression_overlay = overlay.clone();
        }
    } else {
        let idx = resolve_series_target_index(&spec, params.series_index, params.data_set_index)?;
        if let Some(s) = spec.series.get_mut(idx) {
            s.regression_overlay = overlay;
        }
    }

    validate_graph_spec_in_doc(ctx.session.document(), &id, &spec)?;
    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = spec;
    graph.initialized = true;

    Ok(CommandOutcome::mutated(
        "graph.setRegressionOverlay",
        format!("updated regression overlay on `{id}`"),
        json!({ "sheet_id": id.as_str(), "cleared": clear }),
    ))
}

fn run_graph_add_comparison_brackets(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_add_comparison_brackets();
    let params: GraphAddComparisonBracketsParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let res_id = parse_sheet_id(&params.result_sheet_id)?;
    let comparisons = require_result_sheet(ctx.session.document(), &res_id)?.pairwise_comparisons.clone();

    let mut spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();
    if params.clear_existing.unwrap_or(false) {
        spec.comparisons.clear();
    }

    let style = parse_bracket_style(params.style.as_deref())?;
    let label_mode = parse_bracket_label_mode(params.label_mode.as_ref())?;
    let hide_if_non_significant = params.hide_if_non_significant.unwrap_or(false);
    let stroke_width_pt = params.stroke_width_pt.unwrap_or(0.9);
    let font_size_pt = params.font_size_pt.unwrap_or(8.5);
    let bracket_step_pt = params.bracket_step_pt.unwrap_or(12.0);

    let added = if let (Some(ga_val), Some(gb_val)) = (params.group_a.as_ref(), params.group_b.as_ref()) {
        let ga = resolve_group_index(ga_val, &comparisons)?;
        let gb = resolve_group_index(gb_val, &comparisons)?;
        spec.comparisons.push(PairwiseBracketSpec {
            result_sheet_id: res_id,
            group_a: ga,
            group_b: gb,
            style,
            label_mode,
            hide_if_non_significant,
            stroke_width_pt,
            font_size_pt,
            bracket_step_pt,
        });
        1_usize
    } else {
        let significant_only = params.significant_only.unwrap_or(false);
        let mut count = 0_usize;
        for cmp in comparisons {
            if significant_only && !cmp.significant {
                continue;
            }
            spec.comparisons.push(PairwiseBracketSpec {
                result_sheet_id: res_id.clone(),
                group_a: cmp.group_a,
                group_b: cmp.group_b,
                style,
                label_mode: label_mode.clone(),
                hide_if_non_significant,
                stroke_width_pt,
                font_size_pt,
                bracket_step_pt,
            });
            count += 1;
        }
        count
    };

    validate_graph_spec_in_doc(ctx.session.document(), &id, &spec)?;
    let total = spec.comparisons.len();
    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = spec;
    graph.initialized = true;

    Ok(CommandOutcome::mutated(
        "graph.addComparisonBrackets",
        format!("added {added} comparison bracket(s) to `{id}`"),
        json!({ "sheet_id": id.as_str(), "added": added, "total_brackets": total }),
    ))
}

fn run_graph_add_annotation(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_add_annotation();
    let params: GraphAddAnnotationParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = ctx.session.document();
    let annotation = if let Some(ref ann_val) = params.annotation {
        let ann: GraphAnnotation = serde_json::from_value(ann_val.clone())
            .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `annotation`: {err}")))?;
        match &ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                let _ = require_result_sheet(doc, result_sheet_id)?;
            }
            _ => {}
        }
        ann
    } else {
        parse_annotation_shorthand(doc, &Value::Object(params.fields))?
    };

    let mut spec = require_graph_sheet(doc, &id)?.spec.clone();
    spec.annotations.push(annotation);
    validate_graph_spec_in_doc(doc, &id, &spec)?;
    let total = spec.annotations.len();

    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = spec;
    graph.initialized = true;

    Ok(CommandOutcome::mutated(
        "graph.addAnnotation",
        format!("added annotation to `{id}`"),
        json!({ "sheet_id": id.as_str(), "total_annotations": total }),
    ))
}

fn run_graph_apply_style_from(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_apply_style_from();
    let params: GraphApplyStyleFromParams = parse_params(&schema, raw)?;
    let clone_palette = params.clone_palette.unwrap_or(true);
    let clone_frame_and_axes = params.clone_frame_and_axes.unwrap_or(true);
    let clone_error_bars_and_legend = params.clone_error_bars_and_legend.unwrap_or(true);
    let target_id = parse_sheet_id(&params.sheet_id)?;
    let donor_id = parse_sheet_id(&params.donor_sheet_id)?;
    let donor_spec = require_graph_sheet(ctx.session.document(), &donor_id)?.spec.clone();

    let mut updated = require_graph_sheet(ctx.session.document(), &target_id)?.spec.clone();
    updated.apply_style_from_selective(&donor_spec, clone_palette, clone_frame_and_axes, clone_error_bars_and_legend);
    validate_graph_spec_in_doc(ctx.session.document(), &target_id, &updated)?;
    let target = require_graph_sheet_mut(&mut ctx.session.doc, &target_id)?;
    target.spec = updated;
    target.initialized = true;

    Ok(CommandOutcome::mutated(
        "graph.applyStyleFrom",
        format!("applied style from `{donor_id}` to `{target_id}`"),
        json!({
            "sheet_id": target_id.as_str(),
            "donor_sheet_id": donor_id.as_str(),
            "primary_source_sheet_id": target.spec.primary_source_sheet_id.as_str(),
            "clone_palette": clone_palette,
            "clone_frame_and_axes": clone_frame_and_axes,
            "clone_error_bars_and_legend": clone_error_bars_and_legend,
        }),
    ))
}

fn run_graph_apply_canvas_preset(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_apply_canvas_preset();
    let params: ApplyCanvasPresetParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let preset = parse_figure_canvas_preset(&params.preset)?;
    let mode = parse_canvas_resize_mode(params.resize_mode.as_deref())?;

    let mut updated_spec = require_graph_sheet(ctx.session.document(), &id)?.spec.clone();
    resize_graph_spec(&mut updated_spec, preset.dimensions_pt(), mode)?;
    updated_spec.validate()?;

    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = updated_spec;
    graph.initialized = true;

    let scene = compile_scene_for_sheet(ctx.session.document(), &id)?;
    let issues = preflight_check_scene(&scene);
    Ok(CommandOutcome::mutated(
        "graph.applyCanvasPreset",
        format!("applied canvas preset `{preset:?}` to `{id}`"),
        json!({
            "sheet_id": id.as_str(),
            "preset": format!("{preset:?}"),
            "resize_mode": format!("{mode:?}"),
            "width_pt": scene.width_pt,
            "height_pt": scene.height_pt,
            "issues": issues,
        }),
    ))
}

fn run_graph_preflight_check(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_preflight_check();
    let params: PreflightCheckParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let scene = compile_scene_for_sheet(ctx.session.document(), &id)?;
    let issues = preflight_check_scene(&scene);
    let passed = issues.is_empty();
    Ok(CommandOutcome::read_only(
        "graph.preflightCheck",
        format!("preflight checked `{id}` ({} issues)", issues.len()),
        json!({
            "sheet_id": id.as_str(),
            "width_pt": scene.width_pt,
            "height_pt": scene.height_pt,
            "passed": passed,
            "issue_count": issues.len(),
            "issues": issues,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Internal Resize, Axis, Series, Multivariate & Annotation Patchers
// ---------------------------------------------------------------------------

fn clamp_positive_stroke(width_pt: f64) -> f64 {
    if width_pt > 0.0 { width_pt.max(0.25) } else { width_pt }
}

fn scale_stroke_or_clamp(width_pt: &mut f64, mode: CanvasResizeMode, scale_1d: f64, field: &str) -> Result<(), EngineError> {
    match mode {
        CanvasResizeMode::KeepTypographyPt => {
            *width_pt = clamp_positive_stroke(*width_pt);
            Ok(())
        }
        CanvasResizeMode::ProportionalScale => {
            *width_pt = scale_finite(*width_pt, scale_1d, field)?;
            Ok(())
        }
    }
}

fn resize_marker_style(marker: &mut MarkerStyle, mode: CanvasResizeMode, scale_1d: f64) -> Result<(), EngineError> {
    match mode {
        CanvasResizeMode::KeepTypographyPt => {
            marker.stroke_width_pt = clamp_positive_stroke(marker.stroke_width_pt);
            Ok(())
        }
        CanvasResizeMode::ProportionalScale => {
            marker.size_pt = scale_finite(marker.size_pt, scale_1d, "marker.size_pt")?;
            marker.stroke_width_pt = scale_finite(marker.stroke_width_pt, scale_1d, "marker.stroke_width_pt")?;
            Ok(())
        }
    }
}

fn resize_error_bar_spec(eb: &mut ErrorBarSpec, mode: CanvasResizeMode, scale_1d: f64) -> Result<(), EngineError> {
    scale_stroke_or_clamp(&mut eb.stroke_width_pt, mode, scale_1d, "error_bar.stroke_width_pt")?;
    if mode == CanvasResizeMode::ProportionalScale {
        eb.cap_width_pt = scale_finite(eb.cap_width_pt, scale_1d, "error_bar.cap_width_pt")?;
    }
    Ok(())
}

fn resize_axis_spec(axis: &mut AxisSpec, mode: CanvasResizeMode, scale_1d: f64) -> Result<(), EngineError> {
    scale_stroke_or_clamp(&mut axis.stroke_width_pt, mode, scale_1d, "axis.stroke_width_pt")?;
    if mode == CanvasResizeMode::ProportionalScale {
        axis.gap_width_pt = scale_finite(axis.gap_width_pt, scale_1d, "axis.gap_width_pt")?;
    }
    for threshold in &mut axis.thresholds {
        scale_stroke_or_clamp(&mut threshold.stroke_width_pt, mode, scale_1d, "axis.threshold.stroke_width_pt")?;
    }
    Ok(())
}

fn resize_gdo(gdo: &mut GraphicalDataObject, mode: CanvasResizeMode, scale_1d: f64) -> Result<(), EngineError> {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                resize_marker_style(m, mode, scale_1d)?;
            }
            if let Some(l) = line {
                scale_stroke_or_clamp(&mut l.stroke_width_pt, mode, scale_1d, "line.stroke_width_pt")?;
            }
        }
        GraphicalDataObject::BarWithReplicates { border_width_pt, .. } | GraphicalDataObject::FloatingBar { border_width_pt, .. } => {
            scale_stroke_or_clamp(border_width_pt, mode, scale_1d, "bar.border_width_pt")?;
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => resize_marker_style(marker, mode, scale_1d)?,
        GraphicalDataObject::BeforeAfterLines { marker, stroke_width_pt, .. } => {
            if let Some(m) = marker {
                resize_marker_style(m, mode, scale_1d)?;
            }
            scale_stroke_or_clamp(stroke_width_pt, mode, scale_1d, "before_after.stroke_width_pt")?;
        }
        GraphicalDataObject::KaplanMeierStep { stroke_width_pt, .. } => {
            scale_stroke_or_clamp(stroke_width_pt, mode, scale_1d, "km.stroke_width_pt")?;
        }
        GraphicalDataObject::BoxAndWhisker { .. } | GraphicalDataObject::Violin { .. } | GraphicalDataObject::PartsOfWhole { .. } => {}
    }
    Ok(())
}

fn resize_graph_annotation(ann: &mut GraphAnnotation, mode: CanvasResizeMode, sx: f64, sy: f64, scale_1d: f64) -> Result<(), EngineError> {
    match ann {
        GraphAnnotation::TextBox { bounds_pt, .. } | GraphAnnotation::EmbeddedResultTable { bounds_pt, .. } => {
            *bounds_pt = scale_rect_pt(*bounds_pt, sx, sy)?;
        }
        GraphAnnotation::ModelEquation { bounds_pt, font_size_pt, .. } => {
            *bounds_pt = scale_rect_pt(*bounds_pt, sx, sy)?;
            if mode == CanvasResizeMode::ProportionalScale {
                *font_size_pt = scale_finite(*font_size_pt, scale_1d, "annotations.model_equation.font_size_pt")?;
            }
        }
        GraphAnnotation::LineOrArrow { start_pt, end_pt, .. } => {
            start_pt.0 = scale_finite(start_pt.0, sx, "annotations.line.start_x")?;
            start_pt.1 = scale_finite(start_pt.1, sy, "annotations.line.start_y")?;
            end_pt.0 = scale_finite(end_pt.0, sx, "annotations.line.end_x")?;
            end_pt.1 = scale_finite(end_pt.1, sy, "annotations.line.end_y")?;
        }
        GraphAnnotation::Shape { bounds_pt, stroke_width_pt, .. } => {
            *bounds_pt = scale_rect_pt(*bounds_pt, sx, sy)?;
            scale_stroke_or_clamp(stroke_width_pt, mode, scale_1d, "annotations.shape.stroke_width_pt")?;
        }
    }
    Ok(())
}

fn resize_graph_spec(spec: &mut GraphSpec, new_size: SizePt, mode: CanvasResizeMode) -> Result<(), EngineError> {
    let sx = safe_scale_ratio(spec.canvas_size_pt.width, new_size.width)?;
    let sy = safe_scale_ratio(spec.canvas_size_pt.height, new_size.height)?;
    let scale_1d = sx.min(sy);
    spec.canvas_size_pt = new_size;

    resize_axis_spec(&mut spec.x_axis, mode, scale_1d)?;
    resize_axis_spec(&mut spec.y_axis, mode, scale_1d)?;
    if let Some(y2) = &mut spec.y2_axis {
        resize_axis_spec(y2, mode, scale_1d)?;
    }

    for series in &mut spec.series {
        resize_gdo(&mut series.gdo, mode, scale_1d)?;
        if let Some(eb) = &mut series.y_error_bars {
            resize_error_bar_spec(eb, mode, scale_1d)?;
        }
        if let Some(eb) = &mut series.x_error_bars {
            resize_error_bar_spec(eb, mode, scale_1d)?;
        }
    }

    if let Some(mv) = &mut spec.multivariate {
        if let MultivariatePlotMode::CategoricalSummary { gdo, y_error_bars } = &mut mv.plot_mode {
            resize_gdo(gdo, mode, scale_1d)?;
            if let Some(eb) = y_error_bars {
                resize_error_bar_spec(eb, mode, scale_1d)?;
            }
        }
        if mode == CanvasResizeMode::ProportionalScale {
            match &mut mv.size {
                SizeChannelMapping::Constant { radius_pt } => {
                    *radius_pt = scale_finite(*radius_pt, scale_1d, "multivariate.size.radius_pt")?;
                }
                SizeChannelMapping::Variable { min_radius_pt, max_radius_pt, .. } => {
                    *min_radius_pt = scale_finite(*min_radius_pt, scale_1d, "multivariate.size.min_radius_pt")?;
                    *max_radius_pt = scale_finite(*max_radius_pt, scale_1d, "multivariate.size.max_radius_pt")?;
                }
            }
        }
    }

    for bracket in &mut spec.comparisons {
        scale_stroke_or_clamp(&mut bracket.stroke_width_pt, mode, scale_1d, "comparisons.stroke_width_pt")?;
        if mode == CanvasResizeMode::ProportionalScale {
            bracket.font_size_pt = scale_finite(bracket.font_size_pt, scale_1d, "comparisons.font_size_pt")?;
            bracket.bracket_step_pt = scale_finite(bracket.bracket_step_pt, scale_1d, "comparisons.bracket_step_pt")?;
        }
    }
    if let Some(cld) = &mut spec.compact_letter_display
        && mode == CanvasResizeMode::ProportionalScale
    {
        cld.font_size_pt = scale_finite(cld.font_size_pt, scale_1d, "compact_letter_display.font_size_pt")?;
        cld.offset_pt = scale_finite(cld.offset_pt, scale_1d, "compact_letter_display.offset_pt")?;
    }

    for ann in &mut spec.annotations {
        resize_graph_annotation(ann, mode, sx, sy, scale_1d)?;
    }

    for ov in &mut spec.point_overrides {
        if let Some(size) = &mut ov.size_pt
            && mode == CanvasResizeMode::ProportionalScale
        {
            *size = scale_finite(*size, scale_1d, "point_overrides.size_pt")?;
        }
        if let Some(bw) = &mut ov.border_width_pt {
            scale_stroke_or_clamp(bw, mode, scale_1d, "point_overrides.border_width_pt")?;
        }
    }

    if let Some((lx, ly)) = &mut spec.legend.position_pt {
        *lx = scale_finite(*lx, sx, "legend.position_pt.x")?;
        *ly = scale_finite(*ly, sy, "legend.position_pt.y")?;
    }
    Ok(())
}

fn okabe_swatch(idx: usize) -> Rgba8 {
    let swatches = PaletteId::OkabeIto.swatches();
    swatches[idx % swatches.len()]
}

fn resolve_graph_option(option_id: &str, requested_kind: Option<GraphKind>) -> Option<(GraphKind, Option<GroupedArrangement>, Option<&'static str>)> {
    match option_id.to_ascii_lowercase().as_str() {
        "diag.residual" | "diag.residuals" | "diag.residual_plot" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot), None, None)),
        "diag.normal_qq" | "diag.qq" | "diag.qq_plot" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot), None, None)),
        "diag.heatmap" | "heatmap" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap), None, None)),
        "diag.volcano" | "volcano" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::Volcano), None, None)),
        "diag.bland_altman" | "bland_altman" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman), None, None)),
        "diag.roc" | "roc" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve), None, None)),
        "diag.forest" | "forest" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot), None, None)),
        "xy.scatter" => Some((GraphKind::Xy, None, Some("xy.scatter"))),
        "xy.scatter_line" | "xy_points_connecting" => Some((GraphKind::Xy, None, Some("xy.scatter_line"))),
        "col.bar_replicates" | "col.scatter_bar" => Some((GraphKind::Column, None, Some("col.bar_replicates"))),
        "col.before_after" => Some((GraphKind::Column, None, Some("col.before_after"))),
        "col.box_whisker" | "col.box_tukey" => Some((GraphKind::Column, None, Some("col.box_whisker"))),
        "col.violin" => Some((GraphKind::Column, None, Some("col.violin"))),
        "col.raincloud" => Some((GraphKind::Column, None, Some("col.raincloud"))),
        "col.aligned_dot" => Some((GraphKind::Column, None, Some("col.aligned_dot"))),
        "grouped.interleaved" => {
            let kind = if requested_kind == Some(GraphKind::Contingency) { GraphKind::Contingency } else { GraphKind::Grouped };
            Some((kind, Some(GroupedArrangement::Interleaved), Some("grouped.interleaved")))
        }
        "grouped.stacked" => {
            let kind = if requested_kind == Some(GraphKind::Contingency) { GraphKind::Contingency } else { GraphKind::Grouped };
            Some((kind, Some(GroupedArrangement::Stacked), Some("grouped.stacked")))
        }
        "survival.km" => Some((GraphKind::Survival, None, Some("survival.km"))),
        "parts.donut" => Some((GraphKind::PartsOfWhole, None, Some("parts.donut"))),
        "parts.pie" => Some((GraphKind::PartsOfWhole, None, Some("parts.pie"))),
        "parts.waffle" => Some((GraphKind::PartsOfWhole, None, Some("parts.waffle"))),
        "mv.bubble" => Some((GraphKind::MultipleVariables, None, Some("mv.bubble"))),
        _ => None,
    }
}

fn default_gdo_for_option(preset: &str, color: Rgba8) -> GraphicalDataObject {
    let marker = MarkerStyle { fill_color: color, stroke_color: color, ..MarkerStyle::default() };
    match preset {
        "xy.scatter" | "mv.bubble" => GraphicalDataObject::ScatterAndLine { marker: Some(marker), line: None, area_fill: None },
        "xy.scatter_line" => GraphicalDataObject::ScatterAndLine {
            marker: Some(marker),
            line: Some(ConnectingLineStyle { kind: LineConnectionKind::PointToPoint, dash_style: LineDashStyle::Solid, stroke_width_pt: 1.5, color }),
            area_fill: None,
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
        "col.aligned_dot" => GraphicalDataObject::AlignedDotPlot { summary: SummaryMeasure::Mean, marker, show_crossbar: true },
        "col.bar_replicates" | "grouped.interleaved" | "grouped.stacked" => GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: color.with_alpha(210),
            border_color: Rgba8::BLACK,
            border_width_pt: 0.75,
            replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
        },
        "survival.km" => default_gdo_for_kind(GraphKind::Survival, color),
        "parts.donut" => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Donut, show_center_total: true },
        "parts.pie" => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Pie, show_center_total: false },
        "parts.waffle" => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Waffle10x10, show_center_total: true },
        _ => default_gdo_for_kind(GraphKind::Xy, color),
    }
}

fn apply_graph_kind_with_option(
    spec: &mut GraphSpec,
    kind: GraphKind,
    opt_arrangement: Option<GroupedArrangement>,
    gdo_preset: Option<&'static str>,
    reset_series: bool,
) {
    spec.kind = kind;
    if let Some(arr) = opt_arrangement {
        spec.grouped_arrangement = arr;
    }
    if !reset_series {
        return;
    }
    let swatch = okabe_swatch(0);
    for (idx, s) in spec.series.iter_mut().enumerate() {
        let color = if idx == 0 { swatch } else { okabe_swatch(idx) };
        s.gdo = match gdo_preset {
            Some(preset) => default_gdo_for_option(preset, color),
            None => default_gdo_for_kind(kind, color),
        };
    }
}

fn apply_graph_kind(spec: &mut GraphSpec, kind: GraphKind, reset_series: bool) {
    apply_graph_kind_with_option(spec, kind, None, None, reset_series);
}

fn default_gdo_for_kind(kind: GraphKind, color: Rgba8) -> GraphicalDataObject {
    match kind {
        GraphKind::Xy | GraphKind::MultipleVariables | GraphKind::Diagnostic(_) => GraphicalDataObject::ScatterAndLine {
            marker: Some(MarkerStyle { fill_color: color, stroke_color: color, ..MarkerStyle::default() }),
            line: Some(ConnectingLineStyle { kind: LineConnectionKind::PointToPoint, dash_style: LineDashStyle::Solid, stroke_width_pt: 1.5, color }),
            area_fill: None,
        },
        GraphKind::Column | GraphKind::Grouped | GraphKind::Contingency => GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: color,
            border_color: Rgba8::BLACK,
            border_width_pt: 0.75,
            replicate_overlay: None,
        },
        GraphKind::Survival => GraphicalDataObject::KaplanMeierStep {
            line_color: color,
            stroke_width_pt: 1.5,
            show_censored_ticks: true,
            show_greenwood_ci: false,
            ci_dashed: true,
        },
        GraphKind::PartsOfWhole => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Donut, show_center_total: true },
    }
}

fn apply_axis_updates(axis: &mut AxisSpec, val: &Value) -> Result<(), EngineError> {
    if let Some(title) = opt_str(val, &["title"]) {
        axis.title = title.to_owned();
    }
    if let Some(gap) = val.get("gap_width_pt").and_then(Value::as_f64) {
        axis.gap_width_pt = gap;
    }
    if let Some(orn_val) = val.get("break_ornament") {
        axis.break_ornament = parse_break_ornament(orn_val)?;
    }
    if let Some(sw) = val.get("stroke_width_pt").and_then(Value::as_f64) {
        axis.stroke_width_pt = sw;
    }
    if let Some(tick_angle) = val.get("tick_angle_deg").and_then(Value::as_f64) {
        axis.tick_angle_deg = tick_angle;
    }
    if let Some(tf_val) = val.get("tick_format") {
        axis.tick_format = parse_tick_format(tf_val)?;
    }
    if let Some(segs_val) = val.get("segments").and_then(Value::as_array) {
        if !(1..=3).contains(&segs_val.len()) {
            return Err(EngineError::with_code(ErrorCode::BadInput, "axis `segments` must contain 1 to 3 segments")
                .with_details(json!({ "pointer": "/segments" })));
        }
        let template = axis.segments.first().cloned().unwrap_or_default();
        let mut parsed = Vec::with_capacity(segs_val.len());
        for s in segs_val {
            let mut seg = template.clone();
            apply_segment_updates(&mut seg, s)?;
            parsed.push(seg);
        }
        axis.segments = parsed;
    } else if let Some(first_seg) = axis.segments.first_mut() {
        apply_segment_updates(first_seg, val)?;
    }
    if let Some(thresh_val) = val.get("thresholds").and_then(Value::as_array) {
        let mut out = Vec::with_capacity(thresh_val.len());
        for item in thresh_val {
            out.push(parse_axis_threshold(item)?);
        }
        axis.thresholds = out;
    }
    if let Some(add_val) = val.get("additional_ticks") {
        axis.additional_ticks = parse_additional_ticks(add_val)?;
    }
    axis.validate()?;
    Ok(())
}

fn apply_segment_updates(seg: &mut AxisSegment, val: &Value) -> Result<(), EngineError> {
    if let Some(scale_val) = val.get("scale") {
        seg.scale = parse_axis_scale(scale_val)?;
    }
    if let Some(min_v) = val.get("min") {
        seg.min = if min_v.is_null() {
            None
        } else {
            Some(min_v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`min` must be numeric or null"))?)
        };
    }
    if let Some(max_v) = val.get("max") {
        seg.max = if max_v.is_null() {
            None
        } else {
            Some(max_v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`max` must be numeric or null"))?)
        };
    }
    if let Some(share) = val.get("length_share").and_then(Value::as_f64) {
        seg.length_share = share;
    }
    if let Some(major) = val.get("major_tick_interval") {
        seg.major_tick_interval = if major.is_null() {
            None
        } else {
            Some(major.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`major_tick_interval` must be numeric or null"))?)
        };
    }
    if let Some(minor) = val.get("minor_ticks_per_interval").and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok()) {
        seg.minor_ticks_per_interval = minor;
    }
    Ok(())
}

fn apply_single_series_patch(series: &mut SeriesSpec, val: &Value) -> Result<(), EngineError> {
    if let Some(src) = opt_str(val, &["source_sheet_id", "source_sheet", "data_sheet_id"]) {
        series.source_sheet_id = parse_sheet_id(src)?;
    }
    if let Some(ds_val) = val.get("data_set_index") {
        let ds_u64 = ds_val.as_u64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`data_set_index` must be a non-negative integer"))?;
        series.data_set_index =
            usize::try_from(ds_u64).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "`data_set_index` is out of range"))?;
    }
    if let Some(overlay_val) = val.get("regression_overlay") {
        series.regression_overlay = if overlay_val.is_null() {
            None
        } else {
            Some(
                serde_json::from_value(overlay_val.clone())
                    .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `regression_overlay`: {err}")))?,
            )
        };
    }
    if let Some(label) = opt_str(val, &["legend_label", "label"]) {
        series.legend_label = Some(label.to_owned());
    }
    if let Some(y2) = val.get("bind_to_right_y2").and_then(Value::as_bool) {
        series.bind_to_right_y2 = y2;
    }
    if let Some(gdo_val) = val.get("gdo").or_else(|| val.get("graphical_object")) {
        series.gdo =
            serde_json::from_value(gdo_val.clone()).map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `gdo`: {err}")))?;
    }
    if let Some(color_val) = val.get("color").or_else(|| val.get("fill_color")) {
        let rgba = parse_rgba8(color_val)?;
        recolor_gdo(&mut series.gdo, rgba);
    }
    if let Some(m_val) = val.get("marker").or_else(|| val.get("markers")) {
        apply_marker_to_gdo(&mut series.gdo, m_val)?;
    }
    if let Some(l_val) = val.get("connecting_line").or_else(|| val.get("line")) {
        apply_line_to_gdo(&mut series.gdo, l_val)?;
    }
    if let Some(eb_val) = val.get("y_error_bars").or_else(|| val.get("error_bars")) {
        series.y_error_bars = patch_error_bars(series.y_error_bars, eb_val)?;
    }
    if let Some(xeb_val) = val.get("x_error_bars") {
        series.x_error_bars = patch_error_bars(series.x_error_bars, xeb_val)?;
    }
    Ok(())
}

fn recolor_gdo(gdo: &mut GraphicalDataObject, color: Rgba8) {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                m.fill_color = color;
                m.stroke_color = color;
            }
            if let Some(l) = line {
                l.color = color;
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => *fill_color = color,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.fill_color = color;
            marker.stroke_color = color;
        }
        GraphicalDataObject::BeforeAfterLines { marker, line_color, .. } => {
            *line_color = color;
            if let Some(m) = marker {
                m.fill_color = color;
                m.stroke_color = color;
            }
        }
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => *line_color = color,
        GraphicalDataObject::PartsOfWhole { .. } => {}
    }
}

fn apply_marker_to_gdo(gdo: &mut GraphicalDataObject, val: &Value) -> Result<(), EngineError> {
    if val.is_null() {
        if let GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } = gdo {
            *marker = None;
        }
        return Ok(());
    }
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } => {
            let mut style = marker.unwrap_or_default();
            apply_marker_patch(&mut style, val)?;
            *marker = Some(style);
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => apply_marker_patch(marker, val)?,
        _ => {}
    }
    Ok(())
}

fn apply_line_to_gdo(gdo: &mut GraphicalDataObject, val: &Value) -> Result<(), EngineError> {
    if val.is_null() {
        if let GraphicalDataObject::ScatterAndLine { line, .. } = gdo {
            *line = None;
        }
        return Ok(());
    }
    if let GraphicalDataObject::ScatterAndLine { line, .. } = gdo {
        if opt_str(val, &["kind"]).is_some_and(|k| k.eq_ignore_ascii_case("none")) {
            *line = None;
            return Ok(());
        }
        let mut style = line.unwrap_or(ConnectingLineStyle {
            kind: LineConnectionKind::PointToPoint,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.5,
            color: Rgba8::BLACK,
        });
        if let Some(k) = val.get("kind") {
            style.kind = parse_line_connection_kind(k)?;
        }
        if let Some(d) = val.get("dash_style").or_else(|| val.get("dash")) {
            style.dash_style = parse_line_dash_style(d)?;
        }
        if let Some(w) = val.get("stroke_width_pt").or_else(|| val.get("width_pt")).and_then(Value::as_f64) {
            style.stroke_width_pt = w;
        }
        if let Some(c) = val.get("color") {
            style.color = parse_rgba8(c)?;
        }
        *line = Some(style);
    } else if let GraphicalDataObject::KaplanMeierStep { line_color, stroke_width_pt, .. } = gdo {
        if let Some(c) = val.get("color") {
            *line_color = parse_rgba8(c)?;
        }
        if let Some(w) = val.get("stroke_width_pt").and_then(Value::as_f64) {
            *stroke_width_pt = w;
        }
    }
    Ok(())
}

fn apply_marker_patch(marker: &mut MarkerStyle, val: &Value) -> Result<(), EngineError> {
    if let Some(shape) = val.get("shape") {
        marker.shape = parse_marker_shape(shape)?;
    }
    if let Some(fill_mode) = val.get("fill_mode") {
        marker.fill_mode = parse_marker_fill_mode(fill_mode)?;
    }
    if let Some(size) = val.get("size_pt").and_then(Value::as_f64) {
        marker.size_pt = size;
    }
    if let Some(fc) = val.get("fill_color").or_else(|| val.get("color")) {
        marker.fill_color = parse_rgba8(fc)?;
    }
    if let Some(sc) = val.get("stroke_color") {
        marker.stroke_color = parse_rgba8(sc)?;
    }
    if let Some(sw) = val.get("stroke_width_pt").and_then(Value::as_f64) {
        marker.stroke_width_pt = sw;
    }
    Ok(())
}

fn patch_error_bars(current: Option<ErrorBarSpec>, val: &Value) -> Result<Option<ErrorBarSpec>, EngineError> {
    if val.is_null() || opt_str(val, &["metric"]).is_some_and(|m| m.eq_ignore_ascii_case("none")) {
        return Ok(None);
    }
    let mut eb = current.unwrap_or_default();
    if let Some(m) = val.get("metric") {
        eb.metric = parse_error_bar_metric(m)?;
    }
    if let Some(d) = val.get("direction") {
        eb.direction = parse_error_bar_direction(d)?;
    }
    if let Some(dash) = val.get("dash_style") {
        eb.dash_style = parse_line_dash_style(dash)?;
    }
    if let Some(w) = val.get("stroke_width_pt").and_then(Value::as_f64) {
        eb.stroke_width_pt = w;
    }
    if let Some(cw) = val.get("cap_width_pt").and_then(Value::as_f64) {
        eb.cap_width_pt = cw;
    }
    if let Some(c) = val.get("color") {
        eb.color = if c.is_null() { None } else { Some(parse_rgba8(c)?) };
    }
    Ok(Some(eb))
}

fn default_series_like(spec: &GraphSpec, data_set_index: usize) -> SeriesSpec {
    SeriesSpec {
        source_sheet_id: spec.primary_source_sheet_id.clone(),
        data_set_index,
        legend_label: None,
        bind_to_right_y2: false,
        gdo: default_gdo_for_kind(spec.kind, okabe_swatch(data_set_index)),
        y_error_bars: None,
        x_error_bars: None,
        regression_overlay: None,
    }
}

fn resolve_series_target_index(spec: &GraphSpec, series_index: Option<usize>, data_set_index: Option<usize>) -> Result<usize, EngineError> {
    if let Some(idx) = series_index {
        if idx >= spec.series.len() {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`series_index` {idx} out of bounds ({} series)", spec.series.len()))
                .with_details(json!({ "pointer": "/series_index" })));
        }
        return Ok(idx);
    }
    if let Some(ds_idx) = data_set_index {
        return spec
            .series
            .iter()
            .position(|s| s.data_set_index == ds_idx)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("no series bound to `data_set_index` {ds_idx}")));
    }
    if spec.series.is_empty() {
        return Err(EngineError::with_code(ErrorCode::Conflict, "graph has no series configured"));
    }
    Ok(0)
}

fn has_series_patch_keys(map: &Map<String, Value>) -> bool {
    const KEYS: &[&str] = &[
        "source_sheet_id",
        "source_sheet",
        "data_sheet_id",
        "regression_overlay",
        "legend_label",
        "label",
        "bind_to_right_y2",
        "color",
        "fill_color",
        "marker",
        "markers",
        "connecting_line",
        "line",
        "error_bars",
        "y_error_bars",
        "x_error_bars",
        "gdo",
        "graphical_object",
    ];
    KEYS.iter().any(|k| map.contains_key(*k))
}

fn parse_multivariate_encoding(val: &Value, current: Option<&MultivariateGraphEncoding>) -> Result<MultivariateGraphEncoding, EngineError> {
    if let Ok(full) = serde_json::from_value::<MultivariateGraphEncoding>(val.clone()) {
        return Ok(full);
    }
    let mut base = current.cloned().unwrap_or(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Constant(okabe_swatch(0)),
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    if let Some(xc) = val.get("x_column").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()) {
        base.x_column = xc;
    }
    if let Some(yc) = val.get("y_column").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()) {
        base.y_column = yc;
    }
    if let Some(lc) = val.get("label_column").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()) {
        base.label_column = Some(lc);
    }
    if let Some(hull) = val.get("show_convex_hulls").and_then(Value::as_bool) {
        base.show_convex_hulls = hull;
    }
    if let Some(ell) = val.get("show_confidence_ellipses").and_then(Value::as_bool) {
        base.show_confidence_ellipses = ell;
    }
    if let Some(color_v) = val.get("color") {
        base.color = parse_color_channel(color_v)?;
    }
    if let Some(size_v) = val.get("size") {
        base.size = parse_size_channel(size_v)?;
    }
    Ok(base)
}

fn parse_color_channel(val: &Value) -> Result<ColorChannelMapping, EngineError> {
    if let Ok(typed) = serde_json::from_value::<ColorChannelMapping>(val.clone()) {
        return Ok(typed);
    }
    if let Some(col) = val.as_u64().and_then(|v| usize::try_from(v).ok()) {
        return Ok(ColorChannelMapping::Continuous {
            column: col,
            ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
            domain_clamp: None,
        });
    }
    if let Some(col) = val.get("column").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()) {
        if val.get("categorical").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(ColorChannelMapping::Categorical { column: col, palette: PaletteId::OkabeIto, level_order_override: None });
        }
        return Ok(ColorChannelMapping::Continuous {
            column: col,
            ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
            domain_clamp: None,
        });
    }
    Ok(ColorChannelMapping::Constant(parse_rgba8(val)?))
}

fn parse_size_channel(val: &Value) -> Result<SizeChannelMapping, EngineError> {
    if let Ok(typed) = serde_json::from_value::<SizeChannelMapping>(val.clone()) {
        return Ok(typed);
    }
    if let Some(col) = val.get("column").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()) {
        let min_r = val.get("min_radius_pt").and_then(Value::as_f64).unwrap_or(3.0);
        let max_r = val.get("max_radius_pt").and_then(Value::as_f64).unwrap_or(14.0);
        return Ok(SizeChannelMapping::Variable { column: col, min_radius_pt: min_r, max_radius_pt: max_r });
    }
    if let Some(r) = val.get("radius_pt").and_then(Value::as_f64).or_else(|| val.as_f64()) {
        return Ok(SizeChannelMapping::Constant { radius_pt: r });
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "invalid multivariate `size` mapping"))
}

fn parse_annotation_shorthand(doc: &ProjectDocument, params: &Value) -> Result<GraphAnnotation, EngineError> {
    let kind = opt_str(params, &["kind", "type"]).unwrap_or("text").trim().to_ascii_lowercase();
    match kind.as_str() {
        "text" | "text_box" | "textbox" => {
            let markdown = opt_str(params, &["markdown", "text"])
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing `text` or `markdown`"))?
                .to_owned();
            let bounds_pt = parse_rect_pt(
                params.get("bounds_pt"),
                RectPt::new(
                    params.get("x_pt").and_then(Value::as_f64).unwrap_or(100.0),
                    params.get("y_pt").and_then(Value::as_f64).unwrap_or(80.0),
                    params.get("width_pt").and_then(Value::as_f64).unwrap_or(120.0),
                    params.get("height_pt").and_then(Value::as_f64).unwrap_or(28.0),
                ),
            )?;
            let bordered = params.get("bordered").or_else(|| params.get("boxed")).and_then(Value::as_bool).unwrap_or(false);
            Ok(GraphAnnotation::TextBox { bounds_pt, markdown, bordered })
        }
        "line" | "arrow" => {
            let start_pt = parse_pair_f64(params.get("start_pt"), (40.0, 40.0))?;
            let end_pt = parse_pair_f64(params.get("end_pt"), (140.0, 40.0))?;
            let arrowhead = params.get("arrowhead").and_then(Value::as_bool).unwrap_or(kind == "arrow");
            let dash =
                params.get("dash").or_else(|| params.get("dash_style")).map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Solid);
            let color = params.get("color").map(parse_rgba8).transpose()?.unwrap_or(Rgba8::BLACK);
            Ok(GraphAnnotation::LineOrArrow { start_pt, end_pt, arrowhead, dash, color })
        }
        "shape" | "rect" | "ellipse" => {
            let shape_kind =
                match opt_str(params, &["shape"]).unwrap_or(if kind == "ellipse" { "ellipse" } else { "rectangle" }).to_ascii_lowercase().as_str() {
                    "ellipse" | "circle" => AnnotationShapeKind::Ellipse,
                    "rounded_rectangle" | "rounded" => AnnotationShapeKind::RoundedRectangle,
                    _ => AnnotationShapeKind::Rectangle,
                };
            let bounds_pt = parse_rect_pt(params.get("bounds_pt").or_else(|| params.get("rect_pt")), RectPt::new(60.0, 60.0, 80.0, 40.0))?;
            let stroke_color = params.get("stroke_color").or_else(|| params.get("color")).map(parse_rgba8).transpose()?.unwrap_or(Rgba8::BLACK);
            let fill_color = params.get("fill_color").filter(|v| !v.is_null()).map(parse_rgba8).transpose()?;
            let stroke_width_pt = params.get("stroke_width_pt").and_then(Value::as_f64).unwrap_or(1.0);
            let dash = params.get("dash").map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Solid);
            Ok(GraphAnnotation::Shape { kind: shape_kind, bounds_pt, stroke_color, stroke_width_pt, dash, fill_color })
        }
        "table" | "embedded_result_table" | "result_table" => {
            let raw_res = opt_str(params, &["result_sheet_id", "result_sheet"])
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing `result_sheet_id`"))?;
            let res_id = parse_sheet_id(raw_res)?;
            let _ = require_result_sheet(doc, &res_id)?;
            let bounds_pt = parse_rect_pt(params.get("bounds_pt"), RectPt::new(180.0, 40.0, 180.0, 90.0))?;
            let tab_id = opt_str(params, &["tab_id"]).map(str::to_owned);
            Ok(GraphAnnotation::EmbeddedResultTable { result_sheet_id: res_id, tab_id, bounds_pt })
        }
        "equation" | "model_equation" => {
            let raw_res = opt_str(params, &["result_sheet_id", "result_sheet"])
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing `result_sheet_id`"))?;
            let res_id = parse_sheet_id(raw_res)?;
            let _ = require_result_sheet(doc, &res_id)?;
            let data_set_index = match params.get("data_set_index") {
                Some(v) if !v.is_null() => {
                    let raw =
                        v.as_u64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`data_set_index` must be a non-negative integer"))?;
                    usize::try_from(raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "`data_set_index` out of range"))?
                }
                _ => 0,
            };
            let mode = match opt_str(params, &["mode"]).map(str::to_ascii_lowercase).as_deref() {
                None | Some("substituted") => EquationDisplayMode::Substituted,
                Some("symbolic") => EquationDisplayMode::Symbolic,
                Some(other) => {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("unknown model equation mode `{other}`; use `substituted` or `symbolic`"),
                    ));
                }
            };
            let bounds_pt = parse_rect_pt(params.get("bounds_pt"), RectPt::new(60.0, 48.0, 220.0, 44.0))?;
            let font_size_pt = match params.get("font_size_pt") {
                Some(v) if !v.is_null() => {
                    v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`font_size_pt` must be a finite number"))?
                }
                _ => 10.0,
            };
            if !font_size_pt.is_finite() || font_size_pt <= 0.0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`font_size_pt` must be positive and finite"));
            }
            let bordered = params.get("bordered").or_else(|| params.get("boxed")).and_then(Value::as_bool).unwrap_or(true);
            Ok(GraphAnnotation::ModelEquation { result_sheet_id: res_id, data_set_index, mode, bounds_pt, font_size_pt, bordered })
        }
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown annotation kind `{other}`"))),
    }
}

fn resolve_group_index(val: &Value, comparisons: &[PairwiseComparisonRecord]) -> Result<usize, EngineError> {
    if let Some(raw) = val.as_u64() {
        return usize::try_from(raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "group index out of bounds"));
    }
    if let Some(label) = val.as_str() {
        for cmp in comparisons {
            if cmp.label_a == label {
                return Ok(cmp.group_a);
            }
            if cmp.label_b == label {
                return Ok(cmp.group_b);
            }
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("unknown comparison group label `{label}`")));
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "group specifier must be integer index or group label"))
}

fn opt_str<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| params.get(*k).and_then(Value::as_str))
}

fn parse_pair_f64(val: Option<&Value>, def: (f64, f64)) -> Result<(f64, f64), EngineError> {
    let Some(v) = val else {
        return Ok(def);
    };
    if let Some(arr) = v.as_array()
        && arr.len() == 2
    {
        let x = arr[0].as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "point x must be f64"))?;
        let y = arr[1].as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "point y must be f64"))?;
        return Ok((x, y));
    }
    if let Some(obj) = v.as_object()
        && let (Some(x), Some(y)) = (obj.get("x").and_then(Value::as_f64), obj.get("y").and_then(Value::as_f64))
    {
        return Ok((x, y));
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "invalid `(x, y)` point"))
}

fn parse_graph_kind(val: &Value) -> Result<GraphKind, EngineError> {
    if let Ok(kind) = serde_json::from_value::<GraphKind>(val.clone()) {
        return Ok(kind);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("xy" | "xy_scatter" | "scatter" | "xy_line" | "line" | "xy_mean_error") => Ok(GraphKind::Xy),
        Some("column" | "column_bar" | "bar" | "column_box" | "column_violin" | "box" | "violin") => Ok(GraphKind::Column),
        Some("grouped" | "grouped_bar" | "grouped_mean_error") => Ok(GraphKind::Grouped),
        Some("contingency" | "contingency_stacked" | "contingency_grouped") => Ok(GraphKind::Contingency),
        Some("survival" | "survival_kaplan_meier" | "kaplan_meier") => Ok(GraphKind::Survival),
        Some("parts_of_whole" | "pie" | "donut") => Ok(GraphKind::PartsOfWhole),
        Some("multiple_variables" | "multivariate" | "bubble") => Ok(GraphKind::MultipleVariables),
        Some("heatmap") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)),
        Some("volcano") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::Volcano)),
        Some("bland_altman") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman)),
        Some("roc") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve)),
        Some("forest") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot)),
        Some("residuals" | "residual_plot") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot)),
        Some("qq" | "normal_qq") => Ok(GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot)),
        Some(other) => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown graph kind `{other}`"))),
        None => Err(EngineError::with_code(ErrorCode::BadInput, "graph `kind` must be a string or typed `GraphKind` object")),
    }
}

fn parse_frame_preset(val: &Value) -> Result<FramePreset, EngineError> {
    if let Ok(fp) = serde_json::from_value::<FramePreset>(val.clone()) {
        return Ok(fp);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("open_l" | "bottom_left" | "classic" | "plain") => Ok(FramePreset::OpenL),
        Some("offset" | "offset_range" | "tufte") => Ok(FramePreset::OffsetRangeTufte),
        Some("plain_boxed" | "box" | "plain_box") => Ok(FramePreset::PlainBoxed),
        Some("dual_y" | "dual_y_secondary_right") => Ok(FramePreset::DualYSecondaryRight),
        Some("origin_crossing" | "origin_cross" | "cross") => Ok(FramePreset::OriginCrossing),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `frame_preset`: {val}"))),
    }
}

fn parse_axis_scale(val: &Value) -> Result<GraphAxisScale, EngineError> {
    if let Ok(scale) = serde_json::from_value::<GraphAxisScale>(val.clone()) {
        return Ok(scale);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("linear") => Ok(GraphAxisScale::Linear),
        Some("log10" | "log") => Ok(GraphAxisScale::Log10),
        Some("log2") => Ok(GraphAxisScale::Log2),
        Some("ln") => Ok(GraphAxisScale::Ln),
        Some("probit" | "probability") => Ok(GraphAxisScale::Probit),
        Some("logit" | "logit_percent") => Ok(GraphAxisScale::Logit(LogitDomain::Percent)),
        Some("logit_proportion") => Ok(GraphAxisScale::Logit(LogitDomain::Proportion)),
        Some("reciprocal") => Ok(GraphAxisScale::Reciprocal),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid axis `scale`: {val}"))),
    }
}

fn parse_tick_format(val: &Value) -> Result<TickLabelFormat, EngineError> {
    if let Ok(tf) = serde_json::from_value::<TickLabelFormat>(val.clone()) {
        return Ok(tf);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("decimal") => Ok(TickLabelFormat::Decimal),
        Some("power_of_ten" | "power_of_10" | "powers_of_10" | "superscript") => Ok(TickLabelFormat::PowerOfTenSuperscript),
        Some("scientific" | "scientific_e") => Ok(TickLabelFormat::ScientificE),
        Some("antilog") => Ok(TickLabelFormat::Antilog),
        Some("engineering" | "si" | "engineering_si") => Ok(TickLabelFormat::EngineeringSi),
        Some("categorical" | "row_titles") => Ok(TickLabelFormat::Categorical(CategoricalTickSource::RowTitles)),
        Some("column_titles") => Ok(TickLabelFormat::Categorical(CategoricalTickSource::ColumnTitles)),
        Some("row_numbers") => Ok(TickLabelFormat::Categorical(CategoricalTickSource::RowNumbers)),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `tick_format`: {val}"))),
    }
}

fn parse_break_ornament(val: &Value) -> Result<BreakOrnament, EngineError> {
    if let Ok(b) = serde_json::from_value::<BreakOrnament>(val.clone()) {
        return Ok(b);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("double_slash" | "double") => Ok(BreakOrnament::DoubleSlash),
        Some("zigzag") => Ok(BreakOrnament::Zigzag),
        Some("blank" | "blank_gap" | "none") => Ok(BreakOrnament::BlankGap),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `break_ornament`: {val}"))),
    }
}

fn parse_axis_threshold(val: &Value) -> Result<AxisThreshold, EngineError> {
    if let Ok(t) = serde_json::from_value::<AxisThreshold>(val.clone()) {
        return Ok(t);
    }
    let value =
        val.get("value").and_then(Value::as_f64).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "threshold missing numeric `value`"))?;
    let shade_to = val.get("shade_to").and_then(Value::as_f64);
    let label = opt_str(val, &["label"]).map(str::to_owned);
    let color = val.get("color").map(parse_rgba8).transpose()?.unwrap_or(Rgba8::rgb(120, 120, 120));
    let dash_style = val.get("dash_style").map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Dashed);
    let stroke_width_pt = val.get("stroke_width_pt").and_then(Value::as_f64).unwrap_or(1.0);
    Ok(AxisThreshold { value, shade_to, color, dash_style, stroke_width_pt, label })
}

fn parse_additional_ticks(val: &Value) -> Result<Vec<AdditionalTick>, EngineError> {
    let arr = val.as_array().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`ticks` must be an array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let value = item
            .get("value")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "additional tick missing finite numeric `value`"))?;
        let label = match item.get("label") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_str().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "additional tick `label` must be string or null"))?.to_owned(),
            ),
        };
        let draw_gridline = match item.get("draw_gridline") {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "additional tick `draw_gridline` must be boolean"))?,
        };
        out.push(AdditionalTick { value, label, draw_gridline });
    }
    Ok(out)
}

fn parse_marker_shape(val: &Value) -> Result<MarkerShape, EngineError> {
    if let Ok(s) = serde_json::from_value::<MarkerShape>(val.clone()) {
        return Ok(s);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("circle") => Ok(MarkerShape::Circle),
        Some("square") => Ok(MarkerShape::Square),
        Some("triangle" | "triangle_up") => Ok(MarkerShape::TriangleUp),
        Some("triangle_down") => Ok(MarkerShape::TriangleDown),
        Some("diamond") => Ok(MarkerShape::Diamond),
        Some("cross" | "x") => Ok(MarkerShape::Cross),
        Some("plus") => Ok(MarkerShape::Plus),
        Some("star") => Ok(MarkerShape::Star),
        Some("hexagon") => Ok(MarkerShape::Hexagon),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid marker `shape`: {val}"))),
    }
}

fn parse_marker_fill_mode(val: &Value) -> Result<MarkerFillMode, EngineError> {
    if let Ok(f) = serde_json::from_value::<MarkerFillMode>(val.clone()) {
        return Ok(f);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("filled" | "solid") => Ok(MarkerFillMode::Filled),
        Some("open" | "hollow") => Ok(MarkerFillMode::Open),
        Some("semi_top") => Ok(MarkerFillMode::SemiFilledTop),
        Some("semi_bottom") => Ok(MarkerFillMode::SemiFilledBottom),
        Some("semi_left") => Ok(MarkerFillMode::SemiFilledLeft),
        Some("semi_right") => Ok(MarkerFillMode::SemiFilledRight),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid marker `fill_mode`: {val}"))),
    }
}

fn parse_line_connection_kind(val: &Value) -> Result<LineConnectionKind, EngineError> {
    if let Ok(k) = serde_json::from_value::<LineConnectionKind>(val.clone()) {
        return Ok(k);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("point_to_point" | "straight") => Ok(LineConnectionKind::PointToPoint),
        Some("staircase" | "step" | "stepped") => Ok(LineConnectionKind::Staircase),
        Some("spline" | "smooth" | "monotonic_cubic") => Ok(LineConnectionKind::MonotonicCubicSpline),
        Some("akima") => Ok(LineConnectionKind::AkimaSpline),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid connecting line `kind`: {val}"))),
    }
}

fn parse_line_dash_style(val: &Value) -> Result<LineDashStyle, EngineError> {
    if let Ok(d) = serde_json::from_value::<LineDashStyle>(val.clone()) {
        return Ok(d);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("solid") => Ok(LineDashStyle::Solid),
        Some("dashed" | "dash") => Ok(LineDashStyle::Dashed),
        Some("dotted" | "dot") => Ok(LineDashStyle::Dotted),
        Some("dash_dot" | "dashdot") => Ok(LineDashStyle::DashDot),
        Some("dash_dot_dot") => Ok(LineDashStyle::DashDotDot),
        Some("long_dash") => Ok(LineDashStyle::LongDash),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `dash_style`: {val}"))),
    }
}

fn parse_error_bar_metric(val: &Value) -> Result<ErrorBarMetric, EngineError> {
    if let Ok(m) = serde_json::from_value::<ErrorBarMetric>(val.clone()) {
        return Ok(m);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("sd") => Ok(ErrorBarMetric::Sd),
        Some("sem") => Ok(ErrorBarMetric::Sem),
        Some("ci95" | "ci_95" | "95ci") => Ok(ErrorBarMetric::ConfidenceInterval95),
        Some("iqr") => Ok(ErrorBarMetric::InterquartileRange),
        Some("range") => Ok(ErrorBarMetric::Range),
        Some("explicit") => Ok(ErrorBarMetric::ExplicitSubcolumns),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid error bar `metric`: {val}"))),
    }
}

fn parse_error_bar_direction(val: &Value) -> Result<ErrorBarDirection, EngineError> {
    if let Ok(d) = serde_json::from_value::<ErrorBarDirection>(val.clone()) {
        return Ok(d);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("both") => Ok(ErrorBarDirection::Both),
        Some("above" | "upper" | "upper_only") => Ok(ErrorBarDirection::UpperOnly),
        Some("below" | "lower" | "lower_only") => Ok(ErrorBarDirection::LowerOnly),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid error bar `direction`: {val}"))),
    }
}

fn parse_pairwise_display_mode(raw: &str) -> Result<PairwiseDisplayMode, EngineError> {
    let norm: String = raw.trim().chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "bracketsonly" | "brackets" => Ok(PairwiseDisplayMode::BracketsOnly),
        "compactletterdisplayonly" | "cldonly" | "cld" | "lettersonly" | "letters" => Ok(PairwiseDisplayMode::CompactLetterDisplayOnly),
        "bracketsandcompactletterdisplay" | "bracketsandcld" | "both" => Ok(PairwiseDisplayMode::BracketsAndCompactLetterDisplay),
        _ => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("invalid `pairwise_display_mode` `{raw}`; expected `brackets_only`, `compact_letter_display_only`, or `brackets_and_compact_letter_display`"),
        )
        .with_details(json!({ "pointer": "/pairwise_display_mode" }))),
    }
}

fn parse_cld_ordering(raw: &str) -> Result<CldOrdering, EngineError> {
    let norm: String = raw.trim().chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "descendingmean" | "descending" | "desc" | "highestfirst" => Ok(CldOrdering::DescendingMean),
        "ascendingmean" | "ascending" | "asc" | "lowestfirst" => Ok(CldOrdering::AscendingMean),
        _ => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("invalid CLD `ordering` `{raw}`; expected `descending_mean` or `ascending_mean`"),
        )
        .with_details(json!({ "pointer": "/ordering" }))),
    }
}

fn apply_cld_field_updates(cld: &mut CompactLetterDisplaySpec, params: &GraphSetCompactLetterDisplayParams) -> Result<(), EngineError> {
    if let Some(ref raw_ord) = params.ordering {
        cld.ordering = parse_cld_ordering(raw_ord)?;
    }
    if let Some(upper) = params.uppercase {
        cld.uppercase = upper;
    }
    if let Some(alpha_opt) = params.alpha_override {
        if let Some(a) = alpha_opt
            && (!a.is_finite() || a <= 0.0 || a >= 1.0)
        {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("CLD `alpha_override` must be finite in (0, 1), got {a}"))
                .with_details(json!({ "pointer": "/alpha_override" })));
        }
        cld.alpha_override = alpha_opt;
    }
    if let Some(fs) = params.font_size_pt {
        if !fs.is_finite() || fs <= 0.0 || fs > 144.0 {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("CLD `font_size_pt` must be in (0, 144], got {fs}"))
                .with_details(json!({ "pointer": "/font_size_pt" })));
        }
        cld.font_size_pt = fs;
    }
    if let Some(off) = params.offset_pt {
        if !off.is_finite() || !(0.0..=200.0).contains(&off) {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("CLD `offset_pt` must be in [0, 200], got {off}"))
                .with_details(json!({ "pointer": "/offset_pt" })));
        }
        cld.offset_pt = off;
    }
    if let Some(ref color_val) = params.color
        && !color_val.is_null()
    {
        cld.color = parse_rgba8(color_val)?;
    }
    Ok(())
}

fn run_graph_set_compact_letter_display(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_set_compact_letter_display();
    let params: GraphSetCompactLetterDisplayParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = ctx.session.document();
    let mut spec = require_graph_sheet(doc, &id)?.spec.clone();

    if params.enabled == Some(false) {
        spec.compact_letter_display = None;
        spec.pairwise_display_mode = match params.pairwise_display_mode.as_deref() {
            Some(m) => parse_pairwise_display_mode(m)?,
            None => PairwiseDisplayMode::BracketsOnly,
        };
    } else {
        let wants_cld = params.enabled == Some(true)
            || params.result_sheet_id.is_some()
            || params.ordering.is_some()
            || params.uppercase.is_some()
            || params.alpha_override.is_some()
            || params.font_size_pt.is_some()
            || params.offset_pt.is_some()
            || params.color.as_ref().is_some_and(|v| !v.is_null());

        if wants_cld {
            let mut cld = match params.result_sheet_id.as_deref() {
                Some(raw_res) => {
                    let res_id = parse_sheet_id(raw_res)?;
                    let _ = require_result_sheet(doc, &res_id)?;
                    match spec.compact_letter_display.take() {
                        Some(mut existing) => {
                            existing.result_sheet_id = res_id;
                            existing
                        }
                        None => CompactLetterDisplaySpec::new(res_id),
                    }
                }
                None => spec.compact_letter_display.take().ok_or_else(|| {
                    EngineError::with_code(ErrorCode::BadInput, "enabling Compact Letter Display requires `result_sheet_id`")
                        .with_details(json!({ "pointer": "/result_sheet_id" }))
                })?,
            };
            apply_cld_field_updates(&mut cld, &params)?;
            spec.compact_letter_display = Some(cld);
        }

        if let Some(raw_mode) = params.pairwise_display_mode.as_deref() {
            spec.pairwise_display_mode = parse_pairwise_display_mode(raw_mode)?;
        } else if spec.compact_letter_display.is_some() && spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly {
            spec.pairwise_display_mode = if spec.comparisons.is_empty() {
                PairwiseDisplayMode::CompactLetterDisplayOnly
            } else {
                PairwiseDisplayMode::BracketsAndCompactLetterDisplay
            };
        }
    }

    validate_graph_spec_in_doc(doc, &id, &spec)?;
    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    graph.spec = spec;
    graph.initialized = true;

    let payload = json!({
        "sheet_id": id.as_str(),
        "enabled": graph.spec.compact_letter_display.is_some(),
        "pairwise_display_mode": graph.spec.pairwise_display_mode,
        "compact_letter_display": graph.spec.compact_letter_display,
    });
    Ok(CommandOutcome::mutated("graph.setCompactLetterDisplay", format!("updated compact letter display on `{id}`"), payload))
}

fn parse_bracket_style(raw: Option<&str>) -> Result<BracketStyle, EngineError> {
    match raw.unwrap_or("square").trim().to_ascii_lowercase().as_str() {
        "square" | "capped" | "ticks" => Ok(BracketStyle::Square),
        "half_tick" => Ok(BracketStyle::HalfTick),
        "flat" | "flat_line" | "plain" => Ok(BracketStyle::FlatLine),
        "curly" | "curly_brace" => Ok(BracketStyle::CurlyBrace),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid bracket `style` `{other}`"))),
    }
}

fn parse_bracket_label_mode(val: Option<&Value>) -> Result<BracketLabelMode, EngineError> {
    let Some(v) = val else {
        return Ok(BracketLabelMode::SummaryStars);
    };
    if let Ok(mode) = serde_json::from_value::<BracketLabelMode>(v.clone()) {
        return Ok(mode);
    }
    match v.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("stars" | "summary_stars" | "stars_only") => Ok(BracketLabelMode::SummaryStars),
        Some("numeric" | "p_value" | "p_only" | "stars_and_p") => Ok(BracketLabelMode::NumericPOrQ),
        Some(custom) => Ok(BracketLabelMode::Custom(custom.to_owned())),
        None => Err(EngineError::with_code(ErrorCode::BadInput, "invalid `label_mode`")),
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, DataSheet, ErrorCode, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};
    use serde_json::json;

    use super::*;

    fn session_with_xy() -> Session {
        let mut doc = ProjectDocument::new("Graph Test").unwrap();
        let mut table = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
        table.x_cells[0][0] = Cell::number(1.0);
        table.x_cells[1][0] = Cell::number(2.0);
        table.data_sets[0].cells[0][0] = Cell::number(10.0);
        table.data_sets[0].cells[1][0] = Cell::number(20.0);
        doc.add_data_sheet(DataSheet::new("xy1", "Dose Response", TableArchetype::Xy(table)).unwrap()).unwrap();
        Session::from_document(doc)
    }

    #[test]
    fn registry_contains_all_fifteen_graph_commands() {
        assert_eq!(specs().len(), 15);
        for spec in specs() {
            assert_eq!(spec.group, CommandGroup::Graph);
            assert!(!spec.summary.is_empty());
            assert!(!spec.beginner.is_empty());
        }
    }

    #[test]
    fn set_compact_letter_display_validates_and_roundtrips_undo_redo() {
        let mut session = Session::new();
        session.execute("sheet.createData", json!({ "sheet_id": "col1", "archetype": "column", "cols": 3, "rows": 4 })).unwrap();
        session
            .execute(
                "table.setCells",
                json!({
                    "sheet_id": "col1",
                    "cells": [
                        { "row": 0, "col": 0, "value": 9.0 }, { "row": 1, "col": 0, "value": 10.0 }, { "row": 2, "col": 0, "value": 11.0 }, { "row": 3, "col": 0, "value": 10.0 },
                        { "row": 0, "col": 1, "value": 13.0 }, { "row": 1, "col": 1, "value": 14.0 }, { "row": 2, "col": 1, "value": 15.0 }, { "row": 3, "col": 1, "value": 14.0 },
                        { "row": 0, "col": 2, "value": 21.0 }, { "row": 1, "col": 2, "value": 22.0 }, { "row": 2, "col": 2, "value": 23.0 }, { "row": 3, "col": 2, "value": 22.0 }
                    ]
                }),
            )
            .unwrap();
        session.execute("stats.run", json!({ "sheet_id": "res1", "source_sheet": "col1", "kind": "one_way_anova" })).unwrap();
        session.execute("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "col1", "kind": "column" })).unwrap();

        let bad_missing = session.execute("graph.setCompactLetterDisplay", json!({ "sheet_id": "g1", "enabled": true })).unwrap_err();
        assert_eq!(bad_missing.code, ErrorCode::BadInput);

        let out = session
            .execute(
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": "g1",
                    "result_sheet_id": "res1",
                    "ordering": "descending_mean",
                    "uppercase": false,
                    "font_size_pt": 10.0,
                    "offset_pt": 7.0
                }),
            )
            .unwrap();
        assert_eq!(out.payload["enabled"], json!(true));
        let gs = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
        assert_eq!(gs.spec.pairwise_display_mode, PairwiseDisplayMode::CompactLetterDisplayOnly);
        assert_eq!(gs.spec.compact_letter_display.as_ref().unwrap().font_size_pt, 10.0);

        session
            .execute(
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": "g1",
                    "pairwise_display_mode": "brackets_and_compact_letter_display",
                    "uppercase": true,
                    "alpha_override": 0.01
                }),
            )
            .unwrap();
        let gs2 = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
        assert_eq!(gs2.spec.pairwise_display_mode, PairwiseDisplayMode::BracketsAndCompactLetterDisplay);
        assert!(gs2.spec.compact_letter_display.as_ref().unwrap().uppercase);
        assert_eq!(gs2.spec.compact_letter_display.as_ref().unwrap().alpha_override, Some(0.01));

        session.undo().unwrap();
        let gs_undone = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
        assert_eq!(gs_undone.spec.pairwise_display_mode, PairwiseDisplayMode::CompactLetterDisplayOnly);
        assert!(!gs_undone.spec.compact_letter_display.as_ref().unwrap().uppercase);

        session.redo().unwrap();
        assert!(session.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.compact_letter_display.as_ref().unwrap().uppercase);

        session.execute("graph.setCompactLetterDisplay", json!({ "sheet_id": "g1", "alpha_override": null })).unwrap();
        assert_eq!(session.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.compact_letter_display.as_ref().unwrap().alpha_override, None);

        let bad_alpha = session.execute("graph.setCompactLetterDisplay", json!({ "sheet_id": "g1", "alpha_override": 1.2 })).unwrap_err();
        assert_eq!(bad_alpha.code, ErrorCode::BadInput);

        session.execute("graph.setCompactLetterDisplay", json!({ "sheet_id": "g1", "enabled": false })).unwrap();
        let gs_off = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
        assert!(gs_off.spec.compact_letter_display.is_none());
        assert_eq!(gs_off.spec.pairwise_display_mode, PairwiseDisplayMode::BracketsOnly);
    }

    #[test]
    fn formats_selected_points_with_undo_redo_and_clear() {
        let mut session = session_with_xy();
        session.execute("graph.create", json!({ "data_sheet_id": "xy1", "sheet_id": "g1", "kind": "xy" })).unwrap();

        let batch = session
            .execute(
                "graph.formatSelectedPoints",
                json!({
                    "sheet_id": "g1",
                    "points": [
                        { "series_index": 0, "point_index": 0 },
                        { "series_index": 0, "point_index": 1 }
                    ],
                    "color": "#FF0000",
                    "symbol": "diamond",
                    "size_pt": 9.0,
                    "border_width_pt": 1.5
                }),
            )
            .unwrap();
        assert_eq!(batch.payload["override_count"], json!(2));

        let single = session
            .execute(
                "graph.formatSelectedPoints",
                json!({
                    "sheet_id": "g1",
                    "overrides": [
                        { "series_index": 0, "point_index": 1, "hidden": true, "symbol": null }
                    ]
                }),
            )
            .unwrap();
        assert_eq!(single.payload["override_count"], json!(2));
        let gs = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
        let ov0 = gs.spec.point_override(0, 0).unwrap();
        assert_eq!(ov0.symbol, Some(MarkerShape::Diamond));
        assert_eq!(ov0.size_pt, Some(9.0));
        let ov1 = gs.spec.point_override(0, 1).unwrap();
        assert!(ov1.hidden);
        assert_eq!(ov1.symbol, None);

        let scene = compile_scene_for_sheet(session.document(), &SheetId::new("g1")).unwrap();
        assert_eq!(scene.plotted_points.len(), 1);
        assert_eq!(scene.plotted_points[0].source_cell.row, 0);

        session.undo().unwrap();
        let ov1_undone = session.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.point_override(0, 1).unwrap();
        assert!(!ov1_undone.hidden);
        assert_eq!(ov1_undone.symbol, Some(MarkerShape::Diamond));

        session.redo().unwrap();
        assert!(session.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.point_override(0, 1).unwrap().hidden);

        session
            .execute("graph.formatSelectedPoints", json!({ "sheet_id": "g1", "mode": "clear", "points": [{ "series_index": 0, "point_index": 0 }] }))
            .unwrap();
        assert_eq!(session.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.point_overrides.len(), 1);

        session.execute("graph.formatSelectedPoints", json!({ "sheet_id": "g1", "mode": "clear" })).unwrap();
        assert!(session.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.point_overrides.is_empty());

        let bad_size = session
            .execute(
                "graph.formatSelectedPoints",
                json!({
                    "sheet_id": "g1",
                    "points": [{ "series_index": 0, "point_index": 0 }],
                    "size_pt": 0.0
                }),
            )
            .unwrap_err();
        assert_eq!(bad_size.code, ErrorCode::BadInput);
    }

    #[test]
    fn creates_styles_and_audits_graph_with_preflight() {
        let mut session = session_with_xy();
        session.execute("graph.create", json!({ "data_sheet_id": "xy1", "sheet_id": "g1", "kind": "xy" })).unwrap();
        session.execute("graph.setAxes", json!({ "sheet_id": "g1", "x_axis": { "title": "Log Dose" }, "y2_axis": null })).unwrap();
        session.execute("graph.setAdditionalTicks", json!({ "sheet_id": "g1", "axis": "y", "ticks": [{ "value": 50.0, "label": "EC50" }] })).unwrap();
        session.execute("graph.applyCanvasPreset", json!({ "sheet_id": "g1", "preset": "SingleColumn88mm" })).unwrap();
        let pf = session.execute("graph.preflightCheck", json!({ "sheet_id": "g1" })).unwrap();
        assert_eq!(pf.payload["sheet_id"], json!("g1"));

        let bad_hex = session.execute("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "color": "not-a-hex-color" })).unwrap_err();
        assert_eq!(bad_hex.code, ErrorCode::BadInput);
        assert!(bad_hex.message().contains("invalid hex"));
    }
}
