# Roadmap: stage gates, execution order and milestones

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** Alpha redefined as the Essentials scope (the former six-workflow alpha gate is now the Core-workflow gate); execution order changed to a vertical slice and two risk spikes before any further widening; added verification oracles (decision `D1`: reference values from open-source statistical software are allowed) and landing rules · **Target:** GraphPad Prism 11 (11.1.x)

Forward-looking engineering plan. Current status and headline numbers live in [`ROADMAP.md`](../ROADMAP.md) and [`target-app-parity.md`](target-app-parity.md); the ranked task queue lives in [`gaps.md`](gaps.md). Hours are single-agent implementation estimates and are sums of the hours in [`gaps.md`](gaps.md), which is the source of truth. They do **not** include the time a person needs to review the work (see [Landing rules](#landing-rules)).

There are three gates on the way to `1.0`: **Alpha** (Essentials scope), **Core workflows**, and **Beta**. Each gate is passed only by behaviour that is merged, command-wired, and reachable from the CLI and the desktop UI.

---

## Alpha gate

The smallest release a real person can use: a student or bench scientist takes one experiment from raw numbers to an exported figure. LumenCraft enters **alpha** when every row below works end to end on desktop and headless CLI, with lossless save and reopen in `.lumencraft` (`crates/format/tests/prop_format.rs`) and numerics verified against a named [oracle](#verification-oracles).

| Workflow | Alpha (Essentials) scope | Works end to end? | Gaps | Est. hours |
|---|---|:---:|---|---:|
| **1. Enter, paste, exclude & transform data** | `XY`, `Column`, and `Grouped` tables with replicate subcolumns; CSV/clipboard paste; non-destructive exclusions; Info constants; log and normalize transforms; `.lumencraft` save/reopen; undo/redo | partial (`L0`/`L1` schemas) | `G1`, `G2`, `G9` | 19–32 |
| **2. Compare groups** | descriptive statistics; unpaired, Welch, and paired *t*-tests; ordinary one-way and two-way ANOVA; Tukey, Dunnett, Bonferroni, and Holm–Šídák comparisons | partial (`L0` math/corpus) | `G3` | 20–30 |
| **3. Fit curves & interpolate unknowns** | weighted Levenberg–Marquardt; linear regression; 3PL/4PL dose–response, Michaelis–Menten, and one-phase exponential models; asymptotic CIs; standard-curve interpolation | no (`M0`) | `G4` | 18–28 |
| **4. Build graphs** | XY scatter with fitted curve; bar with individual replicates; box plot; linear and log axes; error bars | no (`M0`) | `G5` | 14–21 |
| **5. Export** | SVG, PDF, and PNG figures; tidy CSV/JSON results | no (`M0`) | `G5` | 10–15 |
| **6. Drive it headlessly** | create tables, run analyses and fits, read results, and render previews through the CLI, `--control`, and MCP | partial (`M0` wiring) | `G14` | 10–15 |
| **Desktop & WASM shell** | Project Navigator, subcolumn grid, Analyze dialog, Result tabs, basic Format Graph | partial (`M0` skeleton) | `G6` | 30–45 |
| **Total to alpha** | | | | **121–186** |

---

## Core-workflow gate

The checkpoint between alpha and beta, and the scope that was previously called the alpha gate: the six everyday workflows at the depth a working wet-lab biologist or pharmacologist needs. It is passed when the Alpha gate holds **and** each workflow gains the scope below.

| Workflow | Added scope | Governing spec | Gaps | Added hours |
|---|---|---|---|---:|
| **1. Data** | live cross-table `Paste Link`, reactive transpose, 4-stage import | [data-tables-and-transforms.md](data-tables-and-transforms.md) | `G2` | 4–7 |
| **2. Compare groups** | normality battery and Grubbs/ROUT outliers; Mann–Whitney, Wilcoxon, Kruskal–Wallis, Friedman; Welch and repeated-measures ANOVA, three-way ANOVA; REML mixed model for missing replicates; Šídák, Dunn, Games–Howell, and BKY FDR; standardized effect sizes and Analysis Checklists | [statistics-implementation.md](statistics-implementation.md) | `G3` | 25–40 |
| **3. Fit curves** | robust regression and ROUT; shared global parameters; profile-likelihood CIs; AICc and extra-sum-of-squares *F* comparison; confidence and prediction bands; 5PL, enzyme inhibition, binding, and the rest of the built-in library | [curve-fitting-implementation.md](curve-fitting-implementation.md) | `G4` | 32–50 |
| **4. Graphs** | violin and before–after plots; probit, logit, and broken axes; CI bands; auto-stacked live *p*-value brackets | [graphing-and-layouts.md](graphing-and-layouts.md) | `G5` | 10–16 |
| **5. Export** | PDF 1.7 conformance, 300–1200 DPI raster presets | [graphing-and-layouts.md](graphing-and-layouts.md), [file-format-parity.md](file-format-parity.md) | `G5` | 2–4 |
| **6. Headless** | nothing beyond alpha | [scripting-and-automation.md](scripting-and-automation.md), [mcp.md](mcp.md) | — | — |
| **Desktop & WASM shell** | non-modal Format Graph / Axes / Pairwise-Comparisons inspectors, `Import & Paste Special`, `Change Graph Type` gallery, Family pane | [ui-parity.md](ui-parity.md) | `G6` | 15–25 |
| **Added to alpha** | | | | **88–142** |
| **Cumulative** | | | | **209–328** |

---

## Beta gate

LumenCraft enters **beta** when the Core-workflow gate holds **and**:
1. **Prism 11 Multiple Variables workflows (`G7`)** work end to end: in-table `=` Calculated Variables, categorical level ordering, Multifactor ($N$-way) ANOVA, unified *t*-tests / nonlinear regression / survival from `MultipleVariables` tables, and categorical $\times$ continuous `MultipleVariables` graphs.
2. **Survival, Contingency, ROC & Multivariate (`G8`)** ship with exact/score CIs (Kaplan–Meier + Cox PH, Baptista–Pike OR, Koopman RR, multiple linear/logistic/Poisson GLMs, PCA).
3. **Custom Equation DSL & ODEs (`G9`)** and **Multi-panel Page Layouts + Style Cloning (`G11`)** work end to end.
4. **Clean-room `.prism` (ZIP) and `.pzfx` (XML) import (`G10`)** opens real researchers' data tables, exclusions, titles, and supported analyses cleanly without panics under fuzzing (`G12`).

---

## Execution order

Wave 1 and Wave 2 were built layer by layer, following the stack in [`architecture.md`](architecture.md). From here the order changes: **prove one path through every layer, retire the two largest technical risks, then widen one gate at a time.** Building further library breadth before the engine exists adds work that nothing can exercise and that passes no gate.

| Step | What | Crates | Exit criterion | Status |
|---:|---|---|---|---|
| 1 | **Wave 1** — `L0`–`L1` foundations | `math`, `palette`, `doc`, `testkit` | merged, CI green | ✅ done |
| 2 | **Wave 2** — `L2`–`L3` libraries, **scope frozen** | `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf` | everything already in flight is landed one gap per pull request; nothing new is started beyond what steps 3–5 need | in progress |
| 3 | **Slice 0** — one vertical slice | thin additions in `engine`, `chart`, `svg`, `format`, `lumencraft-cli` | the [slice](#slice-0-vertical-slice) runs from the CLI and its saved project reopens identically | next |
| 3 | **Risk spikes `S1`, `S2`** (parallel with Slice 0) | `ui-egui`, `apps/lumencraft-web`; `text`, `pdf` | each [spike](#risk-spikes) ends in a written go or redesign decision | next |
| 4 | **Wave 3** — engine and commands for the Alpha scope | `tools`, `engine` | every Alpha-scope analysis, fit, graph, and export is a `CommandSpec` with tests; DAG recalculation and undo/redo work | planned, **critical path** |
| 5 | **Wave 4** — shell and automation for the Alpha scope | `ui-egui`, `mcp`, `apps/*` | **Alpha gate** passes | planned |
| 6 | **Widen to Core workflows**, one workflow at a time through all layers | all | **Core-workflow gate** passes | planned |
| 7 | **Beta scope**, one gap at a time through all layers | all | **Beta gate** passes | planned |
| 8 | Localization, hardware/print hardening, release polish | `ui-egui`, `text`, `format`, `packaging/` | `1.0` | planned |

**The engine is the critical path.** It is a single crate, so Wave 3 cannot be split across agents the way Wave 2 was, and every gate waits on it. Do not start Core or Beta library work while engine work for the current gate is open.

**Wave 2 scope freeze.** Core- and Beta-scope library work that is already in flight (for example survival, ROC, multivariate, the equation DSL, and legacy import) is finished and landed as "library verified"; it is not thrown away. No further Core- or Beta-scope library work starts until the Alpha gate passes.

### Slice 0: vertical slice

One narrow path through every layer, driven from the CLI with no UI:

1. create an `XY` table and enter dose–response data with replicates,
2. fit a 4PL model and read back parameters, standard errors, and CIs,
3. create a graph of the points and the fitted curve,
4. export it as SVG,
5. save the project as `.lumencraft`, reopen it, and get identical results and an identical SVG.

Each step is an engine command. The fit is checked against a NIST StRD nonlinear dataset or an analytical case. Slice 0 is `15–25 h` and is counted inside the Alpha hours, not added to them. Its purpose is to test the `AnalysisSpec` / `ResultSheet` / `GraphSpec` schemas and the save format against a real consumer before more code depends on them; schema changes it forces are made before Wave 3 proceeds.

### Risk spikes

Two time-boxed experiments on the parts most likely to need redesign and hardest to verify automatically. Each ends with a short written decision (go, or redesign with the reason) recorded in the governing doc. Hours are counted inside the gap named.

| Spike | Question | Evidence required | Time box | Counted in |
|---|---|---|---:|---|
| **`S1` Spreadsheet grid** | Can the virtualized two-tier subcolumn grid hold 60 fps at one million cells, with range selection, paste, and exclusion marking, on desktop **and** in the browser? | measured frame times on both targets; one working paste and one exclusion round-trip through commands | 6–10 h | `G6` |
| **`S2` Text and PDF** | Can `text` and `pdf` produce a publication-quality figure (Greek letters, superscripts and subscripts, rotated axis titles, embedded or substituted fonts) with `BUNDLED_FONTS` empty? | one reference figure exported to PDF and SVG that opens correctly in two independent viewers, with selectable text | 6–10 h | `G5` |

### Gap status

Each gap has two statuses. **Library verified** means the crate is merged with tests against a named [oracle](#verification-oracles) or round-trip properties, and `cargo xtask ci` passes. **End to end** means it is also command-wired and reachable from the CLI, MCP, and desktop UI. Only **End to end** counts toward a gate or toward the percentages in [`ROADMAP.md`](../ROADMAP.md). A status changes only after the work is merged with passing CI.

| Gap | Gates | Library verified | End to end |
|---|:---:|---|---|
| `G1` | Alpha | foundations ✅; `stats`/`fit` corpus wiring in progress (Wave 2) | n/a (test infrastructure) |
| `G2` | Alpha, Core | schemas ✅; `format` in progress (Wave 2) | no |
| `G3` | Alpha, Core | in progress (Wave 2) | no |
| `G4` | Alpha, Core | in progress (Wave 2) | no |
| `G5` | Alpha, Core | not started (Alpha part is in Wave 2 scope) | no |
| `G6` | Alpha, Core | n/a (shell) | no (`M0` skeleton) |
| `G7` | Beta | in progress (Wave 2, frozen) | no |
| `G8` | Beta | in progress (Wave 2, frozen) | no |
| `G9` | Alpha, Beta | in progress (Wave 2; Beta part frozen) | no |
| `G10` | Beta | in progress (Wave 2, frozen) | no |
| `G11` | Beta | not started | no |
| `G12` | Beta | n/a (harnesses) | no |
| `G13` | 1.0 | not started | no (`en` catalog only) |
| `G14` | Alpha, Beta | n/a (wiring) | no (`M0` skeleton) |

---

## Verification oracles

A result is "verified" only against a named public reference. NIST StRD covers four areas and textbook worked examples cover a few cases per method elsewhere. The remaining methods are verified against reference values generated with open-source statistical software, which decision `D1` below allows. A method does not count as library verified until its tests use the reference its row names.

| Area | Reference today | Coverage |
|---|---|---|
| Univariate summary statistics | NIST StRD (9 datasets) | good |
| One-way ANOVA | NIST StRD (11 datasets) | good |
| Linear regression | NIST StRD (11 datasets) | good |
| Nonlinear regression estimates and standard errors | NIST StRD (27 datasets) | good |
| *t*-tests, two-way ANOVA, Tukey, Dunnett, Bonferroni, Holm–Šídák | textbook worked examples (Zar, Montgomery); published critical-value tables | thin: a few cases per method |
| Nonparametric tests, exact small-sample distributions | textbook worked examples and exact tables (Zar) | thin |
| Kaplan–Meier, log-rank, Cox PH | textbook worked examples (Klein & Moeschberger, Hosmer & Lemeshow) | thin |
| Contingency intervals, ROC | worked examples in the original method papers | thin |
| Repeated-measures ANOVA, REML mixed models, three-way ANOVA | reference values from open-source statistical software (`D1`) | to be generated |
| Profile-likelihood CIs, robust regression, ROUT | analytical special cases, plus reference values (`D1`) where an independent implementation exists | to be generated |
| GLMs, PCA, clustering | reference values from open-source statistical software (`D1`) | to be generated |

Rows marked **thin** should be supplemented the same way, so that each method is checked on more than a handful of cases, including ties, missing values, and extreme degrees of freedom.

**Decision `D1` (maintainer, 2026-11-13): allowed.** Reference values produced by running open-source statistical software (for example R or SciPy) may be committed as test fixtures. The limits are in the clean-room section of [`AGENTS.md`](../AGENTS.md): run the software only as a black box and never read or port its source; never use the target application this way; use synthetic, NIST, or cited published input data; record the software, version, exact call, and date beside the values; and keep the software out of the build, test, and CI dependencies. No reference values have been generated yet.

---

## Landing rules

The hours in this plan measure writing the code. A person is accountable for every change ([`AGENTS.md`](../AGENTS.md)), so the rate at which work can be reviewed, not the rate at which it can be written, sets the pace.

1. **One gap per pull request, at most.** A wave or step lands as a series of pull requests, each carrying one gap id (or one part of a gap), its tests, and its doc updates. A pull request that spans several gap ids is split before review.
2. **Nothing counts until it is merged.** Statuses, percentages, and remaining hours change only after merge with `cargo xtask ci` passing.
3. **Limit unmerged work.** Do not start a new gap in a crate while an earlier pull request for that crate is still unreviewed. If review is the bottleneck, stop and say so instead of stacking more work on it.
4. **Numerical pull requests name their oracle.** The description states which reference from [Verification oracles](#verification-oracles) each test uses and the tolerance it asserts.

---

## Milestones

Milestones are retained as scope and hour buckets (and as task-id prefixes); they are **not** the order of work, which follows [Execution order](#execution-order). `M1`–`M5` together cover both the Alpha and the Core-workflow gates.

| # | Milestone | Gaps covered | Status | Est. remaining, one agent (h) |
|---|---|---|---|---:|
| **M0** | Clean-room workspace, `L0`–`L7` layer enforcement, `xtask` gates, packaging, CI & subsystem specs | — | ✅ done | — |
| **M1** | Numerical primitives (`lumencraft-math`), NIST StRD harness & typed data-table model (`lumencraft-doc`, `lumencraft-format`) | `G2` (format part) | Waves 1–2: `math`, `testkit`, `palette`, `doc` done; `format` in progress | 8–13 |
| **M2** | Reactive DAG & undo/redo (`lumencraft-engine`), log/normalize transforms, core biostatistics + REML + multiplicity (`lumencraft-stats`) with StRD wiring | `G2` (engine part, 10–17), `G3`, `G9` (Alpha part), half of `G1` wiring | library in Wave 2; engine in Wave 3 | 59–94 |
| **M3** | Nonlinear regression solver, profile-likelihood CIs & built-in equation library (`lumencraft-fit`) | `G4`, half of `G1` wiring | library in Wave 2; commands in Wave 3 | 51–80 |
| **M4** | Publication chart scene compiler (`lumencraft-chart`), live brackets & SVG/PDF/PNG export | `G5` | library in Wave 2; commands in Wave 3 | 36–56 |
| **M5** | Interactive desktop & WASM shell (`lumencraft-ui-egui`, Navigator, subcolumn grid, inspectors) — **Core-workflow gate** | `G6`, `G14` (Alpha part) | Waves 3–4, then step 6 | 55–85 |
| **M6** | Prism 11 Multiple Variables, Calculated Variables, Multifactor ANOVA, Survival/Cox/GLMs/PCA, custom ODE DSL, Page Layouts & `.prism`/`.pzfx` import — **Beta** | `G7`–`G12`, `G14` (Beta part) | some library parts in Wave 2 (frozen); rest in step 7 | 149–239 |
| **M7** | Localization (`G13`), cross-OS hardware/print hardening & 1.0 release polish | `G13` (35–55) plus 15–25 of hardening and polish not tracked as a gap | step 8 | 50–80 |
| **M8+** | Post-parity, non-breaking quality-of-life enhancements ([`qol-beyond-parity.md`](qol-beyond-parity.md): `QOL-1`–`QOL-11`) | post-`1.0` | scoped (deferred) | 82–136 |

**Cumulative, one agent:** Alpha (Essentials scope within `M1`–`M5`) `121–186 h` · Core workflows (`M1`–`M5`) `209–328 h` · Beta (`M1`–`M6`) `358–567 h` · `1.0` (`M1`–`M7`) `408–647 h`.
