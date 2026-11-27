#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` five-stage `WorkflowRun` acceptance tests (`WF-07`–`WF-12`).
//!
//! Group Comparisons: Unpaired Student/Welch t-tests + R4 predicates (`WF-07`),
//! Paired t-test Equal-N check & Before-After analysis graph (`WF-08`),
//! Mann-Whitney U & Wilcoxon signed-rank with outlier rank invariance (`WF-09`),
//! One-Way ANOVA + Brown-Forsythe/Welch + Dunnett(R3) + Tukey + diagnostic banner & brackets (`WF-10`),
//! Kruskal-Wallis + Dunn's multiple comparisons (`WF-11`),
//! Two-Way ANOVA interaction + Type III + Grouped error bars & Stacked guard (`WF-12`).

use std::collections::BTreeSet;

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, FactorialAnovaSpec, GraphKind, GraphSheet, GraphicalDataObject, GroupedArrangement,
    GroupedTable, MultiplicityMethod, MultiplicitySpec, OneWayAnovaSpec, OneWayMethod, PValueFormat, ProjectDocument, SheetId, SubcolumnMode,
    SumOfSquaresType, TableArchetype, TwoGroupMethod, TwoGroupSpec,
};
use curvebook_ui_egui::interaction::KeyModifiers;
use serde_json::json;

use crate::human_wf_common::TestRes;
use crate::human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};

fn build_column_sheet(id: &str, title: &str, hint: ColumnDesignHint, col_titles: &[&str], cols: &[&[Option<f64>]]) -> Result<DataSheet, String> {
    let max_rows = cols.iter().map(|c| c.len()).max().unwrap_or(1).max(1);
    let mut tbl = ColumnTable::new(hint, cols.len(), max_rows).map_err(|e| e.to_string())?;
    for (c_idx, (&ctitle, values)) in col_titles.iter().zip(cols.iter()).enumerate() {
        tbl.data_sets[c_idx].title = ctitle.to_owned();
        for (r_idx, val_opt) in values.iter().enumerate() {
            tbl.data_sets[c_idx].cells[r_idx][0] = match val_opt {
                Some(v) => Cell::number(*v),
                None => Cell::empty(),
            };
        }
    }
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    DataSheet::new(sid, title, TableArchetype::Column(tbl)).map_err(|e| e.to_string())
}

fn build_grouped_sheet(
    id: &str,
    title: &str,
    col_titles: &[&str],
    row_titles: &[&str],
    replicates: usize,
    rows_flat: &[&[f64]],
) -> Result<DataSheet, String> {
    let mut tbl = GroupedTable::new(SubcolumnMode::RawReplicates(replicates), col_titles.len(), rows_flat.len()).map_err(|e| e.to_string())?;
    tbl.row_factor_title = String::from("Row Factor");
    tbl.column_factor_title = String::from("Column Factor");
    for (r_idx, rtitle) in row_titles.iter().enumerate() {
        tbl.row_titles[r_idx] = (*rtitle).to_owned();
    }
    for (c_idx, ctitle) in col_titles.iter().enumerate() {
        tbl.data_sets[c_idx].title = (*ctitle).to_owned();
    }
    for (r_idx, row_vals) in rows_flat.iter().enumerate() {
        for c_idx in 0..col_titles.len() {
            for sub in 0..replicates {
                let v = row_vals[c_idx * replicates + sub];
                tbl.data_sets[c_idx].cells[r_idx][sub] = Cell::number(v);
            }
        }
    }
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    DataSheet::new(sid, title, TableArchetype::Grouped(tbl)).map_err(|e| e.to_string())
}

fn graph_id_set(doc: &ProjectDocument) -> BTreeSet<SheetId> {
    doc.graph_sheets.iter().map(|g| g.id.clone()).collect()
}

fn diff_new_graph_for_source(doc: &ProjectDocument, before: &BTreeSet<SheetId>, source_id: &SheetId) -> Result<SheetId, String> {
    let mut matches: Vec<&GraphSheet> =
        doc.graph_sheets.iter().filter(|g| !before.contains(&g.id) && &g.spec.primary_source_sheet_id == source_id).collect();
    if matches.len() != 1 {
        return Err(format!("expected exactly 1 newly created GraphSheet for source `{}`, found {}", source_id.as_str(), matches.len()));
    }
    Ok(matches.remove(0).id.clone())
}

fn find_graph_for_source(doc: &ProjectDocument, source_id: &SheetId) -> Result<SheetId, String> {
    doc.graph_sheets
        .iter()
        .find(|g| &g.spec.primary_source_sheet_id == source_id)
        .map(|g| g.id.clone())
        .ok_or_else(|| format!("missing GraphSheet bound to `{}`", source_id.as_str()))
}

fn widget_enabled(run: &WorkflowRun, wid: &str) -> Result<bool, String> {
    let meta = run.frame_meta();
    meta.find_by_widget_id(wid).map(|n| n.enabled).ok_or_else(|| format!("missing inspected widget `{wid}`"))
}

fn widget_disabled_reason(run: &WorkflowRun, wid: &str) -> Result<String, String> {
    let meta = run.frame_meta();
    meta.find_by_widget_id(wid).map(|n| n.disabled_reason.clone().unwrap_or_default()).ok_or_else(|| format!("missing inspected widget `{wid}`"))
}

// ---------------------------------------------------------------------------
// WF-07: Unpaired Student t + Welch t + R4 predicates + linked Column Graph
// ---------------------------------------------------------------------------

#[test]
fn wf07_human_unpaired_student_and_welch_t_test_with_predicates() -> TestRes {
    let mut run = WorkflowRun::start("WF-07");
    let graphs_before = graph_id_set(run.document());
    let col_id = run.create_table_via_welcome("welcome.arch.column", "Unpaired Dose Cohort", &["welcome.stepper.cols.dec"])?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &col_id)?;

    run.enter_column_title(0, "Control")?;
    run.enter_column_title(1, "Treatment")?;
    run.click("grid.cell.0.0.0")?;
    let tsv = "10.0\t14.0\n11.2\t18.0\n9.8\t22.0\n10.5\t15.0\n11.0\t25.0\n10.1\t19.0\n\t21.0";
    run.paste_clipboard(tsv)?;

    let res_id = run.run_analyze_entry("t test", "analyze.entry.stats.two_group")?;

    // Check R4 predicates when unpaired, then configure non-default Welch t + 99% CI + 3-digit custom P-value.
    run.open_cmd_t()?;
    if widget_enabled(&run, "edit.two_group.graph_differences")? {
        return Err(String::from("R4: edit.two_group.graph_differences must be disabled on Unpaired t-test"));
    }
    if !widget_enabled(&run, "edit.two_group.welch")? {
        return Err(String::from("R4: edit.two_group.welch must be enabled on Unpaired t-test"));
    }
    run.click("edit.tab.options")?;
    run.click("edit.conf.99")?;
    run.click("edit.pval.custom")?;
    if !widget_enabled(&run, "edit.pvalue.digits.dec")? {
        return Err(String::from("R9: custom p-value precision stepper must unlock under Custom PValueFormat"));
    }
    run.click("edit.pvalue.digits.dec")?;
    run.click("edit.tab.experimental_design")?;
    run.click("edit.two_group.welch")?;
    run.commit_modal_ok()?;

    // Verify Welch t-test configuration, Student -> Welch parity, and linked scatter-bar graph + bracket.
    let rs = run.document().result_sheet(&res_id).ok_or_else(|| String::from("missing WF-07 ResultSheet"))?;
    if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::WelchT) {
        return Err(format!("expected WelchT after GUI toggle, got {:?}", rs.analysis.kind));
    }
    if (rs.analysis.confidence_level.percent() - 99.0).abs() > 1e-6 {
        return Err(format!("expected 99% CI, got {:?}", rs.analysis.confidence_level));
    }
    if !matches!(rs.analysis.p_value_format, PValueFormat::Custom { decimals: 3, .. }) {
        return Err(format!("expected Custom(decimals=3), got {:?}", rs.analysis.p_value_format));
    }

    run.select_sheet(&graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.col.bar_replicates")?;
    run.commit_modal_ok()?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.pairwise.open")?;
    run.click("pairwise.auto_add")?;
    run.commit_modal_ok()?;

    let ref_col_data: [&[Option<f64>]; 2] = [
        &[Some(10.0), Some(11.2), Some(9.8), Some(10.5), Some(11.0), Some(10.1), None],
        &[Some(14.0), Some(18.0), Some(22.0), Some(15.0), Some(25.0), Some(19.0), Some(21.0)],
    ];
    let ref_sheet =
        build_column_sheet("ref-wf07", "Unpaired Dose Cohort", ColumnDesignHint::UnpairedReplicates, &["Control", "Treatment"], &ref_col_data)?;
    let welch_kind =
        serde_json::to_value(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }))
            .map_err(|e| e.to_string())?;
    let mut ref_student_sheet = ref_sheet.clone();
    ref_student_sheet.id = SheetId::new("ref-wf07-stu");

    // Stage 4a: Student baseline reference check on clean-room dataset before Welch switch (verified independently).
    let run_ref_graph_id = graph_id.clone();
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute(
                "analysis.run",
                json!({
                    "source_sheet_id": "ref-wf07",
                    "sheet_id": "ref-res-wf07",
                    "kind": welch_kind,
                    "confidence_percent": 99.0,
                    "p_value_format": { "Custom": { "decimals": 3, "scientific_notation": false } }
                }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-wf07", "data_sheet_id": "ref-wf07", "kind": "column" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-g-wf07", "kind": "column", "option_id": "col.scatter_bar" }))
                .map_err(|e| e.to_string())?;
            sess.execute(
                "graph.addComparisonBrackets",
                json!({ "sheet_id": "ref-g-wf07", "result_sheet_id": "ref-res-wf07", "significant_only": false }),
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-res-wf07"))
        .with_graph(run_ref_graph_id)
        .ref_graph_id(SheetId::new("ref-g-wf07"))
        .scalar("t", None, 1e-6, 1e-6)
        .scalar("PValue", None, 1e-9, 1e-9)
        .tab_cell(0, 1, 0, 1e-5, 1e-5)
        .tab_cell(0, 2, 0, 1e-5, 1e-5)
        .check_graph(|gui_gs, ref_gs| {
            if gui_gs.spec.kind != GraphKind::Column || gui_gs.spec.comparisons.len() != ref_gs.spec.comparisons.len() {
                return Err(format!(
                    "expected Column graph with {} brackets, got {:?} with {}",
                    ref_gs.spec.comparisons.len(),
                    gui_gs.spec.kind,
                    gui_gs.spec.comparisons.len()
                ));
            }
            if gui_gs.spec.comparisons.is_empty() {
                return Err(String::from("expected pairwise bracket on WF-07 graph"));
            }
            Ok(())
        }),
    )?;

    // Also verify Student t baseline directly against Student oracle on a sibling table created cleanly? No: switch Welch -> Student -> Welch
    // or verify Welch live_edit + Student math parity on Student toggle!
    run.select_sheet(&res_id)?;
    run.open_cmd_t()?;
    run.click("edit.tg.unpaired_t")?;
    run.commit_modal_ok()?;
    let stu_kind =
        serde_json::to_value(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }))
            .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute(
                "analysis.run",
                json!({
                    "source_sheet_id": "ref-wf07-stu",
                    "sheet_id": "ref-res-stu",
                    "kind": stu_kind,
                    "confidence_percent": 99.0
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_student_sheet)?
        .ref_result_id(SheetId::new("ref-res-stu"))
        .scalar("t", None, 1e-6, 1e-6)
        .scalar("PValue", None, 1e-9, 1e-9),
    )?;
    // Switch back to Welch for Stage 5 live edit so non-default Welch stays active.
    run.select_sheet(&res_id)?;
    run.open_cmd_t()?;
    run.click("edit.two_group.welch")?;
    run.commit_modal_ok()?;

    // Stage 5: Live grid cell edit AND Cmd+E exclusion both update Welch ResultSheet + Graph SVG reactively.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(col_id.clone(), 0, 1, 0, "32.0"),
        vec![
            LiveEditExpectation::scalar_changed(res_id.clone(), "t", None, 0.1),
            LiveEditExpectation::scalar_changed(res_id.clone(), "PValue", None, 1e-8),
            LiveEditExpectation::graph_svg_changed(graph_id.clone()),
        ],
    )?;
    let _ = res_id;
    let _ = graph_id;
    run.live_edit(
        LiveEditTarget::gui(move |r| {
            r.select_sheet(&col_id)?;
            r.click("grid.cell.0.1.0")?;
            r.press_key("e", KeyModifiers { meta: true, ctrl: true, shift: false, alt: false })
        }),
        LiveEditExpectation::scalar_changed(res_id, "t", None, 0.1),
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-08: Paired t-test (Equal-N R1 predicate + dedicated Before-After graph)
// ---------------------------------------------------------------------------

#[test]
fn wf08_human_paired_t_test_equal_n_and_before_after_graph() -> TestRes {
    let mut run = WorkflowRun::start("WF-08");

    // Part A: Unequal-N Column table -> edit.paired_or_matched must be disabled (R1).
    let uneq_id = run.create_table_via_welcome("welcome.arch.column", "Unequal Columns", &["welcome.stepper.cols.dec"])?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("10\t20\n11\t21\n12\t\n13\t")?;
    let _uneq_res = run.run_analyze_entry("t test", "analyze.entry.stats.two_group")?;
    run.open_cmd_t()?;
    if widget_enabled(&run, "edit.paired_or_matched")? {
        return Err(String::from("R1: edit.paired_or_matched must be disabled on unequal-N Column sheet"));
    }
    let reason = widget_disabled_reason(&run, "edit.paired_or_matched")?;
    if reason.trim().is_empty() {
        return Err(String::from("R1: expected non-empty disabled_reason on unequal-N Column sheet"));
    }
    run.click("modal.cancel")?;
    let _ = uneq_id;

    // Part B: Equal-N paired table created WITHOUT Welcome auto-graph; create dedicated Before-After graph explicitly.
    let col_id = run.create_table_via_welcome("welcome.arch.column", "Paired Pre vs Post", &["welcome.toggle.auto_graph"])?;
    run.enter_column_title(0, "Baseline")?;
    run.enter_column_title(1, "After")?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("120\t110\n132\t121\n118\t109\n125\t116\n140\t129\n128\t118")?;

    let res_id = run.run_analyze_entry("t test", "analyze.entry.stats.two_group")?;
    run.open_cmd_t()?;
    if !widget_enabled(&run, "edit.paired_or_matched")? {
        return Err(String::from("R1: edit.paired_or_matched must be enabled on equal-N Column sheet"));
    }
    run.click("edit.paired_or_matched")?;
    if !widget_enabled(&run, "edit.two_group.graph_differences")? || widget_enabled(&run, "edit.two_group.welch")? {
        return Err(String::from("R4: Paired mode must enable graph_differences and disable Welch"));
    }
    run.click("edit.two_group.graph_differences")?;
    run.commit_modal_ok()?;

    // Explicitly create the Before-After graph via Insert > New Graph... and disambiguate via diff + primary_source_sheet_id.
    let graphs_before = graph_id_set(run.document());
    run.open_menu_and_click("menu.insert", &[], "menu.ui.graph.create.open")?;
    let source_idx =
        run.document().data_sheets.iter().position(|d| d.id == col_id).ok_or_else(|| String::from("missing paired DataSheet in candidates"))?;
    run.click(&format!("create_graph.source.{source_idx}"))?;
    run.click("create_graph.kind.column")?;
    run.commit_modal_ok()?;
    let ba_graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &col_id)?;
    run.select_sheet(&ba_graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.col.before_after")?;
    run.commit_modal_ok()?;

    let rs = run.document().result_sheet(&res_id).ok_or_else(|| String::from("missing WF-08 ResultSheet"))?;
    if !matches!(rs.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::PairedT) {
        return Err(format!("expected PairedT after GUI switch, got {:?}", rs.analysis.kind));
    }

    let ref_data: [&[Option<f64>]; 2] = [
        &[Some(120.0), Some(132.0), Some(118.0), Some(125.0), Some(140.0), Some(128.0)],
        &[Some(110.0), Some(121.0), Some(109.0), Some(116.0), Some(129.0), Some(118.0)],
    ];
    let ref_sheet =
        build_column_sheet("ref-wf08", "Paired Pre vs Post", ColumnDesignHint::PairedRepeatedMeasures, &["Baseline", "After"], &ref_data)?;
    let paired_kind =
        serde_json::to_value(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }))
            .map_err(|e| e.to_string())?;

    let gui_col_id = col_id.clone();
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf08", "sheet_id": "ref-res-wf08", "kind": paired_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-ba-wf08", "data_sheet_id": "ref-wf08", "kind": "column" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-ba-wf08", "kind": "column", "option_id": "col.before_after" }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-res-wf08"))
        .with_graph(ba_graph_id.clone())
        .ref_graph_id(SheetId::new("ref-ba-wf08"))
        .scalar("t", None, 1e-6, 1e-6)
        .scalar("PValue", None, 1e-9, 1e-9)
        .tab_cell(0, 1, 0, 1e-6, 1e-6)
        .check_graph(move |gui_gs, _ref_gs| {
            if gui_gs.spec.primary_source_sheet_id != gui_col_id {
                return Err(format!("Before-After graph source mismatch: {}", gui_gs.spec.primary_source_sheet_id.as_str()));
            }
            if !gui_gs.spec.series.iter().all(|s| matches!(s.gdo, GraphicalDataObject::BeforeAfterLines { .. })) {
                return Err(format!("expected BeforeAfterLines on all series, got {:?}", gui_gs.spec.series));
            }
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(col_id, 0, 1, 0, "95.0"),
        vec![
            LiveEditExpectation::scalar_changed(res_id.clone(), "t", None, 0.1),
            LiveEditExpectation::scalar_changed(res_id, "PValue", None, 1e-6),
            LiveEditExpectation::graph_svg_changed(ba_graph_id),
        ],
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-09: Non-parametric Mann-Whitney U & Wilcoxon Signed-Rank + Rank Invariance
// ---------------------------------------------------------------------------

#[test]
fn wf09_human_mann_whitney_and_wilcoxon_rank_invariance() -> TestRes {
    let mut run = WorkflowRun::start("WF-09");

    // Part A: Mann-Whitney U on cleanly separated cohorts.
    let mw_col = run.create_table_via_welcome("welcome.arch.column", "Skewed MW Cohort", &["welcome.stepper.cols.dec"])?;
    run.enter_column_title(0, "GroupA")?;
    run.enter_column_title(1, "GroupB")?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("2.0\t10.0\n3.0\t12.0\n4.0\t14.0\n5.0\t16.0\n6.0\t18.0\n7.0\t20.0")?;
    let mw_graph = find_graph_for_source(run.document(), &mw_col)?;

    let mw_res = run.run_analyze_entry("t test", "analyze.entry.stats.two_group")?;
    run.open_cmd_t()?;
    run.click("edit.assume_gaussian")?;
    if !widget_enabled(&run, "edit.two_group.hodges_lehmann")? || !widget_enabled(&run, "edit.two_group.graph_ranks")? {
        return Err(String::from("expected Hodges-Lehmann and Graph Ranks enabled in non-Gaussian mode"));
    }
    if widget_enabled(&run, "edit.two_group.welch")? {
        return Err(String::from("expected Welch disabled in non-Gaussian mode"));
    }
    run.click("edit.two_group.graph_ranks")?;
    run.commit_modal_ok()?;

    let ref_mw_data: [&[Option<f64>]; 2] = [
        &[Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(6.0), Some(7.0)],
        &[Some(10.0), Some(12.0), Some(14.0), Some(16.0), Some(18.0), Some(20.0)],
    ];
    let ref_mw = build_column_sheet("ref-mw", "Skewed MW Cohort", ColumnDesignHint::UnpairedReplicates, &["GroupA", "GroupB"], &ref_mw_data)?;
    let mw_kind =
        serde_json::to_value(AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..TwoGroupSpec::default() }))
            .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(mw_res.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-mw", "sheet_id": "ref-mw-res", "kind": mw_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mw)?
        .ref_result_id(SheetId::new("ref-mw-res"))
        .scalar("MannWhitneyU", None, 1e-9, 1e-9)
        .scalar("PValue", None, 1e-9, 1e-9),
    )?;

    // Part B: Wilcoxon matched-pairs signed-rank test.
    let wil_col = run.create_table_via_welcome("welcome.arch.column", "Wilcoxon Matched", &[])?;
    run.enter_column_title(0, "Pre")?;
    run.enter_column_title(1, "Post")?;
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("10.0\t8.0\n15.0\t10.0\n22.0\t12.0\n30.0\t20.0\n45.0\t25.0\n18.0\t19.0\n60.0\t30.0")?;

    let wil_res = run.run_analyze_entry("t test", "analyze.entry.stats.two_group")?;
    run.open_cmd_t()?;
    run.click("edit.assume_gaussian")?;
    run.click("edit.paired_or_matched")?;
    run.commit_modal_ok()?;

    let ref_wil_data: [&[Option<f64>]; 2] = [
        &[Some(10.0), Some(15.0), Some(22.0), Some(30.0), Some(45.0), Some(18.0), Some(60.0)],
        &[Some(8.0), Some(10.0), Some(12.0), Some(20.0), Some(25.0), Some(19.0), Some(30.0)],
    ];
    let ref_wil = build_column_sheet("ref-wil", "Wilcoxon Matched", ColumnDesignHint::PairedRepeatedMeasures, &["Pre", "Post"], &ref_wil_data)?;
    let wil_kind = serde_json::to_value(AnalysisKind::TwoGroupComparison(TwoGroupSpec {
        method: TwoGroupMethod::WilcoxonSignedRank,
        ..TwoGroupSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(wil_res.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wil", "sheet_id": "ref-wil-res", "kind": wil_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_wil)?
        .ref_result_id(SheetId::new("ref-wil-res"))
        .scalar("PValue", None, 1e-9, 1e-9),
    )?;

    // Stage 5a: Monotone outlier inflation (20.0 -> 20000.0) preserves rank statistics (scalar_invariant)!
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mw_col.clone(), 5, 1, 0, "20000.0"),
        vec![
            LiveEditExpectation::scalar_invariant(mw_res.clone(), "MannWhitneyU", None, 1e-12, 1e-12),
            LiveEditExpectation::scalar_invariant(mw_res.clone(), "PValue", None, 1e-12, 1e-12),
            LiveEditExpectation::graph_svg_changed(mw_graph),
        ],
    )?;

    // Stage 5b: Rank-crossing edit (10.0 -> 1.0) changes Mann-Whitney U and PValue immediately.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mw_col, 0, 1, 0, "1.0"),
        vec![
            LiveEditExpectation::scalar_changed(mw_res.clone(), "MannWhitneyU", None, 0.5),
            LiveEditExpectation::scalar_changed(mw_res, "PValue", None, 1e-6),
        ],
    )?;
    let _ = wil_col;
    let _ = wil_res;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-10: One-Way ANOVA + Welch/Brown-Forsythe + Dunnett(R3) + Tukey + Brackets
// ---------------------------------------------------------------------------

#[test]
fn wf10_human_one_way_anova_welch_dunnett_tukey_and_brackets() -> TestRes {
    let mut run = WorkflowRun::start("WF-10");
    let graphs_before = graph_id_set(run.document());
    let col_id = run.create_table_via_welcome("welcome.arch.column", "Four Dose Cohort", &["welcome.stepper.cols.inc"])?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &col_id)?;

    for (idx, name) in ["Vehicle", "Low", "Mid", "High"].iter().enumerate() {
        run.enter_column_title(idx, name)?;
    }
    run.click("grid.cell.0.0.0")?;
    let tsv = "10.0\t12.0\t20.0\t25.0\n10.2\t12.5\t21.0\t45.0\n9.9\t11.8\t19.5\t65.0\n10.1\t12.2\t20.5\t35.0\n10.0\t12.1\t20.2\t55.0\n10.1\t12.3\t20.3\t40.0";
    run.paste_clipboard(tsv)?;

    let res_id = run.run_analyze_entry("One-way", "analyze.entry.stats.one_way_anova")?;
    run.select_sheet(&res_id)?;

    // Verify diagnostic banner surfaces on heteroscedastic cohort and `Inspect Details` switches non-mutatingly.
    let _ = run
        .frame_meta()
        .find_by_widget_id("result.diagnostic_banner")
        .ok_or_else(|| String::from("expected result.diagnostic_banner on heteroscedastic One-Way ANOVA"))?;
    let rs_before_diag = run.document().result_sheets.len();
    run.click("result.diagnostic_banner.inspect")?;
    if run.document().result_sheets.len() != rs_before_diag {
        return Err(String::from("diagnostic banner Inspect Details must not create sibling ResultSheets"));
    }

    // Rule R3: enable compare_against_control -> unlocks control_column -> test Dunnett vs col 1 ("Low") then cycle to col 0 ("Vehicle").
    run.open_cmd_t()?;
    if widget_enabled(&run, "edit.tab.repeated_measures")? {
        return Err(String::from("R1: RM tab must start disabled when unpaired"));
    }
    run.click("edit.tab.multiple_comparisons")?;
    if widget_enabled(&run, "edit.control_column")? {
        return Err(String::from("R3: edit.control_column must start disabled before compare_against_control"));
    }
    run.click("edit.compare_against_control")?;
    if !widget_enabled(&run, "edit.control_column")? {
        return Err(String::from("R3: edit.compare_against_control must unlock edit.control_column"));
    }
    // Cycle 0 -> 1 ("Low") non-default and commit to verify Dunnett(control=1), then configure Welch and Tukey.
    run.click("edit.control_column")?;
    run.commit_modal_ok()?;

    let rs_dun = run.document().result_sheet(&res_id).ok_or_else(|| String::from("missing Dunnett ResultSheet"))?;
    if !matches!(
        rs_dun.analysis.kind,
        AnalysisKind::OneWayAnova(ref s) if matches!(s.multiplicity.method, MultiplicityMethod::Dunnett { control_column: 1 })
    ) {
        return Err(format!("expected Dunnett(control_column=1), got {:?}", rs_dun.analysis.kind));
    }

    let ref_data: [&[Option<f64>]; 4] = [
        &[Some(10.0), Some(10.2), Some(9.9), Some(10.1), Some(10.0), Some(10.1)],
        &[Some(12.0), Some(12.5), Some(11.8), Some(12.2), Some(12.1), Some(12.3)],
        &[Some(20.0), Some(21.0), Some(19.5), Some(20.5), Some(20.2), Some(20.3)],
        &[Some(25.0), Some(45.0), Some(65.0), Some(35.0), Some(55.0), Some(40.0)],
    ];
    let ref_sheet_dun =
        build_column_sheet("ref-wf10-dun", "Four Dose Cohort", ColumnDesignHint::UnpairedReplicates, &["Vehicle", "Low", "Mid", "High"], &ref_data)?;
    let dun_kind = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 1 }, ..MultiplicitySpec::default() },
        ..OneWayAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf10-dun", "sheet_id": "ref-res-dun", "kind": dun_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet_dun)?
        .ref_result_id(SheetId::new("ref-res-dun"))
        .scalar("F", None, 1e-6, 1e-6)
        .scalar("P value", None, 1e-9, 1e-9),
    )?;

    // Switch via Cmd+T to Brown-Forsythe & Welch ANOVA (`edit.anova.welch`) and verify Welch W + Brown-Forsythe F*.
    run.select_sheet(&res_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.experimental_design")?;
    run.click("edit.anova.welch")?;
    run.commit_modal_ok()?;

    let rs_welch = run.document().result_sheet(&res_id).ok_or_else(|| String::from("missing Welch ANOVA ResultSheet"))?;
    if !matches!(rs_welch.analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.method == OneWayMethod::BrownForsytheAndWelch) {
        return Err(format!("expected BrownForsytheAndWelch, got {:?}", rs_welch.analysis.kind));
    }
    let ref_sheet_welch =
        build_column_sheet("ref-wf10-w", "Four Dose Cohort", ColumnDesignHint::UnpairedReplicates, &["Vehicle", "Low", "Mid", "High"], &ref_data)?;
    let welch_anova_kind = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        method: OneWayMethod::BrownForsytheAndWelch,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 1 }, ..MultiplicitySpec::default() },
        ..OneWayAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf10-w", "sheet_id": "ref-res-w", "kind": welch_anova_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet_welch)?
        .ref_result_id(SheetId::new("ref-res-w"))
        .scalar("Welch W", None, 1e-5, 1e-5)
        .scalar("Brown-Forsythe F*", None, 1e-5, 1e-5),
    )?;

    // Restore Ordinary ANOVA + all-pairwise Tukey HSD and add significance brackets on Column Graph.
    run.select_sheet(&res_id)?;
    run.open_cmd_t()?;
    run.click("edit.tab.experimental_design")?;
    run.click("edit.anova.ordinary")?;
    run.click("edit.tab.multiple_comparisons")?;
    run.click("edit.compare_against_control")?;
    run.click("edit.post_hoc.tukey")?;
    run.commit_modal_ok()?;

    run.select_sheet(&graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.col.bar_replicates")?;
    run.commit_modal_ok()?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.pairwise.open")?;
    run.click("pairwise.auto_add")?;
    run.commit_modal_ok()?;

    let ref_sheet_tuk =
        build_column_sheet("ref-wf10-tuk", "Four Dose Cohort", ColumnDesignHint::UnpairedReplicates, &["Vehicle", "Low", "Mid", "High"], &ref_data)?;
    let tuk_kind = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..MultiplicitySpec::default() },
        ..OneWayAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf10-tuk", "sheet_id": "ref-res-tuk", "kind": tuk_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-wf10", "data_sheet_id": "ref-wf10-tuk", "kind": "column" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-g-wf10", "kind": "column", "option_id": "col.scatter_bar" }))
                .map_err(|e| e.to_string())?;
            sess.execute(
                "graph.addComparisonBrackets",
                json!({ "sheet_id": "ref-g-wf10", "result_sheet_id": "ref-res-tuk", "significant_only": true }),
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet_tuk)?
        .ref_result_id(SheetId::new("ref-res-tuk"))
        .with_graph(graph_id.clone())
        .ref_graph_id(SheetId::new("ref-g-wf10"))
        .scalar("F", None, 1e-6, 1e-6)
        .scalar("P value", None, 1e-9, 1e-9)
        .check_graph(|gui_gs, _ref_gs| {
            if gui_gs.spec.comparisons.is_empty() {
                return Err(String::from("expected significant Tukey comparison brackets on WF-10 Column graph"));
            }
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(col_id, 0, 3, 0, "95.0"),
        vec![LiveEditExpectation::scalar_changed(res_id, "F", None, 0.1), LiveEditExpectation::graph_svg_changed(graph_id)],
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-11: Kruskal-Wallis + Dunn's Multiple Comparisons -> ResultSheet & Graph
// ---------------------------------------------------------------------------

#[test]
fn wf11_human_kruskal_wallis_and_dunns_multiple_comparisons() -> TestRes {
    let mut run = WorkflowRun::start("WF-11");
    let graphs_before = graph_id_set(run.document());
    let col_id = run.create_table_via_welcome("welcome.arch.column", "KW Rank Groups", &[])?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &col_id)?;

    for (idx, name) in ["Ctrl", "DrugA", "DrugB"].iter().enumerate() {
        run.enter_column_title(idx, name)?;
    }
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("1\t8\t20\n2\t9\t21\n3\t10\t22\n4\t11\t23\n5\t12\t24")?;

    let res_id = run.run_analyze_entry("One-way", "analyze.entry.stats.one_way_anova")?;
    run.open_cmd_t()?;
    run.click("edit.assume_gaussian")?;
    run.click("edit.tab.multiple_comparisons")?;
    if widget_enabled(&run, "edit.post_hoc.tukey")? || !widget_enabled(&run, "edit.post_hoc.dunn")? {
        return Err(String::from("R2: expected Tukey disabled and Dunn enabled when Assume Gaussian = No"));
    }
    run.click("edit.post_hoc.dunn")?;
    run.commit_modal_ok()?;

    let rs = run.document().result_sheet(&res_id).ok_or_else(|| String::from("missing WF-11 ResultSheet"))?;
    if !matches!(
        rs.analysis.kind,
        AnalysisKind::OneWayAnova(ref s)
            if s.method == OneWayMethod::KruskalWallis && s.multiplicity.method == MultiplicityMethod::DunnRankSum
    ) {
        return Err(format!("expected KruskalWallis + DunnRankSum, got {:?}", rs.analysis.kind));
    }

    run.select_sheet(&graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.col.box_whisker")?;
    run.commit_modal_ok()?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.graph.pairwise.open")?;
    run.click("pairwise.auto_add")?;
    run.commit_modal_ok()?;

    let ref_data: [&[Option<f64>]; 3] = [
        &[Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0)],
        &[Some(8.0), Some(9.0), Some(10.0), Some(11.0), Some(12.0)],
        &[Some(20.0), Some(21.0), Some(22.0), Some(23.0), Some(24.0)],
    ];
    let ref_sheet = build_column_sheet("ref-wf11", "KW Rank Groups", ColumnDesignHint::UnpairedReplicates, &["Ctrl", "DrugA", "DrugB"], &ref_data)?;
    let kw_kind = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        method: OneWayMethod::KruskalWallis,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::DunnRankSum, ..MultiplicitySpec::default() },
        ..OneWayAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf11", "sheet_id": "ref-res-wf11", "kind": kw_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-wf11", "data_sheet_id": "ref-wf11", "kind": "column" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-g-wf11", "kind": "column", "option_id": "col.box_tukey" }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-res-wf11"))
        .with_graph(graph_id.clone())
        .ref_graph_id(SheetId::new("ref-g-wf11"))
        .scalar("Kruskal-Wallis H", None, 1e-6, 1e-6)
        .scalar("P value", None, 1e-9, 1e-9)
        .tab_cell(0, 0, 1, 1e-9, 1e-9)
        .check_graph(|gui_gs, _ref_gs| {
            if gui_gs.spec.comparisons.is_empty() {
                return Err(String::from("expected Dunn pairwise brackets on WF-11 Box graph"));
            }
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(col_id, 0, 1, 0, "30.0"),
        vec![LiveEditExpectation::scalar_changed(res_id, "Kruskal-Wallis H", None, 0.1), LiveEditExpectation::graph_svg_changed(graph_id)],
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-12: Two-Way ANOVA (Interaction, Type III SS, Grouped error bars & Stacked R1)
// ---------------------------------------------------------------------------

#[test]
fn wf12_human_two_way_anova_interaction_and_grouped_guard() -> TestRes {
    let mut run = WorkflowRun::start("WF-12");
    let graphs_before = graph_id_set(run.document());
    let grp_id = run.create_table_via_welcome("welcome.arch.grouped", "Dose x Genotype", &["welcome.stepper.cols.dec"])?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &grp_id)?;

    run.enter_column_title(0, "WT")?;
    run.enter_column_title(1, "KO")?;
    for (r_idx, label) in ["0uM", "10uM", "100uM"].iter().enumerate() {
        run.enter_row_title(r_idx, label)?;
    }
    run.click("grid.cell.0.0.0")?;
    let tsv = "10\t11\t10\t10\t9\t11\n12\t13\t11\t22\t21\t23\n14\t15\t13\t42\t40\t44";
    run.paste_clipboard(tsv)?;

    let res_id = run.run_analyze_entry("Two-way", "analyze.entry.stats.factorial_anova")?;

    // Exercise non-default SS Type I + Sidak, verify Type III + non-default Sidak explicitly via Cmd+T.
    run.open_cmd_t()?;
    run.click("edit.fac.ss_type_i")?;
    run.click("edit.tab.multiple_comparisons")?;
    run.click("edit.post_hoc.sidak")?;
    run.commit_modal_ok()?;

    let rs_type1 = run.document().result_sheet(&res_id).ok_or_else(|| String::from("missing Factorial Type I ResultSheet"))?;
    if !matches!(
        rs_type1.analysis.kind,
        AnalysisKind::FactorialAnova(ref f) if f.ss_type == SumOfSquaresType::TypeI && f.multiplicity.method == MultiplicityMethod::Sidak
    ) {
        return Err(format!("expected TypeI + Sidak after GUI switch, got {:?}", rs_type1.analysis.kind));
    }

    // Switch back to Type III SS (keeping non-default Sidak multiplicity) and test Grouped Interleaved vs Stacked R1 guard.
    run.select_sheet(&res_id)?;
    run.open_cmd_t()?;
    run.click("edit.fac.ss_type_iii")?;
    run.commit_modal_ok()?;

    run.select_sheet(&graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.grouped.stacked")?;
    run.commit_modal_ok()?;
    let gs_stacked = run.document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing stacked grouped graph"))?;
    if gs_stacked.spec.kind != GraphKind::Grouped
        || gs_stacked.spec.grouped_arrangement != GroupedArrangement::Stacked
        || gs_stacked.spec.series.iter().any(|s| s.y_error_bars.is_some())
    {
        return Err(format!("R1: Grouped Stacked must clear y_error_bars, got {:?}", gs_stacked.spec));
    }

    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.grouped.interleaved")?;
    run.commit_modal_ok()?;

    let rows_flat: [&[f64]; 3] = [&[10.0, 11.0, 10.0, 10.0, 9.0, 11.0], &[12.0, 13.0, 11.0, 22.0, 21.0, 23.0], &[14.0, 15.0, 13.0, 42.0, 40.0, 44.0]];
    let ref_sheet = build_grouped_sheet("ref-wf12", "Dose x Genotype", &["WT", "KO"], &["0uM", "10uM", "100uM"], 3, &rows_flat)?;
    let fac_kind = serde_json::to_value(AnalysisKind::FactorialAnova(FactorialAnovaSpec {
        ss_type: SumOfSquaresType::TypeIII,
        max_interaction_order: 2,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Sidak, ..MultiplicitySpec::default() },
        ..FactorialAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf12", "sheet_id": "ref-res-wf12", "kind": fac_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-wf12", "data_sheet_id": "ref-wf12", "kind": "grouped" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-g-wf12", "kind": "grouped", "option_id": "grouped.interleaved" }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-res-wf12"))
        .with_graph(graph_id.clone())
        .ref_graph_id(SheetId::new("ref-g-wf12"))
        .scalar("Row Factor × Column Factor: F", None, 1e-6, 1e-6)
        .scalar("Row Factor × Column Factor: P", None, 1e-9, 1e-9)
        .tab_cell(0, 2, 3, 1e-5, 1e-5)
        .check_graph(|gui_gs, ref_gs| {
            if gui_gs.spec.grouped_arrangement != GroupedArrangement::Interleaved
                || ref_gs.spec.grouped_arrangement != GroupedArrangement::Interleaved
            {
                return Err(String::from("expected GroupedArrangement::Interleaved"));
            }
            if !gui_gs.spec.series.iter().any(|s| s.y_error_bars.is_some()) {
                return Err(String::from("expected Interleaved grouped graph to attach Y error bars"));
            }
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(grp_id, 2, 1, 2, "68.0"),
        vec![
            LiveEditExpectation::scalar_changed(res_id, "Row Factor × Column Factor: F", None, 0.5),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;
    run.finish()
}

#[test]
fn issue_76_one_way_anova_selected_pairs_human_workflow() -> TestRes {
    // Five-stage human-path WorkflowRun exercising One-Way ANOVA planned selected-pairs UI controls
    // (`edit.compare_selected_pairs` + `edit.selected_pair.<a>.<b>`) end-to-end against a fresh
    // headless Session oracle (Hsu 1996 / Sidak 1967).
    let mut run = WorkflowRun::start("ISSUE-76-SELECTED-PAIRS");
    let col_id = run.create_table_via_welcome("welcome.arch.column", "Four Dose Cohort", &["welcome.stepper.cols.inc"])?;

    for (idx, name) in ["Ctrl", "Low", "Mid", "High"].iter().enumerate() {
        run.enter_column_title(idx, name)?;
    }
    run.click("grid.cell.0.0.0")?;
    let tsv = "10.0\t12.0\t15.0\t22.0\n10.5\t12.4\t15.6\t23.0\n9.5\t11.6\t14.4\t21.0\n10.0\t12.0\t15.0\t22.0";
    run.paste_clipboard(tsv)?;

    let res_id = run.run_analyze_entry("One-way", "analyze.entry.stats.one_way_anova")?;
    run.select_sheet(&res_id)?;

    // Open Cmd+T -> Multiple Comparisons -> choose Sidak + enable selected-pairs -> toggle pairs (0,1) + (0,3).
    run.open_cmd_t()?;
    run.click("edit.tab.multiple_comparisons")?;
    run.click("edit.post_hoc.sidak")?;
    run.click("edit.compare_selected_pairs")?;
    // Default seeded pair is (0, 1); add pair (0, 3) ("Ctrl vs. High") for a 2-pair planned family.
    run.click("edit.selected_pair.0.3")?;
    run.commit_modal_ok()?;

    let rs_sel = run.document().result_sheet(&res_id).ok_or_else(|| String::from("missing selected-pairs ResultSheet"))?;
    match &rs_sel.analysis.kind {
        AnalysisKind::OneWayAnova(s)
            if s.multiplicity.method == MultiplicityMethod::Sidak
                && s.multiplicity.control_column.is_none()
                && s.multiplicity.selected_pairs.as_deref() == Some(&[(0, 1), (0, 3)][..]) => {}
        other => return Err(format!("expected Sidak with selected_pairs=[(0,1),(0,3)], got {other:?}")),
    }
    if rs_sel.pairwise_comparisons.len() != 2 {
        return Err(format!("expected 2 planned comparisons, got {}", rs_sel.pairwise_comparisons.len()));
    }

    let ref_data: [&[Option<f64>]; 4] = [
        &[Some(10.0), Some(10.5), Some(9.5), Some(10.0)],
        &[Some(12.0), Some(12.4), Some(11.6), Some(12.0)],
        &[Some(15.0), Some(15.6), Some(14.4), Some(15.0)],
        &[Some(22.0), Some(23.0), Some(21.0), Some(22.0)],
    ];
    let ref_sheet =
        build_column_sheet("ref-issue76", "Four Dose Cohort", ColumnDesignHint::UnpairedReplicates, &["Ctrl", "Low", "Mid", "High"], &ref_data)?;
    let sel_kind = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec {
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Sidak,
            selected_pairs: Some(vec![(0, 1), (0, 3)]),
            ..MultiplicitySpec::default()
        },
        ..OneWayAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    let mc_tab_idx =
        rs_sel.tabs.iter().position(|t| t.id == "multiple-comparisons").ok_or_else(|| String::from("missing multiple-comparisons tab"))?;
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-issue76", "sheet_id": "ref-res-76", "kind": sel_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-res-76"))
        .scalar("F", None, 1e-6, 1e-6)
        .scalar("P value", None, 1e-9, 1e-9)
        .tab_cell(mc_tab_idx, 0, 5, 1e-9, 1e-9)
        .tab_cell(mc_tab_idx, 1, 5, 1e-9, 1e-9),
    )?;

    run.live_edit_all(LiveEditTarget::sheet_grid_cell(col_id, 0, 1, 0, "18.0"), vec![LiveEditExpectation::scalar_changed(res_id, "F", None, 1.0)])?;
    run.finish()
}
