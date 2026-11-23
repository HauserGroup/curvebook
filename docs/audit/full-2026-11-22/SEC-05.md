# SEC-05 Audit Report — Statistics Core (`crates/stats`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-05` (`crates/stats/Cargo.toml` + 11 source files in `crates/stats/src/`, `16,079` total lines; excludes `multivariate.rs`, `regression.rs`, `survival.rs`, `anova/**` belonging to `SEC-06`)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-sci`
- **Mode:** Report-only static audit + read-only verification (`cargo test --locked --offline -p curvebook-stats`: `292` passed — `18` unit tests + `273` integration tests across `33` binaries + `1` doc-test, `0` failed, `0` ignored)
- **Summary Counts:** `S1: 4` · `S2: 7` · `S3: 1` · `S4: 1` (Total: `13`)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Checked | Findings / Status |
|---|---:|---|---|---|
| `crates/stats/Cargo.toml` | 20 | `1..=20` | `[lints] workspace = true`; permissive dependencies strictly limited to `curvebook-math`, `curvebook-doc`, `serde` (plus `curvebook-testkit`, `proptest` dev-deps); zero `chrono`, zero network/telemetry, zero GPL crates | `Clean` — workspace lints and minimal pure-Rust deps |
| `crates/stats/src/lib.rs` | 109 | `1..=109` | `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`, `AnalysisKind` dispatch in `run_analysis`, zero `std::fs`/OS coupling (`wasm32`-clean), passing doc-test | `Clean` — strict lints, pure dispatch, no unsafe/native I/O |
| `crates/stats/src/error.rs` | 127 | `1..=127` | Structured `StatsError` enum (`EmptyInput`, `InsufficientSampleSize`, `InvalidParameter`, `DimensionMismatch`, `RequiresRawReplicates`, `IncompatibleTable`, `IncompatibleAnalysisSpec`, `ZeroVariance`, `MissingColumn`, `NotConverged`), `Display`/`Error` impls, `From<{MathError, DocError, SummaryPointError}>` | `Clean` — typed error propagation without panicking paths |
| `crates/stats/src/roc.rs` | 460 | `1..=460` | Empirical ROC staircase, trapezoidal AUC, DeLong (1988) `V_{10}`/`V_{01}` SE & paired covariance (`n >= 2` guard for DeLong and `compare_roc_delong_paired`), Hanley–McNeil SE + Wilson score fallback (`#38` closed), unpaired comparison, `run_roc_analysis` | `FA-SEC05-11` |
| `crates/stats/src/cld.rs` | 619 | `1..=619` | Piepho (2004) insert-and-absorb + reverse sweep, base-26 token generation, `compute_cld_from_matrix`, `compute_cld_from_details`, `compute_cld_from_records` / `compute_cld_from_result_sheet` (skipping BFS on `Survival`), symmetry/diagonal/conflict validation | `Clean` — validated matrix symmetry, missing-pair guards, deterministic tie breaks |
| `crates/stats/src/pvalue_stack.rs` | 634 | `1..=634` | `validate_pvalue_threshold` in `(0, 1)`, `validate_raw_pvalues` `[0, 1]` finite check (explicitly rejecting `NaN`/`±Inf` strings and numeric out-of-range), 7 multiplicity dispatches, stable competition ranks, ignored-text warnings, `sanitize_result_sheet_floats` | `FA-SEC05-12` |
| `crates/stats/src/common.rs` | 721 | `1..=721` | `validate_finite_slice`, Hyndman & Fan Type 6 / Type 7 quantiles, `rank_with_ties`, `num_cell`/`ci_cell`/`finite_ci_cell`/`p_cell`/`p_cell_at_alpha`, `resolve_selected_indices` vs `try_resolve_selected_indices`, `extract_numeric_columns` (`Categorical`/`Label` default filtering), `extract_paired_values_with_report`, `sanitize_result_sheet_floats`, Johnson–Mizoguchi monotone matrix selection | `Clean` — overflow-guarded `select_kth_pairwise_sum`, finite sanitation (`resolve_selected_indices` callers flagged in `FA-SEC05-02`) |
| `crates/stats/src/multiplicity.rs` | 1211 | `1..=1211` | Bonferroni, Šídák, Holm–Bonferroni, Holm–Šídák, Benjamini–Hochberg, Benjamini–Yekutieli, Two-stage BKY (2006) (`#40` fixed), `try_adjust_p_values` vs `adjust_p_values`, `linear_trend_test`, pooled/Tukey–Kramer/Newman–Keuls/Dunnett/Games–Howell/Tamhane T2/Dunnett T3/Dunn rank-sum, `build_multiplicity_tab` | `FA-SEC05-10`, `FA-SEC05-12`, `FA-SEC05-13` |
| `crates/stats/src/column.rs` | 2343 | `1..=2343` | Descriptive moments/quantiles/Type-7 custom percentiles (`#68` fixed), geometric/harmonic/quadratic means, exact binomial median CI, `summarize_pre_summarized_points`, 3 `SubcolumnAggregation` modes, D'Agostino–Pearson / Shapiro–Wilk / Anderson–Darling (`#100` fixed) / Lilliefors + Q–Q, Grubbs ESD & column ROUT, `one_sample_t_test` (`#8` fixed for `n=2`), `one_sample_wilcoxon`, `one_sample_ks_normal`, trapezoidal AUC, row descriptive stats, `run_descriptive_analysis`, `run_outlier_analysis` | `FA-SEC05-12`, `FA-SEC05-13` |
| `crates/stats/src/power.rs` | 2757 | `1..=2757` | 8 families (`UnpairedT`/Welch, `PairedT`, `OneSampleT`, `OneWayAnova`, `TwoProportions`, `Correlation` incl. Hotelling `ExactTTransform`, `ChiSquare`, `SimpleLinearRegression`), 3 solve targets, Cohen's `h` `|h| <= pi` cap (`#41` fixed), checked `usize` ceilings, power curve & sensitivity matrix, `run_power_analysis` | `FA-SEC05-08` |
| `crates/stats/src/categorical.rs` | 3526 | `1..=3526` | Wilson / Clopper–Pearson / Agresti–Coull CIs, `exact_binomial_test` (Sterne + doubled tail), Fisher `2x2` & `RxC` deterministic MC, Barnard/Boschloo/CSM unconditional tests, zero-margin reduction (`#17` fixed), Pearson chi2 / Yates (`#22` fixed) / G², Cochran–Armitage, McNemar + OR CI, Baptista–Pike OR CI, Koopman RR CI, Newcombe RD, Altman NNT, Cohen's & Fleiss' kappa (`#90` fixed), `analyze_2x2_ratios` (`#18` fixed), `run_contingency_analysis`, `PartsOfWhole` GOF (`#88`, `#89` fixed) | `FA-SEC05-01`, `FA-SEC05-02`, `FA-SEC05-04`, `FA-SEC05-12`, `FA-SEC05-13` |
| `crates/stats/src/two_group.rs` | 3552 | `1..=3552` | `f_test_variances`, `levene_variance_test`, Student/Welch from summary & raw replicates, `lognormal_unpaired_t_test` (`#67` fixed), Glass's Delta (`#69` fixed), `paired_t_test`/`ratio_paired_t_test` (`#8` fixed), `mann_whitney_u` (exact + ties + Hodges–Lehmann `#21` fixed), `wilcoxon_matched_pairs` (classic & Pratt), `paired_sign_test`, `ks_two_sample`, `bland_altman` (`#4`, `#104`), `multiple_row_t_tests` (`#7`), MV grouping & response resolution, sheet builders | `FA-SEC05-03`, `FA-SEC05-05`, `FA-SEC05-06`, `FA-SEC05-07`, `FA-SEC05-09`, `FA-SEC05-11` |

---

## Findings (`S1` -> `S4`)

### FA-SEC05-01 · S1 · Contingency `PartsOfWhole` runner executes goodness-of-fit on kappa, Fisher, Barnard, trend, and McNemar requests (**REGRESSION / surviving `R2-S07-07`**)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/categorical.rs:2608-2615, 3053-3150`
- **Category:** `wrong-result`
- **Defect:** `run_contingency_analysis_inner` handles `ContingencyMethod::ExactBinomial` first and then unconditionally routes every other `ContingencyMethod` on `TableArchetype::PartsOfWhole` into `run_parts_of_whole_gof` without checking method compatibility.
- **Failure scenario:** Requesting `CohenKappa`, `FleissKappa`, `FisherExact`, `BarnardExact`, `CochranArmitageTrend`, or `McNemarPaired` against a `PartsOfWhole` sheet succeeds with `ResultStatus::UpToDate` and silently emits `"Chi-square goodness of fit"` (`parts-gof`, Pearson $\chi^2$ and $G^2$) instead of returning `StatsError::IncompatibleTable` / `IncompatibleAnalysisSpec`. Even `LikelihoodRatioG2` puts `"Goodness-of-fit chi-square"` first and `PearsonChiSquare { yates_continuity: true }` silently ignores `yates_continuity`.
- **Evidence:**
```rust
// crates/stats/src/categorical.rs:2608-2614
    if matches!(cont_spec.method, ContingencyMethod::ExactBinomial) {
        return run_exact_binomial_analysis(sheet, spec, cont_spec, out, confidence, p_fmt);
    }

    if let TableArchetype::PartsOfWhole(pow) = &sheet.archetype {
        return run_parts_of_whole_gof(pow, spec, cont_spec, &mut out, p_fmt);
    }
```
  Recorded as still open (`R2-S07-07`) in `docs/audit/local-code-audit-2026-11-18.md:665`.
- **Suggested fix:** Allow only `ExactBinomial`, `PearsonChiSquare { yates_continuity: false }`, and `LikelihoodRatioG2` on `PartsOfWhole` sheets (reflecting `LikelihoodRatioG2` in `"parts-gof"`), and return `StatsError::IncompatibleTable` / `IncompatibleAnalysisSpec` for all other `ContingencyMethod` variants.
- **Related:** `docs/audit/local-code-audit-2026-11-18.md:665` (`R2-S07-07`).

---

### FA-SEC05-02 · S1 · Contingency & exact binomial extractors use `resolve_selected_indices`, silently dropping invalid indices and duplicating `[0, 0]`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/categorical.rs:2620-2631, 2905-2926`
- **Category:** `wrong-result`
- **Defect:** `run_contingency_analysis_inner` (`line 2620`) and `extract_exact_binomial_counts` on `Contingency` (`line 2905`) use the silently-filtering `common::resolve_selected_indices` rather than `common::try_resolve_selected_indices`, and `extract_exact_binomial_counts` on `Column` (`line 2923`) takes `selected_data_sets.first()` while ignoring extra selected columns.
- **Failure scenario:** Passing `selected_data_sets: vec![0, 0]` to `run_contingency_analysis` on a `Contingency` table duplicates column `0` into an artificial `R x 2` table of identical columns and silently returns `chi2 = 0.0, p = 1.0` instead of an error; passing `vec![0, 1, 99]` silently drops out-of-range index `99` instead of returning `MissingColumn { index: 99 }`. Passing `vec![0, 1]` to `ExactBinomial` on a `Column` table silently analyzes only column `0` while discarding column `1`.
- **Evidence:**
```rust
// crates/stats/src/categorical.rs:2619-2623
    table.validate_counts()?;
    let selected_cols = resolve_selected_indices(&spec.selected_data_sets, table.data_sets.len());
    if selected_cols.is_empty() {
        return Err(StatsError::EmptyInput);
    }

// crates/stats/src/categorical.rs:2903-2913, 2923-2926
        TableArchetype::Contingency(table) => {
            table.validate_counts()?;
            let selected_cols = resolve_selected_indices(&spec.selected_data_sets, table.data_sets.len());
...
        TableArchetype::Column(col_table) => {
            ...
            let col_idx = spec.selected_data_sets.first().copied().unwrap_or(0);
```
  Compare `resolve_parts_of_whole_series_index` (`categorical.rs:2859-2875`), which rejects out-of-range and multi-column selections.
- **Suggested fix:** Replace `resolve_selected_indices` at lines `2620` and `2905` with `try_resolve_selected_indices`, and validate `spec.selected_data_sets.len() <= 1` (and reject OOB via `try_resolve_selected_indices`) on the `Column` branch of `extract_exact_binomial_counts`.
- **Related:** `crates/stats/src/common.rs:244-266`, `docs/audit/local-code-audit-2026-11-18.md` (`#32`).

---

### FA-SEC05-03 · S1 · Two-group & MV entry points silently truncate multi-column selections, misroute `>= 2` categorical factors, and expose numeric `Label` IDs on explicit selections

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/two_group.rs:1757-1778, 1826-1840, 2176-2187, 2263-2273, 2564-2575`
- **Category:** `wrong-result`
- **Defect:** `run_two_group_analysis_inner`, `resolve_mv_one_sample_selection`, and `extract_multiple_row_inputs` truncate `selected_data_sets` (`&indices[..2]`, `cols.first()`), fall through from `resolve_mv_two_group_inputs` to categorical text columns when `cat_cols.len() != 1`, and keep numeric `VariableRole::Label` columns whenever `selected_data_sets` is non-empty.
- **Failure scenario:** (1) On a `Column` sheet with `selected_data_sets = [0, 1, 2]`, `UnpairedT`/`WelchT`/`PairedT`/`MannWhitneyU`/`KolmogorovSmirnov` silently drop column `2` via `&indices[..2]`, and `OneSampleT`/`OneSampleWilcoxon` silently drop column `1` via `cols.first()`. (2) On a `MultipleVariables` sheet with two `Categorical` columns (e.g. `Sex`, `Arm`) and `Before`/`After` continuous columns with `selected_data_sets = [0, 1, 2, 3]`, `cat_cols.len() == 2` returns `Ok(None)` (`line 2271`), so `line 1833` extracts `[0, 1]` (the two categorical text columns) instead of skipping non-numeric roles or returning an error. (3) Passing `selected_data_sets = [0, 1, 2]` when column `0` is a numeric `VariableRole::Label` (`SubjectID`) causes `OneSampleT`/`OneSampleWilcoxon` (`common.rs:324` only filters `Label` when `selected.is_empty()`) to test `SubjectID`, and causes `extract_multiple_row_inputs` (`line 2571`) to put `SubjectID` into Group A while empty selection skips `Label`.
- **Evidence:**
```rust
// crates/stats/src/two_group.rs:1758-1760, 1829-1834
        TwoGroupMethod::OneSampleT { hypothetical } => {
            let sel = resolve_mv_one_sample_selection(sheet, &spec.selected_data_sets, tg.mv_response_column)?;
            let cols = extract_numeric_columns(sheet, &sel)?;
            let first = cols.first().ok_or(StatsError::EmptyInput)?;
...
    } else {
        let indices = try_resolve_selected_indices(&spec.selected_data_sets, sheet.archetype.column_count())?;
        if indices.len() < 2 {
            return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: indices.len() });
        }
        extract_numeric_columns(sheet, &indices[..2])?
    };

// crates/stats/src/two_group.rs:2566-2574
                .filter(|&i| {
                    mv.variables.get(i).is_some_and(|v| {
                        if spec.selected_data_sets.is_empty() {
                            !matches!(v.role, VariableRole::Categorical | VariableRole::Label)
                        } else {
                            v.role != VariableRole::Categorical
                        }
                    })
                })
```
- **Suggested fix:** Reject `indices.len() != 2` (or `!= 1` for one-sample) when `selected_data_sets` is explicitly supplied on `Column`/unpaired paths (matching `extract_dataset_pair_multiple_row_inputs` at lines `2466-2468`), exclude `VariableRole::Label` consistently across MV selection paths unless explicitly requested as a `mv_response_column`, and reject MV sheets with `cat_cols.len() >= 2` instead of passing categorical text columns to numeric extraction.
- **Related:** `crates/stats/src/common.rs:322-327`, `docs/audit/local-code-audit-2026-11-18.md` (`#66`, `#32`).

---

### FA-SEC05-04 · S1 · Boschloo, CSM, and `noncentral_hypergeom_p_and_mean_log_or` use additive `1e-12` tolerances that inflate extreme-tail *P* values by orders of magnitude

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/categorical.rs:774-791, 869, 1441`
- **Category:** `wrong-result`
- **Defect:** `accumulate_boschloo_region`, `accumulate_csm_region`, and `noncentral_hypergeom_p_and_mean_log_or` compare tiny exact probabilities using an **additive** `+ 1e-12` tolerance rather than a relative multiplicative/log-space tolerance.
- **Failure scenario:** For `barnard_exact_2x2_with_ordering(40, 0, 0, 40, BarnardOrdering::Boschloo)`, the observed Fisher exact probability `p_obs` in slice `s = 40` is `~1.86e-23 << 1e-12`. Because line `788` checks `if pf <= p_obs + 1e-12` (and line `776` adds all `pv <= px + 1e-12` inside each slice's Fisher *P*), every slice sweeps all hypergeometric tables with `pf <= 1e-12` into the unconditional rejection region, inflating the returned Boschloo *P* value to `~3e-13` instead of `~1.65e-24` (over 10 orders of magnitude too large). The same additive pattern affects `accumulate_csm_region` (`line 869`: `val <= best_val + 1e-12`, merging distinct frontier tables once exact tail probabilities fall below `1e-12`) and unwindowed `noncentral_hypergeom_p_and_mean_log_or` (`line 1441`: `p <= obs_p + 1e-12`).
- **Evidence:**
```rust
// crates/stats/src/categorical.rs:775-777, 787-790
        for (x, px) in probs {
            let p_fisher: f64 = sorted.iter().take_while(|(_, pv)| *pv <= px + 1e-12).map(|(_, pv)| *pv).sum();
            out.push((x, px, p_fisher.clamp(0.0, 1.0)));
        }
...
        for (_, hg_p, pf) in fisher_slice(s) {
            if pf <= p_obs + 1e-12 {
                *slot += hg_p;
            }
        }

// crates/stats/src/categorical.rs:869, 1441
            if val <= best_val + 1e-12 {
...
    let p_sum: f64 = probs.iter().map(|(_, w)| w / norm).filter(|&p| p <= obs_p + 1e-12).sum();
```
  Contrast `fisher_exact_2x2` (`categorical.rs:509`) and `noncentral_hypergeom_p_and_mean_windowed` (`categorical.rs:1687`), which use relative log-space tolerances (`lp <= obs_log + log_rel_tol`).
- **Suggested fix:** Replace `+ 1e-12` at lines `776`, `788`, `869`, and `1441` with relative tolerances (e.g., `<= ref_val * (1.0 + 1e-12)` or log-space comparison `<= ln_ref + 1e-12`).
- **Related:** `docs/audit/local-code-audit-2026-11-18.md:1043`.

---

### FA-SEC05-05 · S2 · Multiple row-wise nonparametric tests emit sample counts as finite `df` instead of `f64::INFINITY`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/two_group.rs:1591, 2123, 2653, 2665`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `run_single_row_comparison` populates `MultipleRowTestEntry::df` (and the `"df"` cell in `"multiple-row-comparisons"`) with `out.n_effective as f64` for `WilcoxonSignedRank` and `(ra.len() + rb.len()) as f64 - 2.0` for `MannWhitneyU` instead of `f64::INFINITY` (which `ResultCell` / JSON treat as `Empty` or which is documented as rank-test `df`).
- **Failure scenario:** Running `run_multiple_row_t_tests_analysis` (or `multiple_row_t_tests`) with `nonparametric: true` emits `"df" = 10` on a 6-vs-6 Mann–Whitney row or `"df" = 5` on a 6-pair Wilcoxon row, presenting an incorrect finite degrees-of-freedom value in `ResultSheet::tabs` and violating `MultipleRowTestEntry::df`'s own contract (`two_group.rs:1591`).
- **Evidence:**
```rust
// crates/stats/src/two_group.rs:1591-1592
    /// Degrees of freedom (`n_a + n_b - 2`, Welch *df*, `n_pairs - 1`, or `f64::INFINITY` for rank tests).
    pub df: f64,

// crates/stats/src/two_group.rs:2649-2666
    if mr.paired && mr.nonparametric {
        let out = wilcoxon_matched_pairs(ra, rb, true, conf, false)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok((ma, mb, out.hodges_lehmann_pseudomedian, out.w_stat, out.n_effective as f64, out.p_value));
    }
...
    if mr.nonparametric {
        let out = mann_whitney_u(ra, rb, true, conf)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok((ma, mb, out.hodges_lehmann_diff, out.u_min, (ra.len() + rb.len()) as f64 - 2.0, out.p_value));
    }
```
  Note `compare_dunn_ranks` (`multiplicity.rs:1157`) sets `df: f64::INFINITY` (and `sanitize_result_sheet_floats` maps non-finite `Number` cells to `ResultCell::Empty`).
- **Suggested fix:** Return `f64::INFINITY` in `run_single_row_comparison` for both `WilcoxonSignedRank` and `MannWhitneyU` (so `"df"` becomes `ResultCell::Empty` on rank tests and `MultipleRowTestEntry::df` matches its doc and `multiplicity.rs`).
- **Related:** `crates/stats/src/multiplicity.rs:1157`.

---

### FA-SEC05-06 · S2 · `TwoGroupMethod::WilcoxonSignedRank` spec promises Pratt zero handling while `two_group.rs` hardcodes classic zero exclusion

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/analysis.rs:368`; `crates/stats/src/two_group.rs:1883, 2650`
- **Category:** `contract-drift` / `wrong-result`
- **Defect:** `TwoGroupMethod::WilcoxonSignedRank` in `curvebook-doc` documents Pratt (1959) zero handling, yet both `run_two_group_analysis_inner` and `run_single_row_comparison` hardcode `use_pratt_zeros = false` when calling `wilcoxon_matched_pairs`.
- **Failure scenario:** Calling `run_analysis` with `TwoGroupMethod::WilcoxonSignedRank` or `MultipleRowTestsSpec { paired: true, nonparametric: true, .. }` on paired data containing exact ties (`a_i == b_i`) drops all zero differences (`classic` method) rather than applying Pratt's zero-inclusion adjustment as documented in `TwoGroupMethod::WilcoxonSignedRank`; Pratt mode (`wilcoxon_matched_pairs(..., true)`) is unreachable from `AnalysisSpec`.
- **Evidence:**
```rust
// crates/doc/src/analysis.rs:368-369
    /// Paired nonparametric Wilcoxon signed-rank test with Pratt zero handling, exact small-sample *p*-values, and Walsh-average pseudo-median CI (Wilcoxon 1945; Pratt 1959).
    WilcoxonSignedRank,

// crates/stats/src/two_group.rs:1882-1883, 2650
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = wilcoxon_matched_pairs(&report.xs, &report.ys, tg.two_tailed, conf, false)?;
...
        let out = wilcoxon_matched_pairs(ra, rb, true, conf, false)?;
```
- **Suggested fix:** Either add an explicit zero-handling option to `TwoGroupMethod::WilcoxonSignedRank` / `MultipleRowTestsSpec` (defaulting consistently with the spec) or align the documentation with classic zero exclusion (`use_pratt_zeros = false`) and expose Pratt explicitly if intended.
- **Related:** `docs/implementation/statistics-implementation.md`.

---

### FA-SEC05-07 · S2 · XY Bland–Altman drops replicate subcolumns in `Y[A]` vs `Y[B]` mode and omits missing/text counts in `X` vs `Y[0]` mode

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/two_group.rs:2386-2445` (and `1522-1534`)
- **Category:** `wrong-result`
- **Defect:** On `TableArchetype::Xy`, `extract_bland_altman_pairs` handles replicate subcolumns and diagnostic counts asymmetrically between `xy_compare_x_vs_y = false` and `true`.
- **Failure scenario:** When comparing `Y[column_a]` vs `Y[column_b]` on an `XyTable` with `SubcolumnMode::RawReplicates(k >= 2)`, lines `2417-2418` inspect only replicate subcolumn `0` (`s.first()`) and silently discard replicates `1..k` (same replicate-drop pattern as `#120` in `regression.rs`). Conversely, when comparing `X` vs `Y[0]` (`lines 2396, 2406-2407`), `xy.active_xy_pairs(0)` expands all replicate subcolumns against `X` while hardcoding `skipped_incomplete_rows: 0` and `non_numeric_cells: 0`, omitting listwise-filtered and ignored-text warnings when cells are blank/excluded/text. Additionally, percent mode (`lines 1522-1534`) rejects `avg <= 0.0` (`#104`) while still accepting opposite-sign pairs with positive average (e.g. `-1.0` and `1.5`, giving `avg = 0.25` and `-1000%`, `#104`'s own negative/positive-crossing case).
- **Evidence:**
```rust
// crates/stats/src/two_group.rs:2396-2408, 2417-2418
                let pairs = xy.active_xy_pairs(0);
...
                Ok(BlandAltmanExtractedPairs {
                    label_a: xy.x_title.clone(),
                    label_b: ds.title.clone(),
                    xs,
                    ys,
                    skipped_incomplete_rows: 0,
                    non_numeric_cells: 0,
                })
            } else {
...
                for r in 0..xy.row_count() {
                    let ca = dsa.cells.get(r).and_then(|s| s.first());
                    let cb = dsb.cells.get(r).and_then(|s| s.first());
```
- **Suggested fix:** Either pair every matching replicate subcolumn across `dsa`/`dsb` or reject `SubcolumnMode::RawReplicates(k >= 2)` in `Y[A]` vs `Y[B]` mode (and track `skipped_incomplete_rows` / `non_numeric_cells` in `X` vs `Y[0]` mode); document or guard negative measurements (`a <= 0 || b <= 0`) in percent mode.
- **Related:** `docs/audit/local-code-audit-2026-11-18.md` (`#4`, `#104`, `#120`).

---

### FA-SEC05-08 · S2 · Power detectable-effect solvers (`solve_t_delta`, `solve_correlation_r`) fail on valid one-sided `target_power < alpha` requests

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/power.rs:1761-1798, 1826-1848`
- **Category:** `wrong-result` / `robustness`
- **Defect:** For one-sided `DetectableEffect` solves when `target_power < alpha` (`p_zero == alpha`), `solve_t_delta` checks the wrong inequality sign during bracket expansion and fails with `NotConverged`, while `solve_correlation_r` misclassifies the anti-directional root as `InvalidParameter("target_power")`.
- **Failure scenario:** Requesting `PowerSolveTarget::DetectableEffect` for `OneSampleT` / `UnpairedT` / `PairedT` / `SimpleLinearRegression` with `alpha = 0.05`, `tail_mode = TailMode::OneSided(OneSidedDirection::Greater)`, `sample_size = 20.0`, and `target_power = 0.02`: `p_zero = 0.05 > 0.02`, so `solve_t_delta` sets `search_positive = false` (`lo = -2.0, hi = 0.0`). Because `eval(0.0) = +0.03 > 0` and `eval(-2.0) < 0`, a root already exists in `(-2.0, 0.0)`, yet line `1790` checks `if eval(lo) >= 0.0` (which is never true as `lo -> -infinity`), loops 48 times, and returns `Err(StatsError::NotConverged { algorithm: "solve_t_delta" })`. Symmetrically `OneSided(Less)` with `target_power = 0.02` sets `search_positive = true` and fails at line `1780`. Under `Correlation`, `solve_correlation_r` (`1839, 1844`) hits `eval(±max_r) < 0.0` and errors instead of either rejecting `target_power <= alpha` uniformly upfront (as `SampleSize` does at line `1655`) or checking `f_lo * f_hi <= 0.0` (as `solve_two_proportions_h` does at line `1906`).
- **Evidence:**
```rust
// crates/stats/src/power.rs:1770-1795
    let search_positive = match tail_mode {
        TailMode::TwoSided | TailMode::OneSided(OneSidedDirection::Greater) => target_power >= p_zero,
        TailMode::OneSided(OneSidedDirection::Less) => target_power < p_zero,
    };
...
    if search_positive {
        let mut lo = 0.0_f64;
        let mut hi = 2.0_f64;
        for _ in 0..48 {
            if eval(hi) >= 0.0 {
                return Ok(brent_root(eval, lo, hi, ROOT_TOL)?);
            }
            lo = hi;
            hi *= 2.0;
        }
    } else {
        let mut hi = 0.0_f64;
        let mut lo = -2.0_f64;
        for _ in 0..48 {
            if eval(lo) >= 0.0 {
                return Ok(brent_root(eval, lo, hi, ROOT_TOL)?);
            }
```
- **Suggested fix:** Check bracket sign change `(eval(0.0) * eval(endpoint)) <= 0.0` in `solve_t_delta` and `solve_correlation_r` (matching `solve_two_proportions_h`), or consistently enforce `target_power > alpha` across all families for `DetectableEffect` if sub-alpha detectable effects are intended to be disallowed.
- **Related:** `crates/stats/src/power.rs:1655-1657, 1904-1909`.

---

### FA-SEC05-09 · S2 · Mann–Whitney normal approximation returns `p = 0.5` instead of `1.0` for all-identical samples when `na + nb > 18` and `two_tailed = false`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/two_group.rs:959-973`
- **Category:** `wrong-result`
- **Defect:** In `mann_whitney_u`, when `na + nb > 18` and all values in both groups are identical (exact tie-corrected variance `var_u == 0.0`), line `963` clamps `var_u` to `1e-30` without short-circuiting the zero-variance null case, causing one-tailed calls to evaluate `norm_sf(0.0) = 0.5`.
- **Failure scenario:** Calling `mann_whitney_u(&[5.0; 10], &[5.0; 10], false, ConfidenceLevel::DEFAULT_95)` produces `u_a == mean_u == 50.0`, `var_u = 1e-30`, `cc = ((0.0) - 0.5).max(0.0) = 0.0`, and returns `p_value = 0.5`, whereas the exact conditional permutation DP (`&[5.0; 9]` vs `&[5.0; 9]`, `na + nb = 18` at lines `916-919`) returns `p_value = 1.0`, and `wilcoxon_matched_pairs` (`lines 1243-1245`) explicitly guards `var_w <= 0.0` -> `1.0`.
- **Evidence:**
```rust
// crates/stats/src/two_group.rs:962-971
        let mean_u = 0.5 * naf * nbf;
        let var_u = ((naf * nbf / 12.0) * (nf + 1.0 - ranked.tie_sum_t3_minus_t / (nf * (nf - 1.0)))).max(1e-30);
        let sd_u = var_u.sqrt();
        let p = if two_tailed {
            let cc = (u_a - mean_u).abs() - 0.5;
            if cc <= 0.0 { 1.0 } else { (2.0 * norm_sf(cc / sd_u)).min(1.0) }
        } else {
            let cc = ((u_a - mean_u).abs() - 0.5).max(0.0);
            norm_sf(cc / sd_u).clamp(0.0, 1.0)
        };
```
- **Suggested fix:** Check `var_u <= 0.0` before clamping and return `1.0` when `(u_a - mean_u).abs() <= 1e-12` (matching `wilcoxon_matched_pairs` at `two_group.rs:1243-1245`).
- **Related:** `crates/stats/src/two_group.rs:916-919, 1243-1245`.

---

### FA-SEC05-10 · S2 · `compare_groups_*` lacks domain validation on `mse`, `df_error`, `n`, `mean`, and `sd`, silently clamping negative/`NaN` variances to zero

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/multiplicity.rs:570-612, 653, 708, 766, 795, 853, 887-897`
- **Category:** `robustness` / `wrong-result`
- **Defect:** `compare_groups_impl` validates only `threshold` and `ci_alpha`, leaving `mse`, `df_error`, and per-group `(n, mean, sd)` unvalidated across pooled and Welch comparison engines.
- **Failure scenario:** Calling `compare_groups_with_procedure` (public API) with a pooled procedure and negative or `NaN` `mse` (`f64::max(NaN, 0.0) == 0.0` in Rust/IEEE-754) silently clamps `se` to `0.0` via `.max(0.0).sqrt()`, returning `statistic = ±inf, p = 0.0` (or `p = 1.0` for equal means) instead of `Err(StatsError::InvalidParameter)`. Passing `ga.n <= 0.0` produces `1.0 / ga.n <= 0.0` or `+inf`; `compare_newman_keuls` with `se_q == 0.0` never checks `df_error > 0.0` or finite `df_error`; and `welch_pair_se_df` (`887-897`) squares negative `ga.sd` silently and accepts non-finite `mean`/`sd`/`n`. (Note `r35_power_multiplicity_oracles.rs:74` intentionally passes `mse = 0.0, df_error = 0.0` as unused placeholders for `GamesHowell`/`TamhaneT2`/`DunnettT3`, and `mse = 0.0` / `sd = 0.0` is valid for zero-variance tests.)
- **Evidence:**
```rust
// crates/stats/src/multiplicity.rs:570-589, 653, 887-896
fn compare_groups_impl(
    groups: &[GroupComparisonInput],
    mse: f64,
    df_error: f64,
...
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    if report_ci && (!ci_alpha.is_finite() || ci_alpha <= 0.0 || ci_alpha >= 1.0) {
        return Err(StatsError::InvalidParameter { name: "ci_alpha", value: ci_alpha });
    }
...
            let se = (mse * (1.0 / ga.n + 1.0 / gb.n)).max(0.0).sqrt();
...
pub(crate) fn welch_pair_se_df(ga: &GroupComparisonInput, gb: &GroupComparisonInput) -> (f64, f64) {
    let va = (ga.sd * ga.sd) / ga.n;
    let vb = (gb.sd * gb.sd) / gb.n;
    let v_sum = va + vb;
    if v_sum <= 0.0 {
        return (0.0, (ga.n + gb.n - 2.0).max(1.0));
    }
```
  Contrast `linear_trend_test` (`multiplicity.rs:231, 241`), which checks `!mse.is_finite() || mse < 0.0 || !df_error.is_finite() || df_error <= 0.0` and `g.n <= 0.0`.
- **Suggested fix:** Validate every group (`n.is_finite() && n > 0.0`, `mean.is_finite()`, `sd.is_finite() && sd >= 0.0`) in `compare_groups_impl`, and for pooled procedures enforce `mse.is_finite() && mse >= 0.0` and `df_error.is_finite() && df_error > 0.0`.
- **Related:** `crates/stats/src/multiplicity.rs:231-243`.

---

### FA-SEC05-11 · S2 · `run_roc_analysis` drops `cols[2..]`, skips ignored-text warnings, and omits `sanitize_result_sheet_floats`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/roc.rs:344-354, 459` (plus `crates/stats/src/two_group.rs:2023-2044`)
- **Category:** `wrong-result` / `robustness`
- **Defect:** `run_roc_analysis` extracts all columns via `extract_numeric_columns(sheet, &spec.selected_data_sets)?`, ignores any third+ column instead of rejecting ambiguous multi-column selections, never passes `non_numeric_count` to `append_ignored_text_checklist_and_footnote`, and returns without calling `sanitize_result_sheet_floats`.
- **Failure scenario:** Running `run_roc_analysis` on a 3-column sheet with `selected_data_sets = [0, 1, 2]` silently ignores column `2`; running on a sheet whose Control or Case column contains text strings like `"ND"` drops those rows silently with no `"Numeric input validation"` checklist item or footnote (`controls.non_numeric_count` and `cases.non_numeric_count` are discarded). Likewise, `run_multiple_row_t_tests_analysis` (`two_group.rs:2023-2044`) silently drops non-numeric cells without emitting ignored-text or listwise-incomplete warnings.
- **Evidence:**
```rust
// crates/stats/src/roc.rs:344-354, 459
    let cols = match &sheet.archetype {
        TableArchetype::Column(_) | TableArchetype::MultipleVariables(_) => extract_numeric_columns(sheet, &spec.selected_data_sets)?,
        other => return Err(StatsError::IncompatibleTable { analysis: "ROC curve", actual: other.kind() }),
    };
    if cols.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ROC groups", required: 2, actual: cols.len() });
    }
    let controls = &cols[0];
    let cases = &cols[1];
    let res = roc_curve_analysis(&controls.values, &cases.values, roc_spec.ci_method, confidence)?;
...
    Ok(out)
```
- **Suggested fix:** Reject `spec.selected_data_sets` when `len() != 2` (if non-empty) in `run_roc_analysis`, call `append_ignored_text_checklist_and_footnote` for non-zero `non_numeric_count` (and similarly track ignored text / incomplete pairs in `run_multiple_row_t_tests_analysis`), and call `sanitize_result_sheet_floats(&mut out)?` before returning.
- **Related:** `crates/stats/src/column.rs:1815, 2340`, `crates/stats/src/two_group.rs:1848`.

---

### FA-SEC05-12 · S3 · `p_cell` significance stars ignore configured `alpha` / `threshold` in contingency, multiplicity, and P-value stack tabs; `one_sample_ks_normal` hardcodes `0.05`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/categorical.rs:2539, 2668, 2993`; `crates/stats/src/multiplicity.rs:1199-1200`; `crates/stats/src/pvalue_stack.rs:609-610`; `crates/stats/src/column.rs:1709`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `run_contingency_analysis_inner`, `build_multiplicity_tab`, and `run_pvalue_stack_analysis` format `ResultCell::PValue` via `p_cell(..., p_fmt)` (which evaluates significance stars against fixed `0.05`) instead of `p_cell_at_alpha`, and `one_sample_ks_normal` hardcodes `passed: p_value > 0.05`.
- **Failure scenario:** When a user requests `ConfidenceLevel` 99% (`alpha = 0.01`) or `PValueStackSpec { threshold: 0.01, .. }` and observes `p = 0.03`, `run_pvalue_stack_analysis` sets `"Discovery?" = false` alongside a `"*"` significance star in the same row, and `"contingency-main"` / `"multiple-comparisons"` display `"*"` at `p = 0.03` despite `Significant = false`. Similarly `one_sample_ks_normal` offers no `alpha` parameter and checks `p_value > 0.05`.
- **Evidence:**
```rust
// crates/stats/src/categorical.rs:2668
    let mut row_main = vec![opt_num_cell(primary.stat), opt_num_cell(primary.df), p_cell(primary.p_value, p_fmt)];

// crates/stats/src/multiplicity.rs:1199-1200
            crate::common::p_cell(d.raw_p, p_fmt),
            crate::common::p_cell(d.adjusted_p_or_q, p_fmt),

// crates/stats/src/pvalue_stack.rs:609-611
                p_cell(r.raw_p, p_fmt),
                p_cell(r.adjusted_p_or_q, p_fmt),
                ResultCell::Boolean(r.discovery),

// crates/stats/src/column.rs:1709
    Ok(NormalityTestOutcome { test_name: String::from("One-sample KS (Normal)"), statistic: d, p_value, passed: p_value > 0.05 })
```
- **Suggested fix:** Use `p_cell_at_alpha` with `confidence.alpha()` / `ps.threshold` / `spec.threshold` in those builders (matching `column.rs:2202, 2264` and `two_group.rs:2729, 2952`).
- **Related:** `crates/stats/src/common.rs:181-203`, `docs/audit/local-code-audit-2026-11-18.md` (`#82`).

---

### FA-SEC05-13 · S4 · Documentation drift on `BarnardExactResult::statistic`, `NormalityTestOutcome::passed`, and `adjust_p_values` legacy ANOVA fallback

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/categorical.rs:245-246, 898-901`; `crates/stats/src/column.rs:805, 884`; `crates/stats/src/multiplicity.rs:461-484`
- **Category:** `contract-drift` / `cleanup`
- **Defect:** Public struct/function docstrings overstate or misstate actual field semantics and fallbacks.
- **Failure scenario:** Callers reading doc comments can misinterpret: (1) `BarnardExactResult::statistic`, documented as `(|Z| or -p_fisher)`, actually stores signed `z_pooled_2x2` (for `ScorePooled`, `Boschloo`, and `Csm`) or signed `z_wald_2x2` (`categorical.rs:898-901`); (2) `NormalityTestOutcome::passed`, documented as `"True when p_value >= 0.05"`, actually checks strict `p_value > alpha` with caller-supplied `alpha` (`884, 913, 1040, 1059`); (3) `adjust_p_values` (`multiplicity.rs:461-484`) documents only standalone raw-p procedures while still mapping ANOVA-only variants (`TukeyKramer`, `Dunnett`, `GamesHowell`, `LinearTrend`, `TamhaneT2`) to Bonferroni/Šídák (`try_adjust_p_values` at `492` properly errors).
- **Evidence:**
```rust
// crates/stats/src/categorical.rs:245-246, 898-900
    /// Observed test statistic under the selected `BarnardOrdering` rule (`|Z|` or `-p_fisher`).
    pub statistic: f64,
...
    let z_stat = match ordering {
        BarnardOrdering::WaldUnpooled => z_wald_2x2(a, n1, c, n2),
        BarnardOrdering::ScorePooled | BarnardOrdering::Boschloo | BarnardOrdering::Csm => z_pooled_2x2(a, n1, c, n2),
    };

// crates/stats/src/column.rs:805, 884
    /// True when `p_value >= 0.05` (does not reject normality at the 5% level).
    pub passed: bool,
...
    Ok(NormalityTestOutcome { test_name: String::from("D'Agostino-Pearson K2"), statistic: k2, p_value, passed: p_value > alpha })
```
- **Suggested fix:** Update docstrings on `BarnardExactResult::statistic`, `NormalityTestOutcome::passed`, and `adjust_p_values` to match actual behavior (or deprecate/remove silent fallback in `adjust_p_values` in favor of `try_adjust_p_values`).
- **Related:** `docs/audit/local-code-audit-2026-11-18.md:1045`.
