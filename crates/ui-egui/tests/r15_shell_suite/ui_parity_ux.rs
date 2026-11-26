//! Wave 3 UX parity integration tests covering Navigator drag-reorder, Portfolio Gallery,
//! `F6`/`Shift+F6` pane focus cycling, `InfoSheet` command builders & `FloatingNote` collapse,
//! locale decimal-comma parsing/formatting, and subcolumn header/bio-unit helpers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ConstantValue, DataSheet, DescriptiveSpec, GraphSheet, InfoSheet, LayoutSheet, LayoutSpec, NestedTable,
    ProjectDocument, ResultSheet, SheetFolderKind, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::grid_view::{biological_unit_label, format_cell_display, parse_localized_number_input, subcolumn_header_labels};
use curvebook_ui_egui::i18n::LocalePreset;
use curvebook_ui_egui::workspace_nav::{ActiveWorkspacePane, WorkspaceNavState, build_set_info_constant_command, build_set_info_notes_command};
use serde_json::json;

fn make_xy_sheet(id: &str, title: &str) -> DataSheet {
    DataSheet::new(id, title, TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 1, 4).unwrap())).unwrap()
}

#[test]
fn navigator_drag_reorder_executes_and_undoes_end_to_end() {
    let mut doc = ProjectDocument::new("Navigator Reorder").unwrap();
    for (id, t) in [("d1", "First"), ("d2", "Second"), ("d3", "Third")] {
        doc.add_data_sheet(make_xy_sheet(id, t)).unwrap();
    }
    for (id, t) in [("info-1", "Info A"), ("info-2", "Info B")] {
        doc.add_info_sheet(InfoSheet::new_preseeded(id, t).unwrap()).unwrap();
    }

    let mut session = Session::from_document(doc);
    let mut nav = WorkspaceNavState::default();
    nav.sync_from_document(session.document());

    nav.begin_sheet_drag(SheetFolderKind::Data, "d1", 0);
    nav.update_sheet_hover(2);
    let (cmd, params) = nav.commit_sheet_drop_command().unwrap();
    assert_eq!((cmd, params.clone()), ("sheet.reorder", json!({ "folder": "Data", "from_index": 0, "to_index": 2 })));
    assert!(session.execute(cmd, params).unwrap().mutated);

    fn data_ids(s: &Session) -> Vec<&str> {
        s.document().data_sheets.iter().map(|d| d.id.as_str()).collect()
    }
    assert_eq!(data_ids(&session), vec!["d2", "d3", "d1"]);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(data_ids(&session), vec!["d1", "d2", "d3"]);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(data_ids(&session), vec!["d2", "d3", "d1"]);

    nav.begin_sheet_drag(SheetFolderKind::Data, "d2", 0);
    nav.update_sheet_hover(0);
    assert!(nav.commit_sheet_drop_command().is_none());

    nav.begin_sheet_drag(SheetFolderKind::Info, "info-2", 1);
    nav.update_sheet_hover(0);
    nav.cancel_sheet_drag();
    assert!(nav.commit_sheet_drop_command().is_none());

    nav.begin_sheet_drag(SheetFolderKind::Info, "info-2", 1);
    nav.update_sheet_hover(0);
    let (icmd, iparams) = nav.commit_sheet_drop_command().unwrap();
    session.execute(icmd, iparams).unwrap();
    assert_eq!(session.document().info_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["info-2", "info-1"]);
}

#[test]
fn portfolio_gallery_populates_filters_and_multi_selects_for_batch_export() {
    let mut doc = ProjectDocument::new("Portfolio Gallery").unwrap();
    let (d1, d2) = (make_xy_sheet("d1", "Dose Response"), make_xy_sheet("d2", "Control"));
    let (g1, g2) = (GraphSheet::new_uninitialized_for_data("g1", &d1).unwrap(), GraphSheet::new_uninitialized_for_data("g2", &d2).unwrap());
    doc.add_data_sheet(d1).unwrap();
    doc.add_data_sheet(d2).unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Protocol").unwrap()).unwrap();
    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("res-1", "d1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap()).unwrap(),
    )
    .unwrap();
    doc.add_graph_sheet(g1).unwrap();
    doc.add_graph_sheet(g2).unwrap();
    doc.add_layout_sheet(LayoutSheet::new("lay-1", "Figure 1", LayoutSpec::default()).unwrap()).unwrap();
    doc.active_sheet_id = Some(SheetId::from("g1"));

    let mut nav = WorkspaceNavState::default();
    assert!(!nav.gallery.open && nav.toggle_gallery() && nav.gallery.open);
    nav.sync_gallery(&doc);
    assert_eq!(nav.gallery.cards.len(), 7);
    assert!(nav.gallery.cards.iter().any(|c| c.sheet_id.as_str() == "g1" && c.active && !c.selected));

    nav.set_gallery_filter(Some(SheetFolderKind::Graphs), &doc);
    assert!(nav.gallery.cards.len() == 2 && nav.gallery.cards.iter().all(|c| c.folder == SheetFolderKind::Graphs));

    let (g1_id, g2_id, lay_id) = (SheetId::from("g1"), SheetId::from("g2"), SheetId::from("lay-1"));
    nav.select_gallery_card(&g1_id, false);
    assert!(nav.gallery.selected_ids.len() == 1 && nav.gallery.cards[0].selected && !nav.gallery.cards[1].selected);
    nav.select_gallery_card(&g2_id, true);
    assert_eq!(nav.selected_gallery_ids(), vec![g1_id.clone(), g2_id.clone()]);

    nav.set_gallery_filter(None, &doc);
    nav.select_gallery_card(&lay_id, true);
    assert_eq!(nav.gallery.selected_ids.len(), 3);
    nav.select_gallery_card(&g1_id, true);
    assert!(!nav.gallery.selected_ids.contains(&g1_id) && nav.gallery.selected_ids.len() == 2);

    doc.graph_sheets.retain(|g| g.id != g2_id);
    nav.sync_gallery(&doc);
    assert_eq!(nav.selected_gallery_ids(), vec![lay_id]);
    nav.clear_gallery_selection();
    assert!(nav.gallery.selected_ids.is_empty() && !nav.toggle_gallery());
}

#[test]
fn f6_and_shift_f6_cycle_workspace_panes_in_order() {
    let mut nav = WorkspaceNavState::default();
    assert_eq!(nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
    for (rev, expected) in [
        (false, ActiveWorkspacePane::Inspector),
        (false, ActiveWorkspacePane::FloatingNotes),
        (false, ActiveWorkspacePane::Navigator),
        (false, ActiveWorkspacePane::FamilyStrip),
        (false, ActiveWorkspacePane::MainCanvasOrGrid),
        (true, ActiveWorkspacePane::FamilyStrip),
        (true, ActiveWorkspacePane::Navigator),
        (true, ActiveWorkspacePane::FloatingNotes),
        (true, ActiveWorkspacePane::Inspector),
        (true, ActiveWorkspacePane::MainCanvasOrGrid),
    ] {
        assert_eq!(nav.cycle_focus_pane(rev), expected);
    }
}

#[test]
fn info_sheet_command_builders_and_floating_note_collapse_state() {
    let info_id = SheetId::from("info-1");
    let mut doc = ProjectDocument::new("Info & Notes").unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded(info_id.clone(), "Experiment Info").unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    for (name, val, units) in [
        ("TempC", ConstantValue::Number(37.5), Some("C")),
        ("AssayDate", ConstantValue::Date("2026-11-13".into()), None),
        ("Lot", ConstantValue::Text("LC-402".into()), None),
    ] {
        let cmd = build_set_info_constant_command(&info_id, name, &val, units);
        session.execute(&cmd.command_id, cmd.params).unwrap();
    }
    for (text, append) in [("# Assay Log", false), ("- Equilibrated 10 min", true)] {
        let cmd = build_set_info_notes_command(&info_id, text, append);
        session.execute(&cmd.command_id, cmd.params).unwrap();
    }

    let info = session.document().info_sheet(&info_id).unwrap();
    assert_eq!(info.notes_markdown, "# Assay Log\n- Equilibrated 10 min");
    assert_eq!(
        info.constants.iter().find(|c| c.name == "TempC").map(|c| (&c.value, c.units.as_deref())),
        Some((&ConstantValue::Number(37.5), Some("C")))
    );
    assert_eq!(info.constants.iter().find(|c| c.name == "AssayDate").map(|c| &c.value), Some(&ConstantValue::Date("2026-11-13".into())));
    assert_eq!(info.constants.iter().find(|c| c.name == "Lot").map(|c| &c.value), Some(&ConstantValue::Text("LC-402".into())));

    let mut nav = WorkspaceNavState::default();
    assert!(
        !nav.is_floating_note_collapsed("note-1")
            && nav.toggle_floating_note("note-1")
            && nav.is_floating_note_collapsed("note-1")
            && !nav.is_floating_note_collapsed("note-2")
            && !nav.toggle_floating_note("note-1")
            && !nav.is_floating_note_collapsed("note-1")
    );
}

#[test]
fn locale_decimal_comma_parsing_and_formatting_preserve_formulas() {
    for loc in ["de", "fr", "es", "pl", "ru"] {
        let preset = LocalePreset::for_locale(loc);
        assert_eq!(preset.decimal_separator, ',', "{loc}");
        for (cell, expected) in [
            (Cell::number(12.345), "12,345"),
            (Cell::number(-0.5), "-0,5"),
            (Cell::excluded_number(12.5), "12,5*"),
            (Cell::text("=A1*2,5"), "=A1*2,5"),
        ] {
            assert_eq!(format_cell_display(&cell, &preset), expected);
        }
        let pub_p = LocalePreset::publication(loc);
        assert_eq!(
            (format_cell_display(&Cell::number(-3.25), &pub_p).as_str(), parse_localized_number_input("\u{2212}3,25", &pub_p)),
            ("\u{2212}3,25", Some(-3.25))
        );
        for (raw, expected) in [
            ("12,345", Some(12.345)),
            ("-0,5", Some(-0.5)),
            ("1.234,5", Some(1234.5)),
            ("1\u{00a0}234,5", Some(1234.5)),
            ("1,5e-3", Some(0.0015)),
            ("=SUM(A1;B1)", None),
            ("=1,5+2,5", None),
            (" = 3,5 ", None),
            ("1,2,3", None),
            ("NaN", None),
            ("inf", None),
        ] {
            assert_eq!(parse_localized_number_input(raw, &preset), expected, "{loc}:{raw}");
        }
    }
    for loc in ["en", "ja", "ar", "he"] {
        let preset = LocalePreset::for_locale(loc);
        assert_eq!((preset.decimal_separator, format_cell_display(&Cell::number(12.5), &preset).as_str()), ('.', "12.5"));
        for (raw, expected) in [("12.5", Some(12.5)), ("1,234.5", Some(1234.5)), ("12,5", None), ("=A1*1.5", None)] {
            assert_eq!(parse_localized_number_input(raw, &preset), expected, "{loc}:{raw}");
        }
    }
}

#[test]
fn subcolumn_headers_and_biological_unit_badges_cover_nested_and_xy() {
    let cases: Vec<(TableArchetype, usize, Vec<&str>, Option<&str>)> = vec![
        (TableArchetype::Nested(NestedTable::new(2, 3, 4).unwrap()), 0, vec!["A:1", "A:2", "A:3"], Some("3 bio units")),
        (TableArchetype::Nested(NestedTable::new(2, 3, 4).unwrap()), 1, vec!["B:1", "B:2", "B:3"], Some("3 bio units")),
        (TableArchetype::Nested(NestedTable::new(2, 3, 4).unwrap()), 2, vec![], None),
        (TableArchetype::Nested(NestedTable::new(1, 1, 2).unwrap()), 0, vec!["A:1"], Some("1 bio unit")),
        (TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(3), 2, 4).unwrap()), 0, vec!["Y1", "Y2", "Y3"], Some("3 replicates")),
        (TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(3), 2, 4).unwrap()), 5, vec![], None),
        (TableArchetype::Xy(XyTable::new(SubcolumnMode::MeanSdN, 1, 4).unwrap()), 0, vec!["Mean", "SD", "N"], Some("Mean / SD / N")),
        (TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 4).unwrap()), 0, vec!["Y"], None),
    ];
    for (arch, col, hdrs, badge) in cases {
        assert_eq!(subcolumn_header_labels(&arch, col), hdrs);
        assert_eq!(biological_unit_label(&arch, col).as_deref(), badge);
    }
}
