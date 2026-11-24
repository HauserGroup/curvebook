#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

//! Self-tests enforcing the five-stage `WorkflowRun` contract (`R10.26`).
//!
//! Verifies both:
//! 1. Positive end-to-end 5-stage `WorkflowRun` passing against a headless `Session` oracle (`D1`),
//!    live upstream edit recomputation, and `finish()` (`assert_zero_shortcut_compliance`,
//!    14-locale §9.4 sweep, input-to-paint budget).
//! 2. Negative contract failures: deliberately failing `live_edit`, `Failed` `ResultSheet`,
//!    accumulated `intent_errors` (even after a later commit succeeds), missing ordered stages,
//!    and background-thread panics must each fail `finish()`.

use std::sync::LazyLock;

use crate::human_wf_common::TestRes;
use crate::human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, NonlinearFitSpec, OneWayAnovaSpec, ResultStatus, SheetId, SubcolumnMode,
    TableArchetype, XyTable,
};
use curvebook_platform::sync::RwLock;
use curvebook_testkit::wf_datasets;
use serde_json::json;

/// Isolates background-panic self-testing from parallel tests within this binary.
static SELF_TEST_GATE: LazyLock<RwLock<()>> = LazyLock::new(|| RwLock::named("r10_26_self_test_gate", ()));

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
            sess.execute(
                "analysis.run",
                json!({
                    "source_sheet_id": "ref-col",
                    "kind": kind_val,
                }),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
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

    let err_change = run
        .live_edit(
            LiveEditTarget::sheet_grid_cell(ds_id.clone(), 0, 2, 0, "34.5"),
            LiveEditExpectation::scalar_changed(rs_id.clone(), "F", None, 1e9),
        )
        .expect_err("live_edit with impossible min_delta must fail");
    if !err_change.contains("to change") {
        return Err(format!("unexpected error from failing live_edit: {err_change}"));
    }

    let err_inv = run
        .live_edit(LiveEditTarget::sheet_grid_cell(ds_id, 1, 2, 0, "35.0"), LiveEditExpectation::scalar_invariant(rs_id, "F", None, 1e-9, 1e-9))
        .expect_err("live_edit expecting invariance when F shifts must fail");
    if !err_inv.contains("invariant violated") {
        return Err(format!("unexpected error from failing invariant live_edit: {err_inv}"));
    }

    let err_finish = run.finish().expect_err("finish must fail when live_edit did not succeed");
    if !err_finish.contains("LiveEdit") {
        return Err(format!("unexpected finish error after failed live_edit: {err_finish}"));
    }
    Ok(())
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

    // Corrupt upstream groups via GUI until One-Way ANOVA cannot form >= 2 groups -> ResultStatus::Failed.
    run.select_sheet(&ds_id)?;
    for c in 1..3 {
        for r in 0..6 {
            run.enter_grid_cell(r, c, 0, "")?;
        }
    }
    let is_failed = run.document().result_sheet(&rs_id).is_some_and(|r| matches!(r.status, ResultStatus::Failed { .. }));
    if !is_failed {
        return Err(format!("expected ResultSheet to enter Failed state, got {:?}", run.document().result_sheet(&rs_id).map(|r| &r.status)));
    }

    let err = run.finish().expect_err("finish must fail when a ResultSheet is Failed");
    if !err.contains("Failed ResultSheet") {
        return Err(format!("unexpected finish error for Failed ResultSheet: {err}"));
    }
    Ok(())
}

#[test]
fn neg_intent_error_fails_finish_even_after_later_success() -> TestRes {
    let _guard = SELF_TEST_GATE.read_long_job().map_err(|e| format!("{e}"))?;
    let mut run = WorkflowRun::start("WF-SELF-NEG-INTENT");
    let ds_id = run.create_table_via_welcome("welcome.arch.xy", "Dose Self Test", &[])?;
    let rows = [(-9.0, 5.1), (-8.0, 6.0), (-7.0, 8.4), (-6.0, 28.2), (-5.0, 76.5), (-4.0, 97.1)];
    for (r, (x, y)) in rows.into_iter().enumerate() {
        run.enter_x_cell(r, 0, &format!("{x}"))?;
        run.enter_grid_cell(r, 0, 0, &format!("{y}"))?;
    }
    let fit_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;

    // Trigger an EngineError through the GUI (invalid R² threshold > 1.0) using raw click,
    // then recover cleanly with "0.9" so `last_intent_error` clears while `intent_errors` stays non-empty.
    run.select_sheet(&fit_id)?;
    run.open_cmd_t()?;
    run.click("fit.tab.output")?;
    run.replace_textbox("fit.diagnostics.r2.input", "1.1")?;
    run.click("modal.ok")?;
    if run.intent_errors().is_empty() {
        return Err(String::from("expected invalid R² threshold GUI commit to append to intent_errors"));
    }
    run.replace_textbox("fit.diagnostics.r2.input", "0.9")?;
    run.commit_modal_ok()?;
    if run.state().last_intent_error.is_some() {
        return Err(format!("expected valid follow-up commit to clear last_intent_error, got {:?}", run.state().last_intent_error));
    }

    let mut ref_xy = XyTable::new(SubcolumnMode::Single, 1, rows.len()).map_err(|e| e.to_string())?;
    for (r, (x, y)) in rows.into_iter().enumerate() {
        ref_xy.x_cells[r][0] = Cell::number(x);
        ref_xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ref_sheet = DataSheet::new("ref-xy", "Dose Self Test", TableArchetype::Xy(ref_xy)).map_err(|e| e.to_string())?;
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

    let err = run.finish().expect_err("finish must fail when accumulated intent_errors is non-empty");
    if !err.contains("intent error") {
        return Err(format!("unexpected finish error for intent_errors: {err}"));
    }
    Ok(())
}

#[test]
fn neg_missing_or_out_of_order_stages_fail_finish() -> TestRes {
    let _guard = SELF_TEST_GATE.read_long_job().map_err(|e| format!("{e}"))?;

    // Missing Create
    let r0 = WorkflowRun::start("WF-SELF-S0");
    let e0 = r0.finish().expect_err("finish at stage 0 must fail");
    if !e0.contains("Create") {
        return Err(format!("unexpected stage-0 finish error: {e0}"));
    }

    // Missing Enter (+ cannot call run_analyze_entry before Enter)
    let mut r1 = WorkflowRun::start("WF-SELF-S1");
    let _ = r1.create_table_via_welcome("welcome.arch.column", "Empty Col", &[])?;
    let e_skip_enter = r1.run_analyze_entry("One-Way", "analyze.entry.stats.one_way_anova").expect_err("run_analyze_entry before Enter must fail");
    if !e_skip_enter.contains("Enter") {
        return Err(format!("unexpected skip-Enter error: {e_skip_enter}"));
    }
    let e1 = r1.finish().expect_err("finish without Enter must fail");
    if !e1.contains("Enter") {
        return Err(format!("unexpected stage-1 finish error: {e1}"));
    }

    // Missing Configure
    let mut r2 = WorkflowRun::start("WF-SELF-S2");
    let _ = r2.create_table_via_welcome("welcome.arch.column", "No Config", &[])?;
    r2.enter_grid_cell(0, 0, 0, "10.0")?;
    let e2 = r2.finish().expect_err("finish without Configure must fail");
    if !e2.contains("Configure") {
        return Err(format!("unexpected stage-2 finish error: {e2}"));
    }

    // Missing Verify (+ cannot call live_edit before verify_reference)
    let (mut r3, _ds3, rs3) = setup_column_anova_through_configure("WF-SELF-S3")?;
    let e_skip_verify = r3
        .live_edit(LiveEditTarget::grid_cell(0, 2, 0, "34.5"), LiveEditExpectation::scalar_changed(rs3, "F", None, 0.1))
        .expect_err("live_edit before Verify must fail");
    if !e_skip_verify.contains("verify_reference") {
        return Err(format!("unexpected skip-Verify error: {e_skip_verify}"));
    }
    let e3 = r3.finish().expect_err("finish without Verify must fail");
    if !e3.contains("Verify") {
        return Err(format!("unexpected stage-3 finish error: {e3}"));
    }

    // Missing LiveEdit
    let (mut r4, _ds4, rs4) = setup_column_anova_through_configure("WF-SELF-S4")?;
    verify_column_anova(&mut r4, &rs4)?;
    let e4 = r4.finish().expect_err("finish without LiveEdit must fail");
    if !e4.contains("LiveEdit") {
        return Err(format!("unexpected stage-4 finish error: {e4}"));
    }
    Ok(())
}

#[test]
fn neg_background_thread_panic_fails_finish() -> TestRes {
    let _guard = SELF_TEST_GATE.write_long_job().map_err(|e| format!("{e}"))?;
    let (mut run, ds_id, rs_id) = setup_column_anova_through_configure("WF-SELF-NEG-PANIC")?;
    verify_column_anova(&mut run, &rs_id)?;
    run.live_edit(LiveEditTarget::sheet_grid_cell(ds_id, 0, 2, 0, "34.5"), LiveEditExpectation::scalar_changed(rs_id, "F", None, 0.1))?;

    let join = std::thread::Builder::new()
        .name(String::from("cb-canvas-raster"))
        .spawn(|| {
            panic!("synthetic cb-canvas-raster worker panic for R10.26 self-test");
        })
        .map_err(|e| format!("failed to spawn panic worker thread: {e}"))?;
    let _ = join.join();

    let err = run.finish().expect_err("finish must fail after background-thread panic");
    if !err.contains("background-thread panic") {
        return Err(format!("unexpected finish error after bg panic: {err}"));
    }
    Ok(())
}

#[allow(dead_code)]
fn _unused_helper_type_check(_s: ColumnTable) {
    let _ = ColumnDesignHint::UnpairedReplicates;
}
