#![forbid(unsafe_code)]

//! Wave 4 (`R15.4`) comprehensive test suite for unit spinners, `Analyze...`, statistical modals,
//! 7-tab `Nonlinear Curve Fitting` + 3-branch `Hook Constant`, 4-stage `Power & Sample Size Studio`,
//! and 14-locale modal backdrop/scrollbar parity.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, GlmFamily, GlmSpec, InfoConstant,
    InfoSheet, MultipleVariablesTable, NonlinearFitSpec, ProjectDocument, ResultCell, ResultScalarEntry, ResultSheet, ResultTab, ResultTabKind,
    SheetId, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TwoGroupSpec, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_fit::list_builtin_equations;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver};
use curvebook_ui_egui::egui_shell::fit_modal::FitModalTab;
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    HybridCoexistenceArbiter, HybridModalSurface, check_accesskit_scrollbar_non_overlap, render_egui_shell_frame,
};
use curvebook_ui_egui::egui_shell::power_studio::PowerResultsSubTab;
use curvebook_ui_egui::egui_shell::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_length_pt_field};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{UiApp, UiThemeMode};
use serde_json::json;

use crate::common::{ShellTestState, assert_clean_parity, fill_col_values, make_shell_driver};

type BoxRes = Result<(), Box<dyn std::error::Error>>;

fn make_driver(app: UiApp, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    make_shell_driver(app, ViewportMetrics::new(w, h, 1.0))
}

fn make_result_sheet(id: SheetId, title: &str, src: SheetId, kind: AnalysisKind) -> Result<ResultSheet, Box<dyn std::error::Error>> {
    let mut spec = AnalysisSpec::new(title, src, kind)?;
    spec.id = id;
    Ok(ResultSheet::new(spec)?)
}

fn seed_rich_project_app() -> Result<(UiApp, SheetId, SheetId, SheetId), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();

    let col_id = SheetId::try_new("col-main")?;
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5)?;
    fill_col_values(&mut col_tbl.data_sets[0], "Control", &[10.1, 10.4, 9.8, 10.2, 10.0]);
    fill_col_values(&mut col_tbl.data_sets[1], "Treated A", &[14.2, 14.8, 13.9, 14.5, 14.1]);
    fill_col_values(&mut col_tbl.data_sets[2], "Treated B", &[17.0, 17.5, 16.9, 17.3, 17.1]);
    let col_ds = DataSheet::new(col_id.clone(), "Column Cohort Study", TableArchetype::Column(col_tbl))?;

    let mut alt_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    fill_col_values(&mut alt_tbl.data_sets[0], "Baseline", &[5.0, 5.4, 5.2, 5.1]);
    fill_col_values(&mut alt_tbl.data_sets[1], "Followup", &[7.1, 7.6, 7.3, 7.2]);
    let alt_col_ds = DataSheet::new(SheetId::try_new("col-alt")?, "Followup Column Cohort", TableArchetype::Column(alt_tbl))?;

    let xy_id = SheetId::try_new("xy-dose")?;
    let mut xy_tbl = XyTable::new(SubcolumnMode::Single, 1, 6)?;
    xy_tbl.x_title = String::from("Log[Agonist]");
    for (i, &x) in [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0].iter().enumerate() {
        xy_tbl.x_cells[i][0] = Cell::number(x);
    }
    fill_col_values(&mut xy_tbl.data_sets[0], "Receptor Response", &[12.0, 22.0, 51.0, 82.0, 96.0, 99.0]);
    let xy_ds = DataSheet::new(xy_id.clone(), "Dose Response Curve", TableArchetype::Xy(xy_tbl))?;

    let mv_id = SheetId::try_new("mv-clinical")?;
    let mut mv_tbl = MultipleVariablesTable::new(5, 6)?;
    for (idx, (title, role)) in [
        ("Time_Mo", VariableRole::Continuous),
        ("Event", VariableRole::Continuous),
        ("Age", VariableRole::Continuous),
        ("Arm", VariableRole::Categorical),
        ("Biomarker", VariableRole::Continuous),
    ]
    .into_iter()
    .enumerate()
    {
        mv_tbl.variables[idx].title = title.to_owned();
        mv_tbl.variables[idx].role = role;
    }
    for r in 0..6 {
        mv_tbl.variables[0].cells[r] = Cell::number(6.0 + r as f64 * 2.5);
        mv_tbl.variables[1].cells[r] = Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 });
        mv_tbl.variables[2].cells[r] = Cell::number(40.0 + r as f64 * 4.0);
        mv_tbl.variables[3].cells[r] = Cell::text(if r < 3 { "Placebo" } else { "Drug" });
        mv_tbl.variables[4].cells[r] = Cell::number(1.2 + r as f64 * 0.4);
    }
    let mv_ds = DataSheet::new(mv_id.clone(), "Clinical Survival & Covariates", TableArchetype::MultipleVariables(mv_tbl))?;

    let mut cont_tbl = ContingencyTable::new(2, 2)?;
    fill_col_values(&mut cont_tbl.data_sets[0], "Responders", &[14.0, 5.0]);
    fill_col_values(&mut cont_tbl.data_sets[1], "NonResponders", &[6.0, 15.0]);
    let cont_ds = DataSheet::new(SheetId::try_new("cont-2x2")?, "2x2 Responder Matrix", TableArchetype::Contingency(cont_tbl))?;

    let mut info = InfoSheet::new_preseeded(SheetId::try_new("info-assay")?, "Assay Protocol Metadata")?;
    info.constants.push(InfoConstant::new("ControlBaseline", ConstantValue::Number(10.0), Some(String::from("%")))?);
    info.constants.push(InfoConstant::new("MaxCap", ConstantValue::Number(100.0), None)?);

    let mut seed_rs = make_result_sheet(
        SheetId::try_new("res-seed-fit")?,
        "Seeded Reference Fit",
        xy_id.clone(),
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("michaelis_menten")),
    )?;
    seed_rs.scalars.push(ResultScalarEntry { key: String::from("Km"), data_set_index: Some(0), value: 12.5, units: None });
    seed_rs.tabs.push(ResultTab {
        id: String::from("tab-summary"),
        title: String::from("Fit Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Parameter"), String::from("Best-fit")],
        row_labels: vec![String::from("Km")],
        rows: vec![vec![ResultCell::Text(String::from("Km")), ResultCell::Number { value: 12.5, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });

    let mut doc = ProjectDocument::new("Wave 4 Stats Fit Power Project")?;
    doc.data_sheets = vec![col_ds, alt_col_ds, xy_ds, mv_ds, cont_ds];
    doc.info_sheets = vec![info];
    doc.result_sheets = vec![seed_rs];
    doc.active_sheet_id = Some(col_id.clone());
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();
    Ok((app, col_id, xy_id, mv_id))
}

#[test]
fn unit_spinners_single_commit_undo_and_mm_in_pt_conversion() -> BoxRes {
    struct SpinnerHarness {
        app: UiApp,
        col_id: SheetId,
        width_pt: f64,
        spinner: ScrubUnitFieldState,
        commits_observed: usize,
    }
    let (app, col_id, _, _) = seed_rich_project_app()?;
    let undo_before = app.session().undo_depth();
    let mut driver = EguiHumanPathDriver::new(
        SpinnerHarness { app, col_id, width_pt: 72.0, spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points), commits_observed: 0 },
        ViewportMetrics::new(640.0, 420.0, 1.0),
        |ui, meta, st| {
            let tokens = Tokens::default();
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let out = render_scrub_length_pt_field(
                        ui,
                        st.width_pt,
                        &mut st.spinner,
                        ScrubUnitFieldConfig::new("test.width_spinner", "Column Width", 12.0, 360.0),
                        &tokens,
                        meta,
                    );
                    if let Some(new_pt) = out.committed_value {
                        st.commits_observed += 1;
                        st.width_pt = new_pt;
                        let _ = st
                            .app
                            .execute_command("sheet.rename", json!({ "sheet_id": st.col_id.as_str(), "title": format!("ColWidth_{new_pt:.1}pt") }));
                    }
                });
            });
        },
    );
    driver.run_settled();

    let rect = driver.frame_meta().find_by_widget_id("test.width_spinner").ok_or("missing spinner")?.clipped_rect;
    let start_pos = egui::pos2(rect.left() + 14.0, rect.center().y);
    driver.dispatch_events(vec![
        egui::Event::PointerMoved(start_pos),
        egui::Event::PointerButton { pos: start_pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
    ]);
    for step in 1..=4 {
        driver.dispatch_events(vec![egui::Event::PointerMoved(egui::pos2(start_pos.x + (step as f32) * 10.0, start_pos.y))]);
        assert!(driver.state().spinner.dragging && driver.state().commits_observed == 0 && driver.state().app.session().undo_depth() == undo_before);
    }
    driver.dispatch_events(vec![egui::Event::PointerButton {
        pos: egui::pos2(start_pos.x + 40.0, start_pos.y),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(
        !driver.state().spinner.dragging
            && driver.state().commits_observed == 1
            && driver.state().app.session().undo_depth() == undo_before + 1
            && driver.state().width_pt > 72.0
    );
    assert!(driver.state_mut().app.session_mut().undo()?);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before);

    driver.click_at(rect.center())?;
    assert!(driver.state().spinner.editing);
    driver.state_mut().spinner.draft_text = String::from("1 in");
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!((driver.state().width_pt - 72.0).abs() < 1e-6);

    driver.state_mut().spinner.unit = ScrubUnitKind::Millimeters;
    driver.state_mut().spinner.editing = true;
    driver.state_mut().spinner.draft_text = String::from("2 in");
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!((driver.state().width_pt - 144.0).abs() < 1e-6);
    Ok(())
}

#[test]
fn analyze_autofocus_instant_launch_duplicate_and_retarget_modals() -> BoxRes {
    let (mut app, col_id, _, _) = seed_rich_project_app()?;
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc, &col_id));

    let mut driver = make_driver(app, 1024.0, 640.0);
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiAnalyzeSheet
    );
    assert!(driver.frame_meta().find_by_widget_id("analyze.search").is_some());
    driver.type_text("unpaired")?;
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.two_group").is_some());
    driver.click_widget_id("analyze.entry.stats.two_group")?;
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let first_rs_id = driver.state().app.document().active_sheet_id.clone().ok_or("missing ResultSheet")?;
    assert!(driver.state().app.document().result_sheet(&first_rs_id).is_some());

    let doc2 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_analyze_dialog(&doc2, &col_id));
    driver.run_settled();
    driver.click_widget_id("analyze.entry.stats.two_group")?;
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }));
    driver.click_widget_id("analyze_picker.dup.create_new")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let doc3 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_change_data_analyzed(&doc3, &first_rs_id));
    driver.run_settled();
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiChangeDataAnalyzed
    );
    let incompat = driver.frame_meta().find_by_widget_id("change_data.candidate.2").ok_or("missing XY candidate")?;
    assert!(!incompat.enabled && incompat.disabled_reason.is_some());
    driver.click_widget_id("change_data.candidate.1")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.document().result_sheet(&first_rs_id).ok_or("missing result")?.analysis.source_sheet_id.as_str(), "col-alt");
    Ok(())
}

#[test]
fn statistical_glm_cox_pca_clustering_survival_and_r9_modals() -> BoxRes {
    let (mut app, col_id, _, mv_id) = seed_rich_project_app()?;

    let mut doc = app.document().clone();
    let glm_id = SheetId::try_new("res-glm")?;
    doc.result_sheets.push(make_result_sheet(
        glm_id.clone(),
        "Clinical GLM",
        mv_id.clone(),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 1,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            compute_vif: true,
        }),
    )?);
    let cox_id = SheetId::try_new("res-cox")?;
    doc.result_sheets.push(make_result_sheet(
        cox_id.clone(),
        "Cox PH",
        mv_id,
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 4],
            reference_levels: Vec::new(),
            tie_method: curvebook_doc::CoxTieMethod::Efron,
        }),
    )?);
    let surv2_id = SheetId::try_new("surv-2grp")?;
    doc.data_sheets.push(DataSheet::new(surv2_id.clone(), "Survival 2-Cohort", TableArchetype::Survival(curvebook_doc::SurvivalTable::new(2, 4)?))?);
    let km_id = SheetId::try_new("res-km")?;
    doc.result_sheets.push(make_result_sheet(
        km_id.clone(),
        "KM",
        surv2_id,
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
    )?);
    let pre_id = SheetId::try_new("col-presum")?;
    doc.data_sheets.push(DataSheet::new(
        pre_id.clone(),
        "Pre-Summarized",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 3)?),
    )?);
    let tg_id = SheetId::try_new("res-tg")?;
    doc.result_sheets.push(make_result_sheet(tg_id.clone(), "PreSum TwoGroup", pre_id, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()))?);
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();

    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &glm_id));
    let mut driver = make_driver(app, 1024.0, 640.0);
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiStatParams
    );
    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.glm.logistic")?;
    assert!(driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("formula_bar")?.label.contains("logit("));
    driver.click_widget_id("edit.tab.data")?;
    driver.click_widget_id("edit.glm.pred.4")?;
    assert!(
        matches!(&driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::GeneralizedLinearModel(g)) if g.predictor_columns.contains(&4) && g.family == GlmFamily::Logistic)
    );
    driver.click_widget_id("dialog.button.method_ref")?;
    driver.click_widget_id("dialog.button.save_default")?;
    assert!(driver.state().views.stats_modals.method_saved_as_default);
    driver.click_widget_id("modal.cancel")?;

    let snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap, &cox_id));
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("cox formula")?.label.contains("h(t | X)"));
    for wid in ["edit.tab.experimental_design", "edit.cox.exact", "modal.cancel"] {
        driver.click_widget_id(wid)?;
    }

    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap, &km_id));
    driver.run_settled();
    let km_trend = driver.frame_meta().find_by_widget_id("edit.km.trend").ok_or("km.trend")?;
    assert!(!km_trend.enabled && km_trend.disabled_reason.as_deref().is_some_and(|r| r.contains('3')));
    driver.click_widget_id("modal.cancel")?;

    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap, &tg_id));
    driver.run_settled();
    let gauss = driver.frame_meta().find_by_widget_id("edit.assume_gaussian").ok_or("assume_gaussian")?;
    assert!(!gauss.enabled && gauss.disabled_reason.is_some());
    driver.click_widget_id("modal.cancel")?;

    driver.state_mut().app.canvas_dialogs.random_sample_rows.open_for_sheet(&snap, &col_id);
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::RandomSampleRows { source_sheet_id: Some(col_id) };
    driver.run_settled();
    assert!(!driver.frame_meta().find_by_widget_id("sample.proportional_allocation").ok_or("proportional_allocation")?.enabled);
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn seven_tab_nonlinear_fit_and_three_branch_hook_browser() -> BoxRes {
    assert!(list_builtin_equations().len() >= 117);
    let (mut app, _, xy_id, _) = seed_rich_project_app()?;
    let fit_id = SheetId::try_new("res-nlf")?;
    let mut doc = app.document().clone();
    doc.result_sheets.push(make_result_sheet(
        fit_id.clone(),
        "Dose Curve Fit",
        xy_id,
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")),
    )?);
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &fit_id));

    let mut driver = make_driver(app, 1024.0, 640.0);
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiFitModal
    );
    for tab in FitModalTab::CORE_SEVEN {
        assert!(driver.frame_meta().find_by_widget_id(&format!("fit.tab.{}", tab.slug())).is_some());
    }
    assert!(driver.frame_meta().find_by_widget_id("fit.eq.tex_preview").is_some());

    driver.click_widget_id("fit.eq.search")?;
    driver.type_text("michaelis")?;
    driver.click_widget_id("fit.eq.item.michaelis_menten")?;
    assert!(
        matches!(driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::NonlinearRegression(f)) if f.equation_id == "michaelis_menten")
    );

    for wid in ["fit.eq.custom_toggle", "fit.eq.dsl.ode", "fit.tab.weight", "fit.method.poisson"] {
        driver.click_widget_id(wid)?;
    }
    let rel_wt = driver.frame_meta().find_by_widget_id("fit.wt.rel_sq").ok_or("fit.wt.rel_sq")?;
    assert!(!rel_wt.enabled && rel_wt.disabled_reason.is_some());

    for wid in [
        "fit.tab.compare",
        "fit.cmp.aicc",
        "fit.tab.constraints",
        "fit.tab.initial_values",
        "fit.tab.weights_range",
        "fit.tab.output",
        "modal.cancel",
    ] {
        driver.click_widget_id(wid)?;
    }

    let doc_hook = driver.state().app.document().clone();
    driver.state_mut().app.canvas_dialogs.open_hook_picker(&doc_hook, &fit_id, Some(String::from("Bottom")));
    driver.run_settled();
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiHookPicker
    );
    for br in ["hook.branch.info", "hook.branch.analysis", "hook.branch.document"] {
        assert!(driver.frame_meta().find_by_widget_id(br).is_some());
    }
    driver.click_widget_id("hook.item.0")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn power_and_sample_size_studio_four_stages_65_35_and_sensitivity_sidebar() -> BoxRes {
    let (mut app, _, _, _) = seed_rich_project_app()?;
    app.canvas_dialogs.open_power_wizard();
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize);

    let mut driver = make_driver(app, 1024.0, 640.0);
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiPowerStudio
    );
    for st in 1..=4 {
        assert!(driver.frame_meta().find_by_widget_id(&format!("power.stage.{st}")).is_some());
    }
    assert!(driver.frame_meta().find_by_widget_id("power.sidebar.alpha").is_some());
    driver.click_widget_id("power.family.one_way_anova")?;
    driver.click_widget_id("power.stage.3")?;
    let one_sided = driver.frame_meta().find_by_widget_id("power.one_sided").ok_or("power.one_sided")?;
    assert!(!one_sided.enabled && one_sided.disabled_reason.is_some());

    for wid in ["power.stage.1", "power.family.two_sample_t", "power.compute_defaults"] {
        driver.click_widget_id(wid)?;
    }
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.stage, curvebook_ui_egui::power_wizard::PowerWizardStage::Results);

    driver.click_widget_id("power.copy_narrative")?;
    assert!(driver.state().views.power_studio.narrative_copied);
    driver.click_widget_id("power.tab.curve")?;
    assert_eq!(driver.state().views.power_studio.results_tab, PowerResultsSubTab::PowerCurve);
    driver.hover_at(driver.frame_meta().find_by_widget_id("power.curve_hover").ok_or("power.curve_hover")?.clipped_rect.center())?;
    assert!(driver.state().views.power_studio.hover_n.is_some());

    driver.click_widget_id("power.tab.sensitivity")?;
    assert_eq!(driver.state().views.power_studio.results_tab, PowerResultsSubTab::SensitivityMatrix);
    driver.click_widget_id("power.sensitivity.row.0")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn all_14_beta_locales_backdrop_capture_and_scrollbar_non_overlap_at_640x420_and_1440x600() -> BoxRes {
    assert_eq!(BETA_LOCALE_CODES.len(), 14);
    let (mut template_app, col_id, xy_id, mv_id) = seed_rich_project_app()?;

    let mut doc = template_app.document().clone();
    let glm_id = SheetId::try_new("res-loc-glm")?;
    doc.result_sheets.push(make_result_sheet(
        glm_id.clone(),
        "Loc GLM",
        mv_id,
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 1,
            predictor_columns: vec![2, 4],
            reference_levels: Vec::new(),
            compute_vif: true,
        }),
    )?);
    let fit_id = SheetId::try_new("res-loc-fit")?;
    doc.result_sheets.push(make_result_sheet(
        fit_id.clone(),
        "Loc Fit",
        xy_id,
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")),
    )?);
    *template_app.session_mut() = Session::from_document(doc);
    template_app.sync_views();

    for (loc_idx, &locale) in BETA_LOCALE_CODES.iter().enumerate() {
        let theme = [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast][loc_idx % 3];
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            for surface_kind in 0..4 {
                let mut app = template_app.clone();
                assert_eq!(app.set_locale(locale), locale);
                app.set_theme_mode(theme);
                let d_snap = app.document().clone();
                match surface_kind {
                    0 => assert!(app.canvas_dialogs.open_edit_parameters(&d_snap, &glm_id)),
                    1 => assert!(app.canvas_dialogs.open_edit_parameters(&d_snap, &fit_id)),
                    2 => {
                        app.canvas_dialogs.open_power_wizard();
                        app.canvas_dialogs.power_wizard.stage = curvebook_ui_egui::power_wizard::PowerWizardStage::Results;
                    }
                    _ => assert!(app.canvas_dialogs.open_analyze_dialog(&d_snap, &col_id)),
                }
                app.sync_views();

                let mut driver = make_driver(app, w_pt, h_pt);
                assert_clean_parity(&driver, &format!("locale `{locale}` surf={surface_kind} @ {w_pt}x{h_pt}"))?;
                let outside = egui::pos2(3.0, (h_pt as f32) * 0.5);
                driver.wheel_at(outside, egui::vec2(0.0, -100.0))?;
                driver.click_at(outside)?;
                assert!(driver.state().outside_click_captured && driver.state().outside_wheel_captured);
            }
        }
    }

    let mut ak_app = template_app;
    ak_app.canvas_dialogs.open_power_wizard();
    ak_app.canvas_dialogs.power_wizard.stage = curvebook_ui_egui::power_wizard::PowerWizardStage::Results;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut views, mut meta, mut intents) =
        (EguiShellViewState::default(), curvebook_ui_egui::egui_shell::FrameInspectMeta::default(), PendingUiIntents::new());
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
        |ui| {
            let _ = render_egui_shell_frame(ui, &ak_app, &mut views, &mut meta, &mut intents);
        },
    );
    out.textures_delta.clear();
    check_accesskit_scrollbar_non_overlap(out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?)?;
    Ok(())
}
