#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `R15.6` Wave B (`B1`, `B2`, `B3`) human-path GUI regression suite driving rendered widgets
//! through `EguiHumanPathDriver`:
//! - **B1 (`R15-GUI-09`, `R15-GUI-10`)**: Transform Values standalone Y "None" does NOT fall back
//!   to `log10(Y)`; dead controls removed; `Constant K` evaluated at commit time in both standalone
//!   and edit modes; `Y / K` with `K = 0` or invalid/empty K rejected via `validation_error`.
//! - **B2 (`R15-GUI-03`)**: Factorial / MV ANOVA live Model Formula bar lists every k-subset
//!   interaction in lexicographic order matching the engine.
//! - **B3 (`R15-GUI-16`, `R15-GUI-17`)**: Nonlinear Fit modal seeds X-domain filter from actual
//!   analysed data range (including negative log10[M] X values), validates `xmin < xmax`, computes
//!   data-driven Auto initial guesses without truncating `1e-7`, and provides locale-aware numeric
//!   editors for initial overrides and lower/upper parameter bounds.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, FactorialAnovaSpec, MathTransformPreset, MultipleVariablesTable, NonlinearFitSpec,
    ProjectDocument, ResultSheet, SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, VariableRole, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::interaction::ViewportMetrics;
use egui::{Key, Modifiers};
use serde_json::json;

struct WaveBDriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_intent_error: Option<String>,
}

impl WaveBDriverState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::default(), last_intent_error: None }
    }
}

fn make_shell_driver(doc: ProjectDocument, w: f64, h: f64) -> EguiHumanPathDriver<'static, WaveBDriverState> {
    let app = UiApp::from_session(Session::from_document(doc));
    let tokens = Tokens::default();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        WaveBDriverState::new(app),
        ViewportMetrics::new(w, h, 1.0),
        tokens,
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

fn commit_intents(driver: &mut EguiHumanPathDriver<'_, WaveBDriverState>) {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
        st.last_intent_error = Some(err.to_string());
    }
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

fn replace_focused_text<S>(driver: &mut EguiHumanPathDriver<'_, S>, text: &str) -> Result<(), String> {
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.press_egui_key(Key::Backspace, Modifiers::NONE);
    if !text.is_empty() {
        driver.type_text(text)?;
    }
    Ok(())
}

fn build_pharma_xy_doc() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Wave B Pharmacology").unwrap();
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 4).unwrap();
    xy.x_title = String::from("Conc (uM)");
    xy.data_sets[0].title = String::from("Response");
    let xs = [0.01, 0.1, 1.0, 10.0];
    let ys = [10.0, 35.0, 75.0, 98.0];
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number(xs[r]);
        *xy.data_sets[0].cell_mut(r, 0).unwrap() = Cell::number(ys[r]);
    }
    let ds = DataSheet::new(SheetId::try_new("xy-pharma").unwrap(), "Pharma XY", TableArchetype::Xy(xy)).unwrap();
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(SheetId::from("xy-pharma"));
    doc
}

fn build_dose_response_xy_doc() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Wave B Dose Response").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
    xy.x_title = String::from("Log[Agonist] (M)");
    xy.data_sets[0].title = String::from("Receptor");
    let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let ys = [12.0, 22.0, 51.0, 82.0, 96.0, 99.0];
    for r in 0..6 {
        xy.x_cells[r][0] = Cell::number(xs[r]);
        *xy.data_sets[0].cell_mut(r, 0).unwrap() = Cell::number(ys[r]);
    }
    let xy_id = SheetId::try_new("xy-dose").unwrap();
    doc.data_sheets.push(DataSheet::new(xy_id.clone(), "Dose Response", TableArchetype::Xy(xy)).unwrap());

    let fit_id = SheetId::try_new("res-nlf").unwrap();
    let mut spec =
        AnalysisSpec::new("4PL Fit", xy_id, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).unwrap();
    spec.id = fit_id;
    doc.result_sheets.push(ResultSheet::new(spec).unwrap());
    doc.active_sheet_id = Some(SheetId::from("xy-dose"));
    doc
}

fn build_mv_factorial_doc() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Wave B Factorial").unwrap();
    let mut mv = MultipleVariablesTable::new(5, 4).unwrap();
    mv.variables[0].title = String::from("Response");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("FactorA");
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[2].title = String::from("FactorB");
    mv.variables[2].role = VariableRole::Categorical;
    mv.variables[3].title = String::from("FactorC");
    mv.variables[3].role = VariableRole::Categorical;
    mv.variables[4].title = String::from("FactorD");
    mv.variables[4].role = VariableRole::Categorical;

    let mv_id = SheetId::try_new("mv-fac").unwrap();
    doc.data_sheets.push(DataSheet::new(mv_id.clone(), "MV Factorial", TableArchetype::MultipleVariables(mv)).unwrap());

    let fac = FactorialAnovaSpec { response_column: Some(0), multifactor_columns: vec![1, 2, 3], max_interaction_order: 2, ..Default::default() };
    let res_id = SheetId::try_new("res-fac").unwrap();
    let mut spec = AnalysisSpec::new("Factorial ANOVA", mv_id, AnalysisKind::FactorialAnova(fac)).unwrap();
    spec.id = res_id;
    doc.result_sheets.push(ResultSheet::new(spec).unwrap());
    doc.active_sheet_id = Some(SheetId::from("mv-fac"));
    doc
}

#[test]
fn b1_r15_gui_09_standalone_pharmacology_transform_y_none_does_not_apply_log10() -> Result<(), String> {
    let mut driver = make_shell_driver(build_pharma_xy_doc(), 1280.0, 800.0);

    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("xy-pharma"), kind: TransformModalKind::TransformValues });
    commit_intents(&mut driver);

    // Switch to Pharmacology mode (uM -> -log10[M]) and explicitly select Y "None".
    driver.click_widget_id("edit.transform.mode.pharmacology")?;
    driver.click_widget_id("edit.transform.y_transform.none")?;

    // Active state must show Y "None" active and log10(Y) inactive.
    let y_none = driver.frame_meta().find_by_widget_id("edit.transform.y_transform.none").ok_or("missing Y None pill")?;
    let y_log10 = driver.frame_meta().find_by_widget_id("edit.transform.y_transform.log10").ok_or("missing Y log10 pill")?;
    assert_eq!(y_none.value, Some(json!("true")), "Y None pill must be active");
    assert_eq!(y_log10.value, Some(json!("false")), "Y log10 pill must NOT be active when None is selected");

    // Dead controls from R15-GUI-10 must not be rendered.
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.zero_conc_sub").is_none());
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.create_graph").is_none());

    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none(), "unexpected intent error: {:?}", driver.state().last_intent_error);

    let rs = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { .. })))
        .ok_or("TransformValues must create a Transform ResultSheet")?;

    let AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, .. }) = &rs.analysis.kind else {
        return Err(String::from("expected Math transform spec"));
    };
    assert_eq!(*x_transform, Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 }));
    assert_eq!(*y_transform, None, "Y transform must remain None (not Log10)");

    let transformed = rs.analyzable_data.as_ref().ok_or("missing analyzable_data on transform result")?;
    let TableArchetype::Xy(xy) = &transformed.archetype else {
        return Err(String::from("expected XY transformed sheet"));
    };
    // X = 1.0 uM -> -log10(1e-6) = 6.0; Y must remain untouched [10.0, 35.0, 75.0, 98.0].
    assert!((xy.raw_x_value_at(2).unwrap() - 6.0).abs() < 1e-9);
    for (r, expected_y) in [10.0, 35.0, 75.0, 98.0].into_iter().enumerate() {
        let got_y = transformed.cell(CellCoord::new(r, 0, 0)).unwrap().active_number().unwrap();
        assert!((got_y - expected_y).abs() < 1e-9, "row {r}: expected unchanged Y={expected_y}, got {got_y}");
    }
    Ok(())
}

#[test]
fn b1_r15_gui_10_constant_k_validation_and_commit_time_linear_scale_in_both_modes() -> Result<(), String> {
    let mut driver = make_shell_driver(build_pharma_xy_doc(), 1280.0, 800.0);

    // 1. Standalone mode: click Y / K, enter 0 -> validation error & OK disabled.
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("xy-pharma"), kind: TransformModalKind::TransformValues });
    commit_intents(&mut driver);

    driver.click_widget_id("edit.transform.y_transform.divide_k")?;
    let div_pill = driver.frame_meta().find_by_widget_id("edit.transform.y_transform.divide_k").ok_or("missing Y / K pill")?;
    let log_pill = driver.frame_meta().find_by_widget_id("edit.transform.y_transform.log10").ok_or("missing log10 pill")?;
    assert_eq!(div_pill.value, Some(json!("true")), "Y / K pill must show active in standalone mode");
    assert_eq!(log_pill.value, Some(json!("false")), "log10 pill must NOT show active when Y / K is selected");

    driver.click_widget_id("edit.transform.constant_k")?;
    replace_focused_text(&mut driver, "0")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some(), "K=0 for Y / K must set validation_error");
    let ok_node = driver.frame_meta().find_by_widget_id("modal.ok").ok_or("missing modal.ok")?.clone();
    assert!(!ok_node.enabled, "modal.ok must be disabled when K=0 for Y / K");
    // Clicking the disabled OK button at its screen position must not commit or close the modal.
    assert!(driver.click_widget_id("modal.ok").is_err(), "click_widget_id must refuse disabled modal.ok");
    driver.click_at(ok_node.clipped_rect.center())?;
    assert_eq!(driver.state().views.transform_modals.open_kind, Some(TransformModalKind::TransformValues));
    assert!(driver.state().app.document().result_sheets.is_empty());

    // Now select Y * K FIRST, then type "2" AFTER clicking Y * K, and commit.
    driver.click_widget_id("edit.transform.y_transform.multiply_k")?;
    driver.click_widget_id("edit.transform.constant_k")?;
    replace_focused_text(&mut driver, "2")?;
    assert!(driver.state().views.transform_modals.validation_error.is_none());
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none(), "unexpected intent error: {:?}", driver.state().last_intent_error);

    let rs_id = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { .. })))
        .map(|rs| rs.id.clone())
        .ok_or("expected created Transform ResultSheet")?;
    {
        let rs = driver.state().app.document().result_sheet(&rs_id).unwrap();
        let AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, .. }) = &rs.analysis.kind else {
            return Err(String::from("expected Math transform"));
        };
        assert_eq!(*y_transform, Some(MathTransformPreset::LinearScale { slope: 2.0, intercept: 0.0 }));
        let y0 = rs.analyzable_data.as_ref().unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap();
        assert!((y0 - 20.0).abs() < 1e-9, "expected 10 * 2 = 20, got {y0}");
    }

    // 2. Edit mode: open EditAnalysisParameters, click Y * K, THEN type "3", commit -> slope = 3.0!
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() })).map_err(|e| e.to_string())?;
    driver.run_settled();
    driver.state_mut().intents.push_ui("ui.editParams.open", json!({}));
    commit_intents(&mut driver);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    driver.click_widget_id("edit.transform.y_transform.multiply_k")?;
    driver.click_widget_id("edit.transform.constant_k")?;
    replace_focused_text(&mut driver, "3")?;
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none(), "unexpected intent error: {:?}", driver.state().last_intent_error);

    let rs_after = driver.state().app.document().result_sheet(&rs_id).unwrap();
    let AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, .. }) = &rs_after.analysis.kind else {
        return Err(String::from("expected Math transform after edit"));
    };
    assert_eq!(*y_transform, Some(MathTransformPreset::LinearScale { slope: 3.0, intercept: 0.0 }));
    let y0_after = rs_after.analyzable_data.as_ref().unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap();
    assert!((y0_after - 30.0).abs() < 1e-9, "expected 10 * 3 = 30 after edit, got {y0_after}");

    // Also verify edit mode rejects K = 0 for Y / K.
    driver.state_mut().intents.push_ui("ui.editParams.open", json!({}));
    commit_intents(&mut driver);
    driver.click_widget_id("edit.transform.y_transform.divide_k")?;
    driver.click_widget_id("edit.transform.constant_k")?;
    replace_focused_text(&mut driver, "0")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some());
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|n| !n.enabled));
    driver.click_widget_id("modal.cancel")?;
    Ok(())
}

#[test]
fn b2_r15_gui_03_factorial_anova_rendered_formula_bar_includes_all_k_subsets() -> Result<(), String> {
    let doc = build_mv_factorial_doc();
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    let doc_snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &SheetId::from("res-fac")));
    driver.run_settled();

    // 3 factors (FactorA, FactorB, FactorC) with order 2 -> must list A:B, A:C, and B:C.
    let formula_3f = driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("missing edit.formula_bar")?;
    assert_eq!(
        formula_3f.label, "Response ~ FactorA + FactorB + FactorC + FactorA:FactorB + FactorA:FactorC + FactorB:FactorC",
        "rendered formula bar must include all 2-way pairs"
    );

    // Add 4th factor (FactorD, col 4) via rendered toggle on Experimental Design tab -> all 6 pairs!
    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.fac.col.4")?;
    let formula_4f_ord2 = driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("missing edit.formula_bar after 4th factor")?;
    assert_eq!(
        formula_4f_ord2.label,
        "Response ~ FactorA + FactorB + FactorC + FactorD + FactorA:FactorB + FactorA:FactorC + FactorA:FactorD + FactorB:FactorC + FactorB:FactorD + FactorC:FactorD"
    );

    // Switch to interaction order 3 -> all 6 pairs + all 4 triples!
    driver.click_widget_id("edit.fac.order.3")?;
    let formula_4f_ord3 = driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("missing edit.formula_bar at order 3")?;
    for expected_term in ["FactorA:FactorB:FactorC", "FactorA:FactorB:FactorD", "FactorA:FactorC:FactorD", "FactorB:FactorC:FactorD"] {
        assert!(
            formula_4f_ord3.label.contains(expected_term),
            "order-3 rendered formula bar missing `{expected_term}`; got: {}",
            formula_4f_ord3.label
        );
    }

    driver.click_widget_id("modal.cancel")?;
    Ok(())
}

#[test]
fn b3_r15_gui_16_17_fit_modal_x_domain_seed_initial_scientific_and_bounds_persist() -> Result<(), String> {
    let doc = build_dose_response_xy_doc();
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);

    let fit_id = SheetId::from("res-nlf");
    let doc_snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &fit_id));
    driver.run_settled();

    // 1. R15-GUI-16: Range tab seeds X-domain filter from actual negative log10[M] X data [-9, -4].
    driver.click_widget_id("fit.tab.weights_range")?;
    driver.click_widget_id("fit.wr.x_range_enable")?;

    let xmin_node = driver.frame_meta().find_by_widget_id("fit.wr.x_min").ok_or("missing fit.wr.x_min textbox")?;
    let xmax_node = driver.frame_meta().find_by_widget_id("fit.wr.x_max").ok_or("missing fit.wr.x_max textbox")?;
    assert_eq!(xmin_node.value, Some(json!("-9")));
    assert_eq!(xmax_node.value, Some(json!("-4")));

    // Invalid inverted domain (-3 >= -4) triggers alert and blocks OK.
    driver.click_widget_id("fit.wr.x_min")?;
    replace_focused_text(&mut driver, "-3")?;
    let alert_node = driver.frame_meta().find_by_widget_id("fit.alert.0").ok_or("expected fit.alert.0 for inverted X domain")?;
    assert!(alert_node.label.contains("X min"), "expected X min alert, got: {}", alert_node.label);
    let ok_blocked = driver.frame_meta().find_by_widget_id("modal.ok").ok_or("missing modal.ok")?;
    assert!(!ok_blocked.enabled, "modal.ok must be disabled while X min >= X max");

    // Edit to valid sub-range [-8.5, -4.5].
    driver.click_widget_id("fit.wr.x_min")?;
    replace_focused_text(&mut driver, "-8.5")?;
    driver.click_widget_id("fit.wr.x_max")?;
    replace_focused_text(&mut driver, "-4.5")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|n| n.enabled));

    // 2. R15-GUI-17 (Initial Values tab): data-driven Auto hint + scientific text input `1e-7`.
    driver.click_widget_id("fit.tab.initial_values")?;
    let auto_logec50 = driver.frame_meta().find_by_widget_id("fit.init.auto.LogEC50").ok_or("missing fit.init.auto.LogEC50")?;
    assert!(auto_logec50.label.starts_with("Auto (-"), "expected negative data-driven Auto LogEC50 guess, got: {}", auto_logec50.label);

    driver.click_widget_id("fit.init.input.HillSlope")?;
    replace_focused_text(&mut driver, "1e-7")?;
    let init_input = driver.frame_meta().find_by_widget_id("fit.init.input.HillSlope").ok_or("missing fit.init.input.HillSlope")?;
    assert_eq!(init_input.value, Some(json!("1e-7")));
    let step_val = driver.frame_meta().find_by_widget_id("fit.init.val.HillSlope.val").ok_or("missing fit.init.val.HillSlope.val")?;
    assert_eq!(step_val.value, Some(json!("1e-7")), "small scientific initial value must not truncate to 0.000");

    // 3. R15-GUI-17 (Constrain tab): editable lower & upper bounds + conflict validation.
    driver.click_widget_id("fit.tab.constraints")?;
    driver.click_widget_id("fit.con.lower.Bottom")?;
    replace_focused_text(&mut driver, "0.5")?;
    driver.click_widget_id("fit.con.upper.Bottom")?;
    replace_focused_text(&mut driver, "25")?;

    // Verify inverted bounds (lower=30 >= upper=25) trigger constraint alert and disable OK.
    driver.click_widget_id("fit.con.lower.Bottom")?;
    replace_focused_text(&mut driver, "30")?;
    let con_alert = driver.frame_meta().find_by_widget_id("fit.alert.0").ok_or("expected constraint conflict alert")?;
    assert!(con_alert.label.contains("Constraint conflict"), "unexpected alert: {}", con_alert.label);
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|n| !n.enabled));

    // Restore valid lower bound 0.5 and commit.
    driver.click_widget_id("fit.con.lower.Bottom")?;
    replace_focused_text(&mut driver, "0.5")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|n| n.enabled));
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none(), "unexpected intent error: {:?}", driver.state().last_intent_error);

    // Verify committed NonlinearFitSpec persisted all edited settings.
    let rs = driver.state().app.document().result_sheet(&fit_id).ok_or("missing committed fit ResultSheet")?;
    let AnalysisKind::NonlinearRegression(nl) = &rs.analysis.kind else {
        return Err(String::from("expected NonlinearRegression spec"));
    };
    assert_eq!(nl.x_range_filter, Some((Some(-8.5), Some(-4.5))));

    let hill = nl.parameter_rules.iter().find(|r| r.name == "HillSlope").ok_or("missing HillSlope rule")?;
    assert!(hill.initial_override.is_some_and(|v| (v - 1e-7).abs() < 1e-18), "expected initial_override=1e-7, got {:?}", hill.initial_override);

    let bottom = nl.parameter_rules.iter().find(|r| r.name == "Bottom").ok_or("missing Bottom rule")?;
    assert_eq!(bottom.lower_bound, Some(0.5));
    assert_eq!(bottom.upper_bound, Some(25.0));

    Ok(())
}
