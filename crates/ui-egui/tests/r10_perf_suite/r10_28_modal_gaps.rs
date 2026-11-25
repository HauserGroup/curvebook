#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `R10.28` Wave B Modal & Wizard GUI gap-fix regression suite (`EguiHumanPathDriver`):
//! - `WF-04`: `ImportReshape` target kind pills, `"Paste Clipboard"` button, `TidyLongPivot`
//!   column/delimiter/CSV controls, and empty-document `ImportPasteWizard` commit.
//! - `WF-15`: Power Studio `"two_sample_t"`/`"simple_linear"`/`"pilot"`/`"direct"` action slugs +
//!   `Cmd+T` parameter re-entry (`sync_from_doc_spec`, live draft sync, `modal.ok` / `cancel` / `one_sided`).
//! - `WF-19`: Contingency `edit.contingency.attributable_risk` + `R6` 2×2 CI method selector buttons.
//! - `WF-36`/`43`/`52`: Nonlinear Fit Constrain `fit.con.const.{param}` persistent draft & commit.
//! - `WF-61`: Normalize defaults (`norm_percent: true`, `norm_per_dataset: true`), inspect-recorded
//!   checkboxes, and `edit.transform.normalize.create_graph` in both standalone and `Cmd+T` edit flows.
//! - `WF-10`: One-Way ANOVA `edit.compare_against_control` syncing `compare_against_control` and
//!   advancing `Dunnett { control_column }` via `edit.control_column`.
//! - `WF-06`: Chained `ResultSheet -> KaplanMeier` MV analysis exposes `edit.tab.data` and role pickers.

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, ContingencySpec, ContingencyTable, DataSheet, GroupedTable,
    MultipleVariablesTable, MultiplicityMethod, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, ParameterScope, PowerAnalysisSpec,
    PowerEffectSpec, PowerSolveTarget, PowerTailMode, PowerTestFamily, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, SurvivalAnalysisSpec,
    TableArchetype, TableKind, TransformAnalysisSpec, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_stats::power::PowerTestFamily as StatsPowerTestFamily;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::os_integration::{PlatformClipboardAdapter, StubPlatformClipboardAdapter};
use curvebook_ui_egui::{PowerWizardStage, UiApp};
use egui::{Key, Modifiers};

struct ModalGapDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_intent_error: Option<String>,
}

impl ModalGapDriverState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::default(), last_intent_error: None }
    }
}

fn make_driver(app: UiApp, w: f64, h: f64) -> EguiHumanPathDriver<'static, ModalGapDriverState> {
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ModalGapDriverState::new(app),
        ViewportMetrics::new(w, h, 1.0),
        Tokens::default(),
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
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn replace_focused_text<S>(driver: &mut EguiHumanPathDriver<'_, S>, text: &str) -> Result<(), String> {
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.press_egui_key(Key::Backspace, Modifiers::NONE);
    if !text.is_empty() {
        driver.type_text(text)?;
    }
    Ok(())
}

fn commit_driver_intents(driver: &mut EguiHumanPathDriver<'_, ModalGapDriverState>) {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
        st.last_intent_error = Some(err.to_string());
    }
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

#[test]
fn r10_28_wf04_import_reshape_tidy_long_clipboard_and_empty_doc_import_wizard() {
    // Part A: ImportReshape TidyLongPivot + target_kind + Paste Clipboard + x/rep/excl inputs.
    let mut doc = ProjectDocument::new("WF04 Import").unwrap();
    let seed_id = SheetId::try_new("ds-seed").unwrap();
    let seed_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
    doc.data_sheets.push(DataSheet::new(seed_id.clone(), "Seed", TableArchetype::Column(seed_col)).unwrap());
    doc.active_sheet_id = Some(seed_id);

    let mut app = UiApp::from_session(Session::from_document(doc));
    let clipboard = Arc::new(StubPlatformClipboardAdapter::new());
    clipboard.write_text("X,Group,Rep,Excl,Val\n1,Ctrl,R1,0,10.5\n1,Ctrl,R2,0,11.5\n2,Drug,R1,0,22.0\n2,Drug,R2,1,99.0\n").unwrap();
    app.set_clipboard_adapter(clipboard);
    app.canvas_dialogs.open_import_reshape();

    let mut driver = make_driver(app, 1280.0, 820.0);
    driver.click_widget_id("import_paste.source.tidy").unwrap();
    driver.click_widget_id("import_paste.target.xy").unwrap();
    assert_eq!(driver.state().app.grid.import_reshape.target_kind, TableKind::Xy);
    driver.click_widget_id("import_paste.target.grouped").unwrap();
    assert_eq!(driver.state().app.grid.import_reshape.target_kind, TableKind::Grouped);
    driver.click_widget_id("import_paste.delim.comma").unwrap();
    assert_eq!(driver.state().app.grid.import_reshape.tidy_delimiter, ',');

    driver.click_widget_id("import_paste.btn.paste_clipboard").unwrap();
    assert!(driver.state().app.grid.import_reshape.tidy_csv.contains("Ctrl,R1,0,10.5"), "expected clipboard tidy CSV to populate ImportReshape");
    assert_eq!(driver.state().views.import_wizard.reshape_tidy_csv_draft, driver.state().app.grid.import_reshape.tidy_csv);

    driver.click_widget_id("import_paste.input.x_col").unwrap();
    replace_focused_text(&mut driver, "X").unwrap();
    driver.click_widget_id("import_paste.input.group_col").unwrap();
    replace_focused_text(&mut driver, "Group").unwrap();
    driver.click_widget_id("import_paste.input.replicate_col").unwrap();
    replace_focused_text(&mut driver, "Rep").unwrap();
    driver.click_widget_id("import_paste.input.value_col").unwrap();
    replace_focused_text(&mut driver, "Val").unwrap();
    driver.click_widget_id("import_paste.input.excluded_col").unwrap();
    replace_focused_text(&mut driver, "Excl").unwrap();
    assert!(driver.frame_meta().find_by_widget_id("import_paste.input.csv").is_some(), "expected import_paste.input.csv in frame_meta");

    driver.click_widget_id("modal.ok").unwrap();
    assert_eq!(driver.state().last_intent_error, None);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(driver.state().app.document().data_sheets.len(), 2);
    assert_eq!(driver.state().app.document().data_sheets[1].archetype.kind(), TableKind::Grouped);

    // Part B: Empty document CommitImportWizard with Clipboard origin.
    let empty_doc = ProjectDocument::new("Empty Doc Import").unwrap();
    let mut app_empty = UiApp::from_session(Session::from_document(empty_doc));
    let clip2 = Arc::new(StubPlatformClipboardAdapter::new());
    clip2.write_text("10\t20\n30\t40\n").unwrap();
    app_empty.set_clipboard_adapter(clip2);
    app_empty.grid.import_wizard.open = true;
    app_empty.grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;

    let mut driver2 = make_driver(app_empty, 1280.0, 820.0);
    driver2.click_widget_id("import_paste.btn.paste_clipboard").unwrap();
    assert!(driver2.state().app.grid.import_wizard.raw_clipboard_text.contains("30\t40"));
    driver2.click_widget_id("modal.ok").unwrap();
    assert_eq!(driver2.state().last_intent_error, None);
    assert!(!driver2.state().app.grid.import_wizard.open);
    assert_eq!(driver2.state().app.document().data_sheets.len(), 1);
    let created = &driver2.state().app.document().data_sheets[0];
    assert_eq!(created.cell(CellCoord::new(0, 0, 0)).unwrap().active_number(), Some(10.0));
    assert_eq!(created.cell(CellCoord::new(1, 1, 0)).unwrap().active_number(), Some(40.0));
}

#[test]
fn r10_28_wf15_power_studio_family_aliases_and_cmd_t_reentry() {
    // Part A: Standalone Power Studio family slug aliases + effect_mode:pilot/direct.
    let doc = ProjectDocument::new("WF15 Power").unwrap();
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.open_power_wizard();

    let mut driver = make_driver(app, 1280.0, 820.0);
    driver.click_widget_id("power.family.one_way_anova").unwrap();
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.family, StatsPowerTestFamily::OneWayAnova);
    driver.click_widget_id("power.family.simple_linear").unwrap();
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.family, StatsPowerTestFamily::SimpleLinearRegression);
    driver.click_widget_id("power.family.two_sample_t").unwrap();
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.family, StatsPowerTestFamily::TwoSampleUnpairedT);
    assert_eq!(driver.state().last_intent_error, None);

    driver.click_widget_id("modal.next").unwrap();
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.stage, PowerWizardStage::EffectSize);
    driver.click_widget_id("power.effect_mode.pilot").unwrap();
    assert_eq!(driver.state().last_intent_error, None);
    driver.click_widget_id("power.effect_mode.direct").unwrap();
    assert_eq!(driver.state().last_intent_error, None);

    driver.click_widget_id("modal.next").unwrap();
    driver.click_widget_id("modal.next").unwrap();
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.stage, PowerWizardStage::Results);
    driver.click_widget_id("modal.ok").unwrap();
    assert_eq!(driver.state().last_intent_error, None);

    let power_rs_id = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::PowerAnalysis(_)))
        .map(|rs| rs.id.clone())
        .expect("Power wizard Create must produce a PowerAnalysis ResultSheet");

    let power_source_id = driver.state().app.document().result_sheet(&power_rs_id).unwrap().analysis.source_sheet_id.clone();

    // Part B: Cmd+T Parameter Re-Entry syncs state, exposes OK on every stage, edits & commits.
    let mut doc2 = driver.state().app.document().clone();
    let custom_spec = PowerAnalysisSpec {
        family: PowerTestFamily::TwoSamplePairedT,
        solve_target: PowerSolveTarget::SampleSize,
        tail_mode: PowerTailMode::TwoSided,
        alpha: 0.01,
        target_power: Some(0.90),
        sample_size: None,
        sample_size_group2: None,
        allocation_ratio: 1.0,
        effect: PowerEffectSpec::Standardized(0.65),
        num_groups: None,
        chi_square_df: 1.0,
        two_proportions_mode: Default::default(),
        continuity_correction: false,
        correlation_method: Default::default(),
        info_sheet_id: None,
    };
    let mut edit_spec = AnalysisSpec::new("Power Plan", power_source_id, AnalysisKind::PowerAnalysis(custom_spec)).unwrap();
    edit_spec.id = power_rs_id.clone();
    let rs_edit = ResultSheet::new(edit_spec).unwrap();
    if let Some(slot) = doc2.result_sheet_mut(&power_rs_id) {
        *slot = rs_edit;
    }

    let mut app2 = UiApp::from_session(Session::from_document(doc2));
    assert!(app2.canvas_dialogs.open_edit_parameters(&app2.document().clone(), &power_rs_id));
    assert_eq!(app2.canvas_dialogs.power_wizard.family, StatsPowerTestFamily::TwoSamplePairedT);
    assert!((app2.canvas_dialogs.power_wizard.alpha - 0.01).abs() < 1e-12);
    assert!((app2.canvas_dialogs.power_wizard.target_power - 0.90).abs() < 1e-12);
    assert!((app2.canvas_dialogs.power_wizard.standardized_effect - 0.65).abs() < 1e-12);

    let mut driver2 = make_driver(app2, 1280.0, 820.0);
    assert!(driver2.frame_meta().find_by_widget_id("modal.ok").is_some(), "expected modal.ok on Stage 1 in Cmd+T edit mode");
    driver2.click_widget_id("power.solve.power").unwrap();
    driver2.click_widget_id("modal.next").unwrap();
    driver2.click_widget_id("modal.next").unwrap();
    assert_eq!(driver2.state().app.canvas_dialogs.power_wizard.stage, PowerWizardStage::SignificanceAndSolve);
    let one_sided_entry = driver2.frame_meta().find_by_widget_id("power.one_sided").expect("power.one_sided on Stage 3");
    assert!(one_sided_entry.enabled, "Paired t-test supports one-sided tests in Cmd+T edit mode");
    driver2.click_widget_id("power.one_sided").unwrap();
    driver2.click_widget_id("modal.ok").unwrap();

    assert_eq!(driver2.state().last_intent_error, None);
    assert_eq!(driver2.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let updated = driver2.state().app.document().result_sheet(&power_rs_id).unwrap();
    let AnalysisKind::PowerAnalysis(ref updated_pa) = updated.analysis.kind else {
        panic!("expected PowerAnalysis after Cmd+T commit");
    };
    assert_eq!(updated_pa.solve_target, PowerSolveTarget::Power);
    assert!(matches!(updated_pa.tail_mode, PowerTailMode::OneSided(_)));
}

#[test]
fn r10_28_wf19_contingency_attributable_risk_and_r6_ci_selectors() {
    let mut doc = ProjectDocument::new("WF19 Contingency").unwrap();
    let ds2x2_id = SheetId::try_new("cont-2x2").unwrap();
    let cont2 = ContingencyTable::new(2, 2).unwrap();
    let mut ds2 = DataSheet::new(ds2x2_id.clone(), "2x2 Table", TableArchetype::Contingency(cont2)).unwrap();
    for (r, (a, b)) in [(12.0, 4.0), (5.0, 14.0)].into_iter().enumerate() {
        ds2.set_cell(CellCoord::new(r, 0, 0), Cell::number(a)).unwrap();
        ds2.set_cell(CellCoord::new(r, 1, 0), Cell::number(b)).unwrap();
    }
    doc.data_sheets.push(ds2);

    let rs2x2_id = SheetId::try_new("res-2x2").unwrap();
    let mut spec2 = AnalysisSpec::new("Contingency 2x2", ds2x2_id, AnalysisKind::Contingency(ContingencySpec::default())).unwrap();
    spec2.id = rs2x2_id.clone();
    spec2.selected_data_sets = vec![0, 1];
    doc.result_sheets.push(ResultSheet::new(spec2).unwrap());

    let mut app = UiApp::from_session(Session::from_document(doc.clone()));
    assert!(app.canvas_dialogs.open_edit_parameters(&doc, &rs2x2_id));
    let mut driver = make_driver(app, 1280.0, 820.0);

    let ar = driver.frame_meta().find_by_widget_id("edit.contingency.attributable_risk").expect("attributable_risk checkbox");
    assert!(ar.enabled, "2x2 table must enable attributable risk");
    for wid in [
        "edit.contingency.ci.baptista_pike",
        "edit.contingency.ci.koopman",
        "edit.contingency.ci.newcombe_wilson",
        "edit.contingency.ci.wilson_brown",
    ] {
        assert!(!driver.frame_meta().find_by_widget_id(wid).expect("CI selector rendered").enabled, "{wid} starts disabled before effect checked");
    }

    driver.click_widget_id("edit.cont.or_rr").unwrap();
    driver.click_widget_id("edit.contingency.attributable_risk").unwrap();
    driver.click_widget_id("edit.cont.sens_spec").unwrap();
    assert!(driver.state().app.canvas_dialogs.edit_params.contingency_attributable_risk_checked);

    for wid in [
        "edit.contingency.ci.baptista_pike",
        "edit.contingency.ci.koopman",
        "edit.contingency.ci.newcombe_wilson",
        "edit.contingency.ci.wilson_brown",
    ] {
        assert!(driver.frame_meta().find_by_widget_id(wid).expect("CI selector rendered").enabled, "{wid} enabled once effect checked");
        driver.click_widget_id(wid).unwrap();
    }
    assert_eq!(driver.state().last_intent_error, None);
    let ep = &driver.state().app.canvas_dialogs.edit_params;
    assert_eq!(ep.contingency_or_ci_method, "Woolf");
    assert_eq!(ep.contingency_rr_ci_method, "Katz");
    assert_eq!(ep.contingency_arr_ci_method, "Wald");
    assert_eq!(ep.contingency_diag_ci_method, "Clopper-Pearson");

    // Verify 3x2 disables attributable_risk and all 4 CI selectors.
    let mut doc3 = ProjectDocument::new("WF19 3x2").unwrap();
    let ds3_id = SheetId::try_new("cont-3x2").unwrap();
    let cont3 = ContingencyTable::new(3, 2).unwrap();
    doc3.data_sheets.push(DataSheet::new(ds3_id.clone(), "3x2 Table", TableArchetype::Contingency(cont3)).unwrap());
    let rs3_id = SheetId::try_new("res-3x2").unwrap();
    let mut spec3 = AnalysisSpec::new("Contingency 3x2", ds3_id, AnalysisKind::Contingency(ContingencySpec::default())).unwrap();
    spec3.id = rs3_id.clone();
    spec3.selected_data_sets = vec![0, 1];
    doc3.result_sheets.push(ResultSheet::new(spec3).unwrap());

    let mut app3 = UiApp::from_session(Session::from_document(doc3.clone()));
    assert!(app3.canvas_dialogs.open_edit_parameters(&doc3, &rs3_id));
    let driver3 = make_driver(app3, 1280.0, 820.0);
    assert!(!driver3.frame_meta().find_by_widget_id("edit.contingency.attributable_risk").unwrap().enabled);
    assert!(!driver3.frame_meta().find_by_widget_id("edit.contingency.ci.baptista_pike").unwrap().enabled);
    assert!(!driver3.frame_meta().find_by_widget_id("edit.contingency.ci.newcombe_wilson").unwrap().enabled);
}

#[test]
fn r10_28_wf36_43_52_nonlinear_fit_constant_textbox() {
    let mut doc = ProjectDocument::new("WF36 Fit Constants").unwrap();
    let xy_id = SheetId::try_new("xy-dose").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
    for (r, (x, y)) in [(-9.0, 5.0), (-8.0, 10.0), (-7.0, 28.0), (-6.0, 62.0), (-5.0, 88.0), (-4.0, 95.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        *xy.data_sets[0].cell_mut(r, 0).unwrap() = Cell::number(y);
    }
    doc.data_sheets.push(DataSheet::new(xy_id.clone(), "Dose", TableArchetype::Xy(xy)).unwrap());

    let rs_id = SheetId::try_new("res-fit").unwrap();
    let mut spec =
        AnalysisSpec::new("4PL Fit", xy_id, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).unwrap();
    spec.id = rs_id.clone();
    spec.selected_data_sets = vec![0];
    doc.result_sheets.push(ResultSheet::new(spec).unwrap());

    let mut app = UiApp::from_session(Session::from_document(doc.clone()));
    assert!(app.canvas_dialogs.open_edit_parameters(&doc, &rs_id));
    let mut driver = make_driver(app, 1280.0, 860.0);

    driver.click_widget_id("fit.tab.constraints").unwrap();
    driver.click_widget_id("fit.con.rule.Bottom.fixed").unwrap();
    driver.click_widget_id("fit.con.rule.Top.fixed").unwrap();

    driver.click_widget_id("fit.con.const.Bottom").unwrap();
    replace_focused_text(&mut driver, "5").unwrap();
    driver.click_widget_id("fit.con.const.Top").unwrap();
    replace_focused_text(&mut driver, "95").unwrap();
    driver.click_widget_id("modal.ok").unwrap();

    assert_eq!(driver.state().last_intent_error, None);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let committed = driver.state().app.document().result_sheet(&rs_id).unwrap();
    let AnalysisKind::NonlinearRegression(ref nl) = committed.analysis.kind else {
        panic!("expected NonlinearRegression");
    };
    let bottom_rule = nl.parameter_rules.iter().find(|r| r.name == "Bottom").unwrap();
    let top_rule = nl.parameter_rules.iter().find(|r| r.name == "Top").unwrap();
    assert_eq!(bottom_rule.scope, ParameterScope::Constant(5.0));
    assert_eq!(top_rule.scope, ParameterScope::Constant(95.0));
}

#[test]
fn r10_28_wf61_normalize_defaults_checkboxes_and_create_graph() {
    let mut doc = ProjectDocument::new("WF61 Normalize").unwrap();
    let ds_id = SheetId::try_new("grp-norm").unwrap();
    let grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 4).unwrap();
    let mut ds_grp = DataSheet::new(ds_id.clone(), "Signals", TableArchetype::Grouped(grp)).unwrap();
    for (r, (a, b)) in [(10.0, 20.0), (20.0, 40.0), (50.0, 80.0), (110.0, 100.0)].into_iter().enumerate() {
        ds_grp.set_cell(CellCoord::new(r, 0, 0), Cell::number(a)).unwrap();
        ds_grp.set_cell(CellCoord::new(r, 1, 0), Cell::number(b)).unwrap();
    }
    doc.data_sheets.push(ds_grp);
    doc.active_sheet_id = Some(ds_id.clone());

    // Part A: Standalone Normalize modal defaults and graph creation.
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.egui_views.transform_modals.open_kind = Some(TransformModalKind::Normalize);
    app.egui_views.transform_modals.target_sheet_id = Some(ds_id.clone());
    assert!(app.egui_views.transform_modals.norm_percent);
    assert!(app.egui_views.transform_modals.norm_per_dataset);

    let mut driver = make_driver(app, 1280.0, 820.0);
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.norm_percent").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.norm_per_dataset").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.normalize.create_graph").is_some());

    driver.click_widget_id("edit.transform.normalize.create_graph").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    assert_eq!(driver.state().last_intent_error, None);
    assert_eq!(driver.state().app.document().result_sheets.len(), 1);
    let norm_rs_id = driver.state().app.document().result_sheets[0].id.clone();
    assert_eq!(driver.state().app.document().graph_sheets.len(), 1);
    assert_eq!(driver.state().app.document().graph_sheets[0].spec.primary_source_sheet_id, norm_rs_id);

    // Part B: Cmd+T edit mode on Normalize ResultSheet creates graph when no GraphSheet exists yet.
    let mut doc2 = ProjectDocument::new("WF61 CmdT").unwrap();
    doc2.data_sheets = driver.state().app.document().data_sheets.clone();
    let rs2_id = SheetId::try_new("res-norm-edit").unwrap();
    let mut spec2 = AnalysisSpec::new(
        "Normalize Edit",
        ds_id,
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: true,
        }),
    )
    .unwrap();
    spec2.id = rs2_id.clone();
    spec2.selected_data_sets = vec![0, 1];
    doc2.result_sheets.push(ResultSheet::new(spec2).unwrap());

    let mut app2 = UiApp::from_session(Session::from_document(doc2.clone()));
    assert!(app2.canvas_dialogs.open_edit_parameters(&doc2, &rs2_id));
    let mut driver2 = make_driver(app2, 1280.0, 820.0);
    driver2.click_widget_id("edit.transform.norm_zero.2").unwrap();
    driver2.click_widget_id("edit.transform.norm_hundred.1").unwrap();
    driver2.click_widget_id("edit.transform.norm_percent").unwrap();
    driver2.click_widget_id("edit.transform.norm_per_dataset").unwrap();
    driver2.click_widget_id("edit.transform.normalize.create_graph").unwrap();
    driver2.click_widget_id("modal.ok").unwrap();

    assert_eq!(driver2.state().last_intent_error, None);
    let updated = driver2.state().app.document().result_sheet(&rs2_id).unwrap();
    let AnalysisKind::Transform(TransformAnalysisSpec::Normalize { ref zero_anchor, ref hundred_anchor, output_percent, per_data_set }) =
        updated.analysis.kind
    else {
        panic!("expected Normalize spec");
    };
    assert_eq!(*zero_anchor, NormalizeAnchor::FixedValue(0.0));
    assert_eq!(*hundred_anchor, NormalizeAnchor::ColumnSum);
    assert!(!output_percent, "expected unchecked norm_percent to persist output_percent=false");
    assert!(!per_data_set, "expected unchecked norm_per_dataset to persist per_data_set=false");
    assert_eq!(driver2.state().app.document().graph_sheets.len(), 1);
    assert_eq!(driver2.state().app.document().graph_sheets[0].spec.primary_source_sheet_id, rs2_id);
}

#[test]
fn r10_28_wf06_chained_result_sheet_mv_survival_exposes_data_tab_and_role_pickers() {
    let mut doc = ProjectDocument::new("WF06 Chained ResultSheet").unwrap();
    let ds_id = SheetId::try_new("mv-src").unwrap();
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("Status");
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[2].title = String::from("Arm");
    mv.variables[2].role = VariableRole::Categorical;
    for (r, (t, s, g)) in [(5.0, 1.0, "A"), (10.0, 1.0, "A"), (7.0, 1.0, "B"), (14.0, 0.0, "B")].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(t);
        mv.variables[1].cells[r] = Cell::number(s);
        mv.variables[2].cells[r] = Cell::text(g);
    }
    let upstream_ds = DataSheet::new(ds_id.clone(), "MV Cohort", TableArchetype::MultipleVariables(mv)).unwrap();
    doc.data_sheets.push(upstream_ds.clone());

    let prune_id = SheetId::try_new("res-prune").unwrap();
    let mut prune_spec = AnalysisSpec::new(
        "Prune Cohort",
        ds_id,
        AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 1, average_window: false }),
    )
    .unwrap();
    prune_spec.id = prune_id.clone();
    prune_spec.selected_data_sets = vec![0, 1, 2];
    let mut prune_rs = ResultSheet::new(prune_spec).unwrap();
    prune_rs.analyzable_data = Some(upstream_ds);
    doc.result_sheets.push(prune_rs);

    let km_id = SheetId::try_new("res-km").unwrap();
    let mut km_spec = AnalysisSpec::new(
        "KM Survival",
        prune_id,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: Default::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    )
    .unwrap();
    km_spec.id = km_id.clone();
    km_spec.selected_data_sets = vec![0, 1, 2];
    doc.result_sheets.push(ResultSheet::new(km_spec).unwrap());

    let mut app = UiApp::from_session(Session::from_document(doc.clone()));
    assert!(app.canvas_dialogs.open_edit_parameters(&doc, &km_id));
    let mut driver = make_driver(app, 1280.0, 820.0);

    assert!(driver.frame_meta().find_by_widget_id("edit.tab.data").is_some(), "chained MV ResultSheet must expose edit.tab.data");
    driver.click_widget_id("edit.tab.data").unwrap();
    assert!(driver.frame_meta().find_by_widget_id("edit.km.time.0").is_some(), "expected KM MV time role picker");
    assert!(driver.frame_meta().find_by_widget_id("edit.km.status.1").is_some(), "expected KM MV status role picker");
    assert!(driver.frame_meta().find_by_widget_id("edit.km.group.2").is_some(), "expected KM MV group role picker");

    driver.click_widget_id("edit.km.time.0").unwrap();
    driver.click_widget_id("edit.km.status.1").unwrap();
    driver.click_widget_id("edit.km.group.2").unwrap();
    driver.click_widget_id("modal.ok").unwrap();

    assert_eq!(driver.state().last_intent_error, None);
    let committed = driver.state().app.document().result_sheet(&km_id).unwrap();
    let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, mv_status_column, mv_group_column, .. }) = committed.analysis.kind
    else {
        panic!("expected KaplanMeier");
    };
    assert_eq!(mv_time_column, Some(0));
    assert_eq!(mv_status_column, Some(1));
    assert_eq!(mv_group_column, Some(2));
}

#[test]
fn r10_28_wf10_anova_compare_against_control_dunnett_cycle() {
    let mut doc = ProjectDocument::new("WF10 ANOVA Dunnett").unwrap();
    let ds_id = SheetId::try_new("col-anova").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).unwrap();
    for c in 0..3 {
        col.data_sets[c].title = format!("Grp{}", c + 1);
    }
    let mut ds_col = DataSheet::new(ds_id.clone(), "Groups", TableArchetype::Column(col)).unwrap();
    for c in 0..3 {
        for r in 0..4 {
            ds_col.set_cell(CellCoord::new(r, c, 0), Cell::number((c * 5 + r + 1) as f64)).unwrap();
        }
    }
    doc.data_sheets.push(ds_col);

    let rs_id = SheetId::try_new("res-anova").unwrap();
    let mut spec = AnalysisSpec::new("One-Way ANOVA", ds_id, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
    spec.id = rs_id.clone();
    spec.selected_data_sets = vec![0, 1, 2];
    doc.result_sheets.push(ResultSheet::new(spec).unwrap());

    let mut app = UiApp::from_session(Session::from_document(doc.clone()));
    assert!(app.canvas_dialogs.open_edit_parameters(&doc, &rs_id));
    let mut driver = make_driver(app, 1280.0, 820.0);

    driver.click_widget_id("edit.tab.multiple_comparisons").unwrap();
    assert!(!driver.frame_meta().find_by_widget_id("edit.control_column").unwrap().enabled);

    driver.click_widget_id("edit.compare_against_control").unwrap();
    commit_driver_intents(&mut driver);
    assert!(driver.state().app.canvas_dialogs.edit_params.compare_against_control);
    assert!(driver.frame_meta().find_by_widget_id("edit.control_column").unwrap().enabled);

    driver.click_widget_id("edit.control_column").unwrap();
    driver.click_widget_id("modal.ok").unwrap();

    assert_eq!(driver.state().last_intent_error, None);
    let committed = driver.state().app.document().result_sheet(&rs_id).unwrap();
    let AnalysisKind::OneWayAnova(ref anova) = committed.analysis.kind else {
        panic!("expected OneWayAnova");
    };
    assert_eq!(anova.multiplicity.method, MultiplicityMethod::Dunnett { control_column: 1 });
}
