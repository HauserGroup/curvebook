# SEC-01 Audit Report — Numerical Foundations

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `crates/math` (`24` `.rs` + `Cargo.toml`), `crates/palette` (`6` `.rs` + `Cargo.toml`), `crates/platform` (`5` `.rs` + `Cargo.toml`) — `35` source files (`14,756` LOC) + `3` manifests (`51` LOC) = `38` files (`14,807` lines total)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-core` (report-only; line `1..=N` coverage on all `38` files)
- **Summary Counts:** `S1 = 0`, `S2 = 5`, `S3 = 14`, `S4 = 5` (Total: `24` findings)
- **Prior Remediation Check (`docs/audit/local-code-audit-2026-11-18.md`):**
  - Closed findings `#1`–`#126` affecting `SEC-01` (`#64` in `qr.rs`, `#65` in `svd.rs`, `#82` in `pvalue.rs`): **0 regressions** (regression fixes and permanent tests verified intact and passing).
  - Unnumbered open trace notes from `local-code-audit-2026-11-18.md`: all `3` (`noncentral.rs:382`, `noncentral.rs:642-653`, `quad.rs:230`) remain present and are recorded below (`FA-SEC01-15`, `FA-SEC01-04`, `FA-SEC01-14`).

## Baseline Verification Summary

- `cargo test --locked --offline -p curvebook-math -p curvebook-palette -p curvebook-platform` (with `CARGO_TARGET_DIR=/tmp/cb-sec01-rev`): exited `0`. Observed `0` failures across:
  - `curvebook-math`: `50` unit tests + `1` doctest + `43` integration tests (`audit_foundation_math`: `6`, `audit_r1_s4a_math`: `2`, `audit_r1_s4b_math`: `2`, `audit_r1_s4b_math_spec`: `4`, `audit_r1_s4c_math`: `1`, `d1_distribution_tails`: `8`, `nist_strd`: `9`, `rv_ledger_math`: `10`, `stage3a_s01_fixes`: `1`), all passed.
  - `curvebook-palette`: `15` unit tests + `5` integration tests (`audit_r1_s4b_palette`) + `1` doctest, all passed. Compiler emitted `unused dependency curvebook-math` on `crates/palette/Cargo.toml:14`.
  - `curvebook-platform`: `33` unit tests + `0` doctests, all passed. Compiler emitted `unused dependency serde` on `crates/platform/Cargo.toml:14`.
- `cargo clippy --locked --offline -p curvebook-math -p curvebook-palette -p curvebook-platform --all-targets`: exited `0`; emitted `unused dependency serde` on `crates/platform/Cargo.toml:14`.
- `cargo check --target wasm32-unknown-unknown`: blocked locally because the `wasm32-unknown-unknown` `core` sysroot is not installed (`E0463`); `wasm32` cleanliness (`no` `Instant`/`fs`/`process`, `#[cfg(not(target_arch = "wasm32"))]` gating in `platform`, pure `std` in `math`/`palette`) was verified by source inspection, while `usize` width arithmetic hazards on `wasm32` are noted in `FA-SEC01-17`.
- Numerical checks were cross-verified independently against Python 3 `math`/`numpy` and R (`Rscript`, `/usr/local/bin/Rscript`).

---

## File Coverage Table

| # | File | Lines | Inspected | Concrete Invariants & Dimensions Verified | Findings |
|---|---|---:|---|---|---|
| 1 | `crates/math/Cargo.toml` | 17 | `1..=17` | `[lints] workspace = true`; pure `serde` runtime dep, `curvebook-testkit` dev-dep; zero network/`chrono`/OS deps (`wasm32` clean). | Clean — manifest & lints verified |
| 2 | `crates/palette/Cargo.toml` | 19 | `1..=19` | `[lints] workspace = true`; zero network/`chrono`/OS deps (`wasm32` clean); `curvebook-math` dep unused across crate targets. | `FA-SEC01-22` |
| 3 | `crates/platform/Cargo.toml` | 15 | `1..=15` | `[lints] workspace = true`; `serde_json` only needed; `serde` direct dep unused; zero HTTP/network/`chrono` deps. | `FA-SEC01-22` |
| 4 | `crates/math/src/error.rs` | 80 | `1..=80` | `MathError` covers empty, dimension, parameter, domain, singular, non-SPD, and non-convergence without panics; `Display`/`Error` impls verified. | Clean — structured error surface & formatting verified |
| 5 | `crates/math/src/lib.rs` | 258 | `1..=258` | `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`; `AxisScale` domain checks & `normalize_in_domain` finiteness guards; `Logit(Percent)` underflow & saturated inverse endpoint rounding. | `FA-SEC01-21` |
| 6 | `crates/math/src/pvalue.rs` | 326 | `1..=326` | `#82` closed (`stars_at_alpha` + `audit_82_*` verified); `ConfidenceLevel` bounds `[0.01, 99.99]` on ctor/deser; `PValueFormat::Apa` lacks `p < 1.0` guard. | `FA-SEC01-20` |
| 7 | `crates/math/src/quad.rs` | 261 | `1..=261` | GL-32/64 & GK-15 nodes/weights compensated via `NeumaierSum`; interval cap checked; `16.0x` tolerance relaxation & `0.5 * (a ± b)` midpoint/half-width overflow. | `FA-SEC01-14` |
| 8 | `crates/math/src/quantile.rs` | 111 | `1..=111` | Hyndman-Fan (1996) Def. 7 Type-7 interpolation; empty/`NaN`/out-of-range `p` returns `None`; overflow-safe `(1 - frac) * lo + frac * hi` fallback on extreme spans. | Clean — Def. 7 math, empty/non-finite guards & span fallback verified |
| 9 | `crates/math/src/rng.rs` | 244 | `1..=244` | `SplitMix64` seeding, `xoshiro256**` open-interval `(0, 1)` generation, Durstenfeld shuffle, Box-Muller; PTRS `ln_pmf` ULP cancellation at `lambda in 1e12..=1e15`. | `FA-SEC01-01` |
| 10 | `crates/math/src/roots.rs` | 176 | `1..=176` | Brent-Dekker bracketing, `MAX_BRENT_ITERS = 4_096` cap, `safeguarded_halley_step` denominator clamping & bounds check; `0.5 * (x_c - x_b)` / `0.5 * (lo + hi)` overflow on extreme brackets. | `FA-SEC01-14` |
| 11 | `crates/math/src/sum.rs` | 480 | `1..=480` | `NeumaierSum`, `dot` FMA error compensation, Chan-Golub-LeVeque `SummaryMoments` two-pass scaling; `pearson_r` `(css_x * css_y).sqrt()` overflow/underflow; `#[serde(skip)]` on `scaled_css`. | `FA-SEC01-02`, `FA-SEC01-18` |
| 12 | `crates/math/src/dist/mod.rs` | 33 | `1..=33` | Public re-exports for `core`, `dunnett`, `lilliefors`, `noncentral`, `studentized_range`; zero logic or panics. | Clean — module exports verified |
| 13 | `crates/math/src/dist/core.rs` | 628 | `1..=628` | Normal, `t`, `F`, `chi2` densities, direct `sf`/`ln_sf`, & quantiles (`D1` A&S/DLMF citations); missing `df1 = +Inf` handling & ratio overflow in `F`, Cauchy `t` extreme tail failure. | `FA-SEC01-10` |
| 14 | `crates/math/src/dist/dunnett.rs` | 343 | `1..=343` | One/two-sided Dunnett 1-factor conditioning & Brent critical solver (`D1` Dunnett 1955/1964); loading saturation at `ni / n_control >= 1e17`; inherits `df < 2` `min_gap` bug. | `FA-SEC01-07`, `FA-SEC01-17` |
| 15 | `crates/math/src/dist/lilliefors.rs` | 321 | `1..=321` | Input finiteness & monotonic-sort validation; Dallal-Wilkinson/seeded Monte Carlo quantile grid + PCHIP `p`-value interpolation clamped to `[0, 1]`. | Clean — `D1` citations, sort validation & monotone PCHIP verified |
| 16 | `crates/math/src/dist/noncentral.rs` | 976 | `1..=976` | Benton-Krishnamoorthy / Lenth mixture & CI solvers; small-`df` Gaussian misroute & `t*t` overflow, `ncf`/`ncchi2` lower-tail `0.0` collapse, `1 - cdf` tail, `obs == 0` bypassing `df` check. | `FA-SEC01-03`, `FA-SEC01-04`, `FA-SEC01-11`, `FA-SEC01-15`, `FA-SEC01-16` |
| 17 | `crates/math/src/dist/studentized_range.rs` | 770 | `1..=770` | Tukey `Q` & max-range quadrature, log-`sf` deep tails, Bonferroni bounds; `df < 2.0` `min_gap = 0.02` skipping `t_star`; `c*(c-1)` / `k as i32` `wasm32` overflow. | `FA-SEC01-07`, `FA-SEC01-17` |
| 18 | `crates/math/src/linalg/mod.rs` | 19 | `1..=19` | Public re-exports for matrix/decomposition types and solvers; zero logic or panics. | Clean — module exports verified |
| 19 | `crates/math/src/linalg/cholesky.rs` | 322 | `1..=322` | Scale-invariant Cholesky SPD check & solve/inverse; `sym_eigen` fails silently when `frobenius_norm()` overflows `+Inf` (symmetry check, rotations & convergence skipped). | `FA-SEC01-08` |
| 20 | `crates/math/src/linalg/matrix.rs` | 327 | `1..=327` | `MAX_MATRIX_ELEMENTS = 100_000_000` alloc cap, `checked_mul`, safe `.get()`/`.entry()`, rescaled `col_norm2_from`/`frobenius_norm`; `set()` error overflows `r.saturating_mul(ncols) + c`. | `FA-SEC01-06` |
| 21 | `crates/math/src/linalg/qr.rs` | 971 | `1..=971` | `#64` closed (col-equilibrated pivoted QR, Bjorck refinement, SVD rank match); extreme-exponent overflow/NaN in `parameter_dependencies`, `refined_covariance`, `col_norm2`, Householder `tau`. | `FA-SEC01-09` |
| 22 | `crates/math/src/linalg/svd.rs` | 430 | `1..=430` | `#65` closed (`col_noise_tol`, `complete_orthonormal_columns` verified); `frob_a == +Inf` makes `col_noise_tol = +Inf`, skipping all Jacobi rotations. | `FA-SEC01-08` |
| 23 | `crates/math/src/special/erf.rs` | 291 | `1..=291` | Cody Chebyshev `erf`/`erfc`/`erfcx`, normal tail rational + Halley inversion; `erfc_inv(q)` halves `0.5 * q` so subnormal `5e-324` collapses to `+Inf`. | `FA-SEC01-13` |
| 24 | `crates/math/src/special/gamma.rs` | 405 | `1..=405` | Stirling error, Loader `deviance_bd0`, `aln1p_minus_arg`, `ln_gamma`, `ln_beta`, `digamma`, `ln_choose` verified; `gamma(x)` reflection collapses negative subnormals `-175.5..-171.5` to `±0.0`. | `FA-SEC01-24` |
| 25 | `crates/math/src/special/inc_beta.rs` | 501 | `1..=501` | Lentz CF + `DLMF 8.17.4` symmetry + log-space Halley `reg_beta_inv_pair`; `ln_beta_kernel` & CF numerator/denominator overflow `a*b` and `O(a^2)` products for `a, b >= 1e155`. | `FA-SEC01-11` |
| 26 | `crates/math/src/special/inc_gamma.rs` | 708 | `1..=708` | Temme uniform expansion + small-`a` series + Legendre CF + log-space inverses; `gamma_deviance` overflows `a + x` to `+Inf` returning `0.0` (`reg_gamma_p(1e308, 1.2e308) == 0.5`). | `FA-SEC01-12` |
| 27 | `crates/math/src/special/mod.rs` | 14 | `1..=14` | Public re-exports for `erf`, `gamma`, `inc_beta`, `inc_gamma`; zero logic or panics. | Clean — module exports verified |
| 28 | `crates/palette/src/categorical.rs` | 192 | `1..=192` | Okabe-Ito, Paul Tol (Bright/Muted/Vibrant/HighContrast), Monochrome; safe modulo indexing on `color_at` with white fallback on empty; CVD projection & min pairwise `Delta E_OK`. | Clean — palette constants & safe indexing verified |
| 29 | `crates/palette/src/colormap.rs` | 508 | `1..=508` | Viridis/Cividis/Plasma/Diverging/Grayscale stops, custom `ColorRamp`; `MAX_CUSTOM_STOPS = 256` in constructor and deserializer, `MAX_DISCRETE_SAMPLES = 4_096`, `NaN`/`Inf` clamp. | Clean — bounded alloc, serde limits & ramp sampling verified |
| 30 | `crates/palette/src/cvd.rs` | 175 | `1..=175` | Machado-Oliveira-Fernandes CVD linear matrices & BT.709 grayscale; `severity` sanitized to `[0, 1]`; canonical lowercase serde with PascalCase alias. | Clean — matrices, clamping & serde compatibility verified |
| 31 | `crates/palette/src/lib.rs` | 292 | `1..=292` | `#![forbid(unsafe_code)]`; `Rgba8` WCAG 2.1 luminance & contrast math, safe indexing/re-exports; (`crates/palette` uses `#![forbid(unsafe_code)]` without `#![deny(missing_docs)]`, acceptable per workspace clippy). | Clean — public facade & contrast math verified |
| 32 | `crates/palette/src/space.rs` | 337 | `1..=337` | Bounded UTF-8 safe `from_hex` (`.get()`), exact IEC 61966-2-1 sRGB/Oklab/CMYK transforms & clamping; `Oklab::to_oklch` `rem_euclid(360.0)` can emit `360.0` outside `[0, 360)`. | `FA-SEC01-21` |
| 33 | `crates/palette/src/style.rs` | 85 | `1..=85` | `MarkerShape`, `MarkerFillMode`, `LineDashStyle` (`#[non_exhaustive]`/defaults/serde); bounded stroke dash patterns. | Clean — enums, serde & finite dash invariants verified |
| 34 | `crates/platform/src/fs.rs` | 626 | `1..=626` | Sibling atomic staging, `sync_all` + `sync_dir`, permission preservation, `MAX_SYMLINK_HOPS`, pre/post-open `is_file()` + bounded `File::take`; `resolve_atomic_target` allows non-regular non-dir targets. | `FA-SEC01-23` |
| 35 | `crates/platform/src/lib.rs` | 119 | `1..=119` | `#![forbid(unsafe_code)]`; clean `cfg(not(target_arch = "wasm32"))` platform gating, typed `PlatformError`/`TimeoutKind` `Display` + `Error`. | Clean — `unsafe` forbid & `wasm32` gating verified |
| 36 | `crates/platform/src/path_guard.rs` | 428 | `1..=428` | `WorkdirGuard` component lexical + `canonicalize` containment, symlink & dangling-symlink tail checks, `ensure_regular_file`, recursive JSON `PATH_PARAM_KEYS` rewrite preserving inline CSV/plate data. | Clean — traversal, symlink escape & JSON rewrite invariants verified |
| 37 | `crates/platform/src/process.rs` | 1225 | `1..=1225` | `ProcessCommand` env allowlist + secret/loader stripping + safe `PATH`, direct exec (no shell), pgid teardown, stream caps (`64 MiB`), wall/idle timeouts, zero network egress. | Clean — env/arg sanitization, stream bounds & pgid teardown verified |
| 38 | `crates/platform/src/sync.rs` | 1774 | `1..=1774` | Re-entry & AB/BA wait-cycle detection, writer-priority reader fairness, poisoning recovery, `wasm32` single-thread fallback; `warn_limit > wait_timeout` sleep overrun; unguarded `waiting_on` across `on_contention` panic. | `FA-SEC01-05`, `FA-SEC01-19` |

---

## Findings (`S1` -> `S4`)

### `FA-SEC01-01` · `S2` · Hörmann PTRS `ln_pmf` suffers catastrophic cancellation at large `lambda` (`1e12..=1e15`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/rng.rs:176`
- **Category:** `wrong-result`
- **Defect:** `DeterministicRng::poisson` evaluates the PTRS acceptance test using `(k_f * lambda.ln()) - lambda - ln_factorial_approx(k)`, cancelling `O(1e16)` terms down to an `O(1)` log-probability where IEEE-754 `f64` rounding is `4.0`–`8.0` units.
- **Failure scenario:** `DeterministicRng::poisson(lambda)` documents and validates `0.0..=1e15` (`rng.rs:137-139`). At `lambda = 1e15` in the rejection region (`us < 0.07`, e.g. `u = 0.45`, `k_f = 1_000_000_071_760_245`), `k_f * lambda.ln()` is `~3.454e16` (`ulp = 8.0`), evaluating `ln_pmf = -20.0` instead of the exact `-20.763093` (`0.763` error, `2.145x` acceptance ratio distortion). Across `9,751` `(u, v)` grid points in that tail, `27.94%` (`2,724`) of accept/reject decisions flip, silently distorting generated Poisson distributions (reachable via `curvebook-transform/src/randomize.rs`).
- **Evidence:**
  ```rust
  // crates/math/src/rng.rs:174-179
  let k = k_f as u64;
  let ln_accept = (v * inv_alpha / (a / (us * us) + b)).ln();
  let ln_pmf = (k_f * lambda.ln()) - lambda - ln_factorial_approx(k);
  if ln_accept <= ln_pmf {
      return Ok(k);
  }
  ```
  Python 3 `f64` simulation at `lam = 1e15, u = 0.45`: `naive ln_pmf=-20.000000000000000, exact ln_pmf=-20.763093089510527, err=0.763, naive ratio exp(err) = 2.145x`; `decision flips out of 9751: 2724 (27.94%)`.
- **Suggested fix:** Replace `(k_f * lambda.ln()) - lambda - ln_factorial_approx(k)` with Loader's cancellation-free deviance form `-deviance_bd0(k_f, lambda) - 0.5 * (TAU * k_f).ln() - stirling_err(k_f)` (already provided in `crate::special::gamma`).
- **Related:** `crates/transform/src/randomize.rs` (`sample_distribution_value`).

---

### `FA-SEC01-02` · `S2` · `pearson_r` overflows and underflows `(css_x * css_y).sqrt()` inside the unscaled `(1e-140, 1e130)` band

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/sum.rs:358`
- **Category:** `wrong-result`
- **Defect:** `pearson_r` multiplies `css_x * css_y` before taking `.sqrt()`, computing an intermediate proportional to `deviation^4` even though `pow2_scale_for_max_abs` (`sum.rs:114`) intentionally leaves inputs inside `(1e-140, 1e130)` unscaled (`scale = 1.0`).
- **Failure scenario:** On valid finite vectors well within the unscaled domain:
  1. For `x = y = [-1e80, 1e80]`, `css_x = css_y = 2e160` and `css_x * css_y` overflows to `+Inf`, returning `pearson_r == 0.0` instead of `1.0`.
  2. For `x = [-1e-85, 1e-85, 0.0, 0.0], y = [-1e-85, 0.0, 1e-85, 0.0]` (`true r = 0.5`), `css_x * css_y` underflows to `0.0`, so `cross_s / 0.0 = +Inf` clamps to `+1.0` instead of `0.5`; for orthogonal vectors of magnitude `1e-85` (`cross_s == 0.0`), `0.0 / 0.0` returns `NaN` (`NaN.clamp(-1.0, 1.0) == NaN`).
- **Evidence:**
  ```rust
  // crates/math/src/sum.rs:114, 356-359
  if max_abs == 0.0 || !max_abs.is_finite() || (max_abs > 1e-140 && max_abs < 1e130) {
      return (1.0, 1.0);
  }
  ...
  let (sx, sy, cross_s) = scaled_cross_sum(x, mx.mean, y, my.mean);
  let scale_ratio = (sx / smx) * (sy / smy);
  let denom = (css_x * css_y).sqrt();
  Ok(((cross_s * scale_ratio) / denom).clamp(-1.0, 1.0))
  ```
  Python verification: `mag=1e+80: css=2.000e+160, css^2=inf, denom=inf, r=0.0`; `mag=1e-85: css=2.000e-170, css^2=0.000e+00, denom=0.0, r=inf` (`1.0` after clamp, or `NaN` if `cross_s == 0.0`).
- **Suggested fix:** Compute `let denom = css_x.sqrt() * css_y.sqrt();` (splitting `(cross_s / denom_x_or_y)` or normalizing `cross_s` by `css_x.sqrt()` and `css_y.sqrt()` independently with explicit power-of-2 scaling), avoiding `deviation^4` squaring.
- **Related:** `crates/stats/src/multivariate.rs`, `crates/fit/src/diagnostics.rs`.

---

### `FA-SEC01-03` · `S2` · `nct_positive_t_large_delta_quad` misapplies large-`df` Gaussian approximation to small `df` (`|delta| > 1e9`) and overflows `t * t * var_u`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/dist/noncentral.rs:235,262`
- **Category:** `wrong-result`
- **Defect:** Line `235` triggers the two-moment Gaussian approximation for `U = sqrt(chi2_df / df)` whenever `df > 1e6 || delta.abs() > 1e9`, applying an approximation whose documented `<= 4.70e-5` bound explicitly requires `df > 1e6` to arbitrarily small `df`; additionally line `262` squares `t * t`, overflowing finite `|t| >= 1e155`.
- **Failure scenario:**
  1. At `df = 1.0, delta = 2e9, t = 1e10`, `U` is a folded standard normal `|Z|` (skewness `~0.995`, not Gaussian). `nct_cdf(1e10, 1.0, 2e9)` computes `Phi(0.991829) = 0.839359` instead of R's `pt(1e10, 1, ncp=2e9) = 0.7816617`, an absolute error of `0.0577` (>1,200x above the comment's `4.70e-5` bound).
  2. At `t = 1e160` (e.g., `df = 1e7, delta = 1.0` or `df = 1.0, delta = 2e9`), `t * t` overflows to `+Inf`, evaluating `(1e160 * eu) / Inf = 0.0` so `nct_cdf` returns `0.5` instead of `1.0` (or `NaN` when `t = 1e308` overflows `t * eu` -> `Inf / Inf`), creating a non-monotone drop from the `t = +Inf -> 1.0` endpoint.
- **Evidence:**
  ```rust
  // crates/math/src/dist/noncentral.rs:235, 262-263
  if df > 1e6 || delta.abs() > 1e9 {
      ...
      let z_eff = (t * eu - delta) / (1.0 + t * t * var_u).sqrt();
      return Ok(if upper { norm_sf(z_eff) } else { norm_cdf(z_eff) });
  }
  ```
  Oracle check (`Rscript`): `pt(1e10, 1, ncp=2e9) = 0.7816617` vs `0.839359` (`|err| = 0.0577`); `pt(1.6e9, 1, ncp=2e9) = 0.2397501` vs `0.226623` (`|err| = 0.0131`); Python `t=1e160`: `denom=inf, z_eff=0.0, approx_cdf=0.500000`.
- **Suggested fix:** Restrict the Gaussian `U` approximation strictly to `df > 1e6` (evaluating small-`df` large-`|delta|` via the `s`-space quadrature/saddlepoint centered at the integrand peak `t * e^s ~= delta`), and replace `(1.0 + t * t * var_u).sqrt()` with `1.0_f64.hypot(t * var_u.sqrt())` (handling `|t * var_u.sqrt()| -> Inf` analytically via `(eu / var_u.sqrt()).copysign(t)`).
- **Related:** `crates/stats/src/power.rs`, `crates/stats/src/two_group.rs`, `crates/stats/src/column.rs`.

---

### `FA-SEC01-04` · `S2` · `ncf_cdf` and `ncchi2_cdf` mode-centered recurrences collapse far lower tails (`1e-104..1e-87`) to `0.0`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/dist/noncentral.rs:619-652,739-775`
- **Category:** `wrong-result`
- **Defect:** Both `ncf_eval` (`619-652`) and `ncchi2_eval` (`739-775`) seed their downward recurrence in linear `f64` space at the Poisson mode `k0 = floor(lambda / 2)` (`i0, g0` / `g_fn0, step0`). When those central terms underflow `< 5e-324` (`0.0`) while `k = 0..50` carry normal `f64` probability mass, the backward loop preserves `0.0` and drops the entire distribution tail.
- **Failure scenario:** For valid normal-range tail probabilities:
  - `ncf_cdf(1.0, 1.0, 100.0, 500.0)`: `ln(g0) = -1025.9` (`i0 = 0.0, g0 = 0.0`), so `ncf_cdf` returns exact `0.0` instead of `4.763135e-101` (exact Poisson-Beta log-sum-exp).
  - `ncf_cdf(0.05, 1.0, 10.0, 400.0)`: `ln(g0) = -1045.3`, returning `0.0` instead of `3.559417e-87`.
  - `ncchi2_cdf(0.5, 1.0, 500.0)`: `g_fn0 = 0.0, step0 = 0.0`, returning `0.0` instead of R's `pchisq(0.5, 1, ncp=500) = 2.812314e-104`.
- **Evidence:**
  ```rust
  // crates/math/src/dist/noncentral.rs:618-620, 646-648
  let a0 = 0.5 * df1 + k0_f;
  let i0 = if upper { reg_beta_eval_safe(y, x, b, a0)? } else { reg_beta_eval_safe(x, y, a0, b)? };
  let g0 = (ln_beta_kernel(x, y, a0, b)? - a0.ln()).exp();
  ...
      gk_b *= (a_prev + 1.0) / (x * (a_prev + b));
      ik_b = if upper { (ik_b - gk_b).max(0.0) } else { (ik_b + gk_b).min(1.0) };
      total += pk_b * ik_b;
  ```
  And identically in `ncchi2_eval` (`noncentral.rs:738-742, 768-770`). Python exact Poisson-Beta sim & R check: `f=1, df=(1,100), lam=500 -> exact=4.763135e-101, Rust sim=0.0`; `f=0.05, df=(1,10), lam=400 -> exact=3.559417e-87, Rust sim=0.0`; `R pchisq(0.5, 1, ncp=500) = 2.812314e-104`.
- **Suggested fix:** When `i0 == 0.0 && g0 == 0.0` (or `g_fn0 == 0.0 && step0 == 0.0`) on `!upper`, fall back to upward summation from `k = 0` or accumulate log-terms (`log1p`/log-sum-exp via `ln_reg_beta_xy` / `ln_reg_gamma_p`) anchored at the largest `ln(p_k * I_k)` term.
- **Related:** Carried-over unnumbered trace note in `docs/audit/local-code-audit-2026-11-18.md:634` (`ncf_cdf(1, 1, 100, 500)` returns `0`; not part of closed `#1`–`#126`).

---

### `FA-SEC01-05` · `S2` · `sync::Mutex` / `sync::RwLock` sleeps until `warn_limit` before honoring `wait_timeout` when `warn_limit > wait_timeout`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/platform/src/sync.rs:1200`
- **Category:** `resources`
- **Defect:** `LockLimits::new(warn_limit, wait_timeout)` (`sync.rs:56`, exposed via `Mutex::with_limits` and `RwLock::with_limits`) does not enforce `warn_limit <= wait_timeout`, and the contention wait loop selects `next_deadline = core.limits.warn_limit` without bounding it by `core.limits.wait_timeout`.
- **Failure scenario:** A caller constructs `Mutex::with_limits("m", v, Duration::MAX, Duration::from_millis(50))` (for instance to suppress warning reports while enforcing a `50ms` acquire timeout). On contention (`!warned`), line `1200` sets `next_deadline = Duration::MAX`, causing `core.wait_for_seq_change(observed_seq, step)` (`sync.rs:1203`) to sleep until `warn_limit` (potentially indefinitely or until an unrelated holder drops and bumps `wait_seq`) instead of returning `LockError::TimedOut` after `50ms`.
- **Evidence:**
  ```rust
  // crates/platform/src/sync.rs:1196-1204
  if elapsed >= core.limits.wait_timeout {
      return handle_timeout(core, opts, tid, &thread_label, snapshot.writer_blocking_shared, wait_started, now, &mut try_raw);
  }

  let next_deadline = if warned { core.limits.wait_timeout } else { core.limits.warn_limit };
  let step = next_deadline.saturating_sub(elapsed);
  if !step.is_zero() {
      core.wait_for_seq_change(observed_seq, step);
  }
  ```
- **Suggested fix:** Clamp `next_deadline`: `let next_deadline = if warned { core.limits.wait_timeout } else { core.limits.warn_limit.min(core.limits.wait_timeout) };` (and avoid `emit_contention_report` in `handle_timeout` or document `warn_limit > wait_timeout` suppression cleanly).
- **Related:** `LockLimits::new` (`crates/platform/src/sync.rs:56`), `with_limits` (`sync.rs:332, 563`).

---

### `FA-SEC01-06` · `S3` · `DenseMatrix::set` out-of-bounds error formatting panics on integer overflow

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/linalg/matrix.rs:139`
- **Category:** `no-panic`
- **Defect:** In the out-of-bounds error path of `DenseMatrix::set`, `r.saturating_mul(ncols) + c` uses unchecked `+ c` after a saturating multiply.
- **Failure scenario:** Calling `m.set(usize::MAX, 1, 0.0)` (or any `(r, c)` with `r * ncols >= usize::MAX - c`) on any non-empty matrix (`nrows >= 1, ncols >= 1`) saturates `r.saturating_mul(ncols)` to `usize::MAX` and then panics in debug builds with `attempt to add with overflow` (violating `no-panic` / `wasm32` abort safety) or wraps `actual` to `0` in release builds.
- **Evidence:**
  ```rust
  // crates/math/src/linalg/matrix.rs:138-140
  let Some(slot) = self.get_mut(r, c) else {
      return Err(MathError::DimensionMismatch { expected: nrows * ncols, actual: r.saturating_mul(ncols) + c });
  };
  ```
- **Suggested fix:** Replace `+ c` with `.saturating_add(c)`.
- **Related:** `none`.

---

### `FA-SEC01-07` · `S3` · Fixed `min_gap = 0.02` in `integrate_over_chi_scale_tail` drops `t_star` knots for `df < 2.0`, causing up to 64% tail error

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/dist/studentized_range.rs:252`
- **Category:** `wrong-result`
- **Defect:** For `df < 2.0`, `integrate_over_chi_scale_tail` passes `knots = [0.2 * t_star, t_star, 0.2, 1.0, t_mid]` into `sum_gl32_panels(..., 0.02)` with a hardcoded `min_gap = 0.02`, silently skipping `[0, 0.2 * t_star]` and `[0.2 * t_star, t_star]` panel splits whenever `t_star <= 0.02`.
- **Failure scenario:** At `df < 2.0` and large `q` (where `t_star = (df / (df + rate))^{df / 4} <= 0.02`), `sum_gl32_panels` integrates `[0, 0.2]` in a single GL-32 panel despite the integrand peak concentrated near `t_star`. At `k = 2, df = 1.9, q = 5000` (`t_star = 0.000577`), the tail probability evaluates to `1.9924e-7` vs `1.7104e-7` (`16.5%` relative error); at `df = 1.99, q = 5000` (`t_star = 0.000415`), it returns `1.3529e-7` vs `8.2329e-8` (`64.3%` relative error). Affects both `studentized_range_*` and `dunnett_*` (`dunnett.rs:137, 170`).
- **Evidence:**
  ```rust
  // crates/math/src/dist/studentized_range.rs:55-60, 251-252
  for &x in sorted.get(..count).unwrap_or(&[]) {
      if x > cur + min_gap && x + min_gap < hi {
          total += gauss_legendre_32(&mut f, cur, x);
          cur = x;
      }
  }
  ...
  let knots = [0.2 * t_star, t_star, 0.2, 1.0, t_mid];
  return Ok(sum_gl32_panels(mapped, 0.0, t_max, &knots, 0.02));
  ```
  Python GL-32 comparison (`k = 2`): `df=1.99 q=1000: t_star=0.002058, buggy=2.119396e-06, true=2.121566e-06, rel_err=1.023e-03`; `df=1.99 q=5000: t_star=0.000415, buggy=1.352894e-07, true=8.232949e-08, rel_err=6.433e-01`.
- **Suggested fix:** Scale `min_gap` proportionally to the smallest knot (`(0.05 * t_star).max(1e-12)`) and guard against collapsing intervals only when `(x - cur) <= 1e-12 * hi`.
- **Related:** `crates/math/src/dist/dunnett.rs:137,170`.

---

### `FA-SEC01-08` · `S3` · Unrescaled `frobenius_norm()` overflow to `+Inf` silently disables Jacobi rotations and symmetry checks in `svd` and `sym_eigen`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/linalg/svd.rs:89-90,146`, `crates/math/src/linalg/cholesky.rs:192-233`
- **Category:** `wrong-result`
- **Defect:** Although `DenseMatrix::frobenius_norm` (`matrix.rs:311`) rescales internally, matrices with finite entries whose Frobenius norm exceeds `f64::MAX` (`~1.797e308`) return `+Inf`, yet neither `one_sided_jacobi_svd` nor `sym_eigen` rescales the working matrix before thresholding against `frobenius_norm`.
- **Failure scenario:** Consider a finite symmetric `4x4` block-diagonal matrix made of two `[[9e307, 4e307], [4e307, 9e307]]` blocks (all entries `<= 9e307` and all exact singular/eigenvalues `[1.3e308, 1.3e308, 5e307, 5e307]` finite): `frobenius_norm() = 1.97e308 -> +Inf`.
  - `svd(&A)` computes `col_noise_tol = eps^2 * Inf = +Inf`, making `norm_p <= col_noise_tol` true for every pair (`svd.rs:146`), skipping all Jacobi rotations, setting `max_cosine = 0.0`, and returning un-rotated column norms and non-orthogonal `U`.
  - `sym_eigen(&A)` sets `scale = +Inf` (`cholesky.rs:193`), bypassing the asymmetry rejection `1e-11 * scale` (`cholesky.rs:206`), skipping Jacobi steps (also `d[p,p].abs() + d[q,q].abs()` overflows to `+Inf` at `cholesky.rs:225`), and reporting `converged = true` after Sweep 0 (`cholesky.rs:230`), returning diagonal `[9e307, 9e307, 9e307, 9e307]` and `V = I`.
- **Evidence:**
  ```rust
  // crates/math/src/linalg/svd.rs:89-90, 146
  let frob_a = a.frobenius_norm();
  let col_noise_tol = (f64::EPSILON * f64::EPSILON) * frob_a;
  ...
  if norm_p <= col_noise_tol || norm_q <= col_noise_tol {
      return 0.0;
  }

  // crates/math/src/linalg/cholesky.rs:192-194, 206, 225, 230
  let frob = a.frobenius_norm();
  let scale = if frob > 0.0 { frob } else { ... };
  ...
  if (a.entry(i, j) - a.entry(j, i)).abs() > 1e-11 * scale { ... }
  ...
  if apq.abs() > 0.25 * f64::EPSILON * (d.entry(p, p).abs() + d.entry(q, q).abs()).max(f64::MIN_POSITIVE) { ... }
  ...
  if max_off <= 2.0 * f64::EPSILON * scale { converged = true; }
  ```
- **Suggested fix:** Scale `A` by a power of two (`scale = 2^k` from `max_abs`) before Jacobi orthogonalization/diagonalization when `max_abs` exceeds `f64::MAX / max(m, n)` (or `frobenius_norm().is_infinite()`), and rescale `sigmas`/`eigenvalues` back afterward (returning `DomainError`/`Overflow` only if individual rescaled eigenvalues exceed `f64::MAX`).
- **Related:** `crates/stats/src/anova/{reml,factorial,one_way}.rs` (`sym_eigen`).

---

### `FA-SEC01-09` · `S3` · Unrescaled `qr.rs` operations overflow or produce `NaN` on extreme-scale matrices (`>= 1.35e154` or `<= 1e-310`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/linalg/qr.rs:143-160,514-518,567-573,621`
- **Category:** `wrong-result`
- **Defect:** Four paths in `qr.rs` multiply/divide unrescaled norms or scales directly in raw `f64` instead of factoring in scaled space.
- **Failure scenario:**
  1. `parameter_dependencies(2e154 * I_2)` (`qr.rs:514-518`) squares `col_norm` (`ata_kk = 4e308 -> +Inf`) before multiplying by `xtx_inv[k,k] = 2.5e-309`, returning `[1.0, 1.0]` (full collinearity) for an orthogonal matrix; at `1e162 * I_2`, `inv_kk` underflows to `0.0` so `Inf * 0.0 = NaN`, returning `[NaN, NaN]`.
  2. `solve_linear_least_squares` -> `refined_covariance` (`qr.rs:621`) computes `r_inv[i, j] / scale` on `X = 1e-310 * I_2`: `1.0 / 1e-310 = +Inf`, producing off-diagonal `0.0 * Inf = NaN` in `x.matmul(&preconditioner)` (`qr.rs:624`) and failing with `DomainError`.
  3. `solve_linear_least_squares` (`qr.rs:567-573`) checks `norm_j > 0.0` without `.is_finite()`, so a column `[1.5e308, 1.5e308]^T` (`col_norm2 = +Inf`) divides entries by `+Inf`, zeroing out the scaled column and falsely flagging it aliased.
  4. `compute_householder_column` (`qr.rs:143-160`) sets `beta = -Inf` when `col_norm2_from` overflows to `+Inf`, computing `(beta - alpha) / beta = -Inf / -Inf = NaN` for `tau`.
- **Evidence:**
  ```rust
  // crates/math/src/linalg/qr.rs:149, 160, 514-518, 567-572, 621
  let beta = if alpha >= 0.0 { -norm_x } else { norm_x }; ... (beta - alpha) / beta
  let col_norm = a.col_norm2(k); let ata_kk = col_norm * col_norm; let prod = ata_kk * inv_kk;
  let norm_j = x.col_norm2(j); if norm_j > 0.0 { ... x.entry(i, j) / norm_j ... }
  preconditioner.set_in_bounds(orig, j, r_inv.entry(i, j) / scale);
  ```
  Python IEEE-754 check: `k=2e154: ata_kk=inf, inv_kk=2.5e-309, dep=1.0`; `k=1e162: inv_kk=0.0, prod=nan, dep=nan`; `s=1e-310: 1/s=inf, 0.0*inf=nan`; `(-inf - 1.5e308)/-inf = nan`.
- **Suggested fix:** Equilibrate `parameter_dependencies` in scaled form (`pqr = qr_col_pivoted(&a_scaled)` where `(D^{-1} A^T A D^{-1})^{-1}` cancels `||A_{:,k}||_2^2` analytically so `prod == inv_kk_scaled`), scale columns by `max_abs` before `col_norm2`, skip/normalize sub-`f64::MIN_POSITIVE` `scales` in `refined_covariance`, and guard `!norm_x.is_finite()` in Householder reflectors.
- **Related:** `crates/stats/src/anova/factorial.rs`, `crates/transform/src/curve.rs`.

---

### `FA-SEC01-10` · `S3` · `dist::core` Snedecor `F` functions reject valid `df1 = +Inf`, overflow `(df1 / df2) * f`, and `student_t_isf` fails on representable Cauchy tails

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/dist/core.rs:297-298,375-490`
- **Category:** `wrong-result`
- **Defect:** `f_cdf`, `f_sf`, `f_ln_sf`, `f_ln_pdf`, `f_isf`, and `f_ppf` explicitly allow `+Inf` via `check_pos_param` and implement `df2.is_infinite()`, yet omit `df1.is_infinite()`, multiply `(df1 / df2) * f` / `(df2 / df1) * (y / x)` without exponent ordering, and `student_t_two_sided_isf` solves `x ~ df / (df + t^2)` even when `x` underflows `f64` for representable Cauchy `t`.
- **Failure scenario:**
  1. `f_cdf(0.01, f64::INFINITY, 10.0)` passes `0.5 * df1 = +Inf` to `reg_beta_xy`, returning `Err(InvalidParameter { name: "b", value: inf })` instead of `chi2_sf(10.0 / 0.01, 10.0)` (`1.870291e-208` in R `pf(0.01, Inf, 10)`).
  2. `f_cdf(1e200, 1e-200, 1e200)` underflows `(df1 / df2)` (`1e-400 -> 0.0`) before multiplying by `f = 1e200`, returning `0.0` for `ratio = 0.0` instead of `ratio = 1e-200`; `f_cdf(1e-200, 1e200, 1e-200)` overflows `df1 / df2 -> +Inf` returning `1.0` instead of `ratio = 1e200`. Same hazard affects `(df2 / df1) * (y / x)` in `f_isf` (`468`) and `f_ppf` (`489`).
  3. `student_t_isf(1e-200, 1.0)` delegates to `reg_beta_inv_pair(0.5, 0.5, 2e-200, 1.0)` (`297`), where the true `x ~ (pi * q)^2 ~ 1e-399` underflows `MIN_LN_X = -740` (`t_min` gives `I_{t_min}(0.5, 0.5) ~ 8.3e-162 > 2e-200`), failing with `Err(NotConverged)` even though the exact Cauchy quantile `1 / tan(pi * q / 2) ~ 6.366e199` fits comfortably in `f64`.
- **Evidence:**
  ```rust
  // crates/math/src/dist/core.rs:297-298, 386-390, 464-469
  let (x, y) = reg_beta_inv_pair(0.5 * df, 0.5, alpha, 1.0 - alpha)?;
  Ok((df * (y / x)).sqrt())
  ...
  if df2.is_infinite() {
      return chi2_cdf(df1 * f, df1);
  }
  let ratio = (df1 / df2) * f;
  ```
  Oracle check (`Rscript`): `pf(0.01, Inf, 10) = 1.870291e-208`; Python: `(1e-200 / 1e200) * 1e200 = 0.0` vs `(1e-200 * 1e200) / 1e200 = 1e-200`.
- **Suggested fix:** Handle `df1.is_infinite()` via reciprocal `chi2(df2)` identities (`f_cdf(f, Inf, df2) = if f <= 0 { 0 } else { chi2_sf(df2 / f, df2) }`, plus `(df1, df2) == (Inf, Inf)` step at `f = 1`); compute `(df1, df2, f)` ratios via `ln` or `log2`-exponent splitting when unscaled products/quotients overflow/underflow; and use closed-form `(0.5 * PI * q).tan().recip()` (and asymptotic `1/(pi*q)`) for `df == 1.0` in `student_t_*_isf`.
- **Related:** `none`.

---

### `FA-SEC01-11` · `S3` · `ln_beta_kernel` and `ln_beta_continued_fraction` overflow `a * b` and `O(a^2)` products for `a, b >= 1e155`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/special/inc_beta.rs:51,96,106-107` (and `crates/math/src/dist/noncentral.rs:132-136`)
- **Category:** `wrong-result`
- **Defect:** `ln_beta_kernel` (`inc_beta.rs:51`) computes `((a * b) / (TAU * s)).ln()` and `ln_beta_continued_fraction` (`inc_beta.rs:96, 106-107`) computes unnormalized products `(qam + m2) * (a + m2)` and `-(a + mf) * (qab + mf)` / `(a + m2) * (qap + m2)` instead of separating factors before multiplication.
- **Failure scenario:** For valid finite shapes `a = 1e155, b = 1e155` (e.g. `reg_beta(0.5, 1e155, 1e155)` = `0.5`), `a * b` overflows to `+Inf` making `scale = +Inf`, while `odd_denom = (a + m2) * (qap + m2) = +Inf` and `-(a + mf) * (qab + mf) = -Inf` produce `aa_odd = -Inf / +Inf = NaN`, causing `ln_beta_continued_fraction` to spin for `20,000` iterations and fail with `NotConverged`. Even the normal fallback `reg_beta_eval_safe` (`noncentral.rs:132-136`) overflows `s = a + b` to `+Inf` when `a = b = 1e308` (`pm = NaN`).
- **Evidence:**
  ```rust
  // crates/math/src/special/inc_beta.rs:49-51, 96, 106-107
  let s = a + b;
  let dev = -deviance_bd0(a, s * x) - deviance_bd0(b, s * y);
  let scale = 0.5 * ((a * b) / (std::f64::consts::TAU * s)).ln();
  ...
  let aa_even = mf * (b - mf) * x / ((qam + m2) * (a + m2));
  let odd_denom = (a + m2) * (qap + m2);
  let aa_odd = -(a + mf) * (qab + mf) * x / odd_denom;
  ```
- **Suggested fix:** Compute `0.5 * (a.ln() + b.ln() - s.ln() - TAU.ln())` (computing `s = a + b` via `half_a + half_b` / scaled ratio `a / b` when `a + b` overflows), divide `(a + mf)/(a + m2)` and `(qab + mf)/(qap + m2)` before multiplying in CF steps, and normalize `s` via `max(a, b)` in `reg_beta_eval_safe`.
- **Related:** `crates/math/src/dist/noncentral.rs:132-136` (`reg_beta_eval_safe`).

---

### `FA-SEC01-12` · `S3` · `gamma_deviance` overflows `a + x` to `+Inf` when `a + x > f64::MAX`, collapsing `reg_gamma_p(1e308, 1.2e308)` to `0.5`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/special/inc_gamma.rs:76-80`
- **Category:** `wrong-result`
- **Defect:** `gamma_deviance(a, x)` tests `(a - x).abs() < 0.45 * (a + x)` and evaluates `v = (a - x) / (a + x)` without protecting `a + x` against `f64` overflow.
- **Failure scenario:** At `a = 1e308, x = 1.2e308` (inside `|x - a| <= 0.25 * a`, so `reg_gamma_p`/`q` dispatches to `temme_uniform_pq`), `a + x` overflows to `+Inf`, making `(a - x).abs() < 0.45 * Inf` true. Then `v = -2e307 / Inf = -0.0`, `s = 0.0`, `2.0 * a = +Inf`, `ej = Inf * 0.0 = NaN`, terminating `for` at `s = 0.0` instead of `~0.193e308`. Consequently `temme_corr_and_z` sets `z_sq = 0.0, z = -0.0` and `reg_gamma_p(1e308, 1.2e308)` returns `0.5` instead of `1.0`.
- **Evidence:**
  ```rust
  // crates/math/src/special/inc_gamma.rs:75-80
  fn gamma_deviance(a: f64, x: f64) -> f64 {
      if (a - x).abs() < 0.45 * (a + x) {
          let v = (a - x) / (a + x);
          let v2 = v * v;
          let mut s = (a - x) * v;
          let mut ej = 2.0 * a * v * v2;
  ```
  Python IEEE-754 trace (`a = 1e308, x = 1.2e308`): `a+x: inf, cond: True, v: -0.0, s: 0.0, 2*a: inf, ej: nan`.
- **Suggested fix:** Compute `m = a.max(x)` and `v = ((a - x) / m) / (a / m + x / m)`, test `(a - x).abs() / m < 0.45 * (a / m + x / m)`, and keep `ej = 2.0 * v * v2` multiplied by `a` only at summation or factored via `a * ((s_scaled) + ...)` with overflow clamping.
- **Related:** `crates/math/src/special/inc_gamma.rs:179` (`temme_corr_and_z`).

---

### `FA-SEC01-13` · `S3` · `erfc_inv(q)` halves subnormal `q`, rounding `5e-324` to `0.0` and returning `+Inf`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/special/erf.rs:211`
- **Category:** `wrong-result`
- **Defect:** `erfc_inv(q)` converts `q in (0, 1)` to `half_tail = 0.5 * q` before passing it to `invert_normal_upper_tail`, flushing positive subnormal inputs `< 2 * f64::MIN_SUBNORMAL` (`5e-324`) to `0.0`.
- **Failure scenario:** Calling `erfc_inv(5e-324)` (where `5e-324` is a valid strictly positive finite subnormal `f64`) produces `half_tail = 0.0`, causing `invert_normal_upper_tail(0.0)` to evaluate `-2.0 * 0.0_f64.ln() = +Inf`, returning `Ok(+Inf)` instead of the finite root `~27.272` (`sqrt(-ln(sqrt(pi) * q / 2))`).
- **Evidence:**
  ```rust
  // crates/math/src/special/erf.rs:211-212
  let (half_tail, sign) = if q < 1.0 { (0.5 * q, 1.0) } else { (0.5 * (2.0 - q), -1.0) };
  Ok(sign * invert_normal_upper_tail(half_tail) * std::f64::consts::FRAC_1_SQRT_2)
  ```
  Python check: `0.5 * 5e-324 = 0.0`.
- **Suggested fix:** Work in log-space (`ln_half = q.ln() - LN_2`, or evaluate `invert_normal_upper_tail_ln(ln_half)`) whenever `q < 1e-300` so subnormal `q` never flushes to `0.0`.
- **Related:** `none`.

---

### `FA-SEC01-14` · `S3` · `integrate_adaptive` silently relaxes requested tolerance by `16x`, and quadrature/root midpoints overflow extreme brackets

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/quad.rs:99-100,112-113,208,230`, `crates/math/src/roots.rs:81,154`
- **Category:** `contract-drift`
- **Defect:** `integrate_adaptive` enforces `total_err <= tol` inside its loop (`quad.rs:201`) but exits with `Ok(refined)` whenever `total_err <= final_tol * 16.0` (`quad.rs:230`) without documenting this `16x` tolerance relaxation; additionally both `quad.rs` and `roots.rs` compute `0.5 * (a + b)` / `0.5 * (b - a)` directly, overflowing wide finite intervals.
- **Failure scenario:**
  1. When `max_subdivisions` (default `512`) exhausts with an estimated integration error between `1.1 * tol` and `15.9 * tol`, `integrate_adaptive` returns `Ok(refined)` violating the caller-configured `QuadOptions { abs_tol, rel_tol }` guarantee by up to `16x` without any warning or doc notice.
  2. Calling `gauss_legendre_32`, `gauss_kronrod_15_panel`, `integrate_adaptive`, or `brent_root` / `safeguarded_halley_step` on finite extreme spans such as `[-1e308, 1e308]` overflows `0.5 * (b - a)` / `0.5 * (x_c - x_b)` / `0.5 * (lo + hi)` to `+Inf`, yielding `+Inf` or `NaN`.
- **Evidence:**
  ```rust
  // crates/math/src/quad.rs:99-100, 208, 230
  let center = 0.5 * (a + b);
  let half = 0.5 * (b - a);
  ...
  let mid = 0.5 * (worst.a + worst.b);
  ...
  if total_err <= final_tol * 16.0 { Ok(refined) } else { Err(MathError::NotConverged { algorithm: "integrate_adaptive", iterations: cap }) }

  // crates/math/src/roots.rs:81, 154
  let mid = 0.5 * (x_c - x_b);
  ...
  0.5 * (lo + hi)
  ```
- **Suggested fix:** Either remove the `* 16.0` relaxation (enforce `total_err <= final_tol`) or document the `16x` fallback margin explicitly in `QuadOptions` / `integrate_adaptive`; replace `0.5 * (a + b)` and `0.5 * (b - a)` with `0.5 * a + 0.5 * b` and `0.5 * b - 0.5 * a` (and `(0.5 * x_c - 0.5 * x_b)` in `roots.rs`).
- **Related:** Carried-over unnumbered trace note in `docs/audit/local-code-audit-2026-11-18.md:637` (`quad.rs:230`); `crates/fit/src/dsl/special.rs:1003`.

---

### `FA-SEC01-15` · `S3` · `nct_sf` opposite tail (`t > 0, delta < 0`) evaluates `1.0 - cdf_val` with subtraction cancellation

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/dist/noncentral.rs:382`
- **Category:** `wrong-result`
- **Defect:** `nct_positive_t_eval` uses a dedicated cancellation-free complement sum `nct_bidirectional_upper_sum` when `upper && delta >= 0.0` (`376-378`), but falls back to `(1.0 - cdf_val).clamp(0.0, 1.0)` when `upper && delta < 0.0` (`382`).
- **Failure scenario:** Calling `nct_sf(2.0, 10.0, -10.0)` (or `nct_cdf(-2.0, 10.0, 10.0)` via symmetry `208`) evaluates `cdf_val = 1.0` in IEEE-754 `f64`, returning `0.0` instead of `~1.5e-29` (all significant digits lost for opposite-tail `p < 1e-16`).
- **Evidence:**
  ```rust
  // crates/math/src/dist/noncentral.rs:376-383
  if upper && delta >= 0.0 {
      return Ok(nct_bidirectional_upper_sum(k0, lambda_half, p_k0, q_k0, y, one_minus_y, b)?.clamp(0.0, 1.0));
  }

  let series_half = nct_bidirectional_lower_sum(k0, lambda_half, p_k0, q_k0, y, one_minus_y, b)?;
  let cdf_val = (norm_sf(delta) + 0.5 * series_half).clamp(0.0, 1.0);
  Ok(if upper { (1.0 - cdf_val).clamp(0.0, 1.0) } else { cdf_val })
  ```
- **Suggested fix:** Use `nct_bidirectional_upper_sum` for `upper && delta < 0.0` as well (replacing `norm_sf(delta)` by `norm_cdf(delta)` and summing positive `I_{1-y}(b, j + 0.5)` / negative-signed odd corrections or direct `t < 0` symmetry).
- **Related:** Carried-over unnumbered trace note in `docs/audit/local-code-audit-2026-11-18.md:632` (`noncentral.rs:382`; not part of closed `#1`–`#126`).

---

### `FA-SEC01-16` · `S3` · `noncentral_f_ci` and `noncentral_chi2_ci` skip `df` validation when `obs == 0.0`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/dist/noncentral.rs:667-669,789-791`
- **Category:** `contract-drift`
- **Defect:** Both `noncentral_f_ci` and `noncentral_chi2_ci` return `Ok((0.0, 0.0))` early on `f_obs == 0.0` / `chi2_obs == 0.0` before validating `df1`/`df2` or `df`, violating their documented `# Errors` contracts.
- **Failure scenario:** Calling `noncentral_f_ci(0.0, -1.0, f64::NAN, 95.0)` or `noncentral_chi2_ci(0.0, 0.0, 95.0)` silently returns `Ok((0.0, 0.0))` instead of `Err(MathError::InvalidParameter)` (downstream callers in `crates/stats` currently guard `df >= 1.0`, making this a latent public-API contract drift).
- **Evidence:**
  ```rust
  // crates/math/src/dist/noncentral.rs:660, 667-671, 782, 789-793
  /// Returns an error if `f_obs < 0`, `df1 <= 0`, `df2 <= 0`, or `confidence_percent` is out of `[0.01, 99.99]`.
  ...
  if f_obs == 0.0 {
      return Ok((0.0, 0.0));
  }
  let sf0 = f_sf(f_obs, df1, df2)?;
  ...
  /// Returns an error if `chi2_obs < 0`, `df <= 0`, or `confidence_percent` is outside `[0.01, 99.99]`.
  ...
  if chi2_obs == 0.0 {
      return Ok((0.0, 0.0));
  }
  let sf0 = chi2_sf(chi2_obs, df)?;
  ```
- **Suggested fix:** Validate `df > 0` (`df1 > 0 && df2 > 0`, non-NaN) before the `obs == 0.0` early return.
- **Related:** `crates/stats/src/anova/effect_sizes.rs:132`, `crates/stats/src/categorical.rs:1334`.

---

### `FA-SEC01-17` · `S3` · `wasm32` `usize`/`i32` overflow in `studentized_range` and `dunnett_loadings_from_sizes` rounding at `ni / n_control >= 1e17`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/dist/studentized_range.rs:121,125,459,590`, `crates/math/src/dist/dunnett.rs:42`
- **Category:** `no-panic`
- **Defect:**
  1. `studentized_range.rs` computes `(k * (k - 1)) / 2` and `(c * (c - 1)) / 2` in `usize` (overflowing on 32-bit `wasm32` when `k >= 65_537` with no upper bound on `k` or `c`), and casts `k as i32` / `(k - 1) as i32` (`121, 125`), wrapping `powi` exponents negative for `k > i32::MAX as usize`.
  2. `dunnett_loadings_from_sizes` (`dunnett.rs:42`) computes `(ni / (n_control + ni)).sqrt()` directly; when `ni / n_control >= 1e17`, `n_control + ni` rounds to `ni`, yielding `lambda = 1.0`, which downstream `validate_loadings_and_df` (`dunnett.rs:55`) immediately rejects as `InvalidParameter` (or overflows `n_control + ni` to `+Inf` when `ni = 1e308`).
- **Failure scenario:** Calling `studentized_range_sf(5.0, 70_000, 10.0)` on `wasm32` panics on `k * (k - 1)` arithmetic overflow; calling `dunnett_loadings_from_sizes(1.0, &[1e20])` returns `vec![1.0]`, which fails when passed to `dunnett_two_sided_p_loadings`.
- **Evidence:**
  ```rust
  // crates/math/src/dist/studentized_range.rs:121, 125, 459, 590
  let center_term = if center_prob > 0.0 { center_prob.powi(k as i32) } else { 0.0 };
  let km1_i = (k - 1) as i32;
  ...
  let pairs = (k * (k - 1)) / 2;
  ...
  let pairs_per_range = ((c * (c - 1)) / 2) as f64;

  // crates/math/src/dist/dunnett.rs:42, 55
  loadings.push((ni / (n_control + ni)).sqrt());
  ...
  if !lam.is_finite() || !(0.0..1.0).contains(&lam) { ... }
  ```
- **Suggested fix:** Compute `pairs` in `f64` (`0.5 * (k as f64) * ((k - 1) as f64)`) and replace `powi(k as i32)` with `powf(k as f64)` (or guard `k` vs `i32::MAX`); in `dunnett.rs`, compute `(1.0 / (1.0 + n_control / ni)).sqrt()` and accept `1.0` cleanly or pass `(lam, sqrt(1 - lam^2))` directly as `(n_i / (n_c + n_i), n_c / (n_c + n_i))`.
- **Related:** `crates/stats/src/multiplicity.rs`.

---

### `FA-SEC01-18` · `S3` · `SummaryMoments::scaled_css` is `#[serde(skip)]`, dropping finite dispersion on roundtrip when `css` overflows

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/sum.rs:152-153,207`
- **Category:** `contract-drift`
- **Defect:** `SummaryMoments` derives `serde::Serialize, serde::Deserialize`, yet skips `scaled_css: Option<(f64, f64)>`, the only field preserving finite variance/SD when `|x_i - mean| >= 1e155` causes `css` (`(css_s * scale) * scale` at line `207`) to overflow to `+Inf`.
- **Failure scenario:** Computing `SummaryMoments::from_slice(&[1e160, 2e160])` yields finite `sample_sd() = 7.071e159` via `scaled_css`, while `css = +Inf`. Serializing and deserializing `SummaryMoments` resets `scaled_css = None`, turning `sample_sd()`, `sem()`, `sample_variance()`, and `centered_sum_of_squares()` into `+Inf` silently. (Currently `SummaryMoments` is used only in-process by downstream crates, so severity is `S3`.)
- **Evidence:**
  ```rust
  // crates/math/src/sum.rs:139, 152-153, 207, 212-213
  #[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct SummaryMoments {
      ...
      #[serde(skip)]
      scaled_css: Option<(f64, f64)>,
  }
  ...
  let (css, scaled_css) = if scale == 1.0 { (css_s, None) } else { (((css_s * scale) * scale).max(0.0), Some((scale, css_s))) };
  ```
- **Suggested fix:** Remove `#[serde(skip)]` (using `#[serde(default)]` for backward compatibility if JSON payloads already exist without the field).
- **Related:** `none`.

---

### `FA-SEC01-19` · `S3` · `sync` wait loop leaves `tid` registered in `g.waiting_on` if user `on_contention` callback panics

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/platform/src/sync.rs:1194,1250`
- **Category:** `robustness`
- **Defect:** `prepare_wait_iteration` inserts `tid -> WaitTarget` into the global lock graph `g.waiting_on` (`1250`), after which `acquire` invokes user-supplied `on_contention` (`1194` -> `285-286`) with no RAII `Drop` cleanup guard removing `tid` on panic unwind.
- **Failure scenario:** If a user installs a panicking `on_contention` hook and catches the unwind (`std::panic::catch_unwind`) on a worker thread, that `ThreadId` remains recorded in `g.waiting_on` waiting on the lock, causing subsequent lock waits by or on that thread to falsely trigger `LockError::WouldDeadlock`.
- **Evidence:**
  ```rust
  // crates/platform/src/sync.rs:1192-1195, 1250-1253
  if !warned && elapsed >= core.limits.warn_limit {
      warned = true;
      core.emit_contention_report(elapsed, &thread_label, snapshot.holder_labels);
  }
  ...
  g.waiting_on.insert(
      tid,
      WaitTarget { lock_id: core.slot.id, lock_name: core.name.clone(), mode: opts.mode, thread_label: thread_label.to_owned() },
  );
  ```
- **Suggested fix:** Scope `tid`'s `g.waiting_on` entry inside a small `Drop` guard around the wait loop (calling `cancel_wait` + `bump_seq_and_notify_all` if dropped before acquisition completes).
- **Related:** `crates/platform/src/sync.rs:285-286`.

---

### `FA-SEC01-20` · `S4` · `PValueFormat::Apa` omits `p < 1.0` upper guard, formatting exact `p = 1.0` as `"> .999"`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/pvalue.rs:195`
- **Category:** `contract-drift`
- **Defect:** Unlike `PValueFormat::FourDecimal` (`186`: `if p > 0.9999 && p < 1.0`) and `PValueFormat::Nejm` (`205`: `if p > 0.999 && p < 1.0`), `PValueFormat::Apa` checks `if p > 0.999` without `&& p < 1.0`.
- **Failure scenario:** Formatting an exact `p = 1.0` under `PValueFormat::Apa` yields `"> .999"` (a strict inequality asserting `p` is strictly above `.999`, or inconsistent with `FourDecimal` -> `"1.0000"` and `Nejm` -> `"1.000"` which reserve `"> ..."` strictly for `(threshold, 1.0)`).
- **Evidence:**
  ```rust
  // crates/math/src/pvalue.rs:186, 195, 205
  } else if p > 0.9999 && p < 1.0 {
  ...
  } else if p > 0.999 {
      String::from("> .999")
  ...
  } else if p > 0.999 && p < 1.0 {
  ```
- **Suggested fix:** Align `Self::Apa` with `Self::FourDecimal` and `Self::Nejm` using `else if p > 0.999 && p < 1.0` (yielding `"1.000"` at `p == 1.0`), or adjust unit test expectations explicitly if `> .999` for `1.0` was desired across all three presets.
- **Related:** `crates/stats/src/common.rs:181`.

---

### `FA-SEC01-21` · `S4` · Open-interval boundary rounding in `Oklab::to_oklch` (`h_deg == 360.0`) and `AxisScale::Logit`/`Probit`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/palette/src/space.rs:220`, `crates/math/src/lib.rs:139,176,184,190`
- **Category:** `contract-drift`
- **Defect:**
  1. `Oklab::to_oklch` documents `h_deg` in `[0, 360)` (`space.rs:214`), yet `rem_euclid(360.0)` returns exact `360.0` for tiny negative `b` (e.g. `a = 0.1, b = -1e-20`), violating `< 360.0`.
  2. `AxisScale::Logit(Percent)` computes `(u / (100.0 - u)).ln()`, collapsing subnormal `u < 5e-322` (`5e-323 / 100.0 == 0.0`) to `None`, while `inverse(t)` saturates at `40.0`+ to closed endpoints `Some(100.0)` / `Some(1.0)` (outside the open transform domain `(0, 100)` / `(0, 1)`).
- **Failure scenario:** Converting `Oklab { l: 0.5, a: 0.1, b: -1e-20, alpha: 1.0 }.to_oklch()` returns `h_deg = 360.0` (breaking `< 360.0` assertions); `AxisScale::Logit(Percent).transform(5e-323)` returns `None` even though `ln(u) - ln(100 - u) = -746.74` is finite, and `transform(inverse(40.0).unwrap())` returns `None`.
- **Evidence:**
  ```rust
  // crates/palette/src/space.rs:220
  let h_deg = if c <= 1e-12 { 0.0 } else { b.atan2(a).to_degrees().rem_euclid(360.0) };

  // crates/math/src/lib.rs:139, 176, 184, 190
  (u / (100.0 - u)).ln()
  ...
  100.0 / (1.0 + (-t).exp())
  ...
  1.0 / (1.0 + (-t).exp())
  ...
  Self::Probit => 100.0 * dist::norm_cdf(t),
  ```
- **Suggested fix:** Normalize `if h_deg >= 360.0 { 0.0 } else { h_deg }` in `Oklab::to_oklch`; evaluate `Logit(Percent)` as `u.ln() - (100.0 - u).ln()`.
- **Related:** `none`.

---

### `FA-SEC01-22` · `S4` · Unused `[dependencies]` in `palette/Cargo.toml` and `platform/Cargo.toml` emit `cargo::unused_dependencies` warnings

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/palette/Cargo.toml:14`, `crates/platform/Cargo.toml:14`
- **Category:** `cleanup`
- **Defect:** `crates/palette/Cargo.toml` declares `curvebook-math = { workspace = true }` without referencing `curvebook_math` anywhere in `crates/palette`, and `crates/platform/Cargo.toml` declares `serde = { workspace = true }` without referencing `serde` directly (only `serde_json`).
- **Failure scenario:** Both `cargo test` (`curvebook-palette`, `curvebook-platform`) and `cargo clippy --all-targets` (`curvebook-platform`) emit `warning: unused dependency ...` on every build, adding compile graph edges and log noise.
- **Evidence:**
  ```toml
  # crates/palette/Cargo.toml:13-15
  [dependencies]
  curvebook-math = { workspace = true }
  serde = { workspace = true }

  # crates/platform/Cargo.toml:13-15
  [dependencies]
  serde = { workspace = true }
  serde_json = { workspace = true }
  ```
  Observed compiler output during `cargo test`: `warning: unused dependency curvebook-math --> crates/palette/Cargo.toml:14:1` and `warning: unused dependency serde --> crates/platform/Cargo.toml:14:1`.
- **Suggested fix:** Remove `curvebook-math` from `crates/palette/Cargo.toml` and `serde` from `crates/platform/Cargo.toml`.
- **Related:** `none`.

---

### `FA-SEC01-23` · `S4` · `resolve_atomic_target` only rejects directories, copying mode bits from and renaming over FIFOs/sockets/devices

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/platform/src/fs.rs:157-162`
- **Category:** `robustness`
- **Defect:** Unlike `read_regular_file_inner` / `ensure_regular_file`, `resolve_atomic_target` (`fs.rs:157-162`, and dangling/live target checks at `170, 185`) checks `is_dir()` rather than `!file_type().is_file()`.
- **Failure scenario:** Passing a path pointing to an existing FIFO, Unix socket, or device node to `write_atomic` copies that special file's permission bits onto the staged regular temp file and atomically unlinks/replaces the special node with `fs::rename` instead of returning `PlatformError::InvalidPath`. (Note: because `fs::rename` replaces the directory entry without opening the FIFO/device, it does not block or corrupt device I/O.)
- **Evidence:**
  ```rust
  // crates/platform/src/fs.rs:157-162
  if !lmeta.file_type().is_symlink() {
      if lmeta.is_dir() {
          return Err(PlatformError::InvalidPath { reason: format!("target path `{}` is a directory", path.display()) });
      }
      return Ok((path.to_path_buf(), Some(lmeta.permissions())));
  }
  ```
- **Suggested fix:** Reject `!meta.file_type().is_file()` in `resolve_atomic_target` / `resolve_symlink_target` whenever an existing non-symlink target (or resolved live target) is not a regular file.
- **Related:** `crates/platform/src/path_guard.rs` (`ensure_regular_file`).

---

### `FA-SEC01-24` · `S4` · `gamma(x)` reflection collapses representable negative subnormal values (`x in -175.5..-171.5`) to `±0.0`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/math/src/special/gamma.rs:227-228`
- **Category:** `wrong-result`
- **Defect:** `gamma(x)` for negative non-integer `x` evaluates `lg.exp()` before dividing `PI / (sin_pix * lg.exp())`.
- **Failure scenario:** For `x = -173.5`, `ln_gamma(174.5) = 724.5943 > ln(f64::MAX)`, so `lg.exp()` overflows to `+Inf` and `gamma(-173.5)` returns `0.0` instead of the exact representable subnormal `6.45408359e-315` (similarly `-0.0` at `-172.5` instead of `-1.113e-312`).
- **Evidence:**
  ```rust
  // crates/math/src/special/gamma.rs:227-228
  let lg = ln_gamma(1.0 - x)?;
  return Ok(std::f64::consts::PI / (sin_pix * lg.exp()));
  ```
  Python check: `math.lgamma(174.5) = 724.5943211333924`, `exp(lg) = inf`, `math.gamma(-173.5) = 6.45408359e-315`.
- **Suggested fix:** When `lg > 709.0`, compute `(PI.ln() - lg).exp() / sin_pix`.
- **Related:** `none`.
