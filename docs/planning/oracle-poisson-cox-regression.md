# Poisson and Cox regression oracle scope

## Proposed scope

Add two deterministic synthetic fixtures that compare the results from `curvebook_stats::run_analysis` against independently generated R values. The existing D1 suites cover core Poisson estimates and Cox tie methods. This follow-up checks how the public analysis path encodes factors, filters rows, and reports fitted values and diagnostics.

## Acceptance criteria

- [ ] **Poisson.** Use synthetic counts with zeros, a three-level factor with an explicit reference level, and a continuous covariate. Compare the reported coefficients, model-based standard errors, rate ratios and 95% Wald intervals, fitted counts, Pearson and deviance residuals, and residual deviance against R 4.5.2 `stats::glm` with a Poisson log link and dispersion fixed at one for inference. Assert the contrast labels and fitted-row order. [UCLA Poisson example](https://stats.oarc.ucla.edu/r/dae/poisson-regression/) · [R `summary.glm` reference](https://search.r-project.org/R/refmans/stats/help/summary.glm.html)
- [ ] **Cox.** Use synthetic follow-up times with tied events, right censoring, a categorical predictor, a continuous covariate, and a row with a missing predictor. Verify the reported listwise exclusion. Compare coefficients, standard errors, hazard ratios and 95% Wald intervals, and the likelihood-ratio statistic against R 4.5.2 with `survival` 3.8-3. Run `coxph` with Efron, Breslow, and exact ties explicitly. For Efron, also compare baseline cumulative hazard and survival at event times, evaluated at the same retained design-column means. [UCLA survival walkthrough](https://stats.oarc.ucla.edu/wp-content/uploads/2025/02/survival_r_full.html) · [R `survival` reference](https://cran.r-project.org/web/packages/survival/refman/survival.html)
- [ ] **Reproducibility.** Check in original synthetic inputs and a regeneration script that checks the R and package versions. Record the model settings, full-precision expected values, and justified tolerances alongside the Rust assertions. Cite the UCLA examples for the case design. Run the tests in the existing offline `curvebook-stats` suite.

The planned work covers tests, reference generation, and provenance. This draft records the proposal for review.

Related oracle work includes [#29](https://github.com/HauserGroup/curvebook/pull/29), [#30](https://github.com/HauserGroup/curvebook/pull/30), and [#31](https://github.com/HauserGroup/curvebook/pull/31).
