#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave B (`R15.6`) human-path GUI regression suite for Data Grid (`B4`) and Manage Variables (`B9`):
//! `R15-GUI-11`, `R15-GUI-12`, `R15-GUI-06`, and `R15-GUI-21`.

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, MultipleVariablesTable, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, VariableSource,
    XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::UiIntent;
use curvebook_ui_egui::egui_shell::status_footer::{QuickStatsCache, compute_cached_quick_stats};
use curvebook_ui_egui::interaction::KeyModifiers;
use egui::{Key, Modifiers};
use serde_json::json;

use crate::common::{commit_shell_intents, make_doc_driver, widget_center, widget_selected};

fn build_xy_doc(x_subcols: SubcolumnMode) -> ProjectDocument {
    let mut doc = ProjectDocument::new("R15.6 XY Grid").unwrap();
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: x_subcols }, SubcolumnMode::Single, 2, 4).unwrap();
    xy.x_title = String::from("Conc");
    xy.data_sets[0].title = String::from("Group A");
    xy.data_sets[1].title = String::from("Group B");
    let x_w = xy.x_cells[0].len();
    for r in 0..4 {
        for xs in 0..x_w {
            xy.x_cells[r][xs] = Cell::number((r as f64 + 1.0) * 10.0 + xs as f64);
        }
        xy.data_sets[0].cells[r][0] = Cell::number(100.0 + r as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(200.0 + r as f64);
    }
    doc.add_data_sheet(DataSheet::new("ds-xy", "XY Sheet", TableArchetype::Xy(xy)).unwrap()).unwrap();
    doc
}

#[test]
fn r15_gui_11_multiline_x_inline_entry_populates_x_and_leaves_y_untouched() -> Result<(), String> {
    let mut driver = make_doc_driver(build_xy_doc(SubcolumnMode::Single), 1280.0, 800.0);
    let x0_pos = widget_center(&driver, "grid.xcell.0.0")?;
    driver.click_at(x0_pos)?;
    driver.click_at(x0_pos)?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text("1\n2\n3")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    assert!(driver.state().last_intent_error.is_none());
    let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else { panic!("expected Xy") };
    assert_eq!((0..4).map(|r| xy.x_cells[r][0].active_number()).collect::<Vec<_>>(), vec![Some(1.0), Some(2.0), Some(3.0), Some(40.0)]);
    assert_eq!(
        (0..4).map(|r| xy.data_sets[0].cells[r][0].active_number()).collect::<Vec<_>>(),
        vec![Some(100.0), Some(101.0), Some(102.0), Some(103.0)]
    );

    let mut driver2 = make_doc_driver(build_xy_doc(SubcolumnMode::RawReplicates(2)), 1280.0, 800.0);
    assert_eq!(driver2.state_mut().app.set_locale("de-DE"), "de");
    driver2.run_settled();
    let x11_pos = widget_center(&driver2, "grid.xcell.1.1")?;
    driver2.click_at(x11_pos)?;
    driver2.click_at(x11_pos)?;
    driver2.press_egui_key(Key::A, Modifiers::COMMAND);
    driver2.type_text("1,5\n2,5")?;
    assert!(driver2.state().last_intent_error.is_none());
    let TableArchetype::Xy(xy2) = &driver2.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else {
        panic!("expected Xy")
    };
    assert_eq!(
        (xy2.x_cells[0][1].active_number(), xy2.x_cells[1][1].active_number(), xy2.x_cells[2][1].active_number()),
        (Some(11.0), Some(1.5), Some(2.5))
    );
    assert_eq!((xy2.x_cells[1][0].active_number(), xy2.data_sets[0].cells[1][0].active_number()), (Some(20.0), Some(101.0)));
    Ok(())
}

#[test]
fn r15_gui_12_x_selection_clears_y_and_routes_clipboard_x_first() -> Result<(), String> {
    let mut driver = make_doc_driver(build_xy_doc(SubcolumnMode::Single), 1280.0, 800.0);

    driver.click_widget_id("grid.cell.0.0.0")?;
    assert_eq!((widget_selected(&driver, "grid.cell.0.0.0"), widget_selected(&driver, "grid.xcell.1.0")), (Some(true), Some(false)));

    driver.click_widget_id("grid.xcell.1.0")?;
    assert_eq!((widget_selected(&driver, "grid.xcell.1.0"), widget_selected(&driver, "grid.cell.0.0.0")), (Some(true), Some(false)));
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    driver.click_widget_id("grid.xcell.3.0")?;
    assert_eq!((widget_selected(&driver, "grid.xcell.3.0"), widget_selected(&driver, "grid.cell.0.0.0")), (Some(true), Some(false)));
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    {
        let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else {
            panic!("expected Xy")
        };
        assert_eq!(
            (xy.x_cells[3][0].active_number(), xy.data_sets[0].cells[0][0].active_number(), xy.data_sets[0].cells[3][0].active_number()),
            (Some(20.0), Some(100.0), Some(103.0))
        );
    }

    driver.click_widget_id("grid.xcell.2.0")?;
    driver.press_egui_key(Key::X, Modifiers::COMMAND);
    {
        let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else {
            panic!("expected Xy")
        };
        assert!(
            xy.x_cells[2][0].is_empty()
                && xy.data_sets[0].cells[0][0].active_number() == Some(100.0)
                && xy.data_sets[0].cells[2][0].active_number() == Some(102.0)
        );
    }

    driver.click_widget_id("grid.xcell.0.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    {
        let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else {
            panic!("expected Xy")
        };
        assert_eq!((xy.x_cells[0][0].active_number(), xy.data_sets[0].cells[0][0].active_number()), (Some(30.0), Some(100.0)));
    }

    driver.click_widget_id("grid.cell.0.0.0")?;
    assert_eq!((widget_selected(&driver, "grid.cell.0.0.0"), widget_selected(&driver, "grid.xcell.0.0")), (Some(true), Some(false)));
    assert!(driver.state().last_intent_error.is_none());
    Ok(())
}

#[test]
fn r15_gui_06_quick_stats_empty_column_vec_at_r0_zero_no_panic() {
    let mut doc = ProjectDocument::new("R15.6 Empty Col").unwrap();
    let mut ds =
        DataSheet::new("ds-empty", "Empty", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap())).unwrap();
    if let TableArchetype::Column(ref mut c) = ds.archetype {
        c.data_sets[0].cells.clear();
    }
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(SheetId::from("ds-empty"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    let mut cache = QuickStatsCache::default();
    let summary = compute_cached_quick_stats(&app, &mut cache);
    assert_eq!((summary.n, summary.mean), (0, None));
}

#[test]
fn r15_gui_21_manage_vars_unit_edit_rewrites_formula_reevaluates_and_undoes() -> Result<(), String> {
    let mut doc = ProjectDocument::new("R15.6 MV Rename").unwrap();
    let mut mv = MultipleVariablesTable::new(3, 3).unwrap();
    for (c, title) in ["Dose", "Weight", "Ratio"].into_iter().enumerate() {
        mv.variables[c].title = title.to_owned();
    }
    for (r, (d, w)) in [(10.0, 2.0), (20.0, 5.0), (45.0, 9.0)].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(d);
        mv.variables[1].cells[r] = Cell::number(w);
    }
    doc.add_data_sheet(DataSheet::new("ds-mv", "PK Table", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    let mut driver = make_doc_driver(doc, 1280.0, 800.0);
    driver
        .state_mut()
        .app
        .execute_command("table.setCalculatedVariable", json!({ "sheet_id": "ds-mv", "col": 2, "formula": "[Dose] / [Weight]" }))
        .map_err(|e| e.to_string())?;
    driver.run_settled();

    let ratio_vals = |app: &UiApp| -> Vec<Option<f64>> {
        let TableArchetype::MultipleVariables(mv) = &app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
            panic!("expected MV")
        };
        (0..3).map(|r| mv.variables[2].cells[r].active_number()).collect()
    };
    assert_eq!(ratio_vals(&driver.state().app), vec![Some(5.0), Some(4.0), Some(5.0)]);

    driver.state_mut().intents.push(UiIntent::OpenManageVariables { sheet_id: SheetId::from("ds-mv"), column: Some(0) });
    commit_shell_intents(&mut driver)?;
    driver.click_widget_id("mv_formula.vars.unit_input")?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text("mg/kg")?;
    let undo_before = driver.state().app.session().undo_depth();
    driver.click_widget_id("mv_formula.vars.meta.apply")?;
    assert!(driver.state().last_intent_error.is_none() && driver.state().app.session().undo_depth() == undo_before + 1);
    {
        let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
            panic!("expected MV")
        };
        assert_eq!(mv.variables[0].title, "Dose (mg/kg)");
        assert_eq!(mv.variables[2].source, VariableSource::Calculated { formula: String::from("[Dose (mg/kg)] / [Weight]") });
        assert_eq!(ratio_vals(&driver.state().app), vec![Some(5.0), Some(4.0), Some(5.0)]);
    }

    driver
        .state_mut()
        .app
        .execute_command("table.setCell", json!({ "sheet_id": "ds-mv", "row": 0, "col": 0, "value": 16.0 }))
        .map_err(|e| e.to_string())?;
    assert_eq!(ratio_vals(&driver.state().app)[0], Some(8.0));

    for _ in 0..2 {
        driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    }
    driver.run_settled();
    let TableArchetype::MultipleVariables(mv_undo) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!("expected MV")
    };
    assert_eq!(mv_undo.variables[0].title, "Dose");
    assert_eq!(mv_undo.variables[2].source, VariableSource::Calculated { formula: String::from("[Dose] / [Weight]") });
    assert_eq!(ratio_vals(&driver.state().app), vec![Some(5.0), Some(4.0), Some(5.0)]);
    Ok(())
}
