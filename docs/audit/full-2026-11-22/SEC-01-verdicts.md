# SEC-01 Verification Verdicts — Numerical Foundations (`crates/math`, `crates/palette`, `crates/platform`)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend` (independent code inspection + R/`python3` oracle checks + isolated `.lc-worktrees/ver-backend` dynamic probes `ver_sec01_probe`, cleaned up after execution)
- **Source Report:** [SEC-01.md](../../../docs/audit/full-2026-11-22/SEC-01.md) (`FA-SEC01-01`..`FA-SEC01-24`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 24 | `S1: 0`, `S2: 5`, `S3: 14`, `S4: 5` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **24** | **`S1=0, S2=5, S3=14, S4=5`** |

Dynamic reproducer run against isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095` (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend cargo test --locked --offline -p curvebook-math -p curvebook-palette -p curvebook-platform --test ver_sec01_probe`):
- `curvebook-math`: `4 passed; 0 failed`
- `curvebook-palette`: `1 passed; 0 failed`
- `curvebook-platform`: `3 passed; 0 failed`

Baseline `cargo test --locked --offline -p curvebook-palette -p curvebook-platform` on `main` also passed all unit/integration/doc tests (`15 + 5 + 1` in `palette`, `33` in `platform`) while emitting the two unused-dependency warnings recorded in `FA-SEC01-22`. Note that unnumbered lower-severity trace notes in [local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L629-L638) (`noncentral.rs:382`, `ncf_cdf`, `student_t_isf`, `quad.rs:230`) were never closed in `#1`–`#126` or recorded in `Known defects`, so `FA-SEC01-04`, `FA-SEC01-10`, `FA-SEC01-14`, and `FA-SEC01-15` remain `CONFIRMED`.

---

## File Coverage Completeness Check

Verified `100%` coverage against `crates/math`, `crates/palette`, and `crates/platform` at `7c85a8e516165ddadb101236a88eec937bf1f095` (`38` files, `14,807` total lines, zero files omitted):

- `crates/math`: `Cargo.toml` (`17`), `src/{error.rs (80), lib.rs (258), pvalue.rs (326), quad.rs (261), quantile.rs (111), rng.rs (244), roots.rs (176), sum.rs (480)}`, `src/dist/{mod.rs (33), core.rs (628), dunnett.rs (343), lilliefors.rs (321), noncentral.rs (976), studentized_range.rs (770)}`, `src/linalg/{mod.rs (19), cholesky.rs (322), matrix.rs (327), qr.rs (971), svd.rs (430)}`, `src/special/{mod.rs (14), erf.rs (291), gamma.rs (405), inc_beta.rs (501), inc_gamma.rs (708)}`.
- `crates/palette`: `Cargo.toml` (`19`), `src/{categorical.rs (192), colormap.rs (508), cvd.rs (175), lib.rs (292), space.rs (337), style.rs (85)}`.
- `crates/platform`: `Cargo.toml` (`15`), `src/{fs.rs (626), lib.rs (119), path_guard.rs (428), process.rs (1225), sync.rs (1774)}`.

Every file and line count matches `SEC-01.md`'s File Coverage Table with `1..=N` inspection.

---

## Per-Finding Verdicts

### Verdict `FA-SEC01-01` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [rng.rs](../../../crates/math/src/rng.rs#L138-L181) validates `0.0..=1e15` and evaluates `(k_f * lambda.ln()) - lambda - ln_factorial_approx(k)` on PTRS rejection steps (reachable via [randomize.rs](../../../crates/transform/src/randomize.rs#L827)). At `lambda = 1e15, k_f = 1_000_000_071_760_245.0`, `term1 = k_f * lambda.ln() > 3.4e16` has `ulp = 4.0` (`next_up() - term1 == 4.0`, verified in `probe_01`), and `python3` reference yields naive `-20.0` vs exact `-20.763093089510527` (`|err| = 0.7631`, `exp(err) = 2.1449x` acceptance ratio distortion).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (distorts rejection sampling across valid documented `1e12..=1e15` Poisson domain).
- **Still present on main:** yes

### Verdict `FA-SEC01-02` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [sum.rs](../../../crates/math/src/sum.rs#L114-L119) leaves `(1e-140, 1e130)` unscaled while [pearson_r](../../../crates/math/src/sum.rs#L358) computes `(css_x * css_y).sqrt()`. Dynamic `probe_01`: `pearson_r(&[-1e80, 1e80], &[-1e80, 1e80]) == 0.0` (expected `1.0`), `pearson_r(&[-1e-85, 1e-85, 0.0, 0.0], &[-1e-85, 0.0, 1e-85, 0.0]) == 1.0` (expected `0.5`), and orthogonal `1e-85` vectors return `NaN`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent wrong correlation / `NaN` on extreme-magnitude inputs inside the explicitly unscaled band).
- **Still present on main:** yes

### Verdict `FA-SEC01-03` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [noncentral.rs](../../../crates/math/src/dist/noncentral.rs#L235-L263) dispatches `df > 1e6 || delta.abs() > 1e9` to the Gaussian `U` approximation and divides by `(1.0 + t * t * var_u).sqrt()`. Verified against `Rscript`: `pt(1e10, 1, ncp=2e9) = 0.7816617` vs `nct_cdf(1e10, 1.0, 2e9)` error `> 0.05` (`0.839359`); `nct_cdf(1e160, 1.0, 2e9) == 0.5`. *Precision note:* `nct_positive_t_eval` (`349`) enters `nct_positive_t_large_delta_quad` only when `0.5 * delta^2 > 1e5` (`|delta| > 447.2`, not `delta = 1.0`), and because `eu < 1.0` by Jensen's inequality, `t * eu` stays finite even at `t = 1e308, df = 1e7, delta = 500.0`, producing `0.5` (`0 / Inf`) rather than `NaN`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (wrong noncentral *t* CDF on small `df` with `|delta| > 1e9`, plus non-monotone `0.5` drop at `|t| >= 1e155`).
- **Still present on main:** yes

### Verdict `FA-SEC01-04` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [noncentral.rs](../../../crates/math/src/dist/noncentral.rs#L618-L652) and [noncentral.rs](../../../crates/math/src/dist/noncentral.rs#L738-L775) seed downward recurrences from `k0 = floor(lambda / 2)` in linear `f64` space. Dynamic `probe_01` reproduced `ncf_cdf(1.0, 1.0, 100.0, 500.0) == 0.0`, `ncf_cdf(0.05, 1.0, 10.0, 400.0) == 0.0`, and `ncchi2_cdf(0.5, 1.0, 500.0) == 0.0` (vs `Rscript`: `pchisq(0.5, 1, ncp=500) = 2.812314e-104`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (collapses normal-range `1e-104..1e-87` lower-tail probabilities to exact `0.0`).
- **Still present on main:** yes

### Verdict `FA-SEC01-05` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [sync.rs](../../../crates/platform/src/sync.rs#L56-L58) and [sync.rs](../../../crates/platform/src/sync.rs#L1200-L1204) select `core.limits.warn_limit` without `.min(core.limits.wait_timeout)`. Dynamic `probe_05_warn_limit_exceeds_wait_timeout`: `Mutex::with_limits("ver-05", 0_u32, Duration::from_millis(600), Duration::from_millis(20))` contended for `610ms` (`>= 500ms`) before returning `LockError::TimedOut` despite the `20ms` wait timeout.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (hangs/overruns wait budgets when `warn_limit > wait_timeout`).
- **Still present on main:** yes

### Verdict `FA-SEC01-06` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [matrix.rs](../../../crates/math/src/linalg/matrix.rs#L139) computes `r.saturating_mul(ncols) + c` on out-of-bounds error construction. Dynamic `probe_06_matrix_set_overflow_panic`: calling `m.set(usize::MAX, 1, 0.0)` on a `2x2` matrix panicked at `crates/math/src/linalg/matrix.rs:139:88: attempt to add with overflow`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (debug panic / release wrap in direct error path, not triggered by normal bounded callers).
- **Still present on main:** yes

### Verdict `FA-SEC01-07` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [studentized_range.rs](../../../crates/math/src/dist/studentized_range.rs#L55-L60) and [studentized_range.rs](../../../crates/math/src/dist/studentized_range.rs#L251-L252) pass hardcoded `min_gap = 0.02`, skipping `[0, 0.2 * t_star]` and `[0.2 * t_star, t_star]` whenever `t_star <= 0.02` (`df < 2.0`), also reached from [dunnett.rs](../../../crates/math/src/dist/dunnett.rs#L137) / [dunnett.rs](../../../crates/math/src/dist/dunnett.rs#L170). Dynamic `probe_07`: `max_studentized_range_sf(5000.0, 2, 2, 1.99) = 2.227891e-7 > 2 * studentized_range_sf(5000.0, 2, 1.99) = 1.724939e-7`, violating the Fréchet/Bonferroni union bound. (*Note:* `studentized_range_sf` fast-paths `k == 2` at line `308`, so public callers hit the bug for `k >= 3`, `r >= 2`, or Dunnett `m >= 2`.)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (tail error concentrated in extreme `q` with `df < 2.0`).
- **Still present on main:** yes

### Verdict `FA-SEC01-08` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [svd.rs](../../../crates/math/src/linalg/svd.rs#L89-L90), [svd.rs](../../../crates/math/src/linalg/svd.rs#L146), and [cholesky.rs](../../../crates/math/src/linalg/cholesky.rs#L192-L233) threshold rotations, asymmetry, and convergence on unrescaled `frobenius_norm()`. Dynamic `probe_07_to_17`: on a `4x4` `9e307`/`4e307` symmetric block matrix (`sigma_true = [1.3e308, 1.3e308, 5e307, 5e307]`), `svd` returned `sigma[0] < 1.0e308`. For `sym_eigen`: on `4x4` `9e307`/`4e307`, `0.5 * (a[i,i] + a[i,i])` (`cholesky.rs:212`) overflows to `[inf; 4]` (rather than `[9e307; 4]`), while on a `6x6` `6e307`/`2e307` matrix (`frob == +Inf`, finite `6e307 + 6e307`), Sweep 0 exits prematurely with wrong eigenvalues and accepts `-2e307` off-diagonal asymmetry without `DomainError`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (requires finite entries near `~5e307..f64::MAX` whose Frobenius norm overflows `f64`).
- **Still present on main:** yes

### Verdict `FA-SEC01-09` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [qr.rs](../../../crates/math/src/linalg/qr.rs#L149-L160), [qr.rs](../../../crates/math/src/linalg/qr.rs#L514-L518), [qr.rs](../../../crates/math/src/linalg/qr.rs#L567-L573), and [qr.rs](../../../crates/math/src/linalg/qr.rs#L621). Dynamic `probe_07_to_17`: confirmed `parameter_dependencies(2e154 * I_2) == [1.0, 1.0]`, `parameter_dependencies(1e162 * I_2) == [NaN, NaN]`, false aliasing of column `0` in `[1.5e308, 0; 1.5e308, 1]`, `NaN` in `thin_q()` for `[1.5e308, 1.5e308]^T`, and `Err(DomainError)` in `refined_covariance` for `[[3e-308, 3e-308], [0.0, 6e-310]]`. (*Precision note:* `X = 1e-310 * I_2` degrades earlier inside [DenseMatrix::col_norm2](../../../crates/math/src/linalg/matrix.rs#L278-L281) where `1.0 / 1e-310 = +Inf` causes `0.0 * Inf = NaN`, returning `rank = 0` before reaching `refined_covariance`.)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (triggered at extreme `~1e154..f64::MAX` or sub-`MIN_POSITIVE` scales).
- **Still present on main:** yes

### Verdict `FA-SEC01-10` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [core.rs](../../../crates/math/src/dist/core.rs#L297-L298) and [core.rs](../../../crates/math/src/dist/core.rs#L375-L490). Dynamic `probe_07_to_17` + `Rscript`: `pf(0.01, Inf, 10) = 1.870291e-208` whereas `f_cdf`/`f_sf`/`f_ln_sf`/`f_isf`/`f_ppf` fail with `InvalidParameter` (with `name: "a"` in `f_cdf`/`f_ppf`, `name: "b"` in `f_sf`/`f_ln_sf`); `f_cdf(1e200, 1e-200, 1e200) == 0.0` and `f_cdf(1e-200, 1e200, 1e-200) == 1.0`; `student_t_isf(1e-200, 1.0)` returns `Err(NotConverged)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (extreme parameters / representable extreme Cauchy quantile).
- **Still present on main:** yes

### Verdict `FA-SEC01-11` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [inc_beta.rs](../../../crates/math/src/special/inc_beta.rs#L49-L51), [inc_beta.rs](../../../crates/math/src/special/inc_beta.rs#L96-L107), and [noncentral.rs](../../../crates/math/src/dist/noncentral.rs#L132-L136). Dynamic `probe_07_to_17`: `reg_beta(0.5, 1e155, 1e155)` fails with `Err(NotConverged)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (occurs only at shape parameters `a, b >= 1e155`).
- **Still present on main:** yes

### Verdict `FA-SEC01-12` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [inc_gamma.rs](../../../crates/math/src/special/inc_gamma.rs#L75-L80) computes `a + x` unscaled inside `gamma_deviance`. Dynamic `probe_07_to_17`: `reg_gamma_p(1e308, 1.2e308) == 0.5`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (requires `a + x > f64::MAX`).
- **Still present on main:** yes

### Verdict `FA-SEC01-13` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [erf.rs](../../../crates/math/src/special/erf.rs#L211) halves `0.5 * q` before `invert_normal_upper_tail`. Dynamic `probe_07_to_17`: `erfc_inv(5e-324) == f64::INFINITY` (vs `python3` leading asymptotic `sqrt(-ln(sqrt(pi)*5e-324/2)) ~= 27.284`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (affects positive subnormals `< 2 * f64::MIN_SUBNORMAL`).
- **Still present on main:** yes

### Verdict `FA-SEC01-14` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [quad.rs](../../../crates/math/src/quad.rs#L99-L113), [quad.rs](../../../crates/math/src/quad.rs#L208-L230), and [roots.rs](../../../crates/math/src/roots.rs#L81), [roots.rs](../../../crates/math/src/roots.rs#L154). Dynamic `probe_07_to_17`: `integrate_adaptive` returned `Ok` when error exceeded `abs_tol` by `4x` (under the undocumented `16.0x` final check); `gauss_legendre_32` / `gauss_kronrod_15_panel` on `[-1e308, 1e308]` yielded `NaN`; `brent_root` on `[-1e308, 1e308]` returned `Err`; and `safeguarded_halley_step` overflowed `0.5 * (lo + hi)` to `+Inf` on same-sign extreme brackets `(1e308, 1.5e308)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (contract drift on fallback tolerance margin + extreme bracket midpoint overflow).
- **Still present on main:** yes

### Verdict `FA-SEC01-15` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [noncentral.rs](../../../crates/math/src/dist/noncentral.rs#L376-L383) skips `nct_bidirectional_upper_sum` when `upper && delta < 0.0` and evaluates `(1.0 - cdf_val).clamp(0.0, 1.0)`. Dynamic `probe_07_to_17`: `nct_sf(2.0, 10.0, -10.0) == 0.0`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (opposite-tail cancellation `< 1e-16`, absolute error bounded by `1e-16`).
- **Still present on main:** yes

### Verdict `FA-SEC01-16` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [noncentral.rs](../../../crates/math/src/dist/noncentral.rs#L667-L669) and [noncentral.rs](../../../crates/math/src/dist/noncentral.rs#L789-L791) return `Ok((0.0, 0.0))` before validating `df`/`df1`/`df2`. Dynamic `probe_07_to_17`: `noncentral_f_ci(0.0, -1.0, f64::NAN, 95.0) == (0.0, 0.0)` and `noncentral_chi2_ci(0.0, 0.0, 95.0) == (0.0, 0.0)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (public-API `# Errors` contract drift; downstream callers guard `df >= 1.0`).
- **Still present on main:** yes

### Verdict `FA-SEC01-17` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [studentized_range.rs](../../../crates/math/src/dist/studentized_range.rs#L121-L125), [studentized_range.rs](../../../crates/math/src/dist/studentized_range.rs#L459), and [studentized_range.rs](../../../crates/math/src/dist/studentized_range.rs#L590) perform unchecked `usize` `k * (k - 1)` multiplication (overflowing 32-bit `wasm32` at `k >= 65_537`) and `as i32` casts for `powi`. Meanwhile [dunnett_loadings_from_sizes](../../../crates/math/src/dist/dunnett.rs#L42) rounds `(ni / (n_control + ni)).sqrt()` to `1.0` when `ni / n_control >= 1e17`. Dynamic `probe_07_to_17`: `dunnett_loadings_from_sizes(1.0, &[1e20]) == [1.0]`, immediately rejected by `dunnett_two_sided_p_loadings` with `Err(InvalidParameter { name: "lambda", .. })`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (extreme `k >= 65_537` on `wasm32` / extreme `1e17` group-size ratio).
- **Still present on main:** yes

### Verdict `FA-SEC01-18` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [sum.rs](../../../crates/math/src/sum.rs#L152-L153) marks `scaled_css` with `#[serde(skip)]`. Dynamic `probe_18` (`crates/palette`): on overflow (`[1e160, 2e160]`), `css == +Inf`, so non-JSON `Inf`-preserving formats lose `scaled_css` while `serde_json` rejects deserialization (`invalid type: null, expected f64`); additionally, in the symmetric underflow regime (`[1e-165, 2e-165]`), `css` underflows to `0.0`, so `serde_json` round-trips cleanly and silently collapses `sample_sd()` from `7.071e-166` to `0.0`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (`SummaryMoments` currently stays in-process across downstream crates).
- **Still present on main:** yes

### Verdict `FA-SEC01-19` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [sync.rs](../../../crates/platform/src/sync.rs#L1194) invokes user `on_contention` without a `Drop` guard clearing [g.waiting_on](../../../crates/platform/src/sync.rs#L1250). Dynamic `probe_19_on_contention_panic_leaks_waiting_on`: catching an unwind from a panicking `on_contention` callback on `l1`, then acquiring `l2` on the worker thread while the main thread held `l1` and attempted `l2.lock()`, triggered a false `LockError::WouldDeadlock`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (requires user `on_contention` callback panic caught via `catch_unwind`).
- **Still present on main:** yes

### Verdict `FA-SEC01-20` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [pvalue.rs](../../../crates/math/src/pvalue.rs#L195) checks `if p > 0.999` without `&& p < 1.0`. Dynamic `probe_20_21_24`: `PValueFormat::Apa.format_p(1.0) == "> .999"`, inconsistent with `FourDecimal` (`"1.0000"`) and `Nejm` (`"1.000"`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (minor formatting edge-case inconsistency at exact `p = 1.0`).
- **Still present on main:** yes

### Verdict `FA-SEC01-21` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [space.rs](../../../crates/palette/src/space.rs#L220) computes `rem_euclid(360.0)` and [lib.rs](../../../crates/math/src/lib.rs#L139) computes `(u / (100.0 - u)).ln()`. Dynamic probes: `Oklab { l: 0.5, a: 0.1, b: -1e-20, alpha: 1.0 }.to_oklch().h_deg == 360.0`; `AxisScale::Logit(LogitDomain::Percent).transform(5e-323) == None`; and `inverse(40.0) == Some(100.0)` (`Some(1.0)` for `Proportion`, `Probit.inverse(10.0) == Some(100.0)`), whose forward `transform` returns `None`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (open-interval endpoint / subnormal rounding corner cases).
- **Still present on main:** yes

### Verdict `FA-SEC01-22` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [crates/palette/Cargo.toml](../../../crates/palette/Cargo.toml#L14) (`curvebook-math`) and [crates/platform/Cargo.toml](../../../crates/platform/Cargo.toml#L14) (`serde`). Confirmed zero usage via workspace `rg`, and observed both compiler warnings (`warning: unused dependency 'curvebook-math'` and `warning: unused dependency 'serde'`) during `cargo test --locked --offline -p curvebook-palette -p curvebook-platform`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (build warning and unnecessary manifest edge cleanup).
- **Still present on main:** yes

### Verdict `FA-SEC01-23` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [fs.rs](../../../crates/platform/src/fs.rs#L157-L162) checks only `lmeta.is_dir()` in `resolve_atomic_target` (`meta.is_dir()` in `resolve_symlink_target`). Dynamic `probe_23_write_atomic_replaces_fifo`: calling `write_atomic` on an existing Unix FIFO succeeded and replaced the FIFO node with a regular file via rename instead of rejecting with `PlatformError::InvalidPath`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (`rename` replaces the directory entry cleanly without opening or blocking on the FIFO/device).
- **Still present on main:** yes

### Verdict `FA-SEC01-24` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [gamma.rs](../../../crates/math/src/special/gamma.rs#L227-L228) evaluates `lg.exp()` before dividing `PI / (sin_pix * lg.exp())`. Dynamic `probe_20_21_24`: `gamma(-173.5) == 0.0` and `gamma(-172.5) == 0.0` (vs `python3` `math.gamma(-173.5) = 6.45408359e-315` and `-1.11978350329e-312`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (representable subnormal collapse over a narrow `(-175.5..-171.5)` band).
- **Still present on main:** yes
