//! Wave 7 UI & Human-Path acceptance tests for R3.6 Mixed-Effects REML & R10.3 b2 (pure `egui` shell).

#![forbid(unsafe_code)]

use crate::common::{TestRes, ensure, result_scalar, seed_column_doc};
use crate::human_wf_common::EguiWfHarness;
use crate::suite_helpers::{build_mv_sheet, new_shell_app, run_sheet_cmd};
use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, MixedModelDfMethod, MultiplicityMethod, NestedTable, ProjectDocument,
    ResultSheet, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole,
};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::FrameInspectNodeMeta;
use curvebook_ui_egui::i18n::{LOCALES, try_tr};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::modal_predicates::evaluate_dialog_widget;
use egui::{Pos2, vec2};
use serde_json::{Value, json};

fn require_widget(h: &EguiWfHarness, wid: &str) -> Result<FrameInspectNodeMeta, String> {
    h.driver.frame_meta().find_by_widget_id(wid).cloned().ok_or_else(|| format!("missing widget `{wid}` in FrameInspectMeta"))
}

fn scroll_into_modal_view(h: &mut EguiWfHarness, wid: &str) -> Result<FrameInspectNodeMeta, String> {
    let is_vis = |h: &EguiWfHarness| -> Option<FrameInspectNodeMeta> {
        h.driver.frame_meta().find_by_widget_id(wid).filter(|e| e.clipped_rect.is_positive() && e.clipped_rect.height() >= 8.0).cloned()
    };
    if let Some(hit) = is_vis(h) {
        return Ok(hit);
    }
    let anchor = h
        .driver
        .frame_meta()
        .find_by_widget_id("modal.ok")
        .or_else(|| h.driver.frame_meta().find_by_widget_id("edit.tab.experimental_design"))
        .map_or(Pos2::new(720.0, 450.0), |e| Pos2::new(e.clipped_rect.center().x, 420.0));
    for dy in [-140.0, 140.0] {
        for _ in 0..8 {
            let _ = h.driver.wheel_at(anchor, vec2(0.0, dy));
            if let Some(hit) = is_vis(h) {
                return Ok(hit);
            }
        }
    }
    require_widget(h, wid)
}

fn click_modal_widget(h: &mut EguiWfHarness, wid: &str) -> TestRes {
    let entry = scroll_into_modal_view(h, wid)?;
    h.driver.click_at(entry.clipped_rect.center())
}

fn assert_disabled_tooltip_and_noop_click(h: &mut EguiWfHarness, wid: &str) -> TestRes {
    let entry = scroll_into_modal_view(h, wid)?;
    ensure(!entry.enabled, format!("expected disabled `{wid}`, got enabled"))?;
    ensure(entry.disabled_reason.as_deref().is_some_and(|t| !t.trim().is_empty()), format!("expected disabled tooltip for `{wid}`"))?;
    let before = h.app().canvas_dialogs.edit_params.draft.clone();
    let _ = h.driver.hover_widget_id(wid);
    let _ = h.driver.click_at(entry.clipped_rect.center());
    ensure(h.app().canvas_dialogs.edit_params.draft == before, format!("clicking disabled `{wid}` must be a no-op"))
}

fn result_sheet<'a>(app: &'a UiApp, id: &SheetId) -> Result<&'a ResultSheet, String> {
    app.document().result_sheet(id).ok_or_else(|| format!("missing result sheet `{}`", id.as_str()))
}

fn make_rm_column_doc(project: &str, id: &str, title: &str, cols: [&str; 4], rows: &[[Option<f64>; 4]]) -> Result<ProjectDocument, String> {
    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 4, rows.len()).map_err(|e| e.to_string())?;
    for (c, lbl) in cols.into_iter().enumerate() {
        tab.data_sets[c].title = lbl.into();
    }
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if let Some(num) = v {
                tab.data_sets[c].cells[r][0] = Cell::number(*num);
            }
        }
    }
    let mut doc = ProjectDocument::new(project).map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new(SheetId::new(id), title, TableArchetype::Column(tab)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from(id)).map_err(|e| e.to_string())?;
    Ok(doc)
}

const INCOMPLETE_RM_ROWS: [[Option<f64>; 4]; 8] = [
    [Some(10.2), Some(13.1), Some(17.8), Some(23.3)],
    [Some(12.5), None, Some(20.4), Some(23.4)],
    [Some(9.1), Some(12.5), None, Some(21.4)],
    [Some(14.8), Some(17.6), Some(22.7), None],
    [Some(11.0), Some(15.3), Some(19.3), Some(23.0)],
    [None, Some(16.3), Some(21.1), Some(26.8)],
    [Some(13.1), Some(16.5), Some(19.9), Some(25.1)],
    [Some(10.7), Some(13.7), Some(18.6), Some(22.6)],
];

const COMPLETE_RM_ROWS: [[Option<f64>; 4]; 6] = [
    [Some(10.2), Some(13.1), Some(17.8), Some(23.3)],
    [Some(12.5), Some(15.6), Some(20.4), Some(23.4)],
    [Some(9.1), Some(12.5), Some(16.9), Some(21.4)],
    [Some(14.8), Some(17.6), Some(22.7), Some(26.2)],
    [Some(11.0), Some(15.3), Some(19.3), Some(23.0)],
    [Some(13.1), Some(16.5), Some(19.9), Some(25.1)],
];

fn run_and_open_edit(doc: ProjectDocument, analysis_params: Value) -> Result<(EguiWfHarness, SheetId), String> {
    let mut app = new_shell_app(doc);
    let res_id = run_sheet_cmd(&mut app, "analysis.run", analysis_params)?;
    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&res_id)?;
    h.click("toolbar.edit_params")?;
    Ok((h, res_id))
}

#[test]
fn w7_r36_r1_incomplete_rm_table_allows_matching_and_switches_kr_vs_satterthwaite() -> TestRes {
    let doc = make_rm_column_doc("R3.6 Incomplete RM UI", "ds-rm-inc", "Incomplete RM Blocks", ["C1", "C2", "C3", "C4"], &INCOMPLETE_RM_ROWS)?;
    let (mut h, res_id) = run_and_open_edit(
        doc,
        json!({ "source_sheet_id": "ds-rm-inc", "kind": "rm_one_way_anova", "residual_covariance": "unstructured", "multiplicity": "bonferroni" }),
    )?;
    let (df_kr, p_kr) = {
        let rs = result_sheet(h.app(), &res_id)?;
        (result_scalar(rs, "Denominator df")?, result_scalar(rs, "P value")?)
    };
    h.assert_parity_and_layout("incomplete_rm_design_tab")?;
    let paired_node = require_widget(&h, "edit.paired_or_matched")?;
    ensure(paired_node.enabled && paired_node.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false), format!("got {paired_node:?}"))?;

    h.click("edit.tab.repeated_measures")?;
    h.assert_parity_and_layout("incomplete_rm_rm_tab")?;
    ensure(
        scroll_into_modal_view(&mut h, "edit.rm.df_kenward_roger")?.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false),
        "KR df must be default",
    )?;
    assert_disabled_tooltip_and_noop_click(&mut h, "edit.sphericity.geisser_greenhouse")?;
    assert_disabled_tooltip_and_noop_click(&mut h, "edit.rm.assume_sphericity")?;

    click_modal_widget(&mut h, "edit.rm.df_satterthwaite")?;
    h.click("modal.ok")?;
    let rs_sat = result_sheet(h.app(), &res_id)?;
    let (df_sat, p_sat) = (result_scalar(rs_sat, "Denominator df")?, result_scalar(rs_sat, "P value")?);
    ensure(
        (df_kr - df_sat).abs() >= 1e-3 && (p_kr - p_sat).abs() >= 1e-8,
        format!("expected KR vs Sat to differ: ({df_kr},{p_kr}) vs ({df_sat},{p_sat})"),
    )
}

#[test]
fn w7_r36_r1b_complete_rm_reml_toggle_disables_sphericity_and_restricts_post_hoc() -> TestRes {
    let doc = make_rm_column_doc("R10.3b2 Complete RM REML", "ds-rm-comp", "Complete RM Blocks", ["T0", "T1", "T2", "T3"], &COMPLETE_RM_ROWS)?;
    let (mut h, res_id) = run_and_open_edit(doc, json!({ "source_sheet_id": "ds-rm-comp", "kind": "rm_one_way_anova", "multiplicity": "tukey" }))?;

    h.click("edit.tab.repeated_measures")?;
    ensure(scroll_into_modal_view(&mut h, "edit.sphericity.geisser_greenhouse")?.enabled, "complete RM keeps sphericity enabled")?;
    h.click("edit.tab.multiple_comparisons")?;
    ensure(scroll_into_modal_view(&mut h, "edit.post_hoc.tukey")?.enabled, "complete RM allows Tukey before REML")?;

    h.click("edit.tab.repeated_measures")?;
    click_modal_widget(&mut h, "edit.rm.mixed_effects_reml")?;
    h.assert_parity_and_layout("complete_rm_reml_active")?;
    for wid in ["edit.sphericity.geisser_greenhouse", "edit.rm.assume_sphericity"] {
        assert_disabled_tooltip_and_noop_click(&mut h, wid)?;
    }

    h.click("edit.tab.multiple_comparisons")?;
    h.assert_parity_and_layout("reml_mc_tab")?;
    for disallowed in ["edit.post_hoc.tukey", "edit.post_hoc.dunnett", "edit.post_hoc.games_howell", "edit.post_hoc.dunn"] {
        assert_disabled_tooltip_and_noop_click(&mut h, disallowed)?;
    }
    for allowed in [
        "edit.post_hoc.bonferroni",
        "edit.post_hoc.sidak",
        "edit.post_hoc.holm_sidak",
        "edit.post_hoc.fisher_lsd",
        "edit.post_hoc.bh",
        "edit.post_hoc.bky",
        "edit.post_hoc.by",
        "edit.post_hoc.none",
    ] {
        ensure(scroll_into_modal_view(&mut h, allowed)?.enabled, format!("expected `{allowed}` enabled under REML"))?;
    }
    click_modal_widget(&mut h, "edit.post_hoc.sidak")?;
    click_modal_widget(&mut h, "edit.post_hoc.fisher_lsd")?;
    h.click("modal.ok")?;

    let rs = result_sheet(h.app(), &res_id)?;
    let AnalysisKind::OneWayAnova(spec) = &rs.analysis.kind else { return Err(format!("expected OneWayAnova, got {:?}", rs.analysis.kind)) };
    ensure(spec.mixed_effects_reml && spec.multiplicity.method == MultiplicityMethod::FisherLsd, format!("unexpected spec {spec:?}"))?;
    ensure(rs.tabs.iter().any(|t| t.kind == ResultTabKind::MultipleComparisons), "expected MultipleComparisons tab")
}

#[test]
fn w7_r36_r4_welch_t_disables_diagnostics_residuals_with_tooltip_and_noop() -> TestRes {
    let (doc, _) = seed_column_doc(
        "R4 Welch Diagnostics",
        "ds-tg",
        "Two Group",
        ColumnDesignHint::UnpairedReplicates,
        &[("Ctrl", &[10.0, 11.2, 9.8, 10.5, 11.1]), ("Treated", &[15.2, 16.0, 14.9, 17.3, 15.8])],
    )?;
    let (mut h, res_id) = run_and_open_edit(doc, json!({ "source_sheet_id": "ds-tg", "kind": "unpaired_t_test" }))?;

    h.click("edit.tab.diagnostics")?;
    ensure(require_widget(&h, "edit.diag.qq")?.enabled && require_widget(&h, "edit.diag.shapiro")?.enabled, "Student's t enables diagnostics")?;
    h.click("edit.tab.experimental_design")?;
    click_modal_widget(&mut h, "edit.tg.welch_t")?;
    h.assert_parity_and_layout("welch_t_experimental_design")?;
    assert_disabled_tooltip_and_noop_click(&mut h, "edit.tab.diagnostics")?;
    ensure(
        !evaluate_dialog_widget(&h.app().canvas_dialogs, h.app().document(), "edit.diag.residuals_table").enabled,
        "residuals_table disabled under WelchT",
    )?;

    h.click("modal.ok")?;
    let AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, .. }) = result_sheet(h.app(), &res_id)?.analysis.kind else {
        return Err("expected TwoGroupComparison".into());
    };
    ensure(method == TwoGroupMethod::WelchT, format!("expected WelchT, got {method:?}"))
}

#[test]
fn w7_r36_r103b2_multifactor_anova_preserves_interactions_across_factor_add_remove() -> TestRes {
    let ds = build_mv_sheet(
        "ds-mv-fac",
        "Multifactor 4x2",
        &[
            ("Genotype", VariableRole::Categorical),
            ("Drug", VariableRole::Categorical),
            ("Diet", VariableRole::Categorical),
            ("Timepoint", VariableRole::Categorical),
            ("Response", VariableRole::Continuous),
        ],
        32,
        |mv| {
            for row in 0..32usize {
                let (g, d, dt, tp, rep) = ((row / 16) % 2, (row / 8) % 2, (row / 4) % 2, (row / 2) % 2, row % 2);
                mv.variables[0].cells[row] = Cell::text(if g == 0 { "WT" } else { "KO" });
                mv.variables[1].cells[row] = Cell::text(if d == 0 { "Veh" } else { "Tx" });
                mv.variables[2].cells[row] = Cell::text(if dt == 0 { "Chow" } else { "HFD" });
                mv.variables[3].cells[row] = Cell::text(if tp == 0 { "Baseline" } else { "Week4" });
                let y = 10.0
                    + 2.0 * (g as f64)
                    + 3.0 * (d as f64)
                    + 1.5 * (dt as f64)
                    + 4.0 * (tp as f64)
                    + 2.5 * ((g * d * dt) as f64)
                    + if rep == 0 { -0.25 } else { 0.25 };
                mv.variables[4].cells[row] = Cell::number(y);
            }
        },
    )?;
    let mut doc = ProjectDocument::new("Multifactor Interaction Preservation").map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("ds-mv-fac")).map_err(|e| e.to_string())?;
    let (mut h, res_id) = run_and_open_edit(
        doc,
        json!({ "source_sheet_id": "ds-mv-fac", "kind": "two_way_anova", "response_column": 4, "multifactor_columns": [0, 1, 2] }),
    )?;
    h.click("edit.tab.experimental_design")?;
    h.assert_parity_and_layout("mv_factor_settings_tab")?;

    click_modal_widget(&mut h, "edit.fac.order.3")?;
    ensure(
        matches!(h.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::FactorialAnova(f)) if f.max_interaction_order == 3),
        "expected max_interaction_order=3 after clicking edit.fac.order.3",
    )?;
    for wid in ["edit.fac.col.2", "edit.fac.col.3", "edit.fac.col.1", "edit.fac.col.2"] {
        click_modal_widget(&mut h, wid)?;
    }
    let Some(AnalysisKind::FactorialAnova(f)) = h.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) else {
        return Err("missing FactorialAnova draft".into());
    };
    ensure(f.multifactor_columns == vec![0, 2, 3] && f.max_interaction_order == 3, format!("got {f:?}"))?;
    ensure(require_widget(&h, "edit.formula_bar")?.label.contains("Genotype:Diet:Timepoint"), "missing preserved 3-way term in formula bar")?;

    h.click("modal.ok")?;
    let AnalysisKind::FactorialAnova(fac) = &result_sheet(h.app(), &res_id)?.analysis.kind else { return Err("expected FactorialAnova".into()) };
    ensure(fac.max_interaction_order == 3 && fac.multifactor_columns == vec![0, 2, 3], format!("got {fac:?}"))
}

#[test]
fn w7_r36_3way_rm_and_nested_df_method_human_path_e2e() -> TestRes {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(4), 4, 2).map_err(|e| e.to_string())?;
    grp.row_factor_title = "C".into();
    grp.column_factor_title = "A_B".into();
    grp.three_way_grouping = Some(curvebook_doc::ThreeWayColumnGrouping {
        factor_1_name: "A".into(),
        factor_1_levels: ["A1".into(), "A2".into()],
        factor_2_name: "B".into(),
        factor_2_levels: ["B1".into(), "B2".into()],
    });
    for c in 0..4 {
        grp.data_sets[c].title = format!("C{c}");
        for r in 0..2 {
            for s in 0..4 {
                if !(c == 3 && r == 1 && s == 3) {
                    let jitter = (((s * 5 + c * 3 + r * 7) % 5) as f64) * 0.17;
                    let inter3 = if c == 3 && r == 1 { 1.4 } else { 0.0 };
                    grp.data_sets[c].cells[r][s] = Cell::number(10.0 + (c as f64) * 2.1 + (r as f64) * 3.4 + (s as f64) * 0.9 + jitter + inter3);
                }
            }
        }
    }
    let mut nt = NestedTable::new(3, 5, 5).map_err(|e| e.to_string())?;
    let nested_vals: [&[&[f64]]; 3] = [
        &[&[10.1], &[12.8, 13.4, 13.1, 13.6], &[8.5, 8.9], &[15.2, 15.8, 15.4, 15.6, 15.1]],
        &[&[14.2, 14.9, 14.4], &[18.1, 18.6], &[11.8, 12.1, 12.0, 12.3], &[16.5], &[17.2, 17.8]],
        &[&[19.2, 19.6, 19.4, 19.9, 19.1], &[22.4, 22.8], &[16.5, 16.9, 17.1]],
    ];
    for (g, cl) in nested_vals.iter().enumerate() {
        nt.groups[g].title = format!("Grp{}", g + 1);
        for (b, reps) in cl.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                nt.groups[g].cells[r][b] = Cell::number(v);
            }
        }
    }
    let mut doc = ProjectDocument::new("3way RM & Nested E2E").map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("ds-3way-rm", "3way RM", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("ds-nest", "Nested Unbalanced", TableArchetype::Nested(nt)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("ds-3way-rm")).map_err(|e| e.to_string())?;

    let mut app = new_shell_app(doc);
    let r3_id =
        run_sheet_cmd(&mut app, "analysis.run", json!({ "source_sheet_id": "ds-3way-rm", "kind": "three_way_anova", "repeated_measures": "both" }))?;
    let rn_id = run_sheet_cmd(&mut app, "analysis.run", json!({ "source_sheet_id": "ds-nest", "kind": "nested_anova" }))?;
    let nest_df_kr = result_scalar(result_sheet(&app, &rn_id)?, "Group denominator df")?;
    let nest_f_kr = result_scalar(result_sheet(&app, &rn_id)?, "F (among groups)")?;
    let r3_kr_df = result_scalar(result_sheet(&app, &r3_id)?, "A × B × C: Kenward-Roger df")?;

    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&r3_id)?;
    h.click("toolbar.edit_params")?;
    h.click("edit.tab.repeated_measures")?;
    click_modal_widget(&mut h, "edit.rm.df.sat")?;
    h.click("modal.ok")?;
    let AnalysisKind::FactorialAnova(fac_done) = &result_sheet(h.app(), &r3_id)?.analysis.kind else { return Err("expected FactorialAnova".into()) };
    ensure(fac_done.df_method == MixedModelDfMethod::Satterthwaite && r3_kr_df.is_finite(), format!("got {fac_done:?}"))?;

    h.select_sheet(&rn_id)?;
    h.click("toolbar.edit_params")?;
    click_modal_widget(&mut h, "edit.rm.df.sat")?;
    h.click("modal.ok")?;
    let rsn = result_sheet(h.app(), &rn_id)?;
    let AnalysisKind::NestedComparison(nest_done) = &rsn.analysis.kind else { return Err("expected NestedComparison".into()) };
    let (nest_df_sat, nest_f_sat) = (result_scalar(rsn, "Group denominator df")?, result_scalar(rsn, "F (among groups)")?);
    ensure(
        nest_done.df_method == MixedModelDfMethod::Satterthwaite
            && (nest_df_kr - nest_df_sat).abs() > 1e-8
            && nest_f_kr.is_finite()
            && nest_f_sat.is_finite()
            && nest_f_kr < nest_f_sat,
        format!("df=({nest_df_kr}, {nest_df_sat}), F=({nest_f_kr}, {nest_f_sat})"),
    )
}

#[test]
fn w7_r36_staged_i18n_16_locales_and_rtl_hidpi_parity() -> TestRes {
    for &loc in LOCALES {
        for key in [
            "stat_params.rm.mixed_effects_reml",
            "stat_params.rm.df_kenward_roger",
            "stat_params.rm.df_satterthwaite",
            "stat_params.post_hoc.sidak",
            "stat_params.post_hoc.fisher_lsd",
            "stat_params.fac.interactions",
        ] {
            ensure(try_tr(loc, key).is_some_and(|s| !s.is_empty()), format!("missing translation `{loc}` `{key}`"))?;
        }
    }
    let doc = make_rm_column_doc("RTL HiDPI RM REML", "ds-rtl", "Incomplete RM Blocks", ["C1", "C2", "C3", "C4"], &INCOMPLETE_RM_ROWS)?;
    let mut app = new_shell_app(doc);
    let res_id = run_sheet_cmd(&mut app, "analysis.run", json!({ "source_sheet_id": "ds-rtl", "kind": "rm_one_way_anova" }))?;
    app.locale = "ar".into();
    let mut h = EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 900.0, 1.5));
    h.select_sheet(&res_id)?;
    h.click("toolbar.edit_params")?;
    for (tab, lbl) in [("edit.tab.repeated_measures", "rtl_hidpi_rm_tab"), ("edit.tab.multiple_comparisons", "rtl_hidpi_mc_tab")] {
        h.click(tab)?;
        h.assert_parity_and_layout(lbl)?;
    }
    Ok(())
}
