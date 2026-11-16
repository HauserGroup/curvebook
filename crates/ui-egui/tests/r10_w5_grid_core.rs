//! Wave 5 Shard `w5d1` end-to-end tests (`R10.8` multi-tier headers, Row Titles,
//! column resize, 2-axis scroll & keyboard navigation + `R10.12` linked regions &
//! excluded cell styling/provenance tooltips) driven through `HumanPathDriver`.

#![forbid(unsafe_code)]

mod human_wf_common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use curvebook_doc::{
    CellCoord, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind, LinkBorderColor, LinkSource, PlacementMode, SheetCellRange,
    SheetId, TableArchetype,
};
use curvebook_palette::Rgba8;
use curvebook_text::SpanSlope;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::control_io::dispatch_gui_action;
use curvebook_ui_egui::grid_view::provenance_border_rgba;
use curvebook_ui_egui::interaction::{KeyModifiers, UiEventOutcome};
use curvebook_ui_egui::layout::paint::PaintOpKind;
use curvebook_ui_egui::layout::{WidgetAction, ZLayer};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::views::grid_core::{
    GridCoreViewState, handle_grid_double_click, handle_grid_key_down, handle_grid_pointer_hover, handle_grid_text_input,
};
use curvebook_ui_egui::views::{ViewShardStates, dispatch_custom_view_action};
use curvebook_ui_egui::{HumanPathDriver, SplitWorkspaceState, UiApp};
use human_wf_common::{assert_input_to_paint_budget, assert_zero_shortcut_compliance, default_viewport, new_human_driver};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn create_sheet(app: &mut UiApp, params: Value) -> Result<SheetId, String> {
    let before: Vec<String> = app.document().data_sheets.iter().map(|s| s.id.as_str().to_owned()).collect();
    let out = app.execute_command("sheet.createData", params).map_err(|e| format!("createData failed: {e}"))?;
    let raw = out
        .payload
        .get("sheet_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| app.document().data_sheets.iter().map(|s| s.id.as_str().to_owned()).find(|id| !before.contains(id)))
        .ok_or_else(|| format!("missing sheet_id in {}", out.payload))?;
    app.execute_command("sheet.select", json!({ "sheet_id": raw, "mode": "navigator" })).map_err(|e| format!("sheet.select failed: {e}"))?;
    SheetId::try_new(&raw).map_err(|e| format!("invalid sheet_id {raw}: {e}"))
}

fn paste_cells(app: &mut UiApp, sheet_id: &SheetId, tsv: &str) -> TestRes {
    app.execute_command(
        "table.paste",
        json!({
            "sheet_id": sheet_id.as_str(),
            "text": tsv,
            "start_row": 0,
            "start_col": 0,
            "start_subcol": 0,
        }),
    )
    .map(|_| ())
    .map_err(|e| format!("table.paste failed: {e}"))
}

fn dispatch_grid_custom(driver: &mut HumanPathDriver, payload: &str) -> Result<UiEventOutcome, String> {
    let mut shards = driver.app().view_shards.clone();
    let out = dispatch_custom_view_action(driver.app_mut(), &mut shards, "grid_core", payload)?;
    driver.app_mut().view_shards = shards;
    Ok(out)
}

fn first_col_title(ds: &curvebook_doc::DataSheet) -> Option<&str> {
    match &ds.archetype {
        TableArchetype::Xy(xy) => xy.data_sets.first().map(|c| c.title.as_str()),
        TableArchetype::Column(c) => c.data_sets.first().map(|c| c.title.as_str()),
        TableArchetype::Grouped(g) => g.data_sets.first().map(|c| c.title.as_str()),
        TableArchetype::Nested(n) => n.groups.first().map(|g| g.title.as_str()),
        _ => None,
    }
}

#[test]
fn r10_8_all_archetypes_multi_row_headers_spans_and_tabular_numerals() -> TestRes {
    let mut app = UiApp::new();
    let metrics = default_viewport();

    let xy_id = create_sheet(
        &mut app,
        json!({
            "title": "XY MeanSD",
            "kind": "xy",
            "subcolumn_mode": "mean_sd_n",
            "cols": 1,
            "rows": 5,
            "auto_family": false
        }),
    )?;
    paste_cells(&mut app, &xy_id, "1.0\t12.5\t1.2\t4\n2.0\t24.0\t2.5\t4")?;

    let mut driver = new_human_driver(app, metrics);
    let tree = driver.layout_tree();
    let run_texts: Vec<&str> = tree.text_runs.iter().map(|r| r.text.as_str()).collect();
    if !run_texts.contains(&"X") || !run_texts.iter().any(|t| *t == "Y" || t.starts_with("Y (")) {
        return Err(format!("expected XY Tier 1 identifier bands X and Y in {run_texts:?}"));
    }
    for sub in ["Mean", "SD", "N"] {
        if !run_texts.contains(&sub) {
            return Err(format!("expected XY MeanSdN Tier 3 header `{sub}` in {run_texts:?}"));
        }
    }
    let has_tabular = tree.text_runs.iter().any(|r| r.text == "12.5" && r.style.tabular_nums);
    if !has_tabular {
        return Err(String::from("expected numeric cell `12.5` rendered with tabular_nums=true"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    assert_input_to_paint_budget(&driver, 50.0)?;

    // Grouped super-column spans across Y1..Y3 replicates.
    let grp_id = create_sheet(
        driver.app_mut(),
        json!({
            "title": "Grouped Rep3",
            "kind": "grouped",
            "replicates": 3,
            "cols": 2,
            "rows": 4,
            "auto_family": false
        }),
    )?;
    paste_cells(driver.app_mut(), &grp_id, "10\t11\t12\t20\t21\t22")?;
    let grp_tree = driver.layout_tree();
    let span0 = grp_tree.node("grid.col_super.0").ok_or_else(|| String::from("missing grid.col_super.0 on Grouped table"))?;
    let sub0 = grp_tree.node("grid.subcol_title.0.0").ok_or_else(|| String::from("missing grid.subcol_title.0.0"))?;
    let sub2 = grp_tree.node("grid.subcol_title.0.2").ok_or_else(|| String::from("missing grid.subcol_title.0.2"))?;
    if span0.bounds.rect_px.width < sub0.bounds.rect_px.width + sub2.bounds.rect_px.width {
        return Err(format!("expected Grouped super-column span width ({}) >= sum of subcolumns", span0.bounds.rect_px.width));
    }
    if !grp_tree.text_runs.iter().any(|r| r.text == "Group A")
        || !grp_tree.text_runs.iter().any(|r| r.text == "Y1")
        || !grp_tree.text_runs.iter().any(|r| r.text == "Y3")
    {
        return Err(String::from("expected Grouped super-span Group A and subcolumns Y1..Y3"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // Nested table super-spans + subcolumn headers + remaining 5 archetypes.
    let nested_id = create_sheet(
        driver.app_mut(),
        json!({
            "title": "Nested Demo",
            "kind": "nested",
            "replicates": 2,
            "cols": 2,
            "rows": 3,
            "auto_family": false
        }),
    )?;
    let _ = driver
        .app_mut()
        .execute_command("table.setSubcolumnTitles", json!({ "sheet_id": nested_id.as_str(), "col": 0, "titles": ["Donor 1", "Donor 2"] }))
        .map_err(|e| format!("setSubcolumnTitles failed: {e}"))?;
    let nested_tree = driver.layout_tree();
    if !nested_tree.text_runs.iter().any(|r| r.text == "Donor 1") || !nested_tree.text_runs.iter().any(|r| r.text == "Group A") {
        return Err(String::from("expected Nested Tier 1 Group A and Tier 3 Donor 1"));
    }

    for (title, kind) in [
        ("Col Sheet", "column"),
        ("Cont Sheet", "contingency"),
        ("Surv Sheet", "survival"),
        ("Parts Sheet", "parts_of_whole"),
        ("MV Sheet", "multiple_variables"),
    ] {
        let _ = create_sheet(driver.app_mut(), json!({ "title": title, "kind": kind, "cols": 2, "rows": 3, "auto_family": false }))?;
        let _ = assert_zero_shortcut_compliance(&mut driver)?;
        if kind == "multiple_variables" {
            let mv_tree = driver.layout_tree();
            if mv_tree.node("formula_bar.input").is_none() || mv_tree.node("formula_bar.open_col.0").is_none() {
                return Err(String::from("MultipleVariables grid must preserve formula_bar.input and formula_bar.open_col.0 seams"));
            }
        }
    }
    Ok(())
}

#[test]
fn r10_8_editable_dataset_x_row_and_subcolumn_titles_human_path() -> TestRes {
    let mut app = UiApp::new();
    let xy_id = create_sheet(
        &mut app,
        json!({
            "title": "Editable Titles",
            "kind": "xy",
            "cols": 2,
            "rows": 4,
            "auto_family": false
        }),
    )?;
    let mut driver = new_human_driver(app, default_viewport());

    // 1. Click dataset column header -> type -> click Apply.
    let _ = driver.click_widget("grid.col_title.0")?;
    if driver.layout_tree().node("grid.title_editor.input").is_none() {
        return Err(String::from("expected floating grid.title_editor.input after clicking column header"));
    }
    let _ = driver.press_key("A", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    let _ = driver.type_text("Vehicle")?;
    let _ = driver.click_widget("grid.title_editor.apply")?;

    let ds_after_col = driver.app().document().data_sheet(&xy_id).ok_or_else(|| String::from("missing xy sheet"))?;
    if first_col_title(ds_after_col) != Some("Vehicle") {
        return Err(format!("expected column 0 title updated to `Vehicle`, got {:?}", first_col_title(ds_after_col)));
    }

    // 2. Click X header -> edit -> commit via Enter key handler.
    let _ = driver.click_widget("grid.x_title.0")?;
    let _ = driver.press_key("A", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    let _ = driver.type_text("Dose (nM)")?;
    let mut shards = driver.app().view_shards.clone();
    let enter_out = handle_grid_key_down(driver.app_mut(), &mut shards.grid_core, "Enter", KeyModifiers::default())?;
    driver.app_mut().view_shards = shards;
    if !enter_out.handled {
        return Err(String::from("expected Enter in title editor to commit"));
    }
    let _ = driver.layout_tree();
    if !driver.layout_tree().text_runs.iter().any(|r| r.text == "Dose (nM)") {
        return Err(String::from("expected updated X title `Dose (nM)` rendered in grid"));
    }

    // 3. Edit Row Title cell -> verify Row Titles column reflects updated label.
    let _ = driver.click_widget("grid.row_title.0")?;
    let _ = driver.type_text("Baseline")?;
    let _ = driver.click_widget("grid.title_editor.apply")?;
    let row_tree = driver.layout_tree();
    let row0_node = row_tree.node("grid.row_title.0").ok_or_else(|| String::from("missing grid.row_title.0"))?;
    if !row0_node.a11y.label.contains("Baseline") {
        return Err(format!("expected updated row title `Baseline`, got {:?}", row0_node.a11y.label));
    }

    // 4. Nested subcolumn title editing via Tier 3 button + Cancel check.
    let nest_id = create_sheet(
        driver.app_mut(),
        json!({ "title": "Nested Edit", "kind": "nested", "cols": 1, "replicates": 2, "rows": 2, "auto_family": false }),
    )?;
    let _ = driver.click_widget("grid.subcol_title.0.0")?;
    let _ = driver.press_key("A", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    let _ = driver.type_text("Cohort Alpha")?;
    let _ = driver.click_widget("grid.title_editor.apply")?;
    let nest_tree = driver.layout_tree();
    let nest_ds_updated = driver.app().document().data_sheet(&nest_id).ok_or_else(|| String::from("missing nested sheet after subcol edit"))?;
    let sub0_title = match &nest_ds_updated.archetype {
        TableArchetype::Nested(n) => n.groups.first().and_then(|g| g.subcolumn_titles.first()).map(String::as_str),
        _ => None,
    };
    if sub0_title != Some("Cohort Alpha") || !nest_tree.text_runs.iter().any(|r| r.text.contains("Cohort Alpha")) {
        return Err(format!("expected Nested subcolumn header `Cohort Alpha` (doc={sub0_title:?}) in grid"));
    }

    let _ = driver.click_widget("grid.col_title.0")?;
    let _ = driver.type_text("Discarded")?;
    let _ = driver.click_widget("grid.title_editor.cancel")?;
    let nest_ds = driver.app().document().data_sheet(&nest_id).ok_or_else(|| String::from("missing nested sheet"))?;
    if first_col_title(nest_ds) == Some("Discarded") {
        return Err(String::from("Cancel button must not commit edited column title"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn r10_8_row_titles_collapsible_resizable_and_column_edge_resize() -> TestRes {
    let mut app = UiApp::new();
    let _ = create_sheet(&mut app, json!({ "title": "Resizable Grid", "kind": "column", "cols": 3, "rows": 6, "auto_family": false }))?;
    let mut driver = new_human_driver(app, default_viewport());

    let t0 = driver.layout_tree();
    let corner_x = t0.node("grid.corner.row_num").ok_or_else(|| String::from("missing grid.corner.row_num"))?;
    let rt_band = t0.node("grid.row_titles.band").ok_or_else(|| String::from("missing grid.row_titles.band"))?;
    if rt_band.bounds.rect_px.x < corner_x.bounds.rect_px.right() {
        return Err(String::from("Row Titles column must sit strictly right of row-number gutter"));
    }
    let rt_w0 = rt_band.bounds.rect_px.width;
    let col0_w0 = t0.node("grid.cell.0.0.0").ok_or_else(|| String::from("missing grid.cell.0.0.0"))?.bounds.rect_px.width;

    // Click Row Titles resize handle -> increases rendered width & state.
    let _ = driver.click_widget("grid.row_titles.resize")?;
    let t1 = driver.layout_tree();
    let rt_w1 = t1.node("grid.row_titles.band").ok_or_else(|| String::from("missing grid.row_titles.band after resize"))?.bounds.rect_px.width;
    if rt_w1 <= rt_w0 || driver.app().view_shards.grid_core.row_titles_width_px == 0 {
        return Err(format!("expected Row Titles resize to widen ({rt_w0} -> {rt_w1})"));
    }

    // Collapse Row Titles -> toggles state, hides grid.row_title.0, preserves toggle strip.
    let _ = driver.click_widget("grid.row_titles.toggle")?;
    if !driver.app().view_shards.grid_core.row_titles_collapsed {
        return Err(String::from("expected row_titles_collapsed == true after toggle"));
    }
    let t2 = driver.layout_tree();
    if t2.node("grid.row_title.0").is_some() {
        return Err(String::from("collapsed Row Titles must not emit row title buttons"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // Expand Row Titles back.
    let _ = driver.click_widget("grid.row_titles.toggle")?;
    if driver.app().view_shards.grid_core.row_titles_collapsed || driver.layout_tree().node("grid.row_title.0").is_none() {
        return Err(String::from("expected Row Titles to expand back cleanly"));
    }

    // Column-edge resize handle + signed delta updates width cleanly.
    let _ = driver.click_widget("grid.col_resize.0.0")?;
    let _ = dispatch_grid_custom(&mut driver, "col:resize_delta:0:0:24")?;
    let t3 = driver.layout_tree();
    let col0_w1 = t3.node("grid.cell.0.0.0").ok_or_else(|| String::from("missing grid.cell.0.0.0 after resize"))?.bounds.rect_px.width;
    if col0_w1 <= col0_w0 {
        return Err(format!("expected column width growth ({col0_w0} -> {col0_w1})"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_two_axis_scroll_scrollbar_and_never_canvas_zoom() -> TestRes {
    let mut app = UiApp::new();
    let wide_id = create_sheet(&mut app, json!({ "title": "Wide Tall Table", "kind": "column", "cols": 24, "rows": 80, "auto_family": false }))?;
    let mut driver = new_human_driver(app, default_viewport());
    let container_id = format!("grid.sheet.{}", wide_id.as_str());
    let initial_zoom = driver.app().canvas_dialogs.canvas.zoom;

    let tree0 = driver.layout_tree();
    if tree0.node(&format!("{container_id}.vscroll")).is_none() || tree0.node(&format!("{container_id}.hscroll")).is_none() {
        return Err(String::from("expected both vertical and horizontal scrollbars on overflowing grid"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // 2-axis wheel scroll inside grid only.
    let _ = driver.wheel_widget(&container_id, 220.0, 240.0, KeyModifiers::default())?;
    let off = driver.app().interaction.scroll_offsets.offset(&container_id);
    if off[0] <= 0.0 || off[1] <= 0.0 {
        return Err(format!("expected 2-axis wheel scroll to advance both axes, got {off:?}"));
    }
    if (driver.app().canvas_dialogs.canvas.zoom - initial_zoom).abs() > 1e-9 {
        return Err(String::from("scrolling over data grid must never change canvas zoom"));
    }

    let tree1 = driver.layout_tree();
    if tree1.node("grid.cell.0.0.0").is_some() {
        return Err(String::from("expected offscreen origin cell (0,0,0) virtualized out after 2-axis scroll"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_keyboard_navigation_selection_boundary_and_clear() -> TestRes {
    let mut app = UiApp::new();
    let sid = create_sheet(&mut app, json!({ "title": "Nav Grid", "kind": "column", "cols": 3, "rows": 5, "auto_family": false }))?;
    paste_cells(&mut app, &sid, "10\t20\t30\n40\t50\t60\n70\t80\t90\n\t\t\n100\t110\t120")?;
    let mut driver = new_human_driver(app, default_viewport());
    driver.app_mut().grid.select_single_sync(GridCellCoord::new(0, 0, 0));

    let run_key = |driver: &mut HumanPathDriver, key: &str, mods: KeyModifiers| -> Result<(), String> {
        let mut shards = driver.app().view_shards.clone();
        let out = handle_grid_key_down(driver.app_mut(), &mut shards.grid_core, key, mods)?;
        driver.app_mut().view_shards = shards;
        if !out.handled {
            return Err(format!("key {key} was not handled"));
        }
        Ok(())
    };

    // ArrowRight -> (0,1,0), ArrowDown -> (1,1,0), Shift+Tab -> (1,0,0), Tab wraps row.
    run_key(&mut driver, "ArrowRight", KeyModifiers::default())?;
    if driver.app().grid.tool.active != Some(GridCellCoord::new(0, 1, 0)) {
        return Err(format!("expected (0,1,0) after ArrowRight, got {:?}", driver.app().grid.tool.active));
    }
    run_key(&mut driver, "ArrowDown", KeyModifiers::default())?;
    if driver.app().grid.tool.active != Some(GridCellCoord::new(1, 1, 0)) {
        return Err(format!("expected (1,1,0) after ArrowDown, got {:?}", driver.app().grid.tool.active));
    }
    run_key(&mut driver, "Tab", KeyModifiers { shift: true, ..KeyModifiers::default() })?;
    if driver.app().grid.tool.active != Some(GridCellCoord::new(1, 0, 0)) {
        return Err(format!("expected (1,0,0) after Shift+Tab, got {:?}", driver.app().grid.tool.active));
    }

    run_key(&mut driver, "Tab", KeyModifiers::default())?;
    run_key(&mut driver, "Tab", KeyModifiers::default())?;
    run_key(&mut driver, "Tab", KeyModifiers::default())?;
    if driver.app().grid.tool.active != Some(GridCellCoord::new(2, 0, 0)) {
        return Err(format!("expected Tab wrap to (2,0,0), got {:?}", driver.app().grid.tool.active));
    }

    run_key(&mut driver, "Enter", KeyModifiers { shift: true, ..KeyModifiers::default() })?;
    if driver.app().grid.tool.active != Some(GridCellCoord::new(1, 0, 0)) {
        return Err(String::from("expected Shift+Enter to move up to row 1"));
    }
    run_key(&mut driver, "Enter", KeyModifiers::default())?;
    if driver.app().grid.tool.active != Some(GridCellCoord::new(2, 0, 0)) {
        return Err(String::from("expected Enter to move down to row 2"));
    }

    run_key(&mut driver, "ArrowUp", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    if driver.app().grid.tool.active != Some(GridCellCoord::new(0, 0, 0)) {
        return Err(format!("expected Cmd+ArrowUp to jump to (0,0,0), got {:?}", driver.app().grid.tool.active));
    }

    run_key(&mut driver, "ArrowDown", KeyModifiers { shift: true, ..KeyModifiers::default() })?;
    let ds_cur = driver.app().document().data_sheet(&sid).ok_or_else(|| String::from("missing nav sheet"))?;
    let sel = driver.app().grid.all_selected_coords(ds_cur);
    if sel.len() != 2 {
        return Err(format!("expected 2 selected cells after Shift+ArrowDown, got {sel:?}"));
    }
    let tree = driver.layout_tree();
    let c0_sel = tree.node("grid.cell.0.0.0").and_then(|n| n.a11y.selected);
    let c1_sel = tree.node("grid.cell.1.0.0").and_then(|n| n.a11y.selected);
    if c0_sel != Some(true) || c1_sel != Some(true) {
        return Err(format!("expected both cells selected in a11y tree, got {c0_sel:?}, {c1_sel:?}"));
    }

    run_key(&mut driver, "Delete", KeyModifiers::default())?;
    let ds_cleared = driver.app().document().data_sheet(&sid).ok_or_else(|| String::from("missing cleared sheet"))?;
    if ds_cleared.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| !c.value.is_empty())
        || ds_cleared.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| !c.value.is_empty())
    {
        return Err(String::from("expected Delete key to clear selected cells"));
    }

    let custom_nav = dispatch_grid_custom(&mut driver, "nav:right")?;
    if !custom_nav.handled {
        return Err(String::from("expected Custom nav:right action to be handled"));
    }
    Ok(())
}

#[test]
fn r10_12_linked_regions_borders_go_to_source_unlink_and_typing_guard() -> TestRes {
    let mut app = UiApp::new();
    let src_id = create_sheet(&mut app, json!({ "title": "Upstream Cohort", "kind": "column", "cols": 2, "rows": 4, "auto_family": false }))?;
    paste_cells(&mut app, &src_id, "11\t21\n12\t22\n13\t23")?;

    let dst_id = create_sheet(&mut app, json!({ "title": "Downstream Linked", "kind": "column", "cols": 3, "rows": 4, "auto_family": false }))?;
    app.execute_command(
        "table.pasteLink",
        json!({
            "target_sheet_id": dst_id.as_str(),
            "source_sheet_id": src_id.as_str(),
            "source_kind": "data",
            "anchor": { "row": 0, "col": 0, "subcol": 0 }
        }),
    )
    .map_err(|e| format!("table.pasteLink failed: {e}"))?;

    // Run real One-Way ANOVA so a true AnalyzableResultSheet exists for a Green provenance border.
    app.execute_command("sheet.select", json!({ "sheet_id": src_id.as_str(), "mode": "navigator" }))
        .map_err(|e| format!("select src failed: {e}"))?;
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" })).map_err(|e| format!("open analyze failed: {e}"))?;
    dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.one_way_anova" } }))
        .map_err(|e| format!("run anova failed: {e}"))?;
    let res_id = app.document().result_sheets.last().map(|r| r.id.clone()).ok_or_else(|| String::from("expected result sheet after ANOVA"))?;

    let green_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: res_id,
            kind: InternalLinkKind::AnalyzableResultSheet,
            tab_id: None,
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::Overwrite,
            anchor: CellCoord::new(0, 2, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    app.execute_command(
        "table.pasteLink",
        json!({
            "target_sheet_id": dst_id.as_str(),
            "link_spec": serde_json::to_value(&green_spec).map_err(|e| e.to_string())?,
            "anchor": { "row": 0, "col": 2, "subcol": 0 }
        }),
    )
    .map_err(|e| format!("table.pasteLink green failed: {e}"))?;
    app.execute_command("sheet.select", json!({ "sheet_id": dst_id.as_str(), "mode": "navigator" }))
        .map_err(|e| format!("select dst failed: {e}"))?;

    let mut driver = new_human_driver(app, default_viewport());
    let tree = driver.layout_tree();
    if tree.node("grid.link_badge.0").is_none() || tree.node("grid.link_badge.1").is_none() {
        return Err(String::from("expected link provenance strip chips for both linked regions"));
    }

    let blue_rgba = provenance_border_rgba(LinkBorderColor::Blue);
    let green_rgba = provenance_border_rgba(LinkBorderColor::Green);
    let has_blue = tree
        .paint_ops
        .iter()
        .any(|op| matches!(op.kind, PaintOpKind::FillRect { color, .. } | PaintOpKind::StrokeRect { color, .. } if color == blue_rgba));
    let has_green = tree
        .paint_ops
        .iter()
        .any(|op| matches!(op.kind, PaintOpKind::FillRect { color, .. } | PaintOpKind::StrokeRect { color, .. } if color == green_rgba));
    if !has_blue || !has_green {
        return Err(String::from("expected colour-coded Blue and Green LinkedDataRegion borders"));
    }

    // Attempt typing & double-clicking inside linked cell -> blocks edit and surfaces popover prompt.
    driver.app_mut().grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    let mut shards = driver.app().view_shards.clone();
    let type_out = handle_grid_text_input(driver.app(), &mut shards.grid_core, "99");
    let dbl_out = handle_grid_double_click(driver.app(), &mut shards.grid_core, GridCellCoord::new(0, 0, 0));
    driver.app_mut().view_shards = shards;
    if !type_out.handled || !dbl_out.handled {
        return Err(String::from("expected linked-cell typing and double-click to be blocked"));
    }
    let pop_tree = driver.layout_tree();
    if pop_tree.node("grid.link_popover.panel").is_none()
        || pop_tree.node("grid.link_popover.goto").is_none()
        || pop_tree.node("grid.link_popover.unlink").is_none()
    {
        return Err(String::from("expected Go To Source / Unlink popover when editing linked cell"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // Click Go To Source -> switches active sheet to upstream sheet.
    let _ = driver.click_widget("grid.link_popover.goto")?;
    if driver.app().document().active_sheet_id.as_ref() != Some(&src_id) {
        return Err(format!("expected Go To Source to activate {src_id:?}, got {:?}", driver.app().document().active_sheet_id));
    }

    // Return to downstream sheet, open popover via badge click, click Unlink -> preserves values and detaches link.
    driver
        .app_mut()
        .execute_command("sheet.select", json!({ "sheet_id": dst_id.as_str(), "mode": "navigator" }))
        .map_err(|e| format!("select dst failed: {e}"))?;
    let _ = driver.click_widget("grid.link_badge.0")?;
    let _ = driver.click_widget("grid.link_popover.unlink")?;
    let ds_unlinked = driver.app().document().data_sheet(&dst_id).ok_or_else(|| String::from("missing unlinked sheet"))?;
    if !ds_unlinked.linked_regions.is_empty() {
        return Err(String::from("expected Unlink button to detach linked regions"));
    }
    if ds_unlinked.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.value.finite_f64()) != Some(11.0) {
        return Err(String::from("expected Unlink to preserve cell values"));
    }

    // Verify Delete/Clear works cleanly once unlinked.
    let mut shards2 = driver.app().view_shards.clone();
    let del_after = handle_grid_key_down(driver.app_mut(), &mut shards2.grid_core, "Delete", KeyModifiers::default())?;
    driver.app_mut().view_shards = shards2;
    if del_after.command_id.as_deref() != Some("table.clearCells") {
        return Err(format!("expected clearCells to succeed after unlink, got {del_after:?}"));
    }
    Ok(())
}

#[test]
fn r10_12_excluded_cells_strikethrough_tag_and_hover_provenance_tooltip() -> TestRes {
    let mut app = UiApp::new();
    let sid = create_sheet(&mut app, json!({ "title": "Excluded Sheet", "kind": "column", "cols": 2, "rows": 4, "auto_family": false }))?;
    paste_cells(&mut app, &sid, "42.5\t10.0\n44.0\t12.0")?;
    app.execute_command(
        "table.excludeCellsWithReason",
        json!({
            "sheet_id": sid.as_str(),
            "coords": [[0, 0, 0]],
            "reason_tag": "Outlier",
            "note": "Sensor spike > 3 SD"
        }),
    )
    .map_err(|e| format!("excludeCellsWithReason failed: {e}"))?;

    let mut driver = new_human_driver(app, default_viewport());
    let tree = driver.layout_tree();

    // Excluded cell shows italic `*` marker, strikethrough LineSegment, and corner tag FillRect.
    let has_italic_star =
        tree.text_runs.iter().any(|r| r.owner_widget_id.as_str() == "grid.cell.0.0.0" && r.text.contains('*') && r.style.slope == SpanSlope::Italic);
    let has_strike_line = tree
        .paint_ops
        .iter()
        .any(|op| op.owner_widget_id.as_ref().is_some_and(|o| o.as_str() == "grid.cell.0.0.0") && matches!(op.kind, PaintOpKind::LineSegment { .. }));
    let excluded_rgba = curvebook_ui_egui::theme::UiThemeSpec::for_mode(UiThemeMode::Light).excluded_cell_text;
    let has_tag_fill = tree.paint_ops.iter().any(|op| {
        op.owner_widget_id.as_ref().is_some_and(|o| o.as_str() == "grid.cell.0.0.0")
            && matches!(op.kind, PaintOpKind::FillRect { rect, color } if rect.width == 5 && rect.height == 5 && color == excluded_rgba)
    });
    if !has_italic_star || !has_strike_line || !has_tag_fill {
        return Err(format!("expected excluded styling (italic_star={has_italic_star}, strike={has_strike_line}, note_tag={has_tag_fill})"));
    }

    // Hovering excluded cell sets hover_exclusion_tooltip and renders Tooltip overlay text.
    let _ = driver.hover_widget("grid.cell.0.0.0")?;
    let tip = driver
        .app()
        .grid
        .hover_exclusion_tooltip
        .clone()
        .ok_or_else(|| String::from("expected hover_exclusion_tooltip after hovering excluded cell"))?;
    if !tip.contains("Outlier") || !tip.contains("Sensor spike > 3 SD") {
        return Err(format!("expected exclusion reason/note in tooltip, got `{tip}`"));
    }
    let hover_tree = driver.layout_tree();
    let has_tooltip_run = hover_tree.text_runs.iter().any(|r| r.z_layer == ZLayer::Tooltip && r.text.contains("Outlier"));
    if !has_tooltip_run {
        return Err(String::from("expected rendered ZLayer::Tooltip text containing `Outlier`"));
    }

    // Moving pointer onto a non-excluded cell clears provenance tooltip.
    let _ = driver.hover_widget("grid.cell.1.0.0")?;
    let mut shards = driver.app().view_shards.clone();
    handle_grid_pointer_hover(driver.app_mut(), &mut shards.grid_core, Some(GridCellCoord::new(1, 0, 0)));
    driver.app_mut().view_shards = shards;
    if driver.app().grid.hover_exclusion_tooltip.is_some() {
        return Err(String::from("expected tooltip cleared on non-excluded cell"));
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(())
}

#[test]
fn r10_8_split_pane_r2_s10_09_rtl_hidpi_and_w5_i18n_catalog_integrity() -> TestRes {
    let mut app = UiApp::new();
    let primary = create_sheet(&mut app, json!({ "title": "Primary Split", "kind": "xy", "cols": 1, "rows": 4, "auto_family": false }))?;
    let secondary = create_sheet(&mut app, json!({ "title": "Secondary Split", "kind": "column", "cols": 2, "rows": 4, "auto_family": false }))?;
    app.execute_command("sheet.select", json!({ "sheet_id": primary.as_str(), "mode": "navigator" }))
        .map_err(|e| format!("select primary failed: {e}"))?;
    app.split = SplitWorkspaceState {
        enabled: true,
        ratio: 0.5,
        primary_sheet_id: Some(primary.clone()),
        secondary_sheet_id: Some(secondary),
        dragging_divider: false,
    };

    let mut driver = new_human_driver(app, default_viewport());
    let split_tree = driver.layout_tree();
    if split_tree.grid_geometry.as_ref().is_none_or(|g| g.sheet_id != primary) {
        return Err(format!(
            "R2-S10-09 violation: expected primary grid_geometry {:?}, got {:?}",
            primary,
            split_tree.grid_geometry.as_ref().map(|g| &g.sheet_id)
        ));
    }
    // Verify zero duplicate WidgetIds across primary + secondary grid panes.
    let mut seen = BTreeSet::new();
    for node in &split_tree.nodes {
        if !seen.insert(node.id.as_str().to_owned()) {
            return Err(format!("duplicate WidgetId `{}` in split workspace", node.id.as_str()));
        }
    }
    let _ = assert_zero_shortcut_compliance(&mut driver)?;

    // Sweep RTL ("ar"), HiDPI (1.0 / 2.0), and Light/Dark/HighContrast themes.
    driver.app_mut().split.enabled = false;
    for (locale, scale, mode) in
        [("en", 1.0, UiThemeMode::Light), ("en", 2.0, UiThemeMode::Dark), ("ar", 1.0, UiThemeMode::HighContrast), ("ar", 2.0, UiThemeMode::Light)]
    {
        driver.app_mut().locale = String::from(locale);
        driver.app_mut().theme = curvebook_ui_egui::theme::UiThemeSpec::for_mode(mode);
        let mut m = default_viewport();
        m.scale_factor = scale;
        driver.set_metrics(m);
        if (scale - 1.0).abs() < 1e-6 {
            let _ = assert_zero_shortcut_compliance(&mut driver)?;
        } else {
            let report = driver.verify_parity();
            let grid_violations: Vec<_> = report.violations.iter().filter(|v| v.widget_id.starts_with("grid.")).collect();
            if !grid_violations.is_empty() {
                return Err(format!("HiDPI scale={scale} grid_core parity violations: {grid_violations:?}"));
            }
        }
    }

    // Verify ViewShardStates serialization compatibility for GridCoreViewState.
    let mut shards = ViewShardStates::default();
    shards.grid_core.set_row_titles_width(148);
    shards.grid_core.set_data_col_width(0, 0, 120);
    let json_val = serde_json::to_value(&shards.grid_core).map_err(|e| e.to_string())?;
    let roundtrip: GridCoreViewState = serde_json::from_value(json_val).map_err(|e| e.to_string())?;
    if roundtrip.row_titles_width_px != 148 || roundtrip.effective_data_col_width(0, 0) != 120 {
        return Err(String::from("GridCoreViewState JSON roundtrip mismatch"));
    }

    // Validate 16-locale coverage and cleanroom script hygiene of w5_grid_core.rs.
    let catalog_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/i18n/w5_grid_core.rs");
    let src = std::fs::read_to_string(&catalog_path).map_err(|e| format!("failed reading {}: {e}", catalog_path.display()))?;
    for req_locale in ["en:", "de:", "es:", "fr:", "ja:", "ko:", "pt_br:", "zh_cn:", "zh_tw:", "it:", "nl:", "pl:", "ru:", "tr:", "ar:", "he:"] {
        if !src.contains(req_locale) {
            return Err(format!("w5_grid_core.rs missing locale `{req_locale}`"));
        }
    }
    let has_cyrillic = src.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
    let has_arabic = src.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
    let has_hebrew = src.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c));
    if !has_cyrillic || !has_arabic || !has_hebrew {
        return Err(String::from("w5_grid_core.rs must contain native Cyrillic, Arabic, and Hebrew scripts"));
    }
    let lower = src.to_ascii_lowercase();
    for forbidden in ["graphpad", "prism", "sigmaplot", "originlab"] {
        if lower.contains(forbidden) {
            return Err(format!("banned external brand `{forbidden}` found in w5_grid_core.rs"));
        }
    }
    let _ = Rgba8::rgb(0, 0, 0);
    let _ = WidgetAction::NoOp;
    Ok(())
}
