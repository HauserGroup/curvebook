#![forbid(unsafe_code)]
//! `R10.17` Human-path acceptance tests for Project Navigator, Family lineage tree,
//! and draggable/collapsible `FloatingNote` callout cards (`R10.2`, `R10.9`, `R10.20`).

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, FloatingNote, GraphKind, GraphSheet,
    ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoConstant, InfoSheet, InternalLinkKind, LayoutItem, LayoutSheet, LayoutSpec,
    LinkSource, LinkedDataRegion, ProjectDocument, RectPt, RegressionOverlaySpec, ResultSheet, ResultStatus, SheetCellRange, SheetId, SubcolumnMode,
    TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::Rgba8;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::i18n::{LOCALES, try_tr};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::{LayoutTree, WidgetId, ZLayer};
use curvebook_ui_egui::{UiApp, UiThemeMode};
use serde_json::json;

fn build_rich_lineage_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("Navigator & Lineage Suite").map_err(|e| e.to_string())?;

    let mut info = InfoSheet::new_preseeded("info-uuid-901", "Protocol Metadata").map_err(|e| e.to_string())?;
    info.constants
        .push(InfoConstant::new("Target_pH", curvebook_doc::ConstantValue::Number(7.4), Some(String::from("pH"))).map_err(|e| e.to_string())?);
    info.notes_markdown = String::from("Equilibrated 30 min at 25 C.");
    info.floating_notes.push(FloatingNote::new("note-info-1", RectPt::new(24.0, 28.0, 175.0, 68.0), "Verify buffer pH prior to assay."));
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).map_err(|e| e.to_string())?;
    xy.data_sets[0].title = String::from("Vehicle");
    xy.data_sets[1].title = String::from("Inhibitor");
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).map_err(|e| e.to_string())?;
        xy.data_sets[0].cells[r][0] = Cell::number(100.0 - (r as f64) * 10.0);
        xy.data_sets[1].cells[r][0] = Cell::number(95.0 - (r as f64) * 20.0);
    }
    let mut ds_main = DataSheet::new("data-uuid-101", "Binding Assay", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    ds_main.info_sheet_id = Some(SheetId::from("info-uuid-901"));
    ds_main.frozen = true;
    ds_main.color_tag = Some(Rgba8::rgb(29, 78, 216));
    ds_main.linked_regions.push(LinkedDataRegion {
        id: String::from("link-1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::from("data-uuid-102"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("Secondary Screen A1:A2"),
    });
    ds_main.floating_notes.push(FloatingNote::new(
        "note-data-1",
        RectPt::new(16.0, 22.0, 170.0, 68.0),
        "Replicate 2 outlier flagged for plate re-read.",
    ));
    doc.add_data_sheet(ds_main).map_err(|e| e.to_string())?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Treated");
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + (r as f64));
        col.data_sets[1].cells[r][0] = Cell::number(18.0 + (r as f64));
    }
    let ds_second = DataSheet::new("data-uuid-102", "Secondary Screen", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds_second).map_err(|e| e.to_string())?;

    let spec =
        AnalysisSpec::new("res-uuid-201", "data-uuid-101", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    let mut rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    rs.analysis.set_custom_title(Some(String::from("Binding Summary Stats"))).map_err(|e| e.to_string())?;
    rs.status = ResultStatus::Dirty;
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let ds_ref = doc.data_sheet(&SheetId::from("data-uuid-101")).ok_or_else(|| String::from("missing data-uuid-101"))?.clone();
    let mut gs = GraphSheet::new_uninitialized_for_data("graph-uuid-301", &ds_ref).map_err(|e| e.to_string())?;
    gs.title = String::from("Inhibition Curve");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Xy;
    if let Some(series) = gs.spec.series.first_mut() {
        series.regression_overlay = Some(RegressionOverlaySpec {
            result_sheet_id: SheetId::from("res-uuid-201"),
            sample_segments: 256,
            show_confidence_band: false,
            show_prediction_band: false,
            shaded_ribbon: false,
        });
    }
    gs.floating_notes.push(FloatingNote::new("note-graph-1", RectPt::new(18.0, 20.0, 170.0, 64.0), "Highlight IC50 inflection point."));
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;

    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-1"),
        graph_sheet_id: SheetId::from("graph-uuid-301"),
        bounds_pt: RectPt::new(20.0, 20.0, 240.0, 160.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    let ls = LayoutSheet::new("layout-uuid-401", "Publication Panel 1", lspec).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(ls).map_err(|e| e.to_string())?;

    doc.select_in_navigator(&SheetId::from("data-uuid-101")).map_err(|e| e.to_string())?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

fn verify_nav_folder_toggle_and_search(driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver) -> Result<(), String> {
    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from("nav.folder.0"))?;
    let collapsed_tree = driver.layout_tree();
    assert_eq!(collapsed_tree.node("nav.folder.0").and_then(|n| n.a11y.expanded), Some(false));
    assert!(collapsed_tree.node("nav.sheet.data-uuid-101").is_none(), "collapsed folder must hide child sheets");
    assert!(collapsed_tree.node("nav.create.data").is_none(), "collapsed folder must hide inline create row");

    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from("nav.folder.0"))?;
    let expanded_tree = driver.layout_tree();
    assert_eq!(expanded_tree.node("nav.folder.0").and_then(|n| n.a11y.expanded), Some(true));
    assert!(expanded_tree.node("nav.sheet.data-uuid-101").is_some());

    let search_id = WidgetId::from("navigator.search_input");
    let _ = human_wf_common::click_and_type(driver, &search_id, "Secondary")?;
    let filtered_tree = driver.layout_tree();
    assert!(filtered_tree.node("nav.sheet.data-uuid-102").is_some(), "matching Secondary Screen should remain visible");
    assert!(filtered_tree.node("nav.sheet.data-uuid-101").is_none(), "non-matching Binding Assay should be filtered out");
    assert!(filtered_tree.node("nav.sheet.layout-uuid-401").is_none(), "non-matching Publication Panel 1 should be filtered out");
    let _ = human_wf_common::assert_zero_shortcut_compliance(driver)?;

    let clear_id = WidgetId::from("navigator.search_clear");
    let _ = human_wf_common::click_widget_center(driver, &clear_id)?;
    let restored_tree = driver.layout_tree();
    assert!(restored_tree.node("nav.sheet.data-uuid-101").is_some(), "clearing search must restore Binding Assay");
    assert!(restored_tree.node("nav.sheet.layout-uuid-401").is_some(), "clearing search must restore Publication Panel 1");
    Ok(())
}

fn verify_nav_reorder_and_inline_create(driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver) -> Result<(), String> {
    assert_eq!(driver.app().document().data_sheets[0].id.as_str(), "data-uuid-101");
    assert_eq!(driver.app().document().data_sheets[1].id.as_str(), "data-uuid-102");
    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from("nav.reorder_down.data-uuid-101"))?;
    assert_eq!(driver.app().document().data_sheets[0].id.as_str(), "data-uuid-102");
    assert_eq!(driver.app().document().data_sheets[1].id.as_str(), "data-uuid-101");

    let c0 = human_wf_common::widget_hit_center(driver, &WidgetId::from("nav.sheet.data-uuid-102"))?;
    let c1 = human_wf_common::widget_hit_center(driver, &WidgetId::from("nav.sheet.data-uuid-101"))?;
    let drag_out = driver.drag_logical(c0[0], c0[1], c1[0], c1[1])?;
    assert_eq!(drag_out.command_id.as_deref(), Some("sheet.reorder"));
    assert_eq!(driver.app().document().data_sheets[0].id.as_str(), "data-uuid-101");
    assert_eq!(driver.app().document().data_sheets[1].id.as_str(), "data-uuid-102");

    let nav_container = WidgetId::from("navigator.tree");
    let create_data = WidgetId::from("nav.create.data");
    human_wf_common::scroll_until_widget_visible(driver, &nav_container, &create_data, 80.0, 8)?;
    let data_before = driver.app().document().data_sheets.len();
    let _ = human_wf_common::click_widget_center(driver, &create_data)?;
    assert_eq!(driver.app().document().data_sheets.len(), data_before + 1);

    let create_graph = WidgetId::from("nav.create.graphs");
    human_wf_common::scroll_until_widget_visible(driver, &nav_container, &create_graph, 80.0, 10)?;
    let graphs_before = driver.app().document().graph_sheets.len();
    let _ = human_wf_common::click_widget_center(driver, &create_graph)?;
    assert_eq!(driver.app().document().graph_sheets.len(), graphs_before + 1);

    let create_layout = WidgetId::from("nav.create.layouts");
    human_wf_common::scroll_until_widget_visible(driver, &nav_container, &create_layout, 80.0, 12)?;
    let layouts_before = driver.app().document().layout_sheets.len();
    let _ = human_wf_common::click_widget_center(driver, &create_layout)?;
    assert_eq!(driver.app().document().layout_sheets.len(), layouts_before + 1);

    let create_results = WidgetId::from("nav.create.results");
    human_wf_common::scroll_until_widget_visible(driver, &nav_container, &create_results, -80.0, 12)?;
    let _ = human_wf_common::click_widget_center(driver, &create_results)?;
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    let _ = human_wf_common::press_key_combo(driver, "Escape", KeyModifiers::default())?;
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn navigator_folders_badges_search_reorder_and_inline_create_human_path() -> Result<(), String> {
    let app = build_rich_lineage_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    let report0 = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    assert!(report0.checked_nodes > 0);

    let tree = driver.layout_tree();
    for f_idx in 0..5 {
        let fid = format!("nav.folder.{f_idx}");
        let node = tree.node(&fid).ok_or_else(|| format!("missing folder `{fid}`"))?;
        assert!(node.drawn_in_frame, "folder `{fid}` must be visibly drawn");
        assert_eq!(node.a11y.expanded, Some(true), "folder `{fid}` should start expanded");
    }
    for create_id in ["nav.create.data", "nav.create.info", "nav.create.results", "nav.create.graphs", "nav.create.layouts"] {
        let node = tree.node(create_id).ok_or_else(|| format!("missing inline creation row `{create_id}`"))?;
        assert!(node.drawn_in_frame, "inline creation row `{create_id}` must be drawn");
        assert!(node.state.enablement.enabled, "inline creation row `{create_id}` should be enabled in populated project");
    }
    let frozen_node = tree.node("nav.sheet.data-uuid-101").ok_or_else(|| String::from("missing data-uuid-101 node"))?;
    assert_eq!(frozen_node.a11y.value_text.as_deref(), Some("Frozen"), "frozen badge metadata must be exposed");

    verify_nav_folder_toggle_and_search(&mut driver)?;
    verify_nav_reorder_and_inline_create(&mut driver)?;

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 150.0)?;
    Ok(())
}

#[test]
fn family_lineage_tree_displays_human_titles_indented_and_switches_active_sheet() -> Result<(), String> {
    let app = build_rich_lineage_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    let tree = driver.layout_tree();

    let mut fam_items = Vec::new();
    for idx in 0..10 {
        let id = format!("family.item.{idx}");
        if let Some(node) = tree.node(&id) {
            fam_items.push((id, node.clone()));
        }
    }
    assert!(fam_items.len() >= 6, "expected 6 lineage sheets in Family pane, got {}", fam_items.len());

    // Verify human sheet titles only (NEVER internal IDs!) and upstream-to-downstream indentation order.
    let titles: Vec<&str> = fam_items.iter().map(|(_, n)| n.a11y.label.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Protocol Metadata", "Secondary Screen", "Binding Assay", "Binding Summary Stats", "Inhibition Curve", "Publication Panel 1"]
    );
    for (wid, node) in &fam_items {
        let lbl = &node.a11y.label;
        assert!(!lbl.contains("uuid"), "family item `{wid}` leaked internal ID `{lbl}`");
    }
    for run in &tree.text_runs {
        if run.owner_widget_id.as_str().starts_with("family.") {
            assert!(!run.text.contains("uuid"), "family text run leaked internal ID `{}`", run.text);
        }
    }

    let depths: Vec<usize> =
        fam_items.iter().filter_map(|(_, n)| n.a11y.value_text.as_deref()?.strip_prefix("depth:")?.parse::<usize>().ok()).collect();
    assert_eq!(depths.len(), 6);
    assert!(depths[0] < depths[2], "Protocol Metadata should precede indented Binding Assay: {depths:?}");
    assert!(depths[1] < depths[2], "Secondary Screen should precede indented Binding Assay: {depths:?}");
    assert!(depths[2] < depths[3], "Binding Assay should precede indented Binding Summary Stats: {depths:?}");
    assert!(depths[3] < depths[4], "Binding Summary Stats should precede indented Inhibition Curve: {depths:?}");
    assert!(depths[4] < depths[5], "Inhibition Curve should precede indented Publication Panel 1: {depths:?}");

    // Click Inhibition Curve (`family.item.4`) at physical (x, y) -> switches active sheet while preserving anchor!
    let anchor_before = driver.app().document().family_anchor_sheet_id.clone();
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("family.item.4"))?;
    assert_eq!(driver.app().document().active_sheet_id.as_ref().map(curvebook_doc::SheetId::as_str), Some("graph-uuid-301"));
    assert_eq!(driver.app().document().family_anchor_sheet_id, anchor_before);
    let after_tree = driver.layout_tree();
    assert_eq!(after_tree.node("family.item.4").and_then(|n| n.a11y.selected), Some(true));

    // Toggle Family pane collapse and re-expand via physical clicks.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("navigator.family_toggle"))?;
    assert!(!driver.app().family_open);
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("navigator.family_toggle"))?;
    assert!(driver.app().family_open);

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 150.0)?;
    Ok(())
}

#[test]
fn floating_notes_render_as_collapsible_movable_cards_and_preserve_split_guard() -> Result<(), String> {
    let app = build_rich_lineage_app()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // 1. Active DataSheet (`data-uuid-101`) renders its FloatingNote as an expanded FloatingOverlay callout card.
    let tree0 = driver.layout_tree();
    let card_id = WidgetId::from("floating_note.data-uuid-101.note-data-1.card");
    let collapse_id = WidgetId::from("floating_note.data-uuid-101.note-data-1.collapse");
    let drag_id = WidgetId::from("floating_note.data-uuid-101.note-data-1.drag_handle");
    let move_right_id = WidgetId::from("floating_note.data-uuid-101.note-data-1.move_right");
    let move_down_id = WidgetId::from("floating_note.data-uuid-101.note-data-1.move_down");

    let card_expanded = tree0.node(card_id.as_str()).ok_or_else(|| String::from("missing floating note card"))?.clone();
    assert_eq!(card_expanded.z_layer, ZLayer::FloatingOverlay);
    assert!(
        tree0.text_runs.iter().any(|r| r.owner_widget_id == card_id && r.text.contains("Replicate 2")),
        "expanded floating note must render note markdown body"
    );

    // 2. Physical click on collapse toggle shrinks card & hides body, then click again expands it.
    let _ = human_wf_common::click_widget_center(&mut driver, &collapse_id)?;
    assert!(driver.app().nav.is_floating_note_collapsed("note-data-1"));
    let tree_col = driver.layout_tree();
    let card_collapsed = tree_col.node(card_id.as_str()).ok_or_else(|| String::from("missing collapsed card"))?;
    assert!(
        card_collapsed.bounds.rect_px.height < card_expanded.bounds.rect_px.height,
        "collapsed card height ({}) must be smaller than expanded ({})",
        card_collapsed.bounds.rect_px.height,
        card_expanded.bounds.rect_px.height
    );
    assert!(
        !tree_col.text_runs.iter().any(|r| r.owner_widget_id == card_id && r.text.contains("Replicate 2")),
        "collapsed floating note must hide body lines"
    );

    let _ = human_wf_common::click_widget_center(&mut driver, &collapse_id)?;
    assert!(!driver.app().nav.is_floating_note_collapsed("note-data-1"));

    // 3. Physical click on drag handle + move controls repositions card smoothly.
    let _ = human_wf_common::click_widget_center(&mut driver, &drag_id)?;
    assert_eq!(driver.app().view_shards.nav_notes.active_note_id.as_deref(), Some("note-data-1"));
    let pos_before = driver.layout_tree().node(card_id.as_str()).ok_or_else(|| String::from("missing card before move"))?.bounds.rect_px;

    let _ = human_wf_common::click_widget_center(&mut driver, &move_right_id)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &move_down_id)?;
    let pos_after = driver.layout_tree().node(card_id.as_str()).ok_or_else(|| String::from("missing card after move"))?.bounds.rect_px;
    assert!(pos_after.x > pos_before.x, "expected note x to move right: {} -> {}", pos_before.x, pos_after.x);
    assert!(pos_after.y > pos_before.y, "expected note y to move down: {} -> {}", pos_before.y, pos_after.y);

    // 4. Switch to InfoSheet (`info-uuid-901`) via Navigator click and verify Info floating note + collapse!
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("nav.sheet.info-uuid-901"))?;
    let info_tree = driver.layout_tree();
    assert!(info_tree.node("floating_note.info-uuid-901.note-info-1.card").is_some(), "InfoSheet must display its floating note");
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("floating_note.info-uuid-901.note-info-1.collapse"))?;
    assert!(driver.app().nav.is_floating_note_collapsed("note-info-1"));
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // 5. Preserve R2-S10-09 split-pane guard with two InfoSheets.
    let mut split_app = build_rich_lineage_app()?;
    let mut info2 = InfoSheet::new_preseeded("info-uuid-902", "Secondary Info").map_err(|e| e.to_string())?;
    info2.floating_notes.push(FloatingNote::new("note-info-2", RectPt::new(10.0, 12.0, 140.0, 56.0), "Secondary note."));
    {
        let mut d = split_app.document().clone();
        d.add_info_sheet(info2).map_err(|e| e.to_string())?;
        split_app = UiApp::from_session(Session::from_document(d));
    }
    split_app.split.enabled = true;
    split_app.split.primary_sheet_id = Some(SheetId::from("info-uuid-901"));
    split_app.split.secondary_sheet_id = Some(SheetId::from("info-uuid-902"));
    let split_tree = LayoutTree::build(&split_app, human_wf_common::default_viewport());
    assert_eq!(split_tree.info_sheet_id.as_ref().map(SheetId::as_str), Some("info-uuid-901"), "R2-S10-09 primary info_sheet_id must be preserved");
    let prim_rect = split_tree.split_primary.ok_or_else(|| String::from("missing split_primary"))?;
    let notes_rect = split_tree.info_notes_rect.ok_or_else(|| String::from("missing info_notes_rect"))?;
    assert!(
        notes_rect.x >= prim_rect.x && notes_rect.right() <= prim_rect.right(),
        "R2-S10-09 info_notes_rect {notes_rect:?} escaped primary pane {prim_rect:?}"
    );
    Ok(())
}

fn assert_nav_notes_parity_clean(driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver, context: &str) {
    let nav_violations: Vec<_> = driver
        .verify_parity()
        .violations
        .into_iter()
        .filter(|v| {
            v.widget_id.starts_with("nav.")
                || v.widget_id.starts_with("navigator.")
                || v.widget_id.starts_with("family.")
                || v.widget_id.starts_with("floating_note.")
                || v.widget_id.starts_with("info.")
        })
        .collect();
    assert!(nav_violations.is_empty(), "nav_notes parity violation on {context}: {nav_violations:?}");
}

#[test]
fn navigator_and_notes_pass_parity_across_themes_rtl_and_hidpi_scales() -> Result<(), String> {
    for &locale in LOCALES {
        assert!(try_tr(locale, "nav.data_tables").is_some(), "missing nav.data_tables for {locale}");
        assert!(try_tr(locale, "nav.family").is_some(), "missing nav.family for {locale}");
    }

    for (locale, mode, scale) in [
        ("en", UiThemeMode::Light, 1.0),
        ("ar", UiThemeMode::HighContrast, 1.0),
        ("he", UiThemeMode::Dark, 1.0),
        ("de", UiThemeMode::Dark, 1.5),
        ("he", UiThemeMode::Light, 2.0),
    ] {
        let mut app = build_rich_lineage_app()?;
        app.set_locale(locale);
        app.set_theme_mode(mode);
        let mut driver = human_wf_common::new_human_driver(app, ViewportMetrics::new(1280.0, 800.0, scale));
        if (scale - 1.0).abs() < f64::EPSILON {
            let report = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
            assert!(report.is_clean(), "DataSheet parity violation for ({locale}, {mode:?}, {scale}x): {:?}", report.violations);
        } else {
            assert_nav_notes_parity_clean(&mut driver, &format!("DataSheet ({locale}, {mode:?}, {scale}x)"));
        }

        let _ = driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": "info-uuid-901" })).map_err(|e| e.to_string())?;
        if (scale - 1.0).abs() < f64::EPSILON {
            let info_report = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
            assert!(info_report.is_clean(), "InfoSheet parity violation for ({locale}, {mode:?}, {scale}x): {:?}", info_report.violations);
        } else {
            assert_nav_notes_parity_clean(&mut driver, &format!("InfoSheet ({locale}, {mode:?}, {scale}x)"));
        }
    }
    Ok(())
}
