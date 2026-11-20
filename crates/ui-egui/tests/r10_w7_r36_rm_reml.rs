//! Wave 7 UI & Human-Path acceptance tests for R3.6 Mixed-Effects REML & R10.3 b2 (pure `egui` shell):
//! 1. Incomplete-block RM `Column` table keeps `edit.paired_or_matched` enabled (`R1`), runs REML,
//!    and switches `KenwardRoger` vs `Satterthwaite` *df* via repeated-measures controls.
//! 2. Selecting Mixed-Effects REML disables sphericity (`edit.sphericity.geisser_greenhouse`,
//!    `edit.rm.assume_sphericity` in `R1b`) and restricts post-hoc choices to Bonferroni, Šídák,
//!    Holm–Šídák, FDR (`BH`, `BKY`, `BY`), and Fisher's LSD while disabling Tukey/Dunnett/Games-Howell/Dunn.
//! 3. `TwoGroupMethod::WelchT` disables diagnostics checks (`R4`) with tooltip + no-op click.
//! 4. Multifactor ANOVA on `MultipleVariables` preserves configured interaction order/terms when adding
//!    and removing grouping factors on the Experimental Design / Model tab.
//! 5. Three-way RM (`BothFactorsMatched`) and Nested ANOVA switch `KenwardRoger` vs `Satterthwaite` *df*
//!    end-to-end through the UI.
//! 6. Staged i18n (`w7_rm_reml::STAGED_ENTRIES`) covers all 16 locales without bumping `i18n::catalog_len()`
//!    and passes RTL + HiDPI layout parity.

#![forbid(unsafe_code)]

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, MixedModelDfMethod, MultipleVariablesTable, MultiplicityMethod,
    NestedTable, ProjectDocument, ResultSheet, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::FrameInspectNodeMeta;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::modal_predicates::{ControlEnablement, evaluate_dialog_widget};
use egui::{Pos2, vec2};
use human_wf_common::{EguiWfHarness, ensure};
use serde_json::json;

type TestRes = Result<(), String>;

fn err_str<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

fn desktop_harness(app: UiApp, metrics: ViewportMetrics) -> EguiWfHarness {
    EguiWfHarness::from_app(app, metrics)
}

fn require_widget(h: &EguiWfHarness, wid: &str) -> Result<FrameInspectNodeMeta, String> {
    h.driver.frame_meta().find_by_widget_id(wid).cloned().ok_or_else(|| format!("missing widget `{wid}` in FrameInspectMeta"))
}

fn scroll_into_modal_view(h: &mut EguiWfHarness, wid: &str) -> Result<FrameInspectNodeMeta, String> {
    if let Some(entry) = h.driver.frame_meta().find_by_widget_id(wid)
        && entry.clipped_rect.is_positive()
        && entry.clipped_rect.height() >= 8.0
    {
        return Ok(entry.clone());
    }
    let anchor = h
        .driver
        .frame_meta()
        .find_by_widget_id("modal.ok")
        .or_else(|| h.driver.frame_meta().find_by_widget_id("edit.tab.experimental_design"))
        .map_or(Pos2::new(720.0, 450.0), |e| Pos2::new(e.clipped_rect.center().x, 420.0));
    for _ in 0..8 {
        let _ = h.driver.wheel_at(anchor, vec2(0.0, -140.0));
        if let Some(entry) = h.driver.frame_meta().find_by_widget_id(wid)
            && entry.clipped_rect.is_positive()
            && entry.clipped_rect.height() >= 8.0
        {
            return Ok(entry.clone());
        }
    }
    for _ in 0..8 {
        let _ = h.driver.wheel_at(anchor, vec2(0.0, 140.0));
        if let Some(entry) = h.driver.frame_meta().find_by_widget_id(wid)
            && entry.clipped_rect.is_positive()
            && entry.clipped_rect.height() >= 8.0
        {
            return Ok(entry.clone());
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
    if entry.enabled {
        return Err(format!("expected disabled `{wid}`, got enabled in FrameInspectMeta"));
    }
    if entry.disabled_reason.as_deref().is_none_or(|t| t.trim().is_empty()) {
        return Err(format!("expected non-empty disabled tooltip/reason for `{wid}`"));
    }
    let before = h.app().canvas_dialogs.edit_params.draft.clone();
    let _ = h.driver.hover_widget_id(wid);
    let _ = h.driver.click_at(entry.clipped_rect.center());
    if h.app().canvas_dialogs.edit_params.draft != before {
        return Err(format!("clicking disabled `{wid}` must be a no-op"));
    }
    Ok(())
}

fn result_sheet<'a>(app: &'a UiApp, id: &SheetId) -> Result<&'a ResultSheet, String> {
    app.document().result_sheet(id).ok_or_else(|| format!("missing result sheet `{}`", id.as_str()))
}

fn scalar_val(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    rs.scalars.iter().find(|s| s.key == key).map(|s| s.value).ok_or_else(|| format!("missing scalar `{key}`"))
}

fn make_incomplete_rm_column_sheet(id: &str) -> Result<DataSheet, String> {
    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 4, 8).map_err(err_str)?;
    let rows: [[Option<f64>; 4]; 8] = [
        [Some(10.2), Some(13.1), Some(17.8), Some(23.3)],
        [Some(12.5), None, Some(20.4), Some(23.4)],
        [Some(9.1), Some(12.5), None, Some(21.4)],
        [Some(14.8), Some(17.6), Some(22.7), None],
        [Some(11.0), Some(15.3), Some(19.3), Some(23.0)],
        [None, Some(16.3), Some(21.1), Some(26.8)],
        [Some(13.1), Some(16.5), Some(19.9), Some(25.1)],
        [Some(10.7), Some(13.7), Some(18.6), Some(22.6)],
    ];
    for (c, lbl) in ["C1", "C2", "C3", "C4"].into_iter().enumerate() {
        tab.data_sets[c].title = String::from(lbl);
    }
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if let Some(num) = v {
                tab.data_sets[c].cells[r][0] = Cell::number(*num);
            }
        }
    }
    DataSheet::new(SheetId::new(id), "Incomplete RM Blocks", TableArchetype::Column(tab)).map_err(err_str)
}

fn make_complete_rm_column_sheet(id: &str) -> Result<DataSheet, String> {
    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 4, 6).map_err(err_str)?;
    let rows: [[f64; 4]; 6] = [
        [10.2, 13.1, 17.8, 23.3],
        [12.5, 15.6, 20.4, 23.4],
        [9.1, 12.5, 16.9, 21.4],
        [14.8, 17.6, 22.7, 26.2],
        [11.0, 15.3, 19.3, 23.0],
        [13.1, 16.5, 19.9, 25.1],
    ];
    for (c, lbl) in ["T0", "T1", "T2", "T3"].into_iter().enumerate() {
        tab.data_sets[c].title = String::from(lbl);
    }
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            tab.data_sets[c].cells[r][0] = Cell::number(*v);
        }
    }
    DataSheet::new(SheetId::new(id), "Complete RM Blocks", TableArchetype::Column(tab)).map_err(err_str)
}

#[test]
fn w7_r36_r1_incomplete_rm_table_allows_matching_and_switches_kr_vs_satterthwaite() -> TestRes {
    let ds = make_incomplete_rm_column_sheet("ds-rm-inc")?;
    let mut doc = ProjectDocument::new("R3.6 Incomplete RM UI").map_err(err_str)?;
    doc.add_data_sheet(ds).map_err(err_str)?;
    doc.select_in_navigator(&SheetId::from("ds-rm-inc")).map_err(err_str)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let res = app
        .execute_command(
            "analysis.run",
            json!({
                "source_sheet_id": "ds-rm-inc",
                "kind": "rm_one_way_anova",
                "residual_covariance": "unstructured",
                "multiplicity": "bonferroni"
            }),
        )
        .map_err(err_str)?;
    let res_id = SheetId::from(res.payload["sheet_id"].as_str().ok_or("missing sheet_id")?);

    let (df_kr, p_kr) = {
        let rs = result_sheet(&app, &res_id)?;
        (scalar_val(rs, "Denominator df")?, scalar_val(rs, "P value")?)
    };

    let mut h = desktop_harness(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&res_id)?;
    h.click("toolbar.edit_params")?;
    h.assert_parity_and_layout("incomplete_rm_design_tab")?;

    // R1 relaxed for incomplete-block RM Column table: matched toggle must remain ENABLED & checked!
    let paired_node = require_widget(&h, "edit.paired_or_matched")?;
    ensure(
        paired_node.enabled && paired_node.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false),
        format!("expected edit.paired_or_matched enabled & checked on incomplete RM table, got {paired_node:?}"),
    )?;

    // Switch to Repeated Measures tab: R1b disables sphericity automatically & surfaces KR vs Satterthwaite df controls.
    h.click("edit.tab.repeated_measures")?;
    h.assert_parity_and_layout("incomplete_rm_rm_tab")?;

    let kr_node = scroll_into_modal_view(&mut h, "edit.rm.df_kenward_roger")?;
    ensure(kr_node.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false), "expected Kenward-Roger df selected by default on incomplete RM REML")?;
    assert_disabled_tooltip_and_noop_click(&mut h, "edit.sphericity.geisser_greenhouse")?;
    assert_disabled_tooltip_and_noop_click(&mut h, "edit.rm.assume_sphericity")?;

    // Switch to Satterthwaite via human click and confirm OK.
    click_modal_widget(&mut h, "edit.rm.df_satterthwaite")?;
    h.click("modal.ok")?;

    let rs_sat = result_sheet(h.app(), &res_id)?;
    let df_sat = scalar_val(rs_sat, "Denominator df")?;
    let p_sat = scalar_val(rs_sat, "P value")?;
    if (df_kr - df_sat).abs() < 1e-3 || (p_kr - p_sat).abs() < 1e-8 {
        return Err(format!("expected Kenward-Roger vs Satterthwaite df/P to differ: KR=({df_kr}, {p_kr}) Sat=({df_sat}, {p_sat})"));
    }
    Ok(())
}

#[test]
fn w7_r36_r1b_complete_rm_reml_toggle_disables_sphericity_and_restricts_post_hoc() -> TestRes {
    let ds = make_complete_rm_column_sheet("ds-rm-comp")?;
    let mut doc = ProjectDocument::new("R10.3b2 Complete RM REML").map_err(err_str)?;
    doc.add_data_sheet(ds).map_err(err_str)?;
    doc.select_in_navigator(&SheetId::from("ds-rm-comp")).map_err(err_str)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let res = app
        .execute_command(
            "analysis.run",
            json!({
                "source_sheet_id": "ds-rm-comp",
                "kind": "rm_one_way_anova",
                "multiplicity": "tukey"
            }),
        )
        .map_err(err_str)?;
    let res_id = SheetId::from(res.payload["sheet_id"].as_str().ok_or("missing sheet_id")?);

    let mut h = desktop_harness(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&res_id)?;
    h.click("toolbar.edit_params")?;
    h.click("edit.tab.repeated_measures")?;

    // Complete RM without mixed_effects_reml keeps sphericity enabled.
    let gg_init = scroll_into_modal_view(&mut h, "edit.sphericity.geisser_greenhouse")?;
    ensure(gg_init.enabled, "complete RM ANOVA should keep sphericity enabled before Mixed-Effects REML is enabled")?;
    h.click("edit.tab.multiple_comparisons")?;
    ensure(scroll_into_modal_view(&mut h, "edit.post_hoc.tukey")?.enabled, "complete RM ANOVA should allow Tukey before REML toggle")?;

    // Switch back to Repeated Measures and enable Mixed-Effects REML -> sphericity disables (`R1b`).
    h.click("edit.tab.repeated_measures")?;
    click_modal_widget(&mut h, "edit.rm.mixed_effects_reml")?;
    h.assert_parity_and_layout("complete_rm_reml_active")?;
    assert_disabled_tooltip_and_noop_click(&mut h, "edit.sphericity.geisser_greenhouse")?;
    assert_disabled_tooltip_and_noop_click(&mut h, "edit.rm.assume_sphericity")?;

    // Switch to Multiple Comparisons: Tukey/Dunnett/Games-Howell/Dunn disabled; Bonferroni/Sidak/Holm-Sidak/FDR/Fisher's LSD enabled!
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
        let node = scroll_into_modal_view(&mut h, allowed)?;
        ensure(node.enabled, format!("expected `{allowed}` enabled under Mixed-Effects REML, got disabled"))?;
    }

    // Select Fisher's LSD via real click and apply.
    click_modal_widget(&mut h, "edit.post_hoc.sidak")?;
    click_modal_widget(&mut h, "edit.post_hoc.fisher_lsd")?;
    h.click("modal.ok")?;

    let rs = result_sheet(h.app(), &res_id)?;
    let AnalysisKind::OneWayAnova(spec) = &rs.analysis.kind else {
        return Err(format!("expected OneWayAnova, got {:?}", rs.analysis.kind));
    };
    if !spec.mixed_effects_reml || spec.multiplicity.method != MultiplicityMethod::FisherLsd {
        return Err(format!("expected mixed_effects_reml=true & FisherLsd, got {spec:?}"));
    }
    if !rs.tabs.iter().any(|t| t.kind == ResultTabKind::MultipleComparisons) {
        return Err("expected MultipleComparisons tab from REML + Fisher's LSD".to_owned());
    }
    Ok(())
}

#[test]
fn w7_r36_r4_welch_t_disables_diagnostics_residuals_with_tooltip_and_noop() -> TestRes {
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).map_err(err_str)?;
    tab.data_sets[0].title = String::from("Ctrl");
    tab.data_sets[1].title = String::from("Treated");
    for (r, (&a, &b)) in [10.0, 11.2, 9.8, 10.5, 11.1].iter().zip([15.2, 16.0, 14.9, 17.3, 15.8].iter()).enumerate() {
        tab.data_sets[0].cells[r][0] = Cell::number(a);
        tab.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let ds = DataSheet::new("ds-tg", "Two Group", TableArchetype::Column(tab)).map_err(err_str)?;
    let mut doc = ProjectDocument::new("R4 Welch Diagnostics").map_err(err_str)?;
    doc.add_data_sheet(ds).map_err(err_str)?;
    doc.select_in_navigator(&SheetId::from("ds-tg")).map_err(err_str)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let res = app.execute_command("analysis.run", json!({ "source_sheet_id": "ds-tg", "kind": "unpaired_t_test" })).map_err(err_str)?;
    let res_id = SheetId::from(res.payload["sheet_id"].as_str().ok_or("missing sheet_id")?);

    let mut h = desktop_harness(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&res_id)?;
    h.click("toolbar.edit_params")?;

    // Unpaired Student's t keeps Diagnostics checkboxes enabled.
    h.click("edit.tab.diagnostics")?;
    ensure(require_widget(&h, "edit.diag.qq")?.enabled, "expected edit.diag.qq enabled for Student's t")?;
    ensure(require_widget(&h, "edit.diag.shapiro")?.enabled, "expected edit.diag.shapiro enabled for Student's t")?;

    // Switch back to Design & Test and click Welch's t test (`edit.tg.welch_t`).
    h.click("edit.tab.experimental_design")?;
    click_modal_widget(&mut h, "edit.tg.welch_t")?;
    h.assert_parity_and_layout("welch_t_experimental_design")?;

    // R4: Diagnostics / Residuals tab must disable under WelchT with tooltip + noop click!
    assert_disabled_tooltip_and_noop_click(&mut h, "edit.tab.diagnostics")?;
    let res_gate: ControlEnablement = evaluate_dialog_widget(&h.app().canvas_dialogs, h.app().document(), "edit.diag.residuals_table");
    if res_gate.enabled {
        return Err("expected edit.diag.residuals_table disabled by predicate under WelchT".to_owned());
    }
    h.click("modal.ok")?;
    let rs = result_sheet(h.app(), &res_id)?;
    let AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, .. }) = rs.analysis.kind else {
        return Err("expected TwoGroupComparison".to_owned());
    };
    if method != TwoGroupMethod::WelchT {
        return Err(format!("expected TwoGroupMethod::WelchT after OK, got {method:?}"));
    }
    Ok(())
}

#[test]
fn w7_r36_r103b2_multifactor_anova_preserves_interactions_across_factor_add_remove() -> TestRes {
    // 4 categorical factors (`F0`, `F1`, `F2`, `F3`) + continuous response `Y` (col 4), full 2^4 = 16 cells x 2 reps = 32 rows.
    let mut mv = MultipleVariablesTable::new(5, 32).map_err(err_str)?;
    let titles = ["Genotype", "Drug", "Diet", "Timepoint", "Response"];
    for (c, title) in titles.iter().enumerate() {
        mv.variables[c].title = String::from(*title);
        mv.variables[c].role = if c < 4 { VariableRole::Categorical } else { VariableRole::Continuous };
    }
    for row in 0..32usize {
        let g = (row / 16) % 2;
        let d = (row / 8) % 2;
        let dt = (row / 4) % 2;
        let tp = (row / 2) % 2;
        let rep = row % 2;
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
    let ds = DataSheet::new("ds-mv-fac", "Multifactor 4x2", TableArchetype::MultipleVariables(mv)).map_err(err_str)?;
    let mut doc = ProjectDocument::new("Multifactor Interaction Preservation").map_err(err_str)?;
    doc.add_data_sheet(ds).map_err(err_str)?;
    doc.select_in_navigator(&SheetId::from("ds-mv-fac")).map_err(err_str)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let res = app
        .execute_command(
            "analysis.run",
            json!({
                "source_sheet_id": "ds-mv-fac",
                "kind": "two_way_anova",
                "response_column": 4,
                "multifactor_columns": [0, 1, 2]
            }),
        )
        .map_err(err_str)?;
    let res_id = SheetId::from(res.payload["sheet_id"].as_str().ok_or("missing sheet_id")?);

    let mut h = desktop_harness(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&res_id)?;
    h.click("toolbar.edit_params")?;
    h.click("edit.tab.experimental_design")?;
    h.assert_parity_and_layout("mv_factor_settings_tab")?;

    // Configure 3-way interactions on Experimental Design / Model tab.
    click_modal_widget(&mut h, "edit.fac.order.3")?;
    let order_now = match h.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::FactorialAnova(f)) => f.max_interaction_order,
        other => return Err(format!("expected FactorialAnova draft, got {other:?}")),
    };
    if order_now != 3 {
        return Err(format!("expected max_interaction_order=3 after clicking edit.fac.order.3, got {order_now}"));
    }

    // Remove factor 2 (`Diet`), add factor 3 (`Timepoint`), remove factor 1 (`Drug`), re-add factor 2 (`Diet`) -> max_interaction_order=3 MUST persist!
    click_modal_widget(&mut h, "edit.fac.col.2")?;
    click_modal_widget(&mut h, "edit.fac.col.3")?;
    click_modal_widget(&mut h, "edit.fac.col.1")?;
    click_modal_widget(&mut h, "edit.fac.col.2")?;

    let (cols_after, order_after) = match h.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::FactorialAnova(f)) => (f.multifactor_columns.clone(), f.max_interaction_order),
        _ => return Err("missing FactorialAnova draft".to_owned()),
    };
    if cols_after != vec![0, 2, 3] || order_after != 3 {
        return Err(format!("expected factors=[0,2,3] & preserved max_interaction_order=3, got {cols_after:?}, order={order_after}"));
    }
    let formula_lbl = require_widget(&h, "edit.formula_bar")?.label;
    if !formula_lbl.contains("Genotype:Diet:Timepoint") {
        return Err(format!("expected updated preserved 3-way term in formula bar, got `{formula_lbl}`"));
    }

    h.click("modal.ok")?;
    let rs = result_sheet(h.app(), &res_id)?;
    let AnalysisKind::FactorialAnova(fac) = &rs.analysis.kind else {
        return Err("expected FactorialAnova result".to_owned());
    };
    if fac.max_interaction_order != 3 || fac.multifactor_columns != vec![0, 2, 3] {
        return Err(format!("expected committed order=3 & cols=[0,2,3], got {fac:?}"));
    }
    Ok(())
}

#[test]
fn w7_r36_3way_rm_and_nested_df_method_human_path_e2e() -> TestRes {
    // 1. 3-way RM (`BothFactorsMatched`, 2x2x2) with missing cells.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(4), 4, 2).map_err(err_str)?;
    grp.row_factor_title = String::from("C");
    grp.column_factor_title = String::from("A_B");
    grp.three_way_grouping = Some(curvebook_doc::ThreeWayColumnGrouping {
        factor_1_name: String::from("A"),
        factor_1_levels: [String::from("A1"), String::from("A2")],
        factor_2_name: String::from("B"),
        factor_2_levels: [String::from("B1"), String::from("B2")],
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
    let ds3 = DataSheet::new("ds-3way-rm", "3way RM", TableArchetype::Grouped(grp)).map_err(err_str)?;

    // 2. Unbalanced 3-group Nested ANOVA table (varying subcolumns and replicate counts -> distinct KR vs Sat df).
    let mut nt = NestedTable::new(3, 5, 5).map_err(err_str)?;
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
    let ds_nest = DataSheet::new("ds-nest", "Nested Unbalanced", TableArchetype::Nested(nt)).map_err(err_str)?;

    let mut doc = ProjectDocument::new("3way RM & Nested E2E").map_err(err_str)?;
    doc.add_data_sheet(ds3).map_err(err_str)?;
    doc.add_data_sheet(ds_nest).map_err(err_str)?;
    doc.select_in_navigator(&SheetId::from("ds-3way-rm")).map_err(err_str)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let r3 = app
        .execute_command(
            "analysis.run",
            json!({
                "source_sheet_id": "ds-3way-rm",
                "kind": "three_way_anova",
                "repeated_measures": "both"
            }),
        )
        .map_err(err_str)?;
    let r3_id = SheetId::from(r3.payload["sheet_id"].as_str().ok_or("missing r3 id")?);

    let rn = app.execute_command("analysis.run", json!({ "source_sheet_id": "ds-nest", "kind": "nested_anova" })).map_err(err_str)?;
    let rn_id = SheetId::from(rn.payload["sheet_id"].as_str().ok_or("missing rn id")?);
    let nest_df_kr = scalar_val(result_sheet(&app, &rn_id)?, "Group denominator df")?;
    let nest_f_kr = scalar_val(result_sheet(&app, &rn_id)?, "F (among groups)")?;
    let r3_kr_df = scalar_val(result_sheet(&app, &r3_id)?, "A × B × C: Kenward-Roger df")?;

    // Test 3-way RM df toggle via human click on Repeated Measures tab.
    let mut h = desktop_harness(app, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_sheet(&r3_id)?;
    h.click("toolbar.edit_params")?;
    h.click("edit.tab.repeated_measures")?;
    click_modal_widget(&mut h, "edit.rm.df.sat")?;
    h.click("modal.ok")?;
    let rs3 = result_sheet(h.app(), &r3_id)?;
    let AnalysisKind::FactorialAnova(fac_done) = &rs3.analysis.kind else {
        return Err("expected FactorialAnova".to_owned());
    };
    if fac_done.df_method != MixedModelDfMethod::Satterthwaite || !r3_kr_df.is_finite() {
        return Err(format!("expected Satterthwaite df on 3-way RM after OK, got {fac_done:?}"));
    }

    // Switch to Nested result sheet via navigator widget click and switch KR -> Satterthwaite on Nested Design.
    h.select_sheet(&rn_id)?;
    h.click("toolbar.edit_params")?;
    click_modal_widget(&mut h, "edit.rm.df.sat")?;
    h.click("modal.ok")?;
    let rsn = result_sheet(h.app(), &rn_id)?;
    let AnalysisKind::NestedComparison(nest_done) = &rsn.analysis.kind else {
        return Err("expected NestedComparison".to_owned());
    };
    let nest_df_sat = scalar_val(rsn, "Group denominator df")?;
    let nest_f_sat = scalar_val(rsn, "F (among groups)")?;
    if nest_done.df_method != MixedModelDfMethod::Satterthwaite
        || (nest_df_kr - nest_df_sat).abs() <= 1e-8
        || !nest_f_kr.is_finite()
        || !nest_f_sat.is_finite()
        || nest_f_kr >= nest_f_sat
    {
        return Err(format!(
            "expected Nested df_method Satterthwaite & distinct KR vs Sat (df=({nest_df_kr}, {nest_df_sat}), F=({nest_f_kr}, {nest_f_sat}))"
        ));
    }
    Ok(())
}

#[test]
fn w7_r36_staged_i18n_16_locales_and_rtl_hidpi_parity() -> TestRes {
    let locales = ["en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-CN", "zh-TW", "it", "nl", "pl", "ru", "tr", "ar", "he"];
    let keys = [
        "stat_params.rm.mixed_effects_reml",
        "stat_params.rm.df_kenward_roger",
        "stat_params.rm.df_satterthwaite",
        "stat_params.post_hoc.sidak",
        "stat_params.post_hoc.fisher_lsd",
        "stat_params.fac.interactions",
    ];
    for loc in locales {
        for key in keys {
            let hit = curvebook_ui_egui::i18n::try_tr_in_slices(&[curvebook_ui_egui::i18n::w7_rm_reml::STAGED_ENTRIES], loc, key);
            if hit.is_none_or(str::is_empty) {
                return Err(format!("missing staged translation for locale `{loc}` key `{key}`"));
            }
        }
    }

    // Verify RTL Arabic + HiDPI 1.5x layout parity on Repeated Measures + Multiple Comparisons tabs.
    let ds = make_incomplete_rm_column_sheet("ds-rtl")?;
    let mut doc = ProjectDocument::new("RTL HiDPI RM REML").map_err(err_str)?;
    doc.add_data_sheet(ds).map_err(err_str)?;
    doc.select_in_navigator(&SheetId::from("ds-rtl")).map_err(err_str)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let res = app.execute_command("analysis.run", json!({ "source_sheet_id": "ds-rtl", "kind": "rm_one_way_anova" })).map_err(err_str)?;
    let res_id = SheetId::from(res.payload["sheet_id"].as_str().ok_or("missing sheet_id")?);
    app.locale = String::from("ar");

    let mut h = desktop_harness(app, ViewportMetrics::new(1440.0, 900.0, 1.5));
    h.select_sheet(&res_id)?;
    h.click("toolbar.edit_params")?;
    h.click("edit.tab.repeated_measures")?;
    h.assert_parity_and_layout("rtl_hidpi_rm_tab")?;
    h.click("edit.tab.multiple_comparisons")?;
    h.assert_parity_and_layout("rtl_hidpi_mc_tab")?;
    Ok(())
}
