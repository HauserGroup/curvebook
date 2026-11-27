//! Graph creation, styling, axis, series, overlay, bracket, annotation, canvas preset, and preflight commands.

use curvebook_chart::preflight_check_scene;
use curvebook_doc::{
    AdditionalTick, AnnotationShapeKind, AxisSegment, AxisSpec, AxisThreshold, BracketLabelMode, BracketStyle, CanvasResizeMode,
    CategoricalTickSource, CldOrdering, ColorChannelMapping, CompactLetterDisplaySpec, ConnectingLineStyle, DiagnosticPlotKind, DocError,
    EquationDisplayMode, ErrorBarSpec, ErrorCode, FigureCanvasPreset, GraphAnnotation, GraphAxisScale, GraphKind, GraphSheet, GraphSpec,
    GraphicalDataObject, GroupedArrangement, HeatmapLabelSpec, LevelOrderSpec, LineConnectionKind, MAX_SHEETS_PER_PROJECT, MultivariateGraphEncoding,
    MultivariatePlotMode, PairwiseBracketSpec, PairwiseComparisonRecord, PairwiseDisplayMode, PartsOfWholeLayout, PointStyleOverride,
    ProjectDocument, RectPt, RegressionOverlaySpec, ReplicateLineMode, ReplicateOverlayStyle, SeriesSpec, SizeChannelMapping, SizePt, SummaryMeasure,
    SurvivalDisplayMetric, TickLabelFormat, WhiskerMethod,
};
use curvebook_math::LogitDomain;
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::analysis::{de_present_opt, merge_json_struct, strip_keys};
use super::export::{compile_scene_for_sheet, de_opt_u16, de_opt_usize};
use super::helpers::{
    bad_input, bad_input_ptr, de_some, deserialize_patch, has_analyzable_data_sheet, has_graph_or_layout, has_graph_sheet, next_unique_sheet_id,
    next_unique_sheet_id_excluding, parse_serde_enum, parse_serde_enum_str, parse_sheet_id, require_graph_sheet, require_graph_sheet_mut,
    require_result_sheet, sheet_id_exists, validate_point_override_in_bounds,
};
use super::params::{
    any_json_schema, array_schema, bool_schema, deserialize_strict, enum_str_schema, int_schema, non_empty_str_schema, nullable_schema, num_schema,
    obj_schema, parse_params, str_schema,
};
use super::{CommandGroup, CommandSpec, Ctx, Effects, command_specs};
use crate::{CommandOutcome, EngineError};

const MUTATE_IDEMPOTENT: Effects = Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT);
const READ_ONLY_IDEMPOTENT: Effects = Effects::NONE.with(Effects::IDEMPOTENT);

macro_rules! apply_opt {
    ($($src:expr => $dst:expr),+ $(,)?) => {
        $(
            if let Some(val) = $src {
                $dst = val;
            }
        )+
    };
}

fn mutate_graph_sheet<T>(
    doc: &mut ProjectDocument,
    graph_id: &curvebook_doc::SheetId,
    initialize: bool,
    validate_in_doc: bool,
    f: impl FnOnce(&ProjectDocument, &mut GraphSpec) -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    let mut spec = require_graph_sheet(doc, graph_id)?.spec.clone();
    let out = f(doc, &mut spec)?;
    if validate_in_doc {
        validate_graph_spec_in_doc(doc, graph_id, &spec)?;
    } else {
        spec.validate()?;
    }
    let graph = require_graph_sheet_mut(doc, graph_id)?;
    graph.spec = spec;
    if initialize {
        graph.initialized = true;
    }
    Ok(out)
}

/// Return all registered graph configuration and preflight command specifications.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

static SPECS: &[CommandSpec] = &command_specs!(
    CommandGroup::Graph;
    {
        id: "graph.create",
        label: "New Graph",
        menu_path: "Insert/New Graph",
        summary: "Create a new graph sheet bound to an input data sheet with optional kind, title, and initialization state.",
        beginner: "Creates a new graph linked to one of your data tables.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_graph_create,
        enabled: has_analyzable_data_sheet,
        run: run_graph_create,
    },
    {
        id: "graph.initialize",
        label: "Initialize Graph",
        menu_path: "Graph/Initialize",
        summary: "Initialize an uninitialized graph sheet, applying its graph kind and default series styling.",
        beginner: "Chooses an initial plot type for a newly created graph sheet.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_initialize,
        enabled: has_graph_sheet,
        run: run_graph_initialize,
    },
    {
        id: "graph.setKind",
        label: "Change Graph Type",
        menu_path: "Change/Graph Type",
        summary: "Switch a graph's visualization type (XY, column, grouped, survival, multivariate, diagnostic) with optional style reset.",
        beginner: "Changes the chart type of an existing graph without recreating the sheet.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_kind,
        enabled: has_graph_sheet,
        run: run_graph_set_kind,
    },
    {
        id: "graph.setSpec",
        label: "Update Graph Specification",
        menu_path: "Format/Graph",
        summary: "Patch a graph's full GraphSpec, palette, frame, legend, title, survival options, or multivariate encoding.",
        beginner: "Updates visual graph settings such as palette, frame style, legend, or multivariate channels.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_spec,
        enabled: has_graph_sheet,
        run: run_graph_set_spec,
    },
    {
        id: "graph.setAxes",
        label: "Format Axes",
        menu_path: "Format/Axes",
        summary: "Configure X, Y, and right-hand Y2 axes including log/probit/logit scales, segmented breaks, thresholds, and frame presets.",
        beginner: "Adjusts axis limits, log scales, titles, broken-axis gaps, and frame styling.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_axes,
        enabled: has_graph_sheet,
        run: run_graph_set_axes,
    },
    {
        id: "graph.setAdditionalTicks",
        label: "Set Additional Axis Ticks",
        menu_path: "Format/Additional Ticks",
        summary: "Set custom callout ticks and optional gridlines along the `x`, `y`, or `y2` axis.",
        beginner: "Places custom reference ticks and gridlines at specific axis coordinates.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_additional_ticks,
        enabled: has_graph_sheet,
        run: run_graph_set_additional_ticks,
    },
    {
        id: "graph.setSeries",
        label: "Format Data Series",
        menu_path: "Format/Data on Graph",
        summary: "Style one or all data series: colors, markers, connecting lines, error bars, right-Y binding, or multivariate encoding.",
        beginner: "Customizes markers, connecting lines, error bars, and colors for curves or bars.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_series,
        enabled: has_graph_sheet,
        run: run_graph_set_series,
    },
    {
        id: "graph.setRegressionOverlay",
        label: "Overlay Regression Curve",
        menu_path: "Graph/Regression Overlay",
        summary: "Bind or clear a fitted curve overlay with optional confidence/prediction bands on one or all graph series.",
        beginner: "Overlays a curve fit line and confidence ribbons from a result sheet onto your graph.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_regression_overlay,
        enabled: has_graph_sheet,
        run: run_graph_set_regression_overlay,
    },
    {
        id: "graph.addComparisonBrackets",
        label: "Add Pairwise Significance Brackets",
        menu_path: "Draw/Pairwise Comparison Brackets",
        summary: "Add statistical comparison brackets (stars or numeric P/q values) driven by a result sheet's pairwise comparisons.",
        beginner: "Draws significance brackets above compared groups using P values from your statistical test.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_graph_add_comparison_brackets,
        enabled: has_graph_sheet,
        run: run_graph_add_comparison_brackets,
    },
    {
        id: "graph.addAnnotation",
        label: "Add Graph Annotation",
        menu_path: "Draw/Annotation",
        summary: "Add a text box, line/arrow, shape, live embedded result table, or live fitted model equation annotation to a graph.",
        beginner: "Places labels, arrows, geometric callouts, embedded result summaries, or model equations onto a graph.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_graph_add_annotation,
        enabled: has_graph_sheet,
        run: run_graph_add_annotation,
    },
    {
        id: "graph.applyStyleFrom",
        label: "Apply Graph Style From",
        menu_path: "Format/Apply Style From",
        summary: "Copy visual styling (palette, frame, axis styling, legend, and series appearances) from a donor graph.",
        beginner: "Copies the visual style of another graph sheet onto this graph without changing its data.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_apply_style_from,
        enabled: has_graph_sheet,
        run: run_graph_apply_style_from,
    },
    {
        id: "graph.applyCanvasPreset",
        label: "Apply Graph Canvas Preset",
        menu_path: "Format/Canvas Size Preset",
        summary: "Resize a graph canvas to a journal column, slide, or poster preset (`KeepTypographyPt` or `ProportionalScale`).",
        beginner: "Resizes a graph to standard journal figure widths and runs a publication preflight check.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_apply_canvas_preset,
        enabled: has_graph_sheet,
        run: run_graph_apply_canvas_preset,
    },
    {
        id: "graph.preflightCheck",
        label: "Preflight Check Figure",
        menu_path: "Analyze/Preflight Check",
        summary: "Compile a graph or layout sheet and check typography, hairlines, clipping, and overlap readiness.",
        beginner: "Audits a graph or layout for publication print issues like tiny fonts or clipped labels.",
        effects: READ_ONLY_IDEMPOTENT,
        input_schema: schema_graph_preflight_check,
        enabled: has_graph_or_layout,
        run: run_graph_preflight_check,
    },
    {
        id: "graph.formatSelectedPoints",
        label: "Format Selected Points",
        menu_path: "Format/Selected Points",
        summary: "Apply, merge, replace, or clear per-point visual overrides across one or more selected graph points.",
        beginner: "Highlights or hides specific data points (changing color, symbol, size, border, or visibility) without modifying the whole series.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_format_selected_points,
        enabled: has_graph_sheet,
        run: run_graph_format_selected_points,
    },
    {
        id: "graph.setCompactLetterDisplay",
        label: "Compact Letter Display",
        menu_path: "Draw/Compact Letter Display",
        summary: "Configure or toggle Piepho (2004) compact letter display significance grouping letters and pairwise display mode on a graph.",
        beginner: "Places shared grouping letters (a, b, ab) above graph groups to summarize all-pairwise statistical comparisons.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_set_compact_letter_display,
        enabled: has_graph_sheet,
        run: run_graph_set_compact_letter_display,
    },
    {
        id: "graph.insertImage",
        label: "Insert Image on Graph",
        menu_path: "Insert/Image on Graph",
        summary: "Embed a PNG, JPEG, or TIFF raster image annotation onto a graph sheet.",
        beginner: "Inserts a microscopy micrograph, Western blot, or schematic photo onto a graph.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: schema_graph_insert_image,
        enabled: has_graph_sheet,
        run: run_graph_insert_image,
    },
    {
        id: "graph.updateImage",
        label: "Format Embedded Graph Image",
        menu_path: "Format/Embedded Image",
        summary: "Update placement bounds, aspect-ratio lock, rotation, crop insets, or border on an embedded graph image.",
        beginner: "Crops, rotates, resizes, or frames an embedded image on a graph.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_graph_update_image,
        enabled: has_graph_sheet,
        run: run_graph_update_image,
    },
);

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
    if let Some([x_v, y_v, w_v, h_v]) = v.as_array().map(Vec::as_slice) {
        let x = x_v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "rect x must be f64"))?;
        let y = y_v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "rect y must be f64"))?;
        let w = w_v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "rect width must be f64"))?;
        let h = h_v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "rect height must be f64"))?;
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
    obj_schema(
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
        false,
    )
}

// ---------------------------------------------------------------------------
// Schemas & Typed Parameter Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GraphCreateParams {
    data_sheet_id: String,
    sheet_id: Option<String>,
    title: Option<String>,
    kind: Option<Value>,
    initialized: Option<bool>,
    #[serde(default)]
    one_per_data_set: Option<bool>,
    #[serde(default)]
    y_axis_title: Option<String>,
}

fn schema_graph_create() -> Value {
    obj_schema(
        vec![
            ("data_sheet_id", non_empty_str_schema("Source data sheet ID to plot")),
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional explicit graph sheet ID (prefix when creating multiple graphs)"))),
            ("title", nullable_schema(str_schema("Optional graph title"))),
            ("kind", nullable_schema(any_json_schema("Optional initial graph kind string or GraphKind object"))),
            ("initialized", nullable_schema(bool_schema("Whether the graph starts marked as initialized"))),
            ("one_per_data_set", nullable_schema(bool_schema("When true, create one single-series graph sheet per data set in one undo step"))),
            ("y_axis_title", nullable_schema(str_schema("Optional initial Y-axis title"))),
        ],
        &["data_sheet_id"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphInitializeParams {
    sheet_id: String,
    kind: Option<Value>,
    title: Option<String>,
}

fn schema_graph_initialize() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to initialize")),
            ("kind", nullable_schema(any_json_schema("Optional graph kind string or GraphKind object"))),
            ("title", nullable_schema(str_schema("Optional graph title override"))),
        ],
        &["sheet_id"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetKindParams {
    sheet_id: String,
    kind: Value,
    reset_series_style: Option<bool>,
    #[serde(default)]
    option_id: Option<String>,
}

fn schema_graph_set_kind() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("kind", any_json_schema("Replacement graph kind string or typed GraphKind object")),
            ("reset_series_style", nullable_schema(bool_schema("Reinitialize series GDOs to defaults for the new kind (default true)"))),
            ("option_id", nullable_schema(str_schema("Optional UI graph type catalog identifier"))),
        ],
        &["sheet_id", "kind"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetSpecParams {
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
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("spec", nullable_schema(json!({ "type": "object", "description": "Partial or complete GraphSpec JSON patch" }))),
            ("title", nullable_schema(str_schema("Optional updated graph title"))),
            ("initialized", nullable_schema(bool_schema("Optional initialized state"))),
            ("survival_number_at_risk", nullable_schema(bool_schema("Show number-at-risk table under survival plots"))),
            ("paper_fill", nullable_schema(any_json_schema("Outer canvas paper background color or null for transparent"))),
            ("orientation", nullable_schema(enum_str_schema("Cartesian plot orientation", &["vertical", "horizontal"]))),
            ("forest_show_pooled_diamond", nullable_schema(bool_schema("Show pooled summary diamond row on diagnostic forest plots"))),
            ("heatmap_labels", nullable_schema(json!({ "type": "object", "description": "Heatmap row/column label mode and rotation spec" }))),
            ("multivariate", nullable_schema(json!({ "type": "object", "description": "5-channel multivariate encoding specification" }))),
        ],
        &["sheet_id"],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetAxesParams {
    sheet_id: String,
    frame_preset: Option<Value>,
    x_axis: Option<Value>,
    y_axis: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    y2_axis: Option<Option<Value>>,
    axis: Option<String>,
    #[serde(flatten)]
    axis_fields: Map<String, Value>,
}

fn schema_graph_set_axes() -> Value {
    let axis_obj = nullable_schema(
        json!({ "type": "object", "description": "Axis specification fields (title, segments, scale, min, max, tick_format, additional_ticks, etc.)" }),
    );
    obj_schema(
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
    sheet_id: String,
    axis: String,
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
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("axis", non_empty_str_schema("Target axis (`x`, `y`, or `y2`)")),
            ("ticks", array_schema("Ordered custom ticks along the target axis", tick_schema, None, None)),
        ],
        &["sheet_id", "axis", "ticks"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetSeriesParams {
    sheet_id: String,
    #[serde(default, deserialize_with = "de_opt_usize")]
    series_index: Option<usize>,
    #[serde(default, deserialize_with = "de_opt_usize")]
    data_set_index: Option<usize>,
    series: Option<Vec<Value>>,
    mode: Option<String>,
    #[serde(default)]
    all_series: Option<bool>,
    multivariate: Option<Value>,
    #[serde(flatten)]
    patch_fields: Map<String, Value>,
}

fn schema_graph_set_series() -> Value {
    let series_item_schema = json!({
        "type": "object",
        "description": "Series patch object; optionally includes `series_index` (or `data_set_index` when targeting a bound series) plus styling fields.",
        "properties": {
            "series_index": nullable_schema(int_schema("Optional zero-based target series slot index within `spec.series`", Some(0), None)),
            "data_set_index": nullable_schema(int_schema("Data set column index to bind or match against an existing series", Some(0), None)),
        },
        "additionalProperties": true
    });
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Graph sheet ID to update")),
            ("series_index", nullable_schema(int_schema("Zero-based series slot index", Some(0), None))),
            ("data_set_index", nullable_schema(int_schema("Data set column index to match against bound series", Some(0), None))),
            ("all_series", nullable_schema(bool_schema("When true, apply the single-series patch fields to every series on the graph"))),
            (
                "mode",
                nullable_schema(enum_str_schema(
                    "Array update mode when `series` is provided: `patch` (default; patches matching series slots in place and preserves trailing series) or `replace` (replaces the full series list)",
                    &["patch", "replace"],
                )),
            ),
            (
                "series",
                nullable_schema(array_schema(
                    "Batch series patch array (patches existing `spec.series` slots in place by default; pass `mode: \"replace\"` to replace the full list)",
                    series_item_schema,
                    None,
                    None,
                )),
            ),
            ("multivariate", nullable_schema(json!({ "type": "object", "description": "Multivariate 5-channel bubble/scatter encoding" }))),
        ],
        &["sheet_id"],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetRegressionOverlayParams {
    sheet_id: String,
    clear: Option<bool>,
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
    obj_schema(
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
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphAddComparisonBracketsParams {
    sheet_id: String,
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
    obj_schema(
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
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphAddAnnotationParams {
    sheet_id: String,
    annotation: Option<Value>,
    #[serde(flatten)]
    fields: Map<String, Value>,
}

fn schema_graph_add_annotation() -> Value {
    obj_schema(
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
        true,
    )
}

#[derive(Debug, Deserialize)]
struct GraphApplyStyleFromParams {
    #[serde(default)]
    sheet_id: Option<String>,
    #[serde(default)]
    target_sheet_ids: Option<Vec<String>>,
    donor_sheet_id: String,
    #[serde(default)]
    clone_palette: Option<bool>,
    #[serde(default)]
    clone_frame_and_axes: Option<bool>,
    #[serde(default)]
    clone_error_bars_and_legend: Option<bool>,
}

fn schema_graph_apply_style_from() -> Value {
    obj_schema(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Target graph sheet ID"))),
            (
                "target_sheet_ids",
                nullable_schema(array_schema(
                    "Optional batch list of target graph sheet IDs (any entry matching `donor_sheet_id` is skipped cleanly)",
                    non_empty_str_schema("Target graph sheet ID"),
                    None,
                    None,
                )),
            ),
            ("donor_sheet_id", non_empty_str_schema("Donor graph sheet ID to copy styling from")),
            ("clone_palette", nullable_schema(bool_schema("Copy color palette and series colors"))),
            ("clone_frame_and_axes", nullable_schema(bool_schema("Copy frame and axis styling"))),
            ("clone_error_bars_and_legend", nullable_schema(bool_schema("Copy error bar and legend settings"))),
        ],
        &["donor_sheet_id"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ApplyCanvasPresetParams {
    sheet_id: String,
    preset: String,
    resize_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PreflightCheckParams {
    sheet_id: String,
}

fn schema_graph_preflight_check() -> Value {
    obj_schema(vec![("sheet_id", non_empty_str_schema("Graph or layout sheet ID to audit"))], &["sheet_id"], false)
}

fn schema_graph_format_selected_points() -> Value {
    let coord_schema = obj_schema(
        vec![
            ("series_index", int_schema("Zero-based series index", Some(0), None)),
            ("point_index", int_schema("Zero-based data point index within the series", Some(0), None)),
            ("subcol", nullable_schema(int_schema("Optional zero-based replicate subcolumn index", Some(0), None))),
        ],
        &["series_index", "point_index"],
        false,
    );
    let override_item_schema = obj_schema(
        vec![
            ("series_index", int_schema("Zero-based series index", Some(0), None)),
            ("point_index", int_schema("Zero-based data point index within the series", Some(0), None)),
            ("subcol", nullable_schema(int_schema("Optional zero-based replicate subcolumn index", Some(0), None))),
            ("color", nullable_schema(any_json_schema("Override accent/stroke color (hex, color name, [r,g,b], [r,g,b,a], or null to clear)"))),
            (
                "symbol",
                nullable_schema(str_schema(
                    "Override marker shape (circle, square, triangle_up, triangle_down, diamond, cross, plus, star, hexagon, or null)",
                )),
            ),
            ("size_pt", nullable_schema(num_schema("Override positive marker size in points (> 0, or null to clear)", None, None))),
            ("border_width_pt", nullable_schema(num_schema("Override marker border width in points (>= 0, or null to clear)", None, None))),
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

    obj_schema(
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
            ("size_pt", nullable_schema(num_schema("Shared positive marker size in points for batch `points` (or null to clear)", None, None))),
            (
                "border_width_pt",
                nullable_schema(num_schema("Shared marker border width in points for batch `points` (or null to clear)", None, None)),
            ),
            ("fill_color", nullable_schema(any_json_schema("Shared marker fill color for batch `points` (or null to clear)"))),
            ("fill_mode", nullable_schema(str_schema("Shared marker fill mode for batch `points` (or null to clear)"))),
            ("hidden", bool_schema("Shared hidden flag applied across batch `points`")),
        ],
        &["sheet_id"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GraphSetCompactLetterDisplayParams {
    sheet_id: String,
    #[serde(default)]
    enabled: Option<bool>,
    result_sheet_id: Option<String>,
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
    obj_schema(
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
        false,
    )
}

// ---------------------------------------------------------------------------
// Command Runners
// ---------------------------------------------------------------------------

/// Resolve a source sheet ID (`DataSheet` or `ResultSheet`) from raw slices to its backing
/// [`curvebook_doc::DataSheet`], checking a non-failed result sheet's `analyzable_data` before
/// walking upstream sources and stopping with `None` if any traversed result sheet has
/// [`curvebook_doc::ResultStatus::Failed`].
pub(crate) fn resolve_source_data_sheet_slices<'a>(
    data_sheets: &'a [curvebook_doc::DataSheet],
    result_sheets: &'a [curvebook_doc::ResultSheet],
    source_id: &curvebook_doc::SheetId,
) -> Option<&'a curvebook_doc::DataSheet> {
    let mut cursor = source_id;
    let mut visited = std::collections::HashSet::new();
    while visited.insert(cursor) {
        if let Some(ds) = data_sheets.iter().find(|d| &d.id == cursor) {
            return Some(ds);
        }
        let res = result_sheets.iter().find(|r| &r.id == cursor)?;
        if matches!(res.status, curvebook_doc::ResultStatus::Failed { .. }) {
            return None;
        }
        if let Some(ref analyzable) = res.analyzable_data {
            return Some(analyzable);
        }
        let next_src = res.analysis.source_sheet_id.as_ref()?;
        cursor = next_src;
    }
    None
}

/// Resolve a graph or analysis source sheet ID (`DataSheet` or `ResultSheet`) to its backing
/// [`curvebook_doc::DataSheet`], checking a non-failed result sheet's `analyzable_data` before
/// walking upstream sources and stopping with `None` on [`curvebook_doc::ResultStatus::Failed`].
pub(crate) fn resolve_source_data_sheet<'a>(doc: &'a ProjectDocument, source_id: &curvebook_doc::SheetId) -> Option<&'a curvebook_doc::DataSheet> {
    resolve_source_data_sheet_slices(&doc.data_sheets, &doc.result_sheets, source_id)
}

pub(super) fn resolve_graph_create_source_sheet(
    doc: &ProjectDocument,
    source_id: &curvebook_doc::SheetId,
) -> Result<(curvebook_doc::DataSheet, Option<String>), EngineError> {
    if let Some(ds) = doc.data_sheet(source_id) {
        return Ok((ds.clone(), None));
    }
    let Some(initial_res) = doc.result_sheet(source_id) else {
        return Err(curvebook_doc::DocError::SheetNotFound { sheet_id: source_id.clone() }.into());
    };
    if let curvebook_doc::ResultStatus::Failed { code, ref message } = initial_res.status {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("cannot create graph from failed result sheet `{source_id}` ({code}): {message}"),
        )
        .with_remediation("Fix or recalculate the failed analysis before creating a graph from it."));
    }
    if let Some(ref analyzable) = initial_res.analyzable_data {
        return Ok((analyzable.clone(), None));
    }
    let res_title = initial_res.title().to_owned();
    let Some(mut cursor) = initial_res.analysis.source_sheet_id.clone() else {
        return Err(EngineError::with_code(ErrorCode::Conflict, format!("result sheet `{source_id}` is unlinked and has no upstream data sheet")));
    };
    let mut visited = std::collections::HashSet::new();
    visited.insert(source_id.clone());
    while visited.insert(cursor.clone()) {
        if let Some(ds) = doc.data_sheet(&cursor) {
            return Ok((ds.clone(), Some(res_title)));
        }
        if let Some(res) = doc.result_sheet(&cursor) {
            if let curvebook_doc::ResultStatus::Failed { code, ref message } = res.status {
                return Err(EngineError::with_code(
                    ErrorCode::Conflict,
                    format!("cannot create graph from `{source_id}`: upstream result sheet `{cursor}` failed ({code}): {message}"),
                )
                .with_remediation("Fix or recalculate the failed upstream analysis before creating a graph."));
            }
            if let Some(ref analyzable) = res.analyzable_data {
                return Ok((analyzable.clone(), Some(res_title)));
            }
            let Some(next_src) = res.analysis.source_sheet_id.clone() else {
                return Err(EngineError::with_code(
                    ErrorCode::Conflict,
                    format!("result sheet `{source_id}` has no upstream data sheet (upstream result `{cursor}` is unlinked)"),
                ));
            };
            cursor = next_src;
        } else {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("result sheet `{source_id}` has no upstream data sheet (source `{cursor}` is not a data or result sheet)"),
            ));
        }
    }
    Err(EngineError::with_code(ErrorCode::CycleDetected, format!("cyclic dependency detected while resolving upstream data sheet for `{source_id}`")))
}

fn dataset_display_title(ds: &curvebook_doc::DataSheet, idx: usize) -> Option<&str> {
    match &ds.archetype {
        curvebook_doc::TableArchetype::Xy(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        curvebook_doc::TableArchetype::Column(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        curvebook_doc::TableArchetype::Grouped(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        curvebook_doc::TableArchetype::Contingency(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        curvebook_doc::TableArchetype::Survival(t) => t.cohorts.get(idx).map(|c| c.title.as_str()),
        curvebook_doc::TableArchetype::PartsOfWhole(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        curvebook_doc::TableArchetype::MultipleVariables(t) => t.variables.get(idx).map(|v| v.title.as_str()),
        curvebook_doc::TableArchetype::Nested(t) => t.groups.get(idx).map(|g| g.title.as_str()),
    }
    .map(str::trim)
    .filter(|s| !s.is_empty())
}

fn run_graph_create(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_create();
    let params: GraphCreateParams = parse_params(&schema, raw)?;
    let doc = &mut ctx.session.doc;

    let data_id = parse_sheet_id(&params.data_sheet_id)?;
    let (ds, res_title_opt) = resolve_graph_create_source_sheet(doc, &data_id)?;
    let parsed_kind = params.kind.as_ref().map(parse_graph_kind).transpose()?;
    if let Some(ref y_title) = params.y_axis_title {
        curvebook_doc::validate_title(y_title)?;
    }

    if params.one_per_data_set.unwrap_or(false) {
        if let Some(ref title) = params.title {
            curvebook_doc::validate_title(title)?;
        }
        let seed_id = next_unique_sheet_id(doc, "graph")?;
        let mut proto = GraphSheet::new_uninitialized_for_data(seed_id, &ds)?;
        if let Some(kind) = parsed_kind {
            apply_graph_kind(&mut proto.spec, kind, true);
        }
        let n = proto.spec.series.len();
        if n == 0 {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("source sheet `{data_id}` has no plottable data sets for `one_per_data_set`"),
            ));
        }
        let next_total = doc.total_sheet_count().saturating_add(n);
        if next_total > MAX_SHEETS_PER_PROJECT {
            return Err(DocError::TooManySheets { count: next_total, max: MAX_SHEETS_PER_PROJECT }.into());
        }

        let mut reserved_ids: Vec<curvebook_doc::SheetId> = Vec::with_capacity(n);
        let mut candidates: Vec<GraphSheet> = Vec::with_capacity(n);
        for (idx, proto_series) in proto.spec.series.iter().enumerate() {
            let gid = match params.sheet_id.as_deref() {
                Some(raw_base) => {
                    let s = if n == 1 { raw_base.to_owned() } else { format!("{raw_base}_{}", idx + 1) };
                    let parsed = parse_sheet_id(&s)?;
                    if sheet_id_exists(doc, &parsed) || reserved_ids.contains(&parsed) {
                        return Err(DocError::DuplicateSheetId { sheet_id: parsed }.into());
                    }
                    parsed
                }
                None => {
                    let exclude_refs: Vec<&curvebook_doc::SheetId> = reserved_ids.iter().collect();
                    next_unique_sheet_id_excluding(doc, &format!("graph_{}", idx + 1), &exclude_refs)?
                }
            };
            reserved_ids.push(gid.clone());

            let series_title = dataset_display_title(&ds, proto_series.data_set_index)
                .map(str::to_owned)
                .or_else(|| proto_series.legend_label.clone())
                .unwrap_or_else(|| format!("Set {}", idx + 1));
            let title = match params.title.as_deref() {
                Some(t) if n == 1 => t.to_owned(),
                Some(t) => format!("{t} - {series_title}"),
                None => series_title,
            };
            curvebook_doc::validate_title(&title)?;

            let mut graph = proto.clone();
            graph.id = gid;
            graph.title = title.clone();
            graph.spec.title = title;
            graph.spec.primary_source_sheet_id = data_id.clone();
            let mut single_series = proto_series.clone();
            single_series.source_sheet_id = data_id.clone();
            graph.spec.series = vec![single_series];
            if let Some(ref y_title) = params.y_axis_title {
                graph.spec.y_axis.title = y_title.clone();
            }
            graph.initialized =
                if parsed_kind.is_some() { params.initialized.unwrap_or(true) } else { params.initialized.unwrap_or(proto.initialized) };
            validate_graph_spec_in_doc(doc, &graph.id, &graph.spec)?;
            candidates.push(graph);
        }

        let created_ids: Vec<String> = candidates.iter().map(|g| g.id.as_str().to_owned()).collect();
        let first = candidates.first().ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "no graph candidates created"))?;
        let first_id = first.id.as_str().to_owned();
        let first_title = first.title.clone();
        let first_init = first.initialized;
        let first_kind = first.spec.kind;
        for graph in candidates {
            doc.add_graph_sheet(graph)?;
        }

        return Ok(CommandOutcome::mutated(
            "graph.create",
            format!("created {} graph(s) from `{data_id}`", created_ids.len()),
            json!({
                "sheet_id": first_id,
                "sheet_ids": created_ids,
                "created_count": n,
                "data_sheet_id": data_id.as_str(),
                "title": first_title,
                "initialized": first_init,
                "kind": first_kind,
            }),
        ));
    }

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
        graph.spec.title = title.clone();
    }
    if let Some(kind) = parsed_kind {
        apply_graph_kind(&mut graph.spec, kind, true);
        graph.initialized = params.initialized.unwrap_or(true);
    } else if let Some(init) = params.initialized {
        graph.initialized = init;
    }
    if let Some(ref y_title) = params.y_axis_title {
        graph.spec.y_axis.title = y_title.clone();
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

    let graph_kind = mutate_graph_sheet(&mut ctx.session.doc, &id, true, true, |_, updated_spec| {
        if let Some(kind) = kind_opt {
            apply_graph_kind(updated_spec, kind, true);
        }
        if let Some(ref title) = params.title {
            updated_spec.title = title.clone();
        }
        Ok(updated_spec.kind)
    })?;
    if let Some(title) = params.title {
        require_graph_sheet_mut(&mut ctx.session.doc, &id)?.title = title;
    }

    Ok(CommandOutcome::mutated(
        "graph.initialize",
        format!("initialized graph `{id}`"),
        json!({ "sheet_id": id.as_str(), "kind": graph_kind, "initialized": true }),
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
    key: &str,
    parser: fn(&Value) -> Result<T, EngineError>,
) -> Result<OptionalPatch<T>, EngineError> {
    let Some(val) = obj.get(key) else {
        return Ok(OptionalPatch::Keep);
    };
    if val.is_null() {
        return Ok(OptionalPatch::Clear);
    }
    Ok(OptionalPatch::Set(parser(val)?))
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
        color: parse_optional_patch(obj, "color", parse_rgba8)?,
        symbol: parse_optional_patch(obj, "symbol", parse_marker_shape)?,
        size_pt: parse_optional_patch(obj, "size_pt", parse_positive_f64)?,
        border_width_pt: parse_optional_patch(obj, "border_width_pt", parse_non_negative_f64)?,
        fill_color: parse_optional_patch(obj, "fill_color", parse_rgba8)?,
        fill_mode: parse_optional_patch(obj, "fill_mode", parse_marker_fill_mode)?,
        hidden: obj.get("hidden").and_then(Value::as_bool),
    })
}

fn parse_opt_subcol(obj: &Map<String, Value>) -> Result<Option<usize>, EngineError> {
    let Some(val) = obj.get("subcol") else {
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
    if mode != "clear" {
        let doc = ctx.session.document();
        let current_spec = &require_graph_sheet(doc, &id)?.spec;
        for (series_index, point_index, subcol, _) in &entries {
            validate_point_override_in_bounds(doc, current_spec, *series_index, *point_index, *subcol)?;
        }
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

pub(super) fn validate_graph_spec_in_doc(
    doc: &curvebook_doc::ProjectDocument,
    graph_id: &curvebook_doc::SheetId,
    spec: &GraphSpec,
) -> Result<(), EngineError> {
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
        match ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                validate_result_sheet_ref(doc, graph_id, result_sheet_id, &format!("annotations[{idx}].result_sheet_id"))?;
            }
            GraphAnnotation::EmbeddedImage { asset_id, .. } if doc.image_asset(asset_id).is_none() => {
                return Err(EngineError::with_code(ErrorCode::NotFound, format!("annotations[{idx}] image asset `{asset_id}` not found in project")));
            }
            _ => {}
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

    mutate_graph_sheet(&mut ctx.session.doc, &id, true, true, |_, spec| {
        apply_graph_kind_with_option(spec, kind, opt_arrangement, gdo_preset, reset_series);
        Ok(())
    })?;

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

    if params.spec.is_some() && (!params.patch_fields.is_empty() || params.multivariate.is_some() || params.survival_number_at_risk.is_some()) {
        return Err(bad_input_ptr("/spec", "cannot combine `spec` with top-level spec shorthand fields in `graph.setSpec`")
            .with_remediation("Pass fields inside `spec` OR as top-level shorthand parameters, not both."));
    }
    if let Some(ref title) = params.title {
        curvebook_doc::validate_title(title)?;
    }

    let spec_overrode_title = params.spec.as_ref().and_then(Value::as_object).is_some_and(|o| o.contains_key("title"));
    let base_ptr = if params.spec.is_some() { "/spec" } else { "" };
    let patch =
        params.spec.clone().unwrap_or_else(|| strip_keys(&Value::Object(params.patch_fields.clone()), &["sheet_id", "id", "title", "initialized"]));
    let patch_clean = strip_keys(&patch, &["multivariate", "paper_fill", "orientation", "forest_show_pooled_diamond", "heatmap_labels"]);

    let (kind, series_count) = mutate_graph_sheet(&mut ctx.session.doc, &id, false, true, |doc, current_spec| {
        let mut merged = merge_json_struct(current_spec, &patch_clean)?;
        if let Some(cld_val) = merged.get_mut("compact_letter_display")
            && let Some(cld_obj) = cld_val.as_object_mut()
            && let Some(enabled_val) = cld_obj.remove("enabled")
        {
            let enabled = enabled_val.as_bool().ok_or_else(|| {
                bad_input_ptr(&format!("{base_ptr}/compact_letter_display/enabled"), "`compact_letter_display.enabled` must be a boolean")
            })?;
            if !enabled {
                *cld_val = Value::Null;
            }
        }
        let mut new_spec: GraphSpec = deserialize_strict(&merged, base_ptr).map_err(|mut err| {
            if err.remediation.is_none() {
                err.remediation = Some(String::from("Supply valid GraphSpec fields or shorthand parameters matching the schema."));
            }
            err
        })?;

        if let Some(mv_val) = patch.get("multivariate").or(params.multivariate.as_ref()) {
            new_spec.multivariate =
                if mv_val.is_null() { None } else { Some(parse_multivariate_encoding(mv_val, current_spec.multivariate.as_ref())?) };
        }
        if let Some(pf_val) = patch.get("paper_fill").or_else(|| params.patch_fields.get("paper_fill")) {
            new_spec.paper_fill = if pf_val.is_null() { None } else { Some(parse_rgba8(pf_val)?) };
        }
        if let Some(or_val) = patch.get("orientation").or_else(|| params.patch_fields.get("orientation")) {
            new_spec.orientation = parse_serde_enum(or_val, "`orientation`")?;
        }
        if let Some(f_val) = patch.get("forest_show_pooled_diamond").or_else(|| params.patch_fields.get("forest_show_pooled_diamond")) {
            new_spec.forest_show_pooled_diamond = f_val.as_bool().ok_or_else(|| bad_input("`forest_show_pooled_diamond` must be a boolean"))?;
        }
        if let Some(hm_val) = patch.get("heatmap_labels").or_else(|| params.patch_fields.get("heatmap_labels")) {
            new_spec.heatmap_labels = parse_heatmap_label_spec(hm_val, current_spec.heatmap_labels)?;
        }
        apply_opt!(params.survival_number_at_risk => new_spec.survival_number_at_risk);
        if let Some(ref title) = params.title
            && !spec_overrode_title
        {
            new_spec.title = title.clone();
        }
        let explicit_point_overrides = patch.get("point_overrides").is_some();
        if explicit_point_overrides {
            for ov in &new_spec.point_overrides {
                validate_point_override_in_bounds(doc, &new_spec, ov.series_index, ov.point_index, ov.subcol)?;
            }
        } else if new_spec.series != current_spec.series {
            reconcile_point_overrides_on_series_change(doc, &current_spec.series, &mut new_spec);
        }
        *current_spec = new_spec;
        Ok((current_spec.kind, current_spec.series.len()))
    })?;

    let graph = require_graph_sheet_mut(&mut ctx.session.doc, &id)?;
    if let Some(title) = params.title {
        graph.title = title;
    }
    graph.initialized = params.initialized.unwrap_or(true);

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

    let (x_segs, y_segs) = mutate_graph_sheet(&mut ctx.session.doc, &id, true, true, |_, spec| {
        if let Some(ref fp) = params.frame_preset {
            spec.frame_preset = parse_serde_enum(fp, "`frame_preset`")?;
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
                    return Err(bad_input_ptr("/axis", format!("unknown axis target `{other}`; expected `x`, `y`, or `y2`")));
                }
            }
        } else if !params.axis_fields.is_empty() {
            return Err(bad_input_ptr("/axis", "missing `axis` (`x`, `y`, or `y2`) when supplying top-level axis properties").with_remediation(
                "Pass `axis: \"x\" | \"y\" | \"y2\"` alongside top-level axis fields, or nest them under `x_axis` / `y_axis` / `y2_axis`.",
            ));
        }
        Ok((spec.x_axis.segments.len(), spec.y_axis.segments.len()))
    })?;

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

    let mut ticks = Vec::with_capacity(params.ticks.len());
    for (idx, item) in params.ticks.into_iter().enumerate() {
        if !item.value.is_finite() {
            return Err(bad_input_ptr(&format!("/ticks/{idx}/value"), "additional tick `value` must be finite"));
        }
        ticks.push(AdditionalTick { value: item.value, label: item.label, draw_gridline: item.draw_gridline.unwrap_or(false) });
    }
    let tick_count = ticks.len();

    let norm_axis = mutate_graph_sheet(&mut ctx.session.doc, &id, true, false, |_, spec| {
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
                return Err(bad_input_ptr("/axis", format!("unknown axis target `{other}`; expected `x`, `y`, or `y2`")));
            }
        };
        Ok(norm_axis)
    })?;

    Ok(CommandOutcome::mutated(
        "graph.setAdditionalTicks",
        format!("updated additional ticks on `{id}` ({norm_axis})"),
        json!({ "sheet_id": id.as_str(), "axis": norm_axis, "tick_count": tick_count }),
    ))
}

fn reconcile_point_overrides_on_series_change(doc: &ProjectDocument, old_series: &[SeriesSpec], spec: &mut GraphSpec) {
    if spec.point_overrides.is_empty() || old_series == spec.series.as_slice() {
        return;
    }
    let retained_series = spec.series.clone();
    let spec_snapshot = spec.clone();
    if old_series.is_empty() {
        spec.point_overrides.retain(|ov| validate_point_override_in_bounds(doc, &spec_snapshot, ov.series_index, ov.point_index, ov.subcol).is_ok());
        return;
    }
    let mut remapped = Vec::with_capacity(spec.point_overrides.len());
    for mut ov in std::mem::take(&mut spec.point_overrides) {
        let Some(prev) = old_series.get(ov.series_index) else {
            continue;
        };
        let Some(new_idx) = retained_series.iter().position(|s| s.source_sheet_id == prev.source_sheet_id && s.data_set_index == prev.data_set_index)
        else {
            continue;
        };
        ov.series_index = new_idx;
        if validate_point_override_in_bounds(doc, &spec_snapshot, ov.series_index, ov.point_index, ov.subcol).is_ok() {
            remapped.push(ov);
        }
    }
    spec.point_overrides = remapped;
}

fn run_graph_set_series(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_set_series();
    let params: GraphSetSeriesParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let replace_mode = parse_series_update_mode(params.mode.as_deref())?;
    let all_series = params.all_series.unwrap_or(false);

    if replace_mode && params.series.is_none() {
        return Err(bad_input_ptr("/mode", "`mode: \"replace\"` requires a `series` array parameter"));
    }
    if all_series && (params.series.is_some() || params.series_index.is_some() || params.data_set_index.is_some()) {
        return Err(bad_input_ptr("/all_series", "`all_series: true` cannot be combined with `series`, `series_index`, or `data_set_index`"));
    }

    let count = mutate_graph_sheet(&mut ctx.session.doc, &id, true, true, |doc, spec| {
        let old_series = spec.series.clone();
        if let Some(ref series_list) = params.series {
            if let Some(first_key) = params.patch_fields.keys().next() {
                return Err(bad_input(format!("unknown field `{first_key}` in graph.setSeries")));
            }
            apply_series_array_update(spec, series_list, replace_mode)?;
        } else if all_series {
            let patch_val = Value::Object(params.patch_fields);
            let parsed_patch = parse_series_patch(&patch_val, false)?;
            if spec.series.is_empty() {
                return Err(EngineError::with_code(ErrorCode::Conflict, format!("graph `{id}` has no series configured")));
            }
            for s in &mut spec.series {
                apply_parsed_series_patch(s, &parsed_patch)?;
            }
        } else {
            let has_patch = !params.patch_fields.is_empty();
            let patch_val = Value::Object(params.patch_fields);
            let parsed_patch = parse_series_patch(&patch_val, false)?;
            if params.series_index.is_some() || params.data_set_index.is_some() || has_patch {
                let target_idx = resolve_series_target_index(spec, params.series_index, params.data_set_index)?;
                if let Some(s) = spec.series.get_mut(target_idx) {
                    apply_parsed_series_patch(s, &parsed_patch)?;
                }
            }
        }

        if let Some(ref mv_val) = params.multivariate {
            spec.multivariate = if mv_val.is_null() { None } else { Some(parse_multivariate_encoding(mv_val, spec.multivariate.as_ref())?) };
        }

        reconcile_point_overrides_on_series_change(doc, &old_series, spec);
        Ok(spec.series.len())
    })?;

    Ok(CommandOutcome::mutated(
        "graph.setSeries",
        format!("updated series styling on `{id}`"),
        json!({ "sheet_id": id.as_str(), "series_count": count }),
    ))
}

fn parse_series_update_mode(raw: Option<&str>) -> Result<bool, EngineError> {
    let Some(raw_mode) = raw else {
        return Ok(false);
    };
    let norm = raw_mode.trim().to_ascii_lowercase();
    match norm.as_str() {
        "patch" | "merge" => Ok(false),
        "replace" => Ok(true),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `graph.setSeries` mode `{raw_mode}`; expected `patch` or `replace`"))
            .with_details(json!({ "pointer": "/mode" }))),
    }
}

fn parse_item_usize_index(item: &Value, key: &str, pos: usize) -> Result<Option<usize>, EngineError> {
    let Some(raw) = item.get(key) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let val = raw.as_u64().ok_or_else(|| {
        EngineError::with_code(ErrorCode::BadInput, format!("`series[{pos}].{key}` must be a non-negative integer"))
            .with_details(json!({ "pointer": format!("/series/{pos}/{key}") }))
    })?;
    let idx = usize::try_from(val).map_err(|_| {
        EngineError::with_code(ErrorCode::BadInput, format!("`series[{pos}].{key}` is out of range"))
            .with_details(json!({ "pointer": format!("/series/{pos}/{key}") }))
    })?;
    Ok(Some(idx))
}

fn apply_series_array_update(spec: &mut GraphSpec, series_list: &[Value], replace_mode: bool) -> Result<(), EngineError> {
    if replace_mode {
        let mut updated_vec = Vec::with_capacity(series_list.len());
        for (pos, item) in series_list.iter().enumerate() {
            if !item.is_object() {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("`series[{pos}]` must be an object"))
                    .with_details(json!({ "pointer": format!("/series/{pos}") })));
            }
            let src_idx = parse_item_usize_index(item, "series_index", pos)?.unwrap_or(pos);
            let mut base = spec.series.get(src_idx).cloned().unwrap_or_else(|| default_series_like(spec, src_idx));
            apply_single_series_patch(&mut base, item, true)?;
            updated_vec.push(base);
        }
        spec.series = updated_vec;
        return Ok(());
    }

    let existing_len = spec.series.len();
    if series_list.len() > existing_len {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!(
                "`series` patch array has {} entry/entries, which exceeds the graph's {existing_len} existing series; pass `mode: \"replace\"` to replace the series list",
                series_list.len()
            ),
        )
        .with_details(json!({ "pointer": "/series", "patch_len": series_list.len(), "series_count": existing_len })));
    }

    for (pos, item) in series_list.iter().enumerate() {
        if !item.is_object() {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`series[{pos}]` must be an object"))
                .with_details(json!({ "pointer": format!("/series/{pos}") })));
        }
        let target_idx = if let Some(explicit_idx) = parse_item_usize_index(item, "series_index", pos)? {
            if explicit_idx >= existing_len {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("`series[{pos}].series_index` ({explicit_idx}) out of bounds ({existing_len} series)"),
                )
                .with_details(json!({ "pointer": format!("/series/{pos}/series_index") })));
            }
            explicit_idx
        } else if series_list.len() < existing_len
            && let Some(ds_idx) = parse_item_usize_index(item, "data_set_index", pos)?
        {
            spec.series.iter().position(|s| s.data_set_index == ds_idx).ok_or_else(|| {
                EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("`series[{pos}].data_set_index` ({ds_idx}) does not match any existing series on this graph"),
                )
                .with_details(json!({ "pointer": format!("/series/{pos}/data_set_index") }))
            })?
        } else {
            pos
        };
        if let Some(slot) = spec.series.get_mut(target_idx) {
            apply_single_series_patch(slot, item, true)?;
        }
    }
    Ok(())
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

    mutate_graph_sheet(&mut ctx.session.doc, &id, true, true, |_, spec| {
        if spec.series.is_empty() {
            return Err(EngineError::with_code(ErrorCode::Conflict, format!("graph `{id}` has no series to bind a regression overlay")));
        }

        if params.all_series.unwrap_or(false) {
            for s in &mut spec.series {
                s.regression_overlay = overlay.clone();
            }
        } else {
            let idx = resolve_series_target_index(spec, params.series_index, params.data_set_index)?;
            if let Some(s) = spec.series.get_mut(idx) {
                s.regression_overlay = overlay;
            }
        }
        Ok(())
    })?;

    Ok(CommandOutcome::mutated(
        "graph.setRegressionOverlay",
        format!("updated regression overlay on `{id}`"),
        json!({ "sheet_id": id.as_str(), "cleared": clear }),
    ))
}

fn upsert_comparison_bracket(comparisons: &mut Vec<PairwiseBracketSpec>, candidate: PairwiseBracketSpec) -> (usize, usize) {
    let key = (candidate.group_a.min(candidate.group_b), candidate.group_a.max(candidate.group_b));
    if let Some(existing) =
        comparisons.iter_mut().find(|b| b.result_sheet_id == candidate.result_sheet_id && (b.group_a.min(b.group_b), b.group_a.max(b.group_b)) == key)
    {
        *existing = candidate;
        (0, 1)
    } else {
        comparisons.push(candidate);
        (1, 0)
    }
}

fn run_graph_add_comparison_brackets(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_add_comparison_brackets();
    let params: GraphAddComparisonBracketsParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let res_id = parse_sheet_id(&params.result_sheet_id)?;
    let comparisons = require_result_sheet(ctx.session.document(), &res_id)?.pairwise_comparisons.clone();

    let style = parse_bracket_style(params.style.as_deref())?;
    let label_mode = parse_bracket_label_mode(params.label_mode.as_ref())?;
    let hide_if_non_significant = params.hide_if_non_significant.unwrap_or(false);
    let stroke_width_pt = params.stroke_width_pt.unwrap_or(0.9);
    let font_size_pt = params.font_size_pt.unwrap_or(8.5);
    let bracket_step_pt = params.bracket_step_pt.unwrap_or(12.0);

    let (added, updated, total) = mutate_graph_sheet(&mut ctx.session.doc, &id, true, true, |_, spec| {
        if params.clear_existing.unwrap_or(false) {
            spec.comparisons.clear();
        }
        let (added, updated) = match (params.group_a.as_ref(), params.group_b.as_ref()) {
            (Some(ga_val), Some(gb_val)) => {
                let ga = resolve_group_index(ga_val, &comparisons)?;
                let gb = resolve_group_index(gb_val, &comparisons)?;
                if ga == gb {
                    return Err(bad_input(format!("comparison bracket requires two distinct groups, got index {ga} for both")));
                }
                upsert_comparison_bracket(
                    &mut spec.comparisons,
                    PairwiseBracketSpec {
                        result_sheet_id: res_id,
                        group_a: ga,
                        group_b: gb,
                        style,
                        label_mode,
                        hide_if_non_significant,
                        stroke_width_pt,
                        font_size_pt,
                        bracket_step_pt,
                    },
                )
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(bad_input("both `group_a` and `group_b` must be provided together to add a single comparison bracket")
                    .with_remediation("Pass both `group_a` and `group_b` for a single pair, or omit both to add all comparisons."));
            }
            (None, None) => {
                let significant_only = params.significant_only.unwrap_or(false);
                let mut added = 0_usize;
                let mut updated = 0_usize;
                for cmp in comparisons {
                    if significant_only && !cmp.significant {
                        continue;
                    }
                    let (da, du) = upsert_comparison_bracket(
                        &mut spec.comparisons,
                        PairwiseBracketSpec {
                            result_sheet_id: res_id.clone(),
                            group_a: cmp.group_a,
                            group_b: cmp.group_b,
                            style,
                            label_mode: label_mode.clone(),
                            hide_if_non_significant,
                            stroke_width_pt,
                            font_size_pt,
                            bracket_step_pt,
                        },
                    );
                    added += da;
                    updated += du;
                }
                (added, updated)
            }
        };
        Ok((added, updated, spec.comparisons.len()))
    })?;

    Ok(CommandOutcome::mutated(
        "graph.addComparisonBrackets",
        format!("added {added} and updated {updated} comparison bracket(s) on `{id}`"),
        json!({ "sheet_id": id.as_str(), "added": added, "updated": updated, "total_brackets": total }),
    ))
}

fn run_graph_add_annotation(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_add_annotation();
    let params: GraphAddAnnotationParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    if params.annotation.is_some() && !params.fields.is_empty() {
        return Err(bad_input_ptr("/annotation", "cannot combine `annotation` with shorthand annotation fields in `graph.addAnnotation`")
            .with_remediation("Pass either the full `annotation` object OR top-level shorthand fields, not both."));
    }

    let total = mutate_graph_sheet(&mut ctx.session.doc, &id, true, true, |doc, spec| {
        let annotation = if let Some(ref ann_val) = params.annotation {
            let ann: GraphAnnotation = deserialize_strict(ann_val, "/annotation").map_err(|err| bad_input(format!("invalid `annotation`: {err}")))?;
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
        spec.annotations.push(annotation);
        Ok(spec.annotations.len())
    })?;

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
    let donor_id = parse_sheet_id(&params.donor_sheet_id)?;
    let donor_spec = require_graph_sheet(ctx.session.document(), &donor_id)?.spec.clone();

    let has_batch = params.target_sheet_ids.is_some();
    let mut raw_targets = Vec::new();
    if let Some(ref single) = params.sheet_id {
        raw_targets.push(parse_sheet_id(single)?);
    }
    if let Some(ref list) = params.target_sheet_ids {
        for raw_id in list {
            raw_targets.push(parse_sheet_id(raw_id)?);
        }
    }
    if raw_targets.is_empty() {
        return Err(
            EngineError::with_code(ErrorCode::BadInput, "provide `sheet_id` or `target_sheet_ids`").with_details(json!({ "pointer": "/sheet_id" }))
        );
    }

    let mut deduped: Vec<curvebook_doc::SheetId> = Vec::with_capacity(raw_targets.len());
    for tid in raw_targets {
        if !deduped.contains(&tid) {
            deduped.push(tid);
        }
    }

    let mut skipped_ids: Vec<String> = Vec::new();
    let mut validated_updates: Vec<(curvebook_doc::SheetId, GraphSpec)> = Vec::new();
    for tid in deduped {
        if has_batch && tid == donor_id {
            skipped_ids.push(tid.as_str().to_owned());
            continue;
        }
        let mut updated = require_graph_sheet(ctx.session.document(), &tid)?.spec.clone();
        updated.apply_style_from_selective(&donor_spec, clone_palette, clone_frame_and_axes, clone_error_bars_and_legend);
        validate_graph_spec_in_doc(ctx.session.document(), &tid, &updated)?;
        validated_updates.push((tid, updated));
    }

    let first_target = validated_updates.first().map(|(id, spec)| (id.as_str().to_owned(), spec.primary_source_sheet_id.as_str().to_owned()));
    let mut applied_ids: Vec<String> = Vec::with_capacity(validated_updates.len());
    for (tid, updated) in validated_updates {
        let target = require_graph_sheet_mut(&mut ctx.session.doc, &tid)?;
        target.spec = updated;
        target.initialized = true;
        applied_ids.push(tid.as_str().to_owned());
    }

    let (primary_target_id, primary_source_sheet_id) =
        first_target.unwrap_or_else(|| (donor_id.as_str().to_owned(), donor_spec.primary_source_sheet_id.as_str().to_owned()));
    let mutated = !applied_ids.is_empty();
    let msg = if let [only_id] = applied_ids.as_slice() {
        format!("applied style from `{donor_id}` to `{only_id}`")
    } else {
        format!("applied style from `{donor_id}` to {} graph(s)", applied_ids.len())
    };
    Ok(CommandOutcome {
        command_id: String::from("graph.applyStyleFrom"),
        mutated,
        message: msg,
        payload: json!({
            "sheet_id": primary_target_id,
            "applied_sheet_ids": applied_ids,
            "skipped_sheet_ids": skipped_ids,
            "donor_sheet_id": donor_id.as_str(),
            "primary_source_sheet_id": primary_source_sheet_id,
            "clone_palette": clone_palette,
            "clone_frame_and_axes": clone_frame_and_axes,
            "clone_error_bars_and_legend": clone_error_bars_and_legend,
        }),
        warnings: Vec::new(),
    })
}

fn run_graph_apply_canvas_preset(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_apply_canvas_preset();
    let params: ApplyCanvasPresetParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let preset = parse_figure_canvas_preset(&params.preset)?;
    let mode = parse_canvas_resize_mode(params.resize_mode.as_deref())?;

    mutate_graph_sheet(&mut ctx.session.doc, &id, true, false, |_, updated_spec| resize_graph_spec(updated_spec, preset.dimensions_pt(), mode))?;

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
        GraphAnnotation::EmbeddedImage { bounds_pt, crop, border, .. } => {
            *bounds_pt = scale_rect_pt(*bounds_pt, sx, sy)?;
            crop.left_pt = scale_finite(crop.left_pt, sx, "annotations.image.crop.left_pt")?;
            crop.right_pt = scale_finite(crop.right_pt, sx, "annotations.image.crop.right_pt")?;
            crop.top_pt = scale_finite(crop.top_pt, sy, "annotations.image.crop.top_pt")?;
            crop.bottom_pt = scale_finite(crop.bottom_pt, sy, "annotations.image.crop.bottom_pt")?;
            if let Some(b) = border {
                scale_stroke_or_clamp(&mut b.width_pt, mode, scale_1d, "annotations.image.border.width_pt")?;
            }
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
    swatches.get(idx % swatches.len()).copied().unwrap_or(Rgba8::BLACK)
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
        "diag.homoscedasticity" | "homoscedasticity" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::HomoscedasticityPlot), None, None)),
        "diag.actual_vs_predicted" | "actual_vs_predicted" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::ActualVsPredictedPlot), None, None)),
        "diag.residual_vs_row" | "residual_vs_row" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::ResidualVsRowPlot), None, None)),
        "diag.gap_statistic" | "gap_statistic" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::GapStatisticPlot), None, None)),
        "diag.param_corr_heatmap" | "param_corr_heatmap" => {
            Some((GraphKind::Diagnostic(DiagnosticPlotKind::ParameterCorrelationHeatmap), None, None))
        }
        "diag.lognormal_qq" | "lognormal_qq" => Some((GraphKind::Diagnostic(DiagnosticPlotKind::LognormalQqPlot), None, None)),
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
        "xy.scatter" | "mv.bubble" => GraphicalDataObject::ScatterAndLine {
            marker: Some(marker),
            line: None,
            area_fill: None,
            replicate_lines: curvebook_doc::ReplicateLineMode::SummaryCenter,
        },
        "xy.scatter_line" => GraphicalDataObject::ScatterAndLine {
            marker: Some(marker),
            line: Some(ConnectingLineStyle { kind: LineConnectionKind::PointToPoint, dash_style: LineDashStyle::Solid, stroke_width_pt: 1.5, color }),
            area_fill: None,
            replicate_lines: curvebook_doc::ReplicateLineMode::SummaryCenter,
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
            replicate_lines: curvebook_doc::ReplicateLineMode::SummaryCenter,
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
            display_metric: curvebook_doc::SurvivalDisplayMetric::PercentSurvival,
            staircase: true,
            error_display: None,
        },
        GraphKind::PartsOfWhole => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Donut, show_center_total: true },
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SegmentPatch {
    #[serde(default, deserialize_with = "de_some")]
    scale: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    min: Option<Option<f64>>,
    #[serde(default, deserialize_with = "de_present_opt")]
    max: Option<Option<f64>>,
    #[serde(default, deserialize_with = "de_some")]
    length_share: Option<f64>,
    #[serde(default, deserialize_with = "de_present_opt")]
    major_tick_interval: Option<Option<f64>>,
    #[serde(default, deserialize_with = "de_some")]
    minor_ticks_per_interval: Option<u8>,
}

impl SegmentPatch {
    const fn any_present(&self) -> bool {
        self.scale.is_some()
            || self.min.is_some()
            || self.max.is_some()
            || self.length_share.is_some()
            || self.major_tick_interval.is_some()
            || self.minor_ticks_per_interval.is_some()
    }

    fn apply(&self, seg: &mut AxisSegment) -> Result<(), EngineError> {
        if let Some(ref s) = self.scale {
            seg.scale = parse_axis_scale(s)?;
        }
        apply_opt!(
            self.min => seg.min,
            self.max => seg.max,
            self.length_share => seg.length_share,
            self.major_tick_interval => seg.major_tick_interval,
            self.minor_ticks_per_interval => seg.minor_ticks_per_interval,
        );
        Ok(())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AxisPatch {
    #[serde(default, deserialize_with = "de_some")]
    title: Option<String>,
    #[serde(default, deserialize_with = "de_some")]
    gap_width_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    break_ornament: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    stroke_width_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    tick_angle_deg: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    tick_format: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    tick_direction: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    show_tick_labels: Option<bool>,
    #[serde(default, deserialize_with = "de_some")]
    segments: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    thresholds: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    additional_ticks: Option<Value>,
    #[serde(flatten)]
    segment: SegmentPatch,
}

fn apply_axis_updates(axis: &mut AxisSpec, val: &Value) -> Result<(), EngineError> {
    let patch: AxisPatch = deserialize_patch(val, "axis patch")?;
    if let Some(ref orn) = patch.break_ornament {
        axis.break_ornament = parse_serde_enum(orn, "`break_ornament`")?;
    }
    if let Some(ref tf) = patch.tick_format {
        axis.tick_format = parse_tick_format(tf)?;
    }
    if let Some(ref td) = patch.tick_direction {
        axis.tick_direction = parse_serde_enum(td, "`tick_direction`")?;
    }
    apply_opt!(
        patch.title => axis.title,
        patch.gap_width_pt => axis.gap_width_pt,
        patch.stroke_width_pt => axis.stroke_width_pt,
        patch.tick_angle_deg => axis.tick_angle_deg,
        patch.show_tick_labels => axis.show_tick_labels,
    );
    if let Some(ref raw_segs) = patch.segments {
        if patch.segment.any_present() {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                "cannot combine `segments` with top-level segment fields (`scale`, `min`, `max`, `length_share`, `major_tick_interval`, `minor_ticks_per_interval`)",
            )
            .with_details(json!({ "pointer": "/segments" })));
        }
        let segs_val = raw_segs.as_array().ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "axis `segments` must be an array").with_details(json!({ "pointer": "/segments" }))
        })?;
        if !(1..=3).contains(&segs_val.len()) {
            return Err(EngineError::with_code(ErrorCode::BadInput, "axis `segments` must contain 1 to 3 segments")
                .with_details(json!({ "pointer": "/segments" })));
        }
        let mut parsed = Vec::with_capacity(segs_val.len());
        for (idx, s) in segs_val.iter().enumerate() {
            let seg_patch: SegmentPatch = deserialize_patch(s, "axis segment")?;
            let mut seg = axis.segments.get(idx).cloned().unwrap_or_default();
            seg_patch.apply(&mut seg)?;
            parsed.push(seg);
        }
        axis.segments = parsed;
    } else if let Some(first_seg) = axis.segments.first_mut() {
        patch.segment.apply(first_seg)?;
    }
    if let Some(ref raw_thresh) = patch.thresholds {
        let thresh_val = raw_thresh.as_array().ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "axis `thresholds` must be an array").with_details(json!({ "pointer": "/thresholds" }))
        })?;
        axis.thresholds = thresh_val.iter().map(parse_axis_threshold).collect::<Result<Vec<_>, _>>()?;
    }
    if let Some(ref add_val) = patch.additional_ticks {
        axis.additional_ticks = parse_additional_ticks(add_val)?;
    }
    axis.validate()?;
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeriesOffsetPatch {
    #[serde(default, deserialize_with = "de_some")]
    x_data: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    x_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    y_pt: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeriesPatch {
    #[serde(default)]
    series_index: Option<Value>,
    #[serde(default)]
    data_set_index: Option<usize>,
    #[serde(default, deserialize_with = "de_some")]
    source_sheet_id: Option<String>,
    #[serde(default, deserialize_with = "de_present_opt")]
    regression_overlay: Option<Option<Value>>,
    #[serde(default, deserialize_with = "de_present_opt")]
    legend_label: Option<Option<String>>,
    #[serde(default, deserialize_with = "de_some")]
    bind_to_right_y2: Option<bool>,
    #[serde(default, deserialize_with = "de_some")]
    gdo: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    color: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    marker: Option<Option<Value>>,
    #[serde(default, deserialize_with = "de_present_opt")]
    connecting_line: Option<Option<Value>>,
    #[serde(default, deserialize_with = "de_present_opt")]
    y_error_bars: Option<Option<Value>>,
    #[serde(default, deserialize_with = "de_present_opt")]
    x_error_bars: Option<Option<Value>>,
    #[serde(default, deserialize_with = "de_some")]
    offset: Option<SeriesOffsetPatch>,
    #[serde(default, deserialize_with = "de_some")]
    offset_x_data: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    offset_x_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    offset_y_pt: Option<f64>,
    // Accept `bar_width_override_pt` so serialized `curvebook_doc::SeriesSpec` objects round-trip through `graph.setSeries`.
    #[serde(default, alias = "bar_width_override_pt", deserialize_with = "de_present_opt")]
    bar_width_pt: Option<Option<f64>>,
    #[serde(default, deserialize_with = "de_some")]
    show_raw_data: Option<bool>,
    #[serde(default, deserialize_with = "de_some")]
    border_width_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    replicate_lines: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    display_metric: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    staircase: Option<bool>,
    #[serde(default, deserialize_with = "de_present_opt")]
    error_display: Option<Option<Value>>,
    #[serde(default, deserialize_with = "de_some")]
    show_greenwood_ci: Option<bool>,
    #[serde(default, deserialize_with = "de_some")]
    show_censored_ticks: Option<bool>,
    #[serde(default, deserialize_with = "de_some")]
    ci_dashed: Option<bool>,
    #[serde(default, deserialize_with = "de_some")]
    alpha: Option<Value>,
}

fn parse_series_patch(val: &Value, allow_slot_keys: bool) -> Result<SeriesPatch, EngineError> {
    let patch: SeriesPatch = deserialize_patch(val, "series patch")?;
    if !allow_slot_keys && (patch.series_index.is_some() || patch.data_set_index.is_some()) {
        return Err(EngineError::with_code(ErrorCode::BadInput, "unknown slot index field in top-level series patch"));
    }
    Ok(patch)
}

fn apply_single_series_patch(series: &mut SeriesSpec, val: &Value, allow_slot_keys: bool) -> Result<(), EngineError> {
    let patch = parse_series_patch(val, allow_slot_keys)?;
    apply_parsed_series_patch(series, &patch)
}

fn apply_parsed_series_patch(series: &mut SeriesSpec, patch: &SeriesPatch) -> Result<(), EngineError> {
    if let Some(ref src) = patch.source_sheet_id {
        series.source_sheet_id = parse_sheet_id(src)?;
    }
    apply_opt!(
        patch.data_set_index => series.data_set_index,
        patch.bind_to_right_y2 => series.bind_to_right_y2,
    );
    if let Some(ref overlay_opt) = patch.regression_overlay {
        series.regression_overlay = match overlay_opt {
            None => None,
            Some(v) => Some(deserialize_strict(v, "/regression_overlay").map_err(|err| bad_input(format!("invalid `regression_overlay`: {err}")))?),
        };
    }
    if let Some(ref label_opt) = patch.legend_label {
        series.legend_label = label_opt.clone();
    }
    if let Some(ref gdo_val) = patch.gdo {
        series.gdo = deserialize_strict(gdo_val, "/gdo").map_err(|err| {
            let msg = err.to_string();
            let detail = msg.strip_prefix("invalid command parameters: ").unwrap_or(&msg);
            EngineError::with_code(ErrorCode::BadInput, format!("invalid `gdo`: {detail}"))
        })?;
    }
    if let Some(ref color_val) = patch.color {
        recolor_gdo(&mut series.gdo, parse_rgba8(color_val)?);
    }
    if let Some(ref m_opt) = patch.marker {
        apply_marker_to_gdo(&mut series.gdo, m_opt.as_ref().unwrap_or(&Value::Null))?;
    }
    if let Some(ref l_opt) = patch.connecting_line {
        apply_line_to_gdo(&mut series.gdo, l_opt.as_ref().unwrap_or(&Value::Null))?;
    }
    if let Some(ref eb_opt) = patch.y_error_bars {
        series.y_error_bars = patch_error_bars(series.y_error_bars, eb_opt.as_ref().unwrap_or(&Value::Null))?;
    }
    if let Some(ref xeb_opt) = patch.x_error_bars {
        series.x_error_bars = patch_error_bars(series.x_error_bars, xeb_opt.as_ref().unwrap_or(&Value::Null))?;
    }
    apply_extended_series_fields(series, patch)
}

fn apply_extended_series_fields(series: &mut SeriesSpec, patch: &SeriesPatch) -> Result<(), EngineError> {
    if let Some(ref off) = patch.offset {
        apply_opt!(off.x_data => series.offset.x_data, off.x_pt => series.offset.x_pt, off.y_pt => series.offset.y_pt);
    }
    apply_opt!(
        patch.offset_x_data => series.offset.x_data,
        patch.offset_x_pt => series.offset.x_pt,
        patch.offset_y_pt => series.offset.y_pt,
        patch.show_raw_data => series.show_raw_data,
    );
    if let Some(bw_opt) = patch.bar_width_pt {
        if let Some(w) = bw_opt
            && w <= 0.0
        {
            return Err(bad_input("`bar_width_pt` must be > 0.0"));
        }
        series.bar_width_override_pt = bw_opt;
    }
    if let Some(w) = patch.border_width_pt {
        if w < 0.0 {
            return Err(bad_input("`border_width_pt` must be >= 0.0"));
        }
        if let GraphicalDataObject::BarWithReplicates { border_width_pt, .. } | GraphicalDataObject::FloatingBar { border_width_pt, .. } =
            &mut series.gdo
        {
            *border_width_pt = w;
        }
    }
    if let Some(ref rl_val) = patch.replicate_lines {
        let mode: ReplicateLineMode = parse_serde_enum(rl_val, "`replicate_lines`")?;
        if let GraphicalDataObject::ScatterAndLine { replicate_lines, .. } = &mut series.gdo {
            *replicate_lines = mode;
        }
    }
    let parsed_dm: Option<SurvivalDisplayMetric> = patch.display_metric.as_ref().map(|v| parse_serde_enum(v, "KM `display_metric`")).transpose()?;
    let parsed_err = patch
        .error_display
        .as_ref()
        .map(|opt| match opt {
            None => Ok(None),
            Some(v) if v.as_str().is_some_and(|s| s.trim().eq_ignore_ascii_case("none")) => Ok(None),
            Some(v) => parse_serde_enum(v, "error bar `metric`").map(Some),
        })
        .transpose()?;

    if let GraphicalDataObject::KaplanMeierStep {
        show_censored_ticks, show_greenwood_ci, ci_dashed, display_metric, staircase, error_display, ..
    } = &mut series.gdo
    {
        apply_opt!(
            parsed_dm => *display_metric,
            patch.staircase => *staircase,
            parsed_err => *error_display,
            patch.show_greenwood_ci => *show_greenwood_ci,
            patch.show_censored_ticks => *show_censored_ticks,
            patch.ci_dashed => *ci_dashed,
        );
    }
    if let Some(ref alpha_val) = patch.alpha {
        apply_alpha_to_gdo(&mut series.gdo, parse_alpha_byte(alpha_val)?);
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
        _ => {
            let mut dummy = MarkerStyle::default();
            apply_marker_patch(&mut dummy, val)?;
        }
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectingLinePatch {
    #[serde(default, deserialize_with = "de_some")]
    kind: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    dash_style: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    stroke_width_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    color: Option<Value>,
}

fn apply_line_to_gdo(gdo: &mut GraphicalDataObject, val: &Value) -> Result<(), EngineError> {
    if val.is_null() {
        if let GraphicalDataObject::ScatterAndLine { line, .. } = gdo {
            *line = None;
        }
        return Ok(());
    }
    let patch: ConnectingLinePatch = deserialize_patch(val, "`connecting_line`")?;
    let mut is_none_kind = false;
    let mut parsed_kind: Option<LineConnectionKind> = None;
    if let Some(ref k) = patch.kind {
        if k.as_str().is_some_and(|s| s.eq_ignore_ascii_case("none")) {
            is_none_kind = true;
        } else {
            parsed_kind = Some(parse_serde_enum(k, "connecting line `kind`")?);
        }
    }
    let parsed_dash = patch.dash_style.as_ref().map(parse_line_dash_style).transpose()?;
    let parsed_color = patch.color.as_ref().map(parse_rgba8).transpose()?;

    if let GraphicalDataObject::ScatterAndLine { line, .. } = gdo {
        if is_none_kind {
            *line = None;
            return Ok(());
        }
        let mut style = line.unwrap_or(ConnectingLineStyle {
            kind: LineConnectionKind::PointToPoint,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.5,
            color: Rgba8::BLACK,
        });
        if let Some(k) = parsed_kind {
            style.kind = k;
        }
        if let Some(d) = parsed_dash {
            style.dash_style = d;
        }
        if let Some(w) = patch.stroke_width_pt {
            style.stroke_width_pt = w;
        }
        if let Some(c) = parsed_color {
            style.color = c;
        }
        *line = Some(style);
    } else if let GraphicalDataObject::KaplanMeierStep { line_color, stroke_width_pt, .. } = gdo
        && !is_none_kind
    {
        if let Some(c) = parsed_color {
            *line_color = c;
        }
        if let Some(w) = patch.stroke_width_pt {
            *stroke_width_pt = w;
        }
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerPatch {
    #[serde(default, deserialize_with = "de_some")]
    shape: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    fill_mode: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    size_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    fill_color: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    stroke_color: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    stroke_width_pt: Option<f64>,
}

fn apply_marker_patch(marker: &mut MarkerStyle, val: &Value) -> Result<(), EngineError> {
    let patch: MarkerPatch = deserialize_patch(val, "`marker`")?;
    if let Some(ref shape) = patch.shape {
        marker.shape = parse_marker_shape(shape)?;
    }
    if let Some(ref fill_mode) = patch.fill_mode {
        marker.fill_mode = parse_marker_fill_mode(fill_mode)?;
    }
    if let Some(ref fc) = patch.fill_color {
        marker.fill_color = parse_rgba8(fc)?;
    }
    if let Some(ref sc) = patch.stroke_color {
        marker.stroke_color = parse_rgba8(sc)?;
    }
    apply_opt!(patch.size_pt => marker.size_pt, patch.stroke_width_pt => marker.stroke_width_pt);
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorBarPatch {
    #[serde(default, deserialize_with = "de_some")]
    metric: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    direction: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    dash_style: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    stroke_width_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    cap_width_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_present_opt")]
    color: Option<Option<Value>>,
}

fn patch_error_bars(current: Option<ErrorBarSpec>, val: &Value) -> Result<Option<ErrorBarSpec>, EngineError> {
    if val.is_null() {
        return Ok(None);
    }
    let patch: ErrorBarPatch = deserialize_patch(val, "`error_bars`")?;
    let mut clear_bars = false;
    let mut eb = current.unwrap_or_default();
    if let Some(ref m) = patch.metric {
        if m.as_str().is_some_and(|s| s.trim().eq_ignore_ascii_case("none")) {
            clear_bars = true;
        } else {
            eb.metric = parse_serde_enum(m, "error bar `metric`")?;
        }
    }
    if let Some(ref d) = patch.direction {
        eb.direction = parse_serde_enum(d, "error bar `direction`")?;
    }
    if let Some(ref dash) = patch.dash_style {
        eb.dash_style = parse_line_dash_style(dash)?;
    }
    apply_opt!(patch.stroke_width_pt => eb.stroke_width_pt, patch.cap_width_pt => eb.cap_width_pt);
    if let Some(ref c_opt) = patch.color {
        eb.color = c_opt.as_ref().map(parse_rgba8).transpose()?;
    }
    if clear_bars { Ok(None) } else { Ok(Some(eb)) }
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
        offset: curvebook_doc::SeriesOffsetSpec::default(),
        bar_width_override_pt: None,
        show_raw_data: true,
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MultivariateEncodingPatch {
    #[serde(default, deserialize_with = "de_some")]
    plot_mode: Option<MultivariatePlotMode>,
    #[serde(default, deserialize_with = "de_some")]
    x_column: Option<usize>,
    #[serde(default, deserialize_with = "de_some")]
    y_column: Option<usize>,
    #[serde(default, deserialize_with = "de_some")]
    color: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    size: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    label_column: Option<Option<usize>>,
    #[serde(default, deserialize_with = "de_present_opt")]
    x_level_order_override: Option<Option<LevelOrderSpec>>,
    #[serde(default, deserialize_with = "de_some")]
    show_convex_hulls: Option<bool>,
    #[serde(default, deserialize_with = "de_some")]
    show_confidence_ellipses: Option<bool>,
}

fn parse_multivariate_encoding(val: &Value, current: Option<&MultivariateGraphEncoding>) -> Result<MultivariateGraphEncoding, EngineError> {
    if let Ok(full) = deserialize_strict::<MultivariateGraphEncoding>(val, "/multivariate") {
        return Ok(full);
    }
    let patch: MultivariateEncodingPatch = deserialize_patch(val, "`multivariate`")?;
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
    apply_opt!(
        patch.plot_mode => base.plot_mode,
        patch.x_column => base.x_column,
        patch.y_column => base.y_column,
        patch.label_column => base.label_column,
        patch.x_level_order_override => base.x_level_order_override,
        patch.show_convex_hulls => base.show_convex_hulls,
        patch.show_confidence_ellipses => base.show_confidence_ellipses,
    );
    if let Some(ref color_v) = patch.color {
        base.color = parse_color_channel(color_v)?;
    }
    if let Some(ref size_v) = patch.size {
        base.size = parse_size_channel(size_v)?;
    }
    Ok(base)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColorChannelShorthand {
    column: usize,
    #[serde(default)]
    categorical: bool,
}

fn parse_color_channel(val: &Value) -> Result<ColorChannelMapping, EngineError> {
    if let Ok(typed) = deserialize_strict::<ColorChannelMapping>(val, "/multivariate/color") {
        return Ok(typed);
    }
    if let Some(col) = val.as_u64() {
        let column = usize::try_from(col).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "`multivariate.color` column out of range"))?;
        return Ok(ColorChannelMapping::Continuous {
            column,
            ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
            domain_clamp: None,
        });
    }
    if val.as_object().is_some_and(|o| o.contains_key("column") || o.contains_key("categorical")) {
        let short: ColorChannelShorthand = deserialize_patch(val, "`multivariate.color`")?;
        if short.categorical {
            return Ok(ColorChannelMapping::Categorical { column: short.column, palette: PaletteId::OkabeIto, level_order_override: None });
        }
        return Ok(ColorChannelMapping::Continuous {
            column: short.column,
            ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
            domain_clamp: None,
        });
    }
    Ok(ColorChannelMapping::Constant(parse_rgba8(val)?))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeChannelVariableShorthand {
    column: usize,
    #[serde(default, deserialize_with = "de_some")]
    min_radius_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_some")]
    max_radius_pt: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeChannelConstShorthand {
    radius_pt: f64,
}

fn parse_size_channel(val: &Value) -> Result<SizeChannelMapping, EngineError> {
    if let Ok(typed) = deserialize_strict::<SizeChannelMapping>(val, "/multivariate/size") {
        return Ok(typed);
    }
    if let Some(obj) = val.as_object() {
        if obj.contains_key("column") {
            let v: SizeChannelVariableShorthand = deserialize_patch(val, "`multivariate.size`")?;
            return Ok(SizeChannelMapping::Variable {
                column: v.column,
                min_radius_pt: v.min_radius_pt.unwrap_or(3.0),
                max_radius_pt: v.max_radius_pt.unwrap_or(14.0),
            });
        }
        let c: SizeChannelConstShorthand = deserialize_patch(val, "`multivariate.size`")?;
        return Ok(SizeChannelMapping::Constant { radius_pt: c.radius_pt });
    }
    if val.is_number() {
        return Ok(SizeChannelMapping::Constant { radius_pt: parse_finite_f64(val, "`multivariate.size`")? });
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "invalid multivariate `size` mapping"))
}

fn ensure_annotation_keys(obj: &Map<String, Value>, allowed: &[&str], kind: &str) -> Result<(), EngineError> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(bad_input_ptr(&format!("/{key}"), format!("unknown field `{key}` for `{kind}` annotation"))
                .with_remediation(format!("Allowed fields for `{kind}` annotations: {}.", allowed.join(", "))));
        }
    }
    Ok(())
}

fn opt_ann_f64(obj: &Map<String, Value>, key: &str, default: f64) -> Result<f64, EngineError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad_input_ptr(&format!("/{key}"), format!("`{key}` must be a finite number"))),
    }
}

fn opt_ann_bool(obj: &Map<String, Value>, key: &str, default: bool) -> Result<bool, EngineError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| bad_input_ptr(&format!("/{key}"), format!("`{key}` must be a boolean"))),
    }
}

fn opt_ann_str<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, EngineError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| bad_input_ptr(&format!("/{key}"), format!("`{key}` must be a string"))),
    }
}

fn parse_annotation_shorthand(doc: &ProjectDocument, params: &Value) -> Result<GraphAnnotation, EngineError> {
    let obj = params.as_object().ok_or_else(|| bad_input("annotation parameters must be an object"))?;
    let kind = opt_ann_str(obj, "kind")?.unwrap_or("text").trim().to_ascii_lowercase();
    match kind.as_str() {
        "text" | "text_box" | "textbox" => {
            ensure_annotation_keys(obj, &["kind", "text", "bounds_pt", "x_pt", "y_pt", "width_pt", "height_pt", "bordered"], &kind)?;
            let markdown = opt_ann_str(obj, "text")?.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing `text`"))?.to_owned();
            let bounds_pt = parse_rect_pt(
                obj.get("bounds_pt").filter(|v| !v.is_null()),
                RectPt::new(
                    opt_ann_f64(obj, "x_pt", 100.0)?,
                    opt_ann_f64(obj, "y_pt", 80.0)?,
                    opt_ann_f64(obj, "width_pt", 120.0)?,
                    opt_ann_f64(obj, "height_pt", 28.0)?,
                ),
            )?;
            let bordered = opt_ann_bool(obj, "bordered", false)?;
            Ok(GraphAnnotation::TextBox { bounds_pt, markdown, bordered })
        }
        "line" | "arrow" => {
            ensure_annotation_keys(obj, &["kind", "start_pt", "end_pt", "arrowhead", "dash", "color"], &kind)?;
            let start_pt = parse_pair_f64(obj.get("start_pt").filter(|v| !v.is_null()), (40.0, 40.0))?;
            let end_pt = parse_pair_f64(obj.get("end_pt").filter(|v| !v.is_null()), (140.0, 40.0))?;
            let arrowhead = opt_ann_bool(obj, "arrowhead", kind == "arrow")?;
            let dash = obj.get("dash").filter(|v| !v.is_null()).map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Solid);
            let color = obj.get("color").filter(|v| !v.is_null()).map(parse_rgba8).transpose()?.unwrap_or(Rgba8::BLACK);
            Ok(GraphAnnotation::LineOrArrow { start_pt, end_pt, arrowhead, dash, color })
        }
        "shape" | "rect" | "ellipse" => {
            ensure_annotation_keys(obj, &["kind", "shape", "bounds_pt", "stroke_color", "fill_color", "stroke_width_pt", "dash"], &kind)?;
            let shape_raw = opt_ann_str(obj, "shape")?.unwrap_or(if kind == "ellipse" { "ellipse" } else { "rectangle" });
            let shape_kind = match shape_raw.trim().to_ascii_lowercase().as_str() {
                "rectangle" | "rect" => AnnotationShapeKind::Rectangle,
                "ellipse" | "circle" => AnnotationShapeKind::Ellipse,
                "rounded_rectangle" | "rounded" => AnnotationShapeKind::RoundedRectangle,
                other => {
                    return Err(bad_input_ptr(
                        "/shape",
                        format!("unknown annotation shape `{other}`; use `rectangle`, `rounded_rectangle`, or `ellipse`"),
                    ));
                }
            };
            let bounds_pt = parse_rect_pt(obj.get("bounds_pt").filter(|v| !v.is_null()), RectPt::new(60.0, 60.0, 80.0, 40.0))?;
            let stroke_color = obj.get("stroke_color").filter(|v| !v.is_null()).map(parse_rgba8).transpose()?.unwrap_or(Rgba8::BLACK);
            let fill_color = obj.get("fill_color").filter(|v| !v.is_null()).map(parse_rgba8).transpose()?;
            let stroke_width_pt = opt_ann_f64(obj, "stroke_width_pt", 1.0)?;
            let dash = obj.get("dash").filter(|v| !v.is_null()).map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Solid);
            Ok(GraphAnnotation::Shape { kind: shape_kind, bounds_pt, stroke_color, stroke_width_pt, dash, fill_color })
        }
        "table" | "embedded_result_table" | "result_table" => {
            ensure_annotation_keys(obj, &["kind", "result_sheet_id", "bounds_pt", "tab_id"], &kind)?;
            let raw_res =
                opt_ann_str(obj, "result_sheet_id")?.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing `result_sheet_id`"))?;
            let res_id = parse_sheet_id(raw_res)?;
            let _ = require_result_sheet(doc, &res_id)?;
            let bounds_pt = parse_rect_pt(obj.get("bounds_pt").filter(|v| !v.is_null()), RectPt::new(180.0, 40.0, 180.0, 90.0))?;
            let tab_id = opt_ann_str(obj, "tab_id")?.map(str::to_owned);
            Ok(GraphAnnotation::EmbeddedResultTable {
                result_sheet_id: res_id,
                tab_id,
                bounds_pt,
                selected_rows: None,
                selected_cols: None,
                style: curvebook_doc::EmbeddedTableStyle::default(),
            })
        }
        "equation" | "model_equation" => {
            ensure_annotation_keys(obj, &["kind", "result_sheet_id", "data_set_index", "mode", "bounds_pt", "font_size_pt", "bordered"], &kind)?;
            let raw_res =
                opt_ann_str(obj, "result_sheet_id")?.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing `result_sheet_id`"))?;
            let res_id = parse_sheet_id(raw_res)?;
            let _ = require_result_sheet(doc, &res_id)?;
            let data_set_index = match obj.get("data_set_index") {
                Some(v) if !v.is_null() => {
                    let raw =
                        v.as_u64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`data_set_index` must be a non-negative integer"))?;
                    usize::try_from(raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "`data_set_index` out of range"))?
                }
                _ => 0,
            };
            let mode = match opt_ann_str(obj, "mode")?.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                None | Some("substituted") => EquationDisplayMode::Substituted,
                Some("symbolic") => EquationDisplayMode::Symbolic,
                Some(other) => {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("unknown model equation mode `{other}`; use `substituted` or `symbolic`"),
                    ));
                }
            };
            let bounds_pt = parse_rect_pt(obj.get("bounds_pt").filter(|v| !v.is_null()), RectPt::new(60.0, 48.0, 220.0, 44.0))?;
            let font_size_pt = opt_ann_f64(obj, "font_size_pt", 10.0)?;
            if !font_size_pt.is_finite() || font_size_pt <= 0.0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`font_size_pt` must be positive and finite"));
            }
            let bordered = opt_ann_bool(obj, "bordered", true)?;
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

fn parse_pair_f64(val: Option<&Value>, def: (f64, f64)) -> Result<(f64, f64), EngineError> {
    let Some(v) = val else {
        return Ok(def);
    };
    if let Some([x_v, y_v]) = v.as_array().map(Vec::as_slice) {
        let x = x_v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "point x must be f64"))?;
        let y = y_v.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "point y must be f64"))?;
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
    parse_serde_enum::<GraphKind>(val, "graph `kind`")
        .or_else(|_| parse_serde_enum::<DiagnosticPlotKind>(val, "graph `kind`").map(GraphKind::Diagnostic))
}

fn parse_axis_scale(val: &Value) -> Result<GraphAxisScale, EngineError> {
    if let Ok(scale) = parse_serde_enum::<GraphAxisScale>(val, "axis `scale`") {
        return Ok(scale);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("log") => Ok(GraphAxisScale::Log10),
        Some("probability") => Ok(GraphAxisScale::Probit),
        Some("logit" | "logit_percent") => Ok(GraphAxisScale::Logit(LogitDomain::Percent)),
        Some("logit_proportion") => Ok(GraphAxisScale::Logit(LogitDomain::Proportion)),
        _ => Err(bad_input(format!("invalid axis `scale`: {val}"))),
    }
}

fn parse_tick_format(val: &Value) -> Result<TickLabelFormat, EngineError> {
    parse_serde_enum::<TickLabelFormat>(val, "`tick_format`")
        .or_else(|_| parse_serde_enum::<CategoricalTickSource>(val, "`tick_format`").map(TickLabelFormat::Categorical))
}

fn parse_finite_f64(val: &Value, label: &str) -> Result<f64, EngineError> {
    let f = val.as_f64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("{label} must be numeric")))?;
    if !f.is_finite() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("{label} must be finite")));
    }
    Ok(f)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeatmapLabelPatch {
    #[serde(default, deserialize_with = "de_some")]
    annotate_cells: Option<bool>,
    #[serde(default, deserialize_with = "de_some")]
    cell_gap_pt: Option<f64>,
    #[serde(default, alias = "row_mode", deserialize_with = "de_some")]
    row_labels: Option<Value>,
    #[serde(default, alias = "column_mode", deserialize_with = "de_some")]
    column_labels: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    column_angle: Option<Value>,
}

fn parse_heatmap_label_spec(val: &Value, mut out: HeatmapLabelSpec) -> Result<HeatmapLabelSpec, EngineError> {
    let patch: HeatmapLabelPatch = deserialize_patch(val, "`heatmap_labels`")?;
    apply_opt!(patch.annotate_cells => out.annotate_cells);
    if let Some(gap) = patch.cell_gap_pt {
        out.cell_gap_pt = gap.clamp(0.0, 12.0);
    }
    if let Some(ref rv) = patch.row_labels {
        out.row_labels = parse_serde_enum(rv, "heatmap `row_labels`")?;
    }
    if let Some(ref cv) = patch.column_labels {
        out.column_labels = parse_serde_enum(cv, "heatmap `column_labels`")?;
    }
    if let Some(ref av) = patch.column_angle {
        out.column_angle = parse_serde_enum(av, "heatmap `column_angle`")?;
    }
    Ok(out)
}

fn parse_alpha_byte(val: &Value) -> Result<u8, EngineError> {
    if let Some(u) = val.as_u64() {
        if u <= 1 {
            return Ok(if u == 1 { 255 } else { 0 });
        }
        return u8::try_from(u).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "`alpha` integer must be in 0..=255"));
    }
    let f = parse_finite_f64(val, "`alpha`")?;
    if !(0.0..=1.0).contains(&f) {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`alpha` float must be in 0.0..=1.0"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((f * 255.0).round() as u8)
}

fn apply_alpha_to_gdo(gdo: &mut GraphicalDataObject, alpha: u8) {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill, .. } => {
            if let Some(m) = marker {
                m.fill_color = m.fill_color.with_alpha(alpha);
            }
            if let Some(l) = line {
                l.color = l.color.with_alpha(alpha);
            }
            if let Some(af) = area_fill {
                af.fill_color = af.fill_color.with_alpha(alpha);
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => *fill_color = fill_color.with_alpha(alpha),
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color = marker.fill_color.with_alpha(alpha),
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => {
            *line_color = line_color.with_alpha(alpha);
        }
        GraphicalDataObject::PartsOfWhole { .. } => {}
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AxisThresholdPatch {
    value: f64,
    #[serde(default)]
    shade_to: Option<f64>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default, deserialize_with = "de_some")]
    color: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    dash_style: Option<Value>,
    #[serde(default, deserialize_with = "de_some")]
    stroke_width_pt: Option<f64>,
}

fn parse_axis_threshold(val: &Value) -> Result<AxisThreshold, EngineError> {
    let patch: AxisThresholdPatch = deserialize_patch(val, "threshold")?;
    Ok(AxisThreshold {
        value: patch.value,
        shade_to: patch.shade_to,
        color: patch.color.as_ref().map(parse_rgba8).transpose()?.unwrap_or_else(|| Rgba8::rgb(120, 120, 120)),
        dash_style: patch.dash_style.as_ref().map(parse_line_dash_style).transpose()?.unwrap_or(LineDashStyle::Dashed),
        stroke_width_pt: patch.stroke_width_pt.unwrap_or(1.0),
        label: patch.label,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdditionalTickPatch {
    value: f64,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    draw_gridline: bool,
}

fn parse_additional_ticks(val: &Value) -> Result<Vec<AdditionalTick>, EngineError> {
    let arr = val.as_array().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`ticks` must be an array"))?;
    arr.iter()
        .map(|item| {
            let patch: AdditionalTickPatch = deserialize_patch(item, "additional tick")?;
            Ok(AdditionalTick { value: patch.value, label: patch.label, draw_gridline: patch.draw_gridline })
        })
        .collect()
}

fn parse_marker_shape(val: &Value) -> Result<MarkerShape, EngineError> {
    if let Ok(s) = parse_serde_enum::<MarkerShape>(val, "marker `shape`") {
        return Ok(s);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("triangle") => Ok(MarkerShape::TriangleUp),
        Some("x") => Ok(MarkerShape::Cross),
        _ => Err(bad_input(format!("invalid marker `shape`: {val}"))),
    }
}

fn parse_marker_fill_mode(val: &Value) -> Result<MarkerFillMode, EngineError> {
    if let Ok(f) = parse_serde_enum::<MarkerFillMode>(val, "marker `fill_mode`") {
        return Ok(f);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("solid") => Ok(MarkerFillMode::Filled),
        Some("hollow") => Ok(MarkerFillMode::Open),
        Some("semi_top") => Ok(MarkerFillMode::SemiFilledTop),
        Some("semi_bottom") => Ok(MarkerFillMode::SemiFilledBottom),
        Some("semi_left") => Ok(MarkerFillMode::SemiFilledLeft),
        Some("semi_right") => Ok(MarkerFillMode::SemiFilledRight),
        _ => Err(bad_input(format!("invalid marker `fill_mode`: {val}"))),
    }
}

pub(super) fn parse_line_dash_style(val: &Value) -> Result<LineDashStyle, EngineError> {
    if let Ok(d) = parse_serde_enum::<LineDashStyle>(val, "`dash_style`") {
        return Ok(d);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("dash") => Ok(LineDashStyle::Dashed),
        Some("dot") => Ok(LineDashStyle::Dotted),
        _ => Err(bad_input(format!("invalid `dash_style`: {val}"))),
    }
}

fn parse_pairwise_display_mode(raw: &str) -> Result<PairwiseDisplayMode, EngineError> {
    parse_serde_enum_str(raw, "`pairwise_display_mode`").map_err(|err| err.with_details(json!({ "pointer": "/pairwise_display_mode" })))
}

fn parse_cld_ordering(raw: &str) -> Result<CldOrdering, EngineError> {
    parse_serde_enum_str(raw, "CLD `ordering`").map_err(|err| err.with_details(json!({ "pointer": "/ordering" })))
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

    let updated = mutate_graph_sheet(&mut ctx.session.doc, &id, true, true, |doc, spec| {
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
                    None => spec
                        .compact_letter_display
                        .take()
                        .ok_or_else(|| bad_input_ptr("/result_sheet_id", "enabling Compact Letter Display requires `result_sheet_id`"))?,
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
        Ok((spec.compact_letter_display.is_some(), spec.pairwise_display_mode, spec.compact_letter_display.clone()))
    })?;

    let payload = json!({
        "sheet_id": id.as_str(),
        "enabled": updated.0,
        "pairwise_display_mode": updated.1,
        "compact_letter_display": updated.2,
    });
    Ok(CommandOutcome::mutated("graph.setCompactLetterDisplay", format!("updated compact letter display on `{id}`"), payload))
}

fn parse_bracket_style(raw: Option<&str>) -> Result<BracketStyle, EngineError> {
    raw.map_or(Ok(BracketStyle::Square), |s| parse_serde_enum_str(s, "bracket `style`"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphInsertImageParams {
    sheet_id: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    bytes_base64: Option<String>,
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    bounds_pt: Option<Value>,
    #[serde(default)]
    rotation_deg: Option<f64>,
    #[serde(default)]
    lock_aspect_ratio: Option<bool>,
    #[serde(default)]
    crop: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    border: Option<Option<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphUpdateImageParams {
    sheet_id: String,
    annotation_index: usize,
    #[serde(default)]
    bounds_pt: Option<Value>,
    #[serde(default)]
    width_pt: Option<f64>,
    #[serde(default)]
    height_pt: Option<f64>,
    #[serde(default)]
    rotation_deg: Option<f64>,
    #[serde(default)]
    lock_aspect_ratio: Option<bool>,
    #[serde(default)]
    crop: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    border: Option<Option<Value>>,
}

pub(super) struct GraphImageInsertArgs<'a> {
    pub command_id: &'static str,
    pub path: Option<&'a str>,
    pub bytes_base64: Option<&'a str>,
    pub asset_id: Option<&'a str>,
    pub bounds_pt: Option<&'a Value>,
    pub rotation_deg: Option<f64>,
    pub lock_aspect_ratio: Option<bool>,
    pub crop: Option<&'a Value>,
    pub border: Option<&'a Option<Value>>,
}

fn schema_graph_insert_image() -> Value {
    let mut props = vec![("sheet_id", non_empty_str_schema("Target GraphSheet ID"))];
    props.extend(super::image::insert_image_common_props());
    obj_schema(props, &["sheet_id"], false)
}

fn schema_graph_update_image() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target GraphSheet ID")),
            ("annotation_index", int_schema("Zero-based index of the EmbeddedImage annotation", Some(0), None)),
            ("bounds_pt", nullable_schema(any_json_schema("Updated placement `[x, y, width, height]` or `RectPt` in points"))),
            (
                "width_pt",
                nullable_schema(num_schema("Updated width in points (> 0); adjusts `height_pt` when aspect ratio is locked", Some(0.001), None)),
            ),
            (
                "height_pt",
                nullable_schema(num_schema("Updated height in points (> 0); adjusts `width_pt` when aspect ratio is locked", Some(0.001), None)),
            ),
            ("rotation_deg", nullable_schema(num_schema("Updated clockwise rotation in degrees", Some(-3600.0), Some(3600.0)))),
            ("lock_aspect_ratio", nullable_schema(bool_schema("Lock or unlock intrinsic aspect ratio during single-axis resize"))),
            ("crop", nullable_schema(super::image::crop_schema())),
            ("border", nullable_schema(super::image::border_schema())),
        ],
        &["sheet_id", "annotation_index"],
        false,
    )
}

pub(super) fn insert_image_on_graph(
    doc: &mut ProjectDocument,
    graph_id: &curvebook_doc::SheetId,
    args: GraphImageInsertArgs<'_>,
) -> Result<CommandOutcome, EngineError> {
    let canvas_size = require_graph_sheet(doc, graph_id)?.spec.canvas_size_pt;
    let resolved = super::image::resolve_image_source(doc, args.path, args.bytes_base64, args.asset_id)?;
    let bounds_pt = super::image::parse_image_insert_bounds(args.bounds_pt, resolved.pixel_size, canvas_size.width, canvas_size.height)?;
    let rotation_deg = super::image::parse_rotation_deg(args.rotation_deg, 0.0)?;
    let lock_aspect_ratio = args.lock_aspect_ratio.unwrap_or(true);
    let crop = super::image::apply_crop_patch(args.crop, bounds_pt, curvebook_doc::CropInsetsPt::default())?;
    let border = super::image::apply_border_patch(args.border, None)?;

    let ann_idx = mutate_graph_sheet(doc, graph_id, true, true, |_, spec| {
        spec.annotations.push(GraphAnnotation::EmbeddedImage {
            asset_id: resolved.asset_id.clone(),
            bounds_pt,
            rotation_deg,
            lock_aspect_ratio,
            crop,
            border,
        });
        Ok(spec.annotations.len() - 1)
    })?;

    Ok(CommandOutcome::mutated(
        args.command_id,
        format!("embedded {} image `{}` onto graph `{graph_id}`", resolved.media_type.mime_type(), resolved.asset_id),
        json!({
            "sheet_id": graph_id.as_str(),
            "annotation_index": ann_idx,
            "asset_id": resolved.asset_id,
            "media_type": resolved.media_type.mime_type(),
            "pixel_size": [resolved.pixel_size.0, resolved.pixel_size.1],
            "bounds_pt": bounds_pt,
        }),
    ))
}

fn run_graph_insert_image(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_insert_image();
    let params: GraphInsertImageParams = parse_params(&schema, raw)?;
    let graph_id = parse_sheet_id(&params.sheet_id)?;
    insert_image_on_graph(
        &mut ctx.session.doc,
        &graph_id,
        GraphImageInsertArgs {
            command_id: "graph.insertImage",
            path: params.path.as_deref(),
            bytes_base64: params.bytes_base64.as_deref(),
            asset_id: params.asset_id.as_deref(),
            bounds_pt: params.bounds_pt.as_ref(),
            rotation_deg: params.rotation_deg,
            lock_aspect_ratio: params.lock_aspect_ratio,
            crop: params.crop.as_ref(),
            border: params.border.as_ref(),
        },
    )
}

fn run_graph_update_image(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_graph_update_image();
    let params: GraphUpdateImageParams = parse_params(&schema, raw)?;
    let graph_id = parse_sheet_id(&params.sheet_id)?;
    let updated_summary = mutate_graph_sheet(&mut ctx.session.doc, &graph_id, false, true, |doc, spec| {
        let ann_len = spec.annotations.len();
        let Some(ann) = spec.annotations.get_mut(params.annotation_index) else {
            return Err(EngineError::with_code(
                ErrorCode::NotFound,
                format!("annotation_index {} out of bounds ({ann_len} annotations) on `{graph_id}`", params.annotation_index),
            )
            .with_details(json!({ "pointer": "/annotation_index" })));
        };
        let GraphAnnotation::EmbeddedImage { asset_id, bounds_pt, rotation_deg, lock_aspect_ratio, crop, border } = ann else {
            return Err(bad_input_ptr(
                "/annotation_index",
                format!("annotation {} on `{graph_id}` is not an embedded image", params.annotation_index),
            ));
        };
        let asset_id_str = asset_id.clone();
        let intrinsic_px = doc.image_asset(&asset_id_str).map(|a| a.pixel_size);
        apply_opt!(params.lock_aspect_ratio => *lock_aspect_ratio);
        super::image::apply_image_bounds_update(
            bounds_pt,
            params.bounds_pt.as_ref(),
            params.width_pt,
            params.height_pt,
            *lock_aspect_ratio,
            intrinsic_px,
        )?;
        *rotation_deg = super::image::parse_rotation_deg(params.rotation_deg, *rotation_deg)?;
        *crop = super::image::apply_crop_patch(params.crop.as_ref(), *bounds_pt, *crop)?;
        *border = super::image::apply_border_patch(params.border.as_ref(), *border)?;
        Ok(json!({
            "sheet_id": graph_id.as_str(),
            "annotation_index": params.annotation_index,
            "asset_id": asset_id_str,
            "bounds_pt": *bounds_pt,
            "rotation_deg": *rotation_deg,
            "lock_aspect_ratio": *lock_aspect_ratio,
            "crop": *crop,
            "border": *border,
        }))
    })?;
    Ok(CommandOutcome::mutated("graph.updateImage", format!("updated embedded image on graph `{graph_id}`"), updated_summary))
}

fn parse_bracket_label_mode(val: Option<&Value>) -> Result<BracketLabelMode, EngineError> {
    let Some(v) = val else {
        return Ok(BracketLabelMode::SummaryStars);
    };
    if let Ok(mode) = parse_serde_enum::<BracketLabelMode>(v, "`label_mode`") {
        return Ok(mode);
    }
    let Some(raw) = v.as_str() else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "invalid `label_mode`"));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`label_mode` custom label must not be empty"));
    }
    Ok(BracketLabelMode::Custom(trimmed.to_owned()))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, DataSheet, ErrorCode, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};
    use serde_json::json;

    use super::*;
    use crate::Session;

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
    fn registry_contains_all_seventeen_graph_commands() {
        assert_eq!(specs().len(), 17);
        for spec in specs() {
            assert_eq!(spec.group, CommandGroup::Graph);
            assert!(!spec.summary.is_empty());
            assert!(!spec.beginner.is_empty());
        }
    }

    #[test]
    fn set_compact_letter_display_validates_and_roundtrips_undo_redo() {
        let mut session = Session::new();
        session.execute("sheet.createData", json!({ "sheet_id": "col1", "kind": "column", "cols": 3, "rows": 4 })).unwrap();
        session
            .execute(
                "table.setCells",
                json!({
                    "sheet_id": "col1",
                    "edits": [
                        { "row": 0, "col": 0, "value": 9.0 }, { "row": 1, "col": 0, "value": 10.0 }, { "row": 2, "col": 0, "value": 11.0 }, { "row": 3, "col": 0, "value": 10.0 },
                        { "row": 0, "col": 1, "value": 13.0 }, { "row": 1, "col": 1, "value": 14.0 }, { "row": 2, "col": 1, "value": 15.0 }, { "row": 3, "col": 1, "value": 14.0 },
                        { "row": 0, "col": 2, "value": 21.0 }, { "row": 1, "col": 2, "value": 22.0 }, { "row": 2, "col": 2, "value": 23.0 }, { "row": 3, "col": 2, "value": 22.0 }
                    ]
                }),
            )
            .unwrap();
        session.execute("stats.run", json!({ "sheet_id": "res1", "source_sheet_id": "col1", "kind": "one_way_anova" })).unwrap();
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
