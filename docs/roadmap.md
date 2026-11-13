# Roadmap: stage gates, execution order and milestones

> **Last reviewed:** 2026-11-13 · **Last updated:** 2026-11-13 · **Change:** Wave 2, Wave 3 (`crates/engine`, `crates/tools`), `Slice 0` (`lumencraft-cli`), Risk Spikes `S1` & `S2` (**GO**), headless Alpha/Core/Beta workflows (`e2e_workflows` `11 passed`), Tier-1 i18n (`9` locales × `167` keys), `UiApp` software-compositor shell (`41 passed`), `apps/lumencraft` `--control` (`6 passed`), and initial `D1` oracle suites landed; remaining hours updated across Alpha (`15–25 h`), Core (`22–36 h`), Beta (`16–27 h`), and `1.0` (`20–30 h`) · **Target:** GraphPad Prism 11 (11.1.x)

Forward-looking engineering plan. Current status and headline numbers live in [`ROADMAP.md`](../ROADMAP.md) and [`target-app-parity.md`](target-app-parity.md); the ranked task queue lives in [`gaps.md`](gaps.md). Hours are single-agent implementation estimates and are sums of the hours in [`gaps.md`](gaps.md), which is the source of truth. They do **not** include the time a person needs to review the work (see [Landing rules](#landing-rules)).

There are three gates on the way to `1.0`: **Alpha** (Essentials scope), **Core workflows**, and **Beta**. Each gate is passed only by behaviour that is merged, command-wired, and reachable from the CLI and the desktop UI.

---

## Alpha gate

The smallest release a real person can use: a student or bench scientist takes one experiment from raw numbers to an exported figure. LumenCraft enters **alpha** when every row below works end to end on desktop and headless CLI, with lossless save and reopen in `.lumencraft` (`crates/format/tests/prop_format.rs`) and numerics verified against a named [oracle](#verification-oracles).

| Workflow | Alpha (Essentials) scope | Works end to end? | Gaps | Est. remaining (h) |
|---|---|:---:|---|---:|
| **1. Enter, paste, exclude & transform data** | `XY`, `Column`, and `Grouped` tables with replicate subcolumns; CSV/clipboard paste; non-destructive exclusions; Info constants; log and normalize transforms; `.lumencraft` save/reopen; undo/redo | yes (CLI, MCP & `UiApp` ✅; `e2e_workflows` + `slice0_vertical_gate`) | `G1`, `G2`, `G9` | — (`1–2 h` crash-recovery in Core) |
| **2. Compare groups** | descriptive statistics; unpaired, Welch, and paired *t*-tests; ordinary one-way and two-way ANOVA; Tukey, Dunnett, Bonferroni, and Holm–Šídák comparisons | yes (engine, CLI, MCP & `UiApp` ✅; `stats` `36 passed`) | `G3` | — (Alpha complete; Core edge items below) |
| **3. Fit curves & interpolate unknowns** | weighted Levenberg–Marquardt; linear regression; 3PL/4PL dose–response, Michaelis–Menten, and one-phase exponential models; asymptotic CIs; standard-curve interpolation | yes (engine, CLI, MCP & `UiApp` ✅; `fit` `55 passed` w/ `InterpolatedUnknowns`) | `G4` | — (Alpha complete; Core edge items below) |
| **4. Build graphs** | XY scatter with fitted curve; bar with individual replicates; box plot; linear and log axes; error bars | yes (`chart` + `engine` + `UiApp` ✅; `73` visual tests) | `G5` | — |
| **5. Export** | SVG, PDF, and PNG figures; tidy CSV/JSON results | yes (programmatic SVG/`%PDF-1.4`/PNG/TIFF/CSV ✅) | `G5` | — (viewer check / PDF 1.7 TTF in Core) |
| **6. Drive it headlessly** | create tables, run analyses and fits, read results, and render previews through the CLI, `--control`, and MCP | yes (CLI `7 passed`, MCP `25 passed`, `lumencraft` `6 passed`, `web` `1 passed` ✅) | `G14` | — |
| **Desktop & WASM shell** | Project Navigator, subcolumn grid, Analyze dialog, Result tabs, basic Format Graph | partial (`UiApp` state shell + software compositor `41 passed` ✅; live OS `eframe`/`wgpu` & `wasm32` windowing pending) | `G6` | 15–25 |
| **Remaining to desktop Alpha gate** | *(headless Alpha workflows 1–6 already pass)* | | | **15–25** |

---

## Core-workflow gate

The checkpoint between alpha and beta, and the scope that was previously called the alpha gate: the six everyday workflows at the depth a working wet-lab biologist or pharmacologist needs. It is passed when the Alpha gate holds **and** each workflow gains the scope below. Core-workflow engine/headless support is largely landed (`e2e_workflows`); remaining hours below reflect missing secondary options, `D1` edge fixtures, interactive desktop UI polish, and PDF 1.7 TTF embedding.

| Workflow | Added scope | Governing spec | Gaps | Est. remaining (h) |
|---|---|---|---|---:|
| **1. Data** | live cross-table `Paste Link`, reactive transpose, 4-stage import (done in engine/`UiApp`); crash-recovery journal & forward schema migration (open) | [data-tables-and-transforms.md](data-tables-and-transforms.md) | `G2` | 1–2 |
| **2. Compare groups** | normality battery and Grubbs/ROUT outliers; Mann–Whitney, Wilcoxon, Kruskal–Wallis, Friedman; Welch/RM/3-way ANOVA; REML; multiplicity & Analysis Checklists (done; `NewmanKeuls`/`FisherLsd`, `multiple_row_t_tests`, Q-Q tab & expanded `D1` fixtures open) | [statistics-implementation.md](statistics-implementation.md) | `G3`, `G1` (`0–1 h`) | 3–6 |
| **3. Fit curves** | robust/ROUT, shared globals, profile-likelihood CIs, AICc/*F*, bands, 5PL/kinetics/binding (done; log-grid band sampling, conditional exclusions, centered polynomials & 5 binding models + `D1` edge fixtures open) | [curve-fitting-implementation.md](curve-fitting-implementation.md) | `G4`, `G1` (`1–1 h`) | 5–8 |
| **4. Graphs** | violin and before–after plots; probit, logit, broken axes; CI bands; auto-stacked live *p*-value brackets (done; standalone `additional_ticks` open) | [graphing-and-layouts.md](graphing-and-layouts.md) | `G5` | 1–2 |
| **5. Export** | PDF 1.7 TTF subset glyph embedding, viewer inspection (`%PDF-1.4` Core-14 & 72–1200 DPI PNG/TIFF presets done) | [graphing-and-layouts.md](graphing-and-layouts.md), [file-format-parity.md](file-format-parity.md) | `G5` | 2–3 |
| **6. Headless** | nothing beyond alpha | [scripting-and-automation.md](scripting-and-automation.md), [mcp.md](mcp.md) | — | — |
| **Desktop & WASM shell** | inspectors, `Import & Paste Special`, gallery, Family pane (implemented in `UiApp`; pointer drag-reorder, inline `Info`/`FloatingNote` editing, `F6`/rulers & custom shortcuts open) | [ui-parity.md](ui-parity.md) | `G6` | 10–15 |
| **Added remaining to Core** | | | | **22–36** |
| **Cumulative remaining (Alpha + Core)** | | | | **37–61** |

---

## Beta gate

LumenCraft enters **beta** when the Core-workflow gate holds **and** the remaining edge gaps below (`16–27 h`, `53–88 h` cumulative) are closed alongside engine/CLI/`UiApp` capabilities that are already landed:
1. **Prism 11 Multiple Variables workflows (`G7`, `2–4 h` remaining):** Calculated Variables (`100k` rows in `12.023 ms`), level ordering, Multifactor ANOVA, MV Bubble/Hull/Ellipse plots are done (`e2e_workflows`); remaining work is `TwoGroupSpec` MV grouping-column selector, multi-factor Cartesian splits in `lumencraft-fit`, and MV Kaplan–Meier wiring.
2. **Survival, Contingency, ROC & Multivariate (`G8`, `3–5 h` remaining):** Kaplan–Meier, Cox PH, Contingency CIs, ROC, GLMs, PCA, and clustering work end-to-end; remaining work is standalone `AnalysisKind::BlandAltman` tab, Barnard exact test, Passing–Bablok regression, Multiple Imputation, and `D1` Cox/GLM/PCA edge fixtures.
3. **Custom Equation DSL & ODEs (`G9`, `2–4 h`) + Multi-panel Page Layouts + Style Cloning (`G11`, `4–6 h`):** Custom bytecode/ODE DSL (`dsl_fuzz_props` `16 passed`), `LayoutSheet` composer, `applyStyleFrom`, TIFF/LZW, and headless `ClipboardBundle`/`PrintJobSpec` are done; remaining work covers `InitialValues`/transform DSL blocks, `<C>`+ scoping, live OS clipboard/print spooling, and layout WYSIWYG drag-snap.
4. **Clean-room `.prism`/`.pzfx` import (`G10`, `2–3 h`) & performance budgets (`G12`, `3–5 h`):** `.prism`/`.pzfx` import (`corrupt_archive_fuzz` `18 passed`, `interop_and_export` `4 passed`), `1M`-cell `S1` grid, and fuzz suites are done; remaining work optimizes 96-curve 4PL batch fitting from measured **`16,914.673 ms` (`16.91 s`)** down to the **`< 50 ms`** budget plus edge fixture expansion.

---

## Execution order

Wave 1 and Wave 2 were built layer by layer, following the stack in [`architecture.md`](architecture.md). From here the order changes: **prove one path through every layer, retire the two largest technical risks, then widen one gate at a time.** Building further library breadth before the engine exists adds work that nothing can exercise and that passes no gate.

| Step | What | Crates | Exit criterion | Status |
|---:|---|---|---|---|
| 1 | **Wave 1** — `L0`–`L1` foundations | `math`, `palette`, `doc`, `testkit` | merged, CI green | ✅ done |
| 2 | **Wave 2** — `L2`–`L3` libraries | `transform`, `stats`, `fit`, `text`, `chart`, `render`, `format`, `svg`, `pdf` | library suites & `D1`/StRD oracles passing | ✅ done (`stats` 36, `fit` 55, `format` 36, visual 73/32) |
| 3 | **Slice 0** — one vertical slice | `engine`, `chart`, `svg`, `format`, `lumencraft-cli` | the [slice](#slice-0-vertical-slice) runs from CLI and reopens bit-identically | ✅ done (`lumencraft-cli` `7 passed`, `Eckerle4` & `Rat43`) |
| 3 | **Risk spikes `S1`, `S2`** (parallel with Slice 0) | `tools`, `ui-egui`, `apps/*`; `text`, `pdf` | each [spike](#risk-spikes) ends in a written go or redesign decision | ✅ done — **GO** (`S1` in [`ui-parity.md`](ui-parity.md), `S2` in [`graphing-and-layouts.md`](graphing-and-layouts.md)) |
| 4 | **Wave 3** — engine and commands | `tools`, `engine` | reactive DAG, undo/redo & `CommandSpec` tests pass | ✅ done (`tools` 15, `engine` 48 incl. 73-cmd sweep & 11 E2E) |
| 5 | **Wave 4** — shell and automation for Alpha scope | `ui-egui`, `mcp`, `apps/*` | **Alpha gate** passes on desktop + WASM + `--control` | in progress (`mcp` 25, `cli` 7, `ui-egui` 41, `lumencraft` 6, `web` 1 ✅; live OS/`wgpu`/`wasm32` host open) |
| 6 | **Widen to Core workflows**, one workflow at a time through all layers | all | **Core-workflow gate** passes | partially landed (`e2e_workflows` headless ✅; live OS UI + edge library items open) |
| 7 | **Beta scope**, one gap at a time through all layers | all | **Beta gate** passes | partially landed (`G7`–`G12` wired headlessly ✅; 96-curve `<50ms` & OS print/clipboard open) |
| 8 | Localization, hardware/print hardening, release polish | `ui-egui`, `text`, `format`, `packaging/` | `1.0` | partially landed (Tier-1 `9`×`167` ✅; Tier-2/RTL, `G13` OS CJK & `M7` hardening open) |

**The engine is the critical path.** It is a single crate, so Wave 3 cannot be split across agents the way Wave 2 was, and every gate waits on it. Do not start Core or Beta library work while engine work for the current gate is open.

**Wave 2 scope freeze.** Core- and Beta-scope library work that is already in flight (for example survival, ROC, multivariate, the equation DSL, and legacy import) is finished and landed as "library verified"; it is not thrown away. No further Core- or Beta-scope library work starts until the Alpha gate passes.

### Slice 0: vertical slice

**Status: ✅ DONE (`apps/lumencraft-cli/tests/slice0_vertical_gate.rs`, `7 passed`).** One narrow path through every layer, driven from `lumencraft-cli` with no UI:

1. create an `XY` table and enter dose–response data with replicates,
2. fit a 4PL model and read back parameters, standard errors, and CIs (plus NIST StRD `Eckerle4` and `Rat43` verification),
3. create a graph of the points and the fitted curve,
4. export it as SVG,
5. save the project as `.lumencraft`, reopen it, and obtain bit-identical archive/recomputed results and an identical SVG.

`Slice 0` confirmed the `AnalysisSpec` / `ResultSheet` / `GraphSpec` schemas and `.lumencraft` archive format cleanly before Wave 3 engine wiring proceeded.

### Risk spikes

Two time-boxed experiments on the parts most likely to need redesign and hardest to verify automatically. Both concluded with explicit written **GO** decisions recorded in their governing docs.

| Spike | Question | Measured outcome & decision | Governing doc |
|---|---|---|---|
| **`S1` Spreadsheet grid** | Can the virtualized two-tier subcolumn grid hold 60 fps at one million cells, with range selection, paste, and exclusion marking, on desktop **and** in the browser? | **GO (with WASM sysroot note):** at `1_000 × 1_000` (`1,000,000` cells), `lumencraft-tools` measured `p50 = 0.0390 ms`, `p95 = 0.0432 ms`, `max = 0.0612 ms` (`120` frames, tail-jump `0.0150 ms`); `lumencraft-ui-egui` measured `p50 = 0.061 ms`, `p95 = 0.507 ms`, `max = 7.795 ms`, `Select All` + batched `table.excludeCells` `0.044 ms` (`< 16.67 ms` budget). Engine/UI shell verified headlessly; native OS windowing & `wasm32` sysroot browser build tracked in `G6`. | [`ui-parity.md`](ui-parity.md#risk-spike-s1-decision-spreadsheet-grid-at-60-fps-go) |
| **`S2` Text and PDF** | Can `text` and `pdf` produce a publication-quality figure (Greek letters, superscripts and subscripts, rotated axis titles, embedded or substituted fonts) with `BUNDLED_FONTS` empty? | **GO:** direct pure-Rust `%PDF-1.4` writer (`crates/pdf`) with Core-14 Type1 Helvetica/Symbol selectable Greek/math/super/subscript/rotated text + outlined fallback generated 2-page PDFs (`15` objs, `16` xref entries, `FlateDecode` `2898` B) in **`0.293–0.395 ms`** under `--no-default-features` (`BUNDLED_FONTS` empty) and **`0.422 ms`** default (`73` default / `32` `--no-default-features` visual tests). Verified structurally in tests; interactive GUI viewer inspection & `PDF 1.7` TTF subset embedding queued in Core. | [`graphing-and-layouts.md` §7](graphing-and-layouts.md#7-risk-spike-s2-decision-text--pdf-without-bundled-fonts-go) |

### Gap status

Each gap has two statuses. **Library verified** means the crate is merged with tests against a named [oracle](#verification-oracles) or round-trip properties. **End to end** means it is also command-wired and reachable from `lumencraft-engine`, CLI, MCP, and/or the UI shell (`UiApp`); live OS desktop/`wgpu` windowing status is noted explicitly for `G6` and `G14`.

| Gap | Gates | Library verified | End to end |
|---|:---:|---|---|
| `G1` | Core (`D1` edge polish) | ✅ NIST StRD wired (`stats` `4`, `fit` `1`) + initial `D1` oracles (`stats` `20`, `fit` `18`) | n/a (test infrastructure; expanded `D1` fixtures remaining `1–2 h`) |
| `G2` | Alpha, Core | ✅ (`palette`, `doc`, `format` `36 passed`) | ✅ CLI/MCP/`UiApp` (`engine` `48 passed`, `slice0` `7 passed`; crash journal remaining `1–2 h`) |
| `G3` | Alpha, Core | ✅ (`stats` `36 passed`; `NewmanKeuls`/`FisherLsd` & `multiple_row_t_tests` open) | ✅ CLI/MCP/`UiApp` (`e2e_workflows` ✅; secondary items + `D1` `3–5 h`) |
| `G4` | Alpha, Core | ✅ (`fit` `55 passed` w/ `InterpolatedUnknowns`; 5 binding eqs & log-grid bands open) | ✅ CLI/MCP/`UiApp` (`e2e_workflows` + `Slice 0` ✅; edge items `4–7 h`) |
| `G5` | Alpha, Core | ✅ **S2 GO** (`73` / `32` tests; PDF 1.7 TTF & `additional_ticks` open) | ✅ CLI/MCP/`UiApp` (`e2e_workflows` ✅; viewer check / PDF 1.7 `3–5 h`) |
| `G6` | Alpha, Core | n/a (shell; **S1 GO** `0.0390`/`0.061 ms` p50) | partial (`UiApp` state shell + software compositor `41 passed`; live OS/`wgpu`/`wasm32` + drag/editor polish `25–40 h`) |
| `G7` | Beta | ✅ (`transform` `100k` in `12.023ms`, `stats`/`doc`/`chart`; multi-factor split & MV *t*-test/KM open) | ✅ CLI/MCP/`UiApp` (`e2e_workflows` ✅; edge wiring `2–4 h`) |
| `G8` | Beta | ✅ (`stats`/`chart`; standalone Bland–Altman tab, Barnard/Passing–Bablok/MI open) | ✅ CLI/MCP/`UiApp` (`e2e_workflows` ✅; edge methods `3–5 h`) |
| `G9` | Alpha, Beta | ✅ (`dsl_fuzz_props` `16 passed` + `transform`; `InitialValues` & `<C>`+ open) | ✅ CLI/MCP/`UiApp` (`e2e_workflows` ✅; DSL blocks `2–4 h`) |
| `G10` | Beta | ✅ (`corrupt_archive_fuzz` `18`, `interop_and_export` `4`) | ✅ CLI/MCP/`UiApp` (edge corpus + UI drag-drop `2–3 h`) |
| `G11` | Beta | ✅ (`LayoutSheet`, Wand, LZW TIFF, `ClipboardBundle`, `PrintJobSpec`) | partial (headless ✅; live OS pasteboard/printer & canvas snap `4–6 h`) |
| `G12` | Beta | n/a (`perf` + 7 fuzz/prop suites) | partial (`S1` + `100k` CalcVar `12ms` ✅; 96-curve 4PL **`16.91s`** vs `<50ms` budget `3–5 h`) |
| `G13` | 1.0 | ✅ Tier-1 `9`×`167` = `1,503` keys, CSV/decimal presets | partial (Tier-1 `100%` ✅; Tier-2/RTL `ar`/`he` & desktop CJK `8–12 h`) |
| `G14` | Alpha, Beta | ✅ (`ui-commands --check`, MCP `25 passed`) | ✅ CLI/MCP/`--control` (`cli` `7`, `mcp` `25`, `lumencraft` `6`, `web` `1`; GUI window flags with `G6`) |

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
| Repeated-measures ANOVA, REML mixed models, three-way ANOVA | reference values from open-source statistical software (`D1`) + `d1_reference_oracles.rs` | started; edge matrix to be expanded |
| Profile-likelihood CIs, robust regression, ROUT | analytical special cases + `d1_nist_and_pharma_oracles.rs` (`D1`) | started (`18` tests); edge cases expanding |
| GLMs, PCA, clustering | reference values (`d1_reference_oracles.rs` `20` tests across stats) | started; multi-tie/edge fixtures expanding |

Rows marked **thin** or **started** should continue to be supplemented the same way, so that each method is checked on more than a handful of cases, including ties, missing values, and extreme degrees of freedom.

**Decision `D1` (maintainer, 2026-11-13): allowed.** Reference values produced by running open-source statistical software (for example R or SciPy) may be committed as test fixtures. The limits are in the clean-room section of [`AGENTS.md`](../AGENTS.md): run the software only as a black box and never read or port its source; never use the target application this way; use synthetic, NIST, or cited published input data; record the software, version, exact call, and date beside the values; and keep the software out of the build, test, and CI dependencies. Initial `D1` oracle test suites have landed in [`crates/stats/tests/d1_reference_oracles.rs`](../crates/stats/tests/d1_reference_oracles.rs) (`20 passed`) and [`crates/fit/tests/d1_nist_and_pharma_oracles.rs`](../crates/fit/tests/d1_nist_and_pharma_oracles.rs) (`18 passed`); broader unbalanced/edge-case `D1` matrices continue in `G1`/`G3`/`G8`.

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
| **M1** | Numerical primitives (`lumencraft-math`), NIST StRD harness & typed data-table model (`lumencraft-doc`, `lumencraft-format`) | `G2` (format recovery/migration part) | `math`, `testkit`, `palette`, `doc`, `format` (`36 passed`) landed | 1–2 |
| **M2** | Reactive DAG & undo/redo (`lumencraft-engine`), log/normalize transforms, core biostatistics + REML + multiplicity (`lumencraft-stats`) with StRD wiring | `G2` (engine done), `G3`, `G9` (Alpha done), `G1` stats `D1` edge part (`0–1`) | engine & core stats verified (`stats` 36, `engine` 48); edge multiplicity/`D1` open | 3–6 |
| **M3** | Nonlinear regression solver, profile-likelihood CIs & built-in equation library (`lumencraft-fit`) | `G4`, `G1` fit `D1` edge part (`1–1`) | library & Slice 0 verified (`fit` 55 w/ `InterpolatedUnknowns`); log bands/5 eqs/`D1` open | 5–8 |
| **M4** | Publication chart scene compiler (`lumencraft-chart`), live brackets & SVG/PDF/PNG export | `G5` | **S2 GO** (`73` / `32` tests); `additional_ticks`, PDF 1.7 TTF & viewer spot-check open | 3–5 |
| **M5** | Interactive desktop & WASM shell (`lumencraft-ui-egui`, Navigator, subcolumn grid, inspectors) & Alpha automation — **Core-workflow gate** | `G6`, `G14` (CLI/MCP/`--control` ✅) | **S1 GO**, `UiApp` state shell (`41`), `cli` (`7`), `mcp` (`25`), `lumencraft` (`6`), `web` (`1`); live OS/`wgpu`/`wasm32` host & UI pointer polish open | 25–40 |
| **M6** | Prism 11 Multiple Variables, Calculated Variables, Multifactor ANOVA, Survival/Cox/GLMs/PCA, custom ODE DSL, Page Layouts & `.prism`/`.pzfx` import — **Beta** | `G7`–`G12` | headless/engine Beta workflows (`e2e_workflows`) & fuzz suites landed; **96-curve `<50ms` opt**, OS print/clipboard & remaining edge items open | 16–27 |
| **M7** | Localization (`G13`), cross-OS hardware/print hardening & 1.0 release polish | `G13` (`8–12`) plus `12–18` OS GPU/trackpad/installer hardening not tracked as a gap | Tier-1 `9`×`167` landed; Tier-2/RTL `ar`/`he` & desktop hardware verification open | 20–30 |
| **M8+** | Post-parity, non-breaking quality-of-life enhancements ([`qol-beyond-parity.md`](qol-beyond-parity.md): `QOL-1`–`QOL-11`) | post-`1.0` | scoped (deferred) | 82–136 |

**Cumulative remaining, one agent:** Alpha (`G6` OS/`wgpu`/`wasm32` host `15–25 h`; headless Alpha already passes) · Core workflows (`M1`–`M5`) `37–61 h` · Beta (`M1`–`M6`) `53–88 h` · `1.0` (`M1`–`M7`) **`73–118 h`**.
