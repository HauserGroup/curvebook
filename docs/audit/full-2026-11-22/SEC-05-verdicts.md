# SEC-05 Verification Verdicts — Statistics Core (`crates/stats`)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend` (independent source inspection + isolated `.lc-worktrees/ver-backend` dynamic probe `ver_sec05_probe`, `7 passed; 0 failed`)
- **Source Report:** [SEC-05.md](../../../docs/audit/full-2026-11-22/SEC-05.md) (`FA-SEC05-01`..`FA-SEC05-13`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 13 | `S1: 4`, `S2: 7`, `S3: 1`, `S4: 1` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **13** | **`S1=4, S2=7, S3=1, S4=1`** |

Dynamic reproducer run against an isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095`:
- Command: `CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend cargo test --manifest-path .lc-worktrees/ver-backend/Cargo.toml --locked --offline -p curvebook-stats --test ver_sec05_probe -- --nocapture`
- Result: `running 7 tests` -> `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s` (`probe_01`, `probe_02`, `probe_03`, `probe_04`, `probe_05_06_07_09`, `probe_08`, `probe_10_11_12`).

Ledger cross-check notes:
- `FA-SEC05-01` corresponds to surviving open item `R2-S07-07` in [local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L665).
- `FA-SEC05-02` is an incomplete fix / **REGRESSION** relative to closed `#32` ([local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L323-L329)), which introduced `try_resolve_selected_indices` while leaving `categorical.rs` callers on `resolve_selected_indices` and `.first().copied()`.
- `FA-SEC05-04` and `FA-SEC05-13` align with open unnumbered Wave 5 notes in [local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L1043-L1045).

---

## File Coverage Completeness Check

Verified `100%` coverage against `SEC-05` scope in `crates/stats` at `7c85a8e516165ddadb101236a88eec937bf1f095` (`12` files: `Cargo.toml` + `11` top-level files in `crates/stats/src/`, `16,079` total lines; excludes `multivariate.rs`, `regression.rs`, `survival.rs`, and `anova/**` assigned to `SEC-06`):

- [Cargo.toml](../../../crates/stats/Cargo.toml) (`20`), [lib.rs](../../../crates/stats/src/lib.rs) (`109`), [error.rs](../../../crates/stats/src/error.rs) (`127`), [roc.rs](../../../crates/stats/src/roc.rs) (`460`), [cld.rs](../../../crates/stats/src/cld.rs) (`619`), [pvalue_stack.rs](../../../crates/stats/src/pvalue_stack.rs) (`634`), [common.rs](../../../crates/stats/src/common.rs) (`721`), [multiplicity.rs](../../../crates/stats/src/multiplicity.rs) (`1,211`), [column.rs](../../../crates/stats/src/column.rs) (`2,343`), [power.rs](../../../crates/stats/src/power.rs) (`2,757`), [categorical.rs](../../../crates/stats/src/categorical.rs) (`3,526`), [two_group.rs](../../../crates/stats/src/two_group.rs) (`3,552`).

Every file and line count matches `SEC-05.md`'s File Coverage Table with zero omissions.

---

## Per-Finding Verdicts

### Verdict `FA-SEC05-01` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [run_contingency_analysis_inner](../../../crates/stats/src/categorical.rs#L2608-L2615) checks only `ContingencyMethod::ExactBinomial` before routing every `TableArchetype::PartsOfWhole` request unconditionally to [run_parts_of_whole_gof](../../../crates/stats/src/categorical.rs#L3053-L3150). Dynamic `probe_01`: running `PartsOfWhole` with `CohenKappa`, `FleissKappa`, `FisherExact`, `BarnardExact`, `CochranArmitageTrend`, and `McNemarPaired` all returned `Ok` with `ResultStatus::UpToDate` and output tab `id == "parts-gof"` instead of an incompatibility error. Survives open `R2-S07-07` in [local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L665).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silently executes an unrelated statistical test while reporting `ResultStatus::UpToDate`).
- **Still present on main:** yes

### Verdict `FA-SEC05-02` · `CONFIRMED` (**REGRESSION / incomplete `#32`**)

- **Verifier:** `ver-backend`
- **Evidence:** [run_contingency_analysis_inner](../../../crates/stats/src/categorical.rs#L2619-L2631) and [extract_exact_binomial_counts](../../../crates/stats/src/categorical.rs#L2903-L2926) still call [resolve_selected_indices](../../../crates/stats/src/common.rs#L244-L266) (silently dropping OOB indices and accepting duplicates) instead of `try_resolve_selected_indices`, while the `Column` branch of `extract_exact_binomial_counts` reads `spec.selected_data_sets.first().copied()`. Dynamic `probe_02`: `Contingency` with `selected_data_sets = [0, 0]` returned `Ok` with `scalars[0].value == 0.0`; `[0, 1, 99]` returned `Ok` (dropping `99`); `ExactBinomial` on `Column` with `[0, 1]` returned `Ok` (discarding column `1`). Leaves closed `#32` ([local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L323-L329)) incomplete on contingency/binomial extractors.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silent column duplication/drop corrupts table dimensions and test results).
- **Still present on main:** yes

### Verdict `FA-SEC05-03` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [run_two_group_analysis_inner](../../../crates/stats/src/two_group.rs#L1757-L1778) (`cols.first()`), [line 1834](../../../crates/stats/src/two_group.rs#L1826-L1840) (`&indices[..2]`), [resolve_mv_one_sample_selection](../../../crates/stats/src/two_group.rs#L2176-L2187) (coupled with [extract_numeric_columns](../../../crates/stats/src/common.rs#L322-L327) filtering `VariableRole::Label` only when `selected.is_empty()`), [resolve_mv_two_group_inputs](../../../crates/stats/src/two_group.rs#L2263-L2273) (`cat_cols.len() == 2` returns `Ok(None)`), and [extract_multiple_row_inputs](../../../crates/stats/src/two_group.rs#L2564-L2575) (`v.role != VariableRole::Categorical` when `selected_data_sets` is non-empty). Dynamic `probe_03`: `Column` `UnpairedT` `[0, 1, 2]` and `OneSampleT` `[0, 1]` both returned `Ok`; MV `OneSampleT` with `[0, 1, 2]` where column `0` is numeric `Label` (`1000..1003`) tested `SubjectID` (`t = 1551.517`, `t > 1000.0`); `MultipleRowTTests` `[0, 1, 2, 3]` pulled numeric `Label` into `Group A` (`mean > 400.0`); MV `TwoGroup` with two `Categorical` columns at indices `0, 1` plus numeric columns `2, 3` (`selected_data_sets = [0, 1, 2, 3]`) fell through and tested `Categorical[0]` vs `Categorical[1]` (`t == 0.0`, `"Mean (Variable A)"`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silently tests wrong columns or ID/categorical columns without error).
- **Still present on main:** yes

### Verdict `FA-SEC05-04` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [accumulate_boschloo_region](../../../crates/stats/src/categorical.rs#L774-L791) (`*pv <= px + 1e-12`, `pf <= p_obs + 1e-12`), [accumulate_csm_region](../../../crates/stats/src/categorical.rs#L869) (`val <= best_val + 1e-12`), and [noncentral_hypergeom_p_and_mean_log_or](../../../crates/stats/src/categorical.rs#L1441) (`p <= obs_p + 1e-12`) use additive `+ 1e-12` tie tolerances on probabilities that drop below `1e-12`. Dynamic `probe_04`: `barnard_exact_2x2_with_ordering(40, 0, 0, 40, BarnardOrdering::Boschloo)` returned `p_value > 1e-13` instead of the exact `~1.65e-24` tail (>10 orders of magnitude inflation). Also noted in [local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L1043).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (severe *P*-value inflation on valid extreme `2x2` contingency tables, pushing exact `< 1e-23` significance far past `1e-13`).
- **Still present on main:** yes

### Verdict `FA-SEC05-05` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [MultipleRowTestEntry::df](../../../crates/stats/src/two_group.rs#L1591) documents `f64::INFINITY` for rank tests, yet [run_single_row_comparison](../../../crates/stats/src/two_group.rs#L2649-L2666) returns `out.n_effective as f64` (`WilcoxonSignedRank`) and `(ra.len() + rb.len()) as f64 - 2.0` (`MannWhitneyU`). Dynamic `probe_05_06_07_09`: `MultipleRowTTests { paired: false, nonparametric: true }` emitted finite `ResultCell::Number(10.0)` instead of `ResultCell::Empty` in column 4 of `"multiple-row-comparisons"`. Compare [compare_dunn_ranks](../../../crates/stats/src/multiplicity.rs#L1157) setting `f64::INFINITY`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (publishes fabricated finite degrees of freedom on rank tests in `ResultSheet` outputs).
- **Still present on main:** yes

### Verdict `FA-SEC05-06` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [TwoGroupMethod::WilcoxonSignedRank](../../../crates/doc/src/analysis.rs#L368-L369) specifies Pratt zero handling, while both [run_two_group_analysis_inner](../../../crates/stats/src/two_group.rs#L1883) and [run_single_row_comparison](../../../crates/stats/src/two_group.rs#L2650) pass `use_pratt_zeros = false` to `wilcoxon_matched_pairs`. Dynamic `probe_05_06_07_09`: `TwoGroupMethod::WilcoxonSignedRank` on `[1..5]` vs `[1, 1, 1, 2, 3]` matched `wilcoxon_matched_pairs(..., false)` (`classic`) rather than `wilcoxon_matched_pairs(..., true)` (`Pratt`, differing in `n_effective` and `p_value` by `> 1e-4`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (analysis dispatch silently uses classic zero-dropping instead of documented Pratt zero handling whenever ties occur).
- **Still present on main:** yes

### Verdict `FA-SEC05-07` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [extract_bland_altman_pairs](../../../crates/stats/src/two_group.rs#L2396-L2444) hardcodes `skipped_incomplete_rows: 0` / `non_numeric_cells: 0` on `X` vs `Y[0]` (`line 2396, 2406-2407`) and takes `s.first()` only on `Y[A]` vs `Y[B]` (`lines 2417-2418`), while [bland_altman](../../../crates/stats/src/two_group.rs#L1522-L1534) rejects only `avg <= 0.0` in percent mode. Dynamic `probe_05_06_07_09`: XY `BlandAltman` (`xy_compare_x_vs_y: Some(false)`) silently dropped replicate index `1` (`"Bias" == -2.0`), and `bland_altman(&[-1.0, 10.0], &[1.5, 11.0], true, ...)` produced `-1000.0%` on the sign-crossing pair `(-1.0, 1.5)` despite positive mean `0.25`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (drops replicates silently, suppresses missing/text warnings on `X` vs `Y[0]`, and produces pathological `%` differences on sign-crossing pairs).
- **Still present on main:** yes

### Verdict `FA-SEC05-08` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [solve_t_delta](../../../crates/stats/src/power.rs#L1761-L1798) checks `eval(lo) >= 0.0` in the `search_positive = false` branch (`target_power < alpha` on `OneSided(Greater)`) and fails `NotConverged`, and [solve_correlation_r](../../../crates/stats/src/power.rs#L1826-L1848) rejects `target_power < alpha` on `OneSided` instead of either rejecting `target_power <= alpha` uniformly upfront ([line 1655](../../../crates/stats/src/power.rs#L1655-L1657)) or checking opposite endpoint signs. Dynamic `probe_08`: `PowerAnalysisSpec` with `DetectableEffect`, `TailMode::OneSided(Greater)`, `alpha = 0.05`, `target_power = Some(0.02)`, `sample_size = Some(20.0)` returned `Err` for both `OneSampleT` and `Correlation`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (consistent solver failure on valid/consistent one-sided power targets below `alpha`).
- **Still present on main:** yes

### Verdict `FA-SEC05-09` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [mann_whitney_u](../../../crates/stats/src/two_group.rs#L959-L973) clamps `var_u.max(1e-30)` (`line 963`) without checking `var_u <= 0.0` before calling `norm_sf(cc / sd_u)`. Dynamic `probe_05_06_07_09`: `mann_whitney_u(&[5.0; 10], &[5.0; 10], false, ConfidenceLevel::DEFAULT_95)` returned `p_value == 0.5` instead of `1.0` (whereas exact DP for `na + nb <= 18` returns `1.0` and `wilcoxon_matched_pairs` at [lines 1243-1245](../../../crates/stats/src/two_group.rs#L1243-L1245) guards `var_w <= 0.0`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (reports `p = 0.5` instead of `1.0` for identical groups once `na + nb > 18`).
- **Still present on main:** yes

### Verdict `FA-SEC05-10` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [compare_groups_impl](../../../crates/stats/src/multiplicity.rs#L570-L612) validates only `threshold` and `ci_alpha`, while pooled helpers ([lines 653, 708, 766, 795, 853](../../../crates/stats/src/multiplicity.rs#L653)) use `.max(0.0).sqrt()` and [welch_pair_se_df](../../../crates/stats/src/multiplicity.rs#L887-L897) never validates `(n, mean, sd)` finiteness/sign. Dynamic `probe_10_11_12`: `compare_groups_with_procedure` with `mse = -10.0` and `mse = f64::NAN` under `Bonferroni` both returned `Ok` with `statistic = f64::INFINITY, raw_p = 0.0` instead of `Err(InvalidParameter)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (`f64::max(NaN, 0.0)` and negative variances silently turn invalid inputs into zero-SE `inf` statistics).
- **Still present on main:** yes

### Verdict `FA-SEC05-11` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [run_roc_analysis](../../../crates/stats/src/roc.rs#L344-L354, #L459) slices `&cols[0]`, `&cols[1]` without rejecting `cols[2..]` on explicit selections, drops `non_numeric_count`, and omits `sanitize_result_sheet_floats`; [run_multiple_row_t_tests_analysis](../../../crates/stats/src/two_group.rs#L2023-L2044) similarly omits ignored-text and listwise-incomplete diagnostics. Dynamic `probe_10_11_12`: `RocCurve` with `selected_data_sets = [0, 1, 2]` containing `"ND"` text returned `Ok` with no `"Numeric input"` checklist warning.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent column truncation and missing data-filtering warnings/float sanitization).
- **Still present on main:** yes

### Verdict `FA-SEC05-12` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [categorical.rs](../../../crates/stats/src/categorical.rs#L2539) (also `#L2668`, `#L2993`, and additionally `#L3141` in `run_parts_of_whole_gof`), [build_multiplicity_tab](../../../crates/stats/src/multiplicity.rs#L1199-L1200), and [pvalue_stack.rs](../../../crates/stats/src/pvalue_stack.rs#L609-L611) call [p_cell](../../../crates/stats/src/common.rs#L181-L203) (fixed `0.05` star thresholds) instead of `p_cell_at_alpha`, and [one_sample_ks_normal](../../../crates/stats/src/column.rs#L1709) hardcodes `p_value > 0.05`. Dynamic `probe_10_11_12`: `PValueStack` with `threshold = 0.01` on `p = 0.03` emitted `ResultCell::Boolean(false)` for `"Discovery?"` in the same row as `PValue { stars: "*", .. }`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (contradictory star vs decision indicators when `alpha != 0.05`).
- **Still present on main:** yes

### Verdict `FA-SEC05-13` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [BarnardExactResult::statistic](../../../crates/stats/src/categorical.rs#L245-L246) documents `(|Z| or -p_fisher)` while [lines 898-901](../../../crates/stats/src/categorical.rs#L898-L901) assign signed `z_pooled_2x2` (`ScorePooled`, `Boschloo`, `Csm`) or signed `z_wald_2x2`; [NormalityTestOutcome::passed](../../../crates/stats/src/column.rs#L805) documents `p_value >= 0.05` while [line 884](../../../crates/stats/src/column.rs#L884) computes `p_value > alpha`; [adjust_p_values](../../../crates/stats/src/multiplicity.rs#L461-L484) documents standalone adjustment procedures while silently mapping ANOVA variants onto Bonferroni/Šídák. Also tracked in [local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L1045).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (public docstrings diverge from implemented semantics).
- **Still present on main:** yes
