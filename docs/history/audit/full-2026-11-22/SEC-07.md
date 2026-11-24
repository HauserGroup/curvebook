# SEC-07 Audit Report — Curve Fitting Core & Solver (`crates/fit`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `SEC-07` (`crates/fit/Cargo.toml` + `20` `.rs` files in `crates/fit/src/` and `crates/fit/src/solver/`: `diagnostics.rs`, `error.rs`, `lib.rs`, `linear.rs`, `result.rs`, `solver.rs`, `tex_format.rs`, `types.rs`, and `solver/{assembly,comparison,curves,datasets,fit,identifiability,inference,irls,layout,lm,profile,results}.rs`; `15,025` total lines: `15,005` LOC across `.rs` files + `20` lines in manifest; `dsl.rs`, `dsl/**`, and `library/**` deferred to `SEC-08`)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-fit` (report-only static audit + read-only test verification; zero tracked files edited)
- **Summary Counts:** `S1: 0` · `S2: 9` · `S3: 6` · `S4: 1` (Total: `16` findings)
- **Prior Audit Check (`docs/audit/local-code-audit-2026-11-18.md`):**
  - Closed findings relevant to `SEC-07` (`#10` non-moment pre-summarized central extraction, `#11`/`#115` Poisson domain guards & rescue, `#12` equal inclusive bounds as `Fixed`, `#80` ROUT `Q` validation, `#114` unknown/duplicate `parameter_rules` + alt-model rule filtering, `#116` scaled SVD + subspace VIF identifiability, `#121` column-equilibrated trust-region QR & profile root verification): **0 regressions** observed across `SEC-07`.
  - Known open finding **`#127`** (default `"log_agonist_vs_response_4pl"` equation selection in `crates/ui-egui/src/dialogs_canvas.rs` and `crates/engine/src/cmd/analysis.rs` without checking linear-vs-log X scale) lies outside `SEC-07` file scope (`SEC-08` / UI / engine) and is not duplicated here.

## Baseline Verification Summary

- `cargo test --locked --offline -p curvebook-fit --lib`: exited `0` (`41 passed; 0 failed; 0 ignored`).
- `cargo test --locked --offline -p curvebook-fit --tests`: exited `0` (`227 passed; 0 failed; 0 ignored` across `src/lib.rs` unit binary `41` + `27` integration test binaries `186`; note `tests/nist_strd.rs` runs `1` test without `CURVEBOOK_REQUIRE_CORPUS=1` set).
- `cargo check --target wasm32-unknown-unknown --locked --offline -p curvebook-fit`: blocked locally because the `wasm32-unknown-unknown` `std`/`core` target sysroot is not installed (`E0463`); `wasm32` cleanliness (`#![forbid(unsafe_code)]`, `[lints] workspace = true`, and zero `unsafe`, `std::fs`, `std::net`, `std::process`, `std::thread`, or `SystemTime` across all `20` `SEC-07` `.rs` files) was verified by source inspection and `rg` scan.

---

## File Coverage Table

| # | File | Lines | Inspected | Concrete Invariants & Dimensions Verified | Findings / Status |
|---|---|---:|---|---|---|
| 1 | `crates/fit/Cargo.toml` | 20 | `1..=20` | `[lints] workspace = true`; runtime deps strictly `curvebook-math`, `curvebook-doc`, `serde`; zero OS/network/`chrono` deps (`wasm32`-clean). | Clean — manifest & workspace lints verified |
| 2 | `crates/fit/src/lib.rs` | 270 | `1..=270` | `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`; public facade exports; zero production `unwrap`/`expect`/`panic!`. | Clean — lint attributes & re-export surface verified |
| 3 | `crates/fit/src/error.rs` | 191 | `1..=191` | Structured `FitStatus`, `FitIdentifiability`, `ParameterNote`, and `FitError` variants + `Display`/`Error` impls; zero panics or OS dependencies. | Clean — structured error & status surface verified |
| 4 | `crates/fit/src/solver.rs` | 79 | `1..=79` | `D1` literature citations (Levenberg, Marquardt, Moré, Nelder–Mead, Lagarias, Nocedal–Wright, Bates–Watts, Seber–Wild, Green, McCullagh–Nelder, Motulsky–Brown, Holland–Welsch, Brent); module re-exports. | Clean — citations & module facade verified |
| 5 | `crates/fit/src/types.rs` | 854 | `1..=854` | `DataPoint`, `validated_replicate_moments` (`N in 1..=1e9`, non-negative finite `sd`), `HookContext` cycle guard, `Xy`/`Grouped`/`MultipleVariables` extraction, `filter_datasets_by_range`; `resolve_mv_columns` overlap/role fallback, `populate_xy_column` non-moment unknowns drop, `resolve_selected_indices` duplicates/silent drop, unused `x_sd`. | `FA-SEC07-07`, `FA-SEC07-12`, `FA-SEC07-13`, `FA-SEC07-16` |
| 6 | `crates/fit/src/diagnostics.rs` | 1949 | `1..=1949` | `D1` citations, `compute_covariance_diagnostics`, Hougaard skewness tensor, Venzon–Moolgavkar profile CIs + bracket verification (`#121`), `compare_models_{f_test,lrt,aicc}`, runs & stratified runs PMF, `replicates_lack_of_fit`, normality suite (Shapiro–Wilk, D'Agostino–Pearson, Anderson–Darling, Lilliefors KS), Spearman homoscedasticity, `compute_rsdr`; missing `ss >= 0`/finite guard in `compute_information_criteria`, negative-diff clamping in `F`/LRT, duplicated `rout_detect_outliers`. | `FA-SEC07-06`, `FA-SEC07-15`, `FA-SEC07-16` |
| 7 | `crates/fit/src/linear.rs` | 1360 | `1..=1360` | `D1` citations, `fit_ols_core` (raw + `Mean/SD/N` moments, origin-forced, `S_xx == 0` guard), Fieller all-regime inverse quadratic calibration, ANCOVA line comparison, Model II Deming closed-form & extreme-lambda stability; Deming ignores pre-summarized moments & fails on `n == 3` duplicate point, `r` / `x_intercept` / `f_intercepts` docstring-code drift. | `FA-SEC07-08`, `FA-SEC07-15` |
| 8 | `crates/fit/src/result.rs` | 2128 | `1..=2128` | `execute_analysis` dispatch, `execute_linear_analysis`, `build_deming_result_sheet`, `execute_nonlinear_analysis`, `ResultTab` / scalar / ROUT `SourceCellId` assembly, trapezoidal & fitted AUC; non-convergence (`Diverged`/`HitMaxIters`) masked by `UpToDate` & `Pass` identifiability, `RobustCauchy` weighted-SS misclassification, pooled-vs-independent constant scalar drop, Deming pre-summarized handling. | `FA-SEC07-01`, `FA-SEC07-03`, `FA-SEC07-04`, `FA-SEC07-08` |
| 9 | `crates/fit/src/tex_format.rs` | 2658 | `1..=2658` | `format_number_sig_figs`, polynomial/built-in templates, iterative stackless DSL parser/inliner/renderer, sign normalization; unescaped `_` double subscripts in `format_unknown_fallback` & fallback DSL, ` ^ ` whitespace base bug, missing `curves.rs` aliases, dead `!has_ode && false` / `"\\sqrt{"` cleanup. | `FA-SEC07-09`, `FA-SEC07-16` |
| 10 | `crates/fit/src/solver/results.rs` | 539 | `1..=539` | `SolverConfig` defaults, NaN-safe serde compat deserializers, `InferenceAvailability` helpers, typed `FittedParameterSummary` / `InterpolatedUnknown` / `NonlinearFitResult` contracts. | Clean — serde guards & robust-suppression contract verified |
| 11 | `crates/fit/src/solver/layout.rs` | 438 | `1..=438` | `ParameterLayout::build` (`#114` unknown/duplicate rule checks, `#12` equal bounds as `Fixed`, `PerDatasetConstant`/`ColumnTitleConstant`/`HookedConstant`), `compute_point_weight` (`0`/non-finite checks); `uses_replicate_summary_moments` drops `N` and `ss_within` under all non-`None` weightings. | `FA-SEC07-05` |
| 12 | `crates/fit/src/solver/lm.rs` | 1125 | `1..=1125` | Sweep-ordered ODE eval, Moré trust-region LM (`ftol`/`xtol`/`gtol`, `#121` column-equilibrated QR, active-face bound projection, `stalled_lm_status`), Nelder–Mead simplex rescue & Poisson feasibility barrier; `compute_jacobian` one-sided fallback ignores `-h` when `+h` hits implicit domain `None`. | `FA-SEC07-10` |
| 13 | `crates/fit/src/solver/irls.rs` | 301 | `1..=301` | `assign_poisson_irls_weights` (`mu > 0`), `poisson_deviance` via `deviance_bd0`, `cauchy_marginal_nll`, `compute_unscaled_cov`; outer IRLS loops (`12`/`15`) swallow outer non-convergence, overwrite `iterations`, and omit Poisson deviance monotonicity guard. | `FA-SEC07-02` |
| 14 | `crates/fit/src/solver/fit.rs` | 438 | `1..=438` | `fit_nonlinear` validation (ROUT `Q in (0, 50]`, `RequiresRawReplicates`, Poisson weighting exclusivity, range filter, ODE step budget), `solve_lm_by_method`, `finalize_fit_uncertainty`; `fit_plot_function_only` zero-point & hardcoded `r_squared: 1.0` / `sy_x: 0.0`, `Diverged` skipping `identifiability::assess`. | `FA-SEC07-01`, `FA-SEC07-14` |
| 15 | `crates/fit/src/solver/datasets.rs` | 532 | `1..=532` | `fit_single_or_shared`, `solve_single_dataset_unit`, `fit_independent_datasets` (per-curve ROUT pooling, global covariance block assembly, combined status/TSS/R²/AICc); `fixed_constants: global_layout.fixed_constants` ignores per-dataset `default_parameter_rules`. | `FA-SEC07-03` |
| 16 | `crates/fit/src/solver/assembly.rs` | 298 | `1..=298` | `build_fitted_parameter_summaries`, `assemble_dataset_results` (`k_eff_d` shared df allocation for per-dataset `Sy.x`, `RobustCauchy` inference suppression, curve overlay & unknown interpolation wiring, per-dataset runs test). | Clean — parameter bounds/notes & per-dataset `k_eff_d` math verified |
| 17 | `crates/fit/src/solver/comparison.rs` | 193 | `1..=193` | `ordered_nested_roles`, `filter_config_rules_for_alt_model` (`#114`), `compare_models` / `compare_models_checked` (`RobustCauchy`, Poisson vs Gaussian error model separation, ROUT outlier-index parity); missing convergence-status guard on `primary` / `alt`. | `FA-SEC07-06` |
| 18 | `crates/fit/src/solver/identifiability.rs` | 246 | `1..=246` | `#116` sparse distinct-X check (`-0.0 == +0.0`), column-scaled SVD reciprocal condition number + nullspace permutation attribution, `dependency > 1 - 1e-4`, open non-bounded profile CI detection, multi-dataset reason tagging. | Clean — scaled conditioning, permutation map & unit tests verified |
| 19 | `crates/fit/src/solver/inference.rs` | 433 | `1..=433` | `wald_ci_bounds`, `compute_studentized_residuals` (`1 - h_ii` clamp & `k <= 64`), `poisson_log_likelihood_from_obs`, `poisson_information_criteria` (`K = k_free`), `replicates_lack_of_fit_obs` (moments + weights + deviance), `rout_detect_outliers_mb2006` (top-30% BH FDR), residual diagnostics bundle. | Clean — studentized leverage, Poisson IC, weighted/Poisson LOF & ROUT math verified |
| 20 | `crates/fit/src/solver/curves.rs` | 681 | `1..=681` | `new_observation_inv_weight`, `build_curve_overlay`, `interpolate_x_from_y` (Brent + calibration vs mean-response intervals, extrapolation/plateau notes), `interpolate_y_from_x`; `delta_grad_at_x_into` lacks bound checks / one-sided fallback. | `FA-SEC07-11` |
| 21 | `crates/fit/src/solver/profile.rs` | 292 | `1..=292` | `run_reduced_lm_with_domain`, `solve_profile_subproblem`, `solve_profile_subproblem_poisson` (reduced trust-region LM with `converged` reporting, domain positivity, and step-halving deviance guard). | Clean — nuisance subproblem reduction & deviance monotonicity guard verified |

---

## Findings (`S1` -> `S4`)

### `FA-SEC07-01` · `S2` · Diverged or iteration-capped nonlinear fits emit `ResultStatus::UpToDate` and `Parameter identifiability: Pass` (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/result.rs:455, 495, 813-857` and `crates/fit/src/solver/fit.rs:432-436`
- **Category:** `wrong-result`
- **Defect:** `execute_nonlinear_analysis` marks `result.status = ResultStatus::UpToDate` unconditionally without surfacing `FitStatus::Diverged` or `FitStatus::HitMaxIters` in `result.checklist`, while `finalize_fit_uncertainty` skips `identifiability::assess` when `lm_out.status == FitStatus::Diverged`, causing `append_nonlinear_checklist` to fall through to its default `else` branch and emit `Parameter identifiability` with `ChecklistStatus::Pass`.
- **Failure scenario:** When nonlinear optimization stalls away from a stationary point (`FitStatus::Diverged`) or exhausts `max_iters` (`FitStatus::HitMaxIters`), `execute_nonlinear_analysis` returns `Ok(ResultSheet)` with `ResultStatus::UpToDate` and no checklist warning/violation for non-convergence (`fit.status` appears only inside the parameter tab footnote text); worse, on `FitStatus::Diverged`, `fit.identifiability` remains `FitIdentifiability::NotAssessed`, so the checklist explicitly claims `"All free parameters are cleanly identifiable."` (`ChecklistStatus::Pass`).
- **Evidence:**
```rust
// crates/fit/src/solver/fit.rs:432-436
    let identifiability = if config.identify_unstable_parameters && k_free > 0 && lm_out.status != FitStatus::Diverged {
        identifiability::assess(layout, obs, &lm_out.j_mat, df, &cov.dependencies, profile_computed.then_some(profile_cis.as_slice()))?
    } else {
        Assessment::default()
    };
```
```rust
// crates/fit/src/result.rs:495, 823-857
    result.status = ResultStatus::UpToDate;
// ...
    } else if let FitIdentifiability::Ambiguous { reasons } = &fit.identifiability {
        ...
    } else if fit.status == FitStatus::Ambiguous || fit.covariance.ambiguous {
        ...
    } else if fit.parameters.iter().any(|p| p.dependency > 0.99) {
        ...
    } else {
        result.checklist.push(ChecklistItem {
            assumption: "Parameter identifiability".to_owned(),
            status: ChecklistStatus::Pass,
            diagnostic_metric: None,
            explanation: "All free parameters are cleanly identifiable.".to_owned(),
            remediation: None,
        });
    }
```
- **Suggested fix:** Emit a dedicated `ChecklistStatus::Violated` / `ChecklistStatus::Warn` convergence item in `append_nonlinear_checklist` whenever `fit.status` is `FitStatus::Diverged` or `FitStatus::HitMaxIters`, and guard the `"Parameter identifiability"` `Pass` branch on `fit.identifiability == FitIdentifiability::Identifiable && fit.status == FitStatus::Converged` (emitting `Info`/`Warn` when `NotAssessed` or `Diverged`).
- **Related:** `crates/fit/src/solver/datasets.rs:434-442`.

---

### `FA-SEC07-02` · `S2` · Poisson and Cauchy outer IRLS loops suppress outer non-convergence, drop prior iteration counts, and omit Poisson deviance step-halving (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/solver/irls.rs:120-133, 199-218`
- **Category:** `wrong-result`
- **Defect:** `fit_poisson_irls` (`for _ in 0..12`) and `run_cauchy_irls_loop` (`for _ in 0..15`) do not track whether the outer IRLS fixed-point criterion (`rel_change < 1e-10` or `max_step < 1e-8`) succeeded, unconditionally overwrite `last_out` (resetting `status` and `iterations` to the final frozen-weight inner LM run), and in `fit_poisson_irls` accept steps that increase Poisson deviance without line search.
- **Failure scenario:** If a Poisson nonlinear model oscillates or needs more than `12` outer reweighting passes (or a Cauchy model needs more than `15`), `last_out` from the final inner LM step typically converges on frozen weights and returns `FitStatus::Converged` with a small inner-step `iterations` count (hiding unconverged outer IRLS); furthermore, in `fit_poisson_irls`, an overshooting linearized step (`dev > prev_dev`) is accepted unconditionally via `last_out = next;` (`irls.rs:128`) unlike `solve_profile_subproblem_poisson` (`crates/fit/src/solver/profile.rs:247-289`), allowing Poisson deviance to increase.
- **Evidence:**
```rust
// crates/fit/src/solver/irls.rs:120-133
    for _ in 0..12 {
        theta = last_out.theta.clone();
        assign_poisson_irls_weights(&mut work_obs, &last_out.y_hat)?;
        let next = run_trust_region_lm_with_domain(model, layout, &work_obs, &theta, config.max_iters, 10.0, true)?;
        let dev = poisson_deviance(&work_obs, &next.y_hat).ok_or_else(|| FitError::ImpossibleWeights {
            reason: "Poisson deviance regression encountered non-positive fitted means (mu <= 0)".to_owned(),
        })?;
        let rel_change = (prev_dev - dev).abs() / prev_dev.max(1.0);
        last_out = next;
        prev_dev = dev;
        if rel_change < 1e-10 {
            break;
        }
    }
```
```rust
// crates/fit/src/solver/irls.rs:199-218
    for _ in 0..15 {
        let prev_theta = last_out.theta.clone();
        ...
        last_out = run_trust_region_lm(model, layout, &work_obs, &prev_theta, config.max_iters, 10.0)?;
        let max_step = prev_theta
            .iter()
            .zip(last_out.theta.iter())
            .map(|(a, b)| (a - b).abs() / b.abs().max(a.abs()).max(f64::MIN_POSITIVE))
            .fold(0.0_f64, f64::max);
        if max_step < 1e-8 {
            break;
        }
    }
```
- **Suggested fix:** Accumulate total inner/outer iterations across passes, add step-halving on `dev > prev_dev` in `fit_poisson_irls` (matching `solve_profile_subproblem_poisson`), and set `last_out.status = FitStatus::HitMaxIters` (unless already `Diverged`) when the outer IRLS loop exhausts its pass limit without satisfying the fixed-point tolerance.
- **Related:** `crates/fit/src/solver/profile.rs:247-289`.

---

### `FA-SEC07-03` · `S2` · `fit_independent_datasets` publishes pooled `global_layout.fixed_constants`, contradicting per-dataset `default_parameter_rules` and dropping per-dataset constant scalars (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/solver/datasets.rs:185, 518`, `crates/fit/src/solver/fit.rs:92, 102-104`, and `crates/fit/src/result.rs:594-615`
- **Category:** `wrong-result`
- **Defect:** When `datasets.len() > 1` with no `SharedGlobal` parameter rule, `fit_independent_datasets` fits each dataset with its own `ParameterLayout::build(model, &[ds], ...)` (evaluating `model.default_parameter_rules(&ds.points)` per curve), yet populates `NonlinearFitResult::fixed_constants` from the pooled `global_layout.fixed_constants` computed over `all_points`.
- **Failure scenario:** Fitting two datasets with different X or Y ranges using `centered_poly_2` (`XCenter` from sample X mean) or `absolute_ic50_4pl` (`Control0` / `Control100` from Y min/max) solves dataset A and dataset B using per-dataset constants stored in `ds_results[d].params`, while `fit.fixed_constants` reports one pooled `("XCenter", pooled_mean, ...)` (or `Control0`/`Control100`). `append_nonlinear_parameter_tab` (`result.rs:594-615`) then prints the pooled constant and inserts `("xcenter", None)` into `emitted_scalars`, suppressing per-dataset constant scalars in `result.scalars` (`lookup_scalar("XCenter", Some(1))` returns `None`).
- **Evidence:**
```rust
// crates/fit/src/solver/datasets.rs:184-185, 518
    let ds_slice = std::slice::from_ref(ds);
    let ds_layout = ParameterLayout::build(model, ds_slice, hooks, ds_rules)?;
// ...
        fixed_constants: global_layout.fixed_constants,
```
```rust
// crates/fit/src/result.rs:594-613
    for (name, val, _) in &fit.fixed_constants {
        param_labels.push(format!("{} (Constant)", fixed_constant_display_label(fit, &ds_names, name)));
        ...
        emitted_scalars.insert((name.to_ascii_lowercase(), None));
        result.scalars.push(ResultScalarEntry { key: name.clone(), data_set_index: None, value: *val, units: None });
    }
    for ds_res in &fit.datasets {
        for (pname, &pval) in ds_res.param_names.iter().zip(ds_res.params.iter()) {
            ...
            if !emitted_scalars.contains(&(lower.clone(), None)) && !emitted_scalars.contains(&(lower.clone(), Some(ds_res.data_set_index))) {
                emitted_scalars.insert((lower, Some(ds_res.data_set_index)));
                result.scalars.push(ResultScalarEntry { key: pname.clone(), data_set_index: Some(ds_res.data_set_index), value: pval, units: None });
            }
        }
    }
```
- **Suggested fix:** Collect `fixed_constants` in `fit_independent_datasets` from each dataset's `sub.layout.fixed_constants` keyed via `per_dataset_parameter_key(name, ds.data_set_index)` whenever a constant comes from `default_parameter_rules` (or differs across datasets), instead of using pooled `global_layout.fixed_constants`.
- **Related:** `crates/fit/src/library/curves.rs:1448-1457`, `crates/fit/src/library/pharma.rs:992-1010`, `crates/fit/src/solver/layout.rs:68, 78, 187-193`.

---

### `FA-SEC07-04` · `S2` · `nonlinear_uses_weighted_objective` misclassifies unweighted `FitMethod::RobustCauchy` as weighted least squares (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/result.rs:556-560, 632-635, 689-695, 758-760`
- **Category:** `wrong-result`
- **Defect:** `nonlinear_uses_weighted_objective` treats `(fit.weighted_ss - fit.rss).abs() > 1e-12 * fit.rss.abs().max(1.0)` as weighted least squares without excluding `FitMethod::RobustCauchy`, even though `fit_cauchy_irls` populates `fit.weighted_ss` with final-iteration Cauchy IRLS reweighted SS.
- **Failure scenario:** Executing `FitMethod::RobustCauchy` with `WeightingScheme::None` triggers `weighted_obj = true`, emitting a spurious `"Weighted Sum of Squares"` row on `nonlinear_gof`, a `"Weighted SS"` scalar in `result.scalars`, a `Weighted SS = ...` parameter footnote, and a `nonlinear_gof` footnote stating `"parameter SEs, confidence intervals, AICc/BIC, and model comparisons use the weighted / deviance objective"`, directly contradicting `NotApplicableInRobustMode` and exposing an arbitrary IRLS-pass SS rather than the Lorentzian merit (`cauchy_marginal_nll`).
- **Evidence:**
```rust
// crates/fit/src/result.rs:556-560, 689-695
fn nonlinear_uses_weighted_objective(fit: &NonlinearFitResult, nl_spec: &NonlinearFitSpec) -> bool {
    nl_spec.weighting != curvebook_doc::WeightingScheme::None
        || fit.fit_method == curvebook_doc::FitMethod::PoissonDeviance
        || (fit.weighted_ss - fit.rss).abs() > 1e-12 * fit.rss.abs().max(1.0)
}
// ...
    if weighted_obj {
        gof_labels.push(if is_poisson { "Poisson Deviance".to_owned() } else { "Weighted Sum of Squares".to_owned() });
        gof_vals.push(vec![num_cell(fit.weighted_ss)]);
        gof_footnotes.push(String::from(
            "R squared, Sum of Squares (RSS), and Sy.x are computed from unweighted residuals in original Y units; parameter SEs, confidence intervals, AICc/BIC, and model comparisons use the weighted / deviance objective; residual normality and homoscedasticity checks use weighted / deviance-standardized residuals.",
        ));
    }
```
- **Suggested fix:** Restrict `nonlinear_uses_weighted_objective` to `!fit.is_robust_cauchy()` (unless an explicit non-`None` `weighting` is active, in which case Cauchy uses base weights; even then, never claim SEs/CIs/AICc/comparisons use `weighted_ss` when `is_robust`).
- **Related:** `crates/fit/src/solver/results.rs:161-175, 459-461`, `crates/fit/src/solver/irls.rs:208, 221-262`.

---

### `FA-SEC07-05` · `S2` · `uses_replicate_summary_moments` drops `N` and `(N - 1) * SD^2` on pre-summarized tables under all weighting schemes (`1/|X|`, `1/X^2`, `1/Y^2`, `1/|Y|`) (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/solver/layout.rs:294-296, 393-415`
- **Category:** `wrong-result`
- **Defect:** `uses_replicate_summary_moments` gates replicate moment expansion (`rep_n = N`, `ss_within = (N - 1) * SD^2`, and `sqrt_weight = (N * w).sqrt()`) on `config.weighting == WeightingScheme::None`, causing `build_observations` to replace `rep_n` with `1` and `ss_within` with `0.0` under every non-`None` `WeightingScheme`.
- **Failure scenario:** While `WeightingScheme::ObservedSd` (`1/SD^2`) intentionally weights row means as `1` observation per row (`crates/fit/tests/r46_solver_oracles.rs:245-274`), fitting `MeanSdN`, `MeanSemN`, or `MeanCvN` with predictor weighting (`InverseX`, `InverseX2` — where raw replicates at a row share identical `x`) or response weighting (`RelativeSquare`, `Relative`, `GeneralPowerY`) silently discards `N` and `SD`, dropping `sum(N) - R` residual degrees of freedom, altering weighted residuals, SEs, CIs, AICc, and lack-of-fit, and breaking parity with equivalent `RawReplicates` tables.
- **Evidence:**
```rust
// crates/fit/src/solver/layout.rs:294-296, 401-409
pub(in crate::solver) fn uses_replicate_summary_moments(config: &SolverConfig) -> bool {
    config.method == FitMethod::LeastSquares && config.weighting == WeightingScheme::None
}
// ...
            let (sqrt_weight, rep_n, ss_within) = if honor_rep_moments && pt.explicit_weight.is_none() {
                let (n_f, n_u, ss_i) = validated_replicate_moments(pt)?;
                (n_f.sqrt(), n_u, ss_i)
            } else {
                let w = compute_point_weight(pt, scheme)?;
                (w.sqrt(), 1, 0.0)
            };
```
- **Suggested fix:** Expand `(n_f, n_u, ss_i)` and set `sqrt_weight = (n_f * w).sqrt()` and `ss_within = w * ss_i` for non-`ObservedSd` weighting schemes (or at minimum predictor-only `InverseX`/`InverseX2` and curve-predicted IRLS, matching `docs/implementation/curve-fitting-implementation.md` §1.4), while keeping `ObservedSd` as `1` row-mean observation per distinct row.
- **Related:** `crates/fit/src/solver/datasets.rs:38-45, 187-205`, `crates/fit/tests/s3a_fit_x2_regressions.rs:282-310`, `crates/fit/tests/r46_solver_oracles.rs:245-274`.

---

### `FA-SEC07-06` · `S2` · `compare_models` / `compare_models_checked` compare `Diverged` or `HitMaxIters` fits silently (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/solver/comparison.rs:93-165, 171-193` and `crates/fit/src/diagnostics.rs:1036, 1074`
- **Category:** `wrong-result`
- **Defect:** `compare_models` and `compare_models_checked` do not check whether `primary.status` or `alt.status` converged (accepting `FitStatus::Diverged` and `FitStatus::HitMaxIters`), and `compare_models_f_test` / `compare_models_lrt` clamp negative `(ss_simple - ss_complex)` / `ln(ss_null / ss_alt)` values to `0.0`.
- **Failure scenario:** If either the primary or alternative model diverges or stops at `max_iters`, `compare_models` proceeds to compute Extra-SS *F*, LRT, and $\text{AIC}_c$ from an unconverged objective value (clamping a higher unconverged complex `weighted_ss` to `F = 0.0, p = 1.0`) and presents a definitive `"Preferred model"` row in `append_comparison_tab` without error or caveat.
- **Evidence:**
```rust
// crates/fit/src/solver/comparison.rs:93-115, 128-137
    let alt_config = filter_config_rules_for_alt_model(config, alt_model);
    let alt = fit_nonlinear(alt_model, datasets, hooks, &alt_config)?;
    ...
    if primary.n_obs != alt.n_obs || primary.n_outliers_excluded != alt.n_outliers_excluded {
        return Err(...);
    }
    ...
    let outcome = match spec {
        ModelComparisonSpec::ExtraSumOfSquaresF { .. } => {
            let (null_fit, complex_fit, _) = ordered_nested_roles(primary, &alt);
            ModelComparisonOutcome::ExtraSumOfSquaresF(compare_models_f_test(
                null_fit.weighted_ss,
                null_fit.df,
                complex_fit.weighted_ss,
                complex_fit.df,
                config.confidence.alpha(),
            )?)
        }
```
```rust
// crates/fit/src/diagnostics.rs:1035-1037, 1073-1075
    let f_stat = if ms_den > 0.0 {
        (ms_num / ms_den).max(0.0)
// ...
    } else {
        ((n_obs as f64) * (ss_null / ss_alt).ln()).max(0.0)
    };
```
- **Suggested fix:** Reject `FitStatus::Diverged` / `FitStatus::HitMaxIters` (or emit a `ModelComparisonOutcome` / checklist warning and block model-preference selection) in `compare_models` / `compare_models_checked` when either model fails to converge.
- **Related:** `crates/fit/src/result.rs:474-483, 963-1060`.

---

### `FA-SEC07-07` · `S2` · `resolve_mv_columns` allows overlapping `X`/`Y`/`Group` assignments and falls back to `Categorical`/`Label` columns as `X` (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/types.rs:591-667`
- **Category:** `wrong-result`
- **Defect:** `resolve_mv_columns` validates only `< n_vars` bounds and never checks mutual disjointness among `mv_x_columns`, `mv_y_column`, and `mv_group_columns`, and its `x_cols` auto-fallback (`.or_else(|| (0..n_vars).find(|&idx| idx != y_col && !group_set.contains(&idx)))`) selects `VariableRole::Categorical` or `VariableRole::Label` columns as `X` whenever no second numeric column exists.
- **Failure scenario:** Passing overlapping indices (for example `mv_y_column: Some(1), mv_x_columns: &[1]`) fits `Y = f(Y)` (`R² = 1.0`), and auto-resolving a `MultipleVariablesTable` containing `[Group (Categorical with numeric IDs "1", "2"), Response (Continuous)]` with `selected_data_sets = []` sets `x_cols = [0]` and empties `group_cols` (`!x_used.contains(&idx)`), fitting `Response` against the `Categorical` grouping column instead of rejecting the table for lacking a numeric predictor.
- **Evidence:**
```rust
// crates/fit/src/types.rs:621-642
    let y_col = if let Some(yc) = mv_spec.y_column {
        yc
    } else if let Some(&sel) = selected_numeric.iter().rev().find(|idx| !x_set.contains(idx)) {
        sel
    } else {
        pick_default_mv_numeric_col(mv, (0..n_vars).rev().filter(|idx| !x_set.contains(idx) && !group_set.contains(&idx)))
            .or_else(|| (0..n_vars).rev().find(|idx| !x_set.contains(idx) && !group_set.contains(&idx)))
            .unwrap_or_else(|| selected_data_sets.first().copied().unwrap_or(1))
    };

    let selected_x: Vec<usize> = selected_numeric.iter().copied().filter(|&idx| idx != y_col).collect();
    let x_cols = if !mv_spec.x_columns.is_empty() {
        mv_spec.x_columns.to_vec()
    } else if !selected_x.is_empty() {
        selected_x
    } else {
        let fallback = pick_default_mv_numeric_col(mv, (0..n_vars).filter(|&idx| idx != y_col && !group_set.contains(&idx)))
            .or_else(|| (0..n_vars).find(|&idx| idx != y_col && !group_set.contains(&idx)))
            .unwrap_or(usize::from(y_col == 0));
        vec![fallback]
    };
```
- **Suggested fix:** Reject overlapping `x_columns` / `y_column` / `group_columns` (and duplicate indices within `x_columns` / `group_columns`) with `FitError::InvalidParameter`, and return `FitError::InsufficientData` / `FitError::EmptyInput` when no numeric predictor/response pair exists instead of falling back to `Categorical` or `Label` columns.
- **Related:** `crates/fit/src/result.rs:405-408, 456-472`.

---

### `FA-SEC07-08` · `S2` · `deming_regression` ignores pre-summarized `(SD, N)` moments and aborts on `n = 3` datasets with a duplicate `(x, y)` point (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/linear.rs:1116-1160, 1218-1222` and `crates/fit/src/result.rs:88-95, 306-366`
- **Category:** `wrong-result`
- **Defect:** `deming_regression` ignores `DataPoint::n` and `DataPoint::sd` (`build_deming_result_sheet` neither checks `ds.is_pre_summarized` nor passes `count_replicates = true` to `prepare_linear_points`), and `deming_regression`'s leave-one-out jackknife fails the entire fit via `fit_deming_core` (`lambda * s_yy >= s_xx` evaluates `0.0 >= 0.0` at `s_xx = s_yy = 0`) whenever `n == 3` and two points are identical.
- **Failure scenario:** (1) Running Deming Model II regression on a `MeanSdN` / `MeanSemN` / `MeanCvN` table silently treats each summary row as `n = 1` raw point, yielding incorrect `n`, `df`, `sy_x`, and jackknife SEs/CIs. (2) Calling `deming_regression` on a valid non-degenerate 3-point sample containing a repeated replicate `[(1.0, 1.0), (2.0, 2.0), (2.0, 2.0)]` causes LOO iteration `skip = 0` to evaluate `[(2.0, 2.0), (2.0, 2.0)]` (`s_xx = 0, s_yy = 0, s_xy = 0`), hitting `vertical_axis` (`0.0 >= 0.0`) and returning `Err(FitError::InvalidParameter { reason: "zero X-Y covariance with vertical Deming axis" })`.
- **Evidence:**
```rust
// crates/fit/src/result.rs:91, 333-336
let (points, count) = prepare_linear_points(&ds.points, bounds, !lin_spec.deming_model_ii)?;
// ...
    for (ds, &n_excluded) in datasets.iter().zip(excluded) {
        let dem = deming_regression(&ds.points, &opts)?;
```
```rust
// crates/fit/src/linear.rs:1140-1152, 1218-1222
    for skip in 0..n {
        ...
        let (js, ji, _) = fit_deming_core(&sub_x, &sub_y, opts.variance_ratio_lambda)?;
        jack_slopes.push(js);
        jack_ints.push(ji);
    }
// ...
    let slope = if s_xy == 0.0 {
        let vertical_axis = if lambda <= 1.0 { lambda * s_yy >= s_xx } else { s_yy >= s_xx / lambda };
        if vertical_axis {
            return Err(FitError::InvalidParameter { name: "data", reason: String::from("zero X-Y covariance with vertical Deming axis") });
        }
```
- **Suggested fix:** Reject `ds.is_pre_summarized` in `build_deming_result_sheet` with `Err(FitError::RequiresRawReplicates)` (and return `RequiresRawReplicates` or weight appropriately in `deming_regression` when `points` carry `n > 1` or `sd`), and in `deming_regression` degrade `se_slope`/`se_intercept` to `f64::NAN` (instead of aborting a valid full-sample fit) if a single leave-one-out subset is degenerate when `n >= 3`.
- **Related:** `crates/fit/src/error.rs:104`.

---

### `FA-SEC07-09` · `S2` · `tex_format.rs` emits invalid TeX double subscripts/superscripts and omits `curves.rs` built-in aliases (`wrong-result` / `contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/tex_format.rs:247, 940-955, 2123-2138, 2179, 2298-2300`
- **Category:** `wrong-result`
- **Defect:** `contains_tex_meta` and `escape_tex_fallback_braces` leave `'_'` unescaped in `format_unknown_fallback` (`f_{...}`) and fallback custom DSL formatting, `substitute_identifiers` omits whitespace checks before `^`, and `format_line_poly_curve_tex` omits `library/curves.rs` aliases (`centered_polynomial_1..=6`, `semilog_line`, `loglog_line`).
- **Failure scenario:** (1) Passing `centered_polynomial_2`, `semilog_line`, or `loglog_line` (all valid `get_builtin_model` aliases at `library/curves.rs:179-206`) to `format_model_equation_tex` hits `format_unknown_fallback`, and any ID with >= 2 underscores (including `centered_polynomial_2` and `totally_unknown_id` at `tex_format.rs:2615`) renders `Y = f_{centered_polynomial_2}(X)` — a TeX `"Double subscript"` parse error in LaTeX/KaTeX. (2) Unparsed custom DSL formulas with `a_b_c` fail similarly, and `a ^ 2` (space before `^`) with `a = -2` or `a = 1e-5` renders `-2 ^ 2` (wrong sign, `-4`) or `1 \times 10^{-5} ^ 2` (TeX `"Double superscript"` error) because `chars.get(i) == Some(&'^')` does not skip whitespace.
- **Evidence:**
```rust
// crates/fit/src/tex_format.rs:247, 2298-2300, 2614-2615
        let sub = if contains_tex_meta(&id) { format!("\\text{{{}}}", escape_tex_text(&id)) } else { id };
        return format!("Y = f_{{{sub}}}(X)");
// ...
fn contains_tex_meta(s: &str) -> bool {
    s.chars().any(|ch| ch.is_whitespace() || matches!(ch, '\\' | '{' | '}' | '^' | '*' | '$' | '&' | '%' | '#' | '<' | '>'))
}
// ...
        let fb_empty = format_model_equation_tex("totally_unknown_id", None, &[], false, 4);
        assert_eq!(fb_empty, "Y = f_{totally_unknown_id}(X)");
```
```rust
// crates/fit/src/tex_format.rs:2123-2135, 2179
fn escape_tex_fallback_braces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"), ... '&' => out.push_str("\\&"), _ => out.push(ch),
        }
    }
    out
}
// ...
                let next_is_pow = chars.get(i) == Some(&'^');
```
- **Suggested fix:** Resolve canonical model IDs via `get_builtin_model` (or add `"centered_polynomial_1..=6"`, `"semilog_line"`, `"loglog_line"` arms in `format_line_poly_curve_tex`), include `'_'` in `contains_tex_meta` (or wrap `id` in `tex_ident` / `\text{...}` in `format_unknown_fallback`), format multi-underscore fallback identifiers safely via `tex_ident`, and scan past whitespace before checking `^` in `substitute_identifiers`.
- **Related:** `crates/fit/src/library/curves.rs:179-206`.

---

### `FA-SEC07-10` · `S3` · `compute_jacobian` one-sided finite difference ignores domain failures (`None`) on `+h` when `-h` is valid (`wrong-result` / `resources`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/solver/lm.rs:192-231`
- **Category:** `wrong-result`
- **Defect:** When central finite differencing fails (`!can_plus || !can_minus`), the one-sided fallback chooses the perturbation sign solely from the explicit upper box bound (`ub.is_some_and(|u| xj + h_f > u)`) without checking whether `+h_f` evaluates inside the model domain or falling back to `-h_f`.
- **Failure scenario:** If a custom DSL or built-in model without analytical `grad` approaches an implicit domain upper boundary (`eval` returns `None` or non-finite for `xj + h_c`, e.g., `sqrt(1 - p * X)` or `BetaGrowthDecay` with `tm` near `te`, where `ub = None` or `xj + h_f <= ub`), `can_plus` is `false` and `can_minus` is `true`, yet `compute_jacobian` steps `+h_f`, receives `!ok`, and aborts a valid LM optimization with `FitError::BadInitialValues { reason: "Jacobian evaluation returned non-finite values" }` instead of stepping `-h_f`.
- **Evidence:**
```rust
// crates/fit/src/solver/lm.rs:215-224
        // One-sided fallback with smaller step `eps_forward`.
        let h_f = eps_forward * xj.abs().max(1.0);
        let sign = if ub.is_some_and(|u| xj + h_f > u) { -1.0 } else { 1.0 };
        work[slot] = xj + sign * h_f;
        let ok = eval_predictions_for_slot_into(model, layout, obs, &work, &mut flat_params, info.ds_local_idx, &mut yp);
        work[slot] = xj;
        if !ok {
            return None;
        }
```
- **Suggested fix:** Prefer `-h_f` whenever `!can_plus && can_minus` (respecting `lb`), and if the chosen one-sided sign returns `!ok`, try the opposite sign within `[lb, ub]` before returning `None`.
- **Related:** `crates/fit/src/solver/curves.rs:184-217` (`FA-SEC07-11`).

---

### `FA-SEC07-11` · `S3` · `delta_grad_at_x_into` silently zeroes curve band and unknown-interpolation gradients near parameter domain boundaries (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/solver/curves.rs:184-217`
- **Category:** `wrong-result`
- **Defect:** `delta_grad_at_x_into` uses unguarded central differences `theta[slot] ± h` and skips writing `g_out[slot]` (leaving it `0.0`) whenever either `theta[slot] + h` or `theta[slot] - h` returns `None`, without checking box bounds or attempting a one-sided step.
- **Failure scenario:** For any model without analytical `ParametricModel::grad` (including all custom DSL equations and many built-ins) whose fitted parameter lies within `1e-6 * max(|theta|, 1)` of a domain edge (for example `p = 0.0` with `sqrt(p)` or `ln(p)`), `model.eval` returns `None` on `theta - h`, leaving `g_out[slot] = 0.0` across all curve samples and zeroing that parameter's contribution to confidence/prediction bands and unknown-interpolation intervals.
- **Evidence:**
```rust
// crates/fit/src/solver/curves.rs:205-215
        let h = 1e-6 * theta[slot].abs().max(1.0);
        work[slot] = theta[slot] + h;
        layout.expand_params_into(work, ds_local_idx, work_params);
        let fp = model.eval(&[x], work_params, ds_data_idx);
        work[slot] = theta[slot] - h;
        layout.expand_params_into(work, ds_local_idx, work_params);
        let fm = model.eval(&[x], work_params, ds_data_idx);
        work[slot] = theta[slot];
        if let (Some(p), Some(m)) = (fp, fm) {
            g_out[slot] = (p - m) / (2.0 * h);
        }
```
- **Suggested fix:** Respect `info.lower`/`info.upper`, try `theta[slot] ± h` independently, and fall back to one-sided `(p - y0) / h_f` or `(y0 - m) / h_f` (matching `compute_jacobian`) whenever only one side evaluates finite inside `[lower, upper]`.
- **Related:** `crates/fit/src/solver/lm.rs:186-231` (`FA-SEC07-10`).

---

### `FA-SEC07-12` · `S3` · `populate_xy_column` drops `unknowns_y` and `unknowns_x` on non-moment pre-summarized XY modes (`MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`) (`data-loss`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/types.rs:404-437`
- **Category:** `data-loss`
- **Defect:** In `populate_xy_column`, the `else if xy.y_subcolumn_mode.is_pre_summarized()` branch (handling `MeanLowHigh`, `UpperLowerLimits`, and `MedianIqr`) only handles `(Some(x), Some(y))` pairs and omits queueing `unknowns_y` (blank `X` with active central `Y`) and `unknowns_x` (active `X` with blank `Y` subcells).
- **Failure scenario:** Running nonlinear or linear unknown interpolation (`interpolate_unknowns = true`) on an `XyTable` formatted as `MeanLowHigh`, `UpperLowerLimits`, or `MedianIqr` silently produces zero interpolated rows for blank-`X` or blank-`Y` entries, unlike `Single`, `RawReplicates`, `MeanSdN`, `MeanSemN`, and `MeanCvN` tables.
- **Evidence:**
```rust
// crates/fit/src/types.rs:424-437
        } else if xy.y_subcolumn_mode.is_pre_summarized() {
            if let (Some(x), Some(y)) = (x_opt, subcells.first().and_then(Cell::active_number)) {
                series.points.push(DataPoint {
                    x: vec![x],
                    y,
                    sd: None,
                    n: None,
                    explicit_weight: None,
                    source_subcolumn: 0,
                    data_set_index: ds_idx,
                    row,
                });
            }
        } else {
```
- **Suggested fix:** Mirror lines `407-423` inside the `is_pre_summarized()` branch so `(None, Some(y)) if x_blank` pushes `(row, y).into()` to `series.unknowns_y` and `(Some(x), None) if y_cells_are_blank_unknown(subcells)` pushes `(row, x)` to `series.unknowns_x`.
- **Related:** `crates/fit/src/types.rs:475-550`.

---

### `FA-SEC07-13` · `S3` · `resolve_selected_indices` silently drops out-of-range indices and duplicates datasets on repeated selections (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/types.rs:339-341, 344, 476, 749`
- **Category:** `wrong-result`
- **Defect:** `resolve_selected_indices` silently filters out `i >= total` instead of returning an error, and never deduplicates `selected` before extracting `Xy` and `Grouped` columns.
- **Failure scenario:** Passing `selected_data_sets = &[0, 0]` to `extract_datasets_from_sheet` on an `XyTable` or `GroupedTable` returns two identical `DatasetSeries` entries both carrying `data_set_index: 0`, double-counting every observation in the fit, inflating `df` and `F` statistics, and emitting duplicate `ResultScalarEntry` keys (while `extract_from_multivar` quietly drops the duplicate via `.take()`, and `[99]` silently degrades to `EmptyInput` rather than `InvalidParameter`).
- **Evidence:**
```rust
// crates/fit/src/types.rs:339-352, 756-759
fn resolve_selected_indices(total: usize, selected: &[usize]) -> Vec<usize> {
    if selected.is_empty() { (0..total).collect() } else { selected.iter().copied().filter(|&i| i < total).collect() }
}
// ...
    for idx in keep_indices {
        let Some((tokens, mut pts)) = indexed.get_mut(idx).and_then(Option::take) else {
            continue;
        };
```
- **Suggested fix:** Return `Err(FitError::InvalidParameter { name: "selected_data_sets", ... })` when any index `>= total` (matching `resolve_mv_columns` at `597-611`), and deduplicate preserving first-seen order (or reject duplicates).
- **Related:** `crates/fit/src/types.rs:597-611`.

---

### `FA-SEC07-14` · `S3` · `fit_plot_function_only` succeeds on zero finite points and hardcodes per-dataset `r_squared = 1.0` and `sy_x = 0.0` (`wrong-result` / `contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/solver/fit.rs:108-255`
- **Category:** `wrong-result`
- **Defect:** `fit_plot_function_only` hardcodes `r_squared: 1.0`, `sy_x: if is_robust { f64::NAN } else { 0.0 }`, and `studentized_residuals: vec![0.0; ds_res.len()]` on every `FittedDatasetResult` regardless of actual residual sum of squares, and returns `Ok` with `n_obs: 0` when all observations fail `model.eval` (or have empty `points` when invoked directly via `fit_nonlinear`).
- **Failure scenario:** Calling `fit_nonlinear` with `SolverConfig { plot_function_only: true, .. }` on a dataset whose observed `y` does not match the initial parameters computes accurate top-level `NonlinearFitResult::rss`, `r_squared`, and `rsd` (`201-210`), while each `datasets[i]` reports `r_squared: 1.0` and `sy_x: 0.0` alongside nonzero `ss_residual`; if all `eval` calls return `None` on a non-empty dataset, `fit_plot_function_only` still returns `Ok` with `r_squared: 1.0` and `FitStatus::Converged`.
- **Evidence:**
```rust
// crates/fit/src/solver/fit.rs:182-198
        ds_results.push(FittedDatasetResult {
            ...
            studentized_residuals: if is_robust { Vec::new() } else { vec![0.0; ds_res.len()] },
            ss_residual: ds_ss,
            r_squared: 1.0,
            sy_x: if is_robust { f64::NAN } else { 0.0 },
            rsdr: None,
            runs_test: None,
            curve_overlay: Some(overlay),
        });
```
- **Suggested fix:** Compute per-dataset `r_squared`, `sy_x`, and `observed_x_range` from each dataset's evaluated points (using `tss` and `(ds_n.saturating_sub(k_eff_d))`), and guard `total_obs == 0` when `datasets.iter().any(|d| !d.points.is_empty())` with `FitError::BadInitialValues` (preserving empty-dataset plot-only overlay mode in `execute_nonlinear_analysis:409-417`).
- **Related:** `crates/fit/src/result.rs:409-419`.

---

### `FA-SEC07-15` · `S3` · Linear regression & information-criteria docstring/validation contract drift (`contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/linear.rs:249, 261, 296-302, 523, 585, 1075-1102` and `crates/fit/src/diagnostics.rs:1110-1122`
- **Category:** `contract-drift`
- **Defect:** Four documented API invariants diverge from implementation behavior:
  1. `SimpleLinearResult::r` (`linear.rs:249`) documents `NaN` when `force_through_origin` or zero total variance, whereas `fit_linear_points` (`523`) returns uncentered cosine or `0.0`.
  2. `SimpleLinearResult::x_intercept` (`linear.rs:261`) documents `None` when `force_through_origin` or `slope == 0`, whereas `fit_linear_points` (`585`) returns `Some(0.0)` (`Some(-0.0 / slope)` when `force_through_origin`).
  3. `LinearComparisonResult::{f_intercepts, df_intercepts, p_intercepts, intercepts_significantly_different}` (`linear.rs:296-302`) document `None` when `slopes_significantly_different`, whereas `compare_linear_lines` (`1075-1102`) always returns `Some(...)`.
  4. `compute_information_criteria` (`diagnostics.rs:1110-1122`) does not validate `ss.is_finite() && ss >= 0.0` (contrast `compare_models_f_test`/`compare_models_lrt`), silently clamping negative `ss` to `1e-300 * n_obs` or propagating `NaN`.
- **Failure scenario:** Direct callers relying on documented `NaN`/`None` contracts for origin-forced linear regression or ANCOVA elevation gating receive valid-looking numeric values (uncentered `r`, `Some(-0.0)` X-intercept, or pooled-intercept *F* tests despite heterogeneous slopes), and direct callers passing negative/`NaN` `ss` to `compute_information_criteria` receive finite bogus AICc or `NaN` instead of `Err(FitError::InvalidParameter)`.
- **Evidence:**
```rust
// crates/fit/src/linear.rs:522-523, 585, 1075-1102
    let r = if fit.s_xx > 0.0 && fit.s_yy > 0.0 { (fit.s_xy / (fit.s_xx * fit.s_yy).sqrt()).clamp(-1.0, 1.0) } else { 0.0 };
// ...
        x_intercept: if fit.slope != 0.0 { Some(-fit.intercept / fit.slope) } else { None },
// ...
    let (all_slope, all_int, _, _, _, _, _, _) = fit_with_intercept(&all_xs, &all_ys, &all_ns, n_total, ss_within_total)?;
    ...
        f_intercepts: Some(f_int),
```
```rust
// crates/fit/src/diagnostics.rs:1110-1118
pub fn compute_information_criteria(ss: f64, n_obs: usize, p_free: usize) -> Result<InformationCriteria, FitError> {
    let k = p_free + 1;
    if n_obs <= k + 1 {
        return Err(FitError::InsufficientData { required: k + 2, actual: n_obs });
    }
    let n_f = n_obs as f64;
    let k_f = k as f64;
    let mse_ratio = (ss / n_f).max(1e-300);
```
- **Suggested fix:** Enforce `NaN`/`None` for `r` and `x_intercept` when `force_through_origin` (and align ANCOVA `slopes_significantly_different` suppression or update the `LinearComparisonResult` docs if always-emit is required by downstream UI/tests), and add `!ss.is_finite() || ss < 0.0` validation to `compute_information_criteria`.
- **Related:** `crates/fit/src/result.rs:226-231`, `crates/fit/tests/s3a_fit_x2_regressions.rs:362`.

---

### `FA-SEC07-16` · `S4` · Cleanup: dead expressions, unused closure binding, and duplicated ROUT BH implementation (`cleanup`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/types.rs:384-393`, `crates/fit/src/tex_format.rs:1133, 2150, 2195-2198`, and `crates/fit/src/diagnostics.rs:1806-1901`
- **Category:** `cleanup`
- **Defect:** Dead code and duplicate logic remain in four spots: (1) unused closure `let _ = x_sd;` in `populate_xy_column`; (2) dead `!has_ode && false` guard in `DslRawStmt::is_output` and unused `"\\sqrt{"` replacement string in `fallback_custom_dsl_tex` that `replace_ident_prefix` overrides with `"\\text{sqrt}("`; and (3) `diagnostics::rout_detect_outliers` duplicates ~80 lines of `solver::inference::rout_detect_outliers_mb2006` with slight differences in `y_scale` floor (`1e-12` vs `f64::MIN_POSITIVE`) and `student_t_two_sided_p` error propagation (`unwrap_or(1.0)` vs `?`).
- **Failure scenario:** Maintainability drift between `rout_detect_outliers` (public API) and `rout_detect_outliers_mb2006` (solver internal ROUT) if FDR rules or RSDR floors change in one place and not the other; zero user-visible impact today on well-scaled inputs.
- **Evidence:**
```rust
// crates/fit/src/types.rs:384-393
    let x_sd = |row: usize| -> Option<f64> { ... };
    let _ = x_sd;
```
```rust
// crates/fit/src/tex_format.rs:1133, 2150, 2195-2199
            Self::OdeInitialOrLocal { .. } => !has_ode && false,
// ...
        ("sqrt(", "\\sqrt{"),
// ...
fn replace_ident_prefix(input: &str, pat: &str, rep: &str) -> String {
    if pat == "sqrt(" {
        return replace_ident_word_prefix(input, "sqrt(", "\\text{sqrt}(");
    }
    replace_ident_word_prefix(input, pat, rep)
}
```
- **Suggested fix:** Remove `x_sd` and simplify `!has_ode && false` -> `false` and `replace_ident_prefix`; delegate `diagnostics::rout_detect_outliers` to `rout_detect_outliers_mb2006` (with unit weights) or extract a single shared BH top-30% ROUT helper.
- **Related:** `crates/fit/src/solver/inference.rs:278-369`.
