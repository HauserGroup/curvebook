# Randomization and simulation oracle scope

Add independent checks of weighted treatment allocation, stratified row sampling, and simulated count distributions. Existing integration tests check seed replay, row eligibility, and binomial support and moments. Tail and cumulative probability checks can detect errors those tests miss. [Randomization implementation](../../crates/transform/src/randomize.rs) · [Existing audit tests](../../crates/transform/tests/audit_r2_s04.rs) · [Independent D1 oracles](../../crates/transform/tests/d1_randomization.rs)

## Proposed scope

Exercise the public transform functions for treatment schedules, row sampling, and simulated columns. Use exact integer quotas and small hand-enumerated allocations for schedule and stratum counts. For simulation, use independently evaluated binomial and Poisson CDFs at preselected integer cutoffs. [Durstenfeld's random permutation algorithm](https://doi.org/10.1145/364520.364540) · [SciPy binomial reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.binom.html) · [SciPy Poisson reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.poisson.html)

## Acceptance criteria

- [x] Verify exact complete-balanced arm counts for unequal integer weights and tied remainders. For permuted blocks, check the exact arm ratio in each full block, valid counts in a truncated final block, one assignment per subject, and the recorded seed and allocation settings.
- [x] Verify exact stratum counts for proportional allocation and for equal allocation when a small stratum reaches capacity. Include an excluded or missing stratum key. Check distinct selected rows in source order and the remapped exclusion notes.
- [x] Check binomial cumulative probabilities on both sides of the small-trial and rare-event switches and include a case with success probability above one half. Check Poisson cutoffs on both sides of the switch at a rate of 30. Check support and exact zero-rate or probability endpoint cases.
- [x] Record reference inputs, CDF cutoffs, seeds, draw counts, and statistical bounds before checking CurveBook output. State the false-positive budget across the planned comparisons and retain the existing moment checks and tolerances. Run the new fixtures in the ordinary offline Rust suite.

## Oracle coverage

| Area | Fixture / Case | Pre-specified Parameters & Seed | Verification / Reference Bound |
|---|---|---|---|
| Complete-balanced allocation | `cb_tied_4arm` | $N = 15$, weights `4:3:2:1`, seed `104729` | Exact Hamilton quotas `[6, 5, 3, 1]` (tied remainder `5` resolved by config order), 1:1 subject bijection, ARRIVE constants |
| Complete-balanced allocation | `cb_tied_3arm` | $N = 9$, weights `3:2:2`, seed `224737` | Exact Hamilton quotas `[4, 3, 2]` (tied remainder `4` resolved by config order), 1:1 subject bijection |
| Permuted blocks (fixed) | `pb_fixed` | $N = 29$, weights `1:2:3`, block size `12`, seed `350377` | Exact `[2, 4, 6]` per full block, truncated tail block of `5` bounded by `[2, 4, 6]`, ARRIVE metadata |
| Permuted blocks (variable) | `pb_variable` | $N = 41$, weights `1:2:3`, block sizes `[6, 12]`, seed `479001` | Exact `[m, 2m, 3m]` per full block ($m \in \{1, 2\}$), valid truncated tail, both block sizes exercised, ARRIVE metadata |
| Stratified sampling (proportional) | `strat_prop_15`, `strat_prop_13` | 43-row fixture (`Alpha`: 20, `Beta`: 12, `Gamma`: 5, `Tiny`: 2 + 4 ineligible rows), seeds `611953` ($n=15$), `746773` ($n=13$) | Exact quotas `[8, 4, 2, 1]` (unclamped) and `[6, 4, 2, 1]` (3-way remainder tie + min-1 clamp), distinct source-ordered rows, ineligible rows excluded, remapped `CellExclusionNote`s |
| Stratified sampling (equal, saturated) | `strat_equal_19` | Same 43-row fixture, $n = 19$, seed `882377` | Exact quotas `[6, 6, 5, 2]` (`Tiny` saturated at 2, `Gamma` saturated at 5), guaranteed remapping of active-row notes `(17, 1, 0)` and `(18, 1, 0)` |
| Binomial regime switches & flip | `Binom32_Bernoulli`, `Binom33_Btrs`, `Binom39_Geo`, `Binom40_Btrs`, `Binom2000_Geo`, `Binom60_FlipBtrs`, `Binom64_FlipGeo` | $N = 25{,}000$ draws/col, seed `20261118`, 4 preselected integer cutoffs each | Exact Binomial CDFs at 4 cutoffs (`z = 5.6` SE), sample mean & variance (`z = 5.6` SE), integer support bounds; brackets `trials <= 32` (`32` vs `33`), `trials * q < 10` (`9.75` vs `10.0`), rare event (`2000, 0.004`), and `p > 0.5` flip on BTRS (`0.75`) and Geo (`0.875`) |
| Poisson rate-30 switch & range | `Pois4p5_Small`, `Pois29p5_Below30`, `Pois30_At30`, `Pois120_Large` | $N = 25{,}000$ draws/col, seed `20261118`, 4 preselected integer cutoffs each | Exact Poisson CDFs at 4 cutoffs (`z = 5.6` SE), sample mean & variance (`z = 5.6` SE), non-negative integer support; brackets `lambda < 30.0` (`29.5`) vs `lambda >= 30.0` (`30.0`) |
| Deterministic endpoints | `BinomP0`, `BinomP1`, `BinomTrials0`, `PoisLambda0` | `p = 0`, `p = 1`, `trials = 0`, `lambda = 0`, $N = 512$, seed `20260001` | Exact constant outputs (`0` or `trials`) across both `Column` and `MultipleVariables` archetypes |

## Generator provenance, corroboration, and false-positive budget

- **Pre-specification & provenance:** The independent Python reference generator ([randomization.py](../../crates/transform/tests/reference/randomization.py)) uses `mpmath` 1.3.0 at 120 decimal digits to compute exact integer quotas, exact PMF-summed Binomial and Poisson CDFs (cross-checked internally against `betainc`/`gammainc`), theoretical moments, and exact discrete tail probabilities prior to evaluating CurveBook outputs.
- **Precision & independent tool corroboration:** Running `randomization.py` at 80 and 120 decimal digits produced bit-for-bit identical binary64 constants. All 44 Binomial and Poisson CDF cutoffs were independently corroborated with R 4.5.2 (`stats::pbinom` and `stats::ppois` in [randomization.R](../../crates/transform/tests/reference/randomization.R)), agreeing within `9.99e-16` absolute and `4.40e-15` relative difference.
- **Family-wise false-positive budget:** Across the `66` planned stochastic comparisons (`44` CDF cutoffs + `11` sample means + `11` sample variances at `z = 5.6` standard errors over `N = 25,000` draws per column), the exact discrete two-sided `Binomial(25000, F(k))` tail probabilities sum to `9.9925e-7`, the exact discrete two-sided `Binomial(25000 * n, p)` / `Poisson(25000 * lambda)` mean tail probabilities sum to `2.3612e-7`, and the `11` variance checks (asymptotic two-sided normal tail `2.144e-8`, conservatively budgeted at `1.0e-7` each for finite-sample skewness margin) sum to `1.1000e-6`. By Bonferroni union bound, the family-wise false-positive rate is at most `2.3354e-6 <= 1.0e-5`. All existing moment checks and tolerances in `randomize.rs` and `audit_r2_s04.rs` remain unchanged.

Related work includes distribution tail oracles in [#29](https://github.com/HauserGroup/curvebook/pull/29).
