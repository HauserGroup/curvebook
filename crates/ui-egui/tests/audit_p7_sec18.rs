#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Phase 7 Stage 7.2 (`SEC-18`) regression suite (`crates/ui-egui/tests/audit_p7_sec18.rs`):
//! Covers `FA-SEC18-05` through `FA-SEC18-20`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, GraphicalDataObject,
    InfoSheet, MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod,
    ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype, TwoGroupMethod,
    TwoGroupSpec, VariableRole, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_math::PValueFormat;
use curvebook_palette::{ColorRamp, ColormapId};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, PrintDialogState};
use curvebook_ui_egui::egui_shell::app_modals::PreferencesCategory;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::graph_modals::MvOutOfRangePolicy;
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::power_studio::PowerResultsSubTab;
use curvebook_ui_egui::egui_shell::transform_modals::{MvFilterPredicateRow, TransformModalKind};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend, UiIntent};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::i18n::{LOCALES, catalog_len, missing_translations, tr};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::power_wizard::{PowerEffectInputMode, PowerWizardStage};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::NavigatorNestingMode;
use egui::{Key, Modifiers};
use serde_json::json;

type TestRes = Result<(), String>;

struct Sec18DriverState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_intent_error: Option<String>,
}

impl Sec18DriverState {
    fn new(mut app: UiApp) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        app.set_human_path_only(true);
        app.nav.welcome.open = false;
        app.egui_views.launcher_open = false;
        Self { app, views: EguiShellViewState::default(), intents: PendingUiIntents::new(), last_intent_error: None }
    }
}

fn make_driver(doc: ProjectDocument) -> EguiHumanPathDriver<'static, Sec18DriverState> {
    let app = UiApp::from_session(Session::from_document(doc));
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        Sec18DriverState::new(app),
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            st.views.canvas_view.synchronous_raster = true;
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

fn commit_intents(driver: &mut EguiHumanPathDriver<'_, Sec18DriverState>) {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
        st.last_intent_error = Some(err.to_string());
    }
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

fn sync_app_to_driver(driver: &mut EguiHumanPathDriver<'_, Sec18DriverState>) {
    let st = driver.state_mut();
    st.app.sync_views();
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

fn open_create_analysis(driver: &mut EguiHumanPathDriver<'_, Sec18DriverState>, spec: AnalysisSpec) {
    let target_id = spec.id.clone();
    {
        let app = &mut driver.state_mut().app;
        app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: target_id };
        app.canvas_dialogs.edit_params.draft = Some(spec);
        app.canvas_dialogs.edit_params.sync_rule_flags_from_kind();
    }
    sync_app_to_driver(driver);
}

fn set_textbox(driver: &mut EguiHumanPathDriver<'_, Sec18DriverState>, wid: &str, text: &str) -> TestRes {
    driver.click_widget_id(wid)?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text(text)?;
    Ok(())
}

fn build_rich_project_doc() -> Result<(ProjectDocument, SheetId, SheetId, SheetId, SheetId, SheetId), String> {
    let mut doc = ProjectDocument::new("SEC18 Regression Project").map_err(|e| e.to_string())?;
    let info = InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    // Column table (equal replicates for RM/Welch tests)
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).map_err(|e| e.to_string())?;
    for (c, (title, vals)) in
        [("Ctrl", [10.0, 10.5, 9.8, 10.2]), ("Low", [12.5, 13.0, 12.2, 12.8]), ("High", [16.0, 16.5, 15.8, 16.3])].into_iter().enumerate()
    {
        col.data_sets[c].title = String::from(title);
        for (r, v) in vals.into_iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let mut ds_col = DataSheet::new("ds-col", "Column Data", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    ds_col.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(ds_col).map_err(|e| e.to_string())?;

    // XY table (for NonlinearFit & FormatGraph multi-series)
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 2, 5)
        .map_err(|e| e.to_string())?;
    xy.x_title = String::from("Dose");
    xy.data_sets[0].title = String::from("DrugA");
    xy.data_sets[1].title = String::from("DrugB");
    for r in 0..5 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 * (r + 1) as f64 / ((r + 1) as f64 + 1.5));
        xy.data_sets[1].cells[r][0] = Cell::number(14.0 * (r + 1) as f64 / ((r + 1) as f64 + 2.0));
    }
    let ds_xy = DataSheet::new("ds-xy", "XY Dose Curve", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    let mut gs_xy = GraphSheet::new_uninitialized_for_data("g-xy", &ds_xy).map_err(|e| e.to_string())?;
    gs_xy.initialized = true;
    gs_xy.title = String::from("XY Graph");
    gs_xy.spec.kind = GraphKind::Xy;
    if gs_xy.spec.series.len() >= 2 {
        gs_xy.spec.series[0].gdo = GraphicalDataObject::BarWithReplicates {
            summary: curvebook_doc::SummaryMeasure::Mean,
            fill_color: curvebook_palette::Rgba8::BLACK,
            border_color: curvebook_palette::Rgba8::BLACK,
            border_width_pt: 1.5,
            replicate_overlay: None,
        };
        gs_xy.spec.series[1].gdo = GraphicalDataObject::BarWithReplicates {
            summary: curvebook_doc::SummaryMeasure::Mean,
            fill_color: curvebook_palette::Rgba8::BLACK,
            border_color: curvebook_palette::Rgba8::BLACK,
            border_width_pt: 3.5,
            replicate_overlay: None,
        };
    }
    doc.add_data_sheet(ds_xy).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(gs_xy).map_err(|e| e.to_string())?;

    // MV table (ManageVars, Transform, FormatGraph MV)
    let mut mv = MultipleVariablesTable::new(4, 6).map_err(|e| e.to_string())?;
    for (idx, (t, role)) in [
        ("Score [pts]", VariableRole::Continuous),
        ("Group", VariableRole::Categorical),
        ("Weight", VariableRole::Continuous),
        ("Tag", VariableRole::Label),
    ]
    .into_iter()
    .enumerate()
    {
        mv.variables[idx].title = String::from(t);
        mv.variables[idx].role = role;
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

#[test]
fn fa_sec18_05_preferences_cancel_and_escape_restore_all_5_categories() -> TestRes {
    let (doc, _, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);
    let orig_save = driver.state().app.shortcuts.active_combo("project.save").ok_or("missing project.save binding")?.to_owned();

    // Open Preferences, modify General nesting, Appearance theme + scale, Language locale, Shortcut draft.
    driver.state_mut().intents.push_open_preferences(Some(PreferencesCategory::General));
    commit_intents(&mut driver);
    assert!(driver.state().views.app_modals.preferences.initial_snapshot.is_some());

    driver.click_widget_id("prefs.general.nav_nested")?;
    driver.click_widget_id("edit.tab.prefs.appearance")?;
    driver.click_widget_id("prefs.appearance.theme.dark")?;
    driver.click_widget_id("prefs.appearance.cvd.deuteranopia")?;
    driver.state_mut().intents.push(UiIntent::SetUiScale(1.5));
    commit_intents(&mut driver);
    driver.click_widget_id("edit.tab.prefs.language")?;
    driver.click_widget_id("prefs.locale.de")?;
    driver.click_widget_id("edit.tab.prefs.keyboard_shortcuts")?;
    driver.click_widget_id("prefs.shortcuts.search")?;
    driver.type_text("Save")?;
    driver.click_widget_id("prefs.shortcuts.row.project.save")?;
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("Ctrl+Alt+Q");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;

    assert_eq!(driver.state().app.nav.nesting_mode, NavigatorNestingMode::NestedResultsUnderData);
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Dark);
    assert_eq!(driver.state().app.preview_filter, curvebook_palette::CanvasPreviewFilter::Deuteranopia);
    assert!((driver.state().app.theme.ui_scale - 1.5).abs() < 0.01);
    assert_eq!(driver.state().app.locale, "de");

    // Cancel must roll back ALL 5 categories atomically.
    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(driver.state().app.nav.nesting_mode, NavigatorNestingMode::SeparateFolders);
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Light);
    assert_eq!(driver.state().app.preview_filter, curvebook_palette::CanvasPreviewFilter::Normal);
    assert!((driver.state().app.theme.ui_scale - 1.0).abs() < 0.01);
    assert_eq!(driver.state().app.locale, "en");
    assert_eq!(driver.state().app.shortcuts.active_combo("project.save"), Some(orig_save.as_str()));

    // Escape must also cancel and restore snapshot.
    driver.state_mut().intents.push_open_preferences(Some(PreferencesCategory::Appearance));
    commit_intents(&mut driver);
    driver.click_widget_id("prefs.appearance.theme.dark")?;
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Dark);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Light);

    // OK must preserve committed edits.
    driver.state_mut().intents.push_open_preferences(Some(PreferencesCategory::Appearance));
    commit_intents(&mut driver);
    driver.click_widget_id("prefs.appearance.theme.dark")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Dark);
    Ok(())
}

#[test]
fn fa_sec18_06_save_as_default_applies_clears_and_diagnostics_note() -> TestRes {
    let (mut doc, col_id, xy_id, _, _, _) = build_rich_project_doc()?;
    let fit_id = SheetId::new("res-fit-06");
    let mut fit_spec =
        AnalysisSpec::new("Fit", xy_id, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("michaelis_menten"))).map_err(|e| e.to_string())?;
    fit_spec.id = fit_id.clone();
    let fit_rs = ResultSheet::new(fit_spec).map_err(|e| e.to_string())?;
    doc.add_result_sheet(fit_rs).map_err(|e| e.to_string())?;

    let mut driver = make_driver(doc);

    // 1. OneWayAnova: Save as Default, verify divergence clears `method_saved_as_default`, and verify Diagnostics note.
    let mut anova_spec =
        AnalysisSpec::new("ANOVA", col_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    anova_spec.id = SheetId::new("unsaved-anova-06");
    open_create_analysis(&mut driver, anova_spec);

    driver.click_widget_id("edit.tab.multiple_comparisons")?;
    driver.click_widget_id("dialog.button.save_default")?;
    assert!(driver.state().views.anova_modal.method_saved_as_default);
    assert!(driver.state().views.anova_modal.saved_default_spec.is_some());

    // Switching option diverges from saved default -> clears `method_saved_as_default`.
    driver.click_widget_id("edit.tab.experimental_design")?;
    driver.click_widget_id("edit.assume_gaussian")?;
    assert!(!driver.state().views.anova_modal.method_saved_as_default);

    // Diagnostics tab includes client preview controls.
    driver.click_widget_id("edit.tab.diagnostics")?;
    assert!(driver.frame_meta().find_by_widget_id("edit.diag.qq_plot").is_some());
    driver.click_widget_id("modal.cancel")?;

    // 2. FitModal: Save as Default sets flag and divergence clears it.
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &fit_id));
    }
    sync_app_to_driver(&mut driver);
    driver.click_widget_id("dialog.button.save_default")?;
    assert!(driver.state().views.fit_modal.method_saved_as_default);
    driver.click_widget_id("fit.tab.weight")?;
    driver.click_widget_id("fit.wt.rel_sq")?;
    assert!(!driver.state().views.fit_modal.method_saved_as_default);
    driver.click_widget_id("modal.cancel")?;

    // 3. TwoGroup (stat_params): Save as Default + create-mode application + divergence clear.
    let mut tg_spec =
        AnalysisSpec::new("TTest", col_id.clone(), AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).map_err(|e| e.to_string())?;
    tg_spec.id = SheetId::new("unsaved-tg-1");
    open_create_analysis(&mut driver, tg_spec);
    driver.click_widget_id("edit.tg.welch_t")?;
    driver.click_widget_id("dialog.button.save_default")?;
    assert!(driver.state().views.stats_modals.method_saved_as_default);
    driver.click_widget_id("edit.tg.unpaired_t")?;
    assert!(!driver.state().views.stats_modals.method_saved_as_default);
    driver.click_widget_id("modal.cancel")?;

    // Reopen create-mode TwoGroup -> saved WelchT default pre-populates create draft!
    let mut tg_spec2 = AnalysisSpec::new("TTest", col_id, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).map_err(|e| e.to_string())?;
    tg_spec2.id = SheetId::new("unsaved-tg-2");
    open_create_analysis(&mut driver, tg_spec2);
    let draft_kind = driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind);
    assert!(
        matches!(draft_kind, Some(AnalysisKind::TwoGroupComparison(t)) if t.method == TwoGroupMethod::WelchT),
        "expected saved WelchT default applied on create open, got {draft_kind:?}"
    );
    Ok(())
}

#[test]
fn fa_sec18_07_fit_modal_constant_dsl_r2_validation_and_untruncated_scroll() -> TestRes {
    let (mut doc, _, xy_id, _, _, _) = build_rich_project_doc()?;
    let fit_id = SheetId::new("res-fit-07");
    let mut spec =
        AnalysisSpec::new("Fit", xy_id, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("michaelis_menten"))).map_err(|e| e.to_string())?;
    spec.id = fit_id.clone();
    let rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let mut driver = make_driver(doc);
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &fit_id));
    }
    sync_app_to_driver(&mut driver);

    // 1. Compare tab list is not truncated at 18 items (verify via outer modal body wheel scroll).
    driver.click_widget_id("fit.tab.compare")?;
    driver.click_widget_id("fit.cmp.extra_f")?;
    let mut seen_cmp = std::collections::BTreeSet::new();
    for entry in driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("fit.cmp.alt.")) {
        seen_cmp.insert(entry.widget_id.clone());
    }
    if let Some(scroll_pos) = driver.frame_meta().find_by_widget_id("fit.cmp.extra_f").map(|e| e.clipped_rect.center()) {
        for _ in 0..8 {
            driver.wheel_at(scroll_pos, egui::Vec2::new(0.0, -240.0))?;
            for entry in driver.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("fit.cmp.alt.")) {
                seen_cmp.insert(entry.widget_id.clone());
            }
        }
    }
    assert!(seen_cmp.len() > 18, "Compare list must not truncate at 18 (saw {} items)", seen_cmp.len());
    if let Some(up_pos) = driver.frame_meta().entries.iter().find(|e| e.widget_id.starts_with("fit.cmp.alt.")).map(|e| e.clipped_rect.center()) {
        driver.wheel_at(up_pos, egui::Vec2::new(0.0, 2400.0))?;
    }
    driver.click_widget_id("fit.cmp.none")?;

    // 2. Constant constraint textbox validation + clearing on equation switch.
    driver.click_widget_id("fit.tab.constraints")?;
    driver.click_widget_id("fit.con.rule.Vmax.fixed")?;
    set_textbox(&mut driver, "fit.con.const.Vmax", "bad_const")?;
    assert!(driver.frame_meta().find_by_widget_id("fit.alert.0").is_some(), "invalid constant must surface alert");
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| !e.enabled));

    set_textbox(&mut driver, "fit.con.const.Vmax", "42.5")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| e.enabled));

    driver.click_widget_id("fit.tab.model")?;
    set_textbox(&mut driver, "fit.eq.search", "straight")?;
    assert!(driver.frame_meta().find_by_widget_id("fit.eq.item.straight_line").is_some());
    driver.click_widget_id("fit.eq.item.straight_line")?;
    assert!(driver.state().views.fit_modal.constant_drafts.is_empty());

    // 3. Custom DSL invalid syntax blocks commit.
    driver.click_widget_id("fit.eq.custom_toggle")?;
    set_textbox(&mut driver, "fit.eq.custom_dsl", "Y = +++")?;
    assert!(driver.frame_meta().find_by_widget_id("fit.alert.0").is_some());
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| !e.enabled));

    // 4. R2 threshold validation on Output tab surfaces alert and rejects out-of-range commit, then accepts 0.85.
    driver.click_widget_id("fit.eq.mode.builtin")?;
    driver.click_widget_id("fit.tab.output")?;
    set_textbox(&mut driver, "fit.diagnostics.r2.input", "1.75")?;
    assert!(driver.frame_meta().find_by_widget_id("fit.alert.0").is_some(), "R2 > 1.0 must surface alert");
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.as_deref().is_some_and(|e| e.contains("R² threshold")));
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    driver.state_mut().last_intent_error = None;
    set_textbox(&mut driver, "fit.diagnostics.r2.input", "0.85")?;
    assert!(driver.frame_meta().find_by_widget_id("modal.ok").is_some_and(|e| e.enabled));
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn fa_sec18_08_manage_vars_ok_commits_unapplied_drafts_and_cancel_escape_revert() -> TestRes {
    let (doc, _, _, mv_id, _, _) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);

    // 1. Edit title/unit AND level rename WITHOUT pressing Save/Apply buttons -> click OK -> BOTH commit!
    driver.state_mut().intents.push_open_manage_variables(mv_id.clone(), Some(1));
    commit_intents(&mut driver);
    set_textbox(&mut driver, "mv_formula.vars.title_input", "TreatmentArm")?;
    set_textbox(&mut driver, "mv_formula.vars.unit_input", "mg")?;
    driver.click_widget_id("mv_formula.vars.level.0")?;
    set_textbox(&mut driver, "mv_formula.vars.level_rename.0", "ControlArm")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    {
        let ds = driver.state().app.document().data_sheet(&mv_id).ok_or("missing ds-mv")?;
        let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
            return Err(String::from("expected MV"));
        };
        assert_eq!(mv.variables[1].title, "TreatmentArm (mg)");
        assert_eq!(mv.variables[1].cells[0], Cell::text("ControlArm"));
    }

    // 2. Open ManageVars, execute inline reorder (creates undo step inside session), then press Escape -> reverts cleanly!
    driver.state_mut().intents.push_open_manage_variables(mv_id.clone(), Some(1));
    commit_intents(&mut driver);
    let depth_at_open = driver.state().app.session().undo_depth();
    driver.click_widget_id("mv_formula.vars.level_dn.0")?;
    assert!(driver.state().app.session().undo_depth() > depth_at_open);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.grid.variables_dialog.open);
    assert_eq!(driver.state().app.session().undo_depth(), depth_at_open, "Escape must revert session undo steps");

    // 3. Repeat with explicit Cancel button -> also reverts cleanly.
    driver.state_mut().intents.push_open_manage_variables(mv_id, Some(1));
    commit_intents(&mut driver);
    let depth_at_open_2 = driver.state().app.session().undo_depth();
    driver.click_widget_id("mv_formula.vars.level_dn.0")?;
    assert!(driver.state().app.session().undo_depth() > depth_at_open_2);
    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.grid.variables_dialog.open);
    assert_eq!(driver.state().app.session().undo_depth(), depth_at_open_2, "Cancel must revert session undo steps");
    Ok(())
}

#[test]
fn fa_sec18_09_power_studio_stage2_pilot_modes_and_results_subtab_reset() -> TestRes {
    let (doc, _, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);
    driver.state_mut().app.canvas_dialogs.open_power_wizard();
    sync_app_to_driver(&mut driver);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize);

    // Switch to TwoSamplePairedT and test PairedDiffDirect + MeansAndSd inputs on Stage 2.
    driver.click_widget_id("power.family.paired_t")?;
    driver.click_widget_id("power.stage.2")?;
    driver.click_widget_id("power.effect_mode.pilot")?;
    assert!(driver.frame_meta().find_by_widget_id("power.pilot.corr.inc").is_some());
    driver.click_widget_id("power.effect_mode.paired_diff")?;
    assert_eq!(driver.state().app.canvas_dialogs.power_wizard.effect_mode, PowerEffectInputMode::PairedDiffDirect);
    let d_before = driver.state().app.canvas_dialogs.power_wizard.standardized_effect;
    driver.click_widget_id("power.pilot.paired_sd.inc")?;
    assert_ne!(driver.state().app.canvas_dialogs.power_wizard.standardized_effect, d_before);

    // Switch to OneWayAnova on Stage 1 -> check group means + within-group SD on Stage 2.
    driver.click_widget_id("power.stage.1")?;
    driver.click_widget_id("power.family.one_way_anova")?;
    driver.click_widget_id("power.stage.2")?;
    driver.click_widget_id("power.effect_mode.pilot")?;
    assert!(driver.frame_meta().find_by_widget_id("power.pilot.group_0.inc").is_some());
    assert!(driver.frame_meta().find_by_widget_id("power.pilot.group_1.inc").is_some());

    // Switch to SimpleLinearRegression -> check Slope & R2 modes.
    driver.click_widget_id("power.stage.1")?;
    driver.click_widget_id("power.family.simple_linear")?;
    driver.click_widget_id("power.stage.2")?;
    driver.click_widget_id("power.effect_mode.pilot")?;
    assert!(driver.frame_meta().find_by_widget_id("power.pilot.beta1.inc").is_some());
    driver.click_widget_id("power.effect_mode.r_squared")?;
    assert!(driver.frame_meta().find_by_widget_id("power.pilot.r2.inc").is_some());

    // Stage 4 Sensitivity sub-tab resets back to NarrativeReport after closing & reopening.
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
    let mut driver = make_driver(doc);
    driver.state_mut().intents.push_ui("ui.graph.axes.open", json!({ "sheet_id": g_xy.as_str() }));
    commit_intents(&mut driver);

    // Switch to X Axis, type invalid minor ticks "99" -> text preserved, error banner shown, commit blocked cleanly.
    driver.click_widget_id("edit.tab.axes.x")?;
    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "99")?;
    let x_node = driver.frame_meta().find_by_widget_id("axes.seg.0.minor_ticks_per_interval").ok_or("missing X minor_ticks")?;
    assert_eq!(x_node.value, Some(json!("99")));
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.ticks_error").is_some());
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));

    // Fix X minor ticks to "3", switch to Left Y Axis -> must NOT inherit "99"/"3".
    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "3")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.ticks_error").is_none());
    driver.click_widget_id("edit.tab.axes.y")?;
    let y_initial = driver.frame_meta().find_by_widget_id("axes.seg.0.minor_ticks_per_interval").ok_or("missing Y minor_ticks")?;
    assert_ne!(y_initial.value, Some(json!("3")));

    // Put invalid minor ticks on Y, switch back to X -> X keeps "3", but OK stays blocked because Y has error in cache!
    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "bad")?;
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));
    driver.click_widget_id("edit.tab.axes.x")?;
    assert_eq!(driver.state().views.graph_modals.axis_minor_ticks_draft, 3);
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));

    // Switch back to Y -> "bad" + error preserved, fix to "1" -> OK commits and closes!
    driver.click_widget_id("edit.tab.axes.y")?;
    let y_restored = driver.frame_meta().find_by_widget_id("axes.seg.0.minor_ticks_per_interval").ok_or("missing Y restored")?;
    assert_eq!(y_restored.value, Some(json!("bad")));
    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "1")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.ticks_error").is_none());
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn fa_sec18_11_checkpoints_draft_reset_on_create_and_two_step_restore() -> TestRes {
    let (doc, col_id, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);

    driver.state_mut().intents.push(UiIntent::OpenCheckpointsDialog);
    commit_intents(&mut driver);
    set_textbox(&mut driver, "checkpoints.name", "Before Mutation")?;
    set_textbox(&mut driver, "checkpoints.note", "Safe point")?;
    driver.click_widget_id("checkpoints.create")?;
    assert_eq!(driver.state().app.document().checkpoints.len(), 1);
    assert_ne!(driver.state().app.canvas_dialogs.checkpoints.name_draft, "Before Mutation");
    assert_eq!(driver.state().app.canvas_dialogs.checkpoints.name_draft, "Checkpoint 2");
    assert!(driver.state().app.canvas_dialogs.checkpoints.note_draft.is_empty());

    // Mutate table cell cleanly after creating checkpoint.
    driver
        .state_mut()
        .app
        .execute_command("table.setCell", json!({ "sheet_id": col_id.as_str(), "row": 0, "col": 0, "value": 999.0 }))
        .map_err(|e| e.to_string())?;
    sync_app_to_driver(&mut driver);

    // Step 1 click on Restore only arms confirmation — cell stays 999.0!
    driver.click_widget_id("checkpoints.restore.0")?;
    assert!(driver.frame_meta().find_by_widget_id("checkpoints.restore.confirm.0").is_some());
    assert!(driver.frame_meta().find_by_widget_id("checkpoints.restore.cancel.0").is_some());
    {
        let ds = driver.state().app.document().data_sheet(&col_id).ok_or("missing col")?;
        assert_eq!(ds.cell(curvebook_doc::CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&curvebook_doc::CellValue::Number(999.0)));
    }

    // Cancel restore keeps 999.0.
    driver.click_widget_id("checkpoints.restore.cancel.0")?;
    assert!(driver.state().views.app_modals.checkpoint_pending_restore_id.is_none());

    // Two-step confirm restores 10.0!
    driver.click_widget_id("checkpoints.restore.0")?;
    driver.click_widget_id("checkpoints.restore.confirm.0")?;
    let ds = driver.state().app.document().data_sheet(&col_id).ok_or("missing col")?;
    assert_eq!(ds.cell(curvebook_doc::CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&curvebook_doc::CellValue::Number(10.0)));
    Ok(())
}

#[test]
fn fa_sec18_12_transform_modals_formula_validation_untruncated_and_hook_gate() -> TestRes {
    let (doc, _, _, mv_id, _, _) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);

    driver.state_mut().intents.push_open_transform_modal(mv_id.clone(), TransformModalKind::MvFilterAndTransform);
    commit_intents(&mut driver);

    // Invalid formula on Add Derived preserves user drafts & surfaces error.
    set_textbox(&mut driver, "edit.transform.derived_title", "BadCol")?;
    set_textbox(&mut driver, "edit.transform.formula", "BAD(((")?;
    driver.click_widget_id("edit.transform.add_derived")?;
    assert!(driver.state().views.transform_modals.validation_error.is_some());
    assert_eq!(driver.state().views.transform_modals.derived_title_draft, "BadCol");
    assert_eq!(driver.state().views.transform_modals.formula_draft, "BAD(((");

    // Prepopulate 18 derived columns + 22 filter rows -> all render untruncated inside ScrollArea!
    {
        let tm = &mut driver.state_mut().views.transform_modals;
        tm.validation_error = None;
        tm.derived_cols = (0..18).map(|i| (format!("D{i}"), String::from("=[Score]"))).collect();
        tm.filter_rows = (0..22)
            .map(|_| MvFilterPredicateRow { left_col: 0, op: String::from(">"), right_value_or_col: String::from("0"), compare_variable: false })
            .collect();
    }
    driver.run_settled();
    assert_eq!(driver.state().views.transform_modals.derived_cols.len(), 18);
    assert_eq!(driver.state().views.transform_modals.filter_rows.len(), 22);

    // Select Rows tab must NOT expose dead "Hook Constant" button.
    driver.click_widget_id("edit.tab.select_rows")?;
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.hook_open").is_none());

    // Switching to another Transform modal resets baseline/hook drafts cleanly.
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    driver.state_mut().intents.push_open_transform_modal(mv_id, TransformModalKind::RemoveBaseline);
    commit_intents(&mut driver);
    assert!(driver.state().views.transform_modals.derived_title_draft.is_empty());
    assert!(!driver.state().views.transform_modals.hook_picker_open);
    Ok(())
}

#[test]
fn fa_sec18_13_import_wizard_internal_result_sheet_tabular_preview_and_import() -> TestRes {
    let (mut doc, col_id, _, _, _, _) = build_rich_project_doc()?;
    // Seed a terminal Anova ResultSheet (`analyzable_data == None`) with populated ResultTab rows!
    let mut rspec = AnalysisSpec::new("ANOVA", col_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    rspec.id = SheetId::new("res-term-13");
    rspec.custom_title = Some(String::from("Terminal ANOVA Summary"));
    let mut rs = ResultSheet::new(rspec).map_err(|e| e.to_string())?;
    rs.id = SheetId::new("res-term-13");
    rs.analyzable_data = None;
    rs.tabs = vec![ResultTab {
        id: String::from("tab-sum"),
        title: String::from("Summary"),
        kind: ResultTabKind::AnovaTable,
        column_headers: vec![String::from("Source"), String::from("F Ratio")],
        row_labels: vec![String::from("Between"), String::from("Within")],
        rows: vec![
            vec![ResultCell::Text(String::from("Treatment")), ResultCell::Number { value: 14.25, prefix: None, units: None }],
            vec![ResultCell::Text(String::from("Residual")), ResultCell::Number { value: 1.0, prefix: None, units: None }],
        ],
        footnotes: Vec::new(),
    }];
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let mut driver = make_driver(doc);
    driver.state_mut().intents.push_open_import_wizard(ImportOriginKind::Clipboard, false, Some(col_id.clone()));
    commit_intents(&mut driver);

    driver.click_widget_id("import_paste.source.internal_result")?;
    assert_eq!(driver.state().app.grid.import_wizard.origin_kind, ImportOriginKind::InternalResultSheet);
    assert_eq!(driver.state().app.grid.import_wizard.source_sheet_id.as_ref().map(SheetId::as_str), Some("res-term-13"));
    assert!(driver.frame_meta().find_by_widget_id("import_paste.source_sheet.res-term-13").is_some());
    assert!(driver.frame_meta().find_by_widget_id("import_paste.source_sheet.ds-col").is_none());

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    let ds = driver.state().app.document().data_sheet(&col_id).ok_or("missing target sheet")?;
    assert_eq!(ds.cell(curvebook_doc::CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&curvebook_doc::CellValue::Text(String::from("Source"))));
    assert_eq!(ds.cell(curvebook_doc::CellCoord::new(1, 0, 0)).map(|c| &c.value), Some(&curvebook_doc::CellValue::Text(String::from("Treatment"))));
    assert_eq!(ds.cell(curvebook_doc::CellCoord::new(1, 1, 0)).map(|c| &c.value), Some(&curvebook_doc::CellValue::Number(14.25)));
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

    let mut driver = make_driver(doc);

    // 1. ExportFigure
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::ExportFigure { sheet_id: g_xy.clone() };
    sync_app_to_driver(&mut driver);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 2. Print
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::Print { sheet_id: Some(g_xy.clone()) };
    driver.state_mut().app.canvas_dialogs.print = PrintDialogState::default();
    sync_app_to_driver(&mut driver);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 3. ResultViewer
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::ResultViewer { result_sheet_id: SheetId::new("res-esc") };
    sync_app_to_driver(&mut driver);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 4. Find & Replace
    driver.state_mut().views.app_modals.find_replace_open = true;
    driver.run_settled();
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.egui_views.app_modals.find_replace_open);

    // 5. UnsavedChangesPrompt
    driver.state_mut().views.app_modals.unsaved_changes_open = true;
    driver.run_settled();
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.egui_views.app_modals.unsaved_changes_open);

    // 6. ImportWizard
    driver.state_mut().intents.push_open_import_wizard(ImportOriginKind::Clipboard, false, Some(col_id));
    commit_intents(&mut driver);
    assert!(driver.state().app.grid.import_wizard.open);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().app.grid.import_wizard.open);

    // 7. TransformModal unwinds hook picker first, then closes dialog!
    driver.state_mut().intents.push_open_transform_modal(mv_id, TransformModalKind::MvFilterAndTransform);
    commit_intents(&mut driver);
    driver.state_mut().views.transform_modals.hook_picker_open = true;
    driver.run_settled();
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().views.transform_modals.hook_picker_open);
    assert!(driver.state().views.transform_modals.open_kind.is_some());
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(driver.state().views.transform_modals.open_kind.is_none());

    // 8. FormatAxes unwinds additional_ticks sub-modal first, then closes dialog!
    driver.state_mut().intents.push_ui("ui.graph.axes.open", json!({ "sheet_id": g_xy.as_str() }));
    commit_intents(&mut driver);
    driver.click_widget_id("edit.tab.axes.x")?;
    driver.click_widget_id("axes.additional_ticks.open")?;
    assert!(driver.state().views.graph_modals.additional_ticks.open);
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert!(!driver.state().views.graph_modals.additional_ticks.open);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));
    driver.press_egui_key(Key::Escape, Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn fa_sec18_15_anova_assume_sphericity_blocked_unless_geisser_greenhouse() -> TestRes {
    let (doc, col_id, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);
    let mut spec = AnalysisSpec::new("ANOVA", col_id, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    spec.id = SheetId::new("res-rm-15");

    open_create_analysis(&mut driver, spec);
    driver.click_widget_id("edit.paired_or_matched")?;
    driver.click_widget_id("edit.tab.repeated_measures")?;

    // When `assume_sphericity` is already active (gg_checked == false), clicking `edit.rm.assume_sphericity` must NOT disable it!
    driver.click_widget_id("edit.rm.assume_sphericity")?;
    let m_after_same = match &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::OneWayAnova(a)) => a.method,
        _ => return Err(String::from("expected OneWayAnova")),
    };
    assert_eq!(m_after_same, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false });

    // Toggle GG on via checkbox -> now clicking `edit.rm.assume_sphericity` switches back cleanly!
    driver.click_widget_id("edit.sphericity.geisser_greenhouse")?;
    let m_gg = match &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::OneWayAnova(a)) => a.method,
        _ => return Err(String::from("expected OneWayAnova")),
    };
    assert_eq!(m_gg, OneWayMethod::RepeatedMeasures { geisser_greenhouse: true });

    driver.click_widget_id("edit.rm.assume_sphericity")?;
    let m_unadj = match &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::OneWayAnova(a)) => a.method,
        _ => return Err(String::from("expected OneWayAnova")),
    };
    assert_eq!(m_unadj, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false });
    Ok(())
}

#[test]
fn fa_sec18_16_mv_format_graph_out_of_range_init_and_unclamped_autoscale() -> TestRes {
    let (doc, _, _, _, _, g_mv) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);
    let doc_snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_format_graph(&doc_snap, &g_mv));
    sync_app_to_driver(&mut driver);

    // MV graph initialized with domain_clamp: None -> mv_out_of_range starts as Omit ("Auto-scale (no clamp)").
    assert_eq!(driver.state().views.graph_modals.mv_out_of_range, MvOutOfRangePolicy::Omit);
    driver.click_widget_id("edit.tab.format_graph.data_objects")?;

    // Switch to Clamp to Bounds -> clamp values commit; click Auto-scale (no clamp) -> clears clamp to None!
    set_textbox(&mut driver, "format_graph.mv.color_clamp_min", "1.0")?;
    set_textbox(&mut driver, "format_graph.mv.color_clamp_max", "5.0")?;
    driver.click_widget_id("format_graph.mv.oor_clamp")?;
    assert_eq!(driver.state().views.graph_modals.mv_out_of_range, MvOutOfRangePolicy::ClampToBounds);

    driver.click_widget_id("format_graph.mv.oor_omit")?;
    assert_eq!(driver.state().views.graph_modals.mv_out_of_range, MvOutOfRangePolicy::Omit);
    driver.click_widget_id("modal.ok")?;

    let gs = driver.state().app.document().graph_sheet(&g_mv).ok_or("missing g-mv")?;
    let ColorChannelMapping::Continuous { domain_clamp, .. } = &gs.spec.multivariate.as_ref().ok_or("missing mv")?.color else {
        return Err(String::from("expected continuous color mapping"));
    };
    assert_eq!(*domain_clamp, None);
    Ok(())
}

#[test]
fn fa_sec18_17_close_all_clears_stale_state_and_spinner_ids_are_series_keyed() -> TestRes {
    let (doc, _, _, _, g_xy, _) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);

    // 1. Populate transient AppModalsEguiState fields and call close_all().
    {
        let am = &mut driver.state_mut().views.app_modals;
        am.find_replace_open = true;
        am.find_replace.match_count = 7;
        am.find_replace.status_message = Some(String::from("stale"));
        am.checkpoint_pending_restore_id = Some(String::from("cp-0"));
        am.preferences.ui_scale_spinner.editing = true;
        am.close_all();
        assert!(!am.any_open());
        assert_eq!(am.find_replace.match_count, 0);
        assert!(am.find_replace.status_message.is_none());
        assert!(am.checkpoint_pending_restore_id.is_none());
        assert!(!am.preferences.ui_scale_spinner.editing);
        assert!(am.preferences.initial_snapshot.is_none());
    }

    // 2. Switching selected series on Bar graph uses distinct bar_border_width_pt per series without stale spinner carryover.
    let doc_snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_format_graph(&doc_snap, &g_xy));
    sync_app_to_driver(&mut driver);
    driver.click_widget_id("edit.tab.format_graph.appearance")?;
    let val0 = driver.frame_meta().find_by_widget_id("format_graph.bar_border_width_pt").ok_or("missing series 0 bar border")?.value.clone();
    driver.click_widget_id("format_graph.series_next")?;
    let val1 = driver.frame_meta().find_by_widget_id("format_graph.bar_border_width_pt").ok_or("missing series 1 bar border")?.value.clone();
    assert_ne!(val0, val1, "series 0 ({val0:?}) and series 1 ({val1:?}) must render distinct bar border spinner values");
    Ok(())
}

#[test]
fn fa_sec18_18_all_16_locales_modal_translations_and_partial_helpers_removed() -> TestRes {
    assert_eq!(LOCALES.len(), 16);
    assert_eq!(catalog_len(), 750);
    assert!(missing_translations().is_empty());

    let sec18_sample_keys = [
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
    ];
    for key in sec18_sample_keys {
        let en = tr("en", key);
        assert!(!en.is_empty(), "missing en for {key}");
        for &loc in &LOCALES[1..] {
            let localized = tr(loc, key);
            assert!(!localized.is_empty(), "empty translation for {loc}:{key}");
            assert_ne!(localized, en, "expected {loc}:{key} to differ from English `{en}`");
        }
    }

    // Render Preferences in non-English locales (`de`, `ja`, `ar`) cleanly.
    for loc in ["de", "ja", "ar"] {
        let (doc, _, _, _, _, _) = build_rich_project_doc()?;
        let mut driver = make_driver(doc);
        assert_eq!(driver.state_mut().app.set_locale(loc), loc);
        driver.state_mut().intents.push_open_preferences(Some(PreferencesCategory::General));
        commit_intents(&mut driver);
        let tab = driver.frame_meta().find_by_widget_id("edit.tab.prefs.keyboard_shortcuts");
        assert!(tab.is_some(), "missing keyboard shortcuts tab in `{loc}`");
        if let Some(node) = tab {
            assert_eq!(node.label, tr(loc, "prefs.cat.shortcuts"));
        }
    }
    Ok(())
}

#[test]
fn fa_sec18_19_two_group_welch_enabled_and_one_sample_controls_round_trip() -> TestRes {
    let (doc, col_id, _, _, _, _) = build_rich_project_doc()?;
    let mut driver = make_driver(doc);

    let mut spec = AnalysisSpec::new("TwoGroup", col_id, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).map_err(|e| e.to_string())?;
    spec.id = SheetId::new("unsaved-tg-19");
    open_create_analysis(&mut driver, spec);

    // Even when `assume_gaussian` is unchecked, Welch's t stays enabled & selectable.
    driver.click_widget_id("edit.assume_gaussian")?;
    let welch_node = driver.frame_meta().find_by_widget_id("edit.tg.welch_t").ok_or("missing edit.tg.welch_t")?;
    assert!(welch_node.enabled, "Welch's t must stay enabled");
    driver.click_widget_id("edit.tg.welch_t")?;

    // Switch to One-sample t and enter hypothetical mean 12.5, then step!
    driver.click_widget_id("edit.tg.one_sample_t")?;
    assert!(driver.frame_meta().find_by_widget_id("edit.two_group.hypothetical").is_some());
    set_textbox(&mut driver, "edit.two_group.hypothetical", "12.5")?;
    let hyp_after_type = match &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::TwoGroupComparison(t)) => match t.method {
            TwoGroupMethod::OneSampleT { hypothetical } => hypothetical,
            _ => return Err(String::from("expected OneSampleT")),
        },
        _ => return Err(String::from("expected TwoGroupComparison")),
    };
    assert!((hyp_after_type - 12.5).abs() < 1e-6);

    driver.click_widget_id("edit.two_group.hypothetical.step.inc")?;
    let hyp_after_step = match &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::TwoGroupComparison(t)) => match t.method {
            TwoGroupMethod::OneSampleT { hypothetical } => hypothetical,
            _ => return Err(String::from("expected OneSampleT")),
        },
        _ => return Err(String::from("expected TwoGroupComparison")),
    };
    assert!((hyp_after_step - 13.5).abs() < 1e-6);

    // Also verify One-sample Wilcoxon radio exists & switches method cleanly.
    driver.click_widget_id("edit.tg.one_sample_wilcoxon")?;
    let kind_wil = driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind);
    assert!(matches!(kind_wil, Some(AnalysisKind::TwoGroupComparison(t)) if matches!(t.method, TwoGroupMethod::OneSampleWilcoxon { .. })));
    Ok(())
}

#[test]
fn fa_sec18_20_fit_output_pvalue_controls_and_dead_state_removed() -> TestRes {
    let (mut doc, _, xy_id, _, _, _) = build_rich_project_doc()?;
    let fit_id = SheetId::new("res-fit-20");
    let mut spec =
        AnalysisSpec::new("Fit", xy_id, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).map_err(|e| e.to_string())?;
    spec.id = fit_id.clone();
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut driver = make_driver(doc);
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &fit_id));
    }
    sync_app_to_driver(&mut driver);

    driver.click_widget_id("fit.tab.output")?;
    assert!(driver.frame_meta().find_by_widget_id("fit.out.pval.apa").is_some());
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
