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
//! Across `M = 260` planned stochastic comparisons (`44` empirical CDF cutoffs, `11` sample means,
//! and `11` sample variances across 11 distribution columns at `N = 25_000`, `seed = 2026_1118`,
//! `z = 5.6`, plus `194` multi-seed randomization & row-sampling uniformity checks across
//! `K = 12_000` / `6_000` seeds at `z = 6.0`):
//! - Exact discrete two-sided `Binomial(25_000, F(k))` tail sum over all 44 CDF checks: `9.9925e-7`.
//! - Exact discrete two-sided `Binomial(25_000 * n, p)` / `Poisson(25_000 * lambda)` tail sum over
//!   all 11 mean checks: `2.3612e-7`.
//! - Asymptotic two-sided normal tail at `z = 5.6` is `2.144e-8` per variance check; budgeting
//!   `1.0e-7` per variance check (`1.1000e-6` total across 11 checks) for finite-sample margin.
//! - Exact discrete two-sided Binomial tail sum across all `194` multi-seed uniformity & inclusion
//!   checks (`complete-balanced` and `permuted-block` multiset permutations, truncated-tail arm
//!   marginals, variable block-size selection, and unstratified/stratified per-row inclusion
//!   frequencies): `9.7364e-7`.
//! - Combined Bonferroni family-wise upper bound: `3.3090e-6 <= 1.0e-5`.
//!
//! Deterministic schedule, stratified-quota, support, and endpoint assertions (`p = 0`, `p = 1`,
//! `trials = 0`, `lambda = 0`) have zero random false-positive probability.

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, MultipleVariablesTable, TableArchetype, TransformAnalysisSpec, VariableRole,
};
use curvebook_math::sum::SummaryMoments;
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, RowSampleConfig, SimulatedColumnSpec, SimulatedDistribution,
    SubjectRandomizationConfig, TransformContext, execute_analysis_transform, random_sample_rows, randomize_subjects_to_groups, sample_row_indices,
    simulate_distribution_columns,
};

use crate::common::{as_col, as_mv, empty, excl, excl_note, excl_txt, mv_sheet, num, txt};

const STOCHASTIC_DRAWS: usize = 25_000;
const STOCHASTIC_SEED: u64 = 2026_1118;
const ENDPOINT_DRAWS: usize = 512;
const ENDPOINT_SEED: u64 = 2026_0001;

fn assert_arrive_constants(actual: &[(String, String)], expected: &[(&str, &str)]) {
    let map: BTreeMap<&str, &str> = actual.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    for &(k, v) in expected {
        assert_eq!(map.get(k).copied(), Some(v), "ARRIVE constant `{k}` mismatch");
    }
}

fn schedule_arm_counts(sheet: &DataSheet) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for cell in &as_mv(sheet).variables[4].cells {
        *counts.entry(cell.active_text().expect("active group label").to_owned()).or_insert(0) += 1;
    }
    counts
}

fn assert_schedule_subject_bijection(sheet: &DataSheet, expected_ids: &[String], expected_arms: &[&str]) {
    let mv = as_mv(sheet);
    assert_eq!(
        mv.variables.iter().map(|v| v.title.as_str()).collect::<Vec<_>>(),
        ["Subject_ID", "Allocation_Order", "Block_Index", "Block_Size", "Assigned_Group"]
    );
    assert_eq!(
        mv.variables.iter().map(|v| v.role).collect::<Vec<_>>(),
        [VariableRole::Categorical, VariableRole::Continuous, VariableRole::Continuous, VariableRole::Continuous, VariableRole::Categorical]
    );
    assert_eq!(mv.row_count(), expected_ids.len());

    let mut seen = BTreeSet::new();
    for (row, expected_id) in expected_ids.iter().enumerate() {
        let sid = mv.variables[0].cells[row].active_text().expect("subject id");
        assert_eq!(sid, expected_id.trim());
        assert!(seen.insert(sid.to_owned()), "duplicate subject assignment for {sid}");
        assert_eq!(mv.variables[1].cells[row].active_number(), Some((row + 1) as f64));
        let arm = mv.variables[4].cells[row].active_text().expect("assigned arm");
        assert!(expected_arms.contains(&arm), "unexpected arm `{arm}` at row {row}");
    }
}

fn arm_abc_index(cell: &Cell) -> usize {
    match cell.active_text() {
        Some("A") => 0,
        Some("B") => 1,
        Some("C") => 2,
        other => panic!("unexpected arm {other:?}"),
    }
}

#[test]
fn complete_balanced_unequal_weights_with_tied_remainders_match_hamilton_quotas() {
    // Case 1: N=15, weights [4,3,2,1] -> floors [6,4,3,1], tied remainders [0,5,0,5] -> [6,5,3,1].
    // Case 2: N=9, weights [3,2,2] -> floors [3,2,2], remainders [6,4,4], leftover 2 -> [4,3,2].
    let ids_15: Vec<String> = (1..=15).map(|i| format!("SUBJ-{i:02}")).collect();
    let cfg_15 = SubjectRandomizationConfig {
        sheet_id: String::from("cb-tied-4arm"),
        title: String::from("Complete Balanced 4-Arm Tied Remainders"),
        subject_ids: ids_15.clone(),
        groups: [("ArmA", 4), ("ArmB", 3), ("ArmC", 2), ("ArmD", 1)].into_iter().map(|(l, w)| GroupAllocationWeight::with_weight(l, w)).collect(),
        scheme: AllocationScheme::CompleteBalanced,
        seed: 104_729,
    };
    let out_15 = randomize_subjects_to_groups(&cfg_15).unwrap();
    assert_schedule_subject_bijection(&out_15.data_sheet, &ids_15, &["ArmA", "ArmB", "ArmC", "ArmD"]);
    let counts_15 = schedule_arm_counts(&out_15.data_sheet);
    for (arm, exp) in [("ArmA", 6), ("ArmB", 5), ("ArmC", 3), ("ArmD", 1)] {
        assert_eq!(counts_15.get(arm).copied(), Some(exp));
    }
    let mv15 = as_mv(&out_15.data_sheet);
    assert!((0..15).all(|r| mv15.variables[2].cells[r].active_number() == Some(1.0) && mv15.variables[3].cells[r].active_number() == Some(15.0)));
    assert_arrive_constants(
        &out_15.arrive_constants,
        &[
            ("randomization.seed", "104729"),
            ("randomization.method", "complete_balanced"),
            ("randomization.allocation_ratio", "4:3:2:1"),
            ("randomization.block_sizes", "15"),
            ("randomization.total_subjects", "15"),
        ],
    );

    let cfg_9 = SubjectRandomizationConfig::from_subject_count(
        "cb-tied-3arm",
        "Complete Balanced 3-Arm Tied Remainders",
        9,
        [("High", 3), ("Mid", 2), ("Low", 2)].into_iter().map(|(l, w)| GroupAllocationWeight::with_weight(l, w)).collect(),
        AllocationScheme::CompleteBalanced,
        224_737,
    );
    let out_9 = randomize_subjects_to_groups(&cfg_9).unwrap();
    let ids_9: Vec<String> = (1..=9).map(|i| i.to_string()).collect();
    assert_schedule_subject_bijection(&out_9.data_sheet, &ids_9, &["High", "Mid", "Low"]);
    let counts_9 = schedule_arm_counts(&out_9.data_sheet);
    for (arm, exp) in [("High", 4), ("Mid", 3), ("Low", 2)] {
        assert_eq!(counts_9.get(arm).copied(), Some(exp));
    }
}

fn verify_permuted_block_123(id: &str, ids: Vec<String>, block_sizes: Vec<usize>, seed: u64, expected_sizes_str: &str) -> BTreeSet<usize> {
    let arm_names = ["Ctrl", "Low", "High"];
    let cfg = SubjectRandomizationConfig {
        sheet_id: id.to_owned(),
        title: id.to_owned(),
        subject_ids: ids.clone(),
        groups: [("Ctrl", 1), ("Low", 2), ("High", 3)].into_iter().map(|(l, w)| GroupAllocationWeight::with_weight(l, w)).collect(),
        scheme: AllocationScheme::PermutedBlock { block_sizes: block_sizes.clone() },
        seed,
    };
    let out = randomize_subjects_to_groups(&cfg).unwrap();
    assert_eq!(out, randomize_subjects_to_groups(&cfg).unwrap());
    assert_schedule_subject_bijection(&out.data_sheet, &ids, &arm_names);

    let mv = as_mv(&out.data_sheet);
    let (mut row, mut expected_blk, mut seen_sizes) = (0_usize, 1_usize, BTreeSet::new());
    while row < mv.row_count() {
        let block_idx = mv.variables[2].cells[row].active_number().unwrap() as usize;
        let block_size = mv.variables[3].cells[row].active_number().unwrap() as usize;
        assert_eq!(block_idx, expected_blk);
        assert!(block_sizes.contains(&block_size));
        seen_sizes.insert(block_size);

        let end = (row + block_size).min(mv.row_count());
        let mult = block_size / 6;
        let mut counts = [0_usize; 3];
        for r in row..end {
            assert_eq!(mv.variables[2].cells[r].active_number(), Some(block_idx as f64));
            assert_eq!(mv.variables[3].cells[r].active_number(), Some(block_size as f64));
            let arm = mv.variables[4].cells[r].active_text().unwrap();
            counts[arm_names.iter().position(|&a| a == arm).unwrap()] += 1;
        }
        if end - row == block_size {
            assert_eq!(counts, [mult, 2 * mult, 3 * mult], "full block {block_idx} ratio mismatch");
        } else {
            assert_eq!(end, mv.row_count(), "only the final block may be truncated");
            assert!(counts[0] <= mult && counts[1] <= 2 * mult && counts[2] <= 3 * mult, "truncated block exceeds capacities: {counts:?}");
        }
        row = end;
        expected_blk += 1;
    }
    assert_arrive_constants(
        &out.arrive_constants,
        &[
            ("randomization.seed", &seed.to_string()),
            ("randomization.method", "permuted_block"),
            ("randomization.allocation_ratio", "1:2:3"),
            ("randomization.block_sizes", expected_sizes_str),
            ("randomization.total_subjects", &ids.len().to_string()),
        ],
    );
    seen_sizes
}

#[test]
fn fixed_permuted_blocks_preserve_full_block_ratios_valid_truncated_tail_and_arrive_metadata() {
    let ids: Vec<String> = (1..=29).map(|i| format!("M-{i:02}")).collect();
    assert_eq!(verify_permuted_block_123("pb-fixed", ids, vec![12], 350_377, "12"), BTreeSet::from([12]));
}

#[test]
fn variable_permuted_blocks_preserve_full_block_ratios_valid_truncated_tail_and_arrive_metadata() {
    let ids: Vec<String> = (1..=41).map(|i| format!("S-{i:02}")).collect();
    assert_eq!(verify_permuted_block_123("pb-variable", ids, vec![6, 12], 479_001, "6, 12"), BTreeSet::from([6, 12]));
}

fn build_stratified_fixture() -> (DataSheet, BTreeSet<usize>) {
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

    let mut mv = MultipleVariablesTable::new(2, 43).unwrap();
    (mv.variables[0].title, mv.variables[1].title) = (String::from("Stratum"), String::from("Biomarker"));
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].lock_role(VariableRole::Continuous);

    let mut ineligible = BTreeSet::new();
    for (r, slot) in pattern.iter().enumerate() {
        mv.row_titles[r] = format!("Row-{r:02}");
        match *slot {
            Some("Tiny") => {
                mv.variables[0].cells[r] = txt("Tiny");
                mv.variables[1].cells[r] = excl(1000.0 + r as f64);
            }
            Some(label) => {
                mv.variables[0].cells[r] = txt(label);
                mv.variables[1].cells[r] = num(100.0 + r as f64);
            }
            None => {
                ineligible.insert(r);
                mv.variables[1].cells[r] = num(9000.0 + r as f64);
                mv.variables[0].cells[r] = match r {
                    3 => excl_txt("Alpha"),
                    11 => empty(),
                    22 => txt("   "),
                    33 => excl_txt("MissingOnly"),
                    other => panic!("unexpected ineligible row {other}"),
                };
            }
        }
    }

    let mut s = DataSheet::new("strat-src", "Stratified Source", TableArchetype::MultipleVariables(mv)).unwrap();
    s.exclusion_notes = vec![
        excl_note(CellCoord::new(3, 0, 0), "ineligible_stratum", "excluded alpha stratum"),
        excl_note(CellCoord::new(33, 0, 0), "missing_only", "excluded missing-only stratum"),
        excl_note(CellCoord::new(41, 1, 0), "hemolyzed", "tiny row 41 biomarker"),
        excl_note(CellCoord::new(42, 1, 0), "clotted", "tiny row 42 biomarker"),
    ];
    (s, ineligible)
}

fn assert_stratified_sample_matches(
    source: &DataSheet,
    ineligible: &BTreeSet<usize>,
    seed: u64,
    sample_size: usize,
    proportional: bool,
    expected_quotas: [(&str, usize); 4],
) -> (Vec<usize>, DataSheet) {
    let cfg = RowSampleConfig {
        seed,
        sample_size,
        stratify_column: Some(0),
        proportional_allocation: proportional,
        output_id: Some(format!("samp-{seed}")),
        output_title: Some(format!("Sample {seed}")),
    };
    let selected = sample_row_indices(source, &cfg).unwrap();
    assert_eq!(selected.len(), sample_size);
    assert_eq!(selected.iter().copied().collect::<BTreeSet<_>>().len(), sample_size);
    assert!(selected.windows(2).all(|w| w[0] < w[1]));
    assert!(selected.iter().all(|idx| !ineligible.contains(idx)));

    let out = random_sample_rows(source, &cfg).unwrap();
    let (src_mv, out_mv) = (as_mv(source), as_mv(&out));
    assert_eq!(out_mv.row_count(), sample_size);

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (new_r, &orig_r) in selected.iter().enumerate() {
        assert_eq!(out_mv.row_titles[new_r], format!("Row-{orig_r:02}"));
        let stratum = out_mv.variables[0].cells[new_r].active_text().unwrap();
        assert_eq!(stratum, src_mv.variables[0].cells[orig_r].active_text().unwrap());
        assert_eq!(out_mv.variables[1].cells[new_r], src_mv.variables[1].cells[orig_r]);
        *counts.entry(stratum.to_owned()).or_insert(0) += 1;
    }
    for (label, expected_count) in expected_quotas {
        assert_eq!(counts.get(label).copied(), Some(expected_count), "stratum `{label}` mismatch");
    }
    assert!(!counts.contains_key("MissingOnly"));

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
    (selected, out)
}

#[test]
fn stratified_sampling_matches_proportional_and_capacity_saturated_equal_quotas() {
    let (source, ineligible) = build_stratified_fixture();

    // Proportional unclamped N=15 -> [8, 4, 2, 1] + AnalysisSpec parity.
    let (sel_prop_15, out_prop_15) =
        assert_stratified_sample_matches(&source, &ineligible, 611_953, 15, true, [("Alpha", 8), ("Beta", 4), ("Gamma", 2), ("Tiny", 1)]);
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
    assert_eq!(
        (&res_sheet.archetype, &res_sheet.exclusion_notes, res.lookup_scalar("sample_size", None), res.lookup_scalar("population_size", None)),
        (&out_prop_15.archetype, &out_prop_15.exclusion_notes, Some(15.0), Some(43.0))
    );
    assert!(sel_prop_15.iter().any(|&r| r == 41 || r == 42));

    // Proportional 3-way remainder tie + min-1 floor clamp on Tiny: N=13 -> [6, 4, 2, 1].
    assert_stratified_sample_matches(&source, &ineligible, 746_773, 13, true, [("Alpha", 6), ("Beta", 4), ("Gamma", 2), ("Tiny", 1)]);

    // Equal allocation N=19: Tiny saturates at 2, Gamma at 5 -> [6, 6, 5, 2].
    let (sel_eq_19, out_eq_19) =
        assert_stratified_sample_matches(&source, &ineligible, 882_377, 19, false, [("Alpha", 6), ("Beta", 6), ("Gamma", 5), ("Tiny", 2)]);
    assert_eq!((sel_eq_19[17], sel_eq_19[18]), (41, 42));
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

impl DiscreteCdfOracle {
    const fn binom(name: &'static str, trials: u64, p: f64, mean: (f64, f64), var: (f64, f64), cutoffs: &'static [(u64, f64, f64)]) -> Self {
        Self {
            name,
            dist: SimulatedDistribution::Binomial { trials, p },
            upper_support: Some(trials as f64),
            mean: mean.0,
            mean_tol: mean.1,
            var: var.0,
            var_tol: var.1,
            cutoffs,
        }
    }
    const fn pois(name: &'static str, lambda: f64, mean_tol: f64, var_tol: f64, cutoffs: &'static [(u64, f64, f64)]) -> Self {
        Self { name, dist: SimulatedDistribution::Poisson { lambda }, upper_support: None, mean: lambda, mean_tol, var: lambda, var_tol, cutoffs }
    }
}

const DISCRETE_ORACLES: &[DiscreteCdfOracle] = &[
    // Small-trial switch lower/upper (Bernoulli <= 32 vs Hormann BTRS > 32), rare-event switch (<10 Geo vs >=10 BTRS),
    // large-n rare Geo, p > 0.5 complement flip (BTRS & Geo), and Poisson rate-30 switch + Stirling branch.
    DiscreteCdfOracle::binom(
        "Binom32_Bernoulli",
        32,
        0.375,
        (12.0, 0.096_994_845_223_857_12),
        (7.5, 0.370_545_064_770_536_4),
        &[
            (8, 0.098_376_623_799_427_74, 0.010_548_156_413_885_78),
            (10, 0.296_079_529_022_410_1, 0.016_169_031_641_938_825),
            (12, 0.578_080_294_842_149_7, 0.017_491_498_212_101_674),
            (15, 0.898_145_344_144_319_8, 0.010_712_276_575_218_302),
        ],
    ),
    DiscreteCdfOracle::binom(
        "Binom33_Btrs",
        33,
        0.375,
        (12.375, 0.098_498_730_956_292_01),
        (7.734_375, 0.382_285_728_680_308),
        &[
            (8, 0.078_907_456_399_580_48, 0.009_548_357_652_599_065),
            (11, 0.382_055_372_260_135_6, 0.017_209_012_838_312_228),
            (12, 0.523_915_513_602_382_8, 0.017_688_486_189_426_67),
            (16, 0.929_290_688_574_562_1, 0.009_078_876_731_163_766),
        ],
    ),
    DiscreteCdfOracle::binom(
        "Binom39_Geo",
        39,
        0.25,
        (9.75, 0.095_774_735_708_327_59),
        (7.312_5, 0.364_706_685_300_190_35),
        &[
            (6, 0.111_227_266_572_013_61, 0.011_135_738_301_986_702),
            (8, 0.331_266_471_905_340_5, 0.016_669_912_213_491),
            (10, 0.619_995_164_617_854, 0.017_191_221_471_770_94),
            (13, 0.913_846_342_554_611_8, 0.009_937_827_893_641_144),
        ],
    ),
    DiscreteCdfOracle::binom(
        "Binom40_Btrs",
        40,
        0.25,
        (10.0, 0.096_994_845_223_857_12),
        (7.5, 0.374_098_442_960_941_8),
        &[
            (6, 0.096_224_593_481_104_96, 0.010_444_588_274_121_891),
            (9, 0.439_539_731_672_533, 0.017_578_811_421_379_147),
            (10, 0.583_904_078_028_789_9, 0.017_457_638_917_611_682),
            (14, 0.945_562_751_981_365_2, 0.008_035_469_956_903_357),
        ],
    ),
    DiscreteCdfOracle::binom(
        "Binom2000_Geo",
        2_000,
        0.004,
        (8.0, 0.099_975_292_947_807_85),
        (7.968, 0.411_149_330_805_097_9),
        &[
            (4, 0.099_174_038_858_523_04, 0.010_586_136_014_090_034),
            (7, 0.452_681_006_369_709_8, 0.017_629_273_639_851_832),
            (8, 0.592_547_528_300_229_5, 0.017_402_758_619_222_473),
            (12, 0.936_588_363_033_097_1, 0.008_631_313_354_621_225),
        ],
    ),
    DiscreteCdfOracle::binom(
        "Binom60_FlipBtrs",
        60,
        0.75,
        (45.0, 0.118_793_939_239_339_98),
        (11.25, 0.561_933_004_287_924_3),
        &[
            (40, 0.092_484_270_561_278_4, 0.010_260_748_457_991_62),
            (44, 0.431_202_725_035_632_56, 0.017_540_320_499_086_08),
            (45, 0.549_430_729_647_175_5, 0.017_622_003_369_686_366),
            (49, 0.914_113_228_178_659_1, 0.009_923_872_119_288_971),
        ],
    ),
    DiscreteCdfOracle::binom(
        "Binom64_FlipGeo",
        64,
        0.875,
        (56.0, 0.093_705_922_971_816_46),
        (7.0, 0.354_900_714_911_505_6),
        &[
            (52, 0.097_349_443_007_202_02, 0.010_498_919_007_170_779),
            (55, 0.407_245_173_772_677_94, 0.017_401_374_156_949_64),
            (56, 0.556_441_699_173_960_6, 0.017_595_564_819_441_297),
            (59, 0.915_007_790_571_787, 0.009_876_884_567_234_217),
        ],
    ),
    DiscreteCdfOracle::pois(
        "Pois4p5_Small",
        4.5,
        0.075_131_884_043_992_93,
        0.237_592_155_193_071_06,
        &[
            (1, 0.061_099_480_960_332_686, 0.008_482_943_466_432_498),
            (3, 0.342_295_955_834_591_05, 0.016_804_831_524_860_356),
            (4, 0.532_103_576_374_715_5, 0.017_672_214_509_382_528),
            (7, 0.913_413_528_352_644, 0.009_960_399_637_509_637),
        ],
    ),
    DiscreteCdfOracle::pois(
        "Pois29p5_Below30",
        29.5,
        0.192_366_317_217_957_88,
        1.490_092_391_370_881_7,
        &[
            (22, 0.094_527_893_797_569_29, 0.010_361_808_254_233_896),
            (27, 0.366_425_472_190_222_53, 0.017_065_134_087_395_37),
            (29, 0.512_288_853_581_830_6, 0.017_703_405_481_064_85),
            (37, 0.925_410_474_966_664_4, 0.009_305_166_512_108_543),
        ],
    ),
    DiscreteCdfOracle::pois(
        "Pois30_At30",
        30.0,
        0.193_989_690_447_714_25,
        1.515_137_723_249_215_4,
        &[
            (22, 0.080_569_021_094_979_63, 0.009_639_658_142_059_99),
            (27, 0.332_869_084_045_523_34, 0.016_690_151_799_662_392),
            (30, 0.548_351_512_577_911_4, 0.017_625_758_917_468_017),
            (37, 0.910_987_007_682_256_9, 0.010_085_578_515_154_83),
        ],
    ),
    DiscreteCdfOracle::pois(
        "Pois120_Large",
        120.0,
        0.387_979_380_895_428_5,
        6.023_179_652_526_152,
        &[
            (105, 0.090_741_413_320_436_67, 0.010_173_361_987_903_298),
            (115, 0.345_288_045_433_952_03, 0.016_839_683_679_387_94),
            (120, 0.524_253_020_835_2, 0.017_687_909_729_693_127),
            (134, 0.905_442_354_677_882_1, 0.010_363_268_493_989_45),
        ],
    ),
];

#[test]
fn binomial_and_poisson_cdfs_across_regime_switches_and_moments_match_oracles() {
    let cfg = DistributionSimulationConfig {
        sheet_id: String::from("d1-sim-cdfs"),
        title: String::from("D1 Discrete Simulation CDF Oracles"),
        n_rows: STOCHASTIC_DRAWS,
        seed: STOCHASTIC_SEED,
        columns: DISCRETE_ORACLES.iter().map(|spec| SimulatedColumnSpec { name: String::from(spec.name), distribution: spec.dist }).collect(),
        use_multiple_variables: false,
    };
    let s = simulate_distribution_columns(&cfg).unwrap();
    let col_tbl = as_col(&s);
    assert_eq!(col_tbl.row_count(), STOCHASTIC_DRAWS);

    for (col_idx, oracle) in DISCRETE_ORACLES.iter().enumerate() {
        let vals = col_tbl.active_column_values(col_idx);
        assert_eq!(vals.len(), STOCHASTIC_DRAWS);
        for &v in &vals {
            assert!(v.is_finite() && v >= 0.0 && v.fract() == 0.0, "{} non-integer/neg {v}", oracle.name);
            if let Some(upper) = oracle.upper_support {
                assert!(v <= upper, "{} sample {v} > {upper}", oracle.name);
            }
        }
        let moments = SummaryMoments::from_slice(&vals).unwrap();
        assert_approx_eq(moments.mean, oracle.mean, oracle.mean_tol, 0.0, &format!("{} sample mean", oracle.name));
        assert_approx_eq(moments.sample_variance().unwrap(), oracle.var, oracle.var_tol, 0.0, &format!("{} sample variance", oracle.name));
        for &(cutoff, expected_cdf, cdf_tol) in oracle.cutoffs {
            let emp_cdf = (vals.iter().filter(|&&x| x <= cutoff as f64).count() as f64) / (STOCHASTIC_DRAWS as f64);
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
        let s = simulate_distribution_columns(&cfg).unwrap();
        for (col_idx, &(name, _, expected)) in endpoint_specs.iter().enumerate() {
            let vals: Vec<f64> = match &s.archetype {
                TableArchetype::Column(t) => t.active_column_values(col_idx),
                TableArchetype::MultipleVariables(mv) => mv.variables[col_idx].cells.iter().filter_map(Cell::active_number).collect(),
                other => panic!("unexpected archetype {other:?}"),
            };
            assert_eq!(vals.len(), ENDPOINT_DRAWS);
            assert!(vals.iter().all(|&x| x == expected && x.is_sign_positive()), "{name} (mv={use_multiple_variables}) expected {expected}");
        }
    }
}

#[test]
fn multi_seed_multiset_permutation_uniformity() {
    let ids_6: Vec<String> = (1..=6).map(|i| format!("S{i}")).collect();
    let groups_123: Vec<_> = [("A", 1), ("B", 2), ("C", 3)].into_iter().map(|(l, w)| GroupAllocationWeight::with_weight(l, w)).collect();
    for (label, scheme) in
        [("CompleteBalanced", AllocationScheme::CompleteBalanced), ("PermutedBlock6", AllocationScheme::PermutedBlock { block_sizes: vec![6] })]
    {
        let mut perm_counts: BTreeMap<[u8; 6], usize> = BTreeMap::new();
        for seed in 1..=12_000_u64 {
            let cfg = SubjectRandomizationConfig {
                sheet_id: String::from("unif-6"),
                title: String::from("Unif 6"),
                subject_ids: ids_6.clone(),
                groups: groups_123.clone(),
                scheme: scheme.clone(),
                seed,
            };
            let out = randomize_subjects_to_groups(&cfg).unwrap();
            let mv = as_mv(&out.data_sheet);
            let key: [u8; 6] = std::array::from_fn(|r| arm_abc_index(&mv.variables[4].cells[r]) as u8);
            *perm_counts.entry(key).or_insert(0) += 1;
        }
        assert_eq!(perm_counts.len(), 60, "{label} must visit all 60 multiset permutations");
        for (perm, count) in perm_counts {
            assert!((116..=284).contains(&count), "{label} permutation {perm:?} count {count} outside z=6.0 bounds [116, 284]");
        }
    }
}

#[test]
fn multi_seed_permuted_block_marginals_and_variable_blocks() {
    let groups_123: Vec<_> = [("A", 1), ("B", 2), ("C", 3)].into_iter().map(|(l, w)| GroupAllocationWeight::with_weight(l, w)).collect();
    let ids_8: Vec<String> = (1..=8).map(|i| format!("T{i}")).collect();
    let mut pos_arm_counts = [[0_usize; 3]; 8];
    for seed in 1..=12_000_u64 {
        let cfg = SubjectRandomizationConfig {
            sheet_id: String::from("unif-trunc"),
            title: String::from("Unif Truncated"),
            subject_ids: ids_8.clone(),
            groups: groups_123.clone(),
            scheme: AllocationScheme::PermutedBlock { block_sizes: vec![6] },
            seed,
        };
        let out = randomize_subjects_to_groups(&cfg).unwrap();
        let mv = as_mv(&out.data_sheet);
        for (pos, row_counts) in pos_arm_counts.iter_mut().enumerate() {
            row_counts[arm_abc_index(&mv.variables[4].cells[pos])] += 1;
        }
    }
    let marginal_bounds = [(1756, 2244), (3691, 4309), (5672, 6328)];
    for (pos, counts) in pos_arm_counts.iter().enumerate() {
        for (arm, &cnt) in counts.iter().enumerate() {
            let (lo, hi) = marginal_bounds[arm];
            assert!((lo..=hi).contains(&cnt), "pos {pos} arm {arm} count {cnt} outside z=6.0 bounds [{lo}, {hi}]");
        }
    }

    let ids_12: Vec<String> = (1..=12).map(|i| format!("V{i}")).collect();
    let block6_first_count = (1..=12_000_u64)
        .filter(|&seed| {
            let cfg = SubjectRandomizationConfig {
                sheet_id: String::from("unif-varblk"),
                title: String::from("Unif VarBlk"),
                subject_ids: ids_12.clone(),
                groups: groups_123.clone(),
                scheme: AllocationScheme::PermutedBlock { block_sizes: vec![6, 12] },
                seed,
            };
            as_mv(&randomize_subjects_to_groups(&cfg).unwrap().data_sheet).variables[3].cells[0].active_number() == Some(6.0)
        })
        .count();
    assert!((5672..=6328).contains(&block6_first_count), "variable first-block size=6 count {block6_first_count} outside z=6.0 bounds [5672, 6328]");
}

fn tally_inclusions(sheet: &DataSheet, n_rows: usize, n_seeds: u64, sample_size: usize, stratify_column: Option<usize>) -> (Vec<usize>, usize) {
    let mut incl = vec![0_usize; n_rows];
    let mut distinct = BTreeSet::new();
    for seed in 1..=n_seeds {
        let picked = sample_row_indices(
            sheet,
            &RowSampleConfig { seed, sample_size, stratify_column, proportional_allocation: true, output_id: None, output_title: None },
        )
        .unwrap();
        assert_eq!(picked.len(), sample_size);
        distinct.insert(picked.clone());
        for r in picked {
            incl[r] += 1;
        }
    }
    (incl, distinct.len())
}

#[test]
fn multi_seed_unstratified_and_stratified_inclusion_uniformity() {
    // 1. Unstratified row sampling (k = 3 from 10 rows) across 6_000 seeds.
    let vals: Vec<_> = (1..=10).map(|r| num(r as f64)).collect();
    let unstrat_sheet = mv_sheet("unif-unstrat", &[("Measure", &vals)]);
    let (unstrat_incl, distinct_u) = tally_inclusions(&unstrat_sheet, 10, 6_000, 3, None);
    assert!(distinct_u >= 115, "expected broad distinct sample coverage, got {distinct_u}");
    for (r, cnt) in unstrat_incl.into_iter().enumerate() {
        assert!((1588..=2012).contains(&cnt), "unstrat row {r} count {cnt} outside z=6.0 bounds [1588, 2012]");
    }

    // 2. Stratified proportional sampling (S = 13 on [20, 12, 5, 2] + 4 ineligible rows) across 6_000 seeds.
    let (strat_sheet, ineligible) = build_stratified_fixture();
    let (strat_incl, distinct_s) = tally_inclusions(&strat_sheet, 43, 6_000, 13, Some(0));
    assert!(distinct_s >= 5_000, "expected distinct stratified samples, got {distinct_s}");
    let smv = as_mv(&strat_sheet);
    for (r, cnt) in strat_incl.into_iter().enumerate() {
        if ineligible.contains(&r) {
            assert_eq!(cnt, 0, "ineligible row {r} must never be sampled");
            continue;
        }
        let (lo, hi) = match smv.variables[0].cells[r].active_text() {
            Some("Alpha") => (1588, 2012),
            Some("Beta") => (1781, 2219),
            Some("Gamma") => (2173, 2627),
            Some("Tiny") => (2768, 3232),
            other => panic!("unexpected stratum {other:?}"),
        };
        assert!((lo..=hi).contains(&cnt), "stratified row {r} count {cnt} outside z=6.0 bounds [{lo}, {hi}]");
    }
}
