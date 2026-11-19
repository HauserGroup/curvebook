//! D1 oracle verification matrix and edge-case tests for `R10.10`
//! ("Analyze a stack of P values") across all 7 multiplicity procedures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellValue, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, MultipleVariablesTable, PValueFormat,
    PValueStackMethod, PValueStackSpec, ResultCell, ResultTabKind, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_math::MathError;
use curvebook_stats::pvalue_stack::{analyze_pvalue_stack, validate_pvalue_threshold, validate_raw_pvalues};
use curvebook_stats::{StatsError, run_analysis};

const TOL: f64 = 1e-12;

fn assert_close(actual: f64, expected: f64, label: &str) {
    assert!((actual - expected).abs() <= TOL, "{label}: expected {expected:.15}, got {actual:.15} (diff = {:.3e})", (actual - expected).abs());
}

fn col_sheet_with_values(cells: Vec<Cell>) -> DataSheet {
    let n_rows = cells.len().max(1);
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, n_rows).expect("column table");
    table.data_sets[0].title = String::from("Raw P");
    for (r, cell) in cells.into_iter().enumerate() {
        table.row_titles[r] = format!("Gene {}", r + 1);
        table.data_sets[0].cells[r][0] = cell;
    }
    DataSheet::new("ds-pvals", "P-Value Stack", TableArchetype::Column(table)).expect("data sheet")
}

/// Independent textbook reference implementations (no calls into `curvebook_stats::multiplicity`)
/// derived directly from the 7 primary papers for differential oracle checking.
mod independent_oracle {
    pub fn bonferroni(raw: &[f64]) -> Vec<f64> {
        let m = raw.len() as f64;
        raw.iter().map(|&p| (p * m).min(1.0)).collect()
    }

    pub fn sidak(raw: &[f64]) -> Vec<f64> {
        let m = raw.len() as f64;
        raw.iter().map(|&p| (1.0 - (1.0 - p).powf(m)).clamp(0.0, 1.0)).collect()
    }

    pub fn holm(raw: &[f64]) -> Vec<f64> {
        let m = raw.len();
        let mut idx: Vec<usize> = (0..m).collect();
        idx.sort_by(|&a, &b| raw[a].total_cmp(&raw[b]));
        let mut out = vec![1.0; m];
        let mut run_max = 0.0_f64;
        for (pos, &orig) in idx.iter().enumerate() {
            let mult = (m - pos) as f64;
            run_max = run_max.max((raw[orig] * mult).min(1.0));
            out[orig] = run_max;
        }
        out
    }

    pub fn holm_sidak(raw: &[f64]) -> Vec<f64> {
        let m = raw.len();
        let mut idx: Vec<usize> = (0..m).collect();
        idx.sort_by(|&a, &b| raw[a].total_cmp(&raw[b]));
        let mut out = vec![1.0; m];
        let mut run_max = 0.0_f64;
        for (pos, &orig) in idx.iter().enumerate() {
            let mult = (m - pos) as f64;
            let cand = (1.0 - (1.0 - raw[orig]).powf(mult)).clamp(0.0, 1.0);
            run_max = run_max.max(cand);
            out[orig] = run_max;
        }
        out
    }

    pub fn bh_scaled(raw: &[f64], factor: f64) -> Vec<f64> {
        let m = raw.len();
        let mut idx: Vec<usize> = (0..m).collect();
        idx.sort_by(|&a, &b| raw[a].total_cmp(&raw[b]));
        let mut out = vec![1.0; m];
        let mut run_min = 1.0_f64;
        for (pos, &orig) in idx.iter().enumerate().rev() {
            let rank = (pos + 1) as f64;
            let cand = ((raw[orig] * (m as f64) * factor) / rank).min(1.0);
            run_min = run_min.min(cand);
            out[orig] = run_min.clamp(0.0, 1.0);
        }
        out
    }

    pub fn benjamini_hochberg(raw: &[f64]) -> Vec<f64> {
        bh_scaled(raw, 1.0)
    }

    pub fn benjamini_yekutieli(raw: &[f64]) -> Vec<f64> {
        let hm: f64 = (1..=raw.len()).map(|i| 1.0 / (i as f64)).sum();
        bh_scaled(raw, hm)
    }
}

#[test]
fn hand_calculated_oracle_unsorted_with_ties_all_seven_methods() {
    // Unsorted vector with exact ties at 0.04: sorted values = [0.002, 0.015, 0.04, 0.04, 0.80].
    let raw = [0.04, 0.002, 0.04, 0.80, 0.015];
    let threshold = 0.05;

    // 1. Bonferroni
    let bonf = analyze_pvalue_stack(&raw, PValueStackMethod::Bonferroni, threshold).unwrap();
    let exp_bonf = [0.20, 0.01, 0.20, 1.0, 0.075];
    for (r, &e) in bonf.rows.iter().zip(&exp_bonf) {
        assert_close(r.adjusted_p_or_q, e, "bonferroni");
    }
    assert_eq!(bonf.discovery_count, 1);
    assert_eq!(bonf.rows.iter().map(|r| r.discovery).collect::<Vec<_>>(), vec![false, true, false, false, false]);

    // 2. Holm (1979)
    let holm = analyze_pvalue_stack(&raw, PValueStackMethod::HolmBonferroni, threshold).unwrap();
    let exp_holm = [0.12, 0.01, 0.12, 0.80, 0.06];
    for (r, &e) in holm.rows.iter().zip(&exp_holm) {
        assert_close(r.adjusted_p_or_q, e, "holm");
    }
    assert_eq!(holm.discovery_count, 1);

    // 3. Sidak (1967)
    let sidak = analyze_pvalue_stack(&raw, PValueStackMethod::Sidak, threshold).unwrap();
    let exp_sidak = [1.0 - 0.96_f64.powi(5), 1.0 - 0.998_f64.powi(5), 1.0 - 0.96_f64.powi(5), 1.0 - 0.20_f64.powi(5), 1.0 - 0.985_f64.powi(5)];
    for (r, &e) in sidak.rows.iter().zip(&exp_sidak) {
        assert_close(r.adjusted_p_or_q, e, "sidak");
    }
    assert_eq!(sidak.discovery_count, 1);

    // 4. Holm-Sidak (1967 / 1979 / 1987)
    let hs = analyze_pvalue_stack(&raw, PValueStackMethod::HolmSidak, threshold).unwrap();
    let hs_04 = 1.0 - 0.96_f64.powi(3);
    let exp_hs = [hs_04, 1.0 - 0.998_f64.powi(5), hs_04, 0.80, 1.0 - 0.985_f64.powi(4)];
    for (r, &e) in hs.rows.iter().zip(&exp_hs) {
        assert_close(r.adjusted_p_or_q, e, "holm_sidak");
    }
    assert_eq!(hs.discovery_count, 1);

    // 5. Benjamini-Hochberg (1995)
    let bh = analyze_pvalue_stack(&raw, PValueStackMethod::BenjaminiHochberg, threshold).unwrap();
    let exp_bh = [0.05, 0.01, 0.05, 0.80, 0.0375];
    for (r, &e) in bh.rows.iter().zip(&exp_bh) {
        assert_close(r.adjusted_p_or_q, e, "bh");
    }
    assert_eq!(bh.discovery_count, 4);
    assert_eq!(bh.rows.iter().map(|r| r.discovery).collect::<Vec<_>>(), vec![true, true, true, false, true]);

    // 6. Benjamini-Yekutieli (2001): H_5 = 137/60
    let by = analyze_pvalue_stack(&raw, PValueStackMethod::BenjaminiYekutieli, threshold).unwrap();
    let h5 = 137.0_f64 / 60.0_f64;
    let exp_by = [5.0 * h5 * 0.04 / 4.0, 5.0 * h5 * 0.002, 5.0 * h5 * 0.04 / 4.0, 1.0, 5.0 * h5 * 0.015 / 2.0];
    for (r, &e) in by.rows.iter().zip(&exp_by) {
        assert_close(r.adjusted_p_or_q, e, "by");
    }
    assert_eq!(by.discovery_count, 1);

    // 7. Benjamini-Krieger-Yekutieli two-stage adaptive (2006)
    // Stage 1 at q' = 0.05 / 1.05 = 0.047619...: BH q at rank 2 is 0.0375 <= q',
    // rank 4 is 0.05 > q', so r1 = 2, m0_hat = 3, scale = (1 + 0.05) * (3 / 5) = 0.63.
    let bky = analyze_pvalue_stack(&raw, PValueStackMethod::TwoStageBky, threshold).unwrap();
    let scale = (1.0 + threshold) * (3.0 / 5.0);
    // Notice rank 4 (p=0.04) = 0.0315 < 0.04 and rank 3 (p=0.04) = 0.042 clamped by running_min to 0.0315.
    let exp_bky = [5.0 * scale * 0.04 / 4.0, 5.0 * scale * 0.002, 5.0 * scale * 0.04 / 4.0, 5.0 * scale * 0.80 / 5.0, 5.0 * scale * 0.015 / 2.0];
    for (r, &e) in bky.rows.iter().zip(&exp_bky) {
        assert_close(r.adjusted_p_or_q, e, "bky");
    }
    assert_eq!(bky.discovery_count, 4);

    // Verify input-row order and competition rank mapping on ties.
    for res in [&bonf, &holm, &sidak, &hs, &bh, &by, &bky] {
        assert_eq!(res.rows.iter().map(|r| r.input_index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert_eq!(res.rows.iter().map(|r| r.table_row_index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert_eq!(res.rows.iter().map(|r| r.rank).collect::<Vec<_>>(), vec![3, 1, 3, 5, 2]);
        assert_eq!(res.rows.iter().map(|r| r.sorted_order).collect::<Vec<_>>(), vec![3, 1, 4, 5, 2]);
        // Tied rows 0 and 2 (both p = 0.04) must receive identical adjusted P/q and discovery flag.
        assert_close(res.rows[0].adjusted_p_or_q, res.rows[2].adjusted_p_or_q, "tie adjusted equality");
        assert_eq!(res.rows[0].discovery, res.rows[2].discovery);
    }
}

#[test]
fn neuhaus_1992_from_bh_1995_differential_oracle_matrix() {
    // Benjamini & Hochberg (1995), JRSS-B 57(1) p. 296 (Neuhaus et al., 1992, m = 15),
    // placed in deterministic scrambled input order to assert order invariance.
    let sorted = [0.0001, 0.0004, 0.0019, 0.0095, 0.0201, 0.0278, 0.0298, 0.0344, 0.0459, 0.3240, 0.4262, 0.5719, 0.6528, 0.7590, 1.0000];
    let perm = [7, 0, 12, 3, 14, 5, 1, 9, 4, 11, 2, 13, 6, 8, 10];
    let raw: Vec<f64> = perm.iter().map(|&i| sorted[i]).collect();

    let checks: [(PValueStackMethod, Vec<f64>, usize); 6] = [
        (PValueStackMethod::Bonferroni, independent_oracle::bonferroni(&raw), 3),
        (PValueStackMethod::HolmBonferroni, independent_oracle::holm(&raw), 3),
        (PValueStackMethod::Sidak, independent_oracle::sidak(&raw), 3),
        (PValueStackMethod::HolmSidak, independent_oracle::holm_sidak(&raw), 3),
        (PValueStackMethod::BenjaminiHochberg, independent_oracle::benjamini_hochberg(&raw), 4),
        (PValueStackMethod::BenjaminiYekutieli, independent_oracle::benjamini_yekutieli(&raw), 3),
    ];

    for (method, expected_vec, expected_disc) in checks {
        let res = analyze_pvalue_stack(&raw, method, 0.05).unwrap();
        assert_eq!(res.discovery_count, expected_disc, "method {:?}", method);
        for (idx, (row, &exp)) in res.rows.iter().zip(&expected_vec).enumerate() {
            assert_close(row.adjusted_p_or_q, exp, &format!("{method:?} row {idx}"));
            assert_close(row.raw_p, raw[idx], &format!("{method:?} raw {idx}"));
        }
    }

    // BKY (2006) at q = 0.05: stage-1 finds 4 rejections (0.035625 <= 0.05/1.05 = 0.047619),
    // giving m0_hat = 11, stage-2 factor (11 * 1.05 / 15) = 0.77, rejecting up to rank 8
    // (0.0344 * 15 * 0.77 / 8 = 0.049665 <= 0.05), yielding 8 discoveries!
    let bky = analyze_pvalue_stack(&raw, PValueStackMethod::TwoStageBky, 0.05).unwrap();
    assert_eq!(bky.discovery_count, 8);
    let expected_bky = independent_oracle::bh_scaled(&raw, (11.0 * 1.05) / 15.0);
    for (idx, (row, &exp)) in bky.rows.iter().zip(&expected_bky).enumerate() {
        assert_close(row.adjusted_p_or_q, exp, &format!("BKY row {idx}"));
    }
}

#[test]
fn bky_stage1_zero_and_all_rejected_boundary_cases() {
    // Stage 1 finds r1 = 0 rejections (all adjusted BH values > q / (1 + q)):
    // BKY (2006) stops after stage 1 and rejects 0 hypotheses.
    let all_large = [0.20, 0.50, 0.90];
    let bky_none = analyze_pvalue_stack(&all_large, PValueStackMethod::TwoStageBky, 0.05).unwrap();
    assert_eq!(bky_none.discovery_count, 0);

    // Stage 1 rejects ALL m hypotheses (r1 = m => m0_hat = 0):
    // BKY (2006) rejects all m hypotheses with discovery = true.
    let all_tiny = [0.0001, 0.0002, 0.0003];
    let bky_all = analyze_pvalue_stack(&all_tiny, PValueStackMethod::TwoStageBky, 0.05).unwrap();
    assert_eq!(bky_all.discovery_count, 3);
    assert!(bky_all.rows.iter().all(|r| r.discovery && r.adjusted_p_or_q <= 0.05));
}

#[test]
fn boundary_zero_and_one_inputs_across_all_methods() {
    let raw = [0.0, 1.0, 0.0, 1.0];
    for method in PValueStackMethod::ALL {
        let res = analyze_pvalue_stack(&raw, method, 0.05).unwrap();
        assert_eq!(res.count, 4);
        assert_close(res.rows[0].raw_p, 0.0, &format!("{method:?} raw 0"));
        assert_close(res.rows[1].raw_p, 1.0, &format!("{method:?} raw 1"));
        assert_close(res.rows[0].adjusted_p_or_q, 0.0, &format!("{method:?} adj 0"));
        assert_close(res.rows[2].adjusted_p_or_q, 0.0, &format!("{method:?} adj 0 tie"));
        let expected_adj1 = if method == PValueStackMethod::TwoStageBky { 0.525 } else { 1.0 };
        assert_close(res.rows[1].adjusted_p_or_q, expected_adj1, &format!("{method:?} adj 1"));
        assert_close(res.rows[3].adjusted_p_or_q, expected_adj1, &format!("{method:?} adj 1 tie"));
        assert_eq!(res.rows.iter().map(|r| r.discovery).collect::<Vec<_>>(), vec![true, false, true, false]);
    }

    // Singleton stack at exact boundary values
    for method in PValueStackMethod::ALL {
        let zero = analyze_pvalue_stack(&[0.0], method, 0.05).unwrap();
        assert_close(zero.rows[0].adjusted_p_or_q, 0.0, "singleton 0");
        assert!(zero.rows[0].discovery);

        let one = analyze_pvalue_stack(&[1.0], method, 0.05).unwrap();
        assert_close(one.rows[0].adjusted_p_or_q, 1.0, "singleton 1");
        assert!(!one.rows[0].discovery);
    }
}

#[test]
fn domain_errors_and_invalid_thresholds_never_coerce() {
    // Pure slice domain errors
    assert!(matches!(validate_raw_pvalues(&[]), Err(StatsError::EmptyInput)));
    for bad_p in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e-12, -0.5, 1.000_000_1, 42.0] {
        let err = analyze_pvalue_stack(&[0.02, bad_p, 0.5], PValueStackMethod::BenjaminiHochberg, 0.05).unwrap_err();
        assert!(
            matches!(err, StatsError::Math(MathError::DomainError { name: "p_value", .. })),
            "expected DomainError for bad_p = {bad_p:?}, got {err:?}"
        );
    }

    for bad_thresh in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -0.05, 1.0, 1.5] {
        let err = validate_pvalue_threshold(bad_thresh).unwrap_err();
        assert!(
            matches!(err, StatsError::InvalidParameter { name: "threshold", .. }),
            "expected InvalidParameter for threshold = {bad_thresh:?}, got {err:?}"
        );
        assert!(analyze_pvalue_stack(&[0.01], PValueStackMethod::Bonferroni, bad_thresh).is_err());
    }

    // DataSheet domain errors (direct non-finite Number cell, Text("NaN"), < 0, > 1)
    let bad_cells = [
        Cell { value: CellValue::Number(f64::NAN), excluded: false },
        Cell { value: CellValue::Number(f64::INFINITY), excluded: false },
        Cell { value: CellValue::Number(f64::NEG_INFINITY), excluded: false },
        Cell::number(-0.001),
        Cell::number(1.05),
        Cell::text("NaN"),
        Cell::text("-Infinity"),
    ];
    for bad_cell in bad_cells {
        let sheet = col_sheet_with_values(vec![Cell::number(0.01), bad_cell, Cell::number(0.20)]);
        let spec = AnalysisSpec::new("res-err", sheet.id.clone(), AnalysisKind::PValueStack(PValueStackSpec::default())).unwrap();
        let err = run_analysis(&sheet, &spec).unwrap_err();
        assert!(
            matches!(err, StatsError::Math(MathError::DomainError { name: "p_value", .. })),
            "expected DomainError on DataSheet cell, got {err:?}"
        );
    }
}

#[test]
fn empty_and_all_excluded_columns_and_row_alignment_in_datasheet() {
    // 1. Completely empty ColumnTable -> EmptyInput
    let empty_col =
        DataSheet::new("ds-empty", "Empty", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5).unwrap())).unwrap();
    let spec_empty = AnalysisSpec::new("res-empty", "ds-empty", AnalysisKind::PValueStack(PValueStackSpec::default())).unwrap();
    assert!(matches!(run_analysis(&empty_col, &spec_empty), Err(StatsError::EmptyInput)));

    // 2. All-excluded column -> EmptyInput
    let excl_sheet = col_sheet_with_values(vec![Cell::excluded_number(0.01), Cell::excluded_number(0.04)]);
    let spec_excl = AnalysisSpec::new("res-excl", excl_sheet.id.clone(), AnalysisKind::PValueStack(PValueStackSpec::default())).unwrap();
    assert!(matches!(run_analysis(&excl_sheet, &spec_excl), Err(StatsError::EmptyInput)));

    // 3. Multi-row column with gap/excluded rows preserves exact 1-based table row indices & input order
    let gap_sheet = col_sheet_with_values(vec![
        Cell::number(0.04),          // row 1
        Cell::empty(),               // row 2 skipped
        Cell::excluded_number(99.0), // row 3 excluded (even out-of-range when excluded is skipped!)
        Cell::number(0.002),         // row 4
        Cell::number(0.04),          // row 5 (tie with row 1)
    ]);
    let spec = AnalysisSpec::new(
        "res-gap",
        gap_sheet.id.clone(),
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::BenjaminiHochberg, threshold: 0.05, column: None }),
    )
    .unwrap();
    let res = run_analysis(&gap_sheet, &spec).unwrap();
    assert_eq!(res.tabs.len(), 2);
    assert_eq!(res.tabs[0].kind, ResultTabKind::MainSummary);
    assert_eq!(res.tabs[1].kind, ResultTabKind::MultipleComparisons);
    // Input row order preserved: rows 1, 4, 5
    assert_eq!(res.tabs[1].row_labels, vec!["Gene 1", "Gene 4", "Gene 5"]);
    assert_eq!(
        res.tabs[1]
            .rows
            .iter()
            .map(|r| match r[1] {
                ResultCell::Number { value: v, .. } => v as usize,
                _ => 0,
            })
            .collect::<Vec<_>>(),
        vec![1, 4, 5]
    );
    // Ranks: row 1 (0.04) -> 2, row 4 (0.002) -> 1, row 5 (0.04) -> 2
    assert_eq!(
        res.tabs[1]
            .rows
            .iter()
            .map(|r| match r[2] {
                ResultCell::Number { value: v, .. } => v as usize,
                _ => 0,
            })
            .collect::<Vec<_>>(),
        vec![2, 1, 2]
    );
    // Row 1 and row 5 tied at 0.04 -> m=3, q = 3*0.04/3 = 0.04 <= 0.05 -> all 3 discoveries
    assert_eq!(
        res.tabs[1]
            .rows
            .iter()
            .map(|r| match r[5] {
                ResultCell::Boolean(b) => b,
                _ => false,
            })
            .collect::<Vec<_>>(),
        vec![true, true, true]
    );
}

#[test]
fn multi_column_selection_missing_column_and_incompatible_tables() {
    // 2-column ColumnTable: Col 0 and Col 1 adjusted independently as separate stacks
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
    table.data_sets[0].title = String::from("Screen A");
    table.data_sets[1].title = String::from("Screen B");
    for (r, &p) in [0.001, 0.20, 0.50].iter().enumerate() {
        table.data_sets[0].cells[r][0] = Cell::number(p);
    }
    for (r, &p) in [0.01, 0.02, 0.90].iter().enumerate() {
        table.data_sets[1].cells[r][0] = Cell::number(p);
    }
    let sheet = DataSheet::new("ds-2col", "Dual Screen", TableArchetype::Column(table)).unwrap();

    // Both columns analyzed when selected_data_sets empty & column = None
    let spec_both = AnalysisSpec::new(
        "res-2col",
        "ds-2col",
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::Bonferroni, threshold: 0.05, column: None }),
    )
    .unwrap();
    let res_both = run_analysis(&sheet, &spec_both).unwrap();
    assert_eq!(res_both.tabs[0].column_headers, vec!["Screen A", "Screen B"]);
    assert_eq!(res_both.tabs[1].rows.len(), 6);

    // Selecting only Column 1 via PValueStackSpec.column
    let spec_col1 = AnalysisSpec::new(
        "res-col1",
        "ds-2col",
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::BenjaminiHochberg, threshold: 0.05, column: Some(1) }),
    )
    .unwrap();
    let res_col1 = run_analysis(&sheet, &spec_col1).unwrap();
    assert_eq!(res_col1.tabs[0].column_headers, vec!["Screen B"]);
    assert_eq!(res_col1.tabs[1].rows.len(), 3);

    // Out-of-bounds column override -> MissingColumn
    let spec_missing = AnalysisSpec::new(
        "res-miss",
        "ds-2col",
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::Bonferroni, threshold: 0.05, column: Some(5) }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&sheet, &spec_missing), Err(StatsError::MissingColumn { index: 5 })));

    // MultipleVariablesTable (with Categorical label col 0 + numeric P col 1) + Xy + Grouped compatibility
    let mut mv = MultipleVariablesTable::new(2, 3).unwrap();
    mv.variables[0].title = String::from("Target");
    mv.variables[0].role = VariableRole::Label;
    mv.variables[0].cells = vec![Cell::text("G1"), Cell::text("G2"), Cell::text("G3")];
    mv.variables[1].title = String::from("P_raw");
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[1].cells = vec![Cell::number(0.005), Cell::number(0.03), Cell::number(0.4)];
    let mv_sheet = DataSheet::new("ds-mv", "MV Pvals", TableArchetype::MultipleVariables(mv)).unwrap();
    let mut mv_spec = AnalysisSpec::new("res-mv", "ds-mv", AnalysisKind::PValueStack(PValueStackSpec::default())).unwrap();
    mv_spec.p_value_format = PValueFormat::FourDecimal;
    let res_mv = run_analysis(&mv_sheet, &mv_spec).unwrap();
    assert_eq!(res_mv.tabs[0].column_headers, vec!["P_raw"]);
    assert_eq!(res_mv.tabs[1].rows.len(), 3);

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy.data_sets[0].cells[0][0] = Cell::number(0.01);
    xy.data_sets[0].cells[1][0] = Cell::number(0.5);
    let xy_sheet = DataSheet::new("ds-xy", "XY Pvals", TableArchetype::Xy(xy)).unwrap();
    let xy_spec = AnalysisSpec::new("res-xy", "ds-xy", AnalysisKind::PValueStack(PValueStackSpec::default())).unwrap();
    assert!(run_analysis(&xy_sheet, &xy_spec).is_ok());

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 1, 2).unwrap();
    grp.data_sets[0].cells[0][0] = Cell::number(0.01);
    grp.data_sets[0].cells[1][0] = Cell::number(0.5);
    let grp_sheet = DataSheet::new("ds-grp", "Grp Pvals", TableArchetype::Grouped(grp)).unwrap();
    let grp_spec = AnalysisSpec::new("res-grp", "ds-grp", AnalysisKind::PValueStack(PValueStackSpec::default())).unwrap();
    assert!(run_analysis(&grp_sheet, &grp_spec).is_ok());

    // Pre-summarized ColumnTable -> RequiresRawReplicates
    let pre = ColumnTable::new(ColumnDesignHint::PrecomputedError, 1, 2).unwrap();
    let pre_sheet = DataSheet::new("ds-pre", "Pre", TableArchetype::Column(pre)).unwrap();
    let pre_spec = AnalysisSpec::new("res-pre", "ds-pre", AnalysisKind::PValueStack(PValueStackSpec::default())).unwrap();
    assert!(matches!(run_analysis(&pre_sheet, &pre_spec), Err(StatsError::RequiresRawReplicates)));

    // Incompatible ContingencyTable -> IncompatibleTable
    let cont = curvebook_doc::ContingencyTable::new(2, 2).unwrap();
    let cont_sheet = DataSheet::new("ds-cont", "Cont", TableArchetype::Contingency(cont)).unwrap();
    let cont_spec = AnalysisSpec::new("res-cont", "ds-cont", AnalysisKind::PValueStack(PValueStackSpec::default())).unwrap();
    assert!(matches!(run_analysis(&cont_sheet, &cont_spec), Err(StatsError::IncompatibleTable { .. })));
}
