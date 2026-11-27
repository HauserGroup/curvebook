//! Editing and history commands (`edit.undo`, `edit.redo`, `edit.pasteResultTable`).

use curvebook_doc::{DocError, EmbeddedTableStyle, ErrorCode, GraphAnnotation, LayoutItem, MAX_LAYOUT_ITEMS, RectPt, SheetSection};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::analysis::de_present_opt;
use crate::cmd::graph::{GraphImageInsertArgs, insert_image_on_graph, parse_rect_pt, parse_rgba8, validate_graph_spec_in_doc};
use crate::cmd::helpers::{
    always_enabled, bad_input, bad_input_ptr, has_graph_or_layout, parse_sheet_id, require_graph_sheet, require_graph_sheet_mut,
    require_layout_sheet, require_layout_sheet_mut, require_result_sheet,
};
use crate::cmd::image::{CropInsetsPatchParams, ImageBorderParams};
use crate::cmd::layout::{LayoutImageInsertArgs, insert_image_on_layout, unique_layout_panel_id, validate_layout_spec_in_doc};
use crate::cmd::params::{command_schema, empty_object_schema, parse_params};
use crate::cmd::{CommandGroup, CommandSpec, Ctx, Effects, command_specs};
use crate::{CommandOutcome, EngineError};

#[derive(Debug, Deserialize)]
struct EmptyEditParams {}

fn run_edit_undo(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let _parsed: EmptyEditParams = parse_params(&empty_object_schema(), params)?;
    let applied = ctx.session.undo()?;
    let message = if applied { "undid last edit" } else { "nothing to undo" };
    Ok(CommandOutcome {
        command_id: String::from("edit.undo"),
        mutated: applied,
        message: String::from(message),
        payload: json!({ "applied": applied }),
        warnings: Vec::new(),
    })
}

fn run_edit_redo(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let _parsed: EmptyEditParams = parse_params(&empty_object_schema(), params)?;
    let applied = ctx.session.redo()?;
    let message = if applied { "redid edit" } else { "nothing to redo" };
    Ok(CommandOutcome {
        command_id: String::from("edit.redo"),
        mutated: applied,
        message: String::from(message),
        payload: json!({ "applied": applied }),
        warnings: Vec::new(),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditPasteResultTableParams {
    /// Source ResultSheet (or DataSheet for Layout target) ID.
    #[schemars(length(min = 1))]
    source_sheet_id: String,
    /// Target GraphSheet or LayoutSheet ID.
    #[schemars(length(min = 1))]
    target_sheet_id: String,
    /// Optional ResultTab ID or title (defaults to first tab).
    #[serde(default)]
    tab_id: Option<String>,
    /// Optional placement `[x, y, width, height]` or `RectPt` in points.
    #[serde(default)]
    bounds_pt: Option<Value>,
    /// Optional zero-based indices to include.
    #[serde(default)]
    selected_rows: Option<Vec<usize>>,
    /// Optional zero-based indices to include.
    #[serde(default)]
    selected_cols: Option<Vec<usize>>,
    /// Optional EmbeddedTableStyle overrides.
    #[serde(default)]
    #[schemars(with = "Option<Map<String, Value>>")]
    style: Option<Value>,
    /// Transpose rows and columns when rendering the pasted table.
    #[serde(default)]
    transpose: Option<bool>,
    /// Optional panel identifier when pasting onto a LayoutSheet.
    #[serde(default)]
    panel_id: Option<String>,
}

fn parse_embedded_table_style(style_val: Option<&Value>, transpose_shorthand: Option<bool>) -> Result<EmbeddedTableStyle, EngineError> {
    let mut style = EmbeddedTableStyle::default();
    if let Some(val) = style_val {
        let Some(obj) = val.as_object() else {
            return Err(bad_input("`style` must be an object"));
        };
        for key in obj.keys() {
            if !matches!(
                key.as_str(),
                "show_column_titles"
                    | "show_row_titles"
                    | "transpose"
                    | "bold_headers"
                    | "font_size_pt"
                    | "outer_border_width_pt"
                    | "gridline_width_pt"
                    | "border_color"
                    | "fill_color"
            ) {
                return Err(bad_input_ptr(&format!("/style/{key}"), format!("unknown `style` field `{key}`")).with_remediation(
                    "Allowed `style` fields: `show_column_titles`, `show_row_titles`, `transpose`, `bold_headers`, `font_size_pt`, `outer_border_width_pt`, `gridline_width_pt`, `border_color`, `fill_color`.",
                ));
            }
        }
        if let Some(v) = obj.get("show_column_titles") {
            style.show_column_titles = v.as_bool().ok_or_else(|| bad_input("`style.show_column_titles` must be bool"))?;
        }
        if let Some(v) = obj.get("show_row_titles") {
            style.show_row_titles = v.as_bool().ok_or_else(|| bad_input("`style.show_row_titles` must be bool"))?;
        }
        if let Some(v) = obj.get("transpose") {
            style.transpose = v.as_bool().ok_or_else(|| bad_input("`style.transpose` must be bool"))?;
        }
        if let Some(v) = obj.get("bold_headers") {
            style.bold_headers = v.as_bool().ok_or_else(|| bad_input("`style.bold_headers` must be bool"))?;
        }
        if let Some(v) = obj.get("font_size_pt") {
            let f = v.as_f64().ok_or_else(|| bad_input("`style.font_size_pt` must be numeric"))?;
            if !f.is_finite() || f <= 0.0 {
                return Err(bad_input("`style.font_size_pt` must be finite and > 0"));
            }
            style.font_size_pt = f;
        }
        if let Some(v) = obj.get("outer_border_width_pt") {
            let f = v.as_f64().ok_or_else(|| bad_input("`style.outer_border_width_pt` must be numeric"))?;
            if !f.is_finite() || f < 0.0 {
                return Err(bad_input("`style.outer_border_width_pt` must be finite and >= 0"));
            }
            style.outer_border_width_pt = f;
        }
        if let Some(v) = obj.get("gridline_width_pt") {
            let f = v.as_f64().ok_or_else(|| bad_input("`style.gridline_width_pt` must be numeric"))?;
            if !f.is_finite() || f < 0.0 {
                return Err(bad_input("`style.gridline_width_pt` must be finite and >= 0"));
            }
            style.gridline_width_pt = f;
        }
        if let Some(v) = obj.get("border_color") {
            style.border_color = parse_rgba8(v)?;
        }
        if let Some(v) = obj.get("fill_color") {
            style.fill_color = if v.is_null() { None } else { Some(parse_rgba8(v)?) };
        }
    }
    if let Some(t) = transpose_shorthand {
        style.transpose = t;
    }
    style.validate(curvebook_doc::TableKind::Column).map_err(EngineError::from)?;
    Ok(style)
}

fn validate_optional_result_tab(
    doc: &curvebook_doc::ProjectDocument,
    res_id: &curvebook_doc::SheetId,
    tab_id: Option<&str>,
) -> Result<(), EngineError> {
    let res = require_result_sheet(doc, res_id)?;
    if let Some(tid) = tab_id
        && res.has_displayable_output()
        && !res.tabs.iter().any(|t| t.id == tid || t.title.eq_ignore_ascii_case(tid))
    {
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("result tab `{tid}` not found on `{res_id}`"))
            .with_details(json!({ "sheet_id": res_id.as_str(), "tab_id": tid })));
    }
    Ok(())
}

fn run_edit_paste_result_table(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let params: EditPasteResultTableParams = parse_params(&command_schema::<EditPasteResultTableParams>(), raw)?;
    let source_id = parse_sheet_id(&params.source_sheet_id)?;
    let target_id = parse_sheet_id(&params.target_sheet_id)?;
    let style = parse_embedded_table_style(params.style.as_ref(), params.transpose)?;
    let doc = &mut ctx.session.doc;

    let tab_opt = params.tab_id.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let section = doc.sheet_section(&target_id).ok_or_else(|| {
        EngineError::with_code(ErrorCode::NotFound, format!("target graph or layout sheet `{target_id}` not found"))
            .with_details(json!({ "sheet_id": target_id.as_str() }))
    })?;

    match section {
        SheetSection::Graphs => {
            validate_optional_result_tab(doc, &source_id, tab_opt.as_deref())?;
            let bounds_pt = parse_rect_pt(params.bounds_pt.as_ref(), RectPt::new(180.0, 40.0, 180.0, 90.0))?;
            if bounds_pt.width <= 0.0 || bounds_pt.height <= 0.0 {
                return Err(bad_input("`bounds_pt` width and height must be > 0"));
            }
            let mut spec = require_graph_sheet(doc, &target_id)?.spec.clone();
            spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
                result_sheet_id: source_id.clone(),
                tab_id: tab_opt.clone(),
                bounds_pt,
                selected_rows: params.selected_rows,
                selected_cols: params.selected_cols,
                style,
            });
            validate_graph_spec_in_doc(doc, &target_id, &spec)?;
            let idx = spec.annotations.len() - 1;
            let graph = require_graph_sheet_mut(doc, &target_id)?;
            graph.spec = spec;
            graph.initialized = true;
            Ok(CommandOutcome::mutated(
                "edit.pasteResultTable",
                format!("pasted result table `{source_id}` onto graph `{target_id}`"),
                json!({
                    "target_sheet_id": target_id.as_str(),
                    "target_kind": "graph",
                    "source_sheet_id": source_id.as_str(),
                    "tab_id": tab_opt,
                    "annotation_index": idx,
                    "bounds_pt": bounds_pt,
                }),
            ))
        }
        SheetSection::Layouts => {
            match doc.sheet_section(&source_id) {
                Some(SheetSection::Analyses) => validate_optional_result_tab(doc, &source_id, tab_opt.as_deref())?,
                Some(SheetSection::Data | SheetSection::Info) => {}
                Some(_) => {
                    return Err(bad_input(format!("table source `{source_id}` must reference a result, data, or info sheet")));
                }
                None => return Err(DocError::SheetNotFound { sheet_id: source_id }.into()),
            }
            let mut candidate_spec = require_layout_sheet(doc, &target_id)?.spec.clone();
            if candidate_spec.items.len() >= MAX_LAYOUT_ITEMS {
                return Err(DocError::TooManyLayoutItems { count: candidate_spec.items.len() + 1, max: MAX_LAYOUT_ITEMS }.into());
            }
            let bounds_pt = parse_rect_pt(params.bounds_pt.as_ref(), RectPt::new(24.0, 24.0, 220.0, 110.0))?;
            if bounds_pt.width <= 0.0 || bounds_pt.height <= 0.0 {
                return Err(bad_input("`bounds_pt` width and height must be > 0"));
            }
            let panel_id = unique_layout_panel_id(&candidate_spec.items, params.panel_id.as_deref());
            candidate_spec
                .add_item(LayoutItem::LiveTable {
                    id: panel_id.clone(),
                    source_sheet_id: source_id.clone(),
                    tab_id: tab_opt.clone(),
                    bounds_pt,
                    selected_rows: params.selected_rows,
                    selected_cols: params.selected_cols,
                    style,
                })
                .map_err(DocError::from)?;
            validate_layout_spec_in_doc(doc, &target_id, &candidate_spec)?;
            let idx = candidate_spec.items.len() - 1;
            let layout = require_layout_sheet_mut(doc, &target_id)?;
            layout.spec = candidate_spec;
            Ok(CommandOutcome::mutated(
                "edit.pasteResultTable",
                format!("pasted live table `{source_id}` onto layout `{target_id}`"),
                json!({
                    "target_sheet_id": target_id.as_str(),
                    "target_kind": "layout",
                    "source_sheet_id": source_id.as_str(),
                    "tab_id": tab_opt,
                    "panel_id": panel_id,
                    "panel_index": idx,
                    "bounds_pt": bounds_pt,
                }),
            ))
        }
        SheetSection::Data | SheetSection::Info | SheetSection::Analyses => {
            Err(bad_input_ptr("/target_sheet_id", format!("target sheet `{target_id}` must be a graph or layout sheet")))
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditPasteImageParams {
    /// Target GraphSheet or LayoutSheet ID.
    #[schemars(length(min = 1))]
    target_sheet_id: String,
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
    /// Optional crop inset patch.
    #[serde(default)]
    #[schemars(with = "Option<CropInsetsPatchParams>")]
    crop: Option<Value>,
    /// Optional image border specification (or null to clear).
    #[serde(default, deserialize_with = "de_present_opt")]
    #[schemars(with = "Option<ImageBorderParams>")]
    border: Option<Option<Value>>,
    /// Optional panel identifier when pasting onto a LayoutSheet.
    #[serde(default)]
    panel_id: Option<String>,
    /// Optional badge label when pasting onto a LayoutSheet.
    #[serde(default)]
    badge_override: Option<String>,
}

fn run_edit_paste_image(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let params: EditPasteImageParams = parse_params(&command_schema::<EditPasteImageParams>(), raw)?;
    let target_id = parse_sheet_id(&params.target_sheet_id)?;
    let doc = &mut ctx.session.doc;
    let section = doc.sheet_section(&target_id).ok_or_else(|| {
        EngineError::with_code(ErrorCode::NotFound, format!("target graph or layout sheet `{target_id}` not found"))
            .with_details(json!({ "pointer": "/target_sheet_id", "sheet_id": target_id.as_str() }))
    })?;
    match section {
        SheetSection::Graphs => {
            if params.panel_id.is_some() || params.badge_override.is_some() {
                return Err(bad_input("`panel_id` and `badge_override` apply only when pasting an image onto a LayoutSheet"));
            }
            insert_image_on_graph(
                doc,
                &target_id,
                GraphImageInsertArgs {
                    command_id: "edit.pasteImage",
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
        SheetSection::Layouts => insert_image_on_layout(
            doc,
            &target_id,
            LayoutImageInsertArgs {
                command_id: "edit.pasteImage",
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
        ),
        SheetSection::Data | SheetSection::Info | SheetSection::Analyses => {
            Err(bad_input_ptr("/target_sheet_id", format!("target sheet `{target_id}` must be a graph or layout sheet")))
        }
    }
}

static SPECS: &[CommandSpec] = &command_specs!(
    CommandGroup::Edit;
    {
        id: "edit.undo",
        label: "Undo",
        menu_path: "Edit/Undo",
        shortcut: "Cmd+Z",
        summary: "Undo the most recent document-mutating command.",
        beginner: "Restores the project snapshot captured immediately before the last edit; succeeds with mutated=false when the undo stack is empty.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: empty_object_schema,
        enabled: always_enabled,
        run: run_edit_undo,
    },
    {
        id: "edit.redo",
        label: "Redo",
        menu_path: "Edit/Redo",
        shortcut: "Cmd+Shift+Z",
        summary: "Redo the most recently undone document edit.",
        beginner: "Re-applies the project snapshot reverted by the last undo command; succeeds with mutated=false when the redo stack is empty.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: empty_object_schema,
        enabled: always_enabled,
        run: run_edit_redo,
    },
    {
        id: "edit.pasteResultTable",
        label: "Paste Result Table",
        menu_path: "Edit/Paste Result Table",
        summary: "Paste a live result or data table onto a graph annotation or multi-panel layout.",
        beginner: "Places a formatted, auto-updating result table onto the selected graph or layout page.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<EditPasteResultTableParams>,
        enabled: has_graph_or_layout,
        run: run_edit_paste_result_table,
    },
    {
        id: "edit.pasteImage",
        label: "Paste Image",
        menu_path: "Edit/Paste Image",
        summary: "Paste or insert a PNG, JPEG, or TIFF raster image onto a graph annotation or multi-panel layout.",
        beginner: "Pastes an image from bytes, local file path, or existing project asset onto a graph or layout page.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: command_schema::<EditPasteImageParams>,
        enabled: has_graph_or_layout,
        run: run_edit_paste_image,
    },
);

/// Registered command specifications for the `edit` group.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use curvebook_doc::{ErrorCode, ProjectDocument};
    use serde_json::json;

    use crate::Session;
    use crate::cmd::Caller;

    use super::*;

    fn find_spec(id: &str) -> &'static CommandSpec {
        specs().iter().find(|spec| spec.id == id).unwrap()
    }

    fn new_session(title: &str) -> Session {
        Session::from_document(ProjectDocument::new(title).unwrap())
    }

    fn run_spec(session: &mut Session, spec: &CommandSpec, caller: Caller, params: Value) -> Result<CommandOutcome, EngineError> {
        let cancel = AtomicBool::new(false);
        session.execute_registered_spec(spec, params, caller, &cancel)
    }

    #[test]
    fn undo_and_redo_stay_enabled_and_noop_cleanly_on_empty_stacks() {
        let mut session = new_session("Empty Edit Stack");
        let undo_spec = find_spec("edit.undo");
        let redo_spec = find_spec("edit.redo");

        assert!((undo_spec.enabled)(&session));
        assert!((redo_spec.enabled)(&session));

        let undo_out = run_spec(&mut session, undo_spec, Caller::Mcp, json!({})).unwrap();
        assert!(!undo_out.mutated);
        assert_eq!(undo_out.payload["applied"], json!(false));

        let redo_out = run_spec(&mut session, redo_spec, Caller::Mcp, json!({})).unwrap();
        assert!(!redo_out.mutated);
        assert_eq!(redo_out.payload["applied"], json!(false));
    }

    #[test]
    fn undo_and_redo_round_trip_mutating_command_and_reject_bad_params() {
        let mut session = new_session("Initial Title");
        let new_spec = crate::cmd::project::specs().iter().find(|s| s.id == "project.new").unwrap();
        let undo_spec = find_spec("edit.undo");
        let redo_spec = find_spec("edit.redo");

        run_spec(&mut session, new_spec, Caller::Ui, json!({ "title": "Updated Title" })).unwrap();
        assert_eq!(session.document().title, "Updated Title");

        let undo_out = run_spec(&mut session, undo_spec, Caller::Ui, json!({})).unwrap();
        assert!(undo_out.mutated);
        assert_eq!(undo_out.payload["applied"], json!(true));
        assert_eq!(session.document().title, "Initial Title");

        let redo_out = run_spec(&mut session, redo_spec, Caller::Ui, json!({})).unwrap();
        assert!(redo_out.mutated);
        assert_eq!(redo_out.payload["applied"], json!(true));
        assert_eq!(session.document().title, "Updated Title");

        let bad_err = run_spec(&mut session, undo_spec, Caller::Mcp, json!(null)).unwrap_err();
        assert_eq!(bad_err.code, ErrorCode::BadInput);
        let extra_err = run_spec(&mut session, redo_spec, Caller::Mcp, json!({ "unexpected": 1 })).unwrap_err();
        assert_eq!(extra_err.code, ErrorCode::BadInput);
    }
}
