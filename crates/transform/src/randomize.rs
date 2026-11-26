//! Seeded experimental subject randomization, uniform/stratified row sampling, and probability
//! distribution column simulation (`R9.14` / `QOL-14`, `docs/planning/calculators-and-quick-stats.md` §4).
//!
//! References:
//! - Durstenfeld, R. (1964). Algorithm 235: Random permutation. *Communications of the ACM*,
//!   7(7), 420. `doi:10.1145/364520.364540`.
//! - Hörmann, W. (1993). The generation of binomial random variates. *Journal of Statistical
//!   Computation and Simulation*, 46(1-2), 101-110. `doi:10.1080/00949659308811496`.

use std::collections::{BTreeSet, HashMap};

use curvebook_doc::error::MAX_SHEET_ID_BYTES;
use curvebook_doc::{
    Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, DataSheet, DocError, MAX_TITLE_BYTES, MultipleVariablesTable,
    SheetId, TableArchetype, VariableRole, validate_data_sets, validate_rows, validate_title,
};
use curvebook_math::{
    DeterministicRng,
    special::{deviance_bd0, stirling_err},
};

use crate::error::TransformError;
use crate::import::validate_cell_budget;
use crate::ops::reshape_select::try_retain_rows;
use crate::ops::{bounded_suffixed_str, snapshot_calculated_variables};

/// Largest exact-integer `f64` trial count (`2^53`) permitted for [`SimulatedDistribution::Binomial`].
const MAX_BINOMIAL_TRIALS: u64 = 1_u64 << 53;
const HALF_LN_2PI: f64 = 0.918_938_533_204_672_7;

const fn default_group_weight() -> usize {
    1
}

/// Experimental subject-to-treatment allocation scheme for [`randomize_subjects_to_groups`].
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AllocationScheme {
    /// Assign counts across arms via largest-remainder (Hamilton) integer apportionment preserving
    /// `weights` ratios, then shuffle globally across all `N` subjects (Durstenfeld 1964).
    ///
    /// **Deterministic tie-breaking contract:** When `N % sum(weights) != 0` and multiple arms
    /// share the same largest remainder `(N * w_i) mod sum(weights)`, surplus slots are assigned
    /// deterministically in **ascending arm-index order** (`0, 1, ...`), matching
    /// `docs/implementation/data-tables-and-transforms.md`. Note that this introduces a systematic
    /// lower-index (arm-0) surplus bias across seeds whenever `N` is not divisible by
    /// `sum(weights)` (for example, `3` vs `2` on `N = 5` with `[1, 1]` for every seed). Use
    /// [`Self::CompleteBalancedUnbiased`] when unbiased marginal allocation across seeds is desired.
    #[default]
    CompleteBalanced,
    /// Assign counts across arms via largest-remainder (Hamilton) integer apportionment with
    /// **seeded random tie-breaking** across equal-remainder tie groups (`rng.shuffle` over each
    /// tied-remainder group), then shuffle globally across all `N` subjects (Durstenfeld 1964).
    ///
    /// Deterministic per PRNG seed while achieving exact marginal allocation probabilities
    /// `w_j / sum(w)` across seeds for symmetric equal-remainder arms when `N % sum(weights) != 0`.
    CompleteBalancedUnbiased,
    /// Sequential permuted blocks whose sizes are fixed (`block_sizes.len() == 1`) or drawn
    /// uniformly per block from `block_sizes` (`block_sizes.len() > 1`). Every block size must be
    /// a positive multiple of `sum(weights)` within the bounded per-block capacity limit
    /// (Matts & Lachin, *Controlled Clinical Trials*, 1988, 9(4), 327–344).
    PermutedBlock {
        /// Candidate positive block sizes (each an integer multiple of the total group allocation weight sum).
        block_sizes: Vec<usize>,
    },
}

/// Named treatment arm and relative allocation weight for [`SubjectRandomizationConfig`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupAllocationWeight {
    /// Non-empty treatment arm label (for example `"Vehicle"` or `"Drug 10 mg/kg"`).
    pub label: String,
    /// Positive integer allocation ratio weight (`>= 1`, default `1`; e.g. `[1, 2]` for `1:2` control:treatment allocation).
    #[serde(default = "default_group_weight")]
    pub weight: usize,
}

impl GroupAllocationWeight {
    /// Create a treatment arm with default allocation weight `1`.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), weight: 1 }
    }

    /// Create a treatment arm with explicit integer allocation weight.
    #[must_use]
    pub fn with_weight(label: impl Into<String>, weight: usize) -> Self {
        Self { label: label.into(), weight }
    }
}

/// Configuration for seeded subject-to-group randomization ([`randomize_subjects_to_groups`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubjectRandomizationConfig {
    /// Identifier assigned to the generated schedule [`DataSheet`].
    pub sheet_id: String,
    /// Display title assigned to the generated schedule [`DataSheet`].
    pub title: String,
    /// Ordered unique subject/animal identifiers (`len >= 1`).
    pub subject_ids: Vec<String>,
    /// Treatment arms and positive integer allocation ratio weights (`len >= 1`).
    pub groups: Vec<GroupAllocationWeight>,
    /// Complete balanced Durstenfeld shuffle vs Matts–Lachin permuted-block randomization.
    pub scheme: AllocationScheme,
    /// 64-bit `DeterministicRng` seed recorded in ARRIVE provenance metadata.
    pub seed: u64,
}

impl SubjectRandomizationConfig {
    /// Construct a configuration for `subject_count` sequentially numbered subjects (`"1"..=N`).
    ///
    /// Prefer [`Self::try_from_subject_count`] when `subject_count` originates from untrusted or
    /// user-supplied input so `subject_count <= MAX_ROWS` is validated before allocating `subject_ids`.
    #[must_use]
    pub fn from_subject_count(
        sheet_id: impl Into<String>,
        title: impl Into<String>,
        subject_count: usize,
        groups: Vec<GroupAllocationWeight>,
        scheme: AllocationScheme,
        seed: u64,
    ) -> Self {
        let subject_ids = (1..=subject_count).map(|i| i.to_string()).collect();
        Self { sheet_id: sheet_id.into(), title: title.into(), subject_ids, groups, scheme, seed }
    }

    /// Fallibly construct a configuration for `subject_count` sequentially numbered subjects
    /// (`"1"..=N`), validating `subject_count <= MAX_ROWS` before allocating `subject_ids`.
    ///
    /// # Errors
    /// Returns [`TransformError::Doc`] (`DocError::TooManyRows`) when `subject_count > MAX_ROWS`.
    pub fn try_from_subject_count(
        sheet_id: impl Into<String>,
        title: impl Into<String>,
        subject_count: usize,
        groups: Vec<GroupAllocationWeight>,
        scheme: AllocationScheme,
        seed: u64,
    ) -> Result<Self, TransformError> {
        validate_rows(subject_count)?;
        let subject_ids = (1..=subject_count).map(|i| i.to_string()).collect();
        Ok(Self { sheet_id: sheet_id.into(), title: title.into(), subject_ids, groups, scheme, seed })
    }
}

/// Generated randomization schedule sheet alongside ARRIVE guideline metadata constants.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RandomizationOutput {
    /// `MultipleVariables` schedule sheet with `Subject_ID`, `Allocation_Order`, `Block_Index`, `Block_Size`, and `Assigned_Group` columns.
    pub data_sheet: DataSheet,
    /// Key–value ARRIVE 2.0 reproducibility metadata pairs (PRNG seed, scheme, arm ratios, total `N`) suitable for `InfoSheet` insertion.
    pub arrive_constants: Vec<(String, String)>,
}

/// Configuration for deterministic row sampling without replacement ([`random_sample_rows`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RowSampleConfig {
    /// 64-bit `DeterministicRng` seed ensuring reproducible row selection.
    pub seed: u64,
    /// Number of rows (`1..=source_rows`) to draw without replacement.
    pub sample_size: usize,
    /// Optional zero-based categorical column index partitioning rows into strata prior to sampling; `None` performs simple random sampling.
    #[serde(default)]
    pub stratify_column: Option<usize>,
    /// When `true` and `stratify_column` is `Some(_)`, allocates draws proportionally to stratum sizes; when `false`, splits draws evenly across strata.
    #[serde(default)]
    pub proportional_allocation: bool,
    /// Optional explicit [`SheetId`] string for the sampled output sheet; `None` derives `<source>-sample`.
    #[serde(default)]
    pub output_id: Option<String>,
    /// Optional explicit display title for the sampled output sheet; `None` derives `<title> (Sample)`.
    #[serde(default)]
    pub output_title: Option<String>,
}

/// Probability distribution family supported by [`simulate_distribution_columns`].
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SimulatedDistribution {
    /// Continuous uniform distribution over `[min, max)` (`min < max`, finite).
    Uniform {
        /// Inclusive lower support bound.
        min: f64,
        /// Exclusive upper support bound (`> min`).
        max: f64,
    },
    /// Univariate normal (Gaussian) distribution `N(mean, sd^2)`.
    Gaussian {
        /// Population mean `mu` (finite).
        mean: f64,
        /// Population standard deviation `sigma >= 0` (finite).
        sd: f64,
    },
    /// Lognormal distribution `exp(N(mean_log, sd_log^2))` parameterized on the natural-log scale.
    LogNormal {
        /// Mean `mu` of `ln(X)` (finite).
        mean_log: f64,
        /// Standard deviation `sigma >= 0` of `ln(X)` (finite).
        sd_log: f64,
    },
    /// Discrete binomial count distribution `Binomial(trials, p)` (Hörmann 1993 BTPE / inversion).
    Binomial {
        /// Number of Bernoulli trials `n` (`<= 2^53` for exact `f64` integer representation).
        trials: u64,
        /// Per-trial success probability `p in [0.0, 1.0]`.
        p: f64,
    },
    /// Discrete Poisson count distribution `Poisson(lambda)`.
    Poisson {
        /// Non-negative rate / mean parameter `lambda >= 0.0` (finite).
        lambda: f64,
    },
    /// Continuous exponential waiting-time distribution with density `rate * exp(-rate * x)` for `x >= 0`.
    Exponential {
        /// Strictly positive rate parameter `lambda > 0.0` (`mean = 1 / rate`).
        rate: f64,
    },
}

/// Single named output column specification for [`DistributionSimulationConfig`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SimulatedColumnSpec {
    /// Column or variable header title (`<= MAX_TITLE_BYTES`).
    pub name: String,
    /// Parametric probability distribution sampled across `n_rows`.
    pub distribution: SimulatedDistribution,
}

/// Configuration for seeded multi-distribution column simulation ([`simulate_distribution_columns`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DistributionSimulationConfig {
    /// Identifier assigned to the simulated output [`DataSheet`].
    pub sheet_id: String,
    /// Display title assigned to the simulated output [`DataSheet`].
    pub title: String,
    /// Number of independent simulated observations (`1..=MAX_ROWS`) per column.
    pub n_rows: usize,
    /// 64-bit `DeterministicRng` seed ensuring reproducible draws across platforms.
    pub seed: u64,
    /// Non-empty list of column specifications (`1..=MAX_DATA_SETS`).
    pub columns: Vec<SimulatedColumnSpec>,
    /// When `true`, emits a [`MultipleVariablesTable`] with `Continuous` variable columns; when `false`, emits an unpaired [`ColumnTable`].
    #[serde(default)]
    pub use_multiple_variables: bool,
}

/// Deterministically randomize subjects across treatment arms and emit an ARRIVE-compliant
/// `MultipleVariables` schedule sheet (`Subject_ID`, `Allocation_Order`, `Block_Index`,
/// `Block_Size`, `Assigned_Group`).
///
/// # Errors
/// Returns [`TransformError::InvalidSpec`] when subjects/groups are empty, weights are zero, or
/// permuted block sizes are not positive multiples of the sum of weights.
#[allow(clippy::cast_precision_loss)]
pub fn randomize_subjects_to_groups(config: &SubjectRandomizationConfig) -> Result<RandomizationOutput, TransformError> {
    let sum_weights = validate_randomization_config(config)?;
    let n = config.subject_ids.len();
    let assignments = assign_subject_groups(config, sum_weights)?;

    let mut mv = MultipleVariablesTable::new(5, n)?;
    let titles = ["Subject_ID", "Allocation_Order", "Block_Index", "Block_Size", "Assigned_Group"];
    let roles = [VariableRole::Categorical, VariableRole::Continuous, VariableRole::Continuous, VariableRole::Continuous, VariableRole::Categorical];
    for (col_idx, (col_title, role)) in titles.iter().zip(roles.iter()).enumerate() {
        if let Some(var) = mv.variables.get_mut(col_idx) {
            var.title = (*col_title).to_owned();
            var.lock_role(*role);
        }
    }

    for (r, (group_idx, block_idx, block_size)) in assignments.iter().enumerate() {
        let Some(subject_id) = config.subject_ids.get(r) else {
            continue;
        };
        let Some(group) = config.groups.get(*group_idx) else {
            continue;
        };
        set_mv_cell(&mut mv, 0, r, Cell::text(subject_id.trim()));
        set_mv_cell(&mut mv, 1, r, Cell::number((r + 1) as f64));
        set_mv_cell(&mut mv, 2, r, Cell::number(*block_idx as f64));
        set_mv_cell(&mut mv, 3, r, Cell::number(*block_size as f64));
        set_mv_cell(&mut mv, 4, r, Cell::text(group.label.trim()));
    }

    let data_sheet = DataSheet::new(config.sheet_id.as_str(), config.title.as_str(), TableArchetype::MultipleVariables(mv))?;
    let arrive_constants = build_arrive_constants(config, n);
    Ok(RandomizationOutput { data_sheet, arrive_constants })
}

/// Deterministically sample `config.sample_size` unique rows from `source` **without replacement**
/// via partial Fisher–Yates shuffling (optionally stratified proportionally or equally across
/// `config.stratify_column`).
///
/// Selected row indices are sorted in ascending original row order to preserve row-paired
/// semantics on `Xy`, `Survival`, and matched `Column` tables while keeping source observations
/// unmutated.
///
/// # Errors
/// Returns [`TransformError::InvalidSpec`] when `sample_size == 0`, `sample_size > row_count`,
/// `stratify_column` is out of bounds, or `output_id` / `output_title` is invalid.
pub fn random_sample_rows(source: &DataSheet, config: &RowSampleConfig) -> Result<DataSheet, TransformError> {
    let selected = sample_row_indices(source, config)?;
    let id = config.output_id.clone().unwrap_or_else(|| bounded_suffixed_str(source.id.as_str(), "-sample", MAX_SHEET_ID_BYTES));
    if id.trim().is_empty() {
        return Err(TransformError::Doc(DocError::EmptyIdentifier));
    }
    let title = config.output_title.clone().unwrap_or_else(|| bounded_suffixed_str(&source.title, " (Random Sample)", MAX_TITLE_BYTES));
    validate_title(&title)?;

    let mut out = source.clone();
    out.id = SheetId::new(id);
    out.title = title;
    out.frozen = false;
    out.linked_regions.clear();
    try_retain_rows(&mut out.archetype, &selected)?;
    snapshot_calculated_variables(&mut out.archetype);
    out.exclusion_notes = remap_exclusion_notes(&source.exclusion_notes, &selected);
    out.validate()?;
    Ok(out)
}

/// Compute the sorted, strictly distinct sampled row indices for [`random_sample_rows`].
///
/// # Errors
/// Returns [`TransformError::InvalidSpec`] when `sample_size == 0`, `sample_size > row_count`, or
/// `stratify_column` is out of bounds.
pub fn sample_row_indices(source: &DataSheet, config: &RowSampleConfig) -> Result<Vec<usize>, TransformError> {
    source.archetype.validate()?;
    let row_count = source.archetype.row_count();
    if config.sample_size == 0 {
        return Err(TransformError::InvalidSpec { message: String::from("random sample_size must be >= 1") });
    }
    if config.sample_size > row_count {
        return Err(TransformError::InvalidSpec {
            message: format!("random sample_size ({}) exceeds table row count ({row_count})", config.sample_size),
        });
    }
    if let Some(col_idx) = config.stratify_column {
        let col_count = source.archetype.column_count();
        if col_idx >= col_count {
            return Err(TransformError::InvalidSpec {
                message: format!("stratify_column index {col_idx} out of bounds for table with {col_count} column(s)"),
            });
        }
        return sample_stratified_indices(source, col_idx, config);
    }

    let mut rng = DeterministicRng::from_seed(config.seed);
    let mut pool: Vec<usize> = (0..row_count).collect();
    partial_fisher_yates(&mut pool, config.sample_size, &mut rng);
    pool.truncate(config.sample_size);
    pool.sort_unstable();
    Ok(pool)
}

/// Generate a synthetic `Column` or `MultipleVariables` [`DataSheet`] populated from independent
/// seeded probability distribution streams per column.
///
/// # Errors
/// Returns [`TransformError::InvalidSpec`] when `n_rows == 0`, `columns` is empty, or any
/// distribution parameter violates its mathematical domain.
pub fn simulate_distribution_columns(config: &DistributionSimulationConfig) -> Result<DataSheet, TransformError> {
    validate_simulation_config(config)?;

    let mut master_rng = DeterministicRng::from_seed(config.seed);
    let archetype = if config.use_multiple_variables {
        let mut mv = MultipleVariablesTable::new(config.columns.len(), config.n_rows)?;
        for (c, spec) in config.columns.iter().enumerate() {
            let col_seed = master_rng.next_u64();
            let mut col_rng = DeterministicRng::from_seed(col_seed);
            if let Some(var) = mv.variables.get_mut(c) {
                var.title = spec.name.clone();
                var.lock_role(VariableRole::Continuous);
                for r in 0..config.n_rows {
                    let v = sample_distribution_value(&mut col_rng, spec.distribution)?;
                    if let Some(cell) = var.cells.get_mut(r) {
                        *cell = Cell::number(v);
                    }
                }
            }
        }
        TableArchetype::MultipleVariables(mv)
    } else {
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, config.columns.len(), config.n_rows)?;
        for (c, spec) in config.columns.iter().enumerate() {
            let col_seed = master_rng.next_u64();
            let mut col_rng = DeterministicRng::from_seed(col_seed);
            if let Some(ds) = table.data_sets.get_mut(c) {
                ds.title = spec.name.clone();
                for r in 0..config.n_rows {
                    let v = sample_distribution_value(&mut col_rng, spec.distribution)?;
                    if let Some(cell) = ds.cells.get_mut(r).and_then(|row| row.get_mut(0)) {
                        *cell = Cell::number(v);
                    }
                }
            }
        }
        TableArchetype::Column(table)
    };

    Ok(DataSheet::new(config.sheet_id.as_str(), config.title.as_str(), archetype)?)
}

fn set_mv_cell(mv: &mut MultipleVariablesTable, col: usize, row: usize, cell: Cell) {
    if let Some(slot) = mv.variables.get_mut(col).and_then(|v| v.cells.get_mut(row)) {
        *slot = cell;
    }
}

fn validate_randomization_config(config: &SubjectRandomizationConfig) -> Result<usize, TransformError> {
    if config.sheet_id.trim().is_empty() {
        return Err(TransformError::Doc(DocError::EmptyIdentifier));
    }
    validate_title(&config.title)?;
    if config.subject_ids.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("subject_ids must not be empty") });
    }
    validate_rows(config.subject_ids.len())?;
    let mut seen_subjects = BTreeSet::new();
    for (idx, sid) in config.subject_ids.iter().enumerate() {
        let trimmed = sid.trim();
        if trimmed.is_empty() {
            return Err(TransformError::InvalidSpec { message: format!("subject_ids[{idx}] must not be blank") });
        }
        validate_title(sid)?;
        if !seen_subjects.insert(trimmed) {
            return Err(TransformError::InvalidSpec { message: format!("duplicate subject_id `{trimmed}` at subject_ids[{idx}]") });
        }
    }
    if config.groups.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("groups must not be empty") });
    }
    let mut seen_groups = BTreeSet::new();
    let mut sum_weights = 0_usize;
    for (idx, g) in config.groups.iter().enumerate() {
        let trimmed = g.label.trim();
        if trimmed.is_empty() {
            return Err(TransformError::InvalidSpec { message: format!("groups[{idx}].label must not be blank") });
        }
        validate_title(&g.label)?;
        if !seen_groups.insert(trimmed) {
            return Err(TransformError::InvalidSpec { message: format!("duplicate group label `{trimmed}` at groups[{idx}].label") });
        }
        if g.weight == 0 {
            return Err(TransformError::InvalidSpec { message: format!("groups[{idx}] (`{trimmed}`) weight must be >= 1") });
        }
        sum_weights = sum_weights
            .checked_add(g.weight)
            .ok_or_else(|| TransformError::InvalidSpec { message: String::from("sum of group weights overflows") })?;
    }
    if let AllocationScheme::PermutedBlock { block_sizes } = &config.scheme {
        if block_sizes.is_empty() {
            return Err(TransformError::InvalidSpec { message: String::from("PermutedBlock block_sizes must not be empty") });
        }
        let n_subjects = config.subject_ids.len();
        let max_block_size = n_subjects.saturating_mul(16).clamp(65_536, curvebook_doc::MAX_ROWS);
        for &b in block_sizes {
            if b == 0 {
                return Err(TransformError::InvalidSpec {
                    message: format!("PermutedBlock size {b} must be a positive multiple of sum(weights) = {sum_weights}"),
                });
            }
            if b > max_block_size {
                return Err(TransformError::InvalidSpec {
                    message: format!("PermutedBlock size {b} exceeds maximum permitted block size {max_block_size} for {n_subjects} subject(s)"),
                });
            }
            if b % sum_weights != 0 {
                return Err(TransformError::InvalidSpec {
                    message: format!("PermutedBlock size {b} must be a positive multiple of sum(weights) = {sum_weights}"),
                });
            }
        }
    }
    Ok(sum_weights)
}

fn assign_subject_groups(config: &SubjectRandomizationConfig, sum_weights: usize) -> Result<Vec<(usize, usize, usize)>, TransformError> {
    let n = config.subject_ids.len();
    let mut rng = DeterministicRng::from_seed(config.seed);

    match &config.scheme {
        AllocationScheme::CompleteBalanced => {
            let weights: Vec<usize> = config.groups.iter().map(|g| g.weight).collect();
            let counts = hamilton_apportion(n, &weights);
            let mut pool = Vec::with_capacity(n);
            for (g_idx, &count) in counts.iter().enumerate() {
                pool.resize(pool.len() + count, g_idx);
            }
            rng.shuffle(&mut pool);
            Ok(pool.into_iter().map(|g_idx| (g_idx, 1, n)).collect())
        }
        AllocationScheme::CompleteBalancedUnbiased => {
            let weights: Vec<usize> = config.groups.iter().map(|g| g.weight).collect();
            let counts = hamilton_apportion_unbiased(n, &weights, &mut rng);
            let mut pool = Vec::with_capacity(n);
            for (g_idx, &count) in counts.iter().enumerate() {
                pool.resize(pool.len() + count, g_idx);
            }
            rng.shuffle(&mut pool);
            Ok(pool.into_iter().map(|g_idx| (g_idx, 1, n)).collect())
        }
        AllocationScheme::PermutedBlock { block_sizes } => {
            let mut out = Vec::with_capacity(n);
            let mut block_idx = 0_usize;
            while out.len() < n {
                block_idx += 1;
                let block_size = if block_sizes.len() == 1 {
                    block_sizes[0]
                } else {
                    let picked = rng.gen_index(block_sizes.len()).unwrap_or(0);
                    block_sizes.get(picked).copied().unwrap_or(block_sizes[0])
                };
                let mult = block_size / sum_weights;
                let mut block = Vec::new();
                block.try_reserve_exact(block_size).map_err(|_| TransformError::InvalidSpec {
                    message: format!("PermutedBlock size {block_size} exceeds available memory capacity"),
                })?;
                for (g_idx, g) in config.groups.iter().enumerate() {
                    let count = g
                        .weight
                        .checked_mul(mult)
                        .ok_or_else(|| TransformError::InvalidSpec { message: String::from("block slot count overflow") })?;
                    block.resize(block.len() + count, g_idx);
                }
                rng.shuffle(&mut block);
                let take = (n - out.len()).min(block_size);
                for g_idx in block.into_iter().take(take) {
                    out.push((g_idx, block_idx, block_size));
                }
            }
            Ok(out)
        }
    }
}

fn build_arrive_constants(config: &SubjectRandomizationConfig, total_subjects: usize) -> Vec<(String, String)> {
    let method = match &config.scheme {
        AllocationScheme::CompleteBalanced => String::from("complete_balanced"),
        AllocationScheme::CompleteBalancedUnbiased => String::from("complete_balanced_unbiased"),
        AllocationScheme::PermutedBlock { .. } => String::from("permuted_block"),
    };
    let ratio = config.groups.iter().map(|g| g.weight.to_string()).collect::<Vec<_>>().join(":");
    let block_sizes = match &config.scheme {
        AllocationScheme::CompleteBalanced | AllocationScheme::CompleteBalancedUnbiased => total_subjects.to_string(),
        AllocationScheme::PermutedBlock { block_sizes } => block_sizes.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "),
    };
    vec![
        (String::from("randomization.seed"), config.seed.to_string()),
        (String::from("randomization.method"), method),
        (String::from("randomization.allocation_ratio"), ratio),
        (String::from("randomization.block_sizes"), block_sizes),
        (String::from("randomization.total_subjects"), total_subjects.to_string()),
    ]
}

/// Largest-remainder (Hamilton) integer apportionment of `total` units across `weights`.
///
/// Assigns base quotas `floor(total * w_i / sum(weights))` and distributes remaining units to
/// entries with largest exact remainders `(total * w_i) mod sum(weights)`, breaking ties deterministically
/// in **ascending index order** (`0, 1, ...`).
fn hamilton_apportion(total: usize, weights: &[usize]) -> Vec<usize> {
    let k = weights.len();
    let mut alloc = vec![0_usize; k];
    if total == 0 || k == 0 {
        return alloc;
    }
    let total_weight: u128 = weights.iter().map(|&w| w as u128).sum();
    if total_weight == 0 {
        return alloc;
    }
    let total_u128 = total as u128;
    let mut remainders: Vec<(usize, u128)> = Vec::with_capacity(k);
    let mut assigned = 0_usize;

    for (idx, &w) in weights.iter().enumerate() {
        let num = total_u128 * (w as u128);
        #[allow(clippy::cast_possible_truncation)]
        let base = (num / total_weight) as usize;
        let rem = num % total_weight;
        alloc[idx] = base;
        assigned += base;
        remainders.push((idx, rem));
    }

    let leftover = total.saturating_sub(assigned);
    remainders.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for &(idx, _) in remainders.iter().take(leftover) {
        alloc[idx] += 1;
    }
    alloc
}

/// Largest-remainder (Hamilton) integer apportionment breaking equal-remainder ties uniformly at
/// random via `rng.shuffle` over each tied-remainder group (Durstenfeld 1964).
fn hamilton_apportion_unbiased(total: usize, weights: &[usize], rng: &mut DeterministicRng) -> Vec<usize> {
    let k = weights.len();
    let mut alloc = vec![0_usize; k];
    if total == 0 || k == 0 {
        return alloc;
    }
    let total_weight: u128 = weights.iter().map(|&w| w as u128).sum();
    if total_weight == 0 {
        return alloc;
    }
    let total_u128 = total as u128;
    let mut remainders: Vec<(usize, u128)> = Vec::with_capacity(k);
    let mut assigned = 0_usize;

    for (idx, &w) in weights.iter().enumerate() {
        let num = total_u128 * (w as u128);
        #[allow(clippy::cast_possible_truncation)]
        let base = (num / total_weight) as usize;
        let rem = num % total_weight;
        alloc[idx] = base;
        assigned += base;
        remainders.push((idx, rem));
    }

    let leftover = total.saturating_sub(assigned);
    remainders.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut start = 0;
    while start < remainders.len() {
        let rem_val = remainders[start].1;
        let mut end = start + 1;
        while end < remainders.len() && remainders[end].1 == rem_val {
            end += 1;
        }
        if end - start > 1 {
            rng.shuffle(&mut remainders[start..end]);
        }
        start = end;
    }
    for &(idx, _) in remainders.iter().take(leftover) {
        alloc[idx] += 1;
    }
    alloc
}

fn extract_active_stratum_key(arch: &TableArchetype, row: usize, col: usize) -> Option<String> {
    let cell = arch.data_cell(CellCoord::new(row, col, 0))?;
    if cell.excluded {
        return None;
    }
    match &cell.value {
        CellValue::Empty => None,
        CellValue::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CellValue::Number(v) if v.is_finite() => {
            let normalized = if *v == 0.0 { 0.0 } else { *v };
            Some(format!("{normalized}"))
        }
        CellValue::Number(_) => None,
    }
}

fn sample_stratified_indices(source: &DataSheet, col_idx: usize, config: &RowSampleConfig) -> Result<Vec<usize>, TransformError> {
    let row_count = source.archetype.row_count();
    let mut strata: Vec<Vec<usize>> = Vec::new();
    let mut key_to_stratum: HashMap<String, usize> = HashMap::new();
    let mut eligible_rows = 0_usize;

    for r in 0..row_count {
        let Some(key) = extract_active_stratum_key(&source.archetype, r, col_idx) else {
            continue;
        };
        eligible_rows += 1;
        if let Some(&s_idx) = key_to_stratum.get(&key) {
            if let Some(bucket) = strata.get_mut(s_idx) {
                bucket.push(r);
            }
        } else {
            let s_idx = strata.len();
            key_to_stratum.insert(key, s_idx);
            strata.push(vec![r]);
        }
    }

    if eligible_rows == 0 {
        return Err(TransformError::InvalidSpec {
            message: format!("stratify_column {col_idx} contains no active non-empty, non-excluded stratum values"),
        });
    }
    if config.sample_size > eligible_rows {
        return Err(TransformError::InvalidSpec {
            message: format!(
                "random sample_size ({}) exceeds active stratified row count ({eligible_rows}) in stratify_column {col_idx}",
                config.sample_size
            ),
        });
    }

    let sizes: Vec<usize> = strata.iter().map(Vec::len).collect();
    let targets = if config.proportional_allocation {
        apportion_stratified_proportional(config.sample_size, &sizes)
    } else {
        apportion_stratified_equal(config.sample_size, &sizes)
    };

    let mut rng = DeterministicRng::from_seed(config.seed);
    let mut selected = Vec::with_capacity(config.sample_size);
    for (bucket, &take) in strata.iter_mut().zip(targets.iter()) {
        if take == 0 {
            continue;
        }
        partial_fisher_yates(bucket, take, &mut rng);
        selected.extend_from_slice(&bucket[..take]);
    }
    selected.sort_unstable();
    Ok(selected)
}

/// Iterative constrained Hamilton allocation for stratified proportional sampling (`sum n_h = S`, `l_h <= n_h <= N_h`).
///
/// Enforces minimum-one clamping (`l_h = 1` when `sample_size >= stratum_sizes.len()` and `N_h >= 1`)
/// and stratum-capacity capping (`n_h <= N_h`), freezing any clamped strata and re-running largest-remainder
/// apportionment (with ascending stratum-index tie-breaks) on the residual `remaining` budget across active strata.
fn apportion_stratified_proportional(sample_size: usize, stratum_sizes: &[usize]) -> Vec<usize> {
    let s_count = stratum_sizes.len();
    let mut alloc = vec![0_usize; s_count];
    if sample_size == 0 || s_count == 0 {
        return alloc;
    }

    let enforce_min_one = sample_size >= s_count;
    let mut active: Vec<usize> = (0..s_count).collect();
    let mut remaining = sample_size;

    loop {
        if active.is_empty() || remaining == 0 {
            break;
        }
        let active_weights: Vec<usize> = active.iter().map(|&idx| stratum_sizes[idx]).collect();
        let proposal = hamilton_apportion(remaining, &active_weights);

        let mut clamped_any = false;
        let mut next_active = Vec::with_capacity(active.len());

        for (&stratum_idx, &proposed) in active.iter().zip(proposal.iter()) {
            let cap = stratum_sizes[stratum_idx];
            if enforce_min_one && proposed == 0 && cap >= 1 {
                alloc[stratum_idx] = 1;
                remaining = remaining.saturating_sub(1);
                clamped_any = true;
            } else if proposed > cap {
                alloc[stratum_idx] = cap;
                remaining = remaining.saturating_sub(cap);
                clamped_any = true;
            } else {
                next_active.push(stratum_idx);
            }
        }

        if !clamped_any {
            for (&stratum_idx, &proposed) in active.iter().zip(proposal.iter()) {
                alloc[stratum_idx] = proposed;
            }
            break;
        }
        active = next_active;
    }

    alloc
}

/// Equal-size stratified water-filling allocation subject to per-stratum capacity (`0 <= n_h <= N_h`).
///
/// Repeatedly distributes `remaining / |unsaturated|` evenly across strata with `n_h < N_h`, allocating
/// residual units (`remaining < |unsaturated|`) one-by-one in **ascending stratum-index order**.
fn apportion_stratified_equal(sample_size: usize, stratum_sizes: &[usize]) -> Vec<usize> {
    let s_count = stratum_sizes.len();
    let mut alloc = vec![0_usize; s_count];
    let mut remaining = sample_size;

    while remaining > 0 {
        let unsaturated: Vec<usize> = (0..s_count).filter(|&i| alloc[i] < stratum_sizes[i]).collect();
        if unsaturated.is_empty() {
            break;
        }
        let step = remaining / unsaturated.len();
        if step == 0 {
            for &idx in unsaturated.iter().take(remaining) {
                alloc[idx] += 1;
            }
            break;
        }
        for idx in unsaturated {
            let add = step.min(stratum_sizes[idx] - alloc[idx]);
            alloc[idx] += add;
            remaining -= add;
        }
    }
    alloc
}

/// First `k` steps of a Fisher-Yates shuffle (Durstenfeld 1964, run front to back): position `i`
/// swaps with a uniformly chosen position in `i..n`, so `pool[..k]` is a uniform random sample
/// without replacement.
fn partial_fisher_yates(pool: &mut [usize], k: usize, rng: &mut DeterministicRng) {
    let n = pool.len();
    let take = k.min(n);
    for i in 0..take {
        if let Some(offset) = rng.gen_index(n - i) {
            pool.swap(i, i + offset);
        }
    }
}

pub(crate) fn remap_exclusion_notes(notes: &[CellExclusionNote], selected_rows: &[usize]) -> Vec<CellExclusionNote> {
    if notes.is_empty() {
        return Vec::new();
    }
    let mut old_to_new: HashMap<usize, usize> = HashMap::with_capacity(selected_rows.len());
    for (new_row, &old_row) in selected_rows.iter().enumerate() {
        old_to_new.insert(old_row, new_row);
    }
    let mut remapped: Vec<CellExclusionNote> = notes
        .iter()
        .filter_map(|note| {
            old_to_new.get(&note.coord.row).map(|&new_row| CellExclusionNote {
                coord: CellCoord::new(new_row, note.coord.col, note.coord.subcol),
                reason_tag: note.reason_tag.clone(),
                note: note.note.clone(),
                timestamp_utc: note.timestamp_utc.clone(),
            })
        })
        .collect();
    remapped.sort_by_key(|n| n.coord);
    remapped
}

fn validate_simulation_config(config: &DistributionSimulationConfig) -> Result<(), TransformError> {
    if config.sheet_id.trim().is_empty() {
        return Err(TransformError::Doc(DocError::EmptyIdentifier));
    }
    validate_title(&config.title)?;
    if config.n_rows == 0 {
        return Err(TransformError::InvalidSpec { message: String::from("distribution simulation n_rows must be >= 1") });
    }
    validate_rows(config.n_rows)?;
    if config.columns.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("distribution simulation columns must not be empty") });
    }
    validate_data_sets(config.columns.len())?;
    validate_cell_budget(config.n_rows, config.columns.len(), 1, 0, "distribution simulation")?;

    for (idx, col) in config.columns.iter().enumerate() {
        validate_title(&col.name)?;
        validate_distribution(idx, col.distribution)?;
    }
    Ok(())
}

fn validate_distribution(col_idx: usize, dist: SimulatedDistribution) -> Result<(), TransformError> {
    match dist {
        SimulatedDistribution::Uniform { min, max } => {
            if !min.is_finite() || !max.is_finite() || min >= max {
                return Err(TransformError::InvalidSpec {
                    message: format!("column {col_idx} Uniform requires finite min < max, got [{min}, {max}]"),
                });
            }
        }
        SimulatedDistribution::Gaussian { mean, sd } => {
            if !mean.is_finite() || !sd.is_finite() || sd < 0.0 {
                return Err(TransformError::InvalidSpec {
                    message: format!("column {col_idx} Gaussian requires finite mean and sd >= 0, got mean={mean}, sd={sd}"),
                });
            }
        }
        SimulatedDistribution::LogNormal { mean_log, sd_log } => {
            if !mean_log.is_finite() || !sd_log.is_finite() || sd_log < 0.0 {
                return Err(TransformError::InvalidSpec {
                    message: format!("column {col_idx} LogNormal requires finite mean_log and sd_log >= 0, got mean_log={mean_log}, sd_log={sd_log}"),
                });
            }
        }
        SimulatedDistribution::Binomial { trials, p } => {
            if trials > MAX_BINOMIAL_TRIALS {
                return Err(TransformError::InvalidSpec {
                    message: format!("column {col_idx} Binomial trials ({trials}) exceeds exact f64 integer bound {MAX_BINOMIAL_TRIALS}"),
                });
            }
            if !p.is_finite() || !(0.0..=1.0).contains(&p) {
                return Err(TransformError::InvalidSpec { message: format!("column {col_idx} Binomial probability p must lie in [0, 1], got {p}") });
            }
        }
        SimulatedDistribution::Poisson { lambda } => {
            if !lambda.is_finite() || !(0.0..=1e15).contains(&lambda) {
                return Err(TransformError::InvalidSpec {
                    message: format!("column {col_idx} Poisson lambda must be finite in [0, 1e15], got {lambda}"),
                });
            }
        }
        SimulatedDistribution::Exponential { rate } => {
            if !rate.is_finite() || rate <= 0.0 {
                return Err(TransformError::InvalidSpec { message: format!("column {col_idx} Exponential rate must be finite and > 0, got {rate}") });
            }
        }
    }
    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn sample_distribution_value(rng: &mut DeterministicRng, dist: SimulatedDistribution) -> Result<f64, TransformError> {
    let val = match dist {
        SimulatedDistribution::Uniform { min, max } => {
            let u = rng.next_f64();
            (1.0 - u) * min + u * max
        }
        SimulatedDistribution::Gaussian { mean, sd } => {
            if sd == 0.0 {
                mean
            } else {
                rng.normal(mean, sd)?
            }
        }
        SimulatedDistribution::LogNormal { mean_log, sd_log } => {
            let z = if sd_log == 0.0 { mean_log } else { rng.normal(mean_log, sd_log)? };
            let exp_z = z.exp();
            if !exp_z.is_finite() {
                return Err(TransformError::EvalError { message: format!("LogNormal sample exp({z}) overflowed finite f64 range") });
            }
            exp_z
        }
        SimulatedDistribution::Binomial { trials, p } => sample_binomial(rng, trials, p)? as f64,
        SimulatedDistribution::Poisson { lambda } => rng.poisson(lambda)? as f64,
        SimulatedDistribution::Exponential { rate } => -rng.next_open_f64().ln() / rate,
    };
    if !val.is_finite() {
        return Err(TransformError::EvalError { message: format!("simulated sample for {dist:?} overflowed finite f64 range") });
    }
    Ok(val)
}

#[allow(clippy::cast_precision_loss)]
fn sample_binomial(rng: &mut DeterministicRng, trials: u64, p: f64) -> Result<u64, TransformError> {
    if trials > MAX_BINOMIAL_TRIALS {
        return Err(TransformError::InvalidSpec {
            message: format!("Binomial trials ({trials}) exceeds exact f64 integer bound {MAX_BINOMIAL_TRIALS}"),
        });
    }
    if trials == 0 || p == 0.0 {
        return Ok(0);
    }
    if p == 1.0 {
        return Ok(trials);
    }
    let flip = p > 0.5;
    let q = if flip { 1.0 - p } else { p };
    if q == 0.0 {
        return Ok(if flip { trials } else { 0 });
    }

    let drawn = if trials <= 32 {
        let mut count = 0_u64;
        for _ in 0..trials {
            if rng.next_f64() < q {
                count += 1;
            }
        }
        count
    } else if (trials as f64) * q < 10.0 {
        sample_binomial_geo(rng, trials, q)
    } else {
        sample_binomial_btrs(rng, trials, q)
    };

    Ok(if flip { trials - drawn } else { drawn })
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn sample_binomial_geo(rng: &mut DeterministicRng, trials: u64, q: f64) -> u64 {
    let ln_1_q = (-q).ln_1p();
    if !ln_1_q.is_finite() || ln_1_q >= 0.0 {
        return 0;
    }
    let mut sum = 0_u64;
    let mut k = 0_u64;
    loop {
        let u = rng.next_open_f64();
        let gap_minus_one = (u.ln() / ln_1_q).floor();
        let remaining = trials - sum;
        if !gap_minus_one.is_finite() || gap_minus_one < 0.0 || gap_minus_one >= (remaining as f64) {
            break;
        }
        let step_minus_one = gap_minus_one as u64;
        if step_minus_one >= remaining {
            break;
        }
        sum += step_minus_one + 1;
        k += 1;
    }
    k
}

// Hörmann (1993) BTRS transformed rejection for binomial with n * q >= 10, q <= 0.5.
// Set-up constants b, a, c and the mode follow the paper's Table 1 and BTRS step 0. Alpha uses
// Table 1's `(2.83 + 5.1 / b) * spq` (also used in the paper's BTRD); the printed BTRS listing
// shows `1.5 / b`, which disagrees with Table 1. The paper's `us >= 0.07 && v <= v_r` squeeze is
// omitted, which only costs speed. The acceptance test compares against the log-pmf ratio computed
// with Loader's saddlepoint form instead of the paper's Stirling log-factorials.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn sample_binomial_btrs(rng: &mut DeterministicRng, trials: u64, q: f64) -> u64 {
    let n = trials as f64;
    let s = (n * q * (1.0 - q)).sqrt();
    let b = 1.15 + 2.53 * s;
    let a = -0.0873 + 0.0248 * b + 0.01 * q;
    let c = n * q + 0.5;
    let alpha = (2.83 + 5.1 / b) * s;
    let mode = ((n * q + q).floor() as u64).min(trials);
    let ln_pm = ln_binomial_pmf(trials, mode, q);

    for _ in 0..10_000 {
        let u = rng.next_open_f64() - 0.5;
        let v = rng.next_open_f64();
        let us = 0.5 - u.abs();
        let k_f = ((2.0 * a / us + b) * u + c).floor();
        if !k_f.is_finite() || k_f < 0.0 || k_f > n {
            continue;
        }
        let k = k_f as u64;
        if k > trials {
            continue;
        }
        let lhs = (v * alpha / (a / (us * us) + b)).ln();
        let rhs = ln_binomial_pmf(trials, k, q) - ln_pm;
        if lhs <= rhs {
            return k;
        }
    }
    mode
}

/// Evaluate `ln C(n, k) q^k (1 - q)^(n - k)` via Loader's saddlepoint deviance without
/// subtracting `O(n)` log-factorial and log-probability terms.
#[allow(clippy::cast_precision_loss)]
fn ln_binomial_pmf(trials: u64, k: u64, q: f64) -> f64 {
    let nf = trials as f64;
    if k == 0 {
        return nf * (-q).ln_1p();
    }
    if k == trials {
        return nf * q.ln();
    }
    let kf = k as f64;
    let mf = (trials - k) as f64;
    let main = -HALF_LN_2PI + 0.5 * (nf / (kf * mf)).ln();
    let corr = stirling_err(nf) - stirling_err(kf) - stirling_err(mf);
    let dev = deviance_bd0(kf, nf * q) + deviance_bd0(mf, nf * (1.0 - q));
    main + corr - dev
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, CellCoord, CellExclusionNote, ColumnTable, ContingencyTable, DataSheet, GroupedTable,
        MultipleVariablesTable, NestedTable, PartsOfWholeTable, SubcolumnMode, SurvivalTable, TableArchetype, TransformAnalysisSpec, VariableRole,
        XyTable,
    };
    use curvebook_math::sum::SummaryMoments;
    use serde::Deserialize;

    use super::{
        AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, RowSampleConfig, SimulatedColumnSpec, SimulatedDistribution,
        SubjectRandomizationConfig, random_sample_rows, randomize_subjects_to_groups, sample_row_indices, simulate_distribution_columns,
    };
    use crate::error::TransformError;
    use crate::ops::{TransformContext, execute_analysis_transform};

    fn group_counts_from_schedule(sheet: &DataSheet) -> BTreeMap<String, usize> {
        let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
            panic!("expected MultipleVariables schedule");
        };
        let mut counts = BTreeMap::new();
        for cell in &mv.variables[4].cells {
            let label = cell.active_text().unwrap().to_owned();
            *counts.entry(label).or_insert(0) += 1;
        }
        counts
    }

    #[test]
    fn complete_balanced_weighted_randomization_is_reproducible_and_exact() {
        let cfg = SubjectRandomizationConfig::from_subject_count(
            "rand-1",
            "Study Arm Schedule",
            20,
            vec![
                GroupAllocationWeight::with_weight("HighDose", 2),
                GroupAllocationWeight::with_weight("LowDose", 1),
                GroupAllocationWeight::new("Vehicle"),
            ],
            AllocationScheme::CompleteBalanced,
            202_601,
        );

        let out_a = randomize_subjects_to_groups(&cfg).unwrap();
        let out_b = randomize_subjects_to_groups(&cfg).unwrap();
        assert_eq!(out_a, out_b);

        let mut cfg_other = cfg.clone();
        cfg_other.seed = 9999;
        let out_c = randomize_subjects_to_groups(&cfg_other).unwrap();
        assert_ne!(out_a.data_sheet, out_c.data_sheet);

        let counts = group_counts_from_schedule(&out_a.data_sheet);
        assert_eq!(counts.get("HighDose").copied(), Some(10));
        assert_eq!(counts.get("LowDose").copied(), Some(5));
        assert_eq!(counts.get("Vehicle").copied(), Some(5));

        let TableArchetype::MultipleVariables(mv) = &out_a.data_sheet.archetype else {
            panic!("expected MultipleVariables");
        };
        let titles: Vec<&str> = mv.variables.iter().map(|v| v.title.as_str()).collect();
        assert_eq!(titles, ["Subject_ID", "Allocation_Order", "Block_Index", "Block_Size", "Assigned_Group"]);
        assert_eq!(mv.variables[0].role, VariableRole::Categorical);
        assert_eq!(mv.variables[1].role, VariableRole::Continuous);
        assert_eq!(mv.variables[4].role, VariableRole::Categorical);
        assert_eq!(mv.variables[1].cells[0].active_number(), Some(1.0));
        assert_eq!(mv.variables[1].cells[19].active_number(), Some(20.0));
        assert_eq!(mv.variables[2].cells[0].active_number(), Some(1.0));
        assert_eq!(mv.variables[3].cells[0].active_number(), Some(20.0));

        let arrive: BTreeMap<String, String> = out_a.arrive_constants.into_iter().collect();
        assert_eq!(arrive.get("randomization.seed").map(String::as_str), Some("202601"));
        assert_eq!(arrive.get("randomization.method").map(String::as_str), Some("complete_balanced"));
        assert_eq!(arrive.get("randomization.allocation_ratio").map(String::as_str), Some("2:1:1"));
        assert_eq!(arrive.get("randomization.block_sizes").map(String::as_str), Some("20"));
        assert_eq!(arrive.get("randomization.total_subjects").map(String::as_str), Some("20"));
    }

    #[test]
    fn permuted_block_fixed_and_variable_sizes_balance_per_block_and_trim_cleanly() {
        let subjects: Vec<String> = (1..=14).map(|i| format!("M-{i:02}")).collect();
        let fixed_cfg = SubjectRandomizationConfig {
            sheet_id: String::from("rand-fixed"),
            title: String::from("Fixed Block"),
            subject_ids: subjects.clone(),
            groups: vec![GroupAllocationWeight::new("A"), GroupAllocationWeight::new("B")],
            scheme: AllocationScheme::PermutedBlock { block_sizes: vec![4] },
            seed: 77,
        };
        let fixed_out = randomize_subjects_to_groups(&fixed_cfg).unwrap();
        let TableArchetype::MultipleVariables(fmv) = &fixed_out.data_sheet.archetype else {
            panic!("expected MultipleVariables");
        };
        assert_eq!(fmv.row_count(), 14);
        for blk in 0..3 {
            let start = blk * 4;
            let mut a_count = 0;
            let mut b_count = 0;
            for r in start..start + 4 {
                assert_eq!(fmv.variables[2].cells[r].active_number(), Some((blk + 1) as f64));
                assert_eq!(fmv.variables[3].cells[r].active_number(), Some(4.0));
                match fmv.variables[4].cells[r].active_text() {
                    Some("A") => a_count += 1,
                    Some("B") => b_count += 1,
                    other => panic!("unexpected group {other:?}"),
                }
            }
            assert_eq!(a_count, 2);
            assert_eq!(b_count, 2);
        }
        assert_eq!(fmv.variables[2].cells[12].active_number(), Some(4.0));
        assert_eq!(fmv.variables[2].cells[13].active_number(), Some(4.0));

        let multi_cfg = SubjectRandomizationConfig {
            sheet_id: String::from("rand-var"),
            title: String::from("Variable Block"),
            subject_ids: (1..=32).map(|i| format!("S{i}")).collect(),
            groups: vec![GroupAllocationWeight::with_weight("Drug", 2), GroupAllocationWeight::with_weight("Placebo", 2)],
            scheme: AllocationScheme::PermutedBlock { block_sizes: vec![4, 8] },
            seed: 4242,
        };
        let multi_out = randomize_subjects_to_groups(&multi_cfg).unwrap();
        let TableArchetype::MultipleVariables(mmv) = &multi_out.data_sheet.archetype else {
            panic!("expected MultipleVariables");
        };
        let mut seen_sizes = BTreeSet::new();
        let mut row = 0;
        while row < mmv.row_count() {
            let b_idx = mmv.variables[2].cells[row].active_number().unwrap() as usize;
            let b_sz = mmv.variables[3].cells[row].active_number().unwrap() as usize;
            assert!(b_sz == 4 || b_sz == 8);
            seen_sizes.insert(b_sz);
            let end = (row + b_sz).min(mmv.row_count());
            let mut drug = 0;
            let mut placebo = 0;
            for r in row..end {
                assert_eq!(mmv.variables[2].cells[r].active_number().unwrap() as usize, b_idx);
                assert_eq!(mmv.variables[3].cells[r].active_number().unwrap() as usize, b_sz);
                match mmv.variables[4].cells[r].active_text() {
                    Some("Drug") => drug += 1,
                    Some("Placebo") => placebo += 1,
                    other => panic!("unexpected group {other:?}"),
                }
            }
            if end - row == b_sz {
                assert_eq!(drug, b_sz / 2);
                assert_eq!(placebo, b_sz / 2);
            }
            row = end;
        }
        assert_eq!(seen_sizes.len(), 2);

        let arrive: BTreeMap<String, String> = multi_out.arrive_constants.into_iter().collect();
        assert_eq!(arrive.get("randomization.method").map(String::as_str), Some("permuted_block"));
        assert_eq!(arrive.get("randomization.block_sizes").map(String::as_str), Some("4, 8"));
        assert_eq!(arrive.get("randomization.allocation_ratio").map(String::as_str), Some("2:2"));
    }

    #[test]
    fn group_weight_serde_defaults_omitted_weight_to_one_and_validates_errors() {
        let de = serde::de::value::MapDeserializer::<_, serde::de::value::Error>::new([("label", "Control")].into_iter());
        let parsed = GroupAllocationWeight::deserialize(de).unwrap();
        assert_eq!(parsed.label, "Control");
        assert_eq!(parsed.weight, 1);

        let empty_subj = SubjectRandomizationConfig {
            sheet_id: String::from("r"),
            title: String::from("R"),
            subject_ids: Vec::new(),
            groups: vec![GroupAllocationWeight::new("A")],
            scheme: AllocationScheme::CompleteBalanced,
            seed: 1,
        };
        assert!(matches!(randomize_subjects_to_groups(&empty_subj), Err(TransformError::InvalidSpec { .. })));

        let bad_block = SubjectRandomizationConfig::from_subject_count(
            "r",
            "R",
            10,
            vec![GroupAllocationWeight::with_weight("A", 2), GroupAllocationWeight::with_weight("B", 1)],
            AllocationScheme::PermutedBlock { block_sizes: vec![4] },
            1,
        );
        assert!(matches!(randomize_subjects_to_groups(&bad_block), Err(TransformError::InvalidSpec { .. })));

        let zero_weight = SubjectRandomizationConfig::from_subject_count(
            "r",
            "R",
            10,
            vec![GroupAllocationWeight::with_weight("A", 0), GroupAllocationWeight::new("B")],
            AllocationScheme::CompleteBalanced,
            1,
        );
        assert!(matches!(randomize_subjects_to_groups(&zero_weight), Err(TransformError::InvalidSpec { .. })));
    }

    #[test]
    fn uniform_and_stratified_row_sampling_preserves_metadata_and_strata() {
        // XY table with 20 rows and exclusion note
        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 20).unwrap();
        xy.x_title = String::from("Dose");
        xy.data_sets[0].title = String::from("Response");
        for r in 0..20 {
            xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
            xy.row_titles[r] = format!("R{r}");
            xy.data_sets[0].cells[r][0] = Cell::number((r * 10) as f64);
            xy.data_sets[0].cells[r][1] = Cell::number((r * 10 + 1) as f64);
        }
        let mut xy_sheet = DataSheet::new("xy-pop", "XY Pop", TableArchetype::Xy(xy)).unwrap();
        xy_sheet.exclusion_notes.push(CellExclusionNote {
            coord: CellCoord::new(5, 0, 0),
            reason_tag: String::from("outlier"),
            note: String::from("bubble"),
            timestamp_utc: String::from("2026-11-14T00:00:00Z"),
        });

        let cfg =
            RowSampleConfig { seed: 555, sample_size: 8, stratify_column: None, proportional_allocation: false, output_id: None, output_title: None };
        let idx1 = sample_row_indices(&xy_sheet, &cfg).unwrap();
        let idx2 = sample_row_indices(&xy_sheet, &cfg).unwrap();
        assert_eq!(idx1, idx2);
        assert_eq!(idx1.len(), 8);
        assert_eq!(idx1.iter().copied().collect::<BTreeSet<_>>().len(), 8);

        let sampled_xy = random_sample_rows(&xy_sheet, &cfg).unwrap();
        let TableArchetype::Xy(sxy) = &sampled_xy.archetype else {
            panic!("expected Xy");
        };
        assert_eq!(sxy.row_count(), 8);
        assert_eq!(sxy.y_subcolumn_mode, SubcolumnMode::RawReplicates(2));
        for (new_r, &orig_r) in idx1.iter().enumerate() {
            assert_eq!(sxy.x_value_at(new_r), Some((orig_r + 1) as f64));
            assert_eq!(sxy.row_titles[new_r], format!("R{orig_r}"));
            assert_eq!(sxy.data_sets[0].cells[new_r][1].active_number(), Some((orig_r * 10 + 1) as f64));
        }
        let expected_note_row = idx1.iter().position(|&r| r == 5);
        assert_eq!(sampled_xy.exclusion_notes.first().map(|n| n.coord.row), expected_note_row);

        // MultipleVariables stratified sampling (strata sizes 50, 30, 20)
        let mut mv = MultipleVariablesTable::new(2, 100).unwrap();
        mv.variables[0].title = String::from("Cohort");
        mv.variables[0].lock_role(VariableRole::Categorical);
        mv.variables[1].title = String::from("Biomarker");
        mv.variables[1].lock_role(VariableRole::Continuous);
        for r in 0..100 {
            let stratum = if r < 50 {
                "North"
            } else if r < 80 {
                "South"
            } else {
                "West"
            };
            mv.variables[0].cells[r] = Cell::text(stratum);
            mv.variables[1].cells[r] = Cell::number(r as f64);
        }
        let mv_sheet = DataSheet::new("mv-pop", "MV Pop", TableArchetype::MultipleVariables(mv)).unwrap();

        // Proportional: 20 rows -> 10 North, 6 South, 4 West
        let prop_spec = AnalysisSpec::new(
            "an-sample-prop",
            "mv-pop",
            AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
                seed: 1234,
                sample_size: 20,
                stratify_column: Some(0),
                proportional_allocation: true,
            }),
        )
        .unwrap();
        let prop_res = execute_analysis_transform(&mv_sheet, &prop_spec, &TransformContext::default()).unwrap();
        assert_eq!(prop_res.lookup_scalar("sample_size", None), Some(20.0));
        assert_eq!(prop_res.lookup_scalar("population_size", None), Some(100.0));
        assert_eq!(prop_res.lookup_scalar("seed", None), Some(1234.0));
        let prop_data = prop_res.analyzable_data.as_ref().unwrap();
        let TableArchetype::MultipleVariables(pmv) = &prop_data.archetype else {
            panic!("expected MultipleVariables");
        };
        let mut prop_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &pmv.variables[0].cells {
            *prop_counts.entry(c.active_text().unwrap()).or_insert(0) += 1;
        }
        assert_eq!(prop_counts.get("North").copied(), Some(10));
        assert_eq!(prop_counts.get("South").copied(), Some(6));
        assert_eq!(prop_counts.get("West").copied(), Some(4));

        // Equal allocation across strata: 18 rows -> 6 North, 6 South, 6 West
        let equal_cfg = RowSampleConfig {
            seed: 1234,
            sample_size: 18,
            stratify_column: Some(0),
            proportional_allocation: false,
            output_id: None,
            output_title: None,
        };
        let eq_sheet = random_sample_rows(&mv_sheet, &equal_cfg).unwrap();
        let TableArchetype::MultipleVariables(emv) = &eq_sheet.archetype else {
            panic!("expected MultipleVariables");
        };
        let mut eq_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &emv.variables[0].cells {
            *eq_counts.entry(c.active_text().unwrap()).or_insert(0) += 1;
        }
        assert_eq!(eq_counts.get("North").copied(), Some(6));
        assert_eq!(eq_counts.get("South").copied(), Some(6));
        assert_eq!(eq_counts.get("West").copied(), Some(6));
    }

    #[test]
    fn row_sampling_supports_remaining_archetypes_and_validates_inputs() {
        let sample_four = |sheet: &DataSheet, seed: u64| -> usize {
            random_sample_rows(
                sheet,
                &RowSampleConfig { seed, sample_size: 4, stratify_column: None, proportional_allocation: false, output_id: None, output_title: None },
            )
            .unwrap()
            .archetype
            .row_count()
        };

        let col_sheet = DataSheet::new(
            "col-pop",
            "Col",
            TableArchetype::Column(ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 2, 10).unwrap()),
        )
        .unwrap();
        assert_eq!(sample_four(&col_sheet, 1), 4);

        let grp_sheet =
            DataSheet::new("grp-pop", "Grp", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 12).unwrap())).unwrap();
        assert_eq!(sample_four(&grp_sheet, 2), 4);

        let mut ctg = ContingencyTable::new(2, 8).unwrap();
        for r in 0..8 {
            ctg.row_titles[r] = format!("Cat{r}");
            ctg.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
            ctg.data_sets[1].cells[r][0] = Cell::number((r + 2) as f64);
        }
        let ctg_sheet = DataSheet::new("ctg-pop", "Ctg", TableArchetype::Contingency(ctg)).unwrap();
        assert_eq!(sample_four(&ctg_sheet, 3), 4);

        let pow_sheet = DataSheet::new("pow-pop", "PoW", TableArchetype::PartsOfWhole(PartsOfWholeTable::new(1, 9).unwrap())).unwrap();
        assert_eq!(sample_four(&pow_sheet, 4), 4);

        let srv_sheet = DataSheet::new("srv-pop", "Srv", TableArchetype::Survival(SurvivalTable::new(2, 10).unwrap())).unwrap();
        assert_eq!(sample_four(&srv_sheet, 5), 4);

        let nst_sheet = DataSheet::new("nst-pop", "Nst", TableArchetype::Nested(NestedTable::new(2, 3, 11).unwrap())).unwrap();
        assert_eq!(sample_four(&nst_sheet, 6), 4);

        let too_big =
            RowSampleConfig { seed: 1, sample_size: 11, stratify_column: None, proportional_allocation: false, output_id: None, output_title: None };
        assert!(matches!(random_sample_rows(&col_sheet, &too_big), Err(TransformError::InvalidSpec { .. })));

        let zero_sz =
            RowSampleConfig { seed: 1, sample_size: 0, stratify_column: None, proportional_allocation: false, output_id: None, output_title: None };
        assert!(matches!(random_sample_rows(&col_sheet, &zero_sz), Err(TransformError::InvalidSpec { .. })));

        let bad_col =
            RowSampleConfig { seed: 1, sample_size: 4, stratify_column: Some(9), proportional_allocation: true, output_id: None, output_title: None };
        assert!(matches!(random_sample_rows(&col_sheet, &bad_col), Err(TransformError::InvalidSpec { .. })));
    }

    #[test]
    fn proportional_apportionment_clamps_small_stratum_to_one_and_equal_handles_saturation() {
        let mut mv = MultipleVariablesTable::new(1, 100).unwrap();
        mv.variables[0].title = String::from("Group");
        for r in 0..100 {
            let label = if r < 90 {
                "Major"
            } else if r < 99 {
                "Minor"
            } else {
                "Tiny"
            };
            mv.variables[0].cells[r] = Cell::text(label);
        }
        let sheet = DataSheet::new("stratum-edge", "Stratum Edge", TableArchetype::MultipleVariables(mv)).unwrap();

        let prop_5 = random_sample_rows(
            &sheet,
            &RowSampleConfig {
                seed: 11,
                sample_size: 5,
                stratify_column: Some(0),
                proportional_allocation: true,
                output_id: None,
                output_title: None,
            },
        )
        .unwrap();
        let TableArchetype::MultipleVariables(pmv) = &prop_5.archetype else {
            panic!("expected MultipleVariables");
        };
        let mut counts = BTreeMap::new();
        for c in &pmv.variables[0].cells {
            *counts.entry(c.active_text().unwrap()).or_insert(0_usize) += 1;
        }
        assert_eq!(counts.get("Major").copied(), Some(3));
        assert_eq!(counts.get("Minor").copied(), Some(1));
        assert_eq!(counts.get("Tiny").copied(), Some(1));

        // Equal allocation of 15 across [90, 9, 1] -> Tiny caps at 1, leaving 14 split 7 & 7
        let eq_15 = random_sample_rows(
            &sheet,
            &RowSampleConfig {
                seed: 12,
                sample_size: 15,
                stratify_column: Some(0),
                proportional_allocation: false,
                output_id: None,
                output_title: None,
            },
        )
        .unwrap();
        let TableArchetype::MultipleVariables(emv) = &eq_15.archetype else {
            panic!("expected MultipleVariables");
        };
        let mut eq_counts = BTreeMap::new();
        for c in &emv.variables[0].cells {
            *eq_counts.entry(c.active_text().unwrap()).or_insert(0_usize) += 1;
        }
        assert_eq!(eq_counts.get("Tiny").copied(), Some(1));
        assert_eq!(eq_counts.get("Minor").copied(), Some(7));
        assert_eq!(eq_counts.get("Major").copied(), Some(7));
    }

    #[test]
    fn continuous_simulated_distributions_match_moments_and_mv_mode() {
        let n = 5_000_usize;
        let cfg = DistributionSimulationConfig {
            sheet_id: String::from("sim-cont"),
            title: String::from("Continuous"),
            n_rows: n,
            seed: 2026,
            columns: vec![
                SimulatedColumnSpec { name: String::from("Unif"), distribution: SimulatedDistribution::Uniform { min: -2.0, max: 6.0 } },
                SimulatedColumnSpec { name: String::from("Gauss"), distribution: SimulatedDistribution::Gaussian { mean: 10.0, sd: 2.5 } },
                SimulatedColumnSpec { name: String::from("LogN"), distribution: SimulatedDistribution::LogNormal { mean_log: 0.5, sd_log: 0.4 } },
                SimulatedColumnSpec { name: String::from("Exp"), distribution: SimulatedDistribution::Exponential { rate: 0.5 } },
            ],
            use_multiple_variables: false,
        };

        let sheet_a = simulate_distribution_columns(&cfg).unwrap();
        let sheet_b = simulate_distribution_columns(&cfg).unwrap();
        assert_eq!(sheet_a, sheet_b);

        let TableArchetype::Column(col_tbl) = &sheet_a.archetype else {
            panic!("expected Column table");
        };
        assert_eq!(col_tbl.data_sets.len(), 4);
        assert_eq!(col_tbl.row_count(), n);

        // Uniform(-2, 6): mean = 2.0, var = 64/12
        let u = col_tbl.active_column_values(0);
        assert!(u.iter().all(|&x| (-2.0..6.0).contains(&x)));
        let mu = SummaryMoments::from_slice(&u).unwrap();
        assert!((mu.mean - 2.0).abs() < 0.16, "uniform mean={}", mu.mean);
        assert!((mu.sample_variance().unwrap() - (64.0 / 12.0)).abs() < 0.35);

        // Gaussian(10, 2.5): mean = 10.0, var = 6.25
        let g = col_tbl.active_column_values(1);
        let mg = SummaryMoments::from_slice(&g).unwrap();
        assert!((mg.mean - 10.0).abs() < 0.18, "gauss mean={}", mg.mean);
        assert!((mg.sample_variance().unwrap() - 6.25).abs() < 0.63);

        // LogNormal(0.5, 0.4): mean = exp(0.58), var = (exp(0.16)-1)*exp(1.16)
        let ln_vals = col_tbl.active_column_values(2);
        assert!(ln_vals.iter().all(|&x| x > 0.0));
        let mln = SummaryMoments::from_slice(&ln_vals).unwrap();
        let exp_mean = (0.58_f64).exp();
        let exp_var = (0.16_f64.exp() - 1.0) * (1.16_f64).exp();
        assert!((mln.mean - exp_mean).abs() < 0.06, "logn mean={} vs {exp_mean}", mln.mean);
        assert!((mln.sample_variance().unwrap() - exp_var).abs() < 0.10);

        // Exponential(0.5): mean = 2.0, var = 4.0
        let ex = col_tbl.active_column_values(3);
        assert!(ex.iter().all(|&x| x > 0.0));
        let mex = SummaryMoments::from_slice(&ex).unwrap();
        assert!((mex.mean - 2.0).abs() < 0.15, "exp mean={}", mex.mean);
        assert!((mex.sample_variance().unwrap() - 4.0).abs() < 0.80);

        // MultipleVariables output mode
        let mut mv_cfg = cfg;
        mv_cfg.use_multiple_variables = true;
        mv_cfg.n_rows = 12;
        let mv_sheet = simulate_distribution_columns(&mv_cfg).unwrap();
        let TableArchetype::MultipleVariables(mv) = &mv_sheet.archetype else {
            panic!("expected MultipleVariables");
        };
        assert_eq!(mv.variables.len(), 4);
        assert_eq!(mv.row_count(), 12);
        assert_eq!(mv.variables[0].role, VariableRole::Continuous);
    }

    #[test]
    fn discrete_simulated_distributions_match_moments_and_bounds() {
        let n = 5_000_usize;
        let cfg = DistributionSimulationConfig {
            sheet_id: String::from("sim-disc"),
            title: String::from("Discrete"),
            n_rows: n,
            seed: 271_828,
            columns: vec![
                SimulatedColumnSpec { name: String::from("BinomSmall"), distribution: SimulatedDistribution::Binomial { trials: 20, p: 0.3 } },
                SimulatedColumnSpec { name: String::from("BinomLarge"), distribution: SimulatedDistribution::Binomial { trials: 500, p: 0.72 } },
                SimulatedColumnSpec { name: String::from("BinomGeo"), distribution: SimulatedDistribution::Binomial { trials: 10_000, p: 0.0005 } },
                SimulatedColumnSpec { name: String::from("PoisSmall"), distribution: SimulatedDistribution::Poisson { lambda: 4.5 } },
                SimulatedColumnSpec { name: String::from("PoisLarge"), distribution: SimulatedDistribution::Poisson { lambda: 1_000.0 } },
                SimulatedColumnSpec { name: String::from("PoisZero"), distribution: SimulatedDistribution::Poisson { lambda: 0.0 } },
                SimulatedColumnSpec { name: String::from("BinomZero"), distribution: SimulatedDistribution::Binomial { trials: 50, p: 0.0 } },
                SimulatedColumnSpec { name: String::from("BinomOne"), distribution: SimulatedDistribution::Binomial { trials: 50, p: 1.0 } },
            ],
            use_multiple_variables: false,
        };

        let sheet = simulate_distribution_columns(&cfg).unwrap();
        let TableArchetype::Column(col_tbl) = &sheet.archetype else {
            panic!("expected Column table");
        };

        let bs = col_tbl.active_column_values(0);
        assert!(bs.iter().all(|&x| (0.0..=20.0).contains(&x) && x.fract() == 0.0));
        let mbs = SummaryMoments::from_slice(&bs).unwrap();
        assert!((mbs.mean - 6.0).abs() < 0.15, "binom small mean={}", mbs.mean);
        assert!((mbs.sample_variance().unwrap() - 4.2).abs() < 0.42);

        let bl = col_tbl.active_column_values(1);
        assert!(bl.iter().all(|&x| (0.0..=500.0).contains(&x) && x.fract() == 0.0));
        let mbl = SummaryMoments::from_slice(&bl).unwrap();
        assert!((mbl.mean - 360.0).abs() < 0.71, "binom large mean={}", mbl.mean);
        assert!((mbl.sample_variance().unwrap() - 100.8).abs() < 10.1);

        let bg = col_tbl.active_column_values(2);
        assert!(bg.iter().all(|&x| (0.0..=10_000.0).contains(&x) && x.fract() == 0.0));
        let mbg = SummaryMoments::from_slice(&bg).unwrap();
        assert!((mbg.mean - 5.0).abs() < 0.16, "binom geo mean={}", mbg.mean);
        assert!((mbg.sample_variance().unwrap() - 4.9975).abs() < 0.55);

        let ps = col_tbl.active_column_values(3);
        assert!(ps.iter().all(|&x| x >= 0.0 && x.fract() == 0.0));
        let mps = SummaryMoments::from_slice(&ps).unwrap();
        assert!((mps.mean - 4.5).abs() < 0.15, "pois small mean={}", mps.mean);
        assert!((mps.sample_variance().unwrap() - 4.5).abs() < 0.48);

        let pl = col_tbl.active_column_values(4);
        assert!(pl.iter().all(|&x| x >= 0.0 && x.fract() == 0.0));
        let mpl = SummaryMoments::from_slice(&pl).unwrap();
        assert!((mpl.mean - 1000.0).abs() < 2.24, "pois large mean={}", mpl.mean);
        assert!((mpl.sample_variance().unwrap() - 1000.0).abs() < 100.0);

        assert!(col_tbl.active_column_values(5).iter().all(|&x| x == 0.0));
        assert!(col_tbl.active_column_values(6).iter().all(|&x| x == 0.0));
        assert!(col_tbl.active_column_values(7).iter().all(|&x| x == 50.0));
    }

    #[test]
    fn simulated_distribution_parameter_validation_errors() {
        let base = |dist: SimulatedDistribution| DistributionSimulationConfig {
            sheet_id: String::from("bad"),
            title: String::from("Bad"),
            n_rows: 10,
            seed: 1,
            columns: vec![SimulatedColumnSpec { name: String::from("X"), distribution: dist }],
            use_multiple_variables: false,
        };

        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::Uniform { min: 5.0, max: 5.0 })),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::Uniform { min: 6.0, max: 2.0 })),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::Gaussian { mean: 0.0, sd: -1.0 })),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::LogNormal { mean_log: 0.0, sd_log: -0.1 })),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::Binomial { trials: 10, p: 1.2 })),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::Binomial { trials: 10, p: -0.01 })),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::Poisson { lambda: -0.5 })),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::Exponential { rate: 0.0 })),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            simulate_distribution_columns(&base(SimulatedDistribution::Exponential { rate: -2.0 })),
            Err(TransformError::InvalidSpec { .. })
        ));
    }

    #[test]
    fn aud04_14_random_sample_snapshots_calculated_mv_variables() {
        let mut mv = MultipleVariablesTable::new(2, 6).unwrap();
        mv.variables[0].title = String::from("X");
        mv.variables[1].title = String::from("Z");
        for r in 0..6 {
            mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
        }
        mv.variables[1].set_calculated_formula("=([X] - mean([X])) / sd([X])");
        crate::formula::evaluate_calculated_variables(&mut mv, None).unwrap();
        let orig_z: Vec<f64> = mv.variables[1].cells.iter().map(|c| c.active_number().unwrap()).collect();
        let sheet = DataSheet::new("mv-samp", "MV Samp", TableArchetype::MultipleVariables(mv)).unwrap();

        let cfg =
            RowSampleConfig { seed: 42, sample_size: 3, stratify_column: None, proportional_allocation: false, output_id: None, output_title: None };
        let idxs = sample_row_indices(&sheet, &cfg).unwrap();
        let sampled = random_sample_rows(&sheet, &cfg).unwrap();
        let TableArchetype::MultipleVariables(mut out_mv) = sampled.archetype else {
            panic!("expected MV");
        };
        assert_eq!(out_mv.variables[1].source, curvebook_doc::VariableSource::Entered);
        assert_eq!(out_mv.variables[1].role, VariableRole::Continuous);
        // Recalculating calculated variables must be a no-op and preserve original population Z-scores.
        assert!(crate::formula::evaluate_calculated_variables(&mut out_mv, None).unwrap().is_empty());
        for (new_r, &orig_r) in idxs.iter().enumerate() {
            assert!((out_mv.variables[1].cells[new_r].active_number().unwrap() - orig_z[orig_r]).abs() < 1e-12);
        }
    }

    #[test]
    fn aud04_24_try_from_subject_count_rejects_excess_rows_before_alloc() {
        let err = SubjectRandomizationConfig::try_from_subject_count(
            "too-many",
            "Too Many",
            curvebook_doc::MAX_ROWS + 1,
            vec![GroupAllocationWeight::new("A"), GroupAllocationWeight::new("B")],
            AllocationScheme::CompleteBalanced,
            1,
        )
        .unwrap_err();
        assert!(matches!(err, TransformError::Doc(curvebook_doc::DocError::TooManyRows { .. })), "unexpected err: {err:?}");

        let ok = SubjectRandomizationConfig::try_from_subject_count(
            "ok-rand",
            "OK",
            4,
            vec![GroupAllocationWeight::new("A"), GroupAllocationWeight::new("B")],
            AllocationScheme::CompleteBalanced,
            7,
        )
        .unwrap();
        assert_eq!(ok.subject_ids, vec!["1", "2", "3", "4"]);
    }

    #[test]
    fn aud04_45_random_sample_validates_source_and_output_and_guards_generated_sequence() {
        // Ragged source rejected by sample_row_indices and random_sample_rows.
        let mut ragged = MultipleVariablesTable::new(2, 4).unwrap();
        ragged.variables[1].cells.truncate(2);
        let ragged_sheet = DataSheet {
            id: curvebook_doc::SheetId::new("rag"),
            title: String::from("Ragged"),
            archetype: TableArchetype::MultipleVariables(ragged),
            frozen: false,
            info_sheet_id: None,
            linked_regions: Vec::new(),
            floating_notes: Vec::new(),
            color_tag: None,
            exclusion_notes: Vec::new(),
        };
        let cfg =
            RowSampleConfig { seed: 1, sample_size: 2, stratify_column: None, proportional_allocation: false, output_id: None, output_title: None };
        assert!(matches!(sample_row_indices(&ragged_sheet, &cfg), Err(TransformError::Doc(_))));
        assert!(matches!(random_sample_rows(&ragged_sheet, &cfg), Err(TransformError::Doc(_))));

        // Long source title auto-suffix stays within MAX_TITLE_BYTES; invalid explicit output_id rejected.
        let mut mv = MultipleVariablesTable::new(1, 3).unwrap();
        mv.variables[0].cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)];
        let long_sheet = DataSheet::new("samp-long", "S".repeat(curvebook_doc::MAX_TITLE_BYTES), TableArchetype::MultipleVariables(mv)).unwrap();
        let sampled = random_sample_rows(&long_sheet, &cfg).unwrap();
        assert!(sampled.validate().is_ok());
        assert!(sampled.title.len() <= curvebook_doc::MAX_TITLE_BYTES);

        let bad_id_cfg = RowSampleConfig { output_id: Some(String::from("bad\0id")), ..cfg };
        assert!(matches!(random_sample_rows(&long_sheet, &bad_id_cfg), Err(TransformError::Doc(curvebook_doc::DocError::InvalidIdentifier { .. }))));

        // GeneratedSequence XY near MAX_SHEET_CELLS rejects materialization overflow cleanly.
        let seq_xy = XyTable::new_with_x_mode(
            curvebook_doc::XColumnMode::GeneratedSequence { start: 0.0, step: 1.0, progression: curvebook_doc::SequenceProgression::Arithmetic },
            SubcolumnMode::RawReplicates(300),
            1,
            100_000,
        )
        .unwrap();
        let seq_sheet = DataSheet::new("seq-cap", "Seq Cap", TableArchetype::Xy(seq_xy)).unwrap();
        let full_cfg = RowSampleConfig {
            seed: 1,
            sample_size: 100_000,
            stratify_column: None,
            proportional_allocation: false,
            output_id: None,
            output_title: None,
        };
        assert!(matches!(random_sample_rows(&seq_sheet, &full_cfg), Err(TransformError::Doc(curvebook_doc::DocError::TooManyCells { .. }))));
    }

    #[test]
    fn aud04_49_complete_balanced_unbiased_d1_marginal_counts_and_arrive_metadata() {
        // Reference: Durstenfeld (1964) uniform permutation over tied Hamilton remainders.
        // For N = 5, weights = [1, 1], each seed assigns either (3, 2) or (2, 3) with exact probability 0.5.
        // Across K = 10_000 seeds, count(arm0 == 3) ~ Binomial(10_000, 0.5): mean = 5_000, sd = 50;
        // z = 6.0 two-sided tolerance [4_700, 5_300] (tail probability < 2e-9).
        let groups = vec![GroupAllocationWeight::new("Control"), GroupAllocationWeight::new("Drug")];
        let mut arm0_got_surplus = 0_usize;
        let mut total_arm0 = 0_usize;
        let mut total_arm1 = 0_usize;

        for seed in 1..=10_000_u64 {
            let cfg = SubjectRandomizationConfig::try_from_subject_count(
                "unb-5",
                "Unbiased 5",
                5,
                groups.clone(),
                AllocationScheme::CompleteBalancedUnbiased,
                seed,
            )
            .unwrap();
            let out = randomize_subjects_to_groups(&cfg).unwrap();
            if seed == 1 {
                // Reproducibility & ARRIVE metadata check.
                assert_eq!(out, randomize_subjects_to_groups(&cfg).unwrap());
                let arrive: BTreeMap<String, String> = out.arrive_constants.iter().cloned().collect();
                assert_eq!(arrive.get("randomization.method").map(String::as_str), Some("complete_balanced_unbiased"));
                assert_eq!(arrive.get("randomization.block_sizes").map(String::as_str), Some("5"));
            }
            let counts = group_counts_from_schedule(&out.data_sheet);
            let c0 = counts.get("Control").copied().unwrap_or(0);
            let c1 = counts.get("Drug").copied().unwrap_or(0);
            assert!((c0 == 3 && c1 == 2) || (c0 == 2 && c1 == 3), "unexpected split ({c0}, {c1})");
            if c0 == 3 {
                arm0_got_surplus += 1;
            }
            total_arm0 += c0;
            total_arm1 += c1;
        }
        assert!(
            (4_700..=5_300).contains(&arm0_got_surplus),
            "CompleteBalancedUnbiased arm0 surplus count {arm0_got_surplus} outside z=6.0 bounds [4700, 5300]"
        );
        assert_eq!(total_arm0 + total_arm1, 50_000);
        assert!((24_700..=25_300).contains(&total_arm0), "total arm0 allocations {total_arm0} outside [24700, 25300]");

        // Confirm CompleteBalanced retains deterministic ascending-index Hamilton tie-breaking (arm 0 always 3).
        for seed in 1..=10_u64 {
            let cfg =
                SubjectRandomizationConfig::try_from_subject_count("det-5", "Det 5", 5, groups.clone(), AllocationScheme::CompleteBalanced, seed)
                    .unwrap();
            let counts = group_counts_from_schedule(&randomize_subjects_to_groups(&cfg).unwrap().data_sheet);
            assert_eq!((counts.get("Control").copied(), counts.get("Drug").copied()), (Some(3), Some(2)));
        }
    }
}
