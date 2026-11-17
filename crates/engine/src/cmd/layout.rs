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
use super::helpers::{next_unique_sheet_id, parse_sheet_id, require_layout_sheet, require_layout_sheet_mut};
use super::params::{
    any_json_schema, array_schema, bool_schema, int_schema, non_empty_str_schema, nullable_schema, num_schema, obj_schema_with_aliases, parse_params,
    str_schema,
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

fn validate_layout_spec_in_doc(
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
            LayoutItem::LiveTable { id: panel_id, source_sheet_id: source_id, tab_id: params.tab_id, bounds_pt }
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
            LayoutItem::LiveTable { .. } | LayoutItem::TextCallout { .. } => None,
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

// ---------------------------------------------------------------------------
// Geometry, Resize, and Field Parsers
// ---------------------------------------------------------------------------

fn align_live_graph_bounds(items: &mut [LayoutItem]) {
    let mut rects: Vec<(usize, RectPt)> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, it)| match it {
            LayoutItem::LiveGraph { bounds_pt, .. } => Some((idx, *bounds_pt)),
            LayoutItem::LiveTable { .. } | LayoutItem::TextCallout { .. } => None,
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
    fn registry_contains_all_five_layout_commands() {
        assert_eq!(specs().len(), 5);
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
