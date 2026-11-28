#![forbid(unsafe_code)]

//! Wave 4 (`R15.4`) comprehensive test suite for unit spinners, `Analyze...`, statistical modals,
//! 7-tab `Nonlinear Curve Fitting` + 3-branch `Hook Constant`, 4-stage `Power & Sample Size Studio`,
//! and 14-locale modal backdrop/scrollbar parity.

use std::sync::Arc;

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
use curvebook_ui_egui::{StubPlatformClipboardAdapter, UiApp, UiThemeMode};
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
    assert_eq!(
        driver.state().app.document().result_sheet(&first_rs_id).ok_or("missing result")?.analysis.source_sheet_id.as_ref().map(SheetId::as_str),
        Some("col-alt")
    );
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
            ..Default::default()
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
            formula: None,
            covariate_profiles: Vec::new(),
            nested_comparison: None,
            emit_cox_snell_residuals: false,
            ci_method: Default::default(),
            event_code: None,
            censored_code: None,
            treat_other_as: Default::default(),
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
            event_code: None,
            censored_code: None,
            treat_other_as: Default::default(),
            ratio_median_ci: false,
            mv_status_columns: Vec::new(),
            mv_group_columns: Vec::new(),
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
    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(Arc::clone(&clip) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);
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
    assert!(clip.last_text().is_some_and(|t| !t.trim().is_empty()), "clipboard adapter must receive copied narrative");
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
            ..Default::default()
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

#[test]
fn aud_07_08_09_10_ext01_stats_fit_power_modals_regressions() -> BoxRes {
    use crate::helpers::{click_input, run_egui_pass_with_ctx, seed_column_app};
    use curvebook_doc::{FactorialAnovaSpec, GroupedTable};
    use curvebook_stats::power::{PowerSolveTarget, PowerTestFamily};
    use curvebook_ui_egui::dialogs_r9::RandomSampleRowsDialogState;
    use curvebook_ui_egui::egui_shell::power_studio::{
        PowerStudioEguiState, build_narrative_paragraph, render_power_studio_modal, sync_anova_group_means,
    };
    use curvebook_ui_egui::egui_shell::{FrameInspectMeta, StatParamsIntent, UiIntent};
    use curvebook_ui_egui::power_wizard::{PowerEffectInputMode, PowerWizardStage, PowerWizardState};

    // 1. AUD-07-24/25 & AUD-08-12/17: PowerWizard RegressionSlope reset, RandomSample stratify reset, ResolveDuplicate err keeps modal, ToggleSchematic MeanSdN guard.
    let mut pw = PowerWizardState::default();
    pw.set_family(PowerTestFamily::SimpleLinearRegression);
    pw.effect_mode = PowerEffectInputMode::RegressionSlope;
    pw.set_family(PowerTestFamily::TwoSamplePairedT);
    assert_eq!(pw.effect_mode, PowerEffectInputMode::Standardized);

    let mut doc_r9 = ProjectDocument::new("R9")?;
    doc_r9.data_sheets.push(DataSheet::new("s-w", "W", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 5)?))?);
    doc_r9.data_sheets.push(DataSheet::new("s-n", "N", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5)?))?);
    let mut rsdlg = RandomSampleRowsDialogState::default();
    assert!(rsdlg.open_for_sheet(&doc_r9, &SheetId::new("s-w")));
    rsdlg.stratify_column = Some(3);
    assert!(rsdlg.open_for_sheet(&doc_r9, &SheetId::new("s-n")) && rsdlg.stratify_column.is_none());

    let (mut dup_app, dup_sid) = seed_column_app(&[("A", &[1.0, 2.0])], ColumnDesignHint::UnpairedReplicates);
    (dup_app.canvas_dialogs.analyze.source_sheet_id, dup_app.canvas_dialogs.analyze.selected_entry_id) =
        (Some(dup_sid.clone()), Some("stats.descriptive".into()));
    dup_app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
        source_sheet_id: dup_sid,
        existing_result_ids: vec![SheetId::new("nope")],
        pending_entry_id: "stats.descriptive".into(),
    };
    let mut pdup = PendingUiIntents::new();
    pdup.push(UiIntent::ResolveDuplicateAnalysis { modify_existing_id: Some(SheetId::new("nope")) });
    assert!(
        pdup.commit_if_settled(&mut dup_app, false).is_err()
            && matches!(dup_app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. })
    );

    let mut doc_f = ProjectDocument::new("Fact")?;
    doc_f.data_sheets.push(DataSheet::new("g-sum", "S", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::MeanSdN, 2, 2)?))?);
    let mut app_f = UiApp::from_session(Session::from_document(doc_f));
    app_f.canvas_dialogs.edit_params.draft = Some(AnalysisSpec::new("r-f", "g-sum", AnalysisKind::FactorialAnova(FactorialAnovaSpec::default()))?);
    let mut pf = PendingUiIntents::new();
    pf.push(UiIntent::StatParams(StatParamsIntent::ToggleSchematic));
    let _ = pf.commit_if_settled(&mut app_f, false)?;
    assert!(!app_f.canvas_dialogs.edit_params.paired_or_matched);

    // 2. AUD-09-03/09/28: PowerStudio clipboard adapter, narrative_copied reset, unavailable clipboard, sync_anova_group_means, solve_target narrative.
    for avail in [true, false] {
        let mut papp = UiApp::new();
        let clip = Arc::new(if avail { StubPlatformClipboardAdapter::new() } else { StubPlatformClipboardAdapter::unavailable() });
        papp.set_clipboard_adapter(clip.clone());
        papp.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
        papp.canvas_dialogs.power_wizard.stage = PowerWizardStage::Results;
        let _ = papp.canvas_dialogs.power_wizard.recalculate();
        let tok = Tokens::from_theme_spec(&papp.theme, &papp.locale);
        let (ctx, mut pst, mut m) = (
            egui::Context::default(),
            PowerStudioEguiState { results_tab: PowerResultsSubTab::NarrativeReport, ..Default::default() },
            FrameInspectMeta::default(),
        );
        for _ in 0..2 {
            m = FrameInspectMeta::default();
            run_egui_pass_with_ctx(&ctx, egui::vec2(1100.0, 760.0), Default::default(), true, |c, _| {
                render_power_studio_modal(c, &papp, &mut pst, &tok, &mut m, &mut PendingUiIntents::new())
            })?;
        }
        let rect = m.find_by_widget_id("power.copy_narrative").ok_or("copy btn")?.clipped_rect;
        run_egui_pass_with_ctx(&ctx, egui::vec2(1100.0, 760.0), click_input(rect.center()), true, |c, _| {
            render_power_studio_modal(c, &papp, &mut pst, &tok, &mut FrameInspectMeta::default(), &mut PendingUiIntents::new())
        })?;
        assert_eq!(pst.narrative_copied, avail);
        assert!(!clip.captured_texts().is_empty());
        if avail {
            papp.canvas_dialogs.power_wizard.alpha = 0.01;
            let _ = papp.canvas_dialogs.power_wizard.recalculate();
            run_egui_pass_with_ctx(&ctx, egui::vec2(1100.0, 760.0), Default::default(), true, |c, _| {
                render_power_studio_modal(c, &papp, &mut pst, &tok, &mut FrameInspectMeta::default(), &mut PendingUiIntents::new())
            })?;
            assert!(!pst.narrative_copied);
        }
    }
    pw.set_family(PowerTestFamily::OneWayAnova);
    pw.effect_mode = PowerEffectInputMode::MeansAndSd;
    for k in [10, 3] {
        pw.anova_groups = k;
        sync_anova_group_means(&mut pw);
        assert_eq!(pw.group_means.len(), k);
    }
    pw.set_family(PowerTestFamily::TwoSampleUnpairedT);
    for (target, phrase) in [
        (PowerSolveTarget::SampleSize, "required sample size"),
        (PowerSolveTarget::Power, "achieved statistical power is"),
        (PowerSolveTarget::DetectableEffect, "minimum detectable standardized effect"),
    ] {
        pw.solve_target = target;
        let _ = pw.recalculate();
        assert!(build_narrative_paragraph(&pw, pw.result.as_ref().ok_or("pw res")?).contains(phrase));
    }
    Ok(())
}

#[test]
fn aud_10_ext01_dose_response_and_log_x_warning_banner() -> BoxRes {
    use crate::helpers::{make_commit_driver, seed_xy_app};

    // 3. EXT-01 + R20.20 #4: Raw vs log10 dose-response default & deferred commit until Parameters > OK.
    let (raw_xs, log_xs, ys) = ([0.01, 0.1, 1.0, 10.0, 100.0, 1000.0], [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0], [5.0, 12.0, 48.0, 88.0, 97.0, 99.0]);
    for (xs, expected_eq) in [(raw_xs.as_slice(), "agonist_vs_response_4pl"), (log_xs.as_slice(), "log_agonist_vs_response_4pl")] {
        let (mut a, s) = seed_xy_app(xs, &ys);
        assert!(a.canvas_dialogs.open_analyze_dialog(&a.document().clone(), &s));
        a.canvas_dialogs.analyze.select_catalog_entry("fit.nonlinear");
        let mut d = make_commit_driver(a);
        d.click_widget_id("modal.ok")?;
        assert!(d.state().app.document().result_sheets.is_empty(), "R20.20 #4: Analyze OK must not pre-commit ResultSheet");
        assert!(d.state().app.canvas_dialogs.edit_params.pending_create);
        if expected_eq == "agonist_vs_response_4pl" {
            assert!(d.frame_meta().find_by_widget_id("fit.warning.log_x_scale").is_none());
        }
        d.click_widget_id("modal.ok")?;
        assert!(
            matches!(&d.state().app.document().result_sheets.last().ok_or("rs")?.analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.equation_id == expected_eq)
        );
    }
    let (mut aw, sw) = seed_xy_app(&raw_xs, &ys);
    let rsw = SheetId::new(
        aw.execute_command(
            "analysis.run",
            json!({ "source_sheet_id": sw.as_str(), "kind": "nonlinear_regression", "equation_id": "log_agonist_vs_response_4pl" }),
        )?
        .payload["sheet_id"]
            .as_str()
            .ok_or("id")?,
    );
    assert!(aw.canvas_dialogs.open_edit_parameters(&aw.document().clone(), &rsw));
    let mut dw = make_commit_driver(aw);
    assert!(dw.frame_meta().find_by_widget_id("fit.warning.log_x_scale").is_some());
    dw.click_widget_id("fit.warning.switch_non_log")?;
    assert!(dw.frame_meta().find_by_widget_id("fit.warning.log_x_scale").is_none());
    assert!(
        matches!(&dw.state().app.canvas_dialogs.edit_params.draft.as_ref().ok_or("draft")?.kind, AnalysisKind::NonlinearRegression(nl) if nl.equation_id == "agonist_vs_response_4pl")
    );
    Ok(())
}

#[test]
fn aud_10_02_04_42_and_13_15_25_factor_and_fit_modal_regressions() -> BoxRes {
    use crate::helpers::{make_commit_driver, seed_mv_app, seed_xy_app};
    use curvebook_doc::{FactorialAnovaSpec, ModelComparisonMethod, ModelComparisonQuestion, ModelComparisonSpec, ParameterRule, ParameterScope};

    let (log_xs, ys) = ([-9.0, -8.0, -7.0, -6.0, -5.0, -4.0], [5.0, 12.0, 48.0, 88.0, 97.0, 99.0]);

    // 4. AUD-10-02/04/42 & AUD-13-15: 25-factor formula cap, Fit Compare/Custom DSL/summary_param.
    let (mut a25, s25) = seed_mv_app(26, 6);
    let sp25 = AnalysisSpec::new(
        "res-25",
        s25,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            response_column: Some(25),
            multifactor_columns: (0..25).collect(),
            max_interaction_order: 25,
            ..Default::default()
        }),
    )?;
    (a25.canvas_dialogs.edit_params.result_sheet_id, a25.canvas_dialogs.edit_params.draft, a25.canvas_dialogs.active_dialog) =
        (Some(sp25.id.clone()), Some(sp25.clone()), ActiveDialog::EditAnalysisParameters { result_sheet_id: sp25.id });
    let d25 = make_commit_driver(a25);
    let fb = d25.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("formula_bar")?;
    assert!(fb.value.as_ref().and_then(|v| v.as_str()).unwrap_or(&fb.label).contains("25"));

    let (mut af, sf) = seed_xy_app(&log_xs, &ys);
    let rsf = SheetId::new(
        af.execute_command(
            "analysis.run",
            json!({ "source_sheet_id": sf.as_str(), "kind": "nonlinear_regression", "equation_id": "log_agonist_vs_response_4pl" }),
        )?
        .payload["sheet_id"]
            .as_str()
            .ok_or("id")?,
    );
    assert!(af.canvas_dialogs.open_edit_parameters(&af.document().clone(), &rsf));
    if let Some(d) = af.canvas_dialogs.edit_params.draft.as_mut()
        && let AnalysisKind::NonlinearRegression(nl) = &mut d.kind
    {
        nl.comparison = Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "straight_line".into() });
        nl.comparison_question = Some(ModelComparisonQuestion::AlternativeEquation { alternative_equation_id: "straight_line".into() });
        nl.comparison_method = Some(ModelComparisonMethod::ExtraSumOfSquaresF);
        nl.parameter_rules.push(ParameterRule {
            name: "Bottom".into(),
            scope: ParameterScope::Constant(0.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        });
    }
    let mut df = make_commit_driver(af);
    df.click_widget_id("fit.tab.compare")?;
    df.click_widget_id("fit.cmp.aicc")?;
    assert!(
        matches!(&df.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind, AnalysisKind::NonlinearRegression(nl) if nl.comparison_method == Some(ModelComparisonMethod::Aicc))
    );
    df.click_widget_id("fit.cmp.none")?;
    assert!(
        matches!(&df.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind, AnalysisKind::NonlinearRegression(nl) if nl.comparison.is_none() && nl.comparison_question.is_none() && nl.comparison_method.is_none())
    );
    for wid in ["fit.tab.model", "fit.eq.custom_toggle"] {
        df.click_widget_id(wid)?;
    }
    assert!(
        matches!(&df.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind, AnalysisKind::NonlinearRegression(nl) if nl.parameter_rules.is_empty())
    );
    for wid in ["fit.eq.mode.builtin", "fit.tab.output"] {
        df.click_widget_id(wid)?;
    }
    let sc = df.state().dialog_rect_pt.ok_or("rect")?.center();
    for target_wid in ["fit.out.create_summary_graph", "fit.out.summary_param.LogEC50"] {
        for _ in 0..4 {
            if df.frame_meta().find_by_widget_id(target_wid).is_some() {
                break;
            }
            df.wheel_at(sc, egui::vec2(0.0, -180.0))?;
        }
        df.click_widget_id(target_wid)?;
    }
    assert!(
        matches!(&df.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind, AnalysisKind::NonlinearRegression(nl) if nl.summary_parameter.as_deref() == Some("LogEC50"))
    );
    Ok(())
}

#[test]
fn aud_10_analyze_picker_dataset_category_pstack_and_pcr_regressions() -> BoxRes {
    use crate::common::set_textbox;
    use crate::helpers::{make_commit_driver, seed_column_app, seed_mv_app};
    use curvebook_ui_egui::dialogs_canvas::AnalysisCategory;

    // 5. R20.19 + AUD-10-06/07/23/24: Analyze picker named Column datasets (`analyze.dataset.*`), empty MV guard, category preserves search, P-Value Stack dropdown & column cycle, PCR leakage disable.
    let (mut ac, sc_id) = seed_column_app(
        &[("Vehicle", &[0.01, 0.04]), ("Low Dose", &[0.02, 0.05]), ("High Dose", &[0.03, 0.06])],
        ColumnDesignHint::UnpairedReplicates,
    );
    assert!(ac.canvas_dialogs.open_analyze_dialog(&ac.document().clone(), &sc_id));
    assert!(ac.canvas_dialogs.analyze.variable_candidates.is_empty());
    let mut dc = make_commit_driver(ac);
    assert!(dc.frame_meta().find_by_widget_id("analyze.mv.var.0").is_none());
    let ds0 = dc.frame_meta().find_by_widget_id("analyze.dataset.0").ok_or("analyze.dataset.0")?;
    assert_eq!((ds0.label.as_str(), ds0.value.as_ref()), ("Vehicle", Some(&json!(true))));
    dc.click_widget_id("analyze.dataset.0")?;
    assert_eq!(dc.state().app.canvas_dialogs.analyze.selected_data_sets, vec![1, 2]);
    set_textbox(&mut dc, "analyze.search", "t-test")?;
    dc.click_widget_id("analyze.cat.column")?;
    assert_eq!(
        (dc.state().app.canvas_dialogs.analyze.category_filter, dc.state().app.canvas_dialogs.analyze.search_query.as_str()),
        (Some(AnalysisCategory::Column), "t-test")
    );
    for wid in ["analyze.cat.all", "analyze.search.clear"] {
        dc.click_widget_id(wid)?;
    }
    set_textbox(&mut dc, "analyze.search", "stack")?;
    dc.click_widget_id("analyze.entry.stats.pvalue_stack")?;
    let m0 = dc.state().app.canvas_dialogs.analyze.pvalue_stack_method;
    dc.click_widget_id("analyze.pvalue_stack.method")?;
    dc.click_widget_id("analyze.pvalue_stack.method")?;
    assert_eq!(dc.state().app.canvas_dialogs.analyze.pvalue_stack_method, m0);
    for exp in [Some(0), Some(1), Some(2), None] {
        dc.click_widget_id("analyze.pvalue_stack.column")?;
        assert_eq!(dc.state().app.canvas_dialogs.analyze.pvalue_stack_column, exp);
    }

    let (mut amv, smv) = seed_mv_app(4, 6);
    assert!(amv.canvas_dialogs.open_analyze_dialog(&amv.document().clone(), &smv));
    let mut dmv = make_commit_driver(amv);
    let mv0 = dmv.frame_meta().find_by_widget_id("analyze.mv.var.0").ok_or("analyze.mv.var.0")?;
    assert_eq!((mv0.label.as_str(), mv0.value.as_ref()), ("V1", Some(&json!(true))));
    dmv.click_widget_id("analyze.mv.var.0")?;
    dmv.click_widget_id("analyze.mv.var.1")?;
    dmv.click_widget_id("analyze.mv.var.2")?;
    assert_eq!(dmv.state().app.canvas_dialogs.analyze.selected_data_sets, vec![3]);
    dmv.click_widget_id("analyze.mv.var.3")?;
    assert_eq!(dmv.state().app.canvas_dialogs.analyze.selected_data_sets, vec![3]);
    assert_eq!(dmv.frame_meta().find_by_widget_id("analyze.mv.var.3").and_then(|e| e.value.clone()), Some(json!(true)));
    set_textbox(&mut dmv, "analyze.search", "PCR")?;
    dmv.click_widget_id("analyze.entry.stats.pcr")?;
    dmv.click_widget_id("analyze.pca.pred.1")?;
    assert_eq!(
        (&dmv.state().app.canvas_dialogs.analyze.pca_predictors, &dmv.state().app.canvas_dialogs.analyze.selected_data_sets),
        (&vec![1], &vec![1])
    );
    assert!(
        !dmv.frame_meta().find_by_widget_id("analyze.pcr.outcome.1").unwrap().enabled
            && dmv.frame_meta().find_by_widget_id("analyze.pcr.outcome.2").unwrap().enabled
    );
    Ok(())
}

#[test]
fn aud_10_stat_params_locale_cancel_cox_multirow_and_mi_regressions() -> BoxRes {
    use crate::common::{set_textbox, sync_app_to_driver};
    use crate::helpers::{make_commit_driver, seed_column_app, seed_mv_app, seed_xy_app};
    use curvebook_doc::{CoxTieMethod, MultipleRowTestsSpec, MultivariateSpec};

    // 6. AUD-10-09/10/26/27/30/51: German decimal + cancel-reopen, Cox PH swap, MultiRow, MI clamp.
    let (mut al, sl) = seed_xy_app(&[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 4.0, 6.0, 8.0, 10.0]);
    al.set_locale("de");
    let rsl = SheetId::new(
        al.execute_command("analysis.run", json!({ "source_sheet_id": sl.as_str(), "kind": "linear_regression" }))?.payload["sheet_id"]
            .as_str()
            .ok_or("id")?,
    );
    assert!(al.canvas_dialogs.open_edit_parameters(&al.document().clone(), &rsl));
    let mut dl = make_commit_driver(al);
    dl.click_widget_id("edit.lin.fixed_point")?;
    set_textbox(&mut dl, "edit.lin.fixed_point_x0", "1,5")?;
    assert!(dl.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    set_textbox(&mut dl, "edit.lin.fixed_point_x0", "bad")?;
    assert!(!dl.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    set_textbox(&mut dl, "edit.lin.fixed_point_x0", "2,5")?;
    for wid in ["edit.tab.options", "edit.conf.99", "edit.tab.diagnostics"] {
        dl.click_widget_id(wid)?;
    }
    assert!(dl.frame_meta().find_by_widget_id("edit.lin.conf_bands").unwrap().label.contains("99%"));
    for wid in ["edit.tab.experimental_design", "edit.lin.deming", "edit.lin.deming.sds"] {
        dl.click_widget_id(wid)?;
    }
    set_textbox(&mut dl, "edit.lin.deming.sd_x", "-5")?;
    assert!(!dl.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    dl.click_widget_id("modal.cancel")?;
    let doc_r = dl.state().app.document().clone();
    assert!(dl.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_r, &rsl));
    sync_app_to_driver(&mut dl);
    for wid in ["edit.lin.deming", "edit.lin.deming.sds"] {
        dl.click_widget_id(wid)?;
    }
    assert!(dl.state().views.stats_modals.numeric_field_errors.is_empty() && dl.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);

    let (mut acox, scox) = seed_mv_app(4, 6);
    let sp_cox = AnalysisSpec::new(
        "rcox",
        scox,
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 1,
            status_column: 2,
            predictor_columns: vec![3],
            reference_levels: vec![],
            tie_method: CoxTieMethod::Efron,
            formula: None,
            covariate_profiles: Vec::new(),
            nested_comparison: None,
            emit_cox_snell_residuals: false,
            ci_method: Default::default(),
            event_code: None,
            censored_code: None,
            treat_other_as: Default::default(),
        }),
    )?;
    (acox.canvas_dialogs.edit_params.result_sheet_id, acox.canvas_dialogs.edit_params.draft, acox.canvas_dialogs.active_dialog) =
        (Some(sp_cox.id.clone()), Some(sp_cox.clone()), ActiveDialog::EditAnalysisParameters { result_sheet_id: sp_cox.id });
    let mut dcox = make_commit_driver(acox);
    for wid in ["edit.tab.data", "edit.cox.time.3", "edit.cox.time.2"] {
        dcox.click_widget_id(wid)?;
    }
    assert!(
        matches!(&dcox.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column: 2, status_column, predictor_columns, .. }) if *status_column != 2 && !predictor_columns.contains(&3))
    );

    let (mut art, srt) = seed_column_app(&[("A", &[1.0, 2.0]), ("B", &[3.0, 4.0])], ColumnDesignHint::UnpairedReplicates);
    let sp_rt = AnalysisSpec::new("rrt", srt, AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default()))?;
    (art.canvas_dialogs.edit_params.result_sheet_id, art.canvas_dialogs.edit_params.draft, art.canvas_dialogs.active_dialog) =
        (Some(sp_rt.id.clone()), Some(sp_rt.clone()), ActiveDialog::EditAnalysisParameters { result_sheet_id: sp_rt.id });
    let mut drt = make_commit_driver(art);
    drt.click_widget_id("edit.tab.multiple_comparisons")?;
    assert!(
        !drt.frame_meta().find_by_widget_id("edit.post_hoc.tukey").unwrap().enabled
            && !drt.frame_meta().find_by_widget_id("edit.post_hoc.dunnett").unwrap().enabled
    );

    let (mut ami, smi) = seed_mv_app(3, 6);
    let sp_mi = AnalysisSpec::new(
        "rmi",
        smi,
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 5,
            max_iters: 100,
            seed: 42,
            response_column: None,
            predictor_columns: vec![],
        }),
    )?;
    (ami.canvas_dialogs.edit_params.result_sheet_id, ami.canvas_dialogs.edit_params.draft, ami.canvas_dialogs.active_dialog) =
        (Some(sp_mi.id.clone()), Some(sp_mi.clone()), ActiveDialog::EditAnalysisParameters { result_sheet_id: sp_mi.id });
    let mut dmi = make_commit_driver(ami);
    for wid in ["edit.tab.experimental_design", "edit.mi.iters.inc"] {
        dmi.click_widget_id(wid)?;
    }
    assert!(matches!(
        &dmi.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind,
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { max_iters: 100, .. })
    ));
    Ok(())
}

#[test]
fn aud_10_r9_defaults_spline_viewer_and_contingency_ci_regressions() -> BoxRes {
    use crate::common::sync_app_to_driver;
    use crate::helpers::{make_commit_driver, seed_mv_app, seed_xy_app};
    use curvebook_doc::{ContingencyMethod, ContingencySpec, KappaWeighting, ResultStatus, SplineLowessMethod, SplineLowessSpec};
    use curvebook_ui_egui::egui_shell::UiIntent;

    let mut arg = UiApp::new();
    arg.canvas_dialogs.active_dialog = ActiveDialog::RandomizeGroups;
    let mut drg = make_commit_driver(arg);
    for wid in ["rand_groups.sch.block", "rand_groups.p3", "rand_groups.block.inc", "rand_groups.block.dec", "rand_groups.block.dec"] {
        drg.click_widget_id(wid)?;
    }
    assert_eq!(drg.state().app.canvas_dialogs.randomize_groups.block_sizes, vec![3]);

    let mut doc_rs = ProjectDocument::new("RS")?;
    doc_rs.data_sheets.push(DataSheet::new("s1", "Big", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 20)?))?);
    doc_rs.data_sheets.push(DataSheet::new("s2", "Sm", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?))?);
    let mut ars = UiApp::from_session(Session::from_document(doc_rs));
    (
        ars.canvas_dialogs.random_sample_rows.source_sheet_id,
        ars.canvas_dialogs.random_sample_rows.stratify_column,
        ars.canvas_dialogs.random_sample_rows.proportional_allocation,
        ars.canvas_dialogs.random_sample_rows.sample_size,
    ) = (Some(SheetId::new("s1")), Some(3), true, 15);
    ars.canvas_dialogs.active_dialog = ActiveDialog::RandomSampleRows { source_sheet_id: Some(SheetId::new("s1")) };
    let mut drs = make_commit_driver(ars);
    drs.click_widget_id("rand_sample.src.1")?;
    let rst = &drs.state().app.canvas_dialogs.random_sample_rows;
    assert!(rst.source_sheet_id == Some(SheetId::new("s2")) && rst.stratify_column.is_none() && !rst.proportional_allocation && rst.sample_size <= 3);

    let (mut adef, sdef) = seed_mv_app(3, 6);
    adef.execute_command("analysis.run", json!({ "sheet_id": "rpca", "source_sheet_id": sdef.as_str(), "kind": "pca" }))?;
    adef.execute_command("analysis.run", json!({ "sheet_id": "rcor", "source_sheet_id": sdef.as_str(), "kind": "correlation_matrix" }))?;
    assert!(adef.canvas_dialogs.open_edit_parameters(&adef.document().clone(), &SheetId::new("rpca")));
    let mut ddef = make_commit_driver(adef);
    for wid in ["dialog.button.save_default", "modal.cancel"] {
        ddef.click_widget_id(wid)?;
    }
    let dsnap = ddef.state().app.document().clone();
    assert!(ddef.state_mut().app.canvas_dialogs.open_edit_parameters(&dsnap, &SheetId::new("rcor")));
    sync_app_to_driver(&mut ddef);
    ddef.click_widget_id("dialog.button.save_default")?;
    assert_eq!(ddef.state().app.egui_views.stats_modals.saved_defaults.len(), 2);

    let (mut asp, ssp) = seed_xy_app(&[0.0, 1.0, 2.0, 3.0, 4.0], &[0.0, 1.0, 4.0, 9.0, 16.0]);
    let spsp = AnalysisSpec::new(
        "rsp",
        ssp,
        AnalysisKind::FitSplineLowess(SplineLowessSpec {
            method: SplineLowessMethod::SmoothingSpline,
            smoothing_df: Some(6.0),
            smoothing_knots: None,
            ..Default::default()
        }),
    )?;
    (asp.canvas_dialogs.edit_params.result_sheet_id, asp.canvas_dialogs.edit_params.draft, asp.canvas_dialogs.active_dialog) =
        (Some(spsp.id.clone()), Some(spsp.clone()), ActiveDialog::EditAnalysisParameters { result_sheet_id: spsp.id });
    let mut dsp = make_commit_driver(asp);
    assert!(dsp.frame_meta().find_by_widget_id("edit.dataset.0").is_some());
    dsp.click_widget_id("edit.spline.knots.inc")?;
    assert!(
        matches!(&dsp.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind, AnalysisKind::FitSplineLowess(s) if s.smoothing_df == Some(7.0))
    );

    let mut doc_rv = ProjectDocument::new("RV")?;
    let mut rrv = ResultSheet::new(AnalysisSpec::new("rv-1", "src", AnalysisKind::LinearRegression(Default::default()))?)?;
    rrv.status = ResultStatus::UpToDate;
    rrv.tabs = vec![
        ResultTab {
            id: "t1".into(),
            title: "T1".into(),
            kind: ResultTabKind::MainSummary,
            column_headers: vec!["V".into()],
            row_labels: vec!["R0".into()],
            rows: vec![vec![ResultCell::Number { value: 1.0, units: None, prefix: None }]],
            footnotes: vec![],
        },
        ResultTab {
            id: "t2".into(),
            title: "T2".into(),
            kind: ResultTabKind::MainSummary,
            column_headers: vec!["V".into()],
            row_labels: vec!["R0".into()],
            rows: vec![vec![ResultCell::Number { value: 99.0, units: None, prefix: None }]],
            footnotes: vec![],
        },
    ];
    doc_rv.result_sheets.push(rrv);
    let mut arv = UiApp::from_session(Session::from_document(doc_rv));
    arv.canvas_dialogs.result_view.active_tab_index = 99;
    arv.canvas_dialogs.active_dialog = ActiveDialog::ResultViewer { result_sheet_id: SheetId::new("rv-1") };
    assert_eq!(make_commit_driver(arv).frame_meta().find_by_widget_id("result.tab.1").ok_or("t1")?.value, Some(json!(true)));

    // Contingency variants + Woolf unsupported CI alert + cycle back
    let mut doc_cg = ProjectDocument::new("CG")?;
    let mut ct = ContingencyTable::new(2, 2)?;
    for (r, (a, b)) in [(12.0, 4.0), (5.0, 14.0)].into_iter().enumerate() {
        (ct.data_sets[0].cells[r][0], ct.data_sets[1].cells[r][0]) = (Cell::number(a), Cell::number(b));
    }
    doc_cg.add_data_sheet(DataSheet::new("cont-1", "2x2", TableArchetype::Contingency(ct))?)?;
    doc_cg.add_result_sheet(ResultSheet::new(AnalysisSpec::new(
        "res-cont",
        "cont-1",
        AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() }),
    )?)?)?;
    let mut acg = UiApp::from_session(Session::from_document(doc_cg));
    assert!(acg.canvas_dialogs.open_edit_parameters(&acg.document().clone(), &SheetId::new("res-cont")));
    let mut dcg = make_commit_driver(acg);
    for wid in ["edit.cont.g2", "edit.cont.barnard", "edit.cont.fleiss_kappa", "edit.cont.exact_binomial"] {
        assert!(dcg.frame_meta().find_by_widget_id(wid).is_some());
    }
    for wid in ["edit.cont.kappa", "edit.cont.kappa.linear"] {
        dcg.click_widget_id(wid)?;
    }
    assert!(
        matches!(&dcg.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind, AnalysisKind::Contingency(c) if c.method == ContingencyMethod::CohenKappa { weighting: KappaWeighting::Linear })
    );
    dcg.click_widget_id("edit.contingency.ci.baptista_pike")?;
    let okn = dcg.frame_meta().find_by_widget_id("modal.ok").ok_or("ok")?;
    assert!(
        !okn.enabled
            && okn.disabled_reason.as_deref().is_some_and(|r| r.contains("Woolf"))
            && dcg.frame_meta().find_by_widget_id("edit.contingency.ci.error").is_some()
    );
    let mut ok_try = PendingUiIntents::new();
    ok_try.push(UiIntent::DialogButtonAction { action: "ok".into() });
    assert!(ok_try.commit_if_settled(&mut dcg.state_mut().app, false).unwrap_err().to_string().contains("Woolf"));
    dcg.click_widget_id("edit.contingency.ci.baptista_pike")?;
    assert!(
        dcg.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled && dcg.frame_meta().find_by_widget_id("edit.contingency.ci.error").is_none()
    );
    Ok(())
}

#[test]
fn r20_20_first_run_nonlinear_fit_cancel_and_ok_commit_gate() -> BoxRes {
    let (mut app, _, xy_id, _) = seed_rich_project_app()?;
    let seq0 = app.mutation_seq();
    let undo0 = app.session().undo_depth();
    let rs0 = app.document().result_sheets.len();

    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc_snap, &xy_id));
    let mut driver = make_driver(app, 1024.0, 640.0);
    driver.click_widget_id("analyze.search")?;
    driver.type_text("Nonlinear")?;
    driver.click_widget_id("analyze.entry.fit.nonlinear")?;
    driver.click_widget_id("modal.ok")?;
    if matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        driver.click_widget_id("analyze_picker.dup.create_new")?;
        driver.click_widget_id("modal.ok")?;
    }

    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiFitModal
    );
    assert_eq!(driver.state().app.document().result_sheets.len(), rs0);
    assert_eq!(driver.state().app.mutation_seq(), seq0);

    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(driver.state().app.document().result_sheets.len(), rs0);
    assert_eq!(driver.state().app.session().undo_depth(), undo0);

    let doc_snap2 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_analyze_dialog(&doc_snap2, &xy_id));
    driver.run_settled();
    driver.click_widget_id("analyze.search")?;
    driver.type_text("Nonlinear")?;
    driver.click_widget_id("analyze.entry.fit.nonlinear")?;
    driver.click_widget_id("modal.ok")?;
    if matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        driver.click_widget_id("analyze_picker.dup.create_new")?;
        driver.click_widget_id("modal.ok")?;
    }
    assert_eq!(driver.state().app.document().result_sheets.len(), rs0);

    driver.click_widget_id("fit.eq.search")?;
    driver.type_text("log_agonist_vs_response_5pl")?;
    driver.click_widget_id("fit.eq.item.log_agonist_vs_response_5pl")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(driver.state().app.document().result_sheets.len(), rs0 + 1);
    assert_eq!(driver.state().app.session().undo_depth(), undo0 + 1);
    let created = driver.state().app.document().result_sheets.last().ok_or("missing created fit")?;
    assert!(matches!(&created.analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.equation_id == "log_agonist_vs_response_5pl"));
    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod analyze_picker_and_fit_defer_tests {
    use curvebook_doc::{
        AnalysisKind, Cell, DataSheet, ParameterScope, ProjectDocument, ResultSheet, ResultStatus, SheetId, SubcolumnMode, TableArchetype,
        WeightingScheme, XyTable,
    };
    use curvebook_engine::Session;
    use curvebook_ui_egui::UiApp;
    use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
    use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use crate::common::{ShellTestState, set_textbox, sync_app_to_driver};
    use crate::helpers::make_commit_driver;

    fn open_named_xy_nonlinear_analyze_driver() -> EguiHumanPathDriver<'static, ShellTestState> {
        let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
        let series: [(&str, [f64; 6]); 3] = [
            ("Vehicle", [4.0, 10.0, 42.0, 84.0, 96.0, 99.0]),
            ("Agonist A", [5.0, 18.0, 58.0, 91.0, 98.0, 100.0]),
            ("Antagonist B", [3.0, 7.0, 22.0, 61.0, 89.0, 97.0]),
        ];
        let mut tbl = XyTable::new(SubcolumnMode::Single, series.len(), xs.len()).unwrap();
        tbl.x_title = String::from("Log[Conc]");
        for (r, &x) in xs.iter().enumerate() {
            tbl.x_cells[r][0] = Cell::number(x);
        }
        for (ci, (name, ys)) in series.iter().enumerate() {
            tbl.data_sets[ci].title = (*name).to_owned();
            for (r, &y) in ys.iter().enumerate() {
                tbl.data_sets[ci].cells[r][0] = Cell::number(y);
            }
        }
        let sid = SheetId::try_new("xy-named").unwrap();
        let mut doc = ProjectDocument::new("Named XY Analyze").unwrap();
        doc.data_sheets.push(DataSheet::new(sid.clone(), "Dose Curve", TableArchetype::Xy(tbl)).unwrap());
        doc.active_sheet_id = Some(sid.clone());
        let mut app = UiApp::from_session(Session::from_document(doc));
        app.sync_views();
        let doc_snap = app.document().clone();
        assert!(app.canvas_dialogs.open_analyze_dialog(&doc_snap, &sid));
        assert!(app.canvas_dialogs.analyze.variable_candidates.is_empty());
        let mut drv = make_commit_driver(app);
        set_textbox(&mut drv, "analyze.search", "Nonlinear").unwrap();
        drv.click_widget_id("analyze.entry.fit.nonlinear").unwrap();
        drv
    }

    fn assert_dataset_checked_states(drv: &EguiHumanPathDriver<'static, ShellTestState>, all_checked: bool, item_checked: [bool; 3]) {
        assert_eq!(drv.frame_meta().find_by_widget_id("analyze.dataset.all").and_then(|e| e.value.clone()), Some(json!(all_checked)));
        for (idx, expected) in item_checked.into_iter().enumerate() {
            assert_eq!(drv.frame_meta().find_by_widget_id(&format!("analyze.dataset.{idx}")).and_then(|e| e.value.clone()), Some(json!(expected)));
        }
    }

    #[test]
    fn r20_19_analyze_picker_named_xy_series_and_selection_semantics() {
        let mut drv_all = open_named_xy_nonlinear_analyze_driver();
        assert!(drv_all.frame_meta().find_by_widget_id("analyze.mv.var.0").is_none());
        assert!(
            !drv_all.frame_meta().entries.iter().any(|e| matches!(e.label.as_str(), "Col A" | "Col B" | "Col C")),
            "named XY Analyze picker must never substitute fake Col A..C labels"
        );
        for (idx, expected_title) in [(0, "Vehicle"), (1, "Agonist A"), (2, "Antagonist B")] {
            let entry = drv_all.frame_meta().find_by_widget_id(&format!("analyze.dataset.{idx}")).expect("dataset checkbox");
            assert_eq!(entry.label, expected_title);
        }
        assert_dataset_checked_states(&drv_all, true, [true, true, true]);
        drv_all.click_widget_id("modal.ok").unwrap();
        drv_all.run_settled();
        assert!(drv_all.state().app.document().result_sheets.is_empty(), "must defer ResultSheet creation until Parameters > OK");
        drv_all.click_widget_id("modal.ok").unwrap();
        drv_all.run_settled();
        let rs_all = drv_all.state().app.document().result_sheets.last().expect("default nonlinear fit sheet");
        assert!(matches!(rs_all.status, ResultStatus::UpToDate));
        assert!(rs_all.analysis.selected_data_sets.is_empty(), "default All commits empty selected_data_sets");

        let mut drv_multi = open_named_xy_nonlinear_analyze_driver();
        drv_multi.click_widget_id("analyze.dataset.1").unwrap();
        assert_eq!(drv_multi.state().app.canvas_dialogs.analyze.selected_data_sets, vec![0, 2]);
        assert_dataset_checked_states(&drv_multi, false, [true, false, true]);
        drv_multi.click_widget_id("modal.ok").unwrap();
        drv_multi.run_settled();
        assert!(drv_multi.state().app.document().result_sheets.is_empty(), "must preserve selected_data_sets through deferred Parameters modal");
        drv_multi.click_widget_id("modal.ok").unwrap();
        drv_multi.run_settled();
        let rs_multi = drv_multi.state().app.document().result_sheets.last().expect("multi-series fit sheet");
        assert!(matches!(rs_multi.status, ResultStatus::UpToDate));
        assert_eq!(rs_multi.analysis.selected_data_sets, vec![0, 2]);

        let mut drv_one = open_named_xy_nonlinear_analyze_driver();
        drv_one.click_widget_id("analyze.dataset.0").unwrap();
        drv_one.click_widget_id("analyze.dataset.2").unwrap();
        assert_eq!(drv_one.state().app.canvas_dialogs.analyze.selected_data_sets, vec![1]);

        drv_one.click_widget_id("analyze.dataset.1").unwrap();
        assert_eq!(drv_one.state().app.canvas_dialogs.analyze.selected_data_sets, vec![1]);
        assert_dataset_checked_states(&drv_one, false, [false, true, false]);

        drv_one.click_widget_id("modal.ok").unwrap();
        drv_one.run_settled();
        assert!(drv_one.state().app.document().result_sheets.is_empty());
        drv_one.click_widget_id("modal.ok").unwrap();
        drv_one.run_settled();
        let rs_one = drv_one.state().app.document().result_sheets.last().expect("single-series fit sheet");
        assert!(matches!(rs_one.status, ResultStatus::UpToDate));
        assert_eq!(rs_one.analysis.selected_data_sets, vec![1]);
    }

    #[test]
    fn r20_20_item4_first_run_nonlinear_cancel_and_esc_discard_draft() {
        let mut drv = open_named_xy_nonlinear_analyze_driver();
        let base_seq = drv.state().app.mutation_seq();
        let base_undo = drv.state().app.session().undo_depth();
        let base_dirty = drv.state().app.session().is_dirty();
        let base_active = drv.state().app.document().active_sheet_id.clone();

        drv.click_widget_id("analyze.dataset.1").unwrap();
        drv.click_widget_id("modal.ok").unwrap();
        drv.run_settled();

        assert!(matches!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
        assert!(drv.state().app.canvas_dialogs.edit_params.pending_create);
        assert_eq!(drv.state().app.document().result_sheets.len(), 0);
        assert_eq!(drv.state().app.document().graph_sheets.len(), 0);
        assert_eq!(drv.state().app.mutation_seq(), base_seq);
        assert_eq!(drv.state().app.session().undo_depth(), base_undo);
        assert_eq!(drv.state().app.session().is_dirty(), base_dirty);
        assert!(drv.frame_meta().find_by_widget_id("modal.apply").is_none(), "create mode must hide Apply");

        set_textbox(&mut drv, "fit.eq.search", "michaelis").unwrap();
        drv.click_widget_id("fit.eq.item.michaelis_menten").unwrap();
        drv.click_widget_id("fit.tab.weight").unwrap();
        drv.click_widget_id("fit.wt.rel_sq").unwrap();
        drv.click_widget_id("modal.cancel").unwrap();
        drv.run_settled();

        assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
        assert!(!drv.state().app.canvas_dialogs.edit_params.pending_create);
        assert!(drv.state().app.canvas_dialogs.edit_params.draft.is_none());
        assert_eq!(drv.state().app.document().result_sheets.len(), 0);
        assert_eq!(drv.state().app.document().graph_sheets.len(), 0);
        assert_eq!(drv.state().app.mutation_seq(), base_seq);
        assert_eq!(drv.state().app.session().undo_depth(), base_undo);
        assert_eq!(drv.state().app.session().is_dirty(), base_dirty);
        assert_eq!(drv.state().app.document().active_sheet_id, base_active);

        let doc_reopen = drv.state().app.document().clone();
        let src_sid = base_active.unwrap();
        assert!(drv.state_mut().app.canvas_dialogs.open_analyze_dialog(&doc_reopen, &src_sid));
        sync_app_to_driver(&mut drv);
        set_textbox(&mut drv, "analyze.search", "Nonlinear").unwrap();
        drv.click_widget_id("analyze.entry.fit.nonlinear").unwrap();
        drv.click_widget_id("modal.ok").unwrap();
        drv.run_settled();
        assert!(matches!(
            &drv.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|s| &s.kind),
            Some(AnalysisKind::NonlinearRegression(nl))
                if nl.equation_id == "log_agonist_vs_response_4pl" && nl.weighting == WeightingScheme::None
        ));
        drv.press_egui_key(Key::Escape, Modifiers::NONE);
        drv.run_settled();
        assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
        assert_eq!(drv.state().app.document().result_sheets.len(), 0);
        assert_eq!(drv.state().app.mutation_seq(), base_seq);
        assert_eq!(drv.state().app.session().undo_depth(), base_undo);
    }

    fn open_mm_enzyme_analyze_driver() -> (EguiHumanPathDriver<'static, ShellTestState>, SheetId) {
        let mut tbl_mm = XyTable::new(SubcolumnMode::Single, 3, 6).unwrap();
        tbl_mm.x_title = String::from("[Substrate] (uM)");
        let mm_xs = [0.5, 1.0, 2.5, 5.0, 10.0, 20.0];
        let mm_series = [("Vehicle", 120.0, 3.0), ("Modulator A", 95.0, 4.0), ("Modulator B", 140.0, 5.0)];
        for (r, &x) in mm_xs.iter().enumerate() {
            tbl_mm.x_cells[r][0] = Cell::number(x);
            for (ci, &(title, vmax, km)) in mm_series.iter().enumerate() {
                tbl_mm.data_sets[ci].title = title.to_owned();
                tbl_mm.data_sets[ci].cells[r][0] = Cell::number(vmax * x / (km + x));
            }
        }
        let mm_sid = SheetId::try_new("xy-mm").unwrap();
        let mut doc_mm = ProjectDocument::new("Enzyme Kinetics").unwrap();
        doc_mm.data_sheets.push(DataSheet::new(mm_sid.clone(), "Enzyme Curve", TableArchetype::Xy(tbl_mm)).unwrap());
        doc_mm.active_sheet_id = Some(mm_sid.clone());
        let mut app_mm = UiApp::from_session(Session::from_document(doc_mm));
        app_mm.sync_views();
        let doc_mm_snap = app_mm.document().clone();
        assert!(app_mm.canvas_dialogs.open_analyze_dialog(&doc_mm_snap, &mm_sid));
        (make_commit_driver(app_mm), mm_sid)
    }

    fn commit_mm_enzyme_fit(drv: &mut EguiHumanPathDriver<'static, ShellTestState>) -> ResultSheet {
        set_textbox(drv, "analyze.search", "Nonlinear").unwrap();
        drv.click_widget_id("analyze.entry.fit.nonlinear").unwrap();
        drv.click_widget_id("analyze.dataset.1").unwrap();
        drv.click_widget_id("modal.ok").unwrap();
        drv.run_settled();
        assert_eq!(drv.state().app.document().result_sheets.len(), 0);

        set_textbox(drv, "fit.eq.search", "michaelis").unwrap();
        drv.click_widget_id("fit.eq.item.michaelis_menten").unwrap();
        drv.click_widget_id("fit.tab.weight").unwrap();
        drv.click_widget_id("fit.wt.rel_sq").unwrap();
        drv.click_widget_id("fit.tab.constraints").unwrap();
        drv.click_widget_id("fit.con.rule.Vmax.fixed").unwrap();
        set_textbox(drv, "fit.con.const.Vmax", "120").unwrap();
        drv.click_widget_id("modal.ok").unwrap();
        drv.run_settled();

        assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None, "last_intent_error={:?}", drv.state().last_intent_error);
        assert!(!drv.state().app.canvas_dialogs.edit_params.pending_create);
        assert_eq!(drv.state().app.document().result_sheets.len(), 1);
        drv.state().app.document().result_sheets.first().unwrap().clone()
    }

    #[test]
    fn r20_20_item4_first_run_nonlinear_ok_commits_once_and_single_undo_reverts() {
        let (mut drv, _) = open_mm_enzyme_analyze_driver();
        let base_undo = drv.state().app.session().undo_depth();
        let created = commit_mm_enzyme_fit(&mut drv);

        assert_eq!(drv.state().app.session().undo_depth(), base_undo + 1);
        assert_eq!(drv.state().app.document().active_sheet_id.as_ref(), Some(&created.id));
        assert!(matches!(created.status, ResultStatus::UpToDate));
        assert_eq!(created.analysis.selected_data_sets, vec![0, 2]);
        let AnalysisKind::NonlinearRegression(created_nl) = &created.analysis.kind else {
            panic!("expected NonlinearRegression");
        };
        assert_eq!(created_nl.equation_id, "michaelis_menten");
        assert_eq!(created_nl.weighting, WeightingScheme::RelativeSquare);
        assert!(
            created_nl.parameter_rules.iter().any(|r| r.name == "Vmax" && matches!(r.scope, ParameterScope::Constant(v) if (v - 120.0).abs() < 1e-9))
        );

        drv.state_mut().app.execute_command("edit.undo", json!({})).unwrap();
        assert_eq!(drv.state().app.document().result_sheets.len(), 0);
        assert_eq!(drv.state().app.session().undo_depth(), base_undo);
    }

    #[test]
    fn r20_20_item4_existing_fit_edit_in_place_and_duplicate_create_new_defer() {
        let (mut drv, mm_sid) = open_mm_enzyme_analyze_driver();
        let created = commit_mm_enzyme_fit(&mut drv);

        let seq_before_edit = drv.state().app.mutation_seq();
        let doc_edit = drv.state().app.document().clone();
        assert!(drv.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_edit, &created.id));
        sync_app_to_driver(&mut drv);
        assert!(drv.frame_meta().find_by_widget_id("modal.apply").is_some());
        drv.click_widget_id("fit.tab.weight").unwrap();
        drv.click_widget_id("fit.wt.none").unwrap();
        drv.click_widget_id("modal.cancel").unwrap();
        drv.run_settled();
        assert_eq!(drv.state().app.mutation_seq(), seq_before_edit);
        assert!(matches!(
            &drv.state().app.document().result_sheet(&created.id).unwrap().analysis.kind,
            AnalysisKind::NonlinearRegression(nl) if nl.equation_id == "michaelis_menten" && nl.weighting == WeightingScheme::RelativeSquare
        ));

        let doc_edit2 = drv.state().app.document().clone();
        assert!(drv.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_edit2, &created.id));
        sync_app_to_driver(&mut drv);
        set_textbox(&mut drv, "fit.eq.search", "agonist_vs_response_4pl").unwrap();
        drv.click_widget_id("fit.eq.item.agonist_vs_response_4pl").unwrap();
        drv.click_widget_id("modal.ok").unwrap();
        drv.run_settled();
        assert_eq!(drv.state().app.document().result_sheets.len(), 1);
        assert!(matches!(
            &drv.state().app.document().result_sheet(&created.id).unwrap().analysis.kind,
            AnalysisKind::NonlinearRegression(nl) if nl.equation_id == "agonist_vs_response_4pl"
        ));

        let doc_dup = drv.state().app.document().clone();
        assert!(drv.state_mut().app.canvas_dialogs.open_analyze_dialog(&doc_dup, &mm_sid));
        sync_app_to_driver(&mut drv);
        set_textbox(&mut drv, "analyze.search", "Nonlinear").unwrap();
        drv.click_widget_id("analyze.entry.fit.nonlinear").unwrap();
        drv.click_widget_id("modal.ok").unwrap();
        drv.run_settled();
        assert!(matches!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }));
        drv.click_widget_id("analyze_picker.dup.create_new").unwrap();
        drv.click_widget_id("modal.ok").unwrap();
        drv.run_settled();
        assert_eq!(drv.state().app.document().result_sheets.len(), 1);
        drv.click_widget_id("modal.cancel").unwrap();
        drv.run_settled();
        assert_eq!(drv.state().app.document().result_sheets.len(), 1);
    }

    fn draft_two_group_method(
        drv: &curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver<'_, crate::common::ShellTestState>,
    ) -> curvebook_doc::TwoGroupMethod {
        match &drv.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind {
            curvebook_doc::AnalysisKind::TwoGroupComparison(t) => t.method,
            other => panic!("expected TwoGroupComparison, got {other:?}"),
        }
    }

    fn verify_one_sample_ratio_validation_and_commit(
        drv: &mut curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver<'_, crate::common::ShellTestState>,
        tg_res_id: &curvebook_doc::SheetId,
    ) {
        use crate::common::set_textbox;
        use curvebook_doc::{AnalysisKind, ResultStatus, TwoGroupMethod};

        drv.click_widget_id("edit.tg.one_sample_ratio_t").unwrap();
        drv.run_settled();
        assert_eq!(draft_two_group_method(drv), TwoGroupMethod::OneSampleRatioT { hypothetical: 1.0 });
        for bad in ["0", "-2.5"] {
            set_textbox(drv, "edit.two_group.hypothetical", bad).unwrap();
            assert!(drv.frame_meta().find_by_widget_id("edit.two_group.hypothetical.error").is_some(), "expected error alert for `{bad}`");
            assert!(!drv.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled, "modal.ok must disable for `{bad}`");
        }
        drv.click_widget_id("edit.two_group.hypothetical.step.dec").unwrap();
        drv.run_settled();
        assert!(drv.frame_meta().find_by_widget_id("edit.two_group.hypothetical.error").is_none());
        let TwoGroupMethod::OneSampleRatioT { hypothetical: stepped } = draft_two_group_method(drv) else {
            panic!("expected OneSampleRatioT after step");
        };
        assert!(stepped > 0.0 && stepped < 1.0);
        set_textbox(drv, "edit.two_group.hypothetical", "15").unwrap();
        assert!(drv.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
        drv.click_widget_id("modal.ok").unwrap();
        drv.run_settled();
        let rs = drv.state().app.document().result_sheet(tg_res_id).unwrap();
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(
            match &rs.analysis.kind {
                AnalysisKind::TwoGroupComparison(t) => t.method,
                other => panic!("unexpected kind {other:?}"),
            },
            TwoGroupMethod::OneSampleRatioT { hypothetical: 15.0 }
        );
    }

    #[test]
    fn two_group_lognormal_welch_and_one_sample_ratio_ui_round_trip() {
        use crate::common::{ShellTestState, make_shell_driver_with_state, seed_column_doc};
        use curvebook_doc::{AnalysisKind, ColumnDesignHint, ResultStatus, SheetId, TwoGroupMethod, TwoGroupSpec};
        use curvebook_engine::Session;
        use curvebook_ui_egui::UiApp;
        use curvebook_ui_egui::egui_shell::Tokens;
        use curvebook_ui_egui::interaction::ViewportMetrics;
        use serde_json::json;

        let (doc, col_id) = seed_column_doc(
            "Two-Group Lognormal UI",
            "col-ln-tg",
            "Assay",
            ColumnDesignHint::UnpairedReplicates,
            &[("Ctrl", &[10.0, 14.0, 18.0, 25.0, 32.0]), ("Drug", &[22.0, 35.0, 50.0, 75.0, 110.0])],
        )
        .unwrap();
        let mut session = Session::from_document(doc);
        let tg_val = serde_json::to_value(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
        let _ = session.execute("analysis.run", json!({ "sheet_id": "res-tg-ln", "source_sheet_id": col_id.as_str(), "kind": tg_val })).unwrap();
        let tg_res_id = SheetId::try_new("res-tg-ln").unwrap();

        let mut app = UiApp::from_session(session);
        app.sync_views();
        let snap = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&snap, &tg_res_id));
        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let mut drv = make_shell_driver_with_state(
            ShellTestState::new(app).with_commit_all().with_merge_passes(),
            ViewportMetrics::new(1280.0, 820.0, 1.0),
            tokens,
        );

        drv.click_widget_id("edit.tg.lognormal_welch").unwrap();
        drv.run_settled();
        assert_eq!(draft_two_group_method(&drv), TwoGroupMethod::LognormalWelchT);
        assert!(drv.state().app.canvas_dialogs.edit_params.welch_correction);
        assert!(!drv.frame_meta().find_by_widget_id("edit.tab.diagnostics").unwrap().enabled);

        drv.click_widget_id("edit.two_group.welch").unwrap();
        drv.run_settled();
        assert_eq!(draft_two_group_method(&drv), TwoGroupMethod::LognormalUnpairedT);
        drv.click_widget_id("edit.two_group.welch").unwrap();
        drv.run_settled();
        assert_eq!(draft_two_group_method(&drv), TwoGroupMethod::LognormalWelchT);

        drv.click_widget_id("modal.apply").unwrap();
        drv.run_settled();
        let rs = drv.state().app.document().result_sheet(&tg_res_id).unwrap();
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert!(matches!(&rs.analysis.kind, AnalysisKind::TwoGroupComparison(t) if t.method == TwoGroupMethod::LognormalWelchT));

        verify_one_sample_ratio_validation_and_commit(&mut drv, &tg_res_id);
    }

    #[test]
    fn multiple_row_lognormal_and_ratio_paired_ui_round_trip() {
        use crate::common::{ShellTestState, make_shell_driver_with_state};
        use curvebook_doc::{
            AnalysisKind, GroupedTable, LognormalRowTestMode, MultipleRowTestsSpec, ResultStatus, SheetId, SubcolumnMode, TableArchetype,
        };
        use curvebook_engine::Session;
        use curvebook_ui_egui::UiApp;
        use curvebook_ui_egui::egui_shell::Tokens;
        use curvebook_ui_egui::interaction::ViewportMetrics;
        use serde_json::json;

        // Part B: Multiple Row t-Tests modal (Lognormal & RatioPaired + nonparametric mutual exclusion)
        let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 3).unwrap();
        let vals = [([10.0, 12.0, 14.0], [20.0, 25.0, 30.0]), ([5.0, 6.5, 8.0], [15.0, 19.0, 24.0]), ([2.0, 2.5, 3.0], [6.0, 8.0, 10.0])];
        for (r, (g0, g1)) in vals.iter().enumerate() {
            for s in 0..3 {
                grp.data_sets[0].cells[r][s] = curvebook_doc::Cell::number(g0[s]);
                grp.data_sets[1].cells[r][s] = curvebook_doc::Cell::number(g1[s]);
            }
        }
        let mut gdoc = curvebook_doc::ProjectDocument::new("Row T Lognormal").unwrap();
        gdoc.add_data_sheet(curvebook_doc::DataSheet::new("grp-rt", "Grouped", TableArchetype::Grouped(grp)).unwrap()).unwrap();
        let mut gsess = Session::from_document(gdoc);
        let rt_val = serde_json::to_value(AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default())).unwrap();
        let _ = gsess.execute("analysis.run", json!({ "sheet_id": "res-rt-ln", "source_sheet_id": "grp-rt", "kind": rt_val })).unwrap();
        let rt_res_id = SheetId::try_new("res-rt-ln").unwrap();

        let mut gapp = UiApp::from_session(gsess);
        gapp.sync_views();
        let gsnap = gapp.document().clone();
        assert!(gapp.canvas_dialogs.open_edit_parameters(&gsnap, &rt_res_id));
        let gtok = Tokens::from_theme_spec(&gapp.theme, &gapp.locale);
        let mut gdrv = make_shell_driver_with_state(
            ShellTestState::new(gapp).with_commit_all().with_merge_passes(),
            ViewportMetrics::new(1280.0, 820.0, 1.0),
            gtok,
        );

        gdrv.click_widget_id("edit.row_t.lognormal.lognormal").unwrap();
        gdrv.run_settled();
        assert!(matches!(
            &gdrv.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind,
            AnalysisKind::MultipleRowTTests(m) if m.lognormal_mode == LognormalRowTestMode::Lognormal
        ));
        assert!(gdrv.state().views.stats_modals.last_resolved_banner.contains("Lognormal"));

        // RatioPaired automatically enables paired
        gdrv.click_widget_id("edit.row_t.lognormal.ratio_paired").unwrap();
        gdrv.run_settled();
        assert!(matches!(
            &gdrv.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind,
            AnalysisKind::MultipleRowTTests(m) if m.lognormal_mode == LognormalRowTestMode::RatioPaired && m.paired
        ));

        // Checking nonparametric clears lognormal_mode and disables lognormal radios
        gdrv.click_widget_id("edit.row_t.nonparam").unwrap();
        gdrv.run_settled();
        assert!(matches!(
            &gdrv.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind,
            AnalysisKind::MultipleRowTTests(m) if m.nonparametric && m.lognormal_mode == LognormalRowTestMode::None
        ));
        for wid in ["edit.row_t.lognormal.none", "edit.row_t.lognormal.lognormal", "edit.row_t.lognormal.ratio_paired"] {
            let n = gdrv.frame_meta().find_by_widget_id(wid).unwrap();
            assert!(!n.enabled && n.disabled_reason.is_some(), "{wid} must disable under nonparametric");
        }

        // Uncheck nonparametric, select RatioPaired, and commit
        gdrv.click_widget_id("edit.row_t.nonparam").unwrap();
        gdrv.run_settled();
        gdrv.click_widget_id("edit.row_t.lognormal.ratio_paired").unwrap();
        gdrv.run_settled();
        gdrv.click_widget_id("modal.ok").unwrap();
        gdrv.run_settled();
        let rs = gdrv.state().app.document().result_sheet(&rt_res_id).unwrap();
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert!(matches!(
            &rs.analysis.kind,
            AnalysisKind::MultipleRowTTests(m) if m.lognormal_mode == LognormalRowTestMode::RatioPaired && m.paired && !m.nonparametric
        ));
    }
}
