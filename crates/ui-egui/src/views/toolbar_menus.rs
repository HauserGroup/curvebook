//! Menu bar, contextual icon-and-label toolbar, dropdown popups, right-click context menus,
//! and status bar view builders (`R10.6`).

use curvebook_doc::{DataSheet, ProjectDocument, ResultSheet, SheetCellRange, SheetId, TableArchetype};
use curvebook_palette::Rgba8;
use curvebook_tools::GridCellCoord;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ActiveInspector, ResultSheetViewState};
use crate::grid_view::data_set_subcolumn_count;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta, scrollbar_thickness_px};
use crate::layout::widgets::{FittedTextSpec, emit_scrollbars_for_meta, maybe_paint_focus_ring, paint_text_fitted, widget_state};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::menus::command_tooltip;
use crate::modal_predicates::ControlEnablement;
use crate::os_integration::ClipboardBundle;
use crate::text_bridge::{TextMeasurer, UiTextStyle};
use crate::theme::UiThemeMode;

use super::a11y::labeled_meta;

/// Transient view state owned by the toolbar / menus / status bar shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolbarMenusViewState {
    pub overflow_open: bool,
    pub active_context_menu_id: Option<String>,
    #[serde(default)]
    pub context_menu_anchor_px: Option<[i32; 2]>,
    #[serde(default)]
    pub context_sheet_id: Option<String>,
    #[serde(default)]
    pub text_bold: bool,
    #[serde(default)]
    pub text_italic: bool,
    #[serde(default)]
    pub text_font_size_pt: u16,
    #[serde(default)]
    pub last_toolbar_notice: Option<String>,
}

/// Active document context governing adaptive toolbar groups and control enablement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveToolbarContext {
    EmptyWorkspace,
    DataSheet,
    InfoSheet,
    ResultSheet,
    GraphSheet,
    LayoutSheet,
}

impl ActiveToolbarContext {
    /// Human-readable short badge label shown in status/chrome summaries.
    #[must_use]
    pub const fn badge(self) -> &'static str {
        match self {
            Self::EmptyWorkspace => "Workspace",
            Self::DataSheet => "Data",
            Self::InfoSheet => "Info",
            Self::ResultSheet => "Result",
            Self::GraphSheet => "Graph",
            Self::LayoutSheet => "Layout",
        }
    }
}

/// Detect the active sheet context from `doc.active_sheet_id`.
#[must_use]
pub fn active_toolbar_context(doc: &ProjectDocument) -> ActiveToolbarContext {
    let Some(id) = doc.active_sheet_id.as_ref() else {
        return ActiveToolbarContext::EmptyWorkspace;
    };
    if doc.data_sheet(id).is_some() {
        ActiveToolbarContext::DataSheet
    } else if doc.info_sheet(id).is_some() {
        ActiveToolbarContext::InfoSheet
    } else if doc.result_sheet(id).is_some() {
        ActiveToolbarContext::ResultSheet
    } else if doc.graph_sheet(id).is_some() {
        ActiveToolbarContext::GraphSheet
    } else if doc.layout_sheet(id).is_some() {
        ActiveToolbarContext::LayoutSheet
    } else {
        ActiveToolbarContext::EmptyWorkspace
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarIconKind {
    New,
    Open,
    Save,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Freeze,
    Analyze,
    Parameters,
    DataAnalyzed,
    Interpret,
    GraphType,
    Axes,
    Pairwise,
    Rulers,
    Align,
    Bold,
    Italic,
    Palette,
    Print,
    Export,
    Theme,
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarGroup {
    File,
    History,
    Clipboard,
    Sheet,
    Analysis,
    GraphLayout,
    TextStyle,
    Output,
}

#[derive(Debug, Clone)]
struct ToolbarControlItem {
    id: &'static str,
    group: ToolbarGroup,
    icon: ToolbarIconKind,
    label: &'static str,
    tooltip: String,
    keyboard_path: String,
    shortcut: Option<String>,
    primary: bool,
    selected: bool,
    essential: bool,
    enablement: ControlEnablement,
    action: WidgetAction,
}

#[derive(Debug, Clone)]
struct ContextMenuItemSpec {
    id: &'static str,
    label: String,
    keyboard_path: &'static str,
    shortcut: Option<&'static str>,
    enablement: ControlEnablement,
    action: WidgetAction,
}

fn tr_with_fallback(locale: &str, key: &'static str, fallback: &'static str) -> &'static str {
    crate::i18n::try_tr(locale, key).unwrap_or(fallback)
}

fn active_data_sheet(doc: &ProjectDocument) -> Option<&DataSheet> {
    doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id))
}

fn target_data_sheet(doc: &ProjectDocument) -> Option<&DataSheet> {
    active_data_sheet(doc).or_else(|| doc.data_sheets.first())
}

fn target_result_sheet(doc: &ProjectDocument) -> Option<&ResultSheet> {
    doc.active_sheet_id.as_ref().and_then(|id| doc.result_sheet(id)).or_else(|| {
        active_data_sheet(doc)
            .and_then(|ds| doc.result_sheets.iter().find(|rs| rs.analysis.source_sheet_id == ds.id))
            .or_else(|| doc.result_sheets.first())
    })
}

fn target_graph_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some()).cloned().or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
}

fn target_visual_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(id) = doc.active_sheet_id.as_ref()
        && (doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some())
    {
        return Some(id.clone());
    }
    doc.graph_sheets.first().map(|g| g.id.clone()).or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
}

fn active_sheet_title(doc: &ProjectDocument) -> Option<&str> {
    let id = doc.active_sheet_id.as_ref()?;
    doc.data_sheet(id)
        .map(|s| s.title.as_str())
        .or_else(|| doc.result_sheet(id).map(ResultSheet::title))
        .or_else(|| doc.graph_sheet(id).map(|s| s.title.as_str()))
        .or_else(|| doc.layout_sheet(id).map(|s| s.title.as_str()))
        .or_else(|| doc.info_sheet(id).map(|s| s.title.as_str()))
}

fn active_selection_coords_for(app: &UiApp, ds: &DataSheet) -> Vec<GridCellCoord> {
    let mut coords = app.grid.all_selected_coords(ds);
    if coords.is_empty()
        && let Some(active) = app.grid.tool.active
    {
        coords.push(active);
    }
    if coords.is_empty() && ds.archetype.row_count() > 0 && ds.archetype.column_count() > 0 {
        coords.push(GridCellCoord::new(0, 0, 0));
    }
    coords
}

fn selection_bounding_range(coords: &[GridCellCoord]) -> Option<SheetCellRange> {
    let first = coords.first()?;
    let (mut min_r, mut max_r) = (first.row, first.row);
    let (mut min_c, mut max_c) = (first.col, first.col);
    let (mut min_s, mut max_s) = (first.subcol, first.subcol);
    for c in coords.iter().skip(1) {
        min_r = min_r.min(c.row);
        max_r = max_r.max(c.row);
        min_c = min_c.min(c.col);
        max_c = max_c.max(c.col);
        min_s = min_s.min(c.subcol);
        max_s = max_s.max(c.subcol);
    }
    Some(SheetCellRange::new(curvebook_doc::CellCoord::new(min_r, min_c, min_s), curvebook_doc::CellCoord::new(max_r, max_c, max_s)))
}

fn active_linked_region<'a>(app: &UiApp, ds: &'a DataSheet) -> Option<&'a curvebook_doc::LinkedDataRegion> {
    let probe = app.grid.tool.active.or_else(|| app.grid.selected_coords.first().copied());
    if let Some(c) = probe
        && let Some(found) = ds.linked_regions.iter().find(|reg| {
            c.row >= reg.target_range.start.row
                && c.row <= reg.target_range.end.row
                && c.col >= reg.target_range.start.col
                && c.col <= reg.target_range.end.col
        })
    {
        return Some(found);
    }
    ds.linked_regions.first()
}

fn copy_active_sheet_to_clipboard(app: &mut UiApp, col_only: Option<usize>) -> Result<String, String> {
    let doc = app.document().clone();
    let Some(sid) = doc.active_sheet_id.as_ref() else {
        return Err(String::from("no active sheet to copy"));
    };
    let dec = app.grid.decimal_separator;
    let payload = if let Some(ds) = doc.data_sheet(sid) {
        if let Some(col_idx) = col_only {
            let max_row = ds.archetype.row_count().saturating_sub(1);
            let subcols = data_set_subcolumn_count(&ds.archetype, col_idx).max(1);
            let range = SheetCellRange::new(
                curvebook_doc::CellCoord::new(0, col_idx, 0),
                curvebook_doc::CellCoord::new(max_row, col_idx, subcols.saturating_sub(1)),
            );
            ClipboardBundle::tsv_from_datasheet_range(ds, range, dec)
        } else {
            let coords = app.grid.all_selected_coords(ds);
            if let Some(range) = selection_bounding_range(&coords)
                && !coords.is_empty()
            {
                ClipboardBundle::tsv_from_datasheet_range(ds, range, dec)
            } else {
                ClipboardBundle::tsv_from_datasheet(ds, dec)?
            }
        }
    } else if let Some(rs) = doc.result_sheet(sid) {
        let mut lines = Vec::new();
        if let Some(tab) = rs.tabs.first() {
            if !tab.column_headers.is_empty() {
                lines.push(tab.column_headers.join("\t"));
            }
            for row in &tab.rows {
                let cols: Vec<String> = row.iter().map(|cell| ResultSheetViewState::format_result_cell(cell, rs.analysis.p_value_format)).collect();
                lines.push(cols.join("\t"));
            }
        } else {
            lines.push(rs.title().to_owned());
        }
        lines.join("\n")
    } else if let Some(info) = doc.info_sheet(sid) {
        let mut lines = vec![info.title.clone()];
        if !info.notes_markdown.is_empty() {
            lines.push(info.notes_markdown.clone());
        }
        for c in &info.constants {
            lines.push(format!("{}\t{:?}", c.name, c.value));
        }
        lines.join("\n")
    } else if doc.graph_sheet(sid).is_some() || doc.layout_sheet(sid).is_some() {
        match app.session().render_sheet_scene(sid) {
            Ok(scene) => {
                let bundle = ClipboardBundle::from_scene(&scene, 144)?;
                if !bundle.svg.is_empty() { bundle.svg } else { active_sheet_title(&doc).unwrap_or("Figure").to_owned() }
            }
            Err(_) => active_sheet_title(&doc).unwrap_or("Figure").to_owned(),
        }
    } else {
        return Err(String::from("active sheet is not copyable"));
    };
    let line_count = payload.lines().count().max(1);
    app.interaction.last_clipboard_text = Some(payload);
    Ok(format!("Copied {line_count} line(s)"))
}

fn range_to_json(range: SheetCellRange) -> Value {
    json!({
        "start": { "row": range.start.row, "col": range.start.col, "subcol": range.start.subcol },
        "end": { "row": range.end.row, "col": range.end.col, "subcol": range.end.subcol },
    })
}

fn run_exclude_action(app: &mut UiApp, exclude: bool, column_scope: bool) -> Result<String, String> {
    let doc = app.document().clone();
    let Some(ds) = active_data_sheet(&doc) else {
        return Err(String::from("no active data sheet"));
    };
    if ds.frozen {
        return Err(format!("sheet `{}` is frozen", ds.title));
    }
    let sheet_id = ds.id.clone();

    if column_scope {
        let col = app.grid.tool.active.map_or(0, |c| c.col).min(ds.archetype.column_count().saturating_sub(1));
        let max_row = ds.archetype.row_count().saturating_sub(1);
        let max_subcol = data_set_subcolumn_count(&ds.archetype, col).saturating_sub(1);
        let range = SheetCellRange::new(curvebook_doc::CellCoord::new(0, col, 0), curvebook_doc::CellCoord::new(max_row, col, max_subcol));
        app.execute_command("table.excludeCells", json!({ "sheet_id": sheet_id.as_str(), "excluded": exclude, "range": range_to_json(range) }))
            .map_err(|e| e.to_string())?;
        app.grid.refresh_selection_stats(app.session.document());
        return Ok(format!("{} column {}", if exclude { "Excluded" } else { "Included" }, col + 1));
    }

    if app.grid.all_selected_coords(ds).is_empty()
        && let Some((row, subcol)) = app.grid.active_x_cell
    {
        let _ = app
            .execute_command(
                "table.excludeCells",
                json!({ "sheet_id": sheet_id.as_str(), "target": "x", "excluded": exclude, "row": row, "subcol": subcol }),
            )
            .map_err(|e| e.to_string())?;
        return Ok(String::from(if exclude { "Excluded X cell" } else { "Included X cell" }));
    }

    app.grid.tool.pending_intents.clear();
    let count = app.grid.emit_toggle_exclude_intents(ds, exclude);
    let intents = std::mem::take(&mut app.grid.tool.pending_intents);
    if !intents.is_empty() {
        for intent in intents {
            app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
        }
        app.grid.refresh_selection_stats(app.session.document());
        return Ok(format!("{} {count} range(s)", if exclude { "Excluded" } else { "Included" }));
    }

    let c = app.grid.tool.active.unwrap_or(GridCellCoord::new(0, 0, 0));
    let single = SheetCellRange::new(curvebook_doc::CellCoord::from(c), curvebook_doc::CellCoord::from(c));
    app.execute_command("table.excludeCells", json!({ "sheet_id": sheet_id.as_str(), "excluded": exclude, "range": range_to_json(single) }))
        .map_err(|e| e.to_string())?;
    app.grid.refresh_selection_stats(app.session.document());
    Ok(String::from(if exclude { "Excluded cell" } else { "Included cell" }))
}

fn dismiss_context_overlays(app: &mut UiApp, state: &mut ToolbarMenusViewState) {
    state.active_context_menu_id = None;
    state.context_menu_anchor_px = None;
    state.context_sheet_id = None;
    state.overflow_open = false;
    app.interaction.context_menu_target = None;
    app.interaction.open_menu = None;
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `toolbar_menus` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut ToolbarMenusViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if payload == "overflow.toggle" {
        state.overflow_open = !state.overflow_open;
        state.active_context_menu_id = None;
        app.interaction.context_menu_target = None;
        app.interaction.open_menu = None;
        return Ok(UiEventOutcome { handled: true, action: format!("toolbar.overflow:{}", state.overflow_open), ..Default::default() });
    }
    if payload == "project.open_browser" {
        dismiss_context_overlays(app, state);
        app.nav.gallery.open = !app.nav.gallery.open;
        state.last_toolbar_notice = Some(String::from(if app.nav.gallery.open { "Opened project gallery" } else { "Closed project gallery" }));
        return Ok(UiEventOutcome { handled: true, action: String::from("toolbar.open_browser"), ..Default::default() });
    }
    if payload == "clipboard.copy" || payload == "clipboard.copy_col" {
        let col_only = if payload == "clipboard.copy_col" { Some(app.grid.tool.active.map_or(0, |c| c.col)) } else { None };
        dismiss_context_overlays(app, state);
        let msg = copy_active_sheet_to_clipboard(app, col_only)?;
        state.last_toolbar_notice = Some(msg.clone());
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from(payload),
            command_result: Some(json!({ "notice": msg })),
            ..Default::default()
        });
    }
    if payload == "clipboard.cut" {
        dismiss_context_overlays(app, state);
        let msg = copy_active_sheet_to_clipboard(app, None)?;
        let doc = app.document().clone();
        if let Some(ds) = active_data_sheet(&doc)
            && !ds.frozen
        {
            let coords = active_selection_coords_for(app, ds);
            if let Some(range) = selection_bounding_range(&coords) {
                app.execute_command("table.clearCells", json!({ "sheet_id": ds.id.as_str(), "range": range_to_json(range) }))
                    .map_err(|e| e.to_string())?;
                app.grid.refresh_selection_stats(app.session.document());
            }
        }
        state.last_toolbar_notice = Some(format!("Cut ({msg})"));
        return Ok(UiEventOutcome { handled: true, action: String::from("clipboard.cut"), ..Default::default() });
    }
    if payload == "clipboard.paste" {
        dismiss_context_overlays(app, state);
        let clip = app.interaction.last_clipboard_text.clone().ok_or_else(|| String::from("clipboard is empty"))?;
        let doc = app.document().clone();
        let ds = active_data_sheet(&doc).ok_or_else(|| String::from("no active data sheet for paste"))?;
        let anchor = app.grid.tool.active.unwrap_or(GridCellCoord::new(0, 0, 0));
        let val = app
            .execute_command(
                "table.paste",
                json!({
                    "sheet_id": ds.id.as_str(),
                    "text": clip,
                    "start_row": anchor.row,
                    "start_col": anchor.col,
                    "start_subcol": anchor.subcol,
                    "propagate_title_header": false,
                }),
            )
            .map_err(|e| e.to_string())?;
        app.grid.refresh_selection_stats(app.session.document());
        state.last_toolbar_notice = Some(String::from("Pasted clipboard cells"));
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from("clipboard.paste"),
            command_id: Some(String::from("table.paste")),
            command_result: Some(serde_json::to_value(val).unwrap_or(Value::Bool(true))),
        });
    }
    if payload == "analyze.parameters" {
        dismiss_context_overlays(app, state);
        let doc = app.document().clone();
        let rs = target_result_sheet(&doc).ok_or_else(|| String::from("no analysis result sheet available"))?;
        let opened = app.canvas_dialogs.open_edit_parameters(&doc, &rs.id);
        state.last_toolbar_notice = Some(format!("Editing parameters: {}", rs.title()));
        return Ok(UiEventOutcome {
            handled: opened,
            action: String::from("analyze.parameters"),
            command_result: Some(json!({ "result_sheet_id": rs.id.as_str(), "opened": opened })),
            ..Default::default()
        });
    }
    if payload == "analyze.data_analyzed" {
        dismiss_context_overlays(app, state);
        let doc = app.document().clone();
        let rs = target_result_sheet(&doc).ok_or_else(|| String::from("no analysis result sheet available"))?;
        let opened = app.canvas_dialogs.open_change_data_analyzed(&doc, &rs.id);
        state.last_toolbar_notice = Some(format!("Change data source: {}", rs.title()));
        return Ok(UiEventOutcome {
            handled: opened,
            action: String::from("analyze.data_analyzed"),
            command_result: Some(json!({ "result_sheet_id": rs.id.as_str(), "opened": opened })),
            ..Default::default()
        });
    }
    if payload == "analyze.interpret" {
        dismiss_context_overlays(app, state);
        let show = !(app.nav.inspector_open && app.canvas_dialogs.active_inspector == ActiveInspector::ResultDetails);
        app.nav.inspector_open = show;
        app.canvas_dialogs.active_inspector = if show { ActiveInspector::ResultDetails } else { ActiveInspector::None };
        state.last_toolbar_notice = Some(String::from(if show { "Opened Interpret Checklist" } else { "Closed Interpret Checklist" }));
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from("analyze.interpret"),
            command_result: Some(json!({ "inspector_open": app.nav.inspector_open, "active_inspector": "ResultDetails" })),
            ..Default::default()
        });
    }
    if payload == "layout.align_origins" {
        dismiss_context_overlays(app, state);
        let next_align = !app.canvas_dialogs.page_layout.align_plot_origins;
        app.canvas_dialogs.page_layout.align_plot_origins = next_align;
        let doc = app.document().clone();
        if let Some(ls_id) =
            doc.active_sheet_id.as_ref().filter(|id| doc.layout_sheet(id).is_some()).or_else(|| doc.layout_sheets.first().map(|l| &l.id))
        {
            let _ = app
                .execute_command("layout.alignOrigins", json!({ "sheet_id": ls_id.as_str(), "align": next_align, "normalize_bounds": true }))
                .map_err(|e| e.to_string())?;
        }
        state.last_toolbar_notice = Some(String::from(if next_align { "Aligned plot origins" } else { "Unaligned plot origins" }));
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from("layout.align_origins"),
            command_result: Some(json!({ "align_plot_origins": next_align })),
            ..Default::default()
        });
    }
    if payload == "text.toggle_bold" {
        dismiss_context_overlays(app, state);
        state.text_bold = !state.text_bold;
        state.last_toolbar_notice = Some(String::from(if state.text_bold { "Text weight: Bold" } else { "Text weight: Regular" }));
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from("text.toggle_bold"),
            command_result: Some(json!({ "text_bold": state.text_bold })),
            ..Default::default()
        });
    }
    if payload == "text.toggle_italic" {
        dismiss_context_overlays(app, state);
        state.text_italic = !state.text_italic;
        state.last_toolbar_notice = Some(String::from(if state.text_italic { "Text style: Italic" } else { "Text style: Upright" }));
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from("text.toggle_italic"),
            command_result: Some(json!({ "text_italic": state.text_italic })),
            ..Default::default()
        });
    }
    handle_context_menu_custom_action(app, state, payload)
}

fn resolve_nav_target_sheet(app: &UiApp, state: &ToolbarMenusViewState) -> Option<SheetId> {
    let doc = app.document();
    if let Some(ref sid_str) = state.context_sheet_id
        && !sid_str.trim().is_empty()
    {
        return Some(SheetId::new(sid_str.as_str()));
    }
    let raw_target = app.interaction.context_menu_target.as_deref().or(state.active_context_menu_id.as_deref());
    if let Some((_, suffix)) = raw_target.and_then(|t| t.split_once(':'))
        && !suffix.trim().is_empty()
    {
        return Some(SheetId::new(suffix.trim()));
    }
    doc.active_sheet_id.clone().or_else(|| doc.data_sheets.first().map(|s| s.id.clone()))
}

fn handle_context_menu_custom_action(app: &mut UiApp, state: &mut ToolbarMenusViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let nav_sid = resolve_nav_target_sheet(app, state);
    dismiss_context_overlays(app, state);
    match payload {
        "ctx.exclude" => {
            let msg = run_exclude_action(app, true, false)?;
            state.last_toolbar_notice = Some(msg.clone());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: Some(json!({ "notice": msg })), ..Default::default() })
        }
        "ctx.include" => {
            let msg = run_exclude_action(app, false, false)?;
            state.last_toolbar_notice = Some(msg.clone());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: Some(json!({ "notice": msg })), ..Default::default() })
        }
        "ctx.exclude_column" => {
            let msg = run_exclude_action(app, true, true)?;
            state.last_toolbar_notice = Some(msg.clone());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: Some(json!({ "notice": msg })), ..Default::default() })
        }
        "ctx.include_column" => {
            let msg = run_exclude_action(app, false, true)?;
            state.last_toolbar_notice = Some(msg.clone());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: Some(json!({ "notice": msg })), ..Default::default() })
        }
        "ctx.exclude_with_reason" => {
            let doc = app.document().clone();
            let ds = target_data_sheet(&doc).ok_or_else(|| String::from("no data sheet available"))?;
            let coords = active_selection_coords_for(app, ds);
            app.canvas_dialogs.open_exclude_with_reason(ds.id.clone(), coords);
            state.last_toolbar_notice = Some(String::from("Exclude with Reason"));
            Ok(UiEventOutcome {
                handled: true,
                action: String::from(payload),
                command_id: Some(String::from("ui.grid.excludeWithReason")),
                command_result: Some(json!({ "dialog": "ExcludeWithReason", "coords": app.canvas_dialogs.exclude_with_reason.coords.len() })),
            })
        }
        "ctx.clear_cells" => {
            let doc = app.document().clone();
            let ds = active_data_sheet(&doc).ok_or_else(|| String::from("no active data sheet"))?;
            let coords = active_selection_coords_for(app, ds);
            if let Some(range) = selection_bounding_range(&coords) {
                app.execute_command("table.clearCells", json!({ "sheet_id": ds.id.as_str(), "range": range_to_json(range) }))
                    .map_err(|e| e.to_string())?;
                app.grid.refresh_selection_stats(app.session.document());
            }
            state.last_toolbar_notice = Some(format!("Cleared {} cell(s)", coords.len()));
            Ok(UiEventOutcome { handled: true, action: String::from(payload), ..Default::default() })
        }
        "ctx.formula_bar" => {
            let doc = app.document().clone();
            let ds = target_data_sheet(&doc).ok_or_else(|| String::from("no data sheet available"))?;
            let col = app.grid.tool.active.map_or(0, |c| c.col);
            let res = crate::control_io::dispatch_ui_command(app, "ui.table.openFormulaBar", &json!({ "sheet_id": ds.id.as_str(), "column": col }))
                .map_err(|e| e.to_string())?;
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: res, ..Default::default() })
        }
        "ctx.variables_role" => {
            let doc = app.document().clone();
            let ds = target_data_sheet(&doc).ok_or_else(|| String::from("no data sheet available"))?;
            let col = app.grid.tool.active.map_or(0, |c| c.col);
            app.grid.variables_dialog.open_for_column(ds, col);
            state.last_toolbar_notice = Some(String::from("Opened Variables & Roles"));
            Ok(UiEventOutcome { handled: true, action: String::from(payload), ..Default::default() })
        }
        "ctx.analyze_sheet" => {
            let doc = app.document().clone();
            let ds = target_data_sheet(&doc).ok_or_else(|| String::from("no data sheet available"))?;
            let res =
                crate::control_io::dispatch_ui_command(app, "ui.analyze.open", &json!({ "sheet_id": ds.id.as_str() })).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: res, ..Default::default() })
        }
        "ctx.nav_open" => {
            let sid = nav_sid.ok_or_else(|| String::from("no navigator sheet target"))?;
            app.execute_command("sheet.select", json!({ "sheet_id": sid.as_str() })).map_err(|e| e.to_string())?;
            app.nav.sync_from_document(app.session.document());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), ..Default::default() })
        }
        "ctx.nav_duplicate" => {
            let sid = nav_sid.ok_or_else(|| String::from("no navigator sheet target"))?;
            app.execute_command("sheet.duplicate", json!({ "sheet_id": sid.as_str() })).map_err(|e| e.to_string())?;
            app.nav.sync_from_document(app.session.document());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), ..Default::default() })
        }
        "ctx.nav_freeze_toggle" => {
            let sid = nav_sid.ok_or_else(|| String::from("no navigator sheet target"))?;
            let is_frozen = app.document().data_sheet(&sid).is_some_and(|ds| ds.frozen);
            let cmd = if is_frozen { "sheet.unfreeze" } else { "sheet.freeze" };
            app.execute_command(cmd, json!({ "sheet_id": sid.as_str() })).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome { handled: true, action: String::from(payload), ..Default::default() })
        }
        "ctx.nav_clone" => {
            let sid = nav_sid.ok_or_else(|| String::from("no navigator sheet target"))?;
            let res = crate::control_io::dispatch_ui_command(app, "ui.clonePipeline.open", &json!({ "sheet_id": sid.as_str() }))
                .map_err(|e| e.to_string())?;
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: res, ..Default::default() })
        }
        "ctx.nav_delete" => {
            let sid = nav_sid.ok_or_else(|| String::from("no navigator sheet target"))?;
            app.execute_command("sheet.delete", json!({ "sheet_id": sid.as_str(), "cascade": false })).map_err(|e| e.to_string())?;
            app.nav.sync_from_document(app.session.document());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), ..Default::default() })
        }
        "ctx.graph_points" | "ctx.graph_format" | "ctx.graph_axes" | "ctx.graph_type" | "ctx.graph_pairwise" | "ctx.export_figure" => {
            let doc = app.document().clone();
            let gid = target_graph_id(&doc).or_else(|| target_visual_sheet_id(&doc)).ok_or_else(|| String::from("no graph sheet available"))?;
            let cmd = match payload {
                "ctx.graph_points" => "ui.formatPoints.open",
                "ctx.graph_format" => "ui.graph.format.open",
                "ctx.graph_axes" => "ui.graph.axes.open",
                "ctx.graph_type" => "ui.changeGraphType.open",
                "ctx.graph_pairwise" => "ui.graph.pairwise.open",
                _ => "ui.export.figure.open",
            };
            let params = if cmd == "ui.export.figure.open" {
                json!({ "sheet_id": gid.as_str(), "format": "svg" })
            } else {
                json!({ "sheet_id": gid.as_str() })
            };
            let res = crate::control_io::dispatch_ui_command(app, cmd, &params).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_id: Some(cmd.to_owned()), command_result: res })
        }
        "ctx.go_to_source" => {
            let doc = app.document().clone();
            let src_sid = active_data_sheet(&doc)
                .and_then(|ds| active_linked_region(app, ds))
                .and_then(|reg| reg.spec.source.upstream_sheet_id().cloned())
                .or_else(|| target_result_sheet(&doc).map(|rs| rs.analysis.source_sheet_id.clone()))
                .ok_or_else(|| String::from("no upstream linked source sheet"))?;
            app.execute_command("sheet.select", json!({ "sheet_id": src_sid.as_str() })).map_err(|e| e.to_string())?;
            app.nav.sync_from_document(app.session.document());
            state.last_toolbar_notice = Some(format!("Navigated to source sheet {}", src_sid.as_str()));
            Ok(UiEventOutcome {
                handled: true,
                action: String::from(payload),
                command_id: Some(String::from("sheet.select")),
                command_result: Some(json!({ "source_sheet_id": src_sid.as_str() })),
            })
        }
        "ctx.refresh_link" => {
            app.execute_command("project.recalculate", json!({ "include_external_links": false })).map_err(|e| e.to_string())?;
            state.last_toolbar_notice = Some(String::from("Refreshed linked region"));
            Ok(UiEventOutcome {
                handled: true,
                action: String::from(payload),
                command_id: Some(String::from("project.recalculate")),
                ..Default::default()
            })
        }
        "ctx.unlink_region" => {
            let doc = app.document().clone();
            let ds = active_data_sheet(&doc).ok_or_else(|| String::from("no active data sheet"))?;
            app.execute_command("table.clearCells", json!({ "sheet_id": ds.id.as_str(), "unlink": true, "cells": [] })).map_err(|e| e.to_string())?;
            state.last_toolbar_notice = Some(String::from("Detached linked region"));
            Ok(UiEventOutcome {
                handled: true,
                action: String::from(payload),
                command_id: Some(String::from("table.clearCells")),
                ..Default::default()
            })
        }
        _ => Ok(UiEventOutcome::default()),
    }
}

fn non_null_obj_with_sheet(sheet_id: Option<&SheetId>) -> Value {
    let mut map = Map::new();
    if let Some(sid) = sheet_id {
        map.insert(String::from("sheet_id"), Value::String(sid.as_str().to_owned()));
    }
    Value::Object(map)
}

fn custom_toolbar_action(payload: &str) -> WidgetAction {
    WidgetAction::Custom { domain: String::from("toolbar_menus"), payload: payload.to_owned() }
}

fn append_toolbar_file_and_history_items(app: &UiApp, locale: &str, items: &mut Vec<ToolbarControlItem>) {
    let can_undo = app.session().undo_depth() > 0;
    let can_redo = app.session().redo_depth() > 0;

    items.push(ToolbarControlItem {
        id: "toolbar.new",
        group: ToolbarGroup::File,
        icon: ToolbarIconKind::New,
        label: tr_with_fallback(locale, "toolbar.new", "New"),
        tooltip: String::from("Create a new scientific project"),
        keyboard_path: String::from("Cmd+N"),
        shortcut: Some(String::from("Cmd+N")),
        primary: false,
        selected: false,
        essential: true,
        enablement: ControlEnablement::enabled(),
        action: WidgetAction::InvokeCommand { command_id: String::from("project.new"), params: json!({}) },
    });
    items.push(ToolbarControlItem {
        id: "toolbar.open",
        group: ToolbarGroup::File,
        icon: ToolbarIconKind::Open,
        label: tr_with_fallback(locale, "toolbar.open", "Open"),
        tooltip: String::from("Browse projects and sample templates"),
        keyboard_path: String::from("Cmd+O"),
        shortcut: Some(String::from("Cmd+O")),
        primary: false,
        selected: app.nav.gallery.open,
        essential: true,
        enablement: ControlEnablement::enabled(),
        action: custom_toolbar_action("project.open_browser"),
    });
    items.push(ToolbarControlItem {
        id: "toolbar.save",
        group: ToolbarGroup::File,
        icon: ToolbarIconKind::Save,
        label: tr_with_fallback(locale, "toolbar.save", "Save"),
        tooltip: String::from("Save project state"),
        keyboard_path: String::from("Cmd+S"),
        shortcut: Some(String::from("Cmd+S")),
        primary: false,
        selected: false,
        essential: true,
        enablement: ControlEnablement::enabled(),
        action: WidgetAction::InvokeCommand { command_id: String::from("project.save"), params: json!({ "return_bytes": true }) },
    });
    items.push(ToolbarControlItem {
        id: "toolbar.undo",
        group: ToolbarGroup::History,
        icon: ToolbarIconKind::Undo,
        label: tr_with_fallback(locale, "toolbar.undo", "Undo"),
        tooltip: String::from("Undo last edit"),
        keyboard_path: String::from("Cmd+Z"),
        shortcut: Some(String::from("Cmd+Z")),
        primary: false,
        selected: false,
        essential: true,
        enablement: if can_undo { ControlEnablement::enabled() } else { ControlEnablement::disabled("Nothing to undo.") },
        action: WidgetAction::InvokeCommand { command_id: String::from("edit.undo"), params: json!({}) },
    });
    items.push(ToolbarControlItem {
        id: "toolbar.redo",
        group: ToolbarGroup::History,
        icon: ToolbarIconKind::Redo,
        label: tr_with_fallback(locale, "toolbar.redo", "Redo"),
        tooltip: String::from("Redo undone edit"),
        keyboard_path: String::from("Cmd+Shift+Z"),
        shortcut: Some(String::from("Cmd+Shift+Z")),
        primary: false,
        selected: false,
        essential: true,
        enablement: if can_redo { ControlEnablement::enabled() } else { ControlEnablement::disabled("Nothing to redo.") },
        action: WidgetAction::InvokeCommand { command_id: String::from("edit.redo"), params: json!({}) },
    });
}

fn append_toolbar_clipboard_and_sheet_items(app: &UiApp, locale: &str, ctx: ActiveToolbarContext, items: &mut Vec<ToolbarControlItem>) {
    let doc = app.document();
    let active_sid = doc.active_sheet_id.as_ref();
    let active_ds = active_data_sheet(doc);
    let active_frozen = active_ds.is_some_and(|ds| ds.frozen);
    let can_clip_paste = active_ds.is_some_and(|ds| !ds.frozen) && app.interaction.last_clipboard_text.as_deref().is_some_and(|t| !t.is_empty());

    if ctx != ActiveToolbarContext::EmptyWorkspace {
        items.push(ToolbarControlItem {
            id: "toolbar.copy",
            group: ToolbarGroup::Clipboard,
            icon: ToolbarIconKind::Copy,
            label: tr_with_fallback(locale, "toolbar.copy", "Copy"),
            tooltip: String::from("Copy selected cells or active sheet"),
            keyboard_path: String::from("Cmd+C"),
            shortcut: Some(String::from("Cmd+C")),
            primary: false,
            selected: false,
            essential: false,
            enablement: if active_sid.is_some() { ControlEnablement::enabled() } else { ControlEnablement::disabled("Select a sheet to copy.") },
            action: custom_toolbar_action("clipboard.copy"),
        });
        if matches!(ctx, ActiveToolbarContext::DataSheet | ActiveToolbarContext::InfoSheet) {
            items.push(ToolbarControlItem {
                id: "toolbar.cut",
                group: ToolbarGroup::Clipboard,
                icon: ToolbarIconKind::Cut,
                label: tr_with_fallback(locale, "toolbar.cut", "Cut"),
                tooltip: String::from("Cut selected cells to clipboard"),
                keyboard_path: String::from("Cmd+X"),
                shortcut: Some(String::from("Cmd+X")),
                primary: false,
                selected: false,
                essential: false,
                enablement: if active_ds.is_some_and(|ds| !ds.frozen) {
                    ControlEnablement::enabled()
                } else {
                    ControlEnablement::disabled("Requires an unfrozen data sheet.")
                },
                action: custom_toolbar_action("clipboard.cut"),
            });
            items.push(ToolbarControlItem {
                id: "toolbar.paste",
                group: ToolbarGroup::Clipboard,
                icon: ToolbarIconKind::Paste,
                label: tr_with_fallback(locale, "toolbar.paste", "Paste"),
                tooltip: String::from("Paste tabular clipboard data at active cell"),
                keyboard_path: String::from("Cmd+V"),
                shortcut: Some(String::from("Cmd+V")),
                primary: false,
                selected: false,
                essential: false,
                enablement: if can_clip_paste {
                    ControlEnablement::enabled()
                } else {
                    ControlEnablement::disabled("Copy data first or unfreeze the target table.")
                },
                action: custom_toolbar_action("clipboard.paste"),
            });
        }
    }

    if let Some(sid) = active_sid {
        items.push(ToolbarControlItem {
            id: "toolbar.sheet_duplicate",
            group: ToolbarGroup::Sheet,
            icon: ToolbarIconKind::Duplicate,
            label: tr_with_fallback(locale, "toolbar.sheet_duplicate", "Duplicate"),
            tooltip: String::from("Duplicate the active sheet"),
            keyboard_path: String::from("sheet.duplicate"),
            shortcut: None,
            primary: false,
            selected: false,
            essential: false,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::InvokeCommand { command_id: String::from("sheet.duplicate"), params: json!({ "sheet_id": sid.as_str() }) },
        });
        if ctx == ActiveToolbarContext::DataSheet {
            let (freeze_lbl, freeze_cmd) = if active_frozen {
                (tr_with_fallback(locale, "toolbar.sheet_unfreeze", "Unfreeze"), "sheet.unfreeze")
            } else {
                (tr_with_fallback(locale, "toolbar.sheet_freeze", "Freeze"), "sheet.freeze")
            };
            items.push(ToolbarControlItem {
                id: "toolbar.sheet_freeze",
                group: ToolbarGroup::Sheet,
                icon: ToolbarIconKind::Freeze,
                label: freeze_lbl,
                tooltip: String::from(if active_frozen { "Unlock data table edits" } else { "Lock data table against accidental edits" }),
                keyboard_path: String::from(freeze_cmd),
                shortcut: None,
                primary: false,
                selected: active_frozen,
                essential: false,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::InvokeCommand { command_id: String::from(freeze_cmd), params: json!({ "sheet_id": sid.as_str() }) },
            });
        }
    }
}

fn append_toolbar_analysis_items(app: &UiApp, locale: &str, ctx: ActiveToolbarContext, items: &mut Vec<ToolbarControlItem>) {
    let doc = app.document();
    let active_ds = active_data_sheet(doc);
    let target_analyze_sid = active_ds
        .map(|ds| &ds.id)
        .or_else(|| doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()))
        .or_else(|| doc.data_sheets.first().map(|ds| &ds.id));
    let target_rs = target_result_sheet(doc);
    let can_analyze = command_tooltip(app, "ui.analyze.open").is_some_and(|t| t.enabled);

    items.push(ToolbarControlItem {
        id: "toolbar.analyze",
        group: ToolbarGroup::Analysis,
        icon: ToolbarIconKind::Analyze,
        label: tr_with_fallback(locale, "btn.analyze", "Analyze"),
        tooltip: String::from("Choose statistical test or regression analysis"),
        keyboard_path: String::from("Cmd+Shift+A"),
        shortcut: Some(String::from("Cmd+Shift+A")),
        primary: true,
        selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }),
        essential: true,
        enablement: if can_analyze { ControlEnablement::enabled() } else { ControlEnablement::disabled("Create or select a data sheet first.") },
        action: WidgetAction::InvokeCommand { command_id: String::from("ui.analyze.open"), params: non_null_obj_with_sheet(target_analyze_sid) },
    });
    if ctx == ActiveToolbarContext::ResultSheet || (ctx == ActiveToolbarContext::DataSheet && target_rs.is_some()) {
        items.push(ToolbarControlItem {
            id: "toolbar.parameters",
            group: ToolbarGroup::Analysis,
            icon: ToolbarIconKind::Parameters,
            label: tr_with_fallback(locale, "toolbar.parameters", "Parameters"),
            tooltip: String::from("Edit statistical analysis parameters"),
            keyboard_path: String::from("Cmd+T"),
            shortcut: Some(String::from("Cmd+T")),
            primary: false,
            selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
            essential: ctx == ActiveToolbarContext::ResultSheet,
            enablement: if target_rs.is_some() {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled("No analysis result sheet in project.")
            },
            action: custom_toolbar_action("analyze.parameters"),
        });
    }
    if ctx == ActiveToolbarContext::ResultSheet {
        items.push(ToolbarControlItem {
            id: "toolbar.data_analyzed",
            group: ToolbarGroup::Analysis,
            icon: ToolbarIconKind::DataAnalyzed,
            label: tr_with_fallback(locale, "toolbar.data_analyzed", "Data Analyzed"),
            tooltip: String::from("Switch or inspect source data analyzed"),
            keyboard_path: String::from("analyze.data_analyzed"),
            shortcut: None,
            primary: false,
            selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeDataAnalyzed { .. }),
            essential: true,
            enablement: if target_rs.is_some() { ControlEnablement::enabled() } else { ControlEnablement::disabled("Requires a result sheet.") },
            action: custom_toolbar_action("analyze.data_analyzed"),
        });
        items.push(ToolbarControlItem {
            id: "toolbar.interpret",
            group: ToolbarGroup::Analysis,
            icon: ToolbarIconKind::Interpret,
            label: tr_with_fallback(locale, "toolbar.interpret", "Interpret"),
            tooltip: String::from("Open analysis assumptions and checklist"),
            keyboard_path: String::from("analyze.interpret"),
            shortcut: None,
            primary: false,
            selected: app.nav.inspector_open && app.canvas_dialogs.active_inspector == ActiveInspector::ResultDetails,
            essential: true,
            enablement: ControlEnablement::enabled(),
            action: custom_toolbar_action("analyze.interpret"),
        });
    }
}

fn append_toolbar_graph_and_text_items(
    app: &UiApp,
    locale: &str,
    ctx: ActiveToolbarContext,
    state: &ToolbarMenusViewState,
    items: &mut Vec<ToolbarControlItem>,
) {
    let doc = app.document();
    let target_gid = target_graph_id(doc);
    let can_graph = !doc.graph_sheets.is_empty();

    if ctx == ActiveToolbarContext::GraphSheet {
        items.push(ToolbarControlItem {
            id: "toolbar.change_graph",
            group: ToolbarGroup::GraphLayout,
            icon: ToolbarIconKind::GraphType,
            label: tr_with_fallback(locale, "toolbar.graph_type", "Graph Type"),
            tooltip: String::from("Change graph plot family and summary error bars"),
            keyboard_path: String::from("Cmd+Shift+G"),
            shortcut: Some(String::from("Cmd+Shift+G")),
            primary: false,
            selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }),
            essential: true,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::InvokeCommand {
                command_id: String::from("ui.changeGraphType.open"),
                params: non_null_obj_with_sheet(target_gid.as_ref()),
            },
        });
    }
    items.push(ToolbarControlItem {
        id: "toolbar.axes",
        group: ToolbarGroup::GraphLayout,
        icon: ToolbarIconKind::Axes,
        label: tr_with_fallback(locale, "toolbar.axes", "Axes"),
        tooltip: String::from("Configure axis scale, bounds, breaks, and thresholds"),
        keyboard_path: String::from("ui.graph.axes.open"),
        shortcut: None,
        primary: false,
        selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }),
        essential: true,
        enablement: if can_graph { ControlEnablement::enabled() } else { ControlEnablement::disabled("Create a graph sheet to format axes.") },
        action: WidgetAction::InvokeCommand { command_id: String::from("ui.graph.axes.open"), params: non_null_obj_with_sheet(target_gid.as_ref()) },
    });
    if ctx == ActiveToolbarContext::GraphSheet {
        items.push(ToolbarControlItem {
            id: "toolbar.pairwise",
            group: ToolbarGroup::GraphLayout,
            icon: ToolbarIconKind::Pairwise,
            label: tr_with_fallback(locale, "toolbar.pairwise", "Pairwise"),
            tooltip: String::from("Add statistical comparison significance brackets"),
            keyboard_path: String::from("ui.graph.pairwise.open"),
            shortcut: None,
            primary: false,
            selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::PairwiseComparisons { .. }),
            essential: true,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::InvokeCommand {
                command_id: String::from("ui.graph.pairwise.open"),
                params: non_null_obj_with_sheet(target_gid.as_ref()),
            },
        });
    }
    if matches!(ctx, ActiveToolbarContext::GraphSheet | ActiveToolbarContext::LayoutSheet) {
        items.push(ToolbarControlItem {
            id: "toolbar.rulers",
            group: ToolbarGroup::GraphLayout,
            icon: ToolbarIconKind::Rulers,
            label: tr_with_fallback(locale, "toolbar.rulers", "Rulers"),
            tooltip: String::from("Toggle calibrated physical print rulers"),
            keyboard_path: String::from("ui.rulers.toggle"),
            shortcut: None,
            primary: false,
            selected: app.canvas_dialogs.rulers_visible,
            essential: true,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::InvokeCommand {
                command_id: String::from("ui.rulers.toggle"),
                params: json!({ "visible": !app.canvas_dialogs.rulers_visible }),
            },
        });
    }
    if ctx == ActiveToolbarContext::LayoutSheet {
        items.push(ToolbarControlItem {
            id: "toolbar.alignment",
            group: ToolbarGroup::GraphLayout,
            icon: ToolbarIconKind::Align,
            label: tr_with_fallback(locale, "toolbar.alignment", "Align"),
            tooltip: String::from("Align plot origins across page layout panels"),
            keyboard_path: String::from("layout.alignOrigins"),
            shortcut: None,
            primary: false,
            selected: app.canvas_dialogs.page_layout.align_plot_origins,
            essential: true,
            enablement: ControlEnablement::enabled(),
            action: custom_toolbar_action("layout.align_origins"),
        });
    }
    if matches!(ctx, ActiveToolbarContext::InfoSheet | ActiveToolbarContext::GraphSheet | ActiveToolbarContext::LayoutSheet) {
        items.push(ToolbarControlItem {
            id: "toolbar.text_bold",
            group: ToolbarGroup::TextStyle,
            icon: ToolbarIconKind::Bold,
            label: tr_with_fallback(locale, "toolbar.text_bold", "Bold"),
            tooltip: String::from("Toggle bold title/callout typography"),
            keyboard_path: String::from("Cmd+B"),
            shortcut: Some(String::from("Cmd+B")),
            primary: false,
            selected: state.text_bold,
            essential: false,
            enablement: ControlEnablement::enabled(),
            action: custom_toolbar_action("text.toggle_bold"),
        });
        items.push(ToolbarControlItem {
            id: "toolbar.text_italic",
            group: ToolbarGroup::TextStyle,
            icon: ToolbarIconKind::Italic,
            label: tr_with_fallback(locale, "toolbar.text_italic", "Italic"),
            tooltip: String::from("Toggle italic title/callout typography"),
            keyboard_path: String::from("Cmd+I"),
            shortcut: Some(String::from("Cmd+I")),
            primary: false,
            selected: state.text_italic,
            essential: false,
            enablement: ControlEnablement::enabled(),
            action: custom_toolbar_action("text.toggle_italic"),
        });
    }
}

fn append_toolbar_output_items(app: &UiApp, locale: &str, items: &mut Vec<ToolbarControlItem>) {
    let doc = app.document();
    let target_vis = target_visual_sheet_id(doc);
    let can_visual = !doc.graph_sheets.is_empty() || !doc.layout_sheets.is_empty();

    items.push(ToolbarControlItem {
        id: "toolbar.palette",
        group: ToolbarGroup::Output,
        icon: ToolbarIconKind::Palette,
        label: tr_with_fallback(locale, "toolbar.palette", "Cmd+K"),
        tooltip: String::from("Open Command Palette"),
        keyboard_path: String::from("Cmd+K"),
        shortcut: Some(String::from("Cmd+K")),
        primary: false,
        selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette),
        essential: true,
        enablement: ControlEnablement::enabled(),
        action: WidgetAction::InvokeCommand { command_id: String::from("ui.palette.open"), params: json!({}) },
    });
    items.push(ToolbarControlItem {
        id: "toolbar.print",
        group: ToolbarGroup::Output,
        icon: ToolbarIconKind::Print,
        label: tr_with_fallback(locale, "toolbar.print", "Print"),
        tooltip: String::from("Open physical-scale Print Setup"),
        keyboard_path: String::from("Cmd+P"),
        shortcut: Some(String::from("Cmd+P")),
        primary: false,
        selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::Print { .. }),
        essential: false,
        enablement: if can_visual { ControlEnablement::enabled() } else { ControlEnablement::disabled("Create a graph or layout sheet to print.") },
        action: WidgetAction::InvokeCommand { command_id: String::from("ui.print.open"), params: non_null_obj_with_sheet(target_vis.as_ref()) },
    });
    let mut export_map = Map::new();
    if let Some(ref sid) = target_vis {
        export_map.insert(String::from("sheet_id"), Value::String(sid.as_str().to_owned()));
    }
    export_map.insert(String::from("format"), Value::String(String::from("svg")));
    items.push(ToolbarControlItem {
        id: "toolbar.export_svg",
        group: ToolbarGroup::Output,
        icon: ToolbarIconKind::Export,
        label: tr_with_fallback(locale, "toolbar.export_svg", "Export"),
        tooltip: String::from("Open publication SVG/PDF/PNG/TIFF export dialog"),
        keyboard_path: String::from("ui.export.figure.open"),
        shortcut: None,
        primary: false,
        selected: matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }),
        essential: true,
        enablement: if can_visual { ControlEnablement::enabled() } else { ControlEnablement::disabled("Create a graph or layout sheet to export.") },
        action: WidgetAction::InvokeCommand { command_id: String::from("ui.export.figure.open"), params: Value::Object(export_map) },
    });
    for (id, label, mode, mode_str, tip) in [
        ("toolbar.theme.light", "Light", UiThemeMode::Light, "light", "Switch to Light theme"),
        ("toolbar.theme.dark", "Dark", UiThemeMode::Dark, "dark", "Switch to Dark theme"),
        ("toolbar.theme.high_contrast", "HC", UiThemeMode::HighContrast, "high_contrast", "Switch to High Contrast theme"),
    ] {
        items.push(ToolbarControlItem {
            id,
            group: ToolbarGroup::Output,
            icon: ToolbarIconKind::Theme,
            label,
            tooltip: String::from(tip),
            keyboard_path: String::from("ui.theme.set"),
            shortcut: None,
            primary: false,
            selected: app.theme.mode == mode,
            essential: false,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::InvokeCommand { command_id: String::from("ui.theme.set"), params: json!({ "mode": mode_str }) },
        });
    }
}

fn build_contextual_toolbar_items(app: &UiApp, locale: &str, state: &ToolbarMenusViewState) -> Vec<ToolbarControlItem> {
    let ctx = active_toolbar_context(app.document());
    let mut items = Vec::with_capacity(20);
    append_toolbar_file_and_history_items(app, locale, &mut items);
    append_toolbar_clipboard_and_sheet_items(app, locale, ctx, &mut items);
    append_toolbar_analysis_items(app, locale, ctx, &mut items);
    append_toolbar_graph_and_text_items(app, locale, ctx, state, &mut items);
    append_toolbar_output_items(app, locale, &mut items);
    items
}

fn paint_toolbar_icon(
    builder: &mut LayoutBuilder<'_>,
    owner: &WidgetId,
    z_layer: ZLayer,
    icon: ToolbarIconKind,
    icon_rect: LayoutRectPx,
    color: Rgba8,
    accent: Rgba8,
) {
    let x = icon_rect.x;
    let y = icon_rect.y;
    let w = i32::try_from(icon_rect.width).unwrap_or(10);
    let h = i32::try_from(icon_rect.height).unwrap_or(10);
    let r = x + w;
    let b = y + h;
    let cx = x + w / 2;
    let cy = y + h / 2;

    match icon {
        ToolbarIconKind::New => {
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 1, y + 1, icon_rect.width.saturating_sub(2), icon_rect.height.saturating_sub(2)),
                color,
                1,
            );
            builder.paint_line(z_layer, Some(owner), [cx, y + 3], [cx, b - 3], accent, 1);
            builder.paint_line(z_layer, Some(owner), [x + 3, cy], [r - 3, cy], accent, 1);
        }
        ToolbarIconKind::Open => {
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 1, y + 2, (icon_rect.width / 2).max(3), 2), color);
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 1, y + 3, icon_rect.width.saturating_sub(2), icon_rect.height.saturating_sub(4)),
                color,
                1,
            );
        }
        ToolbarIconKind::Save => {
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 1, y + 1, icon_rect.width.saturating_sub(2), icon_rect.height.saturating_sub(2)),
                color,
                1,
            );
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 3, y + 2, icon_rect.width.saturating_sub(6).max(2), 3), accent);
        }
        ToolbarIconKind::Undo => {
            builder.paint_line(z_layer, Some(owner), [x + 2, cy], [r - 2, cy], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 2, cy], [x + 5, cy - 3], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 2, cy], [x + 5, cy + 3], color, 1);
        }
        ToolbarIconKind::Redo => {
            builder.paint_line(z_layer, Some(owner), [x + 2, cy], [r - 2, cy], color, 1);
            builder.paint_line(z_layer, Some(owner), [r - 2, cy], [r - 5, cy - 3], color, 1);
            builder.paint_line(z_layer, Some(owner), [r - 2, cy], [r - 5, cy + 3], color, 1);
        }
        ToolbarIconKind::Cut => {
            builder.paint_line(z_layer, Some(owner), [x + 2, y + 2], [r - 2, b - 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [r - 2, y + 2], [x + 2, b - 2], color, 1);
        }
        ToolbarIconKind::Copy => {
            let sz = icon_rect.width.saturating_sub(4).max(4);
            builder.paint_stroke(z_layer, Some(owner), LayoutRectPx::new(x + 1, y + 1, sz, sz), color, 1);
            builder.paint_stroke(z_layer, Some(owner), LayoutRectPx::new(x + 3, y + 3, sz, sz), color, 1);
        }
        ToolbarIconKind::Paste => {
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 2, y + 2, icon_rect.width.saturating_sub(4), icon_rect.height.saturating_sub(3)),
                color,
                1,
            );
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 4, y + 1, icon_rect.width.saturating_sub(8).max(2), 2), accent);
        }
        ToolbarIconKind::Duplicate => {
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 1, y + 3, icon_rect.width.saturating_sub(4), icon_rect.height.saturating_sub(4)),
                color,
                1,
            );
            builder.paint_line(z_layer, Some(owner), [x + 4, y + 1], [r - 1, y + 1], accent, 1);
            builder.paint_line(z_layer, Some(owner), [r - 1, y + 1], [r - 1, b - 3], accent, 1);
        }
        ToolbarIconKind::Freeze => {
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 2, cy - 1, icon_rect.width.saturating_sub(4), (icon_rect.height / 2).max(4)),
                color,
                1,
            );
            builder.paint_line(z_layer, Some(owner), [x + 4, cy - 1], [x + 4, y + 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [r - 4, cy - 1], [r - 4, y + 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 4, y + 2], [r - 4, y + 2], color, 1);
        }
        ToolbarIconKind::Analyze => {
            builder.paint_line(z_layer, Some(owner), [x + 1, b - 2], [cx, cy], color, 1);
            builder.paint_line(z_layer, Some(owner), [cx, cy], [r - 1, y + 2], color, 1);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 1, b - 4, 3, 3), color);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(r - 4, y + 1, 3, 3), color);
        }
        ToolbarIconKind::Parameters => {
            builder.paint_line(z_layer, Some(owner), [x + 1, y + 3], [r - 1, y + 3], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 1, cy], [r - 1, cy], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 1, b - 3], [r - 1, b - 3], color, 1);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 3, y + 2, 3, 3), accent);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(r - 5, cy - 1, 3, 3), accent);
        }
        ToolbarIconKind::DataAnalyzed => {
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 1, y + 2, icon_rect.width.saturating_sub(3), icon_rect.height.saturating_sub(4)),
                color,
                1,
            );
            builder.paint_line(z_layer, Some(owner), [cx, y + 2], [cx, b - 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 1, cy], [r - 2, cy], accent, 1);
        }
        ToolbarIconKind::Interpret => {
            builder.paint_line(z_layer, Some(owner), [x + 1, cy], [cx - 1, b - 2], accent, 1);
            builder.paint_line(z_layer, Some(owner), [cx - 1, b - 2], [r - 2, y + 3], accent, 1);
            builder.paint_line(z_layer, Some(owner), [x + 1, y + 2], [r - 2, y + 2], color, 1);
        }
        ToolbarIconKind::GraphType => {
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 2, cy, 2, (b - cy - 1).max(2) as u32), color);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(cx - 1, y + 2, 2, (b - y - 3).max(3) as u32), accent);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(r - 4, cy - 2, 2, (b - cy + 1).max(3) as u32), color);
        }
        ToolbarIconKind::Axes => {
            builder.paint_line(z_layer, Some(owner), [x + 3, y + 1], [x + 3, b - 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 2, b - 3], [r - 1, b - 3], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 1, cy], [x + 3, cy], accent, 1);
            builder.paint_line(z_layer, Some(owner), [cx + 1, b - 3], [cx + 1, b - 1], accent, 1);
        }
        ToolbarIconKind::Pairwise => {
            builder.paint_line(z_layer, Some(owner), [x + 2, cy], [x + 2, y + 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 2, y + 2], [r - 2, y + 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [r - 2, y + 2], [r - 2, cy], color, 1);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(cx - 1, y, 2, 2), accent);
        }
        ToolbarIconKind::Rulers => {
            builder.paint_stroke(z_layer, Some(owner), LayoutRectPx::new(x + 1, y + 1, icon_rect.width.saturating_sub(2), 4), color, 1);
            builder.paint_stroke(z_layer, Some(owner), LayoutRectPx::new(x + 1, y + 1, 4, icon_rect.height.saturating_sub(2)), color, 1);
        }
        ToolbarIconKind::Align => {
            builder.paint_line(z_layer, Some(owner), [x + 2, y + 1], [x + 2, b - 1], accent, 1);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 4, y + 2, icon_rect.width.saturating_sub(5), 3), color);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 4, cy + 1, icon_rect.width.saturating_sub(7).max(3), 3), color);
        }
        ToolbarIconKind::Bold => {
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 2, y + 2, 2, icon_rect.height.saturating_sub(4)), color);
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 2, y + 2, icon_rect.width.saturating_sub(4), icon_rect.height.saturating_sub(4)),
                color,
                1,
            );
        }
        ToolbarIconKind::Italic => {
            builder.paint_line(z_layer, Some(owner), [cx + 2, y + 2], [cx - 2, b - 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [cx, y + 2], [r - 2, y + 2], color, 1);
            builder.paint_line(z_layer, Some(owner), [x + 2, b - 2], [cx, b - 2], color, 1);
        }
        ToolbarIconKind::Palette => {
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 1, y + 2, icon_rect.width.saturating_sub(2), icon_rect.height.saturating_sub(4)),
                color,
                1,
            );
            builder.paint_line(z_layer, Some(owner), [x + 3, y + 4], [cx, cy], accent, 1);
            builder.paint_line(z_layer, Some(owner), [cx, cy], [x + 3, b - 4], accent, 1);
        }
        ToolbarIconKind::Print => {
            builder.paint_stroke(
                z_layer,
                Some(owner),
                LayoutRectPx::new(x + 1, y + 4, icon_rect.width.saturating_sub(2), icon_rect.height.saturating_sub(6)),
                color,
                1,
            );
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 3, y + 1, icon_rect.width.saturating_sub(6).max(3), 3), color);
        }
        ToolbarIconKind::Export => {
            builder.paint_line(z_layer, Some(owner), [cx, y + 1], [cx, b - 3], accent, 1);
            builder.paint_line(z_layer, Some(owner), [cx, y + 1], [cx - 3, y + 4], accent, 1);
            builder.paint_line(z_layer, Some(owner), [cx, y + 1], [cx + 3, y + 4], accent, 1);
            builder.paint_line(z_layer, Some(owner), [x + 2, b - 2], [r - 2, b - 2], color, 1);
        }
        ToolbarIconKind::Theme => {
            let sz = icon_rect.width.saturating_sub(2).max(4);
            let half = (sz / 2).max(2);
            builder.paint_stroke(z_layer, Some(owner), LayoutRectPx::new(x + 1, y + 1, sz, sz), color, 1);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 1, y + 1, half, sz), accent);
        }
        ToolbarIconKind::Overflow => {
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(x + 1, cy - 1, 2, 2), color);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(cx - 1, cy - 1, 2, 2), color);
            builder.paint_fill(z_layer, Some(owner), LayoutRectPx::new(r - 3, cy - 1, 2, 2), color);
        }
    }
}

fn emit_icon_toolbar_button(builder: &mut LayoutBuilder<'_>, item: &ToolbarControlItem, rect: LayoutRectPx, text_style: &UiTextStyle) {
    let mut state = widget_state(builder, item.id, item.enablement.clone());
    let bg = if !state.enablement.enabled {
        builder.palette.panel
    } else if item.primary {
        builder.palette.accent
    } else if item.selected || state.hovered || state.active {
        builder.palette.header
    } else {
        builder.palette.surface
    };
    let fg = if !state.enablement.enabled {
        builder.palette.muted
    } else if item.primary {
        Rgba8::WHITE
    } else {
        builder.palette.text
    };
    let icon_accent = if !state.enablement.enabled {
        builder.palette.muted
    } else if item.primary {
        Rgba8::WHITE
    } else {
        builder.palette.accent
    };
    let border = if item.selected || state.focused { builder.palette.accent } else { builder.palette.border };

    let mut a11y = AccessibilityMeta::new(WidgetRole::Button, item.label, FocusScope::Workspace).with_keyboard_path(&item.keyboard_path);
    a11y.selected = Some(item.selected);
    a11y.shortcut = item.shortcut.clone();
    a11y.value_text = Some(item.tooltip.clone());

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(item.id),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y,
        state: state.clone(),
        action: if state.enablement.enabled { item.action.clone() } else { WidgetAction::NoOp },
    });

    builder.paint_fill(ZLayer::Workspace, Some(&wid), rect, bg);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), rect, border, 1);
    maybe_paint_focus_ring(builder, ZLayer::Workspace, &wid, rect, state.focused);

    let is_rtl = builder.tree.is_rtl;
    let icon_sz = rect.height.saturating_sub(10).clamp(8, 12);
    let icon_y = rect.y + i32::try_from((rect.height.saturating_sub(icon_sz)) / 2).unwrap_or(0);
    let icon_rect = if is_rtl {
        LayoutRectPx::new(rect.right() - 4 - i32::try_from(icon_sz).unwrap_or(10), icon_y, icon_sz, icon_sz)
    } else {
        LayoutRectPx::new(rect.x + 4, icon_y, icon_sz, icon_sz)
    };
    paint_toolbar_icon(builder, &wid, ZLayer::Workspace, item.icon, icon_rect, fg, icon_accent);

    let (text_x, text_w) = if is_rtl {
        let tx = rect.x + 2;
        let tw = u32::try_from((icon_rect.x - 3 - tx).max(0)).unwrap_or(0);
        (tx, tw)
    } else {
        let tx = icon_rect.right() + 3;
        let tw = u32::try_from((rect.right() - tx).max(0)).unwrap_or(0);
        (tx, tw)
    };
    if text_w > 6 {
        let text_rect = LayoutRectPx::new(text_x, rect.y, text_w, rect.height);
        state.ellipsized = paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: text_rect,
                pad_x: 2,
                text: item.label,
                style: text_style,
                color: fg,
                scroll_container: None,
            },
        );
    }
}

fn measure_toolbar_item_width(builder: &LayoutBuilder<'_>, item: &ToolbarControlItem, text_style: &UiTextStyle, icon_sz: u32) -> u32 {
    let measured = builder.measurer.measure(item.label, text_style, &builder.tree.locale).width_px.ceil() as u32;
    let chrome_pad = 4 + icon_sz + 3 + 6;
    (measured + chrome_pad).clamp(36, 124)
}

fn layout_toolbar_row(
    builder: &LayoutBuilder<'_>,
    items: &[ToolbarControlItem],
    avail_w: u32,
    text_style: &UiTextStyle,
    icon_sz: u32,
) -> (Vec<(ToolbarControlItem, u32)>, Vec<ToolbarControlItem>) {
    let overflow_reserve = 56u32;
    let mut widths: Vec<(ToolbarControlItem, u32, bool)> = items
        .iter()
        .cloned()
        .map(|item| {
            let w = measure_toolbar_item_width(builder, &item, text_style, icon_sz);
            let keep = !item.essential;
            (item, w, keep)
        })
        .collect();

    let total_needed = |list: &[(ToolbarControlItem, u32, bool)], with_overflow: bool| -> u32 {
        let mut sum = 0u32;
        let mut prev_group: Option<ToolbarGroup> = None;
        for (it, w, _) in list {
            if prev_group.is_some_and(|g| g != it.group) {
                sum += 5;
            } else if prev_group.is_some() {
                sum += 2;
            }
            sum += *w;
            prev_group = Some(it.group);
        }
        if with_overflow {
            sum += overflow_reserve;
        }
        sum
    };

    let mut overflow = Vec::new();
    while total_needed(&widths, !overflow.is_empty()) > avail_w {
        if let Some(pos) = widths.iter().rposition(|(_, _, secondary)| *secondary) {
            let (removed, _, _) = widths.remove(pos);
            overflow.push(removed);
        } else if widths.len() > 1 {
            // Preserve leading controls and rightmost palette/export if an ultra-narrow width forces truncation.
            let drop_idx = widths
                .iter()
                .rposition(|(it, _, _)| !matches!(it.id, "toolbar.export_svg" | "toolbar.palette" | "toolbar.analyze" | "toolbar.axes"))
                .unwrap_or(widths.len() - 1);
            let (removed, _, _) = widths.remove(drop_idx);
            overflow.push(removed);
        } else {
            break;
        }
    }
    overflow.reverse();
    let kept = widths.into_iter().map(|(it, w, _)| (it, w)).collect();
    (kept, overflow)
}

fn emit_menu_bar_button(
    builder: &mut LayoutBuilder<'_>,
    id: &str,
    rect: LayoutRectPx,
    label: &str,
    keyboard_path: &str,
    selected: bool,
    style: &UiTextStyle,
) {
    let state = widget_state(builder, id, ControlEnablement::enabled());
    let bg = if selected || state.hovered || state.active { builder.palette.header } else { builder.palette.surface };
    let border = if selected || state.focused { builder.palette.accent } else { builder.palette.border };
    let mut a11y = AccessibilityMeta::new(WidgetRole::Button, label, FocusScope::Workspace).with_keyboard_path(keyboard_path);
    a11y.selected = Some(selected);
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(id),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y,
        state: state.clone(),
        action: WidgetAction::MenuOpen(id.to_owned()),
    });
    builder.paint_fill(ZLayer::Workspace, Some(&wid), rect, bg);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), rect, border, 1);
    maybe_paint_focus_ring(builder, ZLayer::Workspace, &wid, rect, state.focused);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec { z_layer: ZLayer::Workspace, rect, pad_x: 6, text: label, style, color: builder.palette.text, scroll_container: None },
    );
}

fn emit_menu_bar_and_badge(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx, two_rows: bool, menu_y: i32, menu_h: u32) -> i32 {
    let is_rtl = builder.tree.is_rtl;
    let mut cur_x = if is_rtl { rect.right() - 8 } else { rect.x + 8 };

    let max_row_font = f64::from(menu_h.saturating_sub(5)).max(8.5) * 0.78;
    let menu_size_px = (11.5 * builder.scale()).min(max_row_font).clamp(8.5, 18.0);
    let menu_style = UiTextStyle::ui_sans(menu_size_px);
    let scale = builder.scale();
    let max_bw = (f64::from(96u32) * scale).round().clamp(96.0, 240.0) as u32;
    let min_bw = (f64::from(38u32) * scale).round().clamp(38.0, 96.0) as u32;
    let pad_bw = (f64::from(14u32) * scale).round().clamp(14.0, 32.0) as u32;
    let menus = crate::menus::menu_bar_tooltips(builder.app);
    for (idx, sec) in menus.iter().enumerate() {
        let measured = builder.measurer.measure(sec.title, &menu_style, &builder.tree.locale).width_px.ceil() as u32;
        let bw = (measured + pad_bw).clamp(min_bw, max_bw);
        let btn_x = if is_rtl { cur_x - i32::try_from(bw).unwrap_or(44) } else { cur_x };
        if if is_rtl { btn_x < rect.x + 8 } else { cur_x + i32::try_from(bw).unwrap_or(44) > rect.right() - 8 } {
            break;
        }
        let selected = builder.app.interaction.open_menu.as_deref() == Some(sec.menu_key);
        let kpath = format!("Alt+{}", idx + 1);
        emit_menu_bar_button(builder, sec.menu_key, LayoutRectPx::new(btn_x, menu_y, bw, menu_h), sec.title, &kpath, selected, &menu_style);
        cur_x = if is_rtl { cur_x - i32::try_from(bw + 3).unwrap_or(46) } else { cur_x + i32::try_from(bw + 3).unwrap_or(46) };
    }

    let ctx = active_toolbar_context(builder.app.document());
    let remaining_menu_w = if is_rtl { cur_x - (rect.x + 8) } else { rect.right() - cur_x };
    if two_rows && remaining_menu_w > 110 {
        let badge = format!("[{}]", ctx.badge());
        let bw = (builder.measurer.measure(&badge, &menu_style, &builder.tree.locale).width_px.ceil() as u32 + 12).clamp(52, 92);
        let bx = if is_rtl { rect.x + 8 } else { rect.right() - i32::try_from(bw + 8).unwrap_or(72) };
        let fits_badge = if is_rtl { bx + i32::try_from(bw + 8).unwrap_or(72) < cur_x } else { bx > cur_x + 8 };
        if fits_badge {
            let b_rect = LayoutRectPx::new(bx, menu_y, bw, menu_h);
            let b_wid = builder.push_node(NodeSpec {
                id: WidgetId::from("toolbar.context_badge"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: b_rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, badge.clone(), FocusScope::Workspace),
                state: WidgetVisualState::default(),
                action: WidgetAction::NoOp,
            });
            paint_text_fitted(
                builder,
                &b_wid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: b_rect,
                    pad_x: 4,
                    text: &badge,
                    style: &menu_style,
                    color: builder.palette.muted,
                    scroll_container: None,
                },
            );
        }
    }
    cur_x
}

fn emit_contextual_toolbar_strip(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx, tb_x_start: i32, avail_w: u32, tb_y: i32, tb_h: u32) {
    let is_rtl = builder.tree.is_rtl;
    let max_tb_font = f64::from(tb_h.saturating_sub(4)).max(8.5) * 0.78;
    let tb_size_px = (10.5 * builder.scale()).min(max_tb_font).clamp(8.5, 17.0);
    let tb_style = UiTextStyle::ui_sans(tb_size_px);
    let icon_sz = tb_h.saturating_sub(10).clamp(8, 12);
    let locale = builder.tree.locale.clone();
    let shard_state = &builder.app.view_shards.toolbar_menus;

    let all_items = build_contextual_toolbar_items(builder.app, &locale, shard_state);
    let (visible_items, overflow_items) = layout_toolbar_row(builder, &all_items, avail_w, &tb_style, icon_sz);

    let mut tb_x = tb_x_start;
    let mut prev_group: Option<ToolbarGroup> = None;
    for (item, bw) in &visible_items {
        if let Some(pg) = prev_group {
            if pg != item.group {
                let div_x = if is_rtl { tb_x - 3 } else { tb_x + 2 };
                builder.paint_line(
                    ZLayer::Workspace,
                    None,
                    [div_x, tb_y + 3],
                    [div_x, tb_y + i32::try_from(tb_h).unwrap_or(20) - 3],
                    builder.palette.border,
                    1,
                );
                tb_x += if is_rtl { -5 } else { 5 };
            } else {
                tb_x += if is_rtl { -2 } else { 2 };
            }
        }
        let item_x = if is_rtl { tb_x - i32::try_from(*bw).unwrap_or(44) } else { tb_x };
        emit_icon_toolbar_button(builder, item, LayoutRectPx::new(item_x, tb_y, *bw, tb_h), &tb_style);
        tb_x += if is_rtl { -i32::try_from(*bw).unwrap_or(44) } else { i32::try_from(*bw).unwrap_or(44) };
        prev_group = Some(item.group);
    }

    let can_overflow =
        if is_rtl { !overflow_items.is_empty() && tb_x - 44 >= rect.x + 6 } else { !overflow_items.is_empty() && tb_x + 44 <= rect.right() - 6 };
    if can_overflow {
        let ov_item = ToolbarControlItem {
            id: "toolbar.overflow",
            group: ToolbarGroup::Output,
            icon: ToolbarIconKind::Overflow,
            label: tr_with_fallback(&locale, "toolbar.overflow", "More"),
            tooltip: format!("{} hidden toolbar command(s)", overflow_items.len()),
            keyboard_path: String::from("toolbar.overflow"),
            shortcut: None,
            primary: false,
            selected: shard_state.overflow_open,
            essential: true,
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("toolbar_menus"), payload: String::from("overflow.toggle") },
        };
        let max_ov_w = if is_rtl {
            u32::try_from((tb_x - 3 - (rect.x + 6)).max(36)).unwrap_or(52)
        } else {
            u32::try_from((rect.right() - 6 - tb_x - 3).max(36)).unwrap_or(52)
        };
        let ov_w = measure_toolbar_item_width(builder, &ov_item, &tb_style, icon_sz).min(max_ov_w);
        let ov_x = if is_rtl { tb_x - 3 - i32::try_from(ov_w).unwrap_or(44) } else { tb_x + 3 };
        emit_icon_toolbar_button(builder, &ov_item, LayoutRectPx::new(ov_x, tb_y, ov_w, tb_h), &tb_style);
    }
}

/// Build the top menu bar + contextual icon-and-label toolbar strip inside `rect`.
pub fn build_toolbar_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.panel);
    builder.paint_line(ZLayer::Workspace, None, [rect.x, rect.bottom() - 1], [rect.right(), rect.bottom() - 1], builder.palette.border, 1);

    if rect.width < 120 || rect.height < 14 {
        return;
    }

    let is_rtl = builder.tree.is_rtl;
    builder.push_clip(rect);
    let two_rows = rect.height >= 40;
    let menu_h = if two_rows { (rect.height / 2).saturating_sub(3).max(14) } else { rect.height.saturating_sub(8).clamp(14, 28) };
    let menu_y = if two_rows { rect.y + 2 } else { rect.y + i32::try_from((rect.height.saturating_sub(menu_h)) / 2).unwrap_or(0) };
    let cur_x = emit_menu_bar_and_badge(builder, rect, two_rows, menu_y, menu_h);

    let tb_y = if two_rows { rect.y + i32::try_from(rect.height / 2 + 1).unwrap_or(23) } else { menu_y };
    let tb_h = if two_rows { rect.height.saturating_sub(rect.height / 2 + 3).max(14) } else { menu_h };
    let (tb_x_start, avail_w) = if is_rtl {
        let right_edge = if two_rows { rect.right() - 8 } else { cur_x - 6 };
        (right_edge, u32::try_from((right_edge - (rect.x + 8)).max(0)).unwrap_or(0))
    } else {
        let left_edge = if two_rows { rect.x + 8 } else { cur_x + 6 };
        (left_edge, u32::try_from((rect.right() - 8 - left_edge).max(0)).unwrap_or(0))
    };
    if avail_w >= 36 {
        emit_contextual_toolbar_strip(builder, rect, tb_x_start, avail_w, tb_y, tb_h);
    }
    builder.pop_clip();
}

fn menu_item_contextual_enablement(app: &UiApp, command_id: &str, registry_enabled: bool, label: &str) -> ControlEnablement {
    if !registry_enabled {
        return ControlEnablement::disabled(format!("{label} is unavailable in this context."));
    }
    let doc = app.document();
    let active_ds = active_data_sheet(doc);
    let has_any_sheet = doc.active_sheet_id.is_some();
    match command_id {
        "sheet.duplicate" | "sheet.delete" | "sheet.clonePipeline" => {
            if has_any_sheet {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled("Select a sheet first.")
            }
        }
        "sheet.freeze" => match active_ds {
            Some(ds) if !ds.frozen => ControlEnablement::enabled(),
            Some(_) => ControlEnablement::disabled("Data sheet is already frozen."),
            None => ControlEnablement::disabled("Select a data sheet to freeze."),
        },
        "sheet.unfreeze" => match active_ds {
            Some(ds) if ds.frozen => ControlEnablement::enabled(),
            _ => ControlEnablement::disabled("Select a frozen data sheet to unfreeze."),
        },
        "table.excludeCells" | "table.clearCells" => match active_ds {
            Some(ds) if !ds.frozen => ControlEnablement::enabled(),
            Some(_) => ControlEnablement::disabled("Unfreeze the data sheet before editing cells."),
            None => ControlEnablement::disabled("Select a data sheet first."),
        },
        "export.svg" | "export.pdf" | "export.png" | "export.tiff" | "graph.exportSvg" => {
            if !doc.graph_sheets.is_empty() || !doc.layout_sheets.is_empty() {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled("Create a graph or layout sheet to export.")
            }
        }
        "layout.alignOrigins" => {
            if doc.layout_sheets.is_empty() {
                ControlEnablement::disabled("Create a page layout sheet first.")
            } else {
                ControlEnablement::enabled()
            }
        }
        _ => ControlEnablement::enabled(),
    }
}

fn resolve_menu_item_action(app: &UiApp, command_id: &str) -> WidgetAction {
    let doc = app.document();
    let active_sid = doc.active_sheet_id.as_ref();
    let target_gid = target_graph_id(doc);
    let target_vis = target_visual_sheet_id(doc);
    match command_id {
        "project.open" => WidgetAction::InvokeCommand { command_id: String::from("project.open"), params: json!({}) },
        "project.save" => WidgetAction::InvokeCommand { command_id: String::from("project.save"), params: json!({ "return_bytes": true }) },
        "graph.setAxes" => {
            WidgetAction::InvokeCommand { command_id: String::from("ui.graph.axes.open"), params: non_null_obj_with_sheet(target_gid.as_ref()) }
        }
        "graph.applyStyleFrom" | "ui.applyStyleFrom.open" => {
            WidgetAction::InvokeCommand { command_id: String::from("ui.applyStyleFrom.open"), params: non_null_obj_with_sheet(target_gid.as_ref()) }
        }
        "sheet.duplicate" | "sheet.freeze" | "sheet.unfreeze" | "sheet.select" | "sheet.duplicateFamily" | "sheet.clonePipeline" => {
            WidgetAction::InvokeCommand { command_id: command_id.to_owned(), params: non_null_obj_with_sheet(active_sid) }
        }
        "sheet.delete" => {
            let mut map = Map::new();
            if let Some(sid) = active_sid {
                map.insert(String::from("sheet_id"), Value::String(sid.as_str().to_owned()));
            }
            map.insert(String::from("cascade"), Value::Bool(false));
            WidgetAction::InvokeCommand { command_id: String::from("sheet.delete"), params: Value::Object(map) }
        }
        "export.svg" | "export.pdf" | "export.png" | "export.tiff" | "graph.exportSvg" => {
            let mut map = Map::new();
            if let Some(ref sid) = target_vis {
                map.insert(String::from("sheet_id"), Value::String(sid.as_str().to_owned()));
            }
            WidgetAction::InvokeCommand { command_id: command_id.to_owned(), params: Value::Object(map) }
        }
        "layout.alignOrigins" => {
            let ls_id = doc.active_sheet_id.as_ref().filter(|id| doc.layout_sheet(id).is_some()).or_else(|| doc.layout_sheets.first().map(|l| &l.id));
            let next_align = !app.canvas_dialogs.page_layout.align_plot_origins;
            let mut map = Map::new();
            if let Some(sid) = ls_id {
                map.insert(String::from("sheet_id"), Value::String(sid.as_str().to_owned()));
            }
            map.insert(String::from("align"), Value::Bool(next_align));
            map.insert(String::from("normalize_bounds"), Value::Bool(true));
            WidgetAction::InvokeCommand { command_id: String::from("layout.alignOrigins"), params: Value::Object(map) }
        }
        "table.excludeCells" => {
            let mut map = Map::new();
            if let Some(ds) = active_data_sheet(doc)
                && let Some(range) = selection_bounding_range(&active_selection_coords_for(app, ds))
            {
                map.insert(String::from("sheet_id"), Value::String(ds.id.as_str().to_owned()));
                map.insert(String::from("range"), range_to_json(range));
                map.insert(String::from("excluded"), Value::Bool(true));
            }
            WidgetAction::InvokeCommand { command_id: String::from("table.excludeCells"), params: Value::Object(map) }
        }
        "table.clearCells" => {
            let mut map = Map::new();
            if let Some(ds) = active_data_sheet(doc)
                && let Some(range) = selection_bounding_range(&active_selection_coords_for(app, ds))
            {
                map.insert(String::from("sheet_id"), Value::String(ds.id.as_str().to_owned()));
                map.insert(String::from("range"), range_to_json(range));
            }
            WidgetAction::InvokeCommand { command_id: String::from("table.clearCells"), params: Value::Object(map) }
        }
        "ui.analyze.open" => WidgetAction::InvokeCommand {
            command_id: String::from("ui.analyze.open"),
            params: non_null_obj_with_sheet(active_data_sheet(doc).map(|ds| &ds.id).or(active_sid)),
        },
        _ => WidgetAction::InvokeCommand { command_id: command_id.to_owned(), params: json!({}) },
    }
}

/// Build the open top-level menu dropdown popup anchored below `toolbar_rect`.
pub fn build_open_menu_popup(builder: &mut LayoutBuilder<'_>, menu_key: &str, toolbar_rect: LayoutRectPx) {
    if let Some(target) = menu_key.strip_prefix("context:") {
        build_open_context_menu_popup(builder, target);
        return;
    }
    let sections = crate::menus::menu_bar_tooltips(builder.app);
    let Some(section) = sections.into_iter().find(|s| s.menu_key == menu_key) else {
        return;
    };
    let mut items = section.items;
    if menu_key == "menu.graph" {
        if !items.iter().any(|it| it.command_id == "ui.applyStyleFrom.open")
            && let Some(tip) = command_tooltip(builder.app, "ui.applyStyleFrom.open")
        {
            let pos = items.iter().position(|it| it.command_id == "graph.applyStyleFrom").unwrap_or(0).min(12);
            items.insert(pos, tip);
        }
    } else if menu_key == "menu.view" {
        items.retain(|it| it.command_id != "ui.applyStyleFrom.open");
    }
    let scale = builder.scale();
    let su = |px: u32| -> u32 { (f64::from(px) * scale).round().clamp(f64::from(px) * 0.75, f64::from(px) * 3.0) as u32 };
    let pop_w = su(256).min(builder.tree.metrics.physical_width_px().saturating_sub(12).max(120));
    let anchor_x = builder.tree.node(menu_key).map_or(toolbar_rect.x + 8, |n| {
        if builder.tree.is_rtl { n.bounds.rect_px.right() - i32::try_from(pop_w).unwrap_or(256) } else { n.bounds.rect_px.x }
    });
    let row_h = su(22).max(20);
    let max_h = builder.tree.metrics.physical_height_px().saturating_sub(toolbar_rect.bottom() as u32 + 12).clamp(60, su(320));
    let content_h = (items.len() as u32).saturating_mul(row_h) + su(6);
    let pop_h = content_h.min(max_h);
    let clamped_x = anchor_x.min(i32::try_from(builder.tree.metrics.physical_width_px().saturating_sub(pop_w + 6)).unwrap_or(0)).max(2);
    let pop_rect = LayoutRectPx::new(clamped_x, toolbar_rect.bottom(), pop_w, pop_h);

    let scroll_id = format!("scroll.menu.{menu_key}");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), pop_rect, [pop_w, content_h], scroll_off, ScrollAxes::Vertical);

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("popup.{menu_key}")),
        parent: None,
        z_layer: ZLayer::DropdownMenu,
        rect_px: pop_rect,
        a11y: labeled_meta(WidgetRole::ContextMenu, format!("{} Menu", section.title), FocusScope::PopupMenu, menu_key),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::DropdownMenu, Some(&wid), pop_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::DropdownMenu, Some(&wid), pop_rect, builder.palette.accent, 1);

    let sb_reserve = scrollbar_thickness_px(scale).saturating_add(4);
    let clip = LayoutRectPx::new(pop_rect.x + 2, pop_rect.y + 2, pop_w.saturating_sub(sb_reserve), pop_h.saturating_sub(4));
    builder.push_clip(clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    for (idx, tip) in items.iter().enumerate() {
        let ry = clip.y + i32::try_from(idx as u32 * row_h).unwrap_or(0) - y_scroll;
        if ry + i32::try_from(row_h).unwrap_or(22) <= clip.y || ry >= clip.bottom() {
            continue;
        }
        let item_rect = LayoutRectPx::new(clip.x, ry, clip.width, row_h);
        let item_id = format!("menu.{}", tip.command_id);
        let en = menu_item_contextual_enablement(builder.app, tip.command_id, tip.enabled, tip.localized_label);
        let st = widget_state(builder, &item_id, en.clone());
        let mut a11y = labeled_meta(
            WidgetRole::MenuItem,
            tip.localized_label,
            FocusScope::PopupMenu,
            if tip.shortcut.is_empty() { tip.command_id } else { tip.shortcut.as_str() },
        );
        a11y.shortcut = (!tip.shortcut.is_empty()).then(|| tip.shortcut.clone());
        let resolved_action = if en.enabled { resolve_menu_item_action(builder.app, tip.command_id) } else { WidgetAction::NoOp };
        let item_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(item_id),
            parent: Some(wid.clone()),
            z_layer: ZLayer::DropdownMenu,
            rect_px: item_rect,
            a11y,
            state: st.clone(),
            action: resolved_action,
        });
        let bg = if st.hovered && en.enabled { builder.palette.header } else { builder.palette.surface };
        let fg = if en.enabled { builder.palette.text } else { builder.palette.muted };
        builder.paint_fill(ZLayer::DropdownMenu, Some(&item_wid), item_rect, bg);
        paint_text_fitted(
            builder,
            &item_wid,
            FittedTextSpec {
                z_layer: ZLayer::DropdownMenu,
                rect: item_rect,
                pad_x: 6,
                text: tip.localized_label,
                style: &UiTextStyle::ui_sans(size_px),
                color: fg,
                scroll_container: Some(&scroll_id),
            },
        );
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::DropdownMenu, &scroll_meta, Some(wid));
}

pub(crate) fn build_toolbar_overflow_popup(builder: &mut LayoutBuilder<'_>) {
    let locale = builder.tree.locale.clone();
    let shard_state = &builder.app.view_shards.toolbar_menus;
    let toolbar_rect = LayoutRectPx::from_workspace_rect(builder.tree.frame.toolbar);
    let avail_w = toolbar_rect.width.saturating_sub(16);
    let tb_h = if toolbar_rect.height >= 40 {
        toolbar_rect.height.saturating_sub(toolbar_rect.height / 2 + 3).max(14)
    } else {
        toolbar_rect.height.saturating_sub(8).max(14)
    };
    let max_tb_font = f64::from(tb_h.saturating_sub(6)).max(8.5);
    let tb_size_px = (10.5 * builder.scale()).min(max_tb_font).clamp(8.5, 20.0);
    let tb_style = UiTextStyle::ui_sans(tb_size_px);
    let icon_sz = tb_h.saturating_sub(10).clamp(8, 12);
    let all_items = build_contextual_toolbar_items(builder.app, &locale, shard_state);
    let (_, overflow_items) = layout_toolbar_row(builder, &all_items, avail_w, &tb_style, icon_sz);
    if overflow_items.is_empty() {
        return;
    }

    let anchor_rect = builder
        .tree
        .node("toolbar.overflow")
        .map_or(LayoutRectPx::new(toolbar_rect.right() - 210, toolbar_rect.bottom(), 56, 20), |n| n.bounds.rect_px);
    let pop_w = 212u32;
    let row_h = 22u32;
    let pop_h = (overflow_items.len() as u32).saturating_mul(row_h) + 6;
    let max_x = i32::try_from(builder.tree.metrics.physical_width_px().saturating_sub(pop_w + 6)).unwrap_or(0);
    let raw_x = if builder.tree.is_rtl { anchor_rect.right() - i32::try_from(pop_w).unwrap_or(212) } else { anchor_rect.x };
    let pop_x = raw_x.clamp(2, max_x.max(2));
    let pop_rect = LayoutRectPx::new(pop_x, toolbar_rect.bottom(), pop_w, pop_h);

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from("popup.toolbar_overflow"),
        parent: None,
        z_layer: ZLayer::DropdownMenu,
        rect_px: pop_rect,
        a11y: labeled_meta(WidgetRole::ContextMenu, "Toolbar Overflow", FocusScope::PopupMenu, "toolbar.overflow"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::DropdownMenu, Some(&wid), pop_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::DropdownMenu, Some(&wid), pop_rect, builder.palette.accent, 1);

    let text_size = (11.5 * builder.scale()).clamp(9.0, 20.0);
    let text_style = UiTextStyle::ui_sans(text_size);
    for (idx, item) in overflow_items.iter().enumerate() {
        let ry = pop_rect.y + 3 + i32::try_from(idx as u32 * row_h).unwrap_or(0);
        let item_rect = LayoutRectPx::new(pop_rect.x + 3, ry, pop_w - 6, row_h);
        let st = widget_state(builder, item.id, item.enablement.clone());
        let mut a11y = labeled_meta(WidgetRole::MenuItem, item.label, FocusScope::PopupMenu, &item.keyboard_path);
        a11y.selected = Some(item.selected);
        let item_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(item.id),
            parent: Some(wid.clone()),
            z_layer: ZLayer::DropdownMenu,
            rect_px: item_rect,
            a11y,
            state: st.clone(),
            action: if st.enablement.enabled { item.action.clone() } else { WidgetAction::NoOp },
        });
        let bg = if st.hovered && st.enablement.enabled { builder.palette.header } else { builder.palette.surface };
        let fg = if st.enablement.enabled { builder.palette.text } else { builder.palette.muted };
        builder.paint_fill(ZLayer::DropdownMenu, Some(&item_wid), item_rect, bg);
        paint_text_fitted(
            builder,
            &item_wid,
            FittedTextSpec {
                z_layer: ZLayer::DropdownMenu,
                rect: item_rect,
                pad_x: 6,
                text: item.label,
                style: &text_style,
                color: fg,
                scroll_container: None,
            },
        );
    }
}

fn ctx_custom_item(
    id: &'static str,
    label: &str,
    keyboard_path: &'static str,
    shortcut: Option<&'static str>,
    enablement: ControlEnablement,
    payload: &str,
) -> ContextMenuItemSpec {
    ContextMenuItemSpec { id, label: label.to_owned(), keyboard_path, shortcut, enablement, action: custom_toolbar_action(payload) }
}

fn enable_if(cond: bool, disabled_reason: &str) -> ControlEnablement {
    if cond { ControlEnablement::enabled() } else { ControlEnablement::disabled(disabled_reason) }
}

fn build_column_header_ctx_items(app: &UiApp, locale: &str) -> (&'static str, Vec<ContextMenuItemSpec>) {
    let active_ds = active_data_sheet(app.document());
    let ds_unfrozen = active_ds.is_some_and(|ds| !ds.frozen);
    let is_mv = active_ds.is_some_and(|ds| matches!(ds.archetype, TableArchetype::MultipleVariables(_)));
    let items = vec![
        ctx_custom_item(
            "ctx.col.copy",
            tr_with_fallback(locale, "toolbar.copy", "Copy Column"),
            "ctx.col.copy",
            Some("Cmd+C"),
            enable_if(active_ds.is_some(), "No active data sheet."),
            "clipboard.copy_col",
        ),
        ctx_custom_item(
            "ctx.col.formula_bar",
            tr_with_fallback(locale, "ctx.formula_bar", "Open Formula Bar..."),
            "ctx.col.formula_bar",
            None,
            enable_if(is_mv && ds_unfrozen, "Requires an unfrozen MultipleVariables table."),
            "ctx.formula_bar",
        ),
        ctx_custom_item(
            "ctx.col.variables",
            tr_with_fallback(locale, "ctx.variables_role", "Variables & Roles..."),
            "ctx.col.variables",
            None,
            enable_if(is_mv, "Available on MultipleVariables tables."),
            "ctx.variables_role",
        ),
        ctx_custom_item(
            "ctx.col.exclude",
            tr_with_fallback(locale, "ctx.exclude", "Exclude Column Cells"),
            "Cmd+E",
            Some("Cmd+E"),
            enable_if(ds_unfrozen, "Table is frozen or absent."),
            "ctx.exclude_column",
        ),
        ctx_custom_item(
            "ctx.col.include",
            tr_with_fallback(locale, "ctx.include", "Include Column Cells"),
            "ctx.col.include",
            None,
            enable_if(ds_unfrozen, "Table is frozen or absent."),
            "ctx.include_column",
        ),
        ctx_custom_item(
            "ctx.col.exclude_reason",
            tr_with_fallback(locale, "ctx.exclude_with_reason", "Exclude with Reason..."),
            "Cmd+Shift+E",
            Some("Cmd+Shift+E"),
            enable_if(ds_unfrozen, "Table is frozen or absent."),
            "ctx.exclude_with_reason",
        ),
        ctx_custom_item(
            "ctx.col.analyze",
            tr_with_fallback(locale, "btn.analyze", "Analyze..."),
            "Cmd+Shift+A",
            Some("Cmd+Shift+A"),
            enable_if(active_ds.is_some(), "No data sheet selected."),
            "ctx.analyze_sheet",
        ),
    ];
    (tr_with_fallback(locale, "ctx.menu.column_header", "Column Actions"), items)
}

fn build_navigator_ctx_items(app: &UiApp, locale: &str) -> (&'static str, Vec<ContextMenuItemSpec>) {
    let doc = app.document();
    let sid = resolve_nav_target_sheet(app, &app.view_shards.toolbar_menus);
    let exists = sid.is_some();
    let is_frozen = sid.as_ref().and_then(|s| doc.data_sheet(s)).is_some_and(|ds| ds.frozen);
    let is_data = sid.as_ref().is_some_and(|s| doc.data_sheet(s).is_some());
    let freeze_lbl = if is_frozen {
        tr_with_fallback(locale, "toolbar.sheet_unfreeze", "Unfreeze Sheet")
    } else {
        tr_with_fallback(locale, "toolbar.sheet_freeze", "Freeze Sheet")
    };
    let items = vec![
        ctx_custom_item(
            "ctx.nav.open",
            tr_with_fallback(locale, "ctx.select_sheet", "Open Sheet"),
            "Enter",
            None,
            enable_if(exists, "No sheet target."),
            "ctx.nav_open",
        ),
        ctx_custom_item(
            "ctx.nav.duplicate",
            tr_with_fallback(locale, "ctx.duplicate_sheet", "Duplicate Sheet"),
            "ctx.nav.duplicate",
            None,
            enable_if(exists, "No sheet target."),
            "ctx.nav_duplicate",
        ),
        ctx_custom_item(
            "ctx.nav.freeze",
            freeze_lbl,
            "ctx.nav.freeze",
            None,
            enable_if(is_data, "Only data tables can be frozen."),
            "ctx.nav_freeze_toggle",
        ),
        ctx_custom_item(
            "ctx.nav.clone_pipeline",
            tr_with_fallback(locale, "btn.clone_pipeline", "Clone Analysis Pipeline..."),
            "ctx.nav.clone_pipeline",
            None,
            enable_if(exists, "No sheet target."),
            "ctx.nav_clone",
        ),
        ctx_custom_item(
            "ctx.nav.delete",
            tr_with_fallback(locale, "ctx.delete_sheet", "Delete Sheet"),
            "ctx.nav.delete",
            None,
            enable_if(exists && !is_frozen, "Cannot delete a frozen or missing sheet."),
            "ctx.nav_delete",
        ),
    ];
    (tr_with_fallback(locale, "ctx.menu.navigator", "Sheet Actions"), items)
}

fn build_graph_object_ctx_items(app: &UiApp, locale: &str) -> (&'static str, Vec<ContextMenuItemSpec>) {
    let doc = app.document();
    let has_graph = target_graph_id(doc).is_some();
    let has_vis = target_visual_sheet_id(doc).is_some();
    let items = vec![
        ctx_custom_item(
            "ctx.graph.format_points",
            tr_with_fallback(locale, "ctx.format_points", "Format Selected Points..."),
            "ctx.graph.format_points",
            None,
            enable_if(has_graph, "Requires a graph sheet."),
            "ctx.graph_points",
        ),
        ctx_custom_item(
            "ctx.graph.format_graph",
            tr_with_fallback(locale, "ctx.format_graph", "Format Graph..."),
            "ctx.graph.format_graph",
            None,
            enable_if(has_graph, "Requires a graph sheet."),
            "ctx.graph_format",
        ),
        ctx_custom_item(
            "ctx.graph.axes",
            tr_with_fallback(locale, "ctx.format_axes", "Format Axes..."),
            "ctx.graph.axes",
            None,
            enable_if(has_graph, "Requires a graph sheet."),
            "ctx.graph_axes",
        ),
        ctx_custom_item(
            "ctx.graph.change_type",
            tr_with_fallback(locale, "ctx.change_graph_type", "Change Graph Type..."),
            "Cmd+Shift+G",
            Some("Cmd+Shift+G"),
            enable_if(has_graph, "Requires a graph sheet."),
            "ctx.graph_type",
        ),
        ctx_custom_item(
            "ctx.graph.pairwise",
            tr_with_fallback(locale, "ctx.pairwise_brackets", "Pairwise Brackets..."),
            "ctx.graph.pairwise",
            None,
            enable_if(has_graph, "Requires a graph sheet."),
            "ctx.graph_pairwise",
        ),
        ctx_custom_item(
            "ctx.graph.export_svg",
            tr_with_fallback(locale, "toolbar.export_svg", "Export Figure..."),
            "ctx.graph.export_svg",
            None,
            enable_if(has_vis, "Requires a graph or layout sheet."),
            "ctx.export_figure",
        ),
    ];
    (tr_with_fallback(locale, "ctx.menu.graph_object", "Graph Object"), items)
}

fn build_linked_region_ctx_items(app: &UiApp, locale: &str) -> (&'static str, Vec<ContextMenuItemSpec>) {
    let doc = app.document();
    let active_ds = active_data_sheet(doc);
    let ds_unfrozen = active_ds.is_some_and(|ds| !ds.frozen);
    let link_reg = active_ds.and_then(|ds| active_linked_region(app, ds));
    let has_upstream = link_reg.and_then(|reg| reg.spec.source.upstream_sheet_id()).is_some()
        || doc.active_sheet_id.as_ref().and_then(|id| doc.result_sheet(id)).is_some();
    let has_link = link_reg.is_some();
    let items = vec![
        ctx_custom_item(
            "ctx.link.go_to_source",
            tr_with_fallback(locale, "ctx.go_to_source", "Go to Source Sheet"),
            "ctx.link.go_to_source",
            None,
            enable_if(has_upstream, "Link has no upstream internal sheet."),
            "ctx.go_to_source",
        ),
        ctx_custom_item(
            "ctx.link.refresh",
            tr_with_fallback(locale, "ctx.refresh_link", "Refresh Linked Region"),
            "ctx.link.refresh",
            None,
            enable_if(active_ds.is_some(), "No active data sheet."),
            "ctx.refresh_link",
        ),
        ctx_custom_item(
            "ctx.link.copy",
            tr_with_fallback(locale, "toolbar.copy", "Copy Linked Cells"),
            "Cmd+C",
            Some("Cmd+C"),
            enable_if(active_ds.is_some(), "No active data sheet."),
            "clipboard.copy",
        ),
        ctx_custom_item(
            "ctx.link.unlink",
            tr_with_fallback(locale, "ctx.unlink_region", "Unlink Region"),
            "ctx.link.unlink",
            None,
            enable_if(has_link && ds_unfrozen, "No unfrozen linked region to detach."),
            "ctx.unlink_region",
        ),
    ];
    (tr_with_fallback(locale, "ctx.menu.linked_region", "Linked Region"), items)
}

fn build_grid_cell_ctx_items(app: &UiApp, locale: &str) -> (&'static str, Vec<ContextMenuItemSpec>) {
    let active_ds = active_data_sheet(app.document());
    let ds_unfrozen = active_ds.is_some_and(|ds| !ds.frozen);
    let has_clip = app.interaction.last_clipboard_text.as_deref().is_some_and(|s| !s.is_empty());
    let items = vec![
        ctx_custom_item(
            "ctx.grid.copy",
            tr_with_fallback(locale, "toolbar.copy", "Copy"),
            "Cmd+C",
            Some("Cmd+C"),
            enable_if(active_ds.is_some(), "No data sheet selected."),
            "clipboard.copy",
        ),
        ctx_custom_item(
            "ctx.grid.cut",
            tr_with_fallback(locale, "toolbar.cut", "Cut"),
            "Cmd+X",
            Some("Cmd+X"),
            enable_if(ds_unfrozen, "Table is frozen or absent."),
            "clipboard.cut",
        ),
        ctx_custom_item(
            "ctx.grid.paste",
            tr_with_fallback(locale, "toolbar.paste", "Paste"),
            "Cmd+V",
            Some("Cmd+V"),
            enable_if(ds_unfrozen && has_clip, "Copy data first or unfreeze the target table."),
            "clipboard.paste",
        ),
        ctx_custom_item(
            "ctx.grid.exclude",
            tr_with_fallback(locale, "ctx.exclude", "Exclude Selected Cells"),
            "Cmd+E",
            Some("Cmd+E"),
            enable_if(ds_unfrozen, "Table is frozen or absent."),
            "ctx.exclude",
        ),
        ctx_custom_item(
            "ctx.grid.include",
            tr_with_fallback(locale, "ctx.include", "Include Selected Cells"),
            "ctx.grid.include",
            None,
            enable_if(ds_unfrozen, "Table is frozen or absent."),
            "ctx.include",
        ),
        ctx_custom_item(
            "ctx.grid.exclude_reason",
            tr_with_fallback(locale, "ctx.exclude_with_reason", "Exclude with Reason..."),
            "Cmd+Shift+E",
            Some("Cmd+Shift+E"),
            enable_if(ds_unfrozen, "Table is frozen or absent."),
            "ctx.exclude_with_reason",
        ),
        ctx_custom_item(
            "ctx.grid.clear",
            tr_with_fallback(locale, "ctx.clear_cells", "Clear Selected Cells"),
            "Delete",
            Some("Delete"),
            enable_if(ds_unfrozen, "Table is frozen or absent."),
            "ctx.clear_cells",
        ),
    ];
    (tr_with_fallback(locale, "ctx.menu.grid", "Cell Actions"), items)
}

fn context_menu_items_for_target(app: &UiApp, target: &str, locale: &str) -> (&'static str, Vec<ContextMenuItemSpec>) {
    let head = target.split(':').next().unwrap_or(target);
    match head {
        "column_header" | "column" | "col_header" => build_column_header_ctx_items(app, locale),
        "navigator" | "nav" | "sheet" => build_navigator_ctx_items(app, locale),
        "graph_object" | "canvas" | "graph" => build_graph_object_ctx_items(app, locale),
        "linked_region" | "link" => build_linked_region_ctx_items(app, locale),
        _ => build_grid_cell_ctx_items(app, locale),
    }
}

/// Build an open right-click context menu overlay at [`ZLayer::ContextMenu`] for `target`
/// (`grid`, `column_header`, `navigator`, `graph_object`, or `linked_region`).
pub fn build_open_context_menu_popup(builder: &mut LayoutBuilder<'_>, target: &str) {
    if builder.app.canvas_dialogs.active_dialog != ActiveDialog::None || builder.tree.node("popup.context_menu").is_some() {
        return;
    }
    let locale = builder.tree.locale.clone();
    let (title, items) = context_menu_items_for_target(builder.app, target, &locale);
    if items.is_empty() {
        return;
    }

    let vp_w = builder.tree.metrics.physical_width_px().max(260);
    let vp_h = builder.tree.metrics.physical_height_px().max(180);
    let pop_w = 244u32.min(vp_w.saturating_sub(16));
    let hdr_h = 20u32;
    let row_h = 22u32;
    let pop_h = (hdr_h + 6 + (items.len() as u32).saturating_mul(row_h)).min(vp_h.saturating_sub(16));

    let default_anchor = if let Some(a) = builder.app.view_shards.toolbar_menus.context_menu_anchor_px {
        a
    } else if target.starts_with("navigator") || target == "nav" || target == "sheet" {
        let nav = builder.tree.frame.navigator;
        [i32::try_from(nav.x + 24).unwrap_or(24), i32::try_from(nav.y + 48).unwrap_or(96)]
    } else {
        let canvas = builder.tree.frame.canvas;
        [i32::try_from(canvas.x + 36).unwrap_or(180), i32::try_from(canvas.y + 44).unwrap_or(110)]
    };

    let max_x = i32::try_from(vp_w.saturating_sub(pop_w + 4)).unwrap_or(0).max(2);
    let max_y = i32::try_from(vp_h.saturating_sub(pop_h + 4)).unwrap_or(0).max(2);
    let pop_rect = LayoutRectPx::new(default_anchor[0].clamp(2, max_x), default_anchor[1].clamp(2, max_y), pop_w, pop_h);

    let menu_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("popup.context_menu"),
        parent: None,
        z_layer: ZLayer::ContextMenu,
        rect_px: pop_rect,
        a11y: labeled_meta(WidgetRole::ContextMenu, title, FocusScope::PopupMenu, "Escape"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ContextMenu, Some(&menu_wid), pop_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ContextMenu, Some(&menu_wid), pop_rect, builder.palette.accent, 1);

    let hdr_rect = LayoutRectPx::new(pop_rect.x + 2, pop_rect.y + 2, pop_w.saturating_sub(4), hdr_h);
    let hdr_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("popup.context_menu.header"),
        parent: Some(menu_wid.clone()),
        z_layer: ZLayer::ContextMenu,
        rect_px: hdr_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, title, FocusScope::PopupMenu),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ContextMenu, Some(&hdr_wid), hdr_rect, builder.palette.panel);
    let hdr_size = (11.0 * builder.scale()).clamp(8.5, 19.0);
    paint_text_fitted(
        builder,
        &hdr_wid,
        FittedTextSpec {
            z_layer: ZLayer::ContextMenu,
            rect: hdr_rect,
            pad_x: 6,
            text: title,
            style: &UiTextStyle::ui_bold(hdr_size),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );

    let item_size = (11.5 * builder.scale()).clamp(9.0, 20.0);
    let item_style = UiTextStyle::ui_sans(item_size);
    let mut cur_y = pop_rect.y + 4 + i32::try_from(hdr_h).unwrap_or(20);
    for item in items {
        if cur_y + i32::try_from(row_h).unwrap_or(22) > pop_rect.bottom() - 2 {
            break;
        }
        let item_rect = LayoutRectPx::new(pop_rect.x + 3, cur_y, pop_w.saturating_sub(6), row_h);
        let st = widget_state(builder, item.id, item.enablement.clone());
        let mut a11y = labeled_meta(WidgetRole::MenuItem, &item.label, FocusScope::PopupMenu, item.keyboard_path);
        a11y.shortcut = item.shortcut.map(str::to_owned);
        let item_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(item.id),
            parent: Some(menu_wid.clone()),
            z_layer: ZLayer::ContextMenu,
            rect_px: item_rect,
            a11y,
            state: st.clone(),
            action: if st.enablement.enabled { item.action } else { WidgetAction::NoOp },
        });
        let bg = if st.hovered && st.enablement.enabled { builder.palette.header } else { builder.palette.surface };
        let fg = if st.enablement.enabled { builder.palette.text } else { builder.palette.muted };
        builder.paint_fill(ZLayer::ContextMenu, Some(&item_wid), item_rect, bg);
        if st.enablement.enabled {
            let bar_x = if builder.tree.is_rtl { item_rect.right() - 4 } else { item_rect.x + 2 };
            let bar = LayoutRectPx::new(bar_x, item_rect.y + 4, 2, row_h.saturating_sub(8).max(4));
            builder.paint_fill(ZLayer::ContextMenu, Some(&item_wid), bar, builder.palette.accent);
        }
        paint_text_fitted(
            builder,
            &item_wid,
            FittedTextSpec {
                z_layer: ZLayer::ContextMenu,
                rect: item_rect,
                pad_x: 8,
                text: &item.label,
                style: &item_style,
                color: fg,
                scroll_container: None,
            },
        );
        cur_y += i32::try_from(row_h).unwrap_or(22);
    }
}

/// Build the bottom workspace status bar inside `rect`.
pub fn build_status_bar_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width > 0 && rect.height > 0 {
        builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.panel);
        builder.paint_line(ZLayer::Workspace, None, [rect.x, rect.y], [rect.right(), rect.y], builder.palette.border, 1);

        let doc = builder.app.document();
        let ctx = active_toolbar_context(doc);
        let active_title = active_sheet_title(doc).unwrap_or("Ready");
        let frozen_tag = if active_data_sheet(doc).is_some_and(|ds| ds.frozen) { " [Frozen]" } else { "" };
        let notice_tag = builder.app.view_shards.toolbar_menus.last_toolbar_notice.as_deref().map(|n| format!(" | {n}")).unwrap_or_default();

        let summary = if let Some(qs) = &builder.app.grid.quick_stats
            && qs.numeric_count > 0
        {
            if let Some(mean) = qs.mean {
                format!(
                    "{active_title} ({}){frozen_tag} | n={} | excl={} | mean={mean:.3}{notice_tag}",
                    ctx.badge(),
                    qs.numeric_count,
                    qs.excluded_count
                )
            } else {
                format!("{active_title} ({}){frozen_tag} | n={} | excl={}{notice_tag}", ctx.badge(), qs.numeric_count, qs.excluded_count)
            }
        } else {
            format!(
                "{active_title} ({}){frozen_tag} | Zoom {:.0}% | {} | {:?}{notice_tag}",
                ctx.badge(),
                builder.app.canvas_dialogs.canvas.zoom * 100.0,
                builder.app.locale,
                builder.app.theme.mode
            )
        };

        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from("status.summary"),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, summary.clone(), FocusScope::Workspace),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        let size_px = (11.5 * builder.scale()).clamp(8.5, 20.0);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect,
                pad_x: 8,
                text: &summary,
                style: &UiTextStyle::ui_sans(size_px),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewportMetrics;
    use crate::dialogs_canvas::ActiveDialog;
    use crate::human_driver::HumanPathDriver;
    use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, TableArchetype};
    use curvebook_engine::Session;

    #[test]
    fn toolbar_themes_apply_style_menu_and_rtl_order_work() -> Result<(), String> {
        let mut doc = ProjectDocument::new("Toolbar W6").map_err(|e| format!("{e}"))?;
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| format!("{e}"))?;
        doc.add_data_sheet(DataSheet::new("ds-1", "Data", TableArchetype::Column(col)).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;
        let mut session = Session::from_document(doc);
        session.execute("graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "ds-1", "kind": "column" })).map_err(|e| format!("{e}"))?;
        session.execute("sheet.select", json!({ "sheet_id": "gr-1" })).map_err(|e| format!("{e}"))?;
        let app = UiApp::from_session(session);
        let mut driver = HumanPathDriver::new(app, ViewportMetrics::new(1280.0, 800.0, 1.0));

        driver.click_widget("toolbar.theme.dark")?;
        assert_eq!(driver.app().theme.mode, UiThemeMode::Dark);
        driver.click_widget("toolbar.theme.high_contrast")?;
        assert_eq!(driver.app().theme.mode, UiThemeMode::HighContrast);
        driver.click_widget("toolbar.theme.light")?;
        assert_eq!(driver.app().theme.mode, UiThemeMode::Light);

        driver.click_widget("menu.graph")?;
        let graph_popup = driver.layout_tree();
        assert!(graph_popup.node("menu.ui.applyStyleFrom.open").is_some());
        driver.click_widget("menu.ui.applyStyleFrom.open")?;
        assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ApplyStyleFrom { .. }));
        driver.click_widget("modal.cancel")?;

        driver.app_mut().locale = String::from("he");
        let rtl_tree = driver.layout_tree();
        assert!(rtl_tree.is_rtl);
        let tn = rtl_tree.node("toolbar.new").ok_or("missing toolbar.new")?;
        let te = rtl_tree.node("toolbar.export_svg").ok_or("missing toolbar.export_svg")?;
        assert!(tn.bounds.rect_px.x > te.bounds.rect_px.x);
        Ok(())
    }
}
