#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave 4 (`R15.4`) comprehensive UI/human-path integration suite (`crates/ui-egui/tests/r15_4_grids_and_ingestion.rs`):
//! - All 8 virtualized Data Grid archetypes (`Column`, `XY`, `Grouped`, `Contingency`, `Survival`,
//!   `PartsOfWhole`, `MultipleVariables` Tier-3 role/FX= chips, `Nested` biological-unit spans).
//! - Inline editing (`X` cells, `X` title, Column/Subcolumn titles, Row titles, Sheet title),
//!   corner archetype trigger, `Cmd/Ctrl+E` exclusions, clipboard `Cmd/Ctrl+C/X/V`, single-commit
//!   `PendingUiIntents` undo/`ops.journal` parity, and `GeneratedSequence` read-only guard.
//! - `Manage Variables…` master-detail modal (backdrop capture, filters, metadata edit,
//!   inline level rename & merge, explicit order / pointer drag / up-down / pin ref).
//! - 5-stage `Import & Paste Special` wizard + `ImportReshape` + `ConvertArchetype`.
//! - Reactive Transform modals (`TransformValues` w/ `Hook Constant`, `RemoveBaseline`,
//!   `MvFilterAndTransform` AST validation, `ExtractAndRearrange`, `EditAnalysisParameters`,
//!   `ExcludeWithReason`).
//! - Responsive `640×420` & `1440×600`, light/dark/high-contrast themes, all 14 `BETA_LOCALE_CODES`,
//!   `Role::Cell` + non-empty AccessKit names, scrollbar non-overlap, and `1M`-cell `XY`/`MV`
//!   virtualization guard.

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, GroupedTable, InfoSheet,
    LevelOrderKind, MultipleVariablesTable, NestedTable, PartsOfWholeTable, ProjectDocument, SequenceProgression, SheetId, SubcolumnMode,
    SurvivalTable, TableArchetype, TableKind, VariableRole, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{
    BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, accesskit_accessible_name, build_driver_font_definitions,
};
use curvebook_ui_egui::egui_shell::import_wizard::ImportWizardStep;
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::theme::UiThemeSpec;
use curvebook_ui_egui::{UiApp, UiThemeMode};
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

struct ShellDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    trailing_rect_pt: Option<egui::Rect>,
    footer_rect_pt: Option<egui::Rect>,
    leading_rect_pt: Option<egui::Rect>,
    last_intent_error: Option<String>,
}

impl ShellDriverState {
    fn new(mut app: UiApp) -> Self {
        app.set_gui_only(true);
        Self {
            app,
            views: EguiShellViewState::default(),
            intents: PendingUiIntents::default(),
            trailing_rect_pt: None,
            footer_rect_pt: None,
            leading_rect_pt: None,
            last_intent_error: None,
        }
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
            let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
            st.trailing_rect_pt = rep.trailing_dock_rect_pt;
            st.footer_rect_pt = rep.footer_rect_pt;
            st.leading_rect_pt = rep.leading_dock_rect_pt;
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_intent_error = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn commit_intents(driver: &mut EguiHumanPathDriver<'_, ShellDriverState>) {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
        st.last_intent_error = Some(err.to_string());
    }
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

fn select_sheet(driver: &mut EguiHumanPathDriver<'_, ShellDriverState>, sheet_id: &str) -> Result<(), String> {
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": sheet_id })).map_err(|e| e.to_string())?;
    driver.run_settled();
    Ok(())
}

fn widget_center(driver: &EguiHumanPathDriver<'_, ShellDriverState>, widget_id: &str) -> Result<Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    Ok(entry.clipped_rect.center())
}

fn replace_focused_text<S>(driver: &mut EguiHumanPathDriver<'_, S>, text: &str) -> Result<(), String> {
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text(text)?;
    Ok(())
}

fn replace_focused_text_and_commit<S>(driver: &mut EguiHumanPathDriver<'_, S>, text: &str) -> Result<(), String> {
    replace_focused_text(driver, text)?;
    driver.press_key("Enter", KeyModifiers::default())?;
    Ok(())
}

fn build_all_eight_archetypes_doc() -> ProjectDocument {
    let mut doc = ProjectDocument::new("R15.4 All 8 Archetypes").expect("project");

    // 1. XY (Numbers + RawReplicates(2))
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

    // 2. Column
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("col");
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Low");
    col.data_sets[2].title = String::from("High");
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        col.data_sets[1].cells[r][0] = Cell::number(14.0 + r as f64);
        col.data_sets[2].cells[r][0] = Cell::number(19.0 + r as f64);
    }
    doc.add_data_sheet(DataSheet::new("ds-col", "Column Assay", TableArchetype::Column(col)).unwrap()).unwrap();

    // 3. Grouped
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

    // 4. Contingency (including a negative count highlighting check)
    let mut cont = ContingencyTable::new(2, 2).expect("cont");
    cont.row_titles = vec![String::from("Responder"), String::from("NonResponder")];
    let mut ds_cont = DataSheet::new("ds-cont", "Contingency 2x2", TableArchetype::Contingency(cont)).unwrap();
    ds_cont.set_cell(CellCoord::new(0, 0, 0), Cell::number(24.0)).unwrap();
    ds_cont.set_cell(CellCoord::new(0, 1, 0), Cell::number(12.0)).unwrap();
    ds_cont.set_cell(CellCoord::new(1, 0, 0), Cell::number(6.0)).unwrap();
    ds_cont.set_cell(CellCoord::new(1, 1, 0), Cell::number(-3.0)).unwrap();
    doc.add_data_sheet(ds_cont).unwrap();

    // 5. Survival
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

    // 6. PartsOfWhole
    let mut pow = PartsOfWholeTable::new(1, 3).expect("pow");
    pow.slice_labels = vec![String::from("G1"), String::from("S"), String::from("G2/M")];
    let mut ds_pow = DataSheet::new("ds-pow", "Cell Cycle", TableArchetype::PartsOfWhole(pow)).unwrap();
    ds_pow.set_cell(CellCoord::new(0, 0, 0), Cell::number(52.0)).unwrap();
    ds_pow.set_cell(CellCoord::new(1, 0, 0), Cell::number(31.0)).unwrap();
    ds_pow.set_cell(CellCoord::new(2, 0, 0), Cell::number(17.0)).unwrap();
    doc.add_data_sheet(ds_pow).unwrap();

    // 7. MultipleVariables
    let mut mv = MultipleVariablesTable::new(4, 4).expect("mv");
    mv.variables[0].title = String::from("Dose (mg)");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("Treatment");
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[2].title = String::from("PatientID");
    mv.variables[2].role = VariableRole::Label;
    mv.variables[3].title = String::from("LogDose");
    mv.variables[3].set_calculated_formula("ln([Dose (mg)])");
    let treatments = ["Control", "Low", "High", "Low"];
    for (r, trt) in treatments.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number((r as f64 + 1.0) * 10.0);
        mv.variables[1].cells[r] = Cell::text(*trt);
        mv.variables[2].cells[r] = Cell::text(format!("SUBJ-{r:02}"));
    }
    doc.add_data_sheet(DataSheet::new("ds-mv", "MV Study", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    // 8. Nested with unequal biological unit counts (Group 0 = 3 bio units, Group 1 = 2 bio units)
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

    // Read-only GeneratedSequence XY sheet
    let xy_seq = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 0.0, step: 2.5, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        1,
        4,
    )
    .expect("xy_seq");
    doc.add_data_sheet(DataSheet::new("ds-xy-seq", "XY Generated Seq", TableArchetype::Xy(xy_seq)).unwrap()).unwrap();

    // InfoSheet with hookable Constant
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Notes").unwrap();
    info.set_constant("VehicleMass", ConstantValue::Number(12.5), Some(String::from("mg"))).unwrap();
    doc.add_info_sheet(info).unwrap();

    doc.select_in_navigator(&SheetId::from("ds-xy")).unwrap();
    doc
}

#[test]
fn all_eight_archetypes_render_with_accesskit_cells_and_clean_parity() -> Result<(), String> {
    let doc = build_all_eight_archetypes_doc();
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    for sheet_id in ["ds-xy", "ds-col", "ds-grp", "ds-cont", "ds-surv", "ds-pow", "ds-mv", "ds-nest"] {
        select_sheet(&mut driver, sheet_id)?;

        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "archetype `{sheet_id}` parity violations: {:?}", parity.violations);
        assert!(driver.check_scrollbar_non_overlap().is_empty(), "scrollbar overlap on `{sheet_id}`");

        // Every visible gridcell in FrameInspectMeta must correspond to an AccessKit `Role::Cell`
        // node with a non-empty accessible name.
        let ak_nodes = &driver.accesskit_tree().ok_or("missing AccessKit tree")?.nodes;
        let grid_cells: Vec<_> =
            driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("grid.cell.") || e.widget_id.starts_with("grid.xcell.")).collect();
        assert!(!grid_cells.is_empty(), "expected rendered cells for `{sheet_id}`");
        for entry in grid_cells {
            let (_, node) = ak_nodes
                .iter()
                .find(|(id, _)| *id == entry.accesskit_id)
                .ok_or_else(|| format!("missing AccessKit node for `{}` on `{sheet_id}`", entry.widget_id))?;
            assert_eq!(node.role(), egui::accesskit::Role::Cell, "widget `{}` wrong role", entry.widget_id);
            let name = accesskit_accessible_name(node).unwrap_or("");
            assert!(!name.trim().is_empty(), "empty AccessKit accessible name for `{}` on `{sheet_id}`", entry.widget_id);
        }
    }

    // Nested unequal biological-unit check: Group 0 has 3 subcols, Group 1 has 2 subcols.
    select_sheet(&mut driver, "ds-nest")?;
    assert!(driver.frame_meta().find_by_widget_id("grid.subcol_title.0.2").is_some());
    assert!(driver.frame_meta().find_by_widget_id("grid.subcol_title.1.2").is_none());

    // MultipleVariables Tier-3 chips (`Auto` / `NUM` / `CAT` / `LBL` / `FX=`): role menu switch + FX= guard.
    select_sheet(&mut driver, "ds-mv")?;
    assert!(driver.frame_meta().find_by_widget_id("mv_formula.role_badge.0").is_some());
    assert!(driver.frame_meta().find_by_widget_id("mv_formula.role_badge.1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("mv_formula.role_badge.3").is_some());

    // Switch column 1 (`Treatment`) from Categorical to Continuous via the Tier-3 badge menu
    driver.click_widget_id("mv_formula.role_badge.1")?;
    driver.click_widget_id("mv_formula.role_opt.1.continuous")?;
    let ds_mv = driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap();
    let TableArchetype::MultipleVariables(mv) = &ds_mv.archetype else { panic!("expected MV") };
    assert_eq!(mv.variables[1].role, VariableRole::Continuous);

    // Clicking calculated FX= badge opens Formula Bar, and FX= cells are read-only
    driver.click_widget_id("mv_formula.role_badge.3")?;
    assert!(driver.state().app.grid.formula_bar.active, "clicking FX= chip must open formula bar");
    assert!(driver.frame_meta().find_by_widget_id("grid.cell.0.3.0").is_some_and(|e| !e.enabled), "calculated MV cells must be read-only");

    Ok(())
}

#[test]
fn inline_edits_exclusions_clipboard_and_undo_parity_on_xy_and_nested() -> Result<(), String> {
    let doc = build_all_eight_archetypes_doc();
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    // 1. Inline X-cell double-click edit on XY -> single `table.setXCells` commit & undo parity
    let x0_pos = widget_center(&driver, "grid.xcell.0.0")?;
    let undo_before = driver.state().app.session().undo_depth();
    driver.click_at(x0_pos)?;
    driver.click_at(x0_pos)?;
    replace_focused_text_and_commit(&mut driver, "42.5")?;
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);
    let ds_xy = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    let TableArchetype::Xy(xy) = &ds_xy.archetype else { panic!("expected Xy") };
    assert_eq!(xy.x_cells[0][0].active_number(), Some(42.5));

    driver.state_mut().app.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    let ds_xy = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    let TableArchetype::Xy(xy) = &ds_xy.archetype else { panic!("expected Xy") };
    assert_eq!(xy.x_cells[0][0].active_number(), Some(10.0));

    // 2. GeneratedSequence X cells must be read-only (not inline editable)
    select_sheet(&mut driver, "ds-xy-seq")?;
    assert!(
        driver.frame_meta().find_by_widget_id("grid.xcell.0.0").is_some_and(|e| !e.enabled),
        "GeneratedSequence X cells must be disabled/read-only"
    );

    // 3. Inline Column, X Title, Row Title, and Sheet Title edits on XY
    select_sheet(&mut driver, "ds-xy")?;

    let xt = widget_center(&driver, "grid.x_title.0")?;
    driver.click_at(xt)?;
    driver.click_at(xt)?;
    replace_focused_text_and_commit(&mut driver, "Dose (uM)")?;
    let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else { panic!() };
    assert_eq!(xy.x_title, "Dose (uM)");

    let ct = widget_center(&driver, "grid.col_title.0")?;
    driver.click_at(ct)?;
    driver.click_at(ct)?;
    replace_focused_text_and_commit(&mut driver, "WT Treated")?;
    let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else { panic!() };
    assert_eq!(xy.data_sets[0].title, "WT Treated");

    let rt = widget_center(&driver, "grid.row_title.0")?;
    driver.click_at(rt)?;
    driver.click_at(rt)?;
    replace_focused_text_and_commit(&mut driver, "Replicate-Alpha")?;
    let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().archetype else { panic!() };
    assert_eq!(xy.row_titles[0], "Replicate-Alpha");

    let st_pos = widget_center(&driver, "grid.sheet_title")?;
    driver.click_at(st_pos)?;
    driver.click_at(st_pos)?;
    replace_focused_text_and_commit(&mut driver, "XY Renamed Sheet")?;
    assert_eq!(driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap().title, "XY Renamed Sheet");

    // 4. Nested biological-unit subcolumn inline edit (`grid.subcol_title.0.2`)
    select_sheet(&mut driver, "ds-nest")?;
    let sub_pos = widget_center(&driver, "grid.subcol_title.0.2")?;
    driver.click_at(sub_pos)?;
    driver.click_at(sub_pos)?;
    replace_focused_text_and_commit(&mut driver, "BioUnit-Gamma")?;
    let TableArchetype::Nested(nest) = &driver.state().app.document().data_sheet(&SheetId::from("ds-nest")).unwrap().archetype else { panic!() };
    assert_eq!(nest.groups[0].subcolumn_titles[2], "BioUnit-Gamma");

    // 5. Cmd/Ctrl+E exclusion & Cmd/Ctrl+C/V pasteboard on grid cells inside XY
    select_sheet(&mut driver, "ds-xy")?;
    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.press_egui_key(Key::E, Modifiers::COMMAND);
    let ds_xy = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    assert!(ds_xy.cell(CellCoord::new(0, 0, 0)).unwrap().excluded, "Cmd+E must exclude active cell");

    // Copy & Paste cell (1,0,0) -> (2,1,0)
    let val_10 = ds_xy.cell(CellCoord::new(1, 0, 0)).unwrap().active_number();
    driver.click_widget_id("grid.cell.1.0.0")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    driver.click_widget_id("grid.cell.2.1.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    let ds_xy = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    assert_eq!(ds_xy.cell(CellCoord::new(2, 1, 0)).unwrap().active_number(), val_10);

    // 6. Corner archetype button opens `ConvertArchetype`
    driver.click_widget_id("grid.corner.archetype")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }));

    Ok(())
}

#[test]
fn manage_variables_modal_master_detail_level_rename_merge_reorder_pin_and_backdrop() -> Result<(), String> {
    let doc = build_all_eight_archetypes_doc();
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);
    select_sheet(&mut driver, "ds-mv")?;

    // Open Manage Variables modal
    driver.state_mut().intents.push(UiIntent::OpenManageVariables { sheet_id: SheetId::from("ds-mv"), column: Some(1) });
    commit_intents(&mut driver);

    assert!(driver.frame_meta().find_by_widget_id("mv_formula.vars.search").is_some());
    let mv_parity = driver.verify_pixel_to_hit_box_parity();
    assert!(mv_parity.is_clean(), "ManageVariables parity violations: {:?}", mv_parity.violations);
    assert!(driver.check_scrollbar_non_overlap().is_empty());

    // Outside backdrop click must be captured and NOT select underlying grid cell
    let before_active = driver.state().app.grid.tool.active;
    driver.click_at(Pos2::new(8.0, 760.0))?;
    assert_eq!(driver.state().app.grid.tool.active, before_active);

    // Filter tabs work
    driver.click_widget_id("mv_formula.vars.filter.calc")?;
    assert!(driver.frame_meta().find_by_widget_id("mv_formula.vars.col.3").is_some());
    driver.click_widget_id("mv_formula.vars.filter.all")?;
    driver.click_widget_id("mv_formula.vars.col.1")?;

    // Metadata Name & Unit edit
    driver.click_widget_id("mv_formula.vars.unit_input")?;
    replace_focused_text(&mut driver, "arm")?;
    driver.click_widget_id("mv_formula.vars.meta.apply")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    assert_eq!(mv.variables[1].title, "Treatment (arm)");

    // Select Categorical level & Inline Rename (`Low` -> `Medium`)
    driver.click_widget_id("mv_formula.vars.level.1")?;
    driver.click_widget_id("mv_formula.vars.level_rename.1")?;
    replace_focused_text(&mut driver, "Medium")?;
    driver.click_widget_id("mv_formula.vars.level_merge.1")?;

    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    let has_medium = mv.variables[1].cells.iter().any(|c| c.active_text() == Some("Medium"));
    assert!(has_medium, "expected level inline rename `Low` -> `Medium`");

    // Merge `Medium` -> `High`, collapsing distinct levels from 3 to 2
    driver.click_widget_id("mv_formula.vars.level_rename.1")?;
    replace_focused_text(&mut driver, "High")?;
    driver.click_widget_id("mv_formula.vars.level_merge.1")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    let distinct_after_merge: std::collections::BTreeSet<&str> = mv.variables[1].cells.iter().filter_map(|c| c.active_text()).collect();
    assert_eq!(distinct_after_merge.len(), 2, "expected level merge `Medium` -> `High` to collapse 3 -> 2 levels");

    // Reorder level up (`mv_formula.vars.level_up.1`) & Pin Ref (`mv_formula.vars.level_ref.1`)
    driver.click_widget_id("mv_formula.vars.level_up.1")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    assert!(matches!(mv.variables[1].level_order.order, LevelOrderKind::Explicit(_)));

    driver.click_widget_id("mv_formula.vars.level_ref.1")?;
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::from("ds-mv")).unwrap().archetype else {
        panic!()
    };
    let LevelOrderKind::Explicit(levels) = &mv.variables[1].level_order.order else { panic!("expected explicit order") };
    assert!(!levels.is_empty());

    // Close modal cleanly via `modal.ok`
    driver.click_widget_id("modal.ok")?;
    assert!(!driver.state().app.grid.variables_dialog.open);

    Ok(())
}

#[test]
fn five_stage_import_wizard_reshape_and_convert_archetype_single_commit() -> Result<(), String> {
    let doc = build_all_eight_archetypes_doc();
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    // 1. Open 5-stage `Import & Paste Special` wizard seeded from CSV text
    driver.state_mut().intents.push(UiIntent::OpenImportWizard { origin: ImportOriginKind::Clipboard, is_live_link: false, target_sheet_id: None });
    commit_intents(&mut driver);

    driver.click_widget_id("import_paste.input.csv")?;
    driver.state_mut().views.import_wizard.raw_source_draft = String::from("Dose,Control,Treated\n1,10*,15\n10,20,NA");
    driver.run_settled();
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::Source);
    let wiz_parity = driver.verify_pixel_to_hit_box_parity();
    assert!(wiz_parity.is_clean(), "ImportWizard parity: {:?}", wiz_parity.violations);
    assert!(driver.check_scrollbar_non_overlap().is_empty());

    // Advance Stage 1 -> Stage 2 (`Preview`) using `modal.next`
    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::Preview);
    assert!(driver.frame_meta().find_by_widget_id("import_paste.preview.summary").is_some());

    // Advance Stage 2 -> Stage 3 (`Filter & Unstack`) and verify unstack disables col range
    driver.click_widget_id("modal.next")?;
    assert_eq!(driver.state().views.import_wizard.active_step, ImportWizardStep::FilterAndUnstack);
    assert!(driver.frame_meta().find_by_widget_id("import_paste.input.col_start").is_some_and(|e| e.enabled));
    driver.click_widget_id("import_paste.opt.unstack")?;
    assert!(
        driver.frame_meta().find_by_widget_id("import_paste.input.col_start").is_some_and(|e| !e.enabled),
        "enabling Unstack must disable `col_start`"
    );
    driver.click_widget_id("import_paste.opt.unstack")?; // uncheck for clean standard placement

    // Stage 4 -> Stage 5 -> Back -> Stage 5 -> Finish (`modal.ok`) in a single undo commit
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
    assert!(!driver.state().app.grid.import_wizard.open);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1, "import wizard must single-commit");

    // 2. ImportReshape modal (`ui.import.reshape.open` -> Subtract Blank Mean + 96 well values + commit)
    driver.state_mut().intents.push_ui("ui.import.reshape.open", json!({ "mode": "plate_96" }));
    commit_intents(&mut driver);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ImportReshape));
    let plate96_csv = (1..=96).map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
    driver.click_widget_id("import_paste.input.plate_values")?;
    replace_focused_text(&mut driver, &plate96_csv)?;
    driver.click_widget_id("import.subtract_blank_mean")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(
        driver.state().app.canvas_dialogs.active_dialog,
        ActiveDialog::None,
        "ImportReshape error: {:?}, values_len={}, roles_len={}, origin={:?}",
        driver.state().last_intent_error,
        driver.state().app.grid.import_reshape.plate_values.len(),
        driver.state().app.grid.import_reshape.plate_roles.len(),
        driver.state().app.grid.import_paste.source_origin,
    );

    // 3. ConvertArchetype modal (`ds-col` -> `Grouped`)
    select_sheet(&mut driver, "ds-col")?;
    driver.click_widget_id("grid.corner.archetype")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }));
    driver.click_widget_id("convert.target.grouped")?;
    driver.click_widget_id("convert.allow_lossy")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.document().data_sheet(&SheetId::from("ds-col")).unwrap().kind(), TableKind::Grouped);

    Ok(())
}

#[test]
fn transform_baseline_mv_filter_extract_and_exclude_reason_modals() -> Result<(), String> {
    let doc = build_all_eight_archetypes_doc();
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    // 1. TransformValues with Pharmacology mode + Hook Constant from InfoSheet `VehicleMass = 12.5`
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("ds-xy"), kind: TransformModalKind::TransformValues });
    commit_intents(&mut driver);

    driver.click_widget_id("edit.transform.mode.pharmacology")?;
    driver.click_widget_id("edit.transform.mode.standard")?;
    driver.click_widget_id("edit.transform.x_transform.log10")?;
    driver.click_widget_id("edit.transform.y_transform.divide_k")?;
    driver.click_widget_id("edit.transform.hook_k")?;
    driver.click_widget_id("edit.transform.hook_item.0")?;
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
        .ok_or("TransformValues must produce a Transform ResultSheet")?;

    // 2. EditAnalysisParameters (`ui.editParams.open` on existing Transform result sheet)
    select_sheet(&mut driver, &transform_rs_id)?;
    driver.state_mut().intents.push_ui("ui.editParams.open", json!({}));
    commit_intents(&mut driver);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    driver.click_widget_id("edit.transform.y_transform.multiply_k")?;
    driver.click_widget_id("modal.ok")?;

    // 3. RemoveBaseline
    select_sheet(&mut driver, "ds-xy")?;
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("ds-xy"), kind: TransformModalKind::RemoveBaseline });
    commit_intents(&mut driver);
    driver.click_widget_id("edit.transform.baseline_operation.ratio")?;
    driver.click_widget_id("modal.ok")?;

    // 4. MvFilterAndTransform AST validation: invalid syntax disables `modal.ok`, valid syntax enables
    select_sheet(&mut driver, "ds-mv")?;
    driver
        .state_mut()
        .intents
        .push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("ds-mv"), kind: TransformModalKind::MvFilterAndTransform });
    commit_intents(&mut driver);

    driver.click_widget_id("edit.transform.formula")?;
    replace_focused_text(&mut driver, "ln([")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| !e.enabled), "invalid formula syntax must disable OK");
    driver.click_widget_id("edit.transform.formula")?;
    replace_focused_text(&mut driver, "ln([Dose (mg)])")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| e.enabled));
    driver.click_widget_id("modal.ok")?;

    // 5. ExtractAndRearrange
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("ds-mv"), kind: TransformModalKind::ExtractAndRearrange });
    commit_intents(&mut driver);
    driver.click_widget_id("edit.transform.target_archetype.xy")?;
    driver.click_widget_id("modal.ok")?;

    // 6. ExcludeWithReason modal (`ui.grid.excludeWithReason`)
    select_sheet(&mut driver, "ds-xy")?;
    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.state_mut().intents.push_ui("ui.grid.excludeWithReason", json!({ "sheet_id": "ds-xy" }));
    commit_intents(&mut driver);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }));
    driver.click_widget_id("exclude.preset.0")?;
    driver.click_widget_id("exclude.note")?;
    replace_focused_text(&mut driver, "Pipette bubble verified")?;
    driver.click_widget_id("modal.ok")?;
    let ds_xy = driver.state().app.document().data_sheet(&SheetId::from("ds-xy")).unwrap();
    assert!(ds_xy.cell(CellCoord::new(0, 0, 0)).unwrap().excluded);

    Ok(())
}

#[test]
fn responsive_viewports_themes_and_all_14_beta_locales() -> Result<(), String> {
    let doc = build_all_eight_archetypes_doc();
    let mut driver = make_shell_driver(doc, 1440.0, 600.0);

    for (w, h) in [(640.0, 420.0), (1440.0, 600.0)] {
        driver.set_viewport(ViewportMetrics::new(w, h, 1.0));
        for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
            driver.state_mut().app.theme = UiThemeSpec::for_mode(theme);
            let mut toks = Tokens::from_theme_spec(&driver.state().app.theme, driver.tokens().locale);
            toks.ui_scale = 1.0;
            driver.set_tokens(toks);
            let parity = driver.verify_pixel_to_hit_box_parity();
            assert!(parity.is_clean(), "viewport ({w}x{h}) theme {:?} parity: {:?}", theme, parity.violations);
            assert!(driver.check_scrollbar_non_overlap().is_empty());
        }
    }

    // Also exercise ManageVariables responsive layout & all 14 Beta locales cleanly across 640x420 and 1440x600.
    driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
    driver.state_mut().views.trailing_collapsed_rail = true;
    select_sheet(&mut driver, "ds-mv")?;
    driver.state_mut().intents.push(UiIntent::OpenManageVariables { sheet_id: SheetId::from("ds-mv"), column: Some(1) });
    commit_intents(&mut driver);

    for (w, h) in [(640.0, 420.0), (1440.0, 600.0)] {
        driver.set_viewport(ViewportMetrics::new(w, h, 1.0));
        for locale in BETA_LOCALE_CODES {
            driver.state_mut().app.locale = String::from(locale);
            driver.set_tokens(Tokens::from_theme_spec(&driver.state().app.theme, locale));
            let parity = driver.verify_pixel_to_hit_box_parity();
            assert!(parity.is_clean(), "ManageVariables ({w}x{h}) locale `{locale}` parity: {:?}", parity.violations);
            assert!(driver.check_scrollbar_non_overlap().is_empty(), "scrollbar overlap ({w}x{h}) in locale `{locale}`");
        }
    }

    Ok(())
}

#[test]
fn one_million_cell_xy_and_mv_virtualization_regression_guard() -> Result<(), String> {
    let mut doc = ProjectDocument::new("1M Cell Guard").unwrap();

    // 1,000 vars x 1,000 rows = 1,000,000 MV cells
    let mv = MultipleVariablesTable::new(1_000, 1_000).unwrap();
    doc.add_data_sheet(DataSheet::new("mv-1m", "MV 1M", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    // 1 X col + 999 Y cols x 1,000 rows = 1,000,000 XY cells
    let xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 999, 1_000).unwrap();
    doc.add_data_sheet(DataSheet::new("xy-1m", "XY 1M", TableArchetype::Xy(xy)).unwrap()).unwrap();

    doc.select_in_navigator(&SheetId::from("mv-1m")).unwrap();
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    for sheet_id in ["mv-1m", "xy-1m"] {
        select_sheet(&mut driver, sheet_id)?;
        let vis_cells = driver.state().views.column_grid.last_visible_rows.len() * driver.state().views.column_grid.last_visible_subcols.len();
        assert!(vis_cells > 0 && vis_cells < 1_500, "O(1) viewport cell budget violated on `{sheet_id}`: {vis_cells} visible cells");
        let meta_cells =
            driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("grid.cell.") || e.widget_id.starts_with("grid.xcell.")).count();
        assert!(meta_cells > 0 && meta_cells < 1_500, "unexpected non-virtualized meta cell count {meta_cells} on `{sheet_id}`");

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
            let p50 = samples[samples.len() / 2];
            assert!(p50 <= 16.67, "1M-cell `{sheet_id}` p50 frame exceeded 16.67 ms: {p50:.3} ms");
        }
    }

    Ok(())
}
