#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `R10.28` Wave B Modal & Wizard GUI gap-fix regression suite (`EguiHumanPathDriver`):
//! `WF-04`, `WF-15`, `WF-19`, `WF-36`/`43`/`52`, `WF-61`, `WF-06`, and `WF-10`.

use std::sync::Arc;

use crate::common::{ShellTestState, TestRes, cell_number, commit_shell_intents, make_shell_driver, seed_column_doc, seed_xy_doc, set_textbox};
use crate::suite_helpers::{assert_widget_enabled, assert_widgets, build_mv_sheet, click_all};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ContingencySpec, ContingencyTable, DataSheet, GroupedTable, MultiplicityMethod,
    NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, ParameterScope, PowerAnalysisSpec, PowerEffectSpec, PowerSolveTarget, PowerTailMode,
    PowerTestFamily, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec,
    VariableRole,
};
use curvebook_engine::Session;
use curvebook_stats::power::PowerTestFamily as StatsPowerTestFamily;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::os_integration::{PlatformClipboardAdapter, StubPlatformClipboardAdapter};
use curvebook_ui_egui::{PowerWizardStage, UiApp};

fn make_modal_driver(app: UiApp, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    make_shell_driver(app, ViewportMetrics::new(1280.0, h, 1.0))
}

fn add_result_sheet(doc: &mut ProjectDocument, id: &str, title: &str, src: SheetId, cols: Vec<usize>, kind: AnalysisKind) -> SheetId {
    let rs_id = SheetId::try_new(id).unwrap();
    let mut spec = AnalysisSpec::new(title, src, kind).unwrap();
    spec.id = rs_id.clone();
    spec.selected_data_sets = cols;
    doc.result_sheets.push(ResultSheet::new(spec).unwrap());
    rs_id
}

fn open_edit_driver(doc: ProjectDocument, rs_id: &SheetId, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));
    assert!(app.canvas_dialogs.open_edit_parameters(&doc, rs_id));
    make_modal_driver(app, h)
}

#[test]
fn r10_28_wf04_import_reshape_tidy_long_clipboard_and_empty_doc_import_wizard() -> TestRes {
    // Part A: ImportReshape TidyLongPivot + target_kind + Paste Clipboard + x/rep/excl inputs.
    let (doc, _) = seed_column_doc("WF04 Import", "ds-seed", "Seed", ColumnDesignHint::UnpairedReplicates, &[("A", &[])])?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    let clipboard = Arc::new(StubPlatformClipboardAdapter::new());
    clipboard.write_text("X,Group,Rep,Excl,Val\n1,Ctrl,R1,0,10.5\n1,Ctrl,R2,0,11.5\n2,Drug,R1,0,22.0\n2,Drug,R2,1,99.0\n")?;
    app.set_clipboard_adapter(clipboard);
    app.canvas_dialogs.open_import_reshape();

    let mut driver = make_modal_driver(app, 820.0);
    click_all(&mut driver, &["import_paste.source.tidy", "import_paste.target.xy"])?;
    assert_eq!(driver.state().app.grid.import_reshape.target_kind, TableKind::Xy);
    click_all(&mut driver, &["import_paste.target.grouped", "import_paste.delim.comma", "import_paste.btn.paste_clipboard"])?;
    assert_eq!(
        (driver.state().app.grid.import_reshape.target_kind, driver.state().app.grid.import_reshape.tidy_delimiter),
        (TableKind::Grouped, ',')
    );
    assert!(driver.state().app.grid.import_reshape.tidy_csv.contains("Ctrl,R1,0,10.5"));
    assert_eq!(driver.state().views.import_wizard.reshape_tidy_csv_draft, driver.state().app.grid.import_reshape.tidy_csv);

    for (wid, val) in [
        ("import_paste.input.x_col", "X"),
        ("import_paste.input.group_col", "Group"),
        ("import_paste.input.replicate_col", "Rep"),
        ("import_paste.input.value_col", "Val"),
        ("import_paste.input.excluded_col", "Excl"),
    ] {
        set_textbox(&mut driver, wid, val)?;
    }
    assert_widgets(&driver, &["import_paste.input.csv"])?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!((driver.state().last_intent_error.as_ref(), &driver.state().app.canvas_dialogs.active_dialog), (None, &ActiveDialog::None));
    assert_eq!(driver.state().app.document().data_sheets.len(), 2);
    assert_eq!(driver.state().app.document().data_sheets[1].archetype.kind(), TableKind::Grouped);

    // Part B: Empty document CommitImportWizard with Clipboard origin.
    let mut app_empty = UiApp::from_session(Session::from_document(ProjectDocument::new("Empty Doc Import").unwrap()));
    let clip2 = Arc::new(StubPlatformClipboardAdapter::new());
    clip2.write_text("10\t20\n30\t40\n")?;
    app_empty.set_clipboard_adapter(clip2);
    app_empty.grid.import_wizard.open = true;
    app_empty.grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;

    let mut driver2 = make_modal_driver(app_empty, 820.0);
    driver2.click_widget_id("import_paste.btn.paste_clipboard")?;
    assert!(driver2.state().app.grid.import_wizard.raw_clipboard_text.contains("30\t40"));
    driver2.click_widget_id("modal.ok")?;
    assert!(driver2.state().last_intent_error.is_none() && !driver2.state().app.grid.import_wizard.open);
    assert_eq!(driver2.state().app.document().data_sheets.len(), 1);
    let created = &driver2.state().app.document().data_sheets[0];
    assert_eq!((cell_number(created, 0, 0), cell_number(created, 1, 1)), (Some(10.0), Some(40.0)));
    Ok(())
}

#[test]
fn r10_28_wf15_power_studio_family_aliases_and_cmd_t_reentry() -> TestRes {
    let mut app = UiApp::from_session(Session::from_document(ProjectDocument::new("WF15 Power").unwrap()));
    app.canvas_dialogs.open_power_wizard();
    let mut driver = make_modal_driver(app, 820.0);
    for (wid, fam) in [
        ("power.family.one_way_anova", StatsPowerTestFamily::OneWayAnova),
        ("power.family.simple_linear", StatsPowerTestFamily::SimpleLinearRegression),
        ("power.family.two_sample_t", StatsPowerTestFamily::TwoSampleUnpairedT),
    ] {
        driver.click_widget_id(wid)?;
        assert_eq!(driver.state().app.canvas_dialogs.power_wizard.family, fam);
    }
    click_all(&mut driver, &["modal.next", "power.effect_mode.pilot", "power.effect_mode.direct", "modal.next", "modal.next"])?;
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.stage, PowerWizardStage::Results);
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let power_rs = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::PowerAnalysis(_)))
        .expect("PowerAnalysis ResultSheet")
        .clone();
    let power_rs_id = power_rs.id.clone();

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
    let mut edit_spec =
        AnalysisSpec::new("Power Plan", power_rs.analysis.source_sheet_id.expect("power source"), AnalysisKind::PowerAnalysis(custom_spec)).unwrap();
    edit_spec.id = power_rs_id.clone();
    *doc2.result_sheet_mut(&power_rs_id).unwrap() = ResultSheet::new(edit_spec).unwrap();

    let mut driver2 = open_edit_driver(doc2, &power_rs_id, 820.0);
    let pw = &driver2.state().app.canvas_dialogs.power_wizard;
    assert!(
        pw.family == StatsPowerTestFamily::TwoSamplePairedT
            && (pw.alpha - 0.01).abs() < 1e-12
            && (pw.target_power - 0.90).abs() < 1e-12
            && (pw.standardized_effect - 0.65).abs() < 1e-12
    );
    assert_widgets(&driver2, &["modal.ok"])?;
    click_all(&mut driver2, &["power.solve.power", "modal.next", "modal.next"])?;
    assert_eq!(driver2.state().app.canvas_dialogs.power_wizard.stage, PowerWizardStage::SignificanceAndSolve);
    assert_widget_enabled(&driver2, "power.one_sided", true)?;
    click_all(&mut driver2, &["power.one_sided", "modal.ok"])?;
    assert!(driver2.state().last_intent_error.is_none() && driver2.state().app.canvas_dialogs.active_dialog == ActiveDialog::None);
    let AnalysisKind::PowerAnalysis(ref pa) = driver2.state().app.document().result_sheet(&power_rs_id).unwrap().analysis.kind else {
        panic!("expected PowerAnalysis");
    };
    assert!(pa.solve_target == PowerSolveTarget::Power && matches!(pa.tail_mode, PowerTailMode::OneSided(_)));
    Ok(())
}

#[test]
fn r10_28_wf19_contingency_attributable_risk_and_r6_ci_selectors() -> TestRes {
    let ci_wids = [
        "edit.contingency.ci.baptista_pike",
        "edit.contingency.ci.koopman",
        "edit.contingency.ci.newcombe_wilson",
        "edit.contingency.ci.wilson_brown",
    ];
    let make_cont = |rows: usize, cols: usize, seed: bool| -> EguiHumanPathDriver<'static, ShellTestState> {
        let mut doc = ProjectDocument::new("WF19 Contingency").unwrap();
        let ds_id = SheetId::try_new(format!("cont-{rows}x{cols}")).unwrap();
        let mut ds = DataSheet::new(ds_id.clone(), "Table", TableArchetype::Contingency(ContingencyTable::new(rows, cols).unwrap())).unwrap();
        if seed {
            for (r, (a, b)) in [(12.0, 4.0), (5.0, 14.0)].into_iter().enumerate() {
                ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(a)).unwrap();
                ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(b)).unwrap();
            }
        }
        doc.data_sheets.push(ds);
        let rs_id = add_result_sheet(&mut doc, "res-cont", "Contingency", ds_id, vec![0, 1], AnalysisKind::Contingency(ContingencySpec::default()));
        open_edit_driver(doc, &rs_id, 820.0)
    };

    let mut driver = make_cont(2, 2, true);
    assert_widget_enabled(&driver, "edit.contingency.attributable_risk", true)?;
    for wid in ci_wids {
        assert_widget_enabled(&driver, wid, false)?;
    }
    click_all(&mut driver, &["edit.cont.or_rr", "edit.contingency.attributable_risk", "edit.cont.sens_spec"])?;
    assert!(driver.state().app.canvas_dialogs.edit_params.contingency_attributable_risk_checked);
    for wid in ci_wids {
        assert_widget_enabled(&driver, wid, true)?;
        driver.click_widget_id(wid)?;
    }
    assert_eq!(driver.state().last_intent_error, None);
    let ep = &driver.state().app.canvas_dialogs.edit_params;
    assert_eq!(
        (
            ep.contingency_or_ci_method.as_str(),
            ep.contingency_rr_ci_method.as_str(),
            ep.contingency_arr_ci_method.as_str(),
            ep.contingency_diag_ci_method.as_str()
        ),
        ("Woolf", "Katz", "Wald", "Clopper-Pearson")
    );

    // 3x2 disables attributable_risk and CI selectors.
    let driver3 = make_cont(3, 2, false);
    for wid in ["edit.contingency.attributable_risk", "edit.contingency.ci.baptista_pike", "edit.contingency.ci.newcombe_wilson"] {
        assert_widget_enabled(&driver3, wid, false)?;
    }
    Ok(())
}

#[test]
fn r10_28_wf36_43_52_nonlinear_fit_constant_textbox() -> TestRes {
    let (mut doc, xy_id) = seed_xy_doc(
        "WF36 Fit Constants",
        "xy-dose",
        "Dose",
        "X",
        &[-9.0, -8.0, -7.0, -6.0, -5.0, -4.0],
        &[("Y", &[5.0, 10.0, 28.0, 62.0, 88.0, 95.0])],
    )?;
    let rs_id = add_result_sheet(
        &mut doc,
        "res-fit",
        "4PL Fit",
        xy_id,
        vec![0],
        AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")),
    );
    let mut driver = open_edit_driver(doc, &rs_id, 860.0);

    click_all(&mut driver, &["fit.tab.constraints", "fit.con.rule.Bottom.fixed", "fit.con.rule.Top.fixed"])?;
    set_textbox(&mut driver, "fit.con.const.Bottom", "5")?;
    set_textbox(&mut driver, "fit.con.const.Top", "95")?;
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none() && driver.state().app.canvas_dialogs.active_dialog == ActiveDialog::None);
    let AnalysisKind::NonlinearRegression(ref nl) = driver.state().app.document().result_sheet(&rs_id).unwrap().analysis.kind else {
        panic!("expected NonlinearRegression");
    };
    assert_eq!(nl.parameter_rules.iter().find(|r| r.name == "Bottom").unwrap().scope, ParameterScope::Constant(5.0));
    assert_eq!(nl.parameter_rules.iter().find(|r| r.name == "Top").unwrap().scope, ParameterScope::Constant(95.0));
    Ok(())
}

#[test]
fn r10_28_wf61_normalize_defaults_checkboxes_and_create_graph() -> TestRes {
    let mut doc = ProjectDocument::new("WF61 Normalize").unwrap();
    let ds_id = SheetId::try_new("grp-norm").unwrap();
    let mut ds_grp =
        DataSheet::new(ds_id.clone(), "Signals", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 4).unwrap())).unwrap();
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
    assert!(app.egui_views.transform_modals.norm_percent && app.egui_views.transform_modals.norm_per_dataset);
    let mut driver = make_modal_driver(app, 820.0);
    assert_widgets(&driver, &["edit.transform.norm_percent", "edit.transform.norm_per_dataset", "edit.transform.normalize.create_graph"])?;
    click_all(&mut driver, &["edit.transform.normalize.create_graph", "modal.ok"])?;
    assert_eq!(driver.state().last_intent_error, None);
    assert_eq!((driver.state().app.document().result_sheets.len(), driver.state().app.document().graph_sheets.len()), (1, 1));
    let norm_rs_id = driver.state().app.document().result_sheets[0].id.clone();
    assert_eq!(driver.state().app.document().graph_sheets[0].spec.primary_source_sheet_id, norm_rs_id);

    // Part B: Cmd+T edit mode on Normalize ResultSheet persists flags + creates graph.
    let mut doc2 = ProjectDocument::new("WF61 CmdT").unwrap();
    doc2.data_sheets = driver.state().app.document().data_sheets.clone();
    let rs2_id = add_result_sheet(
        &mut doc2,
        "res-norm-edit",
        "Normalize Edit",
        ds_id,
        vec![0, 1],
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: true,
        }),
    );
    let mut driver2 = open_edit_driver(doc2, &rs2_id, 820.0);
    click_all(
        &mut driver2,
        &[
            "edit.transform.norm_zero.2",
            "edit.transform.norm_hundred.1",
            "edit.transform.norm_percent",
            "edit.transform.norm_per_dataset",
            "edit.transform.normalize.create_graph",
            "modal.ok",
        ],
    )?;
    assert_eq!(driver2.state().last_intent_error, None);
    let AnalysisKind::Transform(TransformAnalysisSpec::Normalize { ref zero_anchor, ref hundred_anchor, output_percent, per_data_set }) =
        driver2.state().app.document().result_sheet(&rs2_id).unwrap().analysis.kind
    else {
        panic!("expected Normalize spec");
    };
    assert_eq!(
        (zero_anchor, hundred_anchor, output_percent, per_data_set),
        (&NormalizeAnchor::FixedValue(0.0), &NormalizeAnchor::ColumnSum, false, false)
    );
    assert_eq!(driver2.state().app.document().graph_sheets.len(), 1);
    assert_eq!(driver2.state().app.document().graph_sheets[0].spec.primary_source_sheet_id, rs2_id);
    Ok(())
}

#[test]
fn r10_28_wf06_chained_result_sheet_mv_survival_exposes_data_tab_and_role_pickers() -> TestRes {
    let mut doc = ProjectDocument::new("WF06 Chained ResultSheet").unwrap();
    let upstream_ds = build_mv_sheet(
        "mv-src",
        "MV Cohort",
        &[("Time", VariableRole::Continuous), ("Status", VariableRole::Continuous), ("Arm", VariableRole::Categorical)],
        4,
        |mv| {
            for (r, (t, s, g)) in [(5.0, 1.0, "A"), (10.0, 1.0, "A"), (7.0, 1.0, "B"), (14.0, 0.0, "B")].into_iter().enumerate() {
                mv.variables[0].cells[r] = Cell::number(t);
                mv.variables[1].cells[r] = Cell::number(s);
                mv.variables[2].cells[r] = Cell::text(g);
            }
        },
    )?;
    let ds_id = upstream_ds.id.clone();
    doc.data_sheets.push(upstream_ds.clone());
    let prune_id = add_result_sheet(
        &mut doc,
        "res-prune",
        "Prune Cohort",
        ds_id,
        vec![0, 1, 2],
        AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 1, average_window: false }),
    );
    doc.result_sheet_mut(&prune_id).unwrap().analyzable_data = Some(upstream_ds);
    let km_id = add_result_sheet(
        &mut doc,
        "res-km",
        "KM Survival",
        prune_id,
        vec![0, 1, 2],
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
    );

    let mut driver = open_edit_driver(doc, &km_id, 820.0);
    assert_widgets(&driver, &["edit.tab.data"])?;
    driver.click_widget_id("edit.tab.data")?;
    assert_widgets(&driver, &["edit.km.time.0", "edit.km.status.1", "edit.km.group.2"])?;
    click_all(&mut driver, &["edit.km.time.0", "edit.km.status.1", "edit.km.group.2", "modal.ok"])?;
    assert_eq!(driver.state().last_intent_error, None);
    let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, mv_status_column, mv_group_column, .. }) =
        driver.state().app.document().result_sheet(&km_id).unwrap().analysis.kind
    else {
        panic!("expected KaplanMeier");
    };
    assert_eq!((mv_time_column, mv_status_column, mv_group_column), (Some(0), Some(1), Some(2)));
    Ok(())
}

#[test]
fn r10_28_wf10_anova_compare_against_control_dunnett_cycle() -> TestRes {
    let (mut doc, ds_id) = seed_column_doc(
        "WF10 ANOVA Dunnett",
        "col-anova",
        "Groups",
        ColumnDesignHint::UnpairedReplicates,
        &[("Grp1", &[1.0, 2.0, 3.0, 4.0]), ("Grp2", &[6.0, 7.0, 8.0, 9.0]), ("Grp3", &[11.0, 12.0, 13.0, 14.0])],
    )?;
    let rs_id = add_result_sheet(&mut doc, "res-anova", "One-Way ANOVA", ds_id, vec![0, 1, 2], AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()));
    let mut driver = open_edit_driver(doc, &rs_id, 820.0);

    driver.click_widget_id("edit.tab.multiple_comparisons")?;
    assert_widget_enabled(&driver, "edit.control_column", false)?;
    driver.click_widget_id("edit.compare_against_control")?;
    commit_shell_intents(&mut driver)?;
    assert!(driver.state().app.canvas_dialogs.edit_params.compare_against_control);
    assert_widget_enabled(&driver, "edit.control_column", true)?;
    click_all(&mut driver, &["edit.control_column", "modal.ok"])?;
    assert_eq!(driver.state().last_intent_error, None);
    let AnalysisKind::OneWayAnova(ref anova) = driver.state().app.document().result_sheet(&rs_id).unwrap().analysis.kind else {
        panic!("expected OneWayAnova");
    };
    assert_eq!(anova.multiplicity.method, MultiplicityMethod::Dunnett { control_column: 1 });
    Ok(())
}
