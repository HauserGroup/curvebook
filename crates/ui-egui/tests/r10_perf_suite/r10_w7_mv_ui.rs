#![forbid(unsafe_code)]

use crate::common::{TestRes, cell_number, ensure, seed_xy_doc};
use crate::human_wf_common::{EguiWfHarness, assert_input_to_paint_budget, assert_zero_shortcut_compliance, default_viewport};
use crate::suite_helpers::{build_mv_sheet, new_shell_app, run_cmd};
use curvebook_doc::{
    Cell, ColorChannelMapping, GraphKind, GraphSpec, LevelOrderKind, MultipleVariablesTable, MultivariatePlotMode, ProjectDocument, SheetId,
    SizeChannelMapping, TableArchetype, VariableRole,
};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::i18n::{CATALOG, LOCALES};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use egui::{Key, Modifiers};
use serde_json::json;

const DEFAULT_MV_COLS: [(&str, VariableRole); 4] =
    [("SampleID", VariableRole::Label), ("Arm", VariableRole::Categorical), ("Score", VariableRole::Continuous), ("Size", VariableRole::Continuous)];

fn build_mv_cat_and_graph_app(col_order: [(&str, VariableRole); 4]) -> Result<UiApp, String> {
    let ds = build_mv_sheet("mv-1", "Cohort MV", &col_order, 6, |mv| {
        for (c, (title, _)) in col_order.iter().enumerate() {
            mv.variables[c].cells = match *title {
                "SampleID" => ["S1", "S2", "S3", "S4", "S5", "S6"].iter().map(|s| Cell::text(*s)).collect(),
                "Arm" => ["Ctrl", "DrugA", "DrugB", "Ctrl", "DrugA", "DrugB"].iter().map(|s| Cell::text(*s)).collect(),
                "Score" => [10.0, 22.0, 34.0, 12.0, 24.0, 36.0].iter().map(|v| Cell::number(*v)).collect(),
                _ => [2.0, 4.0, 8.0, 3.0, 5.0, 9.0].iter().map(|v| Cell::number(*v)).collect(),
            };
        }
    })?;
    let mut doc = ProjectDocument::new("Wave 7 MV UI Study").map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    let mut app = new_shell_app(doc);
    run_cmd(&mut app, "graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "mv-1", "kind": "multiple_variables", "initialized": true }))?;
    run_cmd(&mut app, "sheet.select", json!({ "sheet_id": "mv-1" }))?;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    Ok(app)
}

fn mv_table(h: &EguiWfHarness) -> Result<&MultipleVariablesTable, String> {
    match &h.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype {
        TableArchetype::MultipleVariables(mv) => Ok(mv),
        _ => Err("expected MV".into()),
    }
}

#[test]
fn r10_8_b3_frozen_table_delete_and_backspace_blocked_human_path() -> TestRes {
    let (mut doc, id) = seed_xy_doc(
        "Frozen Guard Study",
        "ds-frozen",
        "Frozen XY",
        "X",
        &[1.0, 2.0, 3.0, 4.0],
        &[("Y1", &[10.0, 20.0, 30.0, 40.0]), ("Y2", &[20.0, 40.0, 60.0, 80.0])],
    )?;
    doc.data_sheet_mut(&id).ok_or("missing ds-frozen")?.frozen = true;
    let mut h = EguiWfHarness::from_app(new_shell_app(doc), default_viewport());

    h.driver.click_at(h.widget_center("grid.cell.0.0.0")?)?;
    h.driver.press_key("ArrowDown", KeyModifiers { shift: true, ..Default::default() })?;
    for key in ["Delete", "Backspace"] {
        h.driver.press_key(key, KeyModifiers::default())?;
    }
    h.driver.type_text("999")?;
    ensure(!h.app().inline_editor.active, "typing on frozen grid must not activate inline editor")?;
    for wid in ["grid.cell.0.0.0", "grid.xcell.0.0"] {
        h.dbl_click_widget(wid)?;
        ensure(!h.app().inline_editor.active, format!("double-click on frozen `{wid}` must not open editor"))?;
    }
    let ds = h.app().document().data_sheet(&id).ok_or("missing ds-frozen")?;
    assert_eq!(
        (cell_number(ds, 0, 0), cell_number(ds, 1, 0), ds.x_or_time_cell(0, 0).and_then(Cell::active_number)),
        (Some(10.0), Some(20.0), Some(1.0))
    );

    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)?;
    h.assert_parity_and_layout("b3_frozen_table_guard")
}

#[test]
fn r10_8_b4_keyboard_stepwise_explicit_level_reorder_and_format_graph_mirror() -> TestRes {
    let mut h = EguiWfHarness::from_app(build_mv_cat_and_graph_app(DEFAULT_MV_COLS)?, ViewportMetrics::new(1440.0, 900.0, 1.0));
    for wid in ["mv_formula.role_badge.1", "mv_formula.role_manage.1", "mv_formula.vars.col.1"] {
        h.click(wid)?;
    }
    h.assert_parity_and_layout("b4_manage_vars_open")?;

    for (select_wid, action_wid, expected) in [
        (Some("mv_formula.vars.level.2"), "mv_formula.vars.move_up", ["Ctrl", "DrugB", "DrugA"]),
        (None, "mv_formula.vars.move_top", ["DrugB", "Ctrl", "DrugA"]),
        (None, "mv_formula.vars.move_end", ["Ctrl", "DrugA", "DrugB"]),
        (Some("mv_formula.vars.level.0"), "mv_formula.vars.move_down", ["DrugA", "Ctrl", "DrugB"]),
    ] {
        if let Some(sel) = select_wid {
            h.click(sel)?;
        }
        h.click(action_wid)?;
        assert_eq!(mv_table(&h)?.variables[1].level_order.order, LevelOrderKind::Explicit(expected.into_iter().map(String::from).collect()));
    }
    h.click("mv_formula.vars.close")?;

    // Mirror Explicit level reordering in FormatGraph modal + verify Cmd+Z reverts.
    h.select_sheet(&SheetId::from("gr-1"))?;
    h.click("inspector.graph.more_format")?;
    h.click("edit.tab.format_graph.axis_variables")?;
    h.assert_parity_and_layout("b4_format_graph_axis_vars")?;
    for wid in ["format_graph.mv.level_cycle", "format_graph.mv.level_top", "modal.ok"] {
        h.click(wid)?;
    }
    let enc = h.app().document().graph_sheet(&SheetId::from("gr-1")).and_then(|g| g.spec.multivariate.as_ref()).ok_or("gr-1 enc")?;
    assert_eq!(
        enc.x_level_order_override.as_ref().map(|s| &s.order),
        Some(&LevelOrderKind::Explicit(vec!["Ctrl".into(), "DrugA".into(), "DrugB".into()]))
    );

    h.driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert!(
        h.app()
            .document()
            .graph_sheet(&SheetId::from("gr-1"))
            .and_then(|g| g.spec.multivariate.as_ref())
            .ok_or("gr-1")?
            .x_level_order_override
            .is_none()
    );
    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn r10_8_b5_calculated_variable_confirmation_only_validation() -> TestRes {
    let mut h = EguiWfHarness::from_app(build_mv_cat_and_graph_app(DEFAULT_MV_COLS)?, default_viewport());
    h.click("inspector.mv.col.2")?;
    let cols_before = mv_table(&h)?.variables.len();
    h.replace_textbox("inspector.mv.formula_input", "= [Score] +")?;
    let apply_bad = h.driver.frame_meta().find_by_widget_id("inspector.mv.apply_formula").ok_or("missing apply_formula")?.clone();
    ensure(!apply_bad.enabled, "invalid syntax must disable inspector.mv.apply_formula")?;
    let _ = h.driver.click_at(apply_bad.clipped_rect.center());
    assert_eq!(mv_table(&h)?.variables.len(), cols_before);

    // 2-hop circular reference guard + valid formula apply.
    run_cmd(h.app_mut(), "table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "title": "DerivedScore", "formula": "= [Score] * 2" }))?;
    run_cmd(h.app_mut(), "table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "title": "DoubleDerived", "formula": "= [DerivedScore] + 1" }))?;
    h.driver.run_settled();
    h.click("inspector.mv.col.4")?;
    h.replace_textbox("inspector.mv.formula_input", "= [DoubleDerived] + 5")?;
    let apply_cyc = h.driver.frame_meta().find_by_widget_id("inspector.mv.apply_formula").ok_or("missing apply on cycle")?.clone();
    ensure(!apply_cyc.enabled, "2-hop circular reference must disable inspector.mv.apply_formula")?;
    let _ = h.driver.click_at(apply_cyc.clipped_rect.center());

    h.replace_textbox("inspector.mv.formula_input", "= [Score] * 3")?;
    h.click("inspector.mv.apply_formula")?;
    assert_eq!(mv_table(&h)?.variables[4].cells[0].active_number(), Some(30.0));
    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)?;
    h.assert_parity_and_layout("b5_calculated_variable_guard")
}

#[test]
fn r10_18_b1_inspector_mv_channels_variable_defs_grouping_and_defaults() -> TestRes {
    let app = build_mv_cat_and_graph_app([
        ("Score", VariableRole::Continuous),
        ("Size", VariableRole::Continuous),
        ("SampleID", VariableRole::Label),
        ("Arm", VariableRole::Categorical),
    ])?;
    let def_spec = GraphSpec::default_for_data_sheet(app.document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?);
    let def_enc = def_spec.multivariate.as_ref().ok_or("default mv enc")?;
    let live_enc = app.document().graph_sheet(&SheetId::from("gr-1")).and_then(|g| g.spec.multivariate.as_ref()).ok_or("live enc")?;
    assert_eq!((def_enc.x_column, def_enc.y_column, live_enc.x_column, live_enc.y_column), (3, 0, 3, 0));

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&SheetId::from("gr-1"))?;
    for wid in [
        "inspector.graph.more_bubble",
        "bubble.x_next",
        "bubble.y_next",
        "bubble.show_convex_hulls",
        "bubble.show_confidence_ellipses",
        "modal.ok",
        "inspector.graph.more_format",
        "edit.tab.format_graph.axis_variables",
        "format_graph.mv.color_categorical",
        "edit.tab.format_graph.data_objects",
        "format_graph.mv.size_variable",
        "modal.ok",
    ] {
        h.click(wid)?;
    }
    let gs = h.app().document().graph_sheet(&SheetId::from("gr-1")).ok_or("gr-1")?;
    let enc = gs.spec.multivariate.as_ref().ok_or("mv enc after")?;
    assert_eq!(
        (gs.spec.kind, enc.plot_mode.clone(), enc.x_column, enc.y_column),
        (GraphKind::MultipleVariables, MultivariatePlotMode::BubbleScatter, 0, 1)
    );
    assert!(matches!(enc.color, ColorChannelMapping::Categorical { .. }) && matches!(enc.size, SizeChannelMapping::Variable { .. }));

    h.select_sheet(&SheetId::from("mv-1"))?;
    h.set_mv_role(2, "categorical")?;
    assert_eq!(mv_table(&h)?.variables[2].role, VariableRole::Categorical);
    h.driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(mv_table(&h)?.variables[2].role, VariableRole::Label);
    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn r10_8_w7_staged_i18n_16_locales_and_rtl_parity() -> TestRes {
    let mv_entries: Vec<_> = CATALOG.iter().filter(|e| e.key.starts_with("w7.mv.")).collect();
    assert!(!mv_entries.is_empty());
    for entry in mv_entries {
        assert!(!entry.key.trim().is_empty() && entry.translations.iter().all(|tr| !tr.trim().is_empty()));
    }
    for loc in LOCALES {
        let mut app = build_mv_cat_and_graph_app(DEFAULT_MV_COLS)?;
        assert_eq!(app.set_locale(loc), *loc);
        let mut h = EguiWfHarness::from_app(app, default_viewport());
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
