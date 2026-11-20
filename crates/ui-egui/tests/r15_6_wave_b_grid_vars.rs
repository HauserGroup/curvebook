#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave B (`R15.6`) human-path GUI regression suite for Data Grid (`B4`) and Manage Variables (`B9`):
//! - `R15-GUI-11`: Multi-line / multi-cell X entry via inline edit routes through `table.setXCells`
//!   (preserving target X subcolumn, starting row, and locale decimal parsing) without touching Y.
//! - `R15-GUI-12`: Selecting an X cell clears Y selection highlights (and vice versa) and routes
//!   `Cmd/Ctrl+C`, `Cmd/Ctrl+X`, `Cmd/Ctrl+V` through an X-first branch leaving Y cells untouched.
//! - `R15-GUI-06`: Quick-stats scan on an empty column vector at `r0 == 0` never panics.
//! - `R15-GUI-21`: Adding a unit to a `MultipleVariables` column in Manage Variables rewrites
//!   `[Title]` references in calculated variables, re-evaluates cleanly, and undoes atomically.

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, MultipleVariablesTable, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, VariableSource,
    XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::status_footer::{QuickStatsCache, compute_cached_quick_stats};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

struct ShellDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_intent_error: Option<String>,
}

impl ShellDriverState {
    fn new(mut app: UiApp) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        Self { app, views: EguiShellViewState::default(), intents: PendingUiIntents::default(), last_intent_error: None }
    }
}

fn make_shell_driver(doc: ProjectDocument, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellDriverState> {
    let app = UiApp::from_session(Session::from_document(doc));
    let tokens = Tokens::default();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ShellDriverState::new(app),
        ViewportMetrics::new(w, h, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_intent_error = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn widget_center(driver: &EguiHumanPathDriver<'_, ShellDriverState>, widget_id: &str) -> Result<Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    Ok(entry.clipped_rect.center())
}

fn widget_selected(driver: &EguiHumanPathDriver<'_, ShellDriverState>, widget_id: &str) -> Option<bool> {
    driver
        .frame_meta()
        .find_by_widget_id(widget_id)
        .and_then(|e| e.value.as_ref())
        .and_then(|v| v.get("selected"))
        .and_then(serde_json::Value::as_bool)
}

fn build_xy_doc(x_subcols: SubcolumnMode) -> ProjectDocument {
    let mut doc = ProjectDocument::new("R15.6 XY Grid").expect("project");
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: x_subcols }, SubcolumnMode::Single, 2, 4).expect("xy");
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
    let doc = build_xy_doc(SubcolumnMode::Single);
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    // Double-click X cell row 0 to start inline editing, then enter multi-line `"1\n2\n3"`.
    let x0_pos = widget_center(&driver, "grid.xcell.0.0")?;
    driver.click_at(x0_pos)?;
    driver.click_at(x0_pos)?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text("1\n2\n3")?;
    driver.press_key("Enter", KeyModifiers::default())?;

    assert!(driver.state().last_intent_error.is_none(), "unexpected intent error: {:?}", driver.state().last_intent_error);
    let ds = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    let TableArchetype::Xy(xy) = &ds.archetype else { panic!("expected Xy") };
    assert_eq!(xy.x_cells[0][0].active_number(), Some(1.0));
    assert_eq!(xy.x_cells[1][0].active_number(), Some(2.0));
    assert_eq!(xy.x_cells[2][0].active_number(), Some(3.0));
    assert_eq!(xy.x_cells[3][0].active_number(), Some(40.0));
    // Y data set A must be completely untouched.
    assert_eq!(xy.data_sets[0].cells[0][0].active_number(), Some(100.0));
    assert_eq!(xy.data_sets[0].cells[1][0].active_number(), Some(101.0));
    assert_eq!(xy.data_sets[0].cells[2][0].active_number(), Some(102.0));
    assert_eq!(xy.data_sets[0].cells[3][0].active_number(), Some(103.0));

    // Also verify non-zero starting row + X subcolumn index + comma-decimal locale parsing.
    let doc2 = build_xy_doc(SubcolumnMode::RawReplicates(2));
    let mut driver2 = make_shell_driver(doc2, 1280.0, 800.0);
    let _ = driver2.state_mut().app.set_locale("de-DE");
    driver2.run_settled();

    let x11_pos = widget_center(&driver2, "grid.xcell.1.1")?;
    driver2.click_at(x11_pos)?;
    driver2.click_at(x11_pos)?;
    driver2.press_egui_key(Key::A, Modifiers::COMMAND);
    driver2.type_text("1,5\n2,5")?;

    assert!(driver2.state().last_intent_error.is_none(), "unexpected intent error: {:?}", driver2.state().last_intent_error);
    let ds2 = driver2.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    let TableArchetype::Xy(xy2) = &ds2.archetype else { panic!("expected Xy") };
    assert_eq!(xy2.x_cells[0][1].active_number(), Some(11.0));
    assert_eq!(xy2.x_cells[1][1].active_number(), Some(1.5));
    assert_eq!(xy2.x_cells[2][1].active_number(), Some(2.5));
    assert_eq!(xy2.x_cells[1][0].active_number(), Some(20.0));
    assert_eq!(xy2.data_sets[0].cells[1][0].active_number(), Some(101.0));
    Ok(())
}

#[test]
fn r15_gui_12_x_selection_clears_y_and_routes_clipboard_x_first() -> Result<(), String> {
    let doc = build_xy_doc(SubcolumnMode::Single);
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    // 1. Select Y cell (0,0,0) and confirm it is selected.
    driver.click_widget_id("grid.cell.0.0.0")?;
    assert_eq!(widget_selected(&driver, "grid.cell.0.0.0"), Some(true));
    assert_eq!(widget_selected(&driver, "grid.xcell.1.0"), Some(false));

    // 2. Click X cell row 1 -> Y selection highlight must clear and X row 1 must be selected.
    driver.click_widget_id("grid.xcell.1.0")?;
    assert_eq!(widget_selected(&driver, "grid.xcell.1.0"), Some(true));
    assert_eq!(widget_selected(&driver, "grid.cell.0.0.0"), Some(false));

    // 3. Copy X row 1 (value 20.0), click X row 3, Paste -> updates X row 3 to 20.0, leaves Y untouched.
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    driver.click_widget_id("grid.xcell.3.0")?;
    assert_eq!(widget_selected(&driver, "grid.xcell.3.0"), Some(true));
    assert_eq!(widget_selected(&driver, "grid.cell.0.0.0"), Some(false));
    driver.press_egui_key(Key::V, Modifiers::COMMAND);

    let ds = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    let TableArchetype::Xy(xy) = &ds.archetype else { panic!("expected Xy") };
    assert_eq!(xy.x_cells[3][0].active_number(), Some(20.0));
    assert_eq!(xy.data_sets[0].cells[0][0].active_number(), Some(100.0));
    assert_eq!(xy.data_sets[0].cells[3][0].active_number(), Some(103.0));

    // 4. Cut X row 2 (value 30.0) -> clears X row 2 and leaves Y row 0/2 completely untouched.
    driver.click_widget_id("grid.xcell.2.0")?;
    driver.press_egui_key(Key::X, Modifiers::COMMAND);
    let ds_after_cut = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    let TableArchetype::Xy(xy_after_cut) = &ds_after_cut.archetype else { panic!("expected Xy") };
    assert!(xy_after_cut.x_cells[2][0].is_empty(), "Cmd+X must clear selected X cell");
    assert_eq!(xy_after_cut.data_sets[0].cells[0][0].active_number(), Some(100.0));
    assert_eq!(xy_after_cut.data_sets[0].cells[2][0].active_number(), Some(102.0));

    // Paste the cut value (30.0) into X row 0.
    driver.click_widget_id("grid.xcell.0.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    let ds_after_paste = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    let TableArchetype::Xy(xy_after_paste) = &ds_after_paste.archetype else { panic!("expected Xy") };
    assert_eq!(xy_after_paste.x_cells[0][0].active_number(), Some(30.0));
    assert_eq!(xy_after_paste.data_sets[0].cells[0][0].active_number(), Some(100.0));

    // 5. Clicking a Y cell again must clear the X selection highlight.
    driver.click_widget_id("grid.cell.0.0.0")?;
    assert_eq!(widget_selected(&driver, "grid.cell.0.0.0"), Some(true));
    assert_eq!(widget_selected(&driver, "grid.xcell.0.0"), Some(false));
    assert!(driver.state().last_intent_error.is_none(), "unexpected intent error: {:?}", driver.state().last_intent_error);
    Ok(())
}

#[test]
fn r15_gui_06_quick_stats_empty_column_vec_at_r0_zero_no_panic() {
    let mut doc = ProjectDocument::new("R15.6 Empty Col").expect("project");
    let col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).expect("col");
    let mut ds = DataSheet::new("ds-empty", "Empty", TableArchetype::Column(col_tbl)).unwrap();
    if let TableArchetype::Column(ref mut c) = ds.archetype {
        c.data_sets[0].cells.clear();
    }
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(SheetId::from("ds-empty"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    let mut cache = QuickStatsCache::default();
    let summary = compute_cached_quick_stats(&app, &mut cache);
    assert_eq!(summary.n, 0);
    assert_eq!(summary.mean, None);
}

#[test]
fn r15_gui_21_manage_vars_unit_edit_rewrites_formula_reevaluates_and_undoes() -> Result<(), String> {
    let mut doc = ProjectDocument::new("R15.6 MV Rename").expect("project");
    let mut mv = MultipleVariablesTable::new(3, 3).expect("mv");
    mv.variables[0].title = String::from("Dose");
    mv.variables[1].title = String::from("Weight");
    mv.variables[2].title = String::from("Ratio");
    let doses = [10.0, 20.0, 45.0];
    let weights = [2.0, 5.0, 9.0];
    for r in 0..3 {
        mv.variables[0].cells[r] = Cell::number(doses[r]);
        mv.variables[1].cells[r] = Cell::number(weights[r]);
    }
    doc.add_data_sheet(DataSheet::new("ds-mv", "PK Table", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    let mut driver = make_shell_driver(doc, 1280.0, 800.0);
    driver
        .state_mut()
        .app
        .execute_command("table.setCalculatedVariable", json!({ "sheet_id": "ds-mv", "col": 2, "formula": "[Dose] / [Weight]" }))
        .map_err(|e| e.to_string())?;
    driver.run_settled();

    // Verify initial calculated values [5.0, 4.0, 5.0].
    {
        let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
            panic!("expected MultipleVariables")
        };
        assert_eq!(mv.variables[2].cells[0].active_number(), Some(5.0));
        assert_eq!(mv.variables[2].cells[1].active_number(), Some(4.0));
        assert_eq!(mv.variables[2].cells[2].active_number(), Some(5.0));
    }

    // Open Manage Variables modal targeting `Dose` (col 0) and add unit `mg/kg` through the rendered widget.
    driver.state_mut().intents.push_open_manage_variables(SheetId::from("ds-mv"), Some(0));
    {
        let st = driver.state_mut();
        st.app.egui_views = st.views.clone();
        st.intents.commit_if_settled(&mut st.app, false).map_err(|e| e.to_string())?;
        st.views = st.app.egui_views.clone();
    }
    driver.run_settled();

    driver.click_widget_id("mv_formula.vars.unit_input")?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text("mg/kg")?;
    let undo_before = driver.state().app.session().undo_depth();
    driver.click_widget_id("mv_formula.vars.meta.apply")?;

    assert!(driver.state().last_intent_error.is_none(), "rename via Manage Variables failed: {:?}", driver.state().last_intent_error);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);

    // Title is `Dose (mg/kg)`, formula rewrote to `[Dose (mg/kg)] / [Weight]`, and values evaluate cleanly.
    {
        let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
            panic!("expected MultipleVariables")
        };
        assert_eq!(mv.variables[0].title, "Dose (mg/kg)");
        assert_eq!(mv.variables[2].source, VariableSource::Calculated { formula: String::from("[Dose (mg/kg)] / [Weight]") });
        assert_eq!(mv.variables[2].cells[0].active_number(), Some(5.0));
        assert_eq!(mv.variables[2].cells[1].active_number(), Some(4.0));
        assert_eq!(mv.variables[2].cells[2].active_number(), Some(5.0));
    }

    // Mutate Dose row 0 -> 16.0 to confirm live recalculation uses the rewritten formula without error.
    driver
        .state_mut()
        .app
        .execute_command("table.setCell", json!({ "sheet_id": "ds-mv", "row": 0, "col": 0, "value": 16.0 }))
        .map_err(|e| e.to_string())?;
    {
        let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
            panic!("expected MultipleVariables")
        };
        assert_eq!(mv.variables[2].cells[0].active_number(), Some(8.0));
    }

    // Undo cell edit, then Undo rename -> restores original `Dose` title AND `[Dose] / [Weight]` formula!
    driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    driver.run_settled();

    let TableArchetype::MultipleVariables(mv_undo) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!("expected MultipleVariables")
    };
    assert_eq!(mv_undo.variables[0].title, "Dose");
    assert_eq!(mv_undo.variables[2].source, VariableSource::Calculated { formula: String::from("[Dose] / [Weight]") });
    assert_eq!(mv_undo.variables[2].cells[0].active_number(), Some(5.0));
    assert_eq!(mv_undo.variables[2].cells[1].active_number(), Some(4.0));
    assert_eq!(mv_undo.variables[2].cells[2].active_number(), Some(5.0));
    Ok(())
}
