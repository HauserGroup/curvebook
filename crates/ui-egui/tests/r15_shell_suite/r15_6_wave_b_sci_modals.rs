#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `R15.6` Wave B (`B1`, `B2`, `B3`) human-path GUI regression suite (`R15-GUI-03`, `09`, `10`, `16`, `17`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, DataSheet, FactorialAnovaSpec, MathTransformPreset, MultipleVariablesTable, NonlinearFitSpec,
    ProjectDocument, ResultSheet, SheetId, TableArchetype, TransformAnalysisSpec, VariableRole,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::UiIntent;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use serde_json::json;

use crate::common::{commit_shell_intents, make_doc_driver, seed_xy_doc, set_textbox};

fn build_pharma_xy_doc() -> ProjectDocument {
    seed_xy_doc("Wave B Pharmacology", "xy-pharma", "Pharma XY", "Conc (uM)", &[0.01, 0.1, 1.0, 10.0], &[("Response", &[10.0, 35.0, 75.0, 98.0])])
        .unwrap()
        .0
}

#[test]
fn b1_r15_gui_09_standalone_pharmacology_transform_y_none_does_not_apply_log10() -> Result<(), String> {
    let mut driver = make_doc_driver(build_pharma_xy_doc(), 1280.0, 800.0);
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("xy-pharma"), kind: TransformModalKind::TransformValues });
    commit_shell_intents(&mut driver)?;

    driver.click_widget_id("edit.transform.mode.pharmacology")?;
    driver.click_widget_id("edit.transform.y_transform.none")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("edit.transform.y_transform.none").ok_or("missing none")?.value, Some(json!("true")));
    assert_eq!(driver.frame_meta().find_by_widget_id("edit.transform.y_transform.log10").ok_or("missing log10")?.value, Some(json!("false")));
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.zero_conc_sub").is_none());
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.create_graph").is_none());

    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none());
    let rs = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { .. })))
        .ok_or("missing Transform ResultSheet")?;
    let AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, .. }) = &rs.analysis.kind else {
        return Err(String::from("expected Math transform spec"));
    };
    assert_eq!(*x_transform, Some(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 }));
    assert_eq!(*y_transform, None);
    let transformed = rs.analyzable_data.as_ref().ok_or("missing analyzable_data")?;
    let TableArchetype::Xy(xy) = &transformed.archetype else { return Err(String::from("expected XY")) };
    assert!((xy.raw_x_value_at(2).unwrap() - 6.0).abs() < 1e-9);
    for (r, expected_y) in [10.0, 35.0, 75.0, 98.0].into_iter().enumerate() {
        assert!((transformed.cell(CellCoord::new(r, 0, 0)).unwrap().active_number().unwrap() - expected_y).abs() < 1e-9);
    }
    Ok(())
}

#[test]
fn b1_r15_gui_10_constant_k_validation_and_commit_time_linear_scale_in_both_modes() -> Result<(), String> {
    let mut driver = make_doc_driver(build_pharma_xy_doc(), 1280.0, 800.0);
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::from("xy-pharma"), kind: TransformModalKind::TransformValues });
    commit_shell_intents(&mut driver)?;

    driver.click_widget_id("edit.transform.y_transform.divide_k")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("edit.transform.y_transform.divide_k").ok_or("missing div")?.value, Some(json!("true")));
    assert_eq!(driver.frame_meta().find_by_widget_id("edit.transform.y_transform.log10").ok_or("missing log10")?.value, Some(json!("false")));

    set_textbox(&mut driver, "edit.transform.constant_k", "0")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some());
    let ok_node = driver.frame_meta().find_by_widget_id("modal.ok").ok_or("missing modal.ok")?.clone();
    assert!(!ok_node.enabled && driver.click_widget_id("modal.ok").is_err());
    driver.click_at(ok_node.clipped_rect.center())?;
    assert_eq!(driver.state().views.transform_modals.open_kind, Some(TransformModalKind::TransformValues));
    assert!(driver.state().app.document().result_sheets.is_empty());

    driver.click_widget_id("edit.transform.y_transform.multiply_k")?;
    set_textbox(&mut driver, "edit.transform.constant_k", "2")?;
    assert!(driver.state().views.transform_modals.validation_error.is_none());
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none());

    let rs_id = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { .. })))
        .map(|rs| rs.id.clone())
        .ok_or("missing Transform ResultSheet")?;
    let assert_slope = |app: &curvebook_ui_egui::UiApp, slope: f64, y0: f64| {
        let rs = app.document().result_sheet(&rs_id).unwrap();
        assert!(
            matches!(&rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform: Some(MathTransformPreset::LinearScale { slope: s, intercept: 0.0 }), .. }) if (*s - slope).abs() < 1e-12)
        );
        assert!((rs.analyzable_data.as_ref().unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap() - y0).abs() < 1e-9);
    };
    assert_slope(&driver.state().app, 2.0, 20.0);

    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() })).map_err(|e| e.to_string())?;
    driver.run_settled();
    driver.state_mut().intents.push_ui("ui.editParams.open", json!({}));
    commit_shell_intents(&mut driver)?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    driver.click_widget_id("edit.transform.y_transform.multiply_k")?;
    set_textbox(&mut driver, "edit.transform.constant_k", "3")?;
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none());
    assert_slope(&driver.state().app, 3.0, 30.0);

    driver.state_mut().intents.push_ui("ui.editParams.open", json!({}));
    commit_shell_intents(&mut driver)?;
    driver.click_widget_id("edit.transform.y_transform.divide_k")?;
    set_textbox(&mut driver, "edit.transform.constant_k", "0")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some());
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|n| !n.enabled));
    driver.click_widget_id("modal.cancel")?;
    Ok(())
}

#[test]
fn b2_r15_gui_03_factorial_anova_rendered_formula_bar_includes_all_k_subsets() -> Result<(), String> {
    let mut doc = ProjectDocument::new("Wave B Factorial").unwrap();
    let mut mv = MultipleVariablesTable::new(5, 4).unwrap();
    for (i, (t, r)) in [
        ("Response", VariableRole::Continuous),
        ("FactorA", VariableRole::Categorical),
        ("FactorB", VariableRole::Categorical),
        ("FactorC", VariableRole::Categorical),
        ("FactorD", VariableRole::Categorical),
    ]
    .into_iter()
    .enumerate()
    {
        mv.variables[i].title = t.to_owned();
        mv.variables[i].role = r;
    }
    let mv_id = SheetId::try_new("mv-fac").unwrap();
    doc.data_sheets.push(DataSheet::new(mv_id.clone(), "MV Factorial", TableArchetype::MultipleVariables(mv)).unwrap());
    let fac = FactorialAnovaSpec { response_column: Some(0), multifactor_columns: vec![1, 2, 3], max_interaction_order: 2, ..Default::default() };
    let mut spec = AnalysisSpec::new("Factorial ANOVA", mv_id, AnalysisKind::FactorialAnova(fac)).unwrap();
    spec.id = SheetId::try_new("res-fac").unwrap();
    doc.result_sheets.push(ResultSheet::new(spec).unwrap());
    doc.active_sheet_id = Some(SheetId::from("mv-fac"));

    let mut driver = make_doc_driver(doc, 1280.0, 800.0);
    let snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap, &SheetId::from("res-fac")));
    driver.run_settled();

    assert_eq!(
        driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("missing formula")?.label,
        "Response ~ FactorA + FactorB + FactorC + FactorA:FactorB + FactorA:FactorC + FactorB:FactorC"
    );
    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.fac.col.4")?;
    assert_eq!(
        driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("missing formula")?.label,
        "Response ~ FactorA + FactorB + FactorC + FactorD + FactorA:FactorB + FactorA:FactorC + FactorA:FactorD + FactorB:FactorC + FactorB:FactorD + FactorC:FactorD"
    );
    driver.click_widget_id("edit.fac.order.3")?;
    let f3 = &driver.frame_meta().find_by_widget_id("edit.formula_bar").ok_or("missing formula")?.label;
    for term in ["FactorA:FactorB:FactorC", "FactorA:FactorB:FactorD", "FactorA:FactorC:FactorD", "FactorB:FactorC:FactorD"] {
        assert!(f3.contains(term), "missing `{term}` in `{f3}`");
    }
    driver.click_widget_id("modal.cancel")?;
    Ok(())
}

#[test]
fn b3_r15_gui_16_17_fit_modal_x_domain_seed_initial_scientific_and_bounds_persist() -> Result<(), String> {
    let (mut doc, xy_id) = seed_xy_doc(
        "Wave B Dose Response",
        "xy-dose",
        "Dose Response",
        "Log[Agonist] (M)",
        &[-9.0, -8.0, -7.0, -6.0, -5.0, -4.0],
        &[("Receptor", &[12.0, 22.0, 51.0, 82.0, 96.0, 99.0])],
    )?;
    let fit_id = SheetId::try_new("res-nlf").unwrap();
    let mut spec =
        AnalysisSpec::new("4PL Fit", xy_id, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).unwrap();
    spec.id = fit_id.clone();
    doc.result_sheets.push(ResultSheet::new(spec).unwrap());

    let mut driver = make_doc_driver(doc, 1280.0, 800.0);
    let snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap, &fit_id));
    driver.run_settled();

    driver.click_widget_id("fit.tab.weights_range")?;
    driver.click_widget_id("fit.wr.x_range_enable")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("fit.wr.x_min").ok_or("missing x_min")?.value, Some(json!("-9")));
    assert_eq!(driver.frame_meta().find_by_widget_id("fit.wr.x_max").ok_or("missing x_max")?.value, Some(json!("-4")));

    set_textbox(&mut driver, "fit.wr.x_min", "-3")?;
    assert!(driver.frame_meta().find_by_widget_id("fit.alert.0").ok_or("missing alert")?.label.contains("X min"));
    assert!(!driver.frame_meta().find_by_widget_id("modal.ok").ok_or("missing ok")?.enabled);
    set_textbox(&mut driver, "fit.wr.x_min", "-8.5")?;
    set_textbox(&mut driver, "fit.wr.x_max", "-4.5")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|n| n.enabled));

    driver.click_widget_id("fit.tab.initial_values")?;
    assert!(driver.frame_meta().find_by_widget_id("fit.init.auto.LogEC50").ok_or("missing auto")?.label.starts_with("Auto (-"));
    set_textbox(&mut driver, "fit.init.input.HillSlope", "1e-7")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("fit.init.input.HillSlope").ok_or("missing init")?.value, Some(json!("1e-7")));
    assert_eq!(driver.frame_meta().find_by_widget_id("fit.init.val.HillSlope.val").ok_or("missing step")?.value, Some(json!("1e-7")));

    driver.click_widget_id("fit.tab.constraints")?;
    set_textbox(&mut driver, "fit.con.lower.Bottom", "0.5")?;
    set_textbox(&mut driver, "fit.con.upper.Bottom", "25")?;
    set_textbox(&mut driver, "fit.con.lower.Bottom", "30")?;
    assert!(driver.frame_meta().find_by_widget_id("fit.alert.0").ok_or("missing con alert")?.label.contains("Constraint conflict"));
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|n| !n.enabled));
    set_textbox(&mut driver, "fit.con.lower.Bottom", "0.5")?;
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none());

    let rs = driver.state().app.document().result_sheet(&fit_id).ok_or("missing fit rs")?;
    let AnalysisKind::NonlinearRegression(nl) = &rs.analysis.kind else { return Err(String::from("expected NonlinearRegression")) };
    assert_eq!(nl.x_range_filter, Some((Some(-8.5), Some(-4.5))));
    let hill = nl.parameter_rules.iter().find(|r| r.name == "HillSlope").ok_or("missing HillSlope")?;
    let bottom = nl.parameter_rules.iter().find(|r| r.name == "Bottom").ok_or("missing Bottom")?;
    assert!(hill.initial_override.is_some_and(|v| (v - 1e-7).abs() < 1e-18));
    assert_eq!((bottom.lower_bound, bottom.upper_bound), (Some(0.5), Some(25.0)));
    Ok(())
}
