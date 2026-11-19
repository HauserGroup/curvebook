# Scientific QoL Extensions and Deferred Calculator Reference (`R9.12`–`R9.16`)

> Related specifications: [`statistics-implementation.md`](../implementation/statistics-implementation.md), [`curve-fitting-implementation.md`](../implementation/curve-fitting-implementation.md), [`data-tables-and-transforms.md`](../implementation/data-tables-and-transforms.md), [`graphing-and-layouts.md`](../implementation/graphing-and-layouts.md), [`qol-enhancements.md`](qol-enhancements.md), [`gui-workflows.md`](../gui-workflows.md) · Roadmap: [`ROADMAP.md`](../ROADMAP.md#r9-post-parity-scientific-quality-of-life-suite) (`R9.12`–`R9.16` / `QOL-12`–`QOL-16`) · Live metrics: [`STATUS.md`](../STATUS.md)

Active scope covers categorical/continuous table-analysis completions (`R9.12`), seeded randomization and sampling (`R9.14`), and symbolic/live best-fit equation annotations only (`R9.16`) — all implemented across Waves 21–22b. Statistical/distribution/Bayesian calculators (`R9.13`) and bench-chemistry/radiochemical calculators (`R9.15`) remain deferred without scheduled milestones; their detailed designs remain as inactive reference material. Compact Letter Display (`CLD`) is not part of any calculator or `R9` item; it is back in product scope and tracked separately as [`R10.11`](roadmap-history/r10.md#r1011-compact-letter-display-cld) (scope confirmed 2026-11-15).

> **Status Note (Implemented scope vs. deferred calculators):**
> - **Shipped end-to-end (`R1`–`R7` & Waves 21–22b `R9.12`/`R9.14`/`R9.16`):** $2 \times 2$ & $R \times C$ contingency ($\chi^2$, Yates, $G^2$, Fisher's exact, Barnard's exact, Baptista–Pike OR, Koopman RR, Newcombe RD, inverted Altman `NNTB`/`NNTH` CIs via `altman_nnt_from_rd`, McNemar's test + discordant-pair OR CIs via `mcnemar_detailed`, weighted Cohen's `cohens_kappa` & Fleiss' `fleiss_kappa`, exact binomial `ContingencyMethod::ExactBinomial`), uniform & custom expected proportion goodness-of-fit $\chi^2$, descriptive statistics + arbitrary $0.01\%\text{–}99.99\%$ mean CIs & exact $\chi^2$ `sd_ci` (`RawReplicates` and pre-summarized `Mean + SD/SEM + N`), Grubbs/ROUT outliers, unpaired/Welch/paired/ratio/one-sample *t*-tests (with direct `Mean + SD/SEM + N` in `run_two_group_analysis`) + paired exact Sign test (`TwoGroupMethod::PairedSignTest`), simple/Deming linear regression, factorial ANOVA main/simple-effect multiplicity post-hoc tabs (`run_factorial_anova`), extra-sum-of-squares *F* & $\text{AIC}_c$ fit comparisons, `log_agonist_vs_response_ecf` ($\log\text{EC}_F$ curve fitting), live pairwise significance brackets (`GraphSpec::comparisons`), seeded XY Monte Carlo simulation (`table.simulateXy`), seeded group randomization/row sampling/distribution simulation (`crates/transform/src/randomize.rs`: `table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions`), and linked symbolic/live-substituted $\TeX$ model equation annotations (`crates/fit/src/tex_format.rs`, `crates/chart/src/equation_box.rs`, `GraphAnnotation::ModelEquation`, `ui.graph.insertModelEquation`).
> - **Deferred (outside active backlog):** `R9.13` (`QOL-13` instant statistical/distribution/Bayesian calculators) and `R9.15` (`QOL-15` bench molarity/dilution/radiochemical calculators) remain deferred without milestones; their reference sections below are not active implementation assignments. Existing core probability distributions (`norm`/`student_t`/`f`/`chi2`, noncentral, Dunnett, studentized range in `crates/math/src/dist/`), regularized incomplete beta/gamma special functions (`crates/math/src/special/`), and statistical routines remain supported. None of the `calc.*` command IDs below are registered in `cmd::registry()` (see the generated [`commands.md`](../commands.md)).

---

## 1. Non-Disruptive Design Principles

Calculator-specific design rules below apply only if deferred work is explicitly resumed; the described calculator modules and commands are proposed seams, not implementation claims.

1. **Zero Archive-Schema Churn for Instant Calculators:**
   - Summary-input statistical calculators, probability tail/quantile evaluators, clinical Bayes calculators, two-mean error propagation, and wet-lab chemistry/radiochemistry tools execute as **stateless, read-only engine queries** (proposed: a new `CommandGroup::Calculator` with `READ_ONLY` effects in a new `crates/engine/src/cmd/calc.rs`; neither exists today).
   - Opening or running an instant calculator mutates zero sheets, consumes zero undo slots, and leaves `.curvebook` / `.curvebook.d/` archive schemas (`crates/format/src/versioned.rs`) untouched unless the user explicitly invokes *Insert Result into Info Sheet* or *Create Dilution / Randomization Data Sheet*.
2. **Additive `#[serde(default)]` Table & Graph Extensions:**
   - New variants and optional fields on `ContingencyMethod`, `ContingencySpec`, `DescriptiveSummary`, `TransformAnalysisSpec`, and `GraphSpec` serialize with `#[serde(default, skip_serializing_if = "...")]`, preserving 100% forward/backward compatibility with existing `.curvebook` projects and batch scripts.
3. **100% Offline & WASM-Safe (`#![forbid(unsafe_code)]`):**
   - All active mathematical, randomization, and equation-rendering routines, and any deferred calculator routines if resumed, run deterministically in pure Rust (`L0`–`L3`) identically across desktop, headless CLI/MCP, and `wasm32-unknown-unknown` (`curvebook-web`) with zero network calls.

---

<a id="qol12-table-completions"></a>

## 2. Categorical & Continuous Table-Analysis Completions (`R9.12` / `QOL-12` — Implemented, Wave 21)

Implemented across `crates/doc/src/analysis.rs`, `crates/stats/src/{categorical,column,two_group,anova}.rs`, `crates/engine/src/{cmd/analysis.rs,methods_report.rs}`, and `crates/ui-egui` (verified by `crates/stats/tests/wave21_r97_r912_clustering.rs` and `crates/ui-egui/tests/gui_w22b_qol_and_r9.rs`; live test counts in [`STATUS.md`](../STATUS.md)):

### 2.1 Factorial & Multifactor ANOVA Post-Hoc Multiplicity Tabs (`crates/stats/src/anova.rs`)
- Wired `FactorialAnovaSpec::multiplicity` (`crates/doc/src/analysis.rs`) inside `run_factorial_anova` to emit `ResultTabKind::MultipleComparisons` tabs (Tukey, Šídák, Bonferroni, Dunnett, Holm–Šídák, Newman–Keuls, Fisher LSD, and BH/BY/BKY FDR) across main-effect marginal means and simple-effect cell means using `compare_groups_with_procedure(&groups, ms_residual, df_residual, ...)` (or Satterthwaite/Kenward–Roger cell $df$ under REML mixed models), completing the backend tab seam used by `WF-09`, `WF-10`, and `WF-33` ([`gui-workflows.md`](../gui-workflows.md)).

### 2.2 Inter-Rater Agreement ($\kappa$), NNT CIs & Custom Goodness-of-Fit (`crates/stats/src/categorical.rs`)
- **Cohen's & Fleiss' $\kappa$ Wiring:** Exposed `cohens_kappa` (`Unweighted`, `Linear`, `Quadratic` weights; Cohen 1960, <https://doi.org/10.1177/001316446002000104>; Fleiss, Cohen & Everitt 1969, <https://doi.org/10.1037/h0028106>) and `fleiss_kappa` (Fleiss 1971, <https://doi.org/10.1037/h0031619>) via `ContingencyMethod::CohenKappa { weighting }` and `ContingencyMethod::FleissKappa`, emitting observed agreement $p_o$, chance agreement $p_e$, asymptotic SE, confidence bounds, $z$-statistic, two-sided $p$-value, and `methods_report.rs` summary prose.
- **Number Needed to Treat (`NNT`) Confidence Intervals:** Invert the Newcombe (1998, <https://doi.org/10.1002/(SICI)1097-0258(19980430)17:8%3C873::AID-SIM779%3E3.0.CO;2-I>) Wilson-score risk-difference interval $[\Delta_L, \Delta_U]$ following Altman (1998, <https://doi.org/10.1136/bmj.317.7168.1309>):
  - When $\Delta_L$ and $\Delta_U$ share the same strict sign, report finite `NNTB` or `NNTH` bounds $[1 / |\Delta_{\text{far}}|, 1 / |\Delta_{\text{near}}|]$.
  - When $[\Delta_L, \Delta_U]$ spans $0$, report the exact continuous union through $\text{ARR} = 0$ ($\infty$): $\text{NNTH } 1/|\Delta_L| \text{ to } \infty \text{ to NNTB } 1/\Delta_U$ (e.g., $\text{ARR} = -0.08\ [-0.27, 0.11] \to \text{NNTH } 12.5\ [\text{NNTH } 3.7 \text{ to } \infty \text{ to NNTB } 9.1]$), never collapsing disjoint benefit/harm rays into a misleading finite interval.
- **McNemar Matched-Pair Odds Ratio:** Emit discordant-pair odds ratio $\widehat{\text{OR}} = b / c$ with exact Clopper–Pearson transformed bounds $[p_L / (1 - p_L), p_U / (1 - p_U)]$ alongside continuity-corrected $\chi^2$ and exact binomial $p$.
- **Custom Expected Proportions in Goodness-of-Fit $\chi^2$:** Extend `PartsOfWhole` goodness-of-fit (`run_parts_of_whole_gof`) with optional `expected_proportions: Option<Vec<f64>>` (normalized from user-specified expected counts or percentages $E_i = N \pi_i$, testing $\chi^2 = \sum_{i=1}^k (O_i - E_i)^2 / E_i$ on $k - 1$ degrees of freedom alongside likelihood-ratio $G^2$).
- **Exact One-Sample Binomial & Paired Sign Test:**
  - Exact one-sample Binomial test of $k$ successes out of $n$ trials against null proportion $p_0 \in (0, 1)$ (both Sterne/small-probability sum $\sum_{P(X=j) \le P(X=k)} P(X=j)$ and doubled one-tail two-sided conventions), accompanied by Clopper–Pearson exact, Wilson score, and Agresti–Coull (1998, <https://doi.org/10.1080/00031305.1998.10480550>) modified-Wald confidence intervals.
  - Nonparametric paired Sign test on non-zero paired differences $d_i = x_i - y_i \ne 0$ via exact $\text{Binomial}(n_+, n_+ + n_-, 0.5)$.

### 2.3 Exact $\chi^2$ Standard-Deviation CI & Summary-Input *t*-Tests (`column.rs`, `two_group.rs`)
- **Confidence Interval of a Standard Deviation:** Added `sd_ci: Option<(f64, f64)>` to `DescriptiveSummary` / `PooledSummaryResult` under Gaussian sampling:
  $$\left[s \sqrt{\frac{n - 1}{\chi^2_{1 - \alpha/2,\, n - 1}}},\; s \sqrt{\frac{n - 1}{\chi^2_{\alpha/2,\, n - 1}}}\right]$$
  evaluated via `chi2_isf` / `chi2_ppf` (`crates/math/src/dist/core.rs`).
- **Pre-Summarized Two-Group & One-Sample *t*-Tests:** Extended `run_two_group_analysis` (`UnpairedT`, `WelchT`, `OneSampleT`) to accept `SubcolumnMode::{MeanSdN, MeanSemN}` summary rows directly without requiring raw individual replicates.

---

<a id="qol13-calculators"></a>

## 3. Instant Statistical, Distribution & Bayesian Calculators (`R9.13` / `QOL-13`)

**Deferred reference only:** No scheduled milestone, active implementation task, formula extension, or Calculators drawer is authorized by this section.

If resumed, the proposed design would use pure domain functions in `crates/stats/src/calculators.rs`, exposed via read-only `calc.*` commands (`crates/engine/src/cmd/calc.rs`), MCP tools, scalar table-formula builtins (`crates/transform/src/formula/eval.rs`), and the non-modal **Calculators** drawer (`ui.calculator.open` in `crates/ui-egui`).

| Proposed command ID (not registered) | Inputs | Exact Mathematical Output |
|---|---|---|
| `calc.proportionAndCountCi` | $(k, n, 1-\alpha)$ or Poisson count $(c, 1-\alpha)$ | Binomial Wilson score, Clopper–Pearson exact, and Agresti–Coull modified-Wald CIs; Garwood (1936, <https://doi.org/10.1093/biomet/28.3-4.437>) exact Poisson count CI $\left[\tfrac{1}{2}\chi^2_{\alpha/2,\,2c},\; \tfrac{1}{2}\chi^2_{1-\alpha/2,\,2(c+1)}\right]$ (with lower bound $0$ when $c = 0$). |
| `calc.clinicalPredictiveValues` | Sensitivity $Se$, Specificity $Sp$, prior prevalence $\pi \in (0, 1)$ | Positive/negative likelihood ratios $\text{LR}^+ = Se/(1-Sp)$, $\text{LR}^- = (1-Se)/Sp$, and Bayes posterior $\text{PPV} = \frac{Se \cdot \pi}{Se \cdot \pi + (1-Sp)(1-\pi)}$, $\text{NPV} = \frac{Sp(1-\pi)}{Sp(1-\pi) + (1-Se)\pi}$. |
| `calc.errorPropagation` | $(m_1, \text{SE}_1, n_1)$, $(m_2, \text{SE}_2, n_2)$, optional $\text{Cov}_{12}$, operation $\in \{+, -, \times, \div\}$, $1-\alpha$ | Sum/difference $\text{SE} = \sqrt{\text{SE}_1^2 \pm 2\text{Cov}_{12} + \text{SE}_2^2}$ (pooled or Welch–Satterthwaite $df$); unbiased independent product $\text{SE} = \sqrt{m_2^2\text{SE}_1^2 + m_1^2\text{SE}_2^2 + \text{SE}_1^2\text{SE}_2^2}$; **Fieller's (1954) exact quotient CI** (<https://doi.org/10.1111/j.2517-6161.1954.tb00159.x>; Franz 2007, <https://doi.org/10.48550/arXiv.0710.2024>) solving $(m_1 - r m_2)^2 \le t_{\text{crit}}^2(\text{SE}_1^2 - 2 r\text{Cov}_{12} + r^2\text{SE}_2^2)$ across all three regimes: bounded $[r_L, r_U]$ when $g = t_{\text{crit}}^2\text{SE}_2^2 / m_2^2 < 1$, exclusive two-ray $(-\infty, r_L] \cup [r_U, +\infty)$ when $g \ge 1$ with positive discriminant, and unbounded $(-\infty, +\infty)$ otherwise. |
| `calc.anovaPostTestSummary` | Group tuples $(m_i, n_i)$, pooled $\text{MS}_{\text{resid}}$, $df_{\text{resid}}$, procedure | Simultaneous CIs and multiplicity-adjusted *p*-values following two-way or higher-order ANOVA from published summary tables; letter displays are excluded. |
| `calc.distribution` | Statistic $\leftrightarrow$ probability for $z, t(df), F(df_1, df_2), r(n), \chi^2(df)$; or $\text{Binomial}(n,p)$, $\text{Poisson}(\lambda)$, $\mathcal{N}(\mu,\sigma)$ | Exact lower, upper, and two-sided tail probabilities (`norm_sf`, `student_t_sf`, `f_sf`, `chi2_sf`), critical quantiles (`*_isf` / `*_ppf`), and discrete/continuous PMF, PDF, and CDF tables; mirrored in `=` Calculated Variables as `norm_p`, `t_p`, `f_p`, `chi2_p`, `norm_inv`, `t_inv`, `f_inv`, `chi2_inv`, `binom_pmf`, `binom_cdf`, `pois_pmf`, `pois_cdf`. |
| `calc.interpretPValue` | Observed $p$, comparisons $m$, prior real-effect probability $\pi_1$, $df$ & effect/power, or target $\text{FPR}$ | Bonferroni & Šídák adjusted *p*-values; **Sellke–Bayarri–Berger (2001)** (<https://doi.org/10.1198/000313001300339950>) Bayes factor bound $B(p) = -e\,p\ln p$ and minimum $\text{FPR}_{\min} = (1 + [-e\,p\ln p]^{-1})^{-1}$ ($p < e^{-1}$); **Colquhoun (2017)** (<https://doi.org/10.1098/rsos.171085>) exact *"p-equals"* noncentral-$t$ point-density likelihood ratio $L_{10} = \frac{f_t(t_{\text{crit}} \mid df, \delta)}{2 f_t(t_{\text{crit}} \mid df, 0)}$ yielding $\text{FPR} = \frac{1 - \pi_1}{(1 - \pi_1) + \pi_1 L_{10}}$, post-study $\text{PPV} = 1 - \text{FPR}$, and reverse-Bayes required prior $\pi_1^* = \frac{1 - \text{FPR}}{(1 - \text{FPR}) + L_{10}\cdot\text{FPR}}$; plus **Colquhoun (2014)** (<https://doi.org/10.1098/rsos.140216>) *"p-less-than"* tail ratio $L_{10}^{\le} = (1-\beta)/p$. |
| `calc.compareFitsSummary` | $(\text{SS}_1, df_1)$, $(\text{SS}_2, df_2)$, $N$ | Extra-sum-of-squares *F*-ratio, $p$-value, $\Delta\text{AIC}_c$, and Burnham–Anderson Akaike model weights (`crates/fit/src/diagnostics.rs`). |
| `calc.ecAnything` | $\text{EC}_{50}$ (or $\log_{10}\text{EC}_{50}$), Hill slope $h$, fractional response $F \in (0, 100)$ | $\text{EC}_F = \text{EC}_{50} \cdot \left(\frac{F}{100 - F}\right)^{1/h}$ and $\log_{10}\text{EC}_F = \log_{10}\text{EC}_{50} + \frac{1}{h}\log_{10}\!\left(\frac{F}{100 - F}\right)$ (with optional delta-method CI when parameter SEs are supplied). |

---

<a id="qol14-randomization"></a>

## 4. Experimental Randomization & Subset Sampling (`R9.14` / `QOL-14` — Implemented, Wave 22)

Implemented in `crates/transform/src/randomize.rs` over `curvebook_math::DeterministicRng`, wired into `crates/engine/src/cmd/table.rs` (`table.randomizeGroups`, `table.randomSampleRows`, `table.simulateDistributions`), `crates/ui-egui/src/dialogs_r9.rs` (`RandomizeGroupsDialogState`, `RandomSampleRowsDialogState`, `SimulateDistributionsDialogState`), and `crates/ui-egui/src/menus/ui_commands.rs` (`ui.table.{randomizeGroups,randomSampleRows,simulateDistributions}`) so every experimental allocation or subsample is 100% reproducible from its recorded 64-bit seed:

1. **Subject-to-Group Randomization (`table.randomizeGroups`):**
   - Assigns $N$ subjects (numeric `1..=N` or custom identifier labels) across $k \ge 2$ treatment arms with equal or weighted allocation ratios $(w_1 : \dots : w_k)$.
   - Supports **Complete Balanced Randomization** and **Permuted-Block Randomization** (`AllocationScheme::PermutedBlock { block_sizes }`: one fixed block size, or sizes drawn uniformly per block from a list; every size must be a positive multiple of the summed allocation weights, which limits allocation prediction in unblinded sequential studies).
   - Emits a structured `MultipleVariables` schedule sheet (`Subject_ID`, `Allocation_Order`, `Block_Index`, `Block_Size`, `Assigned_Group`) and writes `randomization.seed`, `randomization.method`, `randomization.allocation_ratio`, `randomization.block_sizes`, and `randomization.total_subjects` constants into a linked `InfoSheet` for ARRIVE guideline reporting (`build_arrive_constants` in `crates/transform/src/randomize.rs`).
2. **Uniform & Stratified Random Subset Selection (`table.randomSampleRows` / `TransformAnalysisSpec::RandomSample`):**
   - Selects $k$ of $N$ active rows uniformly **without replacement** via partial Fisher–Yates shuffle (optionally stratified proportionally or equally across a categorical `MultipleVariables` column), producing a non-destructive derived sheet while leaving source observations intact.
3. **Standalone Multi-Distribution Table Generator (`table.simulateDistributions`):**
   - Populates `Column` or `MultipleVariables` sheets directly from $\text{Uniform}(a, b)$, $\mathcal{N}(\mu, \sigma)$, $\text{LogNormal}(\mu, \sigma)$, $\text{Binomial}(n, p)$, $\text{Poisson}(\lambda)$, or $\text{Exponential}(\lambda)$ draws with a recorded PRNG seed.

---

<a id="qol15-bench-calculators"></a>

## 5. Bench Molarity, Dilution & Radiochemical Calculators (`R9.15` / `QOL-15`)

**Deferred reference only:** No scheduled milestone or active implementation task. The full calculator scope, including dilution-series conveniences, remains deferred.

If resumed, the proposed stateless wet-lab preparation calculators would live in `crates/stats/src/calculators.rs` (proposed, not present) behind proposed `calc.*` commands (none registered), with optional one-click export of computed concentrations/masses into `InfoSheet` constants or `XY` dilution series:

1. **Molarity & Solution Preparation (`calc.molarity`):**
   - Solves for mass $m$, molar concentration $C$, volume $V$, or formula weight $\text{FW}$ in $m = C \cdot V \cdot \text{FW}$ across SI unit prefixes ($\text{fg}$–$\text{kg}$, $\text{fM}$–$\text{M}$, $\text{nL}$–$\text{L}$, $\text{g/mol}$–$\text{kDa}$).
2. **Stock & Serial Dilution (`calc.dilution`):**
   - Solves $C_1 V_1 = C_2 V_2$ for single-step stock dilutions ($V_{\text{stock}} = V_1$, $V_{\text{diluent}} = V_2 - V_1$, guarding $C_1 \ge C_2 > 0$) and generates multi-point serial dilution tables ($k$-fold or $\text{half-log}_{10}$ steps, with optional dead-volume overage) ready to seed an `XY` dose–response concentration column.
3. **Moles $\leftrightarrow$ Grams Converter (`calc.molesGrams`):**
   - Bidirectional conversion $n = m / \text{FW}$ and $m = n \cdot \text{FW}$ across $\text{fmol}$–$\text{mol}$ and $\text{fg}$–$\text{kg}$.
4. **Radiochemical Decay & Specific Activity (`calc.radioactivity`):**
   - Uses **2019 SI exact** physical constants ($N_A = 6.022\,140\,76 \times 10^{23}\text{ mol}^{-1}$, $1\text{ Ci} = 3.7 \times 10^{10}\text{ Bq} = 2.22 \times 10^{12}\text{ dpm}$) and the IUPAP-endorsed **NUBASE2020** evaluation (Kondev et al. 2021, <https://doi.org/10.1088/1674-1137/abddae>; tropical year $1\text{ y} = 365.2422\text{ d} = 31{,}556{,}926\text{ s}$):

   | Nuclide | Evaluated Half-Life $t_{1/2}$ (NUBASE2020) | Standard Uncertainty | Primary Decay Mode |
   |---|---|---|---|
   | $^3\text{H}$ | $12.32\text{ y}$ | $\pm 0.02\text{ y}$ | $\beta^-$ |
   | $^{14}\text{C}$ | $5700\text{ y}$ ($5.70\text{ ky}$) | $\pm 30\text{ y}$ | $\beta^-$ |
   | $^{18}\text{F}$ | $109.734\text{ min}$ | $\pm 0.008\text{ min}$ | $\beta^+ / \varepsilon$ |
   | $^{32}\text{P}$ | $14.269\text{ d}$ | $\pm 0.007\text{ d}$ | $\beta^-$ |
   | $^{33}\text{P}$ | $25.35\text{ d}$ | $\pm 0.11\text{ d}$ | $\beta^-$ |
   | $^{35}\text{S}$ | $87.37\text{ d}$ | $\pm 0.04\text{ d}$ | $\beta^-$ |
   | $^{51}\text{Cr}$ | $27.7015\text{ d}$ | $\pm 0.0011\text{ d}$ | $\varepsilon$ |
   | $^{99\text{m}}\text{Tc}$ | $6.0066\text{ h}$ | $\pm 0.0002\text{ h}$ | $\text{IT}$ |
   | $^{125}\text{I}$ | $59.392\text{ d}$ | $\pm 0.008\text{ d}$ | $\varepsilon$ |
   | $^{131}\text{I}$ | $8.0249\text{ d}$ | $\pm 0.0006\text{ d}$ | $\beta^-$ |

   - Supports custom user-entered $t_{1/2}$ (`s`, `min`, `h`, `d`, `y`) and computes elapsed-time decay fraction $2^{-\Delta t / t_{1/2}}$, remaining activity $A(t)$, carrier-free theoretical maximum specific activity $a_{\max} = (\ln 2 \cdot N_A) / t_{1/2}$, decay-corrected specific activity, required stock volume, and molar radioligand concentration (feeding `HotNM` / `HotConc_nM` in competitive binding models).

---

<a id="qol16-equations"></a>

## 6. Live Best-Fit Equation Annotations (`R9.16` / `QOL-16`, Equations Only — Implemented, Wave 22b)

- Implemented across `crates/doc/src/graph.rs` (`GraphAnnotation::ModelEquation`), `crates/fit/src/tex_format.rs` (`format_model_equation_tex`, `format_polynomial_equation_tex`, `format_number_sig_figs`), `crates/chart/src/equation_box.rs` (`ModelEquationParams`, `render_model_equation_annotation`), `EquationDisplayMode::{Symbolic, Substituted}` (`crates/doc/src/graph.rs`), `crates/engine/src/cmd/graph.rs` (`graph.addAnnotation` with kind `model_equation`), `crates/ui-egui/src/dialogs_r9.rs` (`InsertModelEquationDialogState`), and `crates/ui-egui/src/menus/ui_commands.rs` (`ui.graph.insertModelEquation`): provides one-click placement of a linked model equation on a fitted `XY` graph in symbolic or live-substituted form.
- Renders canonical $\TeX$-subset templates for built-in models (`NonlinearRegression`, `LinearRegression`, `PassingBablok`) and supported custom `fitted_curves` DSL expressions (`\frac{...}{...}`, superscripts, subscripts, Greek letters, and `\log_{10}`), with deterministic rendering across RGBA, SVG, and `%PDF-1.7`.
- Stays linked to the selected fit and updates substituted coefficients automatically after upstream edits and fit recalculation, using versioned graph metadata and existing command seams.
- **Excluded from this item:** Compact Letter Display (tracked separately in [`R10.11`](roadmap-history/r10.md#r1011-compact-letter-display-cld)), group letters, significance-matrix algorithms, pairwise-display modes, and additional $R^2$ / $\text{AIC}_c$ callouts. Existing significance brackets remain unchanged.
- **Verification:** Symbolic and live-substituted equations stay synced after recalculation and match across screen and export rendering (`gui_w22b_qol_and_r9`, `WF-49`; live test counts in [`STATUS.md`](../STATUS.md)).
- **Planned follow-up:** Rendering is single-line text today; the two-dimensional typeset layout (stacked fractions, sub/superscripts, Greek parameters) is planned in [`R10.19`](roadmap-history/r10.md#r1019-typeset-mathematical-equation-annotations) (todo).
