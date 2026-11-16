#![forbid(unsafe_code)]

mod human_wf_common;

use curvebook_doc::{
    Cell, ConstantValue, DataSheet, InfoSheet, LevelOrderKind, MultipleVariablesTable, ProjectDocument, SheetId, TableArchetype, VariableColumn,
    VariableRole, VariableSource,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::interaction::{KeyModifiers, UiEventOutcome, ViewportMetrics};
use curvebook_ui_egui::layout::WidgetId;

const MV_SHEET_ID: &str = "mv-main";

fn mv_cell_text(s: &str) -> Cell {
    Cell::text(s)
}

fn build_mv_app(locale: Option<&str>, frozen: bool, with_info: bool) -> Result<UiApp, String> {
    let mut mv = MultipleVariablesTable::new(4, 4).map_err(|e| format!("create mv table: {e}"))?;
    mv.variables[0] = VariableColumn {
        title: String::from("Dose"),
        role: VariableRole::Continuous,
        role_inferred: true,
        source: VariableSource::Entered,
        level_order: Default::default(),
        cells: vec![Cell::number(10.0), Cell::number(20.0), Cell::number(40.0), Cell::number(80.0)],
    };
    mv.variables[1] = VariableColumn {
        title: String::from("Arm"),
        role: VariableRole::Categorical,
        role_inferred: true,
        source: VariableSource::Entered,
        level_order: Default::default(),
        cells: vec![mv_cell_text("Placebo"), mv_cell_text("High"), mv_cell_text("Low"), mv_cell_text("Placebo")],
    };
    mv.variables[2] = VariableColumn {
        title: String::from("Notes"),
        role: VariableRole::Label,
        role_inferred: true,
        source: VariableSource::Entered,
        level_order: Default::default(),
        cells: vec![mv_cell_text("S1"), mv_cell_text("S2"), mv_cell_text("S3"), mv_cell_text("S4")],
    };
    mv.variables[3] = VariableColumn {
        title: String::from("Ratio"),
        role: VariableRole::Continuous,
        role_inferred: false,
        source: VariableSource::Entered,
        level_order: Default::default(),
        cells: vec![Cell::number(1.5), Cell::number(2.5), Cell::number(3.5), Cell::number(4.5)],
    };

    let mut ds = DataSheet::new(MV_SHEET_ID, "Cohorts", TableArchetype::MultipleVariables(mv)).map_err(|e| format!("create ds: {e}"))?;
    ds.frozen = frozen;

    let mut doc = ProjectDocument::new("R10.8 MV Formula Project").map_err(|e| format!("create doc: {e}"))?;
    if with_info {
        let mut info = InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| format!("create info: {e}"))?;
        info.set_constant("ScaleFactor", ConstantValue::Number(2.0), None).map_err(|e| format!("set info constant: {e}"))?;
        doc.add_info_sheet(info).map_err(|e| format!("add info sheet: {e}"))?;
    }
    doc.add_data_sheet(ds).map_err(|e| format!("add ds: {e}"))?;
    doc.select_in_navigator(&SheetId::from(MV_SHEET_ID)).map_err(|e| format!("select sheet: {e}"))?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    if let Some(loc) = locale {
        let _ = app.set_locale(loc);
    }
    Ok(app)
}

fn current_mv(app: &UiApp) -> Result<&MultipleVariablesTable, String> {
    let ds = app.document().data_sheet(&SheetId::from(MV_SHEET_ID)).ok_or_else(|| String::from("missing mv-main sheet"))?;
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => Ok(mv),
        other => Err(format!("expected MultipleVariables sheet, found {other:?}")),
    }
}

fn replace_textbox_text(driver: &mut curvebook_ui_egui::HumanPathDriver, id: &str, text: &str) -> Result<UiEventOutcome, String> {
    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from(id))?;
    let _ = human_wf_common::press_key_combo(driver, "a", KeyModifiers { ctrl: true, ..Default::default() })?;
    let _ = human_wf_common::press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    human_wf_common::type_into_focused(driver, text)
}

fn cell_text_or_num(cell: &Cell) -> String {
    if let Some(t) = cell.active_text() {
        return t.to_owned();
    }
    if let Some(n) = cell.active_number() {
        return format!("{n:.4}");
    }
    String::new()
}

#[test]
fn r10_8_role_badges_dropdown_lock_and_calculated_option() -> Result<(), String> {
    let app = build_mv_app(None, false, false)?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    assert!(driver.find_widget("mv_formula.role_badge.0").is_some(), "role badge 0 must render");
    assert!(driver.find_widget("mv_formula.role_lock.0").is_some(), "role lock 0 must render");
    assert!(driver.find_widget("mv_formula.role_badge.3").is_some(), "role badge 3 must render");

    // Lock auto-inferred column 0 via badge lock toggle.
    assert!(current_mv(driver.app())?.variables[0].role_inferred);
    let out_lock = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.role_lock.0"))?;
    assert!(out_lock.handled);
    assert!(!current_mv(driver.app())?.variables[0].role_inferred, "clicking auto lock button must lock role");

    // Clicking lock toggle again issues the `auto` role transition command cleanly.
    let out_unlock = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.role_lock.0"))?;
    assert!(out_unlock.handled);
    assert_eq!(out_unlock.command_id.as_deref(), Some("table.setVariableRole"));

    // Open column 0 role dropdown and switch it to Categorical via dropdown item.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.role_badge.0"))?;
    assert!(driver.find_widget("mv_formula.role_popup.0").is_some(), "role dropdown popup must open");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    let out_cat = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.role_opt.0.categorical"))?;
    assert!(out_cat.handled);
    assert!(driver.find_widget("mv_formula.role_popup.0").is_none(), "dropdown popup closes after selection");
    assert_eq!(current_mv(driver.app())?.variables[0].role, VariableRole::Categorical);
    assert!(!current_mv(driver.app())?.variables[0].role_inferred);

    // Selecting `Calculated` in column 3's dropdown opens the right-side Calculated Variable editor.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.role_badge.3"))?;
    let out_calc = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.role_opt.3.calculated"))?;
    assert!(out_calc.handled);
    assert!(driver.find_widget("mv_formula.panel").is_some(), "Calculated option must open right-side panel");
    assert_eq!(driver.app().grid.formula_bar.target_column, Some(3));
    assert!(driver.app().grid.formula_bar.active);
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_variables_drawer_level_recoding_and_ordering() -> Result<(), String> {
    let app = build_mv_app(None, false, false)?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // Open Variables... drawer via physical click.
    let out_open = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.variables_btn"))?;
    assert!(out_open.handled);
    assert!(driver.find_widget("mv_formula.vars_drawer").is_some(), "variables drawer must open");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // Switch to Categorical column 1 (`Arm`).
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.col.1"))?;
    assert_eq!(driver.app().grid.variables_dialog.selected_column, Some(1));

    // Click level 0 (`Placebo`) -> fills From box, type `Vehicle` into To box, stage, verify live preview, and apply.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.level.0"))?;
    let from_buf = driver.app().interaction.text_field_buffers.get("mv_formula.vars.recode_from.input").map(|b| b.text.clone()).unwrap_or_default();
    assert_eq!(from_buf, "Placebo");

    let _ = replace_textbox_text(&mut driver, "mv_formula.vars.recode_to.input", "Vehicle")?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.recode_stage"))?;
    assert_eq!(driver.app().grid.variables_dialog.recode_mappings.len(), 1);
    let preview_node = driver.find_widget("mv_formula.vars.recode_preview").ok_or_else(|| String::from("missing recode preview label"))?;
    assert!(preview_node.a11y.label.contains("2 cell(s)"), "expected live preview count of 2 cells, got `{}`", preview_node.a11y.label);

    let out_apply = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.recode_apply"))?;
    assert!(out_apply.handled);
    assert_eq!(out_apply.command_id.as_deref(), Some("table.recodeLevels"));
    let mv_after_recode = current_mv(driver.app())?;
    assert_eq!(cell_text_or_num(&mv_after_recode.variables[1].cells[0]), "Vehicle");
    assert_eq!(cell_text_or_num(&mv_after_recode.variables[1].cells[3]), "Vehicle");
    assert_eq!(cell_text_or_num(&mv_after_recode.variables[1].cells[1]), "High");

    // Level ordering controls: Alphabetical, Reverse toggle, Move Up, Set Reference level.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.order.alpha"))?;
    assert_eq!(current_mv(driver.app())?.variables[1].level_order.order, LevelOrderKind::Alphabetical);

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.order.reversed"))?;
    assert!(current_mv(driver.app())?.variables[1].level_order.reversed);

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.level_up.1"))?;
    assert!(matches!(current_mv(driver.app())?.variables[1].level_order.order, LevelOrderKind::Explicit(_)));

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.level_ref.2"))?;
    assert_eq!(
        driver.app().grid.variables_dialog.ordered_levels.first().map(|l| l.level.as_str()),
        Some(match &current_mv(driver.app())?.variables[1].level_order.order {
            LevelOrderKind::Explicit(v) => v.first().map_or("", String::as_str),
            _ => "",
        })
    );

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.vars.close"))?;
    assert!(driver.find_widget("mv_formula.vars_drawer").is_none(), "close button must hide drawer");
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_right_panel_new_calc_r2_s10_13_funcs_refs_validation_and_defs() -> Result<(), String> {
    let app = build_mv_app(None, false, true)?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // Open new calculated variable panel via `+ Calc` (`R2-S10-13` named target with `target_column == None`).
    let out_new = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.new_calc"))?;
    assert!(out_new.handled);
    assert!(driver.find_widget("mv_formula.panel").is_some(), "new_calc must open right panel");
    let commit_initial = driver.find_widget("formula_bar.commit").ok_or_else(|| String::from("missing commit button"))?;
    assert!(!commit_initial.state.enablement.enabled, "Apply must be disabled until name and valid formula exist");

    // Enter target variable name (`R2-S10-13`).
    let _ = replace_textbox_text(&mut driver, "mv_formula.name.input", "LogDose")?;

    // Search function reference for `log`, verify filtered list + parameter hints + insertion.
    let _ = replace_textbox_text(&mut driver, "mv_formula.func.search", "log")?;
    assert!(driver.find_widget("mv_formula.func.log10").is_some(), "log10 must match function search");
    assert!(driver.find_widget("mv_formula.func.sqrt").is_none(), "sqrt must be filtered out while searching log");
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.func.log10"))?;
    let hint_node = driver.find_widget("mv_formula.param_hint").ok_or_else(|| String::from("missing parameter hint banner"))?;
    assert!(hint_node.a11y.label.contains("log10(x)"), "parameter hint must show log10 signature, got `{}`", hint_node.a11y.label);

    // Click column 0 (`Dose`) to insert variable reference, close parens, verify Apply enables (`R2-S10-13`).
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.insert_col.0"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("formula_bar.input"))?;
    let _ = human_wf_common::type_into_focused(&mut driver, ")")?;

    let val_node_ok = driver.find_widget("mv_formula.validation_status").ok_or_else(|| String::from("missing validation banner"))?;
    assert_eq!(val_node_ok.a11y.label, "Formula valid");
    let commit_ready = driver.find_widget("formula_bar.commit").ok_or_else(|| String::from("missing commit button"))?;
    assert!(commit_ready.state.enablement.enabled, "Apply must enable once named target and valid formula are present");

    // Test invalid syntax disables Apply and updates live feedback banner.
    let _ = human_wf_common::type_into_focused(&mut driver, " +")?;
    let commit_bad = driver.find_widget("formula_bar.commit").ok_or_else(|| String::from("missing commit button"))?;
    assert!(!commit_bad.state.enablement.enabled, "syntax error must disable Apply");
    let val_node_err = driver.find_widget("mv_formula.validation_status").ok_or_else(|| String::from("missing validation banner"))?;
    assert_ne!(val_node_err.a11y.label, "Formula valid");

    // Clear function search and scroll function list to `zscore` (`scroll_until_widget_visible`), proving real scrollable reference.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.func.clear"))?;
    human_wf_common::scroll_until_widget_visible(
        &mut driver,
        &WidgetId::from("mv_formula.func_list"),
        &WidgetId::from("mv_formula.func.zscore"),
        60.0,
        24,
    )?;

    // Fix formula back to valid `= log10(Dose)` and click Apply to create col 4 (`LogDose`).
    let _ = replace_textbox_text(&mut driver, "formula_bar.input", "= log10(Dose)")?;
    let out_commit = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("formula_bar.commit"))?;
    assert!(out_commit.handled);
    let mv_one = current_mv(driver.app())?;
    assert_eq!(mv_one.variables.len(), 5);
    assert_eq!(mv_one.variables[4].title, "LogDose");
    assert_eq!(mv_one.variables[4].role, VariableRole::Calculated);
    assert!((mv_one.variables[4].cells[0].active_number().unwrap_or(0.0) - 1.0).abs() < 1e-6);

    // Create second calculated column 5 (`DoubleLog` = `LogDose * 2`) using `ScaleFactor` Info constant hook too.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.new_calc"))?;
    let _ = replace_textbox_text(&mut driver, "mv_formula.name.input", "DoubleLog")?;
    let _ = replace_textbox_text(&mut driver, "formula_bar.input", "= LogDose * 2")?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("formula_bar.commit"))?;
    assert_eq!(current_mv(driver.app())?.variables.len(), 6);

    // Re-open column 4 via column `fx` button and introduce a 2-hop cycle (`= DoubleLog + 1`): live circularity check must disable Apply.
    let active_sid = driver.app().document().active_sheet_id.as_ref().ok_or_else(|| String::from("missing active sheet"))?.as_str().to_owned();
    let grid_scroll_wid = WidgetId::from(format!("grid.sheet.{active_sid}"));
    for _ in 0..8 {
        if driver.find_widget("formula_bar.open_col.4").is_some_and(|n| n.drawn_in_frame && n.bounds.visible_rect_px.width > 0) {
            break;
        }
        let _ = human_wf_common::scroll_widget_wheel(&mut driver, &grid_scroll_wid, 180.0, 0.0)?;
    }
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("formula_bar.open_col.4"))?;
    assert!(driver.find_widget("mv_formula.panel").is_some());
    assert_eq!(driver.app().grid.formula_bar.target_column, Some(4));
    let _ = replace_textbox_text(&mut driver, "formula_bar.input", "= DoubleLog + 1")?;
    let cycle_status = driver.find_widget("mv_formula.validation_status").ok_or_else(|| String::from("missing validation status"))?;
    assert!(
        cycle_status.a11y.label.to_ascii_lowercase().contains("circul"),
        "expected circular dependency feedback, got `{}`",
        cycle_status.a11y.label
    );
    let commit_cycle = driver.find_widget("formula_bar.commit").ok_or_else(|| String::from("missing commit button"))?;
    assert!(!commit_cycle.state.enablement.enabled, "circular dependency must disable Apply");

    // Toggle Formula Definitions list inside the open panel, then Cancel closes active edit & panel.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.panel_defs_toggle"))?;
    assert!(driver.find_widget("mv_formula.defs_list").is_some(), "fx Defs toggle must show definitions list");
    assert!(driver.find_widget("mv_formula.def_item.4").is_some());
    assert!(driver.find_widget("mv_formula.def_item.5").is_some());

    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("formula_bar.cancel"))?;
    assert!(!driver.app().grid.formula_bar.active);
    assert!(driver.find_widget("mv_formula.panel").is_none(), "Cancel closes right-side editor panel");

    // Re-open right panel via `fx Editor` + open Variables drawer concurrently and verify zero parity regressions.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.panel_toggle"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("mv_formula.variables_btn"))?;
    assert!(driver.find_widget("mv_formula.panel").is_some());
    assert!(driver.find_widget("mv_formula.vars_drawer").is_some());

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    let _ = driver.render_frame().map_err(|e| e.to_string())?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_frozen_sheet_rtl_and_narrow_viewport_parity() -> Result<(), String> {
    // 1. Frozen sheet exposes explicit disabled reasons on mutating controls while allowing panel/drawer inspection.
    let frozen_app = build_mv_app(None, true, false)?;
    let mut frozen_driver = human_wf_common::new_human_driver(frozen_app, human_wf_common::default_viewport());

    let new_calc = frozen_driver.find_widget("mv_formula.new_calc").ok_or_else(|| String::from("missing new_calc"))?;
    assert!(!new_calc.state.enablement.enabled);
    assert!(new_calc.state.enablement.disabled_reason.as_deref().is_some_and(|r| r.contains("frozen")));
    let _ = frozen_driver.hover_widget("mv_formula.new_calc")?;

    let _ = human_wf_common::click_widget_center(&mut frozen_driver, &WidgetId::from("mv_formula.variables_btn"))?;
    let recode_apply = frozen_driver.find_widget("mv_formula.vars.recode_apply").ok_or_else(|| String::from("missing recode_apply"))?;
    assert!(!recode_apply.state.enablement.enabled);
    assert!(recode_apply.state.enablement.disabled_reason.as_deref().is_some_and(|r| r.contains("frozen")));
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut frozen_driver)?;

    // 2. RTL (`ar`) + narrow (`1024x640`) viewport passes parity with editor panel, definitions view, and role dropdown open.
    let rtl_app = build_mv_app(Some("ar"), false, false)?;
    let narrow = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let mut rtl_driver = human_wf_common::new_human_driver(rtl_app, narrow);

    let _ = human_wf_common::click_widget_center(&mut rtl_driver, &WidgetId::from("mv_formula.panel_toggle"))?;
    let _ = human_wf_common::click_widget_center(&mut rtl_driver, &WidgetId::from("mv_formula.defs_toggle"))?;
    let _ = human_wf_common::click_widget_center(&mut rtl_driver, &WidgetId::from("mv_formula.role_badge.1"))?;
    assert!(rtl_driver.find_widget("mv_formula.panel").is_some());
    assert!(rtl_driver.find_widget("mv_formula.defs_list").is_some());
    assert!(rtl_driver.find_widget("mv_formula.role_popup.1").is_some());
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut rtl_driver)?;
    let _ = rtl_driver.render_frame().map_err(|e| e.to_string())?;
    human_wf_common::assert_input_to_paint_budget(&rtl_driver, 50.0)?;
    Ok(())
}
