#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Native `egui` five-stage `WorkflowRun` & `GWF-11` acceptance tests (`WF-01`–`WF-06`).
//!
//! `GWF-11` Column ANOVA 14-locale parity gate (`r15_4_phase_a_column_anova_layout_and_14_locale_gate`),
//! XY 4PL dose-response + multi-row Format Selected Points + Cmd+Shift+E ExcludeWithReason (`WF-01`),
//! Normalize -> ResultSheet chaining -> Transform Log10(X) multi-hop reactive propagation (`WF-02`),
//! Column graph canvas brushing + Format Selected Points + Edit menu Undo/Redo (`WF-03`),
//! File > Import Plate96 / TidyLongPivot + Convert Table Archetype (`WF-04`),
//! PruneRows -> chained PCA + Clustering (`WF-05`),
//! PruneRows -> chained Descriptive + Kaplan-Meier + Multiple Imputation quantitative delta (`WF-06`).

use std::collections::BTreeSet;
use std::sync::Arc;

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, GraphSheet, MathTransformPreset,
    MultipleVariablesTable, MultivariateSpec, NonlinearFitSpec, NormalizeAnchor, OneWayMethod, ProjectDocument, ResultCell, ResultStatus, SheetId,
    SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, VariableRole, XyTable,
};
use curvebook_palette::{MarkerShape, Rgba8};
use curvebook_testkit::wf_datasets::{MvCellValue, MvFixture, clustering_three_clusters, dose_response_4pl};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::inspect_bridge::inspect_live_egui_snapshot;
use curvebook_ui_egui::egui_shell::stat_anova_modal::resolve_one_way_anova_banner;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton};
use curvebook_ui_egui::os_integration::{PlatformClipboardAdapter, StubPlatformClipboardAdapter};
use serde_json::json;

use crate::human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use crate::human_wf_common::{EguiWfHarness, TestRes, assert_input_to_paint_budget, assert_zero_shortcut_compliance, default_viewport, ensure};

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

fn mv_fixture_to_data_tsv(fx: &MvFixture) -> String {
    fx.rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| match c {
                    MvCellValue::Text(s) => (*s).to_owned(),
                    MvCellValue::Number(v) => format!("{v}"),
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_xy_single_sheet(id: &str, title: &str, col_titles: &[&str], rows: &[(f64, &[f64])]) -> Result<DataSheet, String> {
    let mut tbl = XyTable::new(SubcolumnMode::Single, col_titles.len(), rows.len()).map_err(|e| e.to_string())?;
    for (c_idx, &ct) in col_titles.iter().enumerate() {
        tbl.data_sets[c_idx].title = ct.to_owned();
    }
    for (r_idx, &(x_val, ys)) in rows.iter().enumerate() {
        tbl.x_cells[r_idx][0] = Cell::number(x_val);
        for (c_idx, &yv) in ys.iter().enumerate() {
            tbl.data_sets[c_idx].cells[r_idx][0] = Cell::number(yv);
        }
    }
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    DataSheet::new(sid, title, TableArchetype::Xy(tbl)).map_err(|e| e.to_string())
}

fn build_column_sheet(id: &str, title: &str, col_titles: &[&str], rows: &[&[f64]]) -> Result<DataSheet, String> {
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, col_titles.len(), rows.len()).map_err(|e| e.to_string())?;
    for (c_idx, &ct) in col_titles.iter().enumerate() {
        tbl.data_sets[c_idx].title = ct.to_owned();
    }
    for (r_idx, row_vals) in rows.iter().enumerate() {
        for (c_idx, &v) in row_vals.iter().enumerate() {
            tbl.data_sets[c_idx].cells[r_idx][0] = Cell::number(v);
        }
    }
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    DataSheet::new(sid, title, TableArchetype::Column(tbl)).map_err(|e| e.to_string())
}

fn build_mv_survival_sheet(id: &str, title: &str, rows: &[(&str, f64, f64, f64)]) -> Result<DataSheet, String> {
    let mut tbl = MultipleVariablesTable::new(4, rows.len()).map_err(|e| e.to_string())?;
    let specs = [
        ("Group", VariableRole::Label),
        ("Time", VariableRole::Continuous),
        ("Status", VariableRole::Continuous),
        ("Biomarker", VariableRole::Continuous),
    ];
    for (c_idx, &(name, role)) in specs.iter().enumerate() {
        tbl.variables[c_idx].title = name.to_owned();
        tbl.variables[c_idx].role = role;
        tbl.variables[c_idx].role_inferred = false;
    }
    for (r_idx, &(grp, t, st, bio)) in rows.iter().enumerate() {
        tbl.variables[0].cells[r_idx] = Cell::text(grp);
        tbl.variables[1].cells[r_idx] = Cell::number(t);
        tbl.variables[2].cells[r_idx] = Cell::number(st);
        tbl.variables[3].cells[r_idx] = Cell::number(bio);
    }
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    DataSheet::new(sid, title, TableArchetype::MultipleVariables(tbl)).map_err(|e| e.to_string())
}

fn build_mv_missing_sheet(id: &str, title: &str, rows: &[[Option<f64>; 3]]) -> Result<DataSheet, String> {
    let mut tbl = MultipleVariablesTable::new(3, rows.len()).map_err(|e| e.to_string())?;
    for (c_idx, name) in ["X1", "X2", "X3"].iter().enumerate() {
        tbl.variables[c_idx].title = (*name).to_owned();
        tbl.variables[c_idx].role = VariableRole::Continuous;
        tbl.variables[c_idx].role_inferred = false;
    }
    for (r_idx, vals) in rows.iter().enumerate() {
        for (c_idx, v_opt) in vals.iter().enumerate() {
            tbl.variables[c_idx].cells[r_idx] = v_opt.map_or_else(Cell::empty, Cell::number);
        }
    }
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    DataSheet::new(sid, title, TableArchetype::MultipleVariables(tbl)).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// GWF-11: Strict GUI-only Phase A Column ANOVA + R3/R2 + 14-locale §9.4 Gate
// ---------------------------------------------------------------------------

#[test]
fn r15_4_phase_a_column_anova_layout_and_14_locale_gate() -> TestRes {
    let mut h = EguiWfHarness::new(default_viewport());
    h.assert_parity_and_layout("Phase A Column ANOVA initial Welcome")?;

    let ds_id = h.create_table_via_welcome("welcome.arch.column", "Phase A Drug Cohort", &[])?;
    for (r, vals) in [[10.0, 14.0, 22.0], [11.0, 15.0, 24.0], [9.5, 13.5, 20.5], [10.5, 14.5, 23.0]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            h.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let val = h.app().document().data_sheet(&ds_id).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(val == Some(10.0), "Phase A column grid edit failed")?;

    let rs_id = h.run_analyze_entry("One-way", "analyze.entry.stats.one_way_anova")?;
    h.select_sheet(&rs_id)?;
    h.open_cmd_t()?;
    h.click("edit.tab.multiple_comparisons")?;
    let ctrl_before =
        h.driver.frame_meta().find_by_widget_id("edit.control_column").ok_or_else(|| String::from("missing edit.control_column"))?.enabled;
    ensure(!ctrl_before, "R3 control_column starts disabled")?;
    h.click("edit.compare_against_control")?;
    let ctrl_after = h
        .driver
        .frame_meta()
        .find_by_widget_id("edit.control_column")
        .ok_or_else(|| String::from("missing edit.control_column after unlock"))?
        .enabled;
    ensure(ctrl_after, "R3 unlocks control_column")?;
    h.click("edit.tab.experimental_design")?;
    h.click("edit.assume_gaussian")?;
    ensure(resolve_one_way_anova_banner(h.app()).contains("Kruskal-Wallis"), "R2 banner")?;
    h.click("modal.ok")?;

    let rs = h.app().document().result_sheet(&rs_id).ok_or_else(|| String::from("missing updated ResultSheet"))?;
    ensure(matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::KruskalWallis), "expected Kruskal-Wallis")?;
    let inspect_snap = inspect_live_egui_snapshot(h.app(), default_viewport(), None, true);
    ensure(inspect_snap.active_sheet_id.as_deref() == Some(rs_id.as_str()), "ui.inspect mismatch")?;
    let shot = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    ensure(shot.starts_with(b"\x89PNG\r\n\x1a\n") && shot.len() > 256, "invalid egui PNG screenshot")?;
    h.driver.set_viewport(curvebook_ui_egui::interaction::ViewportMetrics::new(1440.0, 600.0, 1.0));
    h.assert_parity_and_layout("Phase A Column ANOVA ResultSheet 1440x600")?;
    h.select_sheet(&ds_id)?;
    assert_zero_shortcut_compliance(&mut h)?;
    h.verify_14_locales_section_9_4("Phase A Column ANOVA")?;
    assert_input_to_paint_budget(&h, 50.0)
}

// ---------------------------------------------------------------------------
// WF-01: XY 4PL Dose-Response + Multi-Row Format Points + Cmd+Shift+E Exclude
// ---------------------------------------------------------------------------

#[test]
fn wf01_human_xy_dose_response_format_exclude_with_reason() -> TestRes {
    let mut run = WorkflowRun::start("WF-01");
    let graphs_before = graph_id_set(run.document());
    let fx = dose_response_4pl();
    let xy_id = run.create_table_via_welcome("welcome.arch.xy", "Dose-Response 4PL", &["welcome.stepper.cols.dec", "welcome.subcol.raw"])?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &xy_id)?;

    run.enter_column_title(0, "Agonist_A")?;
    run.enter_column_title(1, "Agonist_B")?;
    let data_tsv: String = fx
        .rows
        .iter()
        .map(|row| {
            let mut cols = vec![format!("{}", row.x)];
            for series in &row.series_replicates {
                for &v in series {
                    cols.push(format!("{v}"));
                }
            }
            cols.join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n");
    run.click("grid.xcell.0.0")?;
    run.paste_clipboard(&data_tsv)?;

    // Multi-row range selection (`Shift+Click` across rows 1..=3) -> Trailing Studio Style Subset (`Format Selected Points`).
    run.click("grid.cell.1.0.0")?;
    let end_pos = run.widget_center("grid.cell.3.0.0")?;
    run.click_at_button(end_pos, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    run.click("inspector.table.style_subset")?;
    run.click("fmt_pts.col.Red")?;
    run.click("fmt_pts.sym.Diamond")?;
    run.commit_modal_ok()?;
    let gs_styled = run.document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing WF-01 graph"))?;
    if gs_styled.spec.point_overrides.len() < 3 {
        return Err(format!("expected >= 3 point overrides from multi-row selection, got {}", gs_styled.spec.point_overrides.len()));
    }

    // `Cmd+Shift+E` -> ExcludeWithReason audit entry on cell (2, 0, 0).
    run.click("grid.cell.2.0.0")?;
    run.press_key("e", KeyModifiers { meta: true, ctrl: true, shift: true, alt: false })?;
    if !matches!(run.active_dialog(), ActiveDialog::ExcludeWithReason { .. }) {
        return Err(format!("expected ExcludeWithReason dialog, found {:?}", run.active_dialog()));
    }
    run.click("exclude.preset.0")?;
    run.replace_textbox("exclude.note", "edge well bubble")?;
    run.commit_modal_ok()?;
    let excl_cell = run.document().data_sheet(&xy_id).and_then(|d| d.cell(CellCoord::new(2, 0, 0))).ok_or_else(|| String::from("missing cell"))?;
    if !excl_cell.excluded {
        return Err(String::from("expected cell (2,0,0) excluded after ExcludeWithReason"));
    }

    // Fit 4PL nonlinear curve and enable non-default Profile-likelihood CI + 99% confidence + bands.
    let fit_id = run.run_analyze_entry("nonlinear", "analyze.entry.fit.nonlinear")?;
    run.open_cmd_t()?;
    run.click("fit.tab.model")?;
    run.click("fit.eq.item.log_agonist_vs_response_4pl")?;
    run.click("fit.tab.output")?;
    run.click("fit.ci.profile")?;
    run.click("fit.conf.99")?;
    run.click("fit.out.conf_bands")?;
    run.commit_modal_ok()?;

    let mut ref_sheet = fx.to_data_sheet("ref-wf01").map_err(|e| e.to_string())?;
    ref_sheet.title = String::from("Dose-Response 4PL");
    if let TableArchetype::Xy(ref mut xy) = ref_sheet.archetype {
        xy.data_sets[0].cells[2][0].excluded = true;
    }
    let mut ref_nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    ref_nl.profile_likelihood_ci = true;
    ref_nl.confidence_bands = true;
    let nl_kind = serde_json::to_value(AnalysisKind::NonlinearRegression(ref_nl)).map_err(|e| e.to_string())?;

    run.verify_reference(
        ReferenceSpec::for_result(fit_id.clone(), move |sess| {
            sess.execute(
                "analysis.run",
                json!({
                    "source_sheet_id": "ref-wf01",
                    "sheet_id": "ref-fit-wf01",
                    "kind": nl_kind,
                    "confidence_percent": 99.0
                }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-wf01", "data_sheet_id": "ref-wf01", "kind": "xy" }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-fit-wf01"))
        .with_graph(graph_id.clone())
        .ref_graph_id(SheetId::new("ref-g-wf01"))
        .scalar("LogEC50", Some(0), 1e-4, 1e-4)
        .scalar("HillSlope", Some(0), 1e-4, 1e-4)
        .scalar("R2", None, 1e-6, 1e-6)
        .check_graph(|gui_gs, _ref_gs| {
            if !gui_gs
                .spec
                .point_overrides
                .iter()
                .any(|o| o.color == Some(Rgba8::rgba(0xDC, 0x26, 0x26, 0xFF)) && o.symbol == Some(MarkerShape::Diamond))
            {
                return Err(format!("expected Red Diamond point override on WF-01 graph: {:?}", gui_gs.spec.point_overrides));
            }
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(xy_id, 4, 0, 0, "45.0"),
        vec![
            LiveEditExpectation::scalar_changed(fit_id.clone(), "LogEC50", Some(0), 1e-4),
            LiveEditExpectation::scalar_changed(fit_id, "R2", None, 1e-5),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-02: Normalize -> ResultSheet Chaining -> Transform Log10(X) + Multi-Hop LiveEdit
// ---------------------------------------------------------------------------

#[test]
fn wf02_human_normalize_to_result_sheet_chaining_and_log_x() -> TestRes {
    let mut run = WorkflowRun::start("WF-02");
    let graphs_before = graph_id_set(run.document());
    let xy_id = run.create_table_via_welcome("welcome.arch.xy", "Normalize LogX", &["welcome.stepper.cols.dec"])?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &xy_id)?;

    run.enter_column_title(0, "Vehicle")?;
    run.enter_column_title(1, "Drug")?;
    run.click("grid.xcell.0.0")?;
    run.paste_clipboard("1.0\t10.0\t5.0\n10.0\t25.0\t12.5\n100.0\t55.0\t27.5\n1000.0\t110.0\t55.0")?;

    let norm_id = run.run_analyze_entry("normalize", "analyze.entry.transform.normalize")?;

    // Chain Transform Math directly onto the Normalize ResultSheet and configure Log10(X), Y = None.
    run.select_sheet(&norm_id)?;
    let tx_id = run.run_analyze_entry("transform", "analyze.entry.transform.math")?;
    run.open_cmd_t()?;
    run.click("edit.transform.x_transform.log10")?;
    run.click("edit.transform.y_transform.none")?;
    run.commit_modal_ok()?;

    let tx_rs = run.document().result_sheet(&tx_id).ok_or_else(|| String::from("missing chained Transform ResultSheet"))?;
    if tx_rs.analysis.source_sheet_id.as_ref() != Some(&norm_id) {
        return Err(format!("expected Transform parent `{}`, got `{:?}`", norm_id.as_str(), tx_rs.analysis.source_sheet_id));
    }
    if !matches!(
        tx_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: Some(MathTransformPreset::Log10), y_transform: None, .. })
    ) {
        return Err(format!("expected Log10(X), Y=None on chained Transform, got {:?}", tx_rs.analysis.kind));
    }

    // Chain Descriptive AND Linear Regression on Transform ResultSheet so Stage 4/5 verify both normalized Y and Log10(X) numerically!
    run.select_sheet(&tx_id)?;
    let tx_desc_id = run.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    run.select_sheet(&tx_id)?;
    let tx_reg_id = run.run_analyze_entry("linear", "analyze.entry.fit.linear")?;

    let ref_rows: [(f64, &[f64]); 4] = [(1.0, &[10.0, 5.0]), (10.0, &[25.0, 12.5]), (100.0, &[55.0, 27.5]), (1000.0, &[110.0, 55.0])];
    let ref_xy = build_xy_single_sheet("ref-wf02", "Normalize LogX", &["Vehicle", "Drug"], &ref_rows)?;
    let norm_kind = serde_json::to_value(AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
        zero_anchor: NormalizeAnchor::ColumnMin,
        hundred_anchor: NormalizeAnchor::ColumnMax,
        output_percent: true,
        per_data_set: true,
    }))
    .map_err(|e| e.to_string())?;
    let math_kind = serde_json::to_value(AnalysisKind::Transform(TransformAnalysisSpec::Math {
        x_transform: Some(MathTransformPreset::Log10),
        y_transform: None,
        error_propagation: curvebook_doc::ErrorPropagationMode::ReplicateEndpoints,
        pharmacology_preset: None,
        interchange_xy: false,
        zero_x_substitute: None,
        asymmetric_ci_policy: Default::default(),
    }))
    .map_err(|e| e.to_string())?;
    let desc_kind = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    let lin_kind = serde_json::to_value(AnalysisKind::LinearRegression(curvebook_doc::LinearRegressionSpec::default())).map_err(|e| e.to_string())?;

    let ref_xy2 = ref_xy.clone();
    let ref_xy3 = ref_xy.clone();
    let norm_kind2 = norm_kind.clone();
    let norm_kind3 = norm_kind.clone();
    let math_kind2 = math_kind.clone();
    let math_kind3 = math_kind.clone();
    run.verify_reference(
        ReferenceSpec::for_result(tx_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf02", "sheet_id": "ref-norm-wf02", "kind": norm_kind2 }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-norm-wf02", "sheet_id": "ref-tx-wf02", "kind": math_kind2 }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_xy2)?
        .ref_result_id(SheetId::new("ref-tx-wf02"))
        .tab_cell(0, 0, 0, 1e-9, 1e-9),
    )?;

    run.verify_reference(
        ReferenceSpec::for_result(tx_desc_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf02", "sheet_id": "ref-norm2-wf02", "kind": norm_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-norm2-wf02", "sheet_id": "ref-tx2-wf02", "kind": math_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-tx2-wf02", "sheet_id": "ref-desc-wf02", "kind": desc_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_xy)?
        .ref_result_id(SheetId::new("ref-desc-wf02"))
        .scalar("Mean", Some(0), 1e-6, 1e-6)
        .scalar("Mean", Some(1), 1e-6, 1e-6)
        .tab_cell(0, 9, 0, 1e-6, 1e-6)
        .tab_cell(0, 9, 1, 1e-6, 1e-6),
    )?;

    run.verify_reference(
        ReferenceSpec::for_result(tx_reg_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf02", "sheet_id": "ref-norm3-wf02", "kind": norm_kind3 }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-norm3-wf02", "sheet_id": "ref-tx3-wf02", "kind": math_kind3 }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-tx3-wf02", "sheet_id": "ref-lin-wf02", "kind": lin_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_xy3)?
        .ref_result_id(SheetId::new("ref-lin-wf02"))
        .scalar("Slope", Some(0), 1e-6, 1e-6)
        .scalar("YIntercept", Some(0), 1e-6, 1e-6)
        .scalar("R2", Some(0), 1e-6, 1e-6),
    )?;

    // Stage 5: Editing upstream max Y (110.0 -> 220.0) propagates across Normalize -> Transform -> Descriptive & LinearRegression!
    let norm_chk = norm_id.clone();
    let tx_chk = tx_id.clone();
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(xy_id, 3, 0, 0, "220.0"),
        vec![
            LiveEditExpectation::tab_cell_changed(tx_desc_id, 0, 9, 0),
            LiveEditExpectation::scalar_changed(tx_reg_id, "Slope", Some(0), 1e-5),
            LiveEditExpectation::custom(move |_before, _after, doc| {
                let norm_y = doc
                    .result_sheet(&norm_chk)
                    .and_then(|r| r.analyzable_data.as_ref())
                    .and_then(|d| d.cell(CellCoord::new(1, 0, 0)))
                    .and_then(Cell::active_number)
                    .ok_or_else(|| String::from("missing norm_id analyzable_data cell (1,0,0)"))?;
                let tx_y = doc
                    .result_sheet(&tx_chk)
                    .and_then(|r| r.analyzable_data.as_ref())
                    .and_then(|d| d.cell(CellCoord::new(1, 0, 0)))
                    .and_then(Cell::active_number)
                    .ok_or_else(|| String::from("missing tx_id analyzable_data cell (1,0,0)"))?;
                if (norm_y - ((25.0 - 10.0) / (220.0 - 10.0) * 100.0)).abs() > 1e-5 || (tx_y - norm_y).abs() > 1e-5 {
                    return Err(format!("expected multi-hop normalized Y = 7.1429, got norm={norm_y} tx={tx_y}"));
                }
                Ok(())
            }),
            LiveEditExpectation::graph_svg_changed(graph_id),
        ],
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-03: Column Graph Brushing + Format Selected Points + Edit Menu Undo/Redo
// ---------------------------------------------------------------------------

#[test]
fn wf03_human_column_brushing_format_and_undo_redo() -> TestRes {
    let mut run = WorkflowRun::start("WF-03");
    let graphs_before = graph_id_set(run.document());
    let col_id = run.create_table_via_welcome("welcome.arch.column", "Brushing Cohort", &[])?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &col_id)?;

    for (idx, name) in ["Control", "DrugA", "DrugB"].iter().enumerate() {
        run.enter_column_title(idx, name)?;
    }
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("10.0\t14.0\t20.0\n11.0\t15.0\t22.0\n12.0\t16.0\t24.0\n13.0\t17.0\t26.0")?;

    // Switch Column graph to scatter_bar so plotted points render on canvas, then hover & click-brush a point.
    run.select_sheet(&graph_id)?;
    run.open_menu_and_click("menu.graph", &[], "menu.ui.changeGraphType.open")?;
    run.click("graph_type.card.col.bar_replicates")?;
    run.commit_modal_ok()?;

    let gs = run.document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing WF-03 graph"))?;
    let scene = compile_graph(gs, run.document()).map_err(|e| e.to_string())?;
    let first_pt = scene.plotted_points.first().ok_or_else(|| String::from("expected plotted_points on col.scatter_bar"))?;
    let canvas_rect = run.frame_meta().find_by_widget_id("canvas.viewport").ok_or_else(|| String::from("missing canvas.viewport"))?.clipped_rect;
    let fit_scale = ((canvas_rect.width() - 28.0).max(40.0) / (scene.width_pt.max(60.0) as f32))
        .min((canvas_rect.height() - 28.0).max(40.0) / (scene.height_pt.max(60.0) as f32))
        .clamp(0.2, 4.0);
    let paper_min = egui::pos2(
        canvas_rect.center().x - (scene.width_pt as f32) * fit_scale * 0.5,
        canvas_rect.center().y - (scene.height_pt as f32) * fit_scale * 0.5,
    );
    let screen_pt = egui::pos2(paper_min.x + (first_pt.center_pt.x as f32) * fit_scale, paper_min.y + (first_pt.center_pt.y as f32) * fit_scale);
    run.hover_at(screen_pt)?;
    run.click_at(screen_pt)?;
    if run.app().brushed_points.is_empty() {
        return Err(String::from("expected canvas click on plotted point to populate brushed_points"));
    }

    // Open Format Selected Points for the canvas-brushed point via Graph > Format Selected Points...
    run.open_menu_and_click("menu.graph", &[], "menu.ui.formatPoints.open")?;
    run.click("fmt_pts.col.Red")?;
    run.click("fmt_pts.sym.Diamond")?;
    run.commit_modal_ok()?;
    let gs_fmt = run.document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing formatted graph"))?;
    if gs_fmt.spec.point_overrides.is_empty() {
        return Err(String::from("expected canvas drill-back FormatSelectedPoints override"));
    }

    // Edit > Undo and Edit > Redo round-trip via top menu bar on an upstream grid edit.
    run.select_sheet(&col_id)?;
    run.enter_grid_cell(0, 0, 0, "99.0")?;
    let v_edited = run.document().data_sheet(&col_id).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(v_edited == Some(99.0), "expected edited cell 99.0")?;
    run.open_menu_and_click("menu.edit", &[], "menu.edit.undo")?;
    let v_undone = run.document().data_sheet(&col_id).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(v_undone == Some(10.0), "Edit > Undo must restore 10.0")?;
    run.open_menu_and_click("menu.edit", &[], "menu.edit.redo")?;
    let v_redone = run.document().data_sheet(&col_id).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    ensure(v_redone == Some(99.0), "Edit > Redo must re-apply 99.0")?;
    run.open_menu_and_click("menu.edit", &[], "menu.edit.undo")?;

    // Analyze with non-default Welch One-Way ANOVA and verify ResultSheet + brushed/formatted Graph vs oracle.
    let res_id = run.run_analyze_entry("One-way", "analyze.entry.stats.one_way_anova")?;
    run.open_cmd_t()?;
    run.click("edit.anova.welch")?;
    run.commit_modal_ok()?;

    let ref_rows: [&[f64]; 4] = [&[10.0, 14.0, 20.0], &[11.0, 15.0, 22.0], &[12.0, 16.0, 24.0], &[13.0, 17.0, 26.0]];
    let ref_col = build_column_sheet("ref-wf03", "Brushing Cohort", &["Control", "DrugA", "DrugB"], &ref_rows)?;
    let welch_kind = serde_json::to_value(AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec {
        method: OneWayMethod::BrownForsytheAndWelch,
        ..curvebook_doc::OneWayAnovaSpec::default()
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf03", "sheet_id": "ref-res-wf03", "kind": welch_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-wf03", "data_sheet_id": "ref-wf03", "kind": "column" }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.setKind", json!({ "sheet_id": "ref-g-wf03", "kind": "column", "option_id": "col.scatter_bar" }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_col)?
        .ref_result_id(SheetId::new("ref-res-wf03"))
        .with_graph(graph_id.clone())
        .ref_graph_id(SheetId::new("ref-g-wf03"))
        .scalar("Welch W", None, 1e-6, 1e-6)
        .scalar("Welch P value", None, 1e-9, 1e-9)
        .check_graph(|gui_gs, _ref_gs| {
            if gui_gs.spec.point_overrides.is_empty() {
                return Err(String::from("expected canvas-brushed point_overrides on WF-03 GraphSheet"));
            }
            Ok(())
        }),
    )?;

    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(col_id, 0, 2, 0, "45.0"),
        vec![LiveEditExpectation::scalar_changed(res_id, "Welch W", None, 0.1), LiveEditExpectation::graph_svg_changed(graph_id)],
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-04: File > Import (Plate96 & TidyLongPivot) + Convert Table Archetype
// ---------------------------------------------------------------------------

#[test]
fn wf04_human_import_reshape_and_convert_archetype() -> TestRes {
    let clipboard = Arc::new(StubPlatformClipboardAdapter::new());
    clipboard.write_text("Dose,Response\nVehicle,10.0\nVehicle,12.0\nDrug,22.0\nDrug,24.0").map_err(|e| e.to_string())?;
    let mut run = WorkflowRun::start_with_clipboard("WF-04", clipboard);

    // 1. Real File > Import > Import Plate Map... (96-well plate with blank subtraction).
    run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPlateMap")?;
    run.replace_textbox("import_paste.input.title", "Plate96 Screen")?;
    run.click("import.subtract_blank_mean")?;
    let vals: String = (0..96).map(|i| format!("{:.1}", 10.0 + f64::from(i % 12) * 5.0)).collect::<Vec<_>>().join(", ");
    run.replace_textbox("import_paste.input.plate_values", &vals)?;
    run.commit_modal_ok()?;

    let plate_id = run.document().data_sheets.last().map(|d| d.id.clone()).ok_or_else(|| String::from("missing imported Plate96 sheet"))?;

    // 2. Real File > Import > Import Tidy Long Table... via GUI target/delimiter/column/clipboard controls.
    run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importTidyLong")?;
    run.click("import_paste.source.tidy")?;
    run.click("import_paste.target.column")?;
    run.click("import_paste.delim.comma")?;
    run.replace_textbox("import_paste.input.title", "Tidy Pivot Cohort")?;
    run.replace_textbox("import_paste.input.group_col", "Dose")?;
    run.replace_textbox("import_paste.input.value_col", "Response")?;
    run.click("import_paste.btn.paste_clipboard")?;
    run.commit_modal_ok()?;

    // 3. Convert Table Archetype via Data > Convert Table Archetype...
    run.select_sheet(&plate_id)?;
    run.open_menu_and_click("menu.data", &[], "menu.ui.table.convertArchetype.open")?;
    run.click("convert.target.column")?;
    run.click("convert.allow_lossy")?;
    run.commit_modal_ok()?;

    let converted = run.document().data_sheet(&plate_id).ok_or_else(|| String::from("missing converted sheet"))?;
    if converted.kind() != TableKind::Column {
        return Err(format!("expected Column archetype after conversion, got {:?}", converted.kind()));
    }
    let graphs_before = graph_id_set(run.document());
    run.open_menu_and_click("menu.insert", &[], "menu.ui.graph.create.open")?;
    run.click("create_graph.kind.column")?;
    run.commit_modal_ok()?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &plate_id)?;

    let res_id = run.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    let tidy_sheet = build_column_sheet("ref-wf04", "Tidy Pivot Cohort", &["Vehicle", "Drug"], &[&[10.0, 22.0], &[12.0, 24.0]])?;
    let desc_kind = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    let tidy_id = run
        .document()
        .data_sheets
        .iter()
        .find(|d| d.title == "Tidy Pivot Cohort")
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("missing Tidy Pivot Cohort sheet"))?;
    run.select_sheet(&tidy_id)?;
    let tidy_desc_id = run.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;
    run.verify_reference(
        ReferenceSpec::for_result(tidy_desc_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf04", "sheet_id": "ref-res-wf04", "kind": desc_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(tidy_sheet)?
        .ref_result_id(SheetId::new("ref-res-wf04"))
        .scalar("Mean", Some(0), 1e-6, 1e-6)
        .scalar("Mean", Some(1), 1e-6, 1e-6)
        .tab_cell(0, 9, 0, 1e-6, 1e-6)
        .tab_cell(0, 9, 1, 1e-6, 1e-6),
    )?;
    let _ = (res_id, graph_id);
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(tidy_id, 0, 1, 0, "40.0"),
        vec![
            LiveEditExpectation::scalar_changed(tidy_desc_id.clone(), "Mean", Some(1), 1.0),
            LiveEditExpectation::tab_cell_changed(tidy_desc_id, 0, 9, 1),
        ],
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-05: PruneRows -> Chained PCA (corr) + Clustering (Non-Default Options)
// ---------------------------------------------------------------------------

#[test]
fn wf05_human_transform_pca_and_clustering_pipeline() -> TestRes {
    let mut run = WorkflowRun::start("WF-05");
    let fx = clustering_three_clusters();
    let mv_id = run.create_table_via_welcome(
        "welcome.arch.multiple_variables",
        "MV Pipeline Cohort",
        &[
            "welcome.stepper.cols.inc",
            "welcome.mv.role.0.Label",
            "welcome.mv.role.1.Continuous",
            "welcome.mv.role.2.Continuous",
            "welcome.mv.role.3.Continuous",
        ],
    )?;
    for (c_idx, col) in fx.columns.iter().enumerate() {
        run.enter_column_title(c_idx, col.title)?;
    }
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard(&mv_fixture_to_data_tsv(&fx))?;

    // Stage 1: PruneRows with non-default `average_window = true` (every_k_rows = 2 -> 6 cluster pair averages).
    let prune_id = run.run_analyze_entry("Prune", "analyze.entry.transform.prune")?;
    run.open_cmd_t()?;
    run.click("edit.transform.prune.average_window")?;
    run.commit_modal_ok()?;

    // Stage 2: Chain PCA (`corr` standardize + toggle parallel analysis non-default None) onto PruneRows.
    run.select_sheet(&prune_id)?;
    let pca_id = run.run_analyze_entry("PCA", "analyze.entry.stats.pca")?;
    run.open_cmd_t()?;
    run.click("edit.pca.cov")?;
    run.click("edit.pca.corr")?;
    run.click("edit.pca.parallel")?;
    run.commit_modal_ok()?;

    // Stage 3: Chain Clustering onto PruneRows, selecting non-default Manhattan distance + Complete linkage + K=2 + sweep max.
    run.select_sheet(&prune_id)?;
    let clust_id = run.run_analyze_entry("Cluster", "analyze.entry.stats.clustering")?;
    run.open_cmd_t()?;
    run.click("edit.cluster.dist.manhattan")?;
    run.click("edit.cluster.link.complete")?;
    run.click("edit.cluster.k_means_k.dec")?;
    run.click("edit.cluster.k_sweep_max.inc")?;
    run.commit_modal_ok()?;

    let ref_mv = fx.to_data_sheet("ref-wf05").map_err(|e| e.to_string())?;
    let prune_kind = serde_json::to_value(AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: true }))
        .map_err(|e| e.to_string())?;
    let pca_kind = serde_json::to_value(AnalysisKind::Multivariate(MultivariateSpec::Pca {
        standardize: true,
        parallel_analysis_seed: None,
        pcr_outcome_column: None,
        retention_rule: Default::default(),
        emit_extra_matrices: false,
    }))
    .map_err(|e| e.to_string())?;
    let clust_kind = serde_json::to_value(AnalysisKind::Multivariate(MultivariateSpec::Clustering {
        k_means_k: Some(2),
        distance: curvebook_doc::ClusterDistance::Manhattan,
        linkage: curvebook_doc::ClusterLinkage::Complete,
        k_sweep_max: Some(7),
        consensus_optimal_k: false,
        kmeans_algorithm: Default::default(),
        cluster_direction: Default::default(),
        cut_height: None,
        emit_anova_tab: false,
    }))
    .map_err(|e| e.to_string())?;

    let ref_mv_clone = ref_mv.clone();
    let prune_kind_clone = prune_kind.clone();
    run.verify_reference(
        ReferenceSpec::for_result(pca_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf05", "sheet_id": "ref-prune-wf05", "kind": prune_kind_clone }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-prune-wf05", "sheet_id": "ref-pca-wf05", "kind": pca_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mv_clone)?
        .ref_result_id(SheetId::new("ref-pca-wf05"))
        .scalar("PC1 eigenvalue", None, 1e-6, 1e-6)
        .scalar("PC1 variance proportion", None, 1e-6, 1e-6),
    )?;

    run.verify_reference(
        ReferenceSpec::for_result(clust_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf05", "sheet_id": "ref-prune2-wf05", "kind": prune_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-prune2-wf05", "sheet_id": "ref-clust-wf05", "kind": clust_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mv)?
        .ref_result_id(SheetId::new("ref-clust-wf05"))
        .scalar("Clusters (k)", None, 1e-9, 1e-9)
        .scalar("Total WCSS", None, 1e-4, 1e-4)
        .scalar("Mean silhouette", None, 1e-5, 1e-5),
    )?;

    // Stage 5: Editing row 0 FeatureX propagates reactively through PruneRows -> PCA & Clustering.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(mv_id, 0, 1, 0, "18.5"),
        vec![
            LiveEditExpectation::scalar_changed(pca_id, "PC1 eigenvalue", None, 1e-4),
            LiveEditExpectation::scalar_changed(clust_id, "Total WCSS", None, 0.1),
        ],
    )?;
    run.finish()
}

// ---------------------------------------------------------------------------
// WF-06: PruneRows -> Chained Descriptive + Kaplan-Meier + Multiple Imputation
// ---------------------------------------------------------------------------

#[test]
fn wf06_human_prune_descriptive_survival_imputation_reactive() -> TestRes {
    let mut run = WorkflowRun::start("WF-06");
    let surv_id = run.create_table_via_welcome(
        "welcome.arch.multiple_variables",
        "MV Survival Cohort",
        &[
            "welcome.stepper.cols.inc",
            "welcome.mv.role.0.Label",
            "welcome.mv.role.1.Continuous",
            "welcome.mv.role.2.Continuous",
            "welcome.mv.role.3.Continuous",
        ],
    )?;
    for (c_idx, title) in ["Group", "Time", "Status", "Biomarker"].iter().enumerate() {
        run.enter_column_title(c_idx, title)?;
    }
    let surv_rows: [(&str, f64, f64, f64); 12] = [
        ("Ctrl", 10.0, 1.0, 100.0),
        ("Ctrl", 12.0, 0.0, 105.0),
        ("Ctrl", 16.0, 1.0, 120.0),
        ("Ctrl", 18.0, 1.0, 125.0),
        ("Ctrl", 22.0, 0.0, 140.0),
        ("Ctrl", 25.0, 1.0, 145.0),
        ("Drug", 20.0, 1.0, 200.0),
        ("Drug", 24.0, 0.0, 205.0),
        ("Drug", 30.0, 1.0, 220.0),
        ("Drug", 32.0, 0.0, 225.0),
        ("Drug", 40.0, 1.0, 240.0),
        ("Drug", 44.0, 0.0, 245.0),
    ];
    let surv_tsv = surv_rows.iter().map(|&(g, t, s, b)| format!("{g}\t{t}\t{s}\t{b}")).collect::<Vec<_>>().join("\n");
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard(&surv_tsv)?;

    // PruneRows every_k_rows = 2 (exercises stepper +3 then -2 back to 2 -> keeps even rows [0,2,4,6,8,10]).
    let prune_id = run.run_analyze_entry("Prune", "analyze.entry.transform.prune")?;
    run.open_cmd_t()?;
    run.click("edit.transform.prune.every_k.plus")?;
    run.click("edit.transform.prune.every_k.minus")?;
    run.commit_modal_ok()?;

    // Chain Descriptive Statistics (`edit.desc.normality` non-default off -> verify summary tab).
    run.select_sheet(&prune_id)?;
    let desc_id = run.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;

    // Chain Kaplan-Meier, bind Group/Time/Status MV roles via `edit.tab.data`, and enable Gehan-Breslow-Wilcoxon test (`edit.km.gehan`).
    run.select_sheet(&prune_id)?;
    let km_id = run.run_analyze_entry("Kaplan-Meier", "analyze.entry.stats.survival_km")?;
    run.open_cmd_t()?;
    run.click("edit.km.gehan")?;
    run.click("edit.tab.data")?;
    run.click("edit.km.time.1")?;
    run.click("edit.km.status.2")?;
    run.click("edit.km.group.0")?;
    run.commit_modal_ok()?;

    // Stage 4a: Verify chained Descriptive Statistics + Kaplan-Meier against headless Session oracle.
    let ref_surv = build_mv_survival_sheet("ref-wf06-surv", "MV Survival Cohort", &surv_rows)?;
    let prune_kind = serde_json::to_value(AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: false }))
        .map_err(|e| e.to_string())?;
    let desc_kind = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    let km_kind = serde_json::to_value(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox: true,
        gehan_breslow_wilcoxon: true,
        trend_test: false,
        asymmetrical_greenwood_ci: true,
        multiplicity: curvebook_doc::MultiplicitySpec::default(),
        mv_time_column: Some(1),
        mv_status_column: Some(2),
        mv_group_column: Some(0),
        event_code: None,
        censored_code: None,
        treat_other_as: Default::default(),
        ratio_median_ci: false,
        mv_status_columns: Vec::new(),
        mv_group_columns: Vec::new(),
    }))
    .map_err(|e| e.to_string())?;

    let ref_surv2 = ref_surv.clone();
    let prune_kind2 = prune_kind.clone();
    run.verify_reference(
        ReferenceSpec::for_result(desc_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf06-surv", "sheet_id": "ref-prune-wf06", "kind": prune_kind2 }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-prune-wf06", "sheet_id": "ref-desc-wf06", "kind": desc_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_surv2)?
        .ref_result_id(SheetId::new("ref-desc-wf06"))
        .scalar("Mean", Some(2), 1e-6, 1e-6)
        .tab_cell(0, 9, 2, 1e-6, 1e-6),
    )?;

    run.verify_reference(
        ReferenceSpec::for_result(km_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf06-surv", "sheet_id": "ref-prune2-wf06", "kind": prune_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-prune2-wf06", "sheet_id": "ref-km-wf06", "kind": km_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_surv)?
        .ref_result_id(SheetId::new("ref-km-wf06"))
        .scalar("Gehan-Breslow-Wilcoxon chi-square", None, 1e-6, 1e-6)
        .scalar("Median survival (Ctrl)", None, 1e-6, 1e-6),
    )?;

    // Create missing-data MV table via GUI and verify non-default Multiple Imputation (m = 6 via edit.mi.m.inc).
    let mi_table_id = run.create_table_via_welcome(
        "welcome.arch.multiple_variables",
        "Impute Cohort",
        &["welcome.stepper.cols.dec", "welcome.mv.role.0.Continuous", "welcome.mv.role.1.Continuous", "welcome.mv.role.2.Continuous"],
    )?;
    for (c_idx, title) in ["X1", "X2", "X3"].iter().enumerate() {
        run.enter_column_title(c_idx, title)?;
    }
    let mi_rows: [[Option<f64>; 3]; 6] = [
        [Some(10.0), Some(20.0), Some(30.0)],
        [Some(12.0), None, Some(36.0)],
        [Some(14.0), Some(28.0), Some(42.0)],
        [Some(16.0), Some(32.0), Some(48.0)],
        [Some(18.0), Some(36.0), Some(54.0)],
        [Some(20.0), Some(40.0), Some(60.0)],
    ];
    run.click("grid.cell.0.0.0")?;
    run.paste_clipboard("10\t20\t30\n12\t\t36\n14\t28\t42\n16\t32\t48\n18\t36\t54\n20\t40\t60")?;
    let mi_id = run.run_analyze_entry("Imputation", "analyze.entry.stats.multiple_imputation")?;
    run.open_cmd_t()?;
    run.click("edit.mi.m.inc")?;
    run.commit_modal_ok()?;
    let ref_mi = build_mv_missing_sheet("ref-wf06-mi", "Impute Cohort", &mi_rows)?;
    let mi_kind = serde_json::to_value(AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
        imputations_m: 6,
        max_iters: 10,
        seed: 42,
        response_column: None,
        predictor_columns: Vec::new(),
    }))
    .map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(mi_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-wf06-mi", "sheet_id": "ref-mi-wf06", "kind": mi_kind }))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_mi)?
        .ref_result_id(SheetId::new("ref-mi-wf06"))
        .scalar("Imputations (m)", None, 1e-9, 1e-9)
        .scalar("Total missing cells", None, 1e-9, 1e-9)
        .scalar("X2: Pooled mean", None, 1e-6, 1e-6),
    )?;
    let _ = (mi_table_id, mi_id, km_id);

    // Stage 5: Quantitative live edit (+60 on kept row 0 Biomarker -> exact +10.0 Biomarker Mean across 6 kept rows).
    let desc_check_id = desc_id.clone();
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(surv_id, 0, 3, 0, "160.0"),
        vec![
            LiveEditExpectation::tab_cell_changed(desc_id, 0, 9, 2),
            LiveEditExpectation::custom(move |before, after, doc| {
                let b_cell = before.tab_cell(&desc_check_id, 0, 9, 2).ok_or_else(|| String::from("missing Biomarker Mean before edit"))?;
                let a_cell = after.tab_cell(&desc_check_id, 0, 9, 2).ok_or_else(|| String::from("missing Biomarker Mean after edit"))?;
                let (ResultCell::Number { value: bv, .. }, ResultCell::Number { value: av, .. }) = (b_cell, a_cell) else {
                    return Err(format!("expected numeric Biomarker Mean cells, got {b_cell:?} / {a_cell:?}"));
                };
                if ((av - bv) - 10.0).abs() > 1e-6 {
                    return Err(format!("expected Biomarker Mean delta = +10.0, got {bv} -> {av}"));
                }
                let rs = doc.result_sheet(&desc_check_id).ok_or_else(|| String::from("missing desc ResultSheet"))?;
                if rs.status != ResultStatus::UpToDate {
                    return Err(format!("expected UpToDate ResultStatus, got {:?}", rs.status));
                }
                Ok(())
            }),
        ],
    )?;
    let _ = prune_id;
    run.finish()
}

// ---------------------------------------------------------------------------
// R20.4: Five-Stage Dose-Response Split-View Acceptance Proof (pitfalls.md #7)
// ---------------------------------------------------------------------------

#[test]
fn r20_4_five_stage_dose_response_split_view_acceptance() -> TestRes {
    use curvebook_ui_egui::egui_shell::driver::{panel_header_baseline_parity, split_targeting_parity, split_unified_datagrid_parity};

    // Stage 1: Empty UiApp -> XY dose-response table via Welcome Hub -> paste 4PL dataset.
    let mut run = WorkflowRun::start("R20.4-Split-4PL");
    let fx = dose_response_4pl();
    let graphs_before = graph_id_set(run.document());
    let xy_id = run.create_table_via_welcome("welcome.arch.xy", "Dose-Response Split 4PL", &["welcome.stepper.cols.dec", "welcome.subcol.raw"])?;
    let graph_id = diff_new_graph_for_source(run.document(), &graphs_before, &xy_id)?;

    run.enter_column_title(0, "Agonist_A")?;
    run.enter_column_title(1, "Agonist_B")?;
    let data_tsv: String = fx
        .rows
        .iter()
        .map(|row| {
            let mut cols = vec![format!("{}", row.x)];
            for series in &row.series_replicates {
                for &v in series {
                    cols.push(format!("{v}"));
                }
            }
            cols.join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n");
    run.click("grid.xcell.0.0")?;
    run.paste_clipboard(&data_tsv)?;

    // Stage 2: Run 4PL nonlinear curve fit through Analyze modal.
    let fit_id = run.run_analyze_entry("nonlinear", "analyze.entry.fit.nonlinear")?;
    run.open_cmd_t()?;
    run.click("fit.tab.model")?;
    run.click("fit.eq.item.log_agonist_vs_response_4pl")?;
    run.commit_modal_ok()?;

    // Stage 3: Select XY DataSheet, open default `Data + Graph` split (auto-pinned Graph Inspector, zero manual pinning).
    run.select_sheet(&xy_id)?;
    run.click("toolbar.split")?;
    run.click("toolbar.split.preset.data_graph")?;

    ensure(run.app().split.enabled, "R20.4 split view must be enabled after Data + Graph preset")?;
    ensure(run.app().split.inspector_pinned, "Data + Graph preset must auto-pin Graph Inspector with zero manual pinning")?;
    ensure(
        run.document().active_sheet_id.as_ref() == Some(&xy_id),
        format!("active sheet must remain XY DataSheet `{}` in Data + Graph split, got {:?}", xy_id.as_str(), run.document().active_sheet_id),
    )?;
    ensure(
        run.app().split.effective_inspector_target(run.document()) == Some(&graph_id),
        format!(
            "effective inspector target must be auto-pinned GraphSheet `{}`, got {:?}",
            graph_id.as_str(),
            run.app().split.effective_inspector_target(run.document())
        ),
    )?;

    // Verify all three R20.3/R20.4 shell parity gates in split view.
    run.assert_shell_health("r20_4_split_data_graph")?;
    let hdr_v = panel_header_baseline_parity(run.frame_meta());
    ensure(hdr_v.is_empty(), format!("panel_header_baseline_parity violations: {hdr_v:?}"))?;
    let tgt_v = split_targeting_parity(run.app());
    ensure(tgt_v.is_empty(), format!("split_targeting_parity violations: {tgt_v:?}"))?;
    let grid_v = split_unified_datagrid_parity(run.frame_meta());
    ensure(grid_v.is_empty(), format!("split_unified_datagrid_parity violations: {grid_v:?}"))?;

    // Stage 4: D1 oracle parity against fresh headless Session.
    let mut ref_sheet = fx.to_data_sheet("ref-r20-4").map_err(|e| e.to_string())?;
    ref_sheet.title = String::from("Dose-Response Split 4PL");
    let ref_nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    let nl_kind = serde_json::to_value(AnalysisKind::NonlinearRegression(ref_nl)).map_err(|e| e.to_string())?;

    run.verify_reference(
        ReferenceSpec::for_result(fit_id.clone(), move |sess| {
            sess.execute(
                "analysis.run",
                json!({
                    "source_sheet_id": "ref-r20-4",
                    "sheet_id": "ref-fit-r20-4",
                    "kind": nl_kind
                }),
            )
            .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-g-r20-4", "data_sheet_id": "ref-r20-4", "kind": "xy" }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-fit-r20-4"))
        .with_graph(graph_id.clone())
        .ref_graph_id(SheetId::new("ref-g-r20-4"))
        .scalar("LogEC50", Some(0), 1e-4, 1e-4)
        .scalar("HillSlope", Some(0), 1e-4, 1e-4)
        .scalar("R2", None, 1e-6, 1e-6),
    )?;

    // Stage 5a: Edit a cell directly in the live split DataGrid -> live 4PL refit + SVG delta while split & pin stay intact.
    let fit_check_id = fit_id.clone();
    run.live_edit_all(
        LiveEditTarget::gui(|wf| wf.enter_grid_cell(4, 0, 0, "45.0")),
        vec![
            LiveEditExpectation::scalar_changed(fit_id.clone(), "LogEC50", Some(0), 1e-4),
            LiveEditExpectation::scalar_changed(fit_id, "R2", None, 1e-5),
            LiveEditExpectation::graph_svg_changed(graph_id.clone()),
            LiveEditExpectation::custom(move |_before, _after, doc| {
                let rs = doc.result_sheet(&fit_check_id).ok_or_else(|| String::from("missing 4PL ResultSheet after split edit"))?;
                if rs.status != ResultStatus::UpToDate {
                    return Err(format!("expected UpToDate 4PL fit after split DataGrid edit, got {:?}", rs.status));
                }
                Ok(())
            }),
        ],
    )?;

    // Stage 5b: Adjust Graph Inspector controls while DataSheet stays active and GraphSheet remains auto-pinned.
    ensure(run.app().split.enabled && run.app().split.inspector_pinned, "split and auto-pin must persist after live DataGrid edit")?;
    ensure(run.document().active_sheet_id.as_ref() == Some(&xy_id), "active sheet must remain XY DataSheet before Graph Inspector edit")?;
    run.click("inspector.graph.shape.Square")?;
    let gs_after = run.document().graph_sheet(&graph_id).ok_or_else(|| String::from("missing GraphSheet after inspector edit"))?;
    let first_marker = gs_after.spec.series.first().and_then(|s| match &s.gdo {
        curvebook_doc::GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } => Some(m.shape),
        _ => None,
    });
    ensure(
        first_marker == Some(MarkerShape::Square),
        format!("expected Graph Inspector shape control to mutate pinned GraphSheet `{}` to Square, got {first_marker:?}", graph_id.as_str()),
    )?;
    ensure(
        run.document().active_sheet_id.as_ref() == Some(&xy_id),
        "adjusting pinned Graph Inspector control must NOT steal active_sheet_id from DataSheet",
    )?;
    ensure(
        run.app().split.effective_inspector_target(run.document()) == Some(&graph_id),
        "Graph Inspector target must remain pinned to GraphSheet after control edit",
    )?;

    run.finish()
}
