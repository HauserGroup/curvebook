#![forbid(unsafe_code)]

mod human_wf_common;

use curvebook_doc::{
    Cell, CellCoord, ColorChannelMapping, DataSheet, GraphKind, GraphSpec, LevelOrderKind, MultipleVariablesTable, MultivariatePlotMode,
    ProjectDocument, SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype, VariableColumn, VariableRole, VariableSource, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::i18n::{CATALOG, LOCALES};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use egui::{Key, Modifiers};
use human_wf_common::{EguiWfHarness, assert_input_to_paint_budget, assert_zero_shortcut_compliance, default_viewport, ensure};
use serde_json::json;

fn build_mv_cat_and_graph_app(col_order: [(&str, VariableRole); 4]) -> Result<UiApp, String> {
    let mut mv = MultipleVariablesTable::new(4, 6).map_err(|e| format!("create mv: {e}"))?;
    let sample_ids = ["S1", "S2", "S3", "S4", "S5", "S6"];
    let arms = ["Ctrl", "DrugA", "DrugB", "Ctrl", "DrugA", "DrugB"];
    let scores = [10.0, 22.0, 34.0, 12.0, 24.0, 36.0];
    let sizes = [2.0, 4.0, 8.0, 3.0, 5.0, 9.0];

    for (col_idx, (title, role)) in col_order.iter().enumerate() {
        let cells = match *title {
            "SampleID" => sample_ids.iter().map(|s| Cell::text(*s)).collect(),
            "Arm" => arms.iter().map(|s| Cell::text(*s)).collect(),
            "Score" => scores.iter().map(|v| Cell::number(*v)).collect(),
            _ => sizes.iter().map(|v| Cell::number(*v)).collect(),
        };
        mv.variables[col_idx] = VariableColumn {
            title: String::from(*title),
            role: *role,
            role_inferred: false,
            source: VariableSource::Entered,
            level_order: Default::default(),
            cells,
        };
    }

    let ds = DataSheet::new("mv-1", "Cohort MV", TableArchetype::MultipleVariables(mv)).map_err(|e| format!("ds: {e}"))?;
    let mut doc = ProjectDocument::new("Wave 7 MV UI Study").map_err(|e| format!("doc: {e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("add ds: {e}"))?;
    let mut session = Session::from_document(doc);
    session
        .execute("graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "mv-1", "kind": "multiple_variables", "initialized": true }))
        .map_err(|e| format!("graph.create: {e}"))?;
    session.execute("sheet.select", json!({ "sheet_id": "mv-1" })).map_err(|e| format!("select mv-1: {e}"))?;
    let mut app = UiApp::from_session(session);
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    Ok(app)
}

#[test]
fn r10_8_b3_frozen_table_delete_and_backspace_blocked_human_path() -> Result<(), String> {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).map_err(|e| format!("xy: {e}"))?;
    for row in 0..4 {
        xy.set_x_cell(row, Cell::number((row + 1) as f64)).map_err(|e| format!("set x: {e}"))?;
        xy.data_sets[0].cells[row][0] = Cell::number(10.0 * (row + 1) as f64);
        xy.data_sets[1].cells[row][0] = Cell::number(20.0 * (row + 1) as f64);
    }
    let mut ds = DataSheet::new("ds-frozen", "Frozen XY", TableArchetype::Xy(xy)).map_err(|e| format!("ds: {e}"))?;
    ds.frozen = true;
    let mut doc = ProjectDocument::new("Frozen Guard Study").map_err(|e| format!("doc: {e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("add ds: {e}"))?;
    doc.select_in_navigator(&SheetId::from("ds-frozen")).map_err(|e| format!("select: {e}"))?;

    let mut h = EguiWfHarness::from_app(UiApp::from_session(Session::from_document(doc)), default_viewport());

    // Click frozen cell (0,0,0) directly by coordinate, extend to range with Shift+ArrowDown, press Delete & Backspace, type, and double-click cells.
    let cell_pos = h.widget_center("grid.cell.0.0.0")?;
    h.driver.click_at(cell_pos)?;
    h.driver.press_key("ArrowDown", KeyModifiers { shift: true, ..Default::default() })?;
    h.driver.press_key("Delete", KeyModifiers::default())?;
    h.driver.press_key("Backspace", KeyModifiers::default())?;
    h.driver.type_text("999")?;
    ensure(!h.app().inline_editor.active, "typing on frozen data grid must not activate inline editor")?;

    h.dbl_click_widget("grid.cell.0.0.0")?;
    ensure(!h.app().inline_editor.active, "double-click on frozen data cell must not open inline editor")?;

    h.dbl_click_widget("grid.xcell.0.0")?;
    ensure(!h.app().inline_editor.active, "double-click on frozen X cell must not open inline editor")?;

    let ds_after = h.app().document().data_sheet(&SheetId::from("ds-frozen")).ok_or("missing ds-frozen")?;
    assert_eq!(ds_after.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(10.0));
    assert_eq!(ds_after.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(20.0));
    assert_eq!(ds_after.x_or_time_cell(0, 0).and_then(|c| c.active_number()), Some(1.0));

    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)?;
    h.assert_parity_and_layout("b3_frozen_table_guard")
}

#[test]
fn r10_8_b4_keyboard_stepwise_explicit_level_reorder_and_format_graph_mirror() -> Result<(), String> {
    let app = build_mv_cat_and_graph_app([
        ("SampleID", VariableRole::Label),
        ("Arm", VariableRole::Categorical),
        ("Score", VariableRole::Continuous),
        ("Size", VariableRole::Continuous),
    ])?;
    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 900.0, 1.0));

    // 1. Open Manage Variables modal via column role badge menu, select Arm (col 1), operate Explicit Move Up/Top/End/Down.
    h.click("mv_formula.role_badge.1")?;
    h.click("mv_formula.role_manage.1")?;
    h.click("mv_formula.vars.col.1")?;
    h.assert_parity_and_layout("b4_manage_vars_open")?;

    h.click("mv_formula.vars.level.2")?; // DrugB
    h.click("mv_formula.vars.move_up")?;
    {
        let TableArchetype::MultipleVariables(mv) = &h.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(
            mv.variables[1].level_order.order,
            LevelOrderKind::Explicit(vec![String::from("Ctrl"), String::from("DrugB"), String::from("DrugA")])
        );
    }

    h.click("mv_formula.vars.move_top")?;
    {
        let TableArchetype::MultipleVariables(mv) = &h.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(
            mv.variables[1].level_order.order,
            LevelOrderKind::Explicit(vec![String::from("DrugB"), String::from("Ctrl"), String::from("DrugA")])
        );
    }

    h.click("mv_formula.vars.move_end")?;
    {
        let TableArchetype::MultipleVariables(mv) = &h.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(
            mv.variables[1].level_order.order,
            LevelOrderKind::Explicit(vec![String::from("Ctrl"), String::from("DrugA"), String::from("DrugB")])
        );
    }

    h.click("mv_formula.vars.level.0")?; // Ctrl
    h.click("mv_formula.vars.move_down")?;
    {
        let TableArchetype::MultipleVariables(mv) = &h.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(
            mv.variables[1].level_order.order,
            LevelOrderKind::Explicit(vec![String::from("DrugA"), String::from("Ctrl"), String::from("DrugB")])
        );
    }
    h.click("mv_formula.vars.close")?;

    // 2. Mirror Explicit level reordering in FormatGraph modal + verify Cmd+Z reverts cleanly.
    h.select_sheet(&SheetId::from("gr-1"))?;
    h.click("inspector.graph.more_format")?;
    h.click("edit.tab.format_graph.axis_variables")?;
    h.assert_parity_and_layout("b4_format_graph_axis_vars")?;

    h.click("format_graph.mv.level_cycle")?; // cycles DrugA -> Ctrl
    h.click("format_graph.mv.level_top")?;
    h.click("modal.ok")?;

    let enc_fg =
        h.app().document().graph_sheet(&SheetId::from("gr-1")).and_then(|g| g.spec.multivariate.as_ref()).ok_or("missing mv encoding on gr-1")?;
    assert_eq!(
        enc_fg.x_level_order_override.as_ref().map(|s| &s.order),
        Some(&LevelOrderKind::Explicit(vec![String::from("Ctrl"), String::from("DrugA"), String::from("DrugB")]))
    );

    // Cmd+Z reverts Format Graph level override cleanly.
    h.driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    let enc_undo =
        h.app().document().graph_sheet(&SheetId::from("gr-1")).and_then(|g| g.spec.multivariate.as_ref()).ok_or("missing mv encoding after undo")?;
    assert!(enc_undo.x_level_order_override.is_none(), "Cmd+Z must revert x_level_order_override");

    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_b5_calculated_variable_confirmation_only_validation() -> Result<(), String> {
    let app = build_mv_cat_and_graph_app([
        ("SampleID", VariableRole::Label),
        ("Arm", VariableRole::Categorical),
        ("Score", VariableRole::Continuous),
        ("Size", VariableRole::Continuous),
    ])?;
    let mut h = EguiWfHarness::from_app(app, default_viewport());

    // Add calculated variable slot via Trailing Studio -> enter invalid formula -> Apply remains disabled & columns untouched.
    h.click("inspector.mv.col.2")?;
    let cols_before = h
        .app()
        .document()
        .data_sheet(&SheetId::from("mv-1"))
        .and_then(|d| if let TableArchetype::MultipleVariables(mv) = &d.archetype { Some(mv.variables.len()) } else { None });
    h.replace_textbox("inspector.mv.formula_input", "= [Score] +")?;
    let apply_bad = h.driver.frame_meta().find_by_widget_id("inspector.mv.apply_formula").ok_or("missing inspector.mv.apply_formula")?;
    ensure(!apply_bad.enabled, "invalid syntax must keep inspector.mv.apply_formula disabled")?;
    let _ = h.driver.click_at(apply_bad.clipped_rect.center());
    let cols_after_bad = h
        .app()
        .document()
        .data_sheet(&SheetId::from("mv-1"))
        .and_then(|d| if let TableArchetype::MultipleVariables(mv) = &d.archetype { Some(mv.variables.len()) } else { None });
    assert_eq!(cols_before, cols_after_bad);

    // Valid formula commits calculated variable on Score column (or new column via command); verify 2-hop cycle guard!
    h.app_mut()
        .execute_command("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "title": "DerivedScore", "formula": "= [Score] * 2" }))
        .map_err(|e| format!("DerivedScore: {e}"))?;
    h.app_mut()
        .execute_command("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "title": "DoubleDerived", "formula": "= [DerivedScore] + 1" }))
        .map_err(|e| format!("DoubleDerived: {e}"))?;
    h.driver.run_settled();

    // Select DerivedScore (col 4) and type circular reference `= [DoubleDerived] + 5`: Apply disables cleanly + UI stays healthy!
    h.click("inspector.mv.col.4")?;
    h.replace_textbox("inspector.mv.formula_input", "= [DoubleDerived] + 5")?;
    let apply_cyc = h.driver.frame_meta().find_by_widget_id("inspector.mv.apply_formula").ok_or("missing apply button on cycle check")?;
    ensure(!apply_cyc.enabled, "2-hop circular reference must disable inspector.mv.apply_formula")?;
    let _ = h.driver.click_at(apply_cyc.clipped_rect.center());

    // Verify valid update succeeds cleanly through the UI button.
    h.replace_textbox("inspector.mv.formula_input", "= [Score] * 3")?;
    h.click("inspector.mv.apply_formula")?;
    let TableArchetype::MultipleVariables(mv_final) = &h.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
        return Err(String::from("expected MV"));
    };
    assert_eq!(mv_final.variables[4].cells[0].active_number(), Some(30.0));

    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)?;
    h.assert_parity_and_layout("b5_calculated_variable_guard")
}

#[test]
fn r10_18_b1_inspector_mv_channels_variable_defs_grouping_and_defaults() -> Result<(), String> {
    // 1. Verify GraphSpec::default_for_data_sheet assigns Categorical X and Continuous Y automatically.
    let app = build_mv_cat_and_graph_app([
        ("Score", VariableRole::Continuous),
        ("Size", VariableRole::Continuous),
        ("SampleID", VariableRole::Label),
        ("Arm", VariableRole::Categorical),
    ])?;
    {
        let ds_ref = app.document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?;
        let def_spec = GraphSpec::default_for_data_sheet(ds_ref);
        let def_enc = def_spec.multivariate.as_ref().ok_or("default mv encoding")?;
        assert_eq!(def_enc.x_column, 3, "expected first Categorical column (Arm=3) as X default");
        assert_eq!(def_enc.y_column, 0, "expected first Continuous column (Score=0) as Y default");

        let gs_live = app.document().graph_sheet(&SheetId::from("gr-1")).ok_or("gr-1")?;
        let live_enc = gs_live.spec.multivariate.as_ref().ok_or("live mv encoding")?;
        assert_eq!(live_enc.x_column, 3);
        assert_eq!(live_enc.y_column, 0);
    }

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&SheetId::from("gr-1"))?;

    // Open Bubble & MV Encoding dialog via Trailing Studio -> configure X, Y, Bubble mode, hulls/ellipses, then Size in FormatGraph.
    h.click("inspector.graph.more_bubble")?;
    h.click("bubble.x_next")?; // 3 -> 0 (Score)
    h.click("bubble.y_next")?; // 0 -> 1 (Size)
    h.click("bubble.show_convex_hulls")?;
    h.click("bubble.show_confidence_ellipses")?;
    h.click("modal.ok")?;

    h.click("inspector.graph.more_format")?;
    h.click("edit.tab.format_graph.axis_variables")?;
    h.click("format_graph.mv.color_categorical")?;
    h.click("edit.tab.format_graph.data_objects")?;
    h.click("format_graph.mv.size_variable")?;
    h.click("modal.ok")?;

    let gs_after = h.app().document().graph_sheet(&SheetId::from("gr-1")).ok_or("gr-1")?;
    let enc_after = gs_after.spec.multivariate.as_ref().ok_or("mv enc after")?;
    assert_eq!(enc_after.plot_mode, MultivariatePlotMode::BubbleScatter);
    assert_eq!(enc_after.x_column, 0);
    assert_eq!(enc_after.y_column, 1);
    assert!(matches!(enc_after.color, ColorChannelMapping::Categorical { .. }));
    assert!(matches!(enc_after.size, SizeChannelMapping::Variable { .. }));
    assert_eq!(gs_after.spec.kind, GraphKind::MultipleVariables);

    // Switch back to mv-1 and change variable role via column role menu (`mv_formula.role_badge.2`) + undo with Cmd+Z.
    h.select_sheet(&SheetId::from("mv-1"))?;
    h.click("mv_formula.role_badge.2")?;
    h.click("mv_formula.role_opt.2.categorical")?;

    let TableArchetype::MultipleVariables(mv_role_after) = &h.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
        return Err(String::from("expected MV"));
    };
    assert_eq!(mv_role_after.variables[2].role, VariableRole::Categorical);

    h.driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    let TableArchetype::MultipleVariables(mv_role_undo) = &h.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
        return Err(String::from("expected MV"));
    };
    assert_eq!(mv_role_undo.variables[2].role, VariableRole::Label);

    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_w7_staged_i18n_16_locales_and_rtl_parity() -> Result<(), String> {
    let mv_entries: Vec<_> = CATALOG.iter().filter(|e| e.key.starts_with("w7.mv.")).collect();
    assert!(!mv_entries.is_empty());
    for entry in mv_entries {
        assert!(!entry.key.trim().is_empty());
        for (i, tr) in entry.translations.iter().enumerate() {
            assert!(!tr.trim().is_empty(), "empty staged translation for `{}` locale idx {i}", entry.key);
        }
    }
    for loc in LOCALES {
        let mut app = build_mv_cat_and_graph_app([
            ("SampleID", VariableRole::Label),
            ("Arm", VariableRole::Categorical),
            ("Score", VariableRole::Continuous),
            ("Size", VariableRole::Continuous),
        ])?;
        assert_eq!(app.set_locale(loc), *loc);
        let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
        h.click("mv_formula.role_badge.1")?;
        h.click("mv_formula.role_manage.1")?;
        h.assert_parity_and_layout(&format!("mv_manage_vars_{loc}"))?;
        h.click("mv_formula.vars.close")?;
        h.select_sheet(&SheetId::from("gr-1"))?;
        h.click("inspector.graph.more_format")?;
        h.assert_parity_and_layout(&format!("mv_format_graph_{loc}"))?;
    }
    Ok(())
}
