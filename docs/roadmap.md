# Roadmap: milestones and stage gates

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** Reconciled `G1` numerical foundations + 58-file NIST StRD corpus harness and `G2` `L0`/`L1` schema foundation against Alpha/Beta gates · **Target:** GraphPad Prism 11 (11.1.x)

Forward-looking engineering plan. Current status and headline numbers live in [`ROADMAP.md`](../ROADMAP.md) and [`target-app-parity.md`](target-app-parity.md); the ranked task queue lives in [`gaps.md`](gaps.md). Hours are single-agent wall-clock estimates.

---

## Alpha gate

The core-workflow gate: the six everyday workflows a wet-lab biologist, pharmacologist, or student performs daily, verified end to end on desktop and headless CLI with lossless save and reopen in `.lumencraft` (`crates/format/tests/prop_format.rs`) and certified NIST StRD accuracy. LumenCraft enters **alpha** when all six pass.

| Core workflow | Works end to end? | Governing spec | Est. hours to pass |
|---|:---:|---|---:|
| **1. Enter, paste, exclude & transform structured experimental data** (`XY`, `Column`, `Grouped` with replicate subcolumns, CSV/clipboard paste, blue-italic exclusions, Info constants, log/normalize transforms) | partial (`L0`/`L1` schemas) | [data-tables-and-transforms.md](data-tables-and-transforms.md) | 23–35 |
| **2. Compare groups with parametric & nonparametric tests** (unpaired/Welch/paired *t*-tests, Mann–Whitney, Wilcoxon, ordinary/Welch/RM one-way & two-way ANOVA, REML missing-replicate mixed model, Dunnett/Tukey/Šídák/BKY FDR, standardized effect sizes & Analysis Checklists) | partial (`L0` math/corpus) | [statistics-implementation.md](statistics-implementation.md) | 35–55 |
| **3. Fit nonlinear curves, compare models & interpolate unknowns** (weighted/robust Levenberg–Marquardt, 3PL/4PL/5PL dose–response, Michaelis–Menten & enzyme inhibition, binding, exponentials, shared global parameters, profile-likelihood CIs, AICc vs. *F*-test, standard-curve interpolation) | no (`M0`) | [curve-fitting-implementation.md](curve-fitting-implementation.md) | 45–70 |
| **4. Build publication graphs with live significance brackets** (XY scatter + smooth fit curves + CI bands, bar + individual beeswarm replicates, box/violin, linear/log/probit & broken axes, auto-stacked live *p*-value brackets) | no (`M0`) | [graphing-and-layouts.md](graphing-and-layouts.md) | 30–45 |
| **5. Compose figures & export journal-ready vectors/rasters** ( SVG, PDF 1.7, 300–1200 DPI PNG, lossless `.lumencraft` save/reopen, tidy CSV/JSON results export) | no (`M0`) | [graphing-and-layouts.md](graphing-and-layouts.md), [file-format-parity.md](file-format-parity.md) | 15–25 |
| **6. Drive everything headlessly via CLI, `--control` & MCP** (create tables, run stats/fits, inspect diagnostics, render PNG/SVG/PDF previews without a GUI) | partial (`M0` wiring) | [scripting-and-automation.md](scripting-and-automation.md), [mcp.md](mcp.md) | 10–15 |

---

## Beta gate

LumenCraft enters **beta** when the Alpha gate holds **and**:
1. **Prism 11 Multiple Variables workflows (`G7`)** work end to end: in-table `=` Calculated Variables, categorical level ordering, Multifactor ($N$-way) ANOVA, unified *t*-tests / nonlinear regression / survival from `MultipleVariables` tables, and categorical $\times$ continuous `MultipleVariables` graphs.
2. **Survival, Contingency, ROC & Multivariate (`G8`)** ship with exact/score CIs (Kaplan–Meier + Cox PH, Baptista–Pike OR, Koopman RR, multiple linear/logistic/Poisson GLMs, PCA).
3. **Custom Equation DSL & ODEs (`G9`)** and **Multi-panel Page Layouts + Style Cloning (`G11`)** work end to end.
4. **Clean-room `.prism` (ZIP) and `.pzfx` (XML) import (`G10`)** opens real researchers' data tables, exclusions, titles, and supported analyses cleanly without panics under fuzzing (`G12`).

---

## Milestones

| # | Milestone | Gaps covered | Status | Est. remaining, one agent (h) |
|---|---|---|---|---:|
| **M0** | Clean-room workspace, `L0`–`L7` layer enforcement, `xtask` gates, packaging, CI & subsystem specs | — | ✅ done | — |
| **M1** | Numerical primitives (`lumencraft-math`), NIST StRD harness & typed data-table model (`lumencraft-doc`, `lumencraft-format`) | `G1`, `G2` | in progress (`math`, `testkit`, `palette`, `doc` done; `format` & `G1` wiring left) | 20–34 |
| **M2** | Reactive DAG (`lumencraft-engine`) & core biostatistics + REML + multiplicity (`lumencraft-stats`) | `G2`, `G3` | planned | 45–70 |
| **M3** | Nonlinear regression solver, profile-likelihood CIs & built-in equation library (`lumencraft-fit`) | `G4` | planned | 50–78 |
| **M4** | Publication chart scene compiler (`lumencraft-chart`), live brackets & SVG/PDF/PNG export | `G5` | planned | 40–62 |
| **M5** | Interactive desktop & WASM shell (`lumencraft-ui-egui`, Navigator, subcolumn grid, inspectors) — **Alpha** | `G6`, `G14` | planned | 45–70 |
| **M6** | Prism 11 Multiple Variables, Calculated Variables, Multifactor ANOVA, Survival/Cox/GLMs/PCA, custom ODE DSL, Page Layouts & `.prism`/`.pzfx` import — **Beta** | `G7`–`G12` | planned | 125–200 |
| **M7** | Localization (`G13`), cross-OS hardware/print hardening & 1.0 release polish | `G13` | planned | 50–80 |
