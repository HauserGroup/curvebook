//! Multi-tab Statistical & GLM Parameter Dialog (`R10.3` `w5f2`) human-path tests.
//!
//! Tests opening via toolbar/menu/`Cmd+T`, switching across all 5 tabs, toggling
//! dependent `R1`–`R9` controls + pairing/REML vector schematics, hovering disabled
//! tooltips from `modal_predicates.rs`, confirming to update `ResultSheet`, and
//! clean delegation of nonlinear regression to `fit_params`.

#![forbid(unsafe_code)]

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencyTable, CoxTieMethod, DataSheet,
    GlmFamily, GroupedTable, MultipleVariablesTable, MultiplicityMethod, OneWayMethod, PValueFormat, ProjectDocument, RepeatedMeasuresDesign,
    ResidualCovariance, ResultStatus, RocCiMethod, SubcolumnMode, SurvivalAnalysisSpec, SurvivalTable, TableArchetype, TwoGroupMethod, VariableRole,
    XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::layout::{WidgetId, WidgetNode};
use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_widget_center, default_viewport, new_human_driver, press_cmd_shortcut,
    scroll_until_widget_visible,
};

fn fail(msg: impl Into<String>) -> String {
    msg.into()
}

fn check(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn make_column_sheet(id: &str, title: &str, hint: ColumnDesignHint, cols: &[Vec<f64>]) -> Result<DataSheet, String> {
    let rows = cols.iter().map(Vec::len).max().unwrap_or(4);
    let mut tbl = ColumnTable::new(hint, cols.len(), rows).map_err(err_str)?;
    for (c, _) in cols.iter().enumerate() {
        tbl.data_sets[c].title = format!("Group {}", (b'A' + u8::try_from(c).unwrap_or(0)) as char);
    }
    let mut ds = DataSheet::new(id, title, TableArchetype::Column(tbl)).map_err(err_str)?;
    for (c, vals) in cols.iter().enumerate() {
        for (r, &v) in vals.iter().enumerate() {
            ds.set_cell(CellCoord::new(r, c, 0), Cell::number(v)).map_err(err_str)?;
        }
    }
    Ok(ds)
}

fn make_grouped_sheet(id: &str, title: &str) -> Result<DataSheet, String> {
    let mut tbl = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 3).map_err(err_str)?;
    tbl.data_sets[0].title = String::from("Vehicle");
    tbl.data_sets[1].title = String::from("Treatment");
    let mut ds = DataSheet::new(id, title, TableArchetype::Grouped(tbl)).map_err(err_str)?;
    for r in 0..3 {
        for s in 0..3 {
            let base = 10.0 + (r as f64) * 3.0 + (s as f64) * 0.4;
            ds.set_cell(CellCoord::new(r, 0, s), Cell::number(base)).map_err(err_str)?;
            ds.set_cell(CellCoord::new(r, 1, s), Cell::number(base + 5.5 + (r as f64))).map_err(err_str)?;
        }
    }
    Ok(ds)
}

fn make_contingency_sheet(id: &str, rows: usize, cols: usize) -> Result<DataSheet, String> {
    let mut tbl = ContingencyTable::new(cols, rows).map_err(err_str)?;
    for c in 0..cols {
        tbl.data_sets[c].title = format!("Arm {}", c + 1);
    }
    let mut ds = DataSheet::new(id, "Contingency Counts", TableArchetype::Contingency(tbl)).map_err(err_str)?;
    for r in 0..rows {
        for c in 0..cols {
            let val = 12.0 + ((r * 3 + c * 5) as f64);
            ds.set_cell(CellCoord::new(r, c, 0), Cell::number(val)).map_err(err_str)?;
        }
    }
    Ok(ds)
}

fn make_survival_sheet(id: &str) -> Result<DataSheet, String> {
    let mut tbl = SurvivalTable::new(2, 6).map_err(err_str)?;
    tbl.cohorts[0].title = String::from("Placebo");
    tbl.cohorts[1].title = String::from("Drug");
    let mut ds = DataSheet::new(id, "KM Survival", TableArchetype::Survival(tbl)).map_err(err_str)?;
    let rows = [(5.0, 1.0, 0.0), (10.0, 1.0, 0.0), (14.0, 0.0, 1.0), (18.0, 1.0, 0.0), (24.0, 1.0, 1.0), (30.0, 0.0, 0.0)];
    for (r, (t, s0, s1)) in rows.into_iter().enumerate() {
        ds.set_x_or_time_cell(r, 0, Cell::number(t)).map_err(err_str)?;
        ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(s0)).map_err(err_str)?;
        ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(s1)).map_err(err_str)?;
    }
    Ok(ds)
}

fn make_mv_sheet(id: &str) -> Result<DataSheet, String> {
    let mut mv = MultipleVariablesTable::new(4, 8).map_err(err_str)?;
    mv.variables[0].title = String::from("Outcome");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("Status");
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[2].title = String::from("Dose");
    mv.variables[2].role = VariableRole::Continuous;
    mv.variables[3].title = String::from("Age");
    mv.variables[3].role = VariableRole::Continuous;
    let mut ds = DataSheet::new(id, "Cohort MV", TableArchetype::MultipleVariables(mv)).map_err(err_str)?;
    for r in 0..8 {
        let rf = r as f64;
        let bin = if r >= 4 { 1.0 } else { 0.0 };
        ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(2.0 + rf * 1.3 + bin * 2.5)).map_err(err_str)?;
        ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(bin)).map_err(err_str)?;
        ds.set_cell(CellCoord::new(r, 2, 0), Cell::number(1.0 + rf * 0.5)).map_err(err_str)?;
        ds.set_cell(CellCoord::new(r, 3, 0), Cell::number(30.0 + rf * 3.0 + if r % 2 == 0 { 2.0 } else { -2.0 })).map_err(err_str)?;
    }
    Ok(ds)
}

fn make_xy_sheet(id: &str) -> Result<DataSheet, String> {
    let xy = XyTable::new(SubcolumnMode::Single, 1, 6).map_err(err_str)?;
    let mut ds = DataSheet::new(id, "Dose Curve", TableArchetype::Xy(xy)).map_err(err_str)?;
    for (r, (x, y)) in [(-8.0, 4.0), (-7.0, 12.0), (-6.0, 44.0), (-5.0, 81.0), (-4.0, 94.0), (-3.0, 98.0)].into_iter().enumerate() {
        ds.set_x_or_time_cell(r, 0, Cell::number(x)).map_err(err_str)?;
        ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(y)).map_err(err_str)?;
    }
    Ok(ds)
}

fn driver_for_sheet(sheet: DataSheet) -> Result<curvebook_ui_egui::human_driver::HumanPathDriver, String> {
    let sid = sheet.id.clone();
    let mut doc = ProjectDocument::new("StatParams Test Project").map_err(err_str)?;
    doc.add_data_sheet(sheet).map_err(err_str)?;
    doc.active_sheet_id = Some(sid);
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_human_path_only(true);
    let mut driver = new_human_driver(app, default_viewport());
    let _ = driver.render_frame().map_err(err_str)?;
    Ok(driver)
}

fn open_analysis_and_edit_via_cmd_t(driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver, catalog_entry: &str) -> Result<(), String> {
    click_widget_center(driver, &WidgetId::from("toolbar.analyze"))?;
    check(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }), "AnalyzeSheet dialog did not open")?;
    let entry_wid = WidgetId::from(format!("analyze.entry.{catalog_entry}"));
    scroll_until_widget_visible(driver, &WidgetId::from("analyze.entries_list"), &entry_wid, 140.0, 10)?;
    click_widget_center(driver, &entry_wid)?;
    click_widget_center(driver, &WidgetId::from("modal.ok"))?;
    check(!driver.app().document().result_sheets.is_empty(), format!("ResultSheet not created for `{catalog_entry}`"))?;
    check(driver.find_widget("result.edit_params_btn").is_some_and(|n| n.state.enablement.enabled), "result.edit_params_btn missing/disabled")?;
    press_cmd_shortcut(driver, "t")?;
    check(
        matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
        format!("Cmd+T did not open EditAnalysisParameters for `{catalog_entry}`"),
    )?;
    Ok(())
}

fn click_in_content_scroll(driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver, target_id: &str) -> Result<(), String> {
    let target = WidgetId::from(target_id);
    if driver.find_widget(target_id).is_none_or(|n| !n.drawn_in_frame) {
        scroll_until_widget_visible(driver, &WidgetId::from("edit.content_panel"), &target, 120.0, 8)?;
    }
    click_widget_center(driver, &target)?;
    Ok(())
}

fn require_node(driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver, id: &str) -> Result<WidgetNode, String> {
    driver.find_widget(id).ok_or_else(|| format!("missing widget `{id}`"))
}

fn assert_all_tabs_clean(driver: &mut curvebook_ui_egui::human_driver::HumanPathDriver, supports_rm: bool) -> Result<(), String> {
    let tab2 = if supports_rm { "edit.tab.repeated_measures" } else { "edit.tab.factor_settings" };
    for tid in ["edit.tab.experimental_design", tab2, "edit.tab.multiple_comparisons", "edit.tab.options", "edit.tab.diagnostics"] {
        if driver.find_widget(tid).is_some_and(|n| n.state.enablement.enabled) {
            click_widget_center(driver, &WidgetId::from(tid))?;
            assert_zero_shortcut_compliance(driver)?;
        }
    }
    click_widget_center(driver, &WidgetId::from("edit.tab.experimental_design"))?;
    Ok(())
}

#[test]
fn w5f2_open_via_menu_toolbar_and_cmd_t_plus_tab_parity_one_way_anova() -> Result<(), String> {
    let col = make_column_sheet(
        "col-rm",
        "Repeated Measures Cohort",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[vec![10.1, 11.4, 9.8, 10.6, 11.1], vec![16.2, 17.5, 15.9, 16.8, 17.2], vec![12.0, 13.1, 11.4, 12.2, 12.8]],
    )?;
    let mut driver = driver_for_sheet(col)?;

    // Open AnalyzeSheet from toolbar and verify menu entry presence on resulting ResultSheet.
    click_widget_center(&mut driver, &WidgetId::from("toolbar.analyze"))?;
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    let anova_id = WidgetId::from("analyze.entry.stats.one_way_anova");
    scroll_until_widget_visible(&mut driver, &WidgetId::from("analyze.entries_list"), &anova_id, 140.0, 8)?;
    click_widget_center(&mut driver, &anova_id)?;
    click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    // Check Analyze menu bar dropdown contains `menu.analysis.updateParams`, then escape.
    click_widget_center(&mut driver, &WidgetId::from("menu.analyze"))?;
    check(driver.find_widget("menu.analysis.updateParams").is_some(), "menu.analysis.updateParams item missing in Analyze menu")?;
    driver.press_key("Escape", Default::default()).map_err(err_str)?;

    // Open Edit Analysis Parameters via Cmd+T shortcut.
    press_cmd_shortcut(&mut driver, "t")?;
    check(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }), "EditAnalysisParameters not open")?;
    assert_all_tabs_clean(&mut driver, true)?;

    // Starts unpaired (`OrdinaryAnova`); verify R1 disables Repeated Measures tab, then toggle pairing on & check schematic pixels update.
    let _ = require_node(&mut driver, "edit.design.schematic")?;
    check(!driver.app().canvas_dialogs.edit_params.paired_or_matched, "expected initial paired_or_matched false")?;
    check(
        !require_node(&mut driver, "edit.tab.repeated_measures")?.state.enablement.enabled,
        "R1: Repeated Measures tab should disable when unpaired",
    )?;
    let frame_unpaired = driver.render_frame().map_err(err_str)?;

    click_in_content_scroll(&mut driver, "edit.paired_or_matched")?;
    check(driver.app().canvas_dialogs.edit_params.paired_or_matched, "expected paired_or_matched true after toggle")?;
    check(require_node(&mut driver, "edit.tab.repeated_measures")?.state.enablement.enabled, "R1: Repeated Measures tab should enable when paired")?;
    let frame_paired = driver.render_frame().map_err(err_str)?;
    check(frame_unpaired != frame_paired, "Pairing schematic and modal pixels did not update when toggling paired_or_matched")?;

    // Tab 2: Repeated Measures -> toggle Geisser-Greenhouse off cleanly.
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.repeated_measures"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    check(require_node(&mut driver, "edit.sphericity.geisser_greenhouse")?.state.enablement.enabled, "GG toggle should be enabled in paired mode")?;
    click_in_content_scroll(&mut driver, "edit.sphericity.geisser_greenhouse")?;

    // Tab 3: Multiple Comparisons -> R2 disabled Dunn's + hover tooltip, R3 control column, Dunnett CLD status -> Tukey CLD status.
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.multiple_comparisons"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    let dunn_node = require_node(&mut driver, "edit.post_hoc.dunn")?;
    check(!dunn_node.state.enablement.enabled, "R2: Dunn should be disabled in Gaussian mode")?;
    let dunn_reason = dunn_node.state.enablement.disabled_reason.clone().ok_or_else(|| fail("R2: Dunn missing disabled_reason"))?;
    driver.hover_widget("edit.post_hoc.dunn").map_err(err_str)?;
    let tip = require_node(&mut driver, "overlay.tooltip")?;
    let tip_prefix = tip.a11y.label.trim_end_matches('…');
    check(!tip_prefix.is_empty() && dunn_reason.starts_with(tip_prefix), format!("Tooltip `{}` mismatch reason `{dunn_reason}`", tip.a11y.label))?;

    let ctrl_before = require_node(&mut driver, "edit.control_column")?.state.enablement.enabled;
    check(!ctrl_before, "R3: control column picker should initially be disabled when using Tukey")?;
    click_in_content_scroll(&mut driver, "edit.post_hoc.dunnett")?;
    check(require_node(&mut driver, "edit.control_column")?.state.enablement.enabled, "R3: control column picker should enable for Dunnett")?;
    let cld_dunnett = require_node(&mut driver, "edit.mc.cld_status")?.a11y.label.clone();
    check(cld_dunnett.contains("Incompatible"), format!("Expected CLD incompatible notice for Dunnett, got `{cld_dunnett}`"))?;
    click_in_content_scroll(&mut driver, "edit.control_column")?;
    check(driver.app().canvas_dialogs.edit_params.control_column == 1, "Control column did not advance to col 1")?;

    click_in_content_scroll(&mut driver, "edit.post_hoc.tukey")?;
    let cld_tukey = require_node(&mut driver, "edit.mc.cld_status")?.a11y.label.clone();
    check(cld_tukey.contains("Compatible"), format!("Expected CLD compatible notice for Tukey, got `{cld_tukey}`"))?;
    click_in_content_scroll(&mut driver, "edit.mult_report_ci")?;
    click_in_content_scroll(&mut driver, "edit.mult_threshold.inc")?;

    // Tab 4: Options -> 99% CI, R9 precision spinner gate, effect sizes toggle.
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.options"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    click_in_content_scroll(&mut driver, "edit.conf.99")?;
    click_in_content_scroll(&mut driver, "edit.pval.four_decimal")?;
    check(!require_node(&mut driver, "edit.pvalue.digits.inc")?.state.enablement.enabled, "R9: precision spinner must disable for FourDecimal")?;
    click_in_content_scroll(&mut driver, "edit.pval.custom")?;
    check(require_node(&mut driver, "edit.pvalue.digits.inc")?.state.enablement.enabled, "R9: precision spinner must enable for Custom format")?;
    click_in_content_scroll(&mut driver, "edit.pvalue.digits.inc")?;
    click_in_content_scroll(&mut driver, "edit.compute_effect_sizes")?;

    // Tab 5: Diagnostics / Residuals.
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.diagnostics"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    click_in_content_scroll(&mut driver, "edit.diag.qq_plot")?;
    click_in_content_scroll(&mut driver, "edit.diag.influence_leverage")?;
    check(
        driver.app().view_shards.stat_params.diag_influence_leverage && !driver.app().view_shards.stat_params.diag_qq_plot,
        "Diagnostics view state toggles did not update StatParamsViewState",
    )?;

    // Verify total interactive + compute flow ceiling and strict <= 50 ms steady-state tab-switch budget.
    assert_input_to_paint_budget(&driver, 250.0)?;
    for tid in ["edit.tab.experimental_design", "edit.tab.options", "edit.tab.diagnostics"] {
        click_widget_center(&mut driver, &WidgetId::from(tid))?;
        let step_ms = driver.last_input_to_paint_ms();
        check(step_ms <= 50.0, format!("Steady-state tab `{tid}` input-to-paint {step_ms:.2} ms > 50.00 ms"))?;
    }

    // Confirm and verify ResultSheet reflects confirmed parameters.
    click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    check(!matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }), "Modal failed to close on OK")?;
    let rs = driver.app().document().result_sheets.first().ok_or_else(|| fail("ResultSheet missing after confirm"))?;
    check(rs.status == ResultStatus::UpToDate, "ResultSheet not UpToDate after edit params confirm")?;
    check((rs.analysis.confidence_level.percent() - 99.0).abs() < 0.05, "Expected 99% confidence level persisted")?;
    check(matches!(rs.analysis.p_value_format, PValueFormat::Custom { decimals: 5, .. }), "Expected PValueFormat::Custom with 5 decimals")?;
    let AnalysisKind::OneWayAnova(ref a) = rs.analysis.kind else {
        return Err(fail("Expected OneWayAnova ResultSheet"));
    };
    check(matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }), "Expected RM geisser_greenhouse=false")?;
    check(a.multiplicity.method == MultiplicityMethod::TukeyHsd, "Expected TukeyHsd multiplicity")?;
    check(!a.multiplicity.report_confidence_intervals, "Expected report_confidence_intervals toggled false")?;
    check(a.compute_effect_sizes, "Expected compute_effect_sizes toggled true from default false")?;
    Ok(())
}

#[test]
fn w5f2_unpaired_r1_r2_disabled_reasons_and_kruskal_confirm() -> Result<(), String> {
    let col = make_column_sheet(
        "col-unp",
        "Unpaired Unequal Rows",
        ColumnDesignHint::UnpairedReplicates,
        &[vec![4.0, 5.1, 4.7, 5.3], vec![8.2, 8.9, 9.4], vec![12.0, 11.4, 12.6, 13.0]],
    )?;
    let mut driver = driver_for_sheet(col)?;
    open_analysis_and_edit_via_cmd_t(&mut driver, "stats.one_way_anova")?;
    assert_zero_shortcut_compliance(&mut driver)?;

    let paired_node = require_node(&mut driver, "edit.paired_or_matched")?;
    check(!paired_node.state.enablement.enabled, "R1: paired toggle must be disabled for uneven unpaired table")?;
    let paired_reason = paired_node.state.enablement.disabled_reason.clone().ok_or_else(|| fail("R1: paired toggle missing disabled_reason"))?;
    driver.hover_widget("edit.paired_or_matched").map_err(err_str)?;
    let tip = require_node(&mut driver, "overlay.tooltip")?;
    let tip_prefix = tip.a11y.label.trim_end_matches('…');
    check(
        !tip_prefix.is_empty() && paired_reason.starts_with(tip_prefix),
        format!("Hovering disabled paired toggle tooltip `{}` mismatch `{paired_reason}`", tip.a11y.label),
    )?;

    // Uncheck Gaussian -> parametric post-hoc disables and method switches to Kruskal-Wallis / Dunn.
    click_in_content_scroll(&mut driver, "edit.assume_gaussian")?;
    check(!driver.app().canvas_dialogs.edit_params.assume_gaussian, "assume_gaussian should be false")?;
    let tukey = require_node(&mut driver, "edit.post_hoc.tukey")?;
    check(!tukey.state.enablement.enabled && tukey.state.enablement.disabled_reason.is_some(), "R2: Tukey should disable with reason")?;
    click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let rs = driver.app().document().result_sheets.first().ok_or_else(|| fail("Missing ResultSheet"))?;
    let AnalysisKind::OneWayAnova(ref a) = rs.analysis.kind else {
        return Err(fail("Expected OneWayAnova"));
    };
    check(a.method == OneWayMethod::KruskalWallis, "Expected KruskalWallis after unchecking assume_gaussian")?;
    check(a.multiplicity.method == MultiplicityMethod::DunnRankSum, "Expected DunnRankSum multiplicity")?;
    Ok(())
}

#[test]
fn w5f2_factorial_anova_rm_reml_and_covariance_schematic() -> Result<(), String> {
    let grouped = make_grouped_sheet("grp-fac", "Two-Way Assay")?;
    let mut driver = driver_for_sheet(grouped)?;
    open_analysis_and_edit_via_cmd_t(&mut driver, "stats.factorial_anova")?;
    assert_all_tabs_clean(&mut driver, true)?;

    // Initially RepeatedMeasuresDesign::None -> REML covariance controls disabled (R1).
    let cov_un_init = require_node(&mut driver, "edit.factorial.cov.un")?;
    check(!cov_un_init.state.enablement.enabled && cov_un_init.state.enablement.disabled_reason.is_some(), "R1: REML cov disabled when No RM")?;

    // Switch to Row Factor RM -> CS enabled, render CS schematic frame, then Unstructured -> framebuffer changes.
    click_in_content_scroll(&mut driver, "edit.fac.rm.row")?;
    check(require_node(&mut driver, "edit.factorial.cov.un")?.state.enablement.enabled, "R1: REML cov must enable once RM selected")?;
    let frame_cs = driver.render_frame().map_err(err_str)?;
    click_in_content_scroll(&mut driver, "edit.factorial.cov.un")?;
    let frame_un = driver.render_frame().map_err(err_str)?;
    check(frame_cs != frame_un, "REML covariance schematic did not update between CompoundSymmetry and Unstructured")?;

    click_widget_center(&mut driver, &WidgetId::from("edit.tab.repeated_measures"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    click_in_content_scroll(&mut driver, "edit.factorial.fit_reml_on_missing")?;
    click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let rs = driver.app().document().result_sheets.first().ok_or_else(|| fail("Missing Factorial ResultSheet"))?;
    let AnalysisKind::FactorialAnova(ref f) = rs.analysis.kind else {
        return Err(fail("Expected FactorialAnova"));
    };
    check(f.repeated_measures == RepeatedMeasuresDesign::RowFactorMatched, "Expected RowFactorMatched")?;
    check(f.residual_covariance == ResidualCovariance::Unstructured, "Expected ResidualCovariance::Unstructured")?;
    check(!f.fit_reml_on_missing, "Expected fit_reml_on_missing toggled false from default true")?;
    Ok(())
}

#[test]
fn w5f2_two_group_r4_toggles_and_confirm() -> Result<(), String> {
    let col = make_column_sheet(
        "col-tg",
        "Two-Group Matched",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[vec![10.0, 11.5, 9.9, 10.7, 11.0], vec![15.2, 16.4, 14.8, 15.9, 16.1]],
    )?;
    let mut driver = driver_for_sheet(col)?;
    open_analysis_and_edit_via_cmd_t(&mut driver, "stats.two_group")?;
    assert_all_tabs_clean(&mut driver, true)?;

    // Options tab R4 checks while in Unpaired mode.
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.options"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    check(!require_node(&mut driver, "edit.two_group.graph_ranks")?.state.enablement.enabled, "R4: graph_ranks disabled for parametric t")?;
    check(!require_node(&mut driver, "edit.two_group.hodges_lehmann")?.state.enablement.enabled, "R4: hodges_lehmann disabled for parametric t")?;
    check(require_node(&mut driver, "edit.two_group.welch")?.state.enablement.enabled, "R4: welch toggle enabled on UnpairedT")?;

    // Paired T enables Repeated Measures tab (`R1`) and estimation plot (`R4`); verify parity cleanly, then switch to Mann-Whitney.
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.experimental_design"))?;
    click_in_content_scroll(&mut driver, "edit.tg.paired")?;
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.repeated_measures"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    check(require_node(&mut driver, "edit.two_group.graph_differences")?.state.enablement.enabled, "R4: graph_differences enabled on PairedT")?;

    click_widget_center(&mut driver, &WidgetId::from("edit.tab.experimental_design"))?;
    click_in_content_scroll(&mut driver, "edit.tg.mw")?;
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.options"))?;
    check(require_node(&mut driver, "edit.two_group.graph_ranks")?.state.enablement.enabled, "R4: graph_ranks enabled for MannWhitneyU")?;
    check(require_node(&mut driver, "edit.two_group.hodges_lehmann")?.state.enablement.enabled, "R4: hodges_lehmann enabled for MannWhitneyU")?;
    click_in_content_scroll(&mut driver, "edit.two_group.two_tailed")?;
    click_in_content_scroll(&mut driver, "edit.two_group.graph_ranks")?;

    click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    let rs = driver.app().document().result_sheets.first().ok_or_else(|| fail("Missing TwoGroup ResultSheet"))?;
    let AnalysisKind::TwoGroupComparison(ref tg) = rs.analysis.kind else {
        return Err(fail("Expected TwoGroupComparison"));
    };
    check(tg.method == TwoGroupMethod::MannWhitneyU, "Expected MannWhitneyU method persisted")?;
    check(!tg.two_tailed, "Expected one-tailed persisted")?;
    Ok(())
}

#[test]
fn w5f2_contingency_2x2_vs_3x2_survival_and_roc() -> Result<(), String> {
    // 2x2 Contingency
    let cont2 = make_contingency_sheet("cont-2x2", 2, 2)?;
    let mut d2 = driver_for_sheet(cont2)?;
    open_analysis_and_edit_via_cmd_t(&mut d2, "stats.contingency")?;
    assert_all_tabs_clean(&mut d2, false)?;
    check(require_node(&mut d2, "edit.contingency.fisher")?.state.enablement.enabled, "2x2 Fisher must be enabled")?;
    click_widget_center(&mut d2, &WidgetId::from("edit.tab.options"))?;
    check(require_node(&mut d2, "edit.contingency.odds_ratio")?.state.enablement.enabled, "2x2 OR must be enabled")?;
    click_in_content_scroll(&mut d2, "edit.contingency.odds_ratio")?;
    let or_ci_en = require_node(&mut d2, "edit.contingency.ci.baptista_pike")?.state.enablement.enabled;
    check(or_ci_en == d2.app().canvas_dialogs.edit_params.contingency_odds_ratio_checked, "R6 OR CI enablement must match checkbox")?;

    // 3x2 Contingency (R5 disables 2x2 Fisher, enables Cochran-Armitage Trend)
    let cont3 = make_contingency_sheet("cont-3x2", 3, 2)?;
    let mut d3 = driver_for_sheet(cont3)?;
    open_analysis_and_edit_via_cmd_t(&mut d3, "stats.contingency")?;
    assert_zero_shortcut_compliance(&mut d3)?;
    check(!require_node(&mut d3, "edit.contingency.fisher")?.state.enablement.enabled, "R5: 3x2 Fisher must disable")?;
    check(require_node(&mut d3, "edit.contingency.cochran_armitage")?.state.enablement.enabled, "R5: 3x2 Trend test must enable")?;
    click_in_content_scroll(&mut d3, "edit.contingency.cochran_armitage")?;
    click_widget_center(&mut d3, &WidgetId::from("modal.ok"))?;
    let rs_cont = d3.app().document().result_sheets.first().ok_or_else(|| fail("Missing Contingency ResultSheet"))?;
    check(
        matches!(rs_cont.analysis.kind, AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::CochranArmitageTrend),
        "Expected CochranArmitageTrend persisted",
    )?;

    // Kaplan-Meier Survival
    let surv = make_survival_sheet("surv-1")?;
    let mut dk = driver_for_sheet(surv)?;
    open_analysis_and_edit_via_cmd_t(&mut dk, "stats.survival_km")?;
    assert_all_tabs_clean(&mut dk, false)?;
    click_in_content_scroll(&mut dk, "edit.km.gehan_breslow_wilcoxon")?;
    click_widget_center(&mut dk, &WidgetId::from("edit.tab.options"))?;
    click_in_content_scroll(&mut dk, "edit.km.asymmetrical_greenwood_ci")?;
    click_widget_center(&mut dk, &WidgetId::from("modal.ok"))?;
    let rs_km = dk.app().document().result_sheets.first().ok_or_else(|| fail("Missing KM ResultSheet"))?;
    check(
        matches!(rs_km.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { gehan_breslow_wilcoxon: true, .. })),
        "Expected Gehan-Breslow-Wilcoxon enabled on KM ResultSheet",
    )?;

    // Cox Proportional Hazards
    let mv_cox = make_mv_sheet("mv-cox")?;
    let mut dc = driver_for_sheet(mv_cox)?;
    open_analysis_and_edit_via_cmd_t(&mut dc, "stats.survival_cox")?;
    assert_all_tabs_clean(&mut dc, false)?;
    click_in_content_scroll(&mut dc, "edit.cox.tie.breslow")?;
    click_widget_center(&mut dc, &WidgetId::from("modal.ok"))?;
    let rs_cox = dc.app().document().result_sheets.first().ok_or_else(|| fail("Missing Cox ResultSheet"))?;
    check(
        matches!(
            rs_cox.analysis.kind,
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: CoxTieMethod::Breslow, .. })
        ),
        "Expected Breslow tie method persisted",
    )?;
    press_cmd_shortcut(&mut dc, "t")?;
    click_in_content_scroll(&mut dc, "edit.cox.tie.exact")?;
    click_widget_center(&mut dc, &WidgetId::from("modal.ok"))?;
    let rs_cox_exact = dc.app().document().result_sheets.first().ok_or_else(|| fail("Missing Cox ResultSheet"))?;
    check(
        matches!(
            rs_cox_exact.analysis.kind,
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: CoxTieMethod::Exact, .. })
        ),
        "Expected Exact tie method persisted",
    )?;

    // ROC Curve
    let roc_col = make_column_sheet(
        "col-roc",
        "Diagnostic Assay",
        ColumnDesignHint::UnpairedReplicates,
        &[vec![1.1, 1.8, 2.2, 2.5, 1.9], vec![4.2, 5.0, 5.6, 6.1, 4.9]],
    )?;
    let mut dr = driver_for_sheet(roc_col)?;
    open_analysis_and_edit_via_cmd_t(&mut dr, "stats.roc")?;
    assert_all_tabs_clean(&mut dr, false)?;
    click_in_content_scroll(&mut dr, "edit.roc.ci.hanley_mcneil")?;
    click_widget_center(&mut dr, &WidgetId::from("modal.ok"))?;
    let rs_roc = dr.app().document().result_sheets.first().ok_or_else(|| fail("Missing ROC ResultSheet"))?;
    check(
        matches!(rs_roc.analysis.kind, AnalysisKind::RocCurve(ref r) if r.ci_method == RocCiMethod::HanleyMcNeil),
        "Expected HanleyMcNeil ROC CI persisted",
    )?;
    Ok(())
}

#[test]
fn w5f2_glm_multi_tab_and_nonlinear_fit_delegation() -> Result<(), String> {
    let mv = make_mv_sheet("mv-glm")?;
    let mut driver = driver_for_sheet(mv)?;
    open_analysis_and_edit_via_cmd_t(&mut driver, "stats.glm")?;
    assert_all_tabs_clean(&mut driver, false)?;

    click_in_content_scroll(&mut driver, "edit.glm.family.logistic")?;
    click_widget_center(&mut driver, &WidgetId::from("edit.tab.factor_settings"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    click_in_content_scroll(&mut driver, "edit.glm.resp.1")?;

    click_widget_center(&mut driver, &WidgetId::from("edit.tab.diagnostics"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    // GlmSpec::default() initializes compute_vif = true; toggling once disables it and persists to ResultSheet.
    click_in_content_scroll(&mut driver, "edit.diag.influence_leverage")?;
    let diag_preview = require_node(&mut driver, "edit.diag.tabs_preview")?.a11y.label.clone();
    check(diag_preview.contains("Classification Table"), format!("Expected logistic diagnostics preview, got `{diag_preview}`"))?;

    click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    let rs_glm = driver.app().document().result_sheets.first().ok_or_else(|| fail("Missing GLM ResultSheet"))?;
    let AnalysisKind::GeneralizedLinearModel(ref glm) = rs_glm.analysis.kind else {
        return Err(fail("Expected GeneralizedLinearModel"));
    };
    check(glm.family == GlmFamily::Logistic, "Expected GlmFamily::Logistic persisted")?;
    check(glm.response_column == 1, "Expected response column 1 persisted")?;
    check(!glm.compute_vif, "Expected compute_vif toggled false via Diagnostics flow")?;

    // Nonlinear regression must cleanly delegate to `fit_params` (no statistical RM tabs rendered).
    let xy = make_xy_sheet("xy-fit")?;
    let mut fit_drv = driver_for_sheet(xy)?;
    open_analysis_and_edit_via_cmd_t(&mut fit_drv, "fit.nonlinear")?;
    assert_zero_shortcut_compliance(&mut fit_drv)?;
    check(fit_drv.find_widget("edit.tab.repeated_measures").is_none(), "Nonlinear fit must delegate to fit_params without stat RM tab")?;
    Ok(())
}

#[test]
fn w5f2_multilocale_and_hidpi_parity() -> Result<(), String> {
    for locale in ["en", "de", "ja", "ar"] {
        let col = make_column_sheet(
            "col-loc",
            "Locale Check",
            ColumnDesignHint::PairedRepeatedMeasures,
            &[vec![2.0, 2.5, 3.0], vec![5.0, 5.8, 6.2], vec![3.1, 3.6, 4.0]],
        )?;
        let sid = col.id.clone();
        let mut doc = ProjectDocument::new("Locale Project").map_err(err_str)?;
        doc.add_data_sheet(col).map_err(err_str)?;
        let spec = AnalysisSpec::new(
            "res-loc",
            sid.clone(),
            AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec {
                method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
                ..Default::default()
            }),
        )
        .map_err(err_str)?;
        let res = curvebook_doc::ResultSheet::new(spec).map_err(err_str)?;
        let rsid = res.id.clone();
        doc.add_result_sheet(res).map_err(err_str)?;
        doc.active_sheet_id = Some(rsid);

        let mut app = UiApp::from_session(Session::from_document(doc));
        app.set_locale(locale);
        app.set_human_path_only(true);
        let mut driver = new_human_driver(app, default_viewport());
        press_cmd_shortcut(&mut driver, "t")?;
        check(require_node(&mut driver, "edit.tab.repeated_measures")?.state.enablement.enabled, "RM tab must be enabled in multilocale check")?;
        assert_all_tabs_clean(&mut driver, true)?;
    }

    // Verify HiDPI 1.5x / 2.0x layout and interaction dispatch on EditAnalysisParameters.
    for scale in [1.5_f64, 2.0] {
        let col = make_column_sheet(
            "col-hi",
            "HiDPI Check",
            ColumnDesignHint::PairedRepeatedMeasures,
            &[vec![2.0, 2.5, 3.0], vec![5.0, 5.8, 6.2], vec![3.1, 3.6, 4.0]],
        )?;
        let sid = col.id.clone();
        let mut doc = ProjectDocument::new("HiDPI Project").map_err(err_str)?;
        doc.add_data_sheet(col).map_err(err_str)?;
        let spec = AnalysisSpec::new(
            "res-hi",
            sid,
            AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec {
                method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
                ..Default::default()
            }),
        )
        .map_err(err_str)?;
        let res = curvebook_doc::ResultSheet::new(spec).map_err(err_str)?;
        let rsid = res.id.clone();
        doc.add_result_sheet(res).map_err(err_str)?;
        doc.active_sheet_id = Some(rsid);

        let mut app = UiApp::from_session(Session::from_document(doc));
        app.set_human_path_only(true);
        let metrics = curvebook_ui_egui::ViewportMetrics::new(1280.0, 800.0, scale);
        let mut driver = new_human_driver(app, metrics);
        press_cmd_shortcut(&mut driver, "t")?;
        click_widget_center(&mut driver, &WidgetId::from("edit.tab.multiple_comparisons"))?;
        let img = driver.render_frame().map_err(err_str)?;
        check(!img.rgba.is_empty(), "Expected non-empty HiDPI framebuffer")?;
    }
    Ok(())
}
