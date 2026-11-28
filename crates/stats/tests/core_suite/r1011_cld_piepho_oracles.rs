//! D1 Oracle & Structural Verification Suite for Compact Letter Display (`R10.11`).
//!
//! References:
//! - Piepho, H.-P. (2004). "An Algorithm for a Letter-Based Representation of
//!   All-Pairwise Comparisons." *Journal of Computational and Graphical Statistics*,
//!   13(2), 456–466. <https://doi.org/10.1198/1061860043515>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, MultiplicityMethod, MultiplicitySpec,
    OneWayAnovaSpec, OneWayMethod, PairwiseComparisonRecord, ResultSheet, SheetId, TableArchetype,
};
use curvebook_stats::StatsError;
use curvebook_stats::anova::run_one_way_anova;
use curvebook_stats::cld::{
    CldGroupInput, CldOptions, CldOrdering, CldPairwiseInput, CldResult, MAX_CLD_COLUMNS, column_letter_token, compute_cld, compute_cld_from_details,
    compute_cld_from_matrix, compute_cld_from_records, compute_cld_from_result_sheet,
};
use curvebook_stats::multiplicity::{GroupComparisonInput, compare_groups_pairwise};

fn assert_cld_invariants(res: &CldResult, groups: &[CldGroupInput], is_sig: impl Fn(usize, usize) -> bool, ordering: CldOrdering) {
    assert_eq!(res.sorted_assignments.len(), groups.len(), "group assignment count mismatch");
    assert_eq!(res.columns.len(), res.letter_count);

    // 1. Monotonic sorting check
    for win in res.sorted_assignments.windows(2) {
        let a = &win[0];
        let b = &win[1];
        match ordering {
            CldOrdering::DescendingMean => assert!(a.mean >= b.mean, "descending order violated: {} < {}", a.mean, b.mean),
            CldOrdering::AscendingMean => assert!(a.mean <= b.mean, "ascending order violated: {} > {}", a.mean, b.mean),
        }
    }

    // 2. Every group receives >= 1 valid letter and rank matches matrix rows.
    let mut seen = BTreeSet::new();
    for (rank, a) in res.sorted_assignments.iter().enumerate() {
        assert!(seen.insert(a.group_index), "duplicate group index {}", a.group_index);
        assert_eq!(a.rank, rank);
        assert!(!a.letters.is_empty(), "group {} received empty letters", a.group_index);
        assert!(!a.letter_columns.is_empty(), "group {} received empty column indices", a.group_index);
        for &col_idx in &a.letter_columns {
            assert!(col_idx < res.letter_count);
            assert!(res.columns[col_idx][rank], "matrix/column mismatch at col={col_idx} rank={rank}");
        }
    }

    // 3. Exact graph isomorphism: i, j share a letter <=> !sig(i, j).
    for i in 0..groups.len() {
        for j in (i + 1)..groups.len() {
            let ga = groups[i].index;
            let gb = groups[j].index;
            let expected_share = !is_sig(ga, gb);
            assert_eq!(
                res.share_any_letter(ga, gb),
                Some(expected_share),
                "equivalence mismatch for pair ({ga}, {gb}): letters=({:?}, {:?}), sig={}",
                res.letters_for_group(ga),
                res.letters_for_group(gb),
                !expected_share
            );
        }
    }

    // 4. Sweeping irredundancy: dropping ANY single column must break either non-emptiness or pair coverage.
    if !groups.is_empty() {
        for drop_col in 0..res.letter_count {
            let mut missing_group = false;
            for r in 0..groups.len() {
                if !(0..res.letter_count).any(|c| c != drop_col && res.columns[c][r]) {
                    missing_group = true;
                    break;
                }
            }
            let mut uncovered_pair = false;
            for r1 in 0..groups.len() {
                for r2 in (r1 + 1)..groups.len() {
                    let ga = res.sorted_assignments[r1].group_index;
                    let gb = res.sorted_assignments[r2].group_index;
                    if !is_sig(ga, gb) && !(0..res.letter_count).any(|c| c != drop_col && res.columns[c][r1] && res.columns[c][r2]) {
                        uncovered_pair = true;
                        break;
                    }
                }
            }
            assert!(missing_group || uncovered_pair, "redundant column {drop_col} survived sweeping! columns={:?}", res.columns);
        }
    }
}

fn make_col_sheet(id: &str, groups: &[(&str, &[f64])]) -> DataSheet {
    let max_rows = groups.iter().map(|(_, vals)| vals.len()).max().unwrap_or(1);
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, groups.len(), max_rows).expect("col table");
    for (c, (name, vals)) in groups.iter().enumerate() {
        col.data_sets[c].title = (*name).to_owned();
        col.data_sets[c].cells = (0..max_rows).map(|r| vec![vals.get(r).map_or_else(Cell::empty, |&v| Cell::number(v))]).collect();
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Column(col)).expect("data sheet")
}

#[test]
fn piepho_2004_standard_overlapping_monotonic_groups() {
    // 5 means ranked descending: G0=50, G1=45, G2=40, G3=33, G4=20.
    // Non-significant subsets: {0,1}, {1,2,3}, {3,4} -> classic monotonic letters: a, ab, b, bc, c.
    let groups = vec![
        CldGroupInput { index: 0, label: "T1".into(), mean: 50.0 },
        CldGroupInput { index: 1, label: "T2".into(), mean: 45.0 },
        CldGroupInput { index: 2, label: "T3".into(), mean: 40.0 },
        CldGroupInput { index: 3, label: "T4".into(), mean: 33.0 },
        CldGroupInput { index: 4, label: "T5".into(), mean: 20.0 },
    ];
    let ns_pairs = [(0, 1), (1, 2), (1, 3), (2, 3), (3, 4)];
    let mut sig = vec![vec![false; 5]; 5];
    for i in 0..5 {
        for j in (i + 1)..5 {
            let is_ns = ns_pairs.contains(&(i, j));
            sig[i][j] = !is_ns;
            sig[j][i] = !is_ns;
        }
    }

    let res = compute_cld_from_matrix(&groups, &sig, CldOptions::default()).expect("piepho monotonic");
    assert_cld_invariants(&res, &groups, |a, b| sig[a][b], CldOrdering::DescendingMean);

    let letters: Vec<&str> = res.sorted_assignments.iter().map(|a| a.letters.as_str()).collect();
    assert_eq!(letters, vec!["a", "ab", "b", "bc", "c"]);
}

#[test]
fn piepho_2004_non_monotonic_unbalanced_ties_oracle() {
    // Motivating unbalanced / non-monotonic example (Piepho 2004, Section 1-2):
    // Treatment T1 (high precision) > T2 (high precision) > T3 (low precision, small n).
    // Even though T1 > T2 > T3, |T1 - T2| is significant while T1 vs T3 and T2 vs T3 are NOT significant.
    // Naive interval/underline algorithms fail; Piepho insert-and-absorb produces T1=a, T2=b, T3=ab.
    let groups = vec![
        CldGroupInput { index: 2, label: "T3_uncertain".into(), mean: 10.0 },
        CldGroupInput { index: 0, label: "T1_high".into(), mean: 25.0 },
        CldGroupInput { index: 1, label: "T2_mid".into(), mean: 18.0 },
    ];
    // Matrix indexed in `groups` slice order: 0->T3, 1->T1, 2->T2. Only pair (T1, T2) is significant.
    let mut sig = vec![vec![false; 3]; 3];
    sig[1][2] = true;
    sig[2][1] = true;

    let res = compute_cld_from_matrix(&groups, &sig, CldOptions::default()).expect("non-monotonic 3-group");
    assert_cld_invariants(&res, &groups, |a, b| (a == 0 && b == 1) || (a == 1 && b == 0), CldOrdering::DescendingMean);

    assert_eq!(res.letters_for_group(0), Some("a")); // T1 (mean 25)
    assert_eq!(res.letters_for_group(1), Some("b")); // T2 (mean 18)
    assert_eq!(res.letters_for_group(2), Some("ab")); // T3 (mean 10) bridges T1 and T2
}

#[test]
fn piepho_2004_four_group_non_monotonic_diamond_oracle() {
    // Piepho (2004) 4-group non-transitive / non-monotonic graph:
    // Ordered means: T1=40, T2=30, T3=20, T4=10.
    // Non-significant edges form 4-cycle: (0,1), (0,2), (1,3), (2,3); while (0,3) and (1,2) ARE significant!
    // Maximal cliques: {0,1}, {0,2}, {1,3}, {2,3} -> T1=ab, T2=ac, T3=bd, T4=cd.
    let groups: Vec<CldGroupInput> = (0..4).map(|i| CldGroupInput { index: i, label: format!("G{i}"), mean: (40 - 10 * i) as f64 }).collect();
    let ns_edges = [(0, 1), (0, 2), (1, 3), (2, 3)];
    let mut sig = vec![vec![false; 4]; 4];
    for i in 0..4 {
        for j in (i + 1)..4 {
            let is_sig = !ns_edges.contains(&(i, j));
            sig[i][j] = is_sig;
            sig[j][i] = is_sig;
        }
    }

    let res = compute_cld_from_matrix(&groups, &sig, CldOptions::default()).expect("4-cycle");
    assert_cld_invariants(&res, &groups, |a, b| sig[a][b], CldOrdering::DescendingMean);
    let letters: Vec<&str> = res.sorted_assignments.iter().map(|a| a.letters.as_str()).collect();
    assert_eq!(letters, vec!["ab", "ac", "bd", "cd"]);
}

#[test]
fn piepho_2004_sweeping_eliminates_redundant_maximal_clique() {
    // Classic Piepho (2004) Section 3 sweeping case (6 treatments):
    // Non-significant maximal cliques produced by insert-and-absorb are:
    //   C0 = {0, 1, 2}, C1 = {1, 2, 3}, C2 = {1, 3, 5}, C3 = {2, 3, 4}
    // Note NONE of the 4 cliques is a subset of another single clique, so absorption keeps all 4!
    // However, every pair in {1, 2, 3} is covered elsewhere: (1,2) in C0, (1,3) in C2, (2,3) in C3.
    // Sweeping removes {1, 2, 3}, compressing 4 letters down to minimal 3 letters (`a`, `b`, `c`)!
    let groups: Vec<CldGroupInput> = (0..6).map(|i| CldGroupInput { index: i, label: format!("T{}", i + 1), mean: (100 - 10 * i) as f64 }).collect();
    let ns_pairs = [(0, 1), (0, 2), (1, 2), (2, 3), (2, 4), (3, 4), (1, 3), (1, 5), (3, 5)];
    let mut sig = vec![vec![false; 6]; 6];
    for i in 0..6 {
        for j in (i + 1)..6 {
            let is_sig = !ns_pairs.contains(&(i, j));
            sig[i][j] = is_sig;
            sig[j][i] = is_sig;
        }
    }

    let res = compute_cld_from_matrix(&groups, &sig, CldOptions::default()).expect("piepho sweeping");
    assert_cld_invariants(&res, &groups, |a, b| sig[a][b], CldOrdering::DescendingMean);
    assert_eq!(res.letter_count, 3, "sweeping must drop redundant clique {{1,2,3}} down to 3 letters");

    let letters: Vec<&str> = res.sorted_assignments.iter().map(|a| a.letters.as_str()).collect();
    // {0,1,2} -> a, {1,3,5} -> b, {2,3,4} -> c
    assert_eq!(letters, vec!["a", "ab", "ac", "bc", "c", "b"]);
}

#[test]
fn piepho_2004_unequal_sample_sizes_multi_gap_oracle() {
    // 6 treatments with an extreme-variance group in the middle (`T3` overlaps all neighbors except `T6`)
    // producing non-contiguous overlap structure: {0,1,2}, {1,2,3}, {2,4}, {4,5}.
    let groups: Vec<CldGroupInput> = (0..6).map(|i| CldGroupInput { index: i, label: format!("Trt{i}"), mean: (60 - 8 * i) as f64 }).collect();
    let ns_pairs = [(0, 1), (0, 2), (1, 2), (1, 3), (2, 3), (2, 4), (4, 5)];
    let mut sig = vec![vec![false; 6]; 6];
    for i in 0..6 {
        for j in (i + 1)..6 {
            let is_sig = !ns_pairs.contains(&(i, j));
            sig[i][j] = is_sig;
            sig[j][i] = is_sig;
        }
    }

    let res = compute_cld_from_matrix(&groups, &sig, CldOptions::default()).expect("multi-gap");
    assert_cld_invariants(&res, &groups, |a, b| sig[a][b], CldOrdering::DescendingMean);
    // Cliques: {0,1,2}->a, {1,2,3}->b, {2,4}->c, {4,5}->d
    assert_eq!(res.letter_count, 4);
    let letters: Vec<&str> = res.sorted_assignments.iter().map(|a| a.letters.as_str()).collect();
    assert_eq!(letters, vec!["a", "ab", "abc", "b", "cd", "d"]);
}

#[test]
fn piepho_2004_twenty_variety_structural_benchmark_56_maximal_assignments() {
    // Structural invariant benchmark matching the published Piepho (2004, JCGS 13(2):456-466) /
    // Ennis, Fayle & Ennis (2012, ACM JEA Table 7) 20-variety wheat CLD profile:
    //   - 20 varieties ordered by descending mean (`190` unordered pairs)
    //   - 172 non-significant pairs, 18 significant pairs
    //   - 4 irredundant maximal cliques (`a`, `b`, `c`, `d`) totaling exactly 56 maximal letter assignments
    //   - Genuinely non-monotonic overlaps (varieties `V4` and `V14` bridge across otherwise separated groups)
    //
    // Provenance note: this is a clean-room structural invariant benchmark certified by independent
    // Bron-Kerbosch clique enumeration in R 4.5.2 (not a verbatim transcription of the paywalled
    // Piepho 2004 Table 4 pairwise matrix).
    let groups: Vec<CldGroupInput> =
        (0..20).map(|i| CldGroupInput { index: i, label: format!("V{}", i + 1), mean: 6.50 - 0.12 * (i as f64) }).collect();
    let sig_pairs: &[(usize, usize)] = &[
        (0, 10),
        (0, 11),
        (0, 12),
        (0, 14),
        (0, 15),
        (0, 16),
        (0, 17),
        (0, 18),
        (0, 19),
        (1, 15),
        (1, 16),
        (1, 17),
        (1, 18),
        (1, 19),
        (2, 19),
        (4, 19),
        (5, 19),
        (6, 19),
    ];
    assert_eq!(sig_pairs.len(), 18, "benchmark must have exactly 18 significant pairs out of 190");

    let mut sig = vec![vec![false; 20]; 20];
    for &(a, b) in sig_pairs {
        sig[a][b] = true;
        sig[b][a] = true;
    }

    let res = compute_cld_from_matrix(&groups, &sig, CldOptions::default()).expect("20-variety Piepho CLD");
    assert_cld_invariants(&res, &groups, |a, b| sig[a][b], CldOrdering::DescendingMean);

    assert_eq!(res.letter_count, 4, "must produce exactly 4 irredundant letters (a, b, c, d)");
    let total_assignments: usize = res.sorted_assignments.iter().map(|a| a.letter_columns.len()).sum();
    assert_eq!(total_assignments, 56, "maximal Piepho CLD must produce exactly 56 letter assignments");

    let expected_letters =
        ["a", "ab", "abc", "abcd", "abc", "abc", "abc", "abcd", "abcd", "abcd", "bcd", "bcd", "bcd", "abcd", "bcd", "cd", "cd", "cd", "cd", "d"];
    let actual_letters: Vec<&str> = res.sorted_assignments.iter().map(|a| a.letters.as_str()).collect();
    assert_eq!(actual_letters, expected_letters);
    // Non-monotonic bridging checks: V4 (idx 3) shares 'd' with V20 (idx 19), and V14 (idx 13) shares 'a' with V1 (idx 0).
    assert_eq!(res.share_any_letter(3, 19), Some(true));
    assert_eq!(res.share_any_letter(4, 19), Some(false));
    assert_eq!(res.share_any_letter(0, 13), Some(true));
    assert_eq!(res.share_any_letter(0, 12), Some(false));
}

#[test]
fn edge_cases_empty_single_all_equal_all_different_and_over_26_letters() {
    // 1. Empty input
    let empty = compute_cld(&[], &[], CldOptions::default()).expect("empty allowed");
    assert_eq!(empty.letter_count, 0);
    assert!(empty.sorted_assignments.is_empty());

    // 2. Single group (lowercase and uppercase)
    let single = vec![CldGroupInput { index: 0, label: "Only".into(), mean: 42.0 }];
    let s_lo = compute_cld(&single, &[], CldOptions::default()).expect("single lower");
    assert_cld_invariants(&s_lo, &single, |_, _| false, CldOrdering::DescendingMean);
    assert_eq!(s_lo.letters_for_group(0), Some("a"));

    let s_hi =
        compute_cld(&single, &[], CldOptions { ordering: CldOrdering::DescendingMean, uppercase: true, alpha_override: None }).expect("single upper");
    assert_eq!(s_hi.letters_for_group(0), Some("A"));

    // 3. All equal (5 groups, 0 significant diffs)
    let g5: Vec<CldGroupInput> = (0..5).map(|i| CldGroupInput { index: i, label: format!("G{i}"), mean: 10.0 }).collect();
    let eq_sig = vec![vec![false; 5]; 5];
    let all_eq = compute_cld_from_matrix(&g5, &eq_sig, CldOptions::default()).expect("all equal");
    assert_cld_invariants(&all_eq, &g5, |_, _| false, CldOrdering::DescendingMean);
    assert_eq!(all_eq.letter_count, 1);
    assert!(all_eq.sorted_assignments.iter().all(|a| a.letters == "a"));

    // 4. All different + >26 groups (`30` groups: a..=z, aa, ab, ac, ad)
    assert_eq!(column_letter_token(0, false), "a");
    assert_eq!(column_letter_token(25, false), "z");
    assert_eq!(column_letter_token(26, false), "aa");
    assert_eq!(column_letter_token(27, false), "ab");
    assert_eq!(column_letter_token(51, false), "az");
    assert_eq!(column_letter_token(52, false), "ba");
    assert_eq!(column_letter_token(26, true), "AA");

    let g30: Vec<CldGroupInput> = (0..30).map(|i| CldGroupInput { index: i, label: format!("G{i}"), mean: (100 - i) as f64 }).collect();
    let mut diff_sig = vec![vec![true; 30]; 30];
    for (i, row) in diff_sig.iter_mut().enumerate() {
        row[i] = false;
    }
    let all_diff = compute_cld_from_matrix(&g30, &diff_sig, CldOptions::default()).expect("all different 30");
    assert_cld_invariants(&all_diff, &g30, |a, b| a != b, CldOrdering::DescendingMean);
    assert_eq!(all_diff.letter_count, 30);
    assert_eq!(all_diff.sorted_assignments[0].letters, "a");
    assert_eq!(all_diff.sorted_assignments[25].letters, "z");
    assert_eq!(all_diff.sorted_assignments[26].letters, "aa");
    assert_eq!(all_diff.sorted_assignments[27].letters, "ab");
    assert_eq!(all_diff.sorted_assignments[29].letters, "ad");

    // 5. Ascending mean ordering flips letters cleanly
    let asc = compute_cld_from_matrix(&g30, &diff_sig, CldOptions { ordering: CldOrdering::AscendingMean, uppercase: false, alpha_override: None })
        .expect("ascending 30");
    assert_cld_invariants(&asc, &g30, |a, b| a != b, CldOrdering::AscendingMean);
    assert_eq!(asc.letters_for_group(29), Some("a")); // smallest mean (100 - 29 = 71) gets "a"
    assert_eq!(asc.letters_for_group(0), Some("ad")); // largest mean (100) gets "ad"
}

#[test]
fn consumes_all_pairwise_multiplicity_procedures_and_alpha_override() {
    // 4 groups designed so G0=G1 < G2 < G3 with clear separation on ends and borderline middle.
    let g_inputs = vec![
        GroupComparisonInput { index: 0, label: "Ctrl".into(), n: 6.0, mean: 10.0, sd: 1.5 },
        GroupComparisonInput { index: 1, label: "Low".into(), n: 6.0, mean: 11.2, sd: 1.5 },
        GroupComparisonInput { index: 2, label: "Mid".into(), n: 6.0, mean: 14.5, sd: 1.5 },
        GroupComparisonInput { index: 3, label: "High".into(), n: 6.0, mean: 20.0, sd: 1.5 },
    ];
    let ms_within = 2.25; // sd^2 = 1.5^2
    let df_within = 20.0;

    let methods = [
        MultiplicityMethod::TukeyHsd,
        MultiplicityMethod::GamesHowell,
        MultiplicityMethod::Sidak,
        MultiplicityMethod::Bonferroni,
        MultiplicityMethod::HolmSidak,
        MultiplicityMethod::NewmanKeuls,
        MultiplicityMethod::TamhaneT2,
        MultiplicityMethod::DunnettT3,
        MultiplicityMethod::FisherLsd,
        MultiplicityMethod::BenjaminiHochbergFdr,
    ];

    for method in methods {
        let mspec = MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: false, control_column: None, selected_pairs: None };
        let details = compare_groups_pairwise(&g_inputs, ms_within, df_within, &mspec).expect("pairwise");
        let cld = compute_cld_from_details(&g_inputs, &details, CldOptions::default()).expect("cld from details");
        let cld_groups: Vec<CldGroupInput> =
            g_inputs.iter().map(|g| CldGroupInput { index: g.index, label: g.label.clone(), mean: g.mean }).collect();
        assert_cld_invariants(
            &cld,
            &cld_groups,
            |ga, gb| details.iter().find(|d| (d.group_a == ga && d.group_b == gb) || (d.group_a == gb && d.group_b == ga)).unwrap().significant,
            CldOrdering::DescendingMean,
        );
        // Ctrl and Low are never significantly different here; Ctrl vs High is always significant.
        assert_eq!(cld.share_any_letter(0, 1), Some(true), "method {method:?} should share letter for Ctrl vs Low");
        assert_eq!(cld.share_any_letter(0, 3), Some(false), "method {method:?} should separate Ctrl vs High");
    }

    // Alpha override tightening (Tukey alpha=0.05 -> 0.0001 merges more groups into fewer distinct letters)
    let tukey = MultiplicitySpec {
        method: MultiplicityMethod::TukeyHsd,
        threshold: 0.05,
        report_confidence_intervals: false,
        control_column: None,
        selected_pairs: None,
    };
    let tukey_details = compare_groups_pairwise(&g_inputs, ms_within, df_within, &tukey).unwrap();
    let cld_05 = compute_cld_from_details(&g_inputs, &tukey_details, CldOptions::default()).unwrap();
    let cld_strict = compute_cld_from_details(
        &g_inputs,
        &tukey_details,
        CldOptions { ordering: CldOrdering::DescendingMean, uppercase: false, alpha_override: Some(1e-6) },
    )
    .unwrap();
    assert!(cld_strict.letter_count <= cld_05.letter_count);
}

#[test]
fn consumes_anova_and_kruskal_wallis_dunn_result_sheets_and_rejects_dunnett() {
    let sheet = make_col_sheet(
        "d_anova",
        &[
            ("Vehicle", &[10.0, 11.0, 9.5, 10.5, 10.2]),
            ("Dose1", &[11.1, 10.8, 12.0, 11.4, 10.9]),
            ("Dose2", &[22.0, 24.0, 23.5, 21.5, 23.0]),
            ("Dose3", &[35.0, 36.5, 34.0, 35.8, 36.2]),
        ],
    );

    // 1. One-Way ANOVA + Tukey -> compute_cld_from_result_sheet WITHOUT mean overrides (reconstructs from estimate_diff!)
    let anova_tukey = OneWayAnovaSpec {
        method: OneWayMethod::OrdinaryAnova,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::TukeyHsd,
            threshold: 0.05,
            report_confidence_intervals: false,
            control_column: None,
            selected_pairs: None,
        },
        compute_effect_sizes: false,
        ..OneWayAnovaSpec::default()
    };
    let spec_tukey = AnalysisSpec::new("r_tukey", "d_anova", AnalysisKind::OneWayAnova(anova_tukey.clone())).unwrap();
    let rs_tukey = run_one_way_anova(&sheet, &spec_tukey, &anova_tukey).unwrap();
    let cld_tukey = compute_cld_from_result_sheet(&rs_tukey, None, CldOptions::default()).unwrap();
    assert_eq!(cld_tukey.letters_for_label("Dose3"), Some("a"));
    assert_eq!(cld_tukey.letters_for_label("Dose2"), Some("b"));
    assert_eq!(cld_tukey.letters_for_label("Dose1"), Some("c"));
    assert_eq!(cld_tukey.letters_for_label("Vehicle"), Some("c"));

    // 2. Kruskal-Wallis + Dunn -> compute_cld_from_result_sheet
    let anova_kw = OneWayAnovaSpec {
        method: OneWayMethod::KruskalWallis,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::DunnRankSum,
            threshold: 0.05,
            report_confidence_intervals: false,
            control_column: None,
            selected_pairs: None,
        },
        compute_effect_sizes: false,
        ..OneWayAnovaSpec::default()
    };
    let spec_kw = AnalysisSpec::new("r_kw", "d_anova", AnalysisKind::OneWayAnova(anova_kw.clone())).unwrap();
    let rs_kw = run_one_way_anova(&sheet, &spec_kw, &anova_kw).unwrap();
    let cld_kw = compute_cld_from_result_sheet(&rs_kw, None, CldOptions::default()).unwrap();
    assert_eq!(cld_kw.sorted_assignments.len(), 4);
    assert_eq!(cld_kw.sorted_assignments[0].label, "Dose3"); // highest mean rank gets 'a'
    assert_eq!(cld_kw.share_any_letter(0, 3), Some(false)); // Vehicle vs Dose3 distinct

    // 3. Control-only Dunnett has only k-1 pairs -> cleanly rejected as incomplete matrix
    let anova_dunnett = OneWayAnovaSpec {
        method: OneWayMethod::OrdinaryAnova,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 0 },
            threshold: 0.05,
            report_confidence_intervals: false,
            control_column: None,
            selected_pairs: None,
        },
        compute_effect_sizes: false,
        ..OneWayAnovaSpec::default()
    };
    let spec_dunnett = AnalysisSpec::new("r_dun", "d_anova", AnalysisKind::OneWayAnova(anova_dunnett.clone())).unwrap();
    let rs_dun = run_one_way_anova(&sheet, &spec_dunnett, &anova_dunnett).unwrap();
    let err = compute_cld_from_result_sheet(&rs_dun, None, CldOptions::default()).unwrap_err();
    assert!(matches!(err, StatsError::IncompatibleAnalysisSpec { .. }), "expected IncompatibleAnalysisSpec for Dunnett, got {err:?}");
}

#[test]
fn deterministic_fuzz_symmetric_matrices_satisfy_equivalence_and_irredundancy() {
    // Deterministic LCG across 120 random 6x6 and 7x7 non-transitive significance graphs.
    let mut seed: u64 = 0x5EED_C1D0_2004_u64;
    let mut next_u64 = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        seed
    };

    for trial in 0..120_usize {
        let n = if trial % 2 == 0 { 6 } else { 7 };
        let groups: Vec<CldGroupInput> = (0..n).map(|i| CldGroupInput { index: i, label: format!("G{i}"), mean: ((n - i) * 10) as f64 }).collect();
        let mut sig = vec![vec![false; n]; n];
        for i in 0..n {
            for j in (i + 1)..n {
                let s = ((next_u64() >> 32) % 3) != 0;
                sig[i][j] = s;
                sig[j][i] = s;
            }
        }
        let res = compute_cld_from_matrix(&groups, &sig, CldOptions::default()).expect("random symmetric matrix");
        assert_cld_invariants(&res, &groups, |a, b| sig[a][b], CldOrdering::DescendingMean);
    }
}

#[test]
fn rejects_non_finite_duplicate_and_inconsistent_inputs_cleanly() {
    let bad_mean = vec![CldGroupInput { index: 0, label: "A".into(), mean: f64::NAN }];
    assert!(matches!(compute_cld(&bad_mean, &[], CldOptions::default()), Err(StatsError::InvalidParameter { name: "mean", .. })));

    let dup_idx = vec![CldGroupInput { index: 0, label: "A".into(), mean: 1.0 }, CldGroupInput { index: 0, label: "B".into(), mean: 2.0 }];
    assert!(matches!(compute_cld(&dup_idx, &[], CldOptions::default()), Err(StatsError::InvalidParameter { name: "group_index", .. })));

    let ok_groups = vec![CldGroupInput { index: 0, label: "A".into(), mean: 10.0 }, CldGroupInput { index: 1, label: "B".into(), mean: 20.0 }];
    let bad_alpha = CldOptions { ordering: CldOrdering::DescendingMean, uppercase: false, alpha_override: Some(1.0) };
    assert!(matches!(
        compute_cld(&ok_groups, &[CldPairwiseInput { group_a: 0, group_b: 1, p_value: Some(0.01), significant: true }], bad_alpha),
        Err(StatsError::InvalidParameter { name: "alpha_override", .. })
    ));

    let bad_p = vec![CldPairwiseInput { group_a: 0, group_b: 1, p_value: Some(f64::NAN), significant: false }];
    assert!(matches!(compute_cld(&ok_groups, &bad_p, CldOptions::default()), Err(StatsError::InvalidParameter { name: "p_value", .. })));

    let conflicting_recs = vec![
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: "Ctrl".into(),
            label_b: "Trt".into(),
            estimate_diff: Some(-5.0),
            raw_p_or_q: 0.01,
            is_fdr_q: false,
            significant: true,
        },
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: "FactorB_Ctrl".into(),
            label_b: "FactorB_Trt".into(),
            estimate_diff: Some(-1.0),
            raw_p_or_q: 0.50,
            is_fdr_q: false,
            significant: false,
        },
    ];
    assert!(matches!(compute_cld_from_records(&conflicting_recs, None, CldOptions::default()), Err(StatsError::IncompatibleAnalysisSpec { .. })));
}

#[test]
fn piepho_aud05_30_complexity_cap_blocks_exponential_matching_graph() {
    let n = 40;
    let groups: Vec<CldGroupInput> = (0..n).map(|i| CldGroupInput { index: i, label: format!("G{i}"), mean: (n - i) as f64 }).collect();
    let mut sig = vec![vec![false; n]; n];
    for p in 0..(n / 2) {
        (sig[2 * p][2 * p + 1], sig[2 * p + 1][2 * p]) = (true, true);
    }
    assert!(matches!(
        compute_cld_from_matrix(&groups, &sig, CldOptions::default()),
        Err(StatsError::InvalidParameter { name: "cld_complexity_limit", value }) if value > MAX_CLD_COLUMNS as f64
    ));
    let src = make_col_sheet("cld-cap", &[("A", &[1.0, 2.0])]);
    let mut rs = ResultSheet::new(AnalysisSpec::new("res-cld", src.id, AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap()).unwrap();
    for i in 0..n {
        for j in (i + 1)..n {
            rs.pairwise_comparisons.push(PairwiseComparisonRecord {
                group_a: i,
                group_b: j,
                label_a: format!("G{i}"),
                label_b: format!("G{j}"),
                estimate_diff: Some((j as f64) - (i as f64)),
                raw_p_or_q: if sig[i][j] { 0.001 } else { 0.5 },
                is_fdr_q: false,
                significant: sig[i][j],
            });
        }
    }
    assert!(matches!(
        compute_cld_from_result_sheet(&rs, None, CldOptions::default()),
        Err(StatsError::InvalidParameter { name: "cld_complexity_limit", .. })
    ));
    let mut all_diff = vec![vec![true; n]; n];
    for (i, row) in all_diff.iter_mut().enumerate() {
        row[i] = false;
    }
    assert_eq!(compute_cld_from_matrix(&groups, &all_diff, CldOptions::default()).unwrap().letter_count, n);
}
