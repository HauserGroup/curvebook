#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Self-tests enforcing the five-stage `WorkflowRun` contract (`R10.26`).

use std::sync::LazyLock;

use crate::common::{TestRes, ensure, seed_xy_doc};
use crate::human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use curvebook_doc::{AnalysisKind, NonlinearFitSpec, OneWayAnovaSpec, ResultStatus, SheetId};
use curvebook_platform::sync::RwLock;
use curvebook_testkit::wf_datasets;
use serde_json::json;

static SELF_TEST_GATE: LazyLock<RwLock<()>> = LazyLock::new(|| RwLock::named("r10_26_self_test_gate", ()));

fn expect_err_contains<T: std::fmt::Debug>(res: Result<T, String>, needle: &str, ctx: &str) -> TestRes {
    let err = res.expect_err(ctx);
    ensure(err.contains(needle), format!("{ctx}: expected error containing `{needle}`, got `{err}`"))
}

fn setup_column_anova_through_configure(wf_id: &str) -> Result<(WorkflowRun, SheetId, SheetId), String> {
    let mut run = WorkflowRun::start(wf_id);
    let col_fx = wf_datasets::column_three_group();
    let ds_id = run.create_table_via_welcome("welcome.arch.column", col_fx.title, &[])?;
    for (c, title) in col_fx.group_titles.iter().enumerate() {
        run.enter_column_title(c, title)?;
        for (r, &v) in col_fx.columns[c].iter().enumerate() {
            run.enter_grid_cell(r, c, 0, &format!("{v}"))?;
        }
    }
    let rs_id = run.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova")?;
    Ok((run, ds_id, rs_id))
}

fn verify_column_anova(run: &mut WorkflowRun, rs_id: &SheetId) -> TestRes {
    let col_sheet = wf_datasets::column_three_group().to_data_sheet("ref-col").map_err(|e| e.to_string())?;
    let kind_val = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(rs_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-col", "kind": kind_val })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(col_sheet)?
        .scalar("F", None, 1e-9, 1e-9)
        .scalar("P value", None, 1e-12, 1e-12)
        .scalar("R squared", None, 1e-9, 1e-9)
        .scalar("Eta squared", None, 1e-9, 1e-9),
    )
}

#[test]
fn pos_five_stage_workflow_run_passes_and_verifies_oracle() -> TestRes {
    let _guard = SELF_TEST_GATE.read_long_job().map_err(|e| format!("{e}"))?;
    let (mut run, ds_id, rs_id) = setup_column_anova_through_configure("WF-SELF-POS")?;
    verify_column_anova(&mut run, &rs_id)?;
    run.live_edit(LiveEditTarget::sheet_grid_cell(ds_id, 0, 2, 0, "34.5"), LiveEditExpectation::scalar_changed(rs_id, "F", None, 0.1))?;
    run.finish()
}

#[test]
fn neg_deliberately_failing_live_edit_fails_and_blocks_finish() -> TestRes {
    let _guard = SELF_TEST_GATE.read_long_job().map_err(|e| format!("{e}"))?;
    let (mut run, ds_id, rs_id) = setup_column_anova_through_configure("WF-SELF-NEG-LIVE")?;
    verify_column_anova(&mut run, &rs_id)?;
    expect_err_contains(
        run.live_edit(
            LiveEditTarget::sheet_grid_cell(ds_id.clone(), 0, 2, 0, "34.5"),
            LiveEditExpectation::scalar_changed(rs_id.clone(), "F", None, 1e9),
        ),
        "to change",
        "impossible min_delta",
    )?;
    expect_err_contains(
        run.live_edit(LiveEditTarget::sheet_grid_cell(ds_id, 1, 2, 0, "35.0"), LiveEditExpectation::scalar_invariant(rs_id, "F", None, 1e-9, 1e-9)),
        "invariant violated",
        "violated invariance",
    )?;
    expect_err_contains(run.finish(), "LiveEdit", "finish after failed live_edit")
}

#[test]
fn neg_failed_result_sheet_fails_finish() -> TestRes {
    let _guard = SELF_TEST_GATE.read_long_job().map_err(|e| format!("{e}"))?;
    let (mut run, ds_id, rs_id) = setup_column_anova_through_configure("WF-SELF-NEG-FAILED")?;
    verify_column_anova(&mut run, &rs_id)?;
    run.live_edit(
        LiveEditTarget::sheet_grid_cell(ds_id.clone(), 0, 2, 0, "34.5"),
        LiveEditExpectation::scalar_changed(rs_id.clone(), "F", None, 0.1),
    )?;

    run.select_sheet(&ds_id)?;
    for c in 1..3 {
        for r in 0..6 {
            run.enter_grid_cell(r, c, 0, "")?;
        }
    }
    ensure(run.document().result_sheet(&rs_id).is_some_and(|r| matches!(r.status, ResultStatus::Failed { .. })), "expected Failed ResultSheet")?;
    expect_err_contains(run.finish(), "Failed ResultSheet", "finish with Failed ResultSheet")
}

#[test]
fn neg_intent_error_fails_finish_even_after_later_success() -> TestRes {
    let _guard = SELF_TEST_GATE.read_long_job().map_err(|e| format!("{e}"))?;
    let mut run = WorkflowRun::start("WF-SELF-NEG-INTENT");
    let ds_id = run.create_table_via_welcome("welcome.arch.xy", "Dose Self Test", &[])?;
    let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let ys = [5.1, 6.0, 8.4, 28.2, 76.5, 97.1];
    for r in 0..xs.len() {
        run.enter_x_cell(r, 0, &format!("{}", xs[r]))?;
        run.enter_grid_cell(r, 0, 0, &format!("{}", ys[r]))?;
    }
    let fit_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;

    // Invalid R² threshold (> 1.0) appends to `intent_errors`; valid follow-up clears `last_intent_error`.
    run.select_sheet(&fit_id)?;
    run.open_cmd_t()?;
    run.click("fit.tab.output")?;
    run.replace_textbox("fit.diagnostics.r2.input", "1.1")?;
    run.click("modal.ok")?;
    ensure(!run.intent_errors().is_empty(), "invalid R² commit must append to intent_errors")?;
    run.replace_textbox("fit.diagnostics.r2.input", "0.9")?;
    run.commit_modal_ok()?;
    ensure(run.state().last_intent_error.is_none(), "valid follow-up must clear last_intent_error")?;

    let (ref_doc, ref_id) = seed_xy_doc("Ref", "ref-xy", "Dose Self Test", "X", &xs, &[("Y", &ys)])?;
    let ref_sheet = ref_doc.data_sheet(&ref_id).unwrap().clone();
    let kind_val =
        serde_json::to_value(AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).map_err(|e| e.to_string())?;
    run.verify_reference(
        ReferenceSpec::for_result(fit_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-xy", "kind": kind_val })).map(|_| ()).map_err(|e| e.to_string())
        })
        .with_reference_sheet(ref_sheet)?
        .scalar("R2", None, 1e-9, 1e-9),
    )?;
    run.live_edit(LiveEditTarget::sheet_grid_cell(ds_id, 2, 0, 0, "14.5"), LiveEditExpectation::scalar_changed(fit_id, "R2", None, 1e-4))?;
    expect_err_contains(run.finish(), "intent error", "finish with accumulated intent_errors")
}

#[test]
fn neg_missing_or_out_of_order_stages_fail_finish() -> TestRes {
    let _guard = SELF_TEST_GATE.read_long_job().map_err(|e| format!("{e}"))?;
    expect_err_contains(WorkflowRun::start("WF-SELF-S0").finish(), "Create", "stage 0")?;

    let mut r1 = WorkflowRun::start("WF-SELF-S1");
    let _ = r1.create_table_via_welcome("welcome.arch.column", "Empty Col", &[])?;
    expect_err_contains(r1.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova"), "Enter", "skip Enter")?;
    expect_err_contains(r1.finish(), "Enter", "stage 1")?;

    let mut r2 = WorkflowRun::start("WF-SELF-S2");
    let _ = r2.create_table_via_welcome("welcome.arch.column", "No Config", &[])?;
    r2.enter_grid_cell(0, 0, 0, "10.0")?;
    expect_err_contains(r2.finish(), "Configure", "stage 2")?;

    let (mut r3, _, rs3) = setup_column_anova_through_configure("WF-SELF-S3")?;
    expect_err_contains(
        r3.live_edit(LiveEditTarget::grid_cell(0, 2, 0, "34.5"), LiveEditExpectation::scalar_changed(rs3, "F", None, 0.1)),
        "verify_reference",
        "skip Verify",
    )?;
    expect_err_contains(r3.finish(), "Verify", "stage 3")?;

    let (mut r4, _, rs4) = setup_column_anova_through_configure("WF-SELF-S4")?;
    verify_column_anova(&mut r4, &rs4)?;
    expect_err_contains(r4.finish(), "LiveEdit", "stage 4")
}

#[test]
fn neg_background_thread_panic_fails_finish() -> TestRes {
    let _guard = SELF_TEST_GATE.write_long_job().map_err(|e| format!("{e}"))?;
    let (mut run, ds_id, rs_id) = setup_column_anova_through_configure("WF-SELF-NEG-PANIC")?;
    verify_column_anova(&mut run, &rs_id)?;
    run.live_edit(LiveEditTarget::sheet_grid_cell(ds_id, 0, 2, 0, "34.5"), LiveEditExpectation::scalar_changed(rs_id, "F", None, 0.1))?;
    let join = std::thread::Builder::new()
        .name("cb-canvas-raster".into())
        .spawn(|| panic!("synthetic cb-canvas-raster worker panic for R10.26 self-test"))
        .map_err(|e| format!("spawn panic thread: {e}"))?;
    let _ = join.join();
    expect_err_contains(run.finish(), "background-thread panic", "finish after bg panic")
}
