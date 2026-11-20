//! Menu bar, contextual icon-and-label toolbar, dropdown popups, right-click context menus,
//! and status bar view builders (`R10.6`).

use curvebook_doc::{DataSheet, ProjectDocument, ResultSheet, SheetCellRange, SheetId};
use curvebook_tools::GridCellCoord;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveInspector, ResultSheetViewState};
use crate::grid_view::data_set_subcolumn_count;
use crate::interaction::UiEventOutcome;
use crate::os_integration::ClipboardBundle;

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
                let _ = app.write_clipboard_bundle(&bundle);
                let text = if !bundle.svg.is_empty() { bundle.svg.clone() } else { active_sheet_title(&doc).unwrap_or("Figure").to_owned() };
                let line_count = text.lines().count().max(1);
                app.interaction.last_clipboard_text = Some(text);
                app.interaction.last_clipboard_bundle = Some(bundle);
                return Ok(format!("Copied {line_count} line(s)"));
            }
            Err(_) => active_sheet_title(&doc).unwrap_or("Figure").to_owned(),
        }
    } else {
        return Err(String::from("active sheet is not copyable"));
    };
    let line_count = payload.lines().count().max(1);
    let _ = app.write_clipboard_text(&payload);
    app.interaction.last_clipboard_bundle = None;
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
        if app.nav.gallery.open {
            app.nav.sync_gallery(app.session.document());
        }
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
            if ds.frozen {
                state.last_toolbar_notice = Some(String::from(crate::i18n::tr(&app.locale, "w7.mv.frozen_clear_blocked")));
                return Ok(UiEventOutcome { handled: true, action: String::from("grid_core.frozen_read_only"), ..Default::default() });
            }
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
