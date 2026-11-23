#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression tests for verified Delta Audit Section DA-02 findings (`DA-02-01`..`DA-02-07`).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, FitMethod, MultipleVariablesTable, NestedTable, OneWayMethod,
    ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TwoGroupMethod, VariableRole, XyTable,
};
use curvebook_engine::services::{ExportArtifactFormat, ExportArtifactRequest, ExportTarget, RasterSizingPolicy, export_document_artifact};
use curvebook_engine::{BatchCommandStep, Caller, ExternalPathPolicy, Session};
use curvebook_render::{MAX_CAPPED_RASTER_DIM_PX, MAX_RASTER_PIXELS};
use serde_json::{Value, json};

static SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    root: PathBuf,
    workdir: PathBuf,
    outside: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-da02-{label}-{}-{id}", std::process::id()));
        let workdir = root.join("workdir");
        let outside = root.join("outside");
        fs::create_dir_all(&workdir).expect("create workdir");
        fs::create_dir_all(&outside).expect("create outside");
        Self { root, workdir, outside }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> Value {
    session.execute(id, params).unwrap_or_else(|e| panic!("command `{id}` unexpectedly failed: {e}")).payload
}

fn err_cmd(session: &mut Session, id: &str, params: Value) -> curvebook_engine::EngineError {
    session.execute(id, params).expect_err(&format!("expected `{id}` to fail"))
}

fn make_col_sheet(id: &str, cols: usize, rows: usize) -> DataSheet {
    let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows).unwrap();
    for (c, ds) in t.data_sets.iter_mut().enumerate() {
        for (r, row_cells) in ds.cells.iter_mut().enumerate() {
            if let Some(cell) = row_cells.first_mut() {
                *cell = Cell::number(10.0 + (c as f64) * 3.0 + (r as f64) * 0.5);
            }
        }
    }
    DataSheet::new(id, "Columns", TableArchetype::Column(t)).unwrap()
}

fn make_pvalue_col_sheet(id: &str) -> DataSheet {
    let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5).unwrap();
    for (r, p) in [0.001, 0.015, 0.04, 0.2, 0.6].iter().enumerate() {
        if let Some(cell) = t.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut()) {
            *cell = Cell::number(*p);
        }
    }
    DataSheet::new(id, "PValues", TableArchetype::Column(t)).unwrap()
}

fn make_dose_xy_sheet(id: &str) -> DataSheet {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).unwrap();
    let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0];
    let ys = [10.0, 12.0, 22.0, 52.0, 84.0, 96.0, 99.0, 100.0];
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        if let Some(xc) = xy.x_cells.get_mut(r).and_then(|row| row.first_mut()) {
            *xc = Cell::number(x);
        }
        if let Some(yc) = xy.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut()) {
            *yc = Cell::number(y);
        }
    }
    DataSheet::new(id, "Dose", TableArchetype::Xy(xy)).unwrap()
}

fn make_mv_sheet(id: &str) -> DataSheet {
    let mut mv = MultipleVariablesTable::new(4, 8).unwrap();
    for (c, (title, role)) in [
        ("Time", VariableRole::Continuous),
        ("Status", VariableRole::Continuous),
        ("X1", VariableRole::Continuous),
        ("Group", VariableRole::Categorical),
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(var) = mv.variables.get_mut(c) {
            var.title = String::from(title);
            var.role = role;
            var.role_inferred = false;
            for (r, cell) in var.cells.iter_mut().enumerate() {
                *cell = match c {
                    0 => Cell::number((r + 1) as f64 * 2.0),
                    1 => Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 }),
                    2 => Cell::number(5.0 + (r as f64) * 1.3 + if r % 3 == 0 { 0.4 } else { -0.2 }),
                    _ => Cell::text(if r < 4 { "A" } else { "B" }),
                };
            }
        }
    }
    DataSheet::new(id, "MV", TableArchetype::MultipleVariables(mv)).unwrap()
}

#[test]
fn da02_01_retired_aliases_rejected_and_overrides_applied() {
    let mut doc = ProjectDocument::new("DA-02-01").unwrap();
    doc.add_data_sheet(make_col_sheet("col3", 3, 6)).unwrap();
    doc.add_data_sheet(make_pvalue_col_sheet("pvals")).unwrap();
    doc.add_data_sheet(make_dose_xy_sheet("xy1")).unwrap();
    doc.add_data_sheet(make_mv_sheet("mv1")).unwrap();
    let mut session = Session::from_document(doc);

    // 1. Pruned retired aliases across analysis families must fail with BadInput instead of silently defaulting.
    let retired_cases = [
        // Multiplicity aliases
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "threshold": 0.01 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "post_hoc": "tukey" })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "multiplicity_method": "tukey" })),
        // PValueStack aliases
        ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "alpha": 0.01 })),
        ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "q": 0.01 })),
        ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "fdr_q": 0.01 })),
        ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "pvalue_stack_threshold": 0.01 })),
        ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "pvalue_stack_method": "bonferroni" })),
        ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "multiplicity_method": "bonferroni" })),
        ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "multiplicity": "bonferroni" })),
        ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "pvalue_stack_column": 0 })),
        // Two-group aliases
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_sample_t", "hypothetical_mean": 5.0 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "unpaired_t", "group_column": 3 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "unpaired_t", "response_column": 2 })),
        // Contingency / Survival / GLM / Power / Multivariate aliases
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "log_rank": false })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "gehan": true })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "time_column": 0 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "status_column": 1 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "group_column": 3 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "cox_ph", "cox_covariate_columns": [2] })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "glm", "glm_family": "linear" })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "n": 20 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "n2": 20 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "standardized_effect": 0.5 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "df": 2 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "anova_groups": 3 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "correlation_matrix", "correlation_all_pairs": true })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "correlation_matrix", "correlation_heatmap": true })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "clustering", "distance_metric": "euclidean" })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "clustering", "linkage_method": "ward" })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "m": 5 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "num_imputations": 5 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "iterations": 5 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "target_column": 0 })),
        ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "predictors": [2] })),
        // Nonlinear aliases
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "fit_method": "rout" })),
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "model_comparison": "aicc" })),
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "normality_tests": true })),
        (
            "fit.run",
            json!({ "source_sheet_id": "xy1", "kind": "4pl", "comparison": "aicc", "alternative_equation_id": "log_agonist_vs_response_3pl", "alternative_model": "3pl" }),
        ),
        // Transform aliases
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "math", "preset": "log10" })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "math", "operation": "log10" })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "math", "propagate_error": "delta" })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "normalize", "zero": "smallest" })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "normalize", "hundred": "largest" })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "prune_rows", "step": 2 })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "prune_rows", "average_dropped": true })),
        ("transform.run", json!({ "source_sheet_id": "mv1", "kind": "select_and_transform", "derived": [] })),
        ("transform.run", json!({ "source_sheet_id": "mv1", "kind": "select_and_transform", "column_formulas": [] })),
        ("transform.run", json!({ "source_sheet_id": "mv1", "kind": "select_and_transform", "formulas": [] })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "simulate", "n_points": 20 })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "simulate", "params": { "Top": 100.0 } })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "random_sample", "n_samples": 4 })),
    ];
    for (cmd, payload) in retired_cases {
        let err = err_cmd(&mut session, cmd, payload.clone());
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for {cmd} {payload}, got {err}");
    }

    // 2. Nonlinear ROUT Q: both `fdr_q_percent` and `q_percent` are parsed and applied; conflicts fail.
    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "nl-rout", "source_sheet_id": "xy1", "kind": "4pl", "method": "rout", "fdr_q_percent": 5.0 }),
    );
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("nl-rout")).unwrap().analysis.kind,
        AnalysisKind::NonlinearRegression(ref nl)
            if matches!(nl.method, FitMethod::RoutOutlierElimination { fdr_q_percent } if (fdr_q_percent - 5.0).abs() < 1e-12)
    ));
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "nl-rout", "fdr_q_percent": 3.5 }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("nl-rout")).unwrap().analysis.kind,
        AnalysisKind::NonlinearRegression(ref nl)
            if matches!(nl.method, FitMethod::RoutOutlierElimination { fdr_q_percent } if (fdr_q_percent - 3.5).abs() < 1e-12)
    ));
    let conflict = err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "nl-rout", "q_percent": 2.0, "fdr_q_percent": 4.0 }));
    assert_eq!(conflict.code, ErrorCode::BadInput);

    // 3. `analysis.updateParams` updates `hypothetical` (OneSampleT/Wilcoxon) and `geisser_greenhouse` (RM ANOVA).
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "os-t", "source_sheet_id": "col3", "kind": "one_sample_t", "hypothetical": 1.0 }));
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "os-t", "hypothetical": 12.5 }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("os-t")).unwrap().analysis.kind,
        AnalysisKind::TwoGroupComparison(ref tg)
            if matches!(tg.method, TwoGroupMethod::OneSampleT { hypothetical } if (hypothetical - 12.5).abs() < 1e-12)
    ));

    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "rm-anova", "source_sheet_id": "col3", "kind": "rm_one_way_anova", "geisser_greenhouse": true }),
    );
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "rm-anova", "geisser_greenhouse": false }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("rm-anova")).unwrap().analysis.kind,
        AnalysisKind::OneWayAnova(ref spec) if spec.method == OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }
    ));

    // 4. `extract_clean_patch_map` rejects retired metadata aliases inside `params`.
    for retired_meta in [
        json!({ "sheet_id": "os-t", "params": { "confidence_level": 90.0 } }),
        json!({ "sheet_id": "os-t", "params": { "custom_title": "X" } }),
        json!({ "sheet_id": "os-t", "params": { "selected_columns": [0] } }),
        json!({ "sheet_id": "os-t", "params": { "columns": [0] } }),
    ] {
        assert_eq!(err_cmd(&mut session, "analysis.updateParams", retired_meta).code, ErrorCode::BadInput);
    }

    // 5. CorrelationMatrix accepts UI-compat booleans only at top-level of `analysis.updateParams`, rejecting nested/bad types.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "corr-1", "source_sheet_id": "mv1", "kind": "correlation_matrix" }));
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "corr-1", "correlation_all_pairs": true, "correlation_heatmap": false }));
    assert_eq!(
        err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "corr-1", "correlation_heatmap": "yes" })).code,
        ErrorCode::BadInput
    );
    assert_eq!(
        err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "corr-1", "params": { "correlation_heatmap": true } })).code,
        ErrorCode::BadInput
    );

    // 6. Error/remediation text references canonical `source_sheet_id`, never retired `source_sheet`.
    let missing_res = err_cmd(&mut session, "analysis.run", json!({ "sheet_id": "does-not-exist" }));
    assert!(missing_res.message().contains("source_sheet_id"));
    assert!(!missing_res.message().contains("`source_sheet`"));
}

#[test]
fn da02_02_nested_subcolumn_count_uses_biological_units() {
    let mut doc = ProjectDocument::new("DA-02-02 Nested").unwrap();
    // Case A: rows (12) > biological_units (3)
    let mut tall = NestedTable::new(2, 3, 12).unwrap();
    for g in &mut tall.groups {
        for r in 0..12 {
            for s in 0..3 {
                if let Some(cell) = g.cells.get_mut(r).and_then(|row| row.get_mut(s)) {
                    *cell = Cell::number((r * 10 + s) as f64);
                }
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("nest-tall", "Tall Nested", TableArchetype::Nested(tall)).unwrap()).unwrap();

    // Case B: rows (2) < biological_units (5)
    let mut wide = NestedTable::new(2, 5, 2).unwrap();
    for g in &mut wide.groups {
        for r in 0..2 {
            for s in 0..5 {
                if let Some(cell) = g.cells.get_mut(r).and_then(|row| row.get_mut(s)) {
                    *cell = Cell::number((r * 100 + s) as f64);
                }
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("nest-wide", "Wide Nested", TableArchetype::Nested(wide)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    // Case A: `table.clearCells { all: true }` must succeed (no OutOfBounds) and clear every cell.
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "nest-tall", "all": true }));
    let tall_after = session.document().data_sheet(&SheetId::new("nest-tall")).unwrap();
    for col in 0..2 {
        for row in 0..12 {
            for sub in 0..3 {
                assert!(tall_after.cell(CellCoord::new(row, col, sub)).unwrap().is_empty());
            }
        }
    }
    // Out-of-bounds subcolumn (3 >= 3 bio units, even though 3 < 12 rows) must be rejected.
    let oob = err_cmd(
        &mut session,
        "table.excludeCells",
        json!({
            "sheet_id": "nest-tall",
            "range": { "start_row": 0, "end_row": 0, "start_col": 0, "end_col": 0, "start_subcol": 3, "end_subcol": 3 },
            "excluded": true
        }),
    );
    assert_eq!(oob.code, ErrorCode::BadInput);

    // Case B: exclude cell in subcolumn 4 (>= 2 rows) and verify exclusionAuditReport sees it.
    ok_cmd(
        &mut session,
        "table.excludeCellsWithReason",
        json!({
            "sheet_id": "nest-wide",
            "coords": [{ "row": 1, "col": 0, "subcol": 4 }],
            "reason_tag": "outlier"
        }),
    );
    let audit = ok_cmd(&mut session, "table.exclusionAuditReport", json!({ "sheet_id": "nest-wide" }));
    assert_eq!(audit.get("total_excluded").and_then(Value::as_u64), Some(1));

    // `table.clearCells { all: true }` must also clear subcolumns 2..5 (previously missed when rows < bio_units).
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "nest-wide", "all": true }));
    let wide_after = session.document().data_sheet(&SheetId::new("nest-wide")).unwrap();
    for col in 0..2 {
        for row in 0..2 {
            for sub in 0..5 {
                assert!(wide_after.cell(CellCoord::new(row, col, sub)).unwrap().is_empty(), "expected (row={row}, col={col}, sub={sub}) cleared");
            }
        }
    }
}

#[test]
fn da02_03_atomic_batch_selection_save_open_and_undo_redo() {
    let tmp = TempDirGuard::new("batch");
    let save_path = tmp.workdir.join("project.curvebook");
    let save_path_2 = tmp.workdir.join("project-2.curvebook");
    let cancel = AtomicBool::new(false);

    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "s1", "title": "First", "kind": "column", "cols": 1, "rows": 2 }));
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "s2", "title": "Second", "kind": "column", "cols": 1, "rows": 2 }));
    ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "s2", "title": "Second v2" }));
    // Save cleanly so session starts clean with non-empty undo_stack and non-empty redo_stack.
    ok_cmd(&mut session, "edit.undo", json!({}));
    ok_cmd(&mut session, "project.save", json!({ "path": save_path.to_str().unwrap() }));
    assert!(!session.is_dirty());
    assert!(session.undo_depth() > 0);
    assert_eq!(session.redo_depth(), 1);
    let undo_before = session.undo_depth();
    let redo_before = session.redo_depth();

    // (1) Read-only `sheet.select` inside atomic batch must not push undo, clear redo, or dirty session.
    let select_steps = vec![BatchCommandStep::new("sheet.select", json!({ "sheet_id": "s1" }))];
    session.execute_batch_atomic(&select_steps, Caller::Ui, &cancel).unwrap();
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("s1"));
    assert!(!session.is_dirty(), "read-only sheet.select must not mark session dirty");
    assert_eq!(session.undo_depth(), undo_before, "read-only sheet.select must not push undo entry");
    assert_eq!(session.redo_depth(), redo_before, "read-only sheet.select must not clear redo stack");

    // (2a) Mutating batch ending with `project.save` collapses undo AND preserves `dirty == false`.
    let edit_and_save = vec![
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "s1", "title": "First A" })),
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "s1", "title": "First B" })),
        BatchCommandStep::new("project.save", json!({ "path": save_path.to_str().unwrap() })),
    ];
    session.execute_batch_atomic(&edit_and_save, Caller::Ui, &cancel).unwrap();
    assert!(!session.is_dirty(), "batch ending in project.save must remain clean");
    assert_eq!(session.undo_depth(), undo_before + 1);
    assert_eq!(session.redo_depth(), 0);

    // (2b) Save-only batch on an already-dirty session (same path) must clear dirty and preserve stacks.
    ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "s1", "title": "First Dirty" }));
    assert!(session.is_dirty());
    let dirty_undo = session.undo_depth();
    let save_same_path = vec![BatchCommandStep::new("project.save", json!({ "path": save_path.to_str().unwrap() }))];
    session.execute_batch_atomic(&save_same_path, Caller::Ui, &cancel).unwrap();
    assert!(!session.is_dirty(), "project.save to same path inside batch must clear dirty");
    assert_eq!(session.undo_depth(), dirty_undo);

    // (2c) Save-As batch to a new path updates file_path, stays clean, and does not push a spurious undo entry.
    let save_new_path = vec![BatchCommandStep::new("project.save", json!({ "path": save_path_2.to_str().unwrap() }))];
    session.execute_batch_atomic(&save_new_path, Caller::Ui, &cancel).unwrap();
    assert!(!session.is_dirty());
    assert_eq!(session.undo_depth(), dirty_undo);
    assert!(session.file_path().is_some());

    // (2d) Batch ending in `project.open { path }` preserves `dirty == false`.
    let edit_and_open = vec![
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "s1", "title": "Unsaved Edit" })),
        BatchCommandStep::new("project.open", json!({ "path": save_path.to_str().unwrap() })),
    ];
    session.execute_batch_atomic(&edit_and_open, Caller::Ui, &cancel).unwrap();
    assert!(!session.is_dirty(), "batch ending in project.open(path) must remain clean");

    // (2e) Batch with `project.save` followed by a mutation MUST be dirty.
    let save_then_edit = vec![
        BatchCommandStep::new("project.save", json!({ "path": save_path.to_str().unwrap() })),
        BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "s1", "title": "After Save" })),
    ];
    session.execute_batch_atomic(&save_then_edit, Caller::Ui, &cancel).unwrap();
    assert!(session.is_dirty(), "mutation after project.save inside batch must mark session dirty");

    // (3) `edit.undo` inside a batch must pop undo -> push redo cleanly (never invert/overwrite history).
    let doc_before_undo = session.document().clone();
    let undo_depth_pre = session.undo_depth();
    let undo_batch = vec![BatchCommandStep::new("edit.undo", json!({}))];
    session.execute_batch_atomic(&undo_batch, Caller::Ui, &cancel).unwrap();
    assert_eq!(session.undo_depth(), undo_depth_pre - 1, "edit.undo in batch must pop undo stack");
    assert_eq!(session.redo_depth(), 1, "edit.undo in batch must populate redo stack");

    let redo_batch = vec![BatchCommandStep::new("edit.redo", json!({}))];
    session.execute_batch_atomic(&redo_batch, Caller::Ui, &cancel).unwrap();
    assert_eq!(session.undo_depth(), undo_depth_pre);
    assert_eq!(session.redo_depth(), 0);
    assert_eq!(session.document(), &doc_before_undo);
}

#[test]
fn da02_04_capped_raster_export_bounds_and_zero_validation() {
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "xy1", "title": "Dose", "kind": "xy", "rows": 3, "cols": 1, "auto_graph": true }));
    ok_cmd(&mut session, "table.setXCells", json!({ "sheet_id": "xy1", "values": [1.0, 2.0, 3.0] }));
    ok_cmd(&mut session, "table.setCells", json!({ "sheet_id": "xy1", "rows": [[1.0], [2.0], [3.0]] }));
    let graph_id = session.document().graph_sheets.first().unwrap().id.clone();

    // Oversized explicit and single-axis requests under CapToLimits cap cleanly instead of failing.
    for (w, h) in [(Some(10_000), Some(10_000)), (Some(20_000), Some(1_000)), (Some(12_000), None), (None, Some(12_000))] {
        let art = export_document_artifact(
            session.document(),
            &ExportArtifactRequest {
                target: ExportTarget::Explicit(graph_id.clone()),
                format: ExportArtifactFormat::Png {
                    width_px: w,
                    height_px: h,
                    dpi: Some(300),
                    sizing: RasterSizingPolicy::CapToLimits { default_dpi: 150 },
                },
            },
            None,
            "test.da02_04",
        )
        .unwrap_or_else(|e| panic!("CapToLimits failed for ({w:?}, {h:?}): {e}"));
        assert!(art.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let meta = art.raster.unwrap();
        assert!(meta.width_px <= MAX_CAPPED_RASTER_DIM_PX);
        assert!(meta.height_px <= MAX_CAPPED_RASTER_DIM_PX);
        assert!(u64::from(meta.width_px) * u64::from(meta.height_px) <= MAX_RASTER_PIXELS);
    }

    // Zero width_px, height_px, and dpi rejected upfront with BadInput.
    for (w, h, dpi, default_dpi) in
        [(Some(0), Some(200), Some(150), 150), (Some(200), Some(0), Some(150), 150), (None, None, Some(0), 150), (None, None, None, 0)]
    {
        let err = export_document_artifact(
            session.document(),
            &ExportArtifactRequest {
                target: ExportTarget::Explicit(graph_id.clone()),
                format: ExportArtifactFormat::Png { width_px: w, height_px: h, dpi, sizing: RasterSizingPolicy::CapToLimits { default_dpi } },
            },
            None,
            "test.da02_04.zero",
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for ({w:?}, {h:?}, {dpi:?}, {default_dpi})");
    }
}

#[test]
fn da02_05_patch_analysis_kind_fields_handles_schema_nullable_non_option_fields() {
    let mut doc = ProjectDocument::new("DA-02-05").unwrap();
    doc.add_data_sheet(make_dose_xy_sheet("xy1")).unwrap();
    doc.add_data_sheet(make_pvalue_col_sheet("pvals")).unwrap();
    doc.add_data_sheet(make_col_sheet("col2", 2, 5)).unwrap();
    let mut session = Session::from_document(doc);

    // Schema-valid `null` on non-Option fields must act as no-op instead of serde_json null-type failure.
    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "fit-null", "source_sheet_id": "xy1", "kind": "4pl", "identify_unstable_parameters": null, "equation_id": null }),
    );
    ok_cmd(
        &mut session,
        "stats.pvalueStack",
        json!({ "sheet_id": "pstack-null", "source_sheet_id": "pvals", "method": "benjamini_hochberg", "threshold": null }),
    );
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("pstack-null")).unwrap().analysis.kind,
        AnalysisKind::PValueStack(ref ps) if (ps.threshold - 0.05).abs() < 1e-12
    ));

    // True Option fields on DescriptiveStats (`hypothetical_value`, `custom_upper_percentile`) must clear to None on `null`.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "desc-1",
            "source_sheet_id": "col2",
            "kind": "descriptive",
            "hypothetical_value": 10.0,
            "custom_upper_percentile": 90.0
        }),
    );
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "desc-1", "hypothetical_value": null, "custom_upper_percentile": null }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("desc-1")).unwrap().analysis.kind,
        AnalysisKind::DescriptiveStats(ref d) if d.hypothetical_value.is_none() && d.custom_upper_percentile.is_none()
    ));

    // Strict non-null usize fields still reject `null` with BadInput.
    let bad = err_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col2", "kind": "bland_altman", "column_a": null }));
    assert_eq!(bad.code, ErrorCode::BadInput);
}

#[test]
fn da02_06_broken_axis_segments_do_not_leak_segment_zero() {
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "xy1", "title": "Data", "kind": "xy", "rows": 3, "cols": 1, "auto_graph": true }));
    let graph_id = session.document().graph_sheets.first().unwrap().id.as_str().to_owned();

    // Give segment 0 explicit bounds and tick interval.
    ok_cmd(
        &mut session,
        "graph.setAxes",
        json!({
            "sheet_id": graph_id,
            "y_axis": { "min": 0.0, "max": 10.0, "major_tick_interval": 2.0 }
        }),
    );

    // Patch into a 2-segment broken axis where segment 1 omits `max` and `major_tick_interval`.
    ok_cmd(
        &mut session,
        "graph.setAxes",
        json!({
            "sheet_id": graph_id,
            "y_axis": {
                "segments": [
                    { "min": 0.0, "max": 5.0, "length_share": 0.4 },
                    { "min": 50.0, "length_share": 0.6 }
                ]
            }
        }),
    );

    let gs = session.document().graph_sheet(&SheetId::new(&graph_id)).unwrap();
    assert_eq!(gs.spec.y_axis.segments.len(), 2);
    let seg0 = gs.spec.y_axis.segments.first().unwrap();
    let seg1 = gs.spec.y_axis.segments.get(1).unwrap();
    assert_eq!(seg0.min, Some(0.0));
    assert_eq!(seg0.max, Some(5.0));
    assert_eq!(seg1.min, Some(50.0));
    assert_eq!(seg1.max, None, "segment 0's max must not leak onto segment 1");
    assert_eq!(seg1.major_tick_interval, None, "segment 0's major_tick_interval must not leak onto segment 1");
}

#[test]
fn da02_07_analysis_exports_enforce_path_policy() {
    let tmp = TempDirGuard::new("export-policy");
    let mut doc = ProjectDocument::new("DA-02-07").unwrap();
    doc.add_data_sheet(make_col_sheet("col2", 2, 5)).unwrap();
    let mut session = Session::from_document(doc).with_path_policy(ExternalPathPolicy::workdir(&tmp.workdir).expect("workdir policy"));
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res1", "source_sheet_id": "col2", "kind": "descriptive" }));

    // Inside workdir succeeds for all three commands.
    ok_cmd(&mut session, "analysis.exportJson", json!({ "sheet_id": "res1", "path": "out.json" }));
    ok_cmd(&mut session, "analysis.exportCsv", json!({ "sheet_id": "res1", "path": "out.csv" }));
    ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res1", "path": "report.md" }));
    assert!(tmp.workdir.join("out.json").is_file());
    assert!(tmp.workdir.join("out.csv").is_file());
    assert!(tmp.workdir.join("report.md").is_file());

    // Outside / traversal paths are blocked with PathEscape.
    let outside_json = tmp.outside.join("escape.json");
    let outside_csv = tmp.outside.join("escape.csv");
    let outside_md = tmp.outside.join("escape.md");
    assert_eq!(
        err_cmd(&mut session, "analysis.exportJson", json!({ "sheet_id": "res1", "path": outside_json.to_str().unwrap() })).code,
        ErrorCode::PathEscape
    );
    assert_eq!(
        err_cmd(&mut session, "analysis.exportCsv", json!({ "sheet_id": "res1", "path": "../outside/escape.csv" })).code,
        ErrorCode::PathEscape
    );
    assert_eq!(
        err_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res1", "path": outside_md.to_str().unwrap() })).code,
        ErrorCode::PathEscape
    );
    assert!(!outside_json.exists());
    assert!(!outside_csv.exists());
    assert!(!outside_md.exists());
}
