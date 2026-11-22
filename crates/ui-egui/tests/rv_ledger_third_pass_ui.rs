//! Wave 8 third-pass review ledger tests (`RV-75..RV-81` UI/UX items and Section 11 `R8` notes).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellValue, ContingencySpec, MixedModelDfMethod, MultiplicityMethod, MultiplicitySpec, NonlinearFitSpec,
    OneWayAnovaSpec, OneWayMethod, OutlierSpec, ResidualCovariance, ResultSheet, SheetId, TableArchetype, TwoGroupMethod, TwoGroupSpec,
};
use curvebook_ui_egui::control_io::{dispatch_gui_action, dispatch_ui_command};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::i18n::{LOCALES, catalog_len, w8_rv_ui};
use curvebook_ui_egui::menus::{ShortcutRegistry, ShortcutScope, UI_COMMANDS, normalize_shortcut_combo};
use curvebook_ui_egui::modal_predicates::apply_r2_gaussian_toggle;
mod human_wf_common;

use curvebook_ui_egui::{KeyModifiers, UiApp, UiInputEvent, ViewportMetrics, dispatch_ui_event, inspect_ui_tree};
use serde_json::json;

type TestRes = Result<(), String>;

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn build_column_app(cols: usize, rows: usize) -> Result<(UiApp, SheetId), String> {
    let mut app = UiApp::new();
    let created = app
        .execute_command("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "title": "Groups", "cols": cols, "rows": rows }))
        .map_err(err_str)?;
    let ds_id = SheetId::try_new(created.payload["sheet_id"].as_str().ok_or("missing sheet_id")?).map_err(err_str)?;
    let mut lines = Vec::new();
    for r in 0..rows {
        let row_vals: Vec<String> = (0..cols).map(|c| format!("{:.2}", (r as f64) * 1.5 + (c as f64) * 5.0 + 10.0)).collect();
        lines.push(row_vals.join("\t"));
    }
    app.execute_command("table.paste", json!({ "sheet_id": ds_id.as_str(), "start_row": 0, "start_col": 0, "text": lines.join("\n") }))
        .map_err(err_str)?;
    Ok((app, ds_id))
}

#[test]
fn rv75_context_scoped_cmd_t_shortcuts_rebind_and_conflicts() -> TestRes {
    // Integrated UI command & i18n invariants:
    assert_eq!(UI_COMMANDS.len(), 53, "UI_COMMANDS length should be 53 after R13.8 Part 2, app.feedback, and R15-GUI-35");
    assert_eq!(catalog_len(), 799, "i18n::catalog_len() should be 799 after Wave 6 cleanup and SEC-16 nav entries");
    assert!(UI_COMMANDS.iter().any(|c| c.id == "ui.editParams.open" && c.shortcut == "Cmd+T"));
    for entry in w8_rv_ui::STAGED_ENTRIES {
        assert_eq!(entry.translations.len(), LOCALES.len());
        assert!(entry.translations.iter().all(|s| !s.trim().is_empty()), "empty locale in {}", entry.key);
    }

    let reg = ShortcutRegistry::new();
    let edit_b = reg.get("ui.editParams.open").ok_or("missing ui.editParams.open binding")?;
    let create_b = reg.get("sheet.createData").ok_or("missing sheet.createData binding")?;
    assert_eq!(edit_b.scope, ShortcutScope::ResultSheet);
    assert_eq!(create_b.scope, ShortcutScope::NonResultSheet);
    assert_eq!(edit_b.active_combo, "Cmd+T");
    assert_eq!(create_b.active_combo, "Cmd+T");

    // Disjoint scopes (`ResultSheet` vs `NonResultSheet`) do not conflict with each other, while `Global` conflicts:
    assert_eq!(reg.find_conflict("Cmd+T", Some("ui.editParams.open")), None);
    assert_eq!(reg.find_conflict("Cmd+T", Some("sheet.createData")), None);
    assert!(reg.find_conflict("Cmd+T", Some("ui.gallery.toggle")).is_some(), "Global binding must detect Cmd+T conflict");

    // Context-aware shortcut resolution in UiApp:
    let (mut app, ds_id) = build_column_app(3, 6)?;
    let res = app.execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "one_way_anova" })).map_err(err_str)?;
    let res_id = SheetId::try_new(res.payload["sheet_id"].as_str().ok_or("missing res_id")?).map_err(err_str)?;

    // 1) On DataSheet: Cmd+T invokes `sheet.createData` (not EditAnalysisParameters).
    app.execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(err_str)?;
    let ds_before = app.document().data_sheets.len();
    let out_data = dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("t"), modifiers: KeyModifiers { meta: true, ..Default::default() } },
        ViewportMetrics::default(),
    )
    .map_err(err_str)?;
    assert_eq!(out_data.command_id.as_deref(), Some("sheet.createData"));
    assert_eq!(app.document().data_sheets.len(), ds_before + 1);
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 2) On ResultSheet: Cmd+T invokes `ui.editParams.open` through ShortcutRegistry.
    app.execute_command("sheet.select", json!({ "sheet_id": res_id.as_str() })).map_err(err_str)?;
    let out_res = dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("t"), modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
        ViewportMetrics::default(),
    )
    .map_err(err_str)?;
    assert_eq!(out_res.command_id.as_deref(), Some("ui.editParams.open"));
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    app.canvas_dialogs.close_dialog();

    // 3) Rebinding `ui.editParams.open` to `Cmd+Alt+Shift+P` routes `Cmd+Alt+Shift+P` on ResultSheet and stops `Cmd+T` from opening it.
    let _ = dispatch_ui_command(&mut app, "ui.shortcut.set", &json!({ "command_id": "ui.editParams.open", "combo": "Cmd+Alt+Shift+P" }))
        .map_err(err_str)?;
    let out_rebound_t = dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("t"), modifiers: KeyModifiers { meta: true, ..Default::default() } },
        ViewportMetrics::default(),
    )
    .map_err(err_str)?;
    assert!(!out_rebound_t.handled, "unbound Cmd+T on ResultSheet must not stay hardcoded");
    let out_rebound_e = dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("p"), modifiers: KeyModifiers { meta: true, alt: true, shift: true, ..Default::default() } },
        ViewportMetrics::default(),
    )
    .map_err(err_str)?;
    assert_eq!(out_rebound_e.command_id.as_deref(), Some("ui.editParams.open"));
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    Ok(())
}

#[test]
fn rv76_outlier_dialog_methods_parameters_and_parse_fit_method() -> TestRes {
    // 1) AnalyzeDialog: configure Grubbs (method + alpha + iterative) for `stats.outliers` & click OK.
    let (mut app, ds_id) = build_column_app(2, 8)?;
    app.nav.welcome.open = false;
    app.execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(err_str)?;
    let _ = dispatch_ui_command(&mut app, "ui.analyze.open", &json!({})).map_err(err_str)?;

    app.canvas_dialogs.analyze.select_catalog_entry("stats.outliers");
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::default());
    harness.click("analyze.outlier.grubbs")?;
    harness.click("analyze.outlier.iterative")?;
    harness.click("analyze.outlier.param.dec")?;
    assert_eq!(harness.app().canvas_dialogs.analyze.outlier_method, "grubbs");
    assert!(harness.app().canvas_dialogs.analyze.outlier_iterative);
    assert!((harness.app().canvas_dialogs.analyze.outlier_alpha - 0.04).abs() < 1e-9);
    let built_grubbs = harness.app().canvas_dialogs.analyze.build_analysis_kind().ok_or("missing built grubbs kind")?;
    assert!(matches!(
        built_grubbs,
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative: true }) if (alpha - 0.04).abs() < 1e-9
    ));
    harness.click("modal.ok")?;
    let grubbs_rs = harness.app().document().result_sheets.last().ok_or("expected Grubbs outlier ResultSheet")?;
    assert!(matches!(
        grubbs_rs.analysis.kind,
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative: true }) if (alpha - 0.04).abs() < 1e-9
    ));

    // Also test JSON patch order-independence (`fdr_q_percent` before `outlier_method`) in AnalyzeDialog.
    let (mut app, ds2_id) = build_column_app(2, 8)?;
    app.execute_command("sheet.select", json!({ "sheet_id": ds2_id.as_str() })).map_err(err_str)?;
    let _ = dispatch_ui_command(&mut app, "ui.analyze.open", &json!({})).map_err(err_str)?;
    dispatch_gui_action(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "entry_id": "stats.outliers",
                "fdr_q_percent": 3.25,
                "outlier_method": "rout"
            }
        }),
    )
    .map_err(err_str)?;
    let rout_rs = app.document().result_sheets.last().ok_or("expected created outlier ResultSheet")?;
    assert!(matches!(
        rout_rs.analysis.kind,
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent }) if (fdr_q_percent - 3.25).abs() < 1e-9
    ));
    let rs_id = rout_rs.id.clone();

    // 2) EditAnalysisParameters on existing OutlierDetection sheet: switch ROUT <-> Grubbs with explicit alpha & Q%.
    app.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() })).map_err(err_str)?;
    let _ = dispatch_ui_command(&mut app, "ui.editParams.open", &json!({ "sheet_id": rs_id.as_str() })).map_err(err_str)?;
    dispatch_gui_action(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "alpha": 0.01,
                "iterative": false,
                "outlier_method": "grubbs"
            }
        }),
    )
    .map_err(err_str)?;
    let updated_rs = app.document().result_sheet(&rs_id).ok_or("missing updated outlier result")?;
    assert!(matches!(
        updated_rs.analysis.kind,
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative: false }) if (alpha - 0.01).abs() < 1e-9
    ));

    // 3) parse_fit_method + NonlinearFit ROUT Q% wiring across object, string, and split-field forms.
    let mut xy_app = UiApp::new();
    let xy_out = xy_app.execute_command("sheet.createData", json!({ "sheet_id": "xy-fit", "kind": "xy", "cols": 1, "rows": 6 })).map_err(err_str)?;
    let xy_id = SheetId::try_new(xy_out.payload["sheet_id"].as_str().ok_or("missing xy_id")?).map_err(err_str)?;
    xy_app.execute_command("table.setXCells", json!({ "sheet_id": xy_id.as_str(), "values": [-2.0, -1.0, 0.0, 1.0, 2.0, 3.0] })).map_err(err_str)?;
    xy_app
        .execute_command("table.paste", json!({ "sheet_id": xy_id.as_str(), "start_row": 0, "start_col": 0, "text": "10\n15\n50\n85\n95\n98" }))
        .map_err(err_str)?;
    let fit_res = xy_app
        .execute_command(
            "analysis.run",
            json!({ "source_sheet_id": xy_id.as_str(), "kind": "nonlinear_regression", "equation_id": "log_agonist_vs_response_4pl" }),
        )
        .map_err(err_str)?;
    let fit_rs_id = SheetId::try_new(fit_res.payload["sheet_id"].as_str().ok_or("missing fit_rs_id")?).map_err(err_str)?;

    xy_app.execute_command("sheet.select", json!({ "sheet_id": fit_rs_id.as_str() })).map_err(err_str)?;
    let _ = dispatch_ui_command(&mut xy_app, "ui.editParams.open", &json!({ "sheet_id": fit_rs_id.as_str() })).map_err(err_str)?;
    dispatch_gui_action(&mut xy_app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "fdr_q_percent": 4.5, "fit_method": "rout" } }))
        .map_err(err_str)?;
    let fit_rs = xy_app.document().result_sheet(&fit_rs_id).ok_or("missing fit_rs")?;
    assert!(matches!(
        fit_rs.analysis.kind,
        AnalysisKind::NonlinearRegression(NonlinearFitSpec {
            method: curvebook_doc::FitMethod::RoutOutlierElimination { fdr_q_percent },
            ..
        }) if (fdr_q_percent - 4.5).abs() < 1e-9
    ));

    Ok(())
}

#[test]
fn rv76_fit_method_shorthands_parse_cleanly() -> TestRes {
    let (mut app, ds_id) = build_column_app(1, 5)?;
    app.execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(err_str)?;
    let _ = dispatch_ui_command(&mut app, "ui.analyze.open", &json!({})).map_err(err_str)?;
    let mut patch_obj = serde_json::Map::new();
    patch_obj.insert(String::from("entry_id"), json!("fit.nonlinear"));
    patch_obj.insert(String::from("fit_method"), json!({ "method": "rout", "fdr_q_percent": 3.2 }));
    app.canvas_dialogs.analyze.apply_field_patch(&patch_obj)?;
    assert_eq!(app.canvas_dialogs.analyze.fit_method, curvebook_doc::FitMethod::RoutOutlierElimination { fdr_q_percent: 3.2 });

    let mut patch_str = serde_json::Map::new();
    patch_str.insert(String::from("fit_method"), json!("rout:2.75"));
    app.canvas_dialogs.analyze.apply_field_patch(&patch_str)?;
    assert_eq!(app.canvas_dialogs.analyze.fit_method, curvebook_doc::FitMethod::RoutOutlierElimination { fdr_q_percent: 2.75 });
    Ok(())
}

#[test]
fn rv77_ui_type_text_preserves_numeric_and_boolean_strings_and_r8_clear_existing() -> TestRes {
    let (mut app, ds_id) = build_column_app(2, 6)?;
    let res = app.execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "unpaired_t_test" })).map_err(err_str)?;
    let rs_id = SheetId::try_new(res.payload["sheet_id"].as_str().ok_or("missing rs_id")?).map_err(err_str)?;
    app.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() })).map_err(err_str)?;

    // Type numeric-looking and boolean-looking strings into custom_title without losing string type.
    let _ = dispatch_ui_command(&mut app, "ui.editParams.open", &json!({ "sheet_id": rs_id.as_str() })).map_err(err_str)?;
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "edit.custom_title", "text": "2024" })).map_err(err_str)?;
    assert_eq!(
        app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.custom_title.as_deref()),
        Some("2024"),
        "typing \"2024\" into custom_title must preserve string"
    );
    // R8: clear_existing: false appends instead of wiping.
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "edit.custom_title", "text": "-Run", "clear_existing": false }))
        .map_err(err_str)?;
    assert_eq!(app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.custom_title.as_deref()), Some("2024-Run"));

    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "custom_title", "text": "true" })).map_err(err_str)?;
    assert_eq!(
        app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.custom_title.as_deref()),
        Some("true"),
        "typing \"true\" into custom_title must preserve string"
    );
    app.canvas_dialogs.close_dialog();

    // Check Analyze search box + CommandPalette clear_existing: false.
    app.execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(err_str)?;
    let _ = dispatch_ui_command(&mut app, "ui.analyze.open", &json!({})).map_err(err_str)?;
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "analyze.search", "text": "2024" })).map_err(err_str)?;
    assert_eq!(app.canvas_dialogs.analyze.search_query, "2024");
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "analyze.search", "text": " anova", "clear_existing": false }))
        .map_err(err_str)?;
    assert_eq!(app.canvas_dialogs.analyze.search_query, "2024 anova");
    app.canvas_dialogs.close_dialog();

    let _ = dispatch_ui_command(&mut app, "ui.palette.open", &json!({})).map_err(err_str)?;
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "palette.query", "text": "true" })).map_err(err_str)?;
    assert_eq!(app.canvas_dialogs.command_palette.query, "true");
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "palette.query", "text": " sheet", "clear_existing": false }))
        .map_err(err_str)?;
    assert_eq!(app.canvas_dialogs.command_palette.query, "true sheet");
    Ok(())
}

#[test]
fn rv78_assumption_and_matching_toggles_preserve_compatible_submethods_and_reml() -> TestRes {
    // 1) Direct apply_r2_gaussian_toggle preserves compatible submethods cleanly.
    let mut welch_kind = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() });
    apply_r2_gaussian_toggle(&mut welch_kind, true, false, 0);
    assert!(matches!(welch_kind, AnalysisKind::TwoGroupComparison(ref t) if t.method == TwoGroupMethod::WelchT));

    let mut ratio_kind = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::RatioPairedT, ..TwoGroupSpec::default() });
    apply_r2_gaussian_toggle(&mut ratio_kind, true, true, 0);
    assert!(matches!(ratio_kind, AnalysisKind::TwoGroupComparison(ref t) if t.method == TwoGroupMethod::RatioPairedT));

    let mut ks_kind = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::KolmogorovSmirnov, ..TwoGroupSpec::default() });
    apply_r2_gaussian_toggle(&mut ks_kind, false, false, 0);
    assert!(matches!(ks_kind, AnalysisKind::TwoGroupComparison(ref t) if t.method == TwoGroupMethod::KolmogorovSmirnov));

    let mut one_sample =
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 42.5 }, ..TwoGroupSpec::default() });
    apply_r2_gaussian_toggle(&mut one_sample, false, false, 0);
    assert!(matches!(
        one_sample,
        AnalysisKind::TwoGroupComparison(ref t)
            if matches!(t.method, TwoGroupMethod::OneSampleWilcoxon { hypothetical } if (hypothetical - 42.5).abs() < 1e-9)
    ));

    // 2) Full EditAnalysisParameters round-trip preserves Welch, RatioPaired, Lognormal, KS, and REML sphericity.
    let (mut app, ds_id) = build_column_app(3, 6)?;
    let tg_res = app.execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "welch_t_test" })).map_err(err_str)?;
    let tg_id = SheetId::try_new(tg_res.payload["sheet_id"].as_str().ok_or("missing tg_id")?).map_err(err_str)?;
    let rs_tg = app.document().result_sheet(&tg_id).ok_or("missing tg sheet")?.clone();
    app.canvas_dialogs.edit_params.open_for_result(&rs_tg);

    // Welch -> nonparam -> Gaussian back -> restores WelchT!
    app.canvas_dialogs.edit_params.apply_field_patch(&serde_json::Map::from_iter([(String::from("assume_gaussian"), json!(false))]))?;
    assert!(matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::MannWhitneyU
    ));
    app.canvas_dialogs.edit_params.apply_field_patch(&serde_json::Map::from_iter([(String::from("assume_gaussian"), json!(true))]))?;
    assert!(matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::WelchT
    ));
    assert!(app.canvas_dialogs.edit_params.welch_correction);

    // RatioPaired -> unpaired -> paired back -> restores RatioPairedT!
    app.canvas_dialogs.edit_params.apply_field_patch(&serde_json::Map::from_iter([(String::from("two_group_method"), json!("ratio_paired"))]))?;
    app.canvas_dialogs.edit_params.apply_field_patch(&serde_json::Map::from_iter([(String::from("paired_or_matched"), json!(false))]))?;
    app.canvas_dialogs.edit_params.apply_field_patch(&serde_json::Map::from_iter([(String::from("paired_or_matched"), json!(true))]))?;
    assert!(matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::RatioPairedT
    ));

    // OneWay REML with geisser_greenhouse=false preserves REML + df_method + residual_covariance + gg=false across toggle!
    let mut rm_spec = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
        mixed_effects_reml: true,
        df_method: MixedModelDfMethod::KenwardRoger,
        residual_covariance: ResidualCovariance::Unstructured,
        multiplicity: MultiplicitySpec::default(),
        ..OneWayAnovaSpec::default()
    };
    rm_spec.multiplicity.method = MultiplicityMethod::HolmSidak;
    let rm_analysis = AnalysisSpec::new("res-rm", ds_id, AnalysisKind::OneWayAnova(rm_spec)).map_err(err_str)?;
    let rm_sheet = ResultSheet::new(rm_analysis).map_err(err_str)?;
    app.canvas_dialogs.edit_params.open_for_result(&rm_sheet);
    app.canvas_dialogs.edit_params.apply_field_patch(&serde_json::Map::from_iter([(String::from("paired_or_matched"), json!(false))]))?;
    app.canvas_dialogs.edit_params.apply_field_patch(&serde_json::Map::from_iter([(String::from("paired_or_matched"), json!(true))]))?;
    let Some(AnalysisKind::OneWayAnova(after_rm)) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) else {
        return Err(String::from("expected OneWayAnova after RM round-trip"));
    };
    assert_eq!(after_rm.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false });
    assert!(after_rm.mixed_effects_reml);
    assert_eq!(after_rm.df_method, MixedModelDfMethod::KenwardRoger);
    assert_eq!(after_rm.residual_covariance, ResidualCovariance::Unstructured);
    Ok(())
}

#[test]
fn rv79_edit_params_open_resets_stale_fields_across_reopens() -> TestRes {
    let mut app = UiApp::new();
    let _ = app.execute_command("sheet.createData", json!({ "sheet_id": "col-a", "kind": "column", "cols": 3, "rows": 5 })).map_err(err_str)?;
    app.execute_command(
        "table.paste",
        json!({ "sheet_id": "col-a", "start_row": 0, "start_col": 0, "text": "1\t2\t3\n2\t3\t4\n3\t4\t5\n4\t5\t6\n5\t6\t7" }),
    )
    .map_err(err_str)?;
    let ds_id = SheetId::from("col-a");

    // Sheet 1: Dunnett control_column = 2.
    let mut anova1 = OneWayAnovaSpec::default();
    anova1.multiplicity.method = MultiplicityMethod::Dunnett { control_column: 2 };
    let rs1 =
        ResultSheet::new(AnalysisSpec::new("rs-dunnett", ds_id.clone(), AnalysisKind::OneWayAnova(anova1)).map_err(err_str)?).map_err(err_str)?;

    // Sheet 2: Contingency (no attributable risk / default OR CI method).
    let cont = ContingencySpec { compute_odds_and_risk_ratios: false, ..ContingencySpec::default() };
    let rs2 = ResultSheet::new(AnalysisSpec::new("rs-cont", ds_id, AnalysisKind::Contingency(cont)).map_err(err_str)?).map_err(err_str)?;

    app.canvas_dialogs.edit_params.open_for_result(&rs1);
    assert!(app.canvas_dialogs.edit_params.compare_against_control);
    assert_eq!(app.canvas_dialogs.edit_params.control_column, 2);
    app.canvas_dialogs.edit_params.contingency_or_ci_method = String::from("Woolf");
    app.canvas_dialogs.edit_params.contingency_attributable_risk_checked = true;
    app.canvas_dialogs.edit_params.welch_correction = true;
    app.canvas_dialogs.edit_params.hodges_lehmann_ci = true;

    // Reopening for Sheet 2 must reset ALL unrelated/prior state cleanly back to defaults!
    app.canvas_dialogs.edit_params.open_for_result(&rs2);
    assert!(!app.canvas_dialogs.edit_params.compare_against_control);
    assert_eq!(app.canvas_dialogs.edit_params.control_column, 0);
    assert_eq!(app.canvas_dialogs.edit_params.contingency_or_ci_method, "Baptista-Pike");
    assert!(!app.canvas_dialogs.edit_params.contingency_attributable_risk_checked);
    assert!(!app.canvas_dialogs.edit_params.welch_correction);
    assert!(!app.canvas_dialogs.edit_params.hodges_lehmann_ci);
    Ok(())
}

#[test]
fn r8_shortcuts_plus_shift_f6_ctrl_cmd_and_multiline_x_paste() -> TestRes {
    // 1. `+` shortcut normalization & binding.
    assert_eq!(normalize_shortcut_combo("+").map_err(err_str)?, "+");
    assert_eq!(normalize_shortcut_combo("Cmd++").map_err(err_str)?, "Cmd++");
    assert_eq!(normalize_shortcut_combo("Ctrl + Shift + +").map_err(err_str)?, "Ctrl+Shift++");
    assert!(normalize_shortcut_combo("Cmd+").is_err(), "dangling trailing + must error");
    assert!(normalize_shortcut_combo("++").is_err(), "missing modifier before + must error");

    let mut reg = ShortcutRegistry::new();
    reg.set_shortcut("ui.gallery.toggle", "Cmd++").map_err(err_str)?;
    assert_eq!(reg.handle_key_combo("Cmd++").map(|m| m.command_id).as_deref(), Some("ui.gallery.toggle"));

    // 2. Shift+F6 conflicts with implicit reverse `ui.pane.cycle`.
    assert_eq!(reg.find_conflict("Shift+F6", Some("ui.gallery.toggle")), Some("ui.pane.cycle"));
    assert!(reg.set_shortcut("ui.gallery.toggle", "Shift+F6").is_err(), "Shift+F6 must conflict with ui.pane.cycle");

    // 3. Ctrl+Cmd dual modifiers are preserved (never dropping Ctrl when Meta is held).
    let mut app = UiApp::new();
    let _ = app.execute_command("sheet.createData", json!({ "sheet_id": "xy-paste", "kind": "xy", "cols": 2, "rows": 4 })).map_err(err_str)?;
    app.shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Cmd+B").map_err(err_str)?;
    let combo_out = dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("b"), modifiers: KeyModifiers { ctrl: true, meta: true, ..Default::default() } },
        ViewportMetrics::default(),
    )
    .map_err(err_str)?;
    assert_eq!(combo_out.command_id.as_deref(), Some("ui.gallery.toggle"), "Ctrl+Cmd+B must preserve both Ctrl and Cmd");

    // 4. Multi-line, multi-column paste into an XY X cell pastes both X values and spilling Y columns.
    app.execute_command("sheet.select", json!({ "sheet_id": "xy-paste" })).map_err(err_str)?;
    let _ = dispatch_gui_action(
        &mut app,
        "ui.type_text",
        json!({ "widget_id": "grid.xcell.0.0", "text": "10\t100\t1000\n20\t200\t2000\n30\t300\t3000" }),
    )
    .map_err(err_str)?;
    let ds = app.document().data_sheet(&SheetId::from("xy-paste")).ok_or("missing xy-paste")?;
    let TableArchetype::Xy(xy) = &ds.archetype else {
        return Err(String::from("expected XY archetype"));
    };
    assert_eq!(xy.x_cells[0][0].value, CellValue::Number(10.0));
    assert_eq!(xy.x_cells[2][0].value, CellValue::Number(30.0));
    assert_eq!(xy.data_sets[0].cells[0][0].value, CellValue::Number(100.0));
    assert_eq!(xy.data_sets[1].cells[2][0].value, CellValue::Number(3000.0));

    // Also inspect widget metadata to ensure build remains valid.
    let tree = inspect_ui_tree(&app, Some("workspace"), false);
    assert!(tree.find_widget("toolbar.new_table").is_some() || tree.active_sheet_id.is_some());
    Ok(())
}
