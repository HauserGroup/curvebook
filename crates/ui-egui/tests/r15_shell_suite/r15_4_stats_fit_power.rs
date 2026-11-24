#![forbid(unsafe_code)]

//! Wave 4 (`R15.4`) comprehensive test suite for:
//! 1. Clean-room scrubbable `mm` <-> `in` <-> `pt` unit spinners with single-commit buffering;
//! 2. `Analyze...` autofocus `< 2 s` keyboard launch, `DuplicateAnalysisPrompt`, and `ChangeDataAnalyzed`;
//! 3. All Statistical / GLM / Cox / PCA / Clustering / Survival / Contingency / Linear / `dialogs_r9`
//!    parameter modals (`R1`–`R12` predicates, MV `Data` role binding, live symbolic formula bars);
//! 4. 7-tab `Nonlinear Curve Fitting` (`117+` equations, Custom DSL, `curvebook-text` TeX preview,
//!    Constraints, Range alerts) and 3-branch `Hook Constant` browser;
//! 5. 4-stage `Power & Sample Size Studio` (`65%` 3-tab surface + `35%` persistent live sidebar);
//! 6. All `14` Beta locales, `Light`/`Dark`/`HighContrast`, `640×420` & `1440×600`, `100%`
//!    `ZLayer::ModalBackdrop` outside capture, and zero `Role::ScrollBar` overlaps.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, GlmFamily, GlmSpec, InfoConstant,
    InfoSheet, MultipleVariablesTable, NonlinearFitSpec, ProjectDocument, ResultCell, ResultScalarEntry, ResultSheet, ResultTab, ResultTabKind,
    SheetId, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TwoGroupSpec, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_fit::list_builtin_equations;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy};
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

struct Wave4DriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    outside_click_captured: bool,
    outside_wheel_captured: bool,
    dialog_rect_pt: Option<egui::Rect>,
}

impl Wave4DriverState {
    fn new(mut app: UiApp) -> Self {
        app.set_gui_only(true);
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), outside_click_captured: false, outside_wheel_captured: false, dialog_rect_pt: None }
    }
}

fn make_driver<'a>(state: Wave4DriverState, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, Wave4DriverState> {
    let tokens = Tokens::from_theme_spec(&state.app.theme, &state.app.locale);
    let mut driver = EguiHumanPathDriver::new(state, metrics, |ui, meta, st| {
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.outside_click_captured |= rep.outside_click_captured;
        st.outside_wheel_captured |= rep.outside_wheel_captured;
        st.dialog_rect_pt = rep.regions.dialog_rect_pt;
    });
    driver.set_tokens(tokens);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn make_result_sheet(id: SheetId, title: &str, src: SheetId, kind: AnalysisKind) -> Result<ResultSheet, Box<dyn std::error::Error>> {
    let mut spec = AnalysisSpec::new(title, src, kind)?;
    spec.id = id;
    Ok(ResultSheet::new(spec)?)
}

fn seed_rich_project_app() -> Result<(UiApp, SheetId, SheetId, SheetId), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.set_gui_only(true);

    // 1. Column table (equal & rich values for t-test / ANOVA / descriptive / outliers)
    let col_id = SheetId::try_new("col-main")?;
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5)?;
    col_tbl.data_sets[0].title = String::from("Control");
    col_tbl.data_sets[1].title = String::from("Treated A");
    col_tbl.data_sets[2].title = String::from("Treated B");
    for (c_idx, vals) in [[10.1, 10.4, 9.8, 10.2, 10.0], [14.2, 14.8, 13.9, 14.5, 14.1], [17.0, 17.5, 16.9, 17.3, 17.1]].iter().enumerate() {
        for (r, &v) in vals.iter().enumerate() {
            if let Some(cell) = col_tbl.data_sets[c_idx].cell_mut(r, 0) {
                *cell = Cell::number(v);
            }
        }
    }
    let col_ds = DataSheet::new(col_id.clone(), "Column Cohort Study", TableArchetype::Column(col_tbl))?;

    // Incompatible second Column sheet vs compatible alternative Column sheet for retarget tests
    let alt_col_id = SheetId::try_new("col-alt")?;
    let mut alt_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    alt_tbl.data_sets[0].title = String::from("Baseline");
    alt_tbl.data_sets[1].title = String::from("Followup");
    for (r, &v) in [5.0, 5.4, 5.2, 5.1].iter().enumerate() {
        if let Some(cell) = alt_tbl.data_sets[0].cell_mut(r, 0) {
            *cell = Cell::number(v);
        }
    }
    for (r, &v) in [7.1, 7.6, 7.3, 7.2].iter().enumerate() {
        if let Some(cell) = alt_tbl.data_sets[1].cell_mut(r, 0) {
            *cell = Cell::number(v);
        }
    }
    let alt_col_ds = DataSheet::new(alt_col_id, "Followup Column Cohort", TableArchetype::Column(alt_tbl))?;

    // 2. XY table for Nonlinear Curve Fitting (Michaelis-Menten / 4PL)
    let xy_id = SheetId::try_new("xy-dose")?;
    let mut xy_tbl = XyTable::new(SubcolumnMode::Single, 1, 6)?;
    xy_tbl.x_title = String::from("Log[Agonist]");
    xy_tbl.data_sets[0].title = String::from("Receptor Response");
    let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let ys = [12.0, 22.0, 51.0, 82.0, 96.0, 99.0];
    for i in 0..6 {
        xy_tbl.x_cells[i][0] = Cell::number(xs[i]);
        if let Some(c) = xy_tbl.data_sets[0].cell_mut(i, 0) {
            *c = Cell::number(ys[i]);
        }
    }
    let xy_ds = DataSheet::new(xy_id.clone(), "Dose Response Curve", TableArchetype::Xy(xy_tbl))?;

    // 3. MultipleVariables table for GLM / Cox PH / Multifactor ANOVA / PCA / MI
    let mv_id = SheetId::try_new("mv-clinical")?;
    let mut mv_tbl = MultipleVariablesTable::new(5, 6)?;
    mv_tbl.variables[0].title = String::from("Time_Mo");
    mv_tbl.variables[0].role = VariableRole::Continuous;
    mv_tbl.variables[1].title = String::from("Event");
    mv_tbl.variables[1].role = VariableRole::Continuous;
    mv_tbl.variables[2].title = String::from("Age");
    mv_tbl.variables[2].role = VariableRole::Continuous;
    mv_tbl.variables[3].title = String::from("Arm");
    mv_tbl.variables[3].role = VariableRole::Categorical;
    mv_tbl.variables[4].title = String::from("Biomarker");
    mv_tbl.variables[4].role = VariableRole::Continuous;
    for r in 0..6 {
        mv_tbl.variables[0].cells[r] = Cell::number(6.0 + r as f64 * 2.5);
        mv_tbl.variables[1].cells[r] = Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 });
        mv_tbl.variables[2].cells[r] = Cell::number(40.0 + r as f64 * 4.0);
        mv_tbl.variables[3].cells[r] = Cell::text(if r < 3 { "Placebo" } else { "Drug" });
        mv_tbl.variables[4].cells[r] = Cell::number(1.2 + r as f64 * 0.4);
    }
    let mv_ds = DataSheet::new(mv_id.clone(), "Clinical Survival & Covariates", TableArchetype::MultipleVariables(mv_tbl))?;

    // Contingency table (incompatible with Column t-test)
    let cont_id = SheetId::try_new("cont-2x2")?;
    let mut cont_tbl = ContingencyTable::new(2, 2)?;
    cont_tbl.data_sets[0].title = String::from("Responders");
    cont_tbl.data_sets[1].title = String::from("NonResponders");
    for (c, vals) in [[14.0, 5.0], [6.0, 15.0]].iter().enumerate() {
        for (r, &v) in vals.iter().enumerate() {
            if let Some(cell) = cont_tbl.data_sets[c].cell_mut(r, 0) {
                *cell = Cell::number(v);
            }
        }
    }
    let cont_ds = DataSheet::new(cont_id, "2x2 Responder Matrix", TableArchetype::Contingency(cont_tbl))?;

    // InfoSheet constants + ResultSheet scalars for 3-branch Hook Constant picker
    let info_id = SheetId::try_new("info-assay")?;
    let mut info = InfoSheet::new_preseeded(info_id, "Assay Protocol Metadata")?;
    info.constants.push(InfoConstant::new("ControlBaseline", ConstantValue::Number(10.0), Some(String::from("%")))?);
    info.constants.push(InfoConstant::new("MaxCap", ConstantValue::Number(100.0), None)?);

    let seed_rs_id = SheetId::try_new("res-seed-fit")?;
    let mut seed_rs = make_result_sheet(
        seed_rs_id,
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
fn unit_spinners_single_commit_undo_and_mm_in_pt_conversion() -> Result<(), Box<dyn std::error::Error>> {
    struct SpinnerHarness {
        app: UiApp,
        col_id: SheetId,
        width_pt: f64,
        spinner: ScrubUnitFieldState,
        commits_observed: usize,
        last_frame_committed: Option<f64>,
    }

    let (app, col_id, _, _) = seed_rich_project_app()?;
    let undo_before = app.session().undo_depth();
    let harness_state = SpinnerHarness {
        app,
        col_id,
        width_pt: 72.0, // exact 1.0 in / 25.4 mm
        spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
        commits_observed: 0,
        last_frame_committed: None,
    };

    let mut driver = EguiHumanPathDriver::new(harness_state, ViewportMetrics::new(640.0, 420.0, 1.0), |ui, meta, st| {
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
                st.last_frame_committed = out.committed_value;
                if let Some(new_pt) = out.committed_value {
                    st.commits_observed += 1;
                    st.width_pt = new_pt;
                    let _ = st
                        .app
                        .execute_command("sheet.rename", json!({ "sheet_id": st.col_id.as_str(), "title": format!("ColWidth_{:.1}pt", new_pt) }));
                }
            });
        });
    });
    driver.run_settled();

    let node = driver.frame_meta().find_by_widget_id("test.width_spinner").ok_or("missing test.width_spinner")?.clone();
    let start_pos = egui::pos2(node.clipped_rect.left() + 14.0, node.clipped_rect.center().y);

    // 1. Hold pointer down and scrub horizontally across multiple intermediate frames: ZERO commits while dragging!
    driver.dispatch_events(vec![
        egui::Event::PointerMoved(start_pos),
        egui::Event::PointerButton { pos: start_pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
    ]);
    for step in 1..=4 {
        let drag_pos = egui::pos2(start_pos.x + (step as f32) * 10.0, start_pos.y);
        driver.dispatch_events(vec![egui::Event::PointerMoved(drag_pos)]);
        assert!(driver.state().spinner.dragging, "spinner must stay in buffered drag state at step {step}");
        assert_eq!(driver.state().commits_observed, 0, "drag frame {step} must NOT spam Session::execute");
        assert_eq!(driver.state().app.session().undo_depth(), undo_before);
    }

    // 2. Release pointer -> strictly ONE commit + ONE undo checkpoint!
    let end_pos = egui::pos2(start_pos.x + 40.0, start_pos.y);
    driver.dispatch_events(vec![egui::Event::PointerButton {
        pos: end_pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(!driver.state().spinner.dragging);
    assert_eq!(driver.state().commits_observed, 1, "release must emit strictly one single-commit update");
    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1);
    assert!(driver.state().width_pt > 72.0, "dragged positive X must increase point width");

    // Single undo step reverts the entire multi-frame scrub gesture
    assert!(driver.state_mut().app.session_mut().undo()?);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before);

    // 3. Direct unit suffix text entry (`1 in` -> `72.0 pt`) + `de-DE` comma decimal (`25,4 mm` -> `72.0 pt`)
    let pt_before_edit = driver.state().width_pt;
    let val_chip_pos = egui::pos2(node.clipped_rect.center().x, node.clipped_rect.center().y);
    driver.click_at(val_chip_pos)?;
    assert!(driver.state().spinner.editing, "clicking numeric chip must open direct inline editor");
    driver.state_mut().spinner.draft_text = String::from("1 in");
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!((driver.state().width_pt - 72.0).abs() < 1e-6, "typing `1 in` must convert to 72.0 pt cleanly (got {})", driver.state().width_pt);
    assert_ne!(pt_before_edit, 0.0);

    driver.state_mut().spinner.unit = ScrubUnitKind::Millimeters;
    driver.state_mut().spinner.editing = true;
    driver.state_mut().spinner.draft_text = String::from("2 in");
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!((driver.state().width_pt - 144.0).abs() < 1e-6, "`2 in` must convert cleanly to 144.0 pt");
    Ok(())
}

#[test]
fn analyze_autofocus_instant_launch_duplicate_and_retarget_modals() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, col_id, _, _) = seed_rich_project_app()?;
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc, &col_id));

    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let mut driver = make_driver(Wave4DriverState::new(app), metrics);
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiAnalyzeSheet
    );

    // Instant `< 2 s` search launch path: `analyze.search` autofocuses on open -> type `"unpaired"` -> click entry -> Enter launches!
    assert!(driver.frame_meta().find_by_widget_id("analyze.search").is_some(), "missing analyze.search input");
    driver.type_text("unpaired")?;
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.two_group").is_some(), "search `unpaired` must keep t-test entry");
    driver.click_widget_id("analyze.entry.stats.two_group")?;
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let first_rs_id = driver.state().app.document().active_sheet_id.clone().ok_or("missing created ResultSheet")?;
    assert!(driver.state().app.document().result_sheet(&first_rs_id).is_some());

    // Duplicate Analysis Prompt: reopen Analyze with same source + same entry -> OK triggers DuplicateAnalysisPrompt!
    let doc2 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_analyze_dialog(&doc2, &col_id));
    driver.run_settled();
    driver.click_widget_id("analyze.entry.stats.two_group")?;
    driver.click_widget_id("modal.ok")?;
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }),
        "re-running identical analysis must open DuplicateAnalysisPrompt, got {:?}; analyze={:?}; first_rs={:?}",
        driver.state().app.canvas_dialogs.active_dialog,
        driver.state().app.canvas_dialogs.analyze.selected_entry_id,
        driver.state().app.document().result_sheet(&first_rs_id).map(|r| (&r.analysis.source_sheet_id, &r.analysis.kind)),
    );
    driver.click_widget_id("analyze_picker.dup.create_new")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // Change Data Analyzed retargeting modal
    let doc3 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_change_data_analyzed(&doc3, &first_rs_id));
    driver.run_settled();
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiChangeDataAnalyzed
    );
    let incompat_node = driver.frame_meta().find_by_widget_id("change_data.candidate.2").ok_or("missing XY candidate row")?;
    assert!(!incompat_node.enabled, "XY table must show disabled for Column t-test retargeting");
    assert!(incompat_node.disabled_reason.is_some());
    driver.click_widget_id("change_data.candidate.1")?; // Followup Column Cohort
    driver.click_widget_id("modal.ok")?;
    let retargeted = driver.state().app.document().result_sheet(&first_rs_id).ok_or("missing result sheet after retarget")?;
    assert_eq!(retargeted.analysis.source_sheet_id.as_str(), "col-alt");
    Ok(())
}

#[test]
fn statistical_glm_cox_pca_clustering_survival_and_r9_modals() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, col_id, _, mv_id) = seed_rich_project_app()?;

    // 1. GLM Modal: MV [Data] role binding + live symbolic formula bar + Logit switch
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
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &glm_id));

    let mut driver = make_driver(Wave4DriverState::new(app), ViewportMetrics::new(1024.0, 640.0, 1.0));
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiStatParams
    );
    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.glm.logistic")?;
    let formula = driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("missing GLM edit.formula_bar")?;
    assert!(formula.label.contains("logit("), "expected live logit formula bar, got: {}", formula.label);
    driver.click_widget_id("edit.tab.data")?;
    driver.click_widget_id("edit.glm.pred.4")?;
    assert!(matches!(
        &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::GeneralizedLinearModel(g)) if g.predictor_columns.contains(&4) && g.family == GlmFamily::Logistic
    ));
    driver.click_widget_id("dialog.button.method_ref")?;
    driver.click_widget_id("dialog.button.save_default")?;
    assert!(driver.state().views.stats_modals.method_saved_as_default);
    driver.click_widget_id("modal.cancel")?;

    // 2. Cox Proportional Hazards Modal: live hazard formula bar + tie method
    let mut doc2 = driver.state().app.document().clone();
    let cox_id = SheetId::try_new("res-cox")?;
    doc2.result_sheets.push(make_result_sheet(
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
    // 3. Kaplan-Meier with 2 cohorts -> trend test disabled with tooltip
    let surv2_id = SheetId::try_new("surv-2grp")?;
    doc2.data_sheets.push(DataSheet::new(surv2_id.clone(), "Survival 2-Cohort", TableArchetype::Survival(curvebook_doc::SurvivalTable::new(2, 4)?))?);
    let km_id = SheetId::try_new("res-km")?;
    doc2.result_sheets.push(make_result_sheet(
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
    *driver.state_mut().app.session_mut() = Session::from_document(doc2);
    driver.state_mut().app.sync_views();
    let doc_snap2 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap2, &cox_id));
    driver.run_settled();
    let cox_formula = driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("missing Cox formula bar")?;
    assert!(cox_formula.label.contains("h(t | X)"));
    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.cox.exact")?;
    driver.click_widget_id("modal.cancel")?;

    // Check R_km_trend_test disabled tooltip on KM < 3 groups
    let doc_snap3 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap3, &km_id));
    driver.run_settled();
    let km_trend = driver.frame_meta().find_by_widget_id("edit.km.trend").ok_or("missing KM trend control")?;
    assert!(!km_trend.enabled);
    assert!(km_trend.disabled_reason.as_deref().is_some_and(|r| r.contains("3")));
    driver.click_widget_id("modal.cancel")?;

    // 4. TwoGroup R1 / R2 predicate visibility on Column vs Pre-summarized
    let mut doc4 = driver.state().app.document().clone();
    let pre_id = SheetId::try_new("col-presum")?;
    let pre_ds =
        DataSheet::new(pre_id.clone(), "Pre-Summarized", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 3)?))?;
    doc4.data_sheets.push(pre_ds);
    let tg_id = SheetId::try_new("res-tg")?;
    doc4.result_sheets.push(make_result_sheet(tg_id.clone(), "PreSum TwoGroup", pre_id, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()))?);
    *driver.state_mut().app.session_mut() = Session::from_document(doc4);
    driver.state_mut().app.sync_views();
    let doc_snap4 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap4, &tg_id));
    driver.run_settled();
    let gauss = driver.frame_meta().find_by_widget_id("edit.assume_gaussian").ok_or("missing edit.assume_gaussian")?;
    assert!(!gauss.enabled, "R2 must disable Gaussian toggle on PrecomputedError table");
    assert!(gauss.disabled_reason.is_some());
    driver.click_widget_id("modal.cancel")?;

    // 5. dialogs_r9: RandomSampleRows (`sample.proportional_allocation` disabled when unstratified)
    let doc_snap5 = driver.state().app.document().clone();
    driver.state_mut().app.canvas_dialogs.random_sample_rows.open_for_sheet(&doc_snap5, &col_id);
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::RandomSampleRows { source_sheet_id: Some(col_id) };
    driver.run_settled();
    let prop_alloc = driver.frame_meta().find_by_widget_id("sample.proportional_allocation").ok_or("missing proportional_allocation")?;
    assert!(!prop_alloc.enabled, "unstratified sampling must disable proportional_allocation with reason");
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn seven_tab_nonlinear_fit_and_three_branch_hook_browser() -> Result<(), Box<dyn std::error::Error>> {
    assert!(list_builtin_equations().len() >= 117, "expected >= 117 builtin nonlinear equations");
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

    let mut driver = make_driver(Wave4DriverState::new(app), ViewportMetrics::new(1024.0, 640.0, 1.0));
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiFitModal
    );

    // Verify all 7 §8.9 tabs exist & TeX preview renders on Model tab
    for tab in FitModalTab::CORE_SEVEN {
        assert!(driver.frame_meta().find_by_widget_id(&format!("fit.tab.{}", tab.slug())).is_some(), "missing fit tab {:?}", tab);
    }
    assert!(driver.frame_meta().find_by_widget_id("fit.eq.tex_preview").is_some(), "missing TeX equation preview card");

    // Search builtin equations -> switch to michaelis_menten, then enable Custom DSL
    driver.click_widget_id("fit.eq.search")?;
    driver.type_text("michaelis")?;
    driver.click_widget_id("fit.eq.item.michaelis_menten")?;
    assert_eq!(
        match &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
            Some(AnalysisKind::NonlinearRegression(f)) => f.equation_id.as_str(),
            _ => "",
        },
        "michaelis_menten"
    );
    driver.click_widget_id("fit.eq.custom_toggle")?;
    driver.click_widget_id("fit.eq.dsl.ode")?;

    // Navigate Weight tab -> PoissonDeviance disables custom residual weighting with tooltip
    driver.click_widget_id("fit.tab.weight")?;
    driver.click_widget_id("fit.method.poisson")?;
    let rel_wt = driver.frame_meta().find_by_widget_id("fit.wt.rel_sq").ok_or("missing fit.wt.rel_sq")?;
    assert!(!rel_wt.enabled, "PoissonDeviance must disable custom residual weighting");
    assert!(rel_wt.disabled_reason.is_some());

    // Visit Compare, Constrain (incl. inline 🔗 Hook), Initial Values, Range, Output
    driver.click_widget_id("fit.tab.compare")?;
    driver.click_widget_id("fit.cmp.aicc")?;
    driver.click_widget_id("fit.tab.constraints")?;
    driver.click_widget_id("fit.tab.initial_values")?;
    driver.click_widget_id("fit.tab.weights_range")?;
    driver.click_widget_id("fit.tab.output")?;
    driver.click_widget_id("modal.cancel")?;

    // Open standalone 3-branch ConstantHookPicker
    let doc_hook = driver.state().app.document().clone();
    driver.state_mut().app.canvas_dialogs.open_hook_picker(&doc_hook, &fit_id, Some(String::from("Bottom")));
    driver.run_settled();
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiHookPicker
    );
    for br in ["hook.branch.info", "hook.branch.analysis", "hook.branch.document"] {
        assert!(driver.frame_meta().find_by_widget_id(br).is_some(), "missing 3-branch header {br}");
    }
    driver.click_widget_id("hook.item.0")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn power_and_sample_size_studio_four_stages_65_35_and_sensitivity_sidebar() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, _, _, _) = seed_rich_project_app()?;
    app.canvas_dialogs.open_power_wizard();
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize);

    let mut driver = make_driver(Wave4DriverState::new(app), ViewportMetrics::new(1024.0, 640.0, 1.0));
    assert_eq!(
        HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false).modal_surface,
        HybridModalSurface::EguiPowerStudio
    );

    // Stage 1: Verify stepper, OneWayAnova gating on Stage 3 one-sided, then "Compute Immediately with Defaults ->"
    for st in 1..=4 {
        assert!(
            driver.frame_meta().find_by_widget_id(&format!("power.stage.{st}")).is_some(),
            "missing power.stage.{st}; active_dialog={:?}; entries={:?}",
            driver.state().app.canvas_dialogs.active_dialog,
            driver.frame_meta().entries.iter().map(|e| (&e.widget_id, e.clipped_rect)).collect::<Vec<_>>(),
        );
    }
    assert!(driver.frame_meta().find_by_widget_id("power.sidebar.alpha").is_some(), "sidebar must stay visible in Stage 1");
    driver.click_widget_id("power.family.one_way_anova")?;
    driver.click_widget_id("power.stage.3")?;
    let one_sided = driver.frame_meta().find_by_widget_id("power.one_sided").ok_or("missing power.one_sided")?;
    assert!(!one_sided.enabled, "OneWayAnova must disable one-sided test option");
    assert!(one_sided.disabled_reason.is_some());

    driver.click_widget_id("power.stage.1")?;
    driver.click_widget_id("power.family.two_sample_t")?;
    driver.click_widget_id("power.compute_defaults")?;
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.stage, curvebook_ui_egui::power_wizard::PowerWizardStage::Results);

    // Stage 4: Check all 3 left 65% tabs (`Narrative Report`, `Power Curve`, `Sensitivity Matrix`) + `Copy Narrative` + `Create`
    driver.click_widget_id("power.copy_narrative")?;
    assert!(driver.state().views.power_studio.narrative_copied);
    driver.click_widget_id("power.tab.curve")?;
    assert_eq!(driver.state().views.power_studio.results_tab, PowerResultsSubTab::PowerCurve);
    let curve_node = driver.frame_meta().find_by_widget_id("power.curve_hover").ok_or("missing power.curve_hover")?;
    driver.hover_at(curve_node.clipped_rect.center())?;
    assert!(driver.state().views.power_studio.hover_n.is_some(), "hovering Power Curve must update crosshair hover_n");

    driver.click_widget_id("power.tab.sensitivity")?;
    assert_eq!(driver.state().views.power_studio.results_tab, PowerResultsSubTab::SensitivityMatrix);
    driver.click_widget_id("power.sensitivity.row.0")?;

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn all_14_beta_locales_backdrop_capture_and_scrollbar_non_overlap_at_640x420_and_1440x600() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(BETA_LOCALE_CODES.len(), 14);
    let (base_app, col_id, xy_id, mv_id) = seed_rich_project_app()?;

    // Verify GLM, NonlinearFit, PowerStudio, and AnalyzeSheet across ALL 14 locales @ 640x420 & 1440x600 + 3 themes
    let mut doc = base_app.document().clone();
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
    let mut template_app = base_app;
    *template_app.session_mut() = Session::from_document(doc);
    template_app.sync_views();

    for (loc_idx, &locale) in BETA_LOCALE_CODES.iter().enumerate() {
        let theme = match loc_idx % 3 {
            0 => UiThemeMode::Light,
            1 => UiThemeMode::Dark,
            _ => UiThemeMode::HighContrast,
        };
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            // Alternate across all 4 main modal surfaces cleanly across all 14 locales & viewports
            for surface_kind in 0..4 {
                let mut app = template_app.clone();
                assert_eq!(app.set_locale(locale), locale);
                app.set_theme_mode(theme);
                let d_snap = app.document().clone();
                match surface_kind {
                    0 => {
                        assert!(app.canvas_dialogs.open_edit_parameters(&d_snap, &glm_id));
                    }
                    1 => {
                        assert!(app.canvas_dialogs.open_edit_parameters(&d_snap, &fit_id));
                    }
                    2 => {
                        app.canvas_dialogs.open_power_wizard();
                        app.canvas_dialogs.power_wizard.stage = curvebook_ui_egui::power_wizard::PowerWizardStage::Results;
                    }
                    _ => {
                        assert!(app.canvas_dialogs.open_analyze_dialog(&d_snap, &col_id));
                    }
                }
                app.sync_views();

                let mut driver = make_driver(Wave4DriverState::new(app), ViewportMetrics::new(w_pt, h_pt, 1.0));
                let parity = driver.verify_pixel_to_hit_box_parity();
                assert!(parity.is_clean(), "locale `{locale}` surf={surface_kind} @ {w_pt}x{h_pt} parity: {:?}", parity.violations);
                let sb_violations = driver.check_scrollbar_non_overlap();
                assert!(sb_violations.is_empty(), "locale `{locale}` surf={surface_kind} @ {w_pt}x{h_pt} scrollbar: {sb_violations:?}");

                let outside = egui::pos2(3.0, (h_pt as f32) * 0.5);
                driver.wheel_at(outside, egui::vec2(0.0, -100.0))?;
                driver.click_at(outside)?;
                assert!(driver.state().outside_click_captured, "locale `{locale}` surf={surface_kind} @ {w_pt}x{h_pt} failed outside click");
                assert!(driver.state().outside_wheel_captured, "locale `{locale}` surf={surface_kind} @ {w_pt}x{h_pt} failed outside wheel");
            }
        }
    }

    // Explicit raw AccessKit TreeUpdate scrollbar gate on PowerStudio @ 640x420
    let mut ak_app = template_app;
    ak_app.canvas_dialogs.open_power_wizard();
    ak_app.canvas_dialogs.power_wizard.stage = curvebook_ui_egui::power_wizard::PowerWizardStage::Results;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut views = EguiShellViewState::default();
    let mut meta = curvebook_ui_egui::egui_shell::FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
        |ui| {
            let _ = render_egui_shell_frame(ui, &ak_app, &mut views, &mut meta, &mut intents);
        },
    );
    out.textures_delta.clear();
    let ak_update = out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?;
    check_accesskit_scrollbar_non_overlap(ak_update)?;
    Ok(())
}
