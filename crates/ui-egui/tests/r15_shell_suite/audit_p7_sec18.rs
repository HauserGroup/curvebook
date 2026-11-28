//! Phase 7 Stage 7.2 (`SEC-18`) regression suite (`FA-SEC18-05`..`FA-SEC18-20`).
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet,
    GraphicalDataObject, InfoSheet, MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec, OneWayAnovaSpec,
    OneWayMethod, ProjectDocument, ResultCell, ResultSheet, ResultTabKind, SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype,
    TwoGroupMethod, TwoGroupSpec, VariableRole, XColumnMode, XyTable,
};
use curvebook_math::PValueFormat;
use curvebook_palette::{ColorRamp, ColormapId};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, PrintDialogState};
use curvebook_ui_egui::egui_shell::UiIntent;
use curvebook_ui_egui::egui_shell::app_modals::PreferencesCategory;
use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
use curvebook_ui_egui::egui_shell::graph_modals::MvOutOfRangePolicy;
use curvebook_ui_egui::egui_shell::power_studio::PowerResultsSubTab;
use curvebook_ui_egui::egui_shell::transform_modals::{MvFilterPredicateRow, TransformModalKind};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::i18n::{LOCALES, catalog_len, missing_translations, tr};
use curvebook_ui_egui::power_wizard::{PowerEffectInputMode, PowerWizardStage};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::NavigatorNestingMode;
use egui::{Key, Modifiers};
use serde_json::json;

use crate::common::{ShellTestState, TestRes, commit_shell_intents, fill_col_values, set_textbox, sync_app_to_driver, widget_center};
use crate::helpers::{
    exec, has_widget, make_human_driver, make_result_tab, open_edit_params_driver, open_format_graph_driver, widget_enabled, widget_label,
    widget_value,
};

fn open_create_analysis(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, spec: AnalysisSpec) {
    let target_id = spec.id.clone();
    let app = &mut driver.state_mut().app;
    app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: target_id };
    app.canvas_dialogs.edit_params.draft = Some(spec);
    app.canvas_dialogs.edit_params.sync_rule_flags_from_kind();
    sync_app_to_driver(driver);
}

fn push_and_commit(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, intent: UiIntent) {
    driver.state_mut().intents.push(intent);
    let _ = commit_shell_intents(driver);
}

fn build_rich_project_doc() -> Result<(ProjectDocument, SheetId, SheetId, SheetId, SheetId, SheetId), String> {
    let mut doc = ProjectDocument::new("SEC18 Regression Project").map_err(|e| e.to_string())?;
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).map_err(|e| e.to_string())?;
    for (c, (t, v)) in
        [("Ctrl", &[10.0, 10.5, 9.8, 10.2][..]), ("Low", &[12.5, 13.0, 12.2, 12.8]), ("High", &[16.0, 16.5, 15.8, 16.3])].into_iter().enumerate()
    {
        fill_col_values(&mut col.data_sets[c], t, v);
    }
    let mut ds_col = DataSheet::new("ds-col", "Column Data", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    ds_col.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(ds_col).map_err(|e| e.to_string())?;

    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 2, 5)
        .map_err(|e| e.to_string())?;
    xy.x_title = String::from("Dose");
    xy.data_sets[0].title = String::from("DrugA");
    xy.data_sets[1].title = String::from("DrugB");
    for r in 0..5 {
        let x = (r + 1) as f64;
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 * x / (x + 1.5));
        xy.data_sets[1].cells[r][0] = Cell::number(14.0 * x / (x + 2.0));
    }
    let ds_xy = DataSheet::new("ds-xy", "XY Dose Curve", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    let mut gs_xy = GraphSheet::new_uninitialized_for_data("g-xy", &ds_xy).map_err(|e| e.to_string())?;
    gs_xy.initialized = true;
    gs_xy.title = String::from("XY Graph");
    gs_xy.spec.kind = GraphKind::Xy;
    for (i, bw) in [(0, 1.5), (1, 3.5)] {
        if let Some(s) = gs_xy.spec.series.get_mut(i) {
            s.gdo = GraphicalDataObject::BarWithReplicates {
                summary: curvebook_doc::SummaryMeasure::Mean,
                fill_color: curvebook_palette::Rgba8::BLACK,
                border_color: curvebook_palette::Rgba8::BLACK,
                border_width_pt: bw,
                replicate_overlay: None,
            };
        }
    }
    doc.add_data_sheet(ds_xy).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(gs_xy).map_err(|e| e.to_string())?;

    let mut mv = MultipleVariablesTable::new(4, 6).map_err(|e| e.to_string())?;
    for (i, (t, role)) in [
        ("Score [pts]", VariableRole::Continuous),
        ("Group", VariableRole::Categorical),
        ("Weight", VariableRole::Continuous),
        ("Tag", VariableRole::Label),
    ]
    .into_iter()
    .enumerate()
    {
        mv.variables[i].title = t.into();
        mv.variables[i].role = role;
    }
    for r in 0..6 {
        mv.variables[0].cells[r] = Cell::number(10.0 + r as f64 * 2.5);
        mv.variables[1].cells[r] = Cell::text(if r < 3 { "Alpha" } else { "Beta" });
        mv.variables[2].cells[r] = Cell::number(1.0 + r as f64 * 0.3);
        mv.variables[3].cells[r] = Cell::text(format!("S{r}"));
    }
    let ds_mv = DataSheet::new("ds-mv", "MV Study", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
    let mut gs_mv = GraphSheet::new_uninitialized_for_data("g-mv", &ds_mv).map_err(|e| e.to_string())?;
    gs_mv.initialized = true;
    gs_mv.title = String::from("MV Bubble");
    gs_mv.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 2,
        color: ColorChannelMapping::Continuous { column: 2, ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false }, domain_clamp: None },
        size: SizeChannelMapping::Constant { radius_pt: 6.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    doc.add_data_sheet(ds_mv).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(gs_mv).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::new("ds-col"));
    Ok((doc, SheetId::new("ds-col"), SheetId::new("ds-xy"), SheetId::new("ds-mv"), SheetId::new("g-xy"), SheetId::new("g-mv")))
}

fn add_fit_result(doc: &mut ProjectDocument, id: &str, xy_id: SheetId, eq: &str) -> Result<SheetId, String> {
    let sid = SheetId::new(id);
    let mut sp = AnalysisSpec::new("Fit", xy_id, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new(eq))).map_err(|e| e.to_string())?;
    sp.id = sid.clone();
    doc.add_result_sheet(ResultSheet::new(sp).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(sid)
}

#[test]
fn fa_sec18_05_preferences_cancel_and_escape_restore_all_5_categories() -> TestRes {
    let (doc, _, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);
    let orig_save = driver.state().app.shortcuts.active_combo("project.save").ok_or("missing project.save")?.to_owned();

    push_and_commit(&mut driver, UiIntent::OpenPreferences { category: Some(PreferencesCategory::General) });
    assert!(driver.state().views.app_modals.preferences.initial_snapshot.is_some());
    for wid in ["prefs.general.nav_nested", "edit.tab.prefs.appearance", "prefs.appearance.theme.dark", "prefs.appearance.cvd.deuteranopia"] {
        driver.click_widget_id(wid)?;
    }
    push_and_commit(&mut driver, UiIntent::SetUiScale(1.5));
    for wid in ["edit.tab.prefs.language", "prefs.locale.de", "edit.tab.prefs.keyboard_shortcuts", "prefs.shortcuts.search"] {
        driver.click_widget_id(wid)?;
    }
    driver.type_text("Save")?;
    driver.click_widget_id("prefs.shortcuts.row.project.save")?;
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("Ctrl+Alt+Q");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;

    assert_eq!(
        (driver.state().app.nav.nesting_mode, driver.state().app.theme.mode, driver.state().app.preview_filter, driver.state().app.locale.as_str()),
        (NavigatorNestingMode::NestedResultsUnderData, UiThemeMode::Dark, curvebook_palette::CanvasPreviewFilter::Deuteranopia, "de")
    );
    assert!((driver.state().app.theme.ui_scale - 1.5).abs() < 0.01);

    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(
        (driver.state().app.nav.nesting_mode, driver.state().app.theme.mode, driver.state().app.preview_filter, driver.state().app.locale.as_str()),
        (NavigatorNestingMode::SeparateFolders, UiThemeMode::Light, curvebook_palette::CanvasPreviewFilter::Normal, "en")
    );
    assert!((driver.state().app.theme.ui_scale - 1.0).abs() < 0.01);
    assert_eq!(driver.state().app.shortcuts.active_combo("project.save"), Some(orig_save.as_str()));

    push_and_commit(&mut driver, UiIntent::OpenPreferences { category: Some(PreferencesCategory::Appearance) });
    driver.click_widget_id("prefs.appearance.theme.dark")?;
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.egui_views.app_modals.preferences_open && driver.state().app.theme.mode == UiThemeMode::Light);

    push_and_commit(&mut driver, UiIntent::OpenPreferences { category: Some(PreferencesCategory::Appearance) });
    driver.click_widget_id("prefs.appearance.theme.dark")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Dark);
    Ok(())
}

#[test]
fn fa_sec18_06_save_as_default_applies_clears_and_diagnostics_note() -> TestRes {
    let (mut doc, col_id, xy_id, _, _, _) = build_rich_project_doc()?;
    let fit_id = add_fit_result(&mut doc, "res-fit-06", xy_id, "michaelis_menten")?;
    let mut driver = make_human_driver(doc);

    let mut anova = AnalysisSpec::new("ANOVA", col_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    anova.id = SheetId::new("unsaved-anova-06");
    open_create_analysis(&mut driver, anova);
    driver.click_widget_id("edit.tab.multiple_comparisons")?;
    driver.click_widget_id("dialog.button.save_default")?;
    assert!(driver.state().views.anova_modal.method_saved_as_default && driver.state().views.anova_modal.saved_default_spec.is_some());
    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.assume_gaussian")?;
    assert!(!driver.state().views.anova_modal.method_saved_as_default);
    driver.click_widget_id("edit.tab.diagnostics")?;
    assert!(has_widget(&driver, "edit.diag.qq_plot"));
    driver.click_widget_id("modal.cancel")?;

    open_edit_params_driver(&mut driver, &fit_id)?;
    driver.click_widget_id("dialog.button.save_default")?;
    assert!(driver.state().views.fit_modal.method_saved_as_default);
    driver.click_widget_id("fit.tab.weight")?;
    driver.click_widget_id("fit.wt.rel_sq")?;
    assert!(!driver.state().views.fit_modal.method_saved_as_default);
    driver.click_widget_id("modal.cancel")?;

    for (idx, save_and_diverge) in [(1, true), (2, false)] {
        let mut tg =
            AnalysisSpec::new("TTest", col_id.clone(), AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).map_err(|e| e.to_string())?;
        tg.id = SheetId::new(format!("unsaved-tg-{idx}"));
        open_create_analysis(&mut driver, tg);
        if save_and_diverge {
            driver.click_widget_id("edit.tg.welch_t")?;
            driver.click_widget_id("dialog.button.save_default")?;
            assert!(driver.state().views.stats_modals.method_saved_as_default);
            driver.click_widget_id("edit.tg.unpaired_t")?;
            assert!(!driver.state().views.stats_modals.method_saved_as_default);
            driver.click_widget_id("modal.cancel")?;
        } else {
            assert!(
                matches!(driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::WelchT)
            );
        }
    }
    Ok(())
}

#[test]
fn fa_sec18_07_fit_modal_constant_dsl_r2_validation_and_untruncated_scroll() -> TestRes {
    let (mut doc, _, xy_id, _, _, _) = build_rich_project_doc()?;
    let fit_id = add_fit_result(&mut doc, "res-fit-07", xy_id, "michaelis_menten")?;
    let mut driver = make_human_driver(doc);
    open_edit_params_driver(&mut driver, &fit_id)?;

    driver.click_widget_id("fit.tab.compare")?;
    driver.click_widget_id("fit.cmp.extra_f")?;
    let mut seen_cmp: std::collections::BTreeSet<String> =
        driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("fit.cmp.alt.")).map(|e| e.widget_id.clone()).collect();
    if let Ok(pos) = widget_center(&driver, "fit.cmp.extra_f") {
        for _ in 0..8 {
            driver.wheel_at(pos, egui::Vec2::new(0.0, -240.0))?;
            seen_cmp.extend(driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("fit.cmp.alt.")).map(|e| e.widget_id.clone()));
        }
    }
    assert!(seen_cmp.len() > 18, "Compare list truncated at {}", seen_cmp.len());
    if let Some(up_pos) = driver.frame_meta().entries.iter().find(|e| e.widget_id.starts_with("fit.cmp.alt.")).map(|e| e.clipped_rect.center()) {
        driver.wheel_at(up_pos, egui::Vec2::new(0.0, 2400.0))?;
    }
    driver.click_widget_id("fit.cmp.none")?;

    driver.click_widget_id("fit.tab.constraints")?;
    driver.click_widget_id("fit.con.rule.Vmax.fixed")?;
    set_textbox(&mut driver, "fit.con.const.Vmax", "bad_const")?;
    assert!(has_widget(&driver, "fit.alert.0") && !widget_enabled(&driver, "modal.ok")?);
    set_textbox(&mut driver, "fit.con.const.Vmax", "42.5")?;
    assert!(widget_enabled(&driver, "modal.ok")?);

    driver.click_widget_id("fit.tab.model")?;
    set_textbox(&mut driver, "fit.eq.search", "straight")?;
    assert!(has_widget(&driver, "fit.eq.item.straight_line"));
    driver.click_widget_id("fit.eq.item.straight_line")?;
    assert!(driver.state().views.fit_modal.constant_drafts.is_empty());

    driver.click_widget_id("fit.eq.custom_toggle")?;
    set_textbox(&mut driver, "fit.eq.custom_dsl", "Y = +++")?;
    assert!(has_widget(&driver, "fit.alert.0") && !widget_enabled(&driver, "modal.ok")?);

    driver.click_widget_id("fit.eq.mode.builtin")?;
    driver.click_widget_id("fit.tab.output")?;
    set_textbox(&mut driver, "fit.diagnostics.r2.input", "1.75")?;
    assert!(has_widget(&driver, "fit.alert.0"));
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.as_deref().is_some_and(|e| e.contains("R² threshold")));
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    driver.state_mut().last_intent_error = None;
    set_textbox(&mut driver, "fit.diagnostics.r2.input", "0.85")?;
    assert!(widget_enabled(&driver, "modal.ok")?);
    driver.click_widget_id("modal.ok")?;
    assert_eq!((&driver.state().last_intent_error, &driver.state().app.canvas_dialogs.active_dialog), (&None, &ActiveDialog::None));
    Ok(())
}

#[test]
fn fa_sec18_08_manage_vars_ok_commits_unapplied_drafts_and_cancel_escape_revert() -> TestRes {
    let (doc, _, _, mv_id, _, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);

    push_and_commit(&mut driver, UiIntent::OpenManageVariables { sheet_id: mv_id.clone(), column: Some(1) });
    set_textbox(&mut driver, "mv_formula.vars.title_input", "TreatmentArm")?;
    set_textbox(&mut driver, "mv_formula.vars.unit_input", "mg")?;
    driver.click_widget_id("mv_formula.vars.level.0")?;
    set_textbox(&mut driver, "mv_formula.vars.level_rename.0", "ControlArm")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&mv_id).ok_or("ds-mv")?.archetype else {
        return Err("expected MV".into());
    };
    assert_eq!((mv.variables[1].title.as_str(), &mv.variables[1].cells[0]), ("TreatmentArm (mg)", &Cell::text("ControlArm")));

    for use_escape in [true, false] {
        push_and_commit(&mut driver, UiIntent::OpenManageVariables { sheet_id: mv_id.clone(), column: Some(1) });
        let depth = driver.state().app.session().undo_depth();
        driver.click_widget_id("mv_formula.vars.level_dn.0")?;
        assert!(driver.state().app.session().undo_depth() > depth);
        if use_escape {
            driver.press_egui_key(Key::Escape, Modifiers::NONE);
        } else {
            driver.click_widget_id("modal.cancel")?;
        }
        assert!(!driver.state().app.grid.variables_dialog.open && driver.state().app.session().undo_depth() == depth);
    }
    Ok(())
}

#[test]
fn fa_sec18_09_power_studio_stage2_pilot_modes_and_results_subtab_reset() -> TestRes {
    let (doc, _, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);
    driver.state_mut().app.canvas_dialogs.open_power_wizard();
    sync_app_to_driver(&mut driver);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize);

    driver.click_widget_id("power.family.paired_t")?;
    driver.click_widget_id("power.stage.2")?;
    driver.click_widget_id("power.effect_mode.pilot")?;
    assert!(has_widget(&driver, "power.pilot.corr.inc"));
    driver.click_widget_id("power.effect_mode.paired_diff")?;
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.effect_mode, PowerEffectInputMode::PairedDiffDirect);
    let d_before = driver.state().app.canvas_dialogs.power_wizard.standardized_effect;
    driver.click_widget_id("power.pilot.paired_sd.inc")?;
    assert_ne!(driver.state().app.canvas_dialogs.power_wizard.standardized_effect, d_before);

    for (fam_wid, mode_wid, expect_wids) in [
        ("power.family.one_way_anova", "power.effect_mode.pilot", &["power.pilot.group_0.inc", "power.pilot.group_1.inc"][..]),
        ("power.family.simple_linear", "power.effect_mode.pilot", &["power.pilot.beta1.inc"]),
    ] {
        for wid in ["power.stage.1", fam_wid, "power.stage.2", mode_wid] {
            driver.click_widget_id(wid)?;
        }
        for w in expect_wids {
            assert!(has_widget(&driver, w), "missing {w}");
        }
    }
    driver.click_widget_id("power.effect_mode.r_squared")?;
    assert!(has_widget(&driver, "power.pilot.r2.inc"));

    driver.state_mut().app.canvas_dialogs.power_wizard.stage = PowerWizardStage::Results;
    let _ = driver.state_mut().app.canvas_dialogs.power_wizard.recalculate();
    sync_app_to_driver(&mut driver);
    driver.click_widget_id("power.tab.sensitivity")?;
    assert_eq!(driver.state().views.power_studio.results_tab, PowerResultsSubTab::SensitivityMatrix);
    driver.click_widget_id("modal.cancel")?;

    driver.state_mut().app.canvas_dialogs.open_power_wizard();
    sync_app_to_driver(&mut driver);
    assert_eq!(driver.state().views.power_studio.results_tab, PowerResultsSubTab::NarrativeReport);
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.family.label(), "Simple linear regression");
    Ok(())
}

#[test]
fn fa_sec18_10_format_axes_minor_ticks_validation_and_per_axis_cache() -> TestRes {
    let (doc, _, _, _, g_xy, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);
    driver.state_mut().intents.push_ui("ui.graph.axes.open", json!({ "sheet_id": g_xy.as_str() }));
    let _ = commit_shell_intents(&mut driver);

    driver.click_widget_id("edit.tab.axes.x")?;
    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "99")?;
    assert_eq!(widget_value(&driver, "axes.seg.0.minor_ticks_per_interval")?, Some(json!("99")));
    assert!(has_widget(&driver, "axes.seg.0.ticks_error"));
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));

    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "3")?;
    assert!(!has_widget(&driver, "axes.seg.0.ticks_error"));
    driver.click_widget_id("edit.tab.axes.y")?;
    assert_ne!(widget_value(&driver, "axes.seg.0.minor_ticks_per_interval")?, Some(json!("3")));

    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "bad")?;
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));
    driver.click_widget_id("edit.tab.axes.x")?;
    assert_eq!(driver.state().views.graph_modals.axis_minor_ticks_draft, 3);
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));

    driver.click_widget_id("edit.tab.axes.y")?;
    assert_eq!(widget_value(&driver, "axes.seg.0.minor_ticks_per_interval")?, Some(json!("bad")));
    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "1")?;
    assert!(!has_widget(&driver, "axes.seg.0.ticks_error"));
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn fa_sec18_11_checkpoints_draft_reset_on_create_and_two_step_restore() -> TestRes {
    let (doc, col_id, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);

    push_and_commit(&mut driver, UiIntent::OpenCheckpointsDialog);
    set_textbox(&mut driver, "checkpoints.name", "Before Mutation")?;
    set_textbox(&mut driver, "checkpoints.note", "Safe point")?;
    driver.click_widget_id("checkpoints.create")?;
    assert_eq!(driver.state().app.document().checkpoints.len(), 1);
    assert_eq!(driver.state().app.canvas_dialogs.checkpoints.name_draft, "Checkpoint 2");
    assert!(driver.state().app.canvas_dialogs.checkpoints.note_draft.is_empty());

    exec(&mut driver.state_mut().app, "table.setCell", json!({ "sheet_id": col_id.as_str(), "row": 0, "col": 0, "value": 999.0 }))?;
    sync_app_to_driver(&mut driver);

    driver.click_widget_id("checkpoints.restore.0")?;
    assert!(has_widget(&driver, "checkpoints.restore.confirm.0") && has_widget(&driver, "checkpoints.restore.cancel.0"));
    assert_eq!(
        driver.state().app.document().data_sheet(&col_id).ok_or("col")?.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value),
        Some(&CellValue::Number(999.0))
    );

    driver.click_widget_id("checkpoints.restore.cancel.0")?;
    assert!(driver.state().views.app_modals.checkpoint_pending_restore_id.is_none());

    driver.click_widget_id("checkpoints.restore.0")?;
    driver.click_widget_id("checkpoints.restore.confirm.0")?;
    assert_eq!(
        driver.state().app.document().data_sheet(&col_id).ok_or("col")?.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value),
        Some(&CellValue::Number(10.0))
    );
    Ok(())
}

#[test]
fn fa_sec18_12_transform_modals_formula_validation_untruncated_and_hook_gate() -> TestRes {
    let (doc, _, _, mv_id, _, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);

    push_and_commit(&mut driver, UiIntent::OpenTransformModal { sheet_id: mv_id.clone(), kind: TransformModalKind::MvFilterAndTransform });
    set_textbox(&mut driver, "edit.transform.derived_title", "BadCol")?;
    set_textbox(&mut driver, "edit.transform.formula", "BAD(((")?;
    driver.click_widget_id("edit.transform.add_derived")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some());
    assert_eq!(
        (driver.state().views.transform_modals.derived_title_draft.as_str(), driver.state().views.transform_modals.formula_draft.as_str()),
        ("BadCol", "BAD(((")
    );

    let tm = &mut driver.state_mut().views.transform_modals;
    tm.validation_error = None;
    tm.derived_cols = (0..18).map(|i| (format!("D{i}"), String::from("=[Score]"))).collect();
    tm.filter_rows =
        (0..22).map(|_| MvFilterPredicateRow { left_col: 0, op: ">".into(), right_value_or_col: "0".into(), compare_variable: false }).collect();
    driver.run_settled();
    assert_eq!((driver.state().views.transform_modals.derived_cols.len(), driver.state().views.transform_modals.filter_rows.len()), (18, 22));

    driver.click_widget_id("edit.tab.select_rows")?;
    assert!(!has_widget(&driver, "edit.transform.hook_open"));

    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    push_and_commit(&mut driver, UiIntent::OpenTransformModal { sheet_id: mv_id, kind: TransformModalKind::RemoveBaseline });
    assert!(driver.state().views.transform_modals.derived_title_draft.is_empty() && !driver.state().views.transform_modals.hook_picker_open);
    Ok(())
}

#[test]
fn fa_sec18_13_import_wizard_internal_result_sheet_tabular_preview_and_import() -> TestRes {
    let (mut doc, col_id, _, _, _, _) = build_rich_project_doc()?;
    let mut rspec = AnalysisSpec::new("ANOVA", col_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    rspec.id = SheetId::new("res-term-13");
    rspec.custom_title = Some("Terminal ANOVA Summary".into());
    let mut rs = ResultSheet::new(rspec).map_err(|e| e.to_string())?;
    rs.id = SheetId::new("res-term-13");
    rs.analyzable_data = None;
    rs.tabs = vec![make_result_tab(
        "tab-sum",
        "Summary",
        ResultTabKind::AnovaTable,
        &["Source", "F Ratio"],
        &["Between", "Within"],
        vec![
            vec![ResultCell::Text("Treatment".into()), ResultCell::Number { value: 14.25, prefix: None, units: None }],
            vec![ResultCell::Text("Residual".into()), ResultCell::Number { value: 1.0, prefix: None, units: None }],
        ],
    )];
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let mut driver = make_human_driver(doc);
    push_and_commit(
        &mut driver,
        UiIntent::OpenImportWizard { origin: ImportOriginKind::Clipboard, is_live_link: false, target_sheet_id: Some(col_id.clone()) },
    );
    driver.click_widget_id("import_paste.source.internal_result")?;
    assert_eq!(driver.state().app.grid.import_wizard.origin_kind, ImportOriginKind::InternalResultSheet);
    assert_eq!(driver.state().app.grid.import_wizard.source_sheet_id.as_ref().map(SheetId::as_str), Some("res-term-13"));
    assert!(has_widget(&driver, "import_paste.source_sheet.res-term-13") && !has_widget(&driver, "import_paste.source_sheet.ds-col"));

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    let ds = driver.state().app.document().data_sheet(&col_id).ok_or("missing target sheet")?;
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&CellValue::Text("Source".into())));
    assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).map(|c| &c.value), Some(&CellValue::Text("Treatment".into())));
    assert_eq!(ds.cell(CellCoord::new(1, 1, 0)).map(|c| &c.value), Some(&CellValue::Number(14.25)));
    Ok(())
}

#[test]
fn fa_sec18_14_escape_closes_and_unwinds_all_modal_families() -> TestRes {
    let (mut doc, col_id, _, mv_id, g_xy, _) = build_rich_project_doc()?;
    let mut rspec = AnalysisSpec::new("ANOVA", col_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    rspec.id = SheetId::new("res-esc");
    let mut rs = ResultSheet::new(rspec).map_err(|e| e.to_string())?;
    rs.id = SheetId::new("res-esc");
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;
    let mut driver = make_human_driver(doc);

    for dlg in [
        ActiveDialog::ExportFigure { sheet_id: g_xy.clone() },
        ActiveDialog::Print { sheet_id: Some(g_xy.clone()) },
        ActiveDialog::ResultViewer { result_sheet_id: SheetId::new("res-esc") },
    ] {
        driver.state_mut().app.canvas_dialogs.active_dialog = dlg;
        driver.state_mut().app.canvas_dialogs.print = PrintDialogState::default();
        sync_app_to_driver(&mut driver);
        driver.press_egui_key(Key::Escape, Modifiers::NONE);
        assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    driver.state_mut().views.app_modals.find_replace_open = true;
    driver.run_settled();
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.egui_views.app_modals.find_replace_open);

    driver.state_mut().views.app_modals.unsaved_changes_open = true;
    driver.run_settled();
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.egui_views.app_modals.unsaved_changes_open);

    push_and_commit(
        &mut driver,
        UiIntent::OpenImportWizard { origin: ImportOriginKind::Clipboard, is_live_link: false, target_sheet_id: Some(col_id) },
    );
    assert!(driver.state().app.grid.import_wizard.open);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.grid.import_wizard.open);

    // Nested sub-modal Escape unwinds inner popup first, then outer dialog.
    push_and_commit(&mut driver, UiIntent::OpenTransformModal { sheet_id: mv_id, kind: TransformModalKind::MvFilterAndTransform });
    driver.state_mut().views.transform_modals.hook_picker_open = true;
    driver.run_settled();
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().views.transform_modals.hook_picker_open && driver.state().views.transform_modals.open_kind.is_some());
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(driver.state().views.transform_modals.open_kind.is_none());

    driver.state_mut().intents.push_ui("ui.graph.axes.open", json!({ "sheet_id": g_xy.as_str() }));
    let _ = commit_shell_intents(&mut driver);
    driver.click_widget_id("edit.tab.axes.x")?;
    driver.click_widget_id("axes.additional_ticks.open")?;
    assert!(driver.state().views.graph_modals.additional_ticks.open);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(
        !driver.state().views.graph_modals.additional_ticks.open
            && matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. })
    );
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn fa_sec18_15_anova_assume_sphericity_blocked_unless_geisser_greenhouse() -> TestRes {
    let (doc, col_id, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);
    let mut spec = AnalysisSpec::new("ANOVA", col_id, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    spec.id = SheetId::new("res-rm-15");
    open_create_analysis(&mut driver, spec);
    driver.click_widget_id("edit.paired_or_matched")?;
    driver.click_widget_id("edit.tab.repeated_measures")?;

    for (wid, expect_gg) in [("edit.rm.assume_sphericity", false), ("edit.sphericity.geisser_greenhouse", true), ("edit.rm.assume_sphericity", false)]
    {
        driver.click_widget_id(wid)?;
        let Some(AnalysisKind::OneWayAnova(a)) = driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) else {
            return Err("expected OneWayAnova".into());
        };
        assert_eq!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: expect_gg });
    }
    Ok(())
}

#[test]
fn fa_sec18_16_mv_format_graph_out_of_range_init_and_unclamped_autoscale() -> TestRes {
    let (doc, _, _, _, _, g_mv) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);
    open_format_graph_driver(&mut driver, &g_mv)?;
    assert_eq!(driver.state().views.graph_modals.mv_out_of_range, MvOutOfRangePolicy::Omit);

    driver.click_widget_id("edit.tab.format_graph.data_objects")?;
    set_textbox(&mut driver, "format_graph.mv.color_clamp_min", "1.0")?;
    set_textbox(&mut driver, "format_graph.mv.color_clamp_max", "5.0")?;
    driver.click_widget_id("format_graph.mv.oor_clamp")?;
    assert_eq!(driver.state().views.graph_modals.mv_out_of_range, MvOutOfRangePolicy::ClampToBounds);
    driver.click_widget_id("format_graph.mv.oor_omit")?;
    assert_eq!(driver.state().views.graph_modals.mv_out_of_range, MvOutOfRangePolicy::Omit);
    driver.click_widget_id("modal.ok")?;

    let gs = driver.state().app.document().graph_sheet(&g_mv).ok_or("missing g-mv")?;
    let ColorChannelMapping::Continuous { domain_clamp, .. } = &gs.spec.multivariate.as_ref().ok_or("mv")?.color else {
        return Err("expected continuous".into());
    };
    assert_eq!(*domain_clamp, None);
    Ok(())
}

#[test]
fn fa_sec18_17_close_all_clears_stale_state_and_spinner_ids_are_series_keyed() -> TestRes {
    let (doc, _, _, _, g_xy, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);
    {
        let am = &mut driver.state_mut().views.app_modals;
        am.find_replace_open = true;
        am.find_replace.match_count = 7;
        am.find_replace.status_message = Some("stale".into());
        am.checkpoint_pending_restore_id = Some("cp-0".into());
        am.preferences.ui_scale_spinner.editing = true;
        am.close_all();
        assert!(
            !am.any_open()
                && am.find_replace.match_count == 0
                && am.find_replace.status_message.is_none()
                && am.checkpoint_pending_restore_id.is_none()
                && !am.preferences.ui_scale_spinner.editing
                && am.preferences.initial_snapshot.is_none()
        );
    }

    open_format_graph_driver(&mut driver, &g_xy)?;
    driver.click_widget_id("edit.tab.format_graph.appearance")?;
    let val0 = widget_value(&driver, "format_graph.bar_border_width_pt")?;
    driver.click_widget_id("format_graph.series_next")?;
    assert_ne!(val0, widget_value(&driver, "format_graph.bar_border_width_pt")?);
    Ok(())
}

#[test]
fn fa_sec18_18_all_16_locales_modal_translations_and_partial_helpers_removed() -> TestRes {
    assert_eq!((LOCALES.len(), catalog_len(), missing_translations().is_empty()), (16, 1047, true));
    for key in [
        "prefs.title",
        "prefs.cat.shortcuts",
        "find.btn.replace_all",
        "checkpoints.confirm_restore",
        "fg.mv.auto_scale_no_clamp",
        "axes.guard.confirm_btn",
        "anova.run_analysis",
        "fit.commit.update",
        "pw.sec.effect_pilot",
        "mv.vars.apply_rename_merge",
        "import.btn.next",
        "transform.title.values",
        "two_group.hypothetical",
    ] {
        let en = tr("en", key);
        assert!(!en.is_empty(), "missing en:{key}");
        for &loc in &LOCALES[1..] {
            let v = tr(loc, key);
            assert!(!v.is_empty() && v != en, "bad translation {loc}:{key} = `{v}`");
        }
    }
    for loc in ["de", "ja", "ar"] {
        let (doc, _, _, _, _, _) = build_rich_project_doc()?;
        let mut driver = make_human_driver(doc);
        assert_eq!(driver.state_mut().app.set_locale(loc), loc);
        push_and_commit(&mut driver, UiIntent::OpenPreferences { category: Some(PreferencesCategory::General) });
        assert_eq!(widget_label(&driver, "edit.tab.prefs.keyboard_shortcuts")?, tr(loc, "prefs.cat.shortcuts"));
    }
    Ok(())
}

#[test]
fn fa_sec18_19_two_group_welch_enabled_and_one_sample_controls_round_trip() -> TestRes {
    let (doc, col_id, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_human_driver(doc);
    let mut spec = AnalysisSpec::new("TwoGroup", col_id, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).map_err(|e| e.to_string())?;
    spec.id = SheetId::new("unsaved-tg-19");
    open_create_analysis(&mut driver, spec);

    driver.click_widget_id("edit.assume_gaussian")?;
    assert!(widget_enabled(&driver, "edit.tg.welch_t")?);
    driver.click_widget_id("edit.tg.welch_t")?;
    driver.click_widget_id("edit.tg.one_sample_t")?;
    assert!(has_widget(&driver, "edit.two_group.hypothetical"));

    let one_sample_hyp = |d: &EguiHumanPathDriver<'_, ShellTestState>| -> Result<f64, String> {
        match d.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|x| &x.kind) {
            Some(AnalysisKind::TwoGroupComparison(t)) => match t.method {
                TwoGroupMethod::OneSampleT { hypothetical } => Ok(hypothetical),
                _ => Err("expected OneSampleT".into()),
            },
            _ => Err("expected TwoGroupComparison".into()),
        }
    };
    set_textbox(&mut driver, "edit.two_group.hypothetical", "12.5")?;
    assert!((one_sample_hyp(&driver)? - 12.5).abs() < 1e-6);
    driver.click_widget_id("edit.two_group.hypothetical.step.inc")?;
    assert!((one_sample_hyp(&driver)? - 13.5).abs() < 1e-6);

    driver.click_widget_id("edit.tg.one_sample_wilcoxon")?;
    assert!(
        matches!(driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::TwoGroupComparison(t)) if matches!(t.method, TwoGroupMethod::OneSampleWilcoxon { .. }))
    );
    Ok(())
}

#[test]
fn fa_sec18_20_fit_output_pvalue_controls_and_dead_state_removed() -> TestRes {
    let (mut doc, _, xy_id, _, _, _) = build_rich_project_doc()?;
    let fit_id = add_fit_result(&mut doc, "res-fit-20", xy_id, "straight_line")?;
    let mut driver = make_human_driver(doc);
    open_edit_params_driver(&mut driver, &fit_id)?;

    driver.click_widget_id("fit.tab.output")?;
    let scroll_pt = widget_center(&driver, "fit.out.conf_bands")?;
    for _ in 0..8 {
        if has_widget(&driver, "fit.out.pvalue.digits.inc") {
            break;
        }
        driver.wheel_at(scroll_pt, egui::vec2(0.0, -160.0))?;
    }
    assert!(has_widget(&driver, "fit.out.pval.apa"));
    driver.click_widget_id("fit.out.pval.apa")?;
    assert_eq!(driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| d.p_value_format), Some(PValueFormat::Apa));

    driver.click_widget_id("fit.out.pval.custom")?;
    driver.click_widget_id("fit.out.pvalue.digits.inc")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    let committed = driver.state().app.document().result_sheet(&fit_id).ok_or("missing committed fit")?;
    assert!(matches!(committed.analysis.p_value_format, PValueFormat::Custom { .. }));
    Ok(())
}
