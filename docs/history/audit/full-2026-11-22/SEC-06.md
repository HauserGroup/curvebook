# SEC-06 Audit Report — Statistics Regression, Survival, Multivariate & ANOVA (`crates/stats`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-06` (`18` source files in `crates/stats/src/`: `multivariate.rs`, `regression.rs`, `survival.rs`, and `anova/**`; `17,399` total lines)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-sci`
- **Mode:** Report-only static audit + read-only test verification (`cargo test --locked --offline -p curvebook-stats`: `292` passed — `18` unit tests + `273` integration tests across `33` binaries + `1` doc-test, `0` failed, `0` ignored)
- **Baseline Regression Check (`#1`–`#126`):** Verified closed with zero regressions across `SEC-06` (`#16`, `#19`, `#20`, `#30`–`#37`, `#39`, `#64`, `#84`–`#87`, `#91`–`#93`, `#96`, `#101`–`#103`, `#109`–`#111`, `#113`, `#117`, `#120` remain fixed on `main`).
- **Summary Counts:** `S1: 0` · `S2: 4` · `S3: 4` · `S4: 3` (Total: `11`)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Checked | Findings / Status |
|---|---:|---|---|---|
| `crates/stats/src/multivariate.rs` | 2899 | `1..=2899` | Pearson *r* + Fisher-*z* CI, Spearman *rho*, Kendall *tau_b*, correlation matrix BH-FDR (`#39` fixed), SVD PCA + deterministic loading signs + Horn (1965) parallel analysis, *k*-means++ + empty cluster repair (`#92`), Lance–Williams / NN-chain / heap hierarchical linkage + cophenetic distance + silhouette cuts (`n <= 2_000`), MICE PMM (`#91`, `#113`) + Rubin (1987) / Barnard–Rubin (1999) pooling (`#87`), `run_multivariate_analysis` MV/Column/Xy extraction (`#93`) | `FA-SEC06-02`, `FA-SEC06-07`, `FA-SEC06-09`, `FA-SEC06-11` |
| `crates/stats/src/regression.rs` | 2534 | `1..=2534` | Multiple linear regression QR/SVD + Dot2 residuals/Durbin–Watson (`#64`), Type I/III partial SS/F, VIFs, Cook's D & leverage; logistic IRLS with standardization, Albert–Anderson separation safeguard (`#84`–`#86`), Hosmer–Lemeshow, Tjur/Nagelkerke/McFadden *R*^2, ROC AUC; Poisson log-link IRLS, large-count `ln_gamma` (`#101`, `#103`), deviance/Pearson dispersion; Passing–Bablok shifted median/CIs & CUSUM (`#102`, `#117`); GLM/PB `ResultSheet` builders (`#96`, `#120`) | `FA-SEC06-07`, `FA-SEC06-08`, `FA-SEC06-09`, `FA-SEC06-11` |
| `crates/stats/src/survival.rs` | 3048 | `1..=3048` | Kaplan–Meier product-limit + Greenwood/log-log CIs + Brookmeyer–Crowley median CI, multi-cohort weighted log-rank / Gehan–Breslow–Wilcoxon / Tarone trend (`#19`, `#20`), two-cohort HR (log-rank/MH/Peto), Cox PH Newton–Raphson with covariate centering & per-risk-set stabilization (Breslow/Efron/Exact GLR DP), separation/collinearity non-convergence (`#96`), Grambsch–Therneau Schoenfeld diagnostics, Harrell *C*-index (`#16`), MV & Survival `ResultSheet` + pairwise multiplicity | `FA-SEC06-09`, `FA-SEC06-11` |
| `crates/stats/src/anova/mod.rs` | 114 | `1..=114` | `D1` literature citations, public module surface re-exports across `ANOVA` / `REML` / `nested` / `posthoc` | `Clean` — pure re-export boundary, zero `unsafe` / native I/O |
| `crates/stats/src/anova/effect_sizes.rs` | 228 | `1..=228` | Roundoff-floor SS clamping, `f_ratio_p` non-negative finite `F`/`P`, Steiger (2004) noncentral-*F* partial $\eta^2$ CI, clamped $\omega^2$/Cohen's *f*, Bartlett & median-centered Brown–Forsythe variance homogeneity (`available = false` for `n < 2`) | `Clean` — bounded clamping and safe small-sample guards |
| `crates/stats/src/anova/factorial.rs` | 353 | `1..=353` | Effect-coded `TypeI` / `TypeIII` sums of squares, rank-drop `df` via SVD/QR, `build_xtx_pinv_and_proj`, `build_cell_stats_map`, `kronecker_term_columns`, `OlsPostHocModel` construction | `FA-SEC06-03` |
| `crates/stats/src/anova/nested.rs` | 208 | `1..=208` | Hierarchical group / biological-subcluster / technical-replicate SS/MS, REML/KR/Satterthwaite group *F* and two-group nested *t*, subcluster-vs-replicate *F* | `FA-SEC06-10` |
| `crates/stats/src/anova/nonparametric.rs` | 118 | `1..=118` | Tie-corrected Kruskal–Wallis `H` & epsilon-squared (`total_n >= 3`, all-tied `H = 0`); rectangular complete-block within-block Friedman `Q` & Kendall's `W` (`n >= 2, k >= 2`) | `Clean` — validates finiteness, rectangularity, and all-tied denominators |
| `crates/stats/src/anova/one_way.rs` | 629 | `1..=629` | Two-pass NIST StRD ordinary one-way ANOVA, integer/finite `n` summary ANOVA (`#36`), Welch `W` + Brown–Forsythe `F*` (`#33`), complete-block RM one-way ANOVA with Helmert contrasts, Lecoutre (1991) `N - g + 1` Huynh–Feldt, Mauchly sphericity (`eps >= 1/(k-1)` lower bound) | `Clean` — numerically stable two-pass moments and `df`/variance validation |
| `crates/stats/src/anova/posthoc.rs` | 1652 | `1..=1652` | Estimability projection checks (`row_proj`), OLS/RM-balanced/REML contrast variance & KR/Satterthwaite `df`, exact rank-1 contrast-loading factorization (`#72`) with conservative SMM fallback, Newman–Keuls step-down, simple-effects multi-slice adjustment (`#71`, `#111`), asymmetrical Dunnett factor targeting (`#66`) | `Clean` — estimability, positive-SE, loading, and multiplicity invariants hold |
| `crates/stats/src/anova/reml.rs` | 1054 | `1..=1054` | Within/between orthogonal cluster decomposition, profiled `u = ln(1 + tau)` search in `[0, 36]` (`#37`), diagonal Fisher equilibration, multi-VC EM warm-start + AI REML, Fai–Cornelius Satterthwaite & Kenward–Roger (1997) `Phi_A` + scaled `F*`, boundary LRT (`0.5 * chi2(1)`) | `FA-SEC06-04`, `FA-SEC06-05`, `FA-SEC06-06` |
| `crates/stats/src/anova/reml_unstructured.rs` | 943 | `1..=943` | Pinheiro–Bates (1996) log-Cholesky parameterization (`LOG_CHOL_DIAG_{MIN,MAX}`), all-missing timepoint compaction + Gram-Schmidt rank reduction, EM warm-start + BFGS Armijo optimization, linear `vech(Sigma)` Fisher/KR matrices, documented SPD shrinkage fallbacks | `FA-SEC06-04` |
| `crates/stats/src/anova/results.rs` | 438 | `1..=438` | Shared ANOVA / RM / REML / nested / nonparametric result structs & `UnstructuredFallbackReason` / `RmSphericityCorrection` enums | `FA-SEC06-06` |
| `crates/stats/src/anova/rm_two_way.rs` | 382 | `1..=382` | Balanced two-way RM SS/df across `BothFactorsMatched`, `ColumnFactorMatched`, `RowFactorMatched`, `None`, Mauchly/GG/HF sphericity & `apply_rm_sphericity_p`, `RmBalancedPostHocModel` | `Clean` — rectangular validation, roundoff clamping, Lecoutre H-F/GG `df` floors |
| `crates/stats/src/anova/sheet/mod.rs` | 6 | `1..=6` | Submodule exports (`factorial`, `nested`, `one_way`) | `Clean` — pure module wire-up |
| `crates/stats/src/anova/sheet/factorial.rs` | 929 | `1..=929` | 2-way/3-way `2x2xK` Grouped & *N*-way MV OLS (`#30`), balanced RM two-way (`#109`), 2-way/3-way CS & Unstructured REML (`#110`), missing-timepoint footnotes, sphericity diagnostics tab, multiplicity runner, `sanitize_result_sheet_floats` | `FA-SEC06-05`, `FA-SEC06-09` |
| `crates/stats/src/anova/sheet/nested.rs` | 191 | `1..=191` | Nested ANOVA / two-group nested *t* `ResultSheet`, REML variance-components tab & boundary checklist, hierarchical `Var(mean_g)` group post-tests, `sanitize_result_sheet_floats` | `FA-SEC06-09`, `FA-SEC06-10` |
| `crates/stats/src/anova/sheet/one_way.rs` | 1673 | `1..=1673` | Ordinary/summary/Welch-BF/Kruskal–Wallis (`#31`–`#36`) + complete/incomplete RM & Friedman (`#35`) `ResultSheet` builders, paired GG contrasts vs pooled post-tests, Welch Dunnett/trend/pairwise, CS & Unstructured RM REML, `sanitize_result_sheet_floats` | `FA-SEC06-01`, `FA-SEC06-06`, `FA-SEC06-09` |

---

## Findings (`S1` -> `S4`)

### `FA-SEC06-01` · `S2` · Repeated-measures one-way ANOVA & Friedman runner accepts pre-summarized `ColumnTable` inputs (`wrong-result` / `contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/anova/sheet/one_way.rs:224-289, 742-763`
- **Category:** `wrong-result`
- **Defect:** `run_rm_one_way_anova_sheet` (invoked for both `OneWayMethod::RepeatedMeasures { .. }` and `OneWayMethod::Friedman`) scans the `ColumnTable` via `scan_rm_one_way_column_table` without checking `col_tab.subcolumn_mode.is_pre_summarized()`. Inside `scan_rm_one_way_column_table`, only subcolumn `0` (`col_tab.data_sets.get(c).and_then(|ds| ds.cell(r, 0))`) is read, treating pre-summarized summary values (`Mean`/`Median`) as raw subject-block observations while silently ignoring `SD`, `SEM`, `%CV`, `N`, `Q1`, `Q3`, or `Lower`/`Upper` subcolumns.
- **Failure scenario:** Running `OneWayMethod::Friedman` or `OneWayMethod::RepeatedMeasures` on a `ColumnTable` in `MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, or `MedianIqr` mode succeeds and computes matched-subject *F* tests, REML variance components, or Friedman rank statistics directly on group summary means/medians instead of returning `Err(StatsError::RequiresRawReplicates)` (contrast `KruskalWallis` at `sheet/one_way.rs:128-130`, `extract_numeric_columns` at `common.rs:297-299`, and `run_grouped_factorial` at `sheet/factorial.rs:277-279`).
- **Evidence:**
```rust
// crates/stats/src/anova/sheet/one_way.rs:749-763
    let TableArchetype::Column(col_tab) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "RM one-way ANOVA", actual: sheet.archetype.kind() });
    };
    let indices = try_resolve_selected_indices(&spec_selected_from_sheet(out), col_tab.data_sets.len())?;
    let scan = scan_rm_one_way_column_table(col_tab, &indices);

    if let OneWayMethod::RepeatedMeasures { .. } = anova_spec.method {
        validate_rm_parametric_multiplicity(anova_spec.multiplicity.method)?;
        if scan.y_vec.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        if scan.incomplete_rows > 0 || anova_spec.mixed_effects_reml || anova_spec.residual_covariance == ResidualCovariance::Unstructured {
            return run_rm_one_way_reml(col_tab, out, &indices, &scan, anova_spec, confidence, p_fmt);
        }
    }
```
```rust
// crates/stats/src/anova/sheet/one_way.rs:251-252
        for (pos, &c) in indices.iter().enumerate() {
            let cell = col_tab.data_sets.get(c).and_then(|ds| ds.cell(r, 0));
```
- **Suggested fix:** Check `if col_tab.subcolumn_mode.is_pre_summarized() { return Err(StatsError::RequiresRawReplicates); }` at the entry of `run_rm_one_way_anova_sheet` before `scan_rm_one_way_column_table`.
- **Related:** `crates/stats/src/common.rs:297-299`, `crates/stats/src/anova/sheet/factorial.rs:277-279`.

---

### `FA-SEC06-02` · `S2` · Multivariate analyses (`CorrelationMatrix`, `PCA`, `Clustering`, `MultipleImputation`) drop replicates and accept pre-summarized tables (`data-loss` / `wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/multivariate.rs:2391-2407, 2754-2783`
- **Category:** `data-loss`
- **Defect:** Both `extract_raw_optional_columns` (used by `MultivariateSpec::MultipleImputation`) and `extract_optional_columns` (used by `CorrelationMatrix`, `Pca`, and `Clustering`) extract only `subs.first()` on `TableArchetype::Column` and `TableArchetype::Xy` without checking `subcolumn_mode.is_pre_summarized()` or flattening/rejecting multi-replicate subcolumns.
- **Failure scenario:** When a `ColumnTable` or `XyTable` uses `SubcolumnMode::RawReplicates(k)` (`k >= 2`), replicate subcolumns `1..k` are silently dropped from imputation, correlation, PCA, and clustering. When the table is pre-summarized (`MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`), `Mean`/`Median` values are treated as raw case-level observations (for example, computing MICE within-imputation sampling variance `var_sample / n` on point summaries and discarding `SD`/`SEM`/`N`) rather than returning `Err(StatsError::RequiresRawReplicates)` as `extract_numeric_columns` (`common.rs:297-299`) and `build_xy_glm_design` (`regression.rs:1462-1464`) do.
- **Evidence:**
```rust
// crates/stats/src/multivariate.rs:2391-2406
        TableArchetype::Column(col_tab) => {
            let labels = col_tab.data_sets.iter().map(|ds| ds.title.clone()).collect();
            let cols =
                col_tab.data_sets.iter().map(|ds| ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect()).collect();
            let is_label = vec![false; col_tab.data_sets.len()];
            Ok(RawOptionalColumns { labels, cols, is_label })
        }
        TableArchetype::Xy(xy) => {
            let mut labels = vec![xy.x_title.clone()];
            let mut cols = vec![xy.x_cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect()];
            for ds in &xy.data_sets {
                labels.push(ds.title.clone());
                cols.push(ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect());
            }
```
```rust
// crates/stats/src/multivariate.rs:2754-2783
        TableArchetype::Column(col_tab) => {
            validate_selected_data_sets(selected, col_tab.data_sets.len())?;
            let indices = resolve_selected_indices(selected, col_tab.data_sets.len());
            ...
            for idx in indices {
                let ds = &col_tab.data_sets[idx];
                labels.push(ds.title.clone());
                cols.push(ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect());
            }
```
- **Suggested fix:** Reject `col_tab.subcolumn_mode.is_pre_summarized()` / `xy.y_subcolumn_mode.is_pre_summarized()` with `Err(StatsError::RequiresRawReplicates)` across `extract_raw_optional_columns` and `extract_optional_columns`, and either reject `RawReplicates(k > 1)` (`IncompatibleTable`) or explicitly define row/subcolumn expansion semantics consistent with `build_xy_glm_design`.
- **Related:** `crates/stats/src/common.rs:297-299`, `crates/stats/src/regression.rs:1462-1464`.

---

### `FA-SEC06-03` · `S2` · Public `fit_factorial_anova` panics via unchecked indexing on malformed `FactorialObservations` (`no-panic`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/anova/factorial.rs:114-160, 311-326`
- **Category:** `no-panic`
- **Defect:** `fit_factorial_anova` (re-exported publicly at `crates/stats/src/anova/mod.rs:100`) accepts a public `FactorialObservations` struct whose fields (`factor_names`, `factor_levels`, `rows`) are all `pub`, without validating that `factor_levels.len() == factor_names.len()`, that each row's `lvls.len() == factor_names.len()`, or that every `lvls[f_idx] < factor_levels[f_idx].len()`. `kronecker_term_columns` then performs unchecked indexing on `factor_basis[f_idx]` and `f_rows[lvls[f_idx]][c_f]`.
- **Failure scenario:** Calling `curvebook_stats::anova::fit_factorial_anova` directly with mismatched `factor_names`/`factor_levels`, a ragged row level vector, or an out-of-range level index triggers an out-of-bounds slice panic (`index out of bounds`) instead of returning `StatsError::DimensionMismatch` / `StatsError::InvalidParameter`. (Sheet-level callers in `anova/sheet/factorial.rs` construct valid dense indices before calling.)
- **Evidence:**
```rust
// crates/stats/src/anova/factorial.rs:123-135
pub(in crate::anova) fn fit_factorial_anova_with_model(
    data: &FactorialObservations,
    max_interaction_order: usize,
    ss_type: SumOfSquaresType,
) -> Result<(FactorialAnovaResult, OlsPostHocModel), StatsError> {
    let n_obs = data.rows.len();
    let n_factors = data.factor_names.len();
    if n_obs < 3 || n_factors == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial ANOVA observations", required: 3, actual: n_obs });
    }
```
```rust
// crates/stats/src/anova/factorial.rs:311-324
pub(in crate::anova) fn kronecker_term_columns(rows: &[(Vec<usize>, f64)], factor_basis: &[Vec<Vec<f64>>], sub_factors: &[usize]) -> Vec<Vec<f64>> {
    let mut prod = vec![vec![1.0_f64; rows.len()]];
    for &f_idx in sub_factors {
        let n_cols_f = factor_basis[f_idx].first().map_or(0, Vec::len);
        if n_cols_f == 0 {
            return Vec::new();
        }
        let f_rows = &factor_basis[f_idx];
        let mut next = Vec::with_capacity(prod.len() * n_cols_f);
        for prev in &prod {
            next.extend((0..n_cols_f).map(|c_f| rows.iter().enumerate().map(|(r_idx, (lvls, _))| prev[r_idx] * f_rows[lvls[f_idx]][c_f]).collect()));
        }
```
- **Suggested fix:** Validate at the top of `fit_factorial_anova_with_model` that `data.factor_levels.len() == n_factors` (`DimensionMismatch`), that every `(lvls, _)` in `data.rows` has `lvls.len() == n_factors` (`DimensionMismatch`), and that `lvls[f_idx] < data.factor_levels[f_idx].len()` (`InvalidParameter`).
- **Related:** `crates/stats/src/anova/mod.rs:100`.

---

### `FA-SEC06-04` · `S2` · Public REML entrypoints panic on out-of-range or wrapping `terms` ranges (`no-panic`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/anova/reml_unstructured.rs:254-258` and `crates/stats/src/anova/reml.rs:587-590`
- **Category:** `no-panic`
- **Defect:** Two public REML entrypoints (`fit_unstructured_rm_reml*` and `fit_random_intercept_reml*` / `fit_multi_vc_reml*` at `anova/mod.rs:104-105`) fail to guard caller-supplied `(name, start, len)` term ranges cleanly:
  1. `unstructured_reduce_design` (`reml_unstructured.rs:255-256`) computes `let end = start.saturating_add(len).min(p);` and directly slices `col_map[start..end]`. If `start > p`, `start > end` and slicing panics.
  2. `eval_reml_wald_terms_for_df` (`reml.rs:588`) checks `if len == 0 || start + len > p` using unchecked addition `start + len`, which panics on arithmetic overflow in debug builds for large `start`/`len` values (and silently skips valid truncated terms rather than checking range validity).
- **Failure scenario:** Calling `fit_unstructured_rm_reml(&x, &y, &c, &t, &lbls, &[("T", p + 1, 1)])` panics in `unstructured_reduce_design`; calling `fit_random_intercept_reml` or `fit_multi_vc_reml` with `("T", usize::MAX, 1)` overflows in debug builds.
- **Evidence:**
```rust
// crates/stats/src/anova/reml_unstructured.rs:253-259
    let mut reduced_terms = Vec::with_capacity(terms.len());
    for &(name, start, len) in terms {
        let end = start.saturating_add(len).min(p);
        let mapped: Vec<usize> = col_map[start..end].iter().filter_map(|&m| m).collect();
        let new_start = mapped.first().copied().unwrap_or(0);
        reduced_terms.push((name.to_owned(), new_start, mapped.len()));
    }
```
```rust
// crates/stats/src/anova/reml.rs:587-590
    for &(name, start, len) in terms {
        if len == 0 || start + len > p {
            continue;
        }
```
- **Suggested fix:** Validate `len == 0` / `start >= p` / `start.checked_add(len).is_none_or(|end| end > p)` (returning `StatsError::InvalidParameter` or clamping `start.min(p)..end` consistently across both REML modules).
- **Related:** `crates/stats/src/anova/mod.rs:104-105`.

---

### `FA-SEC06-05` · `S3` · Compound-symmetry RM REML fails on all-missing-cell / collinear designs instead of rank-reducing `X` (`robustness`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/anova/sheet/factorial.rs:636-642, 702-767` and `crates/stats/src/anova/reml.rs:272, 802`
- **Category:** `robustness`
- **Defect:** In `run_rm_grouped_reml`, `build_reml_effect_design` emits the full effect-coded fixed-effects design matrix `x_mat` (`p_total` columns including all interaction products up to `max_interaction_order`). When `residual_covariance == ResidualCovariance::Unstructured`, `unstructured_reduce_design` (`reml_unstructured.rs:193`) removes structurally aliased columns before Cholesky factorization. Under the default `ResidualCovariance::CompoundSymmetry`, however, `x_mat` is passed unreduced to `fit_random_intercept_reml_with_method_and_core` / `fit_multi_vc_reml_with_method_and_core`, whose inner likelihood evaluators require strict positive-definiteness (`cholesky(&xtvx_mat)?` at `reml.rs:272`, `cholesky(&xtvix)?` at `reml.rs:802`).
- **Failure scenario:** On any 2-way or 3-way `2x2xK` `GroupedTable` repeated-measures dataset where all subjects miss a particular `(row, col)` combination (or where an entire `(A, B, Row)` interaction cell is empty/excluded while all factor levels still appear elsewhere), interaction columns become collinear with lower-order terms. With default `ResidualCovariance::CompoundSymmetry`, `run_factorial_anova` aborts with `StatsError::Math(MathError::NonPositiveDefinite)`, whereas `ResidualCovariance::Unstructured` and fixed-effects OLS (`factorial.rs`) succeed on the estimable subspace.
- **Evidence:**
```rust
// crates/stats/src/anova/sheet/factorial.rs:636-641, 757-765
    let RemlEffectDesign { x_mat, subsets, factor_basis, terms: term_ranges, observed_cells } =
        build_reml_effect_design(&obs_cells, &factor_names, &factor_levels, max_order)?;
    let term_slices: Vec<(&str, usize, usize)> = term_ranges.iter().map(|(n, s, l)| (n.as_str(), *s, *l)).collect();
    ...
                    emit_multi_vc_rm_reml(out, &x_mat, &y_vec, &strata, &term_slices, fac_spec.df_method)?
                }
                RepeatedMeasuresDesign::ColumnFactorMatched => {
                    emit_random_intercept_reml(out, &x_mat, &y_vec, &subj_row_ids, &term_slices, fac_spec.df_method, p_fmt)?
```
```rust
// crates/stats/src/anova/reml.rs:271-272 & 801-802
    let xtvx_mat = DenseMatrix::from_row_major(p, p, xtvx)?;
    let chol_x = cholesky(&xtvx_mat)?;
...
    let xtvix = symmetrize_dense(&xt_vi.matmul(x)?)?;
    let chol_m = cholesky(&xtvix)?;
```
- **Suggested fix:** Reuse `unstructured_reduce_design` (promoting it to `pub(in crate::anova)`) in the `CompoundSymmetry` REML paths (populating `core.col_map` consistently for `RemlContrastModel`), or apply rank-preserving projection inside `ri_eval_profile` / `multi_vc_eval_state`.
- **Related:** `crates/stats/src/anova/reml_unstructured.rs:193-261`.

---

### `FA-SEC06-06` · `S3` · Multi-VC REML omits convergence tracking and silently suppresses non-convergence diagnostics (`robustness` / `contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/anova/results.rs:331-344`, `crates/stats/src/anova/reml.rs:940-1054`, and `crates/stats/src/anova/sheet/one_way.rs:365-415`
- **Category:** `robustness`
- **Defect:** Unlike `MixedRemlResult` (`results.rs:321-326`) and `UnstructuredRemlResult` (`results.rs:386-390`), `MultiVcRemlResult` (`results.rs:331-344`) contains no `converged` or boundary status field. In `fit_multi_vc_reml_with_method_and_core` (`reml.rs:1006-1011`), if the 80-iteration Average-Information loop reaches the iteration limit without satisfying gradient/step convergence (`(converged, _) = multi_vc_ai_step(...)`), the loop simply terminates and proceeds with the unconverged `theta` without recording or surfacing non-convergence. `emit_multi_vc_rm_reml` (`sheet/one_way.rs:365-415`) therefore emits no convergence footnote or checklist warning (unlike `emit_random_intercept_reml` at `sheet/one_way.rs:327-335` and `emit_unstructured_rm_reml` at `sheet/one_way.rs:460-489`).
- **Failure scenario:** In doubly repeated-measures 2-way (`BothFactorsMatched`) or 3-way REML fits where AI REML fails to converge within 80 iterations (or stalls in EM backtracking), both `fit_multi_vc_reml*` callers and `ResultSheet` consumers receive variance components and Wald *F*/Kenward–Roger/Satterthwaite *p*-values with no indicator or warning that optimization did not converge.
- **Evidence:**
```rust
// crates/stats/src/anova/reml.rs:1006-1013
    for _ in 0..80 {
        let (converged, _) = multi_vc_ai_step(x, &y_scaled, &gs, &dofs, &mut theta, n_minus_p)?;
        if converged {
            break;
        }
    }

    let st = multi_vc_eval_state(x, &y_scaled, &gs, &theta)?;
```
```rust
// crates/stats/src/anova/results.rs:331-344
pub struct MultiVcRemlResult {
    pub variance_components: Vec<(String, f64)>,
    pub sigma2_error: f64,
    pub reml_log_lik: f64,
    pub fixed_terms: Vec<FactorialAnovaTerm>,
    pub satterthwaite_df: Vec<f64>,
    pub kenward_roger_df: Vec<f64>,
}
```
- **Suggested fix:** Add `#[serde(default = "default_true")] pub converged: bool` (and optionally `at_boundary: bool`) to `MultiVcRemlResult`, track the final `multi_vc_ai_step` convergence boolean + active score norm check, and emit corresponding variance-component footnotes and checklist warnings in `emit_multi_vc_rm_reml`.
- **Related:** `crates/stats/src/anova/sheet/one_way.rs:327-363`.

---

### `FA-SEC06-07` · `S3` · Passing–Bablok drops `XyTable` replicate subcolumns and Regression/MV sheet runners skip `sanitize_result_sheet_floats` (`data-loss` / `robustness`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/regression.rs:1811-1826, 1939-1959, 1991, 2426-2439` and `crates/stats/src/multivariate.rs:1828-1830`
- **Category:** `data-loss`
- **Defect:**
  1. In `extract_passing_bablok_pairs` (`regression.rs:1811-1835`), `TableArchetype::Xy` rejects `is_pre_summarized()` (allowing `RawReplicates(k)`), but `val_for` reads only subcolumn `0` (`t.data_sets[col - 1].cell(row, 0)`) instead of iterating replicate subcolumns `0..row_len` like `build_xy_glm_design` (`regression.rs:1510-1518`, updated in `#120`), silently discarding replicates `1..k`.
  2. Unlike every `anova/sheet/*.rs` runner and `run_survival_analysis` (`survival.rs:2070`), `run_glm_analysis` (`regression.rs:2438`), `run_passing_bablok_analysis` (`regression.rs:1991`), and `run_multivariate_analysis` (`multivariate.rs:1829`) never call `sanitize_result_sheet_floats(&mut out)?;` before returning `Ok(out)`. For example, in `run_passing_bablok_analysis` (`regression.rs:1940`), large-magnitude inputs where `res.intercept + res.slope * xv` overflows `f64` emit `y: f64::INFINITY` directly into `out.fitted_curves`.
- **Failure scenario:** Running Passing–Bablok regression on an `XyTable` with `RawReplicates(3)` silently fits only the `Y1` subcolumn (`1/3` of the measurements); running Passing–Bablok near `f64` overflow emits non-finite curve overlay points rather than catching them in `sanitize_result_sheet_floats`.
- **Evidence:**
```rust
// crates/stats/src/regression.rs:1824-1834
            let val_for = |row: usize, col: usize| -> Option<f64> {
                if col == 0 { t.x_value_at(row) } else { t.data_sets[col - 1].cell(row, 0).and_then(Cell::active_number) }
            };
            for r in 0..t.row_count() {
                if let (Some(xv), Some(yv)) = (val_for(r, x_col), val_for(r, y_col)) {
                    xs.push(xv);
                    ys.push(yv);
                    source_rows.push(r + 1);
                }
            }
```
```rust
// crates/stats/src/regression.rs:1940-1951
            let yv = res.intercept + res.slope * xv;
            ...
            pts.push(CurveSamplePoint { x: xv, y: yv, ci_lower: ci_lo, ci_upper: ci_hi, pred_lower: None, pred_upper: None });
```
- **Suggested fix:** Pair replicate subcolumns on `XyTable` in `extract_passing_bablok_pairs` (or aggregate/document explicitly, e.g., aligning `(t.x_value_at(r), ds.cell(r, sub))` when `x_col == 0` and `(ds_x.cell(r, sub), ds_y.cell(r, sub))` otherwise), and invoke `sanitize_result_sheet_floats(&mut out)?;` at the exit of `run_glm_analysis`, `run_passing_bablok_analysis`, and `run_multivariate_analysis`.
- **Related:** `crates/stats/src/regression.rs:1510-1518`, `crates/stats/src/common.rs:494-548`.

---

### `FA-SEC06-08` · `S3` · `compute_vifs_from_matrix` mixes uncentered `rss` with centered `css` when `has_intercept == false` (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/regression.rs:502-538`
- **Category:** `wrong-result`
- **Defect:** When public `multiple_linear_regression` (`regression.rs:242`) is called with `has_intercept = false` and `compute_vif = true` for `p >= 2` predictors, `compute_vifs_from_matrix` regresses column `j` on `retained_cols` (`532`: `solve_linear_least_squares(&xm, &target)`) without an intercept (since `x` has none), producing an **uncentered** residual sum of squares `sol.rss`, and then computes `r2 = (1.0 - sol.rss / tm.css).clamp(0.0, ...)` against the **centered** total sum of squares `tm.css` (`SummaryMoments::from_slice(&target)`). (The internal `run_glm_analysis` runner always sets `has_intercept = true`.)
- **Failure scenario:** Public library callers computing VIFs on a no-intercept regression with non-zero-mean collinear predictors get artificially deflated `R^2` (clamped to `0.0` whenever uncentered `sol.rss >= tm.css`) and falsely low `VIF = 1.0`.
- **Evidence:**
```rust
// crates/stats/src/regression.rs:516-535
        let retained_cols: Vec<usize> = (0..p).filter(|&c| c != j && !aliased.get(c).copied().unwrap_or(false)).collect();
        if retained_cols.len() <= start {
            continue;
        }
        ...
        let Ok(xm) = DenseMatrix::from_row_major(n, retained_cols.len(), pred_cols) else { continue };
        let Ok(tm) = SummaryMoments::from_slice(&target) else { continue };
        if tm.css <= 0.0 {
            continue;
        }
        if let Ok((sol, _)) = solve_linear_least_squares(&xm, &target) {
            let r2 = (1.0 - sol.rss / tm.css).clamp(0.0, 0.999_999_999);
            *slot = Some(1.0 / (1.0 - r2));
        }
```
- **Suggested fix:** Include an intercept column in `xm` when computing auxiliary centered `R_j^2` for VIF (or compute uncentered `ss_tot = sum(target^2)` when `has_intercept == false`, per standard regression collinearity convention).
- **Related:** `crates/stats/src/regression.rs:272-280`.

---

### `FA-SEC06-09` · `S4` · `SEC-06` `ResultSheet` builders use `p_cell` instead of `p_cell_at_alpha` (`contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/anova/sheet/one_way.rs:109, 111, 147, 315, 620, 681, 807-809, 816, 909, 1048, 1059, 1105, 1138, 1288`, `crates/stats/src/anova/sheet/factorial.rs:80, 183, 186-187`, `crates/stats/src/anova/sheet/nested.rs:54, 61, 177`, `crates/stats/src/regression.rs:1898, 2022, 2043, 2094, 2127, 2259, 2361`, `crates/stats/src/survival.rs:1967, 1977, 1985, 2123, 2126, 2875-2876, 2895-2896`, `crates/stats/src/multivariate.rs:1484, 1492, 2626`
- **Category:** `contract-drift`
- **Defect:** All ANOVA, regression/Passing–Bablok, survival/Cox, correlation matrix, and MICE `ResultSheet` builders format `ResultCell::PValue` via `p_cell(raw_p, p_fmt)` (which evaluates star glyphs against a fixed `0.05` threshold) instead of `p_cell_at_alpha(raw_p, p_fmt, alpha)` using `spec.confidence_level.alpha()` (matching `column.rs` and `two_group.rs`).
- **Failure scenario:** Setting `AnalysisSpec::confidence_level` to `99%` (`alpha = 0.01`) updates confidence intervals and checklist checks (`p < 0.01`), while a cell with `p = 0.03` in the ANOVA, regression, Cox, survival, or MICE table still displays `*`.
- **Evidence:**
```rust
// crates/stats/src/anova/sheet/factorial.rs:80
            t.p_value.map_or(ResultCell::Empty, |p| p_cell(p, p_fmt)),
// crates/stats/src/regression.rs:2094
            vec![num_cell(t.estimate), num_cell(t.se), num_cell(t.t_statistic), p_cell(t.p_value, p_fmt), finite_ci_cell(t.ci.0, t.ci.1)]
// crates/stats/src/survival.rs:2123
            p_cell(cov.p_value, p_fmt),
// crates/stats/src/multivariate.rs:2626
            p_cell(term.p_value, p_fmt),
```
- **Suggested fix:** Replace `p_cell(..., p_fmt)` with `p_cell_at_alpha(..., p_fmt, confidence.alpha())` (and `mult.threshold` for adjusted survival multiplicity columns).
- **Related:** `SEC-05` (`FA-SEC05-12`).

---

### `FA-SEC06-10` · `S4` · Nested ANOVA error classification on `g_count > 2` and skipped `k = 2` multiplicity / ignored-text disclosure (`cleanup` / `contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/anova/nested.rs:60-66` and `crates/stats/src/anova/sheet/nested.rs:110`
- **Category:** `cleanup`
- **Defect:**
  1. When `two_group_t_test == true` and `selected_groups.len() > 2` (e.g., `3` groups), `nested_one_way_anova_with_reml_status` (`nested.rs:61-63`) returns `StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: g_count }`, reporting `"required: 2, actual: 3"` instead of `StatsError::InvalidParameter`.
  2. `run_nested_analysis` (`sheet/nested.rs:110`) skips `append_nested_multiplicity` when `!nested_spec.two_group_t_test && selected_groups.len() == 2` (whereas `sheet/one_way.rs` emits multiplicity output even for `k == 2`), and never tracks/reports ignored non-numeric cells in `NestedTable` (`append_ignored_text_checklist_and_footnote`).
- **Failure scenario:** Passing 3 selected groups with `two_group_t_test: true` yields a misleading insufficient-sample-size error message; selecting 2 groups with `two_group_t_test: false` and non-`None` multiplicity silently omits the comparison tab.
- **Evidence:**
```rust
// crates/stats/src/anova/nested.rs:60-66
    let g_count = selected_groups.len();
    if two_group_t_test && g_count != 2 {
        return Err(StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: g_count });
    }
    if g_count < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: g_count });
    }
```
```rust
// crates/stats/src/anova/sheet/nested.rs:110-112
    if !nested_spec.two_group_t_test && selected_groups.len() >= 3 {
        append_nested_multiplicity(&mut out, nested, &selected_groups, &res, active_df2, spec, nested_spec)?;
    }
```
- **Suggested fix:** Check `g_count < 2` first (`InsufficientSampleSize`), then `if two_group_t_test && g_count != 2 { return Err(StatsError::InvalidParameter { name: "nested_t_test_groups", value: g_count as f64 }); }`. Allow `selected_groups.len() >= 2` for nested multiplicity when `!two_group_t_test` and `method != MultiplicityMethod::None`, and report ignored non-numeric `NestedTable` cells cleanly.
- **Related:** `crates/stats/src/anova/sheet/one_way.rs:209`.

---

### `FA-SEC06-11` · `S4` · Minor diagnostic context, docstring drift, and missing listwise/non-numeric footnotes across MV, Regression & Survival (`cleanup`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/stats/src/multivariate.rs:297-298, 323-324`, `crates/stats/src/regression.rs:1564, 1756, 1847-1992, 2426-2439`, and `crates/stats/src/survival.rs:774, 819`
- **Category:** `cleanup`
- **Defect:** Several small error-message, docstring, and diagnostic-disclosure inconsistencies remain:
  1. `spearman_correlation` (`multivariate.rs:298`) and `kendall_tau_b` (`multivariate.rs:324`) return `InsufficientSampleSize { context: "Pearson correlation", ... }` copy-pasted from `pearson_correlation`.
  2. `PassingBablokResult::linearity_passed` doc comment (`regression.rs:1564`) states `` `cusum_p_value >= 0.05` `` whereas `passing_bablok_regression` (`regression.rs:1756`) uses `conf.alpha()`.
  3. `CoxSubject::time` doc comment (`survival.rs:774`) states `` `<= 0` subjects are rejected `` whereas `cox_ph_regression` (`survival.rs:819`) checks `s.time < 0.0` (accepting `0.0`).
  4. `run_glm_analysis` and `run_passing_bablok_analysis` (`regression.rs`) silently omit `append_listwise_filtered_checklist_and_footnote` and `append_ignored_text_checklist_and_footnote` disclosures when incomplete or non-numeric cells are skipped (unlike `survival.rs` and `anova/sheet/one_way.rs`).
- **Failure scenario:** Callers reading error contexts or doc comments observe mismatched test names or thresholds (`0.05` vs `alpha`, `< 0.0` vs `<= 0.0`), and users with text/blank entries in GLM or Passing–Bablok columns receive no row-filtering checklist note.
- **Evidence:**
```rust
// crates/stats/src/multivariate.rs:297-299 & 323-325
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Pearson correlation", required: 3, actual: n });
    }
```
```rust
// crates/stats/src/regression.rs:1564 & 1756
    /// True when `cusum_p_value >= 0.05` (no significant departure from linearity detected by CUSUM test).
    pub linearity_passed: bool,
    ...
    let linearity_passed = cusum_p_value >= conf.alpha();
```
```rust
// crates/stats/src/survival.rs:774 & 819
    /// Positive follow-up time (`> 0.0`; non-finite or `<= 0` subjects are rejected).
    pub time: f64,
    ...
        if !s.time.is_finite() || s.time < 0.0 || s.x.len() != p || s.x.iter().any(|v| !v.is_finite()) {
```
- **Suggested fix:** Update the error `context` strings to `"Spearman correlation"` / `"Kendall tau-b correlation"`, align the `PassingBablokResult::linearity_passed` and `CoxSubject::time` doc comments with implementation, and wire `append_listwise_filtered_checklist_and_footnote` / `append_ignored_text_checklist_and_footnote` into `run_glm_analysis` and `run_passing_bablok_analysis`.
- **Related:** `crates/stats/src/survival.rs:2050-2051, 2065-2066`.
