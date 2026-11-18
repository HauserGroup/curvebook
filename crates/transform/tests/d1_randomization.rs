//! Independent D1 randomization, stratified sampling, and discrete simulation oracles (R9.14).
//!
//! Reference inputs, exact integer quotas, Binomial/Poisson CDF cutoffs, seeds, draw counts,
//! statistical acceptance bounds, and the family-wise false-positive budget were pre-specified
//! and generated offline prior to checking CurveBook output.
//!
//! Reproduce exact quotas, CDFs, moments, and per-comparison tail probabilities with:
//! `crates/transform/tests/reference/randomization.py` (mpmath 1.3.0 at 120 digits, verified
//! bit-for-bit identical at 80 digits) and `crates/transform/tests/reference/randomization.R`
//! (R 4.5.2 `stats::pbinom` / `stats::ppois`, max relative difference `4.40e-15`).
//!
//! # Family-Wise False-Positive Budget
//!
//! Across the 11 stochastic distribution columns (`N = 25_000` draws per column, `seed = 2026_1118`),
//! there are `M = 66` planned stochastic comparisons (`44` empirical CDF cutoffs + `11` sample
//! means + `11` sample variances). Each check uses `z = 5.6` standard errors:
//! - Exact discrete two-sided `Binomial(25_000, F(k))` tail sum over all 44 CDF checks: `9.9925e-7`.
//! - Exact discrete two-sided `Binomial(25_000 * n, p)` / `Poisson(25_000 * lambda)` tail sum over
//!   all 11 mean checks: `2.3612e-7`.
//! - Asymptotic two-sided normal tail at `z = 5.6` is `2.144e-8` per variance check; budgeting
//!   `1.0e-7` per variance check (`1.1000e-6` total across 11 checks) for finite-sample margin gives
//!   a Bonferroni family-wise upper bound of `2.3354e-6 <= 1.0e-5`.
//!
//! Deterministic schedule, stratified-quota, support, and endpoint assertions (`p = 0`, `p = 1`,
//! `trials = 0`, `lambda = 0`) have zero random false-positive probability.

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellExclusionNote, DataSheet, MultipleVariablesTable, TableArchetype, TransformAnalysisSpec,
    VariableRole,
};
use curvebook_math::sum::SummaryMoments;
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, RowSampleConfig, SimulatedColumnSpec, SimulatedDistribution,
    SubjectRandomizationConfig, TransformContext, execute_analysis_transform, random_sample_rows, randomize_subjects_to_groups, sample_row_indices,
    simulate_distribution_columns,
};

const STOCHASTIC_DRAWS: usize = 25_000;
const STOCHASTIC_SEED: u64 = 2026_1118;
const ENDPOINT_DRAWS: usize = 512;
const ENDPOINT_SEED: u64 = 2026_0001;

fn arrive_map(constants: &[(String, String)]) -> BTreeMap<&str, &str> {
    constants.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

fn schedule_mv(sheet: &DataSheet) -> Result<&MultipleVariablesTable, String> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => Ok(mv),
        other => Err(format!("expected MultipleVariables schedule sheet, found {other:?}")),
    }
}

fn schedule_arm_counts(sheet: &DataSheet) -> Result<BTreeMap<String, usize>, String> {
    let mv = schedule_mv(sheet)?;
    let mut counts = BTreeMap::new();
    for cell in &mv.variables[4].cells {
        let arm = cell.active_text().ok_or_else(|| String::from("missing active group label"))?;
        *counts.entry(arm.to_owned()).or_insert(0) += 1;
    }
    Ok(counts)
}

fn assert_schedule_subject_bijection(sheet: &DataSheet, expected_ids: &[String], expected_arms: &[&str]) -> Result<(), String> {
    let mv = schedule_mv(sheet)?;
    let titles: Vec<&str> = mv.variables.iter().map(|v| v.title.as_str()).collect();
    assert_eq!(titles, ["Subject_ID", "Allocation_Order", "Block_Index", "Block_Size", "Assigned_Group"]);
    assert_eq!(mv.variables[0].role, VariableRole::Categorical);
    assert_eq!(mv.variables[1].role, VariableRole::Continuous);
    assert_eq!(mv.variables[2].role, VariableRole::Continuous);
    assert_eq!(mv.variables[3].role, VariableRole::Continuous);
    assert_eq!(mv.variables[4].role, VariableRole::Categorical);
    assert_eq!(mv.row_count(), expected_ids.len());

    let mut seen_subjects = BTreeSet::new();
    for (row, expected_id) in expected_ids.iter().enumerate() {
        let sid = mv.variables[0].cells[row].active_text().ok_or_else(|| format!("missing subject id at row {row}"))?;
        assert_eq!(sid, expected_id.trim());
        assert!(seen_subjects.insert(sid.to_owned()), "duplicate subject assignment for {sid}");
        assert_eq!(mv.variables[1].cells[row].active_number(), Some((row + 1) as f64));
        let arm = mv.variables[4].cells[row].active_text().ok_or_else(|| format!("missing assigned arm at row {row}"))?;
        assert!(expected_arms.contains(&arm), "unexpected arm `{arm}` at row {row}");
    }
    Ok(())
}

#[test]
fn complete_balanced_unequal_weights_with_tied_remainders_match_hamilton_quotas() {
    // Case 1: N = 15, weights [4, 3, 2, 1] (sum = 10).
    // Exact products: [60, 45, 30, 15] -> floors [6, 4, 3, 1], remainders [0, 5, 0, 5].
    // Single leftover seat breaks the 5-vs-5 tie in favor of earlier arm "ArmB": [6, 5, 3, 1].
    let ids_15: Vec<String> = (1..=15).map(|i| format!("SUBJ-{i:02}")).collect();
    let cfg_15 = SubjectRandomizationConfig {
        sheet_id: String::from("cb-tied-4arm"),
        title: String::from("Complete Balanced 4-Arm Tied Remainders"),
        subject_ids: ids_15.clone(),
        groups: vec![
            GroupAllocationWeight::with_weight("ArmA", 4),
            GroupAllocationWeight::with_weight("ArmB", 3),
            GroupAllocationWeight::with_weight("ArmC", 2),
            GroupAllocationWeight::with_weight("ArmD", 1),
        ],
        scheme: AllocationScheme::CompleteBalanced,
        seed: 104_729,
    };
    let out_15 = randomize_subjects_to_groups(&cfg_15).unwrap();
    assert_schedule_subject_bijection(&out_15.data_sheet, &ids_15, &["ArmA", "ArmB", "ArmC", "ArmD"]).unwrap();
    let counts_15 = schedule_arm_counts(&out_15.data_sheet).unwrap();
    assert_eq!(counts_15.get("ArmA").copied(), Some(6));
    assert_eq!(counts_15.get("ArmB").copied(), Some(5));
    assert_eq!(counts_15.get("ArmC").copied(), Some(3));
    assert_eq!(counts_15.get("ArmD").copied(), Some(1));

    let TableArchetype::MultipleVariables(mv15) = &out_15.data_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    for r in 0..15 {
        assert_eq!(mv15.variables[2].cells[r].active_number(), Some(1.0));
        assert_eq!(mv15.variables[3].cells[r].active_number(), Some(15.0));
    }
    let arrive_15 = arrive_map(&out_15.arrive_constants);
    assert_eq!(arrive_15.get("randomization.seed").copied(), Some("104729"));
    assert_eq!(arrive_15.get("randomization.method").copied(), Some("complete_balanced"));
    assert_eq!(arrive_15.get("randomization.allocation_ratio").copied(), Some("4:3:2:1"));
    assert_eq!(arrive_15.get("randomization.block_sizes").copied(), Some("15"));
    assert_eq!(arrive_15.get("randomization.total_subjects").copied(), Some("15"));

    // Case 2: N = 9, weights [3, 2, 2] (sum = 7).
    // Exact products: [27, 18, 18] -> floors [3, 2, 2], remainders [6, 4, 4].
    // Leftover = 2: "High" (rem 6) gets +1, then "Mid" beats tied "Low" (rem 4): [4, 3, 2].
    let cfg_9 = SubjectRandomizationConfig::from_subject_count(
        "cb-tied-3arm",
        "Complete Balanced 3-Arm Tied Remainders",
        9,
        vec![
            GroupAllocationWeight::with_weight("High", 3),
            GroupAllocationWeight::with_weight("Mid", 2),
            GroupAllocationWeight::with_weight("Low", 2),
        ],
        AllocationScheme::CompleteBalanced,
        224_737,
    );
    let out_9 = randomize_subjects_to_groups(&cfg_9).unwrap();
    let ids_9: Vec<String> = (1..=9).map(|i| i.to_string()).collect();
    assert_schedule_subject_bijection(&out_9.data_sheet, &ids_9, &["High", "Mid", "Low"]).unwrap();
    let counts_9 = schedule_arm_counts(&out_9.data_sheet).unwrap();
    assert_eq!(counts_9.get("High").copied(), Some(4));
    assert_eq!(counts_9.get("Mid").copied(), Some(3));
    assert_eq!(counts_9.get("Low").copied(), Some(2));
}

#[test]
fn fixed_permuted_blocks_preserve_full_block_ratios_valid_truncated_tail_and_arrive_metadata() {
    let weights = [("Ctrl", 1_usize), ("Low", 2_usize), ("High", 3_usize)];
    let group_specs: Vec<GroupAllocationWeight> = weights.iter().map(|&(l, w)| GroupAllocationWeight::with_weight(l, w)).collect();
    let arm_names = ["Ctrl", "Low", "High"];

    // Fixed block size 12 over 29 subjects -> 2 full blocks (2:4:6) + 1 truncated block of 5.
    let ids_29: Vec<String> = (1..=29).map(|i| format!("M-{i:02}")).collect();
    let fixed_cfg = SubjectRandomizationConfig {
        sheet_id: String::from("pb-fixed"),
        title: String::from("Fixed Permuted Block 1:2:3"),
        subject_ids: ids_29.clone(),
        groups: group_specs,
        scheme: AllocationScheme::PermutedBlock { block_sizes: vec![12] },
        seed: 350_377,
    };
    let fixed_out = randomize_subjects_to_groups(&fixed_cfg).unwrap();
    assert_eq!(fixed_out, randomize_subjects_to_groups(&fixed_cfg).unwrap());
    assert_schedule_subject_bijection(&fixed_out.data_sheet, &ids_29, &arm_names).unwrap();

    let TableArchetype::MultipleVariables(fmv) = &fixed_out.data_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    for blk in 0..3_usize {
        let start = blk * 12;
        let end = (start + 12).min(29);
        let mut counts = [0_usize; 3];
        for r in start..end {
            assert_eq!(fmv.variables[2].cells[r].active_number(), Some((blk + 1) as f64));
            assert_eq!(fmv.variables[3].cells[r].active_number(), Some(12.0));
            let arm = fmv.variables[4].cells[r].active_text().unwrap();
            let idx = arm_names.iter().position(|&a| a == arm).unwrap();
            counts[idx] += 1;
        }
        if end - start == 12 {
            assert_eq!(counts, [2, 4, 6], "full block {} must match 1:2:3 ratio", blk + 1);
        } else {
            assert_eq!(end - start, 5);
            assert_eq!(counts.iter().sum::<usize>(), 5);
            assert!(counts[0] <= 2 && counts[1] <= 4 && counts[2] <= 6, "truncated block counts exceed full-block cap: {counts:?}");
        }
    }
    let fixed_arrive = arrive_map(&fixed_out.arrive_constants);
    assert_eq!(fixed_arrive.get("randomization.seed").copied(), Some("350377"));
    assert_eq!(fixed_arrive.get("randomization.method").copied(), Some("permuted_block"));
    assert_eq!(fixed_arrive.get("randomization.allocation_ratio").copied(), Some("1:2:3"));
    assert_eq!(fixed_arrive.get("randomization.block_sizes").copied(), Some("12"));
    assert_eq!(fixed_arrive.get("randomization.total_subjects").copied(), Some("29"));
}

#[test]
fn variable_permuted_blocks_preserve_full_block_ratios_valid_truncated_tail_and_arrive_metadata() {
    let weights = [("Ctrl", 1_usize), ("Low", 2_usize), ("High", 3_usize)];
    let group_specs: Vec<GroupAllocationWeight> = weights.iter().map(|&(l, w)| GroupAllocationWeight::with_weight(l, w)).collect();
    let arm_names = ["Ctrl", "Low", "High"];

    // Variable block sizes [6, 12] over 41 subjects.
    let ids_41: Vec<String> = (1..=41).map(|i| format!("S-{i:02}")).collect();
    let var_cfg = SubjectRandomizationConfig {
        sheet_id: String::from("pb-variable"),
        title: String::from("Variable Permuted Block 1:2:3"),
        subject_ids: ids_41.clone(),
        groups: group_specs,
        scheme: AllocationScheme::PermutedBlock { block_sizes: vec![6, 12] },
        seed: 479_001,
    };
    let var_out = randomize_subjects_to_groups(&var_cfg).unwrap();
    assert_schedule_subject_bijection(&var_out.data_sheet, &ids_41, &arm_names).unwrap();

    let TableArchetype::MultipleVariables(vmv) = &var_out.data_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    let mut row = 0_usize;
    let mut expected_block_idx = 1_usize;
    let mut seen_sizes = BTreeSet::new();
    while row < vmv.row_count() {
        let block_idx = vmv.variables[2].cells[row].active_number().unwrap() as usize;
        let block_size = vmv.variables[3].cells[row].active_number().unwrap() as usize;
        assert_eq!(block_idx, expected_block_idx);
        assert!(block_size == 6 || block_size == 12);
        seen_sizes.insert(block_size);

        let end = (row + block_size).min(vmv.row_count());
        let mult = block_size / 6;
        let mut counts = [0_usize; 3];
        for r in row..end {
            assert_eq!(vmv.variables[2].cells[r].active_number(), Some(block_idx as f64));
            assert_eq!(vmv.variables[3].cells[r].active_number(), Some(block_size as f64));
            let arm = vmv.variables[4].cells[r].active_text().unwrap();
            let idx = arm_names.iter().position(|&a| a == arm).unwrap();
            counts[idx] += 1;
        }
        if end - row == block_size {
            assert_eq!(counts, [mult, 2 * mult, 3 * mult], "full block {block_idx} ratio mismatch");
        } else {
            assert_eq!(end, vmv.row_count(), "only the final block may be truncated");
            assert!(counts[0] <= mult && counts[1] <= 2 * mult && counts[2] <= 3 * mult, "truncated block exceeds capacities: {counts:?}");
        }
        row = end;
        expected_block_idx += 1;
    }
    assert_eq!(seen_sizes, BTreeSet::from([6, 12]));

    let var_arrive = arrive_map(&var_out.arrive_constants);
    assert_eq!(var_arrive.get("randomization.seed").copied(), Some("479001"));
    assert_eq!(var_arrive.get("randomization.method").copied(), Some("permuted_block"));
    assert_eq!(var_arrive.get("randomization.allocation_ratio").copied(), Some("1:2:3"));
    assert_eq!(var_arrive.get("randomization.block_sizes").copied(), Some("6, 12"));
    assert_eq!(var_arrive.get("randomization.total_subjects").copied(), Some("41"));
}

fn build_stratified_fixture() -> Result<(DataSheet, BTreeSet<usize>), String> {
    // 39 active rows (20 Alpha, 12 Beta, 5 Gamma, 2 Tiny) + 4 interspersed ineligible rows = 43 rows.
    let mut pattern: Vec<Option<&str>> = Vec::with_capacity(43);
    pattern.extend(std::iter::repeat_n(Some("Alpha"), 3));
    pattern.push(None); // row 3: excluded "Alpha"
    pattern.extend(std::iter::repeat_n(Some("Alpha"), 7));
    pattern.push(None); // row 11: empty stratum
    pattern.extend(std::iter::repeat_n(Some("Alpha"), 10));
    pattern.push(None); // row 22: whitespace stratum
    pattern.extend(std::iter::repeat_n(Some("Beta"), 10));
    pattern.push(None); // row 33: excluded "MissingOnly"
    pattern.extend(std::iter::repeat_n(Some("Beta"), 2));
    pattern.extend(std::iter::repeat_n(Some("Gamma"), 5));
    pattern.extend(std::iter::repeat_n(Some("Tiny"), 2));
    assert_eq!(pattern.len(), 43);

    let mut mv = MultipleVariablesTable::new(2, 43).map_err(|e| format!("{e}"))?;
    mv.variables[0].title = String::from("Stratum");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Biomarker");
    mv.variables[1].lock_role(VariableRole::Continuous);

    let mut ineligible = BTreeSet::new();
    let mut tiny_rows = Vec::new();
    for (r, slot) in pattern.iter().enumerate() {
        mv.row_titles[r] = format!("Row-{r:02}");
        match slot {
            Some(label) => {
                mv.variables[0].cells[r] = Cell::text(*label);
                if *label == "Tiny" {
                    tiny_rows.push(r);
                    mv.variables[1].cells[r] = Cell::excluded_number(1000.0 + r as f64);
                } else {
                    mv.variables[1].cells[r] = Cell::number(100.0 + r as f64);
                }
            }
            None => {
                ineligible.insert(r);
                mv.variables[1].cells[r] = Cell::number(9000.0 + r as f64);
                mv.variables[0].cells[r] = match r {
                    3 => Cell::excluded_text("Alpha"),
                    11 => Cell::empty(),
                    22 => Cell::text("   "),
                    33 => Cell::excluded_text("MissingOnly"),
                    other => return Err(format!("unexpected ineligible row index {other}")),
                };
            }
        }
    }
    assert_eq!(tiny_rows, vec![41, 42]);

    let mut sheet = DataSheet::new("strat-src", "Stratified Source", TableArchetype::MultipleVariables(mv)).map_err(|e| format!("{e}"))?;
    sheet.exclusion_notes = vec![
        CellExclusionNote {
            coord: CellCoord::new(3, 0, 0),
            reason_tag: String::from("ineligible_stratum"),
            note: String::from("excluded alpha stratum"),
            timestamp_utc: String::from("2026-11-18T00:00:00Z"),
        },
        CellExclusionNote {
            coord: CellCoord::new(33, 0, 0),
            reason_tag: String::from("missing_only"),
            note: String::from("excluded missing-only stratum"),
            timestamp_utc: String::from("2026-11-18T00:00:00Z"),
        },
        CellExclusionNote {
            coord: CellCoord::new(41, 1, 0),
            reason_tag: String::from("hemolyzed"),
            note: String::from("tiny row 41 biomarker"),
            timestamp_utc: String::from("2026-11-18T00:00:00Z"),
        },
        CellExclusionNote {
            coord: CellCoord::new(42, 1, 0),
            reason_tag: String::from("clotted"),
            note: String::from("tiny row 42 biomarker"),
            timestamp_utc: String::from("2026-11-18T00:00:00Z"),
        },
    ];
    Ok((sheet, ineligible))
}

fn assert_stratified_sample_matches(
    source: &DataSheet,
    ineligible: &BTreeSet<usize>,
    seed: u64,
    sample_size: usize,
    proportional: bool,
    expected_quotas: [(&str, usize); 4],
) -> Result<(Vec<usize>, DataSheet), String> {
    let cfg = RowSampleConfig {
        seed,
        sample_size,
        stratify_column: Some(0),
        proportional_allocation: proportional,
        output_id: Some(format!("samp-{seed}")),
        output_title: Some(format!("Sample {seed}")),
    };
    let selected = sample_row_indices(source, &cfg).map_err(|e| format!("{e}"))?;
    assert_eq!(selected.len(), sample_size);
    assert_eq!(selected.iter().copied().collect::<BTreeSet<_>>().len(), sample_size, "selected indices must be distinct");
    assert!(selected.windows(2).all(|w| w[0] < w[1]), "selected indices must remain in strictly ascending source order");
    for &idx in &selected {
        assert!(!ineligible.contains(&idx), "ineligible row {idx} must not be sampled");
    }

    let out = random_sample_rows(source, &cfg).map_err(|e| format!("{e}"))?;
    let src_mv = schedule_mv(source)?;
    let out_mv = schedule_mv(&out)?;
    assert_eq!(out_mv.row_count(), sample_size);

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (new_r, &orig_r) in selected.iter().enumerate() {
        assert_eq!(out_mv.row_titles[new_r], format!("Row-{orig_r:02}"));
        let stratum = out_mv.variables[0].cells[new_r].active_text().ok_or_else(|| format!("missing sampled stratum at row {new_r}"))?;
        let src_stratum = src_mv.variables[0].cells[orig_r].active_text().ok_or_else(|| format!("missing source stratum at row {orig_r}"))?;
        assert_eq!(stratum, src_stratum);
        assert_eq!(out_mv.variables[1].cells[new_r], src_mv.variables[1].cells[orig_r]);
        *counts.entry(stratum.to_owned()).or_insert(0) += 1;
    }
    for (label, expected_count) in expected_quotas {
        assert_eq!(counts.get(label).copied(), Some(expected_count), "stratum `{label}` count mismatch");
    }
    assert!(!counts.contains_key("MissingOnly"));

    // Verify exclusion notes were remapped strictly for selected rows and dropped for ineligible rows.
    let expected_notes: Vec<(CellCoord, &str)> = source
        .exclusion_notes
        .iter()
        .filter_map(|note| {
            selected
                .iter()
                .position(|&orig_r| orig_r == note.coord.row)
                .map(|new_r| (CellCoord::new(new_r, note.coord.col, note.coord.subcol), note.reason_tag.as_str()))
        })
        .collect();
    let actual_notes: Vec<(CellCoord, &str)> = out.exclusion_notes.iter().map(|n| (n.coord, n.reason_tag.as_str())).collect();
    assert_eq!(actual_notes, expected_notes);
    assert!(out.exclusion_notes.iter().all(|n| n.reason_tag != "ineligible_stratum" && n.reason_tag != "missing_only"));

    Ok((selected, out))
}

#[test]
fn stratified_sampling_matches_proportional_and_capacity_saturated_equal_quotas() {
    let (source, ineligible) = build_stratified_fixture().unwrap();

    // Proportional unclamped: N=15 over [20, 12, 5, 2] -> [8, 4, 2, 1].
    let (sel_prop_15, out_prop_15) =
        assert_stratified_sample_matches(&source, &ineligible, 611_953, 15, true, [("Alpha", 8), ("Beta", 4), ("Gamma", 2), ("Tiny", 1)]).unwrap();

    // Verify AnalysisSpec::Transform(RandomSample) agrees with random_sample_rows.
    let spec = AnalysisSpec::new(
        "an-strat-prop-15",
        "strat-src",
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
            seed: 611_953,
            sample_size: 15,
            stratify_column: Some(0),
            proportional_allocation: true,
        }),
    )
    .unwrap();
    let res = execute_analysis_transform(&source, &spec, &TransformContext::default()).unwrap();
    let res_sheet = res.analyzable_data.as_ref().unwrap();
    assert_eq!(res_sheet.archetype, out_prop_15.archetype);
    assert_eq!(res_sheet.exclusion_notes, out_prop_15.exclusion_notes);
    assert_eq!(res.lookup_scalar("sample_size", None), Some(15.0));
    assert_eq!(res.lookup_scalar("population_size", None), Some(43.0));
    assert!(sel_prop_15.iter().any(|&r| r == 41 || r == 42));

    // Proportional with 3-way remainder tie (26/39) + min-1 floor clamp on Tiny: [6, 4, 2, 1].
    assert_stratified_sample_matches(&source, &ineligible, 746_773, 13, true, [("Alpha", 6), ("Beta", 4), ("Gamma", 2), ("Tiny", 1)]).unwrap();

    // Equal allocation N=19 over [20, 12, 5, 2]: Tiny saturates at 2, Gamma saturates at 5 -> [6, 6, 5, 2].
    let (sel_eq_19, out_eq_19) =
        assert_stratified_sample_matches(&source, &ineligible, 882_377, 19, false, [("Alpha", 6), ("Beta", 6), ("Gamma", 5), ("Tiny", 2)]).unwrap();
    assert_eq!(sel_eq_19[17], 41);
    assert_eq!(sel_eq_19[18], 42);
    assert_eq!(
        out_eq_19.exclusion_notes.iter().map(|n| (n.coord, n.reason_tag.as_str())).collect::<Vec<_>>(),
        vec![(CellCoord::new(17, 1, 0), "hemolyzed"), (CellCoord::new(18, 1, 0), "clotted")]
    );
}

#[derive(Clone, Copy)]
struct DiscreteCdfOracle {
    name: &'static str,
    dist: SimulatedDistribution,
    upper_support: Option<f64>,
    mean: f64,
    mean_tol: f64,
    var: f64,
    var_tol: f64,
    cutoffs: &'static [(u64, f64, f64)],
}

const DISCRETE_ORACLES: &[DiscreteCdfOracle] = &[
    // Small-trial switch lower side: trials <= 32 (Bernoulli summation).
    DiscreteCdfOracle {
        name: "Binom32_Bernoulli",
        dist: SimulatedDistribution::Binomial { trials: 32, p: 0.375 },
        upper_support: Some(32.0),
        mean: 12.0,
        mean_tol: 0.096_994_845_223_857_12,
        var: 7.5,
        var_tol: 0.370_545_064_770_536_4,
        cutoffs: &[
            (8, 0.098_376_623_799_427_74, 0.010_548_156_413_885_78),
            (10, 0.296_079_529_022_410_1, 0.016_169_031_641_938_825),
            (12, 0.578_080_294_842_149_7, 0.017_491_498_212_101_674),
            (15, 0.898_145_344_144_319_8, 0.010_712_276_575_218_302),
        ],
    },
    // Small-trial switch upper side: trials > 32, n*q = 12.375 >= 10 (Hormann BTRS).
    DiscreteCdfOracle {
        name: "Binom33_Btrs",
        dist: SimulatedDistribution::Binomial { trials: 33, p: 0.375 },
        upper_support: Some(33.0),
        mean: 12.375,
        mean_tol: 0.098_498_730_956_292_01,
        var: 7.734_375,
        var_tol: 0.382_285_728_680_308,
        cutoffs: &[
            (8, 0.078_907_456_399_580_48, 0.009_548_357_652_599_065),
            (11, 0.382_055_372_260_135_6, 0.017_209_012_838_312_228),
            (12, 0.523_915_513_602_382_8, 0.017_688_486_189_426_67),
            (16, 0.929_290_688_574_562_1, 0.009_078_876_731_163_766),
        ],
    },
    // Rare-event switch lower side: trials > 32, n*q = 9.75 < 10 (geometric waiting time).
    DiscreteCdfOracle {
        name: "Binom39_Geo",
        dist: SimulatedDistribution::Binomial { trials: 39, p: 0.25 },
        upper_support: Some(39.0),
        mean: 9.75,
        mean_tol: 0.095_774_735_708_327_59,
        var: 7.312_5,
        var_tol: 0.364_706_685_300_190_35,
        cutoffs: &[
            (6, 0.111_227_266_572_013_61, 0.011_135_738_301_986_702),
            (8, 0.331_266_471_905_340_5, 0.016_669_912_213_491),
            (10, 0.619_995_164_617_854, 0.017_191_221_471_770_94),
            (13, 0.913_846_342_554_611_8, 0.009_937_827_893_641_144),
        ],
    },
    // Rare-event switch upper side: trials > 32, n*q = 10.0 >= 10 (Hormann BTRS).
    DiscreteCdfOracle {
        name: "Binom40_Btrs",
        dist: SimulatedDistribution::Binomial { trials: 40, p: 0.25 },
        upper_support: Some(40.0),
        mean: 10.0,
        mean_tol: 0.096_994_845_223_857_12,
        var: 7.5,
        var_tol: 0.374_098_442_960_941_8,
        cutoffs: &[
            (6, 0.096_224_593_481_104_96, 0.010_444_588_274_121_891),
            (9, 0.439_539_731_672_533, 0.017_578_811_421_379_147),
            (10, 0.583_904_078_028_789_9, 0.017_457_638_917_611_682),
            (14, 0.945_562_751_981_365_2, 0.008_035_469_956_903_357),
        ],
    },
    // Large-n rare event: n*q = 8.0 < 10 (geometric waiting time).
    DiscreteCdfOracle {
        name: "Binom2000_Geo",
        dist: SimulatedDistribution::Binomial { trials: 2_000, p: 0.004 },
        upper_support: Some(2_000.0),
        mean: 8.0,
        mean_tol: 0.099_975_292_947_807_85,
        var: 7.968,
        var_tol: 0.411_149_330_805_097_9,
        cutoffs: &[
            (4, 0.099_174_038_858_523_04, 0.010_586_136_014_090_034),
            (7, 0.452_681_006_369_709_8, 0.017_629_273_639_851_832),
            (8, 0.592_547_528_300_229_5, 0.017_402_758_619_222_473),
            (12, 0.936_588_363_033_097_1, 0.008_631_313_354_621_225),
        ],
    },
    // Success probability p > 0.5 complement flip on BTRS path (q = 0.25, n*q = 15 >= 10).
    DiscreteCdfOracle {
        name: "Binom60_FlipBtrs",
        dist: SimulatedDistribution::Binomial { trials: 60, p: 0.75 },
        upper_support: Some(60.0),
        mean: 45.0,
        mean_tol: 0.118_793_939_239_339_98,
        var: 11.25,
        var_tol: 0.561_933_004_287_924_3,
        cutoffs: &[
            (40, 0.092_484_270_561_278_4, 0.010_260_748_457_991_62),
            (44, 0.431_202_725_035_632_56, 0.017_540_320_499_086_08),
            (45, 0.549_430_729_647_175_5, 0.017_622_003_369_686_366),
            (49, 0.914_113_228_178_659_1, 0.009_923_872_119_288_971),
        ],
    },
    // Success probability p > 0.5 complement flip on Geo path (q = 0.125, n*q = 8 < 10).
    DiscreteCdfOracle {
        name: "Binom64_FlipGeo",
        dist: SimulatedDistribution::Binomial { trials: 64, p: 0.875 },
        upper_support: Some(64.0),
        mean: 56.0,
        mean_tol: 0.093_705_922_971_816_46,
        var: 7.0,
        var_tol: 0.354_900_714_911_505_6,
        cutoffs: &[
            (52, 0.097_349_443_007_202_02, 0.010_498_919_007_170_779),
            (55, 0.407_245_173_772_677_94, 0.017_401_374_156_949_64),
            (56, 0.556_441_699_173_960_6, 0.017_595_564_819_441_297),
            (59, 0.915_007_790_571_787, 0.009_876_884_567_234_217),
        ],
    },
    // Small Poisson rate (lambda < 30, product of uniforms).
    DiscreteCdfOracle {
        name: "Pois4p5_Small",
        dist: SimulatedDistribution::Poisson { lambda: 4.5 },
        upper_support: None,
        mean: 4.5,
        mean_tol: 0.075_131_884_043_992_93,
        var: 4.5,
        var_tol: 0.237_592_155_193_071_06,
        cutoffs: &[
            (1, 0.061_099_480_960_332_686, 0.008_482_943_466_432_498),
            (3, 0.342_295_955_834_591_05, 0.016_804_831_524_860_356),
            (4, 0.532_103_576_374_715_5, 0.017_672_214_509_382_528),
            (7, 0.913_413_528_352_644, 0.009_960_399_637_509_637),
        ],
    },
    // Poisson rate-30 switch lower side: lambda = 29.5 < 30.0 (product of uniforms).
    DiscreteCdfOracle {
        name: "Pois29p5_Below30",
        dist: SimulatedDistribution::Poisson { lambda: 29.5 },
        upper_support: None,
        mean: 29.5,
        mean_tol: 0.192_366_317_217_957_88,
        var: 29.5,
        var_tol: 1.490_092_391_370_881_7,
        cutoffs: &[
            (22, 0.094_527_893_797_569_29, 0.010_361_808_254_233_896),
            (27, 0.366_425_472_190_222_53, 0.017_065_134_087_395_37),
            (29, 0.512_288_853_581_830_6, 0.017_703_405_481_064_85),
            (37, 0.925_410_474_966_664_4, 0.009_305_166_512_108_543),
        ],
    },
    // Poisson rate-30 switch upper side: lambda = 30.0 >= 30.0 (Hormann PTRS).
    DiscreteCdfOracle {
        name: "Pois30_At30",
        dist: SimulatedDistribution::Poisson { lambda: 30.0 },
        upper_support: None,
        mean: 30.0,
        mean_tol: 0.193_989_690_447_714_25,
        var: 30.0,
        var_tol: 1.515_137_723_249_215_4,
        cutoffs: &[
            (22, 0.080_569_021_094_979_63, 0.009_639_658_142_059_99),
            (27, 0.332_869_084_045_523_34, 0.016_690_151_799_662_392),
            (30, 0.548_351_512_577_911_4, 0.017_625_758_917_468_017),
            (37, 0.910_987_007_682_256_9, 0.010_085_578_515_154_83),
        ],
    },
    // Large Poisson rate: lambda = 120.0 >= 30.0 (PTRS + Stirling factorial branch).
    DiscreteCdfOracle {
        name: "Pois120_Large",
        dist: SimulatedDistribution::Poisson { lambda: 120.0 },
        upper_support: None,
        mean: 120.0,
        mean_tol: 0.387_979_380_895_428_5,
        var: 120.0,
        var_tol: 6.023_179_652_526_152,
        cutoffs: &[
            (105, 0.090_741_413_320_436_67, 0.010_173_361_987_903_298),
            (115, 0.345_288_045_433_952_03, 0.016_839_683_679_387_94),
            (120, 0.524_253_020_835_2, 0.017_687_909_729_693_127),
            (134, 0.905_442_354_677_882_1, 0.010_363_268_493_989_45),
        ],
    },
];

#[test]
fn binomial_and_poisson_cdfs_across_regime_switches_and_moments_match_oracles() {
    let columns: Vec<SimulatedColumnSpec> =
        DISCRETE_ORACLES.iter().map(|spec| SimulatedColumnSpec { name: String::from(spec.name), distribution: spec.dist }).collect();
    let cfg = DistributionSimulationConfig {
        sheet_id: String::from("d1-sim-cdfs"),
        title: String::from("D1 Discrete Simulation CDF Oracles"),
        n_rows: STOCHASTIC_DRAWS,
        seed: STOCHASTIC_SEED,
        columns,
        use_multiple_variables: false,
    };
    let sheet = simulate_distribution_columns(&cfg).unwrap();
    let TableArchetype::Column(col_tbl) = &sheet.archetype else {
        panic!("expected Column table");
    };
    assert_eq!(col_tbl.row_count(), STOCHASTIC_DRAWS);

    for (col_idx, oracle) in DISCRETE_ORACLES.iter().enumerate() {
        let vals = col_tbl.active_column_values(col_idx);
        assert_eq!(vals.len(), STOCHASTIC_DRAWS);
        for &v in &vals {
            assert!(v.is_finite() && v >= 0.0 && v.fract() == 0.0, "{} produced non-integer/negative sample {v}", oracle.name);
            if let Some(upper) = oracle.upper_support {
                assert!(v <= upper, "{} sample {v} exceeded upper support {upper}", oracle.name);
            }
        }

        let moments = SummaryMoments::from_slice(&vals).unwrap();
        assert_approx_eq(moments.mean, oracle.mean, oracle.mean_tol, 0.0, &format!("{} sample mean", oracle.name));
        let emp_var = moments.sample_variance().unwrap();
        assert_approx_eq(emp_var, oracle.var, oracle.var_tol, 0.0, &format!("{} sample variance", oracle.name));

        for &(cutoff, expected_cdf, cdf_tol) in oracle.cutoffs {
            let count = vals.iter().filter(|&&x| x <= cutoff as f64).count();
            let emp_cdf = (count as f64) / (STOCHASTIC_DRAWS as f64);
            assert_approx_eq(emp_cdf, expected_cdf, cdf_tol, 0.0, &format!("{} empirical CDF at k={cutoff}", oracle.name));
        }
    }
}

#[test]
fn binomial_and_poisson_endpoint_distributions_are_exact_constants() {
    let endpoint_specs = [
        ("BinomP0", SimulatedDistribution::Binomial { trials: 50, p: 0.0 }, 0.0_f64),
        ("BinomP1", SimulatedDistribution::Binomial { trials: 50, p: 1.0 }, 50.0_f64),
        ("BinomTrials0", SimulatedDistribution::Binomial { trials: 0, p: 0.5 }, 0.0_f64),
        ("PoisLambda0", SimulatedDistribution::Poisson { lambda: 0.0 }, 0.0_f64),
    ];

    for use_multiple_variables in [false, true] {
        let cfg = DistributionSimulationConfig {
            sheet_id: String::from("d1-sim-endpoints"),
            title: String::from("Discrete Endpoints"),
            n_rows: ENDPOINT_DRAWS,
            seed: ENDPOINT_SEED,
            columns: endpoint_specs.iter().map(|&(name, dist, _)| SimulatedColumnSpec { name: String::from(name), distribution: dist }).collect(),
            use_multiple_variables,
        };
        let sheet = simulate_distribution_columns(&cfg).unwrap();
        for (col_idx, &(name, _, expected)) in endpoint_specs.iter().enumerate() {
            let vals: Vec<f64> = match &sheet.archetype {
                TableArchetype::Column(t) => t.active_column_values(col_idx),
                TableArchetype::MultipleVariables(mv) => mv.variables[col_idx].cells.iter().filter_map(Cell::active_number).collect(),
                other => panic!("unexpected archetype {other:?}"),
            };
            assert_eq!(vals.len(), ENDPOINT_DRAWS);
            assert!(vals.iter().all(|&x| x == expected && x.is_sign_positive()), "{name} (mv={use_multiple_variables}) expected constant {expected}");
        }
    }
}
