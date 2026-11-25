//! Unit tests for `two_group` `MultipleVariables` column resolution and analysis dispatch.
//!
//! # References
//!
//! - Student (1908). The probable error of a mean. *Biometrika* 6(1):1–25. doi:10.1093/biomet/6.1.1
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different population
//!   variances are involved. *Biometrika* 34(1–2):28–35. doi:10.1093/biomet/34.1-2.28

use super::*;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, MultipleVariablesTable, ResultSheet, SheetId, TableArchetype, TwoGroupMethod, TwoGroupSpec,
    VariableRole,
};
use curvebook_math::ConfidenceLevel;

use crate::error::StatsError;

fn run_mv_tg(mv: MultipleVariablesTable, tg: TwoGroupSpec) -> Result<ResultSheet, StatsError> {
    let sheet = DataSheet::new(SheetId::new("mv-tg"), "MV TwoGroup", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(SheetId::new("res-tg"), sheet.id.clone(), AnalysisKind::TwoGroupComparison(tg)).unwrap();
    run_two_group_analysis(&sheet, &spec, &tg)
}

fn scalar_val(res: &ResultSheet, key: &str) -> f64 {
    res.scalars.iter().find(|s| s.key == key).map(|s| s.value).unwrap()
}

#[test]
fn mv_two_group_excludes_label_columns_and_resolves_single_response() {
    let mut mv = MultipleVariablesTable::new(3, 8).unwrap();
    mv.variables[0].title = String::from("SampleID");
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[1].title = String::from("Treatment");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Biomarker");
    mv.variables[2].lock_role(VariableRole::Continuous);

    let g_a = [18.5, 20.2, 21.0, 22.4];
    let g_b = [31.0, 34.5, 42.0, 38.2];
    for (i, &v) in g_a.iter().enumerate() {
        mv.variables[0].cells[i] = Cell::text(format!("V{}", i + 1));
        mv.variables[1].cells[i] = Cell::text("Vehicle");
        mv.variables[2].cells[i] = Cell::number(v);
    }
    for (i, &v) in g_b.iter().enumerate() {
        mv.variables[0].cells[4 + i] = Cell::text(format!("T{}", i + 1));
        mv.variables[1].cells[4 + i] = Cell::text("Treated");
        mv.variables[2].cells[4 + i] = Cell::number(v);
    }
    let expected = welch_t_test(&g_a, &g_b, true, ConfidenceLevel::DEFAULT_95).unwrap();

    for mv_group_column in [None, Some(1)] {
        let res = run_mv_tg(
            mv.clone(),
            TwoGroupSpec { method: TwoGroupMethod::WelchT, mv_group_column, mv_response_column: None, ..TwoGroupSpec::default() },
        )
        .unwrap();
        assert!((scalar_val(&res, "PValue") - expected.p_value).abs() < 1e-12);
        assert!((scalar_val(&res, "t") - expected.t_statistic).abs() < 1e-12);
    }

    // Numeric Label IDs locked to VariableRole::Label must still be excluded.
    let mut mv_num_lbl = mv;
    for i in 0..8 {
        mv_num_lbl.variables[0].cells[i] = Cell::number((101 + i) as f64);
    }
    let res_num_lbl = run_mv_tg(
        mv_num_lbl,
        TwoGroupSpec { method: TwoGroupMethod::WelchT, mv_group_column: None, mv_response_column: None, ..TwoGroupSpec::default() },
    )
    .unwrap();
    assert!((scalar_val(&res_num_lbl, "PValue") - expected.p_value).abs() < 1e-12);
}

#[test]
fn mv_two_group_derived_calculated_response_fallback_picks_last_active_column() {
    let mut mv = MultipleVariablesTable::new(7, 8).unwrap();
    mv.variables[0].title = String::from("SampleID");
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[1].title = String::from("Condition");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("TargetCt");
    mv.variables[2].lock_role(VariableRole::Continuous);
    mv.variables[3].title = String::from("RefCt");
    mv.variables[3].lock_role(VariableRole::Continuous);
    mv.variables[4].title = String::from("DeltaCt");
    mv.variables[4].set_calculated_formula("[TargetCt] - [RefCt]");
    mv.variables[5].title = String::from("Log2FC");
    mv.variables[5].set_calculated_formula("-([DeltaCt] - 4.0)");
    mv.variables[6].title = String::from("TrailingEmpty");
    mv.variables[6].lock_role(VariableRole::Continuous);

    let ctrl_log2fc = [0.0, -0.1, 0.0, 0.1];
    let pert_log2fc = [3.0, 3.1, 3.0, 2.9];
    for r in 0..8 {
        let is_ctrl = r < 4;
        let idx = r % 4;
        let l2fc = if is_ctrl { ctrl_log2fc[idx] } else { pert_log2fc[idx] };
        mv.variables[0].cells[r] = Cell::text(format!("S{}", r + 1));
        mv.variables[1].cells[r] = Cell::text(if is_ctrl { "Control" } else { "Perturbed" });
        mv.variables[2].cells[r] = Cell::number(if is_ctrl { 22.0 } else { 19.0 });
        mv.variables[3].cells[r] = Cell::number(18.0);
        mv.variables[4].cells[r] = Cell::number(4.0 - l2fc);
        mv.variables[5].cells[r] = Cell::number(l2fc);
    }
    let expected = welch_t_test(&ctrl_log2fc, &pert_log2fc, true, ConfidenceLevel::DEFAULT_95).unwrap();

    for mv_group_column in [None, Some(1)] {
        let res = run_mv_tg(
            mv.clone(),
            TwoGroupSpec { method: TwoGroupMethod::WelchT, mv_group_column, mv_response_column: None, ..TwoGroupSpec::default() },
        )
        .unwrap();
        assert!((scalar_val(&res, "PValue") - expected.p_value).abs() < 1e-12);
        assert!((scalar_val(&res, "t") - expected.t_statistic).abs() < 1e-12);
    }
}

#[test]
fn mv_two_group_continuous_fallback_picks_last_active_non_empty_column() {
    let mut mv = MultipleVariablesTable::new(5, 6).unwrap();
    mv.variables[0].title = String::from("SampleID");
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[1].title = String::from("Arm");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Baseline");
    mv.variables[3].title = String::from("Outcome");
    mv.variables[4].title = String::from("EmptyCol");

    let g_a = [10.0, 11.0, 12.0];
    let g_b = [20.0, 22.0, 24.0];
    for i in 0..3 {
        mv.variables[0].cells[i] = Cell::text(format!("A{i}"));
        mv.variables[1].cells[i] = Cell::text("Placebo");
        mv.variables[2].cells[i] = Cell::number(1.0);
        mv.variables[3].cells[i] = Cell::number(g_a[i]);

        mv.variables[0].cells[3 + i] = Cell::text(format!("B{i}"));
        mv.variables[1].cells[3 + i] = Cell::text("Active");
        mv.variables[2].cells[3 + i] = Cell::number(1.0);
        mv.variables[3].cells[3 + i] = Cell::number(g_b[i]);
    }
    let expected = unpaired_t_test(&g_a, &g_b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    let res = run_mv_tg(mv, TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }).unwrap();
    assert!((scalar_val(&res, "PValue") - expected.p_value).abs() < 1e-12);
    assert!((scalar_val(&res, "t") - expected.t_statistic).abs() < 1e-12);
}
