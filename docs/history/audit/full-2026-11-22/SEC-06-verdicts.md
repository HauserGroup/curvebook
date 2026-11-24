# SEC-06 Verification Verdicts — Statistics Regression, Survival, Multivariate & ANOVA (`crates/stats`)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend-2` (independent source inspection + engine guard cross-check + isolated `.lc-worktrees/ver-backend-2` dynamic probe `ver_sec06_probe`, cleaned up after execution)
- **Source Report:** [SEC-06.md](SEC-06.md) (`FA-SEC06-01`..`FA-SEC06-11`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 11 | `S1: 0`, `S2: 4`, `S3: 5`, `S4: 2` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **11** | **`S1=0, S2=4, S3=5, S4=2`** |

Dynamic verification run against an isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095` (removed cleanly afterward):
- Baseline suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-2 cargo test --manifest-path .lc-worktrees/ver-backend-2/Cargo.toml --locked --offline -p curvebook-stats`): `292 passed; 0 failed; 0 ignored` (`18` unit + `273` integration across `33` binaries + `1` doc-test).
- Targeted reproducer suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-2 cargo test --manifest-path .lc-worktrees/ver-backend-2/Cargo.toml --locked --offline -p curvebook-stats --test ver_sec06_probe -- --nocapture`): `running 11 tests` -> `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`.

Cross-reference notes:
- `FA-SEC06-09` parallels `FA-SEC05-12` (`SEC-05`) on non-overlapping `SEC-06` modules (`anova/**`, `multivariate.rs`, `regression.rs`, `survival.rs`) and is adjusted `S4 -> S3` for consistency with `FA-SEC05-12` after confirming direct intra-row/intra-sheet contradictions at `99%` confidence.
- Precision nuances identified during verification (without refuting the underlying defects) are recorded on `FA-SEC06-02` (`ColumnTable` rejects `RawReplicates` at construction), `FA-SEC06-04` (`unstructured_reduce_design` slice panic triggers only on rank-deficient `X`), `FA-SEC06-05` (`MathError::NotPositiveDefinite` variant spelling), and `FA-SEC06-10` (`k == 2` omitted multiplicity matches `NestedAnalysisSpec::multiplicity` docstring).

---

## File Coverage Completeness Check

Verified `100%` coverage against `SEC-06` scope at `7c85a8e516165ddadb101236a88eec937bf1f095` (`18` source files, `17,399` total lines; zero files omitted under `crates/stats/src/anova/**` plus `multivariate.rs`, `regression.rs`, `survival.rs`):

- [multivariate.rs](../../../../crates/stats/src/multivariate.rs) (`2,899`), [regression.rs](../../../../crates/stats/src/regression.rs) (`2,534`), [survival.rs](../../../../crates/stats/src/survival.rs) (`3,048`)
- `crates/stats/src/anova/`: [effect_sizes.rs](../../../../crates/stats/src/anova/effect_sizes.rs) (`228`), [factorial.rs](../../../../crates/stats/src/anova/factorial.rs) (`353`), [mod.rs](../../../../crates/stats/src/anova/mod.rs) (`114`), [nested.rs](../../../../crates/stats/src/anova/nested.rs) (`208`), [nonparametric.rs](../../../../crates/stats/src/anova/nonparametric.rs) (`118`), [one_way.rs](../../../../crates/stats/src/anova/one_way.rs) (`629`), [posthoc.rs](../../../../crates/stats/src/anova/posthoc.rs) (`1,652`), [reml.rs](../../../../crates/stats/src/anova/reml.rs) (`1,054`), [reml_unstructured.rs](../../../../crates/stats/src/anova/reml_unstructured.rs) (`943`), [results.rs](../../../../crates/stats/src/anova/results.rs) (`438`), [rm_two_way.rs](../../../../crates/stats/src/anova/rm_two_way.rs) (`382`)
- `crates/stats/src/anova/sheet/`: [factorial.rs](../../../../crates/stats/src/anova/sheet/factorial.rs) (`929`), [mod.rs](../../../../crates/stats/src/anova/sheet/mod.rs) (`6`), [nested.rs](../../../../crates/stats/src/anova/sheet/nested.rs) (`191`), [one_way.rs](../../../../crates/stats/src/anova/sheet/one_way.rs) (`1,673`)

Every file and line count matches `SEC-06.md`'s File Coverage Table with `1..=N` inspection and zero omitted files.

---

## Per-Finding Verdicts

### Verdict `FA-SEC06-01` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [run_one_way_anova](../../../../crates/stats/src/anova/sheet/one_way.rs#L53-L55) dispatches `OneWayMethod::RepeatedMeasures { .. } | OneWayMethod::Friedman` directly to [run_rm_one_way_anova_sheet](../../../../crates/stats/src/anova/sheet/one_way.rs#L742-L763), which calls [scan_rm_one_way_column_table](../../../../crates/stats/src/anova/sheet/one_way.rs#L224-L289) (`ds.cell(r, 0)`) without checking `col_tab.subcolumn_mode.is_pre_summarized()`. Workspace search confirms `crates/engine/src/cmd/analysis.rs` adds no `is_pre_summarized()` guard for `OneWayAnova`. Contrast `KruskalWallis` (`one_way.rs:128-130`), [extract_numeric_columns](../../../../crates/stats/src/common.rs#L297-L299), and [factorial.rs](../../../../crates/stats/src/anova/sheet/factorial.rs#L277-L279). Dynamic `probe_01_rm_and_friedman_accept_presummarized_column_table`: on a `ColumnTable` with `SubcolumnMode::MeanSdN`, `KruskalWallis` returned `Err(StatsError::RequiresRawReplicates)` while both `RepeatedMeasures` (`4` scalars) and `Friedman` (`3` scalars) returned `Ok(ResultSheet)` with `ResultStatus::UpToDate`, silently treating `Mean` cells as raw subject-block observations.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent wrong matched-subject / rank ANOVA outputs on pre-summarized `ColumnTable`s instead of `RequiresRawReplicates`).
- **Still present on main:** yes

### Verdict `FA-SEC06-02` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Both [extract_raw_optional_columns](../../../../crates/stats/src/multivariate.rs#L2391-L2407) (`MultipleImputation`) and [extract_optional_columns](../../../../crates/stats/src/multivariate.rs#L2754-L2783) (`Pca`, `CorrelationMatrix`, `Clustering`) read `subs.first().and_then(|c| c.active_number())` on `TableArchetype::Column` and `TableArchetype::Xy` without checking `subcolumn_mode.is_pre_summarized()` / `y_subcolumn_mode.is_pre_summarized()` or handling replicate subcolumns `1..k`. Engine `crates/engine/src/cmd/analysis.rs` adds no pre-summarized guard for `Multivariate`. Dynamic `probe_02_multivariate_drops_replicates_and_accepts_presummarized`: all four analyses (`Pca`, `CorrelationMatrix`, `Clustering`, `MultipleImputation`) returned `Ok` (`ResultStatus::UpToDate`) on both `ColumnTable(MeanSdN)` and `XyTable(MeanSdN)`, and on `XyTable(RawReplicates(3))` produced identical `scalars` and `tabs[0].rows` to a single-replicate `XyTable(Single)` (discarding replicates `1..=2`). *Precision note:* [ColumnTable::new_with_subcolumns](../../../../crates/doc/src/archetypes.rs#L592-L598) rejects `SubcolumnMode::RawReplicates(3)` at table construction with `Err(DocError::IncompatibleSubcolumnMode { archetype: Column, mode: RawReplicates(3) })`, so the `RawReplicates(k)` replicate-dropping defect applies to `XyTable`, whereas silent pre-summarized acceptance affects both `ColumnTable` and `XyTable`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent replicate data loss on `XyTable` and silent raw-observation misinterpretation of pre-summarized `ColumnTable`/`XyTable` inputs).
- **Still present on main:** yes

### Verdict `FA-SEC06-03` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Public [fit_factorial_anova](../../../../crates/stats/src/anova/factorial.rs#L114-L160) (re-exported at [anova/mod.rs](../../../../crates/stats/src/anova/mod.rs#L100)) accepts [FactorialObservations](../../../../crates/stats/src/anova/factorial.rs#L22-L29) (`pub factor_names`, `pub factor_levels`, `pub rows`) and validates only `n_obs < 3 || n_factors == 0` (`135`), leaving [kronecker_term_columns](../../../../crates/stats/src/anova/factorial.rs#L311-L326) to perform unchecked slice indexing on `factor_basis[f_idx]` (`314`), `f_rows[lvls[f_idx]][c_f]` (`321`), and `lvls[f_idx]`. Dynamic `probe_03_fit_factorial_anova_panics_on_malformed_observations` caught three distinct panics (`catch_unwind`): `factor_levels.len() != factor_names.len()` panicked at `factorial.rs:314:24` (`len is 1 but index is 1`), ragged row `lvls.len() < n_factors` panicked at `factorial.rs:321:123` (`len is 1 but index is 1`), and out-of-range level index panicked at `factorial.rs:321:118` (`len is 2 but index is 5`). (Internal callers in `anova/sheet/factorial.rs` construct dense valid indices.)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (unconditional release+debug out-of-bounds panics in public `curvebook_stats::anova::fit_factorial_anova` on malformed `FactorialObservations`, violating `no-panic`; scoped to direct Rust library callers).
- **Still present on main:** yes

### Verdict `FA-SEC06-04` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Public REML entrypoints ([anova/mod.rs](../../../../crates/stats/src/anova/mod.rs#L104-L105)) panic on malformed caller `(name, start, len)` term ranges: (1) [unstructured_reduce_design](../../../../crates/stats/src/anova/reml_unstructured.rs#L253-L259) computes `let end = start.saturating_add(len).min(p);` and slices `col_map[start..end]`, panicking when `start > p`; (2) [eval_reml_wald_terms_for_df](../../../../crates/stats/src/anova/reml.rs#L587-L591) checks `start + len > p` with unchecked addition, panicking on overflow in debug builds across [fit_random_intercept_reml](../../../../crates/stats/src/anova/reml.rs#L616-L622), [fit_multi_vc_reml](../../../../crates/stats/src/anova/reml.rs#L940-L946), and [fit_unstructured_rm_reml](../../../../crates/stats/src/anova/reml_unstructured.rs#L831-L837). Dynamic `probe_04_public_reml_entrypoints_panic_nuance` confirmed panics at `reml_unstructured.rs:256:41` (`range start index 4 out of range for slice of length 3`) and `reml.rs:588:24` (`attempt to add with overflow`) on `("T", usize::MAX, 1)` across all three public REML entrypoints. *Precision note:* [reml_unstructured.rs](../../../../crates/stats/src/anova/reml_unstructured.rs#L197-L201) short-circuits full-rank designs (`if qr.rank() == p { ... return Ok(...); }`) before reaching line `255`, so `fit_unstructured_rm_reml` with `("T", p + 1, 1)` returns `Ok` when `X` is full-rank (`probe_04` confirmed) and panics at line `256` only when `X` is rank-deficient (`qr.rank() < p`), while `("T", usize::MAX, 1)` still panics in `reml.rs:588` regardless of rank.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (release+debug slice panic in `unstructured_reduce_design` on rank-deficient `X` with `start > p` plus debug overflow panic in `eval_reml_wald_terms_for_df` across all public REML APIs, violating `no-panic`; internal sheet callers supply valid ranges).
- **Still present on main:** yes

### Verdict `FA-SEC06-05` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [sheet/factorial.rs](../../../../crates/stats/src/anova/sheet/factorial.rs#L636-L642) constructs full effect-coded `x_mat` (`build_reml_effect_design`). Whereas `ResidualCovariance::Unstructured` reduces collinear columns via `unstructured_reduce_design`, the default `ResidualCovariance::CompoundSymmetry` branches ([sheet/factorial.rs](../../../../crates/stats/src/anova/sheet/factorial.rs#L702-L767)) pass unreduced `x_mat` to [ri_eval_profile](../../../../crates/stats/src/anova/reml.rs#L271-L272) (`cholesky(&xtvx_mat)?`) and [multi_vc_eval_state](../../../../crates/stats/src/anova/reml.rs#L801-L802) (`cholesky(&xtvix)?`). Dynamic `probe_05_compound_symmetry_rm_reml_fails_on_all_missing_cell`: on a `2x2` `GroupedTable` with all `4` subjects missing cell `(row=1, col=1)` (all factor levels still observed), `CompoundSymmetry` failed with `Err(StatsError::Math(MathError::NotPositiveDefinite { .. }))` for both `ColumnFactorMatched` and `BothFactorsMatched`, while `ResidualCovariance::Unstructured` (`ColumnFactorMatched`) and `RepeatedMeasuresDesign::None` (OLS) succeeded (`ResultStatus::UpToDate`). (*Precision note:* the actual error variant in `curvebook_math` is `MathError::NotPositiveDefinite`, whereas `SEC-06.md` wrote `MathError::NonPositiveDefinite`.)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (clean error failure on collinear/all-missing-cell CS RM designs, with `Unstructured` / OLS workarounds where supported).
- **Still present on main:** yes

### Verdict `FA-SEC06-06` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [MultiVcRemlResult](../../../../crates/stats/src/anova/results.rs#L331-L344) contains no `converged` or boundary status fields (unlike `MixedRemlResult` at `#L321-L326` and `UnstructuredRemlResult` at `#L386-L390`). [fit_multi_vc_reml_with_method_and_core](../../../../crates/stats/src/anova/reml.rs#L1006-L1013) runs `for _ in 0..80 { let (converged, _) = multi_vc_ai_step(...)?; if converged { break; } }` and discards `converged` after loop exit. Consequently [emit_multi_vc_rm_reml](../../../../crates/stats/src/anova/sheet/one_way.rs#L365-L415) emits no convergence or boundary checklist/footnote (contrasting `emit_random_intercept_reml` and `emit_unstructured_rm_reml`). Dynamic `probe_06_multi_vc_reml_result_lacks_convergence_diagnostic` exhaustively destructured `MultiVcRemlResult`'s `6` fields (`variance_components`, `sigma2_error`, `reml_log_lik`, `fixed_terms`, `satterthwaite_df`, `kenward_roger_df`) and verified zero convergence checklist items or tab footnotes on a `BothFactorsMatched` `GroupedTable` fit.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (suppresses non-convergence diagnostics when the 80-iteration AI REML loop fails to converge).
- **Still present on main:** yes

### Verdict `FA-SEC06-07` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** (1) [extract_passing_bablok_pairs](../../../../crates/stats/src/regression.rs#L1811-L1835) rejects `t.y_subcolumn_mode.is_pre_summarized()` but reads only subcolumn `0` (`t.data_sets[col - 1].cell(row, 0)`) on `TableArchetype::Xy`, silently discarding replicates `1..k` under `RawReplicates(k)`; (2) [run_passing_bablok_analysis](../../../../crates/stats/src/regression.rs#L1940-L1991), [run_glm_analysis](../../../../crates/stats/src/regression.rs#L2438), and [run_multivariate_analysis](../../../../crates/stats/src/multivariate.rs#L1829) all return `Ok(out)` without invoking [sanitize_result_sheet_floats](../../../../crates/stats/src/common.rs#L494-L548). Dynamic `probe_07_pb_drops_xy_replicates_and_unsanitized_overflow` verified Passing–Bablok on `XyTable(RawReplicates(3))` produced `N = 5.0` and `Slope = 1.0` solely from subcolumn `0` (dropping `100*x` and `-100*x` in subcolumns `1..=2`), and on `1e307..1e308` inputs emitted `40` non-finite points (`y = +Inf` / non-finite CI bands) in `out.fitted_curves` instead of `Err(MathError::DomainError)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (silent replicate truncation on `XyTable` `RawReplicates` Passing–Bablok and unguarded non-finite curve points on extreme-scale inputs).
- **Still present on main:** yes

### Verdict `FA-SEC06-08` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [compute_vifs_from_matrix](../../../../crates/stats/src/regression.rs#L502-L538) regresses `target` (`col j`) on `retained_cols` without adding an intercept column when `has_intercept == false` (`start = usize::from(has_intercept)` at `#L516`), producing an uncentered `sol.rss`, then evaluates `r2 = (1.0 - sol.rss / tm.css).clamp(0.0, 0.999_999_999)` (`#L533`) against the **centered** `tm.css` from `SummaryMoments::from_slice(&target)`. Dynamic `probe_08_vif_uncentered_rss_vs_centered_css_without_intercept`: on near-collinear non-zero-mean predictors `x1 = 1..=6` and `x2 = 1007 - x1 + jitter`, `multiple_linear_regression(..., has_intercept: true, compute_vif: true)` reported `VIF = 1.579e6`, whereas `has_intercept: false` collapsed both VIFs to `(1.0, 1.0)`. Internal `run_glm_analysis` passes `has_intercept = true`, scoping impact to direct `multiple_linear_regression` library calls.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (wrong VIF `1.0` for collinear no-intercept designs in direct `multiple_linear_regression` calls).
- **Still present on main:** yes

### Verdict `FA-SEC06-09` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Repo-wide `rg` confirms every single one of the `46` `p_cell(..., p_fmt)` call sites listed in `SEC-06.md` uses fixed `0.05` star thresholds instead of `p_cell_at_alpha(..., p_fmt, confidence.alpha())`: `anova/sheet/one_way.rs` (`109, 111, 147, 315, 620, 681, 807, 808, 809, 816, 909, 1048, 1059, 1105, 1138, 1288`), `anova/sheet/factorial.rs` (`80, 183, 186, 187`), `anova/sheet/nested.rs` (`54, 61, 177`), `regression.rs` (`1898, 2022, 2043, 2094, 2127, 2259, 2361`), `survival.rs` (`1967, 1977, 1985, 2123, 2126, 2875, 2876, 2895, 2896`), and `multivariate.rs` (`1484, 1492, 2626`). Dynamic `probe_09_p_cell_ignores_confidence_alpha_in_sec06_sheets` (`confidence_level = 99%`, `alpha = 0.01`): `Linear` GLM (`glm-linear-params` row `X`) emitted `PValue { raw: 0.01614758248366501, stars: "*" }` in the **same row** as `ConfidenceInterval { lower: Finite(-0.07926430904343185), upper: Finite(1.4102166899958126) }` (which includes `0`); and `PassingBablok` emitted `Linearity assumption (Passing-Bablok CUSUM): status: Pass` (`P = 0.0212 >= 0.01`) in the same `ResultSheet` as summary row `PValue { raw: 0.021230667527873257, stars: "*" }`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity adjusted:** `S4` -> `S3`
- **Rationale for adjustment:** Consistent with `FA-SEC05-12` (`SEC-05`, settled `S3`), hardcoding `0.05` star thresholds inside sheets whose CIs and hypothesis decisions honor `alpha = 0.01` creates contradictory intra-row (`99% CI` containing zero alongside `stars: "*"`) and intra-sheet (`Pass` linearity alongside `stars: "*"`) statistical significance indicators whenever `confidence_level != 95%`.
- **Still present on main:** yes

### Verdict `FA-SEC06-10` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** (1) [nested_one_way_anova_with_reml_status](../../../../crates/stats/src/anova/nested.rs#L60-L66) checks `if two_group_t_test && g_count != 2` before `g_count < 2`, returning `StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: 3 }` when `g_count > 2` (`probe_10` confirmed); (2) [NestedTable::active_technical_replicates](../../../../crates/doc/src/archetypes.rs#L1761-L1766) drops non-numeric cells silently and [run_nested_analysis](../../../../crates/stats/src/anova/sheet/nested.rs#L110-L112) emits no ignored-text checklist/footnote disclosure (`probe_10` confirmed on `"ND"` text cell). *Precision note:* The third sub-claim that `sheet/nested.rs:110` skips `append_nested_multiplicity` when `!two_group_t_test && selected_groups.len() == 2` (`probe_10` observed tabs `["nested-anova", "variance-components"]`) actually matches the explicit API contract on [NestedAnalysisSpec::multiplicity](../../../../crates/doc/src/analysis.rs#L692) (`"when k >= 3 groups are analyzed"`), so that behavior reflects an intentional spec distinction from `sheet/one_way.rs:209` rather than a defect.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (misclassified error variant on `g_count > 2` and omitted non-numeric cell disclosure).
- **Still present on main:** yes

### Verdict `FA-SEC06-11` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Verified all four minor items: (1) [spearman_correlation](../../../../crates/stats/src/multivariate.rs#L298) and [kendall_tau_b](../../../../crates/stats/src/multivariate.rs#L324) emit `InsufficientSampleSize { context: "Pearson correlation", required: 3, actual: 2 }`; (2) [PassingBablokResult::linearity_passed](../../../../crates/stats/src/regression.rs#L1564) documents `cusum_p_value >= 0.05` while [passing_bablok_regression](../../../../crates/stats/src/regression.rs#L1756) evaluates `cusum_p_value >= conf.alpha()` (`probe_11`: `cusum_p_value = 0.02123` gave `linearity_passed = false` at `95%` and `true` at `99%`); (3) [CoxSubject::time](../../../../crates/stats/src/survival.rs#L774) documents `` `<= 0` subjects are rejected `` while [cox_ph_regression](../../../../crates/stats/src/survival.rs#L819) checks `s.time < 0.0` (`probe_11`: `time = 0.0` returned `Ok`); (4) `run_glm_analysis` and `run_passing_bablok_analysis` omit listwise-incomplete and ignored non-numeric cell checklist/footnote disclosures (`probe_11`: zero filtering checklists/footnotes on `"ND"` text inputs).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (cleanup / error-context / docstring / disclosure parity).
- **Still present on main:** yes
