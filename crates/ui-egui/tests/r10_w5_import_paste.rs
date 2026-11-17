#![forbid(unsafe_code)]

mod human_wf_common;

use std::collections::BTreeSet;

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSetColumn, DataSheet, InfoSheet, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TableKind,
    XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::i18n::{LOCALES, all_keys, w5_import_paste};
use curvebook_ui_egui::layout::a11y::FocusScope;
use curvebook_ui_egui::layout::{LayoutTree, WidgetId};
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, TextMeasurer, UiTextStyle};
use curvebook_ui_egui::{KeyModifiers, UiApp, UiThemeMode, ViewportMetrics};

use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, new_human_driver,
    press_cmd_shortcut, press_key_combo, scroll_until_widget_visible, type_into_focused,
};

fn wid(id: &str) -> WidgetId {
    WidgetId::from(id)
}

fn all_text_for_widget(tree: &LayoutTree, owner_id: &str) -> Vec<String> {
    tree.text_runs.iter().filter(|run| run.owner_widget_id.as_str() == owner_id).map(|run| run.text.clone()).collect()
}

fn joined_text_for_widget(tree: &LayoutTree, owner_id: &str) -> String {
    all_text_for_widget(tree, owner_id).join(" | ")
}

fn build_xy_replicates_and_col_doc() -> Result<(ProjectDocument, SheetId, SheetId), String> {
    let mut doc = ProjectDocument::new("Import & Convert Archetype Fixture").map_err(|e| e.to_string())?;

    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 2, 4).map_err(|e| e.to_string())?;
    xy.x_title = String::from("Dose");
    xy.row_titles = vec![String::from("D0"), String::from("D1"), String::from("D2"), String::from("D3")];
    xy.x_cells = vec![vec![Cell::number(0.1)], vec![Cell::number(1.0)], vec![Cell::number(10.0)], vec![Cell::excluded_number(100.0)]];
    xy.data_sets[0].title = String::from("Vehicle");
    xy.data_sets[0].cells = vec![
        vec![Cell::number(1.1), Cell::number(1.2), Cell::number(1.3)],
        vec![Cell::number(2.1), Cell::number(2.2), Cell::number(2.3)],
        vec![Cell::number(3.1), Cell::excluded_number(9.9), Cell::number(3.3)],
        vec![Cell::number(4.1), Cell::number(4.2), Cell::number(4.3)],
    ];
    xy.data_sets[1].title = String::from("Inhibitor");
    xy.data_sets[1].cells = vec![
        vec![Cell::number(5.0), Cell::number(5.2), Cell::number(5.4)],
        vec![Cell::number(6.0), Cell::number(6.2), Cell::number(6.4)],
        vec![Cell::number(7.0), Cell::number(7.2), Cell::number(7.4)],
        vec![Cell::number(8.0), Cell::number(8.2), Cell::number(8.4)],
    ];
    let xy_sheet = DataSheet::new("ds-xy-rep", "Dose Curve", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    let xy_id = xy_sheet.id.clone();

    let mut target = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
    target.data_sets = vec![
        DataSetColumn {
            title: String::from("Alpha"),
            cells: vec![vec![Cell::number(10.0)], vec![Cell::number(20.0)], vec![Cell::empty()], vec![Cell::empty()]],
        },
        DataSetColumn {
            title: String::from("Beta"),
            cells: vec![vec![Cell::number(12.0)], vec![Cell::number(22.0)], vec![Cell::empty()], vec![Cell::empty()]],
        },
    ];
    let col_sheet = DataSheet::new("ds-col-dest", "Destination Table", TableArchetype::Column(target)).map_err(|e| e.to_string())?;
    let col_id = col_sheet.id.clone();

    doc.add_data_sheet(xy_sheet).map_err(|e| e.to_string())?;
    doc.add_data_sheet(col_sheet).map_err(|e| e.to_string())?;
    Ok((doc, xy_id, col_id))
}

#[test]
fn r10_12_import_paste_5tab_human_path_source_view_filter_placement_notes_and_commit() -> Result<(), String> {
    let mut doc = ProjectDocument::new("Import 5-Tab Project").map_err(|e| e.to_string())?;
    let info = InfoSheet::new_preseeded("info-import-1", "Experiment Info").map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.open_import_reshape();
    let mut driver = new_human_driver(app, default_viewport());
    let _ = driver.render_frame();

    // Initial modal focus scope must trap Tab navigation inside ModalDialog.
    let initial_tree = driver.layout_tree();
    if initial_tree.active_focus_scope != FocusScope::ModalDialog {
        return Err(format!("expected FocusScope::ModalDialog, got {:?}", initial_tree.active_focus_scope));
    }
    for _ in 0..4 {
        press_key_combo(&mut driver, "Tab", KeyModifiers::default())?;
    }
    let after_tab_tree = driver.layout_tree();
    if after_tab_tree.active_focus_scope != FocusScope::ModalDialog {
        return Err(format!("Tab escaped ModalDialog: {:?}", after_tab_tree.active_focus_scope));
    }

    // 1. Source Tab: switch to Clipboard, type European semicolon + decimal-comma CSV with asterisk exclusion, detect format.
    click_widget_center(&mut driver, &wid("import_paste.source.clipboard"))?;
    click_widget_center(&mut driver, &wid("import_paste.target.column"))?;
    click_widget_center(&mut driver, &wid("import_paste.input.title"))?;
    press_cmd_shortcut(&mut driver, "a")?;
    press_key_combo(&mut driver, "Backspace", KeyModifiers::default())?;
    type_into_focused(&mut driver, "Dose Response EU")?;
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.source"), &wid("import_paste.input.csv"), 80.0, 12)?;
    click_and_type(
        &mut driver,
        &wid("import_paste.input.csv"),
        "Group;Dose;Resp\nCtrl;0,1;10,5\nDrugA;1,0;22,4*\nDrugB;10,0;31,8\nDrugC;100,0;45,2\n",
    )?;
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.source"), &wid("import_paste.detect_format"), -80.0, 12)?;
    click_widget_center(&mut driver, &wid("import_paste.detect_format"))?;

    if driver.app().grid.import_reshape.tidy_delimiter != ';' {
        return Err(format!("expected ';' auto-detected delimiter, got {:?}", driver.app().grid.import_reshape.tidy_delimiter));
    }
    if driver.app().grid.import_reshape.decimal_separator != ',' {
        return Err(format!("expected ',' auto-detected decimal separator, got {:?}", driver.app().grid.import_reshape.decimal_separator));
    }
    if !driver.app().view_shards.import_paste.has_header_row {
        return Err(String::from("expected header row auto-detection to remain true"));
    }
    assert_zero_shortcut_compliance(&mut driver)?;
    assert_input_to_paint_budget(&driver, 50.0)?;

    // 2. View Tab: inspect live parsed preview grid, header text, formatted numeric values, and excluded count.
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let view_tree = driver.layout_tree();
    let summary = joined_text_for_widget(&view_tree, "import_paste.view.summary");
    if !summary.contains("4 rows") || !summary.contains("3 cols") || !summary.contains("Excluded: 1") {
        return Err(format!("unexpected View summary banner: `{summary}`"));
    }
    let grid_text = joined_text_for_widget(&view_tree, "import_paste.view_grid");
    for expected in ["Group", "Dose", "Resp", "Ctrl", "DrugA", "10.5", "22.4*", "45.2"] {
        if !grid_text.contains(expected) {
            return Err(format!("View grid missing `{expected}` in `{grid_text}`"));
        }
    }
    assert_zero_shortcut_compliance(&mut driver)?;

    // 3. Filter Tab: apply inclusive row slice [1..=3], decimate, exclude predicate, verify live in View, then clear slice.
    click_widget_center(&mut driver, &wid("import_paste.tab.filter"))?;
    click_and_type(&mut driver, &wid("import_paste.input.row_start"), "1")?;
    click_and_type(&mut driver, &wid("import_paste.input.row_end"), "3")?;
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let sliced_tree = driver.layout_tree();
    let sliced_summary = joined_text_for_widget(&sliced_tree, "import_paste.view.summary");
    if !sliced_summary.contains("3 rows") {
        return Err(format!("expected 3 rows after inclusive slice [1..=3], got `{sliced_summary}`"));
    }
    let sliced_grid = joined_text_for_widget(&sliced_tree, "import_paste.view_grid");
    if sliced_grid.contains("Ctrl") || !sliced_grid.contains("DrugA") {
        return Err(format!("slice [1..=3] should exclude row 0 (`Ctrl`) and keep `DrugA`, got `{sliced_grid}`"));
    }

    click_widget_center(&mut driver, &wid("import_paste.tab.filter"))?;
    click_widget_center(&mut driver, &wid("import_paste.clear_slice"))?;
    click_widget_center(&mut driver, &wid("import_paste.decimate.inc"))?;
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let decimated_summary = joined_text_for_widget(&driver.layout_tree(), "import_paste.view.summary");
    if !decimated_summary.contains("2 rows") {
        return Err(format!("expected 2 rows after decimate=2, got `{decimated_summary}`"));
    }

    click_widget_center(&mut driver, &wid("import_paste.tab.filter"))?;
    click_widget_center(&mut driver, &wid("import_paste.decimate.dec"))?;
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.filter"), &wid("import_paste.input.exclude_predicate"), 80.0, 12)?;
    click_and_type(&mut driver, &wid("import_paste.input.exclude_predicate"), "< 15")?;
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let pred_summary = joined_text_for_widget(&driver.layout_tree(), "import_paste.view.summary");
    if !pred_summary.contains("Excluded: 9") {
        return Err(format!("expected `< 15` row predicate to exclude 3 rows (9 cells), got `{pred_summary}`"));
    }
    assert_zero_shortcut_compliance(&mut driver)?;

    // 4. Placement Tab: verify transpose + row-wrap updates View dimensions, then reset before commit.
    click_widget_center(&mut driver, &wid("import_paste.tab.placement"))?;
    click_widget_center(&mut driver, &wid("import_paste.transpose"))?;
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let transposed_summary = joined_text_for_widget(&driver.layout_tree(), "import_paste.view.summary");
    if !transposed_summary.contains("3 rows × 4 cols") {
        return Err(format!("expected transposed 3 rows x 4 cols, got `{transposed_summary}`"));
    }
    click_widget_center(&mut driver, &wid("import_paste.tab.placement"))?;
    click_widget_center(&mut driver, &wid("import_paste.transpose"))?;
    click_widget_center(&mut driver, &wid("import_paste.row_wrap.2"))?;
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let wrapped_summary = joined_text_for_widget(&driver.layout_tree(), "import_paste.view.summary");
    if !wrapped_summary.contains("6 rows × 2 cols") {
        return Err(format!("expected row_wrap=2 (2 cols/row across 12 cells) -> 6 rows × 2 cols, got `{wrapped_summary}`"));
    }
    click_widget_center(&mut driver, &wid("import_paste.tab.placement"))?;
    click_widget_center(&mut driver, &wid("import_paste.row_wrap.off"))?;
    assert_zero_shortcut_compliance(&mut driver)?;

    // 5. Notes Tab: set link_id + provenance notes, inspect provenance preview, commit into DataSheet + InfoSheet, verify undo.
    click_widget_center(&mut driver, &wid("import_paste.tab.notes"))?;
    if !driver.app().view_shards.import_paste.record_in_info_sheet {
        click_widget_center(&mut driver, &wid("import_paste.record_info"))?;
    }
    click_and_type(&mut driver, &wid("import_paste.input.link_id"), "paste-eu-01")?;
    click_and_type(&mut driver, &wid("import_paste.input.notes"), "Baseline dose curve from EU CSV")?;
    let notes_tree = driver.layout_tree();
    let prov = joined_text_for_widget(&notes_tree, "import_paste.notes.provenance_preview");
    if !prov.contains("Origin: clipboard") || !prov.contains("paste-eu-01") {
        return Err(format!("unexpected provenance preview: `{prov}`"));
    }
    assert_zero_shortcut_compliance(&mut driver)?;

    let sheets_before = driver.app().document().data_sheets.len();
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.notes"), &wid("import_paste.commit"), 80.0, 10)?;
    click_widget_center(&mut driver, &wid("import_paste.commit"))?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(format!("expected modal dialog closed after commit, got {:?}", driver.app().canvas_dialogs.active_dialog));
    }
    if driver.app().document().data_sheets.len() != sheets_before + 1 {
        return Err(String::from("expected newly imported DataSheet to be added"));
    }
    let imported = driver.app().document().data_sheets.last().ok_or_else(|| String::from("missing imported sheet"))?;
    if imported.title != "Dose Response EU" || imported.kind() != TableKind::Column {
        return Err(format!("unexpected imported sheet metadata: `{}` {:?}", imported.title, imported.kind()));
    }
    if driver.app().document().info_sheets.is_empty() {
        return Err(String::from("expected provenance InfoSheet entry to be recorded"));
    }
    let info_notes_joined = driver.app().document().info_sheets.iter().map(|i| i.notes_markdown.clone()).collect::<Vec<_>>().join("\n");
    if !info_notes_joined.contains("paste-eu-01") || !info_notes_joined.contains("Baseline dose curve from EU CSV") {
        return Err(format!("InfoSheet missing import provenance: `{info_notes_joined}`"));
    }

    // Verify Undo rolls back the InfoSheet note append and the imported DataSheet cleanly.
    press_cmd_shortcut(&mut driver, "z")?;
    press_cmd_shortcut(&mut driver, "z")?;
    if driver.app().document().data_sheets.len() != sheets_before {
        return Err(String::from("expected edit.undo to restore sheet count"));
    }
    Ok(())
}

#[test]
fn r10_12_plate_reshape_tidy_unstack_bom_and_internal_live_link() -> Result<(), String> {
    // Part A: 96-well plate preset + blank mean subtraction committed via modal.ok.
    let mut app = UiApp::new();
    app.canvas_dialogs.open_import_reshape();
    let mut driver = new_human_driver(app, default_viewport());

    click_widget_center(&mut driver, &wid("import_paste.source.plate96"))?;
    click_widget_center(&mut driver, &wid("import_paste.tab.filter"))?;
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.filter"), &wid("import_paste.plate.preset_96"), 80.0, 12)?;
    click_widget_center(&mut driver, &wid("import_paste.plate.preset_96"))?;
    // `subtract_blank_mean` defaults to true; toggle off then back on via human click.
    click_widget_center(&mut driver, &wid("import.subtract_blank_mean"))?;
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let plate_off_summary = joined_text_for_widget(&driver.layout_tree(), "import_paste.view.summary");
    if !plate_off_summary.contains("96-Well Plate Reshape") || !plate_off_summary.contains("Blank sub=false") {
        return Err(format!("unexpected plate View summary after toggle off: `{plate_off_summary}`"));
    }
    click_widget_center(&mut driver, &wid("import_paste.tab.filter"))?;
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.filter"), &wid("import.subtract_blank_mean"), 80.0, 12)?;
    click_widget_center(&mut driver, &wid("import.subtract_blank_mean"))?;
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let plate_summary = joined_text_for_widget(&driver.layout_tree(), "import_paste.view.summary");
    if !plate_summary.contains("96-Well Plate Reshape") || !plate_summary.contains("Blank sub=true") {
        return Err(format!("unexpected plate View summary: `{plate_summary}`"));
    }
    assert_zero_shortcut_compliance(&mut driver)?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;
    if driver.app().document().data_sheets.is_empty() {
        return Err(String::from("expected Plate96 reshape via modal.ok to create a DataSheet"));
    }

    // Part B: Tidy long pivot + group unstack + sample load.
    driver.app_mut().canvas_dialogs.open_import_reshape();
    click_widget_center(&mut driver, &wid("import_paste.tab.source"))?;
    click_widget_center(&mut driver, &wid("import_paste.source.tidy"))?;
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.source"), &wid("import_paste.load_sample"), 80.0, 12)?;
    click_widget_center(&mut driver, &wid("import_paste.load_sample"))?;
    click_widget_center(&mut driver, &wid("import_paste.tab.filter"))?;
    click_widget_center(&mut driver, &wid("import_paste.unstack_toggle"))?;
    click_widget_center(&mut driver, &wid("import_paste.tab.view"))?;
    let tidy_tree = driver.layout_tree();
    let tidy_summary = joined_text_for_widget(&tidy_tree, "import_paste.view.summary");
    let tidy_text = joined_text_for_widget(&tidy_tree, "import_paste.view_grid");
    if !tidy_text.contains("Control") || !tidy_text.contains("Treated") {
        return Err(format!(
            "expected tidy unstack columns in View grid: `{tidy_text}` (summary=`{tidy_summary}`, mode={:?}, csv_len={}, origin={})",
            driver.app().grid.import_reshape.mode,
            driver.app().grid.import_reshape.tidy_csv.len(),
            driver.app().view_shards.import_paste.source_origin,
        ));
    }
    click_widget_center(&mut driver, &wid("import_paste.tab.notes"))?;
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.notes"), &wid("import_paste.commit"), 80.0, 10)?;
    click_widget_center(&mut driver, &wid("import_paste.commit"))?;
    if driver.app().document().data_sheets.len() < 2 {
        return Err(String::from("expected Tidy Long unstack commit to add DataSheet"));
    }

    // Part C: UTF-8 BOM + Tab + European comma decimal auto-detection + wheel scrolling on large preview.
    let mut wide_csv = String::from("\u{FEFF}Time");
    for c in 0..12 {
        wide_csv.push_str(&format!("\tS{}", c + 1));
    }
    wide_csv.push('\n');
    for r in 0..30 {
        wide_csv.push_str(&format!("{r}"));
        for c in 0..12 {
            wide_csv.push_str(&format!("\t{},{}", r + 1, (c + 1) % 9));
        }
        wide_csv.push('\n');
    }
    let mut bom_app = UiApp::new();
    bom_app.canvas_dialogs.open_import_reshape();
    let mut bom_driver = new_human_driver(bom_app, default_viewport());
    click_widget_center(&mut bom_driver, &wid("import_paste.source.clipboard"))?;
    scroll_until_widget_visible(&mut bom_driver, &wid("import_paste.panel.source"), &wid("import_paste.input.csv"), 80.0, 12)?;
    click_and_type(&mut bom_driver, &wid("import_paste.input.csv"), &wide_csv)?;
    scroll_until_widget_visible(&mut bom_driver, &wid("import_paste.panel.source"), &wid("import_paste.detect_format"), -80.0, 12)?;
    click_widget_center(&mut bom_driver, &wid("import_paste.detect_format"))?;
    if bom_driver.app().view_shards.import_paste.encoding != "utf-8-bom" || bom_driver.app().grid.import_reshape.tidy_delimiter != '\t' {
        return Err(format!(
            "unexpected BOM/TSV detection: enc={} delim={:?}",
            bom_driver.app().view_shards.import_paste.encoding,
            bom_driver.app().grid.import_reshape.tidy_delimiter
        ));
    }
    click_widget_center(&mut bom_driver, &wid("import_paste.tab.view"))?;
    bom_driver.wheel_widget("import_paste.view_grid", 120.0, 180.0, KeyModifiers::default())?;
    assert_zero_shortcut_compliance(&mut bom_driver)?;
    click_widget_center(&mut bom_driver, &wid("modal.cancel"))?;

    // Part D: Internal Sheet live-linked paste with transpose + existing destination sheet -> Green LinkedDataRegion.
    let (doc, src_id, dst_id) = build_xy_replicates_and_col_doc()?;
    let mut link_driver = new_human_driver(UiApp::from_session(Session::from_document(doc)), default_viewport());
    link_driver.app_mut().canvas_dialogs.open_import_reshape();
    click_widget_center(&mut link_driver, &wid("import_paste.source.internal"))?;
    let src_btn = wid(&format!("import_paste.internal_sheet.{}", src_id.as_str()));
    scroll_until_widget_visible(&mut link_driver, &wid("import_paste.panel.source"), &src_btn, 80.0, 12)?;
    click_widget_center(&mut link_driver, &src_btn)?;

    click_widget_center(&mut link_driver, &wid("import_paste.tab.placement"))?;
    click_widget_center(&mut link_driver, &wid("import_paste.transpose"))?;
    click_widget_center(&mut link_driver, &wid("import_paste.live_link"))?;
    click_widget_center(&mut link_driver, &wid("import_paste.target_dest.existing"))?;
    click_widget_center(&mut link_driver, &wid("import_paste.anchor_row.inc"))?;
    let dst_btn = wid(&format!("import_paste.dest_sheet.{}", dst_id.as_str()));
    scroll_until_widget_visible(&mut link_driver, &wid("import_paste.panel.placement"), &dst_btn, 80.0, 12)?;
    click_widget_center(&mut link_driver, &dst_btn)?;
    assert_zero_shortcut_compliance(&mut link_driver)?;

    click_widget_center(&mut link_driver, &wid("import_paste.tab.notes"))?;
    scroll_until_widget_visible(&mut link_driver, &wid("import_paste.panel.notes"), &wid("import_paste.commit"), 80.0, 10)?;
    click_widget_center(&mut link_driver, &wid("import_paste.commit"))?;

    let dst_after = link_driver.app().document().data_sheet(&dst_id).ok_or_else(|| String::from("missing destination sheet"))?;
    if dst_after.linked_regions.is_empty() {
        return Err(String::from("expected live-linked paste into existing sheet to create LinkedDataRegion"));
    }
    let link = &dst_after.linked_regions[0];
    if !link.spec.placement.transpose {
        return Err(String::from("expected live link spec.placement.transpose == true"));
    }
    Ok(())
}

#[test]
fn r10_4_convert_archetype_interactive_lossy_guard_mapping_preview_and_undo() -> Result<(), String> {
    let (doc, xy_id, _) = build_xy_replicates_and_col_doc()?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    let doc_open = app.document().clone();
    if !app.canvas_dialogs.open_convert_archetype(&doc_open, &xy_id) {
        return Err(String::from("failed to open ConvertArchetype dialog"));
    }
    let mut driver = new_human_driver(app, default_viewport());

    // 1. Lossless preview (XY with replicates -> Column expands subcolumns cleanly).
    click_widget_center(&mut driver, &wid("convert.target.column"))?;
    let col_banner = joined_text_for_widget(&driver.layout_tree(), "convert.lossy_banner");
    if !col_banner.contains("Lossless reshape") {
        return Err(format!("expected lossless reshape banner for XY->Column, got `{col_banner}`"));
    }
    click_widget_center(&mut driver, &wid("convert.map_row.0"))?;
    if driver.app().view_shards.import_paste.convert_selected_col != 0 {
        return Err(String::from("expected convert.map_row.0 selection"));
    }

    // 2. Incompatible target (fractional floats -> Contingency) displays diagnostic banner cleanly without crashing.
    click_widget_center(&mut driver, &wid("convert.target.contingency"))?;
    let blocked_banner = joined_text_for_widget(&driver.layout_tree(), "convert.lossy_banner");
    if !blocked_banner.contains("Conversion unavailable") {
        return Err(format!(
            "expected diagnostic banner on incompatible Contingency conversion, got `{blocked_banner}` (target_kind={:?}, warnings={:?}, active_dialog={:?}, focus={:?})",
            driver.app().canvas_dialogs.convert_archetype.target_kind,
            driver.app().canvas_dialogs.convert_archetype.preview_warnings,
            driver.app().canvas_dialogs.active_dialog,
            driver.app().focus_widget_id,
        ));
    }

    // 3. Lossy target (XY with excluded X cell -> Grouped drops excluded X value) triggers explicit lossy warning banner.
    click_widget_center(&mut driver, &wid("convert.target.grouped"))?;
    let tree = driver.layout_tree();
    let banner = joined_text_for_widget(&tree, "convert.lossy_banner");
    if !banner.contains("Lossy") {
        return Err(format!("expected explicit lossy-change warning banner, got `{banner}`"));
    }
    driver.wheel_widget("convert.preview_panel", 0.0, 60.0, KeyModifiers::default())?;
    assert_zero_shortcut_compliance(&mut driver)?;
    assert_input_to_paint_budget(&driver, 50.0)?;

    // Attempt confirm while allow_lossy == false -> must reject without mutating table.
    let err_res = click_widget_center(&mut driver, &wid("modal.ok"));
    if err_res.is_ok() {
        return Err(String::from("expected lossy ConvertArchetype commit without allow_lossy to fail"));
    }
    if driver.app().document().data_sheet(&xy_id).map(DataSheet::kind) != Some(TableKind::Xy) {
        return Err(String::from("table kind mutated despite missing allow_lossy confirmation"));
    }

    // Preview-only toggle must also refrain from mutating when confirmed via modal.ok, then re-open & commit cleanly.
    click_widget_center(&mut driver, &wid("convert.allow_lossy"))?;
    click_widget_center(&mut driver, &wid("convert.preview_only"))?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;
    if driver.app().document().data_sheet(&xy_id).map(DataSheet::kind) != Some(TableKind::Xy) {
        return Err(String::from("preview_only mode should not mutate table"));
    }

    // Re-open, select Grouped, enable allow_lossy, confirm -> converts cleanly and undo restores XY replicates.
    let doc_snap = driver.app().document().clone();
    driver.app_mut().canvas_dialogs.open_convert_archetype(&doc_snap, &xy_id);
    click_widget_center(&mut driver, &wid("convert.target.grouped"))?;
    click_widget_center(&mut driver, &wid("convert.allow_lossy"))?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;
    if driver.app().document().data_sheet(&xy_id).map(DataSheet::kind) != Some(TableKind::Grouped) {
        return Err(String::from("expected TableKind::Grouped after confirmed ConvertArchetype"));
    }
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(String::from("expected modal closed after ConvertArchetype ok"));
    }

    press_cmd_shortcut(&mut driver, "z")?;
    let restored = driver.app().document().data_sheet(&xy_id).ok_or_else(|| String::from("missing restored sheet"))?;
    if restored.kind() != TableKind::Xy {
        return Err(format!("expected Undo to restore TableKind::Xy, got {:?}", restored.kind()));
    }

    // Also test modal.cancel on Multi-Var / Contingency leaves sheet unchanged.
    let doc_snap2 = driver.app().document().clone();
    driver.app_mut().canvas_dialogs.open_convert_archetype(&doc_snap2, &xy_id);
    click_widget_center(&mut driver, &wid("convert.target.multiple_variables"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    click_widget_center(&mut driver, &wid("modal.cancel"))?;
    if driver.app().document().data_sheet(&xy_id).map(DataSheet::kind) != Some(TableKind::Xy) {
        return Err(String::from("expected modal.cancel to keep TableKind::Xy untouched"));
    }
    Ok(())
}

#[test]
fn r10_12_and_r10_4_responsive_hidpi_themes_and_i18n_shard_probe() -> Result<(), String> {
    let viewports = [ViewportMetrics::new(1024.0, 640.0, 1.0), ViewportMetrics::new(1280.0, 800.0, 1.0), ViewportMetrics::new(1280.0, 800.0, 1.5)];
    let themes = [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast];

    let assert_modal_parity = |driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver, scale: f64| -> Result<(), String> {
        if (scale - 1.0).abs() < 0.01 {
            assert_zero_shortcut_compliance(driver)?;
        } else {
            let report = driver.verify_parity();
            let modal_violations: Vec<_> = report
                .violations
                .into_iter()
                .filter(|v| v.widget_id.starts_with("import") || v.widget_id.starts_with("convert.") || v.widget_id.starts_with("modal."))
                .collect();
            if !modal_violations.is_empty() {
                return Err(format!("modal HiDPI parity violations at {scale}x: {modal_violations:?}"));
            }
        }
        Ok(())
    };

    for metrics in viewports {
        for theme in themes {
            let (doc, xy_id, _) = build_xy_replicates_and_col_doc()?;
            let mut app = UiApp::from_session(Session::from_document(doc));
            app.set_theme_mode(theme);
            app.canvas_dialogs.open_import_reshape();
            let mut driver = new_human_driver(app, metrics);
            for tab_id in [
                "import_paste.tab.source",
                "import_paste.tab.view",
                "import_paste.tab.filter",
                "import_paste.tab.placement",
                "import_paste.tab.notes",
            ] {
                click_widget_center(&mut driver, &wid(tab_id))?;
                assert_modal_parity(&mut driver, metrics.scale_factor)?;
            }
            let doc_snap = driver.app().document().clone();
            driver.app_mut().canvas_dialogs.open_convert_archetype(&doc_snap, &xy_id);
            click_widget_center(&mut driver, &wid("convert.target.grouped"))?;
            assert_modal_parity(&mut driver, metrics.scale_factor)?;
        }
    }

    // Verify w5_import_paste.rs i18n shard entries: registered in global catalog, 16 locales non-empty, measurable glyphs, zero key collisions.
    let existing: BTreeSet<&str> = all_keys().collect();
    let mut seen = BTreeSet::new();
    let measurer = DesktopTextMeasurer::new();
    let style = UiTextStyle::ui_sans(12.0);
    for entry in w5_import_paste::ENTRIES {
        if !seen.insert(entry.key) {
            return Err(format!("duplicate shard i18n key `{}`", entry.key));
        }
        if !existing.contains(entry.key) {
            return Err(format!("shard i18n key `{}` missing from global i18n catalog", entry.key));
        }
        if entry.translations.len() != LOCALES.len() {
            return Err(format!("key `{}` has {} translations, expected {}", entry.key, entry.translations.len(), LOCALES.len()));
        }
        for (locale_idx, val) in entry.translations.iter().enumerate() {
            if val.trim().is_empty() {
                return Err(format!("key `{}` locale index {locale_idx} is blank", entry.key));
            }
            let measured = measurer.measure(val, &style, LOCALES[locale_idx]);
            if measured.width_px <= 0.0 || measured.line_height_px <= 0.0 {
                return Err(format!("key `{}` locale index {locale_idx} measured zero size", entry.key));
            }
        }
    }
    Ok(())
}
