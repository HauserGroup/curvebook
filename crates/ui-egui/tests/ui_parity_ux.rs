//! Wave 3 UX parity integration tests covering Navigator drag-reorder (`sheet.reorder`), Portfolio
//! Gallery multi-select & filtering, `F6`/`Shift+F6` pane focus cycling, interactive `InfoSheet`
//! command builders & collapsible `FloatingNote` state, locale decimal-comma parsing/formatting, and
//! `Nested`/`Xy` subcolumn header & biological-unit badge helpers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, ConstantValue, DataSheet, DescriptiveSpec, GraphSheet, InfoSheet, LayoutSheet, LayoutSpec, NestedTable,
    ProjectDocument, ResultSheet, SheetFolderKind, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use lumencraft_engine::Session;
use lumencraft_ui_egui::grid_view::{biological_unit_label, format_cell_display, parse_localized_number_input, subcolumn_header_labels};
use lumencraft_ui_egui::i18n::LocalePreset;
use lumencraft_ui_egui::workspace_nav::{ActiveWorkspacePane, WorkspaceNavState, build_set_info_constant_command, build_set_info_notes_command};
use serde_json::json;

fn make_xy_sheet(id: &str, title: &str) -> DataSheet {
    let xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 4).unwrap();
    DataSheet::new(id, title, TableArchetype::Xy(xy)).unwrap()
}

#[test]
fn navigator_drag_reorder_executes_and_undoes_end_to_end() {
    let mut doc = ProjectDocument::new("Navigator Reorder").unwrap();
    doc.add_data_sheet(make_xy_sheet("d1", "First")).unwrap();
    doc.add_data_sheet(make_xy_sheet("d2", "Second")).unwrap();
    doc.add_data_sheet(make_xy_sheet("d3", "Third")).unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Info A").unwrap()).unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-2", "Info B").unwrap()).unwrap();

    let mut session = Session::from_document(doc);
    let mut nav = WorkspaceNavState::default();
    nav.sync_from_document(session.document());

    // Drag d1 from index 0 to index 2 inside Data folder.
    nav.begin_sheet_drag(SheetFolderKind::Data, "d1", 0);
    nav.update_sheet_hover(2);
    let (cmd, params) = nav.commit_sheet_drop_command().expect("drag drop emits sheet.reorder");
    assert_eq!(cmd, "sheet.reorder");
    assert_eq!(params, json!({ "folder": "Data", "from_index": 0, "to_index": 2 }));

    let outcome = session.execute(cmd, params).unwrap();
    assert!(outcome.mutated);

    let ordered_ids: Vec<&str> = session.document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ordered_ids, vec!["d2", "d3", "d1"]);

    // Undo and redo restore and re-apply the reorder cleanly.
    session.execute("edit.undo", json!({})).unwrap();
    let undone_ids: Vec<&str> = session.document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(undone_ids, vec!["d1", "d2", "d3"]);

    session.execute("edit.redo", json!({})).unwrap();
    let redone_ids: Vec<&str> = session.document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(redone_ids, vec!["d2", "d3", "d1"]);

    // Dropping on the originating index or canceling produces no command.
    nav.begin_sheet_drag(SheetFolderKind::Data, "d2", 0);
    nav.update_sheet_hover(0);
    assert!(nav.commit_sheet_drop_command().is_none());

    nav.begin_sheet_drag(SheetFolderKind::Info, "info-2", 1);
    nav.update_sheet_hover(0);
    nav.cancel_sheet_drag();
    assert!(nav.commit_sheet_drop_command().is_none());

    // Info folder reorder also succeeds end-to-end.
    nav.begin_sheet_drag(SheetFolderKind::Info, "info-2", 1);
    nav.update_sheet_hover(0);
    let (info_cmd, info_params) = nav.commit_sheet_drop_command().unwrap();
    session.execute(info_cmd, info_params).unwrap();
    let info_ids: Vec<&str> = session.document().info_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(info_ids, vec!["info-2", "info-1"]);
}

#[test]
fn portfolio_gallery_populates_filters_and_multi_selects_for_batch_export() {
    let mut doc = ProjectDocument::new("Portfolio Gallery").unwrap();
    let d1 = make_xy_sheet("d1", "Dose Response");
    let d2 = make_xy_sheet("d2", "Control");
    let g1 = GraphSheet::new_uninitialized_for_data("g1", &d1).unwrap();
    let g2 = GraphSheet::new_uninitialized_for_data("g2", &d2).unwrap();
    let info = InfoSheet::new_preseeded("info-1", "Protocol").unwrap();
    let analysis = AnalysisSpec::new("res-1", "d1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let res = ResultSheet::new(analysis).unwrap();
    let lay = LayoutSheet::new("lay-1", "Figure 1", LayoutSpec::default()).unwrap();

    doc.add_data_sheet(d1).unwrap();
    doc.add_data_sheet(d2).unwrap();
    doc.add_info_sheet(info).unwrap();
    doc.add_result_sheet(res).unwrap();
    doc.add_graph_sheet(g1).unwrap();
    doc.add_graph_sheet(g2).unwrap();
    doc.add_layout_sheet(lay).unwrap();
    doc.active_sheet_id = Some(SheetId::from("g1"));

    let mut nav = WorkspaceNavState::default();
    assert!(!nav.gallery.open);
    assert!(nav.toggle_gallery());
    assert!(nav.gallery.open);

    nav.sync_gallery(&doc);
    assert_eq!(nav.gallery.cards.len(), 7);
    assert!(nav.gallery.cards.iter().any(|c| c.sheet_id.as_str() == "g1" && c.active && !c.selected));

    // Filter to Graphs for batch figure export selection.
    nav.set_gallery_filter(Some(SheetFolderKind::Graphs), &doc);
    assert_eq!(nav.gallery.cards.len(), 2);
    assert!(nav.gallery.cards.iter().all(|c| c.folder == SheetFolderKind::Graphs));

    let g1_id = SheetId::from("g1");
    let g2_id = SheetId::from("g2");
    let lay_id = SheetId::from("lay-1");

    nav.select_gallery_card(&g1_id, false);
    assert_eq!(nav.gallery.selected_ids.len(), 1);
    assert!(nav.gallery.cards[0].selected);
    assert!(!nav.gallery.cards[1].selected);

    nav.select_gallery_card(&g2_id, true);
    assert_eq!(nav.selected_gallery_ids(), vec![g1_id.clone(), g2_id.clone()]);

    // Clear filter and add a Layout sheet into the multi-select batch export set.
    nav.set_gallery_filter(None, &doc);
    nav.select_gallery_card(&lay_id, true);
    assert_eq!(nav.gallery.selected_ids.len(), 3);

    // Toggle off g1 cleanly with additive click.
    nav.select_gallery_card(&g1_id, true);
    assert!(!nav.gallery.selected_ids.contains(&g1_id));
    assert_eq!(nav.gallery.selected_ids.len(), 2);

    // Removing a sheet from document prunes stale selections on sync.
    doc.graph_sheets.retain(|g| g.id != g2_id);
    nav.sync_gallery(&doc);
    assert!(!nav.gallery.selected_ids.contains(&g2_id));
    assert_eq!(nav.selected_gallery_ids(), vec![lay_id]);

    nav.clear_gallery_selection();
    assert!(nav.gallery.selected_ids.is_empty());
    assert!(!nav.toggle_gallery());
}

#[test]
fn f6_and_shift_f6_cycle_workspace_panes_in_order() {
    let mut nav = WorkspaceNavState::default();
    assert_eq!(nav.active_pane, ActiveWorkspacePane::default());
    assert_eq!(nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);

    assert_eq!(nav.cycle_focus_pane(false), ActiveWorkspacePane::Inspector);
    assert_eq!(nav.cycle_focus_pane(false), ActiveWorkspacePane::FloatingNotes);
    assert_eq!(nav.cycle_focus_pane(false), ActiveWorkspacePane::Navigator);
    assert_eq!(nav.cycle_focus_pane(false), ActiveWorkspacePane::FamilyStrip);
    assert_eq!(nav.cycle_focus_pane(false), ActiveWorkspacePane::MainCanvasOrGrid);

    assert_eq!(nav.cycle_focus_pane(true), ActiveWorkspacePane::FamilyStrip);
    assert_eq!(nav.cycle_focus_pane(true), ActiveWorkspacePane::Navigator);
    assert_eq!(nav.cycle_focus_pane(true), ActiveWorkspacePane::FloatingNotes);
    assert_eq!(nav.cycle_focus_pane(true), ActiveWorkspacePane::Inspector);
    assert_eq!(nav.cycle_focus_pane(true), ActiveWorkspacePane::MainCanvasOrGrid);
}

#[test]
fn info_sheet_command_builders_and_floating_note_collapse_state() {
    let info_id = SheetId::from("info-1");
    let mut doc = ProjectDocument::new("Info & Notes").unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded(info_id.clone(), "Experiment Info").unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    let c1 = build_set_info_constant_command(&info_id, "TempC", &ConstantValue::Number(37.5), Some("C"));
    session.execute(&c1.command_id, c1.params).unwrap();

    let c2 = build_set_info_constant_command(&info_id, "AssayDate", &ConstantValue::Date(String::from("2026-11-13")), None);
    session.execute(&c2.command_id, c2.params).unwrap();

    let c3 = build_set_info_constant_command(&info_id, "Lot", &ConstantValue::Text(String::from("LC-402")), None);
    session.execute(&c3.command_id, c3.params).unwrap();

    let n1 = build_set_info_notes_command(&info_id, "# Assay Log", false);
    session.execute(&n1.command_id, n1.params).unwrap();
    let n2 = build_set_info_notes_command(&info_id, "- Equilibrated 10 min", true);
    session.execute(&n2.command_id, n2.params).unwrap();

    let info = session.document().info_sheet(&info_id).unwrap();
    assert_eq!(info.notes_markdown, "# Assay Log\n- Equilibrated 10 min");
    assert_eq!(
        info.constants.iter().find(|c| c.name == "TempC").map(|c| (&c.value, c.units.as_deref())),
        Some((&ConstantValue::Number(37.5), Some("C")))
    );
    assert_eq!(info.constants.iter().find(|c| c.name == "AssayDate").map(|c| &c.value), Some(&ConstantValue::Date(String::from("2026-11-13"))));
    assert_eq!(info.constants.iter().find(|c| c.name == "Lot").map(|c| &c.value), Some(&ConstantValue::Text(String::from("LC-402"))));

    let mut nav = WorkspaceNavState::default();
    assert!(!nav.is_floating_note_collapsed("note-1"));
    assert!(nav.toggle_floating_note("note-1"));
    assert!(nav.is_floating_note_collapsed("note-1"));
    assert!(!nav.is_floating_note_collapsed("note-2"));
    assert!(!nav.toggle_floating_note("note-1"));
    assert!(!nav.is_floating_note_collapsed("note-1"));
}

#[test]
fn locale_decimal_comma_parsing_and_formatting_preserve_formulas() {
    for loc in ["de", "fr", "es", "pl", "ru"] {
        let preset = LocalePreset::for_locale(loc);
        assert_eq!(preset.decimal_separator, ',', "expected comma decimal for `{loc}`");

        assert_eq!(format_cell_display(&Cell::number(12.345), &preset), "12,345");
        assert_eq!(format_cell_display(&Cell::number(-0.5), &preset), "-0,5");
        assert_eq!(format_cell_display(&Cell::excluded_number(12.5), &preset), "12,5*");
        assert_eq!(format_cell_display(&Cell::text("=A1*2,5"), &preset), "=A1*2,5");

        let pub_preset = LocalePreset::publication(loc);
        assert_eq!(format_cell_display(&Cell::number(-3.25), &pub_preset), "\u{2212}3,25");
        assert_eq!(parse_localized_number_input("\u{2212}3,25", &pub_preset), Some(-3.25));

        assert_eq!(parse_localized_number_input("12,345", &preset), Some(12.345));
        assert_eq!(parse_localized_number_input("-0,5", &preset), Some(-0.5));
        assert_eq!(parse_localized_number_input("1.234,5", &preset), Some(1234.5));
        assert_eq!(parse_localized_number_input("1\u{00a0}234,5", &preset), Some(1234.5));
        assert_eq!(parse_localized_number_input("1,5e-3", &preset), Some(0.0015));

        // Formulas starting with `=` must never be coerced or corrupted as numbers.
        assert_eq!(parse_localized_number_input("=SUM(A1;B1)", &preset), None);
        assert_eq!(parse_localized_number_input("=1,5+2,5", &preset), None);
        assert_eq!(parse_localized_number_input(" = 3,5 ", &preset), None);

        assert_eq!(parse_localized_number_input("1,2,3", &preset), None);
        assert_eq!(parse_localized_number_input("NaN", &preset), None);
        assert_eq!(parse_localized_number_input("inf", &preset), None);
    }

    for loc in ["en", "ja", "ar", "he"] {
        let preset = LocalePreset::for_locale(loc);
        assert_eq!(preset.decimal_separator, '.', "expected dot decimal for `{loc}`");
        assert_eq!(format_cell_display(&Cell::number(12.5), &preset), "12.5");
        assert_eq!(parse_localized_number_input("12.5", &preset), Some(12.5));
        assert_eq!(parse_localized_number_input("1,234.5", &preset), Some(1234.5));
        assert_eq!(parse_localized_number_input("12,5", &preset), None);
        assert_eq!(parse_localized_number_input("=A1*1.5", &preset), None);
    }
}

#[test]
fn subcolumn_headers_and_biological_unit_badges_cover_nested_and_xy() {
    let nested = NestedTable::new(2, 3, 4).unwrap();
    let nested_arch = TableArchetype::Nested(nested);
    assert_eq!(subcolumn_header_labels(&nested_arch, 0), vec!["A:1", "A:2", "A:3"]);
    assert_eq!(subcolumn_header_labels(&nested_arch, 1), vec!["B:1", "B:2", "B:3"]);
    assert!(subcolumn_header_labels(&nested_arch, 2).is_empty());
    assert_eq!(biological_unit_label(&nested_arch, 0), Some(String::from("3 bio units")));
    assert_eq!(biological_unit_label(&nested_arch, 2), None);

    let nested_single = TableArchetype::Nested(NestedTable::new(1, 1, 2).unwrap());
    assert_eq!(biological_unit_label(&nested_single, 0), Some(String::from("1 bio unit")));

    let xy_rep = TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(3), 2, 4).unwrap());
    assert_eq!(subcolumn_header_labels(&xy_rep, 0), vec!["Y1", "Y2", "Y3"]);
    assert_eq!(biological_unit_label(&xy_rep, 0), Some(String::from("3 replicates")));
    assert_eq!(biological_unit_label(&xy_rep, 5), None);

    let xy_summary = TableArchetype::Xy(XyTable::new(SubcolumnMode::MeanSdN, 1, 4).unwrap());
    assert_eq!(subcolumn_header_labels(&xy_summary, 0), vec!["Mean", "SD", "N"]);
    assert_eq!(biological_unit_label(&xy_summary, 0), Some(String::from("Mean / SD / N")));

    let xy_single = TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 4).unwrap());
    assert_eq!(subcolumn_header_labels(&xy_single, 0), vec!["Y"]);
    assert_eq!(biological_unit_label(&xy_single, 0), None);
}
