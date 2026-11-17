#![forbid(unsafe_code)]

mod human_wf_common;

use curvebook_doc::{
    Cell, CellCoord, ColorChannelMapping, DataSheet, GraphKind, GraphSpec, LevelOrderKind, MultipleVariablesTable, MultivariatePlotMode,
    ProjectDocument, SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype, VariableColumn, VariableRole, VariableSource, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::i18n::{LOCALES, w7_mv_ui};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::WidgetId;
use serde_json::json;

fn wid(s: &str) -> WidgetId {
    WidgetId(String::from(s))
}

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

fn replace_textbox(driver: &mut curvebook_ui_egui::HumanPathDriver, id: &str, text: &str) -> Result<(), String> {
    let _ = human_wf_common::click_widget_center(driver, &wid(id))?;
    let _ = human_wf_common::press_key_combo(driver, "a", KeyModifiers { ctrl: true, ..Default::default() })?;
    let _ = human_wf_common::press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    let _ = human_wf_common::type_into_focused(driver, text)?;
    Ok(())
}

fn focus_widget_via_tab_dir(driver: &mut curvebook_ui_egui::HumanPathDriver, target_id: &str, shift: bool, max_steps: usize) -> Result<(), String> {
    let mods = KeyModifiers { shift, ..Default::default() };
    for _ in 0..max_steps {
        if driver.app().focus_widget_id.as_deref() == Some(target_id) {
            return Ok(());
        }
        let _ = driver.press_key("Tab", mods)?;
    }
    Err(format!("could not reach `{target_id}` within {max_steps} Tab steps (current focus: {:?})", driver.app().focus_widget_id))
}

fn focus_widget_via_tab(driver: &mut curvebook_ui_egui::HumanPathDriver, target_id: &str, max_steps: usize) -> Result<(), String> {
    focus_widget_via_tab_dir(driver, target_id, false, max_steps)
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

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // Click cell (0,0,0), extend to range with Shift+ArrowDown, press Delete & Backspace.
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("grid.cell.0.0.0"))?;
    let _ = driver.press_key("ArrowDown", KeyModifiers { shift: true, ..Default::default() })?;
    let del_out = driver.press_key("Delete", KeyModifiers::default())?;
    assert!(del_out.handled, "Delete on frozen grid must be handled");
    assert_eq!(del_out.action, "grid_core.frozen_read_only");
    assert!(del_out.command_id.is_none(), "Delete on frozen grid must emit no mutation command");

    let bs_out = driver.press_key("Backspace", KeyModifiers::default())?;
    assert!(bs_out.handled, "Backspace on frozen grid must be handled");
    assert_eq!(bs_out.action, "grid_core.frozen_read_only");
    assert!(bs_out.command_id.is_none(), "Backspace on frozen grid must emit no mutation command");

    // Verify typing & double-click inline editing (data cell + X cell) are also blocked cleanly.
    let type_out = driver.type_text("999")?;
    assert!(type_out.handled);
    assert_eq!(type_out.action, "grid_core.frozen_read_only");
    assert!(!driver.app().inline_editor.active);

    let [lx_cell, ly_cell] = human_wf_common::widget_hit_center(&mut driver, &wid("grid.cell.0.0.0"))?;
    let dbl_data = driver.double_click_logical(lx_cell, ly_cell)?;
    assert!(dbl_data.handled);
    assert_eq!(dbl_data.action, "grid_core.frozen_read_only");
    assert!(!driver.app().inline_editor.active, "double-click on frozen data cell must not open inline editor");

    let [lx_x, ly_x] = human_wf_common::widget_hit_center(&mut driver, &wid("grid.xcell.0.0"))?;
    let dbl_x = driver.double_click_logical(lx_x, ly_x)?;
    assert!(dbl_x.handled);
    assert_eq!(dbl_x.action, "grid_core.frozen_read_only");
    assert!(!driver.app().inline_editor.active, "double-click on frozen X cell must not open inline editor");

    let ds_after = driver.app().document().data_sheet(&SheetId::from("ds-frozen")).ok_or("missing ds-frozen")?;
    assert_eq!(ds_after.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(10.0));
    assert_eq!(ds_after.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(20.0));
    assert_eq!(ds_after.x_or_time_cell(0, 0).and_then(|c| c.active_number()), Some(1.0));

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_b4_keyboard_stepwise_explicit_level_reorder_and_format_graph_mirror() -> Result<(), String> {
    let app = build_mv_cat_and_graph_app([
        ("SampleID", VariableRole::Label),
        ("Arm", VariableRole::Categorical),
        ("Score", VariableRole::Continuous),
        ("Size", VariableRole::Continuous),
    ])?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // 1. Open Variables... drawer, select Arm (col 1), operate Explicit Move Up/Down/Top/End purely via keyboard.
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("mv_formula.variables_btn"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("mv_formula.vars.col.1"))?;

    // Select level 2 ("DrugB") via Tab + Space and verify stepwise keyboard controls follow it.
    driver.app_mut().focus_widget_id = Some(String::from("mv_formula.vars.level.1"));
    focus_widget_via_tab(&mut driver, "mv_formula.vars.level.2", 16)?;
    let _ = driver.press_key("Space", KeyModifiers::default())?;
    assert_eq!(driver.app().view_shards.mv_formula.selected_level_name.as_deref(), Some("DrugB"));

    driver.app_mut().focus_widget_id = Some(String::from("mv_formula.vars.order.apply"));
    focus_widget_via_tab(&mut driver, "mv_formula.vars.move_up", 32)?;
    let up_out = driver.press_key("Space", KeyModifiers::default())?;
    assert_eq!(up_out.command_id.as_deref(), Some("table.setCategoricalOrder"));
    {
        let TableArchetype::MultipleVariables(mv) = &driver.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(
            mv.variables[1].level_order.order,
            LevelOrderKind::Explicit(vec![String::from("Ctrl"), String::from("DrugB"), String::from("DrugA")])
        );
    }

    focus_widget_via_tab_dir(&mut driver, "mv_formula.vars.move_top", true, 4)?;
    let _ = driver.press_key("Enter", KeyModifiers::default())?;
    {
        let TableArchetype::MultipleVariables(mv) = &driver.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(
            mv.variables[1].level_order.order,
            LevelOrderKind::Explicit(vec![String::from("DrugB"), String::from("Ctrl"), String::from("DrugA")])
        );
    }

    focus_widget_via_tab(&mut driver, "mv_formula.vars.move_end", 6)?;
    let _ = driver.press_key("Space", KeyModifiers::default())?;
    {
        let TableArchetype::MultipleVariables(mv) = &driver.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(
            mv.variables[1].level_order.order,
            LevelOrderKind::Explicit(vec![String::from("Ctrl"), String::from("DrugA"), String::from("DrugB")])
        );
    }

    // Select first item ("Ctrl") and step Move down once -> ["DrugA", "Ctrl", "DrugB"].
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("mv_formula.vars.level.0"))?;
    driver.app_mut().focus_widget_id = Some(String::from("mv_formula.vars.order.apply"));
    focus_widget_via_tab(&mut driver, "mv_formula.vars.move_down", 32)?;
    let _ = driver.press_key("Space", KeyModifiers::default())?;
    {
        let TableArchetype::MultipleVariables(mv) = &driver.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(
            mv.variables[1].level_order.order,
            LevelOrderKind::Explicit(vec![String::from("DrugA"), String::from("Ctrl"), String::from("DrugB")])
        );
    }

    // 2. Switch to GraphSheet and mirror Explicit level reordering in Inspector > Format Graph via keyboard.
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("nav.sheet.gr-1"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.tab.graph"))?;
    human_wf_common::scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.fg.level.2"), 120.0, 16)?;

    driver.app_mut().focus_widget_id = Some(String::from("inspector.fg.level.1"));
    focus_widget_via_tab(&mut driver, "inspector.fg.level.2", 32)?;
    let _ = driver.press_key("Space", KeyModifiers::default())?; // Select "DrugB" at index 2

    driver.app_mut().focus_widget_id = Some(String::from("inspector.cvd.grayscale"));
    focus_widget_via_tab(&mut driver, "inspector.fg.level_top", 32)?;
    let fg_top_out = driver.press_key("Space", KeyModifiers::default())?;
    assert_eq!(fg_top_out.command_id.as_deref(), Some("graph.setSpec"));
    let enc_fg = driver
        .app()
        .document()
        .graph_sheet(&SheetId::from("gr-1"))
        .and_then(|g| g.spec.multivariate.as_ref())
        .ok_or("missing mv encoding on gr-1")?;
    assert_eq!(
        enc_fg.x_level_order_override.as_ref().map(|s| &s.order),
        Some(&LevelOrderKind::Explicit(vec![String::from("DrugB"), String::from("DrugA"), String::from("Ctrl")]))
    );

    // Cmd+Z reverts Inspector Format Graph level override cleanly.
    let _ = human_wf_common::press_cmd_shortcut(&mut driver, "z")?;
    let enc_undo = driver
        .app()
        .document()
        .graph_sheet(&SheetId::from("gr-1"))
        .and_then(|g| g.spec.multivariate.as_ref())
        .ok_or("missing mv encoding after undo")?;
    assert!(enc_undo.x_level_order_override.is_none(), "Cmd+Z must revert x_level_order_override");

    // 3. Mirror Explicit level reordering in FormatGraph modal (`ActiveDialog::FormatGraph`) via Tab + Space + Enter.
    let _ = human_wf_common::press_cmd_shortcut(&mut driver, "k")?;
    let _ = human_wf_common::click_and_type(&mut driver, &wid("palette.query"), "ui.graph.format.open")?;
    let _ = driver.press_key("Enter", KeyModifiers::default())?;
    assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. }));

    // Tab to `format_graph.selected_level`, cycle "DrugA" -> "Ctrl", Tab to `format_graph.move_to_top`, press Space, then Enter.
    driver.app_mut().focus_widget_id = None;
    focus_widget_via_tab(&mut driver, "format_graph.selected_level", 20)?;
    let _ = driver.press_key("Space", KeyModifiers::default())?; // Ctrl
    focus_widget_via_tab(&mut driver, "format_graph.move_to_top", 6)?;
    let _ = driver.press_key("Space", KeyModifiers::default())?;
    let _ = driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::None);

    let enc_modal = driver
        .app()
        .document()
        .graph_sheet(&SheetId::from("gr-1"))
        .and_then(|g| g.spec.multivariate.as_ref())
        .ok_or("missing mv encoding after FormatGraph modal")?;
    assert_eq!(
        enc_modal.x_level_order_override.as_ref().map(|s| &s.order),
        Some(&LevelOrderKind::Explicit(vec![String::from("Ctrl"), String::from("DrugA"), String::from("DrugB")]))
    );

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
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
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // Open + Calc right-side editor panel.
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("mv_formula.new_calc"))?;
    replace_textbox(&mut driver, "mv_formula.name.input", "DerivedScore")?;
    replace_textbox(&mut driver, "formula_bar.input", "= Score +")?;

    // Mid-typing: syntax error must stay silent; Apply remains enabled so confirmation can validate.
    let mid_status = driver.find_widget("mv_formula.validation_status").ok_or("missing validation_status")?;
    assert_eq!(mid_status.a11y.label, "Validation runs on Apply or Enter.");
    let apply_btn = driver.find_widget("formula_bar.commit").ok_or("missing Apply")?;
    assert!(apply_btn.state.enablement.enabled);

    // Confirm with Apply -> surfaces syntax error and leaves columns untouched.
    let bad_apply = human_wf_common::click_widget_center(&mut driver, &wid("formula_bar.commit"))?;
    assert!(bad_apply.handled);
    let err_status = driver.find_widget("mv_formula.validation_status").ok_or("missing err status")?;
    assert_ne!(err_status.a11y.label, "Validation runs on Apply or Enter.");

    // Editing formula clears stale error mid-typing, then Enter commits cleanly.
    replace_textbox(&mut driver, "formula_bar.input", "= Score * 2")?;
    let cleared_status = driver.find_widget("mv_formula.validation_status").ok_or("missing cleared status")?;
    assert_eq!(cleared_status.a11y.label, "Validation runs on Apply or Enter.");
    let ok_enter = driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(ok_enter.action, "mv_formula.committed");

    // Add dependent column DoubleDerived = DerivedScore + 1, then trigger 2-hop circularity via Enter confirmation.
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("mv_formula.new_calc"))?;
    replace_textbox(&mut driver, "mv_formula.name.input", "DoubleDerived")?;
    replace_textbox(&mut driver, "formula_bar.input", "= DerivedScore + 1")?;
    let _ = driver.press_key("Enter", KeyModifiers::default())?;

    for _ in 0..10 {
        if driver.find_widget("formula_bar.open_col.4").is_some_and(|n| n.drawn_in_frame && n.bounds.visible_rect_px.width > 0) {
            break;
        }
        let _ = human_wf_common::scroll_widget_wheel(&mut driver, &wid("grid.sheet.mv-1"), 180.0, 0.0)?;
    }
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("formula_bar.open_col.4"))?;
    replace_textbox(&mut driver, "formula_bar.input", "= DoubleDerived + 5")?;
    let mid_cycle = driver.find_widget("mv_formula.validation_status").ok_or("missing mid cycle")?;
    assert!(!mid_cycle.a11y.label.to_ascii_lowercase().contains("circul"), "cycle error must not show mid-typing");

    let cyc_enter = driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(cyc_enter.action, "mv_formula.validation_error");
    let post_cycle = driver.find_widget("mv_formula.validation_status").ok_or("missing post cycle")?;
    assert!(
        post_cycle.a11y.label.to_ascii_lowercase().contains("circul"),
        "Enter confirmation must surface cycle error, got `{}`",
        post_cycle.a11y.label
    );

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
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

    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("nav.sheet.gr-1"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.tab.multi"))?;

    // Switch to Bubble mode, configure X, Y, Colour, Size, and Grouping/Label directly in Inspector without opening FormatGraph.
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.mv.mode_bubble"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.mv.x_btn"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.popup.mv_x.item_0"))?; // Score
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.mv.y_btn"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.popup.mv_y.item_1"))?; // Size
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.mv.color_btn"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.popup.mv_color.item_4"))?; // Arm (item_0=None, 1=0, 2=1, 3=2, 4=3)
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.mv.size_btn"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.popup.mv_size.item_2"))?; // Size col 1

    human_wf_common::scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.mv.group_btn"), 120.0, 12)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.mv.group_btn"))?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.popup.mv_group.item_3"))?; // SampleID col 2

    // Change a channel's variable definition directly in Inspector (`Group` col 2 Label -> Categorical) and undo via Cmd+Z.
    human_wf_common::scroll_until_widget_visible(&mut driver, &wid("scroll.inspector"), &wid("inspector.mv.def_group_btn"), 120.0, 16)?;
    let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.mv.def_group_btn"))?;
    let role_out = human_wf_common::click_widget_center(&mut driver, &wid("inspector.popup.mv_def_group.item_1"))?; // Categorical
    assert_eq!(role_out.command_id.as_deref(), Some("table.setVariableRole"));

    let gs_after = driver.app().document().graph_sheet(&SheetId::from("gr-1")).ok_or("gr-1")?;
    let enc_after = gs_after.spec.multivariate.as_ref().ok_or("mv enc after")?;
    assert_eq!(enc_after.plot_mode, MultivariatePlotMode::BubbleScatter);
    assert_eq!(enc_after.x_column, 0);
    assert_eq!(enc_after.y_column, 1);
    assert!(matches!(enc_after.color, ColorChannelMapping::Categorical { column: 3, .. }));
    assert!(matches!(enc_after.size, SizeChannelMapping::Variable { column: 1, .. }));
    assert_eq!(enc_after.label_column, Some(2));
    assert_eq!(gs_after.spec.kind, GraphKind::MultipleVariables);

    let TableArchetype::MultipleVariables(mv_role_after) = &driver.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype
    else {
        return Err(String::from("expected MV"));
    };
    assert_eq!(mv_role_after.variables[2].role, VariableRole::Categorical);

    // Undo role change -> reverts Group column back to Label.
    let _ = human_wf_common::press_cmd_shortcut(&mut driver, "z")?;
    let TableArchetype::MultipleVariables(mv_role_undo) = &driver.app().document().data_sheet(&SheetId::from("mv-1")).ok_or("mv-1")?.archetype else {
        return Err(String::from("expected MV"));
    };
    assert_eq!(mv_role_undo.variables[2].role, VariableRole::Label);

    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_8_w7_staged_i18n_16_locales_and_rtl_parity() -> Result<(), String> {
    assert!(!w7_mv_ui::STAGED_ENTRIES.is_empty());
    for entry in w7_mv_ui::STAGED_ENTRIES {
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
        let _ = app.set_locale(loc);
        let mut driver = human_wf_common::new_human_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
        let _ = human_wf_common::click_widget_center(&mut driver, &wid("mv_formula.variables_btn"))?;
        let parity = driver.verify_parity();
        assert!(parity.is_clean(), "parity failed for `{loc}`: {:?}", parity.violations);
        let _ = human_wf_common::click_widget_center(&mut driver, &wid("nav.sheet.gr-1"))?;
        let _ = human_wf_common::click_widget_center(&mut driver, &wid("inspector.tab.multi"))?;
        let parity_insp = driver.verify_parity();
        assert!(parity_insp.is_clean(), "inspector parity failed for `{loc}`: {:?}", parity_insp.violations);
    }
    Ok(())
}
