#![forbid(unsafe_code)]
//! `R10.6` Contextual Toolbar, Menu Routing without `Value::Null`, and All-5 Context Menus (`w5c1`).

mod human_wf_common;

use curvebook_doc::{
    Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec,
    MultipleVariablesTable, ProjectDocument, RectPt, SheetId, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ActiveInspector};
use curvebook_ui_egui::human_driver::HumanPathDriver;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, UiEventOutcome, UiInputEvent, ViewportMetrics, dispatch_ui_event};
use curvebook_ui_egui::layout::verify::verify_click_action_parity;
use curvebook_ui_egui::layout::{WidgetAction, WidgetId, ZLayer};
use curvebook_ui_egui::menus::TOOLBAR_BUTTONS;
use curvebook_ui_egui::views::toolbar_menus::{ActiveToolbarContext, active_toolbar_context};
use serde_json::{Value, json};

use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_widget_center, default_viewport, new_human_driver, press_cmd_shortcut,
    scroll_until_widget_visible, scroll_widget_wheel,
};

fn contains_json_null(val: &Value) -> bool {
    match val {
        Value::Null => true,
        Value::Array(arr) => arr.iter().any(contains_json_null),
        Value::Object(map) => map.values().any(contains_json_null),
        _ => false,
    }
}

fn right_click_widget_center(driver: &mut HumanPathDriver, widget_id: &str) -> Result<UiEventOutcome, String> {
    let [lx, ly] = driver.widget_center_logical(widget_id).ok_or_else(|| format!("widget `{widget_id}` is not visibly drawn"))?;
    let metrics = driver.metrics();
    dispatch_ui_event(
        driver.app_mut(),
        UiInputEvent::PointerDown { x: lx, y: ly, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
        metrics,
    )
}

fn build_workspace_with_sheets() -> Result<(UiApp, SheetId, SheetId, SheetId, SheetId), String> {
    let mut doc = ProjectDocument::default();

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Treated");
    col.data_sets[2].title = String::from("HighDose");
    let c0 = [10.0, 11.2, 9.8, 10.6, 10.4];
    let c1 = [14.5, 15.1, 13.9, 14.8, 15.3];
    let c2 = [20.1, 21.0, 19.6, 20.8, 21.5];
    for r in 0..5 {
        col.data_sets[0].cells[r][0] = Cell::number(c0[r]);
        col.data_sets[1].cells[r][0] = Cell::number(c1[r]);
        col.data_sets[2].cells[r][0] = Cell::number(c2[r]);
    }
    let ds = DataSheet::new("ds-1", "Dose Response", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    let ds_id = ds.id.clone();
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;

    let info = InfoSheet::new_preseeded("info-1", "Protocol Info").map_err(|e| e.to_string())?;
    let info_id = info.id.clone();
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let ds_ref = doc.data_sheet(&ds_id).ok_or_else(|| String::from("missing ds-1"))?;
    let mut gs = GraphSheet::new_uninitialized_for_data("g-1", ds_ref).map_err(|e| e.to_string())?;
    gs.title = String::from("Dose Plot");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Column;
    let gs_id = gs.id.clone();
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;

    let ls_spec = LayoutSpec {
        items: vec![LayoutItem::LiveGraph {
            id: String::from("panel-1"),
            graph_sheet_id: gs_id.clone(),
            bounds_pt: RectPt::new(36.0, 36.0, 280.0, 200.0),
            scale_percent: 100.0,
            badge_override: None,
        }],
        ..LayoutSpec::default()
    };
    let ls = LayoutSheet::new("lay-1", "Figure Layout", ls_spec).map_err(|e| e.to_string())?;
    let ls_id = ls.id.clone();
    doc.add_layout_sheet(ls).map_err(|e| e.to_string())?;

    doc.select_in_navigator(&ds_id).map_err(|e| e.to_string())?;
    Ok((UiApp::from_session(Session::from_document(doc)), ds_id, info_id, gs_id, ls_id))
}

#[test]
fn r10_6_contextual_toolbar_adapts_across_all_sheet_kinds_and_preserves_parity() -> Result<(), String> {
    // 1. EmptyWorkspace context: core buttons present, sheet/graph/analysis commands dynamically disabled.
    let mut empty_driver = new_human_driver(UiApp::new(), default_viewport());
    assert_eq!(active_toolbar_context(empty_driver.app().document()), ActiveToolbarContext::EmptyWorkspace);
    {
        let tree = empty_driver.layout_tree();
        for spec in TOOLBAR_BUTTONS {
            let node = tree.node(spec.id).ok_or_else(|| format!("empty workspace missing core button `{}`", spec.id))?;
            if !node.drawn_in_frame {
                return Err(format!("core toolbar button `{}` not drawn in EmptyWorkspace", spec.id));
            }
        }
        for disabled_id in ["toolbar.analyze", "toolbar.axes", "toolbar.print", "toolbar.export_svg"] {
            let node = tree.node(disabled_id).ok_or_else(|| format!("missing `{disabled_id}`"))?;
            if node.state.enablement.enabled {
                return Err(format!("expected `{disabled_id}` disabled on EmptyWorkspace"));
            }
            if node.state.enablement.disabled_reason.as_deref().is_none_or(str::is_empty) {
                return Err(format!("expected `{disabled_id}` to provide non-empty disabled_reason"));
            }
        }
        for enabled_id in ["toolbar.new", "toolbar.open", "toolbar.save", "toolbar.palette"] {
            let node = tree.node(enabled_id).ok_or_else(|| format!("missing `{enabled_id}`"))?;
            if !node.state.enablement.enabled {
                return Err(format!("expected `{enabled_id}` enabled on EmptyWorkspace"));
            }
        }
    }
    let _ = assert_zero_shortcut_compliance(&mut empty_driver)?;
    let _ = empty_driver.render_frame().map_err(|e| e.to_string())?;

    // 2. DataSheet context: clipboard, sheet duplicate/freeze, Analyze, plus Undo/Redo & core actions.
    let (app, ds_id, info_id, gs_id, ls_id) = build_workspace_with_sheets()?;
    let mut driver = new_human_driver(app, default_viewport());
    assert_eq!(active_toolbar_context(driver.app().document()), ActiveToolbarContext::DataSheet);
    {
        let tree = driver.layout_tree();
        for id in
            ["toolbar.copy", "toolbar.cut", "toolbar.paste", "toolbar.sheet_duplicate", "toolbar.sheet_freeze", "toolbar.analyze", "toolbar.axes"]
        {
            if tree.node(id).is_none_or(|n| !n.drawn_in_frame) {
                return Err(format!("expected DataSheet toolbar button `{id}` drawn"));
            }
        }
    }
    // Select cell (0,0,0), Copy -> populates clipboard & enables Paste -> Paste -> Freeze/Unfreeze -> Duplicate.
    driver.app_mut().grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.copy"))?;
    let clip = driver.app().interaction.last_clipboard_text.as_deref().unwrap_or("");
    if clip.is_empty() {
        return Err(String::from("toolbar.copy did not populate last_clipboard_text"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.paste"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.sheet_freeze"))?;
    if !driver.app().document().data_sheet(&ds_id).is_some_and(|ds| ds.frozen) {
        return Err(String::from("toolbar.sheet_freeze did not freeze DataSheet"));
    }
    if driver.layout_tree().node("toolbar.cut").is_some_and(|n| n.state.enablement.enabled) {
        return Err(String::from("expected toolbar.cut disabled when DataSheet is frozen"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.sheet_freeze"))?;
    if driver.app().document().data_sheet(&ds_id).is_some_and(|ds| ds.frozen) {
        return Err(String::from("second toolbar.sheet_freeze click did not unfreeze DataSheet"));
    }
    let before_sheets = driver.app().document().data_sheets.len();
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.sheet_duplicate"))?;
    if driver.app().document().data_sheets.len() != before_sheets + 1 {
        return Err(String::from("toolbar.sheet_duplicate did not duplicate DataSheet"));
    }
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(|e| e.to_string())?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 3. InfoSheet context: text styling (Bold / Italic) toggles and stays parity-clean.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": info_id.as_str() })).map_err(|e| e.to_string())?;
    assert_eq!(active_toolbar_context(driver.app().document()), ActiveToolbarContext::InfoSheet);
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.text_bold"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.text_italic"))?;
    if !driver.app().view_shards.toolbar_menus.text_bold || !driver.app().view_shards.toolbar_menus.text_italic {
        return Err(String::from("expected toolbar.text_bold and toolbar.text_italic toggles to update state"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 4. ResultSheet context: Analyze -> One-Way ANOVA -> Parameters, Data Analyzed, Interpret.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(|e| e.to_string())?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
    if driver.find_widget("analyze.entry.stats.one_way_anova").is_none_or(|n| !n.drawn_in_frame) {
        let _ = scroll_widget_wheel(&mut driver, &WidgetId::from("analyze.entries_list"), 0.0, -180.0);
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("analyze.entry.stats.one_way_anova"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    assert_eq!(active_toolbar_context(driver.app().document()), ActiveToolbarContext::ResultSheet);

    {
        let tree = driver.layout_tree();
        for id in ["toolbar.parameters", "toolbar.data_analyzed", "toolbar.interpret"] {
            let node = tree.node(id).ok_or_else(|| format!("missing ResultSheet button `{id}`"))?;
            if !node.drawn_in_frame || !node.state.enablement.enabled {
                return Err(format!("expected ResultSheet button `{id}` drawn and enabled"));
            }
        }
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.interpret"))?;
    if !driver.app().nav.inspector_open || driver.app().canvas_dialogs.active_inspector != ActiveInspector::ResultDetails {
        return Err(String::from("toolbar.interpret did not open ActiveInspector::ResultDetails"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.parameters"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(String::from("toolbar.parameters did not open EditAnalysisParameters"));
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.data_analyzed"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeDataAnalyzed { .. }) {
        return Err(String::from("toolbar.data_analyzed did not open ChangeDataAnalyzed"));
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 5. GraphSheet context across small 1024x640@1.0, 1280x800@1.0, and HiDPI 1920x1200@2.0 viewports.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": gs_id.as_str() })).map_err(|e| e.to_string())?;
    assert_eq!(active_toolbar_context(driver.app().document()), ActiveToolbarContext::GraphSheet);

    for vp in [ViewportMetrics::new(1024.0, 640.0, 1.0), ViewportMetrics::new(1920.0, 1200.0, 2.0), default_viewport()] {
        driver.set_metrics(vp);
        let rep = driver.verify_parity();
        if (vp.scale_factor - 1.0).abs() < 0.01 {
            if !rep.is_clean() {
                return Err(format!("GraphSheet parity failed at {vp:?}: {:?}", rep.violations));
            }
        } else {
            let own_violations: Vec<_> =
                rep.violations.iter().filter(|v| v.widget_id.starts_with("toolbar.") || v.widget_id.starts_with("menu.")).collect();
            if !own_violations.is_empty() {
                return Err(format!("GraphSheet toolbar/menu parity failed at {vp:?}: {own_violations:?}"));
            }
        }
        let tree = driver.layout_tree();
        for id in ["toolbar.change_graph", "toolbar.axes", "toolbar.pairwise", "toolbar.rulers", "toolbar.palette", "toolbar.export_svg"] {
            if tree.node(id).is_none_or(|n| !n.drawn_in_frame) {
                return Err(format!("expected GraphSheet button `{id}` drawn at {vp:?}"));
            }
        }
    }

    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.rulers"))?;
    if !driver.app().canvas_dialogs.rulers_visible {
        return Err(String::from("toolbar.rulers did not toggle rulers_visible on"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.axes"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }) {
        return Err(String::from("toolbar.axes did not open FormatAxes"));
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.change_graph"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        return Err(String::from("toolbar.change_graph did not open ChangeGraphType"));
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.pairwise"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::PairwiseComparisons { .. }) {
        return Err(String::from("toolbar.pairwise did not open PairwiseComparisons"));
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.export_svg"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }) {
        return Err(String::from("toolbar.export_svg did not open ExportFigure"));
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;

    // 6. LayoutSheet context: Alignment + Rulers.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": ls_id.as_str() })).map_err(|e| e.to_string())?;
    assert_eq!(active_toolbar_context(driver.app().document()), ActiveToolbarContext::LayoutSheet);
    let align_before = driver.app().canvas_dialogs.page_layout.align_plot_origins;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.alignment"))?;
    if driver.app().canvas_dialogs.page_layout.align_plot_origins == align_before {
        return Err(String::from("toolbar.alignment did not toggle align_plot_origins"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.alignment"))?;
    if driver.app().canvas_dialogs.page_layout.align_plot_origins != align_before {
        return Err(String::from("second toolbar.alignment click did not restore align_plot_origins"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_6_open_menu_popups_route_without_null_and_handle_narrow_overflow() -> Result<(), String> {
    let (app, ds_id, _, gs_id, _) = build_workspace_with_sheets()?;
    let mut driver = new_human_driver(app, default_viewport());
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": gs_id.as_str() })).map_err(|e| e.to_string())?;

    for menu_key in ["menu.file", "menu.edit", "menu.analyze", "menu.graph"] {
        let _ = click_widget_center(&mut driver, &WidgetId::from(menu_key))?;
        let rep = driver.verify_parity();
        if !rep.is_clean() {
            return Err(format!("open menu `{menu_key}` parity violations: {:?}", rep.violations));
        }
        let tree = driver.layout_tree();
        let dropdown_items: Vec<_> = tree
            .nodes
            .iter()
            .filter(|n| {
                n.id.as_str().starts_with("menu.")
                    || n.id.as_str().starts_with("toolbar.")
                    || n.id.as_str().starts_with("popup.")
                    || n.id.as_str().starts_with("ctx.")
            })
            .collect();
        if dropdown_items.iter().filter(|n| n.z_layer == ZLayer::DropdownMenu).count() == 0 {
            return Err(format!("expected dropdown menu items rendered for `{menu_key}`"));
        }
        for node in dropdown_items {
            if let WidgetAction::InvokeCommand { params, .. } = &node.action
                && contains_json_null(params)
            {
                return Err(format!("widget `{}` dispatched Value::Null in InvokeCommand params: {params}", node.id.as_str()));
            }
            if let Some(val_json) = &node.a11y.value_json
                && contains_json_null(val_json)
            {
                return Err(format!("widget `{}` contains Value::Null in a11y.value_json", node.id.as_str()));
            }
        }
        // Close open menu before opening the next menu section.
        let _ = driver.press_key("Escape", KeyModifiers::default())?;
    }

    // Verify clicking menu.file -> scroll to Save -> click executes cleanly and closes popup.
    let _ = click_widget_center(&mut driver, &WidgetId::from("menu.file"))?;
    scroll_until_widget_visible(&mut driver, &WidgetId::from("popup.menu.file"), &WidgetId::from("menu.project.save"), 80.0, 8)?;
    let save_out = click_widget_center(&mut driver, &WidgetId::from("menu.project.save"))?;
    if !save_out.handled {
        return Err(String::from("menu.project.save click was not handled"));
    }
    if driver.app().interaction.open_menu.is_some() {
        return Err(String::from("expected open_menu dismissed after menu item execution"));
    }

    // Switch to narrow 640x480 viewport so secondary toolbar items move into overflow popup cleanly.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(|e| e.to_string())?;
    driver.set_metrics(ViewportMetrics::new(640.0, 480.0, 1.0));
    let narrow_tree = driver.layout_tree();
    if narrow_tree.node("toolbar.overflow").is_none_or(|n| !n.drawn_in_frame) {
        return Err(String::from("expected toolbar.overflow on narrow 640px Graph/Data viewport"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.overflow"))?;
    if !driver.app().view_shards.toolbar_menus.overflow_open {
        return Err(String::from("clicking toolbar.overflow did not open overflow popup"));
    }
    if driver.layout_tree().node("popup.toolbar_overflow").is_none_or(|n| !n.drawn_in_frame) {
        return Err(String::from("expected popup.toolbar_overflow node drawn"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn r10_6_all_five_right_click_context_menus_render_and_execute_actions() -> Result<(), String> {
    let mut doc = ProjectDocument::default();

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("GroupA");
    col.data_sets[1].title = String::from("GroupB");
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + f64::from(u32::try_from(r).unwrap_or(0)));
        col.data_sets[1].cells[r][0] = Cell::number(20.0 + f64::from(u32::try_from(r).unwrap_or(0)));
    }
    let col_src = DataSheet::new("col-src", "Source Column", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    let col_id = col_src.id.clone();
    doc.add_data_sheet(col_src).map_err(|e| e.to_string())?;

    let mut mv = MultipleVariablesTable::new(2, 4).map_err(|e| e.to_string())?;
    mv.variables[0].title = String::from("Dose");
    mv.variables[1].title = String::from("Response");
    for r in 0..4 {
        mv.variables[0].cells[r] = Cell::number(f64::from(u32::try_from(r + 1).unwrap_or(1)));
        mv.variables[1].cells[r] = Cell::number(f64::from(u32::try_from((r + 1) * 5).unwrap_or(5)));
    }
    let mv_ds = DataSheet::new("mv-src", "Multivar Source", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
    let mv_id = mv_ds.id.clone();
    doc.add_data_sheet(mv_ds).map_err(|e| e.to_string())?;

    let empty_dest = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
    let ds_dest = DataSheet::new("ds-dest", "Linked Target", TableArchetype::Column(empty_dest)).map_err(|e| e.to_string())?;
    let dest_id = ds_dest.id.clone();
    doc.add_data_sheet(ds_dest).map_err(|e| e.to_string())?;

    let col_ref = doc.data_sheet(&col_id).ok_or_else(|| String::from("missing col-src"))?;
    let mut gs = GraphSheet::new_uninitialized_for_data("g-ctx", col_ref).map_err(|e| e.to_string())?;
    gs.title = String::from("Ctx Graph");
    gs.initialized = true;
    let gs_id = gs.id.clone();
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;

    doc.select_in_navigator(&col_id).map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.execute_command("table.pasteLink", json!({ "target_sheet_id": dest_id.as_str(), "source_sheet_id": col_id.as_str() }))
        .map_err(|e| format!("pasteLink setup failed: {e}"))?;
    app.execute_command("sheet.select", json!({ "sheet_id": col_id.as_str() })).map_err(|e| format!("select col-src failed: {e}"))?;

    let mut driver = new_human_driver(app, default_viewport());

    // Verify genuine PointerButton::Secondary click on rendered grid cell renders ZLayer::ContextMenu popup.
    let rc_out = right_click_widget_center(&mut driver, "grid.cell.0.0.0").map_err(|e| format!("secondary click grid.cell.0.0.0: {e}"))?;
    if !rc_out.handled || driver.app().interaction.context_menu_target.as_deref() != Some("grid") {
        return Err(format!("expected secondary click on grid.cell.0.0.0 to set context_menu_target=grid, got {rc_out:?}"));
    }
    {
        let tree = driver.layout_tree();
        let ctx_node = tree.node("popup.context_menu").ok_or_else(|| String::from("popup.context_menu missing after secondary click"))?;
        if ctx_node.z_layer != ZLayer::ContextMenu || !ctx_node.drawn_in_frame {
            return Err(String::from("popup.context_menu must render at ZLayer::ContextMenu"));
        }
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    // Right-click empty top-left margin clears context_menu_target.
    let m = driver.metrics();
    let _ = dispatch_ui_event(
        driver.app_mut(),
        UiInputEvent::PointerDown { x: 4.0, y: 4.0, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
        m,
    )?;
    if driver.app().interaction.context_menu_target.is_some() {
        return Err(String::from("expected empty margin right-click to clear context_menu_target"));
    }

    // 1. Grid Cell context menu: select 2 cells -> Exclude -> verify parity -> Include -> Exclude With Reason (with selection!).
    {
        let ds_clone = driver.app().document().data_sheet(&col_id).ok_or_else(|| String::from("missing col-src"))?.clone();
        driver.app_mut().grid.select_single_sync(GridCellCoord::new(0, 0, 0));
        driver.app_mut().grid.extend_shift_range(&ds_clone, GridCellCoord::new(1, 0, 0));
        driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("grid"));
    }
    let parity_ok = verify_click_action_parity(driver.app(), driver.metrics(), "ctx.grid.exclude")
        .map_err(|e| format!("verify_click_action_parity ctx.grid.exclude: {e}"))?;
    if !parity_ok {
        return Err(String::from("verify_click_action_parity failed on ctx.grid.exclude"));
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.grid.exclude")).map_err(|e| format!("click ctx.grid.exclude: {e}"))?;
    let ds_after_excl = driver.app().document().data_sheet(&col_id).ok_or_else(|| String::from("missing col-src"))?;
    for r in 0..=1 {
        let excluded = ds_after_excl.archetype.data_cell(CellCoord::new(r, 0, 0)).is_some_and(|c| c.excluded);
        if !excluded {
            return Err(format!("expected cell ({r}, 0, 0) excluded after ctx.grid.exclude"));
        }
    }
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("grid"));
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.grid.include"))?;
    let ds_after_incl = driver.app().document().data_sheet(&col_id).ok_or_else(|| String::from("missing col-src"))?;
    for r in 0..=1 {
        let excluded = ds_after_incl.archetype.data_cell(CellCoord::new(r, 0, 0)).is_some_and(|c| c.excluded);
        if excluded {
            return Err(format!("expected cell ({r}, 0, 0) included after ctx.grid.include"));
        }
    }
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("grid"));
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.grid.exclude_reason"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. })
        || driver.app().canvas_dialogs.exclude_with_reason.coords.len() != 2
    {
        return Err(format!(
            "expected ExcludeWithReason dialog with 2 selected coords, got dialog={:?}, coords={}",
            driver.app().canvas_dialogs.active_dialog,
            driver.app().canvas_dialogs.exclude_with_reason.coords.len()
        ));
    }
    let _ = press_cmd_shortcut(&mut driver, "Escape").or_else(|_| driver.press_key("Escape", KeyModifiers::default()))?;

    // 2. Column Header context menu on MultipleVariables table -> Open Formula Bar & Variables & Roles.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": mv_id.as_str() })).map_err(|e| e.to_string())?;
    driver.app_mut().grid.select_single_sync(GridCellCoord::new(0, 1, 0));
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("column_header"));
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.col.formula_bar"))?;
    if !driver.app().grid.formula_bar.active {
        return Err(String::from("ctx.col.formula_bar did not activate formula bar"));
    }
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("column_header"));
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.col.variables"))?;
    if !driver.app().grid.variables_dialog.open {
        return Err(String::from("ctx.col.variables did not open VariablesDialogState"));
    }
    driver.app_mut().grid.variables_dialog.open = false;

    // 3. Navigator Entry context menu -> Duplicate Sheet.
    let before_count = driver.app().document().data_sheets.len();
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(format!("navigator:{}", col_id.as_str()));
    driver.app_mut().view_shards.toolbar_menus.context_sheet_id = Some(col_id.as_str().to_owned());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.nav.duplicate"))?;
    if driver.app().document().data_sheets.len() != before_count + 1 {
        return Err(String::from("ctx.nav.duplicate did not duplicate target navigator sheet"));
    }

    // 4. Graph Object context menu -> Format Axes & Format Selected Points.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": gs_id.as_str() })).map_err(|e| e.to_string())?;
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("graph_object"));
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.graph.axes"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }) {
        return Err(String::from("ctx.graph.axes did not open FormatAxes"));
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("graph_object"));
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.graph.format_points"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::FormatSelectedPoints { .. }) {
        return Err(String::from("ctx.graph.format_points did not open FormatSelectedPoints"));
    }
    let _ = driver.press_key("Escape", KeyModifiers::default())?;

    // 5. Linked Region context menu -> Go to Source Sheet & Unlink Region.
    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": dest_id.as_str() })).map_err(|e| e.to_string())?;
    if driver.app().document().data_sheet(&dest_id).is_none_or(|ds| ds.linked_regions.is_empty()) {
        return Err(String::from("expected linked_regions present on ds-dest"));
    }
    driver.app_mut().grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("linked_region"));
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.link.go_to_source"))?;
    if driver.app().document().active_sheet_id.as_ref() != Some(&col_id) {
        return Err(format!("expected ctx.link.go_to_source to switch to `{col_id}`, got {:?}", driver.app().document().active_sheet_id));
    }

    driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": dest_id.as_str() })).map_err(|e| e.to_string())?;
    driver.app_mut().view_shards.toolbar_menus.active_context_menu_id = Some(String::from("linked_region"));
    let _ = click_widget_center(&mut driver, &WidgetId::from("ctx.link.unlink"))?;
    if driver.app().document().data_sheet(&dest_id).is_some_and(|ds| !ds.linked_regions.is_empty()) {
        return Err(String::from("expected ctx.link.unlink to detach all linked regions"));
    }

    let img = driver.render_frame().map_err(|e| e.to_string())?;
    if img.width_px == 0 || img.height_px == 0 {
        return Err(String::from("rendered frame was empty"));
    }
    Ok(())
}
