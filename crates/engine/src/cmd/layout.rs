//! Multi-panel publication layout creation, panel composition, alignment, and canvas preset commands.

use curvebook_chart::preflight_check_scene;
use curvebook_doc::{
    CanvasResizeMode, DocError, ErrorCode, FigureCanvasPreset, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MAX_LAYOUT_ITEMS,
    PageOrientation, PanelBadgeStyle, PhysicalUnit, RectPt,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::analysis::{de_present_opt, merge_json_struct};
use super::export::{compile_scene_for_sheet, de_opt_usize};
use super::graph::{
    parse_canvas_resize_mode, parse_figure_canvas_preset, parse_rect_pt, parse_rgba8, safe_scale_ratio, scale_finite, scale_rect_pt,
    schema_apply_canvas_preset,
};
use super::helpers::{next_unique_sheet_id, parse_sheet_id, require_graph_sheet, require_layout_sheet, require_layout_sheet_mut};
use super::params::{
    any_json_schema, array_schema, bool_schema, enum_str_schema, int_schema, non_empty_str_schema, nullable_schema, num_schema, obj_schema,
    obj_schema_with_aliases, parse_params, str_schema,
};
use super::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::{CommandOutcome, EngineError, Session};

const MUTATE_IDEMPOTENT: Effects = Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT);

/// Return all registered publication layout command specifications.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

static SPECS: &[CommandSpec] = &[
    CommandSpec {
        id: "layout.create",
        label: "New Layout",
        menu_path: "Insert/New Layout...",
        shortcut: "",
        summary: "Create a new publication layout sheet with a journal width preset, page orientation, and grid template.",
        beginner: "Creates a multi-panel figure page where you can arrange graphs, tables, and text callouts.",
        group: CommandGroup::Layout,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_layout_create,
        enabled: always_enabled,
        run: run_layout_create,
    },
    CommandSpec {
        id: "layout.setSpec",
        label: "Format Layout",
        menu_path: "Format/Layout...",
        shortcut: "",
        summary: "Update a layout sheet's page dimensions, journal preset, grid template, origin alignment, badge style, or background.",
        beginner: "Changes the page setup, grid arrangement, or panel lettering of an existing figure layout.",
        group: CommandGroup::Layout,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_layout_set_spec,
        enabled: has_layout_sheet,
        run: run_layout_set_spec,
    },
    CommandSpec {
        id: "layout.addPanel",
        label: "Add Panel to Layout",
        menu_path: "Insert/Layout Panel...",
        shortcut: "",
        summary: "Place a live graph panel, live table panel, or markdown text callout onto a layout sheet.",
        beginner: "Adds a graph, data or result table, or label callout to an explicit rectangle or grid slot on a layout.",
        group: CommandGroup::Layout,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_layout_add_panel,
        enabled: has_layout_sheet,
        run: run_layout_add_panel,
    },
    CommandSpec {
        id: "layout.alignOrigins",
        label: "Align Panel Origins",
        menu_path: "Arrange/Align Plot Origins",
        shortcut: "",
        summary: "Enable or disable plot-area origin alignment across layout panels and optionally normalize live graph bounds.",
        beginner: "Lines up the X and Y axis origins of neighboring panels so multi-panel figures align cleanly.",
        group: CommandGroup::Layout,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_layout_align_origins,
        enabled: has_layout_sheet,
        run: run_layout_align_origins,
    },
    CommandSpec {
        id: "layout.applyCanvasPreset",
        label: "Apply Layout Canvas Preset",
        menu_path: "Format/Layout Canvas Size Preset...",
        shortcut: "",
        summary: "Resize a layout page to a standard publication canvas preset while optionally preserving typography sizes.",
        beginner: "Resizes a whole layout figure to a journal column width or slide preset and runs preflight checks.",
        group: CommandGroup::Layout,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_apply_canvas_preset,
        enabled: has_layout_sheet,
        run: run_layout_apply_canvas_preset,
    },
    CommandSpec {
        id: "layout.equalizeScaling",
        label: "Equalize Scaling",
        menu_path: "Arrange/Equalize Scaling",
        shortcut: "",
        summary: "Equalize rendered inner plot scaling across live graph panels on a layout sheet.",
        beginner: "Resizes selected or all graph panels on a layout to match the smallest, largest, or an explicit scale percentage.",
        group: CommandGroup::Layout,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_layout_equalize_scaling,
        enabled: has_layout_sheet,
        run: run_layout_equalize_scaling,
    },
    CommandSpec {
        id: "layout.reorderPanel",
        label: "Reorder Panel",
        menu_path: "Arrange/Reorder Panel",
        shortcut: "",
        summary: "Change the z-order stacking depth of a panel on a layout sheet.",
        beginner: "Moves a graph, table, or text panel forward or backward in the layout paint order.",
        group: CommandGroup::Layout,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_layout_reorder_panel,
        enabled: has_layout_sheet,
        run: run_layout_reorder_panel,
    },
    CommandSpec {
        id: "layout.insertImage",
        label: "Insert Image on Layout",
        menu_path: "Insert/Image on Layout...",
        shortcut: "",
        summary: "Embed a PNG, JPEG, or TIFF raster image item onto a publication layout sheet.",
        beginner: "Places a microscopy image, gel photo, or schematic panel onto a layout page.",
        group: CommandGroup::Layout,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: schema_layout_insert_image,
        enabled: has_layout_sheet,
        run: run_layout_insert_image,
    },
    CommandSpec {
        id: "layout.updateImage",
        label: "Format Embedded Layout Image",
        menu_path: "Format/Layout Image...",
        shortcut: "",
        summary: "Update placement bounds, aspect-ratio lock, rotation, crop insets, border, or badge override on a layout image.",
        beginner: "Crops, rotates, resizes, frames, or labels an embedded image panel on a layout sheet.",
        group: CommandGroup::Layout,
        effects: MUTATE_IDEMPOTENT,
        input_schema: schema_layout_update_image,
        enabled: has_layout_sheet,
        run: run_layout_update_image,
    },
];

fn always_enabled(_session: &Session) -> bool {
    true
}

fn has_layout_sheet(session: &Session) -> bool {
    !session.document().layout_sheets.is_empty()
}

// ---------------------------------------------------------------------------
// Schemas & Typed Parameter Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct LayoutSpecFieldsParams {
    preset: Option<Value>,
    width: Option<f64>,
    height: Option<f64>,
    unit: Option<Value>,
    orientation: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    grid_template: Option<Option<Value>>,
    align_plot_origins: Option<bool>,
    panel_badges: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    background_fill: Option<Option<Value>>,
}

#[derive(Debug, Deserialize)]
struct LayoutCreateParams {
    #[serde(alias = "id")]
    sheet_id: Option<String>,
    title: Option<String>,
    #[serde(flatten)]
    fields: LayoutSpecFieldsParams,
}

fn layout_spec_field_properties() -> Vec<(&'static str, Value)> {
    vec![
        ("preset", nullable_schema(any_json_schema("Journal width preset (e.g., `single_column`, `double_column`, `a4_portrait`)"))),
        ("width", nullable_schema(num_schema("Page width in the configured physical unit", Some(0.01), None))),
        ("height", nullable_schema(num_schema("Page height in the configured physical unit", Some(0.01), None))),
        ("unit", nullable_schema(any_json_schema("Physical unit (`mm`, `cm`, `in`, `pt`)"))),
        ("orientation", nullable_schema(any_json_schema("Page orientation (`portrait` or `landscape`)"))),
        (
            "grid_template",
            nullable_schema(array_schema(
                "Optional `[rows, cols]` panel grid template; pass null to clear",
                int_schema("Grid dimension count", Some(0), Some(255)),
                Some(2),
                Some(2),
            )),
        ),
        ("align_plot_origins", nullable_schema(bool_schema("Align adjacent live graph plot-area origins"))),
        ("panel_badges", nullable_schema(any_json_schema("Automatic panel badge style (`none`, `uppercase`, `lowercase`)"))),
        ("background_fill", nullable_schema(any_json_schema("Optional page background RGBA fill or `#RRGGBB` hex; null for transparent"))),
    ]
}

fn schema_layout_create() -> Value {
    let mut props = vec![
        ("sheet_id", nullable_schema(non_empty_str_schema("Optional explicit layout sheet ID"))),
        ("title", nullable_schema(str_schema("Optional layout sheet title (defaults to `Layout 1`)"))),
    ];
    props.extend(layout_spec_field_properties());
    obj_schema_with_aliases(props, &[], vec![("sheet_id", &["id"])], false)
}

#[derive(Debug, Deserialize)]
struct LayoutSetSpecParams {
    #[serde(alias = "id")]
    sheet_id: String,
    spec: Option<Value>,
    title: Option<String>,
    #[serde(flatten)]
    fields: LayoutSpecFieldsParams,
}

fn schema_layout_set_spec() -> Value {
    let mut props = vec![
        ("sheet_id", non_empty_str_schema("Target layout sheet ID")),
        ("spec", nullable_schema(json!({ "type": "object", "description": "Partial or full `LayoutSpec` JSON overlay" }))),
        ("title", nullable_schema(str_schema("Optional updated layout sheet title"))),
    ];
    props.extend(layout_spec_field_properties());
    obj_schema_with_aliases(props, &["sheet_id"], vec![("sheet_id", &["id"])], false)
}

#[derive(Debug, Deserialize)]
struct LayoutAddPanelParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(alias = "type")]
    kind: Option<String>,
    panel_id: Option<String>,
    #[serde(alias = "graph_sheet")]
    graph_sheet_id: Option<String>,
    #[serde(alias = "source_sheet")]
    source_sheet_id: Option<String>,
    tab_id: Option<String>,
    #[serde(alias = "text")]
    markdown: Option<String>,
    bounds_pt: Option<Value>,
    margin_pt: Option<f64>,
    gutter_pt: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_usize")]
    row: Option<usize>,
    #[serde(default, deserialize_with = "de_opt_usize")]
    col: Option<usize>,
    scale_percent: Option<f64>,
    #[serde(alias = "badge")]
    badge_override: Option<String>,
    font_size_pt: Option<f64>,
}

fn schema_layout_add_panel() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target layout sheet ID")),
            ("kind", nullable_schema(non_empty_str_schema("Panel kind (`graph`, `table`, or `text`)"))),
            ("panel_id", nullable_schema(str_schema("Optional explicit panel identifier"))),
            ("graph_sheet_id", nullable_schema(non_empty_str_schema("Source graph sheet ID when adding a live graph panel"))),
            ("source_sheet_id", nullable_schema(non_empty_str_schema("Source data or result sheet ID when adding a live table panel"))),
            ("tab_id", nullable_schema(str_schema("Optional result tab ID or title for live table panels"))),
            ("markdown", nullable_schema(str_schema("Markdown body text for text callout panels"))),
            ("bounds_pt", nullable_schema(any_json_schema("Explicit `[x, y, width, height]` array or `RectPt` object in points"))),
            ("margin_pt", nullable_schema(num_schema("Grid auto-placement outer page margin in points (default 18.0)", Some(0.0), None))),
            ("gutter_pt", nullable_schema(num_schema("Grid auto-placement inter-panel gutter in points (default 12.0)", Some(0.0), None))),
            ("row", nullable_schema(int_schema("Zero-based grid row index when auto-placing", Some(0), None))),
            ("col", nullable_schema(int_schema("Zero-based grid column index when auto-placing", Some(0), None))),
            ("scale_percent", nullable_schema(num_schema("Live graph display scale percentage (default 100.0)", Some(0.01), None))),
            ("badge_override", nullable_schema(str_schema("Custom panel badge label overriding automatic lettering"))),
            ("font_size_pt", nullable_schema(num_schema("Text callout font size in points (default 10.0)", Some(0.5), None))),
        ],
        &["sheet_id"],
        vec![
            ("sheet_id", &["id"]),
            ("kind", &["type"]),
            ("graph_sheet_id", &["graph_sheet"]),
            ("source_sheet_id", &["source_sheet"]),
            ("markdown", &["text"]),
            ("badge_override", &["badge"]),
        ],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct LayoutAlignOriginsParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(alias = "align_plot_origins")]
    align: Option<bool>,
    normalize_bounds: Option<bool>,
}

fn schema_layout_align_origins() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target layout sheet ID")),
            ("align", nullable_schema(bool_schema("Enable or disable plot-origin alignment (default true)"))),
            ("normalize_bounds", nullable_schema(bool_schema("Snap nearby live graph panel X/Y bounds into shared row/column bands (default true)"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"]), ("align", &["align_plot_origins"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct LayoutApplyCanvasPresetParams {
    #[serde(alias = "id")]
    sheet_id: String,
    preset: String,
    #[serde(alias = "mode")]
    resize_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LayoutEqualizeScalingParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    percent: Option<f64>,
    #[serde(default)]
    panel_ids: Option<Vec<String>>,
    #[serde(default)]
    panel_indices: Option<Vec<usize>>,
}

fn schema_layout_equalize_scaling() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target layout sheet ID")),
            (
                "mode",
                nullable_schema(enum_str_schema(
                    "Scaling equalization rule (`match_smallest`, `match_largest`, or `explicit_percent`)",
                    &["match_smallest", "match_largest", "explicit_percent", "MatchSmallest", "MatchLargest", "ExplicitPercent"],
                )),
            ),
            (
                "percent",
                nullable_schema(num_schema("Explicit target scale percentage in [5.0, 500.0] for `explicit_percent`", Some(5.0), Some(500.0))),
            ),
            (
                "panel_ids",
                nullable_schema(array_schema("Optional subset of live graph panel IDs to equalize", non_empty_str_schema("Panel ID"), None, None)),
            ),
            (
                "panel_indices",
                nullable_schema(array_schema(
                    "Optional subset of zero-based panel indices to equalize",
                    int_schema("Panel index", Some(0), None),
                    None,
                    None,
                )),
            ),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct LayoutReorderPanelParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(default, deserialize_with = "de_opt_usize")]
    panel_index: Option<usize>,
    #[serde(default)]
    panel_id: Option<String>,
    #[serde(alias = "operation", alias = "direction", alias = "order")]
    action: String,
}

fn schema_layout_reorder_panel() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target layout sheet ID")),
            ("panel_index", nullable_schema(int_schema("Zero-based index in `layout.spec.items`", Some(0), None))),
            ("panel_id", nullable_schema(non_empty_str_schema("Panel ID in `layout.spec.items`"))),
            (
                "action",
                enum_str_schema(
                    "Z-order stacking action",
                    &[
                        "bring_to_front",
                        "bring_forward",
                        "send_backward",
                        "send_to_back",
                        "BringToFront",
                        "BringForward",
                        "SendBackward",
                        "SendToBack",
                    ],
                ),
            ),
        ],
        &["sheet_id", "action"],
        vec![("sheet_id", &["id"]), ("action", &["operation", "direction", "order"])],
        false,
    )
}

// ---------------------------------------------------------------------------
// Command Runners
// ---------------------------------------------------------------------------

fn run_layout_create(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_layout_create();
    let params: LayoutCreateParams = parse_params(&schema, raw)?;
    let doc = &mut ctx.session.doc;

    let id = match params.sheet_id.as_deref() {
        Some(raw_id) => parse_sheet_id(raw_id)?,
        None => next_unique_sheet_id(doc, "layout")?,
    };
    let title = params.title.unwrap_or_else(|| String::from("Layout 1"));
    let mut spec = if let Some(ref preset_val) = params.fields.preset {
        LayoutSpec::from_preset(parse_journal_preset(preset_val)?)
    } else {
        LayoutSpec::default()
    };
    apply_layout_spec_fields(&mut spec, &params.fields)?;

    let layout = LayoutSheet::new(id.clone(), title.clone(), spec)?;
    validate_layout_spec_in_doc(doc, &id, &layout.spec)?;
    let page_pt = layout.spec.page_size_pt();
    doc.add_layout_sheet(layout)?;

    Ok(CommandOutcome::mutated(
        "layout.create",
        format!("created layout `{id}`"),
        json!({ "sheet_id": id.as_str(), "title": title, "width_pt": page_pt.width, "height_pt": page_pt.height }),
    ))
}

pub(super) fn validate_layout_spec_in_doc(
    doc: &curvebook_doc::ProjectDocument,
    layout_id: &curvebook_doc::SheetId,
    spec: &LayoutSpec,
) -> Result<(), EngineError> {
    spec.validate_item_count().map_err(DocError::from)?;
    spec.validate()?;
    if spec.width < 0.01 || spec.height < 0.01 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "layout `width` and `height` must be finite and >= 0.01"));
    }
    for (idx, item) in spec.items.iter().enumerate() {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, scale_percent, .. } => {
                if *scale_percent < 0.01 {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("layout item[{idx}] `scale_percent` must be >= 0.01 (got {scale_percent})"),
                    ));
                }
                if graph_sheet_id == layout_id {
                    return Err(EngineError::with_code(
                        ErrorCode::CycleDetected,
                        format!("self-referential layout item[{idx}] graph `{graph_sheet_id}` on `{layout_id}`"),
                    ));
                }
                let _ = crate::cmd::helpers::require_graph_sheet(doc, graph_sheet_id)?;
            }
            LayoutItem::LiveTable { source_sheet_id, .. } => {
                if source_sheet_id == layout_id {
                    return Err(EngineError::with_code(
                        ErrorCode::CycleDetected,
                        format!("self-referential layout item[{idx}] source `{source_sheet_id}` on `{layout_id}`"),
                    ));
                }
                match doc.sheet_section(source_sheet_id) {
                    Some(curvebook_doc::SheetSection::Data | curvebook_doc::SheetSection::Info | curvebook_doc::SheetSection::Analyses) => {}
                    Some(_) => {
                        return Err(EngineError::with_code(
                            ErrorCode::BadInput,
                            format!("layout item[{idx}] source `{source_sheet_id}` must reference a data, info, or result sheet"),
                        ));
                    }
                    None => return Err(DocError::SheetNotFound { sheet_id: source_sheet_id.clone() }.into()),
                }
            }
            LayoutItem::TextCallout { font_size_pt, .. } => {
                if *font_size_pt < 0.5 {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("layout item[{idx}] `font_size_pt` must be >= 0.5 (got {font_size_pt})"),
                    ));
                }
            }
            LayoutItem::EmbeddedImage { asset_id, .. } => {
                if doc.image_asset(asset_id).is_none() {
                    return Err(EngineError::with_code(
                        ErrorCode::NotFound,
                        format!("layout item[{idx}] image asset `{asset_id}` not found in project"),
                    ));
                }
            }
        }
    }
    crate::dag::validate_inline_hook_refs(doc, layout_id, &spec.inline_hooks())?;
    crate::dag::validate_layout_spec_acyclic(doc, layout_id, spec)
}

fn run_layout_set_spec(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_layout_set_spec();
    let params: LayoutSetSpecParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = &mut ctx.session.doc;

    let current = require_layout_sheet(doc, &id)?;
    let mut spec = current.spec.clone();

    if let Some(ref spec_val) = params.spec {
        if let Some(items_arr) = spec_val.get("items").and_then(Value::as_array)
            && items_arr.len() > MAX_LAYOUT_ITEMS
        {
            return Err(DocError::TooManyLayoutItems { count: items_arr.len(), max: MAX_LAYOUT_ITEMS }.into());
        }
        let merged = merge_json_struct(&spec, spec_val)?;
        spec = serde_json::from_value(merged).map_err(|err| {
            EngineError::with_code(ErrorCode::BadInput, format!("invalid layout `spec`: {err}"))
                .with_remediation("Supply fields matching LayoutSpec or use shorthand fields without `spec`.")
                .with_details(json!({ "pointer": "/spec" }))
        })?;
    } else {
        apply_layout_spec_fields(&mut spec, &params.fields)?;
    }
    if let Some(ref title) = params.title {
        curvebook_doc::validate_title(title)?;
    }
    validate_layout_spec_in_doc(doc, &id, &spec)?;

    let page_pt = spec.page_size_pt();
    let items_len = spec.items.len();
    let layout_mut = require_layout_sheet_mut(doc, &id)?;
    layout_mut.spec = spec;
    if let Some(title) = params.title {
        layout_mut.title = title;
    }

    Ok(CommandOutcome::mutated(
        "layout.setSpec",
        format!("updated layout spec on `{id}`"),
        json!({ "sheet_id": id.as_str(), "items_count": items_len, "width_pt": page_pt.width, "height_pt": page_pt.height }),
    ))
}

fn run_layout_add_panel(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_layout_add_panel();
    let params: LayoutAddPanelParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = &mut ctx.session.doc;

    let next_idx = {
        let layout = require_layout_sheet(doc, &id)?;
        layout.spec.items.len()
    };
    if next_idx >= MAX_LAYOUT_ITEMS {
        return Err(DocError::TooManyLayoutItems { count: next_idx + 1, max: MAX_LAYOUT_ITEMS }.into());
    }
    let bounds_pt = {
        let layout = require_layout_sheet(doc, &id)?;
        resolve_panel_bounds(&layout.spec, next_idx, &params)?
    };
    let panel_id = params.panel_id.unwrap_or_else(|| format!("panel-{}", next_idx + 1));

    let inferred_kind = if params.graph_sheet_id.is_some() {
        "graph"
    } else if params.source_sheet_id.is_some() {
        "table"
    } else {
        "text"
    };
    let panel_kind = params.kind.as_deref().unwrap_or(inferred_kind);

    let item = match panel_kind.trim().to_ascii_lowercase().as_str() {
        "graph" | "live_graph" => {
            let raw_graph = params.graph_sheet_id.as_deref().ok_or_else(|| {
                EngineError::with_code(ErrorCode::BadInput, "missing `graph_sheet_id` for `graph` panel")
                    .with_remediation("Provide a valid `graph_sheet_id` referencing an existing graph sheet.")
                    .with_details(json!({ "pointer": "/graph_sheet_id" }))
            })?;
            let graph_id = parse_sheet_id(raw_graph)?;
            if doc.graph_sheet(&graph_id).is_none() {
                return Err(EngineError::with_code(ErrorCode::NotFound, format!("graph sheet `{graph_id}` not found"))
                    .with_remediation("Verify `graph_sheet_id` exists in the current project.")
                    .with_details(json!({ "pointer": "/graph_sheet_id", "sheet_id": graph_id.as_str() })));
            }
            LayoutItem::LiveGraph {
                id: panel_id,
                graph_sheet_id: graph_id,
                bounds_pt,
                scale_percent: params.scale_percent.unwrap_or(100.0),
                badge_override: params.badge_override,
            }
        }
        "table" | "live_table" => {
            let raw_source = params.source_sheet_id.as_deref().ok_or_else(|| {
                EngineError::with_code(ErrorCode::BadInput, "missing `source_sheet_id` for `table` panel")
                    .with_remediation("Provide a `source_sheet_id` referencing an existing sheet.")
                    .with_details(json!({ "pointer": "/source_sheet_id" }))
            })?;
            let source_id = parse_sheet_id(raw_source)?;
            if doc.sheet_section(&source_id).is_none() {
                return Err(EngineError::with_code(ErrorCode::NotFound, format!("table source sheet `{source_id}` not found"))
                    .with_remediation("Verify `source_sheet_id` references an existing sheet in the project.")
                    .with_details(json!({ "pointer": "/source_sheet_id", "sheet_id": source_id.as_str() })));
            }
            LayoutItem::LiveTable {
                id: panel_id,
                source_sheet_id: source_id,
                tab_id: params.tab_id,
                bounds_pt,
                selected_rows: None,
                selected_cols: None,
                style: curvebook_doc::EmbeddedTableStyle::default(),
            }
        }
        "text" | "callout" | "text_callout" => LayoutItem::TextCallout {
            id: panel_id,
            markdown: params.markdown.ok_or_else(|| {
                EngineError::with_code(ErrorCode::BadInput, "missing `markdown` or `text` for text callout panel")
                    .with_remediation("Supply `markdown` (or `text`) for a text panel.")
                    .with_details(json!({ "pointer": "/markdown" }))
            })?,
            bounds_pt,
            font_size_pt: params.font_size_pt.unwrap_or(10.0),
        },
        other => {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown layout panel kind `{other}`"))
                .with_remediation("Use `graph`, `table`, or `text`.")
                .with_details(json!({ "pointer": "/kind", "kind": other })));
        }
    };

    let mut candidate_spec = require_layout_sheet(doc, &id)?.spec.clone();
    candidate_spec.add_item(item).map_err(DocError::from)?;
    validate_layout_spec_in_doc(doc, &id, &candidate_spec)?;
    let layout_mut = require_layout_sheet_mut(doc, &id)?;
    layout_mut.spec = candidate_spec;
    let panel_count = layout_mut.spec.items.len();

    Ok(CommandOutcome::mutated(
        "layout.addPanel",
        format!("added panel to layout `{id}`"),
        json!({ "sheet_id": id.as_str(), "panel_index": panel_count - 1, "panel_count": panel_count, "bounds_pt": bounds_pt }),
    ))
}

fn run_layout_align_origins(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_layout_align_origins();
    let params: LayoutAlignOriginsParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let align = params.align.unwrap_or(true);
    let normalize_bounds = params.normalize_bounds.unwrap_or(true);

    let layout = require_layout_sheet_mut(&mut ctx.session.doc, &id)?;
    layout.spec.align_plot_origins = align;
    if align && normalize_bounds {
        align_live_graph_bounds(&mut layout.spec.items);
    }

    let aligned_rects: Vec<Value> = layout
        .spec
        .items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| match item {
            LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, .. } => Some(json!({
                "panel_index": idx,
                "graph_sheet_id": graph_sheet_id.as_str(),
                "x": bounds_pt.x,
                "y": bounds_pt.y,
                "width": bounds_pt.width,
                "height": bounds_pt.height,
            })),
            LayoutItem::LiveTable { .. } | LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => None,
        })
        .collect();

    Ok(CommandOutcome::mutated(
        "layout.alignOrigins",
        format!("aligned plot origins on layout `{id}`"),
        json!({
            "sheet_id": id.as_str(),
            "align_plot_origins": align,
            "aligned_panels": aligned_rects.len(),
            "panels": aligned_rects,
        }),
    ))
}

fn run_layout_apply_canvas_preset(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_apply_canvas_preset();
    let params: LayoutApplyCanvasPresetParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let preset = parse_figure_canvas_preset(&params.preset)?;
    let mode = parse_canvas_resize_mode(params.resize_mode.as_deref())?;

    let doc = &mut ctx.session.doc;
    let mut updated_spec = require_layout_sheet(doc, &id)?.spec.clone();
    resize_layout_spec(&mut updated_spec, preset, mode)?;
    updated_spec.validate()?;

    require_layout_sheet_mut(doc, &id)?.spec = updated_spec;

    let scene = compile_scene_for_sheet(doc, &id)?;
    let issues = preflight_check_scene(&scene);
    Ok(CommandOutcome::mutated(
        "layout.applyCanvasPreset",
        format!("applied canvas preset `{preset:?}` to layout `{id}`"),
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

#[derive(Debug, Clone, Copy)]
enum EqualizeScaleMode {
    MatchSmallest,
    MatchLargest,
    ExplicitPercent(f64),
}

fn parse_equalize_scale_mode(mode: Option<&str>, percent: Option<f64>) -> Result<EqualizeScaleMode, EngineError> {
    let norm = mode.map(|m| m.trim().chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect::<String>());
    match norm.as_deref() {
        None => {
            if let Some(p) = percent {
                validate_scale_percent_range(p)?;
                Ok(EqualizeScaleMode::ExplicitPercent(p))
            } else {
                Ok(EqualizeScaleMode::MatchSmallest)
            }
        }
        Some("matchsmallest" | "smallest") => Ok(EqualizeScaleMode::MatchSmallest),
        Some("matchlargest" | "largest") => Ok(EqualizeScaleMode::MatchLargest),
        Some("explicitpercent" | "explicit") => {
            let p = percent.ok_or_else(|| {
                EngineError::with_code(ErrorCode::BadInput, "`explicit_percent` mode requires `percent` in [5.0, 500.0]")
                    .with_details(json!({ "pointer": "/percent" }))
            })?;
            validate_scale_percent_range(p)?;
            Ok(EqualizeScaleMode::ExplicitPercent(p))
        }
        Some(other) => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `equalizeScaling` mode `{other}`"))
            .with_details(json!({ "pointer": "/mode" }))),
    }
}

fn validate_scale_percent_range(p: f64) -> Result<(), EngineError> {
    if !p.is_finite() || !(5.0..=500.0).contains(&p) {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`percent` must be finite and in [5.0, 500.0]")
            .with_details(json!({ "pointer": "/percent" })));
    }
    Ok(())
}

fn layout_item_id(item: &LayoutItem) -> &str {
    item.id()
}

fn run_layout_equalize_scaling(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_layout_equalize_scaling();
    let params: LayoutEqualizeScalingParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let mode = parse_equalize_scale_mode(params.mode.as_deref(), params.percent)?;
    let doc = &ctx.session.doc;
    let mut candidate_spec = require_layout_sheet(doc, &id)?.spec.clone();

    let mut target_indices: Vec<usize> = Vec::new();
    let filter_by_id_or_idx = params.panel_ids.is_some() || params.panel_indices.is_some();
    if let Some(ref indices) = params.panel_indices {
        for &idx in indices {
            let Some(item) = candidate_spec.items.get(idx) else {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("`panel_index` {idx} out of bounds")));
            };
            if !matches!(item, LayoutItem::LiveGraph { .. }) {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("panel `{idx}` is not a live graph panel")));
            }
            if !target_indices.contains(&idx) {
                target_indices.push(idx);
            }
        }
    }
    if let Some(ref ids) = params.panel_ids {
        for pid in ids {
            let Some(idx) = candidate_spec.items.iter().position(|it| layout_item_id(it) == pid) else {
                return Err(EngineError::with_code(ErrorCode::NotFound, format!("panel `{pid}` not found on layout `{id}`")));
            };
            if !matches!(candidate_spec.items[idx], LayoutItem::LiveGraph { .. }) {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("panel `{pid}` is not a live graph panel")));
            }
            if !target_indices.contains(&idx) {
                target_indices.push(idx);
            }
        }
    }
    if !filter_by_id_or_idx {
        for (idx, item) in candidate_spec.items.iter().enumerate() {
            if matches!(item, LayoutItem::LiveGraph { .. }) {
                target_indices.push(idx);
            }
        }
    }
    if target_indices.is_empty() {
        return Err(EngineError::with_code(ErrorCode::Conflict, format!("layout `{id}` has no target live graph panels to equalize")));
    }

    struct PanelMetric {
        idx: usize,
        sub_w: f64,
        sub_h: f64,
        plot_w: f64,
        plot_h: f64,
        cur_plot_w: f64,
    }
    let mut metrics: Vec<PanelMetric> = Vec::with_capacity(target_indices.len());
    for &idx in &target_indices {
        let LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, scale_percent, .. } = &candidate_spec.items[idx] else {
            continue;
        };
        let gs = require_graph_sheet(doc, graph_sheet_id)?;
        let sub = curvebook_chart::compile_graph(gs, doc)
            .map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed compiling panel graph `{graph_sheet_id}`: {err}")))?;
        let sub_w = sub.width_pt.max(1.0);
        let sub_h = sub.height_pt.max(1.0);
        let plot_w = sub.plot_rect_pt.width.max(1.0);
        let plot_h = sub.plot_rect_pt.height.max(1.0);
        let fit = (bounds_pt.width / sub_w).min(bounds_pt.height / sub_h);
        let pct_frac = if scale_percent.is_finite() && *scale_percent > 0.0 { *scale_percent / 100.0 } else { 1.0 };
        let cur_scale = (fit * pct_frac).max(0.05);
        metrics.push(PanelMetric { idx, sub_w, sub_h, plot_w, plot_h, cur_plot_w: plot_w * cur_scale });
    }

    let target_plot_w = match mode {
        EqualizeScaleMode::MatchSmallest => metrics.iter().map(|m| m.cur_plot_w).fold(f64::INFINITY, f64::min),
        EqualizeScaleMode::MatchLargest => metrics.iter().map(|m| m.cur_plot_w).fold(0.0_f64, f64::max),
        EqualizeScaleMode::ExplicitPercent(p) => metrics[0].plot_w * (p / 100.0).max(0.05),
    };

    let mut summary_panels = Vec::with_capacity(metrics.len());
    for m in metrics {
        let desired_scale = match mode {
            EqualizeScaleMode::MatchSmallest | EqualizeScaleMode::MatchLargest => (target_plot_w / m.plot_w).max(0.05),
            EqualizeScaleMode::ExplicitPercent(p) => (p / 100.0).max(0.05),
        };
        if let Some(LayoutItem::LiveGraph { id: panel_id, graph_sheet_id, bounds_pt, scale_percent, .. }) = candidate_spec.items.get_mut(m.idx) {
            let needed_w = m.sub_w * desired_scale;
            let needed_h = m.sub_h * desired_scale;
            if bounds_pt.width + 1e-6 < needed_w {
                bounds_pt.width = needed_w;
            }
            if bounds_pt.height + 1e-6 < needed_h {
                bounds_pt.height = needed_h;
            }
            let new_fit = (bounds_pt.width / m.sub_w).min(bounds_pt.height / m.sub_h);
            let next_pct = ((desired_scale / new_fit.max(1e-6)) * 100.0).max(0.01);
            *scale_percent = next_pct;
            summary_panels.push(json!({
                "panel_index": m.idx,
                "panel_id": panel_id,
                "graph_sheet_id": graph_sheet_id.as_str(),
                "scale_percent": next_pct,
                "effective_scale": desired_scale,
                "rendered_plot_width_pt": m.plot_w * desired_scale,
                "rendered_plot_height_pt": m.plot_h * desired_scale,
            }));
        }
    }

    validate_layout_spec_in_doc(doc, &id, &candidate_spec)?;
    require_layout_sheet_mut(&mut ctx.session.doc, &id)?.spec = candidate_spec;

    Ok(CommandOutcome::mutated(
        "layout.equalizeScaling",
        format!("equalized scaling across {} panel(s) on layout `{id}`", summary_panels.len()),
        json!({
            "sheet_id": id.as_str(),
            "target_plot_width_pt": target_plot_w,
            "panels": summary_panels,
        }),
    ))
}

fn run_layout_reorder_panel(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_layout_reorder_panel();
    let params: LayoutReorderPanelParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = &ctx.session.doc;
    let mut candidate_spec = require_layout_sheet(doc, &id)?.spec.clone();
    let len = candidate_spec.items.len();
    if len == 0 {
        return Err(EngineError::with_code(ErrorCode::Conflict, format!("layout `{id}` has no panels to reorder")));
    }

    let idx_from_opt = if let Some(idx) = params.panel_index {
        if idx >= len {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`panel_index` {idx} out of bounds ({len} panels)"))
                .with_details(json!({ "pointer": "/panel_index" })));
        }
        Some(idx)
    } else {
        None
    };
    let idx_from_id = if let Some(ref pid) = params.panel_id {
        let pos = candidate_spec.items.iter().position(|it| layout_item_id(it) == pid).ok_or_else(|| {
            EngineError::with_code(ErrorCode::NotFound, format!("panel `{pid}` not found on layout `{id}`"))
                .with_details(json!({ "pointer": "/panel_id" }))
        })?;
        Some(pos)
    } else {
        None
    };

    let old_idx = match (idx_from_opt, idx_from_id) {
        (Some(a), Some(b)) if a != b => {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`panel_index` and `panel_id` refer to different panels"));
        }
        (Some(a), _) | (None, Some(a)) => a,
        (None, None) => {
            return Err(EngineError::with_code(ErrorCode::BadInput, "provide either `panel_index` or `panel_id`"));
        }
    };

    let norm_action: String = params.action.trim().chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let (canonical_action, new_idx) = match norm_action.as_str() {
        "bringtofront" | "front" | "top" => ("bring_to_front", len - 1),
        "bringforward" | "forward" | "up" => ("bring_forward", (old_idx + 1).min(len - 1)),
        "sendbackward" | "backward" | "down" => ("send_backward", old_idx.saturating_sub(1)),
        "sendtoback" | "back" | "bottom" => ("send_to_back", 0),
        _ => {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("unknown reorder action `{}`; expected `bring_to_front`, `bring_forward`, `send_backward`, or `send_to_back`", params.action),
            )
            .with_details(json!({ "pointer": "/action" })));
        }
    };

    let moved_id = layout_item_id(&candidate_spec.items[old_idx]).to_owned();
    let mutated = old_idx != new_idx;
    if mutated {
        let item = candidate_spec.items.remove(old_idx);
        candidate_spec.items.insert(new_idx, item);
        validate_layout_spec_in_doc(doc, &id, &candidate_spec)?;
        require_layout_sheet_mut(&mut ctx.session.doc, &id)?.spec = candidate_spec;
    }

    Ok(CommandOutcome {
        command_id: String::from("layout.reorderPanel"),
        mutated,
        message: format!("reordered panel `{moved_id}` on layout `{id}` ({canonical_action}: {old_idx} -> {new_idx})"),
        payload: json!({
            "sheet_id": id.as_str(),
            "panel_id": moved_id,
            "old_index": old_idx,
            "new_index": new_idx,
            "action": canonical_action,
            "panel_count": len,
        }),
        warnings: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Geometry, Resize, and Field Parsers
// ---------------------------------------------------------------------------

fn align_live_graph_bounds(items: &mut [LayoutItem]) {
    let mut rects: Vec<(usize, RectPt)> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, it)| match it {
            LayoutItem::LiveGraph { bounds_pt, .. } => Some((idx, *bounds_pt)),
            LayoutItem::LiveTable { .. } | LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => None,
        })
        .collect();
    if rects.len() < 2 {
        return;
    }
    align_axis_bands(&mut rects, true);
    align_axis_bands(&mut rects, false);
    for (item_idx, new_rect) in rects {
        if let Some(LayoutItem::LiveGraph { bounds_pt, .. }) = items.get_mut(item_idx) {
            *bounds_pt = new_rect;
        }
    }
}

fn align_axis_bands(rects: &mut [(usize, RectPt)], vertical_col: bool) {
    let mut done = vec![false; rects.len()];
    for i in 0..rects.len() {
        if done[i] {
            continue;
        }
        let ref_r = rects[i].1;
        let span = if vertical_col { ref_r.width } else { ref_r.height }.max(40.0) * 0.35;
        let ref_coord = if vertical_col { ref_r.x } else { ref_r.y };
        let mut target = ref_coord;
        let mut group = vec![i];
        for (j, (_, rj)) in rects.iter().enumerate().skip(i + 1) {
            let coord_j = if vertical_col { rj.x } else { rj.y };
            if !done[j] && (coord_j - ref_coord).abs() <= span {
                target = target.max(coord_j);
                group.push(j);
            }
        }
        for idx in group {
            done[idx] = true;
            if vertical_col {
                rects[idx].1.x = target;
            } else {
                rects[idx].1.y = target;
            }
        }
    }
}

fn resize_layout_spec(spec: &mut LayoutSpec, preset: FigureCanvasPreset, mode: CanvasResizeMode) -> Result<(), EngineError> {
    let old_pt = spec.page_size_pt();
    let (new_w, new_h, unit) = preset.dimensions();
    let new_pt = preset.dimensions_pt();
    let sx = safe_scale_ratio(old_pt.width, new_pt.width)?;
    let sy = safe_scale_ratio(old_pt.height, new_pt.height)?;
    let scale_1d = sx.min(sy);

    spec.width = new_w;
    spec.height = new_h;
    spec.unit = unit;
    spec.orientation = if new_w >= new_h { PageOrientation::Landscape } else { PageOrientation::Portrait };

    for item in &mut spec.items {
        match item {
            LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } => {
                *bounds_pt = scale_rect_pt(*bounds_pt, sx, sy)?;
            }
            LayoutItem::TextCallout { bounds_pt, font_size_pt, .. } => {
                *bounds_pt = scale_rect_pt(*bounds_pt, sx, sy)?;
                if mode == CanvasResizeMode::ProportionalScale {
                    *font_size_pt = scale_finite(*font_size_pt, scale_1d, "layout.text_callout.font_size_pt")?;
                }
            }
            LayoutItem::EmbeddedImage { bounds_pt, crop, border, .. } => {
                *bounds_pt = scale_rect_pt(*bounds_pt, sx, sy)?;
                crop.left_pt = scale_finite(crop.left_pt, sx, "layout.image.crop.left_pt")?;
                crop.right_pt = scale_finite(crop.right_pt, sx, "layout.image.crop.right_pt")?;
                crop.top_pt = scale_finite(crop.top_pt, sy, "layout.image.crop.top_pt")?;
                crop.bottom_pt = scale_finite(crop.bottom_pt, sy, "layout.image.crop.bottom_pt")?;
                if mode == CanvasResizeMode::ProportionalScale
                    && let Some(b) = border
                {
                    b.width_pt = scale_finite(b.width_pt, scale_1d, "layout.image.border.width_pt")?;
                }
            }
        }
    }
    Ok(())
}

fn apply_layout_spec_fields(spec: &mut LayoutSpec, fields: &LayoutSpecFieldsParams) -> Result<(), EngineError> {
    if let Some(ref preset_val) = fields.preset {
        let fresh = LayoutSpec::from_preset(parse_journal_preset(preset_val)?);
        spec.width = fresh.width;
        spec.height = fresh.height;
        spec.unit = fresh.unit;
    }
    if let Some(ref unit_val) = fields.unit {
        spec.unit = parse_physical_unit(unit_val)?;
    }
    if let Some(w) = fields.width {
        spec.width = w;
    }
    if let Some(h) = fields.height {
        spec.height = h;
    }
    if let Some(ref ori) = fields.orientation {
        spec.orientation = parse_page_orientation(ori)?;
    }
    match &fields.grid_template {
        None => {}
        Some(None) => spec.grid_template = None,
        Some(Some(grid_val)) => spec.grid_template = Some(parse_grid_template(grid_val)?),
    }
    if let Some(align) = fields.align_plot_origins {
        spec.align_plot_origins = align;
    }
    if let Some(ref badges) = fields.panel_badges {
        spec.panel_badges = parse_panel_badge_style(badges)?;
    }
    match &fields.background_fill {
        None => {}
        Some(None) => spec.background_fill = None,
        Some(Some(bg)) => spec.background_fill = Some(parse_rgba8(bg)?),
    }
    Ok(())
}

fn resolve_panel_bounds(spec: &LayoutSpec, panel_index: usize, params: &LayoutAddPanelParams) -> Result<RectPt, EngineError> {
    if let Some(ref b) = params.bounds_pt {
        return parse_rect_pt(Some(b), RectPt::new(18.0, 18.0, 200.0, 160.0));
    }
    let page = spec.page_size_pt();
    let margin = params.margin_pt.unwrap_or(18.0);
    let gutter = params.gutter_pt.unwrap_or(12.0);
    let (rows_u8, cols_u8) = spec.grid_template.unwrap_or((2, 2));
    let rows = usize::from(rows_u8.max(1));
    let cols = usize::from(cols_u8.max(1));
    let col = params.col.unwrap_or(panel_index % cols);
    let row = params.row.unwrap_or(panel_index / cols);

    let usable_w = (page.width - 2.0 * margin - gutter * ((cols - 1) as f64)).max(40.0);
    let usable_h = (page.height - 2.0 * margin - gutter * ((rows - 1) as f64)).max(40.0);
    let cell_w = usable_w / (cols as f64);
    let cell_h = usable_h / (rows as f64);
    Ok(RectPt::new(margin + (col as f64) * (cell_w + gutter), margin + (row as f64) * (cell_h + gutter), cell_w, cell_h))
}

fn parse_grid_template(val: &Value) -> Result<(u8, u8), EngineError> {
    let arr = val.as_array().filter(|a| a.len() == 2).ok_or_else(|| {
        EngineError::with_code(ErrorCode::BadInput, "`grid_template` must be `[rows, cols]`").with_details(json!({ "pointer": "/grid_template" }))
    })?;
    let rows = parse_u8_count(&arr[0], "`grid_template[0]`", "/grid_template/0")?;
    let cols = parse_u8_count(&arr[1], "`grid_template[1]`", "/grid_template/1")?;
    Ok((rows.max(1), cols.max(1)))
}

fn parse_u8_count(val: &Value, label: &str, pointer: &str) -> Result<u8, EngineError> {
    if let Some(u) = val.as_u64().and_then(|v| u8::try_from(v).ok()) {
        return Ok(u);
    }
    if let Some(f) = val.as_f64()
        && f.is_finite()
        && f >= 0.0
        && f <= f64::from(u8::MAX)
        && f.fract() == 0.0
    {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Ok(f as u8);
    }
    Err(EngineError::with_code(ErrorCode::BadInput, format!("{label} must be u8")).with_details(json!({ "pointer": pointer })))
}

fn parse_journal_preset(val: &Value) -> Result<JournalWidthPreset, EngineError> {
    if let Ok(p) = serde_json::from_value::<JournalWidthPreset>(val.clone()) {
        return Ok(p);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("single_column" | "single" | "85mm") => Ok(JournalWidthPreset::SingleColumn85Mm),
        Some("89mm") => Ok(JournalWidthPreset::SingleColumn89Mm),
        Some("column_1_5" | "one_and_half" | "120mm") => Ok(JournalWidthPreset::OneAndHalfColumn120Mm),
        Some("136mm") => Ok(JournalWidthPreset::OneAndHalfColumn136Mm),
        Some("double_column" | "double" | "174mm") => Ok(JournalWidthPreset::DoubleColumn174Mm),
        Some("183mm") => Ok(JournalWidthPreset::DoubleColumn183Mm),
        Some("us_letter" | "letter" | "letter_portrait") => Ok(JournalWidthPreset::LetterPortrait),
        Some("letter_landscape") => Ok(JournalWidthPreset::LetterLandscape),
        Some("full_page" | "a4" | "a4_portrait") => Ok(JournalWidthPreset::A4Portrait),
        Some("a4_landscape") => Ok(JournalWidthPreset::A4Landscape),
        _ => {
            Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid layout `preset`: {val}")).with_details(json!({ "pointer": "/preset" })))
        }
    }
}

fn parse_physical_unit(val: &Value) -> Result<PhysicalUnit, EngineError> {
    if let Ok(u) = serde_json::from_value::<PhysicalUnit>(val.clone()) {
        return Ok(u);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("mm" | "millimeters") => Ok(PhysicalUnit::Millimeters),
        Some("cm" | "centimeters") => Ok(PhysicalUnit::Centimeters),
        Some("in" | "inch" | "inches") => Ok(PhysicalUnit::Inches),
        Some("pt" | "points") => Ok(PhysicalUnit::Points),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid physical `unit`: {val}")).with_details(json!({ "pointer": "/unit" }))),
    }
}

fn parse_page_orientation(val: &Value) -> Result<PageOrientation, EngineError> {
    if let Ok(o) = serde_json::from_value::<PageOrientation>(val.clone()) {
        return Ok(o);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("portrait") => Ok(PageOrientation::Portrait),
        Some("landscape") => Ok(PageOrientation::Landscape),
        _ => {
            Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `orientation`: {val}"))
                .with_details(json!({ "pointer": "/orientation" })))
        }
    }
}

fn parse_panel_badge_style(val: &Value) -> Result<PanelBadgeStyle, EngineError> {
    if let Ok(b) = serde_json::from_value::<PanelBadgeStyle>(val.clone()) {
        return Ok(b);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("none") => Ok(PanelBadgeStyle::None),
        Some("uppercase" | "abc") => Ok(PanelBadgeStyle::UpperCaseLetters),
        Some("lowercase" | "abc_lower") => Ok(PanelBadgeStyle::LowerCaseLetters),
        _ => {
            Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `panel_badges`: {val}"))
                .with_details(json!({ "pointer": "/panel_badges" })))
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutInsertImageParams {
    sheet_id: String,
    #[serde(default)]
    panel_id: Option<String>,
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
    #[serde(default)]
    badge_override: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutUpdateImageParams {
    sheet_id: String,
    #[serde(default)]
    panel_id: Option<String>,
    #[serde(default)]
    item_index: Option<usize>,
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
    #[serde(default, deserialize_with = "de_present_opt")]
    badge_override: Option<Option<String>>,
}

pub(super) struct LayoutImageInsertArgs<'a> {
    pub command_id: &'static str,
    pub panel_id: Option<&'a str>,
    pub path: Option<&'a str>,
    pub bytes_base64: Option<&'a str>,
    pub asset_id: Option<&'a str>,
    pub bounds_pt: Option<&'a Value>,
    pub rotation_deg: Option<f64>,
    pub lock_aspect_ratio: Option<bool>,
    pub crop: Option<&'a Value>,
    pub border: Option<&'a Option<Value>>,
    pub badge_override: Option<&'a str>,
}

fn schema_layout_insert_image() -> Value {
    let mut props = vec![
        ("sheet_id", non_empty_str_schema("Target LayoutSheet ID")),
        ("panel_id", nullable_schema(str_schema("Optional explicit layout panel identifier"))),
    ];
    props.extend(super::image::insert_image_common_props());
    props.push(("badge_override", nullable_schema(str_schema("Optional panel badge text rendered in top-left corner"))));
    obj_schema(props, &["sheet_id"], false)
}

fn schema_layout_update_image() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target LayoutSheet ID")),
            ("panel_id", nullable_schema(str_schema("Target EmbeddedImage `panel_id` (provide `panel_id` or `item_index`)"))),
            ("item_index", nullable_schema(int_schema("Zero-based index of the EmbeddedImage item", Some(0), None))),
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
            ("badge_override", nullable_schema(str_schema("Updated corner badge label; pass null to clear"))),
        ],
        &["sheet_id"],
        false,
    )
}

fn unique_layout_panel_id(items: &[LayoutItem], requested: Option<&str>) -> String {
    if let Some(id) = requested.map(str::trim).filter(|s| !s.is_empty()) {
        return id.to_owned();
    }
    let mut seq = items.len().saturating_add(1);
    loop {
        let candidate = format!("panel-{seq}");
        if items.iter().all(|it| it.id() != candidate) {
            return candidate;
        }
        seq += 1;
    }
}

pub(super) fn insert_image_on_layout(
    doc: &mut curvebook_doc::ProjectDocument,
    layout_id: &curvebook_doc::SheetId,
    args: LayoutImageInsertArgs<'_>,
) -> Result<CommandOutcome, EngineError> {
    let page_pt = {
        let layout = require_layout_sheet(doc, layout_id)?;
        if layout.spec.items.len() >= MAX_LAYOUT_ITEMS {
            return Err(DocError::TooManyLayoutItems { count: layout.spec.items.len() + 1, max: MAX_LAYOUT_ITEMS }.into());
        }
        layout.spec.page_size_pt()
    };
    let resolved = super::image::resolve_image_source(doc, args.path, args.bytes_base64, args.asset_id)?;
    let bounds_pt = super::image::parse_image_insert_bounds(args.bounds_pt, resolved.pixel_size, page_pt.width, page_pt.height)?;
    let rotation_deg = super::image::parse_rotation_deg(args.rotation_deg, 0.0)?;
    let lock_aspect_ratio = args.lock_aspect_ratio.unwrap_or(true);
    let crop = super::image::apply_crop_patch(args.crop, bounds_pt, curvebook_doc::CropInsetsPt::default())?;
    let border = super::image::apply_border_patch(args.border, None)?;

    let mut candidate_spec = require_layout_sheet(doc, layout_id)?.spec.clone();
    let panel_id = unique_layout_panel_id(&candidate_spec.items, args.panel_id);
    candidate_spec
        .add_item(LayoutItem::EmbeddedImage {
            id: panel_id.clone(),
            asset_id: resolved.asset_id.clone(),
            bounds_pt,
            rotation_deg,
            lock_aspect_ratio,
            crop,
            border,
            badge_override: args.badge_override.map(str::to_owned),
        })
        .map_err(DocError::from)?;
    validate_layout_spec_in_doc(doc, layout_id, &candidate_spec)?;
    let item_index = candidate_spec.items.len() - 1;
    require_layout_sheet_mut(doc, layout_id)?.spec = candidate_spec;

    Ok(CommandOutcome::mutated(
        args.command_id,
        format!("embedded {} image `{}` onto layout `{layout_id}`", resolved.media_type.mime_type(), resolved.asset_id),
        json!({
            "sheet_id": layout_id.as_str(),
            "panel_id": panel_id,
            "item_index": item_index,
            "asset_id": resolved.asset_id,
            "media_type": resolved.media_type.mime_type(),
            "pixel_size": [resolved.pixel_size.0, resolved.pixel_size.1],
            "bounds_pt": bounds_pt,
        }),
    ))
}

fn run_layout_insert_image(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_layout_insert_image();
    let params: LayoutInsertImageParams = parse_params(&schema, raw)?;
    let layout_id = parse_sheet_id(&params.sheet_id)?;
    insert_image_on_layout(
        &mut ctx.session.doc,
        &layout_id,
        LayoutImageInsertArgs {
            command_id: "layout.insertImage",
            panel_id: params.panel_id.as_deref(),
            path: params.path.as_deref(),
            bytes_base64: params.bytes_base64.as_deref(),
            asset_id: params.asset_id.as_deref(),
            bounds_pt: params.bounds_pt.as_ref(),
            rotation_deg: params.rotation_deg,
            lock_aspect_ratio: params.lock_aspect_ratio,
            crop: params.crop.as_ref(),
            border: params.border.as_ref(),
            badge_override: params.badge_override.as_deref(),
        },
    )
}

fn run_layout_update_image(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_layout_update_image();
    let params: LayoutUpdateImageParams = parse_params(&schema, raw)?;
    let layout_id = parse_sheet_id(&params.sheet_id)?;
    let doc = &ctx.session.doc;
    let mut candidate_spec = require_layout_sheet(doc, &layout_id)?.spec.clone();
    let len = candidate_spec.items.len();

    let idx_from_num = match params.item_index {
        Some(idx) if idx >= len => {
            return Err(EngineError::with_code(ErrorCode::NotFound, format!("item_index {idx} out of bounds ({len} items) on `{layout_id}`"))
                .with_details(json!({ "pointer": "/item_index" })));
        }
        opt => opt,
    };
    let idx_from_id = match params.panel_id.as_deref() {
        Some(pid) => {
            let pos = candidate_spec.items.iter().position(|it| it.id() == pid).ok_or_else(|| {
                EngineError::with_code(ErrorCode::NotFound, format!("panel `{pid}` not found on layout `{layout_id}`"))
                    .with_details(json!({ "pointer": "/panel_id" }))
            })?;
            Some(pos)
        }
        None => None,
    };
    let target_idx = match (idx_from_num, idx_from_id) {
        (Some(a), Some(b)) if a != b => {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`item_index` and `panel_id` refer to different layout items"));
        }
        (Some(a), _) | (None, Some(a)) => a,
        (None, None) => {
            return Err(EngineError::with_code(ErrorCode::BadInput, "provide either `panel_id` or `item_index`"));
        }
    };

    let item = &mut candidate_spec.items[target_idx];
    let LayoutItem::EmbeddedImage { id: item_id, asset_id, bounds_pt, rotation_deg, lock_aspect_ratio, crop, border, badge_override } = item else {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("layout item `{target_idx}` on `{layout_id}` is not an embedded image")));
    };
    let panel_id_str = item_id.clone();
    let asset_id_str = asset_id.clone();
    let intrinsic_px = doc.image_asset(&asset_id_str).map(|a| a.pixel_size);
    if let Some(lock) = params.lock_aspect_ratio {
        *lock_aspect_ratio = lock;
    }
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
    if let Some(next_badge) = params.badge_override {
        *badge_override = next_badge;
    }
    let summary = json!({
        "sheet_id": layout_id.as_str(),
        "panel_id": panel_id_str,
        "item_index": target_idx,
        "asset_id": asset_id_str,
        "bounds_pt": *bounds_pt,
        "rotation_deg": *rotation_deg,
        "lock_aspect_ratio": *lock_aspect_ratio,
        "crop": *crop,
        "border": *border,
        "badge_override": *badge_override,
    });
    validate_layout_spec_in_doc(doc, &layout_id, &candidate_spec)?;
    require_layout_sheet_mut(&mut ctx.session.doc, &layout_id)?.spec = candidate_spec;
    Ok(CommandOutcome::mutated("layout.updateImage", format!("updated embedded image on layout `{layout_id}`"), summary))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, DataSheet, ErrorCode, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};
    use serde_json::json;

    use super::*;

    fn session_with_two_graphs() -> Session {
        let mut doc = ProjectDocument::new("Layout Test").unwrap();
        let mut table = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
        table.x_cells[0][0] = Cell::number(1.0);
        table.x_cells[1][0] = Cell::number(2.0);
        table.data_sets[0].cells[0][0] = Cell::number(10.0);
        table.data_sets[0].cells[1][0] = Cell::number(20.0);
        doc.add_data_sheet(DataSheet::new("xy1", "Data", TableArchetype::Xy(table)).unwrap()).unwrap();
        let mut s = Session::from_document(doc);
        s.execute("graph.create", json!({ "data_sheet_id": "xy1", "sheet_id": "g1", "kind": "xy" })).unwrap();
        s.execute("graph.create", json!({ "data_sheet_id": "xy1", "sheet_id": "g2", "kind": "xy" })).unwrap();
        s
    }

    #[test]
    fn registry_contains_all_nine_layout_commands() {
        assert_eq!(specs().len(), 9);
        for spec in specs() {
            assert_eq!(spec.group, CommandGroup::Layout);
            assert!(!spec.summary.is_empty());
            assert!(!spec.beginner.is_empty());
        }
    }

    #[test]
    fn creates_composes_aligns_and_resizes_layout() {
        let mut s = session_with_two_graphs();
        s.execute(
            "layout.create",
            json!({ "sheet_id": "lay1", "title": "Figure 1", "preset": "double_column", "grid_template": [1, 2], "panel_badges": "uppercase" }),
        )
        .unwrap();
        s.execute("layout.addPanel", json!({ "sheet_id": "lay1", "graph_sheet_id": "g1", "row": 0, "col": 0 })).unwrap();
        s.execute("layout.addPanel", json!({ "sheet_id": "lay1", "graph_sheet_id": "g2", "bounds_pt": [260.0, 22.0, 200.0, 140.0] })).unwrap();
        s.execute("layout.addPanel", json!({ "sheet_id": "lay1", "kind": "text", "text": "Summary note" })).unwrap();
        let aligned = s.execute("layout.alignOrigins", json!({ "sheet_id": "lay1", "align": true, "normalize_bounds": true })).unwrap();
        assert_eq!(aligned.payload["aligned_panels"], json!(2));

        s.execute("layout.setSpec", json!({ "sheet_id": "lay1", "grid_template": null, "background_fill": null })).unwrap();
        let lay_id = SheetId::try_new("lay1").unwrap();
        let lay = s.document().layout_sheet(&lay_id).unwrap();
        assert_eq!(lay.spec.grid_template, None);
        assert_eq!(lay.spec.background_fill, None);

        let resized = s.execute("layout.applyCanvasPreset", json!({ "sheet_id": "lay1", "preset": "SingleColumn88mm" })).unwrap();
        assert_eq!(resized.payload["sheet_id"], json!("lay1"));

        let bad = s.execute("layout.addPanel", json!({ "sheet_id": "lay1", "bounds_pt": ["bad"] })).unwrap_err();
        assert_eq!(bad.code, ErrorCode::BadInput);
    }
}
