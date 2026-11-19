# Local CurveBook code audit

Baseline `3c78fdb416902349672e3f0eb7dfc424ac3491d6`. Started 2026-11-18. The checkout contained unrelated documentation edits when this review started.

## Review order and coverage

| Order | Risk and review focus | Status |
|---|---|---|
| 1 | Data loss and trust boundaries. Native archive save/open, recovery checkpoints and replay, control dispatch, CSV parsing and import placement entry points. | Targeted pass complete |
| 2 | Scientific correctness. Dataset selection, paired input extraction, analysis dispatch, and failed-result updates. | Targeted pass complete |
| 3 | Publication output and release. PDF page budget, PNG/TIFF sizing, browser audit carry-over, and release draft assembly. | Targeted pass complete |
| 4 | Audit records. Index reconciliation and prior-finding deduplication. | Complete |

Files and tests examined will be listed under each pass. Coverage is risk-based across subsystem seams.

## Prior evidence and audit documentation

- `docs/audit/slices.md` maps the codebase and governing documents. The X1 and X2 slices identify cross-crate input and data-correctness paths worth following.
- `docs/audit/review-2026-11-16-ledger.md` lists remediation and review gaps by area. `docs/gaps.md` and the Round 1 and Round 2 reports are deduplication references.
- The index in `docs/audit/README.md:15–20` mixes historical and current status. Round 2 had 149 open survivors at its recheck, and the third-pass `RV-75` through `RV-99` were once open. The archived `R0.19` and `R0.25` roadmap entries record later remediation at `docs/planning/roadmap-history/r0.md:107–114,153–161`. Merge `77dacc4` is in the audited baseline. Refresh the index's current status and link to the closure records. Preserve dated counts and the underlying reports, and link these follow-on findings when they are ready to be tracked.
- The seven top-level audit documents had no missing relative link destinations in the local path check. Their dated survivor counts are useful historical evidence. The chronological sections in the review ledger intentionally retain what was known at each pass.
- The previous web host adapter report, issue #21, is existing evidence. Its six findings came from native bridge probes with browser-side code tracing. This local review does not duplicate them.

## Findings

### 1. High · Desktop saves can overwrite archives created by a newer version

**Reproduced.** `project.save` and `project.saveTemplate` serialize the current document and write the bytes through the engine path policy. This bypasses the existing-target schema check used by the guarded format writers. Desktop Save and Save Template call these engine commands. A save can succeed and replace a newer file's bytes, including metadata this version of CurveBook cannot interpret.

- Evidence from `crates/engine/src/cmd/project.rs:198–229,264–273`, `crates/format/src/archive.rs:184–241,255–259`, and `apps/curvebook/src/gui_window.rs:599–620`.
- With synthetic project and template archives marked `CURRENT_SCHEMA_VERSION + 1`, the guarded format APIs rejected both writes and preserved the files byte-for-byte. The ordinary engine commands then succeeded and replaced both files.
- Regression target is byte-for-byte preservation and a typed version error from every save entry point. This is a surviving engine path for the class reported as `R2-S06-03` and `RV-07`.

### 2. High · A desktop control batch can replay a successful edit twice after a crash

**Reproduced.** The post-request hook iterates successful batch steps while holding the final session state. A step that requires a checkpoint, such as `edit.undo`, snapshots the effects of later steps too. The hook then appends those later steps to the journal and recovery applies them again.

- Evidence from `crates/ui-egui/src/control_io.rs:1318–1344,1673–1701`, `apps/curvebook/src/desktop_host.rs:698–725`, and `crates/engine/src/lib.rs:900–929,1000–1025`.
- Started with one table, modified a cell, and issued one successful batch containing undo followed by a table creation with an automatic ID. The live document had two tables. After dropping the runtime without a clean shutdown, recovery produced three tables.
- Regression target is recovering the exact live document for mixed batches. A checkpoint of the final state must cover the whole batch or each step must be persisted against its own state. Related to `R1-S07-06`, which covered individual undo and filesystem-dependent commands.

### 3. High · An interrupted unpacked save can leave the project unreadable

**Code trace and intermediate state reproduced.** Native unpacked save stages and syncs files, then publishes them through individual renames. `document.json` is published before `manifest.json`. An abrupt termination between renames bypasses the in-process rollback and can leave a manifest pointing to the prior document CRC.

- Evidence from `crates/format/src/archive.rs:284–342,607–647,975–999`.
- Two independently saved, valid directories both opened. Publishing only the new `document.json` into the old directory, matching the first rename in a save whose document changed, caused `CorruptCrc` on reopen. The probe simulated the intermediate disk state without killing a process or simulating lost power.
- Regression target is reopening a complete prior or new generation after an interrupted commit. `R2-S06-04` addressed errors returned during save. This is the remaining process-interruption case.

### 4. High · Bland–Altman can silently analyze the wrong columns

**Reproduced.** On an XY table with one Y data set, the fallback interprets the request as X versus Y, silently replaces an out-of-range `column_a` with Y data set zero, and ignores `column_b`.

- Evidence from `crates/doc/src/analysis.rs:263–277`, `crates/engine/src/cmd/analysis.rs:1553–1557`, and `crates/stats/src/two_group.rs:2155–2209`.
- With X values `[11, 13, 15]` and Y values `[10, 10, 10]`, `analysis.run` with explicit column indices 99 and 98 returned an up-to-date result with bias 3. That bias is the arithmetic mean of the three X-minus-Y differences and verifies the substitution.
- Regression target is retaining valid default X-versus-Y behavior while rejecting invalid explicit columns or representing the X-versus-Y mode explicitly.

### 5. High · Ordinary desktop cell edits can be lost after the undo buffer fills

**Reproduced.** The desktop input path checkpoints only when an undo or redo depth changes or the session first becomes dirty. When a new edit evicts the oldest undo entry, the depth stays at 64. On an already-dirty session with empty redo, the edit therefore produces no checkpoint.

- Evidence from `apps/curvebook/src/desktop_host.rs:1133–1158,1781–1790`, `crates/ui-egui/src/interaction.rs:3129–3144`, and `crates/engine/src/lib.rs:699,1161–1172,1374–1382`.
- Filled the undo stack with successful writes, then double-clicked a blank cell, entered 8, and committed through the native desktop host event dispatcher. The live cell contained 8 and the undo depth was still 64. Recovery after the runtime was dropped left the cell blank.
- Regression target is persisting a committed GUI edit at both the undo entry and memory limits. Use a mutation signal independent of undo stack length. The prior desktop recovery findings covered missing routes and individual commands.

### 6. High · A temporary failed analysis deletes persisted result-sheet notes

**Reproduced.** Failed recomputation clears derived outputs and the result sheet's floating notes. The success branch explicitly preserves those notes, so temporarily invalid data can delete a user annotation even after the analysis recovers.

- Evidence from `crates/engine/src/dag.rs:853–861,1011–1045` and `crates/doc/src/ids.rs:130–149`.
- Round-tripped a successful t-test result with a synthetic floating note through native archive save/open. Excluding one of two observations made the dependent result fail and removed the note. Removing the exclusion restored the result to up-to-date while the note stayed absent.
- Regression target is retaining notes across a failed and then successful recomputation while clearing invalid analytical output.

### 7. High · Paired row-wise tests silently drop an unmatched selected column

**Reproduced for Column tables and traced for Multiple Variables tables.** The input adapter splits a selection at its midpoint. Paired extraction zips the halves, dropping the final selected column when the count is odd. The result reports a successful test and gives no warning about the discarded data.

- Evidence from `crates/stats/src/two_group.rs:1793–1826,1928–1938,2268–2305,2313–2357`.
- Ran `multiple_row_t_tests` with `paired: true` and explicit selection `[0,1,2,3,4,5,6]` on the row `[10,20,30,21,23,28,1000]`. The reported Group B mean was 24 with 2 degrees of freedom. Those values account for the three pairs formed from the first six columns. The populated seventh column had no effect and did not count as a skipped row.
- Regression target is rejecting an ambiguous odd-width paired design or requiring explicit one-to-one replicate assignments. Missing values within valid pairs remain a separate input condition.

### 8. Medium · A valid two-pair t-test fails during optional effect-size calculation

**Reproduced.** The paired t-test accepts two complete pairs and computes its primary statistics with one degree of freedom. It then requires Hedges' correction, whose gamma denominator is out of domain at that degree of freedom. The error propagates even if effect sizes are disabled at the command level, since that option is applied only to formatting.

- Evidence from `crates/stats/src/two_group.rs:684–729,1643–1655`, `crates/stats/src/column.rs:1329–1336`, and `crates/math/src/special/gamma.rs:214–220`.
- `analysis.run` with `kind: "paired_t_test"`, `compute_effect_sizes: false`, and paired columns `[10,20]` and `[21,23]` returned `solver_failed` with `argument x = 0 is outside domain`. The exact primary statistic for the differences `[-11,-3]` is `t = -1.75` on one degree of freedom. The paired row-test path calls the same routine and can omit an otherwise usable row.
- Regression target is returning the primary test and representing an unavailable effect size separately. Verify the single comparison and row-wise callers.

### 9. Medium · DPI-based raster export can distort the figure and misstate its physical size

**Reproduced for PNG and traced through the shared TIFF sizing path.** The engine independently clamps the two derived dimensions and keeps the original density. The rasterizer uses the two separate scale factors, and the encoder writes the requested density into the output even when an axis was clamped.

- Evidence from `crates/engine/src/cmd/export.rs:610–648,659–685,700–729`, `crates/render/src/rasterizer.rs:196–227`, and `crates/render/src/png.rs:53–70`.
- Exported a 2,000 × 60 point layout at 360 DPI. The engine returned 8,192 × 300 pixels labeled 360 DPI. A uniform scale at 360 DPI requires 10,000 × 300, so the returned output is horizontally compressed by about 18%. The output fit the total pixel budget.
- Regression target is preserving scene proportions and matching pixel dimensions to the reported density for DPI-only export. Return a typed limit error if the requested output cannot be produced.

## Verification and boundaries

- Native macOS arm64 at the opening baseline, with Rust `1.99.1`, the locked dependencies, and offline Cargo resolution. `cargo test --locked --offline -p curvebook --test local_audit_probe` passed all nine temporary checks by asserting the observed defects. The unpacked save probe reproduced the interrupted file state with two synthetic saves. The desktop recovery probes dropped the host runtime without clean shutdown. The probe source was removed after verification.
- `cargo test --locked --offline -p curvebook --test recovery_wiring r1_s11_01_default_single_instance_control_stream_journals_and_recovers_after_crash` passed the selected test. It covers individual requests, including undo, and a failed atomic batch. The new successful mixed batch probe covers a separate path.
- `cargo test --locked --offline -p curvebook-format --test unpacked_and_pzfx_budget` passed all seven tests, including native archive round-trips and import budget checks.
- `cargo test --locked --offline -p curvebook-stats --test rv_ledger_stats_core` passed all eleven tests.
- The source review covered the high-risk paths listed above. The full workspace suite, every statistical routine, live browser and OS UI behavior, and the hosted release workflow were outside the verification performed in this pass.

## Wave 2 coverage log

Baseline `bb0e36c4c44f1d0a134317b84fe1c618d401229d`. This wave extends the first pass. Each row records the actual code reviewed and its verification.

| Priority | Area | Coverage and status |
|---|---|---|
| 1 | Regression and nonlinear fitting | Reviewed Grouped, XY, and Multiple Variables observation extraction in `crates/fit/src/types.rs`, including summarized data and unknowns. Traced parameter bindings, weight and observation preparation, the LM trust-region and active-bound steps, Poisson IRLS, covariance finalization, model comparison guards, and nonlinear result assembly in `crates/fit/src/{solver,result}.rs`. Findings 10–12 reproduced. |
| 2 | GLM and Cox regression | Reviewed complete-case design and categorical coding, XY GLM selection, residual and prediction tables, and the Cox likelihood loop, baseline/residual assembly, and concordance in `crates/stats/src/{regression,survival}.rs`. Findings 13, 15, and 16 reproduced. |
| 3 | Calculated variables and windowed aggregates | Reviewed cell coercion, conditional argument resolution, partition and frame handling, and calculated-column dependency traversal and exclusion preservation in `crates/transform/src/formula/{eval,dag}.rs`. Finding 14 reproduced. |

### Wave 2 findings

#### 10. High · Grouped regression treats error-bar endpoints as observations

**Reproduced.** Grouped extraction handles summary modes that provide moments, then treats all cells of other modes as raw replicates. In `UpperLowerLimits`, this includes both interval endpoints. The same branch also treats asymmetric errors or quartiles as measurements in their respective summary modes. XY extraction instead uses only the central value for these modes.

- Evidence from `crates/doc/src/cell.rs:153–171,195–207`, `crates/fit/src/types.rs:345–378,391–437`, and `crates/fit/src/solver.rs:828–845,2568–2576`.
- Fit a Grouped straight line to four means `[10,12,14,16]` at implicit X values 0 through 3, with intervals `[mean - 1, mean + 4]`. The result used 12 observations and fitted an intercept of 11. These are the results from including both limits. The four central values alone give intercept 10 exactly.
- Regression target is accepting only the central observations when the fit explicitly supports that input, or rejecting summary modes that cannot support an inferential fit.

#### 11. High · Nonlinear Poisson regression accepts impossible negative fitted means

**Reproduced.** The solver checks observed counts, floors the means when calculating weights and deviance, and keeps the unfloored predictions in the returned fit. It can therefore report convergence with an invalid Poisson model. Its separate log-likelihood calculation rejects the same means, leaving the information criteria absent.

- Evidence from `crates/doc/src/analysis.rs:978–983`, `crates/fit/src/solver.rs:1113–1129,1873–1888,3465–3525`, and `crates/fit/src/result.rs:371–395`.
- A Poisson fit to `[1,3,2,4]` with `Slope = 0` and `YIntercept = -1` returned `Converged` and finite deviance. Every fitted mean was −1 by construction, so the requested model had no admissible solution.
- Regression target is rejecting infeasible fitted means and keeping optimization, likelihood, predictions, and convergence status consistent about the model's domain.

#### 12. High · Equal coefficient bounds incorrectly consume a degree of freedom

**Reproduced.** Identical inclusive lower and upper bounds leave a coefficient with exactly one possible value. The layout still allocates a free slot, and the solver subtracts it when calculating residual degrees of freedom and uses its Jacobian column for uncertainty.

- Evidence from `crates/fit/src/solver.rs:535–547,627–668,2470–2490,2563–2576`.
- For a five-point straight-line fit with slope bounded to `[2,2]`, the result reported two free parameters and 3 residual degrees of freedom. Fixing the slope explicitly to 2 gave one free parameter and 4 degrees of freedom for the same model and data.
- Regression target is treating equal bounds as a fixed parameter or returning a typed validation error that directs callers to a fixed parameter rule.

#### 13. High · XY GLMs silently use the first response despite explicit column selection

**Reproduced.** The XY design path always builds a matrix from the first Y dataset's first subcolumn and the first stored X subcolumn. It ignores the requested response, predictor columns, and dataset selection.

- Evidence from `crates/doc/src/analysis.rs:838–849,1920–1923` and `crates/stats/src/regression.rs:1095–1115,1620–1632`.
- A linear GLM on an XY sheet with distinctly valued Y datasets, dataset selection `[1]`, `response_column: 1`, and invalid `predictor_columns: [99]` succeeded. Its residual table listed the observed values of dataset zero. A request that names an impossible predictor should fail even if XY allows fewer choices than Multiple Variables.
- Regression target is honoring the supported XY mapping and returning a typed error on unsupported or invalid explicit requests.

#### 14. High · Conditional aggregates can reinterpret their arguments when the data change

**Reproduced.** The documented `sumif(value, condition)` order is overridden by a heuristic that decides whether each evaluated column looks Boolean. Conditions can contain any nonzero finite number, but the heuristic recognizes only 0 and 1 as Boolean. When the value column contains only 0 and 1, a valid numeric mask can be mistaken for the values to sum.

- Evidence from `docs/implementation/data-tables-and-transforms.md:93–97` and `crates/transform/src/formula/eval.rs:66–81,1123–1160,1252–1267,1412–1421`.
- With `Reading = [1,1,0]` and `Flag = [2,0,2]`, `sumif(Reading, Flag)` returned 2. The equivalent explicit condition `sumif(Reading, Flag != 0)` returned the correct sum of 1.
- Regression target is stable documented argument order and a compatibility rule based on explicit condition syntax, independent of the evaluated cell contents.

#### 15. Medium · GLM diagnostics shift source row labels after exclusions

**Reproduced for linear GLM and traced for logistic and Poisson result tables.** The design builder compacts complete rows without retaining source indices. The result builder then labels the compacted data from Row 1, so every residual or prediction after a gap can point to the wrong source observation.

- Evidence from `crates/stats/src/regression.rs:966–1005,1071–1093,1707–1731,1825–1837,1903–1926`.
- Excluded source row 2 from a five-row table. The output correctly kept the observations from rows 1, 3, 4, and 5, but labeled them Row 1 through Row 4. This can direct an outlier investigation to the wrong row.
- Regression target is retaining source indices through complete-case filtering and showing them alongside the fitted values, residuals, and influence statistics.

#### 16. Medium · Cox regression reports chance discrimination without any comparable pairs

**Reproduced.** The concordance code correctly excludes event–event time ties from its denominator. When no pairs remain it substitutes 0.5, which the result sheet publishes as a numerical C-index. This resembles an estimated chance-level result even though concordance is undefined.

- Evidence from `crates/stats/src/survival.rs:846–859,1632–1658,1867–1876`.
- With five distinct covariate values and all five events at exactly the same time, the Breslow fit converged, and the reported C-index was 0.5. The comparable-pair count is zero under the documented pair rule.
- Regression target is displaying concordance as unavailable when its denominator is zero and retaining the comparable-pair count in the diagnostic.

### Wave 2 verification and remaining coverage

- Native macOS arm64, Rust `1.99.1`, lockfile unchanged. `cargo test --locked --offline -p curvebook-engine --test local_audit_wave2_probe` passed seven temporary synthetic checks that assert the observed failure behavior. The tests used the public fit, statistics, and formula APIs. The temporary test source was removed after verification.
- `cargo test --locked --offline -p curvebook-fit -p curvebook-stats -p curvebook-transform --test r46_solver_oracles --test d1_edge_matrix --test r63_window_and_conditional` passed 32 existing tests. Coverage includes shared and box-constrained fits, Poisson likelihood and AICc, weighted and summarized-moment fits, tied Cox regressions, logistic and Poisson GLMs, and conditional and windowed formulas.
- The findings are scoped to the reviewed functions and cases. The full equation catalog, custom DSL and ODE solver, mixed-effects covariance estimation, plotting of derived diagnostics, and live GUI integration are still outside these waves' verification.

## Wave 3 coverage log

Baseline `a5fdb7f39d1bf87b2404fbdccbf850867e80b0c8`. Started 2026-11-18. This wave covers high-risk areas that Waves 1 and 2 did not review. Each area was audited in its own isolated worktree and build directory.

| Priority | Area | Coverage and status |
|---|---|---|
| 1 | ANOVA family and multiple comparisons | Reviewed ordinary one-way ANOVA from raw and summary data, variance tests, Welch and Brown–Forsythe, repeated-measures one-way, Kruskal–Wallis, Friedman, the Type I and III factorial engine, repeated-measures two-way, nested one-way, the one-way, factorial, Grouped, and Multiple Variables runners, and factorial multiplicity in `crates/stats/src/anova.rs`. Reviewed `crates/stats/src/{multiplicity,cld}.rs` in full and the engine selection and override parsing in `crates/engine/src/cmd/analysis.rs`. Variance-correction formulas, sphericity corrections, step-down procedures, and compact letter display raised no issues. Findings 30–36 reproduced. Complete. |
| 2 | Third-party and versioned file import | Reviewed `crates/format/src/{pzfx,prism,xml,zip,versioned,registry}.rs` in full. Also reviewed the archive migration framework and the v0→v3 steps in `crates/format/src/archive.rs:1140–1310` and the engine import and merge entry points at `crates/engine/src/cmd/table.rs:3180–3380`. `crates/format/src/schemas.rs` and `crates/doc/src/schema.rs` generate JSON Schema only and were skimmed. The ZIP reader, XML parser, migration steps, and `.prism` newer-version refusal raised no issues. Findings 26–29 reproduced, and one subcolumn-format question is open. Complete. |
| 3 | Contingency, survival estimation, and nonparametric tests | Reviewed margins, chi-square and G², Fisher 2×2 and R×C, Cochran–Armitage, McNemar, `analyze_contingency`, `analyze_2x2_ratios`, and the contingency sheet runner in `crates/stats/src/categorical.rs`. Reviewed Kaplan–Meier estimation, CIs and median, weighted log-rank and Gehan, the trend test, and hazard-ratio estimators in `survival.rs`. Reviewed Mann–Whitney, Wilcoxon one-sample and matched pairs, and Hodges–Lehmann intervals in `{two_group,column}.rs`, and the contingency and survival input adapters in `crates/doc/src/archetypes.rs` and `crates/engine/src/cmd/analysis.rs`. Findings 17–22 reproduced. Complete. |
| 4 | MCP and control trust boundary | Reviewed JSON-RPC framing, size caps and batch limits, the `--gui-only` resource and prompt lock, tool gating, `command_run` and `command_batch` rollback, remote forwarding, and tool annotations in `crates/mcp/src/{protocol,tools,gui_tools,toolsets,formats}.rs`. Also reviewed the CLI MCP entry at `apps/curvebook-cli/src/main.rs:95–260`, the engine path policy and strict gate, `WorkdirGuard`, the engine file writers, the GUI-only menu, print, and screenshot enforcement, and loopback binding. Control batch journaling was excluded (finding 2). Findings 23–25 reproduced. Complete. |

### Wave 3 findings

#### 17. High · Empty rows or columns inflate contingency degrees of freedom

**Reproduced.** Cells with zero expected count are skipped in the statistic, but degrees of freedom are always `(R−1)(C−1)` over the full table. Cramér's V and its interval use the same full dimensions. The sheet path fills unselected or empty categories with zero, so selecting a subset of data sets can introduce an all-zero row.

- Evidence from `crates/stats/src/categorical.rs:950–962,1116–1124,1169–1175,2300–2309`.
- `[[10,2],[3,9],[0,0]]` returned χ² 8.224 on 2 df with P 0.0164. The informative 2×2 table has 1 df and P 0.00413. A 3×3 sheet with only data sets 0 and 1 selected showed the same df and P, with one fewer significance star. With an empty column, `[[10,2,0],[3,9,0],[4,4,0]]` reported 4 df, P 0.083, and V 0.359, against 2 df, P 0.016, and V 0.507 for the reduced table. R `chisq.test` was the external reference: NaN on the padded table and the stated values on the reduced table.
- Regression target is removing zero-margin rows and columns before computing df, effect size, and intervals, with a visible note, and rejecting tables that fall below 2×2.

#### 18. Medium · Undefined 2×2 quantities are reported as numbers

**Reproduced.** With an empty margin, the code substitutes fixed values for quantities that are 0/0. Odds ratio is infinite whenever `b` or `c` is zero. Relative risk is 1 when both risks are zero. Sensitivity and specificity are 0 when their column total is zero. The chi-square path then reports χ² 0 and P 1.

- Evidence from `crates/stats/src/categorical.rs:957,1121,1965–1985`.
- `analyze_contingency([[5,0],[3,0]])` reported χ² 0, P 1, Fisher P 1, and phi 0. `analyze_2x2_ratios(5,0,3,0)` reported an infinite odds ratio and specificity 0 with CI (0, 1). `analyze_2x2_ratios(0,5,0,3)` reported an infinite odds ratio, relative risk 1, and sensitivity 0. Each of these is 0/0.
- Regression target is reporting each undefined quantity as not estimable. This is the same pattern as the Round 1 kappa item for `p_e = 1`.

#### 19. Medium · Log-rank and Gehan count a cohort with nobody at risk as a degree of freedom

**Reproduced.** When one cohort contributes no risk set at any event time, the reduced covariance matrix is singular. The least-squares fallback produces the correct statistic, but df stays `K − 1`.

- Evidence from `crates/stats/src/survival.rs:412–433`.
- Cohort A had events at 2, 4, 6, 8, and 10. Cohort B had events at 3, 5, 7, and 9 and was censored at 11. Cohort C had three subjects censored at 0.5. Log-rank returned χ² 0.3623 on 2 df with P 0.834. R `survdiff` gives the same χ² on 1 df with P 0.547. Gehan also reported 2 df. Cohort order did not change the result.
- Regression target is basing df on the rank of the covariance matrix or the number of cohorts with nonzero expected events. An alternative is rejecting cohorts with no exposure.

#### 20. Medium · Survival comparisons report P = 1 when no events occurred

**Reproduced.** All-censored data produce a zero covariance matrix or a zero trend denominator. Each is converted to χ² 0, so the result looks like a test that found no difference.

- Evidence from `crates/stats/src/survival.rs:418–433,457`.
- Two cohorts censored at {1, 2} and {3, 4}, plus a third at {5, 6} for the trend test, returned χ² 0 and P 1 from log-rank, Gehan, and the trend test. The test is undefined without events, and R `survdiff` returns P NA.
- Regression target is reporting the comparison as not estimable when there are no events.

#### 21. Medium · Hodges–Lehmann intervals claim confidence levels they cannot reach

**Reproduced.** The order-statistic index comes from the normal approximation, `k = max(1, floor(μ − zσ))`, without checking the coverage actually achieved. The output still uses the requested level in its label.

- Evidence from `crates/stats/src/column.rs:1494–1505`, `crates/stats/src/two_group.rs:996–1006,2754,3001`.
- At a requested 95%, the one-sample interval for {1,2,3,4} was (1, 4) with exact coverage 1 − 2/2⁴ = 0.875. For n = 5 the exact coverage is 0.9375. The paired n = 4 case gave 0.875. Mann–Whitney for {1,2,3} versus {10,20,30} gave (−29, −7) with coverage 0.90. R `wilcox.test` returns the same bounds with the lower achieved level and a warning.
- The interval is documented as distribution-free, but for many samples with n ≤ 30 the approximate index gives an interval one order statistic wider than the exact one. At m = n = 8 the coverage is 0.962 against 0.950 for the exact critical value. For a one-sample n = 10 it is 0.963 against 0.951.
- Regression target is using exact critical values for small samples without ties and reporting the achieved level or a warning when the nominal level is not achievable.

#### 22. Medium · Contingency results display the chi-square df beside tests with different df

**Reproduced for the trend test. The Yates label was traced from the code.** The main result row always writes the full-table chi-square df, whichever method was selected.

- Evidence from `crates/stats/src/categorical.rs:2226–2229,2360–2363,942`.
- A 4×2 sheet `[[15,5],[10,10],[8,12],[3,17]]` with the Cochran–Armitage trend test returned z −3.819 and P 0.000134, which match R `prop.trend.test`. The row showed df 3, but the trend test has 1 df. Fisher rows also receive a df. The label "Chi-square (Yates)" is used for any table, but the correction is applied only to 2×2 tables.
- Regression target is method-specific df, empty for exact tests, and the Yates label only when the correction was applied.
- Related documentation drift, traced only: `crates/doc/src/analysis.rs:741–743` describes Woolf odds-ratio intervals and likelihood ratios that the code does not compute.

#### 23. High · `--workdir` does not confine an MCP server attached to the desktop

**Reproduced.** `set_workdir` installs the guard only on the local and in-process hosted sessions. In live mode with `--control-port`, MCP rewrites only the path keys in the request and forwards the engine command to the desktop. The desktop session never receives a path policy and stays `Unconstrained`. Any stored external links in a project opened through MCP then resolve without the workdir boundary, including projects opened from `bytes_base64`.

- Evidence from `apps/curvebook-cli/src/main.rs:170–192`, `crates/mcp/src/tools.rs:331–350,1673–1687,1725–1737`, `crates/mcp/src/formats.rs:17–22,103–112,326–335`, and `crates/engine/src/cmd/project.rs:486–490`. No `set_path_policy` or workdir handling exists in `apps/curvebook/src`.
- Set the workdir and attached to a desktop control handler over `UiApp`. Called `open_file` on an archive inside the workdir with a stored delimited-file link to a file outside it, then `project.recalculate` with `include_external_links: true`, then `doc_inspect`. The refresh reported one link and returned the outside file's contents. The same sequence with a local MCP session, or a direct `open_file` of the outside path, returned `path_escape`.
- `--workdir` is documented as the hard boundary for untrusted MCP clients, and this route lets a client read any file the desktop user can read.
- Regression target is rejecting `--control-port` with `--workdir`, or enforcing the policy on the desktop host for stored links and byte-based opens. This is the live-attach variant of the stored-link escape class in `R1-S12-01`, `R1-S07-02`, and `R1-X1-04`, which covered the local session only.

#### 24. Medium · GUI tools that change state or write files are annotated read-only

**Reproduced.** `Effects::read_only_hint` ignores `CHANGES_UI_STATE`, and the tool annotations pass it through as `readOnlyHint`. `ui_invoke_menu`, `ui_pointer_gesture`, and `ui_press_key` are annotated with UI-state changes only, so they advertise `readOnlyHint: true` and `destructiveHint: false`. `ui_invoke_menu` runs any enabled menu command. `ui_screenshot` is annotated read-only and closed-world although it accepts an output path.

- Evidence from `crates/engine/src/cmd/effects.rs:104–122`, `crates/mcp/src/gui_tools.rs:122,177,215,238`, `crates/mcp/src/toolsets.rs:28–35`, and `crates/ui-egui/src/gui_actions.rs:1719–1771`.
- On a `--gui-only` in-process host, `ui_invoke_menu` with `sheet.createData` added a sheet, and the `File/New Project` path ran `file.new` (`MAY_DISCARD_DATA`) and removed it. On a host without `--gui-only`, `ui_screenshot` with a path to an existing file replaced it with PNG bytes.
- The server instructions tell clients to check annotations before running mutating tools. A client that approves read-only tools automatically would discard a document without asking.
- Regression target is annotation tests for all eight GUI tools against their worst-case effects. `RV-13` fixed the same gap for the strict gate but not for annotations, and `annotations_match_engine_effects_flags` covers only engine command specs.

#### 25. Medium · Tools that overwrite files are annotated non-destructive

**Reproduced.** `destructive_hint` is true only for `MAY_DISCARD_DATA`. Tools whose effects are `WRITES_FILESYSTEM` report `destructiveHint: false`, even though the export path replaces any existing target without checking its type or contents.

- Evidence from `crates/engine/src/cmd/effects.rs:118–122`, `crates/mcp/src/tools.rs:864,969`, `crates/mcp/src/formats.rs:850–866`, `crates/engine/src/cmd/project.rs:871,884`, `crates/engine/src/cmd/export.rs:24`, `crates/engine/src/cmd/analysis.rs:41`, and `crates/engine/src/cmd/table.rs:237`.
- With the default unconstrained headless server, `export` to CSV with the path of an existing valid `.curvebook` archive replaced it with `A\n\n`. After enabling the `table` toolset, `table_export_csv` replaced an existing text file the same way. `tools/list` shows `destructiveHint: false` for `export`, `save_file`, `project_save`, `table_export_csv`, and `export_svg`.
- Under the MCP specification, `destructiveHint: false` means the tool only adds. In the documented unconstrained default these tools can overwrite any file the user can write, including newer-version archives through `project_save` (finding 1).
- Regression target is a destructive annotation for tools that can replace existing files, or an explicit `overwrite` parameter that refuses by default. `RV-17` documented the unconstrained trust boundary but did not change the annotations.

Related note, not numbered: under `--gui-only --workdir`, `ui_invoke_menu` with `command_id: "project.save"` and a `path` parameter wrote the file without using UI widgets. `enforce_gui_only_menu_command_policy` at `crates/ui-egui/src/gui_actions.rs:1649–1671` allows path parameters once a workdir is set. This is the surviving `R2-S10-01` class, and the owners should decide whether the residual is accepted.

#### 26. Medium · `.pzfx` titles, labels, and text cells are rewritten by the CSV token parser

**Reproduced.** Each `<d>` cell, row title, and parts-of-whole slice label goes through the CSV token parser with CSV defaults. These defaults treat `NA`, `NaN`, `NULL`, and `.` as missing in any case and turn a trailing `*` into an exclusion. Numeric-looking titles are also reformatted. CSV import keeps row titles verbatim.

- Evidence from `crates/format/src/pzfx.rs:195,261,492,598–610,613,644,760–766` and `crates/format/src/csv.rs:43–54,106–107,301–357,947–956`.
- Row titles `Na, 007, Control*, 1.50, ., null` imported as `"", "7", "Control", "1.5", "", ""` without warnings. A Multiple Variables text column `Na, K, Ca*` imported as empty, `K`, and `Ca` marked excluded. The format already carries exclusion in its own `Excluded` attribute.
- Regression target is importing titles and labels byte for byte and parsing `.pzfx` cells without CSV sentinels or asterisk exclusions.

#### 27. Medium · Embedded `.pzfx` tables in a `.prism` archive link to another file's Info sheet

**Reproduced.** `merge_embedded_pzfx` renames clashing info and data sheet IDs but does not remap each data sheet's `info_sheet_id`. A table from the second embedded file keeps pointing at the old ID, which now belongs to the first file's Info sheet.

- Evidence from `crates/format/src/prism.rs:249–269` and `crates/format/src/pzfx.rs:139,158–160`.
- A `.prism` archive held `a.pzfx` with an Info constant `Dose = 1` and `b.pzfx` with `Dose = 2`. Info B was renamed `info-1-1`, but Table B kept `info_sheet_id = "info-1"`, which is Info A. The import succeeded without a warning, so info hooks and paired-constant lookups in Table B would use 1 instead of 2.
- Regression target is a per-document old-to-new ID map applied to `info_sheet_id`, asserting that Table B resolves `Dose = 2`. `R1-S07-14` fixed the equivalent remap in the engine merge path only.

#### 28. Medium · `.prism` JSON entries bypass the bounded JSON parser

**Reproduced.** The `.prism` importer parses `document.json` and `manifest.json` with an unbounded `serde_json::Value` parse. Native open uses `parse_bounded_json_value` with node and byte limits.

- Evidence from `crates/format/src/prism.rs:58–62,195–199` and `crates/format/src/archive.rs:423–425,540–545,557–569`.
- A 58 KB `.prism` archive with a 60 MB `document.json` of 20 million empty arrays, within the 64 MiB entry cap, raised peak resident memory to 812 MB before failing with a type error. A control that only built and decompressed the same archive peaked at 192 MB.
- Regression target is the same bounded parse as native open, with a typed budget error. This path survived the `R1-S06-05` fix for native open.

#### 29. Medium · Imported sheet IDs skip identifier validation

**Reproduced end to end.** The `.pzfx` and `.prism` importers build sheet IDs from the file's `ID` attribute or metadata through `SheetId::new` and not through `SheetId::try_new`. IDs containing whitespace, `:`, `{{`, or `}}` are kept, persist through save and reopen, and cannot be addressed by commands that validate sheet IDs.

- Evidence from `crates/format/src/pzfx.rs:80,173,274,749–758`, `crates/format/src/prism.rs:260,265,343,362,443–452`, `crates/doc/src/ids.rs:17–39`, and `crates/engine/src/cmd/helpers.rs:21–22`.
- `table.importPzfx` with tables `ID="Dose Response"` and `ID="a:b"` succeeded. `sheet.rename` and `sheet.delete` on both IDs returned `BadInput`. Saving and reopening kept the invalid IDs.
- Regression target is validating imported IDs or replacing them with generated IDs and a warning. This is an import-path bypass of the `RV-89` fix.

Open question, not numbered: `crates/format/src/pzfx.rs:677–684` maps the `YFormat` token `SDN` to `MeanLowHigh`, while `SD` and `MeanSDN` map to mean, SD, and N. The mapping was reproduced, and an `SDN` table with subcolumns `10 / 2 / 5` was imported as mean, plus error, and minus error without a warning. The token's intended meaning was not confirmed from a public format description, so this remains unverified until it can be. The unrecognized tokens `SEN`, `CVN`, and `upper-lower-limits` do raise a coercion warning.

#### 30. High · A blank or excluded row or column in a Grouped table can erase a main effect or abort two-way ANOVA

**Reproduced through the engine.** The factor level count comes from allocated rows and data sets, not observed cells. Effect coding uses the last level as the reference. When that level has no data, the main-effect column equals the sum of the interaction columns. Type III comparison then reports zero sum of squares on zero df, and with more empty levels the least-squares solve fails. New Grouped tables have 12 rows by default.

- Evidence from `crates/stats/src/anova.rs:1433–1441,1550–1561,6772–6774,6885–6889,6905,6978–6994` and `crates/engine/src/cmd/sheet.rs:320–324`.
- A balanced 2×2 design with three replicates per cell was used. Row A0 had B0 `[10,11,12]` and B1 `[14,15.5,16]`, and row A1 had B0 `[20,21.5,22]` and B1 `[21,23,24.5]`. The trimmed table gives Column SS 25.52 on 1 df, F 16.33, and P 0.00373, which the hand computation confirms. In the default 12-row sheet, `analysis.run` failed with an SVD non-convergence error. With one trailing blank row, or with every cell of a third row excluded, the result was up to date but the column factor showed SS 0 and df 0. An empty third data set erased the row factor instead. With a matched column factor, blank rows were treated as missing and forced REML, which failed. Type I was correct in every case.
- The Multiple Variables path keeps levels from excluded or missing-response rows, so the same mechanism is expected there. That path was traced and not run.
- Regression target is dropping unobserved levels before coding, so that results with blank or excluded rows and columns match the trimmed table for Type III and repeated measures. This is the ANOVA counterpart of finding 17.

#### 31. Medium · Raw-data one-way ANOVA shows a 95% effect-size interval under the requested level

**Reproduced.** Input extraction always runs the ANOVA at 95%, and that result is reused for the effect-size output. The heading uses the requested level. The summary-data path passes the requested level correctly.

- Evidence from `crates/stats/src/anova.rs:4025–4031,4902–4906`.
- With three groups and a requested 99% level, the heading read "99% CI (partial eta2)" with interval (0, 0.6245), identical to the direct 95% computation. The 99% interval is (0, 0.6867).
- Regression target is matching the direct calculation at the requested level. `RV-65` fixed the same issue for post-hoc intervals.

#### 32. Medium · Duplicate and out-of-range data set selections are silently accepted

**Reproduced.** The engine stores selected indices without deduplication or bounds checks. The resolver drops out-of-range indices and keeps duplicates.

- Evidence from `crates/engine/src/cmd/analysis.rs:1159–1165` and `crates/stats/src/common.rs:228–233`.
- With groups A `[1,2,3]`, B `[4,5,6]`, and C `[7,8,9]`, the selection `[0,1]` gave F 13.5 on 1 and 4 df, P 0.0213. `[0,1,1]` entered B twice and gave F 9 on 2 and 6 df. `[0,0]` compared A with itself and gave F 0, P 1. `[0,1,99]` succeeded as `[0,1]`, while the sheet kept the invalid selection.
- Regression target is a typed error for invalid indices and either rejection or deduplication with a note for duplicates. Related to `RV-59` and finding 4.

#### 33. Medium · One single-observation group aborts ordinary ANOVA and Kruskal–Wallis

**Reproduced.** Ordinary one-way ANOVA always runs the variance-homogeneity tests, which require at least two observations per group. Kruskal–Wallis uses the same extraction, so both analyses fail.

- Evidence from `crates/stats/src/anova.rs:536–541,602–606,4902–4906`.
- For groups `[5]`, `[1,2,3]`, and `[7,8,9]`, the hand-computed ANOVA gives F 27 on 2 and 4 df, P 0.00476. Kruskal–Wallis gives H 5.143, P 0.0764, which the direct `kruskal_wallis_test` reproduces. `run_one_way_anova` returned `InsufficientSampleSize` for both methods.
- Regression target is returning the primary test and marking only the variance tests as unavailable.

#### 34. Medium · Welch and Brown–Forsythe ANOVA replace the requested post-test with Games–Howell

**Reproduced.** Every multiple-comparison request other than Games–Howell, Tamhane T2, or Dunnett T3 is mapped to all-pairs Games–Howell without a note.

- Evidence from `crates/stats/src/anova.rs:4085–4100`.
- Requests for none, Dunnett against group 0, Bonferroni, and Benjamini–Hochberg FDR all produced the same three all-pairs rows, with adjusted P `[0.0477, 0.810, 0.870]`. Dunnett should give two comparisons against the control, none should give no post-test, and FDR should give q-values.
- Regression target is honoring the request, mapping it explicitly with a footnote, or rejecting the combination. This is the same class as `RV-67` for survival.

#### 35. Medium · Repeated-measures post-tests can ignore pairing, and Friedman drops its post-test

**Reproduced.** Repeated-measures one-way ANOVA passes column summaries to the one-way multiplicity routine. Games–Howell, Tamhane T2, and Dunnett T3 then use per-column SDs and Welch df as if the groups were independent. A Friedman request with Dunn's rank-sum post-test produces no comparisons.

- Evidence from `crates/stats/src/anova.rs:4794` and `crates/stats/src/multiplicity.rs:819–1000`.
- Six subjects had baselines spread over 0–50, condition offsets 0, 2, and 4, and noise up to 0.3. Repeated-measures ANOVA gave F 491.7 with P about 1e-10, and Tukey on the residual mean square gave adjusted P near 1e-8. Games–Howell gave 0.98, 0.93, and 0.98, and Tamhane T2 gave about 0.98. The paired differences have SD about 0.2 with |t| above 20, so every pair differs.
- Regression target is rejecting independent-group unequal-variance procedures for repeated-measures designs or using paired contrasts, and emitting or rejecting Friedman post-tests explicitly.

#### 36. Medium · Summary-data ANOVA accepts a fractional N

**Reproduced.** Only `n < 1` is rejected, and degrees of freedom use the raw value. The displayed N is rounded.

- Evidence from `crates/stats/src/anova.rs:711–714,732`.
- Mean, SD, and N inputs `(1, 1, 2.5)`, `(2, 1, 3)`, and `(3, 1, 3)` returned df within 5.5, df total 7.5, F 2.735, and P 0.150.
- Regression target is a typed error for fractional or non-finite N.

### Wave 3 verification and remaining coverage

- Native macOS arm64, Rust `1.99.1`, lockfile unchanged, offline resolution. Each area ran in its own git worktree at the baseline with an isolated build directory. Each finding was reproduced by a temporary probe that asserted the observed behavior, and every probe was removed afterwards with the worktree left clean. Reference values came from hand derivation, closed forms, and R 4.x base and `survival` as an external numerical oracle only. Tests ran against compiled binaries.
- Probes: `curvebook-stats` and `curvebook-engine` for areas 1 and 3, `curvebook-format` and `curvebook-engine` for area 2, and `curvebook-cli` for area 4. The MCP probe drove the desktop control dispatcher over a real `UiApp` and kept temporary files under its build directory.
- Existing tests passed:
  - Areas 1 and 3: the `curvebook-stats` library tests, `rv_ledger_stats_core`, `d1_reference_oracles`, `d1_edge_matrix`, `nist_strd`, `audit_r2_s02`, `audit_r1_s4a_stats`, `audit_r1_s4b_stats`, `r35_anova_reml_oracles`, `r36_rm_reml_oracles`, `r34_unstructured_reml_oracles`, `r1011_cld_piepho_oracles`, `r35_power_multiplicity_oracles`, `r35_survival_twogroup_oracles`, `parity_g8_barnard_pb_mi`, and `wave21_r97_r912_clustering`.
  - Area 2: all 105 `curvebook-format` tests.
  - Area 4: `gui_only_surface` in `curvebook-mcp`, 7 of 7.
- Not covered in this wave:
  - REML and Kenward–Roger internals, LS-means internals, and three-way repeated-measures REML.
  - Baptista–Pike, Koopman, Barnard, kappa, exact binomial, and parts-of-whole goodness of fit.
  - MCP `open_file` and CLI `convert` beyond their registry routing, real third-party-written files, and whether the XML node cap rejects large legitimate `.pzfx` files.
  - Dialog-field file paths in `ui_answer_dialog`, symlink races, Windows path handling, and the web host adapters (issue #21).
- Still outside all waves: the full equation catalog, the custom DSL and ODE solver, mixed-effects covariance estimation, power analysis, multivariate methods, ROC, chart and graph rendering of derived results, and live GUI behavior.

## Wave 4 coverage log

Baseline `a5fdb7f39d1bf87b2404fbdccbf850867e80b0c8`. Started 2026-11-18. During the wave `main` advanced to `033d4be`, which changes only `AGENTS.md`, so all line references hold for both commits. GUI and figure code (`crates/{ui-egui,chart,render,svg,pdf,tools}`) are out of scope for this wave. The areas are ranked by how widely a defect would propagate. Each area was audited in its own isolated worktree and build directory.

| Priority | Area | Coverage and status |
|---|---|---|
| 1 | Numerical foundations | About 9,000 values were probed against external oracles. They covered the normal, t, F, and chi-square CDF, survival, log-survival, density, and quantile functions for df 0.05 to 1e13 and tails down to 1e-300. They also covered the regularized incomplete beta and gamma functions, their inverses, gamma, digamma, and the error-function family. Noncentral t, F, and chi-square were checked with their intervals, along with the studentized range and Dunnett for equal and unequal sizes. Compensated summation, Pearson r, P-value formatting, the Brent root finder, QR rank and least squares, SVD, and `sym_eigen` were reviewed. The distributions matched to working precision in the ranges analyses use. Findings 64–65 reproduced, and lower-severity tail-accuracy notes follow them. Complete. |
| 2 | Linear fitting, fit diagnostics, and equation catalog | Reviewed simple linear regression with and without an intercept, intervals, inverse and calibration intervals, lack of fit, Deming regression, and line comparison in `crates/fit/src/linear.rs`. Reviewed normality tests, the runs test, AICc, BIC, likelihood-ratio and extra-sum-of-squares comparison, covariance dependency, RSDR, ROUT stepping, and Hougaard skewness in `crates/fit/src/diagnostics.rs`. Reviewed residual-diagnostic wiring in `crates/fit/src/solver.rs` and summary tables in `crates/fit/src/result.rs`. A finite-difference gradient check covered 70 catalog models in `crates/fit/src/library/*`, and the published forms of the main pharmacology, binding, kinetics, and growth families were read. Every statistical routine matched R or exact enumeration, and the catalog had no gradient or formula mismatch. Findings 51–53 reproduced. Complete. |
| 3 | Custom equation language and ODE models | Reviewed the compiler, statement parsing, dataset prefixes, equation branch resolution, free-identifier collection, the tokenizer, parser, VM, and built-in functions, ODE evaluation with its trajectory cache, and `integrate_rk45` in `crates/fit/src/dsl.rs`. Also reviewed the finite-difference Jacobian at `crates/fit/src/solver.rs:961`, custom-model resolution at `crates/fit/src/result.rs:314–320`, and the engine custom-equation entry at `crates/engine/src/cmd/analysis.rs:1453–1478,2063–2068`. Operator precedence, log bases, division by zero, parse limits, and RK45 handling of blow-up, stiffness, reverse direction, and unsorted X were correct. Findings 42–50 reproduced. Complete. |
| 4 | Data transforms and table conversion | Reviewed in full `crates/transform/src/ops/{mod,normalize_baseline,math,reshape_select}.rs`, `curve.rs`, every conversion path in `archetype_convert.rs`, the plate-mapping and tidy pivots in `plate_and_tidy.rs`, the `import.rs` pipeline, row sampling and stratification in `randomize.rs`, and `simulate.rs`. Also reviewed the engine adapters for table conversion, plate import, simulation, random sampling, baseline resolution, the transform shorthand parsers, and DAG transform execution. Formula evaluation was excluded (finding 14). `RV-59`, `RV-60`, `RV-62`, `RV-63`, `R2-S04-02`, `R2-S04-03`, and `R2-S04-06` appear fixed. Findings 54–63 reproduced. Complete. |
| 5 | Power, multivariate, ROC, and mixed-effects REML | Reviewed all of `roc.rs`. Reviewed the `power.rs` evaluators, effect resolvers, N and effect solvers, and rounding. Reviewed Pearson, Spearman, Kendall, the correlation matrix, and PCA in `multivariate.rs`. Reviewed the random-intercept REML, Fisher information, Kenward–Roger and Satterthwaite, multi-component AI-REML, unstructured REML, and nested paths in `anova.rs:2088–3990`, plus `pvalue_stack.rs:88–240` and the two-stage BKY adjustment. PCA, Kendall τ-b, the power noncentralities and rounding, the Kenward–Roger formulas, and ROC tie handling matched the references. Findings 37–41 reproduced. Complete. |

### Wave 4 findings

#### 37. High · A hidden variance-ratio cap gives wrong REML F and P without any convergence warning

**Reproduced.** The random-intercept REML fit searches only over `u = ln(1 + τ)` in `[0, 12]`, so the ratio of the random and residual variances can never exceed about 1.63e5. `MixedRemlResult` has no converged or boundary field, so a fit stopped at the search limit looks like a normal result.

- Evidence from `crates/stats/src/anova.rs:310–322,2526–2561`.
- A balanced nested design had 2 groups, 3 subclusters per group, and 3 replicates per subcluster, at `m − ε`, `m`, and `m + ε`. Subcluster means were 10, 20, 30 and 15, 25, 35. The closed form is F = 112.5/300 = 0.375 on 1 and 4 df, with P = 0.5734. `nested_one_way_anova` was correct at ε = 0.1. At ε = 0.01 it gave σ²_u 37.2, F 1.008, and P 0.372. At ε = 0.001 it gave σ²_u 25.1, F 1.493, and P 0.289. CurveBook's `fit_multi_vc_reml` and R `nlme::lme` both give F 0.375 on the same data.
- The same routine supplies the repeated-measures one-way and two-way REML paths through `emit_random_intercept_reml` (`anova.rs:4536,7184–7206`). That route was traced and not run.
- Regression target is F 0.375 ± 1e-6 and P 0.5734 for the ε = 0.01 table, with any fit stopped at the search limit flagged in the result.

#### 38. Medium · DeLong ROC with one case or one control treats an undefined variance as zero

**Reproduced.** When there is only one case or one control, the DeLong component variance divides by `n − 1 = 0`. The code substitutes 0, so the standard error is understated and the P value is far too small.

- Evidence from `crates/stats/src/roc.rs:154–158`.
- Controls 1 through 10 and a single case at 9.5 gave AUC 0.9, SE 0.1, CI (0.704, 1.0), and P 6.3e-5 against 0.5. The exact Mann–Whitney two-sided P is 4/11 = 0.364, and Hanley–McNeil on the same data gives SE 0.218 and P 0.066. A single control with ten cases gives the same 6.3e-5.
- Regression target is an error, an unavailable value, or a documented fallback when either group has one subject. A single subject must never produce P < 0.05. This is distinct from the fixed `R1-S02-03` perfect-separation case.

#### 39. Medium · Undefined correlations are reported as r = 0, P = 1 and added to the FDR family

**Reproduced.** Any pairwise correlation error, including a constant column or fewer than three complete pairs, becomes r 0 and P 1. That P value enters the Benjamini–Hochberg family and inflates the q-values of the defined pairs.

- Evidence from `crates/stats/src/multivariate.rs:348–367,1326–1352`.
- With columns A = 1–5, B = `[2,1,4,3,6]`, and a constant K = 7, Pearson reported r(A,K) 0 with P 1. The q-value for A–B was 0.263 with three tests, and adding a fourth column with two complete pairs raised it to 0.526. R `cor.test` gives NA for the constant column, and the only defined test gives q(A,B) = P = 0.0877.
- Regression target is showing undefined pairs as unavailable and excluding them from the FDR family.

#### 40. Medium · Two-stage BKY adjusted values drop discoveries the published procedure makes

**Reproduced and recomputed by hand.** The adjusted value is clamped to at least the raw P. The adaptive step can legitimately reject a P above q once `m̂0` is small, and the clamp then overrides that rejection.

- Evidence from `crates/stats/src/multiplicity.rs:400–411`, reached from `crates/stats/src/pvalue_stack.rs:131` and the post-hoc `TwoStageBkyFdr` option.
- For P values 0.001 through 0.009 and 0.2 at q 0.05, the code reported 9 discoveries and gave 0.2 an adjusted value of 0.2. Benjamini, Krieger, and Yekutieli (2006), Definition 6, sets q′ = 0.05/1.05. Stage 1 rejects 9, so m̂0 = 1. Stage 2 then rejects all 10, because 10q′/m̂0 = 0.476 ≥ 0.2. The candidate adjusted value 0.021 is replaced by the clamp. This happens whenever `i > m̂0(1 + q)` and `q < p(i) ≤ i·q′/m̂0`.
- Regression target is 10 discoveries for this stack. This contradicts the clean BKY verdict in `docs/audit/findings/r1/S02.md:198`.

#### 41. Medium · Power analysis accepts and returns impossible Cohen's h values

**Reproduced.** Cohen's h is bounded by π, but the standardized input checks only that h is finite. The effect-size solver searches far beyond π.

- Evidence from `crates/stats/src/power.rs:910–917,1806–1838`.
- A detectable-effect request with n = 2 per group, α 0.05, and power 0.9 returned h = 3.2415, with no implied proportion. A sample-size request with h = 5 returned n = 2 per group at power 0.9988. By definition |h| = |2 asin√p1 − 2 asin√p2| ≤ π (Cohen 1988, chapter 6), so both requests are unreachable.
- Regression target is rejecting |h| > π on input and returning an error when the detectable-effect root lies beyond π. Separate from `R2-S02-03` through `R2-S02-11`, whose Welch df and allocation-ratio issues appear fixed.

#### 42. High · ODE initial conditions ignore dataset prefixes

**Reproduced.** The ODE initial-condition branch drops each statement's dataset filter and overwrites a single global `t0`. Only the last initial condition is compiled and applied to every dataset, although dataset prefixes are documented for global models.

- Evidence from `crates/fit/src/dsl.rs:1706–1716,806–812`.
- With `<A> Y0 = A0`, `<B> Y0 = B0`, `dY/dX = -k*Y`, A0 = 10, B0 = 3, and k = 0.5, dataset A gave 3 at X = 0 instead of 10, and 1.8196 at X = 1 instead of 6.0653. With `<A> Y(0) = A0` and `<B> Y(5) = B0`, dataset A gave 36.55 at X = 0, because t0 = 5 applied everywhere. A global fit with shared k reported A0 as 1 with SE 0, labelled dataset A's initial value as `B0`, and returned an ambiguous status.
- Regression target is each dataset matching its own initial condition at its own t0, or a compile error for dataset-scoped initial conditions.

#### 43. Medium · An ODE initial condition that names a local creates a new fitted parameter

**Reproduced.** Free identifiers in initial conditions are collected and compiled with an empty local set, so a reference to a body local becomes an extra parameter.

- Evidence from `crates/fit/src/dsl.rs:768–770,810`.
- `Ytop = Bottom + Span`, `Y0 = Ytop`, `dY/dX = -k*(Y - Bottom)` produced the parameters Bottom, Span, k, and Ytop. With Bottom 1, Span 4, and the spurious Ytop at 99, X = 0 gave 99 instead of 5, and X = 1 gave 37.05 instead of 2.4715.
- Regression target is making locals visible to initial conditions or rejecting the reference at compile time. This is a different path from `R1-S03-08`.

#### 44. Medium · `Y` referenced through a local is silently evaluated as 0

**Reproduced.** Whether an equation is implicit is decided only from the target expression. A local that uses `Y` leaves the equation explicit, and explicit evaluation sets `Y` to 0.

- Evidence from `crates/fit/src/dsl.rs:1223,1729,1746`.
- `t = c*Y` followed by `Y = A*X + t`, with A = 1, c = 0.5, and X = 2, compiled as explicit and gave 2. The equivalent `Y = A*X + c*Y` compiled as implicit and gave the correct 4.
- Regression target is the same result for both forms, or a compile error.

#### 45. Medium · A local assigned after `Y` is dropped and becomes a fitted parameter

**Reproduced.** Branch locals stop at the target line, and free-identifier collection then treats the later name as unbound.

- Evidence from `crates/fit/src/dsl.rs:748–756,2012,2018`.
- `Y = A*X*k` followed by `k = 2` produced the parameters A and k. With A = 1 and the fitted k at 7, X = 1 gave 7 instead of 2.
- Regression target is rejecting a local defined after use, or applying it.

#### 46. Medium · NaN and infinite conditions select definite branches inconsistently

**Reproduced.** Conditional jumps treat any non-finite condition as false, `NOT` treats NaN and infinity as true, powers outside their domain return NaN without a domain failure, and equality is computed as `|a − b| == 0`. Only the final result is checked for finiteness.

- Evidence from `crates/fit/src/dsl.rs:3020–3023,3050–3051,3071–3081`.
- At X = 1, `IF((X-5)^0.5 > 1, 10, 20)` returned 20, while the `sqrt(X-5)` form correctly returned no value. `IF((-8)^(1/3) < 0, 1, 2)` returned 2. `NOT((X-5)^0.5)` returned 0. `((X-5)^0.5) || 0` returned 0, while `0 || ((X-5)^0.5)` returned 1. `IF(INF,1,2)` returned 2 while `NOT(INF)` returned 0. At X = +∞, `IF(X == INF, 1, 2)` returned 2.
- Regression target is a domain failure for NaN conditions, as `sqrt`, `min`, and `max` already give, and `INF == INF` evaluating to true. `RV-42` fixed NaN propagation only for `min`, `max`, and `sign`.

#### 47. Medium · `exp` overflow fails the model where the equivalent power form fits

**Reproduced.** `exp` and `exp10` return no value for arguments above 709 and 308. The power operator returns infinity, so `1/(1+E^800)` correctly evaluates to 0, but `1/(1+exp(800))` fails.

- Evidence from `crates/fit/src/dsl.rs:3100–3113`.
- A four-parameter logistic written with `exp(H*(LogEC50-X))`, H = 200, and LogEC50 = −6 failed at X = −10, where the true value is the bottom plateau. Fitting 17 points over −12 to −4 returned `BadInitialValues`, while the same model written with `E^(...)` fitted.
- Regression target is evaluating the `exp` form to its limit so that it fits like the power form.

#### 48. Medium · Undocumented reserved names silently replace intended parameters

**Reproduced.** The names `E`, `T`, `PI`, `DS`, `DATASET`, `INF`, `NAN`, `LN2`, `LN10`, and `SQRT2` are reserved in any case, along with `X1`, `X2`, and so on. The curve-fitting specification in `docs/implementation/curve-fitting-implementation.md` §4 does not list them.

- Evidence from `crates/fit/src/dsl.rs:2223–2237,2881–2885`.
- `Y = E*X + B` fitted only B and fixed E at 2.71828. `Y = A*exp(-T*X)` fitted only A and treated T as X, so the model became exp(−X²). `Y = Bmax*X/(Kd+X) + e` added Euler's number. `Y = Pi*X + B` fitted only B.
- Regression target is documenting the reserved names and rejecting them as parameter names, or warning when they are used.

#### 49. Medium · ODE accuracy depends on the data's scale and on evaluation order

**Reproduced.** The RK45 tolerance is `1e-10 + 1e-8·|y|`, so the absolute term dominates whenever |Y| is below about 0.01. The trajectory cache also changes results depending on which X values were evaluated first. Each error is within the absolute tolerance, but relative accuracy collapses for small quantities.

- Evidence from `crates/fit/src/dsl.rs:1020–1039,1362–1363,1445–1448`.
- For `Y0 = A0`, `dY/dX = -k*Y`, A0 = 1e-9, k = 1, and X = 10, the model returned 1.14e-11 against the exact 4.54e-14. A0 = 1e-6, k = 2, and X = 5 gave a 12.6% error, while A0 = 1 gave 4e-7. At X = 10, a fresh evaluation gave 2.66e-12, and the same evaluation after X = 1 through 9 gave 4.59e-14. Noise-free data in molar units, with true A0 = 5e-9 and k = 0.3, fitted k = 0.30084 with SE 8e-4. The same data in nanomolar units fitted k = 0.3000000007.
- Regression target is a relative error of about 1e-6 regardless of magnitude and call order, with fitted rate constants independent of units. The implicit solver has the same flaw, reported as `R1-S03-04`.

#### 50. Medium · ODE evaluation cost is not bounded by right-hand-side size

**Reproduced with measurements.** The budgets count RK steps per point and per fit, not VM operations. A long chain of locals multiplies the cost of each right-hand-side call. The compile budget of 500k opcodes is also larger than the 100k per-evaluation VM step cap, so some programs that compile can never evaluate.

- Evidence from `crates/fit/src/dsl.rs:44–58`.
- A right-hand side with `n` chained locals and an oscillating forcing term, evaluated at one point (about 15,300 steps and 107,000 calls), took 2.57 s for 1,000 locals, 10.5 s for 4,000, and 29.7 s for 12,000. That is linear at about 0.28 ms per call. Extrapolated to the 50k-step point cap, one point takes about 97 s, and one 300k-step fit budget takes about 10 minutes, or up to four times that with several datasets. The equation can come from a project file or an MCP request. A 16,000-line program of about 128k opcodes compiled in 52 ms and then always evaluated to no value.
- Regression target is a budget that counts VM operations, or steps multiplied by program length, with a compile cap no larger than the evaluation cap. Related to `R1-S03-09` and `R1-S03-10`.

#### 51. High · The runs test pools residuals across data sets and hides systematic misfit

**Reproduced.** Residuals from every data set are sorted together by X before the runs test. Data sets in an XY sheet share their X values, so the interleaving turns long same-sign runs into alternation. The goodness-of-fit table and checklist report only this pooled value.

- Evidence from `crates/fit/src/solver.rs:2139–2148`, called at `:2639` for single and shared fits and at `:3001` for independent fits, and `crates/fit/src/result.rs:540–543,653–664`.
- A straight line was fitted to x = 0 through 12, with data set A at `(x−6)²` and data set B at `50 − (x−6)²`. Each curve fitted alone gave 3 runs and P(too few runs) 0.00758, which matches exact enumeration. Fitted together, independently or with a shared slope, the result was 24 runs and P 0.99999, and the checklist reported that residuals do not systematically deviate.
- Regression target is a runs test computed per data set within its own X order, reported or combined per curve and never interleaved across curves.

#### 52. Medium · Weighted fits test normality and constant variance on unweighted residuals

**Reproduced.** Shapiro–Wilk with its D'Agostino fallback, Anderson–Darling, Lilliefors, and the Spearman homoscedasticity check all receive raw residuals `y − ŷ`, even when weighting is active.

- Evidence from `crates/fit/src/solver.rs:2155–2169,2629–2632`, with the same wiring in the independent path at `:2934` and `:3001`.
- A line through the origin with relative 1/Y² weighting was fitted to 30 points, x = 1.25ⁱ and y = 2x(1 + 0.05z), with z the standard-normal quantiles in a fixed permutation. The weighting model is therefore correct. The fit reported W 0.641 with P 2.4e-7, Anderson–Darling P 4.6e-11, Lilliefors P 3.0e-9, and a homoscedasticity correlation of 0.887 with P 6.7e-11. All four failed. The weighted residuals give W 0.9988 with P near 1.
- Regression target is running these diagnostics on weighted residuals `(y − ŷ)√w` when weighting is active, as `RV-37` did for ROUT, or labelling them clearly. `RV-96` labelled only Sy.x and R² as unweighted.

#### 53. Medium · The linear regression summary always labels the slope interval 95%

**Reproduced.** The row label is fixed, while the interval is computed at the requested level.

- Evidence from `crates/fit/src/result.rs:144–147,157–160`.
- A linear regression at 99% on x = 1 through 8 and y = `[2.3, 4.1, 6.4, 7.7, 10.6, 11.8, 14.5, 15.6]` showed "95% CI Slope" with interval [1.7272, 2.1823]. R `confint` gives exactly that range at 99%, and [1.8046, 2.1049] at 95%.
- Regression target is a label derived from the requested level. This is the same pattern as finding 31.

#### 54. Medium · Decreasing transforms do not swap asymmetric error limits

**Reproduced.** Normalization, baseline ratios with a negative baseline, and delta-mode math transforms scale the lower and upper error cells by the absolute factor or derivative. They do not swap the two cells when the map reverses order, so the lower and upper errors trade places.

- Evidence from `crates/transform/src/ops/normalize_baseline.rs:337–347,546–555` and `crates/transform/src/ops/math.rs:585–595`.
- Normalizing with 0% at 100 and 100% at 0 maps v to 100 − v, the usual percent-inhibition setup. The row [80, −5, +15] spans [75, 95] and should become [20, −15, +5], but it became [20, 5, 15]. A baseline ratio with b = −2 on [10, 1, 3] gave [−5, 0.5, 1.5] instead of [−5, 1.5, 0.5]. A linear scale with slope −1 in delta mode gave [−10, 1, 3], while the replicate-endpoint mode correctly gave [−10, 3, 1]. Reciprocal and negative-log concentration transforms behave the same way.
- Regression target is swapping the lower and upper cells whenever the factor or derivative is negative. `RV-60` fixed only upper/lower limits and median/IQR modes.

#### 55. Medium · A domain-restricted math transform silently blanks the SD

**Reproduced.** The default replicate-endpoint mode evaluates g(mean − SD). If that falls outside the domain, the SD cell becomes empty without a warning, while the mean is transformed and N is kept.

- Evidence from `crates/transform/src/ops/math.rs:521–547` and the mode contract at `crates/doc/src/analysis.rs:128`.
- Log10 of mean 1, SD 2, N 5 gave [0, empty, 5] with no warnings. The delta method would give SD 2/ln 10 = 0.869. The documented contract describes asymmetric lower and upper limits, but the code stores a symmetric half-width.
- Regression target is a warning or a delta-method fallback whenever the spread is blanked, and code that matches the documented contract.

#### 56. Medium · Round trips through Multiple Variables move replicate values to other subjects

**Reproduced end to end through `table.convertArchetype`.** The pivots back to Grouped and XY place the s-th sorted entry in slot s rather than in its replicate slot. The one-factor pivot buckets each numeric column independently, and Grouped to Multiple Variables omits empty replicates. A subcolumn is the subject in repeated-measures designs (`crates/doc/src/analysis.rs:607–611`).

- Evidence from `crates/transform/src/archetype_convert.rs:990–998,1290–1302,1453–1492,2071–2084`.
- A Grouped table with three replicates and B = [10, empty, 30] came back as [10, 30, empty], so subject 3's value moved to subject 2. Both steps reported no loss. XY replicates [1, empty, 3] came back with two replicates, [1, 3]. Multiple Variables rows (A 1, B blank) and (A 2, B 20) became Grouped A = [1, 2] and B = [20, empty], which pairs B's 20 with A's 1.
- Regression target is using the replicate value or source-row identity as the slot index. Grouped summary to Multiple Variables also still reports no schema loss, the surviving part of `R2-S04-05` (`archetype_convert.rs:1093–1100`).

#### 57. Medium · A ragged plate-map grid shifts wells

**Reproduced through `Session::execute`.** `table.importPlateMap` flattens a grid of values without checking each row's length and validates only the total.

- Evidence from `crates/engine/src/cmd/table.rs:4282–4296`.
- With 96 values where row A had 11 entries and row B had 13, B01 imported the 13th value, 7, instead of 99.
- Regression target is requiring the plate's row count and every row's length to match the plate geometry.

#### 58. Medium · Unknown baseline operations and normalization anchors fall back to defaults

**Reproduced.** The `transform.run` parsers map unrecognized strings to Difference and to the column minimum. The math preset and error-mode parsers reject unknown values.

- Evidence from `crates/engine/src/cmd/analysis.rs:2581–2598`.
- `"operation": "pct_change"` ran as Difference. Columns A [10, 20] and B [15, 30] gave [5, 10] instead of percent changes [50, 50]. `table.pasteLink` accepts `pctchange`, a spelling this parser also maps to Difference. `"hundred_anchor": "maximum"` became the column minimum, so [20, 10, 40] with the first row at zero gave [0, 100, −200] instead of [0, −50, 100].
- Regression target is a `BadInput` error for unrecognized strings.

#### 59. Medium · Tidy-to-Grouped drops records whose numeric X is blank

**Reproduced.** For numeric X levels, the lookup has no missing-level key, so the record is skipped. Text levels create a "(Missing)" row instead.

- Evidence from `crates/transform/src/plate_and_tidy.rs:1117–1118,1323–1358`.
- Records (1, A, 10), (blank, A, 20), and (2, A, 30) produced rows 1 and 2 only. The value 20 was lost without a warning.
- Regression target is a missing-level row, or a warning, in the numeric path as in the text path.

#### 60. Medium · Selecting from an XY table into another table kind drops the X column

**Reproduced.** The select-and-transform path flattens data sets only, so a projection from XY to another kind loses X without a warning. Survival times are dropped the same way, which was traced and not run. The interface lets users choose the target kind (`crates/ui-egui/src/dialogs_canvas.rs:2833`).

- Evidence from `crates/transform/src/ops/reshape_select.rs:1417–1449`.
- An XY table with X Time [1, 2] and Y Conc, projected to Multiple Variables, contained only `Conc` and returned no warnings. `convert_datasheet_archetype` on the same table keeps X as a variable.
- Regression target is carrying X or survival times as the first column, or warning. `R1-S04-09` covered subcolumns only.

#### 61. Medium · Pruning rows leaves exclusion notes on the wrong cells

**Reproduced.** Pruning clones the source and then removes or bins rows, but it keeps exclusion notes at their original coordinates. Random sampling remaps notes correctly (`crates/transform/src/randomize.rs:696`).

- Evidence from `crates/transform/src/ops/reshape_select.rs:369–405`.
- A column [10, 20, 30*, 40, 50] with a note on row 2 was pruned to every second row, giving [10, 30*, 50]. The note stayed at row 2 and now labelled the active value 50.
- Regression target is remapping notes in the same way as `remap_exclusion_notes`. This affects audit metadata only.

#### 62. Medium · Curve AUC mishandles summary rows, tied X, and total area

**Reproduced through the public API. No engine, interface, MCP, or CLI code calls `curve.rs`.** The sheet AUC averages error and limit cells as if they were replicates. Tied X values are not collapsed, so the result depends on row order. `total_area` is documented as positive plus negative area but is set to the net area.

- Evidence from `crates/transform/src/curve.rs:59,148–161,191,239`.
- Two points of [10, 1, 3] in mean, low, high mode gave AUC 4.667 with SE 1.93 instead of 10. The points (0, 0), (1, 2), (1, 10), (3, 0) gave 11, and swapping the tied rows gave 7. The tie-collapsing AUC in `crates/stats/src/column.rs:1639` gives 9. For (0, 2), (1, −2), the positive and negative areas were 0.5 each, but `total_area` was 0.
- Regression target is using only the central value of summary rows, collapsing tied X values, and setting `total_area` to match its documentation, or removing the unused API. Related to `R1-S04-10`.

#### 63. Medium · Savitzky–Golay smoothing treats non-uniform X as uniform at small scales

**Reproduced through the public API. No engine, interface, MCP, or CLI code calls `curve.rs`.** The uniform-grid check uses an absolute tolerance of 1e-7 when the spacing is below 1. Nanometre, nanosecond, or molar X values therefore pass, and the fixed convolution weights are applied to uneven spacing.

- Evidence from `crates/transform/src/curve.rs:348–357`.
- X = [0, 1, 2, 10, 50, 60, 70] × 1e-9 with linear y = x × 1e9 gave 0.457 at index 2. A linear series must be reproduced exactly, which gives 2. The same data in whole units were reproduced exactly.
- Regression target is a relative tolerance.

Code-trace notes from this area, not numbered: the Probit transform documentation says Φ⁻¹, but the code returns 5 + Φ⁻¹. `FirstAndLastLinear` baselines interpolate over the row index instead of X. Transposing an XY table without swapping titles discards X without a warning. XY replicates converted to Multiple Variables lose row titles. Plate templates with duplicate replicate numbers move the extra well into the next replicate slot. Random-sample seeds are still stored as floating-point values (`crates/transform/src/ops/mod.rs:186`), the `R1-S04-12` pattern.

#### 64. High · Rank-deficient least squares flags aliased columns but still gives them nonzero coefficients

**Reproduced.** The least-squares coefficients come from the SVD minimum-norm solution of the column-scaled design, while `aliased_columns` comes from the QR pivot order. Both the field documentation and `PivotedQr::solve_basic` set aliased coefficients to zero. Multiple linear regression prints a coefficient, SE, CI, t, and P for every term, and the GLM sheet builder never reads `aliased_columns`. A retained predictor can therefore show a halved estimate with no marker. The auditor rated this Medium; it is rated High here because the wrong estimate is silent.

- Evidence from `crates/math/src/linalg/qr.rs:46–49,627–646` and `crates/stats/src/regression.rs:284–309,1632–1680`.
- For X = [1, x, x], x = 0.1 + 0.5i for i = 0 through 9, and y = 1 + 2x + 0.01 sin(7i), the coefficients were (1.00649, 0.99864, 0.99864) with column 2 aliased. R `lm` gives (1.00649, 1.99728, NA). With the duplicate scaled by 1000, the slope was still halved. A 6×6 effect-coded design with three aliased columns gave intercept 7.667 and halved, duplicated effects. `lm` gives 15.333, −4.833, −0.583, and three NA values.
- Multiple and general linear regression, multivariate fits (`crates/stats/src/multivariate.rs:2154`), and ANOVA through `solve_linear_least_squares` share this solver. Those calls were traced and not run.
- Regression target is a zero or unavailable coefficient for each aliased column, with a zero-variance covariance row, matching the `lm` basic solution, or a visible flag in the output. This follows on from the `RV-48` change to SVD-aligned solutions.

#### 65. Medium · The Jacobi SVD fails to converge on exactly rank-deficient matrices

**Reproduced.** The only convergence test is a maximum pair cosine of 4ε. Pairs are skipped only for exactly zero column norms or cosines at most ε. A column reduced to rounding noise keeps an O(1) normalized cosine, so the sweeps never converge. The QR-preconditioned path for tall matrices is affected as well.

- Evidence from `crates/math/src/linalg/svd.rs:64–80,84–106,140–146,160–164`.
- A 6×6 matrix with entries 0 and ±1, X = [A, A] from a two-way effect-coded design with one B level observed, returned `NotConverged` after 64 sweeps. R gives singular values (3.4641, 3.4641, 2, 0, 0, 0). A 7×3 matrix with a scaled duplicate column also failed. Of the matrices tried, 75 of 1,107 rank-deficient effect-coded designs failed, and 2 of 200 tall standardized matrices with a dependent column failed.
- Multiple logistic and Poisson regression call `svd(x)?.rank()` (`crates/stats/src/regression.rs:554,759`), so collinear predictors return a convergence error and not a singularity. PCA aborts when one variable is redundant (`crates/stats/src/multivariate.rs:442`). The rank-deficient Jacobian branch of nonlinear-fit diagnostics fails without reporting an ambiguous fit (`crates/fit/src/diagnostics.rs:305`). These callers were traced and not run. This is the likely root cause of the SVD error in finding 30.
- Regression target is convergence with the correct rank for exact duplicate and linear-combination columns on both paths. Distinct from `R1-S01-03`, which concerned `sym_eigen`.

Lower-severity notes from this area, reproduced unless marked:

- **Huge-df t log tails:** `student_t_ln_sf` and `student_t_ln_two_sided_p` switch to the normal approximation for df ≥ 1e12 (`crates/math/src/dist/core.rs:38,233`). At t = 1e10 and df = 1e13 they return −5e19 instead of −8.06e13. No caller outside the math crate is affected.
- **Noncentral t opposite tails:** these are computed as 1 − CDF (`crates/math/src/dist/noncentral.rs:382`), so for example `nct_sf(2, 10, −10)` underflows to 0 instead of 1.5e-29. The absolute error is at most 1e-16, so power and noncentral intervals are unaffected.
- **Studentized range accuracy:** the documented 11 to 14 digits (`crates/math/src/dist/studentized_range.rs:7`) does not hold at k = 200 with df ≤ 3, where the relative error is about 3e-5. Accuracy is within 1e-14 for k ≤ 50.
- **Far lower noncentral F tail:** `ncf_cdf(1, 1, 100, 500)` returns 0 instead of 4.8e-101.
- **Incomplete beta inverse with tiny b:** `reg_beta_inv(0.1, 1, 1e-30)` has relative error 2.3e-7 because the polishing step exits early (`crates/math/src/special/inc_beta.rs:399–434`).
- **t quantile for a tiny probability:** `student_t_isf(1e-300, 1)` returns `NotConverged` although the answer 3.18e299 is representable. Other zero-iteration `NotConverged` errors occur only where the true answer is not representable.
- **Quadrature tolerance (traced):** `integrate_adaptive` returns success when its error estimate is up to 16 times the requested tolerance, which is not documented (`crates/math/src/quad.rs:230`).

### Wave 4 verification and remaining coverage

- Native macOS arm64, Rust `1.99.1`, lockfile unchanged, offline resolution. The first auditors for all five areas were stopped by a usage limit. Replacements resumed the four areas whose worktrees and partial probes still existed and restarted the transform area from scratch. Every finding was reproduced by a temporary probe through the public API or `Session::execute`, and every probe was removed afterwards with the worktrees left clean. Reference values came from closed forms, cited published procedures, exact enumeration, finite differences, analytic ODE solutions, and R 4.5.2 with `nlme` as an external numerical oracle only. Where R's own routines are inaccurate in deep tails, independent oracles were built from cited identities.
- Existing tests passed:
  - `curvebook-math`: all targets, including `nist_strd` and `rv_ledger_math`.
  - `curvebook-fit`: all targets, including `nist_strd`, `r46_linear_oracles`, `r46_library_oracles`, `r46_shapiro_rk45_oracles`, `dsl_fuzz_props`, `equations_and_dsl_blocks`, `pharma_binding_and_dsl`, `rv_ledger_fit`, and `audit_r2_s03`.
  - `curvebook-transform`: all 95 tests.
  - `curvebook-stats`: `r35_anova_reml_oracles`, `r36_rm_reml_oracles`, `r34_unstructured_reml_oracles`, `r35_power_multiplicity_oracles`, `power_oracle`, `rv_ledger_stats_models`, `wave21_r97_r912_clustering`, and the library tests.
  - `curvebook-engine` ran only the temporary probes.
- Not covered in this wave:
  - Clustering, k-means, multiple imputation, and parallel analysis.
  - Power curves and sensitivity tables, the multi-component REML iteration cap (traced as having no converged flag, not reproduced), and unstructured REML internals beyond the optimizer.
  - The `pvalue_stack` sheet extraction, profile-likelihood and ROUT internals, Bessel and hypergeometric functions, and `tex_format.rs`.
  - The `import.rs` growth helpers, plate-template validation, subject randomization, and distribution simulation.
  - Cholesky beyond the prior fixes, `rng.rs`, and pseudoinverse tolerances.
- Still outside all waves: GUI and figure code, engine table commands beyond the adapters reviewed, the layout and methods-report modules, and live GUI behavior.

## Wave 5 coverage log

Auditors ran in worktrees at `033d4be`, while `main` was at `ce80964` on 2026-11-19. Between those commits `main` merged substantial code changes, including commits whose messages report fixes for ledger findings 1–16 (`3583bad`, `69c3891`, and `c17e0e0`). Those fixes have not been re-verified in this ledger. Each Wave 5 finding below was re-checked against the cited code at `ce80964` before it was recorded, and line references are for `ce80964`. `main` may continue to move. GUI and figure code (`crates/{ui-egui,chart,render,svg,pdf,tools,text}`) remain out of scope. Each area is audited in its own worktree. Builds reuse the retired Wave 3 and Wave 4 build directories, and the unused Wave 3 directories were deleted.

| Priority | Area | Coverage and status |
|---|---|---|
| 1 | Descriptive statistics and unpaired and one-sample t-tests | Reviewed the descriptive core, custom percentiles, geometric, harmonic, and quadratic means, skewness and kurtosis, summarized-data extraction, the descriptive, pooled, and one-sample tabs, the exact median interval, the normality battery, Grubbs, ROUT, and the one-sample t-test in `crates/stats/src/column.rs`. Reviewed the F test, Levene, effect sizes, Student, Welch, and lognormal t-tests, summary-data t-tests, Multiple Variables group and response resolution, and the sheet builders in `two_group.rs`. The common extraction and quantile helpers were also reviewed. No frequency-distribution or MAD analysis exists. Values matched R 4.5.2 apart from the findings. Findings 66–69 reproduced. Complete. |
| 2 | Engine document integrity | Reviewed command execution, undo snapshots and eviction, undo and redo, and the read-only branch in `crates/engine/src/lib.rs`. Reviewed topological ordering, cycle checks, scoped recalculation, dirty closure, link refresh, and hook resolution in `dag.rs`. Reviewed sheet delete, duplicate, family clone, freeze, and reorder in `cmd/sheet.rs`; paste, paste-link, clear, archetype conversion, and project-import merge in `cmd/table.rs`; and row placement in `crates/transform/src/import.rs`. Cycle rejection, whole-document undo restoration, redo clearing, failed-command rollback, and missing-hook failures were correct. The engine has no commands that insert, delete, or move rows and columns directly. Findings 74–77 reproduced. Complete. |
| 3 | CLI batch and convert, platform file layer, and data export | Reviewed `cmd_run_batch.rs` in full, plus the load, export, argument, and target code in `cmd_convert.rs`, `sandbox.rs`, the `main.rs` dispatch, and direct command persistence in `cmd_commands.rs`. Reviewed `crates/platform/src/{fs,path_guard}.rs` in full and `process.rs`, `crates/format/src/export.rs` in full, the CSV token format, parsing, and exporters, the registry exporters and archive save guards, and the engine table and analysis export paths. `WorkdirGuard` rejected every escape tried, and `process.rs` raised no issues. Findings 70–73 reproduced. Complete. |
| 4 | Remaining statistics | The first auditor was stopped by a usage limit, and a restart completed the area at `ce80964`. Reviewed PCA, Horn parallel analysis, k-means, distances, silhouette, hierarchical linkage, cophenetic correlation, dendrogram cutting, the clustering runner, Rubin pooling, and multiple imputation in `crates/stats/src/multivariate.rs`. Reviewed multiple linear regression, VIF, the Type III table, logistic and Poisson IRLS, Hosmer–Lemeshow, the classification table, GLM design and categorical coding, and `run_glm_analysis` in `regression.rs`. Reviewed Cohen and Fleiss kappa, Barnard, binomial intervals and the exact test, and parts-of-whole goodness of fit in `categorical.rs`, plus the survival pairwise plan. Hierarchical clustering, Barnard's pooled-score test, multiple regression with VIF, the B > 0 Rubin path, Cohen's kappa variances, and parallel-analysis thresholds matched R. `R2-S02-02` appears fixed, and `R2-S07-07` (kappa requests on parts-of-whole sheets running goodness of fit) is still present. Findings 84–92 reproduced. Complete. |
| 5 | Reported text, result labels, and parameter validation | Reviewed `crates/engine/src/methods_report.rs` (renderers skimmed), `crates/doc/src/result.rs`, P-value stars and formatting in `crates/math/src/pvalue.rs`, analysis schemas, spec building, overrides, patching, and enum parsers in `crates/engine/src/cmd/analysis.rs`, and parameter strictness in `cmd/params.rs`. The report text was cross-checked against the two-group, row t-test, survival, nested, GLM, and fit result builders. A search of every confidence-interval header and every default-95% site found no fixed-level labels beyond findings 31 and 53. Findings 78–83 reproduced. Complete. |

### Wave 5 findings

#### 66. High · t-tests on Multiple Variables tables can silently test the ID column

**Reproduced through `run_analysis` and `Session::execute`.** Without exactly one selected categorical column, Multiple Variables resolution returns nothing, and the two-group runner falls through to the generic path. That path takes the first two variables from `extract_numeric_columns`, which ignores variable roles. The one-sample test uses the raw selection when no response column is given, so it tests variable 0.

- Evidence from `crates/stats/src/two_group.rs:1720–1724,2062–2071,2132–2158` and `crates/stats/src/common.rs:258–300`.
- A table had a label column `SubjectID` with values 101–106, then `Before` and `After` with six values each. An unpaired t-test with no selection compared SubjectID mean 103.5 with Before mean 5.083 and gave t = 127.7, P = 2.1e-17, and Cohen's d = 73.7. The result was up to date with no warning. A one-sample test against 0 also tested the ID column. Before versus After gives t = −8.40.
- Paired t-tests take the same first-two-variables route, and descriptive statistics and outlier detection on these tables also include label and categorical columns (`crates/stats/src/column.rs:474–505`). Those routes were traced and not run.
- Regression target is excluding label and categorical roles from default selections, or a typed error. This is the same role-filter gap as `R2-S02-07` and is distinct from findings 13 and 32.

#### 67. Medium · Lognormal unpaired t-tests report log-scale values under arithmetic labels

**Reproduced.** The lognormal test runs Student's t on ln(x), and the shared sheet builder labels the rows "Mean", "Difference between means", the difference interval, and Cohen's d without any log-scale note.

- Evidence from `crates/stats/src/two_group.rs:667–690,2563–2600`.
- A = [10, 100, 1000] and B = [1, 10, 100] were reported as means 4.605 and 2.303, difference 2.303, and CI (−2.92, 7.52). The arithmetic means are 370 and 37. The reported values are means of ln(x), and the effect sizes are also on the log scale. The geometric-mean ratio of 10 is correct.
- Regression target is log-scale labels, or back-transformed geometric means with ratio intervals.

#### 68. Medium · Custom percentiles use an undocumented definition and never reach the result sheet

**Reproduced.** The specification documents Hyndman–Fan Type 6 percentiles. `compute_custom_percentiles` uses the Type 7 helper, and the descriptive tab never emits either custom value.

- Evidence from `crates/doc/src/analysis.rs:317`, `crates/stats/src/column.rs:148–151,212,292–300`, and `crates/stats/src/common.rs:41–61`.
- For [1, 2, 3, 4, 5] at 90, the API returned 1.4 and 4.6, the Type 7 values. Type 6 gives 1 and 5. The result sheet showed neither. With 75, the custom 25th percentile was 2.0 while the sheet's 25% percentile row, which uses Type 6, showed 1.5. `docs/implementation/statistics-implementation.md:53` also promises configurable quantile types that do not exist. An existing oracle asserts Type 7, so the documentation may be what is stale.
- Regression target is one documented definition with the requested percentiles shown in the result.

#### 69. Medium · Glass's Δ substitutes the other group's SD when it is undefined

**Reproduced.** When the standardizing group's SD is 0, the code divides by the other group's SD under the same label. The label never names the standardizing group.

- Evidence from `crates/stats/src/two_group.rs:313–316,344–356`.
- A = [5, 5, 5] and B = [1, 2, 3] gave a valid Student t = 5.196 on 4 df, and Glass's Δ = 3.0 with CI (−0.15, 6.12). Δ = diff/s_A is undefined when s_A = 0, so 3.0 = 3/s_B is a different statistic.
- Regression target is an empty or flagged Δ when the standardizing SD is 0, and a label that names the standardizing group.

Related to finding 8, not numbered: the one-sample t-test also fails at n = 2 because the Hedges correction factor is out of domain. The raw API, the summary-data API through `analysis.run`, and the descriptive one-sample tab were reproduced at `033d4be`, and the descriptive tab silently shows blank t, df, and P. R gives t = 2 and P = 0.295. The `69c3891` remediation reports a fix for finding 8, so this caller should be rechecked against that fix.

#### 70. High · CLI convert, export, and batch overwrite a newer-version project archive

**Reproduced with the CLI binary.** When the output format is a native archive, the CLI builds the bytes and writes them with `write_atomic`. It skips the existing-target version check used by `save_project_file` and `ArchiveAdapter::export_path`.

- Evidence from `apps/curvebook-cli/src/cmd_convert.rs:140–164,429–435,470–479`, `apps/curvebook-cli/src/cmd_run_batch.rs:155–168`, `crates/format/src/archive.rs:242–247`, and `crates/format/src/registry.rs:403–409`.
- The output paths held synthetic archives marked `CURRENT_SCHEMA_VERSION + 1`. `convert input.curvebook conv.curvebook`, `export input.curvebook --out exp.curvebook`, and a batch manifest exporting `batch.curvebook` all exited 0 and replaced those archives. The direct `project.inspect --out` command refused with an unsupported-version error and left its file byte for byte intact.
- Regression target is a typed version error with the target preserved for all three subcommands. This is the CLI entry point of the class in finding 1, `R2-S06-03`, and `RV-07`.

#### 71. High · Batch runs report success and export empty results when an analysis fails

**Reproduced with the CLI binary.** Recalculation collects failed result sheets, but neither the batch runner nor `table.importCsv` looks at them. `Session::recalculate_all` discards the report, so the batch prints success. A failed result exports as a header-only CSV. `analysis.run` itself turns a failed status into an error.

- Evidence from `crates/engine/src/dag.rs:460–466`, `crates/engine/src/cmd/project.rs:45–56`, `crates/engine/src/lib.rs:1420–1425`, `crates/engine/src/cmd/table.rs:3085–3093`, `crates/format/src/export.rs:55–69`, and `apps/curvebook-cli/src/cmd_run_batch.rs:149–153,177–192`.
- A template project with a linear regression on `data_1` was rerun with a new plate CSV whose Y values were all blank. In a second case, one manifest added the analysis and then replaced the data with text values. Each run exited 0, reported `ok: true` with two exports, and wrote a header-only CSV. The JSON export recorded the failure "regression input contained zero active observations". `docs/implementation/scripting-and-automation.md:56` promises exit 1 for analysis failures and "never silent partial exports".
- Regression target is a typed error that names each failed result sheet after the final recalculation, without writing its exports.

#### 72. Medium · CSV export allows spreadsheet formula injection

**Reproduced.** CSV fields are quoted only for delimiters, quotes, and line breaks. Titles, row labels, text cells, and result labels that begin with `=`, `+`, `-`, or `@` are written unchanged and become live formulas when the file is opened in a spreadsheet. These values can come from imported third-party files.

- Evidence from `crates/format/src/csv.rs:362–400` and `crates/format/src/export.rs:307–338`, reached through `table.exportCsv`, `analysis.exportCsv`, and CLI convert, export, and batch.
- Text cells `=1+2`, `@SUM(A1)`, and `+1+cmd`, and a data set title `=HYPERLINK("http://example.invalid","x")`, were exported unchanged. The title was quoted only because it contains a quotation mark, which spreadsheets still evaluate.
- Regression target is neutralizing text fields that start with `=`, `+`, `-`, `@`, tab, or carriage return, while leaving numeric cells such as `-1.5` unchanged.

#### 73. Medium · Exported CSV does not import back faithfully

**Reproduced.** Text cells are written verbatim, and re-import parses the unquoted token, so the format cannot escape missing-value markers, asterisks, or numeric-looking text. Multi-subcolumn headers are written as `title:suffix`, but import keeps only the text before the first colon.

- Evidence from `crates/format/src/csv.rs:301–380,603–616,975`.
- After export and re-import, text `NA`, `.`, and `NULL` became empty. `abc*` and `*abc` became `abc` marked excluded. `007` became the number 7 and `1e3` became 1000. Surrounding spaces were trimmed, and the Mean/SD/N title `Drug: high` became `Drug`. Numbers round-tripped exactly for every delimiter and decimal pair tested, apart from −0 becoming 0. The exporter is documented as preserving headers, titles, and asterisk exclusions.
- Regression target is quote-aware escaping in which a quoted field is always text, and splitting headers at the last colon. This is the export side of the parser reuse in finding 26.

Lower-severity notes from area 3, not numbered:

- `write_atomic` replaces read-only targets and breaks hard links, as atomic rename does (`crates/platform/src/fs.rs:77–92`). The owners should decide whether read-only targets should be respected.
- When a later batch export fails, earlier exports are already written. The run exits 1, but the error does not list them (`apps/curvebook-cli/src/cmd_run_batch.rs:156–175`).
- Only the immediate parent directory is synced, so newly created parent directories are not durable (`crates/platform/src/fs.rs:94`).
- Temporary files left after a crash are never cleaned up.
- The archive fallback from `R2-S12-05` remains at `apps/curvebook-cli/src/cmd_convert.rs:379–385`.

#### 74. High · Converting a table's kind silently retargets every dependent column reference

**Reproduced through `Session::execute`.** `table.convertArchetype` replaces the sheet in place and recalculates. Analysis selections, graph series indices, column-title hooks, and other sheets' links are not remapped, although XY-to-Column conversion moves X into column 0 and shifts every Y data set by one. Column-to-XY shifts them the other way. The conversion builds a coordinate map but applies it only to exclusion notes. Each converter also drops links into the converted sheet, and the preview still reports no loss.

- Evidence from `crates/engine/src/cmd/table.rs:4229–4234` and `crates/transform/src/archetype_convert.rs:170–190,534–600,676–698,797,904,1594`.
- An XY sheet had X 1–4 and Y 100–130, with descriptive statistics on data set 0, a graph, and a paste-linked copy of the Y column. Converting it to a Column table reported no loss and no warnings. The statistics stayed up to date but now described X (median 2.5 instead of 115). The linked copy changed from 100–130 to 1–4, and the graph series plotted X.
- Regression target is remapping dependent indices through the conversion map, or refusing or marking dependents stale with a warning. Related to findings 32 and 56; `R2-S07-01` covered only the alias.

#### 75. High · Undoing a project open keeps the new file path, so the next save overwrites that file

**Reproduced.** `project.open` sets both the document and the session file path. Undo and redo snapshot and swap only the document. After opening A, opening B, and undoing, the session holds A's document with B's path. Desktop Save and MCP `save_file` without a path both save to the session path.

- Evidence from `crates/engine/src/cmd/project.rs:154`, `crates/engine/src/lib.rs:1355–1416`, `apps/curvebook/src/desktop_host.rs:2934–2939`, and `crates/mcp/src/formats.rs:617–623`.
- `project.open a.curvebook`, `project.open b.curvebook`, `edit.undo`, then `project.save` to the session path wrote A's sheets into `b.curvebook`, and B's data was lost. This is reachable from MCP or CLI command sequences, and from live-attach MCP `open_file` followed by desktop undo and save. Native desktop open uses `Session::from_document` and is not affected.
- Regression target is undo restoring the path that matches the restored snapshot, or history being reset on open. Related to `R1-S12-03` and `R1-S12-05`.

#### 76. Medium · Cloned pipelines stay bound to the original family's results and Info sheet

**Reproduced.** `clone_downstream_graphs` remaps graph sources, comparisons, and annotations but not regression overlays, compact letter displays, or inline hook tokens. The local `remap_analysis_hooks` in `sheet.rs` handles only nonlinear regression, while the shared helper also remaps normalization anchors and power Info sheets.

- Evidence from `crates/engine/src/cmd/sheet.rs:1458–1532` and `crates/engine/src/cmd/helpers.rs:467–491,559–585`.
- After cloning, the clone graph's regression overlay still pointed at the original fit and drew the original data's curve over the clone's points. A cloned compact letter display still pointed at the original ANOVA, and deleting that ANOVA removed the clone's letters. A cloned normalization hooked to the original Info constant: after the clone's Info constant was changed from 200 to 400, it still output 0, 33.3, and 66.7 instead of 0, 14.3, and 28.6. The shared graph remap used by the project-import merge also skips compact letter displays; that was traced and not run.
- Regression target is every overlay, letter display, inline token, normalization anchor, and power Info reference pointing at the cloned IDs. `R1-S07-10` covered only a hang, and `RV-61` covered delete pruning.

#### 77. Medium · Inserting pasted or imported rows leaves exclusion notes on the old rows

**Reproduced for `table.paste`. Paste-link and CSV import share the pipeline and were traced.** Inserting rows shifts cells, X values, and row titles but not exclusion notes. The later prune keeps a note wherever its old coordinate is still excluded.

- Evidence from `crates/transform/src/import.rs:683–685,942–1000` and `crates/engine/src/cmd/table.rs:2517–2522`.
- In a column [1, 2, 3, 4], row 0 was excluded without a reason and row 1 with the reason "pipetting". Pasting 99 at row 0 in insert mode gave [99, 1*, 2*, 3, 4]. The note stayed on row 1, now value 1, and value 2 lost its reason.
- Regression target is shifting notes at or below the insertion point. This is the same class as finding 61 and `R2-S05-01`.

#### 78. High · A fractional confidence level such as 0.95 is accepted as 0.95% confidence

**Reproduced.** `confidence_level` is an alias for `confidence_percent`, and validation accepts any value from 0.01 to 99.99. A caller passing 0.95 gets 0.95% intervals and α = 0.9905, so almost every result is marked significant. One of the repository's own test fixtures passes `"confidence_level": 0.95` (`crates/format/tests/recovery_and_interop_parity.rs:227`).

- Evidence from `crates/engine/src/cmd/analysis.rs:349,436,509,539` and `crates/math/src/pvalue.rs:26–34`.
- An unpaired t-test with `confidence_level: 0.95` labelled its row "0.95% CI of difference" with interval [−0.512, −0.488], where the 95% interval is [−2.806, 1.806]. With P = 0.63, the pairwise record was marked significant. The methods report stated 0.95% intervals and α = 0.9905.
- Regression target is rejecting values at or below 1, or treating them explicitly as fractions with a warning, across `analysis.run`, `stats.run`, `fit.run`, `transform.run`, and `analysis.updateParams`.

#### 79. Medium · Analysis commands silently ignore unknown or inapplicable keys

**Reproduced.** The analysis command schemas allow additional properties and collect every unrecognized key. The patch logic skips keys that are not fields of the resolved specification, and the analysis specifications do not deny unknown fields.

- Evidence from `crates/engine/src/cmd/analysis.rs:331,355,517` and the patch and override logic in the same file.
- One-way ANOVA with `post_test: "dunnett"` and `confidence: 99` ran with no post-test at 95%. An unpaired t-test with `tails: 1` and `one_sided: true` stayed two-tailed. With `alpha: 0.01` and P = 0.037, the test kept 95% and marked the result significant. `analysis.updateParams` with the misspelled keys `two_taild` and `onetailed` returned success and changed nothing.
- Regression target is a `BadInput` error for keys unknown to the resolved kind. `RV-16` fixed only closed schemas, and finding 58 is the same class for transform options.

#### 80. Medium · The engine ignores the requested ROUT Q and misreports out-of-range values

**Reproduced, except the 50% cap, which was traced.** `parse_fit_method("rout")` hard-codes Q = 1%, and no fit override reads `q_percent`. The solver accepts Q = 0 and silently caps the false-discovery rate at 50%, while the result and the methods report print the requested Q.

- Evidence from `crates/engine/src/cmd/analysis.rs:2678`, `crates/fit/src/solver.rs:2240`, `crates/fit/src/result.rs:711–719`, and `crates/engine/src/methods_report.rs:817–823`.
- A four-parameter logistic fit with one outlier and `method: "rout"`, `q_percent: 5` stored and reported Q = 1%, and `fdr_q_percent: 5` behaved the same way. With Q = 0, the checklist reported one outlier removed, although a zero false-discovery rate should reject nothing. With Q = 80 the report says 80% while the solver applies 50%.
- Regression target is an engine Q override, rejection of Q ≤ 0 and other out-of-range values, and reporting of the Q actually applied. `RV-76` fixed only the interface parser.

#### 81. Medium · Multiple row t-tests apply Bonferroni or Šidák under the requested method's name

**Reproduced.** For raw-P families, Tukey, Newman–Keuls, Dunnett, Dunnett T3, Games–Howell, Dunn, and linear-trend requests become Bonferroni, and Tamhane T2 becomes Šidák. The result summary prints the requested method, and the methods report describes it.

- Evidence from `crates/stats/src/multiplicity.rs:464–480`, `crates/stats/src/two_group.rs:1932,1975`, and the multiplicity description in `crates/engine/src/methods_report.rs`.
- Multiple row t-tests with `multiplicity: "tukey"` adjusted raw P [0.0436, 0.908, 0.000998] to [0.131, 1.0, 0.00300], exactly Bonferroni. The summary row read `TukeyHsd`, and the report described Tukey's HSD across all pairs. Dunnett was described as a many-to-one comparison against a control, which row tests do not have.
- Regression target is rejecting methods that do not apply to raw-P families, or labelling and footnoting the procedure actually applied. Same class as findings 34 and `RV-67`.

#### 82. Medium · Significance stars use a fixed 0.05 cut-off while significance flags use the analysis α

**Reproduced.** Stars mark any P ≤ 0.05, while significance flags and the methods text use α = 1 − confidence with a strict inequality.

- Evidence from `crates/math/src/pvalue.rs:128–143`, the `p <= conf.alpha()` flag in `crates/stats/src/two_group.rs`, and the α statement in `crates/engine/src/methods_report.rs`.
- At 99% confidence, an unpaired t-test with P = 0.0369 showed "0.0369 (*)" in the result and the report table, while its record was marked not significant and the report stated α = 0.01. Graph brackets reuse these stars.
- Regression target is a star cut-off derived from the analysis α, or an explicit statement that stars use fixed thresholds, with one consistent boundary convention.

#### 83. Medium · The methods report describes post-tests that were remapped or never computed

**Reproduced.** The survival and nested descriptions use the requested multiplicity method. Survival pairwise comparisons map Tukey, Games–Howell, T2, T3, Dunn, and linear trend to Bonferroni, with a result footnote. Nested analyses compute comparisons only with at least three groups, and not for the nested t-test.

- Evidence from `crates/engine/src/methods_report.rs:454–464,606–650`, the survival pairwise plan in `crates/stats/src/survival.rs`, and the nested comparison condition in `crates/stats/src/anova.rs`.
- Kaplan–Meier with three cohorts and `multiplicity: "tukey"` footnoted Bonferroni adjustment, while the report described Tukey's HSD. A nested t-test with Tukey produced no pairwise comparisons, while the report said multiple comparisons used Tukey's HSD.
- Regression target is describing the effective plan, as the Welch and Kruskal–Wallis branches already do. This extends item 3 of `R2-S07-03`, whose other items appear fixed.

Lower-severity notes from areas 2 and 5, not numbered:

- A custom P-value format with zero decimals or a negative scientific threshold is clamped during formatting, but the report prints the raw settings.
- Profile-likelihood intervals fall back to Wald intervals for ambiguous fits or zero df, while the report still says profile likelihood (`crates/fit/src/solver.rs`; traced).
- Duplicating a data sheet shares the original's Info sheet instead of cloning it. This is a design question.

#### 84. High · Logistic separation detection depends on the predictor's units, and non-convergence is never shown

**Reproduced.** Separation is flagged whenever any raw coefficient exceeds 15 in absolute value, and the flag permanently adds a ridge penalty. Whether separation is detected therefore depends on the units of X. The logistic and Poisson `converged` flags are stored but never read by the result builder, so a fit that did not converge can still be reported as converged.

- Evidence from `crates/stats/src/regression.rs:560–566,588–600,650,723,992,1983–1993`.
- The data were 40 points that are not separated, with x = (i/40)·s. R `glm` converges with slope 15.46 per unit of i/40. With s = 100, CurveBook matched R. With s = 1, it flagged separation and reported slope 15.37. With s = 0.25 it reported slope 18.0, SE 6.0, and deviance 28.06, where R gives 61.85, 21.42, and 16.98. With s = 0.1 it reported 18.0 against R's 154.6. In the other direction, x = 100–1000 with y = (x > 500) is completely separated, but the coefficients never exceed 15. The checklist reported "IRLS converged without separation" with odds ratio 1.31, CI (2.3e-11, 7.4e10), P 0.98, and Hosmer–Lemeshow P 1.0, while the stored convergence flag was false.
- Regression target is a scale-invariant separation check, such as Albert–Anderson geometry or fitted probabilities diverging to 0 or 1. The penalty should apply only after separation is confirmed, and the result should show non-convergence.

#### 85. Medium · Hosmer–Lemeshow depends on row order and splits tied fitted probabilities

**Reproduced.** Rows are stably sorted by fitted probability alone and cut into equal-count bins, so tied probabilities can fall into different bins depending on input order.

- Evidence from `crates/stats/src/regression.rs:747–775`.
- A saturated binary-predictor model, with fitted probabilities exactly 0.25 and 0.75, gave χ² 32.0 on 8 df with P 9.3e-5 when events came first within each group. The same data interleaved gave χ² 0 and P 1. The correct statistic is 0.
- Regression target is keeping tied probabilities in one bin, independent of row order, and reporting the test as unavailable when fewer than three groups remain.

#### 86. Medium · An exact linear fit reports P = 1 for every coefficient while the model F has P = 0

**Reproduced.** With zero residual variance every coefficient has SE 0, so t is set to 0 and P to 1, while the model F is infinite with P 0. The infinite F is dropped from the sheet.

- Evidence from `crates/stats/src/regression.rs:289–297,366–370`.
- y = 1 + 2x for x = 1 through 6 gave SE 0, t 0, P 1 ("ns"), and CI (2, 2) for the slope, both directly and through the GLM sheet.
- Regression target is marking t, P, and the interval as not estimable when SE and RSS are 0. This is the same pattern as findings 18, 20, and 39.

#### 87. Medium · Rubin pooling reports t = 0 and P = 1 when the total variance is zero

**Reproduced.** When both the between- and within-imputation variances are zero, pooling returns SE 0, t 0, and P 1.

- Evidence from `crates/stats/src/multivariate.rs:1783–1801`.
- `rubin_pool_estimates` with estimates [5, 5, 5], variances [0, 0, 0], and complete-data df 9 returned estimate 5, P 1, and CI (5, 5). A constant column with two imputed cells gave the same pooled mean row through multiple imputation. The pooling path with between-imputation variance matched the Barnard–Rubin formulas exactly.
- Regression target is not-estimable t, P, and interval for zero total variance.

#### 88. Medium · Parts-of-whole goodness of fit treats non-count values as counts

**Reproduced.** The chi-square and G² goodness-of-fit tests use raw slice values as counts without an integer check. The exact binomial path does check (`crates/stats/src/categorical.rs:2701–2707`).

- Evidence from `crates/stats/src/categorical.rs:2866–2925`.
- The same proportions in different units gave χ² 0.14 with P 0.93 for [0.5, 0.3, 0.2], χ² 14.0 with P 9.1e-4 for [50, 30, 20], and χ² 140 with P 4e-31 for [500, 300, 200].
- Regression target is a typed error for non-integer slice values. Distinct from `R2-S02-12` and `R2-S10-03`.

#### 89. Medium · Parts-of-whole tests ignore the selected data set

**Reproduced.** Goodness of fit and the exact binomial test always analyze series 0, and `selected_data_sets` is never read for parts-of-whole sheets.

- Evidence from `crates/stats/src/categorical.rs:2697,2872`.
- With series 0 = [50, 30, 20] and series 1 = [34, 33, 33], every selection gave χ² 14.0 and P 9.1e-4. Series 1 should give χ² 0.02 and P 0.99. The exact binomial test on series 1 = [5, 5] reported k = 9 from series 0, with P 0.0215 instead of 1.
- Regression target is analyzing the selected series or rejecting a multi-series selection. This is the same pattern as finding 13.

#### 90. Medium · The Fleiss kappa interval uses the null-hypothesis variance

**Reproduced with a Monte Carlo coverage check.** The Fleiss, Nee, and Landis (1979) variance holds only when κ = 0, but it is shown as the standard error and used for κ ± z·SE.

- Evidence from `crates/stats/src/categorical.rs:2223–2230,2569–2583`.
- A 30-subject, 4-rater, 3-category table gave κ 0.583, SE 0.0721, and CI (0.442, 0.725), which is exactly the null formula. In 3,000 simulated replicates with population κ 0.487, the empirical SD of κ̂ was 0.0853 against a mean null SE of 0.0573. Coverage at a nominal 95% was 79.5%.
- Regression target is a non-null standard error for the interval and the displayed SE, keeping the null SE for the z test only. Cohen's kappa variances matched `psych::cohen.kappa`.

#### 91. Medium · Single-variable multiple imputation draws only from values near the mean

**Reproduced.** With one variable, donors are the 2 to 5 observed values closest to a drawn mean rather than a predictive-mean-matching donor pool. An intercept-only model gives every observed case the same predicted mean, so every observed value should be a donor (van Buuren 2018, §3.4). Adding an unrelated, fully observed column switches to that behavior.

- Evidence from `crates/stats/src/multivariate.rs:1886–1888,1972–1991,2014–2033`.
- Forty draws from N(10, 4) with about half missing had observed values from 5.0 to 16.5, but every value imputed in the first dataset came from five central values between 11.0 and 12.7. Adding an unrelated, fully observed column spread the imputations over 8.4 to 16.5. Over 400 replicates with m = 5, the mean within-imputation variance was 0.054 against σ²/n = 0.10. The pooled standard error averaged 0.339 against 0.403, and 95% interval coverage was 88.2%.
- Regression target is the intercept-only predictive-mean-matching donor pool, or a Bayesian bootstrap, for the one-variable path.

#### 92. Medium · k-means reports empty clusters, and automatic k picks the largest k on ties

**Reproduced.** When the seeding distances total zero, seeding can pick an existing point again. An empty cluster keeps a stale centroid and is reported with size 0. The automatic sweep uses `max_by` on mean silhouette, which returns the last of several tied maxima.

- Evidence from `crates/stats/src/multivariate.rs:891–908,931–933,1525–1531,1554–1559`.
- Four points at (0, 0) and four at (5, 5) gave automatic k = 6 with cluster sizes [4, 4, 0, 0, 0, 0], because every k from 2 to 6 had silhouette 1.0. Requesting k = 3 for two distinct points produced an empty cluster with centroid (0, 0). R `kmeans` refuses more centers than distinct points.
- Regression target is rejecting or reducing k when there are fewer distinct points than k, re-seeding empty clusters, and breaking ties toward the smaller k. Distinct from `R2-S02-16`.

### Wave 5 verification and remaining coverage

- Native macOS arm64, Rust `1.99.1`, lockfile unchanged, offline resolution. Areas 1, 2, 3, and 5 were audited in worktrees at `033d4be`, and area 4 at `ce80964`. Each finding was reproduced by a temporary probe through public APIs, `Session::execute`, or the CLI binary, and every probe was removed. Before recording, every cited code path was re-checked at `ce80964`, and line references are for that commit. Reference values came from hand derivation, closed forms, cited published procedures, Monte Carlo coverage checks, and R 4.5.2 with `psych` and `cluster` as an external numerical oracle only.
- Existing tests passed:
  - `curvebook-stats`: every target, including `d1_reference_oracles`, `d1_edge_matrix`, `nist_strd`, `parity_g8_barnard_pb_mi`, `rv_ledger_stats_core`, `rv_ledger_stats_models`, and `wave21_r97_r912_clustering`.
  - `curvebook-engine`: library tests plus `dag_reactivity`, `undo_and_methods_qol`, `rv_ledger_sheet_table`, `cmd_data`, `audit_r1_s3b`, `audit_r2_s05_s07`, `rv_ledger_third_pass_engine`, `sci_qol_workflows`, and `rv_ledger_persist_engine`.
  - `curvebook-platform` and `curvebook-format`: every target.
  - `curvebook-cli`: eight audit and smoke targets.
- After the wave, every Wave 3–5 build directory under `/tmp/cb-audit-*` was deleted, as `AGENTS.md` requires. The five Wave 4 auditor worktrees and the Wave 5 worktrees under `.claude/worktrees/` have no changes and were left in place.
- Not covered in this wave:
  - Frequency distributions and MAD, which do not exist in the code. The internals of the normality tests.
  - Inline-hook Info fallback for graphs and layouts, `sheet.extractResultTab`, and the Multiple Variables role and recode commands.
  - Windows path and process behavior, symlink races, and real power-loss tests.
  - LaTeX and Markdown report renderers, contingency interval-method claims, and factorial and REML post-test report fidelity.
  - Boschloo and CSM Barnard orderings, Poisson regression numbers, the multiple-imputation sheet layout, and hazard-ratio internals.
- Still outside all waves: GUI and figure code, the layout module, and live GUI behavior. Remediation commits merged to `main` claim fixes for findings 1–16, and those fixes have not been re-verified here.

## Wave 6 coverage log

Baseline `ce80964` (`main` on 2026-11-19). Each auditor checks that its worktree is at this commit before starting. GUI and figure code remain out of scope, so the PNG and TIFF sizing fix for finding 9 is not re-verified. Each area uses its own worktree and a `/tmp/cb-audit-w6-*` build directory, and those directories are deleted when the wave lands.

| Priority | Area | Coverage and status |
|---|---|---|
| 1 | Remediation check for engine and format findings | Re-ran the original reproductions for findings 1, 2, 3, 5, 6, and 13 at `ce80964` across the engine, desktop host, MCP over stdio, and CLI entry points. Reviewed the unpacked-save transaction code in `crates/format/src/archive.rs`, the desktop journaling and dirty-state hooks, the DAG failure branches, and the version-check write path. Four findings are fixed and two are partly fixed; the verdicts are in the remediation status section. Findings 118–120 reproduced. Complete. |
| 2 | Remediation check for statistics and fitting findings | Re-ran the original reproductions for findings 4, 7, 8, 10–12, and 14–16 at `ce80964`, including every caller named in each regression target. Reviewed the merged identifiability, R², midpoint-warning, and linear range-control code, plus the remediation hunks in `crates/stats/src/{regression,survival,two_group,anova}.rs`. Six findings are fixed and three are partly fixed; the verdicts are in the remediation status section below. New findings 115–117 reproduced, and 115 is a regression from the fix for finding 11. Complete. |
| 3 | Document model, links, and Info | Reviewed in full `crates/doc/src/{info,ids,cell,links,project}.rs`, plus the XY, Column, summary, and Multiple Variables role, recode, and level-order code in `archetypes.rs`. Also reviewed `cmd/info.rs`, the Multiple Variables role, recode, categorical-order, and calculated-variable commands, Info upserts, paste-link, `sheet.extractResultTab`, DAG link refresh, hook, Info, and power-Info resolution, `HookContext::resolve`, and the transform hook and formula DAG. Summary-mode reconstruction from SEM, %CV, and limits was correct. `crates/doc/src/lib.rs` beyond its tests and the Grouped, Contingency, Survival, Parts of Whole, and Nested accessors were not covered. Findings 93–99 reproduced, plus one out-of-scope chart note. Complete. |
| 4 | Report renderers and remaining report fidelity | Reviewed `crates/engine/src/methods_report.rs` in full, including its text, Markdown, and LaTeX renderers and every `describe_*` function. Also reviewed JSON and CSV export, `result.generateReport`, `crates/format/src/export.rs`, factorial multiplicity, simple effects, the Dunnett and studentized-maximum-modulus contrast paths, repeated-measures and REML selection, the contingency interval methods, the power runner with Info hooks, the multiple-imputation clamps, and the fit weighting and parameter rules. Generated LaTeX was compiled with pdflatex, lualatex, and xelatex, and generated Markdown was rendered with pandoc CommonMark and GFM. No HTML renderer exists. The custom-equation and exclusion-count parts of `R2-S07-12` appear fixed, and its `[`-row fix caused finding 105. Findings 105–114 reproduced. Complete. |
| 5 | Remaining statistical numerics | Reviewed Poisson regression and its deviance, Pearson dispersion, rate-ratio intervals, and likelihood-ratio test; Barnard's score, Boschloo, and CSM orderings; the observed-over-expected, Peto, and Mantel–Haenszel hazard ratios; Bland–Altman raw and percent modes; Passing–Bablok; and Shapiro–Wilk, D'Agostino, Anderson–Darling, and Lilliefors at small and large n in both the statistics and fit copies. The Dobson Poisson example, the Barnard orderings on 14 tables, the hazard ratios, Bland–Altman, Passing–Bablok estimates, Shapiro–Wilk up to n = 4500, D'Agostino, and Lilliefors matched R or independent implementations. Deming regression and the engine sheet paths for findings 101–104 were not covered. Findings 100–104 reproduced. Complete. |

### Wave 6 findings

#### 93. High · Adding a calculated variable changes which variable an existing Multiple Variables analysis uses

**Reproduced.** When a request does not name the response or Y column, the choice is not saved in the specification and is made again on every recompute. The t-test prefers the last calculated column and then the last numeric one, and the regression takes the last numeric column as Y. Adding a calculated variable therefore silently retargets existing results.

- Evidence from `crates/stats/src/two_group.rs:2105–2121,2137–2155`, `crates/fit/src/types.rs:544–557`, and `crates/engine/src/cmd/analysis.rs:1662–1663`.
- An unpaired t-test of `Weight` [10, 12, 11 | 15, 17, 16] by `Group` gave t = −6.124. Adding `LogWeight = log10([Weight])` changed t to −5.890, and the group means became the log10 means 1.040 and 1.204, under the same headings. The result stayed up to date without a warning. A linear regression of `Resp` ≈ 2·Dose + 1 on `Dose` gave slope 2.0. Adding `DoseSq = [Dose]*[Dose]` changed it to slope 6.0 and intercept −7.0, which is DoseSq regressed on Dose.
- Regression target is saving the resolved columns when the analysis is created, or marking results stale when the resolution changes. Related to finding 66 and `R2-S02-07` through a different path.

#### 94. Medium · Column-title hooks in fits read hyphens as minus signs and digits inside names as values

**Reproduced through `HookContext::resolve`.** The fit's leading-number parser takes the first numeric token in a column title. The transform path for the same hook requires the whole title to be a number, so the two paths disagree.

- Evidence from `crates/fit/src/types.rs:205–245` and `crates/transform/src/ops/mod.rs:80–96`.
- `Inhibitor-10 nM` resolved to −10, `IL-6 10 ng/mL` to −6, and `CO2 5%` to 2. The transform path rejected all three, but also rejected `[I] = 10 nM`, which the fit resolves to 10.
- Regression target is one strict shared title parser with a typed error.

#### 95. Medium · A second randomization or row sample overwrites the first one's provenance in a shared Info sheet

**Reproduced.** `upsert_info_constants` falls back to the document's first Info sheet when the output has no paired sheet. A later randomization or row sample therefore overwrites the earlier one's recorded seed and settings.

- Evidence from `crates/engine/src/cmd/table.rs:4777–4810`, with callers at `:4958` and `:5064`.
- A first randomization with seed 111, four subjects, and 1:1 allocation created `info_1`. A second, with seed 222, six subjects, and 2:1 allocation, was paired with `info_1` and replaced its seed, ratio, and subject count, so the first cohort's record was lost. Two row samples from the same source left only the second seed and size.
- Regression target is a separate Info sheet per output, or namespaced keys, and never the first sheet as a fallback.

#### 96. Medium · Recoding a level silently changes the GLM reference level

**Reproduced for GLM. The Cox path shares the fallback and was traced.** `table.recodeLevels` updates the level order and graphs but not the reference levels saved in GLM or Cox specifications. A reference level that no longer exists falls back to the first level without a warning.

- Evidence from `crates/engine/src/cmd/table.rs:3648–3685`, `crates/stats/src/regression.rs:1112–1118`, and `crates/stats/src/survival.rs:2232–2237`.
- A GLM with `ctrl` as reference gave intercept 2, drugA versus ctrl 4, and drugB versus ctrl 8. After `ctrl` was recoded to `Control`, the saved reference stayed `ctrl`. The up-to-date result switched to drugA as the reference, with intercept 6 and contrasts −4 and 4.
- Regression target is remapping saved reference levels on recode and returning an error when a reference level is missing. Related to `RV-56` and `R2-S07-04`.

#### 97. Medium · Extracted result tabs keep stale labels while their linked values shift

**Reproduced.** Tab headers and labels are applied only when a tab is extracted, and the link refresh updates values only. The extracted link's column range is also fixed at extraction, so columns added to the tab later are dropped; that was traced and not run.

- Evidence from `crates/engine/src/cmd/sheet.rs:1998–2026,2077` and `crates/engine/src/dag.rs:782–828`.
- After a normal Q–Q tab was extracted, one cell was excluded and the groups were renamed. The row titled "A #3" then held Low #1, "B #3" held High #1, and "C #3" was blank. An extracted multiple-comparisons tab kept rows titled "A vs. B" while holding the "Vehicle vs. Low" values.
- Regression target is re-applying tab headers and labels on every refresh, with an open column bound. `R2-S07-11` covered the extraction step only.

#### 98. Medium · Paste-link accepts overlapping targets and duplicate link IDs

**Reproduced.** `table.pasteLink` does not check whether the target overlaps an existing live link, unlike paste and import, and does not check that `link_id` is unique. The interface looks up regions by ID and uses the first match, which was traced.

- Evidence from `crates/engine/src/cmd/table.rs:2528,2781–2801,3023` and `crates/ui-egui/src/views/grid_core.rs:747,756,2305`.
- A target column already linked from `src1` accepted a second link from `src3` with a duplicate ID. After `src1` changed, the target still showed `src3`'s values inside `src1`'s link region, and `src1`'s link never updated.
- Regression target is a conflict error for overlapping targets and duplicate IDs.

#### 99. Medium · A standalone power analysis reads α and N from the source table's Info sheet

**Reproduced.** When `PowerAnalysisSpec.info_sheet_id` is unset, which is documented as standalone, recomputation falls back to the source table's Info sheet.

- Evidence from `crates/engine/src/dag.rs:1050–1105` and `crates/doc/src/analysis.rs:1423`.
- A power analysis with α 0.05, power 0.8, and effect size 0.5 gave 64 per group. After `alpha = 0.01` was set as an Info constant on the source table, the result became 96 per group, the answer for α 0.01, while the specification still showed α 0.05 and no Info sheet.
- Regression target is reading Info constants only through an explicit `info_sheet_id`.

Out-of-scope note, not numbered: graph-title `{{info:…}}` hooks fall back to the document's first Info sheet when the graph's own table is no longer paired with one (`crates/chart/src/{xy,survival_cat,scales}.rs`). After `info-b` was deleted, a graph on table b rendered table a's value "Drug A". This is chart rendering code, which is outside this audit's scope.

Code-trace notes from area 3, not numbered:

- Excluding only the SD or N cell of a summary row makes the whole row an error instead of skipping it.
- Info constant names may contain `:` or `}}`, so no hook can address them, the constant-name form of `RV-89`. Date constants accept any string.
- `setVariableRole` with automatic role on an empty column keeps the old role while marking it inferred.
- `recodeLevels` trims the source level when rewriting the order but not when matching cells.
- Hooks in extra tick labels are drawn but are not in `GraphSpec::inline_hooks`, so dependency tracking, validation, and clone remapping skip them.
- One-way ANOVA on a Multiple Variables table with one categorical and one numeric column and no selection failed with "input dataset contains no valid observations". This was not investigated.

#### 100. High · The Anderson–Darling P value rises back to 1 for grossly non-normal data

**Reproduced.** For A* ≥ 0.6, P is computed as exp(1.2937 − 5.709A* + 0.0186A*²) with no upper bound on A*. The exponent is smallest at A* ≈ 153.5, so P then increases, passes 0.05 near A* = 306, and is clamped to 1 near 307. The statistics and fit copies are identical.

- Evidence from `crates/stats/src/column.rs:1015–1024` and `crates/fit/src/diagnostics.rs:1468–1477`.
- 999 zeros and a single 1 gave A* 385.8, P 1.0, and a passed test in both copies. The same data gave Shapiro–Wilk W 0.012 with P < 2e-16 and Lilliefors P 0. A lognormal sample with σ = 2 and n = 3000 gave A* 734 and P 1.0. Exponential samples showed P rising as A* rose, from 6.7e-165 at A* 97 to 4.9e-140 at A* 232. Under the null at n = 1000, A* never exceeded 1.61 in 2,000 simulations.
- Regression target is P non-increasing in A*, with a floor or an out-of-range report beyond the approximation's domain, and never P = 1 for large A*.

#### 101. Medium · Poisson regression gives wrong coefficients when a fitted mean exceeds about 7×10¹⁰

**Reproduced. The auditor proposed High; it is rated Medium here because it requires counts near 10¹¹.** The linear predictor is clamped to [−25, 25], so fitted means above e²⁵ are capped and the fit converges to the wrong answer. The stored convergence flag is false, but finding 84 shows that flag is never displayed.

- Evidence from `crates/stats/src/regression.rs:865–868`.
- For x = 0–3 and y = 1e11, 2.1e11, 3.9e11, and 8e11, the fit gave intercept 25.000, slope 9.646, likelihood-ratio χ² 0 with P 1, and deviance 3.26e12. R `glm` gives 25.346, 0.6854, and deviance 6.67e8. Exact doubling from 1e11 gave slope 9.036 instead of ln 2 = 0.693. The same pattern at the 1e9 scale matched R.
- Regression target is no clamp on the linear predictor, or a typed error or visible non-convergence when the bound is reached.

#### 102. Medium · The Passing–Bablok CUSUM linearity test almost never rejects

**Reproduced by simulating the shipped statistic. The published normalization was recalled by the auditor and not checked against the paper.** The statistic is H/√(positive + negative residual counts). With the shipped score, ordering, and normalization, the null rejection rate at nominal 5% was 0% at n = 30 and 0.2% at n = 100. Normalizing by √(L + 1), with L the count of one residual sign, gave null rates of 3.6–7.9%.

- Evidence from `crates/stats/src/regression.rs:1360–1363` and `crates/engine/src/methods_report.rs:502`.
- With x = 1–40 and y = x + 0.004(x − 20.5)² + 0.5 sin(5x), the shipped test gave H 7, P 0.173, and linearity passed, while the √(L + 1) form gave P 0.019. At n = 40 with mild curvature, power was 25% against 86%. The methods text also says the test is against the identity line, but it tests the fitted line; that was traced.
- Regression target is the published normalization, confirmed by a null rejection rate near nominal.

#### 103. Medium · Poisson regression accepts non-integer responses, so its P values depend on units

**Reproduced.** Responses are checked only for being finite and non-negative, although the model is documented for counts. There is no offset or exposure option for rates.

- Evidence from `crates/stats/src/regression.rs:193,836–840` and `crates/doc/src/analysis.rs:838–849`.
- Rates of 0.12 to 0.36 over x = 0–9 gave slope P 0.615. The same rates per 100 gave P 4.9e-7. R gives the same numbers but warns about non-integer values.
- Regression target is a typed error for non-integer responses, or offset and exposure support for rates. Same pattern as finding 88.

#### 104. Medium · Bland–Altman percent mode reverses the bias direction when means are negative

**Reproduced.** Percent differences are rejected only when the pair average is exactly 0. They are undefined unless both measurements are positive.

- Evidence from `crates/stats/src/two_group.rs:1443–1452`.
- A = [−3, −5, −2, −4] and B = [−2, −4.5, −1, −3.6] had a raw bias of −0.725, with A reading lower. The percent bias was +31.9%, with limits (−21.0, 84.9). A pair of −1 and 1.5 gave −1000%.
- Regression target is rejecting non-positive averages in percent mode, as the lognormal battery already does.

Lower-severity notes from area 5, not numbered:

- **Boschloo tails:** absolute 1e-12 tolerances inflate P values below about 1e-11 (`crates/stats/src/categorical.rs:765,777`). For example, [[40,0],[0,40]] gives 3.0e-13 instead of 1.65e-24.
- **Unreachable orderings:** the contingency sheet always uses the score ordering, so Boschloo and CSM are library-only (`categorical.rs:2265`; traced).
- **Documentation mismatches:** `BarnardExactResult::statistic` stores the pooled z, not −p_fisher for Boschloo. The normality-battery documentation says Anderson–Darling needs n ≥ 5, but the code needs 8.
- **Hazard-ratio sheet:** log-rank, Mantel–Haenszel, and Peto hazard ratios appear without intervals or the numerator cohort (`crates/stats/src/survival.rs:1895–1904`; traced). The values are correct.
- **All-zero Poisson category:** returns a misleading `SingularMatrix` error on a full-rank design.
- **D'Agostino:** simulated null size is about 0.056–0.060 at n = 8–30, a mild excess.

#### 105. High · Every LaTeX report with a result table fails to compile

**Reproduced with pdflatex and lualatex.** Each table row ends with `\\\relax`, and `\hline` follows it. The `\relax` begins a new row, so the next `\hline` raises `! Misplaced \noalign`. This regression came from the `R2-S07-12` fix for rows that begin with `[`. The existing test only checks that the string is present and never compiles the output.

- Evidence from `crates/engine/src/methods_report.rs:1820–1830` and `crates/engine/tests/audit_r2_s05_s07.rs:305`.
- A plain one-way ANOVA report with three groups and Tukey failed at the first `\hline` after the header. Replacing the row ending with `\\` made the same file compile.
- Regression target is a test that compiles generated LaTeX, with plain `\\` before `\hline` and `{[}` to guard a leading bracket.

#### 106. Medium · Markdown reports escape nothing outside tables

**Reproduced with pandoc.** Text outside tables is emitted raw, and titles are checked for length only, so they can contain newlines and markup. The same titles also break LaTeX headings.

- Evidence from `crates/engine/src/methods_report.rs:1585–1620,1766–1797` and `crates/doc/src/error.rs:41–46`.
- A result sheet renamed to a title containing blank lines, a `<script>` element, and a `# Injected heading` line, and a project title containing an `<img onerror>` element and `*star*`, rendered as a raw script element, a raw image handler, an extra level-1 heading, and italics. In LaTeX, the blank lines inside `\section*{…}` caused a paragraph-ended error.
- Regression target is escaping Markdown and HTML specials and collapsing newlines outside tables, and using the single-line LaTeX escaper for titles.

#### 107. Medium · Bold labels never render in Markdown reports

**Reproduced with pandoc CommonMark and GFM.** Bold runs end with a space, as in `**Project: **`, and a closing `**` after whitespace cannot close emphasis in CommonMark.

- Evidence from `crates/engine/src/methods_report.rs:143–160,1213,1325`.
- A clean report rendered 19 lines with literal asterisks, covering every project, sheet, procedure, scalar, and checklist label.
- Regression target is placing the space outside the bold markers.

#### 108. Medium · LaTeX reports contain a raw ∞ that pdflatex rejects

**Reproduced.** Contingency and column-statistics text can contain ∞, and the report's Unicode-to-LaTeX map has no entry for it.

- Evidence from `crates/stats/src/categorical.rs:1161,1999,2011–2013,2456`, `crates/stats/src/column.rs:2226`, and `crates/engine/src/methods_report.rs:1647–1672`.
- Fisher's exact test with ratios on [[10,12],[11,10]] produced "NNTH 2.97 to ∞ to NNTB 4.70". With finding 105 patched, pdflatex failed on U+221E, and xelatex silently dropped the character. This happens for every 2×2 table whose risk-difference interval crosses zero.
- Regression target is mapping ∞ to `$\infty$`.

#### 109. Medium · The methods text for two-way repeated-measures ANOVA names the wrong estimator

**Reproduced.** The report describes REML whenever `fit_reml_on_missing` is true, which is the default, regardless of the path computed. The code also falls through to REML when data are missing even if the flag is false.

- Evidence from `crates/engine/src/methods_report.rs:431–441`, `crates/stats/src/anova.rs:7057–7075`, and `crates/doc/src/analysis.rs:669`.
- Complete 2×3 data with four matched subjects were analyzed by classical repeated-measures ANOVA with a sphericity tab and no REML output, but the report described a compound-symmetry REML fit with Kenward–Roger df. With the flag false and one missing cell, the computation produced a REML tab with Satterthwaite and Kenward–Roger df, and the report contained no REML sentence.
- Regression target is describing the path actually taken, and honoring or removing the flag.

#### 110. Medium · The factorial report claims omega-squared effect sizes that are never computed

**Reproduced.** The factorial table has only partial eta squared and eta squared columns, and eta squared is empty on the REML path. The report says partial eta-squared and omega-squared were calculated, and the specification documentation also promises generalized eta squared and partial omega squared.

- Evidence from `crates/engine/src/methods_report.rs:446–447` and `crates/stats/src/anova.rs:5292–5311`.
- Regression target is describing only the effect sizes actually emitted. This is the same class as item 1 of `R2-S07-03` for factorial ANOVA.

#### 111. Medium · The report claims multiplicity-adjusted post-test intervals that are absent or unadjusted

**Reproduced.** Whenever intervals are requested, which is the default, the report says multiplicity-adjusted confidence intervals were produced, whatever the method.

- Evidence from `crates/engine/src/methods_report.rs:1105–1108`, `crates/stats/src/multiplicity.rs:609–616`, and `crates/stats/src/anova.rs:6181–6186`.
- For a one-way ANOVA with three groups of five, the interval column was empty for Holm–Šidák and Benjamini–Hochberg. Fisher's LSD gave the unadjusted interval [−4.553, 0.153], which is −2.2 ± t(0.975, 12) × 1.0801, against Tukey's [−5.082, 0.682].
- Regression target is describing intervals only when they are computed and calling LSD intervals unadjusted.

#### 112. Medium · The power report shows the stored settings rather than the Info values used

**Reproduced.** Recomputation patches α and target power from Info hooks into a temporary copy, and the report reads the unpatched specification.

- Evidence from `crates/engine/src/methods_report.rs:238,1840–1931` and `crates/engine/src/dag.rs:1018,1047–1104`.
- With Info constants α 0.01 and power 0.95 and a stored specification of 0.05 and 0.80 with d = 0.5, the computation gave 145 per group with achieved power 0.951. The report text and header said α 0.05 and power 0.8, while the tables in the same report showed 0.01 and 0.95.
- Regression target is reporting the values used. Related to `R2-S07-10` and `RV-86`.

#### 113. Medium · The multiple-imputation report prints requested settings, not the clamped values used

**Reproduced.** The imputation code clamps m to 2–50 and the iteration count to 1–100, and the engine rejects only m = 0. The report prints the requested values.

- Evidence from `crates/engine/src/methods_report.rs:749–759`, `crates/stats/src/multivariate.rs:1893–1894`, and `crates/engine/src/cmd/analysis.rs:2159`.
- A request for m = 1 with zero iterations was reported as such but ran with m = 2 and one iteration, which the result checklist itself showed. A request for 200 and 500 ran with 50 and 100.
- Regression target is rejecting out-of-range values or reporting the values used. Related to finding 91.

#### 114. Medium · The nonlinear-fit report lists parameter rules the solver ignored

**Reproduced.** Parameter rules are matched case-insensitively with the first match winning, and rule names are never checked against the model. The report lists every rule as given.

- Evidence from `crates/engine/src/methods_report.rs:854–861`, `crates/fit/src/solver.rs:634–636`, and `crates/engine/src/cmd/analysis.rs:2915–2936`.
- A four-parameter logistic fit with rules `EC50` constant 1e-7, `HillSlope` constant 1, and `HillSlope` constant 2 reported all three. LogEC50 was fitted freely to −7.0033, so EC50 was 9.92e-8, and HillSlope was fixed at 1. The `EC50` rule and the second HillSlope rule had no effect.
- Regression target is rejecting unknown or duplicate rule names, or reporting only the rules applied.

Notes from area 4, not numbered:

- The LaTeX fragment uses `\text{…}`, which needs amsmath, but it does not declare that requirement (traced).
- Simple effects in two-level factorial designs are adjusted with the studentized maximum modulus across slices when Tukey, Dunnett, or Games–Howell is requested, but the report says Tukey's HSD and the footnote does not name the procedure. Raw P 0.269 became 0.571 with m = 3. The SMM identity was traced and not checked against an external oracle.
- `analysis.exportCsv` on a failed result succeeds with only a header row, extending finding 71. `result.generateReport` shows the failed status but still describes the methods as performed. `analysis.exportJson` includes the status.

### Remediation status at `ce80964`

The original reproductions were re-run at `ce80964` against the remediation commits. Finding 9 is figure work and was not re-checked.

| Finding | Verdict | Evidence |
|---|---|---|
| 1 | Fixed | Engine `project.save` and `project.saveTemplate` returned `UnsupportedVersion` and left the bytes unchanged for newer-version archives, templates, cross-extension targets, targets with no extension, and newer unpacked directories. So did desktop Save As, Save Template, unpacked Save As, and MCP `save_file` over stdio. The CLI convert and export survivor is still open as finding 70 (`apps/curvebook-cli/src/cmd_convert.rs:479`). Engine saves now also refuse to replace an existing file that is damaged or not a zip, so a corrupted project cannot be saved over. |
| 2 | Partly fixed | The original undo-then-create batch now recovers the live document. GUI actions inside a mixed batch remain unrecorded, recorded as finding 118. |
| 3 | Fixed for process termination | A hand-built interrupted commit, with the commit marker, both snapshots, and only the new `document.json` published, reopened as the new generation. A later save reconciled the folder and removed the transaction directory, and a crash before the marker reopened the prior generation. A failed commit followed by a failed rollback still breaks the project, recorded as finding 119. Root directory sync before the first rename is missing, which matters only for power loss (traced). |
| 5 | Fixed | A GUI edit at undo depth 64 persists through recovery. The new mutation counter does not depend on undo length, so eviction by cell or byte budget is also covered (traced). |
| 6 | Fixed | A note survived a failed recompute, a save and reopen while failed, and recovery to up to date. Every failure branch uses the preserving helper. |
| 13 | Partly fixed | An invalid predictor now returns `BadInput`, and the selected data set is analyzed. Only the first replicate subcolumn is used, recorded as finding 120. |
| 4 | Partly fixed | Invalid explicit columns now return `BadInput`, and the default still gives bias 3. However, the same column pair means X versus Y0 when the table has one Y data set and Y0 versus Y1 when it has two, and the check runs only when the specification is built, updated, or retargeted. A default Bland–Altman on a one-data-set XY table was followed by a paste that added a second Y data set. The result recomputed as up to date with bias −190 instead of 3 and no warning (`crates/stats/src/two_group.rs:2269–2272,2307–2316`, `crates/engine/src/cmd/analysis.rs:842,894,1097,1162–1200`). The X-versus-Y mode is still not represented explicitly. |
| 7 | Partly fixed | Column and Multiple Variables tables now reject odd paired selections, although they report `solver_failed` rather than `bad_input`. Paired row tests on a Grouped table with data sets [0, 1, 2] selected still silently drop data set 2 and give the same result as [0, 1]. The XY branch has the same pattern, which was traced (`crates/stats/src/two_group.rs:2345–2348,2371`). This overlaps finding 32. |
| 8 | Fixed | Paired t, ratio-paired t, row-wise paired t, and one-sample t at n = 2 through the raw, summary, and descriptive-tab callers all return the primary test. For example, t = 2 with P = 0.2952 matches the closed form, and the Hedges' g cell is empty. |
| 10 | Fixed | Upper/lower limits, low/high, and median/IQR modes use central values only, giving intercept 10 and n = 4. Moment modes expand by N. |
| 11 | Fixed | Infeasible Poisson means return `ImpossibleWeights`, including bounded and shared cases. The fix introduced finding 115. |
| 12 | Fixed | Equal bounds match an explicit constant in free-parameter count and df under every sharing mode. |
| 14 | Partly fixed | The evaluator now decides argument order from formula syntax, and `sumif(Reading, Flag)` returns 1. The interface formula hints still document the reverse order, `sumif(cond, var)` and `meanif(cond, var)` (`crates/ui-egui/src/grid_view.rs:466–467`, `crates/ui-egui/src/views/mv_formula.rs:357–369`). A user following the hint gets 2, and `r63_window_and_conditional.rs:567` asserts that value. |
| 15 | Fixed | GLM residual and prediction rows keep source row numbers for linear, logistic, and Poisson fits on XY and Multiple Variables tables. Passing–Bablok residuals have the same defect and are recorded as finding 117. |
| 16 | Fixed | With no comparable pairs, the concordance is absent and the sheet shows zero comparable pairs. |

#### 115. Medium · Poisson fits with an infeasible starting point now fail with a misleading error

**Reproduced. This is a regression from the fix for finding 11, and the pre-fix commit `a5fdb7f` was checked.** Every trial point with a non-positive mean now scores as infinite, so neither the Nelder–Mead rescue nor the least-squares pre-fit can leave an infeasible start. Built-in equations with data-driven starts are unaffected.

- Evidence from `crates/fit/src/solver.rs:1240–1262,1769–1790,1888–1907`.
- The custom model `Y = A + B*X` with Poisson deviance, X = −1 to −10, and Y = 50 + 3X returned `ImpossibleWeights` ("requires strictly positive fitted means") from the default start, from A = B = 1, and from A = 0, B = 1. The exact solution A = 50, B = 3 has every fitted mean between 20 and 47. At `a5fdb7f`, all three starts converged to it.
- Regression target is reaching a feasible optimum from an infeasible start, reserving `ImpossibleWeights` for models with no admissible solution and reporting infeasible starting values otherwise.

#### 116. Medium · Unstable-parameter identification flags identifiable parameters when the Jacobian is rank-deficient

**Reproduced.** In the rank-deficient branch, covariance diagnostics set every dependency to the sentinel 1.0. The identifiability check then reports every parameter as having variance inflation above 10,000, which undoes its targeted weak-slot detection.

- Evidence from `crates/fit/src/solver/identifiability.rs:58–71` and `crates/fit/src/diagnostics.rs:379–382`.
- For x = 0.1 + 0.5i over 12 points and y ≈ 1 + 2x, the custom model `Y = A*X + B + C*0` flagged A, B, and C as unstable. Only C has zero sensitivity. A and B are the ordinary slope and intercept, fitted as 1.9997 and 1.0025, with an uncentred dependency of (Σx)²/(nΣx²) = 0.732.
- Regression target is taking variance-inflation evidence only for the weak slots in the rank-deficient branch, or computing dependencies from the identifiable subspace.

#### 117. Medium · Passing–Bablok residuals use compacted row labels

**Reproduced.** This is the same defect fixed for GLM tables in finding 15. Passing–Bablok residual rows are labelled from 1 after exclusions.

- Evidence from `crates/stats/src/regression.rs:1617–1620`.
- With source row 2 excluded from a six-row paired table, the residual tab was labelled Row 1 through Row 5, and its second row held source row 3.
- Regression target is carrying source indices through, as the GLM fix does.

Trace-only note: the midpoint-outside-range warning compares the unshifted LogEC50 for the Gaddum/Schild and allosteric-shift models, so a shifted data set can be flagged.

#### 118. High · GUI actions in a mixed control batch are lost on recovery and can leave a changed window marked clean

**Reproduced. This is the surviving path of finding 2 and was not introduced by the fix.** GUI-action methods in a control batch return before a recovery step is recorded. When no step in the batch needs a checkpoint, only recorded steps reach the journal. The save hook marks the window clean based on recorded steps alone.

- Evidence from `crates/ui-egui/src/control_io.rs:1408–1410` and `apps/curvebook/src/desktop_host.rs:709–740,858–888`.
- A non-atomic batch of `ui.invoke_menu` creating `g1` followed by `sheet.createData` creating `t2` left the live document with t1, g1, and t2. Recovery after an unclean shutdown produced only t1 and t2. A batch of `project.save` followed by a GUI action creating `g1` left g1 in the live document and 0 sheets in the saved file, while the window reported no unsaved changes, so closing it would not prompt.
- Regression target is recording GUI-action mutations as steps or checkpointing whenever the mutation counter moved beyond the recorded steps, with dirty state reflecting unrecorded changes after a save.

#### 119. Medium · A failed unpacked commit whose rollback also fails leaves the project unreadable

**Reproduced with a temporary in-crate probe that was removed afterwards.** Rollback errors are discarded, and the transaction guard's `Drop` removes the commit marker and snapshots unconditionally. If a commit fails after `document.json` is published and the rollback write also fails, the recovery record is deleted.

- Evidence from `crates/format/src/archive.rs:386–392,1082–1090,1475,1483–1494`.
- A second save failed at `manifest.json` after `document.json` was renamed, and the root was read-only so the rollback rewrite failed. The marker and the next-generation snapshot were removed, reopening returned `CorruptCrc` (the end state of finding 3), and the save error mentioned only the commit failure. Saving again from memory would repair the folder. Realistic triggers are a rename failure combined with a rollback write failure from sharing violations, I/O errors, or a full disk.
- Regression target is keeping the commit record whenever rollback is incomplete, reporting the rollback failure, and reopening a complete generation.

#### 120. Medium · The XY GLM silently drops replicate subcolumns

**Reproduced. This is the surviving part of finding 13.** XY GLM extraction reads only subcolumn 0. Parameter validation rejects summarized tables but not tables with several raw replicates.

- Evidence from `crates/stats/src/regression.rs:1227–1236` and `crates/engine/src/cmd/analysis.rs:1203–1234`.
- An XY sheet with two raw replicates and two Y data sets over 5 rows, with replicate 2 of data set 1 set to 9000 + r, gave an up-to-date linear GLM with 5 residual rows and 3 residual df. The replicate-2 values were absent, with no footnote or warning.
- Regression target is using every replicate or returning a typed error.

### Wave 6 verification and remaining coverage

- Native macOS arm64, Rust `1.99.1`, lockfile unchanged, offline resolution. Every auditor confirmed its worktree was at `ce80964` and left it clean. Each finding was reproduced by a temporary probe through public APIs, `Session::execute`, the desktop host, MCP over stdio, or the CLI binary, and every probe was removed. One auditor patched `archive.rs` temporarily for an in-crate probe and restored it with a matching checksum. Another detached its worktree at `a5fdb7f` to confirm the regression in finding 115 and returned it to `ce80964`. LaTeX was compiled with pdflatex, lualatex, and xelatex, and Markdown was rendered with pandoc. R 4.5.2 was used as an external numerical oracle only, alongside closed forms, published worked examples, independent reference implementations of published definitions, and Monte Carlo checks.
- Existing tests passed:
  - `curvebook-format`: every target, including the new archive transaction tests and `audit_r2_s01_s06`.
  - `curvebook-stats`: every target.
  - `curvebook-fit`: library tests, `audit_wave2_regressions`, `d1_nist_and_pharma_oracles`, `r46_solver_oracles`, `r46_linear_oracles`, `r46_shapiro_rk45_oracles`, and `d1_edge_matrix`.
  - `curvebook-engine`: library tests, `dag_reactivity`, `s3a_x2_regressions`, `parity_wave3_workflows`, `undo_and_methods_qol`, `audit_r2_s05_s07`, `sci_qol_workflows`, `r45_robust_e2e`, `rv_ledger_third_pass_engine`, `rv_ledger_sheet_table`, `cmd_data`, `audit_r1_s3b_eng_links`, and `data_qol_workflows`.
  - `curvebook-doc`: every target.
  - `curvebook`: `recovery_wiring`, `desktop_host_loop`, and `r10_command_routing`.
  - `curvebook-ui-egui`: `r10_23_beta_desktop_gaps`.
- Every `/tmp/cb-audit-w6-*` build directory was deleted after the wave. The worktrees under `.claude/worktrees/` have no changes and were left in place.
- Not covered in this wave:
  - Baptista–Pike and Koopman root accuracy, REML LS-means post-test internals, and report text for GLM, ROC, survival, and clustering beyond findings 83 and 90.
  - Deming regression internals, CSM tables beyond the cell cap, Passing–Bablok wrap-around medians, and exact Bland–Altman limit intervals.
  - The remaining archetype accessors in `crates/doc`, layout renderers, and the interface effect of duplicate link IDs.
- Still outside all waves: GUI and figure code, including the re-check of the fix for finding 9, the layout module, and live GUI behavior.

## GitHub issues carried into this audit

On 2026-11-19 the six GitHub issues still open were copied here so that the audit is the single record of open defects. The issues were then closed with a link back to this file. Each entry summarizes the issue report, including its reproduction data, reference values, and suggested fixes. These findings were **not re-reproduced** during this audit unless the entry says so. The original issue pages remain readable on GitHub after closure.

### Carried issue findings

#### 121. High · Profile-likelihood CIs report finite upper bounds where they should be open (issue #11)

**Reported in [issue #11](https://github.com/HauserGroup/curvebook/issues/11), opened 2026-11-17, with root-cause analysis.** A `log_agonist_vs_response_3pl` fit (Hill slope fixed at 1, least squares, default 95 % profile-likelihood CIs) to nearly flat data reports finite upper bounds for LogEC50 and Top. Both should be `CiBound::Open`. The fit status says Converged and the identifiability check does not warn, so imprecise EC50 values look bounded. Partial agonists, inactive compounds, and controls are all affected.

- Input, data set A (X is log10 agonist in M, triplicate Y, nothing excluded):

  ```text
  X      Y1        Y2        Y3
  -13    0.158073  0.142182  0.125239
  -12    0.14263   0.128691  0.127492
  -11    0.148439  0.143951  0.124994
  -10    0.153396  0.130963  0.131028
  -9.5   0.166767  0.1237    0.147693
  -9     0.162657  0.130172  0.137912
  -8.5   0.159241  0.140491  0.13292
  -8     0.155937  0.138415  0.148216
  -7.5   0.168419  0.148875  0.143729
  -7     0.154269  0.149198  0.13891
  -6     0.163195  0.149607  0.134643
  -5     0.162081  0.162229  0.153057
  ```

- Input, data set B (same X values):

  ```text
  X      Y1        Y2        Y3
  -13    0.153568  0.158465  0.131933
  -12    0.130976  0.132838  0.137386
  -11    0.156624  0.136648  0.136798
  -10    0.145428  0.13714   0.145892
  -9.5   0.152816  0.132685  0.132433
  -9     0.166189  0.146827  0.144328
  -8.5   0.15708   0.141104  0.139714
  -8     0.153376  0.149128  0.141243
  -7.5   0.152285  0.136487  0.140378
  -7     0.161958  0.150063  0.140534
  -6     0.167567  0.14229   0.146858
  -5     0.158073  0.141105  0.152835
  ```

- CurveBook output through MCP `fit.run` (`curvebook-cli 0.1.0-alpha.1`, built from `2aa54b2`). Data set A gave DF 33 and RSS 0.0047845434228874, Top 0.15242 with CI [0.14491, 0.28610], and LogEC50 −8.49737 with CI [−11.45856, 9.35496]. Data set B gave Top CI [0.14443, 0.23295] and LogEC50 CI [−12.53959, 9.34957]. Both LogEC50 upper bounds land near +9.35, about 14 log units past the highest concentration. Joint fitting gives the same result.
- Reference is the profile-t endpoint of Bates & Watts (1988, §6.1), SS_crit = SS_min · (1 + t²/33) with t₀.₉₇₅,₃₃ = 2.034515. For A, SS_min = 0.0047845 and SS_crit = 0.0053847. The LogEC50 profile plateaus at 0.0052220 for every L from −2 to 50, and the Top profile plateaus at 0.0052220 up to T = 1000, so both upper bounds are open. For B, SS_crit = 0.0034889 and the LogEC50 plateau is 0.0033978, so the upper bound is open. The analytical limit as LogEC50 → +∞ with Top free is Bottom + c·10^X, which gives the same plateau values. The profile was computed as an exact two-column linear least-squares problem for each fixed L (Python 3, NumPy 2.5.4, `numpy.linalg.lstsq`), with the Top profile minimized over a LogEC50 grid from −14 to 4 in steps of 0.002. CurveBook's main solver agrees when the profiled parameter is a `Constant` rule, for example RSS 0.005217294867808649 at Top = 0.286, so the fault is in the profile path. Further citations are Venzon & Moolgavkar (1988, *Appl. Statist.* 37, 87–94) and Golub & Pereyra (1973, *SIAM J. Numer. Anal.* 10, 413–432).
- Fault A, the LogEC50 bound. `solve_trust_region_subproblem` (`crates/fit/src/solver.rs:1388` at `2aa54b2`) factors the unscaled Jacobian with `qr_col_pivoted`, with rank tolerance eps·max(n,k)·|R₀₀| ≈ 4.8e-14. The Top column has norm about 10^(−5−L), so |R₁₁| drops below the tolerance between L = 8.5 and 8.7, Top gets no Gauss–Newton step, and the nuisance fit stalls at the Bottom-only solution, SS = 0.0058987, above SS_crit. Brent in `refine_profile_bracket` converges onto that spurious jump. The 9.35 value depends only on max(X) and n. `compute_unscaled_cov` (`solver.rs:3574`) already column-equilibrates before its QR, but the trust-region subproblem does not. This also affects point fits whenever two Jacobian columns differ by about 14 orders of magnitude.
- Fault B, the Top bound. At Top = 0.286 the profiled LogEC50 must travel from −8.5 to about −4.1 along a very flat valley. `solve_profile_subproblem` → `run_reduced_lm` (`solver.rs:3618`) caps at 40 iterations, has no gain-ratio test, and returns on the first accepted step with a relative decrease of at most 1e-11, seeded by the warm start in `trace_profile_endpoint` (`crates/fit/src/diagnostics.rs:724`). The under-converged profile rises above SS_crit and gives a false crossing. Which exit is taken has not been confirmed with an in-crate test.
- Regression target: column-equilibrate J before the QR rank decision in `solve_trust_region_subproblem` (MINPACK scaling, Moré 1978); make `profile_solve` report convergence and treat a non-converged sub-solve or a non-monotone or discontinuous profile as `CiBound::Open`; re-verify a Brent root from the last non-interpolated warm start; optionally profile Bottom and Top for the 3PL and 4PL families exactly by variable projection. Add regression tests with both data sets expecting open upper bounds for LogEC50 and Top, and re-run `nist_strd` and the `r46_*` oracle suites, since the scaling change affects every fit. The issue proposed roadmap item R4.7 and branch `bugfix/R4.7-profile-ci-asymptote`.

#### 122. Medium · A partial `graph.setSeries` array silently drops series (issue #12)

**Reported in [issue #12](https://github.com/HauserGroup/curvebook/issues/12), opened 2026-11-18.** The `graph.setSeries` schema describes `series` as a "Batch series patch array", but the array replaces the series list. A one-element array sent to an 8-series XY graph succeeded with `"series_count": 1`, and the other 7 series, with their markers, error bars, and fitted-curve overlays, were removed without a warning. `edit.undo` restores them.

- Evidence from `run_graph_set_series` in `crates/engine/src/cmd/graph.rs`, the `if let Some(ref series_list) = params.series` branch near line 1541 at `2aa54b2`. It builds a new `Vec` matched to existing series by position and assigns it to `spec.series`. A shorter array drops the trailing series. A longer array invents series with `default_series_like` bound to `data_set_index = position`, which may not exist. `series_index` or `data_set_index` inside an item is ignored because the params accept extra properties. The command is marked `IDEMPOTENT`, which suggests a safe patch.
- Reproduction through MCP: inspect a graph with N > 1 series, run `command_run {"command_id": "graph.setSeries", "params": {"sheet_id": "<graph_id>", "series": [{"y_error_bars": {"metric": "Sem"}}]}}`, and inspect again to find 1 series. The response is `{"command_id":"graph.setSeries","dirty":true,"message":"updated series styling on `graph_1`","mutated":true,"payload":{"series_count":1,"sheet_id":"graph_1"}}`.
- The GUI is unaffected. The only in-tree caller of the array form, the intent builder in `crates/ui-egui/src/dialogs_canvas.rs` near line 4555, always sends one patch per series in order. `views/inspector.rs` and the Format Data dialog use the single-series form. MCP, `--control`, CLI, and batch callers can hit the truncation. The workaround is to send exactly one entry per existing series.
- Regression target: patch `spec.series[idx]` in place and leave series past the end alone, honouring an optional `series_index` or `data_set_index` in each item, with an explicit `"mode": "replace"` if replace semantics are needed (as `graph.formatSelectedPoints` has `merge | replace | clear`); or reject length mismatches with `ErrorCode::BadInput` naming both lengths. Optionally add `all_series: true` with a single patch object, matching `graph.setRegressionOverlay`. Update the `schema_graph_set_series` description and regenerate `docs/commands.md` and `docs/schemas`. Tests should cover a short array leaving N series, an embedded `series_index`, an over-long array, and the GUI's full-length path.

#### 123. High · Web host adapters mishandle lock handoff, persistence, sheet targeting, import, and touch (issue #21)

**Reported in [issue #21](https://github.com/HauserGroup/curvebook/issues/21), opened 2026-11-18.** An audit of the CurveBook Web host adapters at `daaecdc` found six defects. All 11 `browser_host_adapters` integration tests passed, and seven temporary native Rust probes reproduced the bridge behaviours with synthetic data. Browser consequences were traced through the JavaScript callers, and live browser behaviour and actual filesystem writes were not verified. The paths below are `apps/curvebook-web/src/host_adapters.rs` unless stated otherwise, with line numbers at `daaecdc`.

1. **A waiting tab can stay read-only after the browser grants it the lock.** If the editing tab closes while the waiting tab's last lock observation is within the 15-second lease TTL, JavaScript sets the ownership flag and emits `acquire_lock`, but Rust still treats the old observation as a live lease. The new owner's edits and autosave stay blocked. The browser grant should replace the cached observation and clear the banner. Evidence from `apps/curvebook-web/index.html:1555–1597` and `host_adapters.rs:2323–2335,626–674`.
2. **Folder saves advance the diff baseline and clear the dirty flag before disk I/O.** If a directory sync fails or is not applied, a retry without edits produces an empty plan because `last_synced_tree` already advanced. The shared virtual export helper also clears the session's dirty flag during preparation, and directory ZIP export has the same side effect before the picker succeeds or is cancelled. Tie the baseline and dirty state to confirmed persistence, build ZIP downloads without changing save state, and test a failed or partial write followed by a retry. The folder write-back workflow is tracked under R11.5. Evidence from `host_adapters.rs:947–1009`, `apps/curvebook-web/src/lib.rs:342–354`, and `index.html:865–881`.
3. **Saving preferences near quota drops the eviction list.** With the storage mirror near budget, a preference save evicts the autosave in memory and emits a browser write with an empty removal list, so a browser quota failure has no candidates to retry with. Later saves use an in-memory inventory that omits the browser's remaining snapshot. Keep eviction candidates in the persistence command, update the mirror from the browser outcome, and cover both a direct preference save and a UI change. Evidence from `host_adapters.rs:315–338,2315–2321,2938–2953` and `index.html:673–739`.
4. **Export and copy can return content from a different sheet.** A CSV export with a missing explicit sheet ID returns the active or first data sheet, and figure export falls back the same way. Copying with the active data sheet's ID explicitly supplied includes a graph image when a graph exists, while copying with no ID gives tabular content. Resolve and validate the target once, return a typed error for a bad explicit ID, and make every clipboard format represent that selection. Evidence from `host_adapters.rs:1035–1079,2088–2148`.
5. **Dropped folders can silently accept duplicate paths.** The drop handler overwrites map entries whose paths normalize to the same name before the directory import validator sees the batch. A synthetic folder with two `document.json` versions, one prefixed by `./`, was accepted on drop and rejected by direct directory import. Exact duplicates in the directory JSON event are also collapsed during decoding. Reject duplicate keys before building the map. Evidence from `host_adapters.rs:880–901,1137–1147,1626–1630`.
6. **Starting a pinch after a one-finger press leaves the pointer down.** A one-finger touch followed by a second finger in a separate event emits a primary `PointerDown` that is never ended, so a drag started by the first touch can stay armed after both fingers lift. Cancel the press when the gesture becomes a pinch and verify that releasing leaves no active pointer or drag. Evidence from `host_adapters.rs:1201–1288` and `crates/ui-egui/src/interaction.rs:1766–1780`.

#### 124. Medium · Wampler3 regression precision misses the 13.50-digit requirement, and CI does not require the NIST corpus (issue #28)

**Reported in [issue #28](https://github.com/HauserGroup/curvebook/issues/28), opened 2026-11-18.** On `a5fdb7f` with Rust 1.99.1 on Apple silicon, the full NIST StRD corpus exposes the same precision failure in the math and statistics linear-regression suites. The Wampler3 coefficient standard error SE(B1) reaches 13.33 log-relative digits against the required 13.50. CurveBook returns about 2363.55173469670035, and NIST certifies 2363.55173469680994.

- Reproduction:

  ```sh
  cargo xtask corpus --fetch-only
  CURVEBOOK_REQUIRE_CORPUS=1 cargo test --locked --no-fail-fast -p curvebook-math -p curvebook-stats -p curvebook-fit --test nist_strd
  ```

- The hosted correctness job runs without provisioning the external corpus, so on a clean runner the tests can return early when the corpus is absent.
- Regression target: correct the numerical path shared by math and statistics while keeping the 13.50-digit requirement, the certified values, and every assertion. Have the hosted correctness stage fetch and verify all 58 pinned NIST StRD files before its normal test archive, and fail the required workspace CI check when fixtures are missing, incomplete, or invalid. Verify the univariate, one-way ANOVA, and linear regression suites and all 27 nonlinear datasets from both starting points on the fixed code.

#### 125. Medium · `FirstAndLastLinear` baselines anchor at nominal window midpoints when window cells are blank or excluded (issue #39)

**Reported in [issue #39](https://github.com/HauserGroup/curvebook/issues/39), opened 2026-11-19, against R4.4.** `linear_baseline_for_col` computes `y_start` and `y_end` as means of the active numeric observations in the leading and trailing windows, skipping empty, non-finite, and excluded cells. It then pairs those means with the nominal window midpoints, `x_start = (first_k − 1)/2` and `x_end = (row_count − last_k) + (last_k − 1)/2`. When a window row is blank, excluded, or non-finite, or when replicate rows have unequal active counts, the anchor is at the wrong row coordinate and `BaselineOperation::Difference` no longer cancels exact linear drift. This is distinct from the trace note in the Wave 4 section that the baseline interpolates over the row index rather than X.

- Evidence from `crates/transform/src/ops/normalize_baseline.rs:443–468` at `ce80964` (`fn linear_baseline_for_col`). PR #36 (`e3a6ab0`) copied the midpoint formula into its reference generator `crates/transform/tests/reference/normalization.py:114–163` and baked the output into `crates/transform/tests/d1_normalization.rs:369–480` (`baseline_first_and_last_linear_windows_zero_denominator_and_missing_anchor`).
- For an affine baseline y(r) = α + βr, the active-window mean equals y(r̄), where r̄ is the mean row index of the contributing observations. The nominal midpoint equals r̄ only when every window row contributes the same nonzero number of observations or the active observations are symmetric about the window centre.
- Counterexample: 9 rows on y = 10 + 2r with row 1 blank and `first_k = last_k = 2`. The leading window gives 10 at r̄ = 0, and the trailing window gives 25 at r̄ = 7.5, so the correct baseline is 10 + 2r and Difference gives 0 everywhere. CurveBook uses x_start = 0.5, giving slope 15/7 and residuals from +15/14 at r = 0 down to −1/14 at r = 8.
- The PR #36 test asserts the defect. With `first_k = 2`, `last_k = 4`, and `row_count = 9`, column 1 has leading window `[1.0, Empty]`, so the correct anchor is (0, 1) and y(0) − B(0) = 0, but the test asserts 5/6 (`d1_normalization.rs:449`). In column 0 the trailing window `[6.0, 100.0 (excluded), 9.0, 12.0]` has active rows {5, 7, 8} with r̄ = 20/3, but both the code and the generator pair y_end = 9 with 6.5.
- Regression target: compute `x_start` and `x_end` from the mean row index of exactly the observations averaged into `y_start` and `y_end`, weighting per observation for unequal replicate counts, and handle the case where overlapping windows reduce to the same active row position. Document the anchor definition in `docs/implementation/data-tables-and-transforms.md` §5 and on `BaselineKind::FirstAndLastLinear`. Regenerate `normalization.py` and `d1_normalization.rs` from the mathematical definition, not from the production index expressions, and add an invariance check that Difference cancels noiseless affine data to 0 (and Ratio gives 1 where B(r) ≠ 0) with blank, excluded, and unbalanced-replicate cells in either window.

#### 126. Medium · Randomization oracles mirror production quota loops and omit multi-seed uniformity checks (issue #40)

**Reported in [issue #40](https://github.com/HauserGroup/curvebook/issues/40), opened 2026-11-19, against R9.14.** PR #34 (`93405f6`) adds independent `mpmath` and R (`pbinom`, `ppois`) oracles for Binomial and Poisson simulation, but its allocation and sampling coverage has two structural gaps. The quota reference functions are procedural ports of the Rust code, and no test checks that shuffles or samples are uniform.

- Mirroring: `hamilton_apportion`, `apportion_stratified_proportional`, and `apportion_stratified_equal` in `crates/transform/tests/reference/randomization.py:20–99` are line-for-line translations of the private functions in `crates/transform/src/randomize.rs:502–681` at `93405f6`, sharing loop structure, clamping order, and variable names (`enforce_min_one`, `clamped_any`, `next_active`, `unsaturated`, `step`). They cannot catch a mismatch between the iterative clamping loop and the intended allocation rule, such as how min-1 clamping interacts with largest-remainder tie-breaking across rounds.
- Uniformity: every test of `randomize_subjects_to_groups`, `sample_row_indices`, and `random_sample_rows` uses a single seed per fixture and checks only same-seed reproducibility, sizes, distinctness, ascending order, per-stratum quotas (`[8, 4, 2, 1]`, `[6, 4, 2, 1]`, `[6, 6, 5, 2]`), and subject bijection. The `Tiny` check `sel_prop_15.iter().any(|&r| r == 41 || r == 42)` is satisfied by always taking row 41. Replacing the body of `partial_fisher_yates` (`randomize.rs:686–694`) with a no-op would pass every unit, audit, and D1 test in `randomize.rs:1135–1363`, `tests/audit_r2_s04.rs:578–607`, and `tests/d1_randomization.rs:266–465`.
- Undetected defect classes: a naive full-range swap (`pool.swap(i, rng.gen_index(n))`), which for n = 3 gives permutation probabilities 4/27 and 5/27 instead of 1/6 while keeping counts and bijection; an off-by-one window (`gen_index(n − i − 1)`), which would give row 42 of `Tiny` inclusion probability 0 instead of 1/2; and truncated-final-block bias in `assign_subject_groups` (`randomize.rs:472–476`), where every tail subject must have marginal arm probability c_g/B = w_g/Σw, and variable `PermutedBlock { block_sizes }` (`randomize.rs:457`) must pick each size with probability 1/|block_sizes|.
- Regression target: replace the procedural ports with a declarative brute-force enumeration or constrained integer optimization over {n ∈ ℤ≥0^H : Σ n_h = S, ℓ_h ≤ n_h ≤ N_h}, with hand-worked fractional derivations in comments, and document the tie-break and clamping rule in `docs/implementation/data-tables-and-transforms.md` or the `randomize.rs` module docs. Add pre-specified multi-seed uniformity checks in `d1_randomization.rs`, with tail probabilities folded into a stated family-wise false-positive budget. For complete-balanced and permuted-block schedules, test uniformity over all B!/∏c_g! multiset permutations on a small case (for example B = 6 with weights 1:2:3), marginal arm frequencies w_g/Σw at every position including a truncated final block, and uniform selection of variable block sizes. For unstratified and stratified row sampling, test that every eligible row, including the first and last of each stratum, has inclusion frequency n_h/N_h (k/N unstratified), and that distinct seeds give distinct samples whenever more than one sample is possible.

## Remediation status on `bugfix/audit-2026-11-18-integ`

All 126 findings (`#1`–`#126`) in this audit have been reconciled across the `ce80964` baseline verification and Waves 1–4 on `bugfix/audit-2026-11-18-integ`, with two UI-overlapping items explicitly deferred to R15 to avoid colliding with concurrent R15 UI work.

### Summary by status (`#1`–`#126`)

| Status | Findings | Summary & Regression Evidence |
|---|---|---|
| **Baseline verified fixed (`ce80964` / Wave 1 re-check)** | `#1`, `#3` (process termination), `#5`, `#6`, `#8`, `#9`, `#10`, `#11`, `#12`, `#15`, `#16`, `#124` | Verified at `ce80964` and confirmed in Wave 1: `#9` DPI scaling & `ResourceLimit` rejection verified in `crates/render/tests/render_export.rs`; `#124` Wampler3 `SE(B1) >= 13.50` digits & `.github/workflows/ci.yml` `CURVEBOOK_REQUIRE_CORPUS=1` NIST StRD gate verified in `crates/math/tests/nist_strd.rs`. Surviving follow-on paths (`#70` from `#1`, `#119` from `#3`, `#115` from `#11`, `#117` from `#15`) were fixed in Waves 1–2 below. |
| **Evaluator fixed at baseline; UI hint remainder deferred to R15** | `#14` (UI formula hints) | Formula conditional argument semantics (`sumif`/`meanif`) were fixed at `ce80964` and verified in `crates/transform/tests/r63_window_and_conditional.rs`. The remaining reversed argument hints in `crates/ui-egui/src/grid_view.rs` and `crates/ui-egui/src/views/mv_formula.rs` are **Deferred to R15**. |
| **Partly fixed at baseline; surviving UI batch path deferred to R15** | `#2` (remainder), `#118` | The non-GUI `edit.undo`-then-create batch recovery (`#2`) was fixed at `ce80964`. The surviving mixed GUI-action batch journaling and dirty-state path (`#118` in `crates/ui-egui/src/control_io.rs` and `apps/curvebook/src/desktop_host.rs`) is **Deferred to R15**. |
| **Fixed in Wave 1 (`foundation`, `fit-dsl`, `stats-cat-roc-pow`, `format-io`, `mcp-cli-trust`, `web-adapters`)** | `#9` (verify), `#17`, `#18`, `#22`, `#23`, `#24`, `#25`, `#26`–`#29`, `#38`, `#41`, `#42`–`#50`, `#64` (linalg), `#65`, `#70`, `#71` (CLI/format), `#72`, `#73`, `#82` (math), `#88`, `#89`, `#90`, `#94` (doc), `#110` (doc), `#119`, `#123`, `#124` (verify); `#4` (doc) | Covered by `crates/doc/tests/audit_foundation_doc.rs`, `crates/math/tests/audit_foundation_math.rs`, `crates/fit/tests/audit_dsl_fixes.rs`, `crates/stats/tests/audit_cat_roc_pow_fixes.rs`, `crates/format/tests/{audit_wave1_format_io,recovery_and_interop_parity}.rs`, `crates/render/tests/render_export.rs`, `crates/mcp/tests/{audit_r2_s12,gui_only_surface,mcp_surface}.rs`, `apps/curvebook-cli/tests/{audit_r1_s4c_s12_10,audit_r2_s12,mcp_gui_only_parity,path_policy_and_mcp_workdir}.rs`, `apps/curvebook-web/tests/browser_host_adapters.rs`, and `docs/implementation/curve-fitting-implementation.md`. |
| **Fixed in Wave 2 (`stats-anova`, `stats-twogroup`, `stats-reg-surv`, `fit-solver`, `fit-comp-fix`, `transform-ops`)** | `#4` (stats), `#7` (stats), `#19`, `#20`, `#21`, `#30`–`#37`, `#39`, `#40`, `#51`–`#56`, `#59`–`#63`, `#64` (stats), `#66`–`#69` (code), `#77`, `#80` (fit), `#81` (multiplicity), `#82` (stats), `#84`–`#87`, `#91`, `#92`, `#93` (stats/fit), `#94` (fit/transform), `#96` (stats), `#100`–`#104`, `#109` (stats), `#113` (stats), `#114` (fit + `#87` alt-model rule filter), `#115`–`#117`, `#120` (stats), `#121`, `#125`, `#126` | Covered by `crates/stats/tests/{audit_anova_multiplicity_fixes,audit_twogroup_col_fixes,audit_reg_surv_mv_fixes,parity_g8_barnard_pb_mi,r35_power_multiplicity_oracles,r1010_pvalue_stack_oracles,engine_smoke,audit_r1_s4a_stats,prop_stats}.rs`, `crates/fit/tests/{audit_solver_fixes,nist_strd}.rs`, `crates/transform/tests/{audit_w2_transform_ops,d1_curve_transforms,d1_normalization,d1_randomization}.rs`, `crates/transform/tests/reference/{normalization,randomization}.py`, and `docs/implementation/data-tables-and-transforms.md`. |
| **Fixed in Wave 3 (`engine-analysis`, `engine-core`, `engine-reports`)** | `#4` (engine), `#7` (engine), `#13` (`#120`), `#32` (engine), `#57`, `#58`, `#71` (engine), `#74`–`#76`, `#78`–`#80` (engine), `#81` (reports), `#82` (reports), `#83`, `#93` (engine), `#95`–`#99`, `#102` (report check), `#105`–`#114` (reports/engine), `#120` (engine), `#122` | Covered by `crates/engine/tests/{audit_engine_analysis_fixes,audit_engine_core_fixes,audit_methods_report_fixes,audit_r2_s05_s07,rv_ledger_third_pass_engine,undo_and_methods_qol}.rs`. |
| **Fixed in Wave 4 (`audit-integ` polish & catalog sync)** | `#68` (docs sync), `#79`/`#93` (UI compat in `analysis.rs`), `archetype_convert.rs:2684` guard, catalog/schema sync | Fixed eager `label_cols[0]` panic in `crates/transform/src/archetype_convert.rs:2684` for `MultipleVariables -> Contingency` with zero `Label` columns (`crates/transform/tests/audit_w2_transform_ops.rs::wave4_mv_to_contingency_with_zero_label_columns_does_not_panic`); preserved explicit empty `"selected_data_sets": []` on all-continuous MV PCA (`#93` / UI EditParams compat) and validated top-level `parameter_rules` in `analysis.updateParams` on `LinearRegression` (`#79` / UI `ConstantHookPicker` compat) in `crates/engine/src/cmd/analysis.rs` + `crates/engine/tests/audit_engine_analysis_fixes.rs`; updated `#68` percentile docs (`crates/doc/src/analysis.rs`, `docs/implementation/statistics-implementation.md`); regenerated `docs/commands.md` and `docs/schemas/` via `cargo xtask commands` / `schemas`. |
| **Landed on `main` (R16 split port)** | All Waves 1–4 fixes (`#1`–`#126` except R15-deferred `#2` remainder/`#118`, `#14` UI hints) | Ported ANOVA (`crates/stats/src/anova/`), fit DSL/solver (`crates/fit/src/{dsl,solver}/`), and engine (`crates/engine/src/cmd/table/import.rs`, `crates/engine/src/cmd/spellings.rs`, `crates/engine/tests/param_spellings/golden.rs`) fixes onto R16's split module layout while preserving `main`'s submodule structure and spelling tables. |

### Deferred to R15 (intentional non-collision exclusions)

- **`#2` (GUI mixed-batch remainder) / `#118`:** `crates/ui-egui/src/control_io.rs` and `apps/curvebook/src/desktop_host.rs` — deferred to R15 desktop overhaul.
- **`#14` (UI formula argument-order hints):** `crates/ui-egui/src/grid_view.rs` and `crates/ui-egui/src/views/mv_formula.rs` — deferred to R15 desktop overhaul (formula evaluator behavior itself is fixed and tested in `crates/transform/tests/r63_window_and_conditional.rs`).
