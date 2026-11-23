# SEC-07 Verification Verdicts — Curve Fitting Core & Solver (`crates/fit`)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend-2` (independent `100%` line-by-line source inspection + `crates/engine`, `crates/doc`, `crates/chart`, and `crates/ui-egui` caller cross-checks + isolated `.lc-worktrees/ver-backend-2` dynamic probe `ver_sec07_probe`, cleaned up after execution)
- **Source Report:** [SEC-07.md](../../../docs/audit/full-2026-11-22/SEC-07.md) (`FA-SEC07-01`..`FA-SEC07-16`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 16 | `S1: 0`, `S2: 8`, `S3: 7`, `S4: 1` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **16** | **`S1=0, S2=8, S3=7, S4=1`** |

Dynamic verification run against an isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095` (removed cleanly afterward):
- Baseline suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-2 cargo test --manifest-path .lc-worktrees/ver-backend-2/Cargo.toml --locked --offline -p curvebook-fit`): `228 passed; 0 failed; 0 ignored` (`41` unit in `src/lib.rs` + `186` integration across `27` test binaries + `1` doc-test).
- Clippy check (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-2 cargo clippy --manifest-path .lc-worktrees/ver-backend-2/Cargo.toml --locked --offline -p curvebook-fit --all-targets`): exited `0` with zero warnings.
- Targeted reproducer suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-2 cargo test --manifest-path .lc-worktrees/ver-backend-2/Cargo.toml --locked --offline -p curvebook-fit --test ver_sec07_probe -- --nocapture`): `running 15 tests` -> `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s` (probing `FA-SEC07-01`..`FA-SEC07-15`; `FA-SEC07-16` verified statically).

Cross-reference and calibration notes:
- `FA-SEC07-09` is adjusted `S2 -> S3` after verifying `crates/fit/src/tex_format.rs` and `crates/chart/src/equation_box.rs`: (a) `execute_analysis` stores the canonical `model.id()` onto `FittedCurveOverlay::equation_id` (preferred by `equation_box.rs#L208`), limiting the missing `library/curves.rs` alias path to pre-fit modal previews (`fit_modal.rs#L714`), overlay-absent `equation_box.rs#L223`, or direct `format_model_equation_tex` calls; and (b) `escape_tex_fallback_braces` and `substitute_identifiers` only execute inside `fallback_custom_dsl_tex` / `fallback_lhs_tex` when DSL AST parsing fails (`render_lhs_rhs_dsl#L1443-L1449` and `#L1400`), whereas valid compiled custom DSL expressions (`Y = k_on_1 * X`, `Y = a ^ 2 * X`) take the AST formatter and render valid TeX (`Y = k_{\text{on,1}} \cdot X`, `Y = 3^{2} \cdot X`).
- Precision nuances identified during verification (without refuting any finding) are recorded on `FA-SEC07-02` (dynamic `probe_02` reproduced outer `15`-pass non-convergence returning `FitStatus::Converged` with `|M16 - M15| = 0.665` in `fit_cauchy_irls` and iteration/status overwrite under `max_iters = 1`; Poisson `dev > prev_dev` step acceptance and missing 12-pass `HitMaxIters` flag confirmed statically), `FA-SEC07-03` (`ResultSheet::lookup_scalar` matches `data_set_index.is_none()` only when queried with `None`, returning `None` for `Some(1)`, while `canvas_view.rs#L1355` matches `Some(idx) || is_none()`, feeding pooled `53.0` into both plots' equations), `FA-SEC07-13` (`crates/engine/src/cmd/analysis.rs#L1428-L1454` guards duplicates and out-of-range indices on command/DAG workflows, leaving direct `curvebook-fit` callers exposed), and `FA-SEC07-14` (`FittedDatasetResult::observed_x_range` explicitly documents `None` for plot-only evaluations in `solver/results.rs#L340-L343`, while hardcoded `r_squared: 1.0`, `sy_x: 0.0`, and `Ok` on zero finite evaluations contradict `NonlinearFitResult::r_squared` and non-zero `ss_residual`).

---

## File Coverage Completeness Check

Verified `100%` coverage against `SEC-07` scope at `7c85a8e516165ddadb101236a88eec937bf1f095` (`21` files, `15,025` total lines: `crates/fit/Cargo.toml` + `20` source files across `crates/fit/src/` and `crates/fit/src/solver/`, zero files omitted; `dsl.rs`, `dsl/**`, and `library/**` deferred to `SEC-08`):

- [Cargo.toml](../../../crates/fit/Cargo.toml) (`20`)
- [lib.rs](../../../crates/fit/src/lib.rs) (`270`), [error.rs](../../../crates/fit/src/error.rs) (`191`), [solver.rs](../../../crates/fit/src/solver.rs) (`79`), [types.rs](../../../crates/fit/src/types.rs) (`854`), [diagnostics.rs](../../../crates/fit/src/diagnostics.rs) (`1,949`), [linear.rs](../../../crates/fit/src/linear.rs) (`1,360`), [result.rs](../../../crates/fit/src/result.rs) (`2,128`), [tex_format.rs](../../../crates/fit/src/tex_format.rs) (`2,658`)
- `crates/fit/src/solver/`: [results.rs](../../../crates/fit/src/solver/results.rs) (`539`), [layout.rs](../../../crates/fit/src/solver/layout.rs) (`438`), [lm.rs](../../../crates/fit/src/solver/lm.rs) (`1,125`), [irls.rs](../../../crates/fit/src/solver/irls.rs) (`301`), [fit.rs](../../../crates/fit/src/solver/fit.rs) (`438`), [datasets.rs](../../../crates/fit/src/solver/datasets.rs) (`532`), [assembly.rs](../../../crates/fit/src/solver/assembly.rs) (`298`), [comparison.rs](../../../crates/fit/src/solver/comparison.rs) (`193`), [identifiability.rs](../../../crates/fit/src/solver/identifiability.rs) (`246`), [inference.rs](../../../crates/fit/src/solver/inference.rs) (`433`), [curves.rs](../../../crates/fit/src/solver/curves.rs) (`681`), [profile.rs](../../../crates/fit/src/solver/profile.rs) (`292`)

Every file and line count matches `SEC-07.md`'s File Coverage Table with `1..=N` inspection and zero omitted files.

---

## Per-Finding Verdicts

### Verdict `FA-SEC07-01` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [finalize_fit_uncertainty](../../../crates/fit/src/solver/fit.rs#L432-L436) leaves `identifiability` as `FitIdentifiability::NotAssessed` whenever `lm_out.status == FitStatus::Diverged`, [fit_independent_datasets](../../../crates/fit/src/solver/datasets.rs#L406) merges per-dataset assessments via `identifiability::include_dataset` (ignoring `NotAssessed` whenever another dataset is `Identifiable`), and [execute_nonlinear_analysis](../../../crates/fit/src/result.rs#L455-L496) sets `result.status = ResultStatus::UpToDate` (`#L495`) while [append_nonlinear_checklist](../../../crates/fit/src/result.rs#L813-L857) checks neither `FitStatus::Diverged` nor `FitStatus::HitMaxIters`. Dynamic `probe_01_diverged_hitmaxiters_result_status_and_checklist`: (1) step DSL fit (`fit.status == FitStatus::Diverged`, `cov.ambiguous == false`, `fit.identifiability == NotAssessed`) returned `ResultStatus::UpToDate` and emitted `ChecklistItem { assumption: "Parameter identifiability", status: Pass, explanation: "All free parameters are cleanly identifiable." }`; (2) 4PL fit with `max_iters = 1` (`HitMaxIters`) returned `ResultStatus::UpToDate`; (3) multi-dataset `<A>/<B>` DSL (`DS0` convergent, `DS1` diverged) returned `fit.status == FitStatus::Diverged` with `fit.identifiability == FitIdentifiability::Identifiable`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (non-converged fits marked `UpToDate` without checklist warning/violation, and `Diverged` fits explicitly claim `Pass` identifiability).
- **Still present on main:** yes

### Verdict `FA-SEC07-02` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** In [fit_poisson_irls](../../../crates/fit/src/solver/irls.rs#L120-L133), the `for _ in 0..12` outer loop unconditionally executes `last_out = next; prev_dev = dev;` (`#L128-L129`) without tracking outer convergence (`rel_change < 1e-10`), setting `FitStatus::HitMaxIters` if `12` passes exhaust, accumulating `iterations`, or step-halving when `dev > prev_dev` (contrast [solve_profile_subproblem_poisson](../../../crates/fit/src/solver/profile.rs#L247-L289)). Likewise [run_cauchy_irls_loop](../../../crates/fit/src/solver/irls.rs#L199-L218) loops `0..15`, overwrites `last_out = run_trust_region_lm(...)`, and discards prior iteration counts without setting `HitMaxIters` when `max_step >= 1e-8` at pass `15`. Dynamic `probe_02_irls_poisson_and_cauchy_outer_loop`: on `[0.0, 0.1, 0.2, 1.5, 2.0, 5.0, 6.0]` with `MeanLocModel`, `fit_cauchy_irls` exited pass `15` returning `status = FitStatus::Converged` and `iterations = 1` (`M15 = 1.058229`), even though the very next Cauchy IRLS step had magnitude `|M16 - M15| = 0.665` (`>> 1e-8`). Poisson `dev > prev_dev` acceptance and 12-pass flag omission are confirmed statically (`irls.rs#L120-L133`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (outer IRLS non-convergence masked as `FitStatus::Converged` with truncated iteration counts, and missing Poisson deviance monotonicity guard).
- **Still present on main:** yes

### Verdict `FA-SEC07-03` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [fit_nonlinear](../../../crates/fit/src/solver/fit.rs#L92-L104) constructs `global_layout` using `all_points`, [solve_single_dataset_unit](../../../crates/fit/src/solver/datasets.rs#L184-L185) fits each curve with per-dataset `ds_layout` (`default_parameter_rules(&ds.points)`), yet [fit_independent_datasets](../../../crates/fit/src/solver/datasets.rs#L518) returns `fixed_constants: global_layout.fixed_constants`. [append_nonlinear_parameter_tab](../../../crates/fit/src/result.rs#L594-L615) then emits `(name, data_set_index: None, pooled_val)` into `result.scalars` and inserts `(lower, None)` into `emitted_scalars`, blocking per-dataset constant emission at `#L610`. Dynamic `probe_03_per_dataset_default_parameter_rules_xcenter`: fitting `centered_poly_2` to `LowX` (`X = 1..=5`, mean `3.0`) and `HighX` (`X = 101..=105`, mean `103.0`) solved `ds_results[0].params = [10, 2, 0.5, 3.0]` and `ds_results[1].params = [20, 3, 1.5, 103.0]`, yet reported `fit.fixed_constants = [("XCenter", 53.0, [Constant])]`, a single `"XCenter (Constant)" = 53.0` row, and `result.scalars` containing only `("XCenter", None, 53.0)` (leaving `lookup_scalar("XCenter", Some(1))` as `None` via [ResultSheet::lookup_scalar](../../../crates/doc/src/result.rs#L549-L559) while [canvas_view.rs](../../../crates/ui-egui/src/egui_shell/canvas_view.rs#L1355-L1366) / [equation_box.rs](../../../crates/chart/src/equation_box.rs#L223) match `data_set_index.is_none()` and substitute `53.0` into both curves' equations).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (wrong constant value reported on multi-dataset fits with dataset-dependent `default_parameter_rules`, dropped per-dataset scalar lookups, and corrupted chart equation substitution).
- **Still present on main:** yes

### Verdict `FA-SEC07-04` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [nonlinear_uses_weighted_objective](../../../crates/fit/src/result.rs#L556-L560) checks `(fit.weighted_ss - fit.rss).abs() > 1e-12 * fit.rss.abs().max(1.0)` without checking `!fit.is_robust_cauchy()`, triggering weighted-objective UI output in `#L632-L635`, `#L689-L695`, and `#L758-L760` whenever Cauchy IRLS sets `fit.weighted_ss != fit.rss`. Dynamic `probe_04_robust_cauchy_weighted_ss_false_row_and_footnote`: `execute_analysis` with `FitMethod::RobustCauchy` (`WeightingScheme::None`) emitted `"Weighted Sum of Squares"` (`32.355010`) on `nonlinear_gof`, scalar `"Weighted SS" = 32.355009965817565`, `, Weighted SS = 32.355010` in the `nonlinear_parameters` footnote, and the `nonlinear_gof` footnote `"parameter SEs, confidence intervals, AICc/BIC, and model comparisons use the weighted / deviance objective"`, contradicting `NotApplicableInRobustMode`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (misleading objective value and directly false inference footnote on robust fits).
- **Still present on main:** yes

### Verdict `FA-SEC07-05` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [uses_replicate_summary_moments](../../../crates/fit/src/solver/layout.rs#L294-L296) returns `config.method == FitMethod::LeastSquares && config.weighting == WeightingScheme::None`, so in [build_observations](../../../crates/fit/src/solver/layout.rs#L401-L409) `honor_rep_moments` is `false` for all non-`None` schemes (`InverseX`, `InverseX2`, `RelativeSquare`, `Relative`, `GeneralPowerY`, in addition to intentionally row-mean-weighted `ObservedSd` tested at [r46_solver_oracles.rs](../../../crates/fit/tests/r46_solver_oracles.rs#L245-L274)), falling back to `rep_n = 1, ss_within = 0.0`. Dynamic `probe_05_mean_sd_n_with_non_none_weighting_discards_n_and_sd`: fitting `michaelis_menten` under `WeightingScheme::InverseX` on identical `5`-row, `N = 5`, `SD = 2.0` data produced `n_obs = 5, df = 3, rss = 16.2460, se = 13.2337` on `MeanSdN` vs `n_obs = 25, df = 23, rss = 161.2300, se = 9.6489` on equivalent `RawReplicates(5)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silently drops `N - 1` degrees of freedom and within-row `SD` variance across weighted pre-summarized fits).
- **Still present on main:** yes

### Verdict `FA-SEC07-06` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [compare_models_checked](../../../crates/fit/src/solver/comparison.rs#L93-L165) and [compare_models](../../../crates/fit/src/solver/comparison.rs#L171-L193) check `n_obs`, `n_outliers_excluded`, `RobustCauchy`, and `PoissonDeviance` compatibility, but never reject or warn on `FitStatus::Diverged` / `FitStatus::HitMaxIters` in `primary` or `alt`; [compare_models_f_test](../../../crates/fit/src/diagnostics.rs#L1036) and [compare_models_lrt](../../../crates/fit/src/diagnostics.rs#L1074) clamp negative `(ss_simple - ss_complex)` / `ln(ss_null / ss_alt)` to `0.0`. Dynamic `probe_06_compare_models_non_converged_and_clamped_zero`: comparing `HitMaxIters` 3-parameter `prim_fit` (`rss = 4.836e-13`) against converged 2-parameter linear `alt` (`rss = 0.0`) returned `Ok` with `InferenceAvailability::Available` and `ExtraSumOfSquaresResult { f_stat: 0.0, df_num: 1, df_den: 3, p_value: 1.0, preferred_complex_model: false }`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silently compares unconverged/diverged models and reports valid-looking clamped model-preference decisions).
- **Still present on main:** yes

### Verdict `FA-SEC07-07` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [resolve_mv_columns](../../../crates/fit/src/types.rs#L591-L667) checks `< n_vars` bounds only, without checking disjointness among `mv_x_columns`, `mv_y_column`, and `mv_group_columns`, and falls back (`#L627, #L638`) to `VariableRole::Categorical` or `VariableRole::Label` columns when no distinct numeric column remains; upstream [validate_nl_mv_group_selection](../../../crates/engine/src/cmd/analysis.rs#L1457-L1523) enforces bounds only without checking disjointness either. Dynamic `probe_07_mv_column_collisions_and_role_fallback`: colliding `group_columns = &[0], x_columns = &[0], y_column = Some(0)` extracted `4` series with `x = y = group` (`column_title_value: Some(1.0)..Some(4.0)`), and auto-resolving a `[Continuous, Categorical]` table fell back to the `Categorical` column and errored with `EmptyInput` instead of rejecting missing numeric predictor.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (allows nonsensical `Y = f(Y)` / self-grouped fits and `Categorical`/`Label` predictor selection without upstream validation).
- **Still present on main:** yes

### Verdict `FA-SEC07-08` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [execute_linear_analysis](../../../crates/fit/src/result.rs#L91) passes `count_replicates = !lin_spec.deming_model_ii` (`false` for Deming) to `prepare_linear_points`, and [build_deming_result_sheet](../../../crates/fit/src/result.rs#L306-L366) neither rejects `ds.is_pre_summarized` nor forwards `N`/`SD`, while [deming_regression](../../../crates/fit/src/linear.rs#L1116-L1160) runs `fit_deming_core` (`#L1218-L1222`, `vertical_axis` check `0.0 >= 0.0`) inside every leave-one-out jackknife subset. Dynamic `probe_08_deming_presummarized_and_n3_duplicate`: (1) `execute_analysis` with `deming_model_ii: true` on `SubcolumnMode::MeanSdN` (`4` rows, `SD = 0.5, N = 10`) succeeded with `ResultStatus::UpToDate`, silently ignoring `SD` and `N`; (2) `deming_regression` on valid non-collinear `[(1.0, 1.0), (2.0, 2.0), (2.0, 2.0)]` returned `Err(FitError::InvalidParameter { name: "data", reason: "zero X-Y covariance with vertical Deming axis" })` on LOO `skip = 0`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent misanalysis on pre-summarized Deming tables and unwarranted failure on valid `n = 3` replicate pairs).
- **Still present on main:** yes

### Verdict `FA-SEC07-09` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [format_line_poly_curve_tex](../../../crates/fit/src/tex_format.rs#L940-L955) omits `library/curves.rs#L179-L206` aliases (`centered_polynomial_1..=6`, `semilog_line`, `loglog_line`), routing them to [format_unknown_fallback](../../../crates/fit/src/tex_format.rs#L247), where [contains_tex_meta](../../../crates/fit/src/tex_format.rs#L2298-L2300) excludes `'_'`; on unparseable custom DSL fallback (`render_lhs_rhs_dsl#L1400, #L1443-L1449`), [escape_tex_fallback_braces](../../../crates/fit/src/tex_format.rs#L2123-L2135) leaves `'_'` unescaped and [substitute_identifiers](../../../crates/fit/src/tex_format.rs#L2179) checks `chars.get(i) == Some(&'^')` without skipping whitespace. Dynamic `probe_09_tex_aliases_underscore_and_spaces`: `format_model_equation_tex` returned `"Y = f_{centered_polynomial_2}(X)"`, `"Y = f_{semilog_line}(X)"`, and `"Y = f_{loglog_line}(X)"`, and unparseable fallback DSL `"Y = k_on_1 * [X"` / `"Y = a ^ 2 * [X"` (`a = -3.0`) emitted `"Y = k_on_1  \\cdot  [X"` and `"Y = -3 ^ 2  \\cdot  [X"` (missing `(-3)` parens). *Calibration & precision note:* Post-fit [FittedCurveOverlay](../../../crates/doc/src/result.rs#L393-L404) carries canonical `model.id()` (used by [equation_box.rs](../../../crates/chart/src/equation_box.rs#L208,L223) when an overlay exists), and valid compiled custom DSL expressions (`"Y = k_on_1 * X"`, `"Y = a ^ 2 * X"`) go through the AST formatter cleanly (`"Y = k_{\\text{on,1}} \\cdot X"`, `"Y = 3^{2} \\cdot X"`), restricting the fallback bugs to invalid/incomplete DSL strings and restricting the alias bug to direct `format_model_equation_tex` callers / pre-fit previews (`fit_modal.rs#L714`, overlay-absent `equation_box.rs#L223`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity adjusted:** `S2 -> S3` (valid compiled DSL equations and post-fit canonical overlays render cleanly; defect affects unparseable DSL fallbacks and pre-fit/direct non-canonical `curves.rs` alias formatting).
- **Still present on main:** yes

### Verdict `FA-SEC07-10` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** In [compute_jacobian](../../../crates/fit/src/solver/lm.rs#L192-L231), the one-sided finite-difference fallback (`#L217`) picks `sign = if ub.is_some_and(|u| xj + h_f > u) { -1.0 } else { 1.0 }` without testing whether `+h_f` returns `None` (`!can_plus && can_minus`) or retrying `-h_f`. Dynamic `probe_10_compute_jacobian_one_sided_domain_edge`: an unbounded model (`default_bounds = [(None, None), (None, None)]`) valid for `A <= 2.0` (`None` for `A > 2.0`) starting at `A = 2.0` failed immediately with `Err(FitError::BadInitialValues { reason: "Jacobian evaluation returned non-finite values" })`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (prematurely aborts gradient-less fits at implicit upper domain boundaries even when `-h` is finite).
- **Still present on main:** yes

### Verdict `FA-SEC07-11` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [delta_grad_at_x_into](../../../crates/fit/src/solver/curves.rs#L184-L217) computes central differences `theta[slot] +/- h` without respecting `[lower, upper]` or trying one-sided steps, leaving `g_out[slot]` as `0.0` whenever either side returns `None`. Dynamic `probe_11_delta_grad_at_x_into_silently_zeroes_one_sided_band`: `LeftBoundModel` (`A >= 0.0`, `y = 10*A*x + B`, no analytical `grad`) converged to `A = 0.0` with non-zero SE (`se_A = 5.878e-4, cov_00 = 3.455e-7`), yet `ci_upper - ci_lower` stayed flat at `0.385180895` across `x = 1..=20` because `g[0]` was zeroed at the lower bound.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (silently narrows confidence/prediction bands and unknown intervals at domain edges for finite-difference models).
- **Still present on main:** yes

### Verdict `FA-SEC07-12` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Inside [populate_xy_column](../../../crates/fit/src/types.rs#L424-L436), the `xy.y_subcolumn_mode.is_pre_summarized()` branch (`MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`) pushes paired `series.points` only and omits pushing `unknowns_y` and `unknowns_x`. Dynamic `probe_12_presummarized_mean_low_high_drops_unknowns`: an `XyTable(SubcolumnMode::MeanLowHigh)` containing paired rows plus unpaired `(2.5, empty)` and `(empty, 25.0)` extracted `unknowns_x = []` and `unknowns_y = []`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (unknown-row interpolation silently yields empty output on `MeanLowHigh`/`UpperLowerLimits`/`MedianIqr` XY tables).
- **Still present on main:** yes

### Verdict `FA-SEC07-13` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [resolve_selected_indices](../../../crates/fit/src/types.rs#L339-L341) filters `i < total` without erroring on out-of-range indices and without deduplicating repeated indices when used in `Xy` (`#L344`) and `Grouped` (`#L476`) extraction (contrast `extract_from_multivar` at `#L749` and upstream DAG guard [validate_selected_data_sets_for_sheet](../../../crates/engine/src/cmd/analysis.rs#L1428-L1454)). Dynamic `probe_13_selected_data_sets_duplication_and_oob_drop`: `extract_datasets_from_sheet(&sheet, &[0, 0, 99])` on a 2-column `XyTable` returned `2` duplicate `DatasetSeries` entries both with `data_set_index = 0` and silently dropped `99`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (`engine` commands validate `selected_data_sets` upfront, exposing direct `curvebook-fit` callers only).
- **Still present on main:** yes

### Verdict `FA-SEC07-14` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [fit_plot_function_only](../../../crates/fit/src/solver/fit.rs#L108-L255) hardcodes `observed_x_range: None`, `r_squared: 1.0`, `sy_x: if is_robust { f64::NAN } else { 0.0 }`, and `studentized_residuals: vec![0.0; ds_res.len()]` per dataset even when points exist with non-zero residuals, and succeeds with `FitStatus::Converged` and `n_obs = 0` when all points evaluate to `None`. Dynamic `probe_14_plot_function_only_artifacts_and_all_none_eval`: (1) `fit_nonlinear` with `plot_function_only = true` on noisy data yielded `datasets[0].observed_x_range = None`, `datasets[0].r_squared = 1.0`, `datasets[0].sy_x = 0.0`, and `studentized_residuals = [0.0, 0.0, 0.0]`, while `execute_analysis` on the same table computed top-level scalar `R2 = 0.072451` (exposing the contradiction); (2) non-empty negative-X data with `Y = A * ln(X)` (`all eval -> None`) returned `Ok((FitStatus::Converged, n_obs = 0, datasets[0].r_squared = 1.0))`. *Precision note:* [FittedDatasetResult::observed_x_range](../../../crates/fit/src/solver/results.rs#L340-L343) explicitly documents `None` for plot-only evaluations, whereas hardcoded `r_squared: 1.0` / `sy_x: 0.0` alongside non-zero `ss_residual` and `Ok` on zero evaluated points are bugs.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (hardcoded per-dataset GOF placeholders and silent success when all evaluations fail).
- **Still present on main:** yes

### Verdict `FA-SEC07-15` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Static and dynamic checks verify all four doc-vs-code mismatches: (1) [SimpleLinearResult::r](../../../crates/fit/src/linear.rs#L249) documents `NaN` when `force_through_origin`, whereas [fit_linear_points](../../../crates/fit/src/linear.rs#L523) returns uncentered cosine `0.999592` (`probe_15`); (2) [SimpleLinearResult::x_intercept](../../../crates/fit/src/linear.rs#L261) documents `None` when `force_through_origin`, whereas `#L585` returns `Some(-0.0)`; (3) [LinearComparisonResult](../../../crates/fit/src/linear.rs#L296-L302) documents `None` for intercept-comparison fields when `slopes_significantly_different`, whereas [compare_linear_lines](../../../crates/fit/src/linear.rs#L1075-L1102) always populates `Some(...)` (and [execute_linear_analysis](../../../crates/fit/src/result.rs#L226-L231) displays `"Are intercepts equal?"` whenever `Some`, though [s3a_fit_x2_regressions.rs](../../../crates/fit/tests/s3a_fit_x2_regressions.rs#L362) only exercises equal slopes); (4) [compute_information_criteria](../../../crates/fit/src/diagnostics.rs#L1110-L1122) omits `!ss.is_finite() || ss < 0.0` guards (`(ss / n_f).max(1e-300)` returns `Ok` with `-6897.76` for both `ss = -1.0` and `ss = NaN`). Dynamic `probe_15_linear_and_ic_divergences` confirmed all four behaviors (`p_slopes = 1.185e-12, slopes_significantly_different = true` still returned `f_intercepts = Some(25.00), p_intercepts = Some(0.004105)`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (API contract drift and missing negative/`NaN` `ss` validation).
- **Still present on main:** yes

### Verdict `FA-SEC07-16` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Confirmed statically at all four locations: unused `let _ = x_sd;` closure in [populate_xy_column](../../../crates/fit/src/types.rs#L384-L393); dead `!has_ode && false` at [tex_format.rs](../../../crates/fit/src/tex_format.rs#L1133) and dead `"\\sqrt{"` entry at `#L2150` overridden by `replace_ident_prefix` at `#L2195-L2199`; and duplicated Benjamini–Hochberg top-30% ROUT routine across [rout_detect_outliers](../../../crates/fit/src/diagnostics.rs#L1806-L1901) vs [rout_detect_outliers_mb2006](../../../crates/fit/src/solver/inference.rs#L278-L369).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (dead code and duplicated ROUT helper with minor scale-floor / error-propagation divergence).
- **Still present on main:** yes
