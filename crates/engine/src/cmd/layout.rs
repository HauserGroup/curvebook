//! Multi-panel publication layout creation, panel composition, alignment, and canvas preset commands.

use curvebook_chart::preflight_check_scene;
use curvebook_doc::{
    CanvasResizeMode, DocError, ErrorCode, FigureCanvasPreset, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MAX_LAYOUT_ITEMS,
    PageOrientation, PanelBadgeStyle, PhysicalUnit, RectPt,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::analysis::{de_present_opt, merge_json_struct};
use super::export::{compile_scene_for_sheet, de_opt_usize};
use super::graph::{parse_canvas_resize_mode, parse_figure_canvas_preset, parse_rect_pt, parse_rgba8, safe_scale_ratio, scale_finite, scale_rect_pt};
use super::helpers::{
    always_enabled, bad_input, bad_input_ptr, has_layout_sheet, next_unique_sheet_id, parse_sheet_id, require_graph_sheet, require_layout_sheet,
    require_layout_sheet_mut,
};
use super::image::{CropInsetsPatchParams, ImageBorderParams};
use super::params::{NonEmptyStrSchema, command_schema, deserialize_strict, parse_params};
use super::{CommandGroup, CommandSpec, Ctx, Effects, command_specs};
use crate::{CommandOutcome, EngineError};

const MUTATE_IDEMPOTENT: Effects = Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT);

/// Return all registered publication layout command specifications.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

static SPECS: &[CommandSpec] = &command_specs!(
    CommandGroup::Layout;
    {
        id: "layout.create",
        label: "New Layout",
        menu_path: "Insert/New Layout",
        summary: "Create a new publication layout sheet with a journal width preset, page orientation, and grid template.",
        beginner: "Creates a multi-panel figure page where you can arrange graphs, tables, and text callouts.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<LayoutCreateParams>,
        enabled: always_enabled,
        run: run_layout_create,
    },
    {
        id: "layout.setSpec",
        label: "Format Layout",
        menu_path: "Format/Layout",
        summary: "Update a layout sheet's page dimensions, journal preset, grid template, origin alignment, badge style, or background.",
        beginner: "Changes the page setup, grid arrangement, or panel lettering of an existing figure layout.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: command_schema::<LayoutSetSpecParams>,
        enabled: has_layout_sheet,
        run: run_layout_set_spec,
    },
    {
        id: "layout.addPanel",
        label: "Add Panel to Layout",
        menu_path: "Insert/Layout Panel",
        summary: "Place a live graph panel, live table panel, or markdown text callout onto a layout sheet.",
        beginner: "Adds a graph, data or result table, or label callout to an explicit rectangle or grid slot on a layout.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<LayoutAddPanelParams>,
        enabled: has_layout_sheet,
        run: run_layout_add_panel,
    },
    {
        id: "layout.alignOrigins",
        label: "Align Panel Origins",
        menu_path: "Arrange/Align Plot Origins",
        summary: "Enable or disable plot-area origin alignment across layout panels and optionally normalize live graph bounds.",
        beginner: "Lines up the X and Y axis origins of neighboring panels so multi-panel figures align cleanly.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: command_schema::<LayoutAlignOriginsParams>,
        enabled: has_layout_sheet,
        run: run_layout_align_origins,
    },
    {
        id: "layout.applyCanvasPreset",
        label: "Apply Layout Canvas Preset",
        menu_path: "Format/Layout Canvas Size Preset",
        summary: "Resize a layout page to a standard publication canvas preset while optionally preserving typography sizes.",
        beginner: "Resizes a whole layout figure to a journal column width or slide preset and runs preflight checks.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: command_schema::<LayoutApplyCanvasPresetParams>,
        enabled: has_layout_sheet,
        run: run_layout_apply_canvas_preset,
    },
    {
        id: "layout.equalizeScaling",
        label: "Equalize Scaling",
        menu_path: "Arrange/Equalize Scaling",
        summary: "Equalize rendered inner plot scaling across live graph panels on a layout sheet.",
        beginner: "Resizes selected or all graph panels on a layout to match the smallest, largest, or an explicit scale percentage.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: command_schema::<LayoutEqualizeScalingParams>,
        enabled: has_layout_sheet,
        run: run_layout_equalize_scaling,
    },
    {
        id: "layout.reorderPanel",
        label: "Reorder Panel",
        menu_path: "Arrange/Reorder Panel",
        summary: "Change the z-order stacking depth of a panel on a layout sheet.",
        beginner: "Moves a graph, table, or text panel forward or backward in the layout paint order.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<LayoutReorderPanelParams>,
        enabled: has_layout_sheet,
        run: run_layout_reorder_panel,
    },
    {
        id: "layout.insertImage",
        label: "Insert Image on Layout",
        menu_path: "Insert/Image on Layout",
        summary: "Embed a PNG, JPEG, or TIFF raster image item onto a publication layout sheet.",
        beginner: "Places a microscopy image, gel photo, or schematic panel onto a layout page.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: command_schema::<LayoutInsertImageParams>,
        enabled: has_layout_sheet,
        run: run_layout_insert_image,
    },
    {
        id: "layout.updateImage",
        label: "Format Embedded Layout Image",
        menu_path: "Format/Layout Image",
        summary: "Update placement bounds, aspect-ratio lock, rotation, crop insets, border, or badge override on a layout image.",
        beginner: "Crops, rotates, resizes, frames, or labels an embedded image panel on a layout sheet.",
        effects: MUTATE_IDEMPOTENT,
        input_schema: command_schema::<LayoutUpdateImageParams>,
        enabled: has_layout_sheet,
        run: run_layout_update_image,
    },
);

// ---------------------------------------------------------------------------
// Schemas & Typed Parameter Structs
// ---------------------------------------------------------------------------

fn grid_template_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["array", "null"],
        "items": { "type": "integer", "minimum": 0, "maximum": 255 },
        "minItems": 2,
        "maxItems": 2
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LayoutSpecFieldsParams {
    /// Journal width preset (e.g., `single_column`, `double_column`, `a4_portrait`).
    preset: Option<Value>,
    /// Page width in the configured physical unit.
    #[schemars(range(min = 0.01))]
    width: Option<f64>,
    /// Page height in the configured physical unit.
    #[schemars(range(min = 0.01))]
    height: Option<f64>,
    /// Physical unit (`mm`, `cm`, `in`, `pt`).
    unit: Option<Value>,
    /// Page orientation (`portrait` or `landscape`).
    orientation: Option<Value>,
    /// Optional `[rows, cols]` panel grid template; pass null to clear.
    #[serde(default, deserialize_with = "de_present_opt")]
    #[schemars(schema_with = "grid_template_schema")]
    grid_template: Option<Option<Value>>,
    /// Align adjacent live graph plot-area origins.
    align_plot_origins: Option<bool>,
    /// Automatic panel badge style (`none`, `uppercase`, `lowercase`).
    panel_badges: Option<Value>,
    /// Optional page background RGBA fill or `#RRGGBB` hex; null for transparent.
    #[serde(default, deserialize_with = "de_present_opt")]
    background_fill: Option<Option<Value>>,
}

impl LayoutSpecFieldsParams {
    fn has_any(&self) -> bool {
        self.preset.is_some()
            || self.width.is_some()
            || self.height.is_some()
            || self.unit.is_some()
            || self.orientation.is_some()
            || self.grid_template.is_some()
            || self.align_plot_origins.is_some()
            || self.panel_badges.is_some()
            || self.background_fill.is_some()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LayoutCreateParams {
    /// Optional explicit layout sheet ID.
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Optional layout sheet title (defaults to `Layout 1`).
    title: Option<String>,
    #[serde(flatten)]
    fields: LayoutSpecFieldsParams,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LayoutSetSpecParams {
    /// Target layout sheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Partial or full `LayoutSpec` JSON overlay.
    #[schemars(with = "Option<Map<String, Value>>")]
    spec: Option<Value>,
    /// Optional updated layout sheet title.
    title: Option<String>,
    #[serde(flatten)]
    fields: LayoutSpecFieldsParams,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayoutAddPanelParams {
    /// Target layout sheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Panel kind (`graph`, `table`, or `text`).
    #[schemars(length(min = 1))]
    kind: Option<String>,
    /// Optional explicit panel identifier.
    panel_id: Option<String>,
    /// Source graph sheet ID when adding a live graph panel.
    #[schemars(length(min = 1))]
    graph_sheet_id: Option<String>,
    /// Source data or result sheet ID when adding a live table panel.
    #[schemars(length(min = 1))]
    source_sheet_id: Option<String>,
    /// Optional result tab ID or title for live table panels.
    tab_id: Option<String>,
    /// Markdown body text for text callout panels.
    markdown: Option<String>,
    /// Explicit `[x, y, width, height]` array or `RectPt` object in points.
    bounds_pt: Option<Value>,
    /// Grid auto-placement outer page margin in points (default 18.0).
    #[schemars(range(min = 0.0))]
    margin_pt: Option<f64>,
    /// Grid auto-placement inter-panel gutter in points (default 12.0).
    #[schemars(range(min = 0.0))]
    gutter_pt: Option<f64>,
    /// Zero-based grid row index when auto-placing.
    #[serde(default, deserialize_with = "de_opt_usize")]
    row: Option<usize>,
    /// Zero-based grid column index when auto-placing.
    #[serde(default, deserialize_with = "de_opt_usize")]
    col: Option<usize>,
    /// Live graph display scale percentage (default 100.0).
    #[schemars(range(min = 0.01))]
    scale_percent: Option<f64>,
    /// Custom panel badge label overriding automatic lettering.
    badge_override: Option<String>,
    /// Text callout font size in points (default 10.0).
    #[schemars(range(min = 0.5))]
    font_size_pt: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayoutAlignOriginsParams {
    /// Target layout sheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Enable or disable plot-origin alignment (default true).
    align: Option<bool>,
    /// Snap nearby live graph panel X/Y bounds into shared row/column bands (default true).
    normalize_bounds: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayoutApplyCanvasPresetParams {
    /// Target graph or layout sheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Canvas preset (`SingleColumn88mm`, `ColumnAndHalf130mm`, `DoubleColumn180mm`, `FullPagePortrait`, `Slide16x9`, `PosterPanel`).
    #[schemars(length(min = 1))]
    preset: String,
    /// Resize mode (`KeepTypographyPt` or `ProportionalScale`).
    #[schemars(length(min = 1))]
    resize_mode: Option<String>,
}

fn equalize_mode_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "anyOf": [
            {
                "type": "string",
                "enum": ["match_smallest", "match_largest", "explicit_percent", "MatchSmallest", "MatchLargest", "ExplicitPercent"]
            },
            { "type": "null" }
        ]
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayoutEqualizeScalingParams {
    /// Target layout sheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Scaling equalization rule (`match_smallest`, `match_largest`, or `explicit_percent`).
    #[serde(default)]
    #[schemars(schema_with = "equalize_mode_schema")]
    mode: Option<String>,
    /// Explicit target scale percentage in [5.0, 500.0] for `explicit_percent`.
    #[serde(default)]
    #[schemars(range(min = 5.0, max = 500.0))]
    percent: Option<f64>,
    /// Optional subset of live graph panel IDs to equalize.
    #[serde(default)]
    #[schemars(with = "Option<Vec<NonEmptyStrSchema>>")]
    panel_ids: Option<Vec<String>>,
    /// Optional subset of zero-based panel indices to equalize.
    #[serde(default)]
    panel_indices: Option<Vec<usize>>,
}

fn reorder_action_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "enum": [
            "bring_to_front",
            "bring_forward",
            "send_backward",
            "send_to_back",
            "BringToFront",
            "BringForward",
            "SendBackward",
            "SendToBack"
        ]
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayoutReorderPanelParams {
    /// Target layout sheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Zero-based index in `layout.spec.items`.
    #[serde(default, deserialize_with = "de_opt_usize")]
    panel_index: Option<usize>,
    /// Panel ID in `layout.spec.items`.
    #[serde(default)]
    #[schemars(length(min = 1))]
    panel_id: Option<String>,
    /// Z-order stacking action.
    #[schemars(schema_with = "reorder_action_schema")]
    action: String,
}

// ---------------------------------------------------------------------------
// Command Runners
// ---------------------------------------------------------------------------

fn run_layout_create(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let params: LayoutCreateParams = parse_params(&command_schema::<LayoutCreateParams>(), raw)?;
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
        return Err(bad_input("layout `width` and `height` must be finite and >= 0.01"));
    }
    let mut seen_item_ids = std::collections::HashSet::with_capacity(spec.items.len());
    for (idx, item) in spec.items.iter().enumerate() {
        let item_id = item.id();
        if !seen_item_ids.insert(item_id) {
            return Err(bad_input(format!("duplicate layout item id `{item_id}` at item[{idx}]"))
                .with_remediation("Assign a unique `panel_id` to each layout item."));
        }
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, scale_percent, .. } => {
                if *scale_percent < 0.01 {
                    return Err(bad_input(format!("layout item[{idx}] `scale_percent` must be >= 0.01 (got {scale_percent})")));
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
                        return Err(bad_input(format!("layout item[{idx}] source `{source_sheet_id}` must reference a data, info, or result sheet")));
                    }
                    None => return Err(DocError::SheetNotFound { sheet_id: source_sheet_id.clone() }.into()),
                }
            }
            LayoutItem::TextCallout { font_size_pt, .. } => {
                if *font_size_pt < 0.5 {
                    return Err(bad_input(format!("layout item[{idx}] `font_size_pt` must be >= 0.5 (got {font_size_pt})")));
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
    let params: LayoutSetSpecParams = parse_params(&command_schema::<LayoutSetSpecParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = &mut ctx.session.doc;

    let current = require_layout_sheet(doc, &id)?;
    let mut spec = current.spec.clone();

    if let Some(ref spec_val) = params.spec {
        if params.fields.has_any() {
            return Err(bad_input_ptr("/spec", "cannot combine `spec` with shorthand layout spec fields in `layout.setSpec`")
                .with_remediation("Supply either `spec` or top-level shorthand fields (`preset`, `width`, `height`, `unit`, `orientation`, `grid_template`, `align_plot_origins`, `panel_badges`, `background_fill`), not both."));
        }
        if let Some(items_arr) = spec_val.get("items").and_then(Value::as_array)
            && items_arr.len() > MAX_LAYOUT_ITEMS
        {
            return Err(DocError::TooManyLayoutItems { count: items_arr.len(), max: MAX_LAYOUT_ITEMS }.into());
        }
        let merged = merge_json_struct(&spec, spec_val)?;
        spec = deserialize_strict::<LayoutSpec>(&merged, "/spec").map_err(|mut err| {
            let mut mapped = bad_input(format!("invalid layout `spec`: {err}"))
                .with_remediation("Supply fields matching LayoutSpec or use shorthand fields without `spec`.");
            mapped.details = err.details.take().or_else(|| Some(json!({ "pointer": "/spec" })));
            mapped
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
    let params: LayoutAddPanelParams = parse_params(&command_schema::<LayoutAddPanelParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = &mut ctx.session.doc;

    let next_idx = {
        let layout = require_layout_sheet(doc, &id)?;
        layout.spec.items.len()
    };
    if next_idx >= MAX_LAYOUT_ITEMS {
        return Err(DocError::TooManyLayoutItems { count: next_idx + 1, max: MAX_LAYOUT_ITEMS }.into());
    }
    let (bounds_pt, panel_id) = {
        let layout = require_layout_sheet(doc, &id)?;
        let bounds = resolve_panel_bounds(&layout.spec, next_idx, &params)?;
        let pid = unique_layout_panel_id(&layout.spec.items, params.panel_id.as_deref());
        (bounds, pid)
    };

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
                bad_input_ptr("/graph_sheet_id", "missing `graph_sheet_id` for `graph` panel")
                    .with_remediation("Provide a valid `graph_sheet_id` referencing an existing graph sheet.")
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
                bad_input_ptr("/source_sheet_id", "missing `source_sheet_id` for `table` panel")
                    .with_remediation("Provide a `source_sheet_id` referencing an existing sheet.")
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
                bad_input_ptr("/markdown", "missing `markdown` for text callout panel").with_remediation("Supply `markdown` for a text panel.")
            })?,
            bounds_pt,
            font_size_pt: params.font_size_pt.unwrap_or(10.0),
        },
        other => {
            return Err(bad_input(format!("unknown layout panel kind `{other}`"))
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
    let params: LayoutAlignOriginsParams = parse_params(&command_schema::<LayoutAlignOriginsParams>(), raw)?;
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
    let params: LayoutApplyCanvasPresetParams = parse_params(&command_schema::<LayoutApplyCanvasPresetParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let preset = parse_figure_canvas_preset(&params.preset)?;
    let mode = parse_canvas_resize_mode(params.resize_mode.as_deref())?;

    let doc = &mut ctx.session.doc;
    let mut updated_spec = require_layout_sheet(doc, &id)?.spec.clone();
    resize_layout_spec(&mut updated_spec, preset, mode)?;
    validate_layout_spec_in_doc(doc, &id, &updated_spec)?;

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
            let p = percent.ok_or_else(|| bad_input_ptr("/percent", "`explicit_percent` mode requires `percent` in [5.0, 500.0]"))?;
            validate_scale_percent_range(p)?;
            Ok(EqualizeScaleMode::ExplicitPercent(p))
        }
        Some(other) => Err(bad_input_ptr("/mode", format!("invalid `equalizeScaling` mode `{other}`"))),
    }
}

fn validate_scale_percent_range(p: f64) -> Result<(), EngineError> {
    if !p.is_finite() || !(5.0..=500.0).contains(&p) {
        return Err(bad_input_ptr("/percent", "`percent` must be finite and in [5.0, 500.0]"));
    }
    Ok(())
}

fn layout_item_id(item: &LayoutItem) -> &str {
    item.id()
}

fn run_layout_equalize_scaling(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let params: LayoutEqualizeScalingParams = parse_params(&command_schema::<LayoutEqualizeScalingParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let mode = parse_equalize_scale_mode(params.mode.as_deref(), params.percent)?;
    let doc = &ctx.session.doc;
    let mut candidate_spec = require_layout_sheet(doc, &id)?.spec.clone();

    let mut target_indices: Vec<usize> = Vec::new();
    let filter_by_id_or_idx = params.panel_ids.is_some() || params.panel_indices.is_some();
    if let Some(ref indices) = params.panel_indices {
        for &idx in indices {
            let Some(item) = candidate_spec.items.get(idx) else {
                return Err(bad_input(format!("`panel_index` {idx} out of bounds")));
            };
            if !matches!(item, LayoutItem::LiveGraph { .. }) {
                return Err(bad_input(format!("panel `{idx}` is not a live graph panel")));
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
            if !matches!(candidate_spec.items.get(idx), Some(LayoutItem::LiveGraph { .. })) {
                return Err(bad_input(format!("panel `{pid}` is not a live graph panel")));
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
        let Some(LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, scale_percent, .. }) = candidate_spec.items.get(idx) else {
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
        EqualizeScaleMode::ExplicitPercent(p) => metrics.first().map_or(1.0, |m| m.plot_w) * (p / 100.0).max(0.05),
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
    let params: LayoutReorderPanelParams = parse_params(&command_schema::<LayoutReorderPanelParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = &ctx.session.doc;
    let mut candidate_spec = require_layout_sheet(doc, &id)?.spec.clone();
    let len = candidate_spec.items.len();
    if len == 0 {
        return Err(EngineError::with_code(ErrorCode::Conflict, format!("layout `{id}` has no panels to reorder")));
    }

    let idx_from_opt = if let Some(idx) = params.panel_index {
        if idx >= len {
            return Err(bad_input_ptr("/panel_index", format!("`panel_index` {idx} out of bounds ({len} panels)")));
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
            return Err(bad_input("`panel_index` and `panel_id` refer to different panels"));
        }
        (Some(a), _) | (None, Some(a)) => a,
        (None, None) => {
            return Err(bad_input("provide either `panel_index` or `panel_id`"));
        }
    };

    let norm_action: String = params.action.trim().chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let (canonical_action, new_idx) = match norm_action.as_str() {
        "bringtofront" | "front" | "top" => ("bring_to_front", len - 1),
        "bringforward" | "forward" | "up" => ("bring_forward", (old_idx + 1).min(len - 1)),
        "sendbackward" | "backward" | "down" => ("send_backward", old_idx.saturating_sub(1)),
        "sendtoback" | "back" | "bottom" => ("send_to_back", 0),
        _ => {
            return Err(bad_input_ptr(
                "/action",
                format!("unknown reorder action `{}`; expected `bring_to_front`, `bring_forward`, `send_backward`, or `send_to_back`", params.action),
            ));
        }
    };

    let moved_item = candidate_spec.items.get(old_idx).ok_or_else(|| bad_input(format!("panel index {old_idx} out of bounds")))?;
    let moved_id = layout_item_id(moved_item).to_owned();
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
        if done.get(i).copied().unwrap_or(true) {
            continue;
        }
        let Some(&(_, ref_r)) = rects.get(i) else {
            continue;
        };
        let span = if vertical_col { ref_r.width } else { ref_r.height }.max(40.0) * 0.35;
        let ref_coord = if vertical_col { ref_r.x } else { ref_r.y };
        let mut target = ref_coord;
        let mut group = vec![i];
        for (j, (_, rj)) in rects.iter().enumerate().skip(i + 1) {
            let coord_j = if vertical_col { rj.x } else { rj.y };
            if !done.get(j).copied().unwrap_or(true) && (coord_j - ref_coord).abs() <= span {
                target = target.max(coord_j);
                group.push(j);
            }
        }
        for idx in group {
            if let Some(d) = done.get_mut(idx) {
                *d = true;
            }
            if let Some((_, rect)) = rects.get_mut(idx) {
                if vertical_col {
                    rect.x = target;
                } else {
                    rect.y = target;
                }
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
                    *font_size_pt = scale_finite(*font_size_pt, scale_1d, "layout.text_callout.font_size_pt")?.max(0.5);
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
    let [rows_v, cols_v] = val
        .as_array()
        .map(Vec::as_slice)
        .filter(|a| a.len() == 2)
        .ok_or_else(|| bad_input_ptr("/grid_template", "`grid_template` must be `[rows, cols]`"))?
    else {
        return Err(bad_input_ptr("/grid_template", "`grid_template` must be `[rows, cols]`"));
    };
    let rows = parse_u8_count(rows_v, "`grid_template[0]`", "/grid_template/0")?;
    let cols = parse_u8_count(cols_v, "`grid_template[1]`", "/grid_template/1")?;
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
    Err(bad_input_ptr(pointer, format!("{label} must be u8")))
}

fn parse_journal_preset(val: &Value) -> Result<JournalWidthPreset, EngineError> {
    if let Ok(p) = serde_json::from_value::<JournalWidthPreset>(val.clone()) {
        return Ok(p);
    }
    if let Some(norm) = val.as_str().map(|s| s.trim().to_ascii_lowercase()) {
        if let Some(preset) = JournalWidthPreset::from_canonical_name(&norm) {
            return Ok(preset);
        }
        match norm.as_str() {
            "single_column" | "single" => return Ok(JournalWidthPreset::SingleColumn85Mm),
            "column_1_5" | "one_and_half" => return Ok(JournalWidthPreset::OneAndHalfColumn120Mm),
            "double_column" | "double" => return Ok(JournalWidthPreset::DoubleColumn174Mm),
            "us_letter" => return Ok(JournalWidthPreset::LetterPortrait),
            "full_page" => return Ok(JournalWidthPreset::A4Portrait),
            _ => {}
        }
    }
    Err(bad_input_ptr("/preset", format!("invalid layout `preset`: {val}")))
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
        _ => Err(bad_input_ptr("/unit", format!("invalid physical `unit`: {val}"))),
    }
}

fn parse_page_orientation(val: &Value) -> Result<PageOrientation, EngineError> {
    if let Ok(o) = serde_json::from_value::<PageOrientation>(val.clone()) {
        return Ok(o);
    }
    match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("portrait") => Ok(PageOrientation::Portrait),
        Some("landscape") => Ok(PageOrientation::Landscape),
        _ => Err(bad_input_ptr("/orientation", format!("invalid `orientation`: {val}"))),
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
        _ => Err(bad_input_ptr("/panel_badges", format!("invalid `panel_badges`: {val}"))),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayoutInsertImageParams {
    /// Target LayoutSheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional explicit layout panel identifier.
    #[serde(default)]
    panel_id: Option<String>,
    /// Regular local file path to a PNG, JPEG, or TIFF image (URLs rejected).
    #[serde(default)]
    #[schemars(length(min = 1))]
    path: Option<String>,
    /// Base64-encoded PNG, JPEG, or TIFF image payload.
    #[serde(default)]
    #[schemars(length(min = 1))]
    bytes_base64: Option<String>,
    /// Existing content-addressed image `asset_id` already stored in the project.
    #[serde(default)]
    #[schemars(length(min = 1))]
    asset_id: Option<String>,
    /// Optional placement `[x, y, width, height]` or `RectPt` in points.
    #[serde(default)]
    bounds_pt: Option<Value>,
    /// Clockwise rotation in degrees around the center of `bounds_pt`.
    #[serde(default)]
    #[schemars(range(min = -3600.0, max = 3600.0))]
    rotation_deg: Option<f64>,
    /// Preserve intrinsic image aspect ratio when resizing one dimension (default `true`).
    #[serde(default)]
    lock_aspect_ratio: Option<bool>,
    /// Optional crop insets object.
    #[serde(default)]
    #[schemars(with = "Option<CropInsetsPatchParams>")]
    crop: Option<Value>,
    /// Optional image border specification.
    #[serde(default, deserialize_with = "de_present_opt")]
    #[schemars(with = "Option<ImageBorderParams>")]
    border: Option<Option<Value>>,
    /// Optional panel badge text rendered in top-left corner.
    #[serde(default)]
    badge_override: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayoutUpdateImageParams {
    /// Target LayoutSheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Target EmbeddedImage `panel_id` (provide `panel_id` or `item_index`).
    #[serde(default)]
    panel_id: Option<String>,
    /// Zero-based index of the EmbeddedImage item.
    #[serde(default)]
    item_index: Option<usize>,
    /// Updated placement `[x, y, width, height]` or `RectPt` in points.
    #[serde(default)]
    bounds_pt: Option<Value>,
    /// Updated width in points (> 0); adjusts `height_pt` when aspect ratio is locked.
    #[serde(default)]
    #[schemars(range(min = 0.001))]
    width_pt: Option<f64>,
    /// Updated height in points (> 0); adjusts `width_pt` when aspect ratio is locked.
    #[serde(default)]
    #[schemars(range(min = 0.001))]
    height_pt: Option<f64>,
    /// Updated clockwise rotation in degrees.
    #[serde(default)]
    #[schemars(range(min = -3600.0, max = 3600.0))]
    rotation_deg: Option<f64>,
    /// Lock or unlock intrinsic aspect ratio during single-axis resize.
    #[serde(default)]
    lock_aspect_ratio: Option<bool>,
    /// Optional crop insets object.
    #[serde(default)]
    #[schemars(with = "Option<CropInsetsPatchParams>")]
    crop: Option<Value>,
    /// Optional image border specification.
    #[serde(default, deserialize_with = "de_present_opt")]
    #[schemars(with = "Option<ImageBorderParams>")]
    border: Option<Option<Value>>,
    /// Updated corner badge label; pass null to clear.
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

pub(super) fn unique_layout_panel_id(items: &[LayoutItem], requested: Option<&str>) -> String {
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
    let params: LayoutInsertImageParams = parse_params(&command_schema::<LayoutInsertImageParams>(), raw)?;
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
    let params: LayoutUpdateImageParams = parse_params(&command_schema::<LayoutUpdateImageParams>(), raw)?;
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
            return Err(bad_input("`item_index` and `panel_id` refer to different layout items"));
        }
        (Some(a), _) | (None, Some(a)) => a,
        (None, None) => {
            return Err(bad_input("provide either `panel_id` or `item_index`"));
        }
    };

    let item =
        candidate_spec.items.get_mut(target_idx).ok_or_else(|| bad_input(format!("layout item `{target_idx}` out of bounds on `{layout_id}`")))?;
    let LayoutItem::EmbeddedImage { id: item_id, asset_id, bounds_pt, rotation_deg, lock_aspect_ratio, crop, border, badge_override } = item else {
        return Err(bad_input(format!("layout item `{target_idx}` on `{layout_id}` is not an embedded image")));
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
    use crate::Session;

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
        s.execute("layout.addPanel", json!({ "sheet_id": "lay1", "kind": "text", "markdown": "Summary note" })).unwrap();
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
