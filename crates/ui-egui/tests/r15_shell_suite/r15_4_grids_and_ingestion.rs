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

#[test]
fn aud_07_08_10_import_paste_wizard_hardening_and_transform_modals() -> TestRes {
    use crate::helpers::seed_column_app;
    use curvebook_doc::{AnalysisSpec, CiBound, DescriptiveSpec, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind};
    use curvebook_engine::Session;
    use curvebook_ui_egui::UiApp;
    use curvebook_ui_egui::egui_shell::PendingUiIntents;
    use curvebook_ui_egui::import_paste::commit_import_paste_dialog;

    // 1. AUD-07-02/03/04/06/07/08: internal extraction (>200 rows, Survival, Nested, ResultTab CI precision), asterisk subcol atomic undo, bounds, blanks, predicates.
    let mut doc = ProjectDocument::new("Ext").unwrap();
    let mut big = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 240).unwrap();
    for r in 0..240 {
        big.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    doc.data_sheets.push(DataSheet::new("big-col", "Big", TableArchetype::Column(big)).unwrap());
    let mut surv = SurvivalTable::new(2, 3).unwrap();
    for r in 0..3 {
        surv.times[r] = Cell::number(((r + 1) * 10) as f64);
        surv.cohorts[0].cells[r][0] = Cell::number(1.0);
    }
    doc.data_sheets.push(DataSheet::new("surv-1", "Surv", TableArchetype::Survival(surv)).unwrap());
    let mut nst = NestedTable::new(2, 2, 2).unwrap();
    nst.groups[0].cells[0][0] = Cell::number(11.0);
    doc.data_sheets.push(DataSheet::new("nest-1", "Nest", TableArchetype::Nested(nst)).unwrap());
    let mut rs =
        ResultSheet::new(AnalysisSpec::new("res-1", "big-col", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap()).unwrap();
    rs.status = ResultStatus::UpToDate;
    rs.tabs = vec![
        ResultTab {
            id: "t0".into(),
            title: "T0".into(),
            kind: ResultTabKind::MainSummary,
            column_headers: vec!["A".into()],
            row_labels: vec!["R".into()],
            rows: vec![vec![ResultCell::Number { value: 1.0, prefix: None, units: None }]],
            footnotes: vec![],
        },
        ResultTab {
            id: "t1".into(),
            title: "T1".into(),
            kind: ResultTabKind::MainSummary,
            column_headers: vec!["CI".into()],
            row_labels: vec!["P".into()],
            rows: vec![vec![ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.00012345), upper: CiBound::Finite(0.00098765) }]],
            footnotes: vec![],
        },
    ];
    doc.result_sheets.push(rs);
    doc.active_sheet_id = Some(SheetId::new("big-col"));
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.grid.import_paste.source_origin = "internal_sheet".into();
    app.grid.import_paste.target_mode = "new_sheet".into();
    for (src, kind, check_tab) in [
        ("big-col", TableKind::Column, false),
        ("surv-1", TableKind::Survival, false),
        ("nest-1", TableKind::Column, false),
        ("res-1", TableKind::Column, true),
    ] {
        if check_tab {
            (app.canvas_dialogs.result_view.result_sheet_id, app.canvas_dialogs.result_view.active_tab_index) = (Some(SheetId::new("res-1")), 1);
        }
        app.grid.import_paste.internal_source_sheet_id = Some(src.into());
        app.grid.import_reshape.target_kind = kind;
        let out = commit_import_paste_dialog(&mut app).unwrap();
        let ds = app.document().data_sheet(&SheetId::new(out["sheet_id"].as_str().unwrap())).unwrap();
        match src {
            "big-col" => assert_eq!(ds.archetype.row_count(), 240),
            "surv-1" => {
                assert!(matches!(&ds.archetype, TableArchetype::Survival(s) if s.times[0].active_number() == Some(10.0) && s.cohorts.len() == 2))
            }
            "nest-1" => assert_eq!(ds.archetype.column_count(), 4),
            _ => assert!(
                matches!(ds.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(curvebook_doc::CellValue::Text(t)) if t.contains("0.00012345") && t.contains("0.00098765"))
            ),
        }
    }

    // Asterisk subcol restore + atomic undo + PartsOfWhole/Nested wire names + OOM bounds + blanks + exclude_predicate
    let mut gdoc = ProjectDocument::new("Grp").unwrap();
    gdoc.data_sheets
        .push(DataSheet::new("grp-1", "G", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap())).unwrap());
    gdoc.active_sheet_id = Some(SheetId::new("grp-1"));
    let mut gapp = UiApp::from_session(Session::from_document(gdoc));
    let u0 = gapp.session().undo_depth();
    gapp.grid.import_paste.source_origin = "clipboard".into();
    gapp.grid.import_paste.raw_source_text = "A1\tA2\tB1\tB2\n1\t*litA2\t3\t*litB2".into();
    gapp.grid.import_paste.has_header_row = true;
    gapp.grid.import_paste.parse_asterisk_exclusions = false;
    gapp.grid.import_paste.target_mode = "existing_sheet".into();
    gapp.grid.import_paste.target_sheet_id = Some("grp-1".into());
    gapp.grid.import_reshape.tidy_delimiter = '\t';
    commit_import_paste_dialog(&mut gapp).unwrap();
    let gds = gapp.document().data_sheet(&SheetId::new("grp-1")).unwrap();
    assert_eq!(gapp.session().undo_depth(), u0 + 1);
    for col in [0, 1] {
        let c = gds.cell(CellCoord::new(0, col, 1)).unwrap();
        assert!(!c.excluded && matches!(&c.value, curvebook_doc::CellValue::Text(s) if s.starts_with("*lit")));
    }
    for kind in [TableKind::PartsOfWhole, TableKind::Nested] {
        gapp.grid.import_paste.target_mode = "new_sheet".into();
        gapp.grid.import_paste.parse_asterisk_exclusions = true;
        gapp.grid.import_paste.raw_source_text = "G1\tG2\n10\t20\n30\t40".into();
        gapp.grid.import_reshape.target_kind = kind;
        let sid = commit_import_paste_dialog(&mut gapp).unwrap()["sheet_id"].as_str().unwrap().to_owned();
        assert_eq!(gapp.document().data_sheet(&SheetId::new(sid)).unwrap().kind(), kind);
    }

    let (mut bapp, bsid) = seed_column_app(&[("A", &[1.0, 2.0]), ("B", &[3.0, 4.0])], ColumnDesignHint::UnpairedReplicates);
    bapp.grid.import_paste.source_origin = "clipboard".into();
    bapp.grid.import_paste.target_mode = "new_sheet".into();
    bapp.grid.import_reshape.tidy_delimiter = '\t';
    bapp.grid.import_paste.raw_source_text = "A\tB\n1\t2".into();
    bapp.grid.import_paste.row_wrap_width = curvebook_doc::MAX_DATA_SETS + 1;
    assert!(commit_import_paste_dialog(&mut bapp).is_err());
    bapp.grid.import_paste.row_wrap_width = 0;
    let mut huge = String::from("Group\tVal\n");
    for i in 0..=(curvebook_doc::MAX_DATA_SETS + 2) {
        huge.push_str(&format!("G{i}\t{i}\n"));
    }
    (bapp.grid.import_paste.raw_source_text, bapp.grid.import_paste.unstack_enabled, bapp.grid.import_paste.unstack_group_col) = (huge, true, 0);
    assert!(commit_import_paste_dialog(&mut bapp).is_err());
    bapp.grid.import_paste.unstack_enabled = false;
    (
        bapp.grid.import_paste.raw_source_text,
        bapp.grid.import_paste.skip_contiguous_blanks,
        bapp.grid.import_paste.target_mode,
        bapp.grid.import_paste.target_sheet_id,
    ) = ("Val\n10\n\n\n20\n\n30".into(), true, "existing_sheet".into(), Some(bsid.as_str().into()));
    commit_import_paste_dialog(&mut bapp).unwrap();
    let dsb = bapp.document().data_sheet(&bsid).unwrap();
    assert_eq!(
        (
            dsb.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number),
            dsb.cell(CellCoord::new(1, 0, 0)).is_some_and(Cell::is_empty),
            dsb.cell(CellCoord::new(2, 0, 0)).and_then(Cell::active_number),
            dsb.cell(CellCoord::new(4, 0, 0)).and_then(Cell::active_number)
        ),
        (Some(10.0), true, Some(20.0), Some(30.0))
    );
    (
        bapp.grid.import_paste.skip_contiguous_blanks,
        bapp.grid.import_paste.target_mode,
        bapp.grid.import_paste.raw_source_text,
        bapp.grid.import_paste.exclude_predicate,
    ) = (false, "new_sheet".into(), "Dose\tResp\n1\t100\n10\t200".into(), "Dose < 5".into());
    let psid = commit_import_paste_dialog(&mut bapp).unwrap()["sheet_id"].as_str().unwrap().to_owned();
    let dsp = bapp.document().data_sheet(&SheetId::new(psid)).unwrap();
    assert!(dsp.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded) && dsp.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| !c.excluded));
    for bad in ["NonExistent > 0", "not a predicate"] {
        bapp.grid.import_paste.exclude_predicate = bad.into();
        assert!(commit_import_paste_dialog(&mut bapp).is_err());
    }

    // AUD-08-12: Failed CommitImportWizard rolls back created sheet.
    let n_before = bapp.document().data_sheets.len();
    (bapp.grid.import_wizard.origin_kind, bapp.grid.import_wizard.target_sheet_id, bapp.grid.import_wizard.raw_clipboard_text) =
        (ImportOriginKind::Clipboard, None, String::new());
    let mut pw = PendingUiIntents::new();
    pw.push(UiIntent::CommitImportWizard { close_after: true });
    let _ = pw.commit_if_settled(&mut bapp, false);
    assert_eq!(bapp.document().data_sheets.len(), n_before);
    Ok(())
}

#[test]
fn aud_10_import_wizard_policy_atomicity_unstack_and_transform_modals() -> TestRes {
    use crate::common::temp_dir;
    use crate::helpers::{make_commit_driver, seed_column_app, seed_xy_app};
    use curvebook_doc::{ErrorPropagationMode, MathTransformPreset, SimulationNoise, TransformAnalysisSpec};
    use curvebook_engine::{ExternalPathPolicy, Session};
    use curvebook_ui_egui::UiApp;
    use serde_json::Value;

    // 2. AUD-10-03/05/20/21 & AUD-13-02: ImportWizard UI policy, atomic empty/frozen InfoSheet, unstack cap & 1-row header, XY replicate subcols.
    let mut app_rej = UiApp::from_session(Session::new().with_path_policy(ExternalPathPolicy::RejectExternal));
    (app_rej.grid.import_wizard.open, app_rej.grid.import_wizard.origin_kind, app_rej.grid.import_wizard.external_file_path) =
        (true, ImportOriginKind::XlsxWorkbook, "/etc/passwd".into());
    let dr_rej = make_commit_driver(app_rej);
    for wid in ["import_paste.input.path", "import_wizard.xlsx.load_path", "import_wizard.xlsx.select_all", "import_wizard.xlsx.select_none"] {
        assert!(!dr_rej.frame_meta().find_by_widget_id(wid).unwrap().enabled);
    }
    let wdir = temp_dir("cb-aud10-20")?;
    let mut app_wd = UiApp::from_session(Session::new().with_path_policy(ExternalPathPolicy::workdir(wdir.path()).unwrap()));
    (app_wd.grid.import_wizard.open, app_wd.grid.import_wizard.origin_kind, app_wd.grid.import_wizard.external_file_path) =
        (true, ImportOriginKind::XlsxWorkbook, "../out.xlsx".into());
    let mut dr_wd = make_commit_driver(app_wd);
    dr_wd.click_widget_id("import_wizard.xlsx.load_path")?;
    assert!(dr_wd.state().app.grid.import_wizard.xlsx_error.as_deref().is_some_and(|e| e.contains("workdir policy")));

    let mut app_emp = UiApp::new();
    (
        app_emp.grid.import_wizard.open,
        app_emp.grid.import_wizard.origin_kind,
        app_emp.grid.import_wizard.delimiter,
        app_emp.grid.import_wizard.raw_clipboard_text,
    ) = (true, ImportOriginKind::Clipboard, ',', "Instrument: PlateReader\nA,B\n1,2\n3,4".into());
    let mut dr_emp = make_commit_driver(app_emp);
    for wid in ["import_paste.tab.notes", "import_paste.opt.extract_info_metadata", "import_paste.opt.add_floating_note", "modal.ok"] {
        dr_emp.click_widget_id(wid)?;
    }
    assert!(!dr_emp.state().app.grid.import_wizard.open && dr_emp.state().app.session().undo_depth() == 1);
    let info = dr_emp.state().app.document().info_sheets.first().unwrap();
    assert!(info.constants.iter().any(|c| c.name == "Instrument") && info.notes_markdown.contains("[Import Summary]"));

    let mut fdoc = ProjectDocument::new("Frozen").unwrap();
    let mut finfo = InfoSheet::new_preseeded("info-f", "I").unwrap();
    finfo.constants.clear();
    finfo.notes_markdown.clear();
    fdoc.info_sheets.push(finfo);
    let mut fds =
        DataSheet::new("col-f", "F", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap())).unwrap();
    fds.info_sheet_id = Some(SheetId::new("info-f"));
    fdoc.data_sheets.push(fds);
    fdoc.active_sheet_id = Some(SheetId::new("col-f"));
    let mut app_f = UiApp::from_session(Session::from_document(fdoc));
    app_f.execute_command("sheet.freeze", json!({ "sheet_id": "col-f" })).unwrap();
    (app_f.grid.import_wizard.open, app_f.grid.import_wizard.target_sheet_id, app_f.grid.import_wizard.raw_clipboard_text) =
        (true, Some(SheetId::new("col-f")), "Batch: B99\n10\t20".into());
    let mut dr_f = make_commit_driver(app_f);
    for wid in ["import_paste.tab.notes", "import_paste.opt.extract_info_metadata", "import_paste.opt.add_floating_note", "modal.ok"] {
        dr_f.click_widget_id(wid)?;
    }
    assert!(
        dr_f.state().app.grid.import_wizard.open && dr_f.state().app.document().info_sheet(&SheetId::new("info-f")).unwrap().constants.is_empty()
    );

    let mut big_u = String::from("Grp,Val\n");
    for i in 0..250 {
        big_u.push_str(&format!("G{i},{i}\n"));
    }
    for j in 0..250 {
        big_u.push_str(&format!("G0,{j}\n"));
    }
    let mut app_u = UiApp::new();
    (
        app_u.grid.import_wizard.open,
        app_u.grid.import_wizard.delimiter,
        app_u.grid.import_wizard.header_rows,
        app_u.grid.import_wizard.filter.unstack_group_column,
        app_u.grid.import_wizard.raw_clipboard_text,
    ) = (true, ',', 1, Some(0), big_u);
    let mut dr_u = make_commit_driver(app_u);
    dr_u.click_widget_id("import_paste.tab.filter")?;
    assert!(dr_u.state().views.import_wizard.unstack_error.is_some() && !dr_u.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);

    let mut app_u1 = UiApp::new();
    (
        app_u1.grid.import_wizard.open,
        app_u1.grid.import_wizard.delimiter,
        app_u1.grid.import_wizard.header_rows,
        app_u1.grid.import_wizard.filter.unstack_group_column,
        app_u1.grid.import_wizard.raw_clipboard_text,
    ) = (true, ',', 1, Some(0), "Grp,Val".into());
    let mut dr_u1 = make_commit_driver(app_u1);
    for wid in ["import_paste.tab.filter", "modal.ok"] {
        dr_u1.click_widget_id(wid)?;
    }
    let dsu1 = dr_u1.state().app.document().data_sheets.first().unwrap();
    assert!(dsu1.archetype.data_cell(CellCoord::new(0, 0, 0)).is_none_or(Cell::is_empty));

    let mut xy2 = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
    (xy2.x_cells[0][0], xy2.data_sets[0].cells[0][0], xy2.data_sets[0].cells[0][1]) = (Cell::number(1.0), Cell::number(10.0), Cell::number(11.0));
    let mut doc_xy = ProjectDocument::new("XYR").unwrap();
    doc_xy.data_sheets.push(DataSheet::new("xy-s", "S", TableArchetype::Xy(xy2)).unwrap());
    doc_xy
        .data_sheets
        .push(DataSheet::new("col-d", "D", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2).unwrap())).unwrap());
    let mut app_xy = UiApp::from_session(Session::from_document(doc_xy));
    (
        app_xy.grid.import_wizard.open,
        app_xy.grid.import_wizard.origin_kind,
        app_xy.grid.import_wizard.source_sheet_id,
        app_xy.grid.import_wizard.target_sheet_id,
    ) = (true, ImportOriginKind::InternalDataSheet, Some(SheetId::new("xy-s")), Some(SheetId::new("col-d")));
    let mut dr_xy = make_commit_driver(app_xy);
    dr_xy.click_widget_id("modal.ok")?;
    let dxy = dr_xy.state().app.document().data_sheet(&SheetId::new("col-d")).unwrap();
    assert_eq!(
        (0..3).map(|c| dxy.archetype.data_cell(CellCoord::new(0, c, 0)).and_then(Cell::active_number)).collect::<Vec<_>>(),
        vec![Some(1.0), Some(10.0), Some(11.0)]
    );

    // 3. AUD-10-14/16/45: LinearScale slope preservation, German Simulate semicolon format, RemoveBaseline cycle, ExcludeWithReason preset.
    let (mut app_lin, sid_lin) = seed_xy_app(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);
    let lin_k = AnalysisKind::Transform(TransformAnalysisSpec::Math {
        x_transform: None,
        y_transform: Some(MathTransformPreset::LinearScale { slope: 2.5, intercept: 10.0 }),
        error_propagation: ErrorPropagationMode::ReplicateEndpoints,
        pharmacology_preset: None,
        interchange_xy: false,
        zero_x_substitute: None,
        asymmetric_ci_policy: Default::default(),
    });
    let rsid = SheetId::new(
        app_lin
            .execute_command("analysis.run", json!({ "source_sheet_id": sid_lin.as_str(), "kind": serde_json::to_value(&lin_k).unwrap() }))
            .unwrap()
            .payload["sheet_id"]
            .as_str()
            .unwrap(),
    );
    app_lin.canvas_dialogs.open_edit_parameters(&app_lin.document().clone(), &rsid);
    let mut dr_lin = make_commit_driver(app_lin);
    dr_lin.click_widget_id("modal.ok")?;
    assert!(
        matches!(&dr_lin.state().app.document().result_sheet(&rsid).unwrap().analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform: Some(MathTransformPreset::LinearScale { slope, intercept }), .. }) if (*slope - 2.5).abs() < 1e-12 && (*intercept - 10.0).abs() < 1e-12)
    );

    let (mut app_de, sid_de) = seed_xy_app(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]);
    app_de.set_locale("de");
    let sim_k = AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
        seed: 42,
        row_count: 10,
        replicates: 2,
        x_min: -9.0,
        x_max: -3.0,
        equation_id: "log_agonist_vs_response_4pl".into(),
        parameters: vec![("Top".into(), 100.5), ("HillSlope".into(), 1.5)],
        noise: SimulationNoise::None,
        simulation_mode: None,
    });
    let sim_id = SheetId::new(
        app_de
            .execute_command("analysis.run", json!({ "source_sheet_id": sid_de.as_str(), "kind": serde_json::to_value(&sim_k).unwrap() }))
            .unwrap()
            .payload["sheet_id"]
            .as_str()
            .unwrap(),
    );
    app_de.canvas_dialogs.open_edit_parameters(&app_de.document().clone(), &sim_id);
    let dr_de = make_commit_driver(app_de);
    assert!(
        dr_de.state().views.transform_modals.sim_params_draft.contains(';')
            && dr_de.state().views.transform_modals.validation_error.is_none()
            && dr_de.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled
    );

    let (mut app_rb, sid_rb) = seed_column_app(&[("Ctrl", &[1.0, 2.0]), ("Tx", &[3.0, 4.0])], ColumnDesignHint::UnpairedReplicates);
    (app_rb.egui_views.transform_modals.open_kind, app_rb.egui_views.transform_modals.target_sheet_id) =
        (Some(TransformModalKind::RemoveBaseline), Some(sid_rb.clone()));
    let mut dr_rb = make_commit_driver(app_rb);
    let op0 = dr_rb.state().views.transform_modals.baseline_op;
    dr_rb.click_widget_id("edit.transform.baseline_operation")?;
    assert_eq!(dr_rb.state().views.transform_modals.baseline_op, (op0 + 1) % 4);

    let (mut app_ex, sid_ex) = seed_column_app(&[("Ctrl", &[1.0, 2.0])], ColumnDesignHint::UnpairedReplicates);
    (app_ex.canvas_dialogs.exclude_with_reason.sheet_id, app_ex.canvas_dialogs.exclude_with_reason.coords) =
        (Some(sid_ex.clone()), vec![CellCoord::new(0, 0, 0).into()]);
    app_ex.canvas_dialogs.exclude_with_reason.reason_tag.clear();
    app_ex.canvas_dialogs.active_dialog = ActiveDialog::ExcludeWithReason { sheet_id: sid_ex };
    let mut dr_ex = make_commit_driver(app_ex);
    assert!(!dr_ex.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    dr_ex.click_widget_id("exclude.preset.0")?;
    assert!(
        !dr_ex.state().views.transform_modals.exclude_reason_draft.is_empty() && dr_ex.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled
    );
    let _ = Value::Null;
    Ok(())
}

#[test]
fn r10_32_paste_transpose_via_edit_menu_and_grid_context_menu_with_undo() -> TestRes {
    use std::sync::Arc;

    use crate::common::make_shell_driver;
    use curvebook_doc::CellValue;
    use curvebook_engine::Session;
    use curvebook_ui_egui::UiApp;
    use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;

    let mut doc = ProjectDocument::new("R10.32 Paste Transpose").expect("project");
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 5).expect("col");
    doc.add_data_sheet(DataSheet::new("ds-pt", "Paste Transpose Target", TableArchetype::Column(col)).unwrap()).unwrap();
    doc.select_in_navigator(&SheetId::from("ds-pt")).unwrap();

    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.host_draws_menu_bar = false;
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;
    app.set_clipboard_adapter(clip.clone());

    let mut driver = make_shell_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    let cmd_mod = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };

    // 1. Top menu `Edit › Paste Transpose` pastes 2x3 TSV transposed as 3x2 at anchor (row=1, col=1).
    clip.seed_text("1\t2\t3\n4\t5\t6");
    driver.click_widget_id("grid.cell.1.1.0")?;
    driver.click_widget_id("menu.edit")?;
    let pt_menu = driver.frame_meta().find_by_widget_id("menu.table.pasteTranspose").ok_or("missing menu.table.pasteTranspose")?;
    assert!(pt_menu.enabled, "Edit › Paste Transpose should be enabled on unfrozen DataSheet");
    driver.click_widget_id("menu.table.pasteTranspose")?;

    let expected_menu_vals = [(1, 1, 1.0), (1, 2, 4.0), (2, 1, 2.0), (2, 2, 5.0), (3, 1, 3.0), (3, 2, 6.0)];
    {
        let ds = driver.state().app.document().data_sheet(&SheetId::from("ds-pt")).ok_or("ds-pt")?;
        for (r, c, val) in expected_menu_vals {
            assert_eq!(ds.cell(CellCoord::new(r, c, 0)).and_then(Cell::active_number), Some(val), "Edit › Paste Transpose mismatch at ({r},{c})");
        }
        assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&CellValue::Empty));
        assert_eq!(ds.cell(CellCoord::new(1, 3, 0)).map(|c| &c.value), Some(&CellValue::Empty));
    }

    // Undo cleanly with Cmd+Z.
    driver.press_egui_key(Key::Z, cmd_mod);
    {
        let ds = driver.state().app.document().data_sheet(&SheetId::from("ds-pt")).ok_or("ds-pt")?;
        for (r, c, _) in expected_menu_vals {
            assert_eq!(
                ds.cell(CellCoord::new(r, c, 0)).map(|cell| &cell.value),
                Some(&CellValue::Empty),
                "Cmd+Z should revert pasted cell ({r},{c})"
            );
        }
    }

    // 2. Grid right-click context menu `grid.ctx.paste_transpose` pastes 2x3 TSV transposed as 3x2 at (0, 1).
    clip.seed_text("10\t20\t30\n40\t50\t60");
    let cell_01 = widget_center(&driver, "grid.cell.0.1.0")?;
    driver.click_at_button(cell_01, curvebook_ui_egui::PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("popup.grid.ctx").is_some(), "expected row/cell context menu");
    let pt_ctx = driver.frame_meta().find_by_widget_id("grid.ctx.paste_transpose").ok_or("missing grid.ctx.paste_transpose")?;
    assert!(pt_ctx.enabled, "grid.ctx.paste_transpose should be enabled on unfrozen DataSheet");
    assert_eq!(pt_ctx.command_id.as_deref(), Some("table.pasteTranspose"));
    driver.click_widget_id("grid.ctx.paste_transpose")?;

    let expected_ctx_vals = [(0, 1, 10.0), (0, 2, 40.0), (1, 1, 20.0), (1, 2, 50.0), (2, 1, 30.0), (2, 2, 60.0)];
    {
        let ds = driver.state().app.document().data_sheet(&SheetId::from("ds-pt")).ok_or("ds-pt")?;
        for (r, c, val) in expected_ctx_vals {
            assert_eq!(ds.cell(CellCoord::new(r, c, 0)).and_then(Cell::active_number), Some(val), "grid.ctx.paste_transpose mismatch at ({r},{c})");
        }
    }

    // Undo cleanly with Cmd+Z.
    driver.press_egui_key(Key::Z, cmd_mod);
    {
        let ds = driver.state().app.document().data_sheet(&SheetId::from("ds-pt")).ok_or("ds-pt")?;
        for (r, c, _) in expected_ctx_vals {
            assert_eq!(
                ds.cell(CellCoord::new(r, c, 0)).map(|cell| &cell.value),
                Some(&CellValue::Empty),
                "Cmd+Z should revert context-menu pasted cell ({r},{c})"
            );
        }
    }

    // 3. Frozen sheet disables both `grid.ctx.paste_transpose` and `Edit › Paste Transpose`.
    driver.state_mut().app.execute_command("sheet.freeze", json!({ "sheet_id": "ds-pt" })).map_err(|e| e.to_string())?;
    driver.run_settled();
    let cell_01_frozen = widget_center(&driver, "grid.cell.0.1.0")?;
    driver.click_at_button(cell_01_frozen, curvebook_ui_egui::PointerButton::Secondary, KeyModifiers::default())?;
    let pt_ctx_frozen = driver.frame_meta().find_by_widget_id("grid.ctx.paste_transpose").ok_or("missing frozen grid.ctx.paste_transpose")?;
    assert!(!pt_ctx_frozen.enabled && pt_ctx_frozen.disabled_reason.is_some());
    driver.press_egui_key(Key::Escape, Modifiers::NONE);

    driver.click_widget_id("menu.edit")?;
    let pt_menu_frozen = driver.frame_meta().find_by_widget_id("menu.table.pasteTranspose").ok_or("missing frozen menu.table.pasteTranspose")?;
    assert!(!pt_menu_frozen.enabled);
    Ok(())
}
