# Randomization and simulation oracle scope

Add independent checks of weighted treatment allocation, stratified row sampling, and simulated count distributions. Existing integration tests check seed replay, row eligibility, and binomial support and moments. Tail and cumulative probability checks can detect errors those tests miss. [Randomization implementation](../../crates/transform/src/randomize.rs) · [Existing audit tests](../../crates/transform/tests/audit_r2_s04.rs)

## Proposed scope

Exercise the public transform functions for treatment schedules, row sampling, and simulated columns. Use exact integer quotas and small hand-enumerated allocations for schedule and stratum counts. For simulation, use independently evaluated binomial and Poisson CDFs at preselected integer cutoffs. [Durstenfeld's random permutation algorithm](https://doi.org/10.1145/364520.364540) · [SciPy binomial reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.binom.html) · [SciPy Poisson reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.poisson.html)

## Acceptance criteria

- [ ] Verify exact complete-balanced arm counts for unequal integer weights and tied remainders. For permuted blocks, check the exact arm ratio in each full block, valid counts in a truncated final block, one assignment per subject, and the recorded seed and allocation settings.
- [ ] Verify exact stratum counts for proportional allocation and for equal allocation when a small stratum reaches capacity. Include an excluded or missing stratum key. Check distinct selected rows in source order and the remapped exclusion notes.
- [ ] Check binomial cumulative probabilities on both sides of the small-trial and rare-event switches and include a case with success probability above one half. Check Poisson cutoffs on both sides of the switch at a rate of 30. Check support and exact zero-rate or probability endpoint cases.
- [ ] Record reference inputs, CDF cutoffs, seeds, draw counts, and statistical bounds before checking CurveBook output. State the false-positive budget across the planned comparisons and retain the existing moment checks and tolerances. Run the new fixtures in the ordinary offline Rust suite.

Related work includes distribution tail oracles in [#29](https://github.com/HauserGroup/curvebook/pull/29).
