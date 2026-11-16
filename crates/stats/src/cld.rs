//! Clean-room Compact Letter Display (CLD) engine using the Piepho (2004)
//! insert-and-absorb plus sweeping algorithm.
//!
//! # Reference
//! - Piepho, H.-P. (2004). An algorithm for a letter-based representation of
//!   all-pairwise comparisons. *Journal of Computational and Graphical Statistics*,
//!   13(2), 456–466. <https://doi.org/10.1198/1061860043515>

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};

use lumencraft_doc::{AnalysisKind, PairwiseComparisonRecord, ResultSheet};

pub use lumencraft_doc::CldOrdering;

use crate::StatsError;
use crate::multiplicity::{GroupComparisonInput, PairwiseComparisonDetail};

/// Single group input for Piepho (2004) CLD generation.
#[derive(Debug, Clone, PartialEq)]
pub struct CldGroupInput {
    /// Zero-based group/column identifier matching [`PairwiseComparisonRecord::group_a`] / `group_b`.
    pub index: usize,
    /// Group display label.
    pub label: String,
    /// Summary location measure (mean, median, or mean rank) used for monotonic ordering.
    pub mean: f64,
}

/// Pairwise significance input between two distinct groups.
#[derive(Debug, Clone, PartialEq)]
pub struct CldPairwiseInput {
    pub group_a: usize,
    pub group_b: usize,
    /// Multiplicity-adjusted p-value or FDR q-value, when available.
    pub p_value: Option<f64>,
    /// Precomputed rejection flag under the analysis procedure's significance rule.
    pub significant: bool,
}

/// Options governing Piepho (2004) ordering, case, and significance thresholding.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CldOptions {
    pub ordering: CldOrdering,
    pub uppercase: bool,
    /// Optional explicit threshold (`p <= alpha`) overriding [`CldPairwiseInput::significant`].
    pub alpha_override: Option<f64>,
}

/// Per-group CLD assignment emitted in sorted monotonic display order.
#[derive(Debug, Clone, PartialEq)]
pub struct CldGroupAssignment {
    pub group_index: usize,
    pub label: String,
    pub mean: f64,
    /// Zero-based rank after applying [`CldOptions::ordering`].
    pub rank: usize,
    /// Zero-based column indices assigned to this group (in display order `0..letter_count`).
    pub letter_columns: Vec<usize>,
    /// Compact display text (`"a"`, `"ab"`, `"aa"`, ...).
    pub letters: String,
}

/// Complete Piepho (2004) Compact Letter Display solution.
#[derive(Debug, Clone, PartialEq)]
pub struct CldResult {
    /// Assignments sorted monotonically according to [`CldOptions::ordering`].
    pub sorted_assignments: Vec<CldGroupAssignment>,
    /// Final non-redundant boolean columns (`letter_count x n`), indexed `[col][rank]`.
    pub columns: Vec<Vec<bool>>,
    /// Number of distinct letter columns retained after absorption and sweeping.
    pub letter_count: usize,
}

impl CldResult {
    /// Empty CLD result for zero-group input.
    #[must_use]
    pub fn empty() -> Self {
        Self { sorted_assignments: Vec::new(), columns: Vec::new(), letter_count: 0 }
    }

    /// Look up the assignment for a group by original `group_index`.
    #[must_use]
    pub fn assignment_for_group(&self, group_index: usize) -> Option<&CldGroupAssignment> {
        self.sorted_assignments.iter().find(|a| a.group_index == group_index)
    }

    /// Look up assigned CLD text for a group by original `group_index`.
    #[must_use]
    pub fn letters_for_group(&self, group_index: usize) -> Option<&str> {
        self.assignment_for_group(group_index).map(|a| a.letters.as_str())
    }

    /// Look up assigned CLD text for a group by trimmed label match.
    #[must_use]
    pub fn letters_for_label(&self, label: &str) -> Option<&str> {
        let needle = label.trim();
        self.sorted_assignments.iter().find(|a| a.label.trim() == needle).map(|a| a.letters.as_str())
    }

    /// Return whether two groups share at least one letter column (`true` means not significantly different).
    #[must_use]
    pub fn share_any_letter(&self, group_a: usize, group_b: usize) -> Option<bool> {
        let a = self.assignment_for_group(group_a)?;
        let b = self.assignment_for_group(group_b)?;
        if group_a == group_b {
            return Some(!a.letters.is_empty());
        }
        Some(a.letter_columns.iter().any(|col| b.letter_columns.contains(col)))
    }
}

#[derive(Clone, PartialEq, Eq)]
struct BitCol {
    words: Vec<u64>,
    len: usize,
}

impl BitCol {
    fn all_ones(len: usize) -> Self {
        let word_count = len.div_ceil(64);
        let mut words = vec![u64::MAX; word_count];
        let rem = len % 64;
        if rem != 0
            && let Some(last) = words.last_mut()
        {
            *last = (1_u64 << rem) - 1;
        }
        Self { words, len }
    }

    fn get(&self, row: usize) -> bool {
        if row >= self.len {
            return false;
        }
        let word = self.words.get(row / 64).copied().unwrap_or(0);
        ((word >> (row % 64)) & 1) != 0
    }

    fn clear(&mut self, row: usize) {
        if row < self.len
            && let Some(word) = self.words.get_mut(row / 64)
        {
            *word &= !(1_u64 << (row % 64));
        }
    }

    fn without(&self, row: usize) -> Self {
        let mut cloned = self.clone();
        cloned.clear(row);
        cloned
    }

    fn is_subset_of(&self, other: &Self) -> bool {
        self.words.iter().zip(other.words.iter()).all(|(a, b)| (a & b) == *a)
    }

    fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    fn cmp_piepho(&self, other: &Self) -> Ordering {
        for r in 0..self.len {
            match (self.get(r), other.get(r)) {
                (true, false) => return Ordering::Less,
                (false, true) => return Ordering::Greater,
                _ => {}
            }
        }
        Ordering::Equal
    }

    fn member_rows(&self) -> Vec<usize> {
        let mut rows = Vec::new();
        for r in 0..self.len {
            if self.get(r) {
                rows.push(r);
            }
        }
        rows
    }

    fn to_bool_vec(&self) -> Vec<bool> {
        (0..self.len).map(|r| self.get(r)).collect()
    }
}

fn absorb_insert(cols: &mut Vec<BitCol>, candidate: BitCol) {
    if candidate.is_empty() {
        return;
    }
    if cols.iter().any(|existing| candidate.is_subset_of(existing)) {
        return;
    }
    cols.retain(|existing| !existing.is_subset_of(&candidate));
    cols.push(candidate);
}

fn insert_and_absorb(n: usize, sig_matrix: &[bool]) -> Vec<BitCol> {
    let mut cols = vec![BitCol::all_ones(n)];
    for i in 0..n {
        for j in (i + 1)..n {
            if !sig_matrix.get(i * n + j).copied().unwrap_or(false) {
                continue;
            }
            let mut next_cols = Vec::with_capacity(cols.len() + 1);
            let mut conflicts = Vec::new();
            for col in cols {
                if col.get(i) && col.get(j) {
                    conflicts.push(col);
                } else {
                    next_cols.push(col);
                }
            }
            if conflicts.is_empty() {
                cols = next_cols;
                continue;
            }
            for col in conflicts {
                absorb_insert(&mut next_cols, col.without(i));
                absorb_insert(&mut next_cols, col.without(j));
            }
            cols = next_cols;
        }
    }
    cols.sort_by(BitCol::cmp_piepho);
    cols
}

fn is_column_redundant(cols: &[BitCol], active: &[bool], candidate_idx: usize) -> bool {
    let Some(col) = cols.get(candidate_idx) else {
        return false;
    };
    let members = col.member_rows();
    if members.len() <= 1 {
        return members
            .iter()
            .all(|&r| cols.iter().enumerate().any(|(k, other)| k != candidate_idx && active.get(k).copied().unwrap_or(false) && other.get(r)));
    }
    for (idx_a, &r1) in members.iter().enumerate() {
        let row_covered = cols.iter().enumerate().any(|(k, other)| k != candidate_idx && active.get(k).copied().unwrap_or(false) && other.get(r1));
        if !row_covered {
            return false;
        }
        for &r2 in members.iter().skip(idx_a + 1) {
            let pair_covered = cols
                .iter()
                .enumerate()
                .any(|(k, other)| k != candidate_idx && active.get(k).copied().unwrap_or(false) && other.get(r1) && other.get(r2));
            if !pair_covered {
                return false;
            }
        }
    }
    true
}

fn sweep_redundant_columns(cols: Vec<BitCol>) -> Vec<BitCol> {
    let mut active = vec![true; cols.len()];
    // Scan in reverse canonical Piepho order so upper monotonic blocks stay intact on ties.
    for idx in (0..cols.len()).rev() {
        if is_column_redundant(&cols, &active, idx)
            && let Some(slot) = active.get_mut(idx)
        {
            *slot = false;
        }
    }
    let mut kept: Vec<BitCol> = cols.into_iter().zip(active).filter_map(|(c, keep)| keep.then_some(c)).collect();
    kept.sort_by(BitCol::cmp_piepho);
    kept
}

/// Format zero-based column index as bijective base-26 token (`0 -> "a"`, `25 -> "z"`, `26 -> "aa"`, `27 -> "ab"`).
#[must_use]
pub fn column_letter_token(mut index: usize, uppercase: bool) -> String {
    let base = if uppercase { b'A' } else { b'a' };
    let mut rev_digits = Vec::new();
    loop {
        let rem = (index % 26) as u8;
        rev_digits.push(base + rem);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    rev_digits.reverse();
    String::from_utf8(rev_digits).unwrap_or_default()
}

fn format_group_letters(letter_cols: &[usize], total_cols: usize, uppercase: bool) -> String {
    let tokens: Vec<String> = letter_cols.iter().map(|&c| column_letter_token(c, uppercase)).collect();
    if total_cols > 26 && tokens.len() > 1 { tokens.join(" ") } else { tokens.concat() }
}

fn validate_options(options: CldOptions) -> Result<(), StatsError> {
    if let Some(alpha) = options.alpha_override
        && (!alpha.is_finite() || alpha <= 0.0 || alpha >= 1.0)
    {
        return Err(StatsError::InvalidParameter { name: "alpha_override", value: alpha });
    }
    Ok(())
}

fn validate_and_sort_groups(groups: &[CldGroupInput], ordering: CldOrdering) -> Result<Vec<(usize, &CldGroupInput)>, StatsError> {
    let mut seen_indices = BTreeMap::new();
    for g in groups {
        if !g.mean.is_finite() {
            return Err(StatsError::InvalidParameter { name: "mean", value: g.mean });
        }
        if seen_indices.insert(g.index, ()).is_some() {
            return Err(StatsError::InvalidParameter { name: "group_index", value: g.index as f64 });
        }
    }
    let mut indexed: Vec<(usize, &CldGroupInput)> = groups.iter().enumerate().collect();
    indexed.sort_by(|(_, a), (_, b)| {
        let mean_cmp = match ordering {
            CldOrdering::DescendingMean => b.mean.total_cmp(&a.mean),
            CldOrdering::AscendingMean => a.mean.total_cmp(&b.mean),
        };
        mean_cmp.then_with(|| a.index.cmp(&b.index))
    });
    Ok(indexed)
}

fn build_significance_matrix(
    sorted_groups: &[(usize, &CldGroupInput)],
    pairs: &[CldPairwiseInput],
    alpha_override: Option<f64>,
) -> Result<Vec<bool>, StatsError> {
    let n = sorted_groups.len();
    let mut pos_by_group = BTreeMap::new();
    for (rank, &(_, g)) in sorted_groups.iter().enumerate() {
        pos_by_group.insert(g.index, rank);
    }

    let mut sig_matrix = vec![false; n * n];
    let mut seen_matrix = vec![false; n * n];
    for p in pairs {
        let Some(&ra) = pos_by_group.get(&p.group_a) else {
            return Err(StatsError::MissingColumn { index: p.group_a });
        };
        let Some(&rb) = pos_by_group.get(&p.group_b) else {
            return Err(StatsError::MissingColumn { index: p.group_b });
        };
        if let Some(pv) = p.p_value
            && (!pv.is_finite() || !(0.0..=1.0).contains(&pv))
        {
            return Err(StatsError::InvalidParameter { name: "p_value", value: pv });
        }
        if ra == rb {
            continue;
        }
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        let is_sig = match (alpha_override, p.p_value) {
            (Some(alpha), Some(pv)) => pv <= alpha,
            _ => p.significant,
        };
        let flat = lo * n + hi;
        let reverse_flat = hi * n + lo;
        if seen_matrix.get(flat).copied().unwrap_or(false) && sig_matrix.get(flat).copied().unwrap_or(false) != is_sig {
            return Err(StatsError::InvalidParameter { name: "significant", value: 0.0 });
        }
        if let Some(cell) = seen_matrix.get_mut(flat) {
            *cell = true;
        }
        if let Some(cell) = seen_matrix.get_mut(reverse_flat) {
            *cell = true;
        }
        if let Some(cell) = sig_matrix.get_mut(flat) {
            *cell = is_sig;
        }
        if let Some(cell) = sig_matrix.get_mut(reverse_flat) {
            *cell = is_sig;
        }
    }

    for i in 0..n {
        for j in (i + 1)..n {
            if !seen_matrix.get(i * n + j).copied().unwrap_or(false) {
                return Err(StatsError::IncompatibleAnalysisSpec {
                    expected: "complete all-pairwise comparisons (Tukey, Games-Howell, Dunn, Sidak, Holm-Sidak, or Bonferroni)",
                });
            }
        }
    }
    Ok(sig_matrix)
}

/// Compute Piepho (2004) Compact Letter Display from explicit groups and pairwise decisions.
///
/// # Errors
/// Returns [`StatsError`] on invalid `alpha_override`, non-finite group means or p-values, duplicate
/// group IDs, contradictory pair entries, or incomplete pair graphs (such as control-only Dunnett).
pub fn compute_cld(groups: &[CldGroupInput], pairs: &[CldPairwiseInput], options: CldOptions) -> Result<CldResult, StatsError> {
    validate_options(options)?;
    if groups.is_empty() {
        return if pairs.is_empty() { Ok(CldResult::empty()) } else { Err(StatsError::EmptyInput) };
    }
    let sorted = validate_and_sort_groups(groups, options.ordering)?;
    let n = sorted.len();

    if n == 1 {
        for p in pairs {
            if let Some(pv) = p.p_value
                && (!pv.is_finite() || !(0.0..=1.0).contains(&pv))
            {
                return Err(StatsError::InvalidParameter { name: "p_value", value: pv });
            }
        }
        if let Some(&(_, g0)) = sorted.first() {
            let letter = column_letter_token(0, options.uppercase);
            return Ok(CldResult {
                sorted_assignments: vec![CldGroupAssignment {
                    group_index: g0.index,
                    label: g0.label.clone(),
                    mean: g0.mean,
                    rank: 0,
                    letter_columns: vec![0],
                    letters: letter,
                }],
                columns: vec![vec![true]],
                letter_count: 1,
            });
        }
    }

    let sig_matrix = build_significance_matrix(&sorted, pairs, options.alpha_override)?;
    let absorbed = insert_and_absorb(n, &sig_matrix);
    let swept = sweep_redundant_columns(absorbed);

    let total_cols = swept.len();
    let columns: Vec<Vec<bool>> = swept.iter().map(BitCol::to_bool_vec).collect();
    let mut assignments = Vec::with_capacity(n);
    for (rank, &(_, group)) in sorted.iter().enumerate() {
        let mut letter_columns = Vec::new();
        for (col_idx, col) in swept.iter().enumerate() {
            if col.get(rank) {
                letter_columns.push(col_idx);
            }
        }
        let letters = format_group_letters(&letter_columns, total_cols, options.uppercase);
        assignments.push(CldGroupAssignment {
            group_index: group.index,
            label: group.label.clone(),
            mean: group.mean,
            rank,
            letter_columns,
            letters,
        });
    }

    Ok(CldResult { sorted_assignments: assignments, columns, letter_count: total_cols })
}

/// Compute Piepho (2004) CLD directly from an `n x n` symmetric boolean significance matrix
/// indexed in `groups` slice order.
///
/// # Errors
/// Returns [`StatsError`] if matrix dimensions do not match `groups.len()`, diagonal entries are
/// `true`, the matrix is asymmetric, or group validation fails.
pub fn compute_cld_from_matrix(groups: &[CldGroupInput], significant_matrix: &[Vec<bool>], options: CldOptions) -> Result<CldResult, StatsError> {
    let n = groups.len();
    if significant_matrix.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: significant_matrix.len() });
    }
    for row in significant_matrix {
        if row.len() != n {
            return Err(StatsError::DimensionMismatch { expected: n, actual: row.len() });
        }
    }
    let mut pairs = Vec::with_capacity(n.saturating_mul(n.saturating_sub(1)) / 2);
    for (i, gi) in groups.iter().enumerate() {
        if significant_matrix.get(i).and_then(|r| r.get(i)).copied().unwrap_or(false) {
            return Err(StatsError::InvalidParameter { name: "significant_diagonal", value: i as f64 });
        }
        for (j, gj) in groups.iter().enumerate().skip(i + 1) {
            let sij = significant_matrix.get(i).and_then(|r| r.get(j)).copied().unwrap_or(false);
            let sji = significant_matrix.get(j).and_then(|r| r.get(i)).copied().unwrap_or(false);
            if sij != sji {
                return Err(StatsError::InvalidParameter { name: "significant_asymmetry", value: 1.0 });
            }
            pairs.push(CldPairwiseInput { group_a: gi.index, group_b: gj.index, p_value: None, significant: sij });
        }
    }
    compute_cld(groups, &pairs, options)
}

/// Compute Piepho (2004) CLD from [`multiplicity`](crate::multiplicity) group inputs and pairwise details.
///
/// # Errors
/// Returns [`StatsError`] on invalid options or incomplete (e.g. Dunnett) comparison sets.
pub fn compute_cld_from_details(
    groups: &[GroupComparisonInput],
    details: &[PairwiseComparisonDetail],
    options: CldOptions,
) -> Result<CldResult, StatsError> {
    let cld_groups: Vec<CldGroupInput> = groups.iter().map(|g| CldGroupInput { index: g.index, label: g.label.clone(), mean: g.mean }).collect();
    let cld_pairs: Vec<CldPairwiseInput> = details
        .iter()
        .map(|d| CldPairwiseInput { group_a: d.group_a, group_b: d.group_b, p_value: Some(d.adjusted_p_or_q), significant: d.significant })
        .collect();
    compute_cld(&cld_groups, &cld_pairs, options)
}

fn solve_relative_means_bfs(group_indices: &[usize], records: &[PairwiseComparisonRecord]) -> BTreeMap<usize, f64> {
    let mut adj: BTreeMap<usize, Vec<(usize, f64)>> = BTreeMap::new();
    for rec in records {
        if let Some(diff) = rec.estimate_diff
            && diff.is_finite()
        {
            // rec.estimate_diff == mean_a - mean_b
            adj.entry(rec.group_a).or_default().push((rec.group_b, -diff));
            adj.entry(rec.group_b).or_default().push((rec.group_a, diff));
        }
    }

    let mut resolved = BTreeMap::new();
    for &start in group_indices {
        if resolved.contains_key(&start) {
            continue;
        }
        resolved.insert(start, 0.0_f64);
        let mut queue = VecDeque::new();
        queue.push_back(start);
        while let Some(cur) = queue.pop_front() {
            let cur_val = resolved.get(&cur).copied().unwrap_or(0.0);
            if let Some(edges) = adj.get(&cur) {
                for &(nxt, delta) in edges {
                    if let std::collections::btree_map::Entry::Vacant(entry) = resolved.entry(nxt) {
                        entry.insert(cur_val + delta);
                        queue.push_back(nxt);
                    }
                }
            }
        }
    }
    resolved
}

fn compute_cld_from_records_inner(
    records: &[PairwiseComparisonRecord],
    group_means_override: Option<&[(usize, f64)]>,
    allow_additive_diff_inference: bool,
    options: CldOptions,
) -> Result<CldResult, StatsError> {
    if records.is_empty() {
        return Err(StatsError::IncompatibleAnalysisSpec { expected: "all-pairwise comparison records" });
    }

    let mut labels_by_idx: BTreeMap<usize, String> = BTreeMap::new();
    for rec in records {
        for (idx, lbl) in [(rec.group_a, rec.label_a.as_str()), (rec.group_b, rec.label_b.as_str())] {
            match labels_by_idx.get(&idx) {
                Some(existing) if existing != lbl => {
                    return Err(StatsError::IncompatibleAnalysisSpec {
                        expected: "single all-pairwise comparison factor without conflicting group IDs",
                    });
                }
                None => {
                    labels_by_idx.insert(idx, lbl.to_owned());
                }
                _ => {}
            }
        }
    }

    let override_map: BTreeMap<usize, f64> = group_means_override.unwrap_or(&[]).iter().copied().collect();
    let group_indices: Vec<usize> = labels_by_idx.keys().copied().collect();
    let inferred = if allow_additive_diff_inference { solve_relative_means_bfs(&group_indices, records) } else { BTreeMap::new() };

    let groups: Vec<CldGroupInput> = labels_by_idx
        .into_iter()
        .map(|(index, label)| {
            let mean = override_map.get(&index).copied().or_else(|| inferred.get(&index).copied()).unwrap_or(-(index as f64));
            CldGroupInput { index, label, mean }
        })
        .collect();

    let pairs: Vec<CldPairwiseInput> = records
        .iter()
        .map(|r| CldPairwiseInput { group_a: r.group_a, group_b: r.group_b, p_value: Some(r.raw_p_or_q), significant: r.significant })
        .collect();

    compute_cld(&groups, &pairs, options)
}

/// Compute Piepho (2004) CLD from [`PairwiseComparisonRecord`] slices, using `group_means_override`
/// when available and inferring relative ordering from additive `estimate_diff` otherwise.
///
/// # Errors
/// Returns [`StatsError`] on empty records, missing pairs (e.g. Dunnett), conflicting group labels
/// (e.g. mixed multi-factor scopes), or invalid CLD parameters.
pub fn compute_cld_from_records(
    records: &[PairwiseComparisonRecord],
    group_means_override: Option<&[(usize, f64)]>,
    options: CldOptions,
) -> Result<CldResult, StatsError> {
    compute_cld_from_records_inner(records, group_means_override, true, options)
}

/// Compute Piepho (2004) CLD directly from a completed [`ResultSheet`].
///
/// # Errors
/// Returns [`StatsError`] if the sheet lacks complete all-pairwise comparisons or fails validation.
pub fn compute_cld_from_result_sheet(
    sheet: &ResultSheet,
    group_means_override: Option<&[(usize, f64)]>,
    options: CldOptions,
) -> Result<CldResult, StatsError> {
    let allow_diff = !matches!(sheet.analysis.kind, AnalysisKind::Survival(_));
    compute_cld_from_records_inner(&sheet.pairwise_comparisons, group_means_override, allow_diff, options)
}
