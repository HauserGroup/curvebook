//! Shared host command routing, read-only command allowlists, contextual parameter synthesis,
//! active sheet resolution, preference fingerprinting, and modal commit dispatch used by both
//! desktop (`apps/curvebook`) and browser (`apps/curvebook-web`) hosts.

use curvebook_doc::{ProjectDocument, SheetId, SheetSection, TableArchetype};
use curvebook_engine::cmd;
use curvebook_palette::CanvasPreviewFilter;
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ExportFigureFormat};
use crate::gui_actions::dispatch as dispatch_gui_action;
use crate::i18n::{BETA_ACTIVE_LOCALES, normalize_locale};
use crate::interaction::{UiEventOutcome, clear_modal_text_buffers, sync_edit_params_text_buffers, sync_format_axes_text_buffers};
use crate::power_wizard::PowerWizardStage;
use crate::theme::UiThemeMode;
use crate::views::import_paste::commit_import_paste_dialog;

/// Maximum recursive nesting depth for `command.run` / `engine.execute` / `command.batch` read-only inspection.
pub const MAX_READ_ONLY_NESTING_DEPTH: usize = 8;

/// Snapshot of user-persisted UI preferences used to detect mutations across control/UI commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPrefFingerprint {
    pub locale: String,
    pub theme_mode: UiThemeMode,
    pub ui_scale_bits: u32,
    pub shortcuts_json: Option<String>,
}

/// Capture the current preference fingerprint for `app`.
#[must_use]
pub fn capture_pref_fingerprint(app: &UiApp) -> HostPrefFingerprint {
    HostPrefFingerprint {
        locale: normalize_locale(&app.locale).to_owned(),
        theme_mode: app.theme.mode,
        ui_scale_bits: app.ui_scale().to_bits(),
        shortcuts_json: app.shortcuts.export_overrides_json().ok(),
    }
}

/// Cycle to the next theme mode token (`light` -> `dark` -> `high_contrast` -> `light`).
#[must_use]
pub const fn next_theme_mode_name(current: UiThemeMode) -> &'static str {
    match current {
        UiThemeMode::Light => "dark",
        UiThemeMode::Dark => "high_contrast",
        UiThemeMode::HighContrast => "light",
    }
}

/// Cycle to the next CVD/grayscale canvas preview filter token.
#[must_use]
pub const fn next_preview_filter_name(current: CanvasPreviewFilter) -> &'static str {
    match current {
        CanvasPreviewFilter::Normal => "deuteranopia",
        CanvasPreviewFilter::Deuteranopia => "protanopia",
        CanvasPreviewFilter::Protanopia => "tritanopia",
        CanvasPreviewFilter::Tritanopia => "grayscale",
        CanvasPreviewFilter::GrayscalePrint => "normal",
    }
}

/// Cycle to the next Beta active GUI locale code.
#[must_use]
pub fn next_locale_code(current: &str) -> &'static str {
    if BETA_ACTIVE_LOCALES.is_empty() {
        return "en";
    }
    let idx = BETA_ACTIVE_LOCALES.iter().position(|&loc| loc.eq_ignore_ascii_case(current)).unwrap_or(0);
    BETA_ACTIVE_LOCALES.get((idx + 1) % BETA_ACTIVE_LOCALES.len()).copied().unwrap_or("en")
}

/// Split an optional `:variant` suffix on known parameterized chrome commands.
#[must_use]
pub fn split_variant_command(command_id: &str) -> (&str, Option<&str>) {
    let Some((prefix, suffix)) = command_id.split_once(':') else {
        return (command_id, None);
    };
    if matches!(
        prefix,
        "ui.theme.set" | "ui.locale.set" | "ui.previewFilter.set" | "ui.canvas.applyPreset" | "graph.applyCanvasPreset" | "layout.applyCanvasPreset"
    ) && !suffix.trim().is_empty()
    {
        (prefix, Some(suffix.trim()))
    } else {
        (command_id, None)
    }
}

/// Explicit allowlist of front-end UI commands that do not mutate the project document (`FA-SEC20-02`).
#[must_use]
pub fn is_read_only_ui_command(command_id: &str) -> bool {
    let base = command_id.split(':').next().unwrap_or(command_id);
    matches!(
        base,
        "app.about"
            | "app.feedback"
            | "ui.previewFilter.set"
            | "ui.rulers.toggle"
            | "ui.theme.set"
            | "ui.locale.set"
            | "ui.palette.open"
            | "ui.about.open"
            | "ui.sheet.select"
            | "ui.split.set"
            | "ui.grid.selectRange"
            | "ui.clipboard.copy"
            | "ui.export.figure"
            | "ui.print.open"
            | "file.print"
            | "ui.navigator.toggle"
            | "ui.inspector.toggle"
            | "ui.family.toggle"
            | "ui.zoom.fitSheet"
            | "ui.split.toggle"
            | "ui.gallery.toggle"
            | "ui.pane.cycle"
            | "ui.shortcut.set"
            | "ui.shortcut.reset"
            | "ui.closeWorkspace.prompt"
            | "ui.analyze.open"
            | "ui.powerWizard.open"
            | "ui.formatPoints.open"
            | "ui.changeGraphType.open"
            | "ui.clonePipeline.open"
            | "ui.graph.axes.open"
            | "ui.graph.format.open"
            | "ui.graph.pairwise.open"
            | "ui.graph.insertModelEquation"
            | "ui.graph.bubblePlot.open"
            | "ui.applyStyleFrom.open"
            | "ui.layout.composer.open"
            | "ui.layout.equalizeScaling.open"
            | "ui.graph.create.open"
            | "ui.view.rowTitles.toggle"
            | "ui.gridOverlay.toggle"
            | "ui.embeddedTable.format.open"
            | "ui.image.format.open"
            | "ui.export.figure.open"
            | "ui.import.reshape.open"
            | "ui.result.copyMethodsReport"
            | "ui.result.applyRemediation"
            | "ui.result.inspectDiagnostics"
            | "ui.canvas.preflight"
            | "ui.brush.point"
            | "ui.table.convertArchetype.open"
            | "ui.table.randomizeGroups"
            | "ui.table.randomSampleRows"
            | "ui.table.simulateDistributions"
            | "ui.table.openFormulaBar"
    )
}

/// Explicit allowlist of unconditionally read-only control-protocol inspection and vector/preview methods.
///
/// Path-capable control methods (`doc.export`, `render.pdf`) are gated on `!has_non_null_path_param`
/// inside [`is_command_read_only`].
#[must_use]
pub fn is_read_only_control_method(command_id: &str) -> bool {
    matches!(command_id, "ping" | "command.list" | "doc.inspect" | "render.svg" | "render.preview" | "ui.state" | "state.snapshot")
}

/// Whether `params` carries a non-null filesystem export path (`path`, `file_path`, `filePath`, `output_path`, or `outputPath`).
#[must_use]
pub fn has_non_null_path_param(params: &Value) -> bool {
    ["path", "file_path", "filePath", "output_path", "outputPath"].iter().any(|&key| params.get(key).is_some_and(|v| !v.is_null()))
}

/// Return the first non-null, non-empty trimmed string value across candidate keys in `obj`.
#[must_use]
pub fn first_present_trimmed_str<'a>(obj: &'a Map<String, Value>, fields: &[&str]) -> Option<&'a str> {
    for &field in fields {
        if let Some(val) = obj.get(field)
            && !val.is_null()
        {
            return val.as_str().map(str::trim).filter(|s| !s.is_empty());
        }
    }
    None
}

/// Extract nested `params` matching `control_io`'s flat/nested request conventions.
#[must_use]
pub fn extract_nested_params_like_control_io(obj: &Map<String, Value>) -> Value {
    if let Some(explicit) = obj.get("params") {
        return explicit.clone();
    }
    let mut stripped = obj.clone();
    stripped.remove("command");
    stripped.remove("command_id");
    if stripped.is_empty() { Value::Null } else { Value::Object(stripped) }
}

/// Normalize flat/top-level control-line request parameters into the nested object expected by command inspection.
#[must_use]
pub fn normalize_control_request_params(method: &str, obj: &Map<String, Value>) -> Value {
    if method == "command.run" && (obj.contains_key("command") || obj.contains_key("command_id")) {
        let mut out = Map::new();
        if let Some(cmd) = obj.get("command") {
            out.insert(String::from("command"), cmd.clone());
        }
        if let Some(cmd_id) = obj.get("command_id") {
            out.insert(String::from("command_id"), cmd_id.clone());
        }
        if let Some(inner) = obj.get("params") {
            out.insert(String::from("params"), inner.clone());
        } else {
            let mut inner = Map::new();
            for (key, value) in obj {
                if !matches!(key.as_str(), "id" | "method" | "op" | "command" | "command_id" | "params" | "token" | "auth_token") {
                    inner.insert(key.clone(), value.clone());
                }
            }
            if !inner.is_empty() {
                out.insert(String::from("params"), Value::Object(inner));
            }
        }
        return Value::Object(out);
    }

    let mut base = match obj.get("params") {
        Some(Value::Object(map)) => map.clone(),
        Some(other) if !other.is_null() => return other.clone(),
        _ => Map::new(),
    };
    for (key, value) in obj {
        if matches!(key.as_str(), "id" | "method" | "op" | "params" | "token" | "auth_token") {
            continue;
        }
        base.entry(key.clone()).or_insert_with(|| value.clone());
    }
    Value::Object(base)
}

/// Extract the inner `(command_id, params)` from a `command.run` or `engine.execute` envelope.
#[must_use]
pub fn extract_control_subcommand(command_id: &str, params: Option<&Value>) -> Option<(String, Value)> {
    let obj = params?.as_object()?;
    match command_id {
        "command.run" => {
            let inner = first_present_trimmed_str(obj, &["command", "command_id"])?.to_owned();
            Some((inner, extract_nested_params_like_control_io(obj)))
        }
        "engine.execute" => {
            let inner = first_present_trimmed_str(obj, &["command_id", "command"])?.to_owned();
            let nested = obj.get("params").cloned().unwrap_or(Value::Null);
            Some((inner, nested))
        }
        _ => None,
    }
}

/// Extract `(command_id, params)` from one item of a `command.batch` array.
#[must_use]
pub fn extract_batch_item_command(entry: &Value) -> Option<(String, Value)> {
    let obj = entry.as_object()?;
    let inner = first_present_trimmed_str(obj, &["command", "command_id"])?.to_owned();
    Some((inner, extract_nested_params_like_control_io(obj)))
}

/// Return `true` if `command_id` (directly or wrapped in `command.run` / `engine.execute` / `command.batch`)
/// replaces the active workspace project (`project.new`, `file.new`, `project.open`, `project.instantiateTemplate`).
#[must_use]
pub fn command_replaces_project(command_id: &str, params: &Value) -> bool {
    command_replaces_project_depth(command_id, params, 0)
}

fn command_replaces_project_depth(command_id: &str, params: &Value, depth: usize) -> bool {
    if depth > MAX_READ_ONLY_NESTING_DEPTH {
        return false;
    }
    let trimmed = command_id.trim();
    match trimmed {
        "project.new" | "file.new" | "project.open" | "project.instantiateTemplate" => true,
        "command.run" | "engine.execute" => extract_control_subcommand(trimmed, Some(params))
            .is_some_and(|(inner, inner_params)| command_replaces_project_depth(&inner, &inner_params, depth + 1)),
        "command.batch" => params.get("commands").and_then(Value::as_array).is_some_and(|cmds| {
            cmds.iter().any(|entry| {
                extract_batch_item_command(entry)
                    .is_some_and(|(inner, inner_params)| command_replaces_project_depth(&inner, &inner_params, depth + 1))
            })
        }),
        _ => false,
    }
}

/// Strict read-only classification covering engine commands, UI commands, control methods,
/// path-bearing exports, and recursively unwrapped `command.run` / `engine.execute` / `command.batch` (`FA-SEC20-02`).
#[must_use]
pub fn is_command_read_only(command_id: &str, params: Option<&Value>) -> bool {
    is_command_read_only_depth(command_id, params, 0)
}

fn is_command_read_only_depth(command_id: &str, params: Option<&Value>, depth: usize) -> bool {
    if depth > MAX_READ_ONLY_NESTING_DEPTH {
        return false;
    }
    let command_id = command_id.trim();
    let has_path = params.is_some_and(has_non_null_path_param);
    if matches!(
        command_id,
        "analysis.exportJson"
            | "analysis.exportCsv"
            | "result.generateReport"
            | "table.exportCsv"
            | "export.svg"
            | "graph.exportSvg"
            | "export.pdf"
            | "graph.exportPdf"
            | "export.png"
            | "graph.exportPng"
            | "export.tiff"
            | "doc.export"
            | "render.pdf"
    ) {
        return !has_path;
    }
    if let Some(spec) = cmd::registry().get(command_id) {
        return spec.effects.read_only_hint();
    }
    if matches!(command_id, "command.run" | "engine.execute") {
        return extract_control_subcommand(command_id, params)
            .is_some_and(|(inner, inner_params)| is_command_read_only_depth(&inner, Some(&inner_params), depth + 1));
    }
    if command_id == "command.batch" {
        return params.and_then(|p| p.get("commands")).and_then(Value::as_array).is_some_and(|cmds| {
            cmds.iter().all(|entry| extract_batch_item_command(entry).is_some_and(|(id, p)| is_command_read_only_depth(&id, Some(&p), depth + 1)))
        });
    }
    let base = command_id.split(':').next().unwrap_or(command_id);
    if matches!(
        base,
        "ui.export.csv"
            | "table.exportCsv"
            | "export.svg"
            | "graph.exportSvg"
            | "export.pdf"
            | "graph.exportPdf"
            | "export.png"
            | "graph.exportPng"
            | "export.tiff"
    ) {
        return !has_path;
    }
    is_read_only_ui_command(base) || is_read_only_control_method(command_id)
}

/// Resolve any active or first sheet ID in `doc`.
#[must_use]
pub fn resolve_any_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id
        .clone()
        .or_else(|| doc.data_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.graph_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.result_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.layout_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.info_sheets.first().map(|s| s.id.clone()))
}

/// Resolve the active sheet ID only when it is a `DataSheet`.
#[must_use]
pub fn active_data_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()).cloned()
}

/// Resolve the active `DataSheet` ID or fall back to the first `DataSheet` in `doc`.
#[must_use]
pub fn first_data_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    active_data_sheet_id(doc).or_else(|| doc.data_sheets.first().map(|s| s.id.clone()))
}

/// Resolve the most relevant `DataSheet` ID, following the active graph/result's source table before falling back to the first `DataSheet`.
#[must_use]
pub fn resolve_data_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = active_data_sheet_id(doc) {
        return Some(active);
    }
    if let Some(active) = doc.active_sheet_id.as_ref() {
        if let Some(graph) = doc.graph_sheet(active) {
            let src = &graph.spec.primary_source_sheet_id;
            if doc.data_sheet(src).is_some() {
                return Some(src.clone());
            }
        }
        if let Some(res) = doc.result_sheet(active) {
            let src = &res.analysis.source_sheet_id;
            if doc.data_sheet(src).is_some() {
                return Some(src.clone());
            }
        }
    }
    doc.data_sheets.first().map(|s| s.id.clone())
}

/// Resolve an XY/Survival `DataSheet` ID, falling back to any `DataSheet`.
#[must_use]
pub fn resolve_xy_or_data_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref()
        && let Some(ds) = doc.data_sheet(active)
        && matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_))
    {
        return Some(ds.id.clone());
    }
    doc.data_sheets
        .iter()
        .find(|ds| matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_)))
        .map(|ds| ds.id.clone())
        .or_else(|| resolve_data_sheet_id(doc))
}

/// Resolve the active or first `MultipleVariables` `DataSheet` ID.
#[must_use]
pub fn resolve_mv_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref()
        && let Some(ds) = doc.data_sheet(active)
        && matches!(ds.archetype, TableArchetype::MultipleVariables(_))
    {
        return Some(ds.id.clone());
    }
    doc.data_sheets.iter().find(|ds| matches!(ds.archetype, TableArchetype::MultipleVariables(_))).map(|ds| ds.id.clone())
}

/// Resolve the active or first `ResultSheet` ID.
#[must_use]
pub fn resolve_result_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()).cloned().or_else(|| doc.result_sheets.first().map(|r| r.id.clone()))
}

/// Resolve the active or first `GraphSheet` ID.
#[must_use]
pub fn resolve_graph_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some()).cloned().or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
}

/// Resolve the active `GraphSheet`, first uninitialized `GraphSheet`, or first `GraphSheet`.
#[must_use]
pub fn resolve_uninit_or_graph_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id
        .as_ref()
        .filter(|id| doc.graph_sheet(id).is_some())
        .cloned()
        .or_else(|| doc.graph_sheets.iter().find(|g| !g.initialized).map(|g| g.id.clone()))
        .or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
}

/// Resolve the active or first `LayoutSheet` ID.
#[must_use]
pub fn resolve_layout_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.layout_sheet(id).is_some()).cloned().or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
}

/// Resolve the active `GraphSheet`/`LayoutSheet`, or fall back to the first graph or layout sheet.
#[must_use]
pub fn resolve_renderable_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref()
        && matches!(doc.sheet_section(active), Some(SheetSection::Graphs | SheetSection::Layouts))
    {
        return Some(active.clone());
    }
    doc.graph_sheets.first().map(|g| g.id.clone()).or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
}

/// Resolve the active `InfoSheet`, the active `DataSheet`'s paired `InfoSheet`, or the first `InfoSheet`.
#[must_use]
pub fn resolve_info_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref() {
        if doc.info_sheet(active).is_some() {
            return Some(active.clone());
        }
        if let Some(ds) = doc.data_sheet(active)
            && let Some(linked) = &ds.info_sheet_id
            && doc.info_sheet(linked).is_some()
        {
            return Some(linked.clone());
        }
    }
    doc.info_sheets.first().map(|i| i.id.clone())
}

/// Wrap an optional [`SheetId`] into `{ "sheet_id": "..." }` or `{}`.
#[must_use]
pub fn sheet_id_obj_or_empty(sheet_id: Option<SheetId>) -> Value {
    match sheet_id {
        Some(id) => json!({ "sheet_id": id.as_str() }),
        None => json!({}),
    }
}

/// Map interactive chrome/menu engine command IDs to their corresponding UI modal opener command.
#[must_use]
pub fn dialog_ui_command_for_engine_cmd(cmd_id: &str) -> Option<&'static str> {
    match cmd_id {
        "toolbar.axes" | "graph.setAxes" => Some("ui.graph.axes.open"),
        "graph.setKind" => Some("ui.changeGraphType.open"),
        "graph.setSpec" | "graph.setSeries" | "graph.setAdditionalTicks" | "graph.setRegressionOverlay" | "graph.applyStyleFrom" => {
            Some("ui.graph.format.open")
        }
        "graph.formatSelectedPoints" => Some("ui.formatPoints.open"),
        "graph.addComparisonBrackets" | "graph.setCompactLetterDisplay" => Some("ui.graph.pairwise.open"),
        "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => Some("ui.canvas.applyPreset"),
        "analysis.run" | "stats.run" | "fit.run" | "transform.run" | "analysis.add" | "stats.pvalueStack" | "analysis.retargetData" => {
            Some("ui.analyze.open")
        }
        "table.convertArchetype" => Some("ui.table.convertArchetype.open"),
        "table.excludeCellsWithReason" => Some("ui.grid.excludeWithReason"),
        "table.randomizeGroups" => Some("ui.table.randomizeGroups"),
        "table.randomSampleRows" => Some("ui.table.randomSampleRows"),
        "table.simulateDistributions" => Some("ui.table.simulateDistributions"),
        "table.setCalculatedVariable" => Some("ui.table.openFormulaBar"),
        "table.pasteLink" => Some("ui.import.reshape.open"),
        "sheet.clonePipeline" => Some("ui.clonePipeline.open"),
        "layout.setSpec" | "layout.addPanel" => Some("ui.layout.composer.open"),
        _ => None,
    }
}

/// Build contextual default parameters for menu/toolbar/palette commands (always returns a JSON object, never `Value::Null`).
#[must_use]
pub fn contextual_command_params(app: &UiApp, command_id: &str) -> Value {
    let doc = app.document();
    let active_sheet = doc.active_sheet_id.as_ref().map(SheetId::as_str);
    let active_data = first_data_sheet_id(doc);
    let active_graph = resolve_graph_sheet_id(doc);
    let active_layout = resolve_layout_sheet_id(doc);
    let active_render = resolve_renderable_sheet_id(doc);
    let active_result = resolve_result_sheet_id(doc);

    let data_str = active_data.as_ref().map(SheetId::as_str);
    let graph_str = active_graph.as_ref().map(SheetId::as_str);
    let layout_str = active_layout.as_ref().map(SheetId::as_str);
    let render_str = active_render.as_ref().map(SheetId::as_str);
    let result_str = active_result.as_ref().map(SheetId::as_str);

    match command_id {
        "sheet.createData" => json!({ "kind": "xy", "title": "Data Table", "auto_family": true }),
        "sheet.rename" => json!({ "sheet_id": active_sheet, "title": "Renamed Sheet" }),
        "sheet.duplicate" | "sheet.duplicateFamily" | "sheet.clonePipeline" | "sheet.delete" | "sheet.freeze" | "sheet.unfreeze" | "sheet.select" => {
            json!({ "sheet_id": active_sheet })
        }
        "sheet.extractResultTab" => json!({ "result_sheet_id": result_str }),
        "graph.create" => json!({ "data_sheet_id": data_str, "title": "Graph" }),
        "graph.initialize" | "graph.preflightCheck" | "graph.setAxes" | "graph.setKind" | "graph.setSpec" | "graph.setSeries" => {
            json!({ "sheet_id": graph_str })
        }
        "graph.addAnnotation" => json!({ "sheet_id": graph_str, "text": "Annotation", "x_pt": 72.0, "y_pt": 72.0 }),
        "graph.applyStyleFrom" => json!({
            "target_sheet_id": graph_str,
            "source_sheet_id": doc.graph_sheets.first().map(|s| s.id.as_str()),
        }),
        "graph.applyCanvasPreset" | "ui.canvas.applyPreset" => json!({ "sheet_id": render_str, "preset": "single_column_88mm" }),
        "layout.create" => json!({ "title": "Layout" }),
        "layout.alignOrigins" => json!({ "sheet_id": layout_str, "align": true, "normalize_bounds": true }),
        "layout.applyCanvasPreset" => json!({ "sheet_id": layout_str, "preset": "double_column_180mm" }),
        "layout.addPanel" | "layout.setSpec" => json!({ "sheet_id": layout_str }),
        "table.clearCells" | "table.excludeCells" => {
            let coords = app.grid.selected_coords.iter().map(|c| json!({ "row": c.row, "col": c.col, "subcol": c.subcol })).collect::<Vec<_>>();
            if coords.is_empty() {
                if let Some(c) = app.grid.tool.active {
                    json!({ "sheet_id": data_str, "cells": [{ "row": c.row, "col": c.col, "subcol": c.subcol }] })
                } else {
                    json!({ "sheet_id": data_str })
                }
            } else {
                json!({ "sheet_id": data_str, "cells": coords })
            }
        }
        "table.exclusionAuditReport" | "table.exportCsv" => json!({ "sheet_id": data_str }),
        "export.svg" | "graph.exportSvg" | "export.pdf" | "graph.exportPdf" | "export.png" | "graph.exportPng" | "export.tiff"
        | "graph.exportEmf" | "layout.exportEmf" | "ui.print.open" | "file.print" => json!({ "sheet_id": render_str }),
        "analysis.run" | "stats.run" | "fit.run" | "transform.run" | "analysis.add" => {
            json!({ "source_sheet_id": data_str, "kind": "descriptive_stats" })
        }
        "analysis.exportJson"
        | "analysis.exportCsv"
        | "ui.result.copyMethodsReport"
        | "ui.result.inspectDiagnostics"
        | "ui.result.appendClusterColumn" => json!({ "sheet_id": result_str }),
        "result.generateReport" => json!({ "result_sheet_id": result_str, "format": "markdown" }),
        "project.createCheckpoint" => json!({ "name": "Checkpoint" }),
        "ui.theme.set" => json!({ "mode": next_theme_mode_name(app.theme.mode) }),
        "ui.locale.set" => json!({ "locale": if app.locale == "en" { "de" } else { "en" } }),
        "ui.previewFilter.set" => json!({ "filter": next_preview_filter_name(app.preview_filter) }),
        "ui.brush.point" => {
            if let (Some(d), Some(c)) = (data_str, app.grid.tool.active) {
                json!({ "sheet_id": d, "row": c.row, "col": c.col, "subcol": c.subcol })
            } else {
                json!({ "clear": true })
            }
        }
        _ => json!({}),
    }
}

fn finish_modal_commit(app: &mut UiApp, action: &str, command_id: Option<String>, command_result: Option<Value>) -> UiEventOutcome {
    app.canvas_dialogs.close_dialog();
    clear_modal_text_buffers(app);
    app.focus_widget_id = None;
    UiEventOutcome { handled: true, action: action.to_owned(), command_id, command_result }
}

fn commit_modal_intent(app: &mut UiApp, command_id: &str, params: Value, action: &str) -> Result<UiEventOutcome, String> {
    let out = app.execute_command(command_id, params).map_err(|e| e.to_string())?;
    Ok(finish_modal_commit(app, action, Some(command_id.to_owned()), Some(out.payload)))
}

fn commit_modal_intent_batch<I>(app: &mut UiApp, intents: I, action: &str) -> Result<UiEventOutcome, String>
where
    I: IntoIterator<Item = (String, Value)>,
{
    let mut last_cmd = None;
    let mut last_payload = None;
    for (command_id, params) in intents {
        let out = app.execute_command(&command_id, params).map_err(|e| e.to_string())?;
        last_cmd = Some(command_id);
        last_payload = Some(out.payload);
    }
    Ok(finish_modal_commit(app, action, last_cmd, last_payload))
}

/// Commit the active modal dialog cleanly via `UiApp` state machines (never via `ui.answer_dialog`).
///
/// The `on_palette_command` callback handles executing the selected `CommandPalette` item ID using
/// the host's command router.
///
/// # Errors
/// Returns a descriptive error message when validation or engine execution fails.
pub fn commit_active_modal_dialog<F>(app: &mut UiApp, on_palette_command: F) -> Result<UiEventOutcome, String>
where
    F: FnOnce(&mut UiApp, &str) -> Result<UiEventOutcome, String>,
{
    let dialog = app.canvas_dialogs.active_dialog.clone();
    match dialog {
        ActiveDialog::None => Ok(UiEventOutcome { handled: false, action: String::from("dialog.none"), ..Default::default() }),
        ActiveDialog::AnalyzeSheet { .. } => commit_analyze_dialog(app),
        ActiveDialog::DuplicateAnalysisPrompt { .. } => {
            let intent = app
                .canvas_dialogs
                .analyze
                .build_create_new_intent()
                .ok_or_else(|| String::from("no selected analysis for duplicate confirmation"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.duplicate_created_new")
        }
        ActiveDialog::EditAnalysisParameters { .. } => commit_edit_params_dialog(app),
        ActiveDialog::CommandPalette => {
            let hits = app.canvas_dialogs.command_palette.matches_for_session(app.session());
            let selected = hits.get(app.canvas_dialogs.command_palette.selected_index).or_else(|| hits.first()).cloned();
            app.canvas_dialogs.close_dialog();
            app.focus_widget_id = None;
            app.interaction.text_field_buffers.remove("palette.query");
            clear_modal_text_buffers(app);
            match selected {
                Some(hit) => on_palette_command(app, hit.id),
                None => Ok(UiEventOutcome { handled: true, action: String::from("palette.no_match"), ..Default::default() }),
            }
        }
        ActiveDialog::ExcludeWithReason { .. } => {
            let intent = app.canvas_dialogs.exclude_with_reason.commit_intent()?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.exclude_committed")
        }
        ActiveDialog::ExportFigure { sheet_id } => commit_export_figure_dialog(app, &sheet_id),
        ActiveDialog::ImportReshape => {
            let payload = commit_import_paste_dialog(app)?;
            Ok(finish_modal_commit(app, "dialog.import_reshape_committed", Some(String::from("ui.importReshape.commit")), Some(payload)))
        }
        ActiveDialog::Print { .. } => {
            let submit = app.canvas_dialogs.print.submit_to_os;
            if submit && app.is_gui_only() {
                return Err(String::from("submitting print jobs directly to the OS queue is blocked under --gui-only"));
            }
            let out = app.execute_print_dialog_spool(submit)?;
            Ok(finish_modal_commit(
                app,
                "dialog.print_committed",
                Some(String::from("ui.print.open")),
                Some(json!({ "page_count": out.page_count, "submitted_to_os": submit })),
            ))
        }
        ActiveDialog::RandomizeGroups => {
            let intent =
                app.canvas_dialogs.randomize_groups.commit_intent().ok_or_else(|| String::from("no active sheet bound for Randomize Groups"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.randomize_groups_committed")
        }
        ActiveDialog::RandomSampleRows { .. } => {
            let intent = app
                .canvas_dialogs
                .random_sample_rows
                .commit_intent()
                .ok_or_else(|| String::from("no source sheet selected for Random Sample Rows"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.random_sample_rows_committed")
        }
        ActiveDialog::SimulateDistributions => {
            let intent = app
                .canvas_dialogs
                .simulate_distributions
                .commit_intent()
                .ok_or_else(|| String::from("invalid Simulate Distributions configuration"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.simulate_distributions_committed")
        }
        ActiveDialog::ConstantHookPicker { .. } => {
            let intent = app.canvas_dialogs.hook_picker.commit_intent().ok_or_else(|| String::from("no constant hook option selected"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.constant_hook_committed")
        }
        ActiveDialog::Checkpoints => commit_checkpoints_dialog(app),
        ActiveDialog::PowerAndSampleSize => commit_power_wizard_dialog(app),
        ActiveDialog::ConvertArchetype { .. } => {
            let out = app.commit_convert_archetype()?;
            app.focus_widget_id = None;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("dialog.convert_archetype"),
                command_id: Some(out.command_id),
                command_result: Some(out.payload),
            })
        }
        ActiveDialog::ChangeGraphType { .. } => {
            let intents = app.canvas_dialogs.change_graph_type.commit_intents(app.document());
            commit_modal_intent_batch(app, intents.into_iter().map(|i| (i.command_id, i.params)), "dialog.change_graph_type_committed")
        }
        ActiveDialog::FormatAxes { .. } => {
            sync_format_axes_text_buffers(app);
            let intent = app.canvas_dialogs.format_axes.commit_intent().ok_or_else(|| String::from("invalid Format Axes settings"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.format_axes_committed")
        }
        ActiveDialog::FormatGraph { .. } => {
            let intents = app.canvas_dialogs.format_graph.commit_intents();
            commit_modal_intent_batch(app, intents.into_iter().map(|i| (i.command_id, i.params)), "dialog.format_graph_committed")
        }
        ActiveDialog::PairwiseComparisons { .. } => {
            let intents = app.canvas_dialogs.pairwise.commit_intents();
            commit_modal_intent_batch(app, intents.into_iter().map(|i| (i.command_id, i.params)), "dialog.pairwise_committed")
        }
        ActiveDialog::BubblePlotInspector { .. } => {
            let intent = app.canvas_dialogs.bubble_plot.commit_intent().ok_or_else(|| String::from("no bubble plot graph target selected"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.bubble_plot_committed")
        }
        ActiveDialog::InsertModelEquation { .. } => {
            let doc = app.document().clone();
            let intent = app
                .canvas_dialogs
                .insert_model_equation
                .commit_intent(Some(&doc))
                .ok_or_else(|| String::from("no model equation result sheet selected"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.insert_model_equation_committed")
        }
        ActiveDialog::ApplyStyleFrom { .. } => {
            let intent =
                app.canvas_dialogs.apply_style.commit_intent().ok_or_else(|| String::from("no donor graph selected for Apply Style From"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.apply_style_committed")
        }
        ActiveDialog::ChangeDataAnalyzed { .. } => {
            let intent =
                app.canvas_dialogs.change_data.commit_intent().ok_or_else(|| String::from("no compatible replacement data sheet selected"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.change_data_committed")
        }
        ActiveDialog::FormatSelectedPoints { .. } => commit_format_points_dialog(app),
        ActiveDialog::PageLayoutComposer { .. } => {
            let intents = app.canvas_dialogs.page_layout.commit_intents();
            commit_modal_intent_batch(app, intents.into_iter().map(|i| (i.command_id, i.params)), "dialog.page_layout_committed")
        }
        ActiveDialog::CreateGraph { .. } => {
            let intent = app.canvas_dialogs.create_graph.commit_intent().ok_or_else(|| String::from("no source data sheet selected"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.create_graph_committed")
        }
        ActiveDialog::EqualizeScaling { .. } => {
            let intent =
                app.canvas_dialogs.equalize_scaling.commit_intent().ok_or_else(|| String::from("no live graph panels available to equalize"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.equalize_scaling_committed")
        }
        ActiveDialog::FormatEmbeddedTable { .. } => {
            let doc = app.document().clone();
            let intent = app.canvas_dialogs.format_embedded_table.commit_intent(&doc).ok_or_else(|| String::from("no embedded table selected"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.format_embedded_table_committed")
        }
        ActiveDialog::FormatImage { .. } => {
            let intent = app.canvas_dialogs.format_image.commit_intent().ok_or_else(|| String::from("no embedded image selected"))?;
            commit_modal_intent(app, &intent.command_id, intent.params, "dialog.format_image_committed")
        }
        _ => Ok(finish_modal_commit(app, "dialog.closed", None, None)),
    }
}

fn commit_analyze_dialog(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    let intent = match app.canvas_dialogs.analyze.try_build_run_intent(app.document()) {
        Ok(intent) => intent,
        Err(existing_ids) => {
            let entry_id = app.canvas_dialogs.analyze.selected_entry_id.clone().unwrap_or_default();
            let Some(src) = app.canvas_dialogs.analyze.source_sheet_id.clone().or_else(|| app.document().data_sheets.first().map(|ds| ds.id.clone()))
            else {
                return Err(String::from("no source data sheet selected"));
            };
            app.canvas_dialogs.active_dialog =
                ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id: src, existing_result_ids: existing_ids, pending_entry_id: entry_id };
            return Ok(UiEventOutcome { handled: true, action: String::from("dialog.duplicate_prompt"), ..Default::default() });
        }
    };
    commit_modal_intent(app, &intent.command_id, intent.params, "dialog.analyze_committed")
}

fn commit_edit_params_dialog(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    sync_edit_params_text_buffers(app);
    let doc = app.document().clone();
    let intents = app.canvas_dialogs.edit_params.commit_intents(Some(&doc));
    let mut iter = intents.into_iter();
    let primary = iter.next().ok_or_else(|| String::from("no draft parameters ready to commit"))?;
    let primary_cmd = primary.command_id.clone();
    let out = app.execute_command(&primary.command_id, primary.params).map_err(|e| e.to_string())?;
    for extra in iter {
        let _ = app.execute_command(&extra.command_id, extra.params).map_err(|e| e.to_string())?;
    }
    Ok(finish_modal_commit(app, "dialog.edit_params_committed", Some(primary_cmd), Some(out.payload)))
}

fn commit_export_figure_dialog(app: &mut UiApp, sheet_id: &SheetId) -> Result<UiEventOutcome, String> {
    let mut intent = app.canvas_dialogs.export_figure.commit_intent(sheet_id);
    let export_opts = &app.view_shards.dialogs_graph_misc;
    if matches!(app.canvas_dialogs.export_figure.format, ExportFigureFormat::Pdf)
        && let Some(obj) = intent.params.as_object_mut()
    {
        let pdf_cs = if export_opts.export_color_space == "CMYK-Proof" { "cmyk" } else { "srgb" };
        obj.insert(String::from("color_space"), json!(pdf_cs));
    }
    let opts_snapshot = json!({
        "transparent_bg": export_opts.export_transparent_bg,
        "color_space": export_opts.export_color_space,
        "unit": export_opts.export_unit,
    });
    let out = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
    let mut payload = out.payload;
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(String::from("export_options"), opts_snapshot);
    }
    Ok(finish_modal_commit(app, "dialog.export_committed", Some(intent.command_id), Some(payload)))
}

fn commit_checkpoints_dialog(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    let name = app.canvas_dialogs.checkpoints.name_draft.trim().to_owned();
    if name.is_empty() {
        return Err(String::from("Enter a checkpoint name before saving."));
    }
    let mut params = Map::new();
    params.insert(String::from("name"), Value::String(name));
    let note = app.canvas_dialogs.checkpoints.note_draft.trim().to_owned();
    if !note.is_empty() {
        params.insert(String::from("note"), Value::String(note));
    }
    let out = app.execute_command("project.createCheckpoint", Value::Object(params)).map_err(|e| e.to_string())?;
    let doc = app.document().clone();
    app.canvas_dialogs.checkpoints.name_draft = format!("Checkpoint {}", doc.checkpoints.len() + 1);
    app.canvas_dialogs.checkpoints.note_draft.clear();
    app.interaction.text_field_buffers.remove("checkpoints.name");
    app.interaction.text_field_buffers.remove("checkpoints.note");
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("dialog.checkpoint_created"),
        command_id: Some(String::from("project.createCheckpoint")),
        command_result: Some(out.payload),
    })
}

fn commit_power_wizard_dialog(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    if app.canvas_dialogs.power_wizard.stage != PowerWizardStage::Results {
        let next = app.canvas_dialogs.power_wizard.advance()?;
        return Ok(UiEventOutcome { handled: true, action: format!("dialog.power_advance:{}", next.label()), ..Default::default() });
    }
    let payload = dispatch_gui_action(app, "ui.answer_dialog", json!({ "action": "create" })).map_err(|e| e.to_string())?;
    app.focus_widget_id = None;
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("dialog.power_created"),
        command_id: Some(String::from("ui.powerWizard.create")),
        command_result: Some(payload),
    })
}

fn commit_format_points_dialog(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    if app.canvas_dialogs.format_points.selected_points.is_empty()
        && let Some(active_ds_id) = app.brushed_points.first().map(|p| p.sheet_id.clone())
        && let Some(ds) = app.document().data_sheet(&active_ds_id).cloned()
    {
        let targets = app.grid.selected_graph_point_targets(&ds, usize::MAX);
        app.canvas_dialogs.format_points.set_selected_targets(targets);
    }
    let mut pairs: Vec<(String, Value)> = Vec::new();
    if let Some(intent) = app.canvas_dialogs.format_points.build_commit_intent() {
        pairs.push((intent.command_id, intent.params));
    }
    let scene = app.canvas_dialogs.format_points.graph_sheet_id.as_ref().and_then(|gid| app.session().render_sheet_scene(gid).ok());
    let doc = app.document().clone();
    for intent in app.canvas_dialogs.format_points.build_row_label_intents_for_scene(&doc, scene.as_ref()) {
        pairs.push((intent.command_id, intent.params));
    }
    commit_modal_intent_batch(app, pairs, "dialog.format_points_committed")
}
