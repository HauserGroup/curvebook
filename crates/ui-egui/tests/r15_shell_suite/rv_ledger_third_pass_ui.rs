//! Wave 8 third-pass review ledger tests (`RV-75..RV-81` UI/UX items and Section 11 `R8` notes).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellValue, ContingencySpec, MixedModelDfMethod, MultiplicityMethod, MultiplicitySpec, NonlinearFitSpec,
    OneWayAnovaSpec, OneWayMethod, OutlierSpec, ResidualCovariance, ResultSheet, SheetId, TableArchetype, TwoGroupMethod, TwoGroupSpec,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::i18n::{CATALOG, LOCALES, catalog_len};
use curvebook_ui_egui::menus::{ShortcutRegistry, ShortcutScope, UI_COMMANDS, normalize_shortcut_combo};
use curvebook_ui_egui::modal_predicates::apply_r2_gaussian_toggle;
use curvebook_ui_egui::{KeyModifiers, UiApp, ViewportMetrics, inspect_ui_tree};
use serde_json::json;

use crate::common::{TestRes, key_down};
use crate::helpers::{exec, gui, ui_cmd};

fn build_column_app(cols: usize, rows: usize) -> Result<(UiApp, SheetId), String> {
    let mut app = UiApp::new();
    let created =
        exec(&mut app, "sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "title": "Groups", "cols": cols, "rows": rows }))?;
    let ds_id = SheetId::try_new(created.payload["sheet_id"].as_str().ok_or("missing sheet_id")?).map_err(|e| e.to_string())?;
    let text = (0..rows)
        .map(|r| (0..cols).map(|c| format!("{:.2}", (r as f64) * 1.5 + (c as f64) * 5.0 + 10.0)).collect::<Vec<_>>().join("\t"))
        .collect::<Vec<_>>()
        .join("\n");
    exec(&mut app, "table.paste", json!({ "sheet_id": ds_id.as_str(), "start_row": 0, "start_col": 0, "text": text }))?;
    Ok((app, ds_id))
}

#[test]
fn rv75_context_scoped_cmd_t_shortcuts_rebind_and_conflicts() -> TestRes {
    assert_eq!((UI_COMMANDS.len(), catalog_len()), (68, 1120));
    assert!(UI_COMMANDS.iter().any(|c| c.id == "ui.editParams.open" && c.shortcut == "Cmd+Shift+P"));
    let ep_i18n = CATALOG.iter().find(|e| e.key == "cmd.ui.editParams.open").ok_or("missing cmd.ui.editParams.open")?;
    assert_eq!(ep_i18n.translations.len(), LOCALES.len());
    assert!(ep_i18n.translations.iter().all(|s| !s.trim().is_empty()));

    let reg = ShortcutRegistry::new();
    let (edit_b, create_b) = (reg.get("ui.editParams.open").ok_or("edit_b")?, reg.get("sheet.createData").ok_or("create_b")?);
    assert_eq!(
        (edit_b.scope, create_b.scope, edit_b.active_combo.as_str(), create_b.active_combo.as_str()),
        (ShortcutScope::Global, ShortcutScope::Global, "Cmd+Shift+P", "Cmd+Shift+T")
    );
    assert_eq!(
        (reg.find_conflict("Cmd+Shift+P", Some("ui.editParams.open")), reg.find_conflict("Cmd+Shift+T", Some("sheet.createData"))),
        (None, None)
    );
    assert!(reg.find_conflict("Cmd+Shift+P", Some("ui.gallery.toggle")).is_some());
    assert!(reg.find_conflict("Cmd+Shift+T", Some("ui.gallery.toggle")).is_some());

    let (mut app, ds_id) = build_column_app(3, 6)?;
    let res = exec(&mut app, "analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "one_way_anova" }))?;
    let res_id = SheetId::try_new(res.payload["sheet_id"].as_str().ok_or("res_id")?).map_err(|e| e.to_string())?;

    // Unscoped distinct defaults: Cmd+Shift+T -> sheet.createData regardless of active sheet.
    exec(&mut app, "sheet.select", json!({ "sheet_id": ds_id.as_str() }))?;
    let ds_before = app.document().data_sheets.len();
    let out_data = key_down(&mut app, "t", KeyModifiers { meta: true, shift: true, ..Default::default() })?;
    assert_eq!(
        (out_data.command_id.as_deref(), app.document().data_sheets.len(), &app.canvas_dialogs.active_dialog),
        (Some("sheet.createData"), ds_before + 1, &ActiveDialog::None)
    );

    // ResultSheet: Ctrl/Cmd+Shift+P -> ui.editParams.open, then rebind to Cmd+Alt+Shift+P.
    exec(&mut app, "sheet.select", json!({ "sheet_id": res_id.as_str() }))?;
    let out_res = key_down(&mut app, "p", KeyModifiers { ctrl: true, shift: true, ..Default::default() })?;
    assert!(
        out_res.command_id.as_deref() == Some("ui.editParams.open")
            && matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. })
    );
    app.canvas_dialogs.close_dialog();

    ui_cmd(&mut app, "ui.shortcut.set", json!({ "command_id": "ui.editParams.open", "combo": "Cmd+Alt+Shift+P" }))?;
    assert!(!key_down(&mut app, "p", KeyModifiers { meta: true, shift: true, ..Default::default() })?.handled);
    let out_rebound = key_down(&mut app, "p", KeyModifiers { meta: true, alt: true, shift: true, ..Default::default() })?;
    assert!(
        out_rebound.command_id.as_deref() == Some("ui.editParams.open")
            && matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. })
    );
    Ok(())
}

#[test]
fn rv76_outlier_dialog_methods_parameters_and_parse_fit_method() -> TestRes {
    // 1) AnalyzeDialog human-path Grubbs configuration.
    let (mut app, ds_id) = build_column_app(2, 8)?;
    app.nav.welcome.open = false;
    exec(&mut app, "sheet.select", json!({ "sheet_id": ds_id.as_str() }))?;
    ui_cmd(&mut app, "ui.analyze.open", json!({}))?;
    app.canvas_dialogs.analyze.select_catalog_entry("stats.outliers");
    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::default());
    for wid in ["analyze.outlier.grubbs", "analyze.outlier.iterative", "analyze.outlier.param.dec"] {
        h.click(wid)?;
    }
    assert!(
        h.app().canvas_dialogs.analyze.outlier_method == "grubbs"
            && h.app().canvas_dialogs.analyze.outlier_iterative
            && (h.app().canvas_dialogs.analyze.outlier_alpha - 0.04).abs() < 1e-9
    );
    assert!(
        matches!(h.app().canvas_dialogs.analyze.build_analysis_kind(), Some(AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative: true })) if (alpha - 0.04).abs() < 1e-9)
    );
    h.click("modal.ok")?;
    assert!(
        matches!(h.app().document().result_sheets.last().ok_or("grubbs rs")?.analysis.kind, AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative: true }) if (alpha - 0.04).abs() < 1e-9)
    );

    // 2) ROUT dialog state -> EditAnalysisParameters switch to Grubbs.
    let (mut app, ds2_id) = build_column_app(2, 8)?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": ds2_id.as_str() }))?;
    ui_cmd(&mut app, "ui.analyze.open", json!({}))?;
    app.canvas_dialogs.analyze.select_catalog_entry("stats.outliers");
    app.canvas_dialogs.analyze.outlier_method = String::from("rout");
    app.canvas_dialogs.analyze.outlier_fdr_q_percent = 3.25;
    gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" }))?;
    let rout_rs = app.document().result_sheets.last().ok_or("rout rs")?;
    assert!(
        matches!(rout_rs.analysis.kind, AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent }) if (fdr_q_percent - 3.25).abs() < 1e-9)
    );
    let rs_id = rout_rs.id.clone();

    exec(&mut app, "sheet.select", json!({ "sheet_id": rs_id.as_str() }))?;
    ui_cmd(&mut app, "ui.editParams.open", json!({ "sheet_id": rs_id.as_str() }))?;
    if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft {
        spec.update_kind(AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.01, iterative: false }));
    }
    gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" }))?;
    assert!(
        matches!(app.document().result_sheet(&rs_id).ok_or("updated")?.analysis.kind, AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative: false }) if (alpha - 0.01).abs() < 1e-9)
    );

    // 3) NonlinearFit ROUT Q% wiring.
    let mut xy_app = UiApp::new();
    let xy_out = exec(&mut xy_app, "sheet.createData", json!({ "sheet_id": "xy-fit", "kind": "xy", "cols": 1, "rows": 6 }))?;
    let xy_id = SheetId::try_new(xy_out.payload["sheet_id"].as_str().ok_or("xy_id")?).map_err(|e| e.to_string())?;
    exec(&mut xy_app, "table.setXCells", json!({ "sheet_id": xy_id.as_str(), "values": [-2.0, -1.0, 0.0, 1.0, 2.0, 3.0] }))?;
    exec(&mut xy_app, "table.paste", json!({ "sheet_id": xy_id.as_str(), "start_row": 0, "start_col": 0, "text": "10\n15\n50\n85\n95\n98" }))?;
    let fit = exec(
        &mut xy_app,
        "analysis.run",
        json!({ "source_sheet_id": xy_id.as_str(), "kind": "nonlinear_regression", "equation_id": "log_agonist_vs_response_4pl" }),
    )?;
    let fit_rs_id = SheetId::try_new(fit.payload["sheet_id"].as_str().ok_or("fit_rs_id")?).map_err(|e| e.to_string())?;
    exec(&mut xy_app, "sheet.select", json!({ "sheet_id": fit_rs_id.as_str() }))?;
    ui_cmd(&mut xy_app, "ui.editParams.open", json!({ "sheet_id": fit_rs_id.as_str() }))?;
    if let Some(spec) = &mut xy_app.canvas_dialogs.edit_params.draft
        && let AnalysisKind::NonlinearRegression(nl) = &mut spec.kind
    {
        nl.method = curvebook_doc::FitMethod::RoutOutlierElimination { fdr_q_percent: 4.5 };
    }
    gui(&mut xy_app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" }))?;
    assert!(
        matches!(xy_app.document().result_sheet(&fit_rs_id).ok_or("fit_rs")?.analysis.kind, AnalysisKind::NonlinearRegression(NonlinearFitSpec { method: curvebook_doc::FitMethod::RoutOutlierElimination { fdr_q_percent }, .. }) if (fdr_q_percent - 4.5).abs() < 1e-9)
    );
    Ok(())
}

#[test]
fn rv77_ui_type_text_preserves_numeric_and_boolean_strings_and_r8_clear_existing() -> TestRes {
    let (mut app, ds_id) = build_column_app(2, 6)?;
    let res = exec(&mut app, "analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "unpaired_t_test" }))?;
    let rs_id = SheetId::try_new(res.payload["sheet_id"].as_str().ok_or("rs_id")?).map_err(|e| e.to_string())?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": rs_id.as_str() }))?;

    ui_cmd(&mut app, "ui.editParams.open", json!({ "sheet_id": rs_id.as_str() }))?;
    for (params, expected) in [
        (json!({ "widget_id": "edit.custom_title", "text": "2024" }), Some("2024")),
        (json!({ "widget_id": "edit.custom_title", "text": "-Run", "clear_existing": false }), Some("2024-Run")),
        (json!({ "widget_id": "custom_title", "text": "true" }), Some("true")),
    ] {
        gui(&mut app, "ui.type_text", params)?;
        assert_eq!(app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.custom_title.as_deref()), expected);
    }
    app.canvas_dialogs.close_dialog();

    exec(&mut app, "sheet.select", json!({ "sheet_id": ds_id.as_str() }))?;
    ui_cmd(&mut app, "ui.analyze.open", json!({}))?;
    gui(&mut app, "ui.type_text", json!({ "widget_id": "analyze.search", "text": "2024" }))?;
    assert_eq!(app.canvas_dialogs.analyze.search_query, "2024");
    gui(&mut app, "ui.type_text", json!({ "widget_id": "analyze.search", "text": " anova", "clear_existing": false }))?;
    assert_eq!(app.canvas_dialogs.analyze.search_query, "2024 anova");
    app.canvas_dialogs.close_dialog();

    ui_cmd(&mut app, "ui.palette.open", json!({}))?;
    gui(&mut app, "ui.type_text", json!({ "widget_id": "palette.query", "text": "true" }))?;
    assert_eq!(app.canvas_dialogs.command_palette.query, "true");
    gui(&mut app, "ui.type_text", json!({ "widget_id": "palette.query", "text": " sheet", "clear_existing": false }))?;
    assert_eq!(app.canvas_dialogs.command_palette.query, "true sheet");
    Ok(())
}

#[test]
fn rv78_assumption_and_matching_toggles_preserve_compatible_submethods_and_reml() -> TestRes {
    for (init, gauss, paired, expected) in [
        (TwoGroupMethod::WelchT, true, false, TwoGroupMethod::WelchT),
        (TwoGroupMethod::RatioPairedT, true, true, TwoGroupMethod::RatioPairedT),
        (TwoGroupMethod::KolmogorovSmirnov, false, false, TwoGroupMethod::KolmogorovSmirnov),
        (TwoGroupMethod::OneSampleT { hypothetical: 42.5 }, false, false, TwoGroupMethod::OneSampleWilcoxon { hypothetical: 42.5 }),
    ] {
        let mut kind = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: init, ..Default::default() });
        apply_r2_gaussian_toggle(&mut kind, gauss, paired, 0);
        assert_eq!(kind, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: expected, ..Default::default() }));
    }

    let (mut app, ds_id) = build_column_app(3, 6)?;
    let tg = exec(&mut app, "analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "welch_t_test" }))?;
    let tg_id = SheetId::try_new(tg.payload["sheet_id"].as_str().ok_or("tg_id")?).map_err(|e| e.to_string())?;
    app.canvas_dialogs.edit_params.open_for_result(&app.document().result_sheet(&tg_id).ok_or("tg")?.clone());

    app.canvas_dialogs.edit_params.set_assume_gaussian(false);
    assert!(
        matches!(app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::MannWhitneyU)
    );
    app.canvas_dialogs.edit_params.set_assume_gaussian(true);
    assert!(
        matches!(app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::WelchT)
    );
    assert!(app.canvas_dialogs.edit_params.welch_correction);

    if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft
        && let AnalysisKind::TwoGroupComparison(t) = &mut spec.kind
    {
        t.method = TwoGroupMethod::RatioPairedT;
    }
    app.canvas_dialogs.edit_params.sync_rule_flags_from_kind();
    app.canvas_dialogs.edit_params.set_paired_or_matched(false);
    app.canvas_dialogs.edit_params.set_paired_or_matched(true);
    assert!(
        matches!(app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::RatioPairedT)
    );

    let mut rm_spec = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
        mixed_effects_reml: true,
        df_method: MixedModelDfMethod::KenwardRoger,
        residual_covariance: ResidualCovariance::Unstructured,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::HolmSidak, ..Default::default() },
        ..Default::default()
    };
    rm_spec.multiplicity.method = MultiplicityMethod::HolmSidak;
    let rm_sheet = ResultSheet::new(AnalysisSpec::new("res-rm", ds_id, AnalysisKind::OneWayAnova(rm_spec)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    app.canvas_dialogs.edit_params.open_for_result(&rm_sheet);
    app.canvas_dialogs.edit_params.set_paired_or_matched(false);
    app.canvas_dialogs.edit_params.set_paired_or_matched(true);
    let Some(AnalysisKind::OneWayAnova(after)) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) else {
        return Err("expected OneWayAnova".into());
    };
    assert_eq!(
        (after.method, after.mixed_effects_reml, after.df_method, after.residual_covariance),
        (OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }, true, MixedModelDfMethod::KenwardRoger, ResidualCovariance::Unstructured)
    );
    Ok(())
}

#[test]
fn rv79_edit_params_open_resets_stale_fields_across_reopens() -> TestRes {
    let (mut app, ds_id) = build_column_app(3, 5)?;
    let mut anova1 = OneWayAnovaSpec::default();
    anova1.multiplicity.method = MultiplicityMethod::Dunnett { control_column: 2 };
    let rs1 = ResultSheet::new(AnalysisSpec::new("rs-dunnett", ds_id.clone(), AnalysisKind::OneWayAnova(anova1)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let rs2 = ResultSheet::new(
        AnalysisSpec::new("rs-cont", ds_id, AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: false, ..Default::default() }))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    let ep = &mut app.canvas_dialogs.edit_params;
    ep.open_for_result(&rs1);
    assert!(ep.compare_against_control && ep.control_column == 2);
    ep.contingency_or_ci_method = "Woolf".into();
    ep.contingency_attributable_risk_checked = true;
    ep.welch_correction = true;
    ep.hodges_lehmann_ci = true;

    ep.open_for_result(&rs2);
    assert_eq!(
        (
            ep.compare_against_control,
            ep.control_column,
            ep.contingency_or_ci_method.as_str(),
            ep.contingency_attributable_risk_checked,
            ep.welch_correction,
            ep.hodges_lehmann_ci
        ),
        (false, 0, "Baptista-Pike", false, false, false)
    );
    Ok(())
}

#[test]
fn r8_shortcuts_plus_shift_f6_ctrl_cmd_and_multiline_x_paste() -> TestRes {
    for (raw, expected) in [("+", "+"), ("Cmd++", "Cmd++"), ("Ctrl + Shift + +", "Ctrl+Shift++")] {
        assert_eq!(normalize_shortcut_combo(raw).map_err(|e| e.to_string())?, expected);
    }
    assert!(normalize_shortcut_combo("Cmd+").is_err() && normalize_shortcut_combo("++").is_err());

    let mut reg = ShortcutRegistry::new();
    reg.set_shortcut("ui.gallery.toggle", "Cmd++").map_err(|e| e.to_string())?;
    assert_eq!(reg.handle_key_combo("Cmd++").map(|m| m.command_id).as_deref(), Some("ui.gallery.toggle"));
    assert_eq!(reg.find_conflict("Shift+F6", Some("ui.gallery.toggle")), Some("ui.pane.cycle"));
    assert!(reg.set_shortcut("ui.gallery.toggle", "Shift+F6").is_err());

    let mut app = UiApp::new();
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "xy-paste", "kind": "xy", "cols": 2, "rows": 4 }))?;
    app.shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Cmd+B").map_err(|e| e.to_string())?;
    let dual = KeyModifiers { ctrl: true, meta: true, ..Default::default() };
    assert_eq!(key_down(&mut app, "b", dual)?.command_id.as_deref(), Some("ui.gallery.toggle"));
    assert!(app.nav.gallery.open);
    key_down(&mut app, "b", dual)?;
    assert!(!app.nav.gallery.open);

    exec(&mut app, "sheet.select", json!({ "sheet_id": "xy-paste" }))?;
    gui(&mut app, "ui.type_text", json!({ "widget_id": "grid.xcell.0.0", "text": "10\t100\t1000\n20\t200\t2000\n30\t300\t3000" }))?;
    let TableArchetype::Xy(xy) = &app.document().data_sheet(&SheetId::from("xy-paste")).ok_or("xy-paste")?.archetype else {
        return Err("expected XY".into());
    };
    assert_eq!(
        (&xy.x_cells[0][0].value, &xy.x_cells[2][0].value, &xy.data_sets[0].cells[0][0].value, &xy.data_sets[1].cells[2][0].value),
        (&CellValue::Number(10.0), &CellValue::Number(30.0), &CellValue::Number(100.0), &CellValue::Number(3000.0))
    );

    let tree = inspect_ui_tree(&app, Some("workspace"), false);
    assert!(tree.find_widget("toolbar.new_table").is_some() || tree.active_sheet_id.is_some());
    Ok(())
}
