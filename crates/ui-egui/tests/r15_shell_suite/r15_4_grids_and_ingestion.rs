#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave 4 (`R15.4`) comprehensive UI/human-path integration suite for all 8 Data Grid archetypes,
//! inline editing, `Manage Variables…`, 5-stage `Import & Paste Special` wizard, reactive Transform
//! modals, responsive `14`-locale parity, and `1M`-cell `XY`/`MV` virtualization guard.

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, GroupedTable, InfoSheet,
    LevelOrderKind, MultipleVariablesTable, NestedTable, PartsOfWholeTable, ProjectDocument, SequenceProgression, SheetId, SubcolumnMode,
    SurvivalTable, TableArchetype, TableKind, VariableRole, XColumnMode, XyTable,
};
use curvebook_ui_egui::UiThemeMode;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, accesskit_accessible_name};
use curvebook_ui_egui::egui_shell::import_wizard::ImportWizardStep;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::egui_shell::{Tokens, UiIntent};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::theme::UiThemeSpec;
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

use crate::common::{ShellTestState, TestRes, assert_clean_parity, commit_shell_intents, fill_col_values, make_doc_driver, widget_center};

fn select_sheet(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, sheet_id: &str) -> TestRes {
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": sheet_id })).map_err(|e| e.to_string())?;
    driver.run_settled();
    Ok(())
}

fn replace_focused_text<S>(driver: &mut EguiHumanPathDriver<'_, S>, text: &str) -> TestRes {
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text(text)
}

fn double_click_and_commit(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, wid: &str, text: &str) -> TestRes {
    let pos = widget_center(driver, wid)?;
    driver.click_at(pos)?;
    driver.click_at(pos)?;
    replace_focused_text(driver, text)?;
    driver.press_key("Enter", KeyModifiers::default())
}

fn build_all_eight_archetypes_doc() -> ProjectDocument {
    let mut doc = ProjectDocument::new("R15.4 All 8 Archetypes").expect("project");

    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(2), 2, 4).expect("xy");
    xy.x_title = String::from("Conc (nM)");
    xy.data_sets[0].title = String::from("Wildtype");
    xy.data_sets[1].title = String::from("Mutant");
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r as f64 + 1.0) * 10.0);
        xy.data_sets[0].cells[r][0] = Cell::number(100.0 - r as f64 * 12.0);
        xy.data_sets[0].cells[r][1] = Cell::number(98.0 - r as f64 * 12.0);
        xy.data_sets[1].cells[r][0] = Cell::number(82.0 - r as f64 * 8.0);
        xy.data_sets[1].cells[r][1] = Cell::number(80.0 - r as f64 * 8.0);
    }
    doc.add_data_sheet(DataSheet::new("ds-xy", "XY Dose", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("col");
    fill_col_values(&mut col.data_sets[0], "Control", &[10.0, 11.0, 12.0, 13.0]);
    fill_col_values(&mut col.data_sets[1], "Low", &[14.0, 15.0, 16.0, 17.0]);
    fill_col_values(&mut col.data_sets[2], "High", &[19.0, 20.0, 21.0, 22.0]);
    doc.add_data_sheet(DataSheet::new("ds-col", "Column Assay", TableArchetype::Column(col)).unwrap()).unwrap();

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).expect("grouped");
    grp.row_titles = vec![String::from("Vehicle"), String::from("Agonist"), String::from("Antagonist")];
    grp.data_sets[0].title = String::from("Male");
    grp.data_sets[1].title = String::from("Female");
    for r in 0..3 {
        for c in 0..2 {
            grp.data_sets[c].cells[r][0] = Cell::number(20.0 + (r * 3 + c) as f64);
            grp.data_sets[c].cells[r][1] = Cell::number(21.5 + (r * 3 + c) as f64);
        }
    }
    doc.add_data_sheet(DataSheet::new("ds-grp", "Grouped Factorial", TableArchetype::Grouped(grp)).unwrap()).unwrap();

    let mut cont = ContingencyTable::new(2, 2).expect("cont");
    cont.row_titles = vec![String::from("Responder"), String::from("NonResponder")];
    let mut ds_cont = DataSheet::new("ds-cont", "Contingency 2x2", TableArchetype::Contingency(cont)).unwrap();
    for (r, c, v) in [(0, 0, 24.0), (0, 1, 12.0), (1, 0, 6.0), (1, 1, -3.0)] {
        ds_cont.set_cell(CellCoord::new(r, c, 0), Cell::number(v)).unwrap();
    }
    doc.add_data_sheet(ds_cont).unwrap();

    let mut surv = SurvivalTable::new(2, 4).expect("surv");
    surv.time_title = String::from("Days");
    for r in 0..4 {
        surv.times[r] = Cell::number((r as f64 + 1.0) * 7.0);
    }
    let mut ds_surv = DataSheet::new("ds-surv", "KM Survival", TableArchetype::Survival(surv)).unwrap();
    for r in 0..4 {
        ds_surv.set_cell(CellCoord::new(r, 0, 0), Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 })).unwrap();
        ds_surv.set_cell(CellCoord::new(r, 1, 0), Cell::number(1.0)).unwrap();
    }
    doc.add_data_sheet(ds_surv).unwrap();

    let mut pow = PartsOfWholeTable::new(1, 3).expect("pow");
    pow.slice_labels = vec![String::from("G1"), String::from("S"), String::from("G2/M")];
    let mut ds_pow = DataSheet::new("ds-pow", "Cell Cycle", TableArchetype::PartsOfWhole(pow)).unwrap();
    for (r, v) in [52.0, 31.0, 17.0].into_iter().enumerate() {
        ds_pow.set_cell(CellCoord::new(r, 0, 0), Cell::number(v)).unwrap();
    }
    doc.add_data_sheet(ds_pow).unwrap();

    let mut mv = MultipleVariablesTable::new(4, 4).expect("mv");
    for (idx, (title, role)) in [
        ("Dose (mg)", VariableRole::Continuous),
        ("Treatment", VariableRole::Categorical),
        ("PatientID", VariableRole::Label),
        ("LogDose", VariableRole::Continuous),
    ]
    .into_iter()
    .enumerate()
    {
        mv.variables[idx].title = title.to_owned();
        mv.variables[idx].role = role;
    }
    mv.variables[3].set_calculated_formula("ln([Dose (mg)])");
    for (r, trt) in ["Control", "Low", "High", "Low"].iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number((r as f64 + 1.0) * 10.0);
        mv.variables[1].cells[r] = Cell::text(*trt);
        mv.variables[2].cells[r] = Cell::text(format!("SUBJ-{r:02}"));
    }
    doc.add_data_sheet(DataSheet::new("ds-mv", "MV Study", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    let mut nest = NestedTable::new(2, 3, 4).expect("nested");
    nest.groups[0].title = String::from("Placebo");
    nest.groups[0].subcolumn_titles = vec![String::from("Mouse1"), String::from("Mouse2"), String::from("Mouse3")];
    nest.groups[1].title = String::from("Drug");
    nest.groups[1].subcolumn_titles.pop();
    for r in &mut nest.groups[1].cells {
        r.pop();
    }
    for r in 0..4 {
        for s in 0..3 {
            nest.groups[0].cells[r][s] = Cell::number(10.0 + r as f64 + s as f64 * 0.5);
        }
        for s in 0..2 {
            nest.groups[1].cells[r][s] = Cell::number(18.0 + r as f64 + s as f64 * 0.5);
        }
    }
    doc.add_data_sheet(DataSheet::new("ds-nest", "Nested Assay", TableArchetype::Nested(nest)).unwrap()).unwrap();

    let xy_seq = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 0.0, step: 2.5, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        1,
        4,
    )
    .expect("xy_seq");
    doc.add_data_sheet(DataSheet::new("ds-xy-seq", "XY Generated Seq", TableArchetype::Xy(xy_seq)).unwrap()).unwrap();

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Notes").unwrap();
    info.set_constant("VehicleMass", ConstantValue::Number(12.5), Some(String::from("mg"))).unwrap();
    doc.add_info_sheet(info).unwrap();

    doc.select_in_navigator(&SheetId::from("ds-xy")).unwrap();
    doc
}

#[test]
fn all_eight_archetypes_render_with_accesskit_cells_and_clean_parity() -> TestRes {
    let mut driver = make_doc_driver(build_all_eight_archetypes_doc(), 1280.0, 800.0);

    for sheet_id in ["ds-xy", "ds-col", "ds-grp", "ds-cont", "ds-surv", "ds-pow", "ds-mv", "ds-nest"] {
        select_sheet(&mut driver, sheet_id)?;
        assert_clean_parity(&driver, sheet_id)?;

        let ak_nodes = &driver.accesskit_tree().ok_or("missing AccessKit tree")?.nodes;
        let grid_cells: Vec<_> =
            driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("grid.cell.") || e.widget_id.starts_with("grid.xcell.")).collect();
        assert!(!grid_cells.is_empty(), "expected rendered cells for `{sheet_id}`");
        for entry in grid_cells {
            let (_, node) =
                ak_nodes.iter().find(|(id, _)| *id == entry.accesskit_id).ok_or_else(|| format!("missing AK node for `{}`", entry.widget_id))?;
            assert_eq!(node.role(), egui::accesskit::Role::Cell);
            assert!(!accesskit_accessible_name(node).unwrap_or("").trim().is_empty());
        }
    }

    select_sheet(&mut driver, "ds-nest")?;
    assert!(
        driver.frame_meta().find_by_widget_id("grid.subcol_title.0.2").is_some()
            && driver.frame_meta().find_by_widget_id("grid.subcol_title.1.2").is_none()
    );

    select_sheet(&mut driver, "ds-mv")?;
    for wid in ["mv_formula.role_badge.0", "mv_formula.role_badge.1", "mv_formula.role_badge.3"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing `{wid}`");
    }
    driver.click_widget_id("mv_formula.role_badge.1")?;
    driver.click_widget_id("mv_formula.role_opt.1.continuous")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!("MV")
    };
    assert_eq!(mv.variables[1].role, VariableRole::Continuous);

    driver.click_widget_id("mv_formula.role_badge.3")?;
    assert!(driver.state().app.grid.formula_bar.active);
    assert!(driver.frame_meta().find_by_widget_id("grid.cell.0.3.0").is_some_and(|e| !e.enabled));
    Ok(())
}

#[test]
fn inline_edits_exclusions_clipboard_and_undo_parity_on_xy_and_nested() -> TestRes {
    let mut driver = make_doc_driver(build_all_eight_archetypes_doc(), 1280.0, 800.0);

    let undo_before = driver.state().app.session().undo_depth();
    double_click_and_commit(&mut driver, "grid.xcell.0.0", "42.5")?;
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);
    let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else { panic!("Xy") };
    assert_eq!(xy.x_cells[0][0].active_number(), Some(42.5));

    driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else { panic!("Xy") };
    assert_eq!(xy.x_cells[0][0].active_number(), Some(10.0));

    select_sheet(&mut driver, "ds-xy-seq")?;
    assert!(driver.frame_meta().find_by_widget_id("grid.xcell.0.0").is_some_and(|e| !e.enabled));

    select_sheet(&mut driver, "ds-xy")?;
    for (wid, text) in [
        ("grid.x_title.0", "Dose (uM)"),
        ("grid.col_title.0", "WT Treated"),
        ("grid.row_title.0", "Replicate-Alpha"),
        ("grid.sheet_title", "XY Renamed Sheet"),
    ] {
        double_click_and_commit(&mut driver, wid, text)?;
    }
    let ds_xy = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    assert_eq!(ds_xy.title, "XY Renamed Sheet");
    let TableArchetype::Xy(xy) = &ds_xy.archetype else { panic!("Xy") };
    assert_eq!((xy.x_title.as_str(), xy.data_sets[0].title.as_str(), xy.row_titles[0].as_str()), ("Dose (uM)", "WT Treated", "Replicate-Alpha"));

    select_sheet(&mut driver, "ds-nest")?;
    double_click_and_commit(&mut driver, "grid.subcol_title.0.2", "BioUnit-Gamma")?;
    let TableArchetype::Nested(nest) = &driver.state().app.document().data_sheet(&SheetId::from("ds-nest")).unwrap().archetype else {
        panic!("Nested")
    };
    assert_eq!(nest.groups[0].subcolumn_titles[2], "BioUnit-Gamma");

    select_sheet(&mut driver, "ds-xy")?;
    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.press_egui_key(Key::E, Modifiers::COMMAND);
    let ds_xy = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    assert!(ds_xy.cell(CellCoord::new(0, 0, 0)).unwrap().excluded);

    let val_10 = ds_xy.cell(CellCoord::new(1, 0, 0)).unwrap().active_number();
    driver.click_widget_id("grid.cell.1.0.0")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    driver.click_widget_id("grid.cell.2.1.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().cell(CellCoord::new(2, 1, 0)).unwrap().active_number(),
        val_10
    );

    driver.click_widget_id("grid.corner.archetype")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }));
    Ok(())
}

#[test]
fn manage_variables_modal_master_detail_level_rename_merge_reorder_pin_and_backdrop() -> TestRes {
    let mut driver = make_doc_driver(build_all_eight_archetypes_doc(), 1280.0, 800.0);
    select_sheet(&mut driver, "ds-mv")?;

    driver.state_mut().intents.push(UiIntent::OpenManageVariables { sheet_id: SheetId::from("ds-mv"), column: Some(1) });
    let _ = commit_shell_intents(&mut driver);
    assert!(driver.frame_meta().find_by_widget_id("mv_formula.vars.search").is_some());
    assert_clean_parity(&driver, "ManageVariables")?;

    let before_active = driver.state().app.grid.tool.active;
    driver.click_at(Pos2::new(8.0, 760.0))?;
    assert_eq!(driver.state().app.grid.tool.active, before_active);

    driver.click_widget_id("mv_formula.vars.filter.calc")?;
    assert!(driver.frame_meta().find_by_widget_id("mv_formula.vars.col.3").is_some());
    driver.click_widget_id("mv_formula.vars.filter.all")?;
    driver.click_widget_id("mv_formula.vars.col.1")?;

    driver.click_widget_id("mv_formula.vars.unit_input")?;
    replace_focused_text(&mut driver, "arm")?;
    driver.click_widget_id("mv_formula.vars.meta.apply")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    assert_eq!(mv.variables[1].title, "Treatment (arm)");

    driver.click_widget_id("mv_formula.vars.level.1")?;
    driver.click_widget_id("mv_formula.vars.level_rename.1")?;
    replace_focused_text(&mut driver, "Medium")?;
    driver.click_widget_id("mv_formula.vars.level_merge.1")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    assert!(mv.variables[1].cells.iter().any(|c| c.active_text() == Some("Medium")));

    driver.click_widget_id("mv_formula.vars.level_rename.1")?;
    replace_focused_text(&mut driver, "High")?;
    driver.click_widget_id("mv_formula.vars.level_merge.1")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    let distinct: std::collections::BTreeSet<&str> = mv.variables[1].cells.iter().filter_map(|c| c.active_text()).collect();
    assert_eq!(distinct.len(), 2);

    driver.click_widget_id("mv_formula.vars.level_up.1")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    assert!(matches!(mv.variables[1].level_order.order, LevelOrderKind::Explicit(_)));

    driver.click_widget_id("mv_formula.vars.level_ref.1")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    let LevelOrderKind::Explicit(levels) = &mv.variables[1].level_order.order else { panic!("explicit order") };
    assert!(!levels.is_empty());

    driver.click_widget_id("modal.ok")?;
    assert!(!driver.state().app.grid.variables_dialog.open);
    Ok(())
}

#[test]
fn five_stage_import_wizard_reshape_and_convert_archetype_single_commit() -> TestRes {
    let mut driver = make_doc_driver(build_all_eight_archetypes_doc(), 1280.0, 800.0);

    driver.state_mut().intents.push(UiIntent::OpenImportWizard { origin: ImportOriginKind::Clipboard, is_live_link: false, target_sheet_id: None });
    let _ = commit_shell_intents(&mut driver);
    driver.click_widget_id("import_paste.input.csv")?;
    driver.state_mut().views.import_wizard.raw_source_draft = String::from("Dose,Control,Treated\n1,10*,15\n10,20,NA");
    driver.run_settled();
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::Source);
    assert_clean_parity(&driver, "ImportWizard")?;

    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::Preview);
    assert!(driver.frame_meta().find_by_widget_id("import_paste.preview.summary").is_some());

    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::FilterAndUnstack);
    assert!(driver.frame_meta().find_by_widget_id("import_paste.input.col_start").is_some_and(|e| e.enabled));
    driver.click_widget_id("import_paste.opt.unstack")?;
    assert!(driver.frame_meta().find_by_widget_id("import_paste.input.col_start").is_some_and(|e| !e.enabled));
    driver.click_widget_id("import_paste.opt.unstack")?;

    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::PlacementAndReshape);
    driver.click_widget_id("import_paste.reshape.plate96")?;
    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::Metadata);
    driver.click_widget_id("modal.back")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::PlacementAndReshape);
    driver.click_widget_id("import_paste.reshape.none")?;
    driver.click_widget_id("modal.next")?;

    let undo_before = driver.state().app.session().undo_depth();
    driver.click_widget_id("modal.ok")?;
    assert!(!driver.state().app.grid.import_wizard.open && driver.state().app.session().undo_depth() == undo_before + 1);

    driver.state_mut().intents.push_ui("ui.import.reshape.open", json!({ "mode": "plate_96" }));
    let _ = commit_shell_intents(&mut driver);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ImportReshape));
    let plate96_csv = (1..=96).map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
    driver.click_widget_id("import_paste.input.plate_values")?;
    replace_focused_text(&mut driver, &plate96_csv)?;
    driver.click_widget_id("import.subtract_blank_mean")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None, "ImportReshape error: {:?}", driver.state().last_intent_error);

    select_sheet(&mut driver, "ds-col")?;
    driver.click_widget_id("grid.corner.archetype")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }));
    for wid in ["convert.target.grouped", "convert.allow_lossy", "modal.ok"] {
        driver.click_widget_id(wid)?;
    }
    assert_eq!(driver.state().app.document().data_sheet(&SheetId::from("ds-col")).unwrap().kind(), TableKind::Grouped);
    Ok(())
}

#[test]
fn transform_baseline_mv_filter_extract_and_exclude_reason_modals() -> TestRes {
    let mut driver = make_doc_driver(build_all_eight_archetypes_doc(), 1280.0, 800.0);

    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("ds-xy"), kind: TransformModalKind::TransformValues });
    let _ = commit_shell_intents(&mut driver);
    for wid in [
        "edit.transform.mode.pharmacology",
        "edit.transform.mode.standard",
        "edit.transform.x_transform.log10",
        "edit.transform.y_transform.divide_k",
        "edit.transform.hook_k",
        "edit.transform.hook_item.0",
    ] {
        driver.click_widget_id(wid)?;
    }
    assert_eq!(driver.state().views.transform_modals.constant_k_draft, "12.5");
    driver.click_widget_id("modal.ok")?;

    let transform_rs_id = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(_)))
        .map(|rs| rs.id.as_str().to_owned())
        .ok_or("missing Transform ResultSheet")?;
    select_sheet(&mut driver, &transform_rs_id)?;
    driver.state_mut().intents.push_ui("ui.editParams.open", json!({}));
    let _ = commit_shell_intents(&mut driver);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    driver.click_widget_id("edit.transform.y_transform.multiply_k")?;
    driver.click_widget_id("modal.ok")?;

    select_sheet(&mut driver, "ds-xy")?;
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("ds-xy"), kind: TransformModalKind::RemoveBaseline });
    let _ = commit_shell_intents(&mut driver);
    driver.click_widget_id("edit.transform.baseline_operation.ratio")?;
    driver.click_widget_id("modal.ok")?;

    select_sheet(&mut driver, "ds-mv")?;
    driver
        .state_mut()
        .intents
        .push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("ds-mv"), kind: TransformModalKind::MvFilterAndTransform });
    let _ = commit_shell_intents(&mut driver);
    driver.click_widget_id("edit.transform.formula")?;
    replace_focused_text(&mut driver, "ln([")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| !e.enabled));
    driver.click_widget_id("edit.transform.formula")?;
    replace_focused_text(&mut driver, "ln([Dose (mg)])")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| e.enabled));
    driver.click_widget_id("modal.ok")?;

    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("ds-mv"), kind: TransformModalKind::ExtractAndRearrange });
    let _ = commit_shell_intents(&mut driver);
    driver.click_widget_id("edit.transform.target_archetype.xy")?;
    driver.click_widget_id("modal.ok")?;

    select_sheet(&mut driver, "ds-xy")?;
    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.state_mut().intents.push_ui("ui.grid.excludeWithReason", json!({ "sheet_id": "ds-xy" }));
    let _ = commit_shell_intents(&mut driver);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }));
    driver.click_widget_id("exclude.preset.0")?;
    driver.click_widget_id("exclude.note")?;
    replace_focused_text(&mut driver, "Pipette bubble verified")?;
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().excluded);
    Ok(())
}

#[test]
fn responsive_viewports_themes_and_all_14_beta_locales() -> TestRes {
    let mut driver = make_doc_driver(build_all_eight_archetypes_doc(), 1440.0, 600.0);

    for (w, h) in [(640.0, 420.0), (1440.0, 600.0)] {
        driver.set_viewport(ViewportMetrics::new(w, h, 1.0));
        for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
            driver.state_mut().app.theme = UiThemeSpec::for_mode(theme);
            let mut toks = Tokens::from_theme_spec(&driver.state().app.theme, driver.tokens().locale);
            toks.ui_scale = 1.0;
            driver.set_tokens(toks);
            assert_clean_parity(&driver, &format!("viewport ({w}x{h}) {theme:?}"))?;
        }
    }

    driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
    driver.state_mut().views.trailing_collapsed_rail = true;
    select_sheet(&mut driver, "ds-mv")?;
    driver.state_mut().intents.push(UiIntent::OpenManageVariables { sheet_id: SheetId::from("ds-mv"), column: Some(1) });
    let _ = commit_shell_intents(&mut driver);

    for (w, h) in [(640.0, 420.0), (1440.0, 600.0)] {
        driver.set_viewport(ViewportMetrics::new(w, h, 1.0));
        for locale in BETA_LOCALE_CODES {
            driver.state_mut().app.locale = String::from(locale);
            driver.set_tokens(Tokens::from_theme_spec(&driver.state().app.theme, locale));
            assert_clean_parity(&driver, &format!("ManageVariables ({w}x{h}) `{locale}`"))?;
        }
    }
    Ok(())
}

#[test]
fn one_million_cell_xy_and_mv_virtualization_regression_guard() -> TestRes {
    let mut doc = ProjectDocument::new("1M Cell Guard").unwrap();
    doc.add_data_sheet(
        DataSheet::new("mv-1m", "MV 1M", TableArchetype::MultipleVariables(MultipleVariablesTable::new(1_000, 1_000).unwrap())).unwrap(),
    )
    .unwrap();
    doc.add_data_sheet(
        DataSheet::new(
            "xy-1m",
            "XY 1M",
            TableArchetype::Xy(
                XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 999, 1_000).unwrap(),
            ),
        )
        .unwrap(),
    )
    .unwrap();
    doc.select_in_navigator(&SheetId::from("mv-1m")).unwrap();
    let mut driver = make_doc_driver(doc, 1280.0, 800.0);

    for sheet_id in ["mv-1m", "xy-1m"] {
        select_sheet(&mut driver, sheet_id)?;
        let vis_cells = driver.state().views.column_grid.last_visible_rows.len() * driver.state().views.column_grid.last_visible_subcols.len();
        assert!(vis_cells > 0 && vis_cells < 1_500);
        let meta_cells =
            driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("grid.cell.") || e.widget_id.starts_with("grid.xcell.")).count();
        assert!(meta_cells > 0 && meta_cells < 1_500);

        #[cfg(not(debug_assertions))]
        {
            driver.state_mut().views.column_grid.record_inspect_meta = false;
            let mut samples = Vec::with_capacity(12);
            for _ in 0..12 {
                let t0 = std::time::Instant::now();
                driver.run_settled();
                samples.push(t0.elapsed().as_secs_f64() * 1000.0);
            }
            driver.state_mut().views.column_grid.record_inspect_meta = true;
            samples.sort_by(|a, b| a.total_cmp(b));
            assert!(samples[samples.len() / 2] <= 16.67, "1M-cell `{sheet_id}` p50 exceeded 16.67 ms");
        }
    }
    Ok(())
}
